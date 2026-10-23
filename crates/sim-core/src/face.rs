//! The rock faces' shape, owned by the sim so that what is drawn is what
//! is struck. Both layers were the client's alone until 2026-10-17, drawn on
//! the claim that nobody reached a face, and a body slid through metres of
//! drawn rock (operator: "why cant things be real?"). The client draws these
//! functions; the sim stands bodies, rests items and stops arrows on them
//! ([`surface`]).
//!
//! **The relief.** A scarp is the sim's smooth heightfield, so its outline
//! against the sky was a rounded hump. The near ground is pushed in and out
//! on faces too steep to climb, and only there: a vertical offset that is a
//! pure function of the sim's heights on a 5×5 stencil of the mesh's own
//! lattice, so every chunk, seam and LOD that samples the same point agrees.
//! It is exactly zero unless every vertex around it is steeper than
//! [`RELIEF_SLOPE_LO`], so no triangle that touches walkable ground moves,
//! and the drawn height stays inside the stencil's own heights, so a cut
//! never digs a pit below the foot and a push never stands a wall above the
//! lip.

use crate::fmath::{fabs, floor_i32};
use crate::terrain::{self, SEA_LEVEL};

/// Slope (rise/run, 1 m arms) where the relief starts: past the sim's cliff
/// ratio by a margin, so the lip and the foot stay the sim's surface.
pub const RELIEF_SLOPE_LO: f32 = terrain::CLIFF_SLOPE_RATIO * 1.15;
/// Slope where the relief is at full strength.
pub const RELIEF_SLOPE_HI: f32 = terrain::CLIFF_SLOPE_RATIO * 1.45;
/// Most a face is pushed out, metres (vertical).
pub const RELIEF_UP_M: f32 = 3.0;
/// Most a face is cut back, metres (vertical). Deeper than the push: the
/// scarps run 50 m tall, and a cut never closes over a body on the face.
pub const RELIEF_DOWN_M: f32 = 6.0;
/// Slope past which a horizontal offset stops growing in height.
const RELIEF_SLOPE_CAP: f32 = 2.8;
/// Spacing of the gullies, metres: the far mesh's share of the relief.
const GULLY_M: f32 = 16.0;
/// Height of one bed of the stepped strata, metres.
const BED_M: f32 = 8.0;

/// Integer hash to [0, 1).
fn relief_hash(ix: i32, iz: i32, key: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x8da6_b343)
        ^ (iz as u32).wrapping_mul(0xd816_3841)
        ^ key.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Quintic-faded value noise in [0, 1) on a unit lattice.
fn relief_noise(x: f32, z: f32, key: u32) -> f32 {
    let (x0, z0) = (floor_i32(x) as f32, floor_i32(z) as f32);
    let (tx, tz) = (x - x0, z - z0);
    let (ix, iz) = (x0 as i32, z0 as i32);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(tx), fade(tz));
    let c00 = relief_hash(ix, iz, key);
    let c10 = relief_hash(ix + 1, iz, key);
    let c01 = relief_hash(ix, iz + 1, key);
    let c11 = relief_hash(ix + 1, iz + 1, key);
    let a = c00 + (c10 - c00) * u;
    let b = c01 + (c11 - c01) * u;
    a + (b - a) * v
}

/// A ridged field in [0, 1]: 1 on the crests, which meander as lines.
fn ridged(x: f32, z: f32, key: u32) -> f32 {
    let r = 1.0 - fabs(relief_noise(x, z, key) * 2.0 - 1.0);
    r * r
}

