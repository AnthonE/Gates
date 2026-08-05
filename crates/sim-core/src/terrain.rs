//! The island as a pure function of the seed (TERRAIN.md). `height`, masks,
//! biomes, and the scatter pass — all integer hashes + walled float ops, so
//! native and wasm agree bit for bit. The coast road (stage 7) is below and
//! costs the goldens nothing — it reads `height`, never writes it, and the
//! golden's 256 scatter cells sit at the island center, far inside the ring.
//! The haven pad (stage 8) costs them nothing for the same reason: it is
//! placement and a veto, not a write. Its carve is the part still unbuilt.
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
const CH_CLUMP: u32 = 72; // +octave index, 3 octaves
const CH_CLUTTER: u32 = 80; // the sub-metre ground population

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

// The grove/clearing field (`clump`, DECISIONS.md §open: scatter clumping v0).
/// Base wavelength of the field in meters — the size of one grove, and of
/// one clearing. Derived from the thing it has to change rather than
/// chosen: the measurement that says our forest is an orchard counts trees
/// in a 40 m window, so the field has to be wide enough that a window sits
/// mostly inside one grove or one clearing rather than averaging several.
/// 96 m is twelve scatter cells and 2.4 windows; two octaves down it is
/// still 24 m, so a grove has an edge instead of a contour.
const CLUMP_FREQ: f32 = 1.0 / 96.0;
/// `SPAWN.md` §9.3 asks for "a cheap 2–3 octave value-noise channel"; the
/// top of that range, because the third octave is what stops a clearing
/// from being an ellipse.
const CLUMP_OCTAVES: u32 = 3;
/// Stretch on the fBm before it is remapped to [0, 1] — the same job
/// `RELIEF_GAIN` does for height, for the same reason (fBm output clusters
/// well inside [-1, 1]). Above ~2.9 the field spends most of its area
/// clamped at one rail or the other, which is a stencil, not a field.
const CLUMP_GAIN: f32 = 2.4;
/// What a clearing keeps. Not zero: a bald clearing is as unlike the
/// reference as an orchard, and the roll still has to be able to put a
/// rock in one. At 0.15 the floor survives the square as 0.0225 before
/// normalization — a clearing runs at roughly a twentieth of a grove.
const CLUMP_FLOOR: f32 = 0.15;
/// Reciprocal of the island mean of the squared factor, so the field
/// redistributes density without changing how much of it there is —
/// `TERRAIN.md` §6's live-slot band is a budget, and a texture change is
/// not allowed to spend it. Derived by measurement, not chosen, and
/// `tests/scatter.rs` re-derives it independently and fails if the mean has
/// drifted off 1.0 (the same discipline `Haven::relief` carries).
///
/// Measured: the squared factor means 0.3699 / 0.3695 / 0.3692 / 0.3734 on
/// seeds 0, 1, 7 and 12345 over the 65,536-sample 8 m grid. 2.70 is the
/// reciprocal of the middle of that, and inside 1% of all four — the
/// residual is the field's own seed-to-seed wobble, which no single
/// constant can take out and which the gate's tolerance carries instead.
const CLUMP_NORM: f32 = 2.70;

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

/// The grove/clearing field: a multiplier on the scatter weight row, mean 1
/// over the island (TERRAIN.md §1 stage 9, `reference/SPAWN.md` §9.3).
///
/// This is the whole of our answer to the one defect that file calls "the
/// highest-value item" in it. The reference gets clumping from a stateful
/// sampler — `ClusterSizeMin..Max` objects drawn out of one quadtree leaf,
/// braked by a 20 m local density cap (`SPAWN.md` §3.4) — and we cannot
/// have that: `scatter` is a pure function of one cell and must stay one,
/// or every caller that resolves a cell on demand (the client bridge, per
/// chunk) has to resolve the island instead. So the clumping moves from the
/// sampler into the *weight*: a cell still decides alone, but what it
/// decides against is a low-frequency field shared with its neighbours.
/// Groves where the field is high, clearings where it is low, still O(1),
/// still one hash draw per cell, one extra fBm read.
///
/// Measured on the tree field before it existed: inside the forest biome,
/// the variance of the tree count in a 40 m window was 1.05x the
/// independent-draw null on the shipped seed (0.98–1.05 over three seeds),
/// and 3 windows in 10,000 were empty. That is white noise with a number on
/// it — `TERRAIN.md` §1 stage 6 asks forest for "cover, low visibility" and
/// an independent draw delivers an orchard. `tests/scatter.rs` gates both
/// halves of the fix against that same closed-form null.
pub fn clump(seed: u64, x: f32, z: f32) -> f32 {
    let n = fbm(seed, CH_CLUMP, x, z, CLUMP_FREQ, CLUMP_OCTAVES);
    let t = (n * CLUMP_GAIN * 0.5 + 0.5).clamp(0.0, 1.0);
    let f = CLUMP_FLOOR + (1.0 - CLUMP_FLOOR) * t;
    // Squared, per `SPAWN.md` §9.4: the reference accepts a decor candidate
    // with probability `factor²`, so marginal ground thins out quadratically
    // and an edge reads as a gradient with a soft tail instead of a step at
    // the threshold. The same multiply here is what makes a grove edge ragged
    // rather than a contour line of the noise field.
    f * f * CLUMP_NORM
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

/// The land line: ground below this is water's edge, not somewhere a thing
/// stands or a road runs. Scatter has always used it; naming it is what let
/// the coast road share one definition instead of inventing a second.
pub const LAND_MIN_H: f32 = SEA_LEVEL + 0.6;

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

// --- The coast road (TERRAIN.md §1 stage 7) -------------------------------
//
// The road is the loot route: a ring offset inland from the coastline that
// pulls players out of their bases into a circulation loop, with barrel
// slots along it and no monument art. TERRAIN.md's stage 7–9 constraint
// block says the ring needs "something derived once from the seed and then
// queried", and warns off a raster. It turns out to need neither.
//
// The trick is to never ask "where is the ring", only "am I on it". The
// road's center line is the set of points exactly ROAD_INLAND_M inland of
// the shoreline, so a sample is on the road iff the shoreline crossing lies
// in a window around the point ROAD_INLAND_M seaward of it — and that is
// three `height` taps along the sample's own outward radial, no memo, no
// cap in limits.rs, no signature to thread through eight call sites, and
// the golden untouched. It also follows the wobble exactly rather than
// approximating it with control points.
//
// The one thing it assumes is that height falls monotonically across the
// ~10 m shoulder window. A shore that rises again inside 10 m would read as
// off-road; that is a coastline steeper than ROAD_MAX_GRADE, which is above
// CLIFF_SLOPE_RATIO, where scatter already vetoes and no player walks.

/// How far inland of the shoreline the road's center line runs, meters
/// (DECISIONS.md §open: coast road v0; TERRAIN.md §1 stage 7 "~40 m").
pub const ROAD_INLAND_M: f32 = 40.0;
/// Carriageway half-width, meters — the cleared surface (TERRAIN.md §6
/// "roads: 1 coast ring, ~4 m wide").
pub const ROAD_HALF_W: f32 = 2.0;
/// Shoulder half-width, meters: the barrel band, outside the carriageway.
pub const ROAD_SHOULDER_HALF_W: f32 = 5.0;
/// The radial bracket the ring may live in, meters from island center.
/// The shoreline sits at CONTINENT_RADIUS ± COAST_WOBBLE modulated by
/// relief; these bound it with margin, and double as the broad phase.
pub const ROAD_R_MIN: f32 = 600.0;
pub const ROAD_R_MAX: f32 = 1000.0;
/// The steepest shore the one-probe early-out will still find a crossing
/// on, as rise/run. Above CLIFF_SLOPE_RATIO on purpose: the road declining
/// to cross ground scatter itself vetoes costs nothing.
pub const ROAD_MAX_GRADE: f32 = 2.0;
/// Per-mille of shoulder cells that become barrel slots — the same rate
/// the beach row already washes barrels up at (`alpha_default`), so the
/// route and the shore agree (DECISIONS.md §open: coast road v0).
pub const ROAD_BARREL_PERMILLE: u16 = 250;

/// Where a point stands relative to the coast road.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum RoadBand {
    Off = 0,
    /// Outside the carriageway, inside the shoulder: the barrel band.
    Shoulder = 1,
    /// The cleared surface itself — scatter never occupies it.
    Carriageway = 2,
}

/// The coast road as a pure function of (seed, x, z) — no state, no memo,
/// so it costs the same in the server, the wasm client and the golden.
pub fn road_band(seed: u64, x: f32, z: f32) -> RoadBand {
    let c = ISLAND_SIZE * 0.5;
    let dx = x - c;
    let dz = z - c;
    let d = (dx * dx + dz * dz).sqrt();
    if !(ROAD_R_MIN..=ROAD_R_MAX).contains(&d) {
        return RoadBand::Off;
    }
    // d >= ROAD_R_MIN > 0, so the normalize is safe without a guard.
    let ux = dx / d;
    let uz = dz / d;
    // Where the shoreline would be if this sample were on the center line.
    let r = d + ROAD_INLAND_M;

    // One probe first. If the ground there is far from sea level, no
    // crossing can be inside the shoulder window without a grade steeper
    // than ROAD_MAX_GRADE, so most of the island answers in one tap.
    let hp = height(seed, c + ux * r, c + uz * r);
    if fabs(hp) > ROAD_SHOULDER_HALF_W * ROAD_MAX_GRADE {
        return RoadBand::Off;
    }

    // The probe's sign says which way the crossing lies, so the window only
    // costs one more tap per width tested, not two.
    let band = if hp > SEA_LEVEL {
        let w = ROAD_SHOULDER_HALF_W;
        if height(seed, c + ux * (r + ROAD_HALF_W), c + uz * (r + ROAD_HALF_W)) <= SEA_LEVEL {
            RoadBand::Carriageway
        } else if height(seed, c + ux * (r + w), c + uz * (r + w)) <= SEA_LEVEL {
            RoadBand::Shoulder
        } else {
            RoadBand::Off
        }
    } else {
        let w = ROAD_SHOULDER_HALF_W;
        if height(seed, c + ux * (r - ROAD_HALF_W), c + uz * (r - ROAD_HALF_W)) > SEA_LEVEL {
            RoadBand::Carriageway
        } else if height(seed, c + ux * (r - w), c + uz * (r - w)) > SEA_LEVEL {
            RoadBand::Shoulder
        } else {
            RoadBand::Off
        }
    };

    // Last, and only for candidates: the road has to be ON land. A radial
    // 40 m out of an inlet's far shore satisfies the window while the sample
    // itself sits in the water — measured, not hypothetical (the ring dipped
    // to 0.00 m before this guard). Costs one tap, and only on the ring.
    if band != RoadBand::Off && height(seed, x, z) < LAND_MIN_H {
        return RoadBand::Off;
    }
    band
}

// --- The haven pad (TERRAIN.md §1 stage 8) --------------------------------
//
// The one destination the coast road runs to, and the hook every later
// monument reuses: "carve pad + exclusion zone + scatter table". Stage 8 is
// explicitly a global argmax — score candidate sites on the road ring by
// flatness and coast distance, take the best — so unlike stage 7 it really
// is the memoized thing the constraint block anticipated. The memo is not a
// raster and not control points: it is one site, three floats wide.
//
// Where the block's guidance stops being enough is the CARVE. A carve is a
// write to `height`, and `height(seed, x, z)` is called from ~50 sites in
// four crates — `movement.rs`, `collide.rs`, `build.rs`, `deploy.rs` among
// them. Threading the memo through it is the whole change, not a detail of
// it, and it cannot be half-done: a client mesh that sees the pad and a
// collision path that does not is a player standing in the air. So this
// slice FINDS a flat site rather than MAKING one — the argmax scores
// flatness precisely so the eventual cut is small — and `haven` reports the
// relief it settled for, so the gate can say in meters how flat "found" got
// and whether the carve is optional or mandatory (DECISIONS.md §open).
//
// Cost: `HAVEN_CANDIDATES` bearings, each a bracket + `HAVEN_BISECT_ITERS`
// halvings + a `HAVEN_PROBES`-point rosette + the ring check chain below —
// bounded above by roughly 28,000 `height` taps and typically far under it,
// once, at world init and never in a tick (wall 2). The "under 1,000" this
// comment used to claim was wrong before the check chain was added: the
// march to the shoreline alone is up to 100 taps per bearing, and the judge
// measured 5,453 mean on the client's path. Bounded by
// `limits::MAX_HAVEN_CANDIDATES` (wall 4), and float-walled like everything
// else here, so native and wasm agree bit for bit (wall 1). The two callers
// that could pay it repeatedly hold it instead: `World` at init, and the
// client bridge memoizes it on the seed.
//
// The hook is three parts — "carve pad + exclusion zone + scatter table"
// (TERRAIN.md §1 stage 8) — and the third is the one the carve does not
// block. `HAVEN_CRATES` containers on a ring is that third part: it is what
// makes the destination pay more than the route to it, which is the defect
// the road left behind (`ROAD_BARREL_PERMILLE` is literally the beach row's
// own rate, so the loop paid what standing still paid). It also gives
// `content/loot.toml`'s `loot.crate` its first spawn site — that table was
// parsed, validated and hashed with nothing in the world able to produce
// its container. What it does NOT do is open one: no verb opens a container
// yet (`crates/content/src/validate.rs`), so the table is reachable content
// rather than reachable loot, and that half is the systems lane's.

