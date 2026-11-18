//! Colour themes, in the same file format as Brum (`themes/*.toml`, `[[themes]]` entries).
//!
//! Built-in themes are compiled in. Extra theme files are read at startup from
//! `./themes` and `<data_dir>/themes`; a theme with the same id replaces the built-in one.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub bg_dark: String,
    pub bg_panel: String,
    pub bg_active: String,
    pub accent: String,
    pub accent_hover: String,
    pub text_main: String,
    pub text_muted: String,
    pub border: String,
    #[serde(default)]
    pub radius: Option<String>,
}

#[derive(Deserialize)]
struct ThemeFile {
    #[serde(default)]
    themes: Vec<ThemeDefinition>,
}

/// The theme used until a user picks one.
pub const DEFAULT_THEME: &str = "amber-charcoal";

const BUILT_IN: &[&str] = &[
    include_str!("../themes/amber.toml"),
    include_str!("../themes/woofsons-plus.toml"),
    include_str!("../themes/catppuccin.toml"),
    include_str!("../themes/nord.toml"),
    include_str!("../themes/dracula.toml"),
    include_str!("../themes/tokyo-night.toml"),
    include_str!("../themes/gruvbox.toml"),
    include_str!("../themes/solarized.toml"),
    include_str!("../themes/monokai.toml"),
    include_str!("../themes/cyberpunk.toml"),
    include_str!("../themes/kittelsen.toml"),
];

fn is_colour(s: &str) -> bool {
    let s = s.trim();
    (s.starts_with('#') && (s.len() == 7 || s.len() == 4) && s[1..].chars().all(|c| c.is_ascii_hexdigit()))
        || s.starts_with("rgb")
}

/// Only plain colours and a simple radius get through, so a theme file cannot inject CSS.
fn valid(t: &ThemeDefinition) -> bool {
    let colours = [
        &t.bg_dark, &t.bg_panel, &t.bg_active, &t.accent, &t.accent_hover, &t.text_main, &t.text_muted, &t.border,
    ];
    let radius_ok = t
        .radius
        .as_deref()
        .map(|r| r.len() <= 6 && r.ends_with("px") && r[..r.len() - 2].chars().all(|c| c.is_ascii_digit()))
        .unwrap_or(true);
    !t.id.is_empty()
        && t.id.len() <= 64
        && t.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && colours.iter().all(|c| is_colour(c) && !c.contains(';') && !c.contains('}'))
        && radius_ok
}

fn parse_into(list: &mut Vec<ThemeDefinition>, content: &str, source: &str) {
    match toml::from_str::<ThemeFile>(content) {
        Ok(file) => {
            for t in file.themes {
                if !valid(&t) {
                    warn!("Themes: skipped invalid theme '{}' in {}", t.id, source);
                    continue;
                }
                match list.iter_mut().find(|x| x.id == t.id) {
                    Some(existing) => *existing = t,
                    None => list.push(t),
                }
            }
        }
        Err(e) => warn!("Themes: cannot read {}: {}", source, e),
    }
}

/// All themes: built-in, then extra files from the given folders (sorted by file name).
pub fn load_themes(extra_dirs: &[&Path]) -> Vec<ThemeDefinition> {
    let mut list = Vec::new();
    for content in BUILT_IN {
        parse_into(&mut list, content, "built-in themes");
    }
    for dir in extra_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for f in files {
            if let Ok(content) = std::fs::read_to_string(&f) {
                info!("Themes: loading {}", f.display());
                parse_into(&mut list, &content, &f.display().to_string());
            }
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_load_and_bad_themes_are_rejected() {
        let list = load_themes(&[]);
        assert!(list.len() >= 30, "built-in themes: {}", list.len());
        assert!(list.iter().any(|t| t.id == DEFAULT_THEME));
        assert!(list.iter().any(|t| t.id == "zink"));

        let mut l = Vec::new();
        parse_into(
            &mut l,
            r##"[[themes]]
id = "evil"
name = "Evil"
bg_dark = "#000000; } body { display:none"
bg_panel = "#111111"
bg_active = "#222222"
accent = "#f59e0b"
accent_hover = "#fbbf24"
text_main = "#ffffff"
text_muted = "#aaaaaa"
border = "#333333"
"##,
            "test",
        );
        assert!(l.is_empty(), "CSS injection must be rejected");
    }
}
