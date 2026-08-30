//! `rock` — **the thing in your hand when you open your eyes is a weapon.**
//!
//! A fresh body wakes holding a rock and a torch (`spawn_kit.rs` gates
//! which), on hotbar slot 1, and nothing anywhere gated that the rock can
//! actually hurt a person. Three separate gates each cover a piece and
//! none covers the sentence:
//!
//!   · `content/tests/content.rs` reads `weapons.toml` and holds every
//!     `kind = "melee"` row inside the TTK band — but the band is arithmetic
//!     over a number in a file, and a number in a file is not a swing;
//!   · `crates/sim-core/tests/combat.rs` drives `combat::strike` against
//!     `CombatContent::probe_fixture`, whose damage is invented for the
//!     fixture — it would stay green with `weapons.toml` deleted;
//!   · `crates/server/tests/hunt.rs` kills a pig with the kit, and does it
//!     by *searching* the hotbar for any pocket that swings (its own
//!     comment says the assertion is "SOME pocket can kill, not which
//!     one") — so a kit that dropped the rock and kept a spear would pass.
//!
//! **What none of them can see** is the gap `weapons.toml`'s own header
//! records as a real, shipped, three-month hole: for the whole life of the
//! game before 2026-08-08 no tool had a weapon row, so a fresh character
//! could chop a tree and could not hit a person with the same swing, and
//! every automated check was green. This file is that sentence as an
//! assertion — shipped content, shipped kit, the default hotbar cell, one
//! `World::tick` at a time, through the real verb.
//!
//! No sockets and no clock: a `World`, shipped bakes, and ticks.

use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::TICK_HZ;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::world::{Command, World, DEATH_BY_HAND};

/// The island the shard ships.
const SEED: u64 = 20260731;

/// Ticks a duel may take before it is called a failure — 30 s.
///
/// Generous for `hunt.rs`'s reason: what is gated is *possible*, not
/// quick. The swing cadence is one every 38 ticks, so 30 s is ~23 swings
/// against the five a 20-damage rock needs — the slack is there because a
/// standing node inside the 3×3 ring absorbs a swing before combat sees
/// it (`gather::swing` gets first claim on the arm), and how many nodes
/// stand next to the spawn ring is a property of the seed.
const DUEL_LIMIT_TICKS: u32 = 30 * TICK_HZ;

fn shipped() -> content::Content {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    content::Content::load_dir(&dir).expect("shipped content loads")
}

/// A world with the shipped weapon table, gather table and spawn kit —
/// **no fixtures anywhere in the damage path.** `backpack` is left inert
/// so a death destroys the pockets outright rather than standing a bag up;
/// nothing here reads the corpse's loot, and one less store moving is one
/// less thing that could make a green run mean something else.
fn shipped_world(c: &content::Content) -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.combat = c.bake_combat().expect("weapons bake");
    w.gather = c.bake_gather().expect("gather bakes");
    w.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");
    w
}

/// The wire yaw whose LUT entry points closest to `(dx, dz)` — `hunt.rs`'s
/// helper, so the test steers on the sim's own direction space rather than
/// on an angle it computed some other way.
fn yaw_toward(dx: f32, dz: f32) -> u16 {
    let (mut best, mut best_dot) = (0u16, f32::NEG_INFINITY);
    for i in 0..256u16 {
        let (ex, ez) = sim_core::yaw_dir(i << 8);
        let dot = ex * dx + ez * dz;
        if dot > best_dot {
            best_dot = dot;
            best = i << 8;
        }
    }
    best
}

/// **The rock is a weapon in the shipped table, and it is the one the
/// starting hand is already on.**
///
/// The cheap half, stated first so a failure below has somewhere to point:
/// if this is red the swing was never going to land and the duel that
/// follows would report a mystery.
#[test]
fn the_starter_rock_has_a_melee_row_on_the_default_hotbar_cell() {
    let c = shipped();
    let combat = c.bake_combat().expect("weapons bake");
    let kit = c.bake_spawn_kit().expect("the kit bakes");
    let rock = c.item_index("item.rock").expect("the rock is an item");

    // Slot 0 and not "somewhere in the kit": `InputFrame::sel` defaults to
    // 0 and a fresh client sends that until the player presses a number,
    // so the cell the rock is in IS whether a naked spawn can fight
    // without knowing the hotbar exists.
    assert_eq!(
        kit.stacks[0].item, rock,
        "the spawn kit's first cell is not the rock — a fresh body's default \
         selection is slot 0, so whatever is here is what it swings with"
    );
    assert!(kit.stacks[0].count > 0, "the kit's rock cell is empty");

    let def = combat.held_melee(rock).expect(
        "`item.rock` has no melee row in the shipped weapons table — this is \
         the 2026-08-08 hole again: the starting tool gathers and cannot fight, \
         and every band and fixture check stays green",
    );
    assert!(def.damage > 0, "the rock's melee row does no damage");
    assert!(def.reach_cm > 0, "the rock's melee row has no reach");
}

