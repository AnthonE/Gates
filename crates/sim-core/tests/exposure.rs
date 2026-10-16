//! Wet and cold (weather v0): the rates are the rates the content states,
//! a roof and a fire do what they say, the road sign jacket draws the cold
//! in, the cold's hp is exact to the point, it kills with its own cause, and
//! it never touches a body nobody is driving. Comfort and regen are Rust's.

use sim_core::combat::CombatContent;
use sim_core::exposure::{self, ExposureContent, Inputs};
use sim_core::gather::ItemStack;
use sim_core::survival::{Step, SurvivalContent};
use sim_core::terrain;
use sim_core::weather;
use sim_core::world::{Command, EventQueue, Player, World, DEATH_BY_COLD, DEATH_BY_FEL};

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
    // The sheet of metal makes the night's cold worse by its share...
    assert_eq!(
        exposure::target_chill(&ec, &night, &sign),
        base + (base as u32 * 340 / 1000) as u16
    );
    // ...and cannot make a warm, still afternoon cold at all.
    assert_eq!(exposure::target_chill(&ec, &Inputs::default(), &sign), 0);
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

/// **The island's own roofs keep the rain off** — the haven's shelter, a
/// waystation's canopy and the depot's warehouse, against the real
/// geometry of the shipped seed. `collide::roofed` asked only built slabs,
/// so a player sheltering at spawn was soaked and chilled under a roof, and
/// the client rained inside it. Open ground beside each stays open.
#[test]
fn the_sites_own_roofs_keep_the_rain_off() {
    use sim_core::collide::{roofed, ColIndex};
    use sim_core::{depot, terrain};
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let cols = ColIndex::new();
    let under = |x: f32, z: f32, feet: f32| roofed(seed, &h, &cols, x, z, feet);

    // The shelter: standing on the plinth (its top is 0.2 m over the slot).
    let (sx, sz, _) = terrain::haven_shelter(&h);
    let floor = terrain::ground(seed, &h, sx, sz);
    assert!(
        under(sx, sz, floor + 0.2),
        "the shelter's roof keeps no rain off"
    );
    // Fifteen metres out, toward the pad's middle: open sky.
    let (dx, dz) = (h.x - sx, h.z - sz);
    let k = 15.0 / (dx * dx + dz * dz).sqrt();
    let (ox, oz) = (sx + dx * k, sz + dz * k);
    assert!(
        !under(ox, oz, terrain::ground(seed, &h, ox, oz)),
        "open pad ground reads as roofed"
    );

    // A waystation's canopy: standing on its deck (flush with the slot).
    let ws = h
        .minor
        .iter()
        .find(|w| w.live && w.kind != terrain::SiteKind::Inland)
        .expect("a live waystation");
    let (kx, kz, _) = terrain::waystation_canopy(ws);
    let deck = terrain::ground(seed, &h, kx, kz);
    assert!(under(kx, kz, deck), "the canopy keeps no rain off");
    // On top of it, a body is under nothing.
    assert!(
        !under(kx, kz, deck + 4.2),
        "the canopy roofed a body standing on it"
    );

    // The depot: inside the warehouse, and out in its open yard.
    let site = h
        .minor
        .iter()
        .find(|s| depot::is_depot(s))
        .expect("a depot");
    let (wx, wz) = depot::to_world(site, -11.0, 0.0);
    assert!(
        under(wx, wz, site.floor_y),
        "the warehouse roof keeps no rain off"
    );
    let (yx, yz) = depot::to_world(site, 0.0, 0.0);
    assert!(
        !under(yx, yz, site.floor_y),
        "the open yard reads as roofed"
    );
}

