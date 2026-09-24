//! The weather, as the renderer reads it (weather v0).
//!
//! The sky is `sim_core::weather::now(seed, tick, &env)` — a function of the
//! seed the welcome carried, the tick estimate `Feed` already keeps and the
//! admin's stored `Env` — so every client at one campfire sees one sky
//! without a byte of weather on the wire. This file turns that into the
//! per-frame numbers the rig, the deck, the rain and the audio read: one
//! resource, one writer, like the day clock it sits beside.
//!
//! What is local here and why: the cloud **drift** (integrated from the
//! wind, a look nobody's gameplay reads), how **wet** the ground looks
//! (a lagged echo of the rain), and whether the eye is **under a roof or
//! underwater**. Lightning is not local: its bolts are drawn off the seed
//! and the second (`weather::bolt`), so a flash is the same flash for
//! everyone.

use bevy::prelude::*;
use sim_core::limits::TICK_HZ;
use sim_core::weather::{self, Wx};

use super::feed::Feed;
use super::rig::{self, DayPin};
use super::{Eye, Net, WorldId, EYE_HEIGHT};

/// `--weather` on a capture run: the preset the probe shoots under, so a
/// frame is not a function of which segment the capture shard booted in.
/// `None` in play.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WeatherPin(pub Option<u8>);

impl WeatherPin {
    /// A capture run's pin: the asked-for preset, clear by default.
    pub fn capture(preset: Option<u8>) -> Self {
        Self(Some(preset.unwrap_or(weather::CLEAR)))
    }
}

/// How many thunderclaps can be waiting for their sound at once.
pub const THUNDER_QUEUE: usize = 4;

/// This frame's weather, `0..=1` per field. `Default` is a still, clear,
/// dry sky with nothing occluding the sun — what every reader falls back
/// to when the resource is absent (the headless fixtures).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct WeatherNow {
    pub cloud: f32,
    pub dark: f32,
    pub rain: f32,
    pub fog: f32,
    pub wind: f32,
    pub thunder: f32,
    /// Which way the wind blows toward, world XZ, unit.
    pub wind_dir: Vec2,
    /// The lightning flash this frame, `0..=1`.
    pub flash: f32,
    /// A roof over the eye (`collide::roofed`).
    pub sheltered: bool,
    /// The eye is under the sea.
    pub underwater: bool,
    /// How wet the ground looks: the rain, arriving over a minute and
    /// drying over several.
    pub ground_wet: f32,
    /// How far the cloud deck has drifted downwind, metres.
    pub drift: Vec2,
    /// Toward the sun, unit (`rig::to_sun` at this hour).
    pub sun: Vec3,
    /// The sun's share of full daylight, twilight tail included
    /// (`rig::sun_lux`).
    pub sun_lux: f32,
    /// How far into the night the sky is, `0..=1` — what stars and the
    /// moon scale by.
    pub night: f32,
    /// Toward the moon, unit.
    pub moon: Vec3,
    /// How much of the sun's disk the deck covers right now (`sky`).
    pub sun_cover: f32,
    /// Thunder due: `(seconds of server time it should sound, gain)`.
    pub thunder_due: [(f64, f32); THUNDER_QUEUE],
    pub thunder_n: usize,
    /// The last second whose bolt was queued, so a bolt is heard once.
    last_bolt: u64,
}

impl WeatherNow {
    /// How much of the direct sun the sky takes away, `0..=1`: the deck
    /// over the sun's own disk, or a heavy sky over all of it, whichever
    /// is more — and thick fog, which a sun cannot shine through either:
    /// the fog preset's 900‰ puts the shadows out (`rig::day_night`).
    pub fn occlusion(&self) -> f32 {
        let heavy = ((self.cloud - 0.5) / 0.5).clamp(0.0, 1.0);
        self.sun_cover.max(heavy).max(self.fog)
    }

    /// Per-metre extinction of the weather's fog and rain on geometry —
    /// the `DistanceFog` density. Zero in clear weather, so on the desktop
    /// the atmosphere stays the only haze until the weather adds one.
    /// ~150 m sight in the fog preset, ~500 m in a storm.
    pub fn fog_sigma(&self) -> f32 {
        let f = self.fog;
        let r = self.rain;
        f * f * f * (3.0 / 110.0) + r * r * (3.0 / 600.0)
    }

    /// Queue a thunderclap, dropping it if the queue is full (a storm is
    /// loud enough without the fifth).
    fn queue_thunder(&mut self, at: f64, gain: f32) {
        if self.thunder_n < THUNDER_QUEUE {
            self.thunder_due[self.thunder_n] = (at, gain);
            self.thunder_n += 1;
        }
    }

    /// Take every clap whose time has come, for the audio system.
    pub fn take_thunder(&mut self, now_s: f64) -> Option<f32> {
        let i = (0..self.thunder_n).find(|&i| self.thunder_due[i].0 <= now_s)?;
        let gain = self.thunder_due[i].1;
        self.thunder_n -= 1;
        self.thunder_due[i] = self.thunder_due[self.thunder_n];
        Some(gain)
    }
}

fn pm(v: u16) -> f32 {
    v as f32 * 0.001
}