/// Pad radius in meters: inside this, scatter places nothing. Sized to read
/// as a clearing rather than a gap — 32 m across clears ~12 scatter cells
/// and is four carriageways wide (knob, DECISIONS.md §open: haven pad v0).
pub const HAVEN_RADIUS_M: f32 = 16.0;
/// Bearings the argmax scores, evenly spaced around the island. Capped by
/// `limits::MAX_HAVEN_CANDIDATES` (knob, DECISIONS.md §open: haven pad v0).
pub const HAVEN_CANDIDATES: i32 = 64;
/// Rim samples per candidate footprint; with the center that is 9 taps.
/// Powers of two only — the yaw LUT is indexed by 256 / this (knob).
pub const HAVEN_PROBES: i32 = 8;
/// Coarse march step, meters, used to bracket the *first* shoreline
/// crossing seaward of a bearing before bisecting it. Bisecting the whole
/// 400 m bracket instead was measurably wrong: a radial that crosses water
/// more than once (an inlet, a channel behind an islet) has no single
/// crossing to converge on, and `tests/haven.rs` caught the argmax landing
/// 131 m off the best site on seed 1 because of it. Below the narrowest
/// land a coastline can be and still be one (knob).
pub const HAVEN_MARCH_M: f32 = 4.0;
/// Halvings of the bracketed crossing. 12 over a `HAVEN_MARCH_M` bracket
/// resolves it to under a millimeter — far inside `ROAD_HALF_W` (knob).
pub const HAVEN_BISECT_ITERS: i32 = 12;
/// How many meters of footprint relief one meter of elevation above the
/// land line is worth, when the two scores trade off. Below 1 because
/// flatness is the term that decides whether the pad needs a carve at all;
/// elevation only breaks near-ties toward the shore (knob).
pub const HAVEN_HEIGHT_W: f32 = 0.25;

/// Containers standing on the pad — the third of the monument hook, after
/// the exclusion zone and ahead of the carve. Not invented: it is the
/// reference `SpawnGroup`'s default `maxPopulation` (`reference/SPAWN.md`
/// §10), the one per-destination container count either document names
/// (knob, DECISIONS.md §open: haven crates v0).
pub const HAVEN_CRATES: i32 = 5;
/// Radius of the ring they stand on, meters. Bounded on both sides by
/// arithmetic rather than taste: above `CELL_SIZE * 1.5 / sin(180/N)` so no
/// two anchors can share a scatter cell (one cell holds one slot, so a
/// closer ring would silently drop a crate), and far enough inside
/// `HAVEN_RADIUS_M` to leave a walkable rim. `tests/haven.rs` measures the
/// separation it actually buys (knob, DECISIONS.md §open: haven crates v0).
pub const HAVEN_CRATE_R_M: f32 = 10.0;
/// Rotations of the ring a candidate site may try before it is refused.
///
/// The pad stands ON the road — that is the point of it — so a ring centred
/// on the pad crosses the carriageway at two bearings, and `tests/road.rs`
/// requires the carriageway clear so the loop stays walkable. Rotating the
/// ring is the only degree of freedom that fixes it without moving the pad,
/// shrinking the ring into its own cell, or exempting the pad from the road
/// rule. The alternative was to derive the road's local direction from the
/// pad's bearing and phase away from it analytically; that is an
/// approximation of a coastline that wobbles between 630 and 920 m of
/// radius, and the margin at 5 crates on a 10 m ring is 6.8 degrees of
/// road-direction error. Testing `road_band` at the anchor is exact and the
/// search is bounded, which is the trade `SPAWN.md` §2 records the reference
/// making (10,000 rejection-sampled attempts against a check chain, never a
/// closed form) (knob, DECISIONS.md §open: haven crates v0).
pub const HAVEN_PHASE_TRIES: i32 = 16;
/// LUT steps between tried phases. Derived, not chosen: the tries divide one
/// anchor gap (256 / `HAVEN_CRATES`) as evenly as integers allow.
pub const HAVEN_PHASE_STEP: i32 = 256 / HAVEN_CRATES / HAVEN_PHASE_TRIES;

/// Half the shelter's outer footprint, meters — the one number the sim knows
/// about the greybox standing on the pad, and the one the client's mesh is
/// gated against (`ci/haven_shelter.mjs`, the `PINE_MAX_R` pattern).
///
/// The sim does not care what the structure looks like; it cares that the
/// thing occupies its corner of the pad without reaching a container or the
/// rim, and that is a distance (knob, DECISIONS.md §open: haven shelter v0).
pub const HAVEN_SHELTER_HALF_M: f32 = 3.5;
/// How far off the pad center the shelter stands, meters.
///
/// **Not zero, and the road gate is why.** The pad center is the road's own
/// center line — that is how stage 8 places it — so a structure at the
/// center stands ON the carriageway, and `tests/road.rs` requires that
/// surface clear so the loop stays walkable. It caught this on the first
/// run. The correction is the one the reference already makes: a
/// destination sits BESIDE the road it is reached by, and the road runs
/// past it. Inside the container ring so the composition still reads as one
/// place (knob, DECISIONS.md §open: haven shelter v0).
pub const HAVEN_SHELTER_R_M: f32 = 6.5;
/// LUT steps from the container ring's phase to the first bearing the
/// shelter is tried on. Derived, not chosen: half an anchor gap, so the
/// structure stands in a gap between two containers rather than behind one.
pub const HAVEN_SHELTER_YAW_STEP: i32 = 256 / HAVEN_CRATES / 2;
/// Bearings the shelter may be tried on before the site is refused.
/// Derived, not chosen: one per gap in the container ring, because the gaps
/// are exactly the bearings that clear the containers by construction.
pub const HAVEN_SHELTER_TRIES: i32 = HAVEN_CRATES;

// Wall 4 at the definition, not in a test: the search is capped, and both
// counts must divide the 256-entry yaw LUT evenly or the bearings bunch.
// The crate count is exempt from the divisibility rule — it indexes the LUT
// by truncating division, which spreads the 1-index remainder over one
// bearing instead of bunching, and `tests/haven.rs` gates the separation
// that actually matters. What it is not exempt from is the ring fitting
// inside the pad and inside the broad phase `scatter` uses to find it.
const _: () = {
    assert!(HAVEN_CANDIDATES as usize <= crate::limits::MAX_HAVEN_CANDIDATES);
    assert!(HAVEN_CANDIDATES > 0 && 256 % HAVEN_CANDIDATES == 0);
    assert!(HAVEN_PROBES > 0 && 256 % HAVEN_PROBES == 0);
    assert!(HAVEN_CRATES > 0 && HAVEN_CRATES <= 256);
    // The tried phases must not run past one anchor gap: beyond that the
    // ring repeats and the extra tries are the same rings again.
    assert!(HAVEN_PHASE_TRIES > 0);
    assert!(HAVEN_PHASE_TRIES * HAVEN_PHASE_STEP <= 256 / HAVEN_CRATES);
    // A walkable rim: the ring is strictly inside the exclusion zone.
    assert!(HAVEN_CRATE_R_M > 0.0 && HAVEN_CRATE_R_M < HAVEN_RADIUS_M);
    // `scatter`'s broad phase tests |cell - haven cell| <= 2, which covers
    // every anchor iff the ring is inside two cells of the center.
    assert!(HAVEN_CRATE_R_M <= 2.0 * CELL_SIZE);
    // The shelter, footprint and all, stands on ground the exclusion zone
    // already cleared — otherwise a tree grows through a wall. 1.5 stands
    // in for √2 on the half-diagonal: larger, exact in binary, and no sqrt
    // in a const block.
    assert!(HAVEN_SHELTER_HALF_M > 0.0);
    assert!(HAVEN_SHELTER_R_M > 0.0);
    assert!(HAVEN_SHELTER_R_M + 1.5 * HAVEN_SHELTER_HALF_M < HAVEN_RADIUS_M);
    // `scatter`'s broad phase is the pad's cell plus two in each direction;
    // the shelter has to be inside it or the branch never fires.
    assert!(HAVEN_SHELTER_R_M <= 2.0 * CELL_SIZE);
    // Half an anchor gap, to within the truncation an odd crate count
    // forces: at 5 the gap is 51 LUT steps and half is 25.5, so the tried
    // bearing sits 0.5 steps (0.7 degrees) off the gap's center. Stated as
    // a bound rather than an equality because the equality is false, and a
    // false assert is worse than no assert.
    assert!(HAVEN_SHELTER_YAW_STEP > 0);
    assert!(2 * HAVEN_SHELTER_YAW_STEP <= 256 / HAVEN_CRATES);
    assert!(2 * HAVEN_SHELTER_YAW_STEP + 1 >= 256 / HAVEN_CRATES);
    // One tried bearing per gap: fewer would leave a gap unreachable,
    // more would try a bearing that is not a gap at all.
    assert!(HAVEN_SHELTER_TRIES == HAVEN_CRATES);
};

// ── Waystations: the second tier of destination ────────────────────────────
//
// `TERRAIN.md`'s own numbers table read `monuments | 0 — haven pad only`, and
// both judge reports of 2026-08-05 named the consequence rather than the
// count: "there is one place on the island worth walking to". A ring road
// with a single bead on it is a commute, not a circulation loop — a player
// leaves the base for one destination or not at all, and the two players who
// were supposed to meet on the way there have exactly one place to be.
//
// The fix is a SECOND TIER, not a second haven, and the whole of it comes out
// of work `haven()` already does and throws away. That search scores
// `HAVEN_CANDIDATES` bearings and keeps the argmin; the other 63 are sites
// that passed the same land and road checks and lost on flatness by
// centimetres. Taking the best of the losers costs no new shoreline march, no
// new bisect and no new `height` fan — one array, filled in the loop that was
// already running.
//
// **The tier is defined by the gradient, and the gradient is const-asserted.**
// A waystation is a smaller place holding fewer of the same containers, so
// `haven > waystation > shoulder` in containers per square metre, and the
// LESSER TIER IN AGGREGATE STILL DOES NOT OUTPAY THE ONE DESTINATION. That
// last clause is what fixes the count at two: it is the bound
// `WAYSTATIONS * WAYSTATION_CRATES < HAVEN_CRATES`, and at two crates apiece
// — the fewest that can still read as arranged rather than as one drawn
// barrel — it admits two sites and refuses three. The count is derived from
// the rule, not chosen; widening either factor fails the const block below
// rather than quietly making the haven the second-best place on the island.
//
// What this is NOT: it is not the carve (`height` has ~80 call sites in four
// crates, so a carve is a cross-lane change and still open, `TERRAIN.md` §7),
// and it is not new art. A waystation is `Occupant::CrateSlot` — the
// archetype the pad ring already ships — so no client, wire or protocol
// change carries this, and nothing here is in `state_hash`.

