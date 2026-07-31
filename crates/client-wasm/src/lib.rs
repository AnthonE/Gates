//! client-wasm — the client's netcode core (DESIGN.md §4/§5.6/§5.7),
//! pure and native-tested, plus the raw C-ABI bridge the browser drives
//! (`bridge.rs`, wasm32 only — no bindgen, the parity-probe pattern).
//!
//! Dependency direction is law: client-wasm may use `sim-core` and
//! `protocol`, never the reverse. The `server` crate depends on this one
//! so its bot client and gates reconstruct snapshots through the exact
//! code the browser ships — the load tool cannot drift from the client
//! it stands in for.

pub mod clock;
pub mod core;
pub mod interp;
pub mod predict;
pub mod view;

#[cfg(target_arch = "wasm32")]
mod bridge;

pub use protocol::PROTO_VER;