/// The wind's bearing (a yaw-LUT index, the sim's convention) as a unit
/// vector in world XZ.
pub fn wind_vec(dir: u8) -> Vec2 {
    let a = dir as f32 * (std::f32::consts::TAU / 256.0);
    Vec2::new(a.sin(), a.cos())
}

/// The flash a bolt `dt` seconds after it struck: a bright stroke and a
/// fainter restrike, gone inside a third of a second.
fn flash_at(dt: f64) -> f32 {
    if !(0.0..0.35).contains(&dt) {
        return 0.0;
    }
    let t = dt as f32;
    let first = (1.0 - t / 0.08).max(0.0);
    let second = 0.6 * (1.0 - ((t - 0.16) / 0.07).abs()).max(0.0);
    first.max(second)
}

/// Read the sky for this frame. Before `rig::day_night`, which lights the
/// frame from it, and before the deck and the rain, which draw it.
#[allow(clippy::too_many_arguments)]
pub fn update(
    feed: Res<Feed>,
    pin: Res<DayPin>,
    wpin: Res<WeatherPin>,
    world: Res<WorldId>,
    eye: Res<Eye>,
    net: Option<NonSend<Net>>,
    time: Res<Time>,
    composer: Option<Res<super::sky::SkyComposer>>,
    mut now_res: ResMut<WeatherNow>,
) {
    let now = &mut *now_res;
    let tick_est = feed.server_tick_est.max(0.0);
    let tick = pin.tick(tick_est);
    let pinned = wpin.0.is_some();
    let wx: Wx = match wpin.0 {
        Some(p) => weather::preset(p, 0),
        None => weather::now(world.seed, tick, &feed.env),
    };
    now.cloud = pm(wx.cloud);
    now.dark = pm(wx.dark);
    now.rain = pm(wx.rain);
    now.fog = pm(wx.fog);
    now.wind = pm(wx.wind);
    now.thunder = pm(wx.thunder);
    now.wind_dir = wind_vec(wx.wind_dir);

    // The hour, for the deck's lit side, its stars and its moon.
    let frac = sim_core::world::day_frac(pin.day_tick(tick_est, &feed.env));
    let elev = rig::sun_elevation(frac);
    now.sun = rig::to_sun(frac);
    now.sun_lux = rig::sun_lux(frac);
    now.night = (-elev / 0.15).clamp(0.0, 1.0);
    // The moon rides opposite the sun and higher as the night deepens.
    now.moon = rig::to_sun_at(
        (0.25 - elev).min(1.1),
        rig::sun_azimuth(frac) + std::f32::consts::PI,
    );

    let dt = time.delta_secs();
    if pinned {
        // A probe's sky holds still and dry-or-soaked as asked.
        now.drift = Vec2::ZERO;
        now.ground_wet = now.rain;
        now.flash = 0.0;
    } else {
        // The deck drifts downwind: a breeze at clear, a gale in a storm.
        let speed = 1.5 + 6.0 * now.wind;
        let period = super::sky::FIELD_PERIOD as f32 * super::sky::CLOUD_SCALE_M;
        now.drift += now.wind_dir * speed * dt;
        now.drift.x = now.drift.x.rem_euclid(period);
        now.drift.y = now.drift.y.rem_euclid(period);
        // Wet arrives in about a minute and dries over about eight.
        let tau = if now.rain > now.ground_wet {
            40.0
        } else {
            300.0
        };
        now.ground_wet += (now.rain - now.ground_wet) * (dt / tau).min(1.0);

        // Lightning: this second's bolt and the last one's, since a bolt
        // late in a second is still flashing early in the next.
        let now_s = tick_est / TICK_HZ as f64;
        let sec = now_s as u64;
        let mut flash = 0.0f32;
        for s in [sec.saturating_sub(1), sec] {
            let Some(b) = weather::bolt(world.seed, s, wx.thunder) else {
                continue;
            };
            let struck = s as f64 + b.at_ms as f64 / 1000.0;
            let since = now_s - struck;
            // Far bolts light the sky less.
            let near = (1.0 - b.dist_m as f32 / 5000.0).clamp(0.2, 1.0);
            flash = flash.max(flash_at(since) * near);
            if since >= 0.0 && s > now.last_bolt {
                now.last_bolt = s;
                let delay = b.dist_m as f64 / 343.0;
                let gain = (1.2 - b.dist_m as f32 / 4000.0).clamp(0.15, 1.0);
                now.queue_thunder(struck + delay, gain);
            }
        }
        now.flash = flash;
    }

    // How much of the sun's disk the deck covers: the rig puts the disk
    // out behind cloud and takes the direct light with it.
    now.sun_cover = composer.map_or(0.0, |c| {
        c.cover_at(now.sun, super::sky::deck_cover(now.cloud), now.drift)
    });

    // Under a roof, or under the sea: what the rain and its sound ask.
    now.underwater = eye.pos.y < sim_core::terrain::SEA_LEVEL;
    now.sheltered = net.as_ref().is_some_and(|net| {
        sim_core::collide::roofed(
            world.seed,
            &world.haven,
            net.session.core.pieces.cols(),
            eye.pos.x,
            eye.pos.z,
            eye.pos.y - EYE_HEIGHT,
        )
    });
}
