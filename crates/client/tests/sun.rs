//! Gate: the sun the scene is *shaded by* and the sun the sky *draws* are one
//! vector, a caster's shadow falls away from it, and **the vector sweeps**.
//!
//! ## The sweep (2026-08-15)
//!
//! The operator played the shard and said *"the sun donest move"*. It did —
//! up and down — because `to_sun` read `RIG_SUN_AZIMUTH` as a constant, so
//! every shadow changed length all day and never direction. `rig::sun_azimuth`
//! now sweeps `RIG_SUN_ARC` of bearing across the cycle, anchored so that noon
//! is bit-identical to what this rig shipped before.
//!
//! **One gate in this file died to make room and it was NOT deleted.**
//! `the_azimuth_does_not_move_with_the_elevation` swept 33 elevations and
//! asserted the horizontal bearing never moved — a real claim, correctly
//! written, and red by construction the moment the sweep landed. Deleting it
//! would have left the bearing ungated in the other direction, which is the
//! same hole one axis over. It is replaced, not removed, by the five
//! assertions under §The sweep below: the arc's size, its monotonicity, its
//! symmetry about noon, its direction against the compass's own owner, and
//! the pin that noon did not move. `CLAUDE.md`'s "a law without a gate is a
//! mood" cuts both ways — so does a gate whose law has changed.
//!
//! **Why this exists.** `pass-20260814-223652-01-visual.md` ranked "the scene
//! is lit from the opposite hemisphere to the sun it draws" as a first-order
//! defect — criterion 1, where broken beats ugly — and asked for the seam to be
//! closed "by construction" and then gated arithmetically, in the shape of
//! `tests/tree.rs`. Its own note on the class is the reason the gate is worth
//! more than the fix: **a 180° azimuth error is invisible to
//! `test_protocol_golden`, `test_replay`, clippy and every pixel statistic this
//! repo owns.** It would survive a whole capture set, which is exactly what a
//! wall is for.
//!
//! ✅ **The mirrored compass named below is FIXED** (2026-08-15, `DECISIONS.md`
//! — east is `-X`). The paragraph stays because the *diagnosis* is what this
//! file is evidence for, and because the fix does not change a line here: this
//! gate was written convention-free on purpose and that is still the right
//! shape. `crates/client/tests/compass.rs` is the gate the other half got.
//!
//! ⚠ **The report's diagnosis did not survive the code, and the correction is
//! the point rather than a footnote.** There is no hemisphere disagreement to
//! find: `bevy_pbr`'s `light.rs` fills `dir_to_light: light.transform.back()`
//! ONCE, and that single field feeds both the `N·L` term and the atmosphere's
//! sun disc (`atmosphere/functions.wgsl` reads `(*light).direction_to_light`
//! for `sun_disk_angular_size` and for every scattering integral). Two
//! consumers, one field, no seam — they cannot disagree about the azimuth even
//! in principle. What the judge actually measured is real and is a different
//! bug: it read a shadow running screen-right in a frame whose compass said
//! `N 000°` and called that "due east", because **the compass is mirrored** —
//! facing the direction `look::bearing_deg` calls north, the body's right is
//! the direction it calls west. That was `NOW.md` §0gj, landed 2026-08-15, and
//! it was never this file's business. `CLAUDE.md`'s trap list: a judge names
//! the symptom, and the cause is diagnosed before it is acted on — this is the
//! entry's own example now, since acting on the report's literal sentence would
//! have put a second sun vector in the tree to cancel a compass error.
//!
//! So the seam that WAS real is the one the report named second and the tree
//! carried in silence: `sky.rs` re-derived `to_sun` from the two constants by
//! hand, three lines identical to `rig.rs`'s. They agreed by coincidence of two
//! matching edits rather than by construction — the deck bakes at noon and the
//! rig sweeps, so the next hand to touch either would have had no gate.
//! `rig::to_sun` is the one owner now, and §`the_cloud_deck_and_the_shadows_…`
//! below is what makes that structural rather than a comment.
//!
//! **Convention-free on purpose, and it stays that way now the convention is
//! settled.** Every assertion here is a sign, a collinearity, a ratio or a
//! monotonic sweep. Nothing asserts "the sun is in the south-east": *which
//! compass point `+X` is* has one owner (`look::bearing_of`) and one gate
//! (`tests/compass.rs`), and a copy of today's answer here would be a second
//! place to keep in step — which is the exact defect the compass pass was
//! spent on. **The one test that needs a compass CALLS that owner** rather
//! than restating it: the sweep's direction is a genuine claim (backwards,
//! the sun rises in the west) and it can only be checked against whatever the
//! compass currently says east is. Asking the owner is the opposite of
//! copying its answer — a re-labelling of the axes moves both sides at once,
//! which is what a cross-consumer gate is for. There is exactly one
//! literal-number anchor, hand-checkable on paper, and it deliberately avoids
//! 35° and 45°: at 45° `tan == cot == 1` and an inverted shadow-length formula
//! is invisible.
//!
//! Headless — no GPU, no window, no shard. A sun direction is arithmetic.

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::rig::{
    sun_azimuth, sun_elevation, sun_rotation_at, to_sun, to_sun_at, CAPTURE_DAY_FRAC, RIG_SUN_ARC,
    RIG_SUN_AZIMUTH, RIG_SUN_ELEVATION,
};
use sim_core::limits::DAY_PORTION;

