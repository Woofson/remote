//! Desktop recordings (RDP, VNC).
//!
//! What is stored is exactly the picture updates the browser was sent (full tiles, batches
//! of tiles, copied areas), each with its time. To make jumping around quick without asking
//! the remote computer for extra pictures, the writer keeps its own copy of the screen and
//! regularly starts a *seek point*: a chunk that begins with the screen size and a full
//! picture. The player loads the seek point before the wanted moment and plays from there.
//!
//! File layout: `REMOTEREC2`, then chunks `[u32 length][u8 flags][u32 time ms][nonce][ciphertext]`
//! where flags bit 0 marks a seek point. Flags and time are authenticated with the chunk
//! number, so they cannot be altered, but can be read without the key to build the index.
//!
//! Plaintext events: `[u32 time ms][u8 kind][u32 length][payload]`, kind 0 = picture update
//! (as sent to the browser), kind 1 = screen size (`u16 width, u16 height`).

use super::{chunk_aad, unlock, ChunkWriter, Cmd, Finisher, CHUNK_SECS, MAX_CHUNK};
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::Nonce;
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tracing::warn;

pub(super) const MAGIC: &[u8] = b"REMOTEREC2";
const FLAG_SEEK: u8 = 1;
const EV_PICTURE: u8 = 0;
const EV_SIZE: u8 = 1;
/// A new seek point at most this often while the picture changes...
const SEEK_SECS: u64 = 60;
/// ...or once this many bytes of updates were recorded since the last one.
const SEEK_BYTES: usize = 64 * 1024 * 1024;
/// Write a chunk once this much is waiting (or every few seconds).
const CHUNK_BYTES: usize = 2 * 1024 * 1024;

/// A place playback can start from.
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub t_ms: u32,
    #[serde(skip)]
    chunk: u64,
    #[serde(skip)]
    offset: u64,
}

/// The writer's copy of the screen (RGBA).
#[derive(Default)]
struct Screen {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

fn be16(b: &[u8], at: usize) -> usize {
    u16::from_be_bytes([b[at], b[at + 1]]) as usize
}

impl Screen {
    fn resize(&mut self, w: u16, h: u16) {
        self.w = w as usize;
        self.h = h as usize;
        self.px = vec![0; self.w * self.h * 4];
        for a in self.px.iter_mut().skip(3).step_by(4) {
            *a = 255;
        }
    }

    fn put(&mut self, x: usize, y: usize, w: usize, h: usize, data: &[u8]) {
        if data.len() < w * h * 4 || x >= self.w {
            return;
        }
        let cw = w.min(self.w - x);
        for row in 0..h {
            let sy = y + row;
            if sy >= self.h {
                break;
            }
            let src = &data[row * w * 4..row * w * 4 + cw * 4];
            let at = (sy * self.w + x) * 4;
            self.px[at..at + cw * 4].copy_from_slice(src);
        }
    }

    /// Apply an update exactly as the browser draws it.
    fn apply(&mut self, p: &[u8]) {
        match p.first() {
            Some(0x01) if p.len() >= 9 => {
                let (x, y, w, h) = (be16(p, 1), be16(p, 3), be16(p, 5), be16(p, 7));
                self.put(x, y, w, h, &p[9..]);
            }
            Some(0x03) if p.len() >= 3 => {
                let count = be16(p, 1);
                let mut at = 3;
                for _ in 0..count {
                    if at + 8 > p.len() {
                        break;
                    }
                    let (x, y, w, h) = (be16(p, at), be16(p, at + 2), be16(p, at + 4), be16(p, at + 6));
                    at += 8;
                    let len = w * h * 4;
                    if at + len > p.len() {
                        break;
                    }
                    self.put(x, y, w, h, &p[at..at + len]);
                    at += len;
                }
            }
            Some(0x02) if p.len() >= 13 => {
                let (x, y, w, h, sx, sy) = (be16(p, 1), be16(p, 3), be16(p, 5), be16(p, 7), be16(p, 9), be16(p, 11));
                if sx >= self.w || sy >= self.h {
                    return;
                }
                let (w, h) = (w.min(self.w - sx), h.min(self.h - sy));
                let mut area = Vec::with_capacity(w * h * 4);
                for row in 0..h {
                    let at = ((sy + row) * self.w + sx) * 4;
                    area.extend_from_slice(&self.px[at..at + w * 4]);
                }
                self.put(x, y, w, h, &area);
            }
            _ => {}
        }
    }

