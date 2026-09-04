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
    p.inv[0] = ItemStack {
        item: 1,
        count: 37,
        cond: 0,
    };
    p.inv[7] = ItemStack {
        item: 2,
        count: 4,
        cond: 0,
    };
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
    assert_eq!(
        p.inv[0],
        ItemStack {
            item: 1,
            count: 37,
            cond: 0,
        }
    );
    assert_eq!(
        p.inv[7],
        ItemStack {
            item: 2,
            count: 4,
            cond: 0,
        }
    );
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
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 90,
        cond: 0,
    };
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
    w.players[0].inv[0] = ItemStack {
        item: 1,
        count: 9,
        cond: 0,
    };
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

/// A two-entry kit of stand-in item ids, in `SpawnKit`'s own shape —
/// `bag_respawn.rs`'s probe kit, duplicated rather than shared because a
/// tests/ helper crate is a bigger structure than two functions deserve.
/// Ids 8 and 9 are outside every table this file arms, so nothing else here
/// can pay them into a pocket and make a re-grant look like a restore.
fn probe_kit() -> sim_core::inventory::SpawnKit {
    let mut kit = sim_core::inventory::SpawnKit::EMPTY;
    assert!(
        kit.set(
            0,
            ItemStack {
                item: 8,
                count: 1,
                cond: 0
            }
        ),
        "kit slot 0"
    );
    assert!(
        kit.set(
            1,
            ItemStack {
                item: 9,
                count: 1,
                cond: 0
            }
        ),
        "kit slot 1"
    );
    kit
}

