//! Wox discovery-metadata stub generation.
//!
//! Wox discovers script plugins by scanning
//! `~/.wox/wox-user/plugins/scripts/` and parsing each file's leading `#`/`//`
//! comment block as JSON (`wox.core/plugin/inline_metadata.go`). It then
//! executes the plugin file itself, resolving the interpreter from the
//! extension: an empty extension yields interpreter `""` (direct execution),
//! and on Windows Go's `exec` appends PATHEXT so an extension-less stub
//! resolves to the sibling `quiver.exe`.
//!
//! POSIX has no PATHEXT, so a single file cannot be both the `#`-header stub
//! and an ELF. There the stub carries an extension Wox does not know and a
//! shebang naming the interpreter by basename, so `quiver` must be on PATH.
//!
//! Both flavours are rendered here from `identity`, which keeps the plugin's
//! identity in Rust and the repository free of generated stub files.
//!
//! The metadata fields this module writes (`Id` / `Name` / `Description` /
//! `Version` / `Build` / `TriggerKeywords` / `SupportedOS` / `Runtime`) are
//! the wire contract Wox's discovery parser consumes. The authoritative
//! field reference lives in [`docs/catalog-contract.md`]; any field added
//! here belongs in a section over there.

use crate::identity;

use serde::Serialize;

/// Which stub flavour to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Extension-less stub executed directly; sibling `quiver.exe` is the binary.
    Windows,
    /// `quiver.sh` stub whose shebang names `quiver` on PATH.
    Posix,
}

impl Layout {
    /// Parse the `--stub` argument. With no argument, render the flavour that
    /// suits the host's own Wox (`platform::IS_WINDOWS`, a compile-time const).
    pub fn from_arg(arg: Option<&str>) -> Self {
        match arg {
            Some("windows") => Layout::Windows,
            Some("posix") => Layout::Posix,
            Some(other) => {
                tracing::warn!(
                    got = other,
                    "--stub received unknown layout; falling back to host default"
                );
                if crate::platform::IS_WINDOWS {
                    Layout::Windows
                } else {
                    Layout::Posix
                }
            }
            _ if crate::platform::IS_WINDOWS => Layout::Windows,
            _ => Layout::Posix,
        }
    }
}

/// Render the full stub file contents.
///
/// `MinWoxVersion` is deliberately absent: Wox defaults it to 2.0.0 for script
/// plugins (`validateAndSetScriptMetadataDefaults`, `plugin/manager.go`), and a
/// floor is a **hard** compatibility gate — declaring 2.4.2 would silently
/// refuse to load on a 2.4.0/2.4.1 install that works fine.
///
/// The comment header is the JSON block and nothing else. Wox strips the comment
/// markers from the leading header and takes everything from the first `{`
/// onwards as the metadata (`inline_metadata.go: parseInlineMetadataContent`,
/// `strings.Index(header, "{")`), so any prose in that header would only be safe
/// while it contains no brace — one `{` in a description and the block is
/// misplaced, discovery fails, silently. Emitting nothing but the block makes
/// that trap structurally unreachable, and the file's purpose is already carried
/// by `Runtime`/`Name`/`Description` inside it.
pub fn render(layout: Layout) -> String {
    let metadata = build_metadata();

    let mut out = String::new();
    if layout == Layout::Posix {
        out.push_str("#!/usr/bin/env quiver\n");
    }
    out.push_str(&comment_block(&metadata));
    out
}

/// Wox discovery-metadata object, written into the `#`-commented header of
/// the stub. Field order is pinned by the wire tests in `protocol::tests`
/// (and by Wox's own plugin metadata parser); `serde` emits fields in
/// struct-declaration order, so what you see here is what hits the wire.
#[derive(Serialize)]
struct Metadata<'a> {
    #[serde(rename = "Id")]
    id: &'a str,
    #[serde(rename = "Name")]
    name: &'a str,
    #[serde(rename = "Description")]
    description: &'a str,
    #[serde(rename = "Author")]
    author: &'a str,
    #[serde(rename = "Version")]
    version: &'a str,
    /// `commit (profile timestamp)` from [`identity::build_field`] — Wox
    /// shows `Version` to users, so the stub adds `Build` for operators
    /// who need to diff two artefacts by reading the embedded commit.
    /// Wox ignores unknown fields, so the addition is forward-safe.
    /// Empty in dev builds without a discoverable HEAD → the field is
    /// omitted entirely (`skip_serializing_if`).
    #[serde(rename = "Build", skip_serializing_if = "str::is_empty")]
    build: String,
    #[serde(rename = "Runtime")]
    runtime: &'static str,
    #[serde(rename = "Website")]
    website: &'a str,
    #[serde(rename = "Icon")]
    icon: &'a str,
    #[serde(rename = "TriggerKeywords")]
    trigger_keywords: &'a [&'a str],
    // The spellings Wox's own plugins declare (note "Macos").
    #[serde(rename = "SupportedOS")]
    supported_os: &'a [&'static str],
}

