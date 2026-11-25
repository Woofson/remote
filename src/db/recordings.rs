//! Session recordings (#31): the list of recordings, who they are shared with, and the
//! admin switches that decide which sessions are recorded. The recordings themselves are
//! encrypted files in the recordings folder (see `crate::recording`).

use super::{Database, Reencrypt};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Result as SqlResult, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS recordings (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        username TEXT NOT NULL,
        connection_id TEXT,
        connection_name TEXT NOT NULL,
        protocol TEXT NOT NULL,
        kind TEXT NOT NULL,
        started_at TEXT NOT NULL,
        ended_at TEXT,
        duration_ms INTEGER NOT NULL DEFAULT 0,
        size_bytes INTEGER NOT NULL DEFAULT 0,
        status TEXT NOT NULL,
        file_name TEXT NOT NULL,
        key_enc TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_recordings_user ON recordings(user_id);
    CREATE INDEX IF NOT EXISTS idx_recordings_started ON recordings(started_at);

    CREATE TABLE IF NOT EXISTS recording_shares (
        id TEXT PRIMARY KEY,
        recording_id TEXT NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
        user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
        group_id TEXT REFERENCES groups(id) ON DELETE CASCADE,
        allow_download INTEGER NOT NULL DEFAULT 0,
        created_by TEXT,
        created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_recording_shares_rec ON recording_shares(recording_id);
";

/// One recording. The key that unlocks its file is stored encrypted with the master secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingRecord {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub connection_id: Option<String>,
    pub connection_name: String,
    pub protocol: String,
    /// "terminal" or "graphics".
    pub kind: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: i64,
    pub size_bytes: i64,
    /// "recording", "done" or "interrupted" (the server stopped while it was running).
    pub status: String,
    #[serde(skip_serializing)]
    pub file_name: String,
    #[serde(skip_serializing)]
    pub key_enc: String,
}

/// Someone a recording is shared with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingShare {
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    /// Username or group name, for display.
    #[serde(default)]
    pub name: Option<String>,
    pub allow_download: bool,
}

const COLUMNS: &str = "r.id, r.user_id, r.username, r.connection_id, r.connection_name, r.protocol, r.kind, \
    r.started_at, r.ended_at, r.duration_ms, r.size_bytes, r.status, r.file_name, r.key_enc";

fn from_row(row: &Row) -> SqlResult<RecordingRecord> {
    Ok(RecordingRecord {
        id: row.get(0)?,
        user_id: row.get(1)?,
        username: row.get(2)?,
        connection_id: row.get(3)?,
        connection_name: row.get(4)?,
        protocol: row.get(5)?,
        kind: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        duration_ms: row.get(9)?,
        size_bytes: row.get(10)?,
        status: row.get(11)?,
        file_name: row.get(12)?,
        key_enc: row.get(13)?,
    })
}

impl Database {
    // ---- which sessions are recorded ----

    /// Admin switch on a connection. Returns false when the connection does not exist.
    pub fn set_connection_recording(&self, connection_id: &str, on: bool) -> SqlResult<bool> {
        let n = self.conn.lock().execute(
            "UPDATE connections SET record_sessions = ?1 WHERE id = ?2",
            params![on as i64, connection_id],
        )?;
        Ok(n > 0)
    }

    /// Is recording switched on for this connection itself?
    pub fn connection_records_sessions(&self, connection_id: &str) -> SqlResult<bool> {
        let v: Option<i64> = self
            .conn
            .lock()
            .query_row("SELECT record_sessions FROM connections WHERE id = ?1", params![connection_id], |r| r.get(0))
            .optional()?;
        Ok(v.is_some_and(|v| v != 0))
    }

    /// Admin switch on a group. Returns false when the group does not exist.
    pub fn set_group_recording(&self, group_id: &str, on: bool) -> SqlResult<bool> {
        let n = self
            .conn
            .lock()
            .execute("UPDATE groups SET record_sessions = ?1 WHERE id = ?2", params![on as i64, group_id])?;
        Ok(n > 0)
    }

    /// A session is recorded when its connection says so, or when the user is in a group that does.
    pub fn session_recording_required(&self, connection_id: &str, user_id: &str) -> SqlResult<bool> {
        let conn = self.conn.lock();
        let by_conn: bool = conn
            .query_row("SELECT record_sessions FROM connections WHERE id = ?1", params![connection_id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .is_some_and(|v| v != 0);
        if by_conn {
            return Ok(true);
        }
        let by_group: i64 = conn.query_row(
            "SELECT COUNT(*) FROM user_groups ug JOIN groups g ON g.id = ug.group_id
             WHERE ug.user_id = ?1 AND g.record_sessions = 1",
            params![user_id],
            |r| r.get(0),
        )?;
        Ok(by_group > 0)
    }

    // ---- recordings ----

    pub fn create_recording(&self, r: &RecordingRecord) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO recordings (id, user_id, username, connection_id, connection_name, protocol, kind,
                started_at, ended_at, duration_ms, size_bytes, status, file_name, key_enc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                r.id,
                r.user_id,
                r.username,
                r.connection_id,
                r.connection_name,
                r.protocol,
                r.kind,
                r.started_at,
                r.ended_at,
                r.duration_ms,
                r.size_bytes,
                r.status,
                r.file_name,
                r.key_enc
            ],
        )?;
        Ok(())
    }

