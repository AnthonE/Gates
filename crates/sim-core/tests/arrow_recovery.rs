//! Arrows come back — the store, the break roll and the lodge timer
//! (`reference/PROJECTILES.md` §5 and §9.7, arrow recovery v0).
//!
//! **What this suite gates, and where the other half went.** §9.7
//! decomposes recovery into four pieces; this file is 1 and 2 — that a
//! landing produces a record, that the record carries the round rather
//! than the bow, that the lodge is exact, that the odds are the odds, that
//! the cap evicts by its stated rule, and that a world remembers all of it
//! across a save. It said "no player can press anything here" until arrow
//! recovery v1 built pieces 3 and 4; **the verb is gated next door**, in
//! `tests/arrow_pickup.rs`, and the split is deliberate — this file drives
//! the store directly and that one drives `World::tick`, which is the only
//! path a player has.
//!
//! **Every assertion below was run against a mutant** and the file's own
//! header used to be able to say that without it being true, which is why
//! the mutants are named in each test's doc: `CLAUDE.md`'s lattice entry is
//! about a gate that passed under the bug it was written for.

// Measurements are this gate's output — same allow and same reason as
// `tests/shoot.rs`: the L5 wall bans format/print in SIM code, and a test
// harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::combat::{AmmoDef, CombatContent, RangedDef};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS, MAX_SPENT_ARROWS, TICK_HZ};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Occupants, Pristine, Scratch};
use sim_core::ranged::{self, Arrows, Kill, ARROW_EYE_MM};
use sim_core::spent::{self, SpentArrows, SpentRec};
use sim_core::world::{EventQueue, Player};

/// Item indices the fixture makes a bow and its ammo. Distinct so that
/// "the round came back, not the weapon" is a checkable claim rather than
/// a coincidence — the whole point of `Arrow::round` existing beside
/// `Arrow::item`.
const BOW: u16 = 3;
const ARROW: u16 = 4;

/// Seconds of lodge the fixture arms, in ticks. Ten is the reference's
/// number and `content/balance.toml` ships it; the fixture states it
/// itself so a content edit cannot quietly turn a red here into a green
/// (`shoot.rs`'s rule for the bow's ballistics).
const LODGE_TICKS: u32 = 10 * TICK_HZ;

/// A bow fixture with recovery armed. `break_pct` is a parameter because
/// three tests need three different answers out of the same flight: never,
/// always, and the shipped odds.
fn bow(break_pct: u16) -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.arrow_break_pct = break_pct;
    c.arrow_lodge_ticks = LODGE_TICKS;
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
    };
    c.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 22,
    };
    c
}

fn archer(id: u32, x: f32, feet_y: f32, z: f32, pitch: u8) -> Player {
    let mut p = Player {
        id,
        active: true,
        hp: 100,
        hp_max: 100,
        ..Player::default()
    };
    p.body = Body {
        qx: (x / POS_XZ_Q) as i32,
        qy: (feet_y / POS_Y_Q) as i32,
        qz: (z / POS_XZ_Q) as i32,
        ..Body::default()
    };
    p.inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
    };
    p.inv[7] = ItemStack {
        item: ARROW,
        count: 10,
        cond: 0,
    };
    p.frame = InputFrame {
        buttons: BTN_PRIMARY,
        yaw: 0,
        pitch,
        ..InputFrame::default()
    };
    p
}

/// Fire one arrow and fly it until the store empties. Returns the spent
/// store and the tick the loop reached.
///
/// The archer stands high and shoots down, so the arrow meets the ground
/// inside a handful of ticks whatever the seed's relief happens to be —
/// the flight is not what this suite is about, `tests/shoot.rs` owns that.
fn fire_into_the_ground(seed: u64, break_pct: u16) -> (SpentArrows, u64) {
    let mut sc = Scratch::with(seed, Pristine);
    let ground = sim_core::terrain::ground(seed, &sc.haven, 1024.0, 1024.0);
    let cc = bow(break_pct);
    let cols = sim_core::collide::ColIndex::new();
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    // 20 m up, aimed straight down. `pitch` is the wire's byte and its
    // poles are not where a reader guesses: `pitch_lut.rs` puts **0 at
    // straight down** and 255 straight up, with level between 127 and 128.
    players[0] = archer(1, 1024.0, ground + 20.0, 1024.0, 0);
    let mut arrows = Arrows::new();
    let mut spent = SpentArrows::new();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    let mut events = EventQueue::default();
    assert!(
        ranged::draw(0, &cc, &mut arrows, &mut events, &mut players[0]),
        "a bow in hand must take the arm"
    );
    assert_eq!(arrows.len(), 1, "the draw must have produced one arrow");
    let mut t = 0u64;
    while !arrows.is_empty() && t < 120 {
        t += 1;
        let mut ev = EventQueue::default();
        // Built from the fields rather than through `Scratch::occupants`,
        // which borrows the whole struct: `step` also wants `&sc.haven`,
        // and disjoint field borrows are what let one immutable and one
        // mutable coexist here.
        let mut occ = Occupants {
            table: &sc.table,
            haven: &sc.haven,
            harvested: &sc.harvested,
            cache: &mut sc.cache,
        };
        ranged::step(
            seed,
            t,
            &sc.haven,
            &cols,
            &mut occ,
            &cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut ev,
            &mut kills,
            &mut chips,
        );
    }
    assert!(arrows.is_empty(), "the arrow never resolved");
    (spent, t)
}

