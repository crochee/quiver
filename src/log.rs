//! Tracing dispatch — wires `tracing`'s macro layer (`info!`/`warn!`/`…`)
//! to a `tracing-subscriber` that writes structured single-line records to
//! stderr, gated by `QUIVER_LOG` (with `RUST_LOG` as a fallback for tools
//! that already set it).
//!
//! Quiver is a one-shot script plugin — one process per Wox query, lifetime
//! ≤ `WOX_SCRIPT_EXECUTION_TIMEOUT` (10 s default). The plugin runs in
//! Wox's own Go process and is hot-loaded by fsnotify; this module is the
//! only place that touches a `Subscriber`, so the rest of the crate just
//! writes `tracing::info!("query received", search = …)` and stays out of
//! the way.
//!
//! # Why the standard `EnvFilter`
//!
//! `tracing` is what every other tool in this workstation expects —
//! `RUST_LOG` semantics, span context for future Wox-side correlation,
//! structured key-value fields an OpenTelemetry exporter can consume
//! unchanged — and `tracing-subscriber`'s `EnvFilter` is the boring,
//! battle-tested directive parser for it: `info`, `quiver=debug,info`,
//! target prefixes, the lot.
//!
//! One floor is added on top ([`resolve_filter`]): every composed
//! directive list is *prefixed* with `error,`. `EnvFilter` reads a bare
//! word as a **target** directive, so a typo'd `QUIVER_LOG=infoo` used to
//! match only a nonexistent target and silence even `tracing::error!` —
//! exactly when debugging was most wanted. Prepending the floor keeps the
//! user's directive winning for the global scope (a later global directive
//! replaces an earlier one) while unmatched targets still emit errors; a
//! genuinely unparseable whole string falls back to plain `error`.
//!
//! # Defaults
//!
//! * `QUIVER_LOG` unset **and** `RUST_LOG` unset → `error` baseline. Wox
//!   captures stderr per-plugin; a healthy install emits only on real
//!   errors.
//! * `QUIVER_LOG` wins over `RUST_LOG` when both are set — the user-facing
//!   knob is the documented one.
//! * ANSI colour only when stderr is a tty; Wox pipes stderr into a log
//!   file, and the plain form is what `grep` expects there.

use std::sync::LazyLock;

use tracing_subscriber::Layer;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The baseline prepended to every user directive — the "errors always
/// loud" contract from the README.
const ERROR_FLOOR: &str = "error";

/// Initialise the global tracing subscriber. Idempotent — second and
/// later calls are a no-op (the underlying `try_init` swallows the
/// "already installed" error), so tests and the `stub` subcommand can
/// call it freely without panicking.
pub fn init() {
    static INIT: LazyLock<()> = LazyLock::new(|| {
        let filter = resolve_filter();
        let writer = BoxMakeWriter::new(std::io::stderr);

        // The format is intentionally compact:
        //
        //   `<RFC3339-ish>  LEVEL  message key=value key=value\n`
        //
        // `with_target(false)` drops `quiver::catalog` etc. — we are a
        // single crate with a few modules, the redundancy outweighs the
        // signal. `with_level(true)` keeps the level tag (the smoke
        // harness greps on it). `compact()` collapses the timestamp to
        // one short field.
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_target(false)
            .with_level(true)
            .compact()
            .with_ansi(supports_ansi())
            .with_filter(filter);

        // `try_init` returns Err if a subscriber is already installed — the
        // right call here, since `init` must be idempotent.
        let _ = tracing_subscriber::registry().with(layer).try_init();
    });
    LazyLock::force(&INIT);
}

/// Build the env filter. `QUIVER_LOG` wins over `RUST_LOG`; with neither
/// set, the binary uses an `error` baseline so `tracing::error!` still
/// leaks (the "errors always loud" contract from the README).
///
/// The user's directive is composed as `error,<src>` — **prepended**, never
/// appended: a later *global* directive replaces an earlier one, so
/// `info,error` would suppress INFO while `error,info` keeps it. See the
/// module docs for why the floor exists at all.
fn resolve_filter() -> EnvFilter {
    let from_quiver =
        std::env::var("QUIVER_LOG").ok().filter(|v| !v.is_empty());
    let from_rust = std::env::var("RUST_LOG").ok().filter(|v| !v.is_empty());

    let src = from_quiver
        .as_deref()
        .or(from_rust.as_deref())
        .unwrap_or(ERROR_FLOOR);

    compose_filter(src)
}

/// `error,<src>` when `src` parses, plain `error` when it does not. Split
/// out so the composition rule is testable without touching the process
/// environment.
fn compose_filter(src: &str) -> EnvFilter {
    EnvFilter::try_new(format!("{ERROR_FLOOR},{src}"))
        .unwrap_or_else(|_| EnvFilter::new(ERROR_FLOOR))
}

/// Probe whether stderr is a tty; if not (which is the common case — Wox
/// captures the plugin's stderr into a file), skip ANSI escape codes.
fn supports_ansi() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stderr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_floor_does_not_override_the_user_directive() {
        // `error,info` keeps INFO (a later *global* directive replaces the
        // earlier one); the floor is a minimum, never a ceiling. Appending
        // instead (`info,error`) would suppress INFO — that is the ordering
        // this test pins.
        assert_eq!(
            compose_filter("info").max_level_hint(),
            Some(tracing::Level::INFO.into())
        );
        assert_eq!(
            compose_filter("quiver=debug,info").max_level_hint(),
            Some(tracing::Level::DEBUG.into())
        );
        assert_eq!(
            compose_filter("trace").max_level_hint(),
            Some(tracing::Level::TRACE.into())
        );
    }

    #[test]
    fn unparseable_directives_end_at_an_error_floor_anyway() {
        // Whether the composed string parses (EnvFilter is famously
        // lenient — `a=` becomes a target directive) or the fallback arm
        // fires, a degenerate directive must never disable error-level
        // output: the hint can only sit *at or above* error. The binary
        // level proof (garbage QUIVER_LOG still emits ERROR lines on the
        // real binary) lives in examples/quiver-smoke.sh §7.
        let error = Some(tracing_subscriber::filter::LevelFilter::ERROR);
        for bad in ["=", "a=", "==", ",,,,", "not-a-level"] {
            let hint = compose_filter(bad).max_level_hint();
            assert!(
                hint >= error,
                "{bad:?} must never fall below the error floor: {hint:?}"
            );
        }
    }
    // as `error,not-a-level` the string still *parses* (target

    #[test]
    fn bare_words_compose_with_the_floor_instead_of_replacing_it() {
        // `EnvFilter` alone would read `not-a-level` as a target directive
        // and silence everything else — the bug this module fixes. Composed
        // end-to-end proof (garbage QUIVER_LOG still emits ERROR lines)
        // lives in examples/quiver-smoke.sh §7, which runs the real binary.
        assert!(EnvFilter::try_new("error,not-a-level").is_ok());
        assert_eq!(
            compose_filter("not-a-level").max_level_hint(),
            Some(tracing::Level::TRACE.into()),
            "a target directive raises the hint; the floor keeps errors on"
        );
    }
}
