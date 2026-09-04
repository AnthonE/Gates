//! `test_loot_storm` — the withdrawal side, at population.
//!
//! `tests/combat_storm.rs` names what it left out, in its own words:
//! *"Nobody loots. `Command::Loot` empties the nearest bag in reach, and a
//! hundred bodies each emptying one bag a tick would hold the store near
//! zero — the deposit side is what has never been driven at population,
//! and the two do not fit in one fixture."* This is that other half, and a
//! **sibling rather than a retrofit** for the reason that file's own
//! header gives for being one: the combat storm holds an equality
//! assertion about saturation (`peak_bags == MAX_BACKPACKS`) that was
//! sized for a store nobody empties, and turning looting on inside it
//! would have quietly desaturated it. `NOW.md` §0cs item 1.
//!
//! ## Why the withdrawal side needed its own saturation argument
//!
//! The deposit side saturates the **bag store**: bodies die faster than
//! bags despawn, the store pins at `MAX_BACKPACKS`, and the overflow
//! policy is eviction. Withdrawal cannot press that wall at all — it only
//! ever removes — so a copy of the sibling's argument would be a gate that
//! cannot go red.
//!
//! What withdrawal presses is the **event ring**, and it is the most
//! event-expensive verb a client can send: one `Command::Loot` is up to
//! `INV_SLOTS` `EV_GATHER`s plus an `EV_BAG_REMOVED`, against a swing's
//! one. A hundred of them on a tick, over a full store, is that ring's
//! worst case anywhere in this game.
//!
//! **The claim is about the transient, not an aggregate**, and that is a
//! correction the measurement forced: over the whole run the burst ticks
//! overflow only a little more often than the fill ticks (8 against 6),
//! because once the store is empty a `Loot` finds nothing and costs
//! nothing. Averaged, the verb looks ordinary. The real number is the
//! **worst tick: 291 events refused against a 256-slot ring** — one tick
//! throwing away more than the ring can hold.
//!
//! ## What it is
//!
//! The combat storm's island, marks, duels, seating and profile —
//! deliberately, so a seed whose generator stopped offering flat cells
//! fails both files with one cause. Two schedules run over it:
//!
//! * **Fill.** The duel storm exactly as the sibling runs it: two
//!   `brawl_step` commands per body per tick, nobody looting. Not a second
//!   copy of that gate — this file's *precondition*. Measured, an empty
//!   store reaches `MAX_BACKPACKS` at **tick 172**; `FILL_TICKS` is 250.
//! * **Bursts.** Every `LOOT_PERIOD` ticks thereafter, `LOOT_BURST` ticks
//!   in which every body still fights *and* also reaches for the ground:
//!   one `brawl_step` and one `Command::Loot` each per tick, so the store
//!   is driven from both ends at once by a hundred bodies.
//!
//! **Periodic rather than one long scavenge phase**, because the first
//! draft was one phase and the measurement said why that is weaker: the
//! store drained to zero in the opening ticks and never came back — peak
//! 104 of 256 for the remaining 350 ticks — so the interesting event
//! happened exactly once and everything after it was a hundred bodies
//! looting an almost empty island. Four bursts make the transient recur,
//! and they buy a second claim for free: the store has to climb from zero
//! back to its cap between them.
//!
//! **There is no `bots::loot_step`, on purpose.** `Command::Loot` carries
//! only the sender's id — no target, no aim, no reach (`world.rs`: *"No
//! target crosses: the pick is the sim's"*) — so a shipped loot profile
//! would be a function returning a constant, and the policy this file
//! gates lives entirely in `backpack::loot_nearest`. The fight still runs
//! the shipped `bots::brawl_step`.
//!
//! ## The one content difference from the sibling, and why
//!
//! `combat_storm.rs` leaves `GatherContent::EMPTY` in place so that
//! `gather::swing` returns `Swing::Free` on every path and the arm reaches
//! `combat::strike` — a duel beside a harvestable tree would have had its
//! swing eaten by the tree. But `loot_nearest` skips any item whose
//! `stack_max_of` is 0, and under `EMPTY` that is *every* item, so a
//! withdrawal storm on `EMPTY` would take nothing and empty no bag: the
//! gate would pass while asserting nothing.
//!
//! So this file arms the **stack ladder only** — `probe_fixture`'s table
//! with every node put back to `NodeDef::INERT`. Nothing is invented: the
//! ceilings, the condition rows and the light rate are the ones every
//! other wall runs, and `swing`'s inert-content guard (`def.output ==
//! NO_ITEM` → `Swing::Free`) is exactly the branch `EMPTY` takes, so the
//! fight is the sibling's fight while the ladder can size a stack.
//!
//! ## What it asserts, and why each one can go red
//!
//! 1. **Every store stays inside its cap, every tick** — plus one the
//!    sibling cannot make: **no stack a withdrawal created is over its own
//!    ladder ceiling, and no item the ladder cannot size ever reaches an
//!    inventory.** That is the bounded-everything statement this path
//!    owes, because `loot_nearest` is the one verb that moves an arbitrary
//!    stack into a player without the player naming a slot.
//! 2. **Every burst began against a full store** (4 of 4), earned through
//!    `World::die` rather than by standing bags up — which also gates the
//!    store's *recovery* from empty back to its cap.
//! 3. **A hundred bodies actually withdrew**: 856 `BAG_GONE_EMPTIED`
//!    announcements and 2,729 `EV_GATHER`s, which under an inert node
//!    table can only have come out of bags.
//! 4. **Every burst took a full store to zero**, with the slowest drain
//!    finishing on tick 5 of 8 — margin, not a fit, and gated as such.
//! 5. **The ring's worst tick refuses more than the ring holds** (291 of
//!    256), and every burst reaches it.
//! 6. **Drop-newest, with its consequence made observable.** At least
//!    `bursts_to_zero * MAX_BACKPACKS` = 1,024 bags left the store — ground
//!    truth off `Backpacks::len` — while only 856 removals were announced.
//!    The 168 missing are events no client will ever hear, which is what
//!    the overflow policy promises and what
//!    `event_ring_overflow_heals_by_resync` exists to recover from. No
//!    other gate in the tree is positioned to measure that gap.
//! 7. **It is deterministic** (wall 5) and **survives a save/load round
//!    trip byte for byte**, after a hundred bodies have been swap-removing
//!    from the store for four full drains.
//!
//! ## The lesson, which is bigger than this file
//!
//! **An event count taken while the ring is overflowing is an undercount
//! of the thing it names.** During a burst the ring is saturated by
//! `EV_GATHER`, so `EV_BAG_DROPPED` is itself dropped: **6** bags
//! announced across the 32 burst ticks, where the run's own death rate
//! (1,700 over 1,050 ticks) implies roughly fifty. The first draft of this
//! file asserted "the fight kept feeding the store" off that counter — a
//! number that is a property of the ring's saturation, read as a property
//! of the fight. Ground truth inside a burst has to come off the store.
//!
//! ## The mutants
//!
//! Eight run, six killed, and the two survivors are named because a
//! survivor you cannot explain is a hole (`CLAUDE.md`: *after writing a
//! gate for an optimization, run the mutant*).
//!
//! Killed: `loot_nearest` taking nothing; an emptied bag never removed;
//! `Command::Loot`'s dispatch arm made a no-op; the eviction announcing
//! `BAG_GONE_DESPAWN`; and **both** of `inv_add`'s stack ceilings —
//! partial-slot and fresh-slot — dropped to `left`.
//!
//! The fresh-slot one is why `KEEPSAKE_COUNT` is 250, and it is the
//! finding worth carrying out of this pass: with the keepsake at 10 that
//! mutant **survived**, because nothing this fixture ever handed `inv_add`
//! was over the ceiling, so deleting the ceiling was an equivalent
//! mutation. The assertion read as coverage and was arithmetic about
//! numbers that could not fail it.
//!
//! Survived, both explained:
//! * **`loot_nearest`'s reach check deleted.** An unbounded reach makes
//!   looting strictly *more* effective, so every assertion here still
//!   passes. Not a hole: `tests/backpack.rs` gates the boundary exactly,
//!   at `LOOT_REACH_M ± 0.5`, which is the right shape for a radius and
//!   the wrong shape for a storm.
//! * **`loot_nearest`'s `cap == 0` skip deleted.** Equivalent by
//!   construction — `inv_add` opens with its own `if stack_max == 0 {
//!   return 0; }`, and the call site already skips a zero `took`, so no
//!   test anywhere can distinguish the two. The guard is defence in depth
//!   and reads as duplication because it is.
//!
//! ## What it deliberately does NOT claim
//!
//! **Not conservation.** `restock` overwrites slots 0–2 every
//! `RESTOCK_TICKS` — a body that respawned naked drops no bag, and the
//! storm's subject is the bag — so the fixture destroys some of what the
//! scavengers picked up. That is a fixture act, stated here rather than
//! hidden, and it is why nothing below counts items in against items out.
//! It also means the looters never fill up, so the drain is the store's
//! behaviour and not the looters' bookkeeping.
//!
//! **Not an aggregate claim about the ring**, for the measured reason
//! above: burst ticks overflow 8 times and fill ticks 6, and a gate on
//! that difference would be a gate on the phase lengths.
//!
//! **Nobody moves**, exactly as the sibling says: `move_x`/`move_z` are
//! zero, so the rewind depth each command carries is a lookup and not a
//! correction. That is `NOW.md` §0cs item 2 and stays there.

