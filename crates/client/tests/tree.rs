//! Gate: the generated conifer fits the volume the sim blocks, and the forest
//! it makes fits the frame's triangle budget.
//!
//! **The load-bearing assertion is the first one, and it is not about art.**
//! `world.rs` derives `SPAWN_CLEAR_M = 4.0` from a sentence about the widest
//! thing the client draws — "the tree cone at radius <= 1.7 m × 1.1 max scale
//! ≈ 1.9 m, plus the 0.4 m capsule". When the pine was authored by hand that
//! sentence was checked by reading the constants. It is now the output of a
//! GENERATOR: nobody authored these vertices, the shape moves when any of a
//! dozen parameters moves, and a canopy that grew past the ceiling would put
//! fresh spawns back inside trees with every other gate green. `ci/
//! pine_shape.mjs` closes exactly this arithmetic for the browser and cannot
//! see across the language boundary; this closes it on our side.
//!
//! It runs headless — no GPU, no window, no shard. The generator is a pure
//! function and geometry is countable, which is the same reason
//! `ci/pine_shape.mjs` was arithmetic rather than a screenshot (that gate is
//! deleted; this suite is the native shape of the same idea).

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::terrain_mesh::{CHUNK_M, NEAR_RADIUS};
use client::render::tree::{
    bounds, canopy_shade, conifer, impostor_of, leaf_image, min_y, needle_image, species_of, tris,
    TreeLod, CONIFER_MAX_TRIS, CONIFER_POOL, IMPOSTOR_MAX_TRIS, SPECIES, TREE_LOD_CAP,
    TREE_LOD_FADE_M, TREE_LOD_SWAP_M, TREE_MAX_R,
};

/// Trees inside the client's 5×5×64 m prop ring, p90 over 271 land eye
/// positions on the shipped seed (20260731), off `sim_core::terrain::scatter`
/// — `cargo run --release -p sim-core --example ring_census` is the command,
/// and the numbers here are its print, never typed from memory. Recorded
/// because the budget assertion below is meaningless without them.
///
/// 328 until forest density v1 (2026-09-14); the four gate seeds read
/// 842–966 at p90 and 1,024–1,080 at the densest ring.
const RING_TREES_P90: usize = 811;

/// The densest ring on the shipped seed, from the same sweep — the worst
/// case the cap arithmetic below is held at, where the p90 is the typical.
const RING_TREES_MAX: usize = 1_086;

/// …and how many trees draw ANY near geometry at the p90 eye — inside
/// [`TREE_LOD_SWAP_M`] plus the [`TREE_LOD_FADE_M`] crossfade, where both
/// LODs are resident — from the same sweep. 198 are inside the swap alone;
/// this is the count `TREE_LOD_CAP` is measured against, so it is the row
/// the budget arithmetic uses. (82 inside the swap before v1, when the
/// distance was enough on its own.)
const NEAR_TREES_DRAWN_P90: usize = 278;

/// …and at the densest eye on the shipped seed (357 with the fade; the four
/// gate seeds reach 361). Over the cap, which is the whole reason the cap
/// exists.
const NEAR_TREES_DRAWN_MAX: usize = 357;

#[test]
fn every_variant_fits_the_volume_the_sim_blocks() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let (h, r) = bounds(&[&bark, &needles]);
        // **Per species now, not one global pair.** A broadleaf is shorter
        // and much wider than a conifer, and holding both to the conifer's
        // envelope would either forbid the broadleaf or slacken the pine's
        // ceiling to fit it. `TREE_MAX_R` is the island-wide number the sim
        // clears; each species has its own inside it.
        let sp = &SPECIES[species_of(v)];
        assert!(
            r <= sp.max_r_m,
            "variant {v} canopy reaches {r:.3} m, past its species ceiling \
             {:.3} m — world.rs clears TREE_MAX_R, so this puts fresh spawns \
             inside trees",
            sp.max_r_m
        );
        assert!(
            sp.max_r_m <= TREE_MAX_R,
            "species ceiling {:.3} exceeds TREE_MAX_R {TREE_MAX_R} — the \
             island-wide constant is what the sim's clearance is derived \
             from, so a species may not quietly opt out of it",
            sp.max_r_m
        );
        // Height is normalised, so this is an equality within float slop and
        // not a range. A tree that is not its species' height is a tree whose
        // fell pivot, wind band and colour bands are measured against the
        // wrong number.
        assert!(
            (h - sp.height_m).abs() < 1e-3,
            "variant {v} is {h:.3} m tall, expected {:.3}",
            sp.height_m
        );
    }
}

/// **The arithmetic `world.rs` claimed was closed and was not.**
///
/// `SPAWN_CLEAR_M`'s comment derived itself from the tree's canopy radius, the
/// slot scale range and the player capsule — and credited `ci/pine_shape.mjs`
/// with checking it. That gate went with the browser client and **does not
/// exist**, so from then until now the derivation was a dead citation: a
/// sentence asserting that something was enforced while nothing was. This is
/// exactly the class `CLAUDE.md` warns is still un-swept in `crates/`.
///
/// Adding a second, wider species is what made it urgent — the ceiling moved
/// 1.7 → 2.9 m, which is precisely the change the missing gate existed to
/// catch. So it is written in Rust, in the crate that can see both sides.
#[test]
fn a_fresh_spawn_stands_clear_of_the_widest_tree() {
    // The widest canopy edge a slot can actually present: the ceiling at the
    // largest scale `scatter` may roll.
    let canopy = TREE_MAX_R * sim_core::terrain::SLOT_SCALE_MAX;
    // Touching distance — canopy edge plus the player's own radius.
    let touching = canopy + sim_core::collide::CAPSULE_RADIUS_M;
    assert!(
        touching < sim_core::world::SPAWN_CLEAR_M,
        "a fresh spawn would touch the widest tree: canopy {canopy:.3} m + \
         capsule {:.3} m = {touching:.3} m against SPAWN_CLEAR_M {:.3} m",
        sim_core::collide::CAPSULE_RADIUS_M,
        sim_core::world::SPAWN_CLEAR_M
    );
    // **Not merely outside it — standing clear of it**, which is the phrase
    // `world.rs` uses and the reason the constant is not simply `touching`.
    // Room to stand and turn, so a spawn does not open with a trunk filling
    // the frame.
    const STANDING_ROOM_M: f32 = 0.85;
    assert!(
        touching + STANDING_ROOM_M <= sim_core::world::SPAWN_CLEAR_M,
        "clearance {:.3} m leaves only {:.3} m past touching the widest \
         canopy; the rule is standing room, not tangency",
        sim_core::world::SPAWN_CLEAR_M,
        sim_core::world::SPAWN_CLEAR_M - touching
    );
}

