//! JSON-RPC wire types and the query/action handlers.
//!
//! Shapes are fixed by Wox (see `wox.core/plugin/host/host_script.go`):
//! requests arrive as a JSON-RPC object on stdin and responses go out on
//! stdout. There is no schema to validate — Wox fills `method`, `params` and an
//! opaque trace `id`, and ignores anything else — so the request is read
//! field-by-field rather than deserialized into typed structs.
//!
//! The built-in `copy-to-clipboard` action is executed by Wox itself; Wox still
//! calls the plugin afterwards as a hook, so the arm exists and does nothing.
//!
//! `{query}` substitution: Wox's saved-command matching splits the search on
//! the first space — the first token is the alias, the rest is the query
//! argument (`wox.core/plugin/system/shell/shell.go: queryCommands`). For a
//! `{query}`-bearing command line whose alias the typed token is a
//! case-insensitive prefix of, the trailing text is substituted into the
//! command before execution. Commands selected by fuzzy/tag match pass through
//! verbatim, since the user did not explicitly target them.

use crate::StrField;
use crate::catalog;
use crate::identity::{ICON, NAME};
use crate::platform;
use crate::spawn;

use serde::Serialize;

type Value = serde_json::Value;

/// One incoming JSON-RPC request.
pub struct Request {
    pub method: String,
    pub params: Value,
    /// Echoed verbatim in the response envelope. JSON-RPC 2.0 §4 allows
    /// string, number or null; Wox itself always sends a string trace id,
    /// but the smoke harness and hand-run examples use numbers, and
    /// coercing those to `"1"` changes the shape a caller sent.
    pub id: Value,
}

impl Request {
    /// Read a request off the parsed object. Missing fields default to
    /// empty, which keeps `dispatch` a pure function of what actually
    /// arrived.
    pub fn from_value(value: &Value) -> Request {
        Request {
            method: value.str_field("method"),
            params: value.get("params").cloned().unwrap_or(Value::Null),
            id: value.get("id").cloned().unwrap_or(Value::Null),
        }
    }
}

/// Upper bound on returned rows. Wox v2.4.x removed `query.max_result_count`
/// and truncates in the UI, so the plugin caps defensively instead.
const MAX_ROWS: usize = 50;

/// Longest command text shown in a result subtitle before eliding.
const SUBTITLE_LIMIT: usize = 100;

/// JSON-RPC 2.0 success response. Field order is pinned by the wire-format
/// tests (`envelopes_echo_the_id_and_unknown_methods_get_32601`,
/// `action_no_ops_for_missing_data_unknown_ids_and_clipboard`); `serde`
/// emits fields in struct-declaration order.
#[derive(Serialize)]
struct Envelope<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    result: &'a Value,
}

/// JSON-RPC 2.0 error response, sent on an unknown method (the only path
/// that produces one — `main::error_envelope` is the parse-error shape).
#[derive(Serialize)]
struct ErrorResponse<'a> {
    jsonrpc: &'static str,
    id: &'a Value,
    error: ErrorPayload,
}

/// Inner `{code, message}` payload for [`ErrorResponse`].
#[derive(Serialize)]
struct ErrorPayload {
    code: i64,
    message: &'static str,
}

/// Route a request to its handler and wrap the result in a JSON-RPC envelope.
///
/// Wox only ever sends `query` and `action` (`host_script.go`); anything
/// else is a protocol violation and is answered with the JSON-RPC 2.0
/// method-not-found error rather than a fabricated empty result — the two
/// are indistinguishable to a caller that expected data. Notifications
/// (requests without an `id`) and batches are out of scope: Wox sends
/// neither, and answering them as if they were requests matches what the
/// reference host does with unexpected traffic.
pub fn dispatch(req: &Request) -> Value {
    match req.method.as_str() {
        "query" => {
            let result = query(req);
            serde_json::to_value(Envelope {
                jsonrpc: "2.0",
                id: &req.id,
                result: &result,
            })
            .unwrap_or(Value::Null)
        }
        "action" => {
            let result = action(req);
            serde_json::to_value(Envelope {
                jsonrpc: "2.0",
                id: &req.id,
                result: &result,
            })
            .unwrap_or(Value::Null)
        }
        _ => serde_json::to_value(ErrorResponse {
            jsonrpc: "2.0",
            id: &req.id,
            error: ErrorPayload {
                code: -32601,
                message: "method not found",
            },
        })
        .unwrap_or(Value::Null),
    }
}

/// Split a Wox search string into `(alias, rest)` exactly as the Wox shell
/// plugin does (`wox.core/plugin/system/shell/shell.go: queryCommands`):
///
/// ```go
/// search := strings.TrimSpace(query.Search)
/// parts := strings.SplitN(search, " ", 2)
/// ```
///
/// so: trimmed first, then split on the **first single space**. A tab is not a
/// separator, and the remainder keeps its own leading spaces (Wox never trims
/// `queryParam`). Both details are observable — they decide the text that
/// reaches `{query}`.
fn split_search(search: &str) -> (String, String) {
    let trimmed = search.trim();
    match trimmed.find(' ') {
        Some(idx) => {
            (trimmed[..idx].to_string(), trimmed[idx + 1..].to_string())
        }
        None => (trimmed.to_string(), String::new()),
    }
}
/// (`src/index.ts`: `search.substring(shortcut.length).trim().split(" ").filter(Boolean)`):
/// trimmed first, then single spaces, empty fields dropped.
fn split_args(rest: &str) -> Vec<String> {
    rest.trim()
        .split(' ')
        .filter(|field| !field.is_empty())
        .map(str::to_string)
        .collect()
}

/// Apply the Custom Commands plugin's argument placeholders
/// (`src/Utils.ts: substitutePlaceholders`):
///
/// ```js
/// result = result.replace(/\$@/g, args.join(" "))
/// result = result.replace(/\$(\d+)/g, (_, i) => args[parseInt(i, 10) - 1] || "")
/// ```
///
/// `$@` is every argument joined by one space; `$N` is the Nth argument, so
/// `$1` is the first. `$0`, an out-of-range index and an empty argument list
/// all substitute the empty string — the blanking the reference performs on
/// its prefix-match path, where no entry was named and therefore no
/// arguments exist (`executeScript` substitutes with `shortcut.args || []`).
/// The `$N` pass then scans the `$@` pass's *output*, exactly as the
/// reference's second `.replace` runs over the first's result: an argument
/// whose value is itself a `$N` spelling **is** re-substituted once (`k $@`
/// with argument `$1` yields `k $1`, because the substituted value is the
/// text `$1`), and the result of that substitution is not scanned again.
///
/// Substitution is **textual**, not shell-aware: arguments are injected
/// verbatim, which is the reference contract (its README documents
/// `search=$@`). A `$N` meant for an *inner* shell must be written `${N}` —
/// the reference regex only matches digits directly after a `$`, so `${1}` is
/// the spelling that survives (see `ShellCommands.json`'s `pwip`/`pwu`).
fn substitute_arg_placeholders(command: &str, args: &[String]) -> String {
    if !command.contains('$') {
        return command.to_string();
    }
    let after_at = command.replace("$@", &args.join(" "));
    if !after_at.contains('$') {
        return after_at;
    }

    let mut out = String::with_capacity(after_at.len());
    let mut rest = after_at.as_str();
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos + 1..];
        let digits: String =
            tail.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            // Not a placeholder: `$(…)`, `$HOME`, `$_`, a lone `$`.
            out.push('$');
            rest = tail;
            continue;
        }
        let index: usize = digits.parse().unwrap_or(0);
        if let Some(arg) = index.checked_sub(1).and_then(|i| args.get(i)) {
            out.push_str(arg);
        }
        rest = &tail[digits.len()..];
    }
    out.push_str(rest);
    out
}

