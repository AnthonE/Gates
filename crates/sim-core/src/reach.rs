//! **A hand's reach: how far, how high, and through what.**
//!
//! Every container verb measured reach one way — `BUILD_REACH_M` on the flat
//! (`backpack::LOOT_REACH_M` is the same arm) — and flat alone passed walls
//! and floors. A box in a walled 1×1 could be emptied from outside it, a
//! furnace or a research table worked through a wall, a bag on the floor
//! above taken from below. The reference game stops all of that with a line
//! of sight (`reference/PROJECTILES.md`), and `NETCODE.md` promised the same.
//!
//! The planar test stays where it was, each store's own `in_reach`; this
//! module is the other two halves, asked after it:
//!
//! - **a clear line**: nothing built between the eye and the thing, the shot
//!   walk ([`collide::shot_blocked`]) stepped the way a shot steps it, and
//!   none of the ziggurat's or a landmark's stone;
//! - **the same storey**, for a thing standing on a floor (a box, an oven, a
//!   research table): within [`LOOT_REACH_DY_M`] of the player's feet, the
//!   world crate's rule (`worldcont.rs`). A thing lying where it fell — a
//!   bag, a loose stack, a sleeper — answers to the line alone: a floor in
//!   the way already blocks it, and a bag left hanging where a floor was
//!   broken under it must still be reachable from below.
//!
//! Pure and bounded: a fixed sample cap, no allocation, `+ − × ÷ sqrt` only.

use crate::collide::{self, ColIndex};
use crate::deploy::{DeployContent, Deploys};
use crate::grounditem::GroundItemRec;
use crate::hull::Hull;
use crate::limits::{ARROW_STEP_MM, HAND_SIGHT_TESTS_MAX};
use crate::movement::{POS_XZ_Q, POS_Y_Q, STEP_UP};
use crate::ranged::{ARROW_R_M, MM_PER_M};
use crate::terrain::Haven;
use crate::world::Player;

/// How far above or below a player a container may be and still be reached,
/// metres — under a storey (`build::LEVEL_H_M`), so a room's floor is never
/// reached through from the room below.
pub use crate::worldcont::LOOT_REACH_DY_M;

/// What a thing lying on a floor is aimed at, above the surface it rests on:
/// clear of the slab under it, which the walk's last sample would otherwise
/// read as a floor in the way.
pub const LIFT_M: f32 = 0.3;

/// Most taps one walk takes. Planar reach is 5 m and the storey rule 2.5 m,
/// so a walk is under 6 m: 36 taps at `ARROW_STEP_MM`.
const HAND_SAMPLES: usize = 48;

/// The eye a hand reaches from, metres: the body plus its stance's eye — the
/// origin a swing and a shot use (`melee::ray`).
pub fn eye(p: &Player) -> (f32, f32, f32) {
    eye_of(&p.body, p.crouched())
}

/// [`eye`] for a bare body and its stance: what a client or an agent asks
/// from its predicted body, so its prompt and the shard's verb agree.
pub fn eye_of(b: &crate::movement::Body, crouched: bool) -> (f32, f32, f32) {
    (
        b.qx as f32 * POS_XZ_Q,
        b.qy as f32 * POS_Y_Q + crate::ranged::eye_mm(crouched) as f32 / MM_PER_M,
        b.qz as f32 * POS_XZ_Q,
    )
}

