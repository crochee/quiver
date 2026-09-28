//! The command catalog — `ShellCommands.json` plus its ranking.
//!
//! Each entry is one "arrow": an alias, the command it looses, and the
//! interpreter/working-directory overrides it carries.

use std::path::PathBuf;

use serde::Deserialize;

use crate::fuzzy;
use crate::platform;

type Value = serde_json::Value;

/// Per-field custom deserializers that keep the catalog's "warn and use the
/// default" semantics instead of `serde`'s default "fail on type mismatch".
///
/// The deserializer-visitor pattern below is standard `serde` boilerplate
/// (the same shape `serde_derive` itself emits); the only twist is the
/// [`or_else`] arm that catches the `deserialize_any` error path and
/// downgrades it to a `tracing::warn!` + the field's default. The
/// alias-context warns the previous hand-rolled `Value` walk could carry
/// are dropped here: serde visits each field through its own deserializer,
/// with no access to its sibling fields. The functional contract — warn
/// loud, use default, keep loading — is unchanged and pinned by the loader
/// tests.
mod de {
    use serde::de::{self, Deserialize, Deserializer, SeqAccess, Visitor};

    /// Deserialize a string field, defaulting to `""` when the JSON value
    /// is missing, null, or of the wrong type.
    pub(super) fn string<'de, D>(d: D) -> Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = String;
            fn expecting(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.write_str("a string")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<String, E> {
                Ok(s.to_string())
            }
            fn visit_borrowed_str<E: de::Error>(
                self,
                s: &'de str,
            ) -> Result<String, E> {
                Ok(s.to_string())
            }
            fn visit_string<E: de::Error>(
                self,
                s: String,
            ) -> Result<String, E> {
                Ok(s)
            }
            fn visit_unit<E: de::Error>(self) -> Result<String, E> {
                Ok(String::new())
            }
            fn visit_none<E: de::Error>(self) -> Result<String, E> {
                Ok(String::new())
            }
        }
        d.deserialize_any(V).or_else(|_| {
            tracing::warn!("catalog field is not a string; using empty");
            Ok(String::new())
        })
    }

    /// Deserialize a bool field with an explicit default. A missing key
    /// uses the default; a present-but-wrong-typed value warns and keeps
    /// the default.
    pub(super) fn bool<'de, D>(d: D, default: bool) -> Result<bool, D::Error>
    where
        D: Deserializer<'de>,
    {
        // The `default` value isn't read by `visit_bool`, only by the
        // `or_else` arm when `deserialize_any` returns a "wrong type"
        // error. Pass it via `deserialize_any` anyway so the field's
        // borrow lifetime stays scoped to the closure, and mark the
        // visitor field with `_` to silence the otherwise-unused warning.
        struct V(#[expect(dead_code)] bool);
        impl<'de> Visitor<'de> for V {
            type Value = bool;
            fn expecting(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.write_str("a bool")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<bool, E> {
                Ok(b)
            }
        }
        d.deserialize_any(V(default)).or_else(|_| {
            tracing::warn!("catalog field is not a bool; keeping default");
            Ok(default)
        })
    }

    /// `silent` / `capture`: default `false` when the key is missing.
    pub(super) fn bool_default_false<'de, D>(d: D) -> Result<bool, D::Error>
    where
        D: Deserializer<'de>,
    {
        bool(d, false)
    }

    /// `enabled`: default `true` when the key is missing.
    pub(super) fn bool_default_true<'de, D>(d: D) -> Result<bool, D::Error>
    where
        D: Deserializer<'de>,
    {
        bool(d, true)
    }

    /// Deserialize the `tags` array: strings kept, non-string elements
    /// warned about and dropped, a non-array value warned about and
    /// treated as no tags.
    pub(super) fn tags<'de, D>(d: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Vec<String>;
            fn expecting(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.write_str("an array of strings")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Vec<String>, A::Error> {
                let mut out = Vec::new();
                while let Some(item) = seq.next_element::<StringWrap>()? {
                    match item.0 {
                        Some(s) => out.push(s),
                        None => {
                            tracing::warn!("non-string tag element dropped")
                        }
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_any(V).or_else(|_| {
            tracing::warn!(
                "catalog field \"tags\" is not an array; using none"
            );
            Ok(Vec::new())
        })
    }

    /// Wrapper around `Option<String>` so the tag-sequence deserializer
    /// can distinguish "absent / null / wrong type" from "present string".
    /// The wrapped [`Option`] is dropped before the wrapper escapes.
    struct StringWrap(Option<String>);

    impl<'de> Deserialize<'de> for StringWrap {
        fn deserialize<D>(d: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = StringWrap;
                fn expecting(
                    &self,
                    f: &mut std::fmt::Formatter<'_>,
                ) -> std::fmt::Result {
                    f.write_str("a string or null")
                }
                fn visit_str<E: de::Error>(
                    self,
                    s: &str,
                ) -> Result<StringWrap, E> {
                    Ok(StringWrap(Some(s.to_string())))
                }
                fn visit_borrowed_str<E: de::Error>(
                    self,
                    s: &'de str,
                ) -> Result<StringWrap, E> {
                    Ok(StringWrap(Some(s.to_string())))
                }
                fn visit_string<E: de::Error>(
                    self,
                    s: String,
                ) -> Result<StringWrap, E> {
                    Ok(StringWrap(Some(s)))
                }
                fn visit_unit<E: de::Error>(self) -> Result<StringWrap, E> {
                    Ok(StringWrap(None))
                }
                fn visit_none<E: de::Error>(self) -> Result<StringWrap, E> {
                    Ok(StringWrap(None))
                }
            }
            d.deserialize_any(V)
        }
    }
}

/// Default value used by `RawCmd::enabled` when the JSON key is absent.
fn enabled_default() -> bool {
    true
}

/// One catalog entry as it appears in `ShellCommands.json`. Every field
/// is optional with a warn-on-wrong-type fallback implemented by the
/// per-field deserializers in [`de`]; [`Cmd`] is the post-processed form
/// after [`load`] has dropped empty-alias, empty-command and duplicate
/// rows.
#[derive(Debug, Deserialize)]
struct RawCmd {
    #[serde(deserialize_with = "de::string")]
    alias: String,
    #[serde(deserialize_with = "de::string")]
    command: String,
    #[serde(default, deserialize_with = "de::string")]
    interpreter: String,
    #[serde(
        rename = "workingDirectory",
        default,
        deserialize_with = "de::string"
    )]
    working_directory: String,
    #[serde(default, deserialize_with = "de::bool_default_false")]
    silent: bool,
    #[serde(default, deserialize_with = "de::bool_default_false")]
    capture: bool,
    #[serde(
        default = "enabled_default",
        deserialize_with = "de::bool_default_true"
    )]
    enabled: bool,
    #[serde(default, deserialize_with = "de::tags")]
    tags: Vec<String>,
    #[serde(default, deserialize_with = "de::string")]
    description: String,
}

