//! The animal roster (`mob.rs`, `reference/ANIMALS.md` §9).
//!
//! Everything here asserts on observable sim state — never on elapsed time,
//! and never on a number this file chose. The species numbers come from
//! `MobContent::probe_fixture`; the placement rules come from `terrain.rs`.

use sim_core::backpack::BackpackContent;
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_MOBS, MAX_PLAYERS, MOB_ID_TAG, MOB_THINK_TICKS, MOB_WAKE_CM};
use sim_core::mob::{self, MobContent, MOB_PIG};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::terrain;
use sim_core::world::{Command, World};
use sim_core::yaw_dir;

fn armed(seed: u64) -> World {
    let mut w = World::new(seed);
    w.mob = MobContent::probe_fixture();
    w
}

/// A seed's roster finds land for effectively all of its slots, and every
/// home is somewhere a player could stand: inland, walkable, and outside
/// the two authored sites.
#[test]
fn every_home_is_on_walkable_inland_ground() {
    for seed in [1u64, 7, 99, 2026] {
        let w = World::new(seed);
        let homed = w.mobs.homed();
        assert!(
            homed >= MAX_MOBS - 2,
            "seed {seed}: only {homed}/{MAX_MOBS} roster slots found a home — \
             24 draws against a continent should not miss this often"
        );
        for m in w.mobs.m.iter() {
            if !m.homed {
                continue;
            }
            let x = m.home_qx as f32 * POS_XZ_Q;
            let z = m.home_qz as f32 * POS_XZ_Q;
            assert!(
                terrain::height(seed, x, z) > terrain::BEACH_MAX_H,
                "seed {seed}: a home at ({x:.1}, {z:.1}) is on the beach or in the sea"
            );
            assert!(terrain::slope(seed, x, z) < 1.0);
            assert!(!terrain::in_haven(&w.haven, x, z));
            assert!(!terrain::in_waystation(&w.haven, x, z));
        }
    }
}

/// Inert content is a shard without wildlife, not a shard with invisible
/// wildlife: no slot hatches and the roster contributes nothing to the
/// state hash.
#[test]
fn inert_content_never_hatches() {
    let mut w = World::new(4);
    let bare = World::new(4).state_hash();
    for _ in 0..600 {
        w.tick(&[]);
    }
    assert_eq!(w.mobs.alive(), 0);
    let mut quiet = World::new(4);
    for _ in 0..600 {
        quiet.tick(&[]);
    }
    assert_eq!(w.state_hash(), quiet.state_hash());
    assert_ne!(bare, 0);
}

/// Armed content hatches every homed slot on the first tick, at its own
/// home, standing on the ground.
#[test]
fn armed_content_hatches_the_whole_roster() {
    let mut w = armed(11);
    w.tick(&[]);
    assert_eq!(w.mobs.alive(), w.mobs.homed());
    for m in w.mobs.m.iter().filter(|m| m.alive) {
        assert_eq!(m.kind, MOB_PIG);
        assert_eq!(m.hp, MobContent::probe_fixture().def(MOB_PIG).hp);
        assert_eq!(m.body.qx, m.home_qx);
        assert_eq!(m.body.qz, m.home_qz);
        assert!(m.body.grounded);
    }
}

/// Dormancy is the reference game's measure and ours is a hard skip: with
/// nobody in the world, no animal moves a quantum however long the shard
/// runs.
#[test]
fn an_empty_shard_never_moves_an_animal() {
    let mut w = armed(11);
    w.tick(&[]);
    let before: Vec<_> = w.mobs.m.iter().map(|m| (m.body.qx, m.body.qz)).collect();
    for _ in 0..900 {
        w.tick(&[]);
    }
    let after: Vec<_> = w.mobs.m.iter().map(|m| (m.body.qx, m.body.qz)).collect();
    assert_eq!(
        before, after,
        "a dormant roster stepped with nobody watching"
    );
    assert!(w.mobs.m.iter().filter(|m| m.alive).all(|m| !m.awake));
}

