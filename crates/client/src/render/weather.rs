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
//! (a lagged echo of the rain), how much of the rain is **snow** (the air
//! the server last read at the body, the sim's own line through it), and
//! whether the eye is **under a roof or underwater**. Lightning is not
//! local: its bolts are drawn off the seed and the second
//! (`weather::bolt`), so a flash is the same flash for everyone.

use bevy::math::DVec2;
use bevy::prelude::*;
use sim_core::limits::TICK_HZ;
use sim_core::weather::{self, Wx};

use super::feed::Feed;
use super::rig::{self, DayPin};
use super::{Eye, Net, WorldId};

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
    /// Toward the bolt making this frame's flash, world XZ, unit — the
    /// bolt's `bearing` (`sim_core::weather::Bolt`), so the flash lights the
    /// island from the side it struck (`rig::BoltLight`).
    pub flash_dir: Vec2,
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
    /// How much of what falls is snow, `0..=1` (snow v0): the air the
    /// server last read at the body (`ClientCore::air_c`) through the sim's
    /// own line (`climate::snow_pm`), eased over a few seconds so a
    /// whole-degree step reads as a spell of sleet. Whether anything falls
    /// at all is still `rain`.
    pub snow: f32,
    /// How white the ground lies, `0..=1`: falling snow settles over a few
    /// minutes, stays while it freezes, and melts once the air is over it.
    pub ground_snow: f32,
    /// The lowest the eye has seen snow fall since the ground was last
    /// bare, metres: the snow lies there and on everything higher, where
    /// the air is colder still (`climate::air_dc_at`'s lapse).
    pub snow_floor: f32,
    /// The least and the most arid ground (`terrain::aridity`) the eye has
    /// seen snow fall on since the ground was last bare: the snow lies only
    /// on that band. The air is a function of the aridity and the height
    /// alone at any one moment, and which of desert and green is colder
    /// turns with the hour, so only ground as arid as where it fell, and as
    /// high, is surely as cold. A dawn storm that snows on the desert
    /// leaves the green south bare.
    pub snow_arid: Vec2,
    /// How fast [`Self::drift`] moves, m/s: what the ground's cloud
    /// shadows run on between the writes that re-anchor them
    /// (`cloud_shadow.rs`). Zero on a pinned sky, which holds still.
    pub drift_vel: Vec2,
    /// [`Self::drift`] as it is summed, in f64: f32 metres a field across
    /// wander off the wind's straight run by a metre in half a minute,
    /// and the ground's shadows run on that straight line.
    drift_sum: DVec2,
    /// How far through its month the moon is, `0..1`: 0 new, ½ full
    /// (`sim_core::world::moon_phase`).
    pub moon_phase: f32,
    /// What the moon gives the night, `0..=1`: its lit fraction while it
    /// is up (`moon::shine`). A day moon has it too, so readers scale it by
    /// `night` as they scale the moon.
    pub moonlight: f32,
    /// Toward the sun the moon is lit by, unit (`moon::lighting_sun`): the
    /// side its terminator faces.
    pub moon_sun: Vec3,
    /// How much sand the air carries round the eye, `0..=1` ([`dust_at`]):
    /// a reading of where the eye stands, the same for everyone there.
    pub dust: f32,
    /// The most rain of the last [`DUST_DRY_TICKS`], off the shared
    /// schedule — what the sand dries by. Not [`Self::ground_wet`], which
    /// is this client's own echo and starts dry on a reload.
    pub rain_lately: f32,
}

impl WeatherNow {
    /// How much of the direct sun the sky takes away, `0..=1`: the deck
    /// over the sun's own disk, or a heavy sky over all of it, whichever
    /// is more — and thick fog, which a sun cannot shine through either:
    /// the fog preset's 900‰ puts the shadows out (`rig::day_night`).
    pub fn occlusion(&self) -> f32 {
        self.sun_cover.max(self.occlusion_floor())
    }

