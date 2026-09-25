//! Wounded v0 — a lethal blow lays the body down instead of killing it, and
//! the minute that follows (`sim-core/src/wound.rs`, `reference/WOUNDED.md`
//! §9). What this suite holds:
//!
//! 1. the fall — a lethal swing makes a crawl, not a corpse, and the feed
//!    hears nothing;
//! 2. the finish — a blow on a downed body is the death, credited to the
//!    hand that struck it;
//! 3. the roll — at `wound_until` the body gets up or dies, and which is
//!    exactly what `wound::recovers` says for that tick (wall 5: the die is
//!    a function of the seed);
//! 4. the minute — a second lethal blow inside `REWOUND_TICKS` of getting up
//!    is a death outright, and one after it is a fall again;
//! 5. the crawl — a downed body moves at a third of the walk and its swing
//!    button swings nothing;
//! 6. the doors — `live_slot_of` refuses a downed body, `awake_slot_of`
//!    keeps it;
//! 7. the sleeper — a lethal blow on a sleeper is a death, and a downed body
//!    whose owner leaves still faces the roll;
//! 8. the hash — `wounded` is sim state and `state_hash` says so;
//! 9. the knobs — the window is the reference's minute at this tick rate.
//!
//! The fixture is `event_roles.rs`'s duel, copied rather than shared: each
//! suite owns one `World` (the stack argument in that file's `duel_world`).

use sim_core::backpack::BackpackContent;
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, TICK_HZ};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q, WALK_SPEED};
use sim_core::world::{
    Command, SimEvent, World, DEATH_BY_HAND, EV_CONSUMED, EV_DEATH, EV_HEALTH, EV_HIT,
    EV_RECOVERED, EV_SWING, EV_WOUNDED,
};
use sim_core::wound::{
    crawl_frame, recover_chance_pm, recovers, CRAWL_DIV, REWOUND_TICKS, WOUNDED_HP,
    WOUND_MAX_TICKS, WOUND_MIN_TICKS,
};
use sim_core::yaw_dir;

const SEED: u64 = 20260913;
const ATTACKER: u32 = 1;
const VICTIM: u32 = 2;
const SPEAR: u16 = 0;
const YAW: u16 = 0;
const REACH_M: f32 = 1.0;
const MAX_STEPS: u32 = 600;
const EYE_M: f32 = sim_core::ranged::ARROW_EYE_MM as f32 / 1000.0;
const CHEST_M: f32 = 1.0;

fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
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

/// Two players a spear's length apart, the attacker armed, the victim on
/// its last hp so the first landed blow is the lethal one.
fn duel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(ATTACKER));
    w.tick(&[Command::Join { id: ATTACKER }, Command::Join { id: VICTIM }]);
    w.players[0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    let (fx, fz) = yaw_dir(YAW);
    let a = w.players[0].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[1].body = Body::at(SEED, hv(SEED), ax + fx * REACH_M, az + fz * REACH_M);
    w.players[1].hp = 1;
    w
}

fn planar_m(w: &World, a: usize, b: usize) -> f32 {
    let (pa, pb) = (w.players[a].body, w.players[b].body);
    let (dx, dz) = (
        (pb.qx - pa.qx) as f32 * POS_XZ_Q,
        (pb.qz - pa.qz) as f32 * POS_XZ_Q,
    );
    (dx * dx + dz * dz).sqrt()
}

fn aim_at(w: &World, attacker: usize, victim: usize) -> u8 {
    let a = w.players[attacker].body;
    let target = &w.players[victim].body;
    let (dx, dz) = (
        (target.qx - a.qx) as f32 * POS_XZ_Q,
        (target.qz - a.qz) as f32 * POS_XZ_Q,
    );
    let run = (dx * dx + dz * dz).sqrt();
    let rise = (target.qy - a.qy) as f32 * POS_Y_Q + CHEST_M - EYE_M;
    let len = (rise * rise + run * run).sqrt();
    let mut best = 128u8;
    let mut best_dot = f32::MIN;
    for b in 0..=255u8 {
        let (ch, sv) = sim_core::pitch_dir(b);
        let dot = (ch * run + sv * rise) / len;
        if dot > best_dot {
            best_dot = dot;
            best = b;
        }
    }
    best
}

