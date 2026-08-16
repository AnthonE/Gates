//! The site carve — `TERRAIN.md` §1 stage 8's "carve a flat pad with a smooth
//! blend radius", and the seam that carries it.
//!
//! **Armed 2026-08-16.** It shipped dark for one pass — the seam without the
//! cut, so the cross-cutting edit and the behaviour change were separate
//! commits — and the tests below were written against `site_stamp_with` so the
//! arithmetic was exercised at full depth while the shipped constant was still
//! zero. That shape is kept: the strength is pinned in §A rather than assumed
//! by the rest, so a future change to it fails one named gate instead of
//! quietly re-meaning seven.
//!
//! What this file holds, in the order the mechanism reads:
//!   §A the carve is armed, at the three constants that were measured
//!   §B the carve flattens the floor it claims to flatten
//!   §C it never reaches outside the blend that publishes it
//!   §D the blend has no edge in it
//!   §E the relief the pad settled for by *finding* goes to zero by *making*
//!   §F the stamp cannot read the terrain — the circularity is structural
//!   §G the carve never makes an authored structure's footing WORSE
//!   §H the carve never builds a wall — the gradient it ADDS stays bounded

use sim_core::fmath::fabs;
use sim_core::terrain::{
    self, Haven, HAVEN_FOOTPRINT, SITE_STAMP_STRENGTH, WAYSTATIONS, WAYSTATION_FOOTPRINT,
};

/// The `tests/haven.rs` seed list, for the same reason it gives: "a seed that
/// fails is a bug in the generator, not a reroll".
const SEEDS: [u64; 16] = [
    1,
    2,
    7,
    42,
    99,
    1337,
    20_260_731,
    20_260_804,
    555_555,
    8_675_309,
    31_337,
    4_294_967_291,
    123_456_789,
    999_999_937,
    0xDEAD_BEEF,
    0x0BAD_C0DE,
];

/// Seeds for the checks that sweep the whole island.
const SWEEP_SEEDS: [u64; 4] = [1, 42, 20_260_804, 0xDEAD_BEEF];

/// Seeds whose sites land on genuinely rough ground — where the cut clamp
/// binds. Found by sweeping 256 seeds for the worst gradient the carve adds;
/// see §H for why the ordinary list cannot stand in for these.
const ROUGH_SEEDS: [u64; 5] = [
    5_513_890_487_717_207,
    659_067_054_832_189,
    6_576_107_712_789_375,
    8_125_474_678_997_397,
    8_888_394_596_499_862,
];

/// A millimetre. The carve's claims are physical — "this floor is flat" — so
/// they are asserted at a physical tolerance rather than on bit equality:
/// `raw + (site_y - raw)` is within an ulp of `site_y` but is not it, and
/// pretending otherwise would be a gate that fails on rounding.
const MM: f32 = 1.0e-3;

/// Every live site on a seed as `(x, z, y, footprint)` — including the ones
/// whose footprint does not currently fit and therefore carve nothing.
fn sites(h: &Haven) -> Vec<(f32, f32, f32, terrain::SiteFootprint)> {
    let mut v = vec![(h.x, h.z, h.y, HAVEN_FOOTPRINT)];
    for w in 0..WAYSTATIONS {
        let ws = &h.minor[w];
        if ws.live {
            v.push((ws.x, ws.z, ws.y, WAYSTATION_FOOTPRINT));
        }
    }
    v
}

/// The live sites whose footprint actually carves — floor strictly inside the
/// blend that has to return it to raw ground.
///
/// Both tiers qualify since `WAYSTATION_RADIUS_M` widened and `blend_m` stopped
/// being bounded by the scatter mask. The filter stays because it is the
/// degenerate case `stamp_of` states out loud: a footprint whose floor does not
/// fit inside its own ramp carves nothing at all, silently, and a gate that
/// quietly covered zero sites is worse than one that covers one.
fn carving_sites(h: &Haven) -> Vec<(f32, f32, f32, terrain::SiteFootprint)> {
    let v: Vec<_> = sites(h)
        .into_iter()
        .filter(|(_, _, _, fp)| fp.stamp_m < fp.blend_m)
        .collect();
    assert!(
        v.len() == sites(h).len(),
        "a site's floor no longer fits inside its blend, so it carves nothing \
         and every check in this file skips it in silence"
    );
    v
}