/// Seat a player next to a chosen animal, run, and watch that animal — and
/// only animals near a player — come alive and walk.
#[test]
fn an_animal_near_a_player_wakes_and_walks() {
    let mut w = armed(11);
    w.tick(&[]);
    let slot = w.mobs.m.iter().position(|m| m.alive).expect("a live pig");
    let (mx, mz) = (
        w.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
        w.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
    );
    // Well outside the spook radius (12 m) so the animal ambles rather
    // than bolting, and well inside the wake radius.
    w.dev_spawn = Some((mx + 40.0, mz));
    w.tick(&[Command::Join { id: 1 }]);
    let start = (w.mobs.m[slot].body.qx, w.mobs.m[slot].body.qz);

    let mut moved = false;
    for _ in 0..(MOB_THINK_TICKS * 20) {
        w.tick(&[]);
        if (w.mobs.m[slot].body.qx, w.mobs.m[slot].body.qz) != start {
            moved = true;
        }
    }
    assert!(moved, "an animal 40 m from a player never took a step");
    assert!(w.mobs.m[slot].awake);

    // And the far half of the island is still asleep: the wake radius is a
    // radius, not a global switch.
    let far = w
        .mobs
        .m
        .iter()
        .filter(|m| m.alive)
        .find(|m| {
            let dx = (m.body.qx - w.players[0].body.qx) as i64 * 3;
            let dz = (m.body.qz - w.players[0].body.qz) as i64 * 3;
            dx * dx + dz * dz > MOB_WAKE_CM * MOB_WAKE_CM
        })
        .map(|m| m.awake);
    assert_eq!(far, Some(false), "an animal past the wake radius was awake");
}

/// The leash. Whatever the wander draws, an animal stays inside its roam
/// radius of the home the seed gave it — the reference game's own fix for
/// animals that walked to the coast.
#[test]
fn the_leash_holds() {
    let mut w = armed(11);
    w.tick(&[]);
    let slot = w.mobs.m.iter().position(|m| m.alive).expect("a live pig");
    let (mx, mz) = (
        w.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
        w.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
    );
    w.dev_spawn = Some((mx + 40.0, mz));
    w.tick(&[Command::Join { id: 1 }]);

    let roam = MobContent::probe_fixture().def(MOB_PIG).roam_cm;
    // The leash is a decision, not a wall: the animal turns for home on its
    // next think tick, so it can overshoot by up to one think's walk.
    // `MOB_THINK_TICKS` steps at `WALK_SPEED` is 1.5 m; two thinks of
    // margin is the honest bound and it is derived, not chosen.
    let slack = 2 * (MOB_THINK_TICKS as i64) * 10;
    for _ in 0..9_000 {
        w.tick(&[]);
        let m = &w.mobs.m[slot];
        let dx = (m.body.qx - m.home_qx) as i64 * 3;
        let dz = (m.body.qz - m.home_qz) as i64 * 3;
        let d2 = dx * dx + dz * dz;
        assert!(
            d2 <= (roam + slack) * (roam + slack),
            "a pig wandered {} cm² from home, past a {roam} cm leash",
            d2
        );
    }
}

/// Determinism, the whole point: two worlds on one seed, fed identical
/// commands, agree on every hash including the ticks animals are moving.
#[test]
fn two_shards_agree_about_the_roster() {
    let build = || {
        let mut w = armed(11);
        w.tick(&[]);
        let slot = w.mobs.m.iter().position(|m| m.alive).unwrap();
        let (mx, mz) = (
            w.mobs.m[slot].body.qx as f32 * POS_XZ_Q,
            w.mobs.m[slot].body.qz as f32 * POS_XZ_Q,
        );
        w.dev_spawn = Some((mx + 20.0, mz));
        w.tick(&[Command::Join { id: 1 }]);
        w
    };
    let mut a = build();
    let mut b = build();
    for _ in 0..1_200 {
        a.tick(&[]);
        b.tick(&[]);
        assert_eq!(a.state_hash(), b.state_hash());
    }
    assert!(
        a.mobs.m.iter().any(|m| m.awake),
        "nothing woke up to compare"
    );
}

/// The fixture's item 0: 34 damage, 2 m reach — three swings kill the
/// 80 hp fixture pig. Same numbers `tests/backpack.rs`'s duel uses.
const SPEAR: u16 = 0;
/// Two fixture item indices for the loot rows. Indices are a property of
/// the loaded set; these exist in `GatherContent::probe_fixture`'s ladder
/// (stack cap 100) and `BackpackContent::probe_fixture`'s (90 ticks).
const MEAT: u16 = 5;
const HIDE: u16 = 7;

/// A world with a huntable, stationary pig one metre in front of an armed
/// player, on the spawn ring's own pad — which the spawn selector keeps
/// clear of scatter for 4 m, so no tree or barrel can absorb the swing.
///
/// The pig is made docile (gait and flee gait 0) because this file gates
/// the *kill's consequences*, not the chase — `crates/server/tests/hunt.rs`
/// owns the chase, against shipped content. Returns the world and the
/// pig's roster slot.
fn hunt_world() -> (World, usize) {
    let mut w = World::new(11);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.mob = MobContent::probe_fixture();
    let def = &mut w.mob.defs[MOB_PIG as usize];
    def.gait = 0;
    def.flee_gait = 0;
    def.loot[0] = ItemStack {
        item: MEAT,
        count: 3,
    };
    def.loot[1] = ItemStack {
        item: HIDE,
        count: 15,
    };
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }]);
    let slot = w.mobs.m.iter().position(|m| m.alive).expect("a live pig");
    // Stand the pig one metre in front of the player's yaw-0 facing —
    // inside the weapon's 2 m reach, outside point blank.
    let (fx, fz) = yaw_dir(0);
    let b = w.players[0].body;
    let (ax, az) = (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q);
    w.mobs.m[slot].body = Body::at(11, ax + fx, az + fz);
    (w, slot)
}

