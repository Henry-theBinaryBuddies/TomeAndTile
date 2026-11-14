# Dev log: @Henry-theBinaryBuddies

Newest entry first. Format in [README.md](README.md).

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
