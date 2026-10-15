//! The Verdigris spreads (`ARC.md` §2): the green that `terrain::fel` lays
//! down on wipe day creeps outward every hour of the wipe, surges each time
//! the island's fel tier climbs (`works::fel_tier_of`, the trigger that
//! already hardens the animals), erupts in outbreaks at inland spots this
//! wipe's salt picks, and burns hot at the loot sites once act II begins.
//!
//! **A layer over the heart, never a change to it.** Worldgen keeps reading
//! the static `terrain::fel` (scatter, ore, spires, the mob survey), so no
//! golden moves for the terrain. What follows the layer is what the meter
//! reads (`World::exposure_inputs`), what turns an animal, and what the
//! client draws: the ground, the sky at the eye, the map, the dying trees.
//!
//! **Both sides, one function.** The client never opens `content/`, so every
//! knob that shapes or paces the spread is a const here, and the client
//! draws from the same tick, tier and salt the sim reads. Only the meter's
//! response (how fast each kind of green fills it) is server content.
//!
//! **Wipe day is exactly today's heart.** At hour 0 and tier A the front sits
//! on the heart's own outer edge with the heart's own fade, so the creep is
//! 0.7 × the static profile at a potential that is never lower, outbreaks
//! have not begun, and every hot spot is cold: [`depth`] returns
//! `terrain::fel` bit for bit, held at 0 (`tests/blight.rs`).
//!
//! Pure in its inputs, no allocation, no trig: `+ − × ÷ sqrt min max` and the
//! quintic `fade`, like the heart itself.

use crate::fmath::fade;
use crate::limits::TICK_HZ;
use crate::terrain::{
    self, Haven, BEACH_MAX_H, CONTINENT_RADIUS, FEL_HEART, FEL_R, FEL_WANDER, ISLAND_SIZE,
    MINOR_SITES,
};

/// Ticks in one hour of the arc clock (`World::tick`).
pub const TICKS_PER_HOUR: u64 = 3_600 * TICK_HZ as u64;
/// Ticks in a thousandth of an hour: the clock's unit here, so a week of
/// ticks (~18 M, past f32's exact integers) reaches the floats as a count
/// that is (168 000 at a week).
const TICKS_PER_MILLIHOUR: u64 = TICKS_PER_HOUR / 1_000;

// --- The creep: the heart's front, in units of `CONTINENT_RADIUS` --------

/// Where the front stands at hour 0: the heart's own outer edge. **(knob)**
pub const F0: f32 = FEL_R.1;
/// The front's soft edge: the heart's own fade width, which is what keeps
/// hour 0 equal to the heart.
pub const BAND: f32 = FEL_R.1 - FEL_R.0;
/// How deep the crept ground gets: the old heart stays the worst place.
/// **(knob)**
pub const CREEP_DEPTH: f32 = 0.7;
/// The hour the creep starts moving, and how long it grows. **(knob)**
pub const START_H: f32 = 12.0;
pub const GROW_H: f32 = 156.0;
/// How far the front walks an hour while it grows. **(knob)**
pub const RATE_PER_H: f32 = 0.0017;
/// What each fel tier adds to the front at once: the surges. **(knob)**
pub const SURGE: [f32; 3] = [0.0, 0.06, 0.12];
/// High ground holds out: the potential added at `RIDGE_HI_M` and above,
/// rising from nothing at `RIDGE_LO_M`. **(knob)**
pub const RIDGE: f32 = 0.10;
pub const RIDGE_LO_M: f32 = 15.0;
pub const RIDGE_HI_M: f32 = 90.0;
/// The crept and erupted green fades out over this much height above the
/// beach, so the coast stays clean. **(knob)**
pub const COAST_M: f32 = 4.0;

// --- THE GATE holds ---------------------------------------------------------

/// Inside this, metres from the town's centre, nothing that spreads ever
/// reaches; past `WARD_OUT_M` the ward is gone. **(knob)**
pub const WARD_IN_M: f32 = 300.0;
pub const WARD_OUT_M: f32 = 420.0;
/// The ward's potential: far past any front, so it is a wall, not a slope.
pub const WARD: f32 = 2.0;

// --- Outbreaks: this wipe's eruptions -------------------------------------