/// **The whole item, end to end**: two bodies on the shipped island, one
/// of them swinging the rock its own spawn kit granted, and the other one
/// loses hp and then dies of it.
///
/// Nothing is placed by hand except where the two stand. The rock comes
/// from `World::seat`'s grant, the damage from `weapons.toml`, the cadence
/// from `gather::SWING_INTERVAL_TICKS`, and the arm reaches `combat::strike`
/// only by passing through `gather::swing` first — which is the ordering
/// that made the old hole invisible.
#[test]
fn a_naked_spawn_can_kill_another_player_with_the_rock_it_woke_holding() {
    let c = shipped();
    let mut w = shipped_world(&c);
    let rock = c.item_index("item.rock").expect("the rock is an item");

    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    assert!(
        w.players[0].active && w.players[1].active,
        "two bodies seated"
    );
    assert_eq!(
        w.players[0].inv[0].item, rock,
        "the attacker did not wake holding the rock"
    );

    // Stand them 0.6 m apart — inside the rock's 1 m reach and outside
    // `POINT_BLANK_M2`, so the aim cone is actually tested rather than
    // short-circuited by two capsules occupying one point.
    let (x, z) = (
        w.players[0].body.qx as f32 * POS_XZ_Q,
        w.players[0].body.qz as f32 * POS_XZ_Q,
    );
    w.players[1].body = Body::at(w.seed, &w.haven, x + 0.6, z);

    let full = w.combat.player_hp;
    assert!(full > 0, "the shipped table disarms combat");
    assert_eq!(w.players[1].hp, full, "the victim did not seat at full hp");

    let mut frame = InputFrame {
        // Slot 0 explicitly: the default, restated so a future change to
        // `InputFrame::default` cannot quietly move this test's hand.
        sel: 0,
        ..w.players[0].frame
    };
    let mut first_blood = None;
    for t in 0..DUEL_LIMIT_TICKS {
        let (ax, az) = (
            w.players[0].body.qx as f32 * POS_XZ_Q,
            w.players[0].body.qz as f32 * POS_XZ_Q,
        );
        let (bx, bz) = (
            w.players[1].body.qx as f32 * POS_XZ_Q,
            w.players[1].body.qz as f32 * POS_XZ_Q,
        );
        frame.seq = frame.seq.wrapping_add(1);
        frame.yaw = yaw_toward(bx - ax, bz - az);
        frame.buttons = BTN_PRIMARY;
        w.tick(&[Command::Input {
            id: 1,
            frame,
            favour: 0,
        }]);
        if first_blood.is_none() && w.players[1].hp < full {
            first_blood = Some(t);
        }
        if w.players[1].dead {
            break;
        }
    }

    let t = first_blood.unwrap_or_else(|| {
        panic!(
            "30 s of swinging a rock at a man standing 0.6 m away and he is \
             still at {full} hp — the starter tool cannot fight. Check \
             `weapons.toml` for an `item.rock` row before anything else."
        )
    });
    println!("first blood at tick {t}");

    assert!(
        w.players[1].dead,
        "the rock drew blood and could not finish: {} hp left after 30 s",
        w.players[1].hp
    );
    assert_eq!(
        w.players[1].death_cause, DEATH_BY_HAND,
        "a rock kill is a hand kill"
    );
    assert_eq!(w.players[1].death_by, 1, "the death names the wrong killer");
    assert_eq!(
        w.players[1].death_item, rock,
        "the death screen would name the wrong weapon — the sentence a \
         player reads is made of this field (`ui::death::sentence`)"
    );
}

/// …and the reach is a **wall, not a suggestion**. The same duel with the
/// victim standing well outside the rock's 1 m row leaves them untouched.
///
/// Its own test because the one above would pass on a weapon with infinite
/// reach, which is the shape a dropped `reach_cm` check takes: everything
/// still dies, from anywhere.
#[test]
fn the_rocks_reach_is_a_metre_and_not_the_island() {
    let c = shipped();
    let mut w = shipped_world(&c);

    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    let (x, z) = (
        w.players[0].body.qx as f32 * POS_XZ_Q,
        w.players[0].body.qz as f32 * POS_XZ_Q,
    );
    // Four metres: past every melee row in the shipped table (the metal
    // spear's 2 m is the longest) and well past the rock's 1 m.
    w.players[1].body = Body::at(w.seed, &w.haven, x + 4.0, z);
    let full = w.players[1].hp;

    let mut frame = InputFrame {
        sel: 0,
        ..w.players[0].frame
    };
    for _ in 0..DUEL_LIMIT_TICKS {
        let (ax, az) = (
            w.players[0].body.qx as f32 * POS_XZ_Q,
            w.players[0].body.qz as f32 * POS_XZ_Q,
        );
        let (bx, bz) = (
            w.players[1].body.qx as f32 * POS_XZ_Q,
            w.players[1].body.qz as f32 * POS_XZ_Q,
        );
        frame.seq = frame.seq.wrapping_add(1);
        frame.yaw = yaw_toward(bx - ax, bz - az);
        frame.buttons = BTN_PRIMARY;
        // No movement: the attacker swings from where they stand, so the
        // gap is the whole of what this measures.
        w.tick(&[Command::Input {
            id: 1,
            frame,
            favour: 0,
        }]);
    }
    assert_eq!(
        w.players[1].hp, full,
        "a rock reached four metres — the melee reach check is not being applied"
    );
}
