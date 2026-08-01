//! The island as a pure function of the seed (TERRAIN.md). `height`, masks,
//! biomes, and the scatter pass — all integer hashes + walled float ops, so
//! native and wasm agree bit for bit. Road ring and haven pad are the next
//! worldgen slice; adding them regenerates the goldens in the same commit.
//!
//! Numbers of record are TERRAIN.md §6; generator shape params (frequencies,
//! warp amplitude, remap LUT, scatter weights) are registered in DECISIONS.md
//! §open and pinned by `test_terrain_golden`.

use crate::fmath::{fabs, fade, floor_i32, lerp};
use crate::rng::cell_hash;

/// Island edge length in meters (knob, DECISIONS.md: 2,048).
pub const ISLAND_SIZE: f32 = 2048.0;
/// Sea level (TERRAIN.md §6: 0).
pub const SEA_LEVEL: f32 = 0.0;
/// Relief amplitude (TERRAIN.md §1: ~90 m).
pub const AMPLITUDE: f32 = 90.0;
/// Scatter cell size in meters (TERRAIN.md §6: 8 m).
pub const CELL_SIZE: f32 = 8.0;
/// Scatter cells per island side (2048 / 8).
pub const CELLS_PER_SIDE: i32 = 256;
/// Cliff threshold as a rise/run ratio: tan(50°), authored offline —
/// no trig at runtime (TERRAIN.md §1 knob: slope > ~50°).
pub const CLIFF_SLOPE_RATIO: f32 = 1.191_753_6;

// Noise channels: independent fields from one seed (TERRAIN.md §0).
const CH_RELIEF: u32 = 0; // +octave index, 5 octaves
const CH_WARP_X: u32 = 16;
const CH_WARP_Z: u32 = 24;
const CH_COAST: u32 = 32;
const CH_MOIST: u32 = 40;
const CH_RIDGE: u32 = 48;
const CH_SCATTER: u32 = 64;

// Generator shape (DECISIONS.md §open: worldgen shape params, golden-pinned).
const RELIEF_FREQ: f32 = 1.0 / 600.0;
/// fBm output clusters well inside [-1, 1]; this gain stretches it so the
/// remap LUT sees its full domain and peaks actually reach the amplitude.
const RELIEF_GAIN: f32 = 2.4;
const WARP_FREQ: f32 = 1.0 / 1200.0;
const WARP_AMP: f32 = 45.0;
const COAST_FREQ: f32 = 1.0 / 900.0;
const COAST_WOBBLE: f32 = 100.0;
const CONTINENT_RADIUS: f32 = 960.0;
const COAST_EDGE_WIDTH: f32 = 160.0;
const SEA_FLOOR_DEPTH: f32 = 12.0;
const MOIST_FREQ: f32 = 1.0 / 700.0;
const RIDGE_FREQ: f32 = 1.0 / 220.0;
const RIDGE_AMP: f32 = 16.0;
const RIDGE_START_H: f32 = 52.0;
const RIDGE_FULL_H: f32 = 80.0;

/// 8 gradient directions, unit length; diagonals use the std constant
/// √2/2 — a constant, not a runtime trig call (TERRAIN.md §0).
const DIAG: f32 = core::f32::consts::FRAC_1_SQRT_2;
const GRAD8: [[f32; 2]; 8] = [
    [1.0, 0.0],
    [-1.0, 0.0],
    [0.0, 1.0],
    [0.0, -1.0],
    [DIAG, DIAG],
    [-DIAG, DIAG],
    [DIAG, -DIAG],
    [-DIAG, -DIAG],
];

/// Gradient noise, one octave, quintic-smoothed, output ~[-1, 1].
/// `fx`/`fz` are already frequency-scaled lattice coordinates.
fn noise2(seed: u64, channel: u32, fx: f32, fz: f32) -> f32 {
    let x0 = floor_i32(fx);
    let z0 = floor_i32(fz);
    let tx = fx - x0 as f32;
    let tz = fz - z0 as f32;

    #[inline]
    fn corner(seed: u64, channel: u32, cx: i32, cz: i32, dx: f32, dz: f32) -> f32 {
        let g = GRAD8[(cell_hash(seed, cx, cz, channel) & 7) as usize];
        g[0] * dx + g[1] * dz
    }

    let d00 = corner(seed, channel, x0, z0, tx, tz);
    let d10 = corner(seed, channel, x0 + 1, z0, tx - 1.0, tz);
    let d01 = corner(seed, channel, x0, z0 + 1, tx, tz - 1.0);
    let d11 = corner(seed, channel, x0 + 1, z0 + 1, tx - 1.0, tz - 1.0);

    let u = fade(tx);
    let v = fade(tz);
    lerp(lerp(d00, d10, u), lerp(d01, d11, u), v) * core::f32::consts::SQRT_2
}