/// Outbreaks a wipe. **(knob)**
pub const OUTBREAKS: usize = 5;
/// When each begins, hours, before its jitter. **(knob)**
pub const OUTBREAK_H: [f32; OUTBREAKS] = [30.0, 54.0, 78.0, 108.0, 132.0];
pub const OUTBREAK_JITTER_H: f32 = 6.0;
/// How long one takes to reach its full radius, hours. **(knob)**
pub const OUTBREAK_GROW_H: f32 = 36.0;
/// Its full radius, metres, drawn per site. **(knob)**
pub const OUTBREAK_R_M: (f32, f32) = (180.0, 260.0);
/// How deep its core gets. **(knob)**
pub const OUTBREAK_DEPTH: f32 = 0.8;
/// How far its edge wanders, a share of its radius.
pub const OUTBREAK_WANDER: f32 = 0.35;
/// Where a site may stand: this band of the island, as a share of
/// `CONTINENT_RADIUS` from the centre, inland of any coast's wobble and
/// outside the central plain's heart.
pub const OUTBREAK_RING: (f32, f32) = (0.25, 0.65);
/// Clearances a site keeps, metres: from the haven pad, the lesser sites and
/// the ziggurat (they burn on their own), and from each other.
const OUTBREAK_CLEAR_SITE_M: f32 = 300.0;
const OUTBREAK_CLEAR_EACH_M: f32 = 2.5 * OUTBREAK_R_M.1;
/// Draws per site before it gives up and the wipe has one fewer.
const OUTBREAK_TRIES: i32 = 64;
const CH_BLIGHT_SITE: u32 = 0x4246_4C54; // "BFLT"

// --- Hot spots: the loot burns ---------------------------------------------

/// A hot spot: radius, metres, and its depth in each fel tier. Cold on wipe
/// day, so hour 0 is the heart alone. **(knob)**
pub const HOT_HAVEN: (f32, [f32; 3]) = (65.0, [0.0, 0.45, 0.75]);
pub const HOT_ZIGGURAT: (f32, [f32; 3]) = (90.0, [0.0, 0.35, 0.60]);
pub const HOT_MINOR: (f32, [f32; 3]) = (45.0, [0.0, 0.27, 0.45]);
/// A hot spot's edge: its fade and its wander, shares of its radius.
pub const HOT_BAND: f32 = 0.4;
pub const HOT_WANDER: f32 = 0.15;
/// A disc's soft edge, a share of its radius.
pub const OUTBREAK_BAND: f32 = 0.3;

/// An animal standing this deep in the green turns (`mob::Mob::turned`), and
/// one turned recovers below [`TURN_OFF`]: the gap keeps an animal on the
/// front's edge from flipping every think.
pub const TURN_ON: f32 = 0.55;
pub const TURN_OFF: f32 = 0.45;

/// Whether an animal that `was` turned is turned at `depth`.
pub fn turns(was: bool, depth: f32) -> bool {
    if was {
        depth > TURN_OFF
    } else {
        depth >= TURN_ON
    }
}

/// Every disc a moment can hold: the outbreaks, then the hot spots.
pub const MAX_DISCS: usize = OUTBREAKS + 2 + MINOR_SITES;

/// The island's centre, metres.
const CENTRE: f32 = ISLAND_SIZE * 0.5;

/// One of this wipe's outbreak sites.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Site {
    pub x: f32,
    pub z: f32,
    /// The hour it begins.
    pub start_h: f32,
    /// Its full radius, metres.
    pub r: f32,
}

/// This wipe's outbreaks, drawn once from the seed and the blight salt.
/// Derived state: rebuilt whenever the salt is set, never saved or hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plan {
    pub sites: [Site; OUTBREAKS],
    pub n: u8,
}

impl Plan {
    /// Place this wipe's outbreaks by hashes alone, no terrain taps, so a
    /// fresh `World` pays a few hundred hashes for it. A draw is rejected —
    /// never clipped — inside the town's ward, near the loot sites, inside
    /// the heart's reach or near another site; a site that runs out of
    /// draws is left out.
    pub fn new(seed: u64, salt: u32, haven: &Haven) -> Self {
        let mut p = Plan::default();
        let key = seed ^ (salt as u64).rotate_left(29) ^ 0x5645_5244_4947_5249; // "VERDIGRI"
        let (tx, tz) = ward_centre(haven);
        let ring_lo = OUTBREAK_RING.0 * CONTINENT_RADIUS;
        let ring_hi = OUTBREAK_RING.1 * CONTINENT_RADIUS;
        for (i, start_h) in OUTBREAK_H.iter().enumerate() {
            let mut t = 0;
            while t < OUTBREAK_TRIES {
                let h = crate::rng::cell_hash(key, i as i32, t, CH_BLIGHT_SITE);
                t += 1;
                let u = unit(h);
                let v = unit(h >> 16);
                let x = CENTRE + (u * 2.0 - 1.0) * ring_hi;
                let z = CENTRE + (v * 2.0 - 1.0) * ring_hi;
                let r = OUTBREAK_R_M.0 + (OUTBREAK_R_M.1 - OUTBREAK_R_M.0) * unit(h >> 32);
                let from_centre = dist(x, z, CENTRE, CENTRE);
                if from_centre < ring_lo || from_centre > ring_hi {
                    continue;
                }
                if dist(x, z, tx, tz) < WARD_OUT_M + r * (1.0 + OUTBREAK_WANDER) {
                    continue;
                }
                if dist(x, z, FEL_HEART.0, FEL_HEART.1)
                    < terrain::FEL_REACH_M + r * (1.0 + OUTBREAK_WANDER)
                {
                    continue;
                }
                if near_site(haven, x, z, OUTBREAK_CLEAR_SITE_M + r) {
                    continue;
                }
                let mut crowded = false;
                for s in p.sites.iter().take(p.n as usize) {
                    if dist(x, z, s.x, s.z) < OUTBREAK_CLEAR_EACH_M {
                        crowded = true;
                    }
                }
                if crowded {
                    continue;
                }
                let jitter = (unit(h >> 48) * 2.0 - 1.0) * OUTBREAK_JITTER_H;
                p.sites[p.n as usize] = Site {
                    x,
                    z,
                    start_h: start_h + jitter,
                    r,
                };
                p.n += 1;
                break;
            }
        }
        p
    }
}

