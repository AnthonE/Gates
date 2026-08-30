//! A head is worth double, a leg half, and only where there is one to hit.
//!
//! **The ladder, not the head** — since 2026-08-30 (limb band v0). The
//! head arrived alone and the reduction that made it a boolean is written
//! into `reference/PROJECTILES.md` §9.4: with a head and a body and
//! nothing else, "most significant part crossed" is "was the head interval
//! crossed at all". A third band retires that, so `ranged::head_crossed`
//! is `ranged::part_crossed` returning a `collide::Part`, and §7's rule is
//! the derived `Ord` — the `max` over the bands a span touches, never the
//! one it entered through.
//!
//! **The subject is the multiplier's whole route**, from the column in
//! `content/weapons.toml` to the number a body loses, because every previous
//! failure of this column was a break in the route rather than a wrong
//! value: it has been parsed, banded and content-hashed since the content
//! crate was written and dropped at the bake every time
//! (`reference/PROJECTILES.md` §9.4). A suite that only checked
//! `combat::headshot(20, 2) == 40` would have been green for all of that.
//!
//! **The law is rebuilt from published parts, never from the function under
//! test** — `CLAUDE.md`'s lattice trap, which cost ten assertions that could
//! not see a mutant because the naive side called the thing it was checking.
//! `ranged::head_crossed` is `pub` for this and `collide::HEAD_BAND_M` is
//! the band; where a check needs the geometry it computes the altitude from
//! the pitch LUT and the eye constant and compares, rather than asking
//! `nearest_body` what it thought.
//!
//! **Ten mutants were run against this suite and two survived**, which is
//! recorded here rather than in a claim that they did not:
//!
//! | mutant | caught by |
//! |---|---|
//! | `bake_ranged` drops the column again | `content`'s own row |
//! | `draw` writes `1` instead of the bow's column | the arrow check |
//! | the band measured off the feet, not the crown | five of nine |
//! | `>=` weakened to `>` at a rail | the rails check |
//! | the span collapsed to the closest approach | the crown check |
//! | `min` dropped from `combat::headshot` | the saturation check |
//! | `EV_HIT` reverts to the unscaled column | the events check |
//! | `EV_HURT` reverts to the unscaled column | the events check |
//! | `HEAD_BAND_M` moved 0.25 → 0.20 | **nothing** |
//! | `exit.min(stop_t)` dropped at both call sites | **nothing** |
//!
//! Eleven more were run for the leg band, across both crates. Ten were
//! caught outright and the eleventh is the interesting one:
//!
//! | mutant | caught by |
//! |---|---|
//! | `bake_ranged` drops `limb_pct` | `content`'s own row |
//! | `draw` writes `100` instead of the bow's column | the arrow check |
//! | `Part::Limb` returned before the `hi > limb_hi` test | the shin check |
//! | `Part`'s variant order reversed | the shin check, the ladder check |
//! | `/ 100` dropped from `combat::limb` | the percent check |
//! | `Part::Chest` routed through `combat::limb` | the ladder check |
//! | `validate`'s `limb_pct == 0` bound dropped | `content`'s refusal row |
//! | `validate`'s `> 100` bound dropped | `content`'s refusal row |
//! | `balance`'s `!=` weakened to `<` | **nothing, at first** |
//! | `balance`'s `!=` weakened to `>` | `content`'s refusal row |
//! | `balance`'s headshot `!=` weakened to `<` | `content`'s refusal row |
//!
//! The ninth is worth carrying past this file. A row-by-row assertion that
//! every weapon's column equals the band is green under a comparison that
//! can never fire, because *shipped content agrees with the band* — which
//! is the whole point of shipping it. Proving a refusal needs a row that
//! disagrees, so `the_body_part_ladder_refuses_what_it_names` hands the
//! loader one; it did not exist for `headshot_mult` either, in the eleven
//! days that column had a band.
//!
//! The first survivor is correct and wanted: the band is a knob
//! (`DECISIONS.md` §open), the checks measure that the sim *implements*
//! whatever it says, and a gate that reddened when an operator moved a knob
//! would be a gate arguing with its own registry.
//!
//! The second is a real hole, `NOW.md` §0hs carries it, and the leg band
//! sharpened what it would take to close — **the fixture `NOW.md` proposed
//! cannot be built.** The clip is what stops a shot that dies in cover from
//! being credited with the part behind the cover, and it only ever matters
//! on a **rising** span, because clipping trims the far end and only a
//! rising far end reaches a more significant band. So the world must go
//! solid *inside* the victim's own cylinder, between the closest approach
//! and the exit — `nearest_body` filters any body whose closest approach
//! is past the stop, so the window is the far half of the footprint, 0.4 m
//! of travel. A plain wall cannot do it: a body standing legally in front
//! of one has its far edge at the wall's face, so `stop_t` lands on `exit`
//! and there is nothing to clip. "The victim 0.3 m in front of the wall"
//! puts a 0.4 m cylinder a quarter-metre inside the slab.
//!
//! What *can* reach it is the arrow's own tick boundary. `world_stop`
//! returns 1.0 with nothing in the way, an arrow's segment is one tick of
//! flight (1.33 m against a 0.8 m cylinder), and `BodyHit::exit` is
//! deliberately unclipped — so a steeply-climbing shaft that enters late
//! in its tick has `exit > 1` and the `min` is load-bearing with no cover
//! anywhere. `t` is the midpoint of the span, so `exit` can exceed 1 by at
//! most `1 - enter`: the fixture is an arrow entering near `t = 0.1` with
//! a chord about 1.8 ticks long, which is a planar speed near 0.44 m/tick
//! against 1.26 m/tick of climb. That is a `tests/shoot.rs`-shaped fixture
//! driving `step`, not a line, and it is not in this pass.
//!
//! `a_stop_before_the_head_is_not_a_headshot` pins the predicate half at
//! **both** ends of the ladder now — the head behind cover and the chest
//! above a clipped shin — and nothing here proves the two resolvers pass a
//! clipped span in. Until then the guard is conservative in the safe
//! direction: dropping it can only promote a part, never delete one.
//!
//! `tests/gun.rs` and `tests/shoot.rs` stay the suites for *who* is hit;
//! nothing here re-asserts a hit decision, because the head is a question
//! asked of a hit and never a second way to score one.

