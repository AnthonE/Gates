//! **Where do the wire's charges go?** The shard's 30 s raid run armed 27
//! charges against a 300-tick fuse and `EV_STRUCT_HIT` never once reached a
//! client (`findings/note-20260814-charge-never-detonates.md`). `raid.rs`
//! then proved the verb itself is correct on shipped content — green first
//! run — so the defect is in the *arrangement*, not in `detonate`.
//!
//! `NOW.md` §0rc names the next step as an instrumented run recording per
//! plant the tick planted and the tick due, and forbids the obvious shape it
//! could take: *"do not lengthen a wire gate to reach a detonation — that is
//! a gate waiting on a clock."* So this is not a wire gate. It is the wire's
//! **arrangement**, replayed straight into `World::tick`, where a tick is a
//! number rather than a wall-clock hope.
//!
//! ## The experiment, and the two hypotheses it kills
//!
//! Two arms, 905 ticks each — the run the note assumed, to the tick. Same
//! shipped content, same `raid_step` profile, same kit, same rng streams,
//! same everything **except where the eight bodies start**:
//!
//! - `Seating::ShardRing` — no `dev_spawn`, so `Command::Join` puts each
//!   body where a joining player's goes. This is the shard.
//! - `Seating::OnePlot` — every body on one cell centre, which is
//!   `raid_storm.rs`'s arrangement and the one `bots.rs`'s own doc describes.
//!
//! **Both arms raid.** Shard seating: 21 plants, 12 `EV_STRUCT_HIT`, first
//! at tick 355. Shared plot: 40 plants, 12 hits, first at 301. So:
//!
//! 1. **`NOW.md` §0rc's stated mechanism is refuted.** *"Attacker and owner
//!    never share a plot"* is true — this file measures it as an integer,
//!    `peak_shared_plot == 1`, which no test in the tree did before — and it
//!    **does not stop the raid**. Each attacker lays its own foundation at
//!    step 4 and plants on it at step 5; a charge is area-not-address
//!    (`charge.rs`), so it takes down whatever it built. A self-raid is a
//!    poor game and a perfectly good `EV_STRUCT_HIT`.
//! 2. **§0rc's stated suspicion is refuted too.** The plants do not cluster
//!    in the final fuse-length: the first lands on tick 55 and 17 of 21 come
//!    due inside the window.
//!
//! What that leaves is the list under *not reproduced* below — the wire-only
//! differences — and one arithmetic fact this file makes concrete: **the
//! earliest breach here is tick 355**, so any window shorter than that sees
//! zero by construction, exactly as the 4 s gate does. Whether the 30 s run
//! contained 355 ticks was never measured; `bot_smoke.rs` reads
//! `ShardStats::ticks` across its window now, so the next probe can say.
//!
//! ## What is reproduced from `botclient.rs`, exactly
//!
//! 1. **One raid action per client per tick.** `core::wants_action` hands
//!    the sim at most one and silently drops the rest, so the wire's rate is
//!    the server's ceiling and not a chosen number.
//! 2. **The plot is derived from the bot's own body** through
//!    `build_cell_of` on the quantized position, re-seated every
//!    `RAID_CYCLE` steps — `botclient::body_cell` and the `steps_in_cycle`
//!    reset, which is the line `NOW.md` §0rc suspects.
//! 3. **Odd streams raid, even streams own** (`seed_stream % 2 == 1`).
//! 4. **The hotbar selection rides the input lane and lasts one frame.**
//!    `raid_step`'s `Command::Input` never becomes an action: `botclient.rs`
//!    folds its `sel` into the *next* `bot_frame` with `.take()`, and every
//!    other frame carries `bot_frame`'s own `rng.next_bounded(6)`.
//! 5. **The bodies walk**, on `bot_frame`, seeded the way `botclient.rs`
//!    seeds it, with the raid on its own second stream.
//! 6. Shipped content and `bot_smoke.rs`'s kit layout, slot for slot.
//!
//! ## What is deliberately NOT reproduced, and which way each one leans
//!
//! - **No jitter buffer.** `client::consume_input` executes one buffered
//!   frame per tick, so over the wire the frame carrying `charge_slot` need
//!   not be the frame in force when the throw lands. Here the input command
//!   is applied the same tick it is issued. That makes both arms the
//!   **optimistic** case for held-item timing: they can only find *more*
//!   plants than the shard, never fewer.
//! - No packet loss, no reordering, no join stagger. Same direction.
//! - No dropped actions. `core::wants_action` takes one per client per tick
//!   and silently drops the rest; a dropped step does not retry, it *skips*,
//!   so over the wire a lost step 4 leaves step 5 throwing at an empty
//!   address. The shard's own counters already show the loss is real —
//!   `bot_smoke.rs` measured ~110 actions per bot across 120 ticks.
//!
//! Every one of those leans the same way: this harness can only find *more*
//! raiding than the shard, never less. That is what makes it evidence — the
//! refutations above survive the divergences, because a more-favourable
//! arrangement that still raids cannot be the reason a less-favourable one
//! did not.

