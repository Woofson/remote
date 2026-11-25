//! Session recording (#31).
//!
//! A recording is what the browser was shown, with timing: never keystrokes. Terminal
//! sessions are stored as a timed text stream (one `[seconds, "o", text]` line per output,
//! after a JSON header line), the widely used terminal recording format, so a downloaded
//! recording can be read as text or played by common terminal players.
//!
//! On disk the stream is cut into chunks; each chunk is compressed and then encrypted with
//! the recording's own random key (AES-256-GCM, chunk number as associated data, so chunks
//! cannot be reordered or swapped). The key itself is kept in the database, encrypted with
//! the server's master secret. Chunks are written every few seconds, so a crash loses at
//! most the last few seconds.
//!
//! File layout: `REMOTEREC1` then repeated `[u32 length][12-byte nonce][ciphertext]`.

use crate::db::{Database, RecordingRecord};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde_json::json;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const MAGIC: &[u8] = b"REMOTEREC1";
/// Write a chunk once this much is waiting...
const CHUNK_BYTES: usize = 64 * 1024;
/// ...or this long after the last chunk.
const CHUNK_SECS: u64 = 5;
/// Largest chunk accepted when reading (guards against damaged files).
const MAX_CHUNK: usize = 64 * 1024 * 1024;

/// What the activity log hears about a recording.
pub type AuditFn = Box<dyn Fn(&'static str, String) + Send + Sync>;

/// Everything needed to start a recording for one session.
pub struct RecordingStart {
    pub folder: PathBuf,
    pub master_secret: String,
    pub user_id: String,
    pub username: String,
    pub connection_id: String,
    pub connection_name: String,
    pub protocol: String,
    pub cols: u16,
    pub rows: u16,
}

enum Cmd {
    Data(Vec<u8>),
    Finish,
}

/// Records one terminal session. Lives in the session relay; dropping it ends the recording.
pub struct Recorder {
    tx: mpsc::Sender<Cmd>,
    started: Instant,
    utf8_carry: Vec<u8>,
    finished: bool,
}

impl Recorder {
    /// Create the file and the database entry, and start the writer thread.
    pub fn start(db: Database, s: RecordingStart, audit: AuditFn) -> Result<Recorder, String> {
        create_private_dir(&s.folder)?;
        let id = uuid::Uuid::new_v4().to_string();
        let file_name = format!("{id}.rrec");
        let path = s.folder.join(&file_name);

        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).map_err(|e| format!("No random source: {e}"))?;
        let key_enc = crate::auth::encrypt_secret(&hex::encode(key), &s.master_secret)?;

        let mut file = open_private_file(&path)?;
        file.write_all(MAGIC).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;

        let now = chrono::Utc::now();
        let rec = RecordingRecord {
            id: id.clone(),
            user_id: s.user_id,
            username: s.username,
            connection_id: Some(s.connection_id),
            connection_name: s.connection_name.clone(),
            protocol: s.protocol,
            kind: "terminal".into(),
            started_at: now.to_rfc3339(),
            ended_at: None,
            duration_ms: 0,
            size_bytes: MAGIC.len() as i64,
            status: "recording".into(),
            file_name,
            key_enc,
        };
        if let Err(e) = db.create_recording(&rec) {
            let _ = std::fs::remove_file(&path);
            return Err(format!("Cannot save the recording entry: {e}"));
        }

        let header = json!({
            "version": 2,
            "width": s.cols,
            "height": s.rows,
            "timestamp": now.timestamp(),
            "title": s.connection_name,
        });
        let mut first = header.to_string().into_bytes();
        first.push(b'\n');

        let (tx, rx) = mpsc::channel::<Cmd>();
        let started = Instant::now();
        let writer = ChunkWriter { file, cipher: Aes256Gcm::new_from_slice(&key).expect("32-byte key"), index: 0, size: MAGIC.len() as u64 };
        std::thread::Builder::new()
            .name("recording".into())
            .spawn(move || write_loop(rx, writer, first, db, id, path, started, audit))
            .map_err(|e| format!("Cannot start the recording writer: {e}"))?;

        Ok(Recorder { tx, started, utf8_carry: Vec::new(), finished: false })
    }

    fn secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn event(&mut self, code: &str, data: &str) {
        let mut line = json!([(self.secs() * 1_000_000.0).round() / 1_000_000.0, code, data]).to_string().into_bytes();
        line.push(b'\n');
        let _ = self.tx.send(Cmd::Data(line));
    }

    /// Output the browser was shown.
    pub fn output(&mut self, bytes: &[u8]) {
        let text = take_utf8(&mut self.utf8_carry, bytes);
        if !text.is_empty() {
            self.event("o", &text);
        }
    }

    /// The terminal changed size.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.event("r", &format!("{cols}x{rows}"));
    }

    /// End the recording (also happens when the recorder is dropped).
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        if !self.utf8_carry.is_empty() {
            let rest = String::from_utf8_lossy(&std::mem::take(&mut self.utf8_carry)).into_owned();
            self.event("o", &rest);
        }
        let _ = self.tx.send(Cmd::Finish);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.finish();
    }
}

