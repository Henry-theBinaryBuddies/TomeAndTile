# 0010: Integration with the other artifacts

Status: Accepted (2026-11-12)

## Decision

- Creature Cauldron, Chronomancer, Effigy Engine, Scenic Sorcerer and Adventurer's Anvil stay independent applications. The VTT is coupled to them only through file formats and, where one exists, a REST API.
- Each artifact gets one importer under `integrations/`. An importer is a pure function: foreign file in, contract messages out. It never talks to the live game and is tested against sample files checked into its own directory.
- Where an artifact has no export, the artifact is changed to add one. The VTT does not reach into another artifact's internals.

| Artifact | Brings | State today |
|---|---|---|
| Creature Cauldron | Monsters and NPC stat blocks | Has an export. First importer, due 2026-12-06. |
| Adventurer's Anvil | Player characters | Not finished. It will export the `Character` contract when ready. |
| Scenic Sorcerer | Maps with walls, floors and stairs | Export format to be defined. |
| Effigy Engine | 3D busts for tokens or a viewer | After the class. |
| Chronomancer | To be decided | After the class. |

- **Characters before Adventurer's Anvil is ready:** the `Character` contract is fixed in week 1 and the demo uses hand-authored SRD pregenerated characters in that format. Characters are imported and then edited in the VTT; there is no built-in character builder.
- **Map formats in the design:** plain images with manual grid alignment (first), Scenic Sorcerer exports, Universal VTT (`.dd2vtt`, `.uvtt`), and animated maps (WebM, MP4).

## Why

- Loose coupling means a change in one artifact cannot break the VTT build, and a beginner can own an importer without touching the game loop.

## Rejected

- **Live two-way sync:** needs shared accounts and conflict handling across five projects.
- **Porting the artifacts into the VTT:** a rewrite of each.
- **A built-in character builder:** duplicates Adventurer's Anvil.
