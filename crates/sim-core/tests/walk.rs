//! Walking into the world and being stopped by it — the caller side of
//! `tests/solid.rs`.
//!
//! `solid.rs` gates the shapes and the per-slot predicate and says in its own
//! header that it "deliberately does NOT test anybody calling this". This
//! suite is that missing half: it drives real bodies through
//! `movement::step` and asserts they stop, which is the only statement that
//! makes a forest cover rather than decoration.
//!
//! Every check here walks with an EMPTY `ColIndex`. Nothing is built, so the
//! only thing that can stop a body is a scattered occupant — a failure here
//! cannot be a wall taking the credit.

// The measurements are the gate's output, same allow and same reason as
// `tests/solid.rs`: the L5 wall bans format/print in SIM code, and a test
// harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::collide::{ColIndex, CAPSULE_RADIUS_M};
use sim_core::input::{InputFrame, BTN_SPRINT};
use sim_core::movement::{self, Body, POS_XZ_Q};
use sim_core::occupy::{Harvested, Occupants, Pristine, Scratch, SlotCache};
use sim_core::terrain::{self, Occupant, ScatterTable, Slot, CELLS_PER_SIDE};
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

/// Seeds every search runs over — same set and same rule as `solid.rs`: a
/// seed that fails is a bug in the generator, not a reroll.
const SEEDS: [u64; 4] = [0, 1, 7, 12345];

/// Long enough for a body to cross the standoff at walking pace and then keep
/// pushing for several seconds, so "it stopped" means stopped and not
/// "still travelling when we stopped looking".
const TICKS: usize = 200;

/// How far from a slot a walk starts. Comfortably outside every occupant's
/// reach (the widest scattered one is `Rock` at 1.5 m) and inside the same
/// cell neighbourhood, so the walk is a walk and not a commute.
const STANDOFF_M: f32 = 4.0;

fn pos(b: &Body) -> (f32, f32) {
    (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q)
}

fn dist(b: &Body, s: &Slot) -> f32 {
    let (x, z) = pos(b);
    let (dx, dz) = (x - s.x, z - s.z);
    (dx * dx + dz * dz).sqrt()
}

/// Full-tilt forward on the given yaw, sprinting — the fastest a body can
/// approach anything, which is the case a destination-point test has to
/// survive without tunnelling.
fn charge(yaw: u16) -> InputFrame {
    InputFrame {
        seq: 1,
        buttons: BTN_SPRINT,
        yaw,
        move_z: 127,
        ..InputFrame::default()
    }
}

/// The first slot of `want` this seed draws, scanning cells in a fixed order
/// so the fixture is reproducible. Skips anything near the haven pad, whose
/// shelter and crates would confuse "what stopped the body".
fn find(
    seed: u64,
    table: &ScatterTable,
    haven: &terrain::Haven,
    want: Occupant,
) -> (i32, i32, Slot) {
    let mut cz = 0;
    while cz < CELLS_PER_SIDE {
        let mut cx = 0;
        while cx < CELLS_PER_SIDE {
            let s = terrain::scatter(seed, table, haven, cx, cz);
            if s.occupant == want {
                let (dx, dz) = (s.x - haven.x, s.z - haven.z);
                if dx * dx + dz * dz > 64.0 * 64.0 {
                    return (cx, cz, s);
                }
            }
            cx += 1;
        }
        cz += 1;
    }
    panic!("seed {seed} drew no {want:?} anywhere on the island");
}

/// Walk at `s` from bearing `k` of four and report where the body ended up.
fn charge_at(seed: u64, occ: &mut Occupants, s: &Slot, k: u16) -> Body {
    let cols = ColIndex::new();
    let yaw = (k * 64) << 8;
    let (fx, fz) = yaw_dir(yaw);
    // Start a standoff back along the bearing, aimed at the slot.
    let mut b = Body::at(seed, hv(seed), s.x - fx * STANDOFF_M, s.z - fz * STANDOFF_M);
    let f = charge(yaw);
    for _ in 0..TICKS {
        movement::step(seed, hv(seed), &cols, occ, &mut b, &f);
    }
    b
}

/// The radius at which a slot stops a capsule.
fn reach(s: &Slot) -> f32 {
    let (r, _) = terrain::occupant_volume(s.occupant);
    r * s.scale + CAPSULE_RADIUS_M
}