/// Fractal Brownian motion: `octaves` octaves, lacunarity 2, gain 0.5,
/// normalized to ~[-1, 1]. Octave index offsets the channel so octaves
/// decorrelate (TERRAIN.md §1: 5-octave fBm for relief).
fn fbm(seed: u64, channel: u32, x: f32, z: f32, base_freq: f32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let mut freq = base_freq;
    let mut o = 0;
    while o < octaves {
        sum += noise2(seed, channel + o, x * freq, z * freq) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
        o += 1;
    }
    sum / norm
}

/// Height remap LUT (TERRAIN.md §1 stage 4): flattens mid-elevations into
/// buildable shelves, steepens transitions. 17 entries over n ∈ [0, 1].
const REMAP_LUT: [f32; 17] = [
    0.000, 0.030, 0.060, 0.090, 0.115, 0.135, 0.150, 0.160, 0.240, 0.330, 0.370, 0.390, 0.400,
    0.520, 0.700, 0.850, 1.000,
];

fn remap(n: f32) -> f32 {
    let t = n.clamp(0.0, 1.0) * 16.0;
    let i = floor_i32(t).clamp(0, 15) as usize;
    lerp(REMAP_LUT[i], REMAP_LUT[i + 1], t - i as f32)
}

/// Continent mask (TERRAIN.md §1 stage 1): radial falloff, coastline
/// wobbled by low-frequency noise. 1 inland, 0 at sea.
fn continent(seed: u64, x: f32, z: f32) -> f32 {
    let dx = x - ISLAND_SIZE * 0.5;
    let dz = z - ISLAND_SIZE * 0.5;
    let d = (dx * dx + dz * dz).sqrt();
    let wobble = fbm(seed, CH_COAST, x, z, COAST_FREQ, 2) * COAST_WOBBLE;
    let t = ((CONTINENT_RADIUS + wobble - d) / COAST_EDGE_WIDTH).clamp(0.0, 1.0);
    fade(t)
}

/// The authoritative height function: TERRAIN.md §1 stages 1–4 composed.
pub fn height(seed: u64, x: f32, z: f32) -> f32 {
    let wx = x + fbm(seed, CH_WARP_X, x, z, WARP_FREQ, 2) * WARP_AMP;
    let wz = z + fbm(seed, CH_WARP_Z, x, z, WARP_FREQ, 2) * WARP_AMP;
    let relief = fbm(seed, CH_RELIEF, wx, wz, RELIEF_FREQ, 5);
    let shelfed = remap(relief * RELIEF_GAIN * 0.5 + 0.5);
    let mut land = shelfed * AMPLITUDE;

    // Ridged blend above the treeline: fakes erosion, no simulation.
    let ridge_t = ((land - RIDGE_START_H) / (RIDGE_FULL_H - RIDGE_START_H)).clamp(0.0, 1.0);
    if ridge_t > 0.0 {
        let ridged = 1.0 - fabs(noise2(seed, CH_RIDGE, wx * RIDGE_FREQ, wz * RIDGE_FREQ));
        land += ridge_t * ridged * RIDGE_AMP;
    }

    let m = continent(seed, x, z);
    m * land - (1.0 - m) * SEA_FLOOR_DEPTH
}

/// Slope as rise/run from central finite differences at 1 m (TERRAIN.md §1
/// stage 5 — derived, never stored).
pub fn slope(seed: u64, x: f32, z: f32) -> f32 {
    let sx = (height(seed, x + 1.0, z) - height(seed, x - 1.0, z)) * 0.5;
    let sz = (height(seed, x, z + 1.0) - height(seed, x, z - 1.0)) * 0.5;
    (sx * sx + sz * sz).sqrt()
}

/// Moisture channel in ~[-1, 1] (TERRAIN.md §1 stage 5).
pub fn moisture(seed: u64, x: f32, z: f32) -> f32 {
    fbm(seed, CH_MOIST, x, z, MOIST_FREQ, 2)
}

/// The four alpha biomes (TERRAIN.md §1 stage 6). Data, not behavior.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Biome {
    Beach = 0,
    Meadow = 1,
    Forest = 2,
    Highland = 3,
}