// ---------------------------------------------------------------------
// The flight, end to end
// ---------------------------------------------------------------------

/// A shot that hit nothing but the hillside is an arrow you can walk over
/// and pick up — and it is the ROUND that is lying there, not the bow.
///
/// Mutant run: returning `a.item` instead of `a.round` from `land` (the
/// obvious copy-paste, and both are `u16` on the same struct) fails on the
/// `ARROW` assertion. Dropping the `land` call at the world-stop site
/// entirely fails on the length.
#[test]
fn a_missed_arrow_lies_where_it_landed_and_is_takeable_at_once() {
    for seed in [0u64, 1, 7, 12345] {
        let (mut spent, t) = fire_into_the_ground(seed, 0);
        assert_eq!(
            spent.len(),
            1,
            "seed {seed}: a landing with break_pct 0 must leave exactly one arrow"
        );
        let rec = spent.entries()[0];
        assert_eq!(
            rec.round, ARROW,
            "seed {seed}: the thing on the ground is the round, not the bow \
             ({BOW}) that fired it"
        );
        assert!(
            rec.ready_at <= t,
            "seed {seed}: a missed arrow waits for nothing — ready_at {} \
             against a landing on tick {t}",
            rec.ready_at
        );
        // And the verb-facing half: standing on it, it comes back.
        assert_eq!(
            spent.take_near(t, rec.qx, rec.qy, rec.qz, 100),
            Some(ARROW),
            "seed {seed}: an arrow at the taker's own feet must be takeable"
        );
        assert!(spent.is_empty(), "seed {seed}: taking must remove it");
    }
}

/// The odds are real: with `break_pct` at 100 the hillside keeps nothing.
///
/// This is the pre-recovery game, and it is the value `CombatContent::EMPTY`
/// ships — so this test is also the gate on the inert default being the
/// harsh end rather than the free one.
#[test]
fn a_broken_arrow_leaves_nothing_and_that_is_the_inert_default() {
    let (spent, _) = fire_into_the_ground(7, 100);
    assert!(spent.is_empty(), "break_pct 100 must destroy every landing");
    assert_eq!(
        CombatContent::EMPTY.arrow_break_pct,
        100,
        "an unarmed content set must destroy arrows, never hand them back: \
         the opposite failure is invisible to anyone looking at the game"
    );
}