/// The drawn ground's vertical offset from the sim's at `(x, z)`, metres.
///
/// `y` is the sim's ground there and `slope` its rise/run read at arms of
/// `step` (the pitch of the mesh being built). Exactly `0.0` below
/// [`RELIEF_SLOPE_LO`] and on authored sites. Features finer than the pitch
/// can hold are left out, so the 8 m far mesh carries the gullies and the
/// near ring the ribs and strata as well.
pub fn cliff_relief(
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
    y: f32,
    slope: f32,
    step: f32,
) -> f32 {
    if slope <= RELIEF_SLOPE_LO || y < SEA_LEVEL {
        return 0.0;
    }
    let sweep = terrain::site_sweep(haven, x, z);
    if sweep >= 1.0 {
        return 0.0;
    }
    let t = ((slope - RELIEF_SLOPE_LO) / (RELIEF_SLOPE_HI - RELIEF_SLOPE_LO)).min(1.0);
    let mask = t * t * (3.0 - 2.0 * t) * (1.0 - sweep);
    let key = seed as u32 ^ (seed >> 32) as u32;
    // A horizontal offset of the face becomes `slope` times as much height.
    let lever = slope.min(RELIEF_SLOPE_CAP);

    // Gullies cut back along the ridged field's crest lines, ~16 m apart;
    // the face between them noses out as buttresses. Horizontal metres.
    let mut out = 1.6 - 4.0 * ridged(x / GULLY_M, z / GULLY_M, key ^ 0x0b07);
    let mut cut = 0.0;
    if step <= 2.0 {
        // Ribs down the buttresses. Nothing finer: the vertices are a metre
        // apart and a feature near that size draws as a sawtooth outline.
        out += (ridged(x / 6.0, z / 6.0, key ^ 0x51b5) - 0.4) * 0.9;
        // Bedding: the face steps down in benches and risers. The beds dip
        // across the island (`tilt`) and each one is its own strength, so
        // no two ledges run parallel for long.
        let tilt = (relief_noise(x / 70.0, z / 70.0, key ^ 0x7e11) - 0.5) * 9.0;
        let b = (y + tilt) / BED_M;
        let bed = floor_i32(b) as f32;
        let f = b - bed;
        let strength = 0.5 + 0.5 * relief_hash(bed as i32, 0x5be7, key);
        // `f³ − f`: flat at the foot of each bed, sheer at its top.
        cut = BED_M * (f * f * f - f) * strength;
    }
    (mask * (out * lever + cut)).clamp(-RELIEF_DOWN_M, RELIEF_UP_M)
}

/// The relief at a vertex, from the sim's heights `h[j][i]` on the 5×5 stencil
/// of the mesh's lattice around it (`h[2][2]` is the vertex, pitch `step`).
///
/// [`cliff_relief`] fed the gentlest slope of the 3×3 around the vertex, so a
/// vertex beside walkable ground does not move; then held inside the
/// stencil's own range of heights. The offset is returned as `drawn − y`, and
/// the drawn height is `y + offset` (or `y` itself where it is zero).
pub fn stencil_relief(
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
    h: &[[f32; 5]; 5],
    step: f32,
) -> f32 {
    let s =
        |i: usize, j: usize| relief_slope(h[j][i + 1], h[j][i - 1], h[j + 1][i], h[j - 1][i], step);
    let mut gentlest = s(2, 2);
    if gentlest <= RELIEF_SLOPE_LO {
        return 0.0;
    }
    for j in 1..4 {
        for i in 1..4 {
            gentlest = gentlest.min(s(i, j));
        }
    }
    let y = h[2][2];
    let r = cliff_relief(seed, haven, x, z, y, gentlest, step);
    if r == 0.0 {
        return 0.0;
    }
    let (mut lo, mut hi) = (y, y);
    for v in h.iter().flatten() {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    (y + r).clamp(lo, hi) - y
}

/// The drawn ground at a vertex of a mesh of pitch `step`: the sim's height
/// plus [`stencil_relief`].
///
/// What `heightfield` writes (before its drop), restated for one point —
/// `terrain_seam` joins to it, `far_ground_y` interpolates it. Five taps off a
/// face, twenty-five on one.
pub fn drawn_y(
    lat: &mut terrain::Lattice,
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
    step: f32,
) -> f32 {
    let mut g = |i: i32, j: i32| {
        terrain::ground_memo(lat, seed, haven, x + i as f32 * step, z + j as f32 * step)
    };
    let y = g(0, 0);
    if relief_slope(g(1, 0), g(-1, 0), g(0, 1), g(0, -1), step) <= RELIEF_SLOPE_LO {
        return y;
    }
    let mut h = [[0.0f32; 5]; 5];
    for (j, row) in h.iter_mut().enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            *v = g(i as i32 - 2, j as i32 - 2);
        }
    }
    let r = stencil_relief(seed, haven, x, z, &h, step);
    if r != 0.0 {
        y + r
    } else {
        y
    }
}