/// **The second of `wake`'s three doors is paid too** (`NOW.md` §0kit
/// remainder 1). A save marked `dead` falls through `seat`'s restore arm
/// into `wake`, and `wake` is where a new body is built — so a player who
/// logged off on the death screen logs back in holding the spawn kit, not
/// naked. The test above proves that door wakes the body *somewhere*; with
/// `spawn_kit` EMPTY there, nothing proved the body woke *armed*, and a
/// refactor that reimplemented the beach-wake inline without `grant_kit`
/// stayed green on every suite.
///
/// Proven red 2026-08-17, both ways. Early-returning before the wake in
/// `seat`'s dead arm (`if s.dead { return; }` in place of the `wake` call)
/// fails this on slot 0 reading empty ("woke naked") — beside the beach
/// gate above, which catches that skip on its whole-body hp assertion.
/// Deleting only `inventory::grant_kit` from `wake` fails this on the same
/// line while every pre-existing gate in this suite stays green — that
/// narrower hole, a wake that stands the body up and forgets to arm it, is
/// the coverage this test adds.
#[test]
fn a_dead_save_wakes_holding_the_spawn_kit() {
    let mut w = armed();
    w.spawn_kit = probe_kit();
    w.tick(&[Command::Join { id: ID }]);
    // The fresh arm granted it — otherwise the assertion after the restore
    // could pass on a door that grants nothing anywhere.
    assert_eq!(
        w.players[0].inv[0].item, 8,
        "the fresh spawn did not get the kit — nothing below proves anything"
    );
    // Something the player EARNED, in a slot the kit does not write. The
    // death must take it, or the kit assertion below would be reading a
    // survived inventory rather than a grant.
    w.players[0].inv[6] = ItemStack {
        item: 2,
        count: 5,
        cond: 0,
    };

    // A real death by a real cause, screen left unanswered — the same
    // fixture as the beach test above, because the door under test is the
    // same one: `PlayerSave::of` must see the corpse.
    w.players[0].food = 0;
    w.players[0].water = 0;
    let mut fell = false;
    for _ in 0..120 * sim_core::limits::TICK_HZ {
        w.tick(&[]);
        if w.players[0].dead {
            fell = true;
            break;
        }
    }
    assert!(fell, "the clock did not kill the body — setup failed");
    let save = w.save_of(ID).expect("a corpse keeps its slot");
    assert!(save.dead, "the record must know the body was down");

    let mut w2 = armed();
    w2.spawn_kit = probe_kit();
    w2.tick(&[Command::JoinAs { id: REJOIN, save }]);
    let p = w2.players[0];
    assert!(!p.dead, "a corpse must not come back as a corpse");
    assert_eq!(
        (p.inv[0], p.inv[1]),
        (
            ItemStack {
                item: 8,
                count: 1,
                cond: 0
            },
            ItemStack {
                item: 9,
                count: 1,
                cond: 0
            }
        ),
        "a body that logged off dead woke naked — this door skipped the kit"
    );
    assert_eq!(
        p.inv[6],
        ItemStack::default(),
        "the earned stack survived the death, so the slots above may be a \
         restored inventory rather than a grant — this test proves nothing"
    );
    assert_eq!(p.hp, w2.combat.player_hp, "a wake is a whole body");
    assert!(
        p.food > 0 && p.water > 0,
        "a body that starved to death respawned already starving"
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
                favour: 0,
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
                favour: 0,
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
    w.players[0].inv[0] = ItemStack {
        item: 3,
        count: 8,
        cond: 0,
    };
    let hash = w.state_hash();
    let mut stale = PlayerSave::EMPTY;
    stale.hp = 1;
    stale.hp_max = 1;
    stale.inv[0] = ItemStack {
        item: 1,
        count: 1,
        cond: 0,
    };
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
        ItemStack {
            item: 3,
            count: 8,
            cond: 0,
        },
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
            cond: 0,
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

// ---------------------------------------------------------------------
// What a body carries through a death — stated, not inferred.
// ---------------------------------------------------------------------

/// `world.rs` builds a `Player` from scratch in four places — `seat`'s two
/// arms, `die` and `wake` — and three of them end in `..Player::default()`.
/// That spread is a **silent answer**: a field added to `Player` is
/// defaulted at each of those doors without anybody deciding it should be,
/// and the code reads correct because it compiles and produces a legal
/// `Player`.
///
/// It has cost the tree once already. `known` — the blueprint mask, bought
/// with the scarcest currency on the shard — landed at research v0 into
/// `Player`, `Default`, `PlayerSave`, `state_hash` and `worldsave.rs`, and
/// four doors quietly answered "no, a body does not keep that". Dying
/// deleted every blueprint you owned; so did reconnecting after a restart.
/// Every gate was green: `test_replay`'s stream contains no `JoinAs`,
/// `state_hash` agreed with itself because both sides zeroed the same
/// field, and the codec's own round-trip test never seated a world.
///
/// So this is the ledger for it. Every field of `Player` is classified
/// **carried** or **re-derived**, the classification is checked against the
/// struct as `world.rs` actually declares it, and each carried field must
/// be named in both `die`'s and `wake`'s literal. A new field on `Player`
/// therefore cannot land without someone writing down what a death does to
/// it — which is the decision the spread was making on its own.
///
/// `worldsave.rs` solved the same problem the other way, by naming every
/// field so the compiler refuses the omission, and `seat`'s restore arm
/// does now too. `die` and `wake` cannot: both deliberately reset most of
/// the record, so the spread is the right shape there and a ledger is what
/// is left.
mod carried_through_death {
    /// Named in `die`'s literal **and** `wake`'s, because both rebuild the
    /// record and a field carried by only one is lost anyway.
    ///
    /// `id`, `active` and `body`/`frame` are here as mechanics — a body
    /// keeps its identity through a death. The two that are *decisions*:
    /// `deaths` is the spawn-ring cursor, so a respawn must not send
    /// everyone back to the same beach; `known` is the blueprint mask, and
    /// it is here because a blueprint is a thing the player *did*, not a
    /// thing they were holding when they fell. The corpse's inventory is
    /// the backpack's business and is correctly on the other list.
    pub const CARRIED: [&str; 5] = ["id", "active", "frame", "deaths", "known"];

    /// Re-derived, dropped, or owned by the death itself. Anything here is
    /// a field a death is *allowed* to erase — the inventory (the backpack
    /// takes it), the meters and health (a respawn is a whole body), the
    /// craft queue, the weak-spot chase, and the death record itself.
    pub const RE_DERIVED: [&str; 29] = [
        "body",
        "inv",
        // **The corpse does not keep its plates.** `worn` is here rather
        // than on `CARRIED` because a death is where armor becomes loot:
        // `World::die` sheds it into the death bag through `drain_spill`
        // (the same drain the six spill producers use), so a killer gets
        // the burlap shirt that just cost them two extra swings. A body
        // that kept its armor through a death would make killing an
        // armored player pay less than killing a naked one, which is the
        // wrong way round.
        "worn",
        "next_swing",
        // **A corpse's cylinder does not follow it to the beach.** `worn`'s
        // argument one step on: the gun itself is on the other list by
        // being inside `inv`, so it goes to the death bag, and a magazine
        // that stayed with the *player* would mean a killer looted an
        // empty revolver while the dead player respawned holding its
        // rounds. `..Player::default()` is what clears it, and it is
        // correct by construction — `Player::default` writes
        // `[NO_ITEM; MAX_MAGS]` into `mag_round`, so an emptied magazine
        // remembers no round rather than naming item 0.
        //
        // **And the rounds are no longer destroyed on the way** — that
        // cost stood here until 2026-08-30 and read "up to a magazine's
        // worth per death". `die` now sheds `mag`/`mag_round` into the
        // same `shed` buffer as `worn`, as an ordinary `ItemStack` built
        // from the pair the arrays already hold, so the field stays on
        // this list (the *player* keeps nothing) while the ammunition
        // reaches the killer. `tests/reload.rs`'s
        // `a_death_empties_the_cylinder` holds both halves.
        "mag",
        "mag_round",
        "ws_cell",
        "ws_hits",
        "jobs",
        "craft_done_at",
        "hp",
        "hp_max",
        "food",
        "water",
        "food_acc",
        "water_acc",
        "hurt_acc",
        "heal_rem",
        "heal_total",
        "heal_span",
        "heal_acc",
        "dead",
        "death_by",
        "death_cause",
        "death_item",
        "death_range_cm",
        "sleeping",
        "slept_at",
        // **A corpse is not holding a torch up.** `light_acc` is the
        // sub-point remainder of a flame, and the flame is derived from a
        // held stack the backpack has just taken (`inv`, four lines up),
        // so carrying the remainder would be keeping change for a purchase
        // somebody else now owns. Six seconds of light, and the decision
        // is about where it lives rather than what it is worth: a
        // remainder without the item it was burning is state nothing can
        // spend.
        "light_acc",
    ];
}

const WORLD_SRC: &str = include_str!("../src/world.rs");

/// Every field name declared by `pub struct Player`, in order.
///
/// Parsed from source rather than imported for `event_roles.rs`'s reason:
/// the fact under assertion is *what the struct contains*, and no import
/// can see a field the importer was never told about — which is precisely
/// the field this ledger exists to catch.
fn player_fields() -> Vec<&'static str> {
    let start = WORLD_SRC
        .find("pub struct Player {")
        .expect("world.rs no longer declares `pub struct Player`");
    let body = &WORLD_SRC[start..];
    let end = body
        .find("\n}\n")
        .expect("the `Player` declaration is unterminated");
    let mut out = Vec::new();
    for line in body[..end].lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub ") else {
            continue; // a doc comment, a blank, or an attribute
        };
        let Some((name, _)) = rest.split_once(':') else {
            continue;
        };
        if name.contains(' ') || name.is_empty() {
            continue;
        }
        out.push(name);
    }
    out
}

