//! `zoo` — breed a wolf brain against the agent.
//!
//! Quake Golf's `tools/zoo.mjs`, for wolves: a (1+λ) evolution strategy over
//! a genome that is a base seed and one 16-bit seed per accepted mutation
//! (`sim_core::genome`), whose net chooses a wolf's fights
//! (`sim_core::brain::WOLF_NET`). Every bout is the arena's lockstep
//! (`tests/arena/rig.rs`): a real `ShardCore` with the shipped content, a
//! synthetic clock, and the agent on the other side — perception, hands and
//! reflexes exactly as it plays on a shard, so a wolf is bred against
//! something human-paced that fights back, never against a seat that reads
//! the world.
//!
//! A bout: the agent (a hands preset, a temperament and a weapon, drawn
//! from the bout's seed) stands about in the arena clearing; a free pack of
//! one to three wolves is put 18–28 m off at the seed's bearing, inside
//! their notice radius, and it plays until the agent is down, the pack is
//! dead, or 90 s pass. The island plays at the medium fel tier, the one a
//! bred brain first drives in (`mobs.toml` `genome_tier`).
//!
//! The start: a random net hardly ever charges, and a wolf that never
//! charges never bites, so `--screen` base seeds are first scored without a
//! bout — the share of a fixed set of made-up fight snapshots each one
//! answers with a charge — and the best `--bases` of those play the first
//! batch for the starting parent. That is still the encoding: a base seed
//! the screen liked, nothing hand-set.
//!
//! Selection: the parent and λ children (each the parent plus one fresh
//! mutation) play the same batch of bouts; the best child is tried again
//! against the parent on a confirm batch it has never seen, and replaces it
//! only if it wins there too — without that second test the best of fifteen
//! beats its parent most generations by luck alone. The end is a held-out
//! table against the rule-table wolf on bouts neither stage saw, including
//! a temperament no training bout used.
//!
//! ```text
//! cargo run --release -p server --example zoo                      # breed from scratch
//! cargo run --release -p server --example zoo -- --gens 100 --kids 15 --batch 24
//! cargo run --release -p server --example zoo -- --from gw1:...    # keep breeding a genome
//! cargo run --release -p server --example zoo -- --bench gw1:...   # the held-out table only
//! ```
//!
//! Every bout is a function of its seed and the genome, so a run replays
//! whatever `--threads` is.
//!
//! **`--agent` breeds the other side**: the agent's tactics brain
//! (`agent::combat`, `jev-bot --tactics ga1:…`), the fight-or-run call and
//! the hand, scored from the agent's side against wolf packs (half of them
//! on the bred wolf genome, `--wolves`, by default the one commented in
//! `content/mobs.toml`) and the arena's stand-in players (rusher, kiter,
//! strafer). The held-out table adds the aimbot as the ceiling: a bred
//! brain must still lose to it as the sum does, because the eyes and hands
//! it plays through are a person's.
//!
//! ```text
//! cargo run --release -p server --example zoo -- --agent
//! cargo run --release -p server --example zoo -- --agent --bench ga1:...
//! ```

#[path = "../tests/common/mod.rs"]
mod common;
#[allow(dead_code, unused_imports)]
#[path = "../tests/arena/rig.rs"]
mod rig;

use server::agent::combat::Temperament;
use server::agent::hands::Preset;
use server::explorer::SurvivorOpts;
use server::mind::{Mind, MindConfig};
use sim_core::brain::{wolf_shape, Nets, NET_CHARGE, NO_TARGET, WOLF_NET_IN, WOLF_NET_PREFIX};
use sim_core::genome::{Genome, Net, TEXT_MAX};
use sim_core::limits::{HOTBAR_SLOTS, MAX_MOBS, TICK_HZ};
use sim_core::mob::{guard_site_of, kind_of, pack_of, MOB_WOLF};
use sim_core::movement::{Body, POS_XZ_Q};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// A bout's clock: 90 s from the pack's placing.
const LIMIT_TICKS: u32 = 90 * TICK_HZ;
/// Training bouts draw from this seed range, the confirm stage from the
/// next, the held-out table from below both.
const TRAIN_SEEDS: u32 = 1_000;
const CONFIRM_SEEDS: u32 = 2_000;
const SEED_SPAN: u32 = 1_000;

const GEARS: [(&str, Option<(&str, u16)>); 3] = [
    ("item.bat", None),
    ("item.spear_wood", None),
    ("item.bow", Some(("item.arrow_wood", 40))),
];
const GEAR_NAMES: [&str; 3] = ["rock", "spear", "bow"];
/// Temperaments a training bout draws. `Passive` (runs from every fight) is
/// kept out of training and put in the held-out table: one opponent no
/// selection ever saw.
const TRAINED: [Temperament; 3] = [
    Temperament::Defensive,
    Temperament::Opportunist,
    Temperament::Kos,
];

