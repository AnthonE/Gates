//! The rewind ring: where every body stood, for the last `REWIND_TICKS`
//! ticks.
//!
//! Slice 2 of `findings/lagcomp-design-20260818.md` §7, and **nothing in the
//! sim reads the ring yet** — `combat::strike` is slice 4. So this suite is
//! the whole of what stands behind it, and it is written to fail under a
//! mutant rather than to describe the code: every check that could be
//! satisfied by a ring that never records anything also asserts that the
//! body it is tracking actually moved.
//!
//! The history it compares against is **rebuilt from published parts** — the
//! world's own `players[s].body`, read after each `tick` returns — not from
//! anything `rewind.rs` computes. `CLAUDE.md`'s naive-rebuild trap is about
//! exactly this: a second implementation that calls the function under test
//! carries the same mutant and the gate is green for the wrong reason.

// The measurements are the gate's output. Same allow and same reason as
// `tests/solid.rs`: wall 3 bans format/print in SIM code, and a test harness
// is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::input::InputFrame;
use sim_core::limits::{
    INTERP_DELAY_TICKS, MAX_PLAYERS, REWIND_ACK_BIAS_TICKS, REWIND_MAX_TICKS, REWIND_TICKS,
    SNAPSHOT_INTERVAL_TICKS,
};
use sim_core::movement::Body;
use sim_core::rewind::{Rewind, RewindPose, NO_TENANT};
use sim_core::world::{Command, World};

const SEED: u64 = 20_260_731;
/// The id the walker joins under. Deliberately not 1, so a check that reads
/// the id back is reading a value and not an index.
const WALKER: u32 = 7;
/// Ticks driven before the assertions — comfortably past `REWIND_TICKS`, so
/// the ring has wrapped at least twice and every row has been overwritten.
const WALK_TICKS: usize = 20;

/// A world with one player walking due north at full stick, plus the pose
/// the world held at the end of every tick — `history[t]` is the body as it
/// stood at the end of tick `t`, which is exactly what row `t` should hold.
fn walked_world() -> (World, Vec<Body>) {
    let mut w = World::new(SEED);
    w.tick(&[Command::Join { id: WALKER }]);
    let mut history = vec![w.players[0].body];
    assert_eq!(w.tick, 1, "the join tick is tick 0");

    for seq in 0..WALK_TICKS {
        w.tick(&[Command::Input {
            id: WALKER,
            frame: InputFrame {
                seq: seq as u16,
                move_z: 127,
                ..InputFrame::default()
            },
        }]);
        history.push(w.players[0].body);
    }
    assert_eq!(w.tick as usize, WALK_TICKS + 1);
    assert_eq!(history.len(), WALK_TICKS + 1);
    (w, history)
}

/// The live pose of slot `s` — what a caller hands `pose_at` as the fallback.
fn live_of(w: &World, s: usize) -> RewindPose {
    RewindPose::live(w.players[s].id, &w.players[s].body)
}

/// The walker has to actually move, or every check below is satisfied by a
/// ring that stores nothing. Asserted once, loudly, and reused.
fn assert_the_body_moved(history: &[Body]) {
    let mut distinct = 0;
    for i in 1..history.len() {
        if history[i] != history[i - 1] {
            distinct += 1;
        }
    }
    assert!(
        distinct >= REWIND_TICKS,
        "the fixture must produce at least {REWIND_TICKS} distinct poses or \
         the ring checks are vacuous — got {distinct} changes across \
         {} ticks",
        history.len()
    );
}

#[test]
fn the_ring_holds_where_a_body_stood() {
    let (w, history) = walked_world();
    assert_the_body_moved(&history);
    let live = live_of(&w, 0);

    // Every depth the ring can reach, compared against the independently
    // kept history — not against anything `rewind.rs` derived.
    for back in 1..=REWIND_TICKS as u8 {
        let want_tick = w.tick - back as u64;
        let want = history[want_tick as usize];
        let got = w.rewind.pose_at(w.tick, 0, back, live);
        assert_eq!(
            (got.id, got.qx, got.qy, got.qz),
            (WALKER, want.qx, want.qy, want.qz),
            "{back} ticks back should be the pose at end of tick {want_tick}"
        );
    }
}

