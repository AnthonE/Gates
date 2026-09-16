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

use super::props::{hash01, hash2, Soup, PINE_H, PINE_MAX_R, PINE_NORMAL_BLEND};

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
    // **Shorter than the conifer**, which is the whole read: a pine is a
    // spire and a broadleaf is not, and if the two shared a silhouette
    // envelope there would be no point having both. 11 m against 14 keeps
    // the conifer as the thing that breaks the skyline. It was 5.4 against
    // 6.6 until forest scale v0 (2026-09-14); both share the 2.9 m crown
    // ceiling now, so at this height the broadleaf is a birch's column
    // rather than the dome it was — which is the species the reference's
    // `Alt` mask paints (`FORESTS.md` §3).
    SpeciesDef {
        tree_type: TreeType::Deciduous,
        height_m: 11.0,
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
            // Past horizontal, drooping: a pine's limbs hang. Droop is the
            // ANGLE's job here and NOT the branch force's — see `force`
            // below. 104° since forest scale v0, from 96; ez-tree's own pine
            // preset droops to 110.
            angle: [0.0, 104.0, 0.0, 0.0],
            // 72 limbs at 12 cards each: the same 5,900 triangles as 60 at
            // 16, spread over more limbs, because at 14 m a limb is the
            // thing the eye counts.
            children: [72, 0, 0],
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
            //
            // **Re-swept at 14 m for forest scale v0** (`examples/tree_sweep.rs`,
            // 2026-09-14, three shipped seeds and twelve more): a 2.2 m limb
            // read 2.92 / 2.86 / 2.96 m on the shipped seeds against the
            // 2.9 ceiling, 1.9 read 2.73, and **1.8 reads 2.61 / 2.58 / 2.66**
            // (2.70 over all eighteen) — an 8 % margin. The trunk radius is
            // the sim's number: `tests/tree.rs` holds the widest shipped
            // trunk to `OCCUPANT_R_M[Tree]` within a millimetre, and 0.2540
            // puts variant 0 at 0.2390 m. (Scaled from 0.20 → 0.1887 at this
            // height; the fit is linear in it. One of the twelve extra seeds
            // reads 0.2406, so a fourth seed would need re-sweeping.)
            length: [14.0, 1.8, 0.0, 0.0],
            trunk_base_radius: 0.2540,
            radius_factor: [1.0, 0.13, 0.0, 0.0],
            sections: [10, 4, 0, 0],
            segments: [7, 4, 0, 0],
            // Limbs start at 20% of the trunk — 2.8 m on a 14 m tree, which
            // is the bare trunk the reference's pines show below the crown
            // and clear of `TRUNK_MEASURE_H_M`. It was 10% on the 6.6 m tree
            // (0.66 m), because 27% there was 1.8 m: eye level, a colonnade
            // of poles. The height is what changed the right answer.
            start: [0.0, 0.20, 0.0, 0.0],
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
            // 1.15 m is a real branchlet's size against a 14 m tree (it was
            // 0.55 against 6.6, the same fraction), which is what the mask
            // draws: a sprig cluster, not one needle. 12 per limb × 72 limbs
            // is the 16 × 60 it replaced, in triangles.
            count: 12,
            start: 0.0,
            size: 1.15,
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
            // **Steep, since forest scale v0** — 36° / 32° from 52 / 44. At
            // 11 m the 2.9 m crown ceiling is a column, not a dome, and the
            // sweep (`examples/tree_sweep.rs`, 2026-09-14) could not get
            // under it by shortening limbs alone: half-length limbs at the
            // old angles still read 3.0–3.2 m. Closing the angles is what a
            // birch does anyway; its limbs leave the trunk steeply.
            angle: [0.0, 36.0, 32.0, 0.0],
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
            // Limbs at 0.38 / 0.20 of the 4.5 trunk unit, from 1.75 / 1.0:
            // with the angles above this reads 2.68 m on the shipped seeds
            // and 2.89 over eighteen — the widest thing in the pool by a
            // hair, under the ceiling. The trunk is 0.11 from 0.22 for the
            // doubled height: 0.234 m at the base, inside the sim's cylinder,
            // which the conifer now sets (`OCCUPANT_R_M[Tree]`).
            length: [4.5, 0.38, 0.20, 0.0],
            trunk_base_radius: 0.11,
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
            // 0.60 m at 11 m, from 0.42 at 5.4: smaller as a fraction of the
            // tree, because a card's size adds to the crown radius directly
            // and the ceiling did not grow with the height.
            size: 0.60,
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

/// Pull every needle normal away from the canopy's VOLUME by
/// `PINE_NORMAL_BLEND`.
///
/// A leaf card's own normal is the card's facing, so a canopy of a few hundred
/// cards lit by one directional sun resolves into a few hundred flat plates —
/// the "asset-pack tree" signature `props.rs` names, and the face `ART.md` §5
/// says every judge catches. A needle mass does not have facets; it scatters
/// as a volume, and this function is that volume's normal field.
///
/// ⚠ **That field used to be `Vec3::new(x, 0.0, z)` — purely horizontal, at
/// every vertex, with no exception.** At `PINE_NORMAL_BLEND = 0.7` that leaves
/// at most 30 % of any card's own facing, so **no canopy normal in this game
/// has ever had a meaningful vertical component**, and a canopy whose normals
/// are all horizontal has no top and no bottom under any sun: a high key at
/// 30–40° (`ART.md` §4) lands on every one of them at the same `N·L`. That is
/// most of why the operator's 2026-09-16 frames read as flat green cut-outs
/// while the reference's crowns read as masses — theirs are lit on top and
/// dark underneath, and ours could not be, by construction.
///
/// The crown is an ELLIPSOID, not a cylinder. Its outward normal at `P` is
/// `normalize((P − C) / axes²)` for centre `C` and semi-axes `(R, H/2, R)`,
/// measured off the canopy mesh itself so no species needs a number here. For
/// a tall crown `H ≫ R`, dividing the vertical term by the larger square keeps
/// the field nearly radial through the middle — which is what the old code got
/// right — and turns it up at the apex and down under the skirt, which is what
/// it had no way to express. The underside then reads (rule 3), and it is the
/// hemisphere fill's job (`render/fill.rs`) to keep it off the 0.30 floor.
fn blend_canopy_normals(m: &mut Mesh) {
    let Some(p) = positions(m) else { return };
    let (lo_y, hi_y, r_max) = crown_extent(p);
    let c_y = (lo_y + hi_y) * 0.5;
    // Semi-axes. Floored so a degenerate canopy divides by something.
    let sy = ((hi_y - lo_y) * 0.5).max(1e-3);
    let sr = r_max.max(1e-3);
    let (iy2, ir2) = (1.0 / (sy * sy), 1.0 / (sr * sr));
    let outward: Vec<[f32; 3]> = p
        .iter()
        .map(|v| {
            let e = Vec3::new(v[0] * ir2, (v[1] - c_y) * iy2, v[2] * ir2);
            // Dead centre of the crown has no outward direction; up is the
            // only defensible answer and it affects a handful of vertices.
            e.normalize_or(Vec3::Y).to_array()
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

/// The canopy's vertical extent and widest radius, from its own vertices.
///
/// Shared by [`blend_canopy_normals`] and [`occlude_canopy`] so the two cannot
/// disagree about where the crown is — a normal field and an occlusion field
/// built against two different envelopes is the drift this file warns about
/// elsewhere.
fn crown_extent(p: &[[f32; 3]]) -> (f32, f32, f32) {
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    let mut r = 0.0f32;
    for v in p {
        lo = lo.min(v[1]);
        hi = hi.max(v[1]);
        r = r.max((v[0] * v[0] + v[2] * v[2]).sqrt());
    }
    if lo > hi {
        return (0.0, 0.0, 0.0);
    }
    (lo, hi, r)
}

/// Height bins the crown's radius profile is measured in. Enough that a 14 m
/// conifer's cone is resolved to under a metre, few enough that every bin holds
/// hundreds of vertices and its maximum is a shape rather than a sample.
const CROWN_BINS: usize = 24;

/// How dark the most buried canopy vertex gets, as a fraction of an exposed
/// one. **`ART.md` rule 3's floor, applied to albedo rather than to a light**:
/// nothing in a frame sits below 0.30 of its lit self, and a canopy interior
/// that reaches zero is a black hole with leaves around it.
const CANOPY_AO_FLOOR: f32 = 0.30;

/// How sharply the darkening falls off from the crown's surface inward. Above
/// 1.0 the outer shell keeps its colour and the fall is concentrated deeper in,
/// which is what a foliage mass does — light gets a little way in and then
/// stops.
const CANOPY_AO_GAMMA: f32 = 1.6;

/// Darken each canopy vertex by how deep inside the crown it sits.
///
/// **This is the mechanism the canopy has never had, and it is why a tree in
/// this game reads as a green cut-out.** [`band`] ramps the needle mesh's
/// vertex colour on `y` ALONE, so every needle at a given height is the same
/// colour whether it is buried against the trunk or out at a limb tip.
/// Measured on the shipped seeds before this landed
/// (`examples/canopy_probe.rs`, 2026-09-16), binning canopy vertices by radial
/// depth: the broadleaf ran 0.156 → 0.147 linear luma from core to rim — **6 %
/// over the whole crown** — and the conifer ran 0.129 → 0.077, which is not
/// merely flat but BACKWARDS, its rim darker than its core, because the widest
/// part of a cone is its shaded bottom and the height ramp was the only thing
/// speaking. Real foliage is the other way round by an order of magnitude: the
/// shell is lit and the inside is nearly black. That contrast is what makes
/// the reference's crowns read as masses.
///
/// The depth term is `r / R(y)` — radius against the crown's own radius at
/// that height, never against a global maximum, or a cone's narrow apex would
/// read as buried when it is the most exposed part of the tree.
///
/// **`R(y)` is interpolated between bin centres rather than held flat across a
/// bin**, because a piecewise-constant profile steps at every bin edge and a
/// step in a value that varies along a set of constant elevation is a contour
/// drawn on the tree in shading — the mechanism `terrain::remap`'s LUT drew on
/// the whole island (`CLAUDE.md` traps, `sim-core/tests/contour.rs`).
///
/// ⚠ **Nothing enforces that, and this note says so rather than implying it.**
/// A piecewise-constant profile passes all twenty-two gates in
/// `tests/tree.rs` — run, 2026-09-16, not assumed. The lerp is one line of
/// insurance against a defect whose stakes here are genuinely lower than on
/// the terrain: this is a colour, where `terrain::remap` fed an analytic
/// NORMAL, and the canopy it lands on is alpha-masked noise rather than a
/// smooth surface. So it stays, and it is a habit rather than a wall.
///
/// **One occlusion term, not two.** `ART.md` §4's Frostbite rule forbids
/// multiplying two occlusion terms of the same scale; the height ramp above is
/// a TINT — two authored greens, `ART.md` §5's "two greens minimum" — and not
/// an occlusion term, so this is the only one and it multiplies a tinted
/// albedo. Baked into the albedo is also the documented home for the micro
/// scale, which is the one term that applies to direct light as well as
/// indirect — and a canopy interior is dark to the sun, not only to the sky.
fn occlude_canopy(m: &mut Mesh) {
    let Some(p) = positions(m) else { return };
    let (lo_y, hi_y, _) = crown_extent(p);
    let span = (hi_y - lo_y).max(f32::EPSILON);

    // The radius profile: the widest vertex in each height bin.
    let mut prof = [0.0f32; CROWN_BINS];
    for v in p {
        let f = ((v[1] - lo_y) / span).clamp(0.0, 0.999);
        let b = (f * CROWN_BINS as f32) as usize;
        prof[b] = prof[b].max((v[0] * v[0] + v[2] * v[2]).sqrt());
    }
    // An empty bin (a gap in the canopy) would read as a radius of zero and
    // darken everything near it to the floor. Carry the nearest filled one.
    let mut last = 0.0f32;
    for b in prof.iter_mut() {
        if *b <= 0.0 {
            *b = last;
        } else {
            last = *b;
        }
    }
    for b in prof.iter_mut().rev() {
        if *b <= 0.0 {
            *b = last;
        } else {
            last = *b;
        }
    }

    // `R(y)`, linear between bin CENTRES — see the note above on why a flat
    // bin is a contour.
    let radius_at = |y: f32| -> f32 {
        let f = ((y - lo_y) / span).clamp(0.0, 1.0) * CROWN_BINS as f32 - 0.5;
        let i = f.floor();
        let t = f - i;
        let a = prof[(i.max(0.0) as usize).min(CROWN_BINS - 1)];
        let b = prof[((i + 1.0).max(0.0) as usize).min(CROWN_BINS - 1)];
        a + (b - a) * t
    };

    // Resolved against the positions BEFORE the colours are borrowed mutably —
    // the same shape `blend_canopy_normals` uses next door, and for the same
    // borrow reason.
    let shade: Vec<f32> = p
        .iter()
        .map(|v| {
            let r = (v[0] * v[0] + v[2] * v[2]).sqrt();
            let rr = radius_at(v[1]);
            // Exposure: 0 buried on the axis, 1 out at the crown's surface.
            let e = if rr <= f32::EPSILON {
                1.0
            } else {
                (r / rr).clamp(0.0, 1.0)
            };
            CANOPY_AO_FLOOR + (1.0 - CANOPY_AO_FLOOR) * e.powf(CANOPY_AO_GAMMA)
        })
        .collect();

    let Some(VertexAttributeValues::Float32x4(cols)) = m.attribute_mut(Mesh::ATTRIBUTE_COLOR)
    else {
        return;
    };
    for (c, k) in cols.iter_mut().zip(shade) {
        c[0] *= k;
        c[1] *= k;
        c[2] *= k;
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
    occlude_canopy(&mut needles);
    blend_canopy_normals(&mut needles);
    (bark, needles)
}

/// Needle-sprig size in texels. Two of these tile the leaf card's UV range.
///
/// **256, from 64 (canopy grain v0, 2026-09-16), and the reason is arithmetic
/// rather than taste.** A conifer card measures 1.08 m of world on the
/// shipped seeds (`examples/canopy_probe.rs`; NOT `LeafParams::size`, which is
/// in the generator's units — see [`leaf_image`]), so at 64 texels one texel
/// is **16.9 mm of tree**. `stamp`'s thinnest line that survives the 0.5 alpha
/// cut is about one and a half texels, so the finest thing this card could
/// draw was a **34 mm** stripe — and the sprig drew them 23 cm long. A real
/// conifer needle is 1–2 mm by 3–6 cm. What the mask was drawing, measured,
/// was **25× too wide and 5× too long**: not needles, FRONDS, which is why the
/// operator's 2026-09-16 frames read as a forest of tree ferns.
///
/// At 256 a texel is 4.2 mm, a needle is ~1 texel wide and a 5 cm needle is
/// twelve of them. That is the whole of the change — the card's world size is
/// untouched, because it is swept against `BROADLEAF_MAX_R`/`PINE_MAX_R` and
/// moving it moves the sim's crown ceiling. **Fix what is drawn inside the
/// card, not the card.**
///
/// The cost is 256² × 4 B plus a chain, twice: ~700 kB of VRAM for both cards,
/// against the 12 MB `ART.md` §7 discusses for one photographic set. The mip
/// chain absorbs the rest — `needle_mips` preserves coverage through it by
/// construction, which is what lets the level-0 detail be this fine at all.
const NEEDLE_TEX: u32 = 256;

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
    let size = NEEDLE_TEX as f32;

    // 2 branchlets, mirrored about the card's centre line, so a `Double`
    // billboard pair does not show the same silhouette twice from every angle.
    for (sx, dir) in [(0.30f32, 1.0f32), (0.70, -1.0)] {
        let stem_x = sx * size;
        let stem_len = size * 0.90;

        // The woody axis: thin, tapering, running up the card.
        let steps = stem_len.ceil() as u32;
        for s in 0..=steps {
            let u = s as f32 / steps.max(1) as f32;
            stamp(
                &mut data,
                n,
                stem_x,
                1.0 + u * stem_len,
                1.4 * (1.0 - 0.55 * u),
            );
        }

        // Side twigs, alternating, shortening toward the tip — the same
        // silhouette rule the whorls encode as a radius ramp.
        //
        // Jittered by a hash of the twig's index, for the reason `leaf_image`
        // gives at length: an evenly spaced comb at one fixed angle reads as a
        // FERN. A conifer's needles are genuinely regular along a twig, so the
        // jitter here is smaller than the broadleaf's and lives mostly in
        // where the twigs sit and how far they reach.
        for i in 0..TWIGS {
            let t = i as f32 / (TWIGS - 1).max(1) as f32;
            let j = |salt: u32| hash01(0x6eed_0000 ^ salt, i) - 0.5;
            let root_y = 1.0 + (0.05 + 0.88 * t + 0.04 * j(1)) * stem_len;
            let side = if i % 2 == 0 { 1.0 } else { -1.0 } * dir;
            let tlen = (1.0 - 0.58 * t) * (1.0 + 0.30 * j(2)) * size * 0.22;
            // ~48° off the axis, sweeping up: a conifer's twigs rise. ±10°.
            let a = 0.83 + 0.34 * j(3);
            let (tx, ty) = (side * a.sin(), a.cos());
            let ts = tlen.ceil() as u32;
            for s in 0..=ts {
                let u = s as f32 / ts.max(1) as f32;
                stamp(
                    &mut data,
                    n,
                    stem_x + tx * u * tlen,
                    root_y + ty * u * tlen,
                    1.0 * (1.0 - 0.45 * u),
                );
            }
            needle_twig(&mut data, n, stem_x, root_y, tx, ty, tlen, NEEDLES_PER_TWIG);
        }

        // The axis carries needles between its twigs too. Without this a
        // branchlet reads as a row of separated combs rather than as one
        // dense spray, which is most of what tells a conifer from a fern.
        needle_twig(&mut data, n, stem_x, 1.0, 0.0, 1.0, stem_len, AXIS_NEEDLES);
    }

    alpha_card(data)
}

/// Side twigs per branchlet, and needles per twig and per axis. Tuned on
/// COVERAGE, which is the number that decides whether a canopy is opaque:
/// `tests/tree.rs::the_needle_card_holds_its_world_coverage` holds the total
/// inside a band around the 0.192 the 64² fern card measured, so this change
/// is a change of GRAIN and not of density. Moving any of the three without
/// re-running that gate is how the forest goes bald.
const TWIGS: u32 = 20;
/// See [`TWIGS`].
const NEEDLES_PER_TWIG: u32 = 40;
/// See [`TWIGS`].
const AXIS_NEEDLES: u32 = 200;
/// One needle's length in texels — 5 cm at the card's measured 4.2 mm/texel,
/// which is the middle of a real conifer's 3–6 cm.
const NEEDLE_LEN: f32 = 12.2;
/// One needle's stamp radius in texels. `stamp` writes `1 - d/w`, so the
/// texels that survive the 0.5 cut are those within `w/2` — at 0.85 that is a
/// line about one texel wide, ~4 mm of tree, which is the finest thing this
/// card can draw and about 3× a real needle. Below it the needle stops
/// surviving the cut at all and the canopy goes transparent.
const NEEDLE_W: f32 = 0.85;

/// Clothe one twig in needles: `count` of them alternating down the axis from
/// `(ax, ay)` along the unit direction `(tx, ty)`, each leaving it at ~55° and
/// swept toward its tip.
///
/// One routine for the axis and for every side twig, because a needle is a
/// needle wherever it is rooted — and because two copies of this loop is where
/// the axis and the twigs would drift apart.
#[allow(clippy::too_many_arguments)]
fn needle_twig(
    data: &mut [u8],
    n: usize,
    ax: f32,
    ay: f32,
    tx: f32,
    ty: f32,
    twig_len: f32,
    count: u32,
) {
    // Perpendicular to the twig, in the card's plane.
    let (px, py) = (-ty, tx);
    for i in 0..count {
        let t = i as f32 / (count - 1).max(1) as f32;
        let (rx, ry) = (ax + tx * t * twig_len, ay + ty * t * twig_len);
        let side = if i % 2 == 0 { 1.0 } else { -1.0 };
        // ~55° from the twig, swept toward the tip.
        const SIN: f32 = 0.82;
        const COS: f32 = 0.57;
        let (dx, dy) = (px * side * SIN + tx * COS, py * side * SIN + ty * COS);
        // Needles shorten toward the twig's tip.
        let len = NEEDLE_LEN * (1.0 - 0.30 * t);
        let steps = (len * 2.0).ceil() as u32;
        for s in 0..=steps {
            let u = s as f32 / steps.max(1) as f32;
            // A needle tapers to a point.
            let w = NEEDLE_W * (1.0 - 0.40 * u);
            stamp(data, n, rx + dx * u * len, ry + dy * u * len, w);
        }
    }
}

/// A finished alpha card from a level-0 RGBA buffer: the coverage-preserved
/// mip chain, the descriptor that carries it, and the trilinear sampler.
/// Shared by [`needle_image`] and [`leaf_image`], because a second species'
/// card that built its own chain would be the first place the two drifted.
fn alpha_card(data: Vec<u8>) -> Image {
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

/// The alpha card a BROADLEAF canopy is made of — generated at boot beside
/// [`needle_image`], and for the same reasons.
///
/// **A species is a silhouette AND a card, and until forest scale v0 the
/// broadleaf had only the first.** Both species wore the needle sprig, so on
/// the lavapipe bench (`examples/tree_look.rs`, 2026-09-14) the 11 m broadleaf
/// read as a second, yellower conifer — a column of sprigs — and the whole
/// reason the pool has two species (`reference/PLANTS.md` §6.1) was lost at
/// any distance the card was legible. This is a cluster of rounded leaves on
/// a short stem, two of them mirrored across the card the way the sprig is,
/// so a `Double` billboard shows two outlines and not one twice.
///
/// Denser than the sprig on purpose: a leaf cluster is mostly leaf where a
/// sprig is mostly air, and `tests/tree.rs` holds the two cards apart on
/// exactly that — the same file's cut-out and mip-chain gates run over this
/// card as they do over the needle's.
///
/// ⚠ **`LeafParams::size` is in the GENERATOR's units, not metres, and every
/// comment in this file that read it as metres was wrong.** `fit_to_bounds`
/// scales the whole mesh by `height_m / generated_height` after the cards are
/// built, so the card's world size is `size × k` — and `k` is not 1. Measured
/// on the shipped seeds (`examples/canopy_probe.rs`, 2026-09-16): the conifer
/// authors 1.15 and draws a **1.08 m** card (k ≈ 0.94, near enough to 1 that
/// nobody caught it), and the broadleaf authors 0.60 and draws a **1.25 m**
/// one — **k ≈ 2.09**, because its trunk unit is 4.5 and its height 11 m. So
/// the species whose comment claimed the *smaller* card has the larger one in
/// the frame, by 23 %, and the leaves stamped into it were being sized against
/// a number 2.1× under the truth. That is the whole of why a broadleaf read as
/// one flat green blob: not the shape of the leaf, its SCALE.
///
/// The card size itself stays where the sweep put it — it feeds the crown
/// radius and `BROADLEAF_MAX_R` is the sim's number (`TREE_MAX_R`) — so what
/// changed is what is drawn inside it.
pub fn leaf_image() -> Image {
    let n = NEEDLE_TEX as usize;
    let mut data = vec![0u8; n * n * 4];
    let size = NEEDLE_TEX as f32;

    // Two twigs, mirrored about the centre line, so a `Double` billboard shows
    // two outlines and not one twice.
    for (sx, dir) in [(0.28f32, 1.0f32), (0.72, -1.0)] {
        let stem_x = sx * size;
        let stem_len = size * 0.84;

        // The woody axis.
        let steps = stem_len.ceil() as u32;
        for st in 0..=steps {
            let u = st as f32 / steps.max(1) as f32;
            stamp(
                &mut data,
                n,
                stem_x,
                1.0 + u * stem_len,
                1.2 * (1.0 - 0.5 * u),
            );
        }

        // Side twiglets, each carrying a few leaves — a spray, not a comb.
        //
        // ⚠ **Regular spacing is how this card turned into a FERN**, and the
        // first draft of the 256² rebuild did exactly that: evenly spaced
        // twiglets at one fixed angle, 24 small leaves combed down each, and
        // the mask came out a textbook pinnate frond — the defect the rebuild
        // exists to remove, re-entered through the other door. It was caught
        // by dumping the card and looking at it (`dump_alpha`), which no
        // coverage number could have done. So every twiglet's angle, length
        // and root spacing is jittered by a hash of its index: deterministic,
        // no RNG state, the same card on every client and every run.
        for i in 0..LEAF_TWIGS {
            let t = i as f32 / (LEAF_TWIGS - 1).max(1) as f32;
            let j = |salt: u32| hash01(0x1eaf_0000 ^ salt, i) - 0.5;
            let root_y = 1.0 + (0.06 + 0.86 * t + 0.05 * j(1)) * stem_len;
            let side = if i % 2 == 0 { 1.0 } else { -1.0 } * dir;
            let tlen = (1.0 - 0.45 * t) * (1.0 + 0.45 * j(2)) * size * 0.26;
            // Wider off the axis than a conifer's twig: a broadleaf's limbs
            // leave the stem closer to horizontal (`broadleaf_settings`).
            let a = 0.62 + 0.42 * j(3);
            let (tx, ty) = (side * a.sin(), a.cos());
            let ts = tlen.ceil() as u32;
            for st in 0..=ts {
                let u = st as f32 / ts.max(1) as f32;
                stamp(
                    &mut data,
                    n,
                    stem_x + tx * u * tlen,
                    root_y + ty * u * tlen,
                    0.9 * (1.0 - 0.4 * u),
                );
            }
            leaf_twig(
                &mut data,
                n,
                stem_x,
                root_y,
                tx,
                ty,
                tlen,
                LEAVES_PER_TWIG,
                i,
            );
        }
    }

    alpha_card(data)
}

/// Twiglets per leaf card and leaves per twiglet. Like the sprig's counts
/// these are tuned on COVERAGE — the leaf card must stay DENSER than the
/// needle card (a leaf cluster is mostly leaf where a sprig is mostly air) and
/// `tests/tree.rs` holds both, so this is a change of grain and not of
/// density. 192 leaves a card, where the first 256² draft drew 672.
const LEAF_TWIGS: u32 = 12;
/// See [`LEAF_TWIGS`].
const LEAVES_PER_TWIG: u32 = 8;
/// One leaf's length in texels — **11 cm**, and the number is set by the CARD
/// rather than by botany. Say so plainly, because the two disagree.
///
/// The broadleaf's card measures **1.25 m** of world on the shipped seeds, not
/// the 0.60 its `LeafParams::size` says — see [`leaf_image`]'s note — so at 256
/// a texel is 4.9 mm. The 64² card drew leaves 19 texels at 19.6 mm/texel:
/// **37 cm long and 12 cm wide**, a banana leaf on an 11 m tree, and the single
/// most obvious thing in a frame with a broadleaf in it.
///
/// ⚠ **The first correction went too far the other way, and the card said so
/// when it was looked at.** A birch leaf is 3–7 cm; drawn at 6 cm it takes
/// **672 of them** to fill a 1.25 m card to the density the canopy is built
/// at, and 672 small ovals combed along even twiglets is a textbook PINNATE
/// FROND — the exact read this whole change exists to remove, re-entered
/// through the other door. Coverage was identical either way. It was caught by
/// dumping the mask and looking at it (`examples/canopy_probe.rs`'s
/// `dump_alpha`), which is the whole of `CLAUDE.md`'s point about a person
/// being the visual gate, applied to the one artefact a headless box can
/// still show one.
///
/// So: 11 cm, 192 leaves, irregularly placed. That is a lime or an alder, not
/// the birch `SPECIES` calls this species — and the way to get a birch is a
/// SMALLER CARD, which is `BROADLEAF_MAX_R`'s business and therefore the
/// sim's. `NOW.md` §0t carries it.
const LEAF_LEN: f32 = 22.0;
/// A leaf's half-width at its widest, texels — 6.4 cm against the 11 cm
/// length, a broad ovate leaf's roughly 3:5 proportion.
const LEAF_HALF_W: f32 = 6.6;

/// A row of leaves alternating down one twiglet, each an oval leaving the twig
/// at ~40° and swept toward its tip.
#[allow(clippy::too_many_arguments)]
fn leaf_twig(
    data: &mut [u8],
    n: usize,
    ax: f32,
    ay: f32,
    tx: f32,
    ty: f32,
    twig_len: f32,
    count: u32,
    twig: u32,
) {
    let (px, py) = (-ty, tx);
    for i in 0..count {
        let t = i as f32 / (count - 1).max(1) as f32;
        let j = |salt: u32| hash01(0x0eaf_0000 ^ salt ^ (twig << 8), i) - 0.5;
        // Leaves sit along the twig, spaced unevenly. A perfectly alternating
        // comb is the frond read — see [`leaf_image`].
        let along = (t + 0.10 * j(1)).clamp(0.0, 1.05);
        let (rx, ry) = (ax + tx * along * twig_len, ay + ty * along * twig_len);
        let side = if i % 2 == 0 { 1.0 } else { -1.0 };
        // ~40° off the twig, swept toward its tip, ±18°.
        let a = 0.70 + 0.62 * j(2);
        let (ss, cc) = (a.sin(), a.cos());
        let (dx, dy) = (px * side * ss + tx * cc, py * side * ss + ty * cc);
        let len = LEAF_LEN * (1.0 - 0.20 * t) * (1.0 + 0.30 * j(3));
        let segs = (len * 2.0).ceil() as u32;
        for sg in 0..=segs {
            let u = sg as f32 / segs.max(1) as f32;
            // An oval: widest in the middle, pointed at both ends.
            let w = (std::f32::consts::PI * u).sin() * LEAF_HALF_W + 0.45;
            stamp(data, n, rx + dx * u * len, ry + dy * u * len, w);
        }
    }
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
/// a single tapering column: the generator starts limbs at 20 % of the pine's
/// trunk (2.8 m) and 22 % of the broadleaf's (2.4 m), both comfortably
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
// swapped in by `VisibilityRange` past [`TREE_LOD_SWAP_M`] — on the desktop;
// a browser swaps it by [`swap_by_distance`], because WebGL2 cannot bind the
// table `VisibilityRange` dithers by (`lod_band` says why). Two reasons it is
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

/// The most trees whose near pair may be drawn at once — fully or in the
/// crossfade — and the reason the forest could be made a forest. **(knob)**
///
/// **This is `reference/FORESTS.md` §8 gate 7: the frame budget as a cap,
/// not a print.** [`TREE_LOD_SWAP_M`] alone bounds nothing — a distance is a
/// disc, and what a disc holds is the forest's business, which forest
/// density v1 (2026-09-14) took from ~39 to ~94 stems/ha with stands at
/// ~134. Measured on the shipped seed after it (`sim-core/examples/
/// ring_census.rs`): the 80 m disc plus its 15 m fade holds 278 trees at
/// the p90 eye and 357 at the densest, and a tree is up to
/// [`CONIFER_MAX_TRIS`] — 357 × 5,900 is 2.1 M, over `DESIGN.md` §9's 1.5 M
/// for the whole frame before a hull or a blade of grass is counted. Before
/// v1 the same disc held 82 at p90 and the distance was enough.
///
/// 180 × 6,000 ([`CONIFER_MAX_TRIS`], the ceiling; the generator emits
/// ~5,900) = 1.08 M. With every other tree in both rings a hull at
/// [`IMPOSTOR_MAX_TRIS`] (1,086 near at the shipped seed's densest ring,
/// ~2,430 outer at `OUTER_RADIUS` 4) the trees total under 1.46 M at the
/// worst eye on the island, which `tests/tree.rs` and `tests/outer_ring.rs`
/// hold as arithmetic — at ceilings, not at the measured means, which is
/// why it is 180 and not the 200 the means would admit. The price is where
/// the swap lands when the cap binds: [`cap_swap`] pulls it to the distance
/// holding exactly this many, less the fade — ~61 m at the p90 eye, ~50 m
/// in the densest stand the shipped field makes (~134 stems/ha), and never
/// under 44 m at any density the 8 m grid can produce (156 stems/ha fills
/// 180 by 61 m, less the fade and the step). `tests/tree_cap.rs` holds that
/// floor. A tree 14 m tall is still a tree at 50 m; the hull it becomes is
/// `NOW.md` §0t's "hull pixels" item, which this makes worth more, not less.
pub const TREE_LOD_CAP: usize = 180;

/// The step [`cap_swap`] moves the swap in, metres. Not a knob: a
/// quantization, so a walking eye in a stand does not reband the whole ring
/// on every frame the cap's distance drifts a centimetre. At a sprint's
/// ~5 m/s that is a reband every few frames in a dense stand and none
/// anywhere the cap does not bind, and a reband is `quality::reband_trees`'s
/// three `VisibilityRange` writes per near tree.
pub const TREE_LOD_CAP_STEP_M: f32 = 2.0;

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
/// so both halves come from this one value. **And since forest density v1
/// the bands can sit UNDER the tier** — [`cap_swap`] pulls them in when more
/// than [`TREE_LOD_CAP`] trees would draw their near pair, and lets them back
/// out to the tier when fewer would. `tier_m` is where the tier put them;
/// [`Self::swap_m`] is where they are.
/// `Debug` is deliberately absent: `VisibilityRange` does not implement it,
/// so a derive here would be a wrapper around a type that cannot print. The
/// two `Range<f32>`s inside it print perfectly well and the gates name them
/// individually.
#[derive(Resource, Clone, PartialEq)]
pub struct TreeLod {
    pub near: VisibilityRange,
    pub far: VisibilityRange,
    /// The graphics tier's swap distance, metres — the bands' ceiling.
    /// `quality::apply` writes it with the tier and nothing else does.
    pub tier_m: f32,
}

impl Default for TreeLod {
    /// [`TREE_LOD_SWAP_M`] — the shipped distance, and `Quality::High`'s.
    fn default() -> Self {
        Self::at(TREE_LOD_SWAP_M)
    }
}

impl TreeLod {
    /// The pair for a given swap distance, with that distance as the tier.
    /// The fade width and the reach do not move with it: the fade is how
    /// long a crossfade takes to walk through and the reach is the prop
    /// ring's diagonal, and neither is a function of where the swap happens.
    pub fn at(swap_m: f32) -> Self {
        let (near, far) = Self::bands(swap_m);
        Self {
            near,
            far,
            tier_m: swap_m,
        }
    }

    fn bands(swap_m: f32) -> (VisibilityRange, VisibilityRange) {
        let fade_end = swap_m + TREE_LOD_FADE_M;
        (
            VisibilityRange {
                // No near margin: a tree you stand in is its own geometry.
                start_margin: 0.0..0.0,
                end_margin: swap_m..fade_end,
                use_aabb: false,
            },
            VisibilityRange {
                start_margin: swap_m..fade_end,
                // `use_aabb` stays false on BOTH, which is Bevy's own note
                // about crossfading: the two LODs have different AABBs and a
                // fade needs one distance, so the distance is the shared
                // origin — the same `transform` `spawn_slot` gives every part
                // of a tree.
                end_margin: TREE_LOD_REACH_M..(TREE_LOD_REACH_M + TREE_LOD_FADE_M),
                use_aabb: false,
            },
        )
    }

    /// Move the bands to `swap_m`, clamped to the tier, leaving the tier
    /// where it is. [`cap_swap`]'s write; a no-op when the bands are already
    /// there, so a caller can guard a `ResMut` on the comparison.
    pub fn contract(&mut self, swap_m: f32) {
        let at = swap_m.min(self.tier_m).max(0.0);
        if at != self.swap_m() {
            let (near, far) = Self::bands(at);
            self.near = near;
            self.far = far;
        }
    }

    /// Where the near pair gives way to the hull, metres from the eye — the
    /// start of the near band's end margin, which `tests/tree.rs` holds equal
    /// to the start of the far band's start margin. [`swap_by_distance`]
    /// swaps here with no fade; the desktop fades across `TREE_LOD_FADE_M`
    /// from here. At or under [`Self::tier_m`], never above it.
    pub fn swap_m(&self) -> f32 {
        self.near.end_margin.start
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

/// A tree part's LOD band as the component it carries: the `VisibilityRange`
/// itself on the desktop, and **nothing in a browser**.
///
/// WebGL2 has no storage buffers, so `visibility_ranges` — the table
/// `VisibilityRange` dithers by, view binding 14 — becomes a 1,024-byte
/// uniform array on that target, and Bevy 0.18.1's view layout keeps the
/// storage case's `min_binding_size` of 16 for it. wgpu refuses every pipeline
/// that reads the table (`pbr_opaque_mesh_pipeline` with
/// `VISIBILITY_RANGE_DITHER`), which is the pipeline of the first tree to
/// stream in; no 0.18 point release fixes it
/// (`findings/web-build-20260909.md` §15.6). A pipeline that never touches the
/// binding is untouched, so the browser's answer is to never ask: no tree
/// part carries the component there, and [`swap_by_distance`] toggles
/// `Visibility` by hand instead. Both spawn sites in `props::spawn_slot` go
/// through this one function so the two targets cannot disagree about which
/// parts are banded.
#[cfg(not(target_arch = "wasm32"))]
pub fn lod_band(range: &VisibilityRange) -> VisibilityRange {
    range.clone()
}

/// See the desktop half above: a browser tree carries no `VisibilityRange`.
#[cfg(target_arch = "wasm32")]
pub fn lod_band(_range: &VisibilityRange) {}

/// The count cap: pull the swap in when more than [`TREE_LOD_CAP`] trees
/// would draw their near pair, and let it back out to the tier when fewer
/// would. Both targets, every frame the world runs, before
/// `quality::reband_trees` (which applies the bands it writes) and before
/// [`swap_by_distance`] (which reads them on a browser).
///
/// **The measure is every tree that draws ANY near geometry** — inside the
/// swap and inside the fade past it, where Bevy dithers both LODs and the
/// vertex work is the full pair. So the bound is on distance from the eye
/// to the (`TREE_LOD_CAP` + 1)-th nearest trunk, less the fade: the
/// `TREE_LOD_CAP` trunks nearer than it are the drawn set, exactly. One
/// trunk per tree, and the trunk's origin is the shared pivot every part of
/// the tree measures from (`use_aabb: false`) — the same distance
/// `VisibilityRange` will then evaluate.
///
/// **Allocation-free after warmup** (CLAUDE.md's client hot-path rule): the
/// scratch is a `Local<Vec>` that `clear`s and keeps its capacity, so it
/// grows to the largest near ring it ever sees and never again;
/// `select_nth_unstable_by` is in place. Quantized to
/// [`TREE_LOD_CAP_STEP_M`] and written only on a change, because the write
/// is what `reband_trees` costs.
///
/// **One-frame lag on a freshly streamed chunk, stated rather than hidden.**
/// A part spawned this frame has an identity `GlobalTransform` until
/// `PostUpdate` propagates it, so its trunk reads as standing at the origin
/// — far outside any reach — and is not counted until the next frame. The
/// bound can therefore be exceeded for one frame by one chunk's trees, which
/// is the same one-chunk-a-frame budget the streamer itself pays.
pub fn cap_swap(
    eye: Res<super::Eye>,
    trunks: Query<(&super::props::Fellable, &GlobalTransform)>,
    mut lod: ResMut<TreeLod>,
    mut d2: Local<Vec<f32>>,
) {
    let tier = lod.tier_m;
    let reach = tier + TREE_LOD_FADE_M;
    d2.clear();
    for (f, gt) in trunks.iter() {
        if f.part != super::props::FellPart::Trunk {
            continue;
        }
        let d = (gt.translation() - eye.pos).length_squared();
        if d < reach * reach {
            d2.push(d);
        }
    }
    let want = if d2.len() > TREE_LOD_CAP {
        let (_, nth, _) = d2.select_nth_unstable_by(TREE_LOD_CAP, f32::total_cmp);
        let edge = nth.sqrt() - TREE_LOD_FADE_M;
        (edge / TREE_LOD_CAP_STEP_M).floor() * TREE_LOD_CAP_STEP_M
    } else {
        tier
    };
    let want = want.min(tier).max(0.0);
    // `swap_m` reads through `Deref` and marks nothing; only the write does.
    if want != lod.swap_m() {
        lod.contract(want);
    }
}

/// The LOD swap a browser does by hand — [`lod_band`] says why it has to.
///
/// Every frame in the world, each tree part that would carry a
/// `VisibilityRange` on the desktop is shown or hidden by its distance from
/// the eye: the trunk and canopy inside [`TreeLod::swap_m`], the hull
/// outside it. The distance is the part's own origin, which is the shared
/// pivot `props::spawn_slot` gives all three — the same point the desktop's
/// ranges measure from (`use_aabb: false`). No crossfade: a browser swaps
/// abruptly where the desktop fades over `TREE_LOD_FADE_M`, and that is
/// written here rather than hidden.
///
/// **Only the three banded parts.** The stump and a vanishing prop are the
/// fell system's to show and hide (`props::apply_fell`), which never writes
/// `Visibility` on a trunk, a canopy or a hull — so the two never fight over
/// one entity. Written only on a change, because a `Visibility` write
/// re-extracts the entity.
///
/// Compiled on every target and **gated to wasm32 by its run condition**
/// (`render/mod.rs`), so `tests/tree_swap.rs` can drive it natively.
pub fn swap_by_distance(
    lod: Res<TreeLod>,
    eye: Res<super::Eye>,
    mut parts: Query<(&super::props::Fellable, &GlobalTransform, &mut Visibility)>,
) {
    let swap2 = lod.swap_m() * lod.swap_m();
    for (f, gt, mut vis) in parts.iter_mut() {
        let near_part = match f.part {
            super::props::FellPart::Trunk | super::props::FellPart::Canopy => true,
            super::props::FellPart::Far => false,
            super::props::FellPart::Stump | super::props::FellPart::Vanish => continue,
        };
        let is_near = (gt.translation() - eye.pos).length_squared() < swap2;
        let want = if is_near == near_part {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

/// How much darker the generated canopy is than its own colour ramp — the
/// mean of [`occlude_canopy`]'s factor over the mesh that ships.
///
/// Measured off the canopy rather than recomputed, so the two cannot disagree:
/// whatever `occlude_canopy` does to the near tree, the hull that replaces it
/// wears the same mean, and removing the occlusion returns this to 1.0 without
/// anybody editing a second number. Luma rather than per-channel, because the
/// occlusion is a scalar on all three and a per-channel ratio would only
/// re-derive it three times with more rounding.
fn canopy_mean_shade(
    needles: &Mesh,
    sp: &SpeciesDef,
    h: f32,
    ramp: &dyn Fn(u32, u32, f32, f32, f32) -> [f32; 3],
) -> f32 {
    shade_in_band(needles, sp, h, ramp, 0.0, 1.0)
}

/// [`canopy_mean_shade`], restricted to a slice of the canopy's own height.
///
/// **Split out so a gate can ask the question that the whole-mesh mean cannot
/// answer**: whether the occlusion normalises `r` against the crown's radius at
/// each height or against one global maximum. Those two agree everywhere the
/// crown is at its widest and disagree at the APEX, where a cone is narrow —
/// under a global maximum every vertex up there reads buried and the top of
/// every conifer goes dark, which is the opposite of the truth and is the one
/// thing [`occlude_canopy`]'s doc comment promises not to do. A mutant proved
/// the promise was unchecked (`tests/tree.rs`, 2026-09-16) before this existed.
fn shade_in_band(
    needles: &Mesh,
    sp: &SpeciesDef,
    h: f32,
    ramp: &dyn Fn(u32, u32, f32, f32, f32) -> [f32; 3],
    lo_frac: f32,
    hi_frac: f32,
) -> f32 {
    let (Some(p), Some(VertexAttributeValues::Float32x4(c))) =
        (positions(needles), needles.attribute(Mesh::ATTRIBUTE_COLOR))
    else {
        return 1.0;
    };
    if p.len() != c.len() || p.is_empty() {
        return 1.0;
    }
    let (lo_y, hi_y, _) = crown_extent(p);
    let span = (hi_y - lo_y).max(f32::EPSILON);
    let luma = |v: [f32; 3]| 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
    let mut got = 0.0f64;
    let mut want = 0.0f64;
    for (v, col) in p.iter().zip(c.iter()) {
        let f = (v[1] - lo_y) / span;
        if f < lo_frac || f > hi_frac {
            continue;
        }
        got += luma([col[0], col[1], col[2]]) as f64;
        want += luma(ramp(
            sp.leaf_lo,
            sp.leaf_hi,
            h * LEAF_BAND_BASE_FRAC,
            h,
            v[1],
        )) as f64;
    }
    if want <= 1e-9 {
        return 1.0;
    }
    (got / want) as f32
}

/// The mean occlusion `occlude_canopy` actually applied over a slice of one
/// variant's crown, `0.0..=1.0` of its own height — **measured off the mesh
/// that ships**, so a gate reading it is reading the frame's own numbers and
/// not a second implementation of the law.
///
/// The ramp is rebuilt here because it is `band`'s and `band` bakes it into the
/// colour; that is not the thing under test, and re-deriving the OCCLUSION
/// would be (`CLAUDE.md`: a naive rebuild that calls the function under test).
pub fn canopy_shade(variant: usize, lo_frac: f32, hi_frac: f32) -> f32 {
    let sp = &SPECIES[species_of(variant)];
    let (_, needles) = conifer(variant);
    let ramp = |lo: u32, hi: u32, y0: f32, y1: f32, y: f32| {
        let (a, b) = (linear(lo), linear(hi));
        let t = ((y - y0) / (y1 - y0).max(f32::EPSILON)).clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    shade_in_band(&needles, sp, sp.height_m, &ramp, lo_frac, hi_frac)
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
    // **The shade the near tree's canopy actually wears, measured off it.**
    //
    // `occlude_canopy` darkens the real canopy and this hull is rebuilt from
    // the ramps rather than from those vertices, so without this the hull is
    // the brighter of the two and the swap is a colour POP — the exact failure
    // [`BARK_BAND_TOP_FRAC`] exists to name, arriving through a different door.
    // A closed hull has no interior to occlude, so the honest answer is not to
    // re-derive the depth term but to wear the canopy's MEAN: at the swap
    // distance a real crown reads as its lit shell averaged with the dark it
    // shows between needles, and that average is what an opaque silhouette
    // standing in for it should be.
    //
    // Derived, never typed: the ratio of the canopy mesh's own mean vertex
    // luma to the same ramp evaluated at the same heights. It is 1.0 by
    // construction if the occlusion is ever removed.
    let canopy_k = canopy_mean_shade(needles, sp, h, &ramp);
    let color = |v: Vec3| {
        let wood = ramp(BARK_LO, BARK_HI, 0.0, h * BARK_BAND_TOP_FRAC, v.y);
        let lit = ramp(sp.leaf_lo, sp.leaf_hi, h * LEAF_BAND_BASE_FRAC, h, v.y);
        let leaf = [lit[0] * canopy_k, lit[1] * canopy_k, lit[2] * canopy_k];
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
        // Flat facets here; the blend toward the trunk axis is applied to the
        // finished mesh below, per VERTEX, by the canopy's own
        // `blend_canopy_normals`. ⚠ It used to be done per band, through
        // `Soup::tri`'s `volume_center` set to the band's mid-height on the
        // axis — so a band's bottom ring tilted its normals down and its top
        // ring tilted them up, every ring flipped, and on the bench
        // (2026-09-14, `examples/tree_look.rs`) the hull shaded as a stack of
        // discs: exactly the "not a tree" the tip-closing below exists to
        // avoid, from the shading rather than the outline. A horizontal
        // radial has no tilt to flip.
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
                s.tri(b0, t0, b1, color, None, 0.0);
            }
            if r1 > f32::EPSILON {
                s.tri(b1, t0, t1, color, None, 0.0);
            }
        }
    }
    let mut hull = s.mesh();
    blend_canopy_normals(&mut hull);
    hull
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