#![allow(clippy::disallowed_macros)]

use sim_core::backpack::{BackpackContent, BAG_GONE_EMPTIED, BAG_GONE_EVICTED};
use sim_core::bots::{brawl_step, BrawlPlan};
use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, NodeDef, GATHERABLE_KINDS};
use sim_core::limits::{
    INV_SLOTS, MAX_BACKPACKS, MAX_COMMANDS_PER_TICK, MAX_EVENTS_PER_TICK, MAX_PLAYERS,
};
use sim_core::movement::Body;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World, EV_BAG_DROPPED, EV_BAG_REMOVED, EV_GATHER};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;
use sim_core::yaw_dir;

/// The solved authored sites for `seed`, memoized per seed for
/// `combat_storm.rs`'s stated reason: `terrain::haven` is a few thousand
/// `height` taps and these suites call it from inside assertion loops.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3) and that list is
    // crate-scoped, so it binds this suite too.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// The combat storm's island, deliberately — see the header.
const SEED: u64 = 0x5701_4D21;

/// Every seat in the world, taken. The ring is what a crowd overflows and
/// this file's whole subject is how hard one verb can press it, so it runs
/// the same hundred the sibling does.
const DUELS: usize = MAX_PLAYERS / 2;
const PLAYERS: usize = DUELS * 2;
const _: () = assert!(PLAYERS == MAX_PLAYERS);

