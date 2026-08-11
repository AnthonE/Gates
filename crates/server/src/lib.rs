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
//! handshakes to the accept loop).
//!
//! **Storage is one more thread and one more direction of rings.** The player
//! store (`store`) is sim → accept loop → storage thread: the sim takes a
//! record and knows no identities, the accept loop owns the key table and the
//! index, and one thread owns the file and does nothing else. So the sentence
//! above still holds exactly — the sim thread never touches a file — and it is
//! the reason a `sync_data` cannot land inside a tick. The world half
//! (`worldfile`) rides the same storage thread through its own double-buffered
//! ring pair; the WAL is still a later slice (DESIGN.md §8 says where).
//!
//! Hot-path law enforcement in this crate (DESIGN.md L1–L5): the sim-side
//! modules (`core`, `client`) use fixed-capacity storage only and allocate
//! exclusively at construction; `clippy.toml` next door bans locks,
//! channels, and nondeterministic maps crate-wide (net code included — it
//! never needed them).

pub mod auth;
pub mod boot;
pub mod botclient;
pub mod client;
pub mod config;
pub mod core;
pub mod entitle;
pub mod net;
pub mod slot;
pub mod stats;
pub mod status;
pub mod store;
pub mod worldfile;
/// The client-side snapshot view lives in `client-core` (the native client
/// and the bots share one implementation); re-exported for the gates.
pub use client_core::view;

/// This title's slug in scry's catalog — what its manifest, its depot, its
/// shard list and its ticket contract are all filed under.
///
/// The client has its own copy (`client::scry::SLUG`) and the two are not
/// shared, because the crates do not depend on each other in that direction
/// and never should. `crates/server/tests/entitlement.rs` pins them equal.
pub const ENTITLE_SLUG: &str = "gates";

pub use protocol::PROTO_VER;
pub use sim_core::limits;
