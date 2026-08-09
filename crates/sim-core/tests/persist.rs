//! Persistence at the level the record itself cannot see: what a **restore
//! does to the world**.
//!
//! `crates/sim-core/src/persist.rs`'s own tests own the bytes (the round
//! trip, the golden, every refusal). This file owns the consequence, which
//! lives in `World::seat` and `craft::rearm` — and one property that is not
//! about persistence at all but about wall 5: a `JoinAs` in the command
//! stream must replay to the same hashes, because the whole reason the record
//! rides a `Command` instead of being read out of a file by the sim is that a
//! side-channel restore would replay as a fresh spawn.
//!
//! Nothing here invents a number: hp comes from `CombatContent::probe_fixture`
//! and the recipes from `CraftContent::probe_fixture`.

use sim_core::combat::CombatContent;
use sim_core::craft::{CraftContent, CraftJob};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::INV_SLOTS;
use sim_core::movement::{quant_xz, quant_y};
use sim_core::persist::PlayerSave;
use sim_core::survival::SurvivalContent;
use sim_core::terrain;
use sim_core::world::{Command, World};

const SEED: u64 = 20_260_731;
const ID: u32 = 1;
const REJOIN: u32 = 0x201;

/// A world with every table armed, so a restored body has hp to keep, meters
/// to keep and a recipe to be mid-craft on.
///
/// Boxed, for the reason `gather.rs`'s `world_at` is: a `World` is several
/// hundred KB, and by-value copies through test frames overflow the default
/// 2 MiB test-thread stack. This file is the one that proves it — the restore
/// assertions hold *two* worlds live at once (the played character's and the
/// one they rejoin), which is twice what any sibling suite asks for.
fn armed() -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.craft = CraftContent::probe_fixture();
    w.gather = GatherContent::probe_fixture();
    w
}

/// A world with the meters *disarmed*, for the assertions that have to be
/// exact. With the clock running, one tick of decay lands between the seat and
/// the first thing a test can observe, so an exact comparison of `food` would
/// be asserting the clock's rate rather than the restore. The clock's own
/// restore has its own test below.
fn armed_still() -> Box<World> {
    let mut w = armed();
    w.survival = SurvivalContent::EMPTY;
    w
}

/// A player who has been somewhere and done something: joined, walked to a
/// known point, and picked up a stack. Returns the world and their record.
fn a_played_character() -> (Box<World>, PlayerSave) {
    let mut w = armed_still();
    w.tick(&[Command::Join { id: ID }]);
    // Move the body to a spot the spawn ring would never pick, so "restored
    // in place" is distinguishable from "spawned again".
    let (x, z) = (1024.0f32, 1024.0f32);
    let p = &mut w.players[0];
    p.body.qx = quant_xz(x);
    p.body.qz = quant_xz(z);
    p.body.qy = quant_y(terrain::height(SEED, x, z));
    p.inv[0] = ItemStack { item: 1, count: 37 };
    p.inv[7] = ItemStack { item: 2, count: 4 };
    p.hp = 41;
    p.food = 300;
    p.food_acc = 12_345;
    let save = w.save_of(ID).expect("the player is in the world");
    (w, save)
}

/// The whole point of the slice, in one assertion set: what you had is what
/// you get back. Before this, `Join` built a fresh character every time and a
/// disconnect was a wipe of one.
#[test]
fn a_saved_character_comes_back_as_themselves() {
    let (_, save) = a_played_character();
    let mut w2 = armed_still();
    w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
    let p = w2.players[0];
    assert!(p.active, "a restore must seat a body");
    assert_eq!(p.id, REJOIN, "the id is the connection's, never the save's");
    assert_eq!(p.body, save.body, "restored somewhere else");
    assert_eq!(p.inv[0], ItemStack { item: 1, count: 37 });
    assert_eq!(p.inv[7], ItemStack { item: 2, count: 4 });
    assert_eq!(p.hp, 41, "you log off hurt, you log in hurt");
    assert_eq!(p.hp_max, save.hp_max);
    assert_eq!(p.food, 300);
    assert_eq!(p.food_acc, 12_345, "a zeroed accumulator is free food");
}

