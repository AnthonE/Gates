//! Gate: what a landed blow throws, and that it stays inside its bound.
//!
//! **Driven as arithmetic, not through a window** — `tests/tracer.rs`'s stated
//! posture, and this file exists because that one records what happens when
//! you skip it: *"gating the predicate a call site happens to call is gating
//! the wrong thing"*, after a refusal went ungated for a commit because it was
//! a line inside a Bevy system. So `render/impact.rs` keeps the whole of a
//! chip's state and motion on `Chips` — `ignite`, `step`, `draw`, all taking
//! a pool and plain numbers — and the system is a copy into transforms.
//! Everything below runs with no `World`, no GPU and no shard.
//!
//! What no test here claims is that the systems are SCHEDULED;
//! `tests/frame_gates.rs` owns that for the render app as a whole.

#![cfg(feature = "render")]

use bevy::math::Vec3;
use client::render::impact::{
    gather_burst, strike_height, struck, Burst, Chips, Matter, Struck, CHIP_BURST,
    CHIP_GRAVITY_MPS2 as GRAVITY, CHIP_LIFE_S, CHIP_POOL, CHIP_SIZE_M,
};
use client::ui::interact::SwingPick;
use sim_core::terrain::Occupant;

fn burst_at(at: Vec3, away: Vec3) -> Burst {
    Burst {
        at,
        away,
        matter: Matter::Wood,
    }
}

/// Wall 4, on a client-driven path: a fight cannot make this grow.
///
/// The overflow policy is drop-OLDEST and the difference from `tracer.rs`'s
/// drop-newest is the whole point — a chip refused is a chip missing from the
/// blow the player just landed, which is the one they are watching for. So
/// the assertion is not "it stops at the cap", it is **"it stays at the cap
/// and keeps taking bursts"**.
#[test]
fn the_pool_is_bounded_and_says_so() {
    let mut p = Chips::default();
    // Far more bursts than the pool can hold at once.
    for _ in 0..40 {
        p.ignite(&burst_at(Vec3::ZERO, Vec3::Y));
    }
    assert_eq!(p.bursts, 40, "every burst was taken, none refused");
    assert_eq!(
        p.live(),
        CHIP_POOL,
        "a saturated pool draws exactly its cap, never more"
    );
    assert!(
        p.stolen >= 40 * CHIP_BURST as u64 - CHIP_POOL as u64,
        "past the cap every chip has to come from an older one, got {} steals",
        p.stolen
    );
    // **And the newest burst is all there**, which is the half a live count
    // cannot see: a `claim` that recycles the same slot every time keeps the
    // pool full and keeps stealing, so `live()` and `stolen` are both right
    // while seven of the last eight chips are on top of each other. Thrown
    // from a point of its own, so the newest burst is identifiable by
    // position without the pool having to remember which slots it took.
    let mark = Vec3::new(100.0, 0.0, 0.0);
    p.ignite(&burst_at(mark, Vec3::Y));
    let newest = (0..CHIP_POOL)
        .filter_map(|i| p.draw(i).map(|d| d.0))
        .filter(|pos| pos.distance(mark) < 1.0)
        .count();
    assert_eq!(
        newest, CHIP_BURST,
        "a saturated pool drew {newest} of the newest burst's {CHIP_BURST} chips \
         — the overflow policy is stealing from the burst it is meant to be \
         making room for"
    );
    assert_eq!(p.live(), CHIP_POOL, "and it is still exactly at its cap");
}

/// The chips leave the surface rather than going into it.
///
/// `SPRAY` is the cone, and the failure it exists to prevent is invisible in
/// a screenshot and obvious in motion: half a spherical burst is thrown *into*
/// the tree and vanishes, so the effect reads as half as dense as it costs.
#[test]
fn a_burst_is_thrown_out_of_what_it_hit() {
    let at = Vec3::new(10.0, 2.0, -4.0);
    let away = Vec3::new(0.0, 0.0, 1.0);
    let mut p = Chips::default();
    p.ignite(&burst_at(at, away));
    let mut worst = 1.0f32;
    for i in 0..CHIP_POOL {
        let Some((pos, _, _, _)) = p.draw(i) else {
            continue;
        };
        let dir = (pos - at).normalize();
        worst = worst.min(dir.dot(away));
    }
    assert!(
        worst > 0.0,
        "a chip left at {worst:.2} against the surface normal — that one is \
         thrown into the thing that was hit"
    );
}

