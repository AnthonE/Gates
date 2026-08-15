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
//! `sim_core::collide` by import (`PIECE_LIFT_M`, `WALL_THICKNESS_M`,
//! `DOOR_POST_W_M`) rather than by copied literal — one class of drift the
//! browser could not close and this can.
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

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::build::{
    BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_N, LOC_EDGE_W, SHAPE_DOORWAY, SHAPE_FRAME, SHAPE_STAIRS,
    SHAPE_WALL, SHAPE_WINDOW,
};
use sim_core::collide::{
    DOOR_POST_W_M, FRAME_RIM_M, PIECE_LIFT_M, WALL_THICKNESS_M, WINDOW_HEAD_M, WINDOW_SILL_M,
};
use sim_core::deploy::{
    DeployRec, ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE, ARCH_HEARTH, ARCH_RECYCLER,
    ARCH_RESEARCH, ARCH_WORKBENCH,
};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain;

use super::{Net, WorldId};

/// Plane-piece thickness, metres. Cosmetic — the sim's plane is a surface
/// height and not a slab (`collide.rs`), so this is how thick we *draw* it
/// and nothing stands on the underside.
pub const SLAB_T: f32 = 0.3;

/// The seam a drawn piece leaves at its cell boundary, metres. Without it,
/// two abutting floors z-fight along their shared edge for the whole length
/// of a base. `scene.js` carries the same 0.04 for the same reason.
pub const SEAM_M: f32 = 0.04;

/// Wood, stone, metal — colour, perceptual roughness, metallic.
///
/// Cosmetics (`DECISIONS.md` §open, client cosmetics). The response matters
/// as much as the colour: `ART.md` reads the reference's tier as *sheen* as
/// much as hue, so metal is a conductor with a real specular lobe and wood is
/// flat. A tier told apart by colour alone reads as three paints.
const TIER: [(Color, f32, f32); 3] = [
    (Color::srgb(0.541, 0.416, 0.271), 0.88, 0.0), // wood  0x8a6a45
    (Color::srgb(0.518, 0.514, 0.486), 0.72, 0.0), // stone 0x84837c
    (Color::srgb(0.373, 0.416, 0.447), 0.38, 0.85), // metal 0x5f6a72
];

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
const DEPLOY: [([f32; 3], Color, f32, f32); 10] = [
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
}

/// Shared meshes and materials, built once on first use. A base is hundreds
/// of pieces over five shapes and three materials; one `StandardMaterial`
/// per piece would be one draw call per piece.
struct Kit {
    /// One mesh per (shape, part), sized from [`shape_parts`] — the one
    /// table — and deduplicated by size, so the doorway's two posts share a
    /// mesh and the three slab shapes share one slab.
    shape_mesh: [[Option<Handle<Mesh>>; MAX_PARTS]; N_SHAPES],
    tier: [Handle<StandardMaterial>; 3],
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
    /// are canonical to a cell's west or north boundary, so the same physical
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
/// **This is collision truth, not a look.** `collide.rs`'s header states the
/// same expression — `terrain height + PIECE_LIFT_M + level·LEVEL_H_M` — and
/// calls it "the renderer's formula", because the sim walks players on a
/// surface derived from it. A renderer that drew the floor 10 cm off would
/// put every player ankle-deep in it or hovering above it, and no gate would
/// say so: the sim would be right and the picture wrong.
pub fn level_base_y(seed: u64, cx: u16, cz: u16, level: u8) -> f32 {
    let (cxm, czm) = cell_center(cx, cz);
    terrain::height(seed, cxm, czm) + PIECE_LIFT_M + level as f32 * LEVEL_H_M
}

/// The world XZ of a cell's centre.
pub fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    )
}

/// How far a stairs ramp runs in Z, metres. Shared with the build ghost for
/// the same reason the doorway numbers are: a preview the wrong length is a
/// preview of a different piece.
pub const STAIRS_RUN_M: f32 = 4.15;