/// Long enough to fill the 256-bag store through `World::die` and evict
/// from it. Measured (`cap_reached_tick`): an empty world reaches the cap
/// on **tick 172**, so 250 is margin rather than a fit — a fill phase that
/// only just made it would turn a content change into a mystery.
const FILL_TICKS: u64 = 250;

/// How long one burst of looting lasts. Measured (`slowest_drain`): the
/// slowest of the four drains a full store on its **5th** tick, and
/// `test_loot_storm` asserts that margin rather than trusting it — at 5
/// this was a fit, and one burst of four finished a tick short.
const LOOT_BURST: u64 = 8;
/// Burst to burst. Leaves 192 quiet ticks for the fight to stand the store
/// back up, against the 172 an empty world needed from a standing start —
/// and unlike that one this is not asserted by arithmetic but by
/// `bursts_from_full`, which is the honest way round: the refill happens
/// under a different world (bodies mid-duel, bags mid-despawn) than the
/// first fill did, so a number carried over from it would be a guess.
const LOOT_PERIOD: u64 = 200;
/// The whole storm — `FILL_TICKS` then `BURSTS` periods.
const BURSTS: usize = 4;
const TICKS: u64 = FILL_TICKS + LOOT_PERIOD * BURSTS as u64;

/// Commands per body per tick, in each phase. Two either way, so the tick
/// never carries more than `PLAYERS * 2` and the ceiling is the sibling's:
/// the fill phase spends both on `brawl_step`, and the scavenge phase
/// spends one on the fight and one on the ground.
const STEPS_PER_TICK: usize = 2;
const _: () = assert!(PLAYERS * STEPS_PER_TICK <= MAX_COMMANDS_PER_TICK);

/// How far apart the two halves of a duel stand — the sibling's, and for
/// its reasons (inside melee reach, outside `combat::strike`'s point-blank
/// exemption). It is also comfortably inside `backpack::LOOT_REACH_M`, so
/// both halves of a duel can reach the bag either of them leaves.
const SEPARATION_M: f32 = 1.2;

/// Cells between duels — 7 cells is 21 m against the firearm's 20 m
/// fixture reach, so no duellist shoots into the next duel.
const DUEL_SPACING: i32 = 7;

/// How often the fixture tops the kit back up. A body that respawned with
/// nothing drops no bag, and the storm's subject is the bag.
const RESTOCK_TICKS: u64 = 5;

/// `CombatContent::probe_fixture`'s item 0: 34 damage, 2 m reach.
const BLADE: u16 = 0;
/// Filler with a long bag life — items under 4 get a 360-tick despawn
/// from `BackpackContent::probe_fixture`, and a bag lives as long as its
/// longest-lived item.
const KEEPSAKE: u16 = 2;
/// How much of it a body carries, and it is deliberately **over the
/// ladder's 100-per-slot ceiling**.
///
/// Not filler and not a round number picked for looks. Without it the
/// per-tick ladder-ceiling assertion is **vacuous**, and that is measured
/// rather than argued: with the keepsake at 10, mutating `inv_add`'s
/// fresh-slot `stack_max.min(left)` to `left` — deleting the ceiling
/// outright — SURVIVED the whole suite, because no stack this fixture
/// ever handed it was over the ceiling to begin with, so the mutant was
/// equivalent. It is `CLAUDE.md`'s limb-band lesson exactly: *shipped
/// content agrees with a band by construction*.
///
/// 250 against a ceiling of 100 also buys the path it names — `inv_add`'s
/// two-loop split, which fills partial stacks and then spreads the
/// remainder over fresh slots. A single call still moves all 250 (the
/// fresh-slot loop walks every slot), so the drain is not slowed.
const KEEPSAKE_COUNT: u16 = 250;
/// The fixture's hitscan firearm and its round.
const GUN: u16 = 6;
const ROUND: u16 = 7;

