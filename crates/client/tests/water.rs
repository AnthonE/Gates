//! The sea's gate. **Renderer tier**: it links Bevy for `Image` and `Vec3`,
//! but it opens no window, needs no GPU and reads no pixel.
//!
//! `CLAUDE.md` is explicit that there is no visual gate here and that writing
//! one is forbidden — a pixel statistic cannot see whether the frame is a
//! picture of anything, and this repo has the beige smear to prove it. What
//! *may* be gated about a frame is arithmetic: that the mesh fits the volume
//! the sim blocks, in Rust, in the shape of `tests/tree.rs`. So nothing below
//! asks whether the water looks good. Every assertion is a property that,
//! violated, produces a specific named artefact — a sea that aliases into a
//! crawling ghost wave, that climbs the beach, that tiles with a seam, or that
//! is lit like polished plastic.
//!
//! The person who decides whether it looks good boots the game and looks.
//!
//! **`assertions_on_constants` is allowed here on purpose.** Most of this file
//! asserts relations *between* knobs — that green outlives red in the
//! extinction table, that the reflectance and the IOR agree — and clippy is
//! right that those fold at compile time. That is the point: a knob gate is a
//! statement about values somebody will edit, and a relation that folds to
//! `true` today is exactly the one that stops folding when they do.

#![allow(clippy::assertions_on_constants)]

use bevy::math::Vec2;
use client::render::terrain_mesh::{self, WET_BAND_M, WET_REACH_M, WET_VALUE};
use client::render::water::*;
use sim_core::terrain::{ISLAND_SIZE, SEA_LEVEL};
use sim_core::weather;

// ---------------------------------------------------------------------------
// The wave set.
// ---------------------------------------------------------------------------

/// Directions are unit vectors or every wave's speed and steepness is wrong by
/// its own length — and silently, because a 3% short direction just makes a
/// slightly slower wave.
#[test]
fn wave_directions_are_unit() {
    for (i, w) in WAVES.iter().enumerate() {
        let l = (w.dir[0] * w.dir[0] + w.dir[1] * w.dir[1]).sqrt();
        assert!(
            (l - 1.0).abs() < 1e-3,
            "wave {i}'s direction is {l} long, not 1"
        );
    }
}

/// **The swell must not break.** A wave whose maximum slope reaches 1 has a
/// vertical face; past it a height field folds through itself and the surface
/// self-intersects. The sum over the whole set is what binds, not each wave —
/// four gentle waves that crest together are one steep one.
///
/// `Σ A·k` is the classic Gerstner steepness limit; [`SHAPE_MAX_SLOPE`] is
/// what the crest sharpening multiplies it by, and leaving that factor out is
/// how the same arithmetic ends up 30% optimistic.
#[test]
fn the_swell_cannot_break() {
    let mut slope = 0.0f32;
    for w in WAVES {
        slope += w.amp_m * (std::f32::consts::TAU / w.len_m);
    }
    let peak = slope * SHAPE_MAX_SLOPE;
    assert!(
        peak < 1.0,
        "the swell's peak slope is {peak} - at or past 1 the surface folds through itself"
    );
    // And it must actually be a sea rather than a puddle: if this is near zero
    // the assertion above is passing on nothing.
    assert!(peak > 0.05, "the swell's peak slope is {peak} - it is flat");
}

/// The core must be fine enough to carry every wave in the set. A wave the
/// mesh cannot sample is not a subtle wave: [`band_weight`] retires it, so it
/// is a wave that is simply absent everywhere, in a table that says it is
/// there.
#[test]
fn the_core_carries_every_wave() {
    for (i, w) in WAVES.iter().enumerate() {
        assert!(
            band_weight(STEP_M, w.len_m) > 0.99,
            "wave {i} at {} m is retired by the {STEP_M} m core - it never draws",
            w.len_m
        );
    }
}

/// The mechanism that keeps the far sea from aliasing: past half a wavelength
/// of spacing a wave is gone, and it goes smoothly.
#[test]
fn a_wave_retires_before_its_own_nyquist() {
    let len = 20.0f32;
    assert_eq!(band_weight(len * 0.5, len), 0.0);
    assert_eq!(band_weight(len * 0.6, len), 0.0);
    assert_eq!(band_weight(len * 0.25, len), 1.0);
    assert_eq!(band_weight(1.0, len), 1.0);
    // Monotone across the transition, with no step at either end — a step is a
    // ring of water that visibly stops moving.
    let mut prev = 1.1f32;
    for i in 0..=40 {
        let s = len * (0.25 + 0.25 * i as f32 / 40.0);
        let w = band_weight(s, len);
        assert!(w <= prev + 1e-6, "band weight rose at spacing {s}");
        prev = w;
    }
}

/// Every vertex of the mesh actually satisfies the rule above — the coordinate
/// table and the wave set are two independent knobs and nothing else checks
/// that they agree.
#[test]
fn every_vertex_respects_the_nyquist_rule() {
    let coords = axis_coords();
    let spacing = axis_spacing(&coords);
    for (i, s) in spacing.iter().enumerate() {
        for w in WAVES {
            if *s >= w.len_m * 0.5 {
                assert_eq!(
                    band_weight(*s, w.len_m),
                    0.0,
                    "coordinate {i} at {} m has {s} m spacing and still draws a {} m wave",
                    coords[i],
                    w.len_m
                );
            }
        }
    }
}

/// The grid must reach past the island from anywhere on it, or a player on one
/// shore sees the sea end before the far one does.
#[test]
fn the_grid_reaches_past_the_island() {
    let coords = axis_coords();
    let far = *coords.last().unwrap();
    assert!(
        far >= ISLAND_SIZE,
        "the sea reaches {far} m and the island is {ISLAND_SIZE} m across"
    );
    assert_eq!(coords[0], -far, "the coordinate table is not symmetric");
    // Ascending, or the triangle winding flips somewhere in the middle and
    // half the sea faces away.
    for w in coords.windows(2) {
        assert!(w[1] > w[0], "the coordinate table is not ascending");
    }
    // And the cost has to stay a cost: this is the one number that decides how
    // much work `animate` does every single frame.
    let verts = coords.len() * coords.len();
    assert!(
        verts < 12_000,
        "the sea is {verts} vertices - that is a per-frame budget, not a mesh"
    );
}

/// The core is uniform and the skirt only ever coarsens. A skirt step that
/// went *finer* would put a dense ring in the far field, which costs vertices
/// where nothing can see them.
#[test]
fn the_skirt_only_coarsens() {
    let coords = axis_coords();
    let mid = coords.len() / 2;
    let mut last = 0.0f32;
    for i in mid..coords.len() - 1 {
        let gap = coords[i + 1] - coords[i];
        assert!(
            gap >= last - 1e-3,
            "the skirt narrows from {last} to {gap} at coordinate {i}"
        );
        last = gap;
    }
}

