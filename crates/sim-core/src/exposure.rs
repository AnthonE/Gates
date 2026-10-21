//! Wet and cold (weather v0) — the reference's exposure, in the survival
//! clock's shape.
//!
//! Two meters on the body, both per mille:
//!
//! - **Wet.** Rain soaks you unless something is overhead; standing in the
//!   sea soaks you to the depth you stand in; out of it you dry slowly, and
//!   fast beside a fire. The reference's `weather.wetness_rain` is the rate
//!   and a roof is what stops it. Where the air freezes the rain falls as
//!   snow, which wets at a share of that (`wetness_snow`).
//! - **Chill.** Where the body is heading is the night, the rain and the
//!   wind that reach it, and how wet it is — made worse by a piece that
//!   draws cold in, less what it wears (wet clothes keep half their warmth)
//!   and less any fire or flame in hand.
//!   The body moves toward that at a rate, so stepping under a roof for a
//!   second changes nothing and a night in the rain changes a lot. Past
//!   `hurt_at` it costs hp, through the same unreduced funnel starvation
//!   uses: cold is not a hit and armor does not blunt it.
//!
//! **Comfort and regen are Rust's** (`PlayerMetabolism`). A third meter,
//! comfort, follows what the body's surroundings give it: a lit fire half,
//! and each other body at the same fire a quarter more, a sleeper an eighth
//! (`TriggerComfort`). A calm, fed, watered, warm body mends on its own up
//! to 60 % of its hp, and comfort takes that to 100 % and the rate up to
//! seven-fold; mending eats food. By a fire, 50 % comfort heals you to 80.
//!
//! Stepped once a second per body, staggered by slot, on live and downed
//! bodies only — a sleeper's metabolism runs, but an offline player must not
//! freeze to death in a rainstorm they never saw. Every rate is integer
//! arithmetic and the one partial quantity (hp owed) lives in an exact
//! accumulator, `survival::tick_units`, for wall 5's reason.
//!
//! **The Verdigris** (`terrain::fel`) is a fourth meter, Rust's radiation for
//! the corrupted zone: it rises with how deep the body stands in it — faster
//! in a vent's green fire — less what the body wears wards off, and falls
//! outside it. Past `fel_hurt_at` it costs hp through its own accumulator,
//! because a body can be cold and corroded in the same second
//! ([`corrode`]).
//!
//! **Clothing v1 is Rust's rule** (`PlayerMetabolism`): a worn set does not
//! take a fixed amount of cold off, it takes a share of whatever cold there
//! is — the set's summed warmth over `warm_full_pm` (their 400 %), wet
//! clothes keeping half — so a full winter kit takes the bite out of a
//! mountain night without making any night harmless. Teas ([`Brew`]) warm
//! the air the body feels and floor how cold it gets, cool the heat, or
//! ward off the Verdigris, for minutes. A waterproof piece (the hazmat
//! suit) never gets wet, and clothes do not dry below freezing but by a
//! fire. **Bleeding** ([`bleed`]) is Rust's too: a cut, a bite, an arrow or
//! a round leaves a share of what it dealt to run out of the body at a
//! third of a point a second, until a bandage stanches it.
//!
//! `EMPTY` disarms it: content with no `[exposure]` plays the game it did.