/// Rise/run from the four neighbours `step` away. At 1 m this is bit-for-bit
/// `terrain::ground_slope`, the slope the sim vetoes cliffs by.
pub fn relief_slope(xp: f32, xm: f32, zp: f32, zm: f32, step: f32) -> f32 {
    let k = 0.5 / step;
    let sx = (xp - xm) * k;
    let sz = (zp - zm) * k;
    (sx * sx + sz * sz).sqrt()
}

/// The NEAR ring's drawn surface at any `(x, z)`: its 1 m triangles
/// interpolated exactly as `heightfield` winds them. For things that sit on
/// a cliff face (`cliffs.rs`), which the relief moves off the sim's height.
pub fn near_drawn_y(
    lat: &mut terrain::Lattice,
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
) -> f32 {
    let (x0, z0) = (floor_i32(x) as f32, floor_i32(z) as f32);
    let (fx, fz) = (x - x0, z - z0);
    let mut v = |dx: f32, dz: f32| drawn_y(lat, seed, haven, x0 + dx, z0 + dz, 1.0);
    // Quads split on the (1,0)–(0,1) diagonal: triangles `a c b` and `b c d`.
    let (b, c) = (v(1.0, 0.0), v(0.0, 1.0));
    if fx + fz <= 1.0 {
        let a = v(0.0, 0.0);
        a + (b - a) * fx + (c - a) * fz
    } else {
        let d = v(1.0, 1.0);
        d + (c - d) * (1.0 - fx) + (b - d) * (1.0 - fz)
    }
}
// ── The skin ──────────────────────────────────────────────────────────────
//
// A faceted shell over every face too steep to climb: the drawn face pushed
// out by a field built of planes — piecewise-linear noise from buttress to
// slab size, so the face breaks into flat facets meeting at creases — and
// stepped at the strata, so a bed that stands out further than the one below
// it is a riser and one that stands back leaves a shelf. It was a horizontal
// push with real overhangs until 2026-10-17; it is a height now, what that
// push is worth on the face's own slope, so it is ground the sim can stand a
// body on and stop an arrow with (operator, choosing exact over overhangs).
// It never rises past the highest ground within two metres, so it never
// stands a wall above the lip.

/// The most the face is pushed out, metres along the fall line; drawn as
/// that much times the face's slope in height.
pub const SKIN_OUT_M: f32 = 3.5;
/// Rise/run where the skin starts: the sim's own climbing limit…
pub const SKIN_SLOPE_LO: f32 = terrain::CLIFF_SLOPE_RATIO;
/// …and where it is whole.
pub const SKIN_SLOPE_HI: f32 = terrain::CLIFF_SLOPE_RATIO * 1.4;
/// How far clear of a crag's footprint the skin is whole again, metres.
pub const CRAG_FADE_M: f32 = 2.0;
/// How far the shell sinks under the drawn ground where it fades out.
pub const SKIN_SINK_M: f32 = 0.3;
/// The skin's lattice pitch, metres: half the near ground's, so a stratum's
/// step resolves as a step and not a sawtooth.
pub const SKIN_STEP_M: f32 = 0.5;
/// The buttress-sized facets' lattice, metres…
const FACET_BROAD_M: f32 = 15.0;
/// …the large ones'…
pub const FACET_M: f32 = 6.5;
/// …and the small ones'.
const FACET_FINE_M: f32 = 2.4;
/// Thickness of one stratum, metres.
const STRATUM_M: f32 = 4.5;
/// How far one stratum may stand out from the next, as a share of
/// [`SKIN_OUT_M`].
const STRATUM_STEP: f32 = 0.42;
/// How much of a stratum its step from the one below takes.
const STEP_BAND: f32 = 0.35;