// ---------------------------------------------------------------------------
// The surface.
// ---------------------------------------------------------------------------

const PHASE: [f32; WAVE_COUNT] = [0.3, 1.1, 2.4, 5.0];
/// The clear sea's gains: [`WAVES`] as authored.
const CALM: [f32; WAVE_COUNT] = SeaState::CALM.swell;

/// The gradient must be the height field's own, or the specular disagrees with
/// the silhouette and the sea reads as a lit sheet of plastic laid over a wavy
/// one. Finite differences at fixed weights, which is the property
/// `wave_field` actually claims (see its note on locally-constant weights).
#[test]
fn the_normal_agrees_with_the_height_field() {
    let e = 0.01f32;
    for (x, z) in [(0.0, 0.0), (13.7, -22.1), (-101.5, 47.0), (3.3, 3.3)] {
        let (_, gx, gz) = wave_field(x, z, &PHASE, &CALM, STEP_M, 1.0);
        let (hx1, _, _) = wave_field(x + e, z, &PHASE, &CALM, STEP_M, 1.0);
        let (hx0, _, _) = wave_field(x - e, z, &PHASE, &CALM, STEP_M, 1.0);
        let (hz1, _, _) = wave_field(x, z + e, &PHASE, &CALM, STEP_M, 1.0);
        let (hz0, _, _) = wave_field(x, z - e, &PHASE, &CALM, STEP_M, 1.0);
        let fd_x = (hx1 - hx0) / (2.0 * e);
        let fd_z = (hz1 - hz0) / (2.0 * e);
        assert!(
            (gx - fd_x).abs() < 5e-3 && (gz - fd_z).abs() < 5e-3,
            "at ({x}, {z}) the analytic gradient ({gx}, {gz}) disagrees with \
             finite differences ({fd_x}, {fd_z})"
        );
    }
    // And the normal it makes is a normal.
    let (_, gx, gz) = wave_field(9.0, -4.0, &PHASE, &CALM, STEP_M, 1.0);
    let n = wave_normal(gx, gz);
    assert!((n.length() - 1.0).abs() < 1e-5);
    assert!(n.y > 0.0, "the surface normal points down");
}

/// The crest-sharpened sine has zero mean, and that is not cosmetic: a DC
/// offset here moves the whole sea off `SEA_LEVEL`, so the drawn waterline and
/// the sim's own would sit at different heights.
#[test]
fn the_wave_shape_has_no_dc_offset() {
    let n = 4096;
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for i in 0..n {
        let theta = std::f32::consts::TAU * i as f32 / n as f32;
        let (s, _) = shape(theta);
        sum += s as f64;
        peak = peak.max(s.abs());
    }
    let mean = (sum / n as f64) as f32;
    assert!(
        mean.abs() < 1e-3,
        "the wave shape has a DC offset of {mean}"
    );
    assert!(
        (peak - SHAPE_PEAK).abs() < 1e-3,
        "the wave shape peaks at {peak}, not the documented {SHAPE_PEAK}"
    );
}

/// `shape`'s derivative is its own, checked the same way the field's is —
/// `SHAPE_MAX_SLOPE` is a documented constant that the breaking gate rests on.
#[test]
fn the_wave_shape_slope_is_what_it_claims() {
    let n = 8192;
    let mut peak = 0.0f32;
    for i in 0..n {
        let theta = std::f32::consts::TAU * i as f32 / n as f32;
        let (_, ds) = shape(theta);
        peak = peak.max(ds.abs());
        let e = 1e-3;
        let fd = (shape(theta + e).0 - shape(theta - e).0) / (2.0 * e);
        assert!((ds - fd).abs() < 5e-3, "shape' disagrees at {theta}");
    }
    assert!(
        (peak - SHAPE_MAX_SLOPE).abs() < 1e-2,
        "shape' peaks at {peak}, not the documented {SHAPE_MAX_SLOPE}"
    );
}

/// **The sea must be exactly flat at the waterline.** Any amplitude left at
/// zero depth lifts the surface above the sand it is meeting, and the water
/// visibly climbs the beach and sits there — the artefact `shoal` exists to
/// prevent, and the reason the reference needed two simulations.
#[test]
fn the_swell_dies_at_the_waterline() {
    assert_eq!(shoal(0.0), 0.0);
    assert_eq!(shoal(-3.0), 0.0);
    assert_eq!(shoal(SHOAL_FULL_M), 1.0);
    let (h, gx, gz) = wave_field(20.0, 20.0, &PHASE, &CALM, STEP_M, shoal(0.0));
    assert_eq!((h, gx, gz), (0.0, 0.0, 0.0));
    // Monotone in between, so the shore is a ramp and not a step.
    let mut prev = -1.0f32;
    for i in 0..=20 {
        let d = SHOAL_FULL_M * i as f32 / 20.0;
        let s = shoal(d);
        assert!(s >= prev - 1e-6, "shoaling fell at depth {d}");
        prev = s;
    }
}

// ---------------------------------------------------------------------------
// The sea state.
// ---------------------------------------------------------------------------

/// A preset's wind as the client reads it (`render/weather.rs`'s `pm`).
fn wind_of(preset: u8) -> f32 {
    weather::preset(preset, 0).wind as f32 * 0.001
}

/// **A clear day draws the authored sea, bit for bit**, and every gate on the
/// wave set above rests on it: they measure [`WAVES`], and `WAVES` is what a
/// player sees only while the clear preset's wind raises nothing. Retune the
/// clear wind in `sim_core::weather` and this goes red, rather than the clear
/// sea quietly drawing a little rough under gates that still pass.
#[test]
fn a_clear_day_draws_the_authored_sea() {
    assert!(
        (wind_of(weather::CLEAR) - CALM_WIND).abs() < 1e-6,
        "the clear preset blows {} and the calm sea is drawn at {CALM_WIND}",
        wind_of(weather::CLEAR)
    );
    assert_eq!(SeaState::of_wind(wind_of(weather::CLEAR)), SeaState::CALM);
    // Fog is stiller than clear, and the sea goes no flatter than authored.
    assert_eq!(SeaState::of_wind(wind_of(weather::FOG)), SeaState::CALM);
    assert_eq!(SeaState::of_wind(0.0), SeaState::CALM);
    assert_eq!(SeaState::default(), SeaState::CALM);
    // And a storm is the whole storm table.
    let storm = SeaState::of_wind(wind_of(weather::STORM));
    assert_eq!(storm.swell, STORM_GAIN);
    assert_eq!(storm.surf, STORM_SURF_GAIN);
}

