//! The rock skin: a faceted shell drawn over every face too steep to climb.
//!
//! Blocks stood on the smooth heightfield read as bricks on plaster: the face
//! between them was still one soft sheet. The reference game's cliffs are
//! rock meshes over the whole face — broken planes, shelves, risers. This is
//! that, procedurally, on a half-metre lattice: what is recessed is darker,
//! each large facet is its own shade, and the shelves grow turf.
//!
//! **The sim's shape, drawn.** Every height here is `sim_core::face`'s
//! (`face::skin_point`), the same the sim stands a body on and stops an
//! arrow with, so nothing passes through drawn rock (operator, 2026-10-17:
//! "why cant things be real?"). Where the skin has faded out the shell
//! sinks [`face::SKIN_SINK_M`] under the drawn ground and is hidden by it.

use bevy::prelude::*;
use sim_core::cliff::Crag;
use sim_core::face::{self, Coarse};
use sim_core::terrain::{self, Haven};

use super::boulders::RockSoup;
use super::clutter::tuft;
use super::props::Soup;
use super::terrain_mesh::vertex_mods;

pub use sim_core::face::CRAG_FADE_M;

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

/// [`face::facet_noise`] at a point.
fn facet(p: Vec3, key: u32) -> f32 {
    face::facet_noise(p.x, p.y, p.z, key)
}

/// One point of the skin's lattice, as drawn.
#[derive(Clone, Copy, Debug)]
pub struct SkinPoint {
    /// Where it is drawn.
    pub p: Vec3,
    /// How far it stands out, as a share of [`face::SKIN_OUT_M`].
    pub out: f32,
    /// How fully the skin has grown here, 0..1: none where it lies under
    /// the ground.
    pub fade: f32,
    /// The ground material's value multiplier here.
    pub value: f32,
}

/// The skin's lattice over one cell, [`face::SKIN_STEP_M`] apart and one
/// point past the cell on every side, so a vertex on the cell's edge is
/// shaded from the triangles of both cells alike.
pub struct Shell {
    /// Points a side.
    pub side: usize,
    /// Row by row, from the cell's low corner less one step.
    pub pts: Vec<SkinPoint>,
}

/// The rock skin over one cliff cell's steep faces, into `soup`, and the
/// grass on its shelves into `tufts`. It falls back round every crag in
/// `crags` (the sim's, `sim_core::cliff`), so a crag stands out of it.
#[allow(clippy::too_many_arguments)]
pub(super) fn skin(
    soup: &mut RockSoup,
    tufts: &mut Soup,
    seed: u64,
    haven: &Haven,
    crags: &[Crag],
    cell_m: f32,
    cx: i32,
    cz: i32,
) {
    if let Some(shell) = shell(seed, haven, crags, cell_m, cx, cz) {
        mesh(soup, tufts, &shell, face::seed_key(seed));
    }
}

/// Whether a cell has ground anywhere near steep enough to carry the skin:
/// nine taps, so a flat cell costs nothing more.
pub(super) fn steep_cell(
    lat: &mut terrain::Lattice,
    seed: u64,
    haven: &Haven,
    cell_m: f32,
    cx: i32,
    cz: i32,
) -> bool {
    let (x0, z0) = (cx as f32 * cell_m, cz as f32 * cell_m);
    (0..9).any(|i| {
        let (x, z) = (
            x0 + cell_m * (0.17 + 0.33 * (i % 3) as f32),
            z0 + cell_m * (0.17 + 0.33 * (i / 3) as f32),
        );
        terrain::ground_slope_memo(lat, seed, haven, x, z) > terrain::CLIFF_SLOPE_RATIO * 0.8
    })
}

