//! A syringe or a bandage used on a standing, hurt teammate
//! (`Command::Treat` on someone awake, Rust's medical tool on another
//! player). A syringe lands at once; a bandage is wrapped on in the
//! healer's hands and spends nothing until it lands. The medkit stays
//! self-only, a full target refuses, and a target out of reach or behind a
//! wall is silent. A heal that lands tells the one healed who healed them
//! (`EV_HEALED`); one refused or put away tells them nothing. The downed revive path is `tests/assist.rs`'s and
//! `tests/wounded.rs`'s, and one case here holds that it did not move.
use sim_core::{
    build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE},
    deploy::DeployContent,
    gather::{GatherContent, ItemStack},
    input::InputFrame,
    limits::INV_SLOTS,
    movement::{Body, POS_XZ_Q, POS_Y_Q},
    survival::{ConsumableDef, SurvivalContent, REFUSE_C_FULL, REFUSE_C_NOT_FOOD},
    world::{Command, World, EV_CONSUMED, EV_CONSUME_REFUSED, EV_HEALED, EV_RECOVERED},
};

const HEALER: u32 = 1;
const HURT: u32 = 2;
const SYRINGE: u16 = 40;
const BANDAGE: u16 = 41;
const MEDKIT: u16 = 42;
/// Hotbar slots the healer carries each in.
const S_SYRINGE: u8 = 0;
const S_BANDAGE: u8 = 1;
const S_MEDKIT: u8 = 2;
/// The bandage's wrap, ticks (Rust's 4 s at 30 Hz).
const WRAP: u8 = 120;
/// Wire yaw due +Z (`yaw_lut.rs`), and due +X.
const YAW_PLUS_Z: u16 = 0;
const YAW_PLUS_X: u16 = 64 << 8;

/// The three medical rows, in the shape `content/consumables.toml` gives
/// them: a syringe that revives, a bandage wrapped on, a medkit that is
/// neither. Meters that never drain inside a test, and a bleed that ticks.
fn content() -> SurvivalContent {
    let mut sc = SurvivalContent::EMPTY;
    sc.max_food = 100;
    sc.max_water = 100;
    sc.food_span_ticks = 1 << 30;
    sc.water_span_ticks = 1 << 30;
    sc.bleed_hp_per_min = 20;
    sc.consumable[SYRINGE as usize] = ConsumableDef {
        health: 15,
        seconds: 2,
        ..ConsumableDef::NONE
    };
    sc.revive[SYRINGE as usize] = true;
    sc.consumable[BANDAGE as usize] = ConsumableDef {
        health: 20,
        seconds: 4,
        bleed: -50,
        use_ticks: WRAP,
        ..ConsumableDef::NONE
    };
    sc.consumable[MEDKIT as usize] = ConsumableDef {
        health: 60,
        seconds: 8,
        bleed: -100,
        ..ConsumableDef::NONE
    };
    sc
}

fn stack(item: u16) -> ItemStack {
    ItemStack {
        item,
        count: 2,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    }
}

/// Two players 1.5 m apart on the spawn pad, the healer carrying two of
/// each, the other at half health.
fn fixture() -> Box<World> {
    let mut w = Box::new(World::new(42));
    w.gather = GatherContent::probe_fixture();
    w.survival = content();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[
        Command::Join { id: HEALER },
        Command::Join { id: HURT },
        Command::Join { id: 3 },
    ]);
    let a = w.players[0].body;
    w.players[1].body = Body::at(
        42,
        &w.haven,
        a.qx as f32 * POS_XZ_Q,
        a.qz as f32 * POS_XZ_Q + 1.5,
    );
    for p in w.players.iter_mut() {
        p.inv = [ItemStack::default(); INV_SLOTS];
        p.hp_max = 100;
        p.hp = 100;
    }
    w.players[0].inv[S_SYRINGE as usize] = stack(SYRINGE);
    w.players[0].inv[S_BANDAGE as usize] = stack(BANDAGE);
    w.players[0].inv[S_MEDKIT as usize] = stack(MEDKIT);
    w.players[1].hp = 50;
    w
}

