use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{params, Connection as SqliteConnection, OptionalExtension, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

mod login_states;
mod recordings;
mod trusted_devices;
pub use recordings::{RecordingRecord, RecordingShare};
pub use trusted_devices::TrustedDevice;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: String, // "admin", "operator", "viewer"
    pub is_active: bool,
    pub auth_provider: String, // "local", "oidc", "pam"
    pub oidc_sub: Option<String>,
    pub created_at: String,
    pub last_login: Option<String>,
    pub avatar_data: Option<String>,
}

/// Stored two-factor state for one user.
#[derive(Debug, Clone, Default)]
pub struct TwoFactorRecord {
    pub secret_enc: Option<String>,
    pub pending_enc: Option<String>,
    pub last_step: i64,
    pub recovery_hashes: Vec<String>,
}

impl TwoFactorRecord {
    pub fn enabled(&self) -> bool {
        self.secret_enc.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSafe {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: String,
    pub is_active: bool,
    pub auth_provider: String,
    pub created_at: String,
    pub last_login: Option<String>,
    pub avatar_data: Option<String>,
    pub groups: Vec<String>,
}

impl From<User> for UserSafe {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            email: u.email,
            display_name: u.display_name,
            role: u.role,
            is_active: u.is_active,
            auth_provider: u.auth_provider,
            created_at: u.created_at,
            last_login: u.last_login,
            avatar_data: u.avatar_data,
            groups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    /// Sessions of members are recorded (#31).
    #[serde(default)]
    pub record_sessions: bool,
    /// Directory group (#34) whose members are kept in this group at each directory sign-in.
    #[serde(default)]
    pub directory_group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionRecord {
    pub id: String,
    pub name: String,
    pub protocol: String, // "ssh", "vnc", "rdp", "local_pty", "telnet"
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    #[serde(skip_serializing)]
    pub password_enc: Option<String>,
    #[serde(skip_serializing)]
    pub private_key_enc: Option<String>,
    pub settings_json: String,
    pub icon: Option<String>,
    pub tags: Option<String>,
    pub is_global: bool,
    pub allow_clipboard: String, // "bidirectional", "host_to_remote", "remote_to_host", "disabled"
    pub allow_transfer: String,  // "full", "upload_only", "download_only", "disabled"
    pub view_only: bool,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSafe {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub has_password: bool,
    pub has_private_key: bool,
    pub settings_json: String,
    pub icon: Option<String>,
    pub tags: Option<String>,
    pub is_global: bool,
    pub allow_clipboard: String,
    pub allow_transfer: String,
    pub view_only: bool,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub user_permissions: ConnectionUserPerms,
    /// Why this user sees the connection: "admin", "own", "assigned", "group" or "global".
    pub access: String,
    /// For access "group": the group name.
    pub access_group: Option<String>,
    /// Admins only: owner's username and the names of users/groups it is assigned to.
    pub owner_name: Option<String>,
    pub assigned_to: Vec<String>,
    /// Sessions on this connection are recorded (#31).
    #[serde(default)]
    pub record_sessions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionUserPerms {
    pub can_connect: bool,
    pub can_edit: bool,
    pub can_clipboard: bool,
    pub can_transfer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPermission {
    pub id: String,
    pub connection_id: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    /// Username or group name, for display.
    pub name: Option<String>,
    pub can_connect: bool,
    pub can_edit: bool,
    pub can_clipboard: bool,
    pub can_transfer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub user_id: Option<String>,
    pub username: String,
    pub action: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub details: Option<String>,
    pub ip_address: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferLog {
    pub id: String,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
    pub filename: String,
    pub filesize: u64,
    pub direction: String, // "upload", "download"
    pub status: String,    // "in_progress", "completed", "failed"
    pub timestamp: String,
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<SqliteConnection>>,
}

/// Outcome for one saved credential in [`Database::reencrypt_connection_secrets`].
pub enum Reencrypt {
    /// Already readable with the current secret.
    Keep,
    /// Replace with this newly encrypted value.
    Replace(String),
    /// Cannot be unlocked with any known secret.
    Unreadable,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> SqlResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = SqliteConnection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn new_in_memory() -> SqlResult<Self> {
        let conn = SqliteConnection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;",
        )?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                email TEXT,
                display_name TEXT,
                role TEXT NOT NULL DEFAULT 'operator',
                is_active INTEGER NOT NULL DEFAULT 1,
                auth_provider TEXT NOT NULL DEFAULT 'local',
                oidc_sub TEXT UNIQUE,
                created_at TEXT NOT NULL,
                last_login TEXT,
                avatar_data TEXT
            );

            CREATE TABLE IF NOT EXISTS groups (
                id TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                description TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS user_groups (
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
                PRIMARY KEY (user_id, group_id)
            );

            CREATE TABLE IF NOT EXISTS connections (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                protocol TEXT NOT NULL,
                host TEXT NOT NULL,
                port INTEGER NOT NULL,
                username TEXT,
                password_enc TEXT,
                private_key_enc TEXT,
                settings_json TEXT NOT NULL DEFAULT '{}',
                icon TEXT,
                tags TEXT,
                is_global INTEGER NOT NULL DEFAULT 1,
                allow_clipboard TEXT NOT NULL DEFAULT 'bidirectional',
                allow_transfer TEXT NOT NULL DEFAULT 'full',
                view_only INTEGER NOT NULL DEFAULT 0,
                created_by TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS connection_permissions (
                id TEXT PRIMARY KEY,
                connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
                user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
                group_id TEXT REFERENCES groups(id) ON DELETE CASCADE,
                can_connect INTEGER NOT NULL DEFAULT 1,
                can_edit INTEGER NOT NULL DEFAULT 0,
                can_clipboard INTEGER NOT NULL DEFAULT 1,
                can_transfer INTEGER NOT NULL DEFAULT 1,
                UNIQUE(connection_id, user_id, group_id)
            );

            CREATE TABLE IF NOT EXISTS audit_logs (
                id TEXT PRIMARY KEY,
                user_id TEXT,
                username TEXT NOT NULL,
                action TEXT NOT NULL,
                connection_id TEXT,
                connection_name TEXT,
                details TEXT,
                ip_address TEXT,
                timestamp TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS transfer_logs (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                user_id TEXT,
                filename TEXT NOT NULL,
                filesize INTEGER NOT NULL DEFAULT 0,
                direction TEXT NOT NULL,
                status TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS system_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_logs(timestamp);
            CREATE INDEX IF NOT EXISTS idx_connections_protocol ON connections(protocol);
            CREATE INDEX IF NOT EXISTS idx_connections_global ON connections(is_global);
            ",
        )?;

        // Portable migrations for existing SQLite databases
        let _ = conn.execute("ALTER TABLE users ADD COLUMN avatar_data TEXT;", []);
        let _ = conn.execute("UPDATE users SET display_name = 'admin' WHERE username = 'admin' AND display_name = 'Administrator';", []);
        let _ = conn.execute("UPDATE users SET email = 'admin@remote.local' WHERE username = 'admin' AND (email IS NULL OR email = '' OR email = 'admin@remotedog.local');", []);
        let _ = conn.execute("UPDATE users SET is_active = 1 WHERE username = 'admin';", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN is_global INTEGER NOT NULL DEFAULT 1;", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN allow_clipboard TEXT NOT NULL DEFAULT 'bidirectional';", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN allow_transfer TEXT NOT NULL DEFAULT 'full';", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN view_only INTEGER NOT NULL DEFAULT 0;", []);
        // Session recording (#31): set only through the admin calls, never by a plain connection save.
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN record_sessions INTEGER NOT NULL DEFAULT 0;", []);
        let _ = conn.execute("ALTER TABLE groups ADD COLUMN record_sessions INTEGER NOT NULL DEFAULT 0;", []);
        // Directory sign-in (#34): a Remote group may follow a directory group.
        let _ = conn.execute("ALTER TABLE groups ADD COLUMN directory_group TEXT;", []);
        conn.execute_batch(recordings::SCHEMA)?;
        conn.execute_batch(login_states::SCHEMA)?;
        conn.execute_batch(trusted_devices::SCHEMA)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_preferences (
                user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
                prefs TEXT NOT NULL DEFAULT '{}'
            )",
            [],
        )?;

        // Per-user login security, kept apart from `users` (same approach as Brum):
        // a newer session_generation ends older logins; must_change_password gates the account.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_security (
                user_id TEXT PRIMARY KEY,
                session_generation INTEGER NOT NULL DEFAULT 0,
                must_change_password INTEGER NOT NULL DEFAULT 0,
                password_changed_at TEXT
            )",
            [],
        )?;
        // Two-factor authentication (#26): encrypted TOTP secret, a pending secret during
        // setup, the last accepted time step (no code reuse) and hashed recovery codes.
        let _ = conn.execute("ALTER TABLE user_security ADD COLUMN totp_secret_enc TEXT", []);
        let _ = conn.execute("ALTER TABLE user_security ADD COLUMN totp_pending_enc TEXT", []);
        let _ = conn.execute("ALTER TABLE user_security ADD COLUMN totp_last_step INTEGER NOT NULL DEFAULT 0", []);
        let _ = conn.execute("ALTER TABLE user_security ADD COLUMN recovery_codes TEXT", []);
        // Password reset and set-your-password links (#27). Only a hash of each link's
        // secret is stored; a link works once and expires.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS password_resets (
                token_hash TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                purpose TEXT NOT NULL DEFAULT 'reset',
                created_at TEXT NOT NULL
            )",
            [],
        )?;

        Ok(())
    }

    pub fn seed_initial_admin(&self, password_hash: &str) -> SqlResult<bool> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
        if count == 0 {
            let id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO users (id, username, password_hash, email, display_name, role, is_active, auth_provider, created_at, avatar_data)
                 VALUES (?1, 'admin', ?2, 'admin@remote.local', 'admin', 'admin', 1, 'local', ?3, NULL)",
                params![id, password_hash, now],
            )?;

            // Seed an example connection (Local PTY)
            let conn_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO connections (id, name, protocol, host, port, username, settings_json, icon, tags, created_by, created_at, updated_at)
                 VALUES (?1, 'Local Host Terminal', 'local_pty', 'localhost', 0, 'system', '{\"shell\":\"default\"}', 'terminal', 'system,local', ?2, ?3, ?3)",
                params![conn_id, id, now],
            )?;

            conn.execute(
                "INSERT INTO user_security (user_id, must_change_password) VALUES (?1, 1)",
                params![id],
            )?;

            info!("Created default administrator user 'admin' and initial connection template");
            return Ok(true);
        }
        Ok(false)
    }

