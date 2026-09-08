//! What players built: placed pieces, deployables, and the backpacks the
//! dead leave behind.
//!
//! **The gap this closes was the widest one in the native client.** Every
//! one of these three sets was decoded into `ClientCore` and drawn by
//! nothing, so a player standing inside someone's base saw bare terrain, a
//! door they could not see swung silently, and the bag holding their own
//! loot was invisible at the spot they died. `world.rs` puts them on the
//! wire, `core.rs` calls its mirrors "the renderer's truth", and until now
//! there was no renderer.
//!
//! Geometry is `web/src/scene.js`'s `setPiece`/`setDeploy`/`setBags`, carried
//! across rather than re-invented: both clients must agree about where a wall
//! stands, because the sim's collision is a third opinion and it is the one
//! that wins. The dimensions that ARE collision truth come from
//! `sim_core::collide` by import (`WALL_THICKNESS_M`, `DOOR_POST_W_M`) or by
//! calling the sim's own function (`build::column_floor_y` under
//! [`level_base_y`]) rather than by copied literal or copied formula — one
//! class of drift the browser could not close and this can.
//!
//! ## Reconciled, not evented
//!
//! `ClientCore` exposes both a per-message delta (`piece_changes()`) and the
//! whole mirror (`pieces`, `deploys`, `bags`). This reads the mirror. The
//! delta is cheaper per frame and wrong at exactly the moment it matters: a
//! resync walk restates the world, a removal restarts an in-progress walk,
//! and a renderer driven off deltas has to reproduce that state machine
//! correctly or leave a wall standing where the server has none. Reading the
//! set makes that desync impossible by construction, and the set is bounded
//! (`MAX_PIECES` 8192), not unbounded.
//!
//! **No per-frame allocation**, which is why the entity maps carry a
//! generation stamp instead of building a live-key set each frame and
//! diffing it: mark what the mirror still holds, then `retain` the marked.

use bevy::math::Affine2;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::build::{
    BUILD_CELL_M, DMG_BANDS, LEVEL_H_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO,
    LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI, LOC_TRI_XLO_ZLO, MAT_METAL, SHAPE_DOORWAY,
    SHAPE_FRAME, SHAPE_STAIRS, SHAPE_TRI_FLOOR, SHAPE_TRI_FOUNDATION, SHAPE_TRI_ROOF, SHAPE_WALL,
    SHAPE_WINDOW,
};
use sim_core::collide::{
    ColIndex, DOOR_POST_W_M, FRAME_RIM_M, WALL_THICKNESS_M, WINDOW_HEAD_M, WINDOW_SILL_M,
};
use sim_core::deploy::{
    DeployRec, ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE, ARCH_HEARTH, ARCH_RECYCLER,
    ARCH_RESEARCH, ARCH_WORKBENCH, ARCH_WORKBENCH2, ARCH_WORKBENCH3,
};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain;

use super::textures::MapSet;
use super::{Net, WorldId};

/// Plane-piece thickness, metres — **the sim's number, not a second one**
/// (`collide::PLANE_THICKNESS_M`).
///
/// It said "cosmetic — the sim's plane is a surface height and not a slab, so
/// this is how thick we *draw* it and nothing stands on the underside", and
/// every word of that was true until piece flanks v0 (2026-08-21). A plane
/// has sides now: `collide::plane_blocked` stops a body in exactly this band
/// below the walk surface. A drawn thickness that disagreed with it would be
/// a slab you can see and walk through, or one you cannot see and cannot pass
/// — which is the drawn-vs-collided split this whole lane exists to close.
pub const SLAB_T: f32 = sim_core::collide::PLANE_THICKNESS_M;

// ---------------------------------------------------------------------------
// The gaps (gap v1, 2026-09-05, `DECISIONS.md` §open)
//
// **The seam is retired.** Every drawn piece used to stop `SEAM_M` = 0.04 m
// short of its cell on each side, inherited from the browser client's
// `scene.js` with the reason "two abutting floors z-fight along their shared
// edge". They do not: z-fighting is two faces with the SAME normal at the same
// depth, and two abutting slabs share an edge, not a face — their tops meet
// without overlapping and their sides face each other. What the seam actually
// bought was a see-through slit four centimetres wide between every two
// walls in a line, every two floors, and a notch at every outside corner —
// the 2026-09-04 playtest's "building pieces have a gap in them". Pieces abut
// exactly now, and the three places two faces genuinely DID share a plane are
// closed one by one below, each by moving one of them: the edge drop, the
// corner posts, and the diagonal's extra drop.
// ---------------------------------------------------------------------------

/// How far every EDGE part drops below the storey lattice, metres: an edge
/// piece's foot sits this far under its storey base and its head this far
/// under the next.
///
/// **Why a drawn wall is not exactly a storey tall.** A floor's walk surface
/// IS the next storey's base, and a wall centred on its boundary reaches
/// `WALL_THICKNESS_M / 2` in under that slab: its top face and the slab's top
/// face were coplanar over a 12 cm strip along every wall a floor stood on —
/// the same normal at the same depth, which is z-fighting, visible from the
/// storey above as a flicker outlining the room. Dropping every edge part two
/// centimetres puts a wall's head INSIDE the slab above (the slab hangs
/// `SLAB_T` = 0.3 m) and its foot inside the slab below, so no face of an edge
/// piece is coplanar with a plane's. Where no floor stands above, the drop is
/// a two-centimetre shortfall against the storey the sim blocks — under what
/// a player can see or exploit, the bound the old doorway inset was held to —
/// and a wall stacked on a wall still abuts it exactly, foot to head, because
/// both moved. `tests/lattice_geom.rs` holds the drop at exactly this and the
/// stacked seam at zero.
pub const EDGE_DROP_M: f32 = 0.02;

/// How proud of a wall's face its corner post stands, metres, each side.
///
/// Two straight walls meeting at a corner used to leave an OUTSIDE NOTCH:
/// each ran to its boundary and no further, and the square where neither
/// reached — `WALL_THICKNESS_M / 2` on a side, the wall's own thickness past
/// the other's end — was bare. The reference closes this with conditional
/// models (`reference/BUILDING.md` §7d): a corner piece that appears where two
/// blocks meet and belongs to one of them. Ours is a POST at every corner an
/// edge piece ends at, drawn by exactly one of the pieces meeting there
/// ([`post_owner`]), with the bodies running between the posts. Proud by this
/// much so the post's faces are not coplanar with the bodies' — a post flush
/// with the wall would z-fight down its whole height — and because a pilaster
/// is what the reference's corners look like.
pub const POST_PROUD_M: f32 = 0.03;

/// A corner post's width across, metres: the wall's thickness plus the proud
/// rim on both sides. Derived rather than typed, so the knob is the rim.
pub fn post_w() -> f32 {
    WALL_THICKNESS_M + 2.0 * POST_PROUD_M
}

/// How far below the edge-dropped head a DIAGONAL wall's body stops, metres.
/// A diagonal runs corner to corner, its end inside the post a straight wall
/// owns there, and one more centimetre keeps its head under the post's so the
/// two never share a plane. Alone, the shortfall is invisible; a diagonal
/// draws no posts of its own for the same reason a corner cannot hold two.
pub const DIAG_DROP_M: f32 = 0.01;

/// How many build materials the sim can hand us: `MAT_TWIG`…`MAT_METAL`.
/// Derived from the sim's own last code rather than typed, because a table
/// that disagreed with it is exactly what shipped for six days — see [`TIER`].
pub const N_TIERS: usize = MAT_METAL as usize + 1;

/// One tier's cosmetics: which photograph it wears, how densely that
/// photograph is laid on a piece, and the two PBR scalars beside it.
///
/// **A named struct rather than the five-tuple this row was becoming.** It
/// grew [`Tier::tiles_per_m`] when the UV density moved off the mesh, and
/// `CLAUDE.md`'s positional-payload trap is exactly this shape: `gain`,
/// `roughness`, `metallic` and `tiles_per_m` are four `f32` in a row, and
/// every gate this file has — a length check against `MAT_METAL`, a band
/// check per column — stays green when two of them swap places. A field name
/// is the one thing that does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier {
    /// The role name under `assets/textures/`: `<role>_{albedo,normal,rough}.jpg`.
    pub role: &'static str,
    /// Scalar multiply on the photograph's albedo. 1.0 ships the map's
    /// measured colour whole, which is what every row does since twig got a
    /// map of its own — see the table.
    pub gain: f32,
    pub roughness: f32,
    pub metallic: f32,
    /// Tiles of this tier's photograph per metre of piece surface.
    ///
    /// **Per tier, because a photograph has an authored real-world size and
    /// the four sources do not share one.** This was a single
    /// `PIECE_UV_PER_M = 0.55` for every tier until 2026-08-22, which drew
    /// `bark_brown_02` — a 1 m² patch of tree trunk — across 1.82 m of wall,
    /// and that is most of why a twig base read as *carved out of a log*
    /// rather than built: it was a photograph of a tree, at 1.8× life size.
    ///
    /// It lives on the material rather than in the mesh
    /// ([`build_kit`] sets `uv_transform`) because the piece meshes are
    /// deduplicated by `(kind, size)` and SHARED across all four tiers — a
    /// per-tier density baked into UVs would be four times the meshes. The
    /// mesh's UV is therefore literally metres ([`quad`]), and this scales it.
    pub tiles_per_m: f32,
}

/// Twig, wood, stone, metal — the photograph each tier wears, a scalar gain
/// over it, perceptual roughness, metallic, and its tile density.
///
/// **This table had THREE rows against the sim's four materials until
/// 2026-08-16, and every piece in the game drew one rung off.** `spawn_piece`
/// indexed it `.min(2)`, so wood drew in the stone grey, stone drew as a
/// `metallic: 0.85` conductor, metal was right by accident, and twig — which
/// since twig v0 (`build.rs`, *"every piece enters the world as twig and
/// nothing else"*) is the state every piece is FIRST seen in — had no row of
/// its own at all. Nothing caught it because a colour table has no gate; the
/// gate is `tests/pieces.rs` §A now, and it is a length check against
/// `MAT_METAL`, so a fifth material is a red test rather than a silent
/// inheritance.
///
/// **Every row wears a photograph** (`ART.md` rule 1: no surface may be one
/// flat value). Pieces were the last surface in the client still drawing as a
/// flat `base_color` — `props.rs` and `viewmodel.rs` have sampled these same
/// already-shipped, already-manifested CC0 maps since 2026-08-11, and a wall
/// is the largest flat thing a player ever stands in front of. Paths are
/// `MapSet::load`'s, so three of the four handles are ones `PropMaps` already
/// holds: the asset server keys on path plus settings, and these settings are
/// the same, so those cost zero extra residency.
///
/// **Twig wore `bark` until 2026-08-22, and that is the whole of why a base
/// looked like logs** (operator, on a screenshot: *"we need to work on the
/// foundation look its like logs or something?"*). `MAT_TWIG` is the state
/// every piece is FIRST seen in (`build.rs`: *"every piece enters the world
/// as twig and nothing else"*), so the tier nobody chooses is the tier
/// everybody sees — and it was wearing `bark_brown_02`, a photograph of a
/// living tree trunk, complete with moss in its fissures. It read as a tree
/// because it was one. `MANIFEST.md` had said so as an intention since
/// 2026-08-12: *"`bark` doubles as tier 0 (twig) … for want of a
/// straw/lashed-pole set."* The want is filled — `twig` is Poly Haven
/// `bamboo_wall` (CC0), lashed vertical poles, which is what the reference's
/// twig tier actually is. `bark` stays, on trees.
///
/// **The gain is scalar and that is what makes it legal.** `ART.md` §7 bounds
/// a per-channel gain to stretching a source's colour deviation by at most
/// ×1; a scalar has span **1.000 by construction**, the same argument
/// `GROUND_DETAIL_GAIN` is built on. **No row carries one now.** Twig was the
/// only one that did, and its ×1.6 existed to drag `bark`'s luma 0.107 up to
/// 0.171 so a twig frame would read as the PALER of the first two tiers, the
/// way the reference has it. `bamboo_wall` measures **0.167** off the shipped
/// file — pale of `wood`'s 0.141 on its own, with a gain of exactly 1.0. A
/// number that had to be engineered stopped needing to be, which is the tell
/// that the map was the wrong map and not the gain the wrong gain. All four
/// tiers now ship their colour whole, `base_color` white, as the props do.
///
/// Roughness stays a scalar for the reason `props.rs` states: the `_rough`
/// files are greyscale jpgs and `metallic_roughness_texture` is a glTF-packed
/// ORM slot whose B channel is metallic, so binding one here would make every
/// wall a half-metal. That needs a packing step, not a slot assignment.
/// **Every density below is the source's own published physical size, or
/// says why it is not.** `tiles_per_m = 1000 / <the map's authored mm>` is a
/// measurement of the file, not a taste call, and it is the number that makes
/// a photograph read as the material it is a photograph of. Poly Haven
/// publishes it per asset (`api.polyhaven.com/info/<slug>` → `dimensions`);
/// ambientCG publishes it for some assets and not others, and the two rows
/// that inherit the old 0.55 are the two it does not settle — each with the
/// cross-check that says 0.55 is nonetheless the right size for that surface.
const TIER: [Tier; N_TIERS] = [
    // twig · lashed poles. The roughest thing in the game.
    // `bamboo_wall` is authored at 2000 mm → 0.5. At that density its poles
    // draw ~4 cm across, which is a sapling — a frame you could lash by hand,
    // which is the read the tier wants.
    Tier {
        role: "twig",
        gain: 1.0,
        roughness: 0.95,
        metallic: 0.0,
        tiles_per_m: 0.5,
    },
    // wood · `brown_planks_03`, authored at 1000 mm → 1.0. That puts a plank
    // at ~11 cm, against a real deck board's 10–15. At the old shared 0.55 it
    // drew at 20 cm, which is a beam pretending to be a plank.
    Tier {
        role: "wood",
        gain: 1.0,
        roughness: 0.88,
        metallic: 0.0,
        tiles_per_m: 1.0,
    },
    // stone · `Bricks089`, published 2200 × 1100 mm — NOT square, and the
    // file we ship is 1024², so the authored scale did not survive the fetch
    // and there is no honest ratio to take. Keeping 0.55 is the measured
    // answer anyway: it puts a course at ~23 cm over the map's ~8 courses,
    // which is field-stone size.
    Tier {
        role: "stone",
        gain: 1.0,
        roughness: 0.72,
        metallic: 0.0,
        tiles_per_m: 0.55,
    },
    // metal · `CorrugatedSteel009` publishes no physical size at all
    // (`dimensionX`/`Y` are 0). Counted instead: ~22 ribs across the map, so
    // 0.55 lands the corrugation pitch at ~83 mm against a real profile's 76.
    // The one conductor: `ART.md` reads the reference's tier as *sheen* as
    // much as hue, and a tier told apart by colour alone reads as paint.
    Tier {
        role: "metal",
        gain: 1.0,
        roughness: 0.38,
        metallic: 0.85,
        tiles_per_m: 0.55,
    },
];

