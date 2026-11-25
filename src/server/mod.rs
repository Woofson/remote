use crate::auth::policy::{LoginLimiter, SecurityPolicy, SessionStatus, SessionTracker};
use crate::auth::{
    check_password_policy, create_jwt, decrypt_secret, encrypt_secret, generate_password,
    hash_password, verify_jwt, verify_password, Claims, OidcService,
};
use crate::config::AppConfig;
use crate::db::{ConnectionRecord, ConnectionSafe, ConnectionUserPerms, Database, Group, User, UserSafe};
use crate::protocols::local_pty::{available_shells, handle_local_pty_session};
use crate::protocols::rdp::{handle_rdp_session, RdpConnectionParams};
use crate::protocols::ssh::{
    create_ssh_session, handle_ssh_session, sftp_list_directory,
    sftp_upload_file, SshConnectionParams,
};
use crate::protocols::telnet::{handle_telnet_session, TelnetConnectionParams};
use crate::protocols::vnc::{handle_vnc_session, VncConnectionParams};
use crate::transfer::{Area, TransferManager};
use axum::{
    body::Body,
    extract::{
        ws::WebSocketUpgrade,
        DefaultBodyLimit, Multipart, Path as AxumPath, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post, put},
    Json, Router,
};
use parking_lot::RwLock;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

mod branding;
mod email;
mod proxy;
mod security;
mod share;

pub use proxy::strip_base_path;
use proxy::ClientIp;

#[derive(RustEmbed)]
#[folder = "frontend/"]
pub struct FrontendAssets;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub db: Database,
    pub transfer_mgr: TransferManager,
    pub themes: Arc<Vec<crate::themes::ThemeDefinition>>,
    pub icons: Arc<crate::icons::IconStore>,
    pub oidc_service: Arc<OidcService>,
    /// Admin session rules (login lifetime, idle timeout, 2FA), cached from the database.
    pub policy: Arc<RwLock<SecurityPolicy>>,
    /// Live login state: idle time and lock screen.
    pub sessions: Arc<SessionTracker>,
    /// Brake on repeated failed sign-in attempts.
    pub limiter: Arc<LoginLimiter>,
    /// Open share links of live sessions (#32).
    pub shares: Arc<crate::protocols::hub::ShareRegistry>,
}

/// Database key of the admin session policy.
pub const SECURITY_POLICY_KEY: &str = "security_policy";

/// Load the session policy from the database, or defaults (lifetime from config.toml).
pub fn load_security_policy(db: &Database, default_lifetime_hours: i64) -> SecurityPolicy {
    db.get_setting(SECURITY_POLICY_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<SecurityPolicy>(&v).ok())
        .filter(|p| p.validate().is_ok())
        .unwrap_or_else(|| SecurityPolicy::with_default_lifetime(default_lifetime_hours))
}

pub fn create_router(state: AppState) -> Router {
    let upload_limit = state
        .config
        .read()
        .storage
        .max_upload_size_mb
        .saturating_mul(1024 * 1024);

    Router::new()
        // Auth API
        .route("/api/auth/login", post(api_login))
        .route("/api/auth/logout", post(api_logout))
        .route("/api/auth/login/2fa", post(security::api_login_2fa))
        .route("/api/auth/lock", post(security::api_lock))
        .route("/api/auth/unlock", post(security::api_unlock))
        .route("/api/auth/activity", post(security::api_activity))
        .route("/api/me/2fa", get(security::api_2fa_status))
        .route("/api/me/2fa/setup", post(security::api_2fa_setup))
        .route("/api/me/2fa/enable", post(security::api_2fa_enable))
        .route("/api/me/2fa/disable", post(security::api_2fa_disable))
        .route("/api/me/2fa/recovery-codes", post(security::api_2fa_recovery_codes))
        .route("/api/users/:id/reset-2fa", post(security::api_reset_user_2fa))
        .route(
            "/api/admin/security-policy",
            get(security::api_get_security_policy).put(security::api_set_security_policy),
        )
        .route("/api/auth/me", get(api_me))
        .route("/api/auth/providers", get(api_auth_providers))
        .route("/api/version", get(api_version))
        .route("/api/health", get(proxy::api_health))
        .route("/api/ready", get(proxy::api_ready))
        .route("/api/auth/header-login", post(proxy::api_header_login))
        .route("/api/auth/forgot", post(email::api_forgot_password))
        .route("/api/auth/reset/check", get(email::api_reset_check))
        .route("/api/auth/reset", post(email::api_reset_password))
        .route("/api/users/:id/password-link", post(email::api_send_password_link))
        .route("/api/admin/email", get(email::api_get_email_settings).put(email::api_set_email_settings))
        .route("/api/admin/email/test", post(email::api_test_email))
        .route("/api/branding", get(branding::api_get_branding))
        .route("/api/branding/file/:name", get(branding::api_branding_file))
        .route("/api/admin/branding", axum::routing::put(branding::api_set_branding))
        .route(
            "/api/admin/branding/image",
            post(branding::api_upload_branding_image).layer(DefaultBodyLimit::max(branding::MAX_BRANDING_IMAGE + 64 * 1024)),
        )
        .route("/api/shells", get(api_list_shells))
        .route("/api/auth/oidc/login", get(api_oidc_login))
        .route("/api/auth/oidc/callback", get(api_oidc_callback))
        // Users & Groups API
        .route("/api/users", get(api_list_users).post(api_create_user))
        .route("/api/users/:id/reset-password", post(api_reset_user_password))
        .route("/api/auth/change-password", post(api_change_password))
        .route(
            "/api/users/:id",
            put(api_update_user).delete(api_delete_user),
        )
        .route("/api/groups", get(api_list_groups).post(api_create_group))
        .route("/api/groups/:id", put(api_update_group).delete(api_delete_group))
        .route("/api/groups/:id/members", put(api_set_group_members))
        .route(
            "/api/connections/:id/permissions",
            get(api_get_connection_permissions).put(api_set_connection_permissions),
        )
        // Connections API
        .route(
            "/api/connections",
            get(api_list_connections).post(api_create_connection),
        )
        .route(
            "/api/connections/:id",
            put(api_update_connection).delete(api_delete_connection),
        )
        .route("/api/connections/test", post(api_test_connection))
        // Files (shared folder, also the RDP \\tsclient\Files drive)
        .route("/api/files/list", get(api_files_list))
        .route("/api/files/download", get(api_files_download))
        .route(
            "/api/files/upload",
            post(api_files_upload).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route("/api/files/mkdir", post(api_files_mkdir))
        .route("/api/files/delete", post(api_files_delete))
        // SFTP transfers
        .route(
            "/api/transfers/sftp/:connection_id/list",
            post(api_sftp_list),
        )
        .route(
            "/api/transfers/sftp/:connection_id/upload",
            post(api_sftp_upload_staged),
        )
        // Audit & Settings API
        .route("/api/audit-logs", get(api_list_audit_logs))
        .route("/api/settings", get(api_get_settings))
        .route("/api/themes", get(api_list_themes))
        .route("/api/icons/selfhst", get(api_icons_search))
        .route("/api/icons/selfhst/:file", get(api_icon_selfhst))
        .route(
            "/api/icons/upload",
            post(api_icon_upload).layer(DefaultBodyLimit::max(crate::icons::MAX_UPLOAD_PNG + 64 * 1024)),
        )
        .route("/api/icons/upload/:file", get(api_icon_uploaded))
        .route("/api/me/preferences", get(api_get_preferences).put(api_set_preferences))
        .route("/api/connections/:id/scope", post(api_set_connection_scope))
        // WebSocket Remote Gateway Tunnel
        .route("/ws/tunnel/:connection_id", get(ws_tunnel_handler))
        .route("/ws/share/:token", get(share::ws_share_handler))
        .route("/api/shares/:token", get(share::api_share_info))
        // Static Frontend Files
        .fallback(static_file_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ================= Auth Helpers & Middlewares =================

/// The login token from `Authorization: Bearer` or the `remote_token` cookie.
fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            return Some(token.to_string());
        }
    }
    let cookies = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())?;
    cookies.split(';').find_map(|c| {
        let (name, value) = c.trim().split_once('=')?;
        (name == "remote_token" || name == "remotedog_token").then(|| value.to_string())
    })
}

fn auth_err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg })))
}

/// Which "waiting" states a request may pass through.
#[derive(Clone, Copy, Default)]
struct AuthOpts {
    /// Let through accounts that must still change their password or set up 2FA.
    allow_pending: bool,
    /// Let through logins whose screen is locked.
    allow_locked: bool,
}

fn coded_err(code: StatusCode, err_code: &str, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg, "code": err_code })))
}