    // ================= Login Security =================

    /// (session_generation, must_change_password) for a user; defaults when no row exists.
    pub fn get_user_security(&self, user_id: &str) -> SqlResult<(i64, bool)> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT session_generation, must_change_password FROM user_security WHERE user_id = ?1",
                params![user_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        Ok(row.unwrap_or((0, false)))
    }

    /// A user's two-factor state; empty when they never set it up.
    pub fn get_two_factor(&self, user_id: &str) -> SqlResult<TwoFactorRecord> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT totp_secret_enc, totp_pending_enc, totp_last_step, recovery_codes
                 FROM user_security WHERE user_id = ?1",
                params![user_id],
                |r| {
                    let codes: Option<String> = r.get(3)?;
                    Ok(TwoFactorRecord {
                        secret_enc: r.get(0)?,
                        pending_enc: r.get(1)?,
                        last_step: r.get(2)?,
                        recovery_hashes: codes
                            .and_then(|c| serde_json::from_str(&c).ok())
                            .unwrap_or_default(),
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    /// Store a new secret that is not active until the user confirms a code.
    pub fn set_totp_pending(&self, user_id: &str, pending_enc: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO user_security (user_id, totp_pending_enc) VALUES (?1, ?2)
             ON CONFLICT(user_id) DO UPDATE SET totp_pending_enc = ?2",
            params![user_id, pending_enc],
        )?;
        Ok(())
    }

    /// Turn 2FA on with the confirmed secret and fresh recovery codes.
    pub fn enable_totp(&self, user_id: &str, secret_enc: &str, recovery_hashes: &[String], step: i64) -> SqlResult<()> {
        let codes = serde_json::to_string(recovery_hashes).unwrap_or_else(|_| "[]".into());
        self.conn.lock().execute(
            "INSERT INTO user_security (user_id, totp_secret_enc, totp_pending_enc, totp_last_step, recovery_codes)
             VALUES (?1, ?2, NULL, ?3, ?4)
             ON CONFLICT(user_id) DO UPDATE SET
                totp_secret_enc = ?2, totp_pending_enc = NULL, totp_last_step = ?3, recovery_codes = ?4",
            params![user_id, secret_enc, step, codes],
        )?;
        self.delete_trusted_devices_for_user(user_id)?;
        Ok(())
    }

    pub fn set_totp_last_step(&self, user_id: &str, step: i64) -> SqlResult<()> {
        self.conn.lock().execute(
            "UPDATE user_security SET totp_last_step = ?2 WHERE user_id = ?1",
            params![user_id, step],
        )?;
        Ok(())
    }

    pub fn set_recovery_codes(&self, user_id: &str, recovery_hashes: &[String]) -> SqlResult<()> {
        let codes = serde_json::to_string(recovery_hashes).unwrap_or_else(|_| "[]".into());
        self.conn.lock().execute(
            "UPDATE user_security SET recovery_codes = ?2 WHERE user_id = ?1",
            params![user_id, codes],
        )?;
        Ok(())
    }

    /// Turn 2FA off and forget the secret and recovery codes.
    pub fn disable_totp(&self, user_id: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "UPDATE user_security SET totp_secret_enc = NULL, totp_pending_enc = NULL,
                totp_last_step = 0, recovery_codes = NULL WHERE user_id = ?1",
            params![user_id],
        )?;
        self.delete_trusted_devices_for_user(user_id)?;
        Ok(())
    }

    /// IDs of users who have 2FA turned on.
    pub fn two_factor_user_ids(&self) -> SqlResult<std::collections::HashSet<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT user_id FROM user_security WHERE totp_secret_enc IS NOT NULL")?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<SqlResult<_>>()?;
        Ok(ids)
    }

