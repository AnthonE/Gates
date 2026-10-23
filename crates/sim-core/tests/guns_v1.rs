//! Guns v1: the aim at the frame's whole precision, Rust's aim cone, the
//! semi-automatic trigger, a shotgun's pellets landing as one blow, damage
//! falloff and the cone's bloom. The hitscan's older laws are `gun.rs`'s.

#![allow(clippy::disallowed_macros)]
// Host-side reference angles for the asserts, not sim arithmetic.
#![allow(clippy::disallowed_methods)]

use sim_core::collide::ColIndex;
use sim_core::combat::{CombatContent, Gun, RangedDef};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::ranged::{self, Kill, ARROW_EYE_MM};
use sim_core::rewind::Rewind;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, EV_HIT, EV_SHOT};

const SEED: u64 = 20261018;
const GUN: u16 = 5;
const ROUND: u16 = 6;
const LEVEL: u8 = 128;

fn hv() -> &'static terrain::Haven {
    use std::sync::OnceLock;
    static H: OnceLock<terrain::Haven> = OnceLock::new();
    H.get_or_init(|| terrain::haven(SEED))
}

fn fixture(gun: Gun, damage: u16) -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 4,
        rate_sub: 0,
        hitscan: true,
        range_mm: 100_000,
        structure: 0,
        head_pct: 200,
        limb_pct: 50,
        magazine: 30,
        reload_ticks: 100,
        mag_slot: 0,
        draw_ticks: 0,
        gun,
    };
    c
}

fn body(id: u32, x: f32, z: f32) -> Player {
    let mut p = Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.body = Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (400.0 / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        grounded: true,
        ..Body::default()
    };
    p
}

/// A shooter at the origin facing +Z with a loaded gun, and a target `d`
/// metres down the line.
fn range(d: f32) -> Box<[Player; MAX_PLAYERS]> {
    let mut ps = Box::new([Player::default(); MAX_PLAYERS]);
    let mut s = body(1, 0.0, 0.0);
    s.inv[0] = ItemStack {
        item: GUN,
        count: 1,
        cond: 0,
        skin: 0,
    };
    s.mag[0] = 30;
    s.mag_round[0] = ROUND;
    s.frame = InputFrame {
        seq: 1,
        buttons: BTN_PRIMARY,
        yaw: 0,
        pitch: LEVEL,
        sel: 0,
        ..InputFrame::default()
    };
    ps[0] = s;
    // Feet set so the shooter's eye meets the target's chest.
    let mut t = body(2, 0.0, d);
    t.body.qy = ((400.0 + ARROW_EYE_MM as f32 / 1000.0 - 1.2) / POS_Y_Q) as i32;
    ps[1] = t;
    ps
}

fn fire(tick: u64, cc: &CombatContent, ps: &mut [Player; MAX_PLAYERS]) -> EventQueue {
    let mut events = EventQueue::default();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let mut sc = Scratch::barren();
    ranged::hitscan(
        SEED,
        hv(),
        &ColIndex::new(),
        &mut sc.occupants(),
        tick,
        &Rewind::new(),
        &[0; MAX_PLAYERS],
        cc,
        ps,
        &mut events,
        &mut kills,
        &mut chips,
    );
    events
}

fn count(ev: &EventQueue, code: u8) -> usize {
    ev.entries().iter().filter(|e| e.code == code).count()
}

/// The shot's line is the frame's whole yaw, not the movement LUT's 256
/// steps: a yaw one LUT step's worth of low bits away points elsewhere.
#[test]
fn the_aim_reads_every_bit_of_yaw_and_the_fine_pitch() {
    let a = ranged::aim_dir(0x0100, LEVEL, 0);
    let b = ranged::aim_dir(0x0180, LEVEL, 0);
    assert!(
        (a.0 - b.0).abs() > 1e-3,
        "half a LUT step must move the shot"
    );
    let lo = ranged::aim_dir(0, LEVEL, -100);
    let hi = ranged::aim_dir(0, LEVEL, 100);
    assert!(hi.1 > lo.1, "the fine pitch raises the shot");
    // And the wire's angles come back from a direction to within a step.
    for &(yaw, pitch, fine) in &[(0u16, 128u8, 0i8), (12_345, 140, -50), (60_000, 100, 90)] {
        let d = ranged::aim_dir(yaw, pitch, fine);
        let (y2, p2, f2) = ranged::wire_angles(d);
        let d2 = ranged::aim_dir(y2, p2, f2);
        let err = ((d.0 - d2.0).powi(2) + (d.1 - d2.1).powi(2) + (d.2 - d2.2).powi(2)).sqrt();
        assert!(
            err < 1e-3,
            "({yaw}, {pitch}, {fine}) round-tripped {err} off"
        );
    }
}

