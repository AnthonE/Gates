//! A stopped arrow becomes a loose stack you can see and take back
//! (`spent.rs`, `grounditem.rs`; `NOW.md` §5 item 2), driven through
//! `World::tick` — the only path a player has. `tests/arrow_recovery.rs`
//! gates the stop, the lodge and the fall below this.

#![allow(clippy::disallowed_macros)]

use sim_core::backpack::BackpackContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::spent::{feet_mm, SpentRec};
use sim_core::world::{Command, World, EV_GATHER};

/// The round, at an index of its own.
const ARROW: u16 = 7;

/// A stack ceiling small enough to fill by hand.
const QUIVER: u16 = 4;

const P: usize = 0;

/// A world with one player standing on the island, arrows stackable, and
/// the loose-stack ladder armed (inert content lays nothing down).
fn world_with_archer() -> Box<World> {
    let mut w = Box::new(World::new(20260731));
    let mut g = GatherContent::EMPTY;
    g.stack_max[ARROW as usize] = QUIVER;
    g.item_count = ARROW + 1;
    w.gather = g;
    w.backpack = BackpackContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    w
}

/// An arrow in nothing, `dx` millimetres east of the player and a metre
/// up, as `ranged` hands one on.
fn stop(w: &mut World, dx: i32) {
    let (x, y, z) = feet_mm(&w.players[P].body);
    w.spent.lodge(SpentRec {
        qx: x + dx,
        qy: y + 1_000,
        qz: z,
        round: ARROW,
        ready_at: w.tick,
        ..SpentRec::default()
    });
}

fn carried(w: &World, item: u16) -> u16 {
    w.players[P]
        .inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| s.count)
        .sum()
}

fn gathers(w: &World) -> Vec<(u16, u16)> {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .map(|e| ((e.b >> 16) as u16, e.b as u16))
        .collect()
}

#[test]
fn a_stopped_arrow_lies_on_the_ground_and_comes_back() {
    let mut w = world_with_archer();
    stop(&mut w, 0);
    w.tick(&[]);

    assert!(w.spent.is_empty(), "it left the stopped-arrow store");
    assert_eq!(w.ground_items.len(), 1, "and lies on the ground");
    let g = w.ground_items.entries()[0];
    assert_eq!(
        (g.stack.item, g.stack.count),
        (ARROW, 1),
        "one of the round, never the weapon"
    );
    let feet = w.players[P].body;
    assert_eq!(
        (g.qx, g.qz),
        (feet.qx, feet.qz),
        "straight down from where it stopped"
    );
    assert!(
        (g.qy - feet.qy).abs() <= 2,
        "on the surface the player stands on (y {} vs feet {})",
        g.qy,
        feet.qy
    );

    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(carried(&w, ARROW), 1, "the round entered the quiver");
    assert!(w.ground_items.is_empty(), "and left the ground");
    assert_eq!(gathers(&w), vec![(ARROW, 1)], "announced as a gather");
}

/// Mutant: a `rest_at` that ignored the sea lays arrows on the seabed.
#[test]
fn an_arrow_that_falls_into_deep_water_is_lost() {
    let mut w = world_with_archer();
    // The island's corner is open sea.
    let floor = sim_core::terrain::ground(w.seed, &w.haven, 5.0, 5.0);
    assert!(
        floor < sim_core::terrain::SEA_LEVEL - sim_core::grounditem::SINK_DEPTH_M,
        "the fixture's point must be deep water (floor {floor} m)"
    );
    w.spent.lodge(SpentRec {
        qx: 5_000,
        qy: 2_000,
        qz: 5_000,
        round: ARROW,
        ready_at: w.tick,
        ..SpentRec::default()
    });
    w.tick(&[]);
    assert!(w.spent.is_empty());
    assert!(w.ground_items.is_empty(), "nothing lies on the seabed");
}

