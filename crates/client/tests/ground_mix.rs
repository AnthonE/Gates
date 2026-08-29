//! Gate: the client's ground mix is the **whole island's**, and the window it
//! was measured over is part of the claim.
//!
//! **Why this exists, and it is the trap list's own entry firing a second
//! time.** `CLAUDE.md` records a sweep that measured 40 islands over
//! `-1024..1024` on both axes while `terrain::continent` centres the island on
//! `(ISLAND_SIZE/2, ISLAND_SIZE/2)` — so world coordinates run `0..2048` and
//! that square's *corner* is the island's centre. It sampled one quadrant. The
//! conclusion was retracted 2026-08-14, `sim-core/tests/relief.rs` took a gate
//! against the window coming back, and the entry ends: *"cross-check a sweep's
//! window against something that already knows where the world is, before the
//! finding earns a gate and a doc paragraph."*
//!
//! **Two constants on the client side were derived from that same window and
//! were never re-measured.** `fill::GROUND_MIX` and the paragraph in
//! `terrain_mesh::GROUND_ALBEDO` that justifies granite's value both cite "over
//! 39,521 land samples at seed 20260731 the mean splat is sand 0.008, grass
//! 0.619, litter 0.373, rock 0.0000", and both conclude in as many words that
//! *granite never reaches the ground at this seed at all*. That number is
//! reproducible: the quadrant window at a 4 m step returns land = **39,521**
//! (39,530 since worldgen shape v1 moved the coastline by nine samples), and
//! `0..1024` alone returns the identical count — every land sample in
//! `-1024..1024` lies in the south-west quadrant, because the negative half is
//! open sea. That second equality is the cross-check and it is still exact;
//! the first is now a 1% band, for the reason the test states. `the_retracted_window_is_what_produced_the_old_
//! mix` below asserts exactly that, so the retraction is a checked fact rather
//! than a remembered one.
//!
//! Whole-island the same law says granite is **9.2%** of the mix — the third
//! identity, eight times sand's share — and `sim-core/tests/relief.rs` and
//! `examples/seed_scan.rs` have said so independently since the retraction
//! (10.0% of land carries a legible rock weight; 8.9% within 300 m of the
//! capture spawn). Three sources that already knew where the world is, against
//! one that did not.
//!
//! **What this gate does NOT do.** It does not move `GROUND_ALBEDO`. Granite's
//! albedo was authored as a free parameter *because* the mix said its weight
//! was zero, and it is not free any more — the area-weighted mean linear luma
//! that whole re-placement was pinned to is 14.4% low. Re-placing four coupled
//! albedos is the brightness owner's iteration (`CLAUDE.md`: tonemap, sky,
//! exposure and fog are one owner, and three parallel passes over a coupled set
//! made things measurably worse). This pass corrects the *measurement* and
//! writes down what it now costs; `NOW.md` carries the re-placement.
//!
//! **Four of the five here were run red before they were run green**, against
//! the shipped `[0.008, 0.619, 0.373, 0.0]`: the whole-island match, the
//! "asserting nothing" guard at the end of the retraction test, granite's
//! share, and the mean-luma delta (which read a ratio of 1.0005 while the
//! constant still was the quadrant). Granite's share was a runtime assert when
//! it was seen red and is a `const` block now, so on the old constant it would
//! not fail this suite — it would fail the build. That is the stronger form and
//! the change is recorded here rather than left to look like a green test.
//! The fifth,
//! `the_capture_spawn_stands_on_one_identity`, is independent of the constant
//! by construction — it asks the world a question, not the client — and it
//! passed from the first run. That is the same shape as `ground_identity.rs`'s
//! fifth test and it is said here rather than left for a reader to discover,
//! because a gate nobody has seen fail is not evidence.

use client::render::fill::{luminance, GROUND_MIX};
use client::render::terrain_mesh::GROUND_ALBEDO;
use sim_core::terrain;

/// The shipped island (`shard.toml`, and the seed every `-visual.md` is shot
/// on).
const SEED: u64 = 20260731;

/// The sweep step, metres. `ground_identity.rs`'s granite test already walks
/// the whole world square at 4 m and `ci/gates.sh` runs it, so the cost is
/// known-affordable. The mix is stable to 1e-4 across 2 m / 4 m / 6 m, which is
/// what makes it a property of the island rather than of the step.
const STEP: f32 = 4.0;

/// Land, the shipped test — `relief.rs:197` and `ground_identity.rs:293` both
/// use `height > 0.5`, and `LAND_MIN_H` (0.6) moves the count by 0.05%.
fn is_land(x: f32, z: f32) -> bool {
    terrain::height(SEED, x, z) > 0.5
}

