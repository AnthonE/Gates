//! The conifer, generated rather than authored.
//!
//! **This is the browser's decision, carried across the port.** `props.js`
//! says it in one line — *"a conifer's canopy is made of ALPHA CARDS, and an
//! opaque hull with a polygon edge cannot get there from any amount of
//! geometry"* — and it says so after three passes of building pines out of
//! cones. The native client shipped the cone stack because the port went slice
//! by slice, not because the question reopened. `props.rs`'s whorl builder is
//! still there and still correct, and it is still what a billboard bake would
//! render from — but the far LOD is [`impostor_of`] now, lathed through this
//! generator's own output rather than authored beside it.
//!
//! The generator is `bevy_procedural_tree` (MIT OR Apache-2.0), which is
//! `@dgreenheck/ez-tree`'s algorithm ported to Rust — the same generator the
//! browser already depends on. So `PINE_EZ`'s swept parameters are evidence
//! about *this* code, not a different one, and the numbers below are read
//! against it rather than re-guessed.
//!
//! **One function of it is used**, `meshgen::generate_tree_meshes`: settings
//! and an `Rng` in, two `Mesh`es out, no ECS anywhere. Its
//! `TreeProceduralGenerationPlugin` is deliberately not touched — a plugin
//! that spawns entities and regenerates them when settings change would put
//! tree state in the ECS, and `RENDER.md` §1 is that Bevy draws and does not
//! decide.
//!
//! ## What this module adds that the generator does not
//!
//! Three things, all of them post-passes over a returned `Mesh`, which is why
//! the crate is a dependency rather than a fork:
//!
//!   1. **Fit to the sim's bounds.** The generator has no idea what
//!      `PINE_MAX_R` is. Height is normalised to `PINE_H` and the canopy is
//!      measured against the ceiling `world.rs` derives `SPAWN_CLEAR_M` from.
//!   2. **Vertex colours.** The crate emits none — its `branches_colors` is
//!      commented out in its own source — and `ART.md` §5's "two greens
//!      minimum" is the canopy's whole read. The bands are `props.rs`'s, so
//!      the generated tree wears the same colours the whorl one did.
//!   3. **Canopy normals blended toward the trunk axis.** Leaf cards come out
//!      carrying their own card normal, so a canopy lit by one sun is a pile
//!      of flat plates at a dozen brightnesses. This is exactly the defect
//!      `PINE_NORMAL_BLEND` was introduced for on the whorls, and the fix
//!      transfers: pull each needle normal toward "away from the trunk axis",
//!      which is what a needle mass actually scatters like.
//!
//! Not here, and owed: `aWind`. The browser bakes a per-vertex cantilever
//! weight and `StandardMaterial` cannot read a custom attribute, so wind needs
//! the custom material that `RENDER.md` already lists — a slice, not a knob.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::image::Image;
use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_procedural_tree::enums::{LeafBillboard, TreeType};
use bevy_procedural_tree::meshgen::generate_tree_meshes;
use bevy_procedural_tree::settings::{
    BranchForce, BranchParams, BranchRecursionLevel, LeafParams, TreeMeshSettings,
};

use super::props::{hash2, Soup, PINE_H, PINE_MAX_R, PINE_NORMAL_BLEND};

/// Distinct generated conifers. Each is a mesh PAIR and therefore two draw
/// calls, so this is spent against `DESIGN.md` §9's 300 — three is six.
///
/// **Three, where the browser shipped one.** `props.js` states the reason it
/// stopped at one and it is not a design argument: *"400 copies of one
/// silhouette will read as 400 copies eventually, and the second variant is
/// four draw calls rather than a design problem."* A generated conifer is not
/// rotationally symmetric the way a cone is, so yaw already buys variation the
/// old pool could not — but `ART.md` rule 7 forbids two identical instances
/// adjacent, and at the measured p90 of 328 trees in the draw ring one
/// silhouette is not enough to honour it.
///
/// **Deliberately NOT named `PINE_VARIANTS`.** That is a registered knob
/// (`DECISIONS.md` §open) pinned to `props.js` at 1, and `ci/knob_registry.mjs`
/// goes red on the collision — the same trap `PINE_MESH_POOL` already carries
/// a comment about. One registry name cannot mean two things.
pub const CONIFER_POOL: usize = SEEDS_PER_SPECIES * SPECIES.len();

/// Distinct seeds generated per species. Three was the whole pool when the
/// pool was one species; it is now the per-species figure, so the pool grows
/// with the table rather than being restated.
pub const SEEDS_PER_SPECIES: usize = 3;

/// What separates one species from another, beyond its parameter block: how
/// tall `fit_to_bounds` normalises it to, how wide it is allowed to get, and
/// which two colours its canopy bands between.
///
/// **Height and radius are here rather than in `props.rs` because they are
/// now per-species and `PINE_H`/`PINE_MAX_R` are not.** Those two stay as the
/// conifer's own numbers — `props.rs`'s whorl builder still reads them for the
/// far-LOD silhouette — and [`TREE_MAX_R`] is the island-wide ceiling the sim
/// has to clear.
pub struct SpeciesDef {
    pub tree_type: TreeType,
    pub height_m: f32,
    /// The radius no part of this species may exceed, metres. A CEILING that
    /// `tests/tree.rs` measures every seed against, not what it draws.
    pub max_r_m: f32,
    pub leaf_lo: u32,
    pub leaf_hi: u32,
}

/// The species pool. **Two, where there was one** — `reference/PLANTS.md` §6.1
/// and `NOW.md` §0t item 1.
///
/// The broadleaf is not a taste addition. `TreeType::Deciduous` was a variant
/// of a crate enum we already depended on and never used, and the file's own
/// comment said the pool was "one species at three seeds rather than three
/// species". A temperate island with exactly one tree on it is the single
/// cheapest thing to fix about this forest.
pub const SPECIES: [SpeciesDef; 2] = [
    SpeciesDef {
        tree_type: TreeType::Evergreen,
        height_m: PINE_H,
        max_r_m: PINE_MAX_R,
        leaf_lo: NEEDLE_LO,
        leaf_hi: NEEDLE_HI,
    },
    // **Shorter than the conifer and much wider**, which is the whole read: a
    // pine is a spire and a broadleaf is a dome, and if the two shared a
    // silhouette envelope there would be no point having both. 5.4 m against
    // 6.6 keeps the conifer as the thing that breaks the skyline.
    SpeciesDef {
        tree_type: TreeType::Deciduous,
        height_m: 5.4,
        max_r_m: BROADLEAF_MAX_R,
        leaf_lo: BROADLEAF_LO,
        leaf_hi: BROADLEAF_HI,
    },
];

/// The broadleaf's radius ceiling, metres. Wider than the conifer's by design;
/// see [`TREE_MAX_R`] for what it costs the sim.
pub const BROADLEAF_MAX_R: f32 = 2.9;

