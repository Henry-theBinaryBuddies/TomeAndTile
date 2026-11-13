# tt-server (version 0)

The authoritative game server: permissions, server-side dice, an in-memory event log and per-viewer filtering. Not the mock: the mock (`tt-mock-server`) applies no rules.

Layout: a pure core (`campaign.rs`, `view.rs`, `auth.rs`, `ids.rs`, `rng.rs`) with time, randomness and ids passed in, and a thin socket layer (`net.rs`, `main.rs`).

## Run

```
TT_DEV_AUTH=1 cargo run -p tt-server
```

| Variable | Meaning | Default |
|---|---|---|
| `TT_PORT` | listen port | 8788 (the mock uses 8787) |
| `TT_BIND` | bind address | 127.0.0.1 |
| `TT_SEED` | path to a `Snapshot` JSON file | embedded `contracts/fixtures/mock/snapshot.json` (a note is logged to stderr) |
| `TT_DEV_AUTH` | `1` enables dev sign-in | off: every Hello is refused, and a stderr line says so |

`GET /` returns a one-line banner, `GET /healthz` returns `ok`, `/ws` is the game socket (binary frames; first frame must be `Hello`).

## DEV AUTH IS NOT SECURITY

There are no accounts yet (ADR 0008). With `TT_DEV_AUTH=1` a `Hello.session_token` of `dev:<user_id>` signs in as that member of the seeded campaign, with that member's role. Anyone who can reach the socket can claim to be any member, including the GM. Never enable it on a reachable server. Any other token, an unknown member, or a `Hello.campaign_id` that differs from the campaign's (when both are set) closes the socket. A missing or unknown token is never treated as a player.

## Behaviour

- Reconnect: `Hello.last_seq` is accepted but ignored. Every join gets a full `Snapshot`.
- `media_token_request` has no intent id to address a rejection to, so it is ignored.
- Events are broadcast unfiltered inside the process; each connection filters for its own viewer (using current state, under a brief lock) before encoding. Rejections go only to the sender.
- The event log is in memory only and never trimmed. Nothing is persisted.
- Visibility: only a GM's `GM_ONLY` roll is hidden; every other roll is forced `PUBLIC`.

## Rejection codes (`IntentRejected.code`)

| Code | When |
|---|---|
| `not_permitted` | spectator or non-member sending anything; player moving a token they do not control, that is hidden, or that does not exist; non-GM `set_controllers` |
| `malformed_intent` | the intent has no kind |
| `not_implemented` | any intent kind other than move, chat, roll, set_controllers |
| `unknown_token` | GM names a token that does not exist (move, set_controllers) |
| `empty_path` | move with no waypoints |
| `path_too_long` | more than 256 waypoints |
| `bad_coordinate` | NaN, infinite, or beyond 1,000,000 units |
| `unknown_level` | `level_id` not a level of the token's scene |
| `empty_chat` | blank after trimming |
| `chat_too_long` | over 2000 characters |
| `label_too_long` | roll label over 120 characters |
| `dice_empty`, `dice_syntax`, `dice_bad_sides`, `dice_bad_count`, `dice_too_many_dice`, `dice_too_many_terms`, `dice_bad_keep`, `dice_number_too_large`, `dice_too_long` | bad notation (`tt_rules::dice::ParseError::code`) |
| `rng_unavailable` | the OS gave no entropy; nothing changed, nothing was rolled |
| `unknown_user` | `set_controllers` names a non-member |
| `invalid_controller` | `set_controllers` names a spectator |

## Filtering rules and judgement calls

See ADR 0006 and `view.rs`. Where the contract is silent the more restrictive behaviour was chosen:

- Hidden tokens go to the GM only (snapshot and events), and players cannot move them. Controlling a hidden token reveals neither it nor its creature sheet.
- `hp_current`, `hp_max`, `hp_temp` go to the GM and listed controllers only. The contract has no reveal mechanism, so none is implemented. Spectators never get hp, even if listed as controller.
- Full creature sheets go to the GM, to the owner, and to controllers of a visible linked token. Not to spectators.
- `set_controllers` also emits a `CreatureUpserted` for the token's creature, so a newly added controller receives the sheet. The filter decides who actually gets it. A controller who is removed keeps what they were already sent.
- Unspecified roll visibility is treated as not public (GM only) when filtering.
- `TokenRemoved`, `ResolutionUpdated`, `EventUndone` and events with no kind go to the GM only until a feature emits them with enough information to filter.
- An encounter reaches a non-GM only if every combatant is a known, non-hidden token.
- All scenes and members go to every viewer; chat and public rolls go to everyone including spectators.
- Also stricter than the mock: unknown `level_id` is rejected; a non-GM gets `not_permitted` for a missing token; coordinates are bounded.

## Tests

The core is tested in `src/tests.rs`. `tests/serve.rs` checks `/healthz` and `/` over raw TCP. `tests/socket.rs` connects real WebSocket clients (dev-dependencies `tokio-tungstenite` and `futures-util`) to a server on an ephemeral port and covers:

- join: Snapshot identity and role; dev auth off; unknown user, malformed token, wrong campaign id; first frame not a Hello (an intent, garbage, empty); a text frame before the Hello; the 10 second Hello timeout (run on a paused tokio clock, so it takes no real time);
- broadcast: chat reaches everyone with the same `seq`; a rejection goes only to the sender, with its `client_intent_id` and code, and uses no `seq`; `seq` strictly increases;
- filtering over the wire: hidden tokens and hp in the Snapshot; a `GM_ONLY` roll and a hidden-token move reach the GM only; `set_controllers` delivers the token and creature to the new controller, who can then move it;
- hostile input: a message over 64 KiB (before and after joining), garbage after joining, text frames and a second Hello (ignored);
- a client that stops reading is dropped (broadcast lag, found by flooding until the socket buffers fill; 12000 chats of 2000 characters).

"Nothing arrives" is asserted with a sentinel chat that must be the next message, never with a sleep.

Not tested: the exact lag threshold of 256 events (the drop is tested, not the number); behaviour under many simultaneous connections; TLS and the Cloudflare tunnel (outside this crate).