/// The carved ground at a named strength — `terrain::ground`'s body with the
/// constant lifted out, which is the one thing a test may reimplement here
/// because the arithmetic under test is `site_stamp_with`'s and not this sum.
fn carved(strength: f32, seed: u64, h: &Haven, x: f32, z: f32) -> f32 {
    let raw = terrain::height(seed, x, z);
    raw + terrain::site_stamp_with(strength, h, raw, x, z)
}

// ---------------------------------------------------------------- §A dark

/// The carve is **armed**, and the three constants that decide what it does are
/// pinned here so that changing any of them is a deliberate edit to a gate.
///
/// It shipped dark for one pass (the seam without the cut, so the cross-cutting
/// edit and the behaviour change were separate commits) and was armed on
/// 2026-08-16 by the operator. Arming moved the terrain goldens, which is
/// expected and is a wipe (`DECISIONS.md` 2026-08-10).
#[test]
fn the_carve_is_armed_at_the_measured_constants() {
    assert_eq!(
        SITE_STAMP_STRENGTH, 1.0,
        "the carve's strength moved. It is a worldgen change — goldens and a \
         wipe — and an operator call (DECISIONS.md: site carve v0)."
    );
    assert_eq!(
        terrain::SITE_BLEND_M,
        12.0,
        "SITE_BLEND_M is the ramp length and therefore the cut budget \
         (`max_cut`). It was swept over 128 seeds: 10 m is where every site \
         flattens completely and 12 is the margin over it. Shortening it \
         silently returns partly-levelled sites."
    );
    assert_eq!(
        terrain::SITE_CUT_HEADROOM,
        0.5,
        "SITE_CUT_HEADROOM is the share of the cliff budget the ramp may spend. \
         See §H — at 1.0 the worst carved gradient sits on the threshold."
    );
}

/// Outside every site's blend the ground is untouched — `height`'s own bits.
///
/// This was "the carve is dark, so this holds everywhere" before arming. Armed,
/// it is the statement that actually matters and it is the one §C bounds: the
/// carve reshapes a bowl around each destination and **nothing else on the
/// island**. `to_bits` rather than `==` so a `-0.0` for a `0.0` is caught —
/// `ground`'s early return exists precisely so no worldgen height is put
/// through a `+ 0.0` that could re-sign it.
#[test]
fn ground_is_height_to_the_bit_outside_every_blend() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (sx, sz, _, fp) in sites(&h) {
            let r = fp.blend_m + 8.0;
            let mut i = -24i32;
            while i <= 24 {
                let mut j = -24i32;
                while j <= 24 {
                    let x = sx + r * (i as f32 / 24.0);
                    let z = sz + r * (j as f32 / 24.0);
                    // Only outside every blend — inside is §B/§C/§D's business.
                    if sites(&h).iter().any(|(ox, oz, _, ofp)| {
                        (x - ox) * (x - ox) + (z - oz) * (z - oz) < ofp.blend_m * ofp.blend_m
                    }) {
                        j += 1;
                        continue;
                    }
                    let raw = terrain::height(seed, x, z);
                    let g = terrain::ground(seed, &h, x, z);
                    assert_eq!(
                        g.to_bits(),
                        raw.to_bits(),
                        "seed {seed} at ({x}, {z}): ground {g} != height {raw} \
                         outside every blend"
                    );
                    j += 1;
                }
                i += 1;
            }
        }
    }
}

// ------------------------------------------------------------- §B flatten

/// Armed, the swept floor is flat — that is the whole of what stage 8 asked
/// for and it has never been true before this pass.
///
/// The claim is measured the way a player would feel it: max−min over the
/// floor, not agreement with a formula. Inside `swept_m` the ramp is 0, so the
/// stamp is the full `site_y - raw` and every point lands on the site's own
/// reference height.
#[test]
fn the_armed_carve_flattens_the_floor() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (sx, sz, sy, fp) in carving_sites(&h) {
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            let mut i = -16i32;
            while i <= 16 {
                let mut j = -16i32;
                while j <= 16 {
                    let x = sx + fp.stamp_m * (i as f32 / 16.0);
                    let z = sz + fp.stamp_m * (j as f32 / 16.0);
                    // The floor is the disc, not the square around it.
                    if (x - sx) * (x - sx) + (z - sz) * (z - sz) > fp.stamp_m * fp.stamp_m {
                        j += 1;
                        continue;
                    }
                    let g = carved(1.0, seed, &h, x, z);
                    lo = lo.min(g);
                    hi = hi.max(g);
                    j += 1;
                }
                i += 1;
            }
            assert!(
                hi - lo < MM,
                "seed {seed} site ({sx}, {sz}): armed floor spans {} m, not flat",
                hi - lo
            );
            assert!(
                fabs(lo - sy) < MM && fabs(hi - sy) < MM,
                "seed {seed} site ({sx}, {sz}): armed floor sits at {lo}..{hi}, \
                 not on the site's own y {sy}"
            );
        }
    }
}