/// The widest any species may be, metres — the number the sim's spawn
/// clearance has to cover.
///
/// **This is the constant `world.rs` was always talking about and never had.**
/// `SPAWN_CLEAR_M`'s comment derived itself from `PINE_MAX_R` and credited
/// `ci/pine_shape.mjs` with closing the arithmetic; that gate does not exist
/// (it went with the browser), so the derivation was a dead citation — a claim
/// that something was enforced when nothing was. `tests/tree.rs` closes it in
/// Rust now, against this.
pub const TREE_MAX_R: f32 = if PINE_MAX_R > BROADLEAF_MAX_R {
    PINE_MAX_R
} else {
    BROADLEAF_MAX_R
};

/// Which species a pool index is, and which seed within it. Seeds are grouped
/// by species so a species can be appended without renumbering the ones
/// before it — `Fellable::variant` is stored on live entities and a
/// renumbering would silently re-species every standing tree.
pub fn species_of(variant: usize) -> usize {
    (variant / SEEDS_PER_SPECIES).min(SPECIES.len() - 1)
}

/// Ceiling on one conifer's triangles, bark and needles together.
///
/// Measured rather than chosen. At the client's 5×5×64 m prop ring the p90 is
/// 328 trees and the max 446; `DESIGN.md` §9 allows 1.5 M triangles for the
/// whole frame and the terrain LOD already spends ~250 k. 6,000 × 328 is
/// 1.97 M, which does not fit — **and that is the point of this constant
/// being here rather than in a comment.** Full-detail trees are affordable to
/// roughly 80–100 m (p90 82 trees, ~350 k tris) and past that the draw has to
/// stop being a tree.
///
/// **It does now**, and this ceiling is what it is measured against rather
/// than what it used to be a warning about: past [`TREE_LOD_SWAP_M`] a tree is
/// [`impostor_of`]'s 105-triangle hull, so the same p90 ring is 1.94 M → 510 k
/// and `tests/tree.rs` asserts the fit instead of printing the debt. The
/// billboard `TERRAIN.md` §4 queues is still unbuilt and is still the cheaper
/// end of this; the hull is the step that needed no new material and no bake.
pub const CONIFER_MAX_TRIS: usize = 6_000;

/// `ART.md` §5's two greens, and the trunk. Same constants `props.rs` bands
/// the whorl pine with, so swapping the builder does not swap the palette.
const BARK_LO: u32 = 0x503b2b;
const BARK_HI: u32 = 0x70583f;
const NEEDLE_LO: u32 = 0x204825;
const NEEDLE_HI: u32 = 0x4d8845;
/// The broadleaf's two greens. Yellower and lighter than the conifer's, which
/// is the other half of telling the two apart at range — `ART.md` §5 asks for
/// two greens minimum per canopy, and a second SPECIES wearing the first one's
/// palette would read as the same tree at a different size.
const BROADLEAF_LO: u32 = 0x3a5a22;
const BROADLEAF_HI: u32 = 0x7fa03c;

/// Where the bark's `BARK_LO`→`BARK_HI` ramp tops out, as a fraction of the
/// tree's height, and where the canopy's leaf ramp starts.
///
/// **Named rather than written twice.** They were two literals inside
/// [`conifer`]'s two `band` calls until the far LOD needed the same ramps to
/// colour a hull that spans both — and a silhouette ramping over a different
/// band than the tree it replaces is a colour pop at the swap distance that
/// no gate in this repo could see.
const BARK_BAND_TOP_FRAC: f32 = 0.6;
/// See [`BARK_BAND_TOP_FRAC`].
const LEAF_BAND_BASE_FRAC: f32 = 0.15;

/// sRGB hex to linear, the same conversion `props.rs` uses.
fn linear(hex: u32) -> [f32; 3] {
    let f = |b: u32| {
        let s = b as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f((hex >> 16) & 0xff), f((hex >> 8) & 0xff), f(hex & 0xff)]
}

/// One conifer's settings. `variant` only reseeds; the shape parameters are
/// shared so the pool is one species at three seeds rather than three species.
fn conifer_settings() -> TreeMeshSettings {
    TreeMeshSettings {
        tree_type: TreeType::Evergreen,
        branch: BranchParams {
            // ONE level, and this is the load-bearing choice in the file.
            // Leaves attach only to the LAST branch level, so at two levels
            // the needle mass lands on twigs and every limb reads bare — the
            // "dead sticks" result `props.js` measured when it cut branches to
            // save triangles.
            levels: BranchRecursionLevel::One,
            // Just past horizontal. Droop is the ANGLE's job here and NOT the
            // branch force's — see `force` below.
            angle: [0.0, 96.0, 0.0, 0.0],
            children: [60, 0, 0],
            // **The force points UP, and must.** A straight-down direction is
            // the documented way to get a willow, and it is a trap: the crate
            // builds one global `Quat::from_rotation_arc(Vec3::Y, dir)` and
            // slerps every section toward it, so at `dir = -Y` it hits the
            // antipodal singularity — glam returns 180° about an ARBITRARY
            // perpendicular axis, the same one for every branch, and the whole
            // tree bends sideways into a banana. Measured: five candidate
            // parameter sets, all five bent, `max_r` pinned at ~2.8 m
            // regardless of branch length. Reported upstream.
            force: BranchForce {
                direction: Vec3::Y,
                strength: 0.05,
                radius_cutoff: 0.1,
            },
            gnarliness: [0.01, 0.06, 0.0, 0.0],
            // Trunk length is normalised away by `fit_to_bounds`, so [0] is
            // only the shape's proportion, not the tree's height.
            //
            // **[1] is the canopy's WIDTH, and width is the sim's business.**
            // Swept against the real seeds after the bounds gate caught 1.717 m
            // at 1.75 — the fitted radius is not one number, it is a
            // distribution over seeds, and a value chosen on one seed is a
            // ceiling violation waiting for the next variant. Over eleven
            // seeds: 1.75 → 1.717 (OVER), 1.65 → 1.632, 1.55 → 1.508,
            // 1.45 → 1.441, and this one → 1.464 against the 1.7 ceiling —
            // ~14% margin, the same margin `props.js` took when it swept the
            // browser's copy and for the identical stated reason: "margin for
            // seed variance, which a generator needs and a hand-authored cone
            // did not."
            //
            // **Short limbs are paired with BIG leaf cards, and that pairing
            // is the whole point.** The first parameter set put 11 cards of
            // 0.18 m on 1.45 m limbs, which measured fine and rendered as a
            // spindly stick — because a card's contribution is its opaque
            // AREA, and the needle mask cuts ~60% of every card away. Sized
            // against opaque quads in a probe, the canopy was dense; sized
            // against the mask it was not, and only the frame said so.
            // Coverage (card area × count ÷ frontal silhouette) went 1.20 at
            // 0.18/11 to 16.0 here — and radius, which is the sim's business,
            // barely moved because shorter limbs paid for the larger cards.
            length: [6.6, 1.05, 0.0, 0.0],
            trunk_base_radius: 0.20,
            radius_factor: [1.0, 0.13, 0.0, 0.0],
            sections: [10, 4, 0, 0],
            segments: [7, 4, 0, 0],
            // Limbs start at 10% of the trunk, not the preset's 27%. On a
            // 6.6 m tree 27% is 1.8 m — above eye level, so a player walking
            // the forest sees a colonnade of poles. `props.js` measured the
            // same thing and moved it for the same reason.
            start: [0.0, 0.10, 0.0, 0.0],
            taper: [0.92, 0.90, 0.0, 0.0],
            twist: [0.02, 0.0, 0.0, 0.0],
        },
        leaves: LeafParams {
            // Crossed cards, not single: a flat card edge-on disappears, and a
            // canopy that thins as the camera orbits is the tell.
            leaf_billboard: LeafBillboard::Double,
            angle: 62.0,
            // Card SIZE dominates coverage (it is squared) and card COUNT
            // fills the envelope those cards span. Both push radius, which is
            // why the limbs above had to shorten to pay for them — see there.
            // 0.55 m is a real branchlet's size against a 6.6 m tree, which is
            // what the mask draws: a sprig cluster, not one needle.
            count: 16,
            start: 0.0,
            size: 0.55,
            size_variance: 0.4,
        },
    }
}

