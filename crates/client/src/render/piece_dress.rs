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
//! The risers are dressed over the sim's own walk patches the same way —
//! treads where the plain flight's are, stringers, carriages and blocks in
//! the slab the sim blocks beneath them — and so are the roofs (a lapped
//! covering on a floor's frame) and the floor frames.
//!
//! One mesh per shape × tier (× post ownership × soft side, × skirt step),
//! shared by every address, so a base of hundreds of pieces is still a few
//! hundred meshes. UVs stay metres (scaled per member where a beam has to show
//! one board rather than two), and every face carries the same mean-1
//! luminance tint `box_into` lays down, times a per-member value.

use std::f32::consts::PI;

use bevy::prelude::*;
use sim_core::build::{
    BUILD_CELL_M, LEVEL_H_M, MAT_METAL, MAT_STONE, MAT_TWIG, MAT_WOOD, SHAPE_FOUNDATION_STEPS,
    SHAPE_RAMP, SHAPE_STAIRS_TRI_SPIRAL,
};
use sim_core::collide::{FRAME_RIM_M, WALL_THICKNESS_M};

use super::{
    buffers, face_tint, finish, post_w, Buffers, Part, PartRole, EDGE_DROP_M, SKIRT_MAX_M, SLAB_T,
    STAIR_RISERS,
};

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
/// The chamfers of a block on an upright face, edges `[u0, u1, v0, v1]`: the
/// arris over a bed joint (the block's foot, `v0`) cut half as wide again,
/// so it tips 34° off the face rather than 45°. A 45° arris facing down in
/// a high sun went near-black and drew every bed joint twice as dark as the
/// head joints beside it.
const WALL_JOINTS: [f32; 4] = [JOINT_M, JOINT_M, JOINT_M * 1.5, JOINT_M];

/// How a member wears its tier's photograph: the grain direction (the map's
/// `v`), texture metres per metre across and along it, an offset so two
/// members do not show the same patch, and a scalar albedo tint. `turn`
/// lays the map a quarter-turn round, its `u` along the grain (and `su`
/// with it) — a steel section stretching the sheet's ribs out along its
/// length rather than across it.
#[derive(Clone, Copy, Debug)]
struct Look {
    g: Vec3,
    su: f32,
    sv: f32,
    off: Vec2,
    tint: f32,
    turn: bool,
}

impl Look {
    fn new(g: Vec3, tint: f32) -> Self {
        Self {
            g,
            su: 1.0,
            sv: 1.0,
            off: Vec2::ZERO,
            tint,
            turn: false,
        }
    }

    /// `(u, v)` in map metres: turned if the look says so, then scaled.
    fn map(&self, u: f32, v: f32) -> Vec2 {
        let (u, v) = if self.turn { (v, u) } else { (u, v) };
        Vec2::new(u * self.su, v * self.sv) + self.off
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
        self.map(u, v)
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
        self.face_with(pts, out, look, None, None, None);
    }

    /// A face that takes the face tint of `tint_n` rather than its own
    /// normal: a joint's chamfer reads as part of the stone face it is cut
    /// into, not as a top or an underside. Without it a wall's horizontal
    /// joints carried the 1.14 / 0.86 top-and-bottom gains on their two
    /// chamfers and read twice as stark as the vertical ones beside them.
    fn face_tinted(&mut self, pts: &[Vec3], out: Vec3, look: &Look, tint_n: Vec3) {
        self.face_with(pts, out, look, None, None, Some(tint_n));
    }

