//! The near-ground population — `ART.md`'s single largest structural gap,
//! and the one no shader closes.
//!
//! "The ground is not a surface, it is a population": grass reads as
//! thousands of individual lit blades standing 20–40 cm, not as a textured
//! plane. The reference set's near-ground neighbour contrast is 6.3 luma per
//! pixel and the browser client's was 0.26 — a 24× gap that six visual passes
//! of shader work never moved, because the mechanism is geometry.
//!
//! Placement is `sim_core::terrain::clutter_fill`, which already exists,
//! already runs natively, and is already gated: `crates/sim-core/tests/
//! clutter.rs` measures the largest bare disc inside 15 m against rule 4's
//! bound. It has simply never been drawn by this client.
//!
//! **One mesh per tile, not one entity per element.** A tile peaks at 721
//! elements and the ring is 25 tiles; 18,000 entities would cost more in ECS
//! traversal than the triangles cost to draw. Baking each tile's elements
//! into one buffer is the same trick the browser's `ClutterField` used and it
//! keeps the whole ring at 25 draws.

use bevy::light::NotShadowCaster;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::terrain::{
    self, Clutter, ClutterElem, CLUTTER_PER_TILE, CLUTTER_TILE_M, SKIRT_PER_TILE,
};

use super::props::{hash01, linear, Soup};
use super::{Eye, WorldId};

/// Tiles either side of the player's own — a 7×7 ring, 48–64 m to an edge.
///
/// **Three, so the grass can fade instead of ending.** At two the ring's
/// nearest edge was 32 m away and a tile appeared or vanished whole as the
/// player crossed a boundary — the square, popping edge the reference game
/// never shows (its grass fades with distance, to 100 m by default). The
/// foliage shader now sinks each card into the ground between
/// `foliage::GRASS_FADE_START_M` and `GRASS_FADE_END_M`, and the end sits
/// inside this ring's nearest edge, so a tile only ever streams where its
/// grass has already gone.
pub const CLUTTER_RING: i32 = 3;
/// Tiles either side of the player's own whose grass casts a shadow. **(knob)**
///
/// Only the near ones: the first cascade (`rig::CASCADE_FIRST_M`) is fine
/// enough to resolve a blade, and past it a blade's shadow is acne rather
/// than shade. The reference game's "grass shadows" are a near-field effect
/// for the same reason.
pub const GRASS_SHADOW_TILES: i32 = 1;
/// Tiles filled per frame. The fill is 721 hash draws and a few thousand
/// triangles; one a frame keeps the spike off the frame the player turns on.
pub const CLUTTER_FILLS_PER_FRAME: usize = 1;
/// A tuft's blade height at scale 1, metres — the top of `ART.md` §1's
/// measured 20–40 cm band. A card is twice as wide as it is tall
/// (`CARD_ASPECT`), so at 0.40 a tuft is 0.8 m across on a 0.64 m cell and
/// neighbours overlap into turf; at 0.34 the soil showed between every one
/// of them (2026-09-28, against the reference game's meadows).
pub const TUFT_H: f32 = 0.40;

/// A brush clump's height at scale 1, metres. **(knob)**
///
/// **The understory band, and the number is chosen against what already
/// exists rather than picked.** Before `Clutter::Brush` the tallest thing this
/// population grew was a 0.34 m tuft and the tallest thing on the forest floor
/// at all was the scatter bush at ~7.9 per hectare, so `reference/PLANTS.md`
/// §2's 0.5–2 m shrub layer was empty in both systems. 0.75 m is the bottom of
/// that band: high enough to break a sightline along the ground and to read as
/// a second storey over the turf, low enough that it never reads as a tree and
/// never hides a player standing up.
///
/// It draws through [`card`] and therefore through the **same masked material
/// and the same grass atlas as a tuft** — which is why this is honestly called
/// brush and not a shrub. `card` scales width with height (`half_w` is
/// `hj * CARD_ASPECT * 0.5`), so a taller card is a proportionally wider one
/// and no leaf on the atlas is stretched — the distortion `BUSH_CARD_HALF`
/// exists to refuse, avoided here for free. A leafy shrub wants the bush atlas
/// and a third material per tile; that is a separate slice, and this one buys
/// the layer's HEIGHT without it.
pub const BRUSH_H: f32 = 0.75;

/// A fern clump's card height at scale 1, metres (it is twice as wide, the
/// grass card's layout). **(knob)**
///
/// The litter channel stands up because `sim-core`'s density law says it
/// grows things (`clutter_richness_at` thickens channels 1 and 2 together).
/// It used to stand up as three tapered brown quads per stick, which read in
/// every frame as spikes driven into the ground; a fern from a photograph is
/// what a forest floor actually grows.
pub const FERN_H: f32 = 0.5;

/// The share of litter elements in a fern colony that carry a fern. **(knob)**
/// Ferns grow in colonies, so the share is modulated by a low-frequency patch
/// field ([`fern_at`]): dense in places, absent between.
pub const FERN_SHARE: f32 = 0.5;

