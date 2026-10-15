//! Frost on the frame while the cold bites — Rust's cold overlay, ice
//! creeping in from the edges of the screen once the body is cold enough.
//!
//! **It reads the cold the HUD already shows** (`hud::exposure`): the
//! server's `cold_pct` and its `hurting` bit, the body's own chill with the
//! clothes, the wet and the fire already in it — not the air, which a
//! jacket or a fire makes a different story. Nothing starts under
//! [`FROST_FROM_PCT`], a little past the COLD chip, so the chip warns first
//! and the frost is the cold arriving; it grows with the meter to where the
//! cold starts costing hp ([`FROST_HURT_PCT`]), and on past that only while
//! it does. A fire's warmth thaws it in a few seconds, ahead of the meter,
//! which falls only 2 % a second — unless the cold is still taking hp at
//! the fire, the one case the FREEZING chip also refuses to call won.
//!
//! **A UI material** (`frost.wgsl`), because UI materials draw on every
//! target this client ships — WebGL2 included — and the crystals are
//! procedural, so there is no texture to ship. Spawned when the frost
//! starts and despawned when it has thawed, so a warm body pays nothing.
//! Drawn on the eye's own target, so a lowered render scale lowers its cost
//! with the world's. Under the HUD with the hurt rim and the wounded
//! vignette (`ZIndex(-1)`), hidden off the world screen (the Esc menu, the
//! map, the death wash), and not spawned on a plate run or a film, which
//! have no HUD to be under.
//!
//! **What this file does not do is decide anything**: the cold is the
//! server's, the fire is `ClientCore::near_fire`, the light is the rig's.

use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use super::rig::{EyeCam, DAY_EV100};
use super::weather::WeatherNow;
use super::{Net, Screen, WorldEntity};

pub const SHADER: &str = "shaders/frost.wgsl";

/// No frost at or under this cold, per cent of a full chill. Past the COLD
/// chip's 25, so the chip says it first.
pub const FROST_FROM_PCT: u8 = 30;
/// Where the cold starts costing hp, per cent: `content/balance.toml`'s
/// `[exposure] hurt_at` over ten, the way the server rounds its meter
/// (`tests/frost.rs` holds the two together). A cold short of hurting
/// ramps from [`FROST_FROM_PCT`] to [`FROST_COLD_MAX`] by here, and only a
/// hurting one goes on past it, so the step into hp loss is not a jump.
pub const FROST_HURT_PCT: u8 = 60;
pub const FROST_COLD_MAX: f32 = 0.6;
/// How fast the frost creeps in and thaws, level a second: in over five
/// seconds, out over three.
pub const FROST_RISE_PER_S: f32 = 0.2;
pub const FROST_FALL_PER_S: f32 = 0.35;
/// The frost's brightness with no sun on the world, against noon's. The UI
/// draws after the tone map, so the scene's exposure does not dim it, and
/// the night's ground is near black: a pale rim any brighter reads as a
/// light, not as ice.
pub const FROST_NIGHT_LUM: f32 = 0.12;
/// The uniform's steps, for the level and the brightness alike: fine enough
/// that the front moves a pixel at a time, coarse enough that a creeping
/// frost or a setting sun re-uploads the material a few dozen times a
/// second rather than every frame.
const STEPS: f32 = 128.0;

/// How much frost a body's cold asks for, `0..=1`: none at
/// [`FROST_FROM_PCT`] and under, up with the cold to [`FROST_COLD_MAX`] at
/// [`FROST_HURT_PCT`], then on to 1 at a full meter — that last stretch
/// only while it is `hurting`, which never shows less than the ramp's top.
/// Beside a lit fire (`fire`) the frost that is not costing hp thaws.
pub fn target(cold_pct: u8, hurting: bool, fire: bool) -> f32 {
    if fire && !hurting {
        return 0.0;
    }
    let pct = f32::from(cold_pct.min(100));
    let from = f32::from(FROST_FROM_PCT);
    let at = f32::from(FROST_HURT_PCT);
    let curve = if pct <= at {
        ((pct - from) / (at - from)).max(0.0) * FROST_COLD_MAX
    } else {
        FROST_COLD_MAX + (1.0 - FROST_COLD_MAX) * (pct - at) / (100.0 - at)
    };
    if hurting {
        curve.max(FROST_COLD_MAX)
    } else {
        curve.min(FROST_COLD_MAX)
    }
}

/// One frame of the frost moving toward `want`: creeping in at
/// [`FROST_RISE_PER_S`], thawing at [`FROST_FALL_PER_S`], never past it.
pub fn step(level: f32, want: f32, dt: f32) -> f32 {
    if want > level {
        (level + FROST_RISE_PER_S * dt).min(want)
    } else {
        (level - FROST_FALL_PER_S * dt).max(want)
    }
}