/// Nothing built between `from` and `to` (metres): walls, shut doors, window
/// and frame rims, floors and roofs, and the ziggurat's and landmarks' stone,
/// its card doors shut unless `doors` has their bit (`monument::open_bits`).
/// The thing reached for, other furniture, terrain and bodies never block a
/// hand.
pub fn sight(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    doors: u32,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> bool {
    let (dx, dy, dz) = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
    let len_mm = (dx * dx + dy * dy + dz * dz).sqrt() * MM_PER_M;
    let n = ((len_mm / ARROW_STEP_MM as f32) as usize + 1).min(HAND_SAMPLES);
    // Once per walk, not per sample: a landmark no sample can come near is
    // one no sample has to ask (the walk is under 6 m from `from`).
    let marks =
        crate::landmark::covers(&haven.marks, from.0, from.2, len_mm / MM_PER_M + ARROW_R_M);
    let mut prev = (from.0, from.2);
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let (x, y, z) = (from.0 + dx * t, from.1 + dy * t, from.2 + dz * t);
        if collide::shot_blocked(seed, haven, cols, prev.0, prev.1, x, z, y, ARROW_R_M) {
            return false;
        }
        // The island's own stone, which the shot walk does not see: it
        // walks built pieces, and nothing may be built at the ziggurat or a
        // landmark, so near them that walk is empty — a sanctum crate opened
        // from the ledge outside its wall. A point test (feet a step below
        // it, a step tall), so a floor slab blocks as surely as a wall. Card
        // doors as they stand now (`doors`, `monument::open_bits`): a shut
        // one stops a hand, an open one is a doorway a bag can lie in.
        let feet = y - STEP_UP;
        if crate::monument::blocks(&haven.ziggurat, doors, x, z, feet, ARROW_R_M, STEP_UP)
            || (marks && crate::landmark::blocks(&haven.marks, x, z, feet, ARROW_R_M, STEP_UP))
        {
            return false;
        }
        prev = (x, z);
    }
    true
}

/// How far above its feet a body lying or standing is aimed at, metres.
pub const BODY_MID_M: f32 = 0.9;

/// How far above its floor a door is aimed at, metres: the handle's height.
pub const DOOR_MID_M: f32 = 1.0;

/// How far out of its wall's plane a door is aimed at, on the side the eye
/// stands, metres: clear of half the wall slab
/// (`collide::WALL_THICKNESS_M`) and the walk's own radius, so neither the
/// shut leaf nor the doorway's rim is read as in the way.
pub const DOOR_SIDE_M: f32 = 0.3;

/// How far a stuck arrow is aimed off the point it went in, back along its
/// flight, metres — clear of the surface it is stuck in, which the walk's
/// last sample would otherwise read as in the way.
pub const STUCK_BACK_M: f32 = 0.1;

/// Where a hand aims at a thing: its `(x, z)`, the floor it stands or lies
/// on, how far above that floor its middle is, and whether the storey rule
/// applies to it (a thing standing on a floor) or the line alone (a thing
/// lying where it fell).
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub x: f32,
    pub z: f32,
    pub base_y: f32,
    pub mid: f32,
    pub storey: bool,
}

impl Target {
    /// A thing lying where its quantized position says — a bag, a loose
    /// stack, a sleeper — aimed [`LIFT_M`] above where it rests, or above a
    /// built surface laid over it since (a foundation placed on a bag would
    /// otherwise bury it out of everyone's reach).
    pub fn lying(seed: u64, haven: &Haven, cols: &ColIndex, q: (i32, i32, i32)) -> Self {
        let (x, y, z) = (
            q.0 as f32 * POS_XZ_Q,
            q.1 as f32 * POS_Y_Q,
            q.2 as f32 * POS_XZ_Q,
        );
        let over = collide::piece_ground(seed, haven, cols, x, z, y + LIFT_M);
        Self {
            x,
            z,
            base_y: y.max(over),
            mid: LIFT_M,
            storey: false,
        }
    }

    /// An arrow stuck where it went in, flying along `dir`: aimed
    /// [`STUCK_BACK_M`] back along its flight, so the surface it is stuck in
    /// — a wall, the underside of a floor — is not in the way of reaching it.
    pub fn stuck(q: (i32, i32, i32), dir: [i8; 3]) -> Self {
        let d = (dir[0] as f32, dir[1] as f32, dir[2] as f32);
        let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt().max(1.0);
        let k = STUCK_BACK_M / len;
        Self {
            x: q.0 as f32 * POS_XZ_Q - d.0 * k,
            z: q.2 as f32 * POS_XZ_Q - d.2 * k,
            base_y: q.1 as f32 * POS_Y_Q - d.1 * k,
            mid: 0.0,
            storey: false,
        }
    }

