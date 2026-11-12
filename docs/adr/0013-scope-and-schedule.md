# 0013: Class scope and schedule

Status: Accepted (2026-11-12)

## Decision

The class final is due 2026-12-13. The project is not expected to be finished by then. The target for the final is a playable core loop.

### In scope for 2026-12-13
- Shared map, tokens and synced movement
- Chat and server-rolled dice with a 3D animation
- Character sheets with click-to-attack, automatic saves and damage, GM undo and override
- Pregenerated SRD characters
- Creature Cauldron monster import
- Video and voice
- Phone layout
- Deployed on the home server

### After the class
Fog and dynamic lighting, multi-level maps, Universal VTT and animated maps, plugins, journals, macros, audio, OAuth, reactions, the remaining importers.

### Milestones

| Due (Sunday) | Milestone | Video and voice |
|---|---|---|
| 2026-11-15 | M0: repository, contracts v0, CI, ownership, CLAUDE.md files, ADRs, onboarding, mock server | Signalling messages defined in the contracts |
| 2026-11-22 | M1: map image upload, grid, tokens, synced movement, chat and dice in two browsers | Signalling relay and TURN credentials; a bare call works in two browsers |
| 2026-11-29 | M2: pregenerated characters, sheet screen, clickable abilities, targeting, attack rolls | Docked video panel with mute, camera toggle and device picker |
| 2026-12-06 | M3: saves, damage, conditions, GM undo and override, Creature Cauldron import | Phone layout, reconnect handling |
| 2026-12-13 | M4: 3D dice, health states, deployment, demo rehearsal | Tested over the real deployment |

### Who does what

| Milestone | @Henry-theBinaryBuddies | @aalemany00 | @Danny-Defeato |
|---|---|---|---|
| M0 | Skeleton, contracts, CI, mock server | Machine setup, onboarding, first importer test against a sample file | Machine setup, onboarding, empty UI shell with dockable panels |
| M1 | Server sync, canvas with map, grid and tokens, signalling relay | Map and token upload pipeline, peer connection module | Chat and dice log panel, initiative bar |
| M2 | Rules engine: targeting, attack rolls, event log | Device handling and track management | Character sheet screen, video panel |
| M3 | Saves, damage, conditions, undo and override | Creature Cauldron import | GM override cards, health states, phone layout |
| M4 | 3D dice, deployment | Reconnect handling, demo data | Polish, demo rehearsal |

## Risk

M2 and M3 are where the rules engine, the sheet screen and the importers meet. The mock server exists so that `web/` and `integrations/` keep moving if the server side runs late.