/// The hotbar slot every duellist holds its weapon in.
const WEAPON_SLOT: u8 = 0;

/// Slots `restock` writes by hand: the weapon, its feed, and the
/// over-stacked keepsake. **Everything above this can only have arrived
/// through `inv_add`** — every node is inert so nothing is gathered, no
/// command in this fixture crafts or picks up an arrow, and a respawn
/// clears — so slots `KIT_SLOTS..` are the withdrawal's work and nobody
/// else's. That is what makes the ceiling assertion below a statement
/// about `loot_nearest` rather than about the fixture's own kit, which it
/// deliberately over-stacks.
const KIT_SLOTS: usize = 3;

/// `probe_fixture`'s ladder with every node put back to `NodeDef::INERT`.
///
/// The header has the whole argument. In one line: `loot_nearest` cannot
/// take an item the ladder cannot size, and `swing` cannot be eaten by a
/// node whose `output` is `NO_ITEM` — so this table is the only one on
/// which both halves of this storm are reachable at once.
fn ladder_only() -> GatherContent {
    let mut g = GatherContent::probe_fixture();
    g.nodes = [NodeDef::INERT; GATHERABLE_KINDS];
    g
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `DUELS` buildable cells, `DUEL_SPACING` apart, in ring order from the
/// middle of the map — `combat_storm.rs`'s scan, so the two files stand
/// their duels on the same ground.
fn marks(seed: u64) -> [(u16, u16); DUELS] {
    let mut out = [(0u16, 0u16); DUELS];
    let mut n = 0usize;
    for r in 0..128i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                if n == DUELS {
                    return out;
                }
                if dx % DUEL_SPACING != 0 || dz % DUEL_SPACING != 0 {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if !foundation_terrain_ok(seed, hv(seed), x, z) {
                    continue;
                }
                if out[..n].iter().any(|&(ox, oz)| ox == cx && oz == cz) {
                    continue;
                }
                out[n] = (cx, cz);
                n += 1;
            }
        }
    }
    assert_eq!(
        n, DUELS,
        "the generator no longer offers {DUELS} duel marks"
    );
    out
}

/// Slot 0 is the weapon, slot 1 feeds it, slot 2 is the over-stacked
/// keepsake that keeps the bag on the ground.
///
/// **The whole inventory is cleared first**, which is a fixture act and
/// the reason this file makes no conservation claim (header). It is also
/// load-bearing rather than tidy: a scavenger accumulates ~2.5 slots per
/// bag out of `KEEPSAKE_COUNT` alone, so without the clear every looter
/// is full of keepsakes by the second burst and the store stops draining
/// — the drain would then be measuring the looters' bookkeeping instead
/// of the store, which is exactly what the header promises it is not.
fn restock(inv: &mut [ItemStack], shooter: bool) {
    for s in inv.iter_mut() {
        *s = ItemStack::default();
    }
    inv[0] = ItemStack {
        item: if shooter { GUN } else { BLADE },
        count: 1,
        cond: 0,
    };
    inv[1] = ItemStack {
        item: if shooter { ROUND } else { BLADE },
        count: 100,
        cond: 0,
    };
    inv[2] = ItemStack {
        item: KEEPSAKE,
        count: KEEPSAKE_COUNT,
        cond: 0,
    };
}

/// Where duellist `i` stands and which way it looks — the sibling's, so
/// the two storms seat identically.
fn post(marks: &[(u16, u16); DUELS], i: usize) -> (f32, f32, u16) {
    let duel = i / 2;
    let (mx, mz) = cell_center(marks[duel].0, marks[duel].1);
    let yaw = (duel as u16).wrapping_mul((u16::MAX / DUELS as u16).wrapping_add(1));
    if i.is_multiple_of(2) {
        (mx, mz, yaw)
    } else {
        let (fx, fz) = yaw_dir(yaw);
        (
            mx + fx * SEPARATION_M,
            mz + fz * SEPARATION_M,
            yaw.wrapping_add(1 << 15),
        )
    }
}

/// Duel `d` is a gunfight when it is odd — 25 of each, so neither weapon
/// can be the only reason a bag was left or emptied.
fn is_shooter(i: usize) -> bool {
    (i / 2) % 2 == 1
}

