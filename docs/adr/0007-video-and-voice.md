# 0007: Video and voice

Status: Accepted (2026-11-12)

## Context

- The team's previous project (Couch Crew) used peer-to-peer WebRTC with Cloudflare TURN. TURN did not work well there, and Jitsi Meet and Daily.co were tried and dropped.
- Peer-to-peer calls fail on strict networks (campus Wi-Fi, some mobile carriers) unless TURN works, so a mesh design would rest on the part that failed last time.
- The game server runs at home behind a Cloudflare Tunnel, which carries HTTP and WebSocket traffic but not the UDP media an SFU needs.

## Decision

- Video and voice go through an SFU: **LiveKit**.
- **For the class:** LiveKit Cloud, so nothing media-related is hosted at home and the tunnel limitation does not apply.
- **Later:** self-hosted LiveKit on a small VPS if wanted. It is the same open-source server and the same client library; only the URL and keys change.
- One LiveKit room per campaign.
- The game server mints a short-lived LiveKit join token, and only for members of the campaign. Spectators get a subscribe-only token. The LiveKit API key and secret live only in the server's environment.
- The browser asks for a token over the game socket (`MediaTokenRequest`, `MediaToken` in `media.proto`) and then talks to LiveKit directly. The game server relays no media and no signalling.
- No TURN server is configured by us. LiveKit falls back to TCP on port 443 on its own.

## Why

- Every browser makes one outbound connection to a server with a public address, which is the case that works on almost every network.
- Upload cost per participant stays flat as the table grows.
- LiveKit's client library replaces hand-written peer connection code, which suits a contributor new to WebRTC.

## Costs

- A hosted dependency during the class. LiveKit Cloud's free tier has usage limits; check them before the demo.
- One new client dependency in `web/` (`livekit-client`), approved with this ADR.
- One new server dependency for signing tokens, to be chosen when the endpoint is built.

## Ownership

| Piece | Where | Owner |
|---|---|---|
| Join-token minting, membership and role check | `crates/server` | @Henry-theBinaryBuddies |
| LiveKit client wrapper: connect, devices, tracks | `web/media/` | @aalemany00 |
| Video panel, mute and camera buttons, device picker | `web/` | @Danny-Defeato |

`web/media/` exposes a small TypeScript interface (join, leave, mute, camera, pick a device, list participants, attach a track to an element, subscribe to changes). The panel uses only that interface and never imports `livekit-client` itself, so the SFU can be replaced without touching the panel.

## Rejected

- **Peer-to-peer mesh with Cloudflare TURN:** depends on TURN, which was the weak point last time; quality drops past about six video streams.
- **Self-hosted LiveKit at home now:** needs a forwarded UDP port or a VPS before the class deadline.
- **Daily.co, Jitsi Meet:** already tried and dropped by this team.
