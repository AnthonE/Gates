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
/// 64). It is what `COUNT_BITS = 7` can name, and the encoder refuses the
/// 65th record of a single datagram. It is not a byte bound: the datagram's
/// `DATAGRAM_BUDGET_BYTES` is the other one, and on wide records (every
/// field absolute, a skinned item in hand) the bytes run out first.
/// Overflow policy: **defer** — the priority accumulator keeps accruing
/// what didn't fit, or an overflow datagram carries it (DESIGN.md §5.5).
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

/// Datagrams one client's snapshot may take per tick (wire v121): the
/// snapshot itself, then up to two overflow datagrams of absolute records
/// (`protocol::KIND_SNAPSHOT_MORE`), each a whole `DATAGRAM_BUDGET_BYTES`
/// datagram, never a fragment. The interest set is this many snapshots'
/// worth of bodies (`AOI_RANK_EXIT`); the worst case is this many times
/// the single datagram's bandwidth, and only in a crowd that big.
pub const SNAPSHOT_PARTS: usize = 3;
const _: () = assert!(SNAPSHOT_PARTS >= 1 && SNAPSHOT_PARTS <= 4);

/// `state_hash` cadence in ticks (DESIGN.md §7).
pub const STATE_HASH_INTERVAL: u64 = 32;

/// Snapshot cadence: one snapshot every this many sim ticks — every tick,
/// 30 Hz, since netcode v2 S2 (DECISIONS.md 2026-08-31; was 2, 15 Hz).
/// 15 Hz was the floor under every remote-motion defect: it set the
/// minimum interpolation buffer, halved the sample density every lerp
/// worked from, and made one lost datagram a 133 ms hole. The worst-case
/// datagram is unchanged (~1.06 KB) and doubles in frequency, ~254 kbit/s
/// against §2.1's ~300 kbit/s budget; typical traffic is a fraction of
/// worst case (delta records, a few dozen entities).
pub const SNAPSHOT_INTERVAL_TICKS: u64 = 1;

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
/// area — `AOI_ENTER_CM² / AOI_EXIT_CM²` of the exit rank, which is 137
/// of 192. A body must therefore improve by 55 places to re-enter after
/// being crowded out, the rank equivalent of walking 32 m closer.
///
/// Overflow policy: **evict by rank**, into the same pending-removal set a
/// distance exit uses — one path out of a client's world, not two. The cost
/// is stated rather than hidden: on a shard denser than this, a client sees
/// 45–64 of its neighbours and not the rest. *Near*, not *nearest* — the
/// hysteresis above makes the set path-dependent on purpose, so an incumbent
/// at rank 60 is kept while a newcomer at rank 46 is refused, and only a body
/// that improves 19 places takes a held slot.
/// Proposed default, DECISIONS.md §open (interest rank cap v0).
pub const AOI_RANK_EXIT: usize = MAX_SNAPSHOT_ENTITIES * SNAPSHOT_PARTS;
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
pub const AOI_RANK_ENTER: usize = 137;
const _: () = assert!(
    AOI_RANK_ENTER
        == (AOI_RANK_EXIT as i64 * AOI_ENTER_CM * AOI_ENTER_CM / (AOI_EXIT_CM * AOI_EXIT_CM))
            as usize,
    "AOI_RANK_ENTER must stay the enter/exit area ratio of AOI_RANK_EXIT — \
     a radius or the entity cap moved and this number did not"
);

/// Per-client ring of sent snapshots the server deltas against
/// (NETCODE.md §3). An ack that falls outside it drops the baseline to
/// the canonical zero-state — recovery is the same code path, not a
/// special one. **64 since netcode v2 S2** (was 32): the ring is sized in
/// SNAPSHOTS while its purpose — the baseline window, and the
/// aim-staleness corroboration window derived from it — is wall time, so
/// doubling the snapshot rate without doubling the ring would have halved
/// both to ~1 s and raised zero-state fallbacks under the same loss.
/// Doubling keeps the window at ~2.1 s and `AIM_STALE_CEILING_TICKS` at
/// its measured 80. Cost: ~2.3 KB per slot per client, allocation at
/// construction only (wall 2).
pub const SENT_SNAPSHOT_RING: usize = 64;

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
/// Two ticks of every part (`SNAPSHOT_PARTS`), so a writer one tick behind
/// still finds the newest tick whole.
pub const SNAPSHOT_RING_CAP: usize = 2 * SNAPSHOT_PARTS;

/// The client's inbound datagram ring (netcode v2 S2): snapshots held
/// between render frames, drained in arrival order every `Session::pump`.
/// Overflow policy: **drop-oldest, counted** — past a ~266 ms frame stall
/// at the 30 Hz snapshot rate, older world states are the ones a stalled
/// client least wants to draw. This replaced a depth-one latest-wins
/// `watch`, which was the right shape at 15 Hz for the OWN body (a
/// snapshot replaces its predecessor) and starved the interpolator of the
/// in-between samples every other body's motion is reconstructed from —
/// at ≤ 30 fps, half of a 30 Hz snapshot stream never reached it.
pub const CLIENT_DG_RING: usize = 8 * SNAPSHOT_PARTS;

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

/// Catch-up credit (anti-speedhack): the throttle's second frame is spent
/// from a per-connection credit, never granted by depth alone. Depth alone
/// let a client that minted frames at 60 Hz keep the buffer past the
/// threshold and move at 2× forever (found 2026-09-25). A starved tick on
/// which the body stood still earns one credit; the decay ramp's walking
/// ticks (`input::decay_frame`) earn none, because the ghost already moved
/// the body on them — crediting those too let a lag switch keep the ghost
/// and still run every withheld frame on time (found 2026-10-15). A loss
/// burst still pays for its own catch-up; a drift trickle of one per
/// `INPUT_DRIFT_CREDIT_TICKS` covers clock skew; a fresh connection starts
/// with a full buffer's worth. Net: a tick moves the body at most one stride
/// unless it spends a credit, so a client running ahead or toggling its
/// uplink gains at most ~3% over the tick rate.
pub const INPUT_CATCHUP_CREDIT_CAP: u16 = INPUT_BUFFER_CAP as u16;
/// Ticks per drift credit: one second at 30 Hz.
pub const INPUT_DRIFT_CREDIT_TICKS: u16 = 30;

/// Rewind ring depth in sim ticks — 16 rows of body history, 533 ms
/// (NETCODE.md §8, `findings/lagcomp-design-20260818.md` §1.4). A power of
/// two so the ring index is a mask and never a modulo: wall 1 likes
/// integers and wall 3 likes no division.
///
/// Sixteen holds the fifteen `REWIND_MAX_TICKS` can reach plus the row being
/// overwritten. During tick `T` the rows carry ticks `T-1 ..= T-16`, and the
/// row for `T` is written at the end of `World::tick` — so a rewind of 0 is
/// **the live body**, never a row, and that is what makes a favour of zero
/// bit-identical to the pre-rewind sim.
///
/// Overflow policy: **overwrite oldest**. It is a ring by construction and
/// the row it overwrites is older than `REWIND_MAX_TICKS` can ask for, so
/// nothing reachable is ever discarded. Cost:
/// `MAX_PLAYERS * REWIND_TICKS * 20 B = 32,000 B` of poses plus a
/// `[u64; REWIND_TICKS]` of stamps, all preallocated at construction —
/// nothing allocates in the tick (wall 2).
///
/// Proposed default, DECISIONS.md §open ("lag compensation v0 — the ring").
pub const REWIND_TICKS: usize = 16;

/// Favouring clamp, in ticks: **15 (500 ms)**, and it bounds how far into
/// the victim's past a shooter may be handed a hit they have already run
/// out of.
///
/// It was 7 (233 ms, NETCODE.md §8's 250 ms floored) until the operator's
/// first moving fight over a real link (2026-10-17: *"hard to connect when
/// it visually looked like a hit"*). The favour a frame needs is its age at
/// the shard (RTT plus the input buffer) plus the playout delay plus one,
/// and `tests/lagcomp_e2e.rs` measured 7 clamping every frame from ~100 ms
/// of RTT and missing by ~3.4 ticks at 200 ms — a sprinting body judged
/// 60 cm from where it was drawn. 15 covers ~300 ms of RTT at the playout
/// floor; Source's `sv_maxunlag` default is a second.
///
/// Overflow policy: **clamp**, and a value past it is *also* safe by
/// construction — `Rewind::pose_at` resolves `tick - back` and checks the
/// row's stamp, so an out-of-range `back` lands on a row stamped with a
/// different tick and falls back to the live body. That is deliberate: the
/// failure of a forged favour is the shooter getting *less* help, never
/// more (`findings/lagcomp-design-20260818.md` §4.2c, §6.2).
///
/// Proposed default, DECISIONS.md §open ("lag compensation v0 — the ring").
pub const REWIND_MAX_TICKS: u8 = 15;

