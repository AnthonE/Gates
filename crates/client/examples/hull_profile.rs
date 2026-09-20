//! Dev probe: the far hull's silhouette against the tree it stands in for,
//! band by band.
//!
//! Not a gate — `tests/tree.rs` holds the arithmetic and `CLAUDE.md` forbids a
//! pixel gate. This exists because `the_impostor_stands_inside_the_tree_it_
//! replaces` compares two SCALARS (the widest point of each) and the defect a
//! person sees is a PROFILE: a hull can pass a max-radius ratio while being a
//! capsule where the tree is a ragged cone, because one number cannot tell a
//! narrow shape from a short one.
//!
//! It prints, per variant, a radius per [`IMPOSTOR_BANDS`] height band and
//! the SILHOUETTE AREA that profile sweeps (`∫ 2r dy`) — the quantity the eye
//! integrates at the swap distance, which a max-radius ratio cannot see.
//!
//! **Two profiles of the tree, and the first draft of this file printed only
//! the wrong one.** Reading the tree at `IMPOSTOR_GIRTH_Q` compares the hull
//! against the number the hull is *solving for*, so the two agree by
//! construction — 0.85–0.96 per band, 89 % of the area, on a pair a person
//! reads as two different plants. That is this repo's naive-rebuild trap
//! arriving in an instrument instead of a gate.
//!
//! So it prints both:
//!
//!   · **quantile** — the radius `IMPOSTOR_GIRTH_Q` of the band's area lies
//!     inside. What `impostor_of` targets, and the right check that it hit it.
//!   · **extent** — the band's WIDEST triangle centre. What the eye sees,
//!     because a conifer's outline is the reach of its branches and the cards
//!     out there are alpha-masked air rather than absent.
//!
//! The gap between those two columns IS the defect: the tree is a wide
//! transparent cloud and the hull is a narrow opaque solid.
//!
//! Usage: `cargo run -p client --features render --example hull_profile`

// Host-side probe: printing is its job, not the sim's.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]
#![cfg(feature = "render")]

use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;
use client::render::tree::{
    bounds, conifer, impostor_of, species_of, CONIFER_POOL, IMPOSTOR_BANDS, IMPOSTOR_GIRTH_Q,
    SPECIES,
};

/// Every triangle of a mesh as (centre, area), the same sample
/// `impostor_of` takes.
fn tris_of(m: &Mesh) -> Vec<(Vec3, f32)> {
    let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return Vec::new();
    };
    let pos: Vec<Vec3> = p.iter().map(|v| Vec3::from(*v)).collect();
    let idx: Vec<usize> = match m.indices() {
        Some(i) => i.iter().collect(),
        None => (0..pos.len()).collect(),
    };
    idx.chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
            let area = 0.5 * (b - a).cross(c - a).length();
            ((a + b + c) / 3.0, area)
        })
        .collect()
}

/// The per-band radius of a mesh, two ways.
///
/// `q` of 1.0 is the band's widest triangle centre — its EXTENT, what the
/// outline is. Anything under 1.0 is `impostor_of`'s own rule: the radius that
/// `q` of the band's triangle area lies inside.
fn profile(meshes: &[&Mesh], h: f32, q: f32) -> [f32; IMPOSTOR_BANDS] {
    let step = h / IMPOSTOR_BANDS as f32;
    let mut girth: Vec<Vec<(f32, f32)>> = vec![Vec::new(); IMPOSTOR_BANDS];
    for m in meshes {
        for (mid, area) in tris_of(m) {
            let bin = ((mid.y.max(0.0) / step) as usize).min(IMPOSTOR_BANDS - 1);
            girth[bin].push(((mid.x * mid.x + mid.z * mid.z).sqrt(), area));
        }
    }
    let mut out = [0.0f32; IMPOSTOR_BANDS];
    for (bin, rows) in girth.iter_mut().enumerate() {
        if rows.is_empty() {
            continue;
        }
        rows.sort_by(|x, y| x.0.total_cmp(&y.0));
        out[bin] = rows.last().map_or(0.0, |r| r.0);
        if q >= 1.0 {
            continue;
        }
        let want: f32 = rows.iter().map(|r| r.1).sum::<f32>() * q;
        let mut acc = 0.0;
        for (r, a) in rows.iter() {
            acc += a;
            if acc >= want {
                out[bin] = *r;
                break;
            }
        }
    }
    out
}

/// `∫ 2r dy` over the bands — the side-on area of the solid of revolution
/// each profile describes, in m².
fn silhouette(p: &[f32; IMPOSTOR_BANDS], h: f32) -> f32 {
    let step = h / IMPOSTOR_BANDS as f32;
    p.iter().map(|r| 2.0 * r * step).sum()
}

