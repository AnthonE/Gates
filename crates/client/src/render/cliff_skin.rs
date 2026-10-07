//! The rock skin: a faceted shell drawn over every face too steep to climb.
//!
//! Blocks stood on the smooth heightfield read as bricks on plaster: the face
//! between them was still one soft sheet, and a vertical block face on a 60°
//! slope barely stands proud of it. The reference game's cliffs are rock
//! meshes over the whole face — broken planes, shelves, overhangs. This is
//! that, procedurally: the drawn face on a half-metre lattice, each vertex
//! pushed out horizontally (down the fall line) by a field built of planes —
//! piecewise-linear noise from buttress to slab size, so the face breaks into
//! flat facets meeting at creases — and stepped at the strata, so a bed that
//! stands out further than the one below overhangs it and one that stands
//! back leaves a shelf, which grows turf. What stands out is lit and what is
//! recessed is darker, and each large facet is its own shade.
//!
//! **Drawn, not decided.** The push is zero wherever any of a vertex's
//! neighbours is gentle enough for the sim to let a body climb, and it never
//! reaches further than the nearest walkable metre ahead of it less
//! [`SKIN_CLEAR_M`]: nothing stands out over ground a body can stand on. It grows out of the
//! face over [`SKIN_FADE_M`] past that, and where it runs out the shell sinks
//! [`SKIN_SINK_M`] under the drawn ground and is hidden by it.

use bevy::prelude::*;
use sim_core::terrain::{self, Haven};

use super::boulders::RockSoup;
use super::clutter::tuft;
use super::props::Soup;
use super::terrain_mesh::{relief_slope, stencil_relief, vertex_mods};

/// The most the face is pushed out, metres (horizontal).
const SKIN_OUT_M: f32 = 3.5;
/// Margin kept between the push and the nearest walkable lattice point,
/// metres: the walkable ground itself can reach half a lattice diagonal
/// nearer, and a point between lattice points reads its clearance
/// interpolated.
const SKIN_CLEAR_M: f32 = 1.6;
/// Walkable ground further than this behind a vertex (up the fall line) is
/// no bar to its push, which heads the other way, metres.
const CLEAR_BEHIND_M: f32 = 1.5;
/// Past that margin, the push grows to its full reach over this, metres.
const SKIN_FADE_M: f32 = 2.0;
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
const STRATUM_STEP: f32 = 0.42;
/// The skin's lattice pitch, metres: half the near ground's, so a stratum's
/// step resolves as a step and not a sawtooth.
const SKIN_STEP_M: f32 = 0.5;
/// How much of a stratum its step from the one below takes.
const STEP_BAND: f32 = 0.35;
/// Walkable ground is looked for this far off a vertex, metres.
const CLEAR_REACH: i32 = 5;
/// A facet facing up at least this much carries turf, in the green band.
/// The ground's per-pixel cliff veto keeps the steeper of it bare, and the
/// tufts cling on there anyway.
const SHELF_UP: f32 = 0.55;
/// A shelf triangle grows a grass tuft past this much turf…
const TUFT_MOSS: f32 = 0.3;
/// …with this chance per unit of turf (a triangle is an eighth of a square
/// metre).
const TUFT_SHARE: f32 = 0.6;
/// Two triangles meeting at a vertex are shaded as one surface while their
/// normals are within this cosine (~26°); past it the crease shows.
const CREASE: f32 = 0.9;
/// How much brighter what stands out from its buttress is than what is
/// recessed, as a value multiplier, either way.
const VALUE_RELIEF: f32 = 0.3;
/// Each large facet's own shade, either way.
const FACET_SHADE: f32 = 0.1;
/// How much darker a surface is per unit it faces away from the sky past a
/// 60° face's own.
const SKY_DARK: f32 = 0.6;

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
/// the mask and the clearance; and how far it stands out from the buttress
/// it is part of, roughly -1..1, which is what reads as proud or recessed.
fn push(p: Vec3, key: u32) -> (f32, f32) {
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
    // The step from the bed below over the foot of this one, so it is a
    // short steep band the lattice can draw.
    let below = stratum(bed - 1.0, 1.0, p, key);
    let s = (f / STEP_BAND).min(1.0);
    let strata = below + (stratum(bed, f, p, key) - below) * s * s * (3.0 - 2.0 * s);
    let local = 0.42 * (big - 0.5) * 2.0 + 0.16 * (fine - 0.5) * 2.0 + strata;
    let t = 0.35 * (broad - 0.5) * 2.0 + local;
    // Into (0, 1) without a clamp: a clamped stretch is the smooth face
    // again, laid out at a fixed offset.
    (0.5 + t / (1.0 + 2.0 * t.abs()), local)
}

