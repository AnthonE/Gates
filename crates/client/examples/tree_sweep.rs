//! `tree_sweep` — a bench, not a gate: generate candidate tree settings over
//! many seeds and print what the gates would measure, so a parameter block is
//! chosen against a DISTRIBUTION and not against the one seed it was tuned on.
//!
//! ```text
//! cargo run --release -p client --features render --example tree_sweep
//! ```
//!
//! `tree.rs`'s own comments record that every width number in it was "swept
//! against the real seeds" — 1.75 → 1.717 m OVER the ceiling, 1.65 → 1.632 —
//! and that sweep was done by hand, in the test binary, one edit at a time.
//! This is that sweep as a tool: each candidate block is fitted to its target
//! height exactly as `tree::conifer` fits it, then measured with the same
//! functions the gates use (`bounds`, `trunk_radius`, `tris`), over the three
//! shipped seeds AND a dozen more, and the worst case per column is what
//! matters. Nothing here asserts; the numbers go into `tree.rs` by hand and
//! `tests/tree.rs` holds them.
//!
//! The four columns the gates care about, and which constant each answers to:
//!   h      — species height (`SpeciesDef::height_m`, an equality)
//!   r      — crown radius   (`SpeciesDef::max_r_m` ≤ `TREE_MAX_R`, a ceiling)
//!   trunk  — bark radius under 0.25 m (`OCCUPANT_R_M[Tree]`, held to 1 mm)
//!   tris   — bark + needles (`CONIFER_MAX_TRIS`, a ceiling)

use bevy::prelude::*;
use bevy_procedural_tree::enums::{LeafBillboard, TreeType};
use bevy_procedural_tree::meshgen::generate_tree_meshes;
use bevy_procedural_tree::settings::{
    BranchForce, BranchParams, BranchRecursionLevel, LeafParams, TreeMeshSettings,
};
use client::render::tree::{bounds, tris, trunk_radius};

/// A candidate block and the height it is fitted to.
struct Candidate {
    name: &'static str,
    height_m: f32,
    settings: TreeMeshSettings,
    /// Which of the six shipped variant seeds are this species' own —
    /// `tree::species_of` groups them by species, three each.
    ship: std::ops::Range<usize>,
}

/// `props::hash2`'s mixer, restated: the shipped pool seeds off
/// `hash2(0x9e37_79b9, variant)`, and this bench wants those six seeds in its
/// table beside the extra ones, so it has to produce the same numbers.
fn hash2(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x85eb_ca6b) ^ b.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    h
}