/// The sea rises with the wind through every preset, every wave with it, and
/// without a step: the weather fades its wind (20 s for an admin's
/// `/weather`, 15 min on the schedule), and a step in the curve is a sea that
/// jumps mid-fade.
#[test]
fn the_sea_rises_with_the_wind() {
    let mut presets: Vec<u8> = (weather::CLEAR..=weather::PRESET_MAX).collect();
    presets.sort_by_key(|p| weather::preset(*p, 0).wind);
    let mut prev = SeaState::CALM;
    for p in presets {
        let s = SeaState::of_wind(wind_of(p));
        for i in 0..WAVE_COUNT {
            assert!(
                s.swell[i] >= prev.swell[i],
                "wave {i} is lower under {} than under a stiller sky",
                weather::preset_name(p)
            );
        }
        assert!(
            s.surf >= prev.surf,
            "the breakers fell under {}",
            weather::preset_name(p)
        );
        prev = s;
    }
    // Rain is rougher than clear, heavy rain than mild, a storm than both.
    let mild = SeaState::of_wind(wind_of(weather::RAIN_MILD)).swell[0];
    let heavy = SeaState::of_wind(wind_of(weather::RAIN_HEAVY)).swell[0];
    assert!(1.0 < mild && mild < heavy && heavy < STORM_GAIN[0]);
    let mut last = sea_state(0.0);
    for i in 1..=1000 {
        let s = sea_state(i as f32 / 1000.0);
        assert!(
            s >= last && s - last < 0.01,
            "the sea state steps at wind {i}‰"
        );
        last = s;
    }
    assert_eq!(sea_state(1.0), 1.0);
}

/// **A storm is held to the clear sea's own limits.** The swell's summed
/// steepness under the storm gains is the Gerstner bound
/// `the_swell_cannot_break` holds the authored set to, and the shoaling
/// still takes every wave to nothing at the waterline — a storm that climbed
/// the beach would be `shoal`'s artefact back, taller.
///
/// The gains also have to be the shape the storm table claims: none under
/// one (a storm that calmed a wave), and longest most, which is what makes a
/// storm a different sea rather than the clear one drawn taller.
#[test]
fn a_storm_cannot_break_the_swell() {
    let storm = SeaState::at(1.0);
    let mut slope = 0.0f32;
    for (w, g) in WAVES.iter().zip(storm.swell) {
        slope += w.amp_m * g * (std::f32::consts::TAU / w.len_m);
    }
    let peak = slope * SHAPE_MAX_SLOPE;
    assert!(
        peak < 1.0,
        "a storm swell's peak slope is {peak} - at or past 1 the surface folds through itself"
    );
    assert_eq!(
        wave_field(20.0, 20.0, &PHASE, &storm.swell, STEP_M, shoal(0.0)),
        (0.0, 0.0, 0.0),
        "a storm swell stands on the waterline"
    );
    assert_eq!(surf_envelope(0.0, 0.0, 1.0) * storm.surf, 0.0);

    for w in WAVES.windows(2) {
        assert!(w[0].len_m > w[1].len_m, "WAVES is no longer longest first");
    }
    for i in 0..WAVE_COUNT {
        assert!(STORM_GAIN[i] >= 1.0, "a storm lowers wave {i}");
        if i > 0 {
            assert!(
                STORM_GAIN[i] <= STORM_GAIN[i - 1],
                "a storm raises wave {i} more than the longer wave {}",
                i - 1
            );
        }
    }
    assert!(STORM_SURF_GAIN >= 1.0);
}

/// **A storm trough never bares the seabed.** Every wave and the breaker on
/// top troughing together, at the storm's gains, must stay above the bed at
/// every depth the shoaling and the surf reach — a trough deeper than the
/// water is a dry hole walking across the shallows a metre offshore, where
/// no real wave draws back. The bound is the worst case (all troughs at
/// once), so it holds whatever the phases.
#[test]
fn a_storm_trough_never_bares_the_seabed() {
    let trough = (0..4096)
        .map(|i| shape(std::f32::consts::TAU * i as f32 / 4096.0).0)
        .fold(0.0f32, f32::min)
        .abs();
    let storm = SeaState::at(1.0);
    let swell: f32 = WAVES
        .iter()
        .zip(storm.swell)
        .map(|(w, g)| w.amp_m * g)
        .sum();
    let reach = SURF_FADE_TO_M.max(SHOAL_FULL_M) * 1.5;
    for i in 1..=2000 {
        let d = reach * i as f32 / 2000.0;
        let worst = trough * (swell * shoal(d) + surf_envelope(d, 0.0, 1.0) * storm.surf);
        assert!(
            worst < d,
            "at {d} m of water a storm trough reaches {worst} m down - the seabed shows through"
        );
    }
}

/// **A storm whitens the swell, and a breeze keeps its scatter.**
/// `crest_foam` measures every sea against the clear sea's crest reach, so
/// the clear sea's caps are exactly the old ones and a storm's crests, which
/// pass that reach, cap far more of the surface — but not all of it: a storm
/// is a streaked sea, and one capped everywhere is a white sheet.
#[test]
fn a_storm_caps_more_of_the_swell_than_a_breeze() {
    let reach: f32 = WAVES.iter().map(|w| w.amp_m).sum::<f32>() * SHAPE_PEAK;
    let capped = |s: &SeaState| {
        let mut n = 0usize;
        for iz in 0..64 {
            for ix in 0..64 {
                let (h, _, _) = wave_field(
                    ix as f32 * 3.1,
                    iz as f32 * 2.7,
                    &PHASE,
                    &s.swell,
                    STEP_M,
                    1.0,
                );
                if crest_foam(h, reach) > 0.0 {
                    n += 1;
                }
            }
        }
        n as f32 / (64.0 * 64.0)
    };
    let calm = capped(&SeaState::CALM);
    let storm = capped(&SeaState::at(1.0));
    assert!(calm > 0.0, "the clear sea has no whitecaps at all");
    assert!(
        storm > 3.0 * calm,
        "a storm caps {storm} of the swell against a breeze's {calm}"
    );
    assert!(
        storm < 0.5,
        "a storm caps {storm} of the swell - a white sheet"
    );
}

// ---------------------------------------------------------------------------
// The optics.
// ---------------------------------------------------------------------------