#[test]
fn every_variant_is_rooted_at_the_ground() {
    // `spawn_slot` places the mesh origin at the slot's surface y and applies
    // no lift to a tree, so the TRUNK's base is what has to be 0: a bark mesh
    // that starts anywhere else floats or sinks every tree in the world at
    // once, by exactly that much.
    //
    // **The needles are held to a different rule on purpose.** Foliage that
    // dips below the trunk's base is not a defect — `ART.md` rule 2 is that
    // nothing sits ON the ground and everything sits IN it, and the lowest
    // branch's cards grazing the surface is that rule, not a violation of it.
    // What would be a defect is a canopy hanging far enough under to bury
    // whole cards, which is geometry drawn for nobody. Measured at −0.0011 m
    // with the shipped parameters; the bound is one card's half-height.
    const NEEDLE_DIP_MAX_M: f32 = 0.35;
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let trunk = min_y(&[&bark]);
        assert!(
            trunk.abs() < 1e-3,
            "variant {v}'s trunk starts at y={trunk:.4}, not 0 — every tree would float or sink"
        );
        let canopy = min_y(&[&needles]);
        assert!(
            canopy > -NEEDLE_DIP_MAX_M,
            "variant {v}'s canopy hangs {:.3} m below the trunk base, past \
             {NEEDLE_DIP_MAX_M} m — that is cards buried in the terrain",
            -canopy
        );
    }
}

#[test]
fn the_pool_is_distinct_silhouettes() {
    // `ART.md` rule 7: no two identical instances adjacent. Yaw and scale
    // vary per slot, but at the measured p90 of 811 trees in the draw ring one
    // silhouette repeated 811 times reads as one silhouette repeated.
    let shapes: Vec<(f32, f32)> = (0..CONIFER_POOL)
        .map(|v| {
            let (bark, needles) = conifer(v);
            bounds(&[&bark, &needles])
        })
        .collect();
    for i in 0..shapes.len() {
        for j in (i + 1)..shapes.len() {
            assert!(
                (shapes[i].1 - shapes[j].1).abs() > 1e-4,
                "variants {i} and {j} have the same canopy radius {:.5} — \
                 the pool is not seeding distinct trees",
                shapes[i].1
            );
        }
    }
}

#[test]
fn one_tree_stays_inside_its_triangle_ceiling() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let t = tris(&bark) + tris(&needles);
        assert!(
            t <= CONIFER_MAX_TRIS,
            "variant {v} is {t} tris, past CONIFER_MAX_TRIS {CONIFER_MAX_TRIS}"
        );
    }
}

/// **The ring fits now, and the number this used to print is why the LOD
/// exists — and why, since forest density v1, it is a COUNT and not only a
/// distance.**
///
/// This test asserted the opposite for as long as the generator has shipped:
/// a full-detail conifer at the ring's p90 does not fit `DESIGN.md` §9's 1.5 M
/// and no parameter change makes it fit — measured, by distance band on the
/// old forest: 40 m holds 21 trees (~89 k tris), 80 m holds 82 (~350 k),
/// 120 m holds 168 (~709 k), 160 m holds 288 (~1.22 M). It printed the debt
/// rather than asserting it away, so it could not be forgotten by anyone
/// reading a green suite.
///
/// `tree::impostor_of` is the answer to it and the arithmetic below is the
/// whole claim: everything past [`TREE_LOD_SWAP_M`] costs a hull instead of a
/// tree. Then the forest was made a forest (~39 → ~94 stems/ha) and the 80 m
/// disc stopped being enough — it holds 278 trees at p90 and 357 at the
/// worst, which is 2.1 M before a hull — so [`TREE_LOD_CAP`] bounds the
/// drawn pair by count and the swap distance is where the cap lands. Three
/// halves are asserted — the ring fits at p90, it fits at the densest eye
/// on the island, AND it would not fit without the swap — because a gate
/// that only checks the good number stays green if the impostor quietly
/// becomes a copy of the tree.
#[test]
fn the_far_band_is_an_impostor_and_the_ring_fits() {
    // The band table above is measured AT 80 m. If the knob moves, the tree
    // count that goes with it has to be re-measured, not reused.
    assert!(
        (TREE_LOD_SWAP_M - 80.0).abs() < 1e-6,
        "TREE_LOD_SWAP_M is {TREE_LOD_SWAP_M} m and the p90 band table in this \
         gate was measured at 80 m — re-measure the tree count at the new \
         distance (`sim-core/examples/ring_census.rs`) rather than reusing \
         {NEAR_TREES_DRAWN_P90}"
    );

    // Worst case over the pool rather than variant 0: the budget is paid by
    // whichever tree the scatter actually places, and `CONIFER_POOL` is six.
    let mut per_tree = 0usize;
    let mut per_far = 0usize;
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        per_tree = per_tree.max(tris(&bark) + tris(&needles));
        per_far = per_far.max(tris(&impostor_of(&bark, &needles, v)));
    }

    let was = per_tree * RING_TREES_P90;
    let drawn_p90 = NEAR_TREES_DRAWN_P90.min(TREE_LOD_CAP);
    let now = per_tree * drawn_p90 + per_far * (RING_TREES_P90 - drawn_p90);
    let drawn_max = NEAR_TREES_DRAWN_MAX.min(TREE_LOD_CAP);
    let worst = per_tree * drawn_max + per_far * (RING_TREES_MAX - drawn_max);
    println!(
        "conifer {per_tree} tris/tree · impostor {per_far} tris · ring at p90 \
         {RING_TREES_P90} trees: {was} tris full detail -> {now} with the \
         {TREE_LOD_SWAP_M} m swap and the {TREE_LOD_CAP}-tree cap ({:.1}x); \
         densest ring {RING_TREES_MAX} trees -> {worst}",
        was as f32 / now.max(1) as f32
    );
    assert!(
        now < 1_500_000,
        "the ring costs {now} tris with the LOD in — past DESIGN.md §9's 1.5 M, \
         which is the ceiling this LOD exists to get under"
    );
    assert!(
        worst < 1_500_000,
        "the densest ring on the island costs {worst} tris with the LOD and the \
         cap in — past DESIGN.md §9's 1.5 M. The cap is what bounds this; if \
         the forest got denser, the cap is what moves, not the rail"
    );
    // And the cap is doing work: without it the densest eye is over budget,
    // which is the premise of `tree::cap_swap` and of `tests/tree_cap.rs`.
    let uncapped =
        per_tree * NEAR_TREES_DRAWN_MAX + per_far * (RING_TREES_MAX - NEAR_TREES_DRAWN_MAX);
    assert!(
        uncapped > 1_500_000,
        "the densest eye draws only {uncapped} tris with no cap, so the cap is \
         not what keeps the frame under budget — re-measure the census before \
         believing TREE_LOD_CAP still matters"
    );
    assert!(
        was > 1_500_000,
        "the full-detail ring is only {was} tris, so the premise of this LOD \
         has changed — re-measure the budget claim in tree.rs and RENDER.md \
         rather than deleting this"
    );
}

