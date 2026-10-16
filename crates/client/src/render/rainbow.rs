//! Rainbows: the sun's light turned back by the rain (the reference has had
//! them since 2020, `weather.rainbow`).
//!
//! **Where a bow is, is optics.** Light that enters a drop, reflects once
//! inside and leaves comes out bunched at its angle of least deviation —
//! Descartes' angle, ~42° from the point opposite the sun — so the bow is a
//! ring round the antisolar point, and how far round depends on how hard
//! water bends each colour: red outermost. Twice reflected it is ~51°,
//! fainter, the order reversed. Between the two no drop sends anything back
//! (Alexander's dark band), and inside the primary every drop sends a
//! little, so the sky there is brighter. [`descartes`] works the radii out
//! of water's refractive index; `rainbow.wgsl` draws the rings.
//!
//! **When is weather.** The sun up and lower than the bow is wide, or the
//! whole ring is under the horizon; rain in the air, a light or moderate
//! shower or the minute or two after one; and the sun on that rain, not
//! behind the deck, a heavy sky or fog. [`strength`] is exactly that, a pure
//! function. The rain still in the air is LOOKED BACK FOR along the
//! weather's own clock ([`lingering`]) rather than integrated the way
//! `WeatherNow::ground_wet` is, so a client that joined mid-shower reads the
//! rain a client that watched the shower come reads. The sun's cover is NOT
//! shared: `WeatherNow::sun_cover` is the deck over the sun at this client's
//! own drift (`weather::update`), the occlusion `rig::day_night` takes the
//! direct light away by. So the bow comes and goes with this client's
//! sunshine, and two players at one campfire can disagree about a bow just
//! as they already disagree about which cloud is over the sun.
//!
//! **A layer over the sky, under its fog.** One fullscreen triangle at the
//! far plane in the transparent pass, ordered by [`RAINBOW_ORDER`]: after
//! the deck, the stars and the moon, since the rain hangs in front of all
//! of them, and before the sky's fog (`deck::SkyFogMaterial`), which hides
//! a bow as it hides the land behind the shower. Under WebGL2 the deck folds
//! its fog in and there is no fog layer after this one, so the bow fogs
//! itself by the same band (`deck_fog`).

use bevy::asset::Asset;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use sim_core::limits::TICK_HZ;

use super::deck;
use super::feed::Feed;
use super::rig::DayPin;
use super::sky;
use super::weather::{WeatherNow, WeatherPin};
use super::{Screen, WorldId};

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/rainbow.wgsl";

/// Draw order in the transparent pass (`deck::DECK_ORDER` and its
/// neighbours): over the moon, under the sky's fog.
pub const RAINBOW_ORDER: f32 = -0.75e8;
const _: () = assert!(deck::MOON_ORDER < RAINBOW_ORDER && RAINBOW_ORDER < deck::SKY_FOG_ORDER);

/// Water's refractive index at the two ends of the spectrum, 700 nm and
/// 400 nm at 20 °C: red bends least, so its bow is the widest.
pub const WATER_N_RED: f32 = 1.331;
pub const WATER_N_VIOLET: f32 = 1.343;

/// The bow's colour, violet to red, one entry per ring the shader draws
/// across the spectrum: each channel's share of the sun's white. Worked out
/// once, offline: a 5800 K sun through the CIE 1931 observer (Wyman, Sloan
/// & Shirley's fit) into linear sRGB, each wavelength put at its own
/// Descartes angle (water's dispersion as a Cauchy fit through
/// [`WATER_N_RED`] and [`WATER_N_VIOLET`]), binned seven ways evenly in
/// angle, the out-of-gamut negatives clipped and every channel summed back
/// to one. So the rings add up to the sun's white where they overlap, and
/// the glow inside the bow is no colour at all.
pub const BANDS: [[f32; 3]; 7] = [
    [0.007, 0.000, 0.061],
    [0.030, 0.000, 0.335],
    [0.000, 0.000, 0.460],
    [0.000, 0.158, 0.144],
    [0.000, 0.550, 0.000],
    [0.615, 0.292, 0.000],
    [0.349, 0.000, 0.000],
];

/// How bright a ring is at full sun, deck units (1.0 is one
/// `sky::CLOUD_NITS`): about a clear zenith's brightness, so against the
/// grey of a passing shower the bow is the brightest thing on that side of
/// the sky and against a blue one it is a tint.
pub const GAIN: f32 = 0.3;
/// How far a ring spreads, degrees, toward its dark side and its bright
/// side. The dark side is a caustic's edge blurred by the sun's half-degree
/// disk; the bright side falls off as drops a millimetre across do.
const SPREAD_DARK_DEG: f32 = 0.3;
const SPREAD_BRIGHT_DEG: f32 = 0.4;
/// The secondary's rings over the primary's: spread wider, and a bit under
/// half as bright, the light lost at its second reflection.
const SECONDARY_SPREAD: f32 = 1.4;
const SECONDARY_GAIN: f32 = 0.45;
/// The first supernumerary, just inside the violet: two paths through one
/// drop interfering, a faint pastel ring where the drops are small.
const SUPER_GAIN: f32 = 0.25;
const SUPER_DEG: f32 = 1.4;
const SUPER_SPREAD_DEG: f32 = 0.3;
/// The brighter sky inside the primary, ring units, reached this many
/// degrees in from the violet.
const GLOW: f32 = 0.1;
const GLOW_DEG: f32 = 4.0;
/// How much Alexander's band darkens what is behind it, and how soft its
/// edges are, degrees.
const DARK_BAND: f32 = 0.08;
const DARK_EDGE_DEG: f32 = 1.0;