// The measurement-printing allow every ranged suite carries, and its
// reason: the L5 wall bans format/print in SIM code, and a harness is not
// sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::combat::NO_MAG;
use sim_core::collide::{
    ColIndex, Part, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M, HEAD_BAND_M, LIMB_BAND_M,
};
use sim_core::combat::{self, CombatContent, RangedDef};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_ARROWS, MAX_PLAYERS};
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::ranged::{self, part_crossed, Arrows, Kill};
use sim_core::rewind::Rewind;
use sim_core::spent::SpentArrows;
use sim_core::terrain;
use sim_core::world::{self, EventQueue, Player, EV_HIT, EV_HURT};

/// The seed the shard ships, and `gun.rs`'s memoized haven for it — a
/// `terrain::haven` is a few thousand `height` taps and these checks call
/// it from inside loops.
const SEED: u64 = 20260731;

fn hv() -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Option<&'static terrain::Haven>> = const { RefCell::new(None) };
    }
    if let Some(h) = CACHE.with(|c| *c.borrow()) {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(SEED)));
    CACHE.with(|c| *c.borrow_mut() = Some(h));
    h
}

const GUN: u16 = 5;
const ROUND: u16 = 6;
const BOW: u16 = 7;
const ARROW: u16 = 8;
/// `gun.rs`'s constant and its caveat: the nearest wire step to horizontal,
/// which is 0.353° **up**.
const LEVEL: u8 = 128;

/// The shipped revolver and the shipped bow, with the shipped `= 2`.
/// Written out rather than loaded, `gun.rs`'s rule: `sim-core` has no
/// dependency on the content crate, and a fixture a balance pass can edit
/// is a gate a balance pass can turn green. The pairing of these numbers
/// with the band is gated one crate over in `content/tests/content.rs`.
fn fixture() -> CombatContent {
    let mut c = CombatContent::EMPTY;
    c.player_hp = 100;
    c.ranged[GUN as usize] = RangedDef {
        damage: 20,
        ammo: [ROUND, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 12,
        hitscan: true,
        range_mm: 50_000,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
        // The shipped revolver's magazine (`content/weapons.toml`): eight
        // rounds and 3.4 s, which is 102 ticks at 30 Hz. Slot 0 — this
        // fixture bakes no other magazine.
        magazine: 8,
        reload_ticks: 102,
        mag_slot: 0,
    };
    c.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 1,
        hitscan: false,
        range_mm: 60_000,
        structure: 0,
        headshot_mult: 2,
        limb_pct: 50,
        // No magazine: a bow spends straight out of the quiver
        // (`RangedDef::magazine`), so the arrow path is unchanged by
        // reload v1 and this fixture is what says so.
        magazine: 0,
        reload_ticks: 0,
        mag_slot: NO_MAG,
    };
    c.ammo[ARROW as usize] = sim_core::combat::AmmoDef {
        speed_mmpt: 1333,
        drop_mmpt2: 0,
    };
    c
}

fn shooter(id: u32, x: f32, feet_y: f32, z: f32, weapon: u16, round: u16, pitch: u8) -> Player {
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
        item: weapon,
        count: 1,
        cond: 0,
    };
    p.inv[7] = ItemStack {
        item: round,
        count: 20,
        cond: 0,
    };
    // And the magazine, loaded (reload v1) — harmless for the bow, which
    // has no `mag_slot` and spends straight out of the quiver, and
    // necessary for the gun, which now dry-clicks on an empty cylinder.
    // Eight is the fixture's ceiling and the shipped revolver's. That is
    // enough because every sweep in this file builds a fresh shooter per
    // placement — `pull` fires exactly once per body it is handed — so a
    // cylinder never has to survive a second shot here.
    p.mag[0] = 8;
    p.mag_round[0] = round;
    p.frame = InputFrame {
        seq: 1,
        buttons: BTN_PRIMARY,
        yaw: 0,
        pitch,
        sel: 0,
        ..InputFrame::default()
    };
    p
}

fn target(id: u32, x: f32, feet_y: f32, z: f32) -> Player {
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
    p
}

/// One trigger pull, and everything it produced.
fn pull(cc: &CombatContent, players: &mut [Player; MAX_PLAYERS]) -> EventQueue {
    let mut sc = Scratch::barren();
    let mut events = EventQueue::default();
    let mut kills = [Kill::default(); MAX_ARROWS];
    let mut chips = [ranged::Chip::default(); MAX_ARROWS];
    ranged::hitscan(
        SEED,
        hv(),
        &ColIndex::new(),
        &mut sc.occupants(),
        0,
        // A cold ring and a zero favour: every body resolves live, which is
        // this suite's whole subject — the band's geometry, not the lag
        // compensation that decides which cylinder it is measured off.
        // `tests/gun.rs` owns that pairing.
        &Rewind::new(),
        &[0; MAX_PLAYERS],
        cc,
        players,
        &mut events,
        &mut kills,
        &mut chips,
    );
    events
}

/// Walk one target's feet down through a fixed shot, a centimetre at a
/// time, and record what each placement cost them.
///
/// **The whole geometry of the band, measured rather than re-derived.**
/// The shot never moves; only the body does. So the sequence of damages is
/// a vertical slice through the victim — nothing, then the body's own
/// number over the torso, then double over the head, then nothing again
/// once the shot passes over the crown. Where those runs begin and end is
/// the sim's actual answer for `CAPSULE_HEIGHT_M` and `HEAD_BAND_M`, and no
/// part of this helper knows the pitch, the eye height or the muzzle
/// altitude — which is the point. `CLAUDE.md`'s lattice trap is a naive
/// side that calls the thing under test; this side calls nothing.
///
/// One centimetre is `POS_Y_Q`, the quantum a body's height is stored in,
/// so the sweep cannot step over a rail it is trying to find.
fn sweep(cc: &CombatContent, weapon: u16, round: u16, dist: f32) -> Vec<(f32, u16)> {
    let eye_ground = 400.0f32;
    let mut out = Vec::new();
    // From well above the shot's line to well below it. The muzzle is
    // `ARROW_EYE_MM` up and climbs a little over the range, so a body whose
    // FEET start 2.5 m above the shooter's is entirely over the line and
    // the sweep opens in clear air — which the run assertions depend on.
    // 4.5 m of travel then clears a 1.7 m body out the bottom.
    let steps = (4.5 / POS_Y_Q) as i32;
    for k in 0..steps {
        let feet = eye_ground + 2.5 - k as f32 * POS_Y_Q;
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[0] = shooter(1, 0.0, eye_ground, 0.0, weapon, round, LEVEL);
        players[1] = target(2, 0.0, feet, dist);
        pull(cc, &mut players);
        out.push((feet, 100 - players[1].hp));
    }
    out
}

