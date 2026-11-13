# tt-mock-server

A stand-in for the game server. It speaks the real game-socket protocol (binary WebSocket frames, each a `ClientMessage` up or a `ServerMessage` down) from recorded data, so the web UI and importers can be built before the real server exists.

> **Warning: this is not the server.** It applies no rules, no permissions and no per-viewer filtering. Every client may move any token, every client sees every field, and every die is public. A UI must never rely on the mock for anything the real server will hide or refuse. Do not add rules or permission logic here; that belongs in `tt-server` and `tt-rules`.

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
| anything else | Rejects `not_supported_by_mock`, echoing `client_intent_id`. |
| `media_token_request` | Ignored; no reply. |

Rejections go to the sender only and do not use a sequence number.