/// The sun's height, radians, over which a bow comes up from nothing: a sun
/// on the horizon lights the rain through a hundred kilometres of air.
const SUN_UP: f32 = 0.035;
/// How far the bow's crown has to clear the horizon, radians, to show whole.
const CROWN_FADE: f32 = 0.05;
/// The rain a bow needs, `WeatherNow::rain`: from a drizzle's first curtain
/// to a full one, and gone again as a downpour's haze washes it out.
const RAIN_FIRST: f32 = 0.02;
const RAIN_FULL: f32 = 0.10;
const RAIN_WASH: f32 = 0.5;
const RAIN_GONE: f32 = 0.9;
/// How much of the sun the sky may take (`WeatherNow::occlusion`) before the
/// bow starts to fade, and by when it has gone.
const OCC_CLEAR: f32 = 0.25;
const OCC_HIDDEN: f32 = 0.7;

/// How long rain hangs in the air after it stops, seconds: the last of a
/// shower still falling out of the cloud, still a curtain for the sun.
pub const LINGER_S: u32 = 90;
/// How many looks back across it.
const LINGER_LOOKS: u64 = 6;

/// The angle, radians, between the antisolar point and the light a drop of
/// index `n` sends back after `k` reflections inside it, at least deviation
/// (Descartes): ~42° once, ~51° twice.
pub fn descartes(n: f32, k: u32) -> f32 {
    let k = k as f32;
    let i = ((n * n - 1.0) / (k * k + 2.0 * k)).sqrt().acos();
    let r = (i.sin() / n).asin();
    let deviation = k * std::f32::consts::PI + 2.0 * i - 2.0 * (k + 1.0) * r;
    (-deviation.cos()).acos()
}

fn ramp(x: f32, from: f32, to: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How strongly the bow shows, `0..=1`, which is how bright it is: for a
/// sun `sun_y` up (the sine of its elevation), `light` its share of full
/// daylight (`WeatherNow::sun_lux`), `rain` still in the air and `occ` of
/// the sun taken by the sky (`WeatherNow::occlusion`).
pub fn strength(sun_y: f32, light: f32, rain: f32, occ: f32) -> f32 {
    let elev = sun_y.clamp(-1.0, 1.0).asin();
    // Sunlight on the rain: none from a sun under the horizon.
    let up = (elev / SUN_UP).clamp(0.0, 1.0);
    // The ring's top over the horizon: a sun higher than the red ring is
    // wide puts all of it underground.
    let crown = ((descartes(WATER_N_RED, 1) - elev) / CROWN_FADE).clamp(0.0, 1.0);
    let wet = ramp(rain, RAIN_FIRST, RAIN_FULL) * (1.0 - ramp(rain, RAIN_WASH, RAIN_GONE));
    let sun = 1.0 - ramp(occ, OCC_CLEAR, OCC_HIDDEN);
    up * crown * wet * sun * light.clamp(0.0, 1.0)
}

/// [`strength`] for this frame's sky, with `rain` still in the air: none
/// under the sea, where the eye's sky is the surface.
pub fn strength_now(w: &WeatherNow, rain: f32) -> f32 {
    if w.underwater {
        return 0.0;
    }
    strength(w.sun.y, w.sun_lux, rain, w.occlusion())
}

/// The rain still in the air at `tick`, given the rain at any tick: the
/// most of the last [`LINGER_S`], each look back weighed down linearly
/// with its age. The looks fall on a fixed grid of ticks and each stands
/// for the step after it, so the ages slide and the bow fades rather than
/// steps; and the whole is a function of the clock and the weather, as the
/// sky is.
pub fn lingering(tick: u64, rain_at: impl Fn(u64) -> f32) -> f32 {
    let span = LINGER_S as u64 * TICK_HZ as u64;
    let step = span / LINGER_LOOKS;
    let last = tick - tick % step;
    let mut most = rain_at(tick);
    for k in 0..LINGER_LOOKS {
        let Some(at) = last.checked_sub(k * step) else {
            break;
        };
        let age = (tick - at).saturating_sub(step) as f32 / (span - step) as f32;
        most = most.max(rain_at(at) * (1.0 - age).max(0.0));
    }
    most
}

/// The warmth a low sun puts on what it lights: `sky::DeckLight`'s, so the
/// bow and the cloud it hangs under are lit by one sunset.
fn sunlight(sun_y: f32) -> Vec3 {
    let dusk = ((0.25 - sun_y) / 0.25).clamp(0.0, 1.0);
    Vec3::new(1.0, 1.0 - 0.35 * dusk, 1.0 - 0.6 * dusk)
}

/// The uniform, laid out to match `Rainbow` in the shader.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct RainbowParams {
    pub anti: Vec4,
    pub light: Vec4,
    pub radii: Vec4,
    pub spread: Vec4,
    pub extra: Vec4,
    pub span: Vec4,
    pub air: Vec4,
    pub bands: [Vec4; 7],
}

