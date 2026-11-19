//! Two-factor authentication: time-based one-time codes (RFC 6238, the format
//! authenticator apps use), one-time recovery codes, and the setup QR code.

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use qrcodegen::{QrCode, QrCodeEcc};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// Seconds per code.
const STEP: u64 = 30;
/// Digits per code.
const DIGITS: u32 = 6;
/// How many recovery codes a user gets.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// A new random secret (160 bits), base32-encoded as authenticator apps expect.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 20];
    getrandom::getrandom(&mut bytes).expect("OS random source");
    BASE32_NOPAD.encode(&bytes)
}

/// The `otpauth://` link that the setup QR code contains.
pub fn otpauth_uri(secret: &str, username: &str, issuer: &str) -> String {
    let label = format!("{}:{}", issuer, username);
    format!(
        "otpauth://totp/{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        urlencoding::encode(&label),
        secret,
        urlencoding::encode(issuer),
        DIGITS,
        STEP
    )
}

fn code_at(key: &[u8], step: u64) -> u32 {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);
    bin % 10u32.pow(DIGITS)
}

fn current_step() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / STEP)
        .unwrap_or(0)
}

/// Check a 6-digit code against the secret, allowing one step of clock drift either way.
/// Returns the time step it matched, so the caller can refuse the same code twice.
/// `last_used_step` is the step of the last accepted code; that step and older are refused.
pub fn verify_code(secret: &str, code: &str, last_used_step: i64) -> Option<u64> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != DIGITS as usize || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let wanted: u32 = code.parse().ok()?;
    let key = BASE32_NOPAD.decode(secret.as_bytes()).ok()?;
    let now = current_step();
    let mut matched = None;
    for step in [now.saturating_sub(1), now, now + 1] {
        // Compare every candidate so timing does not reveal which one matched.
        if code_at(&key, step) == wanted && (step as i64) > last_used_step {
            matched = Some(step);
        }
    }
    matched
}

/// Make the recovery codes: returns (codes to show the user once, hashes to store).
pub fn generate_recovery_codes() -> (Vec<String>, Vec<String>) {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    while codes.len() < RECOVERY_CODE_COUNT {
        let mut raw = String::with_capacity(10);
        let mut buf = [0u8; 1];
        while raw.len() < 10 {
            getrandom::getrandom(&mut buf).expect("OS random source");
            let limit = 256 - (256 % ALPHABET.len());
            if (buf[0] as usize) < limit {
                raw.push(ALPHABET[buf[0] as usize % ALPHABET.len()] as char);
            }
        }
        let code = format!("{}-{}", &raw[..5], &raw[5..]);
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    let hashes = codes.iter().map(|c| hash_recovery_code(c)).collect();
    (codes, hashes)
}

/// Hash of a recovery code as stored (case, spaces and dashes ignored).
pub fn hash_recovery_code(code: &str) -> String {
    let normal: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    hex::encode(Sha256::digest(normal.as_bytes()))
}

/// Whether the input looks like a recovery code rather than a 6-digit code.
pub fn looks_like_recovery_code(input: &str) -> bool {
    input.chars().filter(|c| c.is_ascii_alphanumeric()).count() == 10
}

/// The setup QR code as a small SVG (black modules on white, with a quiet zone).
pub fn qr_svg(text: &str) -> Option<String> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let border = 4;
    let size = qr.size();
    let dim = size + border * 2;
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + border, y + border));
            }
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {dim} {dim}\" shape-rendering=\"crispEdges\">\
         <rect width=\"{dim}\" height=\"{dim}\" fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#000000\"/></svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_reference_codes() {
        // RFC 6238 test secret "12345678901234567890" (SHA-1), 8-digit vectors reduced to 6 digits.
        let key = b"12345678901234567890";
        assert_eq!(code_at(key, 59 / STEP), 287082);
        assert_eq!(code_at(key, 1111111109 / STEP), 81804);
        assert_eq!(code_at(key, 1234567890 / STEP), 5924);
    }

    #[test]
    fn current_code_verifies_once() {
        let secret = generate_secret();
        let key = BASE32_NOPAD.decode(secret.as_bytes()).unwrap();
        let code = format!("{:06}", code_at(&key, current_step()));
        let step = verify_code(&secret, &code, 0).expect("valid code");
        assert!(verify_code(&secret, &code, step as i64).is_none(), "replay refused");
        assert!(verify_code(&secret, "12345", 0).is_none());
        assert!(verify_code(&secret, "abcdef", 0).is_none());
    }

    #[test]
    fn recovery_codes_hash_loosely() {
        let (codes, hashes) = generate_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        assert_eq!(hash_recovery_code(&codes[0].to_uppercase().replace('-', " ")), hashes[0]);
        assert!(looks_like_recovery_code(&codes[0]));
        assert!(!looks_like_recovery_code("123456"));
    }

    #[test]
    fn qr_renders() {
        let svg = qr_svg(&otpauth_uri("JBSWY3DPEHPK3PXP", "admin", "Remote")).unwrap();
        assert!(svg.starts_with("<svg"));
    }
}