/// The hull may not be bigger than the tree it stands in for, in any
/// direction.
///
/// **This is the sim's arithmetic, not the picture's.** `world.rs` derives
/// `SPAWN_CLEAR_M` from the widest thing the client draws, and an impostor
/// that bulged past its species ceiling would put fresh spawns inside a tree
/// exactly as a runaway canopy would — with `every_variant_fits_the_volume_
/// the_sim_blocks` above still green, because that one never sees this mesh.
#[test]
fn the_impostor_stands_inside_the_tree_it_replaces() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let far = impostor_of(&bark, &needles, v);
        let (th, tr) = bounds(&[&bark, &needles]);
        let (fh, fr) = bounds(&[&far]);
        let sp = &SPECIES[species_of(v)];

        assert!(
            fr <= tr + 1e-3,
            "variant {v}'s impostor reaches {fr:.3} m against the tree's \
             {tr:.3} — a hull lathed through the tree's own vertices cannot be \
             wider than they are, so this is a radius that came from somewhere \
             else"
        );
        assert!(
            fr <= sp.max_r_m,
            "variant {v}'s impostor reaches {fr:.3} m, past its species \
             ceiling {:.3} — world.rs clears TREE_MAX_R",
            sp.max_r_m
        );
        // …and the other direction, which is the failure `IMPOSTOR_GIRTH_Q`
        // can produce silently: a hull trimmed until it is a stick. Every
        // other assertion here passes on one — it is inside the tree, it is
        // cheap, it is rooted, and the ring arithmetic gets CHEAPER. Measured
        // today the ratio is 0.65 on a conifer and 0.72 on a broadleaf, so a
        // quarter is loose without being decorative.
        assert!(
            fr >= tr * 0.25,
            "variant {v}'s impostor is {fr:.3} m against the tree's {tr:.3} — \
             the far forest would be a field of sticks"
        );
        assert!(
            (fh - th).abs() <= th * 0.02,
            "variant {v}'s impostor is {fh:.3} m tall against the tree's \
             {th:.3} — the swap would read as the tree changing size"
        );
        assert!(
            min_y(&[&far]).abs() <= 1e-3,
            "variant {v}'s impostor is rooted at {:.4} rather than 0 — it \
             would float or sink at the swap distance",
            min_y(&[&far])
        );
    }
}

/// The hull faces outward.
///
/// **The one way this slice fails invisibly.** A lathe assembled with its
/// winding reversed is back-face culled from outside, so the far forest simply
/// is not there — and every other assertion in this suite passes, because the
/// triangle count, the bounds and the budget arithmetic are all identical for
/// a hull turned inside out. `pine_mesh` is where the winding was copied from
/// and it is dead code that has never been drawn natively, so "it worked in
/// the browser" is not evidence about this build.
///
/// ⚠ **The first draft of this test asserted the NORMALS and the mutant
/// survived it**, which is the whole reason `CLAUDE.md` says to run one.
/// `Soup::tri` blends its facet normal 0.7 of the way toward the volume
/// direction (`PINE_NORMAL_BLEND`), so an inside-out hull still shades
/// outward — the assertion was measuring the blend, not the winding. What the
/// rasterizer culls on is the VERTEX ORDER, so that is what this reads:
/// `(b − a) × (c − a)`, the same right-hand rule, against the outward
/// direction at the triangle's own centre.
#[test]
fn the_impostor_faces_outward() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let far = impostor_of(&bark, &needles, v);
        let pos = match far.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(p)) => p.clone(),
            _ => panic!("the impostor has positions"),
        };
        // `Soup` emits three fresh vertices per triangle and indexes them
        // 0..n, so consecutive triples ARE the triangles.
        assert!(
            pos.len() % 3 == 0 && !pos.is_empty(),
            "variant {v}'s impostor has {} positions, which is not a triangle \
             soup",
            pos.len()
        );
        for (t, tri) in pos.chunks_exact(3).enumerate() {
            let [a, b, c] = [
                Vec3::from_array(tri[0]),
                Vec3::from_array(tri[1]),
                Vec3::from_array(tri[2]),
            ];
            let facet = (b - a).cross(c - a);
            let mid = (a + b + c) / 3.0;
            let out = Vec3::new(mid.x, 0.0, mid.z);
            // A triangle whose centre sits on the axis has no outward
            // direction to test against; the tip ring is exactly that.
            if out.length() <= 1e-4 {
                continue;
            }
            assert!(
                facet.dot(out) > 0.0,
                "variant {v} triangle {t} winds the wrong way — its right-hand \
                 normal {facet:?} points against the outward direction {out:?}, \
                 so it is back-face culled and the far forest is invisible"
            );
        }
    }
}

/// …and it has to be cheap, or the swap buys nothing.
#[test]
fn the_impostor_is_cheap() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let far = impostor_of(&bark, &needles, v);
        let t = tris(&far);
        assert!(
            t <= IMPOSTOR_MAX_TRIS,
            "variant {v}'s impostor is {t} tris, past IMPOSTOR_MAX_TRIS \
             {IMPOSTOR_MAX_TRIS} — the lathe emitted more than its own bands \
             and sides allow"
        );
        // A tenth would still be a win on paper and would not be one in a
        // frame: the point of the swap is that a distant tree stops being
        // most of the geometry, not that it gets a discount.
        let full = tris(&bark) + tris(&needles);
        assert!(
            t * 20 <= full,
            "variant {v}'s impostor is {t} tris against the tree's {full} — \
             under 20x is not a level of detail, it is a second tree"
        );
        assert!(
            t > 0,
            "variant {v}'s impostor is empty — an invisible far band is a \
             forest that vanishes at {TREE_LOD_SWAP_M} m, and every triangle \
             count in this suite would still pass"
        );
    }
}