/// Area-weighted mean splat over a square window, plus the land sample count.
///
/// `terrain::splat` and not `splat_from`: it is the call `relief.rs`,
/// `seed_scan.rs` and `ground_identity.rs` all make, so a disagreement between
/// this gate and those three would be a real disagreement rather than a
/// difference in how the slope was taken.
fn mix_over(lo: f32, hi: f32) -> ([f64; 4], u32) {
    let mut acc = [0f64; 4];
    let mut land = 0u32;
    let mut z = lo + STEP * 0.5;
    while z < hi {
        let mut x = lo + STEP * 0.5;
        while x < hi {
            if is_land(x, z) {
                land += 1;
                let w = terrain::splat(SEED, x, z);
                for c in 0..4 {
                    acc[c] += f64::from(w[c]) / 255.0;
                }
            }
            x += STEP;
        }
        z += STEP;
    }
    let n = f64::from(land.max(1));
    ([acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n], land)
}

/// The same, restricted to a disc — what a camera standing at one point sees
/// around it, which is not the same question as what the island averages.
fn mix_in_disc(cx: f32, cz: f32, r: f32, step: f32) -> ([f64; 4], u32) {
    let mut acc = [0f64; 4];
    let mut land = 0u32;
    let mut z = cz - r;
    while z <= cz + r {
        let mut x = cx - r;
        while x <= cx + r {
            let (dx, dz) = (x - cx, z - cz);
            if dx * dx + dz * dz <= r * r && x >= 0.0 && z >= 0.0 && is_land(x, z) {
                land += 1;
                let w = terrain::splat(SEED, x, z);
                for c in 0..4 {
                    acc[c] += f64::from(w[c]) / 255.0;
                }
            }
            x += step;
        }
        z += step;
    }
    let n = f64::from(land.max(1));
    ([acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n], land)
}

const SAND: usize = 0;
const GRASS: usize = 1;
const LITTER: usize = 2;
const ROCK: usize = 3;

/// **The gate.** `GROUND_MIX` is the mean splat over the world square
/// `[0, ISLAND_SIZE)`, not over any sub-window of it.
///
/// The tolerance is 5e-4 per channel: the mix moves by ~1e-4 between a 2 m and
/// a 6 m step, so this is tight enough that a quadrant (which moves rock by
/// 0.09) cannot pass and loose enough that the step is not being pinned.
#[test]
fn the_ground_mix_is_the_whole_island() {
    let (mix, land) = mix_over(0.0, terrain::ISLAND_SIZE);
    println!(
        "whole island, step {STEP} m: land {land}, mix [{:.6}, {:.6}, {:.6}, {:.6}], sum {:.6}",
        mix[0],
        mix[1],
        mix[2],
        mix[3],
        mix.iter().sum::<f64>()
    );
    for c in 0..4 {
        let got = f64::from(GROUND_MIX[c]);
        assert!(
            (got - mix[c]).abs() < 5e-4,
            "channel {c}: GROUND_MIX says {got:.6}, the whole island measures {:.6}",
            mix[c]
        );
    }
}

/// **The retraction, as a checked fact.** The window that produced the old
/// constant is the quadrant, and it is the quadrant *because the other three
/// quarters are sea* — `0..1024` returns the identical land count, which is
/// the cross-check `CLAUDE.md` says was missing the first time.
///
/// This test is the one that would have caught the original error with no
/// renderer and no judge: the land count implied a quarter-disc all along.
#[test]
fn the_retracted_window_is_what_produced_the_old_mix() {
    let (quad, land_signed) = mix_over(-1024.0, 1024.0);
    let (_, land_sw) = mix_over(0.0, 1024.0);

    println!(
        "-1024..1024: land {land_signed}, mix [{:.6}, {:.6}, {:.6}, {:.6}]",
        quad[0], quad[1], quad[2], quad[3]
    );

    // The doc comments this pass corrects both cite 39,521 land samples.
    //
    // ⚠ **A band, not the literal, and the reason is that the literal was a
    // fact about a worldgen rather than about a window.** It was `== 39_521`
    // until 2026-08-26, when `terrain::remap` became a monotone cubic
    // (`DECISIONS.md` §open, worldgen shape v1) and the same window came back
    // 39,530 — nine samples, 0.02%, entirely the coastline breathing under a
    // different shaping curve. Pinning it exactly made this gate go red for a
    // change it has no opinion about, which is the shape of a gate that stops
    // being read. What it is actually for is that **this window is a quadrant
    // of the island**, and 1% is far tighter than the 4× a wrong window costs.
    assert!(
        land_signed.abs_diff(39_521) * 100 < 39_521,
        "the retracted window returns {land_signed} land samples against the \
         39,521 the old doc cited — more than 1% away, so this is no longer \
         the window that produced the retracted constant and the reproduction \
         below is not reproducing anything."
    );
    // And every one of them is in the south-west quadrant: the negative half
    // of a world that starts at 0 is open sea, so the signed window was never
    // sampling anything the unsigned one did not.
    assert_eq!(
        land_sw, land_signed,
        "0..1024 and -1024..1024 disagree, so the negative half is not empty"
    );

    // The old constant, reproduced. Retained as a literal on purpose: this is
    // the number the tree shipped, and a gate that only says "not the current
    // one" would go green if someone re-derived a third wrong window.
    let old = [0.008f64, 0.619, 0.373, 0.0];
    for c in 0..4 {
        assert!(
            (quad[c] - old[c]).abs() < 5e-4,
            "channel {c}: the quadrant measures {:.6}, the retracted constant said {:.6}",
            quad[c],
            old[c]
        );
    }
    // …and it is not the island.
    assert!(
        (quad[ROCK] - f64::from(GROUND_MIX[ROCK])).abs() > 0.05,
        "the quadrant and the island agree about granite, so this gate is asserting nothing"
    );
}