/// The survival clock resumes mid-span rather than restarting it.
///
/// The accumulators are the point: they are the exact-arithmetic remainder of
/// a rate, so a restore that zeroed them would hand back a fraction of a
/// minute of food on every reconnect — a small exploit *and* the drift wall 5
/// is about. Asserted as "not zero" rather than as an exact number because the
/// tick that seats the body also steps its clock, and pinning the rate here
/// would be testing `survival.rs` from the wrong file.
#[test]
fn the_survival_clock_resumes_where_it_stopped() {
    let mut w = armed();
    w.tick(&[Command::Join { id: ID }]);
    // Tick until BOTH accumulators hold a real remainder — a legal partial
    // state produced by the clock rather than written by hand. Searched rather
    // than counted, because each accumulator returns to zero on the tick its
    // meter drops a unit and the two spans do not line up: a fixed count
    // sometimes lands on one of those ticks and the test would fail on the
    // fixture's arithmetic rather than on anything about persistence.
    let mut save = w.save_of(ID).expect("there");
    for _ in 0..sim_core::limits::TICK_HZ {
        w.tick(&[]);
        save = w.save_of(ID).expect("there");
        if save.food_acc > 0 && save.water_acc > 0 {
            break;
        }
    }
    assert!(
        save.food_acc > 0 && save.water_acc > 0,
        "the fixture produced no partial span to restore"
    );
    // Stated as a difference rather than as a value, and that is the honest
    // shape: the tick that seats the body also steps its clock, so the
    // accumulator a test can observe is `(saved + rate) mod span` — a number
    // that is sometimes zero for a perfectly restored save. What cannot be
    // faked is the comparison: `tick_units` is injective inside a span, so two
    // restores that differ only in the saved remainder must land on different
    // remainders. If `seat` dropped the accumulators, both would be the
    // fresh-grant value and these would be equal.
    let restored = |save: PlayerSave| {
        let mut w = armed();
        w.tick(&[Command::JoinAs { id: REJOIN, save }]);
        w.players[0]
    };
    let resumed = restored(save);
    let restarted = restored(PlayerSave {
        food_acc: 0,
        water_acc: 0,
        ..save
    });
    assert_ne!(
        (resumed.food_acc, resumed.water_acc),
        (restarted.food_acc, restarted.water_acc),
        "the restore restarted the span instead of resuming it — a fraction of \
         a minute of food back on every reconnect"
    );
    // And the meters themselves arrive, within the one unit the seating tick's
    // own step may have drained. The exact value is pinned by
    // `a_saved_character_comes_back_as_themselves` against a still clock; here
    // the claim is only that a restored body is not handed a fresh pair.
    assert!(
        save.food - resumed.food <= 1 && save.water - resumed.water <= 1,
        "restored meters ({}, {}) are not the saved pair ({}, {})",
        resumed.food,
        resumed.water,
        save.food,
        save.water
    );
    assert!(
        resumed.food < w2_max_food(),
        "a restored body was granted a full meter"
    );
}

/// The fixture's food ceiling, so the test above can say "not a fresh grant"
/// without restating the number (`SurvivalContent::probe_fixture`).
fn w2_max_food() -> u16 {
    SurvivalContent::probe_fixture().max_food
}

/// A fresh join is untouched by any of this. The two commands share one
/// `seat`, so this is what keeps the shared path from having quietly become
/// the restore path.
#[test]
fn a_fresh_join_still_spawns_on_the_ring() {
    let mut w = armed();
    w.tick(&[Command::Join { id: ID }]);
    let p = w.players[0];
    let (rx, rz) = w.spawn_pos(ID);
    assert_eq!(p.body.qx, quant_xz(rx));
    assert_eq!(p.body.qz, quant_xz(rz));
    assert_eq!(p.hp, w.combat.player_hp, "a fresh body is whole");
    assert!(p.inv.iter().all(|s| s.count == 0), "naked spawn");
}

