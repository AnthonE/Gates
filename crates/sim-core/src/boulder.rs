//! Rock formations: the island's big rocks, as a pure function of the seed.
//!
//! **Why this is not a scatter occupant.** The scatter grid holds one
//! occupant per 8 m cell and proves its 3×3 collision probe complete only
//! because nothing reaches past `CELL_SIZE` (`terrain::OCCUPANT_PROBE_CELLS`).
//! A rock a player reads as a landmark is 6–25 m across, so it lives on its
//! own coarse grid — one formation per 64 m cell at most — and answers its
//! own collision query, the way `depot::blocks` answers for the depot yard.
//! `reference/ROCKS.md` §9.2 is the plan this follows: formations first,
//! smaller rocks clustered around them, the scatter kept off their footprint.
//!
//! **A formation is a few domes.** Each dome is a sunk half-spheroid —
//! `y + h·√(1 − d²/r²)` over a disc — which is what the collision reads:
//!
//! - its flank is steeper than `CLIFF_SLOPE_RATIO` and is a wall;
//! - its crown is gentler and is ground a body can stand on (a player on a
//!   ridge above one can drop onto it);
//! - a projectile inside it has hit rock.
//!
//! The client draws each dome as a jagged rock mesh wearing the ground's own
//! rock material (`render/boulders.rs`), so a boulder and a cliff are one
//! substance at two scales — `ROCKS.md` §1's reading of the reference game.
//!
//! Every number here is a first cut (operator, 2026-09-28: "bolders and
//! gigantic rocks are small they need to be huge").

use crate::fmath::floor_i32;
use crate::rng::cell_hash;
use crate::terrain::{self, Haven};

/// Edge of one formation cell, metres.
pub const BOULDER_CELL_M: f32 = 64.0;
/// Domes one formation may hold: the main rock and its satellites.
pub const DOMES_PER_CELL: usize = 4;
/// Widest a main dome's radius can be drawn, metres.
pub const DOME_R_MAX: f32 = 13.0;
/// Narrowest a main dome is drawn, metres.
pub const DOME_R_MIN: f32 = 3.2;
/// Narrowest any dome is kept, satellites and beach rocks included, metres.
/// Smaller than this is the scatter's own `Rock`.
pub const DOME_R_FLOOR: f32 = 1.6;
/// How far a formation's domes can reach from its cell's centre, metres:
/// the main dome's centre is kept 12 m inside the cell (so at most the
/// half-diagonal of a 40 m square off centre), and a satellite's far edge is
/// within `2.2 r` of the main centre (`plan`). A query only needs the cells
/// this can reach.
pub const FORMATION_REACH_M: f32 = (BOULDER_CELL_M * 0.5 - 12.0) * 1.4143 + DOME_R_MAX * 2.2;
/// Clearance kept between a dome and a road's centreline, beyond the dome's
/// own radius, metres — the carriageway, its shoulder and the ring's blend.
pub const ROAD_CLEAR_M: f32 = terrain::ROAD_SHOULDER_HALF_W + terrain::RING_BLEND_M + 3.0;
/// Clearance beyond a site's blend radius, metres.
pub const SITE_CLEAR_M: f32 = 20.0;

const CH_BOULDER: u32 = 176;

const _: () = {
    // The query scans the formation cells a reach can touch, 3×3 at most.
    assert!(FORMATION_REACH_M < BOULDER_CELL_M);
    assert!(DOME_R_MIN < DOME_R_MAX);
};

/// One dome: a sunk half-spheroid, `r` across in plan and rising `h` above
/// its base `y` at the centre.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dome {
    pub x: f32,
    pub z: f32,
    /// The spheroid's centre height — below the ground, so the dome is sunk.
    pub y: f32,
    pub r: f32,
    pub h: f32,
    /// Bearing and shape draw for the client's mesh; the sim never reads them.
    pub yaw: u8,
    pub shape: u8,
}

/// One cell's formation. `n` domes, the rest zeroed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formation {
    pub n: u8,
    pub domes: [Dome; DOMES_PER_CELL],
}

impl Formation {
    pub const EMPTY: Formation = Formation {
        n: 0,
        domes: [Dome {
            x: 0.0,
            z: 0.0,
            y: 0.0,
            r: 0.0,
            h: 0.0,
            yaw: 0,
            shape: 0,
        }; DOMES_PER_CELL],
    };

    pub fn iter(&self) -> impl Iterator<Item = &Dome> {
        self.domes[..self.n as usize].iter()
    }
}

/// Formation cells a side.
pub fn cells_per_side() -> i32 {
    (terrain::ISLAND_SIZE / BOULDER_CELL_M) as i32
}

#[inline]
fn unit(h: u64, shift: u32) -> f32 {
    ((h >> shift) & 0xFFFF) as f32 * (1.0 / 65_536.0)
}

