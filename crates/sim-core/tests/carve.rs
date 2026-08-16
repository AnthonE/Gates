//! The site carve — `TERRAIN.md` §1 stage 8's "carve a flat pad with a smooth
//! blend radius", and the seam that carries it.
//!
//! **This gate exists because the carve ships dark.** `SITE_STAMP_STRENGTH` is
//! 0.0, so every consumer call site was converted this pass and no ground
//! moved; that is the whole point — a cross-cutting edit and a behaviour change
//! are two commits, not one. But a dark constant makes a gate lie by
//! construction: at strength 0.0 every assertion reachable through `site_stamp`
//! is satisfied by a function that returns zero, so a gate written only against
//! the shipped path would prove that zero is zero and the arming pass would be
//! the first time the arithmetic ever ran. Hence `site_stamp_with`: §B–§E drive
//! the real code path at full depth, and the shipped path differs from what
//! they exercise by exactly one constant.
//!
//! What this file holds, in the order the mechanism reads:
//!   §A the seam is dark, and `ground` is `height`'s own bits while it is
//!   §B the carve flattens the floor it claims to flatten
//!   §C it never reaches outside the footprint that publishes it
//!   §D the blend across the band has no edge in it
//!   §E the relief the pad settled for by *finding* goes to zero by *making*
//!   §F the stamp cannot read the terrain — the circularity is structural
//!   §G the carve never makes an authored structure's footing WORSE

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

/// The live sites whose footprint actually carves — floor strictly inside mask.
///
/// Today that is the haven and not the waystations (`stamp_m` 11.10 against an
/// 11.0 m mask, §F), which is a stated blocked state rather than an oversight:
/// the const block in `terrain.rs` refuses to compile an armed carve while it
/// holds. When the operator widens `WAYSTATION_RADIUS_M` this list grows and
/// every check below starts covering the second tier with no edit here.
fn carving_sites(h: &Haven) -> Vec<(f32, f32, f32, terrain::SiteFootprint)> {
    sites(h)
        .into_iter()
        .filter(|(_, _, _, fp)| fp.stamp_m < fp.scatter_m)
        .collect()
}

/// The carved ground at a named strength — `terrain::ground`'s body with the
/// constant lifted out, which is the one thing a test may reimplement here
/// because the arithmetic under test is `site_stamp_with`'s and not this sum.
fn carved(strength: f32, seed: u64, h: &Haven, x: f32, z: f32) -> f32 {
    let raw = terrain::height(seed, x, z);
    raw + terrain::site_stamp_with(strength, h, raw, x, z)
}

// ---------------------------------------------------------------- §A dark

/// The shipped constant is zero, stated here so that arming it is a deliberate
/// edit to a gate and not a quiet edit to a number.
///
/// When this fails because someone armed the carve, that is the arming pass
/// doing its job: update this assert, regenerate the terrain goldens (the
/// operator's call, `DECISIONS.md` 2026-08-10, "a worldgen change is a wipe"),
/// and §E stops being a hypothetical.
#[test]
fn the_carve_ships_dark() {
    assert_eq!(
        SITE_STAMP_STRENGTH, 0.0,
        "SITE_STAMP_STRENGTH is no longer zero — the carve has been armed. \
         That is a worldgen change: it moves test_terrain_golden and \
         test_replay, and it is an operator call (DECISIONS.md §open, \
         'site carve v0'). Update this gate in the same commit."
    );
}

/// While the carve is dark, `ground` returns `height`'s own bits — not a value
/// that rounds to it, the bits.
///
/// This is what makes the seam landable without touching a golden, and it is
/// asserted **inside the site footprints too**, because that is the only place
/// a stamp could ever be non-zero and therefore the only place the claim has
/// any content. `to_bits` rather than `==` so a `-0.0` for a `0.0` is caught:
/// `ground`'s early return exists precisely so no worldgen height is ever put
/// through a `+ 0.0` that could re-sign it.
#[test]
fn ground_is_height_to_the_bit_while_the_carve_is_dark() {
    assert_eq!(SITE_STAMP_STRENGTH, 0.0, "see the_carve_ships_dark");
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (sx, sz, _, fp) in sites(&h) {
            // A grid across the whole footprint and a margin past it.
            let r = fp.scatter_m + 4.0;
            let mut i = -32i32;
            while i <= 32 {
                let mut j = -32i32;
                while j <= 32 {
                    let x = sx + r * (i as f32 / 32.0);
                    let z = sz + r * (j as f32 / 32.0);
                    let raw = terrain::height(seed, x, z);
                    let g = terrain::ground(seed, &h, x, z);
                    assert_eq!(
                        g.to_bits(),
                        raw.to_bits(),
                        "seed {seed} at ({x}, {z}): ground {g} != height {raw} \
                         with the carve dark"
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
                    (x - sx) * (x - sx) + (z - sz) * (z - sz) < fp.scatter_m * fp.scatter_m
                });
                if !inside {
                    let raw = terrain::height(seed, x, z);
                    let g = carved(1.0, seed, &h, x, z);
                    assert_eq!(
                        g.to_bits(),
                        raw.to_bits(),
                        "seed {seed} at ({x}, {z}): the armed carve moved ground \
                         outside every footprint"
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
            let band = fp.scatter_m - fp.stamp_m;
            // Four radials, so an asymmetric raw terrain cannot hide a step.
            for (ux, uz) in [(1.0f32, 0.0f32), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)] {
                let steps = 128i32;
                let delta = fp.scatter_m / steps as f32;
                let at = |k: i32| {
                    let d = fp.scatter_m * (k as f32 / steps as f32);
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
                     carve is still {} m deep at the scatter mask — the band \
                     does not close",
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
