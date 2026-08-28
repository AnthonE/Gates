//! Gate: **the height remap curve is C¹**, which is the whole of why the
//! island stopped rendering as a topographic map.
//!
//! ## The defect this exists for
//!
//! `terrain::remap` was `lerp` between the 17 knots of `REMAP_LUT` until
//! 2026-08-26. A piecewise-linear curve is continuous and its *slope* is not:
//! it steps at every knot, by up to **12×** where the LUT leaves a shelf for a
//! cliff. `render/terrain_mesh.rs` takes its normal analytically from this
//! field's gradient — deliberately, so the mesh's own triangulation never
//! shades — so a slope step is a **normal** step, and a normal step along a
//! set of constant elevation is a contour line. Sixteen knots drew sixteen
//! rings, nested around every hill the way a survey map nests them, and
//! `examples/hillshade` renders the island as exactly that picture.
//!
//! ## Why the gate is the curve and not the island
//!
//! The obvious instrument — sweep the island, measure |∇‖∇h‖|, bin it by
//! elevation, fail if one elevation carries too much — **was built, measured,
//! and thrown away, and that is worth recording so nobody builds it again.**
//! It does not separate. Over four seeds the worst elevation bin scored
//! 3.58–4.65× the median *before* the fix and 1.54–3.52× *after* it: the
//! ranges overlap, so no threshold splits them. The reason is that the metric
//! cannot tell a crease from a cliff, and the LUT's whole design is to put
//! cliffs at fixed elevations (`TERRAIN.md` §1 stage 4: "it manufactures base
//! spots and the cliffs between them"). A real cliff has real curvature at its
//! lip, sitting in the same bin as the artifact and swamping it — the crease
//! itself is a measure-zero line diluted across a 1 m band of elevation.
//! A gate whose before and after overlap is worse than no gate: it reads as
//! covered.
//!
//! So this gates the **mechanism** instead, exactly and cheaply. Every contour
//! ring the hillshade drew came from a slope step in one of two places, and
//! both are checked here by construction rather than by sampling.

// This suite re-derives the tangent table and measures one-sided slopes, so it
// needs `abs`/`signum` — both on `sim-core/clippy.toml`'s wall-1 list, which is
// crate-scoped and therefore binds a test binary too. The wall is about what
// the SIM may compute; a gate computing a reference value on the host is the
// same exemption `examples/terrain_stats.rs` takes for `println!`. Nothing here
// is compiled into the sim.
#![allow(clippy::disallowed_methods)]

use sim_core::terrain::{self, REMAP_LUT, REMAP_TAN};

/// One-sided slopes of `remap` either side of `n`, over a step wide enough to
/// clear f32 cancellation and narrow enough that the cubic is locally linear.
fn slopes_across(n: f32) -> (f32, f32) {
    const E: f32 = 1.0 / 4096.0;
    let left = (terrain::remap(n) - terrain::remap(n - E)) / E;
    let right = (terrain::remap(n + E) - terrain::remap(n)) / E;
    (left, right)
}

/// **The gate.** No knot of the remap curve is a crease.
///
/// Red the moment `remap` goes back to `lerp`: the worst knot steps by 1.76
/// in these units under the old body (knot 12, a shelf meeting a cliff) and by
/// 0.0007 under the cubic, so the 0.02 bound below is two orders of magnitude
/// clear of the defect and two clear of the noise. Proven red by rebuilding
/// the old body in `a_piecewise_linear_remap_would_fail_this` — the mutant,
/// because a bound nobody has seen fail is a bound nobody has checked.
#[test]
fn the_remap_curve_creases_at_no_knot() {
    const BOUND: f32 = 0.02;
    let mut worst = (0usize, 0.0f32);
    for i in 1..16 {
        let n = i as f32 / 16.0;
        let (l, r) = slopes_across(n);
        let step = (r - l).abs();
        if step > worst.1 {
            worst = (i, step);
        }
    }
    assert!(
        worst.1 < BOUND,
        "knot {} of the remap curve steps its slope by {:.4} (bound {BOUND}) — \
         the height field creases at that elevation everywhere on the island \
         at once, and the renderer's analytic normals draw it as a contour \
         line. See this file's header.",
        worst.0,
        worst.1
    );
}