/// An arrow that drew blood may not be re-used during the fight it was
/// fired in — the lodge, which is the only thing the ten seconds buys.
///
/// Mutant run: writing `ready_at: tick` on the body path (i.e. treating a
/// hit like a miss) fails on the "not yet" assertion; adding the lodge to
/// the *miss* path instead fails the test above.
#[test]
fn an_arrow_that_drew_blood_waits_out_its_lodge() {
    let seed = 7u64;
    let mut sc = Scratch::with(seed, Pristine);
    let cc = bow(0);
    let cols = sim_core::collide::ColIndex::new();
    let ground = sim_core::terrain::ground(seed, &sc.haven, 1024.0, 1024.0);
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    // Shooter and victim on one level line 6 m apart along +Z, so the
    // arrow reaches flesh long before it reaches dirt.
    // 128 is level — the value a client actually sends looking at the
    // horizon (`pitch_lut.rs`: 0 rad has no exact byte).
    players[0] = archer(1, 1024.0, ground, 1024.0, 128);
    players[1] = Player {
        id: 2,
        active: true,
        hp: 100,
        hp_max: 100,
        body: Body {
            qx: (1024.0 / POS_XZ_Q) as i32,
            qy: ((ground + ARROW_EYE_MM as f32 / 1000.0 - 1.2) / POS_Y_Q) as i32,
            qz: (1030.0 / POS_XZ_Q) as i32,
            ..Body::default()
        },
        ..Player::default()
    };

    let mut arrows = Arrows::new();
    let mut spent = SpentArrows::new();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    assert!(ranged::draw(
        0,
        &cc,
        &mut arrows,
        &mut EventQueue::default(),
        &mut players[0]
    ));
    let mut t = 0u64;
    while !arrows.is_empty() && t < 60 {
        t += 1;
        let mut ev = EventQueue::default();
        // Built from the fields rather than through `Scratch::occupants`,
        // which borrows the whole struct: `step` also wants `&sc.haven`,
        // and disjoint field borrows are what let one immutable and one
        // mutable coexist here.
        let mut occ = Occupants {
            table: &sc.table,
            haven: &sc.haven,
            harvested: &sc.harvested,
            cache: &mut sc.cache,
        };
        ranged::step(
            seed,
            t,
            &sc.haven,
            &cols,
            &mut occ,
            &cc,
            &mut arrows,
            &mut spent,
            &mut players,
            &mut ev,
            &mut kills,
            &mut chips,
        );
    }
    assert!(
        players[1].hp < 100,
        "the fixture must actually land a hit — an arrow that missed would \
         make every assertion below pass for the wrong reason"
    );
    assert_eq!(spent.len(), 1, "a hit leaves the arrow in the target");
    let rec = spent.entries()[0];
    assert_eq!(
        rec.ready_at,
        t + u64::from(LODGE_TICKS),
        "the lodge is the content's number of ticks after the hit, exactly"
    );
    // The whole mechanic, stated as the two calls it forbids and allows.
    assert_eq!(
        spent.take_near(rec.ready_at - 1, rec.qx, rec.qy, rec.qz, 1_000),
        None,
        "the arrow you just shot someone with is not yours again mid-fight"
    );
    assert_eq!(
        spent.take_near(rec.ready_at, rec.qx, rec.qy, rec.qz, 1_000),
        Some(ARROW),
        "and it is yours on the tick the lodge runs out"
    );
}

// ---------------------------------------------------------------------
// The roll
// ---------------------------------------------------------------------

/// The break rate is the rate the content declares, and it is the same
/// bits on every run.
///
/// **The band is ±0.5 points over 200 000 draws, and the first draft was
/// ±1.5 over 20 000 — which a mutant walked straight through.** Shifting
/// the comparison by one (`< pct` → `< pct + 1`) makes a declared 15 % pay
/// out at 16 %, and the old band admitted anything from 13.5 to 16.5, so
/// the test could not see a whole percentage point of ammunition tax. The
/// arithmetic: the draw is deterministic — same seed, same keys, the same
/// count on every run forever — so a band is not flake tolerance here, it
/// is tolerance for the multiply-shift's own deviation from an exact 15 %,
/// which is one part in 2³²/100. ±0.5 is six sigma of sampling noise at
/// this n and roughly ten million times the arithmetic's own error.
///
/// Mutant run — five, all caught: `< pct + 1` (16.014 %, the mutant that
/// broke the first band), `+ 1 < pct` (14.016 %), `<=` for `<` (16.014 %
/// too, because the left side is already reduced to 0..99 — an off-by-one
/// there is a whole point of ammunition tax and not the 2⁻³² nudge it
/// looks like), dropping the `>> 32`, and keying on the tick alone, which
/// the independence check below is for.
///
/// The **stated blind spot** is `% 100` in place of the multiply-shift.
/// Its bias is 2⁶⁴ mod 100 over 2⁶⁴, about 10⁻¹⁸, so no sample of any size
/// can see it and this test is not evidence about it. `loot.rs` carries
/// the same reasoning for the same form; the shape is chosen by argument,
/// not by measurement.
#[test]
fn the_break_roll_is_its_stated_rate_and_the_same_bits_twice() {
    let seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut broke = 0usize;
    let mut trace = Vec::with_capacity(64);
    let n = 200_000usize;
    for i in 0..n {
        let (tick, slot) = ((i / MAX_ARROWS) as u64, i % MAX_ARROWS);
        let b = spent::breaks(seed, tick, slot, 15);
        broke += usize::from(b);
        if trace.len() < 64 {
            trace.push(b);
        }
    }
    let pct = broke as f64 * 100.0 / n as f64;
    println!("break rate over {n} draws: {pct:.3}%");
    assert!(
        (14.5..=15.5).contains(&pct),
        "a declared 15 % that measures {pct:.3}% is not 15 %"
    );
    // Same seed, same draws — the whole of what "deterministic" means for
    // a stateless roll. Re-derived rather than remembered, so the check is
    // about the function and not about this vector.
    let again: Vec<bool> = (0..64)
        .map(|i| spent::breaks(seed, (i / MAX_ARROWS) as u64, i % MAX_ARROWS, 15))
        .collect();
    assert_eq!(
        trace, again,
        "the roll must not depend on anything but its key"
    );

    // Independence: two slots on one tick must not agree with each other,
    // which is what a roll keyed on the tick alone would produce. Sixty-four
    // slots at 15 % agreeing on every one is a 1-in-10^45 coincidence and a
    // certainty under that bug.
    let same_tick: Vec<bool> = (0..64).map(|s| spent::breaks(seed, 99, s, 15)).collect();
    assert!(
        same_tick.iter().any(|&b| b) && same_tick.iter().any(|&b| !b),
        "every arrow landing on one tick shared a fate — the roll is not \
         keyed on the slot"
    );
}