/// A chip falls, and it retires. Both halves, because they fail differently:
/// no gravity and the burst hangs in the air like a decal, no retirement and
/// the pool fills with debris that never leaves.
#[test]
fn chips_fall_and_then_stop_existing() {
    let at = Vec3::new(0.0, 5.0, 0.0);
    let mut p = Chips::default();
    p.ignite(&burst_at(at, Vec3::Y));
    let live0 = p.live();
    assert_eq!(live0, CHIP_BURST, "one burst is exactly CHIP_BURST chips");

    // A frame at a time, for a third of a chip's life.
    let dt = 1.0 / 60.0;
    let steps = (CHIP_LIFE_S / 3.0 / dt) as usize;
    let before: Vec<Vec3> = (0..CHIP_POOL)
        .filter_map(|i| p.draw(i).map(|d| d.0))
        .collect();
    for _ in 0..steps {
        p.step(dt);
    }
    let after: Vec<Vec3> = (0..CHIP_POOL)
        .filter_map(|i| p.draw(i).map(|d| d.0))
        .collect();
    assert_eq!(
        before.len(),
        after.len(),
        "nothing should retire this early"
    );
    let moved = before
        .iter()
        .zip(&after)
        .map(|(a, b)| a.distance(*b))
        .fold(0.0f32, f32::max);
    assert!(
        moved > 0.05,
        "the fastest chip moved {moved:.3} m in {:.2} s — the burst is not \
         moving",
        steps as f32 * dt
    );
    // **Gravity, measured as a second difference and not against a
    // predicted height.** The first draft compared the burst's rise against
    // `SPEED_MPS * 0.25 * t` — the launch's lift term — and that is not the
    // launch: `ignite` throws each chip along `away` at up to 1.5 × the
    // nominal speed, and this burst's `away` is straight up. So it was a
    // baseline that ignored most of the velocity it was a baseline for, and
    // it failed on correct code. What actually distinguishes free fall from
    // a straight line is that the vertical STEP shrinks, which needs no
    // model of the launch at all.
    let rise = |p: &mut Chips| -> f32 {
        let a: f32 = (0..CHIP_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.0.y))
            .sum();
        p.step(dt);
        let b: f32 = (0..CHIP_POOL)
            .filter_map(|i| p.draw(i).map(|d| d.0.y))
            .sum();
        b - a
    };
    let early = rise(&mut p);
    for _ in 0..10 {
        p.step(dt);
    }
    let late = rise(&mut p);
    assert!(
        late < early,
        "the burst climbs {late:.4} m/frame after {early:.4} — nothing is \
         pulling it down"
    );
    // And the drop between the two is the acceleration, over the frames
    // between them, times the number of chips still drawing.
    let n = p.live() as f32;
    let want = -GRAVITY * dt * dt * 11.0 * n;
    assert!(
        (late - early - want).abs() < want.abs() * 0.25 + 1e-3,
        "the burst lost {:.4} m/frame of climb where {n} chips under gravity \
         lose {want:.4}",
        late - early
    );

    // Everything is gone within its longest life. `ignite` rolls each chip's
    // life up to 1.3 × CHIP_LIFE_S, so this is the bound and not the mean.
    for _ in 0..((CHIP_LIFE_S * 1.4 / dt) as usize) {
        p.step(dt);
    }
    assert_eq!(p.live(), 0, "chips outlived their own lifetime");
    for i in 0..CHIP_POOL {
        assert!(p.draw(i).is_none());
    }
}

