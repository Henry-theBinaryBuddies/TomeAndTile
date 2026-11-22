# tt-server (version 0)

The authoritative game server: permissions, server-side dice, an in-memory event log and per-viewer filtering. Not the mock: the mock (`tt-mock-server`) applies no rules.

Layout: a pure core (`campaign.rs`, `view.rs`, `auth.rs`, `ids.rs`, `rng.rs`) with time, randomness and ids passed in, and a thin socket layer (`net.rs`, `main.rs`).

## Run

```
TT_DEV_AUTH=1 cargo run -p tt-server
```

The server reads only its environment. To keep settings (and the LiveKit secret) in a git-ignored `.env` instead, copy `.env.example` to `.env`, fill in what you need, and start with `tools/run-server.ps1` (PowerShell) or `bash tools/run-server.sh`. Both load `.env`, skip comments and empty values, print only the variable names, and pass extra arguments to `cargo run`.

| Variable | Meaning | Default |
|---|---|---|
| `TT_PORT` | listen port | 8788 (the mock uses 8787) |
| `TT_BIND` | bind address | 127.0.0.1 |
| `TT_SEED` | path to a `Snapshot` JSON file | embedded `contracts/fixtures/seeds/pregen-table.json`, the pregenerated table (a note is logged to stderr) |
| `TT_DEV_AUTH` | `1` enables dev sign-in | off: every Hello is refused, and a stderr line says so |
| `TT_LIVEKIT_URL` | LiveKit server URL, `wss://...` or `ws://...` | unset |
| `TT_LIVEKIT_API_KEY` | LiveKit API key | unset |
| `TT_LIVEKIT_API_SECRET` | LiveKit API secret | unset |

`GET /` returns a one-line banner, `GET /healthz` returns `ok`, `/ws` is the game socket (binary frames; first frame must be `Hello`).

## DEV AUTH IS NOT SECURITY

There are no accounts yet (ADR 0008). With `TT_DEV_AUTH=1` a `Hello.session_token` of `dev:<user_id>` signs in as that member of the seeded campaign, with that member's role. Anyone who can reach the socket can claim to be any member, including the GM. Never enable it on a reachable server. Any other token, an unknown member, or a `Hello.campaign_id` that differs from the campaign's (when both are set) closes the socket. A missing or unknown token is never treated as a player.

## Behaviour

- Reconnect: `Hello.last_seq` is accepted but ignored. Every join gets a full `Snapshot`.
- Events are broadcast unfiltered inside the process; each connection filters for its own viewer (using current state, under a brief lock) before encoding. Rejections go only to the sender.
- The event log is in memory only and never trimmed. Nothing is persisted.
- Visibility: only a GM's `GM_ONLY` roll is hidden; every other roll is forced `PUBLIC`.
- Dice come from `Config.rolls` (an optional boxed `RollSource`); `None` uses the operating system. Tests inject a scripted one.

## Using an ability (`UseAbilityIntent`)

A GM, or a player who controls the (visible) source token, sends `use_ability` with `source_token_id`, `ability_id`, `target_token_ids` and optionally `mode`. The server finds the creatures behind the tokens, runs `tt-rules` (`resolve_attacks`, attack steps only) and appends, in this order, with consecutive `seq` and the intent's `client_intent_id`:

1. one `ResolutionUpdated` in state `DONE`;
2. a `CreatureUpserted` for the source creature, only if the ability spent a resource.

Nothing else changes: **no hit points, temporary hit points or health states change in M2** (applying damage is M3, ADR 0013). `TargetOutcome.damage` stays empty and no target creature is modified.

