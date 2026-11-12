# web/media/ — rules for Claude

Owner: @aalemany00. This file is maintained by @Henry-theBinaryBuddies; do not edit it.

You are working for @aalemany00. This directory is the WebRTC client module: camera and microphone capture, peer connections and track management. The design is in [ADR 0007](../../docs/adr/0007-video-and-voice.md).

## Rules

- **Export one small interface and nothing else:** join, leave, mute and unmute, camera on and off, pick a device, list participants, attach a participant's track to an element, and subscribe to changes. The video panel in `web/` uses only this. Changing the interface needs a heads-up issue for @Danny-Defeato.
- **No UI here.** No buttons, layout or styles. Those belong to `web/`.
- **Signalling uses only the messages in `media.proto`**, sent over the game socket the app already has. Do not open a second connection or add a signalling server.
- **ICE servers come from the server's `MediaJoined` message.** Never hard-code a TURN URL, username or credential, and never put one in the repository.
- Ask for camera and microphone permission only when the user chooses to join the call, never on page load.
- Handle the failure paths: permission denied, no device, a peer that never answers, a dropped connection. Each must leave the module in a clean state.
- Nothing in this module may assume mesh. An SFU later must be a drop-in behind the same interface.

## When you must stop and ask

- The task needs a file outside `web/media/` or `integrations/`.
- The task needs a new or changed message in `media.proto`: file a "Contract change request" issue.
- The task needs a new dependency.

## Checks

Same commands as `web/CLAUDE.md`, run from `web/`.
