# contracts

The single source of truth for every message and file format in Tome & Tile (see [ADR 0003](../docs/adr/0003-contracts.md)). Types are generated; nobody hand-writes one that crosses a boundary.

## Layout

| Path | What |
|---|---|
| `proto/tomeandtile/v1/` | The Protobuf definitions (package `tomeandtile.v1`). Edit these only per `CLAUDE.md`. |
| `buf.yaml`, `buf.gen.yaml` | Buf lint/breaking rules and the TypeScript generation template. |
| `rust/` | Crate `tt-contracts`. `build.rs` compiles the protos in pure Rust (`protox`, no `protoc` needed) and generates `prost` types plus Protobuf-JSON `serde` impls into `OUT_DIR`. Nothing generated is committed. |
| `ts/` | npm package `@tomeandtile/contracts`. Generated files in `ts/src/gen` ARE committed; `ts/src/index.ts` re-exports them. |

## Regenerating

Rust regenerates on every `cargo build` when a proto file changes. TypeScript:

```bash
cd contracts/ts
npm ci
npm run generate
npm run check
```

(On Windows PowerShell use `npm.cmd`.) Commit the changes under `ts/src/gen` in the same PR as the `.proto` change. CI regenerates and fails if the committed files are stale.

## Using the types

- Rust: add `tt-contracts = { path = "../../contracts/rust" }` (adjust the path), then `use tt_contracts::v1::Creature;`. Messages implement `prost::Message` (binary) and `serde` in the standard Protobuf JSON form (camelCase names, enums as strings), which is the import/export file format.
- TypeScript: depend on `@tomeandtile/contracts` by file path (`"file:../contracts/ts"`), then `import { CreatureSchema, type Creature } from "@tomeandtile/contracts"`. Use `toBinary`, `fromBinary`, `toJson` and `fromJson` from `@bufbuild/protobuf`.
