//! Respawn on your own bag: the mechanic that converts "I built a base"
//! into "I have a base".
//!
//! `deploy.rs`'s own unit tests own the *scan* — nearest, owner-only, spent
//! for its cooldown, blind to every archetype that is not a bag. This file
//! owns the **consequence**, which lives in `World::respawn` and which the
//! deploy module cannot reach: that a death actually consults it, that the
//! spawn ring is still there when it answers `None`, and that a body waking
//! on a bag wakes with everything else a respawn owes it.
//!
//! Nothing here invents a number. The clock that does the killing is
//! `SurvivalContent::probe_fixture`, whose spans are seconds precisely so a
//! whole death fits inside a test; the cooldown is `BAG_COOLDOWN_TICKS`,
//! which is `DECISIONS.md` §open's spoken five minutes.

use sim_core::build::{foundation_terrain_ok, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{DeployContent, BAG_COOLDOWN_TICKS};
use sim_core::gather::ItemStack;
use sim_core::limits::TICK_HZ;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World, EV_RESPAWN};

const SEED: u64 = 20260803;

/// Row 3 of `DeployContent::probe_fixture` is the ground-class bag, and it
/// costs one unit of fixture item 5.
const BAG_ROW: u16 = 3;
const BAG_ITEM: u16 = 5;

/// A cell whose center will hold a ground-class deployable, found by
/// scanning the heightfield rather than by typing a coordinate that held at
/// one seed. The rule asked is `build::foundation_terrain_ok` — the same
/// one `deploy::ground_ok` applies — so this fixture cannot drift away from
/// what the sim actually refuses.
///
/// The scan starts one cell off the given point and spirals outward in
/// rings, so "a second bag" lands near the first rather than across the
/// island: the nearest-bag rule is only interesting when the alternatives
/// are close enough for a player to have built both.
fn buildable_cell_near(seed: u64, cx0: u16, cz0: u16, skip: usize) -> (u16, u16) {
    let mut found = 0usize;
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                // Ring, not disc: the interior was covered by smaller r.
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (cx0 as i32 + dx).clamp(0, 1023) as u16;
                let cz = (cz0 as i32 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, x, z) {
                    if found == skip {
                        return (cx, cz);
                    }
                    found += 1;
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// One player, hp from the combat fixture (an inert table grants zero hp
/// and a body at zero can never *reach* zero), the clock armed so hunger
/// can do the killing, and the deploy table that knows what a bag is.
fn lone_world() -> World {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    w
}

fn stand(w: &mut World, cx: u16, cz: u16) {
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = sim_core::movement::Body::at(w.seed, x, z);
}

/// Stand on the cell and place a bag on it through the real verb — no
/// store surgery, so every refusal the sim would raise is one this fixture
/// would trip over rather than route around.
fn place_bag(w: &mut World, cx: u16, cz: u16) {
    stand(w, cx, cz);
    w.players[0].inv[10] = ItemStack {
        item: BAG_ITEM,
        count: 1,
    };
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: 1,
        row: BAG_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "the bag did not place at ({cx}, {cz}) — the fixture, not the mechanic"
    );
}

/// Empty both meters and run until the clock takes the body. Returns the
/// position it woke on, read on the tick the death landed so a later step
/// of movement cannot smear the answer.
fn die(w: &mut World) -> (i32, i32) {
    let before = w.players[0].deaths;
    w.players[0].food = 0;
    w.players[0].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[0].deaths > before {
            return (w.players[0].body.qx, w.players[0].body.qz);
        }
    }
    panic!("the clock never killed the body — the survival fixture changed under this test");
}

/// Where `Body::at` would put a body standing on this cell — the answer the
/// respawn must produce, computed the way the respawn computes it.
fn body_on(w: &World, cx: u16, cz: u16) -> (i32, i32) {
    let (x, z) = cell_center(cx, cz);
    let b = sim_core::movement::Body::at(w.seed, x, z);
    (b.qx, b.qz)
}

/// The last `EV_RESPAWN` in the tick's ring: (player, woke-on-a-bag).
fn respawn_event(w: &World) -> Option<(u32, bool)> {
    w.events
        .entries()
        .iter()
        .rev()
        .find(|e| e.code == EV_RESPAWN)
        .map(|e| (e.a, e.b == 1))
}

/// The whole item, in one assertion: a player who placed a bag wakes on it
/// instead of on a blind ring point on the far side of the island.
#[test]
fn a_death_wakes_you_on_your_own_bag() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);

    // Walk away first: waking *where you happened to die* would pass a
    // weaker version of this test, so the body dies somewhere else.
    let (fx, fz) = buildable_cell_near(SEED, 341, 341, 40);
    stand(&mut w, fx, fz);
    assert_ne!((fx, fz), (cx, cz), "the fixture must move the body");

    let woke = die(&mut w);
    assert_eq!(
        woke,
        body_on(&w, cx, cz),
        "the body did not wake on its own bag"
    );
    assert_eq!(
        respawn_event(&w),
        Some((1, true)),
        "the respawn did not announce that a bag answered it"
    );
    // And it is still a real respawn in every other way the module owes.
    assert_eq!(w.players[0].deaths, 1, "the death still counts");
    assert_eq!(w.players[0].hp, w.combat.player_hp, "full hp");
    assert!(
        w.players[0].food > 0 && w.players[0].water > 0,
        "a body that wakes on a bag is fed exactly like one that wakes on the ring"
    );
}