/// **The road sign jacket does not freeze its wearer on a clear night.**
/// With the shipped `[exposure]` numbers a naked, dry body on a clear night
/// heads for 380 and takes nothing; the jacket drew a flat 340 in on top
/// and put the same night at 720, past `hurt_at`, so the best early armour
/// cost hp every night while going naked cost none. It still chills.
#[test]
fn the_road_sign_jacket_chills_a_clear_night_without_hurting() {
    let mut ec = ExposureContent::probe_fixture();
    // The shipped `balance.toml` `[exposure]` rows this is about.
    ec.night_cold = 350;
    ec.rain_cold = 250;
    ec.wind_cold = 150;
    ec.wet_cold = 450;
    ec.hurt_at = 600;
    ec.warmth[2] = -340; // armor.toml: the road sign jacket's cold_pct × 10
    let clear = weather::preset(weather::CLEAR, 0);
    let night = Inputs {
        rain: clear.rain,
        wind: clear.wind,
        night: true,
        ..Inputs::default()
    };
    let naked = body();
    let bare = exposure::target_chill(&ec, &night, &naked);
    assert_eq!(bare, 380, "naked, dry, clear night");
    let mut sign = body();
    sign.worn[1] = ItemStack {
        item: 2,
        count: 1,
        cond: 100,
        skin: 0,
    };
    let worn = exposure::target_chill(&ec, &night, &sign);
    assert!(worn > bare, "the jacket no longer draws any cold in");
    assert!(
        worn < ec.hurt_at,
        "the jacket alone makes a clear, dry night cost hp: {worn}"
    );
}

/// The cold eats (`NOW.md` §0wx item 1): at full chill, with
/// `cold_food_pct` 100, food drains twice as fast as a warm body's, and
/// water — at 0 — drains the same.
#[test]
fn the_cold_burns_food() {
    let mut sc = SurvivalContent::probe_fixture();
    sc.exposure = ExposureContent::probe_fixture();
    sc.exposure.cold_food_pct = 100;
    let mut warm = body();
    sim_core::survival::grant(&sc, &mut warm);
    let mut cold = warm;
    cold.chill = 1000;
    let mut ev = EventQueue::default();
    for t in 0..sim_core::limits::TICK_HZ * 3 {
        if t % sim_core::limits::TICK_HZ == 0 {
            exposure::burn(&sc.exposure, sc.max_food, sc.max_water, &mut warm);
            exposure::burn(&sc.exposure, sc.max_food, sc.max_water, &mut cold);
        }
        sim_core::survival::step(&sc, &mut warm, &mut ev);
        sim_core::survival::step(&sc, &mut cold, &mut ev);
    }
    let used = |p: &Player| (sc.max_food - p.food) as i32;
    assert!(used(&warm) > 0, "the fixture's clock never drained");
    assert!(
        (used(&cold) - 2 * used(&warm)).abs() <= 1,
        "cold used {} food against warm's {}",
        used(&cold),
        used(&warm)
    );
    assert_eq!(cold.water, warm.water, "the cold burned water at 0%");
}

/// Rust's regen, fast: a point a second, nothing on the stomach or the
/// comfort, no calm to wait out.
fn regen_fixture() -> ExposureContent {
    let mut ec = ExposureContent::probe_fixture();
    ec.fire_comfort = 500;
    ec.company_comfort = 250;
    ec.comfort_per_s = 200;
    ec.regen_hp_per_min = 60;
    ec.regen_cap_pct = 60;
    ec.regen_food_min = 100;
    ec.regen_water_min = 40;
    ec.regen_chill_max = 300;
    ec
}

fn fed() -> Player {
    Player {
        food: 500,
        water: 250,
        ..body()
    }
}

fn calm() -> Inputs {
    Inputs {
        max_food: 500,
        max_water: 250,
        ..Inputs::default()
    }
}

