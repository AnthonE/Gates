//! The water pass at the level `movement` and `World::breathe` cannot see
//! alone: a body in a lake floats with its head out, one held under drowns —
//! a death like any other, on the reference's clock — and coming up gives
//! the breath back.

use sim_core::combat::CombatContent;
use sim_core::input::{InputFrame, BTN_CROUCH};
use sim_core::limits::TICK_HZ;
use sim_core::movement::{mouth_under, Body, FLOAT_DEPTH_M, POS_Y_Q};
use sim_core::survival::SurvivalContent;
use sim_core::terrain::{self, ISLAND_SIZE, SEA_LEVEL};
use sim_core::world::{Command, World, DEATH_BY_DROWN};

/// The shipped seed: its lakes are carved up to `LAKE_DEPTH_M` deep.
const SEED: u64 = 20260731;

/// One body with hp and drowning armed, and nothing else on the clock — the
/// survival fixture's seconds-long thirst would kill it first.
fn world() -> World {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::EMPTY;
    w.survival.exposure.air_ticks = (10 * TICK_HZ) as u16;
    w.survival.exposure.drown_hp_per_s = 20;
    w.tick(&[Command::Join { id: 1 }]);
    w
}

/// A point in a lake more than three metres deep.
fn deep_lake() -> (f32, f32) {
    let mut x = 8.0f32;
    while x < ISLAND_SIZE {
        let mut z = 8.0f32;
        while z < ISLAND_SIZE {
            let (h, open) = terrain::height_open(SEED, x, z);
            if h < SEA_LEVEL - 3.0 && open < 0.5 {
                return (x, z);
            }
            z += 16.0;
        }
        x += 16.0;
    }
    panic!("seed {SEED} has no lake three metres deep");
}

fn on_the_bed(w: &mut World) {
    let (x, z) = deep_lake();
    w.players[0].body = Body::at(w.seed, &w.haven, x, z);
}

fn hold(w: &mut World, buttons: u8, ticks: u32) {
    for t in 0..ticks {
        let frame = InputFrame {
            seq: t as u16,
            buttons,
            ..InputFrame::default()
        };
        w.tick(&[Command::Input {
            id: 1,
            frame,
            favour: 0,
        }]);
    }
}

fn feet(w: &World) -> f32 {
    w.players[0].body.qy as f32 * POS_Y_Q
}

#[test]
fn a_body_in_a_lake_floats_with_its_head_out() {
    let mut w = world();
    on_the_bed(&mut w);
    assert!(mouth_under(feet(&w)), "the fixture starts on the bed");
    hold(&mut w, 0, 10 * TICK_HZ);
    let float = SEA_LEVEL - FLOAT_DEPTH_M;
    assert!(
        (float - 0.05..float + 0.05).contains(&feet(&w)),
        "floated to {}, not the float line {float}",
        feet(&w)
    );
    assert!(!mouth_under(feet(&w)), "a floating body breathes");
    assert_eq!(w.players[0].under, 0, "and has its whole breath back");
    assert!(!w.players[0].dead);
}

#[test]
fn holding_a_dive_drowns_on_the_references_clock() {
    let mut w = world();
    on_the_bed(&mut w);
    let hp = w.players[0].hp;
    assert!(hp > 0, "the combat fixture grants hp");
    // The first half of the breath is free.
    hold(&mut w, BTN_CROUCH, 5 * TICK_HZ - 5);
    assert_eq!(w.players[0].hp, hp, "the first five seconds cost nothing");
    let mut died = None;
    for s in 0..15 * TICK_HZ {
        hold(&mut w, BTN_CROUCH, 1);
        if w.players[0].dead {
            died = Some(s);
            break;
        }
    }
    let died = died.expect("a body held under never drowned");
    assert_eq!(w.players[0].death_cause, DEATH_BY_DROWN);
    // Rust's oxygen: about 12.5 s from full, so 7.5 s past the free half.
    let secs = (5 * TICK_HZ + died) as f32 / TICK_HZ as f32;
    assert!(
        (9.0..15.0).contains(&secs),
        "drowned after {secs} s held under"
    );
}

#[test]
fn surfacing_gives_the_breath_back() {
    let mut w = world();
    on_the_bed(&mut w);
    hold(&mut w, BTN_CROUCH, 4 * TICK_HZ);
    assert!(
        w.players[0].under > 3 * TICK_HZ as u16,
        "a dive spends breath"
    );
    hold(&mut w, 0, 6 * TICK_HZ);
    assert!(!mouth_under(feet(&w)), "let go, the body comes up");
    assert_eq!(w.players[0].under, 0, "and breathes again");
}

/// A standable point on the sea's shore with no lake inside the drink's
/// reach, so a dip there is salt.
fn seashore() -> (f32, f32) {
    let r = sim_core::survival::DRINK_REACH_M;
    let mut x = 8.0f32;
    while x < ISLAND_SIZE {
        let mut z = 8.0f32;
        while z < ISLAND_SIZE {
            let h = terrain::height(SEED, x, z);
            if (SEA_LEVEL + 0.2..terrain::BEACH_MAX_H).contains(&h) {
                let taps = [(r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)]
                    .map(|(dx, dz)| terrain::height_open(SEED, x + dx, z + dz));
                let wet = |t: &(f32, f32)| t.0 < SEA_LEVEL;
                if taps.iter().any(wet) && taps.iter().filter(|t| wet(t)).all(|t| t.1 >= 0.5) {
                    return (x, z);
                }
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("seed {SEED} has no sea shore");
}

#[test]
fn a_vessel_fills_with_the_sea_and_pours_out_on_a_fire() {
    use sim_core::survival::{Vessel, Water};
    let skin = 3u16;
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.survival.vessels[0] = Vessel {
        item: skin,
        ml: 1000,
        sip_ml: 250,
        water_per_l: 100,
        salt: true,
    };
    w.survival.vessel_count = 1;
    w.tick(&[Command::Join { id: 1 }]);
    let (x, z) = seashore();
    w.players[0].body = Body::at(w.seed, &w.haven, x, z);
    w.players[0].inv[0] = sim_core::gather::ItemStack {
        item: skin,
        count: 1,
        ..sim_core::gather::ItemStack::EMPTY
    };
    w.tick(&[Command::Drink { id: 1 }]);
    assert_eq!(
        Water::of(w.players[0].inv[0].data),
        Water {
            ml: 1000,
            salt: true
        },
        "dipped in the sea, it holds a litre of salt"
    );

    // A fire at the feet, inside the splash of a pour.
    let b = w.players[0].body;
    w.fires.light(sim_core::fire::FireRec {
        qx: b.qx,
        qy: b.qy,
        qz: b.qz,
        until: w.tick + 600,
        owner: 0,
        item: 0,
        range_cm: 0,
    });
    hold(&mut w, sim_core::input::BTN_AIM, 1);
    assert!(w.fires.is_empty(), "the pour put the fire out");
    assert_eq!(w.players[0].inv[0].data, 0, "and left the vessel empty");
}