#![allow(clippy::disallowed_macros)]

use sim_core::bots::{bot_frame, raid_step, RaidPlan, RaidRows, RAID_CYCLE};
use sim_core::build::{build_cell_of, foundation_terrain_ok, BUILD_CELL_M};
use sim_core::charge::ChargeRec;
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::limits::{MAX_BUILD_COORD, MAX_COMMANDS_PER_TICK};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World, EV_STRUCT_HIT};

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

/// The seed the arms share. It only has to be a legal island; the seating
/// is what is under test.
const SEED: u64 = 0xB1A57;

/// `bot_smoke.rs`'s fleet. Half attack, half own — `botclient.rs`'s parity
/// rule, so the count stays even or the comparison means nothing.
const BOTS: usize = 8;

/// 30 s at `TICK_HZ` 30 — the tick count the unexplained run was *assumed*
/// to have held, which is the number this file has to match to be a replay
/// of it. Whether the shard actually completed 905 in those 30 s is the open
/// question `bot_smoke.rs`'s `window_ticks` now instruments.
const TICKS: u64 = 905;

/// The shipped fuse, asserted against content below rather than assumed.
const FUSE_TICKS: u64 = 300;

/// Where the eight bodies start. The one variable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seating {
    /// The shard: `Command::Join` decides, as it does for a player.
    ShardRing,
    /// `raid_storm.rs`: every body on one cell centre.
    OnePlot,
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// `raid.rs`'s search, for `raid.rs`'s reason: ask the terrain rule where a
/// foundation may stand rather than guess a cell and redden on a worldgen
/// change that is nobody's bug.
fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell");
}

/// `botclient::body_cell`, line for line — the same clamp on the same
/// quantized coordinate, so this harness addresses the cell the wire would.
fn body_cell(q: i32) -> u16 {
    build_cell_of(q as f32 * POS_XZ_Q).clamp(0, MAX_BUILD_COORD as i32 - 1) as u16
}

/// One bot's wire-side state: the variables `botclient.rs`'s cadence arm
/// carries between ticks.
struct Wire {
    id: u32,
    attacker: bool,
    rng: Pcg32,
    raid_rng: Pcg32,
    yaw: u16,
    seq: u16,
    plan: Option<RaidPlan>,
    steps_in_cycle: u16,
    sel_override: Option<u8>,
    slot: usize,
}

/// One plant, as the harness saw it happen.
struct Plant {
    /// The tick the charge first appeared in the store.
    at: u64,
    rec: ChargeRec,
}

/// What one arm measured. Every field is a number; the assertions live in
/// the tests, so a failure names which claim moved.
struct Run {
    plants: Vec<Plant>,
    /// Ticks on which `EV_STRUCT_HIT` was announced.
    hit_ticks: Vec<u64>,
    /// The most bots that ever held the same plot cell on one tick. This is
    /// `NOW.md` §0rc's prose claim — *"attacker and owner never share a
    /// plot"* — as an integer.
    peak_shared_plot: usize,
    /// Distinct plot cells seated across the whole run.
    distinct_plots: usize,
    /// Charges still burning when the run ended.
    charges_left: usize,
    peak_pieces: usize,
}