/// Agreement tolerance for two derivations of one direction. The shadow
/// pack's own `directionEpsilon` for "this sun has moved enough to recommit a
/// shadow frame" is 0.002 rad; identity is held far tighter below.
const EPS_RAD: f32 = 2e-3;

/// What Bevy actually uploads for this rotation.
///
/// `bevy_pbr::render::light` builds `GpuDirectionalLight { dir_to_light:
/// light.transform.back(), .. }` — its own comment reads "direction is negated
/// to be ready for N.L". Restating it here rather than calling a helper is the
/// point: the test asserts against **Bevy's** convention, not against ours, so
/// it stays red if we ever start agreeing with ourselves and not with the
/// renderer.
fn dir_to_light(frac: f32) -> Vec3 {
    Transform::from_rotation(sun_rotation_at(frac))
        .back()
        .into()
}

/// The horizontal offset from a caster's foot to its shadow tip, on `y = 0`.
///
/// Solving `p.y + t·(-s).y = 0` for a caster of height `h` under `to_sun = s`
/// gives `t = h / s.y` and `offset = -(h / s.y) · s.xz`.
fn shadow_of(s: Vec3, h: f32) -> Vec2 {
    -(h / s.y) * Vec2::new(s.x, s.z)
}

/// The same, at an hour of the cycle.
fn shadow_offset(frac: f32, h: f32) -> Vec2 {
    shadow_of(to_sun(frac), h)
}

/// The sun's horizontal bearing at an hour, as a unit `(x, z)` — read off the
/// vector the renderer actually gets, never off [`sun_azimuth`], so the two
/// are a cross-check and not one function asked twice.
fn sun_h(frac: f32) -> Vec2 {
    let s = to_sun(frac);
    Vec2::new(s.x, s.z).normalize()
}

/// The signed angle from `a` to `b`, radians in `(-π, π]`. `perp_dot` is the
/// 2D cross product; with `(x, z)` components it comes out positive when the
/// yaw-convention bearing *falls*, which is the direction a sun travelling
/// east to west turns (`rig::sun_azimuth` states why).
fn turn(a: Vec2, b: Vec2) -> f32 {
    a.perp_dot(b).atan2(a.dot(b))
}

/// The hours the daylit half is sampled at. 64 steps inclusive of both
/// endpoints — dawn and dusk are exactly on the horizon, where the horizontal
/// component is at its largest, so no sample here is near the degenerate
/// zenith case.
fn daylit(step: u32, steps: u32) -> f32 {
    step as f32 * DAY_PORTION / steps as f32
}

