//! Firing a revolver, and the wall that eats the shot.
//!
//! **A sibling of `tests/shoot.rs` rather than an extension of it**, and the
//! split is the same one the sim makes. That suite's subject is the *flight*
//! — its header says so, its fixture is a bow, every check drives
//! `ranged::draw` into an `Arrows` store and steps it, and the thing it
//! gates is that a sampler and a predicate disagree about what stops a
//! moving thing. A hitscan has no store, no store cap, no `draw` and no
//! tick of travel: it resolves on the tick the trigger is pulled. Bolting
//! that onto a fixture built around an arrow in the air would make both
//! suites read as one and neither as a claim.
//!
//! **Every check here was watched going red** under a one-line mutation of
//! the thing it names — the world's stop ignored, the round not spent, the
//! cadence dropped, `draw` paying it twice, the owner not skipped, the
//! sleeper allowed to fire, `EV_SHOT` pushed, the reach doubled — except the
//! decal pairing, which took two (see its own doc).
//!
//! What the two share is the **discipline**, and it is borrowed whole: every
//! blocked shot is run twice over identical geometry, once with the cover
//! and once without, and the unblocked run must land the hit. Asserting only
//! that the victim survives is the exact shape of a gate that passes for the
//! wrong reason — a shot that never fired, or fired at the sky, satisfies it
//! just as well.
//!
//! The revolver's numbers are `content/weapons.toml`'s, written out rather
//! than loaded: `sim-core` has no dependency on the content crate (it is the
//! thing content is baked *for*), and a fixture that could be edited by a
//! balance pass is a gate a balance pass can turn green. The pairing of
//! those numbers with the band `balance.toml` declares is gated one crate
//! over, in `content/tests/content.rs`.

// Measurements are this gate's output — `tests/shoot.rs`'s allow and its
// reason: the L5 wall bans format/print in SIM code, and a test harness is
// not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::build::{self, BUILD_CELL_M, LOC_EDGE_ZLO, SHAPE_WALL};
use sim_core::collide::ColIndex;
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Occupants, Scratch};
use sim_core::ranged::{self, Arrows, Kill, ARROW_EYE_MM, SURF_BUILT};
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, EV_DEATH, EV_HIT, EV_IMPACT, EV_SHOT};

/// The solved authored sites for `seed`, memoized per seed — `shoot.rs`'s
/// helper and its reason: `terrain::haven` is a few thousand `height` taps
/// and these suites call it from inside assertion loops, and it is a pure
/// function of the seed so caching cannot change a result.
fn hv(seed: u64) -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// The browser-smoke seed, and the one the shard ships.
const SEED: u64 = 20260731;

/// The item index the fixture makes a revolver, and the one it makes its
/// round. Arbitrary and distinct; nothing else in the fixture uses either.
const GUN: u16 = 5;
const ROUND: u16 = 6;

/// Level pitch — `shoot.rs`'s constant and its reason: the client's encoding
/// puts 0 rad between 127 and 128, and 128 is what it actually sends when
/// you are looking at the horizon.
const LEVEL: u8 = 128;

/// The shipped revolver, converted the way `bake_ranged` converts it: 20
/// damage, 150 rounds/min at 30 Hz (12 ticks), 50 m of reach, and a round
/// with no ballistics — which is the whole of what makes it hitscan.
fn gun_fixture() -> sim_core::combat::CombatContent {
    use sim_core::combat::{CombatContent, RangedDef};
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage: 20,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 12,
        hitscan: true,
        range_mm: 50_000,
        structure: 0,
    };
    c
}

/// A player standing at (x, feet_y, z) holding a loaded revolver, facing
/// `yaw` at `pitch` with the trigger down.
fn shooter(id: u32, x: f32, feet_y: f32, z: f32, yaw: u16, pitch: u8, rounds: u16) -> Player {
    let mut p = Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.body = Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (feet_y / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        ..Body::default()
    };
    p.inv[0] = ItemStack {
        item: GUN,
        count: 1,
        cond: 0,
    };
    p.inv[7] = ItemStack {
        item: ROUND,
        count: rounds,
        cond: 0,
    };
    p.frame = InputFrame {
        seq: 1,
        buttons: BTN_PRIMARY,
        yaw,
        pitch,
        sel: 0,
        ..InputFrame::default()
    };
    p
}

/// A body that is only a target: alive, unarmed, standing still.
fn target(id: u32, x: f32, feet_y: f32, z: f32) -> Player {
    let mut p = Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.body = Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (feet_y / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        ..Body::default()
    };
    p
}