/// Both ends of the percentage mean what they say, and they short-circuit
/// rather than trusting the draw to be exactly extreme.
#[test]
fn zero_never_breaks_and_a_hundred_always_does() {
    for slot in 0..MAX_ARROWS {
        assert!(!spent::breaks(3, slot as u64, slot, 0));
        assert!(spent::breaks(3, slot as u64, slot, 100));
    }
}

// ---------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------

fn rec(x: i32, ready_at: u64) -> SpentRec {
    SpentRec {
        qx: x,
        qy: 0,
        qz: 0,
        round: ARROW,
        ready_at,
    }
}

/// Wall 4: the cap is real, the policy is the stated one, and the eviction
/// is counted rather than silent.
///
/// Mutant run: making `lodge` refuse when full (the other plausible policy,
/// and the one `MAX_ARROWS` itself uses) fails the eviction count; evicting
/// index 0 instead of the smallest `ready_at` fails the survivor check.
#[test]
fn the_store_is_bounded_and_says_so() {
    let mut s = SpentArrows::new();
    // Fill it, newest last, `ready_at` ascending with the index.
    for i in 0..MAX_SPENT_ARROWS {
        assert!(
            !s.lodge(rec(i as i32, i as u64)),
            "no eviction while there is room"
        );
    }
    assert_eq!(s.len(), MAX_SPENT_ARROWS);
    assert_eq!(s.evictions(), 0);

    // One more. The store stays at the cap, the counter moves, and the
    // thing that went is the one that had been takeable longest.
    assert!(
        s.lodge(rec(-1, 9_000)),
        "a full store must evict, not refuse"
    );
    assert_eq!(s.len(), MAX_SPENT_ARROWS, "the cap holds");
    assert_eq!(
        s.evictions(),
        1,
        "an eviction that nothing counts is an absence"
    );
    assert!(
        !s.entries().iter().any(|e| e.ready_at == 0),
        "the evicted entry must be the smallest ready_at, not an arbitrary slot"
    );
    assert!(
        s.entries().iter().any(|e| e.qx == -1),
        "and the arrow that just landed must be the one that survived"
    );
}

/// The reason the policy keys on `ready_at` and not on a landing tick: a
/// busy shard must not steal the arrow you have just put into somebody.
#[test]
fn a_lodged_arrow_is_not_evicted_out_from_under_its_owner() {
    let mut s = SpentArrows::new();
    // One arrow lodged in a body far in the future, then a full store of
    // litter that has been collectable since tick zero.
    s.lodge(rec(777, 100_000));
    for i in 1..MAX_SPENT_ARROWS {
        s.lodge(rec(i as i32, i as u64));
    }
    for i in 0..64 {
        s.lodge(rec(10_000 + i, 50_000 + i as u64));
    }
    assert_eq!(s.evictions(), 64);
    assert!(
        s.entries().iter().any(|e| e.qx == 777),
        "the lodged arrow outlived 64 evictions because its window had not \
         opened — which is the whole reason the policy reads ready_at"
    );
}