/// One in you rides you for the lodge and falls out at your feet, wherever
/// you have walked.
#[test]
fn an_arrow_in_you_falls_out_at_your_feet_when_the_lodge_ends() {
    let mut w = world_with_archer();
    let ready = w.tick + 5;
    let (x, y, z) = feet_mm(&w.players[P].body);
    w.spent.lodge(SpentRec {
        qx: x,
        qy: y + 1_300,
        qz: z,
        round: ARROW,
        ready_at: ready,
        host: w.players[P].id,
        life: u64::from(w.players[P].deaths),
        ..SpentRec::default()
    });
    w.tick(&[]);
    assert_eq!(w.spent.len(), 1, "still in the body");
    assert!(w.ground_items.is_empty());

    // Walked two metres (moved by hand: the arrow reads the body, not the
    // path it took).
    w.players[P].body.qx += (2.0 / POS_XZ_Q) as i32;
    while w.tick < ready {
        w.tick(&[]);
    }
    w.tick(&[]);
    assert!(w.spent.is_empty(), "the lodge ran out");
    assert_eq!(w.ground_items.len(), 1);
    let g = w.ground_items.entries()[0];
    assert_eq!(
        (g.qx, g.qz),
        (w.players[P].body.qx, w.players[P].body.qz),
        "it fell out where the body is now, not where it was hit"
    );
}

#[test]
fn an_arrow_in_a_body_that_dies_falls_out_at_once() {
    let mut w = world_with_archer();
    let (x, y, z) = feet_mm(&w.players[P].body);
    w.spent.lodge(SpentRec {
        qx: x,
        qy: y + 1_300,
        qz: z,
        round: ARROW,
        ready_at: w.tick + 10_000,
        host: w.players[P].id,
        life: u64::from(w.players[P].deaths),
        ..SpentRec::default()
    });
    w.tick(&[]);
    assert_eq!(w.spent.len(), 1, "riding");
    w.players[P].dead = true;
    w.players[P].deaths += 1;
    w.tick(&[]);
    assert!(w.spent.is_empty(), "a corpse carries nothing");
    assert_eq!(w.ground_items.len(), 1, "it fell where the body died");
}

/// Two arrows at once rest as two stacks, and the verb takes the nearer.
#[test]
fn the_nearest_arrow_is_the_one_that_comes_back() {
    let mut w = world_with_archer();
    stop(&mut w, 3_000);
    stop(&mut w, 300);
    w.tick(&[]);
    assert_eq!(w.ground_items.len(), 2);
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.ground_items.len(), 1, "one taken");
    let far = (feet_mm(&w.players[P].body).0 + 3_000) as f32 / 1000.0;
    let left = w.ground_items.entries()[0].qx as f32 * POS_XZ_Q;
    assert!(
        (left - far).max(far - left) < 0.05,
        "the far one is what is left"
    );
}

#[test]
fn a_full_quiver_spills_the_arrow_at_your_feet() {
    let mut w = world_with_archer();
    for s in w.players[P].inv.iter_mut() {
        *s = ItemStack {
            item: ARROW,
            count: QUIVER,
            cond: 0,
            skin: 0,
        };
    }
    stop(&mut w, 0);
    w.tick(&[]);
    w.tick(&[Command::Pickup { id: 1 }]);
    assert!(w.ground_items.is_empty(), "the stack left the ground");
    assert_eq!(
        w.backpacks.len(),
        1,
        "and what did not fit spilled into a bag"
    );
}

#[test]
fn the_dead_do_not_pick_up() {
    let mut w = world_with_archer();
    stop(&mut w, 0);
    w.tick(&[]);
    w.players[P].dead = true;
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.ground_items.len(), 1, "the arrow is untouched");
    assert!(gathers(&w).is_empty());
}

/// The y the loose stack rests at is the body's own quanta.
#[test]
fn a_resting_arrow_is_in_body_quanta() {
    let mut w = world_with_archer();
    stop(&mut w, 0);
    w.tick(&[]);
    let g = w.ground_items.entries()[0];
    let y = g.qy as f32 * POS_Y_Q;
    let feet = w.players[P].body.qy as f32 * POS_Y_Q;
    assert!(
        (y - feet).max(feet - y) < 0.05,
        "{y} m against feet at {feet} m"
    );
}

// ---------------------------------------------------------------------
// Stuck where it went in
// ---------------------------------------------------------------------

/// The bow, at an index of its own.
const BOW: u16 = 3;

