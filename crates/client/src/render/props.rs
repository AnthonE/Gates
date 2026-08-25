//! Scatter: what stands on the ground. Placement is `sim_core::terrain::
//! scatter` — the same cell draw the server resolves and the browser drew —
//! so this file owns only meshes and materials.
//!
//! **Silhouette before surface** (`ART.md` rule 6). A pine in the references
//! is tall, thin and ragged-edged, and a smooth cone is wrong at any texture
//! budget.
//!
//! **The pine is generated now — `tree::conifer`, not the whorl builder
//! below.** That file carries the argument; the short version is the one
//! `web/src/props.js` reached after three passes of stacking cones: a
//! conifer's canopy is made of alpha cards and an opaque hull cannot get there
//! from any amount of geometry. `pine_mesh` stays because a billboard bake
//! still wants a cheap opaque silhouette to render from — but it is **no
//! longer the far LOD's starting point**, which it was until 2026-08-20:
//! `tree::impostor_of` lathes that hull out of the generated tree's own
//! vertices instead, per variant and per species, so it cannot drift from the
//! tree it replaces and a broadleaf does not wear a conifer's cone. (This used
//! to add that `ci/pine_shape.mjs` "still scores the browser's copy of it" —
//! the gate and the copy both went with the browser client, so nothing scores
//! that silhouette now; `crates/client/tests/tree.rs` is the shape that
//! survived, and it measures the hull rather than the cone.)
//!
//! The one thing not carried across yet is the per-instance colour tint. The
//! browser had it per instance because it drew through an `InstancedMesh`
//! with a colour attribute; here a shared material is what makes a forest one
//! draw call, so variation comes from `Slot`'s own yaw and scale plus a small
//! pool of mesh variants. `ART.md` rule 7 asks for more than that and
//! `RENDER.md` records the debt.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::gather::cell_key;
use sim_core::terrain::{self, Occupant};

use super::fresnel;
use super::terrain_mesh::{CHUNK_M, NEAR_RADIUS};
use super::tree;
use super::{Eye, Net, WorldId};

/// Trunk height, metres. NOT the tree's height: the top whorl tapers to a
/// point and the trunk does not, so a trunk run to full height leaves a bare
/// spike above the canopy on every tree in the forest.
pub const PINE_TRUNK_H: f32 = 5.7;
/// Overall height — the top whorl's apex.
pub const PINE_H: f32 = 6.6;
/// How far a whorl vertex may be pulled IN toward its axis, as a fraction of
/// that whorl's radius. It only ever pulls in, never out, which is what keeps
/// the canopy inside `PINE_MAX_R`.
pub const PINE_RAGGED: f32 = 0.34;
/// How far a whorl's rim may hang BELOW its own base plane, metres. Without
/// it every whorl ends in a level disc and five level discs stacked up a
/// trunk are five horizontal lines.
pub const PINE_DROOP: f32 = 0.18;
/// The radius no part of a pine may exceed, metres — a CEILING, not what it
/// draws. `world.rs` derives `SPAWN_CLEAR_M = 4.0` from this, so a canopy
/// that grew past it would invalidate a spoken sim number from the renderer.
pub const PINE_MAX_R: f32 = 1.7;
/// Odd on purpose: an even count puts a vertex diametrically opposite every
/// other one, so the ragged pull reads as a squashed circle, not a whorl.
const PINE_SEGMENTS: usize = 9;
/// How far a canopy vertex's normal is blended from its own facet toward the
/// canopy volume. Nine segments means nine enormous flat plates each catching
/// the sun as one value, which is the whole visual signature of an asset-pack
/// tree — and it is a shading problem, not an outline one.
pub const PINE_NORMAL_BLEND: f32 = 0.7;

/// `(base y, base radius, height, hash seed)` — where a conifer's silhouette
/// actually comes from. Seeds are distinct from the mixer's own constants:
/// seeding a hash with one of the multipliers inside it correlates a whorl
/// with the one below it, and a correlated whorl is a vertical flute.
const PINE_WHORLS: [(f32, f32, f32, u32); 5] = [
    (1.55, 1.38, 2.15, 0x51ed_270b),
    (2.40, 1.19, 2.00, 0x2545_f491),
    (3.20, 0.99, 1.90, 0x1b87_3593),
    (4.00, 0.77, 1.75, 0x7feb_352d),
    (4.80, 0.55, 1.80, 0x846c_a68b),
];

/// `ART.md` §5's "two greens minimum": a dark lower green the bottom two
/// whorls wear and a lit upper green the top three do, plus the trunk. Each
/// band ramps over the span of the whorls that wear it rather than over each
/// whorl's own height, so the gradient is continuous across a boundary
/// instead of resetting five times up the tree.
const BAND_TRUNK: (u32, u32, f32, f32) = (0x503b2b, 0x70583f, 0.0, 1.9);
const BAND_SKIRT: (u32, u32, f32, f32) = (0x204825, 0x3c783d, 1.55, 4.4);
const BAND_CROWN: (u32, u32, f32, f32) = (0x2c5c2c, 0x4d8845, 3.2, PINE_H);
/// Which band each whorl wears — the low two dark, the top three lit.
const PINE_WHORL_BANDS: [u8; 5] = [1, 1, 2, 2, 2];

/// How many pine meshes the pool holds. Variation without breaking the forest
/// into one draw call per tree: the rag hash is seeded per entry, so no two
/// adjacent trees share an outline as well as a yaw and a scale.
///
/// Unused while the near ring is generated; see `pine_mesh`.
///
/// **Deliberately NOT named `PINE_VARIANTS`**, which is a registered knob
/// (`DECISIONS.md` §open) pinned to `web/src/props.js` at 1. `ci/
/// knob_registry.mjs` went red on the collision the first time this compiled,
/// and it was right to: the browser's number is how many distinct pine
/// GEOMETRIES that renderer authors, this is how many baked meshes a shared-
/// material pool holds, and one registry name cannot mean two things.
#[allow(dead_code, reason = "pairs with pine_mesh, the far-LOD silhouette")]
const PINE_MESH_POOL: usize = 4;

/// Prop meshes and materials, built once and shared so a forest is instances
/// rather than draw calls (`DESIGN.md` §9).
#[derive(Resource)]
pub struct PropAssets {
    /// A conifer's BARK half — trunk and limbs. Named `pines` still because
    /// it is the handle the fell swap and its gate already reach for.
    pines: Vec<Handle<Mesh>>,
    /// …and its needle half, index for index with `pines`. Two meshes and not
    /// one because bark is opaque and needles are alpha-masked, and one
    /// `StandardMaterial` has one `alpha_mode`. The browser reached the same
    /// split for the same reason and also pays two draw calls a variant.
    needles: Vec<Handle<Mesh>>,
    /// The far LOD, index for index with `pines` — one opaque hull lathed
    /// through each variant's own vertices (`tree::impostor_of`). Past
    /// `tree::TREE_LOD_SWAP_M` this ONE mesh stands in for both halves above,
    /// which is the "hierarchical" in HLOD: two draws of ~6 k triangles
    /// become one of ~105, in the depth prepass, the normal prepass, the main
    /// pass and all four shadow cascades alike.
    impostors: Vec<Handle<Mesh>>,
    blob: Handle<Mesh>,
    boulder: Handle<Mesh>,
    stump: Handle<Mesh>,
    bush: Handle<Mesh>,
    barrel: Handle<Mesh>,
    crate_box: Handle<Mesh>,
    cache_box: Handle<Mesh>,
    shelter: Handle<Mesh>,
    canopy: Handle<Mesh>,
    foliage: Handle<StandardMaterial>,
    /// The needle card's material: alpha-MASKED, not blended. A canopy is
    /// hundreds of overlapping cards, and blending them would need a per-card
    /// depth sort that changes with the camera — masked cards write depth and
    /// sort themselves. `AlphaMode::Mask` also keeps them in the opaque pass,
    /// so they cast the shadows a forest floor is made of.
    needle: Handle<StandardMaterial>,
    /// The conifer trunk's bark. Separate from `foliage`, which the bark half
    /// used to wear for want of anything better — an untextured white surface
    /// whose only colour was the mesh's own trunk band.
    bark: Handle<StandardMaterial>,
    rock: Handle<StandardMaterial>,
    ore_stone: Handle<StandardMaterial>,
    ore_metal: Handle<StandardMaterial>,
    ore_sulfur: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    /// Coursed field stone, for the two AUTHORED structures. Distinct from
    /// `rock`, which is a natural granite face — see `assets()`.
    stone: Handle<StandardMaterial>,
}

/// A scatter slot the sim can retire under us — a gatherable node, keyed by
/// the same `gather::cell_key` the server and `ClientCore` use.
///
/// **Re-tested every frame rather than driven off `slot_changes()`, and that
/// is deliberate.** `ClientCore::on_stream` resets its change feed at the top
/// of every call, and `Session::pump` drains every queued message in a loop
/// before the renderer looks — so a frame that received two event messages
/// would see only the second one's changes and silently keep drawing a tree
/// the server felled. `HarvestedSet::contains` is authoritative at any moment,
/// costs a scan of a set that is nearly always tiny, and cannot miss an edge.
#[derive(Component)]
pub struct Fellable {
    pub key: u32,
    /// The mesh variant this slot spawned with.
    ///
    /// **Stored rather than re-derived, because re-deriving it got it wrong.**
    /// `spawn_slot` picks the pine from the slot's yaw and the first cut of
    /// the un-fell branch picked it from the key — so a tree that respawned
    /// came back as a DIFFERENT tree. The test written alongside it did not
    /// catch that, because the test spawned by the same wrong rule the buggy
    /// branch used.
    pub variant: usize,
    /// The y this slot was spawned at, so both swaps set an ABSOLUTE height.
    ///
    /// The first cut accumulated (`y += lift`, `y -= lift`), which is correct
    /// only while every transition is observed. Anything that despawns,
    /// re-seeds or teleports an entity without resetting `felled` drifts it
    /// 0.17 m per missed pair; an absolute set cannot drift.
    pub base_y: f32,
    /// The yaw this slot was spawned at, radians. Stored because the fall
    /// COMPOSES onto it — `Quat::from_axis_angle(..) * from_rotation_y(yaw)`
    /// — and a system that read the live rotation to find the yaw back would
    /// be integrating its own output.
    pub yaw: f32,
    /// Which piece of the slot this entity is. Was a `stumps: bool` while a
    /// felled tree was a mesh swap; a topple has four distinct behaviours and
    /// a bool cannot name them.
    pub part: FellPart,
    /// What is currently drawn, so the swap happens on the transition rather
    /// than every frame.
    pub felled: bool,
}

