use crate::auth::{
    create_jwt, decrypt_secret, encrypt_secret, hash_password, verify_jwt, verify_password,
    Claims, OidcService,
};
use crate::config::AppConfig;
use crate::db::{ConnectionRecord, ConnectionSafe, Database, Group, User, UserSafe};
use crate::protocols::local_pty::handle_local_pty_session;
use crate::protocols::rdp::{handle_rdp_session, RdpConnectionParams};
use crate::protocols::ssh::{
    create_ssh_session, handle_ssh_session, sftp_list_directory,
    sftp_upload_file, SshConnectionParams,
};
use crate::protocols::telnet::{handle_telnet_session, TelnetConnectionParams};
use crate::protocols::vnc::{handle_vnc_session, VncConnectionParams};
use crate::transfer::TransferManager;
use axum::{
    body::Body,
    extract::{
        ws::WebSocketUpgrade,
        Multipart, Path as AxumPath, Query, State,
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
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(RustEmbed)]
#[folder = "frontend/"]
pub struct FrontendAssets;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub db: Database,
    pub transfer_mgr: TransferManager,
    pub oidc_service: Arc<OidcService>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Auth API
        .route("/api/auth/login", post(api_login))
        .route("/api/auth/logout", post(api_logout))
        .route("/api/auth/me", get(api_me))
        .route("/api/auth/providers", get(api_auth_providers))
        .route("/api/auth/oidc/login", get(api_oidc_login))
        .route("/api/auth/oidc/callback", get(api_oidc_callback))
        // Users & Groups API
        .route("/api/users", get(api_list_users).post(api_create_user))
        .route(
            "/api/users/:id",
            put(api_update_user).delete(api_delete_user),
        )
        .route("/api/groups", get(api_list_groups).post(api_create_group))
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
        // File Transfers & Dropboxes API
        .route("/api/transfers/upload", post(api_upload_file))
        .route(
            "/api/transfers/download/:id",
            get(api_download_staged_file),
        )
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
        // WebSocket Remote Gateway Tunnel
        .route("/ws/tunnel/:connection_id", get(ws_tunnel_handler))
        // Static Frontend Files
        .fallback(static_file_handler)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ================= Auth Helpers & Middlewares =================

fn extract_claims(headers: &HeaderMap, secret: &str) -> Result<Claims, (StatusCode, Json<Value>)> {
    if let Some(auth_header) = headers.get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                return verify_jwt(token, secret)
                    .map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))));
            }
        }
    }

    if let Some(cookie_header) = headers.get(header::COOKIE) {
        if let Ok(cookies) = cookie_header.to_str() {
            for c in cookies.split(';') {
                let parts: Vec<&str> = c.trim().split('=').collect();
                if parts.len() == 2 && (parts[0] == "remote_token" || parts[0] == "remotedog_token") {
                    return verify_jwt(parts[1], secret)
                        .map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))));
                }
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Authentication token missing or invalid" })),
    ))
}

// ================= Auth Handlers =================

#[derive(Deserialize)]
pub struct LoginPayload {
    pub username: String,
    pub password: Option<String>,
}

pub async fn api_login(
    State(state): State<AppState>,
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

    let user = match user_opt {
        Some(u) => u,
        None => {
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

    let password = payload.password.unwrap_or_default();
    let mut is_valid = verify_password(&password, &user.password_hash);
    if !is_valid && user.username == "admin" {
        if (password == "remote" || password == "remotedog")
            && (verify_password("remote", &user.password_hash) || verify_password("remotedog", &user.password_hash))
        {
            is_valid = true;
        }
    }

    if !is_valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Invalid username or password" })),
        ));
    }

    let token = create_jwt(&user, &cfg.server.jwt_secret, cfg.server.token_expiry_hours)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;

    let _ = state.db.update_last_login(&user.id);
    state.db.log_audit(
        Some(&user.id),
        &user.username,
        "LOGIN_SUCCESS",
        None,
        None,
        Some("User logged in via local auth"),
        None,
    );

    let safe_user: UserSafe = user.into();
    Ok(Json(json!({
        "token": token,
        "user": safe_user,
    })))
}

pub async fn api_logout() -> Json<Value> {
    Json(json!({ "message": "Logged out successfully" }))
}

pub async fn api_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;

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

    let safe_user: UserSafe = user.into();
    Ok(Json(json!(safe_user)))
}