/// The interpolation delay both ends agree on: remote bodies are drawn this
/// many ticks in the past. **Unchanged at 4 across netcode v2 S2 on
/// purpose**: the same 133 ms of wall delay is now FOUR snapshot intervals
/// instead of two, and the buffer is sized in intervals (Valve's
/// `cl_interp_ratio` doctrine — ratio 2 is the bare minimum and survives
/// zero dropped packets, ratio 4 survives two consecutive). S5 makes this
/// adaptive; until then the fixed number errs toward robustness at
/// unchanged latency.
///
/// It lives **here** rather than in the client because the *server* needs it
/// to know how stale a client's aim is
/// (`findings/lagcomp-design-20260818.md` §2.2), and a hand-kept mirror of
/// another crate's constant is exactly the drift `CLAUDE.md` opens with.
/// `client-core/src/interp.rs` re-expresses this one as its `f64`; it does
/// not carry a second copy of the number.
///
/// Proposed default, DECISIONS.md §open ("lag compensation v0 — the ring").
pub const INTERP_DELAY_TICKS: u8 = 4;

/// The adaptive playout delay's rails (netcode v2 S5), here because BOTH
/// ends read them: the client breathes its delay between them off measured
/// jitter (`client-core/core.rs`), and the server clamps the delay a
/// datagram REPORTS to the same rails before favour prices it
/// (`server/stats.rs`) — a claim outside what any honest client can render
/// buys nothing. Floor is Valve's ratio-2 minimum (survives zero dropped
/// snapshots); ceiling is the interp span guard's own bound, past which a
/// straddle reads as a teleport anyway.
pub const PLAYOUT_MIN_TICKS: u8 = 2;
pub const PLAYOUT_MAX_TICKS: u8 = 8;

/// Correction for the age of the newest snapshot a client had applied when
/// it made an input. Snapshots land every `SNAPSHOT_INTERVAL_TICKS`, so the
/// expected age of the newest one is half of that — **derived, not
/// picked**, and at interval 1 the floor of that half is **0** (the
/// expected age is 0.5 ticks and a bias is an integer; flooring is the
/// generous-to-the-shooter direction by half a tick, which the
/// `REWIND_MAX_TICKS` clamp still bounds).
///
/// Written as the number with its derivation asserted beside it rather than
/// substituted for it, which is `AOI_RANK_ENTER`'s shape above and for
/// `AOI_RANK_ENTER`'s reason: `ci/knob_registry.mjs` pins a registry claim
/// against the constant a source file declares and cannot evaluate
/// arithmetic, so a computed initializer fails that gate loudly. It shipped
/// as the expression here for exactly as long as it took to read that
/// comment.
///
/// Proposed default, DECISIONS.md §open ("lag compensation v0 — the ring").
pub const REWIND_ACK_BIAS_TICKS: u8 = 0;
const _: () = assert!(
    REWIND_ACK_BIAS_TICKS as u64 == SNAPSHOT_INTERVAL_TICKS / 2,
    "REWIND_ACK_BIAS_TICKS must stay half the snapshot interval — the \
     interval moved and the bias did not, so every favour the server mints \
     is off by the difference"
);

const _: () = assert!(
    REWIND_TICKS.is_power_of_two(),
    "REWIND_TICKS must be a power of two — the ring index is a mask, and a \
     non-power-of-two silently aliases two ticks onto one row"
);

const _: () = assert!(
    (REWIND_MAX_TICKS as usize) < REWIND_TICKS,
    "REWIND_TICKS must hold every tick REWIND_MAX_TICKS can ask for plus the \
     row being overwritten — at equality the deepest rewind reads the row \
     this tick is about to write"
);

/// Per-client pending-removal set: entity ids that left the client's
/// interest, re-sent in every snapshot until a snapshot carrying them is
/// acked (so a ghost can't survive datagram loss). Overflow policy:
/// **resync** — the client falls to the zero-state baseline, which is the
/// recovery path for everything else too. Proposed default, DECISIONS.md
/// §open.
pub const PENDING_REMOVALS_CAP: usize = 256;

/// Item definitions the sim preallocates for (CONTENT.md §2). The content
/// bake refuses a set past this. 192 since Rust's other rounds and the
/// weapon attachments filled 160 (127 filled the old 7-bit held field); the
/// wire's held field is 8 bits since v135, so anything up to 255 fits
/// beside its empty-hand code (`protocol::HELD_NONE`). Every per-item table
/// in `World` grows with it.
pub const MAX_ITEM_DEFS: usize = 192;

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

/// Projectiles one gunshot may throw — a shotgun's pellets
/// (`combat::Gun::pellets`). Each is traced and paid on its own, so this
/// bounds a pull's work at `MAX_PELLETS × MAX_HITSCAN_SAMPLES` taps.
pub const MAX_PELLETS: usize = 16;

/// Weapons that may carry a **magazine** at once — the width of
/// `Player::mag`, which is indexed by `RangedDef::mag_slot` rather than by
/// item index.
///
/// The index is dense and assigned at bake time to the weapon rows that
/// declare `magazine > 0`, so this is not `MAX_ITEM_DEFS`: sixty-four
/// slots per player would be sixty-four `u16` pairs of which one is used,
/// and the store is per-player so the waste multiplies by `MAX_PLAYERS`.
/// Eight because `content/weapons.toml` has one firearm today and the
/// catalogue a wipe cycle is priced for (`ALPHA.md` §1) is single digits;
/// the bake **refuses** a ninth rather than dropping it, because a weapon
/// with no magazine slot would silently fire straight out of the pack —
/// exactly the mechanic this cap exists to make impossible to lose.
///
/// Structural, not a balance number: it sizes a fixed array on `Player`,
/// so it is a cap in the `MAX_WEAPON_AMMO` sense.
/// Proposed default, DECISIONS.md §open (reload v1).
pub const MAX_MAGS: usize = 16;

/// Stacks a fresh character may be granted at spawn (`content/balance.toml`
/// `[[spawn_kit]]`). Bounded like everything else on a content-driven path:
/// the kit lands in `INV_SLOTS`, so a kit longer than the inventory could
/// only be silently truncated, and wall 4 wants the cap stated rather than
/// discovered. Proposed default, DECISIONS.md §open ("spawn kit v0").
pub const MAX_SPAWN_KIT: usize = INV_SLOTS;

/// Inventory slots per player: 6 hotbar + 24 backpack (ALPHA.md §1).
pub const INV_SLOTS: usize = 30;

/// Worn-equipment slots per player (`Player::worn`) — one head, one body,
/// in that order, and the array is **indexed by the slot** rather than
/// appended to.
///
/// Two because `content/armor.toml`'s `ArmorSlot` has exactly two variants
/// and every row in the shipped set is one of them. It is not a guess at a
/// future roster: a legs or boots rung is a content schema change and a
/// widening here, in one commit, exactly as a new container kind is.
///
/// Overflow policy: **there is none, by construction.** Nothing is ever
/// pushed — a piece goes into the slot its baked row names (`ArmorDef::slot`,
/// one-based) and replaces whatever was there, so the only way to exceed the
/// cap is to name a slot the array does not have, which `combat::worn_pct`
/// refuses by ignoring the stack. Wall 4 wants the cap stated; this is it.
/// Proposed default, DECISIONS.md §open ("armor reduction v0"). 2 → 3 for
/// the feet (Rust's boots, 2026-10-18). 3 → 7 for clothing v1: Rust's seven
/// clothing slots, as head, body, feet, legs, hands, shirt and face.
pub const WEAR_SLOTS: usize = 7;

/// Hotbar width — the first `HOTBAR_SLOTS` inventory slots, the only ones
/// the held-item selector may name (ALPHA.md §1). The wire carries the
/// selector in 3 bits and refuses 6–7; a non-wire command with an invalid
/// selector falls back to slot 0 (`world::apply`).
pub const HOTBAR_SLOTS: usize = 6;

/// Recipe definitions the sim preallocates for (the alpha ladder is ~34
/// rows, CONTENT.md §2). The content bake refuses a set past this.
/// Proposed default, DECISIONS.md §open (craft verb row). 64 → 128 when
/// Rust's tree items arrived (operator, 2026-10-17): the 62 rows of the day
/// left two for a tree that wanted twenty. 128 → 192 for electricity (127
/// were taken): the blueprint mask stays 128 bits, because blueprint recipes
/// rank first (`content` `recipe_index`) and only they are ever asked of it.
/// Below 256 so a recipe index and a count still cross in 8 bits.
pub const MAX_RECIPES: usize = 192;

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

