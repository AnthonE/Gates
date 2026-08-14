//! Site guards — the destination costs something to reach.
//!
//! The merge-gate judge ranked this first twice (`findings/
//! pass-20260813-230343-{03,05}-judge.md`): the road → barrel → cache →
//! crate gradient was fully built, fully gated and *uncontested*, so the
//! pad was a walk priced at time alone and a player who knew where the
//! crates were could clear them on a timer forever.
//!
//! A guard is not a species. It is a **wolf slot whose home is inside an
//! authored site and whose leash is that site's `SiteFootprint` instead of
//! its species' `roam_cm`** — two facts, both pure functions of the slot
//! ordinal, which is why nothing crossed the wire and nothing changed in
//! the client for this to land.
//!
//! What this file gates, in the order the mechanism reads:
//!   §A the roster spends a stated number of slots on guards, and they are
//!      predator slots
//!   §B a guard homes on its site's swept apron — inside the leash, outside
//!      the structures — and the free roster still never does
//!   §C the leash is the site's radius and not the species', and the
//!      difference is what keeps a guard on its pad
//!   §D a guard holds the crates: it hatches within reach of the ring it
//!      keeps
//!   §E an unfilled waystation has no guard rather than a guard nowhere
//!   §F a guard is worth killing and its death is the same body every other
//!      animal leaves

use sim_core::limits::{MAX_MOBS, MAX_PLAYERS, MOB_THINK_TICKS};
use sim_core::mob::{self, MobContent, SITE_GUARDS};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::terrain;
use sim_core::world::{Command, World};

const SEEDS: [u64; 5] = [1, 7, 42, 99, 2026];

fn home_xz(w: &World, slot: usize) -> (f32, f32) {
    let m = &w.mobs.m[slot];
    (m.home_qx as f32 * POS_XZ_Q, m.home_qz as f32 * POS_XZ_Q)
}

/// Site centre for a guard's site index, or `None` when the solver left the
/// waystation unfilled. Mirrors `guard_home_of`'s own lookup; the test owns
/// a second copy on purpose, so a change to the sim's has to be argued for
/// here too.
fn site_centre(w: &World, site: usize) -> Option<(f32, f32, terrain::SiteFootprint)> {
    if site == 0 {
        return Some((w.haven.x, w.haven.z, terrain::HAVEN_FOOTPRINT));
    }
    let ws = &w.haven.minor[site - 1];
    ws.live
        .then_some((ws.x, ws.z, terrain::WAYSTATION_FOOTPRINT))
}

// ---------------------------------------------------------------- §A

/// The guard count is a number, not a distribution. Wall 4's bounded form:
/// a gate asserts it rather than sampling for it.
#[test]
fn the_roster_spends_a_stated_number_of_slots_on_guards() {
    let claimed: Vec<usize> = (0..MAX_MOBS)
        .filter(|&s| mob::guard_site_of(s).is_some())
        .collect();
    assert_eq!(
        claimed.len(),
        SITE_GUARDS,
        "guard slots ({}) disagree with SITE_GUARDS ({SITE_GUARDS}) — the \
         constant is what DECISIONS.md §open registers and what every other \
         assertion here counts against",
        claimed.len()
    );
    // Every site the world can author is covered, and none twice over the
    // stated per-site count.
    let sites = 1 + terrain::WAYSTATIONS;
    for site in 0..sites {
        let n = claimed
            .iter()
            .filter(|&&s| mob::guard_site_of(s) == Some(site))
            .count();
        assert_eq!(
            n,
            SITE_GUARDS / sites,
            "site {site} keeps {n} guards and every other site keeps {}",
            SITE_GUARDS / sites
        );
    }
}