/// Integer hash to [0, 1).
fn h01(a: u32, b: u32) -> f32 {
    let mut x = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

fn corner(i: [i32; 3], key: u32) -> f32 {
    h01(
        (i[0] as u32).wrapping_mul(73_856_093)
            ^ (i[1] as u32).wrapping_mul(19_349_663)
            ^ (i[2] as u32).wrapping_mul(83_492_791),
        key,
    )
}

/// Piecewise-linear value noise in [0, 1] at (`x`, `y`, `z`): each unit cube
/// is split into the six tetrahedra that share its main diagonal and the
/// hashed corner values are interpolated linearly inside each, so the field
/// is a mosaic of planes meeting at creases — fractured rock rather than
/// rolling ground. Read through a fixed rotation so no crease runs along a
/// world axis.
pub fn facet_noise(x: f32, y: f32, z: f32, key: u32) -> f32 {
    let p = [
        0.788 * x - 0.497 * y + 0.364 * z,
        0.326 * x + 0.837 * y + 0.440 * z,
        -0.523 * x - 0.229 * y + 0.821 * z,
    ];
    let base = [floor_i32(p[0]), floor_i32(p[1]), floor_i32(p[2])];
    let f = [
        p[0] - base[0] as f32,
        p[1] - base[1] as f32,
        p[2] - base[2] as f32,
    ];
    // The axes in falling order of their fraction: the walk from the cube's
    // low corner to its high one through the tetrahedron holding `f`.
    let mut ax = [(f[0], 0usize), (f[1], 1), (f[2], 2)];
    if ax[0].0 < ax[1].0 {
        ax.swap(0, 1);
    }
    if ax[1].0 < ax[2].0 {
        ax.swap(1, 2);
    }
    if ax[0].0 < ax[1].0 {
        ax.swap(0, 1);
    }
    let mut c = base;
    let mut v = corner(c, key) * (1.0 - ax[0].0);
    c[ax[0].1] += 1;
    v += corner(c, key) * (ax[0].0 - ax[1].0);
    c[ax[1].1] += 1;
    v += corner(c, key) * (ax[1].0 - ax[2].0);
    c[ax[2].1] += 1;
    v + corner(c, key) * ax[2].0
}

/// How far the face stands out at (`x`, `y`, `z`), as a share of
/// [`SKIN_OUT_M`], before the fade; and how far it stands out from the
/// buttress it is part of, roughly -1..1, which reads as proud or recessed.
fn push(x: f32, y: f32, z: f32, key: u32) -> (f32, f32) {
    let at = |m: f32, k: u32| facet_noise(x / m, y / m, z / m, k);
    let broad = at(FACET_BROAD_M, key ^ 0xb0ad);
    let big = at(FACET_M, key ^ 0xfac3);
    let fine = at(FACET_FINE_M, key ^ 0x5ca1);
    // Strata: the bed's own stand-out, and how its face leans, so the
    // boundary between two beds is a step: a shelf where the upper stands
    // back, a riser where it juts. The beds wander and dip, so no two run
    // parallel for long.
    let lift = (at(23.0, key ^ 0xbed5) - 0.5) * STRATUM_M * 1.6;
    let b = (y + lift) / STRATUM_M;
    let bed = floor_i32(b) as f32;
    let f = b - bed;
    // The step from the bed below over the foot of this one, so it is a
    // short steep band the lattice can draw.
    let below = stratum(bed - 1.0, 1.0, x, z, key);
    let s = (f / STEP_BAND).min(1.0);
    let strata = below + (stratum(bed, f, x, z, key) - below) * s * s * (3.0 - 2.0 * s);
    let local = 0.42 * (big - 0.5) * 2.0 + 0.16 * (fine - 0.5) * 2.0 + strata;
    let t = 0.35 * (broad - 0.5) * 2.0 + local;
    // Into (0, 1) without a clamp: a clamped stretch is the smooth face
    // again, laid out at a fixed offset.
    (0.5 + t / (1.0 + 2.0 * fabs(t)), local)
}

/// Bed `bed`'s stand-out at height `f` (0..1) up it: its own, drifting
/// along the face so a shelf comes and goes, and how its face leans.
fn stratum(bed: f32, f: f32, x: f32, z: f32, key: u32) -> f32 {
    let bed_key = key ^ (bed as i32 as u32).wrapping_mul(0x2545_F491);
    let drift = facet_noise(x / 11.0, 0.0, z / 11.0, bed_key);
    let stand = (h01(bed_key, 1) * 0.6 + drift * 0.4 - 0.5) * 2.0 * STRATUM_STEP;
    let lean = (h01(bed_key, 2) - 0.4) * 0.4;
    stand + lean * (f - 0.5)
}

/// How much of the skin a whole-metre point carries, 0..1, from the gentlest
/// slope of the 3×3 round it: none on ground the sim lets a body climb, nor
/// on an authored site or in the haven, whose carved faces stay clean.
fn skin_mask(haven: &terrain::Haven, x: f32, z: f32, gentlest: f32) -> f32 {
    let t = (gentlest - SKIN_SLOPE_LO) / (SKIN_SLOPE_HI - SKIN_SLOPE_LO);
    if t <= 0.0 || terrain::in_haven(haven, x, z) {
        return 0.0;
    }
    let sweep = terrain::site_sweep(haven, x, z);
    if sweep >= 1.0 {
        return 0.0;
    }
    let t = t.min(1.0);
    t * t * (3.0 - 2.0 * t) * (1.0 - sweep)
}

/// One whole-metre point of the ground, as the skin over it is built from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coarse {
    /// The drawn ground: the sim's, plus the relief.
    pub drawn: f32,
    /// The gentlest slope of the 3×3 round it, rise/run.
    pub gentlest: f32,
    /// How much of the skin it carries, 0..1.
    pub fade: f32,
    /// The sim's gradient across it, two metres a side (`h[2][3] - h[2][1]`,
    /// `h[3][2] - h[1][2]`): twice the rise per metre.
    pub sx: f32,
    pub sz: f32,
    /// The highest of the 5×5 round it: the skin stands no higher.
    pub hi: f32,
}

