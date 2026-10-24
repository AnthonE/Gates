//! Clothing v1: Rust's rule for what a worn set does about the cold (a share
//! of it, never a fixed amount), the teas, waterproofing, drying in the cold,
//! and the two-layer pieces' clashes.

use sim_core::combat::{self, ArmorDef, CombatContent, WEAR_BODY, WEAR_SHIRT};
use sim_core::exposure::{self, Brew, ExposureContent, Inputs, BREW_WARM, NO_FLOOR};
use sim_core::gather::ItemStack;
use sim_core::survival::{self, ConsumableDef, SurvivalContent};
use sim_core::world::{EventQueue, Player};

fn body() -> Player {
    Player {
        id: 1,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    }
}

fn one(item: u16) -> ItemStack {
    ItemStack {
        item,
        count: 1,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    }
}

/// The shipped shape: the air chills 50 a degree under 13 °C, and a set's
/// warmth takes its share of 4000 per mille (Rust's 400 %).
fn rust_rule() -> ExposureContent {
    let mut ec = ExposureContent::probe_fixture();
    ec.cold_below_dc = 130;
    ec.cold_per_c = 50;
    ec.hot_above_dc = 320;
    ec.warm_full_pm = 4000;
    ec.hurt_at = 600;
    // Items 1-3: a hoodie, a jacket, a sheet of metal.
    ec.warmth[1] = 340;
    ec.warmth[2] = 400;
    ec.warmth[3] = -340;
    ec
}

fn air(dc: i16) -> Inputs {
    Inputs {
        air_dc: dc,
        ..Inputs::default()
    }
}

#[test]
fn a_set_takes_its_share_of_the_cold_and_never_all_of_it() {
    let ec = rust_rule();
    let mut p = body();
    // 3 °C: 500 bare.
    assert_eq!(exposure::target_chill(&ec, &air(30), &p), 500);
    // A hoodie and a jacket, 74 of 400: 18.5 % off.
    p.worn[(WEAR_SHIRT - 1) as usize] = one(1);
    p.worn[(WEAR_BODY - 1) as usize] = one(2);
    assert_eq!(exposure::target_chill(&ec, &air(30), &p), 408);
    // The same set at -10 °C takes the same share of a deeper cold: the
    // cold still bites, where a flat 740 off would have made it harmless.
    assert_eq!(exposure::target_chill(&ec, &air(-100), &p), 938);
    // Soaked, the clothes keep half: 9.25 % off.
    p.wet = 1000;
    let soaked = exposure::target_chill(&ec, &air(30), &p);
    assert!(soaked > 408, "wet clothes kept all their warmth: {soaked}");
    // A sheet of metal makes it worse by its share.
    let mut q = body();
    q.worn[(WEAR_BODY - 1) as usize] = one(3);
    assert_eq!(exposure::target_chill(&ec, &air(30), &q), 542);
}

#[test]
fn the_flat_rule_stands_without_a_share() {
    let mut ec = rust_rule();
    ec.warm_full_pm = 0;
    let mut p = body();
    p.worn[(WEAR_SHIRT - 1) as usize] = one(1);
    assert_eq!(
        exposure::target_chill(&ec, &air(30), &p),
        160,
        "500 less the hoodie's flat 340"
    );
}

#[test]
fn a_warming_tea_warms_the_air_floors_the_chill_and_runs_out() {
    let ec = rust_rule();
    let mut p = body();
    p.brew[BREW_WARM as usize] = Brew {
        amount: 100,
        floor_dc: NO_FLOOR,
        left_s: 2,
    };
    // Ten degrees warmer: -10 °C feels like 0 °C.
    assert_eq!(exposure::target_chill(&ec, &air(-100), &p), 650);
    // A floor at 1 °C holds the chill at 600 however cold it is.
    p.brew[BREW_WARM as usize].floor_dc = 10;
    assert_eq!(exposure::target_chill(&ec, &air(-300), &p), 600);
    exposure::steep(&mut p);
    exposure::steep(&mut p);
    assert_eq!(p.brew[BREW_WARM as usize], Brew::NONE, "the tea ran out");
    assert_eq!(exposure::target_chill(&ec, &air(-100), &p), 1000);
}