/// Lesser sites on the road ring. Derived, not chosen: the const block below
/// holds `WAYSTATIONS * WAYSTATION_CRATES < HAVEN_CRATES`, so the whole
/// second tier put together still pays less than the one destination
/// (knob, DECISIONS.md §open: waystations v0).
pub const WAYSTATIONS: usize = 2;
/// Containers on a waystation's ring. Two is the floor, not a taste: one
/// container is a drawn barrel with extra steps and reads as weather, and
/// `SPAWN.md` §6's point about a destination is that it reads as ARRANGED.
/// Two anchors is the smallest arrangement there is (knob).
pub const WAYSTATION_CRATES: i32 = 2;
/// Exclusion radius, meters.
///
/// **Derived from the gradient, and the first draft got it backwards** — which
/// is worth recording, because it type-checked, it looked obviously right, and
/// only writing the assert found it. At 10 m the lesser tier was DENSER than
/// the pad: two containers in 314 m² is 0.00637 per m² against the pad's five
/// in 804 m², 0.00622. The site with fewer crates was the better square metre,
/// so the gradient the whole tier exists to create pointed the wrong way, and
/// a player optimizing for loot-per-walk would have skipped the haven.
///
/// The floor is arithmetic: density stays below the pad's iff
/// `WAYSTATION_CRATES / R² < HAVEN_CRATES / HAVEN_RADIUS_M²`, so
/// `R > sqrt(2 × 256 / 5) = 10.12 m`. 11.0 clears it with margin, stays well
/// inside `HAVEN_RADIUS_M` so the lesser tier still reads as a smaller place
/// on sight, and the const block below asserts the inequality itself rather
/// than the number — widen the crate count and it is the ASSERT that fails,
/// not the design (knob, DECISIONS.md §open: waystations v0).
pub const WAYSTATION_RADIUS_M: f32 = 11.0;
/// Radius of the container ring, meters. Bounded on both sides by arithmetic,
/// the same way `HAVEN_CRATE_R_M` is. Below `CELL_SIZE`, so an anchor is
/// never more than one scatter cell from the site center and `scatter`'s
/// broad phase is complete rather than approximate. Above `0.75 * CELL_SIZE`,
/// so the two diametrically opposite anchors are `2 * R = 13 m` apart against
/// an 11.31 m cell diagonal and CANNOT share a cell — a shared cell would
/// silently drop a container, which is the one failure this shape can have
/// (knob, DECISIONS.md §open: waystations v0).
pub const WAYSTATION_CRATE_R_M: f32 = 6.5;
/// Ring rotations a candidate may try before it is refused, and the LUT step
/// between them. Same mechanism as `HAVEN_PHASE_TRIES` and for the same
/// reason: the site stands ON the road, so its ring crosses the carriageway
/// at two bearings and `tests/road.rs` requires that surface clear. The step
/// is derived — the tries divide one anchor gap (256 / `WAYSTATION_CRATES`)
/// evenly (knob).
pub const WAYSTATION_PHASE_TRIES: i32 = 16;
/// LUT steps between tried phases. Derived, not chosen.
pub const WAYSTATION_PHASE_STEP: i32 = 256 / WAYSTATION_CRATES / WAYSTATION_PHASE_TRIES;
/// Minimum center-to-center separation between any two sites, meters —
/// waystation to waystation and waystation to pad.
///
/// Derived from the ring it sits on rather than picked. Three sites spread
/// around the tightest ring the coast road can take (`ROAD_R_MIN`) are 120°
/// apart, and the chord of 120° at 600 m is 1039 m — so a floor of
/// `ROAD_R_MIN` itself is satisfiable by construction on the worst ring while
/// still forcing roughly-thirds spacing on every seed. It is also far outside
/// any exclusion zone, so "separate sites" is a statement about the walk
/// between them and not about their footprints (knob).
pub const WAYSTATION_MIN_SEP_M: f32 = ROAD_R_MIN;

// Wall 4 at the definition, as the haven block does it. Every one of these is
// a property of the shape rather than a preference, so a later pass that
// widens a number fails HERE instead of shipping a dropped container or a
// second-best haven.
const _: () = {
    // THE GRADIENT, half one: the whole lesser tier, added up, still pays
    // less than the one destination — the rule that fixes `WAYSTATIONS` at
    // two, and the reason a third site is a compile error rather than a
    // balance discussion.
    assert!(WAYSTATIONS as i32 * WAYSTATION_CRATES < HAVEN_CRATES);
    // THE GRADIENT, half two: and it pays less PER SQUARE METRE, so the pad
    // is not merely bigger but richer. `containers / (π r²)` with the π
    // cancelled off both sides, which is why this reads cross-multiplied. The
    // haven's own gate states the pad's edge over the road in exactly these
    // units (`tests/haven.rs`, 2.30× the shoulder), so all three tiers are
    // one comparable number and the middle one is genuinely in the middle.
    assert!(
        (WAYSTATION_CRATES as f32) * HAVEN_RADIUS_M * HAVEN_RADIUS_M
            < (HAVEN_CRATES as f32) * WAYSTATION_RADIUS_M * WAYSTATION_RADIUS_M
    );
    // A destination reads as arranged; one anchor is a barrel with a story.
    assert!(WAYSTATION_CRATES >= 2);
    // A smaller place, and a walkable rim inside it.
    assert!(WAYSTATION_RADIUS_M > 0.0 && WAYSTATION_RADIUS_M < HAVEN_RADIUS_M);
    assert!(WAYSTATION_CRATE_R_M > 0.0 && WAYSTATION_CRATE_R_M < WAYSTATION_RADIUS_M);
    // `scatter`'s broad phase is the site's cell plus ONE in each direction.
    // That is complete iff no anchor is a full cell from the center: a
    // displacement under `CELL_SIZE` can move a floor-divided index by at
    // most one, whatever the site's alignment inside its cell.
    assert!(WAYSTATION_CRATE_R_M < CELL_SIZE);
    // No two anchors in one cell. The pair is exactly diametric (256 /
    // `WAYSTATION_CRATES` is 128 LUT steps), so the separation is `2 * R`,
    // and 1.5 stands in for √2 on the cell diagonal — larger, exact in
    // binary, and no sqrt in a const block, the same substitution the shelter
    // block above makes.
    assert!(2.0 * WAYSTATION_CRATE_R_M > 1.5 * CELL_SIZE);
    // The phase search is capped and covers exactly one anchor gap: beyond
    // that the ring repeats and the extra tries are the same rings again.
    assert!(WAYSTATION_PHASE_TRIES > 0 && WAYSTATION_PHASE_STEP > 0);
    assert!(WAYSTATION_PHASE_TRIES * WAYSTATION_PHASE_STEP <= 256 / WAYSTATION_CRATES);
    // Separate places, not overlapping footprints — by a wide margin, so the
    // word "separation" means the walk and not the geometry.
    assert!(WAYSTATION_MIN_SEP_M > 2.0 * (HAVEN_RADIUS_M + WAYSTATION_RADIUS_M));
    // Every candidate the search can offer lies on the road ring, so a floor
    // above the ring's own diameter could never be met at all.
    assert!(WAYSTATION_MIN_SEP_M < 2.0 * ROAD_R_MIN);
};

/// One lesser site: position, the rotation its container pair stands at, and
/// whether the search found it at all.
///
/// `live` rather than a parked coordinate because `scatter` tests this
/// 65,536 times per island and a bool is the cheapest possible rejection —
/// and because "this seed's road ring had room for one site, not two" is a
/// real answer the gate should be able to read, not something to encode as a
/// position a million metres out to sea.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Waystation {
    pub x: f32,
    pub z: f32,
    pub y: f32,
    /// Rotation of the container pair, as a yaw-LUT index — carried for the
    /// same reason `Haven::phase` is: `waystation_crate` must be a pure
    /// function of the site, and the client, the server and the gate all ask
    /// for anchor `k` and have to be told the same place.
    pub phase: u8,
    pub live: bool,
}

impl Waystation {
    /// A site the search did not fill. Every field is inert; `live` is what
    /// every reader tests.
    pub const NONE: Waystation = Waystation {
        x: 0.0,
        z: 0.0,
        y: 0.0,
        phase: 0,
        live: false,
    };
}

/// The haven pad site: a pure function of the seed, resolved once.
///
/// `relief` is the max−min height over the scored footprint — the size of
/// the cut a carve would have to make. It is carried rather than recomputed
/// so the gate can re-derive it independently and compare.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Haven {
    pub x: f32,
    pub z: f32,
    pub y: f32,
    pub relief: f32,
    /// Rotation of the container ring, as a yaw-LUT index. Carried rather
    /// than recomputed because `haven_crate` must be a pure function of the
    /// pad — the client, the server and the gate all ask for anchor `k` and
    /// have to be told the same place.
    pub phase: u8,
    /// Outward bearing the shelter stands on, as a yaw-LUT index. Carried
    /// for the same reason `phase` is, and it costs more: resolving it reads
    /// `road_band`, and `scatter` runs 65,536 times an island.
    pub shelter: u8,
    /// The island's lesser destinations, the second tier of the same search.
    ///
    /// **They live inside `Haven` rather than beside it, and the reason is a
    /// lane rule rather than taste.** `scatter(seed, table, haven, cx, cz)` is
    /// called from `gather.rs`, `world.rs`, `bridge.rs` and their tests —
    /// files another lane owns and is editing in parallel — and on
    /// 2026-08-04 adding one parameter to this exact function blocked this
    /// lane for two passes when git merged three of the five call sites clean
    /// and they compiled green on both trunks. Widening a struct that only
    /// `terrain::haven` constructs moves no caller at all. So this field is
    /// the whole of "where the authored sites are", and `Haven` is now that
    /// answer rather than one pad: the pad is `x`/`z`, the lesser tier is
    /// here, and no signature moved.
    pub minor: [Waystation; WAYSTATIONS],
}

/// Max−min height over the pad footprint at (x, z): center plus a rim
/// rosette at `HAVEN_RADIUS_M`. The flatness term of the stage 8 score.
fn haven_relief(seed: u64, x: f32, z: f32) -> f32 {
    let h0 = height(seed, x, z);
    let mut lo = h0;
    let mut hi = h0;
    let step = (256 / HAVEN_PROBES) as u16;
    let mut j = 0i32;
    while j < HAVEN_PROBES {
        let (dx, dz) = crate::yaw_lut::yaw_dir((j as u16 * step) << 8);
        let h = height(seed, x + dx * HAVEN_RADIUS_M, z + dz * HAVEN_RADIUS_M);
        lo = lo.min(h);
        hi = hi.max(h);
        j += 1;
    }
    hi - lo
}

/// The first ring rotation at (x, z) every container can stand on, or
/// `None` if the site cannot hold its containers at any tried rotation.
///
/// Two conditions, both re-testing the real thing rather than a proxy for
/// it: every anchor is on land, and no anchor stands on the carriageway
/// (`tests/road.rs` requires that surface clear, and the pad is on the road
/// by construction — see `HAVEN_PHASE_TRIES`). Cheap test first, and the
/// anchor loop breaks on the first failure, so the common case is one pass.
fn haven_ring_phase(seed: u64, x: f32, z: f32) -> Option<u8> {
    let mut t = 0i32;
    while t < HAVEN_PHASE_TRIES {
        let phase = (t * HAVEN_PHASE_STEP) as u8;
        t += 1;
        let probe = Haven {
            x,
            z,
            y: 0.0,
            relief: 0.0,
            phase,
            shelter: 0,
            minor: [Waystation::NONE; WAYSTATIONS],
        };
        let mut k = 0i32;
        let mut ok = true;
        while k < HAVEN_CRATES {
            let (ax, az, _) = haven_crate(&probe, k);
            if height(seed, ax, az) < LAND_MIN_H || road_band(seed, ax, az) == RoadBand::Carriageway
            {
                ok = false;
                break;
            }
            k += 1;
        }
        if ok {
            return Some(phase);
        }
    }
    None
}

/// The first gap in the container ring the shelter can stand in, or `None`
/// if the site cannot hold it at any of them.
///
/// The tried bearings are the ring's own gaps — `HAVEN_SHELTER_YAW_STEP`
/// past each anchor — because a gap clears both its neighbouring containers
/// by construction, and picking bearings some other way would put the
/// distance check back in charge of a property the geometry already has.
///
/// Three conditions, the same posture as the ring's: on land, off the
/// carriageway (the pad's center IS the road's center line, so this is the
/// condition that moved the structure off center at all), and in a scatter
/// cell no container has taken. The third cannot be sized away — the
/// shelter and the containers are 6 m apart against an 11.3 m cell diagonal,
/// so whether they share a cell is a property of where the pad's grid
/// alignment fell, not of either radius. A shared cell would silently
/// delete one of the two. `reference/SPAWN.md` §5: refuse the position,
/// never patch the object.
fn haven_shelter_bearing(seed: u64, x: f32, z: f32, phase: u8) -> Option<u8> {
    let probe = Haven {
        x,
        z,
        y: 0.0,
        relief: 0.0,
        phase,
        shelter: 0,
        minor: [Waystation::NONE; WAYSTATIONS],
    };
    let mut t = 0i32;
    while t < HAVEN_SHELTER_TRIES {
        let bearing = ((t as u32 * 256) / HAVEN_CRATES as u32
            + phase as u32
            + HAVEN_SHELTER_YAW_STEP as u32) as u8;
        t += 1;
        let (dx, dz) = crate::yaw_lut::yaw_dir((bearing as u16) << 8);
        let sx = x + dx * HAVEN_SHELTER_R_M;
        let sz = z + dz * HAVEN_SHELTER_R_M;
        if height(seed, sx, sz) < LAND_MIN_H || road_band(seed, sx, sz) == RoadBand::Carriageway {
            continue;
        }
        let scx = (sx * (1.0 / CELL_SIZE)) as i32;
        let scz = (sz * (1.0 / CELL_SIZE)) as i32;
        let mut k = 0i32;
        let mut ok = true;
        while k < HAVEN_CRATES {
            let (ax, az, _) = haven_crate(&probe, k);
            if (ax * (1.0 / CELL_SIZE)) as i32 == scx && (az * (1.0 / CELL_SIZE)) as i32 == scz {
                ok = false;
                break;
            }
            k += 1;
        }
        if ok {
            return Some(bearing);
        }
    }
    None
}