- Ids and attribution: the server fills `Resolution.resolution_id` and every `RollResult.roll_id` (attack and damage) from its id source, sets `roller_user_id` to the sender and `visibility` to `PUBLIC` (the intent has no visibility field; hiding a GM's ability roll is later).
- Scene grid: taken from the scene of the source token. Without a grid, range is not checked.
- Warnings never stop the action. The engine's warnings are in `Resolution.warnings` and also in `Event.warnings` of the `ResolutionUpdated` event (only that event). Codes: `out_of_range`, `too_many_targets`, `not_enough_resource`, `extra_targets_ignored` (see `common.proto`). A viewer who may not see the source creature's sheet (a player, for a monster) gets the resolution without `not_enough_resource`, because that warning names the creature's resource and how many uses are left.
- Refusals go to the sender only and append nothing. Order of checks: permission, `unknown_token`, `no_creature`, `unknown_ability`, `unknown_target`, then the engine (`no_targets`, `not_implemented`), then `rng_unavailable`. Codes are in the table below.
- A player naming a source token that is missing, hidden or not theirs always gets `not_permitted`, so a player can never learn what abilities a GM creature has.
- Dice that do not parse in the ability's own data (not the client's request) are reported as `not_implemented`.
- Visibility: `ResolutionUpdated` goes to every viewer (spectators included, like public chat) when neither the source token nor any target token is hidden; otherwise to the GM only. `CreatureUpserted` keeps the normal sheet rule, so a player never receives a monster's `Creature` (AC, HP) in a snapshot or an event. The log keeps every event unredacted.

## Video and voice (join tokens, ADR 0007)

A joined client sends `MediaTokenRequest`; the server replies to that client only with `MediaToken` (`sfu_url`, `token`, `room`, `expires_at_ms`, `can_publish`). The reply uses no `seq`, is not broadcast and is not in the event log. The server relays no media and no signalling; the browser talks to LiveKit directly. `MediaTokenRequest` as the first frame closes the socket like any non-Hello first frame.

- Token: a LiveKit access token (JWT, HS256) with `iss` = API key, `sub` = user id, `name` = display name (omitted if empty), `nbf` = now, `exp` = now + 600 s (10 minutes), and a `video` grant `{room, roomJoin: true, canSubscribe: true, canPublish, canPublishData}`. Ask again to rejoin after expiry; `expires_at_ms` says when.
- Room: `tt-<campaign_id>`, one per campaign. A campaign with an empty id has no room, so media is treated as not configured. The embedded dev seed's campaign id is `campaign-mock` (taken from its scene).
- GM and Player: publish and publish data. Spectator: subscribe only (`can_publish` false). Any other role: no token.
- Refusal convention: the request carries no id, so the refusal is an `IntentRejected` with an empty `client_intent_id`, sent to the requester only. Codes: `media_not_configured` (the three variables are unset, or the campaign has no id) and `not_permitted` (the role gets no token; not reachable today, since only GM, Player and Spectator can join).
- Configuration: set all three `TT_LIVEKIT_*` variables to turn media on, none to leave it off. Some but not all, an empty key or secret, or a URL that does not start with `wss://` or `ws://` stops the server at start-up with a message naming the variable(s) but never a value. Start-up prints only whether media is on and the URL.
- Secrets: the key and secret belong in the server's local environment only, never in the repo, a seed file, a log or chat. The secret and minted tokens are never logged; `MediaConfig`'s `Debug` hides the secret.
- NOT verified: tokens have never been used against a real LiveKit server (Cloud or self-hosted). Claim names follow LiveKit's "Access tokens & grants" reference (https://docs.livekit.io/frontends/reference/tokens-grants.md), which does not list the `name` claim; `name` is sent on the strength of LiveKit's server SDKs and is unconfirmed. The signature is checked in tests only with the same HMAC code that makes it.

## Asset upload (ADR 0014)

A GM uploads a map image over HTTP, then sends `set_level_background` with the returned `asset_id`; every viewer's level gets it through a `SceneUpserted` event.

| Variable | Meaning | Default |
|---|---|---|
| `TT_ASSET_DIR` | directory for uploaded files (created if missing; start-up fails if it cannot be created or is not a directory) | unset: uploads OFF |
| `TT_PUBLIC_URL` | `http://` or `https://` base for the `url` in the returned `AssetRef`, trailing `/` trimmed; anything else is a start-up error | unset: the request's `Host` header |

