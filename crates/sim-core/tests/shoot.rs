//! Firing a bow, and the trunk that eats the shot.
//!
//! `tests/walk.rs` gates that the island stops a *body*. This suite is the
//! same claim about a *projectile*, and the two are not the same statement:
//! a body moves 0.18 m a tick and an arrow 1.33 m, so what stops one is a
//! predicate and what stops the other is a sampler.
//!
//! The load-bearing check is
//! `a_trunk_stops_the_shot_and_the_body_behind_it_lives`, and it is written
//! as a **pair**. Asserting only that the victim survives is the exact shape
//! of a gate that passes for the wrong reason — an arrow that never spawned,
//! never flew, or died on the ground would satisfy it just as well. So every
//! blocked shot is run twice over identical geometry, once against the real
//! island and once against `Scratch::barren()` where nothing blocks, and the
//! barren run must land the hit. The trunk is then the only difference
//! between a corpse and a survivor.
//!
//! Every check ABOVE the floor block at the end drives `ranged::draw` and
//! `ranged::step` with an EMPTY `ColIndex`, walk.rs's rule for the same
//! reason: nothing is built, so a wall can never take the credit for what a
//! tree did. The floor block inverts the fixture and says so in its own
//! header — it builds, and runs `Barren`, so a tree can never take the
//! credit for what a floor did.

// Measurements are this gate's output — same allow and same reason as
// `tests/walk.rs` and `tests/solid.rs`: the L5 wall bans format/print in SIM
// code, and a test harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::build::{
    self, BUILD_CELL_M, LEVEL_H_M, LOC_PLANE, PLATE_RISE_MAX_BANDS, SHAPE_FLOOR, SHAPE_FOUNDATION,
};
use sim_core::collide::{ColIndex, PLANE_THICKNESS_M};
// `fmath` only, never `f32::abs`: the walls' clippy list is crate-scoped, so
// it binds this suite exactly as it binds the sim (`tests/flank.rs` says the
// same). `i32::abs` is untouched by that list and stays as it is.
use sim_core::combat::{AmmoDef, CombatContent, RangedDef};
use sim_core::fmath::fabs;
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Barren, Occupants, Pristine, Scratch};
use sim_core::ranged::{self, Arrows, Kill, ARROW_EYE_MM, SURF_BUILT, SURF_GROUND};
use sim_core::spent::SpentArrows;
use sim_core::terrain::{self, Occupant, ScatterTable, Slot, CELLS_PER_SIDE};
use sim_core::world::{EventQueue, Player, EV_DEATH, EV_HIT, EV_IMPACT, EV_SHOT};

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

/// Same set and same rule as `walk.rs`: a seed that fails is a bug in the
/// generator, not a reroll.
const SEEDS: [u64; 4] = [0, 1, 7, 12345];

/// The item index the fixture makes a bow, and the one it makes its ammo.
/// Arbitrary and distinct; nothing else in the fixture uses either.
const BOW: u16 = 3;
const ARROW: u16 = 4;

/// Metres from the trunk to the shooter, and to the victim on the far side.
/// Far enough that the arrow is genuinely in flight for several ticks (8 m
/// at 40 m/s is six ticks), close enough that the drop over the crossing is
/// centimetres rather than the metres a 60 m shot sags.
const STANDOFF_M: f32 = 4.0;

/// A bow whose numbers are the real `content/weapons.toml` bow converted the
/// way `bake_ranged` converts it: 40 m/s and 20 m/s² at 30 Hz. Written out
/// rather than loaded so this suite gates the *flight*, and a content edit
/// cannot quietly turn a red here into a green.
fn bow_fixture() -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        hitscan: false,
        // 60 m, the real bow's reach. `ranged::draw` divides this by the
        // round's speed for the flight, which at 1333 mm/tick is the 45
        // ticks this fixture used to state as a constant.
        range_mm: 60_000,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
    };
    // The ballistics belong to the round now (`reference/PROJECTILES.md`
    // §9.3), so the fixture arms the arrow rather than the bow.
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 22,
    };
    c
}