// --------------------------------------------------------------- §C bound

/// The carve reaches exactly as far as the footprint that publishes it and not
/// one metre further — asserted islandwide, at full strength, on bits.
///
/// This is the assertion that would have caught the failure this whole split
/// exists to avoid. A stamp that leaked past `scatter_m` would move ground that
/// the scatter grid, the clutter population and the site solver all believe is
/// wilderness, and every one of those reads a different function, so the
/// disagreement would surface as a floating tree rather than as a red test.
#[test]
fn the_armed_carve_never_reaches_outside_a_footprint() {
    for seed in SWEEP_SEEDS {
        let h = terrain::haven(seed);
        let all = sites(&h);
        let mut ix = 0i32;
        while ix < 256 {
            let mut iz = 0i32;
            while iz < 256 {
                let x = ix as f32 * (terrain::ISLAND_SIZE / 256.0);
                let z = iz as f32 * (terrain::ISLAND_SIZE / 256.0);
                let inside = all.iter().any(|(sx, sz, _, fp)| {
                    (x - sx) * (x - sx) + (z - sz) * (z - sz) < fp.blend_m * fp.blend_m
                });
                if !inside {
                    let raw = terrain::height(seed, x, z);
                    let g = carved(1.0, seed, &h, x, z);
                    assert_eq!(
                        g.to_bits(),
                        raw.to_bits(),
                        "seed {seed} at ({x}, {z}): the armed carve moved ground \
                         outside every blend"
                    );
                }
                iz += 1;
            }
            ix += 1;
        }
    }
}

// ---------------------------------------------------------------- §D band

/// The blend across the band is monotone — a ramp, not a step.
///
/// `MONUMENTS.md` §3 is the reason this is gated separately from §B: the
/// reference game shipped monuments sitting on visible circular plateaus for
/// years, and the defect there is not "the floor is wrong", it is "the edge is
/// an edge". A carve that flattened the floor and then dropped to raw at the
/// mask would pass §B and §C and draw a cliff ring.
///
/// Continuity along a radial, against a **derived** bound rather than a taste
/// threshold — the carved ground may not step further between two samples than
/// the raw ground does plus what the blend's own steepest slope can add.
///
/// ⚠ **The first draft of this test asserted the wrong thing and the failure
/// taught the mechanism**, which is worth keeping written down. It asserted
/// |stamp| falls monotonically outward; it does not, and cannot. `Haven::y` is
/// the raw height *at the site's own centre*, so at d = 0 the stamp is exactly
/// zero, rises as the raw ground diverges from that datum, and only then is
/// closed by the ramp. Monotone is a property of the **profile factor**
/// `1 - ramp(..)`, never of the metres it is multiplied by. Gating the metres
/// was gating the terrain.
///
/// The bound: smoothstep's derivative peaks at 1.5/band, the stamp is at most
/// `D = max|site_y - raw|` over the radial, and the samples are `δ` apart. So
/// the carve can add at most `D * 1.5 * δ / band` to any one step, and anything
/// beyond that is an edge the ground did not have.
#[test]
fn the_armed_carve_blends_without_an_edge() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (sx, sz, sy, fp) in carving_sites(&h) {
            let band = fp.blend_m - fp.stamp_m;
            // Four radials, so an asymmetric raw terrain cannot hide a step.
            for (ux, uz) in [(1.0f32, 0.0f32), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)] {
                let steps = 128i32;
                let delta = fp.blend_m / steps as f32;
                let at = |k: i32| {
                    let d = fp.blend_m * (k as f32 / steps as f32);
                    let (x, z) = (sx + ux * d, sz + uz * d);
                    let raw = terrain::height(seed, x, z);
                    (raw, carved(1.0, seed, &h, x, z))
                };
                let mut depth = 0.0f32;
                let mut worst_raw = 0.0f32;
                let mut worst_carved = 0.0f32;
                let mut prev = at(0);
                let mut k = 1i32;
                while k <= steps {
                    let cur = at(k);
                    depth = depth.max(fabs(sy - cur.0));
                    worst_raw = worst_raw.max(fabs(cur.0 - prev.0));
                    worst_carved = worst_carved.max(fabs(cur.1 - prev.1));
                    prev = cur;
                    k += 1;
                }
                let allowed = worst_raw + depth * 1.5 * delta / band + MM;
                assert!(
                    worst_carved <= allowed,
                    "seed {seed} site ({sx}, {sz}) bearing ({ux}, {uz}): the \
                     carved ground steps {worst_carved} m between samples \
                     {delta} m apart, against {allowed} m the raw ground and \
                     the blend's own slope can account for — that is an edge"
                );
                // And the band actually closes: at the mask the carve is gone.
                let (raw_end, carved_end) = at(steps);
                assert!(
                    fabs(carved_end - raw_end) < MM,
                    "seed {seed} site ({sx}, {sz}) bearing ({ux}, {uz}): the \
                     carve is still {} m deep at the end of its blend — the \
                     ramp does not close",
                    fabs(carved_end - raw_end)
                );
            }
        }
    }
}

