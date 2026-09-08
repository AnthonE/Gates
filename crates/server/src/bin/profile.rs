//! `profile` — where a tick's 33.3 ms actually goes.
//!
//! Not a gate and never will be: it reports elapsed time, and a gate that
//! waits on a clock is not a gate on this box (CLAUDE.md). It is the
//! instrument the *decision* wants — `NOW.md`'s 100-bot soak asks whether
//! the AOI linear scan needs a spatial structure, and that question has an
//! arithmetic answer nobody had measured.
//!
//! What it builds is the shard's stated worst case, not a plausible one:
//! `MAX_PLAYERS` bodies inside one AOI cell (every body is a candidate for
//! every other, which is the shape `snapshot_budget.rs` already uses), the
//! roster alive, and a piece store filled to whatever the terrain will
//! take. Then it separates the sim from the netcode by *ablation* rather
//! than by instrumenting the hot path: `World::tick` is public, so a run of
//! bare sim ticks measures the sim alone, and `ShardCore::tick` on even and
//! odd ticks straddles the snapshot cadence (`SNAPSHOT_INTERVAL_TICKS`).
//! Three numbers, three subtractions, no `#[cfg]` in the code being timed.
//!
//! ```text
//! cargo run --release -p server --bin profile -- --clients 100 --ticks 600
//! valgrind --tool=callgrind target/release/profile --ticks 40   # per-fn
//! ```
//!
//! Under callgrind the wall-clock column is meaningless (everything is ~50×
//! slower); what is meaningful there is `callgrind_annotate`'s instruction
//! ranking, which is deterministic and therefore the one profile this box
//! can produce twice and get the same answer.

use protocol::InputDatagram;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::{LOC_PLANE, MAT_TWIG, SHAPE_FOUNDATION};
use sim_core::gather::ItemStack;
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{MAX_COMMANDS_PER_TICK, MAX_PIECES, MAX_PLAYERS, SNAPSHOT_INTERVAL_TICKS};
use sim_core::world::Command;
use std::time::{Duration, Instant};

/// Player id for connection slot `s`, as `install` mints them (generation
/// 1) — `snapshot_budget.rs`'s helper, so both read the same shard.
fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

struct Args {
    clients: usize,
    pieces: usize,
    ticks: u32,
    warmup: u32,
    seed: u64,
    content_dir: String,
    /// Metres between bodies in the cluster grid. The default packs every
    /// body inside everyone else's enter radius, which is the worst case
    /// the AOI rank band exists for.
    pitch_m: f32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            clients: MAX_PLAYERS,
            pieces: 2_000,
            ticks: 600,
            warmup: 60,
            seed: 20_260_731,
            content_dir: "content".into(),
            pitch_m: 6.0,
        }
    }
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let val = argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--clients" => a.clients = val.parse().unwrap_or(a.clients).min(MAX_PLAYERS),
            "--pieces" => a.pieces = val.parse().unwrap_or(a.pieces).min(MAX_PIECES),
            "--ticks" => a.ticks = val.parse().unwrap_or(a.ticks),
            "--warmup" => a.warmup = val.parse().unwrap_or(a.warmup),
            "--seed" => a.seed = val.parse().unwrap_or(a.seed),
            "--content" => a.content_dir = val,
            "--pitch" => a.pitch_m = val.parse().unwrap_or(a.pitch_m),
            "--help" | "-h" => {
                println!(
                    "profile [--clients N] [--pieces N] [--ticks N] [--warmup N] \
                     [--seed N] [--content DIR] [--pitch M]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("profile: unknown flag {other}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
    a
}

/// The shard's own boot bake, minus the sockets. Every table matters here:
/// an inert world does no work, so a profile of one would flatter every
/// number in this file.
fn build_core(a: &Args) -> ShardCore {
    let content = match content::Content::load_dir(std::path::Path::new(&a.content_dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("profile: content refused: {e}");
            std::process::exit(1);
        }
    };
    let mut core = ShardCore::new(a.seed);
    macro_rules! bake {
        ($field:ident, $m:ident) => {
            core.world.$field = match content.$m() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("profile: content bake refused: {e}");
                    std::process::exit(1);
                }
            }
        };
    }
    bake!(gather, bake_gather);
    bake!(craft, bake_craft);
    bake!(deploy, bake_deployables);
    bake!(build, bake_building);
    bake!(combat, bake_combat);
    bake!(backpack, bake_backpack);
    bake!(survival, bake_survival);
    bake!(cook, bake_cooking);
    bake!(spawn_kit, bake_spawn_kit);
    bake!(loot, bake_loot);
    bake!(mob, bake_mobs);
    bake!(research, bake_research);
    core.catalog = match server::net::bake_catalog(&content, &core.world.combat) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("profile: catalog bake refused: {e}");
            std::process::exit(1);
        }
    };
    core
}