/// A guard must be a predator or it is scenery: `brave_pct = 0` is the whole
/// of what makes a wolf charge instead of flee, so a guard drawn from a prey
/// slot would run away from the player it is standing there to stop.
#[test]
fn every_guard_slot_is_a_predator_slot() {
    for slot in 0..MAX_MOBS {
        if mob::guard_site_of(slot).is_none() {
            continue;
        }
        assert_eq!(
            mob::kind_of(slot),
            mob::MOB_WOLF,
            "slot {slot} guards a site but is not a wolf — a guard that \
             flees is a gift, not a cost"
        );
    }
    // And the free roster keeps enough hunters to still be a roster.
    let free_wolves = (0..MAX_MOBS)
        .filter(|&s| mob::kind_of(s) == mob::MOB_WOLF && mob::guard_site_of(s).is_none())
        .count();
    assert!(
        free_wolves > 0,
        "every wolf on the island is standing at a site — the open ground \
         has no predator left in it"
    );
}

// ---------------------------------------------------------------- §B

/// The apron: inside the leash, outside the structures. Both bounds matter
/// and each fails a different way — outside `scatter_m` the guard walks off
/// on its first think, inside `swept_m` it hatches in a crate.
#[test]
fn a_guard_homes_on_its_sites_swept_apron() {
    for seed in SEEDS {
        let w = World::new(seed);
        for slot in 0..MAX_MOBS {
            let Some(site) = mob::guard_site_of(slot) else {
                continue;
            };
            let Some((sx, sz, fp)) = site_centre(&w, site) else {
                continue; // §E owns the unfilled case
            };
            assert!(
                w.mobs.m[slot].homed,
                "seed {seed}: slot {slot} guards live site {site} and found \
                 nowhere to stand — the apron is {:.1}..{:.1} m",
                fp.swept_m, fp.scatter_m
            );
            let (x, z) = home_xz(&w, slot);
            let d = ((x - sx) * (x - sx) + (z - sz) * (z - sz)).sqrt();
            assert!(
                d >= fp.swept_m - 0.05,
                "seed {seed}: guard {slot} stands {d:.2} m from site {site}, \
                 inside the swept floor ({:.2} m) — that is where the crates \
                 and the shelter are",
                fp.swept_m
            );
            assert!(
                d <= fp.scatter_m + 0.05,
                "seed {seed}: guard {slot} stands {d:.2} m from site {site}, \
                 past its own leash ({:.2} m) — it would walk away on its \
                 first think",
                fp.scatter_m
            );
            // The land line, which is the guard's floor and deliberately
            // not the free roster's beach band: a pad scored for low relief
            // near the coast road lands inside that band on real seeds
            // (seed 1's centre is 1.28 m against 2.0), and `tests/mob.rs`
            // still holds every non-guard home to the wider margin.
            assert!(
                terrain::height(seed, x, z) > terrain::LAND_MIN_H,
                "seed {seed}: guard {slot} is standing at the water's edge"
            );
            assert!(terrain::slope(seed, x, z) < 1.0);
        }
    }
}

/// The pad is guarded on every seed. This is the claim the whole slice is
/// about, so it is asserted about the *haven* specifically rather than
/// falling out of the loop above: a seed whose waystations both failed
/// would still have to guard its destination.
#[test]
fn the_haven_pad_is_guarded_on_every_seed() {
    for seed in SEEDS {
        let w = World::new(seed);
        let kept = (0..MAX_MOBS)
            .filter(|&s| mob::guard_site_of(s) == Some(0) && w.mobs.m[s].homed)
            .count();
        assert!(
            kept > 0,
            "seed {seed}: nothing stands at the haven pad — the destination \
             is a walk again"
        );
    }
}

/// The complement of `tests/mob.rs`'s narrowed assertion: the exception is
/// exactly the guards and nothing else drifted into a site.
#[test]
fn only_a_guard_stands_in_a_site() {
    for seed in SEEDS {
        let w = World::new(seed);
        for slot in 0..MAX_MOBS {
            if !w.mobs.m[slot].homed || mob::guard_site_of(slot).is_some() {
                continue;
            }
            let (x, z) = home_xz(&w, slot);
            assert!(
                !terrain::in_haven(&w.haven, x, z) && !terrain::in_waystation(&w.haven, x, z),
                "seed {seed}: free-roster slot {slot} homed inside an \
                 authored site at ({x:.1}, {z:.1})"
            );
        }
    }
}

