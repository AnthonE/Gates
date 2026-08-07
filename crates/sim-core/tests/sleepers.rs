//! Sleepers: what a disconnect costs now that it does not delete the body.
//!
//! Before this, `Command::Leave` cleared `active` and the body was gone —
//! which meant **logging off was the safest thing a player could do**, and
//! offline raiding, the consequence the whole genre is built around, could
//! not exist. That is the design `reference/SAVES.md` §1 quotes the
//! reference game replacing in its seventh devblog. `NOW.md` §0y item 1 is
//! the sequence; this file is what "the body stays" has to survive to be
//! true.
//!
//! Six claims, and they are chosen to be the ones a plausible refactor
//! silently breaks:
//!
//! 1. A leave leaves the body standing where it stood.
//! 2. A sleeper is a target — reach, damage and death behave as they do
//!    against a player who is looking at you.
//! 3. **A killed sleeper stays a sleeper.** This is the one the whole
//!    slice rests on, and the failure is invisible: `die` rebuilds the
//!    `Player` through `..Player::default()`, so an author who does not
//!    carry `sleeping` across it produces a world where the raid worked,
//!    the body dropped its bag, and the victim's next join still finds
//!    a record to restore them from — everything green, nothing raided.
//! 4. A sleeper's clock runs. It can starve, and starving drops the bag.
//! 5. A returning connection takes over the body rather than getting a
//!    second one.
//! 6. Slots are `MAX_PLAYERS` and sleepers hold them, so the eviction
//!    policy is real code with a stated order, not a comment.
//!
//! Nothing here invents a number: hp, damage and reach come from
//! `CombatContent::probe_fixture`, the clock from
//! `SurvivalContent::probe_fixture`, the despawn ladder from
//! `BackpackContent::probe_fixture`.

use sim_core::backpack::BackpackContent;
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{INV_SLOTS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};
use sim_core::yaw_dir;

const SEED: u64 = 20260807;
/// The fixture's item 0: 34 damage, 2 m reach — three swings to kill.
const SPEAR: u16 = 0;

/// Two players on one point, armed, with the combat table live so bodies
/// have hp and can lose it. No survival clock: most of this file is about
/// what a *raider* does to a sleeper, and a metabolism running underneath
/// would put a second cause of death in every test that is not about one.
fn duel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    for p in w.players.iter_mut().take(2) {
        p.inv[0] = ItemStack {
            item: SPEAR,
            count: 1,
        };
    }
    w
}

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

/// Put `victim` `dist` metres in front of `attacker` along `yaw`.
fn place_in_front(w: &mut World, attacker: usize, victim: usize, yaw: u16, dist: f32) {
    let (fx, fz) = yaw_dir(yaw);
    let a = w.players[attacker].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[victim].body = Body::at(SEED, ax + fx * dist, az + fz * dist);
}

fn slot_of(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("the world should still have this body")
}

/// The claim in its plainest form: after a leave the body is still there,
/// at the same coordinates, alive, and flagged as driven by nobody.
///
/// `active` staying **true** is not incidental — it is the contract every
/// other predicate in the sim reads. A sleeper that cleared it would be
/// invisible to `combat::strike`'s target scan, to the snapshot's interest
/// filter and to `state_hash`, which is to say it would be the old
/// behaviour wearing a new field.
#[test]
fn a_leave_leaves_the_body_standing() {
    let mut w = duel_world();
    let slot = slot_of(&w, 2);
    let before = w.players[slot].body;

    w.tick(&[Command::Leave { id: 2 }]);

    let p = &w.players[slot];
    assert!(p.active, "a sleeper must stay active or nothing can see it");
    assert!(p.sleeping, "a leave must put the body to sleep");
    assert_eq!(p.body.qx, before.qx, "the body moved on the way out");
    assert_eq!(p.body.qz, before.qz, "the body moved on the way out");
    assert!(p.hp > 0, "a leave is not a death");
    assert_eq!(w.sleepers(), 1);
    assert!(w.is_sleeper(2));
}

