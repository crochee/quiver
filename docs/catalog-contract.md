# Wox catalog contract

The `ShellCommands.json` file is the single user-facing configuration surface of Quiver.
This document is the **authoritative contract**: field semantics, placeholders, the
`capture` / `silent` flags, and the interpreter dispatch table. Reading this once is enough
to author a catalog without consulting the source.

For installation and end-user usage see [`wox-install.md`](wox-install.md); this file is
about the catalog's data shape only.

## §1. File location

Resolution order (highest priority first):

1. **`QUIVER_PATH`** — full path to the JSON file. Useful for shared NFS homes,
   per-project config repos, or CI fixtures.
2. **`WOX_DIRECTORY_USER_DATA`** — directory Wox itself uses; Quiver joins
   `ShellCommands.json`. This is the path Wox points to at runtime.
3. **Platform default** — `$HOME/.wox/wox-user/ShellCommands.json` (POSIX) /
   `%USERPROFILE%\.wox\wox-user\ShellCommands.json` (Windows).

Empty `QUIVER_PATH=""` is treated as unset and falls through to (2).

## §2. Top-level shape

```json
{
  "version": 1,
  "defaultInterpreter@windows": "powershell",
  "defaultInterpreter@darwin":  "bash",
  "defaultInterpreter@linux":   "bash",
  "defaultWorkingDirectory@linux": "~/code",
  "commands": [
    { "alias": "...", "command": "...", "interpreter": "..." }
  ]
}
```

| Top-level key | Type | Required | Purpose |
| :--- | :--- | :--- | :--- |
| `version` | int | yes | Schema version. Currently always `1`. |
| `defaultInterpreter@<os>` | string | no | Per-OS default interpreter; `<os>` ∈ `{windows, darwin, linux}`. Used when a command doesn't override `interpreter`. |
| `defaultWorkingDirectory@<os>` | string | no | Per-OS default cwd; relative paths resolve against **home** (`~/`-aware). Used when a command doesn't override `workingDirectory`. |
| `commands` | array | yes | The actual alias list; see §3. |

`<os>`-suffixed keys let one catalog file ship across heterogeneous machines (e.g. the
same file on a Linux box and a macOS laptop) and pick the right default per host.

## §3. Command entry

A `commands` array element is one alias. All fields are optional except `alias` and
`command`.

| Field | Type | Default | Behaviour |
| :--- | :--- | :--- | :--- |
| `alias` | string | **required** | Trigger name; case-insensitive; duplicates — first wins, the rest are skipped with a `WARN` line. Empty / non-string entries are skipped with a `WARN`. |
| `command` | string | **required** | Command text. Empty / non-string entries are skipped with a `WARN`. |
| `interpreter` | string | OS default | See §4. Anything outside the table is treated as an opaque executable and dispatched verbatim. |
| `workingDirectory` | string | inherit | Child process cwd. Relative paths resolve against **home** (`~/`-aware). Missing directory → `WARN` and falls back to inheritance. |
| `silent` | bool | `false` | If `true`, the launcher hides itself on Enter (the command still runs detached — `silent` only governs UI). |
| `capture` | bool | `false` | If `true`, the command runs **synchronously on query** (not on Enter). See §5. |
| `enabled` | bool | `true` | If `false`, the entry is hidden from results. Note `false` must be the JSON boolean, not the string `"false"`. |
| `tags` | string[] | `[]` | Discovery aids; tag hits always rank below alias hits. |
| `description` | string | the command itself | Used as the result-row subtitle. |

`{query}`, `$@`, `$N` placeholders inside `command` are substituted at query / Enter
time — see §6.

## §4. Interpreter dispatch

Each `interpreter` value goes through a compile-time table that maps it to (a) a real
executable and (b) the flag style used to deliver the command text. Everything outside the
table is dispatched verbatim as `<interpreter> <command...>`.

### POSIX (`cfg(unix)`)

| `interpreter` | Executable | Flag |
| :--- | :--- | :--- |
| (unset) | `sh -c` | the shell |
| `sh` / `bash` / `zsh` | `<shell> -c` | `-c` |
| `python` / `python3` | `python3` | `-c` |
| `node` | `node` | `-e` |
| anything else | treated as an absolute or PATH-relative executable name | appended verbatim |

### Windows (`cfg(windows)`)