struct ChunkWriter {
    file: std::fs::File,
    cipher: Aes256Gcm,
    index: u64,
    size: u64,
}

impl ChunkWriter {
    fn write_chunk(&mut self, plain: &[u8]) -> std::io::Result<()> {
        if plain.is_empty() {
            return Ok(());
        }
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(plain)?;
        let packed = enc.finish()?;
        let mut nonce = [0u8; 12];
        getrandom::getrandom(&mut nonce).map_err(std::io::Error::other)?;
        let aad = self.index.to_le_bytes();
        let sealed = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: &packed, aad: &aad })
            .map_err(|_| std::io::Error::other("encryption failed"))?;
        let len = (12 + sealed.len()) as u32;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&nonce)?;
        self.file.write_all(&sealed)?;
        self.file.flush()?;
        self.index += 1;
        self.size += 4 + len as u64;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn write_loop(
    rx: mpsc::Receiver<Cmd>,
    mut w: ChunkWriter,
    first: Vec<u8>,
    db: Database,
    id: String,
    path: PathBuf,
    started: Instant,
    audit: AuditFn,
) {
    let mut buf = first;
    let mut last_write = Instant::now();
    let mut failed = false;
    audit("RECORDING_STARTED", "session recording started".into());
    loop {
        let wait = Duration::from_secs(CHUNK_SECS).saturating_sub(last_write.elapsed());
        let msg = rx.recv_timeout(wait);
        let done = matches!(msg, Ok(Cmd::Finish) | Err(mpsc::RecvTimeoutError::Disconnected));
        if let Ok(Cmd::Data(d)) = msg {
            buf.extend_from_slice(&d);
        }
        if !failed && (done || buf.len() >= CHUNK_BYTES || last_write.elapsed() >= Duration::from_secs(CHUNK_SECS)) {
            if let Err(e) = w.write_chunk(&buf) {
                warn!("Recording {}: cannot write to {}: {e}", id, path.display());
                failed = true;
            }
            buf.clear();
            last_write = Instant::now();
        }
        if failed {
            buf.clear();
        }
        if done {
            break;
        }
    }
    let _ = w.file.sync_all();
    let duration_ms = started.elapsed().as_millis() as i64;
    let _ = db.finish_recording(&id, &chrono::Utc::now().to_rfc3339(), duration_ms, w.size as i64, if failed { "interrupted" } else { "done" });
    audit(
        "RECORDING_STOPPED",
        format!("session recording stopped ({}, {})", format_duration(duration_ms), format_size(w.size)),
    );
}

