//! `tree_sweep` — a bench, not a gate: grow candidate tree settings over many
//! seeds and print what the gates would measure, so a parameter block is
//! chosen against a DISTRIBUTION and not against the one seed it was tuned on.
//!
//! ```text
//! cargo run --release -p client --features render --example tree_sweep [-- <candidate> [outdir]]
//! ```
//!
//! Each candidate is an edit of the block that ships (`tree::settings`), grown
//! through `tree::grow` — the same fit, crown shaping and shading `tree::conifer`
//! applies — and measured with the functions the gates use (`bounds`,
//! `trunk_radius`, `tris`) over the species' three shipped seeds AND a dozen
//! more; the worst case per column is what matters. Nothing here asserts; the
//! numbers go into `tree.rs` by hand and `tests/tree.rs` holds them. Name a
//! candidate to print its seeds' crown profiles, and add a directory to also
//! write a side-on silhouette of each seed (`<candidate>.ppm`).
//!
//! The columns, and which constant each answers to:
//!   h      — species height (`SpeciesDef::height_m`, an equality)
//!   r      — crown radius   (`SpeciesDef::max_r_m` ≤ `TREE_MAX_R`, a ceiling)
//!   trunk  — bark radius under 0.25 m (`OCCUPANT_R_M[Tree]`, held to 1 mm)
//!   tris   — bark + needles (`CONIFER_MAX_TRIS`, a ceiling)
//!   neck   — the crown's worst waist: a band's silhouette radius against the
//!            narrower of the widest bands above and below it. 1 is a crown
//!            with no waist; ~0.5 is two balls stacked on a stick. Worst seed.
//!   thin   — the crown's emptiest band, as a share of its mean card count.
//!   upper  — the upper half of the crown's cards against the lower half's:
//!            ~0.2 is a full skirt under a sparse top, the stacked-pom-pom read.

use std::path::PathBuf;

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy_procedural_tree::enums::TreeType;
use bevy_procedural_tree::settings::TreeMeshSettings;
use client::render::tree::{bounds, grow, leaf_image, settings, tris, trunk_radius};

/// A candidate block: the species it stands in for, and its settings.
struct Candidate {
    name: &'static str,
    species: usize,
    settings: TreeMeshSettings,
}

/// `species`' shipped block, edited by `f`.
fn edit(name: &'static str, species: usize, f: impl FnOnce(&mut TreeMeshSettings)) -> Candidate {
    let mut s = settings(species);
    f(&mut s);
    Candidate {
        name,
        species,
        settings: s,
    }
}

fn positions(m: &Mesh) -> &[[f32; 3]] {
    match m.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(p)) => p,
        _ => &[],
    }
}

/// The median canopy card's edge, metres — what a millimetre on the card is
/// measured against (`tests/tree.rs` `card_edge_m`).
fn card_edge(needles: &Mesh) -> f32 {
    let mut e: Vec<f32> = positions(needles)
        .chunks_exact(4)
        .map(|q| {
            let a = Vec3::from_array(q[0]);
            (Vec3::from_array(q[1]) - a)
                .length()
                .max((Vec3::from_array(q[3]) - a).length())
        })
        .collect();
    e.sort_by(f32::total_cmp);
    e.get(e.len() / 2).copied().unwrap_or(0.0)
}

/// Height bands a crown profile is cut into.
const BANDS: usize = 16;

/// A crown's profile, bottom band first: each band's silhouette radius (the
/// 85th-percentile vertex radius, so one stray card is not the outline) and
/// how many cards are centred in it.
fn profile(needles: &Mesh) -> (Vec<f32>, Vec<usize>) {
    let p = positions(needles);
    let (lo, hi) = p
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v[1]), h.max(v[1])));
    let span = (hi - lo).max(1e-3);
    let band = |y: f32| (((y - lo) / span * BANDS as f32) as usize).min(BANDS - 1);
    let mut radii = vec![Vec::new(); BANDS];
    for v in p {
        radii[band(v[1])].push((v[0] * v[0] + v[2] * v[2]).sqrt());
    }
    let r = radii
        .into_iter()
        .map(|mut b| {
            if b.is_empty() {
                return 0.0;
            }
            b.sort_by(f32::total_cmp);
            b[(b.len() * 85 / 100).min(b.len() - 1)]
        })
        .collect();
    let mut n = vec![0usize; BANDS];
    for q in p.chunks_exact(4) {
        n[band((q[0][1] + q[1][1] + q[2][1] + q[3][1]) * 0.25)] += 1;
    }
    (r, n)
}

