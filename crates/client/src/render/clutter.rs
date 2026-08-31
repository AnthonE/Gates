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
use super::terrain_mesh::GROUND_ALBEDO;
use super::{Eye, WorldId};

/// Tiles either side of the player's own — a 5×5 ring, 40 m to an edge.
pub const CLUTTER_RING: i32 = 2;
/// Tiles filled per frame. The fill is 721 hash draws and a few thousand
/// triangles; one a frame keeps the spike off the frame the player turns on.
pub const CLUTTER_FILLS_PER_FRAME: usize = 1;
/// A tuft's blade height at scale 1, metres — the browser's, unchanged, and
/// inside `ART.md` §1's measured 20–40 cm band.
pub const TUFT_H: f32 = 0.34;

/// A standing litter stalk's height at scale 1, metres.
///
/// **Why the litter channel stands up at all.** `sim-core`'s own density law
/// says it must: `clutter_richness_at` counts channels 1 and 2 together —
/// *"Grass (channel 1) and forest litter (channel 2) are the ground identities
/// that grow things; sand and rock do not thicken"* — and thickens the
/// population on both. The client then drew every one of those extra elements
/// with `chip` at 16 × 2.2 × 3 cm, an aspect ratio of 7.3 and the flattest
/// thing this file makes. So the sim said *understory* and the mesh said
/// *gravel*, and the capture camera stands on 93 % litter (`NOW.md` §0gp),
/// which is why the visual judge read "flat twig decals and not one 3D clutter
/// mesh" on the near vantage.
///
/// Shorter than `TUFT_H` on purpose: this is standing debris under a canopy —
/// dead stalks, bracken, a fern frond — not turf. `ART.md` §1's measured
/// 20–40 cm band is quoted about GRASS and is not evidence about litter, so
/// this number is not derived from it and is registered as an open knob
/// instead.
pub const FROND_H: f32 = 0.19;

/// Standing stalks per litter clump. Fewer than a tuft's blades were, for the
/// same reason the height is lower — a litter floor is sparser standing matter than
/// turf, and the fallen half of the clump is already carrying its coverage.
pub const FRONDS_PER_CLUMP: u32 = 3;

/// How much brighter a standing litter stalk's tip is than its root.
///
/// The root colour is not authored here: it is `GROUND_ALBEDO[2]`, the island's
/// own forest-litter identity, so a stalk is the same colour as the ground it
/// grew out of and the two cannot drift. That seam was open — every other
/// clutter colour in this file is a hex authored beside the ground rather than
/// from it, and nothing measured the gap — and `ART.md` §3 has no litter row to
/// author one against anyway (its "dirt path" sample pins that identity's hue
/// and saturation, which `GROUND_ALBEDO` already carries).
pub const FROND_TIP_GAIN: f32 = 1.45;

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
    card_material: Option<Handle<StandardMaterial>>,
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

/// A standing quad's root-to-tip colour ramp, linear. The base sits in its own
/// shade and the tip catches a rim of sun; a quad that is one value is rule 1's
/// flat surface at blade scale.
#[derive(Clone, Copy)]
struct Ramp {
    lo: [f32; 3],
    hi: [f32; 3],
}

/// One standing quad's parameters. A struct rather than six arguments because
/// `blade` is now called for two different populations and clippy caps an
/// argument list at seven.
#[derive(Clone, Copy)]
struct Stalk {
    base: Vec3,
    dir: Vec2,
    h: f32,
    lean: f32,
    ramp: Ramp,
}

