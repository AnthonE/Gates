//! The particle sprites: one 256² sheet of 4×4 cells, generated rather than
//! loaded — white, the shape in the alpha — so every sprite takes its
//! particle's colour.
//!
//! A game picks which [`Shape`] sits in each cell (its `CELLS` layout) and
//! keeps its own cell constants beside it; [`sheet`] draws level 0 and the
//! game builds the mips its own way. A cell's noise is seeded by its index,
//! so a layout that keeps a cell keeps its pixels.

pub const SPRITE_COLS: u32 = 4;
pub const SPRITE_ROWS: u32 = 4;
/// One sprite's side, texels.
pub const SPRITE_TEX: u32 = 64;
/// Cells on the sheet.
pub const CELLS: usize = (SPRITE_COLS * SPRITE_ROWS) as usize;

/// What one cell holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// A hot core and a long soft skirt: a spark reads as light without bloom.
    Glow,
    /// A streak along u, soft across v, tapering at both ends.
    Streak,
    /// A star flash of `5 + n` spikes of rolled length around a hot centre.
    Star(u8),
    /// A ragged soft blob with some body in it.
    Puff,
    /// Softer and wispier than a puff.
    Smoke,
    /// Round, a little brighter at its top.
    Droplet,
    /// A thin ring (a ripple on water, a shock front seen flat).
    Ring,
    /// A small hard dot (grit).
    Speck,
    /// An ellipse with a midrib.
    Leaf,
    /// A long thin sliver.
    Splinter,
    /// A flame tongue, tip up: drawn unrolled, or the fire leans.
    Flame,
    /// A jagged bolt of lightning along u with a branch off it.
    Arc,
    /// A long six-sided crystal along u, its spine bright.
    Shard,
    /// Nothing.
    Empty,
}

/// Gates' sheet, and the default: glow, streak, three stars, three puffs, two
/// smokes, droplet, ring, speck, leaf, splinter, flame.
pub const DEFAULT: [Shape; CELLS] = [
    Shape::Glow,
    Shape::Streak,
    Shape::Star(0),
    Shape::Star(1),
    Shape::Star(2),
    Shape::Puff,
    Shape::Puff,
    Shape::Puff,
    Shape::Smoke,
    Shape::Smoke,
    Shape::Droplet,
    Shape::Ring,
    Shape::Speck,
    Shape::Leaf,
    Shape::Splinter,
    Shape::Flame,
];

/// A hash of two words in `[0, 1)`.
pub fn hash01(a: u32, b: u32) -> f32 {
    let mut h = a.wrapping_mul(0x85eb_ca6b) ^ b.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    (h >> 8) as f32 / 16_777_216.0
}

pub fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Value noise on the integer lattice.
pub fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (xf as i32 as u32, yf as i32 as u32);
    let h = |a: u32, b: u32| hash01(a.wrapping_mul(0x9E37_79B1) ^ seed, b ^ seed.rotate_left(16));
    let (a, b) = (h(xi, yi), h(xi.wrapping_add(1), yi));
    let (c, d) = (
        h(xi, yi.wrapping_add(1)),
        h(xi.wrapping_add(1), yi.wrapping_add(1)),
    );
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sy
}

/// Three octaves of [`vnoise`].
pub fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    vnoise(x, y, seed) * 0.55
        + vnoise(x * 2.1, y * 2.1, seed ^ 0x5A) * 0.3
        + vnoise(x * 4.3, y * 4.3, seed ^ 0xC3) * 0.15
}

