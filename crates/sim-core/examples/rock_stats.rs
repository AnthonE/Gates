//! Dev probe: where the island's rock stands. Per biome, against the cliff
//! mask, and against the ore — the numbers `reference/ROCKS.md` §7 reads,
//! measured the way `terrain_stats` measures the forest. Not a gate; the
//! gates pin goldens (`tests/scatter.rs`, `tests/forest.rs`).
//!
//! Three questions, each one a claim the reference game makes about its own
//! rock and that ours has never been asked:
//!
//!  1. **Does rock stand where the cliffs are?** Their cliffs are placed
//!     "wherever the terrain falls off very steeply" (Devblog 54) and their
//!     `Cliffside` topology spawns ore nodes. Ours vetoes the cliff cell.
//!  2. **Is ore near rock?** "Ores are only spawning around other rock
//!     formations" (Devblog 105). Bushes are the control: an occupant with
//!     no reason to be near a rock, drawn from the same row.
//!  3. **Does rock cluster on its own, or only because the forest does?**
//!     The grove field scales the whole row, so a rock clumps with the trees;
//!     the highland has no trees to clump with.
//!
//! Whole-island window, 0..2048 on both axes (`tests/relief.rs`).

// Host-side tuning probe: printing is its job. The L5 wall bans
// format/print in SIM code; an example binary is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE, CELL_SIZE};