/// One bout's setting, all of it from the seed.
#[derive(Clone, Copy, Debug)]
struct Bout {
    seed: u32,
    preset: Preset,
    temperament: Temperament,
    gear: usize,
    pack: usize,
    dist_m: f32,
    bearing: u16,
}

fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn bout_of(seed: u32, held_out: bool) -> Bout {
    let r = |k: u64| mix64((seed as u64) << 8 | k);
    let temperament = if held_out && r(1) % 4 == 0 {
        Temperament::Passive
    } else {
        TRAINED[(r(2) % TRAINED.len() as u64) as usize]
    };
    Bout {
        seed,
        preset: Preset::ALL[(r(3) % 4) as usize],
        temperament,
        gear: (r(4) % GEARS.len() as u64) as usize,
        pack: 1 + (r(5) % 3) as usize,
        dist_m: 18.0 + (r(6) % 1_000) as f32 * 0.01,
        bearing: r(7) as u16,
    }
}

/// How a bout went, from the pack's side.
#[derive(Clone, Copy, Debug, Default)]
struct Outcome {
    /// The agent went down (wounded or dead).
    downed: bool,
    wolves_dead: u32,
    pack: u32,
    /// The agent's hp lost, as a share of its max (1 when downed).
    dealt: f32,
    /// The pack's hp lost, as a share of its whole.
    taken: f32,
    ticks: u32,
}

impl Outcome {
    /// What selection maximises: the agent down, and soon; blood drawn;
    /// wolves kept alive. A bout where the pack drew nothing costs.
    fn fitness(&self) -> f32 {
        let mut f = 0.5 * self.dealt.min(1.0)
            - 0.4 * self.wolves_dead as f32 / self.pack.max(1) as f32
            - 0.1 * self.taken;
        if self.downed {
            f += 1.0 + 0.25 * (1.0 - self.ticks as f32 / LIMIT_TICKS as f32);
        }
        if self.dealt <= 0.0 {
            f -= 0.2;
        }
        f
    }
}

/// Made-up fight snapshots for the base screen, in [`wolf_features`]'
/// ranges: a distance, reach when close, bearings, the target's buttons and
/// swing clock a fifth of the time each, health, the pack, the state.
///
/// [`wolf_features`]: sim_core::brain::wolf_features
fn probes(n: usize) -> Vec<[f32; WOLF_NET_IN]> {
    (0..n as u64)
        .map(|i| {
            let r = |k: u64| mix64(0x5c2e_e000 ^ i << 8 ^ k);
            let u = |k: u64| (r(k) % 10_000) as f32 / 10_000.0;
            let bit = |k: u64| if r(k) % 5 == 0 { 1.0 } else { 0.0 };
            let mut x = [0.0f32; WOLF_NET_IN];
            x[0] = u(0) * 4.0;
            x[1] = if x[0] < 0.25 { 1.0 } else { 0.0 };
            x[2] = u(2) * 2.0 - 1.0;
            x[3] = u(3) * 2.0 - 1.0;
            x[4] = u(4) * 2.0 - 1.0;
            x[5] = (u(5) * 2.0 - 1.0) * x[3];
            for (k, xk) in x.iter_mut().enumerate().take(12).skip(6) {
                *xk = bit(k as u64);
            }
            x[12] = u(12);
            x[13] = u(13);
            x[14] = (r(14) % 3) as f32 / 3.0;
            x[15] = (r(15) % 3) as f32 / 2.0;
            x[16] = bit(16);
            x[17] = bit(17);
            x[18] = (r(18) % 3) as f32 / 3.0;
            x[19] = bit(19);
            x[20] = (r(20) % 2) as f32 / 3.0;
            x[21 + (r(21) % 3) as usize] = 1.0;
            x
        })
        .collect()
}

/// The share of `probes` a genome answers with a charge.
fn charge_share(g: &Genome, probes: &[[f32; WOLF_NET_IN]]) -> f32 {
    let net = Net::build(wolf_shape(), g);
    let charges = probes
        .iter()
        .filter(|x| net.choose(&x[..]) == NET_CHARGE)
        .count();
    charges as f32 / probes.len().max(1) as f32
}

/// The first free pack with at least three slots: not a guard, the same
/// pack. Its first `n` slots are the bout's wolves.
fn free_pack(n: usize) -> Vec<usize> {
    let wolves: Vec<usize> = (0..MAX_MOBS)
        .filter(|&s| kind_of(s) == MOB_WOLF && guard_site_of(s).is_none())
        .collect();
    for &s in &wolves {
        let Some(p) = pack_of(s) else { continue };
        let mates: Vec<usize> = wolves
            .iter()
            .copied()
            .filter(|&o| pack_of(o) == Some(p))
            .collect();
        if mates.len() >= 3 {
            return mates.into_iter().take(n).collect();
        }
    }
    panic!("the roster has no free pack of three");
}

