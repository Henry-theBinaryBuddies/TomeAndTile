# Dev log: @Henry-theBinaryBuddies

Newest entry first. Format in [README.md](README.md).

## 2026-11-21 — Asset upload, the M2 rules engine, target picking, reviews

**Did:** Merged 15 pull requests this week:
- **Asset upload (M1):** ADR 0014 ([#50](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/50)), the contract, mock server and real server ([#55](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/55) to [#57](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/57)).
- **All my M2 issues, 58 to 64:** the seed and codes ([#71](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/71), [#72](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/72)), attack resolution and targeting checks ([#73](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/73), [#75](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/75)), `UseAbilityIntent` on both servers ([#80](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/80), [#83](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/83)), and picking targets on the map ([#84](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/84)).
- **Contributors:** merged #11, #44, #45, #47 and #74 after review, filed the 13 M2 issues, and posted reviews on #77, #78, #79, #81, #82, #86 and #88.

410 Rust and 188 web tests pass. Wrote the 2026-11-22 check-in report.

**Directed Claude to:** Lead and review while Sonnet subagents and multi-agent review workflows wrote the routine code. Each finding was checked by a skeptic before anything changed. Corrections:
- A server test bound `0.0.0.0` and set off a Windows firewall prompt. I asked what the program was, and every test now binds `127.0.0.1` only.
- Claude fixed one leak itself after review: a visible monster's `not_enough_resource` warning would have told players how many uses it had left.
- A merge was refused because "continue in order" was not an explicit yes. I said yes plainly and it went ahead, with no workaround.

**Learned:**
- A mouse-drag threshold of 3 px is too tight for a finger tap, so picking uses its own 8 px allowance.
- A browser pane with a 2.5x pixel ratio can make clicks land in the wrong place; a fixed 800 by 600 viewport made the browser check trustworthy.
- GitHub silently runs no pull request CI while a branch has merge conflicts (issue 49).
- Tests that pass trivially are easy to write: the review found five picking tests that would have passed against broken code.

**Blocked on / next:**
- Issue 5 (the LiveKit Cloud project) is mine and is the only M1 item not met; no real call has run yet.
- Next: close it and try a two-browser call, re-review the contributors' fixes, add the server rate limit and event-log cap, and plan M3.

## 2026-11-15 — Image proof, M1 issues, renderer packaging, contributor reviews

**Did:** Merged [#27](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/27) (check-in report), [#28](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/28) (renderer demo with a map and token portraits, which proved the image path in a browser) and [#41](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/41) (the renderer packaged for `web/`, closing issue 35). Filed the eight Milestone 1 issues (29 to 36) and assigned issues 9 and 10. Approved @Danny-Defeato's #24, #25 and #38 to #40, and merged #38 to #40 in order; refreshed the check-in report. 179 Rust and 113 web tests pass.
**Directed Claude to:** Review the contributor pull requests by running them, not only reading: the web checks locally, and the three panels in a browser against the mock server, including a chat message containing HTML tags and a 360 px wide screen. Claude found nothing blocking and listed small things. A decision of mine: where upload runs is not obvious from the dependency rule, so it is filed as a decision for me (issue 36) instead of being invented inside a contributor's issue.
**Learned:** SvelteKit type-checks JavaScript by default, so the generated glue file for the renderer produced about 2,000 errors until its first line told the checker to skip it. Three stacked pull requests merge cleanly only in order, and the third has to be retargeted to `main` first or it merges into the wrong branch. An approved pull request is not a merged one: #37's content had already reached `main` through the stack, so it only conflicts now.
**Blocked on / next:** Issue 5, the LiveKit Cloud project, is still not created, so the join token has never been tried against real LiveKit. Next: issue 36 (ADR on where upload runs), a server rate limit and event-log cap, then Milestone 2.

## 2026-11-13 — Milestone 1 server, canvas and join token

**Did:** Merged 10 pull requests ([#15](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/15) to [#23](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/23) and [#26](https://github.com/Henry-theBinaryBuddies/TomeAndTile/pull/26)): contract additions and five SRD 5.2 characters, the dice engine, ADR 0006 (dice-box), server sync version 0 with per-viewer filtering, 19 WebSocket tests, renderer canvas version 0, the LiveKit join-token endpoint, and a web CI job. Approved pull requests #24 and #25 from @Danny-Defeato after reading them and running #25's checks locally. 179 tests pass.
**Directed Claude to:** act as lead and reviewer while Sonnet sessions wrote the dice engine, server, socket tests, renderer and join-token code against written briefs, and to ask before adding any dependency. Corrections I made after review: the cleric had no source for heavy armour and the rogue's Cunning Action was marked passive instead of a bonus action; a first server session started in the wrong worktree, wrote no code and was relaunched; I added a guard in the renderer for non-finite canvas sizes. Decision of mine: dice animation uses the dice-box library, already used on the portfolio.
**Learned:** A delegated session's "it works" is not evidence: the renderer only counted as working after I watched it draw and move a token against the real server. The wgpu graphics library fits under the workspace's `unsafe_code = "forbid"` rule. The SRD character values were written from memory and still need checking against the SRD.
**Blocked on / next:** Issue 5, the LiveKit Cloud project: the join token has never been tried against real LiveKit. Next: file the Milestone 1 issues for the other two, prove renderer images, add a rate limit and event-log cap to the server, start Milestone 2.

## 2026-11-12 — Toolchain installed, skeleton verified

**Did:** Installed Rust, the C++ build tools and Buf. The workspace passes `cargo fmt`, `clippy`, `test` and the WASM build; the contracts pass `buf lint` and `buf build`. Removed two unused manifest entries that Cargo warned about.
**Directed Claude to:** diagnose a wall of "not recognized" errors. The installs had worked; the terminal had a stale PATH and PowerShell blocked `npm.ps1`.
**Learned:** On Windows, use `npm.cmd` when script execution is disabled, and restart the app hosting the terminal after an install. Both are now in `docs/SETUP-WINDOWS.md` for the other two.
**Blocked on / next:** Create the GitHub remote and protect `main`.

## 2026-11-12 — Video decision: SFU

**Did:** Accepted ADR 0007 with LiveKit as the SFU (LiveKit Cloud for the class). Cut `media.proto` down to a join-token request and reply and removed the signalling messages.
**Directed Claude to:** give a preference between mesh and an SFU after I said TURN had not worked well in Couch Crew. Claude withdrew its own mesh proposal.
**Learned:** Mesh depends entirely on TURN on strict networks; an SFU needs only one outbound connection per browser.
**Blocked on / next:** Create the LiveKit Cloud project and check its free-tier limits.

## 2026-11-12 — Milestone 0 skeleton

**Did:** Ran a long design interview with Claude and settled the architecture. Created the repository skeleton: `ARCHITECTURE.md`, 13 ADRs, contracts v0 (seven Protobuf files), the Cargo workspace with empty crates, CODEOWNERS, per-directory `CLAUDE.md` files, issue and PR templates, CI, the boundary check script, onboarding and Windows setup documents.
**Directed Claude to:** ask exhaustive questions before writing anything; keep boundaries strict and coupling loose so delegated work cannot break other people's; do not automate death saves; player rolls always public.
**Learned:** A Cloudflare Tunnel cannot carry WebRTC media, which pushed video towards peer-to-peer with Cloudflare TURN (ADR 0007, still Proposed).
**Blocked on / next:** Rust, Buf and Docker are not installed on this machine yet, so the workspace and the contracts have not been compiled or linted. Next: install them, fix whatever they report, create the GitHub remote, set branch protection, file the Milestone 0 issues from `docs/backlog/milestone-0.md`.
