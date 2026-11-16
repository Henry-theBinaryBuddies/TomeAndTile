//! Map canvas renderer.
//!
//! Draws scenes (map, grid, tokens, templates; later walls, lighting, fog).
//! Takes events in and emits pointer-level intents out. `web/` may use this
//! crate only through its published WASM interface.
//! See docs/adr/0011-client-ui.md.
//!
//! Two halves: the pure scene model (`model`, `camera`, `draw`, `wire`),
//! tested natively, and the WebGPU backend plus `#[wasm_bindgen]` interface
//! (`gpu`, `web`), built only for `wasm32`. See README.md.

pub mod camera;
pub mod draw;
pub mod model;
pub mod wire;

#[cfg(target_arch = "wasm32")]
mod gpu;
#[cfg(target_arch = "wasm32")]
mod web;