impl Run {
    /// Plants whose fuse ran out before the run did. A charge due after the
    /// window is not evidence of anything, and separating them is what keeps
    /// a zero from being arithmetic.
    fn due_inside(&self) -> usize {
        self.plants
            .iter()
            .filter(|p| p.rec.fires_at < TICKS)
            .count()
    }
}

fn replay(seating: Seating) -> Run {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut w = World::new(SEED);
    w.gather = c.bake_gather().expect("gather bakes");
    w.build = c.bake_building().expect("building bakes");
    w.deploy = c.bake_deployables().expect("deployables bake");
    w.combat = c.bake_combat().expect("weapons bake");
    w.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");

    let rows = RaidRows {
        foundation: c
            .piece_index("build.foundation_twig")
            .expect("shipped foundation"),
        wall: c.piece_index("build.wall_twig").expect("shipped wall"),
        container: c.deploy_index("item.box_small").expect("shipped box"),
        lock: c.deploy_index("item.lock_code").expect("shipped lock"),
        charge_slot: 0,
        goods_slot: 2,
        code: 4242,
    };
    let satchel = c
        .item_index("item.satchel_charge")
        .expect("shipped satchel");
    let box_item = c.item_index("item.box_small").expect("shipped box item");
    let wood = c.item_index("item.wood").expect("shipped wood");
    let lock_item = c.item_index("item.lock_code").expect("shipped lock item");

    // The fuse this whole file is arithmetic about, read off content rather
    // than assumed: a rebalance that moved it would otherwise leave the gate
    // passing while measuring a different claim.
    let throw = w
        .combat
        .held_throw(satchel)
        .expect("the shipped satchel is a live throwable");
    assert_eq!(
        throw.fuse_ticks as u64, FUSE_TICKS,
        "the shipped fuse moved; this file's window arithmetic is stated \
         against {FUSE_TICKS}"
    );

    let plot = buildable_cell(SEED);
    let (px, pz) = cell_center(plot.0, plot.1);
    if seating == Seating::OnePlot {
        w.dev_spawn = Some((px, pz));
    }

    let mut bots: Vec<Wire> = Vec::with_capacity(BOTS);
    for i in 0..BOTS {
        let id = (i as u32) + 1;
        w.tick(&[Command::Join { id }]);
        let slot = w
            .players
            .iter()
            .position(|p| p.active && p.id == id)
            .expect("the bot seated");
        if seating == Seating::OnePlot {
            // `raid_storm.rs`'s line: both sides of a plot stand on the cell
            // centre, because every verb here is reach-checked.
            w.players[slot].body = Body::at(SEED, hv(SEED), px, pz);
        }
        // `bot_smoke.rs`'s kit, slot for slot. A fixture, and named as one:
        // the judge ranks the progression ladder separately (§B.2) and this
        // gate does not pretend to close it.
        let stack = |item: u16| ItemStack {
            item,
            count: w.gather.stack_max_of(item),
            cond: 0,
        };
        w.players[slot].inv[0] = stack(satchel);
        w.players[slot].inv[1] = stack(box_item);
        w.players[slot].inv[2] = stack(wood);
        w.players[slot].inv[3] = stack(lock_item);
        let stream = i as u64;
        bots.push(Wire {
            id,
            attacker: stream % 2 == 1,
            rng: Pcg32::new(SEED ^ 0xB07B_07B0, stream),
            raid_rng: Pcg32::new(SEED ^ 0x5A1D_C0DE, stream),
            yaw: (stream as u16).wrapping_mul(2557),
            seq: 1,
            plan: None,
            steps_in_cycle: 0,
            sel_override: None,
            slot,
        });
    }

    let mut run = Run {
        plants: Vec::new(),
        hit_ticks: Vec::new(),
        peak_shared_plot: 0,
        distinct_plots: 0,
        charges_left: 0,
        peak_pieces: 0,
    };
    let mut seen_plots: Vec<(u16, u16)> = Vec::new();
    let mut cmds: Vec<Command> = Vec::with_capacity(BOTS * 2);

    for t in 0..TICKS {
        cmds.clear();
        // Inputs first, then actions — `core.rs` queues the datagram lane
        // ahead of the action lane inside one server tick.
        let mut steps: Vec<Command> = Vec::with_capacity(BOTS);
        for b in bots.iter_mut() {
            // The frame first, with any pending selection folded in and
            // consumed — `botclient.rs`'s `.take()`.
            let mut f = bot_frame(&mut b.rng, b.yaw, b.seq);
            if let Some(sel) = b.sel_override.take() {
                f.sel = sel;
            }
            b.yaw = f.yaw;
            b.seq = b.seq.wrapping_add(1);
            cmds.push(Command::Input {
                id: b.id,
                frame: f,
                favour: 0,
            });

            // Then one raid step, on the plot re-seated from the live body.
            if b.steps_in_cycle >= RAID_CYCLE {
                b.plan = None;
            }
            if b.plan.is_none() {
                let body = w.players[b.slot].body;
                let (cx, cz) = (body_cell(body.qx), body_cell(body.qz));
                b.plan = Some(RaidPlan::new(b.id, cx, cz, b.attacker));
                b.steps_in_cycle = 0;
                if !seen_plots.contains(&(cx, cz)) {
                    seen_plots.push((cx, cz));
                }
            }
            let p = b.plan.as_mut().expect("seated above");
            let cmd = raid_step(p, &mut b.raid_rng, rows);
            b.steps_in_cycle += 1;
            match cmd {
                Command::Input { frame, .. } => b.sel_override = Some(frame.sel),
                other => steps.push(other),
            }
        }
        // The measurement `NOW.md` §0rc's prose asserts and nothing counts:
        // how many bots stand on the busiest plot right now.
        for b in bots.iter() {
            let cell = b.plan.map(|p| (p.cx, p.cz));
            let n = bots
                .iter()
                .filter(|o| o.plan.map(|p| (p.cx, p.cz)) == cell)
                .count();
            run.peak_shared_plot = run.peak_shared_plot.max(n);
        }
        cmds.append(&mut steps);
        assert!(
            cmds.len() <= MAX_COMMANDS_PER_TICK,
            "the replay must not out-run the tick's own command cap"
        );
        w.tick(&cmds);

        // Every charge in the store this harness has not seen before was
        // planted on this tick. Identity is the whole record: two charges at
        // one address differ in `fires_at`, and two at one address on one
        // tick are one charge — `place` refuses the second.
        for rec in w.charges.entries() {
            if !run.plants.iter().any(|p| p.rec == *rec) {
                run.plants.push(Plant { at: t, rec: *rec });
            }
        }
        if w.events.entries().iter().any(|e| e.code == EV_STRUCT_HIT) {
            run.hit_ticks.push(t);
        }
        run.peak_pieces = run.peak_pieces.max(w.pieces.len());
    }

    run.charges_left = w.charges.len();
    run.distinct_plots = seen_plots.len();
    run
}