    /// The whole screen as one picture update.
    fn full_picture(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(9 + self.px.len());
        p.push(0x01);
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(&0u16.to_be_bytes());
        p.extend_from_slice(&(self.w as u16).to_be_bytes());
        p.extend_from_slice(&(self.h as u16).to_be_bytes());
        p.extend_from_slice(&self.px);
        p
    }
}

fn push_event(buf: &mut Vec<u8>, t_ms: u32, kind: u8, payload: &[u8]) {
    buf.extend_from_slice(&t_ms.to_le_bytes());
    buf.push(kind);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
}

/// The chunk being filled.
struct Pending {
    buf: Vec<u8>,
    flags: u8,
    t_ms: u32,
}

pub(super) fn write_loop(rx: mpsc::Receiver<Cmd>, mut w: ChunkWriter, queued: Arc<AtomicUsize>, done: Finisher) {
    (done.audit)("RECORDING_STARTED", "session recording started".into());
    let started = done.started;
    let mut screen = Screen::default();
    let mut cur = Pending { buf: Vec::new(), flags: 0, t_ms: 0 };
    let mut last_flush = Instant::now();
    let mut last_seek = Instant::now();
    let mut bytes_since_seek = 0usize;
    let mut failed = false;

    let flush = |w: &mut ChunkWriter, cur: &mut Pending, failed: &mut bool| {
        if !cur.buf.is_empty() && !*failed {
            if let Err(e) = w.write_chunk_at(&cur.buf, cur.flags, cur.t_ms) {
                warn!("Recording {}: cannot write to {}: {e}", done.id, done.path.display());
                *failed = true;
            }
        }
        cur.buf.clear();
        cur.flags = 0;
    };

    loop {
        let wait = Duration::from_secs(CHUNK_SECS).saturating_sub(last_flush.elapsed());
        let msg = rx.recv_timeout(wait);
        let stop = matches!(msg, Ok(Cmd::Finish) | Err(mpsc::RecvTimeoutError::Disconnected));
        let now_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let mut seek_now = false;
        match msg {
            Ok(Cmd::Init(wd, ht)) if wd > 0 && ht > 0 => {
                screen.resize(wd, ht);
                seek_now = true;
            }
            Ok(Cmd::Packet(p)) => {
                queued.fetch_sub(p.len(), Ordering::Relaxed);
                if screen.w > 0 {
                    screen.apply(&p);
                    if cur.buf.is_empty() {
                        cur.t_ms = now_ms;
                    }
                    push_event(&mut cur.buf, now_ms, EV_PICTURE, &p);
                    bytes_since_seek += p.len();
                    seek_now = last_seek.elapsed() >= Duration::from_secs(SEEK_SECS) || bytes_since_seek >= SEEK_BYTES;
                }
            }
            _ => {}
        }
        if seek_now && !stop {
            // Close the current chunk, then start a seek point: size + the whole screen.
            flush(&mut w, &mut cur, &mut failed);
            last_flush = Instant::now();
            cur.flags = FLAG_SEEK;
            cur.t_ms = now_ms;
            push_event(&mut cur.buf, now_ms, EV_SIZE, &[(screen.w as u16).to_le_bytes(), (screen.h as u16).to_le_bytes()].concat());
            push_event(&mut cur.buf, now_ms, EV_PICTURE, &screen.full_picture());
            last_seek = Instant::now();
            bytes_since_seek = 0;
        }
        if stop || cur.buf.len() >= CHUNK_BYTES || last_flush.elapsed() >= Duration::from_secs(CHUNK_SECS) {
            flush(&mut w, &mut cur, &mut failed);
            last_flush = Instant::now();
        }
        if stop {
            break;
        }
    }
    done.finish(w, failed);
}

fn read_header(f: &mut std::fs::File) -> Option<(usize, u8, u32)> {
    let mut h = [0u8; 9];
    f.read_exact(&mut h).ok()?;
    let len = u32::from_le_bytes(h[0..4].try_into().unwrap()) as usize;
    if !(12..=MAX_CHUNK).contains(&len) {
        return None;
    }
    Some((len, h[4], u32::from_le_bytes(h[5..9].try_into().unwrap())))
}

fn open_v2(path: &Path) -> Result<std::fs::File, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("Cannot read the recording file: {e}"))?;
    let mut magic = [0u8; 10];
    if f.read_exact(&mut magic).is_err() || magic != MAGIC {
        return Err("Not a desktop recording".into());
    }
    Ok(f)
}

/// The seek points of a desktop recording (read without the key).
pub fn read_index(path: &Path) -> Result<Vec<Segment>, String> {
    let mut f = open_v2(path)?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut out = Vec::new();
    let mut chunk = 0u64;
    let mut offset = MAGIC.len() as u64;
    while let Some((len, flags, t_ms)) = read_header(&mut f) {
        if offset + 9 + len as u64 > size {
            break; // cut off while writing
        }
        if flags & FLAG_SEEK != 0 {
            out.push(Segment { t_ms, chunk, offset });
        }
        offset += 9 + len as u64;
        if f.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }
        chunk += 1;
    }
    Ok(out)
}

