//! Every platform difference, resolved at compile time.
//!
//! This is the **only** module in the crate whose *production code* knows
//! which OS it is on: no other file contains a non-test `#[cfg]` platform
//! gate, a Windows/POSIX-only identifier (`USERPROFILE`, `LOCALAPPDATA`,
//! `powershell`, `cmd`, `zsh`, …), or a platform-*selecting* rule. Each
//! artifact therefore contains exactly one platform's behaviour, and adding
//! a platform is a single-file review. (Cross-platform *tolerance* —
//! accepting both separator spellings, or `~\` as a home prefix — is not
//! selection and lives where it is used; the `#[cfg(windows)]`/
//! `#[cfg(unix)]` gates inside other modules' `mod tests` run the real
//! platform's shells and select nothing in the shipping binary.)
//!
//! What lives here, and why it is platform knowledge rather than generic code:
//!
//! | Item | Platform fact |
//! |---|---|
//! | [`IS_WINDOWS`] | the host itself |
//! | [`HOME_VARS`] | Windows sets no `HOME`, POSIX no `USERPROFILE` |
//! | [`EXPANDABLE_ENV_VARS`] | `LOCALAPPDATA` is Windows, `XDG_DATA_HOME` POSIX |
//! | [`split_command_line`] | MSVC rules on Windows, quote-aware on POSIX |
//! | [`detach`] | `CREATE_NO_WINDOW` vs `process_group(0)` |
//!
//! Everything is a `const` or a `#[cfg]`-selected function, so the compiler
//! folds it away; there is no runtime platform branch anywhere in the crate.
//! | [`detach`] | `CREATE_NO_WINDOW` vs `process_group(0)` |
//!
//! Everything is a `const` or a `#[cfg]`-selected function, so the compiler
//! folds it away; there is no runtime platform branch anywhere in the crate.

// The crate supports the Windows and Unix families and nothing else: the
// tables below key off `not(windows)`, the genuinely Unix-specific API
// (`process_group`) off `unix`, and any third family fails loudly here
// instead of compiling half a module. Adding a platform means extending
// this file — see the module docs.
#[cfg(not(any(windows, unix)))]
compile_error!(
    "quiver supports the Windows and Unix target families only; \
     porting further means extending src/platform.rs"
);

use std::process::Command;

// ---------------------------------------------------------------------------
// The host
// ---------------------------------------------------------------------------

/// True on Windows. A `const bool` rather than an enum because the other
/// variant would never be constructed on a given host — which the compiler
/// reports as dead code, and an `allow` would only hide that the value folds to
/// a constant anyway.
#[cfg(windows)]
pub const IS_WINDOWS: bool = true;
#[cfg(not(windows))]
pub const IS_WINDOWS: bool = false;

// ---------------------------------------------------------------------------
// Environment model
// ---------------------------------------------------------------------------

/// Spellings of the home directory, in the order this platform should consult
/// them. Both are listed because either may be present, but the native one
/// comes first.
#[cfg(windows)]
pub const HOME_VARS: &[&str] = &["USERPROFILE", "HOME"];
#[cfg(not(windows))]
pub const HOME_VARS: &[&str] = &["HOME", "USERPROFILE"];

/// Variables a catalog entry may reference as `%VAR%` or `$VAR`.
///
/// Both home spellings appear on both platforms (a shared catalog may use
/// either); the third entry is the platform's own data directory.
#[cfg(windows)]
pub const EXPANDABLE_ENV_VARS: &[&str] =
    &["USERPROFILE", "HOME", "LOCALAPPDATA"];
#[cfg(not(windows))]
pub const EXPANDABLE_ENV_VARS: &[&str] =
    &["HOME", "USERPROFILE", "XDG_DATA_HOME"];

/// The other spelling of `name`, when the two denote the same directory.
fn home_alias(name: &str) -> Option<&'static str> {
    match name {
        "HOME" => Some("USERPROFILE"),
        "USERPROFILE" => Some("HOME"),
        _ => None,
    }
}

