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

/// Class-D entity records **one snapshot** may carry — a per-datagram wire
/// count, not a world cap (NETCODE.md §9 budgets table: typical ~15 / cap
/// 64). It is what `COUNT_BITS = 7` can name and what 1,100 B can hold, and
/// the encoder refuses the 65th record of a single datagram.
/// Overflow policy: **defer** — the priority accumulator keeps accruing
/// what didn't fit (DESIGN.md §5.5).
///
/// ⚠ **It reads like an interest-set cap and it is not one by itself.** A
/// client's interest set is cumulative across snapshots — a delta names
/// only what moved and the client keeps the rest — so the set of entities
/// a client knows about is the *union* of many datagrams, and
/// `MAX_PLAYERS + MAX_MOBS` is what bounds a union. Until 2026-08-10 two
/// places read it as the world cap it is not: `update_interest` marked
/// every body in the 176 m radius interesting (up to 164 of them), and the
/// client's `Interp` table was sized on it while being filled cumulatively,
/// so the 65th distinct id since the last zero-state was accepted by the
/// view, refused by the interpolator, and never drawn.
/// `AOI_RANK_ENTER`/`AOI_RANK_EXIT` below are what make the number true of
/// the interest set as well; `client-core`'s `INTERP_SLOTS` is what stops
/// the client depending on it being true.
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

/// AOI v0's **second** band, in rank rather than in metres — the one that
/// makes `MAX_SNAPSHOT_ENTITIES` true of the interest set instead of only
/// of one datagram (wall 4).
///
/// A radius alone cannot bound the set: `MAX_PLAYERS + MAX_MOBS` is 164 and
/// a fight, a raid or a spawn cluster puts far more than 64 of them inside
/// 176 m of each other, at which point the accumulator is ranking a set the
/// wire could never carry and the client is holding ids it will never see
/// move. So the set is additionally capped to the **nearest**
/// `AOI_RANK_EXIT`, ranked by planar distance with the packed candidate
/// index as the tiebreak (unique, so the ordering is total and the cap is
/// exact).
///
/// **Two ranks, for the reason there are two radii.** A single rank
/// threshold makes two bodies a metre apart at the boundary swap places at
/// 15 Hz, and clearing an interest bit is welded to a wire removal: the
/// client destroys that entity's interpolation history, freezes the body
/// for the 133–200 ms until it is re-sent, and rebuilds its mannequin. So
/// an entity enters at rank < `AOI_RANK_ENTER` and only leaves at rank ≥
/// `AOI_RANK_EXIT`, exactly as it enters at 176 m and leaves at 208 m.
///
/// The band width is **derived from the radii rather than picked**: at
/// uniform density the count inside a radius goes as its square, so a rank
/// band with the same forgiveness as the distance band is the same ratio in
/// area — `AOI_ENTER_CM² / AOI_EXIT_CM²` of the exit rank, which is 45 of
/// 64. A body must therefore improve by 19 places to re-enter after being
/// crowded out, the rank equivalent of walking 32 m closer.
///
/// Overflow policy: **evict by rank**, into the same pending-removal set a
/// distance exit uses — one path out of a client's world, not two. The cost
/// is stated rather than hidden: on a shard denser than this, a client sees
/// 45–64 of its neighbours and not the rest. *Near*, not *nearest* — the
/// hysteresis above makes the set path-dependent on purpose, so an incumbent
/// at rank 60 is kept while a newcomer at rank 46 is refused, and only a body
/// that improves 19 places takes a held slot.
/// Proposed default, DECISIONS.md §open (interest rank cap v0).
pub const AOI_RANK_EXIT: usize = MAX_SNAPSHOT_ENTITIES;
/// **Written as the number it is, with its derivation asserted beside it
/// rather than substituted for it.** It shipped as the expression itself,
/// which read well and cost the whole gate suite: `ci/knob_registry.mjs`
/// pins a registry claim against the constant a source file actually
/// declares, and it cannot evaluate arithmetic — so a computed initializer
/// is not a value it can check, and it fails loudly rather than passing
/// over one. `ci/gates.sh` was red on a clean `main` because of it (found
/// 2026-08-10, merging the tracer branch).
///
/// The literal is what the registry pins; the `assert!` below is what
/// keeps the literal honest, and it is strictly stronger than the
/// expression was — it fails at compile time if either radius or
/// `MAX_SNAPSHOT_ENTITIES` moves without this number moving with them,
/// which is the drift the expression existed to prevent and the gate
/// could not see.
pub const AOI_RANK_ENTER: usize = 45;
const _: () = assert!(
    AOI_RANK_ENTER
        == (MAX_SNAPSHOT_ENTITIES as i64 * AOI_ENTER_CM * AOI_ENTER_CM
            / (AOI_EXIT_CM * AOI_EXIT_CM)) as usize,
    "AOI_RANK_ENTER must stay the enter/exit area ratio of AOI_RANK_EXIT — \
     a radius or the entity cap moved and this number did not"
);

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