/// Comfort is Rust's campfire: half by a lit fire, a quarter more for each
/// other body at it and an eighth for a sleeper, and nothing away from one.
/// The meter follows its target at `comfort_per_s`.
#[test]
fn a_fire_gives_half_comfort_and_company_adds() {
    let ec = regen_fixture();
    assert_eq!(exposure::comfort_target(&ec, false, 3, 0), 0);
    assert_eq!(exposure::comfort_target(&ec, true, 0, 0), 500);
    assert_eq!(exposure::comfort_target(&ec, true, 1, 1), 875);
    assert_eq!(exposure::comfort_target(&ec, true, 4, 0), 1000, "capped");
    let mut p = fed();
    let mut ev = EventQueue::default();
    let fire = Inputs {
        fire: true,
        comfort: 500,
        ..calm()
    };
    exposure::step(&ec, &fire, &mut p, &mut ev);
    assert_eq!(p.comfort, 200, "it climbs, it does not jump");
    for _ in 0..2 {
        exposure::step(&ec, &fire, &mut p, &mut ev);
    }
    assert_eq!(p.comfort, 500);
    exposure::step(&ec, &calm(), &mut p, &mut ev);
    assert_eq!(p.comfort, 300, "and falls away from the fire");
    assert_eq!(exposure::readout(&ec, &p).comfort_pct, 30);
}

/// Rust's heal-to: 60 % with no comfort, 80 % by a fire (50 % comfort),
/// full at full comfort.
#[test]
fn comfort_lifts_how_far_the_body_mends() {
    let ec = regen_fixture();
    let mut ev = EventQueue::default();
    let mended = |comfort: u16, ev: &mut EventQueue| {
        let mut p = fed();
        p.hp = 10;
        let inp = Inputs { comfort, ..calm() };
        for _ in 0..200 {
            exposure::step(&ec, &inp, &mut p, ev);
        }
        p.hp
    };
    assert_eq!(mended(0, &mut ev), 60);
    assert_eq!(mended(500, &mut ev), 80);
    assert_eq!(mended(1000, &mut ev), 100);
}

/// Comfort makes it faster too: `regen_comfort_pct` at full comfort, and a
/// full stomach adds `regen_fed_pct`.
#[test]
fn comfort_and_a_full_stomach_speed_the_mend() {
    let mut ec = regen_fixture();
    ec.regen_hp_per_min = 6;
    ec.regen_comfort_pct = 600;
    ec.regen_fed_pct = 50;
    let mut ev = EventQueue::default();
    let minute = |comfort: u16, food: u16, ev: &mut EventQueue| {
        let mut p = fed();
        p.food = food;
        p.comfort = comfort;
        p.hp = 10;
        let inp = Inputs { comfort, ..calm() };
        for _ in 0..60 {
            exposure::step(&ec, &inp, &mut p, ev);
        }
        p.hp - 10
    };
    assert_eq!(minute(0, 250, &mut ev), 7, "6 × 1.25 a minute, half fed");
    assert_eq!(minute(0, 500, &mut ev), 9, "6 × 1.5, full");
    assert_eq!(minute(500, 500, &mut ev), 36, "× 4 by a fire");
}