#[derive(Debug)]
pub struct Cmd {
    pub alias: String,
    pub command: String,
    pub interpreter: String,
    pub working_directory: String,
    pub silent: bool,
    /// Run the command synchronously during `query` so the launcher shows its
    /// output in the right-hand preview pane and binds Enter to copying the
    /// first non-empty line of stdout to the clipboard (the Wox built-in
    /// `copy-to-clipboard` action). Default `false`: capture commands trade
    /// query latency for feedback, so they opt in explicitly. See
    /// `docs/wox/README.md` §4.8 for why this lives at query time rather than
    /// action time.
    pub capture: bool,
    pub enabled: bool,
    pub tags: Vec<String>,
    /// Human-readable description shown as the result subtitle when present.
    /// Mirrors the Wox shell plugin's saved-command display: the user sees the
    /// description instead of the raw command. Optional; falls back to the
    /// command itself (see [`crate::protocol::item`]).
    pub description: String,
}

#[derive(Debug)]
pub struct Catalog {
    pub commands: Vec<Cmd>,
    pub default_interpreter: String,
    pub default_working_directory: String,
}

/// Default filename Wox looks for inside the user-data directory. Kept as
/// the documented name so a stock Wox install reads the same file the
/// launcher always wrote.
const CATALOG_FILENAME: &str = "ShellCommands.json";

/// Wox exports `WOX_DIRECTORY_USER_DATA`; fall back to the documented default
/// so the binary is also runnable standalone for smoke tests.
///
/// Both the variable spelling and the path shape come from [`crate::platform`]
/// (`PathBuf::push` picks the host's separator), so this module holds no
/// platform knowledge either.
fn user_data_dir() -> PathBuf {
    if let Some(dir) = platform::env_var("WOX_DIRECTORY_USER_DATA") {
        return PathBuf::from(dir);
    }
    let mut path = PathBuf::from(platform::home_dir());
    path.push(".wox");
    path.push("wox-user");
    path
}

