//! Bounded walk surfaces for the building catalogue. Simulation and meshes
//! read the same planar patches; turns are exact axis swaps, with no trig.
use crate::build::*;

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub x0: f32,
    pub z0: f32,
    pub x1: f32,
    pub z1: f32,
    /// Height at the low corner, then rise per metre along x and z.
    pub y: f32,
    pub sx: f32,
    pub sz: f32,
}
impl Patch {
    pub const EMPTY: Self = Self {
        x0: 0.0,
        z0: 0.0,
        x1: 0.0,
        z1: 0.0,
        y: 0.0,
        sx: 0.0,
        sz: 0.0,
    };
    pub fn height(self, x: f32, z: f32) -> f32 {
        self.y + (x - self.x0) * self.sx + (z - self.z0) * self.sz
    }
    pub fn contains(self, x: f32, z: f32) -> bool {
        x >= self.x0 && x <= self.x1 && z >= self.z0 && z <= self.z1
    }
}
/// Five patches cover the most articulated flight: two landings and three runs.
pub const MAX_PATCHES: usize = 5;

pub fn is_riser(shape: u8) -> bool {
    matches!(
        shape,
        SHAPE_STAIRS
            | SHAPE_FOUNDATION_STEPS
            | SHAPE_RAMP
            | SHAPE_STAIRS_L
            | SHAPE_STAIRS_U
            | SHAPE_STAIRS_SPIRAL
            | SHAPE_STAIRS_TRI_SPIRAL
    )
}
pub fn rise(shape: u8) -> f32 {
    match shape {
        SHAPE_STAIRS | SHAPE_STAIRS_L | SHAPE_STAIRS_U => LEVEL_H_M,
        _ => LEVEL_H_M * 0.5,
    }
}
/// Default flight orientation ascends toward +Z; each other socket turns it.
pub fn local(loc: u8, x: f32, z: f32) -> (f32, f32) {
    match loc {
        LOC_RISER_XHI => (BUILD_CELL_M - z, x),
        LOC_RISER_ZLO => (BUILD_CELL_M - x, BUILD_CELL_M - z),
        LOC_RISER_XLO => (z, BUILD_CELL_M - x),
        _ => (x, z),
    }
}
/// Rectangle dimensions derive from thirds of the existing cell pitch.
pub fn patches(shape: u8) -> ([Patch; MAX_PATCHES], usize) {
    let w = BUILD_CELL_M / 3.0;
    let h = LEVEL_H_M * 0.5;
    let p = |x0: f32, z0: f32, x1: f32, z1: f32, y: f32, sx: f32, sz: f32| Patch {
        x0: x0 * w,
        z0: z0 * w,
        x1: x1 * w,
        z1: z1 * w,
        y: y * h,
        sx: sx * h / w,
        sz: sz * h / w,
    };
    let e = Patch::EMPTY;
    match shape {
        SHAPE_STAIRS_L => (
            [
                p(0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.5),
                p(0.0, 2.0, 1.0, 3.0, 1.0, 0.0, 0.0),
                p(1.0, 2.0, 3.0, 3.0, 1.0, 0.5, 0.0),
                e,
                e,
            ],
            3,
        ),
        SHAPE_STAIRS_U => (
            [
                p(0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.5),
                p(0.0, 2.0, 3.0, 3.0, 1.0, 0.0, 0.0),
                p(2.0, 0.0, 3.0, 2.0, 2.0, 0.0, -0.5),
                e,
                e,
            ],
            3,
        ),
        SHAPE_STAIRS_SPIRAL => (
            [
                p(0.0, 1.5, 1.0, 2.0, 0.0, 0.0, 2.0 / 3.0),
                p(0.0, 2.0, 1.0, 3.0, 1.0 / 3.0, 0.0, 0.0),
                p(1.0, 2.0, 2.0, 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.0),
                p(2.0, 2.0, 3.0, 3.0, 2.0 / 3.0, 0.0, 0.0),
                p(2.0, 1.5, 3.0, 2.0, 1.0, 0.0, -2.0 / 3.0),
            ],
            5,
        ),
        SHAPE_STAIRS_TRI_SPIRAL => (
            [
                p(0.0, 2.0, 1.0, 3.0, 0.0, 0.0, 0.0),
                p(0.0, 1.0, 1.0, 2.0, 0.5, 0.0, -0.5),
                p(0.0, 0.0, 1.0, 1.0, 0.5, 0.0, 0.0),
                p(1.0, 0.0, 2.0, 1.0, 0.5, 0.5, 0.0),
                p(2.0, 0.0, 3.0, 1.0, 1.0, 0.0, 0.0),
            ],
            5,
        ),
        _ => {
            let bottom = if shape == SHAPE_FOUNDATION_STEPS {
                -h
            } else {
                0.0
            };
            (
                [
                    Patch {
                        x0: 0.0,
                        z0: 0.0,
                        x1: BUILD_CELL_M,
                        z1: BUILD_CELL_M,
                        y: bottom,
                        sx: 0.0,
                        sz: rise(shape) / BUILD_CELL_M,
                    },
                    e,
                    e,
                    e,
                    e,
                ],
                1,
            )
        }
    }
}
pub fn surface(shape: u8, loc: u8, x: f32, z: f32) -> Option<f32> {
    let (x, z) = local(loc, x, z);
    if shape == SHAPE_STAIRS_TRI_SPIRAL && x + z > BUILD_CELL_M {
        return None;
    }
    let (patches, n) = patches(shape);
    let mut best = None;
    for p in &patches[..n] {
        if p.contains(x, z) {
            let y = p.height(x, z);
            best = Some(best.map_or(y, |old: f32| old.max(y)));
        }
    }
    best
}