/// How far through the damage response a band sits: 0 at untouched, 1 at the
/// worst band. The one place the band's 0..=7 becomes a ramp, so the darkening
/// and the roughening cannot disagree about what band 3 means.
pub fn damage_mix(band: u8) -> f32 {
    band.min(DMG_BANDS - 1) as f32 / (DMG_BANDS - 1) as f32
}

/// One tier's cosmetics.
///
/// Public so `tests/pieces.rs` can gate the thing that actually broke — that
/// every material the sim can send resolves to its OWN row, and that the role
/// it names is a file that exists. A private table clamped by `.min(2)` is
/// how four materials became three paints for six days.
pub fn tier(material: u8) -> Tier {
    TIER[(material as usize).min(N_TIERS - 1)]
}

/// What one unit of a piece mesh's UV is worth, in metres of surface: one.
///
/// **A piece mesh carried 0..1 UVs per face until 2026-08-16**, which is one
/// tile stretched over a 3 m wall — the failure `textures.rs` records for the
/// terrain, where a map that does not repeat "reads as no texture at all
/// rather than as an error, so nothing would say so".
///
/// It then carried a shared `0.55` tiles/m for every tier until 2026-08-22,
/// which is the constant this replaces. The density is a property of the
/// PHOTOGRAPH — each has its own authored real-world size — so it belongs on
/// the tier ([`Tier::tiles_per_m`], applied as the material's `uv_transform`),
/// and what belongs in the mesh is the only thing all four tiers agree about:
/// how big the surface is. So the UV is metres, this constant is 1.0, and it
/// stays a named constant rather than a bare `1.0` because it is what
/// `tests/pieces.rs` §B holds the two builders to — the unit is the claim.
pub const PIECE_UV_PER_M: f32 = 1.0;

/// The per-face albedo multipliers a piece mesh carries in its vertices: top,
/// side, bottom. A mean-1 LUMINANCE field, so it modulates the photograph
/// instead of replacing it (`ART.md` §7, `props::tint1`'s form) — and being
/// scalar per vertex, its colour-deviation span is 1.000 like the gain above.
///
/// Why it exists: the reference's foundations do not read as boxes because
/// their top face is a different surface from their sides — a woven mat over
/// poles, planks over beams, flagstone over coursed block. We cannot give a
/// slab two materials without a second draw call, but the value separation is
/// most of the read and it is free in a vertex.
///
/// The three average to exactly 1.0 — `(1.14 + 4 × 1.00 + 0.86) / 6` over a
/// box's six faces — so a wall's delivered mean albedo is still the map's.
pub const FACE_TOP: f32 = 1.14;
pub const FACE_SIDE: f32 = 1.00;
pub const FACE_BOTTOM: f32 = 0.86;

/// A side face's own vertical ramp, foot to head. Rule 2 wants a contact term
/// — *"nothing sits ON the ground; everything sits IN it"* — and a wall whose
/// bottom metre is darker than its top reads as grounded for free. The two
/// average to 1.0 over the face, so the ramp spends none of the mean.
pub const SIDE_FOOT: f32 = 0.90;
pub const SIDE_HEAD: f32 = 1.10;

/// How dark a structure draws at the **worst** damage band, as a fraction of
/// its undamaged albedo (wire v44). Bands in between lerp toward it.
///
/// **Why darkening, and why it is not enough on its own.** A cracked-and-
/// scorched overlay is the right answer and it needs a map nobody has
/// sourced; this is the response available from the material alone. It is a
/// scalar multiply on `base_color`, so it keeps §7's span at 1.000 like every
/// other gain in this file, and it moves in the direction damage actually
/// moves a surface — soot, exposed core, shadowed cracks all darken.
///
/// 0.55 rather than something deeper because of `ART.md` rule 3: no lit
/// surface's shaded face may fall below 0.30 of its lit face, and a wall
/// already carries `FACE_BOTTOM` 0.86 and a 0.90 foot ramp on top of this.
/// 0.55 × 0.86 × 0.90 = 0.43, which still clears it; 0.35 would not.
pub const DMG_DARKEST: f32 = 0.55;
/// …and how much rougher it gets, added to the tier's own roughness at the
/// worst band. A broken surface scatters: the one thing a raided stone wall
/// must not do is keep the clean specular of an intact one.
pub const DMG_ROUGHEN: f32 = 0.12;

/// Deployable stand-ins by archetype (`sim_core::deploy` order: bag, hearth,
/// box, fire, furnace, workbench, door, lock, recycler, research): full size
/// `w × h × d` in metres,
/// colour, roughness, metallic. Cosmetics, same registry row.
/// Public so the DEPLOY GHOST can be the size of what it becomes. Sharing the
/// table is the whole point: a preview sized independently of the thing it
/// previews is a preview of nothing.
pub fn deploy_size(arch: usize) -> Vec3 {
    let [w, h, d] = DEPLOY[arch.min(DEPLOY.len() - 1)].0;
    Vec3::new(w, h, d)
}

// **The sizes are real-world dimensions, not greybox taste** (2026-08-11,
// `DECISIONS.md` §open "deployable proportions"). They were guessed when the
// only consumer was a flat-coloured cuboid, and a guess is invisible on a
// cuboid — every box reads as "a box". It stops being invisible the moment a
// generated mesh is scaled to fit one: the mesh arrives correctly proportioned
// and the row squashes it. Measured instance, same day: a furnace mesh built
// to a stated 1.4 width:height was going into a row of 0.73, a 2x distortion.
//
// The basis is the physical object, which is checkable by anyone and is what
// the reference game's own art is drawn from. It is corroborated where we can
// corroborate it: Meshy's `auto_size` vision estimate returned 0.880 x 0.585 m
// for a steel drum against a real 55-gallon drum's 0.88 x 0.58, so the method
// lands within a centimetre on a standard object.
//
// `BUILD_CELL_M` is 3.0 here and 3 m is also the reference game's foundation,
// so these transfer 1:1 with no scale conversion. Nothing in the sim reads
// this table — `deploy_size` has two callers, the build ghost and its test —
// so a row is a render fact and moving one costs no wire byte and no replay.
const DEPLOY: [([f32; 3], Color, f32, f32); 12] = [
    // 0 · sleeping bag. A human-length bedroll laid flat: it must be longer
    // than a player is tall or it reads as a floor mat. 1.2 was shorter than
    // the body that spawns on it. The 0.32 thickness is the pillow end, not
    // the quilt — measured off the mesh, which was height-clamped to 1.35 m
    // long by an earlier 0.22 that described flat fabric and forgot the head.
    (
        [1.9, 0.32, 0.8],
        Color::srgb(0.478, 0.612, 0.306),
        0.92,
        0.0,
    ), // bag
    // 1 · hearth. Was a perfect 0.9 cube, which is the tell of a number nobody
    // chose — a fireplace is wide, shallow and chest-high, and the depth is
    // what makes it read as a hearth rather than a crate.
    ([1.2, 1.0, 0.6], Color::srgb(0.549, 0.231, 0.180), 0.80, 0.0), // hearth
    // 2 · storage box. A chest is wider than deep; the old square 1.0 x 1.0
    // footprint is why it read as a cube. the reference `storageandtoolchest`
    // shows the same shape language — clearly wide, clearly shallow.
    // ⚠ `box_small` and `box_large` share this archetype, so they draw at one
    // size. Splitting them costs an `ARCH_*` and a `PROTO_VER` bump; filed in
    // §open rather than taken here.
    (
        [1.2, 0.65, 0.7],
        Color::srgb(0.478, 0.361, 0.227),
        0.85,
        0.0,
    ), // box
    // 3 · fire pit. Unchanged, and deliberately: a fire ring is radially
    // symmetric, so a square footprint is correct here and nowhere else.
    ([0.7, 0.4, 0.7], Color::srgb(0.816, 0.439, 0.188), 0.75, 0.0), // fire
    // 4 · furnace. The worst row in the table: 1.1 x 1.5 is TALLER than wide,
    // where a stone smelting forge is squat, wide and shallow. Now 1.37
    // width:height, against the 1.44 a generated mesh reached unprompted.
    (
        [1.3, 0.95, 0.85],
        Color::srgb(0.310, 0.290, 0.271),
        0.70,
        0.0,
    ), // furnace
    // 5 · workbench. Length and height were already right (0.9 is bench
    // height); 0.9 deep was not — a bench you can reach across is ~0.7.
    ([1.6, 0.9, 0.7], Color::srgb(0.631, 0.475, 0.247), 0.85, 0.0), // workbench
    (
        [0.12, 2.1, 0.9],
        Color::srgb(0.420, 0.290, 0.169),
        0.82,
        0.0,
    ), // door
    // 7 · the code lock, and it is **never drawn**: a lock mints no
    // `DeployRec` (it lives in `sim-core/lock.rs`), so nothing indexes
    // here. The row exists so index 8 is the recycler rather than the
    // door, which is `Occupant`'s skipped slot 8 one table over and the
    // same failure if it were left out — no compile error, no golden
    // move, and a recycler drawn as a doorway.
    ([0.2, 0.3, 0.12], Color::srgb(0.235, 0.247, 0.267), 0.6, 0.5), // lock
    // 8 · the recycler (recycler v0). Metal and squat where the furnace
    // is metal and tall, because the two stand next to each other in a
    // base and a silhouette is how you tell them apart at a glance —
    // `ART.md`'s read, applied to a greybox.
    (
        [1.3, 1.15, 0.9],
        Color::srgb(0.325, 0.353, 0.376),
        0.55,
        0.6,
    ), // recycler
    // 9 · the research table (research v0). Waist-high and wide where the
    // workbench is wide and low: they stand side by side in a base and the
    // silhouette is how you tell them apart across a room.
    (
        [1.5, 0.8, 0.8],
        Color::srgb(0.478, 0.427, 0.353),
        0.72,
        0.15,
    ), // research table
    // 10/11 · the bench ladder's upper rungs (bench ladder v0). The rungs
    // rank by silhouette AND material — each taller and deeper than the
    // one below, wood giving way to metal up the ladder — because three
    // benches stand in one base and which is which has to read across a
    // room, greybox or not.
    (
        [1.6, 1.0, 0.8],
        Color::srgb(0.502, 0.443, 0.337),
        0.72,
        0.25,
    ), // workbench 2 — wood frame, metal fittings
    (
        [1.8, 1.1, 0.8],
        Color::srgb(0.361, 0.384, 0.404),
        0.55,
        0.55,
    ), // workbench 3 — the machine bench, metal through
];

/// A locked door wears banded iron: the one bit of door state a passer-by
/// can read off the outside, and the thing they would have to break.
const DOOR_LOCKED: Color = Color::srgb(0.235, 0.247, 0.267);

/// The death backpack (`backpack.rs`) — a low canvas bundle where a body
/// fell, in the sleeping bag's cloth.
const BAG_SIZE: [f32; 3] = [0.6, 0.35, 0.45];
const BAG_COLOR: Color = Color::srgb(0.627, 0.416, 0.235);

/// A grid address: the key both placed stores are addressed by.
pub type Addr = (u16, u16, u8, u8);

/// One drawn thing, and enough of what it was drawn *as* to know when the
/// drawing is stale. An upgrade keeps the address and changes the row; a
/// door swing keeps the row and changes the pose. Both must redraw, and
/// neither shows up as an address appearing or vanishing.
struct Live {
    entity: Entity,
    seen: u64,
    row: u8,
    open: bool,
    locked: bool,
    /// The damage band it was drawn at (wire v44). Without it here, a wall
    /// coming apart in front of the player would keep its first material
    /// forever: this loop redraws on a CHANGE, and a change nobody records
    /// is a change nobody notices.
    dmg: u8,
    /// Which corner posts it was drawn with ([`PostOwn::bits`], gap v1) —
    /// a function of its NEIGHBOURS, re-read on every reconcile, so a wall
    /// is redrawn when the wall beside it arrives or goes and takes or
    /// leaves the post they share. Zero for every shape that has none.
    own: u8,
    /// The plate it was drawn at (build plate v1).
    ///
    /// **It is redraw state for the DEPLOY store and not for the piece one**,
    /// and the asymmetry is the wire's. A piece record carries its own plate
    /// (`protocol::event::write_piece_rec`), so a piece can never be drawn
    /// before the plate that places it. A deployable's does not — it reads its
    /// column's plate out of the piece mirror — so a deploy sync that lands
    /// before its column's pieces would draw a furnace on the terrain the base
    /// is stilted over and, without this, keep it there: the loop redraws on a
    /// change, and the plate was not one of the things it compared.
    plate: i8,
}