/// The beach mask (TERRAIN.md §1 stage 5): height within ~2 m of sea
/// level. One definition — `biome` classifies by it and `World::spawn_pos`
/// spawns into it, so the spawn zone cannot drift away from the biome.
pub const BEACH_MAX_H: f32 = SEA_LEVEL + 2.0;

pub fn biome(h: f32, moist: f32) -> Biome {
    if h < BEACH_MAX_H {
        Biome::Beach
    } else if h > 52.0 {
        Biome::Highland
    } else if moist > 0.05 {
        Biome::Forest
    } else {
        Biome::Meadow
    }
}

/// What a scatter cell holds (TERRAIN.md §1 stage 9's occupant list).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Occupant {
    None = 0,
    Tree = 1,
    StoneNode = 2,
    MetalNode = 3,
    SulfurNode = 4,
    Bush = 5,
    Rock = 6,
    BarrelSlot = 7,
}

const OCCUPANT_KINDS: usize = 7;

/// Per-biome scatter weights in per-mille of a cell draw, order
/// [Tree, Stone, Metal, Sulfur, Bush, Rock, Barrel]; remainder is None.
/// Data-shaped on purpose: the content pass (M1) feeds this from
/// `content/*.toml`; until then the alpha default below is the documented
/// default (DECISIONS.md §open) and the golden pins it.
#[derive(Clone, Copy)]
pub struct ScatterTable {
    pub weights: [[u16; OCCUPANT_KINDS]; 4],
}

impl ScatterTable {
    pub const fn alpha_default() -> Self {
        Self {
            weights: [
                // Beach: barrels wash up, little else (TERRAIN.md §1).
                [0, 0, 0, 0, 20, 30, 250],
                // Meadow: buildable, sparse trees.
                [70, 15, 0, 0, 70, 25, 0],
                // Forest: wood, cover.
                [260, 12, 0, 0, 50, 28, 0],
                // Highland: the ore, the exposure.
                [20, 70, 60, 45, 10, 80, 0],
            ],
        }
    }
}

/// A resolved scatter slot: potential, not state (TERRAIN.md §2). The
/// server owns harvested/standing bits elsewhere; this is the backdrop.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub occupant: Occupant,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: u8,
    /// Visual scale in [0.9, 1.1].
    pub scale: f32,
}

/// One hash draw decides a cell's occupant, offset, yaw, scale
/// (TERRAIN.md §1 stage 9). Slope, water — and later road/haven — veto.
pub fn scatter(seed: u64, table: &ScatterTable, cell_x: i32, cell_z: i32) -> Slot {
    let none = Slot {
        occupant: Occupant::None,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0,
        scale: 1.0,
    };
    if !(0..CELLS_PER_SIDE).contains(&cell_x) || !(0..CELLS_PER_SIDE).contains(&cell_z) {
        return none;
    }
    let h = cell_hash(seed, cell_x, cell_z, CH_SCATTER);

    // Jittered position first: vetoes apply where the thing would stand.
    let jx = ((h >> 16) & 0x3F) as f32 * (6.0 / 63.0) - 3.0;
    let jz = ((h >> 22) & 0x3F) as f32 * (6.0 / 63.0) - 3.0;
    let x = cell_x as f32 * CELL_SIZE + 4.0 + jx;
    let z = cell_z as f32 * CELL_SIZE + 4.0 + jz;

    let hy = height(seed, x, z);
    if hy < 0.6 || slope(seed, x, z) > CLIFF_SLOPE_RATIO {
        return none;
    }

    let row = &table.weights[biome(hy, moisture(seed, x, z)) as usize];
    let roll = (h % 1000) as u16;
    let mut acc = 0u16;
    let mut occupant = Occupant::None;
    for (i, w) in row.iter().enumerate() {
        acc += w;
        if roll < acc {
            occupant = match i {
                0 => Occupant::Tree,
                1 => Occupant::StoneNode,
                2 => Occupant::MetalNode,
                3 => Occupant::SulfurNode,
                4 => Occupant::Bush,
                5 => Occupant::Rock,
                _ => Occupant::BarrelSlot,
            };
            break;
        }
    }
    if occupant == Occupant::None {
        return none;
    }

    Slot {
        occupant,
        x,
        y: hy,
        z,
        yaw: ((h >> 28) & 0xFF) as u8,
        scale: 0.9 + ((h >> 36) & 0xFF) as f32 * (0.2 / 255.0),
    }
}