/// With no bag placed, nothing about the spawn ring moved. This is the
/// assertion that fails if the bag path is wired in as an unconditional
/// replacement rather than as an answer with a fallback.
#[test]
fn no_bag_is_still_the_spawn_ring() {
    let mut w = lone_world();
    let woke = die(&mut w);
    let (x, z) = w.spawn_pos_n(1, 1);
    let b = sim_core::movement::Body::at(SEED, x, z);
    assert_eq!(
        woke,
        (b.qx, b.qz),
        "a bagless death stopped walking the spawn ring"
    );
    assert_eq!(
        respawn_event(&w),
        Some((1, false)),
        "the respawn claimed a bag it did not have"
    );
}

/// The cooldown, from the world's side: killed twice inside five minutes
/// with one bag, the second death goes back to the ring — and the ring it
/// goes to is the generation the *death count* names, not a generation the
/// bag path skipped.
#[test]
fn a_second_death_inside_the_cooldown_falls_back_to_the_ring() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);

    assert_eq!(die(&mut w), body_on(&w, cx, cz), "first death: the bag");
    let after_first = w.tick;

    let woke = die(&mut w);
    assert!(
        w.tick - after_first < BAG_COOLDOWN_TICKS,
        "the second death landed after the cooldown had already lapsed — \
         this test proves nothing at that speed"
    );
    let (x, z) = w.spawn_pos_n(1, 2);
    let b = sim_core::movement::Body::at(SEED, x, z);
    assert_eq!(
        woke,
        (b.qx, b.qz),
        "a bag on cooldown answered a second death inside five minutes"
    );
    assert_eq!(respawn_event(&w), Some((1, false)));
}

/// …and a second bag is a second answer. This is what `BAG_CAP` is a cap
/// *on*: how many deaths in a row a defender can keep answering.
#[test]
fn a_second_bag_answers_the_second_death() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    let (bx, bz) = buildable_cell_near(SEED, 341, 341, 1);
    assert_ne!((cx, cz), (bx, bz));
    place_bag(&mut w, cx, cz);
    place_bag(&mut w, bx, bz);

    // Die on the first bag's cell, so the first bag is the nearer one and
    // the second death has to reach past a spent bag to find the other.
    stand(&mut w, cx, cz);
    assert_eq!(
        die(&mut w),
        body_on(&w, cx, cz),
        "first death: the near bag"
    );
    assert_eq!(
        die(&mut w),
        body_on(&w, bx, bz),
        "the second death did not fall through to the owner's other bag"
    );
    assert_eq!(respawn_event(&w), Some((1, true)));
}

/// The cooldown ends. A bag is a permanent answer that rations itself, not
/// a consumable — the same bag takes the next death once its five minutes
/// are up.
#[test]
fn the_bag_answers_again_once_its_cooldown_lapses() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);
    assert_eq!(die(&mut w), body_on(&w, cx, cz), "first death: the bag");

    // Leap the clock past the cooldown, exactly as the deploy probe leaps
    // it past an upkeep period: every timer here is tick-driven, so the
    // leap is the same arithmetic five real minutes would have done.
    w.tick += BAG_COOLDOWN_TICKS;
    assert_eq!(
        die(&mut w),
        body_on(&w, cx, cz),
        "the bag never woke up — a cooldown that does not lapse is a consumable"
    );
    assert_eq!(respawn_event(&w), Some((1, true)));
}

/// A bag is state, so two worlds that disagree about which bags are spent
/// must not agree about their hashes (wall 5). Without the cooldown in the
/// digest, a WAL replayed from mid-cooldown would put the next death on a
/// bag the first run had already spent.
#[test]
fn the_cooldown_is_hashed_state() {
    let mut w = lone_world();
    let (cx, cz) = buildable_cell_near(SEED, 341, 341, 0);
    place_bag(&mut w, cx, cz);

    // Spend the bag through the store's own verb and nothing else — no
    // death, no movement, no tick — so the cooldown is the only byte of
    // this world that changed and the hash has nothing else to move on.
    let (x, z) = cell_center(cx, cz);
    let before = w.state_hash();
    assert_eq!(
        w.deploys.claim_bag(&w.deploy, 1, x, z, w.tick),
        Some((x, z))
    );
    assert!(w.deploys.bag_ready()[0] > 0, "the bag was not stamped");
    assert_ne!(
        w.state_hash(),
        before,
        "the bag cooldown is not inside state_hash — a replay resuming \
         mid-cooldown would wake a body on a bag the first run had spent, \
         and the two runs would still call themselves the same world"
    );
}
