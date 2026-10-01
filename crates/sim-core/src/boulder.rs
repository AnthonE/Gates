//! Rock formations: the island's big rocks, as a pure function of the seed.
//!
//! **Why this is not a scatter occupant.** The scatter grid holds one
//! occupant per 8 m cell and proves its 3×3 collision probe complete only
//! because nothing reaches past `CELL_SIZE` (`terrain::OCCUPANT_PROBE_CELLS`).
//! A rock a player reads as a landmark is 6–40 m across, so it lives on its
//! own coarse grid — one formation per 64 m cell at most — and answers its
//! own collision query, the way `depot::blocks` answers for the depot yard.
//! `reference/ROCKS.md` §9.2 is the plan this follows: formations first,
//! smaller rocks clustered around them, the scatter kept off their footprint.
//!
//! **A formation is a few fractured blocks** (2026-09-28, after the reference
//! game's own rocks: stacked slabs with near-vertical faces and flat, mossy
//! tops — domes read as igloos). Each block is an oriented box with its
//! corners cut and its top a tilted plane, standing on the ground or resting
//! on another block, and that is what the collision reads:
//!
//! - inside the footprint the top is ground a body can stand on;
//! - the sides are walls;
//! - a block resting on another may overhang it, and a body passes under;
//! - a projectile inside a block has hit rock.
//!
//! Three shapes by hash: an **outcrop** (slabs strung out along a line, their
//! tops following the slope — the commonest), a **tor** (a stack of shrinking
//! blocks) and a **pile**. The client draws each block as a fractured rock
//! mesh wearing the ground's own rock material (`render/boulders.rs`), so a
//! boulder and a cliff are one substance at two scales.
//!
//! Every number here is a first cut (operator, 2026-09-28: "bolders and
//! gigantic rocks are small they need to be huge").

use crate::fmath::{fabs, floor_i32};
use crate::rng::cell_hash;
use crate::terrain::{self, Haven};

/// Edge of one formation cell, metres.
pub const BOULDER_CELL_M: f32 = 64.0;
/// Blocks one formation may hold.
pub const BLOCKS_PER_CELL: usize = 8;
/// Longest half-extent a block is planned with, metres.
pub const BLOCK_HALF_MAX: f32 = 10.0;
/// Shortest half-extent a block is kept with, metres. Smaller is the
/// scatter's own `Rock`.
pub const BLOCK_HALF_FLOOR: f32 = 0.7;
/// How far a formation's blocks reach from its anchor, metres — a block's
/// centre distance plus its half-diagonal. `plan` drops any block past it.
pub const FORMATION_BODY_M: f32 = 34.0;
/// How far a formation's anchor is kept inside its cell, metres.
const ANCHOR_INSET_M: f32 = 16.0;
/// How far a formation's blocks can reach from its cell's centre, metres: the
/// anchor's own reach off centre and the body around it. A query only needs
/// the cells this can reach.
pub const FORMATION_REACH_M: f32 =
    (BOULDER_CELL_M * 0.5 - ANCHOR_INSET_M) * 1.4143 + FORMATION_BODY_M;
/// Corner cut of every block's footprint: a point is inside when
/// `|lx|/hx + |lz|/hz ≤ 2 − CHAMFER`. The drawn block rounds its vertical
/// edges to about the same line (`render/boulders.rs`).
pub const CHAMFER: f32 = 0.2;
/// Steepest a block's top may tilt, rise per metre — under the cliff ratio,
/// so every top is ground.
pub const TILT_MAX: f32 = 0.5;
/// How much of the ground's own grade a seated block's top follows: strata
/// breaking out of a hillside lean with it.
const FOLLOW: f32 = 0.6;
/// Clearance kept between a block and a road's centreline, beyond the block's
/// own half-diagonal, metres — the carriageway, its shoulder and the ring's
/// blend.
pub const ROAD_CLEAR_M: f32 = terrain::ROAD_SHOULDER_HALF_W + terrain::RING_BLEND_M + 3.0;
/// Clearance beyond a site's blend radius, metres.
pub const SITE_CLEAR_M: f32 = 20.0;
/// The slope a block's rim reports: a wall.
const WALL: f32 = 1e3;
/// A block resting on nothing but the ground.
pub const ON_GROUND: u8 = u8::MAX;