/// Proximity voice's radius, centimetres, 3D (wire v113). The server relays
/// a talker's frames to nobody farther (`reference/VOICE.md` §9.1: the cull
/// is the server's or it is a wallhack). **The reference's own number**: its
/// server sends a voice packet to every connection within 100 m of the
/// speaker (`BasePlayer`'s voice relay, read in its decompiled server), and
/// the client's falloff is what makes 40 m clear and 90 m a murmur
/// (`render/voice.rs`). Inside the AOI enter radius, so whoever hears you
/// is already an entity you can see.
pub const VOICE_RADIUS_CM: i64 = 10_000;

/// How far a gunshot is reported past the interest set, centimetres
/// (`EventMsg::Report`, v113). Inside it the shot rides `EV_SHOT` and its
/// body; past it, out to here, the client is told only that a gun went off
/// and where. The reference's pistol class carries 400 m (rifles 500,
/// heavy guns 600; its Sept 2025 maintenance devblog), and the revolver is
/// the only gun on the island.
pub const GUN_REPORT_CM: i64 = 40_000;

/// How far a blast is reported past the interest set, centimetres (v113).
/// The reference sends every explosion to every client; 1500 m on a 4 km
/// island is a raid heard from most of it.
pub const BLAST_REPORT_CM: i64 = 150_000;

/// Per-connection inbound ring of a talker's voice frames. Overflow policy:
/// **drop newest**, counted — the reader rate-limits ahead of it, and a
/// late frame is worth less than the next one.
pub const VOICE_RING_CAP: usize = 16;

/// Voice frames the sim relays per talker per tick: 50 frames a second at
/// 30 Hz is under two, so four takes a burst. The rest wait a tick.
pub const VOICE_PER_TICK: usize = 4;

/// Per-connection outbound ring of relayed voice datagrams: a crowd of
/// talkers, a few ticks deep. Overflow policy: **drop newest**, counted —
/// a listener whose writer is behind loses a frame, never the snapshot.
pub const VOICE_OUT_RING_CAP: usize = 64;

/// The voice token bucket, per connection: frames a second at a steady
/// state (one per 20 ms), and the burst a jittery uplink may bunch.
pub const VOICE_FPS: u32 = 50;
pub const VOICE_BURST: u32 = 10;

/// How far another body's deed is heard, centimetres (wire v93,
/// `SUB_HEARD`): a reload, a meal, a drink, a keypad, a lid. The server
/// sends it to nobody farther, so the fact goes no further than its sound —
/// the widest of those cues' radii (a reload's, 30 m) and a little over.
pub const ACT_HEAR_CM: i64 = 3_200;

/// Sparse slot-life store (harvested/damaged scatter slots). Sized past
/// the ~14–17 k live slots a seed produces (TERRAIN.md §6, since forest
/// density v1 on 2026-09-14 — it was 16,384 past ~8–12 k before, and
/// `tests/scatter.rs::test_scatter_density_preserved` holds the band under
/// this cap so the sizing cannot drift apart from what it is sized for) so
/// harvested entries always fit. Overflow policy: **evict** —
/// standing-damage entries only, lowest hits first (the evicted node
/// heals); harvested entries are never evicted, and a store somehow full
/// of them refuses the hit. Proposed default, DECISIONS.md §open (gather
/// bounds row).
///
/// A save carries at most this many entries (`worldsave.rs` `count32`), so
/// a file written under the wider cap with more than the old 16,384 does
/// not load on a shard built before it — the cap is a bound, not a layout,
/// and `WORLD_SAVE_FORMAT` did not move for it.
pub const MAX_SLOT_LIVES: usize = 131_072;

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
/// The same memo on the shard's `World`, scaled with the roster: a body
/// probes nine cells, and a shard whose players are spread out has
/// `MAX_PLAYERS * 9` live at once. At 1 024 lines those collided and missed
/// over half the time, which made movement 94 % of a spread tick (profile,
/// 2026-10-20: 100 players at 60 m, 9.4 ms of sim against 0.2 ms clustered).
/// About 24 B a line.
pub const SLOT_CACHE_LINES: usize = (128 * MAX_PLAYERS).next_power_of_two();
const _: () = assert!(SLOT_CACHE_LINES >= SLOT_CACHE_SLOTS);

/// Lines in the direct-mapped memo of `cliff::crag` that the occupant query
/// reads cliff crags through (occupy.rs), on the shard's `World`. A body's
/// query scans at most 64 spots, so a full shard has `MAX_PLAYERS * 64` live
/// at once — kept under 5 % of the lines, because at 32 768 a spread shard's
/// spots collided often enough to miss 30 % of the time (profile,
/// 2026-10-20). 44 B a line. `SLOT_CACHE_SLOTS`'s posture: a memo of a pure
/// function, so a collision re-resolves (five ground taps off a cliff) and
/// nothing can be lost. Must stay a power of two.
pub const CRAG_CACHE_LINES: usize = (1_024 * MAX_PLAYERS).next_power_of_two();
/// The same memo for one predicted body (`ClientCore`, fixtures).
pub const CRAG_CACHE_LINES_ONE: usize = 512;
/// Lines in each direct-mapped memo of the faces by whole-metre point (the
/// ground's grade, the coarse points and the solid surface, `face.rs`; the
/// skin's half-metre points get twice as many), on the shard's `World`: a
/// walking body asks a few points a tick and a new row every few ticks.
/// 52 B a point in all. Must stay a power of two.
pub const FACE_CACHE_LINES: usize = 8_192;
/// The same memos for one predicted body.
pub const FACE_CACHE_LINES_ONE: usize = 512;

/// Building-piece definitions the sim preallocates for (the shipped set
/// is 44 rows — 11 shapes × 4 materials, content/building.toml, since
/// triangles v0). The content bake refuses a set past this. Structural
/// cap like `MAX_ITEM_DEFS`; 48 keeps headroom without widening the
/// wire's `PIECE_DEFS_TOTAL_BITS` (6 bits, 63).
pub const MAX_PIECE_DEFS: usize = 96;

/// Cost rows one building piece may carry (alpha data uses 1). The bake
/// refuses past it. Structural cap, not a knob.
pub const MAX_PIECE_COSTS: usize = 2;

/// Placed building pieces per shard. Overflow policy: **refuse** the
/// placement with an integer reason code (EV_BUILD_REFUSED) — a full world
/// refuses loudly, never evicts someone's wall. Proposed default,
/// DECISIONS.md §open (build grid row).
pub const MAX_PIECES: usize = 32_768;

/// Buildings per shard (freeform W3): each is a frame, minted by a
/// foundation placed free and freed with its last piece. A building has at
/// least one piece, so this caps nothing a full piece store would not —
/// except a world of single foundations, which it refuses past 4,096 with
/// `REFUSE_B_FULL`. Structural, not a knob.
pub const MAX_BUILDINGS: usize = 4_096;

/// Build-grid coordinate ceiling: cells index 0..MAX_BUILD_COORD on each
/// axis, matching the wire's 11-bit cell fields (the 4,096 m island spans
/// ~1,366 3 m cells; foundations further out die on the terrain rule first).
pub const MAX_BUILD_COORD: usize = 2_048;

/// Build levels (vertical storeys) per cell, 0-based. The wire carries the
/// level in 3 bits — exactly this range. Proposed default, DECISIONS.md
/// §open (build grid row).
pub const MAX_BUILD_LEVELS: usize = 8;
/// Whole and half-storey socket addresses. Whole storeys retain codes 0..8;
/// codes 8..16 sit half a storey above the corresponding whole level.
pub const MAX_BUILD_SOCKETS: usize = 16;
const _: () = assert!(MAX_BUILD_SOCKETS == MAX_BUILD_LEVELS * 2);

/// Column-index slots (collide.rs): open-addressed map from build column
/// to occupancy masks, power of two, 2 × MAX_PIECES so it never passes
/// half load (each piece occupies at most one column). Structural cap
/// derived from MAX_PIECES, not a knob.
pub const COL_INDEX_SLOTS: usize = 2 * MAX_PIECES;
const _: () = assert!(COL_INDEX_SLOTS.is_power_of_two());

/// Slots in the piece store's address → index table (`build::Pieces`):
/// one row per piece, power of two, 2 × MAX_PIECES so it never passes half
/// load. Structural, not a knob.
pub const PIECE_ADDR_SLOTS: usize = 2 * MAX_PIECES;
const _: () = assert!(PIECE_ADDR_SLOTS.is_power_of_two());
const _: () = assert!(MAX_PIECES <= u16::MAX as usize + 1);

/// Deployable definitions the sim preallocates for (content/deployables.toml).
/// The content bake refuses a set past this; the wire carries the row in 6
/// bits — exactly this range (32 until power's eight parts, wire v149).
/// Structural cap like `MAX_PIECE_DEFS`.
pub const MAX_DEPLOY_DEFS: usize = 64;

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
pub const MAX_DEPLOYS: usize = 4_096;

