//! Connection pools (#39): which machine of a pool a user was last sent to, so a returning
//! user can be sent back to the same one ("keep me on the same machine").

use super::Database;
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Result as SqlResult};

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS pool_sticky (
        pool_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        updated_at TEXT NOT NULL,
        PRIMARY KEY (pool_id, user_id)
    );
";

impl Database {
    /// The pool member this user was last sent to.
    pub fn pool_sticky_member(&self, pool_id: &str, user_id: &str) -> SqlResult<Option<String>> {
        self.conn
            .lock()
            .query_row(
                "SELECT connection_id FROM pool_sticky WHERE pool_id = ?1 AND user_id = ?2",
                params![pool_id, user_id],
                |r| r.get(0),
            )
            .optional()
    }

    pub fn set_pool_sticky_member(&self, pool_id: &str, user_id: &str, connection_id: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO pool_sticky (pool_id, user_id, connection_id, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(pool_id, user_id) DO UPDATE SET connection_id = excluded.connection_id, updated_at = excluded.updated_at",
            params![pool_id, user_id, connection_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
}
