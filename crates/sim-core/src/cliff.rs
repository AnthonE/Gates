//! Cliff crags: the rock ledges stepping down every face too steep to walk,
//! and the teeth standing in a face just under its lip, as a pure function of
//! the seed, so the sim collides with exactly the rocks the client draws.
//!
//! They were the client's alone until 2026-10-07 (`render/cliffs.rs`), drawn
//! on the claim that nobody could reach one. The sim refuses only a RISE past
//! the cliff ratio, so a body walks down a face and along it, and a player
//! walked straight through a tooth standing at a lip. Now `occupy` asks these
//! the way it asks a formation block: a crag's top is ground, its sides are
//! walls.
//!
//! **Spots, not cells.** One candidate per [`SPOT_M`] spot, its crag decided
//! from the ground around it alone, so the sim's memo holds a spot at a time
//! and a walking body pays for a strip of new spots (five ground taps each
//! off a cliff) rather than a whole cell at once. The client walks the same
//! spots to build its meshes and keeps the scree to itself: ankle-high
//! stones, waded through like clutter.
//!
//! Every height here is the sim's. The client draws a ledge down a face by
//! the cliff relief's offset there, as it draws the face (`terrain_mesh`), so
//! a body is as far off a ledge as it is off the rock beside it. A tooth's
//! top is the lip's height plus its rise, and the relief never moves a lip,
//! so a tooth is drawn exactly where it collides.

use crate::fmath::{fabs, floor_i32};
use crate::terrain::{self, Haven, Lattice, CLIFF_SLOPE_RATIO};

/// A spot's side, metres: one crag candidate per spot.
pub const SPOT_M: f32 = 4.0;
/// Spots a side of the island.
pub const SPOTS_PER_SIDE: i32 = (terrain::ISLAND_SIZE / SPOT_M) as i32;

/// The share of steep spots that grow a ledge where the crag field is full;
/// it falls to nothing where the field is empty, so a face carries crags and
/// smooth slabs between them rather than an even brick pattern.
const LEDGE_SHARE: f32 = 0.9;
/// The crag field's wavelength, metres.
const CRAG_M: f32 = 22.0;
/// Tallest visible downhill face, metres.
pub const LEDGE_FACE_MAX_M: f32 = 3.2;
/// Shortest visible downhill face, metres.
pub const LEDGE_FACE_MIN_M: f32 = 0.8;
/// How far a crag's foot is buried below the face, metres.
pub const LEDGE_BURY_M: f32 = 0.7;
/// How far below the lip a tooth may stand, metres along the fall line.
const TOOTH_REACH_M: f32 = 3.5;
/// The share of spots under a lip that grow a tooth where the crag field is
/// full.
const TOOTH_SHARE: f32 = 0.6;
/// The gap kept between a tooth's uphill face and the lip, metres.
const TOOTH_CLEAR_M: f32 = 0.25;
/// Lowest and highest a tooth's head stands above the lip, metres.
const TOOTH_RISE_MIN_M: f32 = 0.3;
const TOOTH_RISE_MAX_M: f32 = 1.5;
/// Tallest a tooth may be from its buried foot, metres: a face that drops
/// away faster than this under the lip would make it a free-standing pillar.
const TOOTH_TALL_MAX_M: f32 = 5.0;
/// A tooth's half-extents, metres: broad and low, a rocky rim rather than a
/// standing stone.
const TOOTH_HX_MAX: f32 = 4.0;
const TOOTH_HZ_MAX: f32 = 0.65;
/// A ledge's widest half-extent along the contour, metres.
const LEDGE_HX_MAX: f32 = 1.4 + 3.6 + LEDGE_FACE_MAX_M * 0.5;
/// A ledge's deepest half-extent down the fall line, metres: deep enough to
/// bury the shelf's uphill end on the gentlest face.
const LEDGE_HZ_MAX: f32 = {
    let buried =
        (LEDGE_FACE_MAX_M + LEDGE_TILT_MAX * LEDGE_HX_MAX) / (2.0 * CLIFF_SLOPE_RATIO) + 0.4;
    if buried > 1.6 {
        buried
    } else {
        1.6
    }
};
/// The most a ledge's top leans along the contour, rise per metre.
const LEDGE_TILT_MAX: f32 = 0.2;
/// How far any crag's footprint reaches from its spot's candidate, metres:
/// its centre's offset plus a bound on its half-diagonal. A query scans the
/// spots this can reach.
pub const CRAG_REACH_M: f32 = LEDGE_HX_MAX + 2.0 * LEDGE_HZ_MAX;

