# quiver

[![CI](https://img.shields.io/github/actions/workflow/status/crochee/quiver/ci.yml?branch=master&label=CI&logo=github)](https://github.com/crochee/quiver/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/crochee/quiver?label=release&logo=github)](https://github.com/crochee/quiver/releases/latest)
[![MSRV](https://img.shields.io/badge/MSRV-1.91-blue?logo=rust)](https://blog.rust-lang.org/2025/05/Rust-1.91.0)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](../LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-orange?logo=rust)](https://doc.rust-lang.org/edition-guide/rust-2024/)

Quiver — a native script plugin for the [Wox](https://github.com/Wox-launcher/Wox)
launcher. Every alias in `ShellCommands.json` is an arrow: nock with a keyword, loose
with Enter.

Wox's ScriptHost `exec`s a Rust binary directly — no Python, no Node.js. JSON over
[`serde_json`](https://docs.rs/serde_json), logging over
[`tracing`](https://docs.rs/tracing) +
[`tracing-subscriber`](https://docs.rs/tracing-subscriber) (any directive floored at
`error` so a typo'd `QUIVER_LOG` never silences ERROR output — see `src/log.rs`),
feature surface kept deliberately minimal.

> **Installation & first alias**: see [`docs/wox-install.md`](docs/wox-install.md) —
> 3-line JSON to get going.
>
> **Catalog contract** (`ShellCommands.json` fields, placeholders, `capture` / `silent`
> semantics, interpreter dispatch table): see
> [`docs/catalog-contract.md`](docs/catalog-contract.md). This is the authoritative
> reference; this README only summarises.

## One-line principle

Every alias is data in the catalog; Quiver is just a JSON-RPC bridge between Wox and the
local shell. The Rust build produces two things: the Wox discovery stub (`--stub`) and
the actual binary. The stub is also rendered by the binary itself (`src/stub.rs` is the
single source), so no generated files are tracked.

```
quiver/
├── Cargo.toml / Cargo.lock         # bin name = quiver; deps = serde + serde_json + tracing + tracing-subscriber + fuzzy-matcher + shellexpand
├── rust-toolchain                  # local cargo toolchain pin (1.98.1, matches Dockerfile)
├── Dockerfile                      # Windows cross-build image (rust:1.98 + mingw-w64)
├── .dockerignore                   # build context = source only (no target/)
├── Makefile                        # manual build entry point
├── CONTRIBUTING.md / SECURITY.md / RELEASE.md   # CNCF-style maintenance entry points
├── .github/                        # CI/release workflows, issue/PR templates, dependabot
├── README.md                       # this file
├── LICENSE                         # MIT
├── rustfmt.toml / clippy.toml      # lint config (deny-by-default gates; see files)
├── docs/                           # wox-install.md + catalog-contract.md
├── examples/                       # offline-runnable sample catalog + smoke harness
│   ├── README.md
│   ├── ShellCommands.json
│   └── quiver-smoke.sh
└── src/                            # main / identity / fuzzy / platform
                                    # / catalog / protocol / spawn / stub / log
                                    # / test_support (cfg(test) only)
```

`src/` module responsibilities (mirrors the crate-level doc):

| Module | Responsibility |
|---|---|
| `main` | entry point: `--stub` branch or JSON-RPC `serve` |
| `identity` | plugin id / trigger keywords / icon (single source) |
| `catalog` | `ShellCommands.json` loader + ranker |
| `fuzzy` | `fuzzy-matcher::skim::SkimMatcherV2` wrapper (ASCII fallback to substring) |
| `protocol` | JSON-RPC wire types + query/action handlers |
| `platform` | compile-time-resolved, crate-wide single source of platform knowledge |
| `spawn` | interpreter dispatch + detached execution |
| `stub` | Wox discovery-metadata rendering |
| `log` | `tracing-subscriber` install + `EnvFilter` parsing (`QUIVER_LOG` / `RUST_LOG`) |
| `test_support` | (`cfg(test)`) crate-wide env lock + auto-restoring `setenv`/`unsetenv` guard (both are unsafe under edition 2024 and concurrent tests share process env) |

All platform knowledge lives in `src/platform.rs`. Every other module is platform-neutral
— no `cfg(unix)` / `cfg(windows)` / OS-specific strings outside that one file. The cross-
platform invariants are spelled out in
[`docs/catalog-contract.md §8`](docs/catalog-contract.md#8-cross-platform-dispatch-invariants).

## Build

Quiver is built **manually** with `make`; release artefacts flow out through CI tagged
releases (see `RELEASE.md`).

```sh
# WSL (kernel contains "microsoft"): default → docker cross-build of Windows PE
cd quiver
make

# native Linux / macOS / Git-Bash / MSYS: default → host cargo build
make

# explicit
make build                # host cargo build --release
make windows              # docker cross-build (no host rust/mingw required)
make windows-image        # rebuild the docker builder image only
make test                 # unit tests (host target)
make test-windows         # Windows-target tests (container-built → WSL interop run; WSL only)
make smoke                # offline smoke (examples/quiver-smoke.sh, no Wox needed)
make lint                 # rustfmt --check + clippy -D warnings
make fmt                  # apply rustfmt
make clean                # delete target/
make help                 # list all targets
```

On WSL, `make` = `make windows` = `docker build` + `docker run` that produces
`target/x86_64-pc-windows-gnu/release/quiver.exe`. The Dockerfile image ships its own
Rust toolchain + mingw-w64 linker — the host only needs **docker** with **buildx**
(Docker 23+ ships it).

On non-WSL, non-Windows hosts (native Linux / macOS / Git-Bash / MSYS), `make` runs
`cargo build --release` directly; the artefact is `quiver` (POSIX) or `quiver.exe`
(Windows).

## Toolchain pinning

Three bindings, one bump:

| Location | Meaning | Current |
| :--- | :--- | :--- |
| `rust-toolchain` (repo root) | local `cargo` toolchain | `1.98.1` |
| `Dockerfile` `ARG RUST_IMAGE` | container cross-build toolchain | `rust:1.98.1-slim-bookworm` |
| `Cargo.toml` `edition` | language edition | `2024` |
| `Cargo.toml` `rust-version` | **MSRV** for downstream consumers | `1.91` |

MSRV is 1.91 because the crate uses let-chains (`if let … && …`, stable since 1.88) on
top of edition 2024's 1.85 syntax floor; tracing / serde_json's actual dependency graph
is clean from 1.91 onward. Bumping the Rust toolchain = edit `rust-toolchain` +
`Dockerfile`'s `RUST_IMAGE` default (two lines, one review); the edition is bumped
separately on a major release. The patch number is **pinned**: the container ships
exactly `1.98.1`; bare `1.98` makes rustup redownload a full ~300 MB toolchain inside
the image.

## Logging

Quiver ships logs through `tracing` to stderr; Wox captures stderr into its own log
file. Default is silent:

- no `QUIVER_LOG` / `RUST_LOG` → `error` baseline; only `tracing::error!` reaches stderr
- `QUIVER_LOG=<level>` (`error` / `warn` / `info` / `debug`) → `EnvFilter` direct parse;
  target-qualified directives (`quiver=debug,other=info`) are accepted
- `RUST_LOG` is a compatibility alias: `QUIVER_LOG` wins, otherwise `RUST_LOG` is read
- every directive is **prefixed with `error`** (`error,<your-directive>`) so a typo
  like `QUIVER_LOG=infoo` never silently swallows ERROR — `errors always loud` (smoke
  §7e is the regression test)

Emission sites (edit-and-go):

| Site | level | fields |
| :--- | :--- | :--- |
| `catalog::load` success | `info` | `count`, `path` |
| `catalog::load` failure | `error` | `error` |
| `catalog::load` skip (empty alias / duplicate alias / empty command / wrong-type field) | `warn` | `path` / `alias` / `field` |
| `catalog::load` path discovery | `debug` | `path` (smoke §7c asserts this line) |
| `protocol::query` request received | `debug` | `search`, `alias` |
| `protocol::action` request received | `debug` | `action` |
| `protocol::action` detached spawn | `info` | `action`, `interpreter` |
| `stub::Layout::from_arg` unknown layout | `warn` | `got` |
| `main::serve` parse failure / stdin failure | `error` | `error` (parse) |
| `main::emit` stdout write failure / `--stub` write failure | `error` | — |

Sample:

```text
$ echo 'not json' | QUIVER_LOG= ./target/release/quiver 2>/tmp/se
2026-09-28T19:48:38.344263Z ERROR json parse failed error=invalid literal at byte 0
```

## Smoke (no Wox required)

The whole pipeline is stdin-receives-JSON-RPC / stdout-emits-JSON-RPC, so it's fully
runnable standalone. `examples/` ships a sample catalog and a one-shot smoke harness:

```sh
cd quiver
make smoke      # builds host-native binary + runs examples/quiver-smoke.sh (covers logging & action path)
```

or manually:

```sh
make build
EXE=target/release/quiver   # WSL: target/release/quiver (same name as cross PE; default goal is PE)

# 1. render the Wox discovery-metadata stub
"$EXE" --stub posix | head -6          # Linux/macOS shebang layout
"$EXE" --stub windows | head -6        # Windows PATHEXT layout

# 2. one-shot query: pipe in stdin, read stdout JSON
echo '{"jsonrpc":"2.0","id":1,"method":"query","params":{"triggerKeyword":"qv","search":"now"}}' \
  | WOX_DIRECTORY_USER_DATA="$(pwd)/examples" "$EXE"

# 3. one-shot: examples/quiver-smoke.sh does both, plus logging & action coverage
WOX_QUIVER_EXE="$EXE" examples/quiver-smoke.sh
```

Pointing `WOX_DIRECTORY_USER_DATA` at `examples/` makes `catalog::load()` read
`examples/ShellCommands.json` instead of the user-real catalog — that's the smoke path;
at deploy time Wox points the variable at `~/.wox/wox-user` (the built-in default).

Full catalog contract (`alias` / `command` / `interpreter` / `capture` / `silent` /
`{query}` / `$@` / `$N`) is in
[`docs/catalog-contract.md`](docs/catalog-contract.md).

## `--version`

Every release artefact embeds the version, the exact git commit it was built from, the
cargo profile, and a UTC timestamp. `quiver --version` (no stdin, no catalog, no
network) prints them on stdout:

```text
$ quiver --version
Quiver 0.2.1 5f6d2c96f028633ff2fb4b8c3b84d0b3f67e3a3a (release 2026-09-30T12:34:56Z)
```

Diffing two artefacts is now `grep` instead of `md5`. Source of truth, in priority
order: `QUIVER_GIT_SHA` env-var (CI passes the tag SHA explicitly) → `git rev-parse HEAD`
(local dev) → `"unknown"` (last-resort fallback, omitted from the stub's `Build`
field). See `build.rs` for the resolution logic.

## Known trade-offs (this crate's own concerns)

1. **One process per query**: Script-runtime model; inherent to Wox's fork-exec.
2. **One stdin read + one stdout write per query**: same — no keep-alive.
3. **POSIX layout requires `~/.local/bin` on `PATH`**: Wox resolves the interpreter by
   basename from the stub's shebang; only PATH lookup works.
4. **`{query}` / `$@` / `$N` are pure textual replacement, no shell escaping**: same
   surface as upstream Custom Commands; need real shell semantics, set
   `interpreter: bash` / `sh` / `zsh` and quote yourself.
5. **`capture: true` entries must return within Wox's `WOX_SCRIPT_EXECUTION_TIMEOUT`
   (default 10 s)**: Quiver also enforces an 8 s local deadline with `killpg` cleanup;
   slow commands belong on `silent: true` instead.

## Maintenance (CNCF style)

| Topic | Entry point |
|---|---|
| Contributing (dev loop / commit rules / DCO / PR checklist) | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| Security (reporting channel / scope / threat model) | [`SECURITY.md`](SECURITY.md) |
| Release (semver / tag / CI artefacts / checksums) | [`RELEASE.md`](RELEASE.md) |
| Changelog (per-version user-visible diffs) | [`CHANGELOG.md`](CHANGELOG.md) |
| Code of conduct | [`.github/CODE_OF_CONDUCT.md`](.github/CODE_OF_CONDUCT.md) |
| Support (where to ask questions vs file issues) | [`.github/SUPPORT.md`](.github/SUPPORT.md) |
| File ownership / review gates | [`.github/CODEOWNERS`](.github/CODEOWNERS) |
| Dependency updates | `.github/dependabot.yml` (weekly; MSRV gated by CI) |
| Templates | `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md` |
| Discussions (Q&A, show-and-tell) | [GitHub Discussions](../../discussions) |

CI (`.github/workflows/`) runs `fmt` + `clippy -D warnings` + tests + smoke + MSRV
(1.91) + cross-build on every PR; tag push auto-publishes release artefacts + sha256.