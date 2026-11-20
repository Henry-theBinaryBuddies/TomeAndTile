# tt-mock-server

A stand-in for the game server. It speaks the real game-socket protocol (binary WebSocket frames, each a `ClientMessage` up or a `ServerMessage` down) from recorded data, so the web UI and importers can be built before the real server exists.

> **Warning: this is not the server.** It applies no rules and no per-viewer filtering. The only permission it checks is the GM role, for asset upload and `set_level_background`, so the UI can exercise those refusal paths; that is still not security. Every client may move any token, every client sees every field, and every die is public. A UI must never rely on the mock for anything the real server will hide or refuse. Do not add rules or permission logic here; that belongs in `tt-server` and `tt-rules`.

## Run

```bash
cargo run -p tt-mock-server
```

It prints `Tome & Tile mock server listening on ws://127.0.0.1:8787/ws`. The port defaults to 8787; set `TT_MOCK_PORT` to change it. It binds `127.0.0.1` only. `GET /` returns a one-line text description.

The starting state is `contracts/fixtures/mock/snapshot.json`: one campaign, one square-grid scene (cell 70, 5 ft per cell), a player token "Aria" (token id `token-aria`, controlled by `mock-player`) and a monster "Goblin" (`token-goblin`, no hit points shown). State is shared by every connection and lives in memory; restart the process to reset it.

## Protocol behaviour

- The first binary frame must be a `ClientMessage` with `hello`. The reply is a `Snapshot` of the current state (`seq` is the last event so far). Anything else first closes the connection.
- Identity: `session_token` of exactly `gm` is user `mock-gm` with role GM; any other token is user `mock-player` with role PLAYER. `campaign_id` and `last_seq` are ignored (there is no replay; reconnect and use the Snapshot).
- Incoming messages are limited to 64 KiB. Text frames are ignored. A frame that does not decode as a `ClientMessage` closes that connection only. A second `hello` is ignored.
- Every event goes to every connected client that has said hello, including the sender. Events carry a strictly increasing global `seq`, the server clock in `at_ms`, the fixture scene id, the sender as `actor_user_id` and the causing `client_intent_id`.
- A client that falls more than 256 events behind is disconnected; it can reconnect and get a fresh Snapshot.

| Intent | What the mock does |
|---|---|
| `move_token` | Sets the token's position to the last waypoint (and `level_id` if given). Broadcasts `token_moved` with `distance_feet` = straight-line path length from the old position, divided by `cell_size`, times `feet_per_cell`. Rejects `unknown_token` or `empty_path`. |
| `chat` | Broadcasts `chat_posted`. Rejects `empty_chat`, or `chat_too_long` above 2000 characters. |
| `roll_dice` | Notation `NdM`, `NdM+K`, `NdM-K` (N 1-100, M one of 4, 6, 8, 10, 12, 20, 100, whitespace allowed). Advantage or disadvantage applies only to a single d20 (two dice, one `dropped`). Always PUBLIC visibility, purpose FREEFORM. Broadcasts `dice_rolled`. Rejects `bad_notation`. The random source is a clock-seeded xorshift and is not suitable for the real server. |
| `set_level_background` | GM only. Sets the level's background to the uploaded `AssetRef` (the same one the upload returned), or clears it when `asset_id` is empty, and broadcasts `scene_upserted` with the updated scene; later Snapshots include it. Rejects `not_permitted` (sender is not `mock-gm`), `unknown_scene`, `unknown_level`, or `unknown_asset` (non-empty id that was not uploaded). |
| anything else | Rejects `not_supported_by_mock`, echoing `client_intent_id`. |
| `media_token_request` | Ignored; no reply. |

Rejections go to the sender only and do not use a sequence number.

## HTTP: asset upload

Lets the web UI build a GM "Set map image" control (ADR 0014). For `web/`: the HTTP base is the socket URL with `ws` changed to `http` and `/ws` removed (`ws://127.0.0.1:8787/ws` becomes `http://127.0.0.1:8787`).

| Route | What it does |
|---|---|
| `POST /campaigns/{campaign_id}/assets` | Body is the raw file bytes. `campaign_id` is ignored. Replies `200` with `Content-Type: application/x-protobuf` and an encoded `AssetRef`. |
| `GET /assets/{asset_id}.{ext}` | Serves the stored bytes with the stored `Content-Type` and `Cache-Control: public, max-age=31536000, immutable`. The id and extension must both match a stored asset. |
| `OPTIONS` on both | CORS preflight: `204`. |

Auth is `Authorization: Bearer <token>` (scheme case-insensitive). The token `gm` is the GM, the same rule as `hello`; any other token is refused. `GET` needs no auth.

Every error is JSON, `{"code": "...", "message": "..."}` with `Content-Type: application/json`:

| Status | `code` | When |
|---|---|---|
| 401 | `unauthorized` | `Authorization` header missing or not `Bearer <token>` |
| 403 | `not_permitted` | token is not `gm` |
| 400 | `empty_file` | empty body |
| 400 | `unknown_format` | not PNG, JPEG or WebP by magic bytes (SVG and GIF are refused) |
| 400 | `bad_body` | the body could not be read |
| 413 | `file_too_large` | body over `MAX_ASSET_BYTES` (20 MiB, as in `tt_asset_pipeline::MAX_FILE_BYTES`) |
| 507 | `storage_full` | a new distinct asset beyond `MAX_STORED_ASSETS` (32); re-uploading an existing one still succeeds |
| 404 | `unknown_asset` | `GET` of an id, extension or path that is not stored (including undecodable percent-encoding) |
| 405 | `method_not_allowed` | any method other than GET/POST/OPTIONS on these routes |

CORS: every response from both routes, success and error, carries `Access-Control-Allow-Origin: *`. Preflight answers also send `Access-Control-Allow-Methods: GET, POST, OPTIONS`, `Access-Control-Allow-Headers: Authorization, Content-Type` and `Access-Control-Max-Age: 600`.

Details to know:

- The mock only sniffs magic bytes. It does not run the real pipeline's other checks (decoding, dimension limits, re-encoding, scanning), so `width` and `height` are always 0 ("unknown"; the renderer should use the image's own size).
- `asset_id` is `mock-` plus 16 lowercase hex digits of FNV-1a 64-bit over the bytes, so the same bytes give the same id and are stored once. The real server uses SHA-256; clients must treat the id as opaque.
- `url` is `http://` plus the request's `Host` header plus `/assets/{asset_id}.{ext}`; if `Host` is missing or unusable, the listener's address is used.
- Assets live in memory only and are lost when the process restarts, along with any background set through `set_level_background`.