/// The two bands have to meet exactly, and the far one has to outlast the ring
/// that spawns it.
///
/// Bevy's own `VisibilityRange` documentation states the first requirement —
/// *"the `end_margin` of a higher LOD is always identical to the
/// `start_margin` of the next lower LOD; this is important for the crossfade
/// effect to function properly"*. A gap between them is a forest that blinks
/// out for fifteen metres; an overlap is both LODs drawn at once, which is the
/// cost this slice exists to avoid, paid twice.
#[test]
fn the_two_lod_bands_meet_and_the_far_one_outlasts_the_ring() {
    let lod = TreeLod::default();
    let (near, far) = (&lod.near, &lod.far);
    assert_eq!(
        near.end_margin, far.start_margin,
        "the near LOD fades out over {:?} and the far one fades in over {:?} — \
         Bevy crossfades between a matched pair and does not interpolate \
         across a gap",
        near.end_margin, far.start_margin
    );
    assert!(
        near.is_visible_at_all(0.0) && !far.is_visible_at_all(0.0),
        "at the eye, a tree must be its own geometry and nothing else"
    );
    let mid = TREE_LOD_SWAP_M + TREE_LOD_FADE_M * 0.5;
    assert!(
        near.is_visible_at_all(mid) && far.is_visible_at_all(mid),
        "inside the fade band both LODs must be resident — that is what the \
         dither crossfades between"
    );
    let past = TREE_LOD_SWAP_M + TREE_LOD_FADE_M + 1.0;
    assert!(
        !near.is_visible_at_all(past) && far.is_visible_at_all(past),
        "past the fade band the tree must be the hull alone, or the swap has \
         bought nothing"
    );

    // The ring's own reach, and this is the assertion that keeps the LOD from
    // quietly becoming a cull. `props::stream` holds chunks within
    // NEAR_RADIUS of the eye's own, so the farthest tree it can have spawned
    // is the far corner of the outermost chunk.
    let diagonal = (NEAR_RADIUS as f32 + 1.0) * CHUNK_M * std::f32::consts::SQRT_2;
    assert!(
        far.end_margin.start >= diagonal,
        "the far LOD fades out at {} m but the prop ring reaches {diagonal:.1} m \
         — trees the ring still holds would disappear, and ART.md §8 wants the \
         far third of the frame populated",
        far.end_margin.start
    );
}

#[test]
fn the_needle_card_is_actually_cut_out() {
    card_is_cut_out(needle_image(), "needle");
}

/// The broadleaf's card, held to the same two bounds — and to a third that is
/// the whole reason it exists: it is DENSER than the sprig, and it is not the
/// sprig. Both species wore the needle card until forest scale v0, and the
/// broadleaf read as a second conifer on the bench.
#[test]
fn the_leaf_card_is_actually_cut_out_and_is_not_the_needle() {
    card_is_cut_out(leaf_image(), "leaf");
    let leaf = level0_alphas(&leaf_image());
    let needle = level0_alphas(&needle_image());
    assert_eq!(leaf.len(), needle.len(), "the two cards are one size");
    let opaque = |a: &[u8]| a.iter().filter(|&&v| v > 128).count();
    assert!(
        opaque(&leaf) > opaque(&needle),
        "the leaf card ({} opaque texels) is not denser than the sprig ({}) — a \
         leaf cluster is mostly leaf where a sprig is mostly air",
        opaque(&leaf),
        opaque(&needle)
    );
    let differ = leaf
        .iter()
        .zip(needle.iter())
        .filter(|(l, n)| (**l > 128) != (**n > 128))
        .count();
    assert!(
        differ * 100 / leaf.len() > 15,
        "the two cards differ on only {}% of texels — the broadleaf is wearing \
         the sprig again",
        differ * 100 / leaf.len()
    );
}

fn level0_alphas(img: &bevy::prelude::Image) -> Vec<u8> {
    let data = img.data.as_ref().expect("card has no data");
    let w = img.texture_descriptor.size.width as usize;
    data[..w * w * 4].chunks_exact(4).map(|p| p[3]).collect()
}

fn card_is_cut_out(img: bevy::prelude::Image, what: &str) {
    // An `AlphaMode::Mask` material with a fully opaque map is an opaque quad,
    // which is the hull `props.js` spent three passes rejecting — and it would
    // look like a solid green square, not like an error. So: the map must have
    // real transparency, and it must have real coverage. LEVEL 0 ONLY: the
    // data carries the whole mip chain, and reading all of it would average
    // the base card together with a 1×1 texel — this is about the card the
    // artist sees.
    let alphas = level0_alphas(&img);
    let opaque = alphas.iter().filter(|&&a| a > 128).count();
    let clear = alphas.iter().filter(|&&a| a < 16).count();
    let total = alphas.len();

    assert!(
        clear * 100 / total > 30,
        "only {}% of the {what} card is transparent — that is a quad, not a card",
        clear * 100 / total
    );
    assert!(
        opaque * 100 / total > 5,
        "only {}% of the {what} card is opaque — the canopy would be invisible",
        opaque * 100 / total
    );
}