/// Verify a login token and the account's live state: it must still exist, be active,
/// and the login must not have been ended by a password change, reset, disable or role change.
/// The admin session policy applies too: maximum login lifetime, idle timeout and lock screen.
/// Unless allowed, a required password change or 2FA setup blocks everything else.
fn authenticate(state: &AppState, token: &str, opts: AuthOpts) -> Result<Claims, (StatusCode, Json<Value>)> {
    let secret = state.config.read().server.jwt_secret.clone();
    let claims = verify_jwt(token, &secret).map_err(|e| auth_err(StatusCode::UNAUTHORIZED, &e))?;

    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .map_err(|e| auth_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| auth_err(StatusCode::UNAUTHORIZED, "Session is no longer valid, please log in again"))?;
    if !user.is_active {
        return Err(auth_err(StatusCode::FORBIDDEN, "Account is disabled"));
    }

    let (generation, must_change) = state
        .db
        .get_user_security(&user.id)
        .map_err(|e| auth_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if claims.gen != generation {
        return Err(auth_err(StatusCode::UNAUTHORIZED, "Session has ended, please log in again"));
    }

    let policy = state.policy.read().clone();
    let age = chrono::Utc::now().timestamp() - claims.iat as i64;
    if age > policy.max_login_hours * 3600 {
        return Err(coded_err(StatusCode::UNAUTHORIZED, "session_expired", "Your login has expired, please sign in again"));
    }
    match state.sessions.check(&claims.sid, &policy) {
        SessionStatus::Expired => {
            return Err(coded_err(StatusCode::UNAUTHORIZED, "session_expired", "Signed out after inactivity, please sign in again"));
        }
        SessionStatus::Locked if !opts.allow_locked => {
            return Err(coded_err(StatusCode::LOCKED, "session_locked", "The session is locked"));
        }
        _ => {}
    }

    if must_change && !opts.allow_pending {
        return Err(coded_err(
            StatusCode::FORBIDDEN,
            "password_change_required",
            "You must choose a new password before continuing",
        ));
    }
    if !opts.allow_pending
        && user.auth_provider == "local"
        && policy.requires_2fa(&user.role)
        && !state.db.get_two_factor(&user.id).map(|t| t.enabled()).unwrap_or(false)
    {
        return Err(coded_err(
            StatusCode::FORBIDDEN,
            "two_factor_setup_required",
            "Set up two-factor authentication before continuing",
        ));
    }
    Ok(claims)
}

fn authenticate_token(state: &AppState, token: &str, allow_pending: bool) -> Result<Claims, (StatusCode, Json<Value>)> {
    authenticate(state, token, AuthOpts { allow_pending, allow_locked: false })
}

fn extract_with(state: &AppState, headers: &HeaderMap, opts: AuthOpts) -> Result<Claims, (StatusCode, Json<Value>)> {
    let token = token_from_headers(headers)
        .ok_or_else(|| auth_err(StatusCode::UNAUTHORIZED, "Authentication token missing or invalid"))?;
    authenticate(state, &token, opts)
}

fn extract_claims(state: &AppState, headers: &HeaderMap) -> Result<Claims, (StatusCode, Json<Value>)> {
    extract_with(state, headers, AuthOpts::default())
}

/// Like [`extract_claims`], but also lets through accounts that must change their password
/// or set up 2FA, and locked logins (only for: who am I, log out, change password,
/// lock, unlock, activity).
fn extract_claims_pending(state: &AppState, headers: &HeaderMap) -> Result<Claims, (StatusCode, Json<Value>)> {
    extract_with(state, headers, AuthOpts { allow_pending: true, allow_locked: true })
}

/// For 2FA setup: lets through accounts that must set up 2FA, but never a locked login
/// (someone at an unattended screen must not be able to tie the account to their phone).
fn extract_claims_setup(state: &AppState, headers: &HeaderMap) -> Result<Claims, (StatusCode, Json<Value>)> {
    extract_with(state, headers, AuthOpts { allow_pending: true, allow_locked: false })
}

/// Login lifetime for new tokens, from the admin policy.
fn login_hours(state: &AppState) -> i64 {
    state.policy.read().max_login_hours
}

// ================= Auth Handlers =================

#[derive(Deserialize)]
pub struct LoginPayload {
    pub username: String,
    pub password: Option<String>,
}

pub async fn api_login(
    State(state): State<AppState>,
    ip: ClientIp,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let user_opt = state
        .db
        .get_user_by_username(&payload.username)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    if let Some(wait) = state.limiter.blocked_for(&payload.username).or_else(|| state.limiter.blocked_for(&ip.limiter_key())) {
        return Err(security::too_many_attempts(wait));
    }

    // Check the password first so the reply never reveals whether an account exists or is disabled.
    let password = payload.password.unwrap_or_default();
    let user = match user_opt {
        Some(u) if u.auth_provider == "local" && verify_password(&password, &u.password_hash) => u,
        other => {
            state.limiter.record_failure(&payload.username);
            state.limiter.record_failure(&ip.limiter_key());
            state.db.log_audit(
                other.as_ref().map(|u| u.id.as_str()),
                &payload.username,
                "LOGIN_FAILED",
                None,
                None,
                Some("Wrong username or password"),
                ip.as_opt(),
            );
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Invalid username or password" })),
            ));
        }
    };

    if !user.is_active {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Account is disabled" })),
        ));
    }

    let (generation, must_change) = state
        .db
        .get_user_security(&user.id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    // With 2FA on, the password only earns a short-lived ticket for the code step.
    if state.db.get_two_factor(&user.id).map(|t| t.enabled()).unwrap_or(false) {
        let challenge = crate::auth::create_2fa_challenge(&user.id, generation, &cfg.server.jwt_secret)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;
        return Ok(Json(json!({ "two_factor_required": true, "challenge": challenge })));
    }

    state.limiter.record_success(&user.username);
    state.limiter.record_success(&ip.limiter_key());
    let token = create_jwt(&user, &cfg.server.jwt_secret, login_hours(&state), generation)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;

    let _ = state.db.update_last_login(&user.id);
    state.db.log_audit(
        Some(&user.id),
        &user.username,
        "LOGIN_SUCCESS",
        None,
        None,
        Some("User logged in via local auth"),
        ip.as_opt(),
    );

    let setup_required = state.policy.read().requires_2fa(&user.role);
    let safe_user: UserSafe = user.into();
    Ok(Json(json!({
        "token": token,
        "user": safe_user,
        "must_change_password": must_change,
        "two_factor_setup_required": setup_required,
    })))
}

/// Log out: forget this login's idle/lock state (the browser drops the token).
pub async fn api_logout(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    if let Some(token) = token_from_headers(&headers) {
        let secret = state.config.read().server.jwt_secret.clone();
        if let Ok(claims) = verify_jwt(&token, &secret) {
            state.sessions.end(&claims.sid);
        }
    }
    Json(json!({ "message": "Logged out successfully" }))
}

pub async fn api_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims_pending(&state, &headers)?;

    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "User not found" })),
            )
        })?;

    if !user.is_active {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Account is disabled" })),
        ));
    }

    let (_, must_change) = state.db.get_user_security(&user.id).unwrap_or((0, false));
    let can_change_password = user.auth_provider == "local";
    let policy = state.policy.read().clone();
    let two_factor = state.db.get_two_factor(&user.id).map(|t| t.enabled()).unwrap_or(false);
    let setup_required = can_change_password && policy.requires_2fa(&user.role) && !two_factor;
    let mut body = json!(UserSafe::from(user));
    body["must_change_password"] = json!(must_change);
    body["can_change_password"] = json!(can_change_password);
    body["two_factor_enabled"] = json!(two_factor);
    body["two_factor_setup_required"] = json!(setup_required);
    body["session_policy"] = json!({
        "max_login_hours": policy.max_login_hours,
        "idle_minutes": policy.idle_minutes,
        "idle_action": policy.idle_action,
        "lock_requires_2fa": policy.lock_requires_2fa,
    });
    Ok(Json(body))
}

/// Login shells installed on the server, for the local shell's Shell choice.
pub async fn api_list_shells(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    extract_claims(&state, &headers)?;
    Ok(Json(json!({ "shells": available_shells() })))
}

/// Public: the running version, from Cargo.toml (single source of truth).
pub async fn api_version() -> Json<Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build_number": env!("REMOTE_BUILD_NUMBER"),
        "build_commit": env!("REMOTE_BUILD_COMMIT"),
        "build_timestamp": env!("REMOTE_BUILD_TIMESTAMP"),
    }))
}

pub async fn api_auth_providers(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.read().clone();
    Json(json!({
        "local": cfg.auth.allow_local_login,
        "pam": cfg.auth.allow_pam_login,
        "min_password_length": cfg.auth.min_password_length,
        "oidc": {
            "enabled": cfg.oidc.enabled,
            "provider_name": cfg.oidc.provider_name,
        },
        "header_login": cfg.auth.header_login.enabled,
        "password_reset": email::EmailSettings::load(&state.db).ready(),
    }))
}

pub async fn api_oidc_login(State(state): State<AppState>) -> Result<Redirect, (StatusCode, Json<Value>)> {
    let state_token = Uuid::new_v4().to_string();
    let auth_url = state
        .oidc_service
        .build_authorization_url(&state_token)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;

    Ok(Redirect::temporary(&auth_url))
}

