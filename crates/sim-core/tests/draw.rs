//! The bow is drawn (`reference/PROJECTILES.md` §6): hold the aim
//! (`BTN_AIM`, the right mouse) for the weapon's `draw_ticks` and only then
//! does a left click loose. Letting go throws the draw away; a draw held
//! through a shot is ready when the cadence is; a body drawing a bow walks;
//! a downed body cannot draw. A weapon with no draw (the crossbow, every
//! fixture before this file) fires from the hip exactly as it did.

use sim_core::combat::{AmmoDef, CombatContent, RangedDef, NO_MAG};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_AIM, BTN_CROUCH, BTN_PRIMARY, BTN_SPRINT};
use sim_core::ranged::{self, Arrows};
use sim_core::world::{EventQueue, Player};

const BOW: u16 = 3;
const ARROW: u16 = 4;
const DRAW: u16 = 30;
const RATE: u16 = 60;

fn bow(draw_ticks: u16) -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: RATE,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        head_pct: 200,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
        draw_ticks,
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 22,
        damage_pct: 100,
    };
    c
}

fn archer() -> Player {
    let mut p = Player {
        id: 1,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
        skin: 0,
    };
    p.inv[7] = ItemStack {
        item: ARROW,
        count: 20,
        cond: 0,
        skin: 0,
    };
    p
}

/// Run ticks `from..to` with these buttons held and return the ticks a
/// shot left the bow on.
fn hold(
    cc: &CombatContent,
    p: &mut Player,
    arrows: &mut Arrows,
    buttons: u8,
    from: u64,
    to: u64,
) -> Vec<u64> {
    let mut shots = Vec::new();
    for tick in from..to {
        p.frame = InputFrame {
            buttons,
            ..InputFrame::default()
        };
        let before = arrows.len();
        assert!(ranged::draw(
            tick,
            cc,
            arrows,
            &mut EventQueue::default(),
            p
        ));
        if arrows.len() > before {
            shots.push(tick);
            *arrows = Arrows::new();
        }
    }
    shots
}

#[test]
fn a_left_click_alone_never_looses_a_drawn_weapon() {
    let cc = bow(DRAW);
    let (mut p, mut arrows) = (archer(), Arrows::new());
    assert!(hold(&cc, &mut p, &mut arrows, BTN_PRIMARY, 100, 400).is_empty());
}

/// Mutant: firing on the first aimed tick, or a tick late, fails here.
#[test]
fn the_aim_held_for_the_draw_then_looses() {
    let cc = bow(DRAW);
    let (mut p, mut arrows) = (archer(), Arrows::new());
    hold(&cc, &mut p, &mut arrows, 0, 100, 110);
    // Aim at 110: full at 110 + DRAW, and the click held from the start
    // looses on exactly that tick.
    let shots = hold(
        &cc,
        &mut p,
        &mut arrows,
        BTN_AIM | BTN_PRIMARY,
        110,
        110 + u64::from(DRAW) + 1,
    );
    assert_eq!(shots, vec![110 + u64::from(DRAW)]);
}

#[test]
fn letting_go_throws_the_draw_away() {
    let cc = bow(DRAW);
    let (mut p, mut arrows) = (archer(), Arrows::new());
    hold(&cc, &mut p, &mut arrows, BTN_AIM, 100, 125);
    hold(&cc, &mut p, &mut arrows, 0, 125, 126);
    let shots = hold(&cc, &mut p, &mut arrows, BTN_AIM | BTN_PRIMARY, 126, 200);
    assert_eq!(
        shots,
        vec![126 + u64::from(DRAW)],
        "the second draw started at the second aim, not the first"
    );
}

/// Held through a shot, the next is ready when the cadence is — the sim's
/// nock, which the client's draw clock mirrors.
#[test]
fn a_draw_held_through_a_shot_is_ready_when_the_cadence_is() {
    let cc = bow(DRAW);
    let (mut p, mut arrows) = (archer(), Arrows::new());
    hold(&cc, &mut p, &mut arrows, 0, 0, 5);
    let shots = hold(&cc, &mut p, &mut arrows, BTN_AIM | BTN_PRIMARY, 5, 300);
    let first = 5 + u64::from(DRAW);
    let want: Vec<u64> = (first..300).step_by(RATE as usize).collect();
    assert_eq!(shots, want);
}