/// **The blocking cylinder is the TRUNK, and until now nothing measured it.**
///
/// `terrain::OCCUPANT_R_M`'s `Tree` row read 0.26 with the comment
/// "`CylinderGeometry(0.13, 0.26)`" — three.js, i.e. the bottom radius of the
/// *browser* client's hand-authored cone pine. That mesh is deleted. The
/// native client draws `bevy_procedural_tree` output whose base measures
/// ~0.19 m (pine) and ~0.24 m (broadleaf), so the sim blocked a cylinder up
/// to 0.11 m proud of the bark at chest height — an invisible skirt a player
/// is stopped by, reported from play as a trunk sitting to the side of the
/// thing that stopped them.
///
/// It survived because `tests/greybox.rs` — which holds every OTHER
/// archetype's row to its drawn mesh at 1 mm — **excuses** `Occupant::Tree`
/// on the grounds that it is "gated variant by variant in tests/tree.rs".
/// That was false: this file gated the CANOPY ceiling (`TREE_MAX_R`, which
/// feeds `SPAWN_CLEAR_M`), the height, the rooting and the tri count, and
/// never mentioned `OCCUPANT_R_M`. The one occupant whose mesh is generated
/// was the one occupant whose blocking radius nothing checked. Now it does,
/// and the excuse over there says what is actually true.
///
/// **Both directions, like greybox's equality**, because the two failures are
/// different and both are real: too narrow and a body stands inside visible
/// bark, too wide and it is stopped by nothing it can see.
#[test]
fn the_blocked_cylinder_is_the_trunk_the_client_draws() {
    use client::render::tree::{trunk_radius, TRUNK_MEASURE_H_M};

    let published = sim_core::terrain::OCCUPANT_R_M[sim_core::terrain::Occupant::Tree as usize];
    let mut widest = 0.0f32;
    for v in 0..CONIFER_POOL {
        let (bark, _) = conifer(v);
        let r = trunk_radius(&bark);
        // The measurement must still BE a trunk. The generator starts limbs
        // well above `TRUNK_MEASURE_H_M` today; a species that changed that
        // would silently turn this gate into one about branches and push the
        // sim's cylinder out to seal the forest, so it fails here instead.
        assert!(
            r > 0.05 && r < 0.5,
            "variant {v}'s bark measures {r:.4} m below {TRUNK_MEASURE_H_M} m — \
             that is not a trunk, so this gate is no longer measuring one"
        );
        assert!(
            r <= published + 1.0e-4,
            "variant {v}'s trunk is {r:.4} m at the base but the sim blocks \
             {published:.4} m — a body would stand inside drawn bark"
        );
        widest = widest.max(r);
    }
    // No invisible skirt: the same 1 mm the authored rows are held to.
    assert!(
        published - widest <= 0.001,
        "the sim blocks {published:.4} m and the widest trunk drawn is \
         {widest:.4} m — that is {:.4} m of cylinder a player is stopped by \
         and cannot see. Set OCCUPANT_R_M's Tree row to the measurement, \
         rounded outward.",
        published - widest
    );
}

/// The limbs are deliberately outside the blocked volume, and saying so here
/// is what keeps the gate above from being read as a bug.
///
/// `greybox.rs`'s rule for every other archetype is "nothing drawn may reach
/// past what stops a body". A tree cannot be held to it and should not be: the
/// bark mesh's limbs reach 0.86 m (pine) and 2.38 m (broadleaf) inside the
/// capsule's height band, and a cylinder that covered them would seal a forest
/// a player is meant to walk through. So a tree is the one archetype whose
/// drawn geometry intentionally exceeds its collision, and this records the
/// measurement rather than leaving the next reader to wonder.
#[test]
fn the_limbs_reach_past_the_trunk_and_that_is_the_intent() {
    use client::render::tree::trunk_radius;

    let band = sim_core::collide::CAPSULE_HEIGHT_M;
    let mut worst = 0.0f32;
    for v in 0..CONIFER_POOL {
        let (bark, _) = conifer(v);
        let (_, full) = bounds(&[&bark]);
        assert!(
            full > trunk_radius(&bark),
            "variant {v}'s bark never reaches past its own trunk — the \
             generator stopped producing limbs, which is a render change \
             this gate should not be the one to discover"
        );
        worst = worst.max(full);
    }
    println!(
        "bark reaches {worst:.3} m against a {:.4} m blocked cylinder — limbs \
         are passable by design (capsule band {band} m)",
        sim_core::terrain::OCCUPANT_R_M[sim_core::terrain::Occupant::Tree as usize],
    );
}

/// **Gate: the canopy keeps its coverage all the way down the mip chain.**
///
/// The map shipped with `mip_level_count` at 1 until 2026-08-25, which is two
/// defects wearing one cause. The visible one is shimmer — a 64² alpha mask
/// minified with no filtering re-picks which needles win the sample every time
/// the camera moves, and no still frame this project has ever captured could
/// show it. The one that outlives the fix is what a *naive* chain would have
/// done instead: box-filtering a sparse mask drives every texel toward the
/// mask's mean, the mean of a sprig is well under the 0.5 cutoff, and each
/// level would test out more of the canopy than the one above it. Four levels
/// down the tree goes bald, which reads as a thinning forest rather than as a
/// texture bug.
///
/// So the assertion is not "there are mips" — it is that each level survives
/// the *same threshold the material applies* at roughly the same rate. Proven
/// red by pinning the rescale at 1.0, i.e. returning the box-filtered level as
/// it comes: **level 1 alone falls to 0.53× the base coverage** (0.102 against
/// 0.192) against the 0.55 floor below, and it is the first level, one halving
/// from full detail.
#[test]
fn the_needle_chain_holds_its_coverage() {
    chain_holds_its_coverage(needle_image(), "needle");
}

/// The leaf card rides the same chain builder and is held to the same band —
/// the gate exists so a second card cannot ship with a chain that goes bald.
#[test]
fn the_leaf_chain_holds_its_coverage() {
    chain_holds_its_coverage(leaf_image(), "leaf");
}

