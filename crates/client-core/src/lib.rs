//! client-core — the client's netcode core (DESIGN.md §4/§5.6/§5.7):
//! `ClientCore`'s prediction, interpolation, own-fact rings and wire
//! decode. Pure, and **native** — the desktop client (`crates/client`)
//! links it as an ordinary rlib and compiles no wasm at all.
//!
//! It was called `client-wasm` and carried a 1,635-line raw C-ABI
//! `bridge.rs` beside this line, because the browser drove it through
//! that door. The browser client was cut (operator, 2026-08-06) and the
//! bridge outlived it by two months as 199 `extern "C"` exports nothing
//! in the tree called, under a name that told every reader this crate
//! was a web build. Both are gone (operator, 2026-08-08: *"we use
//! desktop build no more web"*). What the bridge's smoke gate actually
//! asserted — wire message in, core state out — is `tests/wire.rs` now,
//! in Rust, against the same `ClientCore` the desktop client runs.
//!
//! **wasm has not left the repo, and it is not a client.** `sim-core`
//! still builds for wasm32 so `test_parity_wasm` can diff its state
//! hashes against native byte for byte — that is wall 1's enforcement,
//! and a second target is the whole point of it.
//!
//! Dependency direction is law: client-core may use `sim-core` and
//! `protocol`, never the reverse. The `server` crate depends on this one
//! so its bot client and gates reconstruct snapshots through the exact
//! code the client ships — the load tool cannot drift from the client
//! it stands in for.

pub mod clock;
pub mod core;
pub mod interp;
pub mod predict;
pub mod view;

pub use protocol::PROTO_VER;