/// Slots in the collision index's table of blocking deployables
/// (`collide::SolidPose`): every free-placed solid deployable has its own
/// row, keyed by its cell, so a cell may hold several. Power of two and
/// 2 × `MAX_DEPLOYS`, so it never passes half load. Structural, not a knob.
pub const SOLID_SLOTS: usize = 2 * MAX_DEPLOYS;
const _: () = assert!(SOLID_SLOTS.is_power_of_two());

/// Rows in the freeform broadphase (`space::Space`, unwired): a piece or a
/// deployable files under up to four 4 m buckets, and the table refuses past
/// half load, so this holds every piece and deployable at the caps with room
/// over. Power of two. Structural, not a knob.
pub const SPACE_SLOTS: usize = 16 * MAX_PIECES;
const _: () = assert!(SPACE_SLOTS.is_power_of_two());
const _: () = assert!(SPACE_SLOTS / 2 >= 4 * (MAX_PIECES + MAX_DEPLOYS));

/// Hearths per shard, tracked in their own dense list so claim checks
/// and the upkeep sweep scan hearths, never the whole deploy store.
/// Overflow policy: **refuse** the hearth placement. Proposed default,
/// DECISIONS.md §open (deployables row).
pub const MAX_HEARTHS: usize = 512;

/// Player auto turrets with a gun, shard-wide (`turret.rs`): each rides a
/// roster slot below the town's sentries, taken from the animals' range —
/// 64 leaves 187 animal slots, about three times what the shipped
/// densities spawn. Past it, a placed turret stands inert until one comes
/// down or an idle one (dry or unpowered) gives its entry up to it
/// (`turret::reconcile`). Was 24 until wire v121, which a few bases on a
/// full shard used up.
pub const MAX_TURRETS: usize = 64;

/// Guns a turret can carry (`deployables.toml` `[turret]` `guns`): the
/// baked table is a fixed array this long, and a set naming more refuses to
/// bake. Overflow policy: **refuse** at boot.
pub const MAX_TURRET_GUNS: usize = 16;

/// Rust's turret interference (2023): at most this many turrets within
/// [`TURRET_INTERFERENCE_M`] of each other shoot; the rest stand idle with
/// their guns up, first come first served in table order (`turret::step`).
pub const TURRET_INTERFERENCE: usize = 12;
/// The interference radius, metres.
pub const TURRET_INTERFERENCE_M: f32 = 40.0;

/// Turrets one player may have placed (`deploy::place_deploy`): more than
/// [`TURRET_INTERFERENCE`] in one place never all fire anyway, and the
/// [`MAX_TURRETS`] table is shard-wide, so one hand may not hold all of it.
/// Overflow policy: **refuse** the placement with `REFUSE_D_OWN_CAP`.
pub const MAX_TURRETS_PER_OWNER: usize = TURRET_INTERFERENCE;
const _: () = assert!(MAX_TURRETS_PER_OWNER < MAX_TURRETS);

/// Wires on the island at once (`power::PowerStore`): a part has at most
/// three outputs, and this is two for every part [`MAX_POWER_NODES`] holds.
/// Overflow policy: **refuse** the connect with `power::REFUSE_W_FULL`.
pub const MAX_WIRES: usize = 2_048;

/// Power parts the solve takes in a period (`power::Grid`), shard-wide.
/// Past it, a part stands dark until one comes down: it is not in the grid,
/// so nothing reaches it. Structural, not a knob: the grid's arrays and its
/// index are sized by it.
pub const MAX_POWER_NODES: usize = 1_024;

/// Power parts of one owner the solve takes (`power::Grid`): the grid is
/// shard-wide ([`MAX_POWER_NODES`]), so one hand may not hold all of it.
/// Overflow policy: **refuse** the placement with `REFUSE_D_OWN_CAP`
/// (`deploy::place_deploy`); a part past it all the same (one placed before
/// the cap) stands dark.
pub const MAX_POWER_NODES_PER_OWNER: usize = 64;
const _: () = assert!(MAX_POWER_NODES >= 16 * MAX_POWER_NODES_PER_OWNER);

/// Batteries holding a charge at once (`power::CellRec`). A battery past it
/// holds nothing until one comes down. Structural, like the grid's.
pub const MAX_BATTERIES: usize = 512;
const _: () = assert!(MAX_BATTERIES <= MAX_POWER_NODES && MAX_WIRES <= 2 * MAX_POWER_NODES);

/// Players one auto turret's authorize list holds (`turret::TurretRec`),
/// the lock's `LOCK_AUTH_CAP`. Overflow policy: **refuse** the add with
/// `REFUSE_D_AUTH_FULL`, never evict (`roster.rs`).
pub const TURRET_AUTH_CAP: usize = 10;

/// Turrets with a record of their own (`turret::TurretRecs`): one whose list
/// is still its owner alone keeps none, so this counts the turrets somebody
/// has authorized on, deauthorized from or cleared. Overflow policy:
/// **refuse** the op that would create one with `REFUSE_D_FULL`.
pub const MAX_TURRET_RECS: usize = 512;
const _: () = assert!(MAX_TURRET_RECS <= MAX_BOXES);

/// Stock rows per hearth — one per distinct upkeep material (the union
/// of building-cost items; the alpha build table uses 3). The bake
/// refuses a build table needing more. Structural cap, not a knob.
pub const HEARTH_STOCK_ROWS: usize = 4;

/// Steps in the upkeep rent's size ladder (upkeep v2, `upkeep::tax`): past
/// how many graded pieces a base pays which rate. The reference prices four
/// brackets and the first is our flat `upkeep_pct_per_day`, so three steps
/// carry theirs; the fourth is headroom. The bake refuses a longer ladder.
/// Structural cap, not a knob.
pub const UPKEEP_STEPS: usize = 4;

/// Stocked hearths whose death one tick can turn into grief protection
/// (upkeep v2, `deploy::grieve`): each is parked at removal and drained at
/// the end of the tick, the box spill's shape. Overflow policy: **the
/// excess buys nothing** — a hearth destroyed past the cap in one tick
/// leaves its base unprotected, which under-protects and can never
/// over-protect. Eight is several raids' worth of cupboards dying in one
/// tick; the drain is three passes over the piece store per entry.
/// Proposed default, DECISIONS.md §open (upkeep v2).
pub const MAX_GRIEF_PER_TICK: usize = 8;

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
///
/// **It is authored as `BODY_BROADCAST_ARMS × AOI_RANK_EXIT` and typed as a
/// literal, which is deliberate and was wrong for one commit.** The product
/// is the whole argument for the value; writing the product *as the
/// definition* is what makes the argument uncheckable, because the assertion
/// that pins the relation
/// (`server/tests/snapshot_budget.rs::the_filter_buys_nothing_on_a_clustered_shard`)
/// then compares an expression to itself and is green for every value of
/// every term. It was `BODY_BROADCAST_ARMS * AOI_RANK_EXIT` here between
/// wire v54 and this line; the mutant sweep is in the judge report for pass
/// `20260829-153230-02`. Two independently authored numbers, one assertion
/// between them — the shape every cross-check in this file has.
///
/// A broadcast arm in `pump_events` costs one push into *this* ring per
/// connected client, so the per-client fan-in from one class-D-filtered arm
/// (`ShardCore::body_event_visible`) is bounded by the interest set, which
/// the rank band caps at `AOI_RANK_EXIT`. One such arm can therefore fill
/// `AOI_RANK_EXIT` slots by itself, and the ring has to hold one band per
/// arm or a firefight overflows it on arithmetic alone.
///
/// ⚠ **It was a flat 64 until wire v54, equal to `AOI_RANK_EXIT`, and that
/// left zero headroom** — the doc here said so in those words, and
/// `snapshot_budget.rs` had measured the peak at 50 of 64. The second arm
/// is what called it in: `EV_SHOT` became a firearm's report as well as a
/// bow's, so a hundred co-located players firing offered one client **82**
/// messages in a tick against the 64-slot ring. That is the predicted
/// failure arriving, not a new one, and the fix is the one the assertion
/// named first.
///
/// The overflow is self-amplifying — a refused push calls `ev_resync`, and
/// the recovery drip pushes *more* messages into the ring that just refused
/// — which is why this is sized against the worst case rather than the
/// common one. Overflow policy is unchanged: **resync**.
/// Proposed default, DECISIONS.md §open (event-lane fan-out v0).
///
/// 192 since wire v93, when the heard deeds became the third body arm; 576
/// since wire v121, when snapshot overflow tripled the rank band.
pub const EVENT_RING_CAP: usize = 576;