/// A blade: a tapered quad leaning off vertical, two triangles.
///
/// `v` is the per-quad value jitter — rule 7's "no two identical instances".
fn blade(s: &mut Soup, k: Stalk, v: f32) {
    let (base, dir, h, lean) = (k.base, k.dir, k.h, k.lean);
    // Wider than the first cut's 0.022/0.004. At 2.4 elements per square
    // metre a narrow blade reads as a dark spike standing in mown lawn — the
    // first native capture's exact defect — because the eye is being shown
    // one thin silhouette rather than a mass.
    let half_base = 0.030;
    let half_tip = 0.008;
    let side = Vec3::new(-dir.y, 0.0, dir.x);
    let tip = base + Vec3::new(dir.x * lean * h, h, dir.y * lean * h);

    let b0 = base - side * half_base;
    let b1 = base + side * half_base;
    let t0 = tip - side * half_tip;
    let t1 = tip + side * half_tip;

    let (lo, hi) = (k.ramp.lo, k.ramp.hi);
    let col = move |p: Vec3| {
        let t = ((p.y - base.y) / h).clamp(0.0, 1.0);
        [
            (lo[0] + (hi[0] - lo[0]) * t) * v,
            (lo[1] + (hi[1] - lo[1]) * t) * v,
            (lo[2] + (hi[2] - lo[2]) * t) * v,
            1.0,
        ]
    };
    // Grass scatters light as a mass, not as a set of plates. The first cut
    // blended only 0.72 of the way to vertical and left 0.28 of a FACET
    // normal in. Fully vertical: every blade is lit by the sky above it
    // whichever way it happens to face, which is also what a real blade does
    // once its neighbours have scattered into it.
    //
    // ⚠ **This line has now given TWO false reasons for going fully vertical,
    // and both corrections matter because each pointed at a different fix.**
    //
    // The first said "a blade's two triangles wind opposite ways, so one took
    // the sun and the other went black". They do not: `(b0,t0,b1)` and
    // `(b1,t0,t1)` cross to the same side of the quad, and `tests/contact.rs`
    // computes both facets over a swept blade and holds them in one
    // hemisphere.
    //
    // The second — that the material's `double_sided` flip is what blackens
    // half a tuft — is **also false, and was checked against Bevy's source
    // rather than reasoned about** (2026-08-25). `pbr_functions.wgsl:130-134`
    // wraps that negation in `#ifndef VERTEX_TANGENTS`, and
    // `bevy_pbr/src/render/mesh.rs:2410` pushes `VERTEX_TANGENTS` whenever the
    // layout carries `ATTRIBUTE_TANGENT` — which `Soup::mesh` puts on every
    // clutter tile via `generate_tangents()`. The other `double_sided &&
    // !is_front` in that file is inside `apply_normal_mapping`, and this
    // material has no normal map. **No blade is ever flipped.** Do not
    // "fix" this by turning `double_sided` off; it would change nothing here
    // and would black out the back of every blade for real.
    //
    // **The real defect this line carried, and the fix.** A fully vertical
    // normal is the GROUND's own normal, so every blade was shaded identically
    // to the dirt it stood in — same sun cosine, same hemisphere sample — and
    // albedo was the only thing separating grass from ground. That is the
    // visual judge's "reads as paint" stated as arithmetic, on the layer that
    // fills the bottom half of every frame.
    //
    // One number could not fix it, because BOTH ENDS ARE RIGHT: a blade's root
    // really is bedded in the turf and should shade with it (`ART.md` rule 2 —
    // nothing sits ON the ground), and its tip is a card standing in the light
    // and should shade as itself (rule 1 — no surface may be one flat value).
    // So the blend is a ramp up the blade rather than a constant, which is what
    // `Soup::tri_ramp` exists for.
    let up_volume = Some(base - Vec3::Y * 2.0);
    let root_y = base.y;
    let ramp = move |p: Vec3| {
        let t = ((p.y - root_y) / h).clamp(0.0, 1.0);
        // 1 at the root (the ground's normal), `BLADE_TIP_BLEND` at the tip.
        1.0 - (1.0 - BLADE_TIP_BLEND) * t
    };
    s.tri_ramp(b0, t0, b1, col, up_volume, ramp);
    s.tri_ramp(b1, t0, t1, col, up_volume, ramp);
}

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
        let (cu, cv) = ((cell % CARD_COLS) as f32 * du, (cell / CARD_COLS) as f32 * dv);

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
        let col = move |_: Vec3| [v, v, v, 1.0];
        let root_y = root.y;
        let ramp = move |p: Vec3| {
            let t = ((p.y - root_y) / hj).clamp(0.0, 1.0);
            1.0 - (1.0 - BLADE_TIP_BLEND) * t
        };
        // Both triangles wind the same way — `tests/contact.rs` holds their
        // facets in one hemisphere and that claim is not weakened by the UVs.
        s.tri_uv([(b0, uv_b0), (t0, uv_t0), (b1, uv_b1)], col, up_volume, ramp);
        s.tri_uv([(b1, uv_b1), (t0, uv_t0), (t1, uv_t1)], col, up_volume, ramp);
    }
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

