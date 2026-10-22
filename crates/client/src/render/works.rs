//! The works' terminals, drawn (`ARC.md` F1). Each work wears the look of
//! the site it stands at ([`LOOKS`]): a generated ruin, turned with its site,
//! and the code parts that wake with it — a gold ring, a pool that fills as
//! the quota does, a glass core that glows with its tank, and fallen stones
//! that rise and hover over it. A site with no look of its own wears
//! [`PLINTH`]: a black plinth with a fire bowl and the ring standing behind
//! (`WORLD.md` §11.3's shape, planted, so it stays an exact torus here).
//! Where it stands is `sim_core::spot`'s answer on the seed, so it is drawn
//! where the sim measures reach from.
//!
//! The work's state lights it:
//! - **sealed** — dark, its stones fallen;
//! - **open** — a faint lapis glow; the pool and the stones rise with the
//!   quota, so a player sees what their deposit did;
//! - **burning** — fire, the pool and core bright with the tank, the stones
//!   circling overhead;
//! - **embers** — a low red light, the stones still.

use bevy::prelude::*;
use client_core::arc::WorkView;
use sim_core::landmark::LandmarkKind;
use sim_core::spot::{Spot, SITE_LANDMARK0};
use sim_core::terrain::Haven;
use sim_core::works::{WORK_LIT, WORK_OPEN};

use super::fx::world::{FireFx, Tongues};
use super::structures::FIRE_COLOR;
use super::{Net, WorldEntity, WorldId};

/// One work's terminal; the index is the work's.
#[derive(Component)]
pub struct WorkMark(pub u8);
/// The bowl's fire (light + flames), left at zero until it burns.
#[derive(Component)]
pub struct WorkFire(pub u8);
/// The glow an open work wears, and an ember's.
#[derive(Component)]
pub struct WorkGlow(pub u8);
/// The pool that fills with the quota: its own material, how high it stands
/// now and empty and full, and the state it last showed.
#[derive(Component)]
pub struct WorkPool {
    k: u8,
    kind: PoolKind,
    y: [f32; 2],
    shown: u32,
}
/// The glass core that glows with the tank, and the state it last showed.
#[derive(Component)]
pub struct WorkCore {
    k: u8,
    shown: u32,
}
/// One fallen stone: where it lies and where it hovers once risen (the
/// model's frame), how far risen it is, and how far round it has turned.
#[derive(Component)]
pub struct WorkStone {
    k: u8,
    i: u8,
    fallen: Transform,
    hover: Vec3,
    lift: f32,
    orbit: f32,
}

const FIRE_LUMENS: f32 = 90_000.0;
const FIRE_RANGE_M: f32 = 14.0;
const GLOW_OPEN: Color = Color::srgb(0.35, 0.6, 1.0);
const GLOW_EMBER: Color = Color::srgb(0.9, 0.25, 0.1);
const GLOW_LUMENS: f32 = 2_500.0;

/// The plinth and its fire bowl, generated: 1.6 m square, the bowl's rim
/// 1.58 m over the model's base.
pub const PLINTH_GLB: &str = "models/site/works_plinth.glb";
/// How far a terminal stands in the ground below its highest corner:
/// `ART.md` rule 2, nothing sits ON it. A slope deeper than this gets a
/// footing under the low side.
const SINK_M: f32 = 0.1;
/// How far a footing reaches below the lowest corner.
const FOOTING_M: f32 = 0.3;

/// What a pool is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolKind {
    /// Metal in a crucible: a dull crust while it fills, bright while the
    /// tank burns.
    Molten,
}

/// A pool: a disc `r` across its middle at `at` (x, z) that rises from
/// `y[0]` empty to `y[1]` full.
#[derive(Clone, Copy, Debug)]
pub struct Pool {
    pub at: [f32; 2],
    pub r: f32,
    pub y: [f32; 2],
    pub kind: PoolKind,
}