    /// [`Self::stuck`], lifted out of the island's stone. An arrow's stop
    /// ladder passes through the top step of a ziggurat or landmark box —
    /// that band is ground to a body (`kit::blocks_local`) — so one landing
    /// on a slab or a ledge sinks into it, where the hand's point test reads
    /// it as inside the stone. Aimed just above that top instead.
    pub fn stuck_in(haven: &Haven, doors: u32, q: (i32, i32, i32), dir: [i8; 3]) -> Self {
        let mut t = Self::stuck(q, dir);
        let top = crate::monument::ground(&haven.ziggurat, doors, t.x, t.z, t.base_y, 0.0).max(
            crate::landmark::ground(&haven.marks, t.x, t.z, t.base_y, 0.0),
        );
        if top > t.base_y {
            t.base_y = top;
            // Above the top, not on it: the walk's last sample can land an
            // ulp under the target.
            t.mid = ARROW_R_M;
        }
        t
    }

    /// A loose stack: [`Self::stuck_in`] for an arrow stuck in something,
    /// [`Self::lying`] for everything on the ground.
    pub fn stack(seed: u64, haven: &Haven, cols: &ColIndex, doors: u32, g: &GroundItemRec) -> Self {
        if g.dir == [0; 3] {
            Self::lying(seed, haven, cols, (g.qx, g.qy, g.qz))
        } else {
            Self::stuck_in(haven, doors, (g.qx, g.qy, g.qz), g.dir)
        }
    }

    /// A body standing or lying with its feet at `(x, y, z)` metres, aimed
    /// at its middle: an arrow lodged in it is pulled from there.
    pub fn body(x: f32, y: f32, z: f32) -> Self {
        Self {
            x,
            z,
            base_y: y,
            mid: BODY_MID_M,
            storey: false,
        }
    }

    /// A deployable where it stands now (`deploy::body_base_y`, the floor
    /// the renderer and the collision walks stand it on), aimed at the
    /// middle of its body. The storey rule applies: it stands on a floor.
    pub fn standing(
        seed: u64,
        haven: &Haven,
        cols: &ColIndex,
        rec: &crate::deploy::DeployRec,
        arch: u8,
    ) -> Self {
        let (x, z) = rec.xz();
        Self {
            x,
            z,
            base_y: crate::deploy::body_base_y(seed, haven, cols, rec, arch),
            mid: crate::deploy::solid_vol(arch).map_or(LIFT_M, |(_, h, _)| h * 0.5),
            storey: true,
        }
    }

    /// A door, a garage door or a shutter hanging in a doorway (an edge
    /// address), reached from an eye at `eye_xz`: aimed at the leaf's middle,
    /// [`DOOR_SIDE_M`] out on the eye's side of the wall. A shut leaf and
    /// its own frame are then never in the way, and a walk that has to go
    /// through a floor, a roof or another wall still is. The storey rule
    /// applies: a door is not worked from the room above (the 2026-10-19
    /// playtest opened one "from like above").
    pub fn door(
        seed: u64,
        haven: &Haven,
        cols: &ColIndex,
        rec: &crate::deploy::DeployRec,
        eye_xz: (f32, f32),
    ) -> Self {
        let side = |d: f32| if d < 0.0 { -DOOR_SIDE_M } else { DOOR_SIDE_M };
        // An edge the grid cannot name (freeform W4): out along its own
        // normal, on the eye's side, from its own middle and floor.
        if let Some(at) = crate::deploy::posed_insert(cols, rec.cx, rec.cz, rec.level, rec.loc) {
            let (nx, nz) = at.normal;
            let s = side((eye_xz.0 - at.x) * nx + (eye_xz.1 - at.z) * nz);
            return Self {
                x: at.x + nx * s,
                z: at.z + nz * s,
                base_y: at.floor_y,
                mid: DOOR_MID_M,
                storey: true,
            };
        }
        let (ax, az) = crate::build::anchor(rec.cx, rec.cz, rec.loc);
        let (x, z) = if rec.loc == crate::build::LOC_EDGE_XLO {
            (ax + side(eye_xz.0 - ax), az)
        } else {
            (ax, az + side(eye_xz.1 - az))
        };
        Self {
            x,
            z,
            base_y: crate::collide::col_base_y(seed, haven, cols, rec.cx, rec.cz)
                + crate::build::level_y(rec.level),
            mid: DOOR_MID_M,
            storey: true,
        }
    }

