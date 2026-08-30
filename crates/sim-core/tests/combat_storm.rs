//! `test_combat_storm` — the family `raid_storm.rs` had to leave out.
//!
//! Wall 4's population gate exists (`tests/raid_storm.rs`, 2026-08-14) and
//! its own header names what it cannot reach: *"the throwable's `damage`
//! is 0: bodies are not in this storm … the consequence is that
//! `MAX_BACKPACKS` and the death/respawn ring are the one client-driven
//! family this storm does not reach."* That was the right call there — a
//! blast that killed the raiders would have measured a graveyard instead
//! of a cap. This file is the other half, and it is a **sibling rather
//! than a retrofit** for the same reason: the raid storm holds five
//! equality assertions about saturation (`peak_charges`, `peak_removals`,
//! `peak_events`) that were sized for a plot whose owner is alive to
//! rebuild, and arming that fixture would have quietly desaturated them.
//!
//! It is also the merge-gate judge's ranked gap 3 (pass
//! `20260829-153230-21`): *"nothing has ever fought — at population, over
//! a link, or in front of a person … the evidence for all of it is unit
//! fixtures."* Combat is the most heavily invested subsystem in this tree
//! — the swing cadence, the melee cone, the hitscan solve, the magazine,
//! the corpse bag, the spawn ring, the rewind — and every gate on it
//! drives one or two bodies. This drives a hundred, which is every seat
//! the world has.
//!
//! ## What it is
//!
//! `DUELS` pairs, each pair a **bearing** rather than a place: one body on
//! a buildable cell centre facing `yaw`, its partner `SEPARATION_M` ahead
//! facing back down the same line. Half the duels are gunfights (fixture
//! item 6, hitscan, six rounds and a reload), half are knife fights
//! (fixture item 0, 34 damage, three swings to a kill) — half of each, so
//! neither weapon can be the only reason a cap was reached. Both sides run
//! `sim_core::bots::brawl_step`, the shipped profile, so this file is a
//! *driver* and not a second implementation of a fight.
//!
//! Nobody wins. A body that goes down asks for a respawn on the next tick
//! it is driven, the spawn ring answers, and the fixture seats it back on
//! its mark — which is what keeps the storm a storm across `TICKS`
//! instead of the graveyard the raid storm's header predicted.
//!
//! ## What it asserts, and why each one can go red
//!
//! 1. **Every store stays inside its cap, every tick.** The invariant.
//! 2. **The backpack store fills to `MAX_BACKPACKS` and evicts** —
//!    through `World::die`, which no test has ever done. Today the only
//!    coverage of that overflow policy is `backpack.rs`'s own unit test
//!    calling `drop_for` on a bare `Backpacks`; every world-level bag
//!    test asserts `len() == 1`. Delete the cap check in
//!    `backpack::stand_up` and this goes red.
//! 3. **The event ring overflows and the world survives it.** A death is
//!    five events on one tick — the swing or the shot, the hit, the
//!    health, the death, the bag — so a tick on which a dozen duels
//!    resolve together passes the 256-slot ring. Measured: 4 overflow
//!    ticks of 600, and 188 of 256 at the ring's peak when this storm ran
//!    64 bodies instead of 100, which is why it runs 100.
//! 4. **Both weapons actually fought** — the breadth check, so a storm
//!    that silently stopped landing (a cone that closed, a magazine that
//!    stopped feeding, a bag that stopped dropping) names which.
//! 5. **It is deterministic** (wall 5) and **survives a save/load round
//!    trip byte for byte**, with the bag store part-full and a hundred
//!    corpses' worth of loot in it.
//!
//! ## What it deliberately does NOT do
//!
//! Nobody loots. `Command::Loot` empties the nearest bag in reach, and
//! sixty-four bodies each emptying one bag a tick would hold the store
//! near zero — the deposit side is what has never been driven at
//! population, and the two do not fit in one fixture. The withdrawal side
//! is `NOW.md`'s, named there.
//!
//! Nobody moves either: `move_x`/`move_z` are zero and the bodies stand on
//! their marks. So the rewind depth every command carries drives
//! `Rewind::pose_at`'s **lookup** at population — a hundred distinct
//! depths a tick against the one a unit fixture spends — and cannot be
//! read as a claim about the correction. Lag compensation over a real
//! link is `NOW.md` §0lc's and stays there.

#![allow(clippy::disallowed_macros)]

