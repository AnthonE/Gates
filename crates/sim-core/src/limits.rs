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

/// Item definitions the sim preallocates for (the alpha set is ~48 rows,
/// CONTENT.md §2). The content bake refuses a set past this. Proposed
/// default, DECISIONS.md §open (gather bounds row).
pub const MAX_ITEM_DEFS: usize = 64;

/// Inventory slots per player: 6 hotbar + 24 backpack (ALPHA.md §1).
pub const INV_SLOTS: usize = 30;

/// Hotbar width — the first `HOTBAR_SLOTS` inventory slots, the only ones
/// the held-item selector may name (ALPHA.md §1). The wire carries the
/// selector in 3 bits and refuses 6–7; a non-wire command with an invalid
/// selector falls back to slot 0 (`world::apply`).
pub const HOTBAR_SLOTS: usize = 6;

/// Recipe definitions the sim preallocates for (the alpha ladder is ~34
/// rows, CONTENT.md §2). The content bake refuses a set past this.
/// Proposed default, DECISIONS.md §open (craft verb row).
pub const MAX_RECIPES: usize = 64;

/// Inputs one recipe may carry (alpha data uses ≤ 3). The bake refuses
/// past it. Structural cap like `MAX_TOOLS_PER_NODE`, not a knob.
pub const MAX_RECIPE_INPUTS: usize = 4;

/// Per-player craft queue jobs (the reference crafting screen's queue
/// strip). Overflow policy: **refuse** — the enqueue bounces with an
/// integer reason code (EV_CRAFT_REFUSED). Proposed default, DECISIONS.md
/// §open (craft verb row).
pub const CRAFT_QUEUE: usize = 4;

/// Units one craft request may ask for (the quantity stepper's ceiling).
/// Overflow policy: **refuse** the request. Proposed default, DECISIONS.md
/// §open (craft verb row).
pub const CRAFT_COUNT_MAX: u16 = 99;

/// Per-connection inbound ring of decoded C→S action messages (net task →
/// sim; the reliable bidi lane). Overflow policy: **backpressure** — the
/// reader task stops reading the stream until the ring drains, so QUIC
/// flow control pushes the wait back to the sender; nothing on the
/// reliable lane is dropped. Proposed default, DECISIONS.md §open (craft
/// wire row).
pub const ACTION_RING_CAP: usize = 8;

/// Sparse slot-life store (harvested/damaged scatter slots). Sized past
/// the ~8–12 k live slots a seed produces (TERRAIN.md §6) so harvested
/// entries always fit. Overflow policy: **evict** — standing-damage
/// entries only, lowest hits first (the evicted node heals); harvested
/// entries are never evicted, and a store somehow full of them refuses
/// the hit. Proposed default, DECISIONS.md §open (gather bounds row).
pub const MAX_SLOT_LIVES: usize = 16_384;

/// Building-piece definitions the sim preallocates for (the alpha set is
/// 18 rows — 6 shapes × 3 materials, content/building.toml). The content
/// bake refuses a set past this. Structural cap like `MAX_ITEM_DEFS`.
pub const MAX_PIECE_DEFS: usize = 32;

/// Cost rows one building piece may carry (alpha data uses 1). The bake
/// refuses past it. Structural cap, not a knob.
pub const MAX_PIECE_COSTS: usize = 2;

/// Placed building pieces per shard. Overflow policy: **refuse** the
/// placement with an integer reason code (EV_BUILD_REFUSED) — a full world
/// refuses loudly, never evicts someone's wall. Proposed default,
/// DECISIONS.md §open (build grid row).
pub const MAX_PIECES: usize = 8_192;

/// Build-grid coordinate ceiling: cells index 0..MAX_BUILD_COORD on each
/// axis, matching the wire's 10-bit cell fields (the 2,048 m island spans
/// ~683 3 m cells; foundations further out die on the terrain rule first).
pub const MAX_BUILD_COORD: usize = 1_024;

/// Build levels (vertical storeys) per cell, 0-based. The wire carries the
/// level in 3 bits — exactly this range. Proposed default, DECISIONS.md
/// §open (build grid row).
pub const MAX_BUILD_LEVELS: usize = 8;

/// Column-index slots (collide.rs): open-addressed map from build column
/// to occupancy masks, power of two, 2 × MAX_PIECES so it never passes
/// half load (each piece occupies at most one column). Structural cap
/// derived from MAX_PIECES, not a knob.
pub const COL_INDEX_SLOTS: usize = 16_384;

/// Deployable definitions the sim preallocates for (the alpha set is 9
/// rows, content/deployables.toml). The content bake refuses a set past
/// this; the wire carries the row in 4 bits — exactly this range.
/// Structural cap like `MAX_PIECE_DEFS`.
pub const MAX_DEPLOY_DEFS: usize = 16;

/// Placed deployables per shard. Overflow policy: **refuse** the
/// placement with an integer reason code (EV_DEPLOY_REFUSED) — same
/// posture as `MAX_PIECES`. Proposed default, DECISIONS.md §open
/// (deployables row).
pub const MAX_DEPLOYS: usize = 1_024;

/// Hearths per shard, tracked in their own dense list so claim checks
/// and the upkeep sweep scan hearths, never the whole deploy store.
/// Overflow policy: **refuse** the hearth placement. Proposed default,
/// DECISIONS.md §open (deployables row).
pub const MAX_HEARTHS: usize = 256;

/// Stock rows per hearth — one per distinct upkeep material (the union
/// of building-cost items; the alpha build table uses 3). The bake
/// refuses a build table needing more. Structural cap, not a knob.
pub const HEARTH_STOCK_ROWS: usize = 4;

/// Piece + deployable records the upkeep/decay sweep visits per tick
/// (each store advances its own cursor by this many entries). Bounded
/// per-tick work: a full pass over both stores takes seconds while the
/// upkeep period is an hour. Proposed default, DECISIONS.md §open
/// (upkeep/decay row).
pub const UPKEEP_SWEEP_PER_TICK: usize = 64;

/// Per-connection outbound ring of reliable event-lane messages (sim →
/// net; the bidi stream). Overflow policy: **resync** — a refused push
/// flags the client for an event-lane resync (harvested-set walk restarts
/// with a reset batch, catalog restarts, the inventory shadow already
/// re-diffs), the same recovery path a fresh join uses. Proposed default,
/// DECISIONS.md §open (gather wire row).
pub const EVENT_RING_CAP: usize = 64;

/// Slot-life entries the per-client harvested-set walk scans per tick
/// (join sync / resync is drip-fed: at most one sync message per client
/// per tick, at most this many entries examined to fill it). Proposed
/// default, DECISIONS.md §open (gather wire row).
pub const SYNC_SCAN_PER_TICK: usize = 256;

/// Sim event ring, cleared every tick — the sim's only output channel
/// besides state itself (integer codes, CLAUDE.md wall 3). Overflow
/// policy: **drop newest**, counted; the late-join slot sync (the wire
/// slice) re-derives anything a lost event failed to announce. Proposed
/// default, DECISIONS.md §open (gather bounds row).
pub const MAX_EVENTS_PER_TICK: usize = 256;
