//! `farmwalk` — the **unthreatened solo ceiling**, measured
//! (DECISIONS.md §open, "the economy has never been measured against the
//! sim").
//!
//! `[globals] farm_per_min` declares an *effective* gather rate; the
//! sim's at-node ceiling is ~27–41× hotter (1353/min at the kit hatchet,
//! 2030 at the best tier-1 tool, against the declared 50), and until
//! this test nothing in the repo had ever farmed on the real island to
//! say where between the two a player lands. The named
//! method is the one used here: a bot walk on a real shard. A greedy
//! walker with the spawn kit's stone hatchet targets the nearest standing
//! tree, sprints to it, chops it out, and repeats until it has a wood
//! goal in its pockets — the same `movement::step`, the same
//! `gather::swing`, the same shipped content a player gets.
//!
//! **Read the number for what it is.** This walker is alone on an empty
//! island: nothing contests the node it is walking to, nothing attacks it,
//! it never banks a load at a base and never dies carrying one. The
//! reference game is hardcore PvP and none of those hold there (operator,
//! 2026-08-09) — so what this measures is the CEILING on effective
//! farming, travel and node exhaustion being the only costs the island can
//! currently charge. A populated shard subtracts a threat-and-logistics
//! term on top, and that term is not measurable here yet, because nothing
//! in this game fights back (`NOW.md` §0m item 2) and no shard has ever
//! held a hostile population. The gap between this number and the declared
//! rate is therefore an upper bound on the declaration's error, never a
//! verdict that the declaration is wrong.
//!
//! What it asserts is structure, not pacing: the island is farmable (a
//! spawn can reach trees and fell them), the yield arrives, and the
//! measured rate obeys the arithmetic law the content crate now gates
//! (effective ≤ at-node). The *measurement* is printed, not banded: what
//! the number should be is the operator's knob, and this test is the
//! instrument, refreshed on every run, that lets that knob be spoken from
//! evidence instead of guessed. No clock, no sockets: ticks and counts
//! only.

