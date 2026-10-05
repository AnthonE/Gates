//! What a piece is BUILT of, per tier (piece dress v0): the members, panels,
//! poles and blocks a wall's or a slab's volume is drawn as.
//!
//! [`super::shape_parts`] says where a piece's solid is, and that is collision
//! truth. This fills the same boxes with something that reads as built: a
//! lashed pole frame over a stick mat for twig, plank infill in a heavy timber
//! frame for wood, coursed blocks with cut joints for stone, corrugated panels
//! on a steel frame for metal. All of it is relief INSIDE the volume — frame
//! members flush with the faces, infill recessed a few centimetres, joints cut
//! in — and `tests/pieces.rs` §E holds every vertex inside the part it dresses,
//! so nothing here moves a face a player can touch or an arrow can hit.
//!
//! One mesh per shape × tier (× post ownership × soft side, × skirt step),
//! shared by every address, so a base of hundreds of pieces is still a few
//! hundred meshes. UVs stay metres (scaled per member where a beam has to show
//! one board rather than two), and every face carries the same mean-1
//! luminance tint `box_into` lays down, times a per-member value.

use std::f32::consts::PI;

use bevy::prelude::*;
use sim_core::build::{BUILD_CELL_M, LEVEL_H_M, MAT_STONE, MAT_TWIG, MAT_WOOD};
use sim_core::collide::WALL_THICKNESS_M;

use super::{buffers, face_tint, finish, post_w, Buffers, Part, PartRole, EDGE_DROP_M};

/// Half a wall's thickness: where an edge piece's two faces are.
const T2: f32 = WALL_THICKNESS_M * 0.5;
/// Half a cell: where a slab's sides are.
const H: f32 = BUILD_CELL_M * 0.5;

/// Stone course height, metres, counted from the storey base so two walls
/// side by side run their courses level.
pub const COURSE_M: f32 = 0.5;
/// Stone block length along a wall, metres; alternate courses are offset by
/// half of it (running bond).
const BLOCK_M: f32 = 0.9;
/// A stone joint's chamfer: how far in from the block's edge it starts and
/// how deep it cuts, metres.
pub const JOINT_M: f32 = 0.025;

/// How a member wears its tier's photograph: the grain direction (the map's
/// `v`), texture metres per metre across and along it, an offset so two
/// members do not show the same patch, and a scalar albedo tint.
#[derive(Clone, Copy, Debug)]
struct Look {
    g: Vec3,
    su: f32,
    sv: f32,
    off: Vec2,
    tint: f32,
    /// Lay `v` around a prism's perimeter rather than along it — a steel
    /// section reading the corrugated map's ribs as its own flanges.
    around: bool,
}

impl Look {
    fn new(g: Vec3, tint: f32) -> Self {
        Self {
            g,
            su: 1.0,
            sv: 1.0,
            off: Vec2::ZERO,
            tint,
            around: false,
        }
    }

    fn across(mut self, su: f32) -> Self {
        self.su = su;
        self
    }

    fn grain(mut self, g: Vec3) -> Self {
        self.g = g;
        self
    }

    /// A deterministic patch of the map and a ±`spread` tint wobble, so a
    /// row of blocks or panels is a row of different things.
    fn vary(mut self, seed: u32, spread: f32) -> Self {
        self.off = Vec2::new(rand01(seed, 1) * 4.0, rand01(seed, 2) * 4.0);
        self.tint *= 1.0 + (rand01(seed, 3) * 2.0 - 1.0) * spread;
        self
    }

    /// UV for a point on a flat face with normal `n`: `v` along the grain,
    /// `u` across it on the face; a face looking down the grain lays the map
    /// flat on its two other axes.
    fn uv(&self, p: Vec3, n: Vec3) -> Vec2 {
        let w = self.g.cross(n);
        let (u, v) = if w.length_squared() > 0.09 {
            (p.dot(w.normalize()), p.dot(self.g))
        } else {
            let a = if self.g.x.abs() > 0.9 {
                Vec3::Z
            } else {
                Vec3::X
            };
            let b = self.g.cross(a).normalize();
            (p.dot(a), p.dot(b))
        };
        Vec2::new(u * self.su, v * self.sv) + self.off
    }
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B).rotate_left(13);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

fn rand01(a: u32, b: u32) -> f32 {
    (hash(a, b) >> 8) as f32 / (1u32 << 24) as f32
}

/// Newell's normal: the polygon's area vector, right-handed in its order.
fn newell(pts: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for i in 0..pts.len() {
        let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
        n.x += (p.y - q.y) * (p.z + q.z);
        n.y += (p.z - q.z) * (p.x + q.x);
        n.z += (p.x - q.x) * (p.y + q.y);
    }
    n
}

/// The buffers being filled, and the vertical span the face tint's
/// foot-to-head ramp runs over (the whole piece, not each member, so a
/// frame reads as one object in one light).
struct Out {
    b: Buffers,
    y0: f32,
    y1: f32,
}

impl Out {
    fn new(y0: f32, y1: f32) -> Self {
        Self {
            b: buffers(1024),
            y0,
            y1,
        }
    }

    /// One convex face. The winding is fixed here against `out`, the side the
    /// face looks toward, so no caller can hand over a face culled inside out.
    fn face(&mut self, pts: &[Vec3], out: Vec3, look: &Look) {
        self.face_with(pts, out, look, None, None);
    }

    fn face_with(
        &mut self,
        pts: &[Vec3],
        out: Vec3,
        look: &Look,
        uvs: Option<&[Vec2]>,
        normals: Option<&[Vec3]>,
    ) {
        let raw = newell(pts);
        if raw.length_squared() < 1e-12 {
            return;
        }
        let flip = raw.dot(out) < 0.0;
        let n = if flip { -raw } else { raw }.normalize();
        let k = pts.len();
        let (pos, nor, col, uv, idx) = &mut self.b;
        let base = pos.len() as u32;
        for i in 0..k {
            let j = if flip { k - 1 - i } else { i };
            let p = pts[j];
            let vn = normals.map_or(n, |ns| ns[j]);
            let t = face_tint(vn, p.y, self.y0, self.y1) * look.tint;
            pos.push(p.to_array());
            nor.push(vn.to_array());
            col.push([t, t, t, 1.0]);
            uv.push(uvs.map_or_else(|| look.uv(p, n), |u| u[j]).to_array());
        }
        for i in 1..k as u32 - 1 {
            idx.extend([base, base + i, base + i + 1]);
        }
    }