/// `(neck, thin, upper)` over the crown's interior bands — see the header.
/// The lowest two bands and the top one are left out: a crown is meant to
/// narrow at its foot and its tip.
fn waist(needles: &Mesh) -> (f32, f32, f32) {
    let (r, n) = profile(needles);
    let inner = 2..BANDS - 1;
    let mut neck = f32::MAX;
    for i in inner.clone() {
        let below = r[..i].iter().copied().fold(0.0f32, f32::max);
        let above = r[i + 1..].iter().copied().fold(0.0f32, f32::max);
        let side = below.min(above);
        if side > 1e-3 {
            neck = neck.min(r[i] / side);
        }
    }
    let mean = inner.clone().map(|i| n[i]).sum::<usize>() as f32 / inner.len() as f32;
    let thin = inner.map(|i| n[i] as f32).fold(f32::MAX, f32::min) / mean.max(1e-3);
    let lower: usize = n[..BANDS / 2].iter().sum();
    let upper: usize = n[BANDS / 2..].iter().sum();
    (neck.min(1.0), thin, upper as f32 / lower.max(1) as f32)
}

fn print_profile(label: &str, needles: &Mesh) {
    let (r, n) = profile(needles);
    let (neck, thin, upper) = waist(needles);
    println!("  {label}: neck {neck:.2} thin {thin:.2} upper {upper:.2}");
    for i in (0..BANDS).rev() {
        let bar = "#".repeat((r[i] * 10.0).round() as usize);
        println!("    {i:>2} r {:>5.2} cards {:>4} {bar}", r[i], n[i]);
    }
}

/// Pixels per metre of the side-on silhouette.
const SIDE_PX_M: f32 = 32.0;
/// One silhouette cell, metres: ±3.2 m wide, 12 m tall.
const SIDE_W_M: f32 = 6.4;
const SIDE_H_M: f32 = 12.0;