/// **"Granite never reaches the ground at this seed at all" is false**, and
/// two files said it in the present tense.
///
/// It is the third identity by area — eight times sand's share — which is why
/// its albedo is no longer the free parameter `GROUND_ALBEDO`'s doc treated it
/// as. The floor is 0.05 rather than the measured 0.09 so that this asserts the
/// *mechanism* (granite is a real share of the island) and does not re-pin a
/// number `relief.rs` already owns.
///
/// **Both are `const` blocks, which is the stronger form and is clippy's own
/// suggestion rather than a workaround for it.** Every operand is a constant,
/// so the check belongs at compile time: reintroducing the quadrant's mix does
/// not fail this suite, it fails the *build* of anything that links it, and no
/// one has to remember to run a test to find that out.
#[test]
fn granite_is_a_real_share_of_the_island() {
    // "GROUND_MIX still says granite is absent."
    const { assert!(GROUND_MIX[ROCK] > 0.05) };
    // "granite is not above sand — the ranking the old mix inverted."
    const { assert!(GROUND_MIX[ROCK] > GROUND_MIX[SAND]) };

    // Printed rather than asserted: the magnitude is `relief.rs`'s to own, and
    // a reader of this suite's output should see what the two consts guard.
    println!(
        "GROUND_MIX granite {:.6}, sand {:.6}, ratio {:.2}×",
        GROUND_MIX[ROCK],
        GROUND_MIX[SAND],
        GROUND_MIX[ROCK] / GROUND_MIX[SAND]
    );
}

/// **What the capture camera actually stands on**, and the reason the newest
/// `-visual.md` measures 0.00% green ground cover in four of six frames.
///
/// `gates-loop/art/capture-native.sh` pins `dev_spawn = 1155,140` and the six
/// vantages do not translate, so every frame the visual judge has ever scored
/// natively is shot from this one point. Within 60 m of it the island is
/// **93% forest litter and 7% beach sand, with grass and granite at exactly
/// zero** — so there is no tuft for `clutter_kind_at` to roll (it draws kinds
/// from the same splat weights) and no second identity in the ground colour.
///
/// That is a fact about the world, not a defect in the renderer, and it is
/// asserted here so the next pass reading "no grass geometry" in a visual
/// report knows the near field has no grass in it to geometry. It does not
/// excuse the gap — the reference frames are dense at every bearing — but it
/// does say the fix is density and identity-count, not a broken emitter.
#[test]
fn the_capture_spawn_stands_on_one_identity() {
    // `capture-native.sh`'s `dev_spawn`. Outside this repo and not ours to
    // move, which is why it is a constant here and not a parameter.
    const CX: f32 = 1155.0;
    const CZ: f32 = 140.0;

    let (near, n_near) = mix_in_disc(CX, CZ, 60.0, 1.0);
    println!(
        "60 m disc at ({CX}, {CZ}): land {n_near}, mix [{:.6}, {:.6}, {:.6}, {:.6}]",
        near[0], near[1], near[2], near[3]
    );
    assert!(n_near > 1_000, "the near disc found no land: {n_near}");
    assert!(
        near[LITTER] > 0.90,
        "the near field is not litter-dominant: {:.6}",
        near[LITTER]
    );
    assert_eq!(
        near[GRASS], 0.0,
        "the near field has grass in it now — the 0.00%-green finding has a different cause"
    );
    assert_eq!(
        near[ROCK], 0.0,
        "the near field has granite in it now — re-read the visual report's granite line"
    );

    // The 300 m disc is a different island: `seed_scan --at 1155,140,300`
    // reports 8.9% legible rock there, and it is where the two boulders the
    // judge counted are. So the frames are not granite-free because the seed
    // is — they are granite-free because 60 m is.
    let (mid, n_mid) = mix_in_disc(CX, CZ, 300.0, 4.0);
    println!(
        "300 m disc: land {n_mid}, mix [{:.6}, {:.6}, {:.6}, {:.6}]",
        mid[0], mid[1], mid[2], mid[3]
    );
    assert!(
        mid[ROCK] > 0.05,
        "granite is absent at 300 m too: {:.6}",
        mid[ROCK]
    );
}