fn main() {
    println!(
        "IMPOSTOR_GIRTH_Q {IMPOSTOR_GIRTH_Q}  IMPOSTOR_BANDS {IMPOSTOR_BANDS}  \
         variants {CONIFER_POOL}"
    );
    let mut worst = f32::INFINITY;
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let far = impostor_of(&bark, &needles, v);
        let (th, tr) = bounds(&[&bark, &needles]);
        let (_, fr) = bounds(&[&far]);
        let sp = &SPECIES[species_of(v)];

        let tq = profile(&[&bark, &needles], th, IMPOSTOR_GIRTH_Q);
        let tx = profile(&[&bark, &needles], th, 1.0);
        let fp = profile(&[&far], th, 1.0);
        let (qa, xa, fa) = (
            silhouette(&tq, th),
            silhouette(&tx, th),
            silhouette(&fp, th),
        );
        worst = worst.min(fa / xa);

        println!(
            "\nvariant {v} ({}) — {th:.2} m tall, tree max r {tr:.3}, hull max r {fr:.3} \
             ({:.2}x), species ceiling {:.2}",
            if species_of(v) == 0 { "conifer" } else { "broadleaf" },
            fr / tr,
            sp.max_r_m
        );
        print!("  band   ");
        for b in 0..IMPOSTOR_BANDS {
            print!("{b:>7}");
        }
        print!("\n  quantile");
        for r in tq.iter() {
            print!("{r:>7.3}");
        }
        print!("\n  extent  ");
        for r in tx.iter() {
            print!("{r:>7.3}");
        }
        print!("\n  hull    ");
        for r in fp.iter() {
            print!("{r:>7.3}");
        }
        print!("\n  vs quant");
        for b in 0..IMPOSTOR_BANDS {
            if tq[b] > 1e-4 {
                print!("{:>7.2}", fp[b] / tq[b]);
            } else {
                print!("{:>7}", "-");
            }
        }
        print!("\n  vs ext  ");
        for b in 0..IMPOSTOR_BANDS {
            if tx[b] > 1e-4 {
                print!("{:>7.2}", fp[b] / tx[b]);
            } else {
                print!("{:>7}", "-");
            }
        }
        println!(
            "\n  silhouette m²: quantile {qa:.2}  extent {xa:.2}  hull {fa:.2}  \
             — hull is {:.0}% of what the hull targets, {:.0}% of what the eye sees",
            100.0 * fa / qa,
            100.0 * fa / xa
        );
    }
    println!(
        "\nworst hull-vs-EXTENT ratio over {CONIFER_POOL} variants: {:.1}%",
        100.0 * worst
    );

    // ── Is the trimmed 10% really sub-pixel? ────────────────────────────
    //
    // [`IMPOSTOR_GIRTH_Q`]'s doc rests on it — "the 10% outside it is branch
    // tips that are sub-pixel at the swap distance" — and that sentence was
    // written when a conifer was 7 m tall. Forest scale v0 doubled it and the
    // sentence was not re-derived, so this prints the arithmetic instead of
    // trusting it: a radial gap Δr at distance d subtends Δr/d radians, and a
    // 720-row frame over a 60° vertical FOV is 720/1.047 px per radian.
    const SWAP_M: f32 = 55.0; // quality.rs MEDIUM_TREE_LOD_SWAP_M
    const PX_PER_RAD: f32 = 720.0 / 1.047;
    println!("\ntrimmed band width at the {SWAP_M:.0} m swap, in pixels of a 720-row frame:");
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let far = impostor_of(&bark, &needles, v);
        let (th, _) = bounds(&[&bark, &needles]);
        let tx = profile(&[&bark, &needles], th, 1.0);
        let fp = profile(&[&far], th, 1.0);
        let px: Vec<String> = (0..IMPOSTOR_BANDS)
            .map(|b| format!("{:>6.1}", (tx[b] - fp[b]) / SWAP_M * PX_PER_RAD))
            .collect();
        println!("  variant {v}:{}", px.join(""));
    }

    // ── What quantile would close it ────────────────────────────────────
    //
    // The profile rule is `impostor_of`'s own, so this is the TARGET a given
    // q sets, not the hull it yields — the lathe loses a further ~10% to ring
    // smoothing and the tip closing to a point, which the table above
    // measures. Printed so the knob is read off a sweep rather than picked.
    println!("\ntarget silhouette as a share of EXTENT, per quantile:");
    print!("  variant ");
    for q in [0.90f32, 0.93, 0.95, 0.97, 0.99, 1.0] {
        print!("{q:>8.2}");
    }
    println!();
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let (th, _) = bounds(&[&bark, &needles]);
        let xa = silhouette(&profile(&[&bark, &needles], th, 1.0), th);
        print!("  {v:>7} ");
        for q in [0.90f32, 0.93, 0.95, 0.97, 0.99, 1.0] {
            let a = silhouette(&profile(&[&bark, &needles], th, q), th);
            print!("{:>8.2}", a / xa);
        }
        println!();
    }
}
