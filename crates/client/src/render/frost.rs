//! Frost on the frame while the cold bites — Rust's cold overlay, ice
//! creeping in from the edges of the screen once the body is cold enough.
//!
//! **It reads the cold the HUD already shows** (`hud::exposure`): the
//! server's `cold_pct` and its `hurting` bit, the body's own chill with the
//! clothes, the wet and the fire already in it — not the air, which a
//! jacket or a fire makes a different story. Nothing starts under
//! [`FROST_FROM_PCT`], a little past the COLD chip, so the chip warns first
//! and the frost is the cold arriving; it is strongest while the cold costs
//! hp. A fire's warmth thaws it in a few seconds, ahead of the meter, which
//! falls only 2 % a second — unless the cold is still taking hp at the fire,
//! the one case the FREEZING chip also refuses to call won.
//!
//! **A UI material** (`frost.wgsl`), because UI materials draw on every
//! target this client ships — WebGL2 included — and the crystals are
//! procedural, so there is no texture to ship. Spawned when the frost
//! starts and despawned when it has thawed, so a warm body pays nothing.
//! Under the HUD with the hurt rim and the wounded vignette (`ZIndex(-1)`),
//! hidden off the world screen (the Esc menu, the map, the death wash), and
//! not spawned on a plate run or a film, which have no HUD to be under.
//!
//! **What this file does not do is decide anything**: the cold is the
//! server's, the fire is `ClientCore::near_fire`, the hour is the sky's.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use super::weather::WeatherNow;
use super::{Net, Screen, WorldEntity};

pub const SHADER: &str = "shaders/frost.wgsl";

/// No frost at or under this cold, per cent of a full chill. Past the COLD
/// chip's 25, so the chip says it first.
pub const FROST_FROM_PCT: u8 = 30;
/// Points past [`FROST_FROM_PCT`] to the cold's full frost short of hurting,
/// [`FROST_COLD_MAX`].
pub const FROST_RAMP_PCT: u8 = 40;
pub const FROST_COLD_MAX: f32 = 0.6;
/// While the cold is taking hp the frost is never under this, and the rest
/// of the way to 1 follows the meter — always past anything a cold that is
/// not yet hurting shows.
pub const FROST_HURT_MIN: f32 = 0.75;
/// How fast the frost creeps in and thaws, level a second: in over five
/// seconds, out over three.
pub const FROST_RISE_PER_S: f32 = 0.2;
pub const FROST_FALL_PER_S: f32 = 0.35;
/// The frost's brightness deep in the night against the day's: it is lit
/// by what lights the world, and a white rim glowing at midnight would read
/// as a light, not as ice.
pub const FROST_NIGHT_LUM: f32 = 0.45;
/// The uniform's level steps: fine enough that the front moves a pixel at
/// a time, coarse enough that a creeping frost re-uploads its material a
/// few dozen times a second rather than every frame.
const LEVEL_STEPS: f32 = 128.0;

/// How much frost a body's cold asks for, `0..=1`: none at
/// [`FROST_FROM_PCT`] and under, then up with the cold, and never less than
/// [`FROST_HURT_MIN`] while it is `hurting`. Beside a lit fire (`fire`) the
/// frost that is not costing hp thaws.
pub fn target(cold_pct: u8, hurting: bool, fire: bool) -> f32 {
    let past = cold_pct.saturating_sub(FROST_FROM_PCT) as f32;
    if hurting {
        let deep = (past / (100 - FROST_FROM_PCT) as f32).min(1.0);
        return FROST_HURT_MIN + (1.0 - FROST_HURT_MIN) * deep;
    }
    if fire {
        return 0.0;
    }
    (past / FROST_RAMP_PCT as f32).min(1.0) * FROST_COLD_MAX
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

/// The frost's brightness for how far into the night the sky is.
pub fn lum(night: f32) -> f32 {
    1.0 - (1.0 - FROST_NIGHT_LUM) * night.clamp(0.0, 1.0)
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
    let on_screen = *state.get() == Screen::InWorld;
    // Off the world screen nobody sees it move, so it is wherever the cold
    // is when they come back: a respawn does not wear the last life's frost.
    *level = if on_screen {
        step(*level, want, time.delta_secs())
    } else {
        want
    };
    let night = sky.map_or(0.0, |s| s.night);
    let params = Vec4::new(
        (*level * LEVEL_STEPS).round() / LEVEL_STEPS,
        lum(night),
        0.0,
        0.0,
    );
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
            commands.spawn((bundle(materials.add(FrostMaterial { params })), vis));
        }
    }
}
