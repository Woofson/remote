//! Login and lock screen branding (#30): logo, title, welcome text, wallpapers and
//! accent colour, set by admins and shown to everyone before sign-in.

use super::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg })))
}

fn db_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

/// Database key of the branding settings.
const BRANDING_KEY: &str = "branding";
/// Largest accepted image upload. The browser shrinks pictures first, so this is generous.
pub const MAX_BRANDING_IMAGE: usize = 6 * 1024 * 1024;
const MAX_WALLPAPERS: usize = 20;
const MAX_TITLE: usize = 60;
const MAX_SUBTITLE: usize = 160;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Branding {
    /// Replaces "Hello friend." on the login screen; empty keeps the default.
    pub title: String,
    /// Replaces "Remote is ready when you are."; empty keeps the default.
    pub subtitle: String,
    /// Stored image file name, or none for the Remote logo.
    pub logo: Option<String>,
    /// Stored image file names; one is picked at random per visit.
    pub wallpapers: Vec<String>,
    /// `#rrggbb`, or none for the Woofson amber.
    pub accent: Option<String>,
}

impl Branding {
    fn load(db: &Database) -> Self {
        db.get_setting(BRANDING_KEY)
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    }

    fn is_custom(&self) -> bool {
        *self != Branding::default()
    }

    fn files(&self) -> impl Iterator<Item = &String> {
        self.logo.iter().chain(self.wallpapers.iter())
    }

    fn public_view(&self) -> Value {
        let url = |f: &String| format!("api/branding/file/{f}");
        json!({
            "custom": self.is_custom(),
            "title": self.title,
            "subtitle": self.subtitle,
            "logo": self.logo,
            "logo_url": self.logo.as_ref().map(url),
            "wallpapers": self.wallpapers,
            "wallpaper_urls": self.wallpapers.iter().map(url).collect::<Vec<_>>(),
            "accent": self.accent,
        })
    }
}

fn branding_dir(state: &AppState) -> std::path::PathBuf {
    std::path::Path::new(&state.config.read().server.data_dir).join("branding")
}

/// Stored names are `<sha256>.<png|jpg|webp>`; nothing else is ever read or written.
fn valid_file_name(name: &str) -> bool {
    let Some((hash, ext)) = name.split_once('.') else { return false };
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) && matches!(ext, "png" | "jpg" | "webp")
}

/// Image type from the file's first bytes (not from its name or the browser's claim).
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn valid_accent(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Public: what the login and lock screens need. No secrets in here.
pub async fn api_get_branding(State(state): State<AppState>) -> Json<Value> {
    Json(Branding::load(&state.db).public_view())
}

/// Public: a stored branding image. Names are content hashes, so they can be cached forever.
pub async fn api_branding_file(State(state): State<AppState>, AxumPath(name): AxumPath<String>) -> Result<Response, StatusCode> {
    if !valid_file_name(&name) {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = std::fs::read(branding_dir(&state).join(&name)).map_err(|_| StatusCode::NOT_FOUND)?;
    let mime = match name.rsplit('.').next() {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        _ => "image/webp",
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(bytes))
        .unwrap())
}

/// Admin: store an uploaded image and return its file name (not yet in use until saved).
pub async fn api_upload_branding_image(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "No file"))?;
    let bytes = field.bytes().await.map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;
    if bytes.len() > MAX_BRANDING_IMAGE {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "The image is too large (6 MB at most)"));
    }
    let ext = sniff_image(&bytes).ok_or_else(|| err(StatusCode::BAD_REQUEST, "Only PNG, JPEG or WebP images are accepted"))?;
    let name = format!("{}.{ext}", hex::encode(Sha256::digest(&bytes)));
    let dir = branding_dir(&state);
    std::fs::create_dir_all(&dir).map_err(db_err)?;
    std::fs::write(dir.join(&name), &bytes).map_err(db_err)?;
    Ok(Json(json!({ "file": name, "url": format!("api/branding/file/{name}") })))
}

/// Admin: save the branding. Sending the defaults (empty texts, no images) resets everything.
pub async fn api_set_branding(State(state): State<AppState>, headers: HeaderMap, Json(p): Json<Branding>) -> ApiResult<Json<Value>> {
    let claims = extract_claims(&state, &headers)?;
    require_admin(&claims)?;
    let bad = |m: &str| err(StatusCode::BAD_REQUEST, m);

    let title = p.title.trim().to_string();
    let subtitle = p.subtitle.trim().to_string();
    if title.chars().count() > MAX_TITLE {
        return Err(bad("The title is too long (60 characters at most)"));
    }
    if subtitle.chars().count() > MAX_SUBTITLE {
        return Err(bad("The welcome text is too long (160 characters at most)"));
    }
    let accent = p.accent.map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty());
    if accent.as_deref().is_some_and(|a| !valid_accent(a)) {
        return Err(bad("The accent colour must look like #f59e0b"));
    }
    let mut wallpapers: Vec<String> = Vec::new();
    for w in p.wallpapers {
        if !wallpapers.contains(&w) {
            wallpapers.push(w);
        }
    }
    if wallpapers.len() > MAX_WALLPAPERS {
        return Err(bad("Too many wallpapers (20 at most)"));
    }
    let b = Branding { title, subtitle, logo: p.logo.filter(|l| !l.is_empty()), wallpapers, accent };
    let dir = branding_dir(&state);
    for f in b.files() {
        if !valid_file_name(f) || !dir.join(f).is_file() {
            return Err(bad("An image is missing; upload it again"));
        }
    }

    state.db.set_setting(BRANDING_KEY, &serde_json::to_string(&b).map_err(db_err)?).map_err(db_err)?;

    // Remove stored images that are no longer used (including uploads that were never saved).
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if valid_file_name(&name) && !b.files().any(|f| *f == name) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    let details = if b.is_custom() {
        format!(
            "logo {}, {} wallpaper(s), accent {}, title {}",
            if b.logo.is_some() { "custom" } else { "default" },
            b.wallpapers.len(),
            b.accent.as_deref().unwrap_or("default"),
            if b.title.is_empty() { "default" } else { "custom" }
        )
    } else {
        "reset to Remote defaults".to_string()
    };
    state.db.log_audit(Some(&claims.sub), &claims.username, "BRANDING_CHANGED", None, None, Some(&details), None);
    Ok(Json(b.public_view()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_images() {
        let hash = "a".repeat(64);
        assert!(valid_file_name(&format!("{hash}.webp")));
        assert!(!valid_file_name(&format!("{hash}.svg")));
        assert!(!valid_file_name("../../etc/passwd"));
        assert!(!valid_file_name(&format!("{}.png", "A".repeat(64))));
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image(b"<svg onload=alert(1)>"), None);
        assert!(valid_accent("#f59e0b"));
        assert!(!valid_accent("red"));
        assert!(!Branding::default().is_custom());
    }
}