// ── The law: direction_to_light == -forward() == to_sun ──────────────────

/// The whole negated-forward class in one line, at every elevation the cycle
/// reaches.
///
/// This is the assertion the report asked for, and it is an identity rather
/// than a comparison: `sun_rotation_at` is *defined* as `looking_at(-to_sun)`,
/// so what is really being gated is that nobody has since added a second
/// negation, swapped the `looking_at` target for the position, or reached for
/// `forward()` where `back()` was meant. All three are one-character edits that
/// no other gate in this repo can see.
#[test]
fn the_light_bevy_uploads_is_the_vector_toward_the_sun() {
    for step in 0..=64 {
        let frac = step as f32 / 64.0;
        let s = to_sun(frac);
        let d = dir_to_light(frac);

        assert!(
            (s.length() - 1.0).abs() < 1e-5,
            "to_sun must be a unit vector, |s| = {} at frac {frac}",
            s.length()
        );
        // `dot ≈ 1` rather than component equality: it is the quantity the
        // shader actually uses, and it fails on a flip of ANY axis.
        assert!(
            d.dot(s) > 1.0 - 1e-6,
            "the vector Bevy uploads must BE to_sun at frac {frac}: \
             dir_to_light {d:?} vs to_sun {s:?} (dot {})",
            d.dot(s)
        );
    }
}

/// The light travels the other way, and the sign is asserted rather than
/// assumed.
#[test]
fn the_lights_forward_is_the_direction_sunlight_travels() {
    for step in 0..=32 {
        let frac = step as f32 / 32.0;
        let fwd: Vec3 = Transform::from_rotation(sun_rotation_at(frac))
            .forward()
            .into();
        assert!(
            fwd.dot(to_sun(frac)) < -1.0 + 1e-6,
            "forward must be exactly -to_sun at frac {frac}: dot {}",
            fwd.dot(to_sun(frac))
        );
    }
}

// ── The sweep: the five claims that replaced the fixed-bearing gate ──────
//
// This block is what `the_azimuth_does_not_move_with_the_elevation` became.
// That test asserted the sun's horizontal bearing was the same at every
// elevation, which was true, gated and correct until 2026-08-15 — and the
// operator's *"the sun donest move"* is the other side of the same fact.
// Every test below is red on the constant-bearing sun it replaces, and the
// first one is red the most bluntly: under the old `to_sun` the morning and
// afternoon bearings were bit-identical.

/// **The assertion that was red before the sweep.** Morning and afternoon are
/// not the same direction.
///
/// Stated as a lower bound rather than as "they differ", because a sweep of a
/// tenth of a degree would satisfy inequality and still be a sun that does not
/// move: a **quarter-day** either side of noon — half a day apart — must turn
/// the bearing by half the arc, which is what the linear sweep owes. (This
/// doc said "an hour either side ... a quarter of the arc" until 2026-08-15,
/// which agreed with neither the samples below nor the constant they are
/// checked against; a reader who trusted it would have widened the spacing
/// and weakened the gate to a claim the whole-day sweep test already makes.)
#[test]
fn the_bearing_moves_between_morning_and_afternoon() {
    let quarter = DAY_PORTION * 0.25;
    let morning = sun_h(CAPTURE_DAY_FRAC - quarter);
    let afternoon = sun_h(CAPTURE_DAY_FRAC + quarter);
    let moved = turn(morning, afternoon).abs();
    assert!(
        (moved - RIG_SUN_ARC * 0.5).abs() < 1e-3,
        "half a day either side of noon must turn the sun by half the arc \
         ({} rad); it turned {moved}",
        RIG_SUN_ARC * 0.5
    );
    // And the shadow moved with it, which is the thing a player sees. Same
    // caster, same ground, two hours: the tips are not in the same place.
    let a = shadow_offset(CAPTURE_DAY_FRAC - quarter, 2.0);
    let b = shadow_offset(CAPTURE_DAY_FRAC + quarter, 2.0);
    assert!(
        turn(a.normalize(), b.normalize()).abs() > 0.5,
        "the shadow's DIRECTION must move, not just its length: {a:?} → {b:?}"
    );
}

