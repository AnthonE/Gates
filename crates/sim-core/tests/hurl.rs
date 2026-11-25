//! A spear is thrown (`ranged::hurl`, `combat::HurlDef`): the right mouse
//! held for the windup, a click, and the spear itself flies as an arrow
//! does, hits for its throw, and comes down as a loose stack of itself to
//! be picked up again.
//!
//! The first half drives `ranged::hurl` and `ranged::step` directly over a
//! hand-built roster, as `tests/arrow_recovery.rs` does an arrow: the hit,
//! the ladder, the hand, the windup. The second half goes through
//! `World::tick`, the only path a player has: where it lands, the pickup,
//! the safe zone, the replay.

#![allow(clippy::disallowed_macros)]

use sim_core::backpack::BackpackContent;
use sim_core::combat::{CombatContent, HurlDef, MeleeDef};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_AIM, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Occupants, Pristine, Scratch};
use sim_core::ranged::{self, Arrows, Hurled, Kill};
use sim_core::spent::SpentArrows;
use sim_core::terrain;
use sim_core::world::{Command, EventQueue, Player, World, DEATH_BY_ARROW, EV_SHOT};

const SEED: u64 = 20_260_731;
/// The spear, at an index of its own.
const SPEAR: u16 = 3;
const WINDUP: u8 = 15;
const THROW_DAMAGE: u16 = 30;
const WEAR: u16 = 100;
const COND_MAX: u16 = 10_000;
/// The thrown spear's condition and skin, so "it comes back as itself" is
/// a checkable claim.
const COND: u16 = 5_000;
const SKIN: u16 = 7;
/// Level, as `tests/gun.rs` spells it: 0.35° up, which is nothing at 10 m.
const LEVEL: u8 = 128;

/// A spear that swings for 20 and throws for 30 at 30 m/s, wearing a point
/// a throw, with the shipped ladder.
fn spear() -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.melee[SPEAR as usize] = MeleeDef {
        damage: 20,
        structure: 3,
        reach_cm: 175,
        head_pct: 200,
        limb_pct: 50,
        lit_bonus: 0,
        wear: WEAR,
    };
    c.hurl[SPEAR as usize] = HurlDef {
        speed_mmpt: 1000,
        drop_mmpt2: 11,
        damage: THROW_DAMAGE,
        windup_ticks: WINDUP,
    };
    c.cond_max[SPEAR as usize] = COND_MAX;
    c
}

fn held(count: u16) -> ItemStack {
    ItemStack {
        item: SPEAR,
        count,
        cond: COND,
        skin: SKIN,
        ..ItemStack::EMPTY
    }
}

fn body_at(x: f32, feet_y: f32, z: f32) -> Body {
    Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (feet_y / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        ..Body::default()
    }
}

fn player(id: u32, x: f32, feet_y: f32, z: f32) -> Player {
    Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        body: body_at(x, feet_y, z),
        ..Player::default()
    }
}

/// High over the island's middle, clear of anything that grows.
const UP: f32 = 300.0;
const MID: f32 = 2048.0;

/// The thrower at `MID`, looking down +z, and a body `dz` metres along the
/// look with its feet `dy` metres off the thrower's.
fn face_off(dz: f32, dy: f32) -> Box<[Player; MAX_PLAYERS]> {
    let mut ps = Box::new([Player::default(); MAX_PLAYERS]);
    ps[0] = player(1, MID, UP, MID);
    ps[0].inv[0] = held(1);
    ps[1] = player(2, MID, UP + dy, MID + dz);
    ps
}

/// The tick the aim goes down. Not 0: a body that has never held a weapon
/// carries `draw_on == 0`, which tick 0 would read as an aim already held.
const T0: u64 = 1;
/// The first tick a click throws.
const READY: u64 = T0 + WINDUP as u64;

/// Hold the aim from `T0` and click on tick `click`, letting go of the aim
/// on tick `release` (never, if past the click). Returns the store and what
/// `hurl` answered on the click.
fn throw(
    cc: &CombatContent,
    ps: &mut [Player; MAX_PLAYERS],
    click: u64,
    release: u64,
    events: &mut EventQueue,
) -> (Arrows, bool) {
    let mut arrows = Arrows::new();
    let mut took = false;
    for t in T0..=click {
        let mut buttons = if t < release { BTN_AIM } else { 0 };
        if t == click {
            buttons |= BTN_PRIMARY;
        }
        ps[0].frame = InputFrame {
            buttons,
            yaw: 0,
            pitch: LEVEL,
            ..InputFrame::default()
        };
        took = ranged::hurl(t, cc, &mut arrows, events, &mut ps[0], true);
    }
    (arrows, took)
}

