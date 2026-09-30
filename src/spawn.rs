//! Interpreter dispatch and child execution.
//!
//! Commands always run through a shell interpreter so `>`, `$()`, env
//! expansion and the rest keep working. Two entry points cover the two things
//! Wox asks a plugin to do:
//!
//! * [`run_detached`] — an action: spawn and return, so the launcher hides at
//!   once — the command outlives the plugin RPC, which is what stays inside the
//!   10 s script budget.
//! * [`run_capture`] — a `capture` query: run to completion and hand back the
//!   output, which is the only way a script plugin can show it (see
//!   `docs/catalog-contract.md §5`).
//!
//! Both detach the child from this process's console (`platform::detach`).
//!
//! This module contains **no platform knowledge**: variable spellings, quoting
//! rules and the interpreter table all come from [`crate::platform`].

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use std::borrow::Cow;

use crate::platform;

/// Expand `%VAR%` (cmd form), `$VAR` / `${VAR}` (shell form) and a leading `~`.
///
/// Both variable forms are honoured on every platform on purpose: the catalog
/// is shared, so an entry written as `$HOME/workspace` must also resolve on
/// Windows. An unset reference is left verbatim so a typo stays visible rather
/// than blanking half a path.
///
/// Implementation: a tiny `%NAME%` pre-pass (shellexpand speaks POSIX shell
/// syntax, not cmd syntax), then `shellexpand::full_with_context_no_errors`
/// for `~` / `$X` / `${X}`. The shellexpand lookup callback returns
/// `None` for an unset var, which shellexpand treats as "leave verbatim" —
/// the same contract the hand-rolled walker enforced.
pub fn expand_env(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let after_percent = expand_percent_vars(value);
    let expanded = shellexpand::full_with_context_no_errors(
        &after_percent,
        || {
            let home = platform::home_dir();
            if home.is_empty() { None } else { Some(home) }
        },
        platform::env_var,
    );
    match expanded {
        Cow::Borrowed(_) => after_percent,
        Cow::Owned(s) => s,
    }
}

/// Replace `%NAME%` occurrences using the platform's expandable env list,
/// leaving unset names and names outside the list untouched. Direct calls
/// from tests pass a custom lookup so the rule is exercisable without
/// mutating the process environment.
fn expand_percent_vars(input: &str) -> String {
    expand_percent_vars_with(
        input,
        platform::EXPANDABLE_ENV_VARS,
        &platform::env_var,
    )
}

