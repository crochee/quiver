# docs/

Everything catalog-contract-shaped and end-user-facing lives in this directory — the
repo is self-contained. Two entry points:

| File | Audience | Topic |
| :--- | :--- | :--- |
| [`wox-install.md`](wox-install.md) | End user | How to drop Quiver into Wox, author the first alias, field cheat-sheet, troubleshooting table. |
| [`catalog-contract.md`](catalog-contract.md) | Catalog author | Authoritative reference for `ShellCommands.json`: field semantics, `{query}` / `$@` / `$N` placeholders, `capture` / `silent`, interpreter dispatch table, loader validation rules, cross-build invariants. |

Anything that touches the on-disk format belongs in `catalog-contract.md` — that is the
single source. `wox-install.md` only describes how to put the binary where Wox expects
to find it; it does not redefine any field.