/// The contiguous run of placements that took `dmg`, as (first index, count).
/// Zero-length if there is none.
fn run_of(rows: &[(f32, u16)], dmg: u16) -> (usize, usize) {
    let first = rows.iter().position(|&(_, d)| d == dmg);
    match first {
        None => (0, 0),
        Some(i) => {
            let n = rows[i..].iter().take_while(|&&(_, d)| d == dmg).count();
            (i, n)
        }
    }
}

/// **The whole ladder, measured off the sim rather than asserted from the
/// constants that produced it**: 85 cm of leg at half, 60 cm of torso at
/// full, 25 cm of head at double, and 1.7 m of body when you add them up.
///
/// A shot is fixed in space and a body walks down through it. What comes
/// back is four runs: nothing while the line is under the soles, 10 while
/// it is in the legs, 20 in the torso, 40 in the head, and nothing again
/// once it clears the crown. The height of the doubled run is
/// `HEAD_BAND_M`, the halved run is `LIMB_BAND_M`, the three together are
/// `CAPSULE_HEIGHT_M` — and the **order** is the assertion that separates
/// a head from a shin, which is the one thing a ladder can get wrong that
/// a pair of bands could not.
///
/// **Nothing here knows the pitch, the eye or the muzzle.** That is what
/// makes it a second source: `arrival = muzzle − feet` never appears, so a
/// mutant in the sim's own altitude arithmetic cannot be mirrored here.
/// One quantum of tolerance, because the body's y is stored in centimetres
/// and a rail can only ever be found to the step that crossed it.
///
/// Mutants watched red: the band measured off the feet rather than the
/// crown, and the span collapsed. **`HEAD_BAND_M` moved to 0.20 does NOT
/// redden this**, and that is the design — the assertion compares the sim's
/// measured band against the published constant, so moving the knob moves
/// both. What it catches is the sim disagreeing with the knob, which is the
/// only thing a knob's gate should catch.
#[test]
fn the_three_runs_are_the_ladder_measured_off_the_body() {
    let cc = fixture();
    let rows = sweep(&cc, GUN, ROUND, 10.0);

    let (body_i, body_n) = run_of(&rows, 20);
    let (head_i, head_n) = run_of(&rows, 40);
    assert!(body_n > 0, "the shot must land on a torso somewhere");
    assert!(
        head_n > 0,
        "and on a head somewhere: no doubled run means the column never \
         reached the sim, which is the bug this whole slice is about"
    );

    // Feet descend as the index rises, so the head run is the LATER one.
    assert!(
        head_i > body_i,
        "the doubled band must be at the top of the body, not the bottom: \
         torso run starts at {body_i}, doubled run at {head_i}"
    );
    assert_eq!(
        head_i,
        body_i + body_n,
        "and it must be contiguous with the torso — a gap means a placement \
         that hits nothing between the chest and the head"
    );

    // The third rung, and it is the one that turned this sweep from two
    // runs into a ladder: half of 20 is 10, the fixture's `limb_pct` of
    // 50 applied to its `damage` of 20.
    let (limb_i, limb_n) = run_of(&rows, 10);
    assert!(
        limb_n > 0,
        "and on a leg somewhere: no halved run means `limb_pct` never \
         reached the sim, which is the `headshot_mult` bug one column over"
    );
    assert!(
        limb_i < body_i,
        "the halved band must be at the BOTTOM of the body: halved run \
         starts at {limb_i}, torso run at {body_i}"
    );
    assert_eq!(
        body_i,
        limb_i + limb_n,
        "and contiguous with the torso — a gap means a placement between \
         the hip and the chest that is neither"
    );

    let band_m = head_n as f32 * POS_Y_Q;
    let leg_m = limb_n as f32 * POS_Y_Q;
    let body_m = (limb_n + body_n + head_n) as f32 * POS_Y_Q;
    println!("measured head band {band_m:.3} m, leg band {leg_m:.3} m, body {body_m:.3} m");
    // `max` of the two differences rather than `abs`: wall 1's float set
    // is `+ − × ÷ sqrt min max clamp floor-by-cast`, and it binds this
    // crate's tests too.
    assert!(
        (band_m - HEAD_BAND_M).max(HEAD_BAND_M - band_m) <= POS_Y_Q * 1.5,
        "the doubled run is {band_m:.3} m and HEAD_BAND_M says {HEAD_BAND_M}"
    );
    assert!(
        (leg_m - LIMB_BAND_M).max(LIMB_BAND_M - leg_m) <= POS_Y_Q * 1.5,
        "the halved run is {leg_m:.3} m and LIMB_BAND_M says {LIMB_BAND_M}"
    );
    assert!(
        (body_m - CAPSULE_HEIGHT_M).max(CAPSULE_HEIGHT_M - body_m) <= POS_Y_Q * 1.5,
        "the three runs together are {body_m:.3} m and CAPSULE_HEIGHT_M says \
         {CAPSULE_HEIGHT_M}"
    );
    // Above the crown and below the feet, nothing — or the runs above are
    // the middle of something larger and mean nothing. **This line read
    // `rows[body_i - 1] == 0` until the leg band landed**, and it was the
    // check that noticed: what sits under the torso is no longer clear air.
    assert_eq!(rows[limb_i - 1].1, 0, "clear air below the feet");
    assert_eq!(rows[head_i + head_n].1, 0, "clear air above the crown");
}