use server::core::ShardCore;
use server::stats::ShardStats;
use sim_core::gather::{HIT_UNIT, SWING_INTERVAL_TICKS};
use sim_core::input::{BTN_PRIMARY, BTN_SPRINT};
use sim_core::limits::{HOTBAR_SLOTS, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::{self, Occupant, CELLS_PER_SIDE, CELL_SIZE};
use sim_core::world::Command;

/// The wire yaw whose LUT entry points closest to `(dx, dz)` — the same
/// pick `tests/hunt.rs` steers with, off the public `yaw_dir`, so the
/// walker aims on the sim's own direction space.
fn yaw_toward(dx: f32, dz: f32) -> u16 {
    let (mut best, mut best_dot) = (0u16, f32::NEG_INFINITY);
    for i in 0..256u16 {
        let (ex, ez) = sim_core::yaw_dir(i << 8);
        let dot = ex * dx + ez * dz;
        if dot > best_dot {
            best_dot = dot;
            best = i << 8;
        }
    }
    best
}

/// Trees' worth of wood the walk must bank before it stops. Eight is
/// enough that the between-trees travel dominates the first approach and
/// the measurement is a farming *loop*, not one lucky tree.
const TREES_WORTH: u32 = 8;

/// Stop steering and start swinging inside this planar range — under
/// `gather::REACH_M` (2 m) so a step of overshoot never strands the swing
/// out of reach.
const STOP_M: f32 = 1.5;

/// Ticks a walk may take before the island is called unfarmable — 20
/// sim-minutes, generous for ~8 trees. The claim gated is *possible*, not
/// *quick*: the measured rate is the printed number, never this bound.
const WALK_LIMIT_TICKS: u32 = 20 * 60 * TICK_HZ;

/// Give up on a target if the walker has neither closed distance (blocked
/// by a boulder, wedged on a slope) nor felled it (a swing that cannot
/// land) within this many ticks, and pick another tree — a real player
/// walks around what they cannot walk through.
const STALL_TICKS: u32 = 20 * SWING_INTERVAL_TICKS as u32;

#[test]
fn a_walker_can_farm_the_island_and_the_rate_is_measured() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let stats = ShardStats::default();
    let mut core = ShardCore::new(20260731);
    core.world.gather = c.bake_gather().expect("gather bakes");
    core.world.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");

    // Every tree on the island, off the same pure scatter the sim swings
    // against: potential, not state — `slot_lives` owns harvested bits.
    let seed = core.world.seed;
    let mut trees: Vec<(i32, i32, f32, f32)> = Vec::new();
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(seed, &core.world.scatter, &core.world.haven, cx, cz);
            if s.occupant == Occupant::Tree {
                trees.push((cx, cz, s.x, s.z));
            }
        }
    }
    assert!(
        trees.len() > TREES_WORTH as usize,
        "the island scattered only {} trees",
        trees.len()
    );

    // Spawn beside the tree nearest the island's middle, in the first
    // neighbouring cell that scattered nothing — on open ground, not
    // inside another occupant's volume.
    let mid = CELLS_PER_SIDE as f32 * CELL_SIZE * 0.5;
    let &(fcx, fcz, fx, fz) = trees
        .iter()
        .min_by(|a, b| {
            let da = (a.2 - mid).powi(2) + (a.3 - mid).powi(2);
            let db = (b.2 - mid).powi(2) + (b.3 - mid).powi(2);
            da.partial_cmp(&db).unwrap()
        })
        .unwrap();
    let (sx, sz) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find_map(|(dx, dz)| {
            let (ncx, ncz) = (fcx + dx, fcz + dz);
            let s = terrain::scatter(seed, &core.world.scatter, &core.world.haven, ncx, ncz);
            (s.occupant == Occupant::None).then_some((
                (ncx as f32 + 0.5) * CELL_SIZE,
                (ncz as f32 + 0.5) * CELL_SIZE,
            ))
        })
        .unwrap_or((fx + 5.0, fz));
    core.world.dev_spawn = Some((sx, sz));
    assert!(core.connect(0, 0x100), "connect");
    core.tick(&stats, |_, _, _| true);
    let p = core
        .world
        .players
        .iter()
        .position(|p| p.active)
        .expect("seated");

    // The walker owns a hatchet and nothing else: the kit's bulk grants
    // would mask the count, and a full pocket silently drops yield
    // (`gather::inv_add`'s documented policy), which would corrupt the
    // measurement low.
    let hatchet = c.item_index("item.hatchet_stone").expect("the kit's axe");
    let sel = (0..HOTBAR_SLOTS)
        .find(|&s| {
            core.world.players[p].inv[s].item == hatchet && core.world.players[p].inv[s].count > 0
        })
        .expect("the spawn kit holds a stone hatchet on the hotbar");
    for (i, slot) in core.world.players[p].inv.iter_mut().enumerate() {
        if i != sel {
            *slot = Default::default();
        }
    }

    let wood = c.item_index("item.wood").expect("wood is an item");
    let def = &core.world.gather.nodes[0];
    assert_eq!(def.output, wood, "node slot 0 is the tree paying wood");
    let per_hit = u32::from(def.yield_for(hatchet));
    let per_tree = per_hit * u32::from(def.hits);
    let goal = TREES_WORTH * per_tree;
    // The at-node ceiling for THIS tool. Since the mark buys speed and
    // not yield (2026-08-09), a node's payout is invariant and the
    // ceiling is that whole payout over the fewest swings that can
    // exhaust it — every swing marked, rounding up as the sim's budget
    // arithmetic does. The finish share back-loads WHEN the yield lands
    // and not how much, so it does not enter here.
    let budget = u64::from(def.hits) * u64::from(HIT_UNIT);
    let want = u64::from(HIT_UNIT) + u64::from(def.weak_pct);
    let fewest_swings = budget.div_ceil(want);
    let at_node = (u64::from(per_tree) * u64::from(TICK_HZ) * 60
        / (fewest_swings * SWING_INTERVAL_TICKS)) as u32;

    let carried = |core: &ShardCore| -> u32 {
        core.world.players[p]
            .inv
            .iter()
            .filter(|s| s.item == wood && s.count > 0)
            .map(|s| u32::from(s.count))
            .sum()
    };

    let mut frame = core.world.players[p].frame;
    frame.sel = sel as u8;
    let mut target: Option<usize> = None;
    let mut skip = vec![false; trees.len()];
    let mut stall = 0u32;
    let mut best_d2 = f32::INFINITY;
    let mut banked = None;
    for t in 0..WALK_LIMIT_TICKS {
        let px = core.world.players[p].body.qx as f32 * POS_XZ_Q;
        let pz = core.world.players[p].body.qz as f32 * POS_XZ_Q;

        // Re-target on felled/abandoned: nearest standing tree not yet
        // given up on. Harvested state is read off the sim's own store.
        if target.is_none_or(|i| {
            core.world
                .slot_lives
                .is_harvested(trees[i].0 as u16, trees[i].1 as u16)
                || skip[i]
        }) {
            target = trees
                .iter()
                .enumerate()
                .filter(|(i, (cx, cz, _, _))| {
                    !skip[*i] && !core.world.slot_lives.is_harvested(*cx as u16, *cz as u16)
                })
                .min_by(|(_, a), (_, b)| {
                    let da = (a.2 - px).powi(2) + (a.3 - pz).powi(2);
                    let db = (b.2 - px).powi(2) + (b.3 - pz).powi(2);
                    da.partial_cmp(&db).unwrap()
                })
                .map(|(i, _)| i);
            stall = 0;
            best_d2 = f32::INFINITY;
        }
        let ti = target.unwrap_or_else(|| {
            panic!(
                "every tree is felled or skipped ({} scattered, {} skipped) with \
                 {} of {goal} wood banked — the island ran out of reachable trees \
                 before the walk banked its goal",
                trees.len(),
                skip.iter().filter(|s| **s).count(),
                carried(&core),
            )
        });
        let (dx, dz) = (trees[ti].2 - px, trees[ti].3 - pz);
        let d2 = dx * dx + dz * dz;

        // Progress is either distance closed or a tree felled; a target
        // that yields neither for STALL_TICKS is walked around, exactly
        // once per tree.
        if d2 + 0.25 < best_d2 {
            best_d2 = d2;
            stall = 0;
        } else {
            stall += 1;
            if stall > STALL_TICKS {
                skip[ti] = true;
                continue;
            }
        }

        frame.seq = frame.seq.wrapping_add(1);
        frame.yaw = yaw_toward(dx, dz);
        if d2 > STOP_M * STOP_M {
            frame.move_z = 127;
            frame.buttons = BTN_SPRINT;
        } else {
            frame.move_z = 0;
            frame.buttons = BTN_PRIMARY;
        }
        core.world.tick(&[Command::Input { id: 0x100, frame }]);

        if carried(&core) >= goal {
            banked = Some(t + 1);
            break;
        }
    }

    let ticks = banked.unwrap_or_else(|| {
        panic!(
            "20 sim-minutes and the walker banked {} of {goal} wood \
             ({} trees standing) — the island is not farmable by walking, \
             which no other gate can see",
            carried(&core),
            trees.len()
        )
    });
    let got = carried(&core);
    let effective = u64::from(got) * u64::from(TICK_HZ) * 60 / u64::from(ticks);

    // The arithmetic law the content crate gates, held by the measurement
    // too: travel only ever subtracts.
    assert!(
        effective <= u64::from(at_node),
        "measured {effective}/min beats the at-node ceiling {at_node}/min"
    );
    // The yield arrived whole, and now EXACTLY: a node's payout is
    // invariant under marking, so the walk banks whole trees' worth plus
    // whatever partial tree it was on when it crossed the goal. That
    // upper bound is one tree — much tighter than the old model allowed,
    // and the tightening is the point: a per-swing bonus would show up
    // here as an overshoot.
    assert!(
        got >= goal && got <= goal + per_tree,
        "banked {got} against a goal of {goal} (one tree = {per_tree}) — \
         yield leaked, or the mark is paying yield again"
    );

    // The measurement. `farm_per_min` declares wood at an effective rate
    // 27× under this tool's at-node ceiling; this is where an UNTHREATENED
    // SOLO walk landed. Duty is effective/at-node, both printed. A
    // populated PvP shard subtracts a threat term this island cannot
    // charge — see the header.
    let declared = c.balance.globals.farm_per_min.get("item.wood").copied();
    println!(
        "farmwalk: {got} wood in {ticks} ticks ({:.1} sim-min) — effective \
         {effective}/min · at-node {at_node}/min (stone hatchet) · declared \
         {declared:?}/min · duty {:.1}%",
        ticks as f32 / (60.0 * TICK_HZ as f32),
        100.0 * effective as f32 / at_node as f32,
    );
}