// ---------------------------------------------------------------- §C

/// The leash is the site's radius, not the species'. A wolf roams 90 m and
/// the pad is 16 across: on the species number a guard would be somewhere
/// else most of the time, which is the same as no guard.
#[test]
fn a_guards_leash_is_its_site_and_not_its_species() {
    let mc = MobContent::probe_fixture();
    let wolf = mc.def(mob::MOB_WOLF);
    let mut guards = 0;
    for slot in 0..MAX_MOBS {
        let Some(site) = mob::guard_site_of(slot) else {
            assert_eq!(
                mob::guard_leash_cm(slot),
                None,
                "slot {slot} is not a guard but was handed a site leash"
            );
            continue;
        };
        guards += 1;
        let leash = mob::guard_leash_cm(slot).expect("a guard has a leash");
        let fp = if site == 0 {
            terrain::HAVEN_FOOTPRINT
        } else {
            terrain::WAYSTATION_FOOTPRINT
        };
        assert_eq!(
            leash,
            (fp.scatter_m * 100.0) as i64,
            "slot {slot}'s leash is not site {site}'s scatter radius"
        );
        assert!(
            leash < wolf.roam_cm,
            "slot {slot}'s site leash ({leash} cm) is no tighter than the \
             species roam ({} cm) — binding it to the footprint bought \
             nothing",
            wolf.roam_cm
        );
    }
    assert_eq!(guards, SITE_GUARDS);
}

/// The leash holds the guard *near the crates it keeps*, which is the
/// property the numbers have to produce and not just an ordering between
/// two constants: the whole disc a guard may wander over has to stay inside
/// reach of the container ring.
#[test]
fn the_leash_keeps_a_guard_within_reach_of_the_ring_it_keeps() {
    for seed in SEEDS {
        let w = World::new(seed);
        for slot in 0..MAX_MOBS {
            let Some(site) = mob::guard_site_of(slot) else {
                continue;
            };
            let Some((sx, sz, fp)) = site_centre(&w, site) else {
                continue;
            };
            if !w.mobs.m[slot].homed {
                continue;
            }
            let (hx, hz) = home_xz(&w, slot);
            let leash_m = mob::guard_leash_cm(slot).expect("a guard has a leash") as f32 * 0.01;
            // Worst case: home on the apron's outer edge, then wander a
            // full leash further out. Even then it is on its own site.
            let d_home = ((hx - sx) * (hx - sx) + (hz - sz) * (hz - sz)).sqrt();
            let worst = d_home + leash_m;
            assert!(
                worst <= fp.scatter_m * 2.0 + 0.1,
                "seed {seed}: guard {slot} can stand {worst:.1} m out from \
                 site {site}, more than two site radii ({:.1} m) — a player \
                 could open the crates with the guard out of the picture",
                fp.scatter_m * 2.0
            );
        }
    }
}