/// One disc of green at a moment: an outbreak growing, or a hot spot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Disc {
    pub x: f32,
    pub z: f32,
    /// Its radius now, metres.
    pub r: f32,
    /// How deep its core is now.
    pub depth: f32,
    /// A hot spot (the loot burning) rather than an outbreak: it fills the
    /// meter at its own rate and is not faded at the coast.
    pub hot: bool,
}

/// The spread at one moment: the heart's front and every disc alive.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Now {
    /// The front, in units of `CONTINENT_RADIUS` from the heart.
    pub front: f32,
    /// The town's ward centre, metres.
    pub ward: (f32, f32),
    pub discs: [Disc; MAX_DISCS],
    pub n: u8,
}

/// How deep a point stands, split by what fills the meter how.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Depth {
    /// The heart, the creep and the outbreaks, 0..1.
    pub fel: f32,
    /// The hot spots, 0..1.
    pub hot: f32,
}

impl Depth {
    /// The deeper of the two: what the eye sees and an animal turns in.
    pub fn any(&self) -> f32 {
        self.fel.max(self.hot)
    }
}

/// The arc clock as hours, through integer milli-hours.
pub fn hours(tick: u64) -> f32 {
    (tick / TICKS_PER_MILLIHOUR) as f32 / 1_000.0
}

/// The heart's front at `hours` in fel tier `tier`.
pub fn front(hours: f32, tier: u8) -> f32 {
    let grown = (hours - START_H).clamp(0.0, GROW_H);
    F0 + RATE_PER_H * grown + SURGE[(tier as usize).min(SURGE.len() - 1)]
}

/// The spread at `tick`, with the island in fel tier `tier`.
pub fn now(plan: &Plan, haven: &Haven, tick: u64, tier: u8) -> Now {
    let h = hours(tick);
    let tier = (tier as usize).min(2);
    let mut n = Now {
        front: front(h, tier as u8),
        ward: ward_centre(haven),
        ..Now::default()
    };
    for s in plan.sites.iter().take(plan.n as usize) {
        let grown = ((h - s.start_h) / OUTBREAK_GROW_H).clamp(0.0, 1.0);
        if grown > 0.0 {
            push(
                &mut n,
                Disc {
                    x: s.x,
                    z: s.z,
                    r: s.r * grown,
                    depth: OUTBREAK_DEPTH,
                    hot: false,
                },
            );
        }
    }
    let ward_at = n.ward;
    let mut hot = |x: f32, z: f32, spot: (f32, [f32; 3])| {
        let depth = spot.1[tier];
        // THE GATE holds: a site inside its ward never burns.
        let clear = WARD_OUT_M + spot.0 * (1.0 + HOT_WANDER);
        if depth > 0.0 && dist2(x, z, ward_at.0, ward_at.1) >= clear * clear {
            push(
                &mut n,
                Disc {
                    x,
                    z,
                    r: spot.0,
                    depth,
                    hot: true,
                },
            );
        }
    };
    hot(haven.x, haven.z, HOT_HAVEN);
    if haven.ziggurat.live {
        hot(haven.ziggurat.x, haven.ziggurat.z, HOT_ZIGGURAT);
    }
    for m in haven.minor.iter().filter(|m| m.live) {
        hot(m.x, m.z, HOT_MINOR);
    }
    n
}

fn push(n: &mut Now, d: Disc) {
    if (n.n as usize) < MAX_DISCS {
        n.discs[n.n as usize] = d;
        n.n += 1;
    }
}

/// How deep the green is at (`x`, `z`), ground height `h`, now. The island
/// outside every reach pays a few compares and no noise tap.
pub fn depth(now: &Now, seed: u64, x: f32, z: f32, h: f32) -> Depth {
    if !within_reach(now, x, z) {
        return Depth::default();
    }
    depth_n(now, x, z, h, terrain::fel_wander(seed, x, z))
}

