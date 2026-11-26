# Dev log: @aalemany00

Newest entry first. Format in [README.md](README.md).

## 2026-11-26 — #106 Cauldron importer: actions to abilities

**Did:** Had Claude fill in `Creature.abilities`/`Creature.resources` on top of #105's stat
block, in a new `abilities.rs` module. Every entry in `traits`, `actions`, `bonus_actions`,
`reactions` and `legendary_actions.options` becomes one `Ability`, in file order, with
`ability_id` a slug of the name, unique within the creature (a second "Claw" gets `claw-2`;
Voidbloom Seer's legendary "Mind Lash" colliding with its own attack action of the same name
exercises this directly). `attack`-kind entries become one `AttackStep` with a built dice
string ("1d4+3"/"1d4-1"/"6d6" with no `+0`) and `Targeting` (`TARGET_SHAPE_CREATURES`,
`max_targets` 1 only when Cauldron's wording is exactly "one target" -- Voidbloom Seer's Mind
Lash says "one creature" instead, which is reported and left unset, not guessed). `save`-kind
entries become one `SaveStep` plus an area `Targeting` (cone/line/cube/sphere, "radius" to
sphere per MAPPING.md), `recharge.min` to `Ability.recharge`. `multiattack` becomes a
no-`steps` ability with a plain description built from `sequence` ("Bite once, then Claw
twice"), always reported as resolved by hand -- still a real contract gap (MAPPING.md gap 5).
Legendary actions get a `legendary_actions` `Resource` plus one ability per option.
Spellcasting gets a passive "Spellcasting" ability from `intro`/`footer`, one `Resource` per
spell-slot level and per-day pool (per-day resource keys use `_`-slugged spell names, distinct
from `ability_id`'s `-` slugs), and one no-`steps` ability per spell, every one reported as
"spell effect not in the export" since Cauldron's schema only gives spell names, never their
mechanics. Checked: Cauldron's schema has no structured field for "this attack also applies a
condition" at all -- only the free-text `effects_text`, which none of the five samples sets --
so this importer can never produce an `ApplyEffectStep` from the current schema; said so in the
PR per the issue's own fallback instruction, since a sample can't be hand-written to prove
otherwise. Tests: one per sample checking the abilities exactly (matching the issue's list:
Cinderkite's one attack, Stormhide Drake's Bite/Claw/Stormbreath, Voidbloom Seer's legendary
resource plus the Mind Lash id collision, Hollowvale Hexer's spell-slot resources, Dune
Leviathan's multiattack and save ability), plus a structural comparison of Stormhide Drake's
Bite/Stormbreath against the hand-written shapes in `contracts/fixtures/characters/`. Updated
MAPPING.md's `effects_text`, `"text"`-kind, multiattack-phrasing and spellcasting-intro rows
now that the importer settles them.
**Directed Claude to:** Read issue #106 in full against MAPPING.md's action-kind table and
spellcasting section, the five samples' actual `traits`/`actions`/`legendary_actions`/
`spellcasting` data, issue #95 for the SRD condition ids, and the `Ability`/`EffectStep` shapes
in `contracts/fixtures/characters/`, then implement it end to end with tests.
**Learned:** Still no Rust toolchain in this session for an actual `cargo` run -- but I did find
a stale `target/` directory (gitignored, built on this machine at some earlier point outside
this session) with real `prost`-generated code for `tt-contracts`. Reading the generated
`tomeandtile.v1.rs` directly let me confirm every enum variant name, oneof wrapper shape and
scalar type (`Targeting.max_targets`/`Recharge.min_roll` are `u32`, not `i32`; `DamagePart`'s
`type` field is `r#type`) instead of guessing from the `.proto` source and hoping prost's naming
convention held. That closed most of the risk from writing Rust blind, though it's still not a
compile.
**Blocked on / next:** Still needs a real `cargo fmt`/`clippy`/`test` run before trusting this.
Stacked on #105 (branch `integrations/105-cauldron-stat-block`) -- merge that first.

## 2026-11-26 — #105 Cauldron importer: envelope, identity, core stat block

**Did:** Had Claude write `tt-cauldron-import` from scratch: `import_monster(bytes: &[u8]) ->
Result<Import, ImportError>`, a pure function reading a Creature Cauldron monster export into a
`Creature` (scores, AC, HP, all four movement speeds plus hover, saves, skills, resistances/
immunities/vulnerabilities, condition immunities, passive perception, and a one-line `summary`),
per `MAPPING.md`. Envelope checks (`app`/`kind`/`format_version`) run in order before anything
else is read. Input is capped on size (256 KiB, checked before parsing), JSON nesting depth,
list/object length and string length, walked once right after parsing; invalid UTF-8 and invalid
JSON are errors, never panics. Unknown skill names and unknown damage types are reported and
dropped rather than guessed at; a save-bonus mismatch (printed bonus vs. ability modifier +
proficiency bonus) is reported without changing the creature. Senses other than passive
perception (darkvision, blindsight, tremorsense, truesight) have no home on `Creature` and are
reported when a sample sets them. Tests: one per sample file in `tests/samples/` checking exact
mapped values, plus a save-mismatch and an unknown-skill case built in test code (not by editing
a sample), plus the built-in-code error cases from the issue's acceptance list (oversized input,
deep nesting, a list over its cap, a string over its cap, invalid UTF-8, truncated JSON, wrong
app/kind/format_version, a missing required field, an unknown size string). Traits, actions,
legendary actions and spellcasting are left empty — that's #106. Checked the server's
`crates/server/src/import.rs` (merged to main since I last looked) to make sure nothing there
assumes a different shape from this crate yet — it still 415s on any Cauldron-shaped file with
"arrives with #105", so #103 hasn't been wired to this crate's API yet; nothing to reconcile.
**Directed Claude to:** Read issue #105 in full against `MAPPING.md`, the five real sample
files and the `Creature`/`Provenance`/`AbilityScore`/`CreatureSize`/`DamageType` contract types,
then implement it end to end with tests.
**Learned:** No Rust toolchain is available in this session (confirmed `cargo`/`rustc` missing
before starting), so none of this has been run — not `cargo fmt --all --check`, not `cargo
clippy -p tt-cauldron-import --all-targets -- -D warnings`, not `cargo test -p tt-cauldron-import`,
not `bash tools/check-deps.sh` (that last one I *could* run, since it's a plain bash script with
no Rust in it, and it passes). I chose to have Claude write the rest anyway rather than wait for
a machine with the toolchain, on the understanding that it needs a real `cargo` run before I
trust it.
**Blocked on / next:** Run the three cargo checks above on a machine with Rust installed before
asking for review; fix whatever they turn up. Then #106 (actions to abilities) can build on this.

## 2026-11-23 — #65 Fix: no-choice state named a device nobody picked

**Did:** Per Henry's review on PR #86. The required fix: `refreshDevices` was writing
`list[0]?.deviceId` into `state.selectedDeviceId` even when nothing had actually been chosen,
which both claimed a choice that was never made and silently disabled the no-device/fallback
handling for a device the user never picked (since `selectedDeviceId[kind]` stayed `null`
internally while state showed otherwise). Now `null` is only ever replaced when there was a
real prior selection that's since gone missing; with no explicit choice, `null` stays as it is.
Added a test pinning it: startup listing with no choice leaves `selectedDeviceId` at `null` for
both kinds.
Also took the two optional suggestions: `RoomLike.setMicrophoneEnabled`/`setCameraEnabled` now
take an optional `deviceId`, so a remembered device is captured from the start instead of
opening the default and switching a moment later (no more flash of the wrong camera's light).
Documented in `index.ts` that the client is meant to be created once per app lifetime, not once
per panel mount, since there's no `dispose()` for the `devicechange` listener.
**Directed Claude to:** Read Henry's review on PR #86 in full and fix everything it raised,
required and optional.
**Learned:** Nothing to flag — the fix and both optional items were exactly as described.
**Blocked on / next:** Waiting on re-review.

## 2026-11-21 — #65 Device handling: changes, remembered choices, switch errors

**Did:** Had Claude add device management to `web/media/`: `MediaClientState` now carries
`devices` (`{audioinput, videoinput}`, each a live list) and `selectedDeviceId` (the chosen
device per kind, `null` for "no explicit choice"). Two new `MediaClientOptions`, both with real
defaults and fake-friendly overrides for tests: `listDevices` (defaults to the existing
`listLocalDevices`, which never asks for permission) and `onDeviceChange` (defaults to
`navigator.mediaDevices`'s `devicechange` event). The module lists both kinds at startup and on
every `devicechange`, re-lists a kind right after that kind's permission is first granted (to
pick up real labels), remembers a `selectDevice` choice made before joining and re-applies it
when the microphone or camera is first enabled and after every reconnect, falls back to another
device (or reports `no_device`) when the selected one disappears, and reports a failed
`switchActiveDevice` through `state.failure` without dropping the call. `room.ts`'s
`switchActiveDevice` now classifies its errors the same way the mic/camera calls already did.
28 tests pass (11 new), plus a new `room.test.ts` pinning that device listing never requests
permission (mocks `livekit-client` directly, since that call lives in the one file allowed to
import it). Ran the real `npm run check` / `npm test` this time — both work now that #46 widened
the globs.
**Directed Claude to:** Read issue #65 in full and build exactly its acceptance list, without
touching anything outside `web/media/`.
**Learned:** Claude's first pass had a real race: the startup device listing (fired at
construction, before any selection exists) could resolve *after* an immediate `selectDevice`
call and silently overwrite it, because the stale list it had already fetched didn't contain
the newly-chosen device yet. Caught by a test that called `selectDevice` right after
construction with no `await` in between. Fixed with a per-kind version counter: anything that
sets `selectedDeviceId[kind]` bumps it, and a `refreshDevices` call checks the counter after its
own (asynchronous) listing resolves, backing off if a newer change already happened. Separately,
the first test run crashed three *existing* tests that construct `createMediaClient` directly
(bypassing the new options' defaults) with "navigator is not defined" — the default
`onDeviceChange` assumed a browser. Made it a no-op outside one instead of throwing, which also
makes it safe under SvelteKit's SSR, not just under test.
**Blocked on / next:** Still need to open the heads-up issue for Danny listing the interface
additions (required before this PR can merge) and push/open the PR itself.

## 2026-11-19 — #34 Replace the pre-expiry renewal timer with backoff on reconnect

**Did:** Per Henry's review on PR #45: removed `scheduleRenew`/`renewTimer`/`renewBeforeMs`
(and the now-unused `now` option) entirely — LiveKit's client library refreshes its own token
on a network-drop reconnect, so a token's `expires_at_ms` only matters for the initial
connection, and the pre-expiry timer was tearing down and rejoining the call every ~9m50s for
no reason. Kept the rejoin-on-drop (still asking the server for a new token), with a backoff
delay before it: base 1s, doubling to a 16s cap.
**Directed Claude to:** Apply Henry's three numbered fixes from the review comment.
**Learned:** Resetting the backoff the instant `connect()` resolves would be wrong — a server
that accepts and immediately drops the connection would reset to the base delay every single
cycle, since each cycle *does* briefly reach "joined" before dropping again. That's exactly the
tight-loop scenario the backoff is supposed to prevent. Caught this while writing the test for
"growing on repeated failures, reset after a successful join," not before. Fixed by gating the
reset on the connection staying up for a few seconds (`STABLE_CONNECTION_MS`), not on `connect()`
succeeding.
**Blocked on / next:** `web/vite.config.ts`/`tsconfig.json` still don't cover `web/media/` (same
gap as the #34 entry below); verified again with a throwaway, uncommitted vitest config instead
of `npm test`. Henry said he's fixing that glob himself in a separate PR.

## 2026-11-15 — #8 Second pass: wire up the merged contracts fields

**Did:** Henry's [#16](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/16) merged,
adding every field the first round of corrections left as "pending contracts change." Updated
the branch onto current `main` again and replaced each pending row with the real field: `monster.id`
→ `Provenance.external_id`; `parent_id`/`generator`/`changes` → `Provenance.origin_notes`;
traits → `ACTION_COST_PASSIVE`; `recharge.min` → `Ability.recharge` (`Recharge{min_roll}`);
the four non-walk speeds → `fly_speed_feet`/`swim_speed_feet`/`climb_speed_feet`/`burrow_speed_feet`;
`speeds.hover` → `Creature.can_hover`; `skills` → `Creature.skills` (`repeated SkillBonus`);
`senses.passive_perception` → `Creature.passive_perception`; `condition_immunities` →
`Creature.condition_immunities`. Rewrote the "Gaps" section to say what #16 resolved instead of
what's pending; only multiattack (Milestone 2, rules-engine work) is still open.
**Directed Claude to:** Check whether #16 had merged, update the branch, and replace the
"pending contracts change" rows with the real fields it added.
**Learned:** This is the second round of "update the branch, it's stale again" on this PR —
worth watching for on any review that lands while a dependency (here, a contracts PR) is still
in flight.
**Blocked on / next:** Nothing of mine; waiting on Henry's re-review.

## 2026-11-15 — #34 LiveKit client module

**Did:** Built `web/media/` per ADR 0007: `createMediaClient()` exporting exactly join, leave,
mute/unmute, camera on/off, pick a device, list participants, attach/detach a track, and a
Svelte store to subscribe to state. `livekit-client` (2.24.1, the version ADR 0007 pre-approved)
is imported only in `room.ts`, which adapts a real `Room` to an internal `RoomLike` interface;
`client.ts` (the state machine) and the exported `MediaClient` type never see a LiveKit type.
17 unit tests run against a hand-written fake `RoomLike`, no LiveKit import and no real SFU,
covering: join connects with the token's `sfu_url`/`token` and only then asks for mic/camera
permission; a spectator token (`can_publish: false`) never opens either; the client asks for a
new token and rejoins both ~10s before `expires_at_ms` and on an unexpected disconnect; `leave`
tears down cleanly and a stale in-flight rejoin can't resurrect a left call; mute/unmute and
camera on/off; permission-denied/no-device/token-refused/connection-lost each land as a typed
`MediaFailure` without crashing the client; the token never appears in state or console output.
**Directed Claude to:** Read issue #34, ADR 0007 and `web/media/CLAUDE.md`, then build the
module and its tests without touching anything in `web/` outside `web/media/`.
**Learned:** `web/vite.config.ts`'s test `include` is `src/**/*.test.ts` and `web/tsconfig.json`'s
`include` is `["src", "vite.config.ts"]` — neither looks at `web/media/` at all. Both files are
outside this area (Danny's), so per `web/media/CLAUDE.md` ("stop and ask" for a file outside
`web/media/`) I verified the module with throwaway local-only configs instead of editing them,
and I'm filing an issue for whoever owns those files to widen the globs, since `npm run check`
and `npm test` from `web/` currently skip this module entirely despite issue #34 listing them as
acceptance criteria.
**Blocked on / next:** The `check`/`test` glob gap above. `npm run build` passes as-is (`web/media/`
isn't imported into the app yet, so the gap doesn't show up there). `npm run check` and `npm test`
were run by pointing `tsc`/`vitest` at `media/` directly with throwaway local-only configs
(deleted, not committed) instead of the real `npm run check`/`npm test`, until the glob gap above
is fixed.

## 2026-11-15 — #33 Asset pipeline: inspect and describe uploaded images

**Did:** Added `tt-asset-pipeline`, a pure `inspect(bytes) -> Result<AssetRef, AssetError>`:
sniffs PNG/JPEG/WebP (lossy `VP8 `, lossless `VP8L`, and extended `VP8X`) from each format's own
header bytes, never decoding the full image; computes `asset_id` as the lowercase hex SHA-256 of
the file; rejects empty files, files over 20MiB, unknown formats, SVG (by content, not
extension), truncated headers, 0 or >16384 width/height, and >100,000,000 pixels. Both SHA-256
and the three header parsers are hand-written with zero dependencies, since issue #33 says to
stop and ask before adding one (`sha2`/an image-decoding crate) and the real sample files are
all well under a size where that mattered. Tests use five real PNG/JPEG/WebP samples generated
with Pillow in `tests/samples/`, plus hand-built hostile inputs (truncated headers, a header
claiming 20000x20000, all-random bytes, an SVG with an `onload`) asserting nothing panics.
**Directed Claude to:** Read issue #33, `integrations/CLAUDE.md` and ADRs 0008/0010, then build
the crate without adding a dependency or touching the root `Cargo.toml`.
**Learned:** Issue #33 says to add the crate to the root `Cargo.toml` workspace members myself,
"ask @Henry-theBinaryBuddies ... if the rules stop you" — and the rules do: root files are his
per the working agreement. Left the one-line addition for him rather than self-editing root
`Cargo.toml`, same call as the `web/media/` check/test glob gap on the #34 branch.
**Blocked on / next:** This crate isn't a workspace member yet (needs that one line from Henry),
so `cargo test -p tt-asset-pipeline` etc. can't run from the repo root until then. More importantly:
*no Rust toolchain was installed in this session at all* (no `cargo` on `PATH` anywhere I could
find), so none of `cargo fmt`, `cargo clippy` or `cargo test` actually ran — not here, not as a
standalone crate, not anywhere. Everything above is from careful manual review (including
checking the SHA-256 test vectors against Python's `hashlib` by hand) and real sample files, not
a compiler. This genuinely needs a `cargo build`, `cargo test -p tt-asset-pipeline` and
`cargo clippy -p tt-asset-pipeline --all-targets -- -D warnings` on a machine with the toolchain
before anyone trusts this compiles, let alone merges.

## 2026-11-12 — #8 Field map corrections from Henry's review

**Did:** Updated `integrations/8-cauldron-field-map` branch onto current `main` (clean, only `MAPPING.md` left in the diff) and fixed every correction from Henry's PR #11 review: recount of skills (2/5, not 5/5) and non-walk speeds (4/5, not 3/5); a rule for `speeds.walk` being absent (Voidbloom Seer — default to 0, never borrow another mode); an explicit dice+bonus combining rule for a zero and a negative bonus; moved `monster.id` off `attribution` onto a new pending `Provenance.external_id` field; moved traits off `ACTION_COST_FREE` onto a pending passive `ActionCost` value; set `resource_key = "legendary_actions"` on legendary options so the cost is actually spent from the pool; restricted `Provenance.attribution` to the raw SRD text only (parent_id/generator/changes now point at a separate pending contracts field); defined the `per_day` resource-key slug rule; and folded in Henry's gap decisions (save bonuses computed+compared instead of a contract change, multiattack as an interim description-only `Ability` pending Milestone 2 rules-engine work, `radius`→`TARGET_SHAPE_SPHERE` and `CreatureSize` both confirmed).
**Directed Claude to:** Pull up PR #11 on GitHub, read Henry's review in full, update the branch, and apply each correction and gap decision to `MAPPING.md` without touching anything outside `integrations/`.
**Learned:** Henry is filing the five contract-change issues himself as one `contracts/` PR rather than me opening them — the mapping now says "pending contracts change" on the affected rows instead of promising to file issues.
**Blocked on / next:** Waiting on Henry's `contracts/` PR (external_id, passive ActionCost, per-mode speeds, skills/passive scores, condition immunities, ability recharge) before the importer itself can be written.

## 2026-11-12 — #7 Machine setup and onboarding

**Did:** Accepted the GitHub invite and cloned the repo. Installed the Windows toolchain from `docs/SETUP-WINDOWS.md`: Rustup/cargo, `buf` via npm, and the MSVC C++ build tools. `cargo test --workspace` and `bash tools/check-deps.sh` both pass on this machine.
**Directed Claude to:** Read `docs/ONBOARDING.md`, `ARCHITECTURE.md`, the root `CLAUDE.md`, my directory's `CLAUDE.md` (`integrations/`, `web/media/`), and ADRs 0002, 0007, 0010, 0012 and 0013 before touching anything, then set up the machine and record this entry.
**Learned:** The MSVC Build Tools winget package does not pull in the Windows SDK by default even with `--includeRecommended`; `cargo test --workspace` failed with `LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'` until the SDK (`Microsoft.VisualStudio.Component.Windows11SDK.22621`) was added as a separate modify step. My area (`integrations/`, `web/media/`) can only depend on `contracts/` — never on `crates/server`, `crates/rules`, `crates/renderer`, or `crates/plugin-host` — enforced by `tools/check-deps.sh`. The `tt-cauldron-import` crate already exists as an empty skeleton; the importer itself is due 2026-12-06 (M3).
**Blocked on / next:** Watching the ONBOARDING.md videos (section 1, then the map-upload video in section 5) is still outstanding on my end. The field map for issue #8 is already up in a separate PR using the sample files Henry pushed to `integrations/cauldron-samples`.