/// A player standing at (x, feet_y, z), holding a drawn bow with `ammo`
/// arrows, facing `yaw` at `pitch`.
fn archer(id: u32, x: f32, feet_y: f32, z: f32, yaw: u16, pitch: u8, ammo: u16) -> Player {
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
        item: BOW,
        count: 1,
        cond: 0,
    };
    p.inv[7] = ItemStack {
        item: ARROW,
        count: ammo,
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

/// Level pitch. The client's encoding puts 0 rad between 127 and 128 and
/// there is no exact integer for it (`pitch_lut.rs`); 128 is the value the
/// client actually sends when you are looking at the horizon.
const LEVEL: u8 = 128;

/// The first tree this seed draws that is far enough from the haven pad for
/// the pad's own shelter and crates not to be what stopped the arrow.
/// Fixed scan order, so the fixture is reproducible. Mirrors `walk.rs::find`.
fn find_tree(seed: u64, table: &ScatterTable, haven: &terrain::Haven) -> Slot {
    let mut cz = 0;
    while cz < CELLS_PER_SIDE {
        let mut cx = 0;
        while cx < CELLS_PER_SIDE {
            let s = terrain::scatter(seed, table, haven, cx, cz);
            if s.occupant == Occupant::Tree {
                let (dx, dz) = (s.x - haven.x, s.z - haven.z);
                if dx * dx + dz * dz > 64.0 * 64.0 && line_clears_terrain(seed, &s) {
                    return s;
                }
            }
            cx += 1;
        }
        cz += 1;
    }
    panic!("seed {seed} drew no usable tree anywhere on the island");
}

/// Would a flat shot through this trunk clear the ground for its whole
/// length? A hill in the way would stop the arrow before the trunk did, and
/// the barren control would then fail for a reason that has nothing to do
/// with the claim. Checked when the fixture is chosen rather than asserted
/// later, so the suite selects a clean line instead of reporting a dirty one.
fn line_clears_terrain(seed: u64, s: &Slot) -> bool {
    let y = shot_line_y(s);
    // Sample every half metre from behind the shooter to past the victim.
    let mut i = -14;
    while i <= 14 {
        let z = s.z + i as f32 * 0.5;
        if terrain::height(seed, s.x, z) >= y - 0.6 {
            return false;
        }
        i += 1;
    }
    // And the trunk must actually cover that height: above its top the arrow
    // is supposed to sail over, which would make this a test of nothing.
    let (_, top) = terrain::occupant_volume(Occupant::Tree);
    y > s.y && y < s.y + top * s.scale
}

/// The height the arrow flies at: 2 m up the trunk, which is inside a 5.7 m
/// pine and clear of the ground either side.
fn shot_line_y(s: &Slot) -> f32 {
    s.y + 2.0
}

/// Fire one shot down +Z through `s` and fly it until it is gone, against
/// whatever occupant world `occ` describes. Returns the victim's hp and
/// whether the shot ever became an arrow at all.
fn shoot_through(
    seed: u64,
    occ: &mut Occupants,
    s: &Slot,
    back_m: f32,
    fwd_m: f32,
) -> (u16, bool, usize) {
    let cols = ColIndex::new();
    let y = shot_line_y(s);
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    // Feet placed so the muzzle sits exactly on the shot line.
    players[0] = archer(
        1,
        s.x,
        y - ARROW_EYE_MM as f32 / 1000.0,
        s.z - back_m,
        0,
        LEVEL,
        10,
    );
    // Feet placed so the line passes through the victim's chest, with room
    // for the sag over the crossing.
    players[1] = target(2, s.x, y - 1.2, s.z + fwd_m);

    let mut arrows = Arrows::new();
    let mut events;
    let cc = bow_fixture();

    let took = ranged::draw(
        0,
        &cc,
        &mut arrows,
        &mut EventQueue::default(),
        &mut players[0],
    );
    assert!(took, "a bow in hand must take the arm");
    let fired = arrows.len();

    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    // Arrow recovery's store. These fixtures predate it and assert
    // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
    // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
    // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
    let mut spent = SpentArrows::new();
    let mut ticks = 0;
    // Long enough for a 45-tick arrow to expire on its own if nothing ever
    // stops it — so "the store emptied" is never a timeout.
    while !arrows.is_empty() && ticks < 60 {
        events = EventQueue::default();
        ranged::step(
            seed,
            0,
            hv(seed),
            &cols,
            occ,
            &cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut events,
            &mut kills,
            &mut chips,
        );
        ticks += 1;
    }
    (players[1].hp, fired == 1, ticks)
}

/// The gate the judge asked for, in its own words: a shot stopped by a trunk
/// does not reach the body behind it.
#[test]
fn a_trunk_stops_the_shot_and_the_body_behind_it_lives() {
    for seed in SEEDS {
        let mut live = Scratch::with(seed, Pristine);
        let s = find_tree(seed, &live.table, &live.haven);

        let (hp_blocked, spawned, ticks) =
            shoot_through(seed, &mut live.occupants(), &s, STANDOFF_M, STANDOFF_M);
        assert!(spawned, "seed {seed}: the shot never became an arrow");
        assert_eq!(
            hp_blocked, 100,
            "seed {seed}: an arrow reached a body standing directly behind a \
             trunk ({ticks} ticks of flight)"
        );

        // The control, on the same geometry: with nothing in the world to
        // block, the same shot must land. Without this the assertion above
        // is satisfied by an arrow that never flew.
        let mut barren = Scratch::barren();
        let (hp_open, spawned_open, _) =
            shoot_through(seed, &mut barren.occupants(), &s, STANDOFF_M, STANDOFF_M);
        assert!(spawned_open, "seed {seed}: the control shot never spawned");
        // **85 and not 70, since the leg band landed (2026-08-30).** The
        // number moved because the shot did not: `bow_fixture`'s round
        // drops 22 mm/tick² and takes ~7.5 ticks to cross the standoff, so
        // it arrives roughly 0.6 m below the muzzle's line and lands under
        // `collide::LIMB_BAND_M` — a leg hit, and `limb_pct = 50` halves
        // the bow's 30. The fixture's geometry is deliberately untouched:
        // moving a target to keep a number is how a real behaviour change
        // gets hidden, and what this check needs is that the control shot
        // *landed*, which 85 says exactly as well as 70 did.
        assert_eq!(
            hp_open, 85,
            "seed {seed}: the control shot did not land, so the blocked case \
             proves nothing about the trunk"
        );
    }
}

/// The same claim at point-blank range behind the cover, which is a
/// *different* code path and the sharper one.
///
/// At 4 m back and 4 m on the far side the trunk stops the arrow three
/// ticks before the segment carrying the victim even begins — so the
/// blocked case above is decided by the arrow ceasing to exist, and the
/// comparison of where the world stopped it against where the body is is
/// never consulted. Mutating that comparison away leaves the test above
/// green, which is exactly the "gate that passes for the wrong reason" this
/// file's header is about.
///
/// A body pressed against the far side of the trunk falls inside the same
/// 1.33 m tick segment as the trunk itself, and then only the ordering
/// saves it. The standoff is swept rather than picked because where a tick
/// boundary lands depends on the 3 cm position quantum: over 21 offsets,
/// several put trunk and body in one segment, and every one of them must
/// still refuse the hit.
#[test]
fn a_body_pressed_against_the_far_side_of_the_trunk_lives() {
    for seed in SEEDS {
        let mut live = Scratch::with(seed, Pristine);
        let s = find_tree(seed, &live.table, &live.haven);
        // Just clear of the trunk radius plus a capsule, so the victim is
        // touching cover rather than standing inside it.
        let fwd = 1.0;
        let mut i = 0;
        while i < 21 {
            let back = 3.0 + i as f32 * 0.15;
            let (hp_blocked, spawned, _) =
                shoot_through(seed, &mut live.occupants(), &s, back, fwd);
            assert!(spawned, "seed {seed}: the shot never became an arrow");
            assert_eq!(
                hp_blocked, 100,
                "seed {seed}: an arrow reached a body pressed against the far \
                 side of a trunk from {back:.2} m back"
            );

            let mut barren = Scratch::barren();
            let (hp_open, _, _) = shoot_through(seed, &mut barren.occupants(), &s, back, fwd);
            assert_eq!(
                hp_open, 70,
                "seed {seed}: the control shot from {back:.2} m did not land, \
                 so the blocked case proves nothing"
            );
            i += 1;
        }
    }
}

/// A shot with nothing in the way hits, damages by the weapon's number, and
/// says so on the event lane.
#[test]
fn an_arrow_in_the_open_lands_and_is_announced() {
    let seed = SEEDS[0];
    let cols = ColIndex::new();
    let mut sc = Scratch::barren();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, 0.0, 100.0, 0.0, 0, LEVEL, 5);
    players[1] = target(2, 0.0, 100.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2, 10.0);

    let mut arrows = Arrows::new();
    let mut events;
    let cc = bow_fixture();
    ranged::draw(
        0,
        &cc,
        &mut arrows,
        &mut EventQueue::default(),
        &mut players[0],
    );

    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    // Arrow recovery's store. These fixtures predate it and assert
    // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
    // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
    // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
    let mut spent = SpentArrows::new();
    let mut hits = 0;
    for _ in 0..40 {
        events = EventQueue::default();
        ranged::step(
            seed,
            0,
            hv(seed),
            &cols,
            &mut sc.occupants(),
            &cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut events,
            &mut kills,
            &mut chips,
        );
        hits += events.entries().iter().filter(|e| e.code == EV_HIT).count();
    }
    // The bow's 30, halved by `limb_pct` — see the note in
    // `a_trunk_stops_the_shot_and_the_body_behind_it_lives`: this fixture's
    // arrow drops about 0.6 m over the 10 m and lands on the legs.
    assert_eq!(players[1].hp, 85, "the arrow did not take its half of 30");
    assert_eq!(hits, 1, "exactly one EV_HIT for one arrow");
}

/// Four arrows kill a hundred-point body, and the fourth reports a death
/// with the bow as the weapon — the death screen's "who, with what".
#[test]
fn four_arrows_kill_and_the_kill_names_the_bow() {
    let seed = SEEDS[0];
    let cols = ColIndex::new();
    let mut sc = Scratch::barren();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, 0.0, 100.0, 0.0, 0, LEVEL, 10);
    players[1] = target(2, 0.0, 100.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2, 10.0);

    let mut arrows = Arrows::new();
    let mut events;
    let cc = bow_fixture();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    // Arrow recovery's store. These fixtures predate it and assert
    // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
    // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
    // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
    let mut spent = SpentArrows::new();
    let mut deaths = 0;
    let mut killed: Option<Kill> = None;

    for t in 0..400u64 {
        events = EventQueue::default();
        ranged::draw(
            t,
            &cc,
            &mut arrows,
            &mut EventQueue::default(),
            &mut players[0],
        );
        let (n, _n_chips) = ranged::step(
            seed,
            0,
            hv(seed),
            &cols,
            &mut sc.occupants(),
            &cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut events,
            &mut kills,
            &mut chips,
        );
        deaths += events
            .entries()
            .iter()
            .filter(|e| e.code == EV_DEATH)
            .count();
        if n > 0 {
            killed = Some(kills[0]);
            break;
        }
    }
    let k = killed.expect("four arrows at 30 did not kill a 100 hp body");
    assert_eq!(deaths, 1, "one death, announced once");
    assert_eq!(k.victim, 1);
    assert_eq!(k.by, 1, "the kill is credited to the shooter's id");
    assert_eq!(k.item, BOW, "the death screen names the bow, not the arrow");
    assert!(
        k.range_cm > 900 && k.range_cm < 1100,
        "a 10 m shot reported {} cm",
        k.range_cm
    );
    assert_eq!(players[1].hp, 0);
    assert_eq!(players[1].deaths, 1);
}

/// An empty quiver fires nothing, and the arm is still the bow's — the
/// caller must not fall through to a melee swing because the shot failed.
#[test]
fn an_empty_quiver_fires_nothing_and_still_takes_the_arm() {
    let mut arrows = Arrows::new();
    let cc = bow_fixture();
    let mut p = archer(1, 0.0, 50.0, 0.0, 0, LEVEL, 0);
    p.inv[7] = ItemStack::default();
    assert!(
        ranged::draw(0, &cc, &mut arrows, &mut EventQueue::default(), &mut p),
        "the bow keeps the arm"
    );
    assert_eq!(arrows.len(), 0, "no ammo, no arrow");
}

/// A hand holding anything else hands the arm back, so the gather-and-melee
/// path runs exactly as it did before this module existed.
#[test]
fn an_empty_hand_hands_the_arm_back() {
    let mut arrows = Arrows::new();
    let cc = bow_fixture();
    let mut p = archer(1, 0.0, 50.0, 0.0, 0, LEVEL, 10);
    p.inv[0] = ItemStack {
        item: 9,
        count: 1,
        cond: 0,
    };
    assert!(!ranged::draw(
        0,
        &cc,
        &mut arrows,
        &mut EventQueue::default(),
        &mut p
    ));
    p.inv[0] = ItemStack::default();
    assert!(!ranged::draw(
        0,
        &cc,
        &mut arrows,
        &mut EventQueue::default(),
        &mut p
    ));
    assert_eq!(arrows.len(), 0);
}