impl RainbowParams {
    /// The bow opposite `sun` at strength `k`, through the sky's fog `air`
    /// where the bow has to fog itself.
    pub fn new(sun: Vec3, k: f32, air: Vec4) -> Self {
        let deg = 1.0f32.to_radians();
        let radii = Vec4::new(
            descartes(WATER_N_VIOLET, 1),
            descartes(WATER_N_RED, 1),
            descartes(WATER_N_VIOLET, 2),
            descartes(WATER_N_RED, 2),
        );
        // Where any ring reaches: from inside the primary's supernumerary to
        // past the secondary's bright side. Elsewhere the shader skips them.
        let reach = 3.0 * SPREAD_BRIGHT_DEG * SECONDARY_SPREAD * deg;
        let lo = radii.x - (SUPER_DEG + 3.0 * SUPER_SPREAD_DEG) * deg;
        let hi = radii.z + reach;
        Self {
            anti: (-sun.normalize_or(Vec3::Y)).extend(k),
            light: (sunlight(sun.y) * GAIN * sky::CLOUD_NITS).extend(DARK_BAND),
            radii,
            spread: Vec4::new(
                SPREAD_DARK_DEG * deg,
                SPREAD_BRIGHT_DEG * deg,
                SECONDARY_SPREAD,
                SECONDARY_GAIN,
            ),
            extra: Vec4::new(SUPER_GAIN, SUPER_DEG * deg, SUPER_SPREAD_DEG * deg, GLOW),
            span: Vec4::new(lo, hi, GLOW_DEG * deg, DARK_EDGE_DEG * deg),
            air,
            bands: BANDS.map(|b| Vec3::from_array(b).extend(0.0)),
        }
    }

    /// This frame's bow, with `rain` still in the air.
    pub fn for_weather(w: &WeatherNow, rain: f32) -> Self {
        let k = strength_now(w, rain);
        // Where the sky's fog draws after this layer it fogs the bow too;
        // under WebGL2 the deck folded its fog in and the bow has to.
        let air = if sky::BAKE_BACKDROP {
            sky::SkyFog::new(w.fog_part(), w.rain_part()).lanes()
        } else {
            Vec4::ZERO
        };
        Self::new(w.sun, k, air)
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct RainbowMaterial {
    #[uniform(0)]
    pub params: RainbowParams,
}

impl Material for RainbowMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// Premultiplied: the rings add light, and Alexander's band takes a
    /// little away.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        RAINBOW_ORDER
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The bow's entity.
#[derive(Component)]
pub struct Rainbow;

/// The handle `drive` writes.
#[derive(Resource)]
pub struct RainbowHandle(pub Handle<RainbowMaterial>);

/// Build the bow. After `sky::setup`: no sky, no bow.
pub fn setup(
    mut commands: Commands,
    composer: Option<Res<sky::SkyComposer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RainbowMaterial>>,
) {
    if composer.is_none() {
        return;
    }
    let material = materials.add(RainbowMaterial {
        params: RainbowParams::new(Vec3::Y, 0.0, Vec4::ZERO),
    });
    commands.spawn((
        super::WorldEntity,
        Rainbow,
        Mesh3d(meshes.add(deck::fullscreen_mesh())),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(RainbowHandle(material));
}

/// Put the bow where the sun and the rain say. Hidden whenever there is
/// none, which is nearly always, so a dry sky pays no fullscreen pass —
/// except while the world loads, when it is drawn at nothing so its
/// pipeline is built behind the loading screen and not at the first bow.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    weather: Res<WeatherNow>,
    feed: Res<Feed>,
    pin: Res<DayPin>,
    wpin: Res<WeatherPin>,
    world: Res<WorldId>,
    screen: Res<State<Screen>>,
    handle: Option<Res<RainbowHandle>>,
    mut materials: ResMut<Assets<RainbowMaterial>>,
    mut bow: Query<&mut Visibility, With<Rainbow>>,
) {
    let Some(handle) = handle else {
        return;
    };
    // A probe's sky holds still, and so does what is in its air.
    let rain = if wpin.0.is_some() {
        weather.rain
    } else {
        let tick = pin.tick(feed.server_tick_est);
        lingering(tick, |t| {
            sim_core::weather::now(world.seed, t, &feed.env).rain as f32 * 0.001
        })
    };
    let want = RainbowParams::for_weather(&weather, rain);
    let show = want.anti.w > 0.0 || *screen.get() == Screen::Loading;
    for mut v in bow.iter_mut() {
        let vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != vis {
            *v = vis;
        }
    }
    if !show {
        return;
    }
    let stale = materials.get(&handle.0).is_some_and(|m| m.params != want);
    if stale {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
