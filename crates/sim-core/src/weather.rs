//! Weather (weather v0) — Rust's presets on Rust's schedule.
//!
//! The reference's weather is artist presets the code blends between
//! (Clear, Dust, Fog, Overcast, RainMild, RainHeavy, Storm), re-rolled every
//! 18 in-game hours and faded in over 6 (devblog 193) — 45 and 15 real
//! minutes on its 60-minute day. Which preset a block gets is a function of
//! the world seed and the clock, so every machine that knows both agrees on
//! the sky without a byte on the wire. We keep all of that except Dust (the
//! island has no desert) and keep the real-time numbers.
//!
//! **A pure function of `(seed, tick)`, the way `world::day_frac` is.** The
//! client derives the weather from the seed its welcome carried and the tick
//! estimate it already keeps; the sim reads the same function for animal
//! senses and exposure. The one stored part is [`Env`]: an admin's forced
//! preset (`/weather`, the reference's `weather.load`) and clock offset
//! (`/time`, its `env.time`), which rides one reliable event when it
//! changes and is hashed and saved like any other world state.
//!
//! Integers only, per mille, so the walls hold without a float in sight.

use crate::limits::{DAY_PHASE_TICKS, DAY_TICKS};
use crate::rng::cell_hash;

/// How long one preset holds, ticks: 45 min at 30 Hz (their 18 in-game h).
pub const SEG_TICKS: u64 = 81_000;
/// How long a new preset takes to fade in, ticks: 15 min (their 6 h).
pub const FADE_TICKS: u64 = 27_000;
/// How long an admin's `/weather` takes to arrive, ticks: 20 s — quick
/// enough to show off, slow enough that the sky does not pop.
pub const FORCE_FADE_TICKS: u64 = 600;

/// `Env::mode` when the schedule rules.
pub const MODE_AUTO: u8 = 0;
/// `Command::AdminEnv::weather` that leaves the sky alone.
pub const KEEP_WEATHER: u8 = 0xFF;
/// `Command::AdminEnv::time_pm` that leaves the clock alone.
pub const KEEP_TIME: u16 = 0xFFFF;
pub const CLEAR: u8 = 1;
pub const OVERCAST: u8 = 2;
pub const FOG: u8 = 3;
pub const RAIN_MILD: u8 = 4;
pub const RAIN_HEAVY: u8 = 5;
pub const STORM: u8 = 6;
/// The last preset code; `Env::mode` is `0..=PRESET_MAX`.
pub const PRESET_MAX: u8 = STORM;

const CH_WEATHER: u32 = 103;
const CH_WEATHER_PHASE: u32 = 105;
const CH_WIND: u32 = 106;
const CH_BOLT: u32 = 107;

/// One moment's weather, every field per mille (`0..=1000`) except the
/// wind's bearing, which is a yaw-LUT index (256 per turn, the sim's yaw
/// convention).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wx {
    /// How much of the sky carries cloud.
    pub cloud: u16,
    /// How dark and heavy the cloud is — grey bases, dim light.
    pub dark: u16,
    /// Precipitation.
    pub rain: u16,
    /// Ground fog.
    pub fog: u16,
    /// Wind strength.
    pub wind: u16,
    /// Thunder and lightning.
    pub thunder: u16,
    /// Which way the wind blows toward.
    pub wind_dir: u8,
}

impl Wx {
    /// Every per-mille field at or under 1000 — what a decoder refuses past.
    pub fn in_range(&self) -> bool {
        self.cloud <= 1000
            && self.dark <= 1000
            && self.rain <= 1000
            && self.fog <= 1000
            && self.wind <= 1000
            && self.thunder <= 1000
    }
}

/// `(weight out of 100, cloud, dark, rain, fog, wind, thunder)` per preset,
/// indexed by code − 1. Weights after the reference's defaults (clear 0.8,
/// rain 0.2, storm 0.1, fog 0.05), with some of clear moved to overcast so
/// the sky has a middle register.
const TABLE: [(u16, [u16; 6]); PRESET_MAX as usize] = [
    (55, [350, 0, 0, 0, 200, 0]),            // Clear
    (12, [900, 350, 0, 100, 350, 0]),        // Overcast
    (7, [700, 200, 0, 900, 100, 0]),         // Fog
    (13, [850, 450, 350, 250, 400, 0]),      // RainMild
    (8, [970, 650, 800, 400, 600, 150]),     // RainHeavy
    (5, [1000, 850, 1000, 300, 1000, 1000]), // Storm
];
const WEIGHT_SUM: u64 = 100;
const _: () = {
    let mut s = 0;
    let mut i = 0;
    while i < TABLE.len() {
        s += TABLE[i].0 as u64;
        i += 1;
    }
    assert!(s == WEIGHT_SUM, "preset weights must sum to WEIGHT_SUM");
};