/// A chip shrinks out instead of popping, and it starts at full size.
#[test]
fn a_chip_shrinks_out_rather_than_vanishing() {
    let mut p = Chips::default();
    p.ignite(&burst_at(Vec3::ZERO, Vec3::Y));
    let first = (0..CHIP_POOL).find(|&i| p.draw(i).is_some()).unwrap();
    assert!(
        (p.draw(first).unwrap().2 - CHIP_SIZE_M).abs() < 1e-6,
        "a chip is born at full size"
    );
    let dt = 1.0 / 60.0;
    let mut last = CHIP_SIZE_M;
    let mut shrank = false;
    // **Bounded, and the bound is an assertion rather than a `break`.** The
    // first draft was `while let Some(..)`, which under a mutant that never
    // decrements a chip's clock is not a failure — it is a hang, and a hang
    // is the one test outcome nobody reads.
    let cap = (CHIP_LIFE_S * 3.0 / dt) as usize;
    let mut frames = 0;
    while let Some((_, _, scale, _)) = p.draw(first) {
        frames += 1;
        assert!(
            frames < cap,
            "the chip is still drawing after {frames} frames — it never retires"
        );
        assert!(scale <= last + 1e-6, "the chip grew: {scale} after {last}");
        if scale < last - 1e-6 {
            shrank = true;
        }
        last = scale;
        p.step(dt);
    }
    assert!(shrank, "the chip never shrank — it popped out of existence");
    assert!(
        last < CHIP_SIZE_M * 0.35,
        "the last drawn size was {last:.4} against {CHIP_SIZE_M:.4} — that is a pop"
    );
}

/// Two bursts from the same point are not the same eight chips.
///
/// The tell of a canned effect, and it is one line away: seeding the roll per
/// burst instead of per pool makes every impact in the game identical.
#[test]
fn two_bursts_are_not_the_same_burst() {
    let at = Vec3::new(3.0, 1.0, 3.0);
    let mut p = Chips::default();
    p.ignite(&burst_at(at, Vec3::Y));
    let first: Vec<Vec3> = (0..CHIP_POOL)
        .filter_map(|i| p.draw(i).map(|d| d.0))
        .collect();
    let mut q = Chips::default();
    q.ignite(&burst_at(at, Vec3::Y));
    q.ignite(&burst_at(at, Vec3::Y));
    let both: Vec<Vec3> = (0..CHIP_POOL)
        .filter_map(|i| q.draw(i).map(|d| d.0))
        .collect();
    assert_eq!(both.len(), CHIP_BURST * 2);
    // The second burst's chips are somewhere the first's are not.
    let repeats = both
        .iter()
        .filter(|b| first.iter().any(|f| f.distance(**b) < 1e-6))
        .count();
    assert!(
        repeats <= CHIP_BURST,
        "{repeats} of {} chips repeat the first burst exactly — the roll is \
         being re-seeded per burst",
        both.len()
    );
}

/// Every occupant a swing can take answers with the matter it is made of.
///
/// **Read off `interact`'s own predicate rather than a list retyped here**,
/// which is the hand-kept-mirror trap: a seventh swingable node added to the
/// sim would otherwise spray dirt with every gate green.
#[test]
fn every_swingable_node_has_a_matter() {
    for (o, want) in [
        (Occupant::Tree, Matter::Wood),
        (Occupant::Bush, Matter::Plant),
        (Occupant::StoneNode, Matter::Stone),
        (Occupant::MetalNode, Matter::Metal),
        (Occupant::SulfurNode, Matter::Stone),
        (Occupant::BarrelSlot, Matter::Metal),
    ] {
        assert_eq!(
            Matter::of_occupant(o as u8),
            want,
            "{o:?} throws the wrong debris"
        );
    }
    // The swingable set, as `ui::interact::swing_label` names it — its
    // labels are non-empty for exactly the nodes a swing takes, so this
    // reads the predicate instead of copying it.
    for o in 0u8..16 {
        if client::ui::interact::swing_label(o).is_empty() {
            continue;
        }
        assert_ne!(
            Matter::of_occupant(o),
            Matter::Dirt,
            "occupant {o} is swingable and throws the default debris — \
             `Matter::of_occupant` has not followed the scatter"
        );
    }
}