/// The healer's look at the other's chest along `yaw` (`tests/give.rs`'s
/// aim), holding hotbar slot `sel`.
fn aim(w: &World, yaw: u16, sel: u8) -> Command {
    let (a, b) = (w.players[0].body, w.players[1].body);
    let run = if yaw == YAW_PLUS_X {
        (b.qx - a.qx) as f32 * POS_XZ_Q
    } else {
        (b.qz - a.qz) as f32 * POS_XZ_Q
    };
    let rise = (b.qy - a.qy) as f32 * POS_Y_Q + 1.0 - 1.6;
    let pitch = (0..=255u8)
        .max_by(|&a, &b| {
            let (ac, as_) = sim_core::pitch_dir(a);
            let (bc, bs) = sim_core::pitch_dir(b);
            (ac * run + as_ * rise).total_cmp(&(bc * run + bs * rise))
        })
        .unwrap();
    Command::Input {
        id: HEALER,
        frame: InputFrame {
            seq: w.tick as u16,
            yaw,
            pitch,
            sel,
            ..Default::default()
        },
        favour: 0,
    }
}

/// Aim with `slot` in hand for a tick, then use it on `target`.
fn treat_along(w: &mut World, yaw: u16, slot: u8, target: u32) {
    let a = aim(w, yaw, slot);
    w.tick(&[a]);
    let a = aim(w, yaw, slot);
    w.tick(&[
        a,
        Command::Treat {
            id: HEALER,
            slot,
            target,
        },
    ]);
}

fn treat(w: &mut World, slot: u8) {
    treat_along(w, YAW_PLUS_Z, slot, HURT);
}

/// One tick, still aimed with `sel` up.
fn hold(w: &mut World, sel: u8) {
    let a = aim(w, YAW_PLUS_Z, sel);
    w.tick(&[a]);
}

fn count(w: &World, code: u8) -> usize {
    w.events.entries().iter().filter(|e| e.code == code).count()
}

/// This tick's `EV_HEALED`s as (healed, healer, item).
fn healed(w: &World) -> Vec<(u32, u32, u32)> {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == EV_HEALED)
        .map(|e| (e.a, e.b, e.c))
        .collect()
}

fn left(w: &World, slot: u8) -> u16 {
    w.players[0].inv[slot as usize].count
}

fn refused(w: &World) -> Option<u32> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == EV_CONSUME_REFUSED)
        .map(|e| {
            assert_eq!(e.a, HEALER, "a refusal is the healer's to hear");
            e.b
        })
}

#[test]
fn a_syringe_heals_a_standing_player_over_its_ramp_and_spends_one() {
    let mut w = fixture();
    w.players[0].hp = 40;
    treat(&mut w, S_SYRINGE);
    assert_eq!(left(&w, S_SYRINGE), 1, "one syringe is spent");
    let used: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_CONSUMED)
        .collect();
    assert_eq!(used.len(), 1);
    assert_eq!(
        (used[0].a, used[0].b),
        (HEALER, (SYRINGE as u32) << 16 | S_SYRINGE as u32),
        "EV_CONSUMED names who used it, the item and their slot"
    );
    assert_eq!(
        healed(&w),
        [(HURT, HEALER, SYRINGE as u32)],
        "the one healed hears who healed them, and with what"
    );
    assert!(w.players[1].heal_rem > 0, "the heal never started");
    for _ in 0..90 {
        hold(&mut w, S_SYRINGE);
        assert!(healed(&w).is_empty(), "the ramp said it again");
    }
    assert_eq!(w.players[1].hp, 65, "the syringe's 15 over its ramp");
    assert_eq!(w.players[0].hp, 40, "the healer healed themselves");
    assert!(!w.players[1].wounded && !w.players[1].dead);
}

