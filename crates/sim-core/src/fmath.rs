//! The whole float vocabulary of the sim (CLAUDE.md wall 1): + − × ÷ sqrt
//! min max, plus the transcendentals below.
//!
//! **Trig and friends go through here, never through `f32::sin`.** The std
//! methods call the platform's libm, which differs between a native build
//! and wasm in the last bit, and those bits are what native/wasm parity is
//! about. These call the `libm` crate instead: pure Rust over IEEE `+ − × ÷`
//! and correctly rounded `sqrt`, compiled from the same source for every
//! target, so every build gets the same answer. `clippy.toml` still bans the
//! std methods; the `math` line of the parity probe (`probe::probe_math`)
//! checks these against wasm every night.

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

// The transcendentals, through `libm` (module doc). Thin on purpose: one
// place to read, and one place the parity probe sweeps.

#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// `(sin, cos)`, the order `f32::sin_cos` returns them in.
#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    libm::sincosf(x)
}

#[inline]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}

#[inline]
pub fn asin(x: f32) -> f32 {
    libm::asinf(x)
}

#[inline]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x)
}

#[inline]
pub fn atan(x: f32) -> f32 {
    libm::atanf(x)
}

/// The angle of `(x, y)` from +x, in `(−π, π]` — `f32::atan2`'s argument
/// order: `atan2(y, x)`.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

#[inline]
pub fn exp2(x: f32) -> f32 {
    libm::exp2f(x)
}

/// The natural log.
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

#[inline]
pub fn log2(x: f32) -> f32 {
    libm::log2f(x)
}

#[inline]
pub fn log10(x: f32) -> f32 {
    libm::log10f(x)
}

#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

#[inline]
pub fn hypot(x: f32, y: f32) -> f32 {
    libm::hypotf(x, y)
}

#[inline]
pub fn cbrt(x: f32) -> f32 {
    libm::cbrtf(x)
}

#[inline]
pub fn floor(x: f32) -> f32 {
    libm::floorf(x)
}

#[inline]
pub fn ceil(x: f32) -> f32 {
    libm::ceilf(x)
}

/// Half away from zero, as `f32::round`.
#[inline]
pub fn round(x: f32) -> f32 {
    libm::roundf(x)
}

#[inline]
pub fn trunc(x: f32) -> f32 {
    libm::truncf(x)
}
