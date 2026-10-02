//! Where a blow meets what is actually DRAWN.
//!
//! The sim strikes an occupant's collision cylinder (`terrain::occupant_volume`
//! inflated by `melee::MELEE_PROBE_M`), and a cylinder is not a rock: on a
//! dome-shaped ore node the cap stands up to half a metre over the real
//! surface, and a trunk tapers inside its base radius. A mark laid where the
//! sim struck floated off the model it was meant to be on. So a world-surface
//! contact is moved onto the drawn mesh here, by ray against its triangles,
//! and a mark is then conformed to that surface vertex by vertex.
//!
//! It decides nothing: the sim said a blow landed and on what; this only
//! answers where on the picture of it.

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;

/// A slot part a mark can land on: the trunk of a tree, a node, a rock, a
/// barrel, a crate. `tree` marks a trunk, whose marks bend about its axis.
#[derive(Component, Clone, Copy, Debug)]
pub struct Skin {
    pub tree: bool,
}

/// A ray's hit on a drawn mesh, world space.
#[derive(Clone, Copy, Debug)]
pub struct SkinHit {
    /// Distance along the (normalised) ray, metres.
    pub t: f32,
    pub at: Vec3,
    /// The surface's normal there, facing back along the ray.
    pub normal: Vec3,
}

/// How far a contact may move onto the drawn surface and still be the same
/// blow, metres. The worst honest gap is the cylinder's cap over the shoulder
/// of a dome-shaped node; a hit further than this is some other surface.
pub const SNAP_MAX_M: f32 = 1.0;

/// How far from a contact a candidate's origin may stand, planar metres —
/// the largest drawn prop's half-width at the largest slot scale, rounded up.
pub const SNAP_REACH_M: f32 = 3.0;

/// The nearest hit of the ray `o + d·t` (`d` normalised, `t` in
/// `(0, max_t]`) on `mesh` drawn at `tf`. Two-sided; the normal is the
/// mesh's own, interpolated, where it has one.
pub fn ray_mesh(
    mesh: &Mesh,
    tf: &GlobalTransform,
    o: Vec3,
    d: Vec3,
    max_t: f32,
) -> Option<SkinHit> {
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let nrm = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(n)) if n.len() == pos.len() => Some(n),
        _ => None,
    };
    let world = tf.affine();
    let inv = world.inverse();
    let lo = inv.transform_point3(o);
    let ld = inv.transform_vector3(d);
    let mut best: Option<(f32, usize, usize, usize, f32, f32)> = None;
    let mut test = |a: usize, b: usize, c: usize| {
        if a >= pos.len() || b >= pos.len() || c >= pos.len() {
            return;
        }
        let (p0, p1, p2) = (
            Vec3::from_array(pos[a]),
            Vec3::from_array(pos[b]),
            Vec3::from_array(pos[c]),
        );
        // Möller–Trumbore.
        let (e1, e2) = (p1 - p0, p2 - p0);
        let h = ld.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-12 {
            return;
        }
        let f = 1.0 / det;
        let s = lo - p0;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            return;
        }
        let q = s.cross(e1);
        let v = f * ld.dot(q);
        if v < 0.0 || u + v > 1.0 {
            return;
        }
        let t = f * e2.dot(q);
        if t <= 0.0 || t > max_t || best.is_some_and(|b| t >= b.0) {
            return;
        }
        best = Some((t, a, b, c, u, v));
    };
    match mesh.indices() {
        Some(Indices::U32(ix)) => {
            for tri in ix.chunks_exact(3) {
                test(tri[0] as usize, tri[1] as usize, tri[2] as usize);
            }
        }
        Some(Indices::U16(ix)) => {
            for tri in ix.chunks_exact(3) {
                test(tri[0] as usize, tri[1] as usize, tri[2] as usize);
            }
        }
        None => {
            for i in (0..pos.len().saturating_sub(2)).step_by(3) {
                test(i, i + 1, i + 2);
            }
        }
    }
    let (t, a, b, c, u, v) = best?;
    let (p0, p1, p2) = (
        Vec3::from_array(pos[a]),
        Vec3::from_array(pos[b]),
        Vec3::from_array(pos[c]),
    );
    let face = (p1 - p0).cross(p2 - p0);
    let local_n = match nrm {
        Some(n) => {
            let w = 1.0 - u - v;
            let s = Vec3::from_array(n[a]) * w
                + Vec3::from_array(n[b]) * u
                + Vec3::from_array(n[c]) * v;
            // A smoothed normal can lean past its face at a crease; the face
            // says which side is out.
            if s.dot(face) < 0.0 {
                -s
            } else {
                s
            }
        }
        None => face,
    };
    // Normals go by the inverse transpose, so a squashed slot stays right.
    let m = Mat3::from(world.matrix3);
    let mut n = (m.inverse().transpose() * local_n).normalize_or(-d);
    if n.dot(d) > 0.0 {
        n = -n;
    }
    Some(SkinHit {
        t,
        at: o + d * t,
        normal: n,
    })
}

