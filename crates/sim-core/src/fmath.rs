//! The whole float vocabulary of the sim (CLAUDE.md wall 1): + − × ÷ sqrt
//! min max clamp floor-by-cast, nothing else. Everything here composes only
//! those, so every caller stays inside the wall. libm/trig are clippy-banned.

/// True floor via cast-compare (an `as` cast truncates toward zero, which
/// is wrong for negatives on its own).
#[inline]
pub fn floor_i32(x: f32) -> i32 {
    let t = x as i32;
    if (t as f32) > x {
        t - 1
    } else {
        t
    }
}

/// |x| composed from the allowed set (`f32::abs` is outside the wall's
/// letter, so it stays banned; max(x, -x) is bit-exact anyway).
#[inline]
pub fn fabs(x: f32) -> f32 {
    x.max(-x)
}

/// a + (b − a)·t. Multiply-add as two ops — never fused; FMA contraction
/// would break native/wasm parity (DESIGN.md §4).
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Quintic fade 6t⁵ − 15t⁴ + 10t³ (TERRAIN.md §1: quintic smoothing).
#[inline]
pub fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
