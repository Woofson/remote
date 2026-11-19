//! Two-factor authentication, lock screen, activity and the admin session policy (#26).

use super::*;
use crate::auth::totp;
use crate::auth::verify_2fa_challenge;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg })))
}

fn db_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

pub(super) fn too_many_attempts(wait_secs: u64) -> (StatusCode, Json<Value>) {
    let wait = if wait_secs >= 90 {
        format!("{} minutes", (wait_secs + 59) / 60)
    } else {
        format!("{} seconds", wait_secs)
    };
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({ "error": format!("Too many attempts. Try again in {}.", wait), "code": "rate_limited" })),
    )
}

fn load_user(state: &AppState, id: &str) -> ApiResult<User> {
    state
        .db
        .get_user_by_id(id)
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))
}

fn jwt_secret(state: &AppState) -> String {
    state.config.read().server.jwt_secret.clone()
}

/// Active local accounts without 2FA (for the admin status list).
pub(super) fn local_users_without_2fa(state: &AppState) -> Vec<String> {
    let with = state.db.two_factor_user_ids().unwrap_or_default();
    state
        .db
        .list_users()
        .unwrap_or_default()
        .into_iter()
        .filter(|u| u.is_active && u.auth_provider == "local" && !with.contains(&u.id))
        .map(|u| u.username)
        .collect()
}

/// Check a 6-digit code or a recovery code for a user with 2FA on. A matching code
/// is used up (no reuse). Returns a note for the audit log on success.
fn check_second_factor(state: &AppState, user: &User, input: &str) -> Result<String, String> {
    let tf = state.db.get_two_factor(&user.id).map_err(|e| e.to_string())?;
    let Some(enc) = tf.secret_enc.as_deref() else {
        return Err("Two-factor authentication is not turned on".into());
    };
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter the code from your authenticator app".into());
    }
    if totp::looks_like_recovery_code(input) {
        let hash = totp::hash_recovery_code(input);
        let mut left = tf.recovery_hashes.clone();
        let Some(pos) = left.iter().position(|h| *h == hash) else {
            return Err("That code is not valid".into());
        };
        left.remove(pos);
        state.db.set_recovery_codes(&user.id, &left).map_err(|e| e.to_string())?;
        return Ok(format!("recovery code used, {} left", left.len()));
    }
    let secret = decrypt_secret(enc, &jwt_secret(state)).map_err(|_| "Could not read the 2FA secret".to_string())?;
    match totp::verify_code(&secret, input, tf.last_step) {
        Some(step) => {
            state.db.set_totp_last_step(&user.id, step as i64).map_err(|e| e.to_string())?;
            Ok("authenticator code".into())
        }
        None => Err("That code is not valid".into()),
    }
}

/// Password (and, when asked, second factor) check with the attempt brake and audit log.
fn verify_identity(state: &AppState, user: &User, password: &str, code: Option<&str>, action: &str, ip: &ClientIp) -> ApiResult<()> {
    if let Some(wait) = state.limiter.blocked_for(&user.username) {
        return Err(too_many_attempts(wait));
    }
    if user.auth_provider != "local" || !verify_password(password, &user.password_hash) {
        state.limiter.record_failure(&user.username);
        state.db.log_audit(Some(&user.id), &user.username, &format!("{}_FAILED", action), None, None, Some("Wrong password"), ip.as_opt());
        return Err(err(StatusCode::UNAUTHORIZED, "Wrong password"));
    }
    if let Some(code) = code {
        if let Err(e) = check_second_factor(state, user, code) {
            state.limiter.record_failure(&user.username);
            state.db.log_audit(Some(&user.id), &user.username, &format!("{}_FAILED", action), None, None, Some("Wrong 2FA code"), ip.as_opt());
            return Err(err(StatusCode::UNAUTHORIZED, &e));
        }
    }
    state.limiter.record_success(&user.username);
    Ok(())
}

// ---------------- Sign-in, second step ----------------

#[derive(Deserialize)]
pub struct Login2faPayload {
    pub challenge: String,
    pub code: String,
}

