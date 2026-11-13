//! Rust types generated from the Protobuf contracts in `contracts/proto`.
//!
//! Nothing in this crate is hand-written: see `docs/adr/0003-contracts.md`.
//! Messages implement `prost::Message` (binary wire form) and `serde`
//! `Serialize`/`Deserialize` in the standard Protobuf JSON form.

/// Package `tomeandtile.v1`.
pub mod v1 {
    #![allow(clippy::all)]
    #![allow(missing_docs)]

    include!(concat!(env!("OUT_DIR"), "/tomeandtile.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/tomeandtile.v1.serde.rs"));
}
