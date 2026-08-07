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

/// Each hop of the player-save path: sim → the store's index (accept loop),
/// and the index → the file (storage thread). One cap for both because both
/// carry the same traffic — at most one autosave record per tick plus a
/// leave's, drained every sweep.
///
/// Sized against the burst, which is a mass disconnect: `MAX_PLAYERS` leaves
/// can land in one tick, so the cap sits above that with room for the sweep's
/// steady drip. Overflow policy: **drop newest**, counted
/// (`save_ring_drops`). That costs *freshness*, never correctness — the
/// autosave sweep re-takes the same player a few ticks later, and the record
/// is idempotent (it is filed by key and overwrites in place). Proposed
/// default, DECISIONS.md §open ("player persistence v0").
pub const SAVE_RING_CAP: usize = 256;

/// World-save buffers in flight, each direction. **Two, because that is
/// what a double buffer is**: one being written to disk while the next is
/// being filled. Not a queue depth to tune — a deeper one would only let
/// stale worlds pile up behind a slow disk, and the right answer to a slow
/// disk is to skip a save rather than to write an old world late.
///
/// Overflow policy: **skip the save, counted** (`world_saves_skipped`). The
/// cadence comes around again with a fresher world, so a skipped save costs
/// nothing a later one does not replace — which is the opposite of the
/// player path, where a dropped record is somebody's session.
///
/// Proposed default, DECISIONS.md §open ("world persistence v0").
pub const WORLD_RING_CAP: usize = 2;

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

/// Per-connection inbound ring of decoded C→S chat lines. Overflow
/// policy: **drop newest**, counted — the opposite of the action ring,
/// and deliberately. An action is a transaction the client is owed; a
/// chat line is not, and backpressuring the shared stream on chat would
/// let one spammer stall their own build and craft lane behind their own
/// typing. The reader rate-limits ahead of this, so a full ring means a
/// burst that the limiter already judged legal — dropping its tail is
/// honest. Proposed default, DECISIONS.md §open (chat v0).
pub const CHAT_RING_CAP: usize = 4;

/// Local-chat radius in centimeters: 20 m planar (DECISIONS.md §open,
/// "local chat | on, 20 m"). Centimeters so distance² compares stay in
/// exact i64, same as the AOI radii above — and well inside the AOI
/// enter radius, so anyone who can hear you is already an entity you can
/// see.
pub const CHAT_LOCAL_CM: i64 = 2_000;

/// Sparse slot-life store (harvested/damaged scatter slots). Sized past
/// the ~8–12 k live slots a seed produces (TERRAIN.md §6) so harvested
/// entries always fit. Overflow policy: **evict** — standing-damage
/// entries only, lowest hits first (the evicted node heals); harvested
/// entries are never evicted, and a store somehow full of them refuses
/// the hit. Proposed default, DECISIONS.md §open (gather bounds row).
pub const MAX_SLOT_LIVES: usize = 16_384;

/// Lines in the direct-mapped memo of `terrain::scatter` that makes the
/// occupant collision query affordable (occupy.rs). Sized past the
/// `MAX_PLAYERS * 9` cells a full shard can have under probe at once, so a
/// spread-out server still mostly hits.
///
/// **It has no overflow policy, and that is not an omission.** Every other
/// cap here bounds a store whose contents are state, where dropping an entry
/// loses something; this bounds a memo of a pure function, where a collision
/// re-resolves and the answer is bit-identical either way. Nothing can be
/// lost, so there is nothing to refuse or evict. Must stay a power of two —
/// occupy.rs masks with it and a const block asserts it.
/// Proposed default, DECISIONS.md §open (occupant collision v0 row).
pub const SLOT_CACHE_SLOTS: usize = 1_024;

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

/// Cost rows one deployable may carry, for pricing its repair. A
/// deployable's *placement* costs one crafted item; its repair is priced
/// against the recipe that made that item, so this is the recipe's input
/// width and the bake refuses past it. Structural cap like
/// `MAX_PIECE_COSTS`, not a knob.
pub const MAX_DEPLOY_COSTS: usize = MAX_RECIPE_INPUTS;

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