#[test]
fn a_bandage_closes_a_teammates_bleed_only_when_the_wrap_lands() {
    let mut w = fixture();
    w.players[0].bleed = 30;
    w.players[1].bleed = 30;
    treat(&mut w, S_BANDAGE);
    assert_eq!(left(&w, S_BANDAGE), 2, "the press spent the bandage");
    assert_ne!(w.players[0].use_at, 0, "no wrap began");
    assert_eq!(w.players[0].use_target, HURT);
    assert_eq!(count(&w, EV_CONSUMED), 0);
    assert!(
        healed(&w).is_empty(),
        "the press said healed before the wrap"
    );
    for t in 1..WRAP {
        hold(&mut w, S_BANDAGE);
        assert_eq!(left(&w, S_BANDAGE), 2, "spent {t} ticks into the wrap");
        assert!(w.players[1].bleed > 0, "the bleed closed {t} ticks in");
        assert_eq!(w.players[1].heal_rem, 0, "the heal began {t} ticks in");
        assert!(healed(&w).is_empty(), "healed said {t} ticks into the wrap");
    }
    let mut heard = Vec::new();
    for _ in 0..2 {
        hold(&mut w, S_BANDAGE);
        heard.extend(healed(&w));
    }
    assert_eq!(
        heard,
        [(HURT, HEALER, BANDAGE as u32)],
        "the wrap's landing tells the one healed, once"
    );
    assert_eq!(left(&w, S_BANDAGE), 1, "the wrap landed and spent nothing");
    assert_eq!(w.players[1].bleed, 0, "the bandage did not close the bleed");
    assert!(
        w.players[1].heal_rem > 0,
        "the bandage's heal never started"
    );
    assert!(w.players[0].bleed > 0, "the wrap landed on the healer");
    assert_eq!(w.players[0].use_at, 0);
    assert_eq!(w.players[0].use_target, 0);
}

#[test]
fn a_full_unhurt_target_is_refused_and_keeps_the_item() {
    for slot in [S_SYRINGE, S_BANDAGE] {
        let mut w = fixture();
        w.players[1].hp = 100;
        treat(&mut w, slot);
        assert_eq!(refused(&w), Some(REFUSE_C_FULL), "slot {slot}");
        assert_eq!(left(&w, slot), 2, "slot {slot}: a refused heal was spent");
        assert_eq!(w.players[0].use_at, 0, "slot {slot}: a wrap began");
        assert_eq!(w.players[1].heal_rem, 0);
        assert!(
            healed(&w).is_empty(),
            "slot {slot}: a refused heal said healed"
        );
    }
    // Full hp with a bleed is hurt: the bandage has a wound to close.
    let mut w = fixture();
    w.players[1].hp = 100;
    w.players[1].bleed = 30;
    treat(&mut w, S_BANDAGE);
    assert_eq!(refused(&w), None);
    assert_eq!(w.players[0].use_target, HURT);
}

#[test]
fn a_medkit_is_refused_on_someone_else() {
    let mut w = fixture();
    treat(&mut w, S_MEDKIT);
    assert_eq!(refused(&w), Some(REFUSE_C_NOT_FOOD));
    assert_eq!(left(&w, S_MEDKIT), 2, "a refused medkit was spent");
    assert_eq!(w.players[1].heal_rem, 0);
    assert_eq!(w.players[1].hp, 50);
    assert!(healed(&w).is_empty());
}