    /// [`Self::occlusion`] whatever the deck over the sun's disk: a heavy
    /// sky and the fog. Where the ground draws the deck's own shadows
    /// (`cloud_shadow.rs`), this alone puts the sun's shadows out.
    pub fn occlusion_floor(&self) -> f32 {
        let heavy = ((self.cloud - 0.5) / 0.5).clamp(0.0, 1.0);
        heavy.max(self.fog)
    }

    /// Per-metre extinction of the weather's fog, rain and dust on
    /// geometry — the `DistanceFog` density. Zero in clear weather off the
    /// sand, so on the desktop the atmosphere stays the only haze until the
    /// weather adds one. ~150 m sight in the fog preset, ~500 m in a storm.
    pub fn fog_sigma(&self) -> f32 {
        self.fog_part() + self.rain_part() + self.dust_part()
    }

    /// The ground fog's share of [`Self::fog_sigma`], per metre.
    pub fn fog_part(&self) -> f32 {
        let f = self.fog;
        f * f * f * (3.0 / 110.0)
    }

    /// The rain that falls as rain, `0..=1` — what wets the ground and
    /// drums on it. Snow is near silent and lies white instead.
    pub fn falling_rain(&self) -> f32 {
        self.rain * (1.0 - self.snow)
    }

    /// The rain haze's share of [`Self::fog_sigma`], per metre.
    pub fn rain_part(&self) -> f32 {
        let r = self.rain;
        r * r * (3.0 / 600.0)
    }