/// Everything one pull of the trigger produced.
struct Shot {
    events: EventQueue,
    kills: usize,
    kill: Kill,
}

/// Pull the trigger once, on `tick`, against `cols` and `occ`.
fn pull(
    tick: u64,
    cc: &sim_core::combat::CombatContent,
    cols: &ColIndex,
    occ: &mut Occupants,
    players: &mut [Player; MAX_PLAYERS],
) -> Shot {
    let mut events = EventQueue::default();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let (n, _n_chips) = ranged::hitscan(
        SEED,
        hv(SEED),
        cols,
        occ,
        tick,
        cc,
        players,
        &mut events,
        &mut kills,
        &mut chips,
    );
    Shot {
        events,
        kills: n,
        kill: kills[0],
    }
}

/// Two bodies ten metres apart, high enough that no hill is in the line.
fn face_off() -> Box<[Player; MAX_PLAYERS]> {
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = shooter(1, 0.0, 400.0, 0.0, 0, LEVEL, 6);
    players[1] = target(2, 0.0, 400.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2, 10.0);
    players
}

/// Five shots kill a hundred-point body, and the fifth reports a death with
/// the **gun** on it — never the round, because the weapon is what the
/// killer held.
///
/// Five is `content/weapons.toml`'s 20 damage against `balance.toml`'s 100
/// hp, and `content/tests/content.rs` is what holds that pairing inside the
/// declared `ttk_firearm` band. This asserts the sim plays the arithmetic;
/// that suite asserts the arithmetic is the data's.
#[test]
fn five_shots_kill_and_the_kill_names_the_gun() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();
    let mut players = face_off();
    let mut hits = 0;
    let mut deaths = 0;
    let mut killed: Option<Kill> = None;

    for t in 0..80u64 {
        let s = pull(t, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
        hits += s
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_HIT)
            .count();
        deaths += s
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_DEATH)
            .count();
        if s.kills > 0 {
            killed = Some(s.kill);
            break;
        }
    }

    let k = killed.expect("five shots at 20 did not kill a 100 hp body");
    assert_eq!(hits, 5, "a hundred points at twenty a shot is five hits");
    assert_eq!(deaths, 1, "one death, announced once");
    assert_eq!(k.victim, 1);
    assert_eq!(k.by, 1, "the kill is credited to the shooter's id");
    assert_eq!(k.item, GUN, "the death screen names the gun, not the round");
    assert!(
        k.range_cm > 900 && k.range_cm < 1100,
        "a 10 m shot reported {} cm",
        k.range_cm
    );
    assert_eq!(players[1].hp, 0);
    assert_eq!(players[1].deaths, 1);
    assert_eq!(players[0].inv[7].count, 1, "six rounds, five spent");
}

/// An empty chamber fires nothing, hurts nobody, and takes nothing —
/// including the round it does not have.
///
/// The cadence is still paid, and that is asserted rather than tolerated:
/// it is `draw`'s rule for a bow with an empty quiver, and its reason is
/// that a refused shot must not be re-attempted every tick.
#[test]
fn an_empty_chamber_fires_nothing_and_costs_nothing() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();
    let mut players = face_off();
    players[0].inv[7] = ItemStack::default();

    let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    assert_eq!(s.kills, 0);
    assert!(
        !s.events.entries().iter().any(|e| e.code == EV_HIT),
        "an empty chamber must not hit anybody"
    );
    assert!(
        !s.events.entries().iter().any(|e| e.code == EV_IMPACT),
        "an empty chamber must not mark the world either"
    );
    assert_eq!(players[1].hp, 100, "the target is untouched");
    assert_eq!(players[0].inv[7], ItemStack::default(), "nothing was taken");
    assert_eq!(
        players[0].next_swing, 12,
        "the cadence is paid on the pull, empty or not"
    );

    // And the control, on the same geometry: with a round in the pocket the
    // same pull lands. Without this the assertions above are satisfied by a
    // gun that never fires at all.
    let mut loaded = face_off();
    let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut loaded);
    assert_eq!(s.kills, 0, "one shot does not kill a full body");
    assert_eq!(loaded[1].hp, 80, "the control shot landed");
}