/// A preset's weather, blowing toward `wind_dir`.
pub fn preset(code: u8, wind_dir: u8) -> Wx {
    let i = (code.clamp(CLEAR, PRESET_MAX) - 1) as usize;
    let f = TABLE[i].1;
    Wx {
        cloud: f[0],
        dark: f[1],
        rain: f[2],
        fog: f[3],
        wind: f[4],
        thunder: f[5],
        wind_dir,
    }
}

/// Short name for a preset code, for logs and the admin lane.
pub fn preset_name(code: u8) -> &'static str {
    match code {
        MODE_AUTO => "auto",
        CLEAR => "clear",
        OVERCAST => "overcast",
        FOG => "fog",
        RAIN_MILD => "rain",
        RAIN_HEAVY => "heavy rain",
        STORM => "storm",
        _ => "?",
    }
}

fn seg_hash(seed: u64, seg: u64, channel: u32) -> u64 {
    cell_hash(seed, seg as u32 as i32, (seg >> 32) as u32 as i32, channel)
}

/// Which preset segment `seg` rolled.
pub fn preset_of(seed: u64, seg: u64) -> u8 {
    let mut roll = seg_hash(seed, seg, CH_WEATHER) % WEIGHT_SUM;
    for (i, row) in TABLE.iter().enumerate() {
        if roll < row.0 as u64 {
            return i as u8 + 1;
        }
        roll -= row.0 as u64;
    }
    CLEAR
}

fn wind_of(seed: u64, seg: u64) -> u8 {
    (seg_hash(seed, seg, CH_WIND) >> 24) as u8
}

/// Where this seed's segment boundaries fall, so two shards do not change
/// weather on the same tick and a segment edge is not pinned to dawn.
fn phase(seed: u64) -> u64 {
    cell_hash(seed, 0, 0, CH_WEATHER_PHASE) % SEG_TICKS
}

/// The schedule's segment index at `tick`, and how far into it.
pub fn segment(seed: u64, tick: u64) -> (u64, u64) {
    let t = tick + phase(seed);
    (t / SEG_TICKS, t % SEG_TICKS)
}

/// `x` per mille through a smoothstep, per mille: `x²(3 − 2x)`.
pub fn smooth_pm(x: u64) -> u64 {
    let x = x.min(1000);
    x * x * (3000 - 2 * x) / 1_000_000
}

fn mix(a: u16, b: u16, t_pm: u64) -> u16 {
    let (a, b) = (a as i64, b as i64);
    (a + (b - a) * t_pm as i64 / 1000) as u16
}

/// Bearing blend along the shorter way round.
fn mix_dir(a: u8, b: u8, t_pm: u64) -> u8 {
    let d = b.wrapping_sub(a) as i8 as i64;
    (a as i64 + d * t_pm as i64 / 1000) as u8
}

/// `a` → `b` at `t_pm` per mille.
pub fn blend(a: &Wx, b: &Wx, t_pm: u64) -> Wx {
    Wx {
        cloud: mix(a.cloud, b.cloud, t_pm),
        dark: mix(a.dark, b.dark, t_pm),
        rain: mix(a.rain, b.rain, t_pm),
        fog: mix(a.fog, b.fog, t_pm),
        wind: mix(a.wind, b.wind, t_pm),
        thunder: mix(a.thunder, b.thunder, t_pm),
        wind_dir: mix_dir(a.wind_dir, b.wind_dir, t_pm),
    }
}

/// The schedule's weather at `tick`: the segment's preset, faded in from
/// the last segment's over `FADE_TICKS`.
pub fn auto(seed: u64, tick: u64) -> Wx {
    let (seg, into) = segment(seed, tick);
    let cur = preset(preset_of(seed, seg), wind_of(seed, seg));
    if into >= FADE_TICKS {
        return cur;
    }
    let prev_seg = seg.wrapping_sub(1);
    let prev = preset(preset_of(seed, prev_seg), wind_of(seed, prev_seg));
    blend(&prev, &cur, smooth_pm(into * 1000 / FADE_TICKS))
}