/// How many `pump_events` arms broadcast a **body's** fact to the whole
/// class-D interest set, each therefore able to fan in `AOI_RANK_EXIT`
/// messages to one client in one tick.
///
/// Three: `EV_SWING` (an arm moved), `EV_SHOT` (a round left a weapon), and
/// since wire v93 the heard deeds (`ShardCore::flush_heard`: a reload, a
/// meal, a drink, a keypad, a lid — one per body per tick, so one band).
/// All are filtered by `ShardCore::body_event_visible`, and all fire on
/// exactly the tick everyone is co-located — a raid — so the filter buys
/// nothing precisely when the ring is fullest.
///
/// **This is a count of code sites, so it would rot the way a hand-kept
/// mirror always does** (`CLAUDE.md`'s `props.js` line) — and the answer is
/// the one that file gives: read the surface, do not remember it.
/// `server/tests/snapshot_budget.rs::every_body_broadcast_arm_is_counted`
/// scrapes `server/src/core.rs` for the filter's own call sites and asserts
/// the count equals this constant, so a third arm is red at the arm, in the
/// commit that adds it, whether or not anyone thought about the ring.
///
/// A second assertion in the same file
/// (`the_filter_buys_nothing_on_a_clustered_shard`) then pins
/// `EVENT_RING_CAP == BODY_BROADCAST_ARMS * AOI_RANK_EXIT`, which is the
/// sizing law. The two are a chain and both links are needed: the scrape
/// catches an uncounted arm, the equality catches a counted arm nobody
/// resized the ring for, and a widened rank band. For one commit the second
/// link was the ring's own definition and therefore held nothing — see
/// [`EVENT_RING_CAP`].
/// Proposed default, DECISIONS.md §open (event-lane fan-out v0).
pub const BODY_BROADCAST_ARMS: usize = 3;

/// Slot-life entries the per-client harvested-set walk scans per tick
/// (join sync / resync is drip-fed: at most one sync message per client
/// per tick, at most this many entries examined to fill it). Proposed
/// default, DECISIONS.md §open (gather wire row).
///
/// 2,048 since the walk is aimed and restarts on every class-S re-arm
/// (`server::interest`): a full store, `MAX_SLOT_LIVES`, is then 64 scan
/// windows plus at most ~88 windows a full batch of in-range cells ends
/// early, 153 ticks, inside which a sprinter covers 28 m of the 32 m that
/// re-arms it — the piece walk's inequality, held by the server's
/// `a_full_harvested_walk_finishes_before_a_sprinter_re_arms_it`. A
/// server-side work cap: no wire, save or hash sees it.
pub const SYNC_SCAN_PER_TICK: usize = 2_048;

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

/// Loose stacks lying on the ground at once (`grounditem.rs`: what a
/// smashed barrel leaves behind). **Its own cap and not a share of
/// `MAX_BACKPACKS`**, which is the whole of what splitting the stores
/// bought — a barrel-heavy shard used to evict somebody's death bag to
/// make room for three units of cloth, because the 2026-08 row put both
/// in one store and said so (*"a barrel-heavy shard can evict an old
/// death bag. Raise the cap or split the stores if that is wrong."*).
///
/// Sized off what the island can produce: our barrels roll 1–2 stacks
/// (`content/loot.toml`), so 256 is over a hundred barrels' worth lying
/// around unclaimed at once, against a despawn ladder that clears the
/// common tier in minutes. Overflow policy: **evict** the stack nearest
/// its own despawn — `MAX_BACKPACKS`' policy and for its reason (refusing
/// would quietly eat loot a player paid three swings for), and unlike
/// `MAX_WORLD_CONTS` an eviction here cannot dupe, because a loose stack
/// holds no timer anybody can re-roll. Proposed default, DECISIONS.md
/// §open (ground items v0).
///
/// 256 → 512 when landed arrows became loose stacks (`spent.rs`): they
/// used to lie in their own 512-entry store, and a volley now shares this
/// one with the barrels.
pub const MAX_GROUND_ITEMS: usize = 512;

/// Of [`MAX_GROUND_ITEMS`], the slots a thrown stack (`Command::Drop`)
/// never takes: a throw into a store this full lands in the bag at the
/// thrower's feet instead (`grounditem::GroundItems::toss`). The store is
/// shard-wide and evicts, so a flood of one-unit drops otherwise pushed out
/// every barrel's scatter and every landed arrow on the island; with this,
/// a drop never evicts anything and those keep the last quarter.
pub const DROP_HEADROOM: usize = MAX_GROUND_ITEMS / 4;

/// Stacks in the air at once (`grounditem::Tosses`): a `Command::Drop` throws
/// its stack out of the hand, and the throw is remembered only while it
/// flies (two thirds of a second off a standing hand on level ground).
/// **Overflow forgets the oldest throw**, and that stack simply lies where
/// it landed on every screen that had not seen it fly — the record is the
/// truth either way, the throw is only its picture.
pub const MAX_TOSSES: usize = 32;

/// Line-of-sight walks one take, loot or pull may spend finding the nearest
/// thing in reach (`reach::nearest_clear`), nearest first. Bounds a command
/// against a pile of stacks heaped behind a wall: past this many blocked
/// candidates the hand finds nothing.
pub const HAND_SIGHT_TESTS_MAX: usize = 8;

/// Deployed storage boxes standing at once (`deploy.rs`, `ARCH_BOX`).
/// Held in its own dense list beside the hearths, for the same reason:
/// `DeployRec` is the struct the wire mirrors, so contents may not ride
/// on it. Overflow policy: **refuse** the placement with
/// `REFUSE_D_FULL` — the same posture `MAX_HEARTHS` takes, and correct
/// here for the same reason: nothing is lost by refusing a box that was
/// never placed. Proposed default, DECISIONS.md §open (box v0). 2,048
/// since each hand's share was capped ([`MAX_BOXES_PER_OWNER`]): at 1,024
/// sixteen accounts filled it. A count, not a stride, so old saves load.
pub const MAX_BOXES: usize = 2_048;
/// Of [`MAX_BOXES`], how many one player may have placed — boxes, fires,
/// furnaces, planters, turrets, every container (`deploy::holds_items`).
/// The store is shard-wide and a small box is cheap, so without a share one
/// player kept every fire, furnace and box on the island from being placed.
/// Overflow policy: **refuse** with `REFUSE_D_OWN_CAP` (the store's own is
/// `REFUSE_D_FULL`).
pub const MAX_BOXES_PER_OWNER: usize = 64;
// Every container stands on a deploy record, and filling the store takes
// thirty-two hands' whole shares.
const _: () = assert!(MAX_BOXES <= MAX_DEPLOYS && MAX_BOXES >= 32 * MAX_BOXES_PER_OWNER);

/// Living plants on the island at once, in planter beds and on the open
/// ground (`farm.rs`). One fixed store; a farm period visits at most
/// `ceil(MAX_PLANTS / 15)` of them. Overflow policy: **refuse** the
/// planting with `REFUSE_D_PLANTS_FULL` — the seed stays in the hand.
pub const MAX_PLANTS: usize = 4_096;

/// Of [`MAX_PLANTS`], how many may stand in the open ground, island-wide
/// and per sower (`farm::ground_ok`). A ground plant never dies before it
/// ripens, so these are what bound a field sown to spam. Overflow policy:
/// **refuse** the planting with `REFUSE_D_PLANTS_FULL`.
pub const MAX_GROUND_PLANTS: usize = 512;
pub const MAX_GROUND_PLANTS_PER_OWNER: usize = 48;

/// Water catchers standing at once, island-wide (`deploy::ARCH_CATCHER`).
/// Each one scans the boxes and the ground plants every tenth farm period
/// to water what stands within its reach, so this bounds that scan.
/// Overflow policy: **refuse** the placement with `REFUSE_D_FULL`.
pub const MAX_CATCHERS: usize = 128;

/// Crop rows the sim preallocates for (`content/cooking.toml` `[[crop]]`).
/// Structural cap like [`MAX_COOK_ROWS`]: the bake refuses a set past it.
/// A plant stores its crop as this table's index.
pub const MAX_CROPS: usize = 8;

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
pub const MAX_WORLD_CONTS: usize = 256;

/// Vendor offers on the island, all kiosks together (`vend.rs`). Refused
/// past at bake.
pub const MAX_VEND_OFFERS: usize = 64;
/// Vendors: one per town kiosk.
pub const MAX_VENDORS: usize = crate::town::KIOSKS.len();

/// Works on the island (`works.rs`, `ARC.md` F1): the world-state table
/// `WORLD.md` §5.4 asked for. The bake refuses a longer `[[work]]` list.
pub const MAX_WORKS: usize = 8;
/// Inputs one work's quota names.
pub const MAX_WORK_INPUTS: usize = 4;
/// Unlock flags across every work, floor and ceiling: the width of
/// `Works::unlocks`. A recipe names one as its code, bit + 1.
pub const MAX_UNLOCKS: usize = 32;
/// Contributors one work credits. Full, a new giver displaces the smallest
/// credit only when their deposit alone is bigger; otherwise it goes
/// uncredited (the work still counts it).
pub const MAX_WORK_CREDITS: usize = 16;
/// Effect rows (`[[effect]]`): an unlock turning a knob.
pub const MAX_ARC_EFFECTS: usize = 16;
/// Speakers on the island (`lore.rs`, `ARC.md` F5).
pub const MAX_SPEAKERS: usize = 16;
/// Inscriptions on the island (`lore.rs`, `ARC.md` F6).
pub const MAX_INSCRIPTIONS: usize = 32;
/// The ancients' alphabet: the width of `Player::glyphs`.
pub const MAX_GLYPHS: usize = 64;
/// Mechanisms on the island (`mech.rs`, `ARC.md` F7), and dials one has.
pub const MAX_MECHS: usize = 8;
pub const MAX_DIALS: usize = 4;