/// Relaxed after a shot and aimed again before the cadence ends, the shot
/// waits for the cadence; aimed after it, for a full draw.
#[test]
fn a_new_aim_after_a_shot_waits_for_the_later_of_the_nock_and_the_draw() {
    let cc = bow(DRAW);
    let (mut p, mut arrows) = (archer(), Arrows::new());
    hold(&cc, &mut p, &mut arrows, 0, 0, 5);
    let shot = hold(&cc, &mut p, &mut arrows, BTN_AIM | BTN_PRIMARY, 5, 36)[0];
    hold(&cc, &mut p, &mut arrows, 0, shot + 1, shot + 10);
    let early = hold(
        &cc,
        &mut p,
        &mut arrows,
        BTN_AIM | BTN_PRIMARY,
        shot + 10,
        shot + 200,
    );
    assert_eq!(
        early[0],
        shot + u64::from(RATE),
        "the nock outlasts the draw here"
    );

    let (mut p, mut arrows) = (archer(), Arrows::new());
    hold(&cc, &mut p, &mut arrows, 0, 0, 5);
    let shot = hold(&cc, &mut p, &mut arrows, BTN_AIM | BTN_PRIMARY, 5, 36)[0];
    let aim = shot + u64::from(RATE) + 40;
    hold(&cc, &mut p, &mut arrows, 0, shot + 1, aim);
    let late = hold(
        &cc,
        &mut p,
        &mut arrows,
        BTN_AIM | BTN_PRIMARY,
        aim,
        aim + 200,
    );
    assert_eq!(
        late[0],
        aim + u64::from(DRAW),
        "a full draw from the new aim"
    );
}

/// Every fixture before this file, and the crossbow: no draw, fires from
/// the hip, aim or not.
#[test]
fn a_weapon_with_no_draw_fires_from_the_hip() {
    let cc = bow(0);
    let (mut p, mut arrows) = (archer(), Arrows::new());
    let shots = hold(&cc, &mut p, &mut arrows, BTN_PRIMARY, 100, 101);
    assert_eq!(shots, vec![100]);
}

/// A body drawing a bow walks: the aim takes the sprint.
#[test]
fn a_drawn_bow_walks() {
    use sim_core::movement::{self, Body};
    use sim_core::occupy::{Pristine, Scratch};
    let seed = 7;
    let mut sc = Scratch::with(seed, Pristine);
    let cols = sim_core::collide::ColIndex::new();
    let start = Body::at(seed, &sc.haven, 2048.0, 2048.0);
    let mut run = |buttons: u8| {
        let mut b = start;
        let f = InputFrame {
            buttons,
            move_z: 127,
            ..InputFrame::default()
        };
        for _ in 0..30 {
            let mut occ = sim_core::occupy::Occupants {
                doors: 0,
                table: &sc.table,
                haven: &sc.haven,
                harvested: &sc.harvested,
                cache: &mut sc.cache,
            };
            movement::step(seed, &sc.haven, &cols, &mut occ, &mut b, &f);
        }
        (b.qz - start.qz).abs() + (b.qx - start.qx).abs()
    };
    let sprint = run(BTN_SPRINT);
    let walk = run(0);
    let drawn = run(BTN_SPRINT | BTN_AIM);
    assert!(
        sprint > walk,
        "the fixture must sprint faster than it walks"
    );
    assert_eq!(drawn, walk, "drawing a bow and sprinting is walking");
}

#[test]
fn a_downed_body_cannot_draw() {
    let f = InputFrame {
        buttons: BTN_AIM | BTN_PRIMARY,
        ..InputFrame::default()
    };
    assert_eq!(sim_core::wound::crawl_frame(&f).buttons & BTN_AIM, 0);
}

/// Brought up already drawn — the aim held while the hand changes to the
/// bow — it still takes a full draw: `World` starts the draw on the frame's
/// edge. Mutant: without the edge it looses on its first tick in the hand.
#[test]
fn a_bow_brought_up_drawn_still_takes_a_draw() {
    use sim_core::world::{Command, World, EV_SHOT};
    let mut w = Box::new(World::new(20260731));
    w.combat = bow(DRAW);
    let mut g = sim_core::gather::GatherContent::EMPTY;
    g.stack_max[BOW as usize] = 1;
    g.stack_max[ARROW as usize] = 64;
    g.item_count = ARROW + 1;
    w.gather = g;
    w.tick(&[Command::Join { id: 1 }]);
    let a = archer();
    w.players[0].inv[1] = a.inv[0];
    w.players[0].inv[7] = a.inv[7];
    let frame = |sel| InputFrame {
        buttons: BTN_AIM | BTN_PRIMARY,
        sel,
        ..InputFrame::default()
    };
    let input = |sel| Command::Input {
        id: 1,
        frame: frame(sel),
        favour: 0,
    };
    // Both buttons down with an empty hand for a while, then the bow.
    for _ in 0..40 {
        w.tick(&[input(0)]);
    }
    let up = w.tick;
    let mut shot = None;
    for _ in 0..(2 * DRAW) {
        w.tick(&[input(1)]);
        if shot.is_none() && w.events.entries().iter().any(|e| e.code == EV_SHOT) {
            shot = Some(w.tick);
        }
    }
    let shot = shot.expect("a drawn bow looses");
    assert!(
        shot >= up + u64::from(DRAW),
        "brought up drawn, it loosed {} ticks after coming up, under the {DRAW}-tick draw",
        shot - up
    );
}