/// Walk at a trunk from four bearings and stop — the gate the merge-gate
/// judge asked for by name, and the whole point of the slice.
///
/// Sprinting, because a destination-point test only holds while one tick's
/// travel is shorter than the blocking region is wide; `occupy.rs`'s const
/// block asserts that bound and this is the same claim at runtime, on real
/// worldgen, from every side.
#[test]
fn a_body_stops_at_a_trunk_from_every_bearing() {
    for seed in SEEDS {
        let mut sc = Scratch::live(seed);
        let (_, _, tree) = find(seed, &sc.table, &sc.haven, Occupant::Tree);
        let stop = reach(&tree);
        for k in 0..4u16 {
            let b = charge_at(seed, &mut sc.occupants(), &tree, k);
            let d = dist(&b, &tree);
            // Stopped OUTSIDE the trunk. One position quantum of slack: the
            // body's resting place is a rounded value of a point that was
            // itself strictly outside.
            assert!(
                d >= stop - POS_XZ_Q,
                "seed {seed} bearing {k}: body is {d:.3} m from the trunk, \
                 inside its {stop:.3} m reach — it walked through the tree"
            );
            // And it actually got there. A body that never approached would
            // pass the check above for the wrong reason.
            assert!(
                d < STANDOFF_M - 0.5,
                "seed {seed} bearing {k}: body ended {d:.3} m out, having \
                 started at {STANDOFF_M} — the walk never engaged the trunk"
            );
        }
    }
}

/// The widest scattered thing on the island stops a body too, and by more.
/// A boulder's collision radius is several times a trunk's, so a check that
/// passed on a tree by accident of the quantum cannot pass here the same way.
///
/// **The fixture check below is DERIVED from the table, and it used to be the
/// literal 1.5.** That literal was the boulder's *nominal* radius —
/// "DodecahedronGeometry(1.5)" — and on 2026-08-10 the row became the mesh's
/// measured bound (1.1145; `blob_mesh` displaces vertices inward, so the
/// nominal was blocking 0.39 m of ground nobody could see). The literal went
/// red, correctly: it was asserting a number that had stopped being true.
/// Reading `occupant_volume` here says what the test actually means — this
/// occupant is much wider than a trunk — and cannot go stale that way again.
#[test]
fn a_boulder_stops_a_body_at_its_own_radius() {
    for seed in SEEDS {
        let mut sc = Scratch::live(seed);
        let (_, _, rock) = find(seed, &sc.table, &sc.haven, Occupant::Rock);
        let stop = reach(&rock);
        let (trunk_r, _) = terrain::occupant_volume(Occupant::Tree);
        assert!(
            stop > trunk_r * 3.0 + CAPSULE_RADIUS_M,
            "fixture check: a boulder reaches {stop:.3} m, not comfortably \
             past three trunk radii — this test would prove nothing a tree \
             had not already proved"
        );
        for k in 0..4u16 {
            let b = charge_at(seed, &mut sc.occupants(), &rock, k);
            let d = dist(&b, &rock);
            assert!(
                d >= stop - POS_XZ_Q,
                "seed {seed} bearing {k}: body is {d:.3} m from the boulder, \
                 inside its {stop:.3} m reach"
            );
            // Paired with the check above, and the pairing is the gate: a
            // body that walked straight THROUGH the boulder also ends up
            // further than `stop` from it, on the far side. Only "outside,
            // and stopped nearby" is evidence of a collision.
            assert!(
                d < STANDOFF_M - 0.5,
                "seed {seed} bearing {k}: body ended {d:.3} m out, having \
                 started at {STANDOFF_M} — it passed clean through"
            );
        }
    }
}

/// Marks exactly one cell harvested — the felled-tree fixture.
struct Felled(u16, u16);

impl Harvested for Felled {
    fn is_harvested(&self, cx: u16, cz: u16) -> bool {
        (cx, cz) == (self.0, self.1)
    }
}