/// Rounds one weapon may list in `weapons.toml`'s `ammo` (`RangedDef::ammo`).
///
/// Four because that is what the reference game's bow actually carries —
/// wooden, bone, high-velocity and fire arrows (`reference/PROJECTILES.md`
/// §4) — and a fifth would be a content row nobody has argued for. The bake
/// refuses a longer list rather than truncating it: a silently dropped
/// round is a weapon that stops firing when its first three run out, which
/// reads as a bug in the quiver and not in the table.
///
/// Structural, not a balance number: it sizes a fixed array inside
/// `RangedDef`, so it is a cap in the `MAX_ITEM_DEFS` sense.
/// Proposed default, DECISIONS.md §open (ballistics-on-ammo row).
pub const MAX_WEAPON_AMMO: usize = 4;

/// Stacks a fresh character may be granted at spawn (`content/balance.toml`
/// `[[spawn_kit]]`). Bounded like everything else on a content-driven path:
/// the kit lands in `INV_SLOTS`, so a kit longer than the inventory could
/// only be silently truncated, and wall 4 wants the cap stated rather than
/// discovered. Proposed default, DECISIONS.md §open ("spawn kit v0").
pub const MAX_SPAWN_KIT: usize = INV_SLOTS;

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

/// Building-piece definitions the sim preallocates for (the shipped set
/// is 44 rows — 11 shapes × 4 materials, content/building.toml, since
/// triangles v0). The content bake refuses a set past this. Structural
/// cap like `MAX_ITEM_DEFS`; 48 keeps headroom without widening the
/// wire's `PIECE_DEFS_TOTAL_BITS` (6 bits, 63).
pub const MAX_PIECE_DEFS: usize = 48;

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

/// Authored world containers that have ever been opened at once
/// (`worldcont.rs`: the haven pad's crates, a waystation's caches). Not a
/// budget for how many terrain places — `terrain::scatter` decides that,
/// and this store only ever holds the ones a player has actually reached.
/// The island authors nine today, so the cap is ~7× the live set and the
/// overflow branch is unreachable; `tests/worldcont.rs`'s
/// `the_cap_is_above_what_terrain_authors` is what keeps that true when
/// a monument lands.
///
/// Overflow policy: **refuse** the open — the container does not open and
/// the panel closes, exactly as it does for a bag that despawned. Never
/// **evict**: an emptied record carries the tick it may roll again, so
/// dropping one and re-minting it on the next open would re-roll the loot
/// immediately, which turns the cap into a dupe rather than a limit.
/// Proposed default, DECISIONS.md §open (world containers v0).
pub const MAX_WORLD_CONTS: usize = 64;

/// Slots inside one deployed box. Deliberately under `INV_SLOTS`: a box
/// is a place to put things down, not a second inventory, and the whole
/// store is sized against this (`MAX_BOXES * BOX_SLOTS` stacks). Both
/// box rows in `content/deployables.toml` share it — a per-row slot
/// count is a content field and an open question, not a constant.
/// Proposed default, DECISIONS.md §open (box v0).
pub const BOX_SLOTS: usize = 12;

/// Code locks bolted onto doors at once (`lock.rs`). Its own dense list
/// beside the hearths and the boxes, for the reason both of those have
/// one: `DeployRec` is the struct the wire mirrors, so a code, two
/// remembered lists and two timers may not ride on it. Overflow policy:
/// **refuse** the placement with `REFUSE_D_FULL`, the posture `MAX_BOXES`
/// takes, and correct here for its reason — a lock that was never bolted
/// on loses nothing, and the door stays exactly as usable as it was.
/// Sized under `MAX_DEPLOYS` on purpose: every lock needs a door under
/// it, and a shard with 512 locked doors is a shard with 512 bases.
/// Proposed default, DECISIONS.md §open (lock v1).
pub const MAX_LOCKS: usize = 512;