/// The fern card atlas: four composed clumps from Poly Haven `fern_02` (CC0),
/// in the grass atlas's layout (2x2 cells of 512x256, roots on the bottom
/// edge) so [`card`] draws it unchanged. Baked by `ci/bake_fern_atlas.py`.
pub const FERN_ATLAS: &str = "textures/fern_card_albedo.png";

/// Authored colour per kind, sRGB.
///
/// **Grass is no longer in this table and that is the point of the card.** It
/// used to be `TUFT_LO`/`TUFT_HI`, two hex values ramped root-to-tip, and
/// `ART.md` §3's "shadowed side goes cool" was satisfied by authoring it. A
/// photographed tuft carries its own root-to-tip value, its own cool shadow
/// and its own dead-blade yellows measured off a real clump, so authoring any
/// of them again would be a second opinion fighting the first
/// (`props.rs`'s `photo` states the law). What the card takes instead is a
/// per-instance mean-1 grey, which is rule 7's variation and not a colour.
const PEBBLE_C: u32 = 0x8a8880;
const TWIG_C: u32 = 0x5a4630;
const SHARD_C: u32 = 0x7d7a73;
/// A stone lying in the turf: pale, weathered — the reference game's
/// meadows are dotted with them, and they read at any distance a tuft does.
const STONE_C: u32 = 0xa9a59c;

/// A sprig of grass through the forest litter: a tuft's card, shorter.
pub const SPRIG_H: f32 = TUFT_H * 0.75;

#[derive(Resource, Default)]
pub struct ClutterRing {
    built: HashMap<(i32, i32), Entity>,
    material: Option<Handle<StandardMaterial>>,
    /// The grass cards' own material — alpha-MASKED and wearing the atlas, so
    /// it cannot be the one above. A tile therefore draws twice, not once.
    ///
    /// **That is a real change to this file's stated budget and it is worth
    /// naming rather than absorbing**: the header says one mesh per tile keeps
    /// the ring at 25 draws, and it is 50 now. The alternative was to give the
    /// pebbles and twigs UVs into an opaque corner of the grass atlas so one
    /// material could carry both, which would put every pebble through an
    /// alpha test it does not need and tie two unrelated surfaces to one
    /// texture forever. 25 extra draws is the cheaper half of that trade.
    card_material: Option<Handle<super::foliage::FoliageMaterial>>,
    /// The ferns' material: the card material wearing the fern atlas.
    fern_material: Option<Handle<super::foliage::FoliageMaterial>>,
    /// Each tile's card child, so the near ones can cast shadows
    /// ([`GRASS_SHADOW_TILES`]) and the rest not.
    cards: HashMap<(i32, i32), Entity>,
    /// The tile the shadow casters were last chosen around.
    shadow_at: Option<(i32, i32)>,
    /// The ring's lattice memo, kept across tiles: a neighbour's quads and the
    /// ranges' layout stay warm from one fill to the next.
    lat: terrain::Lattice,
}

/// Tiles in a full clutter ring.
pub const RING_TILES: usize = ((2 * CLUTTER_RING + 1) * (2 * CLUTTER_RING + 1)) as usize;

impl ClutterRing {
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
    pub fn is_full(&self) -> bool {
        self.built.len() >= RING_TILES
    }
}

/// One baked clutter tile.
#[derive(Component)]
pub struct Tile(pub i32, pub i32);

// ---------------------------------------------------------------------------
// The grass card — a photograph of a real tuft, on two crossed quads.
// ---------------------------------------------------------------------------

/// The grass card atlas: four photoscanned tufts, 2×2 cells of 512×256.
///
/// **Why a card at all, when this file already builds blades.** Seven tapered
/// quads of one authored colour is what `ART.md` rule 1 calls a flat surface
/// at blade scale wearing a ramp — the silhouette is seven straight edges, the
/// colour is two hex values, and no amount of either reads as turf. A card
/// carries a photograph of ~30 real blades in one quad: the silhouette is
/// measured rather than authored, the value variation inside it is the
/// scan's, and the density per triangle is roughly thirty times higher.
/// `reference/PLANTS.md` §6.4 names this exact gap — *"Grass blade atlas.
/// Blades are vertex-coloured today with no map at all."*
///
/// Baked by `ci/bake_grass_atlas.py` from Poly Haven `grass_medium_01` (CC0);
/// `assets/textures/MANIFEST.md` carries the provenance row.
pub const CARD_ATLAS: &str = "textures/grass_card_albedo.png";
/// Atlas layout. Four cells, so a tuft's crossed quads can each take a
/// different silhouette and no two tufts in a tile need be the same pair.
pub const CARD_COLS: u32 = 2;
pub const CARD_ROWS: u32 = 2;
/// Cells in the atlas — the count `card` hashes into.
pub const CARD_CELLS: u32 = CARD_COLS * CARD_ROWS;

/// Quads per tuft. Three at 60° apart, so the tuft has a silhouette from every
/// yaw instead of vanishing edge-on — the failure a single card has and the
/// reason nobody ships one.
///
/// Three rather than two because two cross at 90° and present their thinnest
/// pair of edges 45° from either, which is where a player standing still is
/// most likely to be looking; three never leaves a gap wider than 60°.
/// Proposed default, not spoken — `DECISIONS.md` §open, grass cards v0.
pub const CARDS_PER_TUFT: u32 = 3;

