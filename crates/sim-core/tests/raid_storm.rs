//! `test_raid_storm` — wall 4's caps under everything at once.
//!
//! `CLAUDE.md` wall 4 has named this gate for months and marked it absent:
//! *"the caps are gated one site at a time and nothing drives them all at
//! once."* This is the thing that drives them all at once. It is also the
//! sim-side half of the merge-gate judge's ranked gap 1 (pass
//! `20260813-230343-13`): *a player has no opponent, and every balance
//! number is priced for one.* The 100-bot soak proved a population that
//! **walks** — `bot_frame` has never issued an action of any kind, so the
//! whole reliable-action lane (place, deploy, access, throw, move, loot,
//! demolish, repair) has never been driven by more than one player in a
//! unit test.
//!
//! ## What it is
//!
//! `PLOTS` cells, each with two players standing on it: an **owner** that
//! lays a foundation, stands two walls, drops a container, bolts a code
//! lock on it, arms it and banks goods — and an **attacker** that plants
//! charges on the walls and the floor, guesses the code, reaches into the
//! locked box, tries to build on the claim and demolishes what it can.
//! Both sides run `sim_core::bots::raid_step`, the shipped profile, so
//! this file is a *driver* and not a second implementation of a raid.
//!
//! ## What it asserts, and why each one can go red
//!
//! 1. **Every store stays inside its cap, every tick.** The invariant.
//! 2. **The charge store actually fills** and answers `REFUSE_B_FULL`.
//!    Without this the first assertion is a pass it didn't earn — a storm
//!    that never reaches a cap proves nothing about the cap. Delete the
//!    `MAX_LIVE_CHARGES` check in `charge::place` and this goes red.
//! 3. **No tick removes more pieces than `MAX_REMOVALS_PER_TICK`**, and
//!    the storm removes pieces in bulk (asserted separately), so the
//!    budget is under real pressure rather than nominally observed.
//! 4. **The event ring overflows and the world survives it** —
//!    `MAX_EVENTS_PER_TICK`'s drop-newest policy, which no single-site
//!    test can reach because it takes a crowd to overflow 256 events.
//! 5. **The storm is deterministic** (wall 5) and **survives a save/load
//!    round trip byte for byte** (wall 5 + `worldsave`) — the messiest
//!    world this repo can build, through the one non-command path into
//!    `World`.
//!
//! ## What it deliberately does NOT do
//!
//! The throwable's `damage` is **0**: bodies are not in this storm. A
//! blast that killed the players would end the storm in a few ticks and
//! the gate would measure a graveyard instead of a cap. `blast.rs` owns
//! the body arithmetic and owns it properly. The consequence is that
//! `MAX_BACKPACKS` and the death/respawn ring are the one client-driven
//! family this storm does not reach — named in `NOW.md`, not papered over.
//!
//! Inventories are topped up every `RAID_CYCLE` ticks by writing slots
//! directly, the way `blast.rs` stocks its raider. The storm is about the
//! caps, not the economy: a raider that ran out of satchels on tick 40
//! would stop driving anything.

#![allow(clippy::disallowed_macros)]

use sim_core::bots::{raid_step, RaidPlan, RaidRows, RAID_CYCLE};
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, REFUSE_B_FULL};
use sim_core::combat::{CombatContent, ThrowDef};
use sim_core::deploy::{DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::{
    MAX_BOXES, MAX_COMMANDS_PER_TICK, MAX_DEPLOYS, MAX_EVENTS_PER_TICK, MAX_LIVE_CHARGES,
    MAX_LOCKS, MAX_PIECES, MAX_PLAYERS, MAX_REMOVALS_PER_TICK,
};
use sim_core::movement::Body;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World, EV_BUILD_REFUSED};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;

const SEED: u64 = 0x5701_4D21;

