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
    /// The trunk's two colours as they should READ, sRGB: the far hull wears
    /// them as they are, and the near trunk's mean-1 band takes their hue over
    /// the species' bark photo (`props::PropAssets::trunk_material`).
    pub bark_lo: u32,
    pub bark_hi: u32,
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
        bark_lo: BARK_LO,
        bark_hi: BARK_HI,
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
        // A birch's pale trunk: the near trunk wears a birch photograph
        // (`textures::PropMaps::birch`) and the far hull these greys.
        bark_lo: BIRCH_BARK_LO,
        bark_hi: BIRCH_BARK_HI,
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
/// The broadleaf's trunk as it should read: a pale birch grey (the near
/// trunk's band is mean-1, so only the far hull sees the two differ).
const BIRCH_BARK_LO: u32 = 0x8a867c;
const BIRCH_BARK_HI: u32 = 0xa29e93;
/// The broadleaf's two greens (each lobe then leans a little yellow or blue,
/// [`Lobes`]). Yellower and lighter than the conifer's, which
/// is the other half of telling the two apart at range — `ART.md` §5 asks for
/// two greens minimum per canopy, and a second SPECIES wearing the first one's
/// palette would read as the same tree at a different size.
const BROADLEAF_LO: u32 = 0x324f1e;
const BROADLEAF_HI: u32 = 0x6e9138;

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
/// **Generated as the crate's `Evergreen` type, though the species is a
/// broadleaf**, because the crate's `Deciduous` trunk is three end-to-end
/// pieces and only the lowest carries full limbs: the middle one gets bare
/// twigs and the top one a few leaves on the stem. Measured by
/// `examples/tree_sweep.rs` (2026-10-05), its upper crown held 13–19 % of the
/// lower's cards and the crown read as two or three stacked pom-poms at 18 m.
/// `Evergreen` is one continuous trunk with limbs all the way up, shortening
/// toward the top; with these limbs the upper half holds 66–100 % of the
/// lower's cards and the crown is one mass. `SPECIES` still calls it
/// `Deciduous`, which is what picks this block, the leaf card and
/// [`shape_broadleaf`].
///
/// - `levels: Two`: limbs carrying sub-branches carrying the cards. 20 × 4 is
///   80 terminal branches, ~4.9 k triangles with the bark, inside
///   `CONIFER_MAX_TRIS`.
/// - `angle` 36° / 32°: steep, a birch's limbs leave the trunk steeply, and
///   at 11 m the 2.9 m crown ceiling is a column, not a dome.
/// - `force.direction` is `Vec3::Y` and **must be**, for exactly the reason
///   the conifer's block gives at length: a downward direction hits the
///   antipodal singularity in `Quat::from_rotation_arc` and bends the whole
///   tree sideways. Droop is the limb ANGLE's job in both species.
fn broadleaf_settings() -> TreeMeshSettings {
    TreeMeshSettings {
        tree_type: TreeType::Evergreen,
        branch: BranchParams {
            levels: BranchRecursionLevel::Two,
            angle: [0.0, 36.0, 32.0, 0.0],
            children: [20, 4, 0],
            force: BranchForce {
                direction: Vec3::Y,
                strength: 0.04,
                radius_cutoff: 0.1,
            },
            gnarliness: [-0.04, 0.14, 0.10, 0.0],
            // [0] is proportion only — `fit_to_bounds` normalises height away.
            // [1] and [2] set the crown's WIDTH, which is the sim's business
            // through `BROADLEAF_MAX_R`: the sweep reads 2.41 m on the shipped
            // seeds and 2.46 over fifteen against the 2.9 ceiling. The trunk
            // reads 0.233 m at the base, inside the sim's cylinder, which the
            // conifer sets (`OCCUPANT_R_M[Tree]`).
            length: [4.5, 0.9, 0.25, 0.0],
            trunk_base_radius: 0.11,
            radius_factor: [1.0, 0.42, 0.34, 0.0],
            sections: [10, 4, 2, 0],
            segments: [7, 4, 3, 0],
            // Limbs from 24 % of the trunk (2.6 m): clear of head height, so
            // the stand is not a colonnade of bare poles at eye level.
            start: [0.0, 0.24, 0.1, 0.0],
            // An `Evergreen` trunk tapers to `1 - taper` of its base over
            // its own length: a fifth at the top, a slender birch.
            taper: [0.8, 0.82, 0.85, 0.0],
            twist: [0.06, -0.05, 0.0, 0.0],
        },
        leaves: LeafParams {
            leaf_billboard: LeafBillboard::Double,
            angle: 48.0,
            count: 10,
            start: 0.0,
            // A card's size adds to the crown radius directly, and the
            // ceiling did not grow with the height.
            size: 0.60,
            size_variance: 0.35,
        },
    }
}