/// A felled tree is not an invisible wall.
///
/// This is the failure the query was shaped to avoid: `scatter` is potential,
/// not state, so it hands back a pine that was chopped twenty minutes ago.
/// Consulting worldgen alone would leave a trunk you cannot see and cannot
/// walk through for the whole 20–45 minute respawn window.
#[test]
fn a_harvested_slot_stops_blocking() {
    for seed in SEEDS {
        let probe = Scratch::live(seed);
        let (cx, cz, tree) = find(seed, &probe.table, &probe.haven, Occupant::Tree);
        let stop = reach(&tree);

        // Standing: stopped, as the check above already says.
        let mut standing = Scratch::live(seed);
        let b = charge_at(seed, &mut standing.occupants(), &tree, 0);
        let d = dist(&b, &tree);
        assert!(
            d >= stop - POS_XZ_Q && d < STANDOFF_M - 0.5,
            "fixture: the standing tree must stop the body — it ended \
             {d:.3} m out of a {stop:.3} m reach, from {STANDOFF_M} m"
        );

        // Felled: the same walk passes through where the trunk was.
        let mut felled = Scratch::with(seed, Felled(cx as u16, cz as u16));
        let b = charge_at(seed, &mut felled.occupants(), &tree, 0);
        let (x, z) = pos(&b);
        let (fx, fz) = yaw_dir(0);
        // Progress measured along the walk's own bearing from the start, so
        // this cannot be satisfied by sliding sideways around the stump.
        let start = (tree.x - fx * STANDOFF_M, tree.z - fz * STANDOFF_M);
        let along = (x - start.0) * fx + (z - start.1) * fz;
        assert!(
            along > STANDOFF_M + stop,
            "seed {seed}: a felled tree still stopped the body — it advanced \
             {along:.3} m, not past the {:.3} m the trunk stood at",
            STANDOFF_M + stop
        );
    }
}

/// The memo cannot change an answer.
///
/// `SlotCache` is direct-mapped and evicts silently, and the whole argument
/// that this is safe rests on `terrain::scatter` being a pure function of
/// `(seed, cell)`. That argument is load-bearing for determinism — client and
/// server hold different lines — so it is asserted rather than reasoned
/// about. The scan deliberately covers far more cells than there are lines,
/// so eviction is exercised on nearly every lookup, and each cell is asked
/// twice to catch a stale hit as well as a wrong miss.
#[test]
fn the_scatter_memo_is_exact_under_eviction() {
    for seed in SEEDS {
        let table = ScatterTable::alpha_default();
        let haven = terrain::haven(seed);
        let mut cache = SlotCache::new();
        let mut checked = 0u32;
        let mut occupied = 0u32;
        // 64 x 64 = 4096 cells against 1024 lines: every line is overwritten
        // several times over.
        let mut cz = 40;
        while cz < 104 {
            let mut cx = 40;
            while cx < 104 {
                let want = terrain::scatter(seed, &table, &haven, cx, cz);
                let a = cache.slot(seed, &table, &haven, cx, cz);
                let b = cache.slot(seed, &table, &haven, cx, cz);
                for got in [a, b] {
                    assert_eq!(got.occupant, want.occupant, "cell ({cx},{cz})");
                    assert_eq!(got.x.to_bits(), want.x.to_bits(), "cell ({cx},{cz}) x");
                    assert_eq!(got.y.to_bits(), want.y.to_bits(), "cell ({cx},{cz}) y");
                    assert_eq!(got.z.to_bits(), want.z.to_bits(), "cell ({cx},{cz}) z");
                    assert_eq!(got.yaw, want.yaw, "cell ({cx},{cz}) yaw");
                    assert_eq!(
                        got.scale.to_bits(),
                        want.scale.to_bits(),
                        "cell ({cx},{cz})"
                    );
                }
                if want.occupant != Occupant::None {
                    occupied += 1;
                }
                checked += 1;
                cx += 1;
            }
            cz += 1;
        }
        assert_eq!(checked, 64 * 64);
        // Bit-equality over 4096 empty cells would prove nothing.
        assert!(
            occupied > 200,
            "seed {seed}: only {occupied} of {checked} cells held anything — \
             the eviction sweep never had much to get wrong"
        );
    }
}