#[derive(Deserialize)]
pub struct OidcCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub async fn api_oidc_callback(
    State(state): State<AppState>,
    Query(params): Query<OidcCallbackParams>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    if let Some(err) = params.error {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("OIDC error: {}", err) })),
        ));
    }

    let code = match params.code {
        Some(c) => c,
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Missing authorization code" })),
            ));
        }
    };

    let user_info = state
        .oidc_service
        .exchange_code_and_get_user(&code)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))))?;

    let existing_user = state
        .db
        .get_user_by_oidc_sub(&user_info.sub)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    let user = match existing_user {
        Some(mut u) => {
            let role = if let Some(groups) = &user_info.groups {
                state.oidc_service.determine_role_for_groups(groups)
            } else {
                u.role.clone()
            };
            u.role = role;
            u.email = user_info.email;
            u.display_name = user_info.name.or(user_info.preferred_username);
            let _ = state.db.update_user(&u);
            u
        }
        None => {
            if !cfg.oidc.auto_provision_users {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "Auto user provisioning is disabled" })),
                ));
            }

            let username = user_info
                .preferred_username
                .or(user_info.nickname)
                .unwrap_or_else(|| format!("user_{}", &user_info.sub[..8]));

            let role = if let Some(groups) = &user_info.groups {
                state.oidc_service.determine_role_for_groups(groups)
            } else {
                cfg.auth.default_role.clone()
            };

            let new_user = User {
                id: Uuid::new_v4().to_string(),
                username,
                password_hash: hash_password(&Uuid::new_v4().to_string()).unwrap_or_default(),
                email: user_info.email,
                display_name: user_info.name,
                role,
                is_active: true,
                auth_provider: "oidc".to_string(),
                oidc_sub: Some(user_info.sub),
                created_at: chrono::Utc::now().to_rfc3339(),
                last_login: Some(chrono::Utc::now().to_rfc3339()),
                avatar_data: None,
            };

            state.db.create_user(&new_user).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
            })?;
            new_user
        }
    };

    let (generation, _) = state
        .db
        .get_user_security(&user.id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    let token = create_jwt(&user, &cfg.server.jwt_secret, login_hours(&state), generation)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;

    // Links use the configured sub-path so this page also works behind a proxy.
    let base = cfg.server.base();
    let html = format!(
        r#"<!DOCTYPE html>
        <html>
        <head><title>Remote</title></head>
        <body style="background:#121214;color:#f59e0b;font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;">
          <div style="text-align:center;">
            <img src="{base}/assets/icons/remote-160.webp" alt="Remote" style="height:44px;margin-bottom:12px;vertical-align:middle;" />
            <h2 style="margin:0;font-weight:700;color:#f4f4f5;">Signed in</h2>
            <p style="color:#a1a1aa;margin-top:6px;">Opening Remote…</p>
          </div>
          <script>
            localStorage.setItem('remote_token', '{token}');
            document.cookie = 'remote_token={token}; path={base}/; max-age=86400; SameSite=Lax' + (location.protocol === 'https:' ? '; Secure' : '');
            window.location.href = '{base}/';
          </script>
        </body>
        </html>"#,
        base = base,
        token = token
    );

    Ok(Html(html).into_response())
}

// ================= Users Management API =================

pub async fn api_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    let users = state.db.list_users().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    let with_2fa = state.db.two_factor_user_ids().unwrap_or_default();
    let list: Vec<Value> = users
        .into_iter()
        .map(|u| {
            let on = with_2fa.contains(&u.id);
            let mut v = json!(u);
            v["two_factor"] = json!(on);
            v
        })
        .collect();
    Ok(Json(Value::Array(list)))
}

#[derive(Deserialize)]
pub struct CreateUserPayload {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    /// Ask the user to choose their own password at first login (default: yes).
    pub must_change_password: Option<bool>,
}

const ROLES: &[&str] = &["admin", "operator", "viewer"];

fn validate_role(role: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if ROLES.contains(&role) {
        Ok(())
    } else {
        Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Role must be admin, operator or viewer" }))))
    }
}

pub async fn api_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserPayload>,
) -> Result<Json<UserSafe>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    let username = payload.username.trim().to_string();
    if username.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Username is required" }))));
    }
    let role = payload.role.clone().unwrap_or_else(|| "operator".to_string());
    validate_role(&role)?;
    let min_len = state.config.read().auth.min_password_length;
    check_password_policy(&payload.password, &username, None, min_len)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    let pw_hash = hash_password(&payload.password)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;

    let new_user = User {
        id: Uuid::new_v4().to_string(),
        username,
        password_hash: pw_hash,
        email: payload.email,
        display_name: payload.display_name,
        role,
        is_active: true,
        auth_provider: "local".to_string(),
        oidc_sub: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        last_login: None,
        avatar_data: None,
    };

    state.db.create_user(&new_user).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    if payload.must_change_password.unwrap_or(true) {
        let _ = state.db.set_must_change_password(&new_user.id, true);
    }
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "USER_CREATED",
        None,
        None,
        Some(&format!("Created user '{}' ({})", new_user.username, new_user.role)),
        None,
    );
    Ok(Json(new_user.into()))
}

#[derive(Deserialize)]
pub struct UpdateUserPayload {
    pub username: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub is_active: Option<bool>,
    pub new_password: Option<String>,
    pub password: Option<String>,
    pub avatar_data: Option<String>,
}

pub async fn api_update_user(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<UpdateUserPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" && claims.sub != id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Access denied" })),
        ));
    }

    let mut user = state
        .db
        .get_user_by_id(&id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "User not found" })),
            )
        })?;

    // Only administrators modifying another user can change a username. Users cannot change username on themselves.
    if let Some(un) = payload.username {
        if claims.role == "admin" && claims.sub != id {
            let trimmed = un.trim().to_string();
            if !trimmed.is_empty() && trimmed != user.username {
                if let Ok(Some(existing)) = state.db.get_user_by_username(&trimmed) {
                    if existing.id != user.id {
                        return Err((
                            StatusCode::CONFLICT,
                            Json(json!({ "error": "Username is already taken" })),
                        ));
                    }
                }
                user.username = trimmed;
            }
        }
    }

    if let Some(em) = payload.email {
        let trimmed = em.trim().to_string();
        user.email = if trimmed.is_empty() { None } else { Some(trimmed) };
    }
    if let Some(dn) = payload.display_name {
        let trimmed = dn.trim().to_string();
        user.display_name = if trimmed.is_empty() { None } else { Some(trimmed) };
    }
    if let Some(av) = payload.avatar_data {
        user.avatar_data = if av.trim().is_empty() { None } else { Some(av) };
    }

    if payload.new_password.as_deref().or(payload.password.as_deref()).is_some_and(|p| !p.is_empty()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Passwords are changed with Change password or, for admins, Reset password" })),
        ));
    }

    let mut end_logins = false;
    if claims.role == "admin" {
        if let Some(r) = payload.role {
            validate_role(&r)?;
            if r != user.role {
                if claims.sub == id {
                    return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "You cannot change your own role" }))));
                }
                user.role = r;
                end_logins = true;
            }
        }
        if let Some(active) = payload.is_active {
            if !active && claims.sub == id {
                return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "You cannot disable your own account" }))));
            }
            if user.is_active && !active {
                end_logins = true;
            }
            user.is_active = active;
        }
    }

    state.db.update_user(&user).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    if end_logins {
        let _ = state.db.bump_session_generation(&user.id);
    }

    let safe_user: UserSafe = user.into();
    Ok(Json(json!({ "message": "User updated successfully", "user": safe_user })))
}

#[derive(Deserialize)]
pub struct ChangePasswordPayload {
    pub current_password: String,
    pub new_password: String,
}

/// Change your own password. Allowed while a change is required. Ends all other logins
/// and returns a fresh token so this browser stays logged in.
pub async fn api_change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChangePasswordPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims_pending(&state, &headers)?;
    let cfg = state.config.read().clone();
    let bad = |msg: &str| (StatusCode::BAD_REQUEST, Json(json!({ "error": msg })));

    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
        .ok_or_else(|| bad("User not found"))?;
    if user.auth_provider != "local" {
        return Err(bad("This account's password is managed outside Remote"));
    }
    if !verify_password(&payload.current_password, &user.password_hash) {
        state.db.log_audit(Some(&user.id), &user.username, "PASSWORD_CHANGE_FAILED", None, None, Some("Wrong current password"), None);
        return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "Current password is incorrect" }))));
    }
    check_password_policy(&payload.new_password, &user.username, Some(&payload.current_password), cfg.auth.min_password_length)
        .map_err(|e| bad(&e))?;

    let hash = hash_password(&payload.new_password).map_err(|e| bad(&e))?;
    state
        .db
        .set_user_password(&user.id, &hash, false)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    // The first-login note is no longer needed once the built-in admin picks a password.
    if user.username == "admin" {
        let note = std::path::Path::new(&cfg.server.data_dir).join(crate::secrets::ADMIN_NOTE_FILE);
        if note.exists() && std::fs::remove_file(&note).is_ok() {
            tracing::info!("Removed {} after the admin changed the password", note.display());
        }
    }
    state.db.log_audit(Some(&user.id), &user.username, "PASSWORD_CHANGED", None, None, None, None);
    email::notify_user(&state, &user, "Your Remote password was changed", "The password was changed");

    let (generation, _) = state.db.get_user_security(&user.id).unwrap_or((0, false));
    let token = create_jwt(&user, &cfg.server.jwt_secret, login_hours(&state), generation)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "message": "Password changed", "token": token })))
}