/// Build the discovery-metadata JSON object. All fields come from
/// [`identity`]; the only allocation is the JSON value itself, plus the
/// 2-element `SupportedOS` array — the same total as the hand-rolled
/// `IndexMap` insert sequence it replaces.
fn build_metadata() -> serde_json::Value {
    serde_json::to_value(Metadata {
        id: identity::ID,
        name: identity::NAME,
        description: identity::DESCRIPTION,
        author: identity::AUTHOR,
        version: identity::VERSION,
        // Skip the field entirely when `build_field()` is empty (the
        // `COMMIT == "unknown"` path), so dev builds without a
        // discoverable HEAD don't smuggle a "unknown" string into the
        // Wox UI.
        build: identity::build_field(),
        runtime: "SCRIPT",
        website: identity::WEBSITE,
        icon: identity::ICON,
        trigger_keywords: &identity::TRIGGER_KEYWORDS,
        supported_os: &["Windows", "Linux", "Macos"],
    })
    .expect("metadata is a fixed shape; serialization cannot fail")
}

/// Render a JSON value as a `#`-prefixed comment block.
fn comment_block(value: &serde_json::Value) -> String {
    let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
    let mut out = String::new();
    for line in pretty.lines() {
        out.push_str("# ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StrField;

    /// Parse a stub exactly the way `inline_metadata.go` does: skip a shebang,
    /// collect consecutive `#`/`//` comments, then parse the first JSON value.
    fn parse_like_wox(text: &str) -> serde_json::Value {
        let mut header = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("#!") {
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            if !(trimmed.starts_with('#') || trimmed.starts_with("//")) {
                break;
            }
            header.push_str(
                trimmed
                    .trim_start_matches('#')
                    .trim_start_matches("//")
                    .trim(),
            );
            header.push('\n');
        }
        let start = header.find('{').expect("metadata JSON block");
        serde_json::from_str(&header[start..]).expect("valid metadata JSON")
    }

    #[test]
    fn both_layouts_expose_the_identity_wox_needs() {
        for layout in [Layout::Windows, Layout::Posix] {
            let meta = parse_like_wox(&render(layout));
            assert_eq!(meta.str_field("Id"), identity::ID);
            assert_eq!(meta.str_field("Name"), identity::NAME);
            assert_eq!(meta.str_field("Runtime"), "SCRIPT");
            assert!(
                !meta
                    .get("TriggerKeywords")
                    .and_then(serde_json::Value::as_array)
                    .expect("keywords")
                    .is_empty()
            );
            assert!(
                !meta
                    .get("SupportedOS")
                    .and_then(serde_json::Value::as_array)
                    .expect("supported os")
                    .is_empty()
            );
        }
    }

    #[test]
    fn no_artificial_min_wox_version_floor() {
        // Wox defaults MinWoxVersion to 2.0.0 for script plugins; declaring a
        // higher floor would be a hard gate against installs that work.
        let meta = parse_like_wox(&render(Layout::Windows));
        assert!(meta.get("MinWoxVersion").is_none());
        assert!(!meta.str_field("Version").is_empty());
    }

    #[test]
    fn only_the_posix_layout_carries_a_shebang() {
        assert!(render(Layout::Posix).starts_with("#!/usr/bin/env quiver\n"));
        assert!(render(Layout::Windows).starts_with("# "));
    }

    #[test]
    fn nothing_but_comment_markers_precedes_the_metadata_json() {
        // Wox takes the metadata from the first `{` in the whole comment header
        // (`strings.Index(header, "{")`), so any brace in prose emitted *before*
        // the block would displace it and silently break discovery. Pin the
        // ordering: up to the first `{`, the stub is a shebang and comment
        // markers only.
        for layout in [Layout::Windows, Layout::Posix] {
            let stub = render(layout);
            let json_offset = stub.find('{').expect("metadata JSON block");
            for line in stub[..json_offset].lines() {
                let trimmed = line.trim();
                assert!(
                    trimmed.is_empty()
                        || trimmed == "#"
                        || trimmed.starts_with("#!"),
                    "{layout:?}: prose precedes the JSON block: {line:?}"
                );
            }
        }
    }

    /// The entry names the install hook writes — `quiver` (extension-less,
    /// PATHEXT-resolved to the sibling `quiver.exe`) on Windows,
    /// `quiver.sh` on POSIX — pair with the layouts here; the shebang is
    /// what makes the POSIX one runnable.
    #[test]
    fn entry_names_stay_paired_with_their_layouts() {
        assert!(
            render(Layout::Posix).starts_with("#!/usr/bin/env quiver"),
            "the POSIX stub's shebang names the PATH binary"
        );
    }
}