/// Slots inside one deployed box. Deliberately under `INV_SLOTS`: a box
/// is a place to put things down, not a second inventory, and the whole
/// store is sized against this (`MAX_BOXES * BOX_SLOTS` stacks). Both
/// box rows in `content/deployables.toml` share it — a per-row slot
/// count is a content field and an open question, not a constant.
/// Proposed default, DECISIONS.md §open (box v0).
pub const BOX_SLOTS: usize = 12;

/// Vending machines with offers set, shard-wide (`shop.rs`). A machine with
/// no offers holds no record; past the cap a first `SET` refuses.
pub const MAX_SHOPS: usize = 64;
/// Offers one vending machine shows.
pub const SHOP_ROWS: usize = 6;

/// Code locks bolted onto doors at once (`lock.rs`). Its own dense list
/// beside the hearths and the boxes, for the reason both of those have
/// one: `DeployRec` is the struct the wire mirrors, so a code, two
/// remembered lists and two timers may not ride on it. Overflow policy:
/// **refuse** the placement with `REFUSE_D_FULL`, the posture `MAX_BOXES`
/// takes, and correct here for its reason — a lock that was never bolted
/// on loses nothing, and the door stays exactly as usable as it was.
/// Sized under `MAX_DEPLOYS` on purpose: every lock needs a door under
/// it, and a shard with 2,048 locked doors is a shard with hundreds of
/// bases.
/// Proposed default, DECISIONS.md §open (lock v1).
pub const MAX_LOCKS: usize = 2_048;

/// Players one lock remembers with **full** rights — the ones who entered
/// its main code, plus whoever bolted it on. The reference's list is
/// unbounded (`reference/DOORS.md` §2.2); ours cannot be (wall 4).
/// Overflow policy: **refuse the code entry** with `REFUSE_D_AUTH_FULL`.
/// Never evict: dropping the oldest entry would make a door that forgets
/// the person who owns it, which is the one failure this cap must not
/// have. Proposed default, DECISIONS.md §open (lock v1). **Ten, the
/// crew's own cap** (`HEARTH_CREW_CAP`): a code lock on a hearth is how a
/// crew invites a hand, and at eight one code could fill a ten-strong crew
/// only by rotating codes.
pub const LOCK_AUTH_CAP: usize = 10;

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
pub const CLAIM_INDEX_SLOTS: usize = 2 * MAX_PIECES;
const _: () = assert!(CLAIM_INDEX_SLOTS.is_power_of_two());

/// Players one hearth remembers as its **crew** — who may build, upgrade,
/// repair and deploy inside its claim (`reference/BUILDING.md` §2). Ten is
/// **ours**: it was written as the reference's vanilla cap, and upkeep v2's
/// research found none at tier 1 — only a softcore cap of 4 and a 2026
/// devblog example of "a TC with 12 players". It is a number at all because
/// a base is a group, and an unbounded group is an unbounded array in a
/// `HearthRec` the wire mirrors nothing of (wall 4).
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
/// many recipes a blueprint can gate: the mask is a `u128` in the sim and
/// the save file and two 64-bit words on the wire (`SUB_KNOWN`). Blueprint
/// recipes rank first, so the content bake refuses a blueprint recipe or a
/// research row past this bit rather than let it silently become
/// unlearnable; the table itself may be wider ([`MAX_RECIPES`]).
pub const KNOWN_MASK_BITS: usize = 128;
/// The mask's 64-bit words, as `EV_KNOWN` / `EV_KNOWN_HI` and `SUB_KNOWN`
/// state them.
pub const KNOWN_WORDS: usize = KNOWN_MASK_BITS / 64;
const _: () = assert!(
    KNOWN_MASK_BITS <= MAX_RECIPES && MAX_RESEARCH_ROWS <= MAX_RECIPES,
    "the mask covers the first recipes, and a research row teaches one"
);

/// Skin rows the sim preallocates for (`content/skins.toml`, `skin.rs`).
/// The content bake refuses a catalog past this. Structural rather than a
/// knob: it is the width of `Player::skins`, and the owned set crosses the
/// wire as exactly this many bits.
pub const MAX_SKINS: usize = 256;
/// `u64` words in a [`crate::skin::SkinSet`].
pub const SKIN_WORDS: usize = MAX_SKINS / 64;
const _: () = assert!(MAX_SKINS.is_multiple_of(64), "a SkinSet is whole u64 words");

/// Loot tables the sim preallocates for — one per container archetype
/// (`content/loot.toml` ships 2: barrel and crate). The content bake
/// refuses a set past this. Structural cap like `MAX_DEPLOY_DEFS`, not a
/// knob: the index is code (`loot::LOOT_*`), so a table the sim has no
/// verb for is a bake error rather than a silent extra row.
pub const MAX_LOOT_TABLES: usize = 12;

/// Weighted rows one loot table may carry (the cache uses 30 since it took
/// Rust's tree blades, 2026-10-18). The bake refuses past it. Structural cap
/// like `MAX_RECIPE_INPUTS`, not a knob.
pub const MAX_LOOT_ENTRIES: usize = 32;

/// Guaranteed rows one loot table may carry (loot guaranteed column v0,
/// 2026-09-22) — the rows every open pays whatever the weighted draw did,
/// which is how the reference prices its whole container ladder in scrap
/// (barrel 2, military crate 8, every one at 100 %). The bake refuses past
/// it. Structural cap like [`MAX_LOOT_ENTRIES`], not a knob: it bounds the
/// per-open work (a fixed walk, one `inv_add` per row), and how much a row
/// pays is its own `count_min`/`count_max`. The shipped tables use one.
pub const MAX_LOOT_GUARANTEED: usize = 4;

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
pub const MAX_HAVEN_CANDIDATES: usize = 256;

/// Sim event ring, cleared every tick — the sim's only output channel
/// besides state itself (integer codes, CLAUDE.md wall 3). Overflow
/// policy: **drop newest**, counted; the late-join slot sync (the wire
/// slice) re-derives anything a lost event failed to announce. Proposed
/// default, DECISIONS.md §open (gather bounds row).
///
/// **500 since 2026-10-17**: at 256 the hundred-player firefight
/// (`snapshot_budget.rs`, `the_event_lane_holds_at_population`) peaked at
/// 264, overflowing it and resyncing every client at once. 500 is 1.9× that
/// peak, and stays under the 510 that the stair-rotation overflow test
/// (`server/src/core.rs`) can raise in one tick, so that test still reaches
/// a real overflow.
pub const MAX_EVENTS_PER_TICK: usize = 500;

/// "Dropped at your feet" lines one give-back (a craft cancel, a demolish
/// refund, a pick-up, an unbolt) may announce: one per distinct item, so a
/// cancelled queue of twenty recipes cannot spend the event ring on toasts.
pub const SPILL_TOASTS_MAX: usize = 3;

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
/// Sized off the seats: every one of them holding its whole share
/// (`MAX_CHARGES_PER_OWNER`) at once. The store is shard-wide, and one a
/// handful of accounts planting on an empty field could fill would refuse
/// every other raid on the island — at 64 it took six, since a paced hand
/// keeps about ten lit and a per-owner cap above that never bound. Now no
/// planter under its own share meets a full store. Tens of kB, so boxed
/// (`charge::Charges`); the fuse scan is integer compares, and what one
/// tick sets off is capped apart (`MAX_DETONATIONS_PER_TICK`).
///
/// Overflow policy: **refuse** — the plant is refused with `REFUSE_B_FULL`
/// and the charge stays in the raider's inventory. Refusing costs the
/// raider a click; the alternative, evicting someone else's live charge,
/// would let one player disarm another's raid by planting into a full
/// store, which is a grief verb rather than a cap.
/// Proposed default, DECISIONS.md §open (satchel fuse v0).
pub const MAX_LIVE_CHARGES: usize = MAX_PLAYERS * MAX_CHARGES_PER_OWNER;

/// Live charges one planter may have burning at once: the share the store
/// above is sized from. Twelve is more than a paced hand ever has lit
/// honestly — one plant a second (`server::pace`) against a ten-second
/// fuse is ten — so a real raid never meets it; it is the sim's own bound
/// on a hand nothing paces. Overflow policy: **refuse**, `MAX_LIVE_CHARGES`'
/// own (`REFUSE_B_FULL`, the charge stays in the pack).
pub const MAX_CHARGES_PER_OWNER: usize = 12;
const _: () = assert!(MAX_LIVE_CHARGES >= MAX_PLAYERS * MAX_CHARGES_PER_OWNER);

