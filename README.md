# Tome & Tile

An open-source virtual tabletop for D&D 5e: a shared map, synced tokens, server-rolled dice, video and voice, and character sheets where clicking an ability and a target rolls the attack or save and applies the damage.

Status: Milestone 0 (skeleton). Nothing is playable yet.

## Start here

| If you want to | Read |
|---|---|
| Understand what a VTT is | [docs/ONBOARDING.md](docs/ONBOARDING.md) |
| Set up your Windows machine | [docs/SETUP-WINDOWS.md](docs/SETUP-WINDOWS.md) |
| Understand the design | [ARCHITECTURE.md](ARCHITECTURE.md) |
| See why a decision was made | [docs/adr/](docs/adr/) |
| Know what you may edit | [CLAUDE.md](CLAUDE.md) and the `CLAUDE.md` in your directory |
| Pick up work | GitHub issues labelled with your area; drafts in [docs/backlog/](docs/backlog/) |

## Layout

| Path | What it is | Owner |
|---|---|---|
| `contracts/` | Protobuf definitions: the only shared surface | @Henry-theBinaryBuddies |
| `crates/rules` | 5e rules engine | @Henry-theBinaryBuddies |
| `crates/server` | Authoritative game server | @Henry-theBinaryBuddies |
| `crates/renderer` | WASM/WebGPU map canvas | @Henry-theBinaryBuddies |
| `crates/plugin-host` | Sandboxed WASM plugin host | @Henry-theBinaryBuddies |
| `integrations/` | Importers and the asset pipeline | @aalemany00 |
| `web/media/` | WebRTC video and voice client module | @aalemany00 |
| `web/` (everything else) | UI shell, sheets, chat, panels | @Danny-Defeato |

## License

AGPL-3.0-only. Plugins that run in the sandbox and use only the published capability API may carry any license; see [ADR 0009](docs/adr/0009-extensions-and-license.md).