/// Fly the store until it is empty. Returns the kills and what came down.
fn fly(
    cc: &CombatContent,
    ps: &mut [Player; MAX_PLAYERS],
    arrows: &mut Arrows,
    from: u64,
) -> (Vec<Kill>, Vec<Hurled>) {
    let mut sc = Scratch::with(SEED, Pristine);
    let cols = sim_core::collide::ColIndex::new();
    let mut spent = SpentArrows::new();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let mut all = Vec::new();
    let mut t = from;
    while !arrows.is_empty() && t < from + 300 {
        t += 1;
        let mut occ = Occupants {
            doors: 0,
            table: &sc.table,
            haven: &sc.haven,
            harvested: &sc.harvested,
            cache: &mut sc.cache,
        };
        let (n, _) = ranged::step(
            SEED,
            t,
            &sc.haven,
            &cols,
            &mut occ,
            cc,
            arrows,
            &mut spent,
            ps,
            &mut EventQueue::default(),
            &mut kills,
            &mut chips,
        );
        all.extend_from_slice(&kills[..n]);
    }
    assert!(arrows.is_empty(), "the spear never came down");
    assert!(spent.is_empty(), "a thrown spear is never a spent arrow");
    let landed = arrows.hurls().to_vec();
    (all, landed)
}

#[test]
fn a_thrown_spear_hits_for_its_throw_and_comes_down_as_itself() {
    let cc = spear();
    // A chest: the eye's line, less the sag over ten metres, is 0.9 m up.
    let mut ps = face_off(10.0, 0.0);
    ps[0].inv[0] = held(3);
    let mut ev = EventQueue::default();
    let (mut arrows, took) = throw(&cc, &mut ps, READY, u64::MAX, &mut ev);
    assert!(took, "an aimed spear takes the arm");
    assert_eq!(arrows.len(), 1, "one spear in the air");
    assert_eq!(ps[0].inv[0].count, 2, "exactly one left the hand");
    assert_eq!(
        ev.entries().iter().filter(|e| e.code == EV_SHOT).count(),
        1,
        "the throw is announced once, as a shot"
    );
    let a = *arrows.entries().next().unwrap();
    assert_eq!((a.item, a.round), (SPEAR, SPEAR), "the spear is the round");
    assert_eq!(
        a.damage, THROW_DAMAGE,
        "the throw's damage, not the swing's"
    );

    let (kills, landed) = fly(&cc, &mut ps, &mut arrows, READY);
    assert!(kills.is_empty());
    assert_eq!(
        ps[1].hp,
        100 - THROW_DAMAGE,
        "a chest pays the throw's damage"
    );
    assert_eq!(landed.len(), 1, "it came down");
    let h = landed[0];
    assert_eq!(h.dir, [0; 3], "out of a body it falls, it does not stick");
    assert_eq!(
        h.stack,
        ItemStack {
            count: 1,
            cond: COND - WEAR,
            ..held(1)
        },
        "the spear itself: its skin, its condition less one throw's wear"
    );
    let (vx, _, vz) = sim_core::spent::feet_mm(&ps[1].body);
    assert!(
        (h.qx - vx).abs() < 100 && (h.qz - vz).abs() < 100,
        "where it met the body: ({}, {}) against ({vx}, {vz})",
        h.qx,
        h.qz
    );
}

#[test]
fn a_head_pays_the_ladder_and_the_kill_names_the_spear() {
    let cc = spear();
    // The body dropped so the line crosses its head band.
    let mut ps = face_off(10.0, -0.65);
    ps[1].hp = 40;
    let (mut arrows, _) = throw(&cc, &mut ps, READY, u64::MAX, &mut EventQueue::default());
    let (kills, _) = fly(&cc, &mut ps, &mut arrows, READY);
    assert_eq!(kills.len(), 1, "60 to the head kills the 40 left");
    let k = kills[0];
    assert!(k.head, "the line crossed the head band");
    assert_eq!(k.item, SPEAR, "the death names the spear");
    assert_eq!((k.victim, k.by), (1, 1));
}

#[test]
fn the_windup_must_be_held_and_a_plain_click_still_swings() {
    let cc = spear();
    // Clicked before the windup is done: nothing leaves the hand.
    let mut ps = face_off(10.0, 0.0);
    let (arrows, took) = throw(
        &cc,
        &mut ps,
        READY - 3,
        u64::MAX,
        &mut EventQueue::default(),
    );
    assert!(took, "an aimed spear still takes the arm");
    assert!(arrows.is_empty(), "thrown before the windup");
    assert_eq!(ps[0].inv[0].count, 1);

    // The aim let go before the click: the click is a swing, not a throw.
    let mut ps = face_off(10.0, 0.0);
    let (arrows, took) = throw(
        &cc,
        &mut ps,
        READY + 5,
        READY - 2,
        &mut EventQueue::default(),
    );
    assert!(!took, "a plain click hands the arm to the swing");
    assert!(arrows.is_empty());
    assert_eq!(ps[0].inv[0].count, 1, "the spear stays in hand");

    // A weapon with no throw is never aimed off the arm.
    let mut c = spear();
    c.hurl[SPEAR as usize] = HurlDef::default();
    let mut ps = face_off(10.0, 0.0);
    let (_, took) = throw(&c, &mut ps, READY, u64::MAX, &mut EventQueue::default());
    assert!(!took, "a spear with no throw swings");
}