use crate::limits::{MAX_ITEM_DEFS, TICK_HZ};
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
    /// The cold's metabolic cost (`NOW.md` §0wx item 1): percent of the
    /// normal food and water drain added at full chill, scaled by chill.
    pub cold_food_pct: u16,
    pub cold_water_pct: u16,
    /// Comfort, per mille: what a lit fire gives a body in its warmth,
    /// what each other body by the same fire adds (a sleeper half), and how
    /// far the meter moves toward its target a second.
    pub fire_comfort: u16,
    pub company_comfort: u16,
    pub comfort_per_s: u16,
    /// Natural regen: hp a minute at an empty stomach and no comfort; what
    /// a full stomach adds and what full comfort adds, per cent; and how
    /// far it mends with no comfort, per cent of max hp (full comfort
    /// mends to full).
    pub regen_hp_per_min: u16,
    pub regen_fed_pct: u16,
    pub regen_comfort_pct: u16,
    pub regen_cap_pct: u16,
    /// What regen asks: more food and more water than these, no more chill
    /// than this, and this many seconds since the body last lost hp.
    pub regen_food_min: u16,
    pub regen_water_min: u16,
    pub regen_chill_max: u16,
    pub regen_calm_s: u16,
    /// What regen costs: percent of the normal food and water drain added
    /// while it runs.
    pub regen_food_pct: u16,
    pub regen_water_pct: u16,
    /// The air (climate v0, `climate.rs`): where it is, how warm.
    pub air: crate::climate::ClimateContent,
    /// Air under this chills, `cold_per_c` per mille a degree. With
    /// `cold_per_c` 0 the old rule stands: the night adds `night_cold`.
    pub cold_below_dc: i16,
    pub cold_per_c: u16,
    /// Air over this heats, `heat_per_c` per mille a degree (0: no heat).
    pub hot_above_dc: i16,
    pub heat_per_c: u16,
    /// Degrees a roof's shade takes off the heat.
    pub shade_dc: i16,
    /// Heat being soaked through takes away — sweat that does not have to
    /// be sweated, a swim in the sea.
    pub wet_cool: u16,
    /// Per mille of a worn piece's warmth that holds heat in when it is
    /// hot (the reference's warm clothes in the desert).
    pub clothes_heat_pm: u16,
    /// Percent of the normal water drain added at full heat.
    pub heat_water_pct: u16,
    /// Past this heat it costs hp, this many a minute at full heat.
    pub heat_hurt_at: u16,
    pub heat_hp_per_min: u16,
    /// Snow (snow v0): per mille of the rain's wetting that snow does — the
    /// reference's `wetness_snow` 0.2 against `wetness_rain` 0.4. The air
    /// at the body decides what falls ([`snow_pm`]).
    pub snow_wet_pm: u16,
    /// Per item index: the chill a worn piece keeps out. Negative draws it
    /// in — a road sign jacket is a sheet of metal — as a per-mille share
    /// of the cold the body is already in, never as cold of its own.
    pub warmth: [i16; MAX_ITEM_DEFS],
    /// The Verdigris ([`corrode`]), per mille of its meter: what a second at
    /// its heart adds, what a second in a vent's fire adds on top, what a
    /// second out of it takes away; past `fel_hurt_at` it costs hp,
    /// `fel_hp_per_min` at a full meter; over `fel_regen_max` a body does not
    /// mend; and no worn set wards off more than `fel_ward_max`.
    pub fel_rise_per_s: u16,
    pub fel_vent_per_s: u16,
    pub fel_fall_per_s: u16,
    pub fel_hurt_at: u16,
    pub fel_hp_per_min: u16,
    pub fel_regen_max: u16,
    pub fel_ward_max: u16,
    /// What a second at the core of a hot spot adds (`blight.rs`: the loot
    /// sites burning once act II begins), per mille — faster than the heart,
    /// so the best crates want a mask.
    pub fel_pocket_per_s: u16,
    /// Per item index: the share of the Verdigris a worn piece wards off,
    /// per mille (`armor.toml` `fel_pct`).
    pub ward: [u16; MAX_ITEM_DEFS],
    /// Per item index: the comfort a placed deployable of that item gives a
    /// body within `furnish_radius_cm` of it, per mille (`deployables.toml`
    /// `comfort`, a rug's). The best one in reach counts, on top of a fire's
    /// ([`comfort_target`]); it needs no fire.
    pub furnish: [u16; MAX_ITEM_DEFS],
    pub furnish_radius_cm: [u16; MAX_ITEM_DEFS],
    /// The summed worn warmth, per mille, that takes all of the cold away
    /// (Rust's `ColdExposure` sum of 400 %, clothing v1). A set at half of
    /// it takes half the chill off. Zero keeps the old flat rule: the worn
    /// warmth is subtracted whole.
    pub warm_full_pm: u16,
    /// Per item index: a worn piece that keeps every drop out (Rust's
    /// Drowned protection: the hazmat suit). A body wearing one never gets
    /// wet, in the rain or the sea.
    pub waterproof: [bool; MAX_ITEM_DEFS],
    /// The air at which clothes dry at `dry_per_s`, deci-degrees (Rust's
    /// drying runs with the air from 0 °C): slower in the cold, twice as
    /// fast at twice the air, and not at all at or under freezing but by a
    /// fire. Zero keeps the old flat rate.
    pub dry_ref_dc: i16,
    /// Bleeding ([`bleed`]): hit points a minute the wound runs out at
    /// (Rust's third of a point a second is 20).
    pub bleed_hp_per_min: u16,
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
        cold_food_pct: 0,
        cold_water_pct: 0,
        fire_comfort: 0,
        company_comfort: 0,
        comfort_per_s: 0,
        regen_hp_per_min: 0,
        regen_fed_pct: 0,
        regen_comfort_pct: 0,
        regen_cap_pct: 0,
        regen_food_min: 0,
        regen_water_min: 0,
        regen_chill_max: 0,
        regen_calm_s: 0,
        regen_food_pct: 0,
        regen_water_pct: 0,
        air: crate::climate::ClimateContent::EMPTY,
        cold_below_dc: 0,
        cold_per_c: 0,
        hot_above_dc: 0,
        heat_per_c: 0,
        shade_dc: 0,
        wet_cool: 0,
        clothes_heat_pm: 0,
        heat_water_pct: 0,
        heat_hurt_at: 1000,
        heat_hp_per_min: 0,
        snow_wet_pm: 1000,
        warmth: [0; MAX_ITEM_DEFS],
        fel_rise_per_s: 0,
        fel_vent_per_s: 0,
        fel_fall_per_s: 0,
        fel_hurt_at: 1000,
        fel_hp_per_min: 0,
        fel_regen_max: 1000,
        fel_ward_max: 0,
        fel_pocket_per_s: 0,
        ward: [0; MAX_ITEM_DEFS],
        furnish: [0; MAX_ITEM_DEFS],
        furnish_radius_cm: [0; MAX_ITEM_DEFS],
        warm_full_pm: 0,
        waterproof: [false; MAX_ITEM_DEFS],
        dry_ref_dc: 0,
        bleed_hp_per_min: 0,
    };

    /// Is exposure armed? A zero chill rate means nothing ever moves.
    #[inline]
    pub fn armed(&self) -> bool {
        self.chill_rise_per_s > 0 || self.fel_rise_per_s > 0
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

/// How many kinds of tea a body can have in it at once, one of each.
pub const BREW_KINDS: usize = 3;
/// A warming tea: `amount` deci-degrees warmer for the chill, and the body
/// never chills past what `floor_dc` of still, dry air would give it.
pub const BREW_WARM: u8 = 0;
/// A cooling tea: `amount` deci-degrees cooler for the heat.
pub const BREW_COOL: u8 = 1;
/// A warding tea: `amount` per mille of the Verdigris kept out, on top of
/// what is worn ([`BREW_WARD_CEIL_PM`]).
pub const BREW_WARD: u8 = 2;
/// `Brew::floor_dc` for a tea with no floor.
pub const NO_FLOOR: i16 = i16::MIN;
/// The most worn pieces and a tea together keep out of the Verdigris: the
/// heart is never free.
pub const BREW_WARD_CEIL_PM: u32 = 950;

/// One tea in the body (Rust's tea modifiers): what it gives and the
/// seconds it has left. Drinking another of the same kind replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brew {
    pub amount: u16,
    pub floor_dc: i16,
    pub left_s: u16,
}

