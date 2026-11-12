# web/ — rules for Claude

Owner: @Danny-Defeato (except `web/media/`, owned by @aalemany00). This file is maintained by @Henry-theBinaryBuddies; do not edit it.

You are working for @Danny-Defeato. You may create and edit files under `web/` except `web/media/`, plus `docs/devlog/Danny-Defeato.md`. Nothing else.

## What lives here

The TypeScript UI shell: dockable panels, the character sheet, chat and dice log, initiative bar, video panel, GM tools, and the PWA and phone layouts. The layout is in [ADR 0011](../docs/adr/0011-client-ui.md).

## Rules

- **The UI holds no game rules.** It never computes a roll, a hit, damage, a save or a permission. It sends an `Intent` and renders the `Event`s that come back.
- **The UI never changes game state locally.** No optimistic HP changes, no local dice. Showing a "pending" indicator while waiting for the server is fine.
- **Unset means unknown.** The server leaves out what this viewer may not see (a monster's HP, for example). Render "unknown", never zero or a guess.
- **Never draw on the map canvas.** The canvas belongs to the renderer. Talk to it only through its published WASM interface. If the interface lacks something, open an issue for @Henry-theBinaryBuddies.
- **Use `web/media/` only through its exported interface** (join, leave, mute, list participants, attach a track to an element). Do not reach into its internals, import `livekit-client`, or call WebRTC APIs from panel code.
- Message types come from the generated contract code. Never hand-write a type for something that crosses the socket.
- Until the real server exists, develop against the mock server. Do not add fake logic to the UI to stand in for the server.
- Every panel must work at phone width (360 px) with touch, and with a keyboard.
- Treat all text from the server (names, chat, descriptions) as untrusted: render it as text, never as HTML.

## When you must stop and ask

- The task needs a file outside `web/`, or inside `web/media/`.
- The task needs a new or changed contract message: file a "Contract change request" issue.
- The task needs something new from the renderer's interface.
- The task needs a new dependency.

## Checks

Run from `web/` once the shell is scaffolded:

```bash
npm run check
```

```bash
npm test
```

```bash
npm run build
```