/// A wall between the shooter and the body stops the shot — and the body
/// behind it lives.
///
/// The pair is the point (`shoot.rs`'s discipline): the same geometry is
/// fired twice, once through a wall on the cell edge and once through an
/// empty `ColIndex`, and the empty run must land the hit. The wall is then
/// the only difference between a corpse and a survivor.
#[test]
fn a_wall_stops_the_shot_and_the_body_behind_it_lives() {
    let cc = gun_fixture();
    // A cell whose terrain a foundation would accept, so the column's floor
    // is flat enough for a level shot to stay above the ground either side.
    let (cx, cz) = buildable_cell(SEED);
    let fx = (cx as f32 + 0.5) * BUILD_CELL_M;
    let plane_z = cz as f32 * BUILD_CELL_M;
    // Inside the wall's own storey, half way up it.
    let shot_y = build::column_floor_y(SEED, hv(SEED), cx, cz, 0) + 1.5;

    let mut walled = Box::new(ColIndex::new());
    walled.add(cx, cz, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);

    for (cols, expect_hp, what) in [
        (&*walled, 100u16, "walled"),
        (&ColIndex::new(), 80u16, "open"),
    ] {
        let mut sc = Scratch::barren();
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[0] = shooter(
            1,
            fx,
            shot_y - ARROW_EYE_MM as f32 / 1000.0,
            plane_z - 2.0,
            0,
            LEVEL,
            6,
        );
        players[1] = target(2, fx, shot_y - 1.2, plane_z + 2.0);

        let s = pull(0, &cc, cols, &mut sc.occupants(), &mut players);
        assert_eq!(
            players[1].hp, expect_hp,
            "the {what} shot should have left {expect_hp} hp"
        );
        if expect_hp == 100 {
            let marks: Vec<_> = s
                .events
                .entries()
                .iter()
                .filter(|e| e.code == EV_IMPACT)
                .collect();
            assert_eq!(marks.len(), 1, "a stopped shot marks where it stopped");
            assert_eq!(
                (marks[0].a >> 24) as u8,
                SURF_BUILT,
                "the wall is what stopped it, and the surface byte must say so"
            );
        }
        assert_eq!(players[0].inv[7].count, 5, "the {what} shot spent a round");
    }
}

/// One pull spends exactly one round, at the weapon's own cadence rather
/// than the shared melee interval.
#[test]
fn a_shot_spends_one_round_at_the_weapons_own_cadence() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();
    let mut players = face_off();

    pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    assert_eq!(players[0].inv[7].count, 5, "one shot, one round");
    assert_eq!(
        players[0].next_swing, 12,
        "the revolver's 150/min is 12 ticks, not the melee 38"
    );
    assert_eq!(players[1].hp, 80);

    // Held down, every tick, until the cadence comes round.
    for t in 1..12u64 {
        pull(t, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    }
    assert_eq!(
        players[0].inv[7].count, 5,
        "the cadence held the second shot"
    );
    assert_eq!(players[1].hp, 80);
    pull(12, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    assert_eq!(
        players[0].inv[7].count, 4,
        "and released it on the twelfth tick"
    );
    assert_eq!(players[1].hp, 60);
}

/// A bullet never hits the person who fired it, however the geometry lands.
#[test]
fn a_bullet_never_hits_its_owner() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    // Fired straight down, from inside the shooter's own capsule.
    players[0] = shooter(1, 0.0, 400.0, 0.0, 0, 0, 6);

    let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    assert_eq!(players[0].hp, 100, "the shooter shot himself");
    assert_eq!(s.kills, 0);
    assert!(!s.events.entries().iter().any(|e| e.code == EV_HIT));
}

/// A shot that finds no body marks the world instead — and a shot that
/// finds one does not.
///
/// The pairing is `EV_IMPACT`'s whole contract, and it is asserted in one
/// test because the two halves are one rule: a hit draws a hitmarker and a
/// miss draws a decal, never both. A chip of bark where a body was hit
/// would tell everyone in earshot that the shooter missed.
///
/// The second half is **doubly** held today and the assertion is kept for
/// the weaker of the two: the body branch leaves before the mark, and the
/// world was not walked past the body anyway (the truncation). So it takes
/// two changes to break — restoring the full walk *and* dropping the early
/// return — which is exactly the shape the truncation could be undone in.
#[test]
fn a_miss_marks_the_world_and_a_hit_does_not() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();

    // Down at the ground, from five metres over it: nothing in the line
    // but the island, and it is inside the barrel's fifty.
    let mut alone = Box::new([Player::default(); MAX_PLAYERS]);
    let g = terrain::ground(SEED, hv(SEED), 600.0, 600.0);
    alone[0] = shooter(1, 600.0, g + 5.0, 600.0, 0, 0, 6);
    let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut alone);
    let marks: Vec<_> = s
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_IMPACT)
        .collect();
    assert_eq!(marks.len(), 1, "a shot into the ground marks the ground");
    assert_eq!(
        (marks[0].a >> 24) as u8,
        sim_core::ranged::SURF_GROUND,
        "and the surface byte says which"
    );

    // And into a body: a hitmarker, no decal. **On the ground**, so the
    // fifty metres past the victim are full of island — a shot that walked
    // its whole reach and reported both facts would land here, and in the
    // air it would not.
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = shooter(1, 600.0, g + 1.0, 600.0, 0, LEVEL, 6);
    players[1] = target(
        2,
        600.0,
        g + 1.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2,
        610.0,
    );
    let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    assert_eq!(
        s.events
            .entries()
            .iter()
            .filter(|e| e.code == EV_HIT)
            .count(),
        1,
        "one shot, one hitmarker"
    );
    assert!(
        !s.events.entries().iter().any(|e| e.code == EV_IMPACT),
        "a shot that found flesh must not also chip the scenery"
    );
}

