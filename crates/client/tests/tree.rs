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
    bounds, conifer, impostor_of, min_y, needle_image, species_of, tris, TreeLod, CONIFER_MAX_TRIS,
    CONIFER_POOL, IMPOSTOR_MAX_TRIS, SPECIES, TREE_LOD_FADE_M, TREE_LOD_SWAP_M, TREE_MAX_R,
};

/// Trees inside the client's 5×5×64 m prop ring, p90 over 100 eye positions
/// sampled across the island off `sim_core::terrain::scatter`. Recorded here
/// because the budget assertion below is meaningless without it.
const RING_TREES_P90: usize = 328;

/// …and how many of those are inside [`TREE_LOD_SWAP_M`], from the same
/// sweep. The band table in `the_far_band_is_an_impostor_and_the_ring_fits`
/// is the rest of it; this is the row the budget arithmetic uses, so it is a
/// constant rather than a number inside the test.
const NEAR_TREES_AT_SWAP: usize = 82;

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
    // vary per slot, but at the measured p90 of 328 trees in the draw ring one
    // silhouette repeated 328 times reads as one silhouette repeated.
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
/// exists.**
///
/// This test asserted the opposite for as long as the generator has shipped:
/// a full-detail conifer at the ring's p90 does not fit `DESIGN.md` §9's 1.5 M
/// and no parameter change makes it fit — measured, by distance band: 40 m
/// holds 21 trees (~89 k tris), 80 m holds 82 (~350 k), 120 m holds 168
/// (~709 k), 160 m holds 288 (~1.22 M). It printed the debt rather than
/// asserting it away, so it could not be forgotten by anyone reading a green
/// suite.
///
/// `tree::impostor_of` is the answer to it and the arithmetic below is the
/// whole claim: everything past [`TREE_LOD_SWAP_M`] costs a hull instead of a
/// tree. Both halves are asserted — the ring fits, AND it would not fit
/// without the swap — because a gate that only checks the good number stays
/// green if the impostor quietly becomes a copy of the tree.
#[test]
fn the_far_band_is_an_impostor_and_the_ring_fits() {
    // The band table above is measured AT 80 m. If the knob moves, the tree
    // count that goes with it has to be re-measured, not reused.
    assert!(
        (TREE_LOD_SWAP_M - 80.0).abs() < 1e-6,
        "TREE_LOD_SWAP_M is {TREE_LOD_SWAP_M} m and the p90 band table in this \
         gate was measured at 80 m — re-measure the tree count at the new \
         distance rather than reusing {NEAR_TREES_AT_SWAP}"
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
    let now = per_tree * NEAR_TREES_AT_SWAP + per_far * (RING_TREES_P90 - NEAR_TREES_AT_SWAP);
    println!(
        "conifer {per_tree} tris/tree · impostor {per_far} tris · ring at p90 \
         {RING_TREES_P90} trees: {was} tris full detail -> {now} with the \
         {TREE_LOD_SWAP_M} m swap ({:.1}x)",
        was as f32 / now.max(1) as f32
    );
    assert!(
        now < 1_500_000,
        "the ring costs {now} tris with the LOD in — past DESIGN.md §9's 1.5 M, \
         which is the ceiling this LOD exists to get under"
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
    // An `AlphaMode::Mask` material with a fully opaque map is an opaque quad,
    // which is the hull `props.js` spent three passes rejecting — and it would
    // look like a solid green square, not like an error. So: the map must have
    // real transparency, and it must have real coverage.
    let img = needle_image();
    let data = img.data.as_ref().expect("needle image has no data");
    let alphas: Vec<u8> = data.chunks_exact(4).map(|p| p[3]).collect();
    let opaque = alphas.iter().filter(|&&a| a > 128).count();
    let clear = alphas.iter().filter(|&&a| a < 16).count();
    let total = alphas.len();

    assert!(
        clear * 100 / total > 30,
        "only {}% of the needle card is transparent — that is a quad, not a sprig",
        clear * 100 / total
    );
    assert!(
        opaque * 100 / total > 5,
        "only {}% of the needle card is opaque — the canopy would be invisible",
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
