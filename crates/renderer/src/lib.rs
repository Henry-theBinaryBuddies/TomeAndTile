//! Map canvas renderer.
//!
//! Draws scenes (map, grid, tokens, templates; later walls, lighting, fog).
//! Takes events in and emits pointer-level intents out. `web/` may use this
//! crate only through its published WASM interface.
//! See docs/adr/0011-client-ui.md.