/// How far through its topple a trunk or a canopy is, seconds. Negative means
/// "not falling" — the resting state both before a chop and after a respawn.
///
/// **Its own component, and that is not tidiness — it is a bug that was
/// written and caught before it shipped.** `fall_t` lived on [`Fellable`]
/// first, which meant [`fall`] took `&mut Fellable` and marked it changed on
/// every frame of the ~96-frame topple. `audio::fell` fires on
/// `Ref<Fellable>::is_changed()`, so one chop would have played the tree-fall
/// cue ninety-six times — and nothing in this repo would have gone red,
/// because every assertion about felling is about geometry and the defect is
/// audible. Splitting the animating field out means `fall` reads `Fellable`
/// immutably and change detection keeps meaning "the sim retired this slot".
///
/// The general shape, which is `CLAUDE.md`'s silent-merge trap one turn on:
/// **a component that something else change-detects may not carry a field
/// that changes every frame.**
#[derive(Component)]
pub struct Topple {
    pub t: f32,
}

/// Which piece of a harvestable slot an entity draws.
///
/// **The tree is three entities and the split is what lets it topple.** While
/// a felled tree was a mesh swap on one entity, `stumps: bool` said everything
/// there was to say: the bark entity became the stump and the needle entity
/// hid. A tree that falls needs the trunk to still exist after the cut — so
/// the stump has to be its own entity, and the trunk's job changes from
/// "become the stump" to "lie down".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FellPart {
    /// A node that simply stops being there: an ore node, a bush, a boulder,
    /// a barrel. Nothing to animate and nothing left behind.
    Vanish,
    /// The trunk and its limbs. Topples on the slot's own bearing and **stays
    /// down** until the slot respawns.
    Trunk,
    /// The needle canopy. Rides the trunk down on the same bearing — it is a
    /// sibling carrying the same transform, never a child, because a child
    /// would inherit the trunk's pose twice.
    Canopy,
    /// The stump, hidden until the cut lands.
    Stump,
    /// The far LOD's hull — the whole tree as one mesh, past
    /// `tree::TREE_LOD_SWAP_M`. It falls exactly as the trunk does and is
    /// silent, counts for nothing and speaks for nothing, because the trunk
    /// standing at the same metre is already all of those.
    ///
    /// **It is a part of its own rather than a second `Trunk`, and that is a
    /// bug this repo has already paid for twice.** `audio.rs` states the rule
    /// in its own words — *"one cue per SLOT, not one per entity"* — and both
    /// of the places it reads `FellPart` key on `Trunk` to mean "this entity
    /// speaks for the whole slot": the fell cue, which would otherwise play
    /// `TreeFall` twice on one chop, and the ambience bed's cover count,
    /// whose `COVER_FULL` was re-derived 14 → 7 the last time a tree quietly
    /// became one more entity than the units assumed. A variant makes every
    /// consumer say what it wants instead of inheriting an answer.
    Far,
}

/// Chunk radius of the TREE-ONLY outer ring, past [`NEAR_RADIUS`]. **(knob)**
///
/// **The ring the trees stop at was sized for a tree that cost 5,900
/// triangles, and that tree has not existed since 2026-08-20.** `NEAR_RADIUS`
/// is 2, so every prop on the island stops between 128 m and 192 m while the
/// far ground mesh draws all 2,048 m of it — the far two-thirds of every wide
/// frame is a painted heightfield with nothing standing on it, which is
/// `ART.md` §1's "air has depth" and §8's far-third checklist item failing for
/// want of anything to haze. Then `tree::impostor_of` landed and a whole tree
/// became **one 105-triangle opaque hull**, ~55× cheaper, and nobody re-derived
/// the radius the old cost had chosen.
///
/// **5, and the arithmetic is the reason.** An 11×11 ring is 96 chunks beyond
/// the near 25, 3.84× the area, so at the ring's own measured p90 of 328 trees
/// it is ~1,260 more — every one of them past [`tree::TREE_LOD_SWAP_M`] by
/// construction and therefore a hull. 1,260 × 105 = **~132 k triangles**
/// against trees currently costing 510 k and `DESIGN.md` §9's 1.5 M for the
/// whole frame. `tests/outer_ring.rs` holds that arithmetic.
///
/// **Widening `NEAR_RADIUS` instead would have been the wrong lever**, and
/// this is the part worth writing down. That constant is read by the ground
/// streamer and by `clutter`, so moving it drags a 65×65 vertex heightfield
/// and a 721-element clutter fill per chunk along with the trees — and a near
/// tree is FOUR entities (trunk, canopy, hidden stump, hull) where an outer
/// one is a single hull with no `Topple` and no `VisibilityRange`. Same
/// picture, an order of magnitude more of everything else.
pub const OUTER_RADIUS: i32 = 5;

/// What the scatter ring has spawned, one parent entity per chunk.
#[derive(Resource, Default)]
pub struct PropRing {
    built: HashMap<(i32, i32), Entity>,
    outer: HashMap<(i32, i32), Entity>,
}

impl PropRing {
    /// Chunks in the NEAR ring, and deliberately not counting [`Self::outer`].
    ///
    /// `loading::read` measures the boot bar against `RING_CHUNKS` (25) with
    /// this number, so folding the outer ring in here would leave the bar
    /// asking for 121 chunks it does not wait for and never filling.
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
    pub fn is_full(&self) -> bool {
        self.built.len() >= super::terrain_mesh::RING_CHUNKS
    }
    /// Chunks in the tree-only outer ring. Read by gates, not by the bar.
    pub fn outer_len(&self) -> usize {
        self.outer.len()
    }
}

/// Chunks in a full outer ring — the 11×11 block minus the near 5×5 it wraps.
pub const OUTER_CHUNKS: usize = ((2 * OUTER_RADIUS + 1) * (2 * OUTER_RADIUS + 1)
    - (2 * NEAR_RADIUS + 1) * (2 * NEAR_RADIUS + 1)) as usize;

/// sRGB hex to linear, which is what a vertex colour is in.
pub(super) fn linear(hex: u32) -> [f32; 3] {
    let f = |c: u32| {
        let s = c as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f((hex >> 16) & 0xff), f((hex >> 8) & 0xff), f(hex & 0xff)]
}

/// The one hash in this file: a 32-bit mix, used for the ragged pull and for
/// nothing that reaches the sim.
pub(super) fn hash2(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x85eb_ca6b) ^ b.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    h
}

pub(super) fn hash01(a: u32, b: u32) -> f32 {
    (hash2(a, b) >> 8) as f32 / 16_777_216.0
}

/// A band's colour at height `y`, ramped lo→hi across its own span.
fn band_color(band: (u32, u32, f32, f32), y: f32) -> [f32; 4] {
    let (lo, hi, y0, y1) = band;
    let t = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
    let (l, h) = (linear(lo), linear(hi));
    [
        l[0] + (h[0] - l[0]) * t,
        l[1] + (h[1] - l[1]) * t,
        l[2] + (h[2] - l[2]) * t,
        1.0,
    ]
}

/// A linear colour normalized to **mean-1 luminance** — the form `ART.md` §7
/// requires of anything that modulates a photograph.
///
/// The prop meshes used to carry their colour in their vertices because there
/// was no photograph to carry it instead. Now there is, and a vertex colour
/// multiplies it: leaving `0x8e887c` in place would tint every granite face by
/// a second grey and land the delivered albedo at the product of two means.
/// Dividing the authored colour by its own luma keeps exactly the part that
/// was doing useful work — the RELATIVE difference between a shelter's wall
/// and its roof, or one facet and its neighbour — and hands the absolute
/// level back to the map, which measured in band (`textures::PropMaps`).
pub(super) fn tint1(hex: u32) -> [f32; 3] {
    let c = linear(hex);
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    if luma <= 1e-6 {
        return [1.0, 1.0, 1.0];
    }
    [c[0] / luma, c[1] / luma, c[2] / luma]
}

/// Raw triangle soup, so every builder here can flat-write and let the
/// normals be decided per triangle rather than per shared vertex.
///
/// **It emits UVs now, and the projection is the point.** Not one procedural
/// mesh in this client had `ATTRIBUTE_UV_0` — only the terrain did — so no
/// prop could sample a photograph however many were shipped in `assets/`.
/// The usual answer is a triplanar shader, and this tree has no WGSL in it;
/// but a triplanar blend exists to hide the seam where a *shared* vertex
/// belongs to faces on different projection planes, and a soup has no shared
/// vertices. Every triangle here is three fresh vertices with a facet normal
/// already, so the plane can be chosen per triangle on the CPU, for free, with
/// no blend and no seam inside any face. Adjacent faces that pick different
/// planes disagree at their shared EDGE — which is what a triplanar's
/// `pow(w, 8)` is buying down — and on a noisy granite map at a facet
/// boundary that discontinuity is invisible, because a facet boundary is
/// already a shading discontinuity.
///
/// The cost is that the projection is object-space, so two instances of one
/// mesh wear the texture identically. `ART.md` rule 7 wants that broken; the
/// per-instance mean-1 tint is what breaks it, not the UV.
pub(super) struct Soup {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
    uv: Vec<[f32; 2]>,
    /// Texture tiles per metre of object space. A rock map over ~2 m reads at
    /// the scale `ART.md` rule 1 asks for: near-field grain under 5 cm.
    uv_scale: f32,
}

impl Default for Soup {
    fn default() -> Self {
        Self {
            pos: Vec::new(),
            nrm: Vec::new(),
            col: Vec::new(),
            uv: Vec::new(),
            // One tile per metre unless a builder says otherwise. Never 0.0,
            // which a derived `Default` would have given and which collapses
            // every UV onto one texel — the "texture did not load" failure
            // with a different cause.
            uv_scale: 1.0,
        }
    }
}

impl Soup {
    /// A soup whose box projection tiles `scale` times per metre.
    pub(super) fn tiling(scale: f32) -> Self {
        Self {
            uv_scale: scale,
            ..Self::default()
        }
    }

    /// One triangle with a facet normal, blended toward `volume_center` by
    /// `blend` — 0 is a flat plate, 1 is a soft volume. A needle mass does not
    /// have facets; it scatters light as a volume, and this is what stops
    /// nine segments reading as nine plates.
    pub(super) fn tri(
        &mut self,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        color: impl Fn(Vec3) -> [f32; 4],
        volume_center: Option<Vec3>,
        blend: f32,
    ) {
        let facet = (b - a).cross(c - a).normalize_or_zero();
        // The projection plane: drop the facet normal's dominant axis. Chosen
        // from the FACET and not from each vertex's blended normal, so all
        // three vertices of a triangle land on one plane — a per-vertex choice
        // would tear the triangle across two projections.
        let ax = facet.abs();
        for v in [a, b, c] {
            let n = match volume_center {
                Some(ctr) => {
                    let vol = (v - ctr).normalize_or_zero();
                    (facet * (1.0 - blend) + vol * blend).normalize_or_zero()
                }
                None => facet,
            };
            let uv = if ax.x >= ax.y && ax.x >= ax.z {
                [v.z, v.y]
            } else if ax.y >= ax.z {
                [v.x, v.z]
            } else {
                [v.x, v.y]
            };
            self.pos.push([v.x, v.y, v.z]);
            self.nrm.push([n.x, n.y, n.z]);
            self.col.push(color(v));
            self.uv.push([uv[0] * self.uv_scale, uv[1] * self.uv_scale]);
        }
    }