    /// A prism: the convex `poly` in the `(a, b)` plane through `o`, swept
    /// along `a × b` from `s[0]` to `s[1]`. Sides wrap the map around the
    /// perimeter — along the sweep when the look's grain is the sweep (a
    /// beam, a pole), around it otherwise (a slab's rim). `smooth` gives the
    /// sides radial normals, which is what makes an octagon read as a pole.
    #[allow(clippy::too_many_arguments)]
    fn extrude(
        &mut self,
        o: Vec3,
        a: Vec3,
        b: Vec3,
        poly: &[Vec2],
        s: [f32; 2],
        look: &Look,
        caps: [bool; 2],
        smooth: bool,
    ) {
        let g = a.cross(b);
        let at = |p: Vec2, s: f32| o + a * p.x + b * p.y + g * s;
        let c = poly.iter().copied().sum::<Vec2>() / poly.len() as f32;
        let along = look.g.dot(g).abs() > 0.9 && !look.around;
        let uv = |arc: f32, s: f32| {
            look.off
                + if along {
                    Vec2::new(arc * look.su, s * look.sv)
                } else {
                    Vec2::new(s * look.su, arc * look.sv)
                }
        };
        let radial = |p: Vec2| (a * (p.x - c.x) + b * (p.y - c.y)).normalize();
        let mut arc = 0.0;
        for i in 0..poly.len() {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            let len = p.distance(q);
            let pts = [at(p, s[0]), at(q, s[0]), at(q, s[1]), at(p, s[1])];
            let mid = (p + q) * 0.5 - c;
            let out = a * mid.x + b * mid.y;
            let uvs = [
                uv(arc, s[0]),
                uv(arc + len, s[0]),
                uv(arc + len, s[1]),
                uv(arc, s[1]),
            ];
            if smooth {
                let ns = [radial(p), radial(q), radial(q), radial(p)];
                self.face_with(&pts, out, look, Some(&uvs), Some(&ns));
            } else {
                self.face_with(&pts, out, look, Some(&uvs), None);
            }
            arc += len;
        }
        for (end, cap) in caps.iter().enumerate() {
            if *cap {
                let pts: Vec<Vec3> = poly.iter().map(|&p| at(p, s[end])).collect();
                self.face(&pts, if end == 0 { -g } else { g }, look);
            }
        }
    }

    /// An axis-aligned box; `skip` drops faces by bit: +x, −x, +y, −y, +z, −z.
    fn cuboid(&mut self, lo: Vec3, hi: Vec3, look: &Look, skip: u8) {
        let c = [lo, hi];
        let corner = |i: usize| Vec3::new(c[i & 1].x, c[(i >> 1) & 1].y, c[(i >> 2) & 1].z);
        let faces: [(Vec3, [usize; 4]); 6] = [
            (Vec3::X, [1, 3, 7, 5]),
            (Vec3::NEG_X, [0, 4, 6, 2]),
            (Vec3::Y, [2, 6, 7, 3]),
            (Vec3::NEG_Y, [0, 1, 5, 4]),
            (Vec3::Z, [4, 5, 7, 6]),
            (Vec3::NEG_Z, [0, 2, 3, 1]),
        ];
        for (f, (n, ids)) in faces.iter().enumerate() {
            if skip & (1 << f) == 0 {
                self.face(&ids.map(corner), *n, look);
            }
        }
    }

    /// A box with its four edges along `axis` chamfered by `ch` — a sawn
    /// timber or a steel section, grain along its length.
    fn beam(&mut self, lo: Vec3, hi: Vec3, axis: usize, ch: f32, look: &Look) {
        let (a, b) = match axis {
            0 => (Vec3::Y, Vec3::Z),
            1 => (Vec3::Z, Vec3::X),
            _ => (Vec3::X, Vec3::Y),
        };
        let g = a.cross(b);
        let c = (lo + hi) * 0.5;
        let h = (hi - lo) * 0.5;
        let (ha, hb, hg) = (h.dot(a), h.dot(b), h.dot(g));
        let ch = ch.min(ha * 0.45).min(hb * 0.45);
        let poly = chamfer_rect(ha, hb, ch);
        self.extrude(
            c,
            a,
            b,
            &poly,
            [-hg, hg],
            &look.grain(g),
            [true, true],
            false,
        );
    }

    /// A pole: an octagon of apothem `r` from `p0` to `p1`, one flat facing
    /// `side`, smooth-shaded, grain along it.
    fn pole(&mut self, p0: Vec3, p1: Vec3, r: f32, side: Vec3, look: &Look) {
        let g = (p1 - p0).normalize();
        let a = (side - g * side.dot(g)).normalize();
        let b = g.cross(a);
        let rr = r / (PI / 8.0).cos();
        let poly: [Vec2; 8] = std::array::from_fn(|k| {
            let t = PI / 8.0 + k as f32 * PI / 4.0;
            Vec2::new(t.cos(), t.sin()) * rr
        });
        self.extrude(
            p0,
            a,
            b,
            &poly,
            [0.0, (p1 - p0).length()],
            &look.grain(g),
            [true, true],
            true,
        );
    }