pub async fn api_login_2fa(State(state): State<AppState>, ip: ClientIp, Json(p): Json<Login2faPayload>) -> ApiResult<Json<Value>> {
    let secret = jwt_secret(&state);
    let (user_id, gen) = verify_2fa_challenge(&p.challenge, &secret).map_err(|e| err(StatusCode::UNAUTHORIZED, &e))?;
    let user = load_user(&state, &user_id)?;
    let (generation, must_change) = state.db.get_user_security(&user.id).map_err(db_err)?;
    if !user.is_active || gen != generation {
        return Err(err(StatusCode::UNAUTHORIZED, "The sign-in step expired, please enter your password again"));
    }
    if let Some(wait) = state.limiter.blocked_for(&user.username) {
        return Err(too_many_attempts(wait));
    }
    let note = match check_second_factor(&state, &user, &p.code) {
        Ok(n) => n,
        Err(e) => {
            state.limiter.record_failure(&user.username);
            state.db.log_audit(Some(&user.id), &user.username, "TWO_FACTOR_FAILED", None, None, Some("Wrong code at sign-in"), ip.as_opt());
            return Err(err(StatusCode::UNAUTHORIZED, &e));
        }
    };
    state.limiter.record_success(&user.username);
    let token = create_jwt(&user, &secret, login_hours(&state), generation)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let _ = state.db.update_last_login(&user.id);
    state.db.log_audit(
        Some(&user.id),
        &user.username,
        "LOGIN_SUCCESS",
        None,
        None,
        Some(&format!("User logged in via local auth with 2FA ({})", note)),
        ip.as_opt(),
    );
    let safe_user: UserSafe = user.into();
    Ok(Json(json!({
        "token": token,
        "user": safe_user,
        "must_change_password": must_change,
        "two_factor_setup_required": false,
    })))
}

// ---------------- Lock screen & activity ----------------

pub async fn api_lock(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims_pending(&state, &headers)?;
    state.sessions.lock(&claims.sid);
    Ok(Json(json!({ "locked": true })))
}

#[derive(Deserialize)]
pub struct UnlockPayload {
    pub password: String,
    pub code: Option<String>,
}

pub async fn api_unlock(State(state): State<AppState>, headers: HeaderMap, ip: ClientIp, Json(p): Json<UnlockPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims_pending(&state, &headers)?;
    let user = load_user(&state, &claims.sub)?;
    if user.auth_provider != "local" {
        return Err(err(StatusCode::BAD_REQUEST, "This account signs in through its provider; use Switch User to sign in again"));
    }
    let needs_code = state.policy.read().lock_requires_2fa
        && state.db.get_two_factor(&user.id).map(|t| t.enabled()).unwrap_or(false);
    if needs_code && p.code.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Enter the code from your authenticator app", "code": "two_factor_required" })),
        ));
    }
    verify_identity(&state, &user, &p.password, if needs_code { p.code.as_deref() } else { None }, "UNLOCK", &ip)?;
    state.sessions.unlock(&claims.sid);
    Ok(Json(json!({ "unlocked": true })))
}

/// The browser reports that the user did something (typing, clicking): resets the idle timer.
pub async fn api_activity(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims_pending(&state, &headers)?;
    state.sessions.touch(&claims.sid);
    Ok(Json(json!({ "ok": true })))
}

// ---------------- Own 2FA ----------------

pub async fn api_2fa_status(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims_setup(&state, &headers)?;
    let user = load_user(&state, &claims.sub)?;
    let tf = state.db.get_two_factor(&user.id).map_err(db_err)?;
    let policy = state.policy.read().clone();
    Ok(Json(json!({
        "available": user.auth_provider == "local",
        "enabled": tf.enabled(),
        "recovery_codes_left": tf.recovery_hashes.len(),
        "required": policy.requires_2fa(&user.role),
    })))
}

pub async fn api_2fa_setup(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims_setup(&state, &headers)?;
    let user = load_user(&state, &claims.sub)?;
    if user.auth_provider != "local" {
        return Err(err(StatusCode::BAD_REQUEST, "Your sign-in provider handles two-factor authentication"));
    }
    if state.db.get_two_factor(&user.id).map_err(db_err)?.enabled() {
        return Err(err(StatusCode::CONFLICT, "Two-factor authentication is already on"));
    }
    let secret = totp::generate_secret();
    let enc = encrypt_secret(&secret, &jwt_secret(&state)).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    state.db.set_totp_pending(&user.id, &enc).map_err(db_err)?;
    let uri = totp::otpauth_uri(&secret, &user.username, "Remote");
    Ok(Json(json!({
        "secret": secret,
        "otpauth_uri": uri,
        "qr_svg": totp::qr_svg(&uri),
    })))
}

#[derive(Deserialize)]
pub struct CodePayload {
    pub code: String,
}

