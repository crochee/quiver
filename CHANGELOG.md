# Changelog

All notable changes to Quiver are recorded here. Dates are UTC; versions
follow [semver](https://semver.org/) with the 0.x caveat below. Each
release's artefacts are published on
[GitHub Releases](https://github.com/crochee/quiver/releases) with
SLSA-attested provenance.

The format is [Keep a Changelog](https://keepachangelog.com/) 1.1.
Categories: **Added** / **Changed** / **Fixed** / **Removed** /
**Deprecated** / **Security**.

## [Unreleased]

## [0.4.0] — 2026-10-01

MINOR bump — `--stub` is now the `stub` subcommand (`quiver stub [--path
[<file>]]`), parsed by `clap` instead of a hand-rolled `argv` scan. The
old flag-style invocation (`quiver --stub ...`) is gone.

### Added
- `clap` v4 dependency (minimal: `derive` + `std` + `help` + `usage` +
  `error-context`). Replaces the hand-rolled `argv` parse in `main.rs`;
  the `stub` subcommand and its `--path` flag are now declarative.
- `quiver --help` / `-h` prints the auto-generated usage block —
  `quiver stub --help` is the canonical reference for the three
  terminal forms (`stub`, `stub --path`, `stub --path <p>`).

### Changed (breaking)
- The CLI surface is now subcommand-based. Old → new:
  - `quiver --stub`                  → `quiver stub`
  - `quiver --stub --path`           → `quiver stub --path`
  - `quiver --stub --path <file>`    → `quiver stub --path <file>`
  Old form rejected by clap with a clear "unrecognized argument" error.
  Catalog format and the JSON-RPC wire protocol are unchanged.

## [0.3.1] — 2026-10-01

PATCH bump — `--stub` CLI gains an optional `--path` flag with three
terminal forms, replacing the previous positional `<layout>` / `<path>`
arguments. No wire / runtime / catalog-format change.

### Added
- `quiver --stub --path [<file>]`:
  - `quiver --stub`             → host layout (compile-time POSIX on
    Linux/macOS, Windows on Windows) to stdout
  - `quiver --stub --path`      → host layout to the platform's default
    destination (`~/.wox/wox-user/plugins/scripts/...` under `home_dir()`)
  - `quiver --stub --path <p>`  → host layout to `<p>`

  All write forms do `mkdir -p` + atomic rename + `chmod 0o755` so the
  install hook collapses from
  `quiver --stub <layout> > $tmp; cmp; cp; chmod` to a single
  `quiver --stub --path` (or `quiver --stub --path <file>`). The
  previous positional `<layout>` argument is gone: the platform is
  compile-time known.

  When `--path` is present but the home variable is missing (container /
  CI runner), the binary logs a `warn` and falls back to stdout so the
  operator still gets the bytes rather than an opaque JSON-RPC error.

## [0.3.0] — 2026-09-30

MINOR bump — new public CLI (`--version`) and new stub field (`Build`),
both additive. No breaking changes; old Wox still loads the new stub
(ignores unknown fields), and old `ShellCommands.json` works with the
new binary unchanged.

### Added
- `--version` / `-V` flag prints the baked-in version, git commit, cargo
  profile, and UTC timestamp — so two artefacts can be diffed without
  computing md5s. Resolves the previous "no way to compare two binaries"
  complaint.
- `Build` field on the Wox discovery stub carries the same commit +
  profile + timestamp; `Build` is omitted when no commit is discoverable
  so dev builds don't smuggle `"unknown"` into the Wox UI.
- `cargo test` now loads `examples/ShellCommands.json` and pins four
  smoke-harness dependency alias names + at least one `capture: true`
  entry — typos in the shipped sample trip a unit test before the smoke
  step.
- `.github/CODEOWNERS` declares file ownership for CI, the Dockerfile,
  `build.rs`, and the release policy.
- Per-job `contents: write` scope on `release.yml` (was workflow-wide).
- `concurrency: cancel-in-progress` on `ci.yml` PR runs (saves CI minutes).
- SLSA Build Level 2 attestation via
  `actions/attest-build-provenance@v2` — every release artefact carries
  GitHub's "Verified" badge and is verifiable with `gh attestation verify`.

### Changed
- Repository URL corrected from `crochee/dotfiles` (parent repo) to
  `crochee/quiver` — `Cargo.toml` `repository` / `homepage`,
  `src/identity.rs::WEBSITE` (which is baked into the stub), and
  `SECURITY.md` advisory URL.
- All "single-source-of-truth" cross-references to `~/.dotfiles/docs/wox/README.md`
  replaced with in-repo `docs/catalog-contract.md` and
  `docs/wox-install.md` — repo is now self-contained.
- Dependabot groups minor/patch bumps into a single weekly PR to reduce
  noise; major bumps + first-time additions still land as dedicated PRs.

### Fixed
- `cargo build --release` failed when the `mod test_support` declaration
  was `#[cfg(test)]`-gated (latent — masked by `cargo test` cache).

## [0.2.1] — 2025-XX-XX

- Drop empty artifact directories before computing checksums
  (`sha256sum *` would otherwise error on `Is a directory`).

## [0.2.0] — 2025-XX-XX

- Ship a macOS arm64 artefact alongside the Windows PE and Linux ELF;
  release workflow now produces three platform artefacts per tag.

## [0.1.0] — 2025-XX-XX

- Initial public release: Wox script plugin that turns every alias in
  `ShellCommands.json` into a keyword-addressable shell one-liner, with
  `{query}` / `$@` / `$N` placeholder substitution, `capture: true`
  synchronous preview, and `silent: true` detached action.

---

[Unreleased]: <https://github.com/crochee/quiver/compare/v0.4.0...HEAD>
[0.4.0]: <https://github.com/crochee/quiver/compare/v0.3.1...v0.4.0>
[0.3.1]: <https://github.com/crochee/quiver/compare/v0.3.0...v0.3.1>
[0.3.0]: <https://github.com/crochee/quiver/compare/v0.2.1...v0.3.0>
[0.2.1]: <https://github.com/crochee/quiver/compare/v0.2.0...v0.2.1>
[0.2.0]: <https://github.com/crochee/quiver/compare/v0.1.0...v0.2.0>
[0.1.0]: <https://github.com/crochee/quiver/releases/tag/v0.1.0>