/// Players one lock remembers with **full** rights — the ones who entered
/// its main code, plus whoever bolted it on. The reference's list is
/// unbounded (`reference/DOORS.md` §2.2); ours cannot be (wall 4).
/// Overflow policy: **refuse the code entry** with `REFUSE_D_AUTH_FULL`.
/// Never evict: dropping the oldest entry would make a door that forgets
/// the person who owns it, which is the one failure this cap must not
/// have. Proposed default, DECISIONS.md §open (lock v1).
pub const LOCK_AUTH_CAP: usize = 8;

/// Players one lock remembers as **guests** — entered the guest code, may
/// work the door and nothing else (`reference/DOORS.md` §2.2, Devblog
/// 149). Its own list rather than a rights bit inside `LOCK_AUTH_CAP`, so
/// a crowd of guests can never crowd out the owners. Same overflow
/// policy, same refusal. Proposed default, DECISIONS.md §open (lock v1).
pub const LOCK_GUEST_CAP: usize = 8;

/// The **grace window** on a fresh placement, in ticks — 10 min at the
/// 30 Hz tick, the reference's own figure for a foundation
/// (`reference/BUILDING.md` §6; their sources disagree about whether
/// other pieces get 30, and §6 says so rather than picking). Inside it a
/// piece can be demolished for a full refund by anyone who may build
/// there; outside it a piece comes down by explosives only.
///
/// It is a *mistake-fix*, not a verb: the question every player asks in
/// their first hour is "I put the foundation in the wrong place", and ten
/// minutes answers it without making a crewmate able to dismantle a base
/// they were let into. Proposed default, DECISIONS.md §open (demolish v1).
pub const DEMOLISH_WINDOW_TICKS: u64 = 18_000;

/// Cells of connected structure one privilege walk may visit
/// (`claim.rs`, privilege v1). The reference asks its question of a
/// physics volume and needs a persistent building identity to afford it;
/// ours is a breadth-first walk over the build grid, and this is what
/// bounds it (wall 4).
///
/// Overflow policy: **the walk stops, and privilege does not extend past
/// it.** That is the honest failure for a bounded search — it can
/// under-claim and never over-claim, so the worst case is somebody
/// building against the far wall of a base bigger than 256 cells (768 m
/// of footprint), and it can never lock a player out of open ground.
/// Proposed default, DECISIONS.md §open (privilege v1).
pub const PRIV_BFS_CELLS: usize = 256;

/// Slots in the claim cache's open-addressed cell → volume map — the
/// rebuild scratch that makes re-walking every hearth's privilege volume
/// affordable in one tick (`claim::ClaimCache`, the upkeep sweep's
/// coverage shape). Structural cap derived exactly as `COL_INDEX_SLOTS`
/// is: 2 × `MAX_PIECES`, power of two, and it can never pass half load
/// because a cell enters the map at most once and only when built, so the
/// key count is bounded by the piece store that built it. Overflow policy:
/// none reachable — every probe terminates at an empty slot while load
/// stays under half, and the pool insert beside it stops at `MAX_PIECES`
/// regardless. Not a knob.
pub const CLAIM_INDEX_SLOTS: usize = 16_384;

/// Players one hearth remembers as its **crew** — who may build, upgrade,
/// repair and deploy inside its claim (`reference/BUILDING.md` §2). Ten in
/// the reference's own vanilla cap, and the same number here for the same
/// reason it is a number at all: a base is a group, and an unbounded group
/// is an unbounded array in a `HearthRec` the wire mirrors nothing of.
/// Overflow policy: **refuse, never evict** — the `Roster`'s rule, and the
/// one this cap must not break is that a hearth never forgets whoever
/// placed it. Proposed default, DECISIONS.md §open (hearth crew v1).
pub const HEARTH_CREW_CAP: usize = 10;

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

/// Cook rows the sim preallocates for (`content/cooking.toml`,
/// `oven.rs`). Structural cap like `MAX_RECIPES`: the bake refuses a set
/// past this rather than silently dropping the tail, because a row the
/// sim never sees is a transformation a player is told about by the
/// catalog and cannot perform.
pub const MAX_COOK_ROWS: usize = 32;