/// One tree side-on (looking down -Z), orthographic, nearest-sampled through
/// the leaf card's level-0 alpha and lit only by its vertex colour: the crown's
/// silhouette and its holes as an 18 m eye sees them, without a GPU.
fn side_view(bark: &Mesh, needles: &Mesh, card: &Image) -> (usize, usize, Vec<[u8; 3]>) {
    let (w, h) = (
        (SIDE_W_M * SIDE_PX_M) as usize,
        (SIDE_H_M * SIDE_PX_M) as usize,
    );
    let mut depth = vec![f32::NEG_INFINITY; w * h];
    let mut px = vec![[200u8, 214, 232]; w * h];
    let side = card.texture_descriptor.size.width as usize;
    let tex = card.data.as_deref().unwrap_or_default();
    for (m, leaf) in [(bark, false), (needles, true)] {
        let p = positions(m);
        let uv: &[[f32; 2]] = match m.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(VertexAttributeValues::Float32x2(v)) => v,
            _ => &[],
        };
        let col: &[[f32; 4]] = match m.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(v)) => v,
            _ => &[],
        };
        let idx: Vec<usize> = match m.indices() {
            Some(Indices::U32(i)) => i.iter().map(|&k| k as usize).collect(),
            Some(Indices::U16(i)) => i.iter().map(|&k| k as usize).collect(),
            None => (0..p.len()).collect(),
        };
        for t in idx.chunks_exact(3) {
            let s = |k: usize| {
                let v = p[t[k]];
                Vec3::new(
                    (v[0] + SIDE_W_M * 0.5) * SIDE_PX_M,
                    (SIDE_H_M - v[1]) * SIDE_PX_M,
                    v[2],
                )
            };
            let (a, b, c) = (s(0), s(1), s(2));
            let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
            let x1 = (a.x.max(b.x).max(c.x).ceil().max(0.0) as usize).min(w);
            let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
            let y1 = (a.y.max(b.y).max(c.y).ceil().max(0.0) as usize).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b.x - fx) * (c.y - fy) - (b.y - fy) * (c.x - fx)) / area;
                    let w1 = ((c.x - fx) * (a.y - fy) - (c.y - fy) * (a.x - fx)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = a.z * w0 + b.z * w1 + c.z * w2;
                    let i = y * w + x;
                    if z <= depth[i] {
                        continue;
                    }
                    let lerp4 = |q: &[[f32; 4]]| -> [f32; 3] {
                        if q.is_empty() {
                            return [1.0; 3];
                        }
                        let (qa, qb, qc) = (q[t[0]], q[t[1]], q[t[2]]);
                        [0, 1, 2].map(|k| qa[k] * w0 + qb[k] * w1 + qc[k] * w2)
                    };
                    let vc = lerp4(col);
                    let rgb = if leaf {
                        if uv.is_empty() || side == 0 {
                            continue;
                        }
                        let u = uv[t[0]][0] * w0 + uv[t[1]][0] * w1 + uv[t[2]][0] * w2;
                        let v = uv[t[0]][1] * w0 + uv[t[1]][1] * w1 + uv[t[2]][1] * w2;
                        let tx = ((u.clamp(0.0, 1.0) * side as f32) as usize).min(side - 1);
                        let ty = ((v.clamp(0.0, 1.0) * side as f32) as usize).min(side - 1);
                        if tex[(ty * side + tx) * 4 + 3] < 128 {
                            continue;
                        }
                        // A colourless (raw generator) canopy draws mid-green.
                        let g = if col.is_empty() {
                            [0.10, 0.22, 0.06]
                        } else {
                            vc
                        };
                        g.map(|c| (c * 2.4).min(1.0))
                    } else {
                        [
                            0.45 * vc[0].min(1.0),
                            0.42 * vc[1].min(1.0),
                            0.38 * vc[2].min(1.0),
                        ]
                    };
                    depth[i] = z;
                    px[i] = rgb.map(|c| (c.powf(1.0 / 2.2) * 255.0) as u8);
                }
            }
        }
    }
    (w, h, px)
}

