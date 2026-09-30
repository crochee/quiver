//! Inject release-time identity (git commit, build profile, timestamp) into
//! the binary so the produced artefact answers "what is this?" without an
//! external manifest.
//!
//! Source of truth, in priority order:
//!   1. `QUIVER_GIT_SHA` — set by `release.yml` (deterministic per release
//!      tag) and by the Docker build arg, so CI never depends on the build
//!      host having a `.git` dir.
//!   2. `git rev-parse` — local developer builds (`make build`); falls back
//!      gracefully when `.git` is absent or git is not on PATH (cargo-chef
//!      container without git).
//!   3. `"unknown"` — last-resort placeholder, so the binary still links
//!      and `quiver --version` still prints a parseable version string.
//!
//! We emit `cargo:rustc-env=` (not `cargo:rerun-if-changed=`) so a source
//! edit that touches no git object still triggers a rebuild when the HEAD
//! moves — HEAD can change without any tracked file changing (amend,
//! rebase, checkout of a tag). The dependency on `git` is declared via
//! `cargo:rerun-if-env-changed=QUIVER_GIT_SHA` (env-driven) and via the
//! `.git/HEAD` path below (host-driven).
use std::path::Path;
use std::process::Command;

fn main() {
    let commit = std::env::var("QUIVER_GIT_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(git_head)
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = std::env::var("QUIVER_GIT_DIRTY")
        .ok()
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .or_else(git_dirty)
        .unwrap_or(false);
    let mut commit = commit;
    if dirty && commit != "unknown" {
        commit.push_str("-dirty");
    }

    // `PROFILE` is set by cargo itself; "release" / "debug" / "test" — used
    // by `--version` so a debug build is visibly not a release artefact.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    // `chrono` would be one more crate; `BUILD_TIME` is a frozen marker, not
    // a clock, so RFC-3339 from `date -u` is enough and keeps the
    // dependency list unchanged.
    let build_time = std::env::var("QUIVER_BUILD_TIME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            Command::new("date")
                .arg("-u")
                .arg("+%Y-%m-%dT%H:%M:%SZ")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        });

    println!("cargo:rustc-env=QUIVER_GIT_SHA={commit}");
    println!("cargo:rustc-env=QUIVER_BUILD_PROFILE={profile}");
    println!("cargo:rustc-env=QUIVER_BUILD_TIME={build_time}");
    // Rerun when the env-var inputs change (CI) or when git HEAD moves
    // (local dev); git's own mtime against the rest of the repo is irrelevant.
    println!("cargo:rerun-if-env-changed=QUIVER_GIT_SHA");
    println!("cargo:rerun-if-env-changed=QUIVER_GIT_DIRTY");
    println!("cargo:rerun-if-env-changed=QUIVER_BUILD_TIME");
    if Path::new(".git/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
    }
}

fn git_head() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn git_dirty() -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(!output.stdout.is_empty())
}
