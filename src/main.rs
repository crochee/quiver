//! Quiver — a quiver of shell one-liners, as a native Wox script plugin.
//!
//! Every alias in `ShellCommands.json` is an arrow: nock it with a keyword,
//! loose it with Enter. Zero runtime dependencies — Wox's own Go process
//! executes this binary directly; no Python, no Node.js.
//!
//! # Modes
//!
//! * default — read one JSON-RPC request on stdin, write the response on
//!   stdout (see [`protocol`]).
//! * `stub [--path [<path>]]` — render / write the Wox discovery-metadata
//!   stub for this host. The stub flavour is compile-time (`Layout::Windows`
//!   on Windows, `Layout::Posix` on Linux / macOS); no layout parameter is
//!   needed. The `--path` flag flips the destination to a file (mkdir -p,
//!   chmod 0o755, atomic rename — no shell glue):
//!
//!   | invocation                  | effect                                       |
//!   |-----------------------------|----------------------------------------------|
//!   | `quiver stub`               | render host's layout to stdout               |
//!   | `quiver stub --path`        | write host's layout to the platform's        |
//!   |                             | default destination (`~/.wox/wox-user/...`)  |
//!   | `quiver stub --path <p>`    | write host's layout to `<p>`                 |
//!
//! # Module map
//!
//! | module      | responsibility                                    |
//! |-------------|---------------------------------------------------|
//! | [`identity`]| plugin id / name / icon / triggers                |
//! | [`catalog`] | `ShellCommands.json` loading and ranking          |
//! | [`fuzzy`]   | Wox's fuzzy-match scoring                         |
//! | [`protocol`]| JSON-RPC wire types and query/action handlers     |
//! | [`platform`]| per-OS knowledge, resolved at compile time        |
//! | [`spawn`]   | interpreter dispatch and detached execution       |
//! | [`stub`]    | discovery-metadata rendering                       |

mod catalog;
mod fuzzy;
mod identity;
mod log;
mod platform;
mod protocol;
mod spawn;
mod stub;

#[cfg(test)]
mod test_support;

use std::io::{Read, Write};

use clap::{ArgAction, Parser, Subcommand};
use serde::Serialize;

/// Trait extension on [`serde_json::Value`] for the one field-shape the
/// crate reads everywhere: `String`, `""` on absent or wrong type. Owned
/// rather than `&str` because callers build on it (trim, format, default).
pub trait StrField {
    fn str_field(&self, key: &str) -> String;
}

impl StrField for serde_json::Value {
    fn str_field(&self, key: &str) -> String {
        self.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    }
}

/// Top-level CLI.
///
/// `--version` / `-V` is a global flag (`disable_version_flag = true`
/// suppresses clap's auto-injected version handler so we can print the
/// custom `Quiver <version> <commit> (<profile> <timestamp>)` line that
/// the smoke harness parses). `stub` is the only subcommand today; the
/// default (no args) dispatches to the JSON-RPC serve loop.
#[derive(Parser)]
#[command(
    name = identity::NAME,
    version, // clap reads `version` from `Cargo.toml`, but we print our own
    disable_version_flag = true,
    long_about = None,
    about = "Native Wox script plugin: JSON-RPC bridge to a catalog of shell one-liners."
)]
struct Cli {
    /// Print version and exit
    #[arg(short = 'V', long = "version", action = ArgAction::SetTrue)]
    version: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Available subcommands. Today there is only `stub`; the enum exists so
/// future subcommands (`config`, `validate`, …) are a one-line addition.
#[derive(Subcommand)]
enum Command {
    /// Render / write the Wox discovery-metadata stub.
    ///
    /// The stub flavour is compile-time (`Layout::Windows` on Windows,
    /// `Layout::Posix` on Linux / macOS); `--path` controls the destination.
    #[command(long_about = None)]
    Stub {
        /// Write the stub to a file (omit `<PATH>` for the platform default)
        #[arg(
            long = "path",
            num_args = 0..=1,
            default_missing_value = "",
            require_equals = false
        )]
        path: Option<String>,
    },
}

/// Sentinel `""` is clap's `default_missing_value` — it distinguishes
/// `--path` *with no value* (use the platform default destination) from
/// `--path <p>` (use `<p>`).
const PATH_NO_VALUE: &str = "";

fn main() {
    log::init();

    let cli = Cli::parse();

    // `--version` short-circuits before any I/O (the smoke harness runs
    // `--version` as its first probe, before catalog / WOX_DIRECTORY_USER_DATA
    // are even checked).
    if cli.version {
        println!("{} {}", identity::NAME, identity::full_version());
        return;
    }

    // The host's compile-time stub flavour.
    let layout = if crate::platform::IS_WINDOWS {
        stub::Layout::Windows
    } else {
        stub::Layout::Posix
    };

    match cli.command {
        Some(Command::Stub { path }) => {
            run_stub(layout, path.as_deref());
        }
        None => {
            // No subcommand: JSON-RPC serve over stdin / stdout.
            emit(&serve());
        }
    }
}