    /// The dust haze's share of [`Self::fog_sigma`], per metre: squared,
    /// as the rain's is, so the desert's edge and a light breeze are a
    /// tint on the far hills and only a wind over open sand is a wall.
    pub fn dust_part(&self) -> f32 {
        let d = self.dust;
        d * d * DUST_SIGMA
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

/// How long the snow share takes to follow the air, seconds: a whole degree
/// is the reading's step, and this is the sleet that step crosses.
pub const SNOW_EASE_S: f32 = 4.0;
/// Full snow whitens the ground in about this long, seconds; a flurry takes
/// proportionally longer. Melting, once the air is over freezing, takes
/// [`SNOW_MELT_S`].
pub const SNOW_SETTLE_S: f32 = 150.0;
pub const SNOW_MELT_S: f32 = 400.0;
/// How far under the feet that saw it fall the snow is laid, metres.
pub const SNOW_FLOOR_SLACK_M: f32 = 3.0;
/// Where no snow has been seen: above any ground there is.
pub const NO_SNOW_FLOOR: f32 = 10_000.0;
/// [`WeatherNow::snow_arid`] with no snow seen (an empty band), and over
/// all the ground there is.
pub const NO_SNOW_ARID: Vec2 = Vec2::new(2.0, -1.0);
pub const ALL_ARID: Vec2 = Vec2::new(-1.0, 2.0);
// ── Dust ──────────────────────────────────────────────────────────────────
//
// Rust's Dust preset is beige fog over the whole map, and the schedule here
// dropped it because the island had no desert. Climate v0 gave it one in the
// north (`terrain::aridity`), and dust belongs to the ground rather than to
// the sky: the same wind that is a breeze over the meadow lifts sand off the
// dunes. So it is not a preset but a reading of the weather at a place —
// the wind, the dry, and how much sand lies under and upwind of the eye.
// No state and nothing on the wire: a pure function of the seed, the
// weather everyone shares and where the eye stands, so two players on one
// dune see one haze.

/// Full dust's extinction, per metre: ~450 m of sight (`3 / σ`), the
/// reference's beige wall. A gale over open sand; the schedule's driest
/// winds reach about two thirds of it.
const DUST_SIGMA: f32 = 3.0 / 450.0;
/// The wind that starts lifting sand and the wind that carries all it can,
/// smoothstepped: clear weather's 200‰ is a haze on the far dunes (~20 km
/// of sight), overcast's 350‰ a beige wall at ~750 m.
const DUST_WIND: (f32, f32) = (0.12, 0.45);
/// The aridity where open sand starts to show and where it is all sand.
/// The desert biome (`terrain::DESERT_MOIST`) begins near aridity 0.56 on
/// ground of average moisture, 0.2 to 0.95 across the moisture field.
const DUST_ARID: (f32, f32) = (0.25, 0.75);
/// Rain, or rain this heavy within the last [`DUST_DRY_TICKS`], lays the
/// dust: past this there is none.
const DUST_DAMP: f32 = 0.15;
/// How long the sand stays damp after the rain, ticks: 5 min. Read off the
/// schedule (`WeatherNow::rain_lately`), so a player who just joined sees
/// the sand as wet as one who stood through the shower.
pub const DUST_DRY_TICKS: u64 = 5 * 60 * TICK_HZ as u64;
/// How far upwind the sand is read, metres. The air at the eye came from
/// there, so the haze spills off the desert's edge downwind and thins
/// where clean air blows in.
const DUST_FETCH_M: [f32; 2] = [250.0, 500.0];
/// Height above the sea over which the shore turns from water, which lifts
/// no sand, to dry ground, metres.
const DUST_SHORE_M: f32 = 2.0;

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the ground at `(x, z)` is sand a wind can lift, `0..=1`:
/// its aridity on dry land, none over the sea or a lake (aridity is a band
/// of latitude and runs on out over the water).
fn sand_at(seed: u64, x: f32, z: f32) -> f32 {
    let arid = sim_core::terrain::aridity(seed, x, z);
    if arid <= 0.0 {
        // Most of the island: one compare, no height read.
        return 0.0;
    }
    let lo = sim_core::terrain::SEA_LEVEL;
    arid * smooth(lo, lo + DUST_SHORE_M, sim_core::terrain::height(seed, x, z))
}

/// How much sand the air at `(x, z)` carries in weather `w`, `0..=1`:
/// arid land under and upwind, times a wind to lift it, times dry — none in
/// rain or for a while after it. Reads only the seed and the weather every
/// client shares, so two players on one dune see one haze. Per mille, as
/// the weather is, so the sky's materials re-upload when the haze moves and
/// not every frame the eye does.
pub fn dust_at(seed: u64, x: f32, z: f32, w: &WeatherNow) -> f32 {
    let damp = w.rain.max(w.rain_lately);
    let dry = (1.0 - damp / DUST_DAMP).max(0.0);
    let windy = smooth(DUST_WIND.0, DUST_WIND.1, w.wind);
    if dry * windy <= 0.0 {
        return 0.0;
    }
    let up = -w.wind_dir;
    let mut sand = sand_at(seed, x, z);
    for f in DUST_FETCH_M {
        sand += sand_at(seed, x + up.x * f, z + up.y * f);
    }
    let sand = smooth(
        DUST_ARID.0,
        DUST_ARID.1,
        sand / (1 + DUST_FETCH_M.len()) as f32,
    );
    (sand * windy * dry * 1000.0).round() / 1000.0
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
    mut capture_snow: Local<Option<f32>>,
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
    let day_tick = pin.day_tick(tick_est, &feed.env);
    let frac = sim_core::world::day_frac(day_tick);
    let elev = rig::sun_elevation(frac);
    now.sun = rig::to_sun(frac);
    now.sun_lux = rig::sun_lux(frac);
    now.night = (-elev / 0.15).clamp(0.0, 1.0);
    // The moon keeps its calendar, on the sun's path behind it (`moon::toward`).
    now.moon_phase = sim_core::world::moon_phase(day_tick);
    now.moon = super::moon::toward(frac, now.moon_phase);
    now.moonlight = super::moon::shine(frac, now.moon_phase);
    now.moon_sun = super::moon::lighting_sun(frac, now.moon_phase);

    let dt = time.delta_secs();
    if pinned {
        // A probe's sky holds still and dry-or-soaked as asked. Its snow is
        // asked too (`GATES_CAPTURE_SNOW`, `0..1`: what falls, and how white
        // the ground lies), never the capture shard's air, which is a
        // function of the hour it booted at.
        let snow = *capture_snow.get_or_insert_with(|| {
            std::env::var("GATES_CAPTURE_SNOW")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .map_or(0.0, |v| v.clamp(0.0, 1.0))
        });
        now.drift = Vec2::ZERO;
        now.drift_vel = Vec2::ZERO;
        now.drift_sum = DVec2::ZERO;
        now.snow = snow;
        now.ground_wet = now.falling_rain();
        now.ground_snow = snow;
        now.snow_floor = -NO_SNOW_FLOOR;
        now.snow_arid = ALL_ARID;
        now.flash = 0.0;
    } else {
        // The deck drifts downwind: a breeze at clear, a gale in a storm.
        let speed = 1.5 + 6.0 * now.wind;
        let period = f64::from(super::sky::FIELD_PERIOD as f32 * super::sky::CLOUD_SCALE_M);
        now.drift_vel = now.wind_dir * speed;
        let sum = now.drift_sum + now.drift_vel.as_dvec2() * f64::from(dt);
        now.drift_sum = DVec2::new(sum.x.rem_euclid(period), sum.y.rem_euclid(period));
        now.drift = now.drift_sum.as_vec2();
        // Snow (snow v0): the air the server read at the body, through the
        // sim's line, eased so a whole-degree step is a spell of sleet. No
        // reading yet (or a shard whose exposure is off) is no snow.
        let air = net
            .as_ref()
            .map(|n| &n.session.core)
            .filter(|c| c.air_read)
            .map(|c| c.air_c);
        let snow_want = air.map_or(0.0, |c| pm(sim_core::climate::snow_pm(i16::from(c) * 10)));
        now.snow += (snow_want - now.snow) * (dt / SNOW_EASE_S).min(1.0);

        // Wet arrives in about a minute and dries over about eight — from
        // the rain that falls as rain; snow lies instead.
        let wet = now.falling_rain();
        let tau = if wet > now.ground_wet { 40.0 } else { 300.0 };
        now.ground_wet += (wet - now.ground_wet) * (dt / tau).min(1.0);
        // The snow settles as hard as it falls, stays while it freezes and
        // melts as the air warms. It lies from the lowest feet that saw it
        // fall and up, on ground as arid as where it fell; bare ground
        // forgets the line and the band.
        let settle = now.rain * now.snow / SNOW_SETTLE_S;
        let melt = (1.0 - now.snow) * now.ground_snow / SNOW_MELT_S;
        now.ground_snow = (now.ground_snow + (settle - melt) * dt).clamp(0.0, 1.0);
        if now.ground_snow < 0.01 {
            now.snow_floor = NO_SNOW_FLOOR;
            now.snow_arid = NO_SNOW_ARID;
        }
        if now.rain * now.snow > 0.05 {
            let feet = eye.pos.y - eye.height - SNOW_FLOOR_SLACK_M;
            now.snow_floor = now.snow_floor.min(feet);
            let arid = sim_core::terrain::aridity(world.seed, eye.pos.x, eye.pos.z);
            now.snow_arid = Vec2::new(now.snow_arid.x.min(arid), now.snow_arid.y.max(arid));
        }

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
            let f = flash_at(since) * near;
            if f > flash {
                flash = f;
                now.flash_dir = wind_vec(b.bearing);
            }
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

    // The sand in the air, off the wind and the schedule's recent rain. A
    // rain fades over minutes and holds for longer, so the window's two
    // ends bound the rain inside it.
    now.rain_lately = match wpin.0 {
        Some(_) => now.rain,
        None => {
            let then = weather::now(world.seed, tick.saturating_sub(DUST_DRY_TICKS), &feed.env);
            now.rain.max(pm(then.rain))
        }
    };
    now.dust = dust_at(world.seed, eye.pos.x, eye.pos.z, now);

    // Under a roof, or under the sea: what the rain and its sound ask.
    now.underwater = eye.pos.y < sim_core::terrain::SEA_LEVEL;
    now.sheltered = net.as_ref().is_some_and(|net| {
        sim_core::collide::roofed(
            world.seed,
            &world.haven,
            net.session.core.pieces.cols(),
            eye.pos.x,
            eye.pos.z,
            eye.pos.y - eye.height,
        )
    });
}