/// Charges one tick sets off — blown or gone out — at most. With the store
/// sized off the seats, far more than a tick's blast pass was ever sized
/// for can come due together; 64 is the old store's whole size, sixteen
/// stone walls coming down at once. Overflow policy: **defer** — the rest
/// stay where they are stuck, due, and go off on the next tick, in store
/// order; nothing is lost, and a paced island never comes near it.
pub const MAX_DETONATIONS_PER_TICK: usize = 64;

/// Fires burning on the ground at once (`fire.rs`) — what fire arrows leave.
/// A bow looses one every 1.25 s and a fire lasts at most 40 s, so one archer
/// keeps about 32 alight and two fill it. Overflow policy: the **oldest**
/// goes out first, because the newest is the one somebody just shot.
pub const MAX_FIRES: usize = 64;

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

/// Arrows one archer may have in flight at once (`ranged::draw`) — their
/// share of `MAX_ARROWS`, so the store cannot be filled by a handful of
/// accounts lobbing arrows high to keep them airborne for the whole
/// `MAX_ARROW_LIFE_TICKS`, which refused every bow and rocket on the shard.
/// The cadence alone allows seven: a bow nocks every 37 ticks and a lob
/// lives 240. An aimed shot is down inside two seconds, so an archer
/// shooting at anything holds one or two; four is a lob every cadence for
/// over 3.7 s of flight, which is shooting at nothing past ~170 m.
/// Overflow policy: **refuse the shot**, `MAX_ARROWS`' policy and in the
/// same place — before the ammo leaves the quiver.
pub const MAX_ARROWS_PER_OWNER: usize = 4;

/// Weapon attachments the content may declare (`combat::ModDef`): one bit
/// each of a gun's attachment mask (`Player::mods`), so a `u8` holds them.
pub const MAX_MODS: usize = 8;

/// Bullets in flight across the whole shard (`bullet.rs`). A firearm's
/// round flies like Rust's — travel time and drop — so it is a sim entity
/// for the fraction of a second it is in the air. Sized off the per-owner
/// share below: `MAX_PLAYERS × MAX_BULLETS_PER_OWNER` fits, so the store is
/// never the reason a shot is refused while every owner keeps to its share.
/// Overflow policy: **refuse the shot**, `MAX_ARROWS`' policy for its
/// reason — never steal a live round to make room.
pub const MAX_BULLETS: usize = 2048;

/// Bullets one shooter may have in flight at once. A shotgun volley is up to
/// `MAX_PELLETS` and an automatic holds three or four (a rifle round lives
/// ~14 ticks and the cadence is four), so 20 is a full volley plus a burst.
/// `bake_ranged` refuses a gun whose pellets × rounds-in-the-air at full
/// cadence would pass it, so with shipped content this never binds.
/// Overflow policy: **refuse the shot**, before the round is spent.
pub const MAX_BULLETS_PER_OWNER: usize = 20;
const _: () = assert!(MAX_PLAYERS * MAX_BULLETS_PER_OWNER <= MAX_BULLETS);
const _: () = assert!(MAX_PELLETS <= MAX_BULLETS_PER_OWNER);

/// Collision samples one bullet may take in one tick, `ARROW_STEP_MM` apart:
/// 27.2 m a tick, 816 m/s. The arrow's `MAX_ARROW_SUBSTEPS` for a round ten
/// times faster, and a **content wall** the same way — `bake_ammo` refuses a
/// firearm round faster than this rather than ship one that tunnels.
pub const MAX_BULLET_SUBSTEPS: usize = 160;

/// Ticks a bullet may fly before it is dropped, whatever its reach: three
/// seconds. A round's `range_m` ends it long before this; the cap exists so
/// a round fired straight up cannot hold a slot.
pub const MAX_BULLET_LIFE_TICKS: u16 = 90;

/// Stopped arrows not yet lying on the ground (`spent.rs`): the ones in a
/// body, for at most the lodge, and the ones stopped this tick. Four full
/// skies of arrows — nothing reaches this store without leaving
/// `MAX_ARROWS` first — so it can only fill if 512 arrows find bodies
/// inside one lodge.
///
/// Overflow policy: **evict** the arrow due out soonest (smallest
/// `ready_at`), counted by `SpentArrows::evictions` so the policy is
/// measurable (an eviction's only evidence is an absence). Not *refuse*,
/// which is `MAX_ARROWS`' policy: here the arrow has already landed.
pub const MAX_SPENT_ARROWS: usize = 512;

/// Ticks an arrow may stay in flight before it expires, whatever the
/// weapon's own derived life. The backstop that makes `MAX_ARROWS` a
/// bound on *occupancy* rather than a hope: an arrow that somehow misses
/// terrain, occupants, pieces and bodies still leaves the store within
/// eight seconds. No overflow policy — this is a lifetime, not a queue.
///
/// 120 → 240 when arrows took Rust's speeds and drop: eight seconds is
/// their arrow's own hard lifetime, so a 45° lob outlives it here as it
/// does there (and falls, rather than vanishing). Any shot aimed at
/// something lands well inside it — a level shot inside a second.
pub const MAX_ARROW_LIFE_TICKS: u16 = 240;

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
/// shipping a projectile that tunnels. The bow's wooden arrow is 1666
/// mm/tick and the crossbow's 2500, so both sit inside it.
/// Overflow policy: none reachable — the bake refusal is what keeps the
/// clamp from ever binding at tick time.
/// Proposed default, DECISIONS.md §open (ranged v0).
pub const MAX_ARROW_SUBSTEPS: usize = 16;

/// Collision samples one **hitscan** shot may take, over its whole reach
/// (`ranged::hitscan`). A bullet has no flight: the trace that an arrow
/// spreads over forty ticks at `MAX_ARROW_SUBSTEPS` a tick, a firearm pays
/// in one, so the two caps are the same idea against different clocks and
/// neither can be the other.
///
/// With `ARROW_STEP_MM` this is a **content wall**, exactly as
/// `MAX_ARROW_SUBSTEPS` is for a round's muzzle speed: a firearm whose
/// `range_m` exceeds `ARROW_STEP_MM * MAX_HITSCAN_SAMPLES` (306 m) cannot
/// be traced at the spacing a trunk needs, and `bake_combat` refuses it at
/// boot rather than shipping a gun that shoots through cover past some
/// distance nothing writes down. The revolver is 50 m — 295 samples — so it
/// sits inside this with room and nothing else in `content/` is a firearm.
///
/// Overflow policy: **none reachable at tick time** — the bake refusal is
/// what keeps the clamp from ever binding, the same posture
/// `MAX_ARROW_SUBSTEPS` states.
///
/// The per-tick bound this leaves is **derived rather than declared**, which
/// is why there is no separate shots-per-tick cap: one player fires at most
/// one shot a tick (`RangedDef::rate_ticks` gates it and the pass reads one
/// slot once), so the shard-wide worst case is `MAX_PLAYERS *
/// MAX_HITSCAN_SAMPLES` samples and it is already bounded by a cap that
/// exists. A queue would be the wrong shape here anyway: a hitscan shot
/// leaves no entity behind to store, so there is nothing to overflow.
/// Proposed default, DECISIONS.md §open (hitscan v0).
pub const MAX_HITSCAN_SAMPLES: usize = 1800;

/// How far past the shooter a **missed** shot still marks the world, in
/// samples of `ARROW_STEP_MM` — 600 of them, 102 m (guns v1; it was 64,
/// 10.9 m): far enough that a wall a rifle hits across a compound takes the
/// chip and shows the mark. An automatic fires every fourth tick, so a
/// hundred of them at open sky is ~25 full walks a tick, ~9 ms.
///
/// This is a *cosmetic* bound and it is the only number here that admits to
/// being one, so it is worth saying exactly what it buys. `ranged::hitscan`
/// walks the world only as far as the nearest body in the line, because
/// nothing past that body can change who was hit; a shot with nobody in
/// front of it therefore owes no walk at all, and the only reason to take
/// one is `EV_IMPACT` — the puff of dirt where the bullet landed.
///
/// Measured, and this is why the bound exists: the walk is a
/// `terrain::ground` evaluation per sample at ~370 ns, so a hundred players
/// holding the trigger at open sky on one tick is **~20 ms of a 33 ms
/// tick** at the full 320 (`DECISIONS.md` §open, hitscan v0). A decal must
/// not own the tick. At 64 it is ~2.4 ms, and what is lost is the mark on a
/// hillside more than eleven metres away — which is the mark nobody is
/// standing close enough to read.
///
/// It does **not** bound a shot that hits: where the trace was already owed
/// for the hit decision, the mark is free and is emitted wherever the stop
/// fell. So the rule is *the mark is free where the walk was already paid
/// for, and bounded where it would otherwise be the reason for one*.
///
/// Overflow policy: none — a shot past it is a miss with no decal, never a
/// shot that stops early. Nothing about a hit reads this.
/// Proposed default, DECISIONS.md §open (hitscan v0).
pub const MAX_HITSCAN_MARK_SAMPLES: usize = 600;