fn chain_holds_its_coverage(img: bevy::prelude::Image, what: &str) {
    let data = img.data.as_ref().expect("needle image has no data");
    let w0 = img.texture_descriptor.size.width;
    let levels = img.texture_descriptor.mip_level_count;

    // 64 → 1 is seven levels. A chain that stops early is a chain that still
    // aliases at the range the forest is actually seen from.
    assert_eq!(
        levels,
        w0.ilog2() + 1,
        "{what} mask has {levels} mip levels for a {w0}² card — the chain must \
         reach 1×1 or minification falls off the end of it"
    );

    // The cutoff the foliage material tests against, as a byte. Authored in
    // `props.rs` as `AlphaMode::Mask(0.5)`; alpha is linear even in an sRGB
    // texture, so 0.5 is 128.
    const CUT: u8 = 128;

    let coverage = |px: &[u8]| -> f32 {
        let hit = px.chunks_exact(4).filter(|p| p[3] > CUT).count();
        hit as f32 / (px.len() / 4) as f32
    };

    let mut off = 0usize;
    let mut w = w0;
    let mut base = 0.0f32;
    for level in 0..levels {
        let bytes = (w * w) as usize * 4;
        let cov = coverage(&data[off..off + bytes]);
        if level == 0 {
            base = cov;
            assert!(
                base > 0.0,
                "level 0 of the {what} mask has no coverage at all"
            );
        } else if w >= 4 {
            // Below 4×4 there are sixteen texels and exact coverage is not
            // reachable at any scale, so the band is only held where it means
            // something. Those levels are also sub-pixel in every frame that
            // has ever existed.
            let ratio = cov / base;
            assert!(
                (0.55..=1.75).contains(&ratio),
                "{what} mip level {level} ({w}²) tests to {ratio:.2}× level 0's \
                 coverage ({cov:.3} against {base:.3}) — outside 0.55..1.75 the \
                 canopy visibly thickens or goes bald at that range"
            );
        }
        off += bytes;
        w /= 2;
    }
    assert_eq!(
        off,
        data.len(),
        "the mip chain's declared levels and the buffer's length disagree — \
         wgpu would read past the end of the last level"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Canopy grain v0 (2026-09-16). The operator's frames read as a forest of tree
// ferns under a flat green plate; these four gates hold the mechanisms that
// answer it. Each was proven red under the defect it names — `NOW.md` §0t.
// ─────────────────────────────────────────────────────────────────────────────

/// The MEASURED median world-space edge of one canopy card, metres.
///
/// **Not `LeafParams::size`.** That is in the generator's own units and
/// `fit_to_bounds` rescales the mesh after the cards are built, by 0.91 on the
/// conifer and **2.13** on the broadleaf. Every statement about how big a thing
/// on this card is in millimetres divides by this, so reading the authored
/// number instead is how the broadleaf's leaves came to be 38 cm long with
/// a comment beside them saying 0.60 m (`tree::leaf_image`).
fn card_edge_m(variant: usize) -> f32 {
    let (_, needles) = conifer(variant);
    let Some(VertexAttributeValues::Float32x3(p)) = needles.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("canopy mesh has no positions");
    };
    let mut e: Vec<f32> = p
        .chunks_exact(4)
        .map(|q| {
            let a = Vec3::from_array(q[0]);
            (Vec3::from_array(q[1]) - a)
                .length()
                .max((Vec3::from_array(q[3]) - a).length())
        })
        .collect();
    e.sort_by(|x, y| x.partial_cmp(y).expect("card edge is NaN"));
    e[e.len() / 2]
}

/// Level 0's coverage, the median width of one opaque run along a row, and the
/// card's side in texels.
///
/// **The side is returned rather than assumed.** A millimetre figure is
/// `run × card_m / side`, and hard-coding `NEEDLE_TEX`'s current value here
/// would make every such figure silently wrong the next time the card's
/// resolution moves — which is the same class of mistake as reading
/// `LeafParams::size` as metres, one file over.
fn grain(img: &Image) -> (f32, usize, usize) {
    let w = img.texture_descriptor.size.width as usize;
    let d = img.data.as_ref().expect("mask has no data");
    let lvl0 = &d[..w * w * 4];
    let hit = lvl0.chunks_exact(4).filter(|p| p[3] > 128).count();
    let mut runs: Vec<usize> = vec![];
    for row in 0..w {
        let mut run = 0usize;
        for col in 0..w {
            if lvl0[(row * w + col) * 4 + 3] > 128 {
                run += 1;
            } else if run > 0 {
                runs.push(run);
                run = 0;
            }
        }
        if run > 0 {
            runs.push(run);
        }
    }
    runs.sort_unstable();
    assert!(!runs.is_empty(), "mask is empty");
    (hit as f32 / (w * w) as f32, runs[runs.len() / 2], w)
}

/// **A needle is 1–2 mm wide and a frond is not, and the mask could not tell
/// the difference until it had the texels to.**
///
/// The 64² card at 16.9 mm per texel could draw nothing finer than a 34 mm
/// stripe, and drew them 23 cm long: measured median run width **34 mm**
/// (`examples/canopy_probe.rs`, 2026-09-16), which is why the canopy read as
/// fern fans. This holds the median opaque run — the width of one drawn
/// element — to something a conifer could actually grow.
///
/// ⚠ **The bound is in MILLIMETRES OF TREE, not texels**, and that is the
/// whole point: a texel count says nothing without the card's world size
/// beside it, and it was exactly that missing division that let a 38 cm leaf
/// ship under a comment claiming 0.60 m.
#[test]
fn the_canopy_elements_are_the_size_of_real_ones() {
    // A conifer needle: 1–2 mm real, and `NEEDLE_W` draws the finest line that
    // survives the 0.5 alpha cut at all. Anything over a centimetre is a frond.
    let (_, run, side) = grain(&needle_image());
    let mm = run as f32 * card_edge_m(0) * 1000.0 / side as f32;
    assert!(
        (2.0..=10.0).contains(&mm),
        "the needle mask's median element is {mm:.1} mm of tree ({run} texels) \
         — a conifer needle is 1–2 mm and the 64² card drew 34 mm fronds; \
         outside 2–10 mm this canopy is made of the wrong thing"
    );

    // The leaf card's elements fuse where they overlap, so a run measures the
    // spray as much as one leaf. What it still catches is the old defect: the
    // 64² card's 118 mm run WAS a single leaf, 37 cm long.
    let (_, lrun, lside) = grain(&leaf_image());
    let lmm = lrun as f32 * card_edge_m(3) * 1000.0 / lside as f32;
    assert!(
        lmm <= 90.0,
        "the leaf mask's median element is {lmm:.0} mm of tree ({lrun} texels) \
         — the 64² card's 118 mm was a single leaf 37 cm long"
    );
}

/// **Coverage is the density and grain is not allowed to move it.**
///
/// What reaches the frame through `AlphaMode::Mask` is the share of texels over
/// the cut, so coverage IS how opaque a canopy is. Rebuilding both masks at
/// 256² was a change of grain, and these bands are the fern card's own numbers
/// — 0.192 needle, 0.255 leaf — so the forest neither thinned nor thickened
/// under it. They are also the lever if it needs to: `TREE_MAX_R` is the sim's
/// (`NOW.md` §0t item 2) and this is free.
///
/// ⚠ **The bands are ±7 % and CENTRED on the shipped value, and both of those
/// came from mutants walking through a first draft.** At ±15 % (0.17–0.22)
/// tripling `AXIS_NEEDLES` passed — which turned out to say something about
/// the KNOB rather than the gate, and is the useful half: swept in isolation,
/// `AXIS_NEEDLES` 120 → 320 moves coverage only **0.178 → 0.202**, because
/// axis needles land on ground the twigs already cover, while `TWIGS` and
/// `NEEDLES_PER_TWIG` each move it ~0.145 → ~0.228 over the same relative
/// range. So a density retune goes through the twig counts; the axis count is
/// nearly saturated and cannot deliver one. Then the tightened band was left
/// centred on the value it was derived at rather than the value that shipped
/// — 0.188 against 0.185, after the twig jitter landed — and `TWIGS 23` came
/// to rest on **0.201, the inclusive rail exactly**, and passed. A band is a
/// claim about a distance from a number, so it has to be re-centred whenever
/// that number moves.
#[test]
fn the_cards_hold_the_density_the_forest_was_built_at() {
    let (needle, _, _) = grain(&needle_image());
    assert!(
        (0.172..=0.198).contains(&needle),
        "the needle mask tests to {needle:.3} coverage against the 0.185 it is \
         built at and the 0.192 the canopy's stem counts and crown radii were \
         swept at — outside this band the forest is a different density, not a \
         different grain"
    );
    let (leaf, _, _) = grain(&leaf_image());
    assert!(
        (0.241..=0.271).contains(&leaf),
        "the leaf mask tests to {leaf:.3} coverage against 0.256"
    );
    // `tests/tree.rs`'s older gate says the leaf card must be the denser of the
    // two — a cluster is mostly leaf, a sprig mostly air — and two independent
    // bands could both pass while crossing.
    assert!(
        leaf > needle,
        "the leaf card ({leaf:.3}) must stay denser than the needle card \
         ({needle:.3}) or the two species stop reading apart"
    );
}

/// Canopy vertex colours binned by how deep inside the crown they sit, at a
/// fixed height — `(buried, exposed)` mean linear luma.
///
/// **Held at a fixed height on purpose.** `band` ramps the canopy on `y`, so a
/// depth statistic taken across the whole tree mixes the two axes and a cone's
/// widest vertices are also its lowest and darkest — which is how the conifer
/// measured its rim *darker* than its core before this landed, 0.077 against
/// 0.129, and read as evidence of nothing.
fn depth_luma(variant: usize) -> (f64, f64) {
    let (_, needles) = conifer(variant);
    let Some(VertexAttributeValues::Float32x3(p)) = needles.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("no positions")
    };
    let Some(VertexAttributeValues::Float32x4(c)) = needles.attribute(Mesh::ATTRIBUTE_COLOR) else {
        panic!("no colours")
    };
    let (lo, hi) = p
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v[1]), h.max(v[1])));
    // The middle third of the crown: enough vertices to mean anything, narrow
    // enough that the height ramp is near-constant across it.
    let (y0, y1) = (lo + (hi - lo) * 0.40, lo + (hi - lo) * 0.60);
    let r_at = |v: &[f32; 3]| (v[0] * v[0] + v[2] * v[2]).sqrt();
    let band: Vec<usize> = (0..p.len())
        .filter(|&i| p[i][1] >= y0 && p[i][1] <= y1)
        .collect();
    assert!(
        band.len() > 100,
        "only {} vertices in the slice",
        band.len()
    );
    let rmax = band.iter().map(|&i| r_at(&p[i])).fold(0.0f32, f32::max);
    let luma =
        |i: usize| 0.2126 * c[i][0] as f64 + 0.7152 * c[i][1] as f64 + 0.0722 * c[i][2] as f64;
    let mean = |f: &dyn Fn(f32) -> bool| {
        let v: Vec<usize> = band
            .iter()
            .copied()
            .filter(|&i| f(r_at(&p[i]) / rmax))
            .collect();
        assert!(!v.is_empty(), "empty depth bin");
        v.iter().map(|&i| luma(i)).sum::<f64>() / v.len() as f64
    };
    (mean(&|t| t < 0.35), mean(&|t| t > 0.80))
}

