//! Wet and cold (weather v0) — the reference's exposure, in the survival
//! clock's shape.
//!
//! Two meters on the body, both per mille:
//!
//! - **Wet.** Rain soaks you unless something is overhead; standing in the
//!   sea soaks you to the depth you stand in; out of it you dry slowly, and
//!   fast beside a fire. The reference's `weather.wetness_rain` is the rate
//!   and a roof is what stops it.
//! - **Chill.** Where the body is heading is the night, the rain and the
//!   wind that reach it, and how wet it is — made worse by a piece that
//!   draws cold in, less what it wears (wet clothes keep half their warmth)
//!   and less any fire or flame in hand.
//!   The body moves toward that at a rate, so stepping under a roof for a
//!   second changes nothing and a night in the rain changes a lot. Past
//!   `hurt_at` it costs hp, through the same unreduced funnel starvation
//!   uses: cold is not a hit and armor does not blunt it.
//!
//! Stepped once a second per body, staggered by slot, on live and downed
//! bodies only — a sleeper's metabolism runs, but an offline player must not
//! freeze to death in a rainstorm they never saw. Every rate is integer
//! arithmetic and the one partial quantity (hp owed) lives in an exact
//! accumulator, `survival::tick_units`, for wall 5's reason.
//!
//! `EMPTY` disarms it: content with no `[exposure]` plays the game it did.

use crate::limits::MAX_ITEM_DEFS;
use crate::survival::{tick_units, Step};
use crate::world::{EventQueue, Player, EV_HEALTH};

/// The exposure ruleset, baked from `content/balance.toml` `[exposure]` and
/// `armor.toml`'s `cold_pct`. Everything per mille of a full meter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExposureContent {
    /// Wet gained a second in the heaviest rain, uncovered.
    pub wet_rain_per_s: u16,
    /// Wet lost a second when nothing is wetting you.
    pub dry_per_s: u16,
    /// …and beside a fire.
    pub dry_fire_per_s: u16,
    /// Water this deep, centimetres, soaks you through.
    pub soak_depth_cm: u16,
    /// Chill the night adds.
    pub night_cold: u16,
    /// Chill the heaviest rain adds, uncovered.
    pub rain_cold: u16,
    /// Chill the strongest wind adds, uncovered.
    pub wind_cold: u16,
    /// Chill being soaked through adds.
    pub wet_cold: u16,
    /// Chill a fire within `heat_radius_cm` takes away.
    pub fire_warmth: u16,
    /// Chill a lit torch in hand takes away.
    pub torch_warmth: u16,
    /// How near a fire has to be to warm you, centimetres.
    pub heat_radius_cm: u16,
    /// How fast the body's chill follows its target, a second, each way.
    pub chill_rise_per_s: u16,
    pub chill_fall_per_s: u16,
    /// Past this chill the cold costs hp.
    pub hurt_at: u16,
    /// Hit points a minute at full chill, scaling from zero at `hurt_at`.
    pub hurt_hp_per_min: u16,
    /// Per item index: the chill a worn piece keeps out. Negative draws it
    /// in — a road sign jacket is a sheet of metal — as a per-mille share
    /// of the cold the body is already in, never as cold of its own.
    pub warmth: [i16; MAX_ITEM_DEFS],
}

impl ExposureContent {
    pub const EMPTY: Self = Self {
        wet_rain_per_s: 0,
        dry_per_s: 0,
        dry_fire_per_s: 0,
        soak_depth_cm: 0,
        night_cold: 0,
        rain_cold: 0,
        wind_cold: 0,
        wet_cold: 0,
        fire_warmth: 0,
        torch_warmth: 0,
        heat_radius_cm: 0,
        chill_rise_per_s: 0,
        chill_fall_per_s: 0,
        hurt_at: 1000,
        hurt_hp_per_min: 0,
        warmth: [0; MAX_ITEM_DEFS],
    };

    /// Is exposure armed? A zero chill rate means nothing ever moves.
    #[inline]
    pub fn armed(&self) -> bool {
        self.chill_rise_per_s > 0
    }

    /// Synthetic table for the parity/replay/alloc gates: fast enough that a
    /// soak, a chill and a cold death land inside a counted probe window.
    /// Item 0 — the probe's fixture weapon — is worn warmth, so the worn sum
    /// is exercised too.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.wet_rain_per_s = 250;
        c.dry_per_s = 20;
        c.dry_fire_per_s = 200;
        c.soak_depth_cm = 100;
        c.night_cold = 500;
        c.rain_cold = 400;
        c.wind_cold = 200;
        c.wet_cold = 500;
        c.fire_warmth = 1000;
        c.torch_warmth = 150;
        c.heat_radius_cm = 450;
        c.chill_rise_per_s = 200;
        c.chill_fall_per_s = 200;
        c.hurt_at = 500;
        c.hurt_hp_per_min = 1200;
        c.warmth[0] = 100;
        c
    }
}