/// Resolve the haven pad for a seed (TERRAIN.md §1 stage 8).
///
/// One candidate per bearing: bisect the shoreline crossing along that
/// radial, step `ROAD_INLAND_M` back inland — which is the road's own
/// definition of its center line, inverted, so the site lands on the ring
/// by construction and `road_band` confirms it rather than being trusted to
/// agree. Score = footprint relief + `HAVEN_HEIGHT_W` × height above the
/// land line, minimized; the scan runs bearings in ascending order and
/// takes a strict improvement, so ties go to the lowest index and the
/// result is order-independent.
///
/// Two fallbacks, both asserted unreachable by `tests/haven.rs` over 16
/// seeds — the same posture `World::spawn_pos_n` takes: the best site that
/// cleared the land line but not `road_band`, then the island center.
pub fn haven(seed: u64) -> Haven {
    let c = ISLAND_SIZE * 0.5;
    let inner = ROAD_R_MIN + ROAD_INLAND_M;
    let outer = ROAD_R_MAX + ROAD_INLAND_M;
    let bearing_step = (256 / HAVEN_CANDIDATES) as u16;

    let mut best: Option<Haven> = None;
    let mut best_score = 0.0f32;
    let mut relaxed: Option<Haven> = None;
    let mut relaxed_score = 0.0f32;
    // The second tier's candidate list: every on-ring site this scan scored,
    // as `(x, z, y, score)`. Fixed capacity, bounded by the same limit the
    // bearing count is (wall 4 — the cap check below can never bite, and it
    // is written anyway because a push on a bounded path carries one).
    let mut cand = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); crate::limits::MAX_HAVEN_CANDIDATES];
    let mut n_cand = 0usize;

    let mut i = 0i32;
    while i < HAVEN_CANDIDATES {
        let (dx, dz) = crate::yaw_lut::yaw_dir((i as u16 * bearing_step) << 8);
        i += 1;

        // March seaward to the FIRST crossing, then bisect that bracket.
        // "First" is the load-bearing word: the shoreline of a point on
        // land is the nearest water going out, and a radial may meet
        // several. Start inland — a bearing whose inner probe is already
        // wet has no shore in the bracket.
        if height(seed, c + dx * inner, c + dz * inner) <= SEA_LEVEL {
            continue;
        }
        let mut lo = inner;
        let mut hi = inner;
        while hi < outer {
            hi = (lo + HAVEN_MARCH_M).min(outer);
            if height(seed, c + dx * hi, c + dz * hi) <= SEA_LEVEL {
                break;
            }
            lo = hi;
        }
        if height(seed, c + dx * hi, c + dz * hi) > SEA_LEVEL {
            continue; // dry all the way out: no shore on this bearing
        }
        let mut k = 0i32;
        while k < HAVEN_BISECT_ITERS {
            let mid = (lo + hi) * 0.5;
            if height(seed, c + dx * mid, c + dz * mid) > SEA_LEVEL {
                lo = mid;
            } else {
                hi = mid;
            }
            k += 1;
        }

        // `lo` is the landward side of the crossing; the road's center line
        // is ROAD_INLAND_M in from it.
        let r = lo - ROAD_INLAND_M;
        let x = c + dx * r;
        let z = c + dz * r;
        let y = height(seed, x, z);
        if y < LAND_MIN_H {
            continue;
        }

        let relief = haven_relief(seed, x, z);
        let score = relief + HAVEN_HEIGHT_W * (y - LAND_MIN_H);

        // The second tier's candidates, recorded by the scan that was already
        // running — no extra march, no extra bisect, no extra `height` fan.
        //
        // Recorded HERE, ahead of the pad's own ring and shelter checks,
        // because a waystation needs neither of them: it stands two
        // containers on a smaller ring and no structure at all, so a site the
        // pad refuses for want of a shelter bearing is still a place. Its own
        // check chain is `waystation_ring_phase`, applied in `pick_minor`
        // below to the few sites that survive the separation floor rather
        // than to all 64 here.
        //
        // The road test is the pad's own, lifted a few lines earlier so both
        // tiers can read one answer. `road_band` is a pure function, so
        // asking it of more candidates cannot change what any of them says —
        // the pad's branch below now reads this value instead of recomputing
        // it, and its site is bit-identical to what it was.
        let on_road = road_band(seed, x, z) != RoadBand::Off;
        if on_road && n_cand < crate::limits::MAX_HAVEN_CANDIDATES {
            cand[n_cand] = (x, z, y, score);
            n_cand += 1;
        }

        // A site has to hold what stands on it, and `y >= LAND_MIN_H` above
        // tests one point — the center — which is a different question.
        // Seed 555555 puts a center at 0.69 m on a shore shelf whose whole
        // container ring sits under the land line at every radius tried
        // (measured 0.50 m at 6 m out; shrinking the ring cannot save it,
        // the shelf falls away in all directions). Flatness does not catch
        // it either — 1.75 m of relief is unremarkable.
        //
        // So this is the check chain `reference/SPAWN.md` §5 describes and
        // §9 told us to steal: refuse the position, do not patch the object
        // afterwards. It runs on the shipped anchor geometry rather than on
        // a radius of its own, so the constraint cannot drift away from the
        // thing it constrains.
        let phase = match haven_ring_phase(seed, x, z) {
            Some(p) => p,
            None => continue,
        };
        // Same chain, one link further along: a site that cannot stand its
        // structure anywhere is refused rather than shipped without one.
        // Ordered after the ring because the gaps it tries are the ring's.
        let shelter = match haven_shelter_bearing(seed, x, z, phase) {
            Some(b) => b,
            None => continue,
        };
        let site = Haven {
            x,
            z,
            y,
            relief,
            phase,
            shelter,
            minor: [Waystation::NONE; WAYSTATIONS],
        };

        if relaxed.is_none() || score < relaxed_score {
            relaxed = Some(site);
            relaxed_score = score;
        }
        if !on_road {
            continue;
        }
        if best.is_none() || score < best_score {
            best = Some(site);
            best_score = score;
        }
    }

    let mut pad = best.or(relaxed).unwrap_or(Haven {
        x: c,
        z: c,
        y: height(seed, c, c),
        relief: 0.0,
        phase: 0,
        shelter: 0,
        minor: [Waystation::NONE; WAYSTATIONS],
    });
    // The pad is resolved before the lesser tier is chosen, and that order is
    // the design: a waystation is defined as "far from the destination", so
    // the destination has to exist first. It also means nothing below can
    // move the pad, which is what keeps `tests/haven.rs` and the terrain
    // golden answering exactly what they answered before.
    pad.minor = pick_minor(seed, &pad, &cand[..n_cand]);
    pad
}

/// Choose the lesser tier out of the pad search's own scored candidates.
///
/// Greedy, `WAYSTATIONS` times: take the best-scoring candidate that clears
/// `WAYSTATION_MIN_SEP_M` from the pad and from every site already taken, and
/// can stand its container pair at some rotation. Greedy rather than an
/// argmin over subsets because the separation floor is a *constraint* and not
/// a term — there is no trade to make between "flat" and "far", and a site
/// that fails the floor is not a worse site, it is the same place twice.
///
/// **Deterministic by the same construction the pad uses**: the scan runs the
/// candidate array in ascending index order — which is ascending bearing —
/// and takes only a STRICT improvement, so ties go to the lowest bearing and
/// the answer does not depend on evaluation order. Nothing here reads a
/// clock, a map or a float outside the L1 set.
///
/// `waystation_ring_phase` is asked LAST because it is the only expensive
/// test in the chain — the cheap rejections (score, then two squared
/// distances) run first, so the ~110-tap ring check runs on the handful of
/// candidates that could actually win rather than on all 64.
///
/// A seed whose ring cannot hold a full tier gets a short one: the loop
/// breaks and the remaining entries stay `Waystation::NONE`, `live == false`.
/// `tests/waystation.rs` asserts a full tier on every seed it sweeps, so a
/// short one is a finding rather than a silent degradation.
fn pick_minor(seed: u64, pad: &Haven, cand: &[(f32, f32, f32, f32)]) -> [Waystation; WAYSTATIONS] {
    let mut out = [Waystation::NONE; WAYSTATIONS];
    let sep2 = WAYSTATION_MIN_SEP_M * WAYSTATION_MIN_SEP_M;
    let mut filled = 0usize;

    while filled < WAYSTATIONS {
        let mut take: Option<Waystation> = None;
        let mut take_score = 0.0f32;

        let mut i = 0usize;
        while i < cand.len() {
            let (x, z, y, score) = cand[i];
            i += 1;
            if take.is_some() && score >= take_score {
                continue;
            }
            let dx = x - pad.x;
            let dz = z - pad.z;
            if dx * dx + dz * dz < sep2 {
                continue;
            }
            let mut j = 0usize;
            let mut clear = true;
            while j < filled {
                let ex = x - out[j].x;
                let ez = z - out[j].z;
                if ex * ex + ez * ez < sep2 {
                    clear = false;
                    break;
                }
                j += 1;
            }
            if !clear {
                continue;
            }
            let phase = match waystation_ring_phase(seed, x, z) {
                Some(p) => p,
                None => continue,
            };
            take = Some(Waystation {
                x,
                z,
                y,
                phase,
                live: true,
            });
            take_score = score;
        }

        match take {
            Some(w) => {
                out[filled] = w;
                filled += 1;
            }
            None => break,
        }
    }
    out
}

/// The first rotation at (x, z) both containers can stand on, or `None` if
/// the site cannot hold them at any tried rotation.
///
/// The pad's `haven_ring_phase` with the pad's numbers swapped out, and
/// deliberately a separate function rather than a shared generic one: the two
/// tiers are allowed to diverge — a later POI kind with a different check
/// chain is the point of the hook — and a shared body would make the next
/// one's constraint a parameter of this one's.
fn waystation_ring_phase(seed: u64, x: f32, z: f32) -> Option<u8> {
    let mut t = 0i32;
    while t < WAYSTATION_PHASE_TRIES {
        let phase = (t * WAYSTATION_PHASE_STEP) as u8;
        t += 1;
        let probe = Waystation {
            x,
            z,
            y: 0.0,
            phase,
            live: true,
        };
        let mut k = 0i32;
        let mut ok = true;
        while k < WAYSTATION_CRATES {
            let (ax, az, _) = waystation_crate(&probe, k);
            if height(seed, ax, az) < LAND_MIN_H || road_band(seed, ax, az) == RoadBand::Carriageway
            {
                ok = false;
                break;
            }
            k += 1;
        }
        if ok {
            return Some(phase);
        }
    }
    None
}

/// Anchor `k` of a waystation's container pair: position and the yaw that
/// faces it back at the site center — `haven_crate`'s convention exactly,
/// because a second convention is how the shelter's yaw shipped wrong once
/// already (see `haven_shelter`).
pub fn waystation_crate(ws: &Waystation, k: i32) -> (f32, f32, u8) {
    let idx = ((k as u32 * 256) / WAYSTATION_CRATES as u32 + ws.phase as u32) as u16 & 0xFF;
    let (dx, dz) = crate::yaw_lut::yaw_dir(idx << 8);
    (
        ws.x + dx * WAYSTATION_CRATE_R_M,
        ws.z + dz * WAYSTATION_CRATE_R_M,
        // Facing in: half a turn from the outward bearing it stands on.
        (idx as u8).wrapping_add(128),
    )
}

/// True if (x, z) stands inside any live waystation's exclusion zone.
/// Squared compare throughout, `in_haven`'s posture and `SPAWN.md` §9.4's
/// point: the squared form is the acceptance test, not an optimization of one.
pub fn in_waystation(haven: &Haven, x: f32, z: f32) -> bool {
    let mut w = 0usize;
    while w < WAYSTATIONS {
        let ws = &haven.minor[w];
        w += 1;
        if !ws.live {
            continue;
        }
        let dx = x - ws.x;
        let dz = z - ws.z;
        if dx * dx + dz * dz < WAYSTATION_RADIUS_M * WAYSTATION_RADIUS_M {
            return true;
        }
    }
    false
}