#[derive(Deserialize)]
pub struct ResetPasswordPayload {
    /// New password; when omitted a random one is generated and returned once.
    pub password: Option<String>,
    /// Ask the user to choose their own password at next login (default: yes).
    pub must_change_password: Option<bool>,
}

/// Admin: give a user a new password. Ends all of that user's logins.
pub async fn api_reset_user_password(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<ResetPasswordPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "Admin role required" }))));
    }
    let bad = |msg: &str| (StatusCode::BAD_REQUEST, Json(json!({ "error": msg })));
    let user = state
        .db
        .get_user_by_id(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "User not found" }))))?;
    if user.auth_provider != "local" {
        return Err(bad("This account's password is managed outside Remote"));
    }

    let (password, generated) = match payload.password.filter(|p| !p.is_empty()) {
        Some(p) => {
            let min_len = state.config.read().auth.min_password_length;
            check_password_policy(&p, &user.username, None, min_len).map_err(|e| bad(&e))?;
            (p, false)
        }
        None => (generate_password(), true),
    };
    let must_change = payload.must_change_password.unwrap_or(true);
    let hash = hash_password(&password).map_err(|e| bad(&e))?;
    state
        .db
        .set_user_password(&user.id, &hash, must_change)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "PASSWORD_RESET",
        None,
        None,
        Some(&format!(
            "Reset password for '{}'{}",
            user.username,
            if must_change { " (must change at next login)" } else { "" }
        )),
        None,
    );

    email::notify_user(&state, &user, "Your Remote password was reset", "An administrator reset the password");
    Ok(Json(json!({
        "message": "Password reset",
        "password": if generated { Some(password) } else { None },
        "must_change_password": must_change,
    })))
}

pub async fn api_delete_user(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    if claims.sub == id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Cannot delete your own account" })),
        ));
    }

    state.db.delete_user(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!({ "message": "User deleted" })))
}

// ================= Groups API =================

pub async fn api_list_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Value>>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    let internal = |e: rusqlite::Error| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
    let groups = state.db.list_groups().map_err(internal)?;
    let members = if claims.role == "admin" { state.db.group_members().map_err(internal)? } else { Default::default() };
    Ok(Json(
        groups
            .into_iter()
            .map(|g| {
                let ids = members.get(&g.id).cloned().unwrap_or_default();
                let mut v = json!(g);
                v["member_ids"] = json!(ids);
                v
            })
            .collect(),
    ))
}

fn require_admin(claims: &Claims) -> Result<(), (StatusCode, Json<Value>)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "Admin role required" }))));
    }
    Ok(())
}

pub async fn api_update_group(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateGroupPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Group name is required" }))));
    }
    let found = state
        .db
        .rename_group(&id, name, payload.description.as_deref())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))))?;
    if !found {
        return Err((StatusCode::NOT_FOUND, Json(json!({ "error": "Group not found" }))));
    }
    state.db.log_audit(Some(&claims.sub), &claims.username, "GROUP_UPDATED", None, None, Some(&format!("Group '{}'", name)), None);
    Ok(Json(json!({ "message": "Group updated" })))
}

pub async fn api_delete_group(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let name = state.db.list_groups().ok().and_then(|g| g.into_iter().find(|g| g.id == id)).map(|g| g.name);
    let found = state
        .db
        .delete_group(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    if !found {
        return Err((StatusCode::NOT_FOUND, Json(json!({ "error": "Group not found" }))));
    }
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "GROUP_DELETED",
        None,
        None,
        Some(&format!("Group '{}' (memberships and assignments removed)", name.unwrap_or_default())),
        None,
    );
    Ok(Json(json!({ "message": "Group deleted" })))
}

#[derive(Deserialize)]
pub struct GroupMembersPayload {
    pub user_ids: Vec<String>,
}

pub async fn api_set_group_members(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<GroupMembersPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let group = state
        .db
        .list_groups()
        .ok()
        .and_then(|g| g.into_iter().find(|g| g.id == id))
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "Group not found" }))))?;
    for uid in &payload.user_ids {
        if state.db.get_user_by_id(uid).ok().flatten().is_none() {
            return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Unknown user {}", uid) }))));
        }
    }
    state
        .db
        .set_group_members(&id, &payload.user_ids)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "GROUP_MEMBERS_CHANGED",
        None,
        None,
        Some(&format!("Group '{}': {} member(s)", group.name, payload.user_ids.len())),
        None,
    );
    Ok(Json(json!({ "message": "Members updated" })))
}

#[derive(Deserialize)]
pub struct CreateGroupPayload {
    pub name: String,
    pub description: Option<String>,
}

pub async fn api_create_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateGroupPayload>,
) -> Result<Json<Group>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    let group = state
        .db
        .create_group(&payload.name, payload.description.as_deref())
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(Json(group))
}

// ================= Connections API =================

pub async fn api_list_connections(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConnectionSafe>>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;

    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "User not found" })),
            )
        })?;

    let connections = state.db.list_connections_for_user(&user).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(connections))
}

#[derive(Deserialize)]
pub struct SaveConnectionPayload {
    pub id: Option<String>,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub settings_json: Option<String>,
    pub icon: Option<String>,
    pub tags: Option<String>,
    pub is_global: Option<bool>,
    pub allow_clipboard: Option<String>,
    pub allow_transfer: Option<String>,
    pub view_only: Option<bool>,
    /// Admins: the user this connection belongs to (default: the creator).
    pub owner_id: Option<String>,
}

pub async fn api_create_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SaveConnectionPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Operator or Admin role required to create connections" })),
        ));
    }

    let id = payload.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = chrono::Utc::now().to_rfc3339();

    let password_enc = if let Some(pw) = payload.password {
        if !pw.is_empty() {
            Some(
                encrypt_secret(&pw, &cfg.server.jwt_secret)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?,
            )
        } else {
            None
        }
    } else {
        None
    };

    let private_key_enc = if let Some(key) = payload.private_key {
        if !key.is_empty() {
            Some(
                encrypt_secret(&key, &cfg.server.jwt_secret)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?,
            )
        } else {
            None
        }
    } else {
        None
    };

    let is_admin = claims.role == "admin";
    let is_global = if is_admin {
        payload.is_global.unwrap_or(true)
    } else {
        false // Operators create personal private connections by default
    };

    let allow_clipboard = payload
        .allow_clipboard
        .unwrap_or_else(|| "bidirectional".into());
    let allow_transfer = payload.allow_transfer.unwrap_or_else(|| "full".into());
    let view_only = payload.view_only.unwrap_or(false);

    let record = ConnectionRecord {
        id: id.clone(),
        name: payload.name,
        protocol: payload.protocol,
        host: payload.host,
        port: payload.port,
        username: payload.username,
        password_enc,
        private_key_enc,
        settings_json: payload.settings_json.unwrap_or_else(|| "{}".into()),
        icon: clean_icon(payload.icon.clone())?,
        tags: payload.tags,
        is_global,
        allow_clipboard,
        allow_transfer,
        view_only,
        created_by: Some(match payload.owner_id.filter(|o| is_admin && !o.is_empty()) {
            Some(owner) => {
                state
                    .db
                    .get_user_by_id(&owner)
                    .ok()
                    .flatten()
                    .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({ "error": "Owner not found" }))))?;
                owner
            }
            None => claims.sub.clone(),
        }),
        created_at: now.clone(),
        updated_at: now,
    };

    state.db.save_connection(&record).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!({ "id": id, "message": "Connection created successfully" })))
}

