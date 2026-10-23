//! The gun's attachments as you see them (Rust's weapon mods): what a scope
//! does to the view, a laser's dot, a flashlight's beam, and the shares the
//! kick and the crosshair read.
//!
//! Bevy draws, it does not decide. What an attachment does to the shot —
//! the cone the round leaves through, its damage, its speed, the cadence,
//! the magazine — is the shard's (`sim_core::attach`, `CombatContent::
//! modded`); this reads the same rows off the catalog (`protocol::ModRow`)
//! and the mask on the gun in hand (`ClientCore::held_mods`) so the view
//! the player aims through is the gun they are holding.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use protocol::ModRow;

use super::rig::EyeCam;
use super::Net;

/// A sight that narrows the view at least this far is a scope: the view is
/// drawn through its ring and the gun is not drawn over it.
pub const SCOPE_ZOOM: f32 = 0.6;

/// The flashlight's beam: lumens, reach in metres, and its cone (radians).
pub const LIGHT_LUMENS: f32 = 40_000.0;
pub const LIGHT_RANGE_M: f32 = 45.0;
pub const LIGHT_OUTER_RAD: f32 = 0.38;
pub const LIGHT_INNER_RAD: f32 = 0.22;

/// The laser's dot, px across.
pub const LASER_DOT_PX: f32 = 5.0;

/// How fast the scope's ring comes up with the sights, per second.
const SCOPE_EASE: f32 = 14.0;

/// The held gun's attachments, summed for this frame ([`view`]).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ModView {
    /// The share of the gun's kick: 1.0 is the gun's own.
    pub recoil: f32,
    /// The shares of the cone through the sights and from the hip.
    pub aim: f32,
    pub hip: f32,
    /// How far the sights narrow the view, a fraction of the field of view;
    /// 0 keeps the iron sights (`viewmodel::ADS_ZOOM`).
    pub zoom: f32,
    pub quiet: bool,
    pub laser: bool,
    pub light: bool,
}

impl Default for ModView {
    fn default() -> Self {
        Self {
            recoil: 1.0,
            aim: 1.0,
            hip: 1.0,
            zoom: 0.0,
            quiet: false,
            laser: false,
            light: false,
        }
    }
}

/// The view a gun wearing `rows` gives: the shares multiplied, the widest
/// zoom, and any of the three flags.
pub fn sum(rows: impl Iterator<Item = ModRow>) -> ModView {
    let mut v = ModView::default();
    for m in rows {
        v.recoil *= f32::from(m.recoil_pct) / 100.0;
        v.aim *= f32::from(m.aim_pct) / 100.0;
        v.hip *= f32::from(m.hip_pct) / 100.0;
        v.zoom = v.zoom.max(f32::from(m.zoom_pct) / 100.0);
        v.quiet |= m.quiet;
        v.laser |= m.laser;
        v.light |= m.light;
    }
    v
}

/// Read the gun in hand's attachments into [`ModView`]. Before
/// `gunfeel::predict`, which kicks and spreads by it.
pub fn view(net: Option<NonSend<Net>>, mut mv: ResMut<ModView>) {
    let want = net.as_deref().map_or(ModView::default(), |n| {
        let core = &n.session.core;
        sum(core.mod_rows(core.held_mods()).map(|(_, m)| m))
    });
    if *mv != want {
        *mv = want;
    }
}

/// The flashlight on the gun, following the eye.
#[derive(Component)]
pub struct GunLight;

/// The laser's dot, on the aim point.
#[derive(Component)]
pub struct LaserDot;

/// The scope's ring and the black either side of it.
#[derive(Component)]
pub struct ScopeMask;

/// Spawn the beam, the dot and the ring, all off.
pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn((
        SpotLight {
            intensity: 0.0,
            range: LIGHT_RANGE_M,
            outer_angle: LIGHT_OUTER_RAD,
            inner_angle: LIGHT_INNER_RAD,
            color: Color::srgb(1.0, 0.95, 0.85),
            shadows_enabled: false,
            ..default()
        },
        Transform::default(),
        GunLight,
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(50.0),
            width: Val::Px(LASER_DOT_PX),
            height: Val::Px(LASER_DOT_PX),
            margin: UiRect {
                left: Val::Px(-LASER_DOT_PX * 0.5),
                top: Val::Px(-LASER_DOT_PX * 0.5),
                ..default()
            },
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 0.12, 0.08, 0.9)),
        Visibility::Hidden,
        LaserDot,
    ));
    let ring = images.add(scope_image(SCOPE_IMAGE_PX));
    let black = BackgroundColor(Color::BLACK);
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                ..default()
            },
            Visibility::Hidden,
            ScopeMask,
        ))
        .with_children(|c| {
            c.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    ..default()
                },
                black,
            ));
            c.spawn((
                ImageNode::new(ring),
                Node {
                    height: Val::Percent(100.0),
                    aspect_ratio: Some(1.0),
                    ..default()
                },
            ));
            c.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    ..default()
                },
                black,
            ));
        });
}