/// True if this seed filled every authored site the island is supposed to
/// carry: the pad, plus all `WAYSTATIONS` of the lesser tier.
///
/// `pick_minor` leaves a `Waystation` dead when no candidate on the ring
/// clears `WAYSTATION_MIN_SEP_M`, which is the right call — a site placed
/// too close to another is worse than a site missing. But it is silent:
/// `tests/waystation.rs` refuses a short tier over 16 seeds and
/// `test_golden_covers_authored_sites` over 3, while a shard boots whatever
/// seed `shard.toml` names, so a seed outside those 19 can ship an island a
/// third smaller with no counter, event or log line
/// (`pass-20260805-074623-02-judge.md` fix 1).
///
/// Exported here and called at boot by the shard, which is the systems
/// lane's file: sim-core can say what "complete" means, and only the server
/// can decide that an incomplete island is a refusal to start.
pub fn sites_complete(haven: &Haven) -> bool {
    let mut w = 0usize;
    while w < WAYSTATIONS {
        if !haven.minor[w].live {
            return false;
        }
        w += 1;
    }
    true
}

/// How many of this seed's authored sites are live, pad included — the
/// number a boot-time refusal wants to print next to `WAYSTATIONS + 1`.
/// The pad always exists (`haven` returns the best candidate on the ring
/// unconditionally), so the floor is 1.
pub fn sites_live(haven: &Haven) -> u32 {
    let mut n = 1u32;
    let mut w = 0usize;
    while w < WAYSTATIONS {
        if haven.minor[w].live {
            n += 1;
        }
        w += 1;
    }
    n
}

/// True if (x, z) stands inside the pad — the exclusion zone. Squared
/// compare, no sqrt (and `SPAWN.md` §9.4's point: the squared form is the
/// acceptance test, not an optimization of one).
pub fn in_haven(haven: &Haven, x: f32, z: f32) -> bool {
    let dx = x - haven.x;
    let dz = z - haven.z;
    dx * dx + dz * dz < HAVEN_RADIUS_M * HAVEN_RADIUS_M
}

/// Anchor `k` of the pad's container ring: position and the yaw that faces
/// it back at the pad center.
///
/// Authored positions, not a draw — and that is the point rather than a
/// shortcut. `SPAWN.md` §6 records that the reference's `SpawnGroup` hangs
/// its loot on a set of hand-placed child spawn points with no spacing rule
/// at all, so the ring is the faithful analogue: a destination should read
/// as arranged, where scatter reads as weather. It also sidesteps §9.3's
/// complaint about our per-cell independence in the one place where noise
/// would be actively wrong.
///
/// The bearing truncates 256/`HAVEN_CRATES`, so the last gap carries the
/// remainder; at 5 that is 52 LUT steps against 51, which is 2% of one gap
/// and below the jitter every other slot already gets.
pub fn haven_crate(haven: &Haven, k: i32) -> (f32, f32, u8) {
    let idx = ((k as u32 * 256) / HAVEN_CRATES as u32 + haven.phase as u32) as u16 & 0xFF;
    let (dx, dz) = crate::yaw_lut::yaw_dir(idx << 8);
    (
        haven.x + dx * HAVEN_CRATE_R_M,
        haven.z + dz * HAVEN_CRATE_R_M,
        // Facing in: half a turn from the outward bearing it stands on.
        (idx as u8).wrapping_add(128),
    )
}

