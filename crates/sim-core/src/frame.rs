//! A building's own frame: where its lattice (`lattice.rs`) sits in the
//! world and which way it faces (freeform M8, unwired).
//!
//! Every freeform base starts from one free foundation, placed anywhere at
//! any yaw; that placement mints the frame and every piece snapped on after
//! it is an exact lattice point inside it. The only float rotation in a
//! building is this one, computed once from the frame's yaw through
//! `fmath`, so a piece's world position is a pure function of integers.

use crate::fmath;
use crate::lattice::LPt;

/// Centimetres in a metre, for the frame's integer origin. A division,
/// not a multiplication by 0.01: correctly rounded, so an origin on a whole
/// metre — every grid building's — lands on it exactly.
const CM: f32 = 100.0;

/// √3 / 2.
const H3: f32 = 0.866_025_4;

/// (cos, sin) of k × 30°, the lattice's twelve directions in its own
/// metres (x along ζ⁰, z along ζ³).
pub const STEP_CS: [(f32, f32); 12] = [
    (1.0, 0.0),
    (H3, 0.5),
    (0.5, H3),
    (0.0, 1.0),
    (-0.5, H3),
    (-H3, 0.5),
    (-1.0, 0.0),
    (-H3, -0.5),
    (-0.5, -H3),
    (0.0, -1.0),
    (0.5, -H3),
    (H3, -0.5),
];

/// Where a building's lattice stands. `yaw` follows `footprint::Rect` and
/// `yaw_dir`: 0 faces +z, increasing turns toward +x, 65 536 a whole turn,
/// so local +x lands on (c, −s) and local +z on (s, c).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The lattice origin, world centimetres (the island is 409 600 cm a
    /// side, comfortably inside i32).
    pub origin_cm: (i32, i32),
    /// The frame's floor, in `build::BUILD_BASE_Q_M` bands of world height.
    pub band: i32,
    pub yaw: u16,
    s: f32,
    c: f32,
}

impl Frame {
    /// Facing +x from the world origin, on band 0: an unused row.
    pub const ZERO: Frame = Frame {
        origin_cm: (0, 0),
        band: 0,
        yaw: 0,
        s: 0.0,
        c: 1.0,
    };

    pub fn new(origin_cm: (i32, i32), band: i32, yaw: u16) -> Self {
        let a = f32::from(yaw) * (core::f32::consts::TAU / 65536.0);
        let (s, c) = fmath::sin_cos(a);
        Self {
            origin_cm,
            band,
            yaw,
            s,
            c,
        }
    }

    /// The frame's (sin, cos), as every placement in it rotates by.
    pub fn sin_cos(&self) -> (f32, f32) {
        (self.s, self.c)
    }

    /// A lattice point in world metres, (x, z).
    pub fn to_world(&self, p: LPt) -> (f32, f32) {
        let (lx, lz) = p.to_xz();
        let ox = self.origin_cm.0 as f32 / CM;
        let oz = self.origin_cm.1 as f32 / CM;
        (
            ox + lx * self.c + lz * self.s,
            oz - lx * self.s + lz * self.c,
        )
    }

    /// A world point in the frame's own metres — the inverse of
    /// [`Frame::to_world`], for asking which lattice point an aim is near.
    pub fn to_local(&self, x: f32, z: f32) -> (f32, f32) {
        let dx = x - self.origin_cm.0 as f32 / CM;
        let dz = z - self.origin_cm.1 as f32 / CM;
        (dx * self.c - dz * self.s, dx * self.s + dz * self.c)
    }

    /// The world unit vector of lattice direction `k` (ζᵏ).
    pub fn axis(&self, k: u8) -> (f32, f32) {
        let (lx, lz) = STEP_CS[(k % 12) as usize];
        (lx * self.c + lz * self.s, -lx * self.s + lz * self.c)
    }

    /// World height of `y` bands above the frame's floor.
    pub fn y_m(&self, y: i32) -> f32 {
        (self.band + y) as f32 * crate::build::BUILD_BASE_Q_M
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Pcg32;

    #[test]
    fn yaw_zero_is_the_lattice_where_it_stands() {
        let f = Frame::new((150_000, 220_050), 40, 0);
        let p = LPt::new(6, 0, 0, 2);
        let (lx, lz) = p.to_xz();
        let (x, z) = f.to_world(p);
        let (ex, ez) = (1500.0 + lx, 2200.5 + lz);
        assert!(
            (x - ex) * (x - ex) + (z - ez) * (z - ez) < 1e-6,
            "({x}, {z})"
        );
    }

    /// A quarter turn sends the lattice's +z toward the world's +x, as a
    /// deployable's `Rect` turns.
    #[test]
    fn a_quarter_turn_faces_plus_x() {
        let f = Frame::new((0, 0), 0, 16_384);
        let (x, z) = f.to_world(LPt::edge(3));
        assert!((x - 3.0) * (x - 3.0) < 1e-8 && z * z < 1e-8, "({x}, {z})");
        let r = crate::footprint::Rect::new(0.0, 0.0, 1.0, 1.0, 64);
        assert!((r.s - f.s) * (r.s - f.s) < 1e-8 && (r.c - f.c) * (r.c - f.c) < 1e-8);
    }

    /// `axis` is the direction `to_world` moves a point along an edge.
    #[test]
    fn an_axis_is_where_an_edge_goes() {
        for yaw in [0u16, 1, 9_000, 16_384, 40_000, 65_535] {
            let f = Frame::new((10_000, 20_000), 0, yaw);
            let o = f.to_world(LPt::ZERO);
            for k in 0..12 {
                let (x, z) = f.to_world(LPt::edge(k));
                let (ax, az) = f.axis(k);
                let (ex, ez) = ((x - o.0) / 3.0, (z - o.1) / 3.0);
                assert!(
                    crate::fmath::fabs(ax - ex) < 1e-4 && crate::fmath::fabs(az - ez) < 1e-4,
                    "{yaw} {k}"
                );
            }
        }
    }

    #[test]
    fn local_undoes_world() {
        let mut rng = Pcg32::new(0x000F_2A3E, 2);
        for _ in 0..1_000 {
            let f = Frame::new(
                (
                    rng.next_bounded(409_600) as i32,
                    rng.next_bounded(409_600) as i32,
                ),
                0,
                rng.next_bounded(65_536) as u16,
            );
            let p = LPt::new(
                rng.next_bounded(81) as i32 - 40,
                rng.next_bounded(81) as i32 - 40,
                rng.next_bounded(81) as i32 - 40,
                rng.next_bounded(81) as i32 - 40,
            );
            let (wx, wz) = f.to_world(p);
            let (lx, lz) = f.to_local(wx, wz);
            let (ex, ez) = p.to_xz();
            // World coordinates run to 4 km, where an f32 step is ~0.5 mm.
            assert!(
                (lx - ex) * (lx - ex) + (lz - ez) * (lz - ez) < 1e-4,
                "{p:?} in {f:?}"
            );
        }
    }
}
