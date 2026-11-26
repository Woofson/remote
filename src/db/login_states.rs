//! Ended and locked logins, kept so that a server restart does not bring a signed-out
//! login back to life or open a locked screen (see `crate::auth::policy::SessionTracker`).

use super::Database;
use chrono::Utc;
use rusqlite::{params, Result as SqlResult};

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS login_states (
        sid TEXT PRIMARY KEY,
        state TEXT NOT NULL,
        changed_at INTEGER NOT NULL
    );
";

impl Database {
    /// Saved login states changed within `max_age_secs`; older ones are removed (no login
    /// lives that long, so they cannot matter any more).
    pub fn load_login_states(&self, max_age_secs: i64) -> SqlResult<Vec<(String, String)>> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM login_states WHERE changed_at < ?1", params![Utc::now().timestamp() - max_age_secs])?;
        let mut stmt = conn.prepare("SELECT sid, state FROM login_states")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Remember a login's state ("locked", "expired", "logged_out"), or forget it (`None`).
    pub fn set_login_state(&self, sid: &str, state: Option<&str>) -> SqlResult<()> {
        let conn = self.conn.lock();
        match state {
            Some(s) => conn.execute(
                "INSERT INTO login_states (sid, state, changed_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(sid) DO UPDATE SET state = excluded.state, changed_at = excluded.changed_at",
                params![sid, s, Utc::now().timestamp()],
            )?,
            None => conn.execute("DELETE FROM login_states WHERE sid = ?1", params![sid])?,
        };
        Ok(())
    }
}