const CH_BOULDER: u32 = 176;
/// The plan's own draws start here, one channel per four draws.
const CH_BLOCK_DRAW: u32 = 0x00B0_0000;

const _: () = {
    // The query scans the formation cells a reach can touch, 3×3 at most.
    assert!(FORMATION_REACH_M < BOULDER_CELL_M);
    assert!(BLOCK_HALF_FLOOR < BLOCK_HALF_MAX);
    assert!(TILT_MAX < terrain::CLIFF_SLOPE_RATIO);
    assert!(BLOCKS_PER_CELL < ON_GROUND as usize);
};

/// One block: an oriented box, corners cut by [`CHAMFER`], its top a plane.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Block {
    pub x: f32,
    pub z: f32,
    /// Half-extents along the block's own x and z.
    pub hx: f32,
    pub hz: f32,
    /// Its bottom: under the ground for a seated block, sunk into the one
    /// under it for a stacked one.
    pub y0: f32,
    /// Its top over its centre.
    pub y1: f32,
    /// The top's rise per metre along the block's own x and z.
    pub tx: f32,
    pub tz: f32,
    /// Bearing, a yaw-LUT index. Local +Z is `yaw_dir(yaw)`, the landmark
    /// convention.
    pub yaw: u16,
    /// Mesh variation for the client; the sim never reads it.
    pub shape: u8,
    /// The block this one rests on, or [`ON_GROUND`].
    pub on: u8,
}

impl Block {
    /// (`x`, `z`) in the block's own frame.
    #[inline]
    pub fn to_local(&self, x: f32, z: f32) -> (f32, f32) {
        let (s, c) = crate::yaw_dir(self.yaw);
        let (dx, dz) = (x - self.x, z - self.z);
        (dx * c - dz * s, dx * s + dz * c)
    }

    /// A point of the block's own frame in world space.
    #[inline]
    pub fn to_world(&self, lx: f32, lz: f32) -> (f32, f32) {
        let (s, c) = crate::yaw_dir(self.yaw);
        (self.x + lx * c + lz * s, self.z - lx * s + lz * c)
    }

    /// The top's height over a point of the block's own frame, the plane
    /// held level past the footprint's edge.
    #[inline]
    pub fn top_local(&self, lx: f32, lz: f32) -> f32 {
        self.y1 + self.tx * lx.clamp(-self.hx, self.hx) + self.tz * lz.clamp(-self.hz, self.hz)
    }

    /// Half the footprint's diagonal: every point of the block is this close
    /// to its centre in plan.
    #[inline]
    pub fn radius(&self) -> f32 {
        (self.hx * self.hx + self.hz * self.hz).sqrt()
    }
}

/// One cell's formation. `n` blocks, the rest zeroed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formation {
    pub n: u8,
    pub blocks: [Block; BLOCKS_PER_CELL],
}

impl Formation {
    pub const EMPTY: Formation = Formation {
        n: 0,
        blocks: [Block {
            x: 0.0,
            z: 0.0,
            hx: 0.0,
            hz: 0.0,
            y0: 0.0,
            y1: 0.0,
            tx: 0.0,
            tz: 0.0,
            yaw: 0,
            shape: 0,
            on: ON_GROUND,
        }; BLOCKS_PER_CELL],
    };

