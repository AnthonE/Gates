//! The handedness gate: does the client's idea of "right" agree with the
//! engine's?
//!
//! `crate::look`'s own unit tests already pin the mapping — D strafes toward
//! `right_dir`, a mouse pushed right turns toward it — and they run in the
//! code tier, headless, with no Bevy. What they cannot do is check the one
//! assumption underneath all of it: that `right_dir`'s `(−fz, fx)` really is
//! the direction a player perceives as rightward **in the frame Bevy draws**.
//! Restating `r × up = −f` in a test proves only that the same author made
//! the same assumption twice.
//!
//! So this file builds the camera exactly as `render::rig::follow_eye` does,
//! asks BEVY for the resulting basis, and checks the client's arithmetic
//! against it. That closes the loop: if a future Bevy changed `look_to`'s
//! handedness, or if someone "fixed" `follow_eye`'s direction vector, this
//! goes red and `look.rs`'s unit tests would not.
//!
//! Renderer tier (`required-features = ["render"]`), like `tree` and `fell`
//! — it links Bevy for `Transform` alone and needs no GPU and no window.

use bevy::prelude::*;
use client::look::{move_axes, right_dir, yaw_after, yaw_u16};
use sim_core::yaw_dir;

/// `render::rig::follow_eye`, verbatim in its arithmetic: the sim's yaw 0
/// faces +Z, increasing toward +X, pitch +up.
fn camera_at(yaw: f32, pitch: f32) -> Transform {
    let cp = pitch.cos();
    let dir = Vec3::new(yaw.sin() * cp, pitch.sin(), yaw.cos() * cp);
    let mut t = Transform::default();
    t.look_to(dir, Vec3::Y);
    t
}

/// Eight bearings around the circle, on the quantized grid the sim faces.
fn bearings() -> impl Iterator<Item = f32> {
    (0..8).map(|i| i as f32 * std::f32::consts::TAU / 8.0)
}

#[test]
fn the_camera_looks_where_the_sim_faces() {
    for yaw in bearings() {
        let t = camera_at(yaw, 0.0);
        let (fx, fz) = yaw_dir(yaw_u16(yaw));
        let fwd = t.forward();
        assert!(
            (fwd.x - fx).abs() < 0.02 && (fwd.z - fz).abs() < 0.02,
            "yaw {yaw}: camera forward {fwd:?}, sim facing ({fx}, {fz})",
        );
    }
}

/// The load-bearing one. Bevy's own local +X against `look::right_dir`.
#[test]
fn right_dir_is_the_direction_bevy_calls_right() {
    for yaw in bearings() {
        let t = camera_at(yaw, 0.0);
        let (rx, rz) = right_dir(yaw_u16(yaw));
        let r = t.right();
        assert!(
            (r.x - rx).abs() < 0.02 && (r.z - rz).abs() < 0.02,
            "yaw {yaw}: Bevy's right {r:?}, look::right_dir ({rx}, {rz})",
        );
    }
}

/// The regression this whole module was written for, stated the way a player
/// would state it: **press D, move toward the right of the screen.**
///
/// The sim's wish direction is recomputed here from `movement::step`'s own
/// expression rather than imported, because importing it would test that two
/// copies of the sim agree and not that the client agrees with the sim.
#[test]
fn d_moves_toward_the_right_of_the_frame() {
    for yaw in bearings() {
        let (mx, mz) = move_axes(0, 1);
        let (fx, fz) = yaw_dir(yaw_u16(yaw));
        let (sim_rx, sim_rz) = (fz, -fx);
        let wish = Vec3::new(
            fx * (mz as f32 / 127.0) + sim_rx * (mx as f32 / 127.0),
            0.0,
            fz * (mz as f32 / 127.0) + sim_rz * (mx as f32 / 127.0),
        );
        let screen_right = camera_at(yaw, 0.0).right();
        assert!(
            wish.dot(*screen_right) > 0.98,
            "yaw {yaw}: D sent the body {wish:?}, screen-right is {screen_right:?}",
        );
    }
}

#[test]
fn w_moves_into_the_frame() {
    for yaw in bearings() {
        let (mx, mz) = move_axes(1, 0);
        let (fx, fz) = yaw_dir(yaw_u16(yaw));
        let (sim_rx, sim_rz) = (fz, -fx);
        let wish = Vec3::new(
            fx * (mz as f32 / 127.0) + sim_rx * (mx as f32 / 127.0),
            0.0,
            fz * (mz as f32 / 127.0) + sim_rz * (mx as f32 / 127.0),
        );
        let fwd = camera_at(yaw, 0.0).forward();
        assert!(wish.dot(*fwd) > 0.98, "yaw {yaw}: W sent the body {wish:?}");
    }
}

/// **Press mouse right, the frame swings right.** Measured as a world point
/// crossing the frame rather than as an angle, because "the view turned
/// right" is a statement about the picture.
#[test]
fn a_mouse_pushed_right_sweeps_the_world_leftward_across_the_frame() {
    for yaw in bearings() {
        let before = camera_at(yaw, 0.0);
        // A landmark dead ahead at 10 m.
        let mark = before.translation + *before.forward() * 10.0;

        let after = camera_at(yaw_after(yaw, 100.0, 0.0022), 0.0);
        // Where the landmark now sits in view space. Turning the view right
        // must carry a point that was centred off to the LEFT — camera space
        // is +X right, so its x goes negative.
        let local = after.to_matrix().inverse().transform_point3(mark);
        assert!(
            local.x < -0.1,
            "yaw {yaw}: after turning right the landmark sat at x={} (it should have gone left)",
            local.x,
        );
        // And it must still be in front of the camera, not behind it: a sign
        // error big enough to spin 180° would otherwise satisfy the check
        // above on alternating bearings.
        assert!(local.z < 0.0, "yaw {yaw}: the landmark ended up behind us");
    }
}

/// The same, mirrored, so a fix that simply negated everything cannot pass.
#[test]
fn a_mouse_pushed_left_sweeps_the_world_rightward() {
    for yaw in bearings() {
        let before = camera_at(yaw, 0.0);
        let mark = before.translation + *before.forward() * 10.0;
        let after = camera_at(yaw_after(yaw, -100.0, 0.0022), 0.0);
        let local = after.to_matrix().inverse().transform_point3(mark);
        assert!(local.x > 0.1, "yaw {yaw}: landmark x={}", local.x);
        assert!(local.z < 0.0, "yaw {yaw}: the landmark ended up behind us");
    }
}