/// Both outputs come from one transmittance and both must grade with depth: a
/// shoreline whose alpha ramp and colour ramp disagree about where the
/// shallows end has two visible waterlines.
///
/// **The colour goes UP with depth, and that is the correction the first
/// capture forced.** What the water sends you is scatter, and scatter
/// accumulates; a model where depth darkens the body is treating extinction as
/// if it applied to the water's own light, and it renders a grey sheet.
#[test]
fn depth_grades_colour_and_alpha_together() {
    let mut prev = depth_tint(0.0);
    assert_eq!(prev[3], 0.0, "the waterline is not fully transparent");
    assert_eq!(
        [prev[0], prev[1], prev[2]],
        [0.0; 3],
        "water of zero depth is still contributing colour"
    );
    for i in 1..=60 {
        let d = i as f32 * 0.5;
        let t = depth_tint(d);
        assert!(t[3] >= prev[3] - 1e-6, "alpha fell at {d} m");
        assert!(t[3] <= ALPHA_MAX + 1e-6, "alpha passed ALPHA_MAX at {d} m");
        for c in 0..3 {
            assert!(t[c] >= prev[c] - 1e-6, "channel {c} lost scatter at {d} m");
            assert!(
                t[c] <= SCATTER_ALBEDO[c] + 1e-6,
                "channel {c} scattered back more than it took out, at {d} m"
            );
        }
        prev = t;
    }
    // Deep water must actually be deep-looking by the time the seabed is at
    // its own floor: `terrain::SEA_FLOOR_DEPTH` is 12 m.
    assert!(
        depth_tint(12.0)[3] > 0.8,
        "the open sea is still see-through over a 12 m floor"
    );
}

/// The sea is Atlantic, not Caribbean — the reference's own stated retune
/// (`reference/WATER.md` §2). Green must reach at least as far as blue and
/// come back strongest, which is what coastal water does and what tropical
/// water does not.
#[test]
fn the_sea_is_a_coastal_green_not_a_tropical_blue() {
    assert!(
        EXTINCT[1] <= EXTINCT[2],
        "blue penetrates further than green - that is open ocean, not a coast"
    );
    assert!(
        EXTINCT[0] > EXTINCT[1] * 2.0,
        "red is not being eaten fast enough for this to be water"
    );
    assert!(
        SCATTER_ALBEDO[1] > SCATTER_ALBEDO[2] && SCATTER_ALBEDO[2] > SCATTER_ALBEDO[0],
        "the scattering albedo is not green > blue > red"
    );
    // And the deep sea has to be a colour at all, not a dark grey: the whole
    // reason the first cut was replaced.
    let deep = depth_tint(12.0);
    let luma = 0.2126 * deep[0] + 0.7152 * deep[1] + 0.0722 * deep[2];
    assert!(
        luma > 0.03,
        "the deep sea's own colour is {luma} - it is black"
    );
    assert!(
        deep[1] > deep[0] * 3.0,
        "the deep sea is grey: {:?}",
        &deep[..3]
    );
}

/// Transmittance is a transmittance: 1 at the surface, monotone, and it never
/// leaves [0, 1] however deep the sentinel goes.
#[test]
fn transmittance_is_bounded_and_monotone() {
    let t0 = transmittance(0.0);
    for v in t0 {
        assert!((v - 1.0).abs() < 1e-6);
    }
    let mut prev = t0;
    for i in 1..=200 {
        let d = i as f32 * 0.5;
        let t = transmittance(d);
        for c in 0..3 {
            assert!(
                t[c] <= prev[c] + 1e-9 && t[c] >= 0.0,
                "channel {c} at {d} m"
            );
        }
        prev = t;
    }
    let far = transmittance(DEEP_SENTINEL_M);
    for (c, v) in far.iter().enumerate() {
        assert!(
            *v < 1e-3,
            "the sentinel depth does not saturate channel {c}"
        );
    }
    // Negative depth is dry land, not a light source.
    assert_eq!(transmittance(-5.0), t0);
}

/// Foam is a *shore* effect weighted by the LAND's slope — the reference's own
/// published observation about its ocean foam, and the reason this function
/// takes a terrain slope at all.
#[test]
fn foam_is_a_shore_effect_keyed_to_the_land() {
    let mid = 0.5f32; // the noise's neutral value: no contour displacement
    assert_eq!(shore_foam(FOAM_DEPTH_M, 1.0, mid), 0.0);
    assert_eq!(shore_foam(20.0, 1.0, mid), 0.0);
    let flat = shore_foam(FOAM_PEAK_M, 0.0, mid);
    let steep = shore_foam(FOAM_PEAK_M, FOAM_SLOPE_FULL, mid);
    assert!(flat > 0.0, "a flat shore gets no foam at all");
    assert!(steep > flat, "a steep shore is not foamier than a flat one");
    assert!(steep <= 1.0, "foam is over-unity at {steep}");
    // Whitecaps are a fraction of a crest, not a paint bucket.
    assert_eq!(crest_foam(0.0, 1.0), 0.0);
    assert_eq!(crest_foam(1.0, 0.0), 0.0);
    assert!(crest_foam(1.0, 1.0) <= CREST_FOAM_MAX + 1e-6);
    assert!(crest_foam(0.5, 1.0) < crest_foam(0.95, 1.0));
}

/// **The wash stands OFF the waterline, and this is the assertion that says
/// the shore is not a drawn line.** The first cut peaked foam at zero depth,
/// which put the brightest thing on the sea exactly along the polygon edge
/// where water meets sand — outlining the seam instead of hiding it.
#[test]
fn the_wash_does_not_outline_the_waterline() {
    let mid = 0.5f32;
    assert_eq!(
        shore_foam(0.0, 0.4, mid),
        0.0,
        "there is foam at zero depth - the waterline is being outlined"
    );
    // Rises from the edge to the peak...
    let mut prev = -1.0f32;
    for i in 0..=10 {
        let d = FOAM_PEAK_M * i as f32 / 10.0;
        let f = shore_foam(d, 0.4, mid);
        assert!(f >= prev - 1e-6, "the wash fell on the way out at {d} m");
        prev = f;
    }
    // ...and falls from the peak to the outer edge.
    let peak = shore_foam(FOAM_PEAK_M, 0.4, mid);
    let mut prev = peak + 1.0;
    for i in 0..=10 {
        let d = FOAM_PEAK_M + (FOAM_DEPTH_M - FOAM_PEAK_M) * i as f32 / 10.0;
        let f = shore_foam(d, 0.4, mid);
        assert!(f <= prev + 1e-6, "the wash rose again at {d} m");
        prev = f;
    }
    assert!(peak > 0.0);
    // The band is wide enough to be a band. A wash two vertices across on a
    // 2 m grid is a stripe, which is the thing this is replacing.
    assert!(
        FOAM_DEPTH_M > STEP_M,
        "the wash is narrower than the mesh that draws it"
    );
}

