//! The shard server (DESIGN.md §4): wtransport termination, session hello,
//! bounded SPSC rings, the pinned 30 Hz sim thread, AOI v0, priority-filled
//! baseline+delta snapshots, and the bot load client.
//!
//! Thread model is the DESIGN.md §4 picture verbatim: tokio net tasks own
//! sockets and streams; one std sim thread owns the world and every
//! per-client netcode state; one accept loop owns connection lifecycle.
//! The three talk only through bounded lock-free rings and per-slot
//! atomics — the sim thread never touches a socket, a file, a lock, or
//! (outside the tick boundary) the clock. Storage/WAL is a later slice.
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
pub mod view;

pub use protocol::PROTO_VER;
pub use sim_core::limits;
