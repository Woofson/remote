//! Email (#27): SMTP settings, "forgot password" links, set-your-password invitations
//! and security notices (password changed, two-factor turned off, ...).

use super::*;
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg })))
}

fn db_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

pub const EMAIL_SETTINGS_KEY: &str = "email";
/// How long a "forgot password" link works.
const RESET_MINUTES: i64 = 30;
/// How long an admin's set-your-password link works.
const INVITE_HOURS: i64 = 72;

/// SMTP settings, stored in the database (the password encrypted).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EmailSettings {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    /// "starttls", "tls" or "none".
    pub security: String,
    pub username: String,
    /// Encrypted with the server secret; never sent to the browser.
    pub password_enc: Option<String>,
    pub from_name: String,
    pub from_address: String,
    /// Address people open Remote at, for links in emails (e.g. https://example.com/remote).
    pub public_url: String,
}

impl EmailSettings {
    pub fn load(db: &Database) -> Self {
        db.get_setting(EMAIL_SETTINGS_KEY)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    /// Ready to send: switched on and complete.
    pub fn ready(&self) -> bool {
        self.enabled && !self.host.is_empty() && !self.from_address.is_empty() && !self.public_url.is_empty()
    }

    fn base_url(&self) -> String {
        self.public_url.trim().trim_end_matches('/').to_string()
    }
}

// ---------------- Sending ----------------

fn transport(s: &EmailSettings, secret: &str) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let builder = match s.security.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&s.host).map_err(|e| e.to_string())?,
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&s.host),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&s.host).map_err(|e| e.to_string())?,
    };
    let default_port = match s.security.as_str() {
        "tls" => 465,
        "none" => 25,
        _ => 587,
    };
    let mut builder = builder
        .port(if s.port == 0 { default_port } else { s.port })
        .timeout(Some(Duration::from_secs(20)));
    if !s.username.is_empty() {
        let password = s
            .password_enc
            .as_deref()
            .map(|p| decrypt_secret(p, secret))
            .transpose()
            .map_err(|_| "Could not read the saved mail password".to_string())?
            .unwrap_or_default();
        builder = builder.credentials(Credentials::new(s.username.clone(), password));
    }
    Ok(builder.build())
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Send one email (plain text plus a simple HTML version with no remote images).
pub async fn send_mail(s: &EmailSettings, secret: &str, to: &str, subject: &str, lines: &[String], link: Option<(&str, &str)>) -> Result<(), String> {
    let from_name = if s.from_name.trim().is_empty() { "Remote".to_string() } else { s.from_name.clone() };
    let from: Mailbox = format!("{} <{}>", from_name, s.from_address)
        .parse()
        .map_err(|_| "The sender address is not valid".to_string())?;
    let to: Mailbox = to.parse().map_err(|_| "The recipient address is not valid".to_string())?;

    let mut text = lines.join("\n\n");
    let mut html_body = lines
        .iter()
        .map(|l| format!("<p style=\"margin:0 0 14px\">{}</p>", escape_html(l)))
        .collect::<String>();
    if let Some((label, url)) = link {
        text.push_str(&format!("\n\n{}:\n{}", label, url));
        html_body.push_str(&format!(
            "<p style=\"margin:20px 0\"><a href=\"{u}\" style=\"background:#f59e0b;color:#111;padding:10px 18px;border-radius:6px;text-decoration:none;font-weight:600\">{l}</a></p>\
             <p style=\"margin:0 0 14px;color:#666;font-size:12px\">Or copy this address into your browser:<br>{u}</p>",
            u = escape_html(url),
            l = escape_html(label)
        ));
    }
    text.push_str("\n\n-- \nRemote");
    let html = format!(
        "<!doctype html><html><body style=\"font-family:-apple-system,Segoe UI,Roboto,sans-serif;font-size:14px;color:#222;line-height:1.5;max-width:560px\">{}<p style=\"margin-top:24px;color:#888;font-size:12px\">Remote</p></body></html>",
        html_body
    );

    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::builder().header(ContentType::TEXT_PLAIN).body(text))
                .singlepart(SinglePart::builder().header(ContentType::TEXT_HTML).body(html)),
        )
        .map_err(|e| e.to_string())?;
    transport(s, secret)?.send(message).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Send a security notice to a user in the background, if email is set up and they have an address.