/// Shared meshes and materials, built once on first use. A base is hundreds
/// of pieces over five shapes and three materials; one `StandardMaterial`
/// per piece would be one draw call per piece.
/// The shared mesh/material pool for pieces and deployables.
///
/// `pub` for `tests/fire.rs`, the same reason `props::PropAssets` is: the
/// question "does a burnable deployable get a light child" can only be answered
/// by running the real spawn, and the real spawn needs the real pool.
pub struct Kit {
    /// One mesh per (shape, part), sized from [`shape_parts`] — the one
    /// table — and deduplicated by size, so the doorway's two posts share a
    /// mesh and the three slab shapes share one slab.
    shape_mesh: [[Option<Handle<Mesh>>; MAX_PARTS]; N_SHAPES],
    /// The straight edge shapes as ONE mesh each, per corner-post ownership
    /// ([`PostOwn::bits`]): body and owned posts baked together
    /// ([`parts_mesh`]), so a wall, a doorway, a window and a frame are each
    /// one entity with its mesh on it whatever they own. Four variants per
    /// shape, built once.
    edge_mesh: [[Option<Handle<Mesh>>; 4]; N_SHAPES],
    /// The diagonal wall's body ([`diagonal_parts`]) — its own size.
    diag_mesh: Handle<Mesh>,
    /// The footings, for the shapes whose one part is sized PER ADDRESS
    /// rather than per shape — the foundations, whose skirt depth follows
    /// the terrain under their cell ([`foundation_part`]). Indexed by
    /// [`skirt_step`]'s bucket, then by `Tri`, so a hundred foundations at a
    /// hundred depths are `2 × SKIRT_STEPS` meshes and the hammer
    /// highlight's one-entity contract (`Transform` + `Mesh3d`) holds.
    footing: [[Handle<Mesh>; 2]; SKIRT_STEPS],
    /// A ground-storey edge piece's apron, per corner-post ownership
    /// ([`apron_parts`]) — the plinth under its outer half.
    apron: [Handle<Mesh>; 4],
    /// One material per (tier, damage band). `DMG_BANDS` × `N_TIERS` = 32
    /// materials, built once — which is the whole reason the band is 3 bits
    /// and not a float: a continuous damage value would mean a material per
    /// PIECE, and a base is thousands of them.
    tier: [[Handle<StandardMaterial>; DMG_BANDS as usize]; N_TIERS],
    deploy_mesh: [Handle<Mesh>; DEPLOY.len()],
    deploy_mat: [Handle<StandardMaterial>; DEPLOY.len()],
    door_locked: Handle<StandardMaterial>,
    bag_mesh: Handle<Mesh>,
    bag_mat: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
pub struct StructRing {
    pieces: HashMap<Addr, Live>,
    deploys: HashMap<Addr, Live>,
    bags: HashMap<u32, Live>,
    kit: Option<Kit>,
    gen: u64,
}

impl StructRing {
    /// The entity drawing the piece or deployable at `addr`, if one stands.
    ///
    /// Exists for the hammer's highlight, which needs the thing the player is
    /// looking at rather than a second derivation of where it would be. The
    /// addressing arithmetic in `spawn_piece` is subtle enough — edge pieces
    /// are canonical to a cell's low-x or low-z boundary, so the same physical
    /// edge is never addressable twice — that a highlight computing its own
    /// transform would be a second implementation of it, and the wheel's
    /// oldest rule says what that costs.
    pub fn entity_at(&self, addr: Addr, deploy: bool) -> Option<Entity> {
        let map = if deploy { &self.deploys } else { &self.pieces };
        map.get(&addr).map(|l| l.entity)
    }

    /// Standing counts: pieces, deployables, bags. For the gates and for
    /// nothing on the hot path.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.pieces.len(), self.deploys.len(), self.bags.len())
    }
}

/// The world y a piece at `level` sits at, given the terrain under its cell.
///
/// **This is collision truth, not a look, and since 2026-08-15 it is not a
/// copy of it either.** This function restated `collide::col_base_y`'s
/// expression for the whole life of the native client — two crates, one
/// formula, hand-kept in lockstep, which is the exact mirror-drift shape
/// `CLAUDE.md` warns about — and the day the sim's height rule changed (the
/// build lattice, `build::column_floor_y`) is the day the copy would have
/// silently drawn every floor off the surface the sim walks players on. It
/// calls the sim's one implementation now; the storey term is the only
/// arithmetic left here.
///
/// **`plate` is the column's stored floor offset** (build plate v1,
/// `build::plate_for`), and it is a parameter rather than a lookup for the
/// reason every emit in this file takes its address: the caller knows which
/// column it is drawing and where its plate came from — a standing piece
/// carries it on the record, and the ghost asks `plate_for` for the one a
/// placement WOULD get. A lookup here would make a preview impossible to
/// draw, because the column it previews does not exist yet.
pub fn level_base_y(
    seed: u64,
    haven: &terrain::Haven,
    cx: u16,
    cz: u16,
    level: u8,
    plate: i8,
) -> f32 {
    sim_core::build::column_floor_y(seed, haven, cx, cz, plate) + level as f32 * LEVEL_H_M
}

/// The world XZ of a cell's centre.
pub fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    )
}

/// The stair ramp's pitch, radians — the slope of the surface the sim
/// actually stands a player on, derived rather than typed.
///
/// **This was `FRAC_PI_4` until 2026-08-21 and it was right by coincidence.**
/// `collide::piece_ground` walks a rider up `floor + frac·LEVEL_H_M` across
/// `BUILD_CELL_M` of z, so the sim's ramp rises `LEVEL_H_M` per `BUILD_CELL_M`
/// — 45° only while those two numbers are equal, which they are today and
/// which nothing enforces. A literal 45° here is the hand-kept mirror
/// `CLAUDE.md` warns about twice: move the storey height and the drawn tread
/// silently stops being the walked one, with every gate in the repo green.
pub fn stairs_pitch() -> f32 {
    LEVEL_H_M.atan2(BUILD_CELL_M)
}

/// How far a stairs ramp runs along its own slope, metres — the cell's
/// diagonal, so the tread's z extent is exactly the cell and its rise is
/// exactly the storey.
///
/// Shared with the build ghost for the same reason the doorway numbers are: a
/// preview the wrong length is a preview of a different piece. It was a typed
/// 4.15 m, which is 0.09 m short of the diagonal — so the drawn tread stopped
/// 0.066 m below the storey it delivers you to.
pub fn stairs_run() -> f32 {
    (BUILD_CELL_M * BUILD_CELL_M + LEVEL_H_M * LEVEL_H_M).sqrt()
}

/// Where the ramp slab's centre goes, relative to the piece's base point, so
/// that the slab's **top face** is the line the sim walks a player up: the
/// storey's mid-point, pushed half a thickness along the ramp's own DOWNWARD
/// normal.
///
/// **This is the 0.212 m the stairs were wrong by** (measured 2026-08-21,
/// `tests/lattice_geom.rs` §A). The part was centred on the storey's
/// mid-height, which puts the ramp's *centre plane* on the sim's surface and
/// its top face `SLAB_T / (2·cos θ)` above it — a uniform 21 cm of tread the
/// player's feet were inside for the whole climb. A slab has a thickness and
/// the sim's ramp has none, so which of the slab's faces is the surface is a
/// decision rather than a detail: it is the top one, because that is the face
/// a player sees themselves standing on.
///
/// **Half a thickness DOWN-NORMAL, not half a thickness down.** Dropping the
/// centre vertically puts the top face on the right line and slides it
/// `(SLAB_T/2)·sin θ` down that line — the tread then overhangs the cell
/// below by 0.106 m and stops 0.106 m short of the storey it delivers you to.
/// The normal offset moves the face onto the line and leaves it spanning
/// exactly `z ∈ [0, BUILD_CELL_M]`, `y ∈ [0, LEVEL_H_M]`, which is exactly
/// what `collide::piece_ground` ramps over.
pub fn stairs_offset() -> Vec3 {
    let (sin, cos) = stairs_pitch().sin_cos();
    Vec3::new(
        0.0,
        LEVEL_H_M * 0.5 - SLAB_T * 0.5 * cos,
        SLAB_T * 0.5 * sin,
    )
}

/// An edge part's vertical extent under the edge drop: `(height, centre)`
/// for a band `y0..y1` in storey-local metres, with a foot on the storey base
/// moved to `-EDGE_DROP_M` and a head on the storey top to
/// `LEVEL_H_M - EDGE_DROP_M`. A band that starts or ends strictly inside the
/// storey — a sill's top, a lintel's underside, a window's head — is
/// untouched, because those are the heights the sim's shot walk agrees with
/// (`RENDER.md` §8: the frame may not lie about where a player can walk, or
/// an arrow fly).
pub fn edge_band(y0: f32, y1: f32) -> (f32, f32) {
    let lo = if y0 <= 0.0 { -EDGE_DROP_M } else { y0 };
    let hi = if y1 >= LEVEL_H_M {
        LEVEL_H_M - EDGE_DROP_M
    } else {
        y1
    };
    (hi - lo, (hi + lo) * 0.5)
}

/// The doorway's lintel band, storey-local: from the sim's opening head
/// (`collide::DOOR_HEAD_M`, the height `edge_hit` refuses at) to the
/// edge-dropped storey top. Derived, so the underside IS the opening's height
/// rather than a third number that happens to agree with it.
///
/// **Public because the BUILD GHOST draws the same doorway** and the two must
/// not disagree. `RENDER.md` §8 states the stake in one line — the opening is
/// 1.2 m x 2.1 m because `collide::edge_hit` blocks exactly `t` in `[0, 0.9]`
/// and `[2.1, 3.0]`, so "draw it elsewhere and the frame lies about where a
/// player can walk". A ghost that lies about it lies one step earlier, while
/// the player is still deciding.
pub fn lintel_band() -> (f32, f32) {
    let (h, c) = edge_band(sim_core::collide::DOOR_HEAD_M, LEVEL_H_M);
    (c - h * 0.5, c + h * 0.5)
}

/// The doorway's drawn posts: `(width, gap)` — how wide each is along the
/// edge, and how far its centre sits from the piece's mid.
///
/// The sim's post spans `DOOR_POST_W_M` in from the boundary. The drawn one
/// starts where the corner post's face is (`post_w() / 2` in) and runs to the
/// same inner face, so the corner post and the door post together are exactly
/// the sim's solid and share no face: the two abut at the corner post's face
/// and their heads are edge-dropped alike.
pub fn door_post_span() -> (f32, f32) {
    let w = DOOR_POST_W_M - post_w() * 0.5;
    let centre_from_end = post_w() * 0.5 + w * 0.5;
    (w, BUILD_CELL_M * 0.5 - centre_from_end)
}

/// Half the distance between the two drawn door posts' centres, metres.
pub fn door_post_gap() -> f32 {
    door_post_span().1
}

/// The clear span between the posts, metres — the sim's own opening, since
/// the seam went. The lintel spans exactly this.
pub fn door_opening_w() -> f32 {
    (BUILD_CELL_M - 2.0 * DOOR_POST_W_M).max(0.1)
}

/// A box part of an edge piece: the wall's thickness across the boundary,
/// the band `y0..y1` tall under the edge drop, `len` along the edge centred
/// `z` from the piece's mid.
fn edge_part(y0: f32, y1: f32, len: f32, z: f32, role: PartRole) -> Part {
    let (h, c) = edge_band(y0, y1);
    Part {
        size: Vec3::new(WALL_THICKNESS_M, h, len),
        offset: Vec3::new(0.0, c, z),
        x_rot: 0.0,
        kind: PartKind::Box,
        role,
    }
}

/// The two corner posts every straight edge piece emits — square, a wall's
/// thickness plus the proud rim across, the full edge-dropped storey tall,
/// centred on the piece's two corners. The ghost draws both; the standing
/// piece draws the ones [`post_owner`] gives it.
pub fn corner_posts() -> [Part; 2] {
    let (h, c) = edge_band(0.0, LEVEL_H_M);
    let w = post_w();
    let post = |z: f32, role: PartRole| Part {
        size: Vec3::new(w, h, w),
        offset: Vec3::new(0.0, c, z),
        x_rot: 0.0,
        kind: PartKind::Box,
        role,
    };
    [
        post(-BUILD_CELL_M * 0.5, PartRole::PostLow),
        post(BUILD_CELL_M * 0.5, PartRole::PostHigh),
    ]
}

/// One drawn part of a build shape: the box's full extents, its centre, and
/// the pitch it carries.
///
/// The offset is relative to the piece's **base point** — the address's
/// canonical anchor in the low-x/plane orientation: the low-x boundary's
/// midpoint for an edge piece, the cell centre for a body piece, always at
/// the level's base height (`level_base_y`). A low-z edge is the same parts
/// under the root's quarter-turn ([`base_transform`]), exactly as the sim
/// canonicalises the two edges to one shape (`build.rs`).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Part {
    /// Full extents of the box, metres.
    pub size: Vec3,
    /// The box centre, metres, relative to the base point.
    pub offset: Vec3,
    /// Rotation about the part's local X axis, radians — the stairs' ramp
    /// pitch. Zero for every other shape.
    pub x_rot: f32,
    /// What fills the extents: a cuboid, or the NW-cornered half-cell
    /// prism (triangles v0). Orientation is NOT here — the four halves
    /// and both diagonals are one drawn object under the quarter- and
    /// eighth-turns [`base_transform`] hangs on the ROOT, exactly as a
    /// north edge has always been the west edge turned.
    pub kind: PartKind,
    /// Which end of an edge piece this is, for the corner-post rule.
    pub role: PartRole,
}