/// Both events a landed shot pushes carry the number the body actually
/// lost, not the weapon's column.
///
/// **The two halves of a fight must agree.** `EV_HIT` is the attacker's
/// hitmarker and `EV_HURT` (wire v57) is the victim's arc, and each answers
/// "how hard was that". A headshot that doubles the hp loss and reports the
/// row's 20 to both screens is a fight where nobody can tell a head from a
/// chest — which is most of what the multiplier is *for*.
///
/// Mutants watched red: each `push` reverted to `def.damage`
/// independently — 20 reported against a 40 loss.
#[test]
fn both_events_report_the_multiplied_blow() {
    let cc = fixture();
    let rows = sweep(&cc, GUN, ROUND, 10.0);
    let (head_i, head_n) = run_of(&rows, 40);
    assert!(head_n > 0, "a head placement must exist");
    let feet = rows[head_i + head_n / 2].0;

    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = shooter(1, 0.0, 400.0, 0.0, GUN, ROUND, LEVEL);
    players[1] = target(2, 0.0, feet, 10.0);
    let events = pull(&cc, &mut players);
    let dealt = (100 - players[1].hp) as u32;
    assert_eq!(dealt, 40, "the placement must still be a headshot");

    let hit = events
        .entries()
        .iter()
        .find(|e| e.code == EV_HIT)
        .expect("a landed shot pushes EV_HIT");
    // `EV_HIT.c` packs the rung over the damage since v58, so the damage
    // is read through the accessor rather than off the raw field.
    assert_eq!(
        world::hit_damage(hit.c) as u32,
        dealt,
        "EV_HIT must report the blow that arrived"
    );
    let hurt = events
        .entries()
        .iter()
        .find(|e| e.code == EV_HURT)
        .expect("a landed shot pushes EV_HURT");
    assert_eq!(hurt.c, dealt, "EV_HURT must report the blow that arrived");
}

/// Every rung the ladder prices reaches the shooter as *itself* (v58).
///
/// **This is the gate `NOW.md` §0hs item 3 exists for.** The ladder has
/// paid x2 / x1 / x0.5 off the body part a shot crossed since limb band
/// v0, and `EV_HIT` carried only the product — so a leg hit and a graze
/// were the same fact on the attacker's screen, a headshot read as luck,
/// and aim was the one thing in this game a player could not learn by
/// playing it. The rung now rides two spare bits of the payload, and this
/// is what says it is the rung that was actually crossed rather than one
/// re-derived from the number.
///
/// The three placements come out of `sweep`, which knows nothing about the
/// bands — it walks a body down through a fixed shot and records what each
/// centimetre cost. So the fixture finds the head, chest and leg by their
/// *price*, and the assertion is that the payload's part agrees. A `hit_c`
/// that packed a constant, or packed the parts in the wrong order, or read
/// its span off the wrong end of the body, disagrees here.
///
/// Mutants watched red, and both are the ones a byte-golden cannot see:
/// `hit_c` writing `Part::Chest` at every site (the head and leg rows come
/// back as Chest), and `Part::bits`' `Head` and `Limb` arms swapped (the
/// three rows come back in the wrong order while every damage still
/// matches).
#[test]
fn every_rung_the_ladder_prices_reaches_the_hitmarker() {
    let cc = fixture();
    let rows = sweep(&cc, GUN, ROUND, 10.0);

    // Found by price, not by geometry: 40 is the doubled blow, 20 the
    // row's own, 10 the halved one. `run_of` returns the run of rows that
    // cost exactly that, and the middle of a run is the safest placement.
    let want = [(Part::Head, 40u16), (Part::Chest, 20), (Part::Limb, 10)];
    for (part, dmg) in want {
        let (i, n) = run_of(&rows, dmg);
        assert!(
            n > 0,
            "no placement costs {dmg}, so the fixture cannot reach {part:?}              — the ladder or the sweep moved, not this assertion"
        );
        let feet = rows[i + n / 2].0;

        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[0] = shooter(1, 0.0, 400.0, 0.0, GUN, ROUND, LEVEL);
        players[1] = target(2, 0.0, feet, 10.0);
        let events = pull(&cc, &mut players);
        let dealt = 100 - players[1].hp;
        assert_eq!(dealt, dmg, "the placement must still cost {dmg}");

        let hit = events
            .entries()
            .iter()
            .find(|e| e.code == EV_HIT)
            .expect("a landed shot pushes EV_HIT");
        assert_eq!(
            world::hit_part(hit.c),
            part,
            "a blow that cost {dmg} must reach the shooter as {part:?}"
        );
        assert_eq!(
            world::hit_damage(hit.c),
            dmg,
            "the pack must not disturb the damage it sits above"
        );
    }
}

/// The three rungs are three *distinct* payloads, which is the property the
/// marker actually needs (v58).
///
/// Separate from the test above on purpose. That one checks each rung is
/// labelled correctly; this one checks the labels are not all the same
/// value — a `Part::bits` that returned a constant would satisfy "the head
/// row is Head" only if `want` is read one row at a time, and this is the
/// cheap assertion that closes it. It also pins the ORDER, which is what
/// `Feed::hit_part`'s `max` depends on: merging a frame's hits by
/// `Ord` is only the most-significant-part rule if the bits agree with it.
#[test]
fn the_rungs_are_distinct_and_ordered_the_way_the_marker_merges_them() {
    let (limb, chest, head) = (
        world::hit_c(Part::Limb, 7),
        world::hit_c(Part::Chest, 7),
        world::hit_c(Part::Head, 7),
    );
    assert!(
        limb != chest && chest != head && limb != head,
        "two rungs pack to the same payload ({limb}, {chest}, {head}), so          the marker cannot tell them apart"
    );
    assert!(
        Part::Limb < Part::Chest && Part::Chest < Part::Head,
        "the derived Ord is the most-significant-part rule and          `Feed::hit_part` merges a frame with `max` over it"
    );
    assert!(
        Part::Limb.bits() < Part::Chest.bits() && Part::Chest.bits() < Part::Head.bits(),
        "the bits must be ordered the way the values are, or a max taken          over one is not a max over the other"
    );
    // The damage half survives every rung untouched.
    for part in Part::ALL {
        assert_eq!(
            world::hit_damage(world::hit_c(part, 65_535)),
            65_535,
            "{part:?} must not steal a bit from the damage below it"
        );
    }
}

