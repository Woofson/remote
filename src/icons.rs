//! Connection icons: picked from selfh.st/icons or uploaded.
//!
//! The server downloads and caches everything itself (under `<data_dir>/icons`), so
//! browsers never contact selfh.st and chosen icons keep working offline.
//! A connection's `icon` is `selfhst:<slug>` or `upload:<sha256>`; anything else
//! means "protocol default".

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;
use tracing::{info, warn};

const INDEX_URL: &str = "https://raw.githubusercontent.com/selfhst/icons/main/index.json";
const PNG_URL: &str = "https://cdn.jsdelivr.net/gh/selfhst/icons/png/";
const INDEX_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
const MAX_SELFHST_PNG: usize = 1024 * 1024;
pub const MAX_UPLOAD_PNG: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct SelfhstIcon {
    pub name: String,
    pub slug: String,
    pub category: String,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Reference")]
    reference: String,
    #[serde(rename = "PNG", default)]
    png: String,
    #[serde(rename = "Category", default)]
    category: String,
    #[serde(rename = "Tags", default)]
    tags: String,
}

pub struct IconStore {
    dir: PathBuf,
    client: reqwest::Client,
    index: RwLock<Option<(Instant, Arc<Vec<(SelfhstIcon, String)>>)>>,
}

pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn valid_hash(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Allowed values for a connection's icon field.
pub fn valid_icon_ref(s: &str) -> bool {
    match s.split_once(':') {
        Some(("selfhst", slug)) => valid_slug(slug),
        Some(("upload", hash)) => valid_hash(hash),
        _ => false,
    }
}

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

impl IconStore {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let dir = data_dir.join("icons");
        let _ = std::fs::create_dir_all(dir.join("selfhst"));
        let _ = std::fs::create_dir_all(dir.join("upload"));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("Remote/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { dir, client, index: RwLock::new(None) }
    }

    fn index_file(&self) -> PathBuf {
        self.dir.join("selfhst-index.json")
    }

    fn parse_index(bytes: &[u8]) -> Option<Vec<(SelfhstIcon, String)>> {
        let raw: Vec<RawEntry> = serde_json::from_slice(bytes).ok()?;
        Some(
            raw.into_iter()
                .filter(|e| e.png.eq_ignore_ascii_case("yes") && valid_slug(&e.reference))
                .map(|e| {
                    let hay = format!("{} {} {} {}", e.name, e.reference, e.category, e.tags).to_lowercase();
                    (SelfhstIcon { name: e.name, slug: e.reference, category: e.category }, hay)
                })
                .collect(),
        )
    }

    async fn fetch_bytes(&self, url: &str, limit: usize) -> Result<Vec<u8>, String> {
        let resp = self.client.get(url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        if resp.content_length().is_some_and(|l| l as usize > limit) {
            return Err("too large".into());
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        if bytes.len() > limit {
            return Err("too large".into());
        }
        Ok(bytes.to_vec())
    }

    /// The selfh.st list: memory, then a fresh disk copy, then download; a stale copy if offline.
    async fn index(&self) -> Result<Arc<Vec<(SelfhstIcon, String)>>, String> {
        if let Some((at, list)) = self.index.read().await.as_ref() {
            if at.elapsed() < INDEX_MAX_AGE {
                return Ok(list.clone());
            }
        }
        let file = self.index_file();
        let disk_age = std::fs::metadata(&file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok());
        let stale = std::fs::read(&file).ok().and_then(|b| Self::parse_index(&b));

        let list = if let (Some(list), Some(age)) = (&stale, disk_age) {
            if age < INDEX_MAX_AGE {
                Some(list.clone())
            } else {
                None
            }
        } else {
            None
        };
        let list = match list {
            Some(l) => l,
            None => match self.fetch_bytes(INDEX_URL, MAX_INDEX_BYTES).await {
                Ok(bytes) => match Self::parse_index(&bytes) {
                    Some(l) => {
                        let _ = std::fs::write(&file, &bytes);
                        info!("Icons: downloaded the selfh.st list ({} icons)", l.len());
                        l
                    }
                    None => stale.ok_or("The selfh.st icon list could not be read")?,
                },
                Err(e) => {
                    warn!("Icons: cannot download the selfh.st list: {}", e);
                    stale.ok_or("selfh.st is not reachable from the Remote server")?
                }
            },
        };
        let list = Arc::new(list);
        *self.index.write().await = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// Search by name, slug, category or tags. Name/slug prefix matches come first.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SelfhstIcon>, String> {
        let list = self.index().await?;
        let q = query.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(u8, &SelfhstIcon)> = list
            .iter()
            .filter(|(_, hay)| words.iter().all(|w| hay.contains(w)))
            .map(|(icon, _)| {
                let rank = if q.is_empty() {
                    2
                } else if icon.slug.starts_with(&q) || icon.name.to_lowercase().starts_with(&q) {
                    0
                } else {
                    1
                };
                (rank, icon)
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase())));
        Ok(hits.into_iter().take(limit).map(|(_, i)| i.clone()).collect())
    }

    /// A selfh.st PNG, downloaded once and then served from the cache.
    pub async fn selfhst_png(&self, slug: &str) -> Result<Vec<u8>, String> {
        if !valid_slug(slug) {
            return Err("Invalid icon".into());
        }
        let path = self.dir.join("selfhst").join(format!("{slug}.png"));
        if let Ok(bytes) = std::fs::read(&path) {
            return Ok(bytes);
        }
        let bytes = self.fetch_bytes(&format!("{PNG_URL}{slug}.png"), MAX_SELFHST_PNG).await?;
        if !is_png(&bytes) {
            return Err("Not a PNG image".into());
        }
        let _ = std::fs::write(&path, &bytes);
        Ok(bytes)
    }

    /// Store an uploaded PNG (already resized by the browser). Returns the icon reference.
    pub fn save_upload(&self, bytes: &[u8]) -> Result<String, String> {
        if bytes.len() > MAX_UPLOAD_PNG {
            return Err("Icon is too large (max 256 KB)".into());
        }
        if !is_png(bytes) {
            return Err("Icon must be a PNG image".into());
        }
        let hash = hex::encode(Sha256::digest(bytes));
        std::fs::write(self.dir.join("upload").join(format!("{hash}.png")), bytes)
            .map_err(|e| format!("Cannot store icon: {e}"))?;
        Ok(format!("upload:{hash}"))
    }

    pub fn read_upload(&self, hash: &str) -> Result<Vec<u8>, String> {
        if !valid_hash(hash) {
            return Err("Invalid icon".into());
        }
        std::fs::read(self.dir.join("upload").join(format!("{hash}.png"))).map_err(|_| "Icon not found".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_refs_and_index_parsing() {
        assert!(valid_icon_ref("selfhst:proxmox"));
        assert!(valid_icon_ref(&format!("upload:{}", "a".repeat(64))));
        assert!(!valid_icon_ref("selfhst:../etc/passwd"));
        assert!(!valid_icon_ref("upload:xyz"));
        assert!(!valid_icon_ref("https://evil.example/x.png"));
        assert!(!valid_icon_ref("terminal"));

        let json = br#"[{"Name":"Proxmox","Reference":"proxmox","PNG":"Yes","Category":"Hypervisor","Tags":"vm"},
                        {"Name":"Bad","Reference":"../x","PNG":"Yes"},
                        {"Name":"NoPng","Reference":"nopng","PNG":"No"}]"#;
        let list = IconStore::parse_index(json).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0.slug, "proxmox");

        let dir = std::env::temp_dir().join(format!("remote-icons-test-{}", uuid::Uuid::new_v4()));
        let store = IconStore::new(&dir);
        assert!(store.save_upload(b"not a png").is_err());
        let png = b"\x89PNG\r\n\x1a\nrest-of-image";
        let r = store.save_upload(png).unwrap();
        let hash = r.strip_prefix("upload:").unwrap();
        assert_eq!(store.read_upload(hash).unwrap(), png.to_vec());
        assert!(store.read_upload("../../secret").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
