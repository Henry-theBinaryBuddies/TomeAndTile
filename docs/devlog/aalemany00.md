# Dev log: @aalemany00

Newest entry first. Format in [README.md](README.md).

## 2026-11-24 — Fix CI: web/src/lib/video/ compile against PR #88's interface additions

**Did:** Per Henry's PR #88 review: John's video panel (merged over the weekend) didn't account
for the three new `MediaFailureKind` values or the now-required `mic`/`camera`/`isSpeaking`
fields on `ParticipantInfo`, so `npm run check` broke on `main` the moment this PR's additions
existed. Had Claude add the minimum to compile in one commit: `view.ts`'s `FAILURE_TEXT` gained
the three entries (using the wording I'd already suggested in issue #93), and `track.test.ts`/
`view.test.ts`'s fakes and `person()` fixtures got the new required fields. Nothing about how the
panel actually shows these three failures changed — that's still John's call, same as #93 says.
Verified `npm run check` (0 errors, same pre-existing renderer-WASM gap as always), `npm test`
(365 passing), `tools/check-deps.sh` (OK). Commented on the PR summarizing the fix; GitHub had
already added Danny as a code-owner reviewer on these lines once I touched them.
**Directed Claude to:** Read Henry's latest review on PR #88 and fix exactly the three files and
lines he named, nothing more.
**Learned:** A heads-up issue filed as "no action needed unless you're building the panel now"
can go stale the moment someone *does* build the panel — #93 and #87 both needed a follow-up
comment once John's panel actually landed and the gap became a real CI failure, not a someday one.
**Blocked on / next:** Waiting on Henry's re-review and Danny's look at the three compiled-in
lines; once CI is green he merges #86 first, then this.

## 2026-11-23 — Fix: duplicate-identity reconnect loop (from Henry's real-call test)

**Did:** Henry's first real call against LiveKit Cloud found the same identity joining from two
browsers fighting over the connection: our module treated LiveKit's `DUPLICATE_IDENTITY`
disconnect the same as any dropped connection and rejoined after the backoff, which knocked out
the other tab, which rejoined in turn — 34 disconnects in a few minutes until one tab closed.
Had Claude implement his suggested fix: `room.ts` now classifies the `RoomEvent.Disconnected`
argument into a small `DisconnectCause` of our own (`'duplicate_identity' | 'removed' |
'room_closed' | 'other'`), never leaking LiveKit's own `DisconnectReason` type out of the file.
`client.ts` only calls `scheduleReconnect` for `'other'`; the three terminal causes report a
failure instead (`joined_elsewhere`, `removed`, `room_closed` — three new `MediaFailureKind`
values) and leave the call ended. Also checked Henry's "worth checking" note and handled
`PARTICIPANT_REMOVED`/`ROOM_DELETED` the same way, not just duplicate identity, since they're
the same kind of "the server meant to end this, don't loop" case. 8 new tests, including one
per terminal cause plus a control case confirming an ordinary drop still rejoins.
**Directed Claude to:** Read Henry's comment on PR #88 in full and implement the suggested fix,
including the two related reasons he flagged as worth checking.
**Learned:** Nothing to flag — matched his diagnosis and suggested fix directly; the three
`DisconnectReason` values needed were easy to find in `@livekit/protocol`'s own types.
**Blocked on / next:** A new `MediaFailureKind` is an interface addition, so it needs a heads-up
issue for Danny like the ones for #65 and #66 — filing that next.

## 2026-11-23 — #66 Fix: "waiting for video" never reached the panel

**Did:** Per Henry's review on PR #88. The required fix: `onParticipantsChanged` listened for
`TrackSubscribed`/`TrackUnsubscribed` and the local publish events, but never
`RoomEvent.TrackPublished`/`TrackUnpublished` for a remote participant. LiveKit fires
`TrackPublished` the moment someone turns their camera on, and `TrackSubscribed` only once the
video actually arrives — without the first event, state never rebuilt in between, so a tile
jumped straight from "camera off" to fully subscribed and skipped the "waiting for video" state
entirely (the issue's second acceptance item). Added both to `onParticipantsChanged`. Since the
fake room fires its callback directly and can't catch a missing event registration, added a
`room.test.ts` case (mocking `livekit-client`, same pattern as the existing `listLocalDevices`
pin) asserting `onParticipantsChanged` actually registers `TrackPublished`/`TrackUnpublished`,
not just the subscription events.
Also took the minor note: `onDisconnected`'s handler scheduled a rejoin without calling
`dispose()` on the old room first. Harmless under the generation check, but a real set of
listeners left behind on every reconnect over a long session. Now calls `nextRoom.dispose()`
there too, same as `teardown()` already did for an explicit `leave()`.
**Directed Claude to:** Read Henry's review on PR #88 in full and fix everything it raised,
required and minor. Merge the updated #65 branch in first, since #88 is stacked on it.
**Learned:** Nothing to flag — the fix matched Henry's diagnosis exactly once I checked
LiveKit's actual event firing order for `TrackPublished` vs `TrackSubscribed`.
**Blocked on / next:** Waiting on re-review. Henry's real-call test also turned up a duplicate-
identity reconnect loop, not part of this PR's acceptance list; picking that up separately.

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

## 2026-11-21 — #66 Track management: availability, blocked audio, listener clean-up

**Did:** Stacked on #65 (same two files). Had Claude add `mic`/`camera` (`TrackAvailability`:
`{published, subscribed}`) and `isSpeaking` to `ParticipantInfo`, so a tile can tell "camera
off" (not published) from "waiting for video" (published, not yet subscribed) instead of one
flat boolean. `attachTrack` now returns whether a track was actually there (the room wrapper
already knew; it just wasn't surfaced). Added `state.audioPlaybackBlocked` plus a
`startAudioPlayback()` method for the browser's autoplay block, wired to LiveKit's
`AudioPlaybackStatusChanged` event and `canPlaybackAudio`/`startAudio()`. `room.ts`'s
`onParticipantsChanged` now also listens for `LocalTrackPublished`/`LocalTrackUnpublished`/
`ActiveSpeakersChanged`, not just the remote-track events it already had. Added `RoomLike.
dispose()`: `wrapLiveKitRoom` now tracks every listener it registers and removes them on
teardown, instead of relying only on the generation check to make a stale callback a no-op.
9 new tests (37 total in `web/media/`), including one that proves `dispose()` actually
unregisters the fake room's callbacks, not just that state happens to ignore them afterward.
**Directed Claude to:** Read issue #66 in full and build exactly its acceptance list, on top of
the #65 branch, without touching anything outside `web/media/`.
**Learned:** Nothing worth flagging as a misread this round — the acceptance list mapped
cleanly onto LiveKit's own API (`TrackPublication.isSubscribed` is on the base class, so no
casting was needed between local/remote participants).
**Blocked on / next:** Heads-up issue for Danny and the PR itself, same as #65.

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