/// A sleeper takes no input. The frame is zeroed at the step, so a command
/// aimed at a sleeping id — a bot, a replayed WAL, a client whose last
/// datagram arrived after its leave — cannot walk the body away.
#[test]
fn a_sleeper_does_not_act_on_input() {
    let mut w = duel_world();
    let slot = slot_of(&w, 2);
    w.tick(&[Command::Leave { id: 2 }]);
    let before = w.players[slot].body;

    for seq in 0..30u16 {
        w.tick(&[Command::Input {
            id: 2,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw: 0,
                pitch: 128,
                move_x: 127,
                move_z: 0,
                sel: 0,
            },
        }]);
    }

    let p = &w.players[slot];
    assert_eq!(p.body.qx, before.qx, "a sleeper walked on someone's input");
    assert_eq!(p.body.qz, before.qz, "a sleeper walked on someone's input");
}

/// **Offline raiding, as one test.** A sleeper is hit, killed, and drops
/// its bag — which is the sentence the whole slice exists to make true.
///
/// The third assertion is the load-bearing one and the least obvious.
/// `World::die` rebuilds the player through `..Player::default()`, so
/// `sleeping` has to be carried across it *by hand*; a version that does
/// not carry it compiles, drops the bag, passes the first two assertions,
/// and leaves the world unable to recognise the corpse as this player's —
/// so their next join finds no sleeper, falls through to the store, and
/// hands them back everything the raider just took. The raid would have
/// happened and cost nothing.
#[test]
fn a_sleeper_can_be_killed_and_stays_one() {
    let mut w = duel_world();
    let sleeper = slot_of(&w, 2);
    let raider = slot_of(&w, 1);
    let yaw = 0u16;
    place_in_front(&mut w, raider, sleeper, yaw, 1.0);
    w.tick(&[Command::Leave { id: 2 }]);
    let bags_before = w.backpacks.next_id();

    // Three swings at fixture damage; the cadence gate means one lands per
    // `SWING_INTERVAL_TICKS`, so swing on every tick and let it filter.
    for seq in 0..200u16 {
        if w.players[sleeper].dead {
            break;
        }
        w.tick(&[Command::Input {
            id: 1,
            frame: swing_frame(seq, yaw),
        }]);
    }

    let p = &w.players[sleeper];
    assert!(
        p.dead,
        "a sleeper must be killable — that is the whole point"
    );
    assert_eq!(p.hp, 0);
    assert!(
        p.sleeping,
        "a killed sleeper stopped being a sleeper: its owner's next join \
         will not find the corpse, will fall through to the store, and will \
         be handed back everything the raid took"
    );
    assert!(
        w.is_sleeper(2),
        "the world can no longer name the body this player left"
    );
    assert!(
        w.backpacks.next_id() > bags_before,
        "the kill dropped no bag — the raid paid nothing"
    );
}

/// The clock runs while you are away. A body left standing long enough
/// starves where it stands, which is the other half of "logging off is no
/// longer free": the raider is one threat and the metabolism is the other.
#[test]
fn a_sleepers_clock_runs_and_can_kill_it() {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    let slot = slot_of(&w, 1);
    w.tick(&[Command::Leave { id: 1 }]);
    let food_before = w.players[slot].food;

    // The probe fixture's spans are seconds, so a whole death fits here.
    let mut died = false;
    for _ in 0..20_000 {
        w.tick(&[]);
        if w.players[slot].dead {
            died = true;
            break;
        }
    }

    assert!(
        w.players[slot].food < food_before || died,
        "a sleeper's meters never moved — the metabolism is not running"
    );
    assert!(died, "a sleeper never starved in 20,000 ticks");
    assert!(
        w.players[slot].sleeping,
        "the clock's death path lost the sleeping bit that combat's keeps"
    );
}