/// One bout, with `genome` driving the pack (or the rule table, `None`)
/// and `tactics` the agent's fights (or the sum, `None`).
fn play(b: &Bout, genome: Option<&Genome>, tactics: Option<&Genome>) -> Outcome {
    let content = common::content();
    let me = rig::ID + 2 * b.seed;
    let mind = Mind::inline(rig::Sentry, MindConfig::default()).unwrap();
    let opts = SurvivorOpts {
        skill: b.preset.skill(),
        temperament: b.temperament,
        tactics: tactics.copied(),
        ..SurvivorOpts::default()
    };
    let mut a = rig::Arena::seated(opts, mind, true, me, rig::arena_ground(), false);
    // The island plays at the medium tier, and the brain drives from it.
    let medium = a.shard.world.mob.tiers[1];
    a.shard.world.mob.tiers[0] = medium;
    let mut nets = Nets::default();
    if let Some(g) = genome {
        nets.install(MOB_WOLF, *g, 0);
    }
    *a.shard.world.nets = nets;
    // The roster enrols on the first armed tick; then it is cleared, and
    // nothing hatches until the pack is placed by hand.
    a.step();
    for m in a.shard.world.mobs.m.iter_mut() {
        m.alive = false;
        m.homed = false;
    }
    assert!(a.until(900, |a| a.bot.goal().is_some()), "{}", a.explain());
    let (item, rounds) = GEARS[b.gear];
    let (arm, ammo) = (
        common::stack(&content, item),
        rounds.map(|(r, n)| {
            let mut s = common::stack(&content, r);
            s.count = n;
            s
        }),
    );
    a.stage(me, |p| {
        for s in p.inv.iter_mut() {
            *s = Default::default();
        }
        p.inv[0] = arm;
        if let Some(s) = ammo {
            p.inv[HOTBAR_SLOTS + 3] = s;
        }
        p.hp = p.hp_max;
    });
    a.step();

    let at = a.player(me).body;
    let (fx, fz) = sim_core::yaw_dir(b.bearing);
    let (cx, cz) = (
        at.qx as f32 * POS_XZ_Q + fx * b.dist_m,
        at.qz as f32 * POS_XZ_Q + fz * b.dist_m,
    );
    let hp = a.shard.world.mob_now().def(MOB_WOLF).hp;
    let slots = free_pack(b.pack);
    for (k, &s) in slots.iter().enumerate() {
        // Side by side across the line to the agent, two metres apart.
        let side = k as f32 * 2.0 - (slots.len() - 1) as f32;
        let body = Body::at(common::SEED, &a.haven, cx - fz * side, cz + fx * side);
        let m = &mut a.shard.world.mobs.m[s];
        m.homed = true;
        m.alive = true;
        m.hp = hp;
        m.body = body;
        m.home_qx = body.qx;
        m.home_qz = body.qz;
        m.respawn_at = u64::MAX;
        m.state = sim_core::brain::AiState::Idle;
        m.state_until = 0;
        m.target = NO_TARGET;
        m.roused_until = 0;
        m.calm_until = 0;
        m.tries = 0;
        m.awake = false;
        m.pick = 0;
        m.pressed_at = 0;
        m.path.clear();
    }

    let mut ticks = 0;
    while ticks < LIMIT_TICKS {
        let p = a.player(me);
        if p.wounded || p.dead {
            break;
        }
        if slots.iter().all(|&s| !a.shard.world.mobs.m[s].alive) {
            break;
        }
        a.step();
        ticks += 1;
    }
    let p = a.player(me);
    let downed = p.wounded || p.dead;
    let dealt = if downed {
        1.0
    } else {
        1.0 - p.hp as f32 / p.hp_max.max(1) as f32
    };
    let (mut dead, mut lost) = (0, 0u32);
    for &s in &slots {
        let m = &a.shard.world.mobs.m[s];
        if !m.alive {
            dead += 1;
            lost += hp as u32;
        } else {
            lost += hp.saturating_sub(m.hp) as u32;
        }
    }
    Outcome {
        downed,
        wolves_dead: dead,
        pack: slots.len() as u32,
        dealt,
        taken: lost as f32 / (hp as u32 * slots.len() as u32).max(1) as f32,
        ticks,
    }
}

/// `f` over every job on `threads` workers; results in job order (the
/// arena's own `sweep`).
fn pool<J: Sync, O: Send + Clone + Default>(
    jobs: &[J],
    threads: usize,
    f: impl Fn(&J) -> O + Sync,
) -> Vec<O> {
    let next = AtomicUsize::new(0);
    let mut out = vec![O::default(); jobs.len()];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(i) else {
                            return done;
                        };
                        done.push((i, f(job)));
                    }
                })
            })
            .collect();
        for w in workers {
            for (i, o) in w.join().unwrap() {
                out[i] = o;
            }
        }
    });
    out
}