/// A crouch walks slower still, and beats sprint and the draw (v83): the
/// distance is `CROUCH_SPEED`'s over flat ground, whatever else is held.
#[test]
fn a_crouch_is_the_slowest_walk_whatever_else_is_held() {
    use sim_core::movement::{self, Body, CROUCH_SPEED, DT, POS_XZ_Q, WALK_SPEED};
    use sim_core::occupy::{Pristine, Scratch};
    let seed = 7;
    let mut sc = Scratch::with(seed, Pristine);
    let cols = sim_core::collide::ColIndex::new();
    let start = Body::at(seed, &sc.haven, 2048.0, 2048.0);
    let mut run = |buttons: u8| {
        let mut b = start;
        let f = InputFrame {
            buttons,
            move_z: 127,
            ..InputFrame::default()
        };
        for _ in 0..30 {
            let mut occ = sim_core::occupy::Occupants {
                doors: 0,
                table: &sc.table,
                haven: &sc.haven,
                harvested: &sc.harvested,
                cache: &mut sc.cache,
            };
            movement::step(seed, &sc.haven, &cols, &mut occ, &mut b, &f);
        }
        (b.qz - start.qz).abs() + (b.qx - start.qx).abs()
    };
    let walk = run(0);
    let crouch = run(BTN_CROUCH);
    assert_eq!(run(BTN_CROUCH | BTN_SPRINT), crouch, "crouch beats sprint");
    assert_eq!(run(BTN_CROUCH | BTN_AIM), crouch, "and the draw");
    // The body is stored in 3 cm quanta, so each tick's step rounds to a
    // whole quantum: the claim is the per-tick speed to half a quantum.
    let per_tick = CROUCH_SPEED * DT / POS_XZ_Q;
    assert!(
        crouch < walk,
        "crouch {crouch} must be slower than walk {walk}"
    );
    assert!(
        (crouch as f32 - 30.0 * per_tick).max(30.0 * per_tick - crouch as f32) <= 15.0,
        "crouch {crouch} quanta over 30 ticks vs {per_tick} a tick (walk {walk}, {WALK_SPEED} m/s)"
    );
}

/// A downed body's crawl ignores crouch: `crawl_frame` strips the bit, so
/// the crawl is not slowed twice.
#[test]
fn a_crawl_strips_the_crouch() {
    let f = InputFrame {
        buttons: BTN_CROUCH,
        move_z: 120,
        ..InputFrame::default()
    };
    assert_eq!(sim_core::wound::crawl_frame(&f).buttons & BTN_CROUCH, 0);
}

/// A crouched archer's arrow leaves from the crouched eye (v83), not the
/// standing one: a crouched player cannot shoot over cover they cannot see
/// over.
#[test]
fn a_crouched_archer_looses_from_the_crouched_eye() {
    use sim_core::movement::POS_Y_Q;
    let cc = bow(0);
    let spawn_y = |crouch: bool| {
        let mut p = archer();
        p.body.qy = 500;
        p.body.grounded = true;
        p.frame = InputFrame {
            buttons: BTN_PRIMARY | if crouch { BTN_CROUCH } else { 0 },
            ..InputFrame::default()
        };
        let mut arrows = Arrows::new();
        assert!(ranged::draw(
            100,
            &cc,
            &mut arrows,
            &mut EventQueue::default(),
            &mut p
        ));
        let a = arrows.entries().next().expect("the bow looses");
        a.qy - (500.0 * POS_Y_Q * 1000.0) as i32
    };
    assert_eq!(spawn_y(false), ranged::ARROW_EYE_MM);
    assert_eq!(spawn_y(true), ranged::CROUCH_EYE_MM);
}