/// The body of a `fn <name>(&mut self…` in `world.rs`, to its four-space
/// closing brace.
fn fn_body(name: &str) -> &'static str {
    let needle_owned = ["    fn ", name, "(&mut self"].concat();
    let start = WORLD_SRC
        .find(&needle_owned)
        .unwrap_or_else(|| panic!("world.rs no longer declares `fn {name}(&mut self…`"));
    let body = &WORLD_SRC[start..];
    let end = body
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("`fn {name}` is unterminated"));
    &body[..end]
}

/// Whether `src` names `field` in a struct literal — `field: expr` or the
/// `field` shorthand. Deliberately not a substring test: `known` occurs
/// inside `unknown`, and `hp` inside `hp_max`.
fn names_field(src: &str, field: &str) -> bool {
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(field) else {
            continue;
        };
        if rest.starts_with(':') || rest == "," {
            return true;
        }
    }
    false
}

/// The ledger adds up, matches the struct, and every carried field is
/// actually named at both doors.
#[test]
fn every_player_field_is_classified_across_a_death() {
    use carried_through_death::{CARRIED, RE_DERIVED};

    let declared = player_fields();
    assert!(
        declared.len() > 20,
        "the `Player` field parser found only {} fields — the struct's \
         shape changed and this ledger is now scanning nothing",
        declared.len()
    );
    assert_eq!(
        CARRIED.len() + RE_DERIVED.len(),
        declared.len(),
        "the ledger classifies {} fields but `Player` declares {} — a \
         field landed without anyone deciding what a death does to it. \
         Declared: {declared:?}",
        CARRIED.len() + RE_DERIVED.len(),
        declared.len()
    );

    for field in &declared {
        let n = CARRIED.iter().filter(|f| f == &field).count()
            + RE_DERIVED.iter().filter(|f| f == &field).count();
        assert_eq!(
            n, 1,
            "`Player::{field}` is classified {n} times — it is either new \
             and unclassified, or listed as both carried and re-derived"
        );
    }
    for field in CARRIED.iter().chain(RE_DERIVED.iter()) {
        assert!(
            declared.contains(field),
            "the ledger classifies `{field}`, which `Player` does not \
             declare — a field was renamed or removed and the ledger \
             stayed behind, so it is now guarding nothing"
        );
    }

    // The claim is earned at both doors, or it is arithmetic.
    let die = fn_body("die");
    let wake = fn_body("wake");
    for field in CARRIED.iter() {
        assert!(
            names_field(die, field),
            "`{field}` is on the carried list but `World::die` does not \
             name it, so `..Player::default()` clears it at the instant of \
             death — which is exactly how `known` was lost"
        );
        assert!(
            names_field(wake, field),
            "`{field}` is on the carried list but `World::wake` does not \
             name it, so a respawn hands back a default"
        );
    }
}

