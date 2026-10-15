//! Air temperature (climate v0) — the reference's `Climate.GetTemperature`.
//!
//! Rust blends a per-biome temperature curve over the hour of the day by
//! the biome weights at a point: temperate ground mild, the arid band hot
//! by day and cold by night, the snow band cold. Ours has the temperate
//! south and middle and the arid north (`terrain::aridity`), and it has
//! mountains where Rust has snow, so altitude takes the snow's place: the
//! air loses a degree every `lapse_m` above `lapse_from_m`. Cloud flattens
//! the day's swing (an overcast noon is cooler, an overcast night warmer)
//! and rain cools the air outright.
//!
//! **A pure function of `(seed, point, day tick, weather)`**, the way
//! `weather::now` is, so it costs no state, no save and no wire. The sim
//! reads it once a second per body for exposure; the server sends the
//! owner the whole degrees it read.
//!
//! Integers in deci-degrees (tenths of a °C) past the one float the
//! aridity field is.

use crate::limits::{DAY_PHASE_TICKS, DAY_TICKS};
use crate::weather::{smooth_pm, Wx};

/// The air's rule, baked from `content/balance.toml` `[climate]`.
/// Temperatures in deci-degrees Celsius.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClimateContent {
    /// Temperate ground's coldest hour (dawn) and warmest (mid-afternoon).
    pub temperate_low_dc: i16,
    pub temperate_high_dc: i16,
    /// The desert's: hotter days, colder nights.
    pub arid_low_dc: i16,
    pub arid_high_dc: i16,
    /// Above this altitude the air cools, a degree every `lapse_m`.
    pub lapse_from_m: u16,
    pub lapse_m: u16,
    /// How much of the day's swing full cloud takes away, per mille.
    pub cloud_flatten_pm: u16,
    /// How much colder the heaviest rain makes the air.
    pub rain_cool_dc: i16,
    /// A crop grows at full pace from `crop_warm_dc` to `crop_hot_dc`,
    /// slower toward `crop_frost_dc` and `crop_scorch_dc`, and not at all
    /// past either (the reference's per-plant temperature curve).
    pub crop_frost_dc: i16,
    pub crop_warm_dc: i16,
    pub crop_hot_dc: i16,
    pub crop_scorch_dc: i16,
}

impl ClimateContent {
    pub const EMPTY: Self = Self {
        temperate_low_dc: 0,
        temperate_high_dc: 0,
        arid_low_dc: 0,
        arid_high_dc: 0,
        lapse_from_m: 0,
        lapse_m: 0,
        cloud_flatten_pm: 0,
        rain_cool_dc: 0,
        crop_frost_dc: 0,
        crop_warm_dc: 0,
        crop_hot_dc: 0,
        crop_scorch_dc: 0,
    };
}

/// Where the day's warmest hour falls, per mille of the cycle from dawn:
/// past noon (`DAY_PORTION / 2`, 437), as the ground gives back the sun.
pub const PEAK_PM: u64 = 600;

/// How far through its warming the day is at `day_tick` (the day clock,
/// `weather::day_tick`), per mille: 0 at dawn, 1000 at [`PEAK_PM`], back to
/// 0 at the next dawn. Smoothstepped up and down, so the coldest hour is
/// the dawn and the night falls off steepest after dusk.
pub fn warmth_pm(day_tick: u64) -> u64 {
    let f = (day_tick.wrapping_add(DAY_PHASE_TICKS) % DAY_TICKS) * 1000 / DAY_TICKS;
    if f < PEAK_PM {
        smooth_pm(f * 1000 / PEAK_PM)
    } else {
        1000 - smooth_pm((f - PEAK_PM) * 1000 / (1000 - PEAK_PM))
    }
}

fn lerp(a: i16, b: i16, t_pm: i32) -> i32 {
    a as i32 + (b as i32 - a as i32) * t_pm / 1000
}