/// No `EV_SHOT`, ever — and this is a gate on a *deliberate omission*
/// rather than an oversight, which is why it is asserted rather than left
/// to a reading of the code.
///
/// `EV_SHOT` carries a muzzle speed and a drop in mm/tick and the client
/// re-flies exactly those integers (`render/tracer.rs`). A hitscan has
/// neither, and a zero in both fields would hang a motionless tracer at the
/// muzzle for four seconds — a phantom arrow that cannot hit anyone, which
/// is worse than no tracer at all. The day a firearm earns a muzzle flash,
/// this test is the thing that has to be rewritten on purpose.
#[test]
fn a_firearm_draws_no_tracer() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();
    let mut players = face_off();
    let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
    assert!(
        !s.events.entries().iter().any(|e| e.code == EV_SHOT),
        "a hitscan shot has no speed and no drop, so it must not claim EV_SHOT's payload"
    );
    assert_eq!(players[1].hp, 80, "and it fired all the same");
}

/// The gun takes the arm in `draw` — so a revolver does not chop the tree
/// its owner is standing next to — and charges nothing for it.
///
/// The second half is the load-bearing one. `draw` and `hitscan` both read
/// the trigger and both know the cadence, so a firearm that paid in both
/// would fire at half its rate for reasons no number in `content/` would
/// explain.
#[test]
fn a_firearm_takes_the_arm_without_firing_or_charging_for_it() {
    let cc = gun_fixture();
    let mut arrows = Arrows::new();
    let mut p = shooter(1, 0.0, 400.0, 0.0, 0, LEVEL, 6);
    assert!(
        ranged::draw(0, &cc, &mut arrows, &mut EventQueue::default(), &mut p),
        "a gun in hand must take the arm"
    );
    assert!(arrows.is_empty(), "a hitscan shot is not an arrow");
    assert_eq!(p.inv[7].count, 6, "draw must not spend a round");
    assert_eq!(
        p.next_swing, 0,
        "draw must not pay a cadence it is not owed"
    );
}

/// A corpse and a sleeper do not shoot. `World::tick`'s player loop refuses
/// them the arm before `draw` is ever reached; this pass runs outside that
/// loop, so it has to refuse them itself — and a rule restated in two places
/// is a rule that can be dropped in one.
#[test]
fn a_corpse_and_a_sleeper_do_not_shoot() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();

    for (label, mark) in [
        (
            "a corpse",
            (|p: &mut Player| p.dead = true) as fn(&mut Player),
        ),
        ("a sleeper", |p: &mut Player| p.sleeping = true),
    ] {
        let mut players = face_off();
        mark(&mut players[0]);
        let s = pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
        assert_eq!(s.kills, 0);
        assert_eq!(players[1].hp, 100, "{label} pulled the trigger");
        assert_eq!(players[0].inv[7].count, 6, "{label} spent a round");
    }
}

/// The reach in `content/weapons.toml` is the reach the sim plays: a body
/// past the segment's end is not hit, and the same body inside it is.
#[test]
fn the_shot_stops_at_the_weapons_declared_reach() {
    let cc = gun_fixture();
    let mut sc = Scratch::barren();
    let eye = 400.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2;

    for (dist, expect) in [(45.0f32, 80u16), (55.0, 100)] {
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[0] = shooter(1, 0.0, 400.0, 0.0, 0, LEVEL, 6);
        players[1] = target(2, 0.0, eye, dist);
        pull(0, &cc, &ColIndex::new(), &mut sc.occupants(), &mut players);
        assert_eq!(
            players[1].hp, expect,
            "a body {dist} m down a 50 m barrel should be at {expect} hp"
        );
    }
}

/// `solid_deploy.rs`'s search, and its reason: the nearest cell to the
/// island centre whose terrain a foundation would accept, in ring order so
/// it is deterministic per seed.
fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                if build::foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells of centre");
}