/// The broadleaf's parameters.
///
/// **Started from the crate's own `Deciduous` defaults rather than invented**,
/// which are ez-tree's baseline for the same species family; what moved from
/// them is listed here and nothing else did, so the diff against upstream is
/// readable.
///
/// - `levels: Two`, not the default `Three`. Leaves attach only to the LAST
///   level (the conifer's block explains why that matters), and three levels
///   of `children: [7, 4, 10]` is 280 terminal branches before a single leaf
///   card — comfortably past `CONIFER_MAX_TRIS` on branch geometry alone.
///   Two levels puts the canopy on 40 limbs and leaves the budget for cards.
/// - `children: [6, 7, 0]` against the default `[7, 4, 10]`: fewer primaries
///   and more secondaries, because with the third level gone the second one
///   has to carry the crown's whole spread.
/// - `angle[1] = 52°`, wider than the default 39°. A broadleaf's read is that
///   its limbs leave the trunk closer to horizontal than a conifer's; at the
///   default the crown is a narrow vase.
/// - `force.direction` is `Vec3::Y` and **must be**, for exactly the reason
///   the conifer's block gives at length: a downward direction hits the
///   antipodal singularity in `Quat::from_rotation_arc` and bends the whole
///   tree sideways. Droop is the limb ANGLE's job in both species.
/// - Bigger cards (0.42 m) and more of them (11) than the crate default's
///   0.25/3, on the conifer's own measured reasoning: a card's contribution is
///   its OPAQUE area and the mask cuts most of each card away, so a canopy
///   sized against solid quads comes out spindly.
fn broadleaf_settings() -> TreeMeshSettings {
    TreeMeshSettings {
        tree_type: TreeType::Deciduous,
        branch: BranchParams {
            levels: BranchRecursionLevel::Two,
            angle: [0.0, 52.0, 44.0, 0.0],
            children: [6, 7, 0],
            force: BranchForce {
                direction: Vec3::Y,
                strength: 0.04,
                radius_cutoff: 0.1,
            },
            gnarliness: [-0.04, 0.14, 0.10, 0.0],
            // [0] is proportion only — `fit_to_bounds` normalises height away.
            // [1] and [2] are what set the crown's WIDTH, which is the sim's
            // business through `BROADLEAF_MAX_R`; swept against the gate.
            length: [4.5, 1.75, 1.0, 0.0],
            trunk_base_radius: 0.22,
            radius_factor: [1.0, 0.42, 0.34, 0.0],
            sections: [10, 5, 3, 0],
            segments: [7, 5, 3, 0],
            // Limbs from 22% of the trunk, lower than the crate's 32%: on a
            // 5.4 m tree 32% is 1.7 m, which is head height, and a forest of
            // bare poles at eye level is what the conifer's block calls a
            // colonnade.
            start: [0.0, 0.22, 0.3, 0.0],
            taper: [0.94, 0.82, 0.85, 0.0],
            twist: [0.06, -0.05, 0.0, 0.0],
        },
        leaves: LeafParams {
            leaf_billboard: LeafBillboard::Double,
            angle: 48.0,
            count: 11,
            start: 0.0,
            size: 0.42,
            size_variance: 0.35,
        },
    }
}

/// The parameter block for a species index.
fn settings(species: usize) -> TreeMeshSettings {
    match SPECIES[species].tree_type {
        TreeType::Evergreen => conifer_settings(),
        TreeType::Deciduous => broadleaf_settings(),
    }
}

/// Every position in a mesh, as a mutable slice. Returns `None` rather than
/// panicking so a generator change cannot take the client down at boot.
fn positions_mut(m: &mut Mesh) -> Option<&mut Vec<[f32; 3]>> {
    match m.attribute_mut(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(v) => Some(v),
        _ => None,
    }
}

fn positions(m: &Mesh) -> Option<&Vec<[f32; 3]>> {
    match m.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(v) => Some(v),
        _ => None,
    }
}

/// Height and the largest horizontal radius, over any number of meshes. This
/// is the measurement `PINE_MAX_R` is a ceiling on.
pub fn bounds(meshes: &[&Mesh]) -> (f32, f32) {
    let (mut h, mut r) = (0.0f32, 0.0f32);
    for m in meshes {
        let Some(p) = positions(m) else { continue };
        for v in p {
            h = h.max(v[1]);
            r = r.max((v[0] * v[0] + v[2] * v[2]).sqrt());
        }
    }
    (h, r)
}

/// Scale the pair so the tree is exactly `PINE_H` tall and its base sits at
/// y = 0, the convention every other prop mesh in `props.rs` follows.
///
/// **Height is a parameter and width is a consequence** — the browser's
/// framing, and it is why this normalises rather than trusting `length[0]`.
/// The trunk gnarls and the top ring tapers to a tip, so the generated height
/// is never the requested length; measured at 7.34 m for a 7.2 m trunk.
fn fit_to_bounds(bark: &mut Mesh, needles: &mut Mesh, height_m: f32) {
    let (h, _) = bounds(&[bark, needles]);
    if h <= f32::EPSILON {
        return;
    }
    let k = height_m / h;
    // The generator roots the trunk at the origin, so a scale about the origin
    // keeps the base planted and no translate is owed. Measured, not assumed:
    // `tests/tree.rs` asserts the minimum y is 0 after this runs.
    for m in [bark, needles] {
        let Some(p) = positions_mut(m) else { continue };
        for v in p.iter_mut() {
            v[0] *= k;
            v[1] *= k;
            v[2] *= k;
        }
    }
}