/// The air at a point, deci-degrees: `arid_pm` is how arid the ground is
/// (`terrain::aridity` × 1000), `alt_m` how high the body stands above the
/// sea.
pub fn air_dc_at(cc: &ClimateContent, arid_pm: u16, alt_m: i32, day_tick: u64, wx: &Wx) -> i16 {
    let arid = arid_pm.min(1000) as i32;
    let lo = lerp(cc.temperate_low_dc, cc.arid_low_dc, arid);
    let hi = lerp(cc.temperate_high_dc, cc.arid_high_dc, arid);
    // Cloud pulls the hour toward the middle of its swing.
    let w = warmth_pm(day_tick) as i32;
    let keep = 1000 - wx.cloud.min(1000) as i32 * cc.cloud_flatten_pm.min(1000) as i32 / 1000;
    let w = 500 + (w - 500) * keep / 1000;
    let mut t = lo + (hi - lo) * w / 1000;
    t -= wx.rain.min(1000) as i32 * cc.rain_cool_dc as i32 / 1000;
    if cc.lapse_m > 0 && alt_m > cc.lapse_from_m as i32 {
        t -= (alt_m - cc.lapse_from_m as i32) * 10 / cc.lapse_m as i32;
    }
    t.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// [`air_dc_at`] at a world point: reads the aridity field there.
pub fn air_dc(
    cc: &ClimateContent,
    seed: u64,
    x: f32,
    z: f32,
    alt_m: f32,
    day_tick: u64,
    wx: &Wx,
) -> i16 {
    let arid = crate::terrain::aridity(seed, x, z);
    let arid_pm = (arid * 1000.0).clamp(0.0, 1000.0) as u16;
    let alt = alt_m.clamp(-1000.0, 10_000.0) as i32;
    air_dc_at(cc, arid_pm, alt, day_tick, wx)
}

/// How fast a crop grows in this air, per mille (crops and climate v0):
/// the reference's growth is its worst factor, temperature one of them.
/// Content with no crop curve grows at full pace whatever the air.
pub fn crop_pm(cc: &ClimateContent, air_dc: i16) -> u16 {
    let (f, w, h, s) = (
        cc.crop_frost_dc as i32,
        cc.crop_warm_dc as i32,
        cc.crop_hot_dc as i32,
        cc.crop_scorch_dc as i32,
    );
    if !(f < w && w <= h && h < s) {
        return 1000;
    }
    let t = air_dc as i32;
    let pm = if t <= f || t >= s {
        0
    } else if t < w {
        (t - f) * 1000 / (w - f)
    } else if t > h {
        (s - t) * 1000 / (s - h)
    } else {
        1000
    };
    pm as u16
}

/// Whole degrees, rounded, for the HUD.
#[inline]
pub fn whole_c(dc: i16) -> i8 {
    let d = dc as i32;
    let r = if d >= 0 { (d + 5) / 10 } else { (d - 5) / 10 };
    r.clamp(i8::MIN as i32, i8::MAX as i32) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc() -> ClimateContent {
        ClimateContent {
            temperate_low_dc: 80,
            temperate_high_dc: 240,
            arid_low_dc: 40,
            arid_high_dc: 420,
            lapse_from_m: 30,
            lapse_m: 10,
            cloud_flatten_pm: 400,
            rain_cool_dc: 60,
            crop_frost_dc: 0,
            crop_warm_dc: 120,
            crop_hot_dc: 300,
            crop_scorch_dc: 450,
        }
    }

    #[test]
    fn crops_grow_in_the_warm_and_stop_in_a_frost_or_a_scorch() {
        let c = cc();
        assert_eq!(crop_pm(&c, -20), 0);
        assert_eq!(crop_pm(&c, 0), 0);
        assert_eq!(crop_pm(&c, 60), 500);
        assert_eq!(crop_pm(&c, 200), 1000);
        assert_eq!(crop_pm(&c, 375), 500);
        assert_eq!(crop_pm(&c, 450), 0);
        assert_eq!(
            crop_pm(&ClimateContent::EMPTY, -300),
            1000,
            "no curve, no say"
        );
    }

    /// The tick (on a fresh day clock) that is `pm` per mille from dawn.
    fn at(pm: u64) -> u64 {
        (pm * DAY_TICKS / 1000 + DAY_TICKS - DAY_PHASE_TICKS) % DAY_TICKS
    }

    #[test]
    fn the_day_warms_to_the_afternoon_and_cools_to_dawn() {
        assert_eq!(warmth_pm(at(0)), 0);
        assert_eq!(warmth_pm(at(PEAK_PM)), 1000);
        assert!(warmth_pm(at(437)) > 800, "noon is most of the way up");
        assert!(warmth_pm(at(940)) < 100, "deep night is near the low");
        let mut last = 0;
        for pm in 0..=PEAK_PM {
            let w = warmth_pm(at(pm));
            assert!(w >= last, "the morning never cools");
            last = w;
        }
    }

    #[test]
    fn the_desert_swings_wider_and_the_mountain_is_colder() {
        let c = cc();
        let clear = Wx::default();
        let dawn = at(0);
        let peak = at(PEAK_PM);
        assert_eq!(air_dc_at(&c, 0, 10, dawn, &clear), 80);
        assert_eq!(air_dc_at(&c, 0, 10, peak, &clear), 240);
        assert_eq!(air_dc_at(&c, 1000, 10, peak, &clear), 420);
        assert_eq!(air_dc_at(&c, 1000, 10, dawn, &clear), 40);
        // 100 m above the lapse line is ten degrees colder.
        assert_eq!(air_dc_at(&c, 0, 130, peak, &clear), 140);
    }

    #[test]
    fn cloud_flattens_the_swing_and_rain_cools() {
        let c = cc();
        let overcast = Wx {
            cloud: 1000,
            ..Wx::default()
        };
        let peak = at(PEAK_PM);
        let dawn = at(0);
        // Mid is 160; full cloud keeps 60 % of the 80-degree half swing.
        assert_eq!(air_dc_at(&c, 0, 0, peak, &overcast), 160 + 48);
        assert_eq!(air_dc_at(&c, 0, 0, dawn, &overcast), 160 - 48);
        let storm = Wx {
            cloud: 1000,
            rain: 1000,
            ..Wx::default()
        };
        assert_eq!(air_dc_at(&c, 0, 0, peak, &storm), 160 + 48 - 60);
    }

    #[test]
    fn whole_degrees_round_half_away() {
        assert_eq!(whole_c(84), 8);
        assert_eq!(whole_c(85), 9);
        assert_eq!(whole_c(-85), -9);
        assert_eq!(whole_c(-84), -8);
    }
}