/// How one work's terminal is drawn, in the model's frame: +Z its front, y
/// up from the ground under its highest corner.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub glb: &'static str,
    /// Half its footprint (x, z), metres: where the ground is sampled.
    pub half: [f32; 2],
    /// A turn on top of its site's, radians.
    pub turn: f32,
    /// The gold ring's centre, standing on edge across +Z.
    pub ring: Option<[f32; 3]>,
    /// Where the fire burns, and where the open glow shines from.
    pub fire: [f32; 3],
    pub glow: [f32; 3],
    pub pool: Option<Pool>,
    /// A glass core's centre and radius.
    pub core: Option<([f32; 3], f32)>,
    /// How many fallen stones lie around it, and the ring they hover in once
    /// risen: its height and radius.
    pub stones: u8,
    pub hover_y: f32,
    pub hover_r: f32,
}

/// Today's terminal, for a work at a site with no look of its own: the ring
/// 1.3 m over the plinth's top (1.05 m) and behind the bowl, the fire just
/// over the bowl's rim (1.48 m).
pub static PLINTH: Look = Look {
    glb: PLINTH_GLB,
    half: [0.8, 0.8],
    turn: 0.0,
    ring: Some([0.0, 2.35, -0.95]),
    fire: [0.0, 1.73, 0.0],
    glow: [0.0, 2.35, -0.6],
    pool: None,
    core: None,
    stones: 0,
    hover_y: 0.0,
    hover_r: 0.0,
};

/// THE CRUCIBLE (`content/arc.toml`), at the anvil rock: a sandstone furnace
/// with a brass crucible in a geared cradle and a glass dome on its +X side,
/// generated, 3.19 × 2.28 × 3.20 m with its spout and mould on +Z, away from
/// the rock 2.5 m behind it. The crucible's rim is 2.28 m up, its inside
/// 0.44 m across the middle and floored at 1.53 m; the dome 0.42 m round at
/// (1.15, 0.80, −0.30), measured off the model.
const CRUCIBLE: Look = Look {
    glb: "models/site/works_crucible.glb",
    half: [1.6, 1.6],
    turn: 0.0,
    ring: Some([-0.13, 2.85, -1.2]),
    fire: [-0.13, 2.53, -0.31],
    glow: [-0.13, 2.6, -0.31],
    pool: Some(Pool {
        at: [-0.13, -0.31],
        r: 0.42,
        y: [1.65, 2.16],
        kind: PoolKind::Molten,
    }),
    core: Some(([1.15, 0.80, -0.30], 0.43)),
    stones: 5,
    hover_y: 3.3,
    hover_r: 2.0,
};

/// Each site's own look, by `sim_core::spot` site code (keyed on the site,
/// not the row, so a reordered or renamed row keeps its ruin).
pub static LOOKS: [(u8, Look); 1] = [(SITE_LANDMARK0 + LandmarkKind::Anvil as u8, CRUCIBLE)];

/// The look a work at `site` wears.
pub fn look(site: u8) -> &'static Look {
    LOOKS
        .iter()
        .find(|(s, _)| *s == site)
        .map_or(&PLINTH, |(_, l)| l)
}

/// Every model a terminal can draw.
pub fn models() -> impl Iterator<Item = &'static str> {
    [PLINTH_GLB, SHARD_GLB]
        .into_iter()
        .chain(LOOKS.iter().map(|(_, l)| l.glb))
}

/// The site's own heading: where its +Z points in the world, radians about
/// +Y (a landmark's yaw, the town's quarter turn).
fn site_yaw(h: &Haven, s: &Spot) -> f32 {
    let ahead = Spot {
        z_cm: s.z_cm.saturating_add(100),
        ..*s
    };
    match (
        sim_core::spot::world(h, s),
        sim_core::spot::world(h, &ahead),
    ) {
        (Some((x0, _, z0)), Some((x1, _, z1))) => (x1 - x0).atan2(z1 - z0),
        _ => 0.0,
    }
}

