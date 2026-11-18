//! The server's master secret.
//!
//! It signs login tokens and encrypts saved connection passwords and keys.
//! Unless the operator configures a private value in `config.toml`, a random
//! secret is generated once and kept in `<data_dir>/secret.key`.

use crate::auth::{decrypt_secret, encrypt_secret};
use crate::db::{Database, Reencrypt};
use std::io::Write;
use std::path::Path;
use tracing::{info, warn};

/// Values that were published in the repository at some point. They must never
/// be used as a secret; they are only tried to unlock data saved with them.
pub const PUBLIC_DEFAULT_SECRETS: &[&str] = &[
    "remote-super-secret-jwt-key-woofson-2026",
    "remotedog-super-secret-jwt-key-woofson-2026",
    "remote-secret-key-woofson-2026",
];

pub const SECRET_FILE: &str = "secret.key";

/// Decide which secret this server uses, creating `secret.key` and re-encrypting
/// saved credentials when moving away from a public default.
pub fn resolve_master_secret(configured: &str, data_dir: &Path, db: &Database) -> Result<String, String> {
    let configured = configured.trim();
    if !configured.is_empty() && !PUBLIC_DEFAULT_SECRETS.contains(&configured) {
        info!("Secrets: using the private jwt_secret from config.toml");
        return Ok(configured.to_string());
    }

    let path = data_dir.join(SECRET_FILE);
    let secret = load_or_create_secret_file(&path)?;

    // Saved credentials may still be locked with the old public value.
    let mut old_keys: Vec<&str> = PUBLIC_DEFAULT_SECRETS.to_vec();
    if !configured.is_empty() && !old_keys.contains(&configured) {
        old_keys.insert(0, configured);
    }
    let (converted, unreadable) = db
        .reencrypt_connection_secrets(|enc| {
            if decrypt_secret(enc, &secret).is_ok() {
                return Reencrypt::Keep;
            }
            old_keys
                .iter()
                .find_map(|old| decrypt_secret(enc, old).ok())
                .and_then(|plain| encrypt_secret(&plain, &secret).ok())
                .map_or(Reencrypt::Unreadable, Reencrypt::Replace)
        })
        .map_err(|e| format!("Failed to re-encrypt saved credentials: {}", e))?;

    if converted > 0 {
        info!("Secrets: re-encrypted {} saved connection credential(s) with the new secret", converted);
    }
    if unreadable > 0 {
        warn!(
            "Secrets: {} saved connection credential(s) could not be unlocked and must be re-entered",
            unreadable
        );
    }
    Ok(secret)
}

fn load_or_create_secret_file(path: &Path) -> Result<String, String> {
    if path.exists() {
        let secret = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?
            .trim()
            .to_string();
        if secret.len() < 32 {
            return Err(format!(
                "{} is empty or too short; restore it from a backup or delete it to generate a new one \
                 (saved connection passwords will then have to be re-entered)",
                path.display()
            ));
        }
        info!("Secrets: using {}", path.display());
        return Ok(secret);
    }

    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("Cannot generate secret: {}", e))?;
    let secret = hex::encode(bytes);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {}", parent.display(), e))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("Cannot create {}: {}", path.display(), e))?;
    file.write_all(secret.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Cannot write {}: {}", path.display(), e))?;

    info!("Secrets: generated a new secret in {} (include it in backups)", path.display());
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ConnectionRecord, Database};

    #[test]
    fn migrates_from_public_default() {
        let dir = std::env::temp_dir().join(format!("remote-secret-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::new(dir.join("t.db")).unwrap();

        let old = PUBLIC_DEFAULT_SECRETS[0];
        let rec = ConnectionRecord {
            id: "c1".into(),
            name: "x".into(),
            protocol: "ssh".into(),
            host: "127.0.0.1".into(),
            port: 22,
            username: Some("u".into()),
            password_enc: Some(encrypt_secret("hunter2", old).unwrap()),
            private_key_enc: None,
            settings_json: "{}".into(),
            icon: None,
            tags: None,
            is_global: true,
            allow_clipboard: "bidirectional".into(),
            allow_transfer: "full".into(),
            view_only: false,
            created_by: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        db.save_connection(&rec).unwrap();

        let secret = resolve_master_secret(old, &dir, &db).unwrap();
        assert_ne!(secret, old);
        assert!(dir.join(SECRET_FILE).exists());
        let enc = db.get_connection_raw("c1").unwrap().unwrap().password_enc.unwrap();
        assert_eq!(decrypt_secret(&enc, &secret).unwrap(), "hunter2");
        assert!(decrypt_secret(&enc, old).is_err());

        // Second start: same secret, nothing to convert.
        assert_eq!(resolve_master_secret(old, &dir, &db).unwrap(), secret);

        // A private configured secret is used as-is.
        assert_eq!(resolve_master_secret("my-own-private-value-123", &dir, &db).unwrap(), "my-own-private-value-123");

        std::fs::remove_dir_all(dir).unwrap();
    }
}
