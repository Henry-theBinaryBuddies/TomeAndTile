# integrations/ — rules for Claude

Owner: @aalemany00. This file is maintained by @Henry-theBinaryBuddies; do not edit it.

You are working for @aalemany00. You may create and edit files under `integrations/` and `web/media/` (which has its own `CLAUDE.md`), plus `docs/devlog/aalemany00.md`. Nothing else.

## What lives here

- **Importers.** One directory per source: `cauldron-import/` (Creature Cauldron monsters), later Scenic Sorcerer maps, Universal VTT maps and others.
- **The asset pipeline.** Validating, resizing and uploading map and token images to object storage.

## Rules

- **An importer is a pure function:** a foreign file's bytes in, contract messages out (or a clear error). No network, no database, no global state, no clock.
- An importer depends on the generated contract types and on parsing libraries. It must never depend on `crates/server`, `crates/rules`, `crates/renderer` or `crates/plugin-host`. CI checks this.
- **Never guess a missing field.** If the source file lacks something the contract requires, return an error or leave the field unset and report it; do not invent a value.
- **Never trust the input.** An import file comes from a stranger. Reject oversized files, cap nesting and counts, and never execute or evaluate anything from the file.
- Every importer has tests against real sample files kept in its own `tests/samples/` directory. A new field mapping means a new or updated sample.
- Set `Provenance.source` on everything imported ("creature-cauldron", "user", ...).
- If the contract cannot express something the source has, stop and file a "Contract change request" issue. Do not stash it in a free-text field.
- To learn another artifact's export format, ask @Henry-theBinaryBuddies for a sample file. Do not edit the other artifact.

## When you must stop and ask

- The task needs a file outside `integrations/` or `web/media/`.
- The task needs a new or changed contract message.
- The task needs a new dependency.

## Checks

```bash
cargo fmt --all --check
```

```bash
cargo clippy -p tt-cauldron-import --all-targets -- -D warnings
```

```bash
cargo test -p tt-cauldron-import
```

```bash
bash tools/check-deps.sh
```
