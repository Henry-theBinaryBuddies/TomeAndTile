# 0002: Repository, ownership and boundaries

Status: Accepted (2026-11-12)

## Decision

- Tome & Tile is a standalone repository on a personal GitHub account, separate from DndPortfolio.
- Work is split by layer. Each person owns directories nobody else merges into without their approval.

| Owner | Owns |
|---|---|
| @Henry-theBinaryBuddies | `contracts/`, `crates/rules`, `crates/server`, `crates/renderer`, `crates/plugin-host`, CI, ADRs |
| @aalemany00 | `integrations/` (importers, asset pipeline), `web/media/` (WebRTC client module) |
| @Danny-Defeato | `web/` except `web/media/` (UI shell, sheets, chat, panels, PWA) |

- Enforcement is three layers:
  1. **GitHub:** CODEOWNERS, protected `main`, required review by the owner.
  2. **Claude:** a `CLAUDE.md` in each directory telling that contributor's Claude what it may edit and to stop when a task crosses a boundary.
  3. **CI:** `tools/check-deps.sh` fails a PR whose code imports across a boundary; `buf breaking` fails a breaking contract change.
- Independent work is possible because of a mock server that replays recorded contract messages, so `web/` and `integrations/` do not wait on the real server.

## Why

- Two contributors are new to this kind of system and all three use their own Claude instance. Clear directory ownership is the cheapest way to stop accidental cross-edits.
- The project owner keeps the contracts, rules engine, server and renderer because they are the hardest and most shared parts.

## Rejected

- **A branch inside DndPortfolio:** would give collaborators access to unrelated coursework and inherit that repository's rules.
- **Split by feature:** every feature touches shared state, so collisions are much more likely.
- **Everyone anywhere, coordinated by review:** no protection.
