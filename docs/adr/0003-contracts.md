# 0003: Schema-first Protobuf contracts

Status: Accepted (2026-11-12)

## Decision

- Every message, event and file format is defined once, in Protobuf, under `contracts/proto/tomeandtile/v1/`.
- Rust and TypeScript types are generated from those files. Nobody hand-writes a type that crosses a boundary.
- Export and import files exchanged with the other artifacts are the Protobuf JSON form of the same messages, so there is one source of truth for both the wire and the files.
- Compatibility rules:
  - Never renumber or reuse a field number. Remove a field by marking it `reserved`.
  - Adding a field or a message is allowed in `v1`.
  - A breaking change means a new package (`v2`) and a migration note.
  - Enum zero values are always `*_UNSPECIFIED`.
- CI runs `buf lint` and `buf breaking` against `main` on every PR that touches `contracts/`.
- A contract change is its own PR, approved by the contracts owner, merged before any code that uses it.

## Why

- With three people and three Claude instances, the contract is the only thing everyone shares. Generating types from it turns a drift between client and server into a compile error.
- Protobuf's field numbers give versioning rules that a tool can check.
- A compact binary wire format fits the speed goal.

## Rejected

- **Shared Rust types crate:** no protection on the TypeScript side.
- **JSON Schema:** readable, but larger on the wire and weaker compatibility tooling.
- **Protobuf plus JSON Schema:** two systems to keep in step.
- **Informal documentation:** breaks down immediately.