/// Rust's gates: food over 100, water over 40, warm enough, and ten calm
/// seconds after any hp lost. Mending eats.
#[test]
fn regen_wants_food_water_warmth_and_calm_and_eats() {
    let mut ec = regen_fixture();
    ec.regen_calm_s = 10;
    ec.regen_food_pct = 100;
    let mut ev = EventQueue::default();
    let run = |mut p: Player, inp: Inputs, ev: &mut EventQueue| {
        p.hp = 10;
        for _ in 0..20 {
            exposure::step(&ec, &inp, &mut p, ev);
        }
        p
    };
    assert_eq!(
        run(fed(), calm(), &mut ev).hp,
        21,
        "ten calm seconds, then a point a second"
    );
    let hungry = Player { food: 100, ..fed() };
    assert_eq!(
        run(hungry, calm(), &mut ev).hp,
        10,
        "100 food is not over 100"
    );
    let thirsty = Player { water: 40, ..fed() };
    assert_eq!(run(thirsty, calm(), &mut ev).hp, 10);
    let cold = Inputs {
        night: true,
        ..calm()
    };
    let chilled = Player {
        chill: 500,
        ..fed()
    };
    assert_eq!(run(chilled, cold, &mut ev).hp, 10, "too cold to mend");
    let downed = Player {
        wounded: true,
        ..fed()
    };
    assert_eq!(run(downed, calm(), &mut ev).hp, 10);

    // A hit restarts the wait.
    let mut p = fed();
    p.hp = 50;
    p.calm = 100;
    sim_core::combat::hurt_unreduced(&mut p, 1);
    assert_eq!(p.calm, 0);

    // The bill: a second's normal drain on top for each second mending
    // (`regen_food_pct` 100), and nothing for a body past its cap.
    let mended = run(fed(), calm(), &mut ev);
    assert_eq!(mended.food_acc, 11 * 500 * sim_core::limits::TICK_HZ);
    let mut whole = fed();
    for _ in 0..20 {
        exposure::step(&ec, &calm(), &mut whole, &mut ev);
    }
    assert_eq!(whole.food_acc, 0, "a whole body does not pay");
}

/// The air's fixture (climate v0): cold under 14 °C, hot over 30 °C.
fn air_fixture() -> ExposureContent {
    let mut ec = ExposureContent::probe_fixture();
    ec.cold_below_dc = 140;
    ec.cold_per_c = 50;
    ec.hot_above_dc = 300;
    ec.heat_per_c = 50;
    ec.shade_dc = 80;
    ec.wet_cool = 600;
    ec.clothes_heat_pm = 500;
    ec.heat_water_pct = 150;
    ec.heat_hurt_at = 600;
    ec.heat_hp_per_min = 60;
    ec
}

/// With the air armed, the cold is the air's: every degree under
/// `cold_below_dc`, night or day, and nothing at a mild night.
#[test]
fn the_air_chills_and_the_night_alone_does_not() {
    let ec = air_fixture();
    let p = body();
    let frost = Inputs {
        air_dc: 40,
        ..Inputs::default()
    };
    assert_eq!(
        exposure::target_chill(&ec, &frost, &p),
        500,
        "ten degrees under"
    );
    let mild_night = Inputs {
        night: true,
        air_dc: 160,
        ..Inputs::default()
    };
    assert_eq!(exposure::target_chill(&ec, &mild_night, &p), 0);
    // Rain chills a cool day in full, a warm one less, a hot one not at all.
    let rain = |air_dc| Inputs {
        rain: 1000,
        air_dc,
        ..Inputs::default()
    };
    assert_eq!(exposure::target_chill(&ec, &rain(140), &p), ec.rain_cold);
    assert_eq!(
        exposure::target_chill(&ec, &rain(220), &p),
        ec.rain_cold / 2
    );
    assert_eq!(exposure::target_chill(&ec, &rain(300), &p), 0);
}