/// A spray of standing quads out of one root — the ONE builder for every
/// standing thing the near-ground population draws, so a tuft of grass and a
/// clump of standing litter cannot diverge in shape, lean or normal handling.
///
/// That last one is the reason this is a single function rather than two
/// similar ones: `blade` forces its normals fully vertical, which `NOW.md`
/// §0gc owns as a defect and will replace with a root-to-tip ramp. Sharing the
/// builder means that fix lands on both populations at once instead of on
/// whichever one its author happened to be looking at.
fn stand(s: &mut Soup, at: Vec3, yaw: f32, seed: u32, n: u32, h: f32, ramp: Ramp) {
    for i in 0..n {
        let a = yaw + i as f32 * 0.897 + hash01(seed, i + 17) * 0.9;
        let dir = Vec2::new(a.sin(), a.cos());
        let spread = 0.03 + 0.05 * hash01(seed, i + 61);
        blade(
            s,
            Stalk {
                base: at + Vec3::new(dir.x * spread, 0.0, dir.y * spread),
                dir,
                h: h * (0.55 + 0.7 * hash01(seed, i + 47)),
                lean: 0.22 + 0.34 * hash01(seed, i + 31),
                ramp,
            },
            0.85 + 0.3 * hash01(seed, i),
        );
    }
}

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

/// A litter clump: the fallen stick this kind has always been, plus the
/// standing stalks the growing channel was owed.
///
/// **The chip stays, and it is emitted first.** `Clutter::Twig`'s own
/// definition in `sim-core` is "fallen needles, sticks, cones", and that read
/// is correct — a forest floor is mostly fallen matter. What it was missing is
/// that a forest floor also has things standing IN the fallen matter, and one
/// element covers 0.4 m² at the shipped density, so a clump is the honest unit.
/// It is the same relationship a tuft already has to one grass element: seven
/// blades from one placement, not one blade.
///
/// Emitting the chip first is load-bearing for the gate rather than for the
/// picture: `tests/contact.rs` measures the chip's four triangles on all three
/// chip-bearing kinds, and it finds them at a fixed offset.
fn litter(s: &mut Soup, at: Vec3, yaw: f32, scale: f32, seed: u32) {
    chip(
        s,
        at,
        yaw,
        Vec3::new(0.16, 0.022, 0.03) * scale,
        TWIG_C,
        seed,
    );
    let root = GROUND_ALBEDO[2];
    // The seed is offset so the stalks' yaws do not correlate with the corner
    // jitter of the stick they stand in — rule 7, at clump scale.
    stand(
        s,
        at,
        yaw,
        seed ^ 0x9e37_79b9,
        FRONDS_PER_CLUMP,
        FROND_H * scale,
        Ramp {
            lo: root,
            hi: [
                root[0] * FROND_TIP_GAIN,
                root[1] * FROND_TIP_GAIN,
                root[2] * FROND_TIP_GAIN,
            ],
        },
    );
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
    matches!(kind, Clutter::Tuft)
}

pub fn element_mesh(e: &ClutterElem) -> Mesh {
    let mut s = Soup::default();
    element(&mut s, e);
    s.mesh()
}

