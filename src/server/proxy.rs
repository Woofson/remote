//! Running behind a reverse proxy or tunnel (#3): sub-path hosting, the real client IP
//! from trusted proxies, health checks, and sign-in from proxy headers (forward auth).

use super::*;
use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::middleware::Next;
use std::net::{IpAddr, SocketAddr};

// ---------------- Trusted proxies & client IP ----------------

/// Whether `ip` matches an entry like "10.0.0.1", "10.0.0.0/8" or "fd00::/8".
fn ip_matches(ip: IpAddr, entry: &str) -> bool {
    let entry = entry.trim();
    let (addr, bits) = match entry.split_once('/') {
        Some((a, b)) => (a, b.parse::<u32>().ok()),
        None => (entry, None),
    };
    let Ok(net) = addr.parse::<IpAddr>() else { return false };
    let ip = match (ip, net) {
        // Treat IPv4-mapped IPv6 addresses (::ffff:1.2.3.4) as IPv4.
        (IpAddr::V6(v6), IpAddr::V4(_)) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    };
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            let bits = bits.unwrap_or(32).min(32);
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            (u32::from(a) & mask) == (u32::from(n) & mask)
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => {
            let bits = bits.unwrap_or(128).min(128);
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            (u128::from(a) & mask) == (u128::from(n) & mask)
        }
        _ => false,
    }
}

pub fn is_trusted(ip: IpAddr, trusted: &[String]) -> bool {
    trusted.iter().any(|t| ip_matches(ip, t))
}

/// The visitor's address. Forwarded headers are only believed when the direct peer is a
/// trusted proxy; then the rightmost address in X-Forwarded-For that is not itself a
/// trusted proxy is used, else X-Real-IP / CF-Connecting-IP.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[String]) -> IpAddr {
    if !is_trusted(peer, trusted) {
        return peer;
    }
    let header_ip = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
    };
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let hops: Vec<IpAddr> = xff.split(',').filter_map(|h| h.trim().parse().ok()).collect();
        if let Some(ip) = hops.iter().rev().find(|ip| !is_trusted(**ip, trusted)) {
            return *ip;
        }
        if let Some(first) = hops.first() {
            return *first;
        }
    }
    header_ip("x-real-ip").or_else(|| header_ip("cf-connecting-ip")).unwrap_or(peer)
}

/// Extractor: the visitor's IP address as text (for the audit log and the attempt brake).
pub struct ClientIp(pub String);

#[axum::async_trait]
impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let Some(ConnectInfo(peer)) = parts.extensions.get::<ConnectInfo<SocketAddr>>().cloned() else {
            return Ok(ClientIp(String::new()));
        };
        let trusted = state.config.read().server.trusted_proxies.clone();
        Ok(ClientIp(client_ip(peer.ip(), &parts.headers, &trusted).to_string()))
    }
}

impl ClientIp {
    pub fn as_opt(&self) -> Option<&str> {
        (!self.0.is_empty()).then_some(self.0.as_str())
    }
    /// Key for the attempt brake.
    pub fn limiter_key(&self) -> String {
        format!("ip:{}", self.0)
    }
}

// ---------------- Sub-path ----------------

/// Lets Remote answer under `base_path` (e.g. "/remote/...") whether or not the proxy
/// strips that prefix: the prefix is removed before routing, and "/remote" redirects
/// to "/remote/" so the page's relative links resolve.
pub async fn strip_base_path(State(base): State<String>, mut req: Request, next: Next) -> Response {
    if base.is_empty() {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    if path == base {
        let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
        return Redirect::permanent(&format!("{}/{}", base, query)).into_response();
    }
    if let Some(rest) = path.strip_prefix(&format!("{}/", base)) {
        let new_path = format!("/{}", rest);
        let pq = match req.uri().query() {
            Some(q) => format!("{}?{}", new_path, q),
            None => new_path,
        };
        let mut parts = req.uri().clone().into_parts();
        if let Ok(pq) = pq.parse() {
            parts.path_and_query = Some(pq);
            if let Ok(uri) = axum::http::Uri::from_parts(parts) {
                *req.uri_mut() = uri;
            }
        }
    }
    next.run(req).await
}

// ---------------- Health ----------------

/// Public liveness check: the process is up.
pub async fn api_health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Public readiness check: the database answers. 503 otherwise.
pub async fn api_ready(State(state): State<AppState>) -> Response {
    if state.db.ping() {
        (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "unavailable" }))).into_response()
    }
}