/// Coverage of `shape` drawn in cell `cell` at `(u, v) ∈ [-1, 1]²`
/// (value, alpha).
pub fn sprite(shape: Shape, cell: u8, u: f32, v: f32) -> (f32, f32) {
    let seed = 0xF00D_0000 ^ (cell as u32).wrapping_mul(0x0101_0101);
    let r = (u * u + v * v).sqrt();
    let (value, alpha) = match shape {
        Shape::Glow => {
            let core = smooth(0.35, 0.0, r);
            let skirt = (1.0 - r).max(0.0).powi(3);
            (1.0, (core * 0.9 + skirt * 0.6).min(1.0))
        }
        Shape::Streak => {
            let across = smooth(1.0, 0.0, v.abs()).powi(2);
            let ends = smooth(1.0, 0.7, u.abs());
            (1.0, across * ends)
        }
        Shape::Star(n) => {
            let a = v.atan2(u);
            let spikes = 5 + n as u32;
            let mut s: f32 = 0.0;
            for k in 0..spikes {
                let ang =
                    (k as f32 + 0.3 * hash01(k, seed)) / spikes as f32 * core::f32::consts::TAU;
                let len = 0.6 + 0.4 * hash01(k, seed ^ 0x11);
                let d = (a - ang + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU)
                    - core::f32::consts::PI;
                let w = 0.18 * (1.0 - r / len).max(0.0);
                s = s
                    .max(smooth(w, 0.0, d.abs() * r.max(0.05)) * (1.0 - smooth(len * 0.6, len, r)));
            }
            let core = smooth(0.3, 0.0, r);
            (1.0, s.max(core))
        }
        Shape::Puff => {
            let a = v.atan2(u);
            let edge = 0.75 + 0.25 * vnoise(a.cos() * 1.8 + 3.0, a.sin() * 1.8 + 3.0, seed);
            let body = smooth(edge, edge * 0.35, r);
            let n = fbm(u * 3.0, v * 3.0, seed);
            (0.8 + 0.2 * n, body * (0.55 + 0.45 * n))
        }
        Shape::Smoke => {
            let n = fbm(u * 2.2 + 5.0, v * 2.2, seed);
            let body = smooth(1.0, 0.2, r + 0.25 * (n - 0.5));
            (0.85 + 0.15 * n, body * (0.35 + 0.65 * n))
        }
        Shape::Droplet => {
            let body = smooth(0.9, 0.6, r);
            let hi = smooth(0.45, 0.0, ((u + 0.25).powi(2) + (v + 0.3).powi(2)).sqrt());
            (0.8 + 0.2 * hi, body)
        }
        Shape::Ring => (
            1.0,
            smooth(0.18, 0.0, (r - 0.72).abs()) * smooth(1.0, 0.9, r),
        ),
        Shape::Speck => (1.0, smooth(0.85, 0.55, r)),
        Shape::Leaf => {
            let e = (u * u + (v * 2.4).powi(2)).sqrt();
            let rib = 1.0 - 0.25 * smooth(0.06, 0.0, v.abs());
            (rib, smooth(0.95, 0.8, e))
        }
        Shape::Splinter => {
            let e = (u * u + (v * 5.0).powi(2)).sqrt();
            (1.0, smooth(0.95, 0.75, e))
        }
        // A round foot, widest a third of the way up, then a long taper to a
        // tip that noise bends a little, so overlapping tongues do not stack
        // into one shape. Brightest low in the middle, where a flame is
        // hottest.
        Shape::Flame => {
            let h = (1.0 - v) * 0.5;
            let x = u - 0.3 * h * (vnoise(h * 2.5, 0.5, seed) - 0.5);
            let w = if h < 0.3 {
                0.46 * (h / 0.3).max(0.0).sqrt()
            } else {
                0.46 * ((1.0 - h) / 0.7).max(0.0).powf(1.4)
            };
            let body = if w > 1e-3 {
                smooth(w, w * 0.4, x.abs())
            } else {
                0.0
            };
            let core = smooth(
                0.7,
                0.0,
                ((x / 0.28).powi(2) + ((h - 0.3) / 0.32).powi(2)).sqrt(),
            );
            let n = fbm(u * 2.5, v * 3.0 + 7.0, seed);
            (0.5 + 0.5 * core, body * (0.75 + 0.25 * n))
        }
        // A bright jagged thread along u, a branch leaving it a third of the
        // way along, a soft halo round both, fading at the ends.
        Shape::Arc => {
            let path = |x: f32, s: u32| {
                0.32 * (vnoise(x * 2.5 + 1.0, 0.5, s) - 0.5)
                    + 0.18 * (vnoise(x * 7.0 + 3.0, 1.5, s ^ 0x3) - 0.5)
                    + 0.07 * (vnoise(x * 19.0, 2.5, s ^ 0x7) - 0.5)
            };
            let d = (v - path(u, seed)).abs();
            let thread = smooth(0.05, 0.0, d) + 0.45 * smooth(0.3, 0.0, d);
            let bu = (u + 0.3).max(0.0);
            let bv =
                path(-0.3, seed) + bu * 0.9 + 0.12 * (vnoise(bu * 9.0, 4.0, seed ^ 0x11) - 0.5);
            let branch = if bu > 0.0 && bu < 0.55 {
                let bd = (v - bv).abs();
                (smooth(0.035, 0.0, bd) + 0.3 * smooth(0.18, 0.0, bd)) * (1.0 - bu / 0.55)
            } else {
                0.0
            };
            let ends = smooth(1.0, 0.75, u.abs());
            (1.0, (thread.max(branch) * ends).min(1.0))
        }
        // Pointed at both ends, its faces a little darker than its spine.
        Shape::Shard => {
            let half = 0.24 * (1.0 - u.abs().powf(1.3)).max(0.0);
            let body = if half > 1e-3 {
                smooth(half, half * 0.75, v.abs())
            } else {
                0.0
            };
            let spine = smooth(0.06, 0.0, v.abs());
            let facet = if v > 0.0 { 0.7 } else { 0.85 };
            ((facet + (1.0 - facet) * spine).min(1.0), body)
        }
        Shape::Empty => (0.0, 0.0),
    };
    // Nothing reaches the sprite's own edge.
    let pad = 1.0 - smooth(0.9, 0.98, u.abs().max(v.abs()));
    (value.clamp(0.0, 1.0), (alpha * pad).clamp(0.0, 1.0))
}