/// The sweep is monotonic across the daylit half and it is exactly
/// `RIG_SUN_ARC` wide.
///
/// Both facts come out of one accumulation, and the accumulation is over the
/// vectors the renderer receives — `to_sun`'s output — never over
/// `sun_azimuth`, which would be asking one function to confirm itself. The
/// signed steps also catch a sweep that doubles back at some hour, which a
/// dawn-to-dusk endpoint check cannot see.
#[test]
fn the_daylit_sweep_is_monotonic_and_is_the_arc_the_constant_claims() {
    const STEPS: u32 = 64;
    let mut total = 0.0f32;
    let mut prev = sun_h(daylit(0, STEPS));
    for step in 1..=STEPS {
        let frac = daylit(step, STEPS);
        let h = sun_h(frac);
        let d = turn(prev, h);
        assert!(
            d > 0.0,
            "the bearing must sweep one way all day; it turned {d} rad at \
             frac {frac}"
        );
        total += d;
        prev = h;
    }
    assert!(
        (total - RIG_SUN_ARC).abs() < 1e-3,
        "sunrise to sunset must be RIG_SUN_ARC ({RIG_SUN_ARC} rad) of bearing, \
         got {total}"
    );
}

/// Sunrise and sunset are mirror images about the noon bearing.
///
/// This is the property that makes noon the *anchor* rather than one point on
/// a ramp, and it is what stops a future edit to the sweep from dragging the
/// capture register sideways: any asymmetry moves noon off `RIG_SUN_AZIMUTH`
/// and is caught here before `ART.md` §1's band is re-measured on top of it.
#[test]
fn sunrise_and_sunset_are_symmetric_about_noon() {
    let noon = sun_h(CAPTURE_DAY_FRAC);
    for step in 0..=32 {
        let d = step as f32 * (DAY_PORTION * 0.5) / 32.0;
        let before = turn(sun_h(CAPTURE_DAY_FRAC - d), noon);
        let after = turn(noon, sun_h(CAPTURE_DAY_FRAC + d));
        assert!(
            (before - after).abs() < 1e-3,
            "the sweep is lopsided {d} either side of noon: {before} before, \
             {after} after"
        );
    }
}

/// **The sun travels the way the compass calls east to west.**
///
/// The one test in this file that needs a cardinal, and it asks
/// `look::bearing_of` — the owner `tests/compass.rs` gates — instead of
/// restating what east is. Backwards, this is a sun that rises in the west,
/// and that defect is invisible to every other wall in this repo for exactly
/// the reasons the mirrored compass was: not on the wire, not in
/// `state_hash`, an `f32` through an `f32` function, and unreadable by a pixel
/// statistic.
///
/// Accumulated by shortest-step so it is wrap-safe: it holds for any arc,
/// including one that would carry the bearing past 0°/360°.
#[test]
fn the_sun_travels_the_way_the_compass_calls_east_to_west() {
    use client::look::bearing_of;
    const STEPS: u32 = 64;
    let bearing = |frac: f32| {
        let h = sun_h(frac);
        bearing_of(h.x, h.y)
    };
    let mut total = 0.0f32;
    let mut prev = bearing(daylit(0, STEPS));
    for step in 1..=STEPS {
        let b = bearing(daylit(step, STEPS));
        // Shortest signed step, degrees, in (-180, 180].
        let d = (b - prev + 180.0).rem_euclid(360.0) - 180.0;
        assert!(
            d > 0.0,
            "the compass bearing of the sun must RISE through the day — a \
             falling one is a sun rising in the west. Step {step}: {prev}° → \
             {b}°"
        );
        total += d;
        prev = b;
    }
    assert!(
        (total - RIG_SUN_ARC.to_degrees()).abs() < 0.1,
        "the compass agrees the sweep exists but not about its size: {total}° \
         against {}°",
        RIG_SUN_ARC.to_degrees()
    );
    // **And it must RISE in the east half**, which nothing above says.
    // Every assertion so far is wrap-safe by construction, so all of them
    // hold for a path rotated a half-turn: moving `RIG_SUN_AZIMUTH` to
    // `2.35 + π` — the arithmetically obvious way to put noon in the south,
    // which this file's own §remaining invites — leaves the sense, the arc
    // and the noon pin green while sunrise lands at compass 315° (NW).
    // Proved by that exact mutation, 2026-08-15: all 15 tests here passed.
    //
    // Where east is comes from `look::bearing_of` rather than from a
    // restated convention — east is `-X` (`DECISIONS.md` 2026-08-15) and
    // `tests/compass.rs` is that function's owner.
    let east = bearing_of(-1.0, 0.0);
    let dawn = bearing(daylit(0, STEPS));
    let off_east = ((dawn - east + 180.0).rem_euclid(360.0) - 180.0).abs();
    assert!(
        off_east < 90.0,
        "sunrise must fall in the half of the compass centred on east \
         ({east}°); it is at {dawn}°, {off_east}° away — that is a sun that \
         rises in the west, which every other assertion in this test permits"
    );
}