/// Research rows the sim preallocates for (`content/research.toml`,
/// `research.rs`). Structural cap like [`MAX_COOK_ROWS`], and bounded by
/// [`MAX_RECIPES`] for a stronger reason than symmetry: a research row
/// unlocks exactly one recipe and `Player::known` is a bitmask over recipe
/// indices, so a table longer than the recipe set could only be naming a
/// recipe twice. The bake refuses past it.
pub const MAX_RESEARCH_ROWS: usize = MAX_RECIPES;

/// The width of `Player::known`, and therefore the hard ceiling on how
/// many recipes a blueprint can gate. Asserted equal to [`MAX_RECIPES`]
/// rather than assumed: the mask is a `u64` in the sim, in the save file
/// and on the wire, so a 65th recipe is a widening in three places and
/// must fail here rather than silently become unlockable.
pub const KNOWN_MASK_BITS: usize = 64;
const _: () = assert!(
    MAX_RECIPES <= KNOWN_MASK_BITS,
    "Player::known is a u64 mask over recipe indices — a recipe past bit 63 \
     could never be researched, and nothing else would say so"
);

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

/// The widest blast any content may declare, centimetres — one build cell
/// (satchel blast v0). Not a tuning knob: `charge::detonate`'s 3×3 column
/// ring is *complete* only while a blast cannot reach past one cell from
/// its epicentre, the const block there restates it, and `validate`
/// refuses a `blast_m` above it at boot. Widening this means widening the
/// ring in the same commit — the pair is one decision.
pub const BLAST_MAX_CM: u16 = 300;

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

/// Live animals one shard simulates, hard cap and roster length
/// (`mob.rs`). Not a queue: the roster is a fixed array of slots, each
/// with a home the seed chose once, so there is no overflow policy to
/// state — a slot is alive or it is waiting to hatch, and nothing can
/// ever ask for a sixty-fifth pig.
///
/// The number is a density, and it is stated as one: 64 animals over the
/// ~2 km² of land a seed puts inside the continent falloff is ≈ 30 / km²,
/// which is the same order as the reference game's own boar population
/// (`reference/ANIMALS.md` §3). It is also sized against the wire rather
/// than only against the ground: at the 176 m AOI radius one client's
/// disc is 9.7 ha, or 2.3% of the island, so the expectation inside a
/// client's interest set is **1.5 animals** and the tail is nowhere near
/// `MAX_SNAPSHOT_ENTITIES`. A roster ten times this size would still fit
/// the sim's tick budget and would not fit that. Proposed default,
/// DECISIONS.md §open ("animals v0").
pub const MAX_MOBS: usize = 64;

/// The bit that says *this class-D entity is not a player*.
///
/// An animal rides the same snapshot lane a player does, because it is the
/// same thing — a body moving continuously that a client must interpolate
/// (NETCODE.md §1, class D). What it is not is a *player*, and the client
/// has to know which mesh to draw before it can draw anything. The id
/// carries that, in the high bit, rather than a `kind` field on
/// `EntityState`: a field costs bits on **every** record of every snapshot
/// including the 98% that are players, to say something that never changes
/// for the life of an entity and that the id can say for free.
///
/// The cost is that the player id allocator may never set this bit, which
/// is now enforced where ids are minted (`server/net.rs`) rather than
/// hoped for. The wire layout does not move — but `PROTO_VER` does, on
/// v18's precedent: *a widened meaning is a wire change even when the
/// layout is byte-identical*, and a client that did not know this bit
/// would draw a pig as a human being.
pub const MOB_ID_TAG: u32 = 0x8000_0000;

/// Ticks between one animal's decisions — half a second at 30 Hz, and the
/// single most load-bearing number in `mob.rs`.
///
/// It is the reference game's own lesson, applied: their AI moved from
/// thinking every frame to **thinking at a fixed rate**, and it is named
/// in the devblogs as a server-performance change rather than a behaviour
/// one (`reference/ANIMALS.md` §2). Ours is stronger than theirs because
/// it is *phase-offset by roster slot*: mob `i` thinks on ticks where
/// `tick % MOB_THINK_TICKS == i % MOB_THINK_TICKS`, so the per-tick
/// decision work is `MAX_MOBS / MOB_THINK_TICKS` ≈ 4 animals and not 64.
/// Movement still integrates every tick for every waking animal — a body
/// that stepped at 2 Hz would visibly stutter, and the interpolator cannot
/// invent what the sim did not send.
///
/// No overflow policy: it is a cadence, not a queue.
pub const MOB_THINK_TICKS: u64 = 15;