/// The doorway's lintel: how tall it is, and how far its centre sits below the
/// piece's own mid-height.
///
/// **Public because the BUILD GHOST draws the same doorway** and the two must
/// not disagree. `RENDER.md` §8 states the stake in one line — the opening is
/// 1.2 m x 2.1 m because `collide::edge_hit` blocks exactly `t` in `[0, 0.9]`
/// and `[2.1, 3.0]`, so "draw it elsewhere and the frame lies about where a
/// player can walk". A ghost that lies about it lies one step earlier, while
/// the player is still deciding.
///
/// Derivation, and it is why these are two constants rather than one: the
/// lintel's underside must land at 2.1 m so the opening is the height the sim
/// blocks. At `LEVEL_H_M` 3.0 the piece's centre is 1.5, so a 0.9-tall lintel
/// dropped 0.45 from centre spans 2.1..3.0 — exactly the band `edge_hit`
/// refuses.
pub const LINTEL_H_M: f32 = 0.9;
/// How far the lintel's centre sits below the piece's mid-height, metres.
pub const LINTEL_DROP_M: f32 = 0.45;

/// Half the distance between the two door posts' centres, metres — each post
/// hugs one end of the edge and the gap between them is the opening.
pub fn door_post_gap() -> f32 {
    (BUILD_CELL_M - SEAM_M - DOOR_POST_W_M) * 0.5
}

/// The clear span between the posts, metres. The lintel spans exactly this.
pub fn door_opening_w() -> f32 {
    ((BUILD_CELL_M - SEAM_M) - 2.0 * DOOR_POST_W_M).max(0.1)
}

/// One drawn part of a build shape: the box's full extents, its centre, and
/// the pitch it carries.
///
/// The offset is relative to the piece's **base point** — the address's
/// canonical anchor in the west/plane orientation: the west boundary's
/// midpoint for an edge piece, the cell centre for a body piece, always at
/// the level's base height (`level_base_y`). A north edge is the same parts
/// under the root's quarter-turn ([`base_transform`]), exactly as the sim
/// canonicalises the two edges to one shape (`build.rs`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Part {
    /// Full extents of the box, metres.
    pub size: Vec3,
    /// The box centre, metres, relative to the base point.
    pub offset: Vec3,
    /// Rotation about the part's local X axis, radians — the stairs' ramp
    /// pitch. Zero for every other shape.
    pub x_rot: f32,
}

impl Part {
    /// The part's transform relative to the base point. Scale is untouched,
    /// so the ghost can put the unit cube's scale on top of it.
    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.offset).with_rotation(Quat::from_rotation_x(self.x_rot))
    }
}

/// The most parts any shape emits — the window's sill, header and two
/// jambs (the doorway held this at 3 until catalogue v1).
pub const MAX_PARTS: usize = 4;