/// The band's contour is displaced by noise, so its edges are lobes rather
/// than iso-depth curves of a smooth heightfield — which the eye reads as
/// drafting.
#[test]
fn the_wash_has_no_clean_contour() {
    // The noise actually moves the band: a depth outside the plain band is
    // inside the displaced one, and vice versa.
    let just_out = FOAM_DEPTH_M - 0.05;
    assert_eq!(
        shore_foam(just_out, 0.4, 1.0),
        0.0,
        "the jitter cannot pull in"
    );
    assert!(
        shore_foam(just_out, 0.4, 0.0) > 0.0,
        "the jitter cannot push out"
    );
    // And it reaches over the waterline the other way, so the wash runs onto
    // ground the plain band would have left dry.
    assert!(
        shore_foam(0.1, 0.4, 1.0) > 0.0,
        "the jitter never carries the wash inshore"
    );

    // The field itself: bounded, varying, and continuous.
    let mut lo = f32::MAX;
    let mut hi = 0.0f32;
    let mut prev = foam_noise(0.0, 0.0);
    for i in 0..400 {
        let x = i as f32 * 0.25;
        let n = foam_noise(x, 12.5);
        assert!(
            (0.0..=1.0).contains(&n),
            "foam noise left [0,1] at {x}: {n}"
        );
        assert!(
            (n - prev).abs() < 0.35,
            "foam noise stepped by {} at {x} - that is a seam, not a field",
            (n - prev).abs()
        );
        lo = lo.min(n);
        hi = hi.max(n);
        prev = n;
    }
    assert!(
        hi - lo > 0.35,
        "foam noise only spans {} - it is flat",
        hi - lo
    );
    // Deterministic: the wash may not crawl when the grid re-centres.
    assert_eq!(foam_noise(37.5, -9.25), foam_noise(37.5, -9.25));
}

/// The wash runs up and draws back. **A moving edge cannot be a hard edge** —
/// an observer reads a boundary that breathes as a process and one that does
/// not as geometry.
#[test]
fn the_wash_surges() {
    let (mut lo, mut hi) = (f32::MAX, 0.0f32);
    for i in 0..64 {
        let phase = std::f32::consts::TAU * i as f32 / 64.0;
        let s = foam_surge(11.0, -4.0, phase);
        assert!(
            (FOAM_SURGE_FLOOR - 1e-6..=1.0 + 1e-6).contains(&s),
            "the surge left [FOAM_SURGE_FLOOR, 1] at phase {phase}: {s}"
        );
        lo = lo.min(s);
        hi = hi.max(s);
    }
    assert!(
        (lo - FOAM_SURGE_FLOOR).abs() < 1e-3,
        "the surge never draws back"
    );
    assert!((hi - 1.0).abs() < 1e-3, "the surge never runs up");
    // It travels: two points a half wavelength apart are out of phase.
    let w = WAVES[0];
    let half = w.len_m * 0.5;
    // A quarter, not a half: at half a wavelength both samples land on the
    // same zero crossing of the sine and read identical, which says nothing.
    let a = foam_surge(0.0, 0.0, 0.0);
    let b = foam_surge(w.dir[0] * half * 0.5, w.dir[1] * half * 0.5, 0.0);
    assert!(
        (a - b).abs() > 0.2,
        "the wash does not travel along the shore: {a} vs {b}"
    );
}

/// Mixing foam in must not take the colour anywhere a colour cannot go, and it
/// must move the alpha up rather than down — foam is aerated water, which is
/// the one thing on the surface you cannot see through.
#[test]
fn foam_stays_a_colour() {
    for d in [0.0f32, 0.5, 3.0, 30.0] {
        let base = depth_tint(d);
        for f in [0.0f32, 0.4, 1.0] {
            let out = with_foam(base, f);
            for (c, v) in out.iter().enumerate() {
                assert!(
                    (0.0..=1.0).contains(v),
                    "foam {f} at {d} m put channel {c} at {v}"
                );
            }
            assert!(out[3] >= base[3] - 1e-6, "foam made the water clearer");
        }
        assert_eq!(with_foam(base, 0.0), base);
    }
}

/// The vertex buffer carries **premultiplied** colour, and that is not a
/// format detail: it is what keeps the Fresnel reflection of the sky alive in
/// shallow water, where the alpha that carries the *depth* is near zero and
/// under straight blending would scale the specular away with it.
///
/// The invariant a premultiplied buffer has to hold is that no channel exceeds
/// its own alpha — a surface that does is one adding more light to the frame
/// than it transmitted, which is emission, not water. Foam is the case that
/// nearly broke it: it is the brightest thing on the sea and it is mixed in
/// after the grading.
#[test]
fn the_surface_never_emits() {
    for d in [0.0f32, 0.2, 0.4, 2.0, 5.0, 12.0, DEEP_SENTINEL_M] {
        for f in [0.0f32, 0.3, 0.7, 1.0] {
            let c = with_foam(depth_tint(d), f);
            for (ch, v) in c[..3].iter().enumerate() {
                assert!(
                    *v <= c[3] + 1e-6,
                    "at {d} m with foam {f}, channel {ch} is {v} against an alpha of {}",
                    c[3]
                );
                assert!(*v >= 0.0);
            }
        }
    }
    // At the waterline the surface contributes nothing at all — everything you
    // see there is the sand behind it plus the sky on it.
    assert_eq!(depth_tint(0.0), [0.0; 4]);
}

// ---------------------------------------------------------------------------
// The ripple map.
// ---------------------------------------------------------------------------