/// An arrow obeys the same band as a bullet, from the multiplier it copied
/// off the bow at the draw.
///
/// **Fired through `draw` rather than by writing an `Arrow`**, because the
/// copy at the draw is the link this checks: `Arrow::head_mult` is
/// denormalized exactly like `damage` and `structure`, and a bow whose
/// column never reached the shaft is the same class of drop one hop later.
/// The two placements come from the bullet's own sweep, so the two weapons
/// are answered about the same two points on the same body.
///
/// Mutant watched red: `draw` writing `1` instead of `def.headshot_mult`
/// — 30 where 60 is wanted.
#[test]
fn an_arrow_carries_the_bows_multiplier_off_the_string() {
    let cc = fixture();
    let rows = sweep(&cc, GUN, ROUND, 10.0);
    let (body_i, body_n) = run_of(&rows, 20);
    let (head_i, head_n) = run_of(&rows, 40);
    let (limb_i, limb_n) = run_of(&rows, 10);
    assert!(body_n > 0 && head_n > 0 && limb_n > 0);

    // 60 / 30 / 15: the bow's 30 doubled, taken plain, and halved. The
    // third row is the one that makes this a **route** check for the new
    // column rather than a second copy of the old one — `Arrow::limb_pct`
    // is copied at the draw and read a dozen ticks later, so a bow that
    // dropped it at the string would still deal 60 and 30 here.
    for (feet, expect) in [
        (rows[head_i + head_n / 2].0, 60u16),
        (rows[body_i + body_n / 2].0, 30),
        (rows[limb_i + limb_n / 2].0, 15),
    ] {
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[0] = shooter(1, 0.0, 400.0, 0.0, BOW, ARROW, LEVEL);
        players[1] = target(2, 0.0, feet, 10.0);

        let mut arrows = Arrows::new();
        let mut events = EventQueue::default();
        let mut spent = SpentArrows::new();
        assert!(
            ranged::draw(0, &cc, &mut arrows, &mut events, &mut players[0]),
            "the bow must take the arm"
        );
        assert_eq!(arrows.len(), 1, "and actually fire");
        assert_eq!(
            arrows.entries().next().unwrap().head_mult,
            2,
            "the shaft must carry the bow's column, not a default"
        );
        assert_eq!(
            arrows.entries().next().unwrap().limb_pct,
            50,
            "and the other end of the ladder with it — 100 is the identity, \
             so a dropped column is a shaft that never discounts a leg"
        );

        let mut sc = Scratch::barren();
        let mut kills = [Kill::default(); MAX_ARROWS];
        let mut chips = [ranged::Chip::default(); MAX_ARROWS];
        for t in 1..40u64 {
            ranged::step(
                SEED,
                t,
                hv(),
                &ColIndex::new(),
                &mut sc.occupants(),
                &cc,
                &mut arrows,
                &mut spent,
                &mut players,
                &mut events,
                &mut kills,
                &mut chips,
            );
            if players[1].hp != 100 {
                break;
            }
        }
        let dealt = 100 - players[1].hp;
        println!("arrow at feet {feet:.2} dealt {dealt}");
        assert_eq!(
            dealt, expect,
            "an arrow into the same point a bullet took should deal {expect}"
        );
    }
}

/// The band's two rails, exactly, off the published constant — and the
/// published predicate, so nothing here re-derives the arithmetic the sim
/// uses.
///
/// **Half-open at neither end and closed at both**, which is a choice and
/// not an accident: the band is the top of a body, so its upper rail is the
/// crown and a shot grazing the crown is a headshot. A hair under the lower
/// rail is not.
///
/// Mutants watched red: `>=` → `>` at the lower rail, and `head_lo`
/// computed off the feet as `feet + HEAD_BAND_M` rather than off the crown.
#[test]
fn the_band_is_closed_at_both_rails() {
    // Two magnitudes, and the pair is the claim. At zero the rail can be
    // probed to a thousandth of a millimetre; at 400 m — the island's own
    // altitude, where every shipped shot is resolved — one f32 ulp is
    // 0.03 mm, so the finest question that can be asked there is a
    // millimetre. Both are far under `POS_Y_Q`, the centimetre a body's
    // height is actually stored in, which is why the coarsening is a fact
    // worth writing down rather than a defect.
    for (feet_mm, eps) in [(0.0f32, 0.001f32), (400_000.0, 1.0)] {
        let lo = feet_mm + (CAPSULE_HEIGHT_M - HEAD_BAND_M) * 1000.0;
        let hi = feet_mm + CAPSULE_HEIGHT_M * 1000.0;
        // A stationary "segment": the same altitude at both ends, so the
        // span is one point and the check is the rail and nothing else.
        let at = |y: f32| part_crossed(y, 0.0, feet_mm, 0.0, 1.0);

        assert_eq!(
            at(lo),
            Part::Head,
            "the lower rail itself is a head (at {feet_mm} mm)"
        );
        assert_eq!(
            at(hi),
            Part::Head,
            "the crown itself is a head (at {feet_mm} mm)"
        );
        assert_eq!(
            at((lo + hi) * 0.5),
            Part::Head,
            "the middle of the band is a head"
        );
        assert_eq!(
            at(lo - eps),
            Part::Chest,
            "{eps} mm under the band is the chest"
        );
        // Over the crown is the **fallback**, and asserting it by name is
        // what keeps the fallback honest: the three bands do not tile the
        // cylinder's complement, so an altitude with no part is scored at
        // the identity rather than at the nearest band. A mutant that made
        // `Part::Limb` the fallback would deal half damage to a shot that
        // sailed over a head, and only this line would see it.
        assert_eq!(
            at(hi + eps),
            Part::Chest,
            "{eps} mm over the crown is no part at all"
        );
    }
}

