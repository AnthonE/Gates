//! Planting by hand (farming v2): a seed in hand and a planter under the
//! crosshair draw a translucent sprout on the free bed spot the click would
//! plant, and the left click plants it (`ACT_PLANT`, the held slot on the
//! wire). Which spot is `ui::farm::plant_target`'s arithmetic; whether it
//! may is the sim's verdict, refused in words (`ui::refusals`). Nothing is
//! predicted.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use sim_core::deploy::{box_key, ARCH_PLANTER, STATION_REACH_M};

use crate::look::{pitch_u8, yaw_u16};
use crate::ui::hold::{click_in_hand, Click};

use super::crops::CropKit;
use super::hud::Toast;
use super::input::Look;
use super::panels::{Panel, Ui};
use super::verbs::Aimed;
use super::{Net, WorldEntity, WorldId, CROUCH_EYE_M, EYE_HEIGHT};

/// The planting ghost's sprout.
#[derive(Component)]
pub struct SproutGhost;

/// How much bigger than a real seedling the ghost draws, so it reads.
const GHOST_SCALE: f32 = 2.0;

/// Track the bed spot a seed in hand would go into, draw the ghost there,
/// and plant it on the left click.
#[allow(clippy::too_many_arguments)]
pub fn plant_ghost(
    mut commands: Commands,
    net: NonSend<Net>,
    look: Res<Look>,
    aimed: Res<Aimed>,
    world: Option<Res<WorldId>>,
    kit: Option<Res<CropKit>>,
    mouse: Res<ButtonInput<MouseButton>>,
    ui: Option<Res<Ui>>,
    chat: Option<Res<super::chat::Chat>>,
    reports: Option<Res<super::report::Reports>>,
    mut toast: ResMut<Toast>,
    mut ghost: Query<(&mut Transform, &mut Visibility), With<SproutGhost>>,
) {
    let core = &net.session.core;
    let click = click_in_hand(
        &core.catalog,
        &core.research,
        &core.deploy_defs,
        core.deploy_defs_have,
        &core.inv,
        net.sel,
    );
    // A panel, the chat composer or the report overlay owns the click (and
    // the ghost hides), as `ghost::deploy_track` and `input.rs` have it: a
    // text field is a text field whether it sits in a panel or not.
    let busy = ui.as_ref().is_some_and(|u| u.panel != Panel::None)
        || chat.as_ref().is_some_and(|c| c.open())
        || reports.as_ref().is_some_and(|r| r.open);
    let pick = &aimed.0;
    let target = match world.as_deref() {
        Some(w)
            if !busy
                && click == Click::Plant
                && !core.wounded
                && !core.dead
                && pick.arch == ARCH_PLANTER
                && pick.handle != 0 =>
        {
            core.deploys
                .entries()
                .iter()
                .find(|d| box_key(d.cx, d.cz, d.level, d.loc) == pick.handle)
                .and_then(|rec| {
                    let [x, y, z] = core.predict.render_position();
                    let (dx, dz) = (rec.xz().0 - x, rec.xz().1 - z);
                    if dx * dx + dz * dz > STATION_REACH_M * STATION_REACH_M {
                        return None;
                    }
                    let floor = sim_core::deploy::body_base_y(
                        w.seed,
                        &w.haven,
                        core.pieces.cols(),
                        rec,
                        ARCH_PLANTER,
                    );
                    let eye_h = if core.crouched() {
                        CROUCH_EYE_M
                    } else {
                        EYE_HEIGHT
                    };
                    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
                    let (ch, sv) = sim_core::pitch_dir(pitch_u8(look.pitch));
                    crate::ui::farm::plant_target(
                        rec,
                        floor,
                        (x, y + eye_h, z),
                        (fx * ch, sv, fz * ch),
                        core.plants.entries(),
                    )
                })
        }
        _ => None,
    };

    match ghost.single_mut() {
        Ok((mut t, mut vis)) => {
            let want = if target.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *vis != want {
                *vis = want;
            }
            if let Some((_, _, (x, y, z))) = target {
                let at = Vec3::new(x, y, z);
                if t.translation != at {
                    t.translation = at;
                }
            }
        }
        Err(_) => {
            if let Some(kit) = kit.as_deref() {
                commands.spawn((
                    WorldEntity,
                    SproutGhost,
                    Mesh3d(kit.ghost_mesh.clone()),
                    MeshMaterial3d(kit.ghost.clone()),
                    Transform::from_scale(Vec3::splat(GHOST_SCALE)),
                    Visibility::Hidden,
                    NotShadowCaster,
                ));
            }
        }
    }

    let Some((host, spot, _)) = target else {
        return;
    };
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_plant(
        net.sel,
        protocol::PlantTarget::Bed { host, spot },
        &mut buf,
    ) {
        Ok(len) => {
            if let Err(e) = net.session.send_action(&buf[..len]) {
                toast.warn(e.to_string());
            }
        }
        Err(_) => toast.warn("that seed cannot be planted from here"),
    }
}