/// The desert's noon: heat over `hot_above_dc`, cut by shade and by being
/// wet, worse in warm clothes; past `heat_hurt_at` it costs hp, and the
/// rain and wind that would chill a cool body do not chill a hot one.
#[test]
fn the_heat_is_the_airs_and_shade_water_and_clothes_move_it() {
    let ec = air_fixture();
    let naked = body();
    let noon = Inputs {
        air_dc: 420,
        wind: 1000,
        ..Inputs::default()
    };
    assert_eq!(exposure::target_heat(&ec, &noon, &naked), 600);
    assert_eq!(
        exposure::target_chill(&ec, &noon, &naked),
        0,
        "a hot wind chilled"
    );
    let shade = Inputs {
        roofed: true,
        ..noon
    };
    assert_eq!(exposure::target_heat(&ec, &shade, &naked), 200);
    let mut soaked = body();
    soaked.wet = 1000;
    assert_eq!(exposure::target_heat(&ec, &noon, &soaked), 0);
    let mut coat = body();
    coat.worn[1] = ItemStack {
        item: 0,
        count: 1,
        cond: 100,
        skin: 0,
    };
    let held = ec.warmth[0] as u16 * ec.clothes_heat_pm / 1000;
    assert_eq!(exposure::target_heat(&ec, &noon, &coat), 600 + held);
    assert_eq!(exposure::target_heat(&ec, &Inputs::default(), &coat), 0);

    // In the coat at noon it hurts; naked at the threshold it does not.
    let mut ev = EventQueue::default();
    let mut p = coat;
    for _ in 0..60 {
        exposure::step(&ec, &noon, &mut p, &mut ev);
    }
    assert!(p.hp < 100, "the heat past heat_hurt_at took nothing");
    assert_eq!(exposure::readout(&ec, &p).air_c, 42);
    assert!(exposure::readout(&ec, &p).hurting);
    let mut q = naked;
    for _ in 0..60 {
        exposure::step(&ec, &noon, &mut q, &mut ev);
    }
    assert_eq!(q.hp, 100);
    assert_eq!(exposure::readout(&ec, &q).heat_pct, 60);
}

/// The heat sweats water away: `heat_water_pct` 150 at heat 600 is +90 %.
#[test]
fn the_heat_burns_water() {
    let mut sc = SurvivalContent::probe_fixture();
    sc.exposure = air_fixture();
    let mut cool = body();
    sim_core::survival::grant(&sc, &mut cool);
    let mut hot = cool;
    hot.heat = 600;
    let mut ev = EventQueue::default();
    for t in 0..sim_core::limits::TICK_HZ * 3 {
        if t % sim_core::limits::TICK_HZ == 0 {
            exposure::burn(&sc.exposure, sc.max_food, sc.max_water, &mut cool);
            exposure::burn(&sc.exposure, sc.max_food, sc.max_water, &mut hot);
        }
        sim_core::survival::step(&sc, &mut cool, &mut ev);
        sim_core::survival::step(&sc, &mut hot, &mut ev);
    }
    let used = |p: &Player| (sc.max_water - p.water) as i32;
    assert!(used(&cool) > 0, "the fixture's clock never drained");
    assert!(
        (used(&hot) * 10 - used(&cool) * 19).abs() <= 19,
        "hot used {} water against cool's {}",
        used(&hot),
        used(&cool)
    );
    assert_eq!(hot.food, cool.food, "the heat burned food");
}

// --- The Verdigris (`exposure::corrode`) ------------------------------------

/// The Verdigris armed alone: no cold, so nothing else can move the hp.
fn fel_fixture() -> ExposureContent {
    let mut ec = ExposureContent::EMPTY;
    ec.fel_rise_per_s = 6;
    ec.fel_vent_per_s = 60;
    ec.fel_fall_per_s = 5;
    ec.fel_hurt_at = 450;
    ec.fel_hp_per_min = 20;
    ec.fel_regen_max = 200;
    ec.fel_ward_max = 900;
    ec
}

fn deep(fel_pm: u16, vent: bool) -> Inputs {
    Inputs {
        fel_pm,
        vent,
        ..Inputs::default()
    }
}

/// The meter rises by how deep the body stands — rounded up, so the edge
/// still creeps — a vent's fire adds its own, and out of it the meter falls.
#[test]
fn the_verdigris_rises_with_its_depth_and_falls_outside_it() {
    let ec = fel_fixture();
    let mut p = body();
    let mut ev = EventQueue::default();
    for _ in 0..10 {
        exposure::corrode(&ec, &deep(1000, false), &mut p, &mut ev);
    }
    assert_eq!(p.fel, 60, "six a second at the heart");
    exposure::corrode(&ec, &deep(50, false), &mut p, &mut ev);
    assert_eq!(p.fel, 61, "the edge creeps by one, never by none");
    exposure::corrode(&ec, &deep(1000, true), &mut p, &mut ev);
    assert_eq!(p.fel, 61 + 66, "a vent's fire on top of the heart");
    for _ in 0..10 {
        exposure::corrode(&ec, &Inputs::default(), &mut p, &mut ev);
    }
    assert_eq!(p.fel, 127 - 50, "five a second out of it");
    // Disarmed content leaves a clean body alone.
    let mut q = body();
    exposure::corrode(
        &ExposureContent::probe_fixture(),
        &deep(1000, true),
        &mut q,
        &mut ev,
    );
    assert_eq!(q.fel, 0);
}