impl Brew {
    pub const NONE: Self = Self {
        amount: 0,
        floor_dc: NO_FLOOR,
        left_s: 0,
    };

    pub fn to_le_bytes(&self) -> [u8; 6] {
        let mut b = [0u8; 6];
        b[0..2].copy_from_slice(&self.amount.to_le_bytes());
        b[2..4].copy_from_slice(&self.floor_dc.to_le_bytes());
        b[4..6].copy_from_slice(&self.left_s.to_le_bytes());
        b
    }

    pub fn from_le_bytes(b: [u8; 6]) -> Self {
        Self {
            amount: u16::from_le_bytes([b[0], b[1]]),
            floor_dc: i16::from_le_bytes([b[2], b[3]]),
            left_s: u16::from_le_bytes([b[4], b[5]]),
        }
    }
}

impl Default for Brew {
    fn default() -> Self {
        Self::NONE
    }
}

/// The tea of `kind` in this body, if one is working.
#[inline]
pub fn brewing(p: &Player, kind: u8) -> Option<Brew> {
    p.brew.get(kind as usize).copied().filter(|b| b.left_s > 0)
}

/// One second off every tea in the body; a spent one is gone.
pub fn steep(p: &mut Player) {
    for b in p.brew.iter_mut() {
        if b.left_s > 0 {
            b.left_s -= 1;
            if b.left_s == 0 {
                *b = Brew::NONE;
            }
        }
    }
}

/// Is a waterproof piece worn (the hazmat suit)?
fn worn_waterproof(ec: &ExposureContent, p: &Player) -> bool {
    p.worn
        .iter()
        .any(|s| s.count > 0 && (s.item as usize) < MAX_ITEM_DEFS && ec.waterproof[s.item as usize])
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
    /// The comfort the surroundings give, per mille: a lit fire, company
    /// at it, and the best rug in reach (`World::exposure_inputs`).
    pub comfort: u16,
    /// The meters' ceilings, for the stomach's share of regen and its bill.
    pub max_food: u16,
    pub max_water: u16,
    /// The air where the body stands, deci-degrees (`climate::air_dc`).
    pub air_dc: i16,
    /// How deep in the Verdigris the body stands, per mille (`terrain::fel`;
    /// 0 on the beach), and whether it stands in a vent's fire
    /// (`terrain::fel_vent_near`).
    pub fel_pm: u16,
    pub vent: bool,
    /// How deep in a hot spot the body stands, per mille (`blight::Depth`).
    pub hot_pm: u16,
}