fn report(name: &str, r: &Run) {
    println!(
        "{name}: plants={} due-inside={} hits={} peak-shared-plot={} \
         distinct-plots={} charges-left={} peak-pieces={} first-plants={:?} \
         hit-ticks={:?}",
        r.plants.len(),
        r.due_inside(),
        r.hit_ticks.len(),
        r.peak_shared_plot,
        r.distinct_plots,
        r.charges_left,
        r.peak_pieces,
        r.plants.iter().map(|p| p.at).take(8).collect::<Vec<_>>(),
        r.hit_ticks,
    );
}

/// **Nobody shares a plot, and the raid completes anyway.** The refutation.
///
/// Eight bots joining as a player joins, walking as `bot_frame` walks them,
/// running the shipped profile for three fuse-lengths. `peak_shared_plot`
/// comes out **1** — `NOW.md` §0rc's prose is exactly right about the
/// arrangement, and this is the first time anything in the tree has measured
/// it rather than asserted it in a comment. And `EV_STRUCT_HIT` arrives
/// twelve times regardless, because an attacker lays its own foundation four
/// steps before it plants on it and a blast is area, not address.
///
/// So this test is *load-bearing as a negative*: it is what says the shard's
/// zero is not explained by the seating, and every future pass that reaches
/// for that explanation has to get past it first. It goes red the day the
/// wire's arrangement stops being able to raid — which is a thing worth
/// hearing about, since today it can.
#[test]
fn the_wire_seating_raids_without_ever_sharing_a_plot() {
    let r = replay(Seating::ShardRing);
    report("shard-ring", &r);

    // 1. The arrangement reaches the verb. Without this everything below is
    //    a pass it did not earn — a raid that never arms proves nothing.
    assert!(
        !r.plants.is_empty(),
        "no charge armed in {TICKS} ticks; the shard armed 27, so the replay \
         has diverged from the thing it is a replay of"
    );

    // 2. §0rc's prose, as an integer. `bots.rs`'s own doc says a plot is
    //    "one cell with two players on it"; over the wire it never is, and
    //    this is the number that says so.
    assert_eq!(
        r.peak_shared_plot, 1,
        "two bots shared a plot under shard seating — `botclient.rs`'s \
         body-derived plot must have changed, and the refutation below is \
         then about a different arrangement"
    );

    // 3. §0rc's suspicion — "the plants cluster in the final 300 ticks" —
    //    refuted: they come due inside the window, with room.
    assert!(
        r.due_inside() > 0,
        "every one of {} plants was due after tick {TICKS}; the zero this \
         file was written to explain would then be a window, not a bug",
        r.plants.len()
    );

    // 4. And the refutation itself: it breaks things. A self-raid is a poor
    //    game (the judge's gap 3 stands, and this does not close it) but it
    //    is a perfectly good detonation, so the seating cannot be why the
    //    shard saw none.
    assert!(
        !r.hit_ticks.is_empty(),
        "the wire's seating broke nothing over {TICKS} ticks — which would \
         make the seating the cause after all, and this file's header wrong"
    );

    // 5. The arithmetic every window has to clear. Nothing can break before
    //    a fuse has burned, so a run shorter than this sees zero by
    //    construction — which is the one live suspect left for the shard.
    assert!(
        r.hit_ticks[0] >= FUSE_TICKS,
        "a structure was hit on tick {} — before any fuse could run out, so \
         this arm is not measuring detonations at all",
        r.hit_ticks[0]
    );
}