/// Where a candidate's surface is aimed at from a contact: a trunk at its
/// axis at the contact's own height, anything else at its origin — every
/// prop mesh is centred on its own, so a ray toward it crosses the skin.
pub fn core_of(skin: &Skin, tf: &GlobalTransform, at: Vec3) -> Vec3 {
    let o = tf.translation();
    if skin.tree {
        Vec3::new(o.x, at.y.max(o.y + 0.1), o.z)
    } else {
        o
    }
}

/// Move a world-surface contact at `at` onto the nearest drawn candidate.
///
/// `eye` is the swinger's eye when the blow is the local player's own swing:
/// the mark goes where they were looking at the picture, if that ray meets a
/// candidate near the sim's point. Otherwise each candidate is aimed at from
/// just outside the contact toward its core. Returns the entity, the point and
/// the normal, or `None` when nothing drawn is near enough to be this blow.
pub fn snap<'a>(
    at: Vec3,
    eye: Option<Vec3>,
    candidates: impl Iterator<Item = (Entity, &'a Skin, &'a Mesh, &'a GlobalTransform)> + Clone,
) -> Option<(Entity, SkinHit)> {
    let near = |tf: &GlobalTransform| {
        let p = tf.translation();
        (p.x - at.x).powi(2) + (p.z - at.z).powi(2) <= SNAP_REACH_M * SNAP_REACH_M
    };
    if let Some(eye) = eye {
        let to = at - eye;
        let len = to.length();
        if len > 1e-3 {
            let d = to / len;
            let mut best: Option<(Entity, SkinHit)> = None;
            for (e, _, mesh, tf) in candidates.clone().filter(|c| near(c.3)) {
                if let Some(h) = ray_mesh(mesh, tf, eye, d, len + SNAP_MAX_M) {
                    if best.is_none_or(|b| h.t < b.1.t) {
                        best = Some((e, h));
                    }
                }
            }
            if let Some(b) = best.filter(|b| b.1.at.distance(at) <= SNAP_MAX_M) {
                return Some(b);
            }
        }
    }
    let mut best: Option<(Entity, SkinHit, f32)> = None;
    for (e, skin, mesh, tf) in candidates.filter(|c| near(c.3)) {
        let core = core_of(skin, tf, at);
        let to = core - at;
        let len = to.length();
        if len < 1e-3 {
            continue;
        }
        let d = to / len;
        // From a little outside, so a contact the drawn skin already covers
        // (a model a hair wider than its cylinder) still finds the outside.
        let o = at - d * 0.3;
        let Some(h) = ray_mesh(mesh, tf, o, d, len + 0.3) else {
            continue;
        };
        let gap = h.at.distance(at);
        if gap <= SNAP_MAX_M && best.is_none_or(|b| gap < b.2) {
            best = Some((e, h, gap));
        }
    }
    best.map(|(e, h, _)| (e, h))
}

