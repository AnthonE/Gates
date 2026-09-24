//! Wet and cold (weather v0): the rates are the rates the content states,
//! a roof and a fire do what they say, the road sign jacket draws the cold
//! in, the cold's hp is exact to the point, it kills with its own cause, and
//! it never touches a body nobody is driving.

use sim_core::combat::CombatContent;
use sim_core::exposure::{self, ExposureContent, Inputs};
use sim_core::gather::ItemStack;
use sim_core::survival::{Step, SurvivalContent};
use sim_core::weather;
use sim_core::world::{Command, EventQueue, Player, World, DEATH_BY_COLD};

fn body() -> Player {
    Player {
        id: 1,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    }
}

fn storm() -> Inputs {
    Inputs {
        rain: 1000,
        wind: 1000,
        night: true,
        ..Inputs::default()
    }
}

#[test]
fn rain_soaks_an_uncovered_body_and_a_roof_stops_it() {
    let ec = ExposureContent::probe_fixture();
    let mut p = body();
    let mut ev = EventQueue::default();
    exposure::step(&ec, &storm(), &mut p, &mut ev);
    assert_eq!(p.wet, ec.wet_rain_per_s, "one second of full rain");
    // Half the rain soaks half as fast.
    let mut q = body();
    let half = Inputs {
        rain: 500,
        ..storm()
    };
    exposure::step(&ec, &half, &mut q, &mut ev);
    assert_eq!(q.wet, ec.wet_rain_per_s / 2);
    // Under a roof it dries instead, at the dry rate — and faster by a fire.
    let roof = Inputs {
        roofed: true,
        ..storm()
    };
    exposure::step(&ec, &roof, &mut p, &mut ev);
    assert_eq!(p.wet, ec.wet_rain_per_s - ec.dry_per_s);
    let fire = Inputs { fire: true, ..roof };
    let before = p.wet;
    exposure::step(&ec, &fire, &mut p, &mut ev);
    assert_eq!(p.wet, before.saturating_sub(ec.dry_fire_per_s));
}

#[test]
fn the_sea_soaks_you_to_the_depth_you_stand_in() {
    let ec = ExposureContent::probe_fixture();
    let mut p = body();
    let mut ev = EventQueue::default();
    let wade = Inputs {
        depth_cm: ec.soak_depth_cm / 2,
        ..Inputs::default()
    };
    exposure::step(&ec, &wade, &mut p, &mut ev);
    assert_eq!(p.wet, 500, "waist-deep is half soaked");
    let swim = Inputs {
        depth_cm: ec.soak_depth_cm * 3,
        ..Inputs::default()
    };
    exposure::step(&ec, &swim, &mut p, &mut ev);
    assert_eq!(p.wet, 1000);
}

#[test]
fn what_you_wear_keeps_cold_out_and_a_road_sign_lets_it_in() {
    let mut ec = ExposureContent::probe_fixture();
    // Item 1 a coat, item 2 a sheet of metal.
    ec.warmth[1] = 270;
    ec.warmth[2] = -340;
    let night = Inputs {
        night: true,
        ..Inputs::default()
    };
    let naked = body();
    let base = exposure::target_chill(&ec, &night, &naked);
    assert_eq!(base, ec.night_cold);
    let mut coat = body();
    coat.worn[1] = ItemStack {
        item: 1,
        count: 1,
        cond: 100,
        skin: 0,
    };
    assert_eq!(exposure::target_chill(&ec, &night, &coat), base - 270);
    // Soaked, the coat keeps half.
    coat.wet = 1000;
    let wet_target = exposure::target_chill(&ec, &night, &coat);
    assert_eq!(wet_target, base + ec.wet_cold - 135);
    let mut sign = body();
    sign.worn[1] = ItemStack {
        item: 2,
        count: 1,
        cond: 100,
        skin: 0,
    };
    assert_eq!(exposure::target_chill(&ec, &night, &sign), base + 340);
    // A roof and a fire take anyone to zero — even soaked, even at night.
    // A fire out in the storm does not: the rain and the wind still reach.
    let shelter = Inputs {
        fire: true,
        roofed: true,
        ..storm()
    };
    let mut soaked = body();
    soaked.wet = 1000;
    assert_eq!(exposure::target_chill(&ec, &shelter, &soaked), 0);
    let exposed_fire = Inputs {
        fire: true,
        ..storm()
    };
    assert!(exposure::target_chill(&ec, &exposed_fire, &soaked) > 0);
}

#[test]
fn the_colds_hp_is_exact_to_the_point() {
    let mut ec = ExposureContent::probe_fixture();
    // At full chill with the threshold at zero: 60 hp a minute is exactly
    // one a second, and a minute costs exactly sixty.
    ec.hurt_at = 0;
    ec.hurt_hp_per_min = 60;
    let mut p = body();
    p.chill = 1000;
    let mut ev = EventQueue::default();
    for _ in 0..60 {
        exposure::step(&ec, &storm(), &mut p, &mut ev);
    }
    assert_eq!(p.hp, 40);
    // Halfway from the threshold to full, half the rate: 30 hp a minute.
    // The chill is held there (it never falls in this table).
    ec.hurt_at = 500;
    ec.chill_fall_per_s = 0;
    let mut q = body();
    q.chill = 750;
    for _ in 0..60 {
        assert_ne!(
            exposure::step(&ec, &Inputs::default(), &mut q, &mut ev),
            Step::Died
        );
    }
    assert_eq!(q.chill, 750);
    assert_eq!(q.hp, 70, "750 over a 500 threshold is half of 60 a minute");
}

/// A world with only exposure armed: the meters stay out of it, so the only
/// thing that can kill this body is the cold.
fn cold_world() -> World {
    let mut w = World::new(11);
    w.combat = CombatContent::probe_fixture();
    let mut sc = SurvivalContent::EMPTY;
    sc.exposure = ExposureContent::probe_fixture();
    w.survival = sc;
    w.dev_spawn = Some(w.spawn_pos(1));
    w
}

#[test]
fn a_storm_at_night_kills_with_the_colds_own_cause() {
    let mut w = cold_world();
    w.tick(&[
        Command::Join { id: 1 },
        Command::AdminEnv {
            weather: weather::STORM,
            time_pm: 937,
        },
    ]);
    let mut died = false;
    for _ in 0..(90 * 30) {
        w.tick(&[]);
        if w.players[0].dead {
            died = true;
            break;
        }
    }
    assert!(died, "a naked body out in a night storm lived 90 s");
    assert_eq!(w.players[0].death_cause, DEATH_BY_COLD);
}

#[test]
fn a_sleeper_is_left_out_of_the_weather() {
    let mut w = cold_world();
    w.tick(&[
        Command::Join { id: 1 },
        Command::AdminEnv {
            weather: weather::STORM,
            time_pm: 937,
        },
    ]);
    w.tick(&[Command::Leave { id: 1 }]);
    assert!(w.players[0].sleeping);
    let before = (w.players[0].wet, w.players[0].chill, w.players[0].hp);
    for _ in 0..(60 * 30) {
        w.tick(&[]);
    }
    let p = &w.players[0];
    assert!(!p.dead, "an offline body froze in a storm it never saw");
    assert_eq!((p.wet, p.chill, p.hp), before, "a sleeper's exposure moved");
}

#[test]
fn inert_content_changes_nothing() {
    let ec = ExposureContent::EMPTY;
    let mut p = body();
    let mut ev = EventQueue::default();
    assert_eq!(exposure::step(&ec, &storm(), &mut p, &mut ev), Step::Quiet);
    assert_eq!((p.wet, p.chill, p.hp), (0, 0, 100));
}