/// A world whose player holds a bow and five arrows, and whose arrows
/// never break — the trunk below is the thing under test, not the odds.
fn world_with_bow() -> Box<World> {
    use sim_core::combat::{AmmoDef, CombatContent, RangedDef, NO_MAG};
    use sim_core::gather::NO_ITEM;
    let mut w = world_with_archer();
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.arrow_break_pct = 0;
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        head_pct: 200,
        limb_pct: 50,
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
        draw_ticks: 0,
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1666,
        drop_mmpt2: 11,
        damage_pct: 100,
        fire_ticks: [0; 2],
    };
    w.combat = c;
    // Long enough a life that nothing here races the despawn.
    w.backpack.base_ticks = 10_000;
    w.players[P].inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[P].inv[7] = ItemStack {
        item: ARROW,
        count: QUIVER,
        cond: 0,
        skin: 0,
    };
    w
}

/// The first tree whose trunk a level shot from five metres south of it
/// meets at chest height over open ground: the tree, its cell, and where
/// that archer stands.
fn a_tree_to_shoot(w: &World) -> (sim_core::terrain::Slot, (u16, u16), f32, f32) {
    use sim_core::terrain::{self, Occupant};
    let span = (terrain::ISLAND_SIZE / terrain::CELL_SIZE) as i32;
    let (_, top) = terrain::occupant_volume(Occupant::Tree);
    // The cells the five metres south of a trunk cross.
    let reach = (6.0 / terrain::CELL_SIZE) as i32 + 1;
    for cz in reach..span {
        for cx in 0..span {
            let s = terrain::scatter(w.seed, &w.scatter, &w.haven, cx, cz);
            if s.occupant != Occupant::Tree {
                continue;
            }
            let (x, z) = (s.x, s.z - 5.0);
            let eye = terrain::ground(w.seed, &w.haven, x, z) + 1.6;
            if eye < s.y + 1.0 || eye > s.y + top * s.scale - 1.0 {
                continue;
            }
            // Nothing on the line but air: no ground within a metre of the
            // flight, no other occupant in the cells it crosses.
            let clear = (1..20).all(|i| {
                let zi = z + i as f32 * 0.25;
                terrain::ground(w.seed, &w.haven, x, zi) < eye - 1.0
            }) && (cz - reach..cz).all(|oz| {
                (cx - 1..=cx + 1).all(|ox| {
                    terrain::scatter(w.seed, &w.scatter, &w.haven, ox, oz).occupant
                        == Occupant::None
                })
            });
            let (dx, dz) = (s.x - w.haven.x, s.z - w.haven.z);
            if clear && dx * dx + dz * dz > 80.0 * 80.0 {
                return (s, (cx as u16, cz as u16), x, z);
            }
        }
    }
    panic!("this island drew no tree to shoot");
}

fn loose(w: &mut World) {
    use sim_core::input::{InputFrame, BTN_PRIMARY};
    let frame = InputFrame {
        buttons: BTN_PRIMARY,
        // Yaw 0 flies +z; just over level.
        yaw: 0,
        pitch: 128,
        sel: 0,
        ..InputFrame::default()
    };
    w.tick(&[Command::Input {
        id: 1,
        frame,
        favour: 0,
    }]);
    // Let go, or the held button looses again at the bow's cadence.
    let frame = InputFrame {
        buttons: 0,
        seq: 1,
        ..frame
    };
    w.tick(&[Command::Input {
        id: 1,
        frame,
        favour: 0,
    }]);
    for _ in 0..60 {
        if w.arrows.is_empty() {
            break;
        }
        w.tick(&[]);
    }
    assert!(w.arrows.is_empty(), "the arrow never stopped");
}