/// The leg band's two rails, exactly, off the published constant — the
/// crown check's twin at the other end of the body, and written second
/// because the leg band is what turned a boolean into an ordering.
///
/// **Closed at both rails for the same reason the head is**: the band is
/// the bottom of a body, so its lower rail is the sole of the foot and its
/// upper rail is the hip. A hair over the hip is the chest.
///
/// Mutants watched red: `>=` → `>` at the lower rail (the feet stop being
/// a leg), `>` → `>=` at `hi > limb_hi` (the hip stops being a leg), and
/// `LIMB_BAND_M` read off the crown rather than off the feet.
#[test]
fn the_leg_band_is_closed_at_both_rails() {
    for (feet_mm, eps) in [(0.0f32, 0.001f32), (400_000.0, 1.0)] {
        let hip = feet_mm + LIMB_BAND_M * 1000.0;
        let at = |y: f32| part_crossed(y, 0.0, feet_mm, 0.0, 1.0);

        assert_eq!(
            at(feet_mm),
            Part::Limb,
            "the sole itself is a leg (at {feet_mm} mm)"
        );
        assert_eq!(
            at(hip),
            Part::Limb,
            "the hip itself is a leg (at {feet_mm} mm)"
        );
        assert_eq!(
            at(feet_mm + LIMB_BAND_M * 500.0),
            Part::Limb,
            "the knee is a leg"
        );
        assert_eq!(
            at(hip + eps),
            Part::Chest,
            "{eps} mm over the hip is the chest"
        );
        // Under the sole is the fallback again, and it is the half a
        // player can actually reach: a shot that passes under a body is
        // refused as a hit long before this is asked (`nearest_body` has
        // its own vertical test), so what this pins is that the predicate
        // does not *invent* a leg out of an altitude below the feet.
        assert_eq!(
            at(feet_mm - eps),
            Part::Chest,
            "{eps} mm under the sole is no part at all"
        );
    }
}

/// §7's inversion, stated at the band that made it necessary: a line that
/// crosses a shin **and** a chest is a chest hit.
///
/// **This is the assertion the whole `Part` type exists for.** With two
/// parts, "most significant part crossed" reduced to a boolean and the
/// only way to get it wrong was to ask about the closest approach. With
/// three, a span can touch two bands neither of which is the head, and
/// first-intersection and most-significant now disagree in the direction
/// that reads as the game cheating — a visible torso saved by a leg in
/// front of it (`reference/PROJECTILES.md` §7 says they inverted this on
/// purpose).
///
/// Mutants watched red: `Part::Limb` returned before the `hi > limb_hi`
/// check (every graze becomes a leg), and the `Ord` derive's variant order
/// reversed (the ladder inverts and this reads `Limb`).
#[test]
fn a_shin_on_the_way_into_a_chest_is_a_chest_hit() {
    let feet_mm = 400_000.0f32;
    let hip = feet_mm + LIMB_BAND_M * 1000.0;
    // Enters at the ankle, leaves a clear 200 mm above the hip.
    let oy = feet_mm + 100.0;
    let sy = hip + 200.0 - oy;
    assert_eq!(
        part_crossed(oy, sy, feet_mm, 0.0, 0.0),
        Part::Limb,
        "the entry alone is a leg, so a first-intersection rule would score one"
    );
    assert_eq!(
        part_crossed(oy, sy, feet_mm, 0.0, 1.0),
        Part::Chest,
        "and the span rule scores the chest the line reached"
    );
    // The same line carried on into the skull is a headshot, not a chest —
    // the ordering is a ladder and not a pair of special cases.
    let sy_head = feet_mm + CAPSULE_HEIGHT_M * 1000.0 - oy;
    assert_eq!(
        part_crossed(oy, sy_head, feet_mm, 0.0, 1.0),
        Part::Head,
        "a shin-to-skull line is a headshot"
    );
}

/// A shot that climbs into the head **on its way through the body** is a
/// headshot even though its closest approach was at the chest.
///
/// **This is `reference/PROJECTILES.md` §7's actual rule** — damage the
/// most significant part *along the line of sight*, not the part at the
/// first or the nearest intersection — and it is the one assertion here
/// that a closest-approach implementation cannot pass. §9.4 states the
/// two-part reduction: with a head and a body, "most significant part
/// crossed" is "was the head interval crossed at all".
///
/// Mutant watched red: the span collapsed to the closest approach
/// (`enter = exit = t`), which is what the code did before `BodyHit`
/// carried a span — this returns false and the check fails.
#[test]
fn a_climb_through_the_body_counts_the_head_it_left_by() {
    let feet_mm = 400_000.0f32;
    let head_lo = feet_mm + (CAPSULE_HEIGHT_M - HEAD_BAND_M) * 1000.0;
    // A ray whose midpoint (the closest approach) is a clear chest hit and
    // whose far end is in the band.
    let oy = head_lo - 500.0;
    let sy = 600.0;
    let mid = oy + sy * 0.5;
    assert!(mid < head_lo, "the closest approach must be a chest hit");
    assert_eq!(
        part_crossed(oy, sy, feet_mm, 0.5, 0.5),
        Part::Chest,
        "and a closest-approach-only rule must call it a chest hit"
    );
    assert_eq!(
        part_crossed(oy, sy, feet_mm, 0.0, 1.0),
        Part::Head,
        "while the span rule sees the head the line left by"
    );
}