/// One shot spends exactly one arrow, and the cadence is the weapon's own
/// rather than the shared melee interval.
#[test]
fn a_shot_spends_one_arrow_at_the_weapons_own_cadence() {
    let mut arrows = Arrows::new();
    let cc = bow_fixture();
    let mut p = archer(1, 0.0, 50.0, 0.0, 0, LEVEL, 10);

    ranged::draw(0, &cc, &mut arrows, &mut EventQueue::default(), &mut p);
    assert_eq!(arrows.len(), 1);
    assert_eq!(p.inv[7].count, 9, "one shot, one arrow");
    assert_eq!(
        p.next_swing, 60,
        "the bow's 30/min is 60 ticks, not the melee 38"
    );

    // Held down, every tick, until the cadence comes round.
    for t in 1..60u64 {
        ranged::draw(t, &cc, &mut arrows, &mut EventQueue::default(), &mut p);
    }
    assert_eq!(arrows.len(), 1, "the cadence held the second shot");
    assert_eq!(p.inv[7].count, 9);
    ranged::draw(60, &cc, &mut arrows, &mut EventQueue::default(), &mut p);
    assert_eq!(arrows.len(), 2, "and released it on the sixtieth tick");
    assert_eq!(p.inv[7].count, 8);
}

/// `EV_SHOT` announces exactly the arrows that exist, carrying the five
/// numbers a tracer needs to redraw the sim's own arc.
///
/// **The pairing is the point, and it is `shoot_through`'s discipline one
/// level down.** Asserting only that a shot emits an event would pass for a
/// module that announced every button press, and a phantom tracer for a
/// refused shot is worse than no tracer — it draws an arrow that does not
/// exist and cannot hit anyone. So every refusal path is asserted silent in
/// the same test that asserts the fire path speaks.
#[test]
fn every_arrow_is_announced_and_nothing_else_is() {
    let cc = bow_fixture();
    let (yaw, pitch) = (12_345u16, 200u8);

    // A shot that lands in the store speaks, once, with the round's own
    // ballistics rather than the weapon's — the §9.3 move, seen from the
    // wire's end.
    let mut arrows = Arrows::new();
    let mut ev = EventQueue::default();
    let mut p = archer(7, 0.0, 50.0, 0.0, yaw, pitch, 10);
    ranged::draw(0, &cc, &mut arrows, &mut ev, &mut p);
    assert_eq!(arrows.len(), 1, "the shot became an arrow");
    let shots: Vec<_> = ev.entries().iter().filter(|e| e.code == EV_SHOT).collect();
    assert_eq!(shots.len(), 1, "one arrow, one announcement");
    assert_eq!(shots[0].a, 7, "a = the shooter");
    assert_eq!(
        shots[0].b,
        (yaw as u32) << 8 | pitch as u32,
        "b = yaw << 8 | pitch"
    );
    let ball = cc.ammo_def(ARROW).expect("the fixture arms the round");
    assert_eq!(
        shots[0].c,
        (ball.speed_mmpt as u32) << 16 | ball.drop_mmpt2 as u32,
        "c = the ROUND's speed and drop, so a tracer flies the arc the sim flew"
    );

    // Every path that returns without an arrow must say nothing. Cadence:
    let mut ev = EventQueue::default();
    ranged::draw(1, &cc, &mut arrows, &mut ev, &mut p);
    assert_eq!(arrows.len(), 1, "the cadence held it");
    assert!(
        !ev.entries().iter().any(|e| e.code == EV_SHOT),
        "a shot refused for cadence must not draw a tracer"
    );

    // An empty quiver:
    let mut ev = EventQueue::default();
    let mut empty = archer(8, 0.0, 50.0, 0.0, yaw, pitch, 0);
    empty.inv[7] = ItemStack::default();
    ranged::draw(0, &cc, &mut arrows, &mut ev, &mut empty);
    assert!(
        !ev.entries().iter().any(|e| e.code == EV_SHOT),
        "an empty quiver must not draw a tracer"
    );

    // And a full store, which is the one refusal that happens *after* the
    // cadence is paid, so it is the easiest to announce by accident.
    let mut full = Arrows::new();
    let mut stuffer = archer(9, 0.0, 50.0, 0.0, yaw, pitch, u16::MAX);
    for t in 0..MAX_ARROWS as u64 {
        ranged::draw(
            t * 60,
            &cc,
            &mut full,
            &mut EventQueue::default(),
            &mut stuffer,
        );
    }
    assert_eq!(full.len(), MAX_ARROWS, "the store is full");
    let mut ev = EventQueue::default();
    ranged::draw(
        MAX_ARROWS as u64 * 60,
        &cc,
        &mut full,
        &mut ev,
        &mut stuffer,
    );
    assert!(
        !ev.entries().iter().any(|e| e.code == EV_SHOT),
        "a shot refused by a full store must not draw a tracer"
    );
}

/// A full store refuses the shot and the ammo stays in the quiver. The
/// overflow policy `MAX_ARROWS` states, asserted rather than asserted-about.
#[test]
fn a_full_store_refuses_the_shot_and_keeps_the_ammo() {
    let mut arrows = Arrows::new();
    let cc = bow_fixture();
    let mut p = archer(1, 0.0, 50.0, 0.0, 0, LEVEL, 999);

    let mut t = 0u64;
    while arrows.len() < MAX_ARROWS {
        ranged::draw(t, &cc, &mut arrows, &mut EventQueue::default(), &mut p);
        t += 60;
    }
    assert_eq!(arrows.len(), MAX_ARROWS);
    let ammo_before = p.inv[7].count;
    ranged::draw(t, &cc, &mut arrows, &mut EventQueue::default(), &mut p);
    assert_eq!(
        arrows.len(),
        MAX_ARROWS,
        "the store did not grow past its cap"
    );
    assert_eq!(
        p.inv[7].count, ammo_before,
        "a refused shot must cost the shooter nothing"
    );
}

/// Pitch aims the shot. Before this slice `frame.pitch` was transmitted,
/// interpolated and hashed while no sim rule read it (`gather.rs`'s "aim is
/// planar in v0"); a bow that could not be aimed up would be a bow that
/// cannot be used, because the drop is real.
#[test]
fn pitch_aims_the_shot() {
    let cc = bow_fixture();
    let mut up = Arrows::new();
    let mut level = Arrows::new();
    let mut down = Arrows::new();
    ranged::draw(
        0,
        &cc,
        &mut up,
        &mut EventQueue::default(),
        &mut archer(1, 0.0, 50.0, 0.0, 0, 200, 5),
    );
    ranged::draw(
        0,
        &cc,
        &mut level,
        &mut EventQueue::default(),
        &mut archer(1, 0.0, 50.0, 0.0, 0, LEVEL, 5),
    );
    ranged::draw(
        0,
        &cc,
        &mut down,
        &mut EventQueue::default(),
        &mut archer(1, 0.0, 50.0, 0.0, 0, 40, 5),
    );

    let vy = |a: &Arrows| a.entries().next().unwrap().vy;
    assert!(vy(&up) > 500, "looking up must launch upward: {}", vy(&up));
    assert!(
        vy(&down) < -500,
        "looking down must launch downward: {}",
        vy(&down)
    );
    assert!(
        vy(&level).abs() < 50,
        "level must launch nearly flat: {}",
        vy(&level)
    );

    // And the planar speed shrinks as the shot steepens — the cosine, not a
    // second full-speed component bolted onto the vertical one.
    let vz = |a: &Arrows| a.entries().next().unwrap().vz;
    assert!(
        vz(&level) > vz(&up) && vz(&up) > 0,
        "a steep shot must travel less far downrange per tick"
    );
}

/// An arrow never hits the person who fired it, however the geometry lands.
#[test]
fn an_arrow_never_hits_its_owner() {
    let seed = SEEDS[0];
    let cols = ColIndex::new();
    let mut sc = Scratch::barren();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    // Fired straight down, from inside the shooter's own capsule.
    players[0] = archer(1, 0.0, 100.0, 0.0, 0, 0, 5);

    let mut arrows = Arrows::new();
    let mut events;
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    // Arrow recovery's store. These fixtures predate it and assert
    // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
    // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
    // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
    let mut spent = SpentArrows::new();
    ranged::draw(
        0,
        &bow_fixture(),
        &mut arrows,
        &mut EventQueue::default(),
        &mut players[0],
    );
    for _ in 0..10 {
        events = EventQueue::default();
        ranged::step(
            seed,
            0,
            hv(seed),
            &cols,
            &mut sc.occupants(),
            &bow_fixture(),
            &mut arrows,
            &mut spent,
            &mut players,
            &mut events,
            &mut kills,
            &mut chips,
        );
    }
    assert_eq!(players[0].hp, 100, "an archer shot themselves");
}