use sim_core::backpack::{BackpackContent, BAG_GONE_DESPAWN, BAG_GONE_EVICTED};
use sim_core::bots::{brawl_step, BrawlPlan};
use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M};
use sim_core::combat::CombatContent;
use sim_core::gather::ItemStack;
use sim_core::limits::{MAX_BACKPACKS, MAX_COMMANDS_PER_TICK, MAX_EVENTS_PER_TICK, MAX_PLAYERS};
use sim_core::movement::Body;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World, EV_BAG_REMOVED};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;
use sim_core::yaw_dir;

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
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

/// The raid storm's island, deliberately. The duel marks are chosen by the
/// same `foundation_terrain_ok` scan at the same spacing, so a seed whose
/// generator stopped offering `DUELS` flat cells would fail both files
/// with one cause instead of two.
const SEED: u64 = 0x5701_4D21;

/// Every seat in the world, taken. `MAX_PLAYERS` is the one cap this file
/// can pin by construction rather than by pressure, and it is worth
/// pinning: the event ring is what a crowd overflows, and at 64 bodies it
/// peaked at 188 of 256 — a storm that never reached the ring proved
/// nothing about drop-newest.
const DUELS: usize = MAX_PLAYERS / 2;
const PLAYERS: usize = DUELS * 2;
const _: () = assert!(PLAYERS == MAX_PLAYERS);

/// Long enough to fill a 256-bag store several times over. Measured: 954
/// deaths, a bag store pinned at its cap and evicting, and the ring over
/// its ceiling on 4 ticks. 400 ticks reached the cap too (629 deaths) and
/// overflowed the ring on 2, which is a thinner margin than an assertion
/// this cheap needs.
const TICKS: u64 = 600;

/// Commands each duellist issues per tick. Two, not four: this storm's
/// business is the death ring and the bag store, and the tick's command
/// ceiling is already `raid_storm.rs`'s claim — running at it here would
/// be a second, weaker copy of a gate that exists.
const STEPS_PER_TICK: usize = 2;
const _: () = assert!(PLAYERS * STEPS_PER_TICK <= MAX_COMMANDS_PER_TICK);

/// How far apart the two halves of a duel stand.
///
/// Inside the fixture's 2 m melee reach with room for the ground to tilt
/// under it, and far enough out that the shot is a real solve rather than
/// `combat::strike`'s point-blank exemption (`POINT_BLANK_M2`, 0.2 m),
/// which would have made the 30° cone unreachable — a duel that never
/// tested its own aim is a duel that proves nothing about aiming.
const SEPARATION_M: f32 = 1.2;

/// Cells between duels. The firearm's fixture reach is 20 m; 7 cells is
/// 21 m, so no duellist can shoot into the next duel and every kill has
/// exactly one author.
const DUEL_SPACING: i32 = 7;

/// How often the fixture tops the kit back up. A body that respawned with
/// nothing would drop no bag when it died again (`backpack::stand_up`
/// refuses an empty inventory), and the storm's whole subject is the bag.
/// Five ticks is well inside the ~32 ticks the fastest weapon here needs
/// to kill, so nobody dies naked.
const RESTOCK_TICKS: u64 = 5;

/// `CombatContent::probe_fixture`'s item 0: 34 damage, 2 m reach — three
/// swings to a 100 hp kill.
const BLADE: u16 = 0;
/// Filler with a long bag life. `BackpackContent::probe_fixture` gives
/// items under 4 a 360-tick despawn and everything else 90, and a bag
/// lives as long as its longest-lived item — so this one item is what
/// keeps a gunfighter's bag on the ground long enough to be counted.
const KEEPSAKE: u16 = 2;
/// The fixture's hitscan firearm and its round (`combat.rs`, 2026-08-30).
const GUN: u16 = 6;
const ROUND: u16 = 7;

/// The hotbar slot every duellist holds its weapon in.
const WEAPON_SLOT: u8 = 0;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `DUELS` buildable cells, `DUEL_SPACING` apart, in ring order from the
/// middle of the map — the scan `raid_storm.rs` uses, for the same reason:
/// flat ground keeps the two halves of a duel inside `strike`'s vertical
/// window instead of one of them standing on a boulder.
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

/// Slot 0 is the weapon — the profile selects it and both verbs read the
/// held item. Slot 1 feeds it. Slot 2 is the keepsake that keeps the bag
/// on the ground.
fn restock(inv: &mut [ItemStack], shooter: bool) {
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
        count: 10,
        cond: 0,
    };
}

