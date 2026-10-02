//! Where a free-placed deployable stands: its centre inside its build cell,
//! the way it faces, and the rectangle of ground it covers.
//!
//! A body deployable (a bag, a box, a fire, a bench — anything that is not
//! an edge insert) is placed where the crosshair puts it, the way the
//! reference game places one, not at its cell's centre. Its record keeps the
//! grid address it always had — the address is still its identity, its box
//! handle and its wire name — plus a [`Pose`]: an offset from the cell centre
//! and a facing. `Pose::default()` is the cell centre facing +Z, which is
//! exactly where every deployable stood before free placement, so an old
//! record, an edge insert and a town station all mean what they meant.
//!
//! Everything here is pure arithmetic in wall 1's vocabulary (+ − × ÷, min,
//! max, the shared yaw table) because the server's placement verdict, its
//! collision walks, the client's predictor and the client's renderer all
//! read a deployable through these functions and must agree to the bit.

use crate::build::BUILD_CELL_M;
use crate::fmath::{fabs, floor_i32};
use crate::limits::MAX_BUILD_COORD;

/// One step of a pose offset: 1/256 of a cell, 11.7 mm. `3 × 2⁻⁸` is exact
/// in f32, so every deployable centre is an exact multiple of 2⁻⁸ m and two
/// machines computing one get the same bits.
pub const POSE_Q_M: f32 = BUILD_CELL_M / 256.0;

/// How far a free-placed deployable sits from its cell's centre, and which
/// way it faces. `ox`/`oz` are [`POSE_Q_M`] steps (−128..=127 spans the
/// whole cell); `yaw` is a 256th of a turn on the shared yaw table's
/// convention — 0 faces +Z, increasing turns toward +X — so the model's
/// front (authored toward +Z) faces along `yaw_sc(yaw)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pose {
    pub ox: i8,
    pub oz: i8,
    pub yaw: u8,
}

impl Pose {
    /// The cell centre, facing +Z: where every deployable stood before free
    /// placement, and still where an edge insert's record says it is.
    pub const CENTRE: Pose = Pose {
        ox: 0,
        oz: 0,
        yaw: 0,
    };
}

/// A deployable's rectangle on the ground, `[w, d]` metres: `w` along its
/// local X, `d` along its local Z (its front). The rows that block are
/// `deploy::DEPLOY_VOL`'s own footprint (a const block in `deploy.rs` holds
/// the two tables equal); the bag and the fire, which a body walks over,
/// still cover ground nothing else may be placed on. Zero rows are not
/// free-placed: the door and the window inserts live in their edge, and a
/// lock is never a record.
pub const DEPLOY_FOOT: [[f32; 2]; 16] = [
    [1.9, 0.8],  // 0 bag — a bedroll a body is longer than
    [1.2, 0.6],  // 1 hearth
    [1.2, 0.7],  // 2 box
    [0.7, 0.7],  // 3 fire
    [1.3, 0.85], // 4 furnace
    [1.6, 0.7],  // 5 workbench
    [0.0, 0.0],  // 6 door — an edge insert
    [0.0, 0.0],  // 7 lock — never a record
    [1.3, 0.9],  // 8 recycler
    [1.5, 0.8],  // 9 research table
    [1.6, 0.8],  // 10 workbench 2
    [1.8, 0.8],  // 11 workbench 3
    [0.0, 0.0],  // 12 window bars — edge insert
    [0.0, 0.0],  // 13 garage door — edge insert
    [0.0, 0.0],  // 14 glass — edge insert
    [0.0, 0.0],  // 15 shutters — edge insert
];

/// The farthest any footprint corner stands from its centre, metres: how far
/// a scan has to look to find every rectangle that could touch a point.
pub const FOOT_REACH_M: f32 = 1.05;

/// The same bound over the rows that block (`deploy::solid_vol`) — the
/// reach the collision walks look across. Asserted below, and kept under
/// the cell so a capsule never has to look further than one neighbour.
pub const SOLID_REACH_M: f32 = 1.0;

const _: () = {
    let mut i = 0;
    while i < DEPLOY_FOOT.len() {
        let hw = DEPLOY_FOOT[i][0] * 0.5;
        let hd = DEPLOY_FOOT[i][1] * 0.5;
        assert!(hw * hw + hd * hd <= FOOT_REACH_M * FOOT_REACH_M);
        i += 1;
    }
    assert!(SOLID_REACH_M + crate::collide::CAPSULE_RADIUS_M < BUILD_CELL_M);
};

