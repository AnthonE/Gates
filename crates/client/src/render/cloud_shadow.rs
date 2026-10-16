//! The cloud deck's shadows on the land — the desktop's.
//!
//! **Relative to the eye, and why.** The rig dims the one sun by how much of
//! its disk the deck covers as seen from the eye (`WeatherNow::sun_cover` →
//! `rig::day_night`), so everything near the eye — a prop, a player, a tree
//! — stands in the light the eye stands in. Bevy works the sun out inside
//! `apply_pbr_lighting`, with no hook per light, so a per-pixel shadow can
//! only go where the shader is ours: the ground (`ground_splat.wgsl`, and
//! the rocks that wear its material). Shading the ground by its OWN cover
//! would put a prop and the ground under it in two lights. So the ground
//! takes the sun's direct term once more, scaled by how much more or less
//! of the sun reaches it than reaches the eye (`cloud_shadow.wgsl`): nothing
//! at the eye, and a hill a kilometre off goes dark and light as the deck's
//! shadows cross it. Where the rig puts the sun out — night, a sun under
//! the deck's horizon cutoff, a full overcast, the fog — both ends read the
//! same and the term is nothing.
//!
//! **One reading of the deck at the eye.** The far ground is lit at the
//! rig's sun times `here / there`, so the shader's `there` must be the very
//! cover the rig dimmed the sun by: both read [`sky::SoftField`] — one mip
//! of the field, the same bytes, the same bilinear — and the drift and the
//! weather's lanes settle on exactly the weather's values ([`Anchor`],
//! [`Lane`]). The shadow pattern rides the eye, as the deck does: walking
//! moves it with you, and only the wind moves it over the land.
//!
//! **Not in a browser, and not on the GL fallback's texture wall.** The
//! field is a storage buffer of bytes, not a texture: the ground's pipeline
//! under the atmosphere is exactly the sixteen sampled textures a downlevel
//! adapter (WebGPU's floor, a native GL fallback) allows
//! (`quality::FRAGMENT_TEXTURES`), and a buffer counts against none of
//! them. A bind group's layout is fixed when the client is built, and the
//! browser's ground stays the frame it was: this module and the material's
//! two bindings exist only natively, and draw at the HIGH clouds row
//! ([`super::quality::cloud_shadows`]).
//!
//! **Written rarely.** A write to the ground's material re-prepares it and
//! every chunk and rock that wears it, so nothing here moves per frame: the
//! sun is read per frame off the light itself, the drift runs on in the
//! shader from where it was last anchored at the rate it was going
//! ([`Anchor`]), and the weather's lanes are written in steps.

use bevy::asset::{uuid_handle, RenderAssetUsages};
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bevy::render::storage::ShaderStorageBuffer;

use super::ground_splat::GroundMaterial;
use super::sky::{self, SkyComposer};
use super::weather::WeatherNow;

/// How soft a shadow's edge is, metres ([`sky::SOFT_M`]).
pub use super::sky::SOFT_M;

/// How much of the direct sun a full deck takes: `rig::day_night`'s
/// `1 − 0.9 × occlusion`, the same number, so a shadow on a far hill is as
/// deep as the one the eye stands in.
pub const SUN_TAKEN: f32 = 0.9;

/// How far the shader's running drift may wander from the weather's before
/// it is anchored again, metres. The weather sums its drift in f64, so a
/// steady wind's run stays on it to the millimetre and this is a change of
/// wind, a hitch, or the field coming round (`tests/cloud_shadow.rs`).
pub const DRIFT_SLACK_M: f32 = 0.25;

/// How far the sun may move before the lanes say where it is again, as a
/// cosine (~5°, a few minutes of the day). The shader reads the sun off the
/// light every frame; this is only what tells the sun's light from the
/// lightning's, and it looks within 18° for it.
pub const SUN_SLACK_COS: f32 = 0.996;

/// The step a weather lane must move before it is written mid-change: a
/// weather change moves the floor and the field threshold a little every
/// frame, and a step is a few metres of a cloud's edge.
pub const LANE_STEP: f32 = 1.0 / 256.0;

/// How long a lane holds still before the value it settled on is written
/// whole, seconds: so a settled sky's shadows read the threshold and the
/// floor the rig reads, not the nearest step to them.
pub const LANE_SETTLE_S: f32 = 1.0;