/// The world's stop clips the span, so a head behind cover is not credited
/// to a shot that died in the cover.
///
/// **This pins the predicate's contract and NOT the two call sites**, and
/// the module header says so in full: `exit.min(stop_t)` dropped at both
/// damage sites was run as a mutant and **survived**, because reaching it
/// needs a world that stops a rising shot between a chest and a crown.
/// `NOW.md` §0hs carries the fixture that would close it. What is proven
/// here is that a clipped span is refused when one is handed in.
#[test]
fn a_stop_before_the_head_is_not_a_headshot() {
    let feet_mm = 400_000.0f32;
    let head_lo = feet_mm + (CAPSULE_HEIGHT_M - HEAD_BAND_M) * 1000.0;
    let oy = head_lo - 500.0;
    let sy = 600.0;
    // Unclipped, the line reaches the band.
    assert_eq!(part_crossed(oy, sy, feet_mm, 0.0, 1.0), Part::Head);
    // Stopped at the chest, it does not.
    assert_eq!(
        part_crossed(oy, sy, feet_mm, 0.0, 0.4),
        Part::Chest,
        "a span clipped before the band must not be a head"
    );
    // And a stop before the body was even entered is not a hit's question
    // at all — the guard, so a caller's `exit.min(stop_t)` going negative
    // relative to `enter` cannot read as a crossing.
    assert_eq!(
        part_crossed(oy, sy, feet_mm, 0.6, 0.4),
        Part::Chest,
        "an inverted span is not a crossing"
    );
    // The same clip at the other end of the ladder, and it is the half a
    // world can actually produce: a span that starts at a shin and would
    // climb into a chest, stopped inside the leg, is a **leg** hit. The
    // head half needs cover between a chest and a crown; this one needs
    // only the tick boundary an arrow's own segment imposes, which is
    // where `stop_t` is 1.0 by default (`ranged::world_stop`).
    let ankle = feet_mm + 100.0;
    let rise = LIMB_BAND_M * 1000.0 + 400.0;
    assert_eq!(
        part_crossed(ankle, rise, feet_mm, 0.0, 1.0),
        Part::Chest,
        "unclipped, the line reaches the chest"
    );
    assert_eq!(
        part_crossed(ankle, rise, feet_mm, 0.0, 0.4),
        Part::Limb,
        "clipped inside the leg, it is a leg"
    );
}

/// A swing has no head, and that is the sim's answer rather than an
/// oversight.
///
/// **Two bodies at the same feet height, one hit with the melee row's
/// reach.** `combat::strike` resolves feet-to-feet in a plane, so there is
/// no altitude for a band to test; `MeleeDef` has no `headshot_mult` field
/// at all, which is what makes this check a *compile-time* claim as much as
/// a runtime one. If a melee head ever lands, this file is where the
/// decision has to be re-made rather than silently inherited.
///
/// Not mutant-tested, and it cannot usefully be: the claim is the
/// **absence** of a field, which a one-line mutation cannot express — a
/// melee head is a design change, and this check is where it has to be
/// argued rather than inherited.
#[test]
fn a_swing_has_no_head_to_find() {
    let mut cc = fixture();
    const AXE: u16 = 9;
    cc.melee[AXE as usize] = sim_core::combat::MeleeDef {
        damage: 10,
        structure: 0,
        reach_cm: 300,
    };
    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = shooter(1, 0.0, 400.0, 0.0, AXE, NO_ITEM, LEVEL);
    // Standing where a bullet would take the head: same ground, so the
    // attacker's eye is 1.6 m up their 1.7 m body.
    players[1] = target(2, 0.0, 400.0, 1.0);

    let mut events = EventQueue::default();
    let out = combat::strike(
        &cc,
        0,
        &mut players,
        &mut events,
        &sim_core::rewind::Rewind::new(),
        0,
        0,
    );
    assert!(
        !matches!(out, combat::Strike::Missed),
        "the swing must land, or this proves nothing"
    );
    assert_eq!(
        100 - players[1].hp,
        10,
        "melee is the row's own damage: there is no head to double"
    );
}

/// The multiplier saturates rather than wrapping, so a hit can never heal.
///
/// Unreachable with shipped content — `balance.toml` caps a body at three
/// digits and the band is 2 — and asserted anyway, because the reason it is
/// unreachable is a *content* fact and this is a *code* guarantee. A wrap
/// here is a headshot that restores hp, which is the worst shape the bug
/// could take.
///
/// Mutant watched red: `min` dropped from `combat::headshot`, where the
/// product truncates in `u16` instead of saturating.
#[test]
fn the_multiplier_saturates_and_never_wraps() {
    assert_eq!(combat::headshot(20, 2), 40);
    assert_eq!(combat::headshot(20, 1), 20, "the identity is the identity");
    assert_eq!(combat::headshot(0, 2), 0, "nothing doubled is nothing");
    assert_eq!(
        combat::headshot(40_000, 2),
        u16::MAX,
        "and 80 000 saturates rather than becoming 14 464"
    );
    assert_eq!(combat::headshot(u16::MAX, u16::MAX), u16::MAX);
}

/// The other end's arithmetic, and the two edges it has that a multiplier
/// does not: a floor and an identity that is 100 rather than 1.
///
/// **The floor is real and is written down rather than guarded**, which is
/// a decision. `limb(1, 50)` is 0 — a hit that spends a round, pushes
/// `EV_HIT` and moves no hp — and the alternative, a `.max(1)` when the
/// raw is nonzero, would be a number invented in code that no band, no
/// content row and no operator ever spoke. `validate` refuses `damage ==
/// 0` and the smallest shipped weapon is the rock's 20, so the smallest
/// shipped leg hit is 10; the day a one-damage weapon is priced, this line
/// is what says the cost was known.
///
/// Mutants watched red: `/ 100` dropped (the identity becomes ×100),
/// `min` dropped from the clamp (65 535 × 100 wraps), and `pct` and `raw`
/// swapped at the call site in `part_damage`.
#[test]
fn the_leg_percent_floors_and_never_wraps() {
    assert_eq!(combat::limb(20, 50), 10);
    assert_eq!(combat::limb(20, 100), 20, "100 is the identity");
    assert_eq!(combat::limb(0, 50), 0, "nothing halved is nothing");
    assert_eq!(
        combat::limb(1, 50),
        0,
        "and the floor is a zero, on purpose"
    );
    assert_eq!(
        combat::limb(3, 50),
        1,
        "3 halves to 1 and not to 2 — floored"
    );
    assert_eq!(
        combat::limb(u16::MAX, u16::MAX),
        u16::MAX,
        "a pct `validate` refuses still saturates rather than wrapping"
    );
}