/// Bed `bed`'s stand-out at height `f` (0..1) up it: its own, drifting
/// along the face so a shelf comes and goes, and how its face leans.
fn stratum(bed: f32, f: f32, p: Vec3, key: u32) -> f32 {
    let bed_key = key ^ (bed as i32 as u32).wrapping_mul(0x2545_F491);
    let drift = facet_noise(Vec3::new(p.x, 0.0, p.z) / 11.0, bed_key);
    let stand = (h01(bed_key, 1) * 0.6 + drift * 0.4 - 0.5) * 2.0 * STRATUM_STEP;
    let lean = (h01(bed_key, 2) - 0.4) * 0.4;
    stand + lean * (f - 0.5)
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

/// One point of the skin's lattice.
#[derive(Clone, Copy, Debug)]
pub struct SkinPoint {
    /// Where it is drawn.
    pub p: Vec3,
    /// How far it stands out, as a share of [`SKIN_OUT_M`].
    pub out: f32,
    /// How fully the skin has grown here, 0..1: none where it lies under
    /// the ground.
    pub fade: f32,
    /// The ground material's value multiplier here.
    pub value: f32,
}

/// The skin's lattice over one cell, [`SKIN_STEP_M`] apart and one point
/// past the cell on every side, so a vertex on the cell's edge is shaded
/// from the triangles of both cells alike.
pub struct Shell {
    /// Points a side.
    pub side: usize,
    /// Row by row, from the cell's low corner less one step.
    pub pts: Vec<SkinPoint>,
}

/// The rock skin over one cliff cell's steep faces, into `soup`, and the
/// grass on its shelves into `tufts`.
pub(super) fn skin(
    soup: &mut RockSoup,
    tufts: &mut Soup,
    seed: u64,
    haven: &Haven,
    cell_m: f32,
    cx: i32,
    cz: i32,
) {
    if let Some(shell) = shell(seed, haven, cell_m, cx, cz) {
        mesh(soup, tufts, &shell, seed as u32 ^ (seed >> 32) as u32);
    }
}

/// The skin's lattice over one cell; `None` where nothing in it stands out.
pub fn shell(seed: u64, haven: &Haven, cell_m: f32, cx: i32, cz: i32) -> Option<Shell> {
    // The 1 m lattice the near ground is drawn on, over the cell and a point
    // past it, plus a border wide enough for the relief's 5×5 stencil, the
    // 3×3 the mask reads the gentlest slope over, and the search for
    // walkable ground.
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
    // Per 1 m point from one before the cell to one past it: the drawn
    // ground, how fully the skin has grown, the clearance to walkable ground
    // and the sim's gradient.
    let nc = (n + 3) as usize;
    let mut coarse = vec![(0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32); nc * nc];
    for cj in 0..nc {
        for ck in 0..nc {
            let (j, k) = (cj + m as usize - 1, ck + m as usize - 1);
            let (x, z) = (gx(k), gz(j));
            let y = at(j, k);
            let mut stencil = [[0.0f32; 5]; 5];
            for (dj, row) in stencil.iter_mut().enumerate() {
                for (dk, s) in row.iter_mut().enumerate() {
                    *s = at(j + dj - 2, k + dk - 2);
                }
            }
            let r = stencil_relief(seed, haven, x, z, &stencil, 1.0);
            let drawn = if r != 0.0 { y + r } else { y };
            let mut gentlest = slope(j, k);
            for dj in 0..3 {
                for dk in 0..3 {
                    gentlest = gentlest.min(slope(j + dj - 1, k + dk - 1));
                }
            }
            let mask = if y < terrain::SEA_LEVEL {
                0.0
            } else {
                skin_mask(haven, x, z, gentlest)
            };
            let (sx, sz) = (at(j, k + 1) - at(j, k - 1), at(j + 1, k) - at(j - 1, k));
            // The walkable ground the push heads toward: the face's foot and
            // flanks, not the top it falls away from.
            let mut clear = CLEAR_REACH as f32 + 1.0;
            if mask > 0.0 {
                let g = (sx * sx + sz * sz).sqrt().max(1e-4);
                let (dx, dz) = (-sx / g, -sz / g);
                for dj in -CLEAR_REACH..=CLEAR_REACH {
                    for dk in -CLEAR_REACH..=CLEAR_REACH {
                        if walk[(j as i32 + dj) as usize * side + (k as i32 + dk) as usize]
                            && dk as f32 * dx + dj as f32 * dz > -CLEAR_BEHIND_M
                        {
                            clear = clear.min(((dj * dj + dk * dk) as f32).sqrt());
                        }
                    }
                }
            }
            // Grown out of the ground gradually rather than standing up at
            // its edge.
            let e = ((clear - SKIN_CLEAR_M) / SKIN_FADE_M).clamp(0.0, 1.0);
            let fade = mask * e * e * (3.0 - 2.0 * e);
            coarse[cj * nc + ck] = (drawn, fade, clear, sx, sz);
        }
    }
    let key = seed as u32 ^ (seed >> 32) as u32;
    let per = (1.0 / SKIN_STEP_M).round() as i32;
    let nf = (n * per + 3) as usize;
    let mut pts = Vec::with_capacity(nf * nf);
    let mut any = false;
    for fj in 0..nf {
        for fk in 0..nf {
            // Metres from the cell's low corner, then the 1 m quad holding
            // the point and where in it.
            let (u, w) = (
                (fk as i32 - 1) as f32 * SKIN_STEP_M,
                (fj as i32 - 1) as f32 * SKIN_STEP_M,
            );
            let (k0, j0) = (u.floor(), w.floor());
            let (fx, fz) = (u - k0, w - j0);
            let (ck, cj) = ((k0 as i32 + 1) as usize, (j0 as i32 + 1) as usize);
            let c = |dj: usize, dk: usize| coarse[(cj + dj) * nc + ck + dk];
            let (a, b, cc, d) = (c(0, 0), c(0, 1), c(1, 0), c(1, 1));
            // The drawn ground as the near ring's triangles carry it: quads
            // split on the (1,0)–(0,1) diagonal (`near_drawn_y`).
            let drawn = if fx + fz <= 1.0 {
                a.0 + (b.0 - a.0) * fx + (cc.0 - a.0) * fz
            } else {
                d.0 + (cc.0 - d.0) * (1.0 - fx) + (b.0 - d.0) * (1.0 - fz)
            };
            let lerp = |f: fn(&(f32, f32, f32, f32, f32)) -> f32| {
                let top = f(&a) + (f(&b) - f(&a)) * fx;
                let bot = f(&cc) + (f(&d) - f(&cc)) * fx;
                top + (bot - top) * fz
            };
            let (x, z) = (x0 + u, z0 + w);
            let fade = lerp(|q| q.1);
            let base = vertex_mods(drawn, x, z, 0.0)[0];
            if fade <= 0.0 {
                pts.push(SkinPoint {
                    p: Vec3::new(x, drawn - SKIN_SINK_M, z),
                    out: 0.0,
                    fade: 0.0,
                    value: base,
                });
                continue;
            }
            any = true;
            let clear = lerp(|q| q.2);
            // Down the fall line, off the sim's own gradient.
            let (sx, sz) = (lerp(|q| q.3), lerp(|q| q.4));
            let g = (sx * sx + sz * sz).sqrt().max(1e-4);
            let (dx, dz) = (-sx / g, -sz / g);
            let p = Vec3::new(x, drawn, z);
            let (push, local) = push(p, key);
            // Never out over ground a body can reach.
            let out = (push * fade).min((clear - SKIN_CLEAR_M).max(0.0) / SKIN_OUT_M);
            let d = out * SKIN_OUT_M;
            // What stands out is lit, what is recessed is darker — the
            // cheap half of occlusion — and each facet is its own shade;
            // both grow in with the skin, so where it meets the ground the
            // two are one value.
            let shade = VALUE_RELIEF * local.clamp(-1.0, 1.0)
                + FACET_SHADE * (facet_noise(p / FACET_M, key ^ 0x7a1e) * 2.0 - 1.0);
            pts.push(SkinPoint {
                p: Vec3::new(x + dx * d, drawn - SKIN_SINK_M * (1.0 - fade), z + dz * d),
                out,
                fade,
                value: base * (1.0 + shade * fade),
            });
        }
    }
    any.then_some(Shell { side: nf, pts })
}

/// Mesh the skin's lattice into `soup`: the cell's own quads, each split as
/// the near ground's are so that where the skin stands nowhere out it lies
/// exactly under the drawn ground. Shaded smooth across a facet and sharp
/// across a crease: a corner takes the triangles round its vertex that face
/// within [`CREASE`] of its own, and the vertex is shared by every corner
/// that took the same ones.
fn mesh(soup: &mut RockSoup, tufts: &mut Soup, shell: &Shell, key: u32) {
    let v = shell.side;
    let q = v - 1;
    let pts = &shell.pts;
    // Quad `(j, k)` is triangles `a c b` and `b c d` over its corners
    // `a (j, k)`, `b (j, k + 1)`, `c (j + 1, k)` and `d (j + 1, k + 1)`.
    let tris = |j: usize, k: usize| {
        let (a, b, c, d) = (
            j * v + k,
            j * v + k + 1,
            (j + 1) * v + k,
            (j + 1) * v + k + 1,
        );
        [[a, c, b], [b, c, d]]
    };
    // Every triangle of the lattice, the ring past the cell included, as
    // its area-weighted normal.
    let mut face = vec![Vec3::ZERO; q * q * 2];
    for j in 0..q {
        for k in 0..q {
            for (t, c) in tris(j, k).into_iter().enumerate() {
                let (p0, p1, p2) = (pts[c[0]].p, pts[c[1]].p, pts[c[2]].p);
                face[(j * q + k) * 2 + t] = (p1 - p0).cross(p2 - p0);
            }
        }
    }
    // The six triangles round vertex `(j, k)`, by slot.
    let around = |j: usize, k: usize| -> [Option<usize>; 6] {
        let f = |qj: usize, qk: usize, t: usize| (qj * q + qk) * 2 + t;
        [
            (j < q && k < q).then(|| f(j, k, 0)),
            (j < q && k > 0).then(|| f(j, k - 1, 0)),
            (j < q && k > 0).then(|| f(j, k - 1, 1)),
            (j > 0 && k < q).then(|| f(j - 1, k, 0)),
            (j > 0 && k < q).then(|| f(j - 1, k, 1)),
            (j > 0 && k > 0).then(|| f(j - 1, k - 1, 1)),
        ]
    };
    let green = |y: f32| y > terrain::BEACH_MAX_H + 1.5 && y < terrain::TREELINE_H;
    // Per lattice point, the vertices made so far, by the set of triangles
    // their normal took.
    let mut made: Vec<[(u8, u32, f32); 6]> = vec![[(0, u32::MAX, 0.0); 6]; v * v];
    let mut corner = |soup: &mut RockSoup, i: usize, own: usize| -> (u32, f32) {
        let nf = face[own].normalize_or_zero();
        let mut set = 0u8;
        let mut sum = Vec3::ZERO;
        for (slot, f) in around(i / v, i % v).into_iter().enumerate() {
            if let Some(f) = f {
                if f == own || face[f].normalize_or_zero().dot(nf) > CREASE {
                    set |= 1 << slot;
                    sum += face[f];
                }
            }
        }
        for &(s, idx, moss) in &made[i] {
            if idx != u32::MAX && s == set {
                return (idx, moss);
            }
        }
        let p = pts[i];
        let n = sum.normalize_or(nf);
        // What turns from the sky — a riser, the roof of an overhang — is
        // lit by less of it.
        let value = p.value * (1.0 - p.fade * SKY_DARK * (0.5 - n.y).max(0.0));
        // Turf on what faces up, in patches.
        let moss = if green(p.p.y) {
            let up = ((n.y - SHELF_UP) / 0.2).clamp(0.0, 1.0);
            let patch = (facet_noise(p.p / 3.0, key ^ 0x6a55) * 2.0 - 0.3).clamp(0.0, 1.0);
            up * up * (3.0 - 2.0 * up) * patch * p.fade
        } else {
            0.0
        };
        let idx = soup.vertex(p.p, n, moss, value);
        if let Some(free) = made[i].iter_mut().find(|e| e.1 == u32::MAX) {
            *free = (set, idx, moss);
        }
        (idx, moss)
    };
    // The cell's own quads: those whose low corner is inside it.
    for j in 1..q - 1 {
        for k in 1..q - 1 {
            for (t, c) in tris(j, k).into_iter().enumerate() {
                if c.iter().all(|&i| pts[i].out <= 0.0) {
                    continue;
                }
                let own = (j * q + k) * 2 + t;
                let [a, b, cc] = c.map(|i| corner(soup, i, own));
                soup.face(a.0, b.0, cc.0);
                // Grass on a shelf the turf has taken, at a point of the
                // triangle the hash picks.
                let moss = (a.1 + b.1 + cc.1) / 3.0;
                let h = |n: u32| h01(key ^ 0x7f1e, own as u32 * 8 + n);
                if moss > TUFT_MOSS && h(0) < TUFT_SHARE * moss {
                    let (mut r1, mut r2) = (h(1), h(2));
                    if r1 + r2 > 1.0 {
                        (r1, r2) = (1.0 - r1, 1.0 - r2);
                    }
                    let (p0, p1, p2) = (pts[c[0]].p, pts[c[1]].p, pts[c[2]].p);
                    let at = p0 + (p1 - p0) * r1 + (p2 - p0) * r2;
                    let seed = (h(3) * 16_777_216.0) as u32;
                    tuft(
                        tufts,
                        at,
                        h(4) * std::f32::consts::TAU,
                        seed,
                        0.8 + 0.6 * h(5),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skin stands out only over ground the sim refuses a body: every
    /// pushed point, and the ground half a metre round it, is too steep to
    /// walk. On seed 20260731's cliffs north of 1500,600 and 948,1185, 75 m
    /// and 65 m of face.
    #[test]
    fn the_skin_never_stands_over_walkable_ground() {
        let seed = 20260731;
        let haven = sim_core::world::World::new(seed).haven;
        let mut pushed = 0;
        let cells = (19..23)
            .flat_map(|cz| (45..48).map(move |cx| (cx, cz)))
            .chain((37..41).flat_map(|cz| (28..31).map(move |cx| (cx, cz))));
        for (cx, cz) in cells {
            {
                let Some(shell) = shell(seed, &haven, 32.0, cx, cz) else {
                    continue;
                };
                for SkinPoint { p, out, .. } in shell.pts {
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