/// The ground stops an arrow, and it leaves the store — the backstop that
/// makes `MAX_ARROWS` a bound on occupancy rather than a leak.
#[test]
fn the_ground_stops_an_arrow_and_the_store_drains() {
    for seed in SEEDS {
        let cols = ColIndex::new();
        let mut sc = Scratch::with(seed, Pristine);
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        // Aimed steeply down from head height over real terrain.
        let (x, z) = (sc.haven.x + 200.0, sc.haven.z + 200.0);
        let y = terrain::height(seed, x, z);
        players[0] = archer(1, x, y + 3.0, z, 0, 40, 5);

        let mut arrows = Arrows::new();
        let mut events;
        let mut kills = [Kill::default(); MAX_ARROWS];
        let mut chips = [ranged::Chip::default(); MAX_ARROWS];
        // Arrow recovery's store. These fixtures predate it and assert
        // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
        // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
        // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
        let mut spent = SpentArrows::new();
        ranged::draw(
            0,
            &bow_fixture(),
            &mut arrows,
            &mut EventQueue::default(),
            &mut players[0],
        );
        assert_eq!(arrows.len(), 1);
        for _ in 0..60 {
            events = EventQueue::default();
            ranged::step(
                seed,
                0,
                hv(seed),
                &cols,
                &mut sc.occupants(),
                &bow_fixture(),
                &mut arrows,
                &mut spent,
                &mut players,
                &mut events,
                &mut kills,
                &mut chips,
            );
        }
        assert!(
            arrows.is_empty(),
            "seed {seed}: an arrow fired into the ground is still in the air"
        );
    }
}

/// Two runs of the same shot over the same seed produce the same flight,
/// step for step. Wall 5 for the new state: the replay golden covers a
/// script that fires nothing, so this is where a projectile's determinism is
/// actually asserted.
#[test]
fn the_same_shot_flies_the_same_path_twice() {
    let seed = SEEDS[2];
    let path = |_: u8| -> Vec<(i32, i32, i32)> {
        let cols = ColIndex::new();
        let mut sc = Scratch::with(seed, Pristine);
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        let (x, z) = (sc.haven.x + 300.0, sc.haven.z - 150.0);
        let y = terrain::height(seed, x, z);
        players[0] = archer(1, x, y + 2.0, z, 0x2A00, 170, 5);
        let mut arrows = Arrows::new();
        let mut events;
        let mut kills = [Kill::default(); MAX_ARROWS];
        let mut chips = [ranged::Chip::default(); MAX_ARROWS];
        // Arrow recovery's store. These fixtures predate it and assert
        // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
        // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
        // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
        let mut spent = SpentArrows::new();
        ranged::draw(
            0,
            &bow_fixture(),
            &mut arrows,
            &mut EventQueue::default(),
            &mut players[0],
        );
        let mut out = Vec::new();
        for _ in 0..45 {
            events = EventQueue::default();
            ranged::step(
                seed,
                0,
                hv(seed),
                &cols,
                &mut sc.occupants(),
                &bow_fixture(),
                &mut arrows,
                &mut spent,
                &mut players,
                &mut events,
                &mut kills,
                &mut chips,
            );
            if let Some(a) = arrows.entries().next() {
                out.push((a.qx, a.qy, a.qz));
            }
        }
        out
    };
    let a = path(0);
    let b = path(1);
    assert!(
        a.len() > 3,
        "the fixture shot barely flew: {} steps",
        a.len()
    );
    assert_eq!(a, b, "the same shot took two different paths");
}

/// An arrow enters the state hash while it is in the air and leaves no trace
/// behind it — both halves, because only the pair is the claim.
///
/// The "leaves nothing" half is why `GOLDEN_FINAL_HASH` did not have to be
/// regenerated for this slice: `state_hash` folds arrows on the **player**
/// idiom (skip-if-inactive, no length prefix), unlike every store beside it,
/// each of which contributes eight zero bytes when empty. So the pinned
/// replay golden stays evidence that nothing this slice touched changed a
/// path the script walks. The "enters" half is what stops that from being
/// achieved by simply not hashing arrows at all — which would satisfy the
/// first assertion perfectly and put a projectile outside wall 5.
#[test]
fn an_arrow_is_in_the_state_hash_only_while_it_is_in_the_air() {
    let a = sim_core::world::World::new(7);
    let mut b = sim_core::world::World::new(7);
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "two fresh worlds of the same seed must agree"
    );

    let mut p = archer(1, 0.0, 50.0, 0.0, 0, LEVEL, 5);
    ranged::draw(
        0,
        &bow_fixture(),
        &mut b.arrows,
        &mut EventQueue::default(),
        &mut p,
    );
    assert_eq!(b.arrows.len(), 1, "the fixture shot did not spawn");
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "an arrow in the air is state, and a hash that cannot see it puts \
         projectiles outside wall 5"
    );

    *b.arrows = Arrows::new();
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "an emptied arrow store must fold to nothing at all"
    );
}

/// `Barren` is what the control shots above rely on. If it ever started
/// blocking, every paired assertion in this file would still pass and prove
/// nothing — so the fixture itself is gated.
#[test]
fn the_barren_fixture_really_blocks_nothing() {
    let mut sc = Scratch::barren();
    let mut occ = sc.occupants();
    let _: &mut Occupants = &mut occ;
    for i in 0..40 {
        let v = i as f32 * 7.0;
        assert!(
            !occ.blocks_volume(SEEDS[0], v, v, 50.0, 0.05, 0.05),
            "the barren fixture blocked at ({v}, {v})"
        );
    }
    let _ = Barren;
}

// ---------------------------------------------------------------------------
// A floor eats the shot (shot planes v0)
//
// Everything above this line drives an EMPTY `ColIndex`, so that nothing
// built can take the credit for what a tree did. These four are the inverse
// claim and need the opposite fixture: the occupant table is `Barren` — no
// trees, no boulders, no crates — so a shot that stops has been stopped by
// the BASE, exactly as `tests/flank.rs` reads it for a body.
//
// **The defect: `collide::shot_blocked` consulted no plane at all.** It
// walked edges and diagonals, so every floor, roof and foundation on the
// island was transparent to a projectile — an arrow fired down inside a base
// reached the dirt under it and reported `SURF_GROUND`, and a roof was cover
// you could see through. The body walk has read these bits since piece flanks
// v0 (`tests/flank.rs`); only the shot walk had not.

/// Where the floor fixtures start looking for a site to build on.
const FCX: u16 = 341;
const FCZ: u16 = 341;

/// The first run of `cells` columns from (`FCX`, `FCZ`) along +z that a base
/// could actually sit flush on — every column within `PLATE_RISE_MAX_BANDS`
/// of the anchor's terrain band.
///
/// **A scan, not a constant, and it is `find_tree`'s pattern for its reason.**
/// A hard-coded column is a claim about four different islands: on `SEEDS[1]`
/// the third cell of the corridor stands 2 m under the first, so the fixture
/// was a staircase and a shot flying inside the storey met the top face of
/// the next cell's floor. Fixed scan order, so the site is reproducible.
fn flat_run(seed: u64, cells: u16) -> (u16, u16) {
    for cz in FCZ..FCZ + 64 {
        let anchor = build::terrain_band(seed, hv(seed), FCX, cz);
        let flat = (0..cells).all(|d| {
            (build::terrain_band(seed, hv(seed), FCX, cz + d) - anchor).abs()
                <= PLATE_RISE_MAX_BANDS
        });
        if flat {
            return (FCX, cz);
        }
    }
    panic!("seed {seed}: no run of {cells} flush-able columns near ({FCX}, {FCZ})");
}