/// And the ledger is not merely a spelling check: the two fields that are
/// *decisions* rather than mechanics survive a real death and a real
/// respawn, driven through `die` and `wake` themselves.
#[test]
fn the_carried_decisions_survive_a_real_death() {
    const MASK: u64 = 1 << 3 | 1 << 40;
    let mut w = armed();
    w.tick(&[Command::Join { id: ID }]);
    w.players[0].known = MASK;
    let deaths_before = w.players[0].deaths;

    // A real death: hp to zero through the same door combat uses.
    w.players[0].food = 0;
    w.players[0].water = 0;
    let mut died = false;
    for _ in 0..120 * sim_core::limits::TICK_HZ {
        w.tick(&[]);
        if w.players[0].deaths > deaths_before {
            died = true;
            break;
        }
    }
    assert!(died, "the clock never killed the body");
    assert_eq!(
        w.players[0].known, MASK,
        "`die` erased the blueprint mask at the instant of death"
    );

    w.tick(&[Command::Respawn {
        id: ID,
        on_bag: false,
    }]);
    assert!(!w.players[0].dead, "the respawn did not stand the body up");
    assert_eq!(
        w.players[0].known, MASK,
        "`wake` erased the blueprint mask at the respawn"
    );
    assert_eq!(
        w.players[0].deaths,
        deaths_before + 1,
        "the spawn-ring cursor did not survive either, so every death \
         sends the body back to the same beach"
    );
}

/// **A saved tool comes back worn** (item durability v0, gate 7's player
/// half; `SAVE_FORMAT` 3). Condition is per-stack state and the record
/// carries it, so a hatchet at 41.5 points logs back in at 41.5 points —
/// a restore that minted it whole would be a free repair bench on the
/// login screen, and one that zeroed it would hand back a dead tool.
/// Proven red by reverting `write_le`/`read_le`'s slot stride to the
/// four-byte form (cond reads 0 and the worn assert fires).
#[test]
fn a_saved_tool_comes_back_worn() {
    let mut w = armed_still();
    w.tick(&[Command::Join { id: ID }]);
    w.players[0].inv[0] = ItemStack {
        item: 1,
        count: 1,
        cond: 4_150,
    };
    let save = PlayerSave::of(&w.players[0]);

    // Through the bytes, not just the struct: the record IS the format.
    let mut buf = [0u8; sim_core::persist::PLAYER_SAVE_BYTES];
    save.write_le(&mut buf);
    let back = PlayerSave::read_le(&buf).expect("a legal record decodes");
    assert_eq!(
        back.inv[0].cond, 4_150,
        "the record dropped the condition on the way through the file"
    );

    // And through the seat: the restored body holds the worn tool.
    let mut w2 = armed_still();
    w2.tick(&[Command::JoinAs {
        id: REJOIN,
        save: back,
    }]);
    let p = w2
        .players
        .iter()
        .find(|p| p.active && p.id == REJOIN)
        .expect("the body seated");
    assert_eq!(
        p.inv[0],
        ItemStack {
            item: 1,
            count: 1,
            cond: 4_150,
        },
        "a saved tool must come back exactly as worn as it left"
    );
}

/// Eating a stack to zero left `NO_ITEM` (65535) in the slot — a value
/// `read_le` refuses twice over (`item >= MAX_ITEM_DEFS`, and the
/// canonical-empty rule) — so the next autosave of that player wrote a
/// record the next boot could not read back. Pre-existing (the one
/// `NO_ITEM` writer in the tree; every other emptied slot is
/// `ItemStack::default()`), found by the durability slice's review.
#[test]
fn an_eaten_empty_slot_still_saves_and_loads() {
    use sim_core::persist::PLAYER_SAVE_BYTES;
    use sim_core::world::{EventQueue, Player};

    let sc = SurvivalContent::probe_fixture();
    let mut ev = EventQueue::default();
    let mut p = Player::default();
    p.inv[0] = ItemStack {
        item: 0,
        count: 1,
        cond: 0,
    };
    assert!(
        sim_core::survival::consume(&sc, 0, &mut p, &mut ev),
        "the fixture arms item 0 as food and the default meters are hungry"
    );
    assert_eq!(
        p.inv[0],
        ItemStack::default(),
        "eaten-empty is the canonical empty, all three fields"
    );

    let save = PlayerSave::of(&p);
    let mut buf = [0u8; PLAYER_SAVE_BYTES];
    save.write_le(&mut buf);
    PlayerSave::read_le(&buf).expect("an eaten-empty slot must load back");
}