pub fn notify_user(state: &AppState, user: &User, subject: &str, what: &str) {
    let settings = EmailSettings::load(&state.db);
    let Some(to) = user.email.clone().filter(|e| e.contains('@')) else { return };
    if !settings.ready() {
        return;
    }
    let secret = state.config.read().server.jwt_secret.clone();
    let lines = vec![
        format!("Hello {},", user.display_name.clone().unwrap_or_else(|| user.username.clone())),
        format!("{} on your Remote account ({}).", what, user.username),
        format!("Time: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")),
        "If this was you, there is nothing to do. If not, contact your administrator straight away.".to_string(),
    ];
    let subject = subject.to_string();
    tokio::spawn(async move {
        if let Err(e) = send_mail(&settings, &secret, &to, &subject, &lines, None).await {
            tracing::warn!("Could not send the security notice: {}", e);
        }
    });
}

// ---------------- Reset links ----------------

fn new_token() -> String {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("OS random source");
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, b)
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.trim().as_bytes()))
}

/// Make a link for `user` and email it. `invite` = an admin's set-your-password link.
async fn send_reset_link(state: &AppState, user: &User, invite: bool) -> Result<(), String> {
    let settings = EmailSettings::load(&state.db);
    if !settings.ready() {
        return Err("Email is not set up".into());
    }
    let to = user.email.clone().filter(|e| e.contains('@')).ok_or("This account has no email address")?;
    let token = new_token();
    let expires = if invite {
        chrono::Utc::now() + chrono::Duration::hours(INVITE_HOURS)
    } else {
        chrono::Utc::now() + chrono::Duration::minutes(RESET_MINUTES)
    };
    state
        .db
        .create_password_reset(&token_hash(&token), &user.id, expires.timestamp(), if invite { "invite" } else { "reset" })
        .map_err(|e| e.to_string())?;
    // A fragment (#...) is never sent to servers or proxies, so the secret stays out of logs.
    let url = format!("{}/#reset={}", settings.base_url(), token);
    let name = user.display_name.clone().unwrap_or_else(|| user.username.clone());
    let secret = state.config.read().server.jwt_secret.clone();
    let (subject, lines, label) = if invite {
        (
            "Choose your Remote password",
            vec![
                format!("Hello {},", name),
                format!("An administrator has set up a Remote account for you. Your username is: {}", user.username),
                format!("Use the button below to choose your password. The link works once, for {} hours.", INVITE_HOURS),
            ],
            "Choose your password",
        )
    } else {
        (
            "Reset your Remote password",
            vec![
                format!("Hello {},", name),
                format!("Someone asked to reset the password of your Remote account ({}).", user.username),
                format!("Use the button below to choose a new password. The link works once, for {} minutes.", RESET_MINUTES),
                "If you did not ask for this, ignore this email; your password stays the same.".to_string(),
            ],
            "Choose a new password",
        )
    };
    send_mail(&settings, &secret, &to, subject, &lines, Some((label, &url))).await
}

/// Counts requests per key in a sliding hour (forgot-password requests are not failures,
/// so they need their own limit).
#[derive(Default)]
pub struct RequestCounter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RequestCounter {
    /// Record a request; false when `key` already made `max` in the last hour.
    pub fn allow(&self, key: &str, max: usize) -> bool {
        let mut map = self.hits.lock();
        let v = map.entry(key.to_lowercase()).or_default();
        v.retain(|t| t.elapsed() < Duration::from_secs(3600));
        if v.len() >= max {
            return false;
        }
        v.push(Instant::now());
        true
    }
}

static FORGOT_COUNTER: std::sync::OnceLock<RequestCounter> = std::sync::OnceLock::new();

#[derive(Deserialize)]
pub struct ForgotPayload {
    /// Username or email address.
    pub login: String,
}

/// Public: email a reset link. Always gives the same answer, so it never reveals whether
/// an account exists; the email is sent in the background.
pub async fn api_forgot_password(State(state): State<AppState>, ip: ClientIp, Json(p): Json<ForgotPayload>) -> ApiResult<Json<Value>> {
    let answer = Json(json!({
        "message": "If an account with an email address matches, we have sent it a link to choose a new password."
    }));
    if !EmailSettings::load(&state.db).ready() {
        return Err(err(StatusCode::NOT_FOUND, "Password reset by email is not set up"));
    }
    let login = p.login.trim().to_string();
    let counter = FORGOT_COUNTER.get_or_init(RequestCounter::default);
    if login.is_empty() || !counter.allow(&ip.limiter_key(), 10) || !counter.allow(&format!("acct:{}", login), 3) {
        return Ok(answer);
    }
    let user = match state.db.get_user_by_username(&login).ok().flatten() {
        Some(u) => Some(u),
        None if login.contains('@') => state
            .db
            .list_users()
            .unwrap_or_default()
            .into_iter()
            .find(|u| u.email.as_deref().map(|e| e.trim().eq_ignore_ascii_case(&login)).unwrap_or(false))
            .and_then(|u| state.db.get_user_by_id(&u.id).ok().flatten()),
        None => None,
    };
    if let Some(user) = user.filter(|u| u.is_active && u.auth_provider == "local" && u.email.is_some()) {
        state.db.log_audit(Some(&user.id), &user.username, "PASSWORD_RESET_REQUESTED", None, None, Some("Reset link sent by email"), ip.as_opt());
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = send_reset_link(&st, &user, false).await {
                tracing::warn!("Could not send the password reset email: {}", e);
            }
        });
    }
    Ok(answer)
}