/// **The comparison arm: put the fleet on one plot and the count moves, the
/// outcome does not.**
///
/// Identical content, profile, rng streams and tick count — one variable
/// moved. Plants roughly double (40 against 21) because an attacker planting
/// on an owner's wall no longer has to build the wall first, and the hit
/// count does not move at all. That is what makes the arm above a refutation
/// rather than an observation: seating changes how *much* raiding happens,
/// and not *whether* it happens.
#[test]
fn a_shared_plot_changes_the_count_and_not_the_outcome() {
    let one = replay(Seating::OnePlot);
    report("one-plot", &one);
    let ring = replay(Seating::ShardRing);

    assert_eq!(
        one.peak_shared_plot, BOTS,
        "this arm did not actually put the fleet on one plot; it is then not \
         a comparison with anything"
    );
    assert!(
        !one.hit_ticks.is_empty(),
        "the shared-plot arm broke nothing, so the two arms agree for a \
         reason that has nothing to do with seating"
    );
    assert!(
        one.plants.len() > ring.plants.len(),
        "a shared plot armed {} charges against scattered seating's {} — the \
         arm is supposed to raid HARDER, and if it does not, the two \
         arrangements are not actually different",
        one.plants.len(),
        ring.plants.len()
    );
    assert!(
        one.hit_ticks[0] >= FUSE_TICKS,
        "a structure was hit on tick {} — before any fuse could run out",
        one.hit_ticks[0]
    );
}