/// Resolve the catalog's path. Precedence:
///
/// 1. `QUIVER_PATH` — a full file path. Wins outright, including
///    over `WOX_DIRECTORY_USER_DATA`. Intended for ad-hoc smoke runs and
///    for environments where the catalog lives outside the Wox data
///    directory (a shared NFS home, a per-project check-in, a CI fixture).
/// 2. `WOX_DIRECTORY_USER_DATA` — the Wox-standard directory variable; we
///    append [`CATALOG_FILENAME`] so the lookup stays inside what Wox
///    itself reads.
/// 3. The platform default user-data directory + [`CATALOG_FILENAME`].
///
/// Reading the variable is process-global; tests serialise through
/// [`crate::test_support::env_lock`].
fn path() -> PathBuf {
    if let Some(file) = platform::env_var("QUIVER_PATH") {
        return PathBuf::from(file);
    }
    let mut path = user_data_dir();
    path.push(CATALOG_FILENAME);
    path
}

/// Resolve the catalog's default interpreter: the platform-suffixed key, then
/// the unsuffixed `defaultInterpreter`, then [`platform::DEFAULT_INTERPRETER`].
fn default_interpreter(top: &Value) -> String {
    top.get(platform::CATALOG_INTERPRETER_KEY)
        .or_else(|| top.get("defaultInterpreter"))
        .and_then(Value::as_str)
        .unwrap_or(platform::DEFAULT_INTERPRETER)
        .to_string()
}

/// Resolve the catalog's default working directory: same key lookup as
/// [`default_interpreter`], falling back to empty (no `current_dir`, so the
/// child inherits this process's directory).
///
/// The `@<os>` suffix lets one shared catalog name a value per machine without
/// the others tripping over one that is not there (see
/// [`platform::CATALOG_WORKING_DIRECTORY_KEY`]).
fn default_working_directory(top: &Value) -> String {
    top.get(platform::CATALOG_WORKING_DIRECTORY_KEY)
        .or_else(|| top.get("defaultWorkingDirectory"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Read and parse the catalog. Errors are returned as human-readable strings
/// because the only consumer is a one-line error row in the launcher.
///
/// Malformed *entries* are skipped with a `warn!` naming the reason (empty
/// alias, duplicate alias, missing/empty command); a malformed *file* —
/// unreadable, unparseable, or without a `commands` array — is an error,
/// because every one of those turns the whole catalog silently empty and a
/// launcher that lists nothing is indistinguishable from a typo.
///
/// The walk is two-phase on purpose: [`RawCmd`] deserializes one entry
/// through `serde`, which handles every per-field warn-and-default rule
/// (see [`de`]). The loop then applies the cross-entry rules that derive
/// cannot express — empty alias, missing command, case-insensitive
/// duplicate — and turns each survivor into a [`Cmd`].
pub fn load() -> Result<Catalog, String> {
    let path = path();
    let shown = path.display().to_string();
    tracing::debug!(path = %shown, "catalog_load path");
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        let msg = format!("{shown}: {e}");
        tracing::error!(error = %msg, "catalog read failed");
        msg
    })?;
    let top: Value = serde_json::from_str(&raw).map_err(|e| {
        let msg = format!("{shown}: {e}");
        tracing::error!(error = %msg, "catalog parse failed");
        msg
    })?;

    let arr =
        top.get("commands")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                let msg = format!(
                    "{shown}: no \"commands\" array (a truncated or mis-edited \
                 save would otherwise load as silently empty)"
                );
                tracing::error!(error = %msg, "catalog has no commands array");
                msg
            })?;

    let mut commands = Vec::new();
    let mut seen_aliases: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for value in arr {
        let raw: RawCmd = match serde_json::from_value(value.clone()) {
            Ok(raw) => raw,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "catalog entry skipped: shape does not match any field"
                );
                continue;
            }
        };
        let alias = raw.alias.trim().to_string();
        if alias.is_empty() {
            tracing::warn!(path = %shown, "catalog entry skipped: empty alias");
            continue;
        }
        // Hoist the case-folded form: it is the dedup key, and the same
        // lowercased string is what `score` matches against via
        // `cmd.alias.eq_ignore_ascii_case` — folding once saves one
        // allocation per non-duplicate row.
        let alias_lower = alias.to_ascii_lowercase();
        if !seen_aliases.insert(alias_lower) {
            // Skipped, not merged: two rows with one alias would tie on
            // score *and* on the alias tiebreak, so which one Enter runs
            // would be decided by file order alone — invisible to the user.
            tracing::warn!(
                alias = %alias,
                "catalog duplicate alias (case-insensitive); first wins"
            );
            continue;
        }
        if raw.command.is_empty() {
            tracing::warn!(
                alias = %alias,
                "catalog entry skipped: missing, non-string or empty command"
            );
            continue;
        }
        commands.push(Cmd {
            alias,
            command: raw.command,
            interpreter: raw.interpreter,
            working_directory: raw.working_directory,
            silent: raw.silent,
            capture: raw.capture,
            enabled: raw.enabled,
            tags: raw.tags,
            description: raw.description,
        });
    }

    let catalog = Catalog {
        commands,
        default_interpreter: default_interpreter(&top),
        default_working_directory: default_working_directory(&top),
    };
    tracing::info!(
        count = catalog.commands.len(),
        path = %shown,
        "catalog loaded"
    );
    Ok(catalog)
}

