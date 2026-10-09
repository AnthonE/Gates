//! Power poles along the coast ring: one at every ring node, about 37 m
//! apart, standing just off the shoulder with the line slung between them —
//! the reference game's roads are strung with them, and a road with wires
//! along it reads as a road somebody built at a glance from a kilometre off.
//!
//! **Solved once per seed, into `Haven`**, after the ring is built: a pole
//! stands on the seaward side unless that ground is wet, steep or owned
//! (a site, a side road or trail, a landmark), then the landward side, then
//! nowhere. The line runs between neighbours that both stand, so a gap at a
//! site is a gap in the wires too.
//!
//! **A pole stops a body**: a thin post, answered by bearing like the ring
//! itself (`terrain::bearing_index`), so a query tests three poles at most.
//! It is a column of unbounded height — at 9 m nobody stands above one.

use crate::terrain::{self, Haven, RING_BEARINGS};

/// How far past the shoulder's edge a pole stands, metres.
pub const POLE_OFF_M: f32 = 2.5;
/// A pole's radius, for collision, metres.
pub const POLE_R_M: f32 = 0.16;
/// How tall a pole stands above its ground, metres.
pub const POLE_H_M: f32 = 9.0;
/// How far other things keep from a pole: trees, rocks, junk.
pub const POLE_CLEAR_M: f32 = 2.5;

/// Which side of the ring each node's pole stands on: +1 seaward, −1
/// landward, 0 none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Poles {
    pub side: [i8; RING_BEARINGS],
}

impl Poles {
    pub const NONE: Poles = Poles {
        side: [0; RING_BEARINGS],
    };
}

/// Where pole `k` stands on side `side`, before asking whether it may.
fn spot(haven: &Haven, k: usize, side: i8) -> (f32, f32) {
    let (nx, nz) = haven.ring.node(k as i32);
    let (dx, dz) = crate::yaw_lut::yaw_dir((k as u16) << 8);
    let off = haven.ring.widths(nx, nz).1 + POLE_OFF_M;
    let s = side as f32;
    (nx + dx * off * s, nz + dz * off * s)
}

/// Solve the poles for a pad whose ring, sites, roads and landmarks are all
/// in place.
pub fn solve(seed: u64, pad: &Haven) -> Poles {
    let mut out = Poles::NONE;
    let mut lat = terrain::Lattice::new();
    for k in 0..RING_BEARINGS {
        for side in [1i8, -1] {
            let (x, z) = spot(pad, k, side);
            if may_stand(seed, pad, &mut lat, x, z) {
                out.side[k] = side;
                break;
            }
        }
    }
    out
}

fn may_stand(seed: u64, pad: &Haven, lat: &mut terrain::Lattice, x: f32, z: f32) -> bool {
    let y = terrain::ground_memo(lat, seed, pad, x, z);
    if y < terrain::LAND_MIN_H + 0.5 {
        return false;
    }
    if terrain::ground_slope_memo(lat, seed, pad, x, z) > terrain::CLIFF_SLOPE_RATIO * 0.6 {
        return false;
    }
    if terrain::in_haven(pad, x, z) || terrain::site_sweep(pad, x, z) > 0.0 {
        return false;
    }
    if crate::town::covers(&pad.town, x, z, 4.0)
        || crate::monument::covers(&pad.ziggurat, x, z, 4.0)
    {
        return false;
    }
    if crate::landmark::covers(&pad.marks, x, z, 4.0) {
        return false;
    }
    let clear = terrain::SIDE_ROAD_HALF_W + 4.0;
    for r in pad.roads.iter().chain(pad.trails.iter()) {
        if r.live && r.dist2(x, z) < clear * clear {
            return false;
        }
    }
    true
}

/// Pole `k`'s position, if one stands there.
pub fn at(haven: &Haven, k: usize) -> Option<(f32, f32)> {
    let side = haven.poles.side[k % RING_BEARINGS];
    (side != 0).then(|| spot(haven, k % RING_BEARINGS, side))
}

/// Whether a disc of radius `r` at (x, z) reaches a pole.
pub fn blocks(haven: &Haven, x: f32, z: f32, r: f32) -> bool {
    let c = terrain::ISLAND_SIZE * 0.5;
    let (dx, dz) = (x - c, z - c);
    let d2 = dx * dx + dz * dz;
    // Off the ring's radial bracket nothing is near a pole: one compare
    // before any bearing is resolved.
    let lo = terrain::ROAD_R_MIN - terrain::ROAD_SHOULDER_HALF_W - POLE_OFF_M - r - 1.0;
    let hi = terrain::ROAD_R_MAX + terrain::ROAD_SHOULDER_HALF_W + POLE_OFF_M + r + 1.0;
    if d2 < lo * lo || d2 > hi * hi {
        return false;
    }
    let k = terrain::bearing_index(dx, dz) as i32;
    let reach = r + POLE_R_M;
    let mut j = k - 1;
    while j <= k + 1 {
        if let Some((px, pz)) = at(haven, j.rem_euclid(RING_BEARINGS as i32) as usize) {
            let (ex, ez) = (x - px, z - pz);
            if ex * ex + ez * ez < reach * reach {
                return true;
            }
        }
        j += 1;
    }
    false
}
