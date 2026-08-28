#![cfg(feature = "render")]
// ^ everything about the client is behind `--features render`, and the
//   workspace clippy line builds `--all-targets` without it.
//! Gate: the ground's macro break-up is the scale it claims, and it does not
//! move the island's mean brightness.
//!
//! ## Why this exists
//!
//! The ground texture repeats every 4 m — `heightfield` writes a planar UV of
//! `world.xz × 0.25`, so one 1024² photograph covers 4 m and repeats **512
//! times per island side** on a rigid axis-aligned lattice, and
//! `ground_splat.wgsl` samples all sixteen maps at that one UV with nothing to
//! break it. `terrain_mesh::macro_noise` is what breaks it, and it has two
//! properties that a later edit could take away in silence:
//!
//! - **It is low-frequency.** A break-up at or near the tile's own period does
//!   not hide the tile, it beats against it. Nothing about the constant
//!   `MACRO_M = 48` forces the field to actually *vary* at 48 m rather than at
//!   4 m — that is a property of the interpolation, and this file measures it.
//! - **Its mean is exactly 1.** `fill::GROUND_MIX`, the bounce albedo and the
//!   island-weighted luma pin in `ground_mix.rs` are all folded from the splat
//!   weights and the authored identities; none of them reads this slot, so a
//!   field with a mean of 1.05 would brighten the whole island by 5% with
//!   **every one of those gates still green**. That is the shape of defect this
//!   repo's trap list is mostly made of.
//!
//! Both are measured over the world square, not over a window — `relief.rs`
//! and `ground_mix.rs` both carry the retraction that came from sweeping a
//! quadrant and calling it the island.

use client::render::terrain_mesh::{vertex_mods, GROUND_TILE_M, MACRO_AMP, MACRO_M};
use sim_core::terrain::ISLAND_SIZE;

/// The coarsest repeat the break-up has to hide, metres.
///
/// **Derived, because it used to be a bare `4.0` here.** Every identity shared
/// one 4 m tile until 2026-08-28; they now have four sizes
/// (`terrain_mesh::GROUND_TILE_M`), and the hardest case for a low-frequency
/// break-up is the WIDEST of them — the finer ones are further below
/// `MACRO_M` still. A literal would have gone on measuring 4 m after the
/// tiles moved, with this gate green and testing the wrong distance.
fn widest_tile() -> f32 {
    GROUND_TILE_M.iter().copied().fold(f32::MIN, f32::max)
}

/// The modifier a vertex at (x, z) carries, with the wet term out of the way.
/// `y` is above the waterline band and `grad` flat, so `wet_factor` is 0 and
/// `[0]` is the break-up alone.
fn modifier(x: f32, z: f32) -> f32 {
    vertex_mods(40.0, x, z, 0.0)[0]
}

/// **The gate.** The island's mean modifier is 1, so nothing this multiplies
/// gets brighter or darker on average.
#[test]
fn the_break_up_does_not_move_the_islands_brightness() {
    const STEP: f32 = 3.0;
    let (mut sum, mut n) = (0.0f64, 0u32);
    let mut z = STEP * 0.5;
    while z < ISLAND_SIZE {
        let mut x = STEP * 0.5;
        while x < ISLAND_SIZE {
            sum += f64::from(modifier(x, z));
            n += 1;
            x += STEP;
        }
        z += STEP;
    }
    let mean = sum / f64::from(n);
    // **The tolerance is derived, not picked.** `macro_noise` draws one value
    // per 48 m cell, so the island holds (2048/48)² ≈ 1 820 of them, and the
    // spatial mean of a uniform[-1, 1] field over N independent cells has an sd
    // of about `AMP / sqrt(3N)` = 0.0018. That residual is unavoidable without
    // pinning a measured bias constant, and 0.3% of the ground's brightness is
    // not a defect anybody can see. 1% is five times the noise floor and still
    // an order under anything worth catching — a field that means 1.05 brightens
    // the whole island by 5% with every other gate green.
    assert!(
        (mean - 1.0).abs() < 1e-2,
        "the ground's macro break-up means {mean:.5} over {n} samples, not 1. \
         It multiplies the authored identity colour, so this is the island's \
         overall brightness — and `fill::GROUND_MIX`, the bounce and the luma \
         pin in `ground_mix.rs` are all blind to it, because they fold the \
         splat weights and the authored identities and never read this slot."
    );
}

/// It varies at its own wavelength and not at the tile's.
///
/// Two points 4 m apart — one texture repeat — must see nearly the same
/// modifier, or the break-up is beating against the thing it exists to hide.
/// Two points `MACRO_M` apart must see a different one, or it is a constant
/// with a noise function's name.
///
/// The factor is 4 rather than the ~12 the wavelength ratio suggests, because
/// the per-vertex hash dither rides in the same slot and is white noise by
/// design — it contributes the same expected difference at both separations,
/// which drags the ratio toward 1. What is asserted is that the macro term is
/// still clearly the larger of the two at its own scale.
#[test]
fn the_break_up_varies_at_its_own_scale_and_not_the_tiles() {
    let mut near = 0.0f64;
    let mut far = 0.0f64;
    let mut n = 0u32;
    // Off-lattice and irrational-ish so the walk does not land on the lattice
    // corners every time and measure the one place the field is pinned.
    let mut z = 17.3f32;
    while z < ISLAND_SIZE - MACRO_M {
        let mut x = 23.7f32;
        while x < ISLAND_SIZE - MACRO_M {
            let a = modifier(x, z);
            near += f64::from((modifier(x + widest_tile(), z) - a).abs());
            far += f64::from((modifier(x + MACRO_M, z) - a).abs());
            n += 1;
            x += 31.0;
        }
        z += 29.0;
    }
    let (near, far) = (near / f64::from(n), far / f64::from(n));
    assert!(
        far > near * 1.15,
        "the modifier moves {near:.4} across one {} m texture tile and \
         {far:.4} across a whole {MACRO_M} m macro cell — the break-up is not \
         lower-frequency than the repeat it is hiding, so it adds a second \
         pattern instead of dissolving the first.",
        widest_tile()
    );
}

/// It stays inside the band its two constants allow, so it cannot drive a
/// colour negative or blow one out.
#[test]
fn the_break_up_is_bounded_by_its_own_constants() {
    // The dither is `0.88 + 0.24u`, so [0.88, 1.12]; the macro term is
    // `1 ± MACRO_AMP`. The product cannot leave their product.
    let lo = 0.88 * (1.0 - MACRO_AMP);
    let hi = 1.12 * (1.0 + MACRO_AMP);
    let (mut worst_lo, mut worst_hi) = (f32::MAX, f32::MIN);
    let mut z = 1.0f32;
    while z < ISLAND_SIZE {
        let mut x = 1.0f32;
        while x < ISLAND_SIZE {
            let m = modifier(x, z);
            worst_lo = worst_lo.min(m);
            worst_hi = worst_hi.max(m);
            x += 7.0;
        }
        z += 7.0;
    }
    assert!(
        worst_lo >= lo - 1e-5 && worst_hi <= hi + 1e-5,
        "the modifier reaches [{worst_lo:.4}, {worst_hi:.4}], outside the \
         [{lo:.4}, {hi:.4}] its own constants allow"
    );
    // Non-vacuity: it must actually use the band, or this asserts nothing.
    assert!(
        worst_hi - worst_lo > 0.25,
        "the modifier only spans {:.4} — it is nearly a constant, and a \
         constant hides no tiling",
        worst_hi - worst_lo
    );
}