fn element(s: &mut Soup, e: &ClutterElem) {
    let at = Vec3::new(e.x, e.y, e.z);
    let yaw = e.yaw as f32 / 256.0 * std::f32::consts::TAU;
    // The element's own cell coordinates would be a better hash key, but the
    // fill does not return them; the quantized position is stable for the
    // same reason and costs nothing.
    let seed = ((e.x * 64.0) as i32 as u32) ^ ((e.z * 64.0) as i32 as u32).rotate_left(13);
    match e.kind {
        Clutter::None => {}
        Clutter::Tuft => card(s, at, yaw, seed, TUFT_H * e.scale),
        Clutter::Pebble => chip(
            s,
            at,
            yaw,
            Vec3::new(0.05, 0.035, 0.05) * e.scale,
            PEBBLE_C,
            seed,
        ),
        Clutter::Twig => litter(s, at, yaw, e.scale, seed),
        Clutter::Shard => chip(
            s,
            at,
            yaw,
            Vec3::new(0.07, 0.06, 0.055) * e.scale,
            SHARD_C,
            seed,
        ),
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
    mut buf: Local<Vec<ClutterElem>>,
    world: Res<WorldId>,
    eye: Res<Eye>,
    assets: Res<AssetServer>,
) {
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
            materials.add(StandardMaterial {
                // WHITE, and the per-card mean-1 grey rides in the vertex
                // colour: the photograph ships its own colour whole
                // (`textures::PropMaps` has the law).
                base_color: Color::WHITE,
                // `textures::atlas`, not a bare `load`: Bevy's default sampler is
                // clamped and linear (right for an atlas) but leaves anisotropy
                // at 1, and a 34 cm card seen from 1.6 m is almost always at a
                // grazing angle.
                base_color_texture: Some(assets.load_with_settings(
                    CARD_ATLAS,
                    super::textures::atlas(true),
                )),
                alpha_mode: AlphaMode::Mask(CARD_ALPHA_CUT),
                perceptual_roughness: 0.92,
                reflectance: super::fresnel::DIELECTRIC,
                // A card is one quad and the player walks all the way round it.
                double_sided: true,
                cull_mode: None,
                ..default()
            })
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

    let mut filled = 0usize;
    for dz in -CLUTTER_RING..=CLUTTER_RING {
        for dx in -CLUTTER_RING..=CLUTTER_RING {
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
            // client simply never called it.
            let grid = terrain::clutter_fill(world.seed, &world.haven, key.0, key.1, &mut buf);
            let skirt = terrain::skirt_fill(
                world.seed,
                &world.table,
                &world.haven,
                key.0,
                key.1,
                &mut buf[grid..],
            );
            let n = grid + skirt;
            // Two soups, because the grass wears a cutout and nothing else in
            // this file does — see `ClutterRing::card_material`. Split by
            // `masked` rather than by a list here, so a kind that changes
            // material cannot be drawn by the wrong shader.
            let mut solid = Soup::default();
            let mut cards = Soup::default();
            let mut n_solid = 0usize;
            let mut n_cards = 0usize;
            for e in buf.iter().take(n) {
                if masked(e.kind) {
                    n_cards += 1;
                    element(&mut cards, e);
                } else {
                    n_solid += 1;
                    element(&mut solid, e);
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
                commands.entity(e).with_child((
                    Mesh3d(meshes.add(cards.mesh())),
                    MeshMaterial3d(card_material.clone()),
                    // `NotShadowCaster` for the same reason as the solids
                    // above, and one more that is specific to a cutout: a
                    // masked card in the shadow pass is an alpha test per
                    // shadow texel, which for hundreds of overlapping quads is
                    // the most expensive thing on the tile and buys acne.
                    NotShadowCaster,
                    Transform::IDENTITY,
                ));
            }
            ring.built.insert(key, e);
            filled += 1;
        }
    }
}
