//! **Triple T, on the menu and in the inventory** (operator, 2026-10-12: the
//! meme is the base skin, so show him off). The player body — the same rig,
//! the same idle, the same bat and the same clothes a remote body draws — on
//! its own render layer, filmed by its own camera into an image the UI shows
//! ([`view`]). Rust's inventory has the same figure in its middle.
//!
//! **It decides nothing**: what it wears and holds is read off `ClientCore`
//! like every other body's, and on the menu (no session) it is the base look
//! with the starting bat.
//!
//! The camera renders only while a [`PortraitView`] node exists, so a closed
//! inventory costs nothing. The body lives far under the island on
//! [`LAYER`], which only this camera and its lights see; the scene's
//! descendants inherit the layer through Bevy's `Propagate`.

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use client_core::core::WornLook;

use super::anim::{BodyAnim, Clip, Reshade, Rig, HAND_BONE};
use super::screen::Screen;
use super::viewmodel::{Models, SkinMats};
use super::worn::{KeepLook, WornKit, WornRig};
use super::Net;

/// The render layer only the portrait's camera, lights and body are on.
pub const LAYER: usize = 7;

/// The picture's size, px: an upright figure, 2:3.
pub const W_PX: u32 = 400;
pub const H_PX: u32 = 600;

/// Where the body stands: well under the island, out of every world query's
/// way. Only the layer keeps it out of the world's frame; this keeps it out
/// of everything else.
const AT: Vec3 = Vec3::new(0.0, -400.0, 0.0);

/// A three-quarter turn toward the picture's left, radians about up: a
/// figure square to the lens reads as a mugshot.
const TURN: f32 = 0.42;

/// Vertical field of view, degrees, and the distance it frames the whole
/// 1.8 m body (and a raised bat) from.
const FOV_DEG: f32 = 26.0;
const DIST_M: f32 = 4.9;
/// Where the lens looks, metres above the feet: just under the chest.
const AIM_Y_M: f32 = 0.95;

/// On the menu he swings his bat every this many seconds.
const SWING_EVERY_S: f32 = 6.5;

#[derive(Resource)]
pub struct Portrait {
    pub image: Handle<Image>,
    body: Option<Entity>,
    hand: Option<Entity>,
    /// The `HELD_MODELS` row and skin the hand is showing.
    held: Option<(usize, u16)>,
    /// Seconds until the menu swing.
    swing_in: f32,
}

#[derive(Component)]
pub struct PortraitCam;

#[derive(Component)]
pub struct PortraitBody;

/// A UI node showing the picture. While one exists the camera renders.
#[derive(Component)]
pub struct PortraitView;

/// The picture as a UI node, `w`×`h` px.
pub fn view(portrait: &Portrait, w: f32, h: f32) -> impl Bundle {
    (
        PortraitView,
        ImageNode::new(portrait.image.clone()),
        Node {
            width: Val::Px(w),
            height: Val::Px(h),
            ..default()
        },
        Pickable::IGNORE,
    )
}

/// Is the portrait up on a screen with no world behind it? The run
/// condition that lets the body systems animate him on the menu.
pub fn on_menu(state: Res<State<Screen>>) -> bool {
    matches!(state.get(), Screen::Menu)
}

/// The target, the camera and the lights (`Startup`). The body waits for
/// the rig ([`spawn`]).
pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_target_texture(
        W_PX,
        H_PX,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let layer = RenderLayers::layer(LAYER);
    let aim = AT + Vec3::Y * AIM_Y_M;
    commands.spawn((
        PortraitCam,
        Camera3d::default(),
        Camera {
            // Before every window camera, so the UI reads this frame's picture.
            order: -10,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV_DEG.to_radians(),
            near: 0.1,
            far: 20.0,
            ..default()
        }),
        Tonemapping::TonyMcMapface,
        Msaa::Sample4,
        Transform::from_translation(aim + Vec3::Z * DIST_M).looking_at(aim, Vec3::Y),
        layer.clone(),
    ));
    // Key high on the camera's left, a cool weak fill from the right, and a
    // warm rim from behind that pulls the dark bark off a dark menu.
    for (lux, color, from) in [
        (3_400.0, Color::WHITE, Vec3::new(-0.7, 1.0, 0.8)),
        (
            900.0,
            Color::srgb(0.80, 0.86, 1.0),
            Vec3::new(1.0, 0.3, 0.5),
        ),
        (
            2_400.0,
            Color::srgb(1.0, 0.86, 0.66),
            Vec3::new(0.4, 0.7, -1.0),
        ),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance: lux,
                color,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(AT + from).looking_at(AT, Vec3::Y),
            layer.clone(),
        ));
    }
    commands.insert_resource(Portrait {
        image,
        body: None,
        hand: None,
        held: None,
        swing_in: SWING_EVERY_S * 0.5,
    });
}

/// Spawn the body once the rig has loaded. Every frame until then; one
/// branch after.
pub fn spawn(mut commands: Commands, rig: Res<Rig>, mut portrait: ResMut<Portrait>) {
    if portrait.body.is_some() || !rig.ready() {
        return;
    }
    let Some(scene) = rig.scene.clone() else {
        return;
    };
    let body = commands
        .spawn((
            PortraitBody,
            BodyAnim {
                clip: Some(Clip::Idle),
                ..default()
            },
            Reshade::body(),
            super::fingers::Grip::default(),
            SceneRoot(scene),
            Transform::from_translation(AT)
                .with_rotation(Quat::from_rotation_y(TURN))
                .with_scale(Vec3::splat(rig.scale)),
            Propagate(RenderLayers::layer(LAYER)),
        ))
        .id();
    portrait.body = Some(body);
}

