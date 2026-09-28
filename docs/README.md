# docs/

Single-source-of-truth rule: the **Wox catalog contract** — `ShellCommands.json` field
semantics, `{query}` / `$@` / `$N` placeholders, `capture` / `silent` behaviour, interpreter
dispatch table, cross-build stub layout — lives **once** in the dotfiles repo:

> [`~/.dotfiles/docs/wox/README.md`](https://github.com/crochee/dotfiles/blob/main/docs/wox/README.md)

This directory is reserved for crate-local material (CI snippets, contributor notes,
proposals). Anything catalog-contract-shaped belongs in the upstream doc, not here, so the
two surfaces cannot drift.

## Crate-local docs

| File | Purpose |
| :--- | :--- |
| [`wox-install.md`](wox-install.md) | **Installation & usage guide** — how to put quiver into Wox, author the first alias, field cheat-sheet, troubleshooting. End-user facing; overlaps nothing in the upstream contract doc. |
| _add here_ | Further crate-local material (design notes for a new `platform` branch, proposals). Cross-reference the upstream doc for anything that overlaps. |