/// A card's width as a multiple of its height, matching the atlas cell's
/// 512×256. Baked and drawn have to agree or the tuft is stretched, so this
/// is checked against the shipped file by `tests/grass_card.rs`.
pub const CARD_ASPECT: f32 = 2.0;

/// The alpha a card's cutout is tested against.
///
/// The same 0.5 `render::mipmap::MASK_CUT` preserves coverage against. They
/// are two spellings of one number — the mip chain is built to hold the
/// coverage that THIS test draws — and a gate pins them together, because a
/// drift between them thins the grass with distance and looks like an LOD bug.
pub const CARD_ALPHA_CUT: f32 = 0.5;

/// How deep a card's bedding axis sits below its root, as a multiple of the
/// card's own HALF-WIDTH.
///
/// **Proportional, not a fixed depth, and that is the whole subtlety.** The
/// normal law `blade` established blends toward a point below the root, and at
/// the root it blends fully — so the root's normal is the direction from that
/// point to the vertex. A blade's base is a few centimetres wide, so that
/// direction is vertical whatever depth you pick. A card's base is up to
/// `TUFT_H * CARD_ASPECT` across, so at `blade`'s fixed 2 m the corner normals
/// tilt 9.5° off vertical and the root stops being bedded — measured 0.9863
/// against `tests/contact.rs`'s 0.99 floor, which is what caught it.
///
/// Keying the depth to the half-width makes the root angle a constant instead
/// of a function of the card's size: `d = k·w` gives `n.y = k/√(k²+1)`, which
/// is 0.992 at k = 8 for every card at every `scale`. A fixed depth would pass
/// the gate at one tuft size and fail it at another.
pub const CARD_BED: f32 = 8.0;

/// How far a card's baseline sinks below the ground point, as a fraction of
/// its height. `ART.md` rule 2: nothing sits ON the ground. The scan's own
/// roots do most of this work; the sink is what stops a card's straight
/// bottom edge showing as a line on a slope.
pub const CARD_SINK: f32 = 0.04;

/// One tuft as [`CARDS_PER_TUFT`] crossed, alpha-masked, photographed quads.
///
/// The normal law is `blade`'s, deliberately: fully the ground's normal at the
/// root and `BLADE_TIP_BLEND` of it at the tip, so a card is bedded where it
/// meets the turf and shades as itself where it stands in the light. All the
/// reasoning for that ramp is on `blade` and is not repeated.
///
/// The vertex colour is a **mean-1 grey**, not a green ramp. `props.rs`'s
/// `photo` states the law: a surface wearing a photograph keeps the
/// photograph's colour and takes only a per-instance value multiplier, or the
/// authored tint fights the measured one. The root-to-tip value the blade ramp
/// used to author is in the scan already.
fn card(s: &mut Soup, at: Vec3, yaw: f32, seed: u32, h: f32) {
    let root = at - Vec3::Y * h * CARD_SINK;
    // The ground's own macro break-up under the tuft, so a lighter patch of
    // ground grows lighter grass and the turf does not draw a seam over it
    // — the reference game tints its grass with the terrain's biome colour
    // for the same reason (DB54, DB62).
    let ground =
        1.0 + super::terrain_mesh::MACRO_AMP * super::terrain_mesh::macro_noise(at.x, at.z);
    let tint = patch_tint(at.x, at.z).map(|c| c * ground);
    for i in 0..CARDS_PER_TUFT {
        let a = yaw + i as f32 * std::f32::consts::PI / CARDS_PER_TUFT as f32;
        let side = Vec3::new(a.sin(), 0.0, a.cos());
        // Per-card height jitter — rule 7's "no two identical instances", and
        // it also stops three coincident top edges reading as one hard line.
        let hj = h * (0.80 + 0.40 * hash01(seed, i + 5));
        let half_w = hj * CARD_ASPECT * 0.5;
        // Per card, because `half_w` is per card — see `CARD_BED`.
        let up_volume = Some(root - Vec3::Y * half_w * CARD_BED);
        let cell = (hash01(seed, i + 91) * CARD_CELLS as f32) as u32 % CARD_CELLS;
        let (du, dv) = (1.0 / CARD_COLS as f32, 1.0 / CARD_ROWS as f32);
        let (cu, cv) = (
            (cell % CARD_COLS) as f32 * du,
            (cell / CARD_COLS) as f32 * dv,
        );

        let b0 = root - side * half_w;
        let b1 = root + side * half_w;
        let t0 = b0 + Vec3::Y * hj;
        let t1 = b1 + Vec3::Y * hj;
        // V grows downward in image space, so the card's TOP is the cell's top
        // edge and its baseline is `cv + dv`. Getting this backwards plants the
        // tuft upside down, which is obvious in a frame and invisible in a
        // vertex count — `tests/grass_card.rs` asserts the roots are at the
        // bottom of the cell.
        let (uv_b0, uv_b1) = ([cu, cv + dv], [cu + du, cv + dv]);
        let (uv_t0, uv_t1) = ([cu, cv], [cu + du, cv]);

        let v = 0.86 + 0.28 * hash01(seed, i + 13);
        let col = move |_: Vec3| [v * tint[0], v * tint[1], v * tint[2], 1.0];
        let root_y = root.y;
        let ramp = move |p: Vec3| {
            let t = ((p.y - root_y) / hj).clamp(0.0, 1.0);
            1.0 - (1.0 - BLADE_TIP_BLEND) * t
        };
        let start = s.len();
        // Both triangles wind the same way — `tests/contact.rs` holds their
        // facets in one hemisphere and that claim is not weakened by the UVs.
        s.tri_uv(
            [(b0, uv_b0), (t0, uv_t0), (b1, uv_b1)],
            col,
            up_volume,
            ramp,
        );
        s.tri_uv(
            [(b1, uv_b1), (t0, uv_t0), (t1, uv_t1)],
            col,
            up_volume,
            ramp,
        );
        // The foliage shader's handle on this card: how far each corner
        // stands above the root, and one random per card so neighbours do
        // not sway, fade or fall over in lockstep (`foliage.wgsl`).
        let rand = hash01(seed, i + 211);
        s.tag_uv1(start, move |p| [(p.y - root_y).max(0.0), rand]);
    }
}

