# 0007: Video and voice

Status: Proposed (2026-11-12). Needs the project owner's confirmation, because it changes an earlier answer.

## Context

- The first design answer was "an SFU for video".
- The team's previous project (Couch Crew) shipped plain WebRTC with Socket.IO signalling and Cloudflare TURN. Jitsi Meet and Daily.co were tried and dropped.
- The server runs at home behind a Cloudflare Tunnel. A tunnel carries HTTP and WebSocket traffic, not the UDP media an SFU needs, so a self-hosted SFU would need a forwarded router port or a rented VPS.
- A D&D table is usually 4 to 7 people.

## Proposed decision

- **Now: peer-to-peer mesh WebRTC.** Signalling (offer, answer, ICE candidates) travels over the game server's existing WebSocket as messages in `media.proto`. Cloudflare TURN relays when a direct connection fails. The server mints short-lived TURN credentials for campaign members only.
- **Later: an SFU behind the same contract.** `media.proto` describes rooms, participants and tracks without assuming mesh, so swapping in an SFU (LiveKit, or Cloudflare's hosted SFU) changes `web/media/` and one server module, and nothing else.
- Mesh is capped at 8 participants with video. Beyond that, extra participants join audio-only until an SFU exists.

## Why

- It is what the team has already built and debugged once.
- No media passes through the home server, so the tunnel limitation disappears.
- It fits the class deadline; an SFU is a second system to deploy and operate.

## Costs

- Each participant uploads one stream per other participant. Quality drops past roughly 6 video streams on a home connection.
- No server-side recording.

## Ownership

| Piece | Where | Owner |
|---|---|---|
| Signalling relay, TURN credential minting, membership check | `crates/server` | @Henry-theBinaryBuddies |
| Peer connections, device handling, track management | `web/media/` | @aalemany00 |
| Video panel, mute and camera buttons, device picker | `web/` | @Danny-Defeato |

`web/media/` exposes a small TypeScript interface (join, leave, mute, list participants, attach a track to an element). The panel uses only that interface.
