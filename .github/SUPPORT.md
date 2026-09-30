# Support

## Where to ask

| Topic | Where |
| :--- | :--- |
| **Bug reports** | [GitHub issue](../../issues/new?template=bug_report.md) — fill the bug-report template (Wox version, Quiver version, reproduction). |
| **Feature requests** | [GitHub issue](../../issues/new?template=feature_request.md) — focus on the launcher habit you want, not the implementation. |
| **Security vulnerabilities** | [`SECURITY.md`](../SECURITY.md) — **do not** use a public issue. Use GitHub's private advisory channel. |
| **General questions, how-tos, "is this expected?"** | [GitHub Discussions](../../discussions) — `Q&A` category. |
| **Upstream Wox behaviour** | [Wox issue tracker](https://github.com/Wox-launcher/Wox/issues) — Quiver only consumes Wox's ScriptHost contract. |

## Before filing a bug

1. **Upgrade first.** The latest release tag may already fix it.
2. **Reproduce with logs.** `QUIVER_LOG=debug quiver 2>log.txt` then
   attach `log.txt`. Wox captures stderr into its own log file if you
   can't run the binary standalone.
3. **Check the smoke harness.** `make smoke` runs the full JSON-RPC
   surface against the shipped sample catalog; if it passes on your
   machine, the bug is in your catalog or environment, not the plugin.
4. **Search closed issues.** Many "bugs" are documented Wox behaviours
   pinned in [`docs/catalog-contract.md`](../docs/catalog-contract.md).

## What to expect

This is a one-maintainer project. Issues are triaged as time permits.
Pull requests with tests are prioritised — see
[`CONTRIBUTING.md`](../CONTRIBUTING.md) for the dev loop and PR
checklist.