pub async fn api_update_connection(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<SaveConnectionPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Operator or Admin role required" })),
        ));
    }

    let mut record = state
        .db
        .get_connection_raw(&id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Connection not found" })),
            )
        })?;

    let is_admin = claims.role == "admin";
    let is_owner = record.created_by.as_deref() == Some(&claims.sub);
    let can_edit_by_grant = !is_admin
        && !is_owner
        && authorize_connection(&state, &claims, &id, "Edit")
            .map(|a| a.perms.can_edit)
            .unwrap_or(false);
    if !is_admin && !is_owner && !can_edit_by_grant {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Access denied: you may not edit this connection" })),
        ));
    }
    // Access rules stay with the owner and admins; editors by assignment change details only.
    let may_change_rules = is_admin || is_owner;

    record.name = payload.name;
    record.protocol = payload.protocol;
    record.host = payload.host;
    record.port = payload.port;
    record.username = payload.username;
    record.settings_json = payload.settings_json.unwrap_or(record.settings_json);
    record.icon = clean_icon(payload.icon.clone())?;
    record.tags = payload.tags;
    if is_admin {
        if let Some(g) = payload.is_global {
            record.is_global = g;
        }
    }
    if may_change_rules {
        if let Some(clip) = payload.allow_clipboard {
            record.allow_clipboard = clip;
        }
        if let Some(trans) = payload.allow_transfer {
            record.allow_transfer = trans;
        }
        if let Some(vo) = payload.view_only {
            record.view_only = vo;
        }
    }
    if is_admin {
        if let Some(owner) = payload.owner_id.filter(|o| !o.is_empty()) {
            if record.created_by.as_deref() != Some(owner.as_str()) {
                state
                    .db
                    .get_user_by_id(&owner)
                    .ok()
                    .flatten()
                    .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({ "error": "Owner not found" }))))?;
                record.created_by = Some(owner);
            }
        }
    }
    record.updated_at = chrono::Utc::now().to_rfc3339();

    if let Some(pw) = payload.password {
        if !pw.is_empty() {
            record.password_enc = Some(
                encrypt_secret(&pw, &cfg.server.jwt_secret)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?,
            );
        }
    }

    if let Some(key) = payload.private_key {
        if !key.is_empty() {
            record.private_key_enc = Some(
                encrypt_secret(&key, &cfg.server.jwt_secret)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?,
            );
        }
    }

    state.db.save_connection(&record).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!({ "message": "Connection updated" })))
}

pub async fn api_delete_connection(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Access denied" })),
        ));
    }

    let record = state
        .db
        .get_connection_raw(&id)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Connection not found" })),
            )
        })?;

    let is_admin = claims.role == "admin";
    let is_owner = record.created_by.as_deref() == Some(&claims.sub);
    if !is_admin && !is_owner {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Access denied: only the owner or an administrator can delete this connection" })),
        ));
    }

    state.db.delete_connection(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!({ "message": "Connection deleted" })))
}

#[derive(Deserialize)]
pub struct TestConnectionPayload {
    pub host: String,
    pub port: u16,
}

/// Check whether a host and port accept TCP connections. Operator or Admin only,
/// so the gateway cannot be used to probe the internal network anonymously.
pub async fn api_test_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TestConnectionPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "Operator or Admin role required" }))));
    }

    let (host, port) = (payload.host.clone(), payload.port);
    let result = tokio::task::spawn_blocking(move || {
        crate::protocols::net::connect_tcp(&host, port, std::time::Duration::from_secs(4)).map(|_| ())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));

    Ok(Json(json!({
        "reachable": result.is_ok(),
        "error": result.err(),
        "host": payload.host,
        "port": payload.port,
    })))
}

/// RDP drives for a session: the user's own Files and the Shared folder
/// (read-only unless the user is an admin).
fn session_drives(state: &AppState, user: &User) -> Vec<crate::protocols::rdp_drive::DriveSpec> {
    use crate::protocols::rdp_drive::DriveSpec;
    let mut drives = Vec::new();
    if let Ok(mine) = state.transfer_mgr.scope(Area::Mine, &user.id) {
        drives.push(DriveSpec { device_id: 1, name: "Files".into(), path: mine.root, read_only: false });
    }
    if let Ok(shared) = state.transfer_mgr.scope(Area::Shared, &user.id) {
        drives.push(DriveSpec { device_id: 2, name: "Shared".into(), path: shared.root, read_only: user.role != "admin" });
    }
    drives
}

// ================= Connection Icons =================

/// Only `selfhst:<slug>` / `upload:<hash>` (or nothing) may be stored as a connection icon.
fn clean_icon(icon: Option<String>) -> Result<Option<String>, (StatusCode, Json<Value>)> {
    match icon.map(|i| i.trim().to_string()).filter(|i| !i.is_empty()) {
        None => Ok(None),
        Some(i) if crate::icons::valid_icon_ref(&i) => Ok(Some(i)),
        Some(_) => Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid icon" })))),
    }
}

fn png_response(bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(bytes))
        .unwrap()
}

#[derive(Deserialize)]
pub struct IconSearchQuery {
    pub q: Option<String>,
}

pub async fn api_icons_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<IconSearchQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = extract_claims(&state, &headers)?;
    let icons = state
        .icons
        .search(q.q.as_deref().unwrap_or(""), 60)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "icons": icons })))
}

pub async fn api_icon_selfhst(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(file): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let _ = extract_claims(&state, &headers)?;
    let slug = file.strip_suffix(".png").unwrap_or(&file);
    let bytes = state
        .icons
        .selfhst_png(slug)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, Json(json!({ "error": e }))))?;
    Ok(png_response(bytes))
}

pub async fn api_icon_uploaded(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(file): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let _ = extract_claims(&state, &headers)?;
    let hash = file.strip_suffix(".png").unwrap_or(&file);
    let bytes = state
        .icons
        .read_upload(hash)
        .map_err(|e| (StatusCode::NOT_FOUND, Json(json!({ "error": e }))))?;
    Ok(png_response(bytes))
}

/// Upload an icon (a PNG already resized by the browser). Operator or Admin.
pub async fn api_icon_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_files_writer(&claims)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))))?
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({ "error": "No file" }))))?;
    let bytes = field
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))))?;
    let icon = state
        .icons
        .save_upload(&bytes)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
    Ok(Json(json!({ "icon": icon })))
}

// ================= Connection Assignments =================

pub async fn api_get_connection_permissions(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let grants = state
        .db
        .list_connection_permissions(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    Ok(Json(json!({ "grants": grants })))
}

#[derive(Deserialize)]
pub struct GrantPayload {
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    #[serde(default = "default_true")]
    pub can_connect: bool,
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default = "default_true")]
    pub can_clipboard: bool,
    #[serde(default = "default_true")]
    pub can_transfer: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct SetGrantsPayload {
    pub grants: Vec<GrantPayload>,
}

/// Admin: replace who a connection is assigned to.
pub async fn api_set_connection_permissions(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<SetGrantsPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let bad = |m: String| (StatusCode::BAD_REQUEST, Json(json!({ "error": m })));
    let conn = state
        .db
        .get_connection_raw(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "Connection not found" }))))?;

    let groups = state.db.list_groups().unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut grants = Vec::new();
    for g in payload.grants {
        let user_id = g.user_id.filter(|v| !v.is_empty());
        let group_id = g.group_id.filter(|v| !v.is_empty());
        match (&user_id, &group_id) {
            (Some(u), None) => {
                if state.db.get_user_by_id(u).ok().flatten().is_none() {
                    return Err(bad(format!("Unknown user {}", u)));
                }
            }
            (None, Some(gid)) => {
                if !groups.iter().any(|x| &x.id == gid) {
                    return Err(bad(format!("Unknown group {}", gid)));
                }
            }
            _ => return Err(bad("Each assignment needs exactly one user or one group".into())),
        }
        if !seen.insert((user_id.clone(), group_id.clone())) {
            continue; // same target listed twice: keep the first
        }
        grants.push((
            user_id,
            group_id,
            ConnectionUserPerms {
                can_connect: g.can_connect,
                can_edit: g.can_edit,
                can_clipboard: g.can_clipboard,
                can_transfer: g.can_transfer,
            },
        ));
    }

    state
        .db
        .replace_connection_permissions(&id, &grants)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "PERMISSIONS_CHANGED",
        Some(&conn.id),
        Some(&conn.name),
        Some(&format!("{} assignment(s)", grants.len())),
        None,
    );
    Ok(Json(json!({ "message": "Assignments saved", "count": grants.len() })))
}

// ================= Connection Authorization =================

/// A connection the caller may use, with their effective permissions.
pub struct AuthorizedConnection {
    pub user: User,
    pub conn: ConnectionRecord,
    pub perms: ConnectionUserPerms,
}

/// The single permission check for using a connection (sessions and SFTP):
/// active account, existing connection, and admin / owner / grant / global access.
fn authorize_connection(
    state: &AppState,
    claims: &Claims,
    connection_id: &str,
    action: &str,
) -> Result<AuthorizedConnection, (StatusCode, Json<Value>)> {
    let internal = |e: rusqlite::Error| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));

    let user = state
        .db
        .get_user_by_id(&claims.sub)
        .map_err(internal)?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({ "error": "User not found" }))))?;
    if !user.is_active {
        return Err((StatusCode::FORBIDDEN, Json(json!({ "error": "Account is disabled" }))));
    }

    let conn = state
        .db
        .get_connection_raw(connection_id)
        .map_err(internal)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "Connection not found" }))))?;

    let perms = state
        .db
        .list_connections_for_user(&user)
        .map_err(internal)?
        .into_iter()
        .find(|c| c.id == conn.id)
        .map(|c| c.user_permissions)
        .filter(|p| p.can_connect);

    match perms {
        Some(perms) => Ok(AuthorizedConnection { user, conn, perms }),
        None => {
            state.db.log_audit(
                Some(&user.id),
                &user.username,
                "ACCESS_DENIED",
                Some(&conn.id),
                Some(&conn.name),
                Some(action),
                None,
            );
            Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Access denied: you do not have permission to use this connection." })),
            ))
        }
    }
}

