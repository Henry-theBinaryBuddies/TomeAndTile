# Dev log: @Henry-theBinaryBuddies

Newest entry first. Format in [README.md](README.md).

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