/// What one storm saw. Every field is a *measurement*; the assertions live
/// in the tests so a failure names which invariant broke.
struct Storm {
    hash: u64,
    save: Vec<u8>,
    /// Bursts that began against a **full** store — the precondition, and
    /// it has to hold for every burst, not once.
    bursts_from_full: usize,
    /// Bursts run at all.
    bursts: usize,
    /// The store's high-water mark over the whole storm.
    peak_bags: usize,
    /// The fewest bags standing on any burst tick — how far the
    /// withdrawal drove a full store down.
    min_bags_scavenging: usize,
    /// Bursts that began full **and** reached zero. Ground truth, read off
    /// `Backpacks::len` rather than off the ring, which is the point: each
    /// one is at least `MAX_BACKPACKS` bags that left the store.
    bursts_to_zero: usize,
    /// The slowest burst's time-to-empty, in ticks — how much of
    /// `LOOT_BURST` the job actually needs. Sizing evidence, not a claim.
    slowest_drain: u64,
    /// The tick the store first reached its cap, from an empty world —
    /// what `FILL_TICKS` and `LOOT_PERIOD` both have to clear.
    cap_reached_tick: u64,
    /// Bags standing at the end.
    final_bags: usize,
    /// The most events one tick refused. The ring's worst case, and the
    /// number the whole saturation argument rests on.
    max_dropped_tick: u32,
    /// Burst ticks on which the ring refused an event.
    overflow_burst: usize,
    /// Bags stood up and bags emptied while looting — the conveyor.
    dropped_scavenge: u32,
    emptied_scavenge: u32,
    peak_events: usize,
    /// Ticks on which the event ring refused an event, per phase.
    overflow_fill: usize,
    overflow_scavenge: usize,
    /// `EV_BAG_REMOVED` with `BAG_GONE_EMPTIED` — a withdrawal that took
    /// the last stack. Only `loot_nearest` and the per-slot `Move` verb
    /// announce it, and this fixture sends no `Move`.
    emptied: u32,
    /// `EV_BAG_REMOVED` with `BAG_GONE_EVICTED` — the deposit side's
    /// overflow policy, which must still fire while the fill phase runs.
    evicted: u32,
    /// `EV_GATHER` announced. Under an inert node table a swing pays
    /// nothing, so every one of these came out of a bag.
    gathers: u32,
    deaths: u32,
    /// The fewest bodies on their feet on any one tick.
    min_standing: usize,
}

