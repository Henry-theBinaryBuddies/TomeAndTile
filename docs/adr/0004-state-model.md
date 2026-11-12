# 0004: Server-authoritative, event-sourced state

Status: Accepted (2026-11-12)

## Decision

- The server owns game state. Clients send `Intent` messages and render the `Event` messages they receive. A client never applies a rule or a roll itself.
- State is the fold of an append-only event log per campaign. Each event has a sequence number.
  - **Undo:** the GM undoes by appending a compensating event that references the original. History is never deleted.
  - **Reconnect:** a client sends its last sequence number and receives the events it missed, or a snapshot if it is too far behind.
  - **Replay:** a session can be replayed from the log.
- Every token, wall, light and effect carries a `scene_id`. A campaign has many scenes; players can be on different scenes at once.
- The server filters every outgoing event per viewer (see ADR 0006).
- Live state is held in memory and written behind to PostgreSQL.

## Why

- Automatic saves and damage cannot be trusted if a client computes them.
- The GM needs undo. Building undo on an event log also yields reconnect and replay.
- Scene addressing and the event envelope are the hardest things to add later without breaking the contracts, so they are in from the start.

## Rejected

- **Peer-to-peer CRDT state:** cannot keep GM secrets or enforce rules.
- **Mutable state with ad hoc undo:** each undoable action would need its own reversal code.
