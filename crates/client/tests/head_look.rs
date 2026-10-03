//! Gate: a remote's head points where the wire says it is looking, and keeps
//! pointing there.
//!
//! `tests/music.rs`'s shape — the WIRING, on a bare `App` with no window, no
//! device and no assets. Three things in `anim::pose_spine` can be wrong,
//! none is a compile error, and each draws a body that is subtly not a person:
//!
//!   1. **The axis.** "Look up" is a rotation about the body's RIGHT, and
//!      which local axis that is depends on how the exporter oriented the
//!      neck. This rig's bones are not axis-aligned, so a typed `Vec3::X`
//!      gives a head that swings sideways instead of nodding. The axis is
//!      derived from the rest pose instead, and the fixture below rolls the
//!      neck 90° precisely so a typed axis fails it.
//!
//!   2. **Accumulation.** The override composes onto whatever the animation
//!      player wrote. On a frame the clip did not touch the bone, its own
//!      previous delta must come out first, or a clip that leaves the head
//!      alone winds it into the ground at sixty frames a second.
//!
//!   3. **Cancellation**, the other side of 2, and the one that shipped. On a
//!      frame the clip DID rewrite the bone — every frame, on this rig —
//!      removing last frame's delta takes this frame's back out, and the head
//!      holds the clip's gaze whatever the wire says. The fixture's old
//!      `AnimationPlayer` never wrote anything, so only case 2 was ever tested
//!      and every remote head in the game was frozen. [`Clip`] below writes the
//!      rest pose back every frame, the way a real clip does.
//!
//! No glTF is loaded. The fixture is a hand-built hierarchy of the same SHAPE
//! — body → chest → neck → head, with `Name` and an `AnimationPlayer` — which
//! is what the systems actually read. That the shipped rig has a bone called
//! `Head` is `tests/rig_asset.rs`'s business, not this file's.
//!
//! The look is shared down the chain (chest, neck, head), so the gaze is
//! measured off the head in the BODY's frame, which is the sum of all three.

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::anim::{bind_spine, pose_spine, BodyAnim, ANIM_LOOK_PITCH_MAX};

/// The rotation baked into the fixture's neck. Non-identity on purpose: with a
/// zero-rotation chain every axis convention agrees and the gate proves
/// nothing.
///
/// **A ROLL and not a yaw, and the first draft of this file got that wrong in
/// a way worth keeping.** A 90° yaw also breaks a typed axis — but it turns
/// the head to face along the body's right, which is the very axis the pitch
/// rotates about, so the gaze becomes parallel to it and *nothing* can tilt
/// it. Three tests failed against correct code. A roll puts the head's frame
/// 90° out while leaving the gaze perpendicular to the pitch axis, so a wrong
/// axis is visible and a right one works. **A fixture that cannot express the
/// effect is not a strict test, it is a broken one.**
const NECK_ROLL: f32 = std::f32::consts::FRAC_PI_2;

struct Fixture {
    app: App,
    body: Entity,
    head: Entity,
}

/// A stand-in for an animation clip: writes every bone's rest rotation back
/// each frame, before the override, exactly as the animation player does for
/// a bone its clip animates. Off unless [`fixture_animated`] turns it on.
#[derive(Resource, Default)]
struct Clip(Vec<(Entity, Quat)>);

fn play_clip(clip: Res<Clip>, mut xf: Query<&mut Transform>) {
    for &(e, q) in &clip.0 {
        if let Ok(mut t) = xf.get_mut(e) {
            t.rotation = q;
        }
    }
}

fn fixture() -> Fixture {
    let mut app = App::new();
    app.init_resource::<Clip>();
    app.add_systems(Update, (play_clip, bind_spine, pose_spine).chain());

    let world = app.world_mut();
    let head = world
        .spawn((
            Name::new("Head"),
            Transform::IDENTITY,
            AnimationPlayer::default(),
        ))
        .id();
    let neck = world
        .spawn((
            Name::new("neck"),
            Transform::from_rotation(Quat::from_rotation_z(NECK_ROLL)),
        ))
        .id();
    let chest = world.spawn((Name::new("Spine"), Transform::IDENTITY)).id();
    let body = world.spawn((BodyAnim::default(), Transform::IDENTITY)).id();
    world.entity_mut(neck).add_child(head);
    world.entity_mut(chest).add_child(neck);
    world.entity_mut(body).add_child(chest);

    app.update();
    Fixture { app, body, head }
}

/// The fixture with every bone re-posed by a "clip" each frame.
fn fixture_animated() -> Fixture {
    let mut f = fixture();
    let w = f.app.world_mut();
    let mut rest = Vec::new();
    let mut at = f.head;
    while at != f.body {
        rest.push((at, w.get::<Transform>(at).unwrap().rotation));
        at = w.get::<ChildOf>(at).unwrap().0;
    }
    w.resource_mut::<Clip>().0 = rest;
    f
}

impl Fixture {
    fn look(&mut self, pitch: f32) {
        let mut anim = self.app.world_mut().get_mut::<BodyAnim>(self.body).unwrap();
        anim.pitch = pitch;
        self.app.update();
    }