/// Shot into a trunk, an arrow stays in the trunk — at the bark, up off the
/// ground, pointing the way it flew — and falls to the ground under it when
/// the tree is felled. Mutant: laying a stuck arrow with `rest_at` puts it
/// on the ground at once; dropping `unstick_arrows` leaves it in the air
/// where the trunk was.
#[test]
fn an_arrow_sticks_in_a_trunk_and_falls_when_it_is_felled() {
    use sim_core::movement::Body;
    use sim_core::terrain::{self, Occupant};
    let mut w = world_with_bow();
    let (tree, (cx, cz), x, z) = a_tree_to_shoot(&w);
    w.players[P].body = Body::at(w.seed, &w.haven, x, z);
    loose(&mut w);

    assert_eq!(w.ground_items.len(), 1, "the arrow is a loose stack");
    let g = w.ground_items.entries()[0];
    assert_eq!(g.stack.item, ARROW);
    let (gx, gy, gz) = (
        g.qx as f32 * POS_XZ_Q,
        g.qy as f32 * POS_Y_Q,
        g.qz as f32 * POS_XZ_Q,
    );
    let (ox, oz) = (gx - tree.x, gz - tree.z);
    let off = (ox * ox + oz * oz).sqrt();
    assert!(
        off < 0.6 && gz < tree.z,
        "it is in the bark on the archer's side of the trunk ({off} m off \
         the axis, z {gz} against the trunk's {})",
        tree.z
    );
    let ground = terrain::ground(w.seed, &w.haven, gx, gz);
    assert!(
        gy > ground + 0.5,
        "it is up in the trunk, not on the ground (y {gy} m, ground {ground} m)"
    );
    assert_ne!(g.dir, [0; 3], "a stuck arrow carries the way it flew");
    assert_eq!(g.dir[2], 127, "flying +z, and mostly so");
    assert!(g.dir[1] <= 0 && g.dir[0].abs() < 10, "{:?}", g.dir);

    // Felled: within a second it is on the ground, a new stack.
    assert!(
        w.slot_lives
            .harvest(cx, cz, Occupant::Tree, u64::MAX / 2, &mut w.events),
        "the fixture fells the tree"
    );
    for _ in 0..31 {
        w.tick(&[]);
    }
    assert_eq!(w.ground_items.len(), 1, "it fell, it was not lost");
    let f = w.ground_items.entries()[0];
    assert_eq!(f.dir, [0; 3], "it lies now");
    assert_ne!(f.id, g.id, "as a new stack, so every client redraws it");
    let fy = f.qy as f32 * POS_Y_Q;
    let floor = terrain::ground(
        w.seed,
        &w.haven,
        f.qx as f32 * POS_XZ_Q,
        f.qz as f32 * POS_XZ_Q,
    );
    assert!(
        (fy - floor).max(floor - fy) < 0.05,
        "on the ground under where it was (y {fy} m, ground {floor} m)"
    );
    assert_eq!((f.qx, f.qz), (g.qx, g.qz), "straight down");
}

/// A stuck arrow whose trunk still stands stays put, however long it is
/// asked. Mutant: inverting `still_stuck` drops every stuck arrow at its
/// first check.
#[test]
fn an_arrow_in_a_standing_trunk_stays_there() {
    use sim_core::movement::Body;
    let mut w = world_with_bow();
    let (_, _, x, z) = a_tree_to_shoot(&w);
    w.players[P].body = Body::at(w.seed, &w.haven, x, z);
    loose(&mut w);
    let g = w.ground_items.entries()[0];
    for _ in 0..95 {
        w.tick(&[]);
    }
    assert_eq!(
        w.ground_items.entries(),
        &[g][..],
        "three checks later it is the same stack in the same place"
    );
}

/// An arrow standing in you comes out with `E`, into your pack — Rust's
/// can be pulled by anyone, the body's own player too — and a loose stack
/// as near as it is taken first, because that is what `E` names. Mutant:
/// skipping `pull_arrow` leaves the arrow in the body and the pack empty.
#[test]
fn an_arrow_in_you_comes_out_with_e() {
    let mut w = world_with_archer();
    let (x, y, z) = feet_mm(&w.players[P].body);
    let ready_at = w.tick + 10_000;
    w.spent.lodge(SpentRec {
        qx: x,
        qy: y,
        qz: z,
        round: ARROW,
        ready_at,
        host: w.players[P].id,
        life: u64::from(w.players[P].deaths),
        off: [0, 120, 20],
        dir: [0, 0, -127],
    });
    stop(&mut w, 0);
    w.tick(&[]);
    assert_eq!(w.spent.len(), 1, "it stands in the body");
    assert_eq!(w.ground_items.len(), 1, "with a loose stack at its feet");

    w.tick(&[Command::Pickup { id: 1 }]);
    assert!(
        w.ground_items.is_empty(),
        "the loose stack, as near, goes first"
    );
    assert_eq!(w.spent.len(), 1, "and the arrow stays in");

    w.tick(&[Command::Pickup { id: 1 }]);
    assert!(w.spent.is_empty(), "then it comes out");
    assert_eq!(carried(&w, ARROW), 2, "both in the pack");
    assert_eq!(gathers(&w), vec![(ARROW, 1)], "announced as a gather");
}