    fn face_with(
        &mut self,
        pts: &[Vec3],
        out: Vec3,
        look: &Look,
        uvs: Option<&[Vec2]>,
        normals: Option<&[Vec3]>,
        tint_n: Option<Vec3>,
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
            let t = face_tint(tint_n.unwrap_or(vn), p.y, self.y0, self.y1) * look.tint;
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
        let along = look.g.dot(g).abs() > 0.9;
        let uv = |arc: f32, s: f32| {
            if along {
                look.map(arc, s)
            } else {
                look.map(s, arc)
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
                self.face_with(&pts, out, look, Some(&uvs), Some(&ns), None);
            } else {
                self.face_with(&pts, out, look, Some(&uvs), None, None);
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
                    let bn = e[k] * s + e[u] * su;
                    self.face_tinted(&pts, bn, look, side_of(bn));
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
                    self.face_tinted(&pts, s, look, side_of(s));
                }
            }
        }
    }

    /// One block's face on a plane (`o` its point at `u = v = 0`, `n`
    /// outward): a front over `rect` = `[u0, u1, v0, v1]`, and on each edge
    /// `cut` names a chamfer `c[edge]` wide falling `d` deep to the rect's edge —
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
        c: [f32; 4],
        d: f32,
        look: &Look,
    ) {
        let k = |i: usize| if cut[i] { c[i] } else { 0.0 };
        let f = [r[0] + k(0), r[1] - k(1), r[2] + k(2), r[3] - k(3)];
        let at = |uu: f32, vv: f32, depth: f32| o + u * uu + v * vv - n * depth;
        self.face_tinted(
            &[
                at(f[0], f[2], 0.0),
                at(f[1], f[2], 0.0),
                at(f[1], f[3], 0.0),
                at(f[0], f[3], 0.0),
            ],
            n,
            look,
            n,
        );
        // The u-edges (u0, u1) run along v; the v-edges along u.
        for (e, fu, ru, dir) in [(0, f[0], r[0], -u), (1, f[1], r[1], u)] {
            if !cut[e] {
                continue;
            }
            self.face_tinted(
                &[
                    at(fu, f[2], 0.0),
                    at(fu, f[3], 0.0),
                    at(ru, r[3], d),
                    at(ru, r[2], d),
                ],
                dir + n,
                look,
                n,
            );
            for (ve, rv, vdir) in [(2, r[2], -v), (3, r[3], v)] {
                if !cut[ve] {
                    self.face_tinted(
                        &[at(fu, rv, 0.0), at(ru, rv, d), at(ru, rv, 0.0)],
                        vdir,
                        look,
                        n,
                    );
                }
            }
        }
        for (e, fv, rv, dir) in [(2, f[2], r[2], -v), (3, f[3], r[3], v)] {
            if !cut[e] {
                continue;
            }
            self.face_tinted(
                &[
                    at(f[0], fv, 0.0),
                    at(f[1], fv, 0.0),
                    at(r[1], rv, d),
                    at(r[0], rv, d),
                ],
                dir + n,
                look,
                n,
            );
            for (ue, ru, udir) in [(0, r[0], -u), (1, r[1], u)] {
                if !cut[ue] {
                    self.face_tinted(
                        &[at(ru, fv, 0.0), at(ru, rv, d), at(ru, rv, 0.0)],
                        udir,
                        look,
                        n,
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

    /// How many vertices are down so far — the mark [`Out::place_since`]
    /// moves from.
    fn mark(&self) -> usize {
        self.b.0.len()
    }

    /// Turn everything emitted since `start` a whole number of quarter-turns
    /// about +y and move it by `off`: `m` maps `(x, z)` and holds only 0 and
    /// ±1 with determinant +1, so the move is exact and keeps every winding.
    fn place_since(&mut self, start: usize, m: [[f32; 2]; 2], off: Vec3) {
        let turn = |v: [f32; 3]| {
            [
                m[0][0] * v[0] + m[0][1] * v[2],
                v[1],
                m[1][0] * v[0] + m[1][1] * v[2],
            ]
        };
        let (pos, nor, ..) = &mut self.b;
        for p in &mut pos[start..] {
            let q = turn(*p);
            *p = [q[0] + off.x, q[1] + off.y, q[2] + off.z];
        }
        for n in &mut nor[start..] {
            *n = turn(*n);
        }
    }

    /// A plate over the convex footprint `poly` in `(x, z)`, its top and its
    /// foot at per-point heights (each planar); `caps` = `[foot, top]`.
    fn plate(
        &mut self,
        poly: &[Vec2],
        top: impl Fn(Vec2) -> f32,
        foot: impl Fn(Vec2) -> f32,
        look: &Look,
        caps: [bool; 2],
    ) {
        let poly = tidy(poly);
        let n = poly.len();
        if n < 3 {
            return;
        }
        let c = centroid(&poly);
        let up: Vec<Vec3> = poly.iter().map(|&p| Vec3::new(p.x, top(p), p.y)).collect();
        let dn: Vec<Vec3> = poly.iter().map(|&p| Vec3::new(p.x, foot(p), p.y)).collect();
        if caps[1] {
            self.face(&up, Vec3::Y, look);
        }
        if caps[0] {
            self.face(&dn, Vec3::NEG_Y, look);
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let mid = (poly[i] + poly[j]) * 0.5 - c;
            self.face(
                &[dn[i], dn[j], up[j], up[i]],
                Vec3::new(mid.x, 0.0, mid.y),
                look,
            );
        }
    }

    /// A level plate between `y0` and `y1`.
    fn slab(&mut self, poly: &[Vec2], y0: f32, y1: f32, look: &Look, caps: [bool; 2]) {
        self.plate(poly, |_| y1, |_| y0, look, caps);
    }

    /// One stone over the convex `poly` with its face at `y`: the face pulled
    /// in by `c`, and a chamfer ring falling `d` to the polygon's edge — the
    /// flag's half of a joint, [`Out::pillow`] for any outline. False (and
    /// nothing drawn) for a sliver too thin to take the chamfer.
    fn pillow_poly(&mut self, poly: &[Vec2], y: f32, c: f32, d: f32, look: &Look) -> bool {
        let poly = tidy(poly);
        if poly.len() < 3 || area2(&poly).abs() < 0.01 {
            return false;
        }
        let top = inset(&poly, c);
        let (a0, a1) = (area2(&poly), area2(&top));
        if a0 * a1 <= 0.0 || a1.abs() < 0.004 || top.iter().any(|p| !p.is_finite()) {
            return false;
        }
        let up: Vec<Vec3> = top.iter().map(|p| Vec3::new(p.x, y, p.y)).collect();
        self.face(&up, Vec3::Y, look);
        let cen = centroid(&poly);
        for i in 0..poly.len() {
            let j = (i + 1) % poly.len();
            let mid = ((poly[i] + poly[j]) * 0.5 - cen).normalize_or_zero();
            self.face_tinted(
                &[
                    Vec3::new(poly[i].x, y - d, poly[i].y),
                    Vec3::new(poly[j].x, y - d, poly[j].y),
                    up[j],
                    up[i],
                ],
                Vec3::new(mid.x, 1.0, mid.y),
                look,
                Vec3::Y,
            );
        }
        true
    }

    /// Poles along `poly`'s edges, `r` in from them, at height `y`, each
    /// stopped short of its corners so no end pokes past the next edge.
    fn rim_poles(&mut self, poly: &[Vec2], y: f32, r: f32, seed: u32) {
        let poly = tidy(poly);
        let mid = inset(&poly, r);
        let n = poly.len();
        let c = centroid(&poly);
        for i in 0..n {
            let (a, b) = (mid[i], mid[(i + 1) % n]);
            let len = a.distance(b);
            if len < 4.0 * r {
                continue;
            }
            let d = (b - a) / len;
            let mut out = Vec2::new(d.y, -d.x);
            if out.dot(c - a) > 0.0 {
                out = -out;
            }
            let (a, b) = (a + d * r, b - d * r);
            self.pole(
                Vec3::new(a.x, y, a.y),
                Vec3::new(b.x, y, b.y),
                r,
                Vec3::new(out.x, 0.0, out.y),
                &twig_pole(seed.wrapping_add(i as u32 * 17)),
            );
        }
    }

    /// The mitred members of a frame round `poly`, `w` wide, `y0..y1`, each
    /// wearing `look` with its grain along its own edge.
    fn rim_plates(&mut self, poly: &[Vec2], w: f32, y0: f32, y1: f32, look: impl Fn(Vec3) -> Look) {
        let poly = tidy(poly);
        let inner = inset(&poly, w);
        let n = poly.len();
        for i in 0..n {
            let j = (i + 1) % n;
            let d = (poly[j] - poly[i]).normalize();
            self.slab(
                &[poly[i], poly[j], inner[j], inner[i]],
                y0,
                y1,
                &look(Vec3::new(d.x, 0.0, d.y)),
                [true, true],
            );
        }
    }
}

/// A face tint's normal for a chamfer: the horizontal part of `n`, so a
/// joint's bevel takes the side gain of the face it is cut into.
fn side_of(n: Vec3) -> Vec3 {
    let h = Vec3::new(n.x, 0.0, n.z);
    if h.length_squared() > 1e-6 {
        h.normalize()
    } else {
        n
    }
}

fn area2(poly: &[Vec2]) -> f32 {
    (0..poly.len())
        .map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()]))
        .sum()
}

fn centroid(poly: &[Vec2]) -> Vec2 {
    poly.iter().copied().sum::<Vec2>() / poly.len().max(1) as f32
}

/// `poly` without repeated or collinear corners, which a clip leaves and
/// which [`inset`] cannot take (two parallel edges have no corner).
fn tidy(poly: &[Vec2]) -> Vec<Vec2> {
    let mut v: Vec<Vec2> = Vec::with_capacity(poly.len());
    for &p in poly {
        if v.last().is_none_or(|q| q.distance(p) > 1e-4) {
            v.push(p);
        }
    }
    while v.len() > 1 && v[0].distance(v[v.len() - 1]) <= 1e-4 {
        v.pop();
    }
    loop {
        let n = v.len();
        if n < 3 {
            return v;
        }
        let Some(i) = (0..n).find(|&i| {
            let (a, b, c) = (v[(i + n - 1) % n], v[i], v[(i + 1) % n]);
            (b - a).perp_dot(c - b).abs() < 1e-6
        }) else {
            return v;
        };
        v.remove(i);
    }
}

/// `poly` (convex) cut to the half-plane `n · p <= d`.
fn clip_half(poly: &[Vec2], n: Vec2, d: f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (n.dot(a) - d, n.dot(b) - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// `poly` cut to the convex `hull`.
fn clip_to(poly: &[Vec2], hull: &[Vec2]) -> Vec<Vec2> {
    let c = centroid(hull);
    let mut out = poly.to_vec();
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let mut n = Vec2::new(b.y - a.y, a.x - b.x);
        if n.dot(c - a) > 0.0 {
            n = -n;
        }
        out = clip_half(&out, n, n.dot(a));
        if out.len() < 3 {
            return Vec::new();
        }
    }
    tidy(&out)
}

fn rect(x0: f32, x1: f32, z0: f32, z1: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(x0, z0),
        Vec2::new(x1, z0),
        Vec2::new(x1, z1),
        Vec2::new(x0, z1),
    ]
}

/// The square cell's footprint, or the NW half's.
fn footprint(tri: bool) -> Vec<Vec2> {
    if tri {
        tri_xz().to_vec()
    } else {
        rect(-H, H, -H, H)
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
/// Corrugated sheet. The map's ribs vary along `u`, so `g` is the way the
/// ribs RUN: up a wall (`Y`), down a roof. 0.85 holds weathered sheet
/// (luma 0.189) a clear step under the stone tier's ~0.25.
fn sheet(g: Vec3, seed: u32) -> Look {
    Look::new(g, 0.85).vary(seed, 0.06)
}
/// A steel section, darker than its sheet: the map turned so its ribs cross
/// the section and stretched sixteen-fold along it, so a face takes a
/// sliver of one rib — flat plate streaked lengthwise with worn paint, not a
/// pipe of corrugation.
fn steel(g: Vec3) -> Look {
    Look {
        turn: true,
        ..Look::new(g, 0.55).across(0.06)
    }
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
            post(&mut o, tier, lo, hi, salt, Some(0.0));
        }
    }
    finish(o.b)
}

/// A corner post in `tier`: a lashed pole, a chamfered timber, a stack of
/// dressed stones on the course grid `courses` counts from, a steel column.
/// `courses` is `None` for an apron's post, which also takes no lashings.
fn post(o: &mut Out, tier: u8, lo: Vec3, hi: Vec3, salt: u32, courses: Option<f32>) {
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
            if courses.is_some() {
                lashings(o, c, lo.y, hi.y);
            }
        }
        MAT_WOOD => o.beam(lo, hi, 1, 0.04, &timber(Vec3::Y).across(0.5)),
        MAT_STONE => {
            let lines = course_lines(lo.y, hi.y, COURSE_M, courses.unwrap_or(APRON_COURSE_M));
            for (k, w) in lines.windows(2).enumerate() {
                let l = Vec3::new(lo.x, w[0], lo.z);
                let h = Vec3::new(hi.x, w[1], hi.z);
                o.block(l, h, JOINT_M, &stone(salt * 31 + k as u32));
            }
        }
        _ => o.beam(lo, hi, 1, 0.015, &steel(Vec3::Y)),
    }
}

