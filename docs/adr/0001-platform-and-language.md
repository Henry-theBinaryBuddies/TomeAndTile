# 0001: Platform and language

Status: Accepted (2026-11-12)

## Decision

- The client runs in the browser. Players join with a link and install nothing.
- The map renderer is Rust compiled to WASM, drawing with WebGPU.
- The server and the rules engine are Rust (axum and tokio are the expected libraries).
- The UI around the map is TypeScript. SvelteKit is the default framework; the `web/` owner may propose another before the shell is built.
- Mobile and tablet are supported by one responsive PWA with touch-first controls.

## Why

- The goal is a fast VTT written in a low-level language. In a VTT, felt speed comes from rendering and the network, so the renderer is where Rust pays off most.
- Rust is memory-safe without a garbage collector, compiles cleanly to WASM, and its compiler catches a class of mistakes that matters with three contributors.
- A browser client costs players nothing to join, which a native app cannot match.

## Rejected

- **Native desktop client:** install friction, and WebRTC is harder to embed.
- **C++:** manual memory management is a bigger risk across three people.
- **Zig:** too few server and networking libraries today.
- **TypeScript-only client:** leaves the low-level goal on the server only.
- **Separate phone UI:** double the UI work.