/// What is worn, split in two: the warmth that keeps cold out, and the draw
/// that lets it in (a road sign jacket is a sheet of metal). Per mille.
fn worn_split(ec: &ExposureContent, p: &Player) -> (i32, i32) {
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
    (keep, draw)
}

/// How much the rain, the wind and the wet chill at this air, per mille
/// (climate v0): all of it at `cold_below_dc` and under, none at
/// `hot_above_dc` and over, a straight line between. Without the air armed,
/// all of it, as before.
#[inline]
pub fn feel_pm(ec: &ExposureContent, inp: &Inputs) -> i32 {
    if ec.cold_per_c == 0 || ec.hot_above_dc <= ec.cold_below_dc {
        return 1000;
    }
    let span = ec.hot_above_dc as i32 - ec.cold_below_dc as i32;
    (ec.hot_above_dc as i32 - inp.air_dc as i32).clamp(0, span) * 1000 / span
}

/// How much of what falls on this body is snow, per mille (snow v0): the
/// air's say alone (`climate::snow_pm`). It is the air the owner's readout
/// carries, and the client draws its sky off that reading by the same line,
/// so the body and the sky cannot disagree about what falls — whatever the
/// cold or the heat make of the air. Content keeps the air real: an armed
/// `[exposure]` needs a `[climate]` (`content::validate`).
#[inline]
pub fn snow_pm(inp: &Inputs) -> u16 {
    crate::climate::snow_pm(inp.air_dc)
}

/// The chill a body is heading for, per mille — the whole rule in one
/// place so the tests and the step read the same sentence.
pub fn target_chill(ec: &ExposureContent, inp: &Inputs, p: &Player) -> u16 {
    // A warming tea warms the air the body feels (Rust's Warming +5/+10).
    let warm = brewing(p, BREW_WARM);
    let mut felt = *inp;
    if let Some(b) = warm {
        felt.air_dc = inp
            .air_dc
            .saturating_add(b.amount.min(i16::MAX as u16) as i16);
    }
    let inp = &felt;
    let mut t: i32 = 0;
    if ec.cold_per_c > 0 {
        // The air (climate v0): every degree under `cold_below_dc`.
        let under = (ec.cold_below_dc as i32 - inp.air_dc as i32).max(0);
        t += under * ec.cold_per_c as i32 / 10;
    } else if inp.night {
        t += ec.night_cold as i32;
    }
    let mut extra = p.wet as i32 * ec.wet_cold as i32 / 1000;
    if !inp.roofed {
        extra +=
            (inp.rain as i32 * ec.rain_cold as i32 + inp.wind as i32 * ec.wind_cold as i32) / 1000;
    }
    t += extra * feel_pm(ec, inp) / 1000;
    let (keep, draw) = worn_split(ec, p);
    // Wet clothes keep half the warmth dry ones would.
    let keep = keep * (2000 - p.wet as i32) / 2000;
    if ec.warm_full_pm > 0 {
        // Rust's rule (clothing v1): the set takes its share of the cold
        // there is, all of it at `warm_full_pm`; a draw (a sheet of metal)
        // makes it worse by its share. Never a fixed amount, so no kit
        // makes the deep cold harmless.
        let e = ((keep - draw) * 1000 / ec.warm_full_pm as i32).clamp(-1000, 1000);
        t -= t * e / 1000;
    } else {
        // A draw makes the cold the body is already in worse by its share; it
        // cannot make a warm hour cold. Added flat, the jacket alone put a
        // clear, dry night past `hurt_at` and chilled a sunny afternoon.
        t += t * draw / 1000;
        t -= keep;
    }
    if inp.fire {
        t -= ec.fire_warmth as i32;
    }
    if inp.torch {
        t -= ec.torch_warmth as i32;
    }
    // A warming tea's floor (Rust's Min Temperature): never colder than
    // still, dry air at `floor_dc` would make a bare body.
    if let Some(b) = warm.filter(|b| b.floor_dc != NO_FLOOR && ec.cold_per_c > 0) {
        let cap = (ec.cold_below_dc as i32 - b.floor_dc as i32).max(0) * ec.cold_per_c as i32 / 10;
        t = t.min(cap);
    }
    t.clamp(0, 1000) as u16
}

