//! Gate: what a built piece is made of — the tier table, the mesh a part
//! fills its extents with, and the footing's quantised depth.
//!
//! **Written because all three of these failed silently at once, for six
//! days, with every gate in the repo green** (2026-08-16).
//!
//! §A is the one that shipped. `structures::TIER` had three rows against the
//! sim's four materials and `spawn_piece` indexed it `.min(2)`, so every
//! piece in the game drew one rung off: wood in the stone grey, stone as a
//! `metallic: 0.85` conductor, and twig — the state EVERY piece is first seen
//! in since twig v0 — with no row of its own. Nothing caught it because a
//! colour table has no gate and a clamp is not a compile error. The table is
//! `[_; N_TIERS]` now, so a fifth material is a build failure; what this adds
//! is the half a length cannot check, that each material resolves to its own
//! distinct row naming a file that exists.
//!
//! §B is `ART.md` rule 1 on the largest flat surface in the game. A piece
//! mesh carried 0..1 UVs, no vertex colours and no tangents, and each of
//! those three is a silent failure with a different signature: a stretched
//! tile reads as no texture, a missing colour reads as a flat wall, and a
//! missing tangent makes Bevy ignore `normal_map_texture` outright without
//! saying so.
//!
//! §C is the arithmetic behind the footing cache. A quantised depth is what
//! lets a per-address skirt wear a per-metre texture at all, and the two
//! things it must never do are round DOWN (which lifts the skirt out of the
//! ground `tests/ghost.rs` requires it to be buried in) and run off the end
//! of the mesh array.
//!
//! Headless: a table is data, a mesh is countable, and quantisation is
//! arithmetic. No GPU, no window, no asset server — `tests/tree.rs`'s posture.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::structures::{
    damage_mix, footing_of, foundation_part, part_mesh, skirt_step, tier, Part, PartKind,
    DMG_DARKEST, FACE_BOTTOM, N_TIERS, PIECE_TILES_PER_M, SIDE_FOOT, SKIRT_MAX_M, SKIRT_STEPS,
    SKIRT_STEP_M, SLAB_T,
};
use sim_core::build::DMG_BANDS;
use sim_core::build::{MAT_METAL, MAT_STONE, MAT_TWIG, MAT_WOOD};

/// Assets live beside the crate, not inside it — the hop `tests/ui.rs` and
/// `tests/deploy_assets.rs` both make.
fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

// ---------------------------------------------------------------------------
// §A · The tier table covers the sim's materials, one row each
// ---------------------------------------------------------------------------

/// Every material the sim can put on the wire resolves to its own row.
///
/// The assertion is DISTINCTNESS, not a list of expected names: naming the
/// four roles here would be a second copy of the table, and a test that
/// restates its subject cannot catch the subject being wrong. What it can
/// catch is two materials sharing a row, which is precisely what `.min(2)`
/// did to stone and metal.
#[test]
fn every_material_has_its_own_tier() {
    assert_eq!(
        N_TIERS,
        MAT_METAL as usize + 1,
        "the tier table must span MAT_TWIG..=MAT_METAL"
    );
    let all = [MAT_TWIG, MAT_WOOD, MAT_STONE, MAT_METAL];
    for (i, &a) in all.iter().enumerate() {
        for &b in &all[i + 1..] {
            assert_ne!(
                tier(a).0,
                tier(b).0,
                "materials {a} and {b} share the texture role {:?} — the \
                 `.min(2)` clamp is back",
                tier(a).0
            );
        }
    }
}

/// Each row names a texture set that is actually on disk, in all three maps.
///
/// A typo'd role is not a crash and not a warning: Bevy fails the load, the
/// material keeps its `base_color`, and the wall renders as flat grey — which
/// is exactly the state this whole slice exists to leave behind, arrived at
/// from the other direction.
#[test]
fn every_tier_names_maps_that_exist() {
    for m in 0..N_TIERS as u8 {
        let (role, gain, rough, metallic) = tier(m);
        for map in ["albedo", "normal", "rough"] {
            let p = asset_path(&format!("textures/{role}_{map}.jpg"));
            assert!(
                p.exists(),
                "material {m} wants {}, which does not exist",
                p.display()
            );
        }
        // The gain is a SCALAR multiply on a photograph — `ART.md` §7 allows
        // it precisely because a scalar's colour-deviation span is 1.000. A
        // gain that could darken to nothing or blow the map out is neither.
        assert!(
            (1.0..=2.0).contains(&gain),
            "material {m} gain {gain} is outside the measured band"
        );
        assert!((0.0..=1.0).contains(&rough) && (0.0..=1.0).contains(&metallic));
    }
}