/// Read a whole recording back: the decrypted, uncompressed stream. A damaged or cut-off
/// end (power loss while writing) is skipped.
pub fn read_recording(path: &Path, key_enc: &str, master_secret: &str) -> Result<Vec<u8>, String> {
    let key = crate::auth::decrypt_secret(key_enc, master_secret).map_err(|_| "This recording's key cannot be unlocked".to_string())?;
    let key = hex::decode(key).map_err(|_| "Damaged recording key".to_string())?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "Damaged recording key".to_string())?;
    let data = std::fs::read(path).map_err(|e| format!("Cannot read the recording file: {e}"))?;
    if !data.starts_with(MAGIC) {
        return Err("Not a Remote recording".into());
    }
    let mut out = Vec::new();
    let mut pos = MAGIC.len();
    let mut index: u64 = 0;
    while pos + 4 <= data.len() {
        let len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if len < 12 || len > MAX_CHUNK || pos + len > data.len() {
            break;
        }
        let (nonce, sealed) = data[pos..pos + len].split_at(12);
        pos += len;
        let aad = index.to_le_bytes();
        let Ok(packed) = cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: &aad }) else {
            if index == 0 {
                return Err("This recording cannot be unlocked".into());
            }
            break;
        };
        DeflateDecoder::new(&packed[..]).read_to_end(&mut out).map_err(|_| "Damaged recording".to_string())?;
        index += 1;
    }
    Ok(out)
}

/// Readable text of a terminal recording (for searching): output only, control codes removed.
pub fn terminal_text(stream: &[u8]) -> String {
    let mut text = String::new();
    for line in String::from_utf8_lossy(stream).lines().skip(1) {
        if let Ok(serde_json::Value::Array(ev)) = serde_json::from_str::<serde_json::Value>(line) {
            if ev.get(1).and_then(|c| c.as_str()) == Some("o") {
                if let Some(s) = ev.get(2).and_then(|d| d.as_str()) {
                    text.push_str(&strip_ansi(s));
                }
            }
        }
    }
    text
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.peek() == Some(&'\\')) {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c == '\r' || (c.is_control() && c != '\n' && c != '\t') {
            continue;
        } else {
            out.push(c);
        }
    }
    out
}

/// Turn bytes into text, keeping an unfinished multi-byte character for next time.
fn take_utf8(carry: &mut Vec<u8>, bytes: &[u8]) -> String {
    carry.extend_from_slice(bytes);
    let mut out = String::new();
    let mut rest: &[u8] = carry;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                rest = &[];
                break;
            }
            Err(e) => {
                let (ok, after) = rest.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(ok).unwrap());
                match e.error_len() {
                    Some(n) => {
                        out.push('\u{FFFD}');
                        rest = &after[n..];
                    }
                    None => {
                        rest = after;
                        break;
                    }
                }
            }
        }
    }
    *carry = rest.to_vec();
    out
}

// ---------------------------------------------------------------------------------------------
// Housekeeping
// ---------------------------------------------------------------------------------------------

/// At startup: recordings that were running when the server stopped are closed off.
pub fn close_interrupted(db: &Database, folder: &Path) {
    let Ok(list) = db.unfinished_recordings() else { return };
    for r in list {
        let path = folder.join(&r.file_name);
        let meta = std::fs::metadata(&path).ok();
        let size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let ended = meta
            .and_then(|m| m.modified().ok())
            .map(chrono::DateTime::<chrono::Utc>::from)
            .unwrap_or_else(chrono::Utc::now);
        let duration = chrono::DateTime::parse_from_rfc3339(&r.started_at)
            .map(|s| (ended - s.with_timezone(&chrono::Utc)).num_milliseconds().max(0))
            .unwrap_or(0);
        let _ = db.finish_recording(&r.id, &ended.to_rfc3339(), duration, size, "interrupted");
    }
}

/// Delete recordings past the keep time, then the oldest while over the size limit.
/// Returns how many were deleted.
pub fn cleanup(db: &Database, folder: &Path, keep_days: u64, max_total_bytes: u64) -> usize {
    let Ok(mut list) = db.list_recordings() else { return 0 };
    list.retain(|r| r.status != "recording");
    list.sort_by(|a, b| a.started_at.cmp(&b.started_at)); // oldest first
    let cutoff = (keep_days > 0).then(|| chrono::Utc::now() - chrono::Duration::days(keep_days as i64));
    let mut total: u64 = list.iter().map(|r| r.size_bytes.max(0) as u64).sum();
    let mut deleted = 0;
    for r in &list {
        let too_old = cutoff.is_some_and(|c| {
            chrono::DateTime::parse_from_rfc3339(&r.started_at).map(|t| t.with_timezone(&chrono::Utc) < c).unwrap_or(false)
        });
        let over = max_total_bytes > 0 && total > max_total_bytes;
        if !too_old && !over {
            continue;
        }
        if remove_files(folder, &r.file_name) && db.delete_recording(&r.id).unwrap_or(false) {
            total = total.saturating_sub(r.size_bytes.max(0) as u64);
            deleted += 1;
        }
    }
    if deleted > 0 {
        info!("Recordings: deleted {} old recording(s)", deleted);
    }
    deleted
}

