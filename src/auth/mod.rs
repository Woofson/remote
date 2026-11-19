pub mod policy;
pub mod totp;

use crate::config::OidcConfig;
use crate::db::User;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String, // User ID
    pub username: String,
    pub role: String,
    pub auth_provider: String,
    pub exp: usize,
    pub iat: usize,
    /// The user's session generation when issued; a newer generation ends this login.
    #[serde(default)]
    pub gen: i64,
    /// Random ID of this login, used for idle timeout and the lock screen.
    #[serde(default)]
    pub sid: String,
}

/// Short-lived proof that the password was right and a 2FA code is still needed.
/// Signed with a different key than login tokens, so it can never be used as one.
#[derive(Debug, Serialize, Deserialize)]
struct TwoFactorChallenge {
    sub: String,
    purpose: String,
    gen: i64,
    exp: usize,
}

const CHALLENGE_MINUTES: i64 = 5;

fn challenge_key(secret: &str) -> Vec<u8> {
    format!("{}|two-factor-challenge", secret).into_bytes()
}

pub fn create_2fa_challenge(user_id: &str, gen: i64, secret: &str) -> Result<String, String> {
    let claims = TwoFactorChallenge {
        sub: user_id.to_string(),
        purpose: "2fa".into(),
        gen,
        exp: (Utc::now() + Duration::minutes(CHALLENGE_MINUTES)).timestamp() as usize,
    };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(&challenge_key(secret)))
        .map_err(|e| format!("JWT encoding error: {}", e))
}

/// Returns (user id, session generation) of a valid, unexpired challenge.
pub fn verify_2fa_challenge(token: &str, secret: &str) -> Result<(String, i64), String> {
    let data = decode::<TwoFactorChallenge>(
        token,
        &DecodingKey::from_secret(&challenge_key(secret)),
        &Validation::default(),
    )
    .map_err(|_| "The sign-in step expired, please enter your password again".to_string())?;
    if data.claims.purpose != "2fa" {
        return Err("Invalid sign-in step".into());
    }
    Ok((data.claims.sub, data.claims.gen))
}

fn new_session_id() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("OS random source");
    hex::encode(b)
}

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| format!("Password hashing failed: {}", e))
}

/// A random 20-character password (same alphabet as Brum: no look-alike characters).
pub fn generate_password() -> String {
    const CHARSET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
    let mut out = String::with_capacity(20);
    let mut buf = [0u8; 1];
    while out.len() < 20 {
        getrandom::getrandom(&mut buf).expect("OS random source");
        // Rejection sampling keeps every character equally likely.
        let limit = 256 - (256 % CHARSET.len());
        if (buf[0] as usize) < limit {
            out.push(CHARSET[buf[0] as usize % CHARSET.len()] as char);
        }
    }
    out
}

/// Password rules for local accounts.
pub fn check_password_policy(
    password: &str,
    username: &str,
    previous: Option<&str>,
    min_length: usize,
) -> Result<(), String> {
    if password.chars().count() < min_length {
        return Err(format!("Password must be at least {} characters", min_length));
    }
    if password.eq_ignore_ascii_case(username) {
        return Err("Password must not be the username".into());
    }
    if previous == Some(password) {
        return Err("New password must differ from the current one".into());
    }
    Ok(())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

pub fn create_jwt(user: &User, secret: &str, expiry_hours: i64, gen: i64) -> Result<String, String> {
    let now = Utc::now();
    let exp = (now + Duration::hours(expiry_hours)).timestamp() as usize;
    let iat = now.timestamp() as usize;

    let claims = Claims {
        sub: user.id.clone(),
        username: user.username.clone(),
        role: user.role.clone(),
        auth_provider: user.auth_provider.clone(),
        exp,
        iat,
        gen,
        sid: new_session_id(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| format!("JWT encoding error: {}", e))
}

pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, String> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| format!("Invalid or expired JWT: {}", e))?;
    Ok(token_data.claims)
}

// ================= Credential Encryption (AES-GCM-256) =================

pub fn encrypt_secret(secret_text: &str, master_key: &str) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(master_key.as_bytes());
    let key_bytes = hasher.finalize();

    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, secret_text.as_bytes())
        .map_err(|e| format!("Encryption error: {}", e))?;

    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    Ok(hex::encode(combined))
}

