# tt-mock-server

A stand-in for the game server. It speaks the real game-socket protocol (binary WebSocket frames, each a `ClientMessage` up or a `ServerMessage` down) from recorded data, so the web UI and importers can be built before the real server exists.

> **Warning: this is not the server.** It applies simplified rules, and the only permissions it checks are the GM role and token control, so the UI can exercise those refusal paths; that is still not security. Its per-viewer filtering is the creature sheets in the Snapshot and `creature_upserted` events, and token hit points (see below). Everything else goes to everyone: every client may move any token, hidden tokens and hidden-token initiative rolls are not hidden, and every die is public. A UI must never rely on the mock for anything the real server will hide or refuse. Do not add real rules or permission logic here; that belongs in `tt-server` and `tt-rules`.

## Run

```bash
cargo run -p tt-mock-server
```

It prints `Tome & Tile mock server listening on ws://127.0.0.1:8787/ws`. The port defaults to 8787; set `TT_MOCK_PORT` to change it. It binds `127.0.0.1` only. `GET /` returns a one-line text description.

The starting state is the pregen table seed, `contracts/fixtures/seeds/pregen-table.json`: one campaign, one square-grid scene (cell 70, 5 ft per cell), eight tokens and seven creature sheets. The tokens are "Aria" (`token-aria`, no creature, 24 HP), a "Goblin" (`token-goblin`, creature `creature-goblin`, 9 of 9 HP), an "Ember Wisp" (`token-ember-wisp`, creature `creature-ember-wisp`, 11 of 11 HP, with Choking Ash) and the five pregenerated characters (`token-brannoch`, `token-tamsin`, `token-korvath`, `token-maribel`, `token-ilvaren`, each linked to a creature). `mock-player` controls Aria and the five pregens. State is shared by every connection and lives in memory; restart the process to reset it.

## Protocol behaviour

- The first binary frame must be a `ClientMessage` with `hello`. The reply is a `Snapshot` of the current state (`seq` is the last event so far). Anything else first closes the connection.
- Identity: `session_token` of exactly `gm` is user `mock-gm` with role GM; any other token is user `mock-player` with role PLAYER. `campaign_id` and `last_seq` are ignored (there is no replay; reconnect and use the Snapshot).
- Incoming messages are limited to 64 KiB. Text frames are ignored. A frame that does not decode as a `ClientMessage` closes that connection only. A second `hello` is ignored.
- Creatures are per viewer, following `creature_visible` in `tt-server`: `mock-gm` gets every creature, `mock-player` only the creatures linked to a visible token it controls (or that it owns), so the five pregens and not the goblin's. Token hit points are redacted per viewer, as `redact_token` does in `tt-server`: `hp_current`, `hp_max` and `hp_temp` are left unset unless the viewer controls the token (`mock-gm` controls every token; `mock-player` the tokens that list it in `controller_user_ids`). The Snapshot and every `token_upserted` are redacted this way; `health_state`, `conditions` and everything else go to all. `creature_upserted` (sent by `create_token` when it lists controllers) goes only to viewers who may see that sheet. Hidden tokens are not filtered.
- Every event goes to every connected client that has said hello, including the sender, each encoded for that viewer. Events carry a strictly increasing global `seq`, the server clock in `at_ms`, the fixture scene id, the sender as `actor_user_id` and the causing `client_intent_id`.
- One intent produces a group of events with consecutive `seq` and the same `client_intent_id`; no other event falls inside a group.
- A client that falls more than 256 events behind is disconnected; it can reconnect and get a fresh Snapshot.

