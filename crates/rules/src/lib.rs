//! Pure D&D 5e rules engine.
//!
//! Takes game state and an intent, returns events. No I/O, no clock, no
//! network; dice come from an injected roller so tests are deterministic.
//! See docs/adr/0005-rules-engine.md.

pub mod dice;