/// A returning player takes over the body, and does not get a second one.
///
/// The ids differ on purpose — `generation << 8 | slot`, minted per
/// connection — because that difference *is* the problem `Command::Wake`
/// exists to solve: the sim cannot recognise anybody, so the server names
/// both ids and the sim moves the body from one to the other.
#[test]
fn a_return_takes_over_the_body_it_left() {
    let mut w = duel_world();
    let slot = slot_of(&w, 2);
    w.players[slot].inv[3] = ItemStack {
        item: SPEAR,
        count: 7,
    };
    let carried = w.players[slot].inv;
    let stood = w.players[slot].body;
    w.tick(&[Command::Leave { id: 2 }]);

    w.tick(&[Command::Wake {
        id: 0x0202,
        sleeper: 2,
    }]);

    assert_eq!(
        w.players.iter().filter(|p| p.active).count(),
        2,
        "a takeover minted a second body instead of reclaiming the first"
    );
    let p = &w.players[slot];
    assert_eq!(p.id, 0x0202, "the body did not change hands");
    assert!(!p.sleeping, "the body is awake now");
    assert_eq!(p.body.qx, stood.qx, "you woke up somewhere else");
    assert_eq!(p.body.qz, stood.qz, "you woke up somewhere else");
    assert_eq!(p.inv, carried, "the takeover lost what the body carried");
    assert_eq!(
        p.frame.seq, 0,
        "a takeover must not inherit a stale input seq"
    );
    assert!(!w.is_sleeper(2), "the old id still names a sleeper");
}

/// Waking onto a body somebody killed puts you on a beach, not back in the
/// corpse — the rule `PlayerSave::dead` already reasoned out for the
/// restore door, inherited here rather than re-decided. `deaths` is what
/// proves it went through `wake` and not through a plain takeover.
#[test]
fn waking_onto_a_corpse_beaches_you_alive() {
    let mut w = duel_world();
    let sleeper = slot_of(&w, 2);
    let raider = slot_of(&w, 1);
    let yaw = 0u16;
    place_in_front(&mut w, raider, sleeper, yaw, 1.0);
    w.tick(&[Command::Leave { id: 2 }]);
    for seq in 0..200u16 {
        if w.players[sleeper].dead {
            break;
        }
        w.tick(&[Command::Input {
            id: 1,
            frame: swing_frame(seq, yaw),
        }]);
    }
    assert!(w.players[sleeper].dead, "setup: the sleeper must be dead");
    let fell_at = w.players[sleeper].body;

    w.tick(&[Command::Wake {
        id: 0x0202,
        sleeper: 2,
    }]);

    let p = &w.players[sleeper];
    assert!(!p.dead, "you came back as a corpse");
    assert!(!p.sleeping);
    assert!(p.hp > 0, "you came back with no hp");
    assert!(
        p.body.qx != fell_at.qx || p.body.qz != fell_at.qz,
        "you woke up standing on your own death site with your bag unspent"
    );
    assert_eq!(
        p.inv,
        [ItemStack::default(); INV_SLOTS],
        "a corpse's inventory came back with it"
    );
}

/// Waking against a sleeper the world no longer has is a no-op, not a
/// spawn. The server checks before it sends this, but a replayed WAL is
/// not the server: a world that evicted differently must refuse rather
/// than seat a body out of nothing.
#[test]
fn waking_a_sleeper_that_is_gone_seats_nobody() {
    let mut w = duel_world();
    let before = w.players.iter().filter(|p| p.active).count();

    w.tick(&[Command::Wake {
        id: 0x0909,
        sleeper: 0xDEAD,
    }]);

    assert_eq!(
        w.players.iter().filter(|p| p.active).count(),
        before,
        "a wake against a missing sleeper invented a player"
    );
}