/// **A canopy is a VOLUME and this tree had no inside.**
///
/// `band` ramps the needle mesh on height alone, so before `occlude_canopy`
/// every needle at a given height was the same colour whether it was buried
/// against the trunk or out at a limb tip: measured 0.156 → 0.147 core to rim
/// on the broadleaf, **6 % across the whole crown**. Real foliage is a dark
/// interior under a lit shell, and that contrast is what makes the reference's
/// crowns read as masses rather than as cut-outs.
///
/// Proven red by deleting the `occlude_canopy` call: the ratio falls to
/// 1.01–1.03 on both species, against the 1.9× floor here.
#[test]
fn the_crown_is_darker_inside_than_out() {
    for variant in [0usize, 3] {
        let (buried, exposed) = depth_luma(variant);
        let ratio = exposed / buried;
        assert!(
            ratio >= 1.9,
            "variant {variant}: the crown's core reads {buried:.4} against its \
             rim's {exposed:.4} — only {ratio:.2}× — at one height, where a \
             foliage mass is several times darker inside. A canopy with no \
             interior is the flat green plate `NOW.md` §0t is about"
        );
        // And it must not invert: the floor is a floor, not a hole.
        assert!(
            buried > 0.0,
            "variant {variant}: the crown's core is fully black — \
             `CANOPY_AO_FLOOR` is `ART.md` rule 3's 0.30 and must be honoured"
        );
    }
}

