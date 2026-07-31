//! Thin wasm-bindgen bridge exposing sim-core prediction + chunk generation
//! to JS (DESIGN.md §4).
//!
//! M0 stub: the bindgen surface lands with the web client pass. Until then
//! this crate only pins the dependency direction: client-wasm may use
//! sim-core and protocol, never the reverse.

pub use protocol::PROTO_VER;
