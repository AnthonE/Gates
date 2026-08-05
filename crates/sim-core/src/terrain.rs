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
        };

        if relaxed.is_none() || score < relaxed_score {
            relaxed = Some(site);
            relaxed_score = score;
        }
        if road_band(seed, x, z) == RoadBand::Off {
            continue;
        }
        if best.is_none() || score < best_score {
            best = Some(site);
            best_score = score;
        }
    }

    best.or(relaxed).unwrap_or(Haven {
        x: c,
        z: c,
        y: height(seed, c, c),
        relief: 0.0,
        phase: 0,
        shelter: 0,
    })
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
    if in_haven(haven, x, z) {
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

// ── Occupant volume ────────────────────────────────────────────────────────
//
// What a slot does to a body that walks into it. Four passes built a road, a
// pad, a container ring and a building, and every one of them is walk-through:
// `collide.rs` and `movement.rs` contain zero references to `Occupant`, so
// `TERRAIN.md` §1 stage 6 asks the forest for "wood, cover, low visibility"
// and cover does not exist — there is nothing on this island you can break
// line of sight on.
//
// The volume belongs here rather than in `collide.rs` for the same reason the
// occupant enum does: whether a pine has a trunk you stop at is a property of
// the pine, not of the movement code. This module owns the shapes and the
// per-slot test; the movement path owns which slots it asks about and how it
// indexes them. That seam is deliberate — `scatter` costs a `height` fan, a
// `moisture`, a `clump` and a `road_band` per cell, so a movement step must
// never re-derive slots by calling it, and nothing here invites that.
//
// Cylinders, not boxes. Every scattered archetype is radially symmetric about
// its own axis except the crate, and a cylinder needs no yaw and therefore no
// trig — the L1 wall bans libm outright, and the yaw LUT would otherwise have
// to be dragged into a test that runs per body per tick. The one archetype
// that genuinely is a box list with a hole in it, the shelter, is NOT given a
// volume here; see `OCCUPANT_R_M`'s note on index 10.

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
/// - **10, the haven shelter, is 0.0 and that is DEFERRED, not "walk through
///   a building on purpose".** Its volume is a wall list with a doorway in it,
///   which is the one shape a radius cannot express — a cylinder either seals
///   the entrance or blocks nothing. Doing it properly means the sim owning a
///   box list that must not drift from `props.js`'s fourteen boxes, plus the
///   gate that keeps the two equal, and that is its own slice. Until then the
///   deliberate zero is pinned by `tests/solid.rs` so it cannot be mistaken
///   for a tuned value.
///
/// The bush is the third zero and the only one that is a design call: a bush
/// you cannot push through is a wall you cannot see over, which is worse than
/// no bush. It reads as cover and costs nothing to cross, which is what the
/// reference does with the same prop.
pub const OCCUPANT_R_M: [f32; 11] = [
    0.0,  // None
    0.26, // Tree — the TRUNK, not the canopy: `CylinderGeometry(0.13, 0.26)`
    1.0,  // StoneNode  — DodecahedronGeometry(1.0)
    1.0,  // MetalNode  — DodecahedronGeometry(1.0)
    1.0,  // SulfurNode — DodecahedronGeometry(1.0)
    0.0,  // Bush — deliberately passable
    1.5,  // Rock — DodecahedronGeometry(1.5)
    0.45, // BarrelSlot — CylinderGeometry(0.45, 0.45, 0.95)
    0.0,  // 8: the client's stump. Not a sim occupant; the hole is the point.
    0.68, // CrateSlot — BoxGeometry(1.1, 0.8) half-diagonal, 0.55/0.4 in xz
    0.0,  // HavenShelter — DEFERRED, see above
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
pub const OCCUPANT_TOP_M: [f32; 11] = [
    0.0,  // None
    5.7,  // Tree — PINE_TRUNK_H
    1.5,  // StoneNode  — lift 0.5 + radius 1.0
    1.5,  // MetalNode
    1.5,  // SulfurNode
    0.0,  // Bush
    2.05, // Rock — lift 0.55 + radius 1.5
    0.95, // BarrelSlot — lift 0.5 + half-height 0.475, rounded down to the mesh
    0.0,  // 8: the stump
    0.8,  // CrateSlot — lift 0.4 + half-height 0.4
    0.0,  // HavenShelter — DEFERRED
];

/// Widest scale `scatter` can hand a slot. The draw is `0.9 + u8 * (0.2/255)`,
/// so this is the exact supremum and not a rounded one.
pub const SLOT_SCALE_MAX: f32 = 1.1;

/// Cells either side of a body's own that can hold a slot touching it.
///
/// One, and the const block below proves it rather than asserting it by
/// eye: a slot's jitter is ±3 m about its cell's center, so every slot lies
/// strictly inside its own cell, and the widest reach any slot has is
/// `max(OCCUPANT_R_M) * SLOT_SCALE_MAX + CAPSULE_RADIUS_M`. While that stays
/// under `CELL_SIZE` a 3×3 neighbourhood is not a heuristic — it is complete.
///
/// Published so the movement path does not have to re-derive it, and so that
/// widening an occupant past the margin fails HERE, at the definition, rather
/// than as bodies clipping through the far side of a boulder.
pub const OCCUPANT_PROBE_CELLS: i32 = 1;

const _: () = {
    // The table is indexed by `Occupant as usize` and the largest
    // discriminant is `HavenShelter = 10`. If an occupant is ever added
    // above it, this stops the build instead of indexing out of bounds in
    // the movement path.
    assert!(OCCUPANT_R_M.len() == 11);
    assert!(OCCUPANT_TOP_M.len() == 11);
    assert!(Occupant::HavenShelter as usize == OCCUPANT_R_M.len() - 1);
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
    let k = slot.occupant as usize;
    let r = OCCUPANT_R_M[k];
    if r <= 0.0 {
        return false;
    }
    if feet_y >= slot.y + OCCUPANT_TOP_M[k] * slot.scale || feet_y + capsule_h <= slot.y {
        return false;
    }
    let reach = r * slot.scale + capsule_r;
    let dx = x - slot.x;
    let dz = z - slot.z;
    dx * dx + dz * dz < reach * reach
}