/// **The mean is held against the island, and this is what holds it.**
///
/// ⚠ **This test used to assert a debt; it asserts the payment now** (2026-08-15).
/// It read `the_held_mean_luma_was_taken_against_the_quadrant` and checked that
/// the shipped albedos reproduced the doc's 0.09390 under the *quadrant's*
/// weights — a true statement about constants that were placed against the
/// wrong window, and a deliberate placeholder for the re-place that has now
/// happened. Keeping it would have meant keeping four albedos pinned to a
/// quadrant.
///
/// The successor is strictly stronger, because the old form pinned nothing that
/// could go wrong in the future — a gap being "large" stays large under almost
/// any edit. This pins the number itself: **the island-weighted mean linear
/// luma is 0.10746**, the value the pre-re-place constants actually delivered,
/// so the 2026-08-15 identity re-place is provably brightness-neutral and any
/// later albedo edit that moves the island's overall brightness goes red here
/// rather than arriving as a surprise in a frame. ⚠ **Re-pinned 0.10746 →
/// 0.10715 on 2026-08-26** — not an albedo edit: `GROUND_MIX` is a measurement
/// of the island and worldgen shape v1 changed the island. No value in
/// `GROUND_ALBEDO` moved, and −0.29% is what a 1.7%-of-its-own-value shift in
/// the brightest identity's *weight* buys. The gate's job is unchanged; it
/// now holds the four albedos against the island they are actually averaged
/// over. Brightness is the coupled
/// owner's (`CLAUDE.md` traps); this is the gate that keeps an identity edit
/// from taking it by accident.
///
/// The second assert is the retraction's own point, kept: the two windows
/// disagree, and by more than they used to (1.145× → 1.268×), because granite
/// carries more value now and the quadrant still weights it at zero.
#[test]
fn the_mean_luma_is_held_against_the_island_not_the_quadrant() {
    // Rec.601, the estimator `ground_identity.rs` uses and the one the 0.09390
    // figure reproduces under. The repo carries both and they disagree; the
    // point here is the delta between two WEIGHTINGS under one estimator.
    fn luma601(c: [f32; 3]) -> f64 {
        0.299 * f64::from(c[0]) + 0.587 * f64::from(c[1]) + 0.114 * f64::from(c[2])
    }
    let quad = [0.0078f64, 0.6195, 0.3727, 0.000_015];
    let mean_of = |w: &[f64; 4]| -> f64 {
        (0..4)
            .map(|k| luma601(GROUND_ALBEDO[k]) * w[k])
            .sum::<f64>()
    };
    let island: [f64; 4] = [
        f64::from(GROUND_MIX[0]),
        f64::from(GROUND_MIX[1]),
        f64::from(GROUND_MIX[2]),
        f64::from(GROUND_MIX[3]),
    ];
    let (was, now) = (mean_of(&quad), mean_of(&island));
    println!(
        "mean linear luma (Rec.601): quadrant {was:.5}, island {now:.5}, ratio {:.4}",
        now / was
    );

    assert!(
        (now - 0.107_15).abs() < 1e-4,
        "the island-weighted mean linear luma is no longer the 0.10715 the \
         identity re-place was held to: {now:.5}. An albedo edit moved the \
         island's overall brightness — that is the coupled lighting owner's \
         call (`CLAUDE.md` traps), not an identity pass's."
    );
    assert!(
        now > was * 1.10,
        "the island weighting is not materially brighter than the quadrant's: {now:.5} vs {was:.5}"
    );
    // Rec.709 too, so the finding is not an artefact of the estimator split
    // `NOW.md` §0gi flagged (the two disagree by 24× on the OLD constraint).
    let now709: f64 = (0..4)
        .map(|k| f64::from(luminance(GROUND_ALBEDO[k])) * island[k])
        .sum();
    let was709: f64 = (0..4)
        .map(|k| f64::from(luminance(GROUND_ALBEDO[k])) * quad[k])
        .sum();
    println!("mean linear luma (Rec.709): quadrant {was709:.5}, island {now709:.5}");
    assert!(
        now709 > was709 * 1.10,
        "Rec.709 does not agree that the island is brighter: {now709:.5} vs {was709:.5}"
    );
}