| `interpreter` | Executable | Flag |
| :--- | :--- | :--- |
| (unset) | `cmd /c` | the shell |
| `cmd` | `cmd` | `/c` |
| `powershell` | `powershell` | `-NoProfile -Command` |
| `pwsh` | `pwsh` | `-NoProfile -Command` |
| `bash` | `bash` | `-c` |
| `python` / `python3` | `python` / `python3` | `-c` |
| `node` | `node` | `-e` |
| anything else | treated as an executable name | appended verbatim |

## §5. `capture` and `silent` semantics

These are the two non-obvious flags. Both affect lifecycle; neither affects whether the
command actually runs.

**`capture: true`** — runs the command **at query time**, not Enter time. The result is
the right-side preview; the primary action (Enter) becomes "copy the first line of stdout
to clipboard". This is the only moment a script plugin can show output to the user.

Constraints:

- The command **must** return within the Wox `WOX_SCRIPT_EXECUTION_TIMEOUT` (default 10 s).
  Quiver additionally enforces an 8 s local deadline; over that, the child is killed
  (`killpg` on POSIX, terminating the whole process group, so shell pipelines don't leak
  grandchild processes holding the pipes open).
- Slow commands belong on `silent: true`, not `capture: true`.
- Capture runs against the **resolved interpreter's table entry**; `capture` only fires
  for entries whose interpreter is one of `sh`/`bash`/`zsh`/`powershell`/`pwsh` (POSIX /
  Windows defaultisers). Setting `capture: true` with `interpreter: "cmd"` or any opaque
  binary is silently skipped — that combination is the "other platform's shell" case.

**`silent: true`** — only decides whether the launcher hides itself after Enter. Execution
is **always detached** (fire-and-forget) regardless of `silent`. The command always finishes
on its own clock; the launcher's lifetime does not gate it.

## §6. Placeholders

Substituted at two different moments:

| Placeholder | When | Expands to |
| :--- | :--- | :--- |
| `{query}` | **query time** — when the typed text starts with the alias (prefix match) | everything after the alias, **raw**, including spaces, **unescaped** |
| `$@` | **query time** — only on exact alias match | all positional args, joined by single spaces |
| `$1` … `$9` | **query time** — only on exact alias match | the Nth positional arg; out-of-range → empty string |
| `${1}` … `${N}` | **never substituted** | passes through to the inner shell as its own positional args |

`$@` / `$N` require exact alias match; partial prefix matches get `{query}` only.

Substitution is **pure textual replacement** (same surface as upstream Custom Commands).
Arguments containing quotes or shell metacharacters are inserted as-is — the catalog
author is responsible for adding their own quoting when shell semantics are needed:

```json
{ "alias": "upper",
  "command": "sh -c 'python3 -c \"import sys; print(sys.argv[1].upper())\" \"${1}\"' sh \"$1\"" }
```

`qv upper hello` → `HELLO`. The outer layer substitutes `$1` → `hello` (raw); the inner
shell sees `${1}` (passed through), then the trailing `"$1"` re-evaluates it under its own
rules.

## §7. Discovery / loader validation

`catalog::load` is the only entry point that reads the file. It is strict about types
and lenient about unknown fields:

| Input | Outcome |
| :--- | :--- |
| Missing `commands` array | Error envelope returned, no items. |
| `commands` element missing `alias` or `command` | Skipped with `WARN` line. |
| `alias` or `command` is the empty string or wrong type | Skipped with `WARN`. |
| Two entries with the same `alias` (case-insensitive) | First wins; the rest are skipped with `WARN`. |
| `enabled: "false"` (string instead of bool) | Treated as `true` — only JSON `false` hides the entry. |
| Unknown top-level or per-entry field | Ignored (forward-compat). |
| Malformed JSON / unreadable file | Error envelope, no items, `error` log line. |

All `WARN` lines go to stderr under the `catalog_load` target; set
`QUIVER_LOG=debug` to also see path-discovery details.

## §8. Cross-platform dispatch invariants

Three invariants the binary must hold — verifiable mechanically with `strings` /
`objdump`:

1. A POSIX build (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`) contains
   **no** occurrence of the strings `powershell`, `cmd.exe`, `WOX_SCRIPT_EXECUTION_TIMEOUT`
   `WOX_DIRECTORY_USER_DATA`-Windows paths, or backslash-as-path-separator.
2. A Windows build (`x86_64-pc-windows-gnu`) contains **no** occurrence of `bash`,
   `/bin/sh`, `killpg`, or `cfg(unix)`-only paths.
3. Every build's `--version` line (see `--help`) prints the embedded commit SHA.

The platform knowledge lives in **one** place — `src/platform.rs` — so this surface is
audited by reading that file, not by grepping the rest of the crate.