pub fn decrypt_secret(hex_payload: &str, master_key: &str) -> Result<String, String> {
    let data = hex::decode(hex_payload).map_err(|e| format!("Hex decode error: {}", e))?;
    if data.len() < 12 {
        return Err("Ciphertext too short".into());
    }

    let mut hasher = Sha256::new();
    hasher.update(master_key.as_bytes());
    let key_bytes = hasher.finalize();

    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&data[..12]);
    let ciphertext = &data[12..];

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption error: {}", e))?;

    String::from_utf8(plaintext).map_err(|e| format!("UTF-8 decode error: {}", e))
}

// ================= OIDC / Authentik Service =================

#[derive(Debug, Clone, Deserialize)]
pub struct OidcDiscoveryDocument {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: Option<String>,
    pub jwks_uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OidcTokenResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_in: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OidcUserInfo {
    pub sub: String,
    pub preferred_username: Option<String>,
    pub nickname: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub groups: Option<Vec<String>>,
}

pub struct OidcService {
    client: Client,
    pub config: OidcConfig,
}

impl OidcService {
    pub fn new(config: OidcConfig) -> Self {
        Self {
            client: Client::builder().build().unwrap_or_default(),
            config,
        }
    }

    pub async fn fetch_discovery(&self) -> Result<OidcDiscoveryDocument, String> {
        let issuer = self.config.issuer_url.trim_end_matches('/');
        let well_known_url = format!("{}/.well-known/openid-configuration", issuer);
        
        let resp = self
            .client
            .get(&well_known_url)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to OIDC provider at {}: {}", well_known_url, e))?;

        if !resp.status().is_success() {
            return Err(format!("OIDC discovery returned HTTP {}", resp.status()));
        }

        let doc = resp
            .json::<OidcDiscoveryDocument>()
            .await
            .map_err(|e| format!("Failed to parse OIDC discovery document: {}", e))?;

        Ok(doc)
    }

    pub async fn build_authorization_url(&self, state: &str) -> Result<String, String> {
        let doc = self.fetch_discovery().await?;
        let scopes = self.config.scopes.join(" ");
        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
            doc.authorization_endpoint,
            urlencoding::encode(&self.config.client_id),
            urlencoding::encode(&self.config.redirect_uri),
            urlencoding::encode(&scopes),
            urlencoding::encode(state),
        );
        Ok(url)
    }

    pub async fn exchange_code_and_get_user(&self, code: &str) -> Result<OidcUserInfo, String> {
        let doc = self.fetch_discovery().await?;
        
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.config.redirect_uri),
            ("client_id", &self.config.client_id),
            ("client_secret", &self.config.client_secret),
        ];

        let token_resp = self
            .client
            .post(&doc.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Token exchange request failed: {}", e))?;

        if !token_resp.status().is_success() {
            let err_body = token_resp.text().await.unwrap_or_default();
            return Err(format!("OIDC token exchange failed: {}", err_body));
        }

        let token_data: OidcTokenResponse = token_resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse token response: {}", e))?;

        // If userinfo endpoint exists, fetch detailed profile
        if let Some(userinfo_url) = doc.userinfo_endpoint {
            let userinfo_resp = self
                .client
                .get(&userinfo_url)
                .bearer_auth(&token_data.access_token)
                .send()
                .await
                .map_err(|e| format!("Failed to fetch userinfo: {}", e))?;

            if userinfo_resp.status().is_success() {
                let user_info = userinfo_resp
                    .json::<OidcUserInfo>()
                    .await
                    .map_err(|e| format!("Failed to parse userinfo: {}", e))?;
                return Ok(user_info);
            }
        }

        // Fallback: decode claims from ID token
        if let Some(id_token) = token_data.id_token {
            let parts: Vec<&str> = id_token.split('.').collect();
            if parts.len() >= 2 {
                if let Ok(decoded) = base64::Engine::decode(
                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                    parts[1],
                ) {
                    if let Ok(info) = serde_json::from_slice::<OidcUserInfo>(&decoded) {
                        return Ok(info);
                    }
                }
            }
        }

        Err("Could not retrieve user info from OIDC provider".into())
    }

    pub fn determine_role_for_groups(&self, groups: &[String]) -> String {
        if !self.config.admin_group.is_empty() && groups.iter().any(|g| g == &self.config.admin_group) {
            return "admin".to_string();
        }
        if !self.config.operator_group.is_empty() && groups.iter().any(|g| g == &self.config.operator_group) {
            return "operator".to_string();
        }
        "viewer".to_string()
    }
}