/// **A blow on an animal throws chips too, and it did not for one commit.**
///
/// `EV_HIT` carries a victim id and nothing else, and this client draws
/// players out of `bodies::Bodies` and animals out of `mobs::Herd` — two
/// stores, one id space, split by `mob::slot_of_id`. The first cut of
/// `impact::strike` looked in `Bodies` alone, so a wolf took a hatchet and
/// the frame was unchanged. Nothing was red: a miss and an unrecognised
/// victim reach the same `continue`.
///
/// Driven through `struck`, which is the decision split out of the system
/// for `tests/tracer.rs`' stated reason — a rule only a Bevy system can
/// reach is a rule nothing holds.
#[test]
fn a_blow_on_an_animal_is_a_different_store_from_a_blow_on_a_player() {
    // A plain wire id is a player.
    assert!(matches!(struck(7), Struck::Player { .. }));
    assert!(matches!(struck(0), Struck::Player { .. }));

    // Every roster slot the sim can name is an animal, and each answers with
    // its own slot back — read through `mob_id`/`slot_of_id` rather than by
    // reconstructing the tag here, which would be this file keeping its own
    // copy of a bit layout (`CLAUDE.md`'s positional-payload trap).
    for slot in 0..sim_core::limits::MAX_MOBS {
        let id = sim_core::mob::mob_id(slot);
        assert!(
            sim_core::mob::slot_of_id(id) == Some(slot),
            "the fixture's own id round-trip is broken at slot {slot}"
        );
        match struck(id) {
            Struck::Animal { slot: got, lift } => {
                assert_eq!(got, slot);
                assert!(
                    lift > 0.0,
                    "slot {slot} lands a blow at or below the ground"
                );
            }
            other => panic!("mob {slot} resolved as {other:?}"),
        }
    }
}

/// A blow lands ON the animal, not over it or under it.
///
/// The lift is read off the shipped mesh table (`mobs::flank_h_of`) rather
/// than taken as a fraction of a height constant, so this checks the read
/// against the other numbers that describe the same animal.
#[test]
fn the_flank_a_blow_lands_on_is_inside_the_animal() {
    use client::render::mobs::{flank_h_of, PIG_H_M, WOLF_H_M};
    for slot in 0..sim_core::limits::MAX_MOBS {
        let h = flank_h_of(slot);
        let stand = match sim_core::mob::kind_of(slot) {
            sim_core::mob::MOB_WOLF => WOLF_H_M,
            _ => PIG_H_M,
        };
        assert!(
            h > stand * 0.25 && h < stand,
            "slot {slot}: a blow lands {h:.3} m up an animal that stands \
             {stand:.3} m — that is off the body"
        );
    }
    // The two species differ, which is the whole reason this is per-slot: a
    // wolf's chest is higher than a boar's barrel, and one constant for both
    // would put every blow on the wrong one of them.
    let wolf = (0..sim_core::limits::MAX_MOBS)
        .find(|&s| sim_core::mob::kind_of(s) == sim_core::mob::MOB_WOLF)
        .expect("the roster has a wolf");
    let pig = (0..sim_core::limits::MAX_MOBS)
        .find(|&s| sim_core::mob::kind_of(s) != sim_core::mob::MOB_WOLF)
        .expect("the roster has a pig");
    assert!(
        (flank_h_of(wolf) - flank_h_of(pig)).abs() > 0.01,
        "both species take a blow at the same height — flank_h_of has stopped \
         reading the per-species table"
    );
}