// -------------------------------------------------------------- §E relief

/// The defect the carve fixes is real, measured on the ground a player stands
/// on — and this test is the one that corrected the claim it was written to
/// make.
///
/// **`Haven::relief` is not the number the carve drives to zero, and finding
/// that out is why this gate exists.** `relief` is the max−min over a rosette
/// at `HAVEN_RADIUS_M` = 16.0 m, which is exactly `HAVEN_FOOTPRINT.scatter_m` —
/// the outer edge of the band, where the stamp has faded to nothing by
/// construction (§C). So arming the carve leaves `relief` almost untouched, and
/// a plan that promised "3.76 m → 0" was quoting a measurement of a disc the
/// carve deliberately does not flatten. The pad's *floor* is `swept_m` =
/// `HAVEN_CRATE_R_M + CLUTTER_CELL_M` = 10.64 m; the 5.36 m beyond it is blend,
/// and it is supposed to still look like the hill the pad was cut into.
///
/// So the honest measurement is the relief **over the swept floor**, and it is
/// asserted both ways round: the found floor is genuinely uneven (or the carve
/// is machinery with no defect to fix, and this seam wants deleting rather than
/// arming), and the carved floor is flat.
#[test]
fn the_armed_carve_fixes_a_defect_that_is_really_there() {
    let mut worst_found = 0.0f32;
    let mut worst_carved = 0.0f32;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let fp = HAVEN_FOOTPRINT;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut rlo, mut rhi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut i = -16i32;
        while i <= 16 {
            let mut j = -16i32;
            while j <= 16 {
                let dx = fp.stamp_m * (i as f32 / 16.0);
                let dz = fp.stamp_m * (j as f32 / 16.0);
                if dx * dx + dz * dz > fp.stamp_m * fp.stamp_m {
                    j += 1;
                    continue;
                }
                let (x, z) = (h.x + dx, h.z + dz);
                let raw = terrain::height(seed, x, z);
                rlo = rlo.min(raw);
                rhi = rhi.max(raw);
                let g = carved(1.0, seed, &h, x, z);
                lo = lo.min(g);
                hi = hi.max(g);
                j += 1;
            }
            i += 1;
        }
        worst_found = worst_found.max(rhi - rlo);
        worst_carved = worst_carved.max(hi - lo);
    }
    assert!(
        worst_found > 0.5,
        "the found floors are already flat to {worst_found} m across 16 seeds — \
         the carve has no defect to fix, and this seam wants deleting rather \
         than arming"
    );
    assert!(
        worst_carved < MM,
        "the armed carve leaves {worst_carved} m of relief on the swept floor, \
         against {worst_found} m found — it does not do what stage 8 asked"
    );
}

// --------------------------------------------------------- §F structural