/// Half extents `(hw, hd)` of a free-placed archetype's rectangle, or `None`
/// for one that is not free-placed (an edge insert, the lock).
#[inline]
pub fn half_foot(arch: u8) -> Option<(f32, f32)> {
    let [w, d] = *DEPLOY_FOOT.get(arch as usize)?;
    if w <= 0.0 {
        return None;
    }
    Some((w * 0.5, d * 0.5))
}

/// `(sin, cos)` of a pose's facing, off the shared yaw table — the unit
/// vector the deployable's local +Z points along.
#[inline]
pub fn yaw_sc(yaw: u8) -> (f32, f32) {
    crate::yaw_lut::yaw_dir((yaw as u16) << 8)
}

/// The world XZ of a pose's centre in cell `(cx, cz)`.
#[inline]
pub fn centre(cx: u16, cz: u16, pose: Pose) -> (f32, f32) {
    (
        cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5 + pose.ox as f32 * POSE_Q_M,
        cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5 + pose.oz as f32 * POSE_Q_M,
    )
}

/// The cell and the offset that name a world point, to the pose's own
/// resolution: the inverse of [`centre`]. `None` off the build grid. The
/// client's ghost and the agents address a placement through this, so a
/// point and the record it becomes are one rounding apart and never two.
pub fn quantize(x: f32, z: f32) -> Option<(u16, u16, i8, i8)> {
    let axis = |v: f32| -> Option<(u16, i8)> {
        let q = floor_i32(v / POSE_Q_M + 0.5);
        let cell = q.div_euclid(256);
        if cell < 0 || cell >= MAX_BUILD_COORD as i32 {
            return None;
        }
        Some((cell as u16, (q - cell * 256 - 128) as i8))
    };
    let (cx, ox) = axis(x)?;
    let (cz, oz) = axis(z)?;
    Some((cx, cz, ox, oz))
}

/// A rectangle on the ground, possibly turned: centre, half extents along
/// its own axes, and its facing as `(s, c)` — local +Z is `(s, c)` in the
/// world and local +X is `(c, −s)`, which is `Quat::from_rotation_y`'s
/// rotation, so the renderer's model and the sim's rectangle turn the same
/// way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub z: f32,
    pub hw: f32,
    pub hd: f32,
    pub s: f32,
    pub c: f32,
}

/// Rectangles closer than this along a separating axis count as apart, so
/// two boxes pushed flush against each other (or a box against a wall) is a
/// legal placement rather than a rounding argument.
pub const TOUCH_M: f32 = 0.001;

/// How far inside its corners a rectangle's support is sampled, so a corner
/// lying exactly on a cell boundary is read from the cell it is in rather
/// than from whichever side the boundary's rounding picks.
pub const SAMPLE_INSET_M: f32 = 0.02;

impl Rect {
    pub fn new(x: f32, z: f32, hw: f32, hd: f32, yaw: u8) -> Self {
        let (s, c) = yaw_sc(yaw);
        Self { x, z, hw, hd, s, c }
    }

    /// An unturned rectangle spanning `[x0, x1] × [z0, z1]`.
    pub fn aligned(x0: f32, z0: f32, x1: f32, z1: f32) -> Self {
        Self {
            x: (x0 + x1) * 0.5,
            z: (z0 + z1) * 0.5,
            hw: (x1 - x0) * 0.5,
            hd: (z1 - z0) * 0.5,
            s: 0.0,
            c: 1.0,
        }
    }

    /// A world point in this rectangle's own frame.
    #[inline]
    pub fn local(&self, x: f32, z: f32) -> (f32, f32) {
        let (dx, dz) = (x - self.x, z - self.z);
        (dx * self.c - dz * self.s, dx * self.s + dz * self.c)
    }

    /// A point of this rectangle's own frame in the world.
    #[inline]
    pub fn world(&self, lx: f32, lz: f32) -> (f32, f32) {
        (
            self.x + lx * self.c + lz * self.s,
            self.z - lx * self.s + lz * self.c,
        )
    }

