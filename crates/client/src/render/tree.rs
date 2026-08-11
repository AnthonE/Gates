//! The conifer, generated rather than authored.
//!
//! **This is the browser's decision, carried across the port.** `props.js`
//! says it in one line — *"a conifer's canopy is made of ALPHA CARDS, and an
//! opaque hull with a polygon edge cannot get there from any amount of
//! geometry"* — and it says so after three passes of building pines out of
//! cones. The native client shipped the cone stack because the port went slice
//! by slice, not because the question reopened. `props.rs`'s whorl builder is
//! still there and still correct; it is the far-LOD's starting point, which is
//! the same role it holds in the browser.
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
use bevy::image::Image;
use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_procedural_tree::enums::{LeafBillboard, TreeType};
use bevy_procedural_tree::meshgen::generate_tree_meshes;
use bevy_procedural_tree::settings::{
    BranchForce, BranchParams, BranchRecursionLevel, LeafParams, TreeMeshSettings,
};

use super::props::{hash2, PINE_H, PINE_MAX_R, PINE_NORMAL_BLEND};

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
/// become a billboard, which `TERRAIN.md` §4 already queues and this slice
/// does not build. Until it exists the ring is over budget on a dense forest,
/// knowingly, and `tests/tree.rs` prints the arithmetic so it cannot be
/// forgotten.
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
    band(&mut bark, BARK_LO, BARK_HI, 0.0, sp.height_m * 0.6, true);
    band(
        &mut needles,
        sp.leaf_lo,
        sp.leaf_hi,
        sp.height_m * 0.15,
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
    img.sampler = bevy::image::ImageSampler::linear();
    img
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

/// Does the pair fit the volume the sim blocks? `PINE_MAX_R` is not a
/// rendering number: `world.rs` derives `SPAWN_CLEAR_M = 4.0` from it, so a
/// canopy that grew past it puts fresh spawns inside trees.
pub fn fits_sim_bounds(bark: &Mesh, needles: &Mesh) -> bool {
    let (_, r) = bounds(&[bark, needles]);
    r <= PINE_MAX_R
}