    /// A box with all twelve edges chamfered by `c` — one dressed stone.
    fn block(&mut self, lo: Vec3, hi: Vec3, c: f32, look: &Look) {
        let m = (lo + hi) * 0.5;
        let h = (hi - lo) * 0.5;
        let c = c.min(h.min_element() * 0.45);
        let i = h - Vec3::splat(c);
        let e = [Vec3::X, Vec3::Y, Vec3::Z];
        for k in 0..3 {
            let (u, v) = ((k + 1) % 3, (k + 2) % 3);
            for s in [-1.0f32, 1.0] {
                let f = m + e[k] * s * h[k];
                let pts = [
                    f - e[u] * i[u] - e[v] * i[v],
                    f + e[u] * i[u] - e[v] * i[v],
                    f + e[u] * i[u] + e[v] * i[v],
                    f - e[u] * i[u] + e[v] * i[v],
                ];
                self.face(&pts, e[k] * s, look);
                // The bevel along the edge between this face and its `+u`/`-u`
                // neighbour runs along `v`.
                for su in [-1.0f32, 1.0] {
                    let pts = [
                        m + e[k] * s * h[k] + e[u] * su * i[u] - e[v] * i[v],
                        m + e[k] * s * h[k] + e[u] * su * i[u] + e[v] * i[v],
                        m + e[k] * s * i[k] + e[u] * su * h[u] + e[v] * i[v],
                        m + e[k] * s * i[k] + e[u] * su * h[u] - e[v] * i[v],
                    ];
                    self.face(&pts, e[k] * s + e[u] * su, look);
                }
            }
        }
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let s = Vec3::new(sx, sy, sz);
                    let pts = [
                        m + s * Vec3::new(h.x, i.y, i.z),
                        m + s * Vec3::new(i.x, h.y, i.z),
                        m + s * Vec3::new(i.x, i.y, h.z),
                    ];
                    self.face(&pts, s, look);
                }
            }
        }
    }

    /// One block's face on a plane (`o` its point at `u = v = 0`, `n`
    /// outward): a front over `rect` = `[u0, u1, v0, v1]`, and on each edge
    /// `cut` names a chamfer `c` wide falling `d` deep to the rect's edge —
    /// where two blocks meet, their chamfers make the joint. An uncut edge
    /// keeps the front to the rect's edge, and the half-groove a cut edge
    /// opens there is capped.
    #[allow(clippy::too_many_arguments)]
    fn pillow(
        &mut self,
        o: Vec3,
        n: Vec3,
        (u, v): (Vec3, Vec3),
        r: [f32; 4],
        cut: [bool; 4],
        c: f32,
        d: f32,
        look: &Look,
    ) {
        let k = |i: usize| if cut[i] { c } else { 0.0 };
        let f = [r[0] + k(0), r[1] - k(1), r[2] + k(2), r[3] - k(3)];
        let at = |uu: f32, vv: f32, depth: f32| o + u * uu + v * vv - n * depth;
        self.face(
            &[
                at(f[0], f[2], 0.0),
                at(f[1], f[2], 0.0),
                at(f[1], f[3], 0.0),
                at(f[0], f[3], 0.0),
            ],
            n,
            look,
        );
        // The u-edges (u0, u1) run along v; the v-edges along u.
        for (e, fu, ru, dir) in [(0, f[0], r[0], -u), (1, f[1], r[1], u)] {
            if !cut[e] {
                continue;
            }
            self.face(
                &[
                    at(fu, f[2], 0.0),
                    at(fu, f[3], 0.0),
                    at(ru, r[3], d),
                    at(ru, r[2], d),
                ],
                dir + n,
                look,
            );
            for (ve, rv, vdir) in [(2, r[2], -v), (3, r[3], v)] {
                if !cut[ve] {
                    self.face(
                        &[at(fu, rv, 0.0), at(ru, rv, d), at(ru, rv, 0.0)],
                        vdir,
                        look,
                    );
                }
            }
        }
        for (e, fv, rv, dir) in [(2, f[2], r[2], -v), (3, f[3], r[3], v)] {
            if !cut[e] {
                continue;
            }
            self.face(
                &[
                    at(f[0], fv, 0.0),
                    at(f[1], fv, 0.0),
                    at(r[1], rv, d),
                    at(r[0], rv, d),
                ],
                dir + n,
                look,
            );
            for (ue, ru, udir) in [(0, r[0], -u), (1, r[1], u)] {
                if !cut[ue] {
                    self.face(
                        &[at(ru, fv, 0.0), at(ru, rv, d), at(ru, rv, 0.0)],
                        udir,
                        look,
                    );
                }
            }
        }
    }

    /// A slab across an edge piece: `poly` in its `(z, y)` face, swept over
    /// `x0..x1` — a brace, a strap, a panel.
    fn sheet_x(&mut self, poly_zy: &[Vec2], x0: f32, x1: f32, look: &Look) {
        let poly: Vec<Vec2> = poly_zy.iter().map(|p| Vec2::new(p.y, p.x)).collect();
        self.extrude(
            Vec3::ZERO,
            Vec3::Y,
            Vec3::Z,
            &poly,
            [x0.min(x1), x0.max(x1)],
            look,
            [true, true],
            false,
        );
    }

    /// A prism standing up: `poly` in `(x, z)`, swept over `y0..y1`.
    fn prism_y(&mut self, poly_xz: &[Vec2], y0: f32, y1: f32, look: &Look, caps: [bool; 2]) {
        let poly: Vec<Vec2> = poly_xz.iter().map(|p| Vec2::new(p.y, p.x)).collect();
        self.extrude(
            Vec3::ZERO,
            Vec3::Z,
            Vec3::X,
            &poly,
            [y0, y1],
            look,
            caps,
            false,
        );
    }
}

/// A rectangle of half-extents `ha × hb` with its corners cut by `c`.
fn chamfer_rect(ha: f32, hb: f32, c: f32) -> Vec<Vec2> {
    if c <= 1e-4 {
        return vec![
            Vec2::new(ha, -hb),
            Vec2::new(ha, hb),
            Vec2::new(-ha, hb),
            Vec2::new(-ha, -hb),
        ];
    }
    vec![
        Vec2::new(ha, -hb + c),
        Vec2::new(ha, hb - c),
        Vec2::new(ha - c, hb),
        Vec2::new(-ha + c, hb),
        Vec2::new(-ha, hb - c),
        Vec2::new(-ha, -hb + c),
        Vec2::new(-ha + c, -hb),
        Vec2::new(ha - c, -hb),
    ]
}

/// A convex polygon pulled in by `r` on every edge.
fn inset(poly: &[Vec2], r: f32) -> Vec<Vec2> {
    let n = poly.len();
    let area: f32 = (0..n)
        .map(|i| poly[i].perp_dot(poly[(i + 1) % n]))
        .sum::<f32>();
    let ccw = if area > 0.0 { 1.0 } else { -1.0 };
    // Each edge's line, moved inward: a point on it and its direction.
    let line = |i: usize| {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        let d = (q - p).normalize();
        let inward = Vec2::new(-d.y, d.x) * ccw;
        (p + inward * r, d)
    };
    (0..n)
        .map(|i| {
            let (p0, d0) = line((i + n - 1) % n);
            let (p1, d1) = line(i);
            let t = (p1 - p0).perp_dot(d1) / d0.perp_dot(d1);
            p0 + d0 * t
        })
        .collect()
}

/// Lines dividing `y0..y1` into stone courses on the storey's `step` grid,
/// with no sliver under 12 cm.
fn course_lines(y0: f32, y1: f32, step: f32, from: f32) -> Vec<f32> {
    let mut v = vec![y0];
    let mut k = ((y0 - from) / step).floor() + 1.0;
    while from + k * step < y1 - 0.12 {
        let y = from + k * step;
        if y > y0 + 0.12 {
            v.push(y);
        }
        k += 1.0;
    }
    v.push(y1);
    v
}

/// Where the vertical joints of one course fall in `lo..hi`: every `len`
/// from `origin`, shifted half a block on odd courses, no stub under 20 cm.
fn joint_lines(lo: f32, hi: f32, len: f32, origin: f32, course: i32) -> Vec<f32> {
    let phase = if course.rem_euclid(2) == 0 {
        0.0
    } else {
        len * 0.5
    };
    let mut v = vec![lo];
    let mut z = origin + phase;
    while z < hi - 0.2 {
        if z > lo + 0.2 {
            v.push(z);
        }
        z += len;
    }
    v.push(hi);
    v
}

/// A brace across a panel `z0..z1 × y0..y1`, `w` wide, cut level where it
/// meets the members above and below: its outline in `(z, y)` and its grain.
fn brace(z0: f32, z1: f32, y0: f32, y1: f32, w: f32, rising: bool) -> Option<([Vec2; 4], Vec3)> {
    let (dz, dy) = (z1 - z0, y1 - y0);
    if dy < 0.8 || dz < 0.8 {
        return None;
    }
    let mut wz = w;
    for _ in 0..4 {
        let run = dz - wz;
        wz = w * (run * run + dy * dy).sqrt() / dy;
    }
    if wz > dz * 0.4 {
        return None;
    }
    let (a, b) = if rising { (z0, z1) } else { (z1, z0) };
    let s = if rising { 1.0 } else { -1.0 };
    let poly = [
        Vec2::new(a, y0),
        Vec2::new(a + s * wz, y0),
        Vec2::new(b, y1),
        Vec2::new(b - s * wz, y1),
    ];
    let g = Vec3::new(0.0, dy, s * (dz - wz)).normalize();
    Some((poly, g))
}