/// Death backpacks standing at once (backpack.rs). Sized past the worst
/// honest case: `MAX_PLAYERS` bodies each holding a bag, plus headroom
/// for the churn a long fight leaves behind while the timers run down.
/// Overflow policy: **evict** the bag nearest its own despawn —
/// NETCODE.md §6.4's "overflow despawns oldest-lowest-tier first",
/// collapsed into the one key that already encodes both, and counted by
/// the `BAG_GONE_EVICTED` removal event. Never **refuse**: a death that
/// silently kept the inventory would make the cap a way to dodge the
/// loss. Proposed default, DECISIONS.md §open (death backpack v0).
pub const MAX_BACKPACKS: usize = 256;

/// Deployed storage boxes standing at once (`deploy.rs`, `ARCH_BOX`).
/// Held in its own dense list beside the hearths, for the same reason:
/// `DeployRec` is the struct the wire mirrors, so contents may not ride
/// on it. Overflow policy: **refuse** the placement with
/// `REFUSE_D_FULL` — the same posture `MAX_HEARTHS` takes, and correct
/// here for the same reason: nothing is lost by refusing a box that was
/// never placed. Proposed default, DECISIONS.md §open (box v0).
pub const MAX_BOXES: usize = 256;

/// Slots inside one deployed box. Deliberately under `INV_SLOTS`: a box
/// is a place to put things down, not a second inventory, and the whole
/// store is sized against this (`MAX_BOXES * BOX_SLOTS` stacks). Both
/// box rows in `content/deployables.toml` share it — a per-row slot
/// count is a content field and an open question, not a constant.
/// Proposed default, DECISIONS.md §open (box v0).
pub const BOX_SLOTS: usize = 12;

/// Broken-box spills awaiting a ground bag, drained at the end of the
/// tick that broke them (`world.rs`). A box is emptied where it stood by
/// the same `stand_up` a corpse and a barrel use, but the removal path
/// (`deploy::drop_deploy`, reached from decay and from a raid) holds
/// neither the bag store nor the clock, so the contents wait one step of
/// the same tick in this buffer. Overflow policy: **refuse the removal's
/// spill** — a full buffer drops that box's contents rather than growing
/// unbounded work (wall 4). Sized past the worst honest case: an upkeep
/// sweep visits `UPKEEP_SWEEP_PER_TICK` deploys and a piece cascade can
/// take several boxes with one wall, so this is that cascade's headroom,
/// not the sweep's. Proposed default, DECISIONS.md §open (box v0).
pub const MAX_BOX_SPILL_PER_TICK: usize = 16;

/// Loot tables the sim preallocates for — one per container archetype
/// (`content/loot.toml` ships 2: barrel and crate). The content bake
/// refuses a set past this. Structural cap like `MAX_DEPLOY_DEFS`, not a
/// knob: the index is code (`loot::LOOT_*`), so a table the sim has no
/// verb for is a bake error rather than a silent extra row.
pub const MAX_LOOT_TABLES: usize = 8;

/// Weighted rows one loot table may carry (the shipped crate table uses
/// 9). The bake refuses past it. Structural cap like `MAX_RECIPE_INPUTS`,
/// not a knob.
pub const MAX_LOOT_ENTRIES: usize = 16;

/// Draws one smash may make — the cap on a table's `rolls_max`. The bake
/// refuses past it. Structural cap like the two above, **not a knob**: it
/// is `INV_SLOTS` because `LootContent::roll_into` fills exactly that many
/// slots, so a draw past the last one can only deepen a stack that is
/// already standing, and how deep a row pays is what its `count_min` /
/// `count_max` are for. Balance lives in `content/loot.toml`; this is the
/// ceiling on the work, not on the reward.
///
/// It is wall 4's cap on this path, and until this constant existed there
/// was none: `rolls_max` is a `u32` in the TOML narrowed to `u16` by the
/// bake, so `65_535` validated, and one smash then walked the 16-row
/// weight table that many times **inside a tick**. Every other wall stays
/// green through it — the arithmetic is integer, the store is fixed, the
/// allocator is untouched — which is precisely why the bound has to be
/// stated rather than inferred from a passing suite.
pub const MAX_LOOT_ROLLS: usize = INV_SLOTS;

