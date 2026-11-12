# 0011: Client UI, map and token features

Status: Accepted (2026-11-12)

## Decision

### Layout
- The map fills the window. The character sheet, chat and dice log, initiative bar, video and GM tools are panels that dock to an edge, collapse or float. On a phone they become bottom tabs over the map.

```
+----------------------------------+------+
| [init: Aria > Goblin > Bram ]    | CHAT |
|                                  | roll |
|            M A P                 | 17+5 |
|        (o)   (x)                 |      |
|                                  |------|
| [sheet tab] [tools]              | VIDEO|
+----------------------------------+------+
```

### Division between `web/` and the renderer
- The renderer owns the canvas and everything drawn on the map.
- `web/` owns everything outside the canvas and never draws on it.
- `web/` sends intents and renders events. It holds no game rules and computes no rolls.

### Map features in the design
- Square and hex grids, distance rulers and area templates (cone, sphere, line, cube).
- Fog of war, dynamic lighting, walls and doors.
- Multiple levels, stairs and elevation.
- 3D busts from Effigy Engine are out of the launch design.

### Token features in the design
- HP bar, condition icons and name plate, filtered per viewer.
- Sizes from Tiny to Gargantuan, facing and auras the rules engine can read.
- Movement path with distance and waypoints, warning past the creature's speed.
- Elevation and flying, used in range calculations.

### GM tools, in build order
1. Initiative tracker
2. Journals and handouts
3. Macros
4. Audio

## Why

- Dockable panels give the map the most space and adapt to phones with one layout system.
- Keeping `web/` out of the canvas is what lets two people work on the client without colliding.

## Rejected

- **Fixed three columns:** small map, poor on phones.
- **Different layouts for players and GM:** two layouts to maintain.