/// Smooth value noise over the ground plane, in [0, 1]: the patches a meadow
/// is made of, metres across rather than a tuft wide.
fn patch(x: f32, z: f32) -> f32 {
    let octave = |x: f32, z: f32| {
        let (ix, iz) = (x.floor(), z.floor());
        let (fx, fz) = (x - ix, z - iz);
        let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
        let h = |dx: i32, dz: i32| {
            hash01(
                (ix as i32 + dx) as u32,
                ((iz as i32 + dz) as u32) ^ 0x5eed_0f1e,
            )
        };
        let a = h(0, 0) + (h(1, 0) - h(0, 0)) * ux;
        let b = h(0, 1) + (h(1, 1) - h(0, 1)) * ux;
        a + (b - a) * uz
    };
    octave(x / 11.0, z / 11.0) * 0.65 + octave(x / 4.3 + 17.0, z / 4.3 + 5.0) * 0.35
}

/// A grass card's colour multiplier where it stands: straw-yellow in the dry
/// patches, deeper green in the lush ones, the photograph's own colour
/// between. **Not a second authored colour for grass** — a mean-1 shift of
/// the photograph's, the variation the reference game's meadows have and a
/// field of one photograph does not (2026-09-28).
fn patch_tint(x: f32, z: f32) -> [f32; 3] {
    let t = patch(x, z);
    let dry = ((t - 0.55) / 0.25).clamp(0.0, 1.0);
    let lush = ((0.42 - t) / 0.25).clamp(0.0, 1.0);
    [
        1.0 + 0.16 * dry - 0.08 * lush,
        1.0 + 0.03 * dry + 0.06 * lush,
        1.0 - 0.30 * dry - 0.02 * lush,
    ]
}

/// How much of the volume normal a blade's TIP keeps. **(knob)**
///
/// 0 would be the blade's own facet outright, which is the plate-lit look the
/// fully-vertical blend was introduced to kill: seven blades at seven yaws each
/// taking a different sun cosine reads as a pile of foil, not as turf. 1 is
/// what shipped and is the ground's normal, which is the "reads as paint"
/// defect. This keeps most of the volume behaviour and lets a quarter of the
/// blade's own facing through, so a tuft still shades as a mass while its tips
/// separate from the dirt.
///
/// **Invented, and nobody has looked at it** — `DECISIONS.md` §open, clutter
/// contact v0. It is the one number in this slice a person has to judge, and
/// `ART.md` §5's "blades catch a rim of sun at their tips" is what to judge it
/// against.
pub const BLADE_TIP_BLEND: f32 = 0.75;

/// How far a chip's normals are pulled off their facets toward its own
/// centroid. **A 5 cm stone with four hard facets is four flat values, and the
/// visual judge read exactly that**: "stray flat blue triangles poking through
/// it — an engine test surface", and separately the ask to delete or texture
/// "the flat-shaded pebble primitives". Blue is the diagnosis, not a tint —
/// a facet carrying little sun is lit almost entirely by `fill.rs`'s sky half
/// (0.80, 0.85, 0.95 sRGB), so a grey pebble under a hard facet normal comes
/// back blue-grey and does it in four discrete steps.
///
/// The idiom is already in this file for needles and blades: pull the normal
/// toward a volume's field so the surface scatters as a mass rather than as a
/// set of plates. Partial, not 1.0 — a pebble IS angular (`ART.md` rule 1's
/// near-field grain), so it keeps most of a facet's direction and loses only
/// the hard step between one face and the next.
pub const CHIP_VOLUME_BLEND: f32 = 0.55;

