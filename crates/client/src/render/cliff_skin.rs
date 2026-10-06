//! The rock skin: a faceted shell drawn over every face too steep to climb.
//!
//! Blocks stood on the smooth heightfield read as bricks on plaster: the face
//! between them was still one soft sheet, and a vertical block face on a 60°
//! slope barely stands proud of it. The reference game's cliffs are rock
//! meshes over the whole face — broken planes, shelves, overhangs. This is
//! that, procedurally: the drawn face on its own 1 m lattice, each vertex
//! pushed out horizontally (down the fall line) by a field built of planes —
//! piecewise-linear noise from buttress to slab size, so the face breaks into
//! flat facets meeting at creases — and stepped at the strata, so a bed that
//! stands out further than the one below overhangs it and one that stands
//! back leaves a shelf, which grows turf. Each facet is shaded smooth and
//! each crease sharp, and the recesses are darker.
//!
//! **Drawn, not decided.** The push is zero wherever any of a vertex's
//! neighbours is gentle enough for the sim to let a body climb, and it never
//! reaches further than the nearest walkable metre less [`SKIN_CLEAR_M`]:
//! nothing stands out over ground a body can stand on. It grows out of the
//! face over [`SKIN_FADE_M`] past that, and where it runs out the shell sinks
//! [`SKIN_SINK_M`] under the drawn ground and is hidden by it.

use bevy::prelude::*;
use sim_core::terrain::{self, Haven};

use super::boulders::RockSoup;
use super::terrain_mesh::{relief_slope, stencil_relief};

/// The most the face is pushed out, metres (horizontal).
const SKIN_OUT_M: f32 = 3.5;
/// Margin kept between the push and the nearest walkable ground, metres.
const SKIN_CLEAR_M: f32 = 1.0;
/// Past that margin, the push grows to its full reach over this, metres.
const SKIN_FADE_M: f32 = 3.0;
/// Rise/run where the skin starts: the sim's own climbing limit…
const SKIN_SLOPE_LO: f32 = terrain::CLIFF_SLOPE_RATIO;
/// …and where it is whole.
const SKIN_SLOPE_HI: f32 = terrain::CLIFF_SLOPE_RATIO * 1.4;
/// How far the shell sinks under the drawn ground where the mask runs out.
const SKIN_SINK_M: f32 = 0.3;
/// The buttress-sized facets' lattice, metres…
const FACET_BROAD_M: f32 = 15.0;
/// …the large ones'…
const FACET_M: f32 = 6.5;
/// …and the small ones'.
const FACET_FINE_M: f32 = 2.4;
/// Thickness of one stratum, metres.
const STRATUM_M: f32 = 4.5;
/// How far one stratum may stand out from the next, as a share of
/// [`SKIN_OUT_M`].
const STRATUM_STEP: f32 = 0.5;
/// Walkable ground is looked for this far off a vertex, metres.
const CLEAR_REACH: i32 = 4;
/// A facet facing up at least this much carries turf, in the green band:
/// gentle enough that the ground's per-pixel cliff veto leaves it some.
const SHELF_UP: f32 = 0.68;
/// Two triangles meeting at a vertex are shaded as one surface while their
/// normals are within this cosine (~23°); past it the crease shows.
const CREASE: f32 = 0.92;
/// The skin's value against the face's.
const SKIN_VALUE: f32 = 0.95;
/// How much darker the deepest recess is than the proudest facet.
const RECESS_DARK: f32 = 0.45;

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