const _: () = {
    assert!(TOOTH_CLEAR_M + TOOTH_HZ_MAX + TOOTH_HX_MAX + TOOTH_HZ_MAX <= CRAG_REACH_M);
    assert!(CRAG_REACH_M < 16.0);
};

/// One crag: an oriented box, its top a plane — what `occupy` stops a body
/// with and what the client draws a fractured rock over. `hx == 0` is none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crag {
    pub x: f32,
    pub z: f32,
    /// Half-extents along the crag's own x (the contour) and z (downhill).
    pub hx: f32,
    pub hz: f32,
    /// Local +Z in world space, a unit vector: downhill, give or take.
    /// `(sin, cos)` of its bearing, the `yaw_dir` convention.
    pub dir: (f32, f32),
    /// Its top over its centre.
    pub y1: f32,
    /// The top's rise per metre along the crag's own x and z.
    pub tx: f32,
    pub tz: f32,
    /// A tooth under a lip, or a ledge in a face.
    pub tooth: bool,
    /// Whether moss may grow on its top. The sim never reads it.
    pub moss: bool,
}

impl Crag {
    pub const NONE: Crag = Crag {
        x: 0.0,
        z: 0.0,
        hx: 0.0,
        hz: 0.0,
        dir: (0.0, 1.0),
        y1: 0.0,
        tx: 0.0,
        tz: 0.0,
        tooth: false,
        moss: false,
    };

    #[inline]
    pub fn is_some(&self) -> bool {
        self.hx > 0.0
    }

    /// (`x`, `z`) in the crag's own frame.
    #[inline]
    pub fn to_local(&self, x: f32, z: f32) -> (f32, f32) {
        let (s, c) = self.dir;
        let (dx, dz) = (x - self.x, z - self.z);
        (dx * c - dz * s, dx * s + dz * c)
    }

    /// A point of the crag's own frame in world space.
    #[inline]
    pub fn to_world(&self, lx: f32, lz: f32) -> (f32, f32) {
        let (s, c) = self.dir;
        (self.x + lx * c + lz * s, self.z - lx * s + lz * c)
    }

    /// The top over (`x`, `z`) if it lies within the footprint grown by
    /// `dil` metres, the plane held level past the footprint's edge.
    #[inline]
    pub fn top(&self, x: f32, z: f32, dil: f32) -> Option<f32> {
        let (lx, lz) = self.to_local(x, z);
        if fabs(lx) > self.hx + dil || fabs(lz) > self.hz + dil {
            return None;
        }
        Some(
            self.y1 + self.tx * lx.clamp(-self.hx, self.hx) + self.tz * lz.clamp(-self.hz, self.hz),
        )
    }
}

/// One spot, resolved: the ground at its candidate and the crag it grows.
/// The client builds its meshes from these; the sim keeps only the crag.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spot {
    pub x: f32,
    pub z: f32,
    /// The ground's slope there (rise/run, `ground_slope`'s taps), and the
    /// fall line, a unit vector downhill. Zero over the sea and on the flat.
    pub s: f32,
    pub dx: f32,
    pub dz: f32,
    /// The spot's variation key.
    pub key: u32,
    pub crag: Crag,
}

