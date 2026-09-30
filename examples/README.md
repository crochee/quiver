# examples/

Offline samples — no Wox needed, used to:

- see what a catalog looks like (`ShellCommands.json`, 7 minimal demonstration aliases)
- exercise the JSON-RPC protocol without installing a launcher (`quiver-smoke.sh`)

## Run it

```sh
cd quiver
make build                                                # produces target/release/quiver(.exe)
WOX_QUIVER_EXE=target/release/quiver ./examples/quiver-smoke.sh
# or just: make smoke (builds + runs automatically)
```

No Wox, no Node, no extra configuration. The only dependencies are `python3` (the
harness uses it as a JSON-field extractor) and the commands the demo entries themselves
invoke (`ip` / `sed` / `touch`) — the harness fails loud on a missing dep instead of
returning a cryptic want/got diff.

It points `WOX_DIRECTORY_USER_DATA` at `examples/`, so the plugin reads
`examples/ShellCommands.json` instead of the user's real catalog.

## Contract surface demonstrated

| Field / behaviour | Alias demonstrating it |
|---|---|
| `{query}` placeholder | `echo` |
| `interpreter: bash` + `capture: true` (Linux) | `ip`, `now` |
| `interpreter: powershell` + `capture: true` (Windows-only) | `ip-win` |
| `silent: true` (launcher hides on Enter; execution is **always** detached — `silent` only governs UI) | `wk` |
| `$@` (all args, joined by spaces) | `k` |
| `$N` and `${N}` inner-shell escape | `upper` |
| `workingDirectory` field | — (not in the sample; see [`docs/catalog-contract.md §3`](../docs/catalog-contract.md#3-command-entry)) |

## Don't

Don't copy `examples/ShellCommands.json` straight into `~/.wox/wox-user/`. This is the
**smoke** catalog — names like `echo` / `upper` / `now` will collide with your real
aliases. Author your own catalog against
[`docs/catalog-contract.md`](../docs/catalog-contract.md).

## Extend it

Copy `ShellCommands.json`, add entries to its `commands` array, re-run
`quiver-smoke.sh` (the plugin reads only `ShellCommands.json` from the directory —
other JSON files in the same directory are ignored). Case-insensitive duplicate aliases
cause `catalog::load()` to **skip the later ones** with a `WARN` line.

### Point at a specific catalog file (skip directory join)

If your catalog isn't under `WOX_DIRECTORY_USER_DATA` (shared NFS homes, per-project
config repos, CI fixtures), bypass the directory join by giving the full path:

```sh
QUIVER_PATH=/path/to/my.json examples/quiver-smoke.sh
```

This path takes priority over `WOX_DIRECTORY_USER_DATA` and the platform default. Empty
string is treated as unset.

After editing, run `cargo test --release` to confirm the catalog loader hasn't regressed
— ranking and default-key parsing live in `src/catalog.rs::tests`; **the capture gate**
and action-path assertions live in `src/protocol.rs::tests` (the
`capture_query_runs_synchronously_and_binds_enter_to_clipboard` family runs real
subprocesses end-to-end); loader validation (duplicate alias / empty command / wrong-
type field / missing `commands` array) lives in `catalog::tests::loader`.

## CI / Makefile integration

The script's only non-bash dep is `python3`. `make smoke` is wired up: it builds the
host-native binary first, then runs the harness. Cross-built Windows PEs are
**not** auto-selected — running them through WSL interop would mean they can't read
the POSIX path in `WOX_DIRECTORY_USER_DATA`.