/// The cell's layout from its hash alone — no terrain taps — and the roll the
/// biome's density is compared against. Positions and radii are final; the
/// heights are not yet known.
fn plan(seed: u64, bcx: i32, bcz: i32) -> (f32, Formation) {
    let h = cell_hash(seed, bcx, bcz, CH_BOULDER);
    let roll = unit(h, 0);
    let mut f = Formation::EMPTY;
    let x0 = bcx as f32 * BOULDER_CELL_M;
    let z0 = bcz as f32 * BOULDER_CELL_M;
    // The main dome, in the middle of the cell.
    let span = BOULDER_CELL_M - 24.0;
    let cx = x0 + 12.0 + unit(h, 16) * span;
    let cz = z0 + 12.0 + unit(h, 32) * span;
    // Radius skewed small: most formations are house-sized, a few are hills.
    let t = unit(h, 48);
    let r = DOME_R_MIN + (DOME_R_MAX - DOME_R_MIN) * t * t;
    let h2 = cell_hash(seed, bcx, bcz, CH_BOULDER + 1);
    let tall = 0.5 + 0.45 * unit(h2, 0);
    f.domes[0] = Dome {
        x: cx,
        z: cz,
        y: 0.0,
        r,
        h: r * tall,
        yaw: (h2 >> 16) as u8,
        shape: (h2 >> 24) as u8,
    };
    f.n = 1;
    // Satellites: 0–3, around the main rock at its own scale.
    let sats = ((h2 >> 32) & 3) as usize;
    let mut i = 0;
    while i < sats && (f.n as usize) < DOMES_PER_CELL {
        let hs = cell_hash(seed, bcx, bcz, CH_BOULDER + 2 + i as u32);
        // A bearing from the yaw table's own quarter-turns plus a hashed
        // offset, walked with the LUT so no trig runs in the sim.
        let (s, c) = crate::yaw_dir((hs & 0xFFFF) as u16);
        let rs = r * (0.28 + 0.3 * unit(hs, 16));
        let dist = r * (0.75 + 0.6 * unit(hs, 32)) + rs * 0.4;
        f.domes[f.n as usize] = Dome {
            x: cx + s * dist,
            z: cz + c * dist,
            y: 0.0,
            r: rs,
            h: rs * (0.45 + 0.5 * unit(hs, 48)),
            yaw: (hs >> 40) as u8,
            shape: (hs >> 56) as u8,
        };
        f.n += 1;
        i += 1;
    }
    (roll, f)
}

/// Most any biome asks for; a roll above it is an empty cell without a tap.
const DENSITY_MAX: f32 = 0.62;

/// How likely a formation is on ground of this height, moisture and slope.
fn density(h: f32, moist: f32, slope: f32) -> f32 {
    if h < terrain::LAND_MIN_H + 0.4 {
        return 0.0;
    }
    if slope > terrain::CLIFF_SLOPE_RATIO * 1.2 {
        // A cliff is rock already; a dome on one floats.
        return 0.0;
    }
    if h < terrain::BEACH_MAX_H + 1.5 {
        return 0.28;
    }
    if h > terrain::TREELINE_H - 12.0 {
        return DENSITY_MAX;
    }
    // Rocky ground on the rises and slopes, fewer in the wet lowland forest.
    let lift = ((h - 20.0) * (1.0 / 40.0)).clamp(0.0, 1.0) * 0.18 + slope.min(1.0) * 0.12;
    if moist > 0.05 {
        0.10 + lift
    } else {
        0.17 + lift
    }
}

/// Whether a dome at (`x`, `z`) of radius `r` keeps clear of every road and
/// site the island has.
fn clear(haven: &Haven, x: f32, z: f32, r: f32) -> bool {
    let road = r + ROAD_CLEAR_M;
    if haven.ring.dist2(x, z) < road * road {
        return false;
    }
    for sr in haven.roads.iter().chain(haven.trails.iter()) {
        if sr.live && sr.dist2(x, z) < road * road {
            return false;
        }
    }
    let site = |sx: f32, sz: f32, fp: &terrain::SiteFootprint| {
        let d = fp.blend_m + SITE_CLEAR_M + r;
        let (dx, dz) = (x - sx, z - sz);
        dx * dx + dz * dz >= d * d
    };
    if !site(haven.x, haven.z, &terrain::HAVEN_FOOTPRINT) {
        return false;
    }
    if crate::landmark::covers(&haven.marks, x, z, r + SITE_CLEAR_M) {
        return false;
    }
    for ws in haven.minor.iter() {
        if ws.live && !site(ws.x, ws.z, terrain::site_footprint(ws.kind)) {
            return false;
        }
    }
    true
}