// ---------------------------------------------------------------------------
// §B · A piece mesh can wear a photograph
// ---------------------------------------------------------------------------

fn part(size: Vec3, kind: PartKind) -> Part {
    Part {
        size,
        offset: Vec3::ZERO,
        x_rot: 0.0,
        kind,
    }
}

/// Both part kinds carry the four attributes a textured, normal-mapped,
/// vertex-tinted surface needs. Missing any one of them is silent.
#[test]
fn a_part_mesh_carries_what_a_photograph_needs() {
    for kind in [PartKind::Box, PartKind::Tri] {
        let m = part_mesh(&part(Vec3::new(2.96, 3.0, 0.2), kind));
        for (name, present) in [
            ("position", m.attribute(Mesh::ATTRIBUTE_POSITION).is_some()),
            ("normal", m.attribute(Mesh::ATTRIBUTE_NORMAL).is_some()),
            ("uv0", m.attribute(Mesh::ATTRIBUTE_UV_0).is_some()),
            ("colour", m.attribute(Mesh::ATTRIBUTE_COLOR).is_some()),
            ("tangent", m.attribute(Mesh::ATTRIBUTE_TANGENT).is_some()),
        ] {
            assert!(
                present,
                "{kind:?} mesh has no {name} — a normal-mapped surface needs \
                 it and Bevy will not say so at runtime"
            );
        }
    }
}

/// Every triangle is wound counter-clockwise seen from outside — checked
/// against its own stored normal, which is the one thing that cannot be
/// wrong by the same mistake.
///
/// **This is the assertion that pays for hand-writing the builders.** The
/// old meshes came from `Cuboid`, where the winding was Bevy's problem; both
/// builders now walk a `(u, v)` pair per face and a pair swapped anywhere is
/// a face culled away — you look through the wall from one side and nothing
/// errors, no gate reddens, and it is only visible if a person happens to
/// stand on that side. A geometric normal against the stored one catches it
/// exactly, for every face, at no cost.
#[test]
fn every_face_is_wound_outward() {
    for kind in [PartKind::Box, PartKind::Tri] {
        // Deliberately not a cube: an equal-sided box hides an axis swap.
        let m = part_mesh(&part(Vec3::new(2.96, 3.0, 0.24), kind));
        let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("{kind:?} position is not float3");
        };
        let Some(VertexAttributeValues::Float32x3(nn)) = m.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("{kind:?} normal is not float3");
        };
        let idx: Vec<usize> = m.indices().expect("indexed").iter().collect();
        assert_eq!(idx.len() % 3, 0, "{kind:?} is not a triangle list");
        for t in idx.chunks(3) {
            let (a, b, c) = (
                Vec3::from_array(p[t[0]]),
                Vec3::from_array(p[t[1]]),
                Vec3::from_array(p[t[2]]),
            );
            let geo = (b - a).cross(c - a);
            let stored = Vec3::from_array(nn[t[0]]);
            assert!(
                geo.length() > 1e-9,
                "{kind:?} has a degenerate triangle — mikktspace would refuse it"
            );
            assert!(
                geo.normalize().dot(stored) > 0.99,
                "{kind:?} triangle {t:?} winds against its own normal {stored:?} \
                 — this face is culled and you see through the piece"
            );
        }
    }
}

/// The UV is in METRES, not per-face 0..1 — measured by the span the mesh's
/// own UVs cover against the size it was built at.
///
/// This is the assertion that would have caught the shipped defect: a unit
/// cube's UVs cover exactly 1.0 whatever its size, so a 3 m wall wearing one
/// tile passes every structural check and reads as untextured.
#[test]
fn a_part_mesh_tiles_by_the_metre() {
    for kind in [PartKind::Box, PartKind::Tri] {
        for size in [
            Vec3::new(2.96, 3.0, 0.2),
            Vec3::new(2.96, 0.3, 2.96),
            Vec3::new(0.2, 3.0, 0.9),
        ] {
            let m = part_mesh(&part(size, kind));
            let Some(VertexAttributeValues::Float32x2(uv)) = m.attribute(Mesh::ATTRIBUTE_UV_0)
            else {
                panic!("{kind:?} uv0 is not float2");
            };
            let span = uv.iter().fold(0.0f32, |acc, p| acc.max(p[0].max(p[1])));
            // The part's longest EDGE decides its widest face, and for the
            // prism that edge is the hypotenuse rather than a side — the one
            // place a box's max-extent shortcut would quietly under-predict.
            let longest = match kind {
                PartKind::Box => size.max_element(),
                PartKind::Tri => size.y.max(size.x.hypot(size.z)),
            };
            let want = longest * PIECE_TILES_PER_M;
            assert!(
                (span - want).abs() < 0.05,
                "{kind:?} {size:?}: uv span {span} against {want} tiles — \
                 the map is not metre-scaled"
            );
        }
    }
}

