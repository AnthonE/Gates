//! `test_hurt_routes` — **a bite and a blast point somewhere**, and the
//! direction is the right one.
//!
//! The behavioural half of `tests/damage_routes.rs`'s announce column.
//! That gate reads the crate's own source and proves the string `EV_HURT,`
//! is present in every module declared as a damage route; this one drives a
//! world until an animal bites and until a charge goes off, and reads the
//! event that came out. Both are needed and neither substitutes for the
//! other: a scrape cannot tell a live emit from a dead one behind a `false`,
//! and a fixture cannot tell you about the route nobody wrote a fixture for.
//!
//! ## Why these two routes and not the other three
//!
//! A swing, an arrow and a bullet have carried `EV_HURT` since wire v57 and
//! `tests/event_roles.rs` asserts the melee one's payload roles against a
//! duel fixture. The bite and the blast were **silent for eleven days** with
//! every wall green — they debit hp through the same `combat::hurt` funnel,
//! so `damage_routes.rs` classified them and was satisfied; the event queue
//! is outside `state_hash`, so `test_replay` could not see it; and nothing
//! was ever encoded wrong, so `test_protocol_golden` had no opinion. The
//! defect was an event that *did not exist*, which is the one shape a
//! golden of the events that do exist cannot catch.
//!
//! ## What the sectors prove
//!
//! Each route is driven **twice, from two different sides**, and both
//! answers are asserted as literal sector numbers rather than recomputed
//! with `bearing_sector` — a test that calls the function under test to
//! build its expectation is checking that function against itself
//! (`CLAUDE.md`, the naive-rebuild trap). Two sides is what makes them
//! mutant-resistant: one bearing pins the value, and the second pins the
//! *convention*, because an axis swap or a negated delta moves one of the
//! two without moving the other.
//!
//! The two routes also disagree about units on purpose — the bite passes
//! raw position quanta and the blast passes centimetres — so between them
//! they also assert that `bearing_sector` reads only the ratio, which is
//! the property that lets each call site use whatever it already has.

#![allow(clippy::disallowed_macros)]

use sim_core::build::{anchor, foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO};
use sim_core::combat::{CombatContent, HURT_SECTORS};
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::InputFrame;
use sim_core::mob::{MobContent, MOB_PIG};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::world::{Command, SimEvent, World, EV_HURT};

/// The seeded haven, cached per seed. Resolving `terrain::haven` on every
/// call is what took `tests/mob.rs`'s run past five minutes, and a
/// `std::sync::Mutex` is on `sim-core/clippy.toml`'s disallowed list — the
/// list is crate-scoped, so wall 3 binds this suite too.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
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

/// Step until an `EV_HURT` lands, and hand back **that tick's** queue.
///
/// `World::tick` clears the event queue as its first statement, so the only
/// way to read an event is to stop on the tick that raised it. A loop that
/// runs to completion and then looks finds an empty queue, passes nothing,
/// and would have to be written as an hp assertion — which is exactly the
/// blind spot this file exists to close.
fn until_hurt(
    w: &mut World,
    mut step: impl FnMut(&mut World, u16) -> Vec<Command>,
    steps: u16,
    what: &str,
) -> SimEvent {
    for seq in 0..steps {
        let cmds = step(w, seq);
        w.tick(&cmds);
        let hits: Vec<SimEvent> = w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_HURT)
            .copied()
            .collect();
        if !hits.is_empty() {
            assert_eq!(
                hits.len(),
                1,
                "{what}: {} EV_HURTs on one tick — the fixture is meant to \
                 stage exactly one blow, so an assertion on `b` would be \
                 reading whichever of them happened to sort first",
                hits.len()
            );
            assert!(
                hits[0].b < HURT_SECTORS as u32,
                "{what}: EV_HURT.b is {} and there are only {HURT_SECTORS} \
                 sectors — the server range-refuses this at encode and the \
                 player would get nothing",
                hits[0].b
            );
            return hits[0];
        }
    }
    panic!(
        "{what}: no EV_HURT in {steps} ticks. Either the route stopped \
         debiting bodies (which the hp assertions in tests/mob.rs and \
         tests/blast.rs would also catch) or it went back to debiting them \
         silently, which is the regression this gate is for"
    );
}

// ---- a bite -------------------------------------------------------------

const MOB_SEED: u64 = 11;
const BITTEN: u32 = 1;