/// Out of reach, at a sleeper or a corpse, or at yourself: nothing is
/// spent and no wrap begins. Out of reach is silent, as a syringe out of
/// reach of a downed body is; a body that is neither standing nor down is
/// the revive's path, which says nothing to a syringe and refuses a
/// bandage (it does not revive) exactly as it did before this verb.
#[test]
fn a_heal_out_of_reach_or_on_the_wrong_body_is_silent() {
    type Setup = fn(&mut World) -> u32;
    let cases: [(&str, Setup); 4] = [
        ("out of reach", |w| {
            w.players[1].body.qz += 200; // six metres further
            HURT
        }),
        ("a sleeper", |w| {
            w.players[1].sleeping = true;
            HURT
        }),
        ("a corpse", |w| {
            w.players[1].dead = true;
            HURT
        }),
        ("yourself", |w| {
            w.players[0].hp = 50;
            HEALER
        }),
    ];
    for (what, setup) in cases {
        for slot in [S_SYRINGE, S_BANDAGE] {
            let mut w = fixture();
            let target = setup(&mut w);
            treat_along(&mut w, YAW_PLUS_Z, slot, target);
            assert_eq!(left(&w, slot), 2, "{what}, slot {slot}: spent");
            assert_eq!(w.players[0].use_at, 0, "{what}, slot {slot}: a wrap began");
            assert_eq!(count(&w, EV_CONSUMED), 0, "{what}, slot {slot}");
            assert!(healed(&w).is_empty(), "{what}, slot {slot}: said healed");
            if slot == S_SYRINGE || what == "out of reach" {
                assert_eq!(
                    count(&w, EV_CONSUME_REFUSED),
                    0,
                    "{what}, slot {slot}: a refusal was said"
                );
            }
        }
    }
}

#[test]
fn switching_hands_mid_wrap_spends_nothing() {
    let mut w = fixture();
    w.players[1].bleed = 30;
    treat(&mut w, S_BANDAGE);
    assert_eq!(w.players[0].use_target, HURT);
    hold(&mut w, S_BANDAGE);
    hold(&mut w, 5);
    assert_eq!(w.players[0].use_at, 0, "the wrap outlived the hand");
    for _ in 0..=WRAP {
        hold(&mut w, 5);
        assert!(healed(&w).is_empty(), "a put-away wrap said healed");
    }
    assert_eq!(left(&w, S_BANDAGE), 2, "a put-away wrap was spent");
    assert!(w.players[1].bleed > 0, "a put-away wrap closed the bleed");
    assert_eq!(count(&w, EV_CONSUMED), 0);
}

/// The target is checked again on the tick the wrap lands: one who walked
/// out of reach meanwhile costs the healer the wrap and not the bandage.
#[test]
fn a_wrap_landing_on_a_target_gone_out_of_reach_spends_nothing() {
    let mut w = fixture();
    w.players[1].bleed = 30;
    treat(&mut w, S_BANDAGE);
    assert_eq!(w.players[0].use_target, HURT);
    for _ in 0..WRAP / 2 {
        hold(&mut w, S_BANDAGE);
    }
    w.players[1].body.qz += 200;
    for _ in 0..WRAP {
        hold(&mut w, S_BANDAGE);
        assert!(healed(&w).is_empty(), "a wrap on nobody said healed");
    }
    assert_eq!(w.players[0].use_at, 0, "the wrap never ended");
    assert_eq!(left(&w, S_BANDAGE), 2, "a wrap on nobody was spent");
    assert!(w.players[1].bleed > 0);
}

/// A downed body is still the revive's: the syringe stands it up, and a
/// bandage — which does not revive — is refused without a wrap.
#[test]
fn the_downed_revive_path_is_unchanged() {
    let mut w = fixture();
    w.players[1].hp = 10;
    w.players[1].wounded = true;
    w.players[1].wound_until = w.tick + 900;
    treat(&mut w, S_BANDAGE);
    assert_eq!(refused(&w), Some(REFUSE_C_NOT_FOOD));
    assert_eq!(left(&w, S_BANDAGE), 2);
    assert_eq!(w.players[0].use_at, 0, "a bandage began a wrap on the down");
    assert!(w.players[1].wounded);
    treat(&mut w, S_SYRINGE);
    assert!(!w.players[1].wounded, "the syringe did not stand them up");
    assert_eq!(left(&w, S_SYRINGE), 1);
    assert_eq!(count(&w, EV_RECOVERED), 1);
    assert!(
        healed(&w).is_empty(),
        "a body helped up hears it got up, as it did before"
    );
}