pub async fn api_auth_providers(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.read().clone();
    Json(json!({
        "local": cfg.auth.allow_local_login,
        "pam": cfg.auth.allow_pam_login,
        "oidc": {
            "enabled": cfg.oidc.enabled,
            "provider_name": cfg.oidc.provider_name,
        }
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

    let token = create_jwt(&user, &cfg.server.jwt_secret, cfg.server.token_expiry_hours)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;

    let html = format!(
        r#"<!DOCTYPE html>
        <html>
        <head><title>Remote OIDC Authenticated</title></head>
        <body style="background:#121214;color:#f59e0b;font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;">
          <div style="text-align:center;">
            <img src="/assets/Remoteiconsmall.png" alt="Remote" style="height:44px;margin-bottom:12px;vertical-align:middle;" />
            <h2 style="margin:0;font-weight:700;color:#f4f4f5;">Remote Authenticated!</h2>
            <p style="color:#a1a1aa;margin-top:6px;">Redirecting to dashboard...</p>
          </div>
          <script>
            localStorage.setItem('remote_token', '{}');
            document.cookie = 'remote_token={}; path=/; max-age=86400; SameSite=Lax';
            window.location.href = '/';
          </script>
        </body>
        </html>"#,
        token, token
    );

    Ok(Html(html).into_response())
}

// ================= Users Management API =================

pub async fn api_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserSafe>>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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
    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct CreateUserPayload {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
}

pub async fn api_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserPayload>,
) -> Result<Json<UserSafe>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    let pw_hash = hash_password(&payload.password)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;

    let new_user = User {
        id: Uuid::new_v4().to_string(),
        username: payload.username,
        password_hash: pw_hash,
        email: payload.email,
        display_name: payload.display_name,
        role: payload.role.unwrap_or_else(|| "operator".to_string()),
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
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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

    if claims.role == "admin" {
        if let Some(r) = payload.role {
            user.role = r;
        }
        if let Some(active) = payload.is_active {
            user.is_active = active;
        }
    }

    let pw_opt = payload.new_password.or(payload.password);
    if let Some(pw) = pw_opt {
        if !pw.trim().is_empty() {
            user.password_hash = hash_password(&pw)
                .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))))?;
        }
    }

    state.db.update_user(&user).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    let safe_user: UserSafe = user.into();
    Ok(Json(json!({ "message": "User updated successfully", "user": safe_user })))
}

pub async fn api_delete_user(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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
) -> Result<Json<Vec<Group>>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let _ = extract_claims(&headers, &cfg.server.jwt_secret)?;
    let groups = state.db.list_groups().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;
    Ok(Json(groups))
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
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;

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
}

pub async fn api_create_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SaveConnectionPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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
        icon: payload.icon,
        tags: payload.tags,
        is_global,
        allow_clipboard,
        allow_transfer,
        view_only,
        created_by: Some(claims.sub),
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
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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
    if !is_admin && !is_owner {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Access denied: only the owner or an administrator can modify this connection" })),
        ));
    }

    record.name = payload.name;
    record.protocol = payload.protocol;
    record.host = payload.host;
    record.port = payload.port;
    record.username = payload.username;
    record.settings_json = payload.settings_json.unwrap_or(record.settings_json);
    record.icon = payload.icon;
    record.tags = payload.tags;
    if is_admin {
        if let Some(g) = payload.is_global {
            record.is_global = g;
        }
    }
    if let Some(clip) = payload.allow_clipboard {
        record.allow_clipboard = clip;
    }
    if let Some(trans) = payload.allow_transfer {
        record.allow_transfer = trans;
    }
    if let Some(vo) = payload.view_only {
        record.view_only = vo;
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
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
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

pub async fn api_test_connection(
    Json(payload): Json<TestConnectionPayload>,
) -> Json<Value> {
    let addr = format!("{}:{}", payload.host, payload.port);
    let reachable = tokio::task::spawn_blocking(move || {
        std::net::TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
            std::time::Duration::from_secs(4),
        )
        .is_ok()
    })
    .await
    .unwrap_or(false);

    Json(json!({ "reachable": reachable, "host": payload.host, "port": payload.port }))
}

// ================= File Transfers & Dropboxes API =================

pub async fn api_upload_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;

    let mut staged_files = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))))?
    {
        let original_name = field
            .file_name()
            .unwrap_or("upload.bin")
            .to_string();
        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        let data = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))))?;

        let staged = state
            .transfer_mgr
            .save_upload(original_name, content_type, data)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))))?;

        state.db.log_transfer(
            None,
            Some(&claims.sub),
            &staged.original_name,
            staged.file_size,
            "upload",
            "staged",
        );

        staged_files.push(staged);
    }

    Ok(Json(json!({ "files": staged_files })))
}

pub async fn api_download_staged_file(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    if let Some(path) = state.transfer_mgr.get_staged_file(&id) {
        if let Ok(data) = tokio::fs::read(&path).await {
            let filename = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file.bin".to_string());

            let clean_name = if filename.contains('_') {
                filename.split_once('_').map(|(_, b)| b).unwrap_or(&filename)
            } else {
                &filename
            };

            let resp = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", clean_name),
                )
                .body(Body::from(data))
                .unwrap();
            return Ok(resp);
        }
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "Staged file not found" })),
    ))
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
    let _ = extract_claims(&headers, &cfg.server.jwt_secret)?;

    let conn_rec = state
        .db
        .get_connection_raw(&connection_id)
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

    if conn_rec.protocol != "ssh" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "SFTP only available for SSH connections" })),
        ));
    }

    let password = conn_rec
        .password_enc
        .and_then(|p| decrypt_secret(&p, &cfg.server.jwt_secret).ok());
    let private_key = conn_rec
        .private_key_enc
        .and_then(|k| decrypt_secret(&k, &cfg.server.jwt_secret).ok());

    let params = SshConnectionParams {
        host: conn_rec.host,
        port: conn_rec.port,
        username: conn_rec.username.unwrap_or_else(|| "root".into()),
        password,
        private_key,
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
    .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))))?;

    Ok(Json(json!({ "entries": list })))
}