fn storm() -> Storm {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    // The one difference from the sibling, argued in the header.
    w.gather = ladder_only();

    let cells = marks(SEED);
    let (sx, sz) = cell_center(cells[0].0, cells[0].1);
    w.dev_spawn = Some((sx, sz));

    let mut plans = Vec::with_capacity(PLAYERS);
    for i in 0..PLAYERS {
        let id = (i as u32) + 1;
        w.tick(&[Command::Join { id }]);
        let (_, _, yaw) = post(&cells, i);
        plans.push(BrawlPlan::new(id, WEAPON_SLOT, yaw, is_shooter(i)));
    }
    assert_eq!(
        w.players.iter().filter(|p| p.active).count(),
        PLAYERS,
        "every duellist seated"
    );
    for (i, plan) in plans.iter().enumerate() {
        assert_eq!(
            w.players.iter().position(|p| p.active && p.id == plan.id),
            Some(i),
            "join order is slot order"
        );
    }
    let seat = |w: &mut World, i: usize| {
        let (x, z, _) = post(&cells, i);
        w.players[i].body = Body::at(SEED, hv(SEED), x, z);
    };
    for i in 0..PLAYERS {
        seat(&mut w, i);
    }

    // This file's own stream. Distinct from the sibling's so that two
    // storms on one island are not one storm run twice, and distinct from
    // `bot_frame`'s for the reason `bots.rs` states.
    let mut rng = Pcg32::new(SEED ^ 0x100D_57A2, 13);
    let mut s = Storm {
        hash: 0,
        save: Vec::new(),
        bursts_from_full: 0,
        bursts: 0,
        peak_bags: 0,
        min_bags_scavenging: usize::MAX,
        bursts_to_zero: 0,
        slowest_drain: 0,
        cap_reached_tick: 0,
        final_bags: 0,
        max_dropped_tick: 0,
        overflow_burst: 0,
        dropped_scavenge: 0,
        emptied_scavenge: 0,
        peak_events: 0,
        overflow_fill: 0,
        overflow_scavenge: 0,
        emptied: 0,
        evicted: 0,
        gathers: 0,
        deaths: 0,
        min_standing: PLAYERS,
    };
    let mut cmds: Vec<Command> = Vec::with_capacity(PLAYERS * STEPS_PER_TICK);
    let mut was_down = [false; PLAYERS];
    // Per-burst ground truth, read off the store and never off the ring.
    let mut burst_full = false;
    let mut burst_zero = false;

    for t in 0..TICKS {
        // The burst schedule: quiet until the store has filled, then
        // `LOOT_BURST` ticks of a hundred bodies looting every
        // `LOOT_PERIOD`. Periodic rather than one long phase because the
        // interesting event is the *transient* — a full store meeting a
        // hundred looters — and one sample of it is a sample, not a gate.
        let since = t.wrapping_sub(FILL_TICKS);
        let scavenging = t >= FILL_TICKS && since % LOOT_PERIOD < LOOT_BURST;
        if t >= FILL_TICKS && since.is_multiple_of(LOOT_PERIOD) {
            s.bursts += 1;
            burst_full = w.backpacks.len() == MAX_BACKPACKS;
            burst_zero = false;
            if burst_full {
                s.bursts_from_full += 1;
            }
        }
        if t.is_multiple_of(RESTOCK_TICKS) {
            for i in 0..PLAYERS {
                if !w.players[i].dead {
                    restock(&mut w.players[i].inv, is_shooter(i));
                }
            }
        }
        // Read once, before the tick: a client sends the frames it has,
        // against the state it last saw.
        for (i, down) in was_down.iter_mut().enumerate() {
            *down = w.players[i].dead;
        }
        cmds.clear();
        for (i, plan) in plans.iter_mut().enumerate() {
            cmds.push(brawl_step(plan, &mut rng, was_down[i]));
        }
        for (i, plan) in plans.iter_mut().enumerate() {
            if scavenging {
                // The withdrawal driver step. No target and no reach: the
                // pick is `loot_nearest`'s, which is the verb's design.
                // A body on the ground sends it too — `live_slot_of`
                // makes a corpse's loot a silent no-op, which is a real
                // client's behaviour and not a case to route around.
                cmds.push(Command::Loot { id: plan.id });
            } else {
                cmds.push(brawl_step(plan, &mut rng, was_down[i]));
            }
        }
        assert!(
            cmds.len() <= MAX_COMMANDS_PER_TICK,
            "the storm must not out-run the tick's own command cap"
        );

        w.tick(&cmds);

        // ---- the invariant, every tick ----
        assert!(
            w.backpacks.len() <= MAX_BACKPACKS,
            "tick {t}: {} bags past cap",
            w.backpacks.len()
        );
        assert!(
            w.events.len() <= MAX_EVENTS_PER_TICK,
            "tick {t}: event ring past cap"
        );
        assert_eq!(
            w.players.iter().filter(|p| p.active).count(),
            PLAYERS,
            "tick {t}: a death is a respawn, never a disconnect"
        );
        // The withdrawal side's own bound, and the one the sibling cannot
        // make: `loot_nearest` is the only verb that moves an arbitrary
        // stack into a player without the player naming a slot, so a cap
        // it stopped reading would show up here and nowhere else.
        for p in w.players.iter().take(PLAYERS) {
            for (sl, st) in p.inv.iter().enumerate().take(INV_SLOTS).skip(KIT_SLOTS) {
                if st.count == 0 {
                    continue;
                }
                let cap = w.gather.stack_max_of(st.item);
                assert!(
                    cap > 0,
                    "tick {t}: player {} slot {sl} holds item {} the ladder cannot size",
                    p.id,
                    st.item
                );
                assert!(
                    st.count <= cap,
                    "tick {t}: player {} slot {sl} holds {} of item {} over its ceiling {cap}",
                    p.id,
                    st.count,
                    st.item
                );
            }
        }

        // ---- what the storm actually reached ----
        if s.peak_bags < MAX_BACKPACKS && w.backpacks.len() == MAX_BACKPACKS {
            s.cap_reached_tick = t;
        }
        s.peak_bags = s.peak_bags.max(w.backpacks.len());
        s.peak_events = s.peak_events.max(w.events.len());
        if w.events.dropped > 0 {
            if scavenging {
                s.overflow_scavenge += 1;
            } else {
                s.overflow_fill += 1;
            }
        }
        s.max_dropped_tick = s.max_dropped_tick.max(w.events.dropped);
        if scavenging {
            s.min_bags_scavenging = s.min_bags_scavenging.min(w.backpacks.len());
            if w.events.dropped > 0 {
                s.overflow_burst += 1;
            }
            if w.backpacks.is_empty() && burst_full && !burst_zero {
                burst_zero = true;
                s.bursts_to_zero += 1;
                s.slowest_drain = s.slowest_drain.max(since % LOOT_PERIOD + 1);
            }
        }
        s.min_standing = s
            .min_standing
            .min(w.players.iter().take(PLAYERS).filter(|p| !p.dead).count());
        for e in w.events.entries() {
            match e.code {
                EV_GATHER => s.gathers += 1,
                EV_BAG_DROPPED if scavenging => s.dropped_scavenge += 1,
                EV_BAG_REMOVED if e.b == BAG_GONE_EMPTIED => {
                    s.emptied += 1;
                    if scavenging {
                        s.emptied_scavenge += 1;
                    }
                }
                EV_BAG_REMOVED if e.b == BAG_GONE_EVICTED => s.evicted += 1,
                _ => {}
            }
        }

        // A body the ring answered for is put back on its mark, exactly as
        // the sibling does it: `wake` puts you on a beach, and a storm
        // whose survivors walk away is not a storm.
        for (i, &down) in was_down.iter().enumerate() {
            if down && !w.players[i].dead {
                seat(&mut w, i);
            }
        }
    }

    s.final_bags = w.backpacks.len();
    s.deaths = w
        .players
        .iter()
        .take(PLAYERS)
        .map(|p| p.deaths as u32)
        .sum();

    // Quiet the world before the hash is taken: a world save puts every
    // body to bed on load, so a comparison against a world with a hundred
    // people still driving would fail on the sleeping bit.
    for plan in plans.iter() {
        w.tick(&[Command::Leave { id: plan.id }]);
    }
    s.hash = w.state_hash();
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("the storm's world must save");
    buf.truncate(n);
    s.save = buf;
    s
}

