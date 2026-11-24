//! Build information shown in the version text (as in Brum): build number, commit, time.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok().map(|s| s.trim().to_string())
}

fn main() {
    // Commit ID, marked "-dirty" when built with uncommitted changes; "release" without git.
    let commit = git(&["rev-parse", "--short", "HEAD"]);
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let commit_str = match &commit {
        Some(c) if dirty => format!("{c}-dirty"),
        Some(c) => c.clone(),
        None => "release".to_string(),
    };

    // Build number: commit count, or a local counter that keeps rising without git.
    let commit_count: u64 = git(&["rev-list", "--count", "HEAD"]).and_then(|s| s.parse().ok()).unwrap_or(1);
    let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
    let counter_file = std::path::Path::new(&out_dir).join(".build_counter");
    let previous: u64 = std::fs::read_to_string(&counter_file).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let build_number = if commit.is_some() { commit_count } else { previous + 1 };
    let _ = std::fs::write(&counter_file, build_number.to_string());

    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

    println!("cargo:rustc-env=REMOTE_BUILD_NUMBER={build_number}");
    println!("cargo:rustc-env=REMOTE_BUILD_COMMIT={commit_str}");
    println!("cargo:rustc-env=REMOTE_BUILD_TIMESTAMP={timestamp}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
