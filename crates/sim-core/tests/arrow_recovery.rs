//! Arrows come back — the stop, the break roll, the lodge and the fall
//! (`reference/PROJECTILES.md` §5; `NOW.md` §5 item 2).
//!
//! This file drives `ranged::step` and `spent::settle` directly: that a
//! stopped arrow is handed on to rest the same tick, that one in a body
//! rides it for exactly the lodge and falls out at its feet, that it falls
//! at once where its host died or left, that one out of flight falls
//! instead of vanishing, that the odds are the odds, and that a world
//! remembers the arrows in bodies across a save. `tests/arrow_pickup.rs`
//! drives the same through `World::tick`, into the loose-stack store.

// Measurements are this gate's output — same allow and same reason as
// `tests/shoot.rs`: the L5 wall bans format/print in SIM code, and a test
// harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::combat::NO_MAG;
use sim_core::combat::{AmmoDef, CombatContent, RangedDef};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_MOBS, MAX_PLAYERS, MAX_SPENT_ARROWS, TICK_HZ};
use sim_core::mob::{mob_id, Mob, Mobs};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Occupants, Pristine, Scratch};
use sim_core::ranged::{self, Arrows, Kill, ARROW_EYE_MM};
use sim_core::spent::{self, feet_mm, SpentArrows, SpentRec};
use sim_core::world::{EventQueue, Player};

/// Item indices the fixture makes a bow and its ammo. Distinct so that
/// "the round came back, not the weapon" is a checkable claim.
const BOW: u16 = 3;
const ARROW: u16 = 4;

/// The lodge the fixture arms, in ticks. Ten seconds is the reference's
/// number and `content/balance.toml` ships it; the fixture states it
/// itself so a content edit cannot quietly turn a red here into a green.
const LODGE_TICKS: u32 = 10 * TICK_HZ;

/// A bow fixture with recovery armed. `break_pct` is a parameter because
/// tests need never, always and the shipped odds out of one flight.
fn bow(break_pct: u16, range_mm: u32) -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.arrow_break_pct = break_pct;
    c.arrow_lodge_ticks = LODGE_TICKS;
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        hitscan: false,
        range_mm,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
        draw_ticks: 0,
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 22,
    };
    c
}

fn archer(id: u32, x: f32, feet_y: f32, z: f32, pitch: u8) -> Player {
    let mut p = Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.body = body_at(x, feet_y, z);
    p.inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
        skin: 0,
    };
    p.inv[7] = ItemStack {
        item: ARROW,
        count: 10,
        cond: 0,
        skin: 0,
    };
    p.frame = InputFrame {
        buttons: BTN_PRIMARY,
        yaw: 0,
        pitch,
        ..InputFrame::default()
    };
    p
}

fn body_at(x: f32, feet_y: f32, z: f32) -> Body {
    Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (feet_y / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        ..Body::default()
    }
}

fn no_mobs() -> Box<Mobs> {
    Box::new(Mobs {
        m: Box::new([Mob::default(); MAX_MOBS]),
        survey: Default::default(),
    })
}

/// Draw once and fly until the arrow resolves. Returns the store and the
/// tick it resolved on.
fn fly(
    seed: u64,
    cc: &CombatContent,
    players: &mut [Player; MAX_PLAYERS],
    max_ticks: u64,
) -> (SpentArrows, u64) {
    let mut sc = Scratch::with(seed, Pristine);
    let cols = sim_core::collide::ColIndex::new();
    let mut arrows = Arrows::new();
    let mut spent = SpentArrows::new();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    assert!(
        ranged::draw(
            0,
            cc,
            &mut arrows,
            &mut EventQueue::default(),
            &mut players[0]
        ),
        "a bow in hand must take the arm"
    );
    assert_eq!(arrows.len(), 1, "the draw must have produced one arrow");
    let mut t = 0u64;
    while !arrows.is_empty() && t < max_ticks {
        t += 1;
        let mut occ = Occupants {
            doors: 0,
            table: &sc.table,
            haven: &sc.haven,
            harvested: &sc.harvested,
            cache: &mut sc.cache,
        };
        ranged::step(
            seed,
            t,
            &sc.haven,
            &cols,
            &mut occ,
            cc,
            &mut arrows,
            &mut spent,
            players,
            &mut EventQueue::default(),
            &mut kills,
            &mut chips,
        );
    }
    assert!(arrows.is_empty(), "the arrow never resolved");
    (spent, t)
}