fn bounds(p: &Part) -> (Vec3, Vec3) {
    (p.offset - p.size * 0.5, p.offset + p.size * 0.5)
}

/// What a body part's four edges meet, read off the other parts of its
/// shape: `lo_y`/`hi_y`/`lo_z`/`hi_z` are true where the edge is FREE (a
/// storey boundary or an opening) and so carries a frame member, false where
/// another body or a corner post covers it.
#[derive(Clone, Copy, Debug, Default)]
struct Edges {
    lo_y: bool,
    hi_y: bool,
    lo_z: bool,
    hi_z: bool,
    /// The foot is on the storey base / the head at the storey top.
    base: bool,
    head: bool,
}

fn edges_of(me: usize, parts: &[Part], diagonal: bool) -> Edges {
    const E: f32 = 1e-3;
    let (lo, hi) = bounds(&parts[me]);
    let (mut below, mut above, mut left, mut right) = (false, false, false, false);
    for (i, q) in parts.iter().enumerate() {
        if i == me || q.role != PartRole::Body {
            continue;
        }
        let (qlo, qhi) = bounds(q);
        let spans_z = qlo.z <= lo.z + E && qhi.z >= hi.z - E;
        let spans_y = qlo.y <= lo.y + E && qhi.y >= hi.y - E;
        below |= (qhi.y - lo.y).abs() < E && spans_z;
        above |= (qlo.y - hi.y).abs() < E && spans_z;
        left |= (qhi.z - lo.z).abs() < E && spans_y;
        right |= (qlo.z - hi.z).abs() < E && spans_y;
    }
    let post_face = (BUILD_CELL_M - post_w()) * 0.5;
    let at_post = |z: f32| !diagonal && (z.abs() - post_face).abs() < E;
    Edges {
        lo_y: !below,
        hi_y: !above,
        lo_z: !left && !at_post(lo.z),
        hi_z: !right && !at_post(hi.z),
        base: lo.y <= 0.0,
        head: hi.y >= LEVEL_H_M - EDGE_DROP_M - E,
    }
}

/// The free edges' frame members and what they leave for infill.
struct Frame {
    yb: f32,
    yt: f32,
    zb: f32,
    zt: f32,
    /// A mid-storey girt across tall infill, as its `y0..y1`.
    girt: Option<(f32, f32)>,
}

impl Frame {
    fn of(lo: Vec3, hi: Vec3, e: Edges, rail: f32, stile: f32, girt: f32) -> Self {
        let yb = lo.y + if e.lo_y { rail } else { 0.0 };
        let yt = hi.y - if e.hi_y { rail } else { 0.0 };
        let zb = lo.z + if e.lo_z { stile } else { 0.0 };
        let zt = hi.z - if e.hi_z { stile } else { 0.0 };
        let girt = (yt - yb > 1.8 && zt - zb > 0.4).then(|| {
            let c = (LEVEL_H_M * 0.5).clamp(yb + 0.7, yt - 0.7);
            (c - girt * 0.5, c + girt * 0.5)
        });
        Self {
            yb,
            yt,
            zb,
            zt,
            girt,
        }
    }

    /// The infill rows between the rails and the girt.
    fn rows(&self) -> Vec<(f32, f32)> {
        match self.girt {
            Some((g0, g1)) => vec![(self.yb, g0), (g1, self.yt)],
            None => vec![(self.yb, self.yt)],
        }
    }
}

const SKIP_PX: u8 = 1;
const SKIP_NX: u8 = 2;
const SKIP_PY: u8 = 4;
const SKIP_NY: u8 = 8;
const SKIP_PZ: u8 = 16;
const SKIP_NZ: u8 = 32;
/// Everything but the two big faces of an edge piece.
const ONLY_X: u8 = SKIP_PY | SKIP_NY | SKIP_PZ | SKIP_NZ;

// ---------------------------------------------------------------------------
// The four tiers' looks. Tints are scalar (`ART.md` §7) and sit around 1.0:
// a frame darker than its infill, the way an oiled timber is darker than a
// weathered board and a painted section darker than galvanised sheet.
// ---------------------------------------------------------------------------

fn twig_mat() -> Look {
    Look::new(Vec3::Y, 0.84)
}
fn twig_pole(seed: u32) -> Look {
    Look::new(Vec3::Z, 1.10).vary(seed, 0.06)
}
fn planks() -> Look {
    Look::new(Vec3::Y, 1.06)
}
/// A timber shows one board's grain, not two: 0.45 across puts a 20 cm beam
/// over ~9 cm of the plank map, about one board.
fn timber(g: Vec3) -> Look {
    Look::new(g, 0.78).across(0.45)
}
/// The ashlar map is pale limestone (luma 0.35); 0.72 brings it to a light
/// stone a step above twig and wood rather than a white one.
fn stone(seed: u32) -> Look {
    Look::new(Vec3::Y, 0.72).vary(seed, 0.08)
}
fn sheet(g: Vec3, seed: u32) -> Look {
    Look::new(g, 1.04).vary(seed, 0.05)
}
/// A steel section wraps the corrugated map's ribs around its perimeter, a
/// rib or so per face, so it reads as rolled flanges running its length.
fn steel(g: Vec3) -> Look {
    let mut l = Look::new(g, 0.52);
    l.around = true;
    l.sv = 0.35;
    l
}

// ---------------------------------------------------------------------------
// Edge pieces
// ---------------------------------------------------------------------------