/// The field the ground binds until the deck's is written (`init`): four
/// bytes the shader never reads, since its `cover.x` is 0 until then. A
/// storage buffer has no default the way an image has, and a handle with
/// no asset behind it would keep the ground from ever drawing.
pub const NO_FIELD: Handle<ShaderStorageBuffer> =
    uuid_handle!("3c5f0d2e-8b41-4a7e-9d16-5e2a0c7b9f41");

/// The field's mip the shadows read ([`sky::soft_level`]).
pub fn level() -> u32 {
    sky::soft_level()
}

/// The uniform, laid out to match `CloudShadow` in `cloud_shadow.wgsl`.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct CloudShadowParams {
    /// x 1 where the shadows are drawn, 0 off; y the weather's floor under
    /// the deck's cover (a heavy sky, fog: `WeatherNow::occlusion_floor`);
    /// z the field threshold (`1 − cover`); w the coverage edge
    /// (`sky::EDGE`).
    pub cover: Vec4,
    /// xy the field's offset at `sun.w`, noise units (`ComposeParams::drift`);
    /// zw how fast it moves, noise units a second.
    pub drift: Vec4,
    /// xyz toward the sun, which is how the shader knows the sun's light
    /// from the lightning's; w the `globals.time` the drift was taken at.
    pub sun: Vec4,
    /// x the deck over the eye, m (`sky::CLOUD_ALT_M`); y noise units a
    /// metre; z one over the field's period, noise units; w the field's
    /// texels a side ([`sky::SoftField::n`]).
    pub deck: Vec4,
    /// x the horizon cutoff (`sky::DECK_CUTOFF`); y the fade above it
    /// (`sky::HORIZON_FADE`); z [`SUN_TAKEN`]; w how far a texel's centre
    /// sits from its noise coordinate, texels ([`sky::SoftField::offset`]).
    pub fade: Vec4,
}

impl CloudShadowParams {
    /// No shadows: the deck's geometry, and nothing to draw with it.
    pub fn off() -> Self {
        let lod = level();
        Self {
            cover: Vec4::new(0.0, 0.0, 1.0 - sky::CLOUD_COVER, sky::EDGE),
            drift: Vec4::ZERO,
            sun: Vec4::ZERO,
            deck: Vec4::new(
                sky::CLOUD_ALT_M,
                1.0 / sky::CLOUD_SCALE_M,
                1.0 / sky::FIELD_PERIOD as f32,
                (sky::FIELD_N >> lod).max(1) as f32,
            ),
            fade: Vec4::new(
                sky::DECK_CUTOFF,
                sky::HORIZON_FADE,
                SUN_TAKEN,
                0.5 - 0.5 / (1u32 << lod) as f32,
            ),
        }
    }

    /// The lanes as written: the drift and the sun as `anchor` last took
    /// them, the weather's `floor` and field `threshold` as their [`Lane`]s
    /// last wrote them. [`Self::off`] unless `on`.
    pub fn new(on: bool, anchor: &Anchor, floor: f32, threshold: f32) -> Self {
        let mut p = Self::off();
        if on {
            p.cover.x = 1.0;
            p.cover.y = floor;
            p.cover.z = threshold;
            p.drift = Vec4::new(anchor.off.x, anchor.off.y, anchor.vel.x, anchor.vel.y);
            p.sun = anchor.sun.extend(anchor.t);
        }
        p
    }
}

/// Where the shader's drift runs from: the field's offset `off` (noise
/// units) at `t` (seconds of `globals.time`), moving at `vel`, and the sun
/// as it was then. Taken again only when the run has wandered from the
/// weather's own drift, the sun has moved, or the clock has wrapped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub off: Vec2,
    pub vel: Vec2,
    pub t: f32,
    pub sun: Vec3,
    set: bool,
}

impl Anchor {
    /// Follow this frame's offset `off` moving at `vel` and the sun at
    /// `sun`, at `t`. True when it anchored again — when the lanes change.
    pub fn track(&mut self, t: f32, off: Vec2, vel: Vec2, sun: Vec3) -> bool {
        let period = sky::FIELD_PERIOD as f32;
        let run = self.off + self.vel * (t - self.t);
        // The weather wraps its drift a whole field over, which the field
        // does too: a period apart is no distance at all.
        let gap = run - off;
        let gap = gap - (gap / period).round() * period;
        let stale = !self.set
            // `elapsed_secs_wrapped` has come round.
            || t < self.t
            || gap.length() * sky::CLOUD_SCALE_M > DRIFT_SLACK_M
            || self.sun.dot(sun) < SUN_SLACK_COS;
        if stale {
            *self = Self {
                off,
                vel,
                t,
                sun,
                set: true,
            };
        }
        stale
    }
}

