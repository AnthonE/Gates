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
    (
        p.body.qx as f32 * POS_XZ_Q,
        p.body.qy as f32 * POS_Y_Q + crate::ranged::eye_mm(p.crouched()) as f32 / MM_PER_M,
        p.body.qz as f32 * POS_XZ_Q,
    )
}

/// Nothing built between `from` and `to` (metres): walls, shut doors, window
/// and frame rims, floors and roofs, and the ziggurat's and landmarks' stone.
/// The thing reached for, other furniture, terrain and bodies never block a
/// hand.
pub fn sight(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
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
        // doors read shut: a hand does not reach through a card doorway.
        let feet = y - STEP_UP;
        if crate::monument::blocks(&haven.ziggurat, 0, x, z, feet, ARROW_R_M, STEP_UP)
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

    /// A loose stack: [`Self::stuck`] for an arrow stuck in something,
    /// [`Self::lying`] for everything on the ground.
    pub fn stack(seed: u64, haven: &Haven, cols: &ColIndex, g: &GroundItemRec) -> Self {
        if g.dir == [0; 3] {
            Self::lying(seed, haven, cols, (g.qx, g.qy, g.qz))
        } else {
            Self::stuck((g.qx, g.qy, g.qz), g.dir)
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
}

/// Whether a hand with its eye at `eye` and its feet at `feet_y` reaches
/// `t`: in plain sight of the eye, and within a storey of the feet when `t`
/// stands on a floor. The planar distance is the caller's. The client asks this from its predicted body to
/// offer `E` only where the shard would take it.
pub fn clear_from(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    eye: (f32, f32, f32),
    feet_y: f32,
    t: Target,
) -> bool {
    if t.storey && crate::fmath::fabs(feet_y - t.base_y) > LOOT_REACH_DY_M {
        return false;
    }
    sight(seed, haven, cols, eye, (t.x, t.base_y + t.mid, t.z))
}

/// A player's feet, metres — what [`clear_from`] measures a storey from.
pub fn feet_y(p: &Player) -> f32 {
    p.body.qy as f32 * POS_Y_Q
}

/// [`clear_from`] for a player as the sim has them.
pub fn clear(seed: u64, haven: &Haven, cols: &ColIndex, p: &Player, t: Target) -> bool {
    clear_from(seed, haven, cols, eye(p), feet_y(p), t)
}

/// [`clear_from`] for entry `i` of the box store — a box, an oven, a research
/// table — aimed at the middle of its body. False for an index with no
/// record behind it.
#[allow(clippy::too_many_arguments)]
pub fn clear_box(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
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
    clear_from(seed, haven, cols, eye, feet_y, t)
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