    /// Any deployable a hand works: [`Self::door`] for one hanging in a
    /// doorway, [`Self::standing`] for everything else.
    pub fn deploy(
        seed: u64,
        haven: &Haven,
        cols: &ColIndex,
        rec: &crate::deploy::DeployRec,
        arch: u8,
        eye_xz: (f32, f32),
    ) -> Self {
        if crate::deploy::is_edge_loc(rec.loc) {
            Self::door(seed, haven, cols, rec, eye_xz)
        } else {
            Self::standing(seed, haven, cols, rec, arch)
        }
    }

    /// A world container (`worldcont.rs`) at the quantized stand position
    /// its record carries, `(qx, qy, qz)` — `qy` the floor it stands on —
    /// aimed at the middle of its body. The storey rule applies: it stands
    /// on a floor, and the ziggurat stacks rooms.
    pub fn world_cont(q: (i32, i32, i32), o: crate::terrain::Occupant) -> Self {
        Self {
            x: q.0 as f32 * POS_XZ_Q,
            z: q.2 as f32 * POS_XZ_Q,
            base_y: q.1 as f32 * POS_Y_Q,
            mid: crate::terrain::occupant_volume(o).1 * 0.5,
            storey: true,
        }
    }

    /// [`Self::world_cont`] for a scatter slot not yet opened: the record
    /// `worldcont::open` would mint, so a prompt aims where the verb will.
    /// (Not named for the scatter: the client's memo gate greps its swing
    /// path for that call.)
    pub fn unopened(s: &crate::terrain::Slot) -> Self {
        Self::world_cont(crate::worldcont::stand_q(s.x, s.y, s.z), s.occupant)
    }
}

/// Whether a hand with its eye at `eye` and its feet at `feet_y` reaches
/// `t`: in plain sight of the eye, and within a storey of the feet when `t`
/// stands on a floor. The planar distance is the caller's. The client asks this from its predicted body to
/// offer `E` only where the shard would take it.
#[allow(clippy::too_many_arguments)]
pub fn clear_from(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    doors: u32,
    eye: (f32, f32, f32),
    feet_y: f32,
    t: Target,
) -> bool {
    if t.storey && crate::fmath::fabs(feet_y - t.base_y) > LOOT_REACH_DY_M {
        return false;
    }
    sight(seed, haven, cols, doors, eye, (t.x, t.base_y + t.mid, t.z))
}

/// A player's feet, metres — what [`clear_from`] measures a storey from.
pub fn feet_y(p: &Player) -> f32 {
    p.body.qy as f32 * POS_Y_Q
}

/// [`clear_from`] for a player as the sim has them.
pub fn clear(seed: u64, haven: &Haven, cols: &ColIndex, doors: u32, p: &Player, t: Target) -> bool {
    clear_from(seed, haven, cols, doors, eye(p), feet_y(p), t)
}

/// [`clear_from`] for entry `i` of the box store — a box, an oven, a research
/// table — aimed at the middle of its body. False for an index with no
/// record behind it.
#[allow(clippy::too_many_arguments)]
pub fn clear_box(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    doors: u32,
    content: &DeployContent,
    deploys: &Deploys,
    i: usize,
    eye: (f32, f32, f32),
    feet_y: f32,
) -> bool {
    let Some(b) = deploys.boxes().get(i) else {
        return false;
    };
    let Some(rec) = deploys.find(b.cx, b.cz, b.level, b.loc) else {
        return false;
    };
    let Some(def) = content.defs.get(rec.row as usize) else {
        return false;
    };
    let t = Target::standing(seed, haven, cols, rec, def.arch);
    clear_from(seed, haven, cols, doors, eye, feet_y, t)
}

/// [`clear_from`] for the door (or locked box) at an address, as the use
/// and lock verbs find it. True when nothing stands there: the verb's own
/// refusal names that, not this test.
#[allow(clippy::too_many_arguments)]
pub fn clear_door(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    doors: u32,
    content: &DeployContent,
    deploys: &Deploys,
    addr: (u16, u16, u8, u8),
    eye: (f32, f32, f32),
    feet_y: f32,
) -> bool {
    let Some(rec) = deploys.find(addr.0, addr.1, addr.2, addr.3) else {
        return true;
    };
    let Some(def) = content.defs.get(rec.row as usize) else {
        return true;
    };
    let t = Target::deploy(seed, haven, cols, rec, def.arch, (eye.0, eye.2));
    clear_from(seed, haven, cols, doors, eye, feet_y, t)
}