/// Hold the swing until the pig dies. Cadence, not re-pressing, paces the
/// swings; every tick is checked because the kill can land on any of them.
fn kill_the_pig(w: &mut World, slot: usize) {
    w.players[0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
    };
    for seq in 0..(SWING_INTERVAL_TICKS as u16 * 8) {
        let frame = InputFrame {
            seq,
            buttons: BTN_PRIMARY,
            yaw: 0,
            pitch: 128,
            sel: 0,
            ..InputFrame::default()
        };
        w.tick(&[Command::Input { id: 1, frame }]);
        if !w.mobs.m[slot].alive {
            return;
        }
    }
    panic!("three fixture spear hits must kill inside eight swing intervals");
}

/// The kill leaves a body, not a payment: the killer's inventory holds
/// nothing new until the loot verb, a ground bag stands where the animal
/// died holding exactly the content rows, and E takes them — the direct
/// `EV_GATHER` pay this replaced would redden every assert here.
#[test]
fn a_killed_pig_leaves_a_corpse_bag_and_pays_nothing_by_itself() {
    let (mut w, slot) = hunt_world();
    let (bx, bz) = (w.mobs.m[slot].body.qx, w.mobs.m[slot].body.qz);
    kill_the_pig(&mut w, slot);

    let carried: Vec<(u16, u16)> = w.players[0]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(
        carried,
        vec![(SPEAR, 1)],
        "the blow itself paid into the killer's inventory — the corpse bag \
         was bypassed"
    );
    assert_eq!(w.backpacks.len(), 1, "the kill stood up exactly one bag");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        (bag.qx, bag.qz),
        (bx, bz),
        "the bag is where the animal died, not where the killer stood"
    );
    assert_eq!(bag.owner, mob::mob_id(slot), "the bag is the dead animal's");
    let held: Vec<(u16, u16)> = bag
        .items
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(
        held,
        vec![(MEAT, 3), (HIDE, 15)],
        "the corpse holds the content rows verbatim"
    );

    // E: the same loot verb every bag answers. The bag empties into the
    // killer and leaves; the amounts are the rows, exactly.
    w.tick(&[Command::Loot { id: 1 }]);
    let carried: Vec<(u16, u16)> = w.players[0]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(carried, vec![(SPEAR, 1), (MEAT, 3), (HIDE, 15)]);
    assert!(
        w.backpacks.is_empty(),
        "an emptied corpse leaves immediately"
    );
}

/// The stated inert-ladder policy at the call site, as a gate: content
/// that never armed the backpack module stands up no corpse bag, and the
/// kill pays nothing rather than falling back to a direct grant.
#[test]
fn an_inert_bag_ladder_means_a_kill_drops_nothing() {
    let (mut w, slot) = hunt_world();
    w.backpack = BackpackContent::EMPTY;
    kill_the_pig(&mut w, slot);
    assert!(!w.mobs.m[slot].alive, "the kill itself still lands");
    assert!(w.backpacks.is_empty(), "no bag under a disarmed module");
    let carried: Vec<(u16, u16)> = w.players[0]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(carried, vec![(SPEAR, 1)], "and no direct pay either");
}

/// The id tag is the whole wire contract: a mob id is not a player id, and
/// it round-trips to its own roster slot.
#[test]
fn mob_ids_never_collide_with_player_ids() {
    for slot in 0..MAX_MOBS {
        let id = mob::mob_id(slot);
        assert_ne!(id & MOB_ID_TAG, 0);
        assert_eq!(mob::slot_of_id(id), Some(slot));
    }
    // Every player id the server can mint (`net.rs` masks the generation
    // for exactly this reason) is below the tag.
    for slot in 0..MAX_PLAYERS as u32 {
        for gen in [0u32, 1, 0x7F_FFFF] {
            let id = ((gen & 0x007F_FFFF) << 8) | slot;
            assert_eq!(
                id & MOB_ID_TAG,
                0,
                "player id {id:#x} collides with the mob tag"
            );
            assert_eq!(mob::slot_of_id(id), None);
        }
    }
}
