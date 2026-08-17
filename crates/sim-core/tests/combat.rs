//! Melee v0, the sim half: what a swing at a person does, what it refuses
//! to do, and what happens to the person it kills. The content half — that
//! the baked table plays the TTK band `content/balance.toml` declares —
//! is `crates/content/tests/content.rs`; nothing here invents a number,
//! every one comes out of `CombatContent::probe_fixture`.
//!
//! The arrangement throughout: `dev_spawn` pins both players to one point,
//! the ring's own spawn for id 1, which the spawn selector guarantees is
//! clear of scatter for 4 m. Reach is 2 m, so no test here can have its
//! swing stolen by a tree it did not know was there — except the one that
//! is about exactly that, which goes and finds a tree on purpose.

use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM, SWING_INTERVAL_TICKS};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::terrain::{self, Occupant};
use sim_core::world::{Command, World, DEATH_BY_HAND};
use sim_core::yaw_dir;

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
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

const SEED: u64 = 20260802;
/// The fixture's item 0: 34 damage, 2 m reach — three swings to kill.
const SPEAR: u16 = 0;
const FIXTURE_HP: u16 = 100;

/// Two players on one point, armed with fixture item 0 in hotbar slot 0.
fn duel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    for p in w.players.iter_mut().take(2) {
        p.inv[0] = ItemStack {
            item: SPEAR,
            count: 1,
            cond: 0,
        };
    }
    w
}

/// A frame that swings, facing `yaw`, standing still.
fn swing_frame(seq: u16, yaw: u16) -> InputFrame {
    InputFrame {
        seq,
        buttons: BTN_PRIMARY,
        yaw,
        pitch: 128,
        move_x: 0,
        move_z: 0,
        sel: 0,
    }
}

/// Put `victim` `dist` metres in front of `attacker` along `yaw`, and
/// return the yaw the attacker must face to see them.
fn place_in_front(w: &mut World, attacker: usize, victim: usize, yaw: u16, dist: f32) {
    let (fx, fz) = yaw_dir(yaw);
    let a = w.players[attacker].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[victim].body = Body::at(SEED, hv(SEED), ax + fx * dist, az + fz * dist);
}

/// One tick where only `attacker` swings; nobody moves.
fn swing_once(w: &mut World, attacker_id: u32, yaw: u16, seq: u16) {
    w.tick(&[Command::Input {
        id: attacker_id,
        frame: swing_frame(seq, yaw),
    }]);
}

/// Advance past the swing cooldown without swinging.
fn cool_down(w: &mut World) {
    for _ in 0..SWING_INTERVAL_TICKS {
        w.tick(&[]);
    }
}

/// A join grants exactly the content's max hp.
///
/// (One `World` per test throughout this file. It is a large fixed-
/// capacity value — 100 player slots, 8 192 piece records, 16 384 slot
/// lives — and an unoptimized build puts a construction temporary beside
/// every live one, so two in a frame overflow a test thread's stack.)
#[test]
fn hp_comes_from_the_content_table() {
    let w = duel_world();
    assert_eq!(w.players[0].hp, FIXTURE_HP);
    assert_eq!(w.players[1].hp, FIXTURE_HP);
}

/// And inert content grants none — which is what makes a shard whose
/// content never armed a weapon unable to kill anyone.
#[test]
fn inert_content_grants_no_hp_at_all() {
    let mut inert = World::new(SEED);
    inert.tick(&[Command::Join { id: 1 }]);
    assert_eq!(inert.players[0].hp, 0);
}

/// The whole verb, end to end: three swings of a 34-damage spear against
/// a 100-hp player, and the third one kills. The count is not written
/// here as a constant — it is computed from the fixture, so a fixture
/// edit moves the test with it instead of quietly disagreeing.
#[test]
fn three_swings_kill_and_the_count_is_the_content_s() {
    let mut w = duel_world();
    let yaw = 0;
    place_in_front(&mut w, 0, 1, yaw, 1.0);
    let damage = 34u16;
    let expect_hits = FIXTURE_HP.div_ceil(damage);

    let mut hits = 0;
    for seq in 0..expect_hits {
        swing_once(&mut w, 1, yaw, seq);
        hits += 1;
        if w.players[1].deaths > 0 {
            break;
        }
        assert_eq!(
            w.players[1].hp,
            FIXTURE_HP - damage * hits,
            "hp after {hits} landed swings"
        );
        cool_down(&mut w);
        // The victim's position is what the attacker aims at; it never
        // moved, so re-aim is unnecessary — but the respawn below does
        // move it, which is what ends this loop.
    }
    assert_eq!(
        hits, expect_hits,
        "kill took {hits} swings, not {expect_hits}"
    );
    assert_eq!(w.players[1].deaths, 1);
    // Since wire v16 the third swing ends the body rather than replacing
    // it: the corpse waits on the death screen until its own player
    // answers, so "a respawn is a whole body" is asserted after the answer.
    assert!(
        w.players[1].dead,
        "the kill did not put up the death screen"
    );
    assert_eq!(w.players[1].hp, 0);
    w.tick(&[Command::Respawn {
        id: 2,
        on_bag: false,
    }]);
    assert!(!w.players[1].dead);
    assert_eq!(w.players[1].hp, FIXTURE_HP, "a respawn is a whole body");
}