/// How many shapes the parts table covers: the sim's own last shape, plus
/// one. A shape past it is drawn as the fallback slab, same as one the
/// table has no arm for.
pub const N_SHAPES: usize = SHAPE_FRAME as usize + 1;

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
    let span = BUILD_CELL_M - SEAM_M;
    let none = Part {
        size: Vec3::ZERO,
        offset: Vec3::ZERO,
        x_rot: 0.0,
    };
    match shape {
        SHAPE_WALL => (
            [
                Part {
                    size: Vec3::new(WALL_THICKNESS_M, LEVEL_H_M, span),
                    offset: Vec3::new(0.0, LEVEL_H_M * 0.5, 0.0),
                    x_rot: 0.0,
                },
                none,
                none,
                none,
            ],
            1,
        ),
        SHAPE_DOORWAY => {
            // Two posts hugging each end of the edge, and the lintel over
            // what they leave.
            let gap = door_post_gap();
            let post = |z: f32| Part {
                size: Vec3::new(WALL_THICKNESS_M, LEVEL_H_M, DOOR_POST_W_M),
                offset: Vec3::new(0.0, LEVEL_H_M * 0.5, z),
                x_rot: 0.0,
            };
            (
                [
                    post(-gap),
                    post(gap),
                    // The lintel's underside is the top of the opening the
                    // sim refuses to let a player through: centred at
                    // `LEVEL_H_M - LINTEL_DROP_M` = 2.55 m, it spans exactly
                    // 2.1..3.0 (`LINTEL_H_M`'s own derivation, above).
                    Part {
                        size: Vec3::new(WALL_THICKNESS_M, LINTEL_H_M, door_opening_w()),
                        offset: Vec3::new(0.0, LEVEL_H_M - LINTEL_DROP_M, 0.0),
                        x_rot: 0.0,
                    },
                    none,
                ],
                3,
            )
        }
        SHAPE_STAIRS => (
            [
                // A ramp through the level. The grid stores no facing, so it
                // always rises toward +Z (cosmetic v0 — the browser's choice
                // too), pitched the way the standing piece has always been.
                Part {
                    size: Vec3::new(span, SLAB_T, STAIRS_RUN_M),
                    offset: Vec3::new(0.0, LEVEL_H_M * 0.5, 0.0),
                    x_rot: -std::f32::consts::FRAC_PI_4,
                },
                none,
                none,
                none,
            ],
            1,
        ),
        SHAPE_WINDOW => {
            // Sill, header, and two jambs around the aperture the sim's
            // shot walk passes — every extent is the collision constant
            // itself (`collide::window_solid_at`), so the drawn hole IS
            // the hole an arrow threads. The jambs reuse the doorway's
            // post width and gap on purpose: one opening family, one set
            // of numbers.
            let gap = door_post_gap();
            let jamb_h = WINDOW_HEAD_M - WINDOW_SILL_M;
            (
                [
                    Part {
                        size: Vec3::new(WALL_THICKNESS_M, WINDOW_SILL_M, span),
                        offset: Vec3::new(0.0, WINDOW_SILL_M * 0.5, 0.0),
                        x_rot: 0.0,
                    },
                    Part {
                        size: Vec3::new(WALL_THICKNESS_M, LEVEL_H_M - WINDOW_HEAD_M, span),
                        offset: Vec3::new(0.0, (LEVEL_H_M + WINDOW_HEAD_M) * 0.5, 0.0),
                        x_rot: 0.0,
                    },
                    Part {
                        size: Vec3::new(WALL_THICKNESS_M, jamb_h, DOOR_POST_W_M),
                        offset: Vec3::new(0.0, (WINDOW_SILL_M + WINDOW_HEAD_M) * 0.5, -gap),
                        x_rot: 0.0,
                    },
                    Part {
                        size: Vec3::new(WALL_THICKNESS_M, jamb_h, DOOR_POST_W_M),
                        offset: Vec3::new(0.0, (WINDOW_SILL_M + WINDOW_HEAD_M) * 0.5, gap),
                        x_rot: 0.0,
                    },
                ],
                4,
            )
        }
        SHAPE_FRAME => {
            // The rim and nothing else — two thin jambs and the top beam,
            // each `FRAME_RIM_M` thick, which is exactly the solid
            // `collide::frame_solid_at` answers for. The opening is the
            // piece.
            let jamb_off = (span - FRAME_RIM_M) * 0.5;
            let jamb = |z: f32| Part {
                size: Vec3::new(WALL_THICKNESS_M, LEVEL_H_M - FRAME_RIM_M, FRAME_RIM_M),
                offset: Vec3::new(0.0, (LEVEL_H_M - FRAME_RIM_M) * 0.5, z),
                x_rot: 0.0,
            };
            (
                [
                    jamb(-jamb_off),
                    jamb(jamb_off),
                    Part {
                        size: Vec3::new(WALL_THICKNESS_M, FRAME_RIM_M, span),
                        offset: Vec3::new(0.0, LEVEL_H_M - FRAME_RIM_M * 0.5, 0.0),
                        x_rot: 0.0,
                    },
                    none,
                ],
                3,
            )
        }
        // Foundation / floor / roof — and any shape the defs name that this
        // table does not: the slab whose TOP is the level plane, which is
        // the surface the sim stands players on.
        _ => (
            [
                Part {
                    size: Vec3::new(span, SLAB_T, span),
                    offset: Vec3::new(0.0, -SLAB_T * 0.5, 0.0),
                    x_rot: 0.0,
                },
                none,
                none,
                none,
            ],
            1,
        ),
    }
}