/// Read a variable, treating an empty value as unset — Windows and WSL both
/// define variables with empty values, and `` is never a useful path.
fn lookup(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Look up an environment variable, falling back to the other spelling when it
/// names the same directory.
pub fn env_var(name: &str) -> Option<String> {
    lookup(name).or_else(|| home_alias(name).and_then(lookup))
}

/// The user's home directory, or `""` when the environment offers neither
/// spelling.
pub fn home_dir() -> String {
    HOME_VARS
        .iter()
        .find_map(|name| lookup(name))
        .unwrap_or_default()
}

/// Interpreters that exist only on the *other* platform. A shared catalog may
/// name them (the same file is deployed to every machine), but no build of this
/// binary can execute them: `powershell`/`pwsh`/`cmd` are Windows-only and
/// `sh`/`zsh`/`python3` are POSIX-only.
///
/// This is the *only* set treated as unavailable. A name outside every table —
/// an absolute script path, `kubectl`, `git` — is dispatched verbatim by
/// [`crate::spawn::resolve_argv`] and may well exist on the host, so rejecting
/// it would silently disable `capture: true` for exactly the shebang-dispatch
/// entries the catalog documents.
#[cfg(windows)]
pub const FOREIGN_INTERPRETERS: &[&str] = &["sh", "zsh", "python3"];
#[cfg(not(windows))]
pub const FOREIGN_INTERPRETERS: &[&str] = &["powershell", "pwsh", "cmd"];

/// A shell that exists only on the *other* platform, so tests can prove the
/// compile-time split is real rather than merely documented. Test-only: the
/// shipping binary must not carry a name it can never dispatch to.
#[cfg(all(test, windows))]
pub const FOREIGN_SHELL: &str = "sh";
#[cfg(all(test, not(windows)))]
pub const FOREIGN_SHELL: &str = "powershell";

/// Wox's platform primary modifier for in-app shortcuts — its
/// `util.PrimaryModifier()`: `cmd` on macOS, `ctrl` everywhere else.
///
/// Used to spell the second gesture Wox's own Shell plugin attaches to a
/// non-silent saved command (`util.PrimaryHotkey("enter")`, `shell.go:
/// queryCommands`). Wox normalises the modifier itself, so the string is passed
/// through as-is.
#[cfg(target_os = "macos")]
pub const PRIMARY_MODIFIER: &str = "cmd";
#[cfg(not(target_os = "macos"))]
pub const PRIMARY_MODIFIER: &str = "ctrl";

// ---------------------------------------------------------------------------
// Command-line splitting
// ---------------------------------------------------------------------------

/// Split a raw argument string into argv, following this platform's own
/// quoting rules. Used for catalog entries whose `interpreter` is not one of
/// [`INTERPRETERS`] — `kubectl get pods -o wide`, `git commit -m "a b"`.
///
/// Windows follows the Microsoft C runtime rules ("Parsing C command-line
/// arguments"), where backslashes are literal *unless* they immediately precede
/// a double quote. A POSIX shell instead treats quotes as grouping, and this
/// plugin deliberately offers no backslash escaping there: an entry that needs
/// real shell semantics should set `interpreter` to `bash`/`sh`/`zsh` and get a
/// shell, not a half-shell.
#[cfg(windows)]
pub fn split_command_line(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut in_quotes = false;
    let mut backslashes = 0usize;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Held back: their meaning depends on what follows.
            backslashes += 1;
            continue;
        }

        if c == '"' {
            // An even run yields half as many backslashes and the quote acts as
            // a delimiter; an odd run yields the same halves plus a literal
            // quote. `""` inside a quoted run is one literal quote.
            for _ in 0..backslashes / 2 {
                current.push('\\');
            }
            if backslashes % 2 == 1 {
                current.push('"');
            } else if in_quotes && chars.peek() == Some(&'"') {
                current.push('"');
                chars.next();
            } else {
                in_quotes = !in_quotes;
            }
            backslashes = 0;
            started = true;
            continue;
        }

        // Not a quote: any held-back backslashes are literal.
        for _ in 0..backslashes {
            current.push('\\');
        }
        backslashes = 0;

        if !in_quotes && (c == ' ' || c == '\t') {
            if started {
                args.push(std::mem::take(&mut current));
                started = false;
            }
            continue;
        }
        current.push(c);
        started = true;
    }

    for _ in 0..backslashes {
        current.push('\\');
    }
    // An unterminated quote keeps everything read so far, per the same rules.
    if started {
        args.push(current);
    }
    args
}