fn expand_percent_vars_with(
    input: &str,
    names: &[&str],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> String {
    let mut out = input.to_string();
    for name in names {
        let needle = format!("%{name}%");
        // `if … && let …` (a let-chain) is stable on the crate's MSRV (1.91).
        if out.contains(&needle)
            && let Some(value) = lookup(name)
        {
            out = out.replace(&needle, &value);
        }
    }
    out
}

/// The interpreter a catalog entry actually dispatches to: its own name
/// (trimmed, ASCII-lowercased), or the platform default when it names none.
///
/// Both users need the *resolved* name rather than the raw field:
/// [`resolve_argv`] looks it up in [`platform::INTERPRETERS`], and
/// [`platform::prepare_capture_command`] picks the encoding preamble from it —
/// so an empty field cannot silently skip a preamble the default needs.
pub fn effective_interpreter(interpreter: &str) -> String {
    // `to_ascii_lowercase` rather than `to_lowercase`: interpreter names are
    // ASCII, and the Unicode case tables are ~7 KB of the artifact.
    let named = interpreter.trim().to_ascii_lowercase();
    if named.is_empty() {
        platform::DEFAULT_INTERPRETER.to_string()
    } else {
        named
    }
}

/// Map an interpreter name + command to argv.
///
/// This function holds **no platform knowledge**: the recognised names and
/// their argv shapes live in [`platform::INTERPRETERS`], selected at compile
/// time, so a POSIX build simply has no arm for `powershell` and a Windows
/// build none for `zsh`. An empty interpreter takes the platform default; a
/// name that is not in the table is treated as the executable itself, which
/// is how entries like `kubectl get pods` are invoked.
///
/// Lookup goes through the stem, not the exact string: `cmd`, `CMD`,
/// `cmd.exe` and `C:\Windows\System32\cmd.exe` are all the table's `cmd`.
/// Without the stem arm a `cmd.exe`-spelled entry misses the table, loses
/// its mandatory `/c`, and cmd — which ignores arguments given neither `/c`
/// nor `/k` — silently no-ops with exit 0 (measured on real cmd.exe; the
/// same gap breaks `bash.exe` and absolute shell paths).
pub fn resolve_argv(interpreter: &str, command: &str) -> Vec<String> {
    let raw = interpreter.trim();
    let effective = effective_interpreter(interpreter);

    let lookup = |name: &str| {
        platform::INTERPRETERS
            .iter()
            .find(|(table, _)| *table == name)
            .map(|(_, prefix)| *prefix)
    };

    let hit = lookup(effective.as_str())
        .or_else(|| lookup(&platform::exe_stem(&effective)));

    if let Some(prefix) = hit {
        let mut argv: Vec<String> =
            prefix.iter().map(|s| s.to_string()).collect();
        argv.push(command.to_string());
        return argv;
    }

    // Unknown interpreter: the original spelling is the executable name, so
    // case is preserved (`/usr/bin/FFmpeg` stays exact on POSIX).
    let mut argv = vec![raw.to_string()];
    argv.extend(platform::split_command_line(command));
    argv
}
/// Expand a catalog working directory and accept it only if it is a real
/// directory.
///
/// A working directory is a **hint, not a precondition**. The catalog's
/// default is `$HOME/workspace`, which exists on the WSL side but not
/// necessarily on Windows — and a non-existent `current_dir` makes `spawn`
/// fail outright (Windows `ERROR_DIRECTORY`, os error 267), so every alias
/// would silently stop working. Warn and run in the inherited directory
/// instead: the alias is what the user asked for.
///
/// A **relative** path is joined onto the home directory first: resolving it
/// against this process's inherited cwd (whatever directory Wox happened to
/// spawn the plugin from) would make the same catalog entry run in a
/// different directory on every machine — or silently drop the hint when
/// that unrelated directory happens to be missing.
fn resolve_cwd(working_directory: &str) -> Option<String> {
    let mut cwd = expand_env(working_directory);
    if cwd.is_empty() {
        return None;
    }
    let absolute = if Path::new(&cwd).is_absolute() {
        cwd
    } else {
        let mut joined = PathBuf::from(platform::home_dir());
        joined.push(cwd.as_str());
        cwd = joined.display().to_string();
        cwd
    };
    if Path::new(&absolute).is_dir() {
        return Some(absolute);
    }
    eprintln!(
        "{}: working directory '{absolute}' does not exist; running in the inherited directory",
        crate::identity::NAME
    );
    None
}

/// A command's captured result, as returned by [`run_capture`].
#[derive(Debug)]
pub struct CommandResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// `None` when the child was killed by a signal rather than exiting.
    pub exit_code: Option<i32>,
    /// The command line as the catalog wrote it, echoed back for the preview.
    pub command: String,
}

/// Build a detached `Command` for `command` under `interpreter`.
///
/// Infallible: [`resolve_argv`] always yields at least the interpreter or the
/// command itself, and `Command` construction cannot fail — a program that
/// does not exist surfaces at spawn time, where the caller can report it.
fn build(command: &str, interpreter: &str, working_directory: &str) -> Command {
    let argv = resolve_argv(interpreter, command);
    let mut cmd = Command::new(&argv[0]);
    // Quoting is the platform's business: `cmd.exe` reads the raw command line
    // and cannot parse Rust's MSVC-style escapes (see
    // [`platform::apply_command_args`]).
    platform::apply_command_args(&mut cmd, &argv[0], &argv[1..]);
    cmd.stdin(Stdio::null());
    if let Some(cwd) = resolve_cwd(working_directory) {
        cmd.current_dir(cwd);
    }
    platform::detach(&mut cmd);
    cmd
}