/// One pig, `metres` away along `(dx, dz)`, and nothing else alive.
///
/// The roster is emptied because hp is a fact any animal in reach can move
/// and an *event* test is stricter than an hp test: a second biter on the
/// same tick puts two `EV_HURT`s in one queue and `until_hurt` refuses to
/// guess which one it was asked about. Emptying it changes no numbers.
fn alone_with_pig(dx: f32, dz: f32) -> (World, usize) {
    let mut w = World::new(MOB_SEED);
    w.combat = CombatContent::probe_fixture();
    w.mob = MobContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(BITTEN));
    w.tick(&[Command::Join { id: BITTEN }]);
    let slot = w
        .mobs
        .m
        .iter()
        .position(|m| m.alive && m.kind == MOB_PIG)
        .expect("the roster hatched no live pig");
    for (i, m) in w.mobs.m.iter_mut().enumerate() {
        if i != slot {
            m.alive = false;
        }
    }
    let b = w.players[0].body;
    let (px, pz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    w.mobs.m[slot].body = Body::at(MOB_SEED, hv(MOB_SEED), px + dx, pz + dz);
    (w, slot)
}

/// Hold the pig at exactly `(dx, dz)` metres from the body at the top of
/// every tick, and stand still.
///
/// **A turntable, not a cheat, and without it there is no geometry to
/// assert.** A pig charges: left alone it closes the gap, runs
/// *through* the player and bites from the far side, because the bite is
/// phase-locked (`tick % attack_ticks == slot % attack_ticks`) and lands on
/// whichever tick that comes round on. Measured, both offsets below then
/// report the sector 180° from where the animal was placed — the exact
/// signature of a sign flip, produced by a correct sign. An assertion
/// written against that would be pinning the pig's stride, not the bearing.
///
/// Re-anchoring each tick leaves `mob::step` free to think, rouse and take
/// its stride inside the tick; the stride is along the line to the player,
/// so it shortens the range and does not turn the bearing. The animal is
/// therefore always on the side the fixture put it on when the bite
/// resolves, which is the one property these tests are about.
fn pinned_pig(dx: f32, dz: f32, slot: usize) -> impl FnMut(&mut World, u16) -> Vec<Command> {
    move |w: &mut World, seq: u16| {
        let b = w.players[0].body;
        let (px, pz) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
        w.mobs.m[slot].body = Body::at(MOB_SEED, hv(MOB_SEED), px + dx, pz + dz);
        vec![Command::Input {
            id: BITTEN,
            frame: InputFrame {
                seq,
                ..InputFrame::default()
            },
            favour: 0,
        }]
    }
}

/// A pig standing at **+x** bites, and the arc points **west**.
///
/// `+X` is west on `look::bearing_of`'s axes (+Z north, −X east), so a
/// delta of `mob − victim` with a positive x and a zero z is sector 12 of
/// 16, clockwise from north. Asserted as `12`, not as a call to
/// `bearing_sector`.
#[test]
fn a_pig_to_the_west_bites_from_the_west() {
    let (mut w, slot) = alone_with_pig(1.5, 0.0);
    let full = w.players[0].hp;
    assert!(full > 0, "the combat fixture arms bodies");
    let e = until_hurt(&mut w, pinned_pig(1.5, 0.0, slot), 400, "a pig at +x");
    assert_eq!(
        e.a, BITTEN,
        "EV_HURT.a is the VICTIM's player id — a bite has no attacker with \
         a screen, so if this is anything else the server unicasts the arc \
         to nobody"
    );
    assert_eq!(
        e.b, 12,
        "a pig standing at +x is due WEST of the body it bit, which is \
         sector 12 of {HURT_SECTORS}. Sector 4 means the delta was taken \
         victim-minus-mob; sector 0 or 8 means the axes were swapped"
    );
    assert!(
        e.c > 0,
        "EV_HURT.c is the damage the bite carried and a zero-damage bite is \
         not a bite"
    );
    assert!(
        w.players[0].hp < full,
        "the event and the debit are the same blow, so hp must have moved too"
    );
}

/// The same pig at **−z** bites, and the arc points **south**.
///
/// The second side, and the one that makes the first mean something: an
/// axis swap moves this answer to 4 and a negated delta moves it to 0,
/// while a mutant that satisfies one of the two by luck has to satisfy the
/// other by the same luck in a different quadrant.
#[test]
fn a_pig_to_the_south_bites_from_the_south() {
    let (mut w, slot) = alone_with_pig(0.0, -1.5);
    let e = until_hurt(&mut w, pinned_pig(0.0, -1.5, slot), 400, "a pig at -z");
    assert_eq!(
        e.b, 8,
        "a pig standing at −z is due SOUTH, sector 8 of {HURT_SECTORS}. \
         Sector 0 means the delta was negated; 4 or 12 means x and z were \
         swapped"
    );
}