#[derive(Deserialize)]
pub struct TokenQuery {
    pub token: String,
}

/// Public: whether a reset link is still valid (so the page can say so before asking).
pub async fn api_reset_check(State(state): State<AppState>, Query(q): Query<TokenQuery>) -> Json<Value> {
    match state.db.find_password_reset(&token_hash(&q.token)).ok().flatten() {
        Some((user_id, purpose)) => {
            let user = state.db.get_user_by_id(&user_id).ok().flatten();
            Json(json!({ "valid": user.is_some(), "username": user.map(|u| u.username), "invite": purpose == "invite" }))
        }
        None => Json(json!({ "valid": false })),
    }
}

#[derive(Deserialize)]
pub struct ResetPayload {
    pub token: String,
    pub password: String,
}

/// Public: choose a new password with a reset link. Ends all existing logins.
/// Two-factor authentication stays on.
pub async fn api_reset_password(State(state): State<AppState>, ip: ClientIp, Json(p): Json<ResetPayload>) -> ApiResult<Json<Value>> {
    if let Some(wait) = state.limiter.blocked_for(&ip.limiter_key()) {
        return Err(security::too_many_attempts(wait));
    }
    let Some((user_id, purpose)) = state.db.find_password_reset(&token_hash(&p.token)).map_err(db_err)? else {
        state.limiter.record_failure(&ip.limiter_key());
        return Err(err(StatusCode::BAD_REQUEST, "This link is no longer valid. Ask for a new one."));
    };
    let user = state.db.get_user_by_id(&user_id).map_err(db_err)?.ok_or_else(|| err(StatusCode::BAD_REQUEST, "This link is no longer valid."))?;
    if !user.is_active || user.auth_provider != "local" {
        return Err(err(StatusCode::BAD_REQUEST, "This account cannot reset its password here."));
    }
    let min_len = state.config.read().auth.min_password_length;
    check_password_policy(&p.password, &user.username, None, min_len).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    let hash = hash_password(&p.password).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    state.db.set_user_password(&user.id, &hash, false).map_err(db_err)?;
    state.db.delete_password_resets(&user.id).map_err(db_err)?;
    state.limiter.record_success(&user.username);
    let invite = purpose == "invite";
    state.db.log_audit(
        Some(&user.id),
        &user.username,
        if invite { "PASSWORD_SET_BY_LINK" } else { "PASSWORD_RESET_BY_EMAIL" },
        None,
        None,
        None,
        ip.as_opt(),
    );
    if !invite {
        notify_user(&state, &user, "Your Remote password was reset", "The password was reset with an email link");
    }
    Ok(Json(json!({ "message": "Password saved. You can sign in now.", "username": user.username })))
}

// ---------------- Admin ----------------

/// Admin: email a user a link to choose their own password (new account or forgotten password).
pub async fn api_send_password_link(State(state): State<AppState>, headers: HeaderMap, AxumPath(id): AxumPath<String>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let user = state.db.get_user_by_id(&id).map_err(db_err)?.ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))?;
    if user.auth_provider != "local" {
        return Err(err(StatusCode::BAD_REQUEST, "This account's password is managed outside Remote"));
    }
    send_reset_link(&state, &user, true).await.map_err(|e| err(StatusCode::BAD_GATEWAY, &format!("Could not send the email: {}", e)))?;
    state.db.log_audit(Some(&claims.sub), &claims.username, "PASSWORD_LINK_SENT", None, None, Some(&format!("Set-your-password link emailed to {}", user.username)), None);
    Ok(Json(json!({ "message": format!("Link sent to {}", user.email.unwrap_or_default()) })))
}