/// Integer hash to [0, 1).
#[inline]
pub fn h01(a: u32, b: u32) -> f32 {
    let mut x = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// `(sin a, cos a)` of a small angle (|a| ≤ 0.5 rad), the series to a⁵:
/// wall 1 has no trig, and a crag's bearing only has to wander.
#[inline]
fn small_turn(a: f32) -> (f32, f32) {
    let a2 = a * a;
    (
        a * (1.0 - a2 * (1.0 / 6.0) * (1.0 - a2 * (1.0 / 20.0))),
        1.0 - a2 * 0.5 * (1.0 - a2 * (1.0 / 12.0)),
    )
}

/// `(dx, dz)` turned by the angle whose `(sin, cos)` is `(sa, ca)`.
#[inline]
fn turned((dx, dz): (f32, f32), (sa, ca): (f32, f32)) -> (f32, f32) {
    (dx * ca - dz * sa, dx * sa + dz * ca)
}

/// Where the crags gather, 0..1: smooth value noise at [`CRAG_M`], pushed
/// toward its ends.
fn crag_field(x: f32, z: f32, seed: u32) -> f32 {
    let (fx, fz) = (x / CRAG_M, z / CRAG_M);
    let (ix, iz) = (floor_i32(fx), floor_i32(fz));
    let (tx, tz) = (fx - ix as f32, fz - iz as f32);
    let (ux, uz) = (tx * tx * (3.0 - 2.0 * tx), tz * tz * (3.0 - 2.0 * tz));
    let c = |dx: i32, dz: i32| {
        h01(
            ((ix + dx) as u32).wrapping_mul(73_856_093)
                ^ ((iz + dz) as u32).wrapping_mul(83_492_791),
            seed ^ 0xc2a9,
        )
    };
    let a = c(0, 0) + (c(1, 0) - c(0, 0)) * ux;
    let b = c(0, 1) + (c(1, 1) - c(0, 1)) * ux;
    let v = a + (b - a) * uz;
    ((v - 0.25) / 0.5).clamp(0.0, 1.0)
}

/// Whether nothing authored owns this ground: sites, the haven and the build
/// pads keep their carved faces clean.
pub fn open_ground(haven: &Haven, x: f32, z: f32) -> bool {
    !terrain::in_haven(haven, x, z) && terrain::site_sweep(haven, x, z) <= 0.0
}

/// The seed's share of every spot key.
#[inline]
fn seed_key(seed: u64) -> u32 {
    seed as u32 ^ (seed >> 32) as u32
}

/// Spot (`gi`, `gj`), resolved. Off the island it is empty.
pub fn spot(lat: &mut Lattice, seed: u64, haven: &Haven, gi: i32, gj: i32) -> Spot {
    let sk = seed_key(seed);
    let k = (gi as u32).wrapping_mul(73_856_093) ^ (gj as u32).wrapping_mul(19_349_663) ^ sk;
    let x = (gi as f32 + h01(k, 1)) * SPOT_M;
    let z = (gj as f32 + h01(k, 2)) * SPOT_M;
    let mut out = Spot {
        x,
        z,
        key: k,
        crag: Crag::NONE,
        ..Spot::default()
    };
    if gi < 0 || gj < 0 || gi >= SPOTS_PER_SIDE || gj >= SPOTS_PER_SIDE {
        return out;
    }
    let g = |lat: &mut Lattice, x: f32, z: f32| terrain::ground_memo(lat, seed, haven, x, z);
    if g(lat, x, z) < terrain::SEA_LEVEL - 0.5 {
        return out;
    }
    // Central differences at 1 m: `ground_slope`'s own taps.
    let gx = (g(lat, x + 1.0, z) - g(lat, x - 1.0, z)) * 0.5;
    let gz = (g(lat, x, z + 1.0) - g(lat, x, z - 1.0)) * 0.5;
    let s = (gx * gx + gz * gz).sqrt();
    if s < 1e-3 {
        return out;
    }
    // Downhill, along the fall line.
    let (dx, dz) = (-gx / s, -gz / s);
    out.s = s;
    out.dx = dx;
    out.dz = dz;
    if s < CLIFF_SLOPE_RATIO || !open_ground(haven, x, z) {
        return out;
    }
    let field = crag_field(x, z, sk);
    // Just under the lip, a tooth standing above it breaks the outline the
    // walkable top would otherwise draw.
    if let Some(lip) = lip_above(lat, seed, haven, x, z, (dx, dz)) {
        if h01(k, 5) < TOOTH_SHARE * field {
            out.crag = tooth(lat, seed, haven, (x, z), lip, (dx, dz), k);
        }
    }
    if !out.crag.is_some() && h01(k, 3) < LEDGE_SHARE * field {
        out.crag = ledge(lat, seed, haven, (x, z), s, (dx, dz), k);
    }
    out
}

/// The crag spot (`gi`, `gj`) grows, or [`Crag::NONE`].
pub fn crag(lat: &mut Lattice, seed: u64, haven: &Haven, gi: i32, gj: i32) -> Crag {
    spot(lat, seed, haven, gi, gj).crag
}

/// The lip above a steep point: the first walkable ground up the fall line
/// within [`TOOTH_REACH_M`], as `(distance, height)`.
fn lip_above(
    lat: &mut Lattice,
    seed: u64,
    haven: &Haven,
    x: f32,
    z: f32,
    (dx, dz): (f32, f32),
) -> Option<(f32, f32)> {
    let mut u = 0.5;
    while u <= TOOTH_REACH_M {
        let (px, pz) = (x - dx * u, z - dz * u);
        if terrain::ground_slope_memo(lat, seed, haven, px, pz) < CLIFF_SLOPE_RATIO {
            return Some((u, terrain::ground_memo(lat, seed, haven, px, pz)));
        }
        u += 0.5;
    }
    None
}

/// A crag standing in the top of a face, its head above the lip `lip`
/// (`(distance uphill, height)`) and its whole footprint on ground too steep
/// to walk, so the outline breaks at the edge of the walkable top.
fn tooth(
    lat: &mut Lattice,
    seed: u64,
    haven: &Haven,
    (x, z): (f32, f32),
    (ul, y_lip): (f32, f32),
    (dx, dz): (f32, f32),
    k: u32,
) -> Crag {
    let h = |c: u32| h01(k, c + 32);
    let hx = 1.4 + (TOOTH_HX_MAX - 1.4) * h(1) * h(1);
    let hz = 0.35 + (TOOTH_HZ_MAX - 0.35) * h(2);
    // Off the fall line a little, so a row of teeth does not line up.
    let (bx, bz) = turned((dx, dz), small_turn((h(3) - 0.5) * 0.8));
    // The uphill face keeps clear of the lip.
    let back = (TOOTH_CLEAR_M + hz - ul).max(0.0);
    let (cx, cz) = (x + dx * back, z + dz * back);
    let (ax, az) = (-bz, bx);
    let corners = [
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
        (0.0, -1.0),
    ];
    let mut foot = f32::INFINITY;
    for (u, v) in corners {
        // `v` runs downhill: `-1` is the uphill side.
        let (px, pz) = (
            cx + ax * hx * u + bx * hz * v,
            cz + az * hx * u + bz * hz * v,
        );
        if terrain::ground_slope_memo(lat, seed, haven, px, pz) < CLIFF_SLOPE_RATIO {
            return Crag::NONE;
        }
        foot = foot.min(terrain::ground_memo(lat, seed, haven, px, pz));
    }
    let rise = TOOTH_RISE_MIN_M + (TOOTH_RISE_MAX_M - TOOTH_RISE_MIN_M) * h(4) * h(4) * h(4);
    if y_lip + rise - (foot - LEDGE_BURY_M) > TOOTH_TALL_MAX_M {
        return Crag::NONE;
    }
    Crag {
        x: cx,
        z: cz,
        hx,
        hz,
        dir: (bx, bz),
        y1: y_lip + rise,
        tx: (h(6) - 0.5) * 0.5,
        // The top falls away toward the drop, as a weathered rim does.
        tz: -0.1 - 0.25 * h(7),
        tooth: true,
        moss: y_lip > terrain::BEACH_MAX_H + 1.5 && y_lip < terrain::TREELINE_H && h(5) < 0.4,
    }
}

/// One ledge whose downhill face stands at (`x`, `z`), on a face of slope `s`.
fn ledge(
    lat: &mut Lattice,
    seed: u64,
    haven: &Haven,
    (x, z): (f32, f32),
    s: f32,
    (dx, dz): (f32, f32),
    k: u32,
) -> Crag {
    let h = |c: u32| h01(k, c + 16);
    // Most ledges are modest and a few are big, the way a crag breaks.
    let face = LEDGE_FACE_MIN_M + (LEDGE_FACE_MAX_M - LEDGE_FACE_MIN_M) * h(3) * h(3);
    let hx = (1.4 + 3.6 * h(1) * h(1) + face * 0.5).min(LEDGE_HX_MAX);
    // Tops lean along the contour as bedding planes do, which raises one end
    // of the face by up to `|tx| hx`.
    let tx = (h(5) - 0.5) * (2.0 * LEDGE_TILT_MAX);
    let tallest = face + fabs(tx) * hx;
    // The ground below the face, out past where the ledge stands proud of
    // the slope, must be a face too, or the ledge would stand on walkable
    // foot-slope.
    let reach = tallest / s + 1.0;
    let (fx, fz) = (x + dx * reach, z + dz * reach);
    if terrain::ground_slope_memo(lat, seed, haven, fx, fz) < CLIFF_SLOPE_RATIO {
        return Crag::NONE;
    }
    // Deep enough that the shelf's uphill end is buried: the ground climbs
    // `2 hz s` across it, against the face's height.
    let hz = (tallest / (2.0 * s) + 0.4)
        .max(0.8 + 0.8 * h(2))
        .min(LEDGE_HZ_MAX);
    // Off the fall line a little, so no two ledges share a bearing.
    let (dx, dz) = turned((dx, dz), small_turn((h(7) - 0.5) * 0.7));
    // The crag's local +Z points downhill; its centre sits `hz` uphill of
    // the face.
    let (cx, cz) = (x - dx * hz, z - dz * hz);
    // The contour either side of the face: a ledge across a gully would show
    // its whole side, so that ground is left to the face.
    let (ax, az) = (-dz, dx);
    let y = terrain::ground_memo(lat, seed, haven, x, z);
    let yl = terrain::ground_memo(lat, seed, haven, x + ax * hx, z + az * hx);
    let yr = terrain::ground_memo(lat, seed, haven, x - ax * hx, z - az * hx);
    if y - yl.min(yr) > face {
        return Crag::NONE;
    }
    let y1 = y + face;
    // A face that eases off behind the ledge would leave its back standing a
    // metre out of the rock.
    let (bx, bz) = (x - dx * 2.0 * hz, z - dz * 2.0 * hz);
    if terrain::ground_memo(lat, seed, haven, bx, bz) < y1 - 1.0 {
        return Crag::NONE;
    }
    Crag {
        x: cx,
        z: cz,
        hx,
        hz,
        dir: (dx, dz),
        y1,
        tx,
        tz: (h(6) - 0.5) * 0.25,
        tooth: false,
        moss: y > terrain::BEACH_MAX_H + 1.5 && y < terrain::TREELINE_H && h(4) < 0.35,
    }
}

/// The spots whose crags can reach (`x`, `z`) grown by `pad` metres.
#[inline]
pub fn spots_near(x: f32, z: f32, pad: f32, mut f: impl FnMut(i32, i32)) {
    let r = CRAG_REACH_M + pad;
    let last = SPOTS_PER_SIDE - 1;
    let i0 = floor_i32((x - r) / SPOT_M).max(0);
    let i1 = floor_i32((x + r) / SPOT_M).min(last);
    let j0 = floor_i32((z - r) / SPOT_M).max(0);
    let j1 = floor_i32((z + r) / SPOT_M).min(last);
    let mut j = j0;
    while j <= j1 {
        let mut i = i0;
        while i <= i1 {
            f(i, j);
            i += 1;
        }
        j += 1;
    }
}