/// The map tiles over the whole ocean, so a discontinuity at its edge is a
/// grid of seams every [`RIPPLE_TILE_M`] as far as the eye can see. It tiles
/// iff every harmonic is an integer, which is exactly what this measures.
#[test]
fn the_ripple_map_tiles() {
    let img = ripple_map();
    let n = RIPPLE_TEX as usize;
    let data = img.data.as_ref().expect("the ripple map has no data");
    let texel = |x: usize, y: usize| {
        let i = (y * n + x) * 4;
        [
            data[i] as f32 / 255.0 * 2.0 - 1.0,
            data[i + 1] as f32 / 255.0 * 2.0 - 1.0,
            data[i + 2] as f32 / 255.0 * 2.0 - 1.0,
        ]
    };
    // The wrap must be no bigger a step than an ordinary neighbouring pair.
    let mut inner = 0.0f32;
    let mut wrap = 0.0f32;
    for i in 0..n {
        let d = |a: [f32; 3], b: [f32; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        inner = inner.max(d(texel(i, n / 2), texel(i, n / 2 + 1)));
        wrap = wrap.max(d(texel(i, n - 1), texel(i, 0)));
        wrap = wrap.max(d(texel(n - 1, i), texel(0, i)));
    }
    assert!(
        wrap <= inner * 1.5 + 1e-3,
        "the ripple map's wrap steps by {wrap} against an inner step of {inner} - it does not tile"
    );
}

/// A normal map is unit vectors around `+Z`, and it is a *perturbation*: if
/// the mean leaned off the surface normal the whole sea would be tilted by the
/// texture.
#[test]
fn the_ripple_map_is_a_perturbation() {
    let img = ripple_map();
    let n = RIPPLE_TEX as usize;
    let data = img.data.as_ref().expect("the ripple map has no data");
    let mut mean = [0.0f64; 3];
    let mut worst = 0.0f32;
    for i in 0..n * n {
        let v = [
            data[i * 4] as f32 / 255.0 * 2.0 - 1.0,
            data[i * 4 + 1] as f32 / 255.0 * 2.0 - 1.0,
            data[i * 4 + 2] as f32 / 255.0 * 2.0 - 1.0,
        ];
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        worst = worst.max((l - 1.0).abs());
        assert!(v[2] > 0.0, "a ripple normal faces into the surface");
        for c in 0..3 {
            mean[c] += v[c] as f64 / (n * n) as f64;
        }
    }
    // 1/255 of quantization on each channel is the floor here.
    assert!(worst < 0.02, "a ripple normal is {worst} off unit length");
    assert!(
        mean[0].abs() < 0.02 && mean[1].abs() < 0.02,
        "the ripple map leans: mean ({}, {})",
        mean[0],
        mean[1]
    );
    assert!(
        mean[2] > 0.9,
        "the ripple map is not a perturbation, it is terrain"
    );
}

/// **The mip chain must be complete or wgpu rejects the texture at first
/// draw**, which on a box with no display is a black window and a backtrace.
/// Bevy builds no mips for an `Image` made in code, so the count and the byte
/// length are ours to keep in step.
#[test]
fn the_ripple_map_carries_its_own_mips() {
    let img = ripple_map();
    let levels = img.texture_descriptor.mip_level_count;
    assert_eq!(levels, RIPPLE_TEX.trailing_zeros() + 1);
    assert_eq!(
        img.data.as_ref().map(|d| d.len()),
        Some(ripple_bytes(RIPPLE_TEX)),
        "the ripple map's bytes do not match its declared mip chain"
    );
    // A one-level map on a surface that reaches kilometres is a shimmering
    // carpet, so the chain has to go all the way down.
    assert!(levels > 1);
}

// ---------------------------------------------------------------------------
// The material, and the land side of the waterline.
// ---------------------------------------------------------------------------

/// Water's Fresnel reflectance at normal incidence is 2%, and Bevy's field is
/// `F0 = 0.16·reflectance²`. The plane this replaced shipped 0.55 — `F0` of
/// 4.8%, nearly two and a half times too specular. That is not a look, it is a
/// different material.
#[test]
fn the_sea_is_lit_like_water() {
    let f0 = 0.16 * WATER_REFLECTANCE * WATER_REFLECTANCE;
    let ior_f0 = ((WATER_IOR - 1.0) / (WATER_IOR + 1.0)).powi(2);
    assert!(
        (f0 - ior_f0).abs() < 2e-3,
        "reflectance {WATER_REFLECTANCE} is F0 {f0}, but IOR {WATER_IOR} says {ior_f0}"
    );
    assert!(
        WATER_ROUGHNESS > 0.0,
        "a perfectly smooth sea has one pixel of sun on it and nothing else"
    );
}

/// The land half of the shoreline: wet at and below the waterline, dry above
/// the band, monotone between, and never outside `ART.md`'s albedo band on the
/// dark side — a beach that goes black at the tideline is worse than one that
/// stays dry.
#[test]
fn the_shore_is_wet_and_stays_a_surface() {
    // A gentle beach: the height bound is what ends the band.
    let gentle = 0.05f32;
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL, gentle), 1.0);
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL - 4.0, gentle), 1.0);
    assert_eq!(
        terrain_mesh::wet_factor(SEA_LEVEL + WET_BAND_M, gentle),
        0.0
    );
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL + 40.0, gentle), 0.0);
    let mut prev = 1.1f32;
    for i in 0..=40 {
        let y = SEA_LEVEL + WET_BAND_M * i as f32 / 40.0;
        let w = terrain_mesh::wet_factor(y, gentle);
        assert!(w <= prev + 1e-6, "wetness rose with height at {y}");
        prev = w;
    }

    // Every ground identity, soaked, has to stay a surface.
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for (i, dry) in terrain_mesh::GROUND_ALBEDO.iter().enumerate() {
        let wet = terrain_mesh::wetted(*dry, 1.0);
        assert!(
            luma(wet) < luma(*dry),
            "identity {i} does not darken when it is wet"
        );
        assert!(
            luma(wet) >= terrain_mesh::ALBEDO_LUMA_FLOOR - 1e-6,
            "identity {i} soaks to {} luma, under ART.md's albedo floor",
            luma(wet)
        );
        for (c, v) in wet.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(v),
                "identity {i} channel {c} left [0,1]"
            );
        }
        // Dry is dry: the modifier has to be a no-op above the band.
        assert_eq!(terrain_mesh::wetted(*dry, 0.0), *dry);
        // And it must be a *value* change, not a hue rotation: the darkest
        // channel stays the darkest.
        let order = |c: [f32; 3]| {
            let mut ix = [0usize, 1, 2];
            ix.sort_by(|a, b| c[*a].partial_cmp(&c[*b]).unwrap());
            ix
        };
        assert_eq!(order(*dry), order(wet), "identity {i} changed hue when wet");
    }
    // The soak is the documented one, within the saturation stretch.
    let grey = [0.3f32, 0.3, 0.3];
    assert!((luma(terrain_mesh::wetted(grey, 1.0)) / luma(grey) - WET_VALUE).abs() < 1e-3);
}

/// **The damp band is bounded by a distance AND a height, and this is the
/// assertion the shore's softness rests on.** Neither bound alone survives
/// four bank steepnesses: height alone makes a 4% beach damp for sixty metres,
/// and run alone makes a cliff damp for fourteen metres of vertical rock. What
/// has to hold is that the band is *a few metres of ground* on anything a
/// player would call a shore.
#[test]
fn the_damp_band_is_a_few_metres_of_ground_on_any_bank() {
    // How far back from the waterline the ground is still damp, in metres of
    // horizontal run, on banks of four different steepnesses.
    let run_of = |slope: f32| {
        let mut last = 0.0f32;
        for i in 1..=4000 {
            let run = i as f32 * 0.02;
            if terrain_mesh::wet_factor(SEA_LEVEL + run * slope, slope) > 0.02 {
                last = run;
            } else {
                break;
            }
        }
        last
    };
    let gentle = run_of(0.04);
    let mild = run_of(0.15);
    let steep = run_of(0.6);
    let cliff = run_of(2.0);
    // Nothing collapses to a line, and nothing runs away across the frame.
    for (name, run) in [
        ("gentle", gentle),
        ("mild", mild),
        ("steep", steep),
        ("cliff", cliff),
    ] {
        assert!(
            run > 1.0,
            "a {name} bank is damp for only {run:.2} m of ground - that is a line, not a band"
        );
        assert!(
            run <= WET_REACH_M + 0.1,
            "a {name} bank is damp for {run:.2} m of ground - that is a stain, not a shore"
        );
    }
    // Anything walkable gets a band you can see from standing height.
    for (name, run) in [("gentle", gentle), ("mild", mild), ("steep", steep)] {
        assert!(run > 3.0, "a {name} bank is damp for only {run:.2} m");
    }
    // The run bound is what stops the gentle case: on a 4% grade the height
    // bound alone would reach WET_BAND_M / 0.04 metres inland.
    assert!(
        gentle < WET_BAND_M / 0.04 * 0.5,
        "the run bound is not binding on a gentle beach"
    );
    // And the height bound is what stops the flat case, where there is no run
    // bound to have — a gradient of zero would otherwise wet the horizon.
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL + WET_BAND_M, 0.0), 0.0);
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL + WET_BAND_M, 1e-9), 0.0);
}

