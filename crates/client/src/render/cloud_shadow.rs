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
//! **Not in a browser.** The ground's pipeline under WebGPU's atmosphere is
//! exactly the sixteen sampled textures the floor guarantees
//! (`quality::FRAGMENT_TEXTURES`), and the field would be a seventeenth: a
//! refused pipeline and a lost frame. A bind group's layout is fixed when
//! the client is built, so the field binds where the room always is —
//! natively — and the wasm builds, WebGPU and WebGL2 alike, are the frame
//! they were. This module and the material's two bindings exist only there.
//!
//! **Written rarely.** A write to the ground's material re-prepares it and
//! every chunk and rock that wears it, so nothing here moves per frame: the
//! sun is read per frame off the light itself, the drift runs on in the
//! shader from where it was last anchored at the rate it was going
//! ([`Anchor`]), and the weather's lanes are written in steps.

use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

use super::ground_splat::GroundMaterial;
use super::sky::{self, SkyComposer};
use super::weather::WeatherNow;
use crate::config::Clouds;

/// How soft a shadow's edge is, metres: the field is read at the mip whose
/// texel is nearest this. The sun's own disk blurs a shadow cast from a deck
/// 1.4 km up by 13–40 m, and a cumulus edge is a few tens of metres more.
pub const SOFT_M: f32 = 60.0;

/// How much of the direct sun a full deck takes: `rig::day_night`'s
/// `1 − 0.9 × occlusion`, the same number, so a shadow on a far hill is as
/// deep as the one the eye stands in.
pub const SUN_TAKEN: f32 = 0.9;

/// How far the shader's running drift may wander from the weather's before
/// it is anchored again, metres: a shadow a metre off its cloud a kilometre
/// up is nothing anyone sees. The weather's own drift, integrated a frame
/// at a time in f32 metres, wanders from a straight run by that much in
/// half a minute or so where it is large: a write every forty seconds on a
/// steady wind (`tests/cloud_shadow.rs`).
pub const DRIFT_SLACK_M: f32 = 1.0;

/// How far the sun may move before the lanes say where it is again, as a
/// cosine (~5°, a few minutes of the day). The shader reads the sun off the
/// light every frame; this is only what tells the sun's light from the
/// lightning's, and it looks within 18° for it.
pub const SUN_SLACK_COS: f32 = 0.996;

/// The steps the weather's floor and the field threshold are written in:
/// a weather change moves them a little every frame, and a step is a few
/// metres of a cloud's edge.
pub const LANE_STEP: f32 = 1.0 / 256.0;

/// The field's mip the shadows read ([`SOFT_M`]): 1 on a desktop release,
/// whose field texel is 28 m; 0 in a debug build, whose is 56 m.
pub fn level() -> u32 {
    let texel = sky::CLOUD_SCALE_M * sky::FIELD_PERIOD as f32 / sky::FIELD_N as f32;
    (SOFT_M / texel).log2().round().max(0.0) as u32
}

/// The uniform, laid out to match `CloudShadow` in `cloud_shadow.wgsl`.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct CloudShadowParams {
    /// x 1 where the shadows are drawn, 0 off; y the weather's floor under
    /// the deck's cover (a heavy sky, fog: `WeatherNow::occlusion` with no
    /// deck over the sun); z the field threshold (`1 − cover`); w the
    /// coverage edge (`sky::EDGE`).
    pub cover: Vec4,
    /// xy the field's offset at `sun.w`, noise units (`ComposeParams::drift`);
    /// zw how fast it moves, noise units a second.
    pub drift: Vec4,
    /// xyz toward the sun, which is how the shader knows the sun's light
    /// from the lightning's; w the `globals.time` the drift was taken at.
    pub sun: Vec4,
    /// x the deck over the eye, m (`sky::CLOUD_ALT_M`); y noise units a
    /// metre; z one over the field's period, noise units; w the mip read.
    pub deck: Vec4,
    /// x the horizon cutoff (`sky::DECK_CUTOFF`); y the fade above it
    /// (`sky::HORIZON_FADE`); z [`SUN_TAKEN`]; w unused.
    pub fade: Vec4,
}

impl CloudShadowParams {
    /// No shadows: the deck's geometry, and nothing to draw with it.
    pub fn off() -> Self {
        Self {
            cover: Vec4::new(0.0, 0.0, 1.0 - sky::CLOUD_COVER, sky::EDGE),
            drift: Vec4::ZERO,
            sun: Vec4::ZERO,
            deck: Vec4::new(
                sky::CLOUD_ALT_M,
                1.0 / sky::CLOUD_SCALE_M,
                1.0 / sky::FIELD_PERIOD as f32,
                level() as f32,
            ),
            fade: Vec4::new(sky::DECK_CUTOFF, sky::HORIZON_FADE, SUN_TAKEN, 0.0),
        }
    }

    /// This frame's lanes: the weather's, in [`LANE_STEP`]s, the drift and
    /// the sun as `anchor` last took them, drawn when `on`.
    pub fn new(on: bool, w: &WeatherNow, anchor: &Anchor) -> Self {
        let step = |v: f32| (v / LANE_STEP).round() * LANE_STEP;
        // The rig's occlusion with no deck over the sun: what a heavy sky
        // and the fog take whatever the deck does.
        let mut floor = *w;
        floor.sun_cover = 0.0;
        let mut p = Self::off();
        p.cover.x = if on { 1.0 } else { 0.0 };
        p.cover.y = step(floor.occlusion());
        p.cover.z = step(1.0 - sky::deck_cover(w.cloud));
        p.drift = Vec4::new(anchor.off.x, anchor.off.y, anchor.vel.x, anchor.vel.y);
        p.sun = anchor.sun.extend(anchor.t);
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

/// Write the shadows' lanes and the deck's field into every ground
/// material, when they have moved. After `weather::update`.
pub fn drive(
    weather: Res<WeatherNow>,
    settings: Res<super::Settings>,
    time: Res<Time>,
    composer: Option<Res<SkyComposer>>,
    mut anchor: Local<Anchor>,
    mut ground: ResMut<Assets<GroundMaterial>>,
) {
    let Some(composer) = composer else {
        return;
    };
    // LOW is the frame with work taken out, and this is work: a second
    // look at the sun for most of the ground.
    let on = super::quality::effective(settings.gfx).clouds != Clouds::Low;
    anchor.track(
        time.elapsed_secs_wrapped(),
        -weather.drift / sky::CLOUD_SCALE_M,
        -weather.drift_vel / sky::CLOUD_SCALE_M,
        weather.sun,
    );
    let want = CloudShadowParams::new(on, &weather, &anchor);
    let field = composer.field_image();
    let stale: Vec<AssetId<GroundMaterial>> = ground
        .iter()
        .filter(|(_, m)| m.extension.cloud != want || m.extension.cloud_field != *field)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(m) = ground.get_mut(id) {
            m.extension.cloud = want;
            m.extension.cloud_field = field.clone();
        }
    }
}