/// The leash **in motion**, which is the assertion that actually gates the
/// sim change rather than the constant behind it: a guard dragged off its
/// site walks back onto it.
///
/// The displacement is chosen to separate the two candidate rules — 40 m is
/// past the pad's 16 m leash and short of the wolf's own roam, so a guard
/// running on `MobDef::roam_cm` would simply stay out there and graze. Both
/// bounds are asserted before the world is ticked, so the test fails loudly
/// if a content change ever collapses the gap it depends on.
#[test]
fn a_guard_dragged_off_its_site_walks_back_onto_it() {
    const OUT_M: f32 = 40.0;
    let seed = 7;
    let mut w = World::new(seed);
    w.mob = MobContent::probe_fixture();
    w.tick(&[]);

    let slot = (0..MAX_MOBS)
        .find(|&s| mob::guard_site_of(s) == Some(0) && w.mobs.m[s].homed)
        .expect("seed 7's pad keeps a guard");
    let leash_cm = mob::guard_leash_cm(slot).expect("a guard has a leash");
    let roam_cm = w.mob.def(mob::MOB_WOLF).roam_cm;
    assert!(
        (OUT_M * 100.0) as i64 > leash_cm,
        "the displacement is inside the site leash — this test proves nothing"
    );
    assert!(
        (OUT_M * 100.0) as i64 <= roam_cm,
        "the displacement is past the species roam too, so both rules agree \
         here and the test cannot tell them apart"
    );

    // A player far enough to keep the roster awake (`MOB_WAKE_CM`, 240 m)
    // and far enough not to rouse it (the wolf's spook, 30 m) — a roused
    // guard skips the leash entirely and would be chasing, not homing.
    w.dev_spawn = Some((w.haven.x + 100.0, w.haven.z));
    w.tick(&[Command::Join { id: 1 }]);

    let (hx, hz) = home_xz(&w, slot);
    w.mobs.m[slot].body = Body::at(seed, hx - OUT_M, hz);
    let start = {
        let m = &w.mobs.m[slot];
        let dx = (m.body.qx - m.home_qx) as i64 * 3;
        let dz = (m.body.qz - m.home_qz) as i64 * 3;
        dx * dx + dz * dz
    };
    assert!(start > leash_cm * leash_cm, "the drag did not take");

    // Two think ticks of overshoot — the leash is a decision, not a wall,
    // so the animal turns for home on its next think. The bound is
    // `tests/mob.rs::the_leash_holds`'s, derived there rather than chosen.
    let slack = 2 * (MOB_THINK_TICKS as i64) * 10;
    let bound = (leash_cm + slack) * (leash_cm + slack);
    let d2 = |w: &World| -> i64 {
        let m = &w.mobs.m[slot];
        let dx = (m.body.qx - m.home_qx) as i64 * 3;
        let dz = (m.body.qz - m.home_qz) as i64 * 3;
        dx * dx + dz * dz
    };

    // It beelines: outside the leash, `think` steers home and returns
    // before it can draw a wander at all. 40 m at the wolf's walk is well
    // inside this budget.
    for _ in 0..3_000 {
        w.tick(&[]);
    }
    assert!(
        d2(&w) <= bound,
        "a guard dragged {OUT_M} m off its pad never came back inside its \
         {leash_cm} cm leash — it is still {} cm² out, which is what a \
         guard running on the species' {roam_cm} cm roam would do",
        d2(&w)
    );

    // And then it STAYS, which is the half that makes it a guard rather
    // than an animal that happened to walk past. A random wander alone can
    // re-enter a 16 m disc by luck over a long run — it cannot stay inside
    // one for three thousand ticks with a 90 m roam permitting it to leave.
    for _ in 0..6_000 {
        w.tick(&[]);
        let m = &w.mobs.m[slot];
        assert!(m.roused_until <= w.tick, "the guard was roused");
        assert!(
            d2(&w) <= bound,
            "a guard wandered {} cm² from its post, past its {leash_cm} cm \
             site leash — the pad is unattended",
            d2(&w)
        );
    }
}

// ---------------------------------------------------------------- §D