// ---------------- Sign-in from proxy headers ----------------

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return None;
    }
    headers
        .get(name.trim())
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Sign in the user named by the trusted proxy's header (Authelia `Remote-User`,
/// Authentik `X-authentik-username`, ...). Accounts made this way have the provider
/// "header"; an existing local or single sign-on account with the same name is never taken over.
pub async fn api_header_login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = state.config.read().clone();
    let hl = &cfg.auth.header_login;
    let refuse = |code: StatusCode, msg: &str| (code, Json(json!({ "error": msg })));
    if !hl.enabled {
        return Err(refuse(StatusCode::NOT_FOUND, "Sign-in through the proxy is not enabled"));
    }
    // Headers from anyone but a trusted proxy could be forged by the visitor.
    if !is_trusted(peer.ip(), &cfg.server.trusted_proxies) {
        tracing::warn!("Header sign-in refused: {} is not a trusted proxy", peer.ip());
        return Err(refuse(StatusCode::FORBIDDEN, "Sign-in through the proxy is only accepted from a trusted proxy"));
    }
    let Some(username) = header_text(&headers, &hl.username_header) else {
        return Err(refuse(StatusCode::UNAUTHORIZED, "The proxy did not send a username"));
    };
    let ip = client_ip(peer.ip(), &headers, &cfg.server.trusted_proxies).to_string();
    let email = header_text(&headers, &hl.email_header);
    let name = header_text(&headers, &hl.name_header);
    let groups: Vec<String> = header_text(&headers, &hl.groups_header)
        .map(|g| g.split([',', '|']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let role = if !hl.admin_group.is_empty() && groups.iter().any(|g| g == &hl.admin_group) {
        "admin".to_string()
    } else if !hl.operator_group.is_empty() && groups.iter().any(|g| g == &hl.operator_group) {
        "operator".to_string()
    } else {
        cfg.auth.default_role.clone()
    };

    let existing = state.db.get_user_by_username(&username).map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let user = match existing {
        Some(mut u) if u.auth_provider == "header" => {
            if !u.is_active {
                return Err(refuse(StatusCode::FORBIDDEN, "Account is disabled"));
            }
            let role_changed = u.role != role;
            u.role = role;
            if email.is_some() {
                u.email = email;
            }
            if name.is_some() {
                u.display_name = name;
            }
            let _ = state.db.update_user(&u);
            if role_changed {
                let _ = state.db.bump_session_generation(&u.id);
            }
            u
        }
        Some(_) => {
            state.db.log_audit(None, &username, "LOGIN_FAILED", None, None, Some("Proxy sign-in: name belongs to another account type"), Some(&ip));
            return Err(refuse(
                StatusCode::CONFLICT,
                "A Remote account with this name already exists and signs in another way. Ask an admin.",
            ));
        }
        None => {
            let new_user = User {
                id: Uuid::new_v4().to_string(),
                username: username.clone(),
                password_hash: hash_password(&Uuid::new_v4().to_string()).unwrap_or_default(),
                email,
                display_name: name,
                role,
                is_active: true,
                auth_provider: "header".to_string(),
                oidc_sub: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                last_login: None,
                avatar_data: None,
            };
            state.db.create_user(&new_user).map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            new_user
        }
    };

    let (generation, _) = state.db.get_user_security(&user.id).map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let token = create_jwt(&user, &cfg.server.jwt_secret, login_hours(&state), generation)
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let _ = state.db.update_last_login(&user.id);
    state.db.log_audit(Some(&user.id), &user.username, "LOGIN_SUCCESS", None, None, Some("Signed in through the reverse proxy"), Some(&ip));
    let safe_user: UserSafe = user.into();
    Ok(Json(json!({
        "token": token,
        "user": safe_user,
        "must_change_password": false,
        "two_factor_setup_required": false,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted() -> Vec<String> {
        vec!["127.0.0.1".into(), "10.0.0.0/8".into(), "fd00::/8".into()]
    }

    #[test]
    fn cidr_matching() {
        assert!(ip_matches("10.1.2.3".parse().unwrap(), "10.0.0.0/8"));
        assert!(!ip_matches("11.1.2.3".parse().unwrap(), "10.0.0.0/8"));
        assert!(ip_matches("127.0.0.1".parse().unwrap(), "127.0.0.1"));
        assert!(ip_matches("::ffff:10.0.0.5".parse().unwrap(), "10.0.0.0/8"));
        assert!(ip_matches("fd12::1".parse().unwrap(), "fd00::/8"));
        assert!(!ip_matches("10.0.0.1".parse().unwrap(), "not-an-ip"));
    }

    #[test]
    fn forwarded_headers_only_from_trusted_peers() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "203.0.113.7, 10.0.0.2".parse().unwrap());
        // Untrusted peer: headers ignored.
        assert_eq!(client_ip("198.51.100.1".parse().unwrap(), &h, &trusted()).to_string(), "198.51.100.1");
        // Trusted peer: rightmost untrusted hop.
        assert_eq!(client_ip("127.0.0.1".parse().unwrap(), &h, &trusted()).to_string(), "203.0.113.7");
        // A forged leftmost entry does not win over the real client added by the proxy.
        h.insert("x-forwarded-for", "1.1.1.1, 203.0.113.7".parse().unwrap());
        assert_eq!(client_ip("127.0.0.1".parse().unwrap(), &h, &trusted()).to_string(), "203.0.113.7");
        let mut h2 = HeaderMap::new();
        h2.insert("x-real-ip", "203.0.113.9".parse().unwrap());
        assert_eq!(client_ip("10.0.0.3".parse().unwrap(), &h2, &trusted()).to_string(), "203.0.113.9");
    }
}