/// True when `interpreter` is one this build can actually run. Mirrors the
/// table the Wox shell plugin presents (`getInterpreterOptions`): if the
/// catalog names `powershell` on POSIX, the child cannot exist and a
/// `capture: true` entry would only surface a `spawn failed` preview to the
/// user. Treat the entry as non-capturable on this host instead — it still
/// shows as a normal `run` result.
///
/// Only the *other* platform's shells ([`platform::FOREIGN_INTERPRETERS`]) are
/// rejected. A name outside every table — an absolute path to a shebang script,
/// `kubectl`, `git` — is dispatched verbatim by [`spawn::resolve_argv`] and is
/// taken at face value here: reporting a real spawn failure in the preview
/// beats silently not capturing. An empty interpreter is the platform default,
/// which is always in the table.
fn interpreter_available(interpreter: &str) -> bool {
    let lower = interpreter.trim().to_ascii_lowercase();
    !platform::FOREIGN_INTERPRETERS.contains(&lower.as_str())
}

/// `items` envelope returned by `query` — a one-field object holding the
/// ranked result rows. The `query` method never returns `result: null` (the
/// contract is "at least one item"), so the field is non-optional.
#[derive(Serialize)]
struct ItemsEnvelope<'a> {
    items: &'a [Value],
}

/// Error row rendered when the catalog itself fails to load — one
/// actionable row beats an empty list: the user needs to know the catalog
/// is missing, not that nothing matched.
#[derive(Serialize)]
struct CatalogErrorRow<'a> {
    title: String,
    subtitle: &'a str,
    icon: &'static str,
}

fn query(req: &Request) -> Value {
    let needle = req.params.str_field("search").trim().to_string();
    let (alias_token, query_arg) = split_search(&needle);

    tracing::debug!(
        search = %needle,
        alias = %alias_token,
        "query received"
    );

    let catalog = match catalog::load() {
        Ok(catalog) => catalog,
        Err(err) => {
            let title = format!("{NAME}: catalog unavailable");
            let row = CatalogErrorRow {
                title,
                subtitle: &err,
                icon: ICON,
            };
            return serde_json::to_value(ItemsEnvelope {
                items: &[serde_json::to_value(row)
                    .expect("CatalogErrorRow is fixed-shape")],
            })
            .unwrap_or(Value::Null);
        }
    };

    // Score against the alias token, not the full needle, so a trailing query
    // argument does not poison the match (Wox shell plugin splits on the first
    // space and matches the prefix only — `shell.go: queryCommands`).
    let score_against = alias_token.as_str();

    let mut hits: Vec<(&catalog::Cmd, i64)> = catalog
        .commands
        .iter()
        .filter(|c| c.enabled)
        .map(|c| (c, catalog::score(c, score_against)))
        .filter(|(_, score)| *score > 0)
        .collect();
    hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.alias.cmp(&b.0.alias)));
    hits.truncate(MAX_ROWS);

    // `capture` runs the command synchronously at query time, so its gate must
    // be per-entry and strict: it fires only when the typed alias token names
    // *this* entry exactly (ASCII case-insensitive).
    //
    // A looser gate makes the cost unbounded. Measured on the shipped catalog:
    // the four gopass entries (`pwip`/`pwl`/`pwp`/`pwu`) all fuzzy-match a
    // single keystroke `p`, so a non-empty-search gate synchronously ran four
    // `gopass` invocations — **~830 ms per keystroke** — and kept doing it for
    // every keystroke of the family the user was still typing through. Exact
    // match also subsumes the empty-search gate: the launcher sends
    // `search:""` the moment the trigger word is typed, and that token cannot
    // equal any alias, so nothing captures.
    //
    // Cost is therefore O(1) per query, and the preview lands on the entry the
    // user actually named (`qv ip` captures `ip` only, not the `pwip` that also
    // contains `ip`).
    let items: Vec<Value> = hits
        .iter()
        .map(|(cmd, score)| {
            // `named_exactly` is the Custom Commands plugin's exact-match rule
            // (`search === shortcut || search.startsWith(shortcut + " ")`, case
            // insensitive), and it does double duty: it is the `capture` gate
            // *and* the "arguments exist" signal for `$@`/`$N`. The reference
            // extracts arguments by cutting the same prefix
            // (`search.substring(shortcut.length)`), so the two cannot drift.
            let named_exactly = !alias_token.is_empty()
                && cmd.alias.eq_ignore_ascii_case(&alias_token);
            // `{query}` is substituted exactly when Wox's shell plugin
            // substitutes it (`shell.go: queryCommands`): the typed alias token
            // is a case-insensitive prefix of the entry's alias. The trailing
            // text replaces the placeholder as-is — **including the empty
            // text**, which Wox substitutes as `""`; an entry that the user
            // fired with the bare alias therefore runs with an empty argument
            // on both launchers rather than passing a literal `{query}` to the
            // shell.
            //
            // A token-less search prefix-matches every alias (`starts_with("")`
            // is true), which is also what Wox does: its `includeAll` path
            // (`Query`, empty command) substitutes `""` into every listed
            // saved command.
            //
            // Fuzzy/tag matches without a prefix hit pass through verbatim:
            // Wox has no fuzzy path, so there is no upstream behaviour to match
            // and the trailing text was a rank signal, not a query argument.
            let query_for_substitution = if cmd
                .alias
                .to_ascii_lowercase()
                .starts_with(&alias_token.to_ascii_lowercase())
            {
                Some(query_arg.as_str())
            } else {
                None
            };
            item(cmd, *score, &catalog, named_exactly, query_for_substitution)
        })
        .collect();
    serde_json::to_value(ItemsEnvelope { items: &items }).unwrap_or(Value::Null)
}

/// Wox result row — the shape `host_script.go` reads back from a `query`
/// response. `actions` is required; `preview` only appears on the capture
/// path (a single default `copy-to-clipboard` action bound to the captured
/// stdout). Field order matches the previous hand-rolled map so the wire
/// bytes stay byte-stable against the existing test fixtures.
#[derive(Serialize)]
struct ResultRow<'a> {
    title: &'a str,
    subtitle: &'a str,
    score: i64,
    #[serde(rename = "scoreKey")]
    score_key: &'a str,
    icon: &'static str,
    actions: &'a [Value],
    #[serde(skip_serializing_if = "Option::is_none")]
    preview: Option<Value>,
}

/// `data` payload carried by `run` and `run-background` actions. Three
/// fields: the substituted command, the resolved interpreter, and the
/// working directory.
#[derive(Serialize)]
struct RunData<'a> {
    command: &'a str,
    interpreter: &'a str,
    cwd: &'a str,
}

/// Action envelope used for every `run` / `run-background` action. The two
/// optional fields are skipped when not set, so the wire shape stays
/// byte-stable against the previous hand-rolled map (which only inserted
/// them when relevant).
#[derive(Serialize)]
struct RunAction<'a> {
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    #[serde(rename = "isDefault")]
    is_default: bool,
    #[serde(
        rename = "preventHideAfterAction",
        skip_serializing_if = "Option::is_none"
    )]
    prevent_hide_after_action: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hotkey: Option<String>,
    data: RunDataRef<'a>,
}

/// `data` carried by the capture path's `copy-to-clipboard` action: a
/// single string (the first non-empty stdout line). The capture `data`
/// field is a string, not a `RunData` object.
#[derive(Serialize)]
struct CopyCaptureAction<'a> {
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    #[serde(rename = "isDefault")]
    is_default: bool,
    #[serde(
        rename = "preventHideAfterAction",
        skip_serializing_if = "Option::is_none"
    )]
    prevent_hide_after_action: Option<bool>,
    data: &'a str,
}

