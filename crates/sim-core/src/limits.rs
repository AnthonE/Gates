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

/// Snapshot cadence: one snapshot every this many sim ticks — 15 Hz at the
/// 30 Hz tick (DESIGN.md §5.3).
pub const SNAPSHOT_INTERVAL_TICKS: u64 = 2;

/// AOI v0 radii in centimeters (DESIGN.md §5.5): subscribe entering at
/// 176 m, unsubscribe leaving at 208 m — hysteresis so edge-dancers don't
/// flap. Planar (x/z) in M0; the island's ≤ 60 m of relief is well inside
/// the band. Centimeters so distance² compares stay in exact i64.
pub const AOI_ENTER_CM: i64 = 17_600;
pub const AOI_EXIT_CM: i64 = 20_800;

/// Per-client ring of sent snapshots the server deltas against
/// (NETCODE.md §3: "the last 32 sent states"). An ack that falls outside
/// it drops the baseline to the canonical zero-state — recovery is the
/// same code path, not a special one.
pub const SENT_SNAPSHOT_RING: usize = 32;

/// Staleness ceiling (NETCODE.md §3): an interest-set player may never
/// exceed this many unsent snapshots — at the ceiling it preempts the
/// priority order.
pub const STALENESS_CEILING: u8 = 4;

/// Per-connection inbound ring of decoded input datagrams (net task →
/// sim). Overflow policy: **drop newest** — the ring only fills when the
/// sim stalls, and every later datagram re-carries the unacked tail
/// (NETCODE.md §3), so a dropped push costs nothing that the next push
/// doesn't restore. Proposed default, DECISIONS.md §open.
pub const INPUT_RING_CAP: usize = 32;

/// Per-connection outbound ring of encoded snapshots (sim → net).
/// Overflow policy: **skip** — a client not draining skips a snapshot
/// (DESIGN.md §4); the next one supersedes it anyway.
pub const SNAPSHOT_RING_CAP: usize = 4;

/// Connection-lifecycle control rings (accept loop → sim, and the
/// graveyard back). Overflow policy: **refuse** the join / retry the
/// return next tick. Proposed defaults, DECISIONS.md §open.
pub const CTRL_RING_CAP: usize = 8;
pub const GRAVEYARD_RING_CAP: usize = 256;

/// Server-side per-client input frame buffer, keyed by seq (NETCODE.md
/// §4: target depth 1–2 ticks). Overflow policy: **drop oldest** — a
/// too-deep buffer skips ahead via the consume throttle. Proposed
/// default, DECISIONS.md §open.
pub const INPUT_BUFFER_CAP: usize = 16;

/// Consume throttle (NETCODE.md §4, the Rocket League fallback): buffer
/// depth above this consumes two inputs in one tick to re-center.
pub const INPUT_THROTTLE_DEPTH: usize = 6;

/// Per-client pending-removal set: entity ids that left the client's
/// interest, re-sent in every snapshot until a snapshot carrying them is
/// acked (so a ghost can't survive datagram loss). Overflow policy:
/// **resync** — the client falls to the zero-state baseline, which is the
/// recovery path for everything else too. Proposed default, DECISIONS.md
/// §open.
pub const PENDING_REMOVALS_CAP: usize = 256;