/// A foundation at level 0 and a floor at level 1, over `cells` columns
/// running **+z** from (`FCX`, `FCZ`) — put on the index directly, `flank.rs`'
/// route and its reason: where a piece comes from is `tests/plate.rs`' and
/// `tests/base_lattice.rs`' subject, and what one DOES to a shot is this
/// one's.
///
/// **+z because yaw 0 is +z**, and the first draft of these fixtures laid
/// them along +x while firing yaw 0. The corridor check below went green on
/// an arrow that flew off at right angles to the corridor and never met a
/// slab — a gate satisfied by geometry it was not looking at, which is the
/// shape `tests/lattice.rs`' header warns about. `pitch_aims_the_shot` is
/// where the LUT's bearing is written down.
///
/// **FLUSH by construction**, which is `build::plate_for`'s whole job and has
/// to be done by hand here because the index is written directly. Every
/// column gets the plate that puts its level-0 floor on the anchor's band, so
/// the corridor is one height instead of a staircase. Without it the fixture
/// steps with the terrain, and on `SEEDS[1]` a later foundation stood 1.2 m
/// proud of the first — so a shot flying *inside* the storey met the top face
/// of the next cell's floor and the corridor check failed on real geometry it
/// was never trying to describe.
///
/// Boxed because `ColIndex` is a large fixed array and building one in a
/// stack frame is CLAUDE.md's wasm shadow-stack trap.
fn storeys(seed: u64, cells: u16) -> (Box<ColIndex>, u16, u16, f32) {
    let (bcx, bcz) = flat_run(seed, cells);
    let mut cols = Box::new(ColIndex::new());
    let anchor = build::terrain_band(seed, hv(seed), bcx, bcz);
    let base = build::band_y(anchor);
    for d in 0..cells {
        let want = anchor - build::terrain_band(seed, hv(seed), bcx, bcz + d);
        cols.add(bcx, bcz + d, 0, LOC_PLANE, SHAPE_FOUNDATION, want as i8);
        cols.add(bcx, bcz + d, 1, LOC_PLANE, SHAPE_FLOOR, want as i8);
        assert_eq!(
            build::column_floor_y(seed, hv(seed), bcx, bcz + d, want as i8),
            base,
            "seed {seed}: cell +{d} did not land flush with the anchor"
        );
    }
    (cols, bcx, bcz, base)
}

/// The centre of build cell (cx, cz) in world XZ.
fn cell_centre(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    )
}

/// Fire one arrow from `feet` at `pitch` and report the impact it announced:
/// `(surf, y)`, or `None` if it never stopped.
///
/// Reads `EV_IMPACT` rather than the arrow store, because the impact is the
/// statement that crosses the wire and draws the mark — an arrow that
/// vanished and an arrow that landed are the same empty store.
fn impact_of(
    seed: u64,
    cols: &ColIndex,
    at: (f32, f32, f32),
    pitch: u8,
    ticks: u32,
) -> Option<(u8, f32)> {
    let mut sc = Scratch::barren();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, at.0, at.1, at.2, 0, pitch, 5);
    let cc = bow_fixture();
    let mut arrows = Arrows::new();
    assert!(
        ranged::draw(
            0,
            &cc,
            &mut arrows,
            &mut EventQueue::default(),
            &mut players[0],
        ),
        "a bow in hand must take the arm"
    );

    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    // Arrow recovery's store. These fixtures predate it and assert
    // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
    // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
    // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
    let mut spent = SpentArrows::new();
    for _ in 0..ticks {
        let mut events = EventQueue::default();
        ranged::step(
            seed,
            0,
            hv(seed),
            cols,
            &mut sc.occupants(),
            &cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut events,
            &mut kills,
            &mut chips,
        );
        if let Some(e) = events.entries().iter().find(|e| e.code == EV_IMPACT) {
            // `world.rs`' own role line: a = SURF_* << 24 | x, b = z,
            // c = y in POS_Y_Q quanta as a signed i32 reinterpreted.
            return Some(((e.a >> 24) as u8, e.c as i32 as f32 * POS_Y_Q));
        }
    }
    None
}

/// An arrow fired straight down inside a base stops on the floor it was
/// fired from, and does not reach the dirt.
///
/// **Written as a pair, `a_trunk_stops_the_shot_and_the_body_behind_it_lives`'
/// rule and its reason.** Asserting only "it stopped on something built" is
/// satisfied by an arrow that never spawned or died on the ground; so the
/// identical shot is run twice over identical geometry, once with the base on
/// the index and once with an empty one, and the empty run must reach the
/// terrain. The floor is then the only difference between the two impacts.
#[test]
fn a_floor_stops_a_shot_fired_down_through_it() {
    for seed in SEEDS {
        let (cols, bcx, bcz, base) = storeys(seed, 1);
        let (cx, cz) = cell_centre(bcx, bcz);
        // Standing on the level-1 floor, firing at its own feet.
        let feet = base + LEVEL_H_M;
        let ground = terrain::ground(seed, hv(seed), cx, cz);
        assert!(
            feet - ground > LEVEL_H_M,
            "seed {seed}: the fixture's storey is not clear of the ground, so \
             nothing distinguishes the two impacts"
        );

        let (surf, y) = impact_of(seed, &cols, (cx, feet, cz), 0, 20)
            .expect("seed {seed}: the shot never stopped on anything");
        assert_eq!(
            surf, SURF_BUILT,
            "seed {seed}: an arrow fired down inside a base reported surface \
             {surf} at y={y:.2} — the floor at {feet:.2} is transparent to it \
             (ground is {ground:.2})"
        );
        assert!(
            fabs(y - feet) <= PLANE_THICKNESS_M + 0.2,
            "seed {seed}: it stopped at y={y:.2}, which is not the floor at \
             {feet:.2}"
        );

        // The control: the same shot with nothing built must reach the dirt.
        let empty = ColIndex::new();
        let (csurf, cy) = impact_of(seed, &empty, (cx, feet, cz), 0, 20)
            .expect("seed {seed}: the control shot never stopped either");
        assert_eq!(
            csurf, SURF_GROUND,
            "seed {seed}: the control stopped on {csurf}, so the assertion \
             above is not about the floor"
        );
        assert!(
            fabs(cy - ground) < 1.0,
            "seed {seed}: the control stopped at y={cy:.2}, not the ground at \
             {ground:.2}"
        );
    }
}

/// A roof is cover: an arrow fired up from under a floor stops on its
/// underside rather than passing through it.
///
/// **The mirror of the test above, and it is not the same assertion.** The
/// downward shot only proves the slab's top face; the band a plane presents
/// runs `PLANE_THICKNESS_M` below that, and a walk that read only "am I under
/// the top" would pass this and let every roof on the island be shot through
/// from inside. Paired with a barren control for the same reason.
#[test]
fn a_roof_stops_a_shot_fired_up_at_it() {
    for seed in SEEDS {
        let (cols, bcx, bcz, base) = storeys(seed, 1);
        let (cx, cz) = cell_centre(bcx, bcz);
        // Standing on the foundation, under the level-1 floor.
        let feet = base;
        let under = base + LEVEL_H_M - PLANE_THICKNESS_M;
        assert!(
            feet + ARROW_EYE_MM as f32 / 1000.0 < under,
            "seed {seed}: the muzzle is already inside the slab, so the shot \
             has nothing to travel through"
        );

        // 255 is the steepest up the pitch encoding holds.
        let (surf, y) = impact_of(seed, &cols, (cx, feet, cz), 255, 20)
            .expect("seed {seed}: the shot never stopped on anything");
        assert_eq!(
            surf, SURF_BUILT,
            "seed {seed}: an arrow fired up at a roof reported surface {surf} \
             at y={y:.2} — the slab's underside at {under:.2} let it through"
        );

        // The control, and it reads "not built" rather than "the dirt" for a
        // measured reason: fired up, this fixture's arrow **outlives its own
        // range**. The bow reaches 60 m at 1333 mm/tick, so the store frees
        // the slot after 45 ticks of flight, and a shot at this angle is
        // still airborne then — it announces no impact at all. Demanding
        // `SURF_GROUND` here would be demanding the arrow come down, which is
        // a claim about `range_mm` and not about the roof.
        let empty = ColIndex::new();
        let control = impact_of(seed, &empty, (cx, feet, cz), 255, 90);
        assert!(
            !matches!(control, Some((SURF_BUILT, _))),
            "seed {seed}: the control stopped on a built surface with an empty \
             index ({control:?}), so the assertion above is not about the roof"
        );
    }
}

/// A storey you can shoot ACROSS: an arrow fired level under a floor travels
/// the whole corridor instead of stopping on the slab over its head.
///
/// **The assertion that stops this slice from being a wall.** A plane test
/// that forgot the air under a slab — the `level > 0` half of the band —
/// would make every base a solid block, and both tests above would still
/// pass. It is `flank.rs::a_plate_stays_walkable_end_to_end`'s claim for the
/// other mover.
#[test]
fn a_shot_travels_under_a_floor_the_whole_length_of_it() {
    const CELLS: u16 = 4;
    for seed in SEEDS {
        let (cols, bcx, bcz, base) = storeys(seed, CELLS);
        let (cx, cz) = cell_centre(bcx, bcz);
        let eye = base + ARROW_EYE_MM as f32 / 1000.0;
        // The gap it must fly through: over the foundation, under the slab.
        assert!(
            eye > base && eye < base + LEVEL_H_M - PLANE_THICKNESS_M,
            "seed {seed}: the muzzle at {eye:.2} is not inside the storey"
        );

        // Fired down the corridor: yaw 0 is +z, `storeys` lays its cells
        // along +z, and the archer stands one cell short of the first of
        // them. The feet go a muzzle-height below the line so the shot flies
        // inside the storey rather than over the roof — `archer` puts the
        // muzzle at `ARROW_EYE_MM` above the feet.
        let far = (bcz + CELLS) as f32 * BUILD_CELL_M;
        let from = (cx, base, cz - BUILD_CELL_M);
        let hit = impact_of(seed, &cols, from, LEVEL, 40);
        // Level flight sags, so it lands on something eventually — what it
        // must NOT do is stop on the ceiling it is flying under.
        if let Some((surf, y)) = hit {
            assert!(
                surf != SURF_BUILT || y <= base + PLANE_THICKNESS_M,
                "seed {seed}: a level shot under the storey stopped on a built \
                 surface at y={y:.2}; the slab it should have passed under \
                 spans {:.2}..{:.2} (corridor ends at z={far:.2})",
                base + LEVEL_H_M - PLANE_THICKNESS_M,
                base + LEVEL_H_M
            );
        }
    }
}