/// Stand a terminal up for every work whose row has arrived and that stands
/// somewhere on this island.
pub fn spawn(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    have: Query<&WorkMark>,
) {
    let Some(world) = world else { return };
    let core = &net.session.core;
    for (k, w) in core.arc.known() {
        if have.iter().any(|m| m.0 as usize == k) {
            continue;
        }
        let Some((x, _, z)) = sim_core::spot::world(&world.haven, &w.def.spot) else {
            continue;
        };
        let look = look(w.def.spot.site);
        let turn = Quat::from_rotation_y(site_yaw(&world.haven, &w.def.spot) + look.turn);
        let ground = |at: Vec3| {
            let p = turn * at;
            sim_core::terrain::ground(world.seed, &world.haven, x + p.x, z + p.z)
        };
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for (sx, sz) in [
            (0.0, 0.0),
            (-1.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
        ] {
            let g = ground(Vec3::new(sx * look.half[0], 0.0, sz * look.half[1]));
            lo = lo.min(g);
            hi = hi.max(g);
        }
        let (model, model_material) = super::mobs::model(&server, look.glb);
        let gold = materials.add(super::lore::worn_gold());
        commands
            .spawn((
                WorldEntity,
                WorkMark(k as u8),
                Transform::from_xyz(x, hi, z).with_rotation(turn),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((
                    Mesh3d(model),
                    MeshMaterial3d(model_material),
                    Transform::from_xyz(0.0, -SINK_M, 0.0),
                ));
                // On a slope the low side would show daylight under it.
                if hi - lo > SINK_M {
                    let top = -SINK_M + 0.05;
                    let bottom = lo - hi - FOOTING_M;
                    c.spawn((
                        Mesh3d(meshes.add(Cuboid::new(
                            look.half[0] * 1.9,
                            top - bottom,
                            look.half[1] * 1.9,
                        ))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgb(0.46, 0.41, 0.35),
                            perceptual_roughness: 0.95,
                            ..default()
                        })),
                        Transform::from_xyz(0.0, (top + bottom) * 0.5, 0.0),
                    ));
                }
                if let Some(at) = look.ring {
                    c.spawn((
                        Mesh3d(meshes.add(Torus::new(1.05, 1.25))),
                        MeshMaterial3d(gold),
                        Transform::from_translation(Vec3::from_array(at))
                            .with_rotation(Quat::from_rotation_x(core::f32::consts::FRAC_PI_2)),
                    ));
                }
                c.spawn((
                    WorkFire(k as u8),
                    PointLight {
                        color: FIRE_COLOR,
                        intensity: 0.0,
                        range: FIRE_RANGE_M,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(Vec3::from_array(look.fire)),
                    FireFx {
                        flames: true,
                        flame_dy: -0.35,
                        smoke_dy: 0.5,
                        scale: 1.0,
                        tongues: Tongues::Pit,
                    },
                ));
                c.spawn((
                    WorkGlow(k as u8),
                    PointLight {
                        color: GLOW_OPEN,
                        intensity: 0.0,
                        range: 6.0,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(Vec3::from_array(look.glow)),
                ));
                if let Some(p) = look.pool {
                    c.spawn((
                        WorkPool {
                            k: k as u8,
                            kind: p.kind,
                            y: p.y,
                            shown: u32::MAX,
                        },
                        Mesh3d(meshes.add(Cylinder::new(p.r, 0.04))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgb(0.08, 0.05, 0.04),
                            perceptual_roughness: 0.55,
                            ..default()
                        })),
                        Transform::from_xyz(p.at[0], p.y[0], p.at[1]),
                        Visibility::Hidden,
                    ));
                }
                if let Some((at, r)) = look.core {
                    c.spawn((
                        WorkCore {
                            k: k as u8,
                            shown: u32::MAX,
                        },
                        Mesh3d(meshes.add(Sphere::new(r))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::srgb(0.55, 0.42, 0.25),
                            perceptual_roughness: 0.2,
                            ..default()
                        })),
                        Transform::from_translation(Vec3::from_array(at)),
                    ));
                }
                for (i, stone) in stones(look).enumerate() {
                    let at = stone.fallen.translation;
                    let mut fallen = stone.fallen;
                    fallen.translation.y = ground(at) - hi - SINK_M;
                    let (mesh, material) = super::mobs::model(&server, SHARD_GLB);
                    c.spawn((
                        WorkStone {
                            k: k as u8,
                            i: i as u8,
                            fallen,
                            hover: stone.hover,
                            lift: 0.0,
                            orbit: 0.0,
                        },
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        fallen,
                    ));
                }
            });
    }
}