#[test]
fn drinking_a_tea_pours_it_in_and_a_second_replaces_it() {
    let mut sc = SurvivalContent::probe_fixture();
    sc.consumable[0] = ConsumableDef {
        water: 30,
        seconds: 1,
        brew: BREW_WARM + 1,
        brew_amount: 50,
        brew_floor_dc: NO_FLOOR,
        brew_s: 1200,
        ..ConsumableDef::NONE
    };
    let mut p = body();
    p.food = 10;
    p.water = 10;
    p.inv[0] = ItemStack {
        item: 0,
        count: 2,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    let mut ev = EventQueue::default();
    assert!(survival::consume(&sc, 0, &mut p, &mut ev, 100));
    assert_eq!(p.brew[BREW_WARM as usize].left_s, 1200);
    p.brew[BREW_WARM as usize].left_s = 5;
    assert!(survival::consume(&sc, 0, &mut p, &mut ev, 100));
    assert_eq!(
        p.brew[BREW_WARM as usize].left_s, 1200,
        "replaced, not stacked"
    );
}

#[test]
fn a_waterproof_body_stays_dry_in_the_rain_and_the_sea() {
    let mut ec = rust_rule();
    ec.waterproof[5] = true;
    let mut p = body();
    p.worn[(WEAR_BODY - 1) as usize] = one(5);
    let mut ev = EventQueue::default();
    let rain = Inputs {
        rain: 1000,
        depth_cm: 500,
        air_dc: 100,
        ..Inputs::default()
    };
    exposure::step(&ec, &rain, &mut p, &mut ev);
    assert_eq!(p.wet, 0);
    let mut q = body();
    exposure::step(&ec, &rain, &mut q, &mut ev);
    assert_eq!(q.wet, 1000, "the same rain and sea soak a bare body");
}

#[test]
fn clothes_do_not_dry_below_freezing_but_by_a_fire() {
    let mut ec = rust_rule();
    ec.dry_per_s = 4;
    ec.dry_ref_dc = 200;
    let mut ev = EventQueue::default();
    let mut p = body();
    p.wet = 500;
    exposure::step(&ec, &air(-50), &mut p, &mut ev);
    assert_eq!(p.wet, 500, "frozen clothes do not dry");
    exposure::step(&ec, &air(200), &mut p, &mut ev);
    assert_eq!(p.wet, 496, "the full rate at the reference air");
    exposure::step(&ec, &air(30), &mut p, &mut ev);
    assert_eq!(p.wet, 495, "a cold day dries, slowly");
    let fire = Inputs {
        fire: true,
        ..air(-50)
    };
    exposure::step(&ec, &fire, &mut p, &mut ev);
    assert_eq!(p.wet, 495 - ec.dry_fire_per_s);
}

#[test]
fn a_two_layer_piece_fills_the_slot_under_it() {
    let mut cc = CombatContent::probe_fixture();
    // Item 6 a snow jacket (the body, filling the shirt), item 7 a hoodie.
    cc.armor[6] = ArmorDef {
        reduction_pct: 15,
        slot: WEAR_BODY,
        fills: 1 << (WEAR_SHIRT - 1),
    };
    cc.armor[7] = ArmorDef {
        reduction_pct: 12,
        slot: WEAR_SHIRT,
        fills: 0,
    };
    let mut worn = [ItemStack::default(); sim_core::limits::WEAR_SLOTS];
    assert!(!combat::wear_clashes(&cc, &worn, 6, WEAR_BODY - 1));
    worn[(WEAR_BODY - 1) as usize] = one(6);
    assert!(
        combat::wear_clashes(&cc, &worn, 7, WEAR_SHIRT - 1),
        "a hoodie went on under a snow jacket"
    );
    let mut worn = [ItemStack::default(); sim_core::limits::WEAR_SLOTS];
    worn[(WEAR_SHIRT - 1) as usize] = one(7);
    assert!(
        combat::wear_clashes(&cc, &worn, 6, WEAR_BODY - 1),
        "a snow jacket went on over a hoodie"
    );
    // A plain body piece over a hoodie is Rust's layering.
    cc.armor[6].fills = 0;
    assert!(!combat::wear_clashes(&cc, &worn, 6, WEAR_BODY - 1));
}