/// **Noon did not move, and this is the pin that says so.**
///
/// `ART.md` §1's shadow band, `DayPin`'s capture register and every frame a
/// visual judge has ever scored are authored at noon. The sweep was anchored
/// there on purpose, so the vector at `CAPTURE_DAY_FRAC` must still be the one
/// the pre-sweep rig produced — built here from the two constants directly,
/// which is what the old `to_sun` body was.
#[test]
fn noon_is_bit_for_bit_where_the_rig_was_authored() {
    assert_eq!(
        sun_azimuth(CAPTURE_DAY_FRAC),
        RIG_SUN_AZIMUTH,
        "the sweep must pass through the authored bearing exactly at noon"
    );
    assert_eq!(
        sun_elevation(CAPTURE_DAY_FRAC),
        RIG_SUN_ELEVATION,
        "and the authored elevation, which the sweep must not have disturbed"
    );
    // The whole vector, against the construction the fixed-bearing rig used.
    assert_eq!(
        to_sun(CAPTURE_DAY_FRAC),
        to_sun_at(RIG_SUN_ELEVATION, RIG_SUN_AZIMUTH),
        "the noon sun moved — every judged frame is now incomparable"
    );
}

/// The bearing is continuous everywhere, including across dusk, and it closes
/// the loop: the next dawn is the same direction as this one.
///
/// The night half is not decoration. The sun keeps going the same way under
/// the world, so the atmosphere's twilight band sits where the sun set for the
/// first hour of night and swings round to where it will rise before dawn. A
/// night branch that jumped, or that swept back the way it came, would put the
/// glow on the wrong horizon — and `daylight` is zero there, so no brightness
/// assertion anywhere could see it.
#[test]
fn the_bearing_is_continuous_across_the_cycle_and_closes_it() {
    const STEPS: u32 = 400;
    let mut prev = sun_h(0.0);
    // **The cap is derived from the two segment rates, not a flat multiple of
    // the uniform one.** It was `TAU / STEPS * 3.0`, which held only because
    // `DAY_PORTION` was 0.70; at the 80-minute cycle spoken 2026-08-15 night
    // is 12.5% of the cycle and still carries the whole `TAU - RIG_SUN_ARC`
    // of catch-up, so the night bearing rate went to 4× the uniform one and
    // this gate went red at step 351 — on a real change in the sky, not a
    // defect. The two rates below are what the sweep actually does; 1.5×
    // leaves room for the sampling phase without admitting a discontinuity,
    // which is what this gate is for.
    let day_rate = RIG_SUN_ARC / (DAY_PORTION * STEPS as f32);
    let night_rate = (std::f32::consts::TAU - RIG_SUN_ARC) / ((1.0 - DAY_PORTION) * STEPS as f32);
    let cap = day_rate.max(night_rate) * 1.5;
    for step in 1..=STEPS {
        let h = sun_h(step as f32 / STEPS as f32);
        let d = turn(prev, h);
        assert!(
            d > 0.0 && d < cap,
            "the bearing jumped {d} rad at step {step} of the cycle (cap {cap})"
        );
        prev = h;
    }
    // frac 1.0 is frac 0.0 of the next cycle: one full turn, same direction.
    let close = turn(sun_h(1.0), sun_h(0.0)).abs();
    assert!(
        close < 1e-3,
        "the cycle must close on the bearing it opened with; it is {close} \
         rad out, so every dawn drifts"
    );
}