/// Refuse an SFTP action, recording it in the audit log.
fn deny_sftp(state: &AppState, a: &AuthorizedConnection, action: &str, reason: &str) -> (StatusCode, Json<Value>) {
    state.db.log_audit(
        Some(&a.user.id),
        &a.user.username,
        "ACCESS_DENIED",
        Some(&a.conn.id),
        Some(&a.conn.name),
        Some(&format!("{}: {}", action, reason)),
        None,
    );
    (StatusCode::FORBIDDEN, Json(json!({ "error": format!("Access denied: {}", reason) })))
}

// ================= Files API =================

#[derive(Deserialize)]
pub struct FilesPathQuery {
    pub path: Option<String>,
    /// "my" (default) or "shared".
    pub area: Option<String>,
}

#[derive(Deserialize)]
pub struct FilesMkdirPayload {
    pub path: Option<String>,
    pub area: Option<String>,
    pub name: String,
}

#[derive(Deserialize)]
pub struct FilesDeletePayload {
    pub path: String,
    pub area: Option<String>,
}

fn files_err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg.into() })))
}

/// Uploading and deleting need the Operator or Admin role; Viewers can browse and download.
fn require_files_writer(claims: &Claims) -> Result<(), (StatusCode, Json<Value>)> {
    if claims.role != "admin" && claims.role != "operator" {
        return Err(files_err(StatusCode::FORBIDDEN, "Operator or Admin role required"));
    }
    Ok(())
}

/// May this user change files in this area? My files: Operator or Admin. Shared: Admin.
fn area_writable(claims: &Claims, area: Area) -> bool {
    match area {
        Area::Mine => claims.role == "admin" || claims.role == "operator",
        Area::Shared => claims.role == "admin",
    }
}

/// The file manager for the requested area, refusing changes the user may not make.
fn files_area(
    state: &AppState,
    claims: &Claims,
    area: Option<&str>,
    write: bool,
) -> Result<(TransferManager, Area), (StatusCode, Json<Value>)> {
    let area = Area::parse(area).ok_or_else(|| files_err(StatusCode::BAD_REQUEST, "Unknown file area"))?;
    if write {
        require_files_writer(claims)?;
        if !area_writable(claims, area) {
            return Err(files_err(StatusCode::FORBIDDEN, "Only admins can change the Shared folder"));
        }
    }
    let scope = state
        .transfer_mgr
        .scope(area, &claims.sub)
        .map_err(|e| files_err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((scope, area))
}

/// Audit label like "shared/scripts/setup.ps1".
fn area_path(area: Area, rel: &str) -> String {
    format!("{}/{}", area.label(), rel)
}

/// `Content-Disposition` value that is safe for any file name (ASCII fallback + UTF-8 name).
fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || " .-_()[]".contains(c) { c } else { '_' })
        .collect();
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        fallback,
        urlencoding::encode(name)
    )
}

pub async fn api_files_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FilesPathQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    let (scope, area) = files_area(&state, &claims, q.area.as_deref(), false)?;

    let (path, entries) = scope
        .list(q.path.as_deref().unwrap_or(""))
        .map_err(|e| files_err(StatusCode::NOT_FOUND, e))?;

    Ok(Json(json!({
        "path": path,
        "entries": entries,
        "area": area.label(),
        "writable": area_writable(&claims, area),
    })))
}

pub async fn api_files_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FilesPathQuery>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;

    let (scope, area) = files_area(&state, &claims, q.area.as_deref(), false)?;
    let file = scope
        .file(q.path.as_deref().unwrap_or(""))
        .map_err(|e| files_err(StatusCode::NOT_FOUND, e))?;

    let handle = tokio::fs::File::open(&file.abs)
        .await
        .map_err(|_| files_err(StatusCode::NOT_FOUND, "File not found"))?;
    let size = handle.metadata().await.map(|m| m.len()).unwrap_or(0);
    let name = file
        .abs
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file.bin".to_string());

    state
        .db
        .log_transfer(None, Some(&claims.sub), &area_path(area, &file.rel), size, "download", "completed");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size)
        .header(header::CONTENT_DISPOSITION, content_disposition(&name))
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(handle)))
        .map_err(|e| files_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn api_files_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FilesPathQuery>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    use tokio::io::AsyncWriteExt;

    let claims = extract_claims(&state, &headers)?;
    let (scope, area) = files_area(&state, &claims, q.area.as_deref(), true)?;

    let dir = scope
        .folder(q.path.as_deref().unwrap_or(""))
        .map_err(|e| files_err(StatusCode::NOT_FOUND, e))?;

    let mut saved = Vec::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| files_err(StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let name = crate::transfer::sanitize_name(field.file_name().unwrap_or("upload.bin"))
            .map_err(|e| files_err(StatusCode::BAD_REQUEST, e))?;

        // Stream to a temporary file so large uploads never sit in memory.
        let part = TransferManager::part_path(&dir.abs);
        let mut out = tokio::fs::File::create(&part)
            .await
            .map_err(|e| files_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut size: u64 = 0;
        let result: Result<(), String> = async {
            while let Some(chunk) = field.chunk().await.map_err(|e| e.to_string())? {
                size += chunk.len() as u64;
                out.write_all(&chunk).await.map_err(|e| e.to_string())?;
            }
            out.flush().await.map_err(|e| e.to_string())
        }
        .await;
        drop(out);

        if let Err(e) = result {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(files_err(StatusCode::BAD_REQUEST, format!("Upload failed: {}", e)));
        }

        let rel = scope.finish_upload(&dir, &part, &name).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            files_err(StatusCode::INTERNAL_SERVER_ERROR, e)
        })?;

        state
            .db
            .log_transfer(None, Some(&claims.sub), &area_path(area, &rel), size, "upload", "completed");
        saved.push(json!({ "path": rel, "size": size }));
    }

    Ok(Json(json!({ "files": saved })))
}

pub async fn api_files_mkdir(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<FilesMkdirPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    let (scope, _) = files_area(&state, &claims, payload.area.as_deref(), true)?;

    let rel = scope
        .mkdir(payload.path.as_deref().unwrap_or(""), &payload.name)
        .map_err(|e| files_err(StatusCode::BAD_REQUEST, e))?;

    Ok(Json(json!({ "path": rel })))
}

pub async fn api_files_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<FilesDeletePayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    let (scope, area) = files_area(&state, &claims, payload.area.as_deref(), true)?;

    let (rel, was_dir) = scope
        .delete(&payload.path)
        .map_err(|e| files_err(StatusCode::BAD_REQUEST, e))?;

    state.db.log_transfer(
        None,
        Some(&claims.sub),
        &area_path(area, &rel),
        0,
        if was_dir { "delete_folder" } else { "delete" },
        "completed",
    );

    Ok(Json(json!({ "deleted": rel })))
}

#[derive(Deserialize)]
pub struct SftpListPayload {
    pub path: Option<String>,
}