/// Most points [`piece_faces`] offers: a foundation's top and the two of
/// its sides that face the eye.
pub const PIECE_FACES: usize = 3;

/// The collision row of the piece at `addr` (`hull.rs`) — a grid address
/// or a spelled id, the row is the piece's shape either way.
pub fn piece_hull(pieces: &crate::build::Pieces, addr: (u16, u16, u8, u8)) -> Option<&Hull> {
    let rec = pieces.find(addr.0, addr.1, addr.2, addr.3)?;
    pieces.cols().hulls().find(crate::build::hull_key(rec.id))
}

/// Where a hand aims at the built piece `h` from `eye`: a point just off
/// each face of it the eye could be looking at — [`DOOR_SIDE_M`] out of a
/// wall on the eye's side, [`LIFT_M`] above a slab and below its
/// underside, and out from a foundation's sides toward the eye. Taken at a
/// face's nearest stretch to the eye, a margin in from its ends, so a wall
/// seen end-on is aimed at its own face and never round its corner. A face
/// behind another wall, a floor or the piece itself is a point the walk
/// cannot reach ([`clear_piece`]): the second wall of two, a ceiling from
/// the room above it, a foundation's top from under the base.
pub fn piece_faces(h: &Hull, eye: (f32, f32, f32)) -> ([(f32, f32, f32); PIECE_FACES], usize) {
    use crate::build::{BUILD_CELL_M, LEVEL_H_M, LOC_DIAG_B, SHAPE_FOUNDATION_STEPS};
    use crate::hull::{HULL_DIAG, HULL_EDGE, HULL_HALF, HULL_RISER, HULL_TRI};
    let half = BUILD_CELL_M * 0.5;
    let side = |d: f32| if d < 0.0 { -DOOR_SIDE_M } else { DOOR_SIDE_M };
    let inset = |v: f32| v.clamp(DOOR_SIDE_M, BUILD_CELL_M - DOOR_SIDE_M);
    // A point in the row's own frame, in the world.
    let world = |a: f32, b: f32| (h.ox + a * h.ux + b * h.vx, h.oz + a * h.uz + b * h.vz);
    let (lx, lz) = h.local(eye.0, eye.2);
    let mut out = [(0.0, 0.0, 0.0); PIECE_FACES];
    if h.kind == HULL_EDGE || h.kind == HULL_DIAG {
        // A wall's middle, or a door's handle on a tall one.
        let y = h.floor_y + (h.height() * 0.5).min(DOOR_MID_M);
        let (a, b) = if h.kind == HULL_EDGE {
            (inset(lx), side(lz))
        } else {
            // The cell's diagonal: A runs (0,0)->(3,3), B (3,0)->(0,3);
            // `d` along it, `w` across it.
            let k = core::f32::consts::FRAC_1_SQRT_2;
            let (dx, dz) = if h.loc == LOC_DIAG_B { (-k, k) } else { (k, k) };
            let (wx, wz) = (dz, -dx);
            let (ex, ez) = (lx - half, lz - half);
            let reach = half * core::f32::consts::SQRT_2 - DOOR_SIDE_M;
            let t = (ex * dx + ez * dz).clamp(-reach, reach);
            let s = side(ex * wx + ez * wz);
            (half + dx * t + wx * s, half + dz * t + wz * s)
        };
        let (x, z) = world(a, b);
        out[0] = (x, y, z);
        return (out, 1);
    }
    // A body: a square's nearest stretch, a half's or a triangle's middle.
    let (a, b) = match h.kind {
        HULL_HALF => crate::build::anchor(0, 0, h.loc),
        HULL_TRI => (half, BUILD_CELL_M * 0.866_025_4 / 3.0),
        _ => (inset(lx), inset(lz)),
    };
    if h.kind == HULL_RISER {
        // A flight: over and under its surface, at its middle if not over
        // it there, at the stairwell's middle where it has none (a
        // spiral's open core).
        let at =
            |a: f32, b: f32| crate::circulation::surface(h.shape, h.loc, a, b).map(|y| (a, b, y));
        let Some((a, b, dy)) = at(a, b).or_else(|| at(half, half)) else {
            let (x, z) = world(half, half);
            out[0] = (x, h.floor_y + LEVEL_H_M * 0.5, z);
            return (out, 1);
        };
        let (x, z) = world(a, b);
        out[0] = (x, h.floor_y + dy + LIFT_M, z);
        if h.shape == SHAPE_FOUNDATION_STEPS {
            return (out, 1);
        }
        out[1] = (x, h.floor_y + dy - collide::PLANE_THICKNESS_M - LIFT_M, z);
        return (out, 2);
    }
    let (x, z) = world(a, b);
    out[0] = (x, h.floor_y + LIFT_M, z);
    if h.level != 0 {
        out[1] = (x, h.floor_y - collide::PLANE_THICKNESS_M - LIFT_M, z);
        return (out, 2);
    }
    // A foundation is solid to the ground: its sides are faces too, the
    // ones toward an eye standing off it (a raised base's footing from the
    // field). A neighbour's foundation against a side fills it, and the
    // point lands in that solid.
    let mut n = 1;
    let (pts, k) = h.footprint();
    if k < 3 {
        return (out, n);
    }
    let (mut cx, mut cz) = (0.0, 0.0);
    for &(px, pz) in &pts[..k] {
        cx += px / k as f32;
        cz += pz / k as f32;
    }
    for i in 0..k {
        let (p0, p1) = (pts[i], pts[(i + 1) % k]);
        let (ex, ez) = (p1.0 - p0.0, p1.1 - p0.1);
        let len = (ex * ex + ez * ez).sqrt();
        if len <= 2.0 * DOOR_SIDE_M || n == PIECE_FACES {
            continue;
        }
        let (ux, uz) = (ex / len, ez / len);
        // Outward: away from the middle.
        let (mut nx, mut nz) = (uz, -ux);
        if (cx - p0.0) * nx + (cz - p0.1) * nz > 0.0 {
            (nx, nz) = (-nx, -nz);
        }
        if (eye.0 - p0.0) * nx + (eye.2 - p0.1) * nz <= 0.0 {
            continue;
        }
        let t = ((eye.0 - p0.0) * ux + (eye.2 - p0.1) * uz).clamp(DOOR_SIDE_M, len - DOOR_SIDE_M);
        out[n] = (
            p0.0 + ux * t + nx * DOOR_SIDE_M,
            h.floor_y - LIFT_M,
            p0.1 + uz * t + nz * DOOR_SIDE_M,
        );
        n += 1;
    }
    (out, n)
}