/// The coarse point at (`x`, `z`) from the sim's heights `h[j][i]` on the 5×5
/// at a metre round it (`h[2][2]` is the point).
pub fn coarse(seed: u64, haven: &terrain::Haven, x: f32, z: f32, h: &[[f32; 5]; 5]) -> Coarse {
    let y = h[2][2];
    let s =
        |i: usize, j: usize| relief_slope(h[j][i + 1], h[j][i - 1], h[j + 1][i], h[j - 1][i], 1.0);
    let mut gentlest = s(2, 2);
    for j in 1..4 {
        for i in 1..4 {
            gentlest = gentlest.min(s(i, j));
        }
    }
    let r = stencil_relief(seed, haven, x, z, h, 1.0);
    let fade = if y < SEA_LEVEL {
        0.0
    } else {
        skin_mask(haven, x, z, gentlest)
    };
    // A point with no skin caps it at its own height, so a point off the
    // faces is the 3×3 it was gated on and nothing wider.
    let mut hi = y;
    if fade > 0.0 {
        for v in h.iter().flatten() {
            hi = hi.max(*v);
        }
    }
    Coarse {
        drawn: if r != 0.0 { y + r } else { y },
        gentlest,
        fade,
        sx: h[2][3] - h[2][1],
        sz: h[3][2] - h[1][2],
        hi,
    }
}

/// The seed's share of every skin key.
#[inline]
pub fn seed_key(seed: u64) -> u32 {
    seed as u32 ^ (seed >> 32) as u32
}

