//! `school` — breed a distilled student past its teacher.
//!
//! A student (`distill.rs`, `ops/distill/train.py`) copies whoever taught it,
//! and can be no better than its copy. This breeds it the zoo's way
//! (`examples/zoo.rs`, `sim_core::genome`): a genome is the trained student
//! plus one 16-bit seed per accepted mutation, each a small step on every
//! layer (`Student::mutate`, the wolves' own `genome::perturb`), and the
//! base the seeds are mixed with is the student's own fingerprint, so one
//! student file and one `gs1:` line rebuild the bred mind exactly.
//!
//! It is scored on the board a wipe pays most, THE ISLAND (`standings.rs`),
//! on lockstep bench islands (`server::lockstep`, `jev-bot --bench --warp`):
//! the bench's seed, its arc hour and its spawn, `--bots` bodies of the
//! student each, `--minutes` of island time, as fast as the box runs it.
//! Every genome plays the same island variants, so a child is compared with
//! its parent on equal ground; the best child is tried again on variants it
//! has never seen and kept only if it wins there too; the end is a held-out
//! table against the scripted policy and the student as it was taught.
//!
//! ```text
//! cargo run --release -p server --example school -- --student student.json
//! cargo run --release -p server --example school -- --student student.json --gens 40 --out bred.json
//! cargo run --release -p server --example school -- --student student.json --bench gs1:...
//! ```

use server::agent::lock::{LockCode, LockSecret};
use server::distill::Student;
use server::explorer::{Survivor, SurvivorOpts};
use server::lockstep::{wallet, Island};
use server::mind::{Mind, MindConfig, Scripted};
use sim_core::genome::{Genome, TEXT_MAX};
use sim_core::limits::TICK_HZ;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// A bred student's text prefix: a `gates-student-1` file's weights plus
/// these seeds.
const PREFIX: &[u8; 4] = b"gs1:";
/// Island variants: selection draws from the first range, the confirm stage
/// from the second, the held-out table from the third.
const TRAIN: u64 = 10_000;
const CONFIRM: u64 = 20_000;
const HELD_OUT: u64 = 30_000;
const SPAN: u64 = 10_000;

/// Who plays an island. A job carries its genome by value (a few hundred
/// bytes, copied once per island), which is what the lint is about.
#[derive(Clone, Copy)]
#[allow(clippy::large_enum_variant)]
enum Who {
    Scripted,
    /// The base student with this genome's mutations.
    Bred(Genome),
}

struct Ctx {
    content: content::Content,
    cfg: server::config::ShardConfig,
    haven: sim_core::terrain::Haven,
    student: Student,
    bots: usize,
    minutes: u64,
}

impl Ctx {
    fn student(&self, g: &Genome) -> Student {
        let mut s = self.student.clone();
        for (k, &m) in g.muts().iter().enumerate() {
            s.mutate(g.base, k, m);
        }
        s
    }

    /// One island: the mean THE ISLAND of its bodies.
    fn play(&self, who: Who, variant: u64) -> f32 {
        let mut island = Island::new(
            &self.content,
            &self.cfg,
            self.haven,
            server::bench::BENCH_ARC_HOUR,
            Instant::now(),
            variant,
        )
        .expect("a lockstep island");
        for i in 0..self.bots {
            let mind = match who {
                Who::Scripted => Mind::inline(Scripted::default(), MindConfig::default()),
                Who::Bred(g) => Mind::inline(self.student(&g), MindConfig::default()),
            }
            .expect("an inline mind");
            let name = format!("school-{i}");
            let opts = SurvivorOpts {
                lock: Some(LockSecret::Code(LockCode::derive(b"warp", &name))),
                ..SurvivorOpts::default()
            };
            island
                .seat(Survivor::with(mind, opts), &wallet(i))
                .expect("a seat");
        }
        island.run(self.minutes * 60 * u64::from(TICK_HZ));
        let rules = island.core.standings.rules.clone();
        let total: u64 = (0..self.bots)
            .map(|i| {
                island
                    .row(i)
                    .map_or(0, |r| r.score(server::standings::BOARD_ISLAND, &rules))
            })
            .sum();
        total as f32 / self.bots as f32
    }
}

/// Every (who, variant) pair on `threads` workers; results in job order.
fn run(ctx: &Ctx, jobs: &[(Who, u64)], threads: usize) -> Vec<f32> {
    let next = AtomicUsize::new(0);
    let mut out = vec![0.0; jobs.len()];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(who, v)) = jobs.get(i) else {
                            return done;
                        };
                        done.push((i, ctx.play(who, v)));
                    }
                })
            })
            .collect();
        for w in workers {
            for (i, f) in w.join().unwrap() {
                out[i] = f;
            }
        }
    });
    out
}

/// Each player's mean over the same variants.
fn score(ctx: &Ctx, who: &[Who], variants: &[u64], threads: usize) -> Vec<f32> {
    let jobs: Vec<_> = who
        .iter()
        .flat_map(|w| variants.iter().map(move |v| (*w, *v)))
        .collect();
    run(ctx, &jobs, threads)
        .chunks(variants.len())
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect()
}

fn variants(from: u64, n: usize, at: u64) -> Vec<u64> {
    (0..n as u64)
        .map(|i| from + (at * n as u64 + i) % SPAN)
        .collect()
}

fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn text(g: &Genome) -> String {
    let mut buf = [0u8; TEXT_MAX];
    let n = g.encode(PREFIX, &mut buf);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// The held-out table: scripted, the student as taught, the bred student.
fn bench(ctx: &Ctx, g: &Genome, n: usize, threads: usize) {
    let held = variants(HELD_OUT, n, 0);
    let t = Instant::now();
    let base = Genome::new(g.base);
    let all = [Who::Scripted, Who::Bred(base), Who::Bred(*g)];
    let jobs: Vec<_> = all
        .iter()
        .flat_map(|w| held.iter().map(move |v| (*w, *v)))
        .collect();
    let out = run(ctx, &jobs, threads);
    println!(
        "\nheld out: {n} islands × {} bodies × {} min, {:.0} s",
        ctx.bots,
        ctx.minutes,
        t.elapsed().as_secs_f32()
    );
    for (name, c) in ["scripted", "taught student", "bred student"]
        .iter()
        .zip(out.chunks(n))
    {
        let mean = c.iter().sum::<f32>() / n as f32;
        let sd =
            (c.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / (n.max(2) - 1) as f32).sqrt();
        let zeros = c.iter().filter(|&&x| x == 0.0).count();
        println!(
            "{name:<16} THE ISLAND {mean:>8.1} ± {:>6.1} (95%), {zeros}/{n} islands scored nothing",
            1.96 * sd / (n as f32).sqrt()
        );
    }
}

fn main() {
    let mut it = std::env::args().skip(1);
    let (mut path, mut out, mut bench_only) = (None, None, None);
    let (mut gens, mut kids, mut islands, mut confirm, mut held) = (40usize, 15usize, 4, 8, 16);
    let (mut bots, mut minutes, mut seed) = (2usize, 45u64, 1u64);
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    while let Some(flag) = it.next() {
        let mut next = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--student" => path = Some(next()),
            "--out" => out = Some(next()),
            "--bench" => bench_only = Some(next()),
            "--gens" => gens = next().parse().expect("--gens"),
            "--kids" => kids = next().parse().expect("--kids"),
            "--islands" => islands = next().parse().expect("--islands"),
            "--confirm" => confirm = next().parse().expect("--confirm"),
            "--held-out" => held = next().parse().expect("--held-out"),
            "--bots" => bots = next().parse().expect("--bots"),
            "--minutes" => minutes = next().parse().expect("--minutes"),
            "--seed" => seed = next().parse().expect("--seed"),
            "--threads" => threads = next().parse::<usize>().expect("--threads").max(1),
            other => panic!("unknown flag {other} (see the header of examples/school.rs)"),
        }
    }
    let path = path.expect("--student FILE (a gates-student-1 file from ops/distill/train.py)");
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read the student"))
            .expect("the student is JSON");
    let student = Student::from_json(&raw).expect("a student this build can play");
    let cfg = server::agent_demo::local_config().expect("the bench island's config");
    let ctx = Ctx {
        content: content::Content::load_dir(&server::agent_demo::local_content()).expect("content"),
        haven: sim_core::terrain::haven(cfg.seed),
        cfg,
        student,
        bots,
        minutes,
    };
    let base = ctx.student.fingerprint();
    if let Some(text) = bench_only {
        let g = Genome::decode(PREFIX, text.as_bytes()).expect("a gs1: genome");
        assert_eq!(g.base, base, "that genome was bred from another student");
        bench(&ctx, &g, held, threads);
        return;
    }
    let started = Instant::now();
    let draw = |k: u64| mix64(seed.wrapping_mul(0x1_0000_0001) ^ k);
    let mut parent = Genome::new(base);
    let mut confirms = 0u64;
    for gen in 0..gens {
        if parent.full() {
            break;
        }
        let vs = variants(TRAIN, islands, gen as u64);
        let mut pool = vec![Who::Bred(parent)];
        pool.extend((0..kids as u64).map(|k| {
            Who::Bred(
                parent
                    .child(draw((gen as u64) << 16 | k) as u16)
                    .expect("not full"),
            )
        }));
        let fit = score(&ctx, &pool, &vs, threads);
        let best = (1..fit.len())
            .max_by(|&i, &j| fit[i].total_cmp(&fit[j]))
            .unwrap();
        let mut note = String::new();
        if fit[best] > fit[0] {
            let fresh = variants(CONFIRM, confirm, confirms);
            confirms += 1;
            let again = score(&ctx, &[Who::Bred(parent), pool[best]], &fresh, threads);
            if again[1] > again[0] {
                if let Who::Bred(g) = pool[best] {
                    parent = g;
                }
                note = format!("kept ({:.1} > {:.1} on confirm)", again[1], again[0]);
                eprintln!("checkpoint gen {gen}: {}", text(&parent));
            } else {
                note = format!("refused ({:.1} <= {:.1} on confirm)", again[1], again[0]);
            }
        }
        println!(
            "gen {gen:>3}: parent {:.1}, best child {:.1} {note:<34} muts {:>3}  {:.0} s",
            fit[0],
            fit[best],
            parent.len(),
            started.elapsed().as_secs_f32()
        );
    }
    println!("\ngenome ({} mutations): {}", parent.len(), text(&parent));
    if let Some(out) = out {
        let bred = ctx
            .student(&parent)
            .to_json(raw.get("check").map(|c| &c["obs"]));
        std::fs::write(&out, serde_json::to_string(&bred).unwrap()).expect("write --out");
        println!("bred student written to {out} (play it: jev-bot --student {out})");
    }
    bench(&ctx, &parent, held, threads);
}