#[cfg(not(windows))]
pub fn split_command_line(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;

    for c in input.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {
                current.push(c);
                started = true;
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                ' ' | '\t' | '\n' | '\r' => {
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                _ => {
                    current.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        args.push(current);
    }
    args
}

// ---------------------------------------------------------------------------
// Interpreter table
// ---------------------------------------------------------------------------

/// Interpreter used when neither the catalog entry nor the catalog default
/// names one.
#[cfg(windows)]
pub const DEFAULT_INTERPRETER: &str = "powershell";
#[cfg(not(windows))]
pub const DEFAULT_INTERPRETER: &str = "bash";

/// Catalog key that overrides the default interpreter for this platform
/// (`defaultInterpreter@windows` / `@darwin` / `@linux`).
#[cfg(windows)]
pub const CATALOG_INTERPRETER_KEY: &str = "defaultInterpreter@windows";
#[cfg(target_os = "macos")]
pub const CATALOG_INTERPRETER_KEY: &str = "defaultInterpreter@darwin";
#[cfg(all(not(windows), not(target_os = "macos")))]
pub const CATALOG_INTERPRETER_KEY: &str = "defaultInterpreter@linux";

/// Catalog key that overrides the default working directory for this platform
/// (`defaultWorkingDirectory@windows` / `@darwin` / `@linux`).
///
/// The pair exists because a catalog is shared across machines while the paths
/// it names are not: `$HOME/workspace` is a real directory on the WSL side and
/// nothing at all on the Windows side, and a `current_dir` that does not exist
/// makes `spawn` fail outright (`ERROR_DIRECTORY`). The unsuffixed
/// `defaultWorkingDirectory` remains the fallback, so only the platforms that
/// actually differ need a key.
#[cfg(windows)]
pub const CATALOG_WORKING_DIRECTORY_KEY: &str =
    "defaultWorkingDirectory@windows";
#[cfg(target_os = "macos")]
pub const CATALOG_WORKING_DIRECTORY_KEY: &str =
    "defaultWorkingDirectory@darwin";
#[cfg(all(not(windows), not(target_os = "macos")))]
pub const CATALOG_WORKING_DIRECTORY_KEY: &str = "defaultWorkingDirectory@linux";

/// The interpreters this platform actually has, as
/// `(catalog name, argv prefix)`.
///
/// These are Wox's own per-platform sets — mirroring
/// `wox.core/plugin/system/shell/shell.go: getInterpreterOptions()` — because
/// the catalog is consumed by a Wox build that only ever offers these:
///
/// * Windows: `powershell`, `cmd`, `bash` (Wox labels it "Bash (WSL)"),
///   `python`, `node`;
/// * macOS / Linux: `bash`, `sh`, `zsh`, `python3`, `node`.
///
/// Splitting the table at compile time is why a POSIX artifact carries no
/// `powershell`/`cmd` handling and a Windows artifact carries no `zsh`/`sh`
/// handling: the flat "everything on every platform" match this replaces was
/// dead weight in each direction.
///
/// Two deliberate additions to Wox's lists:
/// * `pwsh` — the modern PowerShell executable name, still Windows-side only;
/// * `python` on POSIX — the catalog is shared, so `python` is accepted there
///   and resolves to `python3`, the name that actually exists on the host.
#[cfg(windows)]
pub const INTERPRETERS: &[(&str, &[&str])] = &[
    ("powershell", &["powershell", "-NoProfile", "-Command"]),
    ("pwsh", &["pwsh", "-NoProfile", "-Command"]),
    ("cmd", &["cmd", "/c"]),
    ("bash", &["bash", "-lc"]),
    ("python", &["python", "-c"]),
    ("node", &["node", "-e"]),
];

#[cfg(not(windows))]
pub const INTERPRETERS: &[(&str, &[&str])] = &[
    ("bash", &["bash", "-lc"]),
    ("sh", &["sh", "-lc"]),
    ("zsh", &["zsh", "-lc"]),
    ("python3", &["python3", "-c"]),
    ("python", &["python3", "-c"]),
    ("node", &["node", "-e"]),
];

// ---------------------------------------------------------------------------
// Argument quoting
// ---------------------------------------------------------------------------

/// Append `args` to a `Command`, spelled the way this platform's consumer of
/// the command line expects.
///
/// Almost every interpreter in [`INTERPRETERS`] is an MSVC-runtime program
/// (`powershell.exe`, `pwsh.exe`, `bash.exe`, `python.exe`, `node.exe`), and
/// Rust's own quoting — the documented "Parsing C command-line arguments"
/// rules — is exactly what those read. `cmd.exe` is not: it re-parses the **raw**
/// command line with backslash-insensitive rules, so Rust's escaping of `"` as
/// `\"` reaches it as literal text. Measured through the plugin's `cmd` entry:
///
/// | catalog command | what `cmd.exe` used to run |
/// |---|---|
/// | `echo "a b"` | `\"a b\"` — the backslashes are printed |
/// | `echo hi \| findstr /C:"hi"` | exit code 1, empty stdout |
///
/// Passing those arguments raw hands `cmd.exe` exactly the text the catalog
/// wrote — the same text the user would type into a console.
#[cfg(windows)]
pub fn apply_command_args(cmd: &mut Command, exe: &str, args: &[String]) {
    use std::os::windows::process::CommandExt;

    if is_cmd(exe) {
        for arg in args {
            cmd.raw_arg(arg);
        }
    } else {
        cmd.args(args);
    }
}

/// POSIX has one argument model, so there is nothing to choose.
#[cfg(not(windows))]
pub fn apply_command_args(cmd: &mut Command, _exe: &str, args: &[String]) {
    cmd.args(args);
}

/// The executable stem of `name`: final path segment (either separator
/// spelling), extension dropped, ASCII-lowercased. This is the normalized
/// spelling every interpreter lookup goes through — `cmd`,
/// `CMD.EXE`, `C:\Windows\System32\cmd.exe` and `C:/Windows/System32/cmd.exe`
/// all normalize to `cmd` — so that [`crate::spawn::resolve_argv`],
/// `is_cmd` and [`prepare_capture_command`] agree on what a catalog entry
/// naming an interpreter means. Public because `spawn` dispatches through
/// it; the table it feeds stays private to this module.
pub fn exe_stem(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = match base.rfind('.') {
        Some(dot) => &base[..dot],
        None => base,
    };
    stem.to_ascii_lowercase()
}

/// True when `exe`'s [`exe_stem`] is one of `names`. The extension is
/// dropped rather than matched exactly because Windows resolves the
/// spellings of [`INTERPRETERS`] through `PATHEXT`.
#[cfg(windows)]
fn exe_stem_is(exe: &str, names: &[&str]) -> bool {
    let stem = exe_stem(exe);
    names.iter().any(|n| stem.eq_ignore_ascii_case(n))
}

/// True when `exe` names `cmd.exe`, under any spelling the interpreter table or
/// a catalog can produce: `cmd`, `CMD`, `C:\Windows\System32\cmd.exe`,
/// `C:/Windows/System32/cmd.exe`.
#[cfg(windows)]
fn is_cmd(exe: &str) -> bool {
    exe_stem_is(exe, &["cmd"])
}

/// True when `exe` names either PowerShell, spelled as the table's `powershell`
/// or `pwsh`, at any path/case/extension.
#[cfg(windows)]
fn is_powershell(exe: &str) -> bool {
    exe_stem_is(exe, &["powershell", "pwsh"])
}

// ---------------------------------------------------------------------------
// Captured-output encoding
// ---------------------------------------------------------------------------

/// Wrap `command` so the bytes a `capture: true` entry produces arrive as
/// UTF-8, mirroring `prepareShellCommand` in Wox's own Shell plugin
/// (`wox.core/plugin/system/shell/shell_process_windows.go`). This is the
/// "correct it before it runs" half of the fix; [`decode_captured`] carries
/// the other half.
///
/// | interpreter | preamble |
/// |---|---|
/// | `cmd` | `chcp 65001 >nul & ` |
/// | `powershell`, `pwsh` | `[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; ` |
///
/// Only the *capture* path needs it: the detached `run` path sends the child's
/// stdout to `Stdio::null()`, so there is no output to mis-decode.
///
/// `interpreter` is the already-resolved name
/// ([`crate::spawn::effective_interpreter`]), so an empty catalog field has
/// become this platform's default before it gets here.
#[cfg(windows)]
pub fn prepare_capture_command<'a>(
    interpreter: &str,
    command: &'a str,
) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;

    if is_cmd(interpreter) {
        return Cow::Owned(format!("chcp 65001 >nul & {command}"));
    }
    if is_powershell(interpreter) {
        const UTF8_PREAMBLE: &str = "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding; ";
        return Cow::Owned(format!("{UTF8_PREAMBLE}{command}"));
    }
    Cow::Borrowed(command)
}

