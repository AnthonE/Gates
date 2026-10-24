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

use super::bodies::Bodies;
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

/// How many other players' gun lights and lasers are drawn at once, the
/// nearest first: each light is a spot the renderer pays for.
pub const REMOTE_LIGHTS: usize = 4;
pub const REMOTE_LASERS: usize = 8;

/// How far a laser reaches and the march it finds the ground by, metres.
pub const LASER_RANGE_M: f32 = 50.0;
const LASER_STEP_M: f32 = 1.0;
/// The beam's thickness and the dot's radius, metres.
const LASER_WIDTH_M: f32 = 0.006;
const LASER_DOT_M: f32 = 0.035;

/// One of the pool of other players' gun lights ([`remote`]).
#[derive(Component)]
pub struct RemoteLight(pub usize);

/// One of the pool of other players' laser beams, and its dot.
#[derive(Component)]
pub struct RemoteLaser(pub usize);

#[derive(Component)]
pub struct RemoteDot(pub usize);

/// Spawn the pools, all off.
pub fn remote_setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for i in 0..REMOTE_LIGHTS {
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
            RemoteLight(i),
        ));
    }
    let red = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 0.1, 0.06, 0.55),
        emissive: LinearRgba::rgb(6.0, 0.3, 0.2),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    let beam = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let dot = meshes.add(Sphere::new(1.0));
    for i in 0..REMOTE_LASERS {
        commands.spawn((
            Mesh3d(beam.clone()),
            MeshMaterial3d(red.clone()),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            RemoteLaser(i),
        ));
        commands.spawn((
            Mesh3d(dot.clone()),
            MeshMaterial3d(red.clone()),
            Transform::default(),
            Visibility::Hidden,
            bevy::light::NotShadowCaster,
            RemoteDot(i),
        ));
    }
}

/// Where a laser from `at` along `dir` meets the ground, within
/// [`LASER_RANGE_M`], or `None` when it runs out first.
pub fn laser_end(seed: u64, at: Vec3, dir: Vec3) -> Option<Vec3> {
    let n = (LASER_RANGE_M / LASER_STEP_M) as usize;
    (1..=n)
        .map(|k| at + dir * (k as f32 * LASER_STEP_M))
        .find(|p| p.y <= sim_core::terrain::height(seed, p.x, p.z))
}

/// Other players' lasers and flashlights, Rust's: everyone sees a lit
/// gun's beam and a laser's dot. The nearest few are drawn, from the
/// muzzle `bodies::aim_long_guns` laid this frame, with which attachment
/// is which read off the catalog's rows (`ClientCore::mod_rows`).
#[allow(clippy::type_complexity)]
pub fn remote(
    net: Option<NonSend<Net>>,
    bodies: Res<Bodies>,
    cam: Query<&GlobalTransform, With<EyeCam>>,
    mut lights: Query<(
        &RemoteLight,
        &mut SpotLight,
        &mut Transform,
        &mut GlobalTransform,
    )>,
    mut lasers: Query<
        (
            &RemoteLaser,
            &mut Transform,
            &mut GlobalTransform,
            &mut Visibility,
        ),
        (Without<RemoteLight>, Without<RemoteDot>),
    >,
    mut dots: Query<
        (
            &RemoteDot,
            &mut Transform,
            &mut GlobalTransform,
            &mut Visibility,
        ),
        (Without<RemoteLight>, Without<RemoteLaser>),
    >,
) {
    let eye = cam.single().map_or(Vec3::ZERO, |g| g.translation());
    // (distance², muzzle, barrel, laser, light), nearest first.
    let mut lit: [(f32, Vec3, Vec3, bool, bool); REMOTE_LASERS] =
        [(f32::MAX, Vec3::ZERO, Vec3::Z, false, false); REMOTE_LASERS];
    let mut seed = 0;
    if let Some(n) = net.as_deref() {
        let core = &n.session.core;
        seed = core.seed();
        for (_, at, dir, mask) in bodies.beams() {
            let v = sum(core.mod_rows(mask).map(|(_, m)| m));
            if !v.laser && !v.light {
                continue;
            }
            let d2 = at.distance_squared(eye);
            if let Some(worst) = lit
                .iter_mut()
                .filter(|e| e.0 > d2)
                .max_by(|a, b| a.0.total_cmp(&b.0))
            {
                *worst = (d2, at, dir, v.laser, v.light);
            }
        }
    }
    lit.sort_by(|a, b| a.0.total_cmp(&b.0));
    let show = |on: bool| {
        if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    };
    let set = |tf: &mut Transform, gt: &mut GlobalTransform, t: Transform| {
        *tf = t;
        *gt = GlobalTransform::from(t);
    };
    for (RemoteLight(i), mut spot, mut tf, mut gt) in &mut lights {
        let e = lit.iter().filter(|e| e.4 && e.0 < f32::MAX).nth(*i);
        let want = if e.is_some() { LIGHT_LUMENS } else { 0.0 };
        if spot.intensity != want {
            spot.intensity = want;
        }
        if let Some(&(_, at, dir, _, _)) = e {
            set(
                &mut tf,
                &mut gt,
                Transform::from_translation(at).looking_to(dir, Vec3::Y),
            );
        }
    }
    let beams = || lit.iter().filter(|e| e.3 && e.0 < f32::MAX);
    for (RemoteLaser(i), mut tf, mut gt, mut vis) in &mut lasers {
        let e = beams().nth(*i);
        if let Some(&(_, at, dir, _, _)) = e {
            let end = laser_end(seed, at, dir).unwrap_or(at + dir * LASER_RANGE_M);
            let len = at.distance(end);
            set(
                &mut tf,
                &mut gt,
                Transform::from_translation((at + end) * 0.5)
                    .looking_to(dir, Vec3::Y)
                    .with_scale(Vec3::new(LASER_WIDTH_M, LASER_WIDTH_M, len)),
            );
        }
        let want = show(e.is_some());
        if *vis != want {
            *vis = want;
        }
    }
    for (RemoteDot(i), mut tf, mut gt, mut vis) in &mut dots {
        let hit = beams()
            .nth(*i)
            .and_then(|&(_, at, dir, _, _)| laser_end(seed, at, dir));
        if let Some(p) = hit {
            set(
                &mut tf,
                &mut gt,
                Transform::from_translation(p).with_scale(Vec3::splat(LASER_DOT_M)),
            );
        }
        let want = show(hit.is_some());
        if *vis != want {
            *vis = want;
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

    /// A laser pointed down meets the ground inside its reach; one pointed
    /// at the sky runs out.
    #[test]
    fn a_laser_finds_the_ground_or_runs_out() {
        let seed = 7;
        let (x, z) = (1800.0, 1800.0);
        let at = Vec3::new(x, sim_core::terrain::height(seed, x, z) + 1.5, z);
        let down = laser_end(seed, at, Vec3::new(0.3, -1.0, 0.0).normalize());
        assert!(down.is_some_and(|p| p.distance(at) < 3.0));
        assert_eq!(laser_end(seed, at, Vec3::Y), None);
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
