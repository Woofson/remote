use parking_lot::RwLock;
use remoted::auth::{hash_password, OidcService};
use remoted::config::AppConfig;
use remoted::db::Database;
use remoted::server::{create_router, AppState};
use remoted::transfer::TransferManager;
use std::path::Path;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "remoted=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    remoted::init_crypto_provider();

    println!(
        r#"
  ██████╗ ███████╗███╗   ███╗ ██████╗ ████████╗███████╗
  ██╔══██╗██╔════╝████╗ ████║██╔═══██╗╚══██╔══╝██╔════╝
  ██████╔╝█████╗  ██╔████╔██║██║   ██║   ██║   █████╗  
  ██╔══██╗██╔══╝  ██║╚██╔╝██║██║   ██║   ██║   ██╔══╝  
  ██║  ██║███████╗██║ ╚═╝ ██║╚██████╔╝   ██║   ███████╗
  ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝ ╚═════╝    ╚═╝   ╚══════╝
                 Ultra-Low-Resource Remote Gateway — By Woofson
"#
    );

    let config_path = "config.toml";
    let config = AppConfig::load_or_create(config_path)?;
    info!("Configuration active: binding to {}", config.server.bind_address);

    let db_dir = Path::new(&config.server.data_dir);
    let db_path = db_dir.join("remote.db");
    let legacy_db_path = db_dir.join("remotedog.db");
    if !db_path.exists() && legacy_db_path.exists() {
        let _ = std::fs::rename(&legacy_db_path, &db_path);
        let _ = std::fs::rename(db_dir.join("remotedog.db-wal"), db_dir.join("remote.db-wal"));
        let _ = std::fs::rename(db_dir.join("remotedog.db-shm"), db_dir.join("remote.db-shm"));
    }

    let db = Database::new(&db_path)?;
    info!("Database connected at {}", db_path.display());

    // Seed initial admin user if no users exist
    let default_admin_pw = "remote";
    let initial_hash = hash_password(default_admin_pw)?;
    let seeded = db.seed_initial_admin(&initial_hash)?;
    if seeded {
        info!("============================================================");
        info!(" Default Admin User Created: 'admin'");
        info!(" Default Initial Password:   'remote'");
        info!(" Please change this password upon first login!");
        info!("============================================================");
    }

    let transfer_mgr = TransferManager::new(&config.storage.staging_dir);
    let oidc_service = Arc::new(OidcService::new(config.oidc.clone()));

    let app_state = AppState {
        config: Arc::new(RwLock::new(config.clone())),
        db,
        transfer_mgr,
        oidc_service,
    };

    let router = create_router(app_state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind_address).await?;
    info!("🚀 Remote Server listening on http://{}", config.server.bind_address);

    axum::serve(listener, router).await?;
    Ok(())
}