/// What the record deliberately does not carry (`persist.rs` says why for
/// each). Asserted because every one of them is a field somebody could add to
/// `PlayerSave` in good faith and break something quiet: a restored input
/// `seq` lies to prediction, and a restored `next_swing` from a dead clock
/// freezes the swing arm of a shard that restarted.
#[test]
fn a_restore_carries_no_client_state_and_no_stale_clock() {
    let (mut w, _) = a_played_character();
    w.players[0].next_swing = 9_999;
    w.players[0].frame.seq = 4_242;
    w.players[0].ws_hits = 5;
    let save = w.save_of(ID).expect("still there");

    let mut w2 = armed();
    w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
    let p = w2.players[0];
    assert_eq!(
        p.frame.seq, 0,
        "the input frame is the client's, not the save's"
    );
    assert_eq!(p.next_swing, 0, "a cooldown from a dead clock came back");
    assert_eq!(
        p.ws_hits, 0,
        "the weak-spot chase belongs to an old encounter"
    );
}

/// The craft queue survives a restart and its timer is re-armed against the
/// *current* tick. Both halves matter: dropping the queue would eat the
/// materials the enqueue already spent, and restoring the absolute
/// `craft_done_at` would arm it against a clock that no longer exists — on a
/// restarted shard (tick 0) that means a job that completes instantly, or one
/// that never does.
#[test]
fn a_craft_queue_survives_and_rearms_against_the_new_clock() {
    let mut w = armed_still();
    w.tick(&[Command::Join { id: ID }]);
    // Enough input for the fixture's row 0 (3 of item 0 per unit).
    w.players[0].inv[0] = ItemStack { item: 0, count: 90 };
    w.tick(&[Command::Craft {
        id: ID,
        recipe: 0,
        count: 5,
    }]);
    // Let the queue run a little, so the save is taken mid-batch against a
    // clock well past zero — but not so long that the batch finishes (the
    // fixture's row 0 is 2 ticks a unit, 5 units).
    for _ in 0..6 {
        w.tick(&[]);
    }
    let save = w.save_of(ID).expect("still there");
    assert!(
        save.jobs[0].remaining > 0,
        "the fixture must still be crafting"
    );
    assert!(w.tick > 5, "the source clock must not be near zero");

    // A shard that has been up a while, and one that has just started: both
    // must arm the head job relative to their own clock.
    for warm in [0u32, 97] {
        let mut w2 = armed_still();
        for _ in 0..warm {
            w2.tick(&[]);
        }
        // The tick a command is applied *on*, which is what `craft::enqueue`
        // arms against too — `World::tick` advances the counter after the
        // commands run, so this is the base and `w2.tick` afterwards is not.
        let base = w2.tick;
        w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
        let p = w2.players[0];
        assert_eq!(p.jobs, save.jobs, "the queue itself must survive");
        let unit = w2.craft.recipes[save.jobs[0].recipe as usize].ticks as u64;
        assert_eq!(
            p.craft_done_at,
            base + unit,
            "the head job must be armed one unit from now, not from a dead clock"
        );
        // And it actually completes: the point of re-arming is a queue that
        // runs, not a field with a plausible value in it.
        let before = p.inv.iter().filter(|s| s.item == 2).count();
        for _ in 0..=unit {
            w2.tick(&[]);
        }
        assert!(
            w2.players[0].inv.iter().filter(|s| s.item == 2).count() >= before,
            "the restored queue paid nothing out"
        );
        assert!(
            w2.players[0].jobs[0].remaining < save.jobs[0].remaining,
            "the restored queue did not advance"
        );
    }
}