    pub(super) fn mesh(self) -> Mesh {
        let n = self.pos.len() as u32;
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv);
        m.insert_indices(Indices::U32((0..n).collect()));
        // Tangents, for the same reason `terrain_mesh` generates them: Bevy's
        // PBR shader builds its tangent frame from `ATTRIBUTE_TANGENT`, and
        // without it a `normal_map_texture` is silently ignored and the
        // surface stays flat. That is the failure that looks like "the
        // texture did not load" and reports nothing at all.
        //
        // A soup can contain a degenerate triangle (two coincident corners on
        // a box massing of zero extent), which mikktspace refuses. That is a
        // malformed mesh rather than a missing capability, so it panics at
        // boot rather than shipping a prop whose relief silently vanished.
        m.generate_tangents()
            .expect("prop soup has positions, normals and UVs");
        m
    }
}

/// One pine, built from stacked cone whorls.
///
/// **Nothing draws this today, and it is kept deliberately.** The near ring is
/// `tree::conifer` now, for the reason `props.js` states after three passes of
/// exactly this builder: a conifer's canopy is alpha cards, and an opaque hull
/// with a polygon edge cannot get there from any amount of geometry. But the
/// far LOD is a different problem — a billboard bake needs a cheap opaque
/// silhouette to render from, and 104 triangles that already measure inside
/// `PINE_MAX_R` is the right starting point. `props.js` kept its cone builder
/// for the same role and `ci/pine_shape.mjs` scored that one; both went with
/// the browser client, so this silhouette is ungated.
///
/// ⚠ **It is not what the shipped far LOD draws, and the difference is a
/// species.** `tree::impostor_of` lathes its hull through the generated
/// tree's own vertices, so it fits a 5.4 m broadleaf dome and a 6.6 m conifer
/// spire alike; this builder is a conifer, authored from `PINE_*`. Kept for
/// the bake, which wants one silhouette per species and would rather render
/// an authored one — but a second caller of this function is almost certainly
/// reaching for the wrong hull.
#[allow(
    dead_code,
    reason = "the far-LOD silhouette, per TERRAIN.md §4; see above"
)]
fn pine_mesh(variant: u32) -> Mesh {
    let mut s = Soup::default();

    // The trunk: a tapered prism, 7 sides. Its band stops at 1.9 m because
    // everything above that is inside the canopy and invisible.
    let sides = 7usize;
    let (r0, r1) = (0.22f32, 0.13f32);
    for i in 0..sides {
        let a0 = i as f32 / sides as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let (b0, b1) = (
            Vec3::new(a0.cos() * r0, 0.0, a0.sin() * r0),
            Vec3::new(a1.cos() * r0, 0.0, a1.sin() * r0),
        );
        let (t0, t1) = (
            Vec3::new(a0.cos() * r1, PINE_TRUNK_H, a0.sin() * r1),
            Vec3::new(a1.cos() * r1, PINE_TRUNK_H, a1.sin() * r1),
        );
        let col = |v: Vec3| band_color(BAND_TRUNK, v.y);
        s.tri(b0, t0, b1, col, None, 0.0);
        s.tri(b1, t0, t1, col, None, 0.0);
    }

    for (wi, (base_y, radius, h, seed)) in PINE_WHORLS.iter().enumerate() {
        let band = if PINE_WHORL_BANDS[wi] == 1 {
            BAND_SKIRT
        } else {
            BAND_CROWN
        };
        let col = move |v: Vec3| band_color(band, v.y);
        let apex = Vec3::new(0.0, base_y + h, 0.0);
        // The volume the canopy's normals are pulled toward: the middle of
        // this whorl's own cone.
        let ctr = Vec3::new(0.0, base_y + h * 0.35, 0.0);

        let rim: Vec<Vec3> = (0..PINE_SEGMENTS)
            .map(|i| {
                let a = i as f32 / PINE_SEGMENTS as f32 * std::f32::consts::TAU;
                // Pull IN by up to PINE_RAGGED of the radius, and hang the
                // rim below its own base plane by up to PINE_DROOP. The hash
                // is over the vertex's own index plus the whorl's seed and
                // the variant, so a shared edge's two ends agree and the seam
                // cannot open.
                let pull =
                    1.0 - PINE_RAGGED * hash01(seed ^ variant.wrapping_mul(0x9e37_79b9), i as u32);
                let droop = PINE_DROOP * hash01(seed ^ 0x5bf0_3635, i as u32);
                let r = radius * pull;
                debug_assert!(r <= PINE_MAX_R);
                Vec3::new(a.cos() * r, base_y - droop, a.sin() * r)
            })
            .collect();

        let base_ctr = Vec3::new(0.0, *base_y, 0.0);
        for i in 0..PINE_SEGMENTS {
            let (p, q) = (rim[i], rim[(i + 1) % PINE_SEGMENTS]);
            // The cone's side.
            s.tri(p, apex, q, col, Some(ctr), PINE_NORMAL_BLEND);
            // The underside. The rim droops and the centre does not, so it is
            // a shallow bowl rather than a plate — the face `ART.md` §5 says
            // every judge catches.
            s.tri(q, base_ctr, p, col, Some(ctr), PINE_NORMAL_BLEND);
        }
    }

    s.mesh()
}

/// What a felled pine leaves behind. The browser's `stumpGeometry`, constant
/// for constant: a six-sided taper 0.26 m at the top and 0.32 m at the base,
/// 0.34 m tall, wearing the trunk band. The cut face is lighter — fresh
/// timber against weathered bark is the whole reason a stump reads as a stump
/// and not as a rock.
fn stump_mesh() -> Mesh {
    // A stump is 0.34 m tall and 0.64 m across, so the bark map has to repeat
    // several times over it to read as bark rather than as one blurred patch.
    let mut s = Soup::tiling(1.6);
    let sides = 6usize;
    let (r_base, r_top, h) = (0.32f32, 0.26f32, 0.34f32);
    // Mean-1 over the photograph, per `tint1` — the band still ramps weathered
    // to fresh up the stump, it just no longer sets the absolute colour.
    let col = |v: Vec3| {
        let c = band_color(BAND_TRUNK, v.y + 0.17);
        let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        if l <= 1e-6 {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            [c[0] / l, c[1] / l, c[2] / l, 1.0]
        }
    };
    for i in 0..sides {
        let a0 = i as f32 / sides as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let b0 = Vec3::new(a0.cos() * r_base, -h * 0.5, a0.sin() * r_base);
        let b1 = Vec3::new(a1.cos() * r_base, -h * 0.5, a1.sin() * r_base);
        let t0 = Vec3::new(a0.cos() * r_top, h * 0.5, a0.sin() * r_top);
        let t1 = Vec3::new(a1.cos() * r_top, h * 0.5, a1.sin() * r_top);
        s.tri(b0, t0, b1, col, None, 0.0);
        s.tri(b1, t0, t1, col, None, 0.0);
    }
    // The cut face — fresh timber against weathered bark, and still the reason
    // a stump reads as a stump. Mean-1 like the rest, so "lighter than the
    // bark" survives while the map sets the level.
    let cut = tint1(0xb59a6e);
    let cap = move |_: Vec3| [cut[0], cut[1], cut[2], 1.0];
    let centre = Vec3::new(0.0, h * 0.5, 0.0);
    for i in 0..sides {
        let a0 = i as f32 / sides as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let t0 = Vec3::new(a0.cos() * r_top, h * 0.5, a0.sin() * r_top);
        let t1 = Vec3::new(a1.cos() * r_top, h * 0.5, a1.sin() * r_top);
        s.tri(centre, t0, t1, cap, None, 0.0);
    }
    s.mesh()
}

/// Smooth 3D value noise, trilinear over an integer lattice.
///
/// **It has to be a pure function of the direction vector or the mesh cracks.**
/// The blob displaces shared subdivision vertices along their own direction;
/// two triangles meeting at an edge compute that displacement independently,
/// so they agree only if the displacement depends on nothing but the position
/// they both hold. A per-triangle or per-index hash — which is what the
/// unsubdivided blob used, legitimately, because it had no shared vertices —
/// would open a gap along every edge here.
///
/// Smoothstep on the lattice fraction rather than a raw lerp: a linear
/// interpolation is C0, and a rock lit by a 35° sun shows every C0 crease as a
/// shading band running along the lattice.
fn vnoise3(p: Vec3, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    // 3f² − 2f³, the Hermite smoothstep.
    let w = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let (xi, yi, zi) = (i.x as i32, i.y as i32, i.z as i32);
    let corner = |dx: i32, dy: i32, dz: i32| {
        // Three coordinates into one hash input. The lattice is small (a unit
        // sphere times a handful of octaves), so a wrapping mix is plenty.
        let k = (xi + dx).wrapping_mul(73_856_093)
            ^ (yi + dy).wrapping_mul(19_349_663)
            ^ (zi + dz).wrapping_mul(83_492_791);
        hash01(seed, k as u32)
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), w.x);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), w.x);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), w.x);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), w.x);
    lerp(lerp(x00, x10, w.y), lerp(x01, x11, w.y), w.z)
}

/// Three octaves of `vnoise3`, 0..1 — the radius modulation that turns a
/// sphere into a rock. Frequencies are deliberately non-harmonic so the
/// octaves do not line up into a visible lattice.
///
/// **The first cut of this was too high-frequency and the capture said so.**
/// It ran 1.7 / 3.9 / 8.3, which is three octaves of *surface* detail, and
/// three octaves of surface detail average out to a sphere: the boulder came
/// back an egg. The silhouette is owned by the LOWEST frequency — a rock has
/// two or three big lobes, not fifty small ones — so the base octave drops
/// under 1 cycle per hemisphere and takes most of the amplitude. The high
/// octave is nearly gone because the normal map is better at that scale than
/// geometry is, which is the whole reason to spend the triangles on shape.
fn lumps(dir: Vec3, seed: u32) -> f32 {
    let a = vnoise3(dir * 0.95, seed);
    let b = vnoise3(dir * 2.30, seed ^ 0x9e37_79b9);
    let c = vnoise3(dir * 5.70, seed ^ 0x85eb_ca6b);
    (a * 0.66 + b * 0.25 + c * 0.09).clamp(0.0, 1.0)
}