/// Plots, each holding one owner and one attacker. 32 × 2 = 64 bodies,
/// inside `MAX_PLAYERS` with room to spare, and enough attackers planting
/// in lockstep that a detonation tick removes pieces in bulk rather than
/// one at a time — which is what puts `MAX_REMOVALS_PER_TICK` under real
/// pressure instead of nominal observation.
const PLOTS: usize = 32;
const PLAYERS: usize = PLOTS * 2;
const _: () = assert!(PLAYERS <= MAX_PLAYERS);

/// Long enough that the storm reaches steady state twice over and the
/// owners rebuild what the attackers took down several times.
const TICKS: u64 = 400;

/// Steps each plan takes per tick. `PLAYERS × STEPS_PER_TICK` is exactly
/// `MAX_COMMANDS_PER_TICK`, so the storm runs at the tick's own command
/// ceiling and not below it — which is also what it takes to overflow a
/// 256-entry event ring, the one bounded path a single-site test cannot
/// reach by construction.
const STEPS_PER_TICK: usize = MAX_COMMANDS_PER_TICK / PLAYERS;
const _: () = assert!(PLAYERS * STEPS_PER_TICK == MAX_COMMANDS_PER_TICK);

/// Cells between plots. `claim::PRIV_CUSHION_M` is 16 m and a cell is 3 m,
/// so 7 cells (21 m) keeps every plot's privilege volume to itself; the
/// widest blast in this fixture is 3 m, so no plot can reach another's
/// walls either. Plots that bled into each other would make every
/// refusal ambiguous.
const PLOT_SPACING: i32 = 7;