fn ground_at(seed: u64, x: f32, z: f32) -> f32 {
    let sc = Scratch::with(seed, Pristine);
    sim_core::terrain::ground(seed, &sc.haven, x, z)
}

/// Straight down from 20 m up, into open ground.
fn fire_into_the_ground(seed: u64, break_pct: u16) -> (SpentArrows, u64) {
    let ground = ground_at(seed, 2048.0, 2048.0);
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, 2048.0, ground + 20.0, 2048.0, 0);
    fly(seed, &bow(break_pct, 60_000), &mut players, 120)
}

/// Everything `settle` lays this tick, as `(round, x, y, z)` millimetres.
fn settle(
    spent: &mut SpentArrows,
    tick: u64,
    players: &[Player; MAX_PLAYERS],
    mobs: &Mobs,
) -> Vec<(u16, i32, i32, i32)> {
    let mut laid = Vec::new();
    spent::settle(spent, tick, players, mobs, |r, x, y, z, _dir| {
        laid.push((r, x, y, z))
    });
    laid
}

// ---------------------------------------------------------------------
// The flight, end to end
// ---------------------------------------------------------------------

/// A miss sticks the tick it lands, where it went in: just above the
/// surface it hit, pointing the way it flew. Mutant: resting it from the
/// stop sample itself (inside the ground) fails the height check; skipping
/// the walk that finds where it went in fails the closeness check; lodging
/// it with a host fails the host check.
#[test]
fn a_missed_arrow_rests_the_tick_it_lands() {
    for seed in [0u64, 1, 7, 12345] {
        let (mut spent, t) = fire_into_the_ground(seed, 0);
        assert_eq!(spent.len(), 1, "seed {seed}: break_pct 0 keeps the arrow");
        let rec = spent.entries()[0];
        assert_eq!(rec.round, ARROW, "seed {seed}: the round, not the bow");
        assert_eq!(rec.host, 0, "seed {seed}: a miss is in nothing");
        assert!(rec.ready_at <= t, "seed {seed}: a miss waits for nothing");
        let (x, z) = (rec.qx as f32 / 1000.0, rec.qz as f32 / 1000.0);
        let ground_mm = (ground_at(seed, x, z) * 1000.0) as i32;
        assert!(
            rec.qy >= ground_mm - 1,
            "seed {seed}: it stays on the free side of where it went in, above \
             the ground (y {} mm against ground {ground_mm} mm)",
            rec.qy
        );
        assert!(
            rec.qy - ground_mm <= 40,
            "seed {seed}: and where it went in, not a step short of it (y {} mm \
             against ground {ground_mm} mm)",
            rec.qy
        );
        assert_eq!(
            rec.dir,
            [0, -127, 0],
            "seed {seed}: shot straight down, it stands straight up in the dirt"
        );
        let players = Box::new([Player::default(); MAX_PLAYERS]);
        let laid = settle(&mut spent, t, &players, &no_mobs());
        assert_eq!(
            laid,
            vec![(ARROW, rec.qx, rec.qy, rec.qz)],
            "seed {seed}: it is laid down the same tick, from where it stopped"
        );
        assert!(spent.is_empty(), "seed {seed}: and leaves this store");
    }
}

#[test]
fn a_broken_arrow_leaves_nothing_and_that_is_the_inert_default() {
    let (spent, _) = fire_into_the_ground(7, 100);
    assert!(spent.is_empty(), "break_pct 100 must destroy every landing");
    assert_eq!(
        CombatContent::EMPTY.arrow_break_pct,
        100,
        "an unarmed content set must destroy arrows, never hand them back"
    );
}

/// An arrow that ran out of flight in the air falls instead of vanishing
/// (`NOW.md` §5 item 2). An arrow flies until something stops it, so the
/// only one that runs out is one still up at the backstop: shot straight
/// up under a light drop, it is still in the air when the backstop comes.
/// Mutant: dropping the landing on expiry leaves the store empty.
#[test]
fn an_arrow_out_of_flight_falls_instead_of_vanishing() {
    let seed = 7u64;
    let ground = ground_at(seed, 2048.0, 2048.0);
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, 2048.0, ground, 2048.0, u8::MAX);
    let life = u64::from(sim_core::limits::MAX_ARROW_LIFE_TICKS);
    // Up and back down past the eye takes `2v/g` ticks; past the backstop.
    let mut cc = bow(0, 60_000);
    cc.ammo[ARROW as usize].drop_mmpt2 = 4;
    assert!(2 * 1333 / 4 > life, "the fixture must outlast the backstop");
    let (spent, t) = fly(seed, &cc, &mut players, life + 5);
    assert_eq!(
        t, life,
        "straight up, it is still in the air at the backstop"
    );
    assert_eq!(spent.len(), 1, "the arrow is handed on to fall");
    let rec = spent.entries()[0];
    assert_eq!(rec.host, 0);
    assert!(
        rec.qy > (ground * 1000.0) as i32 + 1_000,
        "it falls from where it ran out, still in the air"
    );
}