/// One of the weather's lanes as the shader has it: written when the
/// weather has moved a [`LANE_STEP`] from it, and once more, exactly, when
/// the weather has held still [`LANE_SETTLE_S`] — so a change writes a few
/// times a second at most, and a settled sky reads what the rig reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lane {
    /// What the shader has.
    pub written: f32,
    seen: f32,
    since: f32,
    set: bool,
}

impl Lane {
    /// Follow the weather's `v` at `t`. True when it wrote.
    pub fn track(&mut self, t: f32, v: f32) -> bool {
        // `elapsed_secs_wrapped` coming round counts as a change.
        if !self.set || v != self.seen || t < self.since {
            self.seen = v;
            self.since = t;
        }
        let settled = t - self.since >= LANE_SETTLE_S;
        let stale =
            !self.set || (v - self.written).abs() >= LANE_STEP || (settled && v != self.written);
        if stale {
            self.written = v;
            self.set = true;
        }
        stale
    }
}

/// What [`drive`] keeps between frames.
#[derive(Default)]
pub struct Track {
    anchor: Anchor,
    floor: Lane,
    threshold: Lane,
    /// The deck's field as the ground binds it, and the world's field image
    /// it was made for: a new world is a new field.
    field: Option<(AssetId<Image>, Handle<ShaderStorageBuffer>)>,
}

/// Put [`NO_FIELD`] behind its handle, before any ground is made.
pub fn init(buffers: Option<ResMut<Assets<ShaderStorageBuffer>>>) {
    if let Some(mut buffers) = buffers {
        let _ = buffers.insert(
            NO_FIELD.id(),
            ShaderStorageBuffer::new(&[0; 4], RenderAssetUsages::RENDER_WORLD),
        );
    }
}

/// The field's bytes as the shader reads them: four to a `u32`, little end
/// first, padded to a whole word.
pub fn field_words(soft: &sky::SoftField) -> Vec<u8> {
    let mut bytes = soft.bytes().to_vec();
    bytes.resize(bytes.len().div_ceil(4).max(1) * 4, 0);
    bytes
}

/// Write the shadows' lanes and the deck's field into every ground
/// material, when they have moved. After `weather::update`.
pub fn drive(
    weather: Res<WeatherNow>,
    settings: Res<super::Settings>,
    time: Res<Time>,
    composer: Option<Res<SkyComposer>>,
    mut track: Local<Track>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut ground: ResMut<Assets<GroundMaterial>>,
) {
    let Some(composer) = composer else {
        return;
    };
    let track = &mut *track;
    // Off writes the one constant and tracks nothing, so a LOW or MEDIUM
    // sky never touches the ground after its first frame.
    let (want, field) = if super::quality::cloud_shadows(settings.gfx) {
        let t = time.elapsed_secs_wrapped();
        track.anchor.track(
            t,
            -weather.drift / sky::CLOUD_SCALE_M,
            -weather.drift_vel / sky::CLOUD_SCALE_M,
            weather.sun,
        );
        track.floor.track(t, weather.occlusion_floor());
        track
            .threshold
            .track(t, 1.0 - sky::cover_for(weather.cloud, weather.fel));
        let id = composer.field_image().id();
        if track.field.as_ref().is_none_or(|(made, _)| *made != id) {
            let words = field_words(composer.soft_field());
            let buffer = ShaderStorageBuffer::new(&words, RenderAssetUsages::RENDER_WORLD);
            track.field = Some((id, buffers.add(buffer)));
        }
        let want = CloudShadowParams::new(
            true,
            &track.anchor,
            track.floor.written,
            track.threshold.written,
        );
        (want, track.field.as_ref().map(|(_, h)| h.clone()))
    } else {
        (CloudShadowParams::off(), None)
    };
    let stale: Vec<AssetId<GroundMaterial>> = ground
        .iter()
        .filter(|(_, m)| {
            m.extension.cloud != want
                || field
                    .as_ref()
                    .is_some_and(|f| m.extension.cloud_field != *f)
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(m) = ground.get_mut(id) {
            m.extension.cloud = want;
            if let Some(f) = &field {
                m.extension.cloud_field = f.clone();
            }
        }
    }
}