/// An arrow flying OVER a base is not stopped by the roof it is clearing.
///
/// **The third open band, and the only mutant the other four all survive.**
/// The walk's first question is "am I above this slab's top"; delete it and
/// level 0 answers `true` for every altitude in the sky over every foundation
/// on the island, which is an invisible ceiling the whole width of a base.
/// The corridor check cannot see it — it flies *under* — so the two of them
/// together are what say the plane is a band rather than a half-space.
#[test]
fn a_shot_flies_over_a_roof_without_stopping_on_it() {
    for seed in SEEDS {
        let (cols, bcx, bcz, base) = storeys(seed, 1);
        let (cx, cz) = cell_centre(bcx, bcz);
        // A muzzle a clear metre over the roof, fired level across it.
        let over = base + LEVEL_H_M + 1.0;
        let from = (cx, over - ARROW_EYE_MM as f32 / 1000.0, cz - BUILD_CELL_M);

        let hit = impact_of(seed, &cols, from, LEVEL, 40);
        assert!(
            !matches!(hit, Some((SURF_BUILT, _))),
            "seed {seed}: a shot passing {:.2} m over the roof at {:.2} stopped \
             on it ({hit:?}) — the slab is a half-space, not a band",
            over - (base + LEVEL_H_M),
            base + LEVEL_H_M
        );
    }
}

/// The foundation is solid to the ground, and an arrow meets the skirt.
///
/// **The `level == 0` half of the band, which nothing else here reaches.** A
/// stilted foundation carries up to a storey of leg (build plate v1) and the
/// renderer draws a skirt down its whole height; a walk that gave level 0 the
/// same 0.3 m slab every upper floor gets would let an arrow through that leg
/// while `plane_blocked` stops a body walking into it — the two movers
/// disagreeing about one volume, which is the drift the pair exists to
/// prevent.
#[test]
fn a_stilted_foundation_stops_a_shot_through_its_skirt() {
    let seed = SEEDS[0];
    let plate = PLATE_RISE_MAX_BANDS as i8;
    let mut cols = Box::new(ColIndex::new());
    cols.add(FCX, FCZ, 0, LOC_PLANE, SHAPE_FOUNDATION, plate);
    let (cx, cz) = cell_centre(FCX, FCZ);
    let top = build::column_floor_y(seed, hv(seed), FCX, FCZ, plate);
    let ground = build::column_floor_y(seed, hv(seed), FCX, FCZ, 0);
    assert!(
        top - ground > PLANE_THICKNESS_M * 2.0,
        "the fixture's stilt is {:.2} m — shorter than the slab, so a level-0 \
         rule that forgot the skirt would still pass",
        top - ground
    );

    // Fired down from over the plate: it must stop on the plate's top, not
    // on the terrain the leg is standing in.
    let (surf, y) = impact_of(seed, &cols, (cx, top, cz), 0, 20).expect("the shot never stopped");
    assert_eq!(surf, SURF_BUILT, "the plate's top let the arrow through");
    assert!(
        fabs(y - top) <= PLANE_THICKNESS_M + 0.2,
        "it stopped at y={y:.2}, not on the plate at {top:.2}"
    );

    // And into the LEG from the side, at an altitude inside the skirt: the
    // half a downward shot cannot ask, because the top face answers it first.
    //
    // The feet go a muzzle-height BELOW the line the shot must fly, which is
    // the whole of what the first draft got wrong: `archer` puts the muzzle at
    // `ARROW_EYE_MM` over the feet, so standing the archer AT the line fired
    // it 1.6 m high and straight over a plate 1.5 m tall.
    let mid = (top + ground) * 0.5;
    let from = (
        cx,
        mid - ARROW_EYE_MM as f32 / 1000.0,
        cz - BUILD_CELL_M * 1.5,
    );
    // The line clears the dirt for its whole run, so the terrain can never be
    // what stopped it — `line_clears_terrain`'s claim, made against the path
    // this shot actually takes.
    for i in 0..=20 {
        let z = from.2 + (cz - from.2) * (i as f32 / 20.0);
        let t = terrain::ground(seed, hv(seed), cx, z);
        assert!(
            t < mid - PLANE_THICKNESS_M,
            "the fixture's shot line at {mid:.2} runs into the ground ({t:.2}) \
             at z={z:.2}, so the skirt cannot be what stops it"
        );
    }
    let (ssurf, sy) = impact_of(seed, &cols, from, LEVEL, 20).expect("the side shot never stopped");
    assert_eq!(
        ssurf, SURF_BUILT,
        "an arrow on the {mid:.2} line stopped on {ssurf} at y={sy:.2} — it \
         passed through the skirt of a plate whose top is {top:.2} and whose \
         ground is {ground:.2}"
    );
}

// ---------------------------------------------------------------------------
// Ranged structure damage v0 (2026-08-28) — the address half.
//
// Everything above asks whether a shot STOPS on a piece. These ask which
// piece, which is the fact `deploy::damage_piece` needs and the one
// `collide::shot_blocked` threw away for as long as there has been a bow:
// the walk resolved a cell, a level and a mask bit and returned `true`.
//
// The store half is `tests/chip.rs`, at `World` level, because a facing
// lives on a `PieceRec` and not on the column index this file writes
// directly. These two suites are the same slice split at the seam the sim
// itself is split at: `ranged` finds the piece, `World` charges it.
// ---------------------------------------------------------------------------

/// `bow_fixture` with a structure column, so a shot out of it produces a
/// `Chip` instead of only an impact. The shared fixture stays at zero on
/// purpose — every test above it is about where an arrow stops, and a
/// fixture that started chipping would quietly widen all of them.
fn chipping_bow(structure: u16) -> CombatContent {
    let mut c = bow_fixture();
    c.ranged[BOW as usize].structure = structure;
    c
}

/// Fire one arrow and report every `Chip` the flight produced, with the
/// impact that came with it.
///
/// Returns `(chips, impact_surface)`. Reading both is the point: a chip that
/// arrives without an impact, or an impact whose chip went missing, are two
/// different defects and a test that watched one of them could not tell.
fn chips_of(
    seed: u64,
    cc: &CombatContent,
    cols: &ColIndex,
    at: (f32, f32, f32),
    pitch: u8,
    ticks: u32,
) -> (Vec<ranged::Chip>, Option<u8>) {
    let mut sc = Scratch::barren();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, at.0, at.1, at.2, 0, pitch, 5);
    let mut arrows = Arrows::new();
    assert!(
        ranged::draw(
            0,
            cc,
            &mut arrows,
            &mut EventQueue::default(),
            &mut players[0],
        ),
        "a bow in hand must take the arm"
    );

    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    // Arrow recovery's store. These fixtures predate it and assert
    // nothing about it; `bow_fixture`'s content leaves `arrow_break_pct`
    // at `CombatContent::EMPTY`'s 100, so every landing breaks and the
    // store stays empty. `tests/arrow_recovery.rs` is where it is driven.
    let mut spent = SpentArrows::new();
    for _ in 0..ticks {
        let mut events = EventQueue::default();
        let (_, n_chips) = ranged::step(
            seed,
            0,
            hv(seed),
            cols,
            &mut sc.occupants(),
            cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut events,
            &mut kills,
            &mut chips,
        );
        let surf = events
            .entries()
            .iter()
            .find(|e| e.code == EV_IMPACT)
            .map(|e| (e.a >> 24) as u8);
        if surf.is_some() || n_chips > 0 {
            return (chips[..n_chips].to_vec(), surf);
        }
    }
    (Vec::new(), None)
}

