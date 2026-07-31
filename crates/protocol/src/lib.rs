//! Packet schemas, bit-level codec, quantization tables, golden tests.
//! Shared native/wasm. Zero game logic (DESIGN.md §4).
//!
//! M0 stub: the codec lands in its own pass. The version constant exists
//! from day one because the join handshake gates on it (DESIGN.md §5.9)
//! and golden fixtures will be keyed by it.

/// Wire protocol version. Bumps only with a packet-layout change and
/// regenerated goldens in the same commit (CLAUDE.md wall 6).
pub const PROTO_VER: u16 = 0;