/// [`Part::role`]: the body of a piece, or the corner post at one of an edge
/// piece's two ends — the low end being the boundary's low-z / low-x corner
/// (part-local −z), the high end the other. `spawn_piece` draws the posts
/// [`post_owner`] gives the piece and the ghost draws both.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PartRole {
    #[default]
    Body,
    PostLow,
    PostHigh,
}

/// [`Part::kind`]'s two fillings.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PartKind {
    #[default]
    Box,
    /// A right triangular prism over the NW half of the unit cell —
    /// hypotenuse from (+x, −z) to (−x, +z) in part-local space — scaled
    /// by `size`. [`tri_prism_unit`] is the one mesh.
    Tri,
}

impl Part {
    /// The part's transform relative to the base point. Scale is untouched,
    /// so the ghost can put the unit cube's scale on top of it.
    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.offset).with_rotation(Quat::from_rotation_x(self.x_rot))
    }
}

/// The most parts any shape emits — the window's sill, header and two
/// jambs, plus its two corner posts (the doorway held this at 3 until
/// catalogue v1, the window at 4 until the posts).
pub const MAX_PARTS: usize = 6;

/// How many shapes the parts table covers: the sim's own last shape, plus
/// one. A shape past it is drawn as the fallback slab, same as one the
/// table has no arm for.
pub const N_SHAPES: usize = SHAPE_TRI_ROOF as usize + 1;

/// Which parts a shape has and where they go — **the one table** both the
/// standing piece ([`spawn_piece`]) and the build ghost (`ghost::track`)
/// emit from (`NOW.md` §0u item 1).
///
/// It used to be written twice, and the copies had already diverged the way
/// only a duplicate can: the ghost drew the doorway's lintel centred at
/// 1.05 m — waist height, hanging in the opening it exists to cap — where
/// the piece drew it at 2.55 m, undersides 0.6 m against 2.1 m. And the
/// ghost previewed stairs as a level plate where the piece pitches its ramp.
/// Both files shared every CONSTANT and still disagreed, because which parts
/// exist and where they go was layout, not dimension, and the layout was the
/// half nothing gated. `crates/client/tests/ghost.rs` now reads this table's
/// own emit against the sim.
///
/// Returns a fixed array and a live count rather than pushing to a `Vec`:
/// the ghost reads it on the frame path, and a [`Part`] is plain data.
pub fn shape_parts(shape: u8) -> ([Part; MAX_PARTS], usize) {
    let none = Part::default();
    // The run between the two corner posts, which is what every straight
    // edge body spans: the bodies abut the posts, never overlap them.
    let between = BUILD_CELL_M - post_w();
    let [lo, hi] = corner_posts();
    let body = PartRole::Body;
    match shape {
        SHAPE_WALL => (
            [
                edge_part(0.0, LEVEL_H_M, between, 0.0, body),
                lo,
                hi,
                none,
                none,
                none,
            ],
            3,
        ),
        SHAPE_DOORWAY => {
            // Two posts hugging each end of the edge, the lintel over what
            // they leave, and the corner posts they abut.
            let (w, gap) = door_post_span();
            let (head, _) = lintel_band();
            (
                [
                    edge_part(0.0, LEVEL_H_M, w, -gap, body),
                    edge_part(0.0, LEVEL_H_M, w, gap, body),
                    // The lintel's underside is the top of the opening the
                    // sim refuses to let a player through (`DOOR_HEAD_M`);
                    // its head is edge-dropped like every other.
                    edge_part(head, LEVEL_H_M, door_opening_w(), 0.0, body),
                    lo,
                    hi,
                    none,
                ],
                5,
            )
        }
        SHAPE_STAIRS => (
            [
                // A ramp through the level. The grid stores no facing, so it
                // always rises toward +Z (cosmetic v0 — the browser's choice
                // too), pitched the way the standing piece has always been.
                Part {
                    size: Vec3::new(BUILD_CELL_M, SLAB_T, stairs_run()),
                    offset: stairs_offset(),
                    x_rot: -stairs_pitch(),
                    kind: PartKind::Box,
                    role: body,
                },
                none,
                none,
                none,
                none,
                none,
            ],
            1,
        ),
        SHAPE_WINDOW => {
            // Sill, header, two jambs around the aperture the sim's shot walk
            // passes, and the corner posts — every extent is the collision
            // constant itself (`collide::window_solid_at`), so the drawn hole
            // IS the hole an arrow threads. The jambs reuse the doorway's
            // post width and gap on purpose: one opening family, one set of
            // numbers.
            let (w, gap) = door_post_span();
            (
                [
                    edge_part(0.0, WINDOW_SILL_M, between, 0.0, body),
                    edge_part(WINDOW_HEAD_M, LEVEL_H_M, between, 0.0, body),
                    edge_part(WINDOW_SILL_M, WINDOW_HEAD_M, w, -gap, body),
                    edge_part(WINDOW_SILL_M, WINDOW_HEAD_M, w, gap, body),
                    lo,
                    hi,
                ],
                6,
            )
        }
        SHAPE_FRAME => (
            // The top beam and the two corner posts, and nothing else: the
            // sim's rim is `FRAME_RIM_M` = 0.15 in from each end, which is
            // exactly the half of a corner post that lies inside the cell
            // (`collide::frame_solid_at`), so the posts ARE the jambs. The
            // opening is the piece.
            [
                edge_part(LEVEL_H_M - FRAME_RIM_M, LEVEL_H_M, between, 0.0, body),
                lo,
                hi,
                none,
                none,
                none,
            ],
            3,
        ),
        SHAPE_TRI_FOUNDATION | SHAPE_TRI_FLOOR | SHAPE_TRI_ROOF => (
            [
                // One NW prism; the other three halves are this part under
                // the root's quarter-turns (`base_transform`), the way the
                // north edge is the west edge turned.
                Part {
                    size: Vec3::new(BUILD_CELL_M, SLAB_T, BUILD_CELL_M),
                    offset: Vec3::new(0.0, -SLAB_T * 0.5, 0.0),
                    x_rot: 0.0,
                    kind: PartKind::Tri,
                    role: body,
                },
                none,
                none,
                none,
                none,
                none,
            ],
            1,
        ),
        // Foundation / floor / roof — and any shape the defs name that this
        // table does not: the slab whose TOP is the level plane, which is
        // the surface the sim stands players on, exactly the cell across.
        _ => (
            [
                Part {
                    size: Vec3::new(BUILD_CELL_M, SLAB_T, BUILD_CELL_M),
                    offset: Vec3::new(0.0, -SLAB_T * 0.5, 0.0),
                    x_rot: 0.0,
                    kind: PartKind::Box,
                    role: body,
                },
                none,
                none,
                none,
                none,
                none,
            ],
            1,
        ),
    }
}

/// The parts a DIAGONAL wall draws: one body the cell's own span — the root
/// scales it by √2 into the cell's diagonal (`base_transform`) — dropped a
/// further [`DIAG_DROP_M`] at its head, and no posts. Its own emit rather
/// than the straight wall's, because the straight body is trimmed to run
/// between two posts the diagonal does not have, and its head sits where a
/// straight wall's post would share a plane with it.
pub fn diagonal_parts() -> ([Part; MAX_PARTS], usize) {
    let lo = -EDGE_DROP_M;
    let hi = LEVEL_H_M - EDGE_DROP_M - DIAG_DROP_M;
    (
        [
            Part {
                size: Vec3::new(WALL_THICKNESS_M, hi - lo, BUILD_CELL_M),
                offset: Vec3::new(0.0, (hi + lo) * 0.5, 0.0),
                x_rot: 0.0,
                kind: PartKind::Box,
                role: PartRole::Body,
            },
            Part::default(),
            Part::default(),
            Part::default(),
            Part::default(),
            Part::default(),
        ],
        1,
    )
}

/// The parts a shape draws at a location: [`shape_parts`], except that a
/// wall on a diagonal is [`diagonal_parts`]. What the ghost and the kit both
/// read, so a diagonal previews as the object it becomes.
pub fn parts_for(shape: u8, loc: u8) -> ([Part; MAX_PARTS], usize) {
    if shape == SHAPE_WALL && (loc == LOC_DIAG_A || loc == LOC_DIAG_B) {
        diagonal_parts()
    } else {
        shape_parts(shape)
    }
}

/// Is `shape` a straight-edge piece, the kind that ends at two corners and
/// takes part in the post rule?
pub fn is_edge_shape(shape: u8) -> bool {
    matches!(
        shape,
        SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME
    )
}

/// Which of its two corner posts an edge piece draws (Rust's conditional
/// models, ported — `reference/BUILDING.md` §7d).
///
/// At a corner where up to four straight edge pieces meet, exactly ONE draws
/// the post — the first present in a fixed order — so two posts never
/// coincide (coincident posts z-fight) and no corner is bare. The order, for
/// the corner at cell `(px, pz)`'s low-x/low-z point:
///
/// 1. the low-x edge of `(px, pz)`, whose low end is this corner;
/// 2. the low-z edge of `(px, pz)`, likewise;
/// 3. the low-x edge of `(px, pz - 1)`, whose HIGH end is this corner;
/// 4. the low-z edge of `(px - 1, pz)`, likewise.
///
/// Read off the predictor's own collision index (`ClientCore::cols`) at
/// spawn time, and re-read on every reconcile — `stream` keeps the answer
/// beside the entity and redraws the piece when it changes, which is how a
/// wall's post moves to the wall that arrived beside it. Every edge shape
/// counts; a diagonal takes no post and blocks none.
pub fn post_owner(cols: &ColIndex, cx: u16, cz: u16, level: u8, loc: u8) -> PostOwn {
    // The corner at each end — as (px, pz), possibly off the grid's far
    // edge — and this piece's rank in the order above at that corner.
    type Corner = (Option<u16>, Option<u16>);
    let (low, high, low_rank, high_rank): (Corner, Corner, u8, u8) = match loc {
        LOC_EDGE_XLO => ((Some(cx), Some(cz)), (Some(cx), cz.checked_add(1)), 0, 2),
        LOC_EDGE_ZLO => ((Some(cx), Some(cz)), (cx.checked_add(1), Some(cz)), 1, 3),
        _ => return PostOwn::default(),
    };
    let owns = |corner: Corner, rank: u8| -> bool {
        // A corner off the grid's far edge belongs to nobody else.
        let (Some(px), Some(pz)) = corner else {
            return true;
        };
        (0..rank).all(|r| !match r {
            0 => edge_at(cols, Some(px), Some(pz), level, LOC_EDGE_XLO),
            1 => edge_at(cols, Some(px), Some(pz), level, LOC_EDGE_ZLO),
            2 => edge_at(cols, Some(px), pz.checked_sub(1), level, LOC_EDGE_XLO),
            _ => edge_at(cols, px.checked_sub(1), Some(pz), level, LOC_EDGE_ZLO),
        })
    };
    PostOwn {
        low: owns(low, low_rank),
        high: owns(high, high_rank),
    }
}

/// Which corner posts a standing edge piece draws — [`post_owner`]'s answer,
/// and `Live`'s redraw key for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PostOwn {
    pub low: bool,
    pub high: bool,
}

impl PostOwn {
    /// Both, for the ghost and for an edge piece with nothing beside it.
    pub const BOTH: Self = Self {
        low: true,
        high: true,
    };

    /// Two bits — the kit's index into its per-ownership meshes.
    pub fn bits(self) -> u8 {
        (self.low as u8) | ((self.high as u8) << 1)
    }

    pub fn from_bits(bits: u8) -> Self {
        Self {
            low: bits & 1 != 0,
            high: bits & 2 != 0,
        }
    }

    /// Does this ownership draw `role`?
    pub fn draws(self, role: PartRole) -> bool {
        match role {
            PartRole::Body => true,
            PartRole::PostLow => self.low,
            PartRole::PostHigh => self.high,
        }
    }
}

/// Does any straight edge piece stand on the named boundary at `level`, in
/// the client's mirror? Every edge shape — a doorway, a window and a frame
/// end at corners exactly as a wall does.
fn edge_at(cols: &ColIndex, cx: Option<u16>, cz: Option<u16>, level: u8, loc: u8) -> bool {
    let (Some(cx), Some(cz)) = (cx, cz) else {
        return false;
    };
    if (cx as usize) >= sim_core::limits::MAX_BUILD_COORD
        || (cz as usize) >= sim_core::limits::MAX_BUILD_COORD
    {
        return false;
    }
    let m = cols.get(cx, cz);
    let mask = if loc == LOC_EDGE_XLO {
        m.walls_xlo | m.doors_xlo | m.wins_xlo | m.frames_xlo
    } else {
        m.walls_zlo | m.doors_zlo | m.wins_zlo | m.frames_zlo
    };
    mask & (1u8 << level) != 0
}