/// The settings as the admin page sees them (the password only as "is set").
fn public_view(s: &EmailSettings) -> Value {
    json!({
        "enabled": s.enabled,
        "host": s.host,
        "port": s.port,
        "security": if s.security.is_empty() { "starttls" } else { s.security.as_str() },
        "username": s.username,
        "has_password": s.password_enc.is_some(),
        "from_name": s.from_name,
        "from_address": s.from_address,
        "public_url": s.public_url,
        "ready": s.ready(),
    })
}

pub async fn api_get_email_settings(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    Ok(Json(public_view(&EmailSettings::load(&state.db))))
}

#[derive(Deserialize)]
pub struct EmailSettingsPayload {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: String,
    /// New password; omitted or null keeps the saved one, "" removes it.
    pub password: Option<String>,
    pub from_name: String,
    pub from_address: String,
    pub public_url: String,
}

pub async fn api_set_email_settings(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<EmailSettingsPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let bad = |m: &str| err(StatusCode::BAD_REQUEST, m);
    if !["starttls", "tls", "none"].contains(&p.security.as_str()) {
        return Err(bad("Encryption must be STARTTLS, TLS or none"));
    }
    let public_url = p.public_url.trim().trim_end_matches('/').to_string();
    if !public_url.is_empty() && !(public_url.starts_with("https://") || public_url.starts_with("http://")) {
        return Err(bad("The public address must start with https:// or http://"));
    }
    let from_address = p.from_address.trim().to_string();
    if !from_address.is_empty() && from_address.parse::<lettre::Address>().is_err() {
        return Err(bad("The sender address is not a valid email address"));
    }
    if p.enabled && (p.host.trim().is_empty() || from_address.is_empty() || public_url.is_empty()) {
        return Err(bad("To switch email on, fill in the mail server, the sender address and Remote's public address"));
    }
    let old = EmailSettings::load(&state.db);
    let secret = state.config.read().server.jwt_secret.clone();
    let password_enc = match p.password {
        None => old.password_enc,
        Some(pw) if pw.is_empty() => None,
        Some(pw) => Some(encrypt_secret(&pw, &secret).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?),
    };
    let s = EmailSettings {
        enabled: p.enabled,
        host: p.host.trim().to_string(),
        port: p.port,
        security: p.security,
        username: p.username.trim().to_string(),
        password_enc,
        from_name: p.from_name.trim().to_string(),
        from_address,
        public_url,
    };
    state.db.set_setting(EMAIL_SETTINGS_KEY, &serde_json::to_string(&s).map_err(db_err)?).map_err(db_err)?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "EMAIL_SETTINGS_CHANGED",
        None,
        None,
        Some(&format!("email {}, server {}:{}", if s.enabled { "on" } else { "off" }, s.host, s.port)),
        None,
    );
    Ok(Json(public_view(&s)))
}

#[derive(Deserialize)]
pub struct TestPayload {
    pub to: Option<String>,
}

/// Admin: send a test email (to the admin's own address unless another is given).
pub async fn api_test_email(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<TestPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let me = state.db.get_user_by_id(&claims.sub).map_err(db_err)?;
    let to = p
        .to
        .filter(|t| !t.trim().is_empty())
        .or_else(|| me.and_then(|u| u.email))
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Enter an address, or add one to your own account first"))?;
    let s = EmailSettings::load(&state.db);
    if s.host.is_empty() || s.from_address.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Save the mail server and sender address first"));
    }
    let secret = state.config.read().server.jwt_secret.clone();
    let lines = vec![
        "This is a test email from Remote.".to_string(),
        "If you can read this, email is set up correctly.".to_string(),
    ];
    send_mail(&s, &secret, to.trim(), "Remote test email", &lines, None)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, &format!("Sending failed: {}", e)))?;
    Ok(Json(json!({ "message": format!("Test email sent to {}", to.trim()) })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_limits_per_hour() {
        let c = RequestCounter::default();
        assert!(c.allow("a", 2));
        assert!(c.allow("A", 2));
        assert!(!c.allow("a", 2));
        assert!(c.allow("b", 2));
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let a = new_token();
        assert_ne!(a, new_token());
        assert_eq!(token_hash(&a), token_hash(&format!(" {} ", a)));
        assert_eq!(token_hash(&a).len(), 64);
    }

    #[test]
    fn readiness_needs_all_parts() {
        let mut s = EmailSettings { enabled: true, host: "smtp.example.com".into(), from_address: "r@example.com".into(), ..Default::default() };
        assert!(!s.ready());
        s.public_url = "https://example.com/remote/".into();
        assert!(s.ready());
        assert_eq!(s.base_url(), "https://example.com/remote");
    }
}
