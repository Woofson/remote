use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::{params, Connection as SqliteConnection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

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
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN is_global INTEGER NOT NULL DEFAULT 1;", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN allow_clipboard TEXT NOT NULL DEFAULT 'bidirectional';", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN allow_transfer TEXT NOT NULL DEFAULT 'full';", []);
        let _ = conn.execute("ALTER TABLE connections ADD COLUMN view_only INTEGER NOT NULL DEFAULT 0;", []);

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

            info!("Created default administrator user 'admin' and initial connection template");
            return Ok(true);
        }
        Ok(false)
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
        let mut stmt = conn.prepare("SELECT id, name, description, created_at FROM groups ORDER BY name ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
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
        })
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
                    created_by, created_at, updated_at
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
            ) = r?;

            let is_owner = created_by.as_deref() == Some(&user.id);
            let can_clipboard_val = allow_clipboard != "disabled";
            let can_transfer_val = allow_transfer != "disabled";

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
                    "SELECT cp.can_connect, cp.can_edit, cp.can_clipboard, cp.can_transfer 
                     FROM connection_permissions cp
                     LEFT JOIN user_groups ug ON ug.group_id = cp.group_id
                     WHERE cp.connection_id = ?1 AND (cp.user_id = ?2 OR ug.user_id = ?2)",
                )?;
                let mut p_rows = p_stmt.query(params![id, user.id])?;
                let mut user_perms = ConnectionUserPerms {
                    can_connect: true,
                    can_edit: false,
                    can_clipboard: can_clipboard_val,
                    can_transfer: can_transfer_val,
                };
                let mut found = false;
                while let Some(p_row) = p_rows.next()? {
                    found = true;
                    if p_row.get::<_, i64>(0)? != 0 { user_perms.can_connect = true; }
                    if p_row.get::<_, i64>(1)? != 0 { user_perms.can_edit = true; }
                    if p_row.get::<_, i64>(2)? == 0 { user_perms.can_clipboard = false; }
                    if p_row.get::<_, i64>(3)? == 0 { user_perms.can_transfer = false; }
                }

                if !found && !is_global {
                    continue;
                }
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
            });
        }

        Ok(connections)
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
