//! Gate: the sun the scene is *shaded by* and the sun the sky *draws* are one
//! vector, and a caster's shadow falls away from it.
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
//! the direction it calls west. That is `NOW.md` §0gj and it is not this
//! file's business. `CLAUDE.md`'s trap list: a judge names the symptom, and the
//! cause is diagnosed before it is acted on.
//!
//! So the seam that WAS real is the one the report named second and the tree
//! carried in silence: `sky.rs` re-derived `to_sun` from the two constants by
//! hand, three lines identical to `rig.rs`'s. They agreed by coincidence of two
//! matching edits rather than by construction — the deck bakes at noon and the
//! rig sweeps, so the next hand to touch either would have had no gate.
//! `rig::to_sun` is the one owner now, and §`the_cloud_deck_and_the_shadows_…`
//! below is what makes that structural rather than a comment.
//!
//! **Convention-free on purpose.** Every assertion here is a sign, a
//! collinearity, a ratio or a monotonic sweep. Nothing asserts "the sun is in
//! the south-east", because *which compass point `+X` is* is the open question
//! one file over (§0gj), and a test that hardcoded today's answer would go
//! green on the bug the moment the answer changed. There is exactly one
//! literal-number anchor, hand-checkable on paper, and it deliberately avoids
//! 35° and 45°: at 45° `tan == cot == 1` and an inverted shadow-length formula
//! is invisible.
//!
//! Headless — no GPU, no window, no shard. A sun direction is arithmetic.

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::rig::{sun_elevation, sun_rotation_at, to_sun, RIG_SUN_ELEVATION};

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
fn dir_to_light(elevation: f32) -> Vec3 {
    Transform::from_rotation(sun_rotation_at(elevation))
        .back()
        .into()
}

/// The horizontal offset from a caster's foot to its shadow tip, on `y = 0`.
///
/// Solving `p.y + t·(-s).y = 0` for a caster of height `h` under `to_sun = s`
/// gives `t = h / s.y` and `offset = -(h / s.y) · s.xz`.
fn shadow_offset(elevation: f32, h: f32) -> Vec2 {
    let s = to_sun(elevation);
    -(h / s.y) * Vec2::new(s.x, s.z)
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
        let e = sun_elevation(frac);
        let s = to_sun(e);
        let d = dir_to_light(e);

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
        let e = sun_elevation(step as f32 / 32.0);
        let fwd: Vec3 = Transform::from_rotation(sun_rotation_at(e))
            .forward()
            .into();
        assert!(
            fwd.dot(to_sun(e)) < -1.0 + 1e-6,
            "forward must be exactly -to_sun at elevation {e}: dot {}",
            fwd.dot(to_sun(e))
        );
    }
}

/// The sun rises and sets on ONE bearing at v0, and the doc on
/// `RIG_SUN_AZIMUTH` says so. A cycle that dragged the azimuth would swing
/// every shadow through a half-turn a day, so this is a real claim and not a
/// restatement — it is red the moment an azimuth term learns about elevation.
///
/// Compared as a *horizontal* direction, because near the zenith the
/// horizontal component vanishes and a raw component test would be noise.
#[test]
fn the_azimuth_does_not_move_with_the_elevation() {
    let noon = to_sun(RIG_SUN_ELEVATION);
    let ref_h = Vec2::new(noon.x, noon.z).normalize();
    for step in 0..=32 {
        // Elevations well clear of the poles, both hemispheres.
        let e = -0.5 + step as f32 * (1.0 / 32.0);
        let s = to_sun(e);
        let h = Vec2::new(s.x, s.z).normalize();
        assert!(
            h.dot(ref_h) > 1.0 - EPS_RAD,
            "the sun's bearing moved at elevation {e}: {h:?} vs {ref_h:?}"
        );
    }
}

// ── The shadow: a known caster on a known plane ──────────────────────────

/// **The assertion that would have caught a mirrored sun**, and the only one
/// here that needs no convention at all: a shadow points AWAY from the sun.
///
/// Sign-only, so it survives any relabelling of which compass point `+X` is —
/// which matters, because that relabelling is an open question (§0gj) and a
/// test written against today's answer would have to be edited to stay green
/// through a fix, which is the shape of a gate that scores the bug.
#[test]
fn a_shadow_falls_away_from_the_sun() {
    for step in 0..=32 {
        let e = 0.05 + step as f32 * (RIG_SUN_ELEVATION - 0.05) / 32.0;
        let s = to_sun(e);
        let sun_h = Vec2::new(s.x, s.z);
        let off = shadow_offset(e, 2.0);

        assert!(
            off.dot(sun_h) < 0.0,
            "the shadow must run away from the sun at elevation {e}: \
             offset {off:?} · sun {sun_h:?} = {}",
            off.dot(sun_h)
        );
        // Collinear with the sun's bearing — the azimuth check with no
        // zero-point in it. `perp_dot` is the 2D cross product.
        assert!(
            off.perp_dot(sun_h).abs() < 1e-4 * off.length().max(1.0),
            "the shadow must lie on the sun's bearing at elevation {e}: \
             cross {}",
            off.perp_dot(sun_h)
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
            let e = 0.10 + step as f32 * (1.2 - 0.10) / 24.0;
            let got = shadow_offset(e, h).length() / h;
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
        let len = shadow_offset(e, 2.0).length();
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
/// The deck bakes at noon and the rig sweeps, so the comparison is pinned at
/// `RIG_SUN_ELEVATION`, which is the hour the deck is actually baked at.
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
    let s = to_sun(RIG_SUN_ELEVATION);
    // What sky.rs actually marches toward, read from sky.rs.
    let deck = client::render::sky::deck_march_dir();
    // What the shadow says, negated: the shadow runs away, so -shadow is
    // toward the sun.
    let from_shadow = (-shadow_offset(RIG_SUN_ELEVATION, 2.0)).normalize();
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
        let s = to_sun(e);
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