/// Render or write the stub per `--path`'s three terminal states.
fn run_stub(layout: stub::Layout, path: Option<&str>) {
    match path {
        // `--path <p>`: explicit destination.
        Some(p) if p != PATH_NO_VALUE => {
            write_stub(layout, std::path::Path::new(p));
        }
        // `--path` without a value: platform default destination; fall
        // back to stdout if HOME / USERPROFILE are missing.
        Some(PATH_NO_VALUE) => {
            let dst = stub::default_destination(layout);
            if dst.is_empty() {
                tracing::warn!(
                    layout = ?layout,
                    "no home variable set; falling back to stdout"
                );
                write_stdout(layout);
            } else {
                write_stub(layout, std::path::Path::new(&dst));
            }
        }
        // No `--path`: stdout.
        None => write_stdout(layout),
        // Clap can't surface a non-empty value that's *also* `PATH_NO_VALUE`
        // (it'd have been matched by `Some(p) if p != PATH_NO_VALUE` first),
        // but the exhaustive-match checker still wants a wildcard arm.
        Some(_) => unreachable!(
            "clap yields only `Some(<non-empty>)` / `Some(\"\")` / `None`"
        ),
    }
}

/// Write the rendered stub to `<path>`, logging the result and exiting
/// non-zero on I/O failure. The path-specific behaviours (`mkdir -p`,
/// `chmod 0o755`, atomic rename) live in [`stub::write_to_file`].
fn write_stub(layout: stub::Layout, path: &std::path::Path) {
    match stub::write_to_file(layout, path) {
        Ok(()) => tracing::info!(
            layout = ?layout,
            path = %path.display(),
            "stub written"
        ),
        Err(err) => {
            tracing::error!(
                layout = ?layout,
                path = %path.display(),
                error = %err,
                "failed to write stub"
            );
            std::process::exit(1);
        }
    }
}

/// Write the rendered stub to stdout. Mirrors `emit`: `write_all` +
/// logged failure, never `print!`'s panic on EPIPE/ENOSPC (a closed
/// consumer must not take the process down while the stub is still being
/// written).
fn write_stdout(layout: stub::Layout) {
    let rendered = stub::render(layout);
    let mut out = std::io::stdout().lock();
    if out.write_all(rendered.as_bytes()).is_err() {
        tracing::error!("failed to write stub to stdout");
    }
    let _ = out.flush();
}

/// Read one JSON-RPC request from stdin and produce its response.
fn serve() -> serde_json::Value {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        tracing::error!("stdin read failed");
        // An I/O failure is not a parse failure: JSON-RPC 2.0 §5.1 reserves
        // -32700 for invalid JSON and -32603 for internal errors.
        return error_envelope(-32603, "stdin read failed");
    }

    match serde_json::from_str::<serde_json::Value>(&buf) {
        Ok(value) => protocol::dispatch(&protocol::Request::from_value(&value)),
        Err(err) => {
            tracing::error!(error = %err, "json parse failed");
            parse_error(&err.to_string())
        }
    }
}

/// JSON-RPC 2.0 error envelope — code + message — sent inside the outer
/// envelope below. Field order is pinned by the wire-format tests in
/// `protocol::tests`; `serde` emits fields in struct-declaration order.
#[derive(Serialize)]
struct ErrorBody<'a> {
    code: i64,
    message: &'a str,
}

/// JSON-RPC 2.0 error response. The `id` is `null` (the spec's spelling for
/// "could not be detected"), because the request never got far enough to
/// carry one.
#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    jsonrpc: &'static str,
    id: Option<serde_json::Value>,
    error: ErrorBody<'a>,
}

/// JSON-RPC parse error, which Wox surfaces as a notification.
fn parse_error(message: &str) -> serde_json::Value {
    error_envelope(-32700, message)
}

/// A JSON-RPC error envelope for a request whose id is unknown.
fn error_envelope(code: i64, message: &str) -> serde_json::Value {
    serde_json::to_value(ErrorEnvelope {
        jsonrpc: "2.0",
        id: None,
        error: ErrorBody { code, message },
    })
    .unwrap_or(serde_json::Value::Null)
}

fn emit(value: &serde_json::Value) {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    if out.write_all(encoded.as_bytes()).is_err() {
        tracing::error!("failed to write response to stdout");
    }
    let _ = out.flush();
}