/// **A canopy whose normals are all horizontal has no top and no bottom.**
///
/// `blend_canopy_normals` built its volume normal as `Vec3::new(x, 0.0, z)` —
/// no vertical component anywhere — and at `PINE_NORMAL_BLEND = 0.7` that
/// leaves at most 30 % of a card's own facing. A key at `ART.md` §4's 30–40°
/// then lands on every canopy vertex at nearly the same `N·L`, which is a flat
/// green cut-out by construction and cannot be fixed by any light rig.
///
/// The crown is an ellipsoid; its normal turns up at the apex and down under
/// the skirt. **Proven red by the accident of running it**: a mutant loop left
/// the old field in the tree and this gate was the only thing that said so,
/// at n.y = 0.062 against the 0.30 floor.
#[test]
fn the_canopy_has_a_top_and_an_underside() {
    for variant in [0usize, 3] {
        let (_, needles) = conifer(variant);
        let Some(VertexAttributeValues::Float32x3(p)) = needles.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions")
        };
        let Some(VertexAttributeValues::Float32x3(n)) = needles.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("no normals")
        };
        // The crown's highest tenth of VERTICES must face up on average and
        // its lowest tenth must face down — that is the whole of what was
        // missing. **By count, not by height**: a conifer is a cone, so the
        // top tenth of its HEIGHT holds seventeen vertices and a mean over
        // those is noise rather than a measurement.
        let mut by_y: Vec<usize> = (0..p.len()).collect();
        by_y.sort_by(|&a, &b| p[a][1].partial_cmp(&p[b][1]).expect("height is NaN"));
        let tenth = (p.len() / 10).max(20);
        let mean_y =
            |idx: &[usize]| -> f32 { idx.iter().map(|&i| n[i][1]).sum::<f32>() / idx.len() as f32 };
        let top = mean_y(&by_y[p.len() - tenth..]);
        let bottom = mean_y(&by_y[..tenth]);
        assert!(
            top >= 0.30,
            "variant {variant}: the crown's top decile averages n.y = {top:.3} \
             — a canopy apex faces the sky, and at the old purely-radial field \
             this reads 0.062 and the tree has no lit top"
        );
        assert!(
            bottom <= -0.10,
            "variant {variant}: the crown's bottom decile averages n.y = \
             {bottom:.3} — the underside is the face `ART.md` §5 says every \
             judge catches, and it cannot read if it faces sideways"
        );
    }
}

/// Mean linear luma of a mesh's vertex colours above `frac` of its own height.
fn upper_luma(m: &Mesh, frac: f32) -> f64 {
    let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("no positions")
    };
    let Some(VertexAttributeValues::Float32x4(c)) = m.attribute(Mesh::ATTRIBUTE_COLOR) else {
        panic!("no colours")
    };
    let hi = p.iter().fold(f32::MIN, |h, v| h.max(v[1]));
    let v: Vec<f64> = (0..p.len())
        .filter(|&i| p[i][1] >= hi * frac)
        .map(|i| 0.2126 * c[i][0] as f64 + 0.7152 * c[i][1] as f64 + 0.0722 * c[i][2] as f64)
        .collect();
    assert!(v.len() > 50, "only {} vertices above {frac}", v.len());
    v.iter().sum::<f64>() / v.len() as f64
}

/// **The hull and the tree it replaces must agree about how dark a canopy is.**
///
/// `impostor_of` rebuilds its colour from `SpeciesDef`'s ramps rather than from
/// the canopy's vertices, which is right — the bark half is mean-1 for a
/// photograph and copying it would give a white hull — and it means
/// `occlude_canopy` reached one side of the swap and not the other. Left alone
/// that is a tree that gets ~1.9× brighter the moment it crosses
/// `TREE_LOD_SWAP_M`: the colour pop [`BARK_BAND_TOP_FRAC`]'s own comment says
/// "no gate in this repo could see".
///
/// Now one can. `canopy_mean_shade` reads the shade off the canopy that ships,
/// so the hull follows it without a second number to keep in step. Proven red
/// by dropping the `canopy_k` multiply: 1.92× on the conifer, 1.90× on the
/// broadleaf, against the 1.25 allowed here.
#[test]
fn the_far_hull_wears_the_shade_of_the_canopy_it_replaces() {
    for variant in [0usize, 3] {
        let (bark, needles) = conifer(variant);
        let hull = impostor_of(&bark, &needles, variant);
        // The top quarter: all canopy on both meshes, so the hull's bark blend
        // is not in the comparison.
        let near = upper_luma(&needles, 0.75);
        let far = upper_luma(&hull, 0.75);
        let ratio = far / near;
        assert!(
            (0.80..=1.25).contains(&ratio),
            "variant {variant}: the far hull's canopy reads {far:.4} against the \
             near tree's {near:.4} — {ratio:.2}× — so a tree changes brightness \
             as it crosses TREE_LOD_SWAP_M"
        );
    }
}

/// **The crown's radius profile is per-HEIGHT, and until this gate that claim
/// was prose.**
///
/// `occlude_canopy` normalises a vertex's radius against `R(y)` — the crown's
/// own width at that height — and its doc comment says why in full: against one
/// global maximum instead, a cone's apex is narrow, so every vertex up there
/// reads buried and **the top of every conifer goes dark**, which is the
/// reverse of the truth and the most exposed part of the tree.
///
/// ⚠ **That paragraph was written, and the mutant walked straight through the
/// five gates beside it.** Swapping `radius_at(v[1])` for `crown_extent(p).2`
/// passed all twenty-one, because every one of them samples the middle of the
/// crown where the two agree. This is the trap `CLAUDE.md` names twice — a doc
/// reading as covered while nothing checks it — caught the only way it can be,
/// by running the mutant rather than by writing the comment more carefully.
///
/// The statistic is the mean shade `occlude_canopy` actually applied, read off
/// the mesh that ships (`tree::canopy_shade`) against `band`'s ramp, which is
/// not the thing under test. The apex is more exposed than mid-crown, so it
/// must come out no darker. Measured: **1.06 / 1.26** on the two species, and
/// **0.93 / 0.98** under the global-radius mutant — a sign flip, not a margin.
#[test]
fn the_crown_profile_follows_the_silhouette_and_not_one_radius() {
    for variant in [0usize, 3] {
        let apex = canopy_shade(variant, 0.85, 1.0);
        let mid = canopy_shade(variant, 0.40, 0.60);
        let ratio = apex / mid;
        assert!(
            ratio >= 1.02,
            "variant {variant}: the crown's apex carries {apex:.4} of its ramp \
             against mid-crown's {mid:.4} — {ratio:.3}× — so the top of the tree \
             is being treated as BURIED. `occlude_canopy` must normalise against \
             the crown's radius at each height, never against one global maximum"
        );
        // And the apex must not be crushed on its own account: a gamma that
        // buries the whole crown would keep the ratio and lose the tree.
        assert!(
            apex >= 0.45,
            "variant {variant}: the crown's apex carries only {apex:.4} of its \
             own colour ramp — `CANOPY_AO_GAMMA` has darkened the lit top of \
             the tree, which is not what the depth term is for"
        );
    }
}