/// Paint a height ramp between two sRGB colours onto a mesh's vertices.
/// Ramp a mesh's vertex colours `lo`→`hi` over `y0..y1`.
///
/// `mean1` normalizes each result to unit luminance, which is what a surface
/// that now wears a PHOTOGRAPH needs: the bark map carries the colour and this
/// band keeps only the light-to-dark ramp up the trunk, per `ART.md` §7. The
/// needles pass `false` — their map is a generated white alpha mask, so their
/// vertex colour is still the only colour they have.
fn band(m: &mut Mesh, lo: u32, hi: u32, y0: f32, y1: f32, mean1: bool) {
    let Some(p) = positions(m) else { return };
    let (l, g) = (linear(lo), linear(hi));
    let span = (y1 - y0).max(f32::EPSILON);
    let cols: Vec<[f32; 4]> = p
        .iter()
        .map(|v| {
            let t = ((v[1] - y0) / span).clamp(0.0, 1.0);
            let c = [
                l[0] + (g[0] - l[0]) * t,
                l[1] + (g[1] - l[1]) * t,
                l[2] + (g[2] - l[2]) * t,
            ];
            if !mean1 {
                return [c[0], c[1], c[2], 1.0];
            }
            let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            if luma <= 1e-6 {
                [1.0, 1.0, 1.0, 1.0]
            } else {
                [c[0] / luma, c[1] / luma, c[2] / luma, 1.0]
            }
        })
        .collect();
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, cols);
}

/// Pull every needle normal away from the trunk axis by `PINE_NORMAL_BLEND`.
///
/// A leaf card's own normal is the card's facing, so a canopy of a few hundred
/// cards lit by one directional sun resolves into a few hundred flat plates —
/// the "asset-pack tree" signature `props.rs` names, and the face `ART.md` §5
/// says every judge catches. A needle mass does not have facets; it scatters
/// as a volume, and "outward from the trunk" is that volume's normal field.
fn blend_canopy_normals(m: &mut Mesh) {
    let Some(p) = positions(m) else { return };
    let outward: Vec<[f32; 3]> = p
        .iter()
        .map(|v| {
            let radial = Vec3::new(v[0], 0.0, v[2]);
            // On the axis there is no outward direction; up is the only
            // defensible answer and it affects a handful of vertices.
            radial.normalize_or(Vec3::Y).to_array()
        })
        .collect();
    let Some(VertexAttributeValues::Float32x3(n)) = m.attribute_mut(Mesh::ATTRIBUTE_NORMAL) else {
        return;
    };
    for (i, v) in n.iter_mut().enumerate() {
        let facet = Vec3::from_array(*v);
        let vol = Vec3::from_array(outward[i]);
        *v = (facet * (1.0 - PINE_NORMAL_BLEND) + vol * PINE_NORMAL_BLEND)
            .normalize_or(facet)
            .to_array();
    }
}

/// One conifer: `(bark, needles)`, fitted, banded and shaded.
///
/// Deterministic in `variant` and nothing else — same variant, same tree, on
/// every client and every run. That is what lets a chunk stream out and back
/// bit-identical, the same law the whorl builder's hashes carried.
pub fn conifer(variant: usize) -> (Mesh, Mesh) {
    let sp = &SPECIES[species_of(variant)];
    // Seeded off the same mixer the rest of `props.rs` uses rather than the
    // raw index, so variant 0 and variant 1 are not neighbouring PRNG streams.
    let mut rng = fastrand::Rng::with_seed(hash2(0x9e37_79b9, variant as u32) as u64);
    let (mut bark, mut needles) =
        match generate_tree_meshes(&settings(species_of(variant)), &mut rng) {
            Ok(pair) => pair,
            // A generator failure must not be a black screen. The only
            // documented error is index overflow, which `u32_indices` makes
            // unreachable at these counts — but "unreachable" is not
            // "impossible", and an empty pair draws nothing rather than
            // panicking a client at boot.
            Err(_) => (Mesh::from(Cuboid::default()), Mesh::from(Cuboid::default())),
        };

    fit_to_bounds(&mut bark, &mut needles, sp.height_m);
    band(
        &mut bark,
        BARK_LO,
        BARK_HI,
        0.0,
        sp.height_m * BARK_BAND_TOP_FRAC,
        true,
    );
    band(
        &mut needles,
        sp.leaf_lo,
        sp.leaf_hi,
        sp.height_m * LEAF_BAND_BASE_FRAC,
        sp.height_m,
        false,
    );
    blend_canopy_normals(&mut needles);
    (bark, needles)
}

/// Needle-sprig size in texels. Two of these tile the leaf card's UV range.
const NEEDLE_TEX: u32 = 64;