/// How far below the lowest ground sample under its cell a foundation's
/// skirt sinks, metres — so a slope's dip between samples cannot open a
/// lit crack under the wall of earth. Cosmetic (`DECISIONS.md` §open,
/// "build base lattice v0").
pub const SKIRT_SINK_M: f32 = 0.35;
/// The skirt's depth ceiling, metres. On ground steep enough to beat it
/// the skirt hangs rather than growing without bound — the cap is wall
/// 4's habit applied to a mesh.
pub const SKIRT_MAX_M: f32 = 4.0;
/// The step the drawn skirt's depth is quantised to, metres.
///
/// **Why a continuous depth had to become a discrete one.** The skirt is the
/// one piece dimension that varies per ADDRESS, and it used to be drawn as a
/// shared unit cube stretched by the transform — which is exactly what a
/// metre-scaled UV cannot survive, because UVs do not scale with a transform
/// (a 4 m skirt would wear one tile of stone stretched over its whole
/// height). Quantising lets the kit cache one true-size mesh per depth and
/// pick, so a hundred foundations at a hundred depths are still
/// [`SKIRT_STEPS`] meshes and every one of them is correctly textured.
///
/// It rounds **up**, never down: a deeper skirt is still buried, where a
/// shallower one could lift its bottom out of the ground and re-open the
/// floating-plank screenshot the skirt exists to close. `tests/ghost.rs`
/// gates that directly — the bottom must sit under every ground sample.
pub const SKIRT_STEP_M: f32 = 0.25;
/// How many quantised depths span [`SLAB_T`]..=[`SKIRT_MAX_M`] at
/// [`SKIRT_STEP_M`]. Not derived in a const — `f32::ceil` is not const — so
/// `tests/pieces.rs` §C sweeps the range and fails if this is short.
pub const SKIRT_STEPS: usize = 16;

/// The bucket a raw skirt depth draws at: its index, and the depth itself.
///
/// **One implementation, two callers** — [`foundation_part`] takes the depth
/// and `build_kit`/`spawn_piece` take the index — for the reason every shared
/// emit in this file exists: a mesh picked by one rule and sized by another
/// is two rules, and they drift.
pub fn skirt_step(raw: f32) -> (usize, f32) {
    let over = (raw - SLAB_T).max(0.0);
    let steps = (over / SKIRT_STEP_M).ceil() as usize;
    let steps = steps.min(SKIRT_STEPS - 1);
    (
        steps,
        (SLAB_T + steps as f32 * SKIRT_STEP_M).min(SKIRT_MAX_M),
    )
}

/// The one drawn part of a foundation at its address: the slab whose top
/// is the level plane, exactly the cell across, extended DOWN as a skirt
/// until it is buried in the ground under every corner of its cell.
///
/// **Per-address where [`shape_parts`] is per-shape**, because the depth
/// depends on the terrain under the cell — the build lattice holds the
/// floor surface up to half a quantum off the ground and steps whole
/// quanta between plates, and a 0.3 m slab floating over that gap was
/// exactly the 2026-08-15 playtest's "pieces don't line up" screenshot:
/// foundations reading as levitating planks with terrain showing through
/// the steps. The skirt is how the reference grounds the same geometry.
///
/// **Exactly the cell across, and not a wall's thickness wider (gap v1).**
/// A wall is centred on its boundary, so its outer half hangs
/// `WALL_THICKNESS_M / 2` past this slab's edge with nothing under it — an
/// undercut the footing's whole height, under every outside wall. The first
/// fix widened the skirt out to the wall's face; `tests/gaps.rs` refused it
/// the day it was written, because two neighbours' widened skirts then
/// overlapped with their tops on one plane at every corner of the base
/// where nothing hid them. What closes the undercut instead is the
/// [`apron_parts`] each ground-storey edge piece hangs from its own foot:
/// the wall's plinth belongs to the wall, owned like its posts, so a
/// foundation with no wall on a side has no rim there to fight over.
///
/// **The one emit site for both the standing piece and the build ghost**
/// (`spawn_piece` and `ghost::track`), the same rule the parts table
/// carries for every other shape: a preview with its own idea of the
/// skirt would promise a different object than the click delivers.
/// `tests/ghost.rs` holds the top at the level plane and the bottom in
/// the ground.
///
/// Drawn, not collided: the sim's plane is still a walkable surface at
/// its base and the volume below it blocks nothing (`NOW.md` carries the
/// residual). The top face is unmoved, so nothing about where a player
/// stands changed with the skirt's depth.
pub fn foundation_part(
    seed: u64,
    haven: &terrain::Haven,
    cx: u16,
    cz: u16,
    tri: bool,
    plate: i8,
) -> Part {
    let (_, depth) = footing_of(seed, haven, cx, cz, plate);
    Part {
        size: Vec3::new(BUILD_CELL_M, depth, BUILD_CELL_M),
        offset: Vec3::new(0.0, -depth * 0.5, 0.0),
        x_rot: 0.0,
        kind: if tri { PartKind::Tri } else { PartKind::Box },
        role: PartRole::Body,
    }
}

/// How far below its foot a ground-storey edge piece's apron reaches, metres:
/// the skirt's own ceiling, so wherever the foundation's skirt stops the apron
/// stops with it — buried under every footing the cap has not cut, and cut
/// exactly where the skirt is on ground steep enough to cut it.
pub const APRON_DEPTH_M: f32 = SKIRT_MAX_M;

/// The apron a ground-storey edge piece hangs from its foot (gap v1): the
/// wall's own plinth, a wall's thickness across and [`APRON_DEPTH_M`] deep
/// from [`EDGE_DROP_M`] under the storey base, running between its corner
/// posts — plus, under each post the piece owns, the post's own continuation
/// down.
///
/// **Why the wall carries it and not the foundation.** The undercut is under
/// the wall's OUTER half, which hangs past the slab's edge
/// ([`foundation_part`] says what widening the skirt cost). A plinth that is
/// part of the wall closes exactly that half — the inner half is inside the
/// skirt and never seen — and has the wall's own answer to every join: it
/// runs between the same posts, its ends abut the same post the wall's ends
/// do, and a corner with no wall has no apron to fight over. Two in-line
/// walls' aprons abut where their bodies do; an interior wall's apron is
/// inside two skirts and draws nothing anyone sees.
///
/// A child of the piece's entity, at the root's own frame, on the ground
/// storey only: a storey up, the foot is inside the slab below and there is
/// no ground to reach.
pub fn apron_parts(own: PostOwn) -> ([Part; 3], usize) {
    let h = APRON_DEPTH_M - EDGE_DROP_M;
    let c = -(APRON_DEPTH_M + EDGE_DROP_M) * 0.5;
    let strip = Part {
        size: Vec3::new(WALL_THICKNESS_M, h, BUILD_CELL_M - post_w()),
        offset: Vec3::new(0.0, c, 0.0),
        x_rot: 0.0,
        kind: PartKind::Box,
        role: PartRole::Body,
    };
    let post = |z: f32, role: PartRole| Part {
        size: Vec3::new(post_w(), h, post_w()),
        offset: Vec3::new(0.0, c, z),
        x_rot: 0.0,
        kind: PartKind::Box,
        role,
    };
    let mut out = [strip, Part::default(), Part::default()];
    let mut n = 1;
    if own.low {
        out[n] = post(-BUILD_CELL_M * 0.5, PartRole::PostLow);
        n += 1;
    }
    if own.high {
        out[n] = post(BUILD_CELL_M * 0.5, PartRole::PostHigh);
        n += 1;
    }
    (out, n)
}

/// A cell's footing: which cached mesh draws it, and how deep that mesh is.
///
/// **Both callers ask this, never each other** — [`foundation_part`] wants
/// the depth and `spawn_piece` wants the index — because recovering the index
/// by dividing the depth back out would be a second quantisation, and two
/// float quantisations of one value disagree at the bucket boundary by
/// exactly one bucket. That is a foundation drawn 0.25 m too deep with every
/// gate green, which is this file's oldest failure shape wearing a new hat.
///
/// **`base` and `lo` are both CARVED, and they must move together.** `depth`
/// is their difference, so one of them reading raw terrain while the other
/// reads the carve is a skirt sized from a subtraction across two different
/// surfaces — a foundation that floats or buries itself on an authored site.
/// That is why this function grew a `haven`: the split above arrived on main
/// in the same window the carve landed here, and each half is correct alone.
pub fn footing_of(seed: u64, haven: &terrain::Haven, cx: u16, cz: u16, plate: i8) -> (usize, f32) {
    let base = sim_core::build::column_floor_y(seed, haven, cx, cz, plate);
    let x0 = cx as f32 * BUILD_CELL_M;
    let z0 = cz as f32 * BUILD_CELL_M;
    // The lowest ground under the cell: four corners and the centre. Five
    // samples bound a 3 m cell's dip well enough for a drawn skirt; the
    // sink margin below covers what they miss.
    let mut lo = f32::MAX;
    for (px, pz) in [
        (x0, z0),
        (x0 + BUILD_CELL_M, z0),
        (x0, z0 + BUILD_CELL_M),
        (x0 + BUILD_CELL_M, z0 + BUILD_CELL_M),
        (x0 + BUILD_CELL_M * 0.5, z0 + BUILD_CELL_M * 0.5),
    ] {
        lo = lo.min(terrain::ground(seed, haven, px, pz));
    }
    skirt_step((base - lo + SKIRT_SINK_M).clamp(SLAB_T, SKIRT_MAX_M))
}

/// The mesh a [`Part`] fills its extents with: the cuboid, or the NW
/// half-cell prism, **built at its true size in metres**. One function for
/// the kit's cache and the ghost's rebuild, so the preview and the piece
/// cannot disagree about what a triangle looks like.
///
/// Both builders emit the four things a piece needs and none of which it had
/// before 2026-08-16, each of which fails SILENTLY when absent:
///
/// 1. **Metre-scaled UVs** ([`PIECE_UV_PER_M`]) instead of 0..1 per face —
///    a stretched tile reads as no texture, not as an error.
/// 2. **Per-face vertex tint** ([`FACE_TOP`] and friends), the mean-1 field
///    that separates a slab's top from its sides.
/// 3. **Tangents**, without which Bevy ignores `normal_map_texture` outright
///    and the surface stays flat — `props.rs` calls this "the failure that
///    looks like 'the texture did not load' and reports nothing at all".
/// 4. **Flat normals on unshared vertices**, which the prism already had and
///    `Cuboid` also gives; a piece is faceted by construction.
///
/// The size is baked into the vertices rather than left to a transform scale
/// for reason 1: UVs do not scale with a transform, so a shared unit mesh
/// cannot carry a correct metre UV for two different sizes. That is also why
/// a foundation's skirt depth is quantised ([`skirt_step`]) — a per-address
/// depth would otherwise mean a per-address mesh.
pub fn part_mesh(part: &Part) -> Mesh {
    match part.kind {
        PartKind::Box => box_mesh(part.size),
        PartKind::Tri => tri_prism_mesh(part.size),
    }
}

/// Several parts as ONE mesh, each at its own offset — what a standing
/// multi-part piece is drawn from (gap v1).
///
/// The hammer highlight reads `Transform` + `Mesh3d` off the one entity
/// `StructRing::entity_at` answers (`highlight.rs`), which is why a piece has
/// always been one entity where it could be. A doorway was three children
/// under a bare root and the highlight could not wash it; the corner posts
/// would have made a wall the same. Baking the parts into one mesh keeps
/// every piece one entity with its mesh on it, and costs one draw instead of
/// six. The offsets are baked and the pitch is not: only the stairs carry
/// one, and the stairs are one part.
pub fn parts_mesh(parts: &[Part]) -> Mesh {
    let mut b = buffers(24 * parts.len());
    for part in parts {
        debug_assert!(part.x_rot == 0.0, "a merged part cannot carry a pitch");
        match part.kind {
            PartKind::Box => box_into(&mut b, part.size, part.offset),
            PartKind::Tri => tri_prism_into(&mut b, part.size, part.offset),
        }
    }
    finish(b)
}

/// A quad's worth of vertices for one face, appended to the buffers.
///
/// `corners` runs the rim of the face in order, and the caller is responsible
/// for handing them over counter-clockwise seen from OUTSIDE — which the two
/// builders below get for free by walking a `(u, v)` pair chosen so
/// `u × v = n`, rather than by a per-face special case.
///
/// The UV runs corner 0→1 then 0→3, in metres, so two faces of different
/// sizes wear the same texture density.
fn quad(b: &mut Buffers, corners: [Vec3; 4], n: Vec3, tint: impl Fn(Vec3) -> f32) {
    let (pos, nor, col, uv, idx) = b;
    let base = pos.len() as u32;
    // Edge lengths in metres, straight off the corners, so a face whose
    // extent is not axis-aligned (the prism's hypotenuse) is still measured
    // rather than assumed.
    let du = (corners[1] - corners[0]).length() * PIECE_UV_PER_M;
    let dv = (corners[3] - corners[0]).length() * PIECE_UV_PER_M;
    for (i, c) in corners.iter().enumerate() {
        pos.push([c.x, c.y, c.z]);
        nor.push([n.x, n.y, n.z]);
        let t = tint(*c);
        col.push([t, t, t, 1.0]);
        uv.push(match i {
            0 => [0.0, 0.0],
            1 => [du, 0.0],
            2 => [du, dv],
            _ => [0.0, dv],
        });
    }
    idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// The mean-1 albedo multiplier a vertex at `y` on a face with normal `n`
/// carries: flat per face, plus the foot-to-head ramp on the uprights.
///
/// `y0`/`y1` are the part's own vertical extent, so the ramp spans the part
/// rather than the world — a 3 m wall and a 0.3 m floor edge each get the
/// full range over their own height.
fn face_tint(n: Vec3, y: f32, y0: f32, y1: f32) -> f32 {
    if n.y > 0.5 {
        return FACE_TOP;
    }
    if n.y < -0.5 {
        return FACE_BOTTOM;
    }
    let span = y1 - y0;
    let t = if span > 1e-6 {
        ((y - y0) / span).clamp(0.0, 1.0)
    } else {
        0.5
    };
    FACE_SIDE * (SIDE_FOOT + (SIDE_HEAD - SIDE_FOOT) * t)
}

/// Positions, normals, colours, UVs, indices — the buffers every piece mesh
/// is assembled into, finished by [`finish`].
type Buffers = (
    Vec<[f32; 3]>,
    Vec<[f32; 3]>,
    Vec<[f32; 4]>,
    Vec<[f32; 2]>,
    Vec<u32>,
);

fn buffers(n: usize) -> Buffers {
    (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    )
}

/// Assemble the buffers into a mesh and generate its tangents.
///
/// The tangent generation is `expect`ed rather than handled: mikktspace
/// refuses a degenerate triangle, and a piece part with a zero extent is a
/// malformed table row, not a missing capability. `props.rs` takes the same
/// position for the same reason — fail at boot, never ship a surface whose
/// relief silently vanished.
fn finish((pos, nor, col, uv, idx): Buffers) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m.generate_tangents()
        .expect("a piece part has positions, normals and UVs");
    m
}

