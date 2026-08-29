//! Dev probe: rank islands. Prints, per seed, the three axes the capture-seed
//! finding measured — max height, max slope, and the share of land carrying a
//! legible rock weight — plus the biome mix, the land share, and whether the
//! haven's three authored sites fill.
//!
//! **Why it exists.** A pass reported the shipped island as the flattest of
//! forty and the repo was one command from wiping a live shard over it; the
//! sweep behind it covered a quarter of each island
//! (`findings/note-20260814-granite-and-the-flattest-island.md`, retracted).
//! Choosing which world we ship is an operator act, and an operator should be
//! able to SEE the candidates rather than take a builder's word for one number
//! — including what a camera at one point can see, which is what `--at` is for
//! and what the retracted finding actually needed. It is **not a gate**;
//! `sim-core/tests/relief.rs` is, and it re-measures the shipped seed against
//! the set every run.
//!
//! ```text
//! cargo run -p sim-core --release --example seed_scan          # 44 islands, whole
//! cargo run -p sim-core --release --example seed_scan -- 42 20260814
//! cargo run -p sim-core --release --example seed_scan -- --at 1155,140,300
//! ```
//!
//! The third form is the capture probe's own spawn and a 300 m disc: what the
//! visual judge's camera stands in, as opposed to what the island contains.
//!
//! Numbers are counted, never timed, so they are worth the same on any box.

// Host-side probe: printing is its job, and `std::env::args` hands back
// `String`. Wall 1 and wall 3 ban both in SIM code — an example binary is not
// sim code, and `terrain_stats.rs` beside it carries the first of these for the
// same reason. Nothing here is compiled into the crate's lib.
#![allow(clippy::disallowed_macros, clippy::disallowed_types)]

use sim_core::terrain::{self, Biome};

/// Sample step in metres over the world square, `[0, ISLAND_SIZE)` on both
/// axes. 6 m is `relief.rs`'s step, and the two agree because a number here
/// that disagreed with the gate would be the worse kind of probe.
///
/// ⚠ **The window is the whole square, and it did not used to be.** `continent`
/// centres the island on `(ISLAND_SIZE/2, ISLAND_SIZE/2)` — 1024, 1024 — with
/// `CONTINENT_RADIUS` 960 and a ±100 m wobble, so world coordinates run 0..2048
/// and the shore is at ~1024 m from the centre. Every island sweep in this repo
/// sampled `-1024..1024` instead (`relief.rs`, `ground_identity.rs`, and this
/// file when it was written), a square whose *corner* is the island's centre:
/// **one quadrant, ~a quarter of the land, reported as "the island"** —
/// consistent between seeds, so the comparison held, but every absolute number
/// off it was a quarter-island number. Found and fixed 2026-08-14 while moving
/// the shipped seed.
const STEP: f32 = 6.0;
const LO: f32 = 0.0;
const HI: f32 = terrain::ISLAND_SIZE;

/// 32/255 is where a splat channel stops being rounding and starts being a
/// visible share of the blend (`relief.rs` uses the same floor).
const ROCK_LEGIBLE: u8 = 32;
const ROCK: usize = 3;

struct Island {
    seed: u64,
    land_permille: u32,
    max_h: f32,
    p50_h: f32,
    max_slope: f32,
    rock_permille: u32,
    biome_permille: [u32; 4],
    sites_live: u32,
    sites_complete: bool,
}

/// An optional disc to restrict the sweep to — `--at x,z,r`. The whole-island
/// numbers are not what a camera standing on one shore can see, and telling the
/// two apart is the difference between "this island is flat" and "the vantage
/// is in its flattest corner" (2026-08-14: the second was true and the first
/// was believed for four passes).
#[derive(Clone, Copy)]
struct Disc {
    x: f32,
    z: f32,
    r: f32,
}