// ── The shadow: a known caster on a known plane ──────────────────────────

/// **The assertion that would have caught a mirrored sun**, and the only one
/// here that needs no convention at all: a shadow points AWAY from the sun.
///
/// Sign-only, so it survives any relabelling of which compass point `+X` is —
/// which matters, because that relabelling is an open question (§0gj) and a
/// test written against today's answer would have to be edited to stay green
/// through a fix, which is the shape of a gate that scores the bug.
/// Swept over the HOURS of the daylit half since the sweep landed, not over a
/// range of elevations: the claim is now "at every hour, the shadow is behind
/// the sun *of that hour*", which is the pairing a swept bearing makes
/// falsifiable. Endpoints trimmed off the horizon, where the shadow runs to
/// infinity and its direction is all that is left of it.
#[test]
fn a_shadow_falls_away_from_the_sun() {
    for step in 1..32 {
        let frac = daylit(step, 32);
        let s = to_sun(frac);
        let sun = Vec2::new(s.x, s.z);
        let off = shadow_offset(frac, 2.0);

        assert!(
            off.dot(sun) < 0.0,
            "the shadow must run away from the sun at frac {frac}: \
             offset {off:?} · sun {sun:?} = {}",
            off.dot(sun)
        );
        // Collinear with the sun's bearing — the azimuth check with no
        // zero-point in it. `perp_dot` is the 2D cross product.
        assert!(
            off.perp_dot(sun).abs() < 1e-4 * off.length().max(1.0),
            "the shadow must lie on the sun's bearing at frac {frac}: \
             cross {}",
            off.perp_dot(sun)
        );
    }
}

/// The solar shadow-length identity, `L = H / tan(θ)`.
///
/// Scale-free (asserted as a ratio), so it survives a unit change, and it is
/// an *independent* derivation: `shadow_offset` solves a ray-plane
/// intersection, this compares against the trig identity. Two routes to one
/// number is what the granite retraction in `CLAUDE.md` asks for.
#[test]
fn a_shadow_is_as_long_as_the_suns_elevation_says() {
    for h in [0.5f32, 2.0, 6.6] {
        for step in 0..=24 {
            // Elevations the cycle never reaches, on the shipped bearing:
            // this is the geometry's gate, not the clock's, which is why it
            // goes through `to_sun_at` (`rig.rs` says why that is `pub`).
            let e = 0.10 + step as f32 * (1.2 - 0.10) / 24.0;
            let got = shadow_of(to_sun_at(e, RIG_SUN_AZIMUTH), h).length() / h;
            let want = 1.0 / e.tan();
            assert!(
                (got - want).abs() < 1e-3 * want,
                "shadow length ratio at elevation {e}: got {got}, want cot = {want}"
            );
        }
    }
}

/// A lower sun casts a longer shadow, strictly. Catches an inverted
/// elevation term that the ratio test above could only catch if it were
/// *also* wrong in the same direction.
#[test]
fn the_lower_the_sun_the_longer_the_shadow() {
    let mut prev = f32::INFINITY;
    // 80° down to 5°, the sweep the validation pack names.
    for step in 0..=48 {
        let e = (80.0 - step as f32 * 75.0 / 48.0).to_radians();
        let len = shadow_of(to_sun_at(e, RIG_SUN_AZIMUTH), 2.0).length();
        assert!(
            len > prev || prev.is_infinite(),
            "shadow must lengthen as the sun sinks: at {}° got {len}, previous {prev}",
            e.to_degrees()
        );
        prev = len;
    }
}