/// `take_near` is the verb's half: nearest, ready, and inside the reach the
/// CALLER names — this module picks no reach of its own.
///
/// Mutant run: taking the first match instead of the nearest fails the
/// nearest assertion; dropping the `tick < ready_at` skip fails the
/// not-yet one; `<` for `>` on the reach compare fails the far one.
#[test]
fn take_near_takes_the_nearest_ready_arrow_and_nothing_else() {
    let mut s = SpentArrows::new();
    s.lodge(rec(500, 0)); // ready, 0.5 m away
    s.lodge(rec(100, 0)); // ready, 0.1 m away — the answer
    s.lodge(rec(50, 900)); // nearest of all, but not yet
    s.lodge(rec(9_000, 0)); // ready, 9 m away — out of a 1 m reach

    // Not ready and out of reach are both refusals, and the nearest READY
    // one inside the reach is what comes back.
    assert_eq!(s.take_near(10, 0, 0, 0, 1_000), Some(ARROW));
    assert_eq!(s.len(), 3, "exactly one arrow leaves the ground");
    assert!(
        !s.entries().iter().any(|e| e.qx == 100),
        "and it is the near ready one that left"
    );
    // With the near one gone the next-nearest ready one answers; the
    // lodged arrow at 50 mm is still skipped even though it is closest.
    assert_eq!(s.take_near(10, 0, 0, 0, 1_000), Some(ARROW));
    assert!(
        !s.entries().iter().any(|e| e.qx == 500),
        "the 0.5 m arrow was the next-nearest ready one"
    );
    // Nothing ready is left inside a 1 m reach.
    assert_eq!(s.take_near(10, 0, 0, 0, 1_000), None);
    // Reach past the far one and it answers; wind the clock past the lodge
    // and the near one does.
    assert_eq!(s.take_near(10, 0, 0, 0, 10_000), Some(ARROW));
    assert_eq!(s.take_near(1_000, 0, 0, 0, 1_000), Some(ARROW));
    assert!(s.is_empty());
}

/// A separation of tens of metres in millimetres overflows `i32` when it is
/// squared, so the distance solve is `i64`. Without it a 60 m arrow reads
/// as *nearer* than one at arm's length, and the reach test lets it
/// through.
#[test]
fn a_distant_arrow_does_not_wrap_into_a_near_one() {
    let mut s = SpentArrows::new();
    // 50 m out on each axis: 50_000² = 2.5e9, past i32::MAX at 2.1e9.
    s.lodge(rec(50_000, 0));
    assert_eq!(
        s.take_near(10, 0, 0, 0, 2_000),
        None,
        "a 50 m arrow is not within 2 m of the origin under any arithmetic"
    );
}

// ---------------------------------------------------------------------
// The world remembers
// ---------------------------------------------------------------------

/// The store is hashed, so it has to be saved — a blob that dropped it
/// would load to a different `state_hash` than it was taken from, which is
/// wall 5 failing at the origin (`worldsave.rs` makes the argument for
/// `PLAYER_TAIL_BYTES` and this is the same one).
///
/// Mutant run: dropping `w.spent.restore(...)` from the installer fails on
/// the hash; dropping the eviction counter from the head fails on the hash
/// too, which is the half a records-only round-trip would have missed.
#[test]
fn a_world_that_remembers_its_arrows_saves_them() {
    use sim_core::world::World;
    let mut w = Box::new(World::new(4242));
    for i in 0..40 {
        w.spent.lodge(SpentRec {
            qx: 1_000 + i,
            qy: 17 * i,
            qz: 2_000 - i,
            round: ARROW,
            ready_at: 900 + i as u64,
        });
    }
    // Force the counter off zero, so the round-trip is about both halves.
    for i in 0..MAX_SPENT_ARROWS + 5 {
        w.spent.lodge(rec(i as i32, i as u64));
    }
    assert!(
        w.spent.evictions() > 0,
        "the fixture must exercise the counter"
    );
    let before = w.state_hash();
    let evicted = w.spent.evictions();

    let mut blob = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("a world must save");
    blob.truncate(n);

    let mut w2 = Box::new(World::new(4242));
    w2.load(&blob).expect("and load again");
    assert_eq!(w2.spent.len(), MAX_SPENT_ARROWS);
    assert_eq!(w2.spent.evictions(), evicted);
    assert_eq!(
        w2.state_hash(),
        before,
        "a save that forgot the arrows on the ground is wall 5 failing at \
         the origin"
    );
}

/// An empty store folds not one byte, so the pinned replay hash stays
/// evidence about the script it pins rather than about this slice landing
/// (`world.rs::state_hash` states the rule; this is the check).
#[test]
fn a_world_that_never_fired_hashes_as_though_this_store_did_not_exist() {
    use sim_core::world::World;
    let a = Box::new(World::new(5));
    let mut b = Box::new(World::new(5));
    assert_eq!(a.state_hash(), b.state_hash());
    // One landing, one eviction's worth of history, and the two must part.
    b.spent.lodge(rec(1, 1));
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "an arrow on the ground is state, and state that does not reach the \
         hash is a divergence nothing can see"
    );
    b.spent.take_near(9, 1, 0, 0, 10);
    assert!(b.spent.is_empty());
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "and a store emptied by pickups with no eviction behind it folds \
         nothing again"
    );
}