/// Whether any face of the built piece at `addr` is in plain sight of
/// `eye` ([`piece_faces`], [`sight`]). No storey rule: a ceiling is
/// reached from the room under it and a roof from the field below. True
/// when nothing stands there: the verb's own refusal names that.
pub fn clear_piece(
    seed: u64,
    haven: &Haven,
    pieces: &crate::build::Pieces,
    doors: u32,
    addr: (u16, u16, u8, u8),
    eye: (f32, f32, f32),
) -> bool {
    let Some(h) = piece_hull(pieces, addr) else {
        return true;
    };
    let (faces, n) = piece_faces(h, eye);
    faces[..n]
        .iter()
        .any(|&f| sight(seed, haven, pieces.cols(), doors, eye, f))
}

/// The first of `cand` — `(key, index)` pairs, smallest key first and ties to
/// the lower index — that `clear` passes, asking nearest-first and at most
/// [`HAND_SIGHT_TESTS_MAX`] times. Reorders `cand`. The stores' nearest-thing
/// searches run through this, so the sight walk is paid for the candidates
/// that would win and no more.
pub fn nearest_clear<K: PartialOrd + Copy>(
    cand: &mut [(K, u16)],
    mut clear: impl FnMut(usize) -> bool,
) -> Option<usize> {
    let mut n = cand.len();
    for _ in 0..HAND_SIGHT_TESTS_MAX {
        if n == 0 {
            return None;
        }
        let mut m = 0;
        for j in 1..n {
            let (a, b) = (cand[j], cand[m]);
            let first = match a.0.partial_cmp(&b.0) {
                Some(core::cmp::Ordering::Less) => true,
                Some(core::cmp::Ordering::Equal) => a.1 < b.1,
                _ => false,
            };
            if first {
                m = j;
            }
        }
        let i = cand[m].1 as usize;
        if clear(i) {
            return Some(i);
        }
        n -= 1;
        cand.swap(m, n);
    }
    None
}