/// A box of `size`, centred on its own origin.
fn box_mesh(size: Vec3) -> Mesh {
    let mut b = buffers(24);
    box_into(&mut b, size, Vec3::ZERO);
    finish(b)
}

/// A box of `size` centred at `at`, appended to the buffers. The face tint's
/// vertical ramp runs over the box's own height, not the world's, so a part
/// baked at an offset wears the same ramp it would standing alone.
fn box_into(b: &mut Buffers, size: Vec3, at: Vec3) {
    let h = size * 0.5;
    // (normal, u, v) with u × v = normal — see [`quad`].
    //
    // **v is UP on all four uprights, and it was not until 2026-08-22.** The
    // old +x row was `(u = Y, v = Z)` and the old −z row `(u = Y, v = X)`, so
    // those two faces carried the map turned a quarter-turn against the other
    // two. Both rows satisfied `u × v = n` — the only thing stated here, and
    // the only thing anything checked — so nothing was wrong except the
    // picture: on a box wearing a DIRECTIONAL map, half a base's walls ran
    // their grain vertically and half ran it horizontally. Invisible while
    // every tier wore tree bark, which is directional but reads as noise;
    // immediately visible on planks or lashed poles, which is what the tiers
    // wear now. Gated by `tests/pieces.rs` §B.
    //
    // Solved rather than tried: fixing v = Y leaves u = ±(Y × n), which is
    // −Z, +Z, +X, −X for +x, −x, +z, −z. The caps keep their own frame; a
    // horizontal surface has no up to agree about.
    for (n, u, v) in [
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ] {
        let c = at + n * h;
        let (hu, hv) = ((u * h).length(), (v * h).length());
        quad(
            b,
            [
                c - u * hu - v * hv,
                c + u * hu - v * hv,
                c + u * hu + v * hv,
                c - u * hu + v * hv,
            ],
            n,
            |p| face_tint(n, p.y - at.y, -h.y, h.y),
        );
    }
}

/// The NW half-cell prism at true size: the triangle {u + w ≤ 0} over
/// x, z ∈ [−½, ½] × `size`, extruded over y — corners at (−½,−½), (½,−½),
/// (−½,½), hypotenuse facing +x+z.
fn tri_prism_mesh(size: Vec3) -> Mesh {
    let mut buf = buffers(21);
    tri_prism_into(&mut buf, size, Vec3::ZERO);
    finish(buf)
}

/// The prism centred at `at`, appended to the buffers — [`box_into`]'s
/// shape for the other filling.
fn tri_prism_into(buf: &mut Buffers, size: Vec3, at: Vec3) {
    let h = size * 0.5;
    // The three cross-section corners, CCW seen from +y (bevy's up).
    let a = Vec3::new(-h.x, 0.0, -h.z); // right angle (NW corner)
    let b = Vec3::new(h.x, 0.0, -h.z); // N-E corner
    let c = Vec3::new(-h.x, 0.0, h.z); // S-W corner
                                       // The two triangular caps. They are emitted by hand rather than through
                                       // `quad` — a triangle is not a quad, and its UV is the cross-section's
                                       // own x/z in metres so the cap's grain matches the sides'.
    for (ny, y, wind) in [(1.0f32, h.y, false), (-1.0, -h.y, true)] {
        let (pos, nor, col, uv, idx) = &mut *buf;
        let base = pos.len() as u32;
        let n = Vec3::new(0.0, ny, 0.0);
        let t = face_tint(n, y, -h.y, h.y);
        for p in [a, b, c] {
            pos.push([p.x + at.x, y + at.y, p.z + at.z]);
            nor.push([0.0, ny, 0.0]);
            col.push([t, t, t, 1.0]);
            uv.push([(p.x + h.x) * PIECE_UV_PER_M, (p.z + h.z) * PIECE_UV_PER_M]);
        }
        if wind {
            idx.extend([base, base + 1, base + 2]);
        } else {
            idx.extend([base, base + 2, base + 1]);
        }
    }
    // The two square sides and the hypotenuse. Outward normals: the N side
    // faces −z, the W side −x, the hypotenuse +x+z. The hypotenuse's normal
    // is derived from the corners rather than the 45° constant the unit
    // prism could assume — a non-square cell would tilt it.
    for (p0, p1) in [(a, b), (c, a), (b, c)] {
        let e = p1 - p0;
        let n = Vec3::new(e.z, 0.0, -e.x).normalize();
        // Corner 0 → 3 is the quad's v axis, and it runs UP — the same
        // invariant `box_mesh`'s frame table states, held here by starting at
        // the far foot rather than the near head. The rim used to run
        // head-to-foot, which is v pointing DOWN: legal, correctly wound, and
        // a quarter-turn's worth of the same disagreement `box_mesh` carried
        // between its own faces — a prism roof drawing its poles upside down
        // against the wall it meets. Starting at `p1` rather than `p0` is what
        // keeps `u × v = n` with v flipped; see the winding note in [`quad`].
        quad(
            buf,
            [
                at + p1 - Vec3::Y * h.y,
                at + p0 - Vec3::Y * h.y,
                at + p0 + Vec3::Y * h.y,
                at + p1 + Vec3::Y * h.y,
            ],
            n,
            |p| face_tint(n, p.y - at.y, -h.y, h.y),
        );
    }
}

/// The world transform of an address's base point: the canonical anchor
/// [`shape_parts`]' offsets are relative to, plus the quarter-turn a low-z
/// edge carries. Shared with the build ghost for the reason the parts are:
/// the ghost and the piece it becomes must be the same object in the same
/// pose, and edge canonicalisation written twice is how they stop being.
pub fn base_transform(
    seed: u64,
    haven: &terrain::Haven,
    (cx, cz, level, loc): Addr,
    plate: i8,
) -> Transform {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2};
    let base_y = level_base_y(seed, haven, cx, cz, level, plate);
    let (cxm, czm) = cell_center(cx, cz);
    // Triangles and diagonals are one drawn object each under a turn of
    // the root (triangles v0): the four halves are the NW prism at 0,
    // ∓90° and 180°, and the two diagonal walls are the straight wall
    // slab turned ±45° and stretched √2 along its length. The scale is
    // the one non-rigid root this function hands out, and it is exactly
    // the diagonal's own arithmetic — the slab thickness axis stays 1.
    let (pos, yaw, scale) = match loc {
        LOC_EDGE_XLO => (
            Vec3::new(cx as f32 * BUILD_CELL_M, base_y, czm),
            0.0,
            Vec3::ONE,
        ),
        LOC_EDGE_ZLO => (
            Vec3::new(cxm, base_y, cz as f32 * BUILD_CELL_M),
            FRAC_PI_2,
            Vec3::ONE,
        ),
        LOC_TRI_XLO_ZLO => (Vec3::new(cxm, base_y, czm), 0.0, Vec3::ONE),
        LOC_TRI_XHI_ZLO => (Vec3::new(cxm, base_y, czm), -FRAC_PI_2, Vec3::ONE),
        LOC_TRI_XLO_ZHI => (Vec3::new(cxm, base_y, czm), FRAC_PI_2, Vec3::ONE),
        LOC_TRI_XHI_ZHI => (Vec3::new(cxm, base_y, czm), PI, Vec3::ONE),
        LOC_DIAG_A => (
            Vec3::new(cxm, base_y, czm),
            FRAC_PI_4,
            Vec3::new(1.0, 1.0, SQRT_2),
        ),
        LOC_DIAG_B => (
            Vec3::new(cxm, base_y, czm),
            -FRAC_PI_4,
            Vec3::new(1.0, 1.0, SQRT_2),
        ),
        _ => (Vec3::new(cxm, base_y, czm), 0.0, Vec3::ONE),
    };
    Transform::from_translation(pos)
        .with_rotation(Quat::from_rotation_y(yaw))
        .with_scale(scale)
}

/// The generated mesh for an archetype, or `None` where the greybox cuboid is
/// still what draws. Index-aligned with [`DEPLOY`], which is what keeps a new
/// archetype from silently inheriting its neighbour's model.
///
/// **A row here overrides both the mesh AND the material**, because a `.glb`
/// carries its own PBR set and the flat colour beside it in `DEPLOY` exists
/// only to make a cuboid legible. The colour columns stay on the covered rows
/// rather than being deleted: the build ghost still reads `deploy_size`, and a
/// row whose model fails to load falls back to a shape rather than to nothing.
///
/// Generated 2026-08-11 (`DECISIONS.md`, and `assets/models/MANIFEST.md` for
/// the per-asset prompt and task id). Sized by `ci/import_meshy.py` against
/// this file's own [`DEPLOY`] row, never by the generator's estimate — the
/// reasons are in that script's header and the measurements in `DECISIONS.md`.
pub const DEPLOY_ASSET: [Option<&str>; DEPLOY.len()] = [
    Some("models/deploy/bag.glb"),       // 0 bag
    Some("models/deploy/hearth.glb"),    // 1 hearth
    Some("models/deploy/box.glb"),       // 2 box
    Some("models/deploy/fire.glb"),      // 3 fire
    None,                                // 4 furnace — greybox
    Some("models/deploy/workbench.glb"), // 5 workbench
    None,                                // 6 door — a door is a slab, and the
    // one archetype whose material is state (`door_locked`), so a baked PBR
    // set would have to be swapped rather than tinted.
    None, // 7 lock — never drawn
    None, // 8 recycler — greybox
    None, // 9 research table — greybox
    None, // 10 workbench 2 — greybox
    None, // 11 workbench 3 — greybox
];