/// A guard is standing there when the world starts ticking, and it is
/// standing on its site — the hatch re-derives the body from the stored
/// home, so this is the mechanism `hatch` shares with every other animal
/// checked at the one place it matters.
#[test]
fn a_guard_hatches_at_its_post() {
    for seed in SEEDS {
        let mut w = World::new(seed);
        w.mob = MobContent::probe_fixture();
        for _ in 0..4 {
            w.tick(&[]);
        }
        let mut standing = 0;
        for slot in 0..MAX_MOBS {
            if mob::guard_site_of(slot) != Some(0) || !w.mobs.m[slot].homed {
                continue;
            }
            let m = &w.mobs.m[slot];
            assert!(m.alive, "seed {seed}: haven guard {slot} never hatched");
            let x = m.body.qx as f32 * POS_XZ_Q;
            let z = m.body.qz as f32 * POS_XZ_Q;
            let d = ((x - w.haven.x) * (x - w.haven.x) + (z - w.haven.z) * (z - w.haven.z)).sqrt();
            assert!(
                d <= terrain::HAVEN_FOOTPRINT.scatter_m + 1.0,
                "seed {seed}: haven guard {slot} hatched {d:.1} m from the \
                 pad it keeps"
            );
            standing += 1;
        }
        assert!(standing > 0, "seed {seed}: the pad hatched no guard");
    }
}

// ---------------------------------------------------------------- §E

/// An unfilled waystation has no guard, rather than a guard at the origin
/// or a guard on a site that is not there. Same honest empty a slot that
/// found no land gets, and `step` skips both by `homed`.
#[test]
fn an_unfilled_waystation_keeps_no_guard() {
    for seed in SEEDS {
        let w = World::new(seed);
        for site in 1..=terrain::WAYSTATIONS {
            if w.haven.minor[site - 1].live {
                continue;
            }
            for slot in 0..MAX_MOBS {
                if mob::guard_site_of(slot) != Some(site) {
                    continue;
                }
                assert!(
                    !w.mobs.m[slot].homed,
                    "seed {seed}: slot {slot} guards waystation {site}, \
                     which the solver never placed"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- §F

/// Killing a guard leaves the same body every other animal leaves — the
/// wolf's own drops, standing up as a ground bag. Stated as a gate because
/// it is the honest half of this slice: a guard has **no loot tier of its
/// own** yet, so what it pays is a wolf's meat and fat and the reason to
/// fight it is the crates behind it (`NOW.md` §0wc).
#[test]
fn a_guard_pays_what_a_wolf_pays() {
    let mc = MobContent::probe_fixture();
    let wolf = mc.def(mob::MOB_WOLF);
    for slot in 0..MAX_MOBS {
        if mob::guard_site_of(slot).is_none() {
            continue;
        }
        assert_eq!(
            mc.def(mob::kind_of(slot)).loot,
            wolf.loot,
            "slot {slot} guards a site and does not drop what a wolf drops \
             — if that is now deliberate, this gate is the thing to rewrite"
        );
        assert!(
            wolf.hp > 0 && wolf.attack > 0,
            "the guard's species cannot fight or cannot be killed"
        );
    }
}

/// Guards do not change what the roster costs. The store is fixed at
/// `MAX_MOBS` and a guard is a slot out of it, never an addition — the
/// thing wall 4 wants said out loud at the one site that could have grown.
#[test]
fn guards_are_taken_out_of_the_roster_and_not_added_to_it() {
    for seed in SEEDS {
        let w = World::new(seed);
        assert_eq!(w.mobs.m.len(), MAX_MOBS);
        assert!(w.mobs.homed() <= MAX_MOBS);
        // Nothing here should have cost the free roster its population: the
        // guards were already wolf slots, so the island's animal count is
        // unchanged and only six homes moved.
        assert!(
            w.mobs.homed() >= MAX_MOBS - SITE_GUARDS - 2,
            "seed {seed}: {} of {MAX_MOBS} slots found a home — guards \
             should have moved homes, not lost them",
            w.mobs.homed()
        );
    }
}

/// The roster still fits the world's other bound: a guard is an entity id
/// like any other and nothing here reaches past the player store.
#[test]
fn the_guard_slots_are_addressable_like_any_other() {
    for slot in 0..MAX_MOBS {
        if mob::guard_site_of(slot).is_none() {
            continue;
        }
        let id = mob::mob_id(slot);
        assert_eq!(mob::slot_of_id(id), Some(slot));
        assert!(slot < MAX_MOBS && MAX_PLAYERS > 0);
    }
}
