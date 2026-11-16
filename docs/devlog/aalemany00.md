# Dev log: @aalemany00

Newest entry first. Format in [README.md](README.md).

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

## 2026-11-12 — #7 Machine setup and onboarding

**Did:** Accepted the GitHub invite and cloned the repo. Installed the Windows toolchain from `docs/SETUP-WINDOWS.md`: Rustup/cargo, `buf` via npm, and the MSVC C++ build tools. `cargo test --workspace` and `bash tools/check-deps.sh` both pass on this machine.
**Directed Claude to:** Read `docs/ONBOARDING.md`, `ARCHITECTURE.md`, the root `CLAUDE.md`, my directory's `CLAUDE.md` (`integrations/`, `web/media/`), and ADRs 0002, 0007, 0010, 0012 and 0013 before touching anything, then set up the machine and record this entry.
**Learned:** The MSVC Build Tools winget package does not pull in the Windows SDK by default even with `--includeRecommended`; `cargo test --workspace` failed with `LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'` until the SDK (`Microsoft.VisualStudio.Component.Windows11SDK.22621`) was added as a separate modify step. My area (`integrations/`, `web/media/`) can only depend on `contracts/` — never on `crates/server`, `crates/rules`, `crates/renderer`, or `crates/plugin-host` — enforced by `tools/check-deps.sh`. The `tt-cauldron-import` crate already exists as an empty skeleton; the importer itself is due 2026-12-06 (M3).
**Blocked on / next:** Watching the ONBOARDING.md videos (section 1, then the map-upload video in section 5) is still outstanding on my end. The field map for issue #8 is already up in a separate PR using the sample files Henry pushed to `integrations/cauldron-samples`.