/// **Wall 4, on the verb that only removes.** A store filled to its cap
/// through `World::die` is driven back down through it by a hundred bodies
/// looting, the ring saturates on the withdrawal rather than on a kill
/// wave, and nothing a withdrawal moved is over its own ladder ceiling.
#[test]
fn test_loot_storm() {
    let s = storm();
    println!(
        "loot storm: bursts={} bursts_from_full={} bursts_to_zero={} slowest_drain={} cap_reached_tick={} peak_bags={} \
         min_bags_scavenging={} final_bags={} max_dropped_tick={} overflow_burst={} \
         dropped_scavenge={} emptied_scavenge={} \
         peak_events={} overflow_fill={} overflow_scavenge={} emptied={} evicted={} \
         gathers={} deaths={} min_standing={} save_bytes={}",
        s.bursts,
        s.bursts_from_full,
        s.bursts_to_zero,
        s.slowest_drain,
        s.cap_reached_tick,
        s.peak_bags,
        s.min_bags_scavenging,
        s.final_bags,
        s.max_dropped_tick,
        s.overflow_burst,
        s.dropped_scavenge,
        s.emptied_scavenge,
        s.peak_events,
        s.overflow_fill,
        s.overflow_scavenge,
        s.emptied,
        s.evicted,
        s.gathers,
        s.deaths,
        s.min_standing,
        s.save.len()
    );

    // 2 — the precondition, and it has to hold for EVERY burst. Equality
    // rather than `<=`: a withdrawal against a half-empty store says
    // nothing about a full one, and this is the clause the file rests on.
    // Asserting it per burst also gates the *recovery* — the store has to
    // climb from zero back to its cap between bursts, which is
    // `Backpacks`'s swap-remove having left the free list intact.
    assert_eq!(
        s.bursts, BURSTS,
        "the burst schedule did not run — the storm looted {} times, not {BURSTS}",
        s.bursts
    );
    assert_eq!(
        s.bursts_from_full, BURSTS,
        "only {} of {BURSTS} bursts began against a full store; a withdrawal from a \
         part-filled store tests the wrong thing, and a store that could not climb \
         back to its cap after being emptied is the more interesting failure",
        s.bursts_from_full
    );
    assert!(
        s.evicted > 0,
        "the fill phase never evicted, so the store was never actually pressed"
    );
    assert_eq!(
        s.peak_bags, MAX_BACKPACKS,
        "the store never reached its cap"
    );

    // 3 — a hundred bodies actually withdrew. `EV_GATHER` can only have
    // come from a bag here: every node is inert, so a swing pays nothing.
    assert!(
        s.emptied as usize > MAX_BACKPACKS,
        "only {} bags were emptied — fewer than the store holds, so the withdrawal \
         never got past what one fill had already stood up",
        s.emptied
    );
    assert!(
        s.gathers > s.emptied,
        "{} gather events against {} emptied bags: a bag that emptied must have paid \
         at least one stack, so these cannot be equal unless the payout stopped",
        s.gathers,
        s.emptied
    );

    // 4 — a full store, emptied. Not "reduced": to zero, in EVERY burst,
    // which is the strongest form of the claim and the one the measurement
    // supports. A withdrawal that silently took nothing cannot satisfy it,
    // and the deposit-only sibling has no way to make it at all.
    assert_eq!(
        s.bursts_to_zero, BURSTS,
        "only {} of {BURSTS} bursts took a full store to zero (min held: {})",
        s.bursts_to_zero, s.min_bags_scavenging
    );
    // And the drain finishes strictly inside the burst, so `LOOT_BURST` is
    // margin rather than a fit. Gated rather than measured-and-forgotten:
    // a fixture sized exactly to the job turns the next content change
    // into a mystery, which is how `LOOT_BURST = 5` read before this line
    // existed — it left one burst of four short by a tick.
    assert!(
        s.slowest_drain < LOOT_BURST,
        "the slowest burst needed all {} of its {LOOT_BURST} ticks to empty the store",
        s.slowest_drain
    );

    // 5 — the ring, and the whole reason this side needed its own
    // saturation argument. The claim is about the TRANSIENT, not about an
    // aggregate: a full store meeting a hundred looters refuses more
    // events on one tick than the ring can hold, because one `Loot` is up
    // to `INV_SLOTS` `EV_GATHER`s plus a removal against a swing's one.
    // The aggregate is deliberately not asserted — see the header.
    assert_eq!(
        s.peak_events, MAX_EVENTS_PER_TICK,
        "an overflowing ring must be exactly full"
    );
    assert!(
        s.max_dropped_tick as usize > MAX_EVENTS_PER_TICK,
        "the worst tick refused {} events against a {MAX_EVENTS_PER_TICK}-slot ring — \
         withdrawal stopped being the most event-expensive verb in the game",
        s.max_dropped_tick
    );
    // And every burst reached the ring, not just the first. A schedule
    // where only the opening transient overflowed would mean the store
    // never refilled and bursts 2..n were looting an empty island.
    assert!(
        s.overflow_burst >= BURSTS,
        "only {} burst ticks overflowed the ring across {BURSTS} bursts",
        s.overflow_burst
    );

    // 6 — drop-newest, with its consequence made observable. This is the
    // assertion this file exists to be able to make, and no other gate in
    // the tree is positioned to make it: `bursts_to_zero` is ground truth
    // off `Backpacks::len`, so at least `bursts_to_zero * MAX_BACKPACKS`
    // bags left the store, while `emptied` counts only the removals the
    // ring still had room to announce. The gap is the events a client
    // never hears about — which is exactly what the overflow policy
    // promises and what `event_ring_overflow_heals_by_resync` exists to
    // recover from. If these ever agree, the ring stopped overflowing and
    // assertion 5 is measuring something else.
    let removed_truth = (s.bursts_to_zero * MAX_BACKPACKS) as u32;
    assert!(
        removed_truth > s.emptied,
        "the store lost at least {removed_truth} bags but announced {} removals — \
         a saturated ring must drop some of them, so these agreeing means the \
         withdrawal stopped saturating it",
        s.emptied
    );
}