/// The same property at the two **domain rails**, which is a separate failure
/// with the same symptom.
///
/// `remap` clamps its input to [0, 1], so outside that domain it is a
/// constant — and a curve is C¹ across a clamp iff its derivative is zero at
/// the rail. Fritsch–Carlson's own end formula gives 0.48 and 2.40 here, i.e.
/// a slope meeting a flat, which drew a rim around the summit of every
/// mountain and around every lowland flat. The top rail is the steeper by 5×
/// and is the one on the ground players look at.
///
/// The bottom rail is held at the LUT's own secant instead of zero **and that
/// is deliberate**: `REMAP_LUT`'s first three segments have equal secants, so
/// the cubic through them is bit-for-bit the old straight line, and the entire
/// low country — the coast, the road ring, the haven pad — is where it was.
/// The rail it leaves is at `remap(0) = 0`, which is `land = 0`, which is the
/// waterline. A crease at sea level is under the sea.
#[test]
fn the_remap_curve_meets_its_top_rail_flat() {
    let (l, r) = slopes_across(1.0);
    assert!(
        r.abs() < 0.01,
        "the curve is not flat above n = 1: {r:.4}. `remap` clamps there, so \
         this is a slope meeting a constant — a hard rim at exactly \
         `AMPLITUDE` around every summit on the island."
    );
    assert!(
        (l - r).abs() < 0.02,
        "the top rail steps by {:.4} — see the test name and this file's header.",
        (l - r).abs()
    );
    // And the bottom rail is the OLD behaviour, on purpose: assert the shape
    // rather than let a later pass "fix" it and move the coastline.
    let (bl, _) = slopes_across(0.0);
    assert!(
        bl.abs() < 0.01 && terrain::remap(0.0) == 0.0,
        "remap(n ≤ 0) must be a flat 0: the waterline is where `land = 0`, and \
         the road, the haven solve and the clutter waterline veto are all \
         measured against it standing still."
    );
}

/// `REMAP_TAN` re-derives from `REMAP_LUT` — the table is authored offline and
/// this is what stops the two from drifting apart.
///
/// Fritsch–Carlson, written out: the harmonic mean of the neighbouring secants
/// at an interior knot, capped at 3× the smaller of them (the cap is what
/// makes the interpolant monotone), and zero at the top rail for the reason
/// the test above states. Editing `REMAP_LUT` without re-deriving `REMAP_TAN`
/// is silent otherwise — the curve still interpolates the new knots, it just
/// stops being monotone between them, which is a lip around a shelf.
#[test]
fn the_tangent_table_re_derives_from_the_knots() {
    const H: f32 = 1.0 / 16.0;
    let d: Vec<f32> = (0..16)
        .map(|i| (REMAP_LUT[i + 1] - REMAP_LUT[i]) / H)
        .collect();
    let mut want = [0.0f32; 17];
    for i in 1..16 {
        if d[i - 1] * d[i] > 0.0 {
            let m = 2.0 * d[i - 1] * d[i] / (d[i - 1] + d[i]);
            let cap = 3.0 * d[i - 1].abs().min(d[i].abs());
            want[i] = m.abs().min(cap) * m.signum();
        }
    }
    // Bottom: the standard three-point end formula. Top: zero, for the rail.
    want[0] = (3.0 * d[0] - d[1]) / 2.0;
    if want[0] * d[0] <= 0.0 {
        want[0] = 0.0;
    }
    want[16] = 0.0;
    for i in 0..17 {
        let got = REMAP_TAN[i] / H;
        assert!(
            (got - want[i]).abs() < 1.5e-4 * want[i].abs().max(1.0),
            "REMAP_TAN[{i}] is {got:.6} and Fritsch-Carlson over REMAP_LUT says \
             {:.6}. The two have drifted — the knots were edited without \
             re-deriving the tangents, and the curve between them is no longer \
             guaranteed monotone.",
            want[i]
        );
    }
}