/// A tag hit must never outrank an alias hit. Raw scores are capped first, so
/// this separation is arithmetic rather than an assumption about input length.
const MAX_FUZZY: i64 = 4000;
const TAG_TIER: i64 = 0;
const ALIAS_TIER: i64 = MAX_FUZZY + 1;

/// Score for an empty needle. Any positive value gets the row listed; the order
/// then comes from Wox, which adds its fibonacci-weighted action history on top
/// and falls back to title order. Wox's own matcher returns 0 for an empty
/// pattern, which this wire protocol reads as "no match" — hence 1.
const EMPTY_QUERY_SCORE: i64 = 1;

/// Rank one catalog entry against the needle. `0` means "no match".
///
/// The alias is scored with [`fuzzy::fuzzy_match`] — Wox's own algorithm and
/// constants — which is what makes the ordering match launcher habits: an exact
/// alias beats a prefix, a prefix beats an abbreviation, a contiguous fragment
/// beats a scattered one, and a scattered match below Wox's own threshold is
/// rejected rather than reported weakly.
///
/// Tags use the same algorithm in a strictly lower tier. A tag is a discovery
/// aid, never the thing the user types; Wox's own saved commands match the
/// alias only, so tag matching is already an extension here.
pub fn score(cmd: &Cmd, needle: &str) -> i64 {
    if needle.is_empty() {
        return EMPTY_QUERY_SCORE;
    }

    let alias = fuzzy::fuzzy_match(&cmd.alias, needle);
    if alias.is_match {
        return ALIAS_TIER + alias.score.min(MAX_FUZZY);
    }

    cmd.tags
        .iter()
        .filter_map(|tag| {
            let hit = fuzzy::fuzzy_match(tag, needle);
            hit.is_match.then(|| hit.score.min(MAX_FUZZY))
        })
        .max()
        .map(|best| TAG_TIER + best)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(alias: &str, tags: &[&str]) -> Cmd {
        Cmd {
            alias: alias.to_string(),
            command: String::new(),
            interpreter: String::new(),
            working_directory: String::new(),
            silent: false,
            capture: false,
            enabled: true,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            description: String::new(),
        }
    }

    #[test]
    fn working_directory_prefers_the_platform_key_then_the_plain_one() {
        // The platform key wins when present...
        let both = serde_json::from_str(
            r#"{"defaultWorkingDirectory":"<plain>","defaultWorkingDirectory@linux":"<os>",
                "defaultWorkingDirectory@darwin":"<os>","defaultWorkingDirectory@windows":"<os>"}"#,
        )
        .expect("valid json");
        assert_eq!(default_working_directory(&both), "<os>");

        // ...and the plain key is the fallback for platforms with no override,
        // so a catalog only has to name the platforms that actually differ.
        let plain =
            serde_json::from_str(r#"{"defaultWorkingDirectory":"<plain>"}"#)
                .expect("valid json");
        assert_eq!(default_working_directory(&plain), "<plain>");

        // Neither key: no `current_dir` (inherit), never a bogus empty path.
        let neither = serde_json::from_str(r#"{}"#).expect("valid json");
        assert_eq!(default_working_directory(&neither), "");
    }

    #[test]
    fn ranking_prefers_exact_then_prefix_then_fragment() {
        let entry = cmd("ipconfig", &[]);
        let exact = score(&entry, "ipconfig");
        let prefix = score(&entry, "ipc");
        let fragment = score(&entry, "fig"); // contiguous, mid-word
        assert!(
            exact > prefix && prefix > fragment,
            "exact={exact} prefix={prefix} fragment={fragment}"
        );
    }

    #[test]
    fn abbreviations_match_and_scattered_patterns_do_not() {
        // The launcher habit this replaced a tier table for: `kp` finds
        // `killport` instead of nothing.
        assert!(score(&cmd("killport", &[]), "kp") > 0);
        assert!(score(&cmd("flushdns", &[]), "fd") > 0);
        assert!(score(&cmd("sysinfo", &[]), "si") > 0);
        // …while scattered letters are still refused.
        assert_eq!(score(&cmd("flushdns", &[]), "fdo"), 0);
        assert_eq!(score(&cmd("killport", &[]), "ktp"), 0);
    }

    /// Skim v2 (the matcher behind `fuzzy-matcher`) treats every contiguous
    /// match of equal span the same: `xipx` and `xxip` both score 4036
    /// against `ip`. The hand-rolled port gave the earlier match a tiny
    /// `leading_gap_penalty` advantage; that micro-ranking is gone, and a
    /// tied pair tiebreaks on the alias lexicographic order the launcher
    /// already applies (`then_with(|| a.0.alias.cmp(&b.0.alias))`). This
    /// test pins the new contract: both hit, both at the same score, and
    /// the launcher-level sort decides the visible order.
    #[test]
    fn contiguous_hits_score_equally_and_tiebreak_alphabetically() {
        let early = score(&cmd("xipx", &[]), "ip");
        let late = score(&cmd("xxip", &[]), "ip");
        assert!(early > 0 && late > 0, "both must hit: {early} / {late}");
        assert_eq!(early, late, "equal span → equal score: {early} vs {late}");
    }

    #[test]
    fn an_alias_hit_always_outranks_a_tag_hit() {
        // The alias match here is the weakest kind (contiguous substring), the
        // tag match the strongest (exact) — the tier offset still wins.
        let weak_alias = cmd("ipconfig", &[]);
        let strong_tag = cmd("zzz", &["ipconfig"]);
        assert!(
            score(&weak_alias, "fig") > score(&strong_tag, "ipconfig"),
            "tiers must separate the two"
        );
        assert!(
            score(&strong_tag, "ipconfig") > 0,
            "a tag hit is still a hit"
        );
        assert_eq!(score(&cmd("zzz", &[]), "ipconfig"), 0, "no alias, no tag");
    }

    #[test]
    fn empty_needle_lists_everything_equally() {
        // Ranking is then Wox's business: it adds frecency/action history.
        let score_a = score(&cmd("ip", &[]), "");
        let score_b = score(&cmd("zzz", &[]), "");
        assert!(score_a > 0, "an empty query must still list rows");
        assert_eq!(score_a, score_b);
    }

    /// `path()` precedence — must hold against the whole test suite, because
    /// every other catalog test mutates the env and one stray `QUIVER_PATH`
    /// left from a parallel test would otherwise silently re-point every
    /// `load()` call. The lock + restore guards keep the assertions honest.
    mod path_resolution {
        use super::super::path;
        use crate::test_support::{env_lock, setenv, unsetenv};

        fn assert_path_ends_with(expected: &std::path::Path) {
            let actual = path();
            assert_eq!(
                actual.file_name().and_then(|f| f.to_str()),
                expected.file_name().and_then(|f| f.to_str()),
                "path() = {actual:?}"
            );
            assert_eq!(
                actual.parent(),
                expected.parent(),
                "path() = {actual:?}"
            );
        }

        #[test]
        fn quiver_catalog_path_wins_over_wox_directory_user_data() {
            let _guard = env_lock();
            let restore_data = unsafe {
                setenv("WOX_DIRECTORY_USER_DATA", "/tmp/quiver-test-data")
            };
            let restore_path = unsafe { unsetenv("QUIVER_PATH") };
            assert_path_ends_with(std::path::Path::new(
                "/tmp/quiver-test-data/ShellCommands.json",
            ));

            // Now set QUIVER_PATH — it must override the directory
            // variable entirely (different parent, different name).
            let set_path = unsafe {
                setenv("QUIVER_PATH", "/opt/shared/catalogs/team.json")
            };
            assert_path_ends_with(std::path::Path::new(
                "/opt/shared/catalogs/team.json",
            ));

            drop(set_path);
            // Restore the directory override: dropping `restore_path` would
            // also drop `restore_data` (declaration order matters), so set
            // the file back to empty *before* the directory guard falls.
            drop(restore_path);
            drop(restore_data);
        }

        #[test]
        fn falls_back_to_wox_directory_user_data_when_quiver_path_unset() {
            let _guard = env_lock();
            let restore_data =
                unsafe { setenv("WOX_DIRECTORY_USER_DATA", "/srv/catalogs") };
            let restore_path = unsafe { unsetenv("QUIVER_PATH") };
            assert_path_ends_with(std::path::Path::new(
                "/srv/catalogs/ShellCommands.json",
            ));

            drop(restore_path);
            drop(restore_data);
        }

        #[test]
        fn empty_quiver_catalog_path_is_treated_as_unset() {
            // Empty value must not produce "<empty>/ShellCommands.json"; the
            // directory variable still wins when the file override is blank.
            let _guard = env_lock();
            let restore_data =
                unsafe { setenv("WOX_DIRECTORY_USER_DATA", "/srv/catalogs") };
            let restore_path = unsafe { setenv("QUIVER_PATH", "") };
            assert_path_ends_with(std::path::Path::new(
                "/srv/catalogs/ShellCommands.json",
            ));

            drop(restore_path);
            drop(restore_data);
        }
    }

    /// Loader-level rules, exercised through `load()` with a real temp
    /// catalog: duplicates skipped, dead rows skipped, wrong-typed fields
    /// tolerated loudly, missing `commands` array an error.
    mod loader {
        use super::super::load;
        use crate::test_support::{env_lock, setenv};

        fn write_catalog(json: &str) -> std::path::PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "quiver-catalog-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("ShellCommands.json"), json)
                .expect("write catalog");
            dir
        }

        fn loaded(json: &str) -> super::super::Catalog {
            let dir = write_catalog(json);
            let _env = env_lock();
            let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };
            load().expect("catalog loads")
        }

        #[test]
        fn duplicate_aliases_keep_only_the_first() {
            let catalog = loaded(
                r#"{"commands":[
                    {"alias":"dup","command":"echo first"},
                    {"alias":"DUP","command":"echo second"}
                ]}"#,
            );
            // Two rows with one alias tie on score *and* on the alias
            // tiebreak, so Enter's target would be file order alone —
            // the loader resolves it instead.
            assert_eq!(catalog.commands.len(), 1);
            assert_eq!(catalog.commands[0].command, "echo first");
        }

        #[test]
        fn entries_without_a_command_are_skipped_not_dead() {
            let catalog = loaded(
                r#"{"commands":[
                    {"alias":"miss"},
                    {"alias":"typo","comand":"echo x"},
                    {"alias":"blank","command":""},
                    {"alias":"live","command":"echo ok"}
                ]}"#,
            );
            let aliases: Vec<&str> =
                catalog.commands.iter().map(|c| c.alias.as_str()).collect();
            assert_eq!(aliases, vec!["live"]);
        }

        #[test]
        fn wrong_typed_fields_keep_their_defaults() {
            let catalog = loaded(
                r#"{"commands":[
                    {"alias":"e","command":"echo x",
                     "enabled":"false","silent":"true","capture":1,
                     "tags":"not-an-array","interpreter":7}
                ]}"#,
            );
            let c = &catalog.commands[0];
            // The sharp edge this pins: a string "false" must not invert to
            // enabled=true silently — the default holds and a warn! fired.
            assert!(c.enabled);
            assert!(!c.silent);
            assert!(!c.capture);
            assert!(c.tags.is_empty());
            assert_eq!(c.interpreter, "");
        }

        #[test]
        fn a_catalog_without_commands_is_an_error_not_empty() {
            let dir = write_catalog(r#"{"version":1}"#);
            let _env = env_lock();
            let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };
            let err = load().expect_err("no commands array must error");
            assert!(err.contains("no \"commands\" array"), "{err}");
        }

        #[test]
        fn default_keys_still_resolve() {
            let catalog = loaded(
                r#"{"defaultInterpreter":"bash",
                    "defaultWorkingDirectory":"/tmp",
                    "commands":[{"alias":"x","command":"true"}]}"#,
            );
            assert_eq!(catalog.default_interpreter, "bash");
            assert_eq!(catalog.default_working_directory, "/tmp");
        }
    }
}