/// The stone fragment a look's fallen stones are drawn with: a broken
/// sandstone block with a bronze inlay, generated, 0.40 × 0.41 × 0.45 m.
pub const SHARD_GLB: &str = "models/site/works_shard.glb";

/// Where a look's stones lie fallen (across its front, ±120°, a pace out
/// from its footprint) and where each hovers once risen (a ring over it).
struct StoneAt {
    fallen: Transform,
    hover: Vec3,
}

fn stones(look: &Look) -> impl Iterator<Item = StoneAt> + '_ {
    let n = look.stones as usize;
    (0..n).map(move |i| {
        let f = if n > 1 {
            i as f32 / (n - 1) as f32
        } else {
            0.5
        };
        let a = -2.1 + 4.2 * f;
        let fallen = Transform::from_xyz(
            a.sin() * (look.half[0] + 0.6),
            0.0,
            a.cos() * (look.half[1] + 0.6),
        )
        .with_rotation(
            Quat::from_rotation_y(i as f32 * 1.3) * Quat::from_rotation_x(0.35 * (i % 2) as f32),
        )
        .with_scale(Vec3::splat(1.0 + 0.6 * (i as f32 * 0.618).fract()));
        let h = i as f32 * core::f32::consts::TAU / n as f32 + 0.3;
        let hover = Vec3::new(
            h.sin() * look.hover_r,
            look.hover_y + 0.35 * (i as f32 * 1.7).sin(),
            h.cos() * look.hover_r,
        );
        StoneAt { fallen, hover }
    })
}

/// How far along a work is, as its terminal shows it: the quota met (0
/// sealed, 1 once lit) and the tank (0 unless it burns, and never below a
/// third while it does, so a nearly dry tank still reads as burning).
fn wake(w: &WorkView) -> (f32, f32) {
    match w.state {
        WORK_OPEN => (w.progress_pm() as f32 / 1000.0, 0.0),
        WORK_LIT if w.fuel > 0 => {
            let tank = w.fuel as f32 / w.def.fuel_max.max(1) as f32;
            (1.0, 0.35 + 0.65 * tank.clamp(0.0, 1.0))
        }
        WORK_LIT => (1.0, 0.0),
        _ => (0.0, 0.0),
    }
}

/// What a pool or core last drew, so a material is written only when what
/// it shows moves.
fn shown(w: &WorkView, fill: f32, heat: f32) -> u32 {
    ((w.state as u32) << 16) | (((fill * 64.0) as u32) << 8) | (heat * 64.0) as u32
}

/// Light each terminal by its work's state.
#[allow(clippy::type_complexity)]
pub fn light(
    net: NonSend<Net>,
    gain: Res<super::rig::FlameGain>,
    mut fires: Query<(&WorkFire, &mut PointLight), Without<WorkGlow>>,
    mut glows: Query<(&WorkGlow, &mut PointLight), Without<WorkFire>>,
) {
    let arc = &net.session.core.arc;
    for (f, mut light) in fires.iter_mut() {
        let burning = arc
            .works
            .get(f.0 as usize)
            .is_some_and(|w| w.state == WORK_LIT && w.fuel > 0);
        let want = if burning { FIRE_LUMENS * gain.0 } else { 0.0 };
        if light.intensity != want {
            light.intensity = want;
        }
    }
    for (g, mut light) in glows.iter_mut() {
        let Some(w) = arc.works.get(g.0 as usize) else {
            continue;
        };
        let (color, lumens) = match w.state {
            WORK_OPEN => (GLOW_OPEN, GLOW_LUMENS),
            WORK_LIT if w.fuel == 0 => (GLOW_EMBER, GLOW_LUMENS),
            _ => (GLOW_OPEN, 0.0),
        };
        if light.intensity != lumens {
            light.intensity = lumens;
        }
        if light.color != color {
            light.color = color;
        }
    }
}