/// The solid ground at (`x`, `z`): where a body stands, an item comes to
/// rest and an arrow strikes. Off the faces it is the sim's ground; over a
/// whole-metre quad with a corner too steep to climb it is the rock the
/// client draws there — the near ground with its relief and the skin over
/// it, each as its triangles carry it, whichever stands higher.
pub fn surface(
    occ: &mut crate::occupy::Occupants,
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
) -> f32 {
    let c = &mut *occ.cache;
    let (x0, z0) = (floor_i32(x), floor_i32(z));
    let q = [
        c.coarse(seed, haven, x0, z0),
        c.coarse(seed, haven, x0 + 1, z0),
        c.coarse(seed, haven, x0, z0 + 1),
        c.coarse(seed, haven, x0 + 1, z0 + 1),
    ];
    if q.iter().all(|q| q.gentlest <= SKIN_SLOPE_LO) {
        return c.ground(seed, haven, x, z);
    }
    let near = tri(
        [q[0].drawn, q[1].drawn, q[2].drawn, q[3].drawn],
        x - x0 as f32,
        z - z0 as f32,
    );
    let (hx, hz) = (floor_i32(x * 2.0), floor_i32(z * 2.0));
    let shell = tri(
        [
            c.skin(seed, haven, hx, hz),
            c.skin(seed, haven, hx + 1, hz),
            c.skin(seed, haven, hx, hz + 1),
            c.skin(seed, haven, hx + 1, hz + 1),
        ],
        x * 2.0 - hx as f32,
        z * 2.0 - hz as f32,
    );
    near.max(shell)
}

/// Whether the sim's ground at (`x`, `z`) is a face: too steep to climb at
/// the nearest whole-metre point and all eight round it. The planner's law,
/// one cached lookup; a body's own is `movement::face_fall`, which reads the
/// drawn rock and so tells a shelf in it from a riser.
pub fn steep(
    occ: &mut crate::occupy::Occupants,
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
) -> bool {
    let c = occ
        .cache
        .coarse(seed, haven, floor_i32(x + 0.5), floor_i32(z + 0.5));
    c.gentlest >= terrain::CLIFF_SLOPE_RATIO
}

/// [`surface`] as the near ground's triangles carry it off the faces too —
/// the drawn ground everywhere, from whole-metre heights the cache already
/// holds, so it costs a nav probe no terrain tap of its own.
pub fn drawn_surface(
    occ: &mut crate::occupy::Occupants,
    seed: u64,
    haven: &terrain::Haven,
    x: f32,
    z: f32,
) -> f32 {
    let c = &mut *occ.cache;
    let (x0, z0) = (floor_i32(x), floor_i32(z));
    let q = [
        c.coarse(seed, haven, x0, z0),
        c.coarse(seed, haven, x0 + 1, z0),
        c.coarse(seed, haven, x0, z0 + 1),
        c.coarse(seed, haven, x0 + 1, z0 + 1),
    ];
    if q.iter().all(|q| q.gentlest <= SKIN_SLOPE_LO) {
        return tri(
            [q[0].drawn, q[1].drawn, q[2].drawn, q[3].drawn],
            x - x0 as f32,
            z - z0 as f32,
        );
    }
    surface(occ, seed, haven, x, z)
}

/// A quad's corners `[a (0,0), b (1,0), c (0,1), d (1,1)]` at (`fx`, `fz`),
/// split on the (1,0)–(0,1) diagonal as every ground mesh is.
#[inline]
fn tri([a, b, c, d]: [f32; 4], fx: f32, fz: f32) -> f32 {
    if fx + fz <= 1.0 {
        a + (b - a) * fx + (c - a) * fz
    } else {
        d + (c - d) * (1.0 - fx) + (b - d) * (1.0 - fz)
    }
}

/// The coarse point at whole-metre (`ix`, `iz`), with `height` giving the
/// sim's ground at any whole-metre point: [`coarse`]'s own answer, the 5×5
/// read only where the 3×3 of slopes round the point is steep enough to
/// carry relief or skin.
pub(crate) fn coarse_at(
    seed: u64,
    haven: &terrain::Haven,
    ix: i32,
    iz: i32,
    mut height: impl FnMut(i32, i32) -> f32,
) -> Coarse {
    let mut slope = |i: i32, j: i32| {
        relief_slope(
            height(i + 1, j),
            height(i - 1, j),
            height(i, j + 1),
            height(i, j - 1),
            1.0,
        )
    };
    let mut gentlest = f32::INFINITY;
    for dz in -1..=1 {
        for dx in -1..=1 {
            gentlest = gentlest.min(slope(ix + dx, iz + dz));
        }
    }
    let (x, z) = (ix as f32, iz as f32);
    if gentlest <= SKIN_SLOPE_LO {
        // No relief and no skin: [`coarse`]'s own answer, without the 5×5.
        let y = height(ix, iz);
        return Coarse {
            drawn: y,
            gentlest,
            fade: 0.0,
            sx: height(ix + 1, iz) - height(ix - 1, iz),
            sz: height(ix, iz + 1) - height(ix, iz - 1),
            hi: y,
        };
    }
    let mut h = [[0.0f32; 5]; 5];
    for (j, row) in h.iter_mut().enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            *v = height(ix + i as i32 - 2, iz + j as i32 - 2);
        }
    }
    coarse(seed, haven, x, z, &h)
}