/// A bow's range is not a flight time any more: a level shot from a height
/// flies on past what the old 60 m reach allowed and lands, rather than
/// running out in the air (`ranged::draw`). Mutant: restoring the derived
/// life ends it mid-air, 45 ticks in.
#[test]
fn an_arrow_flies_until_it_lands() {
    let seed = 7u64;
    let ground = ground_at(seed, 2048.0, 2048.0);
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    // A little above level, from 40 m up: well past 45 ticks of flight.
    players[0] = archer(1, 2048.0, ground + 40.0, 2048.0, 140);
    let life = u64::from(sim_core::limits::MAX_ARROW_LIFE_TICKS);
    let (spent, t) = fly(seed, &bow(0, 60_000), &mut players, life + 5);
    assert!(t > 45 && t < life, "it landed on tick {t}");
    assert_eq!(spent.len(), 1);
    let rec = spent.entries()[0];
    assert!(
        rec.qy < ((ground + 40.0) * 1000.0) as i32,
        "it came down, not out of flight up there (y {} mm)",
        rec.qy
    );
}

/// The shared fixture for a hit: an archer and a target 6 m north, level.
fn shoot_the_target() -> (Box<[Player; MAX_PLAYERS]>, SpentArrows, u64) {
    let seed = 7u64;
    let ground = ground_at(seed, 2048.0, 2048.0);
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = archer(1, 2048.0, ground, 2048.0, 128);
    players[1] = Player {
        id: 2,
        active: true,
        hp: 100,
        hp_max: 100,
        body: body_at(2048.0, ground + ARROW_EYE_MM as f32 / 1000.0 - 1.2, 2054.0),
        ..Player::default()
    };
    let (spent, t) = fly(seed, &bow(0, 60_000), &mut players, 60);
    assert!(
        players[1].hp < 100,
        "the fixture must actually land a hit — a miss would make every \
         assertion below pass for the wrong reason"
    );
    (players, spent, t)
}

/// A hit rides the body for exactly the lodge, then falls out at its feet
/// wherever it has walked. Mutants: an off-by-one on `ready_at`, or not
/// following the host, each fail an assertion here.
#[test]
fn an_arrow_that_drew_blood_rides_its_host_for_the_lodge() {
    let (mut players, mut spent, t) = shoot_the_target();
    assert_eq!(spent.len(), 1, "a hit leaves the arrow in the target");
    let rec = spent.entries()[0];
    assert_eq!(rec.host, 2, "in the body it hit");
    assert_eq!(rec.life, 0, "in the life it hit");
    assert_eq!(
        rec.ready_at,
        t + u64::from(LODGE_TICKS),
        "for the lodge, exactly"
    );

    let mobs = no_mobs();
    // The target walks off 10 m east; the arrow goes with it.
    players[1].body.qx += (10.0 / POS_XZ_Q) as i32;
    assert!(settle(&mut spent, t + 1, &players, &mobs).is_empty());
    let feet = feet_mm(&players[1].body);
    let e = spent.entries()[0];
    assert_eq!((e.qx, e.qy, e.qz), feet, "it rides the body");

    assert!(
        settle(&mut spent, rec.ready_at - 1, &players, &mobs).is_empty(),
        "still in the body a tick before the lodge ends"
    );
    assert_eq!(
        settle(&mut spent, rec.ready_at, &players, &mobs),
        vec![(ARROW, feet.0, feet.1, feet.2)],
        "and it falls out at the body's feet on the tick it ends"
    );
    assert!(spent.is_empty());
}

fn lodged_in(host: u32, life: u64, at: (i32, i32, i32), ready_at: u64) -> SpentRec {
    SpentRec {
        qx: at.0,
        qy: at.1,
        qz: at.2,
        round: ARROW,
        ready_at,
        host,
        life,
        dir: [0; 3],
    }
}