/// The alpha card the canopy is actually made of — **generated at boot, not
/// shipped.**
///
/// `assets/textures/` has bark and no leaf, and the Rust generator ships no
/// textures at all (the JS one builds its own and hands over an `alphaTest`).
/// A leaf quad with no alpha is a solid square, and a canopy of solid squares
/// is the opaque hull the browser already rejected — so without this the
/// generated tree is a *downgrade* on the whorl cone, not an upgrade.
///
/// Generating it follows `sky.rs`, which builds its cloud cubemap the same way
/// and for the same reason: no asset, no download, no licence to carry, and it
/// cannot go missing from a depot. A sprig is a cheap thing to draw — a stem
/// with needles fanning off it — and alpha is the only channel that has to be
/// right, because `AlphaMode::Mask` reads nothing else.
pub fn needle_image() -> Image {
    let n = NEEDLE_TEX as usize;
    let mut data = vec![0u8; n * n * 4];
    let half = NEEDLE_TEX as f32 * 0.5;

    // 2 sprigs, mirrored about the card's centre line, so a `Double` billboard
    // pair does not show the same silhouette twice from every angle.
    for (sx, dir) in [(0.30f32, 1.0f32), (0.70, -1.0)] {
        let stem_x = sx * NEEDLE_TEX as f32;
        // ~34 needles up the stem, alternating sides, shortening toward the tip.
        for i in 0..34u32 {
            let t = i as f32 / 33.0;
            let y = t * (NEEDLE_TEX as f32 - 2.0) + 1.0;
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            // Needles are longest at the base of the sprig and taper to the
            // tip — the same silhouette rule the whorls encode as a radius ramp.
            let len = (1.0 - t * 0.65) * half * 0.72;
            let sweep = 0.55 + 0.25 * t; // needles sweep upward toward the tip
            let steps = len.ceil() as u32;
            for s in 0..=steps {
                let u = s as f32 / steps.max(1) as f32;
                let px = stem_x + side * dir * u * len;
                let py = y + u * len * sweep;
                // A needle thins along its length; below ~0.35 texels it is
                // aliasing rather than a needle, so it stops there.
                let w = (1.0 - u) * 1.15 + 0.35;
                stamp(&mut data, n, px, py, w);
            }
        }
    }

    // Levels 1..n, coverage-preserved. See [`needle_mips`] for why a plain box
    // filter is the wrong tool for an alpha-tested mask.
    let levels = needle_mips(&data, NEEDLE_TEX);
    let mip_level_count = levels.len() as u32;
    let chained: Vec<u8> = levels.concat();

    // **Constructed at level 0, then given the chain.** `Image::new` asserts
    // `data.len() == width · height · block_size` — it describes one mip and
    // has no parameter for a chain — so handing it the concatenated buffer
    // panics inside the constructor rather than failing a gate. The descriptor
    // and the buffer are both plain fields, so the chain is installed after.
    let mut img = Image::new(
        Extent3d {
            width: NEEDLE_TEX,
            height: NEEDLE_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        // The needle's COLOUR comes from the mesh's vertex bands, so this map
        // is a mask that happens to be white. sRGB anyway: it is multiplied
        // into base colour and a linear white is still white.
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(chained);
    img.texture_descriptor.mip_level_count = mip_level_count;
    // Trilinear across the chain, tiling off: the card's UVs are 0..1 and a
    // wrapped needle mask bleeds the far edge into the near.
    //
    // ⚠ This comment used to justify the `mipmap_filter` line by saying
    // `ImageSampler::linear()` leaves it at Nearest. **It does not, in Bevy
    // 0.18.1**: `ImageSamplerDescriptor::linear()` sets all three filters to
    // Linear and it is bare `default()` that leaves `mipmap_filter` at Nearest
    // (`bevy_image/src/image.rs`, the `linear()` and `Default` impls). Spelling
    // the three out stays right — it is explicit and it survives the next
    // upgrade — but the reason was wrong, and it was wrong in the one place
    // somebody checking our mip filtering would read first.
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    img
}

/// Alpha cutoff the canopy's `AlphaMode::Mask` tests against, as a byte.
///
/// The cutoff itself is authored in `props.rs`'s foliage material
/// (`AlphaMode::Mask(0.5)`) and this is the same number as a byte, so the two
/// can drift; `tests/tree.rs::the_needle_chain_holds_its_coverage` pins them
/// together. Alpha is linear even in an sRGB-encoded texture — only RGB carries
/// the transfer function — so 0.5 is 128 and not 188.
const NEEDLE_MASK_BYTE: u8 = 128;

/// The full mip chain for the needle mask, level 0 first, **coverage-preserved**.
///
/// **Why this is not `image::imageops::resize` or a plain box filter.** The
/// canopy is `AlphaMode::Mask(0.5)`, so what reaches the frame is not the
/// filtered alpha — it is the *fraction of texels that survive a threshold*.
/// Box-filtering a sparse mask drives every texel toward the mask's mean, and
/// the mean of a needle sprig is well under 0.5, so each level loses coverage
/// against the one above it. Measured with the rescale pinned at 1.0: **level 1
/// alone keeps 0.53× of level 0's coverage** (0.102 against 0.192), one halving
/// from full detail, and it compounds down the chain. That reads as a thinning
/// forest rather than as a filtering artefact, so nobody looks for it in a
/// texture — `tests/tree.rs::the_needle_chain_holds_its_coverage` is where the
/// number comes from and is red under exactly that mutant.
///
/// The fix is Castaño's: after downsampling, scale the level's alpha so the
/// share of texels above the cutoff matches level 0's. A bisection on the scale
/// is enough — coverage is monotonic in it — and 12 steps resolves the scale to
/// better than one part in 4,000 of the search span, which is finer than the
/// 1/255 the channel can store anyway.
///
/// **This is the whole of the shimmer fix, and the shimmer is why it matters
/// more than the baldness.** The map shipped with `mip_level_count` at 1, so a
/// canopy 60 m out sampled a 64² needle mask at roughly one texel per several
/// pixels with no minification filtering at all — every frame the camera moved,
/// a different set of needles won the sample. `CLAUDE.md`'s "median fps hides
/// shader-compile stalls" entry has the general shape of this: a still frame
/// cannot see it, and every frame this project has ever judged was a still.
fn needle_mips(level0: &[u8], size: u32) -> Vec<Vec<u8>> {
    let coverage = |px: &[u8]| -> f32 {
        let hit = px
            .chunks_exact(4)
            .filter(|p| p[3] > NEEDLE_MASK_BYTE)
            .count();
        hit as f32 / (px.len() / 4) as f32
    };
    let want = coverage(level0);

    let mut out = vec![level0.to_vec()];
    let mut w = size;
    while w > 1 {
        let prev = out.last().expect("out is seeded with level 0");
        let half = w / 2;
        let mut next = vec![0u8; (half * half) as usize * 4];
        for y in 0..half as usize {
            for x in 0..half as usize {
                // Box of four. RGB is a constant white across the whole map, so
                // only alpha carries anything and the average is exact.
                let mut acc = 0u32;
                for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let sy = y * 2 + dy;
                    let sx = x * 2 + dx;
                    acc += u32::from(prev[(sy * w as usize + sx) * 4 + 3]);
                }
                let i = (y * half as usize + x) * 4;
                next[i] = 255;
                next[i + 1] = 255;
                next[i + 2] = 255;
                next[i + 3] = (acc / 4) as u8;
            }
        }

        // Bisect the alpha scale until this level tests to level 0's coverage.
        // The upper bound is 8: past that the scale is pushing near-empty texels
        // over the cutoff, which invents needles rather than preserving them,
        // and the bottom levels are a handful of texels where exact coverage is
        // unreachable at any scale.
        let (mut lo, mut hi) = (1.0f32, 8.0f32);
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            let scaled: Vec<u8> = next
                .chunks_exact(4)
                .flat_map(|p| {
                    let a = (f32::from(p[3]) * mid).min(255.0) as u8;
                    [255, 255, 255, a]
                })
                .collect();
            if coverage(&scaled) < want {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let s = 0.5 * (lo + hi);
        for p in next.chunks_exact_mut(4) {
            p[3] = (f32::from(p[3]) * s).min(255.0) as u8;
        }

        out.push(next);
        w = half;
    }
    out
}

/// Paint one soft dot of needle into the RGBA buffer, alpha-max blended.
///
/// Max rather than add: two needles crossing must not read brighter than one,
/// because the channel is coverage, not light.
fn stamp(data: &mut [u8], n: usize, px: f32, py: f32, w: f32) {
    let r = w.ceil() as i32;
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = (px as i32 + dx, py as i32 + dy);
            if x < 0 || y < 0 || x >= n as i32 || y >= n as i32 {
                continue;
            }
            let d = (((px - x as f32).powi(2)) + ((py - y as f32).powi(2))).sqrt();
            let a = (1.0 - (d / w).clamp(0.0, 1.0)).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let i = (y as usize * n + x as usize) * 4;
            let cur = data[i + 3];
            let new = (a * 255.0) as u8;
            if new > cur {
                data[i] = 255;
                data[i + 1] = 255;
                data[i + 2] = 255;
                data[i + 3] = new;
            }
        }
    }
}