/// Bearings the haven-pad argmax may score (TERRAIN.md §1 stage 8's
/// "something derived once from the seed then queried", which the stage
/// 7–9 constraint block requires to carry a cap here). Overflow policy:
/// **refuse** — the search scores at most this many candidate sites and
/// never grows the set; a seed where none is accepted falls back to the
/// best land site, then the island center, both asserted unreachable by
/// `tests/haven.rs`. Bounded work at world init, never in a tick.
/// Proposed default, DECISIONS.md §open (haven pad v0).
pub const MAX_HAVEN_CANDIDATES: usize = 64;

/// Sim event ring, cleared every tick — the sim's only output channel
/// besides state itself (integer codes, CLAUDE.md wall 3). Overflow
/// policy: **drop newest**, counted; the late-join slot sync (the wire
/// slice) re-derives anything a lost event failed to announce. Proposed
/// default, DECISIONS.md §open (gather bounds row).
pub const MAX_EVENTS_PER_TICK: usize = 256;

/// Pieces one structural collapse may drop in a single tick (build.rs
/// `collapse_from`: take a wall's legs out and what rested on it falls).
/// Sized off `MAX_EVENTS_PER_TICK`, not off the piece store: every
/// removal spends one event slot — two when a deployable stood on it —
/// and `EV_PIECE_REMOVED` is the only thing that tells a client the piece
/// is gone, so a cascade allowed to fill the ring would leave the rest of
/// the collapse drawn on every screen forever. A quarter of the ring
/// keeps the tick's other events inside their own budget.
/// Overflow policy: **defer** — the cascade stops at the cap and whatever
/// is left standing on nothing is dropped by `support_sweep` over the
/// following ticks, so the cap costs latency and never correctness.
/// Proposed default, DECISIONS.md §open (collapse v0).
pub const MAX_COLLAPSE_PIECES: usize = 64;

/// Pieces the standing-support sweep re-checks per tick (build.rs
/// `support_sweep`), cursor order like the upkeep sweep's. The backstop
/// that lets `MAX_COLLAPSE_PIECES` be a cap rather than a promise: it
/// drops at most **one** unsupported piece a tick and runs that piece's
/// own cascade, so a tick's worst case is one capped collapse however
/// much of the island is hanging in the air. No overflow policy — this is
/// a rate, not a queue; the cursor wraps and nothing is skipped.
/// Proposed default, DECISIONS.md §open (collapse v0).
pub const SUPPORT_SWEEP_PER_TICK: usize = 32;

/// Structural piece removals the whole **tick** may make, across every
/// path that takes a piece out of the store: a raider's killing blow
/// (`deploy::damage_piece`), the decay sweep (`deploy::upkeep_sweep`), the
/// standing-support backstop (`build::support_sweep`), and every cascade
/// any of them seeds (`build::collapse_from`).
///
/// `MAX_COLLAPSE_PIECES` bounds one cascade and cannot bound a tick,
/// because a tick holds many: `upkeep_sweep` does not stop at its first
/// removal the way `support_sweep` does, so its 64 visits can each seed a
/// cascade, and up to `MAX_PLAYERS` raiders can land a killing blow
/// besides. The composed worst case is thousands of `EV_PIECE_REMOVED`
/// against a 256-slot drop-newest ring, and a dropped removal is the one
/// event whose loss is permanent — the piece stays drawn on every screen
/// for the rest of the session. The per-cascade cap stays (it is also what
/// sizes `collapse_from`'s stack array); this is the bound that composes.
///
/// Sized off `MAX_EVENTS_PER_TICK`: a removal spends one event slot, two
/// when a deployable stood at the same address, so 64 removals is at most
/// 128 of 256 and leaves half the ring for everything else the tick says.
/// Overflow policy: **defer** — a refused removal is refused *before* the
/// piece leaves the store, so nothing is lost and nothing is half-removed.
/// The decay sweep rewinds its cursor to retry the same entry, a cascade
/// leaves the rest hanging for `support_sweep`, and a raid's killing blow
/// stops one hp short so the wall falls to the next swing.
/// Proposed default, DECISIONS.md §open (collapse budget v0).
pub const MAX_REMOVALS_PER_TICK: usize = 64;