/// POSIX has one text encoding in practice (UTF-8) and no console code page to
/// override, so there is nothing to prepend — not even for the `cmd`/`powershell`
/// spellings, which cannot exist on this host. Borrowed, so a capture query
/// allocates nothing here.
#[cfg(not(windows))]
pub fn prepare_capture_command<'a>(
    _interpreter: &str,
    command: &'a str,
) -> std::borrow::Cow<'a, str> {
    std::borrow::Cow::Borrowed(command)
}

/// Decode the bytes a `capture: true` command wrote, the way Wox's own Shell
/// plugin does (`shell_process_windows.go: decodeShellOutputChunk`): strict
/// UTF-8 wins, anything else is read as the **OEM code page**.
///
/// A Windows console program writes the **console code page** — CP936/GBK on a
/// zh-CN install — not UTF-8, so the preamble above is only half of Wox's fix.
/// Measurement, stdout a pipe:
///
/// | command | bytes |
/// |---|---|
/// | `cmd /c "echo 中文"` | `d6 d0 ce c4` |
/// | `cmd /c "chcp 65001 >nul & echo 中文"` | `d6 d0 ce c4` — `chcp` alone does **not** fix cmd's own builtins |
/// | `powershell -Command "echo 中文"` | `d6 d0 ce c4` |
/// | `powershell` + the UTF-8 preamble above | `e4 b8 ad e6 96 87` — the preamble **does** fix PowerShell |
///
/// So the split is: PowerShell is corrected before it runs, everything else —
/// `cmd` builtins, `tasklist`, `route` — is corrected after it runs. Without the
/// fallback a `capture: true` entry on `cmd` would put `����` on the clipboard.
///
/// One behaviour is inherited from upstream rather than fixed: a tool that
/// writes the **ANSI** page where it differs from the OEM one (Western locales,
/// 1252 vs 437) is mis-decoded. On the East-Asian pages this catalog targets the
/// two are the same page, and matching Wox is worth more than guessing.
#[cfg(windows)]
pub fn decode_captured(bytes: &[u8]) -> String {
    // Strict UTF-8 first: the PowerShell preamble, and every producer that
    // already speaks UTF-8, must pass through untouched.
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    decode_oem_code_page(bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

/// POSIX output is UTF-8 or the locale's encoding, and there is no code-page
/// API to consult; unreadable bytes become U+FFFD rather than an error.
#[cfg(not(windows))]
pub fn decode_captured(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// `GetOEMCP` + `MultiByteToWideChar`, the two Win32 calls Wox reaches through
// `golang.org/x/sys/windows`. Declared here because this crate takes no
// dependencies and `kernel32` is always linked into a Windows binary.
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetOEMCP() -> u32;
    fn MultiByteToWideChar(
        code_page: u32,
        flags: u32,
        bytes: *const u8,
        byte_len: i32,
        out: *mut u16,
        out_len: i32,
    ) -> i32;
}

/// Windows ANSI/OEM code pages are single/double-byte, so the input length is
/// the only bound; `None` means "cannot decode, caller falls back to lossy".
#[cfg(windows)]
fn decode_oem_code_page(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    let byte_len = i32::try_from(bytes.len()).ok()?;
    let code_page = unsafe { GetOEMCP() };
    if code_page == 0 {
        return None;
    }

    // SAFETY: both calls pass a valid pointer/length pair; the first asks only
    // for the needed length (`out` null), the second writes exactly that many
    // UTF-16 units into a buffer sized by the first.
    let needed = unsafe {
        MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            byte_len,
            std::ptr::null_mut(),
            0,
        )
    };
    if needed <= 0 {
        return None;
    }
    let mut wide = vec![0u16; needed as usize];
    let written = unsafe {
        MultiByteToWideChar(
            code_page,
            0,
            bytes.as_ptr(),
            byte_len,
            wide.as_mut_ptr(),
            needed,
        )
    };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(String::from_utf16_lossy(&wide))
}

// ---------------------------------------------------------------------------
// Process detachment
// ---------------------------------------------------------------------------

/// Detach the child so Wox's action handler returns and the launcher can hide
/// immediately, and so the command outlives this short-lived plugin process.
#[cfg(windows)]
pub fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;

    // CREATE_NO_WINDOW: own console, no window, so nothing flashes.
    // CREATE_NEW_PROCESS_GROUP: Ctrl+C elsewhere does not reach the child.
    //
    // Deliberately NOT DETACHED_PROCESS: that leaves the child with *no*
    // console, and console programs launched that way — notably powershell.exe,
    // the Windows default interpreter — exit without running the command.
    // Verified: `cmd /c` worked under DETACHED_PROCESS while `powershell
    // -NoProfile -Command` silently did nothing; both work with
    // CREATE_NO_WINDOW.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
}