Routes (both answer any origin with `Access-Control-Allow-Origin: *`, `OPTIONS` with 204, other methods with JSON 405 `method_not_allowed`):

- `POST /campaigns/{campaign_id}/assets`, body = the raw file, `Authorization: Bearer <token>` (the same token as `Hello`, checked by `auth::authenticate` before the body is read). GM only. 200 with a protobuf `AssetRef` (`Content-Type: application/x-protobuf`). The same file twice returns the same `AssetRef`; the file is stored once as `<sha256>.<png|jpg|webp>`, written to a temp file and renamed.
- `GET /assets/<64 hex>.<png|jpg|webp>`: no auth, `Cache-Control: public, max-age=31536000, immutable`, `X-Content-Type-Options: nosniff`.

Every error is JSON `{"code","message"}`:

| Status | Code | When |
|---|---|---|
| 503 | `uploads_off` | `TT_ASSET_DIR` not set |
| 503 | `busy` | four uploads already in progress |
| 401 | `unauthorized` | no or malformed `Authorization`, dev auth off, bad token, unknown member |
| 403 | `not_permitted` | signed in but not the GM |
| 404 | `unknown_campaign` | the path campaign is not this server's campaign |
| 413 | `file_too_large` | body over 20 MiB (`tt_asset_pipeline::MAX_FILE_BYTES`) |
| 400 | `bad_body` | the body could not be read |
| 408 | `upload_timeout` | the body took longer than 120 s to arrive (frees the upload slot) |
| 400 | pipeline code | `empty_file`, `disallowed_format`, `unknown_format`, `truncated_header`, `invalid_dimensions`, `too_many_pixels` (`tt_asset_pipeline::AssetError::code`) |
| 500 | `storage_error` | the disk write failed (details on stderr, never a token) |
| 404 | `unknown_asset` | `GET` of anything that is not a stored `<id>.<ext>` |

At start-up the directory is scanned once: each file named `<64 hex>.<png|jpg|webp>` whose bytes pass the pipeline, whose hash equals its name and whose extension matches its format is registered (up to 1000 files; others are skipped with a note on stderr), so uploads stay usable after a restart. Without `TT_PUBLIC_URL` their link is only a guess from the listener address (a wildcard bind such as `0.0.0.0` becomes `127.0.0.1`); uploading the same file again replaces it with the `Host`-based link. Set `TT_PUBLIC_URL` when clients are not on this machine. An intent can only name an `asset_id` this server registered.

NOT SECURITY: with `TT_DEV_AUTH=1` anyone who can reach the server can claim to be the GM and upload. Do not expose upload to the internet until real sign-in exists.

## Rejection codes (`IntentRejected.code`)

| Code | When |
|---|---|
| `not_permitted` | spectator or non-member sending anything; (for `MediaTokenRequest`, see above); player moving a token (or using an ability of a source token) they do not control, that is hidden, or that does not exist; non-GM `set_controllers` or `set_level_background` |
| `malformed_intent` | the intent has no kind |
| `not_implemented` | any intent kind other than move, chat, roll, use_ability, set_controllers, set_level_background; for `use_ability`, an area shape, a non-attack step, or ability data the engine cannot use (including dice that do not parse) |
| `unknown_token` | GM names a token that does not exist (move, set_controllers, the source of use_ability) |
| `no_creature` | `use_ability`: the source token has no creature |
| `unknown_ability` | `use_ability`: the ability id is not on the source creature |
| `no_targets` | `use_ability`: a creature-targeting ability with no targets |
| `unknown_target` | `use_ability`: a target token that does not exist, is hidden from a player sender, or has no creature |
| `empty_path` | move with no waypoints |
| `path_too_long` | more than 256 waypoints |
| `bad_coordinate` | NaN, infinite, or beyond 1,000,000 units |
| `unknown_level` | `level_id` not a level of the token's scene (move) or of the named scene (set_level_background) |
| `empty_chat` | blank after trimming |
| `chat_too_long` | over 2000 characters |
| `label_too_long` | roll label over 120 characters |
| `dice_empty`, `dice_syntax`, `dice_bad_sides`, `dice_bad_count`, `dice_too_many_dice`, `dice_too_many_terms`, `dice_bad_keep`, `dice_number_too_large`, `dice_too_long` | bad notation (`tt_rules::dice::ParseError::code`) |
| `rng_unavailable` | the OS gave no entropy; nothing changed, nothing was rolled |
| `unknown_user` | `set_controllers` names a non-member |
| `invalid_controller` | `set_controllers` names a spectator |
| `unknown_scene` | `set_level_background` names a scene that does not exist |
| `unknown_asset` | `set_level_background` names an `asset_id` this server never issued (empty clears) |