/// Piecewise-linear value noise in [0, 1]: each unit cube is split into the
/// six tetrahedra that share its main diagonal and the hashed corner values
/// are interpolated linearly inside each, so the field is a mosaic of planes
/// meeting at creases — fractured rock rather than rolling ground. Read
/// through a fixed rotation so no crease runs along a world axis.
fn facet_noise(p: Vec3, key: u32) -> f32 {
    let p = Vec3::new(
        0.788 * p.x - 0.497 * p.y + 0.364 * p.z,
        0.326 * p.x + 0.837 * p.y + 0.440 * p.z,
        -0.523 * p.x - 0.229 * p.y + 0.821 * p.z,
    );
    let i = p.floor();
    let f = p - i;
    let base = [i.x as i32, i.y as i32, i.z as i32];
    // The axes in falling order of their fraction: the walk from the cube's
    // low corner to its high one through the tetrahedron holding `f`.
    let mut ax = [(f.x, 0usize), (f.y, 1), (f.z, 2)];
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

/// How far the face stands out at `p`, as a share of [`SKIN_OUT_M`], before
/// the mask and the clearance.
fn push(p: Vec3, key: u32) -> f32 {
    let broad = facet_noise(p / FACET_BROAD_M, key ^ 0xb0ad);
    let big = facet_noise(p / FACET_M, key ^ 0xfac3);
    let fine = facet_noise(p / FACET_FINE_M, key ^ 0x5ca1);
    // Strata: the bed's own stand-out, and how its face leans — out at the
    // top (an overhang) or back (a slab) — so the boundary between two beds
    // is a step: a shelf where the upper stands back, a roof where it
    // juts. The beds wander and dip, so no two run parallel for long.
    let lift = (facet_noise(p / 23.0, key ^ 0xbed5) - 0.5) * STRATUM_M * 1.6;
    let b = (p.y + lift) / STRATUM_M;
    let bed = b.floor();
    let f = b - bed;
    let bed_key = key ^ (bed as i32 as u32).wrapping_mul(0x2545_F491);
    // A bed's stand-out drifts along the face, so a shelf comes and goes.
    let drift = facet_noise(Vec3::new(p.x, 0.0, p.z) / 11.0, bed_key);
    let stand = (h01(bed_key, 1) * 0.6 + drift * 0.4 - 0.5) * 2.0 * STRATUM_STEP;
    let lean = (h01(bed_key, 2) - 0.4) * 0.4;
    let t = 0.35 * (broad - 0.5) * 2.0
        + 0.35 * (big - 0.5) * 2.0
        + 0.1 * (fine - 0.5) * 2.0
        + stand
        + lean * (f - 0.5);
    // Into (0, 1) without a clamp: a clamped stretch is the smooth face
    // again, laid out at a fixed offset.
    0.5 + t / (1.0 + 2.0 * t.abs())
}

/// How much of the skin a vertex carries, 0..1, from the gentlest slope of
/// the 3×3 round it: none on ground the sim lets a body climb, nor on an
/// authored site or in the haven, whose carved faces stay clean.
fn skin_mask(haven: &Haven, x: f32, z: f32, gentlest: f32) -> f32 {
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

/// The rock skin over one cliff cell's steep faces, into `soup`.
pub fn skin(soup: &mut RockSoup, seed: u64, haven: &Haven, cell_m: f32, cx: i32, cz: i32) {
    if let Some(pts) = shell(seed, haven, cell_m, cx, cz) {
        mesh(
            soup,
            &pts,
            cell_m as usize + 1,
            seed as u32 ^ (seed >> 32) as u32,
        );
    }
}

/// The skin's lattice over one cell, row by row (`cell_m + 1` a side): each
/// vertex's point and how far it stands out (0..1). `None` where nothing in
/// the cell does.
pub fn shell(seed: u64, haven: &Haven, cell_m: f32, cx: i32, cz: i32) -> Option<Vec<(Vec3, f32)>> {
    // The cell's vertices plus a border wide enough for the relief's 5×5
    // stencil, the 3×3 its mask reads the gentlest slope over, and the
    // search for walkable ground.
    let n = cell_m as i32;
    let m = CLEAR_REACH + 3;
    let side = (n + 1 + 2 * m) as usize;
    let (x0, z0) = (cx as f32 * cell_m, cz as f32 * cell_m);
    let gx = |k: usize| x0 + (k as i32 - m) as f32;
    let gz = |j: usize| z0 + (j as i32 - m) as f32;
    let mut lat = terrain::Lattice::new();
    // A cell with no steep ground anywhere costs nine taps.
    let mut steep = false;
    for j in 0..3 {
        for i in 0..3 {
            let (x, z) = (
                x0 + cell_m * (0.17 + 0.33 * i as f32),
                z0 + cell_m * (0.17 + 0.33 * j as f32),
            );
            steep |= terrain::ground_slope_memo(&mut lat, seed, haven, x, z)
                > terrain::CLIFF_SLOPE_RATIO * 0.8;
        }
    }
    if !steep {
        return None;
    }
    let mut h = vec![0.0f32; side * side];
    for j in 0..side {
        for k in 0..side {
            h[j * side + k] = terrain::ground_memo(&mut lat, seed, haven, gx(k), gz(j));
        }
    }
    let at = |j: usize, k: usize| h[j * side + k];
    let slope = |j: usize, k: usize| {
        relief_slope(at(j, k + 1), at(j, k - 1), at(j + 1, k), at(j - 1, k), 1.0)
    };
    // Walkable ground, by the sim's own law.
    let mut walk = vec![false; side * side];
    for j in 1..side - 1 {
        for k in 1..side - 1 {
            walk[j * side + k] = slope(j, k) < terrain::CLIFF_SLOPE_RATIO;
        }
    }
    let key = seed as u32 ^ (seed >> 32) as u32;
    let v = (n + 1) as usize;
    // Per vertex: the shell's point, and how far it stands out (0..1).
    let mut pts = vec![(Vec3::ZERO, 0.0f32); v * v];
    let mut any = false;
    for vj in 0..v {
        for vk in 0..v {
            let (j, k) = (vj + m as usize, vk + m as usize);
            let (x, z) = (gx(k), gz(j));
            let y = at(j, k);
            let mut gentlest = slope(j, k);
            for dj in 0..3 {
                for dk in 0..3 {
                    gentlest = gentlest.min(slope(j + dj - 1, k + dk - 1));
                }
            }
            let mut stencil = [[0.0f32; 5]; 5];
            for (dj, row) in stencil.iter_mut().enumerate() {
                for (dk, s) in row.iter_mut().enumerate() {
                    *s = at(j + dj - 2, k + dk - 2);
                }
            }
            let r = stencil_relief(seed, haven, x, z, &stencil, 1.0);
            let drawn = if r != 0.0 { y + r } else { y };
            let mask = if y < terrain::SEA_LEVEL {
                0.0
            } else {
                skin_mask(haven, x, z, gentlest)
            };
            if mask <= 0.0 {
                pts[vj * v + vk] = (Vec3::new(x, drawn - SKIN_SINK_M, z), 0.0);
                continue;
            }
            any = true;
            // Down the fall line, off the sim's own gradient.
            let (sx, sz) = (at(j, k + 1) - at(j, k - 1), at(j + 1, k) - at(j - 1, k));
            let g = (sx * sx + sz * sz).sqrt().max(1e-4);
            let (dx, dz) = (-sx / g, -sz / g);
            // Never out over ground a body can reach, and grown out of the
            // ground gradually rather than standing up at its edge.
            let mut clear = CLEAR_REACH as f32 + 1.0;
            for dj in -CLEAR_REACH..=CLEAR_REACH {
                for dk in -CLEAR_REACH..=CLEAR_REACH {
                    if walk[(j as i32 + dj) as usize * side + (k as i32 + dk) as usize] {
                        clear = clear.min(((dj * dj + dk * dk) as f32).sqrt());
                    }
                }
            }
            let e = ((clear - SKIN_CLEAR_M) / SKIN_FADE_M).clamp(0.0, 1.0);
            let fade = mask * e * e * (3.0 - 2.0 * e);
            let p = Vec3::new(x, drawn, z);
            let out = (push(p, key) * fade).min((clear - SKIN_CLEAR_M).max(0.0) / SKIN_OUT_M);
            let d = out * SKIN_OUT_M;
            let sink = SKIN_SINK_M * (1.0 - fade);
            pts[vj * v + vk] = (Vec3::new(x + dx * d, drawn - sink, z + dz * d), out);
        }
    }
    any.then_some(pts)
}

/// Mesh the skin's lattice `pts` (`v` a side) into `soup`.
fn mesh(soup: &mut RockSoup, pts: &[(Vec3, f32)], v: usize, key: u32) {
    // The terrain's own split, on the (1,0)–(0,1) diagonal, so where the
    // skin stands nowhere out it lies exactly under the drawn ground: quad
    // `(j, k)` is triangles `a c b` and `b c d` over its corners `a (j, k)`,
    // `b (j, k + 1)`, `c (j + 1, k)` and `d (j + 1, k + 1)`.
    let corners = |j: usize, k: usize, t: usize| -> [usize; 3] {
        let (a, b, c, d) = (
            j * v + k,
            j * v + k + 1,
            (j + 1) * v + k,
            (j + 1) * v + k + 1,
        );
        if t == 0 {
            [a, c, b]
        } else {
            [b, c, d]
        }
    };
    let q = v - 1;
    let mut face = vec![Vec3::ZERO; q * q * 2];
    for j in 0..q {
        for k in 0..q {
            for t in 0..2 {
                let [i0, i1, i2] = corners(j, k, t);
                let (p0, p1, p2) = (pts[i0].0, pts[i1].0, pts[i2].0);
                face[(j * q + k) * 2 + t] = (p1 - p0).cross(p2 - p0).normalize_or_zero();
            }
        }
    }
    // The six triangles round vertex `(j, k)`, as indices into `face`.
    let around = |j: usize, k: usize| {
        let mut f = [usize::MAX; 6];
        let mut put = |slot: usize, qj: usize, qk: usize, t: usize, ok: bool| {
            if ok {
                f[slot] = (qj * q + qk) * 2 + t;
            }
        };
        put(0, j, k, 0, j < q && k < q);
        put(1, j, k.wrapping_sub(1), 0, j < q && k > 0);
        put(2, j, k.wrapping_sub(1), 1, j < q && k > 0);
        put(3, j.wrapping_sub(1), k, 0, j > 0 && k < q);
        put(4, j.wrapping_sub(1), k, 1, j > 0 && k < q);
        put(5, j.wrapping_sub(1), k.wrapping_sub(1), 1, j > 0 && k > 0);
        f
    };
    let green = |y: f32| y > terrain::BEACH_MAX_H + 1.5 && y < terrain::TREELINE_H;
    // Recesses darker: the cheap half of occlusion.
    let value = |o: f32| SKIN_VALUE * (1.0 - RECESS_DARK * (1.0 - o.clamp(0.0, 1.0)));
    for j in 0..q {
        for k in 0..q {
            for t in 0..2 {
                let idx = corners(j, k, t);
                if idx.iter().all(|&i| pts[i].1 <= 0.0) {
                    continue;
                }
                let nf = face[(j * q + k) * 2 + t];
                // Smooth across a facet, sharp across a crease: each corner
                // averages only the triangles round it that face within
                // [`CREASE`] of this one.
                let mut n = [nf; 3];
                for (c, &i) in idx.iter().enumerate() {
                    let (vj, vk) = (i / v, i % v);
                    let mut sum = Vec3::ZERO;
                    for fi in around(vj, vk) {
                        if fi != usize::MAX && face[fi].dot(nf) > CREASE {
                            sum += face[fi];
                        }
                    }
                    n[c] = sum.normalize_or(nf);
                }
                let p = [pts[idx[0]].0, pts[idx[1]].0, pts[idx[2]].0];
                // Turf on what faces up, in patches.
                let ctr = (p[0] + p[1] + p[2]) / 3.0;
                let moss = if green(ctr.y) && nf.y > SHELF_UP {
                    let up = ((nf.y - SHELF_UP) / 0.15).clamp(0.0, 1.0);
                    let patch = (facet_noise(ctr / 3.0, key ^ 0x6a55) * 2.0 - 0.3).clamp(0.0, 1.0);
                    up * patch
                } else {
                    0.0
                };
                soup.tri_n(
                    p,
                    n,
                    [moss; 3],
                    [
                        value(pts[idx[0]].1),
                        value(pts[idx[1]].1),
                        value(pts[idx[2]].1),
                    ],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skin stands out only over ground the sim refuses a body: every
    /// pushed point, and the ground half a metre round it, is too steep to
    /// walk. On the cliff north of 1500,600 (seed 20260731), 75 m of face.
    #[test]
    fn the_skin_never_stands_over_walkable_ground() {
        let seed = 20260731;
        let haven = sim_core::world::World::new(seed).haven;
        let mut pushed = 0;
        for cz in 19..23 {
            for cx in 45..48 {
                let Some(pts) = shell(seed, &haven, 32.0, cx, cz) else {
                    continue;
                };
                for (p, out) in pts {
                    if out <= 0.0 {
                        continue;
                    }
                    pushed += 1;
                    for (dx, dz) in [(0.0, 0.0), (0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5)] {
                        let s = terrain::ground_slope(seed, &haven, p.x + dx, p.z + dz);
                        assert!(
                            s >= terrain::CLIFF_SLOPE_RATIO,
                            "skin point {p} (out {out:.2}) stands over walkable ground (slope {s:.2})"
                        );
                    }
                }
            }
        }
        assert!(
            pushed > 1000,
            "only {pushed} skin points stand out: no cliff here"
        );
    }
}