/// Connect `n` clients and herd every body into one square, then hand each
/// one an input frame so the sim has something to execute.
fn fill_clients(core: &mut ShardCore, stats: &ShardStats, a: &Args) {
    for slot in 0..a.clients {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        if (slot + 1) % 32 == 0 || slot + 1 == a.clients {
            core.tick_bare(stats, |_, _, _| true);
        }
    }
    let side = (a.clients as f32).sqrt().ceil().max(1.0) as usize;
    for (i, p) in core.world.players.iter_mut().enumerate() {
        if !p.active {
            continue;
        }
        p.body.qx = ((900.0 + (i % side) as f32 * a.pitch_m) / 0.03) as i32;
        p.body.qz = ((900.0 + (i / side) as f32 * a.pitch_m) / 0.03) as i32;
    }
}

/// Fill the piece store the way the game does — one `Command::Place` per
/// piece, through the real refusal chain — by walking a builder to each
/// address. Foundations only: the terrain refuses plenty of them, and what
/// this wants is a *populated store*, not a base that would stand up.
///
/// Returns what actually landed, which is never the number asked for.
///
/// **The sweep is four times the cells it needs since build plate v1.** Every
/// cell here is orthogonally adjacent to the last, so the whole block is ONE
/// plate — and a plate that wide over real terrain hits the stilt limits
/// often. The old `ceil(sqrt(want)) + 1` square assumed the only refusals
/// were `foundation_terrain_ok`'s; with the plate it ran out of cells first
/// and quietly profiled a third of the store it was asked for. Widening is
/// the right fix rather than exempting the bench: a profile that measures a
/// world the game cannot produce is a profile of nothing.
fn fill_pieces(core: &mut ShardCore, want: usize) -> usize {
    let bc = core.world.build;
    let Some(row) = (0..bc.piece_count).find(|&r| {
        let d = &bc.pieces[r as usize];
        d.hp != 0 && d.shape == SHAPE_FOUNDATION && d.material == MAT_TWIG
    }) else {
        return 0;
    };
    let cost = bc.pieces[row as usize];
    // A fixture builder rather than one of the clients: the cluster's bodies
    // are what the AOI measurement reads, and walking one of them across the
    // island to lay foundations would move the scene under the profile. It
    // joins before any client does, so it takes a world slot nobody wants,
    // and it leaves before the clients arrive.
    core.world.tick(&[Command::Join { id: 0xB0_1D_E7 }]);
    let Some(w) = core
        .world
        .players
        .iter()
        .position(|p| p.active && p.id == 0xB0_1D_E7)
    else {
        return 0;
    };
    let id = core.world.players[w].id;
    let mut cmds: Vec<Command> = Vec::with_capacity(MAX_COMMANDS_PER_TICK);
    let mut placed = 0usize;
    // A solid block of foundations on the build grid (`BUILD_CELL_M`), one
    // per cell. The builder teleports to each address because `place`
    // checks reach and this is not a walk simulator.
    let side = ((want as f32).sqrt().ceil() as u16 + 1) * 2;
    let base = 300u16;
    'outer: for gz in 0..side {
        for gx in 0..side {
            if placed >= want || core.world.pieces.len() >= MAX_PIECES {
                break 'outer;
            }
            let (cx, cz) = (base + gx, base + gz);
            // Teleport, refill, place: the builder is a fixture, not a
            // player, and paying for 2,000 foundations honestly would be a
            // gather benchmark rather than a server one.
            let (ax, az) = sim_core::build::anchor(cx, cz, LOC_PLANE);
            core.world.players[w].body.qx = (ax / 0.03) as i32;
            core.world.players[w].body.qz = (az / 0.03) as i32;
            for &(item, units) in cost.costs.iter().take(cost.n_costs as usize) {
                core.world.players[w].inv[0] = ItemStack {
                    item,
                    count: units.saturating_mul(4).max(4),
                    cond: 0,
                };
            }
            cmds.push(Command::Place {
                id,
                row,
                cx,
                cz,
                level: 0,
                loc: LOC_PLANE,
                freehand: false,
                plate: 0,
            });
            let before = core.world.pieces.len();
            core.world.tick(&cmds);
            cmds.clear();
            placed += core.world.pieces.len() - before;
        }
    }
    // The fixture leaves; its body must not sit in the cluster's AOI.
    core.world.tick(&[Command::Leave { id }]);
    core.world.pieces.len()
}

