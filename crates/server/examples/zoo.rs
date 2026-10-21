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

/// One bout, with `genome` driving the pack (or the rule table, `None`).
fn play(b: &Bout, genome: Option<&Genome>) -> Outcome {
    let content = common::content();
    let me = rig::ID + 2 * b.seed;
    let mind = Mind::inline(rig::Sentry, MindConfig::default()).unwrap();
    let opts = SurvivorOpts {
        skill: b.preset.skill(),
        temperament: b.temperament,
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

/// Every (genome, bout) pair, on `threads` workers; results in job order
/// (the arena's own `sweep`).
fn run(jobs: &[(Option<Genome>, Bout)], threads: usize) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let mut out = vec![Outcome::default(); jobs.len()];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((g, b)) = jobs.get(i) else {
                            return done;
                        };
                        done.push((i, play(b, g.as_ref())));
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