    /// Store a new reset link for a user, replacing any older ones.
    pub fn create_password_reset(&self, token_hash: &str, user_id: &str, expires_at: i64, purpose: &str) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM password_resets WHERE user_id = ?1 OR expires_at < ?2", params![user_id, Utc::now().timestamp()])?;
        tx.execute(
            "INSERT INTO password_resets (token_hash, user_id, expires_at, purpose, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token_hash, user_id, expires_at, purpose, Utc::now().to_rfc3339()],
        )?;
        tx.commit()
    }

    /// The user a valid (unexpired) reset link belongs to, with the link's purpose.
    pub fn find_password_reset(&self, token_hash: &str) -> SqlResult<Option<(String, String)>> {
        self.conn
            .lock()
            .query_row(
                "SELECT user_id, purpose FROM password_resets WHERE token_hash = ?1 AND expires_at >= ?2",
                params![token_hash, Utc::now().timestamp()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
    }

    /// Forget all reset links of a user (after one is used, or a password change).
    pub fn delete_password_resets(&self, user_id: &str) -> SqlResult<()> {
        self.conn.lock().execute("DELETE FROM password_resets WHERE user_id = ?1", params![user_id])?;
        Ok(())
    }

    /// Whether the database answers (for the readiness check).
    pub fn ping(&self) -> bool {
        self.conn.lock().query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).is_ok()
    }

    /// A server-wide setting stored in the database (JSON text), if set.
    pub fn get_setting(&self, key: &str) -> SqlResult<Option<String>> {
        self.conn
            .lock()
            .query_row("SELECT value FROM system_settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO system_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    /// End every existing login of this user.
    pub fn bump_session_generation(&self, user_id: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO user_security (user_id, session_generation) VALUES (?1, 1)
             ON CONFLICT(user_id) DO UPDATE SET session_generation = session_generation + 1",
            params![user_id],
        )?;
        Ok(())
    }

    pub fn set_must_change_password(&self, user_id: &str, must_change: bool) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO user_security (user_id, must_change_password) VALUES (?1, ?2)
             ON CONFLICT(user_id) DO UPDATE SET must_change_password = ?2",
            params![user_id, must_change as i64],
        )?;
        Ok(())
    }

    /// Store a new password hash, end all existing logins and set the change-on-next-login flag.
    pub fn set_user_password(&self, user_id: &str, password_hash: &str, must_change: bool) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("UPDATE users SET password_hash = ?1 WHERE id = ?2", params![password_hash, user_id])?;
        tx.execute(
            "INSERT INTO user_security (user_id, session_generation, must_change_password, password_changed_at)
             VALUES (?1, 1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET
                session_generation = session_generation + 1,
                must_change_password = ?2,
                password_changed_at = ?3",
            params![user_id, must_change as i64, Utc::now().to_rfc3339()],
        )?;
        // A new password makes any outstanding reset link useless.
        tx.execute("DELETE FROM password_resets WHERE user_id = ?1", params![user_id])?;
        tx.commit()
    }

    /// A user's saved interface preferences (JSON object text).
    pub fn get_user_preferences(&self, user_id: &str) -> SqlResult<String> {
        let prefs = self
            .conn
            .lock()
            .query_row("SELECT prefs FROM user_preferences WHERE user_id = ?1", params![user_id], |r| r.get(0))
            .optional()?;
        Ok(prefs.unwrap_or_else(|| "{}".to_string()))
    }

    pub fn set_user_preferences(&self, user_id: &str, prefs: &str) -> SqlResult<()> {
        self.conn.lock().execute(
            "INSERT INTO user_preferences (user_id, prefs) VALUES (?1, ?2)
             ON CONFLICT(user_id) DO UPDATE SET prefs = ?2",
            params![user_id, prefs],
        )?;
        Ok(())
    }

    pub fn set_connection_global(&self, id: &str, is_global: bool) -> SqlResult<bool> {
        let n = self.conn.lock().execute(
            "UPDATE connections SET is_global = ?1, updated_at = ?2 WHERE id = ?3",
            params![is_global as i64, Utc::now().to_rfc3339(), id],
        )?;
        Ok(n > 0)
    }

    pub fn count_users(&self) -> SqlResult<i64> {
        self.conn.lock().query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
    }

    // ================= User Operations =================

    pub fn get_user_by_username(&self, username: &str) -> SqlResult<Option<User>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, email, display_name, role, is_active, auth_provider, oidc_sub, created_at, last_login, avatar_data 
             FROM users WHERE username = ?1 COLLATE NOCASE",
        )?;
        let mut rows = stmt.query(params![username])?;
        if let Some(row) = rows.next()? {
            Ok(Some(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                email: row.get(3)?,
                display_name: row.get(4)?,
                role: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                auth_provider: row.get(7)?,
                oidc_sub: row.get(8)?,
                created_at: row.get(9)?,
                last_login: row.get(10)?,
                avatar_data: row.get(11)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_user_by_id(&self, id: &str) -> SqlResult<Option<User>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, email, display_name, role, is_active, auth_provider, oidc_sub, created_at, last_login, avatar_data 
             FROM users WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                email: row.get(3)?,
                display_name: row.get(4)?,
                role: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                auth_provider: row.get(7)?,
                oidc_sub: row.get(8)?,
                created_at: row.get(9)?,
                last_login: row.get(10)?,
                avatar_data: row.get(11)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_user_by_oidc_sub(&self, sub: &str) -> SqlResult<Option<User>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, email, display_name, role, is_active, auth_provider, oidc_sub, created_at, last_login, avatar_data 
             FROM users WHERE oidc_sub = ?1",
        )?;
        let mut rows = stmt.query(params![sub])?;
        if let Some(row) = rows.next()? {
            Ok(Some(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                email: row.get(3)?,
                display_name: row.get(4)?,
                role: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                auth_provider: row.get(7)?,
                oidc_sub: row.get(8)?,
                created_at: row.get(9)?,
                last_login: row.get(10)?,
                avatar_data: row.get(11)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_users(&self) -> SqlResult<Vec<UserSafe>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, username, email, display_name, role, is_active, auth_provider, created_at, last_login, avatar_data 
             FROM users ORDER BY username ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UserSafe {
                id: row.get(0)?,
                username: row.get(1)?,
                email: row.get(2)?,
                display_name: row.get(3)?,
                role: row.get(4)?,
                is_active: row.get::<_, i64>(5)? != 0,
                auth_provider: row.get(6)?,
                created_at: row.get(7)?,
                last_login: row.get(8)?,
                avatar_data: row.get(9)?,
                groups: Vec::new(),
            })
        })?;

        let mut users = Vec::new();
        for u in rows {
            let mut user = u?;
            // fetch groups
            let mut g_stmt = conn.prepare(
                "SELECT g.name FROM groups g 
                 JOIN user_groups ug ON ug.group_id = g.id 
                 WHERE ug.user_id = ?1",
            )?;
            let g_rows = g_stmt.query_map(params![user.id], |r| r.get::<_, String>(0))?;
            for g in g_rows {
                user.groups.push(g?);
            }
            users.push(user);
        }
        Ok(users)
    }

    pub fn create_user(&self, user: &User) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO users (id, username, password_hash, email, display_name, role, is_active, auth_provider, oidc_sub, created_at, last_login, avatar_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                user.id,
                user.username,
                user.password_hash,
                user.email,
                user.display_name,
                user.role,
                if user.is_active { 1 } else { 0 },
                user.auth_provider,
                user.oidc_sub,
                user.created_at,
                user.last_login,
                user.avatar_data
            ],
        )?;
        Ok(())
    }

    pub fn update_user(&self, user: &User) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE users SET username = ?1, email = ?2, display_name = ?3, role = ?4, is_active = ?5, password_hash = ?6, avatar_data = ?7 WHERE id = ?8",
            params![
                user.username,
                user.email,
                user.display_name,
                user.role,
                if user.is_active { 1 } else { 0 },
                user.password_hash,
                user.avatar_data,
                user.id
            ],
        )?;
        Ok(())
    }

    pub fn delete_user(&self, id: &str) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM users WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM user_security WHERE user_id = ?1", params![id])?;
        Ok(())
    }

    pub fn update_last_login(&self, id: &str) -> SqlResult<()> {
        let conn = self.conn.lock();
        let now = Utc::now().to_rfc3339();
        conn.execute("UPDATE users SET last_login = ?1 WHERE id = ?2", params![now, id])?;
        Ok(())
    }

    // ================= Groups =================

    pub fn list_groups(&self) -> SqlResult<Vec<Group>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id, name, description, created_at, record_sessions, directory_group FROM groups ORDER BY name ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                record_sessions: row.get::<_, i64>(4)? != 0,
                directory_group: row.get::<_, Option<String>>(5)?.filter(|s| !s.trim().is_empty()),
            })
        })?;
        let mut list = Vec::new();
        for g in rows {
            list.push(g?);
        }
        Ok(list)
    }

    pub fn create_group(&self, name: &str, description: Option<&str>) -> SqlResult<Group> {
        let conn = self.conn.lock();
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO groups (id, name, description, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, description, now],
        )?;
        Ok(Group {
            id,
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            created_at: now,
            record_sessions: false,
            directory_group: None,
        })
    }

    pub fn rename_group(&self, id: &str, name: &str, description: Option<&str>) -> SqlResult<bool> {
        let n = self.conn.lock().execute(
            "UPDATE groups SET name = ?1, description = ?2 WHERE id = ?3",
            params![name, description, id],
        )?;
        Ok(n > 0)
    }

    /// Delete a group; its memberships and connection grants go with it (ON DELETE CASCADE).
    pub fn delete_group(&self, id: &str) -> SqlResult<bool> {
        let n = self.conn.lock().execute("DELETE FROM groups WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// group id -> member user ids
    pub fn group_members(&self) -> SqlResult<std::collections::HashMap<String, Vec<String>>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT group_id, user_id FROM user_groups")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for row in rows {
            let (g, u) = row?;
            map.entry(g).or_default().push(u);
        }
        Ok(map)
    }

    /// Link a group to a directory group (None removes the link); false when there is no such group.
    pub fn set_group_directory(&self, id: &str, directory_group: Option<&str>) -> SqlResult<bool> {
        let n = self
            .conn
            .lock()
            .execute("UPDATE groups SET directory_group = ?1 WHERE id = ?2", params![directory_group, id])?;
        Ok(n > 0)
    }

    /// Put a user in, or take them out of, these groups. Returns the groups that changed,
    /// as (group name, now a member).
    pub fn sync_user_groups(&self, user_id: &str, wanted: &[(String, String, bool)]) -> SqlResult<Vec<(String, bool)>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut changed = Vec::new();
        for (group_id, name, member) in wanted {
            let n = if *member {
                tx.execute("INSERT OR IGNORE INTO user_groups (user_id, group_id) VALUES (?1, ?2)", params![user_id, group_id])?
            } else {
                tx.execute("DELETE FROM user_groups WHERE user_id = ?1 AND group_id = ?2", params![user_id, group_id])?
            };
            if n > 0 {
                changed.push((name.clone(), *member));
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Replace a group's members in one transaction.
    pub fn set_group_members(&self, group_id: &str, user_ids: &[String]) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM user_groups WHERE group_id = ?1", params![group_id])?;
        for uid in user_ids {
            tx.execute(
                "INSERT OR IGNORE INTO user_groups (user_id, group_id) VALUES (?1, ?2)",
                params![uid, group_id],
            )?;
        }
        tx.commit()
    }

    // ================= Connections =================

    pub fn list_connections_for_user(&self, user: &User) -> SqlResult<Vec<ConnectionSafe>> {
        let conn = self.conn.lock();
        let is_admin = user.role == "admin";

        let mut stmt = conn.prepare(
            "SELECT id, name, protocol, host, port, username, 
                    (password_enc IS NOT NULL AND password_enc != '') as has_pw,
                    (private_key_enc IS NOT NULL AND private_key_enc != '') as has_key,
                    settings_json, icon, tags, is_global, allow_clipboard, allow_transfer, view_only,
                    created_by, created_at, updated_at, record_sessions
             FROM connections
             WHERE (?1 = 1) OR (is_global = 1) OR (created_by = ?2)
                OR (id IN (SELECT connection_id FROM connection_permissions cp 
                           LEFT JOIN user_groups ug ON ug.group_id = cp.group_id 
                           WHERE cp.user_id = ?2 OR ug.user_id = ?2))
             ORDER BY is_global DESC, name ASC",
        )?;

        let rows = stmt.query_map(params![if is_admin { 1 } else { 0 }, user.id], |row| {
            let conn_id: String = row.get(0)?;
            Ok((
                conn_id,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u16>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
                row.get::<_, i64>(7)? != 0,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, i64>(11)? != 0,
                row.get::<_, String>(12).unwrap_or_else(|_| "bidirectional".into()),
                row.get::<_, String>(13).unwrap_or_else(|_| "full".into()),
                row.get::<_, i64>(14)? != 0,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, String>(16)?,
                row.get::<_, String>(17)?,
                row.get::<_, i64>(18)? != 0,
            ))
        })?;

        let mut connections = Vec::new();
        for r in rows {
            let (
                id,
                name,
                protocol,
                host,
                port,
                username,
                has_pw,
                has_key,
                settings_json,
                icon,
                tags,
                is_global,
                allow_clipboard,
                allow_transfer,
                view_only,
                created_by,
                created_at,
                updated_at,
                record_sessions,
            ) = r?;

            let is_owner = created_by.as_deref() == Some(&user.id);
            let can_clipboard_val = allow_clipboard != "disabled";
            let can_transfer_val = allow_transfer != "disabled";
            let mut access = if is_admin { "admin" } else if is_owner { "own" } else { "global" };
            let mut access_group: Option<String> = None;

            let perms = if is_admin {
                ConnectionUserPerms {
                    can_connect: true,
                    can_edit: true,
                    can_clipboard: can_clipboard_val,
                    can_transfer: can_transfer_val,
                }
            } else if is_owner {
                ConnectionUserPerms {
                    can_connect: true,
                    can_edit: true,
                    can_clipboard: can_clipboard_val,
                    can_transfer: can_transfer_val,
                }
            } else {
                // Check granular permissions if defined
                let mut p_stmt = conn.prepare(
                    "SELECT cp.can_connect, cp.can_edit, cp.can_clipboard, cp.can_transfer, cp.user_id, g.name
                     FROM connection_permissions cp
                     LEFT JOIN user_groups ug ON ug.group_id = cp.group_id
                     LEFT JOIN groups g ON g.id = cp.group_id
                     WHERE cp.connection_id = ?1 AND (cp.user_id = ?2 OR ug.user_id = ?2)",
                )?;
                let mut p_rows = p_stmt.query(params![id, user.id])?;
                // Explicit grants decide; any grant allowing a capability allows it.
                // A global connection without grants is connectable by everyone.
                let mut user_perms = ConnectionUserPerms {
                    can_connect: false,
                    can_edit: false,
                    can_clipboard: false,
                    can_transfer: false,
                };
                let mut found = false;
                while let Some(p_row) = p_rows.next()? {
                    found = true;
                    if p_row.get::<_, Option<String>>(4)?.is_some() {
                        access = "assigned";
                    } else if access != "assigned" {
                        access = "group";
                        if access_group.is_none() {
                            access_group = p_row.get::<_, Option<String>>(5)?;
                        }
                    }
                    if p_row.get::<_, i64>(0)? != 0 { user_perms.can_connect = true; }
                    if p_row.get::<_, i64>(1)? != 0 { user_perms.can_edit = true; }
                    if p_row.get::<_, i64>(2)? != 0 { user_perms.can_clipboard = true; }
                    if p_row.get::<_, i64>(3)? != 0 { user_perms.can_transfer = true; }
                }

                if !found {
                    if !is_global {
                        continue;
                    }
                    user_perms.can_connect = true;
                    user_perms.can_clipboard = true;
                    user_perms.can_transfer = true;
                }
                // Connection-wide settings still apply on top of grants.
                user_perms.can_clipboard &= can_clipboard_val;
                user_perms.can_transfer &= can_transfer_val;
                user_perms
            };

            connections.push(ConnectionSafe {
                id,
                name,
                protocol,
                host,
                port,
                username,
                has_password: has_pw,
                has_private_key: has_key,
                settings_json,
                icon,
                tags,
                is_global,
                allow_clipboard,
                allow_transfer,
                view_only,
                created_by,
                created_at,
                updated_at,
                user_permissions: perms,
                access: access.to_string(),
                access_group: if access == "group" { access_group } else { None },
                owner_name: None,
                assigned_to: Vec::new(),
                record_sessions,
            });
        }

        if is_admin {
            for c in connections.iter_mut() {
                if let Some(owner) = &c.created_by {
                    c.owner_name = conn
                        .query_row("SELECT username FROM users WHERE id = ?1", params![owner], |r| r.get(0))
                        .optional()?;
                }
                let mut a_stmt = conn.prepare(
                    "SELECT COALESCE(u.username, 'Group: ' || g.name)
                     FROM connection_permissions cp
                     LEFT JOIN users u ON u.id = cp.user_id
                     LEFT JOIN groups g ON g.id = cp.group_id
                     WHERE cp.connection_id = ?1
                     ORDER BY cp.group_id IS NOT NULL, 1",
                )?;
                let names = a_stmt.query_map(params![c.id], |r| r.get::<_, Option<String>>(0))?;
                c.assigned_to = names.filter_map(|n| n.ok().flatten()).collect();
            }
        }

        Ok(connections)
    }

    /// All grants on a connection, with user/group names for display.
    pub fn list_connection_permissions(&self, connection_id: &str) -> SqlResult<Vec<ConnectionPermission>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT cp.id, cp.connection_id, cp.user_id, cp.group_id, COALESCE(u.username, g.name),
                    cp.can_connect, cp.can_edit, cp.can_clipboard, cp.can_transfer
             FROM connection_permissions cp
             LEFT JOIN users u ON u.id = cp.user_id
             LEFT JOIN groups g ON g.id = cp.group_id
             WHERE cp.connection_id = ?1
             ORDER BY cp.group_id IS NOT NULL, 5",
        )?;
        let rows = stmt.query_map(params![connection_id], |r| {
            Ok(ConnectionPermission {
                id: r.get(0)?,
                connection_id: r.get(1)?,
                user_id: r.get(2)?,
                group_id: r.get(3)?,
                name: r.get(4)?,
                can_connect: r.get::<_, i64>(5)? != 0,
                can_edit: r.get::<_, i64>(6)? != 0,
                can_clipboard: r.get::<_, i64>(7)? != 0,
                can_transfer: r.get::<_, i64>(8)? != 0,
            })
        })?;
        rows.collect()
    }

    /// Replace all grants on a connection in one transaction.
    pub fn replace_connection_permissions(
        &self,
        connection_id: &str,
        grants: &[(Option<String>, Option<String>, ConnectionUserPerms)],
    ) -> SqlResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM connection_permissions WHERE connection_id = ?1", params![connection_id])?;
        for (user_id, group_id, p) in grants {
            tx.execute(
                "INSERT INTO connection_permissions
                    (id, connection_id, user_id, group_id, can_connect, can_edit, can_clipboard, can_transfer)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    Uuid::new_v4().to_string(),
                    connection_id,
                    user_id,
                    group_id,
                    p.can_connect as i64,
                    p.can_edit as i64,
                    p.can_clipboard as i64,
                    p.can_transfer as i64
                ],
            )?;
        }
        tx.commit()
    }

    pub fn get_connection_raw(&self, id: &str) -> SqlResult<Option<ConnectionRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, protocol, host, port, username, password_enc, private_key_enc, 
                    settings_json, icon, tags, is_global, allow_clipboard, allow_transfer, view_only,
                    created_by, created_at, updated_at
             FROM connections WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ConnectionRecord {
                id: row.get::<_, String>(0)?,
                name: row.get::<_, String>(1)?,
                protocol: row.get::<_, String>(2)?,
                host: row.get::<_, String>(3)?,
                port: row.get::<_, u16>(4)?,
                username: row.get::<_, Option<String>>(5)?,
                password_enc: row.get::<_, Option<String>>(6)?,
                private_key_enc: row.get::<_, Option<String>>(7)?,
                settings_json: row.get::<_, String>(8)?,
                icon: row.get::<_, Option<String>>(9)?,
                tags: row.get::<_, Option<String>>(10)?,
                is_global: row.get::<_, i64>(11)? != 0,
                allow_clipboard: row.get::<_, String>(12).unwrap_or_else(|_| "bidirectional".into()),
                allow_transfer: row.get::<_, String>(13).unwrap_or_else(|_| "full".into()),
                view_only: row.get::<_, i64>(14)? != 0,
                created_by: row.get::<_, Option<String>>(15)?,
                created_at: row.get::<_, String>(16)?,
                updated_at: row.get::<_, String>(17)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Grant a user or a group access to a connection.
    pub fn add_connection_permission(
        &self,
        connection_id: &str,
        user_id: Option<&str>,
        group_id: Option<&str>,
        perms: &ConnectionUserPerms,
    ) -> SqlResult<String> {
        let id = Uuid::new_v4().to_string();
        self.conn.lock().execute(
            "INSERT INTO connection_permissions
                (id, connection_id, user_id, group_id, can_connect, can_edit, can_clipboard, can_transfer)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                connection_id,
                user_id,
                group_id,
                perms.can_connect as i64,
                perms.can_edit as i64,
                perms.can_clipboard as i64,
                perms.can_transfer as i64
            ],
        )?;
        Ok(id)
    }

    /// Re-encrypt saved connection passwords and keys in one transaction.
    /// Returns (converted, unreadable).
    pub fn reencrypt_connection_secrets<F>(&self, convert: F) -> SqlResult<(usize, usize)>
    where
        F: Fn(&str) -> Reencrypt,
    {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let rows: Vec<(String, Option<String>, Option<String>)> = {
            let mut stmt = tx.prepare("SELECT id, password_enc, private_key_enc FROM connections")?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            mapped.collect::<SqlResult<_>>()?
        };

        let (mut converted, mut unreadable) = (0, 0);
        for (id, pw, key) in rows {
            for (column, value) in [("password_enc", pw), ("private_key_enc", key)] {
                let Some(enc) = value.filter(|v| !v.is_empty()) else { continue };
                match convert(&enc) {
                    Reencrypt::Keep => {}
                    Reencrypt::Replace(new_enc) => {
                        tx.execute(
                            &format!("UPDATE connections SET {} = ?1 WHERE id = ?2", column),
                            params![new_enc, id],
                        )?;
                        converted += 1;
                    }
                    Reencrypt::Unreadable => unreadable += 1,
                }
            }
        }
        tx.commit()?;
        Ok((converted, unreadable))
    }

    pub fn save_connection(&self, record: &ConnectionRecord) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO connections (
                id, name, protocol, host, port, username, password_enc, private_key_enc,
                settings_json, icon, tags, is_global, allow_clipboard, allow_transfer, view_only,
                created_by, created_at, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                protocol = excluded.protocol,
                host = excluded.host,
                port = excluded.port,
                username = excluded.username,
                password_enc = COALESCE(excluded.password_enc, connections.password_enc),
                private_key_enc = COALESCE(excluded.private_key_enc, connections.private_key_enc),
                settings_json = excluded.settings_json,
                icon = excluded.icon,
                tags = excluded.tags,
                is_global = excluded.is_global,
                allow_clipboard = excluded.allow_clipboard,
                allow_transfer = excluded.allow_transfer,
                view_only = excluded.view_only,
                updated_at = excluded.updated_at",
            params![
                record.id,
                record.name,
                record.protocol,
                record.host,
                record.port,
                record.username,
                record.password_enc,
                record.private_key_enc,
                record.settings_json,
                record.icon,
                record.tags,
                if record.is_global { 1 } else { 0 },
                record.allow_clipboard,
                record.allow_transfer,
                if record.view_only { 1 } else { 0 },
                record.created_by,
                record.created_at,
                record.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_connection(&self, id: &str) -> SqlResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM connections WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ================= Audit & Logs =================

    pub fn log_audit(
        &self,
        user_id: Option<&str>,
        username: &str,
        action: &str,
        connection_id: Option<&str>,
        connection_name: Option<&str>,
        details: Option<&str>,
        ip_address: Option<&str>,
    ) {
        let conn = self.conn.lock();
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let _ = conn.execute(
            "INSERT INTO audit_logs (id, user_id, username, action, connection_id, connection_name, details, ip_address, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, user_id, username, action, connection_id, connection_name, details, ip_address, now],
        );
    }

    pub fn list_audit_logs(&self, limit: usize) -> SqlResult<Vec<AuditLog>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, user_id, username, action, connection_id, connection_name, details, ip_address, timestamp 
             FROM audit_logs ORDER BY timestamp DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(AuditLog {
                id: row.get(0)?,
                user_id: row.get(1)?,
                username: row.get(2)?,
                action: row.get(3)?,
                connection_id: row.get(4)?,
                connection_name: row.get(5)?,
                details: row.get(6)?,
                ip_address: row.get(7)?,
                timestamp: row.get(8)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn log_transfer(
        &self,
        session_id: Option<&str>,
        user_id: Option<&str>,
        filename: &str,
        filesize: u64,
        direction: &str,
        status: &str,
    ) {
        let conn = self.conn.lock();
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let _ = conn.execute(
            "INSERT INTO transfer_logs (id, session_id, user_id, filename, filesize, direction, status, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, session_id, user_id, filename, filesize as i64, direction, status, now],
        );
    }
}