/// How much of the skin's push survives at (`x`, `z`) beside `crags`, 0..1:
/// none over a crag's footprint, all of it [`CRAG_FADE_M`] clear, so a crag
/// stands out of the face rather than being swallowed by it.
pub fn crag_room<'a>(
    crags: impl IntoIterator<Item = &'a crate::cliff::Crag>,
    x: f32,
    z: f32,
) -> f32 {
    let mut room = 1.0f32;
    for c in crags {
        if !c.is_some() {
            continue;
        }
        let reach = c.hx + c.hz + CRAG_FADE_M;
        if fabs(x - c.x) > reach || fabs(z - c.z) > reach {
            continue;
        }
        let (lx, lz) = c.to_local(x, z);
        let (ox, oz) = ((fabs(lx) - c.hx).max(0.0), (fabs(lz) - c.hz).max(0.0));
        let t = ((ox * ox + oz * oz).sqrt() / CRAG_FADE_M).min(1.0);
        room = room.min(t * t * (3.0 - 2.0 * t));
    }
    room
}

/// One point of the skin's lattice.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinPoint {
    /// The shell's height: the drawn ground and the push, or sunk under the
    /// ground where the skin has faded out.
    pub y: f32,
    /// The drawn ground under it.
    pub drawn: f32,
    /// How far it stands out, as a share of [`SKIN_OUT_M`].
    pub out: f32,
    /// How fully the skin has grown here, 0..1.
    pub fade: f32,
    /// How far it stands out from its buttress, roughly -1..1.
    pub local: f32,
}

/// The skin at (`x`, `z`) inside the whole-metre quad whose corners are
/// `q` — `(x0, z0)`, `(x0 + 1, z0)`, `(x0, z0 + 1)`, `(x0 + 1, z0 + 1)` with
/// `x0`, `z0` the floors of `x`, `z` — with `room` the crags leave it.
pub fn skin_point(key: u32, x: f32, z: f32, q: &[Coarse; 4], room: f32) -> SkinPoint {
    let (fx, fz) = (x - floor_i32(x) as f32, z - floor_i32(z) as f32);
    let [a, b, c, d] = q;
    // The drawn ground as the near ring's triangles carry it.
    let drawn = tri([a.drawn, b.drawn, c.drawn, d.drawn], fx, fz);
    let lerp = |f: fn(&Coarse) -> f32| {
        let top = f(a) + (f(b) - f(a)) * fx;
        let bot = f(c) + (f(d) - f(c)) * fx;
        top + (bot - top) * fz
    };
    let fade = lerp(|q| q.fade) * room;
    if fade <= 0.0 {
        return SkinPoint {
            y: drawn - SKIN_SINK_M,
            drawn,
            ..SkinPoint::default()
        };
    }
    let (push, local) = push(x, drawn, z, key);
    let out = push * fade;
    // A push along the fall line is worth the face's slope in height.
    let (sx, sz) = (lerp(|q| q.sx), lerp(|q| q.sz));
    let lever = ((sx * sx + sz * sz).sqrt() * 0.5).min(RELIEF_SLOPE_CAP);
    let y = (drawn - SKIN_SINK_M * (1.0 - fade) + out * SKIN_OUT_M * lever).min(lerp(|q| q.hi));
    SkinPoint {
        y,
        drawn,
        out,
        fade,
        local,
    }
}