## Filtering rules and judgement calls

See ADR 0006 and `view.rs`. Where the contract is silent the more restrictive behaviour was chosen:

- Hidden tokens go to the GM only (snapshot and events), and players cannot move them. Controlling a hidden token reveals neither it nor its creature sheet.
- `hp_current`, `hp_max`, `hp_temp` go to the GM and listed controllers only. The contract has no reveal mechanism, so none is implemented. Spectators never get hp, even if listed as controller.
- Full creature sheets go to the GM, to the owner, and to controllers of a visible linked token. Not to spectators.
- `set_controllers` also emits a `CreatureUpserted` for the token's creature, so a newly added controller receives the sheet. The filter decides who actually gets it. A controller who is removed keeps what they were already sent.
- Unspecified roll visibility is treated as not public (GM only) when filtering.
- `TokenRemoved`, `EventUndone` and events with no kind go to the GM only until a feature emits them with enough information to filter.
- An encounter reaches a non-GM only if every combatant is a known, non-hidden token.
- All scenes and members go to every viewer; chat and public rolls go to everyone including spectators.
- Also stricter than the mock: unknown `level_id` is rejected; a non-GM gets `not_permitted` for a missing token; coordinates are bounded.

## Tests

The core is tested in `src/tests.rs`. `tests/serve.rs` checks `/healthz` and `/` over raw TCP. `tests/socket.rs` connects real WebSocket clients (dev-dependencies `tokio-tungstenite` and `futures-util`) to a server on an ephemeral port and covers:

- join: Snapshot identity and role; dev auth off; unknown user, malformed token, wrong campaign id; first frame not a Hello (an intent, garbage, empty); a text frame before the Hello; the 10 second Hello timeout (run on a paused tokio clock, so it takes no real time);
- broadcast: chat reaches everyone with the same `seq`; a rejection goes only to the sender, with its `client_intent_id` and code, and uses no `seq`; `seq` strictly increases;
- filtering over the wire: hidden tokens and hp in the Snapshot; a `GM_ONLY` roll and a hidden-token move reach the GM only; `set_controllers` delivers the token and creature to the new controller, who can then move it;
- use_ability: with the pregen seed and a scripted roller, a player's attack reaches the GM and the player as the same `ResolutionUpdated` and nothing else (no token or creature event, no goblin sheet);
- media: a player and a spectator get tokens with the right URL, room and grants (payload decoded in the test) and nobody else sees anything; no `seq` used; repeated requests; media off gives `media_not_configured` to the requester only; a request before Hello closes the socket;
- hostile input: a message over 64 KiB (before and after joining), garbage after joining, text frames and a second Hello (ignored);
- a client that stops reading is dropped (broadcast lag, found by flooding until the socket buffers fill; 12000 chats of 2000 characters).

"Nothing arrives" is asserted with a sentinel chat that must be the next message, never with a sleep.

Not tested: the exact lag threshold of 256 events (the drop is tested, not the number); behaviour under many simultaneous connections; TLS and the Cloudflare tunnel (outside this crate).