// ---------------------------------------------------------------------------
// The per-frame field.
// ---------------------------------------------------------------------------

/// `animate` resolves [`wave_field`] once a vertex into a cache, and its three
/// attribute passes read that cache. Before this it called the field inside
/// each pass — three times a vertex, up to twelve `sin_cos` where four would
/// do, at 1.01 ms a frame on the gate box against 0.38 ms.
///
/// The saving is only sound if the cache's index means what the passes think
/// it means, and that is what this pins: `coords[ix]` is an X offset and
/// `coords[iz]` is a Z one. Transposed, the sea's swell would run across its
/// own gradient — the grid is square, so nothing would be out of range and
/// nothing would panic; it would simply be a different sea, on a surface no
/// gate here photographs.
#[test]
fn the_resolved_field_is_the_field_at_that_vertex() {
    let coords = axis_coords();
    let spacing = axis_spacing(&coords);
    let n = coords.len();
    // A shoaling cache that is different at every vertex, so a transposed or
    // off-by-one index cannot land on an equal value by luck.
    let shoal: Vec<f32> = (0..n * n).map(|i| (i % 97) as f32 / 96.0).collect();
    let centre = Vec2::new(-37.0, 118.0);
    // A rough sea, whose gains differ wave to wave, so a gain handed to the
    // wrong wave is a different surface too.
    let swell = SeaState::at(0.7).swell;
    let mut field = vec![[0.0f32; 3]; n * n];
    resolve_field(
        centre, &coords, &spacing, &shoal, &PHASE, &swell, &mut field,
    );

    for (iz, iaz) in [(0usize, 0usize), (1, 5), (n / 2, n / 3), (n - 1, n - 2)] {
        let ix = iaz;
        let i = iz * n + ix;
        let sp = spacing[ix].max(spacing[iz]);
        let want = wave_field(
            centre.x + coords[ix],
            centre.y + coords[iz],
            &PHASE,
            &swell,
            sp,
            shoal[i],
        );
        assert_eq!(
            (field[i][0], field[i][1], field[i][2]),
            want,
            "vertex (ix {ix}, iz {iz}) is not the field at its own coordinates"
        );
    }
    // And it is the WHOLE grid, not a prefix: a pass reading past what was
    // resolved would draw the previous frame's surface out there.
    for iz in 0..n {
        for ix in 0..n {
            let i = iz * n + ix;
            let sp = spacing[ix].max(spacing[iz]);
            let (h, gx, gz) = wave_field(
                centre.x + coords[ix],
                centre.y + coords[iz],
                &PHASE,
                &swell,
                sp,
                shoal[i],
            );
            assert_eq!([h, gx, gz], field[i], "vertex {i} of {}", n * n);
        }
    }
}

// ---------------------------------------------------------------------------
// The breakers.
// ---------------------------------------------------------------------------

/// A breaker never stands on the beach, never runs out to sea and never
/// troubles a lake — the same climbing-the-sand artefact `shoal` exists to
/// prevent, for the wave that runs straight at the sand.
#[test]
fn a_breaker_lives_only_in_the_surf_zone_of_the_sea() {
    assert_eq!(
        surf_envelope(0.0, 0.0, 1.0),
        0.0,
        "a breaker at the waterline"
    );
    assert_eq!(surf_envelope(-0.5, 0.0, 1.0), 0.0, "a breaker on dry land");
    assert_eq!(
        surf_envelope(SURF_FADE_TO_M + 0.1, 20.0, 1.0),
        0.0,
        "a breaker offshore"
    );
    assert_eq!(surf_envelope(1.5, 10.0, 0.0), 0.0, "a breaker on a lake");
    assert!(surf_envelope(1.5, 10.0, 1.0) > SURF_AMP_M * 0.9);
    assert_eq!(
        surf_envelope(1.5, SURF_REACH_M, 1.0),
        0.0,
        "past the field's reach"
    );
    // The core carries it: a quarter wavelength at or above the core step.
    assert!(SURF_LEN_M * 0.25 >= STEP_M);
}

/// The distance field measures what it says on a straight shore.
#[test]
fn the_shore_distance_is_the_distance_to_dry_ground() {
    let n = 40;
    // Dry for x < 10, deepening after.
    let depth: Vec<f32> = (0..n * n).map(|i| (i % n) as f32 - 9.5).collect();
    let mut out = vec![0.0f32; n * n];
    shore_distance(&depth, n, 0, n - 1, 2.0, &mut out);
    for ix in 10..30 {
        let got = out[20 * n + ix];
        let want = ((ix - 9) as f32 * 2.0).min(SURF_REACH_M);
        assert!((got - want).abs() < 1e-3, "x {ix}: {got} m, wanted {want}");
    }
    assert_eq!(out[20 * n + 3], 0.0, "dry ground is at distance zero");
}

/// **The point of the whole thing: the crests move toward land.** Track the
/// highest crest along a line running offshore and step the phase: it must
/// arrive closer to the beach.
#[test]
fn the_breakers_run_toward_land() {
    let crest_at = |phase: f32| {
        let mut best = (0.0f32, f32::MIN);
        // One wavelength's worth of the surf zone, 10 m to 24 m out.
        let mut d = 10.0;
        while d < 10.0 + SURF_LEN_M {
            let (h, _, _, _) = surf_at(SURF_AMP_M, d, [1.0, 0.0], phase);
            if h > best.1 {
                best = (d, h);
            }
            d += 0.05;
        }
        best.0
    };
    let a = crest_at(1.0);
    let b = crest_at(1.3);
    assert!(
        b < a,
        "the crest moved from {a} m to {b} m - away from the beach"
    );
    // And the gradient points the way the distance grows, scaled like the
    // height: a finite difference along the direction agrees.
    let (h0, gx, _, _) = surf_at(SURF_AMP_M, 12.0, [1.0, 0.0], 0.7);
    let (h1, _, _, _) = surf_at(SURF_AMP_M, 12.001, [1.0, 0.0], 0.7);
    assert!(
        ((h1 - h0) / 0.001 - gx).abs() < 0.02,
        "slope {gx} against {}",
        (h1 - h0) / 0.001
    );
}