// The one function that stays `unix` rather than `not(windows)`: its body is
// the Unix-only `process_group(0)` API. The compile_error guard above makes
// the asymmetry total — a third family cannot build half a module.
#[cfg(unix)]
pub fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // New process group: closing the launcher (or its terminal) will not
    // SIGHUP a long-running silent command.
    cmd.process_group(0);
}

/// Kill a child and everything it spawned, then reap it. Used by the capture
/// deadline: a bare `Child::kill` only reaches the direct child, so an
/// `sh -lc 'a; sleep 30'` grandchild survives, keeps the output pipe open,
/// and the capture result never lands.
///
/// Relies on [`detach`] having put the child at the head of its own process
/// group — that is what makes the group id the child's pid.
#[cfg(unix)]
pub fn kill_tree(child: &mut std::process::Child) {
    // SAFETY: `killpg` with the child's pid-as-pgid. Races with the child
    // exiting on its own resolve to ESRCH, which we ignore either way.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.wait();
}

/// Windows: the direct terminate is all std offers; console children of a
/// capture entry are killed by the OS once their pipe handles close.
#[cfg(windows)]
pub fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreter_table_is_the_platforms_own_set() {
        let names: Vec<&str> =
            INTERPRETERS.iter().map(|(name, _)| *name).collect();
        for (name, prefix) in INTERPRETERS {
            let last = prefix
                .last()
                .unwrap_or_else(|| panic!("{name} has no argv prefix"));
            // The command is appended straight after the prefix, so the prefix
            // must end with the flag that consumes it (`-Command`, `-lc`, `-c`,
            // `-e`, `/c`) and never with the bare executable — otherwise the
            // command would land in the wrong slot.
            assert!(
                last.starts_with('-') || *last == "/c",
                "{name}: argv prefix must end with a flag that takes the command, got {last:?}"
            );
        }
        // The default interpreter must be one this platform can actually run,
        // otherwise an empty `interpreter` field would never dispatch.
        assert!(names.contains(&DEFAULT_INTERPRETER));

        if IS_WINDOWS {
            assert!(names.contains(&"powershell") && names.contains(&"cmd"));
            assert!(
                names.contains(&"bash"),
                "Wox offers Bash (WSL) on Windows"
            );
            assert!(!names.contains(&"zsh") && !names.contains(&"sh"));
        } else {
            assert!(
                names.contains(&"bash")
                    && names.contains(&"zsh")
                    && names.contains(&"sh")
            );
            assert!(
                !names.contains(&"powershell") && !names.contains(&"cmd"),
                "POSIX has no PowerShell/CMD to dispatch to"
            );
        }
    }

    #[test]
    fn exactly_one_catalog_key_is_compiled_in() {
        assert!(CATALOG_INTERPRETER_KEY.starts_with("defaultInterpreter@"));
        assert!(
            CATALOG_WORKING_DIRECTORY_KEY
                .starts_with("defaultWorkingDirectory@")
        );
        #[cfg(target_os = "linux")]
        assert_eq!(CATALOG_INTERPRETER_KEY, "defaultInterpreter@linux");
        #[cfg(target_os = "linux")]
        assert_eq!(
            CATALOG_WORKING_DIRECTORY_KEY,
            "defaultWorkingDirectory@linux"
        );
        #[cfg(target_os = "macos")]
        assert_eq!(CATALOG_INTERPRETER_KEY, "defaultInterpreter@darwin");
        #[cfg(target_os = "macos")]
        assert_eq!(
            CATALOG_WORKING_DIRECTORY_KEY,
            "defaultWorkingDirectory@darwin"
        );
        #[cfg(windows)]
        assert_eq!(CATALOG_INTERPRETER_KEY, "defaultInterpreter@windows");
        #[cfg(windows)]
        assert_eq!(
            CATALOG_WORKING_DIRECTORY_KEY,
            "defaultWorkingDirectory@windows"
        );
    }

    #[test]
    fn the_foreign_table_names_only_the_other_platforms_shells() {
        // Every name in it must be absent from this platform's own table — the
        // two sets are disjoint, which is what makes "foreign ⇒ unrunnable" a
        // total rule for a shared catalog.
        for &name in FOREIGN_INTERPRETERS {
            assert!(
                !INTERPRETERS.iter().any(|(n, _)| *n == name),
                "{name} is foreign, so it cannot be in this platform's table"
            );
        }
        assert!(FOREIGN_INTERPRETERS.contains(&FOREIGN_SHELL));
        if IS_WINDOWS {
            assert!(!FOREIGN_INTERPRETERS.contains(&"powershell"));
            assert!(!FOREIGN_INTERPRETERS.contains(&"cmd"));
        } else {
            assert!(FOREIGN_INTERPRETERS.contains(&"powershell"));
            assert!(FOREIGN_INTERPRETERS.contains(&"cmd"));
            assert!(!FOREIGN_INTERPRETERS.contains(&"zsh"));
        }
    }

    #[test]
    fn home_spellings_are_aliases_of_each_other() {
        // Windows sets no HOME, POSIX no USERPROFILE, so either spelling must
        // resolve — the catalog is shared.
        assert_eq!(home_alias("HOME"), Some("USERPROFILE"));
        assert_eq!(home_alias("USERPROFILE"), Some("HOME"));
        assert_eq!(home_alias("PATH"), None);
        // The native spelling is consulted first on each platform.
        assert!(HOME_VARS.contains(&if IS_WINDOWS {
            "USERPROFILE"
        } else {
            "HOME"
        }));
        assert_eq!(
            HOME_VARS[0],
            if IS_WINDOWS { "USERPROFILE" } else { "HOME" }
        );
    }

    #[test]
    fn each_platform_lists_its_own_data_directory_variable() {
        let expected_data_var = if IS_WINDOWS {
            "LOCALAPPDATA"
        } else {
            "XDG_DATA_HOME"
        };
        assert!(EXPANDABLE_ENV_VARS.contains(&expected_data_var));
        let other = if IS_WINDOWS {
            "XDG_DATA_HOME"
        } else {
            "LOCALAPPDATA"
        };
        assert!(
            !EXPANDABLE_ENV_VARS.contains(&other),
            "{other} belongs to the other platform"
        );
        // Both home spellings stay expandable everywhere: a shared catalog may
        // write either, and `env_var` aliases them.
        assert!(EXPANDABLE_ENV_VARS.contains(&"HOME"));
        assert!(EXPANDABLE_ENV_VARS.contains(&"USERPROFILE"));
    }

    /// Vectors transcribed from "Parsing C command-line arguments"
    /// (Microsoft Learn, msvc-170). Backslash counts are given per vector
    /// because that is exactly what the rules turn on.
    #[cfg(windows)]
    #[test]
    fn command_line_splitting_follows_the_msvc_documented_vectors() {
        let cases: &[(&str, &[&str])] = &[
            // "a b c" d e
            (r#""a b c" d e"#, &["a b c", "d", "e"]),
            // "ab\"c" "\\" d  →  2 backslashes in the second word collapse to 1
            (r#""ab\"c" "\\" d"#, &["ab\"c", "\\", "d"]),
            // 3 backslashes are literal (they do not touch a quote)
            (r#"a\\\b d"e f"g h"#, &[r"a\\\b", "de fg", "h"]),
            // 3 backslashes + quote → 1 backslash + a literal quote
            (r#"a\\\"b c d"#, &[r#"a\"b"#, "c", "d"]),
            // 4 backslashes + quote → 2 backslashes, then the quote delimits
            (r#"a\\\\"b c" d e"#, &[r"a\\b c", "d", "e"]),
            // "" inside a quoted run is one literal quote; the quote never closes
            (r#"a"b"" c d"#, &[r#"ab" c d"#]),
        ];
        for (input, expected) in cases {
            assert_eq!(split_command_line(input), *expected, "input: {input}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn posix_splitting_groups_on_quotes_and_keeps_other_characters() {
        assert_eq!(
            split_command_line("get pods -o wide"),
            ["get", "pods", "-o", "wide"]
        );
        assert_eq!(
            split_command_line(r#"commit -m "a b""#),
            ["commit", "-m", "a b"]
        );
        assert_eq!(split_command_line("say 'a b'"), ["say", "a b"]);
        // No backslash escaping by design: use a real shell for that.
        assert_eq!(split_command_line(r"a\ b"), [r"a\", "b"]);
        assert_eq!(split_command_line("   "), Vec::<String>::new());
    }

    /// The capture encoding preamble lands exactly on the interpreters that
    /// write the Windows console code page — and on no others, on either host.
    /// The borrow check pins the POSIX path (and every non-console Windows
    /// interpreter) to zero allocation.
    #[test]
    fn capture_preamble_targets_windows_console_interpreters_only() {
        for interpreter in
            ["bash", "node", "python", "kubectl", "/opt/bin/x", ""]
        {
            let wrapped = prepare_capture_command(interpreter, "echo hi");
            assert_eq!(
                wrapped, "echo hi",
                "{interpreter:?} must reach the child verbatim"
            );
        }
        assert!(
            matches!(
                prepare_capture_command("bash", "echo hi"),
                std::borrow::Cow::Borrowed("echo hi")
            ),
            "a non-console interpreter must not allocate a rewritten command"
        );

        #[cfg(windows)]
        for interpreter in [
            "cmd",
            "CMD",
            "cmd.exe",
            r"C:\Windows\System32\cmd.exe",
            "C:/Windows/System32/cmd.exe",
        ] {
            assert_eq!(
                prepare_capture_command(interpreter, "echo hi"),
                "chcp 65001 >nul & echo hi",
                "{interpreter}"
            );
        }
        #[cfg(windows)]
        for interpreter in [
            "powershell",
            "PowerShell",
            "pwsh",
            "pwsh.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
        ] {
            let wrapped = prepare_capture_command(interpreter, "echo hi");
            assert!(
                wrapped.starts_with("[Console]::OutputEncoding"),
                "{interpreter}: {wrapped:?}"
            );
            assert!(wrapped.ends_with("echo hi"), "{interpreter}: {wrapped:?}");
        }
    }

    /// Strict UTF-8 is the fast path on both hosts: it is what the PowerShell
    /// preamble above produces, and what every POSIX producer emits. Anything
    /// already correct must not be re-encoded.
    #[test]
    fn decoding_passes_utf8_through_unchanged() {
        for text in ["", "192.168.1.5\n", "中文 abc\n", "📋"] {
            assert_eq!(decode_captured(text.as_bytes()), text);
        }
    }

    /// POSIX has no code page to fall back to, so unreadable bytes become
    /// U+FFFD instead of an error or a panic.
    #[cfg(unix)]
    #[test]
    fn posix_decoding_is_lossy_not_failing() {
        assert_eq!(decode_captured(&[0xff, 0xfe]), "\u{fffd}\u{fffd}");
        assert_eq!(decode_captured(b"ok\xff"), "ok\u{fffd}");
    }

    /// The Windows half of `decode_captured` — the OEM code-page fallback — is
    /// proven end to end rather than by hard-coded bytes, because GBK bytes mean
    /// different characters on a Western install: see
    /// `crate::spawn::tests::captured_windows_output_survives_both_console_interpreters`,
    /// which runs real `cmd.exe`/`powershell.exe` and asserts the text survives.
    ///
    /// `cmd.exe` under any spelling a catalog or the table can produce.
    #[cfg(windows)]
    #[test]
    fn recognizes_cmd_in_every_spelling() {
        for exe in [
            "cmd",
            "CMD",
            "cmd.exe",
            "Cmd.EXE",
            r"C:\Windows\System32\cmd.exe",
            "C:/Windows/System32/cmd.exe",
        ] {
            assert!(is_cmd(exe), "{exe} must be recognized as cmd.exe");
        }
        for exe in [
            "powershell",
            "pwsh",
            "bash",
            "python",
            "node",
            "cmd2",
            "mycmd",
        ] {
            assert!(
                !is_cmd(exe),
                "{exe} must not be routed through the raw path"
            );
        }
    }

    /// The observable difference between Rust's MSVC quoting and letting
    /// `cmd.exe` parse the raw line. Both rows used to fail through the plugin:
    /// the first printed the escape characters, the second exited 1 with empty
    /// stdout, because `cmd.exe` has no `\"` escape.
    #[cfg(windows)]
    #[test]
    fn cmd_arguments_reach_the_shell_unquoted() {
        let run = |command: &str| -> (Option<i32>, String) {
            let mut cmd = Command::new("cmd");
            apply_command_args(
                &mut cmd,
                "cmd",
                &["/c".to_string(), command.to_string()],
            );
            let out = cmd.output().expect("spawn cmd");
            (
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            )
        };

        // `cmd`'s own `echo` prints the quotes it was given; what must not
        // appear is Rust's escaping of them. Before the fix this printed
        // `\"a b\"`.
        let (code, printed) = run(r#"echo "a b""#);
        assert_eq!(code, Some(0));
        assert_eq!(
            printed, r#""a b""#,
            "cmd must see the raw quotes, not Rust's backslash-escaped spelling"
        );
        assert!(
            !printed.contains('\\'),
            "no MSVC escape may reach cmd: {printed:?}"
        );

        let (code, printed) = run(r#"echo hi | findstr /C:"hi""#);
        assert_eq!(code, Some(0), "a quoted findstr argument must run");
        assert_eq!(printed, "hi");
    }

    /// The non-`cmd` interpreters keep Rust's quoting, which is what their
    /// MSVC runtimes expect — pin that the fix did not leak into them.
    #[cfg(windows)]
    #[test]
    fn non_cmd_interpreters_keep_msvc_quoting() {
        let mut cmd = Command::new("powershell");
        apply_command_args(
            &mut cmd,
            "powershell",
            &[
                "-NoProfile".to_string(),
                "-Command".to_string(),
                r#"echo "a b""#.to_string(),
            ],
        );
        let out = cmd.output().expect("spawn powershell");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "a b",
            "powershell receives the command through the CRT, so the same text behaves identically"
        );
    }
}