/// Whether anything that spreads could touch (`x`, `z`) now: the bound
/// [`depth`] skips its noise tap by, public so a cached caller (the client's
/// map) can skip the same work.
pub fn within_reach(now: &Now, x: f32, z: f32) -> bool {
    let reach = (now.front + FEL_WANDER) * CONTINENT_RADIUS;
    if dist2(x, z, FEL_HEART.0, FEL_HEART.1) < reach * reach {
        return true;
    }
    for d in now.discs.iter().take(now.n as usize) {
        let wander = if d.hot { HOT_WANDER } else { OUTBREAK_WANDER };
        let r = d.r * (1.0 + wander);
        if dist2(x, z, d.x, d.z) < r * r {
            return true;
        }
    }
    false
}

/// [`depth`] with the heart's wander noise already in hand
/// (`terrain::fel_wander`): what a caller that caches the noise (a terrain
/// vertex, a map texel) and the ground shader compute.
pub fn depth_n(now: &Now, x: f32, z: f32, h: f32, n: f32) -> Depth {
    let d2 = terrain::fel_d2(x, z);
    // The heart, exactly as worldgen has it, held at 0: its fade overshoots
    // by an ulp at its very edge, and nothing downstream wants a negative
    // depth (the meter's `as u16` already reads it as 0).
    let heart = terrain::fel_of(d2, n).max(0.0);
    let coast = ((h - BEACH_MAX_H) / COAST_M).clamp(0.0, 1.0);
    let mut fel = heart;
    if coast > 0.0 {
        // The creep: the heart's own potential, raised by high ground and by
        // the ward, against the front.
        let s = d2.sqrt() + n * FEL_WANDER;
        let ridge = RIDGE * ((h - RIDGE_LO_M) / (RIDGE_HI_M - RIDGE_LO_M)).clamp(0.0, 1.0);
        let p = s + ridge + ward(now, x, z);
        // Written as the heart's own fade from `front − BAND`, so at hour 0
        // (where that is exactly `FEL_R.0`) it rounds as the heart does and
        // the creep is 0.7 × the heart to the bit.
        let lo = now.front - BAND;
        let creep = CREEP_DEPTH * (1.0 - fade(((p - lo) / BAND).clamp(0.0, 1.0)));
        fel = fel.max(creep * coast);
    }
    let mut hot = 0.0f32;
    for d in now.discs.iter().take(now.n as usize) {
        if d.r <= 0.0 {
            continue;
        }
        let (wander, band) = if d.hot {
            (HOT_WANDER, HOT_BAND)
        } else {
            (OUTBREAK_WANDER, OUTBREAK_BAND)
        };
        let reach = d.r * (1.0 + wander);
        let dd2 = dist2(x, z, d.x, d.z);
        if dd2 >= reach * reach {
            continue;
        }
        let edge = dd2.sqrt() + n * wander * d.r;
        let w = band * d.r;
        let v = d.depth * (1.0 - fade(((edge - d.r + w) / w).clamp(0.0, 1.0)));
        if d.hot {
            hot = hot.max(v);
        } else {
            fel = fel.max(v * coast);
        }
    }
    Depth { fel, hot }
}

/// The ward's potential at (`x`, `z`): a wall inside `WARD_IN_M`, nothing
/// past `WARD_OUT_M`.
fn ward(now: &Now, x: f32, z: f32) -> f32 {
    let d = dist(x, z, now.ward.0, now.ward.1);
    WARD * (1.0 - fade(((d - WARD_IN_M) / (WARD_OUT_M - WARD_IN_M)).clamp(0.0, 1.0)))
}

/// Where THE GATE's ward stands: the town, or the island's centre on a seed
/// with no town.
pub fn ward_centre(haven: &Haven) -> (f32, f32) {
    if haven.town.live {
        (haven.town.x, haven.town.z)
    } else {
        (CENTRE, CENTRE)
    }
}

fn near_site(haven: &Haven, x: f32, z: f32, clear: f32) -> bool {
    if dist(x, z, haven.x, haven.z) < clear {
        return true;
    }
    if haven.ziggurat.live && dist(x, z, haven.ziggurat.x, haven.ziggurat.z) < clear {
        return true;
    }
    haven
        .minor
        .iter()
        .any(|m| m.live && dist(x, z, m.x, m.z) < clear)
}

/// The low 16 bits of `h` as a unit fraction.
fn unit(h: u64) -> f32 {
    (h & 0xFFFF) as f32 * (1.0 / 65_536.0)
}

fn dist2(ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    let (dx, dz) = (ax - bx, az - bz);
    dx * dx + dz * dz
}

fn dist(ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    dist2(ax, az, bx, bz).sqrt()
}