    pub fn finish_recording(&self, id: &str, ended_at: &str, duration_ms: i64, size_bytes: i64, status: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "UPDATE recordings SET ended_at = ?1, duration_ms = ?2, size_bytes = ?3, status = ?4 WHERE id = ?5",
            params![ended_at, duration_ms, size_bytes, status, id],
        )?;
        Ok(())
    }

    pub fn get_recording(&self, id: &str) -> SqlResult<Option<RecordingRecord>> {
        let conn = self.conn.lock();
        conn.query_row(&format!("SELECT {COLUMNS} FROM recordings r WHERE r.id = ?1"), params![id], from_row)
            .optional()
    }

    /// Every recording, newest first.
    pub fn list_recordings(&self) -> SqlResult<Vec<RecordingRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM recordings r ORDER BY r.started_at DESC"))?;
        let rows = stmt.query_map([], from_row)?;
        rows.collect()
    }

    /// A user's own recordings (`None`) and those shared with them (`Some(allow_download)`), newest first.
    pub fn list_recordings_for_user(&self, user_id: &str) -> SqlResult<Vec<(RecordingRecord, Option<bool>)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS},
                    CASE WHEN r.user_id = ?1 THEN NULL ELSE
                        (SELECT MAX(s.allow_download) FROM recording_shares s
                         LEFT JOIN user_groups ug ON ug.group_id = s.group_id
                         WHERE s.recording_id = r.id AND (s.user_id = ?1 OR ug.user_id = ?1))
                    END
             FROM recordings r
             WHERE r.user_id = ?1
                OR r.id IN (SELECT s.recording_id FROM recording_shares s
                            LEFT JOIN user_groups ug ON ug.group_id = s.group_id
                            WHERE s.user_id = ?1 OR ug.user_id = ?1)
             ORDER BY r.started_at DESC"
        ))?;
        let rows = stmt.query_map(params![user_id], |row| {
            let rec = from_row(row)?;
            let share: Option<i64> = row.get(14)?;
            Ok((rec, share.map(|v| v != 0)))
        })?;
        rows.collect()
    }

    /// Shared with this user (directly or through a group)? `Some(allow_download)` if so.
    pub fn recording_share_access(&self, recording_id: &str, user_id: &str) -> SqlResult<Option<bool>> {
        let conn = self.conn.lock();
        let v: Option<i64> = conn.query_row(
            "SELECT MAX(s.allow_download) FROM recording_shares s
             LEFT JOIN user_groups ug ON ug.group_id = s.group_id
             WHERE s.recording_id = ?1 AND (s.user_id = ?2 OR ug.user_id = ?2)",
            params![recording_id, user_id],
            |r| r.get(0),
        )?;
        Ok(v.map(|v| v != 0))
    }

    pub fn list_recording_shares(&self, recording_id: &str) -> SqlResult<Vec<RecordingShare>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT s.user_id, s.group_id, COALESCE(u.username, g.name), s.allow_download
             FROM recording_shares s
             LEFT JOIN users u ON u.id = s.user_id
             LEFT JOIN groups g ON g.id = s.group_id
             WHERE s.recording_id = ?1
             ORDER BY s.group_id IS NOT NULL, 3",
        )?;
        let rows = stmt.query_map(params![recording_id], |r| {
            Ok(RecordingShare {
                user_id: r.get(0)?,
                group_id: r.get(1)?,
                name: r.get(2)?,
                allow_download: r.get::<_, i64>(3)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn replace_recording_shares(&self, recording_id: &str, shares: &[RecordingShare], created_by: &str) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM recording_shares WHERE recording_id = ?1", params![recording_id])?;
        let now = Utc::now().to_rfc3339();
        for s in shares {
            tx.execute(
                "INSERT INTO recording_shares (id, recording_id, user_id, group_id, allow_download, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    Uuid::new_v4().to_string(),
                    recording_id,
                    s.user_id,
                    s.group_id,
                    s.allow_download as i64,
                    created_by,
                    now
                ],
            )?;
        }
        tx.commit()
    }

    pub fn delete_recording(&self, id: &str) -> SqlResult<bool> {
        let n = self.conn.lock().execute("DELETE FROM recordings WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// Recordings still marked as running (only meaningful at startup: the server stopped mid-way).
    pub fn unfinished_recordings(&self) -> SqlResult<Vec<RecordingRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM recordings r WHERE r.status = 'recording'"))?;
        let rows = stmt.query_map([], from_row)?;
        rows.collect()
    }

    /// Re-encrypt recording keys after the master secret changed. Returns (converted, unreadable).
    pub fn reencrypt_recording_keys<F>(&self, convert: F) -> SqlResult<(usize, usize)>
    where
        F: Fn(&str) -> Reencrypt,
    {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let rows: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id, key_enc FROM recordings")?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<SqlResult<_>>()?
        };
        let (mut converted, mut unreadable) = (0, 0);
        for (id, enc) in rows {
            match convert(&enc) {
                Reencrypt::Keep => {}
                Reencrypt::Replace(new_enc) => {
                    tx.execute("UPDATE recordings SET key_enc = ?1 WHERE id = ?2", params![new_enc, id])?;
                    converted += 1;
                }
                Reencrypt::Unreadable => unreadable += 1,
            }
        }
        tx.commit()?;
        Ok((converted, unreadable))
    }
}