/// The stamp cannot read the terrain, and that is enforced by its signature
/// rather than by anyone remembering.
///
/// `haven(seed)` is built out of `height` taps, so a carve applied inside
/// `height` would have the site solver scoring ground it had already carved.
/// `site_stamp_with` takes `raw` and no seed, so it *cannot* call `height` —
/// but a later pass could add a seed parameter "just for the road stamp" and
/// re-open the whole hazard, with nothing failing. This is the grep that
/// notices, and it is a grep for the same reason `tests/sound.rs`'s is: the
/// defect would be a call site, not a value.
#[test]
fn the_stamp_cannot_read_the_terrain() {
    let src = include_str!("../src/terrain.rs");
    for name in ["fn site_stamp_with(", "fn stamp_of("] {
        let at = src
            .find(name)
            .unwrap_or_else(|| panic!("{name} has been renamed — re-aim this gate"));
        // Signature: up to the opening brace of the body.
        let sig_end = src[at..].find(" {").expect("a body") + at;
        let sig = &src[at..sig_end];
        assert!(
            !sig.contains("seed"),
            "{name} has grown a seed parameter. That re-opens the circularity \
             this split exists to close: with a seed in scope the stamp can \
             call height, and the site solver reads height. If a stamp really \
             needs terrain, it must be passed in already-sampled, as `raw` is."
        );
        // Body: to the next top-level `\n}`.
        let body_end = src[sig_end..].find("\n}").expect("a close") + sig_end;
        let body = &src[sig_end..body_end];
        assert!(
            !body.contains("height("),
            "{name} calls height() directly. See above — the stamp must be a \
             pure function of the site list and an already-sampled ground."
        );
    }
}

// -------------------------------------------------------------- §G footing

/// **The carve may never make an authored structure's footing worse.** This is
/// the invariant that found the `stamp_m` defect, and it is the one worth
/// keeping, because the failure it catches is invisible to every other check
/// here: §B flattens the floor, §C bounds the reach and §D smooths the band,
/// and a structure standing half on the floor and half on the band satisfies
/// all three while sitting on ground *steeper than the hill it replaced*.
///
/// The mechanism: inside the floor the ground is flat, and from the floor to
/// the mask the ramp compresses the entire raw delta into the band. A structure
/// whose footing crosses that boundary gets the compressed part. It is why the
/// carve's floor is derived from what the site SEATS (`stamp_m`) and not from
/// its container ring (`swept_m`) — the first draft used `swept_m` and made the
/// waystation canopy worse.
///
/// Measured at full strength over the 16 seeds, which is what the numbers in
/// `SiteFootprint::stamp_m`'s doc come from:
///   - haven shelter  1.374 m → 0.063 m (the carve doing its job)
///   - waystation canopy  1.795 m → 1.795 m (untouched: that site does not
///     carve at all today, and `terrain.rs`'s const block is what stops it
///     being armed in that state)
#[test]
fn the_armed_carve_never_worsens_a_structures_footing() {
    // (name, half-extent of the structure's own footing, per-seed anchors)
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let mut anchors: Vec<(&str, f32, f32, f32)> = Vec::new();
        let (sx, sz, _) = terrain::haven_shelter(&h);
        anchors.push(("haven shelter", terrain::SHELTER_CORNER_R_M, sx, sz));
        for w in 0..WAYSTATIONS {
            let ws = &h.minor[w];
            if !ws.live {
                continue;
            }
            let (kx, kz, _) = terrain::waystation_canopy(ws);
            anchors.push(("waystation canopy", terrain::WAYSTATION_CANOPY_R_M, kx, kz));
        }
        for (name, rad, cx, cz) in anchors {
            let (mut rlo, mut rhi) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut clo, mut chi) = (f32::INFINITY, f32::NEG_INFINITY);
            let mut i = -8i32;
            while i <= 8 {
                let mut j = -8i32;
                while j <= 8 {
                    let (dx, dz) = (rad * (i as f32 / 8.0), rad * (j as f32 / 8.0));
                    if dx * dx + dz * dz > rad * rad {
                        j += 1;
                        continue;
                    }
                    let (x, z) = (cx + dx, cz + dz);
                    let raw = terrain::height(seed, x, z);
                    let c = carved(1.0, seed, &h, x, z);
                    rlo = rlo.min(raw);
                    rhi = rhi.max(raw);
                    clo = clo.min(c);
                    chi = chi.max(c);
                    j += 1;
                }
                i += 1;
            }
            let (rawspread, carvedspread) = (rhi - rlo, chi - clo);
            assert!(
                carvedspread <= rawspread + MM,
                "seed {seed}: an armed carve leaves the {name} footed across \
                 {carvedspread} m where the raw ground gave {rawspread} m. The \
                 carve is making this structure's footing WORSE, which means its \
                 site's `stamp_m` does not reach past what the site seats and \
                 part of the structure is standing on the blend ramp."
            );
        }
    }
}

// ---------------------------------------------------------------- §H cliff