/// A per-mesh axis squash, all three factors ≤ 1.
///
/// A displaced sphere is still equidimensional, and almost no real rock is —
/// a boulder is wider than it is tall because that is how it came to rest.
/// **Every factor stays at or under 1 on purpose**: the old blob's widest
/// possible vertex was exactly `radius`, `sim_core::terrain` places these
/// against clearances derived from nominal sizes, and a renderer that grows a
/// prop past the extent the sim reasoned about is the renderer deciding
/// something. Shrinking is always safe; growing would need a spoken number.
fn squash(seed: u32) -> Vec3 {
    Vec3::new(
        0.88 + 0.12 * hash01(seed ^ 0x6d2b_79f5, 1),
        0.62 + 0.22 * hash01(seed ^ 0x6d2b_79f5, 2),
        0.88 + 0.12 * hash01(seed ^ 0x6d2b_79f5, 3),
    )
}

/// A rock, built by subdividing an icosahedron and displacing it.
///
/// **What this replaced, and why it was the worst thing in the frame.** The
/// first version was the bare icosahedron — twelve vertices, twenty flat
/// triangles, no subdivision — with a per-facet value jitter of ±0.36 to keep
/// `ART.md` rule 1 off its back. A capture from the `near` vantage shows the
/// result plainly: the boulder is the closest object to the camera and it
/// reads as a two-tone d20, eight enormous flat facets, no relief, no texture.
/// Rule 6 says silhouette before surface and this was failing both.
///
/// Three things changed and they are not independent:
///
///   · **Subdivision.** `sub` levels of midpoint split, deduped on the edge so
///     the shared vertices stay shared. 20 · 4^sub triangles.
///   · **Displacement**, by `lumps`, which is continuous over the sphere (see
///     `vnoise3`) so subdivision buys relief instead of a smoother ball.
///   · **The facet jitter comes down hard**, ±0.36 → ±0.06. It existed to fake
///     surface variation onto twenty flat faces; the photograph supplies that
///     now, and leaving the old amplitude on would read as the patchwork it
///     always was, only with a texture behind it.
///
/// `textured` picks which of two jobs the vertex colour is doing. For a rock
/// it is a mean-1 field over a photograph (`tint1`); for the bush — which
/// shares this builder and must stay green — it is still the colour itself.
fn blob_mesh(radius: f32, jitter: f32, seed: u32, hex: u32, sub: usize, textured: bool) -> Mesh {
    // Icosahedron vertices.
    let t = (1.0 + 5.0f32.sqrt()) * 0.5;
    let raw = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    const FACES: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    // Subdivide on the UNIT sphere, deduping every midpoint on its edge so
    // the two triangles sharing it get one vertex and therefore one
    // displacement. Displacing after the split (rather than splitting a
    // displaced hull) is what makes the noise the shape rather than a
    // smoothing of the icosahedron's corners.
    let mut dirs: Vec<Vec3> = raw
        .iter()
        .map(|v| Vec3::from_array(*v).normalize())
        .collect();
    let mut faces: Vec<[usize; 3]> = FACES.to_vec();
    for _ in 0..sub {
        let mut mid: HashMap<(usize, usize), usize> = HashMap::default();
        let mut next = Vec::with_capacity(faces.len() * 4);
        for f in &faces {
            let mut m = [0usize; 3];
            for e in 0..3 {
                let (a, b) = (f[e], f[(e + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                m[e] = *mid.entry(key).or_insert_with(|| {
                    dirs.push(((dirs[a] + dirs[b]) * 0.5).normalize());
                    dirs.len() - 1
                });
            }
            next.push([f[0], m[0], m[2]]);
            next.push([m[0], f[1], m[1]]);
            next.push([m[2], m[1], f[2]]);
            next.push([m[0], m[1], m[2]]);
        }
        faces = next;
    }

    let axis = squash(seed);
    let verts: Vec<Vec3> = dirs
        .iter()
        .map(|d| *d * radius * (1.0 - jitter * lumps(*d, seed)) * axis)
        .collect();

    // Tiles per metre. A granite map repeating every ~1.5 m puts its grain
    // under 5 cm at this radius, which is rule 1's near-field scale.
    let mut s = Soup::tiling(0.68);
    let base = if textured { tint1(hex) } else { linear(hex) };
    for (fi, f) in faces.iter().enumerate() {
        // Still a per-facet break-up, and still for rule 1 — but a sixth of
        // the amplitude, because the photograph is doing this job now and two
        // sources of the same variation is the patchwork the old rock was.
        let v = 0.97 + 0.06 * hash01(seed ^ 0x2c1b_3c6d, fi as u32);
        let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
        // Normals blended toward the volume rather than left flat. At twenty
        // faces a facet normal WAS the rock's character; at 1,280 it is just
        // stair-stepping, and the relief that reads is the displacement's and
        // the normal map's.
        s.tri(
            verts[f[0]],
            verts[f[1]],
            verts[f[2]],
            col,
            Some(Vec3::ZERO),
            0.55,
        );
    }
    s.mesh()
}

// ── The authored structures are DERIVED from the sim's tables ──────────────
//
// `TERRAIN.md` §7.1 and `reference/MONUMENTS.md` §9.4. Until this block, the
// haven shelter and the waystation canopy were declared TWICE — once in
// `sim_core::terrain` as the volume the sim blocks, once here as the mesh the
// client draws — and the gate that held the two lists equal was a browser
// `.mjs` deleted with the browser client. **They had drifted, measured:** the
// canopy was 9 rows to a 4.1 m finial in the sim and 6 rows topping out at
// 2.09 m here, so a player was stopped ~0.7 m outside posts they could see;
// the shelter was 14 rows against 9, missing its four corner posts and its
// tower-cap, with a 9.2 m peak drawn at 5.6 m. The two tables were also in
// DIFFERENT UNITS — full size there, half extent here — which is the
// transcription hazard the deleted gate existed to cover.
//
// **The fix is derivation, not a new mirror plus a new gate.** A gate over two
// hand-written tables can only catch drift after someone writes it; a mesh
// built from the sim's own rows cannot drift at all, because there is one
// list. What is left for `tests/greybox.rs` to check is the part derivation
// cannot make true by construction: that the resulting triangles fit the
// broad-phase radius and peak the sim publishes (`OCCUPANT_R_M`,
// `OCCUPANT_TOP_M`), whose own doc admits "nothing in the Rust workspace can
// see a triangle".
//
// Only the COLOURS live here, because the sim has none and should not: what
// a wall is made of is not a fact about the volume it occupies.

/// Per-row colour for `terrain::SHELTER_BOXES`, index for index.
///
/// The array length is the sim table's length as a type, so adding a box to
/// the sim and forgetting to colour it is a compile error rather than a
/// silently untinted wall.
pub const SHELTER_HEX: [u32; terrain::SHELTER_BOXES.len()] = [
    0x6f6a60, // plinth — the one course that is not wall
    0x8a8479, // wall-back
    0x8a8479, // wall-left
    0x8a8479, // wall-right
    0x8a8479, // jamb-left
    0x8a8479, // jamb-right
    0x8a8479, // lintel
    0x5f5b53, // roof — darker, so the mass reads against the sky
    0x8a8479, // post-nw
    0x8a8479, // post-ne
    0x8a8479, // post-sw
    0x8a8479, // post-se
    0x8a8479, // tower
    0x5f5b53, // tower-cap — roof stone, because it is roof
];

/// Per-row colour for `terrain::WAYSTATION_CANOPY_BOXES`, index for index.
/// Timber rather than the pad's field stone: the two tiers differ in material
/// as well as in silhouette (`terrain.rs`'s note on the canopy table).
pub const CANOPY_HEX: [u32; terrain::WAYSTATION_CANOPY_BOXES.len()] = [
    0x6a5940, // deck
    0x6a5940, // post-nw
    0x6a5940, // post-ne
    0x6a5940, // post-sw
    0x6a5940, // post-se
    0x6a5940, // parapet
    0x7b6a4f, // eave — the plates are lighter, sun-bleached above
    0x7b6a4f, // cap
    0x7b6a4f, // finial
];

/// One sim box table plus its colours, as the `(centre, half-extent, hex)`
/// triples `boxes_mesh` wants.
///
/// **The `* 0.5` is the whole point of this function.** The sim states FULL
/// size and the mesh builder wants a HALF extent, and that conversion written
/// by hand at fourteen call sites is exactly how the two lists came apart. It
/// is written once, here, against a length the type system pins.
pub fn authored<const N: usize>(
    rows: &[[f32; 6]; N],
    hex: &[u32; N],
) -> Vec<([f32; 3], [f32; 3], u32)> {
    rows.iter()
        .zip(hex.iter())
        .map(|(r, h)| ([r[0], r[1], r[2]], [r[3] * 0.5, r[4] * 0.5, r[5] * 0.5], *h))
        .collect()
}

/// How far every prop is pushed into the ground, metres.
///
/// `ART.md` rule 2: nothing sits ON the ground, everything sits IN it. Sinking
/// the lift slightly is the cheapest half of that; the crowding half is the
/// clutter skirt, which `sim_core` already places.
///
/// Hoisted out of `spawn_slot` so `tests/greybox.rs` can evaluate the rule
/// rather than restate it — "nothing floats" is `lift + mesh_base - SINK_M
/// <= 0`, which is not checkable while the number is a local.
pub const SINK_M: f32 = 0.06;

/// The mesh the client draws for one occupant, as a pure function.
///
/// **Extracted so the gate can ask the renderer's own question.** Before this,
/// every archetype's geometry was an expression inside `assets()`, reachable
/// only through `Assets<Mesh>` and therefore only from a running app — which
/// is why `OCCUPANT_R_M`'s doc in `sim_core` says, accurately, that "nothing
/// in the Rust workspace can see a triangle, so the asserts below prove only
/// that this file agrees with itself". `tests/greybox.rs` calls this and
/// counts vertices, which is the arithmetic `CLAUDE.md` says a frame may be
/// gated on.
///
/// `None` for the two rows that are not one mesh: `Occupant::None`, and
/// `Tree`, whose `CONIFER_POOL` variants are generated and gated one by one
/// in `tests/tree.rs` already.
pub fn archetype_mesh(o: Occupant) -> Option<Mesh> {
    Some(match o {
        Occupant::None | Occupant::Tree => return None,
        // One blob serves all three ore nodes; they differ by material only.
        Occupant::StoneNode | Occupant::MetalNode | Occupant::SulfurNode => {
            blob_mesh(1.0, 0.46, 0x51ed_270b, 0x9c968a, 2, true)
        }
        Occupant::Bush => blob_mesh(0.7, 0.58, 0x2545_f491, 0x2c5f2e, 1, false),
        Occupant::Rock => blob_mesh(1.5, 0.52, 0x1b87_3593, 0x8e887c, 3, true),
        // The measured drum: 0.585 m across by 0.88 tall, so a radius of
        // 0.2925 and a full height of 0.88 — 1.5x taller than wide, which is
        // what a 55-gallon drum is. It drew 0.9 across by 0.95 tall until
        // 2026-08-18, the browser client's `CylinderGeometry(0.45, 0.45, 0.95)`
        // carried forward: near-spherical and ~44% too fat. The reference set's
        // `barrelroad` and Meshy's independent `auto_size` estimate agree on
        // the pair. The mesh could not move alone — `greybox.rs`'s
        // `every_drawn_archetype_fits_the_volume_the_sim_blocks` fails a mesh
        // narrower than the blocked volume by more than SLACK_R_M, because an
        // invisible collision skirt is a player passing through geometry — so
        // `OCCUPANT_R_M`/`OCCUPANT_TOP_M` moved in the same commit and the
        // replay golden moved with them.
        Occupant::BarrelSlot => Cylinder::new(0.2925, 0.88).mesh().resolution(10).build(),
        Occupant::CrateSlot => boxes_mesh(&[([0., 0., 0.], [0.55, 0.4, 0.4], 0x6b5334)]),
        Occupant::CacheSlot => boxes_mesh(&[([0., 0., 0.], [0.45, 0.275, 0.35], 0x6a5940)]),
        Occupant::HavenShelter => boxes_mesh(&authored(&terrain::SHELTER_BOXES, &SHELTER_HEX)),
        Occupant::WaystationCanopy => {
            boxes_mesh(&authored(&terrain::WAYSTATION_CANOPY_BOXES, &CANOPY_HEX))
        }
    })
}

/// How far an archetype's mesh origin sits above the slot's ground — the
/// browser's `lift`, kept because these meshes are centred and a slot's `y` is
/// the surface.
///
/// **One table, read by the draw and by the gate.** `sim_core`'s
/// `OCCUPANT_TOP_M` documents every one of its rows as "lift + half-extent",
/// which is a claim about a number that lived only in `spawn_slot`'s match
/// arm; now the gate can evaluate it.
pub fn archetype_lift(o: Occupant) -> f32 {
    match o {
        Occupant::StoneNode | Occupant::MetalNode | Occupant::SulfurNode => 0.5,
        Occupant::Bush => 0.45,
        Occupant::Rock => 0.55,
        // The half-height, so the drum's base is the slot's ground and its
        // top is its own 0.88 m. It was 0.5 against a half-height of 0.475.
        Occupant::BarrelSlot => 0.44,
        Occupant::CrateSlot => 0.4,
        Occupant::CacheSlot => 0.275,
        // The two authored structures and the tree stand on their own base:
        // their tables put ground at y = 0 rather than centring the mesh.
        Occupant::HavenShelter | Occupant::WaystationCanopy | Occupant::Tree => 0.0,
        Occupant::None => 0.0,
    }
}

/// A box massing, for the two authored structures. Each entry is
/// `(centre, half-extent, hex)`.
///
/// Half a tile per metre — planks and field stone read at a coarser scale
/// than granite grain, and a box massing has flat metre-scale faces where a
/// tight repeat would be visible tiling (`ART.md` rule 7). Mean-1 tint,
/// because every massing on this path wears a photograph.
pub(super) fn boxes_mesh(parts: &[([f32; 3], [f32; 3], u32)]) -> Mesh {
    boxes_mesh_with(parts, tint1, 0.5)
}

/// The same massing, with the two things a textured prop and an untextured
/// one cannot share.
///
/// **The colour function is not a convenience, it is a correctness split, and
/// it cost a capture to find.** `tint1` divides the authored hex by its own
/// luma, so what reaches the vertex is a mean-1 *modulation* of a photograph
/// — exactly right for a shelter wearing a granite map, and it renders
/// **near-white** on a material with no texture behind it. The pig shipped
/// that way for one build and read as a pale ghost in the frame while every
/// gate stayed green, because no gate in this repo looks at a colour. An
/// untextured massing passes `linear` and carries its own albedo.
pub(super) fn boxes_mesh_with(
    parts: &[([f32; 3], [f32; 3], u32)],
    colour: fn(u32) -> [f32; 3],
    tiles_per_m: f32,
) -> Mesh {
    let mut s = Soup::tiling(tiles_per_m);
    for (c, h, hex) in parts {
        let c = Vec3::from_array(*c);
        let h = Vec3::from_array(*h);
        let base = colour(*hex);
        let corner = |sx: f32, sy: f32, sz: f32| c + Vec3::new(h.x * sx, h.y * sy, h.z * sz);
        // Six faces, each two triangles, each face at its own value so the
        // massing reads as a solid rather than a silhouette.
        let faces: [([Vec3; 4], f32); 6] = [
            (
                [
                    corner(-1., -1., 1.),
                    corner(1., -1., 1.),
                    corner(1., 1., 1.),
                    corner(-1., 1., 1.),
                ],
                0.95,
            ),
            (
                [
                    corner(1., -1., -1.),
                    corner(-1., -1., -1.),
                    corner(-1., 1., -1.),
                    corner(1., 1., -1.),
                ],
                0.78,
            ),
            (
                [
                    corner(1., -1., 1.),
                    corner(1., -1., -1.),
                    corner(1., 1., -1.),
                    corner(1., 1., 1.),
                ],
                0.88,
            ),
            (
                [
                    corner(-1., -1., -1.),
                    corner(-1., -1., 1.),
                    corner(-1., 1., 1.),
                    corner(-1., 1., -1.),
                ],
                0.84,
            ),
            (
                [
                    corner(-1., 1., 1.),
                    corner(1., 1., 1.),
                    corner(1., 1., -1.),
                    corner(-1., 1., -1.),
                ],
                1.0,
            ),
            (
                [
                    corner(-1., -1., -1.),
                    corner(1., -1., -1.),
                    corner(1., -1., 1.),
                    corner(-1., -1., 1.),
                ],
                0.62,
            ),
        ];
        for (q, v) in faces {
            let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
            s.tri(q[0], q[1], q[2], col, None, 0.0);
            s.tri(q[0], q[2], q[3], col, None, 0.0);
        }
    }
    s.mesh()
}

/// Build the shared mesh and material pool.
///
/// `images` is here for exactly one thing: the needle card, which is generated
/// rather than loaded (`tree::needle_image`). `assets/textures/` has bark and
/// no leaf, and an un-masked leaf quad is a solid square — the opaque hull the
/// browser already spent three passes rejecting. Generating it keeps the
/// depot's asset list unchanged and cannot go missing from a build.
pub fn assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    maps: &super::textures::PropMaps,
) -> PropAssets {
    let surface = |rough: f32, refl: f32, materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: rough,
            reflectance: refl,
            ..default()
        })
    };
    // The same, wearing a photograph. `base_color` stays WHITE and no gain is
    // applied: all five sources measured inside `ALBEDO_LUMA_BAND` off the raw
    // file, so the map ships its colour whole and the mesh's vertex colour is
    // a mean-1 field over it (`textures::PropMaps` has the table).
    let photo = |m: &super::textures::MapSet,
                 rough: f32,
                 refl: f32,
                 materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(m.albedo.clone()),
            normal_map_texture: Some(m.normal.clone()),
            // The roughness maps stay unwired: `metallic_roughness_texture`
            // is a glTF-packed ORM slot whose B channel is METALLIC, and these
            // sources are greyscale roughness jpgs, so binding one here would
            // make every prop a half-metal. It needs an ORM packing step, not
            // a slot assignment. **The occlusion slot is a different question**
            // and is wired below: `occlusion_texture` is its own binding and
            // takes a greyscale map directly.
            occlusion_texture: m.ao.clone(),
            perceptual_roughness: rough,
            reflectance: refl,
            ..default()
        })
    };
    // One generated conifer per pool entry, split into its two halves — and
    // its far hull, built from the pair BEFORE the meshes are handed to the
    // asset server, because the hull is a measurement of them.
    let conifers: Vec<(Handle<Mesh>, Handle<Mesh>, Handle<Mesh>)> = (0..tree::CONIFER_POOL)
        .map(|v| {
            let (bark, needles) = tree::conifer(v);
            let far = tree::impostor_of(&bark, &needles, v);
            (meshes.add(bark), meshes.add(needles), meshes.add(far))
        })
        .collect();
    let needle_map = images.add(tree::needle_image());
    PropAssets {
        pines: conifers.iter().map(|(b, _, _)| b.clone()).collect(),
        needles: conifers.iter().map(|(_, n, _)| n.clone()).collect(),
        impostors: conifers.iter().map(|(_, _, f)| f.clone()).collect(),
        // Subdivision levels are picked per role against how close a player
        // gets and how many stand in a ring, not uniformly. 20·4^sub
        // triangles: the boulder is 1,280, the nodes 320, the bush 320.
        // `RENDER.md` §6's 1.5 M ceiling is browser-era and the conifer is
        // what actually presses it — a full scatter ring of rocks at 1,280 is
        // tens of thousands, which is not the budget's problem.
        // Jitter is up from 0.28/0.32 with the frequency drop above: at the
        // old amplitude a low-frequency lobe is a bulge, not a shape.
        blob: meshes.add(archetype_mesh(Occupant::StoneNode).expect("node mesh")),
        boulder: meshes.add(archetype_mesh(Occupant::Rock).expect("boulder mesh")),
        stump: meshes.add(stump_mesh()),
        // The bush keeps its colour in its vertices: it is the one blob that
        // wears the untextured `foliage` material, and there is no leaf map in
        // `assets/` to give it (`tree::needle_image` is generated, and it is a
        // needle sprig rather than broadleaf).
        // One subdivision, not two, and the higher jitter that goes with it: a
        // bush wants a ragged outline (`ART.md` rule 6) and 320 smooth
        // triangles gave it a green dome. 80 is enough to stop reading as a
        // die and few enough that the lobes stay visible.
        bush: meshes.add(archetype_mesh(Occupant::Bush).expect("bush mesh")),
        barrel: meshes.add(archetype_mesh(Occupant::BarrelSlot).expect("barrel mesh")),
        crate_box: meshes.add(archetype_mesh(Occupant::CrateSlot).expect("crate mesh")),
        cache_box: meshes.add(archetype_mesh(Occupant::CacheSlot).expect("cache mesh")),
        // Both authored structures come off the sim's own box tables — see
        // the `authored` block above for what mirroring them by hand cost.
        shelter: meshes.add(archetype_mesh(Occupant::HavenShelter).expect("shelter mesh")),
        canopy: meshes.add(archetype_mesh(Occupant::WaystationCanopy).expect("canopy mesh")),
        foliage: surface(0.86, fresnel::DIELECTRIC, materials),
        needle: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(needle_map),
            perceptual_roughness: 0.90,
            reflectance: fresnel::DIELECTRIC,
            // Cards are two-sided by construction — you see the underside of
            // every branch you stand beneath, and `ART.md` §5 calls that face
            // the one every judge catches.
            cull_mode: None,
            // 0.5 against a mask whose alpha ramps to the needle's edge. Not
            // defaulted anywhere: `props.js` mutation-tested a `|| 0.3`
            // fallback and found it silently restored an opaque hull.
            alpha_mode: AlphaMode::Mask(0.5),
            // Needles are thin and the sun is behind half of them. Without
            // this the canopy's lit side is the only side that reads.
            double_sided: true,
            ..default()
        }),
        // ── The photographs. Every one of these was a flat `base_color` and
        // a roughness scalar until now, which is `ART.md` rule 1 broken on
        // every non-ground surface in the frame.
        //
        // The three ore nodes share the GRANITE map and differ by ROUGHNESS
        // rather than by hue: they are one rock with a different mineral in
        // it, and a node's identity is the glint it gives back. A per-ore
        // albedo is a real want and a sourcing question rather than a code one
        // — there is no sulfur map here.
        //
        // **That identity used to be spelled in `reflectance` and could not
        // work.** The three sat at 0.24/0.42/0.22, i.e. F0 0.9%/2.8%/0.8%, all
        // far under a dielectric's 4% — so the channel carrying the difference
        // had almost no energy in it, and the roughness that shapes the lobe
        // had nothing to shape (`render::fresnel`). Every rock is a dielectric
        // at 4% now and the split moved to the channel that can express it:
        // 0.80 for granite against 0.55 for the metal-bearing node is a wide
        // gap that reads for the first time.
        //
        // **`stone` is deliberately not one of them, and the capture is why.**
        // `Bricks089` is photoscanned stacked FIELD STONE — mortar joints and
        // courses — and the first cut put it on the stone node because the
        // names matched. A 2 m sphere wrapped in coursed masonry reads as a
        // buried castle wall, not as an outcrop: it was the most conspicuous
        // thing in the frame and it was conspicuous because the map is good.
        // Masonry belongs on something a player BUILT (`structures.rs`), and
        // a natural rock face is granite whatever the ore in it.
        rock: photo(&maps.rock, 0.88, fresnel::DIELECTRIC, materials),
        ore_stone: photo(&maps.rock, 0.80, fresnel::DIELECTRIC, materials),
        ore_metal: photo(&maps.metal, 0.55, fresnel::METAL_DIELECTRIC, materials),
        ore_sulfur: photo(&maps.rock, 0.78, fresnel::DIELECTRIC, materials),
        wood: photo(&maps.wood, 0.85, fresnel::DIELECTRIC, materials),
        metal: photo(&maps.metal, 0.50, fresnel::METAL_DIELECTRIC, materials),
        // The haven pad and the waystation ARE built, so they get the
        // masonry — this is the identity that map was sourced for
        // (`MANIFEST.md`: "the identity ART asks for, not brick").
        stone: photo(&maps.stone, 0.82, fresnel::DIELECTRIC, materials),
        // The conifer's trunk. `ART.md` §5 asks for fissures running UP the
        // trunk, and this is the one prop that needed no UV work at all to get
        // them: `bevy_procedural_tree` emits `ATTRIBUTE_UV_0` cylindrically —
        // u around the ring, v along the branch — so a bark map lands with its
        // grain already pointing the right way. Before this the bark half of
        // every tree wore `foliage`, an untextured white surface.
        bark: photo(&maps.bark, 0.92, fresnel::DIELECTRIC, materials),
    }
}