/// The one hand-checked anchor, with literal numbers.
///
/// A caster 2 m tall, sun at 30° elevation on the `+X` bearing: the shadow tip
/// is `2 / tan 30° = 2√3 ≈ 3.4641 m` from the foot, pointing at `-X`.
/// **Deliberately not 45°**, where `tan == cot == 1` and an inverted formula
/// reads correct, and deliberately not 35°, so it is not the shipped
/// elevation either. Written against the local `to_sun` construction rather
/// than the shipped azimuth, so it anchors the arithmetic without asserting
/// anything about where the sun in the game actually is.
#[test]
fn the_arithmetic_matches_a_paper_check() {
    let e = 30f32.to_radians();
    // to_sun on the +X bearing at 30°: (cos30, sin30, 0).
    let s = Vec3::new(e.cos(), e.sin(), 0.0);
    let off = -(2.0 / s.y) * Vec2::new(s.x, s.z);
    assert!(
        (off.x - -3.4641).abs() < 1e-3 && off.y.abs() < 1e-6,
        "2 m caster under a 30° sun on +X: want (-3.4641, 0), got {off:?}"
    );
}

// ── The seam the report asked to close ───────────────────────────────────

/// **The cloud deck and the shadows read the same vector, by construction.**
///
/// `sky.rs::cloud_cubemap` marches its light along `to_sun`'s horizontal to
/// decide which side of a cloud is lit. Until 2026-08-15 it re-derived that
/// vector from `RIG_SUN_AZIMUTH`/`RIG_SUN_ELEVATION` in three lines copied
/// from `rig.rs`. This asserts the deck's lit side agrees with the direction
/// the *shadows* say the light comes from — the cross-consumer check the
/// atmosphere pack's contract calls for ("sky and terrain haze use different
/// sun directions" is the first entry on its failure list).
///
/// The deck bakes at noon, so the comparison is pinned at `CAPTURE_DAY_FRAC`,
/// which is the hour the deck is actually baked at.
/// §`the_cloud_deck_follows_the_sun_all_day` is the other 64 hours.
///
/// ⚠ **It reads `sky::deck_march_dir()`, and the first cut of this test did
/// not — it re-derived `to_sun` on its own side and stayed green under a
/// deliberately flipped deck.** That is the validation pack's "a test that
/// calls `sun_dir()` twice proves nothing", and it is why `sky.rs` grew a
/// named function: a gate cannot see into a bake loop's local. The residual
/// hole is honest and worth stating — this sees the *derivation*, so a flip
/// written at the use site inside `cloud_cubemap` would still pass. There is
/// now one named source for it to be written at instead.
#[test]
fn the_cloud_deck_and_the_shadows_agree_about_the_sun() {
    let s = to_sun(CAPTURE_DAY_FRAC);
    // What sky.rs actually marches toward, read from sky.rs.
    let deck = client::render::sky::deck_march_dir();
    // What the shadow says, negated: the shadow runs away, so -shadow is
    // toward the sun.
    let from_shadow = (-shadow_offset(CAPTURE_DAY_FRAC, 2.0)).normalize();
    assert!(
        deck.dot(from_shadow) > 1.0 - EPS_RAD,
        "the deck marches toward {deck:?} while the shadows put the sun at \
         {from_shadow:?} — one of the two re-derived the vector"
    );
    // And the deck's march is non-degenerate: `normalize_or_zero` in sky.rs
    // silently yields a zero vector at the zenith, which would bake a flat,
    // unlit deck with no error anywhere.
    assert!(
        Vec2::new(s.x, s.z).length() > 0.1,
        "the shipped elevation must leave the deck a horizontal to march \
         along, got {}",
        Vec2::new(s.x, s.z).length()
    );
}