/// How far a chip's base ring sits below the ground it is placed on, as a
/// fraction of the chip's own height. `ART.md` rule 2: "a clean intersection
/// edge reads as a decal" — and a chip whose base ring is exactly coplanar
/// with the ground is that edge by construction, which is what the judge
/// named on three frames as props meeting the ground on a razor line.
///
/// This is geometry and not an occlusion term, deliberately. Occlusion belongs
/// to the indirect slot (SSAO already owns it at `rig.rs`); a visibility
/// scalar multiplied into vertex colour would darken direct sun too and buy
/// "grounded" at the price of "washed out".
pub const CHIP_SINK: f32 = 0.30;

/// A flat-ish chip: pebble, shard and twig are all one builder at different
/// proportions, which is also why none of them reads as a sphere.
fn chip(s: &mut Soup, at: Vec3, yaw: f32, size: Vec3, hex: u32, seed: u32) {
    let base = linear(hex);
    let (sy, cy) = (yaw.sin(), yaw.cos());
    let rot = |p: Vec3| Vec3::new(p.x * cy + p.z * sy, p.y, -p.x * sy + p.z * cy);
    // Four corners jittered in plan, one raised apex — an angular chip rather
    // than a box, so its silhouette is not four right angles.
    //
    // The ring is sunk (`CHIP_SINK`): the chip is pushed into the ground
    // rather than stood on it, so the silhouette that meets the terrain is the
    // chip's own taper and never a straight seam at y == ground.
    let sink = size.y * CHIP_SINK;
    let mut c = [Vec3::ZERO; 4];
    for (i, cc) in c.iter_mut().enumerate() {
        let a = i as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
        let r = 0.6 + 0.4 * hash01(seed, i as u32);
        *cc = at + rot(Vec3::new(a.cos() * size.x * r, -sink, a.sin() * size.z * r));
    }
    let apex = at + Vec3::new(0.0, size.y, 0.0);
    let v = 0.8 + 0.4 * hash01(seed, 5);
    let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
    // The volume centre is the chip's own centroid, so the side faces gain an
    // outward-and-up normal field and the four-step read closes.
    let ctr = at + Vec3::new(0.0, (size.y - sink) * 0.5, 0.0);
    for i in 0..4 {
        s.tri(
            c[i],
            apex,
            c[(i + 1) % 4],
            col,
            Some(ctr),
            CHIP_VOLUME_BLEND,
        );
    }
}

/// Corners around a [`stone`]'s base.
const STONE_SIDES: usize = 6;

/// Vertices one [`stone`] holds: its eight facets, unshared.
pub const STONE_VERTS: usize = 24;

/// A loose stone: a low six-cornered lump under a short ridge — scree, not a
/// spike.
///
/// **It replaced [`chip`] for the pebble and the shard** (2026-09-28). A chip
/// is four facets to one apex, which is a square pyramid, and on bare rock —
/// where every clutter cell is a shard — the ground read as a carpet of grey
/// pyramids. A real stone lying on a surface is wider than it is tall and
/// rounded on top, so this one's top is a ridge, not a point: two crests
/// along its long axis, each over three base corners, eight facets in all
/// (`STONE_VERTS`, cheaper than a litter clump). Same sink and the same
/// volume-blended normals as a chip.
fn stone(s: &mut Soup, at: Vec3, yaw: f32, size: Vec3, hex: u32, seed: u32) {
    let base = linear(hex);
    let (sy, cy) = (yaw.sin(), yaw.cos());
    let rot = |p: Vec3| Vec3::new(p.x * cy + p.z * sy, p.y, -p.x * sy + p.z * cy);
    let sink = size.y * CHIP_SINK;
    let mut lo = [Vec3::ZERO; STONE_SIDES];
    for (i, p) in lo.iter_mut().enumerate() {
        let a = i as f32 * std::f32::consts::TAU / STONE_SIDES as f32 + 0.3;
        let r = 0.7 + 0.3 * hash01(seed, i as u32);
        *p = at + rot(Vec3::new(a.cos() * r * size.x, -sink, a.sin() * r * size.z));
    }
    // The ridge: two crests on the long (x) axis, not quite level.
    let crest = |k: u32, side: f32| {
        let reach = 0.3 + 0.15 * hash01(seed, 20 + k);
        let hy = size.y * (0.85 + 0.15 * hash01(seed, 40 + k));
        at + rot(Vec3::new(side * reach * size.x, hy, 0.0))
    };
    let (h0, h1) = (crest(0, 1.0), crest(1, -1.0));
    let v = 0.8 + 0.4 * hash01(seed, 5);
    let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
    let ctr = at + Vec3::new(0.0, (size.y - sink) * 0.4, 0.0);
    let mut tri = |a: Vec3, b: Vec3, c: Vec3| s.tri(a, b, c, col, Some(ctr), CHIP_VOLUME_BLEND);
    // Corners 5, 0, 1 sit round crest 0 (bearing 0); 2, 3, 4 round crest 1.
    tri(lo[5], h0, lo[0]);
    tri(lo[0], h0, lo[1]);
    tri(lo[1], h0, h1);
    tri(lo[1], h1, lo[2]);
    tri(lo[2], h1, lo[3]);
    tri(lo[3], h1, lo[4]);
    tri(lo[4], h1, h0);
    tri(lo[4], h0, lo[5]);
}