/// Every (genome, bout) pair: the pack's side.
fn run(jobs: &[(Option<Genome>, Bout)], threads: usize) -> Vec<Outcome> {
    pool(jobs, threads, |(g, b)| play(b, g.as_ref(), None))
}

/// Mean fitness of each genome over the same bouts.
fn score(genomes: &[Option<Genome>], bouts: &[Bout], threads: usize) -> Vec<f32> {
    let jobs: Vec<_> = genomes
        .iter()
        .flat_map(|g| bouts.iter().map(move |b| (*g, *b)))
        .collect();
    let out = run(&jobs, threads);
    out.chunks(bouts.len())
        .map(|c| c.iter().map(Outcome::fitness).sum::<f32>() / c.len() as f32)
        .collect()
}

fn batch(from: u32, n: usize, at: u64) -> Vec<Bout> {
    (0..n as u64)
        .map(|i| {
            bout_of(
                from + ((at * n as u64 + i) % SEED_SPAN as u64) as u32,
                false,
            )
        })
        .collect()
}

fn text(g: &Genome) -> String {
    let mut buf = [0u8; TEXT_MAX];
    let n = g.encode(WOLF_NET_PREFIX, &mut buf);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// A summary of some outcomes: wolves' win rate, the agent's, how long a
/// kill took, blood drawn and wolves lost.
fn line(name: &str, o: &[Outcome]) -> String {
    let n = o.len().max(1) as f32;
    let downs: Vec<_> = o.iter().filter(|o| o.downed).collect();
    let cleared = o
        .iter()
        .filter(|o| o.wolves_dead == o.pack && !o.downed)
        .count();
    let ttk = if downs.is_empty() {
        f32::NAN
    } else {
        downs.iter().map(|o| o.ticks as f32).sum::<f32>() / downs.len() as f32 / TICK_HZ as f32
    };
    format!(
        "{name:<26} {:>4} {:>6.0}% {:>6.0}% {:>7.1} {:>7.0}% {:>6.2} {:>7.3}",
        o.len(),
        downs.len() as f32 / n * 100.0,
        cleared as f32 / n * 100.0,
        ttk,
        o.iter().map(|o| o.dealt).sum::<f32>() / n * 100.0,
        o.iter().map(|o| o.wolves_dead as f32).sum::<f32>() / n,
        o.iter().map(|o| o.fitness()).sum::<f32>() / n,
    )
}

/// The held-out table: the rule-table wolf against the bred one, overall
/// and by the agent's weapon and hands.
fn bench(g: &Genome, n: usize, threads: usize) {
    let bouts: Vec<Bout> = (1..=n as u32).map(|s| bout_of(s, true)).collect();
    let mut jobs: Vec<(Option<Genome>, Bout)> = bouts.iter().map(|b| (None, *b)).collect();
    jobs.extend(bouts.iter().map(|b| (Some(*g), *b)));
    let t = Instant::now();
    let out = run(&jobs, threads);
    let (table, bred) = out.split_at(bouts.len());
    println!(
        "\nheld out: {n} bouts (seeds 1-{n}), {:.0} s\n{:<26} {:>4} {:>7} {:>7} {:>7} {:>8} {:>6} {:>7}",
        t.elapsed().as_secs_f32(),
        "",
        "n",
        "downs",
        "clears",
        "ttk s",
        "blood",
        "lost",
        "fit"
    );
    println!("{}", line("rule-table wolf", table));
    println!("{}", line("bred wolf", bred));
    let pick = |o: &[Outcome], f: &dyn Fn(&Bout) -> bool| -> Vec<Outcome> {
        o.iter()
            .zip(&bouts)
            .filter(|(_, b)| f(b))
            .map(|(o, _)| *o)
            .collect()
    };
    for (gear, weapon) in GEAR_NAMES.iter().enumerate() {
        for (name, set) in [("table", table), ("bred", bred)] {
            let o = pick(set, &|b| b.gear == gear);
            println!("{}", line(&format!("  vs {weapon} ({name})"), &o));
        }
    }
    for (label, f) in [
        (
            "Good spear",
            &(|b: &Bout| b.gear == 1 && b.preset == Preset::Good) as &dyn Fn(&Bout) -> bool,
        ),
        ("Pro bow", &|b: &Bout| {
            b.gear == 2 && b.preset == Preset::Pro
        }),
        ("Passive (unseen)", &|b: &Bout| {
            b.temperament == Temperament::Passive
        }),
        ("lone wolf", &|b: &Bout| b.pack == 1),
    ] {
        for (name, set) in [("table", table), ("bred", bred)] {
            let o = pick(set, f);
            println!("{}", line(&format!("  {label} ({name})"), &o));
        }
    }
}

struct Args {
    gens: usize,
    kids: usize,
    batch: usize,
    confirm: usize,
    bases: usize,
    screen: usize,
    held_out: usize,
    seed: u64,
    threads: usize,
    from: Option<Genome>,
    bench_only: Option<Genome>,
}

fn args() -> Args {
    let mut a = Args {
        gens: 100,
        kids: 15,
        batch: 24,
        confirm: 48,
        bases: 32,
        screen: 4096,
        held_out: 48,
        seed: 1,
        threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
        from: None,
        bench_only: None,
    };
    let mut it = std::env::args().skip(1);
    let genome = |s: Option<String>| -> Genome {
        let s = s.expect("a genome after the flag");
        Genome::decode(WOLF_NET_PREFIX, s.as_bytes())
            .unwrap_or_else(|e| panic!("not a gw1: genome ({e:?}): {s}"))
    };
    while let Some(flag) = it.next() {
        let mut num = || -> usize {
            it.next()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{flag} takes a number"))
        };
        match flag.as_str() {
            "--gens" => a.gens = num(),
            "--kids" => a.kids = num(),
            "--batch" => a.batch = num(),
            "--confirm" => a.confirm = num(),
            "--bases" => a.bases = num(),
            "--screen" => a.screen = num(),
            "--held-out" => a.held_out = num(),
            "--seed" => a.seed = num() as u64,
            "--threads" => a.threads = num().max(1),
            "--from" => a.from = Some(genome(it.next())),
            "--bench" => a.bench_only = Some(genome(it.next())),
            other => panic!("unknown flag {other} (see the header of examples/zoo.rs)"),
        }
    }
    a
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--agent") {
        agent::main();
        return;
    }
    let a = args();
    if let Some(g) = a.bench_only {
        bench(&g, a.held_out, a.threads);
        return;
    }
    let started = Instant::now();
    let draw = |k: u64| mix64(a.seed.wrapping_mul(0x1_0000_0001) ^ k);
    let mut parent = match a.from {
        Some(g) => g,
        None => {
            // The `bases` of `screen` unmutated nets that charge most, then
            // the best of those on the first batch.
            let probes = probes(256);
            let mut screened: Vec<(f32, Genome)> = (0..a.screen.max(a.bases) as u64)
                .map(|k| {
                    let g = Genome::new(draw(k) as u32);
                    (charge_share(&g, &probes), g)
                })
                .collect();
            screened.sort_by(|x, y| y.0.total_cmp(&x.0));
            println!(
                "screen: {} bases, the best {} charge {:.0}%-{:.0}% of the probes",
                screened.len(),
                a.bases,
                screened[a.bases.min(screened.len()) - 1].0 * 100.0,
                screened[0].0 * 100.0
            );
            let bases: Vec<Option<Genome>> = screened
                .iter()
                .take(a.bases)
                .map(|(_, g)| Some(*g))
                .collect();
            let fit = score(&bases, &batch(TRAIN_SEEDS, a.batch, 0), a.threads);
            let best = (0..fit.len())
                .max_by(|&i, &j| fit[i].total_cmp(&fit[j]))
                .unwrap();
            println!(
                "base: best of {} is {:#010x} at {:.3} (worst {:.3}), {:.0} s",
                a.bases,
                bases[best].unwrap().base,
                fit[best],
                fit.iter().copied().fold(f32::INFINITY, f32::min),
                started.elapsed().as_secs_f32()
            );
            bases[best].unwrap()
        }
    };
    let mut confirms = 0u64;
    for gen in 0..a.gens {
        if parent.full() {
            println!("the genome is full ({} mutations): done", parent.len());
            break;
        }
        let bouts = batch(TRAIN_SEEDS, a.batch, gen as u64 + 1);
        let mut pool = vec![Some(parent)];
        pool.extend(
            (0..a.kids as u64).map(|k| parent.child(draw(1 << 32 | (gen as u64) << 16 | k) as u16)),
        );
        let fit = score(&pool, &bouts, a.threads);
        let best = (1..fit.len())
            .max_by(|&i, &j| fit[i].total_cmp(&fit[j]))
            .unwrap();
        let mut note = String::new();
        if fit[best] > fit[0] {
            let fresh = batch(CONFIRM_SEEDS, a.confirm, confirms);
            confirms += 1;
            let again = score(&[Some(parent), pool[best]], &fresh, a.threads);
            if again[1] > again[0] {
                parent = pool[best].unwrap();
                note = format!("kept ({:.3} > {:.3} on confirm)", again[1], again[0]);
                // A checkpoint: a stopped run goes on from here with `--from`.
                eprintln!("checkpoint gen {gen}: {}", text(&parent));
            } else {
                note = format!("refused ({:.3} <= {:.3} on confirm)", again[1], again[0]);
            }
        }
        println!(
            "gen {gen:>3}: parent {:.3}, best child {:.3} {note:<36} muts {:>3}  {:.0} s",
            fit[0],
            fit[best],
            parent.len(),
            started.elapsed().as_secs_f32()
        );
    }
    println!("\ngenome ({} mutations): {}", parent.len(), text(&parent));
    bench(&parent, a.held_out, a.threads);
}