/// Where an apron's top course ends, storey-local: a footing's rim depth
/// under the storey base, so the plinth under a wall courses level with the
/// foundation beside it.
const APRON_COURSE_M: f32 = -0.30;

/// A ground-storey edge piece's apron ([`super::apron_parts`]) dressed in
/// `tier` as the footing it stands in front of: a sill timber, log, cap
/// course or steel rim under the wall's foot, and the skirt's infill below.
pub fn apron_mesh(parts: &[Part], tier: u8) -> Mesh {
    let bot = parts
        .iter()
        .map(|p| p.offset.y - p.size.y * 0.5)
        .fold(f32::MAX, f32::min);
    let mut o = Out::new(bot, -EDGE_DROP_M);
    for (i, p) in parts.iter().enumerate() {
        let (lo, hi) = bounds(p);
        let salt = 0x400 | (tier as u32) << 8 | i as u32;
        if p.role != PartRole::Body {
            post(&mut o, tier, lo, hi, salt, None);
            continue;
        }
        match tier {
            MAT_TWIG => {
                let r = T2;
                o.pole(
                    Vec3::new(0.0, hi.y - r, lo.z),
                    Vec3::new(0.0, hi.y - r, hi.z),
                    r,
                    Vec3::X,
                    &twig_pole(salt),
                );
                o.cuboid(
                    Vec3::new(-T2 + 0.05, lo.y, lo.z),
                    Vec3::new(T2 - 0.05, hi.y - r, hi.z),
                    &twig_mat(),
                    ONLY_X,
                );
            }
            MAT_WOOD => {
                let rim = hi.y - 0.26;
                o.beam(
                    Vec3::new(-T2, rim, lo.z),
                    Vec3::new(T2, hi.y, hi.z),
                    2,
                    0.02,
                    &timber(Vec3::Z),
                );
                o.cuboid(
                    Vec3::new(-T2 + 0.035, lo.y, lo.z),
                    Vec3::new(T2 - 0.035, rim, hi.z),
                    &planks(),
                    ONLY_X,
                );
            }
            MAT_STONE => {
                let lines = course_lines(lo.y, hi.y, COURSE_M, APRON_COURSE_M);
                for (k, w) in lines.windows(2).enumerate() {
                    let joints = joint_lines(lo.z, hi.z, BLOCK_M, -H, k as i32);
                    for (j, z) in joints.windows(2).enumerate() {
                        for s in [-1.0f32, 1.0] {
                            o.pillow(
                                Vec3::X * s * T2,
                                Vec3::X * s,
                                (Vec3::Z, Vec3::Y),
                                [z[0], z[1], w[0], w[1]],
                                [true; 4],
                                WALL_JOINTS,
                                JOINT_M,
                                &stone(
                                    (salt.wrapping_mul(131) + (k * 16 + j) as u32) ^ s.to_bits(),
                                ),
                            );
                        }
                    }
                }
                o.cuboid(
                    Vec3::new(-T2 + JOINT_M, lo.y, lo.z),
                    Vec3::new(T2 - JOINT_M, hi.y, hi.z),
                    &stone(salt),
                    SKIP_PX | SKIP_NX,
                );
            }
            _ => {
                let rim = hi.y - 0.22;
                o.beam(
                    Vec3::new(-T2, rim, lo.z),
                    Vec3::new(T2, hi.y, hi.z),
                    2,
                    0.012,
                    &steel(Vec3::Z),
                );
                o.cuboid(
                    Vec3::new(-T2 + 0.04, lo.y, lo.z),
                    Vec3::new(T2 - 0.04, rim, hi.z),
                    &sheet(Vec3::Y, salt),
                    ONLY_X,
                );
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
                    WALL_JOINTS,
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
            // Ribs run up the sheet.
            let look = sheet(Vec3::Y, salt.wrapping_mul(13) + (r * 8 + i) as u32);
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
                for s in [-1.0f32, 1.0] {
                    o.sheet_x(&poly, s * (T2 - RECESS), s * T2, &steel(Vec3::Y));
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
    slab_as(size, tri, foundation, false, tier)
}

/// A roof: a floor's frame under a lapped covering — thatch, shingles,
/// slates or corrugated sheet — in place of its deck, the courses' butts at
/// the walk surface and everything else below it.
pub fn roof_mesh(tri: bool, tier: u8) -> Mesh {
    slab_as(Vec3::new(2.0 * H, SLAB_T, 2.0 * H), tri, false, true, tier)
}

fn slab_as(size: Vec3, tri: bool, foundation: bool, roof: bool, tier: u8) -> Mesh {
    let top = size.y * 0.5;
    let bot = -top;
    let mut o = Out::new(bot, top);
    if tri {
        tri_slab(&mut o, top, bot, foundation, roof, tier);
    } else {
        match tier {
            MAT_TWIG => twig_slab(&mut o, top, bot, foundation, roof),
            MAT_WOOD => wood_slab(&mut o, top, bot, foundation, roof),
            MAT_STONE => stone_slab(&mut o, top, bot, roof),
            _ => metal_slab(&mut o, top, bot, foundation, roof),
        }
    }
    finish(o.b)
}

fn twig_slab(o: &mut Out, top: f32, bot: f32, foundation: bool, roof: bool) {
    const DECK: f32 = 0.07;
    const RIM: f32 = 0.11;
    let deck_bot = top - DECK;
    if roof {
        roofing(o, &footprint(false), top, deck_bot, MAT_TWIG);
    } else {
        o.cuboid(
            Vec3::new(-H, deck_bot, -H),
            Vec3::new(H, top, H),
            &Look::new(Vec3::X, 1.0),
            if foundation { SKIP_NY } else { 0 },
        );
    }
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

fn wood_slab(o: &mut Out, top: f32, bot: f32, foundation: bool, roof: bool) {
    const DECK: f32 = 0.06;
    const RIM_W: f32 = 0.12;
    const CH: f32 = 0.015;
    let deck_bot = top - DECK;
    if roof {
        roofing(o, &footprint(false), top, deck_bot, MAT_WOOD);
    } else {
        o.cuboid(
            Vec3::new(-H, deck_bot, -H),
            Vec3::new(H, top, H),
            &Look::new(Vec3::X, 1.05),
            if foundation { SKIP_NY } else { 0 },
        );
    }
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

fn stone_slab(o: &mut Out, top: f32, bot: f32, roof: bool) {
    const FLAG: f32 = 0.75;
    let d = JOINT_M;
    // Slates on a roof; flagstones on a floor, rows offset by half a stone.
    let rows = if roof {
        roofing(o, &footprint(false), top, top - d - 0.01, MAT_STONE);
        Vec::new()
    } else {
        joint_lines(-H, H, FLAG, -H, 0)
    };
    for (r, zw) in rows.windows(2).enumerate() {
        let cols = joint_lines(-H, H, FLAG, -H, r as i32);
        for (c, xw) in cols.windows(2).enumerate() {
            o.pillow(
                Vec3::Y * top,
                Vec3::Y,
                (Vec3::X, Vec3::Z),
                [xw[0], xw[1], zw[0], zw[1]],
                [true; 4],
                [JOINT_M; 4],
                d,
                &stone(900 + (r * 8 + c) as u32).grain(Vec3::Z),
            );
        }
    }
    // The sides: a rim course, then coursed blocks down the skirt. Outer
    // edges stay square so the four sides and the flags close on each other.
    let lines = stone_courses(top, bot, d);
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
                    WALL_JOINTS,
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

fn metal_slab(o: &mut Out, top: f32, bot: f32, foundation: bool, roof: bool) {
    const DECK: f32 = 0.04;
    const RIM_W: f32 = 0.10;
    const CH: f32 = 0.01;
    let deck_bot = top - DECK;
    if roof {
        roofing(o, &footprint(false), top, deck_bot, MAT_METAL);
    } else {
        o.cuboid(
            Vec3::new(-H, deck_bot, -H),
            Vec3::new(H, top, H),
            &sheet(Vec3::Z, 80),
            if foundation { SKIP_NY } else { 0 },
        );
    }
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
        // Ribs run up every side.
        o.cuboid(lo, hi, &sheet(Vec3::Y, 81), SKIP_PY | SKIP_NY);
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

/// The half-cell triangles, dressed as the square slabs are: the same deck,
/// rim, joists, skirt and corner posts, run round three edges instead of
/// four — the hypotenuse taking a mitred member, a pole, a coursed face of
/// its own.
fn tri_slab(o: &mut Out, top: f32, bot: f32, foundation: bool, roof: bool, tier: u8) {
    let tri = footprint(true);
    match tier {
        MAT_TWIG => twig_tri(o, &tri, top, bot, foundation, roof),
        MAT_STONE => stone_tri(o, &tri, top, bot, roof),
        _ => framed_tri(o, &tri, top, bot, foundation, roof, tier),
    }
}

fn twig_tri(o: &mut Out, tri: &[Vec2], top: f32, bot: f32, foundation: bool, roof: bool) {
    const DECK: f32 = 0.07;
    const RIM: f32 = 0.11;
    const SET: f32 = 0.05;
    let deck_bot = top - DECK;
    if roof {
        roofing(o, tri, top, deck_bot, MAT_TWIG);
    } else {
        o.slab(
            tri,
            deck_bot,
            top,
            &Look::new(Vec3::X, 1.0),
            [!foundation, true],
        );
    }
    let y = deck_bot - RIM;
    o.rim_poles(tri, y, RIM, 40);
    if !foundation {
        // Joists under the mat, stopped inside the hypotenuse's rim pole.
        for (i, x) in [-0.75f32, 0.0].into_iter().enumerate() {
            let r = 0.085;
            let (z0, z1) = (
                -H + 2.0 * RIM,
                -x - RIM * std::f32::consts::SQRT_2 - 1.5 * r,
            );
            if z1 - z0 > 0.3 {
                o.pole(
                    Vec3::new(x, deck_bot - r, z0),
                    Vec3::new(x, deck_bot - r, z1),
                    r,
                    Vec3::NEG_Y,
                    &twig_pole(50 + i as u32),
                );
            }
        }
        return;
    }
    if bot < y - 0.01 {
        o.slab(&inset(tri, SET), bot, y, &twig_mat(), [true, false]);
    }
    let r = 0.12;
    for (i, p) in inset(tri, r).into_iter().enumerate() {
        o.pole(
            Vec3::new(p.x, bot, p.y),
            Vec3::new(p.x, deck_bot, p.y),
            r,
            Vec3::X,
            &Look::new(Vec3::Y, 0.98).vary(60 + i as u32, 0.04),
        );
    }
}

/// Wood and metal: a deck on a mitred rim, joists under a floor, a set-back
/// skirt and corner posts under a footing.
fn framed_tri(
    o: &mut Out,
    tri: &[Vec2],
    top: f32,
    bot: f32,
    foundation: bool,
    roof: bool,
    tier: u8,
) {
    let wood = tier == MAT_WOOD;
    let (deck_t, rim_w, set, member): (f32, f32, f32, fn(Vec3) -> Look) = if wood {
        (0.06, 0.12, 0.035, timber)
    } else {
        (0.04, 0.10, 0.04, steel)
    };
    let deck_bot = top - deck_t;
    if roof {
        roofing(o, tri, top, deck_bot, tier);
    } else {
        let deck = if wood {
            Look::new(Vec3::X, 1.05)
        } else {
            sheet(Vec3::Z, 90)
        };
        o.slab(tri, deck_bot, top, &deck, [!foundation, true]);
    }
    let rim_bot = if foundation {
        (top - 0.30).max(bot)
    } else {
        bot
    };
    o.rim_plates(tri, rim_w, rim_bot, deck_bot, member);
    if !foundation {
        let (xs, hw, foot): (&[f32], f32, f32) = if wood {
            (&[-0.75, 0.0], 0.05, 0.03)
        } else {
            (&[-0.75, 0.0], 0.04, 0.04)
        };
        for &x in xs {
            let z0 = -H + rim_w;
            let z1 = -(x + hw) - rim_w * std::f32::consts::SQRT_2 - 0.01;
            if z1 - z0 > 0.3 {
                o.beam(
                    Vec3::new(x - hw, bot + foot, z0),
                    Vec3::new(x + hw, deck_bot, z1),
                    2,
                    if wood { 0.015 } else { 0.01 },
                    &member(Vec3::Z),
                );
            }
        }
        return;
    }
    if bot >= rim_bot - 0.01 {
        return;
    }
    let skirt = if wood {
        planks().vary(70, 0.0)
    } else {
        sheet(Vec3::Y, 91)
    };
    o.slab(&inset(tri, set), bot, rim_bot, &skirt, [true, false]);
    // Corner posts: a square one in the right angle, a wedge in each of the
    // two sharp corners, where a square post would stand out of the cell.
    let w = if wood { 0.2 } else { 0.18 };
    o.beam(
        Vec3::new(-H, bot, -H),
        Vec3::new(-H + w, rim_bot, -H + w),
        1,
        if wood { 0.03 } else { 0.015 },
        &member(Vec3::Y).across(0.5),
    );
    for (v, a, b) in [(tri[1], tri[0], tri[2]), (tri[2], tri[0], tri[1])] {
        let wedge = [
            v,
            v + (a - v).normalize() * w * 1.6,
            v + (b - v).normalize() * w * 1.6,
        ];
        o.slab(
            &wedge,
            bot,
            rim_bot,
            &member(Vec3::Y).across(0.5),
            [true, false],
        );
    }
    if !wood {
        // Cover straps on the two square sides, as the square footing has.
        for (nrm, along) in [(Vec3::NEG_X, Vec3::Z), (Vec3::NEG_Z, Vec3::X)] {
            for t in [-0.5f32, 0.5] {
                let mid = along * t + nrm * (H - set * 0.5) + Vec3::Y * (bot + rim_bot) * 0.5;
                let half = along * 0.04 + nrm.abs() * set * 0.5;
                let hy = Vec3::Y * (rim_bot - bot) * 0.5;
                o.cuboid(
                    mid - half - hy,
                    mid + half + hy,
                    &steel(Vec3::Y),
                    SKIP_PY | SKIP_NY,
                );
            }
        }
    }
}

/// Stone: flags on top (or slates on a roof), coursed blocks down the two
/// square sides and the hypotenuse, the outer corners square.
fn stone_tri(o: &mut Out, tri: &[Vec2], top: f32, bot: f32, roof: bool) {
    const FLAG: f32 = 0.75;
    let d = JOINT_M;
    if roof {
        roofing(o, tri, top, top - d - 0.01, MAT_STONE);
    } else {
        for (r, zw) in joint_lines(-H, H, FLAG, -H, 0).windows(2).enumerate() {
            for (c, xw) in joint_lines(-H, H, FLAG, -H, r as i32)
                .windows(2)
                .enumerate()
            {
                let cell = clip_to(&rect(xw[0], xw[1], zw[0], zw[1]), tri);
                if cell.len() >= 3 {
                    o.pillow_poly(
                        &cell,
                        top,
                        JOINT_M,
                        d,
                        &stone(900 + (r * 8 + c) as u32).grain(Vec3::Z),
                    );
                }
            }
        }
        // Under the flags, so a corner too thin to dress is not a hole.
        o.slab(
            &inset(tri, 0.01),
            bot + 0.01,
            top - d,
            &stone(1998),
            [false, true],
        );
    }
    let lines = stone_courses(top, bot, d);
    let n = lines.len() - 1;
    let r2 = std::f32::consts::FRAC_1_SQRT_2;
    let hyp = Vec3::new(r2, 0.0, r2);
    for (face, (o_pt, nrm, along, u0, u1)) in [
        (Vec3::NEG_X * H, Vec3::NEG_X, Vec3::Z, -H, H),
        (Vec3::NEG_Z * H, Vec3::NEG_Z, Vec3::X, -H, H),
        (
            Vec3::ZERO,
            hyp,
            Vec3::new(r2, 0.0, -r2),
            -H * std::f32::consts::SQRT_2,
            H * std::f32::consts::SQRT_2,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        for (k, w) in lines.windows(2).enumerate() {
            let joints = joint_lines(u0, u1, 1.0, u0 + face as f32 * 0.25, k as i32);
            let m = joints.len() - 1;
            for (j, u) in joints.windows(2).enumerate() {
                // At a sharp corner a bed joint's chamfer would cut through
                // the other face: the end stones there are square quoins.
                let quoin = (j == m - 1) || (face == 2 && j == 0);
                o.pillow(
                    o_pt,
                    nrm,
                    (along, Vec3::Y),
                    [u[0], u[1], w[0], w[1]],
                    [j > 0, j < m - 1, k > 0 && !quoin, k < n - 1 && !quoin],
                    WALL_JOINTS,
                    d,
                    &stone(1300 + (face * 64 + k * 8 + j) as u32),
                );
            }
        }
    }
    let base: Vec<Vec3> = tri.iter().map(|p| Vec3::new(p.x, bot, p.y)).collect();
    o.face(&base, Vec3::NEG_Y, &stone(1999));
}

/// A stone slab's side courses, bottom up: the bottom, the courses under the
/// rim, the rim, and the line `d` under the walk surface the flags fall to.
fn stone_courses(top: f32, bot: f32, d: f32) -> Vec<f32> {
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
    lines
}

// ---------------------------------------------------------------------------
// Roofs
// ---------------------------------------------------------------------------

/// A roof's covering over `fp`, from `under` to the walk surface `top`:
/// lapped courses whose butts stand at `top` and whose tails fall toward +z,
/// where the next course's butt laps over them — thatch bundles, cut
/// shingles, split slates — or, for metal, corrugated sheets running the
/// other way under a flashed edge. An underlay below the courses closes
/// every gap between them, so no joint is a slit of sky.
fn roofing(o: &mut Out, fp: &[Vec2], top: f32, under: f32, tier: u8) {
    let (xlo, xhi) = fp
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (zlo, zhi) = fp
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    if tier == MAT_METAL {
        const FLASH: f32 = 0.06;
        let seat = top - 0.03;
        o.slab(fp, under, seat, &steel(Vec3::X), [true, true]);
        o.rim_plates(fp, FLASH, seat, top, steel);
        let inner = inset(fp, FLASH);
        let (a, b) = inner
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
        let n = ((b - a) / 0.95).round().max(1.0) as usize;
        let w = (b - a) / n as f32;
        for i in 0..n {
            let x0 = a + w * i as f32;
            let cell = clip_to(&rect(x0, x0 + w, zlo, zhi), &inner);
            let lap = if i % 2 == 0 { 0.008 } else { 0.014 };
            o.plate(
                &cell,
                |_| top - lap,
                |_| seat,
                &sheet(Vec3::Z, 92 + i as u32),
                [false, true],
            );
        }
        return;
    }
    // (course pitch, tail drop, unit widths, gap, how far the course sits
    // under the walk surface)
    let (pitch, drop, wmin, wmax, gap, seat_d) = match tier {
        MAT_TWIG => (0.3, 0.03, 0.9, 1.5, 0.0, 0.045),
        MAT_WOOD => (0.3, 0.02, 0.18, 0.36, 0.007, 0.035),
        _ => (0.375, 0.018, 0.32, 0.6, 0.008, JOINT_M),
    };
    let seat = top - seat_d;
    if tier == MAT_STONE {
        // Inside the side courses, which stand to `seat` at the cell's edge.
        o.slab(&inset(fp, 0.004), under, seat, &stone(1990), [false, true]);
    } else if seat > under + 0.004 {
        let look = if tier == MAT_TWIG {
            twig_mat()
        } else {
            planks().grain(Vec3::X)
        };
        o.slab(fp, under, seat, &look, [true, true]);
    }
    let mut z = zlo;
    let mut row = 0u32;
    while z < zhi - 1e-3 {
        let z1 = (z + pitch).min(zhi);
        let span = z1 - z;
        let mut x = xlo - rand01(row, 9) * wmin;
        let mut i = 0u32;
        while x < xhi - 1e-3 {
            let w = wmin + (wmax - wmin) * rand01(row * 64 + i, 5);
            let a = if x > xlo + 1e-3 { x + gap * 0.5 } else { xlo };
            let b = if x + w < xhi - 1e-3 {
                x + w - gap * 0.5
            } else {
                xhi
            };
            let cell = clip_to(&rect(a, b, z, z1), fp);
            if cell.len() >= 3 && b - a > 0.03 {
                let seed = 2000 + row * 64 + i;
                let look = match tier {
                    MAT_TWIG => Look::new(Vec3::Z, 0.95).vary(seed, 0.08),
                    MAT_WOOD => planks().grain(Vec3::Z).across(0.5).vary(seed, 0.1),
                    _ => stone(seed).grain(Vec3::Z),
                };
                let z0 = z;
                o.plate(
                    &cell,
                    |p| top - drop * (p.y - z0) / span,
                    |_| seat,
                    &look,
                    [false, true],
                );
            }
            x += w;
            i += 1;
        }
        z = z1;
        row += 1;
    }
}

// ---------------------------------------------------------------------------
// Floor frames
// ---------------------------------------------------------------------------

/// A floor frame — the rails round its opening, square or half — in `tier`,
/// baked at the piece's root (walk surface `y = 0`, rails `SLAB_T` deep):
/// lashed poles round lashed corner posts, a butted timber frame, a ring of
/// dressed blocks, a steel ring under a tread plate.
pub fn floor_frame_mesh(tri: bool, tier: u8) -> Mesh {
    let (top, bot) = (0.0, -SLAB_T);
    let r = FRAME_RIM_M;
    let fp = footprint(tri);
    let mut o = Out::new(bot, top);
    match tier {
        MAT_TWIG => {
            let pr = r * 0.5 - 0.004;
            for (k, y) in [top - pr, bot + pr].into_iter().enumerate() {
                o.rim_poles(&fp, y, pr, 300 + k as u32 * 8);
            }
            for (i, p) in inset(&fp, pr).into_iter().enumerate() {
                o.pole(
                    Vec3::new(p.x, bot, p.y),
                    Vec3::new(p.x, top, p.y),
                    pr - 0.006,
                    Vec3::X,
                    &Look::new(Vec3::Y, 0.98).vary(320 + i as u32, 0.04),
                );
                let rope = Look::new(Vec3::X, 0.62).across(3.0);
                for y in [top - pr, bot + pr] {
                    o.pole(
                        Vec3::new(p.x, y - 0.045, p.y),
                        Vec3::new(p.x, y + 0.045, p.y),
                        pr - 0.001,
                        Vec3::X,
                        &rope,
                    );
                }
            }
        }
        MAT_WOOD if !tri => {
            for s in [-1.0f32, 1.0] {
                let (a, b) = (s * H, s * (H - r));
                o.beam(
                    Vec3::new(a.min(b), bot, -H),
                    Vec3::new(a.max(b), top, H),
                    2,
                    0.015,
                    &timber(Vec3::Z),
                );
                o.beam(
                    Vec3::new(-H + r, bot, a.min(b)),
                    Vec3::new(H - r, top, a.max(b)),
                    0,
                    0.015,
                    &timber(Vec3::X),
                );
            }
        }
        MAT_WOOD => o.rim_plates(&fp, r, bot, top, timber),
        MAT_STONE if !tri => {
            for s in [-1.0f32, 1.0] {
                let (a, b) = (s * H, s * (H - r));
                let (lo, hi) = (a.min(b), a.max(b));
                for (j, z) in joint_lines(-H, H, 0.75, -H, 0).windows(2).enumerate() {
                    o.block(
                        Vec3::new(lo, bot, z[0]),
                        Vec3::new(hi, top, z[1]),
                        JOINT_M,
                        &stone(400 + j as u32 + (s > 0.0) as u32 * 16),
                    );
                }
                for (j, x) in joint_lines(-H + r, H - r, 0.75, -H, 1)
                    .windows(2)
                    .enumerate()
                {
                    o.block(
                        Vec3::new(x[0], bot, lo),
                        Vec3::new(x[1], top, hi),
                        JOINT_M,
                        &stone(440 + j as u32 + (s > 0.0) as u32 * 16),
                    );
                }
            }
        }
        MAT_STONE => {
            // Each mitred rail cut into stones along its length, a chamfered
            // face on each and the joints between them.
            let inner = inset(&fp, r);
            for i in 0..fp.len() {
                let j = (i + 1) % fp.len();
                let len = fp[i].distance(fp[j]);
                let cuts = ((len / 0.8).round().max(1.0)) as usize;
                for c in 0..cuts {
                    let (t0, t1) = (c as f32 / cuts as f32, (c + 1) as f32 / cuts as f32);
                    let seg = [
                        fp[i].lerp(fp[j], t0),
                        fp[i].lerp(fp[j], t1),
                        inner[i].lerp(inner[j], t1),
                        inner[i].lerp(inner[j], t0),
                    ];
                    let look = stone(460 + (i * 8 + c) as u32);
                    o.pillow_poly(&seg, top, JOINT_M, JOINT_M, &look);
                    o.slab(&seg, bot, top - JOINT_M, &look, [true, false]);
                }
            }
        }
        _ => {
            const T: f32 = 0.012;
            if tri {
                o.rim_plates(&fp, r, bot, top - T, steel);
            } else {
                for s in [-1.0f32, 1.0] {
                    let (a, b) = (s * H, s * (H - r));
                    o.beam(
                        Vec3::new(a.min(b), bot, -H),
                        Vec3::new(a.max(b), top - T, H),
                        2,
                        0.01,
                        &steel(Vec3::Z),
                    );
                    o.beam(
                        Vec3::new(-H + r, bot, a.min(b)),
                        Vec3::new(H - r, top - T, a.max(b)),
                        0,
                        0.01,
                        &steel(Vec3::X),
                    );
                }
            }
            o.rim_plates(&fp, r, top - T, top, |d| sheet(d.cross(Vec3::Y), 95));
        }
    }
    finish(o.b)
}

// ---------------------------------------------------------------------------
// Risers: stairs, ramps, the turned and spiral flights, foundation steps
// ---------------------------------------------------------------------------

/// A riser dressed in `tier` over the walk patches the sim stands players on
/// (`sim_core::circulation::patches`), in the cell frame at the piece's root.
///
/// Each climbing patch is a [`Run`] drawn in its own frame and turned into
/// place; each level patch is a landing. The treads sit where the plain
/// flight's do — within half a riser of the walk surface — and everything
/// else hangs inside the slab the sim blocks under it ([`SLAB_T`] below the
/// surface, or down to the skirt for foundation steps), which
/// `tests/pieces.rs` §E holds vertex by vertex.
pub fn riser_mesh(shape: u8, tier: u8) -> Mesh {
    let (patches, n) = sim_core::circulation::patches(shape);
    let deep = (shape == SHAPE_FOUNDATION_STEPS).then_some(-SKIRT_MAX_M);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in &patches[..n] {
        let far = p.height(p.x1, p.z1);
        lo = lo.min(p.y.min(far) - SLAB_T);
        hi = hi.max(p.y.max(far));
    }
    let mut o = Out::new(lo, hi);
    let tri = shape == SHAPE_STAIRS_TRI_SPIRAL;
    for (i, p) in patches[..n].iter().enumerate() {
        let seed = (tier as u32) << 12 | (shape as u32) << 4 | i as u32;
        let (x0, x1, z0, z1) = (p.x0 - H, p.x1 - H, p.z0 - H, p.z1 - H);
        let (rx, rz) = (p.sx * (p.x1 - p.x0), p.sz * (p.z1 - p.z0));
        if rx == 0.0 && rz == 0.0 {
            let mut poly = rect(x0, x1, z0, z1);
            if tri {
                poly = tidy(&clip_half(&poly, Vec2::ONE, 0.0));
            }
            let y = if p.y.abs() < 1e-6 { p.y - SINK_M } else { p.y };
            landing(&mut o, &poly, y, p.y - SLAB_T, tier, seed);
            continue;
        }
        let (cx, cz) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
        // Which way it climbs, as the quarter-turn that carries the run's
        // own frame (+z up the slope) onto the cell.
        let (len, w, rise, m, off) = if rz > 0.0 {
            (
                z1 - z0,
                x1 - x0,
                rz,
                [[1.0, 0.0], [0.0, 1.0]],
                Vec3::new(cx, 0.0, z0),
            )
        } else if rz < 0.0 {
            (
                z1 - z0,
                x1 - x0,
                -rz,
                [[-1.0, 0.0], [0.0, -1.0]],
                Vec3::new(cx, 0.0, z1),
            )
        } else if rx > 0.0 {
            (
                x1 - x0,
                z1 - z0,
                rx,
                [[0.0, 1.0], [-1.0, 0.0]],
                Vec3::new(x0, 0.0, cz),
            )
        } else {
            (
                x1 - x0,
                z1 - z0,
                -rx,
                [[0.0, -1.0], [1.0, 0.0]],
                Vec3::new(x1, 0.0, cz),
            )
        };
        let steps = if shape == SHAPE_RAMP {
            0
        } else {
            (rise / LEVEL_H_M * STAIR_RISERS as f32).ceil() as usize
        };
        let run = Run {
            len,
            w,
            y0: p.y.min(p.height(p.x1, p.z1)),
            rise,
            steps,
            deep,
        };
        let start = o.mark();
        match tier {
            MAT_TWIG => run_twig(&mut o, &run, seed),
            MAT_WOOD => run_wood(&mut o, &run, seed),
            MAT_STONE => run_stone(&mut o, &run, seed),
            _ => run_metal(&mut o, &run, seed),
        }
        o.place_since(start, m, off);
    }
    finish(o.b)
}

/// How far a riser's tread or landing that lies ON the storey base sinks
/// under it: the floor beneath already draws that surface, and two faces on
/// one plane would z-fight across it.
const SINK_M: f32 = 0.006;

/// One climbing walk patch in its own frame: `z` from 0 at its foot to
/// `len` at its head, `x` across `±w/2`, the surface rising `rise` from `y0`
/// in `steps` treads (0 for a smooth ramp). `deep` is the floor a foundation
/// flight stands solid on.
#[derive(Clone, Copy, Debug)]
struct Run {
    len: f32,
    w: f32,
    y0: f32,
    rise: f32,
    steps: usize,
    deep: Option<f32>,
}

impl Run {
    /// The walk surface at `z`.
    fn h(&self, z: f32) -> f32 {
        self.y0 + self.rise * z / self.len
    }

    /// Tread `k`: its `z` span and its height — the plain flight's treads,
    /// half-depth at the two ends, each centred on the walk surface. A foot
    /// tread on the storey base sinks [`SINK_M`] under the floor it lies on.
    fn tread(&self, k: usize) -> (f32, f32, f32) {
        let n = self.steps as f32;
        let a = ((k as f32 - 0.5) / n).max(0.0) * self.len;
        let b = ((k as f32 + 0.5) / n).min(1.0) * self.len;
        let y = self.y0 + self.rise * k as f32 / n;
        (a, b, if y.abs() < 1e-6 { y - SINK_M } else { y })
    }

    fn slen(&self) -> f32 {
        (self.len * self.len + self.rise * self.rise).sqrt()
    }

    /// Up the slope, and out of it.
    fn dir(&self) -> Vec3 {
        Vec3::new(0.0, self.rise, self.len) / self.slen()
    }

    fn nrm(&self) -> Vec3 {
        Vec3::new(0.0, self.len, -self.rise) / self.slen()
    }

    /// The surface point `s` metres up the slope.
    fn at(&self, s: f32) -> Vec3 {
        Vec3::new(0.0, self.y0, 0.0) + self.dir() * s
    }

    /// A side-on outline in `(z, y)` running `top` and `bot` metres off the
    /// walk line, its top held under the run's head so a stringer never
    /// stands above the floor it delivers you to.
    fn outline(&self, top: f32, bot: f32) -> Vec<Vec2> {
        let cap = self.y0 + self.rise;
        let t = |z: f32| self.h(z) + top;
        let mut v = vec![
            Vec2::new(0.0, self.h(0.0) + bot),
            Vec2::new(self.len, self.h(self.len) + bot),
        ];
        if t(self.len) > cap + 1e-5 && t(0.0) < cap - 1e-5 {
            v.push(Vec2::new(self.len, cap));
            v.push(Vec2::new((cap - top - self.y0) * self.len / self.rise, cap));
        } else {
            v.push(Vec2::new(self.len, t(self.len).min(cap)));
        }
        v.push(Vec2::new(0.0, t(0.0).min(cap)));
        v
    }

    /// A foundation flight's body under its stringers, down to the floor it
    /// stands on, set `set` in from the run's sides.
    fn body(&self, o: &mut Out, set: f32, look: &Look) {
        let Some(floor) = self.deep else {
            return;
        };
        let poly = [
            Vec2::new(0.0, floor),
            Vec2::new(self.len, floor),
            Vec2::new(self.len, self.h(self.len) - 0.2),
            Vec2::new(0.0, self.h(0.0) - 0.2),
        ];
        let hw = self.w * 0.5 - set;
        o.sheet_x(&poly, -hw, hw, look);
    }
}

/// Twig: round treads lashed across two pole stringers over a stick mat; a
/// ramp is poles laid side by side up the slope.
fn run_twig(o: &mut Out, r: &Run, seed: u32) {
    const RS: f32 = 0.06;
    const RP: f32 = 0.06;
    let hw = r.w * 0.5;
    let nrm = r.nrm();
    let drop = if r.steps > 0 {
        0.15
    } else {
        (2.0 * RP + RS) / nrm.y
    };
    for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let x = side * (hw - RS);
        let (a, b) = (RS * 1.4, r.len - RS * 1.4);
        o.pole(
            Vec3::new(x, r.h(a) - drop, a),
            Vec3::new(x, r.h(b) - drop, b),
            RS,
            Vec3::X,
            &twig_pole(seed ^ ((i as u32 + 1) * 977)),
        );
    }
    if r.steps > 0 {
        for k in 0..=r.steps {
            let (za, zb, y) = r.tread(k);
            // The half-depth treads at the two ends are the floors the
            // flight runs between; a pole there would hang off nothing.
            let rt = 0.065f32.min((zb - za) * 0.5);
            if rt < 0.05 {
                continue;
            }
            let zc = (za + zb) * 0.5;
            o.pole(
                Vec3::new(-hw, y - rt, zc),
                Vec3::new(hw, y - rt, zc),
                rt,
                Vec3::Y,
                &twig_pole(seed.wrapping_mul(31).wrapping_add(k as u32)),
            );
        }
    } else {
        // A pole's corners reach 1.08 RP from its axis; these are the
        // first and last axes whose corners stay over the run.
        let (sin, cos) = (-nrm.z, nrm.y);
        let s0 = RP * (1.08 - sin) / cos;
        let s1 = r.slen() - RP * (1.08 + sin) / cos;
        let n = ((s1 - s0) / (2.0 * RP + 0.004)).floor().max(0.0) as usize + 1;
        let pitch = (s1 - s0) / (n.max(2) - 1) as f32;
        for i in 0..n {
            let c = r.at(s0 + pitch * i as f32) - nrm * RP;
            o.pole(
                c - Vec3::X * hw,
                c + Vec3::X * hw,
                RP,
                nrm,
                &twig_pole(seed.wrapping_mul(31).wrapping_add(i as u32)),
            );
        }
    }
    // A stick mat under the poles closes the flight from below.
    let mat = if r.steps > 0 {
        -0.135
    } else {
        -(2.0 * RP + 0.005) / nrm.y
    };
    o.sheet_x(
        &r.outline(mat, mat - 0.05),
        -(hw - 0.1),
        hw - 0.1,
        &twig_mat(),
    );
    r.body(o, 0.05, &twig_mat());
}

/// Wood: plank treads and set-back riser boards housed in two closed timber
/// stringers, a carriage under a wide flight; a ramp is boards across the
/// slope on the stringers.
fn run_wood(o: &mut Out, r: &Run, seed: u32) {
    const SW: f32 = 0.07;
    const T: f32 = 0.045;
    let hw = r.w * 0.5;
    let inner = hw - SW;
    let grain = r.dir();
    let under = if r.steps > 0 {
        for k in 0..=r.steps {
            let (za, zb, y) = r.tread(k);
            if zb - za < 0.03 {
                continue;
            }
            // Each tread runs on under the next riser, which stands on it.
            let tail = if k < r.steps { 0.045 } else { 0.0 };
            o.beam(
                Vec3::new(-inner, y - T, za),
                Vec3::new(inner, y, (zb + tail).min(r.len)),
                0,
                0.012,
                &planks().vary(seed.wrapping_add(k as u32 * 7), 0.06),
            );
            if k > 0 {
                let below = r.tread(k - 1).2;
                let zr = za + 0.02;
                if y - T > below + 0.02 {
                    o.cuboid(
                        Vec3::new(-inner, below, zr),
                        Vec3::new(inner, y - T, zr + 0.022),
                        &planks()
                            .grain(Vec3::X)
                            .vary(seed.wrapping_add(101 + k as u32), 0.05),
                        0,
                    );
                }
            }
        }
        (0.04, -0.135)
    } else {
        let (dir, nrm) = (r.dir(), r.nrm());
        // Stopped short of the head by what a board's thickness overhangs it.
        let run = r.slen() - T * (-nrm.z / nrm.y);
        let n = (run / 0.146).floor().max(1.0);
        let pitch = run / n;
        for i in 0..n as usize {
            let c = r.at(pitch * (i as f32 + 0.5)) - nrm * (T * 0.5);
            o.extrude(
                c,
                nrm,
                dir,
                &chamfer_rect(T * 0.5, pitch * 0.5 - 0.003, 0.008),
                [-hw, hw],
                &planks()
                    .grain(Vec3::X)
                    .vary(seed.wrapping_add(i as u32 * 7), 0.06),
                [true, true],
                false,
            );
        }
        let u = -(T / nrm.y) - 0.002;
        (u, u)
    };
    for side in [-1.0f32, 1.0] {
        o.sheet_x(
            &r.outline(under.0, -0.27),
            side * inner,
            side * hw,
            &timber(grain),
        );
    }
    if r.w > 1.5 {
        o.sheet_x(&r.outline(under.1, -0.29), -0.045, 0.045, &timber(grain));
    }
    r.body(o, 0.035, &planks());
}

/// Stone: one block per tread, two across a wide flight with the joint
/// staggered, the soffit stepping down under them; a ramp is flags laid on
/// the slope over a sloped core.
fn run_stone(o: &mut Out, r: &Run, seed: u32) {
    let hw = r.w * 0.5;
    if r.steps > 0 {
        for k in 0..=r.steps {
            let (za, zb, y) = r.tread(k);
            if zb - za < 0.03 {
                continue;
            }
            let bot = match r.deep {
                Some(_) => y - SLAB_T,
                None => r.h(zb) - SLAB_T + 0.003,
            }
            .min(y - 0.06);
            let cuts = if r.w > 1.5 {
                let j = if k % 2 == 0 { 0.3 } else { -0.35 } * hw;
                vec![-hw, j, hw]
            } else {
                vec![-hw, hw]
            };
            for (j, x) in cuts.windows(2).enumerate() {
                o.block(
                    Vec3::new(x[0], bot, za),
                    Vec3::new(x[1], y, zb),
                    JOINT_M,
                    &stone(seed.wrapping_mul(131).wrapping_add((k * 4 + j) as u32)),
                );
            }
        }
    } else {
        let (dir, nrm) = (r.dir(), r.nrm());
        let run = r.slen() - JOINT_M * (-nrm.z / nrm.y);
        for (i, s) in joint_lines(0.0, run, 0.6, 0.0, 0).windows(2).enumerate() {
            for (j, x) in joint_lines(-hw, hw, 0.75, -hw, i as i32)
                .windows(2)
                .enumerate()
            {
                o.pillow(
                    r.at(0.0),
                    nrm,
                    (Vec3::X, dir),
                    [x[0], x[1], s[0], s[1]],
                    [true; 4],
                    [JOINT_M; 4],
                    JOINT_M,
                    &stone(seed.wrapping_mul(131).wrapping_add((i * 8 + j) as u32)).grain(dir),
                );
            }
        }
        o.sheet_x(
            &r.outline(-JOINT_M / nrm.y, -SLAB_T + 0.003),
            -hw,
            hw,
            &stone(seed ^ 0x55),
        );
    }
    r.body(o, 0.01, &stone(seed ^ 0x77));
}

/// Metal: tread plates with a turned-down nosing and kick plates between two
/// steel channels; a ramp is a ribbed plate on cross purlins.
fn run_metal(o: &mut Out, r: &Run, seed: u32) {
    const SW: f32 = 0.035;
    const LIP: f32 = 0.06;
    const T: f32 = 0.012;
    const NOSE: f32 = 0.045;
    let hw = r.w * 0.5;
    let inner = hw - SW;
    let grain = r.dir();
    let top = if r.steps > 0 {
        for k in 0..=r.steps {
            let (za, zb, y) = r.tread(k);
            if zb - za < 0.03 {
                continue;
            }
            let tail = if k < r.steps { 0.04 } else { 0.0 };
            o.cuboid(
                Vec3::new(-inner, y - T, za),
                Vec3::new(inner, y, (zb + tail).min(r.len)),
                &sheet(Vec3::X, seed.wrapping_add(k as u32)),
                0,
            );
            o.cuboid(
                Vec3::new(-inner, y - NOSE, za),
                Vec3::new(inner, y - T, za + T),
                &steel(Vec3::X),
                0,
            );
            if k > 0 {
                let below = r.tread(k - 1).2;
                let zr = za + 0.03;
                if y - T > below + 0.02 {
                    o.cuboid(
                        Vec3::new(-inner, below, zr),
                        Vec3::new(inner, y - T, zr + 0.008),
                        &sheet(Vec3::Y, seed.wrapping_add(50 + k as u32)),
                        0,
                    );
                }
            }
        }
        0.03
    } else {
        let (dir, nrm) = (r.dir(), r.nrm());
        let tv = T / nrm.y;
        o.sheet_x(&r.outline(0.0, -tv), -hw, hw, &sheet(Vec3::X, seed));
        let n = (r.slen() / 0.7).round().max(2.0) as usize;
        for i in 0..n {
            let c = r.at(r.slen() * (i as f32 + 0.5) / n as f32) - nrm * (T + 0.04);
            o.extrude(
                c,
                nrm,
                dir,
                &chamfer_rect(0.04, 0.03, 0.006),
                [-inner, inner],
                &steel(Vec3::X),
                [true, true],
                false,
            );
        }
        -tv
    };
    for side in [-1.0f32, 1.0] {
        o.sheet_x(
            &r.outline(top, -0.22),
            side * inner,
            side * hw,
            &steel(grain),
        );
        o.sheet_x(
            &r.outline(-0.2, -0.22),
            side * (inner - LIP),
            side * inner,
            &steel(grain),
        );
    }
    r.body(o, 0.04, &sheet(Vec3::Y, seed ^ 0x33));
}

/// A riser's level patch at `y` over the convex `poly`: the tier's deck on
/// its rim, down to `bot`, the floor of the slab the sim blocks under it.
fn landing(o: &mut Out, poly: &[Vec2], y: f32, bot: f32, tier: u8, seed: u32) {
    match tier {
        MAT_TWIG => {
            const DECK: f32 = 0.07;
            const R: f32 = 0.075;
            o.slab(poly, y - DECK, y, &Look::new(Vec3::X, 1.0), [true, true]);
            o.rim_poles(poly, y - DECK - R, R, seed);
            o.slab(
                &inset(poly, 0.06),
                bot + 0.02,
                y - DECK,
                &twig_mat(),
                [true, false],
            );
        }
        MAT_WOOD => {
            const T: f32 = 0.05;
            let (z0, z1) = poly
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
            let (x0, x1) = poly
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
            let n = ((z1 - z0) / 0.15).round().max(1.0) as usize;
            let pitch = (z1 - z0) / n as f32;
            for i in 0..n {
                let (a, b) = (z0 + pitch * i as f32, z0 + pitch * (i + 1) as f32);
                let (a, b) = (
                    if i > 0 { a + 0.003 } else { a },
                    if i + 1 < n { b - 0.003 } else { b },
                );
                let board = clip_to(&rect(x0, x1, a, b), poly);
                o.slab(
                    &board,
                    y - T,
                    y,
                    &planks()
                        .grain(Vec3::X)
                        .vary(seed.wrapping_mul(17).wrapping_add(i as u32), 0.06),
                    [true, true],
                );
            }
            o.rim_plates(poly, 0.1, bot + 0.04, y - T, timber);
        }
        MAT_STONE => {
            let (z0, z1) = poly
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
            let (x0, x1) = poly
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
            for (r, zw) in joint_lines(z0, z1, 0.6, z0, 0).windows(2).enumerate() {
                for (c, xw) in joint_lines(x0, x1, 0.6, x0, r as i32)
                    .windows(2)
                    .enumerate()
                {
                    let cell = clip_to(&rect(xw[0], xw[1], zw[0], zw[1]), poly);
                    if cell.len() >= 3 {
                        o.pillow_poly(
                            &cell,
                            y,
                            JOINT_M,
                            JOINT_M,
                            &stone(seed.wrapping_mul(29).wrapping_add((r * 8 + c) as u32)),
                        );
                    }
                }
            }
            o.slab(poly, bot, y - JOINT_M, &stone(seed ^ 0x55), [true, true]);
        }
        _ => {
            const T: f32 = 0.03;
            o.slab(poly, y - T, y, &sheet(Vec3::X, seed), [true, true]);
            o.rim_plates(poly, 0.09, bot + 0.05, y - T, steel);
        }
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
