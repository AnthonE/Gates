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
//! - **the same storey**: within [`LOOT_REACH_DY_M`] of the player's feet,
//!   the world crate's rule (`worldcont.rs`) made everyone's;
//! - **a clear line**: nothing built between the eye and the thing, the shot
//!   walk ([`collide::shot_blocked`]) stepped the way a shot steps it.
//!
//! Pure and bounded: a fixed sample cap, no allocation, `+ − × ÷ sqrt` only.

use crate::collide::{self, ColIndex};
use crate::deploy::{DeployContent, Deploys};
use crate::limits::ARROW_STEP_MM;
use crate::movement::{POS_XZ_Q, POS_Y_Q};
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
/// and frame rims, floors and roofs. Pieces only — the thing reached for,
/// other furniture, terrain and bodies never block a hand.
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
    let mut prev = (from.0, from.2);
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let (x, y, z) = (from.0 + dx * t, from.1 + dy * t, from.2 + dz * t);
        if collide::shot_blocked(seed, haven, cols, prev.0, prev.1, x, z, y, ARROW_R_M) {
            return false;
        }
        prev = (x, z);
    }
    true
}

/// Where a hand aims at a thing: its `(x, z)`, the floor it stands or lies
/// on, and how far above that floor its middle is.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub x: f32,
    pub z: f32,
    pub base_y: f32,
    pub mid: f32,
}

impl Target {
    /// A thing lying where its quantized position says — a bag, a loose
    /// stack, a sleeper — aimed [`LIFT_M`] above where it rests.
    pub fn lying(q: (i32, i32, i32)) -> Self {
        Self {
            x: q.0 as f32 * POS_XZ_Q,
            z: q.2 as f32 * POS_XZ_Q,
            base_y: q.1 as f32 * POS_Y_Q,
            mid: LIFT_M,
        }
    }

    /// A deployable where it stands now (`deploy::body_base_y`, the floor
    /// the renderer and the collision walks stand it on), aimed at the
    /// middle of its body.
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
        }
    }
}

/// Whether a hand with its eye at `eye` and its feet at `feet_y` reaches
/// `t`: within a storey of the feet and in plain sight of the eye. The planar
/// distance is the caller's. The client asks this from its predicted body to
/// offer `E` only where the shard would take it.
pub fn clear_from(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    eye: (f32, f32, f32),
    feet_y: f32,
    t: Target,
) -> bool {
    if crate::fmath::fabs(feet_y - t.base_y) > LOOT_REACH_DY_M {
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