/// One tick with the attacker's swing held at the victim's chest.
fn swing(w: &mut World, seq: &mut u16) {
    let pitch = if w.players[1].active && planar_m(w, 0, 1) <= 3.0 {
        aim_at(w, 0, 1)
    } else {
        128
    };
    w.tick(&[Command::Input {
        id: ATTACKER,
        frame: InputFrame {
            seq: *seq,
            buttons: BTN_PRIMARY,
            yaw: YAW,
            pitch,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
        favour: 0,
    }]);
    *seq = seq.wrapping_add(1);
}

fn count(w: &World, code: u8) -> u32 {
    w.events.entries().iter().filter(|e| e.code == code).count() as u32
}

fn only(w: &World, code: u8) -> SimEvent {
    let n = count(w, code);
    assert_eq!(
        n, 1,
        "expected exactly one event code {code} on this tick, saw {n}"
    );
    *w.events.entries().iter().find(|e| e.code == code).unwrap()
}

fn swing_until(w: &mut World, seq: &mut u16, code: u8) {
    for _ in 0..MAX_STEPS {
        swing(w, seq);
        if count(w, code) > 0 {
            return;
        }
    }
    panic!("event code {code} never landed in {MAX_STEPS} swinging ticks");
}

/// The attacker stops swinging: a bare `tick(&[])` re-applies the stored
/// frame, so the stored frame has to be emptied for the victim's minute to
/// run untouched.
fn stand_down(w: &mut World) {
    w.players[0].frame = InputFrame::default();
}

fn chance_of(w: &World, slot: usize) -> u32 {
    let p = &w.players[slot];
    recover_chance_pm(p.food, p.water, w.survival.max_food, w.survival.max_water)
}

// ---------------------------------------------------------------------------

/// Give the victim one stack, so a bag is worth standing up if the body
/// dies — and so "no bag" after a fall is a claim and not a vacuity.
fn arm_victim(w: &mut World) {
    w.players[1].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
}

#[test]
fn a_lethal_swing_lays_the_body_down_instead_of_killing_it() {
    let mut w = duel_world();
    arm_victim(&mut w);
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    let v = &w.players[1];
    assert!(v.wounded, "the body is down");
    assert!(!v.dead, "and it is not a corpse");
    assert!(v.active);
    assert_eq!(v.hp, WOUNDED_HP, "it has the crawl's hp");
    assert_eq!(v.deaths, 0, "nobody died");
    assert_eq!(v.death_by, ATTACKER, "the downing blow is remembered: who");
    assert_eq!(v.death_cause, DEATH_BY_HAND, "how");
    assert_eq!(v.death_item, SPEAR, "with what");
    let span = v.wound_until - (w.tick - 1);
    assert!(
        (WOUND_MIN_TICKS as u64..=WOUND_MAX_TICKS as u64).contains(&span),
        "the clock is inside the reference window: {span} ticks"
    );
    // The attacker still got their hitmarker; the feed heard no kill.
    assert_eq!(count(&w, EV_HIT), 1);
    assert_eq!(count(&w, EV_DEATH), 0, "a wound is not a death");
    // The last health readout on the tick is the crawl's hp, not the zero
    // the emit site announced first — the bar must end where the body is.
    let last_health = w
        .events
        .entries()
        .iter()
        .rfind(|e| e.code == EV_HEALTH && e.a == VICTIM)
        .expect("a health readout");
    assert_eq!(last_health.b, WOUNDED_HP as u32);
    // The inventory stayed with the body: no bag stood up.
    assert_eq!(w.backpacks.len(), 0, "a downed body keeps its pack");
}

#[test]
fn a_blow_on_a_downed_body_is_the_finishing_one() {
    let mut w = duel_world();
    arm_victim(&mut w);
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    swing_until(&mut w, &mut seq, EV_DEATH);
    let e = only(&w, EV_DEATH);
    assert_eq!((e.a, e.b), (VICTIM, ATTACKER), "the feed credits the hand");
    assert_eq!(
        count(&w, EV_WOUNDED),
        0,
        "a downed body does not go down again"
    );
    let v = &w.players[1];
    assert!(v.dead && !v.wounded, "a corpse, not a crawl");
    assert_eq!(v.hp, 0);
    assert_eq!(v.deaths, 1, "counted once, by `die`");
    assert_eq!(v.death_by, ATTACKER);
    assert_eq!(v.death_cause, DEATH_BY_HAND);
    assert_eq!(v.death_item, SPEAR);
    assert_eq!(w.backpacks.len(), 1, "the corpse dropped its bag");
}

#[test]
fn the_roll_gets_the_body_up_or_kills_it_and_the_hash_decides_which() {
    let mut w = duel_world();
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    stand_down(&mut w);
    let until = w.players[1].wound_until;
    let chance = chance_of(&w, 1);
    assert_eq!(
        chance, 200,
        "inert survival content: the base odds and no bonus"
    );
    // Nothing happens before the clock.
    while w.tick < until {
        w.tick(&[]);
        assert!(
            w.players[1].wounded && !w.players[1].dead,
            "early roll at {}",
            w.tick
        );
        assert_eq!(count(&w, EV_RECOVERED) + count(&w, EV_DEATH), 0);
    }
    // The roll's tick.
    w.tick(&[]);
    let v = &w.players[1];
    if recovers(SEED, VICTIM, until, chance) {
        let e = only(&w, EV_RECOVERED);
        assert_eq!((e.a, e.b, e.c), (VICTIM, chance, WOUNDED_HP as u32));
        assert!(!v.wounded && !v.dead, "up");
        assert_eq!(v.hp, WOUNDED_HP, "with what it had");
        assert_eq!(v.deaths, 0);
        assert_eq!(v.rewound_until, until + REWOUND_TICKS);
        assert_eq!(v.death_by, 0, "the downing facts are cleared");
        assert_eq!(v.death_item, NO_ITEM);
    } else {
        let e = only(&w, EV_DEATH);
        assert_eq!(
            (e.a, e.b),
            (VICTIM, ATTACKER),
            "credited to the blow that put it down"
        );
        assert!(v.dead && !v.wounded);
        assert_eq!(v.deaths, 1);
        assert_eq!(v.death_cause, DEATH_BY_HAND);
        assert_eq!(v.death_item, SPEAR);
    }
}

/// Both outcomes are reachable and both are exercised: the clock is aimed
/// at a tick the hash favours and then at one it does not.
#[test]
fn both_ends_of_the_roll_are_reachable() {
    for want_up in [true, false] {
        let mut w = duel_world();
        let mut seq = 0;
        swing_until(&mut w, &mut seq, EV_WOUNDED);
        stand_down(&mut w);
        let chance = chance_of(&w, 1);
        let t = (w.tick..w.tick + 400)
            .find(|&t| recovers(SEED, VICTIM, t, chance) == want_up)
            .expect("both faces of a 20% die show inside 400 ticks");
        w.players[1].wound_until = t;
        while w.tick <= t {
            w.tick(&[]);
        }
        assert_eq!(
            !w.players[1].dead && !w.players[1].wounded,
            want_up,
            "up={want_up}"
        );
        assert_eq!(count(&w, EV_RECOVERED) == 1, want_up);
        assert_eq!(count(&w, EV_DEATH) == 1, !want_up);
    }
}

const RECOVERY_ITEM: u16 = 1;

fn recovery_item(w: &mut World, slot: usize, count: u16) {
    w.survival.belt_recovery[RECOVERY_ITEM as usize] = true;
    w.players[1].inv[slot] = ItemStack {
        item: RECOVERY_ITEM,
        count,
        cond: 0,
        skin: 0,
    };
}

/// Aim the deadline at each face of the natural die. The body must stay
/// down and keep its inventory on every tick before that deadline.
fn roll_on(w: &mut World, natural_recovery: bool) -> u64 {
    let chance = chance_of(w, 1);
    let t = (w.tick + 2..w.tick + 400)
        .find(|&t| recovers(SEED, VICTIM, t, chance) == natural_recovery)
        .unwrap();
    w.players[1].wound_until = t;
    let inv = w.players[1].inv;
    while w.tick < t {
        w.tick(&[]);
        assert!(w.players[1].wounded && !w.players[1].dead);
        assert_eq!(w.players[1].inv, inv, "no early payment");
        assert_eq!(count(w, EV_CONSUMED) + count(w, EV_RECOVERED), 0);
    }
    w.tick(&[]);
    t
}

#[test]
fn a_belt_item_saves_only_failed_rolls_and_every_belt_slot_counts() {
    for inv in 0..HOTBAR_SLOTS {
        for natural_recovery in [false, true] {
            let mut w = duel_world();
            recovery_item(&mut w, inv, 2);
            // This must work without selecting the kit.
            w.players[1].frame.sel = ((inv + 1) % HOTBAR_SLOTS) as u8;
            swing_until(&mut w, &mut 0, EV_WOUNDED);
            assert_eq!(only(&w, EV_WOUNDED).c, 1000, "the screen knows the belt");
            stand_down(&mut w);
            let t = roll_on(&mut w, natural_recovery);
            let p = &w.players[1];
            assert!(!p.dead && !p.wounded);
            assert_eq!(p.hp, WOUNDED_HP);
            assert_eq!(
                p.heal_rem, 0,
                "a rescue does not also grant the ordinary heal"
            );
            assert_eq!(p.deaths, 0);
            assert_eq!(p.rewound_until, t + REWOUND_TICKS);
            assert_eq!((p.death_by, p.death_item), (0, NO_ITEM));
            assert_eq!(p.inv[inv].count, if natural_recovery { 2 } else { 1 });
            assert_eq!(count(&w, EV_DEATH), 0);
            assert_eq!(w.backpacks.len(), 0);
            assert_eq!(count(&w, EV_CONSUMED), u32::from(!natural_recovery));
            let e = only(&w, EV_RECOVERED);
            assert_eq!((e.a, e.c), (VICTIM, WOUNDED_HP as u32));
            assert_eq!(
                e.b,
                if natural_recovery {
                    chance_of(&w, 1)
                } else {
                    1000
                }
            );
            if !natural_recovery {
                let e = only(&w, EV_CONSUMED);
                assert_eq!(
                    (e.a, e.b, e.c),
                    (VICTIM, (RECOVERY_ITEM as u32) << 16 | inv as u32, 0)
                );
            }
        }
    }
}

#[test]
fn recovery_spends_only_the_first_belt_stack_and_clears_its_last_unit() {
    let mut w = duel_world();
    recovery_item(&mut w, 0, 1);
    recovery_item(&mut w, HOTBAR_SLOTS - 1, 2);
    recovery_item(&mut w, HOTBAR_SLOTS, 3);
    swing_until(&mut w, &mut 0, EV_WOUNDED);
    stand_down(&mut w);
    roll_on(&mut w, false);
    assert_eq!(w.players[1].inv[0], ItemStack::default(), "canonical empty");
    assert_eq!(w.players[1].inv[HOTBAR_SLOTS - 1].count, 2);
    assert_eq!(w.players[1].inv[HOTBAR_SLOTS].count, 3);
    assert_eq!(count(&w, EV_CONSUMED), 1);
}

#[test]
fn a_backpack_kit_or_an_unflagged_belt_item_cannot_save_a_failed_roll() {
    for (inv, flagged) in [(HOTBAR_SLOTS, true), (INV_SLOTS - 1, true), (0, false)] {
        let mut w = duel_world();
        recovery_item(&mut w, inv, 1);
        w.survival.belt_recovery[RECOVERY_ITEM as usize] = flagged;
        swing_until(&mut w, &mut 0, EV_WOUNDED);
        assert_eq!(only(&w, EV_WOUNDED).c, chance_of(&w, 1));
        stand_down(&mut w);
        roll_on(&mut w, false);
        assert!(w.players[1].dead);
        assert_eq!(count(&w, EV_CONSUMED) + count(&w, EV_RECOVERED), 0);
        assert_eq!(only(&w, EV_DEATH).b, ATTACKER);
    }
}

#[test]
fn recovery_ignores_empty_and_invalid_belt_stacks() {
    let mut w = duel_world();
    recovery_item(&mut w, 0, 0);
    w.players[1].inv[1] = ItemStack {
        item: NO_ITEM,
        count: 1,
        cond: 0,
        skin: 0,
    };
    assert_eq!(w.survival.belt_recovery_slot(&w.players[1]), None);
    recovery_item(&mut w, 2, 1);
    assert_eq!(w.survival.belt_recovery_slot(&w.players[1]), Some(2));
}

#[test]
fn a_finishing_blow_does_not_spend_the_belt_kit_or_prevent_death() {
    let mut w = duel_world();
    recovery_item(&mut w, 0, 1);
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    swing_until(&mut w, &mut seq, EV_DEATH);
    assert!(w.players[1].dead);
    assert_eq!(count(&w, EV_CONSUMED) + count(&w, EV_RECOVERED), 0);
    assert_eq!(w.backpacks.len(), 1);
    assert_eq!(w.backpacks.entries()[0].items[0].item, RECOVERY_ITEM);
    assert_eq!(w.backpacks.entries()[0].items[0].count, 1);
}

#[test]
fn a_saved_sleeper_replays_the_same_rescue_and_cannot_spend_the_kit_twice() {
    let mut w = duel_world();
    recovery_item(&mut w, HOTBAR_SLOTS - 1, 1);
    swing_until(&mut w, &mut 0, EV_WOUNDED);
    w.tick(&[
        Command::Leave { id: ATTACKER },
        Command::Leave { id: VICTIM },
    ]);
    let t = (w.tick + 2..w.tick + 400)
        .find(|&t| !recovers(SEED, VICTIM, t, chance_of(&w, 1)))
        .unwrap();
    w.players[1].wound_until = t;
    let mut bytes = vec![0; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut bytes).unwrap();
    let mut restored = duel_world();
    restored.survival = w.survival;
    restored.load(&bytes[..n]).unwrap();
    assert_eq!(w.state_hash(), restored.state_hash());
    while w.tick <= t {
        w.tick(&[]);
        restored.tick(&[]);
        assert_eq!(
            w.state_hash(),
            restored.state_hash(),
            "replay after restore"
        );
    }
    assert_eq!(count(&restored, EV_RECOVERED), 1);
    assert_eq!(count(&restored, EV_CONSUMED), 1);
    assert!(restored.players[1].sleeping && !restored.players[1].dead);
    assert_eq!(
        restored.players[1].inv[HOTBAR_SLOTS - 1],
        ItemStack::default()
    );
    // The canonical empty is accepted by the save reader after recovery too.
    let n = restored.save_world(&mut bytes).unwrap();
    restored.load(&bytes[..n]).unwrap();
    restored.tick(&[]);
    assert_eq!(
        count(&restored, EV_CONSUMED) + count(&restored, EV_RECOVERED),
        0
    );
}

#[test]
fn a_second_down_inside_the_minute_is_a_death_and_after_it_a_fall() {
    let mut w = duel_world();
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    stand_down(&mut w);
    let chance = chance_of(&w, 1);
    let t = (w.tick..w.tick + 400)
        .find(|&t| recovers(SEED, VICTIM, t, chance))
        .unwrap();
    w.players[1].wound_until = t;
    while w.tick <= t {
        w.tick(&[]);
    }
    assert_eq!(count(&w, EV_RECOVERED), 1);
    assert!(w.tick < w.players[1].rewound_until, "inside the minute");
    // Downed again inside the window: a death, not a fall.
    w.players[1].hp = 1;
    swing_until(&mut w, &mut seq, EV_DEATH);
    assert_eq!(
        count(&w, EV_WOUNDED),
        0,
        "no second crawl inside the minute"
    );
    assert!(w.players[1].dead);
    assert_eq!(w.players[1].deaths, 1);

    // And past the window (the field is `pub` so the test need not tick out
    // 1,800 ticks): the same blow is a fall again.
    let mut w = duel_world();
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    stand_down(&mut w);
    let chance = chance_of(&w, 1);
    let t = (w.tick..w.tick + 400)
        .find(|&t| recovers(SEED, VICTIM, t, chance))
        .unwrap();
    w.players[1].wound_until = t;
    while w.tick <= t {
        w.tick(&[]);
    }
    assert_eq!(w.players[1].rewound_until, t + REWOUND_TICKS);
    w.players[1].rewound_until = w.tick; // the minute is up
    w.players[1].hp = 1;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    assert!(
        w.players[1].wounded && !w.players[1].dead,
        "down again, alive again"
    );
    assert_eq!(count(&w, EV_DEATH), 0);
}

#[test]
fn a_downed_body_crawls_at_a_third_of_the_walk_and_swings_at_nothing() {
    let mut w = duel_world();
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    stand_down(&mut w);
    // The victim, told to sprint forward with the swing held.
    let go = InputFrame {
        seq: 1,
        buttons: BTN_SPRINT | BTN_PRIMARY,
        yaw: 16_384,
        pitch: 128,
        move_x: 0,
        move_z: 127,
        sel: 0,
    };
    // Give the victim a hand so a swing WOULD be one if it were allowed.
    w.players[1].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    let start = w.players[1].body;
    let ticks = 30;
    for i in 0..ticks {
        w.tick(&[Command::Input {
            id: VICTIM,
            frame: InputFrame {
                seq: i as u16 + 1,
                ..go
            },
            favour: 0,
        }]);
        assert_eq!(
            count(&w, EV_SWING),
            0,
            "a downed body's swing button swings nothing"
        );
    }
    let end = w.players[1].body;
    let (dx, dz) = (
        (end.qx - start.qx) as f32 * POS_XZ_Q,
        (end.qz - start.qz) as f32 * POS_XZ_Q,
    );
    let crawled = (dx * dx + dz * dz).sqrt();
    // The walk over the same ticks, standing, from the same spot: the ratio
    // is the crawl divisor's, terrain and all, because both bodies took the
    // same ground.
    let mut w2 = duel_world();
    w2.players[1].hp = 100;
    w2.players[1].body = start;
    let start2 = w2.players[1].body;
    for i in 0..ticks {
        w2.tick(&[Command::Input {
            id: VICTIM,
            frame: InputFrame {
                seq: i as u16 + 1,
                buttons: 0,
                ..go
            },
            favour: 0,
        }]);
    }
    let end2 = w2.players[1].body;
    let (dx2, dz2) = (
        (end2.qx - start2.qx) as f32 * POS_XZ_Q,
        (end2.qz - start2.qz) as f32 * POS_XZ_Q,
    );
    let walked = (dx2 * dx2 + dz2 * dz2).sqrt();
    assert!(
        walked > WALK_SPEED * 0.5,
        "the standing body walked: {walked} m"
    );
    let ratio = crawled / walked;
    let want = (127 / CRAWL_DIV) as f32 / 127.0;
    assert!(
        sim_core::fmath::fabs(ratio - want) < 0.08,
        "crawl/walk = {ratio:.3} ({crawled:.2} m / {walked:.2} m), want ~{want:.3}"
    );
    assert!(crawled > 0.5, "and it did move: {crawled} m");
    // The frame the sim stepped is what `crawl_frame` says it is.
    assert_eq!(crawl_frame(&go).move_z, 127 / CRAWL_DIV);
}

#[test]
fn verbs_are_refused_while_down_but_a_door_would_not_be() {
    let mut w = duel_world();
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    assert_eq!(w.live_slot_of(VICTIM), None, "a downed body is not live");
    assert_eq!(w.awake_slot_of(VICTIM), Some(1), "but it is conscious");
    assert_eq!(w.live_slot_of(ATTACKER), Some(0));
    // A verb that spends a hand does nothing: the craft queue is untouched.
    let before = w.players[1].jobs;
    w.tick(&[Command::Craft {
        id: VICTIM,
        recipe: 0,
        count: 1,
        skin: 0,
    }]);
    assert_eq!(w.players[1].jobs, before, "a crawl cannot craft");
    // And the corpse loses even the door.
    swing_until(&mut w, &mut seq, EV_DEATH);
    assert_eq!(w.awake_slot_of(VICTIM), None);
}

#[test]
fn a_sleeper_is_killed_outright() {
    let mut w = duel_world();
    let mut seq = 0;
    w.players[1].sleeping = true;
    swing_until(&mut w, &mut seq, EV_DEATH);
    assert!(w.players[1].dead && !w.players[1].wounded);
    assert_eq!(count(&w, EV_WOUNDED), 0, "nobody is there to crawl");
}

#[test]
fn a_downed_body_whose_owner_leaves_still_faces_the_roll() {
    let mut w = duel_world();
    let mut seq = 0;
    swing_until(&mut w, &mut seq, EV_WOUNDED);
    stand_down(&mut w);
    w.tick(&[Command::Leave { id: VICTIM }]);
    assert!(
        w.players[1].sleeping && w.players[1].wounded,
        "asleep on the ground"
    );
    let until = w.players[1].wound_until;
    let chance = chance_of(&w, 1);
    while w.tick <= until {
        w.tick(&[]);
    }
    let v = &w.players[1];
    if recovers(SEED, VICTIM, until, chance) {
        assert!(!v.wounded && !v.dead && v.sleeping, "up, and still asleep");
        assert_eq!(count(&w, EV_RECOVERED), 1);
    } else {
        assert!(v.dead && v.sleeping, "a corpse that is still a sleeper's");
        assert_eq!(count(&w, EV_DEATH), 1);
    }
}

#[test]
fn being_down_is_hashed() {
    let mut w = duel_world();
    let h0 = w.state_hash();
    w.players[1].wounded = true;
    let h1 = w.state_hash();
    w.players[1].wounded = false;
    w.players[1].wound_until = 77;
    let h2 = w.state_hash();
    w.players[1].wound_until = 0;
    w.players[1].rewound_until = 77;
    let h3 = w.state_hash();
    w.players[1].rewound_until = 0;
    assert_eq!(w.state_hash(), h0, "restored, the hash is restored");
    assert_ne!(h0, h1, "`wounded` is sim state");
    assert_ne!(h0, h2, "`wound_until` is sim state");
    assert_ne!(h0, h3, "`rewound_until` is sim state");
}

#[test]
fn the_window_is_the_reference_minute_at_this_tick_rate() {
    assert_eq!(WOUND_MIN_TICKS, 40 * TICK_HZ, "40 s");
    assert_eq!(WOUND_MAX_TICKS, 50 * TICK_HZ, "50 s");
    assert_eq!(REWOUND_TICKS, 60 * TICK_HZ as u64, "60 s between downs");
}