/// The scope picture's side, px.
const SCOPE_IMAGE_PX: u32 = 512;

/// A scope's view: black outside a ring that fills the square, a soft edge,
/// and a thin crosshair with a gap at the middle for the dot of the aim.
pub fn scope_image(n: u32) -> Image {
    let c = (n as f32 - 1.0) * 0.5;
    let r = n as f32 * 0.47;
    let mut data = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            // Outside the ring: black, the last few px fading in.
            let edge = ((d - (r - 6.0)) / 6.0).clamp(0.0, 1.0);
            // The crosshair: a px either side of the middle lines, open for
            // the middle twentieth.
            let gap = n as f32 * 0.025;
            let line = (dx.abs() <= 1.0 && dy.abs() > gap) || (dy.abs() <= 1.0 && dx.abs() > gap);
            let a = if line { 1.0 } else { edge };
            data.extend_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Draw the attachments: the beam on while the light key is and the gun has
/// a flashlight, the dot while it has a laser, and the scope's ring while an
/// 8x is at the sights — with the gun, which would hide the view, not drawn.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn draw(
    time: Res<Time>,
    net: Option<NonSend<Net>>,
    mv: Res<ModView>,
    gf: Res<super::gunfeel::GunFeel>,
    cam: Query<&GlobalTransform, (With<EyeCam>, Without<GunLight>)>,
    mut light: Query<(&mut SpotLight, &mut Transform), With<GunLight>>,
    mut dot: Query<&mut Visibility, (With<LaserDot>, Without<ScopeMask>)>,
    mut ring: Query<&mut Visibility, (With<ScopeMask>, Without<LaserDot>)>,
    mut rig: Query<
        &mut Visibility,
        (
            With<super::viewmodel::HeldRig>,
            Without<LaserDot>,
            Without<ScopeMask>,
        ),
    >,
    mut up: Local<f32>,
) {
    let gun = gf.gun.is_some();
    let lit = net
        .as_deref()
        .is_some_and(|n| n.light && !n.session.core.dead && !n.session.core.wounded);
    if let (Ok(eye), Ok((mut spot, mut tf))) = (cam.single(), light.single_mut()) {
        let want = if gun && mv.light && lit {
            LIGHT_LUMENS
        } else {
            0.0
        };
        if spot.intensity != want {
            spot.intensity = want;
        }
        if want > 0.0 {
            *tf = eye.compute_transform();
        }
    }
    // The ring rises with the sights, as the zoom does.
    let k = 1.0 - (-SCOPE_EASE * time.delta_secs()).exp();
    let target = if gun && gf.ads && mv.zoom >= SCOPE_ZOOM {
        1.0
    } else {
        0.0
    };
    *up += (target - *up) * k;
    let scoped = *up > 0.85;
    let show = |on: bool| {
        if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    };
    for mut v in &mut ring {
        let want = show(scoped);
        if *v != want {
            *v = want;
        }
    }
    for mut v in &mut dot {
        let want = show(gun && mv.laser && !scoped);
        if *v != want {
            *v = want;
        }
    }
    for mut v in &mut rig {
        let want = show(!scoped);
        if *v != want {
            *v = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(bit: u8) -> ModRow {
        ModRow {
            bit,
            aim_pct: 100,
            hip_pct: 100,
            recoil_pct: 100,
            ..ModRow::default()
        }
    }

    /// Two attachments' shares multiply; the widest zoom wins; flags add.
    #[test]
    fn a_guns_attachments_sum_into_one_view() {
        let holo = ModRow {
            aim_pct: 80,
            zoom_pct: 40,
            ..row(0)
        };
        let brake = ModRow {
            recoil_pct: 50,
            ..row(3)
        };
        let laser = ModRow {
            aim_pct: 80,
            hip_pct: 50,
            laser: true,
            ..row(5)
        };
        let v = sum([holo, brake, laser].into_iter());
        assert!((v.aim - 0.64).abs() < 1e-6);
        assert!((v.hip - 0.5).abs() < 1e-6);
        assert!((v.recoil - 0.5).abs() < 1e-6);
        assert!((v.zoom - 0.4).abs() < 1e-6);
        assert!(v.laser && !v.light && !v.quiet);
        assert_eq!(sum(std::iter::empty()), ModView::default());
    }

    /// The scope's picture is see-through in the middle, black at the corner,
    /// and carries its crosshair.
    #[test]
    fn the_scope_ring_is_clear_inside_and_black_outside() {
        let n = 64;
        let img = scope_image(n);
        let data = img.data.expect("pixels");
        let a = |x: u32, y: u32| data[((y * n + x) * 4 + 3) as usize];
        assert_eq!(a(0, 0), 255, "the corner is black");
        assert_eq!(a(20, 20), 0, "inside the ring is clear");
        assert_eq!(a(n / 2, 5), 255, "the crosshair's upright");
    }
}
