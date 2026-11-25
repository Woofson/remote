use parking_lot::RwLock;
use remoted::auth::{generate_password, hash_password, verify_password, OidcService};
use remoted::config::AppConfig;
use remoted::db::{Database, User};
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

    println!("\n{}\n", include_str!("../assets/banner.txt").trim_end());
    info!("Remote v{} starting", env!("CARGO_PKG_VERSION"));

    let config_path = "config.toml";
    let mut config = AppConfig::load_or_create(config_path)?;
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

    config.server.jwt_secret =
        remoted::secrets::resolve_master_secret(&config.server.jwt_secret, db_dir, &db)?;

    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("reset-admin-password") => {
            reset_admin_password(&db, db_dir)?;
            return Ok(());
        }
        Some(other) => {
            eprintln!("Unknown command '{}'.\n", other);
            eprintln!("Usage:");
            eprintln!("    remoted                        Start the server");
            eprintln!("    remoted reset-admin-password   Give the built-in admin a new password (run on the Remote machine)");
            std::process::exit(2);
        }
    }

    bootstrap_admin(&db, db_dir)?;

    let transfer_mgr = TransferManager::new(&config.storage.staging_dir);
    let oidc_service = Arc::new(OidcService::new(config.oidc.clone()));

    let themes = Arc::new(remoted::themes::load_themes(&[
        Path::new("themes"),
        &Path::new(&config.server.data_dir).join("themes"),
    ]));
    info!("Themes: {} available", themes.len());

    let policy = remoted::server::load_security_policy(&db, config.server.token_expiry_hours);
    let sessions = Arc::new(remoted::auth::policy::SessionTracker::default());
    let app_state = AppState {
        config: Arc::new(RwLock::new(config.clone())),
        db,
        transfer_mgr,
        themes,
        icons: Arc::new(remoted::icons::IconStore::new(Path::new(&config.server.data_dir))),
        oidc_service,
        policy: Arc::new(RwLock::new(policy)),
        sessions: sessions.clone(),
        limiter: Arc::new(remoted::auth::policy::LoginLimiter::default()),
        shares: Arc::new(remoted::protocols::hub::ShareRegistry::default()),
    };

    // Forget idle/lock state of logins that cannot be alive any more.
    let prune_policy = app_state.policy.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tick.tick().await;
            let hours = prune_policy.read().max_login_hours.max(1) as u64;
            sessions.prune(std::time::Duration::from_secs(hours * 3600));
        }
    });

    let router = create_router(app_state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind_address).await?;
    info!("Remote Server listening on http://{}", config.server.bind_address);

    // Serve under `base_path` too (works whether or not the proxy strips it), and pass
    // the peer address on so the real client IP can be worked out behind a proxy.
    let base = config.server.base();
    if !base.is_empty() {
        info!("Also answering under the sub-path {}/", base);
    }
    let app = tower::Layer::layer(&axum::middleware::from_fn_with_state(base, remoted::server::strip_base_path), router);
    axum::serve(
        listener,
        axum::ServiceExt::<axum::extract::Request>::into_make_service_with_connect_info::<std::net::SocketAddr>(app),
    )
    .await?;
    Ok(())
}

const ADMIN_USER: &str = "admin";

fn print_admin_box(title: &str, password: &str, note: Option<&Path>) {
    println!("\n================================================================================");
    println!(" {}", title);
    println!("    Username : {}", ADMIN_USER);
    println!("    Password : {}", password);
    if let Some(p) = note {
        println!("    Saved in : {}", p.display());
    }
    println!("    You will be asked to choose your own password after signing in.");
    if note.is_some() {
        println!("    The file is removed once you do.");
    }
    println!("================================================================================\n");
}

/// First start: create the admin with a generated password (shown on screen and in a one-time
/// note, never in the log). Later starts: flag an admin still using a published default password.
fn bootstrap_admin(db: &Database, data_dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if db.count_users()? == 0 {
        let password = generate_password();
        db.seed_initial_admin(&hash_password(&password)?)?;
        let note = remoted::secrets::write_admin_note(data_dir, ADMIN_USER, &password).ok();
        print_admin_box("FIRST ADMIN LOGIN", &password, note.as_deref());
        match &note {
            Some(p) => info!("Created the first admin login '{}'. Its password is in: {}", ADMIN_USER, p.display()),
            None => info!("Created the first admin login '{}'; its password is shown above", ADMIN_USER),
        }
        return Ok(());
    }

    if let Some(admin) = db.get_user_by_username(ADMIN_USER)? {
        // Older installs used one of these published defaults ("remotedog" from the RemoteDog days).
        let default_in_use = if admin.auth_provider == "local" {
            ["remote", "remotedog"].into_iter().find(|p| verify_password(p, &admin.password_hash))
        } else {
            None
        };
        if let Some(default) = default_in_use {
            db.set_must_change_password(&admin.id, true)?;
            tracing::warn!(
                "The '{}' account still uses the published default password '{}'; it must be changed at next login",
                ADMIN_USER,
                default
            );
        }
    }
    let note = data_dir.join(remoted::secrets::ADMIN_NOTE_FILE);
    if note.exists() {
        info!(
            "The first-login note is still at {}; it is removed once the admin changes the password",
            note.display()
        );
    }
    Ok(())
}

/// `remoted reset-admin-password`: new generated password for the built-in admin, re-enabled,
/// role admin, must change at next login, all its logins ended. Creates the account if it is gone.
fn reset_admin_password(db: &Database, data_dir: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let password = generate_password();
    let hash = hash_password(&password)?;
    match db.get_user_by_username(ADMIN_USER)? {
        Some(admin) if admin.auth_provider != "local" => {
            return Err(format!("'{}' is not a local account; its password is managed elsewhere", ADMIN_USER).into());
        }
        Some(mut admin) => {
            admin.is_active = true;
            admin.role = "admin".into();
            db.update_user(&admin)?;
            db.set_user_password(&admin.id, &hash, true)?;
        }
        None => {
            let admin = User {
                id: uuid::Uuid::new_v4().to_string(),
                username: ADMIN_USER.into(),
                password_hash: hash,
                email: None,
                display_name: Some(ADMIN_USER.into()),
                role: "admin".into(),
                is_active: true,
                auth_provider: "local".into(),
                oidc_sub: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                last_login: None,
                avatar_data: None,
            };
            db.create_user(&admin)?;
            db.set_must_change_password(&admin.id, true)?;
        }
    }
    let note = remoted::secrets::write_admin_note(data_dir, ADMIN_USER, &password).ok();
    print_admin_box("ADMIN PASSWORD RESET", &password, note.as_deref());
    info!("Password for built-in admin '{}' was reset from the command line", ADMIN_USER);
    Ok(())
}