/// The one door, and that every rung goes through it.
///
/// **`part_damage` is the reason a third rung did not double the number of
/// places that can disagree.** `ranged` resolves a shot twice — an arrow
/// in `step`, a beam in `hitscan` — and the head multiplier had to be
/// remembered at both; the trap list's own entry for this repo is a verb
/// whose validation and mutation drifted apart across call sites. So the
/// ladder is asserted here as a table, and the two call sites are asserted
/// to *reach* it by `the_three_runs_are_the_ladder_measured_off_the_body`
/// (the beam) and `an_arrow_carries_the_bows_multiplier_off_the_string`
/// (the shaft).
///
/// Mutants watched red: `Part::Chest` routed to `limb` (a chest hit is
/// halved), `Part::Limb` routed to `headshot` (a leg hit is doubled by a
/// percent read as a multiplier — 20 × 50 = 1000), and the `head_mult` and
/// `limb_pct` arguments transposed.
#[test]
fn the_ladder_is_one_door_and_the_chest_is_the_identity() {
    for (part, expect) in [(Part::Head, 40u16), (Part::Chest, 20), (Part::Limb, 10)] {
        assert_eq!(
            combat::part_damage(20, part, 2, 50),
            expect,
            "{part:?} on a 20-damage weapon at ×2 / 50%"
        );
    }
    // The identities together: a weapon that opts out of the ladder at
    // both ends deals its column whatever it hits, which is what the
    // satchel's row says in content.
    for part in [Part::Head, Part::Chest, Part::Limb] {
        assert_eq!(
            combat::part_damage(20, part, 1, 100),
            20,
            "{part:?} with both identities must be the raw column"
        );
    }
    // And the ordering is a ladder, not three unrelated cases.
    assert!(Part::Head > Part::Chest && Part::Chest > Part::Limb);
}

/// The altitude of the sim's own shot line at `dist`, measured by walking a
/// body down through it and taking the last placement that was hit at all.
///
/// **Measured, because the alternative is re-deriving it.** The pitch LUT
/// is private to the crate and would have to be opened to compute this from
/// the encoding, and a test that recomputes the shot's altitude the way the
/// sim computes it is a test of nothing (`CLAUDE.md`'s lattice trap). The
/// last body to be touched is the one whose **crown** the line grazes, so
/// the line is one capsule above those feet, to the centimetre the body's
/// height is stored in.
fn line_alt(cc: &CombatContent, pitch: u8, dist: f32) -> f32 {
    let eye_ground = 400.0f32;
    let steps = (12.0 / POS_Y_Q) as i32;
    let mut last = None;
    for k in 0..steps {
        let feet = eye_ground + 6.0 - k as f32 * POS_Y_Q;
        let mut players = Box::new([Player::default(); MAX_PLAYERS]);
        players[0] = shooter(1, 0.0, eye_ground, 0.0, GUN, ROUND, pitch);
        players[1] = target(2, 0.0, feet, dist);
        pull(cc, &mut players);
        if players[1].hp != 100 {
            last = Some(feet);
        }
    }
    last.expect("the sweep must find the line at all") + CAPSULE_HEIGHT_M
}

/// **The span rule, in the sim rather than in the predicate**: a steeply
/// descending shot that ENTERS through the head and whose closest approach
/// is at the chest is a headshot.
///
/// This is the one check that a closest-approach implementation fails, and
/// it is deliberately an integration check rather than another call to
/// `head_crossed` — `a_climb_through_the_body_counts_the_head_it_left_by`
/// already pins the predicate, and a predicate is only worth what the
/// resolver hands it. Under `nearest_body` returning `(t, t)` for its span
/// — which is exactly what the code did before `BodyHit` — this reads 20.
///
/// **Self-calibrating, and that is the discipline.** Nothing here knows the
/// pitch encoding: two sweeps measure where the sim's own line is at two
/// distances, which gives the slope, and the body is then placed against
/// that measurement. So a mutant in the sim's altitude arithmetic moves the
/// line and the placement with it, and cannot be cancelled out — what it
/// cannot move is the 0.4 m of planar travel between the cylinder's edge
/// and its axis, which is what the whole check turns on.
///
/// The pitch is 96, ~21° down: `CAPSULE_RADIUS_M` of planar travel then
/// buys 0.15 m of altitude, comfortably more than the 0.05 m of margin the
/// placement leaves and than the centimetre a body's height is stored in.
#[test]
fn a_shot_that_enters_through_the_crown_is_a_headshot_at_the_chest() {
    const STEEP: u8 = 96;
    let cc = fixture();
    let (d0, d1) = (5.0f32, 15.0f32);
    let a0 = line_alt(&cc, STEEP, d0);
    let slope = (a0 - line_alt(&cc, STEEP, d1)) / (d1 - d0);
    assert!(
        slope > 0.2,
        "the probe pitch must actually descend, or this proves nothing: {slope:.3}"
    );

    // The line is `CAPSULE_RADIUS_M` of planar travel higher where it
    // crosses the near face of the cylinder than where it passes the axis.
    let entry_alt = a0 + slope * CAPSULE_RADIUS_M;
    // Feet placed so that entry lands 10 cm into the band.
    let feet = entry_alt - (CAPSULE_HEIGHT_M - HEAD_BAND_M) - 0.10;
    let axis_up = a0 - feet;
    let entry_up = entry_alt - feet;
    println!("slope {slope:.3}  entry {entry_up:.3} m up, axis {axis_up:.3} m up");
    assert!(
        entry_up > CAPSULE_HEIGHT_M - HEAD_BAND_M && entry_up <= CAPSULE_HEIGHT_M,
        "the entry must be in the band: {entry_up:.3}"
    );
    assert!(
        axis_up < CAPSULE_HEIGHT_M - HEAD_BAND_M,
        "and the closest approach must not be: {axis_up:.3}"
    );

    let mut players = Box::new([Player::default(); MAX_PLAYERS]);
    players[0] = shooter(1, 0.0, 400.0, 0.0, GUN, ROUND, STEEP);
    players[1] = target(2, 0.0, feet, d0);
    pull(&cc, &mut players);
    assert_eq!(
        100 - players[1].hp,
        40,
        "a shot through the crown is a headshot even though its nearest \
         point to the body was the chest"
    );
}