/// Both victim classes throw the same matter, and it is not the default.
#[test]
fn anything_alive_throws_flesh() {
    let mut p = Chips::default();
    for at in [Vec3::new(1.0, 1.0, 1.0), Vec3::new(-4.0, 0.5, 2.0)] {
        p.ignite(&Burst {
            at,
            away: Vec3::Y,
            matter: Matter::Flesh,
        });
    }
    let flesh = (0..CHIP_POOL)
        .filter_map(|i| p.draw(i))
        .filter(|d| d.3 == Matter::Flesh)
        .count();
    assert_eq!(flesh, CHIP_BURST * 2);
    assert_ne!(
        Matter::Flesh.color(),
        Matter::Dirt.color(),
        "a body and the ground throw the same colour, so a hit reads as a miss"
    );
}

/// **A burst is a fact about CONTACT, not about income** — and the trigger it
/// replaced was a fact about income.
///
/// `EV_GATHER` announces a backpack loot the same way it announces a node
/// paying out (its own doc says so, and deliberately), so the first cut fired
/// wood chips off whatever the crosshair was near, once per slot, while a
/// player emptied a corpse's pack. What decides now is the sim's `EV_SWING` —
/// pushed at `gather::swing`'s cadence gate, once per swing, before the scan.
#[test]
fn only_a_swing_throws_chips_and_a_payout_does_not() {
    let mut pick = SwingPick {
        occupant: Occupant::Tree as u8,
        cx: 40,
        cz: 90,
        x: 10.0,
        y: 2.0,
        z: -4.0,
        ..SwingPick::default()
    };
    // The whole point: no swing, no chips — however much arrived.
    assert!(
        gather_burst(false, &pick).is_none(),
        "a payout with no swing behind it throws debris — that is the \
         backpack-loot defect back"
    );
    // A swing at open air throws nothing either, because the pick is what
    // says whether anything was in reach.
    let empty = SwingPick::default();
    assert!(gather_burst(true, &empty).is_none());

    // A swing at a node throws its own matter, from the node's own place.
    let b = gather_burst(true, &pick).expect("a swing at a tree throws chips");
    assert_eq!(b.matter, Matter::Wood);
    assert!(
        (b.at.x - pick.x).abs() < 1e-6 && (b.at.z - pick.z).abs() < 1e-6,
        "the burst is not at the node the pick named"
    );
    assert!(
        b.at.y > pick.y,
        "the chips come off the ground under the tree rather than the trunk"
    );

    // And what it is made of follows the occupant, not the position.
    pick.occupant = Occupant::StoneNode as u8;
    assert_eq!(
        gather_burst(true, &pick).expect("a swing at stone").matter,
        Matter::Stone
    );
}

/// The strike height puts the chips on the thing, not in the grass under it.
#[test]
fn a_node_is_struck_where_a_person_could_reach_it() {
    // A tree is chopped at chest height and a stone node at the knee; one
    // constant for both puts half the bursts in the wrong place.
    let tree = strike_height(Occupant::Tree as u8);
    let stone = strike_height(Occupant::StoneNode as u8);
    assert!(
        tree > stone,
        "a tree is struck at {tree:.2} m and a stone node at {stone:.2} — \
         the taller thing has to be struck higher"
    );
    for o in 0u8..16 {
        if client::ui::interact::swing_label(o).is_empty() {
            continue;
        }
        let h = strike_height(o);
        assert!(
            (0.2..=1.8).contains(&h),
            "occupant {o} is struck {h:.2} m up — that is under the grass or \
             over a person's head"
        );
    }
}

/// Leaving a world takes the debris with it.
#[test]
fn the_pool_empties_on_the_way_out() {
    let mut p = Chips::default();
    for _ in 0..3 {
        p.ignite(&burst_at(Vec3::ZERO, Vec3::Y));
    }
    assert!(p.live() > 0);
    // `forget` is the Bevy half; this is the state half it wraps, and the
    // system is gated to agree with it by `tests/frame_gates.rs`.
    p = Chips::default();
    assert_eq!(p.live(), 0);
    assert_eq!(p.bursts, 0);
}