| Intent | What the mock does |
|---|---|
| `move_token` | Sets the token's position to the last waypoint (and `level_id` if given). Broadcasts `token_moved` with `distance_feet` = straight-line path length from the old position, divided by `cell_size`, times `feet_per_cell`. Rejects `unknown_token` or `empty_path`. Undoable. |
| `chat` | Broadcasts `chat_posted`. Rejects `empty_chat`, or `chat_too_long` above 2000 characters. |
| `roll_dice` | Notation `NdM`, `NdM+K`, `NdM-K` (N 1-100, M one of 4, 6, 8, 10, 12, 20, 100, whitespace allowed). Advantage or disadvantage applies only to a single d20 (two dice, one `dropped`). Always PUBLIC visibility, purpose FREEFORM. Broadcasts `dice_rolled`. Rejects `bad_notation`. The random source is a clock-seeded xorshift and is not suitable for the real server. |
| `set_level_background` | GM only. Sets the level's background to the uploaded `AssetRef` (the same one the upload returned), or clears it when `asset_id` is empty, and broadcasts `scene_upserted` with the updated scene; later Snapshots include it. Rejects `not_permitted` (sender is not `mock-gm`), `unknown_scene`, `unknown_level`, or `unknown_asset` (non-empty id that was not uploaded). |
| `use_ability` | See below. Undoable. |
| `set_hit_points` | The GM, or a controller of a visible token. Sets `hp_current` and `hp_temp` (both always set) and recomputes `health_state`; answers `token_upserted`. Rejects `not_permitted` (a player who does not control the token or names a missing or hidden one), `unknown_token` (GM, missing), `no_hit_points` (no `hp_max`), `bad_hit_points` (`hp_current` outside 0 to `hp_max`, `hp_temp` outside 0 to 999). Undoable. |
| `set_conditions` | GM only. Removes the `remove` ids, then adds the `add` ids that are absent; always answers `token_upserted`. Rejects `not_permitted`, `unknown_token`, `bad_condition_id` (not `[a-z0-9_]{1,40}`, in either list), `conflicting_conditions`, `too_many_conditions` (over 32 afterwards). Sends no `condition_immune` warning and tracks no timers. Undoable. |
| `create_token` | GM only. Mints `token-new-<n>` (n counts up from 1, only for tokens that were made), copies HP from the creature (a prop, or a creature with no `hp_max`, has none), sets `health_state`, starts with no conditions, answers `token_upserted`, and with controllers and a creature also `creature_upserted`. Name defaults to the creature's, else `Token`; size to the creature's, else Medium; level to the scene's first. Rejects, in this order, `not_permitted`, `unknown_scene`, `unknown_level`, `unknown_creature`, `bad_coordinate` (position missing or not finite; bounds are not checked), `name_too_long` (over 120 characters), `unknown_user` (not a member), `invalid_controller` (a member who is not a player), `too_many_tokens` (500). Undoable. |
| `remove_token` | GM only. Answers `token_removed`. Rejects `not_permitted`, `unknown_token`, `in_encounter` (the token is a combatant of a running encounter). Undoable. |
| `start_encounter` | GM only. Rolls d20 + the creature's `initiative_bonus` (0 for a token with no creature) per token, in list order with duplicates listed once: one `dice_rolled` each (purpose INITIATIVE, label `<token name> initiative`, PUBLIC even for a hidden token), then `encounter_updated` with `encounter-<n>`, round 1, turn 0, combatants sorted by initiative (higher first, ties in list order) and `order` set to the position. Rejects `not_permitted`, `unknown_scene`, `no_combatants`, `unknown_token` (missing, or not on that scene), `encounter_running` (one per scene). Not undoable. |
| `end_turn` | The GM, or a controller of the current combatant's token. Moves to the next combatant; past the last, back to turn 0 and the next round. Answers `encounter_updated`. Rejects `unknown_encounter` (checked first, for everyone, since the sender's right depends on the encounter), then `not_permitted`. Not undoable. |
| `set_initiative` | GM only. Overwrites the combatant's `initiative` and `order`, re-sorts (higher initiative first, then lower `order`, then the earlier position), renumbers `order` to the new positions and keeps the turn on the same combatant. Answers `encounter_updated`. Rejects `not_permitted`, `unknown_encounter`, `unknown_combatant`. Not undoable. |
| `end_encounter` | GM only. Answers `encounter_ended`; later Snapshots have no encounter. Rejects `not_permitted`, `unknown_encounter`. Not undoable. |
| `undo` | GM only; see below. |
| anything else (`set_controllers`) | Rejects `not_supported_by_mock`, echoing `client_intent_id`. |
| `media_token_request` | Ignored; no reply. |

Rejections go to the sender only and do not use a sequence number.

## use_ability

The GM, or a controller of the source token, may use an ability. The mock broadcasts one `resolution_updated` event with the resolution in state DONE, then one `token_upserted` per target token that changed (HP, temporary HP, conditions or health state), in target order. Steps run in order:

- **Attack.** A d20 + `to_hit_bonus` roll (purpose ATTACK, notation like `1d20+5`, label `<Ability> attack`) against the target creature's `armor_class`; a natural 20 is a critical hit, a natural 1 a miss; `mode` gives advantage or disadvantage. On a hit, one DAMAGE roll per damage part (label `<Ability> damage`), with the dice doubled and the modifier unchanged on a critical hit. One outcome per target; with one attack step every listed target is attacked. With several attack steps (Scorching Ray) there is one outcome per step, pairing step *i* with target *i*; targets past the last step are ignored (`extra_targets_ignored`) and steps past the last target are not rolled. `step_index` is the index into `Ability.steps`.
- **Save.** Each target rolls d20 + its ability modifier (`(score - 10)` halved, rounded down) + `proficiency_bonus` if the save is in `save_proficiencies` (purpose SAVE, `roller_creature_id` the target, label `<Ability> save`; the modifiers are listed as `ability` and `proficiency`), against `dc`: a total below it is `SAVE_FAILED`, at or above `SAVE_SUCCEEDED`. All saves are rolled first, then the damage once for the step (no damage is rolled if nobody takes any): full on a failure, halved (rounded down) on a success when `half_on_success`, none otherwise. Every outcome that takes damage lists the same damage rolls.
- **Flat.** Each target gets an `AUTOMATIC` outcome. The damage parts are rolled once for the step, then the `healing_dice` (purpose HEALING, listed in `damage_rolls`); `healing` is the hit points actually restored, capped at `hp_max`.
- **Apply effect.** Adds `effect_id` to the target token's `conditions` (once) and lists it in the `effects_applied` of that target's latest outcome (the attack, save or flat result before it), or in a new `AUTOMATIC` outcome if there is none. With `only_on_failure` it applies only after a hit, a critical hit, a failed save or an automatic result. No immunity check, no timer, no `duration_not_tracked` warning.
- A **plugin** step, or dice text outside `NdM`, `NdM+K`, `NdM-K`, is `not_supported_by_mock`. An ability with several attack steps and any other step is `not_implemented`, as in the real server.

Damage: every `DamageApplied` has `applied` equal to `rolled` (half on a successful save), because resistance, immunity and vulnerability are skipped. Temporary HP is spent first, `hp_current` floors at 0, and `dropped_to_zero` is set on the outcome that takes the token from above 0 to 0. Damage from one use is applied in outcome order, so a token hit twice is reduced twice. `health_state` is recomputed with the thresholds in `scene.proto`. A target token with no `hp_max` keeps its HP: the damage or healing is shown, and `no_hit_points` is warned once per such token.

Targets: `TARGET_SHAPE_SELF` targets the source token and ignores `target_token_ids`. Area shapes (sphere, cone, line, cube) resolve against the listed tokens, with `area_not_checked` once and no template. A creature-targeting ability needs at least one target. `roller_creature_id` is the source creature (the target for a save), `roller_user_id` the sender, visibility PUBLIC. `resolution_id` is `res-<n>` and each `roll_id` is `roll-<n>-<k>`, unique per use; freeform rolls stay `roll-<seq>`. An ability with no steps (a passive) resolves with no outcomes.

`too_many_targets` (one attack step, `max_targets` above 0, more targets than that; every target is still resolved), `extra_targets_ignored`, `area_not_checked` and `no_hit_points` go in `Resolution.warnings` and `Event.warnings`.

Rejections, checked in this order: `not_permitted` (a player naming a source token it does not control, or one that does not exist or is hidden), `unknown_token` (the GM naming a missing one), `no_creature`, `unknown_ability`, `no_targets` (a creature-targeting ability with no targets), `unknown_target` (a target token that does not exist, is hidden from a player, or has no creature), then `not_supported_by_mock` or `not_implemented` as above.

## undo

GM only. Every accepted intent's events form a group (consecutive `seq`). The mock keeps the earlier state of each undoable group (`use_ability`, `set_hit_points`, `set_conditions`, `move_token`, `create_token`, `remove_token`) and can undo only the most recent undoable group that has not been undone; undoing it makes the one before it the latest. `event_seq` must be the group's first event. The mock sends a compensating `token_upserted`, `token_moved` or `token_removed` per token the group changed, with the earlier state, then one `event_undone` per event of the group, all as one group of their own. Rejects, in this order: `not_permitted` (not the GM), `unknown_event` (seq 0 or beyond the log), `not_first_event`, `not_undoable` (chat, dice, background, encounter intents, undo itself, and the compensating events), `already_undone`, and `not_supported_by_mock` for any older group (the real server has a dependency check, `undo_blocked`, instead).

## What the mock skips

Resistance, immunity and vulnerability; condition immunity (never `condition_immune`) and the roll effects of conditions; condition timers (so `condition_timers` stays empty, `end_turn` and `end_encounter` never upsert a token, and `duration_not_tracked` is never sent); range (never `out_of_range`); resources and recharge (never `not_enough_resource` or `ability_recharging`, nothing is spent, and creatures stay as seeded); undo of anything but the latest group, and any dependency check; hidden tokens and hidden initiative rolls; concentration, reactions and area templates; `action_used`, `reaction_used` and movement tracking on combatants. Death saves are not automated, as in the real server. The Creature Cauldron import endpoint is not here either: use the real server (#103).

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