/// A host that dies drops it at once, where it last stood — not wherever a
/// respawn put the body. Mutant: checking `dead` without `deaths` carries
/// the arrow to the new body after a respawn in the same tick.
#[test]
fn an_arrow_in_a_body_that_died_falls_where_it_last_stood() {
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[1] = Player {
        id: 2,
        active: true,
        hp: 100,
        body: body_at(100.0, 5.0, 100.0),
        ..Player::default()
    };
    let mobs = no_mobs();
    let mut spent = SpentArrows::new();
    spent.lodge(lodged_in(2, 0, (0, 0, 0), 1_000));
    assert!(settle(&mut spent, 10, &players, &mobs).is_empty());
    let stood = feet_mm(&players[1].body);

    // Died, and already back on a beach with the next life.
    players[1].deaths = 1;
    players[1].body = body_at(900.0, 2.0, 900.0);
    assert_eq!(
        settle(&mut spent, 11, &players, &mobs),
        vec![(ARROW, stood.0, stood.1, stood.2)],
        "it falls at once, where the body it was in last stood"
    );

    // And a corpse still on the death screen drops it the same way.
    let mut spent = SpentArrows::new();
    spent.lodge(lodged_in(2, 1, (1, 2, 3), 1_000));
    players[1].dead = true;
    assert_eq!(
        settle(&mut spent, 12, &players, &mobs),
        vec![(ARROW, 1, 2, 3)]
    );
}

#[test]
fn a_host_that_left_the_world_drops_it_where_it_last_stood() {
    let players = Box::new([Player::default(); MAX_PLAYERS]);
    let mut spent = SpentArrows::new();
    spent.lodge(lodged_in(2, 0, (7, 8, 9), 1_000));
    assert_eq!(
        settle(&mut spent, 1, &players, &no_mobs()),
        vec![(ARROW, 7, 8, 9)],
        "no body answers to the id, so it falls where it was"
    );
}

/// An animal carries it too, and a slot that died and hatched again is a
/// different animal. Mutant: keying on `alive` alone keeps it in the new
/// life's body.
#[test]
fn an_arrow_in_an_animal_rides_it_and_falls_where_it_dies() {
    let players = Box::new([Player::default(); MAX_PLAYERS]);
    let mut mobs = no_mobs();
    mobs.m[3].alive = true;
    mobs.m[3].respawn_at = 77;
    mobs.m[3].body = body_at(50.0, 4.0, 60.0);
    let mut spent = SpentArrows::new();
    spent.lodge(lodged_in(mob_id(3), 77, (0, 0, 0), 1_000));

    assert!(settle(&mut spent, 5, &players, &mobs).is_empty());
    let stood = feet_mm(&mobs.m[3].body);
    assert_eq!(
        (
            spent.entries()[0].qx,
            spent.entries()[0].qy,
            spent.entries()[0].qz
        ),
        stood,
        "it rides the animal"
    );

    // Dead and hatched again at home: a new life in the same slot.
    mobs.m[3].respawn_at = 900;
    mobs.m[3].body = body_at(10.0, 4.0, 10.0);
    assert_eq!(
        settle(&mut spent, 6, &players, &mobs),
        vec![(ARROW, stood.0, stood.1, stood.2)],
        "it falls where the animal it was in died"
    );
}

// ---------------------------------------------------------------------
// The break roll
// ---------------------------------------------------------------------

/// Mutant: keying the roll on the tick alone makes every arrow landing on
/// one tick share a fate, and the per-slot assertion fails.
#[test]
fn the_break_roll_is_its_stated_rate_and_the_same_bits_twice() {
    let seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut broke = 0usize;
    let mut trace = Vec::with_capacity(64);
    let n = 200_000usize;
    for i in 0..n {
        let (tick, slot) = ((i / MAX_ARROWS) as u64, i % MAX_ARROWS);
        let b = spent::breaks(seed, tick, slot, 15);
        broke += usize::from(b);
        if trace.len() < 64 {
            trace.push(b);
        }
    }
    let pct = broke as f64 * 100.0 / n as f64;
    println!("break rate over {n} draws: {pct:.3}%");
    assert!(
        (14.5..=15.5).contains(&pct),
        "a declared 15 % that measures {pct:.3}% is not 15 %"
    );
    let again: Vec<bool> = (0..64)
        .map(|i| spent::breaks(seed, (i / MAX_ARROWS) as u64, i % MAX_ARROWS, 15))
        .collect();
    assert_eq!(
        trace, again,
        "the roll must not depend on anything but its key"
    );

    let same_tick: Vec<bool> = (0..64).map(|s| spent::breaks(seed, 99, s, 15)).collect();
    assert!(
        same_tick.iter().any(|&b| b) && same_tick.iter().any(|&b| !b),
        "every arrow landing on one tick shared a fate — the roll is not \
         keyed on the slot"
    );
}