/// The sheet's level 0 for `layout`: RGBA8, sRGB-encoded, `(bytes, w, h)`.
pub fn sheet(layout: &[Shape; CELLS]) -> (Vec<u8>, u32, u32) {
    let (w, h) = (SPRITE_COLS * SPRITE_TEX, SPRITE_ROWS * SPRITE_TEX);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (cell, &shape) in layout.iter().enumerate() {
        let cell = cell as u8;
        let (cx, cy) = (cell as u32 % SPRITE_COLS, cell as u32 / SPRITE_COLS);
        for y in 0..SPRITE_TEX {
            for x in 0..SPRITE_TEX {
                let u = (x as f32 + 0.5) / SPRITE_TEX as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / SPRITE_TEX as f32 * 2.0 - 1.0;
                let (val, a) = sprite(shape, cell, u, v);
                let i = (((cy * SPRITE_TEX + y) * w + cx * SPRITE_TEX + x) * 4) as usize;
                let g = (val * 255.0 + 0.5) as u8;
                data[i] = g;
                data[i + 1] = g;
                data[i + 2] = g;
                data[i + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
    }
    (data, w, h)
}

/// The uv rectangle of `cell` (min, max), inset half a texel.
pub fn cell_uv(cell: u8) -> ([f32; 2], [f32; 2]) {
    let (cu, cv) = (
        (cell as u32 % SPRITE_COLS) as f32,
        (cell as u32 / SPRITE_COLS) as f32,
    );
    let inset = 0.5 / SPRITE_TEX as f32;
    (
        [
            (cu + inset) / SPRITE_COLS as f32,
            (cv + inset) / SPRITE_ROWS as f32,
        ],
        [
            (cu + 1.0 - inset) / SPRITE_COLS as f32,
            (cv + 1.0 - inset) / SPRITE_ROWS as f32,
        ],
    )
}