/// The roster's length: the hard cap on animals one shard simulates, plus
/// the heli's slot (`mob.rs`). Not a queue — fixed slots, one species each,
/// of which each species' density decides how many take part
/// (`mob::targets`: habitat-weighted land × `content/mobs.toml` `per_km2`,
/// the reference's `boar.population`). So this is a ceiling, not a count:
/// the shipped densities on a 4 km island fill about a quarter of it, and a
/// bigger island or a denser row grows into the rest.
///
/// Sized against the wire as well as the tick: a dormant animal costs one
/// comparison a tick, and a client is only ever sent the ones inside its
/// 176 m interest band, which density — not this cap — decides.
pub const MAX_MOBS: usize = 256;
// Slots travel as `u8` (`brain.rs` howls, `mob.rs`), so 256 is the ceiling.
const _: () = assert!(MAX_MOBS <= 256);

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

/// How often a body's wet and cold are stepped, ticks: once a second,
/// phase-offset by slot so a hundred bodies do not all take the heat scan on
/// one tick (weather v0, `exposure.rs`). A cadence, not a queue.
pub const EXPOSURE_PERIOD_TICKS: u64 = 30;

/// One full day/night cycle, in ticks — 80 minutes at `TICK_HZ`
/// (`ALPHA.md` §1's knob; day/night v0, `DECISIONS.md` §open).
///
/// **Time of day is a pure function of the tick, and that is the design,
/// not a shortcut.** The snapshot header already carries the tick on
/// every datagram and the client already tracks a smoothed estimate of
/// it (`client-core/clock.rs`), so shipping a second field would be
/// redundant bytes carrying a derivable number — `NETCODE.md` §3's "time
/// of day rides the G channel" is satisfied by the tick itself plus this
/// constant. The set-time admin verb this used to cost arrived with
/// weather v0 as that one field: `weather::Env::day_offset`, which every
/// reader of the hour adds through `weather::day_tick`.
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

/// The moon's month, in days of [`DAY_TICKS`]: full moon to full moon
/// (`world::moon_phase`). Eight days is under eleven hours of play, so a
/// wipe sees the whole calendar many times over and two nights running
/// never wear the same moon. Only the renderer reads it, so it is a code
/// constant and not a balance number.
pub const MOON_DAYS: u64 = 8;

/// The most bites one tick can land across the whole roster (mob.rs
/// `Bites`). Derived, generously: only a *thinking* animal can bite, so
/// the true per-tick ceiling is `MAX_MOBS / MOB_THINK_TICKS` ≈ 17 plus
/// rounding — 20 is that with headroom, not a tuning knob. **Overflow
/// drops the bite**: a full buffer is one merciful tick, and the phase
/// lock retries the same pair two seconds later. Never a queue — a bite
/// carried across ticks would land on a player who has already left.
pub const MAX_MOB_BITES_PER_TICK: usize = 20;

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

// ---------------------------------------------------------------------------
// Spectators (net lane, 2026-09-22; `NETCODE.md` §2.3). Appended as its own
// block so a parallel lane's appendix merges beside it rather than into it.
// ---------------------------------------------------------------------------

/// Read-only spectator seats per shard: connection slots past
/// `MAX_PLAYERS` that watch a consenting player's view and hold no body.
///
/// **The array size behind every per-seat table** — `ShardCore::clients`
/// grows by this, the slot table and the sim thread's link table with it — so
/// a shard's `spectate_seats` can lower it and never raise it. Priced against
/// the tick: a seat runs a connected client's netcode (interest, drips, one
/// snapshot encode) plus the mirror — ~8 µs per client in `NETCODE.md` §9's
/// 100-client row, a seat ~1.4× that in a debug ablation, so sixteen are
/// ~0.2 ms of 33.3 (`bin/profile --spectators` measures it). Overflow
/// policy: **refuse** at the handshake with `REFUSE_WATCH_FULL`, never evict
/// a seat already watching. Proposed default, `DECISIONS.md` §open.
pub const MAX_SPECTATORS: usize = 16;

// ---------------------------------------------------------------------------
// The trust ledger (`trust.rs`, `PLAYERS.md` wall 3). Ledger lane, 2026-09-22.
// ---------------------------------------------------------------------------

/// Trust rows one applied command may mint. **One, and the borrow checker
/// holds it rather than a test**: a row is pushed by spending a
/// `trust::TrustSeat` by value, and a seat is neither `Clone` nor `Copy`, so
/// a second push in one command is `E0382` at compile time. A verb that
/// genuinely needs two rows changes that type and this number in the same
/// commit. Structural, not a knob.
pub const TRUST_ROWS_PER_COMMAND: usize = 1;

/// The trust ledger's per-tick ring (`World::trust`), cleared at tick start
/// and drained by the shard after every tick (`server/src/trustlog.rs`).
///
/// **Derived from a proven bound rather than picked**, because loss is the
/// thing this ring exists to prevent: rows per tick ≤ seats per tick ≤
/// `MAX_COMMANDS_PER_TICK` × `TRUST_ROWS_PER_COMMAND` (`trust.rs` holds the
/// derivation, `tests/trust_ledger.rs` gates each link). So the ring is
/// exactly as deep as the most rows a tick can make.
///
/// Overflow policy: **none reachable, counted anyway** (`TrustLedger::
/// overflow`, surfaced by the shard as `trust_sim_overflow` and as a gap
/// line in the log itself). Never a silent drop.
///
/// Typed as the number, with the derivation asserted beside it: the knob
/// registry (`ci/knob_registry.mjs`) reads literals and cannot evaluate an
/// expression, which is `AOI_RANK_ENTER`'s lesson above. Cost: 256 × 12 B,
/// one boxed allocation at construction, nothing in the tick (wall 2).
/// Registered in DECISIONS.md §open ("trust ledger v1").
pub const MAX_TRUST_ROWS_PER_TICK: usize = 256;
const _: () = assert!(
    MAX_TRUST_ROWS_PER_TICK == MAX_COMMANDS_PER_TICK * TRUST_ROWS_PER_COMMAND,
    "MAX_TRUST_ROWS_PER_TICK must stay the command ceiling times the rows one \
     command may mint. One of them moved and the ring stopped being a proof: \
     a tick can now make more trust rows than it has room to keep"
);
/// Side of one path search's window, in 1 m nav cells (`nav.rs`). 96 m
/// holds a wolf's 30 m notice radius on either side of it with room to go
/// round a base; a goal further off is walked toward and re-planned.
pub const NAV_WIN: usize = 96;

/// Most cells one search may expand before it settles for the nearest
/// cell it reached — the partial path that tells a brain "unreachable".
pub const NAV_MAX_EXPAND: u32 = 768;

/// Cells the whole roster may expand in one tick — the count-based stand-in
/// for the reference's `ai.framebudgetms`. **Overflow defers**: a request
/// that cannot get `nav::MIN_EXPAND` of it keeps its old path and asks again
/// on its next think. Two full searches' worth; the straight-line check
/// every plan tries first costs none of it.
pub const NAV_EXPAND_PER_TICK: u32 = 2 * NAV_MAX_EXPAND;

/// The A* frontier. Each expansion pushes at most eight cells, so this
/// cannot fill inside `NAV_MAX_EXPAND`; if it ever did, the newest push is
/// dropped and the search degrades to a longer route, never a panic.
pub const NAV_HEAP_CAP: usize = 8 * NAV_MAX_EXPAND as usize + 8;

/// Turn points one path carries. A route with more is cut short and the
/// animal re-plans from where the cut lands.
pub const NAV_MAX_CORNERS: usize = 16;

/// Recent noises the animals can hear (`noise.rs`). A ring: **overflow
/// forgets the oldest**, which every animal in range has already heard or
/// never could — a noise is only audible for one think window. Every shot
/// AND every strike lands here (each gather swing and each round's impact
/// is an `EV_IMPACT`), and at 32 a busy server's hatchets pushed a gunshot
/// out before the animals near it had thought: 256 is a full server's
/// shots and strikes over one window with room over.
pub const MAX_NOISES: usize = 256;

/// Pack calls one tick can raise across the roster (`mob::Howls`). Only a
/// thinking pack animal howls, and each at most once per its cooldown, so
/// the true ceiling is the handful thinking on the tick. **Overflow drops
/// the howl**: the call still happened (the pack answers off the roster, not
/// the sound), only its sound is lost.
pub const MAX_HOWLS_PER_TICK: usize = 20;