#[derive(Deserialize)]
pub struct SftpUploadStagedPayload {
    pub staged_id: String,
    pub remote_path: String,
}

pub async fn api_sftp_upload_staged(
    State(state): State<AppState>,
    AxumPath(connection_id): AxumPath<String>,
    headers: HeaderMap,
    Json(payload): Json<SftpUploadStagedPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;

    let local_path = state
        .transfer_mgr
        .get_staged_file(&payload.staged_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Staged file not found" })),
            )
        })?;

    let conn_rec = state
        .db
        .get_connection_raw(&connection_id)
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

    let password = conn_rec
        .password_enc
        .and_then(|p| decrypt_secret(&p, &cfg.server.jwt_secret).ok());
    let private_key = conn_rec
        .private_key_enc
        .and_then(|k| decrypt_secret(&k, &cfg.server.jwt_secret).ok());

    let params = SshConnectionParams {
        host: conn_rec.host,
        port: conn_rec.port,
        username: conn_rec.username.unwrap_or_else(|| "root".into()),
        password,
        private_key,
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
    .map_err(|e| (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))))?;

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
    let cfg = state.config.read().clone();
    let claims = extract_claims(&headers, &cfg.server.jwt_secret)?;
    if claims.role != "admin" {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Admin role required" })),
        ));
    }

    let logs = state.db.list_audit_logs(100).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
    })?;

    Ok(Json(json!(logs)))
}

pub async fn api_get_settings(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.config.read().clone();
    Json(json!({
        "server": {
            "bind_address": cfg.server.bind_address,
            "data_dir": cfg.server.data_dir,
            "token_expiry_hours": cfg.server.token_expiry_hours,
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
    }))
}

// ================= WebSocket Gateway Handler =================

#[derive(Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub width: Option<u16>,
    pub height: Option<u16>,
}

pub async fn ws_tunnel_handler(
    State(state): State<AppState>,
    AxumPath(connection_id): AxumPath<String>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();

    // Authenticate token either via query param or header/cookie
    let claims = if let Some(tok) = &query.token {
        verify_jwt(tok, &cfg.server.jwt_secret)
            .map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))))?
    } else {
        extract_claims(&headers, &cfg.server.jwt_secret)?
    };

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

    if !user.is_active {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Account is disabled" })),
        ));
    }

    let conn_rec = state
        .db
        .get_connection_raw(&connection_id)
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
    let is_owner = conn_rec.created_by.as_deref() == Some(&user.id);
    if !is_admin && !is_owner && !conn_rec.is_global {
        let user_conns = state.db.list_connections_for_user(&user).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
        if !user_conns.iter().any(|c| c.id == conn_rec.id && c.user_permissions.can_connect) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Access denied: you do not have permission to connect to this resource." })),
            ));
        }
    }

    state.db.log_audit(
        Some(&user.id),
        &user.username,
        "CONNECT",
        Some(&conn_rec.id),
        Some(&conn_rec.name),
        Some(&format!("Protocol: {}", conn_rec.protocol)),
        None,
    );

    let cols = query.cols.unwrap_or(100);
    let rows = query.rows.unwrap_or(30);

    let password = conn_rec
        .password_enc
        .and_then(|p| decrypt_secret(&p, &cfg.server.jwt_secret).ok());
    let private_key = conn_rec
        .private_key_enc
        .and_then(|k| decrypt_secret(&k, &cfg.server.jwt_secret).ok());

    Ok(ws.on_upgrade(move |mut socket| async move {
        match conn_rec.protocol.as_str() {
            "local_pty" => {
                handle_local_pty_session(socket, cols, rows).await;
            }
            "ssh" => {
                let params = SshConnectionParams {
                    host: conn_rec.host,
                    port: conn_rec.port,
                    username: conn_rec.username.unwrap_or_else(|| "root".into()),
                    password,
                    private_key,
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
                        enable_drive_redirection = edr && conn_rec.allow_transfer != "disabled";
                    }
                    if let Some(kl) = settings.get("keyboard_layout").and_then(|v| v.as_str()) {
                        if !kl.trim().is_empty() {
                            keyboard_layout = Some(kl.trim().to_string());
                        }
                    }
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
                    staging_dir: Some(cfg.storage.staging_dir.clone()),
                    enable_drive_redirection,
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
            .body(Body::from(file.data))
            .unwrap()
    } else if let Some(index) = FrontendAssets::get("index.html") {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache, must-revalidate")
            .body(Body::from(index.data))
            .unwrap()
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap()
    }
}
