# 0015: Where the Creature Cauldron import runs

Status: Accepted (2026-11-24)

Amends the dependency rule in [ARCHITECTURE.md](../../ARCHITECTURE.md) with a second named exception, next to the one in [0014](0014-asset-upload.md).

## Context

M3 includes importing monsters from Creature Cauldron ([0010](0010-integrations.md), [0013](0013-scope-and-schedule.md)). The pieces:

- `integrations/cauldron-import` (`tt-cauldron-import`) will be a pure library (issues 105 and 106): a Creature Cauldron export file in, a `Creature` and a list of notes about what could not be mapped out, or an error. Its field map is `integrations/cauldron-import/MAPPING.md`.
- Something has to receive the file, check that a GM sent it, give the creature a server id, and add it to the campaign so every client sees it.
- [0010](0010-integrations.md) says an importer "never talks to the live game". The importer has no access to the campaign, the socket or the event log.

The dependency rule says `integrations/` may depend only on `contracts/`. [0014](0014-asset-upload.md) recorded the first exception in the other direction: the server may use the asset pipeline. Using a second `integrations/` crate from the server needs its own decision.

## Decision

### 1. The server runs the importer as a library

`crates/server` may depend on `tt-asset-pipeline` ([0014](0014-asset-upload.md)) and `tt-cauldron-import`, and on no other crate under `integrations/`.

- The importer stays a pure function: export bytes in, a `Creature` and notes, or an error, out. It may not use the network, the disk, a clock, a database or global state. If it ever needs I/O, it loses this exception.
- `tools/check-deps.sh` enforces both halves:
  - rule 7 allows only `integrations/asset-pipeline` and `integrations/cauldron-import` as path dependencies of `crates/server`
  - rule 8 applies the same purity check to both crates: no `tokio`, `axum`, `hyper`, `reqwest`, `tower` or `async-std`, and no `tt-` crate other than `tt-contracts`. Parsing crates such as `serde` and `serde_json` are allowed.
- The ARCHITECTURE.md component table and "The dependency rule" list the exception.
- @aalemany00 owns the importer (issues 105 and 106). @Henry-theBinaryBuddies owns the endpoint (issue 103).

### 2. Import is an HTTP POST, authorised like the socket

- `POST /campaigns/{campaign_id}/imports`, body = the raw export file, `Authorization: Bearer <token>`.
- The token is checked with `auth::authenticate` before the body is read, as for asset upload, so both change together when real accounts arrive.
- **Only a GM of that campaign may import.** Anyone else gets 403.
- The body is capped at the importer's own size constant before it is read in full; the importer's other limits then run on the bytes.
- The reply is the encoded `ImportResult` (issue 95): the creature as stored, and the importer's notes.

**This is not security yet.** With `TT_DEV_AUTH=1`, anyone who can reach the server can claim to be the GM, exactly as with asset upload. The route must not be reachable from the internet until real sign-in exists.

### 3. The server, not the importer, adds the creature to the game

- The server mints `creature_id` (identifiers are minted by the server; the importer leaves it empty).
- It appends one `CreatureUpserted` event to the event log, so it reaches clients through the normal per-viewer filtering. Players do not receive a monster's sheet.
- **Re-import:** a file whose `provenance.external_id` matches a creature imported earlier replaces that creature and keeps its `creature_id`, so tokens already on the map stay linked.
- The importer emits nothing on the socket and never sees the campaign.
- Putting the creature on the map is a separate step (`CreateTokenIntent`, issue 95).

## Why

- **One process, one auth path.** The same reasons as 0014: the server already knows who is a GM.
- **The server checks what it stores.** The creature comes from the server's own run of the importer, not from a browser that could send any `Creature` it likes.
- **The importer stays pure and keeps its owner.** Only the direction of one dependency is recorded, and its purity is checked in CI. The importer can be tested against sample files with no server running ([0010](0010-integrations.md)).

## Rejected

- **Compiling the importer to WASM and running it in the browser:** more moving parts (a second WASM package and its build), and the server would have to accept a `Creature` built by the client and trust it.
- **A separate import service:** a second process to deploy and run, with its own copy of sign-in. Too much for the class timeline, for no gain at this size.
- **Importing over the WebSocket:** the socket's message limit is 64 KiB, and the socket protocol has no file transfer. HTTP already does this.

## Follow-up issues

| Piece | Owner | Issue |
|---|---|---|
| Contract: `ImportResult`, `CreateTokenIntent` | @Henry-theBinaryBuddies | 95 |
| Server: the import endpoint | @Henry-theBinaryBuddies | 103 |
| Importer: stat block, then actions to abilities | @aalemany00 | 105, 106 |
| Web: import a file, put creatures on the map | @Danny-Defeato | 111 |
