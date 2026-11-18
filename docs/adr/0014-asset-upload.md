# 0014: Where asset upload runs

Status: Accepted (2026-11-18)

Amends [0008](0008-data-assets-accounts.md) for the class demo only (where files are stored) and the dependency rule in [ARCHITECTURE.md](../../ARCHITECTURE.md) (one named exception).

## Context

M1 needs a GM to upload a map image and have it appear for every player. The pieces:

- `integrations/asset-pipeline` (`tt-asset-pipeline`, issue 33) is a pure library: `inspect(bytes) -> Result<AssetRef, AssetError>`. It checks the format from the file's own header bytes, enforces size, dimension and pixel caps, and computes `asset_id` as the SHA-256 of the file. It has no I/O and depends only on `contracts/`.
- Something has to receive the bytes, check who sent them, store them, and serve them back.
- The scene has `Level.background` (`AssetRef`), but no intent sets it.
- There are no accounts yet ([0008](0008-data-assets-accounts.md) says email and password; today the server has only the dev stand-in, `TT_DEV_AUTH`).

The dependency rule says `integrations/` may depend only on `contracts/`. It does not stop the server from using an `integrations/` crate, but the component table lists the server's dependencies as contracts, rules and plugin-host, so using the pipeline from the server needs a recorded decision.

## Decision

### 1. The server runs the upload and calls the pipeline as a library

`crates/server` may depend on `tt-asset-pipeline`, and on no other crate under `integrations/`.

- The pipeline stays pure: no network, no disk, no database, no clock. If it ever needs I/O it stops being eligible for this exception.
- `tools/check-deps.sh` gains two checks: `crates/server/Cargo.toml` may name `tt-asset-pipeline` but no other `integrations/` crate, and `integrations/asset-pipeline` must not depend on `tokio`, `axum`, `reqwest` or any other I/O crate.
- The ARCHITECTURE.md component table lists the exception.
- @aalemany00 keeps owning the pipeline; @Henry-theBinaryBuddies owns the endpoint and the store.

### 2. Upload is an HTTP POST, authorised like the socket

- `POST /campaigns/{campaign_id}/assets`, body = the raw file bytes, `Authorization: Bearer <token>`.
- The token is the same one the browser sends in `Hello`. The server checks it with the same code as `Hello` (`auth::authenticate`), so when real accounts arrive both paths change together.
- **Only a GM of that campaign may upload.** Anyone else gets 403.
- The request body is capped at `tt_asset_pipeline::MAX_FILE_BYTES` (20 MiB) before it is read in full; the pipeline's own checks then run on the bytes.
- On success the server stores the file and replies with the encoded `AssetRef` (Protobuf, `application/x-protobuf`), with `url` filled in by the server.
- Uploading the same file twice returns the same `AssetRef`; the file is stored once.

**This is not security yet.** With `TT_DEV_AUTH=1`, anyone who can reach the server can claim to be the GM. Upload is therefore exactly as safe as the rest of the dev server and must not be reachable from the internet until real sign-in exists (M4, deployment).

### 3. A new intent puts an uploaded image on a level

Contract change request (`session.proto`, additive):

```proto
message SetLevelBackgroundIntent {
  string scene_id = 1;
  string level_id = 2;
  // An asset_id the server returned from an upload. Empty clears the background.
  string asset_id = 3;
}
```

added to `Intent.kind` as `SetLevelBackgroundIntent set_level_background = 20;`.

- GM only; others get `IntentRejected`.
- The intent carries only the id. The server fills in `media_type`, `url`, `width` and `height` from its own record of the upload and rejects an unknown id. A client cannot claim a size or URL.
- The result goes out as the existing `SceneUpserted` event, so the renderer and web code need no new event handling.

Token art uses the same upload endpoint later; the intent that sets a token's image is a separate change and not part of M1.

### 4. For the class demo, files go on the server's own disk

- The server stores each file as `<asset_id>.<ext>` in a directory from `TT_ASSET_DIR`, and serves it at `GET /assets/<asset_id>.<ext>` with `Cache-Control: public, max-age=31536000, immutable`.
- Because the name is the file's own hash, a cached copy can never be stale. Behind Cloudflare, image files with these headers are cached at the edge, which removes most of the home-connection bandwidth problem 0008 worried about.
- Storage sits behind a small trait (`put`, `get`, `exists`), so Cloudflare R2 (0008's production choice) can replace the disk store without touching the endpoint or the intent.
- **No credentials are needed** for the disk store. Moving to R2 later needs an R2 access key, which lives only in the server's local environment (like the LiveKit secret), never in the repository, an issue, a PR or chat. Henry holds it.
- Back up `TT_ASSET_DIR` with the database once there is one; until then the demo campaign's maps can be uploaded again.

## Why

- **One process, one auth path.** The server already knows who is a GM. A separate service would need its own copy of sign-in, its own deployment, and its own way to tell the server a file exists.
- **The pipeline stays where it is and who owns it stays the same.** Only the direction of one dependency is recorded, and the pipeline's purity is checked in CI.
- **Content-addressed files make caching free**, which is what makes local disk acceptable for a few players.
- **No new dependencies for M1.** The disk store and the endpoint use what the server already has (axum, tokio, sha2). The only possible addition is tokio's `fs` feature; the store can use `std::fs` inside `spawn_blocking` instead, and the server PR will ask before turning the feature on.

## Rejected

- **(a) A separate upload service under `integrations/`:** a second process to deploy and keep running, and a second sign-in check that must agree with the server's. Too much for the class timeline for no gain at this size.
- **(c) Moving validation into a crate under `crates/`:** takes finished work away from its owner and changes the ownership map ([0002](0002-repository-and-ownership.md)) for no technical benefit, since the pipeline is already pure.
- **Upload over the WebSocket:** a 20 MiB binary message blocks every other message on that connection while it transfers, and the socket protocol would need chunking. HTTP already does this.
- **R2 or MinIO for the demo:** needs an S3 client dependency and credentials before M1, for a demo with a handful of players. It stays the production plan.
- **Letting the client send the full `AssetRef` in the intent:** the server would have to trust a size and URL from the browser.

## Follow-up issues

| Piece | Owner | Files |
|---|---|---|
| Contract change: `SetLevelBackgroundIntent` | @Henry-theBinaryBuddies | `contracts/proto/tomeandtile/v1/session.proto` |
| Server: asset store, upload endpoint, `GET /assets/...`, the intent, the dependency exception in `check-deps.sh` and ARCHITECTURE.md | @Henry-theBinaryBuddies | `crates/server/`, `tools/check-deps.sh`, `ARCHITECTURE.md`, root `Cargo.toml` |
| Mock server: accept the upload and the intent so `web/` can be built against it | @Henry-theBinaryBuddies | `crates/mock-server/` |
| Web: GM-only "Set map image" control (file picker, POST, then the intent, with errors shown by code) | @Danny-Defeato | `web/src/` |
| Pipeline: no change needed; it must stay pure | @aalemany00 | `integrations/asset-pipeline/` |