fn scan(seed: u64, at: Option<Disc>) -> Island {
    let (mut land, mut rock, mut samples) = (0u32, 0u32, 0u32);
    let (mut max_h, mut max_slope) = (0f32, 0f32);
    let mut biomes = [0u32; 4];
    let mut heights: Vec<f32> = Vec::new();

    let mut z = LO + STEP * 0.5;
    while z < HI {
        let mut x = LO + STEP * 0.5;
        while x < HI {
            if let Some(d) = at {
                let (dx, dz) = (x - d.x, z - d.z);
                if dx * dx + dz * dz > d.r * d.r {
                    x += STEP;
                    continue;
                }
            }
            samples += 1;
            let h = terrain::height(seed, x, z);
            if h > 0.5 {
                land += 1;
                heights.push(h);
                if h > max_h {
                    max_h = h;
                }
                let s = terrain::slope(seed, x, z);
                if s > max_slope {
                    max_slope = s;
                }
                biomes[terrain::biome(h, terrain::moisture(seed, x, z)) as usize] += 1;
                if terrain::splat(seed, x, z)[ROCK] >= ROCK_LEGIBLE {
                    rock += 1;
                }
            }
            x += STEP;
        }
        z += STEP;
    }

    heights.sort_by(|a, b| a.partial_cmp(b).expect("terrain height is never NaN"));
    let p50_h = if heights.is_empty() {
        0.0
    } else {
        heights[heights.len() / 2]
    };
    let per_mille = |n: u32| {
        if land == 0 {
            0
        } else {
            (1000u64 * u64::from(n) / u64::from(land)) as u32
        }
    };
    let haven = terrain::haven(seed);

    Island {
        seed,
        land_permille: (1000u64 * u64::from(land) / u64::from(samples)) as u32,
        max_h,
        p50_h,
        max_slope,
        rock_permille: per_mille(rock),
        biome_permille: [
            per_mille(biomes[Biome::Beach as usize]),
            per_mille(biomes[Biome::Meadow as usize]),
            per_mille(biomes[Biome::Forest as usize]),
            per_mille(biomes[Biome::Highland as usize]),
        ],
        sites_live: terrain::sites_live(&haven),
        sites_complete: terrain::sites_complete(&haven),
    }
}

/// The default candidate set: `relief.rs`'s 24 (the seeds this repo names, plus
/// a fixed multiplicative sequence) and twenty date-shaped seeds, because a
/// shipped seed people will type wants to be legible as well as representative.
fn default_seeds() -> Vec<u64> {
    let mut s = vec![20260731, 0x0047_4154_4553, 0x1, 0xDEAD_BEEF];
    for i in 1..=20u64 {
        s.push(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    for d in 0..20u64 {
        s.push(20260801 + d);
    }
    s
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // `--at x,z,r` restricts every row to a disc — what a camera at (x, z)
    // stands in, rather than what the island contains.
    let mut at: Option<Disc> = None;
    if let Some(i) = argv.iter().position(|a| a == "--at") {
        let spec: Vec<f32> = argv
            .get(i + 1)
            .map(|s| s.split(',').filter_map(|v| v.parse().ok()).collect())
            .unwrap_or_default();
        assert!(spec.len() == 3, "--at wants x,z,r (metres, world coords)");
        at = Some(Disc {
            x: spec[0],
            z: spec[1],
            r: spec[2],
        });
    }
    let args: Vec<u64> = argv.iter().filter_map(|a| a.parse().ok()).collect();
    let seeds = if args.is_empty() {
        default_seeds()
    } else {
        args
    };

    if let Some(d) = at {
        println!(
            "restricted to a {:.0} m disc at ({:.0}, {:.0}) — NOT whole-island numbers\n",
            d.r, d.x, d.z
        );
    }
    let mut set: Vec<Island> = seeds.into_iter().map(|s| scan(s, at)).collect();
    set.sort_by_key(|i| i.rock_permille);

    println!(
        "{:>22}  {:>5}  {:>7}  {:>7}  {:>6}  {:>5}   {:>4} {:>4} {:>4} {:>4}  {:>5}",
        "seed", "land‰", "max h", "p50 h", "slope", "rock‰", "bch", "mdw", "for", "hgh", "sites"
    );
    for i in &set {
        println!(
            "{:>22}  {:>5}  {:>7.2}  {:>7.2}  {:>6.3}  {:>5}   {:>4} {:>4} {:>4} {:>4}  {:>5}",
            i.seed,
            i.land_permille,
            i.max_h,
            i.p50_h,
            i.max_slope,
            i.rock_permille,
            i.biome_permille[0],
            i.biome_permille[1],
            i.biome_permille[2],
            i.biome_permille[3],
            if i.sites_complete {
                format!("{}/3", i.sites_live)
            } else {
                format!("{}/3!", i.sites_live)
            },
        );
    }

    // The medians are what "representative" means, so print them rather than
    // leaving the reader to count rows.
    let n = set.len();
    let med = |mut v: Vec<f32>| -> f32 {
        v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
        v[n / 2]
    };
    let mut rocks: Vec<u32> = set.iter().map(|i| i.rock_permille).collect();
    rocks.sort_unstable();
    println!(
        "\nmedian of {n}: max h {:.2} m · slope {:.3} · rock {}‰ · land {}‰",
        med(set.iter().map(|i| i.max_h).collect()),
        med(set.iter().map(|i| i.max_slope).collect()),
        rocks[n / 2],
        {
            let mut l: Vec<u32> = set.iter().map(|i| i.land_permille).collect();
            l.sort_unstable();
            l[n / 2]
        }
    );
}
