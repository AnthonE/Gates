//! The one place bounds live (DESIGN.md L4, CLAUDE.md wall 4). Every queue,
//! map, and per-tick work item caps here, each with its overflow policy
//! stated. Server and protocol import these; nothing redefines them.

/// Shard cap (knob, DECISIONS.md §open: 100 / 4-core VPS). Preallocation
/// needs a number at boot; the accept path hard-refuses past it.
pub const MAX_PLAYERS: usize = 100;

/// Commands the sim will apply in one tick. Overflow policy: **defer** —
/// callers keep the tail for the next tick; the sim itself takes at most
/// this many (`World::tick` ignores the excess rather than growing work).
pub const MAX_COMMANDS_PER_TICK: usize = 256;

/// Fixed sim rate (DESIGN.md §5.2). The tick number is the only clock.
pub const TICK_HZ: u32 = 30;

/// Datagram payload budget (DESIGN.md §5.3): safe under QUIC's ~1200 B
/// initial MTU, never fragmented. Overflow policy: **shed** — the snapshot
/// encoder refuses the entity/removal that won't fit and the priority
/// accumulator re-offers it next snapshot (DESIGN.md §5.5).
pub const DATAGRAM_BUDGET_BYTES: usize = 1100;

/// Unacked input frames one datagram may carry (NETCODE.md §3: ≈333 ms of
/// loss cover, Rocket League's shipped number). Overflow policy: **drop
/// oldest** — the client keeps only the newest `MAX_INPUT_FRAMES` unacked.
pub const MAX_INPUT_FRAMES: usize = 10;

/// Class-D entities in one client's interest set, hard cap (NETCODE.md §9
/// budgets table: typical ~15 / cap 64). Overflow policy: **defer** — the
/// priority accumulator keeps accruing what didn't fit (DESIGN.md §5.5).
pub const MAX_SNAPSHOT_ENTITIES: usize = 64;

/// `state_hash` cadence in ticks (DESIGN.md §7).
pub const STATE_HASH_INTERVAL: u64 = 32;