/// `data` carried by the non-capture path's `copy-to-clipboard` action:
/// the literal command text.
#[derive(Serialize)]
struct CopyCommandAction<'a> {
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    data: &'a str,
}

/// Wrapper that picks between the two `data` shapes (`RunData` object vs.
/// string) without forcing every action to carry both. Lives here so
/// [`RunAction`] stays a struct of static literals + a single `data`
/// field, and the [`Serialize`] impl is what chooses the wire shape.
#[derive(Serialize)]
#[serde(untagged)]
enum RunDataRef<'a> {
    Run(RunData<'a>),
}

#[derive(Serialize)]
#[serde(untagged)]
enum Action<'a> {
    Run(RunAction<'a>),
    CopyCapture(CopyCaptureAction<'a>),
    CopyCommand(CopyCommandAction<'a>),
}

/// Render one catalog entry as a Wox result item.
///
/// `named_exactly` is true only when the typed alias token names this entry
/// exactly — the `capture` gate and the "arguments exist" signal for `$@`/`$N`
/// (see [`query`] for why the gate is strict).
///
/// `query_arg` is `Some` with the trailing text exactly when Wox would
/// substitute `{query}` for this entry; `None` leaves the placeholder
/// untouched. `$@`/`$N` are applied on every path, with an empty argument list
/// whenever the entry was not named exactly, as the Custom Commands plugin
/// does.
fn item(
    cmd: &catalog::Cmd,
    score: i64,
    catalog: &catalog::Catalog,
    named_exactly: bool,
    query_arg: Option<&str>,
) -> Value {
    let mut command = cmd.command.clone();
    // `if let … && …` (a let-chain) is stable on the crate's MSRV (1.91).
    if let Some(arg) = query_arg
        && command.contains("{query}")
    {
        command = command.replace("{query}", arg);
    }
    // Custom Commands' placeholders. Applied to the *executed* text — and so to
    // the subtitle and preview, which show what will run — on every path: the
    // reference plugin substitutes too, with an empty argument list whenever the
    // search did not exactly name an entry.
    let args = if named_exactly {
        split_args(query_arg.unwrap_or(""))
    } else {
        Vec::new()
    };
    let command = substitute_arg_placeholders(&command, &args);

    let working_directory = if cmd.working_directory.is_empty() {
        catalog.default_working_directory.clone()
    } else {
        cmd.working_directory.clone()
    };

    // Subtitle preference, in order:
    //   1. `description` (free-form human prose; what the Wox shell plugin
    //      shows for saved commands, and what the catalog's `$comment`
    //      promises).
    //   2. `command` (always present).
    // The `(cwd: …)` and tag lines follow whichever leads.
    let subtitle_lead = if !cmd.description.is_empty() {
        cmd.description.clone()
    } else {
        elide(&command, SUBTITLE_LIMIT)
    };
    let mut subtitle = subtitle_lead;
    if !working_directory.is_empty() {
        subtitle.push_str("  (cwd: ");
        subtitle.push_str(&working_directory);
        subtitle.push(')');
    }
    if !cmd.tags.is_empty() {
        subtitle.push('\n');
        subtitle.push_str(&cmd.tags.join(", "));
    }

    let interpreter = if cmd.interpreter.is_empty() {
        catalog.default_interpreter.clone()
    } else {
        cmd.interpreter.clone()
    };

    // `capture` is gated on three things: the typed alias token names this
    // entry exactly (the strict gate in [`query`]), the entry's interpreter
    // exists on this host (a `powershell` entry on POSIX would only surface a
    // `spawn failed` preview), and the catalog marked the entry for capture.
    // Otherwise the entry is rendered as a normal `run` result — `silent`
    // semantics still apply.
    let capture_runnable =
        cmd.capture && named_exactly && interpreter_available(&interpreter);

    if capture_runnable {
        // Capture path: run the command synchronously so the launcher can
        // surface its output in the right-hand preview pane and bind Enter to
        // copying the first non-empty stdout line to the clipboard — the query
        // response is the only window Wox renders our output in (see
        // [`spawn::run_detached`]).
        let result =
            spawn::run_capture(&command, &interpreter, &working_directory);
        // Decode once here: the same text feeds the preview body *and* the
        // clipboard, so a stream that needs the platform's code-page fallback
        // (Windows `cmd` output, see [`platform::decode_captured`]) is decoded
        // exactly once and cannot disagree between the two.
        let stdout = platform::decode_captured(&result.stdout);
        let stderr = platform::decode_captured(&result.stderr);
        let preview = build_preview(&result, &stdout, &stderr);
        let clipboard_text = first_nonempty_line(&stdout);

        // Default action: copy. Wox's built-in `copy-to-clipboard`
        // handler writes `data` to the system clipboard synchronously
        // (host_script.go: `clipboard.WriteText`), and `data` must be a
        // *string* for it to be picked up. The preview pane above
        // already showed the full output before the user hit Enter.
        //
        // Same `Silent` mapping as the `run` path (and as Wox's own
        // saved commands): `silent: true` hides the launcher,
        // `silent: false` keeps it open — the user just watched the
        // command run and may want to keep reading the preview.
        let copy_action = Action::CopyCapture(CopyCaptureAction {
            id: "copy-to-clipboard",
            name: "Copy stdout to clipboard",
            icon: "emoji:📋",
            is_default: true,
            prevent_hide_after_action: Some(!cmd.silent),
            data: &clipboard_text,
        });

        let actions = [serde_json::to_value(copy_action)
            .expect("CopyCaptureAction is fixed-shape")];
        let row = ResultRow {
            title: &cmd.alias,
            subtitle: &subtitle,
            score,
            score_key: &cmd.alias,
            icon: ICON,
            actions: &actions,
            preview: Some(preview),
        };
        return serde_json::to_value(row).expect("ResultRow is fixed-shape");
    }

    let mut actions: Vec<Value> = Vec::with_capacity(3);

    // `silent: true` matches the Wox shell plugin's saved-command
    // semantics: run in the background and hide the launcher so it does
    // not stay open over the foreground command's UI.
    // `silent: false` keeps the launcher open (default Wox behavior
    // for non-silent saved commands).
    let run_action = Action::Run(RunAction {
        id: "run",
        name: "Run",
        icon: ICON,
        is_default: true,
        prevent_hide_after_action: Some(!cmd.silent),
        hotkey: None,
        data: RunDataRef::Run(RunData {
            command: &command,
            interpreter: &interpreter,
            cwd: &working_directory,
        }),
    });
    actions.push(
        serde_json::to_value(run_action).expect("RunAction is fixed-shape"),
    );

    if !cmd.silent {
        // Wox's own Shell plugin gives a non-silent saved command *two*
        // gestures: the default action (keep the launcher open —
        // `PreventHideAfterAction: true`) and a second one bound to the primary
        // modifier + Enter that runs in the background and hides it
        // (`PreventHideAfterAction: false`, `util.PrimaryHotkey("enter")`;
        // `shell.go: queryCommands`). A plugin's run is always detached here, so
        // the observable difference is exactly that one bit — same command, the
        // launcher stays or goes.
        let hotkey = format!("{}+enter", platform::PRIMARY_MODIFIER);
        let background_action = Action::Run(RunAction {
            id: "run-background",
            name: "Run and hide",
            icon: ICON,
            is_default: false,
            prevent_hide_after_action: Some(false),
            hotkey: Some(hotkey),
            data: RunDataRef::Run(RunData {
                command: &command,
                interpreter: &interpreter,
                cwd: &working_directory,
            }),
        });
        actions.push(
            serde_json::to_value(background_action)
                .expect("RunAction is fixed-shape"),
        );
    }

    // Executed by Wox; the text comes from the `data` field. Deliberately
    // *without* `preventHideAfterAction`, unlike every other action here: the
    // flag on `run` is the catalog's `silent` contract (what happens to the
    // launcher after the command runs), while copying the command *ends* the
    // interaction — the user's next move is pasting elsewhere, so Wox hiding is
    // the wanted outcome. Wox's own Shell plugin has no copy action to mirror
    // (its secondary actions keep the launcher open because they edit state or
    // open forms).
    let copy_action = Action::CopyCommand(CopyCommandAction {
        id: "copy-to-clipboard",
        name: "Copy command",
        icon: "emoji:📋",
        data: &command,
    });
    actions.push(
        serde_json::to_value(copy_action)
            .expect("CopyCommandAction is fixed-shape"),
    );

    let row = ResultRow {
        title: &cmd.alias,
        subtitle: &subtitle,
        score,
        score_key: &cmd.alias,
        icon: ICON,
        actions: &actions,
        preview: None,
    };
    serde_json::to_value(row).expect("ResultRow is fixed-shape")
}

/// First non-empty line of `text`, trimmed of CR/LF.
///
/// `ipconfig /all`, `Get-NetIPAddress` and similar all emit one IP per line;
/// the convention for `capture: true` aliases is that the first non-empty
/// line is the answer the user wants to copy. An empty string yields an empty
/// string — the clipboard then receives nothing, which is harmless.
///
/// Takes `&str`, not bytes: the caller decodes once with
/// [`platform::decode_captured`] so the preview and the clipboard can never
/// disagree about a byte that needed the code-page fallback.
fn first_nonempty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Trim `text` to `limit` characters, appending `...` when it had to cut.
fn elide(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

/// Wox preview-pane object. Wox v2.4.x swaps the body for a remote preview
/// once it exceeds `previewDataMaxSize` (1024 B), so the body is fetched
/// lazily on demand (`manager.go: shouldWrapRemotePreview`).
#[derive(Serialize)]
struct Preview<'a> {
    #[serde(rename = "type")]
    preview_type: &'static str,
    data: &'a str,
}

/// Render a captured run as a Wox preview object for the detail pane.
///
/// `stdout`/`stderr` arrive already decoded by
/// [`platform::decode_captured`] — the caller decodes once and shares the
/// text with the clipboard. The body is Markdown: a `$ command` header, the
/// exit code, then the output inside a code fence (sized per the CommonMark
/// rule — see `longest_backtick_run`) so indentation and special characters
/// survive verbatim. Both streams are bounded by [`truncate_stream`] — the
/// pane is a glance, and an unbounded `ipconfig /all` would make it scroll
/// for pages.
///
/// The body routinely exceeds Wox's `previewDataMaxSize` (1024 B), at which
/// point Wox swaps it for a `remote` preview the UI fetches on demand
/// (`manager.go: shouldWrapRemotePreview`). That is why the cap is generous
/// rather than tight: the full output is still one lazy fetch away.
fn build_preview(
    result: &spawn::CommandResult,
    stdout: &str,
    stderr: &str,
) -> Value {
    let rc_line = match result.exit_code {
        Some(c) => format!("exit code: {c}"),
        // `None` means the child never ran (spawn failure — the reason is in
        // the stderr block below) or died to a signal. Either way there is no
        // code to print, and claiming one would be a lie.
        None => "exit code: n/a".to_string(),
    };
    let header = format!("$ {}", result.command);
    let stdout = truncate_stream(stdout);
    let stderr = truncate_stream(stderr);
    // CommonMark fence rule: the fence must be strictly longer than any
    // backtick run inside the fenced content, or a ``` in captured output
    // (help text, Markdown notes) closes the block early and the rest
    // renders as live Markdown instead of verbatim output.
    let ticks = 3usize.max(
        longest_backtick_run(&stdout).max(longest_backtick_run(&stderr)) + 1,
    );
    let fence = format!("{}\n", "`".repeat(ticks));
    let close = format!("\n{}`", "`".repeat(ticks));
    let mut body = String::new();
    body.push_str(&header);
    body.push('\n');
    body.push_str(&rc_line);
    body.push('\n');
    body.push('\n');
    body.push_str(&fence);
    body.push_str(&stdout);
    if !stderr.trim().is_empty() {
        body.push_str(&format!("{close}\n\nstderr:\n\n{fence}"));
        body.push_str(&stderr);
    }
    body.push_str(&close);
    body.push('\n');
    serde_json::to_value(Preview {
        preview_type: "markdown",
        data: &body,
    })
    .expect("Preview is fixed-shape")
}

/// Longest run of consecutive backticks in `s` — the length a Markdown code
/// fence around `s` must beat.
fn longest_backtick_run(s: &str) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for b in s.bytes() {
        if b == b'`' {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Bound one captured stream for the preview pane: at most 200 lines **and** at
/// most 64 KB, marking either cut with a trailing `… (truncated)`.
///
/// Two caps, because either one alone is escapable. A line cap alone still lets
/// one pathological line — a minified JSON blob, a base64 dump — ship megabytes
/// into the query response; a byte cap alone would reduce a command with many
/// short lines to a single line. Wox bounds the same surface with
/// `shellOutputSummaryMaxBytes` (64 KB). The byte cut lands on a char boundary,
/// so the body stays valid UTF-8 no matter where the budget runs out.
///
/// This bounds what *leaves* the plugin. The child's output is already fully
/// buffered by `Command::output` before this runs, which is why a `capture`
/// entry must be a quick, bounded lookup (see `docs/catalog-contract.md §5`
/// for the deadline trade-off).
fn truncate_stream(s: &str) -> String {
    const MAX_PREVIEW_LINES: usize = 200;
    const MAX_PREVIEW_BYTES: usize = 64 * 1024;

    let mut out = String::new();
    let mut truncated = false;
    for (index, line) in s.lines().enumerate() {
        if index >= MAX_PREVIEW_LINES {
            truncated = true;
            break;
        }
        if index > 0 {
            out.push('\n');
        }
        let remaining = MAX_PREVIEW_BYTES.saturating_sub(out.len());
        if line.len() > remaining {
            let mut end = remaining;
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&line[..end]);
            truncated = true;
            break;
        }
        out.push_str(line);
    }
    if truncated {
        out.push_str("\n… (truncated)");
    }
    out
}

fn action(req: &Request) -> Value {
    let id = req.params.str_field("id");

    tracing::debug!(action = %id, "action received");

    match id.as_str() {
        // Wox wrote the clipboard before calling us; nothing left to do.
        "copy-to-clipboard" => {}
        // `run-background` is the same execution as `run`; only the action's
        // `preventHideAfterAction` differs (and that bit is Wox's, not ours).
        "run" | "run-background" => {
            let data = req.params.get("data");
            let command =
                data.map(|d| d.str_field("command")).unwrap_or_default();
            if !command.is_empty() {
                let interpreter = data
                    .map(|d| d.str_field("interpreter"))
                    .unwrap_or_default();
                let cwd = data.map(|d| d.str_field("cwd")).unwrap_or_default();
                tracing::info!(
                    action = %id,
                    interpreter = %interpreter,
                    "spawn detached"
                );
                // Detached on purpose: the launcher hides as soon as the action
                // returns, so waiting would only hold the RPC open for no
                // visible gain. Commands whose output matters opt into
                // `capture: true`, which runs at query time so the preview can
                // be rendered.
                spawn::run_detached(&command, &interpreter, &cwd);
            }
        }
        _ => {}
    }

    // `{}` is the canonical empty JSON-RPC 2.0 result object.
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, setenv};

    fn request(text: &str) -> Request {
        Request::from_value(&serde_json::from_str(text).expect("valid request"))
    }

    #[test]
    fn envelopes_echo_the_id_and_unknown_methods_get_32601() {
        let req = request(
            r#"{"jsonrpc":"2.0","method":"nope","params":{},"id":"t-9"}"#,
        );
        assert_eq!(
            serde_json::to_string(&dispatch(&req)).unwrap_or_default(),
            r#"{"jsonrpc":"2.0","id":"t-9","error":{"code":-32601,"message":"method not found"}}"#
        );
    }

    #[test]
    fn numeric_and_absent_ids_are_echoed_verbatim() {
        // JSON-RPC 2.0 §4: the response id keeps the request's type.
        // Wox always sends a string trace id, but the smoke harness sends
        // numbers — coercing those to `"1"` would change the shape a
        // caller can correlate on.
        let numeric = request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":""},"id":1}"#,
        );
        let out =
            serde_json::to_string(&dispatch(&numeric)).unwrap_or_default();
        assert!(out.starts_with(r#"{"jsonrpc":"2.0","id":1,"#), "{out}");

        let notification = request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":""}}"#,
        );
        let out =
            serde_json::to_string(&dispatch(&notification)).unwrap_or_default();
        assert!(out.contains(r#""id":null"#), "{out}");
    }

    #[test]
    fn elides_only_when_over_the_limit() {
        assert_eq!(elide("short", 10), "short");
        assert_eq!(elide("0123456789", 10), "0123456789");
        assert_eq!(elide("01234567890", 10), "0123456...");
    }

    #[test]
    fn preview_streams_are_bounded_by_lines_and_bytes_without_corrupting_utf8()
    {
        // Under both caps: verbatim, no marker, blank lines preserved.
        assert_eq!(truncate_stream("a\n\nb"), "a\n\nb");
        assert_eq!(truncate_stream(""), "");
        assert_eq!(
            truncate_stream("no trailing newline"),
            "no trailing newline"
        );

        // Over the line cap: exactly 200 lines kept, marker appended.
        let many = (0..250)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let cut = truncate_stream(&many);
        assert_eq!(cut.lines().count(), 201, "200 kept lines + the marker");
        assert!(cut.starts_with("0\n1\n"));
        assert!(cut.ends_with("\n… (truncated)"));
        assert!(cut.contains("199"));
        assert!(!cut.contains("200\n"), "line 200 must be cut: {cut:?}");

        // Over the byte cap with a single line: the cut lands on a char
        // boundary, so the body is still valid UTF-8 and never exceeds 64 KB
        // (the marker is appended after the budget, so allow for it).
        let huge = "中".repeat(64 * 1024);
        let cut = truncate_stream(&huge);
        assert!(cut.ends_with("… (truncated)"));
        let payload = cut.trim_end_matches("\n… (truncated)");
        assert!(payload.len() <= 64 * 1024, "payload {} B", payload.len());
        assert!(payload.len() > 64 * 1024 - 4, "payload {} B", payload.len());
        assert_eq!(payload.len() % 3, 0, "cut mid-character: {payload:?}");
    }

    #[test]
    fn query_without_a_catalog_still_returns_one_actionable_row() {
        // Point the plugin at a directory that cannot exist, so the error path
        // (not the empty-result path) is what runs.
        let _env = env_lock();
        let _data = unsafe {
            setenv("WOX_DIRECTORY_USER_DATA", "/nonexistent/quiver-test")
        };
        let req = request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"ip"},"id":"1"}"#,
        );
        let response = dispatch(&req);

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one error row");
        assert!(item.str_field("title").contains("catalog unavailable"));
        assert!(item.str_field("subtitle").contains("ShellCommands.json"));
    }

    #[test]
    fn first_nonempty_line_picks_the_answer_not_the_blanks() {
        // ipconfig /all and Get-NetIPAddress both emit a blank first line on
        // Windows; `first_nonempty_line` must skip it.
        assert_eq!(first_nonempty_line("\n\n192.168.1.5\n"), "192.168.1.5");
        assert_eq!(first_nonempty_line("\r\nfe80::1\r\n"), "fe80::1");
        assert_eq!(first_nonempty_line("10.0.0.1\n10.0.0.2"), "10.0.0.1");
        // Empty text yields empty string; the clipboard then receives
        // nothing, which is harmless.
        assert_eq!(first_nonempty_line(""), "");
        assert_eq!(first_nonempty_line("\n\n\n"), "");
    }

    /// The capture tests drive real child processes through `bash` (or, on
    /// POSIX, the default interpreter which *is* bash). A minimal container
    /// without it would report opaque capture regressions instead of
    /// skipping loudly — the probe distinguishes "cannot test here" from
    /// "broken".
    fn shell_available() -> bool {
        std::process::Command::new("bash")
            .arg("-c")
            .arg("true")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// The action handler is the only path that executes a non-capture
    /// command; these are its first tests. A regression here (a renamed
    /// `data` key, a swapped argument, a broken empty-command guard) used
    /// to pass the entire suite while every plain Enter silently no-oped.
    #[cfg(unix)]
    #[test]
    fn action_run_spawns_the_command_and_answers_an_empty_result() {
        let dir = tempdir();
        let marker = dir.join("ran");
        let req = request(&format!(
            r#"{{"jsonrpc":"2.0","method":"action","params":{{
                "id":"run",
                "data":{{
                    "command":"touch {}",
                    "interpreter":"sh",
                    "cwd":"{}"
                }}
            }},"id":"t"}}"#,
            marker.display(),
            dir.display()
        ));
        let response = dispatch(&req);
        assert_eq!(
            serde_json::to_string(&response).unwrap_or_default(),
            r#"{"jsonrpc":"2.0","id":"t","result":{}}"#,
            "an action answers an empty result object"
        );
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the run action must spawn the command within 5s"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn action_no_ops_for_missing_data_unknown_ids_and_clipboard() {
        // Wox wrote the clipboard before calling us; unknown ids are not
        // errors; a missing `data` object is a no-op, not a panic.
        for params in [
            r#"{"id":"run"}"#,
            r#"{"id":"run","data":{}}"#,
            r#"{"id":"copy-to-clipboard","data":{"command":"touch /nope"}}"#,
            r#"{"id":"definitely-not-an-action"}"#,
            "{}",
        ] {
            let req = request(&format!(
                r#"{{"jsonrpc":"2.0","method":"action","params":{params},"id":"t"}}"#
            ));
            assert_eq!(
                serde_json::to_string(&dispatch(&req)).unwrap_or_default(),
                r#"{"jsonrpc":"2.0","id":"t","result":{}}"#,
                "params {params}"
            );
        }
    }
    #[test]
    fn capture_query_runs_synchronously_and_binds_enter_to_clipboard() {
        if !shell_available() {
            eprintln!("skipping: bash is not on PATH");
            return;
        }

        // Drop a one-shot catalog on disk, point WOX_DIRECTORY_USER_DATA at
        // it, then query "ip" and verify (a) the preview pane carries the
        // command's stdout, (b) the default action is `copy-to-clipboard`,
        // and (c) its `data` is the first non-empty line — i.e. what Wox
        // will write to the system clipboard on Enter.
        let dir = tempdir();
        // `echo` exists in bash, PowerShell and cmd alike, so the test does
        // not pin the platform's default interpreter.
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {
                    "alias": "ip",
                    "command": "echo 192.168.1.5",
                    "capture": true
                }
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let req = request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"ip"},"id":"t"}"#,
        );
        let response = dispatch(&req);

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");

        // Preview pane holds the command output.
        let preview_data = item
            .get("preview")
            .and_then(|p| p.get("data"))
            .and_then(Value::as_str)
            .expect("preview.data string");
        assert!(
            preview_data.contains("192.168.1.5"),
            "preview missing IP: {preview_data}"
        );

        // Default action is the Wox built-in copy-to-clipboard, with the
        // first non-empty stdout line as its data.
        let actions = item
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions");
        let default = actions
            .iter()
            .find(|a| {
                a.get("isDefault").and_then(Value::as_bool).unwrap_or(false)
                    && a.str_field("id") == "copy-to-clipboard"
            })
            .expect("default copy-to-clipboard action");
        assert_eq!(default.str_field("data"), "192.168.1.5");
    }

    #[test]
    fn capture_is_skipped_for_the_empty_query() {
        if !shell_available() {
            eprintln!("skipping: bash is not on PATH");
            return;
        }

        // The launcher sends an empty search as soon as the trigger word is
        // typed, and an empty search matches every alias. Capture commands
        // must not fire there: the marker file proves the child never ran.
        let dir = tempdir();
        let marker = dir.join("ran");
        // `echo x > path` parses in bash, PowerShell and cmd; only the path
        // spelling follows the host, and JSON needs the backslashes escaped.
        let escaped = marker.display().to_string().replace('\\', "\\\\");
        let catalog = format!(
            r#"{{
                "version": 1,
                "defaultWorkingDirectory": "",
                "commands": [{{"alias":"mark","command":"echo ran > {escaped}","capture":true}}]
            }}"#
        );
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let _ = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":""},"id":"t"}"#,
        ));
        assert!(
            !marker.exists(),
            "an empty query must not execute capture commands"
        );

        let matched = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"mark"},"id":"t"}"#,
        ));
        assert!(marker.exists(), "a matched capture entry must execute");

        let item = matched
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        assert!(
            item.get("preview").is_some(),
            "a capture row carries a preview"
        );
    }

    #[test]
    fn capture_fires_only_for_the_exactly_named_alias() {
        if !shell_available() {
            eprintln!("skipping: bash is not on PATH");
            return;
        }

        // Query cost must not scale with the number of capture entries a needle
        // happens to match — the shipped catalog's gopass family is the measured
        // case (see `query`).
        //
        // The observation is the `preview` field, not a marker file: the
        // capture path is the only thing that produces a preview, so
        // `preview.is_some()` is exactly "this entry executed its command at
        // query time". That keeps the assertion independent of host shell
        // path semantics (a marker file written through `bash -lc` lands
        // somewhere different on POSIX than under a Windows-side bash).
        //
        // `bash` is used as the interpreter because it is the one shell both
        // platform tables share, so a missing-interpreter skip can never be
        // what the test is measuring.
        let dir = tempdir();
        let commands: Vec<String> = (0..4)
            .map(|i| {
                format!(
                    r#"{{"alias":"pw{i}","command":"echo probe","interpreter":"bash","capture":true}}"#
                )
            })
            .collect();
        let catalog = format!(
            r#"{{"version":1,"defaultWorkingDirectory":"","commands":[{}]}}"#,
            commands.join(",")
        );
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let titles_with_preview = |response: &Value| -> Vec<String> {
            response
                .get("result")
                .and_then(|r| r.get("items"))
                .and_then(Value::as_array)
                .expect("items")
                .iter()
                .filter(|i| i.get("preview").is_some())
                .map(|i| i.str_field("title"))
                .collect()
        };

        // A needle that fuzzy-matches all four but names none: nothing runs.
        let fuzzy = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"pw"},"id":"t"}"#,
        ));
        assert!(
            fuzzy
                .get("result")
                .and_then(|r| r.get("items"))
                .and_then(Value::as_array)
                .is_some_and(|items| items.len() == 4),
            "all four entries are still listed, they just do not execute"
        );
        assert!(
            titles_with_preview(&fuzzy).is_empty(),
            "a needle that names no entry must not execute any capture command"
        );

        // Naming one entry runs exactly that one and gives it the preview.
        let named = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"pw2"},"id":"t"}"#,
        ));

        assert_eq!(
            titles_with_preview(&named),
            vec!["pw2".to_string()],
            "exactly the named entry carries the captured preview"
        );
        let preview = named
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|i| i.get("preview"))
            .and_then(|p| p.get("data"))
            .and_then(Value::as_str)
            .expect("preview data");
        assert!(
            preview.contains("echo probe"),
            "the preview is the named entry's own output: {preview:?}"
        );
    }

    #[test]
    fn query_placeholder_is_substituted_when_alias_matches() {
        // The Wox shell plugin (`shell.go: queryCommands`) splits the search on
        // the first space and substitutes the rest into `{query}`. The plugin
        // does the same so the catalog can stay portable between Quiver and a
        // Wox-builtin import.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "g", "command": "echo '{query}'", "interpreter": "bash"}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        // Exact alias + trailing text → substituted into `command`.
        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"g hello world"},"id":"t"}"#,
        ));

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        // The substituted command lands in the `run` action's `data.command`.
        let actions = item
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions");
        let run = actions
            .iter()
            .find(|a| a.str_field("id") == "run")
            .expect("run action");
        assert_eq!(
            run.get("data")
                .and_then(|d| d.str_field("command").into())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            "echo 'hello world'"
        );
    }

    #[test]
    fn query_placeholder_is_not_substituted_when_alias_does_not_prefix_match() {
        // Wox's shell plugin substitutes `{query}` only on a strict case-
        // insensitive prefix match (`shell.go: queryCommands`). A typed alias
        // that does not prefix-match (e.g. `dns` against `flushdns`) is a
        // fuzzy/tag hit, not an alias hit — the trailing text is a rank
        // signal, not a query argument.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "flushdns", "command": "echo '{query}'", "interpreter": "bash"}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        // `dns hello` matches `flushdns` only via fuzzy/tag, not as a prefix.
        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"dns hello"},"id":"t"}"#,
        ));

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        let actions = item
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions");
        let run = actions
            .iter()
            .find(|a| a.str_field("id") == "run")
            .expect("run action");
        assert_eq!(
            run.get("data")
                .and_then(|d| d.str_field("command").into())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            "echo '{query}'",
            "non-prefix matches leave the command untouched"
        );
    }

    #[test]
    fn search_splits_on_the_first_single_space_like_the_shell_plugin() {
        // Wox does `strings.SplitN(strings.TrimSpace(search), " ", 2)`: the
        // whole search is trimmed, a tab is not a separator, and the remainder
        // is *not* trimmed (so `{query}` receives its leading spaces).
        assert_eq!(split_search("  greet hi  "), ("greet".into(), "hi".into()));
        assert_eq!(split_search("greet  hi"), ("greet".into(), " hi".into()));
        assert_eq!(split_search("greet"), ("greet".into(), String::new()));
        assert_eq!(split_search("  greet  "), ("greet".into(), String::new()));
        assert_eq!(split_search(""), (String::new(), String::new()));
        // A tab neither separates nor trims: the token is the whole string,
        // exactly as Wox's `SplitN(search, " ", 2)` would see it.
        assert_eq!(
            split_search("greet\thi"),
            ("greet\thi".into(), String::new())
        );
    }

    #[test]
    fn arguments_split_on_single_spaces_and_drop_empties() {
        // Custom Commands: `search.substring(shortcut.length).trim().split(" ").filter(Boolean)`.
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("   "), Vec::<String>::new());
        assert_eq!(
            split_args(" a  b "),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(split_args("one"), vec!["one".to_string()]);
    }

    #[test]
    fn custom_commands_placeholders_expand_like_the_reference() {
        let args = |text: &str| split_args(text);
        // `$@` is every argument joined by one space; `$N` is 1-based.
        assert_eq!(
            substitute_arg_placeholders("kubectl $@", &args("get pods")),
            "kubectl get pods"
        );
        assert_eq!(
            substitute_arg_placeholders(
                "gh repo clone $1 .",
                &args("owner/repo")
            ),
            "gh repo clone owner/repo ."
        );
        assert_eq!(
            substitute_arg_placeholders("a $2 b", &args("one two")),
            "a two b"
        );
        // Out of range, `$0` and an empty argument list all blank the
        // placeholder — the reference's prefix-match path (`shortcut.args || []`).
        assert_eq!(
            substitute_arg_placeholders("a $3 b", &args("one two")),
            "a  b"
        );
        assert_eq!(substitute_arg_placeholders("a $0 b", &args("one")), "a  b");
        assert_eq!(substitute_arg_placeholders("kubectl $@", &[]), "kubectl ");
        assert_eq!(
            substitute_arg_placeholders("gopass rm -f \"$1\"", &[]),
            "gopass rm -f \"\""
        );
        // `$@` expands every occurrence in one pass, matching the
        // reference's global `.replace`.
        assert_eq!(
            substitute_arg_placeholders("$@ $@", &args("a b")),
            "a b a b"
        );
        assert_eq!(
            substitute_arg_placeholders("k $@", &args("$1")),
            "k $1",
            "the $N pass runs over $@'s output, so an argument whose value \
             is a placeholder spelling is substituted once — with itself"
        );
        // Non-placeholder dollars survive untouched: `$(…)`, `$HOME`, `$_`,
        // `awk '{print $1}'`'s cousins are the `${N}` spelling's job (below).
        assert_eq!(
            substitute_arg_placeholders("cd $(pwd) && ls $HOME $_", &args("x")),
            "cd $(pwd) && ls $HOME $_"
        );
        // `${N}` is how an *inner* shell's positional parameter must be spelled:
        // the reference regex only matches digits directly after the `$`.
        assert_eq!(
            substitute_arg_placeholders(
                "sh -c 'gopass show \"${1}\"' _ {}",
                &args("q")
            ),
            "sh -c 'gopass show \"${1}\"' _ {}"
        );
        // A huge index parses as 0 in neither case: it blanks, like `$0`.
        assert_eq!(
            substitute_arg_placeholders("k $99999999999999999999", &args("a")),
            "k "
        );
    }

    #[test]
    fn placeholders_reach_the_run_action_with_the_reference_argument_rules() {
        // One command, three search shapes, covering every combination of the
        // two upstream placeholder families:
        //   `{query}` — Wox's own, substituted on an alias *prefix* match with
        //               the raw trailing text (`shell.go: queryCommands`);
        //   `$@`/`$N`  — Custom Commands', substituted on every path, but only
        //               given arguments when the search exactly names the entry
        //               (`index.ts` exact match ⇒ `args`; prefix ⇒ `[]`).
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "greet", "command": "echo [{query}][$1][$@]", "interpreter": "bash"}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let search = |needle: &str| {
            dispatch(&request(&format!(
                r#"{{"jsonrpc":"2.0","method":"query","params":{{"search":"{needle}"}},"id":"t"}}"#
            )))
        };

        // Exactly named + trailing text: both families see the argument.
        assert_eq!(
            first_run_command(&search("greet hi there")),
            "echo [hi there][hi][hi there]"
        );
        // Exactly named, no argument: both substitute the empty string (Wox
        // does `ReplaceAll(…, "{query}", "")`; the reference does `args || []`).
        assert_eq!(first_run_command(&search("greet")), "echo [][][]");
        // Prefix match but *not* the entry's own name: `{query}` still sees the
        // trailing text, while `$@`/`$N` are blanked — the reference only hands
        // arguments to an exact match.
        assert_eq!(first_run_command(&search("gree x")), "echo [x][][]");
        assert_eq!(first_run_command(&search("gree")), "echo [][][]");
        // Fuzzy-only hit (no prefix: `get` is not a prefix of `greet`):
        // `{query}` is left alone, exactly as before this change — Wox has no
        // fuzzy path to match.
        assert_eq!(first_run_command(&search("get z")), "echo [{query}][][]");
    }

    #[test]
    fn capture_runs_the_substituted_command_and_copies_its_first_line() {
        if !shell_available() {
            eprintln!("skipping: bash is not on PATH");
            return;
        }

        // `capture` executes the *substituted* text — the preview must show what
        // really ran, and the clipboard must receive its first non-empty line.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "echoargs", "command": "echo [$@]", "interpreter": "bash", "capture": true}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"echoargs one two"},"id":"t"}"#,
        ));

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        let preview = item
            .get("preview")
            .and_then(|p| p.get("data"))
            .and_then(Value::as_str)
            .expect("preview data");
        assert!(
            preview.contains("$ echo [one two]"),
            "preview shows the substituted command: {preview:?}"
        );
        let default = item
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions")
            .iter()
            .find(|a| {
                a.get("isDefault").and_then(Value::as_bool).unwrap_or(false)
            })
            .expect("default action");
        assert_eq!(default.str_field("id"), "copy-to-clipboard");
        assert_eq!(default.str_field("data"), "[one two]");
    }

    #[test]
    fn a_capture_entry_keeps_the_launcher_open_unless_it_is_silent() {
        if !shell_available() {
            eprintln!("skipping: bash is not on PATH");
            return;
        }

        // `silent` is the Wox saved-command contract and applies to the capture
        // path too: `Silent: false` ⇒ `PreventHideAfterAction: true`, `Silent:
        // true` ⇒ false (`shell.go: queryCommands`, both action sets).
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "loud", "command": "echo loud", "interpreter": "bash", "capture": true},
                {"alias": "quiet", "command": "echo quiet", "interpreter": "bash", "capture": true, "silent": true}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let search = |needle: &str| {
            dispatch(&request(&format!(
                r#"{{"jsonrpc":"2.0","method":"query","params":{{"search":"{needle}"}},"id":"t"}}"#
            )))
        };
        let loud = search("loud");
        let quiet = search("quiet");

        let prevent_hide = |response: &Value| -> bool {
            response
                .get("result")
                .and_then(|r| r.get("items"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .expect("one item")
                .get("actions")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|a| a.get("preventHideAfterAction"))
                .and_then(Value::as_bool)
                .expect("preventHideAfterAction bool")
        };
        assert!(
            prevent_hide(&loud),
            "a non-silent capture entry keeps the launcher open"
        );
        assert!(
            !prevent_hide(&quiet),
            "a silent capture entry hides the launcher"
        );
    }

    #[test]
    fn a_non_silent_entry_offers_the_primary_modifier_and_enter_gesture() {
        // Wox's own Shell plugin publishes two actions for a non-silent saved
        // command: the default `execute` (PreventHideAfterAction: true) and
        // `execute_background` bound to `util.PrimaryHotkey("enter")`
        // (PreventHideAfterAction: false). Mirror both, so the same two
        // gestures exist here; a silent entry has only the hiding one.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "loud", "command": "echo loud", "interpreter": "bash"},
                {"alias": "quiet", "command": "echo quiet", "interpreter": "bash", "silent": true}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let search = |needle: &str| {
            dispatch(&request(&format!(
                r#"{{"jsonrpc":"2.0","method":"query","params":{{"search":"{needle}"}},"id":"t"}}"#
            )))
        };
        let loud = search("loud");
        let quiet = search("quiet");

        let actions = |response: &Value| -> Vec<(String, bool, String)> {
            response
                .get("result")
                .and_then(|r| r.get("items"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .expect("one item")
                .get("actions")
                .and_then(Value::as_array)
                .expect("actions")
                .iter()
                .map(|a| {
                    (
                        a.str_field("id"),
                        a.get("preventHideAfterAction")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        a.str_field("hotkey"),
                    )
                })
                .collect()
        };

        let expected_hotkey =
            format!("{}+enter", crate::platform::PRIMARY_MODIFIER);
        assert_eq!(
            actions(&loud),
            vec![
                ("run".to_string(), true, String::new()),
                ("run-background".to_string(), false, expected_hotkey),
                ("copy-to-clipboard".to_string(), false, String::new()),
            ],
            "a non-silent entry carries both gestures"
        );
        assert_eq!(
            actions(&quiet),
            vec![
                ("run".to_string(), false, String::new()),
                ("copy-to-clipboard".to_string(), false, String::new()),
            ],
            "a silent entry only needs the hiding one"
        );
    }

    /// Extract the `run` action's `data.command` for the first returned item.
    fn first_run_command(response: &Value) -> String {
        response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item")
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions")
            .iter()
            .find(|a| a.str_field("id") == "run")
            .expect("run action")
            .get("data")
            .map(|d| d.str_field("command"))
            .unwrap_or_default()
    }

    #[test]
    fn query_placeholder_is_substituted_with_an_empty_string_when_no_argument_is_typed()
     {
        // Wox substitutes `{query}` on any alias prefix match, with whatever
        // follows the alias — including nothing (`shell.go: queryCommands`
        // does `strings.ReplaceAll(cmd.Command, "{query}", queryParam)` where
        // `queryParam` is `""` when the search has no space). Leaving the
        // literal `{query}` in place would hand the shell a different command
        // than the launcher the same catalog is importable into.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "greet", "command": "echo {query}", "interpreter": "bash"}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        // Bare alias: prefix match, empty argument.
        let bare = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"greet"},"id":"t"}"#,
        ));
        // Empty search (what the launcher sends the moment the trigger word is
        // typed): Wox's `includeAll` path substitutes `""` into every row.
        let empty = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":""},"id":"t"}"#,
        ));

        assert_eq!(
            first_run_command(&bare),
            "echo ",
            "a prefix match with no argument substitutes an empty string"
        );
        assert_eq!(
            first_run_command(&empty),
            "echo ",
            "the empty search substitutes an empty string too"
        );
    }

    #[test]
    fn description_renders_as_subtitle_when_present() {
        // Mirrors Wox's shell plugin: when a saved command has a description,
        // it is shown instead of the raw command. The `(cwd: …)` suffix and
        // the tag line are appended to whichever lead is used.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {
                    "alias": "pwp",
                    "command": "gopass show -o {query}",
                    "interpreter": "bash",
                    "description": "Gopass: pick entry, copy password (first line) to clipboard.",
                    "tags": ["pm", "gopass"]
                }
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"pwp"},"id":"t"}"#,
        ));

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        let subtitle = item.str_field("subtitle");
        assert!(
            subtitle.starts_with("Gopass: pick entry"),
            "description leads, not the command: {subtitle:?}"
        );
        assert!(
            subtitle.contains("pm, gopass"),
            "tag line still appended: {subtitle:?}"
        );
        assert!(
            !subtitle.contains("gopass show"),
            "raw command must not appear when description is set"
        );
    }

    #[test]
    fn silent_true_hides_the_launcher_after_run() {
        // Matches Wox's saved-command spec (`shell.go: queryCommands`):
        //   Silent: true → execute_background, PreventHideAfterAction: false
        // The catalog has 11 entries that rely on this (taskmgr, sysmon, gh,
        // flushdns, …) — they all expect the launcher to disappear.
        let dir = tempdir();
        let catalog = r#"{
            "version": 1,
            "defaultWorkingDirectory": "",
            "commands": [
                {"alias": "loud", "command": "echo loud", "interpreter": "bash", "silent": false},
                {"alias": "quiet", "command": "echo quiet", "interpreter": "bash", "silent": true}
            ]
        }"#;
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":""},"id":"t"}"#,
        ));

        for item in response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .expect("items")
        {
            let prevent = item
                .get("actions")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|a| a.get("preventHideAfterAction"))
                .and_then(Value::as_bool)
                .expect("preventHideAfterAction bool");
            match item.str_field("title").as_str() {
                "loud" => {
                    assert!(prevent, "non-silent must keep launcher open")
                }
                "quiet" => assert!(!prevent, "silent must hide launcher"),
                other => panic!("unexpected alias {other}"),
            }
        }
    }

    #[test]
    fn capture_skips_when_the_interpreter_is_the_other_platforms_shell() {
        // An entry whose `interpreter` exists only on the *other* platform must
        // surface as a normal `run` result, not a `spawn failed` preview. The
        // captured default action (`copy-to-clipboard` with `data == ""`) is
        // replaced by the run-path default (`run` action + secondary `Copy
        // command` carrying the literal command text).
        //
        // The name comes from `platform::FOREIGN_INTERPRETERS` so the test means
        // the same thing on Windows (`sh`/`zsh`) and on POSIX
        // (`powershell`/`cmd`) — the production path under test is the same:
        // an interpreter this build cannot run at all.
        let dir = tempdir();
        let foreign = crate::platform::FOREIGN_SHELL;
        let catalog = format!(
            r#"{{
                "version": 1,
                "defaultWorkingDirectory": "",
                "commands": [
                    {{"alias": "ps", "command": "Get-NetIPAddress", "interpreter": "{foreign}", "capture": true}}
                ]
            }}"#
        );
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"ps"},"id":"t"}"#,
        ));

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        assert!(
            item.get("preview").is_none(),
            "no preview when capture is skipped"
        );
        let actions = item
            .get("actions")
            .and_then(Value::as_array)
            .expect("actions");
        let default = actions
            .iter()
            .find(|a| {
                a.get("isDefault").and_then(Value::as_bool).unwrap_or(false)
            })
            .expect("default action");
        assert_eq!(
            default.str_field("id"),
            "run",
            "default action flips from copy-to-clipboard to run when capture is skipped"
        );
        // The secondary `copy-to-clipboard` (Copy command) keeps the raw text,
        // not the empty clipboard payload the capture path would emit.
        let copy = actions
            .iter()
            .find(|a| a.str_field("id") == "copy-to-clipboard")
            .expect("secondary copy-to-clipboard");
        assert_eq!(copy.str_field("data"), "Get-NetIPAddress");
    }

    #[cfg(unix)]
    #[test]
    fn capture_runs_through_an_absolute_script_interpreter() {
        // The other half of the gate: an interpreter that is *not* an
        // interpreter name at all — an absolute path to a shebang script — is
        // dispatched verbatim by `spawn::resolve_argv` and must therefore still
        // capture. This is the docs' third minimal catalog form. Gated to
        // POSIX: Windows has no kernel shebang dispatch for a `#!` file.
        let dir = tempdir();
        let script = dir.join("echoer");
        std::fs::write(&script, "#!/bin/sh\necho \"$1\"\n")
            .expect("write script");
        std::fs::set_permissions(
            &script,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("chmod");

        let catalog = format!(
            r#"{{
                "version": 1,
                "defaultWorkingDirectory": "",
                "commands": [
                    {{"alias": "sheb", "command": "alpha", "interpreter": "{}", "capture": true}}
                ]
            }}"#,
            script.display()
        );
        std::fs::write(dir.join("ShellCommands.json"), catalog)
            .expect("write catalog");
        let _env = env_lock();
        let _data = unsafe { setenv("WOX_DIRECTORY_USER_DATA", &dir) };

        let response = dispatch(&request(
            r#"{"jsonrpc":"2.0","method":"query","params":{"search":"sheb"},"id":"t"}"#,
        ));

        let item = response
            .get("result")
            .and_then(|r| r.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("one item");
        let preview = item
            .get("preview")
            .and_then(|p| p.get("data"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                panic!(
                    "capture must run through a script interpreter: {item:?}"
                )
            });
        assert!(
            preview.contains("alpha"),
            "the script's own argv reaches it: {preview:?}"
        );
    }

    /// Create a unique temp directory the test owns. Rust runs tests in
    /// parallel, so the path is unique per process (pid + a monotonic counter)
    /// rather than per test name.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir()
            .join(format!("quiver-protocol-test-{pid}-{n}"));
        std::fs::create_dir_all(&path).expect("create tempdir");
        path
    }
}