#[test]
fn seven_ticks_back_is_not_the_present() {
    let (w, history) = walked_world();
    assert_the_body_moved(&history);
    let live = live_of(&w, 0);

    // The clamp's own depth, stated as the thing the feature is for: a
    // shooter favoured by `REWIND_MAX_TICKS` is handed a body that is
    // measurably not the one standing there now.
    let got = w.rewind.pose_at(w.tick, 0, Rewind::max_back(), live);
    assert_ne!(
        (got.qx, got.qz),
        (live.qx, live.qz),
        "a {REWIND_MAX_TICKS}-tick rewind of a walking body must differ from \
         the present, or the ring is handing back the live pose"
    );
    assert_eq!(got.id, live.id);
}

#[test]
fn a_rewind_of_zero_is_the_live_body() {
    let (mut w, history) = walked_world();
    assert_the_body_moved(&history);

    // ⚠ **This has to be set up, or it passes for the wrong reason.** In a
    // world that has only ever been ticked, row `tick` holds tick
    // `tick - REWIND_TICKS`, so a `back` of 0 falls back on the *stamp*
    // check and the early-out is never exercised — the first draft of this
    // test asserted the rule and a mutant that deleted the early-out passed
    // it. So: write a row FOR the current tick, then doctor the live body so
    // the two disagree. Now the only thing that can return `live` is the
    // early-out.
    //
    // It matters because slice 4's whole no-op proof rests on it: `favour ==
    // 0` must be the live body unconditionally, not merely wherever the
    // write happens to sit in the tick.
    w.rewind.write_row(w.tick, &w.players);
    let stored = w.players[0].body.qx;
    w.players[0].body.qx = stored + 999_999;
    let live = live_of(&w, 0);

    let got = w.rewind.pose_at(w.tick, 0, 0, live);
    assert_eq!(got, live);
    assert_ne!(
        got.qx, stored,
        "a rewind of 0 must not consult the ring even when the ring can answer"
    );
}

#[test]
fn a_cold_ring_answers_present_at_every_depth() {
    // A world loaded at tick N has no history, and the rule that keeps wall
    // 5 whole is that it resolves at present until the ring fills. Asserted
    // against a bare `Rewind` so nothing else can supply the answer.
    let cold = Rewind::new();
    let live = RewindPose {
        id: 3,
        qx: 11,
        qy: 22,
        qz: 33,
    };
    for back in 0..=REWIND_TICKS as u8 {
        assert_eq!(
            cold.pose_at(1_000_000, 0, back, live),
            live,
            "a cold row must fall back to present at depth {back}"
        );
    }
}

#[test]
fn the_first_ticks_of_a_world_answer_present() {
    // `tick - back` has no answer before the world is `back` ticks old, and
    // the underflow must fall back rather than wrap into the top of the u64
    // range and read a garbage row.
    //
    // ⚠ **The body has to have moved for this to mean anything.** The first
    // draft asked at tick 1, where every candidate row holds the join pose —
    // so `saturating_sub` clamping to tick 0 returned the live body anyway
    // and the mutant passed. Walking first makes tick 0's pose distinguish
    // itself from the present, which is what turns a clamp into a failure.
    const YOUNG: usize = 2;
    const {
        assert!(
            YOUNG + 2 < REWIND_TICKS,
            "the world must still be younger than the ring"
        )
    };

    let mut w = World::new(SEED);
    w.tick(&[Command::Join { id: WALKER }]);
    let joined = w.players[0].body;
    for seq in 0..YOUNG {
        w.tick(&[Command::Input {
            id: WALKER,
            frame: InputFrame {
                seq: seq as u16,
                move_z: 127,
                ..InputFrame::default()
            },
        }]);
    }
    let live = live_of(&w, 0);
    assert_ne!(
        (live.qx, live.qz),
        (joined.qx, joined.qz),
        "the walker must have left the join pose or a clamp to tick 0 is \
         indistinguishable from the fallback"
    );

    // Reachable: 1 ..= w.tick. Past that the world has no such tick.
    for back in (w.tick as u8 + 1)..=REWIND_TICKS as u8 {
        assert_eq!(
            w.rewind.pose_at(w.tick, 0, back, live),
            live,
            "tick {} has no tick {} to rewind to",
            w.tick,
            w.tick as i64 - back as i64
        );
    }
}