/// **The floor: one raider, alone, standing still, still completes a raid.**
///
/// The plot is seated once and never re-seated, and the body is never
/// driven. This is the minimum arrangement that can produce an
/// `EV_STRUCT_HIT` at all, and it produces one — so neither the walking, nor
/// the re-seating, nor the fleet, nor the owner is a precondition. Anything
/// that reddens here has broken the raid outright rather than degraded it,
/// which is why this is the cheapest of the three to read on a failure.
#[test]
fn a_raider_alone_on_its_own_plot_still_completes_a_raid() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut w = World::new(SEED);
    w.gather = c.bake_gather().expect("gather bakes");
    w.build = c.bake_building().expect("building bakes");
    w.deploy = c.bake_deployables().expect("deployables bake");
    w.combat = c.bake_combat().expect("weapons bake");
    w.spawn_kit = c.bake_spawn_kit().expect("the kit bakes");

    let rows = RaidRows {
        foundation: c
            .piece_index("build.foundation_twig")
            .expect("shipped foundation"),
        wall: c.piece_index("build.wall_twig").expect("shipped wall"),
        container: c.deploy_index("item.box_small").expect("shipped box"),
        lock: c.deploy_index("item.lock_code").expect("shipped lock"),
        charge_slot: 0,
        goods_slot: 2,
        code: 4242,
    };
    let satchel = c
        .item_index("item.satchel_charge")
        .expect("shipped satchel");
    let wood = c.item_index("item.wood").expect("shipped wood");

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    let id = 1u32;
    w.tick(&[Command::Join { id }]);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    w.players[0].inv[0] = ItemStack {
        item: satchel,
        count: w.gather.stack_max_of(satchel),
        cond: 0,
    };
    w.players[0].inv[2] = ItemStack {
        item: wood,
        count: w.gather.stack_max_of(wood),
        cond: 0,
    };

    // Seated once, from the body, and never re-seated. The body is never
    // driven, so it never leaves the cell it seated from — `bot_frame`'s
    // wander is exactly the variable this test removes.
    let mut plan = RaidPlan::new(
        id,
        body_cell(w.players[0].body.qx),
        body_cell(w.players[0].body.qz),
        true,
    );
    let mut rng = Pcg32::new(SEED ^ 0x5A1D_C0DE, 1);
    let mut hits = 0usize;
    let mut plants = 0usize;
    let mut sel_override: Option<u8> = None;
    let mut seq: u16 = 1;

    for _ in 0..TICKS {
        let mut cmds: Vec<Command> = Vec::with_capacity(2);
        if let Some(sel) = sel_override.take() {
            cmds.push(Command::Input {
                id,
                frame: InputFrame {
                    seq,
                    sel,
                    ..InputFrame::default()
                },
                favour: 0,
            });
            seq = seq.wrapping_add(1);
        }
        let before = w.charges.len();
        match raid_step(&mut plan, &mut rng, rows) {
            Command::Input { frame, .. } => sel_override = Some(frame.sel),
            other => cmds.push(other),
        }
        w.tick(&cmds);
        if w.charges.len() > before {
            plants += 1;
        }
        hits += w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_STRUCT_HIT)
            .count();
    }

    println!("alone-standing-still: plants={plants} hits={hits}");
    assert!(
        plants > 0,
        "the standing raider never armed a charge; the kit or the profile \
         changed and this test is measuring neither"
    );
    assert!(
        hits > 0,
        "a raider that never moves planted {plants} charges on its own \
         foundation over {TICKS} ticks and broke nothing: the defect would \
         then be the walking, not the seating, and this file is wrong"
    );
}