/// The fixture rows. Rows 0..=7 are `DeployContent::probe_fixture`'s; the
/// box is appended at 8 / item 9 exactly as `tests/lock_box.rs` appends
/// one, so the shipped fixture the replay golden installs is left alone.
const BOX_ROW: u16 = 8;
const BOX_ITEM: u16 = 9;
const LOCK_ROW: u16 = 5;
const LOCK_ITEM: u16 = 7;
const FOUNDATION_ROW: u16 = 0;
const WALL_ROW: u16 = 1;
/// Item 3 is the combat fixture's designated throwable.
const CHARGE_ITEM: u16 = 3;
/// Build costs: row 0 wants 5 of item 0, row 1 wants 3 of item 0.
const COST_ITEM_A: u16 = 0;
const COST_ITEM_B: u16 = 1;
/// The code the owners arm with. The attackers guess the whole space.
const OWNER_CODE: u16 = 4242;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `PLOTS` buildable cells, `PLOT_SPACING` apart, in ring order from the
/// middle of the map — deterministic for a seed, the scan every other
/// build test uses, widened to collect a set instead of one.
fn plots(seed: u64) -> [(u16, u16); PLOTS] {
    let mut out = [(0u16, 0u16); PLOTS];
    let mut n = 0usize;
    for r in 0..128i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                if n == PLOTS {
                    return out;
                }
                if dx % PLOT_SPACING != 0 || dz % PLOT_SPACING != 0 {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if !foundation_terrain_ok(seed, x, z) {
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
    assert_eq!(n, PLOTS, "the generator no longer offers {PLOTS} plots");
    out
}

/// `probe_fixture` plus a box on row 8 / item 9. The lock (row 5) bolts
/// onto it the way `lock_box.rs` proves it does.
fn storm_deploy() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[BOX_ROW as usize] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 60,
        item: BOX_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 9;
    d
}

/// The combat fixture with a real charge on it: no body damage (see the
/// header), a structure number that takes a 100 hp piece off the board in
/// one plant, a 3 m blast so the neighbouring pieces take a falloff share,
/// and a **90-tick fuse** — the fuse is what makes the storm a storm,
/// because a charge that resolved in four ticks would never let the live
/// store reach `MAX_LIVE_CHARGES`.
fn storm_combat() -> CombatContent {
    let mut c = CombatContent::probe_fixture();
    c.throw[CHARGE_ITEM as usize] = ThrowDef {
        damage: 0,
        structure: 125,
        fuse_ticks: 90,
        reach_cm: 200,
        blast_cm: 300,
    };
    c
}

fn rows() -> RaidRows {
    RaidRows {
        foundation: FOUNDATION_ROW,
        wall: WALL_ROW,
        container: BOX_ROW,
        lock: LOCK_ROW,
        charge_slot: 0,
        goods_slot: 2,
        code: OWNER_CODE,
    }
}

/// Top up the slots the script spends from. Slot 0 is the hotbar slot the
/// attacker selects, so it has to hold the throwable.
fn restock(inv: &mut [ItemStack]) {
    inv[0] = ItemStack {
        item: CHARGE_ITEM,
        count: 100,
    };
    inv[1] = ItemStack {
        item: COST_ITEM_A,
        count: 100,
    };
    inv[2] = ItemStack {
        item: COST_ITEM_B,
        count: 100,
    };
    inv[3] = ItemStack {
        item: BOX_ITEM,
        count: 50,
    };
    inv[4] = ItemStack {
        item: LOCK_ITEM,
        count: 50,
    };
    inv[5] = ItemStack {
        item: COST_ITEM_A,
        count: 100,
    };
}

/// What one storm saw. Every field is a *measurement*, and the assertions
/// live in the tests so a failure names which invariant broke.
struct Storm {
    hash: u64,
    /// Byte length and buffer of the world save taken after the last tick.
    save: Vec<u8>,
    peak_charges: usize,
    peak_pieces: usize,
    peak_deploys: usize,
    peak_locks: usize,
    peak_boxes: usize,
    peak_events: usize,
    /// Most pieces removed in any single tick.
    peak_removals: usize,
    /// Ticks on which the event ring refused an event.
    overflow_ticks: usize,
    saw_charge_store_full: bool,
    /// Every distinct event code the storm announced — the breadth check.
    codes: [bool; 64],
}

fn storm() -> Storm {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = storm_deploy();
    w.combat = storm_combat();

    let cells = plots(SEED);
    // Everybody spawns on plot 0 and is then seated on their own; the
    // spawn point only has to be somewhere legal.
    let (sx, sz) = cell_center(cells[0].0, cells[0].1);
    w.dev_spawn = Some((sx, sz));

    let mut plans = Vec::with_capacity(PLAYERS);
    for i in 0..PLAYERS {
        let id = (i as u32) + 1;
        w.tick(&[Command::Join { id }]);
        let (cx, cz) = cells[i / 2];
        plans.push(RaidPlan::new(id, cx, cz, i % 2 == 1));
    }
    assert_eq!(
        w.players.iter().filter(|p| p.active).count(),
        PLAYERS,
        "every raider seated"
    );

    // Seat each body on its own plot. Both sides of a plot stand on the
    // cell centre: every verb here is reach-checked, and a raider out of
    // reach would exercise nothing but REFUSE_B_REACH.
    for (i, plan) in plans.iter().enumerate() {
        let (x, z) = cell_center(plan.cx, plan.cz);
        let slot = slot_of(&w, plan.id);
        assert_eq!(slot, i, "join order is slot order");
        w.players[slot].body = Body::at(SEED, x, z);
    }

    // The script's own stream, and deliberately not the one `bot_frame`
    // spends: `bots.rs` says why a fourth draw there would move three
    // digests at once.
    let mut rng = Pcg32::new(SEED ^ 0x5A1D_C0DE, 7);
    let mut s = Storm {
        hash: 0,
        save: Vec::new(),
        peak_charges: 0,
        peak_pieces: 0,
        peak_deploys: 0,
        peak_locks: 0,
        peak_boxes: 0,
        peak_events: 0,
        peak_removals: 0,
        overflow_ticks: 0,
        saw_charge_store_full: false,
        codes: [false; 64],
    };
    let r = rows();
    let mut cmds: Vec<Command> = Vec::with_capacity(PLAYERS * STEPS_PER_TICK);

    for t in 0..TICKS {
        if t % RAID_CYCLE as u64 == 0 {
            for i in 0..PLAYERS {
                restock(&mut w.players[i].inv);
            }
        }
        cmds.clear();
        for _ in 0..STEPS_PER_TICK {
            for plan in plans.iter_mut() {
                cmds.push(raid_step(plan, &mut rng, r));
            }
        }
        assert!(
            cmds.len() <= MAX_COMMANDS_PER_TICK,
            "the storm must not out-run the tick's own command cap"
        );

        let pieces_before = w.pieces.len();
        w.tick(&cmds);

        // ---- the invariant, every tick ----
        assert!(
            w.charges.len() <= MAX_LIVE_CHARGES,
            "tick {t}: charges {} past cap",
            w.charges.len()
        );
        assert!(w.pieces.len() <= MAX_PIECES, "tick {t}: pieces past cap");
        assert!(w.deploys.len() <= MAX_DEPLOYS, "tick {t}: deploys past cap");
        assert!(
            w.deploys.locks().len() <= MAX_LOCKS,
            "tick {t}: locks past cap"
        );
        assert!(
            w.deploys.boxes().len() <= MAX_BOXES,
            "tick {t}: boxes past cap"
        );
        assert!(
            w.events.len() <= MAX_EVENTS_PER_TICK,
            "tick {t}: event ring past cap"
        );
        let removed = pieces_before.saturating_sub(w.pieces.len());
        assert!(
            removed <= MAX_REMOVALS_PER_TICK,
            "tick {t}: {removed} pieces removed in one tick, budget is {MAX_REMOVALS_PER_TICK}"
        );

        // ---- what the storm actually reached ----
        s.peak_charges = s.peak_charges.max(w.charges.len());
        s.peak_pieces = s.peak_pieces.max(w.pieces.len());
        s.peak_deploys = s.peak_deploys.max(w.deploys.len());
        s.peak_locks = s.peak_locks.max(w.deploys.locks().len());
        s.peak_boxes = s.peak_boxes.max(w.deploys.boxes().len());
        s.peak_events = s.peak_events.max(w.events.len());
        s.peak_removals = s.peak_removals.max(removed);
        if w.events.dropped > 0 {
            s.overflow_ticks += 1;
        }
        for e in w.events.entries() {
            if (e.code as usize) < s.codes.len() {
                s.codes[e.code as usize] = true;
            }
            if e.code == EV_BUILD_REFUSED && e.b == REFUSE_B_FULL {
                s.saw_charge_store_full = true;
            }
        }
    }

    // Quiet the world before the hash is taken. A world save puts every
    // body to bed on load, so a comparison against a world with 64 people
    // still driving would fail on the sleeping bit and say nothing about
    // the raid — `tests/worldsave.rs`'s `a_quiet_world` draws the same
    // line for the same reason.
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

fn slot_of(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("a joined raider has a slot")
}

/// **Wall 4, all at once.** Every cap holds through the storm — asserted
/// per tick inside `storm()` — and this is the half that proves the storm
/// was worth running: it reached a cap, the cap refused rather than grew,
/// pieces came off the board in bulk under a budget, and the event ring
/// overflowed and healed.
#[test]
fn test_raid_storm() {
    let s = storm();

    assert_eq!(
        s.peak_charges, MAX_LIVE_CHARGES,
        "the storm never filled the charge store — it proved nothing about the cap"
    );
    assert!(
        s.saw_charge_store_full,
        "a full charge store must answer REFUSE_B_FULL, not silently drop the plant"
    );
    // Measured 64 of 64: the storm pins the removal budget against its
    // ceiling and it holds. Equality rather than `<=` on purpose — `<=`
    // alone is satisfied by a storm that removes nothing, and the whole
    // point of this file is that the budget was under pressure when it
    // held. A drop below is not a benign regression either: it means the
    // detonations stopped landing together and the gate went quiet.
    assert_eq!(
        s.peak_removals, MAX_REMOVALS_PER_TICK,
        "the removal budget was not saturated — the storm is no longer pressing it"
    );
    assert!(
        s.overflow_ticks > 0,
        "the event ring never overflowed; {PLAYERS} players is no longer a crowd, \
         or MAX_EVENTS_PER_TICK moved"
    );
    assert_eq!(
        s.peak_events, MAX_EVENTS_PER_TICK,
        "an overflowing ring must be exactly full"
    );

    // The stores the storm builds into stay well inside their caps, which
    // is the honest reading: 32 plots cannot fill MAX_PIECES. What is
    // proved here is that they filled AT ALL — a storm whose owners never
    // got a wall up would satisfy every cap assertion above by building
    // nothing.
    assert!(s.peak_pieces >= PLOTS, "every plot stood at least a piece");
    assert!(
        s.peak_deploys >= PLOTS,
        "every plot placed at least a deploy"
    );
    assert!(s.peak_locks >= PLOTS, "every plot bolted a lock");
    assert!(s.peak_boxes >= PLOTS, "every plot placed a box");
    assert!(s.peak_pieces <= MAX_PIECES);
    assert!(s.peak_boxes <= MAX_BOXES);
}

/// The storm's breadth: it is only wall 4's gate if it actually walked the
/// verbs. These are the event codes a raid must announce — a placement, a
/// removal, a structure hit, a charge planted, a lock answering, a move
/// and a refusal on three different lanes. If a future change makes one of
/// these unreachable from the profile, this names which.
#[test]
fn the_storm_walks_the_raid_verbs() {
    use sim_core::world::{
        EV_AUTH, EV_CHARGE_PLACED, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED,
        EV_HEALTH, EV_MOVED, EV_MOVE_REFUSED, EV_PIECE_PLACED, EV_PIECE_REMOVED, EV_PIECE_REPAIRED,
        EV_STRUCT_HIT,
    };
    let s = storm();
    for (code, what) in [
        (EV_PIECE_PLACED, "a wall went up"),
        (EV_PIECE_REMOVED, "a wall came down"),
        (EV_PIECE_REPAIRED, "a wall was mended"),
        (EV_STRUCT_HIT, "a blast landed on a structure"),
        (EV_CHARGE_PLACED, "a charge was planted"),
        (EV_DEPLOY_PLACED, "a box or a lock was placed"),
        (EV_DEPLOY_REMOVED, "a box or a lock came off"),
        (EV_DEPLOY_REFUSED, "a deploy verb refused somebody"),
        (EV_AUTH, "a lock answered a correct code"),
        // The only source of body damage in this fixture: the throwable's
        // `damage` is 0, nobody swings, and there are no mobs. So this
        // code means `lock::shock` — the keypad's answer to a wrong guess
        // — which is a ladder no other test drives at population.
        (EV_HEALTH, "a wrong code shocked somebody"),
        (EV_MOVED, "goods moved"),
        (EV_MOVE_REFUSED, "a move was refused"),
        (EV_BUILD_REFUSED, "a build verb refused somebody"),
    ] {
        assert!(
            s.codes[code as usize],
            "the storm never announced EV code {code} — {what}"
        );
    }
    // A charge can only land in reach, so this is also the check that the
    // bodies are still standing where the fixture seated them — a drift
    // would refuse half the storm as `REFUSE_B_REACH` before it began, and
    // every cap assertion above would pass on a world where nothing
    // happened.
    assert!(s.peak_charges > 0, "no charge ever landed — check reach");
}

/// **Wall 5, on the messiest world this repo can build.** Two storms from
/// one seed agree on the state hash, and the save the second one wrote is
/// byte-identical to the first — so a raid at population is deterministic
/// *and* the world store carries all of it.
#[test]
fn the_storm_is_deterministic_and_saves_whole() {
    let a = storm();
    let b = storm();
    assert_eq!(
        a.hash, b.hash,
        "two identical storms disagreed on the state hash"
    );
    assert_eq!(
        a.save, b.save,
        "two identical storms wrote different world saves"
    );

    // And it loads back into the same state it saved from: the save is
    // the one non-command path into `World`, and a raid is where it holds
    // the most (live fuses, locks with guess histories, part-full boxes).
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = storm_deploy();
    w.combat = storm_combat();
    w.load(&a.save).expect("the storm's world must load");
    assert_eq!(
        w.state_hash(),
        a.hash,
        "a storm's world did not survive its own save/load round trip"
    );
}