/// The skin's lattice over one cell; `None` where nothing in it stands out.
pub fn shell(
    seed: u64,
    haven: &Haven,
    crags: &[Crag],
    cell_m: f32,
    cx: i32,
    cz: i32,
) -> Option<Shell> {
    // The 1 m lattice the near ground is drawn on, over the cell and a point
    // past it, plus a border for the 5×5 stencil each coarse point reads.
    let n = cell_m as i32;
    let m = 3;
    let side = (n + 1 + 2 * m) as usize;
    let (x0, z0) = (cx as f32 * cell_m, cz as f32 * cell_m);
    let gx = |k: usize| x0 + (k as i32 - m) as f32;
    let gz = |j: usize| z0 + (j as i32 - m) as f32;
    let mut lat = terrain::Lattice::new();
    if !steep_cell(&mut lat, seed, haven, cell_m, cx, cz) {
        return None;
    }
    let mut h = vec![0.0f32; side * side];
    for j in 0..side {
        for k in 0..side {
            h[j * side + k] = terrain::ground_memo(&mut lat, seed, haven, gx(k), gz(j));
        }
    }
    // Per 1 m point from one before the cell to one past it.
    let nc = (n + 3) as usize;
    let mut coarse = vec![Coarse::default(); nc * nc];
    for cj in 0..nc {
        for ck in 0..nc {
            let (j, k) = (cj + m as usize - 1, ck + m as usize - 1);
            let mut stencil = [[0.0f32; 5]; 5];
            for (dj, row) in stencil.iter_mut().enumerate() {
                for (dk, s) in row.iter_mut().enumerate() {
                    *s = h[(j + dj - 2) * side + k + dk - 2];
                }
            }
            coarse[cj * nc + ck] = face::coarse(seed, haven, gx(k), gz(j), &stencil);
        }
    }
    let key = face::seed_key(seed);
    let per = (1.0 / face::SKIN_STEP_M).round() as i32;
    let nf = (n * per + 3) as usize;
    let mut pts = Vec::with_capacity(nf * nf);
    let mut any = false;
    for fj in 0..nf {
        for fk in 0..nf {
            // Metres from the cell's low corner, then the 1 m quad holding
            // the point.
            let (u, w) = (
                (fk as i32 - 1) as f32 * face::SKIN_STEP_M,
                (fj as i32 - 1) as f32 * face::SKIN_STEP_M,
            );
            let (ck, cj) = (
                (u.floor() as i32 + 1) as usize,
                (w.floor() as i32 + 1) as usize,
            );
            let c = |dj: usize, dk: usize| coarse[(cj + dj) * nc + ck + dk];
            let q = [c(0, 0), c(0, 1), c(1, 0), c(1, 1)];
            let (x, z) = (x0 + u, z0 + w);
            let room = if q.iter().any(|q| q.fade > 0.0) {
                face::crag_room(crags, x, z)
            } else {
                0.0
            };
            let sp = face::skin_point(key, x, z, &q, room);
            let base = vertex_mods(sp.drawn, x, z, 0.0)[0];
            any |= sp.out > 0.0;
            // What stands out is lit, what is recessed is darker — the
            // cheap half of occlusion — and each facet is its own shade;
            // both grow in with the skin, so where it meets the ground the
            // two are one value.
            let shade = VALUE_RELIEF * sp.local.clamp(-1.0, 1.0)
                + FACET_SHADE
                    * (facet(Vec3::new(x, sp.drawn, z) / face::FACET_M, key ^ 0x7a1e) * 2.0 - 1.0);
            pts.push(SkinPoint {
                p: Vec3::new(x, sp.y, z),
                out: sp.out,
                fade: sp.fade,
                value: base * (1.0 + shade * sp.fade),
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
            let patch = (facet(p.p / 3.0, key ^ 0x6a55) * 2.0 - 0.3).clamp(0.0, 1.0);
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

    /// The skin drawn is the sim's: every lattice point stands exactly where
    /// `face::surface` puts the rock a body stands on, wherever it shows. On
    /// seed 20260731's cliffs north of 1500,600 and 948,1185.
    #[test]
    fn the_skin_drawn_is_the_rock_the_sim_stands_on() {
        let seed = 20260731;
        let haven = sim_core::world::World::new(seed).haven;
        let mut sc = sim_core::occupy::Scratch::live(seed);
        let mut lat = terrain::Lattice::new();
        let mut shown = 0;
        let cells = (19..23)
            .flat_map(|cz| (45..48).map(move |cx| (cx, cz)))
            .chain((37..41).flat_map(|cz| (28..31).map(move |cx| (cx, cz))));
        for (cx, cz) in cells {
            let n = (32.0 / sim_core::cliff::SPOT_M) as i32;
            let r = ((sim_core::cliff::CRAG_REACH_M + CRAG_FADE_M) / sim_core::cliff::SPOT_M).ceil()
                as i32;
            let mut crags = Vec::new();
            for j in cz * n - r..(cz + 1) * n + r {
                for i in cx * n - r..(cx + 1) * n + r {
                    let c = sim_core::cliff::crag(&mut lat, seed, &haven, i, j);
                    if c.is_some() {
                        crags.push(c);
                    }
                }
            }
            let Some(shell) = shell(seed, &haven, &crags, 32.0, cx, cz) else {
                continue;
            };
            for SkinPoint { p, out, .. } in shell.pts {
                if out <= 0.0 {
                    continue;
                }
                let s = sim_core::face::surface(&mut sc.occupants(), seed, &haven, p.x, p.z);
                if s - p.y > 0.01 {
                    // Under the near ground: the ground shows there.
                    let near = sim_core::face::near_drawn_y(&mut lat, seed, &haven, p.x, p.z);
                    assert!(
                        (s - near).abs() < 0.01,
                        "at {p}: the sim's rock {s:.3}, drawn {:.3}",
                        p.y.max(near)
                    );
                    continue;
                }
                shown += 1;
                assert!(
                    (s - p.y).abs() < 0.01,
                    "skin point {p} (out {out:.2}): the sim stands a body at {s:.3}"
                );
            }
        }
        assert!(
            shown > 1000,
            "only {shown} skin points stand out: no cliff here"
        );
    }
}