#[test]
fn no_room_on_the_ground_keeps_the_spear_in_hand() {
    let cc = spear();
    let mut ps = face_off(10.0, 0.0);
    let mut arrows = Arrows::new();
    for t in T0..=READY {
        ps[0].frame = InputFrame {
            buttons: BTN_AIM | if t == READY { BTN_PRIMARY } else { 0 },
            pitch: LEVEL,
            ..InputFrame::default()
        };
        ranged::hurl(
            t,
            &cc,
            &mut arrows,
            &mut EventQueue::default(),
            &mut ps[0],
            false,
        );
    }
    assert!(arrows.is_empty(), "a throw never evicts a loose stack");
    assert_eq!(ps[0].inv[0], held(1), "and costs nothing");
}

// ---------------------------------------------------------------------
// Through the world
// ---------------------------------------------------------------------

/// A world with the spear, loose stacks armed, and players 1 and 2 joined.
fn world() -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.combat = spear();
    let mut g = GatherContent::EMPTY;
    g.stack_max[SPEAR as usize] = 1;
    g.item_count = SPEAR + 1;
    w.gather = g;
    w.backpack = BackpackContent::probe_fixture();
    // Long enough a life that nothing here races the despawn.
    w.backpack.base_ticks = 10_000;
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    w
}

/// Level, open, unbuilt ground clear of the town, with `len` metres of it
/// north along +z and nothing growing within a cell of the lane.
fn lane(w: &World, len: f32) -> (f32, f32) {
    let c = terrain::ISLAND_SIZE * 0.5;
    let clear = sim_core::town::TOWN_R_MAX + 100.0;
    sim_core::find::ground(w.seed, (c + clear + 50.0, c), 4.0, |x, z| {
        let (dx, dz) = (x - c, z - c);
        if dx * dx + dz * dz < clear * clear {
            return false;
        }
        let g0 = terrain::ground(w.seed, &w.haven, x, z);
        let n = (len / 0.5) as i32;
        let level = (0..=n).all(|i| {
            let zi = z + i as f32 * 0.5;
            let d = terrain::ground(w.seed, &w.haven, x, zi) - g0;
            d.max(-d) < 0.15
                && g0 > terrain::SEA_LEVEL + 1.0
                && sim_core::find::unbuilt(&w.haven, x, zi, 5.0)
        });
        if !level {
            return false;
        }
        let cell = |v: f32| (v / terrain::CELL_SIZE) as i32;
        (cell(x) - 1..=cell(x) + 1).all(|cx| {
            (cell(z) - 1..=cell(z + len) + 1).all(|cz| {
                terrain::scatter(w.seed, &w.scatter, &w.haven, cx, cz).occupant
                    == terrain::Occupant::None
            })
        })
    })
}

fn input(id: u32, seq: u16, buttons: u8, pitch: u8) -> Command {
    Command::Input {
        id,
        frame: InputFrame {
            seq,
            buttons,
            yaw: 0,
            pitch,
            sel: 0,
            ..InputFrame::default()
        },
        favour: 0,
    }
}

/// Aim, wind up, click, let go, and tick until nothing is in the air.
/// Returns the commands it sent each tick, for the replay.
fn wind_and_throw(w: &mut World, pitch: u8) -> Vec<Vec<Command>> {
    let mut script = vec![vec![input(1, 1, BTN_AIM, pitch)]];
    for _ in 0..WINDUP {
        script.push(vec![]);
    }
    script.push(vec![input(1, 2, BTN_AIM | BTN_PRIMARY, pitch)]);
    script.push(vec![input(1, 3, 0, pitch)]);
    for _ in 0..60 {
        script.push(vec![]);
    }
    for cmds in &script {
        w.tick(cmds);
    }
    assert!(w.arrows.is_empty(), "the spear never came down");
    script
}

fn carried(w: &World) -> Vec<ItemStack> {
    w.players[0]
        .inv
        .iter()
        .filter(|s| s.count > 0 && s.item == SPEAR)
        .copied()
        .collect()
}