/// Share of the rock channel's clutter cells that draw a stone at all.
///
/// Every clutter cell on bare rock is a shard (`terrain::kind_from_splat`), one
/// per 0.64 m, and a stone every 0.64 m is gravel spread by hand. A rock face
/// carries a few loose stones; the rest is the face itself.
pub const SHARD_KEEP: f32 = 0.22;

/// A litter element's fallen stick.
///
/// `Clutter::Twig`'s own definition in `sim-core` is "fallen needles, sticks,
/// cones", and a forest floor is mostly fallen matter. What stands up in it is
/// a fern, where [`fern_at`] says one grows — drawn by the fern material, so
/// `stream` puts it in its own mesh.
fn litter(s: &mut Soup, at: Vec3, yaw: f32, scale: f32, seed: u32) {
    chip(
        s,
        at,
        yaw,
        Vec3::new(0.16, 0.022, 0.03) * scale,
        TWIG_C,
        seed,
    );
}

/// Whether a litter element carries a fern: a colony field (the same smooth
/// patch noise the meadow's tint uses, at its own offset) times
/// [`FERN_SHARE`], rolled on the element's own hash. Visual only.
pub fn fern_at(e: &ClutterElem) -> bool {
    if e.kind != Clutter::Twig {
        return false;
    }
    let colony = ((patch(e.x * 0.8 + 37.0, e.z * 0.8 - 11.0) - 0.30) / 0.2).clamp(0.0, 1.0);
    hash01(seed_of(e), 0xfe41) < FERN_SHARE * colony
}

/// A fern clump: the grass card's crossed quads, wearing the fern atlas.
fn fern(s: &mut Soup, e: &ClutterElem) {
    let at = Vec3::new(e.x, e.y, e.z);
    let yaw = e.yaw as f32 / 256.0 * std::f32::consts::TAU;
    card(s, at, yaw, seed_of(e) ^ 0x0fe4_1000, FERN_H * e.scale);
}

/// An element's hash seed. The element's own cell coordinates would be a
/// better key, but the fill does not return them; the quantized position is
/// stable for the same reason and costs nothing.
fn seed_of(e: &ClutterElem) -> u32 {
    ((e.x * 64.0) as i32 as u32) ^ ((e.z * 64.0) as i32 as u32).rotate_left(13)
}

/// One element's geometry, alone, as a mesh — the same builder `stream` bakes
/// a whole tile through.
///
/// Exists so `tests/contact.rs` can measure the near-ground population's
/// normals and its contact with the ground without standing up an `App`, a
/// GPU or a shard. Rule: this must stay the SAME call as the tile path
/// (`element`), because a gate that measures a parallel builder measures
/// nothing about what ships.
/// Whether a kind draws through the alpha-masked card material rather than the
/// opaque one.
///
/// **A function on the kind, not a list at the call site.** `stream` splits one
/// tile's elements into two meshes by this, and a kind that changes materials
/// without this changing would be drawn by the wrong shader — which for a
/// cutout means a card rendered as an opaque grey quad, and for an opaque
/// solid means an alpha test against a texture it has no UVs for.
pub fn masked(kind: Clutter) -> bool {
    matches!(kind, Clutter::Tuft | Clutter::Brush | Clutter::Sprig)
}

pub fn element_mesh(e: &ClutterElem) -> Mesh {
    let mut s = Soup::default();
    element(&mut s, e);
    // A twig's fern is a second mesh in the tile (its own material); here it
    // follows the stick, so a gate sees the whole element.
    if fern_at(e) {
        fern(&mut s, e);
    }
    s.mesh()
}

fn element(s: &mut Soup, e: &ClutterElem) {
    let at = Vec3::new(e.x, e.y, e.z);
    let yaw = e.yaw as f32 / 256.0 * std::f32::consts::TAU;
    let seed = seed_of(e);
    match e.kind {
        Clutter::None => {}
        Clutter::Tuft => card(s, at, yaw, seed, TUFT_H * e.scale),
        Clutter::Pebble => stone(
            s,
            at,
            yaw,
            Vec3::new(0.06, 0.028, 0.05) * e.scale,
            PEBBLE_C,
            seed,
        ),
        Clutter::Twig => litter(s, at, yaw, e.scale, seed),
        // The same builder as the tuft at more than twice the height — see
        // `BRUSH_H` for why that is the whole of the change, and why this arm
        // must stay beside `Tuft` in `masked` rather than being remembered to.
        Clutter::Brush => card(s, at, yaw, seed, BRUSH_H * e.scale),
        Clutter::Shard => stone(
            s,
            at,
            yaw,
            Vec3::new(0.08, 0.035, 0.06) * e.scale,
            SHARD_C,
            seed,
        ),
        // A stone in the grass: a pebble's shape at two to five times its
        // size, most of them small.
        Clutter::Stone => {
            let k = 0.6 + 1.4 * hash01(seed, 71) * hash01(seed, 72);
            stone(
                s,
                at,
                yaw,
                Vec3::new(0.16, 0.08, 0.12) * e.scale * k,
                STONE_C,
                seed,
            )
        }
        Clutter::Sprig => card(s, at, yaw, seed, SPRIG_H * e.scale),
    }
}