/// Fire-and-forget execution: the command's own effect is the point.
///
/// This is what a `run` action uses. Wox's script host ignores the action
/// response (`host_script.go: handleActionResult`), so there is nothing to
/// render even if we waited — the launcher has already hidden. Returning
/// immediately also keeps the action RPC clear of the 10 s script budget no
/// matter how long the command runs.
pub fn run_detached(command: &str, interpreter: &str, working_directory: &str) {
    let mut cmd = build(command, interpreter, working_directory);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    if let Err(e) = cmd.spawn() {
        eprintln!("{}: spawn failed: {e}", crate::identity::NAME);
    }
}

/// Local bound on a capture run, kept under Wox's 10 s script timeout so the
/// plugin still has time to answer with a "killed" result instead of dying
/// mid-response. See [`run_capture_with_deadline`].
const CAPTURE_DEADLINE: Duration = Duration::from_secs(8);

/// How often the capture loop polls the child while waiting for the
/// deadline. 10 ms is far below human perception for a preview pane.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Run the command to completion, capturing stdout, stderr and the exit code.
///
/// Only `capture: true` catalog entries use this. Running at **query** time is
/// the one way a script plugin can surface a command's output in the launcher:
/// the query response's `preview` field is the only field Wox renders that the
/// script controls (see `docs/catalog-contract.md §5`). The wait is doubly
/// bounded — by Wox's `scriptExecutionTimeout` (10 s by default; override
/// with `WOX_SCRIPT_EXECUTION_TIMEOUT`) *and* by the plugin-local
/// [`CAPTURE_DEADLINE`] below it, which kills a hung child rather than
/// letting it outlive the query — so a capture entry must be a quick lookup,
/// never a build.
///
/// This is also the only path that *reads* the child's bytes, so it is the only
/// path that needs the platform's captured-output encoding preamble
/// ([`platform::prepare_capture_command`]) — and the only path where a
/// mis-decoded byte can reach the user, through the preview and the clipboard's
/// first line. `CommandResult::command` stays the catalog's own text: the
/// preamble is an encoding detail, exactly as it is invisible in Wox.
pub fn run_capture(
    command: &str,
    interpreter: &str,
    working_directory: &str,
) -> CommandResult {
    run_capture_with_deadline(
        command,
        interpreter,
        working_directory,
        CAPTURE_DEADLINE,
    )
}

/// Run the command to completion under a local deadline, capturing stdout,
/// stderr and the exit code; a child that outlives the deadline is killed
/// and reported instead of being orphaned.
///
/// The deadline matters because [`build`] already detached the child into
/// its own process group ([`platform::detach`]) — Wox can only enforce its
/// own script timeout by killing *this plugin*, which does not signal the
/// child. Without a local bound, one hung `capture` entry would take the
/// whole query response with it and leave the command running.
///
/// Test-only parameterization: production always passes
/// [`CAPTURE_DEADLINE`]; tests shrink it so the kill path is exercisable
/// without a slow suite.
fn run_capture_with_deadline(
    command: &str,
    interpreter: &str,
    working_directory: &str,
    deadline: std::time::Duration,
) -> CommandResult {
    let spawned = platform::prepare_capture_command(
        &effective_interpreter(interpreter),
        command,
    );
    let mut cmd = build(&spawned, interpreter, working_directory);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            eprintln!("{}: spawn failed: {e}", crate::identity::NAME);
            return CommandResult {
                stdout: Vec::new(),
                stderr: format!("spawn failed: {e}\n").into_bytes(),
                exit_code: None,
                command: command.to_string(),
            };
        }
    };

    // Both pipes are read on their own threads: reading them sequentially on
    // this thread could deadlock against a child that fills one pipe while
    // blocked writing the other. Closing on kill/exit unblocks the readers.
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || read_all(out_pipe));
    let err_reader = std::thread::spawn(move || read_all(err_pipe));

    let expired = Instant::now() + deadline;
    let mut status = None;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                status = Some(exit);
                break;
            }
            Ok(None) => {
                if Instant::now() >= expired {
                    timed_out = true;
                    // The whole tree: a bare kill leaves `sh -lc '…; sleep
                    // 30'`'s grandchild holding the pipes (and the query).
                    platform::kill_tree(&mut child);
                    break;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                eprintln!("{}: wait failed: {e}", crate::identity::NAME);
                break;
            }
        }
    }

    let stdout = out_reader.join().unwrap_or_default();
    let mut stderr = err_reader.join().unwrap_or_default();
    if timed_out {
        let note = format!(
            "capture exceeded the {}s deadline; child killed\n",
            deadline.as_secs_f64()
        );
        stderr.extend_from_slice(note.as_bytes());
    }
    CommandResult {
        stdout,
        stderr,
        exit_code: status.and_then(|s| s.code()),
        command: command.to_string(),
    }
}

