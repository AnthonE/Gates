//! Your own shadow: the whole body, cast and never drawn.
//!
//! The local player's body is never drawn (the eye is inside its head), and
//! the first-person arms were the only thing of you the sun could see, so
//! your shadow was one arm floating on the ground (operator, 2026-10-21:
//! *"i can only see the shadow of my arm"*). This is the body a remote sees,
//! animated off the predicted body, on [`LAYER`]: only the sun is on that
//! layer as well as the world's, so it lands in the shadow map and in no
//! camera. It holds what you hold ([`hold`]), so the axe is in the shadow
//! too. The first-person arms and their item cast nothing (`viewmodel.rs`).

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use super::anim::{BodyAnim, Reshade, Rig, HAND_BONE};
use super::input::Look;
use super::viewmodel::{Models, SkinMats};
use super::worn::KeepLook;
use super::Net;

/// The render layer only this body and the sun are on. The portrait's is 7.
pub const LAYER: usize = 8;

/// The sun's layers: the world's, and this body's.
pub fn sun_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, LAYER])
}

#[derive(Component, Default)]
pub struct OwnShadow {
    /// The item on the `RightHand` bone, once the scene has one.
    hand: Option<Entity>,
    /// The `HELD_MODELS` row and skin it shows.
    held: Option<(usize, u16)>,
}

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
            OwnShadow::default(),
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

/// Hang an empty item off the body's hand once the scene has spawned, then
/// keep it showing what your first-person hand holds: nothing while
/// holstered in THE GATE or down, as a remote's hand would.
/// Nine parameters, which clippy counts: `portrait::dress`'s set, plus the
/// two scene-graph queries the bone walk needs.
#[allow(clippy::too_many_arguments)]
pub fn hold(
    mut commands: Commands,
    net: NonSend<Net>,
    rig: Res<Rig>,
    models: Res<Models>,
    mut skin_mats: ResMut<SkinMats>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut body: Query<(Entity, &mut OwnShadow)>,
) {
    let Ok((root, mut own)) = body.single_mut() else {
        return;
    };
    let Some(hand) = own.hand else {
        // Bounded walk, `portrait::bind`'s. Nothing found is a scene that
        // has not spawned yet: look again next frame.
        let mut stack = vec![root];
        let mut seen = 0usize;
        while let Some(e) = stack.pop() {
            seen += 1;
            if seen > 512 {
                break;
            }
            if names.get(e).is_ok_and(|n| n.as_str() == HAND_BONE) {
                let hand = commands
                    .spawn((
                        KeepLook,
                        Mesh3d::default(),
                        MeshMaterial3d::<StandardMaterial>::default(),
                        Transform::default(),
                        Visibility::Hidden,
                        ChildOf(e),
                    ))
                    .id();
                own.hand = Some(hand);
                own.held = None;
                return;
            }
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
        }
        return;
    };

    let core = &net.session.core;
    let want = if core.holstered() || core.wounded || core.dead {
        None
    } else {
        crate::ui::hold::held_model_in_hand(&core.catalog, &core.inv, net.sel).map(|row| {
            let skin = core.inv.get(usize::from(net.sel)).map_or(0, |s| s.skin);
            (row, skin)
        })
    };
    if own.held == want {
        return;
    }
    let mut e = commands.entity(hand);
    // The plain look while a skin's material is not ready, and asked again
    // next frame (`portrait::dress`'s rule).
    let mut drawn = true;
    match want {
        Some((row, skin)) => {
            let (mesh, mat) = match models.skinned(row, skin) {
                Some(pair) => pair,
                None => {
                    let (mesh, mat) = models.row(row);
                    let tint = crate::ui::skins::tint_of(&core.skins, skin);
                    let skinned =
                        tint.and_then(|t| skin_mats.for_skin(row, skin, t, &mat, &mut materials));
                    drawn = skinned.is_some() || tint.is_none();
                    (mesh, skinned.unwrap_or(mat))
                }
            };
            e.insert((
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                super::bodies::hand_pose(row, rig.scale),
                Visibility::Inherited,
            ));
        }
        None => {
            e.insert((
                Mesh3d::default(),
                MeshMaterial3d::<StandardMaterial>::default(),
                Visibility::Hidden,
            ));
        }
    }
    let grip = want.is_some_and(|(row, _)| crate::ui::hold::HELD_MODELS[row].grip_roll.is_some());
    commands
        .entity(root)
        .insert(super::fingers::Grip(if grip { 1.0 } else { 0.0 }));
    if drawn {
        own.held = want;
    }
}