/// The light on the world as the camera sees it, against a clear noon's,
/// `0..=1`: the sun's share of full daylight (`WeatherNow::sun_lux`, the
/// twilight tail included) through however far the rig has opened the
/// exposure (`ev100` under [`DAY_EV100`]), so a sunset the eye has opened
/// to does not dim the frost as if it were night.
pub fn seen_light(sun_lux: f32, ev100: f32) -> f32 {
    (sun_lux * (DAY_EV100 - ev100).max(0.0).exp2()).clamp(0.0, 1.0)
}

/// The frost's brightness for a [`seen_light`]: 1 at noon,
/// [`FROST_NIGHT_LUM`] with no sun at all, and between by the light's
/// square root — about how a lit white surface falls off through the tone
/// map and the screen.
pub fn lum(light: f32) -> f32 {
    FROST_NIGHT_LUM + (1.0 - FROST_NIGHT_LUM) * light.clamp(0.0, 1.0).sqrt()
}

/// The uniform for a level and a [`seen_light`], both rounded to [`STEPS`],
/// so a frost that is not moving under a sky that is barely moving leaves
/// its material alone.
pub fn params(level: f32, light: f32) -> Vec4 {
    let q = |v: f32| (v * STEPS).round() / STEPS;
    Vec4::new(q(level.clamp(0.0, 1.0)), q(lum(light)), 0.0, 0.0)
}

/// The frost's one uniform (`frost.wgsl`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct FrostMaterial {
    /// x how far the frost has crept in, `0..=1`; y its brightness; z and
    /// w unused — one whole `vec4`, WebGL2's 16-byte floor.
    #[uniform(0)]
    pub params: Vec4,
}

impl UiMaterial for FrostMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

/// The frost's node.
#[derive(Component)]
pub struct FrostLens;

/// What the frost spawns as: the whole frame, under the HUD and over the
/// world, the hurt rim's place, and never in the pointer's way.
pub fn bundle(material: Handle<FrostMaterial>) -> impl Bundle {
    (
        WorldEntity,
        FrostLens,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        MaterialNode(material),
        ZIndex(-1),
        Pickable::IGNORE,
    )
}

/// Follow the body's cold, and keep the node only while there is frost.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    mut commands: Commands,
    net: NonSend<Net>,
    state: Res<State<Screen>>,
    sky: Option<Res<WeatherNow>>,
    time: Res<Time>,
    eye: Query<(Entity, Option<&Exposure>), With<EyeCam>>,
    mut level: Local<f32>,
    mut materials: ResMut<Assets<FrostMaterial>>,
    mut lens: Query<(Entity, &MaterialNode<FrostMaterial>, &mut Visibility), With<FrostLens>>,
) {
    let core = &net.session.core;
    // One `hurting` bit answers for the heat too; the larger of the two
    // meters is the one hurting (`hud::exposure`).
    let hurting = core.cold_hurting && core.heat_pct <= core.cold_pct;
    // The lit-set walk only while there is frost a fire could thaw.
    let fire = !hurting && core.cold_pct > FROST_FROM_PCT && core.near_fire();
    let want = target(core.cold_pct, hurting, fire);
    let screen = state.get();
    let on_screen = *screen == Screen::InWorld;
    // Under the death wash and the load the body is changing hands, so the
    // frost is wherever the cold is when the world comes back: a respawn
    // does not wear the last life's frost. Behind the map or the Esc menu
    // it keeps moving, so it has crept or thawed as far as it would have.
    *level = if matches!(screen, Screen::Dead | Screen::Loading) {
        want
    } else {
        step(*level, want, time.delta_secs())
    };
    let eye = eye.single().ok();
    let ev100 = eye.and_then(|(_, ex)| ex).map_or(DAY_EV100, |ex| ex.ev100);
    let light = sky.map_or(1.0, |s| seen_light(s.sun_lux, ev100));
    let params = params(*level, light);
    let vis = if on_screen {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    match (params.x > 0.0, lens.single_mut().ok()) {
        (false, Some((e, ..))) => commands.entity(e).despawn(),
        (false, None) => {}
        (true, Some((_, node, mut shown))) => {
            shown.set_if_neq(vis);
            // Read first, so a settled frost does not mark its material
            // changed and re-upload it every frame.
            if materials.get(&node.0).is_some_and(|m| m.params != params) {
                if let Some(m) = materials.get_mut(&node.0) {
                    m.params = params;
                }
            }
        }
        (true, None) => {
            let mut node = commands.spawn((bundle(materials.add(FrostMaterial { params })), vis));
            // On the eye's own target rather than the HUD's camera: the
            // same window at full render scale, and below it the world's
            // smaller image (`render_scale.rs`), where the frost costs what
            // the world does and still lands under every HUD node.
            if let Some((eye, _)) = eye {
                node.insert(UiTargetCamera(eye));
            }
        }
    }
}
