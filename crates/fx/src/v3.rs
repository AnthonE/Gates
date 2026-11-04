//! A vector is `[f32; 3]`. These are glam's scalar `Vec3` operations in the
//! same order of arithmetic, so a pool driven through here moves exactly as
//! it did on `bevy::math::Vec3` (Bevy 0.18's glam 0.30 and 0.19's 0.32 alike).

pub type V3 = [f32; 3];

pub const ZERO: V3 = [0.0; 3];
pub const X: V3 = [1.0, 0.0, 0.0];
pub const Y: V3 = [0.0, 1.0, 0.0];
pub const Z: V3 = [0.0, 0.0, 1.0];

#[inline]
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

#[inline]
pub fn dot(a: V3, b: V3) -> f32 {
    (a[0] * b[0]) + (a[1] * b[1]) + (a[2] * b[2])
}

#[inline]
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - b[1] * a[2],
        a[2] * b[0] - b[2] * a[0],
        a[0] * b[1] - b[0] * a[1],
    ]
}

#[inline]
pub fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}

#[inline]
pub fn dist2(a: V3, b: V3) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

/// `a` at unit length, or `fallback` when it has none (glam's `normalize_or`).
#[inline]
pub fn norm_or(a: V3, fallback: V3) -> V3 {
    let rcp = 1.0 / len(a);
    if rcp.is_finite() && rcp > 0.0 {
        scale(a, rcp)
    } else {
        fallback
    }
}

#[inline]
pub fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add(a, scale(sub(b, a), t))
}