/// Mean and worst of a sample, in microseconds.
fn stat(v: &mut [Duration]) -> (f64, f64, f64) {
    let n = v.len().max(1) as f64;
    let sum: Duration = v.iter().sum();
    v.sort_unstable();
    let p99 = v[(v.len() * 99 / 100).min(v.len().saturating_sub(1))];
    let worst = *v.last().unwrap_or(&Duration::ZERO);
    (
        sum.as_secs_f64() * 1e6 / n,
        p99.as_secs_f64() * 1e6,
        worst.as_secs_f64() * 1e6,
    )
}

fn main() {
    let a = parse_args();
    let stats = ShardStats::default();
    let mut core = build_core(&a);
    let pieces = fill_pieces(&mut core, a.pieces);
    fill_clients(&mut core, &stats, &a);

    // Every client sends every tick, which is what a shard under load
    // looks like — an empty input buffer profiles the starve path instead.
    //
    // **And every client acks**, which is not decoration: an unacked client
    // has no delta baseline, so every snapshot encodes absolute and the
    // whole delta half of the encoder — the baseline lookup, the fits
    // tests, `encode_delta` — never runs. A profile without acks measures a
    // shard where nobody has finished joining.
    let mut seq: u16 = 1;
    let mut last_snap = vec![0u16; MAX_PLAYERS];
    let feed = |core: &mut ShardCore, seq: &mut u16, last_snap: &[u16]| {
        for (slot, &ack) in last_snap.iter().enumerate().take(a.clients) {
            let mut dg = InputDatagram::new(ack, 0, 4);
            let _ = dg.push(InputFrame {
                seq: *seq,
                // Held, because the arm is the verb this game is mostly
                // made of and `gather::swing` early-outs without it — a
                // profile of a hundred players who never chop is a profile
                // of a shard nobody is playing.
                buttons: BTN_PRIMARY,
                move_x: 1,
                move_z: 0,
                yaw: seq.wrapping_mul(97),
                pitch: 0,
                sel: 0,
            });
            core.push_input(slot, &dg);
        }
        *seq = seq.wrapping_add(1);
    };

    for _ in 0..a.warmup {
        feed(&mut core, &mut seq, &last_snap);
        let snap_tick = (core.world.tick + 1) as u16;
        core.tick_bare(&stats, |lane, slot, _| {
            if lane == Lane::Snapshot {
                last_snap[slot] = snap_tick;
            }
            true
        });
    }

    // --- full ticks, split on the snapshot cadence.
    let mut even = Vec::with_capacity(a.ticks as usize);
    let mut odd = Vec::with_capacity(a.ticks as usize);
    for _ in 0..a.ticks {
        feed(&mut core, &mut seq, &last_snap);
        let snap_tick = (core.world.tick + 1) as u16;
        let is_snap = (core.world.tick + 1).is_multiple_of(SNAPSHOT_INTERVAL_TICKS);
        let t = Instant::now();
        core.tick_bare(&stats, |lane, slot, _| {
            if lane == Lane::Snapshot {
                last_snap[slot] = snap_tick;
            }
            true
        });
        let d = t.elapsed();
        if is_snap {
            even.push(d)
        } else {
            odd.push(d)
        }
    }

    // --- the sim alone. `World::tick` is public and takes the command
    // slice the core would have built, so this is the same work minus
    // every line of netcode.
    let mut sim = Vec::with_capacity(a.ticks as usize);
    for _ in 0..a.ticks {
        let t = Instant::now();
        core.world.tick(&[]);
        sim.push(t.elapsed());
    }

    // --- the two whole-world passes, timed apart from the ticks that carry
    // them. Both are periodic and both are on the sim thread, so an average
    // over ticks buries them: the digest lands one tick in
    // `STATE_HASH_INTERVAL` and the world save one tick in
    // `world_save_interval_ticks`, and what a player feels is the tick they
    // land ON. `reference/SAVES.md` §4 is the reference game's version of
    // exactly this, unfixed in thirteen years.
    let mut hash_t = Vec::with_capacity(64);
    for _ in 0..64 {
        let t = Instant::now();
        std::hint::black_box(core.world.state_hash());
        hash_t.push(t.elapsed());
    }
    let mut save_t = Vec::with_capacity(64);
    let mut wbuf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES].into_boxed_slice();
    for _ in 0..64 {
        let t = Instant::now();
        std::hint::black_box(core.encode_world(&mut wbuf));
        save_t.push(t.elapsed());
    }

    let (e_avg, e_p99, e_max) = stat(&mut even);
    let (o_avg, o_p99, o_max) = stat(&mut odd);
    let (s_avg, s_p99, s_max) = stat(&mut sim);
    let (h_avg, h_p99, h_max) = stat(&mut hash_t);
    let (w_avg, w_p99, w_max) = stat(&mut save_t);
    let budget_us = 1e6 / sim_core::limits::TICK_HZ as f64;

    println!(
        "profile: seed {} · {} clients · {pieces} pieces · {} mobs alive · {} ticks",
        a.seed,
        a.clients,
        core.world.mobs.m.iter().filter(|m| m.alive).count(),
        a.ticks
    );
    println!(
        "         budget {budget_us:.0} µs/tick at {} Hz",
        sim_core::limits::TICK_HZ
    );
    println!();
    println!("  phase                         avg µs      p99 µs      max µs   % budget");
    println!("  ------------------------------------------------------------------------");
    println!(
        "  full tick (snapshot)       {e_avg:10.1}  {e_p99:10.1}  {e_max:10.1}  {:8.1}%",
        100.0 * e_avg / budget_us
    );
    println!(
        "  full tick (no snapshot)    {o_avg:10.1}  {o_p99:10.1}  {o_max:10.1}  {:8.1}%",
        100.0 * o_avg / budget_us
    );
    println!(
        "  World::tick alone          {s_avg:10.1}  {s_p99:10.1}  {s_max:10.1}  {:8.1}%",
        100.0 * s_avg / budget_us
    );
    println!(
        "  state_hash (1 tick in {:<2}) {h_avg:10.1}  {h_p99:10.1}  {h_max:10.1}  {:8.1}%",
        sim_core::limits::STATE_HASH_INTERVAL,
        100.0 * h_avg / budget_us
    );
    println!(
        "  encode_world (1 in {:<5}){w_avg:10.1}  {w_p99:10.1}  {w_max:10.1}  {:8.1}%",
        server::config::DEFAULT_WORLD_SAVE_INTERVAL_TICKS,
        100.0 * w_avg / budget_us
    );
    println!();
    println!("  by subtraction:");
    println!(
        "    snapshot encode (all clients) {:10.1} µs",
        e_avg - o_avg
    );
    println!(
        "    interest + events + drip      {:10.1} µs",
        o_avg - s_avg
    );
    println!("    sim                           {s_avg:10.1} µs");
    println!();
    println!(
        "  counters: snap_sent {} · candidates {} · shed {} · ev_sent {} · resyncs {}",
        ShardStats::get(&stats.snap_sent),
        ShardStats::get(&stats.snap_candidates),
        ShardStats::get(&stats.snap_entities_shed),
        ShardStats::get(&stats.ev_sent),
        ShardStats::get(&stats.ev_resyncs),
    );
}
