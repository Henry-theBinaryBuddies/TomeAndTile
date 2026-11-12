# crates/ — rules for Claude

Owner: @Henry-theBinaryBuddies. Only the owner's Claude edits this directory. If you are working for anyone else and need something here, stop and open an issue for the owner.

| Crate | Job | May depend on |
|---|---|---|
| `rules` | Pure 5e rules engine | contracts |
| `server` | Sessions, permissions, event log, filtering, video join tokens | contracts, `rules`, `plugin-host` |
| `renderer` | WASM/WebGPU map canvas | contracts |
| `plugin-host` | Sandboxed plugin runtime | contracts |

## Rules

- `rules` stays pure: no I/O, no clock, no network, no global state. Randomness comes from an injected roller. Every rule gets a deterministic test.
- `rules` and `renderer` must never depend on `server`.
- The server is the only place a die is rolled and the only place state changes.
- Every outgoing event passes through per-viewer filtering. Adding a field that players must not see means adding its filter and a test in the same PR.
- The renderer's WASM interface is a public API used by `web/`. Changing it needs a note in the PR and a heads-up issue for @Danny-Defeato.
- `unsafe` is forbidden workspace-wide.

## Checks

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

```bash
bash tools/check-deps.sh
```
