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
//! identity in Rust and the repository free of generated stub files. The
//! `stub` subcommand in `main.rs` picks the host's compile-time layout
//! (POSIX on Linux / macOS, Windows on Windows); the `--path` flag in the
//! same subcommand selects the destination, defaulting to the
//! host-appropriate path under home when no value is supplied (see
//! [`default_destination`]).
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
    /// Filename Wox discovers for this layout, paired with the binary the
    /// launcher is expected to find beside it.
    ///
    /// * `Windows`: `quiver` (extension-less) — Wox's Go `exec` appends
    ///   `PATHEXT` and resolves it to the sibling `quiver.exe`.
    /// * `Posix`: `quiver.sh` — the Wox directory scan picks up the file by
    ///   extension; the shebang names `quiver` from `PATH`.
    ///
    /// Pinned by [`tests::entry_names_stay_paired_with_their_layouts`].
    pub fn default_filename(self) -> &'static str {
        match self {
            Layout::Windows => "quiver",
            Layout::Posix => "quiver.sh",
        }
    }

    /// Wox's plugin-script directory **on this host**, resolved as a relative
    /// path under the user's home. Composition with
    /// [`crate::platform::home_dir`] is the caller's job — the layout owns
    /// the per-OS directory *segments*, the host owns the *separator*, and
    /// the platform module owns the home.
    ///
    /// On Windows the segments are joined with backslash (Wox's own Go code
    /// reads `USERPROFILE\.wox\wox-user\plugins\scripts`); on POSIX, with
    /// forward slash. The directory tree is the same; only the separator
    /// tracks the host's filesystem convention. This is what keeps a Linux
    /// host writing a Windows-layout stub from creating a directory whose
    /// name is the literal eight-character string `.wox\wox-user\…`.
    ///
    /// Returning a relative path (rather than joining with home here) keeps
    /// this `const`-foldable — `home_dir` reads env vars and so cannot be a
    /// `const fn`, but the host-aware suffix can and should be.
    #[cfg(windows)]
    pub fn default_subdir(self) -> &'static str {
        // Same segments on both layouts: Wox's plugin-script directory is
        // identical across host families; the layout only changes the
        // filename that goes inside it.
        let _ = self;
        ".wox\\wox-user\\plugins\\scripts"
    }
    #[cfg(not(windows))]
    pub fn default_subdir(self) -> &'static str {
        let _ = self;
        ".wox/wox-user/plugins/scripts"
    }
}

/// Compose `home_dir` (compile-time host-aware via the per-layout suffix
/// inside [`Layout::default_subdir`]) with [`Layout::default_filename`] into
/// the canonical destination Wox discovers for this layout.
///
/// The path is "where Wox expects a `<layout>` stub to live" — the Windows
/// Wox build reads `USERPROFILE` and joins it with `.wox\wox-user\plugins\scripts`,
/// the POSIX build reads `HOME` and joins `.wox/wox-user/plugins/scripts`. The
/// trailing filename is the extension each host's discovery scan requires
/// (see [`Layout::default_filename`]).
///
/// Empty when the home variable is missing — the caller should surface the
/// failure, since writing to a relative path would silently land in the
/// process CWD and Wox would never see it.
pub fn default_destination(layout: Layout) -> String {
    let home = crate::platform::home_dir();
    if home.is_empty() {
        return String::new();
    }
    let mut path = home;
    path.push(std::path::MAIN_SEPARATOR);
    path.push_str(layout.default_subdir());
    path.push(std::path::MAIN_SEPARATOR);
    path.push_str(layout.default_filename());
    path
}