/// Where duellist `i` stands and which way it looks. Even index is the
/// body on the mark; odd index is its partner, `SEPARATION_M` down the
/// same bearing, facing back — a half turn is exactly half of `u16`'s
/// range, so the flip is a `wrapping_add` and not a trig call.
fn post(marks: &[(u16, u16); DUELS], i: usize) -> (f32, f32, u16) {
    let duel = i / 2;
    let (mx, mz) = cell_center(marks[duel].0, marks[duel].1);
    // A full turn over `DUELS` duels, so no two duels share a bearing and
    // the yaw LUT is walked rather than sampled at one entry.
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

/// Duel `d` is a gunfight when it is odd — sixteen of each, so neither
/// weapon can be the only reason a cap was reached.
fn is_shooter(i: usize) -> bool {
    (i / 2) % 2 == 1
}

/// What one storm saw. Every field is a *measurement*, and the assertions
/// live in the tests so a failure names which invariant broke.
struct Storm {
    hash: u64,
    save: Vec<u8>,
    peak_bags: usize,
    peak_events: usize,
    /// Ticks on which the event ring refused an event.
    overflow_ticks: usize,
    saw_bag_evicted: bool,
    saw_bag_despawned: bool,
    /// Deaths summed over every duellist at the end of the storm.
    deaths: u32,
    /// The fewest bodies on their feet on any one tick.
    min_standing: usize,
    /// Every distinct event code the storm announced — the breadth check.
    codes: [bool; 64],
}

fn storm() -> Storm {
    let mut w = World::new(SEED);
    // Combat is armed and gathering is not: `World::new` leaves
    // `GatherContent::EMPTY` in place, so `gather::swing` pays the
    // cadence and returns `Swing::Free` on every path (`gather.rs:927`,
    // `:936`) and the arm reaches `combat::strike`. A duel next to a
    // harvestable tree would have had its swing eaten by the tree.
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();

    let cells = marks(SEED);
    // Every respawn lands here and is then seated back on its mark. The
    // spawn ring's own bisection is a shoreline march per call and this
    // storm respawns hundreds of times.
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

    // The script's own stream, and deliberately not the one `bot_frame`
    // spends: `bots.rs` says why a draw there would move three digests.
    let mut rng = Pcg32::new(SEED ^ 0x0D0E_1DAE, 11);
    let mut s = Storm {
        hash: 0,
        save: Vec::new(),
        peak_bags: 0,
        peak_events: 0,
        overflow_ticks: 0,
        saw_bag_evicted: false,
        saw_bag_despawned: false,
        deaths: 0,
        min_standing: PLAYERS,
        codes: [false; 64],
    };
    let mut cmds: Vec<Command> = Vec::with_capacity(PLAYERS * STEPS_PER_TICK);
    let mut was_down = [false; PLAYERS];

    for t in 0..TICKS {
        if t.is_multiple_of(RESTOCK_TICKS) {
            for i in 0..PLAYERS {
                if !w.players[i].dead {
                    restock(&mut w.players[i].inv, is_shooter(i));
                }
            }
        }
        // Read once, before the tick: a client sends the frames it has,
        // against the state it last saw, and only learns it died when the
        // next snapshot tells it.
        for (i, down) in was_down.iter_mut().enumerate() {
            *down = w.players[i].dead;
        }
        cmds.clear();
        for _ in 0..STEPS_PER_TICK {
            for (i, plan) in plans.iter_mut().enumerate() {
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

        // ---- what the storm actually reached ----
        s.peak_bags = s.peak_bags.max(w.backpacks.len());
        s.peak_events = s.peak_events.max(w.events.len());
        if w.events.dropped > 0 {
            s.overflow_ticks += 1;
        }
        s.min_standing = s
            .min_standing
            .min(w.players.iter().take(PLAYERS).filter(|p| !p.dead).count());
        for e in w.events.entries() {
            if (e.code as usize) < s.codes.len() {
                s.codes[e.code as usize] = true;
            }
            if e.code == EV_BAG_REMOVED && e.b == BAG_GONE_EVICTED {
                s.saw_bag_evicted = true;
            }
            if e.code == EV_BAG_REMOVED && e.b == BAG_GONE_DESPAWN {
                s.saw_bag_despawned = true;
            }
        }

        // A body the ring answered for is put back on its mark. The
        // fixture owns positions here exactly as `raid_storm.rs`'s does —
        // `wake` puts you on a beach, and a storm whose survivors walk
        // away is not a storm.
        for (i, &down) in was_down.iter().enumerate() {
            if down && !w.players[i].dead {
                seat(&mut w, i);
            }
        }
    }

    s.deaths = w
        .players
        .iter()
        .take(PLAYERS)
        .map(|p| p.deaths as u32)
        .sum();

    // Quiet the world before the hash is taken, for `raid_storm.rs`'s
    // reason: a world save puts every body to bed on load, so a
    // comparison against a world with 64 people still driving would fail
    // on the sleeping bit and say nothing about the fight.
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

/// **Wall 4, on the family the raid storm could not reach.** The bag store
/// fills to its cap through `World::die` and evicts rather than growing,
/// the event ring overflows and heals, and the population that started the
/// fight is still standing at the end of it.
#[test]
fn test_combat_storm() {
    let s = storm();

    assert_eq!(
        s.peak_bags, MAX_BACKPACKS,
        "the storm never filled the bag store — it proved nothing about the cap"
    );
    assert!(
        s.saw_bag_evicted,
        "a full bag store must evict and announce it, not silently refuse the drop"
    );
    // Equality above rather than `<=` on purpose, and for `raid_storm.rs`'s
    // stated reason: `<=` alone is satisfied by a storm where nobody dies.
    assert!(
        s.deaths as usize > MAX_BACKPACKS,
        "{} deaths is not enough to have pressed a {MAX_BACKPACKS}-bag store",
        s.deaths
    );
    assert!(
        s.overflow_ticks > 0,
        "the event ring never overflowed; {PLAYERS} bodies fighting is no longer a crowd, \
         or MAX_EVENTS_PER_TICK moved"
    );
    assert_eq!(
        s.peak_events, MAX_EVENTS_PER_TICK,
        "an overflowing ring must be exactly full"
    );
}

/// The storm's breadth: it is only a gate on combat if it actually fought
/// with both weapons. These are the codes a fight must announce — a swing
/// taken, a hit landed with a body part on it, health spent, a body
/// dropped, a bag left behind, a bag taken away, somebody back on their
/// feet, a round fired and a magazine fed. If a future change makes one of
/// these unreachable at population, this names which.
#[test]
fn the_storm_walks_the_combat_verbs() {
    use sim_core::world::{
        EV_BAG_DROPPED, EV_DEATH, EV_HEALTH, EV_HIT, EV_HURT, EV_IMPACT, EV_RELOAD,
        EV_RELOAD_REFUSED, EV_RESPAWN, EV_SHOT, EV_SWING,
    };
    let s = storm();
    for (code, what) in [
        (EV_SWING, "somebody swung"),
        (EV_HIT, "a swing landed on a body part"),
        (EV_HURT, "somebody was told where it came from"),
        (EV_HEALTH, "health was spent"),
        (EV_DEATH, "somebody died"),
        (EV_BAG_DROPPED, "a corpse left a bag"),
        (EV_BAG_REMOVED, "a bag left the store"),
        (EV_RESPAWN, "somebody got back up"),
        (EV_SHOT, "a round left a barrel"),
        (EV_IMPACT, "a bullet marked what it reached"),
        (EV_RELOAD, "a magazine was fed"),
        (EV_RELOAD_REFUSED, "a cylinder came up empty"),
    ] {
        assert!(
            s.codes[code as usize],
            "the storm never announced EV code {code} — {what}"
        );
    }
    // The storm is a fight and not a queue: at its worst moment some of
    // the sixty-four were still upright. A zero here means a kill wave
    // took everybody at once and the respawn ring was the only thing
    // running, which would make every cap assertion above a statement
    // about an empty world.
    assert!(
        s.min_standing > 0,
        "every body was down on the same tick — the storm stopped being a fight"
    );
    // And everybody fought: a duel whose bearing drifted out of the cone
    // would leave one pair idle while the rest carried the counts.
    assert!(
        s.deaths as usize >= PLAYERS,
        "only {} deaths across {PLAYERS} duellists — some duel never landed a blow",
        s.deaths
    );
}

/// **Wall 5, with a hundred corpses' worth of loot on the ground.** Two
/// storms from one seed agree on the state hash, the save the second one
/// wrote is byte-identical to the first, and it loads back into the state
/// it saved from — the bag store is the newest thing in `worldsave` and
/// this is the first time it crosses that path anywhere near full.
#[test]
fn the_combat_storm_is_deterministic_and_saves_whole() {
    let a = storm();
    let b = storm();
    assert_eq!(
        a.hash, b.hash,
        "two identical combat storms disagreed on the state hash"
    );
    assert_eq!(
        a.save, b.save,
        "two identical combat storms wrote different world saves"
    );

    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.load(&a.save).expect("the storm's world must load");
    assert_eq!(
        w.state_hash(),
        a.hash,
        "a combat storm's world did not survive its own save/load round trip"
    );
}