/// The heat a body is in, per mille (climate v0): every degree of air over
/// `hot_above_dc`, less a roof's shade, more for warm clothes, less for
/// being wet. No meter of its own: the reference's body temperature
/// follows the air within seconds, so ours is the air's, read each second.
pub fn target_heat(ec: &ExposureContent, inp: &Inputs, p: &Player) -> u16 {
    if ec.heat_per_c == 0 {
        return 0;
    }
    let mut air = inp.air_dc as i32;
    if inp.roofed {
        air -= ec.shade_dc as i32;
    }
    // A cooling tea (Rust's Cooling +10/+15/+20).
    if let Some(b) = brewing(p, BREW_COOL) {
        air -= b.amount as i32;
    }
    let over = (air - ec.hot_above_dc as i32).max(0);
    if over == 0 {
        return 0;
    }
    let mut t = over * ec.heat_per_c as i32 / 10;
    let (keep, _) = worn_split(ec, p);
    if ec.warm_full_pm > 0 {
        // Rust's: a warm set holds up to its full share of extra heat in.
        let e = (keep * 1000 / ec.warm_full_pm as i32).clamp(0, 1000);
        t += e * ec.clothes_heat_pm as i32 / 1000;
    } else {
        t += keep * ec.clothes_heat_pm as i32 / 1000;
    }
    t -= p.wet as i32 * ec.wet_cool as i32 / 1000;
    t.clamp(0, 1000) as u16
}

/// Does a lit fire's warmth reach a body at `(x, z)` with its feet at
/// `feet`? The fire is a deployable at `(cx, cz, level)` moved by `pose`,
/// standing on its built floor or else the ground; its warmth reaches
/// `heat_radius_cm` in the plane, and within a storey.
///
/// One sentence for two readers: the sim's `World::near_fire`, and the
/// client confirming a fire on its WET and COLD chips (`NOW.md` §0wx item
/// 3, `ClientCore::near_fire`), so the HUD says "by a fire" exactly where
/// the sim warms. Whether the fire is lit, and burns at all, is the
/// caller's to ask first.
#[allow(clippy::too_many_arguments)]
pub fn fire_reaches(
    seed: u64,
    haven: &crate::terrain::Haven,
    cols: &crate::collide::ColIndex,
    heat_radius_cm: u16,
    (cx, cz, level, pose): (u16, u16, u8, crate::footprint::Pose),
    x: f32,
    z: f32,
    feet: f32,
) -> bool {
    let r = heat_radius_cm as f32 * 0.01;
    if r <= 0.0 {
        return false;
    }
    let (bx, bz) = crate::footprint::centre(cx, cz, pose);
    let (dx, dz) = (bx - x, bz - z);
    if dx * dx + dz * dz > r * r {
        return false;
    }
    let y = crate::deploy::built_floor(seed, haven, cols, cx, cz, level, bx, bz)
        .unwrap_or_else(|| crate::terrain::ground(seed, haven, bx, bz));
    let dy = y - feet;
    dy * dy < 9.0
}