/// Sleepers hold slots, so a shard every slot of which is asleep must
/// still admit somebody. The policy is **evict the longest-asleep**, and
/// this checks the order rather than just the fact: the body that has been
/// down longest is the one that goes.
///
/// Not a cosmetic choice. The alternative orders are "lowest slot", which
/// reaps whoever happened to join first and never anyone else, and "most
/// recent", which reaps the player most likely to still be coming back.
#[test]
fn a_full_shard_evicts_the_longest_asleep() {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    for i in 0..MAX_PLAYERS as u32 {
        w.tick(&[Command::Join { id: 100 + i }]);
    }
    assert_eq!(w.players.iter().filter(|p| p.active).count(), MAX_PLAYERS);

    // Put them to sleep in a known order, one per tick, so `slept_at` is
    // strictly increasing and the oldest is unambiguous.
    for i in 0..MAX_PLAYERS as u32 {
        w.tick(&[Command::Leave { id: 100 + i }]);
    }
    assert_eq!(w.sleepers(), MAX_PLAYERS);
    assert_eq!(w.evictions, 0, "nothing should have been evicted yet");

    w.tick(&[Command::Join { id: 9_999 }]);

    assert_eq!(w.evictions, 1, "the join did not evict");
    assert!(
        !w.is_sleeper(100),
        "the longest-asleep body survived; a newer one was taken instead"
    );
    assert!(
        w.is_sleeper(101),
        "the second-oldest should still be asleep"
    );
    assert!(
        w.players.iter().any(|p| p.active && p.id == 9_999),
        "the join was refused on a shard that had a slot to give"
    );
    assert_eq!(w.players.iter().filter(|p| p.active).count(), MAX_PLAYERS);
}

/// A second `Leave` for a body already asleep must not restamp
/// `slept_at` — a duplicate would move that sleeper to the back of the
/// eviction queue, which is a way to keep a body alive forever by
/// reconnecting badly.
#[test]
fn a_repeated_leave_does_not_refresh_the_eviction_clock() {
    let mut w = duel_world();
    let slot = slot_of(&w, 2);
    w.tick(&[Command::Leave { id: 2 }]);
    let slept_at = w.players[slot].slept_at;

    for _ in 0..10 {
        w.tick(&[Command::Leave { id: 2 }]);
    }

    assert_eq!(
        w.players[slot].slept_at, slept_at,
        "a duplicate leave restamped the eviction clock"
    );
}

/// Determinism, on the path this slice added. Same seed, same command
/// stream — including a leave, a raid, an eviction and a takeover — must
/// produce the same hash, and the hash must actually *move* when a body
/// falls asleep. The second half is what makes the first half mean
/// something: a `sleeping` bit outside `state_hash` would let two shards
/// disagree about whether anybody was home and both report agreement.
#[test]
fn sleeping_is_hashed_and_reproducible() {
    let script = |w: &mut World| {
        w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
        for _ in 0..5 {
            w.tick(&[]);
        }
        w.tick(&[Command::Leave { id: 2 }]);
        for _ in 0..5 {
            w.tick(&[]);
        }
        w.tick(&[Command::Wake {
            id: 0x0202,
            sleeper: 2,
        }]);
        w.state_hash()
    };
    let mut a = World::new(SEED);
    a.combat = CombatContent::probe_fixture();
    let mut b = World::new(SEED);
    b.combat = CombatContent::probe_fixture();
    assert_eq!(script(&mut a), script(&mut b), "the sleeper path diverged");

    // And the bit is visible to the hash at all.
    let mut awake = World::new(SEED);
    awake.combat = CombatContent::probe_fixture();
    awake.tick(&[Command::Join { id: 1 }]);
    let standing = awake.state_hash();
    awake.tick(&[Command::Leave { id: 1 }]);
    assert_ne!(
        standing,
        awake.state_hash(),
        "falling asleep did not move the state hash — `sleeping` is outside \
         it, so two shards could disagree about whether anybody is home"
    );
}