/// Spawn the scatter for the near ring, one chunk per frame.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<PropRing>,
    mut store: Local<Option<PropAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    maps: Res<super::textures::PropMaps>,
    world: Res<WorldId>,
    eye: Res<Eye>,
    lod: Res<tree::TreeLod>,
) {
    let a = store.get_or_insert_with(|| assets(&mut meshes, &mut materials, &mut images, &maps));

    let cx = (eye.pos.x / CHUNK_M).floor() as i32;
    let cz = (eye.pos.z / CHUNK_M).floor() as i32;

    let mut dropped = 0usize;
    ring.built.retain(|(bx, bz), e| {
        if dropped >= 1 || ((*bx - cx).abs() <= NEAR_RADIUS && (*bz - cz).abs() <= NEAR_RADIUS) {
            return true;
        }
        dropped += 1;
        commands.entity(*e).despawn();
        false
    });

    for dz in -NEAR_RADIUS..=NEAR_RADIUS {
        for dx in -NEAR_RADIUS..=NEAR_RADIUS {
            let key = (cx + dx, cz + dz);
            if ring.built.contains_key(&key) {
                continue;
            }
            let parent = commands
                .spawn((
                    super::WorldEntity,
                    Transform::IDENTITY,
                    Visibility::default(),
                ))
                .id();
            // A 64 m chunk is exactly 8 scatter cells a side (CELL_SIZE 8 m).
            let cells = (CHUNK_M / terrain::CELL_SIZE) as i32;
            // One memo for the chunk's 64 cells: each is a `ground` tap and a
            // four-tap slope fan inside 8 m, so they share nearly every corner.
            // Bit-identical — `sim-core/tests/lattice.rs` holds the two forms
            // of `scatter` equal field for field.
            let mut lat = terrain::Lattice::new();
            for iz in 0..cells {
                for ix in 0..cells {
                    let cell_x = key.0 * cells + ix;
                    let cell_z = key.1 * cells + iz;
                    let slot = terrain::scatter_memo(
                        &mut lat,
                        world.seed,
                        &world.table,
                        &world.haven,
                        cell_x,
                        cell_z,
                    );
                    if slot.occupant == Occupant::None {
                        continue;
                    }
                    // `cell_key` is `sim_core::gather`'s own, not a second
                    // copy: the client's mirror is keyed by it and a renderer
                    // that packed its own would silently never match.
                    let key = cell_key(cell_x as u16, cell_z as u16);
                    spawn_slot(&mut commands, parent, a, &slot, key, &lod);
                }
            }
            ring.built.insert(key, parent);
            // One chunk of scatter per frame, same budget as the ground.
            return;
        }
    }

    // ── The outer ring ──────────────────────────────────────────────────
    //
    // Reached only once the near ring is complete, because the `return` above
    // fires on every frame that built a near chunk. That ordering is the
    // priority rule and it is free: the player is standing in the near ring,
    // and a horizon that arrives a second late is a horizon nobody watched
    // arrive.
    let mut dropped_outer = 0usize;
    ring.outer.retain(|(bx, bz), e| {
        if dropped_outer >= 1
            || ((*bx - cx).abs() <= OUTER_RADIUS && (*bz - cz).abs() <= OUTER_RADIUS)
        {
            return true;
        }
        dropped_outer += 1;
        commands.entity(*e).despawn();
        false
    });
    if dropped_outer > 0 {
        // Stream-out is budgeted too — `CLAUDE.md`'s "the teardown spike is
        // the half everyone forgets". A chunk of hulls despawned is a frame's
        // work on its own.
        return;
    }

    for dz in -OUTER_RADIUS..=OUTER_RADIUS {
        for dx in -OUTER_RADIUS..=OUTER_RADIUS {
            // The near ring already owns this chunk at full detail.
            if dx.abs() <= NEAR_RADIUS && dz.abs() <= NEAR_RADIUS {
                continue;
            }
            let key = (cx + dx, cz + dz);
            if ring.outer.contains_key(&key) {
                continue;
            }
            let parent = commands
                .spawn((
                    super::WorldEntity,
                    Transform::IDENTITY,
                    Visibility::default(),
                ))
                .id();
            let cells = (CHUNK_M / terrain::CELL_SIZE) as i32;
            let mut lat = terrain::Lattice::new();
            for iz in 0..cells {
                for ix in 0..cells {
                    let cell_x = key.0 * cells + ix;
                    let cell_z = key.1 * cells + iz;
                    let slot = terrain::scatter_memo(
                        &mut lat,
                        world.seed,
                        &world.table,
                        &world.haven,
                        cell_x,
                        cell_z,
                    );
                    // **Trees only.** A boulder or a barrel at 300 m is a
                    // sub-pixel lump that costs an entity and changes no
                    // silhouette; a tree at 300 m is the treeline, which is
                    // the whole thing this ring exists to draw.
                    if slot.occupant != Occupant::Tree {
                        continue;
                    }
                    let key = cell_key(cell_x as u16, cell_z as u16);
                    spawn_outer_tree(&mut commands, parent, a, &slot, key, &world);
                }
            }
            ring.outer.insert(key, parent);
            return;
        }
    }
}