/// One second of exposure for one body: wet, then chill, then hurt.
pub fn step(ec: &ExposureContent, inp: &Inputs, p: &mut Player, events: &mut EventQueue) -> Step {
    if !ec.armed() {
        return Step::Quiet;
    }
    let hp_before = p.hp;

    // 1 · Wet. Rain on an uncovered body soaks it — snow at its share of
    // the rate, sleet at a share of each; otherwise it dries, faster by a
    // fire. Standing in the sea holds it at least as wet as the water is
    // deep.
    let proof = worn_waterproof(ec, p);
    if inp.rain > 0 && !inp.roofed && !proof {
        let snow = snow_pm(inp) as u64;
        let keep = 1000 - snow * (1000 - ec.snow_wet_pm.min(1000) as u64) / 1000;
        let add = inp.rain as u64 * ec.wet_rain_per_s as u64 * keep / 1_000_000;
        p.wet = p.wet.saturating_add(add.min(1000) as u16).min(1000);
    } else {
        let dry = if inp.fire {
            ec.dry_fire_per_s
        } else if ec.dry_ref_dc > 0 {
            // Rust's drying runs with the air: none at or under freezing,
            // the full rate at `dry_ref_dc`, rounded up so a cool day
            // still dries.
            let air = (inp.air_dc as i32).clamp(0, 2 * ec.dry_ref_dc as i32);
            let r = ec.dry_ref_dc as i32;
            ((ec.dry_per_s as i32 * air + r - 1) / r) as u16
        } else {
            ec.dry_per_s
        };
        p.wet = p.wet.saturating_sub(dry);
    }
    if ec.soak_depth_cm > 0 && inp.depth_cm > 0 && !proof {
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

    // 2b · Heat, from the air (climate v0), and the air for the HUD.
    p.air_dc = inp.air_dc;
    p.heat = target_heat(ec, inp, p);

    // 3 · Comfort follows its target (Rust's `comfort.MoveTowards`), and
    // the calm clock runs (`combat::debit` zeroes it on any hp lost). A
    // body the heat reaches has none, as Rust's too-hot body has none.
    let want = if p.heat > 0 { 0 } else { inp.comfort };
    p.comfort = if p.comfort < want {
        p.comfort.saturating_add(ec.comfort_per_s).min(want)
    } else {
        p.comfort.saturating_sub(ec.comfort_per_s).max(want)
    };
    p.calm = p.calm.saturating_add(1);

    // 4 · Hurt: nothing at `hurt_at`, the full rate at 1000, exact over
    // the minute through the survival clock's accumulator. The heat shares
    // the cold's accumulator: a body is not past both at once.
    let mut died = false;
    if p.chill > ec.hurt_at && ec.hurt_at < 1000 {
        let over = (p.chill - ec.hurt_at) as u32;
        let span = (1000 - ec.hurt_at) as u32;
        let dmg = tick_units(&mut p.cold_acc, ec.hurt_hp_per_min as u32 * over, span * 60);
        if dmg > 0 && p.hp > 0 {
            let dmg = dmg.min(u16::MAX as u32) as u16;
            died = crate::combat::hurt_unreduced(p, dmg).died;
        }
    } else if p.heat > ec.heat_hurt_at && ec.heat_hurt_at < 1000 {
        let over = (p.heat - ec.heat_hurt_at) as u32;
        let span = (1000 - ec.heat_hurt_at) as u32;
        let dmg = tick_units(&mut p.cold_acc, ec.heat_hp_per_min as u32 * over, span * 60);
        if dmg > 0 && p.hp > 0 {
            let dmg = dmg.min(u16::MAX as u32) as u16;
            died = crate::combat::hurt_unreduced(p, dmg).died;
        }
    } else if regens(ec, p) {
        // Regen (Rust's metabolism): the cold's accumulator carries it,
        // because the two never run at once — regen wants the chill under
        // `regen_chill_max`, which the bake holds under `hurt_at`.
        let full = (p.food as u32 * 1000 / inp.max_food.max(1) as u32).min(1000);
        let fed = 100 + ec.regen_fed_pct as u32 * full / 1000;
        let comfy = 1000 + ec.regen_comfort_pct as u32 * p.comfort as u32 / 100;
        let give = tick_units(
            &mut p.cold_acc,
            ec.regen_hp_per_min as u32 * fed * comfy,
            REGEN_DEN,
        );
        if give > 0 {
            p.hp =
                p.hp.saturating_add(give.min(u16::MAX as u32) as u16)
                    .min(regen_cap(ec, p));
        }
        // The bill: a share of one second's drain, `burn`'s shape.
        let bill = |max: u16, pct: u16| max as u32 * TICK_HZ * pct as u32 / 100;
        p.food_acc = p
            .food_acc
            .saturating_add(bill(inp.max_food, ec.regen_food_pct));
        p.water_acc = p
            .water_acc
            .saturating_add(bill(inp.max_water, ec.regen_water_pct));
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

/// What the worn set wards off of the Verdigris, per mille, held to
/// `fel_ward_max`.
pub(crate) fn worn_ward(ec: &ExposureContent, p: &Player) -> u32 {
    let mut ward = 0u32;
    for s in p.worn.iter() {
        if s.count == 0 || s.item as usize >= MAX_ITEM_DEFS {
            continue;
        }
        ward += ec.ward[s.item as usize] as u32;
    }
    let worn = ward.min(ec.fel_ward_max as u32).min(1000);
    // A warding tea on top of the clothes (Rust's anti-rad tea), never to
    // a whole ward.
    let tea = brewing(p, BREW_WARD).map_or(0, |b| b.amount as u32);
    (worn + tea).min(BREW_WARD_CEIL_PM.max(worn))
}

/// A turned animal's bite (`mob::Bite::fel`): `fel_pm` of the meter at
/// once, less what the worn set wards off. Armor does not blunt it. Nothing
/// on a world whose meter is off, which would never drain it again.
pub fn infect(ec: &ExposureContent, p: &mut Player, fel_pm: u16) {
    if ec.fel_rise_per_s == 0 || fel_pm == 0 {
        return;
    }
    let gain = (u32::from(fel_pm) * (1000 - worn_ward(ec, p))).div_ceil(1000);
    p.fel = (u32::from(p.fel) + gain).min(1000) as u16;
}

/// One second of the Verdigris for one body: its meter rises by how deep it
/// stands (rounded up, so the edge still creeps), plus a vent's fire, plus a
/// hot spot's burn at its own rate, less what it wears wards off; anywhere the Verdigris does not reach it falls.
/// Past `fel_hurt_at` it costs hp, nothing at the line and
/// `fel_hp_per_min` at a full meter, exact over the minute through its own
/// accumulator. Not a hit: armor does not blunt it, as it does not the cold.
pub fn corrode(
    ec: &ExposureContent,
    inp: &Inputs,
    p: &mut Player,
    events: &mut EventQueue,
) -> Step {
    if ec.fel_rise_per_s == 0 && p.fel == 0 {
        return Step::Quiet;
    }
    let hp_before = p.hp;
    let mut gain = (inp.fel_pm as u32 * ec.fel_rise_per_s as u32).div_ceil(1000);
    if inp.vent {
        gain += ec.fel_vent_per_s as u32;
    }
    gain += (inp.hot_pm as u32 * ec.fel_pocket_per_s as u32).div_ceil(1000);
    if gain > 0 {
        let gain = (gain * (1000 - worn_ward(ec, p))).div_ceil(1000);
        p.fel = (p.fel as u32 + gain).min(1000) as u16;
    } else {
        p.fel = p.fel.saturating_sub(ec.fel_fall_per_s);
    }
    let mut died = false;
    if p.fel > ec.fel_hurt_at && ec.fel_hurt_at < 1000 {
        let over = (p.fel - ec.fel_hurt_at) as u32;
        let span = (1000 - ec.fel_hurt_at) as u32;
        let dmg = tick_units(&mut p.fel_acc, ec.fel_hp_per_min as u32 * over, span * 60);
        if dmg > 0 && p.hp > 0 {
            let dmg = dmg.min(u16::MAX as u32) as u16;
            died = crate::combat::hurt_unreduced(p, dmg).died;
        }
    } else {
        p.fel_acc = 0;
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

/// One second of bleeding for one body (Rust's): the wound runs out at
/// `bleed_hp_per_min` until what it was owed is spent, and every point it
/// runs is a point of hp. Not a hit: armor does not blunt it. Its own
/// accumulator, because a body can bleed and freeze in the same second.
pub fn bleed(ec: &ExposureContent, p: &mut Player, events: &mut EventQueue) -> Step {
    if p.bleed == 0 || ec.bleed_hp_per_min == 0 {
        p.bleed_acc = 0;
        return Step::Quiet;
    }
    let owed = tick_units(&mut p.bleed_acc, ec.bleed_hp_per_min as u32, 60);
    let dmg = owed.min(p.bleed as u32) as u16;
    p.bleed -= dmg;
    if p.bleed == 0 {
        p.bleed_acc = 0;
    }
    if dmg == 0 || p.hp == 0 {
        return Step::Quiet;
    }
    if crate::combat::hurt_unreduced(p, dmg).died {
        return Step::Died;
    }
    events.push(EV_HEALTH, p.id, p.hp as u32, p.hp_max as u32);
    Step::Changed
}

/// A blow that cuts (a swing, a bite, an arrow, a round) opens a wound: a
/// share of the hp it dealt still to run out (Rust's 0.2 × damage), held
/// to `max`.
#[inline]
pub fn cut(p: &mut Player, dealt: u16, pct: u8, max: u16) {
    if pct == 0 || dealt == 0 {
        return;
    }
    let add = (dealt as u32 * pct as u32).div_ceil(100);
    p.bleed = (p.bleed as u32 + add).min(max as u32) as u16;
}

/// A bandage (or a medkit) stanches `by` points of the wound.
#[inline]
pub fn stanch(p: &mut Player, by: u16) {
    p.bleed = p.bleed.saturating_sub(by);
    if p.bleed == 0 {
        p.bleed_acc = 0;
    }
}

/// The regen rate's denominator: a minute, times the stomach's per cent
/// and comfort's per mille scales in `step`.
const REGEN_DEN: u32 = 60 * 100 * 1000;

/// How far regen mends this body now: `regen_cap_pct` of max hp with no
/// comfort, all of it at full comfort (Rust's `0.6 + 0.4 × comfort`).
pub fn regen_cap(ec: &ExposureContent, p: &Player) -> u16 {
    let base = (ec.regen_cap_pct as u32 * 10).min(1000);
    let pm = base + (1000 - base) * p.comfort.min(1000) as u32 / 1000;
    (p.hp_max as u32 * pm / 1000) as u16
}

/// Does this body mend on its own this second? Rust's gates: fed past
/// `regen_food_min`, watered past `regen_water_min`, warm enough, calm for
/// `regen_calm_s`, standing, and under its cap.
pub fn regens(ec: &ExposureContent, p: &Player) -> bool {
    ec.regen_hp_per_min > 0
        && p.food > ec.regen_food_min
        && p.water > ec.regen_water_min
        && p.chill <= ec.regen_chill_max
        && p.fel <= ec.fel_regen_max
        && p.calm >= ec.regen_calm_s
        && !p.wounded
        && p.bleed == 0
        && p.hp > 0
        && p.hp < regen_cap(ec, p)
}

/// The comfort a body's surroundings give it, per mille (Rust's
/// `TriggerComfort.CalculateComfort`): `fire_comfort` in a lit fire's
/// warmth, and `company_comfort` for each other body there, half for a
/// sleeper; plus `furnish`, the best comfort deployable in reach (a rug's,
/// [`ExposureContent::furnish`]), which needs no fire. Capped at all of it.
pub fn comfort_target(
    ec: &ExposureContent,
    fire: bool,
    awake: u32,
    asleep: u32,
    furnish: u16,
) -> u16 {
    let at_fire = if fire {
        ec.fire_comfort as u32
            + ec.company_comfort as u32 * awake
            + ec.company_comfort as u32 * asleep / 2
    } else {
        0
    };
    (at_fire + furnish as u32).min(1000) as u16
}

/// One second of the cold's and the heat's metabolic cost (`NOW.md` §0wx
/// item 1): a chilled body burns food and water faster —
/// `cold_food_pct` / `cold_water_pct` percent of the normal drain on top
/// at full chill, scaled by the chill — and a hot one sweats water away,
/// `heat_water_pct` at full heat. Paid into the survival clock's own
/// accumulators as that share of one second's drain (`max × TICK_HZ` is a
/// second of the clock's numerator), so the clock takes whole units and
/// stays exact.
pub fn burn(ec: &ExposureContent, max_food: u16, max_water: u16, p: &mut Player) {
    if !ec.armed() || (p.chill == 0 && p.heat == 0) {
        return;
    }
    let share = |max: u16, pct: u16, by: u16| -> u32 {
        let v = max as u64 * TICK_HZ as u64 * pct as u64 * by as u64 / 100_000;
        v.min(u32::MAX as u64) as u32
    };
    let food = share(max_food, ec.cold_food_pct, p.chill);
    let water = share(max_water, ec.cold_water_pct, p.chill).saturating_add(share(
        max_water,
        ec.heat_water_pct,
        p.heat,
    ));
    p.food_acc = p.food_acc.saturating_add(food);
    p.water_acc = p.water_acc.saturating_add(water);
}

/// A fresh body comes up dry and warm.
#[inline]
pub fn reset(p: &mut Player) {
    p.wet = 0;
    p.chill = 0;
    p.heat = 0;
    p.cold_acc = 0;
    p.comfort = 0;
    p.fel = 0;
    p.fel_acc = 0;
    p.bleed = 0;
    p.bleed_acc = 0;
    p.brew = [Brew::NONE; BREW_KINDS];
}

/// What the owner's HUD reads: wet, cold and heat in per cent, whether
/// either is hurting, comfort in per cent, and the air in whole degrees.
/// Per cent rather than per mille, because that is the resolution a chip
/// shows and every per-mille step would otherwise be a message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Readout {
    pub wet_pct: u8,
    pub cold_pct: u8,
    pub heat_pct: u8,
    pub hurting: bool,
    pub comfort_pct: u8,
    pub air_c: i8,
    /// The Verdigris' meter, per cent, and whether it is costing hp.
    pub fel_pct: u8,
    pub fel_hurting: bool,
    /// Hit points still to bleed out, capped at a byte (clothing v1).
    pub bleed: u8,
    /// Minutes left on each tea, rounded up, by `BREW_*`; 0 for none.
    pub brew_min: [u8; BREW_KINDS],
}

#[inline]
pub fn readout(ec: &ExposureContent, p: &Player) -> Readout {
    Readout {
        wet_pct: (p.wet / 10).min(100) as u8,
        cold_pct: (p.chill / 10).min(100) as u8,
        heat_pct: (p.heat / 10).min(100) as u8,
        hurting: ec.armed() && (p.chill > ec.hurt_at || p.heat > ec.heat_hurt_at),
        comfort_pct: (p.comfort / 10).min(100) as u8,
        air_c: crate::climate::whole_c(p.air_dc),
        fel_pct: (p.fel / 10).min(100) as u8,
        fel_hurting: p.fel > ec.fel_hurt_at && ec.fel_hurt_at < 1000,
        bleed: p.bleed.min(u8::MAX as u16) as u8,
        brew_min: p
            .brew
            .map(|b| b.left_s.div_ceil(60).min(u8::MAX as u16) as u8),
    }
}