/// Triangle count of a mesh, indices or not.
pub fn tris(m: &Mesh) -> usize {
    match m.indices() {
        Some(Indices::U16(v)) => v.len() / 3,
        Some(Indices::U32(v)) => v.len() / 3,
        None => positions(m).map_or(0, |p| p.len()) / 3,
    }
}

/// The lowest y in a pair — asserted to be 0 by the bounds gate, because a
/// tree floating above or sunk below its slot is invisible in a screenshot and
/// obvious in play.
pub fn min_y(meshes: &[&Mesh]) -> f32 {
    let mut lo = f32::MAX;
    for m in meshes {
        let Some(p) = positions(m) else { continue };
        for v in p {
            lo = lo.min(v[1]);
        }
    }
    if lo == f32::MAX {
        0.0
    } else {
        lo
    }
}

/// How high up the bark mesh [`trunk_radius`] measures, metres.
///
/// **Chosen so the measurement is unambiguous, not to be generous.** Above
/// this the bark mesh is trunk AND limbs, and a radial max over both is a
/// number about branches — measured at 0.86 m on a pine and 2.38 m on a
/// broadleaf inside the capsule's own height band. Below it every species is
/// a single tapering column: the generator starts limbs at 10 % of the pine's
/// trunk (0.66 m) and 22 % of the broadleaf's (1.19 m), both comfortably
/// clear. `tests/tree.rs` asserts the result stays trunk-scale, so a future
/// species that puts a limb on the ground fails loudly rather than quietly
/// inflating the cylinder the sim blocks with.
pub const TRUNK_MEASURE_H_M: f32 = 0.25;

/// The drawn trunk's radius at its base, metres, at a slot scale of 1.0 —
/// the number `terrain::OCCUPANT_R_M`'s `Tree` row has to be.
///
/// The base, because a trunk tapers upward and this is therefore its widest
/// point inside the band a player's capsule occupies. Erring outward is the
/// convention `SHELTER_CORNER_R_M` states and the reason the row is rounded
/// up from this: a wasted narrow-phase test costs a compare, while erring
/// inward lets a body stand inside visible bark.
pub fn trunk_radius(bark: &Mesh) -> f32 {
    let mut r = 0.0f32;
    let Some(p) = positions(bark) else { return r };
    for v in p {
        if v[1] <= TRUNK_MEASURE_H_M {
            r = r.max((v[0] * v[0] + v[2] * v[2]).sqrt());
        }
    }
    r
}

/// Does the pair fit the volume the sim blocks? `PINE_MAX_R` is not a
/// rendering number: `world.rs` derives `SPAWN_CLEAR_M = 4.0` from it, so a
/// canopy that grew past it puts fresh spawns inside trees.
pub fn fits_sim_bounds(bark: &Mesh, needles: &Mesh) -> bool {
    let (_, r) = bounds(&[bark, needles]);
    r <= PINE_MAX_R
}

// ── The far LOD ─────────────────────────────────────────────────────────────
//
// `TERRAIN.md` §4 queues a billboard and this is not it. It is the step
// before: one opaque hull per variant, lathed through the tree's OWN vertices,
// swapped in by `VisibilityRange` past [`TREE_LOD_SWAP_M`]. Two reasons it is
// worth landing first.
//
// **The arithmetic.** `tests/tree.rs` has printed the debt since the generator
// landed: 5,900 triangles a tree — [`CONIFER_MAX_TRIS`] is the 6 k ceiling,
// this is what the generator actually emits — at the ring's p90 of 328 is
// 1.94 M against `DESIGN.md` §9's 1.5 M for the whole frame. The band table in
// that suite says 82 of those trees are inside 80 m; the other ~246 are the
// ones paying full price for a silhouette the atmosphere is already eating.
//
// **The passes.** A tree is not drawn once. SSAO carries
// `#[require(DepthPrepass, NormalPrepass)]`, so the same geometry is
// rasterized in the depth prepass, the normal prepass, the main opaque pass
// and each of `rig.rs`'s four shadow cascades — and the canopy is
// `AlphaMode::Mask` with `cull_mode: None`, which is the expensive kind in
// every one of them. `bevy_light`'s `check_dir_light_mesh_visibility` consults
// `VisibleEntityRanges` exactly as the camera's own check does, so a swapped
// tree is cheap in the shadow map too, not only on screen. That is the half
// that makes this worth more than the triangle count suggests.
//
// **What is NOT claimed: that this was measured on a GPU.** No GPU has ever
// run this client (`RENDER.md` §6). Everything above is counts × passes, and
// the gates below are arithmetic for the same reason every other gate here is.

/// Distance from the eye at which a tree stops being its own geometry and
/// becomes [`impostor`]'s hull, metres. **(knob)**
///
/// 80 m is not picked: it is the band `tests/tree.rs` already measured as the
/// affordable one — 82 trees at the ring's p90, ~350 k triangles, inside the
/// ~600 k a forest can have of `DESIGN.md` §9's 1.5 M. The same table is why
/// it is not 120 (168 trees, ~709 k) or 160 (288, ~1.22 M).
pub const TREE_LOD_SWAP_M: f32 = 80.0;

/// How wide the crossfade between the two LODs is, metres. **(knob)**
///
/// Bevy dithers across this band rather than cutting, which is what stops the
/// swap reading as a pop. Wide enough to cross at a walk (the sprint is ~5
/// m/s, so ~3 s of transit) and narrow enough that the doubled draw — both
/// LODs are resident inside it — is a thin annulus of the ring rather than a
/// third of it.
pub const TREE_LOD_FADE_M: f32 = 15.0;

/// Where the far LOD itself fades out, metres — and it is an UPPER BOUND on
/// the prop ring's own diagonal rather than a chosen view distance.
///
/// **This is the number that keeps the slice honest.** `ART.md` §8 wants the
/// far third of the frame populated, so an LOD that also culls would be
/// buying the budget back with the picture. `props::stream` holds chunks
/// within `NEAR_RADIUS` of the eye's own, so no tree it has spawned can be
/// further than `(NEAR_RADIUS + 1) × CHUNK_M × √2`; `× 2.0` is that bound
/// without a `sqrt` a `const` cannot take, and `tests/tree.rs` holds it
/// against the real diagonal. Past here the ring has already despawned the
/// chunk, so this range never fires in play — it exists so the component's
/// arithmetic is total.
pub const TREE_LOD_REACH_M: f32 =
    (super::terrain_mesh::NEAR_RADIUS as f32 + 1.0) * super::terrain_mesh::CHUNK_M * 2.0;

/// Height bands the impostor lathes through. Eight is what separates a
/// conifer's skirt from its crown at the scale this is seen — the whole hull
/// is under 6 pixels wide at the swap distance, and the count buys shape, not
/// detail.
pub const IMPOSTOR_BANDS: usize = 8;