/// What is worn wards off its share of the rise, and no set past the cap.
#[test]
fn what_you_wear_wards_it_off_up_to_the_cap() {
    let mut ec = fel_fixture();
    ec.ward[1] = 500;
    ec.ward[2] = 600;
    let mut ev = EventQueue::default();
    let mut half = body();
    half.worn[0] = ItemStack {
        item: 1,
        count: 1,
        cond: 100,
        skin: 0,
    };
    for _ in 0..10 {
        exposure::corrode(&ec, &deep(1000, true), &mut half, &mut ev);
    }
    assert_eq!(half.fel, 10 * 33, "66 a second, half warded");
    let mut both = half;
    both.fel = 0;
    both.worn[1] = ItemStack {
        item: 2,
        count: 1,
        cond: 100,
        skin: 0,
    };
    for _ in 0..10 {
        exposure::corrode(&ec, &deep(1000, true), &mut both, &mut ev);
    }
    assert_eq!(
        both.fel,
        10 * 7,
        "a set past 90% wards 90%, and the rest rounds up"
    );
}

/// Past the line it costs hp, exact over the minute; under it nothing; and
/// over `fel_regen_max` a body does not mend.
#[test]
fn the_verdigris_hp_is_exact_and_it_stops_the_mend() {
    let mut ec = fel_fixture();
    ec.fel_rise_per_s = 0;
    ec.fel_fall_per_s = 0;
    ec.fel_hurt_at = 0;
    ec.fel_hp_per_min = 60;
    let mut p = body();
    p.fel = 1000;
    let mut ev = EventQueue::default();
    for _ in 0..60 {
        exposure::corrode(&ec, &Inputs::default(), &mut p, &mut ev);
    }
    assert_eq!(p.hp, 40, "60 a minute at a full meter is one a second");
    let ec = fel_fixture();
    let mut q = body();
    q.fel = 450;
    for _ in 0..60 {
        exposure::corrode(&ec, &deep(0, false), &mut q, &mut ev);
    }
    assert_eq!(q.hp, 100, "at or under the line it costs nothing");
    let mut r = fed();
    r.hp = 50;
    r.calm = u16::MAX;
    let mut regen = regen_fixture();
    regen.fel_regen_max = 200;
    assert!(exposure::regens(&regen, &r));
    r.fel = 201;
    assert!(
        !exposure::regens(&regen, &r),
        "corroded past the line, no mend"
    );
}