/// Live satchel charges standing on the world with a fuse burning
/// (`charge.rs`). This is a **client-driven** store — one action plants
/// one charge — so wall 4 applies at its sharpest: without a cap, a
/// client spamming the plant verb grows a per-tick scan without bound.
///
/// Sized off what a raid actually is rather than off a player count: a
/// stone wall at 1750 hp takes four of the alpha's 500-structure charges,
/// so 64 is sixteen simultaneous walls coming down — more than any raid
/// this shard's `MAX_PLAYERS` can mount at once, and small enough that the
/// fuse scan is 64 integer compares a tick.
///
/// Overflow policy: **refuse** — the plant is refused with `REFUSE_B_FULL`
/// and the charge stays in the raider's inventory. Refusing costs the
/// raider a click; the alternative, evicting someone else's live charge,
/// would let one player disarm another's raid by planting into a full
/// store, which is a grief verb rather than a cap.
/// Proposed default, DECISIONS.md §open (satchel fuse v0).
pub const MAX_LIVE_CHARGES: usize = 64;

/// Arrows in flight across the whole shard (`ranged.rs`). Sized off the
/// fire rate, not off `MAX_PLAYERS`: a bow is 30 rounds/min (2 shots a
/// second at 30 Hz is 60 ticks apart) and an arrow lives at most
/// `MAX_ARROW_LIFE_TICKS`, so a shard where every one of the 100 slots is
/// drawing a bow at full cadence holds well under two arrows per player
/// at any instant. 128 is that, doubled.
/// Overflow policy: **refuse the shot** — the store is checked *before*
/// the ammo leaves the quiver, so a refused shot costs the shooter
/// nothing but the tick. It is deliberately not drop-oldest: stealing a
/// live arrow out of the air to make room would make a hit depend on how
/// many other people were shooting, which is the one thing a projectile
/// must never do.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const MAX_ARROWS: usize = 128;

/// Ticks an arrow may stay in flight before it expires, whatever the
/// weapon's own derived life. The backstop that makes `MAX_ARROWS` a
/// bound on *occupancy* rather than a hope: an arrow that somehow misses
/// terrain, occupants, pieces and bodies still leaves the store within
/// four seconds. No overflow policy — this is a lifetime, not a queue.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const MAX_ARROW_LIFE_TICKS: u16 = 120;

/// Millimetres an arrow may advance between two collision samples. Two
/// separate assumptions pin this number and it is the smaller of them:
///
///   * `collide::blocked` documents its endpoint-instead-of-crossing
///     shortcut as costing "at most a fingertip" while steps stay under
///     0.19 m (`SPRINT_SPEED * DT` = 0.183 m). A projectile sampling
///     coarser than that walks through walls the shortcut never promised
///     to catch.
///   * The narrowest thing on the island that blocks is a tree, radius
///     0.26 m at scale 1.0 and 0.234 m at the 0.9 floor — a 0.468 m
///     diameter. At 170 mm a shot through the trunk's centre takes two
///     interior samples, so the trunk stops the arrow rather than
///     flickering past between taps.
///
/// A grazing shot at the very edge of a trunk can still slip through
/// between samples; that is a stated cost of point sampling, not a
/// defect, and the honest fix is a swept test rather than a smaller step.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const ARROW_STEP_MM: i32 = 170;

/// Collision samples one arrow may take in one tick. With
/// `ARROW_STEP_MM` this is also a **content wall**: a weapon whose muzzle
/// speed exceeds `ARROW_STEP_MM * MAX_ARROW_SUBSTEPS` mm/tick (2.72 m per
/// tick, 81.6 m/s) cannot be sampled finely enough to be honest about
/// what it hits, and `bake_combat` refuses it at boot rather than
/// shipping a projectile that tunnels. The bow is 1333 mm/tick and the
/// crossbow 1833, so both sit inside it with room.
/// Overflow policy: none reachable — the bake refusal is what keeps the
/// clamp from ever binding at tick time.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const MAX_ARROW_SUBSTEPS: usize = 16;