/// The world transform of an address's base point: the canonical anchor
/// [`shape_parts`]' offsets are relative to, plus the quarter-turn a north
/// edge carries. Shared with the build ghost for the reason the parts are:
/// the ghost and the piece it becomes must be the same object in the same
/// pose, and edge canonicalisation written twice is how they stop being.
pub fn base_transform(seed: u64, (cx, cz, level, loc): Addr) -> Transform {
    let base_y = level_base_y(seed, cx, cz, level);
    let (cxm, czm) = cell_center(cx, cz);
    let (pos, yaw) = match loc {
        LOC_EDGE_W => (Vec3::new(cx as f32 * BUILD_CELL_M, base_y, czm), 0.0),
        LOC_EDGE_N => (
            Vec3::new(cxm, base_y, cz as f32 * BUILD_CELL_M),
            std::f32::consts::FRAC_PI_2,
        ),
        _ => (Vec3::new(cxm, base_y, czm), 0.0),
    };
    Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw))
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
];

fn build_kit(
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Kit {
    let tier = std::array::from_fn(|i| {
        let (base_color, perceptual_roughness, metallic) = TIER[i];
        materials.add(StandardMaterial {
            base_color,
            perceptual_roughness,
            metallic,
            ..default()
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
    // exact size — the sizes that repeat are the same expressions evaluated
    // twice, so `==` is the right comparison and a near-miss SHOULD build a
    // second mesh, because a near-miss is two parts claiming to differ.
    const NO_MESH: Option<Handle<Mesh>> = None;
    // The outer repeat can't be `[[NO_MESH; MAX_PARTS]; N_SHAPES]`: only the
    // inner repeat's operand is a const item — the outer would Copy a built
    // non-Copy row (E0277). `from_fn` evaluates the const repeat per row.
    let mut shape_mesh: [[Option<Handle<Mesh>>; MAX_PARTS]; N_SHAPES] =
        std::array::from_fn(|_| [NO_MESH; MAX_PARTS]);
    let mut sized: Vec<(Vec3, Handle<Mesh>)> = Vec::new();
    for (shape, row) in shape_mesh.iter_mut().enumerate() {
        let (parts, n) = shape_parts(shape as u8);
        for (slot, part) in row.iter_mut().zip(&parts[..n]) {
            let handle = match sized.iter().find(|(size, _)| *size == part.size) {
                Some((_, h)) => h.clone(),
                None => {
                    let h = meshes.add(Cuboid::new(part.size.x, part.size.y, part.size.z));
                    sized.push((part.size, h.clone()));
                    h
                }
            };
            *slot = Some(handle);
        }
    }
    Kit {
        shape_mesh,
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
        if let Some(live) = ring.pieces.get_mut(&key) {
            live.seen = gen;
            if live.row == rec.row {
                continue;
            }
            // An upgrade in place: same address, new material.
            commands.entity(live.entity).despawn();
            ring.pieces.remove(&key);
        }
        let def = core.piece_defs.pieces[rec.row as usize];
        let entity = spawn_piece(&mut commands, kit, seed, key, def.shape, def.material);
        ring.pieces.insert(
            key,
            Live {
                entity,
                seen: gen,
                row: rec.row,
                open: false,
                locked: false,
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
        if let Some(live) = ring.deploys.get_mut(&key) {
            live.seen = gen;
            // A door swing and a lock are both redraws at one address.
            if live.row == rec.row && live.open == rec.open && live.locked == rec.locked {
                continue;
            }
            commands.entity(live.entity).despawn();
            ring.deploys.remove(&key);
        }
        let arch = core.deploy_defs.defs[rec.row as usize].arch;
        let entity = spawn_deploy(&mut commands, kit, seed, rec, arch);
        ring.deploys.insert(
            key,
            Live {
                entity,
                seen: gen,
                row: rec.row,
                open: rec.open,
                locked: rec.locked,
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

fn spawn_piece(
    commands: &mut Commands,
    kit: &Kit,
    seed: u64,
    addr: Addr,
    shape: u8,
    material: u8,
) -> Entity {
    let mat = kit.tier[(material as usize).min(2)].clone();
    // Edge pieces stand on the cell's west (x = cx·3) or north (z = cz·3)
    // boundary — canonical, so one physical edge is never addressable twice
    // (`build.rs`) — and the parts are the shared table's, so this and the
    // build ghost are the same object in the same pose.
    let root = base_transform(seed, addr);
    let (parts, n) = shape_parts(shape);
    let meshes = &kit.shape_mesh[(shape as usize).min(N_SHAPES - 1)];

    if n == 1 {
        // A one-part shape stays ONE entity with the mesh on it, root and
        // part composed. Not a style choice: the hammer highlight reads
        // `Transform` + `Mesh3d` off the entity `entity_at` answers
        // (`highlight.rs`), and a bare root over a single child would hide
        // both from it.
        return commands
            .spawn((
                super::WorldEntity,
                Mesh3d(meshes[0].clone().expect("every live part has a mesh")),
                MeshMaterial3d(mat),
                root * parts[0].transform(),
            ))
            .id();
    }
    commands
        .spawn((super::WorldEntity, root, Visibility::default()))
        .with_children(|c| {
            for (part, mesh) in parts[..n].iter().zip(meshes) {
                c.spawn((
                    Mesh3d(mesh.clone().expect("every live part has a mesh")),
                    MeshMaterial3d(mat.clone()),
                    part.transform(),
                ));
            }
        })
        .id()
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
pub fn deploy_transform(seed: u64, addr: Addr, arch: u8, open: bool) -> Transform {
    let idx = (arch as usize).min(DEPLOY.len() - 1);
    let [_, h, d] = DEPLOY[idx].0;
    let (cx, cz, level, loc) = addr;
    let base_y = level_base_y(seed, cx, cz, level);
    let (cxm, czm) = cell_center(cx, cz);
    let x0 = cx as f32 * BUILD_CELL_M;
    let z0 = cz as f32 * BUILD_CELL_M;
    let y = base_y + h * 0.5;
    let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);

    match (loc, open) {
        (LOC_EDGE_W, false) => Transform::from_xyz(x0, y, czm),
        (LOC_EDGE_W, true) => {
            Transform::from_xyz(x0 + d * 0.5, y, z0 + BUILD_CELL_M * 0.5 - d * 0.5)
                .with_rotation(quarter)
        }
        (LOC_EDGE_N, false) => Transform::from_xyz(cxm, y, z0).with_rotation(quarter),
        (LOC_EDGE_N, true) => {
            Transform::from_xyz(x0 + BUILD_CELL_M * 0.5 - d * 0.5, y, z0 + d * 0.5)
        }
        _ => Transform::from_xyz(cxm, y, czm),
    }
}

fn spawn_deploy(
    commands: &mut Commands,
    kit: &Kit,
    seed: u64,
    rec: &DeployRec,
    arch: u8,
) -> Entity {
    let idx = (arch as usize).min(DEPLOY.len() - 1);
    let transform = deploy_transform(seed, (rec.cx, rec.cz, rec.level, rec.loc), arch, rec.open);

    let mat = if arch == ARCH_DOOR && rec.locked {
        kit.door_locked.clone()
    } else {
        kit.deploy_mat[idx].clone()
    };

    commands
        .spawn((
            super::WorldEntity,
            Mesh3d(kit.deploy_mesh[idx].clone()),
            MeshMaterial3d(mat),
            transform,
        ))
        .id()
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
        ARCH_WORKBENCH | ARCH_FURNACE | ARCH_FIRE | ARCH_HEARTH
    )
}

/// Which archetypes are checked by PROXIMITY rather than opened — the
/// craft stations above plus the research table, which gates no recipe and
/// so is deliberately not one of them (`craft::enqueue` reads `is_station`'s
/// set and would silently start accepting a table as a workbench).
pub fn is_proximity(arch: u8) -> bool {
    is_station(arch) || arch == ARCH_RESEARCH
}