pub async fn api_sftp_list(
    State(state): State<AppState>,
    AxumPath(connection_id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<SftpListPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&state, &headers)?;
    let authz = authorize_connection(&state, &claims, &connection_id, "SFTP list")?;

    if authz.conn.protocol != "ssh" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "SFTP only available for SSH connections" })),
        ));
    }
    if !authz.perms.can_transfer {
        return Err(deny_sftp(&state, &authz, "SFTP list", "file transfer is disabled for this connection"));
    }
    let conn_rec = authz.conn;

    let password = conn_rec
        .password_enc
        .and_then(|p| decrypt_secret(&p, &cfg.server.jwt_secret).ok());
    let private_key = conn_rec
        .private_key_enc
        .and_then(|k| decrypt_secret(&k, &cfg.server.jwt_secret).ok());

    let certificate = if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&conn_rec.settings_json) {
        settings.get("ssh_certificate")
            .or_else(|| settings.get("certificate"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    } else {
        None
    };

    let conn_label = conn_rec.name.clone();
    let params = SshConnectionParams {
        host: conn_rec.host,
        port: conn_rec.port,
        username: conn_rec.username.unwrap_or_else(|| "root".into()),
        password,
        private_key,
        certificate,
        passphrase: None,
    };

    let remote_dir = payload.path.unwrap_or_else(|| ".".into());

    let list = tokio::task::spawn_blocking(move || {
        let sess = create_ssh_session(&params)?;
        let sftp = sess
            .sftp()
            .map_err(|e| format!("Failed to start SFTP: {}", e))?;
        sftp_list_directory(&sftp, &remote_dir)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
    .map_err(|e| {
        tracing::warn!("SFTP for '{}' failed: {}", conn_label, e);
        (StatusCode::BAD_GATEWAY, Json(json!({ "error": e })))
    })?;

    Ok(Json(json!({ "entries": list })))
}

#[derive(Deserialize)]
pub struct SftpUploadStagedPayload {
    /// File path inside the file area.
    pub file_path: String,
    /// "my" (default) or "shared".
    pub area: Option<String>,
    pub remote_path: String,
}

pub async fn api_sftp_upload_staged(
    State(state): State<AppState>,
    AxumPath(connection_id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<SftpUploadStagedPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&state, &headers)?;
    let authz = authorize_connection(&state, &claims, &connection_id, "SFTP upload")?;

    let reason = if authz.conn.protocol != "ssh" {
        Some("SFTP is only available for SSH connections")
    } else if claims.role != "admin" && claims.role != "operator" {
        Some("sending files needs the Operator or Admin role")
    } else if authz.conn.view_only {
        Some("this connection is view-only")
    } else if !authz.perms.can_transfer
        || !matches!(authz.conn.allow_transfer.as_str(), "full" | "upload_only")
    {
        Some("uploads are not allowed for this connection")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(deny_sftp(&state, &authz, "SFTP upload", reason));
    }

    let (scope, _) = files_area(&state, &claims, payload.area.as_deref(), false)?;
    let local_path = scope
        .file(&payload.file_path)
        .map_err(|e| files_err(StatusCode::NOT_FOUND, e))?
        .abs;

    let conn_rec = authz.conn;

    let password = conn_rec
        .password_enc
        .and_then(|p| decrypt_secret(&p, &cfg.server.jwt_secret).ok());
    let private_key = conn_rec
        .private_key_enc
        .and_then(|k| decrypt_secret(&k, &cfg.server.jwt_secret).ok());

    let certificate = if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&conn_rec.settings_json) {
        settings.get("ssh_certificate")
            .or_else(|| settings.get("certificate"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    } else {
        None
    };

    let conn_label = conn_rec.name.clone();
    let params = SshConnectionParams {
        host: conn_rec.host,
        port: conn_rec.port,
        username: conn_rec.username.unwrap_or_else(|| "root".into()),
        password,
        private_key,
        certificate,
        passphrase: None,
    };

    let rem_path = payload.remote_path.clone();
    let bytes_transferred = tokio::task::spawn_blocking(move || {
        let sess = create_ssh_session(&params)?;
        let sftp = sess
            .sftp()
            .map_err(|e| format!("Failed to start SFTP: {}", e))?;
        sftp_upload_file(&sftp, &local_path, &rem_path)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
    .map_err(|e| {
        tracing::warn!("SFTP for '{}' failed: {}", conn_label, e);
        (StatusCode::BAD_GATEWAY, Json(json!({ "error": e })))
    })?;

    state.db.log_transfer(
        Some(&connection_id),
        Some(&claims.sub),
        &payload.remote_path,
        bytes_transferred,
        "upload_to_remote",
        "completed",
    );

    Ok(Json(json!({
        "message": "File uploaded to remote host successfully",
        "bytes_transferred": bytes_transferred,
    })))
}

// ================= Audit & Settings API =================

pub async fn api_list_audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    let logs = state.db.list_audit_logs(1000).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!(logs)))
}

/// Admin: read-only server settings plus status checks for the Admin Control Panel.
pub async fn api_get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let cfg = state.config.read().clone();
    let data_dir = std::path::Path::new(&cfg.server.data_dir);
    let secret_file = data_dir.join(crate::secrets::SECRET_FILE);
    let admin_user = state.db.get_user_by_username("admin").ok().flatten();
    let all_conns = state.db.get_user_by_id(&claims.sub).ok().flatten()
        .and_then(|u| state.db.list_connections_for_user(&u).ok())
        .unwrap_or_default();
    let global: Vec<String> = all_conns.iter().filter(|c| c.is_global).map(|c| c.name.clone()).collect();
    let global_local_shell = all_conns.iter().any(|c| c.is_global && c.protocol == "local_pty");
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "status": {
            "secret_source": if secret_file.exists() { "file" } else { "config" },
            "secret_file": secret_file.display().to_string(),
            "admin_note_present": data_dir.join(crate::secrets::ADMIN_NOTE_FILE).exists(),
            "admin_must_change": admin_user.map(|a| state.db.get_user_security(&a.id).map(|s| s.1).unwrap_or(false)).unwrap_or(false),
            "global_connections": global,
            "global_local_shell": global_local_shell,
            "themes": state.themes.len(),
            "local_users_without_2fa": security::local_users_without_2fa(&state),
            "email_ready": email::EmailSettings::load(&state.db).ready(),
            "local_users_without_email": state.db.list_users().unwrap_or_default().into_iter()
                .filter(|u| u.is_active && u.auth_provider == "local" && u.email.as_deref().map(|e| !e.trim().is_empty()) != Some(true))
                .map(|u| u.username).collect::<Vec<_>>(),
            "require_2fa_roles": state.policy.read().require_2fa_roles.clone(),
        },
        "auth": {
            "allow_local_login": cfg.auth.allow_local_login,
            "allow_pam_login": cfg.auth.allow_pam_login,
            "default_role": cfg.auth.default_role,
            "min_password_length": cfg.auth.min_password_length,
        },
        "server": {
            "bind_address": cfg.server.bind_address,
            "data_dir": cfg.server.data_dir,
            "token_expiry_hours": state.policy.read().max_login_hours,
        },
        "storage": {
            "staging_dir": cfg.storage.staging_dir,
            "max_upload_size_mb": cfg.storage.max_upload_size_mb,
        },
        "clipboard": {
            "default_mode": cfg.clipboard.default_mode,
            "max_text_size_bytes": cfg.clipboard.max_text_size_bytes,
        },
        "oidc": {
            "enabled": cfg.oidc.enabled,
            "provider_name": cfg.oidc.provider_name,
            "issuer_url": cfg.oidc.issuer_url,
            "client_id": cfg.oidc.client_id,
            "redirect_uri": cfg.oidc.redirect_uri,
            "scopes": cfg.oidc.scopes,
            "admin_group": cfg.oidc.admin_group,
            "operator_group": cfg.oidc.operator_group,
        }
    })))
}

pub async fn api_list_themes(
    State(state): State<AppState>,
) -> Json<Value> {
    // Public: the login screen uses the remembered theme before anyone signs in.
    Json(json!({ "default": crate::themes::DEFAULT_THEME, "themes": *state.themes }))
}

pub async fn api_get_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    let raw = state.db.get_user_preferences(&claims.sub).unwrap_or_else(|_| "{}".into());
    Ok(Json(serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))))
}

/// Save interface preferences (theme and similar). Merged into what is stored; max 16 KB.
pub async fn api_set_preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    let Value::Object(update) = payload else {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "Preferences must be an object" }))));
    };
    let raw = state.db.get_user_preferences(&claims.sub).unwrap_or_else(|_| "{}".into());
    let mut prefs: serde_json::Map<String, Value> = serde_json::from_str(&raw).unwrap_or_default();
    for (k, v) in update {
        if v.is_null() {
            prefs.remove(&k);
        } else {
            prefs.insert(k, v);
        }
    }
    let text = Value::Object(prefs.clone()).to_string();
    if text.len() > 16 * 1024 {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, Json(json!({ "error": "Preferences too large" }))));
    }
    state
        .db
        .set_user_preferences(&claims.sub, &text)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    Ok(Json(Value::Object(prefs)))
}

#[derive(Deserialize)]
pub struct ScopePayload {
    pub is_global: bool,
}

/// Admin: make a connection global or private in one click.
pub async fn api_set_connection_scope(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<ScopePayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let conn = state
        .db
        .get_connection_raw(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "Connection not found" }))))?;
    state
        .db
        .set_connection_global(&id, payload.is_global)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "CONNECTION_SCOPE",
        Some(&conn.id),
        Some(&conn.name),
        Some(if payload.is_global { "Made global" } else { "Made private" }),
        None,
    );
    Ok(Json(json!({ "is_global": payload.is_global })))
}

// ================= WebSocket Gateway Handler =================

#[derive(Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub width: Option<u16>,
    pub height: Option<u16>,
    /// Keyboard layout chosen in the browser for this session (overrides the connection's).
    pub kbd: Option<String>,
}