fn fit(bark: &mut Mesh, needles: &mut Mesh, height_m: f32) {
    let (h, _) = bounds(&[bark, needles]);
    if h <= f32::EPSILON {
        return;
    }
    let k = height_m / h;
    for m in [bark, needles] {
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
            m.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        {
            for v in p.iter_mut() {
                v[0] *= k;
                v[1] *= k;
                v[2] *= k;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn conifer(
    name: &'static str,
    height_m: f32,
    angle: f32,
    children: u8,
    limb: f32,
    trunk_r: f32,
    start: f32,
    count: u32,
    size: f32,
) -> Candidate {
    Candidate {
        name,
        height_m,
        ship: 0..3,
        settings: TreeMeshSettings {
            tree_type: TreeType::Evergreen,
            branch: BranchParams {
                levels: BranchRecursionLevel::One,
                angle: [0.0, angle, 0.0, 0.0],
                children: [children, 0, 0],
                force: BranchForce {
                    direction: Vec3::Y,
                    strength: 0.05,
                    radius_cutoff: 0.1,
                },
                gnarliness: [0.01, 0.06, 0.0, 0.0],
                length: [height_m, limb, 0.0, 0.0],
                trunk_base_radius: trunk_r,
                radius_factor: [1.0, 0.13, 0.0, 0.0],
                sections: [10, 4, 0, 0],
                segments: [7, 4, 0, 0],
                start: [0.0, start, 0.0, 0.0],
                taper: [0.92, 0.90, 0.0, 0.0],
                twist: [0.02, 0.0, 0.0, 0.0],
            },
            leaves: LeafParams {
                leaf_billboard: LeafBillboard::Double,
                angle: 62.0,
                count,
                start: 0.0,
                size,
                size_variance: 0.4,
            },
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn broadleaf(
    name: &'static str,
    height_m: f32,
    angle1: f32,
    angle2: f32,
    limb1: f32,
    limb2: f32,
    trunk_r: f32,
    count: u32,
    size: f32,
) -> Candidate {
    Candidate {
        name,
        height_m,
        ship: 3..6,
        settings: TreeMeshSettings {
            tree_type: TreeType::Deciduous,
            branch: BranchParams {
                levels: BranchRecursionLevel::Two,
                angle: [0.0, angle1, angle2, 0.0],
                children: [6, 7, 0],
                force: BranchForce {
                    direction: Vec3::Y,
                    strength: 0.04,
                    radius_cutoff: 0.1,
                },
                gnarliness: [-0.04, 0.14, 0.10, 0.0],
                length: [4.5, limb1, limb2, 0.0],
                trunk_base_radius: trunk_r,
                radius_factor: [1.0, 0.42, 0.34, 0.0],
                sections: [10, 5, 3, 0],
                segments: [7, 5, 3, 0],
                start: [0.0, 0.22, 0.3, 0.0],
                taper: [0.94, 0.82, 0.85, 0.0],
                twist: [0.06, -0.05, 0.0, 0.0],
            },
            leaves: LeafParams {
                leaf_billboard: LeafBillboard::Double,
                angle: 48.0,
                count,
                start: 0.0,
                size,
                size_variance: 0.35,
            },
        },
    }
}

fn main() {
    let candidates = vec![
        // Controls: what ships today.
        conifer("pine-shipped", 6.6, 96.0, 60, 1.05, 0.20, 0.10, 16, 0.55),
        broadleaf("leaf-shipped", 5.4, 52.0, 44.0, 1.75, 1.0, 0.22, 11, 0.42),
        // The conifer pass 3 settled on: 14 m, limb 1.8, trunk 0.2540 puts
        // variant 0 at 0.2390 m, inside the sim cylinder's 1 mm window.
        conifer("pine14-e3", 14.0, 104.0, 72, 1.8, 0.2540, 0.20, 12, 1.15),
        // Broadleaf, pass 4: pass 3 read ~3.0–3.2 m at every angle, so the
        // cards come down too — their size adds to the crown radius directly.
        broadleaf("leaf11-n", 11.0, 36.0, 32.0, 0.38, 0.20, 0.11, 11, 0.60),
        broadleaf("leaf11-o", 11.0, 38.0, 34.0, 0.40, 0.22, 0.11, 12, 0.60),
        broadleaf("leaf11-p", 11.0, 34.0, 30.0, 0.40, 0.22, 0.11, 11, 0.65),
        broadleaf("leaf10-k", 10.0, 38.0, 34.0, 0.42, 0.23, 0.11, 11, 0.70),
        broadleaf("leaf10-l", 10.0, 36.0, 32.0, 0.40, 0.22, 0.11, 12, 0.70),
        broadleaf("leaf10-m", 10.0, 40.0, 36.0, 0.40, 0.22, 0.11, 11, 0.65),
    ];
    // The six shipped variant seeds (three per species), then a dozen more.
    let shipped: Vec<u64> = (0..6u32).map(|v| hash2(0x9e37_79b9, v) as u64).collect();
    let extra: Vec<u64> = (100..112u64).collect();

    println!(
        "{:<14} {:>6} {:>6} {:>6} {:>7} {:>7} {:>6}   (ship = the species' own shipped seeds; all = {} seeds)",
        "candidate",
        "h",
        "r_ship",
        "r_all",
        "trk_shp",
        "trk_all",
        "tris",
        shipped.len() + extra.len()
    );
    for c in &candidates {
        let (mut r_all, mut r_ship, mut trunk_ship, mut trunk_all, mut tris_max) =
            (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0usize);
        let mut h_out = 0.0f32;
        let mut rows = Vec::new();
        for (i, seed) in shipped.iter().chain(extra.iter()).enumerate() {
            let mut rng = fastrand::Rng::with_seed(*seed);
            let Ok((mut bark, mut needles)) = generate_tree_meshes(&c.settings, &mut rng) else {
                println!("{}: generator failed on seed {seed}", c.name);
                continue;
            };
            fit(&mut bark, &mut needles, c.height_m);
            let (h, r) = bounds(&[&bark, &needles]);
            let t = trunk_radius(&bark);
            let n = tris(&bark) + tris(&needles);
            h_out = h;
            r_all = r_all.max(r);
            trunk_all = trunk_all.max(t);
            tris_max = tris_max.max(n);
            if c.ship.contains(&i) {
                r_ship = r_ship.max(r);
                trunk_ship = trunk_ship.max(t);
                rows.push(format!("    variant {i}: r {r:.3} trunk {t:.4} tris {n}"));
            }
        }
        println!(
            "{:<14} {:>6.2} {:>6.3} {:>6.3} {:>7.4} {:>7.4} {:>6}",
            c.name, h_out, r_ship, r_all, trunk_ship, trunk_all, tris_max
        );
        for row in rows {
            println!("{row}");
        }
    }
    println!(
        "\nceilings: TREE_MAX_R {:.3}  OCCUPANT_R_M[Tree] {:.4}  CONIFER_MAX_TRIS {}",
        client::render::tree::TREE_MAX_R,
        sim_core::terrain::OCCUPANT_R_M[sim_core::terrain::Occupant::Tree as usize],
        client::render::tree::CONIFER_MAX_TRIS
    );
}