// ---------------------------------------------------------------------
// Behind a wall.
// ---------------------------------------------------------------------

const WALL_SEED: u64 = 0x0FEE_0FEE;

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

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `tests/oven.rs`'s scan: the nearest cell a foundation may stand on.
fn buildable_cell(seed: u64) -> (u16, u16) {
    let (hx, hz) = sim_core::find::build_hint(seed, 512, 512);
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (hx + dx).clamp(0, 2047) as u16;
                let cz = (hz + dz).clamp(0, 2047) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// The healer on a foundation, the other on the ground 1.8 m east of it,
/// with a wall on the foundation's east edge between them or not.
fn across_an_edge(wall: bool) -> Box<World> {
    let mut w = Box::new(World::new(WALL_SEED));
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.survival = content();
    let (fx, cz) = buildable_cell(WALL_SEED);
    let cx = fx + 1;
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: HEALER }, Command::Join { id: HURT }]);
    w.players[0].body = Body::at(WALL_SEED, hv(WALL_SEED), x, z);
    // `BuildContent::probe_fixture`: row 0 the foundation, row 1 a wall,
    // both paid in item 0.
    w.players[0].inv[5] = ItemStack {
        item: 0,
        count: 40,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    let pieces: &[(u16, u16, u8)] = if wall {
        &[(0, fx, LOC_PLANE), (1, cx, LOC_EDGE_XLO)]
    } else {
        &[(0, fx, LOC_PLANE)]
    };
    for &(row, at, loc) in pieces {
        let before = w.pieces.len();
        w.tick(&[Command::Place {
            id: HEALER,
            row,
            cx: at,
            cz,
            level: 0,
            loc,
            freehand: false,
            plate: 0,
        }]);
        assert_eq!(w.pieces.len(), before + 1, "setup: piece row {row} placed");
    }
    let x0 = cx as f32 * BUILD_CELL_M;
    w.players[0].body = Body::at(WALL_SEED, hv(WALL_SEED), x0 - 0.9, z);
    w.players[1].body = Body::at(WALL_SEED, hv(WALL_SEED), x0 + 0.9, z);
    for _ in 0..8 {
        w.tick(&[]);
    }
    for p in w.players.iter_mut().take(2) {
        p.hp_max = 100;
        p.hp = 100;
    }
    w.players[0].inv[S_SYRINGE as usize] = stack(SYRINGE);
    w.players[0].inv[S_BANDAGE as usize] = stack(BANDAGE);
    w.players[1].hp = 50;
    w
}

#[test]
fn a_heal_through_a_wall_is_silent_and_spends_nothing() {
    // The control: the same two bodies with no wall reach each other.
    let mut w = across_an_edge(false);
    treat_along(&mut w, YAW_PLUS_X, S_SYRINGE, HURT);
    assert_eq!(
        left(&w, S_SYRINGE),
        1,
        "fixture: the two must reach each other before the wall goes up"
    );

    for slot in [S_SYRINGE, S_BANDAGE] {
        let mut w = across_an_edge(true);
        treat_along(&mut w, YAW_PLUS_X, slot, HURT);
        assert_eq!(left(&w, slot), 2, "slot {slot}: healed through a wall");
        assert_eq!(w.players[0].use_at, 0, "slot {slot}: a wrap began");
        assert_eq!(w.players[1].heal_rem, 0);
        assert_eq!(count(&w, EV_CONSUMED), 0);
        assert_eq!(count(&w, EV_CONSUME_REFUSED), 0);
        assert!(healed(&w).is_empty(), "slot {slot}");
    }
}