/// **The deck's lit side follows the sun at every hour, not only at the one it
/// was baked at** — the coupled-set half of the sweep.
///
/// The deck is baked ONCE with its lit faces pointing at the noon bearing. A
/// fixed sun never noticed, because a bearing that does not move is correct at
/// every hour for free; a swept sun would have left every cloud lit from up to
/// half the arc away by dusk, with the ground lit correctly — one owner
/// disagreeing with itself, which is `CLAUDE.md`'s tonemap/sky/exposure/fog
/// trap in its exact shape. `sky::deck_rotation` answers it by turning the
/// deck, and this is the assertion that the turn is the right size **and the
/// right way round**: a sign error there drags the clouds backwards through
/// the sky while the sun is perfectly correct.
///
/// It reads both owners — `sky::deck_march_dir` for where the lit side was
/// baked, `sky::deck_rotation` for where it is drawn — and compares against
/// the SHADOW, which is a third derivation. Nothing here re-derives `to_sun`.
#[test]
fn the_cloud_deck_follows_the_sun_all_day() {
    let baked = client::render::sky::deck_march_dir();
    let baked3 = Vec3::new(baked.x, 0.0, baked.y);
    // Where the baked lit side ends up once the deck is drawn rotated.
    let drawn = |frac: f32| {
        let d = client::render::sky::deck_rotation(frac) * baked3;
        Vec2::new(d.x, d.z).normalize()
    };
    // The interior hours, against the SHADOW — a third derivation, and the
    // one that carries the negation law with it. The two endpoints are
    // excluded rather than fudged: dawn and dusk put the sun exactly on the
    // horizon, where `h / s.y` is a division by zero and a shadow has a
    // direction but no length. They are covered below against the sun's own
    // horizontal, which is what a shadow of infinite length points along.
    for step in 1..64 {
        let frac = daylit(step, 64);
        let from_shadow = (-shadow_offset(frac, 2.0)).normalize();
        assert!(
            drawn(frac).dot(from_shadow) > 1.0 - EPS_RAD,
            "at frac {frac} the deck's lit side faces {:?} while the \
             shadows put the sun at {from_shadow:?}",
            drawn(frac)
        );
    }
    // Dawn and dusk, where the disagreement a fixed deck would have shown is
    // at its widest — half the arc.
    for frac in [0.0, DAY_PORTION] {
        assert!(
            drawn(frac).dot(sun_h(frac)) > 1.0 - EPS_RAD,
            "at frac {frac} the deck's lit side faces {:?} and the sun is at \
             {:?}",
            drawn(frac),
            sun_h(frac)
        );
    }
    // Noon is the bake hour, so the rotation there must be nothing at all —
    // the property that keeps every captured frame identical to the ones the
    // visual judge has already scored.
    let noon = client::render::sky::deck_rotation(CAPTURE_DAY_FRAC) * baked3;
    assert!(
        (noon - baked3).length() < 1e-6,
        "the deck must be drawn unrotated at the hour it was baked, got {noon:?}"
    );
}

/// Night is one predicate, and both the disc and the shadows are on the same
/// side of it.
///
/// `rig::day_night` sets `shadows_enabled = light > 0.0`; this asserts the
/// geometry underneath agrees — when the cycle says the sun is up, `to_sun`
/// really is above the horizon, and `N·L` on a flat up-face is `s.y`.
#[test]
fn the_sun_is_above_the_horizon_exactly_when_the_cycle_says_it_is() {
    for step in 0..=200 {
        let frac = step as f32 / 200.0;
        let e = sun_elevation(frac);
        let s = to_sun(frac);
        let n_dot_l = s.dot(Vec3::Y);
        assert!(
            (n_dot_l > 0.0) == (e > 0.0),
            "at frac {frac} the elevation is {e} but N·L on a flat face is \
             {n_dot_l} — the horizon and the geometry disagree"
        );
        assert!(
            (n_dot_l - e.sin()).abs() < 1e-5,
            "N·L on a +Y face must be sin(elevation) at frac {frac}"
        );
    }
}