#[test]
fn a_favour_past_the_ring_falls_back_rather_than_aliasing() {
    let (w, history) = walked_world();
    assert_the_body_moved(&history);
    let live = live_of(&w, 0);

    // The security property, and the reason `pose_at` needs no clamp of its
    // own: a `back` past the ring lands on a row stamped with a different
    // tick, so a forged favour costs the shooter their advantage instead of
    // buying more of it. 255 is the widest a `u8` can lie.
    for back in (REWIND_TICKS as u8 + 1)..=u8::MAX {
        assert_eq!(
            w.rewind.pose_at(w.tick, 0, back, live),
            live,
            "a favour of {back} must fall back to the live body"
        );
    }
}

#[test]
fn a_reused_slot_falls_back_to_present() {
    let (mut w, history) = walked_world();
    assert_the_body_moved(&history);

    // Slots are reused and an id is minted per connection. Without the id
    // guard, the next tenant of slot 0 would be rewound onto the previous
    // tenant's footprints.
    let stranger = WALKER + 1;
    w.players[0].id = stranger;
    let live = live_of(&w, 0);
    assert_eq!(live.id, stranger);
    for back in 1..=REWIND_TICKS as u8 {
        assert_eq!(
            w.rewind.pose_at(w.tick, 0, back, live),
            live,
            "a row left by id {WALKER} must not answer for id {stranger}"
        );
    }
}

#[test]
fn an_empty_slot_falls_back_to_present() {
    let (w, history) = walked_world();
    assert_the_body_moved(&history);

    // Slot 1 was never joined, so every row holds `RewindPose::EMPTY`. Both
    // guards are exercised: a caller with a real id (the id compare) and a
    // caller whose id is itself the sentinel (the `NO_TENANT` compare, which
    // is the one the id compare cannot catch).
    for id in [42, NO_TENANT] {
        let live = RewindPose {
            id,
            qx: -5,
            qy: -6,
            qz: -7,
        };
        for back in 1..=REWIND_TICKS as u8 {
            assert_eq!(
                w.rewind.pose_at(w.tick, 1, back, live),
                live,
                "an empty slot must answer present for id {id} at depth {back}"
            );
        }
    }
}

#[test]
fn a_slot_that_was_empty_at_that_tick_falls_back_even_for_the_same_id() {
    // `write_row`'s `!p.active` branch, and the only check that can see it.
    //
    // A departed session is *skipped*, not zeroed: `active` goes false while
    // `id` and `body` linger in the record. So without that branch the ring
    // would record a body that was not in the world, under an id that is
    // still that player's — and the id guard would wave it through. The
    // narrow-but-real case is a reconnect into the same slot inside the
    // ring's depth, after which a shot is favoured against a corpse of a
    // session that had already ended.
    //
    // Found by mutating `write_row` to record every slot as live: eight
    // other mutants reddened this suite and that one did not, because a
    // never-joined slot's `Player::default()` and `RewindPose::EMPTY` are
    // byte-identical. It takes a slot that was occupied *first*.
    let (mut w, history) = walked_world();
    assert_the_body_moved(&history);

    w.players[0].active = false;
    for _ in 0..REWIND_TICKS {
        w.tick(&[]);
    }
    let departed = w.players[0].body;

    // The same id, back in the same slot, standing somewhere else.
    w.players[0].active = true;
    w.players[0].body.qx = departed.qx + 77_777;
    let live = live_of(&w, 0);
    assert_eq!(
        live.id, WALKER,
        "the lingering id is the point of this case"
    );
    assert_ne!(live.qx, departed.qx);

    for back in 1..=REWIND_TICKS as u8 {
        assert_eq!(
            w.rewind.pose_at(w.tick, 0, back, live),
            live,
            "the slot held nobody {back} ticks ago — a row from before the \
             disconnect must not answer for the session after it"
        );
    }
}

#[test]
fn an_out_of_range_slot_falls_back_to_present() {
    let (w, _) = walked_world();
    let live = RewindPose {
        id: 1,
        qx: 1,
        qy: 2,
        qz: 3,
    };
    assert_eq!(w.rewind.pose_at(w.tick, MAX_PLAYERS, 1, live), live);
    assert_eq!(w.rewind.pose_at(w.tick, usize::MAX, 1, live), live);
}

