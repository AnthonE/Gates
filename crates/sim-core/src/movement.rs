//! Kinematic capsule vs terrain and placed pieces (TERRAIN.md §3), shared
//! verbatim by server and wasm prediction. Quantize-both-sides: positions
//! and vertical velocity live as integer quanta; each step decodes,
//! integrates with walled float ops, and re-quantizes — the sim runs on
//! the values it transmits (NETCODE.md §3), so prediction can never drift
//! by rounding. Piece collision comes in through `cols` (collide.rs): the
//! server passes its piece store's index, the predictor the client
//! mirror's — same code, same bits.
//!
//! Movement constants are proposed defaults registered in DECISIONS.md
//! §open (sim movement constants row); capsule and piece-slab dimensions
//! in the piece collision v0 row.

use crate::collide::{self, ColIndex};
use crate::fmath::floor_i32;
use crate::input::{InputFrame, BTN_SPRINT};
use crate::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE, SEA_LEVEL};
use crate::yaw_lut::yaw_dir;

/// Position quantum, x/z: 3 cm (DESIGN.md §5.5).
pub const POS_XZ_Q: f32 = 0.03;
/// Position quantum, y: 1 cm (DESIGN.md §5.5).
pub const POS_Y_Q: f32 = 0.01;
/// Velocity quantum: 1 cm/s (DECISIONS.md §open).
pub const VEL_Q: f32 = 0.01;

/// Fixed tick delta — the only dt the sim knows (30 Hz).
pub const DT: f32 = 1.0 / 30.0;

pub const WALK_SPEED: f32 = 3.0;
pub const SPRINT_SPEED: f32 = 5.5;
pub const GRAVITY: f32 = 20.0;
pub const TERMINAL_VELOCITY: f32 = 50.0;
pub const STEP_UP: f32 = 0.6;
pub const WADE_SPEED_MULT: f32 = 0.5;
/// Ground at or below this height counts as wading (alpha swim = slow
/// wade, TERRAIN.md §3).
pub const WADE_GROUND_MAX: f32 = SEA_LEVEL + 0.4;
/// Hard world border: the sea ring beyond the island is not walkable v1.
pub const BORDER_MARGIN: f32 = 8.0;

#[inline]
pub fn quant_xz(m: f32) -> i32 {
    floor_i32(m / POS_XZ_Q + 0.5)
}
#[inline]
pub fn quant_y(m: f32) -> i32 {
    floor_i32(m / POS_Y_Q + 0.5)
}
#[inline]
pub fn quant_vel(v: f32) -> i32 {
    floor_i32(v / VEL_Q + 0.5)
}

/// The capsule's mutable state, all quantized. Lives inside `Player`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Body {
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    pub qvy: i32,
    pub grounded: bool,
}

impl Body {
    pub fn at(seed: u64, x: f32, z: f32) -> Self {
        let y = terrain::height(seed, x, z);
        Self {
            qx: quant_xz(x),
            qy: quant_y(y),
            qz: quant_xz(z),
            qvy: 0,
            grounded: true,
        }
    }
}

/// Can the capsule move from height `from_y` onto ground at `to_ground`,
/// covering planar distance `run`? Step-up bounds any single rise; the
/// cliff ratio keeps >50° terrain unwalkable (TERRAIN.md §3).
#[inline]
fn climbable(from_y: f32, to_ground: f32, run: f32) -> bool {
    let rise = to_ground - from_y;
    rise <= STEP_UP && rise <= run * CLIFF_SLOPE_RATIO
}

/// One fixed-timestep step of one capsule. Pure: same inputs, same result,
/// native or wasm. `cols` is the placed-piece collision index — the
/// server's store and the client's mirror step through this same code.
pub fn step(seed: u64, cols: &ColIndex, body: &mut Body, frame: &InputFrame) {
    let x = body.qx as f32 * POS_XZ_Q;
    let y = body.qy as f32 * POS_Y_Q;
    let z = body.qz as f32 * POS_XZ_Q;
    let mut vy = body.qvy as f32 * VEL_Q;

    // Wish direction: move vec rotated by the yaw LUT, magnitude clamped.
    let (fx, fz) = yaw_dir(frame.yaw);
    let (rx, rz) = (fz, -fx);
    let mf = frame.move_z as f32 * (1.0 / 127.0);
    let ms = frame.move_x as f32 * (1.0 / 127.0);
    let mut wx = fx * mf + rx * ms;
    let mut wz = fz * mf + rz * ms;
    let len2 = wx * wx + wz * wz;
    if len2 > 1.0 {
        let inv = 1.0 / len2.sqrt();
        wx *= inv;
        wz *= inv;
    }

    // Wade on the effective ground (a floor over shallows stays dry).
    let ground_here = terrain::height(seed, x, z).max(collide::piece_ground(seed, cols, x, z, y));
    let mut speed = if frame.buttons & BTN_SPRINT != 0 {
        SPRINT_SPEED
    } else {
        WALK_SPEED
    };
    if ground_here <= WADE_GROUND_MAX {
        speed *= WADE_SPEED_MULT;
    }

    // Horizontal, axis-resolved: full move, then x-only, then z-only.
    // Walls veto a candidate outright; a built surface accepts on the
    // step rule alone (a stair-step, not a cliff — the ramp and slab
    // rises are bounded by design), terrain keeps the cliff ratio.
    let dx = wx * speed * DT;
    let dz = wz * speed * DT;
    let mut nx = x;
    let mut nz = z;
    if len2 > 0.0 {
        let clamp_x = |v: f32| v.clamp(BORDER_MARGIN, ISLAND_SIZE - BORDER_MARGIN);
        let candidates = [(x + dx, z + dz), (x + dx, z), (x, z + dz)];
        for (cx, cz) in candidates {
            let cx = clamp_x(cx);
            let cz = clamp_x(cz);
            let run2 = (cx - x) * (cx - x) + (cz - z) * (cz - z);
            if run2 <= 0.0 {
                continue;
            }
            if collide::blocked(seed, cols, x, z, cx, cz, y) {
                continue;
            }
            let g = terrain::height(seed, cx, cz);
            let pg = collide::piece_ground(seed, cols, cx, cz, y);
            let ok = if pg > g {
                pg - y <= STEP_UP
            } else {
                climbable(y, g, run2.sqrt())
            };
            if ok {
                nx = cx;
                nz = cz;
                break;
            }
        }
    }

    // Vertical: snap within step range, otherwise fall. Built surfaces
    // count as ground only when the step rule already admitted them
    // (piece_ground's lid), so a floor overhead is a ceiling, not a lift.
    let ground = terrain::height(seed, nx, nz).max(collide::piece_ground(seed, cols, nx, nz, y));
    let mut ny = y;
    if body.grounded && ground - y <= STEP_UP && y - ground <= STEP_UP {
        ny = ground;
        vy = 0.0;
    } else {
        vy = (vy - GRAVITY * DT).max(-TERMINAL_VELOCITY);
        ny += vy * DT;
        if ny <= ground {
            ny = ground;
            vy = 0.0;
        }
    }
    body.grounded = ny <= ground + 0.001;

    body.qx = quant_xz(nx);
    body.qy = quant_y(ny);
    body.qz = quant_xz(nz);
    body.qvy = quant_vel(vy);
}