#[test]
fn zero_never_breaks_and_a_hundred_always_does() {
    for slot in 0..MAX_ARROWS {
        assert!(!spent::breaks(3, slot as u64, slot, 0));
        assert!(spent::breaks(3, slot as u64, slot, 100));
    }
}

// ---------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------

fn rec(x: i32, ready_at: u64) -> SpentRec {
    SpentRec {
        qx: x,
        round: ARROW,
        ready_at,
        host: 2,
        ..SpentRec::default()
    }
}

#[test]
fn the_store_is_bounded_and_says_so() {
    let mut s = SpentArrows::new();
    for i in 0..MAX_SPENT_ARROWS {
        assert!(
            !s.lodge(rec(i as i32, i as u64)),
            "no eviction while there is room"
        );
    }
    assert_eq!(s.len(), MAX_SPENT_ARROWS);
    assert_eq!(s.evictions(), 0);

    assert!(
        s.lodge(rec(-1, 9_000)),
        "a full store must evict, not refuse"
    );
    assert_eq!(s.len(), MAX_SPENT_ARROWS, "the cap holds");
    assert_eq!(
        s.evictions(),
        1,
        "an eviction that nothing counts is an absence"
    );
    assert!(
        !s.entries().iter().any(|e| e.ready_at == 0),
        "the evicted entry must be the one due out soonest"
    );
    assert!(
        s.entries().iter().any(|e| e.qx == -1),
        "and the arrow that just landed must be the one that survived"
    );
}

#[test]
fn a_world_that_remembers_its_arrows_saves_them() {
    use sim_core::world::World;
    let mut w = Box::new(World::new(4242));
    for i in 0..40 {
        w.spent.lodge(SpentRec {
            qx: 1_000 + i,
            qy: 17 * i,
            qz: 2_000 - i,
            round: ARROW,
            ready_at: 900 + i as u64,
            host: 0x100 + i as u32,
            life: i as u64,
            dir: [0; 3],
        });
    }
    for i in 0..MAX_SPENT_ARROWS + 5 {
        w.spent.lodge(rec(i as i32, i as u64));
    }
    assert!(
        w.spent.evictions() > 0,
        "the fixture must exercise the counter"
    );
    let before = w.state_hash();
    let evicted = w.spent.evictions();

    let mut blob = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("a world must save");
    blob.truncate(n);

    let mut w2 = Box::new(World::new(4242));
    w2.load(&blob).expect("and load again");
    assert_eq!(w2.spent.len(), MAX_SPENT_ARROWS);
    assert_eq!(w2.spent.evictions(), evicted);
    assert_eq!(
        w2.spent.entries(),
        w.spent.entries(),
        "host and life survive"
    );
    assert_eq!(
        w2.state_hash(),
        before,
        "a save that forgot the arrows in bodies is wall 5 failing at the origin"
    );
}

/// An empty store folds not one byte, so the pinned replay hash stays
/// evidence about the script it pins (`world.rs::state_hash`).
#[test]
fn a_world_that_never_fired_hashes_as_though_this_store_did_not_exist() {
    use sim_core::world::World;
    let a = Box::new(World::new(5));
    let mut b = Box::new(World::new(5));
    assert_eq!(a.state_hash(), b.state_hash());
    b.spent.lodge(rec(1, 1));
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "an arrow in a body is state, and state that does not reach the \
         hash is a divergence nothing can see"
    );
    b.spent.take_at(0);
    assert!(b.spent.is_empty());
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "and a store emptied with no eviction behind it folds nothing again"
    );
}

/// The host and its life are hashed: two worlds that disagree about which
/// body an arrow is in must not hash alike.
#[test]
fn which_body_an_arrow_is_in_is_state() {
    use sim_core::world::World;
    let mut a = Box::new(World::new(5));
    let mut b = Box::new(World::new(5));
    a.spent.lodge(rec(1, 1));
    b.spent.lodge(SpentRec {
        host: 3,
        ..rec(1, 1)
    });
    assert_ne!(a.state_hash(), b.state_hash(), "the host is hashed");
    let mut c = Box::new(World::new(5));
    c.spent.lodge(SpentRec {
        life: 9,
        ..rec(1, 1)
    });
    assert_ne!(a.state_hash(), c.state_hash(), "and so is its life");
}