/// The pad's greybox: position and the yaw it faces, a pure function of the
/// pad exactly as `haven_crate` is.
///
/// It stands at the center — not at an authored offset — because the center
/// is the only place on the pad with room. The packing is arithmetic, not
/// taste: five containers on a 10 m ring inside a 32 m circle, with every
/// pair required to clear the 11.31 m cell diagonal, already spends the
/// circle's budget (five 5.66 m disks is 503 m² of the pad's 804 m², and a
/// sixth anywhere inside the ring is closer than the diagonal to two of
/// them). So the structure takes the one cell the ring encircles, and
/// `haven_ring_phase` is what keeps that cell its own.
///
/// It stands in a gap in the container ring, `HAVEN_SHELTER_R_M` off the
/// pad center, on the bearing `haven_shelter_bearing` accepted — beside the
/// road rather than across it, with the containers to either side.
///
/// **The yaw is the INWARD facing, back at the pad center, exactly as
/// `haven_crate`'s is.** One convention for both, deliberately: the first
/// draft made this one an OUTWARD bearing and the doorway landed exactly on
/// container 3 at every seed, because `phase + 25` in bearing space is
/// `phase + 153 + 128` in facing space and the two collide identically. No
/// type saw it — the arity was right, the field was right, and the value
/// meant something else. CLAUDE.md's positional-payload trap in three lines.
/// What caught it was `tests/haven.rs` asserting the ANGLE rather than the
/// number, which is the only kind of assert that can.
pub fn haven_shelter(haven: &Haven) -> (f32, f32, u8) {
    let (dx, dz) = crate::yaw_lut::yaw_dir((haven.shelter as u16) << 8);
    (
        haven.x + dx * HAVEN_SHELTER_R_M,
        haven.z + dz * HAVEN_SHELTER_R_M,
        // Facing in: half a turn from the outward bearing it stands on.
        haven.shelter.wrapping_add(128),
    )
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
    // 8 is deliberately skipped: the client's archetype table is indexed by
    // this enum and its slot 8 is the felled-pine stump, which is a
    // CONSEQUENCE of an occupant rather than one (`web/src/props.js`).
    // Taking 8 here would land every pad crate in the stump pool — no
    // compile error, no golden move, no clippy wall, and a crate drawn as a
    // tree stump. Skipping the index costs nothing and keeps the two tables
    // aligned by construction, which is the only kind of alignment that
    // survives (CLAUDE.md's positional-payload trap).
    CrateSlot = 9,
    /// The greybox standing at the pad's center. One slot, one structure:
    /// a scatter cell holds a single occupant and the pad has no room for a
    /// second authored anchor (five containers on a 10 m ring already spend
    /// the 32 m circle's packing budget at the 11.31 m cell diagonal), so
    /// the whole building is one archetype's mesh rather than a kit of
    /// wall-sized slots. `web/src/props.js` index 10.
    HavenShelter = 10,
    /// The lesser tier's container — a waystation's, where the pad's is
    /// `CrateSlot`. A separate variant rather than a reused one because the
    /// two tiers have to differ in what they PAY, and a container's KIND is
    /// the only thing a loot table is selected by (`bake.rs`: the name is
    /// content, the index is code). While both rings placed `CrateSlot`, a
    /// waystation crate drew `loot.crate` and the lesser tier paid exactly
    /// what the destination paid, with nothing but geometry between them.
    /// Content calls this one `cache` (`content/loot.toml`,
    /// `loot::LOOT_CACHE`); `web/src/props.js` index 11.
    CacheSlot = 11,
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
/// (TERRAIN.md §1 stage 9). Slope, water, road and haven veto — except on
/// the pad's own `HAVEN_CRATES` + 1 authored cells (the container ring and
/// the shelter at its center), where the pad PRODUCES a slot instead of
/// clearing one, at a position no draw contributed to.
///
/// The haven arrives as a parameter rather than being resolved here on
/// purpose: `haven` costs ~1,000 `height` taps and `scatter` is called
/// 65,536 times for one island. Callers resolve it once and hold it —
/// `World` at init, the bridge per chunk batch.
pub fn scatter(seed: u64, table: &ScatterTable, haven: &Haven, cell_x: i32, cell_z: i32) -> Slot {
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

    // The pad's own occupants, ahead of every other rule including the hash.
    // A crate is placed because the pad is there, not because a cell rolled
    // one, so nothing below may veto it and nothing below may move it: its
    // position is the anchor's, not the cell's jitter.
    //
    // Broad phase first — five anchor tests would otherwise run on all
    // 65,536 cells. The pad's own cell plus two in each direction covers the
    // ring by the const assert on `HAVEN_CRATE_R_M`; the world is positive
    // in both axes here (the ring bracket is 600..1000 m from a center at
    // half of `ISLAND_SIZE`), so the truncating cast is a floor.
    let hcx = (haven.x * (1.0 / CELL_SIZE)) as i32;
    let hcz = (haven.z * (1.0 / CELL_SIZE)) as i32;
    if (cell_x - hcx).abs() <= 2 && (cell_z - hcz).abs() <= 2 {
        // The shelter ahead of the containers, so that if the two ever did
        // want the same cell the loss would be a CONTAINER — which
        // `tests/haven.rs` counts islandwide and would fail loudly on —
        // rather than the structure, which nothing counts. The failure a
        // one-slot-per-cell rule can actually have is a silent drop, so the
        // ordering picks which drop is audible. `haven_ring_phase` is what
        // stops it happening at all.
        let (sx, sz, syaw) = haven_shelter(haven);
        if (sx * (1.0 / CELL_SIZE)) as i32 == cell_x && (sz * (1.0 / CELL_SIZE)) as i32 == cell_z {
            return Slot {
                occupant: Occupant::HavenShelter,
                x: sx,
                y: height(seed, sx, sz),
                z: sz,
                yaw: syaw,
                scale: 1.0,
            };
        }
        let mut k = 0i32;
        while k < HAVEN_CRATES {
            let (ax, az, yaw) = haven_crate(haven, k);
            k += 1;
            if (ax * (1.0 / CELL_SIZE)) as i32 != cell_x
                || (az * (1.0 / CELL_SIZE)) as i32 != cell_z
            {
                continue;
            }
            // First anchor wins, so a ring that ever put two in one cell
            // would drop the second rather than fight over it — which is
            // exactly what `tests/haven.rs` counts, because a silent drop
            // is the failure this arrangement can actually have.
            return Slot {
                occupant: Occupant::CrateSlot,
                x: ax,
                y: height(seed, ax, az),
                z: az,
                yaw,
                // Authored, not drawn: a monument's containers are placed,
                // and a size wobble would read as scatter.
                scale: 1.0,
            };
        }
    }

    // The lesser tier's own anchors, on the same terms and after the pad's:
    // authored, unvetoable, and first-anchor-wins. Ordered second so that if
    // two zones could ever want one cell the loss would be a WAYSTATION
    // crate, which `tests/waystation.rs` counts islandwide — the ordering
    // picks which drop is audible, exactly as the shelter/container ordering
    // above does. `WAYSTATION_MIN_SEP_M` is what stops it happening at all.
    //
    // The broad phase is ±1 cell, not the pad's ±2, because the const block
    // holds `WAYSTATION_CRATE_R_M < CELL_SIZE`: a displacement under one cell
    // moves a floor-divided index by at most one, whatever the site's
    // alignment inside its own cell. That is a proof, not a margin.
    let mut w = 0usize;
    while w < WAYSTATIONS {
        let ws = &haven.minor[w];
        w += 1;
        if !ws.live {
            continue;
        }
        let wcx = (ws.x * (1.0 / CELL_SIZE)) as i32;
        let wcz = (ws.z * (1.0 / CELL_SIZE)) as i32;
        if (cell_x - wcx).abs() > 1 || (cell_z - wcz).abs() > 1 {
            continue;
        }
        let mut k = 0i32;
        while k < WAYSTATION_CRATES {
            let (ax, az, yaw) = waystation_crate(ws, k);
            k += 1;
            if (ax * (1.0 / CELL_SIZE)) as i32 != cell_x
                || (az * (1.0 / CELL_SIZE)) as i32 != cell_z
            {
                continue;
            }
            return Slot {
                // `CacheSlot`, not the pad's `CrateSlot`: the tier below has
                // to pay less, and the container's kind is the only thing a
                // loot table is selected by. Same authored terms as the pad's
                // — unvetoable, first-anchor-wins, no scale wobble.
                occupant: Occupant::CacheSlot,
                x: ax,
                y: height(seed, ax, az),
                z: az,
                yaw,
                scale: 1.0,
            };
        }
    }

    let h = cell_hash(seed, cell_x, cell_z, CH_SCATTER);

    // Jittered position first: vetoes apply where the thing would stand.
    let jx = ((h >> 16) & 0x3F) as f32 * (6.0 / 63.0) - 3.0;
    let jz = ((h >> 22) & 0x3F) as f32 * (6.0 / 63.0) - 3.0;
    let x = cell_x as f32 * CELL_SIZE + 4.0 + jx;
    let z = cell_z as f32 * CELL_SIZE + 4.0 + jz;

    let hy = height(seed, x, z);
    if hy < LAND_MIN_H || slope(seed, x, z) > CLIFF_SLOPE_RATIO {
        return none;
    }

    // The pad clears before the road does, because the pad sits ON the road
    // and the shoulder rule below would otherwise line the destination with
    // the same barrels as the route to it (TERRAIN.md §1 stage 8).
    if in_haven(haven, x, z) || in_waystation(haven, x, z) {
        return none;
    }

    // The coast road (TERRAIN.md §1 stage 7) vetoes ahead of the table: the
    // carriageway stays clear so the loop is walkable, and the shoulder
    // draws barrels off its own bits so the route is worth walking.
    let mut occupant = Occupant::None;
    match road_band(seed, x, z) {
        RoadBand::Carriageway => return none,
        RoadBand::Shoulder => {
            if (((h >> 44) % 1000) as u16) < ROAD_BARREL_PERMILLE {
                occupant = Occupant::BarrelSlot;
            }
        }
        RoadBand::Off => {}
    }

    if occupant == Occupant::None {
        let row = &table.weights[biome(hy, moisture(seed, x, z)) as usize];
        // The grove/clearing field scales the whole row, not the tree entry
        // alone: a clearing is a clearing, not a clearing with the rocks
        // left standing in it. It also leaves the mix a biome draws
        // untouched, so the content pass (M1) still owns composition and
        // this owns only how much of it stands where.
        //
        // Deliberately below the road and the pad: a shoulder barrel is the
        // road's own rate (`ROAD_BARREL_PERMILLE`) and a pad crate is
        // authored, and neither is weather. Only the biome draw is.
        let g = clump(seed, x, z);
        let roll = (h % 1000) as u16;
        let mut acc = 0u16;
        for (i, w) in row.iter().enumerate() {
            acc += floor_i32(*w as f32 * g).clamp(0, 1000) as u16;
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

// ── Ground clutter (ART.md rule 4) ─────────────────────────────────────────
//
// `ART.md` §1 calls the near ground "the single largest structural difference
// between our frames and the references", and rule 4 makes it checkable: any
// visible ground patch larger than ~3 m² inside 15 m carries scatter. The 8 m
// scatter grid cannot answer that at any weight — two adjacent cells at full
// occupancy still leave ~60 m² of bare turf between their two props. This is
// the layer BELOW it: tufts, pebbles, twigs and shards on a sub-metre grid,
// with no gameplay meaning and no sim state. Worldgen POTENTIAL, exactly like
// a `Slot` — resolved by whoever draws it, never stored, never in a snapshot.
//
// Two properties make it gateable arithmetic rather than a shader.
//
//  1. THE MIX IS THE SPLAT — not a second table beside it. `splat_from` below
//     is the ground material's own four identity weights, lifted out of
//     `web/src/terrainWorker.js` so both sides evaluate ONE function. The
//     fields-pack failure this closes is named in its rejection list:
//     "geometry and shading claim the same feature but evaluate different
//     functions". Sandy ground grows pebbles, grass grows tufts, forest litter
//     grows twigs, rock grows shards, and the population cannot drift from the
//     surface it stands on because there is nothing to drift from.
//     `ci/splat_parity.mjs` holds the two languages to it.
//  2. THE WEIGHTS SUM TO 255 ON LAND, so every land cell yields an element and
//     coverage is TOTAL by construction. Rule 4's 3 m² then reduces to a
//     property of the grid alone: a disc of radius `CLUTTER_CELL_M * √2`
//     contains a whole cell wherever it is centred, so the largest bare disc
//     is 0.905 m — 2.57 m², inside the rule with margin. `tests/clutter.rs`
//     MEASURES the largest empty disc rather than trusting that argument.
//
// What this is not: it is not collision (nothing here has a volume, and a
// tuft you stop at would be a bug), not gathered, not networked, and not in
// `state_hash`. A shard that never draws behaves identically without it.

/// Clutter tile edge in meters — the streaming unit, sized so the client can
/// hold a ring of them inside the ~30 m band ART.md rule 4 is about rather
/// than paying for a 64 m terrain chunk's worth of grass to reach 15 m.
pub const CLUTTER_TILE_M: f32 = 16.0;
/// Clutter cell edge in meters: the stratum of the jittered grid, and the
/// only number rule 4's guarantee depends on (`CLUTTER_CELL_M * √2` is the
/// largest disc that can miss every cell). 0.64 divides 16.0 exactly 25 ways,
/// so a tile is a whole number of cells and no cell straddles two tiles.
pub const CLUTTER_CELL_M: f32 = 0.64;
/// Cells per tile side (16.0 / 0.64, exact).
pub const CLUTTER_CELLS_PER_TILE: i32 = 25;
/// Elements one tile can produce — the fill buffer's cap, and total coverage
/// means the cap is also the typical count, not a rare peak.
pub const CLUTTER_PER_TILE: usize = 625;
/// Clutter cells per island side (2048 / 0.64).
pub const CLUTTER_CELLS_PER_SIDE: i32 = 3200;

// The splat bands — the retired palette's hard thresholds with a ramp hung
// around each (DECISIONS.md §open, materials v0). These are the SAME numbers
// `web/src/terrainWorker.js` shipped; they moved here rather than being
// copied, and `ci/splat_parity.mjs` fails if the two ever disagree.
const SPLAT_BEACH_BAND: (f32, f32) = (1.0, 3.0);
const SPLAT_ALPINE_BAND: (f32, f32) = (44.0, 60.0);
const SPLAT_MOIST_BAND: (f32, f32) = (0.01, 0.09);
/// tan(50°) — the sim's cliff threshold, to the two decimals the worker's
/// copy carried. Kept distinct from `CLIFF_SLOPE_RATIO` on purpose: this one
/// is a shading band edge that has always been 1.19, and silently promoting
/// it to the collision constant's precision would move every ground pixel.
const SPLAT_CLIFF: f32 = 1.19;
const SPLAT_CLIFF_BAND: (f32, f32) = (SPLAT_CLIFF * 0.8, SPLAT_CLIFF * 1.2);

/// Smoothstep, written out — the worker's `ramp`, which is where the ground's
/// four identities get their soft edges instead of `biome()`'s hard ones.
#[inline]
fn ramp(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The four ground identity weights — sand · grass · forest litter · rock —
/// as bytes summing to ~255, from the three channels `biome()` decides on.
/// The ground material blends its textures by these and, from this pass, the
/// clutter population draws its kind from them.
pub fn splat_from(h: f32, moist: f32, slope: f32) -> [u8; 4] {
    let sand = 1.0 - ramp(SPLAT_BEACH_BAND.0, SPLAT_BEACH_BAND.1, h);
    let alpine = ramp(SPLAT_ALPINE_BAND.0, SPLAT_ALPINE_BAND.1, h);
    let wood = ramp(SPLAT_MOIST_BAND.0, SPLAT_MOIST_BAND.1, moist);
    let cliff = ramp(SPLAT_CLIFF_BAND.0, SPLAT_CLIFF_BAND.1, slope);
    let land = 1.0 - sand;
    let mut w = [
        sand,
        land * (1.0 - alpine) * (1.0 - wood),
        land * (1.0 - alpine) * wood,
        land * alpine,
    ];
    // The cliff mask FORCES rock (TERRAIN.md §4) — the one veto that
    // overrides the biome rather than blending with it.
    w[0] += -w[0] * cliff;
    w[1] += -w[1] * cliff;
    w[2] += -w[2] * cliff;
    w[3] += (1.0 - w[3]) * cliff;
    let sum = w[0] + w[1] + w[2] + w[3];
    let k = 255.0 / if sum > 0.0 { sum } else { 1.0 };
    [
        floor_i32(w[0] * k + 0.5).clamp(0, 255) as u8,
        floor_i32(w[1] * k + 0.5).clamp(0, 255) as u8,
        floor_i32(w[2] * k + 0.5).clamp(0, 255) as u8,
        floor_i32(w[3] * k + 0.5).clamp(0, 255) as u8,
    ]
}

/// `splat_from` at a world position, resolving the three channels itself.
/// The client's terrain worker already holds all three per vertex and calls
/// the law directly; this is for callers that hold only (x, z).
pub fn splat(seed: u64, x: f32, z: f32) -> [u8; 4] {
    splat_from(height(seed, x, z), moisture(seed, x, z), slope(seed, x, z))
}

/// What one clutter cell grew. Index order is the splat's channel order, so
/// `kind as usize - 1` indexes the weight that produced it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Clutter {
    None = 0,
    /// Sand channel: grit, shells, small water-worn stones.
    Pebble = 1,
    /// Grass channel: the standing blades ART.md §1 is about.
    Tuft = 2,
    /// Forest-litter channel: fallen needles, sticks, cones.
    Twig = 3,
    /// Rock channel: angular scree off the cliff mask.
    Shard = 4,
}

/// One resolved clutter element. Deliberately the same shape as `Slot` minus
/// the things clutter does not have (an occupant identity the sim knows, a
/// volume, a harvest state).
#[derive(Clone, Copy, Debug)]
pub struct ClutterElem {
    pub kind: Clutter,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: u8,
    /// Visual scale in [0.75, 1.25] — a wider band than a `Slot`'s [0.9, 1.1]
    /// because a uniform tuft field reads as a printed texture, and at this
    /// size nothing downstream measures against the scale.
    pub scale: f32,
}

/// The empty cell — water, or off the island.
pub const CLUTTER_NONE: ClutterElem = ClutterElem {
    kind: Clutter::None,
    x: 0.0,
    y: 0.0,
    z: 0.0,
    yaw: 0,
    scale: 1.0,
};

/// One hash draw decides a clutter cell's jitter, kind, yaw and scale.
///
/// Full-stratum jitter (the offset spans the whole cell, not a fraction of
/// it) is what keeps the grid invisible while leaving the coverage guarantee
/// intact: the element never leaves its own cell, so a disc that contains a
/// cell contains an element, whatever the draw did.
pub fn clutter_cell(seed: u64, cell_x: i32, cell_z: i32) -> ClutterElem {
    if !(0..CLUTTER_CELLS_PER_SIDE).contains(&cell_x)
        || !(0..CLUTTER_CELLS_PER_SIDE).contains(&cell_z)
    {
        return CLUTTER_NONE;
    }
    let h = cell_hash(seed, cell_x, cell_z, CH_CLUTTER);

    let jx = ((h >> 16) & 0xFF) as f32 * (CLUTTER_CELL_M / 255.0);
    let jz = ((h >> 24) & 0xFF) as f32 * (CLUTTER_CELL_M / 255.0);
    let x = cell_x as f32 * CLUTTER_CELL_M + jx;
    let z = cell_z as f32 * CLUTTER_CELL_M + jz;

    let y = height(seed, x, z);
    if y < LAND_MIN_H {
        return CLUTTER_NONE;
    }

    // The carriageway keeps its grit and loses its grass. This is the one
    // place clutter overrides the splat, and it is the same override the
    // scatter grid already makes for the same reason: a road that grows a
    // continuous lawn is not a road, and `TERRAIN.md` §1 stage 7 wants the
    // ring legible from a distance without a road material existing yet.
    let kind = if road_band(seed, x, z) == RoadBand::Carriageway {
        Clutter::Pebble
    } else {
        let w = splat_from(y, moisture(seed, x, z), slope(seed, x, z));
        let mut total: u32 = 0;
        for v in w.iter() {
            total += *v as u32;
        }
        if total == 0 {
            // Unreachable while the weights are normalized to 255, but a
            // rounding change upstream must degrade to a pebble rather than
            // punching a hole in the coverage guarantee.
            Clutter::Pebble
        } else {
            let roll = ((h >> 32) as u32) % total;
            let mut acc = 0u32;
            let mut k = Clutter::Shard;
            for (i, v) in w.iter().enumerate() {
                acc += *v as u32;
                if roll < acc {
                    k = match i {
                        0 => Clutter::Pebble,
                        1 => Clutter::Tuft,
                        2 => Clutter::Twig,
                        _ => Clutter::Shard,
                    };
                    break;
                }
            }
            k
        }
    };

    ClutterElem {
        kind,
        x,
        y,
        z,
        yaw: ((h >> 40) & 0xFF) as u8,
        scale: 0.75 + ((h >> 48) & 0xFF) as f32 * (0.5 / 255.0),
    }
}

/// Fill one tile's elements into a caller-owned buffer, returning the count.
///
/// Caller-owned because sim-core allocates nothing: the bridge holds one
/// static `CLUTTER_PER_TILE` buffer and the client reads it in place, the
/// same arrangement `terrain_fill_slots` already uses for the scatter grid.
/// A buffer shorter than `CLUTTER_PER_TILE` is filled to its own length and
/// the rest of the tile is dropped, so a short buffer is a thinner field and
/// never an overrun.
pub fn clutter_fill(seed: u64, tile_x: i32, tile_z: i32, out: &mut [ClutterElem]) -> usize {
    let cx0 = tile_x * CLUTTER_CELLS_PER_TILE;
    let cz0 = tile_z * CLUTTER_CELLS_PER_TILE;
    let mut n = 0usize;
    for j in 0..CLUTTER_CELLS_PER_TILE {
        for i in 0..CLUTTER_CELLS_PER_TILE {
            if n >= out.len() {
                return n;
            }
            let e = clutter_cell(seed, cx0 + i, cz0 + j);
            if e.kind != Clutter::None {
                out[n] = e;
                n += 1;
            }
        }
    }
    n
}

// ── Occupant volume ────────────────────────────────────────────────────────
//
// What a slot does to a body that walks into it. Four passes built a road, a
// pad, a container ring and a building and every one of them was
// walk-through; `occupy.rs` is the consumer that closed that, so
// `TERRAIN.md` §1 stage 6's "wood, cover, low visibility" is now something a
// body can actually stand behind.
//
// The volume belongs here rather than in `collide.rs` for the same reason the
// occupant enum does: whether a pine has a trunk you stop at is a property of
// the pine, not of the movement code. This module owns the shapes and the
// per-slot test; the movement path owns which slots it asks about and how it
// indexes them. That seam is deliberate — `scatter` costs a `height` fan, a
// `moisture`, a `clump` and a `road_band` per cell, so a movement step must
// never re-derive slots by calling it, and nothing here invites that.
// `occupy.rs` answers it with a memo rather than a re-derivation, which is
// exact because `scatter` is a pure function of `(seed, cell)`.
//
// Cylinders, not boxes. Every scattered archetype is radially symmetric about
// its own axis except the crate, and a cylinder needs no yaw and therefore no
// trig — the L1 wall bans libm outright, and the yaw LUT would otherwise have
// to be dragged into a test that runs per body per tick. The one archetype
// that genuinely is a box list with a hole in it, the shelter, is NOT given a
// volume here; see `OCCUPANT_R_M`'s note on index 10.

/// The shelter's fourteen boxes in the slot's LOCAL frame, as
/// `[cx, cy, cz, sx, sy, sz]` — center and FULL size, meters, ground at
/// y = 0 and the doorway on +Z.
///
/// A row-for-row mirror of `web/src/props.js`'s `HAVEN_SHELTER_PARTS` minus
/// the part name, and `ci/haven_shelter.mjs` holds the two equal number for
/// number. That gate is the whole reason this is a table rather than a shape:
/// a building is the one occupant whose volume cannot be checked by eye
/// against its mesh, because the interesting part is the hole in it. Drift
/// here is not a wrong radius, it is a doorway the client draws and the
/// server walls off — the positional-payload failure `CLAUDE.md` records,
/// with the two halves in different languages.
///
/// Local +Z is the direction `yaw_lut::yaw_dir` hands back for the slot's
/// yaw, which is the same convention `terrain.js` rotates the mesh by, so the
/// two frames agree by construction rather than by comment.
pub const SHELTER_BOXES: [[f32; 6]; 14] = [
    [0.0, -0.6, 0.0, 7.0, 1.6, 7.0], // plinth  — FLOOR, see SHELTER_FLOOR_IX
    [0.0, 2.0, -2.9, 6.2, 3.6, 0.4], // wall-back
    [-2.9, 2.0, 0.0, 0.4, 3.6, 5.4], // wall-left
    [2.9, 2.0, 0.0, 0.4, 3.6, 5.4],  // wall-right
    [-2.15, 2.0, 2.9, 1.9, 3.6, 0.4], // jamb-left
    [2.15, 2.0, 2.9, 1.9, 3.6, 0.4], // jamb-right
    [0.0, 3.4, 2.9, 2.4, 0.8, 0.4],  // lintel
    [0.0, 4.0, 0.0, 7.0, 0.4, 7.0],  // roof
    [-2.9, 2.8, -2.9, 0.7, 5.2, 0.7], // post-nw
    [2.9, 2.8, -2.9, 0.7, 5.2, 0.7], // post-ne
    [-2.9, 2.8, 2.9, 0.7, 5.2, 0.7], // post-sw
    [2.9, 2.8, 2.9, 0.7, 5.2, 0.7],  // post-se
    [0.0, 6.4, -1.4, 2.2, 4.8, 2.2], // tower
    [0.0, 9.0, -1.4, 2.8, 0.4, 2.8], // tower-cap
];

/// The plinth's row. It is FLOOR, not wall, and `slot_blocks` skips it.
///
/// A 7 × 7 box spanning y ∈ [−1.4, 0.2] would otherwise seal the building
/// from the outside: a body standing on pad ground has its feet at 0.0, which
/// overlaps that interval, so the "walls" a player met would be the floor's
/// rim and the doorway would never be reachable. Standing ON a thing is the
/// ground query's business and this predicate is the wall query.
///
/// The exception is structural rather than a threshold, so it cannot drift
/// onto a wall: the const block below asserts this box's top is exactly the
/// lowest bottom of every other box — the plinth is, by measurement, the
/// thing the walls stand on. Widen it into a wall and it stops being the
/// floor line and the build stops.
///
/// **What this costs today:** nothing here makes a body stand on the plinth
/// either, so at 0.2 m the floor reads as a kerb a player sinks into rather
/// than steps onto. That is the ground-query half of the same seam and it
/// lives in the systems lane's `collide.rs`; this table is what it will need.
pub const SHELTER_FLOOR_IX: usize = 0;

/// Bounding radius of the shelter's boxes about the slot, meters — the
/// broad-phase reject before the fourteen-box loop, and the value
/// `OCCUPANT_R_M` publishes for `HavenShelter`.
///
/// The widest boxes are the 7 × 7 plinth and roof, so this is their
/// half-diagonal, 3.5·√2, rounded UP: erring outward costs one extra box
/// loop, erring inward drops a wall. The const block proves the rounding went
/// the right way by squared compare, which needs no `sqrt` in a const.
pub const SHELTER_CORNER_R_M: f32 = 4.9498;

/// Height of the shelter's tallest point above its own ground, meters —
/// `props.js`'s `HAVEN_SHELTER_PEAK`, and `OCCUPANT_TOP_M`'s row 10.
pub const SHELTER_PEAK_M: f32 = 9.2;

/// `|v|` in a const context, which `f32::abs` is not. Sign flip only — no
/// libm, nothing outside the L1 float set.
const fn abs_const(v: f32) -> f32 {
    if v < 0.0 {
        -v
    } else {
        v
    }
}

/// Blocking radius per `Occupant`, meters, at a slot `scale` of 1.0.
///
/// Read off the client's authored geometry (`web/src/props.js` `ARCHETYPES`)
/// rather than chosen, because the defect being fixed is walking through
/// drawn geometry: the rule is the mesh's maximum horizontal extent, so the
/// server never lets a body reach a triangle the client is rendering. Erring
/// outward blocks a little air at a dodecahedron's flats; erring inward is
/// the bug. (knob, DECISIONS.md §open: occupant volume v0.)
///
/// Indexed by `Occupant as usize`, so the array is 11 long and two entries
/// are structural rather than tuned:
///
/// - **8 is the client's felled-pine stump**, which is not a sim occupant at
///   all — the enum skips the discriminant and this table keeps the hole, for
///   the reason the enum states: the two tables stay aligned by construction
///   or they drift silently.
/// - **10, the haven shelter, is a BROAD PHASE, not the volume.** Its real
///   volume is `SHELTER_BOXES` — a wall list with a doorway in it, the one
///   shape a radius cannot express, because a cylinder either seals the
///   entrance or blocks nothing. This row is the bounding circle that rejects
///   the fourteen-box loop cheaply, so it is the only row that is deliberately
///   wider than what it blocks: inside it, `slot_blocks` asks the boxes.
///
/// The bush is the third zero and the only one that is a design call: a bush
/// you cannot push through is a wall you cannot see over, which is worse than
/// no bush. It reads as cover and costs nothing to cross, which is what the
/// reference does with the same prop.
///
/// Every row is held to the mesh by `ci/occupant_volume.mjs`, which measures
/// the vertex buffer `ARCHETYPES` actually builds and sandwiches each row
/// between the mesh's widest horizontal extent below the blocking top and the
/// mesh's own bound. Nothing in the Rust workspace can see a triangle, so the
/// asserts below prove only that this file agrees with itself. The gate found
/// the `CrateSlot` row inward: it read 0.68 against a measured half-diagonal
/// of 0.680074, which is the bug this doc names, so it is 0.6801 now.
pub const OCCUPANT_R_M: [f32; 12] = [
    0.0,                // None
    0.26,               // Tree — the TRUNK, not the canopy: `CylinderGeometry(0.13, 0.26)`
    1.0,                // StoneNode  — DodecahedronGeometry(1.0)
    1.0,                // MetalNode  — DodecahedronGeometry(1.0)
    1.0,                // SulfurNode — DodecahedronGeometry(1.0)
    0.0,                // Bush — deliberately passable
    1.5,                // Rock — DodecahedronGeometry(1.5)
    0.45,               // BarrelSlot — CylinderGeometry(0.45, 0.45, 0.95)
    0.0,                // 8: the client's stump. Not a sim occupant; the hole is the point.
    0.6801,             // CrateSlot — BoxGeometry(1.1, 0.8) half-diagonal, 0.55/0.4 in xz
    SHELTER_CORNER_R_M, // HavenShelter — broad phase; SHELTER_BOXES is the volume
    0.5701,             // CacheSlot — BoxGeometry(0.9, 0.55, 0.7) half-diagonal, 0.45/0.35 in xz
];

/// How high above the slot's own ground each occupant blocks, meters, at a
/// slot `scale` of 1.0.
///
/// Also read off `ARCHETYPES`, where a row's `lift` is the mesh's half-height
/// and therefore the offset that sets it ON the ground: a top is `lift` plus
/// the geometry's half-extent in y. The nodes are lifted less than their own
/// radius on purpose — a dodecahedron of radius 1 lifted 0.5 is buried half a
/// meter so it reads as embedded rather than dropped — so their tops are 1.5
/// and not 2.0.
///
/// The tree stops at the trunk's height, not the crown's: you walk *under* a
/// canopy, and a body is 1.7 m against a 5.7 m trunk, so the distinction
/// costs nothing today and is the correct shape when something flies or a
/// tree falls. (knob, DECISIONS.md §open: occupant volume v0.)
pub const OCCUPANT_TOP_M: [f32; 12] = [
    0.0,            // None
    5.7,            // Tree — PINE_TRUNK_H
    1.5,            // StoneNode  — lift 0.5 + radius 1.0
    1.5,            // MetalNode
    1.5,            // SulfurNode
    0.0,            // Bush
    2.05,           // Rock — lift 0.55 + radius 1.5
    0.975,          // BarrelSlot — lift 0.5 + half-height 0.475
    0.0,            // 8: the stump
    0.8,            // CrateSlot — lift 0.4 + half-height 0.4
    SHELTER_PEAK_M, // HavenShelter — tower-cap at 9.0 + 0.2
    0.55,           // CacheSlot — lift 0.275 + half-height 0.275
];

/// Widest scale `scatter` can hand a slot. The draw is `0.9 + u8 * (0.2/255)`,
/// so this is the exact supremum and not a rounded one.
pub const SLOT_SCALE_MAX: f32 = 1.1;

/// Cells either side of a body's own that can hold a slot touching it.
///
/// One, and the const block below proves it rather than asserting it by eye.
/// The invariant is that **every slot lies inside its own cell** — a drawn
/// slot by its ±3 m jitter, an authored one because `scatter` only returns it
/// for the cell its position falls in — so a body and a slot are each within
/// half a cell of their own centers, and two cells more than one apart are
/// `CELL_SIZE` beyond touching. While the widest reach any slot has,
/// `max(OCCUPANT_R_M) * SLOT_SCALE_MAX + CAPSULE_RADIUS_M`, stays under
/// `CELL_SIZE`, a 3×3 neighbourhood is not a heuristic — it is complete.
///
/// The shelter is the widest thing in that max at `SHELTER_CORNER_R_M`, and
/// it is the reason the bound is stated as the general one: it eats 5.85 m of
/// the 8 m on its own.
///
/// Published so the movement path does not have to re-derive it, and so that
/// widening an occupant past the margin fails HERE, at the definition, rather
/// than as bodies clipping through the far side of a boulder.
pub const OCCUPANT_PROBE_CELLS: i32 = 1;

/// The volume of an occupant as `(radius, top)`, both meters at a slot
/// `scale` of 1.0 — and the LAW, where the two tables above are its published
/// view for the gates and tests that read them.
///
/// This is an exhaustive `match` rather than an index for a reason that cost
/// a real bug. The const block used to assert `OCCUPANT_R_M.len() == 11` and
/// `HavenShelter as usize == len() - 1` and call itself the guard against
/// adding an occupant without a volume — but **both of those hold on an enum
/// edit and only fail on a table edit**, which is the opposite of what the
/// comment claimed. Adding `Foo = 11` left every assert green, the build
/// succeeded, and the first `Foo` slot probed indexed `OCCUPANT_R_M[11]` and
/// panicked. Not a hypothetical: `examples/terrain_stats.rs` carried a
/// `[0u32; 10]` bucket array through the `HavenShelter = 10` commit and
/// crashed on the first haven cell of every seed, exactly this way.
///
/// A `match` cannot be left behind, because a new variant makes it
/// non-exhaustive and the build stops at the definition — which is the point.
/// It also takes the last unchecked index off the sim path: `slot_blocks`
/// runs per body per tick and now cannot panic on an out-of-range occupant,
/// because it no longer indexes anything.
pub const fn occupant_volume(o: Occupant) -> (f32, f32) {
    match o {
        Occupant::None => (0.0, 0.0),
        Occupant::Tree => (0.26, 5.7),
        Occupant::StoneNode => (1.0, 1.5),
        Occupant::MetalNode => (1.0, 1.5),
        Occupant::SulfurNode => (1.0, 1.5),
        Occupant::Bush => (0.0, 0.0),
        Occupant::Rock => (1.5, 2.05),
        Occupant::BarrelSlot => (0.45, 0.975),
        Occupant::CrateSlot => (0.6801, 0.8),
        Occupant::HavenShelter => (SHELTER_CORNER_R_M, SHELTER_PEAK_M),
        Occupant::CacheSlot => (0.5701, 0.55),
    }
}

const _: () = {
    // The published tables ARE the match, row for row. Written out rather
    // than looped because a loop would need the variant list this file is
    // trying not to keep twice; here the compiler checks the pairing and the
    // match checks the completeness.
    assert!(OCCUPANT_R_M.len() == 12 && OCCUPANT_TOP_M.len() == 12);
    // Index 8 is the client's stump and has no variant, so it is the one row
    // the match cannot speak for; it is a hole and stays zero.
    assert!(OCCUPANT_R_M[8] == 0.0 && OCCUPANT_TOP_M[8] == 0.0);
    assert!(occupant_volume(Occupant::None).0 == OCCUPANT_R_M[0]);
    assert!(occupant_volume(Occupant::Tree).0 == OCCUPANT_R_M[1]);
    assert!(occupant_volume(Occupant::StoneNode).0 == OCCUPANT_R_M[2]);
    assert!(occupant_volume(Occupant::MetalNode).0 == OCCUPANT_R_M[3]);
    assert!(occupant_volume(Occupant::SulfurNode).0 == OCCUPANT_R_M[4]);
    assert!(occupant_volume(Occupant::Bush).0 == OCCUPANT_R_M[5]);
    assert!(occupant_volume(Occupant::Rock).0 == OCCUPANT_R_M[6]);
    assert!(occupant_volume(Occupant::BarrelSlot).0 == OCCUPANT_R_M[7]);
    assert!(occupant_volume(Occupant::CrateSlot).0 == OCCUPANT_R_M[9]);
    assert!(occupant_volume(Occupant::HavenShelter).0 == OCCUPANT_R_M[10]);
    assert!(occupant_volume(Occupant::CacheSlot).0 == OCCUPANT_R_M[11]);
    assert!(occupant_volume(Occupant::None).1 == OCCUPANT_TOP_M[0]);
    assert!(occupant_volume(Occupant::Tree).1 == OCCUPANT_TOP_M[1]);
    assert!(occupant_volume(Occupant::StoneNode).1 == OCCUPANT_TOP_M[2]);
    assert!(occupant_volume(Occupant::MetalNode).1 == OCCUPANT_TOP_M[3]);
    assert!(occupant_volume(Occupant::SulfurNode).1 == OCCUPANT_TOP_M[4]);
    assert!(occupant_volume(Occupant::Bush).1 == OCCUPANT_TOP_M[5]);
    assert!(occupant_volume(Occupant::Rock).1 == OCCUPANT_TOP_M[6]);
    assert!(occupant_volume(Occupant::BarrelSlot).1 == OCCUPANT_TOP_M[7]);
    assert!(occupant_volume(Occupant::CrateSlot).1 == OCCUPANT_TOP_M[9]);
    assert!(occupant_volume(Occupant::HavenShelter).1 == OCCUPANT_TOP_M[10]);
    assert!(occupant_volume(Occupant::CacheSlot).1 == OCCUPANT_TOP_M[11]);
    // The lesser tier's container is the lesser silhouette, and it is a
    // structural claim rather than a taste one: the two tiers must be
    // distinguishable at the range either is legible from, and a player who
    // cannot tell them apart cannot make the detour decision the gradient
    // exists to create.
    assert!(OCCUPANT_R_M[11] < OCCUPANT_R_M[9] && OCCUPANT_TOP_M[11] < OCCUPANT_TOP_M[9]);

    // --- the shelter's boxes ------------------------------------------
    //
    // The floor exception is structural: the plinth's top is exactly the
    // lowest bottom of every other box, which is what "the walls stand on
    // it" means as a number. Thicken the plinth into something a body should
    // meet and it stops being the floor line, and this stops the build
    // rather than silently sealing the doorway from outside.
    let floor_top = SHELTER_BOXES[SHELTER_FLOOR_IX][1] + SHELTER_BOXES[SHELTER_FLOOR_IX][4] * 0.5;
    let mut lowest_wall = f32::MAX;
    let mut b = 0;
    while b < SHELTER_BOXES.len() {
        // Every box has positive size in all three axes, or it is not a box.
        assert!(
            SHELTER_BOXES[b][3] > 0.0 && SHELTER_BOXES[b][4] > 0.0 && SHELTER_BOXES[b][5] > 0.0
        );
        if b != SHELTER_FLOOR_IX {
            let bottom = SHELTER_BOXES[b][1] - SHELTER_BOXES[b][4] * 0.5;
            if bottom < lowest_wall {
                lowest_wall = bottom;
            }
        }
        b += 1;
    }
    // Tolerance because the two sides are different float paths to the same
    // 0.2 m (`-0.6 + 1.6/2` against `2.0 - 3.6/2`), not because the rule is
    // approximate: 0.1 mm is four orders below the centimetres any real edit
    // to the plinth or the wall base would move it.
    assert!(abs_const(floor_top - lowest_wall) < 1.0e-4);

    // The published radius really does contain every box's far corner, and
    // the rounding went OUTWARD. Squared, because `sqrt` is not const — and
    // because the squared form is the acceptance test rather than an
    // optimization of one (`reference/SPAWN.md` §9.4).
    let mut c = 0;
    while c < SHELTER_BOXES.len() {
        let ex = abs_const(SHELTER_BOXES[c][0]) + SHELTER_BOXES[c][3] * 0.5;
        let ez = abs_const(SHELTER_BOXES[c][2]) + SHELTER_BOXES[c][5] * 0.5;
        assert!(ex * ex + ez * ez <= SHELTER_CORNER_R_M * SHELTER_CORNER_R_M);
        c += 1;
    }
    // And the peak is the tallest box's top, not a number beside it.
    let mut peak = 0.0f32;
    let mut p = 0;
    while p < SHELTER_BOXES.len() {
        let top = SHELTER_BOXES[p][1] + SHELTER_BOXES[p][4] * 0.5;
        if top > peak {
            peak = top;
        }
        p += 1;
    }
    assert!(peak == SHELTER_PEAK_M);

    // A volume is a radius AND a height, or it is neither. A radius with no
    // height is a body that stops at nothing; a height with no radius is a
    // shape with no width. The pairing is what `tests/solid.rs` walks.
    let mut i = 0;
    while i < OCCUPANT_R_M.len() {
        assert!((OCCUPANT_R_M[i] > 0.0) == (OCCUPANT_TOP_M[i] > 0.0));
        assert!(OCCUPANT_R_M[i] >= 0.0 && OCCUPANT_TOP_M[i] >= 0.0);
        i += 1;
    }
    // The 3×3 probe is complete, not merely usual. `Rock` is the widest at
    // 1.5; at the widest scale and with a capsule on top that is 2.05 m,
    // against an 8 m cell. Written as the general bound so that raising a
    // radius past the margin fails the build.
    let mut widest = 0.0f32;
    let mut j = 0;
    while j < OCCUPANT_R_M.len() {
        if OCCUPANT_R_M[j] > widest {
            widest = OCCUPANT_R_M[j];
        }
        j += 1;
    }
    assert!(widest * SLOT_SCALE_MAX + crate::collide::CAPSULE_RADIUS_M < CELL_SIZE);
    assert!(OCCUPANT_PROBE_CELLS >= 1);
};

/// Does the slot stop a capsule standing at (`x`, `z`) with its feet at
/// `feet_y`?
///
/// Pure, one slot, no seed and no world: the caller has already resolved the
/// slot, which is the whole point of the seam — `scatter` is far too
/// expensive to run inside a movement step, and a signature that took a seed
/// would invite exactly that.
///
/// Squared distance, so there is no `sqrt` on the path at all.
///
/// The vertical test is an INTERVAL OVERLAP, not a ceiling, and the first
/// draft got that wrong in a way worth recording because it type-checks and
/// reads fine: `feet_y < slot.y + top` alone makes a boulder on a clifftop
/// block a body standing at sea level forty metres beneath it, since a body
/// below the top is "not above" it. A slot occupies `[slot.y, slot.y + top]`
/// and a body occupies `[feet_y, feet_y + capsule_h]`; they stop each other
/// only where those overlap.
///
/// Both ends are half-open at the top and closed at the bottom, which puts
/// the two degenerate cases on the side that matches what a player sees:
/// feet exactly at the top of a crate are standing ON it (the ground query's
/// business, not this one's), and a slot whose base is exactly at the body's
/// head height is overhead clearance rather than a wall. Below the slot's own
/// ground there is nothing to test — that volume is inside the terrain, and
/// the terrain already stops you.
pub fn slot_blocks(
    slot: &Slot,
    x: f32,
    z: f32,
    feet_y: f32,
    capsule_r: f32,
    capsule_h: f32,
) -> bool {
    let (r, top) = occupant_volume(slot.occupant);
    if r <= 0.0 {
        return false;
    }
    if feet_y >= slot.y + top * slot.scale || feet_y + capsule_h <= slot.y {
        return false;
    }
    // Broad phase, and for every occupant but one it is also the answer.
    let reach = r * slot.scale + capsule_r;
    let dx = x - slot.x;
    let dz = z - slot.z;
    if dx * dx + dz * dz >= reach * reach {
        return false;
    }
    if slot.occupant == Occupant::HavenShelter {
        return shelter_blocks(slot, dx, dz, feet_y, capsule_r, capsule_h);
    }
    true
}

/// The fourteen-box narrow phase, called only after the shelter's bounding
/// circle has already accepted. `dx`/`dz` are the query's offset from the
/// slot in WORLD axes; this rotates them into the building's frame.
///
/// The horizontal test is a cylinder against an axis-aligned rectangle done
/// the exact way — clamp the point into the rectangle and measure to the
/// clamped point — rather than by growing the rectangle by `capsule_r`, which
/// would round the corners the wrong way and stop a body that is diagonally
/// past a jamb. Squared throughout, so there is still no `sqrt` on the path.
fn shelter_blocks(
    slot: &Slot,
    dx: f32,
    dz: f32,
    feet_y: f32,
    capsule_r: f32,
    capsule_h: f32,
) -> bool {
    // World → local. `yaw_dir` hands back (sin, cos) for the slot's yaw and
    // local +Z is that direction, so local +X is (cos, −sin); this is the
    // inverse of that basis, which needs no trig and no matrix.
    let (s, c) = crate::yaw_lut::yaw_dir((slot.yaw as u16) << 8);
    let lx = dx * c - dz * s;
    let lz = dx * s + dz * c;

    let head_y = feet_y + capsule_h;
    let mut i = 0;
    while i < SHELTER_BOXES.len() {
        let b = &SHELTER_BOXES[i];
        let is_floor = i == SHELTER_FLOOR_IX;
        i += 1;
        if is_floor {
            continue; // floor, not wall — see SHELTER_FLOOR_IX
        }
        let hx = b[3] * 0.5 * slot.scale;
        let hy = b[4] * 0.5 * slot.scale;
        let hz = b[5] * 0.5 * slot.scale;
        let cy = slot.y + b[1] * slot.scale;
        // Same interval overlap as above, per box: a lintel you walk under
        // and a roof you walk beneath must not stop you.
        if feet_y >= cy + hy || head_y <= cy - hy {
            continue;
        }
        let cx = b[0] * slot.scale;
        let cz = b[2] * slot.scale;
        let qx = (lx - cx).clamp(-hx, hx);
        let qz = (lz - cz).clamp(-hz, hz);
        let ex = lx - cx - qx;
        let ez = lz - cz - qz;
        if ex * ex + ez * ez < capsule_r * capsule_r {
            return true;
        }
    }
    false
}