/// Build the pool. `pub` for the same reason [`Kit`] is.
pub fn build_kit(
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Kit {
    // The photographs. `base_color` is the scalar gain in grey rather than a
    // hue — the map ships its own colour, and a coloured multiply here would
    // be the per-channel gain `ART.md` §7 bounds (see [`TIER`]).
    let tier = std::array::from_fn(|i| {
        let t = TIER[i];
        let map = MapSet::load(assets, t.role);
        // The tile density, as a scale on every UV this material samples with
        // — base colour AND normal AND roughness, which is what keeps the
        // relief registered with the colour. Uniform, so the tangent frame
        // `finish` generated is still the right one; a non-uniform scale here
        // would shear it and the normal map would light wrong.
        let uv_transform = Affine2::from_scale(Vec2::splat(t.tiles_per_m));
        // The maps are loaded ONCE per tier and cloned into all eight bands
        // — `MapSet::load` inside the band loop would be eight identical
        // paths and eight identical settings, which the asset server would
        // dedupe anyway, but relying on that to avoid eight loads is
        // relying on a cache for correctness of cost.
        std::array::from_fn(|band| {
            let hurt = damage_mix(band as u8);
            let g = t.gain * (1.0 + (DMG_DARKEST - 1.0) * hurt);
            materials.add(StandardMaterial {
                base_color: Color::linear_rgb(g, g, g),
                base_color_texture: Some(map.albedo.clone()),
                normal_map_texture: Some(map.normal.clone()),
                perceptual_roughness: (t.roughness + DMG_ROUGHEN * hurt).min(1.0),
                metallic: t.metallic,
                uv_transform,
                ..default()
            })
        })
    });
    // A generated asset is one mesh with one primitive and one material —
    // asserted by `tests/deploy_assets.rs`, because `Primitive { mesh: 0,
    // primitive: 0 }` would silently draw a fraction of a multi-part model.
    // Loading the primitive rather than the scene is deliberate: it lands in
    // the same `Handle<Mesh>` the cuboid used, so `spawn_deploy` is unchanged.
    // The cost is that a node transform would be dropped — which is exactly
    // why `ci/import_meshy.py` bakes scale into the vertices.
    let deploy_mesh = std::array::from_fn(|i| match DEPLOY_ASSET[i] {
        Some(path) => assets.load(
            GltfAssetLabel::Primitive {
                mesh: 0,
                primitive: 0,
            }
            .from_asset(path),
        ),
        None => {
            let [w, h, d] = DEPLOY[i].0;
            meshes.add(Cuboid::new(w, h, d))
        }
    });
    let deploy_mat = std::array::from_fn(|i| match DEPLOY_ASSET[i] {
        Some(path) => assets.load(
            GltfAssetLabel::Material {
                index: 0,
                is_scale_inverted: false,
            }
            .from_asset(path),
        ),
        None => {
            let (_, base_color, perceptual_roughness, metallic) = DEPLOY[i];
            materials.add(StandardMaterial {
                base_color,
                perceptual_roughness,
                metallic,
                ..default()
            })
        }
    });
    // The piece meshes, from the shared table and nowhere else. Dedup is by
    // exact (kind, size) — the sizes that repeat are the same expressions
    // evaluated twice, so `==` is the right comparison and a near-miss
    // SHOULD build a second mesh, because a near-miss is two parts claiming
    // to differ.
    const NO_MESH: Option<Handle<Mesh>> = None;
    // The outer repeat can't be `[[NO_MESH; MAX_PARTS]; N_SHAPES]`: only the
    // inner repeat's operand is a const item — the outer would Copy a built
    // non-Copy row (E0277). `from_fn` evaluates the const repeat per row.
    let mut shape_mesh: [[Option<Handle<Mesh>>; MAX_PARTS]; N_SHAPES] =
        std::array::from_fn(|_| [NO_MESH; MAX_PARTS]);
    let mut sized: Vec<(PartKind, Vec3, Handle<Mesh>)> = Vec::new();
    for (shape, row) in shape_mesh.iter_mut().enumerate() {
        let (parts, n) = shape_parts(shape as u8);
        for (slot, part) in row.iter_mut().zip(&parts[..n]) {
            let handle = match sized
                .iter()
                .find(|(kind, size, _)| *kind == part.kind && *size == part.size)
            {
                Some((_, _, h)) => h.clone(),
                None => {
                    let h = meshes.add(part_mesh(part));
                    sized.push((part.kind, part.size, h.clone()));
                    h
                }
            };
            *slot = Some(handle);
        }
    }
    // The straight edge shapes, merged per ownership: the body and whichever
    // of the two corner posts the piece draws (`post_owner`).
    let mut edge_mesh: [[Option<Handle<Mesh>>; 4]; N_SHAPES] =
        std::array::from_fn(|_| [NO_MESH; 4]);
    for shape in [SHAPE_WALL, SHAPE_DOORWAY, SHAPE_WINDOW, SHAPE_FRAME] {
        let (parts, n) = shape_parts(shape);
        for own in 0..4u8 {
            let keep = PostOwn::from_bits(own);
            let owned: Vec<Part> = parts[..n]
                .iter()
                .copied()
                .filter(|p| keep.draws(p.role))
                .collect();
            edge_mesh[shape as usize][own as usize] = Some(meshes.add(parts_mesh(&owned)));
        }
    }
    let diag_mesh = meshes.add(part_mesh(&diagonal_parts().0[0]));
    let apron = std::array::from_fn(|own| {
        let (parts, n) = apron_parts(PostOwn::from_bits(own as u8));
        meshes.add(parts_mesh(&parts[..n]))
    });
    // One footing mesh per quantised depth, per kind — built here rather
    // than on demand so a player walking onto new ground never pays a mesh
    // upload mid-frame (`RENDER.md` §2's prewarm discipline, applied to
    // geometry).
    let footing = std::array::from_fn(|i| {
        let depth = (SLAB_T + i as f32 * SKIRT_STEP_M).min(SKIRT_MAX_M);
        let size = Vec3::new(BUILD_CELL_M, depth, BUILD_CELL_M);
        [meshes.add(box_mesh(size)), meshes.add(tri_prism_mesh(size))]
    });
    Kit {
        shape_mesh,
        edge_mesh,
        diag_mesh,
        footing,
        apron,
        tier,
        deploy_mesh,
        deploy_mat,
        door_locked: materials.add(StandardMaterial {
            base_color: DOOR_LOCKED,
            perceptual_roughness: 0.45,
            metallic: 0.8,
            ..default()
        }),
        bag_mesh: meshes.add(Cuboid::new(BAG_SIZE[0], BAG_SIZE[1], BAG_SIZE[2])),
        bag_mat: materials.add(StandardMaterial {
            base_color: BAG_COLOR,
            perceptual_roughness: 0.95,
            ..default()
        }),
    }
}

/// Every `Feed::applied` bit that can change what this system draws.
///
/// The reconcile-the-whole-mirror shape above is right and stays — it is what
/// makes an upgrade, a raid band and a swap-remove all one code path. What was
/// wrong is that it ran on every frame including the ~99% where the wire said
/// nothing about a piece, a deployable or a bag: 8,192 pieces at the cap is
/// 117 µs of hashing and comparing to conclude that nothing moved.
///
/// `feed::drain` runs `.after(input::place_eye).before(Stream)`, so this word
/// is THIS frame's news — the pump raised it and the drain took word and rings
/// in one move.
///
/// A bit missing from this list is a base that does not redraw, and it
/// produces no error — so the list is not hand-kept: `client/tests/
/// frame_gates.rs` reads every `APPLIED*` constant `client_core` publishes,
/// takes the ones whose NAME says piece, deploy, bag or struct, and fails on
/// any that is not here. It also fails on a mirror-shaped flag landing in
/// `client_core`'s SECOND applied word, which this condition does not read —
/// writing that check is what turned up `APPLIED2_BAGS` and sent someone to
/// find out whether it was the world's bag store (it is the player's own list,
/// for the death screen).
pub const STRUCT_APPLIED: u32 = client_core::core::APPLIED_PIECES
    | client_core::core::APPLIED_PIECE_RESET
    | client_core::core::APPLIED_PIECE_REMOVED
    | client_core::core::APPLIED_PIECE_DEFS
    | client_core::core::APPLIED_STRUCT_HIT
    | client_core::core::APPLIED_DEPLOYS
    | client_core::core::APPLIED_DEPLOY_RESET
    | client_core::core::APPLIED_DEPLOY_REMOVED
    | client_core::core::APPLIED_DEPLOY_DEFS
    | client_core::core::APPLIED_BAGS;

/// Run condition for [`stream`]: the wire said something about what it draws,
/// or the ring has not been built yet.
///
/// The second half is not optional. The first frames of a world arrive with
/// the kit unbuilt and the mirrors already seeded by the join drip, and a
/// condition that only watched the news would leave the base a player logged
/// out inside undrawn until someone hit it.
pub fn structures_changed(ring: Res<StructRing>, feed: Res<super::feed::Feed>) -> bool {
    ring.kit.is_none() || feed.applied & STRUCT_APPLIED != 0
}

/// Reconcile all three stores against the core's mirrors.
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<StructRing>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
) {
    // One reborrow, then field-level borrows. `ResMut`'s `DerefMut` hands out
    // a borrow of the WHOLE resource, so reading `kit` while inserting into
    // `pieces` is a conflict until the struct is split like this.
    let ring = &mut *ring;
    if ring.kit.is_none() {
        ring.kit = Some(build_kit(&assets, &mut meshes, &mut materials));
    }
    let kit = ring.kit.as_ref().expect("built above");
    // The island's solved sites, for the carved ground every piece is footed
    // on (`terrain::ground`). One binding, so every emit in this system is
    // placed against the same surface.
    let haven = &world.haven;
    let core = &net.session.core;
    ring.gen = ring.gen.wrapping_add(1);
    let gen = ring.gen;
    let seed = world.seed;

    // ---- pieces ---------------------------------------------------------
    // A row past `piece_defs_have` has not dripped in yet: its shape and
    // material are unknown, and `PieceDef::INERT` would draw it as a wooden
    // foundation. Skip it — the frame after the defs arrive draws it right,
    // and a wrong wall is worse than a late one.
    let have = core.piece_defs_have.min(core.piece_defs.piece_count);
    for rec in core.pieces.entries() {
        if (rec.row as u16) >= have {
            continue;
        }
        let key = (rec.cx, rec.cz, rec.level, rec.loc);
        let def = core.piece_defs.pieces[rec.row as usize];
        // Which corner posts this piece draws is a fact about its NEIGHBOURS
        // (gap v1), so it is re-read here every pass rather than latched at
        // spawn: the wall that arrived beside this one takes the post they
        // share, and this one must be redrawn without it.
        let own = if is_edge_shape(def.shape) {
            post_owner(core.pieces.cols(), rec.cx, rec.cz, rec.level, rec.loc).bits()
        } else {
            0
        };
        if let Some(live) = ring.pieces.get_mut(&key) {
            live.seen = gen;
            // An upgrade keeps the address and changes the row; a raid keeps
            // both and changes the band; a neighbour keeps everything and
            // changes the posts. All are redraws, and the band is the one
            // that moves while the player is standing there.
            if live.row == rec.row
                && live.dmg == rec.dmg
                && live.plate == rec.plate
                && live.own == own
            {
                continue;
            }
            commands.entity(live.entity).despawn();
            ring.pieces.remove(&key);
        }
        let entity = spawn_piece(
            &mut commands,
            kit,
            seed,
            haven,
            key,
            def.shape,
            def.material,
            rec.dmg,
            rec.plate,
            PostOwn::from_bits(own),
        );
        ring.pieces.insert(
            key,
            Live {
                entity,
                seen: gen,
                row: rec.row,
                open: false,
                locked: false,
                dmg: rec.dmg,
                own,
                plate: rec.plate,
            },
        );
    }
    ring.pieces.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });

    // ---- deployables ----------------------------------------------------
    let dhave = core.deploy_defs_have.min(core.deploy_defs.def_count);
    for rec in core.deploys.entries() {
        if (rec.row as u16) >= dhave {
            continue;
        }
        let key = (rec.cx, rec.cz, rec.level, rec.loc);
        // The column's plate, from the piece mirror the predictor already
        // keeps — a deployable's own record does not carry one
        // (`protocol::event::write_deploy_rec` says why). An unbuilt column
        // is a ground placement and answers 0, the terrain rule.
        let plate = core.pieces.cols().plate(rec.cx, rec.cz).unwrap_or(0);
        if let Some(live) = ring.deploys.get_mut(&key) {
            live.seen = gen;
            // A door swing and a lock are both redraws at one address.
            //
            // **The damage band is deliberately NOT compared here.** A
            // deployable's material is the one baked into its `.glb`
            // (`DEPLOY_ASSET`), so there is no band variant to swap to and
            // comparing would despawn and respawn a furnace on every swing
            // for an identical picture. `Target::damaged` is correct for
            // this store either way — that is the wire's doing, not the
            // renderer's. When deployables get a damage response, this line
            // and `Live::dmg` below change together.
            if live.row == rec.row
                && live.open == rec.open
                && live.locked == rec.locked
                && live.plate == plate
            {
                continue;
            }
            commands.entity(live.entity).despawn();
            ring.deploys.remove(&key);
        }
        let arch = core.deploy_defs.defs[rec.row as usize].arch;
        let entity = spawn_deploy(&mut commands, kit, seed, haven, rec, arch, plate);
        ring.deploys.insert(
            key,
            Live {
                entity,
                seen: gen,
                row: rec.row,
                open: rec.open,
                locked: rec.locked,
                dmg: 0,
                own: 0,
                plate,
            },
        );
    }
    ring.deploys.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });

    // ---- backpacks ------------------------------------------------------
    // A bag never moves, so a known id is left entirely alone.
    for bag in core.bags.entries() {
        if let Some(live) = ring.bags.get_mut(&bag.id) {
            live.seen = gen;
            continue;
        }
        // The sim drops it at the body's FEET, so half its height lifts it
        // onto the ground rather than leaving it sunk to the waist.
        let pos = Vec3::new(
            bag.qx as f32 * POS_XZ_Q,
            bag.qy as f32 * POS_Y_Q + BAG_SIZE[1] * 0.5,
            bag.qz as f32 * POS_XZ_Q,
        );
        let entity = commands
            .spawn((
                super::WorldEntity,
                Mesh3d(kit.bag_mesh.clone()),
                MeshMaterial3d(kit.bag_mat.clone()),
                Transform::from_translation(pos),
            ))
            .id();
        ring.bags.insert(
            bag.id,
            Live {
                entity,
                seen: gen,
                row: 0,
                open: false,
                locked: false,
                dmg: 0,
                own: 0,
                // A bag carries a quantized world position, so it has no
                // column and no plate to be drawn against.
                plate: 0,
            },
        );
    }
    ring.bags.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_piece(
    commands: &mut Commands,
    kit: &Kit,
    seed: u64,
    haven: &terrain::Haven,
    addr: Addr,
    shape: u8,
    material: u8,
    dmg: u8,
    plate: i8,
    own: PostOwn,
) -> Entity {
    // Clamped, not `.min(2)`: the table covers every material the sim has
    // (`N_TIERS`, gated in `tests/pieces.rs` §A), so this only catches a
    // material the sim gained without a row here — which is a red test long
    // before it is a wrong wall.
    let mat = kit.tier[(material as usize).min(N_TIERS - 1)]
        [(dmg as usize).min(DMG_BANDS as usize - 1)]
    .clone();
    // Edge pieces stand on the cell's low-x (x = cx·3) or low-z (z = cz·3)
    // boundary — canonical, so one physical edge is never addressable twice
    // (`build.rs`) — and the parts are the shared table's, so this and the
    // build ghost are the same object in the same pose.
    let root = base_transform(seed, haven, addr, plate);
    let (cx, cz, _, loc) = addr;

    // **Every piece is ONE entity with its mesh on it.** Not a style choice:
    // the hammer highlight reads `Transform` + `Mesh3d` off the entity
    // `entity_at` answers (`highlight.rs`), and a bare root over children
    // would hide both from it. A multi-part shape is therefore baked into one
    // mesh (`parts_mesh`) at build time — per corner-post ownership for the
    // edge shapes, per footing depth for the foundations — and sits on the
    // root; a one-part shape carries its part's own transform, which is where
    // the stairs' pitch lives.
    let (mesh, transform) = if matches!(
        shape,
        sim_core::build::SHAPE_FOUNDATION | SHAPE_TRI_FOUNDATION
    ) {
        // The footing is per address — the terrain-following skirt
        // (`foundation_part`, the shared emit the ghost also draws) — picked
        // by the same call that sized it, already true-size, so the transform
        // carries no scale and the skirt wears a metre-scaled texture.
        let tri = shape == SHAPE_TRI_FOUNDATION;
        let part = foundation_part(seed, haven, cx, cz, tri, plate);
        let (step, _) = footing_of(seed, haven, cx, cz, plate);
        (
            kit.footing[step][usize::from(tri)].clone(),
            root * part.transform(),
        )
    } else if shape == SHAPE_WALL && (loc == LOC_DIAG_A || loc == LOC_DIAG_B) {
        let (parts, _) = diagonal_parts();
        (kit.diag_mesh.clone(), root * parts[0].transform())
    } else if is_edge_shape(shape) {
        (
            kit.edge_mesh[(shape as usize).min(N_SHAPES - 1)][own.bits() as usize]
                .clone()
                .expect("every edge shape has a mesh per ownership"),
            root,
        )
    } else {
        let (parts, _) = shape_parts(shape);
        (
            kit.shape_mesh[(shape as usize).min(N_SHAPES - 1)][0]
                .clone()
                .expect("every live part has a mesh"),
            root * parts[0].transform(),
        )
    };
    let mut piece = commands.spawn((
        super::WorldEntity,
        Mesh3d(mesh),
        MeshMaterial3d(mat.clone()),
        transform,
    ));
    // The ground storey's edge pieces hang their apron (`apron_parts`) as a
    // child at the root's own frame: the wall's plinth, in the wall's
    // material, owned like its posts. The root keeps the mesh the highlight
    // reads; a child is exactly what the one-entity contract allows.
    if addr.2 == 0 && is_edge_shape(shape) && matches!(loc, LOC_EDGE_XLO | LOC_EDGE_ZLO) {
        piece.with_child((
            Mesh3d(kit.apron[own.bits() as usize].clone()),
            MeshMaterial3d(mat),
            Transform::IDENTITY,
        ));
    }
    piece.id()
}