/// Drain an optional child pipe to EOF. `None` covers a child spawned
/// without that pipe.
fn read_all(mut pipe: Option<impl std::io::Read>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(p) = pipe.as_mut() {
        let _ = p.read_to_end(&mut buf);
    }
    buf
}
#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{env_lock, setenv, unsetenv};

    /// Expected argv for a name in the platform table, read from that table
    /// so the assertion cannot drift from it.
    fn expected(name: &str, command: &str) -> Vec<String> {
        let (_, prefix) = platform::INTERPRETERS
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| {
                panic!("{name} missing from this platform's table")
            });
        let mut argv: Vec<String> =
            prefix.iter().map(|s| s.to_string()).collect();
        argv.push(command.to_string());
        argv
    }

    #[test]
    fn every_table_entry_dispatches_with_its_own_flags() {
        for (name, _) in platform::INTERPRETERS {
            assert_eq!(
                resolve_argv(name, "CMD"),
                expected(name, "CMD"),
                "{name}"
            );
            // Lookup is case-insensitive, like Wox's own alias matching.
            assert_eq!(
                resolve_argv(&name.to_ascii_uppercase(), "CMD"),
                expected(name, "CMD"),
                "{name} uppercase"
            );
        }
    }

    /// The dispatch-level arm of the spelling rule: `cmd.exe`, `CMD.EXE` and
    /// full paths must reach the table's `cmd` — with its mandatory `/c` —
    /// exactly as `platform::is_cmd`/`prepare_capture_command` recognize
    /// them. Before the stem lookup existed, these spellings fell through
    /// to the raw-argv path and cmd silently ignored its arguments
    /// (measured: `cmd.exe exit 7` exited 0).
    #[cfg(windows)]
    #[test]
    fn cmd_spellings_dispatch_with_the_table_flags() {
        for exe in [
            "cmd",
            "CMD",
            "cmd.exe",
            "Cmd.EXE",
            r"C:\Windows\System32\cmd.exe",
            "C:/Windows/System32/cmd.exe",
        ] {
            assert_eq!(
                resolve_argv(exe, "echo hi"),
                expected("cmd", "echo hi"),
                "{exe}"
            );
        }
    }

    /// POSIX mirror of the spelling rule: an absolute interpreter path is
    /// the same interpreter the table names, so it gets the table's argv
    /// shape (`-lc`), not a fall-through that would hand the whole command
    /// to bash as a script path.
    #[cfg(not(windows))]
    #[test]
    fn absolute_interpreter_paths_dispatch_with_the_table_flags() {
        assert_eq!(
            resolve_argv("/usr/bin/bash", "echo hi"),
            expected("bash", "echo hi")
        );
        assert_eq!(
            resolve_argv("/bin/sh", "echo hi"),
            expected("sh", "echo hi")
        );
    }

    /// An executable that is *not* a table interpreter — under any spelling —
    /// stays verbatim: the stem lookup must not swallow opaque tools.
    #[test]
    fn opaque_executables_stay_verbatim_under_any_spelling() {
        assert_eq!(
            resolve_argv("/opt/bin/ffmpeg", "-i x"),
            vec![
                "/opt/bin/ffmpeg".to_string(),
                "-i".to_string(),
                "x".to_string()
            ]
        );
        assert_eq!(
            resolve_argv("C:/tools/mytool.exe", "a b"),
            vec![
                "C:/tools/mytool.exe".to_string(),
                "a".to_string(),
                "b".to_string()
            ]
        );
    }

    #[test]
    fn empty_interpreter_uses_platform_default() {
        // The default is selected at compile time in `platform`, so this also
        // pins that an empty interpreter never yields an empty argv.
        let argv = resolve_argv("", "true");
        assert_eq!(argv[0], platform::DEFAULT_INTERPRETER);
        assert_eq!(argv.last().unwrap(), "true");
    }

    #[test]
    fn the_other_platforms_shells_take_the_raw_argv_path() {
        // The other platform's shell is not compiled into this build, so it
        // must fall through to "the name is the executable" instead of
        // receiving flags this host cannot run — the observable form of the
        // compile-time split. The name itself comes from `platform`, so no
        // platform knowledge leaks into this module, not even in a test.
        let foreign = platform::FOREIGN_SHELL;
        assert_eq!(
            resolve_argv(foreign, "echo hi"),
            vec![foreign.to_string(), "echo".to_string(), "hi".to_string()]
        );
        assert!(platform::INTERPRETERS.iter().all(|(n, _)| *n != foreign));
    }

    #[test]
    fn python_resolves_to_a_host_python_with_its_own_flag() {
        assert_eq!(
            resolve_argv("python", "print(1)"),
            expected("python", "print(1)")
        );
    }

    #[test]
    fn raw_argv_fallback_honours_quotes() {
        assert_eq!(
            resolve_argv("kubectl", "get pods -o wide"),
            vec!["kubectl", "get", "pods", "-o", "wide"]
        );
        assert_eq!(
            resolve_argv("git", r#"commit -m "a b""#),
            vec!["git", "commit", "-m", "a b"]
        );
    }

    /// The capture path exists to read the child's bytes back, and a Windows
    /// console program writes the **console code page** rather than UTF-8 — on
    /// the zh-CN install this was measured on, `d6 d0 ce c4` for `中文` (GBK).
    /// `chcp 65001` does not change that for cmd's own builtins, which is why
    /// the pair under test is [`run_capture`] **plus** [`decode_captured`]:
    /// PowerShell is fixed before it runs (the UTF-8 preamble), `cmd` after
    /// (the OEM code-page fallback).
    ///
    /// The needle is `café` rather than `中文` so the assertion holds on any
    /// Windows locale while still forcing the fallback on each of them: `é` is
    /// representable in CP437 (0x82), CP1252 (0xE9) and CP936 (0xA8A6), and
    /// **none** of those byte sequences is valid UTF-8 — so a regression that
    /// dropped the fallback would show up as `caf��` on every Windows, not just
    /// an East-Asian one.
    ///
    /// Windows-only by construction (POSIX has no code page to fall back to),
    /// and it really runs: the windows-gnu test binary is executed through WSL
    /// interop, so `cmd.exe`/`powershell.exe` are the real Windows ones.
    #[cfg(windows)]
    #[test]
    fn captured_windows_output_survives_both_console_interpreters() {
        for interpreter in ["cmd", "powershell"] {
            let result = run_capture("echo café", interpreter, "");
            assert_eq!(result.exit_code, Some(0), "{interpreter}");
            assert_eq!(
                crate::platform::decode_captured(&result.stdout).trim(),
                "café",
                "{interpreter}: raw bytes {:?}",
                result.stdout
            );
        }
    }

    /// The capture contract on the primary dev host: both streams arrive as
    /// separate buffers, a failing exit code is `Some(n)` (and `None` is
    /// reserved for spawn failure / signals), and the working directory is
    /// actually applied.
    #[cfg(unix)]
    #[test]
    fn posix_capture_returns_both_streams_and_the_exit_code() {
        let result = run_capture("echo out; echo err 1>&2; exit 3", "sh", "");
        assert_eq!(result.stdout, b"out\n", "{:?}", result.stdout);
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("err"),
            "{:?}",
            result.stderr
        );
        assert_eq!(result.exit_code, Some(3));
        assert_eq!(result.command, "echo out; echo err 1>&2; exit 3");
    }

    /// A hung capture child is killed at the deadline and *reported*, never
    /// orphaned: without the kill, Wox's own timeout would take this plugin
    /// mid-response and leave the child running in its own process group.
    #[cfg(unix)]
    #[test]
    fn a_hung_capture_child_is_killed_at_the_deadline() {
        let started = Instant::now();
        let result = run_capture_with_deadline(
            "echo start; sleep 30",
            "sh",
            "",
            Duration::from_millis(300),
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the deadline must bound the wait, took {:?}",
            started.elapsed()
        );
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("deadline; child killed"),
            "{:?}",
            result.stderr
        );
        // Whatever the child managed to write before the kill still arrives.
        assert_eq!(result.stdout, b"start\n");
    }

    /// `run_detached` is the whole action path: spawn, return, and the effect
    /// happens without the plugin waiting for it. The marker file is the
    /// observable side effect; polling covers process-start latency.
    #[cfg(unix)]
    #[test]
    fn run_detached_fires_and_forgets() {
        let dir = std::env::temp_dir().join(format!(
            "quiver-detach-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let marker = dir.join("ran");
        run_detached(
            &format!("touch {}", marker.display()),
            "sh",
            &dir.display().to_string(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                Instant::now() < deadline,
                "detached command did not fire within 5s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_is_used_only_when_it_exists() {
        let tmp = std::env::temp_dir();
        assert_eq!(
            resolve_cwd(&tmp.to_string_lossy()),
            Some(tmp.to_string_lossy().into_owned())
        );
        assert_eq!(resolve_cwd(""), None);
        assert_eq!(resolve_cwd("/definitely/not/a/dir/quiver"), None);
    }

    /// A relative working directory resolves against **home**, never against
    /// whatever directory Wox happened to spawn the plugin in — the inherited
    /// cwd is machine-accident, not catalog meaning.
    #[test]
    fn relative_cwd_resolves_against_home_not_the_process_cwd() {
        let home = platform::home_dir();
        // `.` always exists, so the assertion observes *which base* the
        // relative path was joined onto: home, never the process cwd.
        let want = PathBuf::from(&home).join(".").display().to_string();
        assert_eq!(
            resolve_cwd("."),
            Some(want),
            "a relative hint is joined onto home before the is_dir check"
        );
        // A relative path that exists nowhere (under home or anywhere else)
        // is dropped with a warning, and the child inherits — never a
        // half-resolved guess.
        assert_eq!(resolve_cwd("definitely-not/a-real-dir-quiver"), None);
    }

    #[test]
    fn percent_vars_expand_or_stay_verbatim() {
        // The lookup is injected so this rule is testable without mutating the
        // process environment (which other tests read).
        let lookup = |name: &str| (name == "KNOWN").then(|| "/x".to_string());
        assert_eq!(
            expand_percent_vars_with("%KNOWN%/y", &["KNOWN"], &lookup),
            "/x/y",
            "a resolvable name expands"
        );
        assert_eq!(
            expand_percent_vars_with("%UNSET%/y", &["KNOWN"], &lookup),
            "%UNSET%/y",
            "an unresolvable name is left verbatim, never blanked"
        );
        assert_eq!(
            expand_percent_vars_with("%OTHER%/y", &["KNOWN"], &lookup),
            "%OTHER%/y",
            "a name outside the platform list is not touched"
        );
    }

    #[test]
    fn expansion_keeps_unset_verbatim_and_tilde_expands() {
        // Env mutation is process-global: hold the crate-wide lock so the
        // parallel protocol/catalog tests never read a half-mutated environ.
        let _env = env_lock();
        let _unset = unsafe { unsetenv("QUIVER_TEST_UNSET") };
        assert_eq!(expand_env("$QUIVER_TEST_UNSET"), "$QUIVER_TEST_UNSET");

        let _set = unsafe { setenv("QUIVER_TEST_SET", "ok") };
        assert_eq!(expand_env("${QUIVER_TEST_SET}/x"), "ok/x");
        assert_eq!(expand_env("$QUIVER_TEST_SET/x"), "ok/x");

        assert!(!expand_env("~/w").starts_with('~'));
    }
}