    /// Whether the point is on the rectangle, edges included.
    #[inline]
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (lx, lz) = self.local(x, z);
        fabs(lx) <= self.hw && fabs(lz) <= self.hd
    }

    /// Squared distance from the point to the rectangle — zero on it. The
    /// clamp-to-rectangle distance the capsule walks use, in the
    /// rectangle's own frame so a turned box rounds its corners correctly.
    #[inline]
    pub fn dist2(&self, x: f32, z: f32) -> f32 {
        let (lx, lz) = self.local(x, z);
        let ex = (fabs(lx) - self.hw).max(0.0);
        let ez = (fabs(lz) - self.hd).max(0.0);
        ex * ex + ez * ez
    }

    /// The world-axis half extents of the rectangle's bounding box.
    #[inline]
    pub fn half_aabb(&self) -> (f32, f32) {
        (
            self.hw * fabs(self.c) + self.hd * fabs(self.s),
            self.hw * fabs(self.s) + self.hd * fabs(self.c),
        )
    }

    /// The centre and the four corners pulled [`SAMPLE_INSET_M`] inward —
    /// the points support is read at.
    pub fn samples(&self) -> [(f32, f32); 5] {
        let hw = (self.hw - SAMPLE_INSET_M).max(0.0);
        let hd = (self.hd - SAMPLE_INSET_M).max(0.0);
        [
            (self.x, self.z),
            self.world(-hw, -hd),
            self.world(hw, -hd),
            self.world(-hw, hd),
            self.world(hw, hd),
        ]
    }

    /// Whether two rectangles share area: the separating-axis test over
    /// both rectangles' edge normals. Touching ([`TOUCH_M`]) is apart.
    pub fn overlaps(&self, o: &Rect) -> bool {
        let (dx, dz) = (o.x - self.x, o.z - self.z);
        let axes = [(self.c, -self.s), (self.s, self.c), (o.c, -o.s), (o.s, o.c)];
        for (nx, nz) in axes {
            let dist = fabs(dx * nx + dz * nz);
            let ra = self.hw * fabs(self.c * nx - self.s * nz)
                + self.hd * fabs(self.s * nx + self.c * nz);
            let rb = o.hw * fabs(o.c * nx - o.s * nz) + o.hd * fabs(o.s * nx + o.c * nz);
            if dist >= ra + rb - TOUCH_M {
                return false;
            }
        }
        true
    }
}

/// The build cells a rectangle's bounding box touches, grown by `margin`:
/// `(x0, z0, x1, z1)` inclusive, clamped to the grid. A rectangle is under
/// 2.1 m across, so this is at most three cells a side.
pub fn cells_under(r: &Rect, margin: f32) -> (u16, u16, u16, u16) {
    let (ex, ez) = r.half_aabb();
    let max = MAX_BUILD_COORD as i32 - 1;
    let cell = |v: f32| floor_i32(v / BUILD_CELL_M).clamp(0, max) as u16;
    (
        cell(r.x - ex - margin),
        cell(r.z - ez - margin),
        cell(r.x + ex + margin),
        cell(r.z + ez + margin),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_pose_is_the_cell_centre() {
        assert_eq!(centre(4, 7, Pose::CENTRE), (13.5, 22.5));
    }

    #[test]
    fn quantize_inverts_centre_across_the_whole_cell() {
        for (cx, cz) in [(0u16, 0u16), (5, 9), (2047, 1)] {
            for o in [-128i8, -1, 0, 1, 77, 127] {
                let pose = Pose {
                    ox: o,
                    oz: o,
                    yaw: 0,
                };
                let (x, z) = centre(cx, cz, pose);
                assert_eq!(quantize(x, z), Some((cx, cz, pose.ox, pose.oz)));
            }
        }
        // A point just short of the next cell's low edge rounds into it
        // rather than overflowing this cell's offset.
        let x = 3.0 * 5.0 + 2.999;
        let (cx, _, ox, _) = quantize(x, 1.0).unwrap();
        assert_eq!((cx, ox), (6, -128));
        assert_eq!(quantize(-5.0, 1.0), None);
    }

    #[test]
    fn a_quarter_turn_swaps_the_axes() {
        let r = Rect::new(10.0, 10.0, 1.0, 0.25, 64);
        // Local +X now points along world −Z.
        assert!(r.contains(10.2, 9.1));
        assert!(!r.contains(10.9, 10.0));
        let (ex, ez) = r.half_aabb();
        assert!(fabs(ex - 0.25) < 1e-5 && fabs(ez - 1.0) < 1e-5);
    }

    #[test]
    fn flush_rectangles_do_not_overlap_and_crossed_ones_do() {
        let a = Rect::new(10.0, 10.0, 0.6, 0.35, 0);
        let flush = Rect::new(11.2, 10.0, 0.6, 0.35, 0);
        assert!(!a.overlaps(&flush));
        let into = Rect::new(11.1, 10.0, 0.6, 0.35, 0);
        assert!(a.overlaps(&into));
        // Turned 45°, a box near another's corner clears it only when the
        // diagonal gap is real.
        let turned = Rect::new(10.9, 10.3, 0.6, 0.35, 32);
        assert!(a.overlaps(&turned));
        let past_corner = Rect::new(11.3, 10.6, 0.6, 0.35, 32);
        assert!(!a.overlaps(&past_corner));
        let clear = Rect::new(12.2, 11.2, 0.6, 0.35, 32);
        assert!(!a.overlaps(&clear));
    }
}