fn main() {
    let show = std::env::args().nth(1);
    let out_dir = std::env::args().nth(2).map(PathBuf::from);
    let candidates = vec![
        // Controls: what ships today.
        edit("pine-shipped", 0, |_| {}),
        edit("leaf-shipped", 1, |_| {}),
        // The broadleaf as the crate's `Deciduous` type, the block that
        // shipped until 2026-10-05: straight off the generator its upper
        // crown held a fifth of the lower's cards (the stacked pom-poms), and
        // `shape_broadleaf`'s evening only brings that to ~0.4.
        edit("leaf-deciduous", 1, |s| {
            s.tree_type = TreeType::Deciduous;
            s.branch.children = [6, 7, 0];
            s.branch.length = [4.5, 0.38, 0.20, 0.0];
            s.branch.sections = [10, 5, 3, 0];
            s.branch.segments = [7, 5, 3, 0];
            s.branch.start = [0.0, 0.22, 0.3, 0.0];
            s.branch.taper[0] = 0.94;
            s.leaves.count = 11;
        }),
    ];

    // Each species' three shipped variants, then a dozen more seeds of the
    // same mixer.
    let extra: Vec<usize> = (100..112).collect();
    let card = leaf_image();

    println!(
        "{:<14} {:>6} {:>6} {:>6} {:>7} {:>7} {:>6} {:>5} {:>5} {:>5} {:>5}   (ship = the species' own shipped seeds; all = {} seeds)",
        "candidate",
        "h",
        "r_ship",
        "r_all",
        "trk_shp",
        "trk_all",
        "tris",
        "n_shp",
        "n_all",
        "thin",
        "upper",
        3 + extra.len()
    );
    for c in &candidates {
        let ship: Vec<usize> = (c.species * 3..c.species * 3 + 3).collect();
        let (mut r_all, mut r_ship, mut trunk_ship, mut trunk_all, mut tris_max) =
            (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0usize);
        let (mut neck_ship, mut neck_all, mut thin_all, mut upper_all) =
            (1.0f32, 1.0f32, f32::MAX, f32::MAX);
        let mut h_out = 0.0f32;
        let mut rows = Vec::new();
        let mut sides = Vec::new();
        let showing = show.as_deref() == Some(c.name) || show.as_deref() == Some("all");
        for &v in ship.iter().chain(extra.iter()) {
            let (bark, needles) = grow(c.species, v, &c.settings);
            let (h, r) = bounds(&[&bark, &needles]);
            let t = trunk_radius(&bark);
            let n = tris(&bark) + tris(&needles);
            let (neck, thin, upper) = waist(&needles);
            h_out = h;
            r_all = r_all.max(r);
            trunk_all = trunk_all.max(t);
            tris_max = tris_max.max(n);
            neck_all = neck_all.min(neck);
            thin_all = thin_all.min(thin);
            upper_all = upper_all.min(upper);
            let shipped = ship.contains(&v);
            if shipped {
                r_ship = r_ship.max(r);
                trunk_ship = trunk_ship.max(t);
                neck_ship = neck_ship.min(neck);
                rows.push(format!(
                    "    variant {v}: r {r:.3} trunk {t:.4} tris {n} neck {neck:.2} thin {thin:.2} upper {upper:.2} card {:.2} m", card_edge(&needles)
                ));
                if showing {
                    print_profile(&format!("{} variant {v}", c.name), &needles);
                }
            }
            if showing && out_dir.is_some() && sides.len() < 6 {
                sides.push(side_view(&bark, &needles, &card));
            }
        }
        println!(
            "{:<14} {:>6.2} {:>6.3} {:>6.3} {:>7.4} {:>7.4} {:>6} {:>5.2} {:>5.2} {:>5.2} {:>5.2}",
            c.name,
            h_out,
            r_ship,
            r_all,
            trunk_ship,
            trunk_all,
            tris_max,
            neck_ship,
            neck_all,
            thin_all,
            upper_all
        );
        for row in rows {
            println!("{row}");
        }
        if let (Some(dir), Some((w, h, _))) = (out_dir.as_ref(), sides.first()) {
            let (w, h) = (*w, *h);
            let total = w * sides.len();
            let mut ppm = format!("P6\n{total} {h}\n255\n").into_bytes();
            for y in 0..h {
                for (_, _, px) in &sides {
                    for p in &px[y * w..(y + 1) * w] {
                        ppm.extend_from_slice(p);
                    }
                }
            }
            let path = dir.join(format!("{}.ppm", c.name));
            if let Err(e) = std::fs::write(&path, ppm) {
                eprintln!("tree_sweep: {}: {e}", path.display());
            }
        }
    }
    // The crown-profile gate's statistic (`tests/tree.rs`): the shipped
    // apex's shade against mid-crown's, held at ≥ 1.02.
    print!("\nshipped apex / mid-crown shade:");
    for v in 0..6 {
        let apex = client::render::tree::canopy_shade(v, 0.85, 1.0);
        let mid = client::render::tree::canopy_shade(v, 0.40, 0.60);
        print!("  v{v} {:.3} ({apex:.3})", apex / mid);
    }
    println!();
    println!(
        "\nceilings: TREE_MAX_R {:.3}  OCCUPANT_R_M[Tree] {:.4}  CONIFER_MAX_TRIS {}",
        client::render::tree::TREE_MAX_R,
        sim_core::terrain::OCCUPANT_R_M[sim_core::terrain::Occupant::Tree as usize],
        client::render::tree::CONIFER_MAX_TRIS
    );
}
