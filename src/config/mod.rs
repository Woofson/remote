use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub oidc: OidcConfig,
    #[serde(default)]
    pub clipboard: ClipboardConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind_address: String,
    pub data_dir: String,
    pub jwt_secret: String,
    pub token_expiry_hours: i64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:8088".to_string(),
            data_dir: "./data".to_string(),
            jwt_secret: "remote-secret-key-woofson-2026".to_string(),
            token_expiry_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub staging_dir: String,
    pub max_upload_size_mb: usize,
    pub auto_cleanup_hours: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            staging_dir: "./data/staging".to_string(),
            max_upload_size_mb: 2048,
            auto_cleanup_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub allow_local_login: bool,
    pub allow_pam_login: bool,
    pub default_role: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            allow_local_login: true,
            allow_pam_login: false,
            default_role: "operator".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    pub enabled: bool,
    pub provider_name: String,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub auto_provision_users: bool,
    pub admin_group: String,
    pub operator_group: String,
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_name: "Authentik".to_string(),
            issuer_url: "https://authentik.example.com/application/o/remote/".to_string(),
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: "http://localhost:8088/api/auth/oidc/callback".to_string(),
            scopes: vec!["openid".into(), "profile".into(), "email".into(), "groups".into()],
            auto_provision_users: true,
            admin_group: "Remote-Admins".to_string(),
            operator_group: "Remote-Operators".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardConfig {
    pub default_mode: String,
    pub max_text_size_bytes: usize,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            default_mode: "bidirectional".to_string(),
            max_text_size_bytes: 1048576,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            storage: StorageConfig::default(),
            auth: AuthConfig::default(),
            oidc: OidcConfig::default(),
            clipboard: ClipboardConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = path.as_ref();
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let cfg: AppConfig = toml::from_str(&content)?;
            info!("Loaded configuration from {}", path.display());
            Ok(cfg)
        } else {
            let cfg = AppConfig::default();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let toml_str = toml::to_string_pretty(&cfg)?;
            std::fs::write(path, toml_str)?;
            info!("Created default configuration at {}", path.display());
            Ok(cfg)
        }
    }

    pub fn data_path(&self) -> PathBuf {
        PathBuf::from(&self.server.data_dir)
    }

    pub fn staging_path(&self) -> PathBuf {
        PathBuf::from(&self.storage.staging_dir)
    }
}