/// Turn the camera on while something shows the picture.
pub fn activate(
    views: Query<(), With<PortraitView>>,
    mut cams: Query<&mut Camera, With<PortraitCam>>,
) {
    let on = !views.is_empty();
    for mut cam in &mut cams {
        if cam.is_active != on {
            cam.is_active = on;
        }
    }
}

/// Keep his clocks running and, on the menu, swing the bat now and then.
pub fn idle(
    time: Res<Time>,
    state: Res<State<Screen>>,
    mut portrait: ResMut<Portrait>,
    mut bodies: Query<&mut BodyAnim, With<PortraitBody>>,
) {
    let dt = time.delta_secs();
    for mut anim in &mut bodies {
        // Standing still at `AT`, so the gait stays the idle.
        anim.observe(AT, dt, false, false, false);
        if matches!(state.get(), Screen::Menu) {
            portrait.swing_in -= dt;
            if portrait.swing_in <= 0.0 {
                portrait.swing_in = SWING_EVERY_S;
                anim.swing();
            }
        }
    }
}

/// Find the hand bone and the clothing rig once the scene has spawned, and
/// hang an empty, hidden hand off the bone.
#[allow(clippy::too_many_arguments)]
pub fn bind(
    mut commands: Commands,
    mut portrait: ResMut<Portrait>,
    unbound: Query<Entity, (With<PortraitBody>, Without<WornRig>)>,
    children: Query<&Children>,
    names: Query<&Name>,
    skinned: Query<(), With<SkinnedMesh>>,
) {
    for body in &unbound {
        let rig = WornRig::under(body, &children, &names, &skinned);
        if rig.skin.is_none() {
            continue; // the scene has not spawned yet
        }
        let mut stack = vec![body];
        let mut bone = None;
        let mut seen = 0usize;
        while let Some(e) = stack.pop() {
            seen += 1;
            if seen > 512 {
                break;
            }
            if names.get(e).is_ok_and(|n| n.as_str() == HAND_BONE) {
                bone = Some(e);
                break;
            }
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
        }
        if let Some(bone) = bone {
            let hand = commands
                .spawn((
                    KeepLook,
                    Mesh3d::default(),
                    MeshMaterial3d::<StandardMaterial>::default(),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(bone),
                ))
                .id();
            portrait.hand = Some(hand);
            portrait.held = None;
        }
        commands.entity(body).insert(rig);
    }
}

/// What he wears and holds: yours in the world, the base look and the bat on
/// the menu. Written on a change only.
#[allow(clippy::too_many_arguments)]
pub fn dress(
    mut commands: Commands,
    net: Option<NonSend<Net>>,
    rig: Res<Rig>,
    models: Res<Models>,
    kit: Option<Res<WornKit>>,
    binds: Res<Assets<SkinnedMeshInverseBindposes>>,
    skins: Query<&SkinnedMesh>,
    mut skin_mats: ResMut<SkinMats>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut portrait: ResMut<Portrait>,
    mut bodies: Query<(Entity, &mut WornRig), With<PortraitBody>>,
) {
    let Ok((body, mut worn)) = bodies.single_mut() else {
        return;
    };
    let core = net.as_deref().map(|n| (&n.session.core, n.sel));

    // The hand. On the menu, the bat every player starts with; in THE GATE,
    // holstered like the first-person hands.
    let want = match core {
        Some((c, _)) if c.holstered() => None,
        Some((c, sel)) => crate::ui::hold::held_model_in_hand(&c.catalog, &c.inv, sel).map(|row| {
            let skin = c.inv.get(usize::from(sel)).map_or(0, |s| s.skin);
            (row, skin)
        }),
        None => crate::ui::hold::HELD_MODELS
            .iter()
            .position(|m| m.key == "bat")
            .map(|row| (row, 0)),
    };
    if let (Some(hand), true) = (portrait.hand, portrait.held != want) {
        let mut e = commands.entity(hand);
        match want {
            Some((row, skin)) => {
                let (mesh, mat) = models.row(row);
                let tint = core.and_then(|(c, _)| crate::ui::skins::tint_of(&c.skins, skin));
                let mat = tint
                    .and_then(|t| skin_mats.for_skin(row, skin, t, &mat, &mut materials))
                    .unwrap_or(mat);
                e.insert((
                    Mesh3d(mesh),
                    MeshMaterial3d(mat),
                    super::bodies::hand_pose(row, rig.scale),
                    Visibility::Inherited,
                ));
            }
            None => {
                e.insert(Visibility::Hidden);
            }
        }
        let grip =
            want.is_some_and(|(row, _)| crate::ui::hold::HELD_MODELS[row].grip_roll.is_some());
        commands
            .entity(body)
            .insert(super::fingers::Grip(if grip { 1.0 } else { 0.0 }));
        portrait.held = want;
    }

    // The clothes.
    let (Some(kit), Some((c, _))) = (kit, core) else {
        return;
    };
    let look = WornLook {
        items: c.worn.map(|s| {
            if s.count == 0 {
                sim_core::gather::NO_ITEM
            } else {
                s.item
            }
        }),
        skins: c.worn.map(|s| if s.count == 0 { 0 } else { s.skin }),
    };
    super::worn::put_on(
        &mut commands,
        &kit,
        (&binds, &skins),
        (&mut skin_mats, &mut materials),
        c,
        &mut worn,
        look,
    );
}

/// Back on the menu with no session: take the clothes off.
pub fn undress(mut commands: Commands, mut bodies: Query<&mut WornRig, With<PortraitBody>>) {
    for mut worn in &mut bodies {
        worn.strip(&mut commands);
    }
}