/// An arrow that stops on a floor names **that** floor: the cell it was
/// fired down inside, level 1, `LOC_PLANE`.
///
/// This is the assertion the whole slice rests on. `damage_piece` writes
/// against an address, so an address that is merely *a* piece rather than
/// *the* piece would chip a wall on the far side of the base and read as
/// working — every event fires, the hp comes off something, and only a
/// player watching the wrong wall crumble would ever know.
#[test]
fn the_chip_names_the_piece_the_arrow_actually_stopped_on() {
    for seed in SEEDS {
        let (cols, bcx, bcz, base) = storeys(seed, 1);
        let (cx, cz) = cell_centre(bcx, bcz);
        let feet = base + LEVEL_H_M;
        let cc = chipping_bow(3);

        let (chips, surf) = chips_of(seed, &cc, &cols, (cx, feet, cz), 0, 20);
        assert_eq!(
            surf,
            Some(SURF_BUILT),
            "seed {seed}: the fixture shot did not stop on the floor, so this \
             case is not testing what it says"
        );
        assert_eq!(
            chips.len(),
            1,
            "seed {seed}: one arrow stopping on one piece must produce exactly \
             one chip, not {}",
            chips.len()
        );
        let c = chips[0];
        assert_eq!(
            (c.hit.cx, c.hit.cz, c.hit.level, c.hit.loc),
            (bcx, bcz, 1, LOC_PLANE),
            "seed {seed}: the arrow stopped on the level-1 floor of \
             ({bcx}, {bcz}) and the chip names ({}, {}, {}, {})",
            c.hit.cx,
            c.hit.cz,
            c.hit.level,
            c.hit.loc
        );
        assert_eq!(
            c.structure, 3,
            "seed {seed}: the chip carries the bow's structure column, not {}",
            c.structure
        );
    }
}

/// A bow with no structure column produces no chip — and still stops.
///
/// The mutant this kills is the obvious one: charge damage whenever a shot
/// stops on a piece, ignoring the column. Every other assertion in this
/// slice passes under it, because every other fixture has a column.
#[test]
fn a_bow_with_no_structure_column_chips_nothing_and_still_stops() {
    for seed in SEEDS {
        let (cols, bcx, bcz, base) = storeys(seed, 1);
        let (cx, cz) = cell_centre(bcx, bcz);
        let feet = base + LEVEL_H_M;

        let (chips, surf) = chips_of(seed, &chipping_bow(0), &cols, (cx, feet, cz), 0, 20);
        assert_eq!(
            surf,
            Some(SURF_BUILT),
            "seed {seed}: a bow that cannot chip must still be stopped by the \
             floor — the shot and the damage are two questions"
        );
        assert!(
            chips.is_empty(),
            "seed {seed}: a bow with structure 0 produced {} chip(s) at \
             ({bcx}, {bcz})",
            chips.len()
        );
    }
}

/// A shot that stops on the GROUND or on scenery chips nothing.
///
/// The ladder in `world_stop` answers three ways and only one of them is a
/// piece; a chip minted on `SURF_GROUND` would be an address of (0, 0, 0, 0)
/// — a real build cell — so the failure is not a crash, it is a shot into
/// the dirt taking hp off whatever somebody built at the origin.
#[test]
fn a_shot_into_the_dirt_chips_nothing() {
    for seed in SEEDS {
        let cols = ColIndex::new();
        let (bcx, bcz) = flat_run(seed, 1);
        let (cx, cz) = cell_centre(bcx, bcz);
        let feet = terrain::ground(seed, hv(seed), cx, cz);

        // Straight down at its own feet, over an empty index: nothing built
        // is anywhere near, so the only thing that can stop it is dirt.
        let (chips, surf) = chips_of(seed, &chipping_bow(3), &cols, (cx, feet + 2.0, cz), 0, 30);
        assert_eq!(
            surf,
            Some(SURF_GROUND),
            "seed {seed}: the fixture shot did not reach the ground, so this \
             case is not testing what it says"
        );
        assert!(
            chips.is_empty(),
            "seed {seed}: a shot that stopped on the ground minted {} chip(s) \
             — the first would be charged against build cell ({}, {})",
            chips.len(),
            chips.first().map(|c| c.hit.cx).unwrap_or(0),
            chips.first().map(|c| c.hit.cz).unwrap_or(0),
        );
    }
}

// --- The deployable block. -------------------------------------------------
//
// Everything above is about a *piece* stopping a shot. A solid deployable is
// the other half of what a base is made of and it lived in a walk the shot
// path never called (`collide::deploy_blocked` — the body's), so an arrow
// flew through a furnace, a box and a bench (`NOW.md` §0mk item 2).
//
// The fixture inverts the one above it again: **no pieces at all**, one
// `set_solid` nibble on an otherwise empty index, so a plane can never take
// the credit for what a furnace did. And every case is run as a PAIR over
// identical geometry — once with the nibble and once without — because
// "the shot stopped" is satisfied by an arrow that hit the dirt, and the
// dirt is 4 m below the furnace here on purpose.

/// A furnace standing at LEVEL 1 of a bare column: an empty index with one
/// solid nibble, the level chosen so the deployable's band sits a clear
/// storey above the terrain. Returns `(cols, cx, cz, bottom)`.
///
/// **Level 1 and not 0**, which is the whole reason the fixture is trustworthy:
/// at level 0 the furnace's 0.95 m band sits inside the terrain band's own
/// rounding, and `world_stop` asks the ground BEFORE anything built — so a
/// case that failed would not say whether the walk missed the furnace or the
/// dirt answered first. Three metres up, the two answers are 4 m apart.
///
/// Boxed for `storeys`' reason (the wasm shadow stack).
fn furnace_column(seed: u64) -> (Box<ColIndex>, u16, u16, f32) {
    let (bcx, bcz) = flat_run(seed, 1);
    let mut cols = Box::new(ColIndex::new());
    cols.set_solid(bcx, bcz, 1, Some(sim_core::deploy::ARCH_FURNACE));
    let base = build::column_floor_y(seed, hv(seed), bcx, bcz, 0);
    (cols, bcx, bcz, base + LEVEL_H_M)
}

/// The furnace's own volume, as `deploy::solid_vol` gives it.
fn furnace_vol() -> (f32, f32, f32) {
    sim_core::deploy::solid_vol(sim_core::deploy::ARCH_FURNACE)
        .expect("the furnace is a solid archetype")
}

/// An arrow fired down onto a furnace stops **on** it, and the same shot over
/// the same column without it reaches the dirt.
///
/// The pair is the assertion. `SURF_BUILT` alone would be satisfied by any
/// piece anywhere in the walk, and there are none here; "stopped at all"
/// would be satisfied by the ground. The furnace is the only difference
/// between the two runs, and they answer 4 m apart.
///
/// **Three origins, and the two off-centre ones are a mutant this case did
/// not used to catch.** `deploy_stop` measures the sphere against the box by
/// clamping the offset into the box's own extents — `x - cxm - (x -
/// cxm).clamp(-hw, hw)` — and down the cell's exact centre `x - cxm` is
/// zero, so the clamp is the identity there and DELETING IT changes nothing.
/// A shot fired inside the box but off its centre is where the clamp does
/// work: at half an extent out the unclamped term is 0.33 m against an
/// arrowhead of 0.05 m, so the furnace stops answering and the dirt takes
/// the shot. One axis at a time, so each of the two clamps is named by a
/// case of its own. Judged 2026-08-28.
#[test]
fn a_furnace_stops_a_shot_the_bare_column_lets_through() {
    for seed in SEEDS {
        let (cols, bcx, bcz, bottom) = furnace_column(seed);
        let (cx, cz) = cell_centre(bcx, bcz);
        let (hw, h, hd) = furnace_vol();
        // Boxed and hoisted: `ColIndex` is a large fixed array and this loop
        // would otherwise build one per origin (the wasm shadow-stack trap).
        let bare = Box::new(ColIndex::new());
        // Half an extent is the off-centre distance below, and it has to
        // clear the arrowhead in BOTH axes or the clamp-deleted mutant walks
        // through the case that was written to catch it.
        assert!(
            hw * 0.5 > ranged::ARROW_R_M && hd * 0.5 > ranged::ARROW_R_M,
            "half the furnace's extents ({:.2}, {:.2}) must exceed the \
             arrowhead {:.2}",
            hw * 0.5,
            hd * 0.5,
            ranged::ARROW_R_M
        );

        // Centre, then half an extent out along each axis on its own —
        // inside the box every time, so all three must stop on the furnace.
        for (dx, dz) in [(0.0f32, 0.0f32), (hw * 0.5, 0.0), (0.0, hd * 0.5)] {
            let from = (cx + dx, bottom + 2.5, cz + dz);

            let (surf, y) = impact_of(seed, &cols, from, 0, 30).unwrap_or_else(|| {
                panic!("seed {seed}: the shot from (+{dx:.2}, +{dz:.2}) never stopped on anything")
            });
            assert_eq!(
                surf,
                SURF_BUILT,
                "seed {seed}: an arrow fired down onto a furnace from \
                 (+{dx:.2}, +{dz:.2}) — inside half-extents ({hw:.2}, {hd:.2}) \
                 — reported surface {surf} at y={y:.2}; the furnace's band is \
                 {bottom:.2}..{:.2} and it is transparent to the shot",
                bottom + h
            );
            // The TOP, not the first sample in the column: a walk that answered
            // on the nibble alone and never measured the box would stop the
            // arrow 2.5 m higher, and `surf` cannot tell the two apart.
            assert!(
                fabs(y - (bottom + h)) <= 0.3,
                "seed {seed}: the impact landed at y={y:.2}; the furnace's top is \
                 {:.2} and the arrow was fired from {:.2}",
                bottom + h,
                from.1
            );

            let (bsurf, by) = impact_of(seed, &bare, from, 0, 30)
                .unwrap_or_else(|| panic!("seed {seed}: the barren shot never stopped"));
            assert_eq!(
                bsurf, SURF_GROUND,
                "seed {seed}: with the nibble cleared the identical shot must reach \
                 the dirt — it reported {bsurf} at y={by:.2}, so the fixture is not \
                 isolating the furnace"
            );
        }
    }
}