    pub fn iter(&self) -> impl Iterator<Item = &Block> {
        self.blocks[..self.n as usize].iter()
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

/// A cell's stream of uniform draws: four to a hash, channel after channel.
struct Draw {
    seed: u64,
    bx: i32,
    bz: i32,
    ch: u32,
    bits: u64,
    left: u32,
}

impl Draw {
    fn new(seed: u64, bx: i32, bz: i32) -> Self {
        Draw {
            seed,
            bx,
            bz,
            ch: CH_BLOCK_DRAW,
            bits: 0,
            left: 0,
        }
    }

    fn unit(&mut self) -> f32 {
        if self.left == 0 {
            self.bits = cell_hash(self.seed, self.bx, self.bz, self.ch);
            self.ch += 1;
            self.left = 4;
        }
        let v = (self.bits & 0xFFFF) as f32 * (1.0 / 65_536.0);
        self.bits >>= 16;
        self.left -= 1;
        v
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// A bearing within `±spread` of `around`, in yaw-LUT units.
    fn yaw_near(&mut self, around: u16, spread: f32) -> u16 {
        let off = (self.unit() * 2.0 - 1.0) * spread;
        around.wrapping_add(off as i32 as u16)
    }

    fn byte(&mut self) -> u8 {
        (self.unit() * 256.0) as u8
    }
}

/// A planned block: its footprint final, its heights not yet known. `y1`
/// holds the height it stands above what it rests on, `tx`/`tz` the tilt it
/// adds of its own.
#[allow(clippy::too_many_arguments)]
fn planned(x: f32, z: f32, hx: f32, hz: f32, hgt: f32, yaw: u16, d: &mut Draw, on: u8) -> Block {
    Block {
        x,
        z,
        hx,
        hz,
        y0: 0.0,
        y1: hgt,
        tx: d.range(-0.12, 0.12),
        tz: d.range(-0.12, 0.12),
        yaw,
        shape: d.byte(),
        on,
    }
}

/// Keep `b` if it is big enough and inside the formation's body. Its index,
/// if kept.
fn push(f: &mut Formation, b: Block, ax: f32, az: f32) -> Option<u8> {
    if b.hx < BLOCK_HALF_FLOOR || b.hz < BLOCK_HALF_FLOOR || f.n as usize >= BLOCKS_PER_CELL {
        return None;
    }
    let (dx, dz) = (b.x - ax, b.z - az);
    if (dx * dx + dz * dz).sqrt() + b.radius() > FORMATION_BODY_M {
        return None;
    }
    f.blocks[f.n as usize] = b;
    f.n += 1;
    Some(f.n - 1)
}

/// A block resting on `parent`, offset inside its footprint, a share of its
/// size, turned a little.
fn stack_on(
    f: &mut Formation,
    d: &mut Draw,
    parent: u8,
    shrink: (f32, f32),
    ax: f32,
    az: f32,
) -> Option<u8> {
    let p = f.blocks[parent as usize];
    let hx = p.hx * d.range(shrink.0, shrink.1);
    let hz = p.hz * d.range(shrink.0, shrink.1);
    let (lx, lz) = (d.range(-0.3, 0.3) * p.hx, d.range(-0.3, 0.3) * p.hz);
    let (x, z) = p.to_world(lx, lz);
    let yaw = d.yaw_near(p.yaw, 9_000.0);
    let hgt = hx.min(hz) * d.range(0.9, 1.6);
    let b = planned(x, z, hx, hz, hgt, yaw, d, parent);
    push(f, b, ax, az)
}

/// The cell's layout from its hash alone — no terrain taps — and the roll the
/// biome's density is compared against. Footprints are final; heights are
/// not yet known.
fn plan(seed: u64, bcx: i32, bcz: i32) -> (f32, Formation) {
    let h = cell_hash(seed, bcx, bcz, CH_BOULDER);
    let roll = unit(h, 0);
    let mut f = Formation::EMPTY;
    let span = BOULDER_CELL_M - 2.0 * ANCHOR_INSET_M;
    let ax = bcx as f32 * BOULDER_CELL_M + ANCHOR_INSET_M + unit(h, 16) * span;
    let az = bcz as f32 * BOULDER_CELL_M + ANCHOR_INSET_M + unit(h, 32) * span;
    // Size skewed small: most formations are house-sized, a few are hills.
    let t = unit(h, 48);
    let big = BLOCK_HALF_MAX * (0.3 + 0.7 * t * t.sqrt());
    let mut d = Draw::new(seed, bcx, bcz);
    let shape = d.unit();
    let axis = (d.unit() * 65_536.0) as u16;
    if shape < 0.55 {
        outcrop(&mut f, &mut d, ax, az, big, axis);
    } else if shape < 0.85 {
        tor(&mut f, &mut d, ax, az, big, axis);
    } else {
        pile(&mut f, &mut d, ax, az, big);
    }
    (roll, f)
}

/// Slabs strung out along `axis`, biggest in the middle, overlapping, with a
/// smaller one or two resting on top.
fn outcrop(f: &mut Formation, d: &mut Draw, ax: f32, az: f32, big: f32, axis: u16) {
    let n = 2 + (d.unit() * 3.0) as usize;
    let mut half = [0.0f32; 4];
    for (k, hk) in half.iter_mut().enumerate().take(n) {
        let mid = 1.0 - fabs(k as f32 - (n as f32 - 1.0) * 0.5) / n as f32;
        *hk = big * d.range(0.5, 0.75) * (0.55 + 0.45 * mid);
    }
    // The string's own frame: +x along it, +z across.
    let (s, c) = crate::yaw_dir(axis);
    let total: f32 = half[..n].iter().sum::<f32>() * 1.6;
    let mut along = -total * 0.5;
    let mut main = None;
    let mut widest = 0.0f32;
    for &hx in half.iter().take(n) {
        along += hx * 0.8;
        let hz = hx * d.range(0.45, 0.8);
        let lat = d.range(-0.35, 0.35) * hz;
        let (x, z) = (ax + c * along + s * lat, az - s * along + c * lat);
        let hgt = hz * d.range(0.8, 1.5);
        let yaw = d.yaw_near(axis, 3_000.0);
        let b = planned(x, z, hx, hz, hgt, yaw, d, ON_GROUND);
        if let Some(i) = push(f, b, ax, az) {
            if hx > widest {
                widest = hx;
                main = Some(i);
            }
        }
        along += hx * 0.8;
    }
    if let Some(m) = main {
        let tops = (d.unit() * 3.0) as usize;
        for _ in 0..tops {
            stack_on(f, d, m, (0.3, 0.55), ax, az);
        }
    }
}

/// A stack of shrinking blocks on a broad base, with a fallen block or two at
/// its foot.
fn tor(f: &mut Formation, d: &mut Draw, ax: f32, az: f32, big: f32, axis: u16) {
    let hx = big * d.range(0.35, 0.55);
    let hz = hx * d.range(0.7, 1.0);
    let hgt = hx * d.range(0.8, 1.4);
    let base = planned(ax, az, hx, hz, hgt, axis, d, ON_GROUND);
    let Some(mut top) = push(f, base, ax, az) else {
        return;
    };
    let tiers = 1 + (d.unit() * 3.0) as usize;
    for _ in 0..tiers {
        match stack_on(f, d, top, (0.6, 0.85), ax, az) {
            Some(i) => top = i,
            None => break,
        }
    }
    let fallen = (d.unit() * 3.0) as usize;
    for _ in 0..fallen {
        let (s, c) = crate::yaw_dir((d.unit() * 65_536.0) as u16);
        let r = hx.max(hz) * d.range(1.2, 1.8);
        let fx = hx * d.range(0.25, 0.45);
        let b = planned(
            ax + s * r,
            az + c * r,
            fx,
            fx * d.range(0.6, 1.0),
            fx * d.range(0.8, 1.3),
            (d.unit() * 65_536.0) as u16,
            d,
            ON_GROUND,
        );
        push(f, b, ax, az);
    }
}

/// A tumble of middling blocks, a few resting on the others.
fn pile(f: &mut Formation, d: &mut Draw, ax: f32, az: f32, big: f32) {
    let n = 4 + (d.unit() * 4.0) as usize;
    let spread = big * 0.9;
    for _ in 0..n {
        let stackable = f.n > 0 && d.unit() < 0.3;
        if stackable {
            let on = (d.unit() * f.n as f32) as u8;
            if f.blocks[on as usize].on == ON_GROUND {
                stack_on(f, d, on, (0.4, 0.7), ax, az);
                continue;
            }
        }
        let (s, c) = crate::yaw_dir((d.unit() * 65_536.0) as u16);
        let r = spread * d.unit().sqrt();
        let hx = big * d.range(0.2, 0.42);
        let b = planned(
            ax + s * r,
            az + c * r,
            hx,
            hx * d.range(0.6, 1.0),
            hx * d.range(0.8, 1.4),
            (d.unit() * 65_536.0) as u16,
            d,
            ON_GROUND,
        );
        push(f, b, ax, az);
    }
}

/// Most any biome asks for; a roll above it is an empty cell without a tap.
const DENSITY_MAX: f32 = 0.62;

/// How likely a formation is on ground of this height, moisture and slope.
fn density(h: f32, moist: f32, slope: f32) -> f32 {
    if h < terrain::LAND_MIN_H + 0.4 {
        return 0.0;
    }
    if slope > terrain::CLIFF_SLOPE_RATIO * 1.2 {
        // A cliff is rock already; a block on one floats.
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

/// Whether a block at (`x`, `z`) of plan radius `r` keeps clear of every road
/// and site the island has.
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
    if crate::town::covers(&haven.town, x, z, r + SITE_CLEAR_M) {
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
/// vetoes applied. Costs a handful of `height` taps a block for a present
/// cell and a hash for an empty one.
pub fn formation(seed: u64, haven: &Haven, bcx: i32, bcz: i32) -> Formation {
    let side = cells_per_side();
    if bcx < 0 || bcz < 0 || bcx >= side || bcz >= side {
        return Formation::EMPTY;
    }
    let (roll, plan) = plan(seed, bcx, bcz);
    if roll >= DENSITY_MAX || plan.n == 0 {
        return Formation::EMPTY;
    }
    let mut lat = terrain::Lattice::new();
    let main = plan.blocks[0];
    let h0 = terrain::height_memo(&mut lat, seed, main.x, main.z);
    let moist = terrain::moisture_memo(&mut lat, seed, main.x, main.z);
    let slope = terrain::slope_memo(&mut lat, seed, main.x, main.z);
    if roll >= density(h0, moist, slope) {
        return Formation::EMPTY;
    }
    // A beach grows boulders, not hills.
    let beach = h0 < terrain::BEACH_MAX_H + 1.5;
    let mut out = Formation::EMPTY;
    // Plan index → resolved index, for the blocks that rest on others.
    let mut kept = [ON_GROUND; BLOCKS_PER_CELL];
    for (i, pb) in plan.iter().enumerate() {
        let mut b = *pb;
        if beach {
            b.hx *= 0.55;
            b.hz *= 0.55;
            b.y1 *= 0.55;
        }
        if b.hx < BLOCK_HALF_FLOOR || b.hz < BLOCK_HALF_FLOOR {
            continue;
        }
        if !clear(haven, b.x, b.z, b.radius()) {
            // A block that would land on a road is dropped alone; a first
            // block that would takes its formation with it.
            if i == 0 {
                return Formation::EMPTY;
            }
            continue;
        }
        if b.on == ON_GROUND {
            if !seat(&mut b, &mut lat, seed) {
                continue;
            }
        } else {
            let p = kept[b.on as usize];
            if p == ON_GROUND {
                continue;
            }
            let under = out.blocks[p as usize];
            let (lx, lz) = under.to_local(b.x, b.z);
            let base = under.top_local(lx, lz);
            let hgt = b.y1;
            b.y0 = base - 0.3;
            b.y1 = base + hgt;
            b.on = p;
        }
        kept[i] = out.n;
        out.blocks[out.n as usize] = b;
        out.n += 1;
    }
    out
}

/// Seat a ground block: its top follows the ground's grade (plus its own
/// lean), stands its planned height over the ground at its centre and a share
/// of it over every corner, and its bottom is buried under the lowest ground
/// it covers — so the downhill side is never a lip over air. False if it
/// stands in the sea.
fn seat(b: &mut Block, lat: &mut terrain::Lattice, seed: u64) -> bool {
    let gc = terrain::height_memo(lat, seed, b.x, b.z);
    if gc < terrain::LAND_MIN_H {
        return false;
    }
    let corners = [(b.hx, b.hz), (b.hx, -b.hz), (-b.hx, b.hz), (-b.hx, -b.hz)];
    let mut g = [0.0f32; 4];
    let mut lo = gc;
    for (k, &(lx, lz)) in corners.iter().enumerate() {
        let (x, z) = b.to_world(lx, lz);
        g[k] = terrain::height_memo(lat, seed, x, z);
        lo = lo.min(g[k]);
    }
    let gx = ((g[0] + g[1]) - (g[2] + g[3])) / (4.0 * b.hx);
    let gz = ((g[0] + g[2]) - (g[1] + g[3])) / (4.0 * b.hz);
    b.tx = (gx * FOLLOW + b.tx).clamp(-TILT_MAX, TILT_MAX);
    b.tz = (gz * FOLLOW + b.tz).clamp(-TILT_MAX, TILT_MAX);
    let hgt = b.y1;
    let mut y1 = gc + hgt;
    for (k, &(lx, lz)) in corners.iter().enumerate() {
        let top = y1 + b.tx * lx + b.tz * lz;
        let need = g[k] + hgt * 0.4;
        if top < need {
            y1 += need - top;
        }
    }
    b.y1 = y1;
    b.y0 = lo - (0.5 + hgt * 0.3);
    true
}

/// A block's surface over (`x`, `z`), its footprint widened by `dil` metres:
/// `(top, slope as rise/run, bottom)`, or `None` off it. The widened rim
/// reports a wall's slope — it is the block's side.
#[inline]
pub fn surface(b: &Block, x: f32, z: f32, dil: f32) -> Option<(f32, f32, f32)> {
    let (lx, lz) = b.to_local(x, z);
    let (ax, az) = (fabs(lx), fabs(lz));
    let (ex, ez) = (b.hx + dil, b.hz + dil);
    if ax > ex || az > ez || ax / ex + az / ez > 2.0 - CHAMFER {
        return None;
    }
    let rim = ax > b.hx || az > b.hz || ax / b.hx + az / b.hz > 2.0 - CHAMFER;
    let slope = if rim {
        WALL
    } else {
        (b.tx * b.tx + b.tz * b.tz).sqrt()
    };
    Some((b.top_local(lx, lz), slope, b.y0))
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
        // Cheap reject before any tap: a cell whose layout cannot reach. The
        // plan's footprints bound the resolved ones (a beach only shrinks).
        let (roll, plan) = plan(seed, bx, bz);
        if roll >= DENSITY_MAX {
            return;
        }
        let near = plan.iter().any(|b| {
            let r = b.radius() + pad;
            let (dx, dz) = (x - b.x, z - b.z);
            dx * dx + dz * dz < r * r
        });
        if !near {
            return;
        }
        let f = formation(seed, haven, bx, bz);
        hit = f.iter().any(|b| surface(b, x, z, pad).is_some());
    });
    hit
}
