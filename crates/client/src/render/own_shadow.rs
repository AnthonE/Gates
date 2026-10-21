//! Your own shadow: the whole body, cast and never drawn.
//!
//! The local player's body is never drawn (the eye is inside its head), and
//! the first-person arms were the only thing of you the sun could see, so
//! your shadow was one arm floating on the ground (operator, 2026-10-21:
//! *"i can only see the shadow of my arm"*). This is the body a remote sees,
//! animated off the predicted body, on [`LAYER`]: only the sun is on that
//! layer as well as the world's, so it lands in the shadow map and in no
//! camera. The arms and the held item cast nothing (`viewmodel.rs`).

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use super::anim::{BodyAnim, Reshade, Rig};
use super::input::Look;
use super::Net;

/// The render layer only this body and the sun are on. The portrait's is 7.
pub const LAYER: usize = 8;

/// The sun's layers: the world's, and this body's.
pub fn sun_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, LAYER])
}

#[derive(Component)]
pub struct OwnShadow;

/// Spawn the body once the rig is in, then stand it on the predicted feet
/// every frame, turned the way you face, with the gait, crouch and swing a
/// remote would see. Hidden while there is no body of yours to shadow: a
/// spectator's eye is somebody else's, and a corpse is the death's bag.
pub fn stream(
    mut commands: Commands,
    rig: Res<Rig>,
    net: NonSend<Net>,
    look: Res<Look>,
    time: Res<Time>,
    feed: Res<super::feed::Feed>,
    mut body: Query<(&mut Transform, &mut BodyAnim, &mut Visibility), With<OwnShadow>>,
) {
    if !rig.ready() {
        return;
    }
    let core = &net.session.core;
    let [x, y, z] = core.eye_position();
    let feet = Vec3::new(x, y, z);
    let turn = Quat::from_rotation_y(look.yaw);
    let shown = core.predict.started && !core.spectating() && !core.dead;
    let hold = !core.holstered()
        && crate::ui::hold::held_model_in_hand(&core.catalog, &core.inv, net.sel)
            .is_some_and(|row| crate::ui::hold::HELD_MODELS[row].long_gun());

    let Ok((mut t, mut anim, mut vis)) = body.single_mut() else {
        let Some(scene) = rig.scene.clone() else {
            return;
        };
        commands.spawn((
            super::WorldEntity,
            OwnShadow,
            BodyAnim::default(),
            // Found once by the finger bones' climb (`fingers::pose`).
            super::fingers::Grip::default(),
            Reshade::body(),
            SceneRoot(scene),
            Transform::from_translation(feet)
                .with_rotation(turn)
                .with_scale(Vec3::splat(rig.scale)),
            Visibility::Hidden,
            Propagate(RenderLayers::layer(LAYER)),
        ));
        return;
    };

    let want = if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *vis != want {
        *vis = want;
    }
    if !shown {
        return;
    }
    if t.translation != feet {
        t.translation = feet;
    }
    if t.rotation != turn {
        t.rotation = turn;
    }
    anim.airborne = !core.predict.body.grounded;
    anim.crouched = core.crouched();
    anim.facing = look.yaw;
    anim.hold = hold;
    anim.observe(feet, time.delta_secs(), false, false, core.wounded);
    anim.pitch = look.pitch;
    let me = core.player_id;
    if feed.swings().contains(&me) {
        anim.swing();
    }
    if !anim.hold && feed.shots().iter().any(|sh| sh.0 == me) {
        anim.shoot();
    }
}
