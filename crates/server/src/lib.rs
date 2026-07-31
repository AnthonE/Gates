//! tokio + wtransport termination, session/auth, AOI + snapshot encoding,
//! WAL persistence, admin, bots (DESIGN.md §4).
//!
//! M0 stub: the shard binary, rings, and the pinned wtransport dependency
//! (git-pin ≥ `0f7609a`, NETCODE.md §2.2) land in the server pass. Nothing
//! here yet may touch the sim except through `sim_core`'s public API.

pub use protocol::PROTO_VER;
pub use sim_core::limits;