/// A body that logged off dead comes back **standing on a beach**, not alive
/// at its own corpse.
///
/// This is the exploit the `dead` bit exists to close: a corpse's saved
/// position is where it fell, so restoring it in place would stand the player
/// up alive at their death site with their sleeping bag still unspent —
/// strictly better than staying to press the button. Declining the choice is
/// choosing the beach.
#[test]
fn a_body_that_logged_off_dead_wakes_on_a_beach() {
    let mut w = armed();
    w.tick(&[Command::Join { id: ID }]);
    let (x, z) = (1024.0f32, 1024.0f32);
    w.players[0].body.qx = quant_xz(x);
    w.players[0].body.qz = quant_xz(z);
    w.players[0].inv[0] = ItemStack { item: 1, count: 9 };
    // A real death by a real cause — the survival clock, whose fixture spans
    // are seconds so a whole death fits in a test — and then the screen is
    // deliberately left unanswered. Constructing a record with `dead: true`
    // by hand would test the restore and not the *save*, and whether
    // `PlayerSave::of` notices a corpse at all is half of what is at stake.
    let mut fell = false;
    for _ in 0..60 * sim_core::limits::TICK_HZ {
        w.tick(&[]);
        if w.players[0].dead {
            fell = true;
            break;
        }
    }
    assert!(
        fell,
        "the clock did not kill the body — setup failed, not a skip"
    );
    let save = w.save_of(ID).expect("a corpse keeps its slot");
    assert!(save.dead, "the record must know the body was down");

    let mut w2 = armed();
    w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
    let p = w2.players[0];
    assert!(!p.dead, "a corpse must not come back as a corpse");
    assert_eq!(p.hp, w2.combat.player_hp, "a wake is a whole body");
    assert!(
        p.inv.iter().all(|s| s.count == 0),
        "the loot went into the bag"
    );
    let (rx, rz) = w2.spawn_pos_n(REJOIN, save.deaths as u32);
    assert_eq!(
        (p.body.qx, p.body.qz),
        (quant_xz(rx), quant_xz(rz)),
        "a dead save must wake on the ring at its death count, not at its corpse"
    );
    // Compared in quanta, not metres: `f32::abs` is a disallowed method in this
    // crate (wall 1's float set), and the quantized ints are the exact values
    // anyway — which is the same reason `Body` can derive `Eq`.
    assert!(
        (p.body.qx - save.body.qx).abs() > quant_xz(1.0),
        "the wake landed on the corpse"
    );
}

/// **Wall 5, for the restore path.** Same build, same seed, same command
/// stream — including the `JoinAs` and its record — must give the same state
/// hashes, tick for tick.
///
/// This is the assertion that justifies the `Command` carrying 188 bytes
/// rather than the server writing the record into the world directly: with a
/// side channel this stream would replay as a fresh spawn and every hash
/// after it would differ, silently, because a fresh spawn is a perfectly
/// legal `Player`.
#[test]
fn a_restore_replays_bit_for_bit() {
    let (_, save) = a_played_character();
    let stream = |w: &mut World| -> Vec<u64> {
        let mut hashes = Vec::new();
        w.tick(&[Command::JoinAs { id: REJOIN, save }]);
        hashes.push(w.state_hash());
        for t in 0..40u32 {
            // A little input so the body is doing something between hashes.
            w.tick(&[Command::Input {
                id: REJOIN,
                frame: sim_core::input::InputFrame {
                    seq: t as u16 + 1,
                    move_z: 90,
                    ..Default::default()
                },
            }]);
            hashes.push(w.state_hash());
        }
        hashes
    };
    let a = stream(&mut armed());
    let b = stream(&mut armed());
    assert_eq!(a, b, "a restored session did not replay identically");
    // And the restore is not a no-op the comparison could be satisfied by: a
    // fresh join over the same stream must produce a *different* history.
    let fresh = {
        let mut w = armed();
        let mut hashes = Vec::new();
        w.tick(&[Command::Join { id: REJOIN }]);
        hashes.push(w.state_hash());
        for t in 0..40u32 {
            w.tick(&[Command::Input {
                id: REJOIN,
                frame: sim_core::input::InputFrame {
                    seq: t as u16 + 1,
                    move_z: 90,
                    ..Default::default()
                },
            }]);
            hashes.push(w.state_hash());
        }
        hashes
    };
    assert_ne!(
        a, fresh,
        "the restore changed nothing about the world, so this gate proves nothing"
    );
}