/// An edge piece's parts, dressed in `tier` (unsided — the kit tints the soft
/// face per orientation, see [`retint_sides`]). `diagonal` marks the
/// diagonal wall's one body, which ends at corners no post of its own covers.
pub fn edge_mesh(parts: &[Part], tier: u8, diagonal: bool) -> Mesh {
    let top = parts
        .iter()
        .map(|p| p.offset.y + p.size.y * 0.5)
        .fold(f32::MIN, f32::max);
    let mut o = Out::new(-EDGE_DROP_M, top);
    for (i, p) in parts.iter().enumerate() {
        let (lo, hi) = bounds(p);
        let salt = (tier as u32) << 8 | i as u32;
        if p.role == PartRole::Body {
            let e = edges_of(i, parts, diagonal);
            match tier {
                MAT_TWIG => twig_body(&mut o, lo, hi, e, salt),
                MAT_WOOD => wood_body(&mut o, lo, hi, e),
                MAT_STONE => stone_body(&mut o, lo, hi, e, salt),
                _ => metal_body(&mut o, lo, hi, e, salt),
            }
        } else {
            let c = (lo + hi) * 0.5;
            match tier {
                MAT_TWIG => {
                    let look = Look::new(Vec3::Y, 0.98).vary(salt, 0.04);
                    o.pole(
                        Vec3::new(c.x, lo.y, c.z),
                        Vec3::new(c.x, hi.y, c.z),
                        0.14,
                        Vec3::X,
                        &look,
                    );
                    lashings(&mut o, c, lo.y, hi.y);
                }
                MAT_WOOD => o.beam(lo, hi, 1, 0.04, &timber(Vec3::Y).across(0.5)),
                MAT_STONE => {
                    let lines = course_lines(lo.y, hi.y, COURSE_M, 0.0);
                    for (k, w) in lines.windows(2).enumerate() {
                        let l = Vec3::new(lo.x, w[0], lo.z);
                        let h = Vec3::new(hi.x, w[1], hi.z);
                        o.block(l, h, JOINT_M, &stone(salt * 31 + k as u32));
                    }
                }
                _ => o.beam(lo, hi, 1, 0.015, &steel(Vec3::Y)),
            }
        }
    }
    finish(o.b)
}

/// Rope bands on a twig corner post where the frame's poles meet it — the
/// lashing that makes a stick frame a lashed one.
fn lashings(o: &mut Out, c: Vec3, y0: f32, y1: f32) {
    let rope = Look::new(Vec3::X, 0.62).across(3.0);
    for y in [y0 + 0.09, (LEVEL_H_M * 0.5).min(y1 - 0.2), y1 - 0.09] {
        if y < y0 + 0.05 || y > y1 - 0.05 {
            continue;
        }
        o.pole(
            Vec3::new(c.x, y - 0.05, c.z),
            Vec3::new(c.x, y + 0.05, c.z),
            0.148,
            Vec3::X,
            &rope,
        );
    }
}

/// Twig: a mat of lashed sticks recessed between frame poles on both faces —
/// rails on the free edges, a girt across the middle, a brace in each panel.
fn twig_body(o: &mut Out, lo: Vec3, hi: Vec3, e: Edges, salt: u32) {
    const R: f32 = 0.055;
    const RECESS: f32 = 0.07;
    let h = hi.y - lo.y;
    if h < 0.3 {
        // A beam's worth of height: two poles side by side.
        let r = (h * 0.5).min(T2 * 0.5);
        let y = (lo.y + hi.y) * 0.5;
        for s in [-1.0f32, 1.0] {
            let x = s * (T2 - r);
            o.pole(
                Vec3::new(x, y, lo.z),
                Vec3::new(x, y, hi.z),
                r,
                Vec3::X * s,
                &twig_pole(salt ^ s.to_bits()),
            );
        }
        return;
    }
    // The mat, closed on a free edge where only poles cover it: a wall's
    // open top, an opening's head and its sides.
    let mut skip = SKIP_NY | SKIP_PY | SKIP_NZ | SKIP_PZ;
    if e.hi_y {
        skip &= !SKIP_PY;
    }
    if e.lo_y && !e.base {
        skip &= !SKIP_NY;
    }
    if e.lo_z {
        skip &= !SKIP_NZ;
    }
    if e.hi_z {
        skip &= !SKIP_PZ;
    }
    o.cuboid(
        Vec3::new(-T2 + RECESS, lo.y, lo.z),
        Vec3::new(T2 - RECESS, hi.y, hi.z),
        &twig_mat(),
        skip,
    );
    let f = Frame::of(lo, hi, e, 2.0 * R + 0.015, 2.0 * R, 2.0 * R);
    let yb = lo.y + R + 0.015;
    let yt = hi.y - R - 0.015;
    for (k, s) in [-1.0f32, 1.0].into_iter().enumerate() {
        let x = s * (T2 - R);
        let side = Vec3::X * s;
        let mut n = 0u32;
        let mut next = || {
            n += 1;
            twig_pole(salt.wrapping_mul(7) ^ (k as u32) << 16 ^ n)
        };
        let run = |o: &mut Out, y: f32, look: &Look| {
            o.pole(Vec3::new(x, y, lo.z), Vec3::new(x, y, hi.z), R, side, look)
        };
        if e.lo_y {
            run(o, yb, &next());
        }
        if e.hi_y {
            run(o, yt, &next());
        }
        if let Some((g0, g1)) = f.girt {
            run(o, (g0 + g1) * 0.5, &next());
        }
        for (free, z) in [(e.lo_z, lo.z + R), (e.hi_z, hi.z - R)] {
            if free {
                let look = next().grain(Vec3::Y);
                o.pole(Vec3::new(x, lo.y, z), Vec3::new(x, hi.y, z), R, side, &look);
            }
        }
        // Braces end inside the poles they are lashed to.
        let (za, zb) = (f.zb + 0.07, f.zt - 0.07);
        let mut ends = vec![if e.lo_y { yb } else { lo.y + 0.07 }];
        if let Some((g0, g1)) = f.girt {
            ends.push((g0 + g1) * 0.5);
        }
        ends.push(if e.hi_y { yt } else { hi.y - 0.07 });
        if zb - za > 1.0 {
            for (i, w) in ends.windows(2).enumerate() {
                if w[1] - w[0] < 0.6 {
                    continue;
                }
                let (z0, z1) = if i % 2 == 0 { (za, zb) } else { (zb, za) };
                o.pole(
                    Vec3::new(x, w[0], z0),
                    Vec3::new(x, w[1], z1),
                    R * 0.9,
                    side,
                    &next(),
                );
            }
        }
    }
}

/// Wood: vertical planks recessed in a heavy timber frame — sill and top
/// plates, stiles at openings, a girt, a brace in each panel.
fn wood_body(o: &mut Out, lo: Vec3, hi: Vec3, e: Edges) {
    const RAIL: f32 = 0.20;
    const STILE: f32 = 0.16;
    const GIRT: f32 = 0.16;
    const RECESS: f32 = 0.035;
    const CH: f32 = 0.02;
    if hi.y - lo.y < 2.2 * RAIL {
        o.beam(lo, hi, 2, CH, &timber(Vec3::Z));
        return;
    }
    let f = Frame::of(lo, hi, e, RAIL, STILE, GIRT);
    frame_members(o, lo, hi, e, &f, RAIL, STILE, CH, &timber(Vec3::Z));
    o.cuboid(
        Vec3::new(-T2 + RECESS, f.yb, f.zb),
        Vec3::new(T2 - RECESS, f.yt, f.zt),
        &planks(),
        ONLY_X,
    );
    if f.zt - f.zb >= 1.2 {
        for (i, (y0, y1)) in f.rows().into_iter().enumerate() {
            let Some((poly, g)) = brace(f.zb, f.zt, y0, y1, 0.15, i % 2 == 0) else {
                continue;
            };
            let look = timber(g).across(0.6);
            for s in [-1.0f32, 1.0] {
                o.sheet_x(&poly, s * (T2 - RECESS), s * T2, &look);
            }
        }
    }
}