/// Wake each terminal's parts with its work: the pool rises and glows, the
/// core warms, the stones lift and, while the tank burns, circle.
#[allow(clippy::type_complexity)]
pub fn animate(
    net: NonSend<Net>,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut pools: Query<
        (
            &mut WorkPool,
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        Without<WorkStone>,
    >,
    mut cores: Query<(&mut WorkCore, &MeshMaterial3d<StandardMaterial>)>,
    mut stones: Query<(&mut WorkStone, &mut Transform), Without<WorkPool>>,
) {
    let arc = &net.session.core.arc;
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let ease = |rate: f32| 1.0 - (-dt * rate).exp();
    for (mut p, mut at, mut vis, mat) in pools.iter_mut() {
        let Some(w) = arc.works.get(p.k as usize) else {
            continue;
        };
        let (fill, heat) = wake(w);
        let want = if w.state == WORK_OPEN || w.state == WORK_LIT {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        let y = p.y[0] + (p.y[1] - p.y[0]) * fill;
        at.translation.y += (y - at.translation.y) * ease(1.5);
        let key = shown(w, fill, heat);
        if p.shown == key {
            continue;
        }
        p.shown = key;
        if let Some(m) = materials.get_mut(&mat.0) {
            m.emissive = match (p.kind, w.state) {
                (PoolKind::Molten, WORK_LIT) if heat > 0.0 => {
                    LinearRgba::rgb(4.0, 1.3, 0.25) * (1.0 + 4.0 * heat)
                }
                (PoolKind::Molten, WORK_LIT) => LinearRgba::rgb(0.8, 0.1, 0.0),
                (PoolKind::Molten, _) => LinearRgba::rgb(0.6, 0.08, 0.0) * (0.3 + 1.2 * fill),
            };
        }
    }
    for (mut c, mat) in cores.iter_mut() {
        let Some(w) = arc.works.get(c.k as usize) else {
            continue;
        };
        let (fill, heat) = wake(w);
        let key = shown(w, fill, heat);
        if c.shown == key {
            continue;
        }
        c.shown = key;
        if let Some(m) = materials.get_mut(&mat.0) {
            m.emissive = match w.state {
                WORK_LIT if heat > 0.0 => LinearRgba::rgb(3.0, 1.8, 0.8) * (1.0 + 3.0 * heat),
                WORK_LIT => LinearRgba::rgb(0.6, 0.12, 0.02),
                WORK_OPEN => LinearRgba::rgb(1.2, 0.5, 0.1) * fill,
                _ => LinearRgba::BLACK,
            };
        }
    }
    for (mut s, mut at) in stones.iter_mut() {
        let Some(w) = arc.works.get(s.k as usize) else {
            continue;
        };
        let (fill, heat) = wake(w);
        // Stone i rises as the quota passes its share; all are up once lit.
        let n = stone_count(w) as f32;
        let target = (fill * n - s.i as f32).clamp(0.0, 1.0);
        s.lift += (target - s.lift) * ease(0.8);
        if heat > 0.0 {
            s.orbit += dt * (0.08 + 0.12 * heat);
        }
        let up = smooth(s.lift);
        // Straight up first, then in over the ruin, so none passes through it.
        let inward = smooth((s.lift * 2.0 - 1.0).clamp(0.0, 1.0));
        let hover = Quat::from_rotation_y(s.orbit) * s.hover;
        let bob = 0.08 * (t * 0.9 + s.i as f32).sin() * up;
        let from = s.fallen.translation;
        at.translation = Vec3::new(
            from.x + (hover.x - from.x) * inward,
            from.y + (hover.y - from.y) * up + bob,
            from.z + (hover.z - from.z) * inward,
        );
        let tumble = Quat::from_rotation_y(t * 0.2 + s.i as f32) * Quat::from_rotation_x(0.2);
        at.rotation = s.fallen.rotation.slerp(tumble, up);
    }
}

/// How many stones a work's look lays out.
fn stone_count(w: &WorkView) -> u8 {
    look(w.def.spot.site).stones.max(1)
}

fn smooth(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}