/// One far tree: a single entity, and everything it deliberately is not.
///
/// A near tree is FOUR entities — a trunk that topples, a canopy that topples
/// beside it, a stump waiting hidden for a cut, and the hull that replaces the
/// first two past [`tree::TREE_LOD_SWAP_M`]. Out here the swap distance is
/// already behind us, so three of those four would spawn only to be culled by
/// a `VisibilityRange` that can never open. What is left is the hull, and it
/// needs no range component at all: nothing else is ever going to be shown in
/// its place.
///
/// **It carries `FellPart::Vanish`, and that is the one thing it shares with a
/// real prop.** Without a `Fellable` a felled tree would stand out here until
/// the chunk happened to be rebuilt, and `harvest_new`'s `Added<Fellable>`
/// pass is also what makes a chunk streaming into already-cleared ground
/// correct on arrival — which matters far more here than in the near ring,
/// because this ring streams 96 chunks. `Vanish` rather than `Trunk`: a hull
/// has no stump to reveal and no sibling canopy to keep in step, and `Trunk`
/// means *speaks for the slot* to the fell cue and the ambience bed
/// (`FellPart::Far`'s own doc has the two bugs that rule was written from).
/// It does not topple — `apply_fell`'s `Vanish` arm takes `Visibility` and
/// never touches `Topple` — so a tree felled at 300 m blinks out rather than
/// falling. At that range it is a few pixels; the near ring is where a topple
/// is a thing anybody sees.
pub fn spawn_outer_tree(
    commands: &mut Commands,
    parent: Entity,
    a: &PropAssets,
    slot: &terrain::Slot,
    key: u32,
    world: &WorldId,
) {
    let variant = (slot.yaw as usize) % a.impostors.len();
    let yaw = slot.yaw as f32 / 256.0 * std::f32::consts::TAU;
    // **`far_ground_y`, never `slot.y`.** Out here the ground a player sees is
    // the 8 m far mesh sitting `FAR_DROP` below the real heightfield, so
    // planting on `slot.y` floats every tree over a valley and buries it on a
    // ridge. `terrain_mesh::far_ground_y` has the arithmetic and the one place
    // it is only an approximation.
    let y = super::terrain_mesh::far_ground_y(world.seed, &world.haven, slot.x, slot.z);
    commands.entity(parent).with_child((
        Fellable {
            key,
            variant,
            base_y: y,
            yaw,
            part: FellPart::Vanish,
            felled: false,
        },
        Mesh3d(a.impostors[variant].clone()),
        MeshMaterial3d(a.foliage.clone()),
        Transform {
            translation: Vec3::new(slot.x, y - SINK_M, slot.z),
            rotation: Quat::from_rotation_y(yaw),
            scale: Vec3::splat(slot.scale),
        },
    ));
}