/// Reach: 3 m against a 2 m spear is a miss.
#[test]
fn out_of_reach_is_a_miss() {
    let mut w = duel_world();
    place_in_front(&mut w, 0, 1, 0, 3.0);
    swing_once(&mut w, 1, 0, 0);
    assert_eq!(w.players[1].hp, FIXTURE_HP, "3 m is past a 2 m reach");
}

/// In reach but behind: the aim cone is 30° half-angle, so facing the
/// opposite way cannot land it.
#[test]
fn out_of_the_aim_cone_is_a_miss() {
    let mut w = duel_world();
    place_in_front(&mut w, 0, 1, 0, 1.0);
    swing_once(&mut w, 1, u16::MAX / 2, 0);
    assert_eq!(
        w.players[1].hp, FIXTURE_HP,
        "a target behind you is not aimed at"
    );
}

/// Alone in the world, swinging point-blank on your own position: no
/// target exists, and there is no arrangement in which a weapon finds its
/// own holder.
#[test]
fn no_weapon_may_hit_its_own_holder() {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }]);
    w.players[0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
    };
    swing_once(&mut w, 1, 0, 0);
    assert_eq!(w.players[0].hp, FIXTURE_HP);
}

/// An empty hand and a stack of firewood are the same swing: nothing.
/// This is the shape of v0 and it is deliberate — `weapons.toml` has no
/// unarmed row, and inventing one would be inventing a number.
#[test]
fn a_hand_with_no_weapon_in_it_cannot_hurt() {
    for held in [None, Some(NO_ITEM), Some(9)] {
        let mut w = duel_world();
        place_in_front(&mut w, 0, 1, 0, 1.0);
        w.players[0].inv[0] = match held {
            Some(item) if item != NO_ITEM => ItemStack {
                item,
                count: 1,
                cond: 0,
            },
            _ => ItemStack::default(),
        };
        swing_once(&mut w, 1, 0, 0);
        assert_eq!(
            w.players[1].hp, FIXTURE_HP,
            "held {held:?} must not deal damage"
        );
    }
}

/// One arm, one target: a gatherable in reach takes the swing, and the
/// person standing behind it does not get hit through it. Finds a real
/// tree in the world rather than arranging one, then stands the victim
/// past it on the same bearing.
#[test]
fn a_standing_node_outranks_a_person() {
    // Scan for a tree with a walkable stand point (the same shape the
    // gather tests use), then put both players beside it.
    let mut found = None;
    'scan: for cz in 100..160i32 {
        for cx in 100..160i32 {
            let table = sim_core::terrain::ScatterTable::alpha_default();
            let haven = terrain::haven(SEED);
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant == Occupant::Tree {
                found = Some((s.x, s.z));
                break 'scan;
            }
        }
    }
    let (tx, tz) = found.expect("the island has a tree in the scanned block");

    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    // Stand the attacker 1 m short of the tree on the +x bearing, the
    // victim 0.5 m past it — both inside the 2 m reach, the tree nearer.
    w.dev_spawn = Some((tx - 1.0, tz));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    for p in w.players.iter_mut().take(2) {
        p.inv[0] = ItemStack {
            item: SPEAR,
            count: 1,
            cond: 0,
        };
    }
    w.players[1].body = Body::at(SEED, hv(SEED), tx + 0.5, tz);
    // Face +x: find the yaw whose direction points that way.
    let yaw = (0..256u16)
        .map(|i| i << 8)
        .min_by(|&a, &b| {
            let ax = yaw_dir(a).0;
            let bx = yaw_dir(b).0;
            bx.partial_cmp(&ax).unwrap()
        })
        .unwrap();
    let (fx, _) = yaw_dir(yaw);
    assert!(fx > 0.9, "the chosen yaw must actually face +x");

    let carried = |w: &World| -> u32 { w.players[0].inv.iter().map(|s| s.count as u32).sum() };
    let before = carried(&w);
    swing_once(&mut w, 1, yaw, 0);
    assert_eq!(
        w.players[1].hp, FIXTURE_HP,
        "the tree took the swing; the player behind it must be untouched"
    );
    assert!(
        carried(&w) > before,
        "the swing paid a gather yield, so it really did land on the tree \
         (without this the test would also pass on a swing that hit nothing)"
    );
}