/// Remove a recording's file. True when it is gone (also when it was already missing).
pub fn remove_files(folder: &Path, file_name: &str) -> bool {
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return false;
    }
    match std::fs::remove_file(folder.join(file_name)) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            warn!("Recordings: cannot delete {}: {e}", file_name);
            false
        }
    }
}

/// Total size of all recordings, in bytes.
pub fn total_size(db: &Database) -> u64 {
    db.list_recordings().map(|l| l.iter().map(|r| r.size_bytes.max(0) as u64).sum()).unwrap_or(0)
}

fn create_private_dir(dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).map_err(|e| format!("Cannot create the recordings folder {}: {e}", dir.display()))
}

fn open_private_file(path: &Path) -> Result<std::fs::File, String> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path).map_err(|e| format!("Cannot create {}: {e}", path.display()))
}

pub fn format_duration(ms: i64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min {} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_characters_are_kept_whole() {
        let mut carry = Vec::new();
        let euro = "€".as_bytes(); // 3 bytes
        assert_eq!(take_utf8(&mut carry, &euro[..1]), "");
        assert_eq!(take_utf8(&mut carry, &euro[1..]), "€");
        assert!(carry.is_empty());
        assert_eq!(take_utf8(&mut carry, b"a\xffb"), "a\u{FFFD}b");
    }

    #[test]
    fn control_codes_are_removed_for_search() {
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m\r\n\x1b]0;title\x07done"), "ok\ndone");
    }

    #[test]
    fn recording_round_trip_and_cleanup() {
        let dir = std::env::temp_dir().join(format!("remote-rec-test-{}", uuid::Uuid::new_v4()));
        let db = Database::new_in_memory().unwrap();
        let secret = "test-master-secret-0123456789abcdef";
        let mut rec = Recorder::start(
            db.clone(),
            RecordingStart {
                folder: dir.clone(),
                master_secret: secret.into(),
                user_id: "u1".into(),
                username: "alice".into(),
                connection_id: "c1".into(),
                connection_name: "box".into(),
                protocol: "ssh".into(),
                cols: 80,
                rows: 24,
            },
            Box::new(|_, _| {}),
        )
        .unwrap();
        rec.output(b"hello ");
        rec.resize(100, 30);
        rec.output("w\u{f6}rld\r\n".as_bytes());
        rec.finish();
        drop(rec);

        // The writer thread finishes in the background.
        let mut r = None;
        for _ in 0..100 {
            let list = db.list_recordings().unwrap();
            if list.first().is_some_and(|x| x.status == "done") {
                r = list.into_iter().next();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let r = r.expect("recording finished");
        let path = dir.join(&r.file_name);
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(5).any(|w| w == b"hello"), "stored encrypted");
        assert_eq!(r.size_bytes as usize, raw.len());

        let stream = read_recording(&path, &r.key_enc, secret).unwrap();
        let text = String::from_utf8(stream.clone()).unwrap();
        let mut lines = text.lines();
        assert!(lines.next().unwrap().contains("\"width\":80"));
        assert!(text.contains("\"r\",\"100x30\""));
        assert_eq!(terminal_text(&stream), "hello w\u{f6}rld\n");
        assert!(read_recording(&path, &r.key_enc, "another-secret-entirely-000000").is_err());

        // Over the size limit: removed with its file.
        assert_eq!(cleanup(&db, &dir, 30, 1), 1);
        assert!(!path.exists());
        assert!(db.list_recordings().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