/// Draw one scatter slot as a child of its chunk.
///
/// **Public because the LOD is a spawn-site claim.** Every gate in this repo
/// that touches a tree measures a mesh, and a mesh cannot say whether the
/// entity carrying it was given a [`VisibilityRange`] — drop that component
/// and the impostor is simply a fourth tree drawn on top of the first, with
/// `tests/tree.rs` still green because the arithmetic it checks never
/// changed. `tests/tree_lod.rs` drives this function instead, which is the
/// only way to assert what actually reaches the ECS. `CLAUDE.md`'s trap list
/// says it in one line: a spawn is not type-checked, so a bundle is a claim
/// you have to run.
pub fn spawn_slot(
    commands: &mut Commands,
    parent: Entity,
    a: &PropAssets,
    slot: &terrain::Slot,
    key: u32,
    lod: &tree::TreeLod,
) {
    let yaw = slot.yaw as f32 / 256.0 * std::f32::consts::TAU;
    // Which mesh, which material, and how far the mesh's own origin sits
    // above the ground — the browser's `lift`, kept because these meshes are
    // centred and the slot's y is the surface.
    let mut variant = 0usize;
    // The lift comes off `archetype_lift` rather than being written here, so
    // the number the draw uses and the number `tests/greybox.rs` checks
    // against `OCCUPANT_TOP_M` are the same number.
    let lift = archetype_lift(slot.occupant);
    let (mesh, material) = match slot.occupant {
        Occupant::Tree => {
            variant = (slot.yaw as usize) % a.pines.len();
            (a.pines[variant].clone(), a.bark.clone())
        }
        Occupant::StoneNode => (a.blob.clone(), a.ore_stone.clone()),
        Occupant::MetalNode => (a.blob.clone(), a.ore_metal.clone()),
        Occupant::SulfurNode => (a.blob.clone(), a.ore_sulfur.clone()),
        Occupant::Bush => (a.bush.clone(), a.foliage.clone()),
        Occupant::Rock => (a.boulder.clone(), a.rock.clone()),
        Occupant::BarrelSlot => (a.barrel.clone(), a.metal.clone()),
        Occupant::CrateSlot => (a.crate_box.clone(), a.wood.clone()),
        Occupant::CacheSlot => (a.cache_box.clone(), a.wood.clone()),
        Occupant::HavenShelter => (a.shelter.clone(), a.stone.clone()),
        Occupant::WaystationCanopy => (a.canopy.clone(), a.wood.clone()),
        Occupant::None => return,
    };
    let sink = SINK_M;
    let transform = Transform {
        translation: Vec3::new(slot.x, slot.y + lift * slot.scale - sink, slot.z),
        rotation: Quat::from_rotation_y(yaw),
        scale: Vec3::splat(slot.scale),
    };
    // **Every gatherable node carries the harvested bit, not just trees.**
    // `gather::node_index` covers `Occupant` 1..=5 and a barrel shares the same
    // `SlotLives` entry and the same `EV_SLOT_HARVESTED`, so a smashed barrel
    // and a mined ore node are as gone as a felled pine. Tagging only trees
    // left both standing on screen after the server had removed them.
    let harvestable = matches!(
        slot.occupant,
        Occupant::Tree
            | Occupant::StoneNode
            | Occupant::MetalNode
            | Occupant::SulfurNode
            | Occupant::Bush
            | Occupant::Rock
            | Occupant::BarrelSlot
    );
    let is_tree = slot.occupant == Occupant::Tree;
    // One constructor for all three parts: every field but `part` is the same
    // for the trunk, the canopy and the stump, and writing them out three
    // times is how the trunk and the canopy would eventually disagree about
    // the yaw they compose their fall onto.
    let fellable = |part: FellPart| Fellable {
        key,
        variant,
        base_y: transform.translation.y,
        yaw,
        part,
        felled: false,
    };
    let mut e = commands.entity(parent);
    // Three spawn shapes, split rather than merged with a conditional
    // component: only a tree topples, so only a tree carries a [`Topple`],
    // and `fall`'s query is then exactly the set of things that can move. A
    // boulder holding a disabled `Topple` would be a boulder the animation
    // iterates every frame forever to decide it is not animating.
    if is_tree {
        e.with_child((
            fellable(FellPart::Trunk),
            Topple { t: -1.0 },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            lod.near.clone(),
            transform,
        ));
    } else if harvestable {
        e.with_child((
            fellable(FellPart::Vanish),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            transform,
        ));
    } else {
        e.with_child((Mesh3d(mesh), MeshMaterial3d(material), transform));
    }
    // The stump, spawned WITH the tree and hidden until the cut lands.
    //
    // **Spawned up front rather than at fell time, because fell time has no
    // `Commands`.** `apply_fell` runs behind a predicate so it can be gated
    // headless (`tests/fell.rs`), and handing it a command buffer would put
    // entity creation inside the one function whose whole purpose is to be
    // callable without a world to create things in. The cost is one hidden
    // entity per tree in the ring — ~328 at the measured p90 — against a
    // `Visibility` flip on a mesh that was going to be built anyway.
    if is_tree {
        e.with_child((
            fellable(FellPart::Stump),
            Mesh3d(a.stump.clone()),
            MeshMaterial3d(a.wood.clone()),
            Transform {
                translation: Vec3::new(
                    slot.x,
                    transform.translation.y + STUMP_LIFT_M * slot.scale,
                    slot.z,
                ),
                rotation: Quat::from_rotation_y(yaw),
                scale: Vec3::splat(slot.scale),
            },
            Visibility::Hidden,
        ));
    }
    // The needle half rides as a SIBLING carrying the same transform, never as
    // a child of the trunk entity — and felling v0 changed the reason without
    // changing the arrangement. It used to be that the swap LIFTED the bark to
    // the stump's height and a child would inherit that lift. Now the trunk
    // rotates, and a child canopy would inherit the topple and then apply its
    // own on top of it: the needles would swing through twice the angle and
    // leave the tree. `tests/fell.rs` compares the two poses for exactly that.
    //
    // What makes two independent entities agree is that neither stores the
    // bearing: both derive it from the cell key (`fell_bearing`), which is
    // also what makes two CLIENTS agree.
    if is_tree {
        e.with_child((
            fellable(FellPart::Canopy),
            Topple { t: -1.0 },
            Mesh3d(a.needles[variant].clone()),
            MeshMaterial3d(a.needle.clone()),
            lod.near.clone(),
            transform,
        ));
    }
    // The far half of the pair: one opaque hull standing in for both of the
    // entities above past `tree::TREE_LOD_SWAP_M`.
    //
    // **It topples with the trunk and is otherwise nobody**, which is what
    // `FellPart::Far` is for: it shares the trunk's pivot and bearing because
    // it carries the same `transform` and the same cell key, and it is
    // excluded by name everywhere `Trunk` means "the entity that speaks for
    // this slot" (the fell cue, the ambience bed's cover count). The stump is
    // deliberately left at full detail: it is a few dozen triangles and it
    // only exists after a cut.
    //
    // The material is `foliage` — untextured, white, vertex-coloured — which
    // is what the hull's own colours want and what makes the far forest ONE
    // material at six meshes. The bush already banks on that shader path.
    if is_tree {
        e.with_child((
            fellable(FellPart::Far),
            Topple { t: -1.0 },
            Mesh3d(a.impostors[variant].clone()),
            MeshMaterial3d(a.foliage.clone()),
            lod.far.clone(),
            transform,
        ));
    }
}