/// The vertex tint is a MEAN-1 field: it modulates the photograph rather than
/// replacing it (`ART.md` §7). A field whose mean drifts off 1 is a second
/// albedo multiplying the first, which is the pale-ghost failure `props.rs`
/// paid a capture to find.
#[test]
fn the_face_tint_averages_to_one() {
    let m = part_mesh(&part(Vec3::new(2.96, 3.0, 0.2), PartKind::Box));
    let Some(VertexAttributeValues::Float32x4(col)) = m.attribute(Mesh::ATTRIBUTE_COLOR) else {
        panic!("colour is not float4");
    };
    let mean = col.iter().map(|c| c[0] as f64).sum::<f64>() / col.len() as f64;
    assert!(
        (mean - 1.0).abs() < 0.02,
        "face tint mean {mean} is not 1 — it is setting the albedo, not \
         modulating it"
    );
    // …and it is a LUMINANCE field: equal on all three channels, which is
    // what makes its colour-deviation span exactly 1.
    for c in col {
        assert!(
            (c[0] - c[1]).abs() < 1e-6 && (c[1] - c[2]).abs() < 1e-6,
            "face tint {c:?} is chromatic — §7 bounds a per-channel gain"
        );
    }
    // The top face is lighter than the bottom, which is the whole read: a
    // slab whose cap matches its underside is the box we are leaving behind.
    let hi = col.iter().fold(0.0f32, |a, c| a.max(c[0]));
    let lo = col.iter().fold(f32::MAX, |a, c| a.min(c[0]));
    assert!(
        hi - lo > 0.15,
        "face tint range {lo}..{hi} is too flat to read"
    );
}

/// Every mesh `build_kit` builds, built — every shape's every part, and all
/// [`SKIRT_STEPS`] footings in both kinds.
///
/// **This is the one test here that is about the app RUNNING rather than
/// about a value.** `finish` panics on a degenerate triangle rather than
/// shipping a surface whose relief silently vanished, which is the right
/// call — but it moves the failure to first frame, in a system whose name
/// Bevy reports as `<Enable the debug feature to see the name>`, on a screen
/// no other headless test visits. That is the shape of the duplicate-
/// component trap `CLAUDE.md` records: a spawn is not type-checked, so an
/// assembly is a claim you have to run. Running the whole table here is the
/// cheapest possible version of running it.
#[test]
fn the_whole_kit_builds() {
    let mut built = 0;
    for shape in 0..client::render::structures::N_SHAPES as u8 {
        let (parts, n) = client::render::structures::shape_parts(shape);
        for p in &parts[..n] {
            let m = part_mesh(p);
            assert!(m.count_vertices() >= 12, "shape {shape} part is empty");
            built += 1;
        }
    }
    // The footing cache, at every depth the kit pre-builds — including the
    // capped top bucket, whose size is clamped rather than stepped.
    for i in 0..SKIRT_STEPS {
        let depth = (SLAB_T + i as f32 * SKIRT_STEP_M).min(SKIRT_MAX_M);
        let size = Vec3::new(client::render::structures::piece_span(), depth, 2.96);
        for kind in [PartKind::Box, PartKind::Tri] {
            part_mesh(&part(size, kind));
            built += 1;
        }
    }
    assert!(built > 20, "the kit sweep built only {built} meshes");
}

// ---------------------------------------------------------------------------
// §D · The damage response (wire v44)
// ---------------------------------------------------------------------------

/// The band ramp spans its whole range and stays inside `ART.md` rule 3.
///
/// **Rule 3 is the binding constraint and it composes**, which is the part
/// worth gating: a damaged wall's shaded face pays the darkening AND
/// `FACE_BOTTOM` AND the side ramp's foot, all multiplied. Each is defensible
/// alone and the product is what a judge sees, so the assertion is on the
/// product — no lit surface's shaded face below 0.30 of its lit face.
#[test]
fn the_damage_ramp_stays_inside_rule_three() {
    assert_eq!(damage_mix(0), 0.0, "band 0 must be untouched");
    assert_eq!(
        damage_mix(DMG_BANDS - 1),
        1.0,
        "the worst band must be full"
    );
    // Monotonic, so a wall never brightens as it is broken.
    let mut prev = -1.0f32;
    for b in 0..DMG_BANDS {
        let m = damage_mix(b);
        assert!(m > prev, "band {b} mix {m} did not rise from {prev}");
        prev = m;
    }
    // Out of range clamps rather than indexing past the material array.
    assert_eq!(damage_mix(DMG_BANDS + 9), 1.0);

    // The worst case a frame can contain: worst band × bottom face × the
    // side ramp's foot, against rule 3's 0.30 floor.
    let darkest = 1.0 + (DMG_DARKEST - 1.0) * damage_mix(DMG_BANDS - 1);
    let worst = darkest * FACE_BOTTOM * SIDE_FOOT;
    assert!(
        worst >= 0.30,
        "a fully damaged wall's darkest face is {worst} of its lit one — \
         ART.md rule 3 sets the floor at 0.30"
    );
}

