//! Plugin identity — the single source of truth for everything Wox and the
//! install hook need to know about this plugin.
//!
//! `stub` renders this into Wox's discovery metadata and `protocol` uses the
//! icon for result rows, so identity is declared once and referenced, never
//! duplicated.

/// Stable plugin id. Wox keys settings and MRU on this value.
pub const ID: &str = "4f822515-6db6-407f-a35e-1e6d55ddb56e";

pub const NAME: &str = "Quiver";

pub const DESCRIPTION: &str = "A quiver of shell one-liners: every alias in ShellCommands.json is an arrow, nocked by keyword and loosed with Enter. Native Rust, zero runtime dependencies.";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git commit the binary was built from, set by `build.rs`. CI passes the
/// exact tag SHA via the `QUIVER_GIT_SHA` env-var; local dev falls back to
/// `git rev-parse HEAD`. `"unknown"` only appears when git is unreachable
/// AND no env-var was set (e.g. `cargo-chef` recipe cook with no `.git`).
pub const COMMIT: &str = match option_env!("QUIVER_GIT_SHA") {
    Some(s) if !s.is_empty() => s,
    _ => "unknown",
};

/// Cargo profile (`release` / `debug`) the binary was built with, so a
/// `--version` line distinguishes a dev build from a tagged artefact.
pub const BUILD_PROFILE: &str = match option_env!("QUIVER_BUILD_PROFILE") {
    Some(s) if !s.is_empty() => s,
    _ => "unknown",
};

/// UTC timestamp the binary was linked, ISO-8601 (`…Z`). Set by `build.rs`
/// from `date -u` when no `QUIVER_BUILD_TIME` env-var is provided (CI
/// injects the tag-time). `unknown` when neither path is available — the
/// `--version` line is still parseable, it just lacks the clock.
pub const BUILD_TIME: &str = match option_env!("QUIVER_BUILD_TIME") {
    Some(s) if !s.is_empty() => s,
    _ => "unknown",
};

/// `version [-dirty] (profile timestamp)` — the single string
/// `--version` and the Wox discovery stub print. `COMMIT == "unknown"` is
/// the only case where the SHA segment is omitted (see `COMMIT` doc).
pub fn full_version() -> String {
    let sha = if COMMIT == "unknown" {
        String::new()
    } else {
        format!(" {COMMIT}")
    };
    format!("{VERSION}{sha} ({BUILD_PROFILE} {BUILD_TIME})",)
}

/// `commit (profile timestamp)` — the stub's `Build` field. Empty when no
/// commit is discoverable so the stub omits the field entirely
/// (`skip_serializing_if = "str::is_empty"` on `Metadata::build`).
pub fn build_field() -> String {
    if COMMIT == "unknown" {
        return String::new();
    }
    format!("{COMMIT} ({BUILD_PROFILE} {BUILD_TIME})",)
}

pub const WEBSITE: &str = "https://github.com/crochee/quiver";

/// Keywords that route a Wox query to this plugin.
pub const TRIGGER_KEYWORDS: [&str; 2] = ["quiver", "qv"];

/// Author shown in Wox's plugin metadata. Lives here — not inline in
/// `stub.rs` — so every render of the plugin's identity agrees with this
/// one file.
pub const AUTHOR: &str = "crochee";

/// Result-row icon, inline so the plugin ships no image assets.
pub const ICON: &str = "svg:<svg xmlns='http://www.w3.org/2000/svg' width='48' height='48' viewBox='0 0 48 48'><rect width='48' height='48' rx='12' fill='#1f6feb'/><path d='M24 6v30' stroke='#fff' stroke-width='3' stroke-linecap='round'/><path d='M24 6l-6 9h12z' fill='#fff'/><path d='M24 6l-6 9M24 6l6 9' stroke='#fff' stroke-width='3' stroke-linecap='round'/><path d='M14 30c0 6 20 6 20 0' fill='none' stroke='#bfdbfe' stroke-width='3'/></svg>";