/// Fell the slot the sim retired, and restore it when the slot respawns.
///
/// **This is the whole visible half of gathering.** The sim has owned chopping
/// for a long time — ten swings with the right tool, a yield table, a
/// harvested bit and a respawn timer, all authoritative and all on the wire —
/// and the native client's entire contribution was to keep drawing the tree.
/// The most physical verb in the game read as a rendering bug.
///
/// It then read as a *pop*: the first cut swapped the tree's mesh for a stump
/// in one frame and hid the canopy, so the most physical verb in the game
/// happened between two frames with nothing in between. The topple is
/// [`fall`]; this is the discrete state change that starts and reverses it.
pub fn harvest(
    net: NonSend<Net>,
    q: Query<(
        &mut Fellable,
        Option<&mut Topple>,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    if q.is_empty() {
        return;
    }
    // The session is read through a predicate so the swap below is testable
    // without a socket. `HarvestedSet` is the authority; this is the only
    // place it is consulted.
    let core = &net.session.core;
    apply_fell(q, &|key| core.harvested.contains(key));
}

/// The bit that means the harvested set moved — the only thing [`harvest`]
/// reads, and the only thing that can change its answer for a prop already in
/// the ring.
///
/// `client_core::core` raises it at every place that writes `harvested`, and
/// that is a GATED claim rather than a read of the file: its own
/// `stream_tracks_the_harvested_set` drives an insert, a remove, a reset and a
/// re-insert through `on_stream` and asserts the returned word each time. So a
/// frame without this bit is a frame where the sweep below is guaranteed to
/// change nothing for a prop already in the ring.
pub const HARVEST_APPLIED: u32 = client_core::core::APPLIED_SLOTS;

/// Run condition for [`harvest`]: the wire moved the harvested set.
///
/// **`HarvestedSet::contains` is a linear scan**, so the sweep is
/// `fellables × harvested_len` and it ran unconditionally: measured at the
/// caps that `sim_core::limits` actually permits, 1,500 props against a full
/// 16,384-entry set is 2.34 ms of comparing on a frame where nothing changed —
/// on its own, under 60 fps. A denser ring is the case that gets worse, and
/// the ring is what this pass is making denser.
///
/// The `Added` half below is what keeps this exact: a prop streaming into a
/// cell that was harvested before the player arrived gets no bit of its own.
pub fn harvest_changed(feed: Res<super::feed::Feed>) -> bool {
    feed.applied & HARVEST_APPLIED != 0
}

/// The same swap, for props that arrived THIS frame.
///
/// Runs unconditionally and costs one query over an empty set on almost every
/// frame. It is the half [`harvest_changed`] cannot cover: the harvested set is
/// seeded at join and topped up by the change slice, so a tree spawned by
/// `stream` into a cell that was already felled sees no `APPLIED_SLOTS` bit of
/// its own and would stand until the next unrelated harvest anywhere on the
/// shard. `apply_fell` is idempotent — it compares against `Fellable::felled`
/// and returns early when nothing moved — so an entity caught by both systems
/// on one frame is swapped once and touched twice.
pub fn harvest_new(
    net: NonSend<Net>,
    q: Query<
        (
            &mut Fellable,
            Option<&mut Topple>,
            &mut Transform,
            &mut Visibility,
        ),
        Added<Fellable>,
    >,
) {
    if q.is_empty() {
        return;
    }
    let core = &net.session.core;
    apply_fell_added(q, &|key| core.harvested.contains(key));
}

/// Half a metre of stump lift: the mesh is centred on its own axis while the
/// slot's `y` is the ground. `web/src/props.js` ships the same 0.17.
pub const STUMP_LIFT_M: f32 = 0.17;

/// How long a tree takes to go from standing to flat, seconds.
///
/// Knob (`DECISIONS.md` §open, "felling v0"). The browser's `FELL_TICKS` was
/// 33 at 30 Hz — 1.1 s — and this is deliberately slower: the browser then
/// SANK the trunk out of the world over 60 more ticks, so its fall only had
/// to hold the eye until the vanish covered for it. Ours has to survive being
/// looked at afterwards, and 1.1 s reads as a swat rather than as a tree.
pub const FELL_FALL_S: f32 = 1.6;

/// Hash channel for the fall bearing. Its own channel, never the scatter's:
/// `spawn_slot` already derives the tree's YAW from `slot.yaw`, and a bearing
/// sharing that source would make every tree fall the way it happens to face.
const CH_FELL: u32 = 0x_46_45_4c_4c;

/// Which way this slot's tree goes down, radians, as a pure function of the
/// cell key.
///
/// **Derived rather than stored, because two entities have to agree without
/// talking.** The trunk and the canopy are siblings, not parent and child
/// (`spawn_slot` says why), so nothing carries a shared bearing between them.
/// Both call this with the same key and get the same float — which is also
/// what makes two *clients* agree, since the key is the sim's own
/// `gather::cell_key`. A bearing rolled at fell time would differ per client
/// and two players would watch the same tree land two ways.
pub fn fell_bearing(key: u32) -> f32 {
    hash01(key, CH_FELL) * std::f32::consts::TAU
}

/// The pose of a slot part `t` seconds into its topple.
///
/// Split from the system so the curve is gated as arithmetic rather than by
/// looking at it — `crates/client/tests/fell.rs` drives it at both ends and
/// in the middle. `RENDER.md`'s framing: what may be gated about a frame is
/// arithmetic, and an angle is arithmetic.
///
/// **The curve is quadratic and that is not laziness.** A rigid rod pivoting
/// under gravity obeys `θ'' = (3g/2L)·sin θ`, which starts at zero angular
/// acceleration and whips at the end; integrating it per frame would be a
/// state variable and a stiffness knob for a motion that lasts 1.6 s. `p²`
/// has the property that actually reads — slow off the stump, fastest at the
/// ground — and it is one expression with no state.
pub fn fell_rotation(yaw: f32, bearing: f32, t: f32) -> Quat {
    let p = (t / FELL_FALL_S).clamp(0.0, 1.0);
    let angle = std::f32::consts::FRAC_PI_2 * p * p;
    // The axis is horizontal and perpendicular to the bearing, so tipping
    // about it walks the trunk's +Y onto the bearing exactly at 90°.
    let (sb, cb) = (bearing.sin(), bearing.cos());
    Quat::from_axis_angle(Vec3::new(cb, 0.0, -sb), angle) * Quat::from_rotation_y(yaw)
}

/// Advance every topple in flight.
///
/// **Only the ones in flight.** [`Topple::t`] is negative while a slot is
/// standing and clamps at [`FELL_FALL_S`] once it is down, so this touches an
/// entity
/// on the ~96 frames of its fall and never again — a forest of felled trees
/// costs this system one comparison each. That matters because the client is
/// held to the sim thread's discipline (`CLAUDE.md`'s client trap) and this
/// runs every frame over every prop in the ring.
pub fn fall(time: Res<Time>, mut q: Query<(&Fellable, &mut Topple, &mut Transform)>) {
    let dt = time.delta_secs();
    for (f, mut top, mut t) in q.iter_mut() {
        if top.t < 0.0 || top.t >= FELL_FALL_S {
            continue;
        }
        top.t = (top.t + dt).min(FELL_FALL_S);
        t.rotation = fell_rotation(f.yaw, fell_bearing(f.key), top.t);
    }
}

/// The state change itself, with the wire behind a predicate.
///
/// Split out so it can be exercised headless — `crates/client/tests/fell.rs`
/// drives it with a fixed key set and asserts the pose and the visibility of
/// all four parts, in both directions. Without that split the only way to test
/// a felled tree would be to chop one over a live socket, which is not a gate.
///
/// **No mesh or material is touched here any more, and that is the shape of
/// the change.** A tree that topples keeps being the mesh it already was — it
/// is the same trunk, lying down — so the trunk's 6 k triangles are the ones
/// that were already in the frame and a felled forest costs no more than a
/// standing one. What used to be a mesh swap on one entity is now a pose on
/// the trunk and a `Visibility` on a stump that was spawned with it.
pub fn apply_fell(
    q: Query<(
        &mut Fellable,
        Option<&mut Topple>,
        &mut Transform,
        &mut Visibility,
    )>,
    harvested: &dyn Fn(u32) -> bool,
) {
    apply_fell_in(q, harvested);
}

/// [`apply_fell`] over the `Added<Fellable>` query — same body, one filter.
pub fn apply_fell_added(
    q: Query<
        (
            &mut Fellable,
            Option<&mut Topple>,
            &mut Transform,
            &mut Visibility,
        ),
        Added<Fellable>,
    >,
    harvested: &dyn Fn(u32) -> bool,
) {
    apply_fell_in(q, harvested);
}

fn apply_fell_in<F: bevy::ecs::query::QueryFilter>(
    mut q: Query<
        (
            &mut Fellable,
            Option<&mut Topple>,
            &mut Transform,
            &mut Visibility,
        ),
        F,
    >,
    harvested: &dyn Fn(u32) -> bool,
) {
    for (mut f, top, mut t, mut vis) in q.iter_mut() {
        let felled = harvested(f.key);
        if felled == f.felled {
            continue;
        }
        f.felled = felled;
        match f.part {
            // A rock that stops being a rock has nothing to animate.
            FellPart::Vanish => {
                *vis = if felled {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
            }
            // Revealed by the cut, and revealed INSTANTLY rather than when the
            // trunk lands: the stump is what the saw leaves behind, so it is
            // there from the first frame the trunk starts to lean.
            FellPart::Stump => {
                *vis = if felled {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            FellPart::Trunk | FellPart::Canopy | FellPart::Far => {
                // `Option` because only these two parts carry a [`Topple`].
                // A missing one is a spawn-site bug rather than a state to
                // handle, so it leaves the pose alone instead of guessing.
                let Some(mut top) = top else { continue };
                if felled {
                    // Start the topple. `fall` owns the pose from here.
                    top.t = 0.0;
                } else {
                    // Respawned: back upright, and back to "not falling" so
                    // the next chop starts from the top of the curve rather
                    // than from wherever the last one ended.
                    top.t = -1.0;
                    t.rotation = Quat::from_rotation_y(f.yaw);
                }
            }
        }
    }
}

impl PropAssets {
    /// Every material this pool builds, by name.
    ///
    /// **Exists for `tests/fresnel.rs` and for nothing else.** Every
    /// `reflectance` in the client was authored 8–70× under physical because
    /// the field is a remap and not a slider (`render::fresnel`), and the
    /// defect is invisible to every other gate here: it changes no triangle, no
    /// handle, no bit on the wire. The only way to catch the next one is to
    /// read back what actually reached the asset store and ask what F0 it
    /// delivers, which needs the handles.
    ///
    /// Named rather than indexed so a failure says *which* surface is wrong.
    pub fn materials(&self) -> [(&'static str, &Handle<StandardMaterial>); 10] {
        [
            ("foliage", &self.foliage),
            ("needle", &self.needle),
            ("rock", &self.rock),
            ("ore_stone", &self.ore_stone),
            ("ore_metal", &self.ore_metal),
            ("ore_sulfur", &self.ore_sulfur),
            ("wood", &self.wood),
            ("metal", &self.metal),
            ("stone", &self.stone),
            ("bark", &self.bark),
        ]
    }

    /// Read-only handles the fell gate compares against.
    pub fn stump_mesh(&self) -> &Handle<Mesh> {
        &self.stump
    }
    pub fn pine_mesh(&self, variant: usize) -> &Handle<Mesh> {
        &self.pines[variant % self.pines.len()]
    }
    pub fn pine_variants(&self) -> usize {
        self.pines.len()
    }
    /// The canopy half of a variant. Indexed modulo the pool for the reason
    /// `pine_mesh` is: the two arrays are built from one `conifers` vector, so
    /// a variant that is valid for one is valid for the other, and a raw index
    /// would panic the client rather than wrap if that ever stopped holding.
    pub fn needle_mesh(&self, variant: usize) -> &Handle<Mesh> {
        &self.needles[variant % self.needles.len()]
    }
    pub fn impostor_mesh(&self, variant: usize) -> &Handle<Mesh> {
        &self.impostors[variant % self.impostors.len()]
    }
    pub fn wood_material(&self) -> &Handle<StandardMaterial> {
        &self.wood
    }
    /// The standing tree's trunk material. `foliage_material` used to be this
    /// and no longer is: the bark half wears a photograph now, so the fell
    /// gate's "a restored tree is a tree again" assertion compares against
    /// this handle.
    pub fn bark_material(&self) -> &Handle<StandardMaterial> {
        &self.bark
    }
    pub fn foliage_material(&self) -> &Handle<StandardMaterial> {
        &self.foliage
    }
}
