# Quiver — installation & usage guide

> 30-second version: install Wox → put the `quiver` binary and stub into the Wox plugin
> directory → write one alias in `~/.wox/wox-user/ShellCommands.json` → press
> `Alt+Space`, type `qv`, hit Enter. The full guide walks through that path in order;
> skip to the troubleshooting table if you're stuck.

## What Quiver is

A **native script plugin** for the [Wox](https://github.com/Wox-launcher/Wox) launcher:
each alias in `ShellCommands.json` becomes an arrow — nock with a keyword, loose with
Enter. Single Rust binary, no Python / Node.js runtime. Wox `exec`s it directly per
keypress: stdin receives one JSON-RPC request, stdout emits one response.

- Trigger keywords: **`qv`** or **`quiver`**
- Repository: <https://github.com/crochee/quiver>
- Full catalog contract (`ShellCommands.json` field reference): see
  [`catalog-contract.md`](catalog-contract.md)

## Prerequisites

| Component | Requirement | Notes |
| :--- | :--- | :--- |
| Wox | ≥ 2.0 | Script plugin runtime; 2.4.x confirmed working |
| Operating system | Windows / Linux / macOS | Binary is platform-specific; Windows is the primary target |
| `quiver` binary | See *Installation* below | Single file, no runtime deps |
| Stub file | See *Installation* below | Wox discovers the plugin by parsing its leading comment block |

## Installation

### Option A — pre-built release (any machine)

1. **Get the binary** from the GitHub release page matching your host:
   - Windows: `quiver-x86_64-pc-windows-gnu.exe`
   - Linux: `quiver-x86_64-unknown-linux-gnu`
   - macOS (Apple Silicon): `quiver-aarch64-apple-darwin`

   Or build from source (Option B).

2. **Place the binary**:

   | Platform | Location | Notes |
   | :--- | :--- | :--- |
   | Windows | `~\.wox\wox-user\plugins\scripts\` (same directory as the stub) | Same basename as the stub: `quiver.exe`. Windows' `PATHEXT` resolves the extension-less stub to the sibling `.exe`. |
   | Linux / macOS | `~/.local/bin/quiver` (must be on `PATH`) | The stub's shebang resolves the interpreter by basename, so `PATH` lookup is the only way. |

3. **Render the stub** (Wox discovers the plugin by parsing the leading `#`/`//` comment
   block as JSON; the stub renderer writes exactly that). The `--path` flag
   flips the destination to a file; everything else is stdout. The stub
   layout is picked at compile time (POSIX on Linux/macOS, Windows on
   Windows) — no layout parameter is needed. The write forms do `mkdir -p`
   + atomic rename + `chmod 0o755` so no shell glue is needed:

   ```sh
   quiver stub                         # render host's stub to stdout
   quiver stub --path                  # write host's stub to the platform's
                                       # default destination:
                                       #   Windows : %USERPROFILE%\.wox\wox-user\plugins\scripts\quiver
                                       #   Linux  : $HOME/.wox/wox-user/plugins/scripts/quiver.sh
                                       #   macOS  : $HOME/.wox/wox-user/plugins/scripts/quiver.sh
   quiver stub --path <file>           # write host's stub to <file>
   ```

4. **Reload**: Wox uses fsnotify to watch the plugin directory — drop the files in and
   the plugin shows up immediately. If it doesn't, restart Wox once.

### Option B — build from source

```sh
git clone https://github.com/crochee/quiver
cd quiver
make                # WSL: docker cross-build of Windows PE
# or, on a host with rust installed:
make build
```

Then place the binary + render the stub as in Option A, steps 2-3.

WSL uses `make windows`, which builds inside a Docker image — no host Rust toolchain or
mingw-w64 linker required; only docker with buildx (Docker 23+ ships it). On WSL the
default goal is the Windows PE because that's the binary Wox loads. On native Linux /
macOS the default goal is the host-native binary; cross-builds are still one `make
windows` away.

### Verify the install

```sh
echo '{"jsonrpc":"2.0","id":1,"method":"query","params":{"search":"now"}}' \
  | quiver          # should print a JSON envelope containing "result":{"items":[...]}
```

Press `Alt+Space`, type `qv` — the quiver icon and your sample entries confirm the
plugin loaded.

## First alias (3-line starter)

Edit `~/.wox/wox-user/ShellCommands.json`:

```json
{
  "commands": [
    { "alias": "yt", "command": "xdg-open \"https://youtube.com/results?search_query={query}\"" }
  ]
}
```

Save → `Alt+Space` → `yt cats` → Enter. That's the whole loop.

## Field cheat-sheet

For the full contract (defaults, edge cases, dispatcher rules, loader validation,
cross-build invariants) see [`catalog-contract.md`](catalog-contract.md). Quick
reference:

| Field | Type | Default | Effect |
| :--- | :--- | :--- | :--- |
| `alias` | string | required | trigger name; case-insensitive; duplicates — first wins |
| `command` | string | required | command text; empty / non-string entries skipped with `WARN` |
| `interpreter` | string | OS default | `bash`/`sh`/`zsh` (POSIX), `powershell`/`cmd`/`bash`/`python`/`node` (Windows); anything else dispatched as opaque executable |
| `workingDirectory` | string | inherit | child cwd; relative paths resolve against **home**; missing dir → `WARN`, inherit |
| `silent` | bool | `false` | `true` = launcher hides itself on Enter (command still runs detached) |
| `capture` | bool | `false` | `true` = run synchronously on query, output → preview, Enter = copy first line of stdout |
| `enabled` | bool | `true` | `false` = hidden (note: must be JSON `false`, not string `"false"`) |
| `tags` | string[] | `[]` | discovery aid; tag hits always rank below alias hits |
| `description` | string | the command itself | result-row subtitle |

Top-level defaults: `defaultInterpreter(@windows/@darwin/@linux)`,
`defaultWorkingDirectory(同后缀)` — OS-suffixed keys win, so a single catalog file works
across heterogeneous machines.

## Placeholders

| Form | When | Expands to |
| :--- | :--- | :--- |
| `{query}` | query time, on alias prefix match | everything after the alias, **raw** (spaces preserved, **unescaped**) |
| `$@` | query time, on exact alias match | all positional args, joined by single spaces |
| `$1`…`$9` | query time, on exact alias match | the Nth positional arg; out-of-range → empty string |
| `${1}`…`${N}` | **never substituted** | passed through to the inner shell as its own positional args |

**Inner-shell handoff idiom** (outer substitution eats bare `$N`, so brace it to slip
past, then re-introduce it with bare `$1` so the inner shell binds it as its own
positional):

```json
{ "alias": "upper",
  "command": "sh -c 'python3 -c \"import sys; print(sys.argv[1].upper())\" \"${1}\"' sh \"$1\"" }
```

`qv upper hello` → `HELLO`.

Substitution is **pure textual replacement** — same injection surface as upstream Wox
Custom Commands. Arguments carrying quotes / metacharacters enter the command verbatim.
Add your own quoting in `command` when shell semantics matter.

## `capture` and `silent`

- **`capture: true`** — runs the command **at query time**, not Enter time. The result is
  the right-side preview; the primary action (Enter) becomes "copy the first line of
  stdout to clipboard". This is the only moment a script plugin can show output to the
  user.
  - Must return within Wox's `WOX_SCRIPT_EXECUTION_TIMEOUT` (default 10 s). Quiver
    enforces an additional 8 s local deadline with `killpg` cleanup.
  - Slow commands belong on `silent: true`, not `capture: true`.
- **`silent: true`** — only decides whether the launcher hides itself after Enter.
  Execution is **always detached** (fire-and-forget); the command always finishes on its
  own clock, independent of the launcher's lifetime.

## Logging & troubleshooting

Logs flow through `tracing` → stderr, which Wox captures into its own log file. Default
is silent (only `error`):

```sh
QUIVER_LOG=info quiver       # info / warn / error
QUIVER_LOG=debug quiver      # everything (per-query, catalog path discovery)
QUIVER_LOG=quiver=debug,info # target-qualified
RUST_LOG=...                 # compatibility alias when QUIVER_LOG is unset
```

A typo'd directive never silently swallows output — every directive is floored at
`error` (`errors always loud`).

| Symptom | First check |
| :--- | :--- |
| `qv` shows nothing | catalog exists and parses? (`QUIVER_LOG=debug` logs `catalog_load path`); missing `commands` array → "catalog unavailable" error line; set `QUIVER_PATH=/path/to/file.json` to override |
| Entry doesn't appear | `enabled` is JSON `false` (not string `"false"`)? alias duplicated (first wins)? |
| Enter does nothing | `command` empty (skipped at load)? `interpreter` exists on this platform? |
| Preview is garbled | Windows `cmd` output goes through OEM-codepage fallback; `chcp 65001` inside the command if you need UTF-8 |
| Want to confirm what ran | `QUIVER_LOG=debug` → look for `query received` / `spawn detached` |

Offline self-check (no Wox required): `make smoke` — 47 assertions covering stub /
protocol / capture / action / logging end-to-end.

## Catalog path resolution

Same as [`catalog-contract.md §1`](catalog-contract.md#1-file-location):

1. `QUIVER_PATH` — full file path, highest priority
2. `WOX_DIRECTORY_USER_DATA` — directory; Quiver joins `ShellCommands.json` (Wox's runtime path)
3. Platform default — `$HOME/.wox/wox-user/ShellCommands.json` (POSIX) / Windows equivalent

Empty `QUIVER_PATH=""` is treated as unset.

## Extending Quiver

- **Add a command**: edit `ShellCommands.json` only (data-driven; no rebuild).
- **Move to a new machine**: same catalog file; per-OS differences go in the
  `@windows/@darwin/@linux` suffix keys.
- **Add or change an interpreter**: `src/platform.rs`'s `INTERPRETERS` table is the
  single extension point — one-file review for new platforms / interpreters. See
  [`CONTRIBUTING.md`](../CONTRIBUTING.md).

## Uninstall

Delete the stub (`quiver` / `quiver.sh`) and binary from the plugin directory, and
`~/.wox/wox-user/ShellCommands.json` (if no longer needed). Wox forgets the plugin on
its next directory scan.