// ---- a blast ------------------------------------------------------------

const BLAST_SEED: u64 = 0xB1A57;
const RAIDER: u32 = 3;
/// The satchel row in the combat probe fixture.
const SATCHEL: u16 = 9;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell");
}

/// A charge planted on a wall, with the raider standing `(dx, dz)` metres
/// from the epicentre when it goes off.
///
/// The satchel is re-priced down from `blast.rs`'s raiding numbers on
/// purpose: this fixture is about a bearing, and a blow that kills raises
/// `EV_DEATH` and lays the body down, which is a different test's subject.
/// A wide radius and a small number make the survivor's arc the only thing
/// that happens.
fn blasted_from(dx: f32, dz: f32) -> World {
    let mut w = World::new(BLAST_SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.build.pieces[0].hp = 1000;
    w.build.pieces[1].hp = 1000;
    w.deploy = DeployContent::probe_fixture();
    let mut cc = CombatContent::probe_fixture();
    cc.throw[SATCHEL as usize] = sim_core::combat::ThrowDef {
        damage: 40,
        structure: 125,
        fuse_ticks: 60,
        reach_cm: 200,
        blast_cm: 900,
    };
    w.combat = cc;

    let (cx, cz) = buildable_cell(BLAST_SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: RAIDER }]);
    w.players[0].body = Body::at(BLAST_SEED, hv(BLAST_SEED), x, z);
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 20,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: SATCHEL,
        count: 2,
        cond: 0,
    };
    w.tick(&[Command::Place {
        id: RAIDER,
        row: 0,
        cx,
        cz,
        level: 0,
        loc: sim_core::build::LOC_PLANE,
        freehand: false,
    }]);
    w.tick(&[Command::Place {
        id: RAIDER,
        row: 1,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_XLO,
        freehand: false,
    }]);
    // Select the satchel, then plant it. Reach is 200 cm, so this has to
    // happen before the body moves.
    w.tick(&[Command::Input {
        id: RAIDER,
        frame: InputFrame {
            seq: 900,
            sel: 1,
            ..InputFrame::default()
        },
        favour: 0,
    }]);
    w.tick(&[Command::Throw {
        id: RAIDER,
        deploy: false,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_XLO,
    }]);
    assert_eq!(w.charges.len(), 1, "the plant must take");
    // Now stand where the test wants, measured off the epicentre the sim
    // will use — `build::anchor` is the same function `charge::detonate`
    // calls, which is the one part of the expectation it is right to share:
    // this places the body, it does not compute the answer.
    let (ax, az) = anchor(cx, cz, LOC_EDGE_XLO);
    w.players[0].body = Body::at(BLAST_SEED, hv(BLAST_SEED), ax + dx, az + dz);
    w
}

/// A charge at **−x** from the body, i.e. **east**, reports sector 4.
#[test]
fn a_charge_to_the_east_blasts_from_the_east() {
    let mut w = blasted_from(2.0, 0.0);
    let full = w.players[0].hp;
    let e = until_hurt(&mut w, |_, _| Vec::new(), 200, "a charge at -x");
    assert_eq!(e.a, RAIDER, "EV_HURT.a is the body in the blast radius");
    assert_eq!(
        e.b, 4,
        "standing 2 m to the +x side of the charge puts the charge due \
         EAST, sector 4 of {HURT_SECTORS} — `−X` is east. Sector 12 means \
         the delta was taken body-minus-bomb"
    );
    assert!(
        e.c > 0 && (e.c as u16) < full,
        "EV_HURT.c is the damage the blast carried at this distance ({}), \
         which must be a real number and must not be the whole body — a \
         death is a different event and a different fixture",
        e.c
    );
    assert!(
        w.players[0].hp < full && w.players[0].hp > 0,
        "the arc and the debit are the same blast, and this one is survivable"
    );
}

/// The same charge at **−z**, i.e. **south**, reports sector 8.
#[test]
fn a_charge_to_the_south_blasts_from_the_south() {
    let mut w = blasted_from(0.0, 2.0);
    let e = until_hurt(&mut w, |_, _| Vec::new(), 200, "a charge at -z");
    assert_eq!(
        e.b, 8,
        "standing 2 m to the +z side of the charge puts the charge due \
         SOUTH, sector 8 of {HURT_SECTORS}. Sector 0 is the negated delta \
         and also what `bearing_sector` returns for (0, 0), so a fixture \
         that lost its offset lands here too — which is why the east case \
         above is asserted as well"
    );
}