/// A bore is a white front with a fading wake behind it — seaward — and clear
/// water ahead of it: the trail peaks as the crest passes, has faded by the
/// next crest, and there is no breaker at all where the field has no
/// direction.
#[test]
fn a_bore_foams_behind_its_front() {
    let k = std::f32::consts::TAU / SURF_LEN_M;
    // Put the crest at 20 m: `k·d + phase = π/2`.
    let phase = std::f32::consts::FRAC_PI_2 - k * 20.0;
    let at = |d: f32| surf_at(SURF_AMP_M, d, [1.0, 0.0], phase).3;
    assert!(at(20.0) > 0.99, "no foam at the front: {}", at(20.0));
    assert!(at(22.0) > at(24.0), "the wake does not fade seaward");
    assert!(at(19.0) < 0.1, "foam ahead of the front: {}", at(19.0));
    assert_eq!(surf_at(SURF_AMP_M, 20.0, [0.0, 0.0], phase).3, -1.0);
    assert_eq!(
        breaker_foam(SURF_AMP_M, 8.0, 1.0),
        0.0,
        "a bore breaking in deep water"
    );
    assert!(breaker_foam(SURF_AMP_M, 0.5, 1.0) > 0.5 * SURF_FOAM_MAX);
}

// ---------------------------------------------------------------------------
// The surface shader.
// ---------------------------------------------------------------------------

const WATER_WGSL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/shaders/water.wgsl"
);

/// The uniform's two sides list the same fields in the same order. A uniform
/// whose sides disagree still binds; every field after the mismatch is then
/// garbage, and the symptom is a wrong-looking sea rather than a layout error.
#[test]
fn the_water_uniform_declares_the_same_fields_on_both_sides() {
    fn fields(src: &str, header: &str, strip: &str) -> Vec<String> {
        let body = src
            .split_once(header)
            .unwrap_or_else(|| panic!("no `{header}` in the source"))
            .1;
        let body = body.split_once("\n}").expect("unterminated struct").0;
        body.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .filter_map(|l| l.trim_start_matches(strip).trim().split(':').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect()
    }
    let wgsl = std::fs::read_to_string(WATER_WGSL).expect("water.wgsl");
    let rust = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/water.rs"))
        .expect("water.rs");
    let shader = fields(&wgsl, "struct Water {", "");
    let host = fields(&rust, "pub struct WaterParams {", "pub ");
    assert_eq!(shader, host, "water.wgsl and WaterParams disagree");
    assert!(shader.len() >= 9, "the parse matched nothing: {shader:?}");
}

/// Every derivative in the fragment is taken before its first branch, and no
/// texture is sampled with implicit derivatives at all — the reflection's
/// cube read sits under `if is_front`. naga does not enforce uniform control
/// flow in the fragment stage, so either mistake compiles and smears.
#[test]
fn the_water_shader_takes_no_derivative_under_a_branch() {
    let src = std::fs::read_to_string(WATER_WGSL).expect("water.wgsl");
    let code: String = src
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("textureSample("),
        "an implicit-derivative sample in water.wgsl"
    );
    let frag = code.split_once("fn fragment(").expect("no fragment").1;
    let first_if = frag.find("if ").expect("no branch at all");
    for d in ["dpdx", "dpdy", "fwidth"] {
        if let Some(at) = frag.rfind(d) {
            assert!(at < first_if, "`{d}` after the fragment's first branch");
        }
    }
}

/// The refracted path is bounded: straight down it is the depth, and at the
/// most grazing view it is `1/sqrt(1 − 1/n²)` ≈ 1.51× the depth — never the
/// straight line's `1/cos`, which would make the shallows opaque from the sand.
#[test]
fn the_refracted_path_is_bounded() {
    assert!((refracted_path(1.0) - 1.0).abs() < 1e-6);
    let grazing = 1.0 / (1.0 - 1.0 / (WATER_IOR * WATER_IOR)).sqrt();
    assert!((refracted_path(0.0) - grazing).abs() < 1e-3);
    assert!(grazing < 1.6);
    let mut prev = refracted_path(1.0);
    for i in (0..100).rev() {
        let p = refracted_path(i as f32 / 100.0);
        assert!(p >= prev - 1e-6, "the path shortened as the view flattened");
        prev = p;
    }
    // The glint's Fresnel and the reflectance are one number.
    let ior_f0 = ((WATER_IOR - 1.0) / (WATER_IOR + 1.0)).powi(2);
    assert!((water_f0() - ior_f0).abs() < 2e-3);
}

/// The reflected sky is a sky: the horizon brighter than the zenith and
/// greyer, a night floor far under the day and only at night, a warm glow
/// only for a low sun and only on the red side, and a storm greyer and dimmer.
#[test]
fn the_reflected_sky_follows_the_day() {
    use bevy::math::Vec3;
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let noon_sun = Vec3::new(0.3, 0.8, 0.5).normalize();
    let [z, h, g, f] = sky_light(noon_sun, 0.0, 0.0);
    assert!(lum(h) > lum(z), "the horizon is darker than the zenith");
    assert!(z[2] > z[0], "the zenith is not blue");
    assert!(
        h[2] / h[0] < z[2] / z[0],
        "the horizon is not greyer than the zenith"
    );
    assert_eq!(g, [0.0; 3], "a dusk glow at noon");
    assert_eq!(f, [0.0; 3], "a night floor by day");
    let [_, _, _, fnight] = sky_light(noon_sun, 0.0, 1.0);
    assert!(lum(fnight) > 0.0, "the night sky is black");
    assert!(
        lum(fnight) < 0.01 * lum(z),
        "the night sky is as bright as the day"
    );
    let low = Vec3::new(0.95, 0.05, 0.0).normalize();
    let [_, _, gd, _] = sky_light(low, 0.0, 0.0);
    assert!(gd[0] > gd[2] && gd[0] > 0.0, "no warm glow at a low sun");
    let [zs, _, _, _] = sky_light(noon_sun, 1.0, 0.0);
    assert!(
        lum(zs) < lum(z),
        "a storm sky reflects brighter than a clear one"
    );
    assert!(TWILIGHT_SKY > 0.0 && TWILIGHT_SKY < 1.0);
}