/// **A shot fired UP from below a furnace stops on its UNDERSIDE**, not on
/// the first sample of the column.
///
/// The gate for `collide::deploy_stop`'s vertical band, and specifically
/// for the FLOOR of it. `ey = y - y.clamp(bottom, bottom + h)` and
/// `ey = y - y.min(bottom + h)` are bit-identical for every `y >= bottom`,
/// and until 2026-08-28 every case in this file and in `tests/chip.rs`
/// fired straight down from above the box — so deleting the floor rail ran
/// the whole `sim-core` suite green (the merge-gate judge's first ranked
/// fix on `pass-20260828-065501-03`). Without it a box is an infinitely
/// deep column: a bench on the storey above eats an arrow fired level on
/// the ground floor, and a raider shooting up at a ceiling hits a bench
/// they cannot see, 1.4 m before they reach it.
///
/// `surf` cannot separate the two — both report `SURF_BUILT` — so the
/// assertion is on **y**, and the two answers are `LEVEL_H_M − eye` apart,
/// which is 1.4 m against a 0.3 m tolerance.
///
/// The control is the roof case's, for its reason: fired up, this fixture's
/// arrow outlives its own range and announces no impact at all, so
/// "not built" is the honest read rather than "the dirt".
#[test]
fn a_shot_fired_up_stops_on_the_furnace_underside_not_below_it() {
    for seed in SEEDS {
        let (cols, bcx, bcz, bottom) = furnace_column(seed);
        let (cx, cz) = cell_centre(bcx, bcz);
        let (_, h, _) = furnace_vol();
        // Standing on the bare column under the furnace's storey.
        let feet = bottom - LEVEL_H_M;
        let muzzle = feet + ARROW_EYE_MM as f32 / 1000.0;
        assert!(
            muzzle < bottom,
            "seed {seed}: the muzzle at {muzzle:.2} is already inside the              furnace's band, so there is nothing below the box to fire from"
        );

        // 255 is the steepest up the pitch encoding holds.
        let (surf, y) = impact_of(seed, &cols, (cx, feet, cz), 255, 20)
            .unwrap_or_else(|| panic!("seed {seed}: the shot never stopped on anything"));
        assert_eq!(
            surf, SURF_BUILT,
            "seed {seed}: an arrow fired up at a furnace reported surface {surf}              at y={y:.2} — its underside at {bottom:.2} let it through"
        );
        assert!(
            fabs(y - bottom) <= 0.3,
            "seed {seed}: the impact landed at y={y:.2}; the furnace's underside              is {bottom:.2}, its top is {:.2}, and the muzzle was {muzzle:.2} — a              stop below the box is the vertical band with no floor",
            bottom + h
        );

        let bare = ColIndex::new();
        let control = impact_of(seed, &bare, (cx, feet, cz), 255, 90);
        assert!(
            !matches!(control, Some((SURF_BUILT, _))),
            "seed {seed}: the control stopped on a built surface with an empty              index ({control:?}), so the assertion above is not about the furnace"
        );
    }
}

/// The chip a furnace hit mints names the DEPLOY store, at the deployable's
/// own address.
///
/// Two mutants live here and neither is visible from `surf`. Reporting
/// `deploy: false` sends the chip to `Pieces::find_index`, which finds
/// nothing at that address and silently drops it — a furnace that stops
/// arrows forever and never loses hp. Naming level 0 instead of 1 charges a
/// different deployable in the same column, which is a real address a base
/// can hold.
#[test]
fn the_chip_off_a_furnace_names_the_deploy_store() {
    for seed in SEEDS {
        let (cols, bcx, bcz, bottom) = furnace_column(seed);
        let (cx, cz) = cell_centre(bcx, bcz);
        let cc = chipping_bow(3);

        let (chips, surf) = chips_of(seed, &cc, &cols, (cx, bottom + 2.5, cz), 0, 30);
        assert_eq!(
            surf,
            Some(SURF_BUILT),
            "seed {seed}: the fixture shot did not stop on the furnace, so this \
             case is not testing what it says"
        );
        assert_eq!(
            chips.len(),
            1,
            "seed {seed}: one arrow stopping on one furnace must produce exactly \
             one chip, not {}",
            chips.len()
        );
        let c = chips[0];
        assert!(
            c.deploy,
            "seed {seed}: the chip off a furnace is addressed to the piece \
             store, so `World::chip` will look it up in `Pieces` and drop it"
        );
        assert_eq!(
            (c.hit.cx, c.hit.cz, c.hit.level, c.hit.loc),
            (bcx, bcz, 1, LOC_PLANE),
            "seed {seed}: the furnace stands at ({bcx}, {bcz}) level 1 and the \
             chip names ({}, {}, {}, {})",
            c.hit.cx,
            c.hit.cz,
            c.hit.level,
            c.hit.loc
        );
        assert_eq!(
            c.structure, 3,
            "seed {seed}: the chip carries the bow's structure column, not {}",
            c.structure
        );
    }
}

/// A shot down the same column but outside the furnace's footprint misses it
/// and reaches the dirt.
///
/// The mutant: stop on the nibble and skip the extent test. Every assertion
/// in the two cases above passes under it, because both fire down the cell's
/// exact centre — this is the one that reads `DEPLOY_VOL`'s row.
///
/// **One axis at a time, and that correction is the whole reason this
/// paragraph exists.** The first draft offset x AND z together and said in a
/// comment that it therefore caught a mutant that dropped either extent
/// test. It caught neither, and a judge ran it: with both axes out, the
/// surviving axis rejects the sample on its own, so deleting the other one
/// is invisible. A miss has to be caused by exactly one axis for that axis's
/// test to be load-bearing. The diagonal case stays as the third row because
/// it is the ordinary geometry, not because it proves anything the first two
/// do not. Judged 2026-08-28.
#[test]
fn a_shot_past_the_furnace_in_its_own_cell_misses_it() {
    for seed in SEEDS {
        let (cols, bcx, bcz, bottom) = furnace_column(seed);
        let (cx, cz) = cell_centre(bcx, bcz);
        let (hw, _, hd) = furnace_vol();
        // Inside the build cell (half 1.5 m), outside the box (0.65 x 0.425)
        // by more than the arrowhead.
        let off = 1.2f32;
        assert!(
            off > hw + ranged::ARROW_R_M && off > hd + ranged::ARROW_R_M,
            "the fixture offset must clear the furnace's own extents"
        );
        assert!(
            off < BUILD_CELL_M * 0.5,
            "…and stay inside the build cell, or a neighbour answers instead"
        );

        for (dx, dz) in [(off, 0.0f32), (0.0f32, off), (off, off)] {
            let (chips, surf) = chips_of(
                seed,
                &chipping_bow(3),
                &cols,
                (cx + dx, bottom + 2.5, cz + dz),
                0,
                30,
            );
            assert_eq!(
                surf,
                Some(SURF_GROUND),
                "seed {seed}: an arrow fired (+{dx:.1}, +{dz:.1}) m off the cell \
                 centre passed a furnace whose half-extents are ({hw:.2}, \
                 {hd:.2}) and was stopped by it"
            );
            assert!(
                chips.is_empty(),
                "seed {seed}: a shot from (+{dx:.1}, +{dz:.1}) that stopped on \
                 the ground minted {} chip(s)",
                chips.len()
            );
        }
    }
}