// ---------------------------------------------------------------------------
// §C · The footing's quantised depth
// ---------------------------------------------------------------------------

/// `skirt_step` never rounds a skirt SHALLOWER than asked and never indexes
/// past the mesh cache. Rounding down would lift the bottom out of the ground
/// `tests/ghost.rs` requires it to be buried in; running off the end would
/// panic on the steepest cell on the island, found by a player and not by us.
#[test]
fn the_skirt_quantises_up_and_stays_in_range() {
    let mut seen = std::collections::BTreeSet::new();
    // Sweep the whole legal band well past the cap, in steps far finer than
    // the quantum, so every bucket boundary is crossed from both sides.
    let mut raw = SLAB_T;
    while raw <= SKIRT_MAX_M + 1.0 {
        let (i, d) = skirt_step(raw);
        assert!(
            i < SKIRT_STEPS,
            "depth {raw} indexed bucket {i} of {SKIRT_STEPS}"
        );
        assert!(
            d >= raw.min(SKIRT_MAX_M) - 1e-4,
            "depth {raw} quantised DOWN to {d} — the skirt leaves the ground"
        );
        assert!(
            (SLAB_T..=SKIRT_MAX_M).contains(&d),
            "depth {raw} quantised to {d}, outside the drawn band"
        );
        // The bucket the kit built must be the depth the part uses.
        let built = (SLAB_T + i as f32 * SKIRT_STEP_M).min(SKIRT_MAX_M);
        assert!(
            (built - d).abs() < 1e-4,
            "bucket {i} holds a {built} m mesh for a {d} m part"
        );
        seen.insert(i);
        raw += SKIRT_STEP_M / 8.0;
    }
    // The cache is sized to what the sweep actually reaches — a `SKIRT_STEPS`
    // larger than the range is dead meshes, smaller is a clamp nobody sees.
    assert_eq!(
        seen.len(),
        SKIRT_STEPS,
        "the sweep reached {} of {SKIRT_STEPS} buckets",
        seen.len()
    );
}

/// A real cell's footing draws at the depth of the mesh the kit cached for
/// it, and its top is still the level plane.
///
/// **The bucket is read from `footing_of`, never recovered by re-quantising
/// `part.size.y`** — that round trip is the defect this test would otherwise
/// be shipping a copy of. `SLAB_T + 14 × SKIRT_STEP_M` does not reproduce
/// bit-exactly through a subtract-and-divide in f32, so the recovered index
/// can land one bucket high and the foundation would draw 0.25 m deep with
/// its mesh and its size disagreeing. `tests/ghost.rs` owns the burial rule;
/// this owns the part that changed under it — the depth is now discrete.
#[test]
fn a_real_footing_draws_at_its_cached_depth() {
    const SEED: u64 = 20260731;
    // The island's solved sites: both calls read carved ground, so the test
    // has to hand them the same island the client would.
    let haven = sim_core::terrain::haven(SEED);
    let mut checked = 0;
    let mut buckets = std::collections::BTreeSet::new();
    for cx in (300..360).step_by(3) {
        for cz in (300..360).step_by(3) {
            let p = foundation_part(SEED, &haven, cx, cz, false);
            let (i, d) = footing_of(SEED, &haven, cx, cz);
            assert!(i < SKIRT_STEPS, "cell ({cx},{cz}) indexed bucket {i}");
            assert_eq!(
                d, p.size.y,
                "cell ({cx},{cz}): the cached mesh is {d} m and the part is {}",
                p.size.y
            );
            assert!(
                (p.offset.y + p.size.y * 0.5).abs() < 1e-4,
                "cell ({cx},{cz}) footing top left the level plane"
            );
            buckets.insert(i);
            checked += 1;
        }
    }
    assert!(checked >= 400, "the sweep checked only {checked} cells");
    // Real ground has to actually vary, or the sweep proves nothing about
    // quantisation — it would pass identically on a flat plane.
    assert!(
        buckets.len() > 1,
        "every cell in the sweep landed on bucket {:?}",
        buckets
    );
}
