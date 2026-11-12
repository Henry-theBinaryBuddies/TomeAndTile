# Architecture

This document describes the shape of Tome & Tile. The reasons behind each choice are in [docs/adr/](docs/adr/).

## One-paragraph summary

A Rust server owns all game state and every dice roll. Browsers connect over one WebSocket, send **intents** ("use this ability on that token") and receive **events** ("the goblin took 7 damage"), filtered so each viewer gets only what they may see. The map is drawn by a Rust renderer compiled to WASM on WebGPU; everything around the map is a TypeScript UI shell. Every message shape is defined once, in Protobuf, in `contracts/`.

## Components

```
                 contracts/  (Protobuf, generated Rust + TypeScript types)
                      ^
     +----------------+-----------------+------------------+
     |                |                 |                  |
crates/server    crates/renderer      web/           integrations/
  |      |        (WASM canvas)    (UI shell,      (importers, assets)
  |      |                          web/media)
crates/rules  crates/plugin-host
```

| Component | Job | Depends on |
|---|---|---|
| `contracts/` | Message and file-format definitions. Versioned; breaking changes are rejected by CI. | nothing |
| `crates/rules` | Pure 5e rules engine: takes state plus an intent, returns events. No I/O, no clock, no network. Dice come from an injected roller. | contracts |
| `crates/server` | WebSocket sessions, accounts, permissions, the event log, per-viewer filtering, persistence, video join tokens. | contracts, rules, plugin-host |
| `crates/renderer` | Draws scenes: map, grid, tokens, templates, later walls, lighting and fog. Compiled to WASM. Takes events in, emits pointer-level intents out. | contracts |
| `crates/plugin-host` | Runs third-party WASM plugins in a sandbox with an explicit capability list. | contracts |
| `integrations/` | Pure functions from a foreign file (Creature Cauldron export, Universal VTT map, image) to contract messages. Plus the asset upload pipeline. | contracts only |
| `web/` | Dockable panels, character sheet, chat and dice log, initiative bar, GM tools, PWA and touch layouts. Hosts the renderer canvas. | contracts, the renderer's published WASM interface |
| `web/media/` | Camera, microphone and the SFU connection, wrapping LiveKit's client library. Uses the server only to fetch a join token (`media.proto`). | contracts |

## The dependency rule

Everything depends on `contracts/`. Nothing in `integrations/` or `web/` may import from `crates/server`, `crates/rules`, `crates/renderer` internals or `crates/plugin-host`. `tools/check-deps.sh` enforces this in CI.

If two components need to share something, it goes into `contracts/` through a reviewed change. It never goes in through a direct import.

## State and events

- **Server-authoritative.** Clients never change game state. They send an `Intent`; the server validates it, runs the rules engine and appends `Event`s to the campaign's log.
- **Event-sourced.** Current state is the fold of the event log. GM undo, reconnect after a dropped connection and session replay all read the same log.
- **Scene-addressed.** Every token, wall and effect carries a `scene_id`. A campaign has many scenes and players may be on different ones.
- **Per-viewer filtering.** Before an event is sent, the server strips what the viewer may not see. Monster stat blocks, hidden GM rolls and GM notes never reach a player's browser. Players see health states ("Bloodied") for creatures they do not control unless the GM reveals more.
- **Live state in memory, written behind to PostgreSQL.** Assets live in S3-compatible object storage behind a CDN; the database holds references.

## Rules engine

- **Data-driven.** Abilities are data: target type, range, attack roll or save, damage, half-on-save, conditions and duration. SRD 5.1 and 5.2 content ships built in; everything else is imported.
- **Resolution is a pausable state machine.** `Targeting -> AttackOrSave -> Damage -> Effects -> Done`, with an `AwaitingReaction` state reserved for Shield, Counterspell and opportunity attacks.
- **Fully automatic, with GM undo and override.** Click ability, click target; the server rolls and applies. Every step is a roll card in chat.
- **Track and warn, never block.** Turn order, action use, slots, movement and range are tracked. Violations attach a warning to the event; the action still happens.
- **Initiative** is rolled by the server when the GM starts combat. The GM may edit any result or the order.
- **Death saves are not automated.** At 0 HP the engine marks the creature as down and stops. Players roll death saves by hand; the GM decides what happens to monsters.
- **Control handover.** The GM can assign any token to a player.
- **Dice.** The server rolls. Player rolls are always public. Only the GM may hide a roll. The client plays a 3D animation that lands on the server's result.

## Video and voice

An SFU (LiveKit) carries the media: LiveKit Cloud for the class, self-hostable later. The game server only mints short-lived join tokens for campaign members; browsers then talk to the SFU directly. See [ADR 0007](docs/adr/0007-video-and-voice.md).

## Extensions

Third-party code runs as sandboxed WASM with an explicit capability list (wasmtime on the server, a Worker in the browser). Abilities the declarative data cannot express are plugins.

## Deployment

Docker Compose on a home Ubuntu server behind a Cloudflare Tunnel, static front end on Cloudflare Pages. Media does not travel through the tunnel; it goes between browsers and the SFU.

## Scope for the class final (2026-12-13)

In: shared map, tokens, synced movement, chat, dice (3D), character sheets with click-to-attack, automatic saves and damage, GM undo, Creature Cauldron import, video and voice, phone layout.

Designed into the contracts but built later: conditions and timed effects beyond the basics, reactions, multiple simultaneous scenes, fog and dynamic lighting, multi-level maps, Universal VTT and animated maps, plugins, journals, macros, audio, OAuth.
