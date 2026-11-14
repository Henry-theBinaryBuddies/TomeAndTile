//! Authoritative game server for Tome & Tile (version 0).
//!
//! Split in two on purpose:
//!
//! * a pure core ([`campaign`], [`view`], [`auth`], [`ids`], [`rng`]) that
//!   owns state, permissions, dice and per-viewer filtering, with time,
//!   randomness and id minting passed in; and
//! * a thin socket layer ([`net`]) that only moves bytes and broadcasts.
//!
//! See docs/adr/0004-state-model.md and 0006-dice-and-visibility.md.

pub mod auth;
pub mod campaign;
pub mod ids;
pub mod media;
pub mod net;
pub mod rng;
pub mod view;

#[cfg(test)]
mod tests;

pub use net::{serve, Config};
