//! Pure D&D 5e rules engine.
//!
//! Takes game state and an intent, returns events. No I/O, no clock, no
//! network; dice come from an injected roller so tests are deterministic.
//! See docs/adr/0005-rules-engine.md.
//!
//! Hit points are applied by [`damage`] (resistances, temporary HP, healing,
//! health state). No death-save, instant-death or massive-damage rule runs at
//! 0 HP: reaching 0 sets Down and nothing else, and death saves are rolled by
//! hand (ADR 0005).

pub mod ability;
pub mod attack;
pub mod damage;
pub mod dice;
pub mod distance;