const N: usize = (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize;
/// Biome index for a cell under the land line: not a `Biome`, and counted
/// separately so the per-biome rows are land only.
const WATER: u8 = 4;
const BIOME_NAMES: [&str; 4] = ["beach", "meadow", "forest", "highland"];

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20260731);
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);

    let mut biome = vec![WATER; N];
    let mut occ = vec![Occupant::None; N];
    let mut cliff = vec![0u8; N];
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let i = (cz * CELLS_PER_SIDE + cx) as usize;
            let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let h = terrain::height(seed, x, z);
            if h >= terrain::LAND_MIN_H {
                biome[i] = terrain::biome(h, terrain::moisture(seed, x, z)) as u8;
                let sl = terrain::ground_slope(seed, &haven, x, z);
                cliff[i] = u8::from(sl > terrain::CLIFF_SLOPE_RATIO);
            }
            occ[i] = terrain::scatter(seed, &table, &haven, cx, cz).occupant;
        }
    }

    println!("seed {seed}");

    // 1 · Per biome: land, rock, the three ores, bushes, trees — and the
    // densities in per-hectare units so they compare with FORESTS.md §7.
    println!("per biome (cells are 64 m²; ha = cells / 156.25):");
    println!("  biome     cells   rock  stone  metal sulfur   bush   tree | rock/ha node/ha");
    for b in 0..4u8 {
        let (mut cells, mut rock, mut stone, mut metal, mut sulfur, mut bush, mut tree) =
            (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
        for i in 0..N {
            if biome[i] != b {
                continue;
            }
            cells += 1;
            match occ[i] {
                Occupant::Rock => rock += 1,
                Occupant::StoneNode => stone += 1,
                Occupant::MetalNode => metal += 1,
                Occupant::SulfurNode => sulfur += 1,
                Occupant::Bush => bush += 1,
                Occupant::Tree => tree += 1,
                _ => {}
            }
        }
        let ha = f64::from(cells) / 156.25;
        let nodes = stone + metal + sulfur;
        println!(
            "  {:<8} {cells:>6} {rock:>6} {stone:>6} {metal:>6} {sulfur:>6} {bush:>6} {tree:>6} | {:>7.2} {:>7.2}",
            BIOME_NAMES[b as usize],
            f64::from(rock) / ha,
            f64::from(nodes) / ha,
        );
    }

    // 2 · The cliff mask. A cliff cell is one whose centre is over the
    // ratio; a cliffside cell is a land cell that is not a cliff itself but
    // touches one in its 3×3. What stands in each, per 100 cells.
    let idx = |cx: i32, cz: i32| (cz * CELLS_PER_SIDE + cx) as usize;
    let mut side = vec![0u8; N];
    for cz in 1..CELLS_PER_SIDE - 1 {
        for cx in 1..CELLS_PER_SIDE - 1 {
            let i = idx(cx, cz);
            if biome[i] == WATER || cliff[i] != 0 {
                continue;
            }
            let mut touches = false;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    touches |= cliff[idx(cx + dx, cz + dz)] != 0;
                }
            }
            side[i] = u8::from(touches);
        }
    }
    let count = |pred: &dyn Fn(usize) -> bool, o: Occupant| -> u32 {
        (0..N).filter(|&i| pred(i) && occ[i] == o).count() as u32
    };
    let land: u32 = biome.iter().filter(|&&b| b != WATER).count() as u32;
    let cliff_n: u32 = cliff.iter().map(|&c| u32::from(c)).sum();
    let side_n: u32 = side.iter().map(|&s| u32::from(s)).sum();
    let open_n = land - cliff_n - side_n;
    let is_cliff = |i: usize| cliff[i] != 0;
    let is_side = |i: usize| side[i] != 0;
    let is_open = |i: usize| biome[i] != WATER && cliff[i] == 0 && side[i] == 0;
    println!(
        "cliff mask: land {land} cells, cliff {cliff_n} ({:.1}‰), cliffside {side_n} ({:.1}‰), open {open_n}",
        1000.0 * f64::from(cliff_n) / f64::from(land),
        1000.0 * f64::from(side_n) / f64::from(land),
    );
    let per100 = |n: u32, of: u32| 100.0 * f64::from(n) / f64::from(of.max(1));
    for (name, o) in [
        ("rock", Occupant::Rock),
        ("stone", Occupant::StoneNode),
        ("metal", Occupant::MetalNode),
        ("sulfur", Occupant::SulfurNode),
        ("bush", Occupant::Bush),
    ] {
        println!(
            "  {name:<6} per 100 cells: on cliff {:.2}  cliffside {:.2}  open {:.2}",
            per100(count(&is_cliff, o), cliff_n),
            per100(count(&is_side, o), side_n),
            per100(count(&is_open, o), open_n),
        );
    }

    // 2b · The same rates inside the highland alone, because a cliff is
    // mostly a highland thing and the row weight would otherwise be read as
    // a cliff effect. If cliffside ≈ open here, the cliff itself does
    // nothing to what stands beside it.
    let hi = 3u8;
    let (mut hs, mut ho) = (0u32, 0u32);
    let (mut hs_rock, mut ho_rock, mut hs_node, mut ho_node) = (0u32, 0u32, 0u32, 0u32);
    for i in 0..N {
        if biome[i] != hi || cliff[i] != 0 {
            continue;
        }
        let node = matches!(
            occ[i],
            Occupant::StoneNode | Occupant::MetalNode | Occupant::SulfurNode
        );
        if side[i] != 0 {
            hs += 1;
            hs_rock += u32::from(occ[i] == Occupant::Rock);
            hs_node += u32::from(node);
        } else {
            ho += 1;
            ho_rock += u32::from(occ[i] == Occupant::Rock);
            ho_node += u32::from(node);
        }
    }
    println!(
        "  highland only: rock per 100 cells cliffside {:.2} open {:.2}; nodes cliffside {:.2} open {:.2} ({hs} / {ho} cells)",
        per100(hs_rock, hs),
        per100(ho_rock, ho),
        per100(hs_node, hs),
        per100(ho_node, ho),
    );

    // 3 · Is ore near rock? Share of each occupant's cells with a Rock in
    // the surrounding 5×5 (≤ ~20 m), self excluded — island-wide, and then
    // only where the whole 5×5 is highland, so the highland row (which draws
    // both the rock and the ore) cannot masquerade as a coupling. Bush and
    // tree are the controls: occupants with no reason to stand near a rock.
    for (label, only_hi) in [("island-wide", false), ("all-highland windows", true)] {
        println!("share with a rock within the 5×5 (20 m) neighbourhood, {label}:");
        for (name, o) in [
            ("stone", Occupant::StoneNode),
            ("metal", Occupant::MetalNode),
            ("sulfur", Occupant::SulfurNode),
            ("bush", Occupant::Bush),
            ("tree", Occupant::Tree),
            ("rock", Occupant::Rock),
        ] {
            let (mut n, mut near) = (0u32, 0u32);
            for cz in 2..CELLS_PER_SIDE - 2 {
                for cx in 2..CELLS_PER_SIDE - 2 {
                    if occ[idx(cx, cz)] != o {
                        continue;
                    }
                    let (mut hit, mut all_hi) = (false, true);
                    for dz in -2..=2 {
                        for dx in -2..=2 {
                            let j = idx(cx + dx, cz + dz);
                            all_hi &= biome[j] == hi;
                            if (dx, dz) != (0, 0) && occ[j] == Occupant::Rock {
                                hit = true;
                            }
                        }
                    }
                    if only_hi && !all_hi {
                        continue;
                    }
                    n += 1;
                    near += u32::from(hit);
                }
            }
            println!(
                "  {name:<6} {near:>5} of {n:>5} = {:.1}%",
                100.0 * f64::from(near) / f64::from(n.max(1))
            );
        }
    }

    // 4 · Does rock cluster on its own? Index of dispersion (var / mean) of
    // an occupant's count over 40 m windows that lie wholly in one biome —
    // 1.0 is an independent draw, the forest's trees read ~3.0 (FORESTS.md
    // §7). The trees also give the grove field's own spread: for a count
    // thinned from a shared random rate, var/mean = 1 + mean × CV²(rate), so
    // the trees' (dispersion − 1) / mean is CV² and predicts what the SAME
    // field can do for anything rarer. A rock is rare.
    const R: i32 = 2;
    let window_stats = |b: u8, o: Occupant| -> (u32, f64, f64, u32) {
        let (mut windows, mut sum, mut sumsq, mut clusters3) = (0u32, 0f64, 0f64, 0u32);
        for cz in R..CELLS_PER_SIDE - R {
            for cx in R..CELLS_PER_SIDE - R {
                let (mut c, mut c3, mut same) = (0u32, 0u32, true);
                for dz in -R..=R {
                    for dx in -R..=R {
                        let j = idx(cx + dx, cz + dz);
                        same &= biome[j] == b;
                        let r = u32::from(occ[j] == o);
                        c += r;
                        if dx.abs() <= 1 && dz.abs() <= 1 {
                            c3 += r;
                        }
                    }
                }
                if !same {
                    continue;
                }
                windows += 1;
                sum += f64::from(c);
                sumsq += f64::from(c) * f64::from(c);
                clusters3 += u32::from(c3 >= 3);
            }
        }
        let mean = sum / f64::from(windows.max(1));
        let var = sumsq / f64::from(windows.max(1)) - mean * mean;
        // The binomial null `tests/scatter.rs` and `terrain_stats` use:
        // 25 cells, each an independent draw at the window's own rate.
        let null = mean * (1.0 - mean / 25.0);
        (windows, mean, if null > 0.0 { var / null } else { 0.0 }, clusters3)
    };
    // For a count drawn cell by cell against a shared random rate,
    // var = null + mean² × CV²(rate), so CV² = (dispersion − 1) × null / mean².
    let (_, tmean, tdisp, _) = window_stats(2, Occupant::Tree);
    let tnull = tmean * (1.0 - tmean / 25.0);
    let cv2 = if tmean > 0.0 { (tdisp - 1.0) * tnull / (tmean * tmean) } else { 0.0 };
    println!(
        "grove field, off the forest's trees: mean {tmean:.3} per 40 m window, dispersion {tdisp:.3} (binomial null 1.000), CV² {cv2:.3}"
    );
    println!("rock count in all-one-biome 40 m windows: windows, mean, dispersion (shared-field prediction), 3×3 clusters (≥3 rocks):");
    for b in 1..4u8 {
        let (windows, mean, disp, clusters3) = window_stats(b, Occupant::Rock);
        let null = mean * (1.0 - mean / 25.0);
        println!(
            "  {:<8} {windows:>6} windows, mean {mean:.3}, dispersion {disp:.3} (predicted {:.3}), clusters {clusters3}",
            BIOME_NAMES[b as usize],
            if null > 0.0 { 1.0 + mean * mean * cv2 / null } else { 0.0 },
        );
    }
}