/// `--agent`: breed the agent's tactics brain (see the header).
mod agent {
    use super::*;
    use rig::{Gear, Seat, Setup};
    use server::agent::combat::{TACTICS_IN, TACTICS_PREFIX, TACTIC_RUN};

    /// One fight from the agent's side: a wolf pack (on the bred wolf
    /// genome or the rule table) or one of the arena's stand-in players.
    /// Copied once per bout, like `school`'s `Who`.
    #[derive(Clone, Copy)]
    #[allow(clippy::large_enum_variant)]
    enum Fight {
        Wolves { bout: Bout, bred: bool },
        Duel(Setup),
    }

    impl Fight {
        fn foe(&self) -> &'static str {
            match self {
                Fight::Wolves { bred: true, .. } => "bred wolves",
                Fight::Wolves { bred: false, .. } => "rule-table wolves",
                Fight::Duel(s) => s.seat.name(),
            }
        }
    }

    /// How it went for the agent.
    #[derive(Clone, Copy, Debug, Default)]
    struct Went {
        survived: bool,
        foe_down: bool,
        /// Share of the foe's hp it took, and of its own it lost.
        dealt: f32,
        taken: f32,
    }

    impl Went {
        /// Winning is worth more than getting away, and getting away more
        /// than going down.
        fn fitness(&self) -> f32 {
            let bit = |b: bool| if b { 1.0 } else { 0.0 };
            0.5 * bit(self.survived) + bit(self.foe_down) + 0.5 * self.dealt - 0.5 * self.taken
        }
    }

    const MELEE: [(Gear, Gear); 5] = [
        (Gear::ROCK, Gear::ROCK),
        (Gear::SPEAR, Gear::SPEAR),
        (Gear::BOW, Gear::BOW),
        (Gear::SPEAR, Gear::BOW),
        (Gear::BOW, Gear::SPEAR),
    ];

    fn fight_of(seed: u32, held_out: bool) -> Fight {
        let r = |k: u64| mix64(((seed as u64) << 8) ^ 0xa9e ^ k) as usize;
        if r(1) % 2 == 0 {
            let mut bout = bout_of(seed, held_out);
            // A passive body runs from every fight whatever its brain says.
            if bout.temperament == Temperament::Passive {
                bout.temperament = Temperament::Opportunist;
            }
            return Fight::Wolves {
                bout,
                bred: r(2) % 2 == 0,
            };
        }
        let seat = [Seat::Rusher, Seat::Kiter, Seat::Strafer][r(3) % 3];
        let (mine, theirs) = MELEE[r(4) % MELEE.len()];
        let mut s = Setup::new(seat, mine, theirs, [5.0, 15.0, 30.0][r(5) % 3], seed);
        s.preset = Preset::ALL[r(6) % 4];
        s.temperament = TRAINED[r(7) % TRAINED.len()];
        Fight::Duel(s)
    }

    fn play_fight(f: &Fight, tactics: Option<&Genome>, wolves: &Genome) -> Went {
        match *f {
            Fight::Wolves { bout, bred } => {
                let o = play(&bout, bred.then_some(wolves), tactics);
                Went {
                    survived: !o.downed,
                    foe_down: o.wolves_dead == o.pack,
                    dealt: o.taken,
                    taken: o.dealt,
                }
            }
            Fight::Duel(mut s) => {
                s.tactics = tactics.copied();
                let (_, r) = rig::bout(s);
                Went {
                    survived: !r.lost,
                    foe_down: r.won,
                    dealt: (r.hits as f32 / 3.0).min(1.0),
                    taken: (r.their_hits as f32 / 3.0).min(1.0),
                }
            }
        }
    }

    fn score(
        genomes: &[Option<Genome>],
        fights: &[Fight],
        wolves: &Genome,
        threads: usize,
    ) -> Vec<f32> {
        let jobs: Vec<_> = genomes
            .iter()
            .flat_map(|g| fights.iter().map(move |f| (*g, *f)))
            .collect();
        pool(&jobs, threads, |(g, f)| play_fight(f, g.as_ref(), wolves))
            .chunks(fights.len())
            .map(|c| c.iter().map(Went::fitness).sum::<f32>() / c.len() as f32)
            .collect()
    }

    fn batch(from: u32, n: usize, at: u64) -> Vec<Fight> {
        (0..n as u64)
            .map(|i| {
                fight_of(
                    from + ((at * n as u64 + i) % SEED_SPAN as u64) as u32,
                    false,
                )
            })
            .collect()
    }

    fn text(g: &Genome) -> String {
        let mut buf = [0u8; TEXT_MAX];
        let n = g.encode(TACTICS_PREFIX, &mut buf);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    /// The share of made-up fights a genome calls as the sum does: run
    /// when the sum says losing, stay when it does not. A random net runs
    /// from half its fights; breeding starts nearer the sum.
    fn agrees(g: &Genome, probes: &[[f32; TACTICS_IN]]) -> f32 {
        let net = Net::build(server::agent::combat::tactics_shape(), g);
        let same = probes
            .iter()
            .filter(|x| (net.choose(&x[..]) == TACTIC_RUN) == (x[11] > 0.5))
            .count();
        same as f32 / probes.len().max(1) as f32
    }

    fn probes(n: usize) -> Vec<[f32; TACTICS_IN]> {
        (0..n as u64)
            .map(|i| {
                let r = |k: u64| mix64(0x7ac7 ^ (i << 8) ^ k);
                let u = |k: u64| (r(k) % 10_000) as f32 / 10_000.0;
                let bit = |k: u64| if r(k) % 3 == 0 { 1.0 } else { 0.0 };
                let mut x = [0.0f32; TACTICS_IN];
                for (k, xk) in x.iter_mut().enumerate() {
                    *xk = match k {
                        0..=7 => u(k as u64) * 2.0,
                        12..=14 => 0.0,
                        15 => (r(15) % 3) as f32 / 3.0,
                        _ => bit(k as u64),
                    };
                }
                x[12 + (r(12) % 3) as usize] = 1.0;
                x
            })
            .collect()
    }

    /// The bred wolf genome commented in `content/mobs.toml`.
    fn content_wolves() -> Genome {
        let text = std::fs::read_to_string(common::root().join("content/mobs.toml"))
            .expect("content/mobs.toml");
        let line = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("# genome = \""))
            .expect("a bred wolf genome commented in content/mobs.toml (or pass --wolves)");
        Genome::decode(WOLF_NET_PREFIX, line.trim_end_matches('"').as_bytes())
            .expect("the commented wolf genome reads")
    }

    /// The held-out table: the sum against the bred brain, foe by foe,
    /// the aimbot among them as the ceiling.
    fn bench(g: &Genome, n: usize, wolves: &Genome, threads: usize) {
        let mut fights: Vec<Fight> = (1..=n as u32).map(|s| fight_of(s, true)).collect();
        for (k, gear) in [Gear::ROCK, Gear::SPEAR].into_iter().enumerate() {
            for (j, dist) in [5.0, 15.0, 30.0].into_iter().enumerate() {
                for seed in 0..4u32 {
                    let s = Setup::new(
                        Seat::Aimbot,
                        gear,
                        gear,
                        dist,
                        900 + seed * 8 + (k * 3 + j) as u32,
                    );
                    fights.push(Fight::Duel(s));
                }
            }
        }
        let jobs: Vec<_> = [None, Some(*g)]
            .iter()
            .flat_map(|t| fights.iter().map(move |f| (*t, *f)))
            .collect();
        let t = Instant::now();
        let out = pool(&jobs, threads, |(tac, f)| {
            play_fight(f, tac.as_ref(), wolves)
        });
        let (sum, bred) = out.split_at(fights.len());
        println!(
            "\nheld out: {} fights, {:.0} s\n{:<34} {:>4} {:>6} {:>6} {:>7} {:>7} {:>7}",
            fights.len(),
            t.elapsed().as_secs_f32(),
            "",
            "n",
            "wins",
            "downs",
            "dealt",
            "taken",
            "fit"
        );
        let line = |name: &str, w: &[&Went]| {
            let n = w.len().max(1) as f32;
            println!(
                "{name:<34} {:>4} {:>5.0}% {:>5.0}% {:>6.0}% {:>6.0}% {:>7.3}",
                w.len(),
                w.iter().filter(|w| w.foe_down).count() as f32 / n * 100.0,
                w.iter().filter(|w| !w.survived).count() as f32 / n * 100.0,
                w.iter().map(|w| w.dealt).sum::<f32>() / n * 100.0,
                w.iter().map(|w| w.taken).sum::<f32>() / n * 100.0,
                w.iter().map(|w| w.fitness()).sum::<f32>() / n
            );
        };
        for foe in [
            "all but the aimbot",
            "rule-table wolves",
            "bred wolves",
            "rusher",
            "kiter",
            "strafer",
            "aimbot",
        ] {
            for (who, set) in [("sum", sum), ("bred", bred)] {
                let w: Vec<&Went> = set
                    .iter()
                    .zip(&fights)
                    .filter(|(_, f)| match foe {
                        "all but the aimbot" => f.foe() != "aimbot",
                        other => f.foe() == other,
                    })
                    .map(|(w, _)| w)
                    .collect();
                line(&format!("{foe} ({who})"), &w);
            }
        }
        let aimbot: Vec<&Went> = bred
            .iter()
            .zip(&fights)
            .filter(|(_, f)| f.foe() == "aimbot")
            .map(|(w, _)| w)
            .collect();
        let downs = aimbot.iter().filter(|w| !w.survived).count();
        println!(
            "ceiling: the bred brain went down in {downs}/{} aimbot bouts ({:.0}%; the arena gate's floor for the sum is 75%)",
            aimbot.len(),
            downs as f32 * 100.0 / aimbot.len().max(1) as f32
        );
    }

    pub fn main() {
        let mut it = std::env::args().skip(2);
        let (mut gens, mut kids, mut batch_n, mut confirm, mut bases) =
            (60usize, 15usize, 24usize, 48usize, 32usize);
        let (mut screen, mut held, mut seed) = (4096usize, 96usize, 1u64);
        let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let (mut from, mut bench_only, mut wolves) = (None, None, None);
        let tactics = |s: Option<String>| -> Genome {
            let s = s.expect("a genome after the flag");
            Genome::decode(TACTICS_PREFIX, s.as_bytes())
                .unwrap_or_else(|e| panic!("not a ga1: genome ({e:?}): {s}"))
        };
        while let Some(flag) = it.next() {
            let mut num = || -> usize {
                it.next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| panic!("{flag} takes a number"))
            };
            match flag.as_str() {
                "--gens" => gens = num(),
                "--kids" => kids = num(),
                "--batch" => batch_n = num(),
                "--confirm" => confirm = num(),
                "--bases" => bases = num(),
                "--screen" => screen = num(),
                "--held-out" => held = num(),
                "--seed" => seed = num() as u64,
                "--threads" => threads = num().max(1),
                "--from" => from = Some(tactics(it.next())),
                "--bench" => bench_only = Some(tactics(it.next())),
                "--wolves" => {
                    let s = it.next().expect("--wolves gw1:...");
                    wolves =
                        Some(Genome::decode(WOLF_NET_PREFIX, s.as_bytes()).expect("a gw1: genome"));
                }
                other => panic!("unknown flag {other} (see the header of examples/zoo.rs)"),
            }
        }
        let wolves = wolves.unwrap_or_else(content_wolves);
        if let Some(g) = bench_only {
            bench(&g, held, &wolves, threads);
            return;
        }
        let started = Instant::now();
        let draw = |k: u64| mix64(seed.wrapping_mul(0x1_0000_0001) ^ 0xa9e ^ k);
        let mut parent = match from {
            Some(g) => g,
            None => {
                let probes = probes(256);
                let mut screened: Vec<(f32, Genome)> = (0..screen.max(bases) as u64)
                    .map(|k| {
                        let g = Genome::new(draw(k) as u32);
                        (agrees(&g, &probes), g)
                    })
                    .collect();
                screened.sort_by(|x, y| y.0.total_cmp(&x.0));
                let pool: Vec<Option<Genome>> =
                    screened.iter().take(bases).map(|(_, g)| Some(*g)).collect();
                let fit = score(&pool, &batch(TRAIN_SEEDS, batch_n, 0), &wolves, threads);
                let best = (0..fit.len())
                    .max_by(|&i, &j| fit[i].total_cmp(&fit[j]))
                    .unwrap();
                println!(
                    "screen: {} bases, the best {bases} call {:.0}%-{:.0}% of probes as the sum; best on the first batch {:.3}, {:.0} s",
                    screened.len(),
                    screened[bases.min(screened.len()) - 1].0 * 100.0,
                    screened[0].0 * 100.0,
                    fit[best],
                    started.elapsed().as_secs_f32()
                );
                pool[best].unwrap()
            }
        };
        let mut confirms = 0u64;
        for gen in 0..gens {
            if parent.full() {
                break;
            }
            let fights = batch(TRAIN_SEEDS, batch_n, gen as u64 + 1);
            let mut pool = vec![None, Some(parent)];
            pool.extend(
                (0..kids as u64)
                    .map(|k| parent.child(draw((1 << 32) | ((gen as u64) << 16) | k) as u16)),
            );
            let fit = score(&pool, &fights, &wolves, threads);
            let best = (2..fit.len())
                .max_by(|&i, &j| fit[i].total_cmp(&fit[j]))
                .unwrap();
            let mut note = String::new();
            if fit[best] > fit[1] {
                let fresh = batch(CONFIRM_SEEDS, confirm, confirms);
                confirms += 1;
                let again = score(&[Some(parent), pool[best]], &fresh, &wolves, threads);
                if again[1] > again[0] {
                    parent = pool[best].unwrap();
                    note = format!("kept ({:.3} > {:.3} on confirm)", again[1], again[0]);
                    eprintln!("checkpoint gen {gen}: {}", text(&parent));
                } else {
                    note = format!("refused ({:.3} <= {:.3} on confirm)", again[1], again[0]);
                }
            }
            println!(
                "gen {gen:>3}: sum {:.3}, parent {:.3}, best child {:.3} {note:<36} muts {:>3}  {:.0} s",
                fit[0],
                fit[1],
                fit[best],
                parent.len(),
                started.elapsed().as_secs_f32()
            );
        }
        println!("\ngenome ({} mutations): {}", parent.len(), text(&parent));
        bench(&parent, held, &wolves, threads);
    }
}