/// The triangles of one drawn mesh near a point, gathered once so a mark's
/// [`MARK_GRID`](super::decal::MARK_GRID)² rays test a handful of triangles
/// and not a whole trunk. Reused: its buffer keeps its capacity.
#[derive(Default)]
pub struct Patch {
    /// Local-space corners and their normals (the face's where the mesh has
    /// none).
    tris: Vec<([Vec3; 3], [Vec3; 3])>,
    world: bevy::math::Affine3A,
    inv: bevy::math::Affine3A,
    normal_m: Mat3,
}

impl Patch {
    /// Gather every triangle of `mesh` drawn at `tf` with a corner within
    /// `radius` of `centre` (world), or whose bounds straddle it.
    pub fn gather(&mut self, mesh: &Mesh, tf: &GlobalTransform, centre: Vec3, radius: f32) {
        self.tris.clear();
        self.world = tf.affine();
        self.inv = self.world.inverse();
        self.normal_m = Mat3::from(self.world.matrix3).inverse().transpose();
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return;
        };
        let nrm = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(n)) if n.len() == pos.len() => Some(n),
            _ => None,
        };
        // The sphere in local space: a slot is scaled uniformly, so one
        // radius scale is exact.
        let lc = self.inv.transform_point3(centre);
        let lr = radius * self.inv.transform_vector3(Vec3::X).length();
        let mut take = |a: usize, b: usize, c: usize| {
            if a >= pos.len() || b >= pos.len() || c >= pos.len() {
                return;
            }
            let p = [
                Vec3::from_array(pos[a]),
                Vec3::from_array(pos[b]),
                Vec3::from_array(pos[c]),
            ];
            let lo = p[0].min(p[1]).min(p[2]);
            let hi = p[0].max(p[1]).max(p[2]);
            if lc.clamp(lo, hi).distance_squared(lc) > lr * lr {
                return;
            }
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            let n = match nrm {
                Some(n) => [
                    Vec3::from_array(n[a]),
                    Vec3::from_array(n[b]),
                    Vec3::from_array(n[c]),
                ],
                None => [face; 3],
            };
            self.tris.push((p, n));
        };
        match mesh.indices() {
            Some(Indices::U32(ix)) => {
                for t in ix.chunks_exact(3) {
                    take(t[0] as usize, t[1] as usize, t[2] as usize);
                }
            }
            Some(Indices::U16(ix)) => {
                for t in ix.chunks_exact(3) {
                    take(t[0] as usize, t[1] as usize, t[2] as usize);
                }
            }
            None => {
                for i in (0..pos.len().saturating_sub(2)).step_by(3) {
                    take(i, i + 1, i + 2);
                }
            }
        }
    }

    /// The nearest hit of `o + d·t` (world, `d` normalised, `t ≤ max_t`) on
    /// the gathered triangles: the point and the normal facing the ray.
    pub fn cast(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<(Vec3, Vec3)> {
        let lo = self.inv.transform_point3(o);
        let ld = self.inv.transform_vector3(d);
        let mut best: Option<(f32, usize, f32, f32)> = None;
        for (i, (p, _)) in self.tris.iter().enumerate() {
            let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
            let h = ld.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-12 {
                continue;
            }
            let f = 1.0 / det;
            let s = lo - p[0];
            let u = f * s.dot(h);
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = f * ld.dot(q);
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = f * e2.dot(q);
            if t <= 0.0 || t > max_t || best.is_some_and(|b| t >= b.0) {
                continue;
            }
            best = Some((t, i, u, v));
        }
        let (t, i, u, v) = best?;
        let (p, n) = &self.tris[i];
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let mut s = n[0] * (1.0 - u - v) + n[1] * u + n[2] * v;
        if s.dot(face) < 0.0 {
            s = -s;
        }
        let mut wn = (self.normal_m * s).normalize_or(-d);
        if wn.dot(d) > 0.0 {
            wn = -wn;
        }
        Some((o + d * t, wn))
    }
}
