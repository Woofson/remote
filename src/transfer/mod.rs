//! The shared "Files" folder.
//!
//! The same folder is offered to RDP sessions as the redirected drive `\\tsclient\Files`
//! and is browsable from the web UI. Every client-supplied path goes through
//! [`TransferManager::resolve`], which keeps it inside the folder.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Prefix of temporary files written while an upload is in progress.
const PART_PREFIX: &str = ".remote-part-";

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    /// Path relative to the Files root, `/`-separated.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Clone)]
pub struct TransferManager {
    /// Canonical absolute path of the Files root.
    pub root: PathBuf,
}

/// A path inside the Files root.
pub struct Resolved {
    pub abs: PathBuf,
    /// Cleaned relative path (`""` for the root).
    pub rel: String,
}

impl TransferManager {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        let path = dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&path);
        let root = std::fs::canonicalize(&path).unwrap_or(path);
        Self { root }
    }

    /// Turn a client-supplied relative path into a path inside the root.
    ///
    /// Rejects `..`, backslashes and NUL bytes, and refuses anything whose existing
    /// part resolves (through symlinks) to a location outside the root.
    pub fn resolve(&self, rel: &str) -> Result<Resolved, String> {
        let mut abs = self.root.clone();
        let mut parts = Vec::new();
        for part in rel.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." || part.contains('\\') || part.contains('\0') {
                return Err("Invalid path".into());
            }
            abs.push(part);
            parts.push(part);
        }

        // Check the deepest existing ancestor against the root (symlink escape).
        let mut probe: &Path = &abs;
        loop {
            if std::fs::symlink_metadata(probe).is_ok() {
                let canon = std::fs::canonicalize(probe).map_err(|_| "Invalid path".to_string())?;
                if !canon.starts_with(&self.root) {
                    return Err("Invalid path".into());
                }
                break;
            }
            match probe.parent() {
                Some(p) => probe = p,
                None => return Err("Invalid path".into()),
            }
        }

        Ok(Resolved { abs, rel: parts.join("/") })
    }

    /// List a folder: folders first, then files, both sorted by name.
    pub fn list(&self, rel: &str) -> Result<(String, Vec<FileEntry>), String> {
        let dir = self.resolve(rel)?;
        let read = std::fs::read_dir(&dir.abs).map_err(|_| "Folder not found".to_string())?;

        let mut entries = Vec::new();
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(PART_PREFIX) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            // Symlinks are never shown or followed.
            if entry.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
                continue;
            }
            let modified = meta
                .modified()
                .ok()
                .map(|t| DateTime::<Utc>::from(t).to_rfc3339());
            let path = if dir.rel.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", dir.rel, name)
            };
            entries.push(FileEntry {
                name,
                path,
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
                modified,
            });
        }

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok((dir.rel, entries))
    }

    /// Resolve a path that must be an existing regular file.
    pub fn file(&self, rel: &str) -> Result<Resolved, String> {
        let r = self.resolve(rel)?;
        let meta = std::fs::symlink_metadata(&r.abs).map_err(|_| "File not found".to_string())?;
        if !meta.is_file() {
            return Err("File not found".into());
        }
        Ok(r)
    }

    /// Resolve a path that must be an existing folder.
    pub fn folder(&self, rel: &str) -> Result<Resolved, String> {
        let r = self.resolve(rel)?;
        let meta = std::fs::symlink_metadata(&r.abs).map_err(|_| "Folder not found".to_string())?;
        if !meta.is_dir() {
            return Err("Folder not found".into());
        }
        Ok(r)
    }

    /// A fresh temporary path for an upload in progress inside `dir`.
    pub fn part_path(dir: &Path) -> PathBuf {
        dir.join(format!("{}{}", PART_PREFIX, Uuid::new_v4()))
    }

    /// Move a finished upload into place under a free name. Returns the final relative path.
    pub fn finish_upload(&self, dir: &Resolved, part: &Path, name: &str) -> Result<String, String> {
        let target = unique_path(&dir.abs, name);
        std::fs::rename(part, &target).map_err(|e| format!("Failed to store file: {}", e))?;
        let final_name = target
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| name.to_string());
        Ok(join_rel(&dir.rel, &final_name))
    }

    pub fn mkdir(&self, parent_rel: &str, name: &str) -> Result<String, String> {
        let parent = self.folder(parent_rel)?;
        let name = sanitize_name(name)?;
        let target = parent.abs.join(&name);
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(format!("'{}' already exists", name));
        }
        std::fs::create_dir(&target).map_err(|e| format!("Failed to create folder: {}", e))?;
        Ok(join_rel(&parent.rel, &name))
    }

    /// Delete a file, or a folder with everything in it. The root itself cannot be deleted.
    pub fn delete(&self, rel: &str) -> Result<(String, bool), String> {
        let r = self.resolve(rel)?;
        if r.rel.is_empty() {
            return Err("The Files folder itself cannot be deleted".into());
        }
        let meta = std::fs::symlink_metadata(&r.abs).map_err(|_| "Not found".to_string())?;
        if meta.is_dir() {
            std::fs::remove_dir_all(&r.abs).map_err(|e| format!("Failed to delete folder: {}", e))?;
        } else {
            std::fs::remove_file(&r.abs).map_err(|e| format!("Failed to delete file: {}", e))?;
        }
        Ok((r.rel, meta.is_dir()))
    }
}

/// Make a single file or folder name safe for both Linux and the Windows drive view.
pub fn sanitize_name(raw: &str) -> Result<String, String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." || cleaned.starts_with(PART_PREFIX) {
        return Err("Invalid name".into());
    }
    if cleaned.len() > 255 {
        return Err("Name is too long".into());
    }
    Ok(cleaned)
}

/// `name`, or `name (1).ext`, `name (2).ext`, ... whichever does not exist yet in `dir`.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let ext = p.extension().map(|s| s.to_string_lossy().to_string());
    for n in 1.. {
        let candidate = match &ext {
            Some(e) => format!("{} ({}).{}", stem, n, e),
            None => format!("{} ({})", stem, n),
        };
        let path = dir.join(candidate);
        if std::fs::symlink_metadata(&path).is_err() {
            return path;
        }
    }
    unreachable!()
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> (TransferManager, PathBuf) {
        let dir = std::env::temp_dir().join(format!("remote-files-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        (TransferManager::new(&dir), dir)
    }

    #[test]
    fn rejects_escapes() {
        let (m, dir) = temp_root();
        assert!(m.resolve("../etc/passwd").is_err());
        assert!(m.resolve("a/../../x").is_err());
        assert!(m.resolve("a\\..\\x").is_err());
        assert_eq!(m.resolve("/a//b/./c").unwrap().rel, "a/b/c");
        assert_eq!(m.resolve("").unwrap().rel, "");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let (m, dir) = temp_root();
        std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
        assert!(m.resolve("link/passwd").is_err());
        assert!(m.resolve("link").is_err());
        assert!(m.list("").unwrap().1.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_and_clashes() {
        let (m, dir) = temp_root();
        assert_eq!(sanitize_name("C:\\Users\\a\\re:port?.txt").unwrap(), "re_port_.txt");
        assert!(sanitize_name("..").is_err());
        assert!(sanitize_name("   ").is_err());
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        std::fs::write(dir.join("a (1).txt"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        assert_eq!(m.mkdir("", "docs").unwrap(), "docs");
        assert!(m.mkdir("", "docs").is_err());
        assert!(m.delete("").is_err());
        assert_eq!(m.delete("docs").unwrap(), ("docs".to_string(), true));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