/// The parameter block for a species index. Public so `examples/tree_sweep.rs`
/// sweeps candidates as edits of what ships rather than a restated copy.
pub fn settings(species: usize) -> TreeMeshSettings {
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
/// canopy passes `false` — both cards are mean-normalised photos
/// ([`NEEDLE_MAP_GAIN`], [`LEAF_MAP_GAIN`]), so the vertex colour is the
/// canopy's colour.
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
///
/// A broadleaf passes its [`Lobes`]: each vertex's field is then mostly its
/// own lobe's outward normal (centred a little low, so every lobe has a lit
/// top), with the crown's ellipsoid underneath. One ellipsoid over a whole
/// broadleaf shaded it as one smooth ball — the lollipop read.
fn blend_canopy_normals(m: &mut Mesh, lobes: Option<&Lobes>) {
    let Some(p) = positions(m) else { return };
    let (lo_y, hi_y, r_max) = crown_extent(p);
    let c_y = (lo_y + hi_y) * 0.5;
    // Semi-axes. Floored so a degenerate canopy divides by something.
    let sy = ((hi_y - lo_y) * 0.5).max(1e-3);
    let sr = r_max.max(1e-3);
    let (iy2, ir2) = (1.0 / (sy * sy), 1.0 / (sr * sr));
    let outward: Vec<[f32; 3]> = p
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let e = Vec3::new(v[0] * ir2, (v[1] - c_y) * iy2, v[2] * ir2);
            // Dead centre of the crown has no outward direction; up is the
            // only defensible answer and it affects a handful of vertices.
            let crown = e.normalize_or(Vec3::Y);
            let Some(l) = lobes else {
                return crown.to_array();
            };
            let k = l.of[i] as usize;
            let c = l.centre[k] - Vec3::Y * (LOBE_NORMAL_DROP * l.reach[k]);
            let lobe = (Vec3::from_array(*v) - c).normalize_or(crown);
            (crown * (1.0 - LOBE_NORMAL_SHARE) + lobe * LOBE_NORMAL_SHARE)
                .normalize_or(crown)
                .to_array()
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
///
/// A broadleaf's exposure is also scaled by how deep inside its own LOBE a
/// vertex sits ([`LOBE_AO`]), so each lobe reads as a lit shell over a dark
/// core rather than the whole crown as one; and each lobe wears its own value
/// and a touch of hue ([`Lobes`]).
fn occlude_canopy(m: &mut Mesh, lobes: Option<&Lobes>) {
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
        .enumerate()
        .map(|(i, v)| {
            let r = (v[0] * v[0] + v[2] * v[2]).sqrt();
            let rr = radius_at(v[1]);
            // Exposure: 0 buried on the axis, 1 out at the crown's surface.
            let e = if rr <= f32::EPSILON {
                1.0
            } else {
                (r / rr).clamp(0.0, 1.0)
            };
            // The top of the crown is open to the sky whatever its radius:
            // a conifer's spire is a few small cards about the leader, all
            // "near the axis", and it is the most exposed part of the tree.
            let open = (((v[1] - lo_y) / span - 0.82) / 0.18).clamp(0.0, 1.0);
            let mut e = e;
            if let Some(l) = lobes {
                let k = l.of[i] as usize;
                let d = (Vec3::from_array(*v) - l.centre[k]).length();
                let inner = (d / l.reach[k].max(1e-3)).clamp(0.0, 1.0);
                e *= 1.0 - LOBE_AO * (1.0 - inner);
            }
            // Open sky last, so a lobe's dark heart cannot bury the apex.
            let e = e.max(open);
            CANOPY_AO_FLOOR + (1.0 - CANOPY_AO_FLOOR) * e.powf(CANOPY_AO_GAMMA)
        })
        .collect();

    let Some(VertexAttributeValues::Float32x4(cols)) = m.attribute_mut(Mesh::ATTRIBUTE_COLOR)
    else {
        return;
    };
    for (i, (c, k)) in cols.iter_mut().zip(shade).enumerate() {
        let t = lobes.map_or([1.0; 3], |l| l.tint[l.of[i] as usize]);
        c[0] *= k * t[0];
        c[1] *= k * t[1];
        c[2] *= k * t[2];
    }
}

/// Vertical spacing of a conifer's whorls, metres. A pine puts its limbs out
/// in tiers, one a year, and the tiers are what make it read as a pine rather
/// than a cypress at any range: the generator scatters limbs evenly up the
/// trunk, so [`shape_crown`] makes the tiers out of the needle cards.
pub const WHORL_M: f32 = 1.6;
/// How big a needle card is between two whorls, as a share of one in a whorl.
pub const WHORL_GAP_SCALE: f32 = 0.45;
/// How big a needle card is at the apex, as a share of one at the crown's foot
/// — the spire. The limbs already shorten upward (the generator scales an
/// evergreen's limb by how far up the trunk it starts), but a ~1 m card on
/// every limb tip kept the crown a column ~2 m across all the way up.
pub const APEX_SCALE: f32 = 0.35;

/// Shape a conifer's needle cards into tiers and a spire.
///
/// Each card is scaled about its own attach point (the middle of its base
/// edge, where the generator roots it on the limb) by the crown's taper and a
/// whorl pulse up the height. Cards only ever shrink, so the crown stays
/// inside the radius the sim's bounds were swept against.
fn shape_crown(needles: &mut Mesh, variant: usize) {
    let Some(p) = positions_mut(needles) else {
        return;
    };
    if p.len() % 4 != 0 {
        return;
    }
    // The generator's quad: (-w, size), (-w, 0), (w, 0), (w, size) — corners
    // 1 and 2 are the base edge.
    let root = |q: &[[f32; 3]]| {
        Vec3::new(
            (q[1][0] + q[2][0]) * 0.5,
            (q[1][1] + q[2][1]) * 0.5,
            (q[1][2] + q[2][2]) * 0.5,
        )
    };
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for q in p.chunks_exact(4) {
        let y = root(q).y;
        lo = lo.min(y);
        hi = hi.max(y);
    }
    let span = (hi - lo).max(1e-3);
    // Each seed's whorls sit at its own heights.
    let phase0 = hash01(0x3b0e_71a5, variant as u32);
    for q in p.chunks_exact_mut(4) {
        let r = root(q);
        let t = ((r.y - lo) / span).clamp(0.0, 1.0);
        let cone = 1.0 - (1.0 - APEX_SCALE) * t;
        // 1 at a whorl, falling to the gap between two: a raised cosine,
        // flattened on top so a whorl is a band and not a line.
        let ph = ((r.y - lo) / WHORL_M + phase0).fract();
        let w = (0.5 + 0.5 * (ph * std::f32::consts::TAU).cos()).sqrt();
        let s = cone * (WHORL_GAP_SCALE + (1.0 - WHORL_GAP_SCALE) * w);
        for v in q.iter_mut() {
            v[0] = r.x + (v[0] - r.x) * s;
            v[1] = r.y + (v[1] - r.y) * s;
            v[2] = r.z + (v[2] - r.z) * s;
        }
    }
}

/// Lobes a broadleaf crown is gathered into. A broadleaf's crown is a
/// handful of leaf masses with air between them; the generator spreads its
/// cards evenly over the whole envelope, which reads as one smooth ball.
pub const BROADLEAF_LOBES: usize = 14;
/// How far each leaf card is pulled toward its lobe's centre, as a share of
/// the way (each lobe scales it by 0.5-1.5). Tightens the lobes and opens
/// gaps between them; at 0.2 the gaps already read as stacked pom-poms.
pub const LOBE_PULL: f32 = 0.12;
/// How much a card at a lobe's rim shrinks, against one at its heart.
pub const LOBE_EDGE_SHRINK: f32 = 0.28;
/// How big a card is at the foot of the crown, against one above it: a
/// broadleaf's lowest limbs carry little, so the crown lifts off the trunk.
pub const CROWN_FOOT_SCALE: f32 = 0.5;
/// How big a card is at the very top of the crown, against one below it, so
/// the crown rounds off rather than ending flat on its full width.
pub const CROWN_TOP_SCALE: f32 = 0.65;
/// How far each card moves toward the height an evenly filled crown would
/// give it ([`shape_broadleaf`]): 0 leaves the generator's bunching, 1 is a
/// crown of uniform density.
pub const CROWN_EVEN: f32 = 0.6;
/// How much sparser the evened crown is at its top than at its foot: the
/// crown narrows upward, and the same card count at every height packed its
/// top into a dense cap whose inside `occlude_canopy` rightly darkens — a lit
/// apex reading as buried (`tests/tree.rs`, the crown-profile gate). In
/// (0, 1); 0.5 halves the density by the top.
pub const CROWN_EVEN_TAPER: f32 = 0.5;
/// How much height counts against horizontal distance when the lobes are
/// found. Under 1, so a lobe is taller than it is wide and the lobes run up
/// the crown rather than stacking in tiers of balls.
const LOBE_Y_WEIGHT: f32 = 0.5;
/// The share of a lobed canopy normal that is its lobe's rather than the
/// crown's ([`blend_canopy_normals`]).
const LOBE_NORMAL_SHARE: f32 = 0.4;
/// How far below a lobe's centre its normals radiate from, as a share of its
/// reach: every lobe faces a little up, so each has a lit top.
const LOBE_NORMAL_DROP: f32 = 0.3;
/// How much darker a lobe's heart is than its shell ([`occlude_canopy`]).
const LOBE_AO: f32 = 0.35;

/// Which lobe each canopy vertex belongs to, and each lobe's centre, reach
/// (its farthest vertex from the centre) and tint. Measured off the finished
/// mesh, so normals and occlusion agree about where the lobes are.
struct Lobes {
    of: Vec<u16>,
    centre: Vec<Vec3>,
    reach: Vec<f32>,
    tint: Vec<[f32; 3]>,
}

impl Lobes {
    fn measure(m: &Mesh, of: Vec<u16>, variant: usize) -> Option<Self> {
        let p = positions(m)?;
        let k = of.iter().copied().max()? as usize + 1;
        if of.len() != p.len() {
            return None;
        }
        let mut sum = vec![Vec3::ZERO; k];
        let mut n = vec![0u32; k];
        for (v, &l) in p.iter().zip(&of) {
            sum[l as usize] += Vec3::from_array(*v);
            n[l as usize] += 1;
        }
        let centre: Vec<Vec3> = sum
            .iter()
            .zip(&n)
            .map(|(s, &c)| *s / c.max(1) as f32)
            .collect();
        let mut reach = vec![0.0f32; k];
        for (v, &l) in p.iter().zip(&of) {
            let d = (Vec3::from_array(*v) - centre[l as usize]).length();
            reach[l as usize] = reach[l as usize].max(d);
        }
        // Each lobe its own value, and a lean toward yellow or toward blue:
        // leaf masses turned to the sun differently never match.
        let tint = (0..k as u32)
            .map(|i| {
                let v = 0.86 + 0.26 * hash01(0x10be_0001 ^ variant as u32, i);
                let y = hash01(0x10be_0002 ^ variant as u32, i) - 0.5;
                [v * (1.0 + 0.16 * y), v, v * (1.0 - 0.24 * y)]
            })
            .collect();
        Some(Self {
            of,
            centre,
            reach,
            tint,
        })
    }
}

/// Even a broadleaf's leaf cards out up the crown, gather them into
/// [`BROADLEAF_LOBES`] lobes, lift the crown off the trunk and round its top.
/// Returns each VERTEX's lobe.
///
/// Lobes are a few rounds of k-means over the cards' attach points, seeded by
/// farthest-point sampling from a per-variant start, so each seed lobes
/// differently. Each card is pulled toward its lobe's centre and scaled about
/// its attach point, shrinking at the lobe's rim, the crown's foot and its
/// top. Cards move vertically or toward the inside of the crown and only
/// shrink, and any card that would still poke past the crown's original
/// radius is shrunk until it does not: the radius the sim's bounds were swept
/// against holds.
fn shape_broadleaf(needles: &mut Mesh, variant: usize) -> Option<Vec<u16>> {
    let p = positions_mut(needles)?;
    if p.len() % 4 != 0 || p.is_empty() {
        return None;
    }
    let horiz = |v: &[f32; 3]| (v[0] * v[0] + v[2] * v[2]).sqrt();
    let r0 = p.iter().map(horiz).fold(0.0f32, f32::max);
    // The generator's quad: corners 1 and 2 are the base edge.
    let root = |q: &[[f32; 3]]| {
        Vec3::new(
            (q[1][0] + q[2][0]) * 0.5,
            (q[1][1] + q[2][1]) * 0.5,
            (q[1][2] + q[2][2]) * 0.5,
        )
    };
    let mut roots: Vec<Vec3> = p.chunks_exact(4).map(root).collect();
    let cards = roots.len();
    let k = BROADLEAF_LOBES.min(cards);

    // Even the cards out up the crown ([`CROWN_EVEN`]) before anything else:
    // each card moves part of the way to where its height rank would put it
    // if the crown were evenly filled. The generator hangs its limbs at
    // random heights, so a few bunch and leave a bare band, and a bare band
    // between two leafy ones is a waist — the stacked-pom-pom read. Vertical
    // only, so no card's radius changes.
    let mut order: Vec<usize> = (0..cards).collect();
    order.sort_by(|&a, &b| roots[a].y.total_cmp(&roots[b].y).then(a.cmp(&b)));
    let (lo, hi) = (roots[order[0]].y, roots[order[cards - 1]].y);
    let span = (hi - lo).max(1e-3);
    // The even crown thins linearly toward its top ([`CROWN_EVEN_TAPER`]):
    // its card density at height fraction `t` is `1 - taper·t`, and a card's
    // rank fraction `u` inverts that density's integral.
    let taper = CROWN_EVEN_TAPER;
    for (rank, &c) in order.iter().enumerate() {
        let u = (rank as f32 + 0.5) / cards as f32;
        let t = (1.0
            - (1.0 - 2.0 * taper * u * (1.0 - 0.5 * taper))
                .max(0.0)
                .sqrt())
            / taper;
        let even = lo + span * t;
        let lift = CROWN_EVEN * (even - roots[c].y);
        roots[c].y += lift;
        for v in &mut p[c * 4..c * 4 + 4] {
            v[1] += lift;
        }
    }

    // Clustered with height squashed ([`LOBE_Y_WEIGHT`]), so lobes run up
    // the crown rather than stacking in tiers.
    let keys: Vec<Vec3> = roots
        .iter()
        .map(|r| Vec3::new(r.x, r.y * LOBE_Y_WEIGHT, r.z))
        .collect();
    // Seeds: farthest-point sampling from a hashed start.
    let first = (hash2(0x10be_5eed, variant as u32) as usize) % cards;
    let mut centre = vec![keys[first]];
    let mut d2: Vec<f32> = keys.iter().map(|r| r.distance_squared(centre[0])).collect();
    while centre.len() < k {
        let far = (0..cards).max_by(|&a, &b| d2[a].total_cmp(&d2[b]))?;
        let c = keys[far];
        centre.push(c);
        for (d, r) in d2.iter_mut().zip(&keys) {
            *d = d.min(r.distance_squared(c));
        }
    }
    // A few k-means rounds, so the lobes are masses rather than hull points.
    let nearest = |r: &Vec3, centre: &[Vec3]| {
        (0..centre.len())
            .min_by(|&a, &b| {
                r.distance_squared(centre[a])
                    .total_cmp(&r.distance_squared(centre[b]))
            })
            .unwrap_or(0)
    };
    let mut of = vec![0usize; cards];
    for _ in 0..4 {
        for (o, r) in of.iter_mut().zip(&keys) {
            *o = nearest(r, &centre);
        }
        let mut sum = vec![Vec3::ZERO; k];
        let mut n = vec![0u32; k];
        for (&o, r) in of.iter().zip(&keys) {
            sum[o] += *r;
            n[o] += 1;
        }
        for ((c, s), &n) in centre.iter_mut().zip(&sum).zip(&n) {
            if n > 0 {
                *c = *s / n as f32;
            }
        }
    }
    // Back to real space: each lobe's centre is its attach points' mean.
    for c in centre.iter_mut() {
        c.y /= LOBE_Y_WEIGHT;
    }
    // Each lobe's typical radius: the mean attach-point distance from its
    // centre.
    let mut rad = vec![0.0f32; k];
    let mut n = vec![0u32; k];
    for (&o, r) in of.iter().zip(&roots) {
        rad[o] += r.distance(centre[o]);
        n[o] += 1;
    }
    for (r, &n) in rad.iter_mut().zip(&n) {
        *r = (*r / n.max(1) as f32).max(0.05);
    }
    // Some lobes are fuller than others, and some tighter.
    let size: Vec<f32> = (0..k as u32)
        .map(|i| 0.82 + 0.18 * hash01(0x10be_0003 ^ variant as u32, i))
        .collect();
    let pull: Vec<f32> = (0..k as u32)
        .map(|i| 0.5 + hash01(0x10be_0004 ^ variant as u32, i))
        .collect();

    for (ci, q) in p.chunks_exact_mut(4).enumerate() {
        let r = roots[ci];
        let o = of[ci];
        let to = r + (centre[o] - r) * (LOBE_PULL * pull[o]);
        let edge = smooth01((r.distance(centre[o]) / rad[o] - 0.6) / 0.9);
        let t = (r.y - lo) / span;
        let foot = smooth01(t / 0.3);
        let top = smooth01((t - 0.75) / 0.25);
        let mut sc = size[o]
            * (1.0 - LOBE_EDGE_SHRINK * edge)
            * (CROWN_FOOT_SCALE + (1.0 - CROWN_FOOT_SCALE) * foot)
            * (1.0 - (1.0 - CROWN_TOP_SCALE) * top);
        let orig: [[f32; 3]; 4] = [q[0], q[1], q[2], q[3]];
        let place = |sc: f32, out: &mut [[f32; 3]]| {
            for (v, o) in out.iter_mut().zip(&orig) {
                *v = (to + (Vec3::from_array(*o) - r) * sc).to_array();
            }
        };
        place(sc, q);
        for _ in 0..16 {
            if q.iter().map(horiz).fold(0.0f32, f32::max) <= r0 {
                break;
            }
            sc *= 0.85;
            place(sc, q);
        }
    }
    Some(of.iter().flat_map(|&o| [o as u16; 4]).collect())
}

/// `smoothstep(0, 1, x)`.
fn smooth01(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One conifer: `(bark, needles)`, fitted, banded and shaded.
///
/// Deterministic in `variant` and nothing else — same variant, same tree, on
/// every client and every run. That is what lets a chunk stream out and back
/// bit-identical, the same law the whorl builder's hashes carried.
pub fn conifer(variant: usize) -> (Mesh, Mesh) {
    let species = species_of(variant);
    grow(species, variant, &settings(species))
}

/// [`conifer`] with the parameter block passed in: `species`' height, bark
/// and post-passes over `s`, seeded by `variant`. `examples/tree_sweep.rs`
/// measures candidate blocks through it, so a candidate is judged on the
/// finished tree and not on the generator's raw output.
pub fn grow(species: usize, variant: usize, s: &TreeMeshSettings) -> (Mesh, Mesh) {
    let sp = &SPECIES[species];
    // Seeded off the same mixer the rest of `props.rs` uses rather than the
    // raw index, so variant 0 and variant 1 are not neighbouring PRNG streams.
    let mut rng = fastrand::Rng::with_seed(hash2(0x9e37_79b9, variant as u32) as u64);
    let (mut bark, mut needles) = match generate_tree_meshes(s, &mut rng) {
        Ok(pair) => pair,
        // A generator failure must not be a black screen. The only
        // documented error is index overflow, which `u32_indices` makes
        // unreachable at these counts — but "unreachable" is not
        // "impossible", and an empty pair draws nothing rather than
        // panicking a client at boot.
        Err(_) => (Mesh::from(Cuboid::default()), Mesh::from(Cuboid::default())),
    };

    fit_to_bounds(&mut bark, &mut needles, sp.height_m);
    let lobe_of = match sp.tree_type {
        TreeType::Evergreen => {
            shape_crown(&mut needles, variant);
            None
        }
        TreeType::Deciduous => shape_broadleaf(&mut needles, variant),
    };
    // Both shapings shrink the top cards and lower the apex; stretch the pair
    // back up to its height, vertically only, so neither the trunk's radius
    // (the sim's cylinder) nor the crown's grows.
    let (h, _) = bounds(&[&bark, &needles]);
    if h > f32::EPSILON {
        let k = sp.height_m / h;
        for m in [&mut bark, &mut needles] {
            if let Some(p) = positions_mut(m) {
                for v in p.iter_mut() {
                    v[1] *= k;
                }
            }
        }
    }
    let lobes = lobe_of.and_then(|of| Lobes::measure(&needles, of, variant));
    band(
        &mut bark,
        sp.bark_lo,
        sp.bark_hi,
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
    occlude_canopy(&mut needles, lobes.as_ref());
    blend_canopy_normals(&mut needles, lobes.as_ref());
    (bark, needles)
}

/// The conifer needle card: a composed pine branch end from Poly Haven's CC0
/// `pine_tree_01` twig maps, baked by `ci/bake_needle_card.py`.
///
/// Compiled in rather than loaded, because `far_trees::bake_atlas` rasterises
/// the real tree through this card at boot, before any asset could arrive.
pub const NEEDLE_CARD_PNG: &[u8] =
    include_bytes!("../../../../assets/textures/needle_card_albedo.png");

/// What the needle card's RGB is multiplied by to be mean-1.
///
/// The bake divides the photo by its own linear mean and scales it by
/// `1 / NEEDLE_MAP_GAIN` so its brightest texels fit in a byte; the canopy's
/// colour stays in the vertex bands ([`band`]) and the photograph adds only its
/// variation. `tests/tree.rs` holds this against the shipped file.
pub const NEEDLE_MAP_GAIN: f32 = 4.8589;

/// The broadleaf leaf card: birch-shaped leaves from ambientCG's CC0
/// `LeafSet014`/`LeafSet001`, hung in clusters on a composed twig spray by
/// `ci/bake_leaf_card.py`. Compiled in for [`NEEDLE_CARD_PNG`]'s reason.
pub const LEAF_CARD_PNG: &[u8] = include_bytes!("../../../../assets/textures/leaf_card_albedo.png");

/// [`NEEDLE_MAP_GAIN`] for the leaf card: what its RGB is multiplied by to be
/// mean-1. `ci/bake_leaf_card.py` prints it; `tests/tree.rs` holds it.
pub const LEAF_MAP_GAIN: f32 = 2.7338;

/// What a canopy card's specular is scaled by (`StandardMaterial::
/// specular_tint`; `reflectance` stays the physical 4 %).
///
/// A card stands in for a clump of needles whose specular is mostly shadowed
/// by the clump itself. Lit as one plane, a back-lit card's grazing Fresnel
/// sheened whole cards sky-grey or sun-tan — the pale patches on every near
/// crown in the 2026-10 frames. 0.3 is a reflectance of 0.15 (F0 ≈ 0.4 %).
pub const CANOPY_SPECULAR_TINT: f32 = 0.3;

/// The alpha card the conifer canopy is made of: the shipped photograph,
/// decoded, with a coverage-preserving mip chain.
pub fn needle_image() -> Image {
    photo_card(NEEDLE_CARD_PNG)
}

/// The alpha card a broadleaf canopy is made of: the shipped leaf photograph,
/// decoded, with the same coverage-preserving chain as [`needle_image`].
///
/// A species is a silhouette AND a card. The leaf card used to be generated
/// here (white ovals stamped on twiglets), and on every frame it read as a
/// smooth yellow-green mass: a mask with no value in it carries no light and
/// dark, so the crown's only shading was the volume normal, which is a ball.
/// The photo carries each leaf's own veins and edge and a spread of leaf
/// values, and the card's twigs leave air between the clusters.
pub fn leaf_image() -> Image {
    photo_card(LEAF_CARD_PNG)
}

/// A shipped card PNG decoded into a finished alpha card. A broken embed must
/// not take the client down, so it falls back to a plain disc card rather
/// than panicking (`tests/tree.rs` decodes both shipped cards).
fn photo_card(png: &[u8]) -> Image {
    let decoded = Image::from_buffer(
        png,
        bevy::image::ImageType::Extension("png"),
        bevy::image::CompressedImageFormats::NONE,
        true,
        bevy::image::ImageSampler::Default,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let rgba = decoded.ok().and_then(|img| {
        let w = img.texture_descriptor.size.width;
        let h = img.texture_descriptor.size.height;
        let data = img.data?;
        (w == h && data.len() == (w * h * 4) as usize).then_some((data, w))
    });
    match rgba {
        Some((data, w)) => alpha_card(data, w),
        None => {
            const N: u32 = 64;
            let data = (0..N * N)
                .flat_map(|i| {
                    let (x, y) = ((i % N) as f32 + 0.5, (i / N) as f32 + 0.5);
                    let d = ((x - 32.0).powi(2) + (y - 32.0).powi(2)).sqrt();
                    [255, 255, 255, if d < 26.0 { 255 } else { 0 }]
                })
                .collect();
            alpha_card(data, N)
        }
    }
}

/// A finished alpha card from a level-0 RGBA buffer: the coverage-preserved
/// mip chain, the descriptor that carries it, and the trilinear sampler.
/// Shared by both species' cards, because a second card that built its own
/// chain would be the first place the two drifted.
pub fn alpha_card(data: Vec<u8>, size: u32) -> Image {
    // Levels 1..n, coverage-preserved: an alpha-tested card draws the share of
    // texels over the cut, and a plain box filter loses that share at every
    // level (`mipmap::Filter::Mask`).
    let chained = super::mipmap::chain(&data, size, size, super::mipmap::Filter::Mask);
    let mip_level_count = super::mipmap::levels(size, size);

    // **Constructed at level 0, then given the chain.** `Image::new` asserts
    // `data.len() == width · height · block_size` — it describes one mip and
    // has no parameter for a chain — so handing it the concatenated buffer
    // panics inside the constructor rather than failing a gate. The descriptor
    // and the buffer are both plain fields, so the chain is installed after.
    let mut img = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        // The canopy's COLOUR comes from the mesh's vertex bands; the map is
        // a mean-normalised photo.
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(chained);
    img.texture_descriptor.mip_level_count = mip_level_count;
    // Trilinear across the chain, tiling off: the card's UVs are 0..1 and a
    // wrapped mask bleeds the far edge into the near. (Spelled out rather
    // than `ImageSamplerDescriptor::linear()` so it survives the next upgrade.)
    //
    // Anisotropic, at the grass atlas's 4 (`textures::atlas`): most canopy
    // cards are seen at an angle, and at 1 the mip is picked by the card's
    // long axis on screen, so a tilted card drew from a level two or three
    // blurrier than its face needed and its leaves fused into a smear.
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        anisotropy_clamp: 4,
        ..default()
    });
    img
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
            super::props::FellPart::Stump
            | super::props::FellPart::Vanish
            | super::props::FellPart::Emptied => None,
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
/// `Visibility` on a trunk or a canopy, and hides a felled tree's card — so
/// this keeps a felled card hidden rather than fighting it. Written only on a
/// change, because a `Visibility` write re-extracts the entity.
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
            super::props::FellPart::Stump
            | super::props::FellPart::Vanish
            | super::props::FellPart::Emptied => continue,
        };
        let is_near = (gt.translation() - eye.pos).length_squared() < swap2;
        // A felled tree's card is the fell system's to hide (`apply_fell`).
        let want = if f.felled && !near_part {
            Visibility::Hidden
        } else if is_near == near_part {
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
        let wood = ramp(sp.bark_lo, sp.bark_hi, 0.0, h * BARK_BAND_TOP_FRAC, v.y);
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
    blend_canopy_normals(&mut hull, None);
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