/// A body inside an occupant can walk out.
///
/// Nodes respawn, and a respawn does not ask whether anybody is standing
/// there. A veto that applied unconditionally would then pin that player
/// forever, with no verb in the game that frees them — being stuck must never
/// be absorbing. `movement::step` lifts the occupant veto while the body's
/// own position is already blocked; this is that rule.
#[test]
fn a_body_caught_inside_an_occupant_can_walk_out() {
    for seed in SEEDS {
        let mut sc = Scratch::live(seed);
        let (_, _, rock) = find(seed, &sc.table, &sc.haven, Occupant::Rock);
        let cols = ColIndex::new();

        // Dead centre of the boulder — the deepest a respawn could catch you.
        let mut b = Body::at(seed, hv(seed), rock.x, rock.z);
        assert!(
            sc.occupants()
                .blocks(seed, rock.x, rock.z, b.qy as f32 * movement::POS_Y_Q),
            "fixture: the spawn point must be inside the boulder"
        );

        let f = charge(0);
        for _ in 0..TICKS {
            movement::step(seed, hv(seed), &cols, &mut sc.occupants(), &mut b, &f);
        }
        let d = dist(&b, &rock);
        assert!(
            d >= reach(&rock) - POS_XZ_Q,
            "seed {seed}: a body inside a boulder is trapped there — it got \
             {d:.3} m out of a {:.3} m volume",
            reach(&rock)
        );
    }
}

/// The shelter is a building, not a bollard: its walls stop a body and its
/// doorway does not.
///
/// `solid.rs` proves this of the predicate at authored local coordinates.
/// Here it is a body walking at the real placed building on the real pad,
/// which is the claim a player can check.
#[test]
fn the_shelter_walls_stop_a_body_and_the_door_does_not() {
    for seed in SEEDS {
        let mut sc = Scratch::live(seed);
        let (sx, sz, yaw8) = terrain::haven_shelter(&sc.haven);
        let shelter = Slot {
            occupant: Occupant::HavenShelter,
            x: sx,
            y: terrain::height(seed, sx, sz),
            z: sz,
            yaw: yaw8,
            scale: 1.0,
        };
        let cols = ColIndex::new();

        // The doorway is on the slot's local +Z, which is where `yaw_dir` of
        // its yaw points. Walk in along that bearing, from outside.
        let (dx, dz) = yaw_dir((yaw8 as u16) << 8);
        let door_yaw = ((yaw8 as u16) << 8).wrapping_add(0x8000); // face inward
        let mut b = Body::at(seed, hv(seed), sx + dx * 9.0, sz + dz * 9.0);
        let f = charge(door_yaw);
        for _ in 0..TICKS {
            movement::step(seed, hv(seed), &cols, &mut sc.occupants(), &mut b, &f);
        }
        let d = dist(&b, &shelter);
        assert!(
            d < 2.5,
            "seed {seed}: the doorway did not admit a body — it pinned at \
             {d:.3} m from the shelter's centre"
        );

        // The back wall, half a turn round, is not a doorway. Start behind
        // the building and walk the door's own bearing, which from there
        // points at the back wall.
        let (bx, bz) = yaw_dir(door_yaw);
        let mut b = Body::at(seed, hv(seed), sx + bx * 9.0, sz + bz * 9.0);
        let f = charge((yaw8 as u16) << 8);
        for _ in 0..TICKS {
            movement::step(seed, hv(seed), &cols, &mut sc.occupants(), &mut b, &f);
        }
        let d = dist(&b, &shelter);
        assert!(
            d > 2.5,
            "seed {seed}: a body walked in through the back wall, ending \
             {d:.3} m from the shelter's centre"
        );
    }
}

/// Nothing that blocks a body is something the body could have been standing
/// on in the first place — the spawn ring must not put anyone inside a rock.
///
/// A cheap end-to-end sanity check that the two halves agree, since
/// `World::spawn_pos` already clears its own 3x3 with `scatter_clear`.
#[test]
fn every_spawn_the_world_hands_out_is_clear_of_occupants() {
    for seed in SEEDS {
        let mut w = sim_core::world::World::new(seed);
        let mut sc = Scratch::with(seed, Pristine);
        for id in 1..24u32 {
            w.tick(&[sim_core::world::Command::Join { id }]);
            let p = w.players.iter().find(|p| p.active && p.id == id).unwrap();
            let (x, y, z) = (
                p.body.qx as f32 * POS_XZ_Q,
                p.body.qy as f32 * movement::POS_Y_Q,
                p.body.qz as f32 * POS_XZ_Q,
            );
            assert!(
                !sc.occupants().blocks(seed, x, z, y),
                "seed {seed}: player {id} spawned inside an occupant at \
                 ({x:.2}, {y:.2}, {z:.2})"
            );
        }
    }
}