pub async fn api_2fa_enable(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<CodePayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims_setup(&state, &headers)?;
    let user = load_user(&state, &claims.sub)?;
    if let Some(wait) = state.limiter.blocked_for(&user.username) {
        return Err(too_many_attempts(wait));
    }
    let tf = state.db.get_two_factor(&user.id).map_err(db_err)?;
    if tf.enabled() {
        return Err(err(StatusCode::CONFLICT, "Two-factor authentication is already on"));
    }
    let Some(pending) = tf.pending_enc else {
        return Err(err(StatusCode::BAD_REQUEST, "Start the setup again"));
    };
    let secret = decrypt_secret(&pending, &jwt_secret(&state)).map_err(|_| err(StatusCode::BAD_REQUEST, "Start the setup again"))?;
    let Some(step) = totp::verify_code(&secret, &p.code, 0) else {
        state.limiter.record_failure(&user.username);
        return Err(err(StatusCode::UNAUTHORIZED, "That code is not valid. Check the time on your phone and try the newest code."));
    };
    state.limiter.record_success(&user.username);
    let (codes, hashes) = totp::generate_recovery_codes();
    state.db.enable_totp(&user.id, &pending, &hashes, step as i64).map_err(db_err)?;
    state.db.log_audit(Some(&user.id), &user.username, "TWO_FACTOR_ENABLED", None, None, None, None);
    Ok(Json(json!({ "enabled": true, "recovery_codes": codes })))
}

#[derive(Deserialize)]
pub struct ConfirmPayload {
    pub password: String,
    pub code: String,
}

pub async fn api_2fa_disable(State(state): State<AppState>, headers: HeaderMap, ip: ClientIp, Json(p): Json<ConfirmPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    let user = load_user(&state, &claims.sub)?;
    if state.policy.read().requires_2fa(&user.role) {
        return Err(err(StatusCode::FORBIDDEN, "Two-factor authentication is required for your role and cannot be turned off"));
    }
    verify_identity(&state, &user, &p.password, Some(&p.code), "TWO_FACTOR_DISABLE", &ip)?;
    state.db.disable_totp(&user.id).map_err(db_err)?;
    state.db.log_audit(Some(&user.id), &user.username, "TWO_FACTOR_DISABLED", None, None, None, None);
    super::email::notify_user(&state, &user, "Two-factor authentication was turned off", "Two-factor authentication was turned off");
    Ok(Json(json!({ "enabled": false })))
}

pub async fn api_2fa_recovery_codes(State(state): State<AppState>, headers: HeaderMap, ip: ClientIp, Json(p): Json<ConfirmPayload>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    let user = load_user(&state, &claims.sub)?;
    verify_identity(&state, &user, &p.password, Some(&p.code), "RECOVERY_CODES", &ip)?;
    let (codes, hashes) = totp::generate_recovery_codes();
    state.db.set_recovery_codes(&user.id, &hashes).map_err(db_err)?;
    state.db.log_audit(Some(&user.id), &user.username, "RECOVERY_CODES_RENEWED", None, None, None, None);
    super::email::notify_user(&state, &user, "New two-factor recovery codes", "New two-factor recovery codes were made (the old ones no longer work)");
    Ok(Json(json!({ "recovery_codes": codes })))
}

// ---------------- Admin ----------------

/// Admin: turn off a user's 2FA (lost phone). Also ends that user's logins.
pub async fn api_reset_user_2fa(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let target = load_user(&state, &id)?;
    state.db.disable_totp(&target.id).map_err(db_err)?;
    state.db.bump_session_generation(&target.id).map_err(db_err)?;
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "TWO_FACTOR_RESET",
        None,
        None,
        Some(&format!("2FA turned off for {}", target.username)),
        None,
    );
    super::email::notify_user(&state, &target, "Two-factor authentication was reset", "An administrator turned off two-factor authentication");
    Ok(Json(json!({ "message": format!("Two-factor authentication turned off for {}", target.username) })))
}

pub async fn api_get_security_policy(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let policy = state.policy.read().clone();
    Ok(Json(json!(policy)))
}

pub async fn api_set_security_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(policy): Json<SecurityPolicy>,
) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    policy.validate().map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    let text = serde_json::to_string(&policy).map_err(db_err)?;
    state.db.set_setting(SECURITY_POLICY_KEY, &text).map_err(db_err)?;
    *state.policy.write() = policy.clone();
    let roles = if policy.require_2fa_roles.is_empty() { "none".to_string() } else { policy.require_2fa_roles.join(", ") };
    state.db.log_audit(
        Some(&claims.sub),
        &claims.username,
        "SECURITY_POLICY_CHANGED",
        None,
        None,
        Some(&format!(
            "login {} h, idle {} min ({}), lock-screen 2FA {}, 2FA required for: {}",
            policy.max_login_hours,
            policy.idle_minutes,
            policy.idle_action,
            if policy.lock_requires_2fa { "on" } else { "off" },
            roles
        )),
        None,
    );
    Ok(Json(json!(policy)))
}
