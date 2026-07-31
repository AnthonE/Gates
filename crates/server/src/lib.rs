//! The shard server (DESIGN.md §4): wtransport termination, session hello,
//! bounded SPSC rings, the pinned 30 Hz sim thread, AOI v0, priority-filled
//! baseline+delta snapshots, and the bot load client.
//!
//! Thread model is the DESIGN.md §4 picture verbatim: tokio net tasks own
//! sockets and streams; one std sim thread owns the world and every
//! per-client netcode state; one accept loop owns connection lifecycle.
//! Traffic that touches the sim thread rides only bounded lock-free
//! rings and per-slot atomics — it never touches a socket, a file, a
//! lock, or (outside the tick boundary) the clock. Net-side tasks talk
//! tokio-to-tokio through tokio plumbing (a bounded mpsc feeds finished
//! handshakes to the accept loop). Storage/WAL is a later slice.
//!
//! Hot-path law enforcement in this crate (DESIGN.md L1–L5): the sim-side
//! modules (`core`, `client`) use fixed-capacity storage only and allocate
//! exclusively at construction; `clippy.toml` next door bans locks,
//! channels, and nondeterministic maps crate-wide (net code included — it
//! never needed them).

pub mod botclient;
pub mod client;
pub mod config;
pub mod core;
pub mod net;
pub mod slot;
pub mod stats;
/// The client-side snapshot view lives in `client-wasm` (the browser and
/// the bots share one implementation); re-exported for the gates.
pub use client_wasm::view;

pub use protocol::PROTO_VER;
pub use sim_core::limits;