/// One full day/night cycle, in ticks — 45 minutes at `TICK_HZ`
/// (`ALPHA.md` §1's knob; day/night v0, `DECISIONS.md` §open).
///
/// **Time of day is a pure function of the tick, and that is the design,
/// not a shortcut.** The snapshot header already carries the tick on
/// every datagram and the client already tracks a smoothed estimate of
/// it (`client-core/clock.rs`), so shipping a second field would be
/// redundant bytes carrying a derivable number — `NETCODE.md` §3's "time
/// of day rides the G channel" is satisfied by the tick itself plus this
/// constant. What the choice costs is a set-time admin verb: with no
/// offset field anywhere, shifting the clock means shifting the tick,
/// which nothing may do. That is a wire field away if ever wanted.
pub const DAY_TICKS: u64 = 144_000;

/// Phase offset into the cycle at tick zero, so a fresh world — and every
/// capture probe — boots mid-morning with the sun well up, not at the
/// dawn terminator. Exactly halfway from dawn to noon: noon is
/// `DAY_PORTION * 0.5` = 43.75% in, so this is 21.875% of the cycle.
/// **Derived from the two constants around it, not chosen** — it moved with
/// them on 2026-08-15 and would be wrong if it had not.
pub const DAY_PHASE_TICKS: u64 = 31_500;

/// The daylight fraction of the cycle: the first 87.5% is day (70 min),
/// the rest night (10 min).
///
/// **Operator, 2026-08-15, on their own research: the reference's current
/// cycle is ~70 min day + ~10 min night = 80 min.** This is the second value
/// spoken that day and it supersedes the first (60 min as 45 + 15) by the
/// operator's own better source: March 2026's Shipshape update added roughly
/// 20 minutes of daylight and left the ~10-minute night alone. Three
/// generations are now named — 45/15 is the oldest, 50/10 was the 60-minute
/// era, 70/10 is current.
///
/// What moved before this: `NOW.md` §0sun carried "~45 min day + ~15 min
/// night = 60 min" as *measured 2026-08-15* with **no source, no URL and no
/// tier**, and the commit that added it (b59d85e) cited none either. It read
/// as a measurement and was not one. Corrected here rather than only there,
/// because this is the file a builder reads.
///
/// ⚠ **The night SHORTENS.** Read as "a longer day" this looks additive, and
/// it is not: 13.5 min of night becomes 10. Anything tuned against the old
/// night — mob wake windows, `render/audio.rs`'s night threshold, the barrel
/// and bag respawn windows a player waits out in the dark — got a third of
/// its darkness taken away, and none of that was re-derived here.
pub const DAY_PORTION: f32 = 0.875;

/// The most bites one tick can land across the whole roster (mob.rs
/// `Bites`). Derived, generously: only a *thinking* animal can bite, so
/// the true per-tick ceiling is `MAX_MOBS / MOB_THINK_TICKS` ≈ 4 plus
/// rounding — 8 is that with headroom, not a tuning knob. **Overflow
/// drops the bite**: a full buffer is one merciful tick, and the phase
/// lock retries the same pair two seconds later. Never a queue — a bite
/// carried across ticks would land on a player who has already left.
pub const MAX_MOB_BITES_PER_TICK: usize = 8;

/// How close a player must be for an animal to be awake, in centimeters.
///
/// The reference game's word for this is *dormant* — NPCs at a distance
/// from every player stop running (`reference/ANIMALS.md` §2) — and it is
/// the reason a shard can hold thousands of them. Ours is a hard skip of
/// the whole step, and it is **replay-safe rather than a caching trick**:
/// the predicate reads player positions, which are sim state, so a replay
/// wakes exactly the animals the live run woke, on the same ticks.
///
/// 240 m sits deliberately outside `AOI_EXIT_CM` (208 m): an animal is
/// awake for the whole band in which any client can still see it, so
/// nothing freezes in view. It is re-evaluated on the think tick, not
/// every tick, so a waking animal can be up to `MOB_THINK_TICKS` late —
/// half a second, at a distance of two hundred metres.
pub const MOB_WAKE_CM: i64 = 24_000;