impl Default for ExposureContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// What the world around one body is doing this second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    /// The weather's rain and wind, per mille (`weather::Wx`).
    pub rain: u16,
    pub wind: u16,
    /// After dusk on the day clock.
    pub night: bool,
    /// A roof overhead (`collide::roofed`).
    pub roofed: bool,
    /// How deep the sea is at the body's feet, centimetres.
    pub depth_cm: u16,
    /// A lit fire within reach of its warmth.
    pub fire: bool,
    /// A lit torch in hand.
    pub torch: bool,
}

/// The chill a body is heading for, per mille — the whole rule in one
/// place so the tests and the step read the same sentence.
pub fn target_chill(ec: &ExposureContent, inp: &Inputs, p: &Player) -> u16 {
    let mut t: i32 = 0;
    if inp.night {
        t += ec.night_cold as i32;
    }
    if !inp.roofed {
        t += (inp.rain as i32 * ec.rain_cold as i32 + inp.wind as i32 * ec.wind_cold as i32) / 1000;
    }
    t += p.wet as i32 * ec.wet_cold as i32 / 1000;
    // What is worn splits in two: warmth that keeps cold out, and a draw
    // that lets it in (a road sign jacket is a sheet of metal).
    let (mut keep, mut draw) = (0i32, 0i32);
    for s in p.worn.iter() {
        if s.count == 0 || s.item as usize >= MAX_ITEM_DEFS {
            continue;
        }
        let w = ec.warmth[s.item as usize] as i32;
        if w >= 0 {
            keep += w;
        } else {
            draw -= w;
        }
    }
    // A draw makes the cold the body is already in worse by its share; it
    // cannot make a warm hour cold. Added flat, the jacket alone put a
    // clear, dry night past `hurt_at` and chilled a sunny afternoon.
    t += t * draw / 1000;
    // Wet clothes keep half the warmth dry ones would.
    t -= keep * (2000 - p.wet as i32) / 2000;
    if inp.fire {
        t -= ec.fire_warmth as i32;
    }
    if inp.torch {
        t -= ec.torch_warmth as i32;
    }
    t.clamp(0, 1000) as u16
}

/// One second of exposure for one body: wet, then chill, then hurt.
pub fn step(ec: &ExposureContent, inp: &Inputs, p: &mut Player, events: &mut EventQueue) -> Step {
    if !ec.armed() {
        return Step::Quiet;
    }
    let hp_before = p.hp;

    // 1 · Wet. Rain on an uncovered body soaks it; otherwise it dries,
    // faster by a fire. Standing in the sea holds it at least as wet as the
    // water is deep.
    if inp.rain > 0 && !inp.roofed {
        let add = (inp.rain as u32 * ec.wet_rain_per_s as u32 / 1000) as u16;
        p.wet = p.wet.saturating_add(add).min(1000);
    } else {
        let dry = if inp.fire {
            ec.dry_fire_per_s
        } else {
            ec.dry_per_s
        };
        p.wet = p.wet.saturating_sub(dry);
    }
    if ec.soak_depth_cm > 0 && inp.depth_cm > 0 {
        let soak = (inp.depth_cm as u32 * 1000 / ec.soak_depth_cm as u32).min(1000) as u16;
        p.wet = p.wet.max(soak);
    }

    // 2 · Chill, toward where the world is pushing it.
    let target = target_chill(ec, inp, p);
    p.chill = if p.chill < target {
        p.chill.saturating_add(ec.chill_rise_per_s).min(target)
    } else {
        p.chill.saturating_sub(ec.chill_fall_per_s).max(target)
    };

    // 3 · Hurt: nothing at `hurt_at`, the full rate at 1000, exact over
    // the minute through the survival clock's accumulator.
    let mut died = false;
    if p.chill > ec.hurt_at && ec.hurt_at < 1000 {
        let over = (p.chill - ec.hurt_at) as u32;
        let span = (1000 - ec.hurt_at) as u32;
        let dmg = tick_units(&mut p.cold_acc, ec.hurt_hp_per_min as u32 * over, span * 60);
        if dmg > 0 && p.hp > 0 {
            let dmg = dmg.min(u16::MAX as u32) as u16;
            died = crate::combat::hurt_unreduced(p, dmg).died;
        }
    } else {
        // Warm again: the partial point owed does not bank.
        p.cold_acc = 0;
    }
    if died {
        return Step::Died;
    }
    if p.hp != hp_before {
        events.push(EV_HEALTH, p.id, p.hp as u32, p.hp_max as u32);
        return Step::Changed;
    }
    Step::Quiet
}

/// A fresh body comes up dry and warm.
#[inline]
pub fn reset(p: &mut Player) {
    p.wet = 0;
    p.chill = 0;
    p.cold_acc = 0;
}

/// The two meters as the owner's HUD reads them: `(wet %, cold %, hurting)`.
/// Per cent rather than per mille, because that is the resolution a chip
/// shows and every per-mille step would otherwise be a message.
#[inline]
pub fn readout(ec: &ExposureContent, p: &Player) -> (u8, u8, bool) {
    (
        (p.wet / 10).min(100) as u8,
        (p.chill / 10).min(100) as u8,
        ec.armed() && p.chill > ec.hurt_at,
    )
}