#[test]
fn the_ring_is_not_hashed() {
    // Wall 5's half of this slice. The ring is derived output and must stay
    // out of `state_hash` — the event ring's argument and `Pieces::cols`'.
    // Written as a mutation: doctor a row, prove the row moved, prove the
    // hash did not.
    let (mut w, history) = walked_world();
    assert_the_body_moved(&history);
    let before = w.state_hash();

    let keep = w.players[0].body;
    w.players[0].body.qx = keep.qx + 12_345;
    let doctored = w.tick + 99;
    w.rewind.write_row(doctored, &w.players);
    w.players[0].body = keep;

    let live = live_of(&w, 0);
    let got = w.rewind.pose_at(doctored + 1, 0, 1, live);
    assert_eq!(
        got.qx,
        keep.qx + 12_345,
        "the doctored row must be readable, or this test proves nothing"
    );
    assert_eq!(
        w.state_hash(),
        before,
        "the rewind ring reached state_hash — it is derived output and two \
         shards agreeing on every hash already hold identical rings"
    );
}

#[test]
fn the_constants_hold_their_stated_relationships() {
    const {
        assert!(
            REWIND_TICKS.is_power_of_two(),
            "the ring index is a mask, not a modulo"
        )
    };
    const {
        assert!(
            (REWIND_MAX_TICKS as usize) < REWIND_TICKS,
            "the ring must hold every tick the clamp can ask for plus the row \
             being written"
        )
    };
    assert_eq!(Rewind::max_back(), REWIND_MAX_TICKS);
    // Derived, not picked: half the snapshot interval.
    assert_eq!(REWIND_ACK_BIAS_TICKS as u64 * 2, SNAPSHOT_INTERVAL_TICKS);
    // The interpolation delay now lives here because the SERVER needs it.
    // The client's `f64` spelling of it is checked from the other side, in
    // `client-core/tests/interp_capacity.rs` — `sim-core` cannot depend on
    // `client-core`, which is the right direction and the reason the check
    // is over there.
    assert_eq!(INTERP_DELAY_TICKS, 4);
}

#[test]
fn the_ring_costs_what_limits_says() {
    // `limits::REWIND_TICKS` states the cost as
    // `MAX_PLAYERS * REWIND_TICKS * 16 B`. The 16 is the claim that carries
    // risk: a fifth field, or a `u64` id, silently makes it 24.
    assert_eq!(size_of::<RewindPose>(), 16);
    assert_eq!(MAX_PLAYERS * REWIND_TICKS * size_of::<RewindPose>(), 12_800);

    // Boxed inside, not outside: the rows are on the heap and `World` pays
    // only a pointer plus the stamps.
    assert_eq!(
        size_of::<Rewind>(),
        size_of::<usize>() + REWIND_TICKS * size_of::<u64>(),
        "Rewind must be a pointer to the rows plus the stamps — if the rows \
         landed inline, World grew by 12.8 kB of stack"
    );
}

/// `.cargo/config.toml`'s prose, so the number below is checked against the
/// file that states it rather than against memory.
const CARGO_CONFIG: &str = include_str!("../../../.cargo/config.toml");

#[test]
fn the_world_size_note_is_measured() {
    // `.cargo/config.toml` picks the wasm shadow stack off `size_of::<World>()`
    // and states the number in prose. Nothing checked it, and by the time
    // this slice measured it the note was **122 kB stale** — it said 434 kB
    // against a real 312 kB, because `Pieces` and `Deploys` moved to
    // `boxed_array` after the note was written. Wrong in the safe direction
    // and wrong all the same, which is `CLAUDE.md`'s hand-kept-mirror trap
    // in the one file whose whole purpose is to carry a measurement.
    //
    // Decimal kB, matching the note's own units, and truncated the way the
    // note reads.
    let measured_kb = size_of::<World>() / 1000;
    let claim = format!("`World` is {measured_kb} kB");
    println!(
        "size_of::<World>() = {} B = {measured_kb} kB",
        size_of::<World>()
    );
    assert!(
        CARGO_CONFIG.contains(&claim),
        "`.cargo/config.toml` does not say {claim:?} — World is \
         {} B and the note has drifted. Correct the note in the same commit \
         as whatever moved the number.",
        size_of::<World>()
    );
}
