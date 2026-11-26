//! Devices remembered for two-factor sign-in: a browser that passed the code step with
//! "remember this device" may skip the code at sign-in and unlock for the number of days
//! the admin set. Only a fingerprint (SHA-256) of the browser's random device key is kept.

use super::Database;
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Result as SqlResult};
use serde::Serialize;
use uuid::Uuid;

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS trusted_devices (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        key_hash TEXT NOT NULL UNIQUE,
        label TEXT NOT NULL,
        session_generation INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        last_used_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_trusted_devices_user ON trusted_devices(user_id);
";

/// One remembered device, as shown in the lists (times are Unix seconds).
#[derive(Debug, Clone, Serialize)]
pub struct TrustedDevice {
    pub id: String,
    pub label: String,
    pub created_at: i64,
    pub last_used_at: i64,
    pub expires_at: i64,
}

impl Database {
    /// Remember a device; returns its id.
    pub fn add_trusted_device(&self, user_id: &str, key_hash: &str, label: &str, generation: i64) -> SqlResult<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        self.conn.lock().execute(
            "INSERT INTO trusted_devices (id, user_id, key_hash, label, session_generation, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, user_id, key_hash, label, generation, now],
        )?;
        Ok(id)
    }

    /// Is this device key remembered for the user, within `days` of being added and since
    /// the last password change / ended logins? Marks it used when it is.
    pub fn use_trusted_device(&self, user_id: &str, key_hash: &str, days: i64, generation: i64) -> SqlResult<bool> {
        if days <= 0 {
            return Ok(false);
        }
        let now = Utc::now().timestamp();
        let conn = self.conn.lock();
        let id: Option<String> = conn
            .query_row(
                "SELECT id FROM trusted_devices
                 WHERE user_id = ?1 AND key_hash = ?2 AND session_generation = ?3 AND created_at + ?4 > ?5",
                params![user_id, key_hash, generation, days * 86400, now],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => {
                conn.execute("UPDATE trusted_devices SET last_used_at = ?2 WHERE id = ?1", params![id, now])?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// A user's remembered devices that still work, newest first. Ones that no longer can
    /// (ran out, or from before a password change) are removed on the way.
    pub fn list_trusted_devices(&self, user_id: &str, days: i64, generation: i64) -> SqlResult<Vec<TrustedDevice>> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM trusted_devices WHERE user_id = ?1 AND (session_generation != ?2 OR created_at + ?3 <= ?4)",
            params![user_id, generation, days.max(0) * 86400, now],
        )?;
        let mut stmt = conn.prepare(
            "SELECT id, label, created_at, last_used_at FROM trusted_devices WHERE user_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            let created_at: i64 = r.get(2)?;
            Ok(TrustedDevice {
                id: r.get(0)?,
                label: r.get(1)?,
                created_at,
                last_used_at: r.get(3)?,
                expires_at: created_at + days * 86400,
            })
        })?;
        rows.collect()
    }

    /// Forget one of a user's devices; false when there was no such device.
    pub fn delete_trusted_device(&self, user_id: &str, id: &str) -> SqlResult<bool> {
        let n = self
            .conn
            .lock()
            .execute("DELETE FROM trusted_devices WHERE id = ?1 AND user_id = ?2", params![id, user_id])?;
        Ok(n > 0)
    }

    /// Forget all of a user's devices; returns how many there were.
    pub fn delete_trusted_devices_for_user(&self, user_id: &str) -> SqlResult<usize> {
        self.conn.lock().execute("DELETE FROM trusted_devices WHERE user_id = ?1", params![user_id])
    }

    /// Forget every remembered device (the admin turned the feature off).
    pub fn delete_all_trusted_devices(&self) -> SqlResult<usize> {
        self.conn.lock().execute("DELETE FROM trusted_devices", [])
    }
}