/// Taking a save must not change the world. The server reads one at a leave
/// and once per tick on the autosave sweep, and neither is a command — so a
/// `save_of` that mutated anything would put a change in the world that the
/// WAL never saw, which is the exact failure wall 5 is about.
#[test]
fn taking_a_save_changes_nothing() {
    let (w, _) = a_played_character();
    let before = w.state_hash();
    // Against what the last tick left behind, not against zero: the events
    // ring is cleared at tick start and holds that tick's output, so "empty"
    // would be an accident of the fixture rather than the claim.
    let events_before = w.events.len();
    for _ in 0..200 {
        let _ = w.save_of(ID);
    }
    assert_eq!(w.state_hash(), before, "reading a save moved the world");
    assert_eq!(
        w.events.len(),
        events_before,
        "reading a save announced something"
    );
}

/// Nobody by that id, no record — and a corpse still has one, because the
/// death screen keeps its slot.
///
/// **A sleeper has one too, and that is the answer changing rather than the
/// rule.** The rule was always "a record for a body the world has"; a leave
/// used to end the body, and since sleepers it does not. Nothing reads this
/// yet for a sleeper — `ShardCore::disconnect` takes its record *before*
/// queueing the `Leave`, while the body is still awake, and the autosave
/// sweep walks connection slots, which a sleeper does not occupy. So the
/// store's copy of a sleeper is frozen at the moment they left, and a
/// sleeper that starves or is raided before being evicted would come back
/// from the stale one. That is the gap `NOW.md` §0y items 2–3 close, and
/// this assertion is the half of the machinery that is already in place for
/// it.
#[test]
fn save_of_answers_for_any_body_the_world_still_has() {
    let mut w = armed();
    assert_eq!(w.save_of(ID), None, "an empty world remembers nobody");
    w.tick(&[Command::Join { id: ID }]);
    assert!(w.save_of(ID).is_some());
    assert_eq!(w.save_of(ID + 1), None, "a stranger's id must miss");
    w.tick(&[Command::Leave { id: ID }]);
    assert!(
        w.save_of(ID).is_some(),
        "a sleeping body is still a body the world has"
    );
}

/// A `JoinAs` for an id already in the world is ignored, exactly as a `Join`
/// is. Otherwise a duplicated command — a WAL replayed twice over one tick,
/// a control ring retried — would overwrite a live body with an old record.
#[test]
fn a_second_join_as_for_a_live_id_is_ignored() {
    let mut w = armed();
    w.tick(&[Command::Join { id: ID }]);
    w.players[0].inv[0] = ItemStack { item: 3, count: 8 };
    let hash = w.state_hash();
    let mut stale = PlayerSave::EMPTY;
    stale.hp = 1;
    stale.hp_max = 1;
    stale.inv[0] = ItemStack { item: 1, count: 1 };
    stale.jobs[0] = CraftJob {
        recipe: 0,
        remaining: 1,
    };
    w.tick(&[Command::JoinAs {
        id: ID,
        save: stale,
    }]);
    // The tick advanced, so compare the body rather than the whole hash.
    assert_eq!(
        w.players[0].inv[0],
        ItemStack { item: 3, count: 8 },
        "a restore overwrote a live body"
    );
    assert_ne!(w.players[0].hp, 1, "a restore overwrote live hp");
    assert!(hash != 0, "the fixture produced no hash to compare against");
    assert_eq!(
        w.players.iter().filter(|p| p.active).count(),
        1,
        "a duplicate join seated a second body"
    );
}

/// Every inventory slot survives, not just the first few. Walked because the
/// codec writes 30 stacks in a loop and an off-by-one at either end is the
/// classic version of this bug.
#[test]
fn every_inventory_slot_survives_the_trip() {
    let mut w = armed();
    w.tick(&[Command::Join { id: ID }]);
    for (i, s) in w.players[0].inv.iter_mut().enumerate() {
        *s = ItemStack {
            item: (i % 8) as u16 + 1,
            count: i as u16 + 1,
        };
    }
    let want = w.players[0].inv;
    let save = w.save_of(ID).expect("there");
    let mut w2 = armed();
    w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
    for (slot, expect) in want.iter().enumerate() {
        assert_eq!(&w2.players[0].inv[slot], expect, "slot {slot} drifted");
    }
    assert_eq!(
        want.len(),
        INV_SLOTS,
        "the walk did not cover the inventory"
    );
}