    /// The head's rotation relative to the BODY, composed from the local
    /// transforms the systems write — which is what the renderer will
    /// propagate.
    fn head_in_body_space(&self) -> Quat {
        let w = self.app.world();
        // Head, then up through the neck and the chest.
        let mut q = w.get::<Transform>(self.head).unwrap().rotation;
        let neck = w.get::<ChildOf>(self.head).unwrap().0;
        q = w.get::<Transform>(neck).unwrap().rotation * q;
        let chest = w.get::<ChildOf>(neck).unwrap().0;
        w.get::<Transform>(chest).unwrap().rotation * q
    }

    /// Where the head is facing, in the body's frame. The rig's own forward
    /// is **+Z** — glTF's convention, the sim's yaw 0, and what the walk clip
    /// measures. This said `-Z` until 2026-10-03, which paired with a +X pitch
    /// axis to pass here while the real rig, facing +Z, would have looked
    /// down when the wire said up.
    fn gaze(&self) -> Vec3 {
        self.head_in_body_space() * Vec3::Z
    }
}

#[test]
fn a_level_look_leaves_the_pose_alone() {
    let mut f = fixture();
    f.look(0.0);
    let g = f.gaze();
    // The neck's roll turned the head's own frame; what matters is that a
    // level look leaves the gaze level.
    assert!(
        g.y.abs() < 1e-4,
        "a level look tilted the head to y {:.4}",
        g.y
    );
}

#[test]
fn looking_up_raises_the_gaze_and_looking_down_lowers_it() {
    let mut f = fixture();
    f.look(0.5);
    let up = f.gaze();
    f.look(-0.5);
    let down = f.gaze();
    assert!(
        up.y > 0.4,
        "looking up 0.5 rad raised the gaze only to y {:.3} — the derived \
         pitch axis is wrong, which is what a typed Vec3::X gives on a rig \
         whose neck is rolled",
        up.y
    );
    assert!(down.y < -0.4, "looking down gave y {:.3}", down.y);
    assert!(
        (up.y + down.y).abs() < 1e-3,
        "up and down are not symmetric: {:.4} vs {:.4}",
        up.y,
        down.y
    );
}

#[test]
fn the_head_nods_rather_than_swinging_sideways() {
    // The failure a typed axis actually produces on this fixture: the gaze
    // stays level and swings across the body instead of rising. So the test is
    // on the SIDEWAYS component — a pure nod keeps the gaze in the body's
    // forward/up plane, whatever the neck's own frame is doing.
    let mut f = fixture();
    f.look(0.0);
    let level = f.gaze();
    for p in [0.2, 0.45, 0.6, -0.35] {
        f.look(p);
        let g = f.gaze();
        assert!(
            (g.x - level.x).abs() < 1e-3,
            "at pitch {p} the gaze swung sideways: x {:.4} -> {:.4}",
            level.x,
            g.x
        );
        assert!(
            (g.y - p.sin()).abs() < 1e-3,
            "at pitch {p} the gaze rose to {:.4}, not {:.4}",
            g.y,
            p.sin()
        );
    }
}

#[test]
fn the_look_is_clamped_rather_than_scaled() {
    let mut f = fixture();
    f.look(ANIM_LOOK_PITCH_MAX);
    let at_limit = f.gaze();
    f.look(1.55);
    let past_limit = f.gaze();
    assert!(
        (at_limit - past_limit).length() < 1e-3,
        "a look past the limit was not clamped: {at_limit:?} vs {past_limit:?}"
    );
}

#[test]
fn a_clip_that_never_touches_the_head_does_not_wind_it_into_the_ground() {
    // The fixture's `AnimationPlayer` writes nothing, ever — which is exactly
    // the case a real clip with no head channel produces. Without the delta
    // being removed each frame, sixty frames of 0.6 rad is nine full turns.
    let mut f = fixture();
    f.look(0.6);
    let first = f.gaze();
    for _ in 0..60 {
        f.look(0.6);
    }
    let after = f.gaze();
    assert!(
        (first - after).length() < 1e-4,
        "the head drifted over 60 frames: {first:?} -> {after:?}"
    );
}

#[test]
fn the_head_follows_a_change_of_mind() {
    // Not the same test as the one above: that one holds a value, this one
    // proves the override still tracks after it has been applied once.
    let mut f = fixture();
    f.look(0.7);
    for _ in 0..10 {
        f.look(0.7);
    }
    f.look(-0.7);
    assert!(f.gaze().y < -0.5, "the head did not follow back down");
    f.look(0.0);
    assert!(f.gaze().y.abs() < 1e-4, "the head did not return to level");
}

#[test]
fn a_clip_that_rewrites_the_head_every_frame_does_not_cancel_the_look() {
    // Case 3 of the header: the one every remote in the game was in.
    let mut f = fixture_animated();
    for p in [0.5, 0.5, 0.5, -0.4, -0.4, 0.0] {
        f.look(p);
        let g = f.gaze();
        assert!(
            (g.y - p.sin()).abs() < 1e-3,
            "with the clip re-posing the head, a look of {p} rose to {:.4}, not {:.4}",
            g.y,
            p.sin()
        );
    }
}

#[test]
fn the_look_bends_the_chest_as_well_as_the_head() {
    let mut f = fixture();
    let neck = f.app.world().get::<ChildOf>(f.head).unwrap().0;
    let chest = f.app.world().get::<ChildOf>(neck).unwrap().0;
    let rest = f.app.world().get::<Transform>(chest).unwrap().rotation;
    f.look(1.0);
    let bent = f.app.world().get::<Transform>(chest).unwrap().rotation;
    assert!(
        rest.angle_between(bent) > 0.05,
        "a steep look left the chest where it was"
    );
}