/// Where a deployable of `arch` stands at `addr` — the centre of its box,
/// posed. **The one emit site** both the standing deployable
/// ([`spawn_deploy`]) and the deploy ghost (`ghost::deploy_track`) use, the
/// parts-table seam applied to deployables (`NOW.md` §0u item 3): a door
/// used to preview as a box on the cell body because the ghost had its own
/// idea of where a deployable goes, and this function is now the only idea.
///
/// A door fills its doorway edge, oriented like the wall there; open, it
/// swings off the hinge end and lies across the cell — the same read the
/// sim's collision has, so a player never walks through a leaf that still
/// looks shut. Everything else stands on the level plane at cell centre.
pub fn deploy_transform(
    seed: u64,
    haven: &terrain::Haven,
    addr: Addr,
    arch: u8,
    open: bool,
    plate: i8,
) -> Transform {
    let idx = (arch as usize).min(DEPLOY.len() - 1);
    let [_, h, d] = DEPLOY[idx].0;
    let (cx, cz, level, loc) = addr;
    let base_y = level_base_y(seed, haven, cx, cz, level, plate);
    let (cxm, czm) = cell_center(cx, cz);
    let x0 = cx as f32 * BUILD_CELL_M;
    let z0 = cz as f32 * BUILD_CELL_M;
    let y = base_y + h * 0.5;
    let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);

    match (loc, open) {
        (LOC_EDGE_XLO, false) => Transform::from_xyz(x0, y, czm),
        (LOC_EDGE_XLO, true) => {
            Transform::from_xyz(x0 + d * 0.5, y, z0 + BUILD_CELL_M * 0.5 - d * 0.5)
                .with_rotation(quarter)
        }
        (LOC_EDGE_ZLO, false) => Transform::from_xyz(cxm, y, z0).with_rotation(quarter),
        (LOC_EDGE_ZLO, true) => {
            Transform::from_xyz(x0 + BUILD_CELL_M * 0.5 - d * 0.5, y, z0 + d * 0.5)
        }
        _ => Transform::from_xyz(cxm, y, czm),
    }
}

/// Public for `tests/fire.rs`, and for the reason `props::spawn_slot` is:
/// whether a burnable deployable gets a light child is a SPAWN-SHAPE claim, and
/// a spawn is not type-checked. Drop the `with_child` and every other gate in
/// this crate stays green over a shard where no fire lights anything.
#[allow(clippy::too_many_arguments)]
pub fn spawn_deploy(
    commands: &mut Commands,
    kit: &Kit,
    seed: u64,
    haven: &terrain::Haven,
    rec: &DeployRec,
    arch: u8,
    plate: i8,
) -> Entity {
    let idx = (arch as usize).min(DEPLOY.len() - 1);
    let transform = deploy_transform(
        seed,
        haven,
        (rec.cx, rec.cz, rec.level, rec.loc),
        arch,
        rec.open,
        plate,
    );

    let mat = if arch == ARCH_DOOR && rec.locked {
        kit.door_locked.clone()
    } else {
        kit.deploy_mat[idx].clone()
    };

    let mut e = commands.spawn((
        super::WorldEntity,
        Mesh3d(kit.deploy_mesh[idx].clone()),
        MeshMaterial3d(mat),
        transform,
    ));
    // A thing that burns gets a light, hung as a child and dark until the sim
    // says the fire is lit. See [`FireLight`].
    if burns(arch) {
        e.with_child((
            FireLight {
                cx: rec.cx,
                cz: rec.cz,
                level: rec.level,
            },
            PointLight {
                color: FIRE_COLOR,
                intensity: 0.0,
                range: FIRE_RANGE_M,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, FIRE_LIGHT_LIFT_M, 0.0),
        ));
    }
    e.id()
}

/// Archetypes that burn, as opposed to archetypes that merely convert.
///
/// **Narrower than [`is_converter`] on purpose**: that predicate covers the
/// recycler, which `oven::toggle` switches through the same verb and which
/// therefore appears in `ClientCore::ovens()` exactly as a campfire does. A
/// recycler is a machine with a motor in it and putting a warm flickering
/// light inside one would be this module inferring *fire* from *running*.
pub fn burns(arch: u8) -> bool {
    matches!(arch, ARCH_FIRE | ARCH_FURNACE)
}

/// The light a lit fire casts. Hung on every burnable deployable at spawn and
/// left at zero intensity until the sim says that address is alight.
///
/// **There was no dynamic light of any kind in this client.** `PointLight`,
/// `SpotLight` and a non-black `emissive` all returned zero across
/// `crates/client/src` — one directional sun, one hemisphere fill baked into a
/// cubemap, and nothing else. So a lit campfire cooked meat, made a crackling
/// sound and did not put one photon on the ground beside it, and night was ten
/// minutes in eighty of a uniform 60-lux `AmbientLight` (`rig::NIGHT_AMBIENT_
/// LUX`) that `fill.rs`'s own header proves is direction-free.
///
/// **It needs no wire change, which is why it is a small slice rather than a
/// `PROTO_VER` bump.** `DeployRec` carries no burning bit — but it does not
/// have to: `EV_OVEN` announces lit/unlit per address, `ClientCore` keeps the
/// set in `LitOvens`, and `verbs.rs` already reads it to decide what the
/// crosshair says. This reads the same set.
///
/// Shadows are OFF. A shadow-casting point light is six faces of re-rasterised
/// geometry per fire, `rig.rs` already spends four cascades on the sun, and a
/// base with six lit furnaces would multiply that for a light whose whole job
/// is a warm pool on the floor.
///
/// **It carries its own address rather than reading its parent's.** The light
/// is a child of the deployable so it inherits the pose, and the alternative —
/// a `ChildOf` hop into a second query — needs an address component on the
/// deployable that nothing else wants. Three fields on the marker is the whole
/// of it, and the fire's address never changes: a deployable that moves is a
/// deployable that was despawned and respawned.
#[derive(Component)]
pub struct FireLight {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
}

/// Fire colour. Not authored: `DEPLOY`'s own fire-pit row already carries
/// `Color::srgb(0.816, 0.439, 0.188)` as the greybox tint that stood in for the
/// mesh, which is a measured ember orange this file has been carrying since
/// before there was a model. Reused rather than re-picked so the light and the
/// thing it comes out of cannot drift.
///
/// **`pub` since torch light v0**, and for exactly the sentence above: a
/// held torch and a fire pit burn the same thing, so
/// `render::viewmodel::spawn_item` reads this rather than declaring a
/// second ember orange that would drift from it. It is the only flame
/// colour in the client and `tests/hand_light.rs` holds it to that.
///
/// **It survived a cross-check it was never picked against**, which is
/// worth writing down because it was luck rather than method. A sooting
/// diffusion flame — candle, match, pitch torch, wood fire — runs
/// 1 700–1 900 K, and the Planckian locus at 1 800 K is roughly linear
/// sRGB (1.00, 0.42, 0.10). These sRGB-encoded channels are linear
/// (1.00, 0.34, 0.06): the same hue, a little deeper, so the tint a
/// greybox row happened to carry lands inside the physical band rather
/// than beside it.
pub const FIRE_COLOR: Color = Color::srgb(1.0, 0.62, 0.28);

/// How far the pool of light reaches, metres. **(knob)**
///
/// A campfire is not a floodlight: past a few metres its contribution is under
/// the night ambient and all the range buys is more fragments touched. Six
/// metres covers the cell it stands in and the bodies around it.
const FIRE_RANGE_M: f32 = 6.0;

/// Lumens when lit. **(knob)** Bevy's `PointLight` is in lumens and its own
/// docs put a lightbulb near 1,000; a campfire is dimmer and warmer, and this
/// has to sit far enough under `rig::lux::DIRECT_SUNLIGHT` that a fire in
/// daylight is not a second sun. Registered in `DECISIONS.md` §open.
const FIRE_LUMENS: f32 = 900.0;

/// How far above the deployable's own origin the flame sits, metres. The
/// origin is the centre of the archetype's box, so a light at zero is inside
/// the fire ring rather than above it.
const FIRE_LIGHT_LIFT_M: f32 = 0.35;

/// Drive every fire light off the lit set the sim announces.
///
/// Reads the whole mirror rather than the change slice, for the reason
/// `structures.rs` states at the top of this file: a resync restates the world
/// and a renderer driven off deltas has to reproduce that state machine or
/// leave a fire burning that the server put out. Reading the set makes the
/// desync impossible by construction, and the set is capped (`MAX_BOXES`).
///
/// No per-frame allocation: the address rides the marker and `LitOvens::is_lit`
/// is a linear scan of a bounded array.
pub fn fire_lights(net: NonSend<super::Net>, q: Query<(&FireLight, &mut PointLight)>) {
    if q.is_empty() {
        return;
    }
    // The session is read through a predicate so the sweep below is testable
    // without a socket — `props::harvest` / `apply_fell` is the same split for
    // the same reason, and `LitOvens` is the authority exactly as
    // `HarvestedSet` is there.
    let ovens = net.session.core.ovens();
    apply_fire_lights(q, &|cx, cz, level| ovens.is_lit(cx, cz, level));
}

/// [`fire_lights`] with the lit set as a predicate. The half a gate can drive.
pub fn apply_fire_lights(
    mut q: Query<(&FireLight, &mut PointLight)>,
    lit: &dyn Fn(u16, u16, u8) -> bool,
) {
    for (fire, mut light) in q.iter_mut() {
        let want = if lit(fire.cx, fire.cz, fire.level) {
            FIRE_LUMENS
        } else {
            0.0
        };
        // Assign only on a change: `PointLight` is `Changed`-tracked and the
        // render world re-extracts what moved, so writing the same lumens every
        // frame would re-upload every fire on the shard forever.
        if light.intensity != want {
            light.intensity = want;
        }
    }
}

/// Lumens a lit fire delivers — read by the gate so the number is not written
/// twice.
pub fn fire_lumens() -> f32 {
    FIRE_LUMENS
}

/// Which archetypes a player can open. Stated here because it is a property
/// of the archetype table, not of the key that opens one.
pub fn is_container(arch: u8) -> bool {
    matches!(arch, ARCH_BOX | ARCH_BAG)
}

/// Which archetypes convert what is inside them — the ovens and the
/// recycler. `sim_core::oven::OvenState::arch_converts` is the sim's
/// answer and this is the client's read of the same fact; both are the
/// archetype table, so neither invents anything.
pub fn is_converter(arch: u8) -> bool {
    matches!(arch, ARCH_FIRE | ARCH_FURNACE | ARCH_RECYCLER)
}

/// Which archetypes are craft stations — the proximity tokens `craft.rs`
/// gates recipes on.
pub fn is_station(arch: u8) -> bool {
    matches!(
        arch,
        ARCH_WORKBENCH | ARCH_WORKBENCH2 | ARCH_WORKBENCH3 | ARCH_FURNACE | ARCH_FIRE | ARCH_HEARTH
    )
}

/// Which archetypes are checked by PROXIMITY rather than opened — the
/// craft stations above plus the research table, which gates no recipe and
/// so is deliberately not one of them (`craft::enqueue` reads `is_station`'s
/// set and would silently start accepting a table as a workbench).
pub fn is_proximity(arch: u8) -> bool {
    is_station(arch) || arch == ARCH_RESEARCH
}