/// The world's stored say over the sky and the clock: an admin's forced
/// preset and clock offset. `Default` is the schedule, untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// `MODE_AUTO`, or the forced preset's code.
    pub mode: u8,
    /// The tick the last change finishes fading in; before it the weather
    /// is on its way from `from`. 0 when nothing ever changed.
    pub fade_end: u64,
    /// The weather at the moment of the last change.
    pub from: Wx,
    /// Added to the tick for the day clock (`day_tick`), `< DAY_TICKS`.
    pub day_offset: u32,
}

impl Env {
    /// Clear skies held for good — what a fixture pins so the schedule
    /// cannot put fog on its test tick.
    pub const CLEAR: Env = Env {
        mode: CLEAR,
        fade_end: 0,
        from: Wx {
            cloud: 0,
            dark: 0,
            rain: 0,
            fog: 0,
            wind: 0,
            thunder: 0,
            wind_dir: 0,
        },
        day_offset: 0,
    };

    /// Every field inside its domain — what a save or wire decoder admits.
    pub fn valid(&self) -> bool {
        self.mode <= PRESET_MAX && self.from.in_range() && (self.day_offset as u64) < DAY_TICKS
    }

    /// Force a preset (`MODE_AUTO` hands the sky back to the schedule),
    /// fading from whatever the sky is doing right now.
    pub fn force(&mut self, seed: u64, tick: u64, mode: u8) {
        if mode > PRESET_MAX {
            return;
        }
        self.from = now(seed, tick, self);
        self.fade_end = tick + FORCE_FADE_TICKS;
        self.mode = mode;
    }

    /// Move the day clock so that `day_frac` reads `frac_pm` per mille now.
    pub fn set_time(&mut self, tick: u64, frac_pm: u16) {
        let want = frac_pm.min(999) as u64 * DAY_TICKS / 1000;
        let at = (tick + DAY_PHASE_TICKS) % DAY_TICKS;
        self.day_offset = ((want + DAY_TICKS - at) % DAY_TICKS) as u32;
    }
}

/// The weather at `tick` under `env`.
pub fn now(seed: u64, tick: u64, env: &Env) -> Wx {
    let target = if env.mode == MODE_AUTO {
        auto(seed, tick)
    } else {
        // A forced sky keeps the schedule's wind bearing, so the wind does
        // not swing round because an admin asked for rain.
        preset(env.mode, auto(seed, tick).wind_dir)
    };
    if tick < env.fade_end {
        let left = (env.fade_end - tick).min(FORCE_FADE_TICKS);
        let t = smooth_pm(1000 - left * 1000 / FORCE_FADE_TICKS);
        blend(&env.from, &target, t)
    } else {
        target
    }
}

/// The tick the day clock reads (`world::day_frac`/`is_night`): the sim's
/// own, shifted by any `/time`.
#[inline]
pub fn day_tick(tick: u64, env: &Env) -> u64 {
    tick + env.day_offset as u64
}

/// How far an animal notices a player in this weather, per mille of its
/// clear-day reach: fog halves it, rain takes a quarter, never under 40%.
pub fn sense_pm(wx: &Wx) -> u32 {
    let cut = wx.fog as u32 / 2 + wx.rain as u32 / 4;
    1000u32.saturating_sub(cut).max(400)
}

/// One lightning bolt: when in its second it flashes, how far away, and
/// which way. Client-side light and sound, but drawn here so every client
/// sees the same bolt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bolt {
    pub at_ms: u16,
    pub dist_m: u16,
    pub bearing: u8,
}

/// The bolt in second `sec` (ticks / TICK_HZ), if any: about one in eleven
/// seconds at full thunder, none without it.
pub fn bolt(seed: u64, sec: u64, thunder: u16) -> Option<Bolt> {
    let h = seg_hash(seed, sec, CH_BOLT);
    if h % 1000 >= thunder as u64 * 90 / 1000 {
        return None;
    }
    Some(Bolt {
        at_ms: ((h >> 16) % 1000) as u16,
        dist_m: 300 + ((h >> 32) % 3700) as u16,
        bearing: (h >> 56) as u8,
    })
}
