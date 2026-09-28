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
//! * `--stub <windows|posix>` — print the Wox discovery-metadata stub, so the
//!   install hook never needs a checked-in stub file (see [`stub`]).
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

fn main() {
    log::init();

    let argv: Vec<String> = std::env::args().collect();
    // Locate `--stub` and the layout name that may follow it in a single
    // pass over argv. `--stub` is positional — argv[0] is the binary name.
    // `position()` on `[1..]` returns a slice-relative index `i`; the
    // layout arg, when present, sits at argv index `i + 2` (one past the
    // slice element that matched `--stub`). A bare `--stub` yields
    // `Some("")` so the renderer sees the host default.
    let layout_arg = argv[1..]
        .iter()
        .position(|a| a == "--stub")
        .map(|i| argv.get(i + 2).map(String::as_str).unwrap_or(""));
    if layout_arg.is_some() {
        let rendered = stub::render(stub::Layout::from_arg(
            layout_arg.filter(|s| !s.is_empty()),
        ));
        // Mirrors `emit`: write_all + logged failure, never `print!`'s
        // panic on EPIPE/ENOSPC (a closed consumer must not take the
        // process down while the stub is still being written).
        let mut out = std::io::stdout().lock();
        if out.write_all(rendered.as_bytes()).is_err() {
            tracing::error!("failed to write stub to stdout");
        }
        let _ = out.flush();
        return;
    }

    emit(&serve());
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
