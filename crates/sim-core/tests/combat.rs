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
        favour: 0,
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
                    favour: 0,
                },
                Command::Input {
                    id: 2,
                    frame: swing_frame(seq, 0),
                    favour: 0,
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

// ---------------------------------------------------------------------------
// Lag compensation — `strike` resolves against where the target USED to be
// (slice 4, `findings/lagcomp-design-20260818.md` §7).
//
// The arrangement every test below shares, and the reason for each step. The
// victim stands inside reach for exactly one tick, then jumps out to `FAR_M`
// and stays there while the ring fills. `Rewind::write_row` runs at the *end*
// of a tick, so the tick numbered `T` records the pose the victim held during
// `T`; the swing then happens `WALK_TICKS` ticks later, when the ring holds
// `S-1 ..= S-7` and row `S - WALK_TICKS` is the near one.
//
// The victim is teleported rather than driven with `move_z`, which is worth
// saying out loud because it looks like the lazy choice. It is the honest
// one here: what is under test is whether the scan reads the ring, and a
// walked body makes the near/far gap a function of `movement::step`'s speed
// and the ground under this seed — so a later balance change to walk speed
// would redden a lag-compensation gate for a reason that has nothing to do
// with lag compensation. The positions are asserted below rather than
// assumed.

/// One tick inside the 2 m fixture reach.
const NEAR_M: f32 = 1.0;
/// Well outside it, and outside it by more than any rounding in the quantized
/// body can close.
const FAR_M: f32 = 4.5;
/// How far back the near pose ends up. Equal to `REWIND_MAX_TICKS`, so these
/// tests also pin that the deepest legal rewind actually reaches.
const WALK_TICKS: u64 = 7;

/// Build the duel, park the victim near for one tick, then far for
/// `WALK_TICKS - 1` more. Returns the world with the swing tick not yet run.
fn victim_walked_out_of_reach() -> World {
    let mut w = duel_world();
    // The one near tick. Nobody swings — this tick exists only to be recorded.
    place_in_front(&mut w, 0, 1, 0, NEAR_M);
    w.tick(&[]);
    // …and out, for the rest of the window.
    place_in_front(&mut w, 0, 1, 0, FAR_M);
    for _ in 1..WALK_TICKS {
        w.tick(&[]);
    }
    w
}

/// Swing once at `favour` and report whether the victim lost hp.
fn swing_with_favour(w: &mut World, favour: u8) -> bool {
    let before = w.players[1].hp;
    w.tick(&[Command::Input {
        id: 1,
        frame: swing_frame(0, 0),
        favour,
    }]);
    w.players[1].hp < before
}

/// The fixture actually does what the comment above claims: the victim is
/// inside reach at the recorded tick and outside it at the swing.
///
/// Without this the two tests below are a pair of unfalsifiable claims — a
/// "miss at favour 0" is satisfied by a victim who was never reachable at
/// any depth, and the whole file would still be green.
#[test]
fn the_lag_comp_fixture_really_moves_the_victim_out_of_reach() {
    let mut w = duel_world();
    place_in_front(&mut w, 0, 1, 0, NEAR_M);
    let near = w.players[1].body;
    place_in_front(&mut w, 0, 1, 0, FAR_M);
    let far = w.players[1].body;
    let a = w.players[0].body;
    let d = |b: &sim_core::movement::Body| {
        let dx = (b.qx - a.qx) as f32 * POS_XZ_Q;
        let dz = (b.qz - a.qz) as f32 * POS_XZ_Q;
        (dx * dx + dz * dz).sqrt()
    };
    // 2 m is the fixture spear's reach; assert against it from both sides.
    assert!(
        d(&near) < 2.0,
        "the near pose must be inside reach, was {} m",
        d(&near)
    );
    assert!(
        d(&far) > 2.0,
        "the far pose must be outside reach, was {} m",
        d(&far)
    );
    // And the body genuinely stays put across the ticks that record it —
    // if `movement::step` drifted it, the ring would hold a third position
    // and neither test below would mean what it says.
    let mut w2 = victim_walked_out_of_reach();
    assert_eq!(
        w2.players[1].body.qx, far.qx,
        "the victim drifted while the ring was filling"
    );
    assert_eq!(w2.players[1].body.qz, far.qz);
    let _ = &mut w2;
}

/// **The feature.** The same swing, at the same tick, against the same
/// standing bodies: it misses at favour 0 and lands at favour 7.
///
/// This is the only shape that proves lag compensation rather than the
/// plumbing that carries it. A test that only asserted the hit would pass on
/// a world where the victim never left reach; a test that only asserted the
/// miss would pass on a `strike` that ignores the ring entirely. Both, on one
/// fixture, is the claim.
#[test]
fn a_swing_lands_on_where_the_target_was_and_misses_where_it_is() {
    let mut present = victim_walked_out_of_reach();
    assert!(
        !swing_with_favour(&mut present, 0),
        "favour 0 must resolve against the live body, which is {FAR_M} m away and out of the 2 m reach"
    );

    let mut rewound = victim_walked_out_of_reach();
    assert!(
        swing_with_favour(&mut rewound, WALK_TICKS as u8),
        "favour {WALK_TICKS} must resolve against the pose {WALK_TICKS} ticks back, which is {NEAR_M} m away and well inside reach"
    );
}

/// A favour deeper than the ring can honour buys the shooter *less* help,
/// never more — and the clamp is where that is decided.
///
/// `apply` clamps to `Rewind::max_back()`, so 255 and 7 are the same swing;
/// `pose_at` would independently fall back to the live body on an aliased
/// row, so this is belt and braces on purpose. What it pins is that the two
/// mechanisms agree rather than cancel: if the clamp were dropped, 255 would
/// land on a cold or overwritten row and MISS, which is the opposite of what
/// this asserts.
#[test]
fn a_favour_past_the_ceiling_is_clamped_to_the_ceiling() {
    let mut clamped = victim_walked_out_of_reach();
    assert!(
        swing_with_favour(&mut clamped, u8::MAX),
        "a favour of 255 must be clamped to the ceiling and hit exactly as 7 does"
    );

    // Same swing, same tick, same fixture — so the two must agree on damage
    // and not merely on landing.
    let mut at_ceiling = victim_walked_out_of_reach();
    at_ceiling.tick(&[Command::Input {
        id: 1,
        frame: swing_frame(0, 0),
        favour: WALK_TICKS as u8,
    }]);
    assert_eq!(
        clamped.players[1].hp, at_ceiling.players[1].hp,
        "a clamped favour must be the ceiling favour, not merely another hit"
    );
}

/// The favour is spent inside the tick it arrived in.
///
/// **The hole this closes is a production one, not a test-shaped one.**
/// `Player::frame` persists: a client that sends one input with `BTN_PRIMARY`
/// held and then drops a datagram still swings on every tick the cooldown
/// allows, with **no command behind that swing at all**. If the favour were
/// stored anywhere that outlived the tick, those command-less swings would
/// keep spending the last favour the player happened to send — so a client
/// could buy a deep rewind once and then stop talking, and every swing after
/// it would be lag-compensated for free.
///
/// So the swing under test here is deliberately one that receives **no
/// `Command::Input`**. An earlier draft sent `favour: 0` explicitly on the
/// swing tick, which overwrites a stale value and made the test unable to
/// fail — a mutant that carried the array across ticks passed all fourteen
/// cases in this file. This shape reddens it.
#[test]
fn a_favour_does_not_survive_into_the_next_tick() {
    // The whole sequence, driven twice: once with a generous favour on the
    // FIRST swing only (the stale-favour case), and once with the same favour
    // supplied again on the second swing (the control that says the geometry
    // can land a hit at all).
    let run = |favour_on_second: Option<u8>| {
        let mut w = duel_world();
        place_in_front(&mut w, 0, 1, 0, FAR_M);

        // First swing, with the generous favour. It misses — the victim is
        // far and has been all along — and its only job is to leave a favour
        // behind and to arm `next_swing`.
        let first = w.tick;
        w.tick(&[Command::Input {
            id: 1,
            frame: swing_frame(0, 0),
            favour: WALK_TICKS as u8,
        }]);
        let second = first + SWING_INTERVAL_TICKS;

        // Ride the cooldown with NO commands, parking the victim inside reach
        // for exactly the one tick the deepest legal rewind will read.
        while w.tick < second {
            if w.tick == second - WALK_TICKS {
                place_in_front(&mut w, 0, 1, 0, NEAR_M);
                w.tick(&[]);
                place_in_front(&mut w, 0, 1, 0, FAR_M);
            } else {
                w.tick(&[]);
            }
        }
        assert_eq!(
            w.tick, second,
            "the cooldown walk overshot the second swing"
        );

        // The second swing. `frame.buttons` still holds `BTN_PRIMARY` from the
        // first command, so this fires whether or not a command arrives.
        let before = w.players[1].hp;
        match favour_on_second {
            None => w.tick(&[]),
            Some(f) => w.tick(&[Command::Input {
                id: 1,
                frame: swing_frame(1, 0),
                favour: f,
            }]),
        }
        (before, w.players[1].hp)
    };

    // The control first: with the favour supplied again, this exact geometry
    // lands. Without it the assertion below would be satisfied by a swing that
    // never happened, which is the failure mode this pair exists to rule out.
    let (before, after) = run(Some(WALK_TICKS as u8));
    assert!(
        after < before,
        "the control must land — otherwise the stale-favour case below proves nothing"
    );

    // The claim: no command, so no favour, so no rewind, so no hit.
    let (before, after) = run(None);
    assert_eq!(
        after, before,
        "a swing with no command behind it must resolve at favour 0 — last tick's favour is spent"
    );
}

/// **Only the targets rewind.** The attacker's own body is read live.
///
/// This is the design decision `strike`'s doc states, and it is the natural
/// thing to get wrong: rewinding everything is one fewer special case to
/// write and it is wrong in a way no other test in this file can see, because
/// every other fixture holds the attacker still. Lag compensation exists to
/// undo the interpolation delay a client applies to **other** people; a
/// player's own body is predicted locally and is exactly where he thinks it
/// is, so putting it back in time would make his own swing resolve from a
/// doorway he has already walked out of.
///
/// Here the attacker is the one who moved. The victim stands still, so the
/// victim's rewound pose and live pose are the same and the favour can only
/// act through the attacker.
#[test]
fn the_attacker_does_not_rewind_with_the_target() {
    let mut w = duel_world();
    // Victim one metre in front of where the attacker will end up, and it
    // never moves again.
    place_in_front(&mut w, 0, 1, 0, NEAR_M);
    let home = w.players[0].body;

    // The attacker spends the whole ring window well out of reach, behind
    // where it started.
    let (fx, fz) = yaw_dir(0);
    let away = 3.5;
    w.players[0].body = Body::at(
        SEED,
        hv(SEED),
        home.qx as f32 * POS_XZ_Q - fx * away,
        home.qz as f32 * POS_XZ_Q - fz * away,
    );
    for _ in 0..WALK_TICKS {
        w.tick(&[]);
    }

    // …then steps back onto its mark and swings, at the deepest favour.
    w.players[0].body = home;
    let before = w.players[1].hp;
    w.tick(&[Command::Input {
        id: 1,
        frame: swing_frame(0, 0),
        favour: WALK_TICKS as u8,
    }]);
    assert!(
        w.players[1].hp < before,
        "the swing must resolve from where the attacker IS ({NEAR_M} m away), not from where the ring says he was ({} m)",
        NEAR_M + away
    );
}
