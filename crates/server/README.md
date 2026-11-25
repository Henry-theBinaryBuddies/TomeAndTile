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

A GM, or a player who controls the (visible) source token, sends `use_ability` with `source_token_id`, `ability_id`, `target_token_ids` and optionally `mode`. The server finds the creatures behind the tokens, runs `tt_rules::resolve_ability` (attack, save, flat and apply-effect steps; damage, healing and conditions are applied by the engine) and appends, in this order, with consecutive `seq` and the intent's `client_intent_id`:

1. one `ResolutionUpdated` in state `DONE`;
2. one `TokenUpserted` for each target token whose HP, temporary HP, conditions or timers changed, in the order the engine returns them (the order outcomes first touched them; a source that hits or heals itself appears once);
3. a `CreatureUpserted` for the source creature, only if a resource was spent or `recharge_spent` changed.

- **Hit points live on the token** (`Token.hp_*`), never on the creature: three goblins share one stat block. `Creature.hp_*` is the stat block's value and no intent changes it. The token's `health_state` is recomputed with every change and goes to everyone who may see the token; the numbers only to the GM and the token's controllers.
- Death saves stay manual. Reaching 0 sets `HEALTH_STATE_DOWN` (and `dropped_to_zero` on the outcome) and nothing else: no death save, no instant death.
- Encounter: the encounter in state whose `scene_id` is the source token's scene, with its `round`, is passed to the engine (it decides condition timers). While the scene has no running encounter, timed conditions get a `duration_not_tracked` warning (start one with `start_encounter`, below).
- Ids and attribution: the server fills `Resolution.resolution_id` and every `RollResult.roll_id` from its id source, sets `roller_user_id` to the sender and `visibility` to `PUBLIC`. A damage roll that the engine copies into every outcome of one save or flat step gets ONE `roll_id`, the same in every copy. Every other roll (attack rolls, saving throws, each target's attack damage) has its own.
- Scene grid: taken from the scene of the source token. Without a grid, range is not checked.
- Warnings never stop the action. The engine's warnings are in `Resolution.warnings` and also in `Event.warnings` of the `ResolutionUpdated` event (only that event). Codes: `out_of_range`, `too_many_targets`, `not_enough_resource`, `extra_targets_ignored`, `area_not_checked`, `no_hit_points`, `condition_immune`, `duration_not_tracked`, `ability_recharging` (see `common.proto`). A viewer who may not see the source creature's sheet (a player, for a monster) gets the resolution without `not_enough_resource`, because that warning names the creature's resource and how many uses are left.
- Area shapes (sphere, cone, line, cube) resolve: the listed tokens are the targets, no template is placed, and `area_not_checked` is given once.
- Refusals go to the sender only and append nothing. Order of checks: permission, `unknown_token`, `no_creature`, `unknown_ability`, `unknown_target`, then the engine (`no_targets`, `not_implemented`), then `rng_unavailable`. `not_implemented` is now only for plugin steps, mixed multi-attack abilities (several attack steps with a save or flat step) and ability data the engine cannot use (including dice that do not parse). Codes are in the table below.
- A player naming a source token that is missing, hidden or not theirs always gets `not_permitted`, so a player can never learn what abilities a GM creature has.
- Visibility: `ResolutionUpdated` goes to every viewer (spectators included, like public chat) when neither the source token nor any target token is hidden; otherwise to the GM only. `TokenUpserted` and `CreatureUpserted` keep their normal rules (below), so a player never receives a monster's `Creature` (AC, HP) or hit point numbers in a snapshot or an event. The log keeps every event unredacted.

## Encounters (#102)

Four intents run combat. Each validates first and changes state last (a refusal leaves state, `seq`, the log and the change records untouched; the entropy check comes before any change). Each is recorded as a `NotUndoable` group with the tokens and creatures it changed, so it blocks undo of earlier changes to them, and it cannot be undone itself (`not_undoable`).

| Intent | Who | Events | Notes |
|---|---|---|---|
| `start_encounter` | GM | one `DiceRolled` per combatant, then `EncounterUpdated` | duplicate token ids are listed once, in first-listed order. Per token one d20 plus the creature's `initiative_bonus` (0 with no creature), purpose `INITIATIVE`, label `<token name> initiative`, `roller_creature_id` set, `GM_ONLY` for a hidden token and `PUBLIC` otherwise. The encounter has a minted `encounter_id`, the scene id, round 1 and `turn_index` 0. Combatants are sorted by initiative (higher first), then `initiative_bonus` (higher first), then list order; `order` is the position after sorting. Nothing runs for the first turn (no recharge rolls) |
| `end_turn` | GM, or a player who controls the current combatant's visible token | `EncounterUpdated`, a `TokenUpserted` per token whose timers ended, then recharge `DiceRolled`s and one `CreatureUpserted` | `turn_index` moves on; past the last combatant it goes to 0 and `round` goes up by one |
| `set_initiative` | GM | `EncounterUpdated` | sets the combatant's `initiative` and `order`, then re-sorts: higher initiative first, then lower `order`, equal keys keep their places. The turn stays with the combatant whose turn it was |
| `end_encounter` | GM | `EncounterEnded`, a `TokenUpserted` per token that had timers of the encounter | removes the encounter and its condition timers; the conditions stay. Later Snapshots have no encounter |

Timers on `end_turn`: one call of `tt_rules::conditions::expire_timers` with the token whose turn ended (and the round it ended in) and the token whose turn starts (and its round). End-of-turn timers of the first and start-of-turn timers of the second end, together with their conditions; timers of other encounters and of no encounter are never touched. `use_ability` passes the scene's running encounter and its round, so round-based effects get timers: Choking Ash on round 1 ends when the wisp's turn starts in round 2.

Recharge: when a combatant's turn starts, the server rolls a d6 for each ability of its creature with `recharge_spent` true and a `recharge` rule. A roll of at least `min_roll` sets `recharge_spent` false. Each roll is a `DiceRolled` (label `<ability name> recharge`, purpose `FREEFORM`, `roller_creature_id` set), `GM_ONLY` when the creature has no owner or the token is hidden and `PUBLIC` otherwise; one `CreatureUpserted` follows when anything recharged.

Order of refusals:

- `start_encounter`: `not_permitted`, `unknown_scene`, `no_combatants` (empty list), `unknown_token` (missing, or on another scene), `encounter_running` (the scene has one; one encounter per scene), `rng_unavailable`.
- `end_turn`: `unknown_encounter` for the GM. A player naming a missing encounter, or one they may not see (a hidden combatant), or not controlling the current token, gets `not_permitted`, so a player cannot probe for a hidden fight. Then `rng_unavailable`.
- `set_initiative`: `not_permitted`, `unknown_encounter`, `unknown_combatant`.
- `end_encounter`: `not_permitted`, `unknown_encounter`.

Visibility: `EncounterUpdated` keeps the `encounter_visible` rule (a non-GM sees it only if every combatant is a known, non-hidden token), `EncounterEnded` goes to everyone, and initiative and recharge rolls follow the `DiceRolled` filter. Action economy flags stay false; combatants cannot be added or removed in a running encounter.

## Hit points, conditions and tokens

All four intents validate first and change state last: a refusal leaves state, `seq`, the log and the change records untouched. Each is answered with the events below (consecutive `seq`, the intent's `client_intent_id`).

| Intent | Who | Events | Notes |
|---|---|---|---|
| `set_hit_points` | the GM, or a controller of a visible token | one `TokenUpserted` | sets `hp_current` and `hp_temp` (both always sent; `hp_temp` 0 clears it), recomputes `health_state`. `hp_max` cannot be changed. The token is the only thing that changes: `Creature.hp_*` stays |
| `set_conditions` | GM | one `TokenUpserted` | `add` and `remove` condition ids. Adding a present id or removing an absent one changes nothing but still answers. Removing a condition removes its timer. Added ids have no timer |
| `create_token` | GM | `TokenUpserted`, then `CreatureUpserted` when `controller_user_ids` is not empty and there is a creature | the server mints `token_id`; HP is copied from the creature (a creature with `hp_max` 0 and a prop get none) and `health_state` computed; conditions start empty; empty `name` is the creature's name or "Token"; unspecified size is the creature's or Medium; empty `level_id` is the scene's first level. `position` is required (unset is `bad_coordinate`) |
| `remove_token` | GM | one `TokenRemoved` | the creature stays. Refused with `in_encounter` while the token is a combatant of an encounter in state |

Warning: `condition_immune` on `set_conditions` (an id in `add` is in the creature's `condition_immunities`; it is added anyway). It is given for each distinct such id in `add`, whether or not the token already had it. It rides on the one event.

On load (`Campaign::from_snapshot`) `health_state` is recomputed for every token with an `hp_max` (a missing `hp_current` counts as full); a stored value is never trusted.

Limits: 500 tokens per campaign, token name 120 characters, 32 conditions per token, `hp_temp` up to 999, coordinates within `MAX_COORDINATE`.

### Change records and undo (#101)

`Campaign.groups` has one `AppliedGroup` per applied intent, in `seq` order, next to `Campaign.log`: `first_seq`, `last_seq`, `actor_user_id`, `client_intent_id`, `changed_token_ids`, `changed_creature_ids`, a `kind` (`Move`, `Edit`, `NotUndoable`, `Undo`) and `undone`. The id lists are not written by the handlers: `apply` compares `tokens` and `creatures` before and after the handler and lists every id that is new, different or gone. So they are exact for every intent (a miss, or "no change" set, has empty lists) and cannot drift from what the handlers do.

For an undoable group (`Move` or `Edit`) the group also keeps `before_tokens` and `before_creatures`: the whole stored token or creature, as it was before the intent, for each changed id that existed then (a removed token is kept whole). A token the group created has no entry. Nothing is trimmed; the log and these states live in memory only, so undo does not survive a restart.

`UndoIntent` (GM only) names a group by its first event (for an ability use the `ResolutionUpdated`, not its `TokenUpserted`). What can be undone: an ability use, set hit points, set conditions, a move, a token create and a token remove. Not undoable: chat, dice rolls, set controllers, set level background, the encounter intents and an undo itself (no redo). Checks, in this order, each refusal going to the sender only and appending nothing: `not_permitted` (not the GM), `unknown_event` (no group holds that `seq`, including events before the seed), `not_first_event`, `not_undoable`, `already_undone`, `undo_blocked`.

Dependency rule: a group cannot be undone while a later group that has not been undone changed any of the same tokens or creatures. `undo_blocked` names that later group's first `seq`; undo it first. Undo groups never block: each one restored exactly the state of the group it reversed, so with that group marked undone the pair changes nothing for earlier groups (this is what lets "attack twice, undo the second, then the first" work). An undo group is still recorded with the tokens and creatures it changed, and is `Undo` kind, so it cannot be undone.

On success one group is appended with the undo intent's `client_intent_id`: for each token the group changed, in the group's id order, a `TokenUpserted` with its earlier state, or `TokenRemoved` for a token the group created, or `TokenUpserted` (plus `CreatureUpserted` when the token has controllers) for a token the group removed, or `TokenMoved` with a path of one point and the earlier level for a move; then a `CreatureUpserted` with the earlier sheet of each changed creature; then one `EventUndone` per event of the undone group in `seq` order. The state is restored before the events are appended. A put-back token goes to the end of `Campaign.tokens`, and its `removed_token_hidden` entry is deleted (a token removed by undoing a create gets one).

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
| `not_permitted` | spectator or non-member sending anything; (for `MediaTokenRequest`, see above); player moving a token (or using an ability of a source token, or setting its hit points) they do not control, that is hidden, or that does not exist; non-GM `set_controllers`, `set_level_background`, `set_conditions`, `create_token`, `remove_token`, `undo`, `start_encounter`, `set_initiative` or `end_encounter`; `end_turn` by a player who does not control the current combatant's visible token or who cannot see the encounter |
| `malformed_intent` | the intent has no kind |
| `not_implemented` | `use_ability`: a plugin step, several attack steps together with a save or flat step, or ability data the engine cannot use (including dice that do not parse) |
| `unknown_token` | GM names a token that does not exist (move, set_controllers, set_hit_points, set_conditions, remove_token, the source of use_ability), or `start_encounter` lists one that is missing or on another scene |
| `no_hit_points` | `set_hit_points`: the token has no `hp_max` |
| `bad_hit_points` | `set_hit_points`: `hp_current` below 0 or above `hp_max`, or `hp_temp` below 0 or above 999 |
| `bad_condition_id` | `set_conditions`: an id in `add` or `remove` is not `[a-z0-9_]{1,40}` |
| `conflicting_conditions` | `set_conditions`: the same id in `add` and `remove` |
| `too_many_conditions` | `set_conditions`: more than 32 conditions on the token afterwards |
| `unknown_creature` | `create_token` names a creature that does not exist |
| `name_too_long` | `create_token`: name over 120 characters |
| `too_many_tokens` | `create_token`: the campaign already has 500 tokens |
| `in_encounter` | `remove_token`: the token is a combatant of an encounter in state |
| `no_creature` | `use_ability`: the source token has no creature |
| `unknown_ability` | `use_ability`: the ability id is not on the source creature |
| `no_targets` | `use_ability`: a creature-targeting ability with no targets |
| `unknown_target` | `use_ability`: a target token that does not exist, is hidden from a player sender, or has no creature |
| `unknown_event` | `undo`: no event with that `seq` in the log |
| `not_first_event` | `undo`: the event belongs to a group but is not its first |
| `not_undoable` | `undo`: the group is a chat, roll, set controllers, set level background, encounter intent or undo |
| `already_undone` | `undo`: the group was already undone |
| `undo_blocked` | `undo`: a later group not undone changed one of the same tokens or creatures; the message names its first `seq` |
| `empty_path` | move with no waypoints |
| `path_too_long` | more than 256 waypoints |
| `bad_coordinate` | NaN, infinite, or beyond 1,000,000 units (also `create_token` with no position) |
| `unknown_level` | `level_id` not a level of the token's scene (move) or of the named scene (set_level_background, create_token) |
| `empty_chat` | blank after trimming |
| `chat_too_long` | over 2000 characters |
| `label_too_long` | roll label over 120 characters |
| `dice_empty`, `dice_syntax`, `dice_bad_sides`, `dice_bad_count`, `dice_too_many_dice`, `dice_too_many_terms`, `dice_bad_keep`, `dice_number_too_large`, `dice_too_long` | bad notation (`tt_rules::dice::ParseError::code`) |
| `rng_unavailable` | the OS gave no entropy; nothing changed, nothing was rolled |
| `unknown_user` | `set_controllers` or `create_token` names a non-member |
| `invalid_controller` | `set_controllers` or `create_token` names a spectator |
| `unknown_scene` | `set_level_background`, `create_token` or `start_encounter` names a scene that does not exist |
| `no_combatants` | `start_encounter` with no token ids |
| `encounter_running` | `start_encounter`: the scene already has a running encounter |
| `unknown_encounter` | `end_turn` (GM), `set_initiative` or `end_encounter` names an encounter that does not exist |
| `unknown_combatant` | `set_initiative`: the token is not in the encounter |
| `unknown_asset` | `set_level_background` names an `asset_id` this server never issued (empty clears) |

## Filtering rules and judgement calls

See ADR 0006 and `view.rs`. Where the contract is silent the more restrictive behaviour was chosen:

- Hidden tokens go to the GM only (snapshot and events), and players cannot move them. Controlling a hidden token reveals neither it nor its creature sheet.
- `hp_current`, `hp_max`, `hp_temp` go to the GM and listed controllers only, in snapshots and in every `TokenUpserted` (after damage, set HP and create too). `health_state`, `conditions` and `condition_timers` go to everyone who may see the token. The contract has no reveal mechanism, so none is implemented. Spectators never get hp, even if listed as controller. Hidden tokens go to the GM only.
- Full creature sheets go to the GM, to the owner, and to controllers of a visible linked token. Not to spectators.
- `set_controllers` also emits a `CreatureUpserted` for the token's creature, so a newly added controller receives the sheet. The filter decides who actually gets it. A controller who is removed keeps what they were already sent.
- Unspecified roll visibility is treated as not public (GM only) when filtering.
- `TokenRemoved`: the token is gone when the event is filtered, so `remove_token` records whether it was hidden (`Campaign.removed_token_hidden`). A removal of a visible token goes to everyone (spectators included); of a hidden one, or of an id never removed through an intent, to the GM only.
- `EventUndone` goes to a viewer exactly when that viewer may receive the undone original event (found in the log by `undone_seq`), filtered with the current state; one whose original is not in the log goes to the GM only. A `TokenRemoved` whose token undo has put back is judged by the restored token (its hidden flag), since the kept flag is deleted. Events with no kind go to the GM only.
- An encounter reaches a non-GM only if every combatant is a known, non-hidden token.
- All scenes and members go to every viewer; chat and public rolls go to everyone including spectators.
- Also stricter than the mock: unknown `level_id` is rejected; a non-GM gets `not_permitted` for a missing token; coordinates are bounded.

## Tests

The core is tested in `src/tests.rs`. `tests/serve.rs` checks `/healthz` and `/` over raw TCP. `tests/socket.rs` connects real WebSocket clients (dev-dependencies `tokio-tungstenite` and `futures-util`) to a server on an ephemeral port and covers:

- join: Snapshot identity and role; dev auth off; unknown user, malformed token, wrong campaign id; first frame not a Hello (an intent, garbage, empty); a text frame before the Hello; the 10 second Hello timeout (run on a paused tokio clock, so it takes no real time);
- broadcast: chat reaches everyone with the same `seq`; a rejection goes only to the sender, with its `client_intent_id` and code, and uses no `seq`; `seq` strictly increases;
- filtering over the wire: hidden tokens and hp in the Snapshot; a `GM_ONLY` roll and a hidden-token move reach the GM only; `set_controllers` delivers the token and creature to the new controller, who can then move it;
- use_ability: with the pregen seed and a fixed roller, a player's longsword hit for 6 reaches the GM and the player as the same `ResolutionUpdated` (a `DamageApplied` of 6), then a `TokenUpserted` for the goblin with `HEALTH_STATE_BLOODIED`: `hp_current` 3 for the GM, no hit point fields for the player, and no goblin sheet or other event;
- encounters: with the pregen seed and a fixed roller, the GM starts an encounter with Brannoch and the goblin; both clients receive the two `PUBLIC` initiative rolls and the `EncounterUpdated`; the player ends Brannoch's turn when it is current and gets `not_permitted` (to the sender alone) when it is the goblin's; a later join has the encounter in its Snapshot;
- undo: the GM undoes a player's goblin hit; the player's undo is refused to the player alone; both receive the goblin's restored `TokenUpserted` (no hit points for the player) and the two `EventUndone`;
- create and remove: the GM creates a hidden token and removes it, and the player receives neither event while the GM receives both; for a visible token everyone receives both (the player's copy has no hit points);
- media: a player and a spectator get tokens with the right URL, room and grants (payload decoded in the test) and nobody else sees anything; no `seq` used; repeated requests; media off gives `media_not_configured` to the requester only; a request before Hello closes the socket;
- hostile input: a message over 64 KiB (before and after joining), garbage after joining, text frames and a second Hello (ignored);
- a client that stops reading is dropped (broadcast lag, found by flooding until the socket buffers fill; 12000 chats of 2000 characters).

"Nothing arrives" is asserted with a sentinel chat that must be the next message, never with a sleep.

Not tested: the exact lag threshold of 256 events (the drop is tested, not the number); behaviour under many simultaneous connections; TLS and the Cloudflare tunnel (outside this crate).
