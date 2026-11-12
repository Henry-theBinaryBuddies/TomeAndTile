# web/media/ — rules for Claude

Owner: @aalemany00. This file is maintained by @Henry-theBinaryBuddies; do not edit it.

You are working for @aalemany00. This directory is the video and voice client module: a wrapper around LiveKit's client library that handles connecting, devices and tracks. The design is in [ADR 0007](../../docs/adr/0007-video-and-voice.md).

## Rules

- **Export one small interface and nothing else:** join, leave, mute and unmute, camera on and off, pick a device, list participants, attach a participant's track to an element, and subscribe to changes. The video panel in `web/` uses only this. Changing the interface needs a heads-up issue for @Danny-Defeato.
- **`livekit-client` is imported here and nowhere else.** No LiveKit type may appear in the exported interface, so the SFU can be swapped without touching the panel.
- **No UI here.** No buttons, layout or styles. Those belong to `web/`.
- **The SFU URL and join token come from the server's `MediaToken` message**, requested with `MediaTokenRequest` over the game socket the app already has. Never hard-code a URL, key, secret or token, and never put one in the repository.
- **Treat the token as a password:** keep it in memory only. Never log it, store it or put it in a URL.
- When the token expires or the connection drops, ask the server for a new token and rejoin.
- Honour `can_publish`: spectators never open the camera or microphone.
- Ask for camera and microphone permission only when the user chooses to join the call, never on page load.
- Handle the failure paths: permission denied, no device, token refused, a dropped connection. Each must leave the module in a clean state.

## When you must stop and ask

- The task needs a file outside `web/media/` or `integrations/`.
- The task needs a new or changed message in `media.proto`: file a "Contract change request" issue.
- The task needs a dependency other than `livekit-client`.

## Checks

Same commands as `web/CLAUDE.md`, run from `web/`.