/// A draw never leaves the cone: at most half its apex off the line.
#[test]
fn the_cone_holds_every_draw_inside_half_its_apex() {
    let d = ranged::aim_dir(9_000, 135, 0);
    let apex_cdeg = 250;
    let limit = (apex_cdeg as f32 / 200.0).to_radians() + 1e-4;
    let mut widest = 0f32;
    for k in 0..2_000u64 {
        let key = sim_core::rng::splitmix64(k);
        let c = ranged::cone_dir(d, apex_cdeg, key);
        let dot = (d.0 * c.0 + d.1 * c.1 + d.2 * c.2).clamp(-1.0, 1.0);
        widest = widest.max(dot.acos());
    }
    assert!(widest <= limit, "{widest} rad past the cone's {limit}");
    assert!(
        widest > limit * 0.8,
        "the draws never reached the cone's edge"
    );
    assert_eq!(ranged::cone_dir(d, 0, 7), d, "no cone is the line itself");
}

/// Falloff: whole to the near end, the floor from the far end, a line
/// between — Rust's 5.56, whole to 20 m and 80 % from 100 m.
#[test]
fn damage_falls_off_between_the_two_ends() {
    let g = Gun {
        falloff_mm: [20_000, 100_000],
        falloff_pct: 80,
        ..Gun::NONE
    };
    assert_eq!(g.fall_off(50, 10_000), 50);
    assert_eq!(g.fall_off(50, 60_000), 45);
    assert_eq!(g.fall_off(50, 150_000), 40);
    assert_eq!(Gun::NONE.fall_off(50, 150_000), 50, "no falloff, no fall");
}

/// The cone's penalty holds through its delay and then bleeds to nothing.
#[test]
fn bloom_waits_then_bleeds_off() {
    let g = Gun {
        per_shot: 5,
        max: 60,
        recover_ticks: 12,
        delay_ticks: 3,
        ..Gun::NONE
    };
    assert_eq!(g.bloom_after(60, 2), 60, "inside the delay");
    assert_eq!(g.bloom_after(60, 9), 30, "half way through the recovery");
    assert_eq!(g.bloom_after(60, 30), 0, "settled");
}

/// A semi-automatic fires once per pull, however long it is held; let go
/// and pull again and it fires again. An automatic keeps firing.
#[test]
fn a_semi_automatic_fires_once_per_pull() {
    let semi = fixture(
        Gun {
            semi: true,
            ..Gun::NONE
        },
        10,
    );
    let mut ps = range(10.0);
    let mut shots = 0;
    for t in 0..40 {
        shots += count(&fire(t, &semi, &mut ps), EV_SHOT);
    }
    assert_eq!(shots, 1, "held for 40 ticks, one round");
    ps[0].frame.buttons = 0;
    fire(40, &semi, &mut ps);
    ps[0].frame.buttons = BTN_PRIMARY;
    assert_eq!(
        count(&fire(41, &semi, &mut ps), EV_SHOT),
        1,
        "a new pull fires"
    );

    let auto = fixture(Gun::NONE, 10);
    let mut ps = range(10.0);
    let mut shots = 0;
    for t in 0..40 {
        shots += count(&fire(t, &auto, &mut ps), EV_SHOT);
    }
    assert_eq!(shots, 10, "an automatic fires at its cadence while held");
}