pub async fn ws_tunnel_handler(
    State(state): State<AppState>,
    AxumPath(connection_id): AxumPath<String>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    ip: ClientIp,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();

    // Authenticate token either via query param or header/cookie
    let claims = if let Some(tok) = &query.token {
        authenticate_token(&state, tok, false)?
    } else {
        extract_claims(&state, &headers)?
    };

    let authz = authorize_connection(&state, &claims, &connection_id, "Connect")?;
    let (user, conn_rec, conn_perms) = (authz.user, authz.conn, authz.perms);

    state.db.log_audit(
        Some(&user.id),
        &user.username,
        "CONNECT",
        Some(&conn_rec.id),
        Some(&conn_rec.name),
        Some(&format!("Protocol: {}", conn_rec.protocol)),
        ip.as_opt(),
    );

    let cols = query.cols.unwrap_or(100);
    let rows = query.rows.unwrap_or(30);

    let password = conn_rec
        .password_enc
        .and_then(|p| decrypt_secret(&p, &cfg.server.jwt_secret).ok());
    let private_key = conn_rec
        .private_key_enc
        .and_then(|k| decrypt_secret(&k, &cfg.server.jwt_secret).ok());

    // Every session runs behind a relay that can share it with others (#32).
    let hub_cfg = {
        let db = state.db.clone();
        let (owner_id, owner_login) = (user.id.clone(), user.username.clone());
        let (conn_id, conn_name) = (conn_rec.id.clone(), conn_rec.name.clone());
        let ip_text = ip.as_opt().map(str::to_string);
        crate::protocols::hub::HubConfig {
            owner_name: user.display_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| user.username.clone()),
            conn_name: conn_rec.name.clone(),
            conn_icon: conn_rec.icon.clone(),
            protocol: conn_rec.protocol.clone(),
            kind: if matches!(conn_rec.protocol.as_str(), "rdp" | "vnc") {
                crate::protocols::hub::SessionKind::Graphics
            } else {
                crate::protocols::hub::SessionKind::Terminal
            },
            cols,
            rows,
            guest_clipboard: conn_perms.can_clipboard && matches!(conn_rec.allow_clipboard.as_str(), "bidirectional" | "host_to_remote"),
            share_rights: {
                let policy = state.policy.clone();
                Box::new(move || policy.read().share_rights())
            },
            on_event: Box::new(move |ev| {
                use crate::protocols::hub::ShareEvent;
                let (who_id, who, action, details) = match ev {
                    ShareEvent::Created { mode, minutes } => (
                        owner_id.clone(),
                        owner_login.clone(),
                        "SHARE_CREATED",
                        format!("{} link, {}", mode.as_str(), minutes.map(|m| format!("{m} minutes")).unwrap_or_else(|| "until the session ends".into())),
                    ),
                    ShareEvent::Revoked => (owner_id.clone(), owner_login.clone(), "SHARE_REVOKED", "link cancelled".into()),
                    ShareEvent::Joined { guest_id, guest_name, mode } => {
                        (guest_id, guest_name, "SHARE_JOINED", format!("joined {owner_login}'s session ({})", mode.as_str()))
                    }
                    ShareEvent::Left { guest_id, guest_name } => (guest_id, guest_name, "SHARE_LEFT", format!("left {owner_login}'s session")),
                    ShareEvent::Removed { guest_name } => (owner_id.clone(), owner_login.clone(), "SHARE_REMOVED", format!("removed {guest_name}")),
                };
                db.log_audit(Some(&who_id), &who, action, Some(&conn_id), Some(&conn_name), Some(&details), ip_text.as_deref());
            }),
        }
    };
    let shares = state.shares.clone();

    Ok(ws.on_upgrade(move |socket| async move {
        let mut socket = crate::protocols::hub::start(socket, hub_cfg, shares);
        match conn_rec.protocol.as_str() {
            "local_pty" => {
                let shell = serde_json::from_str::<Value>(&conn_rec.settings_json)
                    .ok()
                    .and_then(|s| s.get("shell").and_then(|v| v.as_str()).map(|v| v.to_string()));
                handle_local_pty_session(socket, cols, rows, shell).await;
            }
            "ssh" => {
                let certificate = if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&conn_rec.settings_json) {
                    settings.get("ssh_certificate")
                        .or_else(|| settings.get("certificate"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.to_string())
                } else {
                    None
                };

                let params = SshConnectionParams {
                    host: conn_rec.host,
                    port: conn_rec.port,
                    username: conn_rec.username.unwrap_or_else(|| "root".into()),
                    password,
                    private_key,
                    certificate,
                    passphrase: None,
                };
                handle_ssh_session(socket, params, cols, rows).await;
            }
            "telnet" => {
                let params = TelnetConnectionParams {
                    host: conn_rec.host,
                    port: conn_rec.port,
                };
                handle_telnet_session(socket, params, cols, rows).await;
            }
            "vnc" => {
                let params = VncConnectionParams {
                    host: conn_rec.host,
                    port: conn_rec.port,
                    username: conn_rec.username,
                    password,
                };
                handle_vnc_session(socket, params).await;
            }
            "rdp" => {
                let mut ignore_cert = true;
                let mut domain = None;
                let mut color_depth = 32u32;
                let mut enable_audio = false;
                let mut disable_wallpaper = true;
                let mut disable_full_window_drag = true;
                let mut disable_menu_animations = true;
                let mut disable_themes = false;
                let mut font_smoothing = true;

                let mut enable_drive_redirection = false;
                let mut keyboard_layout = None;

                if let Ok(settings) = serde_json::from_str::<serde_json::Value>(&conn_rec.settings_json) {
                    if let Some(ic) = settings.get("ignore_cert").and_then(|v| v.as_bool()) {
                        ignore_cert = ic;
                    }
                    if let Some(d) = settings.get("domain").and_then(|v| v.as_str()) {
                        if !d.trim().is_empty() {
                            domain = Some(d.trim().to_string());
                        }
                    }
                    if let Some(cd) = settings.get("color_depth").and_then(|v| v.as_u64()) {
                        if cd == 16 || cd == 32 {
                            color_depth = cd as u32;
                        }
                    }
                    if let Some(ea) = settings.get("enable_audio").and_then(|v| v.as_bool()) {
                        enable_audio = ea;
                    }
                    if let Some(dw) = settings.get("disable_wallpaper").and_then(|v| v.as_bool()) {
                        disable_wallpaper = dw;
                    }
                    if let Some(df) = settings.get("disable_window_drag").and_then(|v| v.as_bool()) {
                        disable_full_window_drag = df;
                    }
                    if let Some(dm) = settings.get("disable_menu_anim").and_then(|v| v.as_bool()) {
                        disable_menu_animations = dm;
                    }
                    if let Some(dt) = settings.get("disable_themes").and_then(|v| v.as_bool()) {
                        disable_themes = dt;
                    }
                    if let Some(fs) = settings.get("font_smoothing").and_then(|v| v.as_bool()) {
                        font_smoothing = fs;
                    }
                    if let Some(edr) = settings.get("enable_drive_redirection").and_then(|v| v.as_bool()) {
                        enable_drive_redirection = edr && conn_perms.can_transfer;
                    }
                    if let Some(kl) = settings.get("keyboard_layout").and_then(|v| v.as_str()) {
                        if !kl.trim().is_empty() {
                            keyboard_layout = Some(kl.trim().to_string());
                        }
                    }
                }

                // The browser sends the layout it resolved ("auto" becomes a real layout there).
                if let Some(k) = query.kbd.as_deref().map(str::trim).filter(|k| crate::protocols::rdp::windows_keyboard_layout(k).is_some()) {
                    keyboard_layout = Some(k.to_string());
                }

                let init_w = query.width.unwrap_or(1920).clamp(640, 3840);
                let init_h = query.height.unwrap_or(1080).clamp(480, 2160);

                let params = RdpConnectionParams {
                    host: conn_rec.host,
                    port: conn_rec.port,
                    username: conn_rec.username,
                    password,
                    domain,
                    ignore_cert,
                    width: init_w,
                    height: init_h,
                    color_depth,
                    enable_audio,
                    disable_wallpaper,
                    disable_full_window_drag,
                    disable_menu_animations,
                    disable_themes,
                    font_smoothing,
                    drives: if enable_drive_redirection {
                        session_drives(&state, &user)
                    } else {
                        Vec::new()
                    },
                    keyboard_layout,
                };
                handle_rdp_session(socket, params).await;
            }
            _ => {
                let _ = socket
                    .send(axum::extract::ws::Message::Text(
                        json!({
                            "type": "error",
                            "message": format!("Unsupported protocol '{}'", conn_rec.protocol)
                        })
                        .to_string(),
                    ))
                    .await;
            }
        }
    }))
}

// ================= Static File Handler =================

/// HTML pages get `__REMOTE_VERSION__` replaced with the running version (asset cache-busting).
fn page_body(path: &str, data: std::borrow::Cow<'static, [u8]>) -> Body {
    if path.ends_with(".html") {
        let html = String::from_utf8_lossy(&data).replace("__REMOTE_VERSION__", env!("CARGO_PKG_VERSION"));
        Body::from(html)
    } else {
        Body::from(data)
    }
}

pub async fn static_file_handler(uri: axum::http::Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".to_string();
    }

    if let Some(file) = FrontendAssets::get(&path) {
        let mime = mime_guess::from_path(&path).first_or_octet_stream();
        let cache_control = if path.ends_with(".html") || path.ends_with(".js") || path.ends_with(".css") {
            "no-cache, must-revalidate"
        } else {
            "public, max-age=86400"
        };

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache_control)
            .body(page_body(&path, file.data))
            .unwrap()
    } else if let Some(index) = FrontendAssets::get("index.html") {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache, must-revalidate")
            .body(page_body("index.html", index.data))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap()
    }
}