/// The formation standing in cell (`bcx`, `bcz`), resolved: heights tapped,
/// vetoes applied. Costs a handful of `height` taps for a present cell and a
/// hash for an empty one.
pub fn formation(seed: u64, haven: &Haven, bcx: i32, bcz: i32) -> Formation {
    let side = cells_per_side();
    if bcx < 0 || bcz < 0 || bcx >= side || bcz >= side {
        return Formation::EMPTY;
    }
    let (roll, mut f) = plan(seed, bcx, bcz);
    if roll >= DENSITY_MAX {
        return Formation::EMPTY;
    }
    let mut lat = terrain::Lattice::new();
    let main = f.domes[0];
    let h0 = terrain::height_memo(&mut lat, seed, main.x, main.z);
    let moist = terrain::moisture_memo(&mut lat, seed, main.x, main.z);
    let slope = terrain::slope_memo(&mut lat, seed, main.x, main.z);
    if roll >= density(h0, moist, slope) {
        return Formation::EMPTY;
    }
    // A beach grows boulders, not hills.
    let beach = h0 < terrain::BEACH_MAX_H + 1.5;
    let mut out = Formation::EMPTY;
    for (i, d) in f.domes[..f.n as usize].iter_mut().enumerate() {
        if beach {
            d.r *= 0.55;
            d.h *= 0.55;
        }
        // Smaller than this is the scatter's own `Rock`.
        if d.r < DOME_R_FLOOR {
            continue;
        }
        if !clear(haven, d.x, d.z, d.r) {
            // A satellite that would land on a road is dropped alone; a main
            // rock that would takes its formation with it.
            if i == 0 {
                return Formation::EMPTY;
            }
            continue;
        }
        // Sit on the LOWEST ground under the dome's footprint, so the downhill
        // side is never a lip over air, and sink a share of its own height.
        let k = d.r * 0.7;
        let mut lo = if i == 0 {
            h0
        } else {
            terrain::height_memo(&mut lat, seed, d.x, d.z)
        };
        if lo < terrain::LAND_MIN_H {
            continue;
        }
        for (ox, oz) in [(k, 0.0), (-k, 0.0), (0.0, k), (0.0, -k)] {
            lo = lo.min(terrain::height_memo(&mut lat, seed, d.x + ox, d.z + oz));
        }
        d.y = lo - d.h * 0.28;
        out.domes[out.n as usize] = *d;
        out.n += 1;
    }
    out
}

/// A dome's surface over (`x`, `z`), with its footprint widened by `dil`
/// metres: `(height, slope as rise/run)`, or `None` off it.
#[inline]
pub fn surface(d: &Dome, x: f32, z: f32, dil: f32) -> Option<(f32, f32)> {
    let r = d.r + dil;
    let (dx, dz) = (x - d.x, z - d.z);
    let q = (dx * dx + dz * dz) / (r * r);
    if q >= 1.0 {
        return None;
    }
    let t = (1.0 - q).sqrt();
    let h = d.h + dil * 0.5;
    let slope = if t > 1e-3 { h / r * q.sqrt() / t } else { 1e3 };
    Some((d.y + h * t, slope))
}

/// Can the formation cell (`bcx`, `bcz`) reach (`x`, `z`) at all, from its
/// hash alone? The cheap first half of every query.
#[inline]
pub fn reaches(bcx: i32, bcz: i32, x: f32, z: f32, pad: f32) -> bool {
    let cx = (bcx as f32 + 0.5) * BOULDER_CELL_M;
    let cz = (bcz as f32 + 0.5) * BOULDER_CELL_M;
    let reach = FORMATION_REACH_M + pad;
    let (dx, dz) = (x - cx, z - cz);
    dx * dx + dz * dz < reach * reach
}

/// The formation cells whose rocks can reach (`x`, `z`): the point's own and
/// whichever neighbours `FORMATION_REACH_M` crosses into.
pub fn cells_near(x: f32, z: f32, pad: f32, mut f: impl FnMut(i32, i32)) {
    let bx = floor_i32(x / BOULDER_CELL_M);
    let bz = floor_i32(z / BOULDER_CELL_M);
    let mut dz = -1;
    while dz <= 1 {
        let mut dx = -1;
        while dx <= 1 {
            if reaches(bx + dx, bz + dz, x, z, pad) {
                f(bx + dx, bz + dz);
            }
            dx += 1;
        }
        dz += 1;
    }
}

/// Whether any rock covers (`x`, `z`) — the scatter's veto, uncached: a tree
/// does not grow out of a boulder. Pads the footprint by `pad` metres.
pub fn covers(seed: u64, haven: &Haven, x: f32, z: f32, pad: f32) -> bool {
    let mut hit = false;
    cells_near(x, z, pad, |bx, bz| {
        if hit {
            return;
        }
        // Cheap reject before any tap: a cell whose layout cannot reach.
        let (roll, plan) = plan(seed, bx, bz);
        if roll >= DENSITY_MAX {
            return;
        }
        let near = plan.iter().any(|d| {
            let r = d.r + pad;
            let (dx, dz) = (x - d.x, z - d.z);
            dx * dx + dz * dz < r * r
        });
        if !near {
            return;
        }
        let f = formation(seed, haven, bx, bz);
        hit = f.iter().any(|d| {
            let r = d.r + pad;
            let (dx, dz) = (x - d.x, z - d.z);
            dx * dx + dz * dz < r * r
        });
    });
    hit
}
