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
//! Every check drives `ranged::draw` and `ranged::step` directly with an
//! EMPTY `ColIndex`, walk.rs's rule for the same reason: nothing is built,
//! so a wall can never take the credit for what a tree did.

// Measurements are this gate's output — same allow and same reason as
// `tests/walk.rs` and `tests/solid.rs`: the L5 wall bans format/print in SIM
// code, and a test harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::collide::ColIndex;
use sim_core::combat::{AmmoDef, CombatContent, RangedDef};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Barren, Occupants, Pristine, Scratch};
use sim_core::ranged::{self, Arrows, Kill, ARROW_EYE_MM};
use sim_core::terrain::{self, Occupant, ScatterTable, Slot, CELLS_PER_SIDE};
use sim_core::world::{EventQueue, Player, EV_DEATH, EV_HIT, EV_SHOT};

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
/// way `bake_bow` converts it: 40 m/s and 20 m/s² at 30 Hz. Written out
/// rather than loaded so this suite gates the *flight*, and a content edit
/// cannot quietly turn a red here into a green.
fn bow_fixture() -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        // 60 m, the real bow's reach. `ranged::draw` divides this by the
        // round's speed for the flight, which at 1333 mm/tick is the 45
        // ticks this fixture used to state as a constant.
        range_mm: 60_000,
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
    let mut ticks = 0;
    // Long enough for a 45-tick arrow to expire on its own if nothing ever
    // stops it — so "the store emptied" is never a timeout.
    while !arrows.is_empty() && ticks < 60 {
        events = EventQueue::default();
        ranged::step(
            seed,
            hv(seed),
            &cols,
            occ,
            &mut arrows,
            &mut players,
            &mut events,
            &mut kills,
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
        assert_eq!(
            hp_open, 70,
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
    let mut hits = 0;
    for _ in 0..40 {
        events = EventQueue::default();
        ranged::step(
            seed,
            hv(seed),
            &cols,
            &mut sc.occupants(),
            &mut arrows,
            &mut players,
            &mut events,
            &mut kills,
        );
        hits += events.entries().iter().filter(|e| e.code == EV_HIT).count();
    }
    assert_eq!(players[1].hp, 70, "the arrow did not take its 30");
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
        let n = ranged::step(
            seed,
            hv(seed),
            &cols,
            &mut sc.occupants(),
            &mut arrows,
            &mut players,
            &mut events,
            &mut kills,
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
            hv(seed),
            &cols,
            &mut sc.occupants(),
            &mut arrows,
            &mut players,
            &mut events,
            &mut kills,
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
                hv(seed),
                &cols,
                &mut sc.occupants(),
                &mut arrows,
                &mut players,
                &mut events,
                &mut kills,
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
                hv(seed),
                &cols,
                &mut sc.occupants(),
                &mut arrows,
                &mut players,
                &mut events,
                &mut kills,
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
