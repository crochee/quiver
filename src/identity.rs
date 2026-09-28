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

pub const WEBSITE: &str = "https://github.com/crochee/dotfiles/tree/main/home/dot_config/wox/plugins/quiver";

/// Keywords that route a Wox query to this plugin.
pub const TRIGGER_KEYWORDS: [&str; 2] = ["quiver", "qv"];

/// Author shown in Wox's plugin metadata. Lives here — not inline in
/// `stub.rs` — so every render of the plugin's identity agrees with this
/// one file.
pub const AUTHOR: &str = "dotfiles";

/// Result-row icon, inline so the plugin ships no image assets.
pub const ICON: &str = "svg:<svg xmlns='http://www.w3.org/2000/svg' width='48' height='48' viewBox='0 0 48 48'><rect width='48' height='48' rx='12' fill='#1f6feb'/><path d='M24 6v30' stroke='#fff' stroke-width='3' stroke-linecap='round'/><path d='M24 6l-6 9h12z' fill='#fff'/><path d='M24 6l-6 9M24 6l6 9' stroke='#fff' stroke-width='3' stroke-linecap='round'/><path d='M14 30c0 6 20 6 20 0' fill='none' stroke='#bfdbfe' stroke-width='3'/></svg>";