/// **The carve never builds a wall.** The gradient it ADDS to the island stays
/// under the share of the cliff budget it is allowed, everywhere it reaches.
///
/// This is the gate the arming pass was actually sized against, and the reason
/// the mechanism has a `max_cut` at all. The first armed draft ran the blend
/// out to the scatter mask, which gave it ~3.9 m of ramp to absorb whatever
/// height lay between the made floor and the hill around it — and over 128
/// seeds that produced a worst carved gradient of **2.09 rise/run at the
/// waystations**, against `CLIFF_SLOPE_RATIO` = 1.19. The carve was ringing
/// each destination with a ~64° wall.
///
/// ⚠ **It measures the gradient the carve ADDS, not the absolute one, and the
/// difference is the whole finding.** An earlier draft asserted the absolute
/// carved gradient against the cliff and read 1.16 at blend lengths from 6 m to
/// 32 m — flat, and suspiciously so. It was measuring the *island*: the raw
/// ground inside the window is itself up to 0.72 rise/run, and no amount of
/// ramp changes that. Subtracting the raw gradient at the same point is what
/// separates "this site sits on a hillside" (fine, and none of the carve's
/// business) from "the carve made a step" (the defect).
///
/// ⚠ **What this gate can and cannot fail, stated so nobody over-trusts it.**
/// `max_cut` is derived from the same band the ramp runs over, so the cut and
/// the slack move together: shortening `SITE_BLEND_M` shortens the ramp AND
/// shallows the cut, and the worst gradient stays pinned just under budget.
/// Verified — halving the blend does not fail this test, and neither does
/// raising `SITE_CUT_HEADROOM`, which lifts the bar along with the cut. So it
/// is **not** a check that the constants are well chosen; §A pins those. It is
/// a check that the CLAMP still matches its RAMP, and it fails the moment the
/// two come apart — proven by deleting the clamp, which puts seed
/// 5_513_890_487_717_207 at 0.610 against a 0.596 budget.
#[test]
fn the_armed_carve_never_builds_a_wall() {
    let budget = terrain::CLIFF_SLOPE_RATIO * terrain::SITE_CUT_HEADROOM;
    let mut worst = 0.0f32;
    // ⚠ **Not `SEEDS`.** The 16-seed suite list does not exercise this at all:
    // its sites are flat enough that `max_cut` never binds, so the clamp this
    // gate exists to check is dead code on every one of them — the first draft
    // of this test passed with the cut budget raised 6x. These five are the
    // roughest of a 256-seed sweep, the ones where the clamp actually engages,
    // and on them the carve lands within a thousandth of its budget. Keeping a
    // narrow list is the point: a property that only shows up on rough ground
    // needs seeds picked FOR being rough, not the seeds everything else uses.
    for seed in ROUGH_SEEDS.iter().copied().chain(SEEDS) {
        let h = terrain::haven(seed);
        for (sx, sz, _, fp) in carving_sites(&h) {
            for b in 0..32 {
                // A fixed radial fan; `yaw_lut` is private to the crate, and a
                // wall-1 trig ban does not reach a test binary but the habit is
                // cheap — these are exact eighths and sixteenths of a turn.
                let t = b as f32 / 32.0;
                let (ux, uz) = (1.0 - 2.0 * t, {
                    let v = 1.0 - (1.0 - 2.0 * t) * (1.0 - 2.0 * t);
                    if b % 2 == 0 {
                        v.sqrt()
                    } else {
                        -v.sqrt()
                    }
                });
                let mut d = fp.stamp_m - 2.0;
                while d < fp.blend_m + 2.0 {
                    let p = |r: f32| (sx + ux * r, sz + uz * r);
                    let (x0, z0) = p(d);
                    let (x1, z1) = p(d + 1.0);
                    let raw = terrain::height(seed, x1, z1) - terrain::height(seed, x0, z0);
                    let cv = terrain::ground(seed, &h, x1, z1) - terrain::ground(seed, &h, x0, z0);
                    // What the carve put there that the island had not.
                    let added = fabs(cv) - fabs(raw);
                    worst = worst.max(added);
                    assert!(
                        added <= budget + MM,
                        "seed {seed} site ({sx}, {sz}) at d={d}: the carve adds \
                         {added} rise/run where its budget is {budget} \
                         (CLIFF_SLOPE_RATIO x SITE_CUT_HEADROOM). That is a wall, \
                         and it means max_cut is letting through a deeper cut \
                         than SITE_BLEND_M's ramp can carry."
                    );
                    d += 0.25;
                }
            }
        }
    }
    assert!(
        worst > 0.05,
        "the carve adds at most {worst} rise/run anywhere — it is doing nothing, \
         and every other check in this file is passing vacuously"
    );
}