/// Mutant: a thrown spear handed to `spent` comes back as a fresh spear at
/// full condition and no skin; dropping `land_hurls` loses it.
#[test]
fn a_spear_thrown_at_the_ground_lies_there_as_itself_and_is_picked_up() {
    let mut w = world();
    let (x, z) = lane(&w, 2.0);
    w.players[0].body = Body::at(w.seed, &w.haven, x, z);
    w.players[1].body = Body::at(w.seed, &w.haven, x + 30.0, z);
    w.players[0].inv[0] = held(1);
    w.tick(&[]);
    // Straight down at the feet.
    wind_and_throw(&mut w, 0);
    assert!(carried(&w).is_empty(), "it left the hand");
    assert_eq!(w.ground_items.len(), 1, "and lies on the ground");
    let g = w.ground_items.entries()[0];
    assert_eq!(
        g.stack,
        ItemStack {
            cond: COND - WEAR,
            ..held(1)
        },
        "the thrown spear, as it was thrown"
    );
    w.tick(&[Command::Pickup {
        id: 1,
        target: g.id,
    }]);
    assert!(w.ground_items.is_empty(), "taken back");
    assert_eq!(
        carried(&w),
        vec![ItemStack {
            cond: COND - WEAR,
            ..held(1)
        }],
        "the same spear in the pack"
    );
}

#[test]
fn a_spear_that_hits_drops_at_the_victims_feet_and_the_fall_names_it() {
    let mut w = world();
    let (x, z) = lane(&w, 6.0);
    w.players[0].body = Body::at(w.seed, &w.haven, x, z);
    w.players[1].body = Body::at(w.seed, &w.haven, x, z + 4.0);
    w.players[0].inv[0] = held(1);
    w.players[1].hp = 1;
    w.tick(&[]);
    // A little down, into the chest at four metres.
    wind_and_throw(&mut w, 120);
    let v = &w.players[1];
    assert!(v.wounded || v.dead, "the throw did not land");
    assert_eq!(v.death_cause, DEATH_BY_ARROW);
    assert_eq!(v.death_by, 1, "the thrower");
    assert_eq!(v.death_item, SPEAR, "the spear, on the death screen");
    assert_eq!(w.ground_items.len(), 1, "the spear lies on the ground");
    let g = w.ground_items.entries()[0];
    let (dx, dz) = (
        (g.qx - v.body.qx) as f32 * POS_XZ_Q,
        (g.qz - v.body.qz) as f32 * POS_XZ_Q,
    );
    assert!(
        dx * dx + dz * dz < 0.5 * 0.5,
        "at the victim's feet ({dx}, {dz} m off)"
    );
    let dy = (g.qy - v.body.qy) as f32 * POS_Y_Q;
    assert!(dy.max(-dy) < 0.2, "on the ground they stand on ({dy} m)");
    assert_eq!(g.dir, [0; 3], "lying, not stuck in the air");
}

#[test]
fn no_spear_is_thrown_in_the_safe_zone() {
    let mut w = world();
    let t = w.haven.town;
    let (x, z) = sim_core::kit::to_world(&t.placed(), 0.0, -24.0);
    w.players[0].body = Body::at(w.seed, &w.haven, x, z);
    w.players[0].inv[0] = held(1);
    w.tick(&[]);
    assert!(w.players[0].safe, "the market street is in the zone");
    let mut shots = 0;
    let mut script = vec![vec![input(1, 1, BTN_AIM, LEVEL)]];
    for _ in 0..WINDUP {
        script.push(vec![]);
    }
    script.push(vec![input(1, 2, BTN_AIM | BTN_PRIMARY, LEVEL)]);
    for _ in 0..30 {
        script.push(vec![]);
    }
    for cmds in &script {
        w.tick(cmds);
        shots += w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_SHOT)
            .count();
    }
    assert_eq!(shots, 0, "a spear was thrown in the safe zone");
    assert_eq!(carried(&w), vec![held(1)], "and it is still in hand");
}

/// Two runs of one script agree tick for tick, the spear in flight folded
/// into the hash; and a world where it flew differs from one where it did
/// not while it is in the air.
#[test]
fn a_throw_replays_to_the_same_hash() {
    let run = || {
        let mut w = world();
        let (x, z) = lane(&w, 6.0);
        w.players[0].body = Body::at(w.seed, &w.haven, x, z);
        w.players[1].body = Body::at(w.seed, &w.haven, x, z + 4.0);
        w.players[0].inv[0] = held(1);
        w.tick(&[]);
        let mut hashes = Vec::new();
        let mut script = vec![vec![input(1, 1, BTN_AIM, 120)]];
        for _ in 0..WINDUP {
            script.push(vec![]);
        }
        script.push(vec![input(1, 2, BTN_AIM | BTN_PRIMARY, 120)]);
        for _ in 0..40 {
            script.push(vec![]);
        }
        let mut flew = false;
        for cmds in &script {
            w.tick(cmds);
            flew |= !w.arrows.is_empty();
            hashes.push(w.state_hash());
        }
        assert!(flew, "the spear never left the hand");
        hashes
    };
    assert_eq!(run(), run(), "a throw replayed to a different world");
}