/// The storm is still a fight, and the fight is still what fills the
/// store. A storm that quietly stopped killing would drain the store once
/// and then measure an empty world for the rest of the run, satisfying
/// every cap assertion above.
#[test]
fn the_loot_storm_is_still_a_fight() {
    let s = storm();
    assert!(
        s.deaths as usize > MAX_BACKPACKS,
        "{} deaths is not enough to have kept a {MAX_BACKPACKS}-bag store fed \
         while a hundred bodies emptied it",
        s.deaths
    );
    assert!(
        s.min_standing > 0,
        "every body was down on the same tick — the storm stopped being a fight"
    );
    assert!(
        s.deaths as usize >= PLAYERS,
        "only {} deaths across {PLAYERS} duellists — some duel never landed a blow",
        s.deaths
    );
    // The fight refilled the store four times over, from empty. That is
    // `test_loot_storm`'s `bursts_from_full`, and it is asserted there off
    // `Backpacks::len`; what is asserted HERE is the fight that did it, so
    // a storm whose duels went quiet names the fight rather than the store.
    //
    // Deliberately NOT asserted off `dropped_scavenge`, and this is the
    // file's sharpest lesson: during a burst the ring is saturated by
    // `EV_GATHER`, so `EV_BAG_DROPPED` is itself dropped — 6 announced
    // across 32 burst ticks where the death rate implies roughly fifty.
    // **An event count taken while the ring is overflowing is an undercount
    // of the thing it names**, so ground truth inside a burst comes off the
    // store. The two fields stay in `Storm` because that gap is what
    // assertion 6 gates.
    assert!(
        s.emptied_scavenge > s.dropped_scavenge,
        "{} bag removals announced against {} bags stood up during bursts — the \
         withdrawal no longer outruns the feed inside a burst",
        s.emptied_scavenge,
        s.dropped_scavenge
    );
}

/// **Wall 5, with the store part-drained.** Two storms from one seed agree
/// on the state hash, the save the second wrote is byte-identical, and it
/// loads back into the state it saved from. The sibling crosses this path
/// with the store near full; this crosses it after a hundred bodies have
/// been swap-removing from it for hundreds of ticks, which is the shape
/// `Backpacks::remove` is most likely to get wrong.
#[test]
fn the_loot_storm_is_deterministic_and_saves_whole() {
    let a = storm();
    let b = storm();
    assert_eq!(
        a.hash, b.hash,
        "two identical loot storms disagreed on the state hash"
    );
    assert_eq!(
        a.save, b.save,
        "two identical loot storms wrote different world saves"
    );

    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.gather = ladder_only();
    w.load(&a.save).expect("the storm's world must load");
    assert_eq!(
        w.state_hash(),
        a.hash,
        "a loot storm's world did not survive its own save/load round trip"
    );
}