// Eight injected parameters, and the eighth is `AssetServer` for the card
// atlas. A Bevy system's arity is its dependency list rather than a signature
// somebody designed, and the alternative here — a setup system that builds
// both materials up front — would trade one lint for a second place the
// ring's materials can be half-initialised. Same call the nine other
// `render::` systems make.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<ClutterRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut foliage_mats: ResMut<Assets<super::foliage::FoliageMaterial>>,
    foliages: Option<Res<super::foliage::Foliages>>,
    mut buf: Local<Vec<ClutterElem>>,
    world: Res<WorldId>,
    eye: Res<Eye>,
    assets: Res<AssetServer>,
) {
    let Some(foliages) = foliages else { return };
    // The grid stratum AND the skirt stratum, in one buffer. `CLUTTER_TILE_CAP`
    // is the browser's name for exactly this sum and the two fills are
    // documented as sharing a population, so a single allocation holds both.
    if buf.is_empty() {
        buf.resize(CLUTTER_PER_TILE + SKIRT_PER_TILE, terrain::CLUTTER_NONE);
    }
    let material = ring
        .material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.92,
                // A blade is a leaf: an ordinary dielectric. See
                // `render::fresnel` for what 0.12 was actually delivering.
                reflectance: super::fresnel::DIELECTRIC,
                // Blades are single-sided quads and a player walks all the way
                // around them.
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .clone();
    // The cards' material. `AlphaMode::Mask`, never `Blend`: a tile is
    // hundreds of overlapping quads and blending them needs a per-card depth
    // sort that changes with the camera, where masked cards write depth and
    // sort themselves. `props.rs`'s needle material states the same reasoning
    // for the same reason — this is the second population to need it.
    let card_material = ring
        .card_material
        .get_or_insert_with(|| {
            let base = StandardMaterial {
                // WHITE, and the per-card mean-1 grey rides in the vertex
                // colour: the photograph ships its own colour whole
                // (`textures::PropMaps` has the law).
                base_color: Color::WHITE,
                // `textures::atlas`, not a bare `load`: Bevy's default sampler is
                // clamped and linear (right for an atlas) but leaves anisotropy
                // at 1, and a 34 cm card seen from 1.6 m is almost always at a
                // grazing angle.
                base_color_texture: Some(
                    assets.load_with_settings(CARD_ATLAS, super::textures::atlas(true)),
                ),
                alpha_mode: AlphaMode::Mask(CARD_ALPHA_CUT),
                perceptual_roughness: 0.92,
                reflectance: super::fresnel::DIELECTRIC,
                // A card is one quad and the player walks all the way round it.
                double_sided: true,
                cull_mode: None,
                ..default()
            };
            // Wind, trails, the distance fade and the edge-on cut
            // (`foliage.rs`).
            foliage_mats.add(foliages.make(base, super::foliage::Kind::Grass))
        })
        .clone();
    let fern_material = ring
        .fern_material
        .get_or_insert_with(|| {
            let base = StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(
                    assets.load_with_settings(FERN_ATLAS, super::textures::atlas(true)),
                ),
                alpha_mode: AlphaMode::Mask(CARD_ALPHA_CUT),
                perceptual_roughness: 0.9,
                reflectance: super::fresnel::DIELECTRIC,
                double_sided: true,
                cull_mode: None,
                ..default()
            };
            foliage_mats.add(foliages.make(base, super::foliage::Kind::Grass))
        })
        .clone();

    let tx = (eye.pos.x / CLUTTER_TILE_M).floor() as i32;
    let tz = (eye.pos.z / CLUTTER_TILE_M).floor() as i32;

    let mut dropped = 0usize;
    ring.built.retain(|(bx, bz), e| {
        if dropped >= CLUTTER_FILLS_PER_FRAME
            || ((*bx - tx).abs() <= CLUTTER_RING && (*bz - tz).abs() <= CLUTTER_RING)
        {
            return true;
        }
        dropped += 1;
        commands.entity(*e).despawn();
        false
    });
    let ClutterRing { built, cards, .. } = &mut *ring;
    cards.retain(|k, _| built.contains_key(k));

    // The near tiles' grass casts shadows; re-chosen when the player changes
    // tile. A tile filled later is chosen as it spawns, below.
    let near = |k: (i32, i32)| {
        (k.0 - tx).abs() <= GRASS_SHADOW_TILES && (k.1 - tz).abs() <= GRASS_SHADOW_TILES
    };
    if ring.shadow_at != Some((tx, tz)) {
        ring.shadow_at = Some((tx, tz));
        for (k, e) in ring.cards.iter() {
            if near(*k) {
                commands.entity(*e).remove::<NotShadowCaster>();
            } else {
                commands.entity(*e).insert(NotShadowCaster);
            }
        }
    }

    // Nearest first: the tile under the player before the ring's corners.
    let mut order = [(0i32, 0i32); RING_TILES];
    let mut n_order = 0;
    for r in 0..=CLUTTER_RING {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dz.abs()) == r {
                    order[n_order] = (dx, dz);
                    n_order += 1;
                }
            }
        }
    }

    let mut filled = 0usize;
    {
        for &(dx, dz) in &order[..n_order] {
            if filled >= CLUTTER_FILLS_PER_FRAME {
                return;
            }
            let key = (tx + dx, tz + dz);
            if ring.built.contains_key(&key) {
                continue;
            }
            // Two strata, one buffer, one mesh, one draw.
            //
            // **The grid alone cannot pay rule 2, by construction.** It is
            // 0.64 m cells that do not know a boulder is standing in them, so
            // it answers rule 4 (no bare patch) and leaves every prop meeting
            // the ground on a razor-clean line — which is what the visual
            // judge named twice in one report, once as the ask and once as the
            // symptom. `skirt_fill` is the other half: a stratified ring of
            // the SAME four kinds hugging each prop's footprint, clipped to
            // the tile that emits it so a prop straddling an edge is skirted
            // once. It reaches off `occupant_volume`, the same published
            // footprint table everything else measures against, so a prop that
            // changes size drags its skirt with it.
            //
            // It has been in `sim-core` and gated the whole time. The native
            // client simply never called it. Both fill through one call so
            // they share the tile's corner slopes (`clutter_tile_fill_memo`).
            let n = terrain::clutter_tile_fill_memo(
                &mut ring.lat,
                world.seed,
                &world.table,
                &world.haven,
                key.0,
                key.1,
                &mut buf,
            );
            // Two soups, because the grass wears a cutout and nothing else in
            // this file does — see `ClutterRing::card_material`. Split by
            // `masked` rather than by a list here, so a kind that changes
            // material cannot be drawn by the wrong shader.
            let mut solid = Soup::default();
            let mut cards = Soup::default();
            let mut ferns = Soup::default();
            let mut n_solid = 0usize;
            let mut n_cards = 0usize;
            let mut n_ferns = 0usize;
            for e in buf.iter().take(n) {
                if e.kind == Clutter::Shard
                    && hash01(
                        (e.x * 64.0) as i32 as u32,
                        (e.z * 64.0) as i32 as u32 ^ 0x5eed,
                    ) > SHARD_KEEP
                {
                    continue;
                }
                if masked(e.kind) {
                    n_cards += 1;
                    element(&mut cards, e);
                } else {
                    n_solid += 1;
                    element(&mut solid, e);
                }
                if fern_at(e) {
                    n_ferns += 1;
                    fern(&mut ferns, e);
                }
            }
            // The tile entity carries `Tile` and nothing drawable; each mesh
            // hangs off it as a child. `despawn` follows `Children`
            // (`linked_spawn`), so retiring the tile still takes both with it
            // and the retire path above is unchanged.
            let e = commands
                .spawn((
                    super::WorldEntity,
                    Tile(key.0, key.1),
                    Transform::IDENTITY,
                    Visibility::default(),
                ))
                .id();
            if n_solid > 0 {
                commands.entity(e).with_child((
                    Mesh3d(meshes.add(solid.mesh())),
                    MeshMaterial3d(material.clone()),
                    // A blade is two triangles a few centimetres wide. Against
                    // a cascade sized for a 200 m world that is not a shadow,
                    // it is acne — the black wedges under every tuft in the
                    // first native capture. The ground's contact darkening
                    // comes from the blades' own dark bases instead.
                    //
                    // ⚠ That last sentence is the one to distrust: a blade's
                    // dark base darkens the BLADE, never the ground under it,
                    // so nothing here pays `ART.md` rule 2 for the tile. The
                    // ambient half of that debt is SSAO's (`rig.rs`, and it
                    // is enabled — `NOW.md` §0gi item 4's "no SSAO anywhere"
                    // was stale). What is genuinely missing is any occluder
                    // at blade scale, and `NotShadowCaster` is why.
                    NotShadowCaster,
                    Transform::IDENTITY,
                ));
            }
            if n_cards > 0 {
                let c = commands
                    .spawn((
                        Mesh3d(meshes.add(cards.mesh())),
                        MeshMaterial3d(card_material.clone()),
                        Transform::IDENTITY,
                        ChildOf(e),
                    ))
                    .id();
                // `NotShadowCaster` past the near tiles, for the same reason
                // as the solids above, and one more that is specific to a
                // cutout: a masked card in the shadow pass is an alpha test
                // per shadow texel, which for hundreds of overlapping quads is
                // the most expensive thing on the tile. The near ones cast:
                // see [`GRASS_SHADOW_TILES`].
                if !near(key) {
                    commands.entity(c).insert(NotShadowCaster);
                }
                ring.cards.insert(key, c);
            }
            // The ferns: the same card shape in their own material. Never
            // shadow casters — a cutout per shadow texel for an understory
            // that already sits in the canopy's shade.
            if n_ferns > 0 {
                commands.spawn((
                    Mesh3d(meshes.add(ferns.mesh())),
                    MeshMaterial3d(fern_material.clone()),
                    Transform::IDENTITY,
                    NotShadowCaster,
                    ChildOf(e),
                ));
            }
            ring.built.insert(key, e);
            filled += 1;
        }
    }
}