/// One piece of a desktop recording, from seek point `n` up to the next: the decrypted
/// chunks, still compressed, each as `[u32 length][deflate data]` (the browser unpacks them).
pub fn read_segment(path: &Path, key_enc: &str, master_secret: &str, n: usize) -> Result<Vec<u8>, String> {
    let index = read_index(path)?;
    let seg = index.get(n).ok_or("No such part of the recording")?;
    let cipher = unlock(key_enc, master_secret)?;
    let mut f = open_v2(path)?;
    f.seek(SeekFrom::Start(seg.offset)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut chunk = seg.chunk;
    while let Some((len, flags, t_ms)) = read_header(&mut f) {
        if chunk != seg.chunk && flags & FLAG_SEEK != 0 {
            break;
        }
        let mut body = vec![0u8; len];
        if f.read_exact(&mut body).is_err() {
            break;
        }
        let (nonce, sealed) = body.split_at(12);
        let aad = chunk_aad(chunk, Some((flags, t_ms)));
        match cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: &aad }) {
            Ok(packed) => {
                out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
                out.extend_from_slice(&packed);
            }
            Err(_) if chunk == seg.chunk => return Err("This recording cannot be unlocked".into()),
            Err(_) => break,
        }
        chunk += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::recording::{Recorder, RecordingStart};
    use flate2::read::DeflateDecoder;

    fn tile(x: u16, y: u16, w: u16, h: u16, v: u8) -> Vec<u8> {
        let mut p = vec![0x01];
        for n in [x, y, w, h] {
            p.extend_from_slice(&n.to_be_bytes());
        }
        p.extend(std::iter::repeat([v, v, v, 255]).take(w as usize * h as usize).flatten());
        p
    }

    #[test]
    fn screen_follows_updates() {
        let mut s = Screen::default();
        s.resize(4, 2);
        s.apply(&tile(0, 0, 2, 1, 9));
        assert_eq!(&s.px[0..8], &[9, 9, 9, 255, 9, 9, 9, 255]);
        // Copy the 2x1 area at (0,0) to (2,1).
        let mut c = vec![0x02];
        for n in [2u16, 1, 2, 1, 0, 0] {
            c.extend_from_slice(&n.to_be_bytes());
        }
        s.apply(&c);
        let at = (4 + 2) * 4;
        assert_eq!(&s.px[at..at + 8], &[9, 9, 9, 255, 9, 9, 9, 255]);
        // Updates reaching past the edge are cut, not a crash.
        s.apply(&tile(3, 1, 5, 5, 7));
        assert_eq!(s.px[(4 + 3) * 4], 7);
        assert_eq!(s.full_picture().len(), 9 + 4 * 2 * 4);
    }

    #[test]
    fn desktop_recording_has_seek_points_and_pieces() {
        let dir = std::env::temp_dir().join(format!("remote-grec-test-{}", uuid::Uuid::new_v4()));
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
                connection_name: "desk".into(),
                protocol: "rdp".into(),
                graphics: true,
                cols: 0,
                rows: 0,
            },
            Box::new(|_, _| {}),
        )
        .unwrap();
        rec.screen_size(8, 8);
        rec.picture(&tile(0, 0, 4, 4, 200));
        rec.picture(&[0x10, 1, 2, 3]); // sound: not recorded
        rec.screen_size(16, 8); // resize: a second seek point
        rec.picture(&tile(8, 0, 8, 8, 100));
        rec.finish();
        drop(rec);

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
        assert_eq!(r.kind, "graphics");
        let path = dir.join(&r.file_name);
        let index = read_index(&path).unwrap();
        assert_eq!(index.len(), 2);

        // Unpack the second piece: size 16x8, full picture, then the update.
        let packed = read_segment(&path, &r.key_enc, secret, 1).unwrap();
        let mut plain = Vec::new();
        let mut at = 0;
        while at + 4 <= packed.len() {
            let len = u32::from_le_bytes(packed[at..at + 4].try_into().unwrap()) as usize;
            DeflateDecoder::new(&packed[at + 4..at + 4 + len]).read_to_end(&mut plain).unwrap();
            at += 4 + len;
        }
        let mut kinds = Vec::new();
        let mut p = 0;
        while p + 9 <= plain.len() {
            let len = u32::from_le_bytes(plain[p + 5..p + 9].try_into().unwrap()) as usize;
            kinds.push((plain[p + 4], len));
            p += 9 + len;
        }
        assert_eq!(kinds[0], (EV_SIZE, 4));
        assert_eq!(kinds[1], (EV_PICTURE, 9 + 16 * 8 * 4));
        assert_eq!(kinds[2], (EV_PICTURE, 9 + 8 * 8 * 4));
        assert_eq!(kinds.len(), 3);
        assert!(read_segment(&path, &r.key_enc, "wrong-secret-000000000000000000", 0).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