/// Sides of the lathe. Odd on purpose: an even count puts two silhouette
/// edges parallel from every angle, which is the one thing that reads as
/// "cylinder" rather than "tree". `props.rs`'s own silhouette builder uses 7
/// for the same reason.
pub const IMPOSTOR_SIDES: usize = 7;

/// How much of a height band's surface area the hull encloses, as a fraction.
/// **(knob)**
///
/// **Not `max`, and the first cut of this WAS `max`.** A conifer's outermost
/// needle card at ground level sits at 1.445 m where 90% of that band's
/// surface area is inside 0.922 — so taking the widest vertex built a green
/// drum of the tree's full radius from the ground to 1.65 m, against a tree
/// whose mass there is half that. One stray card is not a silhouette. 0.9 is
/// the envelope with the wisps trimmed; the 10% outside it is branch tips
/// that are sub-pixel at the swap distance, and `tests/tree.rs` holds the
/// hull inside the tree's own measured radius either way.
pub const IMPOSTOR_GIRTH_Q: f32 = 0.9;

/// Ceiling on one impostor's triangles — the lathe's full count, before the
/// tip band's degenerate half is dropped. `tests/tree.rs` measures against it.
pub const IMPOSTOR_MAX_TRIS: usize = IMPOSTOR_BANDS * IMPOSTOR_SIDES * 2;

/// The two [`VisibilityRange`]s a tree's parts carry: the near pair (trunk and
/// canopy) and the far hull.
///
/// **One type because the two are one claim.** Bevy's own documentation
/// states the requirement — *"the `end_margin` of a higher LOD is always
/// identical to the `start_margin` of the next lower LOD; this is important
/// for the crossfade effect to function properly"* — and two ranges written at
/// two spawn sites is exactly how that stops being true. `tests/tree.rs` holds
/// the pair contiguous.
///
/// **A resource because the swap distance is a graphics tier** (`config::
/// Quality`, `render/quality.rs`): `props::stream` reads it when a chunk
/// streams in and `quality::reband_trees` rewrites the trees already standing,
/// so both halves come from this one value.
/// `Debug` is deliberately absent: `VisibilityRange` does not implement it,
/// so a derive here would be a wrapper around a type that cannot print. The
/// two `Range<f32>`s inside it print perfectly well and the gates name them
/// individually.
#[derive(Resource, Clone, PartialEq)]
pub struct TreeLod {
    pub near: VisibilityRange,
    pub far: VisibilityRange,
}

impl Default for TreeLod {
    /// [`TREE_LOD_SWAP_M`] — the shipped distance, and `Quality::High`'s.
    fn default() -> Self {
        Self::at(TREE_LOD_SWAP_M)
    }
}

impl TreeLod {
    /// The pair for a given swap distance. The fade width and the reach do
    /// not move with it: the fade is how long a crossfade takes to walk
    /// through and the reach is the prop ring's diagonal, and neither is a
    /// function of where the swap happens.
    pub fn at(swap_m: f32) -> Self {
        let fade_end = swap_m + TREE_LOD_FADE_M;
        Self {
            near: VisibilityRange {
                // No near margin: a tree you stand in is its own geometry.
                start_margin: 0.0..0.0,
                end_margin: swap_m..fade_end,
                use_aabb: false,
            },
            far: VisibilityRange {
                start_margin: swap_m..fade_end,
                // `use_aabb` stays false on BOTH, which is Bevy's own note
                // about crossfading: the two LODs have different AABBs and a
                // fade needs one distance, so the distance is the shared
                // origin — the same `transform` `spawn_slot` gives every part
                // of a tree.
                end_margin: TREE_LOD_REACH_M..(TREE_LOD_REACH_M + TREE_LOD_FADE_M),
                use_aabb: false,
            },
        }
    }

    /// Which band a tree part belongs in, or `None` for a part that carries
    /// no range at all.
    ///
    /// **The stump is the `None`, and it is stated here rather than assumed
    /// at the two call sites.** It is a few dozen triangles that only exist
    /// after a cut, so it draws at any distance; a `_ => near` default would
    /// have quietly banded it the first time somebody added a part.
    pub fn for_part(&self, part: super::props::FellPart) -> Option<&VisibilityRange> {
        match part {
            super::props::FellPart::Far => Some(&self.far),
            super::props::FellPart::Trunk | super::props::FellPart::Canopy => Some(&self.near),
            super::props::FellPart::Stump | super::props::FellPart::Vanish => None,
        }
    }
}