/// The rails, stiles and girt a timber or steel frame puts on a body's free
/// edges, full thickness and flush with both faces.
#[allow(clippy::too_many_arguments)]
fn frame_members(
    o: &mut Out,
    lo: Vec3,
    hi: Vec3,
    e: Edges,
    f: &Frame,
    rail: f32,
    stile: f32,
    ch: f32,
    look: &Look,
) {
    let x = |y: f32, z: f32| Vec3::new(-T2, y, z);
    let xx = |y: f32, z: f32| Vec3::new(T2, y, z);
    if e.lo_y {
        o.beam(
            x(lo.y, lo.z),
            xx(lo.y + rail, hi.z),
            2,
            ch,
            &look.grain(Vec3::Z),
        );
    }
    if e.hi_y {
        o.beam(
            x(hi.y - rail, lo.z),
            xx(hi.y, hi.z),
            2,
            ch,
            &look.grain(Vec3::Z),
        );
    }
    if e.lo_z {
        o.beam(
            x(f.yb, lo.z),
            xx(f.yt, lo.z + stile),
            1,
            ch,
            &look.grain(Vec3::Y),
        );
    }
    if e.hi_z {
        o.beam(
            x(f.yb, hi.z - stile),
            xx(f.yt, hi.z),
            1,
            ch,
            &look.grain(Vec3::Y),
        );
    }
    if let Some((g0, g1)) = f.girt {
        o.beam(x(g0, f.zb), xx(g1, f.zt), 2, ch, &look.grain(Vec3::Z));
    }
}

/// Stone: courses of dressed blocks, joints chamfered in, a single stone
/// over an opening and along a free top.
fn stone_body(o: &mut Out, lo: Vec3, hi: Vec3, e: Edges, salt: u32) {
    let lines = course_lines(lo.y, hi.y, COURSE_M, 0.0);
    let n = lines.len() - 1;
    for (k, w) in lines.windows(2).enumerate() {
        let (y0, y1) = (w[0], w[1]);
        let course = ((y0 + y1) * 0.5 / COURSE_M).floor() as i32;
        // A lintel over an opening, a sill under one, a cap along a free top:
        // one long stone, which is how masonry spans a gap.
        let single = (k == 0 && e.lo_y && !e.base) || (k == n - 1 && e.hi_y && !e.head);
        let joints = if single {
            vec![lo.z, hi.z]
        } else {
            joint_lines(lo.z, hi.z, BLOCK_M, -H, course)
        };
        for (j, z) in joints.windows(2).enumerate() {
            for s in [-1.0f32, 1.0] {
                let seed = salt
                    .wrapping_mul(977)
                    .wrapping_add((course as u32).wrapping_mul(61) + j as u32 * 7)
                    ^ s.to_bits();
                o.pillow(
                    Vec3::X * s * T2,
                    Vec3::X * s,
                    (Vec3::Z, Vec3::Y),
                    [z[0], z[1], y0, y1],
                    [true; 4],
                    JOINT_M,
                    JOINT_M,
                    &stone(seed),
                );
            }
        }
    }
    // The core between the joint planes: the top, the foot and the ends.
    o.cuboid(
        Vec3::new(-T2 + JOINT_M, lo.y, lo.z),
        Vec3::new(T2 - JOINT_M, hi.y, hi.z),
        &stone(salt ^ 0x55),
        SKIP_PX | SKIP_NX,
    );
}