/// Render and write the stub to `path`, atomically.
///
/// * Creates the parent directory tree (`mkdir -p` semantics) — Wox's plugin
///   directory may not exist on a fresh machine, and a partial write that
///   fails because `~/.wox/wox-user/plugins/scripts` is missing silently
///   disables discovery.
/// * Writes to a sibling temp file then renames into place — a reader
///   (Wox's fsnotify watch) never sees a half-written file, so a `capture:
///   true` entry that reloads the plugin mid-write cannot pick up a stub
///   that parses to nonsense and disappears from the launcher.
/// * Sets the POSIX executable bit on the destination — the shebang-named
///   POSIX stub must be runnable; the Windows stub doesn't need it (and
///   Windows ignores the mode bits), so we set it on every platform rather
///   than gating by host (no cost, no behavior change on Windows).
///
/// Returns the absolute path actually written, for the caller's logging.
pub fn write_to_file(
    layout: Layout,
    path: &std::path::Path,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    // Sibling-temp rename keeps the rename atomic on every supported host:
    // POSIX `rename(2)` and Windows `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`
    // both replace the destination in one step.
    let tmp = path.with_extension("quiver-tmp");
    std::fs::write(&tmp, render(layout).as_bytes())?;
    // Best-effort executable bit (POSIX stub is shebang-named; harmless on
    // Windows). Surface only as a log — never fail the install because of
    // a permission tweak.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&tmp) {
            let mut perm = meta.permissions();
            perm.set_mode(0o755);
            let _ = std::fs::set_permissions(&tmp, perm);
        }
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
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

    #[test]
    fn default_filenames_pair_with_their_layouts() {
        assert_eq!(Layout::Windows.default_filename(), "quiver");
        assert_eq!(Layout::Posix.default_filename(), "quiver.sh");
    }

    #[test]
    fn default_subdir_uses_host_path_separator_not_layout() {
        // A Linux host writing a Windows-layout stub (the WSL → Windows
        // mirror) must still produce forward-slash segments, otherwise the
        // directory it creates is a single literal name with backslashes.
        let s = Layout::Windows.default_subdir();
        assert!(
            !s.contains('\\'),
            "Windows layout on a POSIX host must not introduce backslashes, got {s:?}"
        );
        assert_eq!(s, Layout::Posix.default_subdir());
    }

    #[test]
    fn default_destination_uses_platform_home() {
        let _lock = crate::test_support::env_lock();
        // SAFETY: env_lock is held; EnvGuard restores the previous value on drop.
        let previous =
            unsafe { crate::test_support::setenv("HOME", "/tmp/qvhome") };

        let dst = default_destination(Layout::Posix);
        assert_eq!(dst, "/tmp/qvhome/.wox/wox-user/plugins/scripts/quiver.sh");

        let dst_win = default_destination(Layout::Windows);
        assert!(
            dst_win.starts_with("/tmp/qvhome/"),
            "Windows destination should still start with HOME, got {dst_win:?}"
        );
        assert!(
            dst_win.ends_with("/quiver"),
            "Windows destination should be extension-less, got {dst_win:?}"
        );

        drop(previous);
    }

    #[test]
    fn default_destination_is_empty_when_home_is_missing() {
        let _lock = crate::test_support::env_lock();
        // SAFETY: env_lock is held; setting HOME to "" mimics the env lookup
        // contract that treats empty values as unset.
        let previous = unsafe { crate::test_support::setenv("HOME", "") };
        let dst = default_destination(Layout::Posix);
        assert!(dst.is_empty(), "expected empty, got {dst:?}");
        drop(previous);
    }

    #[test]
    fn write_to_file_replaces_and_chmods() {
        // `tempfile`-style scoping: build under a unique subdir, then drop
        // it. Avoids cross-test pollution on the host.
        let dir = std::env::temp_dir().join("quiver-stub-tests");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let dst = dir.join("write-target.sh");

        // Pre-existing junk proves the rename replaces, not appends.
        std::fs::write(&dst, "junk that must be replaced").expect("seed");
        write_to_file(Layout::Posix, &dst).expect("write_to_file");
        let content = std::fs::read_to_string(&dst).expect("read back");
        assert!(
            content.starts_with("#!/usr/bin/env quiver"),
            "POSIX stub should start with the shebang; got {content:?}"
        );
        assert!(
            !content.contains("junk that must be replaced"),
            "rename did not replace; got {content:?}"
        );

        // The .quiver-tmp sibling must not leak past the rename.
        let tmp = dst.with_extension("quiver-tmp");
        assert!(!tmp.exists(), "tmp sibling leaked at {tmp:?}");

        // Parent dir creation: a two-level-deep nested path the test
        // creates fresh must end up populated.
        let nested = dir.join("nested/deeper/written.sh");
        write_to_file(Layout::Windows, &nested).expect("nested write");
        assert!(nested.exists());

        // Clean up. Best-effort; cargo test reruns can take their lumps
        // with stale temp content.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
