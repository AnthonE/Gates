//! Shared held models resting in the world, with a tied canvas pouch for
//! items without a model. Placement is visual only; pickup uses the wire.

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;

use super::props::Soup;

/// Height of the near terrain's actual triangles. Small loot can disappear
/// under the chord between metre-spaced vertices even when its server height
/// is correct for the continuous heightfield.
pub fn surface_y(seed: u64, haven: &sim_core::terrain::Haven, x: f32, z: f32) -> f32 {
    let step = super::terrain_mesh::CHUNK_M / (super::terrain_mesh::NEAR_N - 1) as f32;
    let ox = (x / step).floor() * step;
    let oz = (z / step).floor() * step;
    let u = (x - ox) / step;
    let v = (z - oz) / step;
    let h = |dx, dz| sim_core::terrain::ground(seed, haven, ox + dx * step, oz + dz * step);
    // heightfield's [a,c,b], [b,c,d] diagonal.
    if u + v <= 1.0 {
        h(0.0, 0.0) * (1.0 - u - v) + h(1.0, 0.0) * u + h(0.0, 1.0) * v
    } else {
        h(1.0, 1.0) * (u + v - 1.0) + h(1.0, 0.0) * (1.0 - v) + h(0.0, 1.0) * (1.0 - u)
    }
}

/// A folded sack, rooted at zero, inside the existing loose-stack envelope.
/// The pinched neck and flared mouth distinguish it from a death backpack.
pub fn sack_mesh(size: [f32; 3]) -> Mesh {
    let mut s = Soup::tiling(1.0);
    let profile = [
        (0.0, 0.55),
        (0.12, 0.90),
        (0.40, 1.0),
        (0.65, 0.80),
        (0.80, 0.26),
        (0.88, 0.25),
        (1.0, 0.52),
    ];
    let point = |row: usize, side: usize| {
        let (y, radius) = profile[row];
        let a = side as f32 * std::f32::consts::TAU / 12.0;
        let fold = if side.is_multiple_of(2) { 1.0 } else { 0.88 };
        Vec3::new(
            a.cos() * radius * fold * size[0] * 0.5,
            (y - if row == profile.len() - 1 && !side.is_multiple_of(2) {
                0.08
            } else {
                0.0
            }) * size[1],
            a.sin() * radius * fold * size[2] * 0.5,
        )
    };
    for row in 0..profile.len() - 1 {
        for side in 0..12 {
            let a = point(row, side);
            let b = point(row + 1, side);
            let c = point(row + 1, side + 1);
            let d = point(row, side + 1);
            // Vertex value follows the fold; the material supplies canvas.
            let value = if row == 4 { 0.55 } else { 0.92 };
            let color = |_| [value, value, value, 1.0];
            let center = Some(Vec3::Y * size[1] * 0.4);
            let soften = if row < 3 { 0.5 } else { 0.0 };
            s.tri(a, b, c, color, center, soften);
            s.tri(a, c, d, color, center, soften);
        }
    }
    for side in 0..12 {
        s.tri(
            Vec3::ZERO,
            point(0, side),
            point(0, side + 1),
            |_| [0.7, 0.7, 0.7, 1.0],
            None,
            0.0,
        );
        let top = profile.len() - 1;
        s.tri(
            Vec3::Y * size[1] * 0.88,
            point(top, side + 1),
            point(top, side),
            |_| [0.8, 0.8, 0.8, 1.0],
            None,
            0.0,
        );
    }
    s.mesh()
}

/// Lay a model on its side and seat its transformed bounds on the surface.
/// The hand's grip offset is deliberately absent: it has no meaning on dirt.
/// Sampling the footprint also covers wire-height quantization and slopes.
pub fn resting_transform(
    mesh: &Mesh,
    position: Vec3,
    id: u32,
    scale: f32,
    laid: bool,
    ground: impl Fn(f32, f32) -> f32,
) -> Transform {
    let yaw = Quat::from_rotation_y((id % 4) as f32 * std::f32::consts::FRAC_PI_2);
    let mut rotation = yaw
        * if laid {
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Transform::from_translation(position);
    };
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in positions {
        let p = rotation * (Vec3::from(*p) * scale);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let center = (lo + hi) * 0.5;
    // Follow the terrain's local plane, then lift only where curvature or
    // quantization puts a corner below it. A horizontal spear seated at the
    // highest corner of its world AABB would hover over a hillside.
    let radius = ((hi - lo).xz().length() * 0.5).max(f32::EPSILON);
    let dx = (ground(position.x + radius, position.z) - ground(position.x - radius, position.z))
        / (2.0 * radius);
    let dz = (ground(position.x, position.z + radius) - ground(position.x, position.z - radius))
        / (2.0 * radius);
    let tilt = Quat::from_rotation_arc(Vec3::Y, Vec3::new(-dx, 1.0, -dz).normalize());
    rotation = tilt * rotation;
    let mut translation = position - tilt * Vec3::new(center.x, lo.y, center.z);
    let mut lift = 0.0_f32;
    for x in [lo.x, center.x, hi.x] {
        for z in [lo.z, center.z, hi.z] {
            let p = translation + tilt * Vec3::new(x, lo.y, z);
            lift = lift.max(ground(p.x, p.z) - p.y);
        }
    }
    translation.y += lift;
    Transform {
        translation,
        rotation,
        scale: Vec3::splat(scale),
    }
}