/// Metal: corrugated panels, one per bay and girt row, recessed in a steel
/// frame with cover straps between the bays.
fn metal_body(o: &mut Out, lo: Vec3, hi: Vec3, e: Edges, salt: u32) {
    const RAIL: f32 = 0.16;
    const STILE: f32 = 0.12;
    const RECESS: f32 = 0.04;
    const STRAP: f32 = 0.08;
    const CH: f32 = 0.012;
    if hi.y - lo.y < 2.2 * RAIL {
        o.beam(lo, hi, 2, CH, &steel(Vec3::Z));
        return;
    }
    let f = Frame::of(lo, hi, e, RAIL, STILE, RAIL);
    frame_members(o, lo, hi, e, &f, RAIL, STILE, CH, &steel(Vec3::Z));
    let bays = ((f.zt - f.zb) / 1.0).round().max(1.0) as usize;
    let bay = (f.zt - f.zb) / bays as f32;
    for (r, (y0, y1)) in f.rows().into_iter().enumerate() {
        for i in 0..bays {
            let z0 = f.zb + bay * i as f32;
            // Ribs run up the sheet: the map's ribs vary along `v`, so `v`
            // runs along the wall.
            let look = sheet(Vec3::Z, salt.wrapping_mul(13) + (r * 8 + i) as u32);
            o.cuboid(
                Vec3::new(-T2 + RECESS, y0, z0),
                Vec3::new(T2 - RECESS, y1, z0 + bay),
                &look,
                ONLY_X,
            );
            if i > 0 {
                let poly = [
                    Vec2::new(z0 - STRAP * 0.5, y0),
                    Vec2::new(z0 + STRAP * 0.5, y0),
                    Vec2::new(z0 + STRAP * 0.5, y1),
                    Vec2::new(z0 - STRAP * 0.5, y1),
                ];
                let strap = Look::new(Vec3::Z, 0.52).across(1.0);
                for s in [-1.0f32, 1.0] {
                    o.sheet_x(
                        &poly,
                        s * (T2 - RECESS),
                        s * T2,
                        &Look { sv: 0.35, ..strap },
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Slabs: floors, roofs and foundations
// ---------------------------------------------------------------------------

/// The right triangle a tri piece fills, in `(x, z)` — `tri_prism_into`'s.
fn tri_xz() -> [Vec2; 3] {
    [Vec2::new(-H, -H), Vec2::new(H, -H), Vec2::new(-H, H)]
}

/// A slab of `size` (a cell across, `size.y` deep, centred on its own
/// origin with its top the walk surface), dressed in `tier`. `foundation`
/// gives it the rim-and-skirt of a footing; otherwise it is a floor, framed
/// underneath.
pub fn slab_mesh(size: Vec3, tri: bool, foundation: bool, tier: u8) -> Mesh {
    let top = size.y * 0.5;
    let bot = -top;
    let mut o = Out::new(bot, top);
    if tri {
        tri_slab(&mut o, top, bot, foundation, tier);
    } else {
        match tier {
            MAT_TWIG => twig_slab(&mut o, top, bot, foundation),
            MAT_WOOD => wood_slab(&mut o, top, bot, foundation),
            MAT_STONE => stone_slab(&mut o, top, bot),
            _ => metal_slab(&mut o, top, bot, foundation),
        }
    }
    finish(o.b)
}

fn twig_slab(o: &mut Out, top: f32, bot: f32, foundation: bool) {
    const DECK: f32 = 0.07;
    const RIM: f32 = 0.11;
    let deck_bot = top - DECK;
    o.cuboid(
        Vec3::new(-H, deck_bot, -H),
        Vec3::new(H, top, H),
        &Look::new(Vec3::X, 1.0),
        if foundation { SKIP_NY } else { 0 },
    );
    let y = deck_bot - RIM;
    for s in [-1.0f32, 1.0] {
        let edge = s * (H - RIM);
        o.pole(
            Vec3::new(edge, y, -H),
            Vec3::new(edge, y, H),
            RIM,
            Vec3::X * s,
            &twig_pole(40 + s.to_bits()),
        );
        o.pole(
            Vec3::new(-H, y, edge),
            Vec3::new(H, y, edge),
            RIM,
            Vec3::Z * s,
            &twig_pole(41 + s.to_bits()),
        );
    }
    if !foundation {
        for (i, x) in [-0.5f32, 0.5].into_iter().enumerate() {
            let r = 0.085;
            o.pole(
                Vec3::new(x, deck_bot - r, -H + RIM),
                Vec3::new(x, deck_bot - r, H - RIM),
                r,
                Vec3::NEG_Y,
                &twig_pole(50 + i as u32),
            );
        }
        return;
    }
    // The skirt: upright sticks set back under the rim poles, lashed corner
    // posts standing proud of it.
    const SET: f32 = 0.05;
    if bot < y - 0.01 {
        o.cuboid(
            Vec3::new(-H + SET, bot, -H + SET),
            Vec3::new(H - SET, y, H - SET),
            &twig_mat(),
            SKIP_PY,
        );
    }
    let r = 0.12;
    for (i, (sx, sz)) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        let (x, z) = (sx * (H - r), sz * (H - r));
        o.pole(
            Vec3::new(x, bot, z),
            Vec3::new(x, deck_bot, z),
            r,
            Vec3::new(sx, 0.0, sz),
            &Look::new(Vec3::Y, 0.98).vary(60 + i as u32, 0.04),
        );
    }
}

fn wood_slab(o: &mut Out, top: f32, bot: f32, foundation: bool) {
    const DECK: f32 = 0.06;
    const RIM_W: f32 = 0.12;
    const CH: f32 = 0.015;
    let deck_bot = top - DECK;
    o.cuboid(
        Vec3::new(-H, deck_bot, -H),
        Vec3::new(H, top, H),
        &Look::new(Vec3::X, 1.05),
        if foundation { SKIP_NY } else { 0 },
    );
    // The rim: four beams under the deck's edges, flush with its sides.
    let rim_bot = if foundation {
        (top - 0.30).max(bot)
    } else {
        bot
    };
    for s in [-1.0f32, 1.0] {
        let (a, b) = (s * H, s * (H - RIM_W));
        o.beam(
            Vec3::new(a.min(b), rim_bot, -H),
            Vec3::new(a.max(b), deck_bot, H),
            2,
            CH,
            &timber(Vec3::Z),
        );
        o.beam(
            Vec3::new(-H + RIM_W, rim_bot, a.min(b)),
            Vec3::new(H - RIM_W, deck_bot, a.max(b)),
            0,
            CH,
            &timber(Vec3::X),
        );
    }
    if !foundation {
        // Joists under the planks, seen from the storey below.
        for x in [-0.75f32, 0.0, 0.75] {
            o.beam(
                Vec3::new(x - 0.05, bot + 0.03, -H + RIM_W),
                Vec3::new(x + 0.05, deck_bot, H - RIM_W),
                2,
                CH,
                &timber(Vec3::Z).vary((x * 10.0) as i32 as u32, 0.05),
            );
        }
        return;
    }
    if bot < rim_bot - 0.01 {
        const SET: f32 = 0.035;
        o.cuboid(
            Vec3::new(-H + SET, bot, -H + SET),
            Vec3::new(H - SET, rim_bot, H - SET),
            &planks().vary(70, 0.0),
            SKIP_PY,
        );
        let w = 0.2;
        for (sx, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let (x0, x1) = (sx * H, sx * (H - w));
            let (z0, z1) = (sz * H, sz * (H - w));
            o.beam(
                Vec3::new(x0.min(x1), bot, z0.min(z1)),
                Vec3::new(x0.max(x1), rim_bot, z0.max(z1)),
                1,
                0.03,
                &timber(Vec3::Y).across(0.5),
            );
        }
    }
}

fn stone_slab(o: &mut Out, top: f32, bot: f32) {
    const FLAG: f32 = 0.75;
    let d = JOINT_M;
    // Flagstones on the walk surface, rows offset by half a stone.
    let rows = joint_lines(-H, H, FLAG, -H, 0);
    for (r, zw) in rows.windows(2).enumerate() {
        let cols = joint_lines(-H, H, FLAG, -H, r as i32);
        for (c, xw) in cols.windows(2).enumerate() {
            o.pillow(
                Vec3::Y * top,
                Vec3::Y,
                (Vec3::X, Vec3::Z),
                [xw[0], xw[1], zw[0], zw[1]],
                [true; 4],
                JOINT_M,
                d,
                &stone(900 + (r * 8 + c) as u32).grain(Vec3::Z),
            );
        }
    }
    // The sides: a rim course, then coursed blocks down the skirt. Outer
    // edges stay square so the four sides and the flags close on each other.
    let mut lines = vec![top - d];
    let rim = top - 0.30;
    if rim > bot + 0.12 {
        lines.push(rim);
        let mut y = rim - COURSE_M;
        while y > bot + 0.12 {
            lines.push(y);
            y -= COURSE_M;
        }
    }
    lines.push(bot);
    lines.reverse();
    let n = lines.len() - 1;
    for (face, (nrm, along)) in [
        (Vec3::X, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z),
        (Vec3::Z, Vec3::X),
        (Vec3::NEG_Z, Vec3::X),
    ]
    .into_iter()
    .enumerate()
    {
        for (k, w) in lines.windows(2).enumerate() {
            let joints = joint_lines(-H, H, 1.0, -H + face as f32 * 0.25, k as i32);
            let m = joints.len() - 1;
            for (j, u) in joints.windows(2).enumerate() {
                o.pillow(
                    nrm * H,
                    nrm,
                    (along, Vec3::Y),
                    [u[0], u[1], w[0], w[1]],
                    [j > 0, j < m - 1, k > 0, k < n - 1],
                    JOINT_M,
                    d,
                    &stone(1200 + (face * 64 + k * 8 + j) as u32),
                );
            }
        }
    }
    o.face(
        &[
            Vec3::new(-H, bot, -H),
            Vec3::new(H, bot, -H),
            Vec3::new(H, bot, H),
            Vec3::new(-H, bot, H),
        ],
        Vec3::NEG_Y,
        &stone(1999),
    );
}

fn metal_slab(o: &mut Out, top: f32, bot: f32, foundation: bool) {
    const DECK: f32 = 0.04;
    const RIM_W: f32 = 0.10;
    const CH: f32 = 0.01;
    let deck_bot = top - DECK;
    o.cuboid(
        Vec3::new(-H, deck_bot, -H),
        Vec3::new(H, top, H),
        &sheet(Vec3::Z, 80),
        if foundation { SKIP_NY } else { 0 },
    );
    let rim_bot = if foundation {
        (top - 0.30).max(bot)
    } else {
        bot
    };
    for s in [-1.0f32, 1.0] {
        let (a, b) = (s * H, s * (H - RIM_W));
        o.beam(
            Vec3::new(a.min(b), rim_bot, -H),
            Vec3::new(a.max(b), deck_bot, H),
            2,
            CH,
            &steel(Vec3::Z),
        );
        o.beam(
            Vec3::new(-H + RIM_W, rim_bot, a.min(b)),
            Vec3::new(H - RIM_W, deck_bot, a.max(b)),
            0,
            CH,
            &steel(Vec3::X),
        );
    }
    if !foundation {
        for x in [-0.75f32, 0.75] {
            o.beam(
                Vec3::new(x - 0.04, bot + 0.04, -H + RIM_W),
                Vec3::new(x + 0.04, deck_bot, H - RIM_W),
                2,
                CH,
                &steel(Vec3::Z),
            );
        }
        return;
    }
    if bot < rim_bot - 0.01 {
        const SET: f32 = 0.04;
        let (lo, hi) = (
            Vec3::new(-H + SET, bot, -H + SET),
            Vec3::new(H - SET, rim_bot, H - SET),
        );
        // Ribs run up every side: `v` along each side's own length.
        o.cuboid(lo, hi, &sheet(Vec3::Z, 81), !(SKIP_PX | SKIP_NX));
        o.cuboid(lo, hi, &sheet(Vec3::X, 82), !(SKIP_PZ | SKIP_NZ));
        o.face(
            &[
                Vec3::new(lo.x, bot, lo.z),
                Vec3::new(hi.x, bot, lo.z),
                Vec3::new(hi.x, bot, hi.z),
                Vec3::new(lo.x, bot, hi.z),
            ],
            Vec3::NEG_Y,
            &steel(Vec3::X),
        );
        // Cover straps between the side panels' bays.
        for (nrm, along) in [
            (Vec3::X, Vec3::Z),
            (Vec3::NEG_X, Vec3::Z),
            (Vec3::Z, Vec3::X),
            (Vec3::NEG_Z, Vec3::X),
        ] {
            for t in [-0.5f32, 0.5] {
                let c = along * t;
                let half = along * 0.04 + nrm.abs() * SET * 0.5;
                let mid = c + nrm * (H - SET * 0.5) + Vec3::Y * (bot + rim_bot) * 0.5;
                let hy = Vec3::Y * (rim_bot - bot) * 0.5;
                o.cuboid(
                    mid - half - hy,
                    mid + half + hy,
                    &steel(Vec3::Y),
                    SKIP_PY | SKIP_NY,
                );
            }
        }
        let w = 0.18;
        for (sx, sz) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let (x0, x1) = (sx * H, sx * (H - w));
            let (z0, z1) = (sz * H, sz * (H - w));
            o.beam(
                Vec3::new(x0.min(x1), bot, z0.min(z1)),
                Vec3::new(x0.max(x1), rim_bot, z0.max(z1)),
                1,
                0.015,
                &steel(Vec3::Y),
            );
        }
    }
}

/// The half-cell triangles: a deck, a rim under it, and a footing's skirt set
/// back from the rim — stone in courses with a groove between each.
fn tri_slab(o: &mut Out, top: f32, bot: f32, foundation: bool, tier: u8) {
    let tri = tri_xz();
    let rim_bot = if foundation {
        (top - 0.30).max(bot)
    } else {
        bot
    };
    if tier == MAT_STONE {
        let d = JOINT_M;
        let groove = inset(&tri, d);
        // A cap course under the walk surface, then the rim and the skirt.
        let mut lines = vec![bot, top - 0.14];
        let mut y = rim_bot;
        while y > bot + 0.12 {
            lines.push(y);
            y -= COURSE_M;
        }
        lines.push(top);
        lines.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        lines.dedup_by(|a, b| (*a - *b).abs() < 0.12);
        let n = lines.len() - 1;
        for (k, w) in lines.windows(2).enumerate() {
            let y0 = if k == 0 { w[0] } else { w[0] + d };
            let y1 = if k == n - 1 { w[1] } else { w[1] - d };
            o.prism_y(
                &tri,
                y0,
                y1,
                &stone(1500 + k as u32).grain(Vec3::Y),
                [true, true],
            );
            if k + 1 < n {
                o.prism_y(
                    &groove,
                    w[1] - d,
                    w[1] + d,
                    &stone(1600 + k as u32),
                    [false, false],
                );
            }
        }
        return;
    }
    let (deck_t, deck, rim, skirt, set) = match tier {
        MAT_TWIG => (
            0.07,
            Look::new(Vec3::X, 1.0),
            Look::new(Vec3::X, 1.04),
            twig_mat(),
            0.05,
        ),
        MAT_WOOD => (
            0.06,
            Look::new(Vec3::X, 1.05),
            timber(Vec3::X),
            planks(),
            0.035,
        ),
        _ => (
            0.04,
            sheet(Vec3::Z, 90),
            steel(Vec3::X),
            sheet(Vec3::X, 91),
            0.04,
        ),
    };
    o.prism_y(&tri, top - deck_t, top, &deck, [false, true]);
    o.prism_y(&tri, rim_bot, top - deck_t, &rim, [true, false]);
    if foundation && bot < rim_bot - 0.01 {
        o.prism_y(&inset(&tri, set), bot, rim_bot, &skirt, [true, false]);
    }
}

/// Recolour a dressed mesh's two big faces for its soft side: what
/// `sided_parts_mesh` does to the plain parts, applied after tangents so the
/// two orientations share one tangent pass.
pub fn retint_sides(mesh: &Mesh, soft_positive_x: bool) -> Mesh {
    use bevy::mesh::VertexAttributeValues;
    let mut m = mesh.clone();
    let normals: Vec<[f32; 3]> = match m.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(n)) => n.clone(),
        _ => return m,
    };
    if let Some(VertexAttributeValues::Float32x4(col)) = m.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
        for (n, c) in normals.iter().zip(col.iter_mut()) {
            if n[0] == 0.0 {
                continue;
            }
            let gain = if (n[0] > 0.0) == soft_positive_x {
                super::FACE_TOP
            } else {
                super::FACE_BOTTOM
            };
            for ch in &mut c[..3] {
                *ch *= gain;
            }
        }
    }
    m
}