/// In a world: the heart of the Verdigris kills with its own cause, and the
/// beach where it reaches the sea is safe.
#[test]
fn the_heart_kills_with_its_own_cause_and_the_beach_is_safe() {
    let mut w = World::new(20260731);
    w.combat = CombatContent::probe_fixture();
    let mut sc = SurvivalContent::EMPTY;
    sc.exposure = fel_fixture();
    sc.exposure.fel_hp_per_min = 600;
    sc.exposure.fel_hurt_at = 100;
    w.survival = sc;
    w.dev_spawn = Some(terrain::FEL_HEART);
    w.tick(&[Command::Join { id: 1 }]);
    let mut died = false;
    for _ in 0..(120 * 30) {
        w.tick(&[]);
        if w.players[0].dead {
            died = true;
            break;
        }
    }
    assert!(
        died,
        "a naked body at the heart of the Verdigris lived 2 min"
    );
    assert_eq!(w.players[0].death_cause, DEATH_BY_FEL);
    let day = weather::day_tick(0, &w.env);
    let wx = weather::now(w.seed, 0, &w.env);
    // Any Verdigris beach: the meter's input there is zero.
    let (hx, hz) = terrain::FEL_HEART;
    let mut beach = None;
    'find: for iz in -150..150 {
        for ix in -150..150 {
            let (x, z) = (hx + ix as f32 * 4.0, hz + iz as f32 * 4.0);
            // The surface a body stands on, carve and all.
            let h = terrain::ground(w.seed, &w.haven, x, z);
            if (terrain::LAND_MIN_H..terrain::BEACH_MAX_H).contains(&h)
                && terrain::fel(w.seed, x, z) > 0.2
            {
                beach = Some((x, z));
                break 'find;
            }
        }
    }
    if let Some((x, z)) = beach {
        w.players[0].body = sim_core::movement::Body::at(w.seed, &w.haven, x, z);
        let inp = w.exposure_inputs(0, &wx, false, day);
        assert_eq!(inp.fel_pm, 0, "the Verdigris' beach at ({x},{z}) corrodes");
    }
}

/// Snow (snow v0): where the air freezes the rain falls as snow and wets a
/// body at `snow_wet_pm` of the rain's rate, sleet at a share of each, and
/// a roof keeps it off the same. The air alone says what falls — it is the
/// reading the owner's sky is drawn from — so content whose cold does not
/// read the air still has its snow wet as snow.
#[test]
fn snow_wets_less_than_rain() {
    let mut ec = air_fixture();
    ec.snow_wet_pm = 500;
    let wet_after = |ec: &ExposureContent, air_dc: i16, roofed: bool| {
        let fall = Inputs {
            rain: 1000,
            air_dc,
            roofed,
            ..Inputs::default()
        };
        let mut p = body();
        let mut ev = EventQueue::default();
        for _ in 0..2 {
            exposure::step(ec, &fall, &mut p, &mut ev);
        }
        p.wet
    };
    let rain = wet_after(&ec, 60, false);
    let sleet = wet_after(&ec, 5, false);
    let snow = wet_after(&ec, -80, false);
    assert_eq!(rain, 2 * ec.wet_rain_per_s);
    assert_eq!(snow, rain / 2, "snow wets at half the rain's rate");
    assert!(snow < sleet && sleet < rain, "{snow} < {sleet} < {rain}");
    let freezing = Inputs {
        air_dc: 0,
        ..Inputs::default()
    };
    assert_eq!(exposure::snow_pm(&freezing), 1000);
    assert_eq!(wet_after(&ec, -80, true), 0, "a roof keeps the snow off");
    // Heat only: the cold ignores the air, and the snow still does not.
    let heat_only = ExposureContent {
        cold_per_c: 0,
        ..ec
    };
    assert_eq!(wet_after(&heat_only, -80, false), snow, "the sky says snow");
}

/// A body reads the air at the door (snow v0). The owner's first readout
/// goes out before the body's first exposure step, which is up to a second
/// away on its stagger, and a default 0 °C there is snow in its sky.
#[test]
fn a_body_reads_the_air_at_the_door() {
    let mut w = cold_world();
    // Mild everywhere at every hour, so the reading is one number.
    w.survival.exposure.air = sim_core::climate::ClimateContent {
        temperate_low_dc: 150,
        temperate_high_dc: 150,
        arid_low_dc: 150,
        arid_high_dc: 150,
        ..sim_core::climate::ClimateContent::EMPTY
    };
    // Off the step's tick, so only the door can have read it.
    w.tick(&[]);
    w.tick(&[]);
    w.tick(&[Command::Join { id: 1 }]);
    assert!(w.players[0].active);
    assert_eq!(w.players[0].air_dc, 150, "the body came in at 0 °C");
}