/// A shotgun's pellets that find one body land on it as one blow: one
/// `EV_HIT`, the damage of every pellet that struck.
#[test]
fn pellets_that_find_one_body_land_as_one_blow() {
    let cc = fixture(
        Gun {
            semi: true,
            pellets: 15,
            pellet: 300,
            ..Gun::NONE
        },
        6,
    );
    let mut ps = range(5.0);
    let ev = fire(0, &cc, &mut ps);
    assert_eq!(
        count(&ev, EV_HIT),
        1,
        "one blow, however many pellets struck"
    );
    let took = 100 - ps[1].hp;
    assert!(
        took > 6 && took.is_multiple_of(3),
        "several pellets landed (6 a chest pellet, 3 a leg): took {took}"
    );
}

/// The cone is the shard's: from the hip at range a spray scatters, and
/// through the sights the same spray groups — fewer misses at 60 m.
#[test]
fn the_sights_tighten_the_cone_at_range() {
    let gun = Gun {
        aim: 20,
        hip: 600,
        ..Gun::NONE
    };
    let cc = fixture(gun, 1);
    let hits = |aim: bool| {
        let mut ps = range(60.0);
        let mut hits = 0;
        for t in 0..120 {
            ps[0].frame.buttons = BTN_PRIMARY | if aim { sim_core::input::BTN_AIM } else { 0 };
            ps[1].hp = 100;
            hits += count(&fire(t, &cc, &mut ps), EV_HIT);
        }
        hits
    };
    let (hip, ads) = (hits(false), hits(true));
    assert!(
        ads > hip,
        "through the sights {ads} hits, from the hip {hip}"
    );
    assert_eq!(ads, 30, "a 0.2° cone at 60 m does not miss a chest");
}

/// The bearing `EV_SHOT` reports for one hip shot on `tick`, fired through
/// the shard's pass under the world's secret seed `secret`
/// (`World::secret_seed`).
fn bearing_salted(tick: u64, secret: u64, cc: &CombatContent) -> u32 {
    use sim_core::mech::{DrumShot, Drums};
    use sim_core::mob::{MobContent, Mobs};
    let mut ps = range(20.0);
    let mut events = EventQueue::default();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let mut sc = Scratch::barren();
    let mobs = Mobs::new(SEED, hv());
    let mut hits = [ranged::MobShot::default(); MAX_ARROWS];
    let mut struck = [DrumShot::default(); MAX_ARROWS];
    let mut quarry = ranged::Quarry {
        mobs: &mobs,
        mc: &MobContent::EMPTY,
        hits: &mut hits,
        n: 0,
        drums: &Drums::NONE,
        struck: &mut struck,
        nd: 0,
    };
    ranged::hitscan_hunting(
        SEED,
        secret,
        hv(),
        &ColIndex::new(),
        &mut sc.occupants(),
        tick,
        &Rewind::new(),
        &[0; MAX_PLAYERS],
        cc,
        &mut ps,
        &mut events,
        &mut kills,
        &mut chips,
        &mut sim_core::bullet::Bullets::new(),
        &mut quarry,
    );
    let shot = events.entries().iter().find(|e| e.code == EV_SHOT);
    shot.expect("the shot fired").b
}

/// The world's secret keys the cone, so the seed, the tick and the slot a
/// shooter knows do not name where its round strays — and an unsalted
/// world's secret (its seed) is the seed's cone exactly, the unsalted
/// `hitscan` every fixture fires. Mutants: passing the seed for the secret
/// reds the first `assert_ne`, a key that ignores which secret it was
/// handed the second.
#[test]
fn the_cone_is_keyed_on_the_worlds_secret() {
    let cc = fixture(
        Gun {
            hip: 600,
            ..Gun::NONE
        },
        1,
    );
    for tick in 0..8 {
        let zero = bearing_salted(tick, SEED, &cc);
        let mut ps = range(20.0);
        let plain = fire(tick, &cc, &mut ps);
        let plain = plain.entries().iter().find(|e| e.code == EV_SHOT);
        assert_eq!(
            zero,
            plain.expect("the shot fired").b,
            "an unsalted secret is the seed's cone"
        );
        let (a, b) = (
            bearing_salted(tick, 0xA11CE, &cc),
            bearing_salted(tick, 0xB0B, &cc),
        );
        assert_ne!(a, zero, "tick {tick}: the secret never reached the cone");
        assert_ne!(a, b, "tick {tick}: two secrets drew one cone");
    }
}