/// Death costs what it should: the body moves to a different beach, the
/// pockets are empty, and the craft queue is gone with them.
#[test]
fn death_takes_the_beach_and_everything_on_you() {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    // No dev_spawn here: the point is that the respawn walks the real
    // ring, so pin the victim beside the attacker by hand instead.
    let before = w.players[1].body;
    let yaw = 0;
    let (fx, fz) = yaw_dir(yaw);
    let a = w.players[0].body;
    w.players[1].body = Body::at(
        SEED,
        hv(SEED),
        a.qx as f32 * POS_XZ_Q + fx,
        a.qz as f32 * POS_XZ_Q + fz,
    );
    let _ = before;
    w.players[0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
    };
    w.players[1].inv[5] = ItemStack {
        item: 3,
        count: 42,
        cond: 0,
    };
    w.players[1].hp = 1; // one swing from the end

    swing_once(&mut w, 1, yaw, 0);

    // The death screen first: the body is down, holding the sentence the
    // wire encodes off it, and it has not moved (wire v16, world.rs).
    {
        let v = &w.players[1];
        assert!(v.dead, "the kill did not put up the death screen");
        assert_eq!(v.death_cause, DEATH_BY_HAND);
        assert_eq!(v.death_by, 1, "the killer is the other player");
        assert_eq!(v.death_item, SPEAR, "the weapon that did it");
        assert!(
            (90..=110).contains(&v.death_range_cm),
            "one metre of reach read as {} cm",
            v.death_range_cm
        );
    }
    w.tick(&[Command::Respawn {
        id: 2,
        on_bag: false,
    }]);

    let v = &w.players[1];
    assert_eq!(v.deaths, 1);
    assert_eq!(v.hp, FIXTURE_HP, "you wake up whole");
    assert!(v.active, "death is a respawn, not a disconnect");
    assert!(
        v.inv.iter().all(|s| s.count == 0),
        "everything you carried is gone"
    );
    assert!(
        v.jobs.iter().all(|j| j.remaining == 0),
        "and the craft queue with it"
    );

    // A different beach: the ring's generation moved, so the point is not
    // the join point.
    let joined = w.spawn_pos_n(2, 0);
    let woke = w.spawn_pos_n(2, 1);
    assert!(
        sim_core::fmath::fabs(joined.0 - woke.0) + sim_core::fmath::fabs(joined.1 - woke.1) > 1.0,
        "generation 1 of the ring landed on the same metre as generation 0"
    );
    assert_eq!(
        (v.body.qx, v.body.qz),
        (
            sim_core::movement::quant_xz(woke.0),
            sim_core::movement::quant_xz(woke.1)
        ),
        "the respawn is the ring's generation-1 point"
    );
}

/// Combat is inside determinism, not beside it: two worlds fed the same
/// inputs hash identically through a kill and the respawn after it. The
/// wasm half of this is `probe_combat` (`test_parity_wasm`).
#[test]
fn a_fight_replays_to_the_same_hash() {
    let run = || {
        let mut w = duel_world();
        place_in_front(&mut w, 0, 1, 0, 1.0);
        for seq in 0..8u16 {
            w.tick(&[
                Command::Input {
                    id: 1,
                    frame: swing_frame(seq, 0),
                },
                Command::Input {
                    id: 2,
                    frame: swing_frame(seq, 0),
                },
            ]);
            for _ in 0..SWING_INTERVAL_TICKS {
                w.tick(&[]);
            }
        }
        (w.state_hash(), w.players[0].deaths + w.players[1].deaths)
    };
    let (h1, deaths) = run();
    let (h2, _) = run();
    assert_eq!(h1, h2, "the same fight must hash the same twice");
    assert!(
        deaths > 0,
        "the arrangement never killed anyone — this test would pass on a world with no combat at all"
    );
}