/// The curve still does the job the knots were authored for: monotone, inside
/// [0, 1], and never more than 2.3% of the amplitude away from the straight
/// lines it replaced.
///
/// That last number is the one that says the shelves are the same shelves. The
/// LUT *is* the game design — where a base spot is and where the cliff above
/// it starts — so a smoother curve is only admissible if it did not move any
/// of that; 2.22% of 90 m is 2.0 m, spent almost entirely on rounding a summit.
#[test]
fn the_curve_is_monotone_and_stays_on_its_knots() {
    const N: usize = 100_000;
    let mut prev = terrain::remap(-0.5);
    let mut worst_dev = 0.0f32;
    for k in 0..=N {
        let n = -0.5 + 2.0 * k as f32 / N as f32;
        let v = terrain::remap(n);
        assert!(
            v >= prev - 1e-6,
            "remap is not monotone: it fell from {prev} to {v} at n = {n}. A \
             non-monotone shaping curve inverts the ground — a hill becomes a \
             pit for the same fBm value."
        );
        assert!((0.0..=1.0).contains(&v), "remap({n}) = {v} left [0, 1]");
        prev = v;
        if (0.0..=1.0).contains(&n) {
            let t = n * 16.0;
            let i = (t as usize).min(15);
            let lin = REMAP_LUT[i] + (REMAP_LUT[i + 1] - REMAP_LUT[i]) * (t - i as f32);
            worst_dev = worst_dev.max((v - lin).abs());
        }
    }
    assert!(
        worst_dev < 0.03,
        "the cubic is {worst_dev:.4} away from the piecewise-linear curve it \
         replaced (bound 0.03 = 2.7 m of AMPLITUDE). The knots are the game \
         design; smoothing them is not licence to move a shelf."
    );
    for (i, &k) in REMAP_LUT.iter().enumerate() {
        let v = terrain::remap(i as f32 / 16.0);
        assert!(
            (v - k).abs() < 1e-5,
            "remap misses its own knot {i}: {v} vs {k}. Hermite interpolates \
             its knots by construction, so this is a basis-function error."
        );
    }
}

/// The mutant: the body this gate was written against must fail it.
///
/// `CLAUDE.md`'s standing rule for a gate on an optimisation or a rewrite —
/// after writing it, run the thing it is supposed to catch. Here that is the
/// old `lerp` body, rebuilt from `REMAP_LUT` alone so it shares no code with
/// what it is checking, and the assertion is that
/// `the_remap_curve_creases_at_no_knot` would have been **red** on it.
#[test]
fn a_piecewise_linear_remap_would_fail_this() {
    let old = |n: f32| {
        let t = n.clamp(0.0, 1.0) * 16.0;
        let i = (t as usize).min(15);
        REMAP_LUT[i] + (REMAP_LUT[i + 1] - REMAP_LUT[i]) * (t - i as f32)
    };
    const E: f32 = 1.0 / 4096.0;
    let mut worst = 0.0f32;
    for i in 1..16 {
        let n = i as f32 / 16.0;
        let l = (old(n) - old(n - E)) / E;
        let r = (old(n + E) - old(n)) / E;
        worst = worst.max((r - l).abs());
    }
    assert!(
        worst > 0.02,
        "the old piecewise-linear remap steps by only {worst:.4}, under this \
         file's 0.02 bound — so the bound no longer catches the defect the \
         file exists for and one of the two is wrong."
    );
    // And the top rail, which is the other half.
    let top = (old(1.0) - old(1.0 - E)) / E;
    assert!(
        top.abs() > 0.01,
        "the old curve met its top rail flat at {top:.4}, so the rail test \
         above is not catching anything either."
    );
}