/// The far LOD for a conifer pair: an opaque lathe through the tree's own
/// vertices.
///
/// **Derived, never authored, and that is the whole design.** Every number in
/// the hull comes off the meshes it replaces — the ring radii are each height
/// band's [`IMPOSTOR_GIRTH_Q`] girth, the height is the pair's measured
/// height, and the colour is the tree's own two ramps mixed by how much of
/// each band is canopy geometry rather than bark. So it cannot drift from the
/// tree: change a generator parameter, or add a species, and the hull moves
/// with it. Measured output, for a reader who wants to know what it makes: a
/// conifer is a spire tapering 0.91 m → 0 over 6.6 m, and a broadleaf is
/// 0.21 m of trunk for its first 0.7 m and then a dome peaking near 1.9 m.
///
/// Three consequences worth stating because a reader will otherwise assume
/// the opposite:
///
///   * **Everything is weighted by triangle AREA, not by vertex count.** A
///     canopy is hundreds of tiny cards and a trunk is a few long quads, so
///     counting either would let the canopy decide the whole tree.
///   * **The hull is NARROWER than the tree's own maximum radius**, by about
///     a third on a conifer, and that is [`IMPOSTOR_GIRTH_Q`]'s doing rather
///     than a bug — the outermost tenth of a canopy's area is wisps.
///   * **The hull is opaque where the canopy is mostly air.** An alpha card
///     canopy at 80 m is perhaps half coverage; a solid hull at the needle
///     colour is denser than that. Narrower and denser push the silhouette in
///     opposite directions, which is the case for the crossfade rather than
///     an argument that they cancel. A person looking is what settles it —
///     `CLAUDE.md` is explicit that the visual gate is the operator booting
///     the game, and no arithmetic here can say whether it reads right.
pub fn impostor_of(bark: &Mesh, needles: &Mesh, variant: usize) -> Mesh {
    let sp = &SPECIES[species_of(variant)];
    let (h, _) = bounds(&[bark, needles]);
    if h <= f32::EPSILON {
        // The same fallback `conifer` takes on a generator failure: draw
        // something rather than panic a client at boot.
        return Mesh::from(Cuboid::default());
    }
    let step = h / IMPOSTOR_BANDS as f32;
    let band_of = |y: f32| ((y.max(0.0) / step) as usize).min(IMPOSTOR_BANDS - 1);

    // Pass one: every triangle as one (radius, area) sample in its own height
    // band, and how much of each band is canopy rather than bark. One sample
    // per triangle at its centre, weighted by its area — a needle card and a
    // trunk quad differ by two orders of magnitude in size, so counting
    // triangles instead of measuring them would let the canopy decide
    // everything.
    let mut girth: [Vec<(f32, f32)>; IMPOSTOR_BANDS] = Default::default();
    let mut leaf_area = [0.0f32; IMPOSTOR_BANDS];
    let mut bark_area = [0.0f32; IMPOSTOR_BANDS];
    for (m, is_leaf) in [(bark, false), (needles, true)] {
        for_each_tri(m, |a, b, c| {
            let area = 0.5 * (b - a).cross(c - a).length();
            let mid = (a + b + c) / 3.0;
            let bin = band_of(mid.y);
            if is_leaf {
                leaf_area[bin] += area;
            } else {
                bark_area[bin] += area;
            }
            girth[bin].push(((mid.x * mid.x + mid.z * mid.z).sqrt(), area));
        });
    }

    // Each band's radius: the one that [`IMPOSTOR_GIRTH_Q`] of its surface
    // area lies inside.
    let mut band_r = [0.0f32; IMPOSTOR_BANDS];
    for (bin, rows) in girth.iter_mut().enumerate() {
        if rows.is_empty() {
            continue;
        }
        // Total-order sort: a NaN radius would come from a degenerate vertex
        // upstream and there is nothing sensible to do with it here, so it
        // sorts last rather than poisoning the comparison.
        rows.sort_by(|x, y| x.0.total_cmp(&y.0));
        let want: f32 = rows.iter().map(|r| r.1).sum::<f32>() * IMPOSTOR_GIRTH_Q;
        let mut acc = 0.0;
        band_r[bin] = rows.last().map_or(0.0, |r| r.0);
        for (r, a) in rows.iter() {
            acc += a;
            if acc >= want {
                band_r[bin] = *r;
                break;
            }
        }
    }

    // The ring radii: a band's own girth at its centre, so a shared boundary
    // is the mean of the two bands it divides and the profile is a
    // piecewise-linear read of the tree rather than a stack of steps. The tip
    // closes to a point — a flat-topped cylinder is the other thing that
    // reads as "not a tree" at range.
    let mut ring = [0.0f32; IMPOSTOR_BANDS + 1];
    ring[0] = band_r[0];
    for i in 1..IMPOSTOR_BANDS {
        ring[i] = (band_r[i - 1] + band_r[i]) * 0.5;
    }
    ring[IMPOSTOR_BANDS] = 0.0;

    // Canopy share, sampled between band CENTRES rather than per band, so the
    // hull has no colour seam where one band ends.
    let share = |bin: usize| {
        let total = leaf_area[bin] + bark_area[bin];
        if total <= f32::EPSILON {
            0.0
        } else {
            leaf_area[bin] / total
        }
    };
    let leaf_share = |y: f32| {
        let t = (y / step - 0.5).clamp(0.0, (IMPOSTOR_BANDS - 1) as f32);
        let lo = t.floor() as usize;
        let hi = (lo + 1).min(IMPOSTOR_BANDS - 1);
        let k = t - lo as f32;
        share(lo) * (1.0 - k) + share(hi) * k
    };

    // The two ramps are `conifer`'s own, over the same bands — see
    // [`BARK_BAND_TOP_FRAC`]. Taken un-normalised: the bark's vertex field is
    // mean-1 because that half wears a photograph, and a hull wearing the
    // untextured `foliage` material would come out white if it copied it.
    let ramp = |lo: u32, hi: u32, y0: f32, y1: f32, y: f32| {
        let (a, b) = (linear(lo), linear(hi));
        let t = ((y - y0) / (y1 - y0).max(f32::EPSILON)).clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let color = |v: Vec3| {
        let wood = ramp(BARK_LO, BARK_HI, 0.0, h * BARK_BAND_TOP_FRAC, v.y);
        let leaf = ramp(sp.leaf_lo, sp.leaf_hi, h * LEAF_BAND_BASE_FRAC, h, v.y);
        let f = leaf_share(v.y);
        [
            wood[0] + (leaf[0] - wood[0]) * f,
            wood[1] + (leaf[1] - wood[1]) * f,
            wood[2] + (leaf[2] - wood[2]) * f,
            1.0,
        ]
    };

    let mut s = Soup::default();
    for bin in 0..IMPOSTOR_BANDS {
        let (y0, y1) = (bin as f32 * step, (bin + 1) as f32 * step);
        let (r0, r1) = (ring[bin], ring[bin + 1]);
        // Normals blend toward the trunk axis by the same `PINE_NORMAL_BLEND`
        // the near canopy uses, for the same reason: a tree is a volume that
        // scatters, not a stack of plates.
        let axis = Vec3::new(0.0, (y0 + y1) * 0.5, 0.0);
        for side in 0..IMPOSTOR_SIDES {
            let a0 = side as f32 / IMPOSTOR_SIDES as f32 * std::f32::consts::TAU;
            let a1 = (side + 1) as f32 / IMPOSTOR_SIDES as f32 * std::f32::consts::TAU;
            let at = |a: f32, r: f32, y: f32| Vec3::new(a.cos() * r, y, a.sin() * r);
            let (b0, b1) = (at(a0, r0, y0), at(a1, r0, y0));
            let (t0, t1) = (at(a0, r1, y1), at(a1, r1, y1));
            // Each half is skipped when its own pair of corners coincides —
            // the tip band's upper edge is one point. `Soup::mesh` generates
            // tangents and mikktspace REFUSES a degenerate triangle, so this
            // is a panic at boot rather than a wasted triangle.
            if r0 > f32::EPSILON {
                s.tri(b0, t0, b1, color, Some(axis), PINE_NORMAL_BLEND);
            }
            if r1 > f32::EPSILON {
                s.tri(b1, t0, t1, color, Some(axis), PINE_NORMAL_BLEND);
            }
        }
    }
    s.mesh()
}

/// Every triangle of a mesh as three corners. Skips an out-of-range index
/// rather than panicking, for [`positions`]'s reason — a generator change must
/// not be able to take the client down at boot.
fn for_each_tri(m: &Mesh, mut f: impl FnMut(Vec3, Vec3, Vec3)) {
    let Some(p) = positions(m) else { return };
    let mut emit = |i: usize, j: usize, k: usize| {
        if let (Some(a), Some(b), Some(c)) = (p.get(i), p.get(j), p.get(k)) {
            f(
                Vec3::from_array(*a),
                Vec3::from_array(*b),
                Vec3::from_array(*c),
            );
        }
    };
    match m.indices() {
        Some(Indices::U32(ix)) => {
            for t in ix.chunks_exact(3) {
                emit(t[0] as usize, t[1] as usize, t[2] as usize);
            }
        }
        Some(Indices::U16(ix)) => {
            for t in ix.chunks_exact(3) {
                emit(t[0] as usize, t[1] as usize, t[2] as usize);
            }
        }
        None => {
            for t in (0..p.len()).step_by(3) {
                emit(t, t + 1, t + 2);
            }
        }
    }
}
