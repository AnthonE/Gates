//! The attack helicopter (heli v0): Rust's Patrol Heli, cut down to *fly
//! over and be a threat* (operator, 2026-10-01: "doesnt even need missiles
//! just a strong gun", and nobody flies it yet). An AI gunship arrives from
//! the sea on a schedule, patrols the island, picks a player it can see,
//! circles them and fires bursts from a heavy gun, then heads back out to
//! sea. It cannot be hurt yet.
//!
//! **It rides the animal roster's last slot** ([`HELI_SLOT`], species
//! [`mob::MOB_HELI`]), so on the wire it is a mob record like a pig: the same
//! interest band, priority fill and interpolator, and no new entity class.
//! The roster slot carries what the wire carries (position, yaw, alive);
//! [`Heli`] carries the brain. The slot is never homed, so `mob::step` never
//! walks it, and `MobContent::def` has no row for the kind, so melee, arrows
//! and bullets find no hit volume on it (`melee::mob_cast`).
//!
//! **Its gun fires the revolver's round**: one `EV_SHOT` at speed zero per
//! round, resolved with the bullet's own body solve and world walk
//! (`ranged::nearest_body`, `ranged::world_stop`). A wall stops it exactly
//! as it stops a player's bullet, and every client draws it with the code
//! that already draws a gunshot. Hiding works: it needs line of sight to
//! pick you, it orbits where it **last saw** you rather than where you are,
//! and it gives up after `lose_ticks` without seeing you.
//!
//! Every balance number is content (`content/mobs.toml` `[heli]`). Under
//! [`HeliDef::INERT`] it never arrives — every test world, and any shard
//! whose boot did not arm it.

use crate::collide::ColIndex;
use crate::combat;
use crate::fmath::fabs;
use crate::limits::{ARROW_STEP_MM, MAX_MOBS, MAX_PLAYERS};
use crate::mob::{self, Mob};
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};
use crate::nav;
use crate::occupy::Occupants;
use crate::pitch_lut::pitch_dir;
use crate::ranged::{self, ARROW_EYE_MM, ARROW_R_M, MM_PER_M};
use crate::rng::cell_hash;
use crate::terrain::{self, Haven};
use crate::world::{EventQueue, Player, EV_SHOT};
use crate::yaw_lut::yaw_dir;

/// The roster slot the helicopter flies in. The last one, which was a pig:
/// `mob::kind_of` answers [`mob::MOB_HELI`] for it on both sides of the
/// wire, so a client knows what it is drawing from the id alone.
pub const HELI_SLOT: usize = MAX_MOBS - 1;

/// No player.
pub const NO_TARGET: u8 = u8::MAX;

/// The highest the heli flies, in `POS_Y_Q` quanta: 140 m. The wire's
/// absolute y window tops out at 143.35 m (`protocol::POS_Y_BITS`), and a
/// record past it is refused by the encoder, which would make the heli
/// vanish over a mountain. `protocol` asserts the margin.
pub const Y_CEIL_Q: i32 = 14_000;

/// Ticks between two looks (the target's line of sight, or a scan for a new
/// one). Half a second: a look is a world walk, the expensive part.
const LOOK_TICKS: u64 = 15;
/// Candidates one scan tests for line of sight, nearest first.
const LOOK_TRIES: usize = 2;
/// A sight line is walked at most this many samples, whatever its length.
/// Coarser than a bullet's spacing on a long look; the round it fires
/// afterwards is walked at the bullet's own spacing.
const SIGHT_SAMPLES: usize = 480;
/// A round is walked at most this many samples (the gun's reach over
/// `ARROW_STEP_MM`, so a 100 m gun at bullet spacing).
const ROUND_SAMPLES: usize = 600;
/// Where on a body it aims, above the feet.
const CHEST_MM: f32 = 1100.0;
/// Turn rate, wire yaw units per tick: three LUT steps, ~126°/s.
const TURN_STEP: u16 = 3 << 8;
/// The gun is on the nose: it fires only when the target is within this of
/// the heading (~56°).
const FACE_GAP: u16 = 40 << 8;
/// Horizontal and vertical acceleration, mm per tick per tick (~7 and
/// ~5 m/s²), and the climb rate cap, mm per tick (~4.8 m/s).
const ACCEL: f32 = 8.0;
const VACCEL: f32 = 6.0;
const CLIMB: f32 = 160.0;
/// It never flies lower than this over the ground under it.
const MIN_CLEAR_MM: i32 = 6_000;
/// How far ahead it reads the ground for its height, in ticks of its
/// current velocity: a hill is climbed before it arrives.
const LOOKAHEAD_TICKS: i32 = 45;
/// It enters and leaves this far inside the island square's border (the
/// wire's x/z window is the square, `protocol::POS_XZ_BITS`).
const EDGE_IN_MM: i32 = 30_000;
/// A waypoint (or the exit) is reached inside this.
const ARRIVE_MM: f32 = 60_000.0;
/// Waypoint draws stay this far inside the square.
const WAY_MARGIN_M: f32 = 400.0;
/// Draws one waypoint takes before falling back to the centre.
const WAY_TRIES: i32 = 8;

const CH_HELI_WAY: u32 = 200;
const CH_HELI_AIM: u32 = 201;
const CH_HELI_ENTRY: u32 = 202;

/// The baked `[heli]` table (`content::bake_heli`). Times are ticks,
/// distances millimetres, speeds mm per tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeliDef {
    /// From the first armed tick to the first arrival.
    pub first_ticks: u64,
    /// From one departure to the next arrival.
    pub every_ticks: u64,
    /// How long a visit patrols before heading out.
    pub patrol_ticks: u64,
    pub speed_mmpt: u32,
    pub engage_speed_mmpt: u32,
    /// Height over the ground, on patrol and while circling a target.
    pub cruise_mm: u32,
    pub engage_mm: u32,
    /// The circle it flies round a target.
    pub orbit_mm: u32,
    /// How far it spots a player (planar), given line of sight.
    pub detect_mm: u32,
    /// Unseen this long and it gives up on its target.
    pub lose_ticks: u64,
    /// The gun's reach.
    pub range_mm: u32,
    /// Per round that lands, before armour.
    pub damage: u16,
    pub burst: u8,
    /// Between rounds of a burst, and between bursts.
    pub rate_ticks: u16,
    pub gap_ticks: u16,
    /// Aim wobble: the aim point is jittered by up to this many mm per
    /// metre of range, on each axis.
    pub spread_pm: u16,
}

impl HeliDef {
    pub const INERT: Self = Self {
        first_ticks: 0,
        every_ticks: 0,
        patrol_ticks: 0,
        speed_mmpt: 0,
        engage_speed_mmpt: 0,
        cruise_mm: 0,
        engage_mm: 0,
        orbit_mm: 0,
        detect_mm: 0,
        lose_ticks: 0,
        range_mm: 0,
        damage: 0,
        burst: 0,
        rate_ticks: 0,
        gap_ticks: 0,
        spread_pm: 0,
    };

    #[inline]
    pub fn armed(&self) -> bool {
        self.speed_mmpt > 0
    }
}

/// The heli's brain and flight state. Hashed whenever it differs from
/// `Default` (`World::state_hash`), so an unarmed world folds nothing.
/// Not saved: like the animals, a restart sends it back to the schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heli {
    /// In the air over (or approaching) the island.
    pub flying: bool,
    /// The patrol is over and it is heading back out to sea.
    pub leaving: bool,
    /// While away, the tick it next arrives; 0 is unscheduled, which the
    /// first armed tick fixes (a fresh world, or a restored one whose tick
    /// is far past any absolute schedule).
    pub next_at: u64,
    /// While flying, the tick the patrol ends.
    pub leave_at: u64,
    /// Visits so far: every draw a visit makes is keyed on it.
    pub sortie: u32,
    /// Waypoints reached this visit.
    pub leg: u32,
    /// Feet position (mm) and velocity (mm per tick). The gun is
    /// `ARROW_EYE_MM` above the feet, which is where a client draws the
    /// shot from too (`render/fx/gun.rs`).
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub vx: i32,
    pub vy: i32,
    pub vz: i32,
    /// The waypoint, or the exit while leaving, mm.
    pub wx: i32,
    pub wz: i32,
    /// The player slot it is after, and that player's id (a slot can
    /// change tenant between two looks).
    pub target: u8,
    pub target_id: u32,
    /// Where it last saw the target (mm) and when.
    pub lx: i32,
    pub lz: i32,
    pub seen_at: u64,
    /// Its bearing round the target, wire yaw units.
    pub orbit: u16,
    pub next_shot: u64,
    pub burst_left: u8,
}

impl Default for Heli {
    fn default() -> Self {
        Self {
            flying: false,
            leaving: false,
            next_at: 0,
            leave_at: 0,
            sortie: 0,
            leg: 0,
            x: 0,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
            wx: 0,
            wz: 0,
            target: NO_TARGET,
            target_id: 0,
            lx: 0,
            lz: 0,
            seen_at: 0,
            orbit: 0,
            next_shot: 0,
            burst_left: 0,
        }
    }
}

impl Heli {
    /// Every field, little-endian, in declaration order — what
    /// `World::state_hash` folds.
    pub fn hash_bytes(&self) -> [u8; 96] {
        let mut b = [0u8; 96];
        b[0] = self.flying as u8;
        b[1] = self.leaving as u8;
        b[2] = self.target;
        b[3] = self.burst_left;
        b[4..12].copy_from_slice(&self.next_at.to_le_bytes());
        b[12..20].copy_from_slice(&self.leave_at.to_le_bytes());
        b[20..24].copy_from_slice(&self.sortie.to_le_bytes());
        b[24..28].copy_from_slice(&self.leg.to_le_bytes());
        for (i, v) in [
            self.x, self.y, self.z, self.vx, self.vy, self.vz, self.wx, self.wz, self.lx, self.lz,
        ]
        .iter()
        .enumerate()
        {
            b[28 + i * 4..32 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        b[68..72].copy_from_slice(&self.target_id.to_le_bytes());
        b[72..80].copy_from_slice(&self.seen_at.to_le_bytes());
        b[80..88].copy_from_slice(&self.next_shot.to_le_bytes());
        b[88..90].copy_from_slice(&self.orbit.to_le_bytes());
        b
    }

    fn engaged(&self) -> bool {
        self.target != NO_TARGET
    }

    fn drop_target(&mut self) {
        self.target = NO_TARGET;
        self.target_id = 0;
        self.burst_left = 0;
    }
}

/// One round that met a body, for `World::tick` to land: the heli reads the
/// player array and cannot write it (the bite's split, `mob::Bites`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Round {
    pub victim: u8,
    pub damage: u16,
    pub range_cm: u16,
}

/// One tick of the helicopter. Returns the round that hit somebody, if one
/// did; every round fired, hit or miss, is an `EV_SHOT` from the heli's id.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &HeliDef,
    cols: &ColIndex,
    occ: &mut Occupants,
    heli: &mut Heli,
    slot: &mut Mob,
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
) -> Option<Round> {
    if !def.armed() {
        return None;
    }
    if !heli.flying {
        if heli.next_at == 0 {
            heli.next_at = tick + def.first_ticks;
        }
        if tick < heli.next_at {
            return None;
        }
        arrive(seed, haven, tick, def, heli, players);
    }

    if !heli.leaving && tick >= heli.leave_at {
        leave(heli);
    }
    if tick.is_multiple_of(LOOK_TICKS) && !heli.leaving {
        look(seed, haven, tick, def, cols, occ, heli, slot, players);
    }
    fly(seed, haven, def, heli, players);

    if heli.leaving {
        let (dx, dz) = ((heli.wx - heli.x) as f32, (heli.wz - heli.z) as f32);
        if dx * dx + dz * dz < ARRIVE_MM * ARRIVE_MM {
            depart(tick, def, heli, slot);
            return None;
        }
    }

    // The heading: at the target while it has one (the gun is on the
    // nose), along the flight otherwise. At where the target IS only while
    // it is in sight — hidden, the nose holds on where it was last seen.
    let want = if heli.engaged() {
        let (tx, tz) = if tick <= heli.seen_at + LOOK_TICKS {
            feet_xz_mm(&players[heli.target as usize])
        } else {
            (heli.lx, heli.lz)
        };
        nav::yaw_toward((tx - heli.x) as f32, (tz - heli.z) as f32, slot.yaw)
    } else {
        nav::yaw_toward(heli.vx as f32, heli.vz as f32, slot.yaw)
    };
    slot.yaw = nav::turn_toward(slot.yaw, want, TURN_STEP);
    slot.body = Body {
        qx: heli.x / (POS_XZ_Q * MM_PER_M) as i32,
        qy: heli.y / (POS_Y_Q * MM_PER_M) as i32,
        qz: heli.z / (POS_XZ_Q * MM_PER_M) as i32,
        qvy: 0,
        grounded: false,
    };
    slot.alive = true;

    fire(
        seed, haven, tick, def, cols, occ, heli, slot, players, events,
    )
}

/// In from the sea: a point on the square's border, heading for the first
/// waypoint at cruise speed and height.
fn arrive(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &HeliDef,
    heli: &mut Heli,
    players: &[Player; MAX_PLAYERS],
) {
    let sortie = heli.sortie.wrapping_add(1);
    *heli = Heli {
        sortie,
        flying: true,
        leave_at: tick + def.patrol_ticks,
        ..Heli::default()
    };
    let h = cell_hash(seed, sortie as i32, 0, CH_HELI_ENTRY);
    let size = (terrain::ISLAND_SIZE * MM_PER_M) as i32;
    let along = EDGE_IN_MM + (unit(h, 8) * (size - 2 * EDGE_IN_MM) as f32) as i32;
    let (lo, hi) = (EDGE_IN_MM, size - EDGE_IN_MM);
    (heli.x, heli.z) = match h & 3 {
        0 => (along, lo),
        1 => (along, hi),
        2 => (lo, along),
        _ => (hi, along),
    };
    heli.y = ((floor_m(seed, haven, heli.x, heli.z) * MM_PER_M) as i32 + def.cruise_mm as i32)
        .min(Y_CEIL_Q * (POS_Y_Q * MM_PER_M) as i32);
    (heli.wx, heli.wz) = waypoint(seed, haven, heli, players);
}

/// Out to sea: the nearest point on the square's border.
fn leave(heli: &mut Heli) {
    heli.leaving = true;
    heli.drop_target();
    let size = (terrain::ISLAND_SIZE * MM_PER_M) as i32;
    let (lo, hi) = (EDGE_IN_MM, size - EDGE_IN_MM);
    let to_edge = [heli.x - lo, hi - heli.x, heli.z - lo, hi - heli.z];
    let mut side = 0;
    for (i, d) in to_edge.iter().enumerate() {
        if *d < to_edge[side] {
            side = i;
        }
    }
    (heli.wx, heli.wz) = match side {
        0 => (lo, heli.z),
        1 => (hi, heli.z),
        2 => (heli.x, lo),
        _ => (heli.x, hi),
    };
}

fn depart(tick: u64, def: &HeliDef, heli: &mut Heli, slot: &mut Mob) {
    *heli = Heli {
        sortie: heli.sortie,
        next_at: tick + def.every_ticks.max(1),
        ..Heli::default()
    };
    slot.alive = false;
}

/// The next waypoint. Odd legs go looking for somebody — a live player
/// outside the town, picked by the draw — which is the reference heli's
/// "seek out targets" rather than a sightseeing tour; even legs, and odd
/// ones with nobody to find, are a land point the draw picks.
fn waypoint(seed: u64, haven: &Haven, heli: &Heli, players: &[Player; MAX_PLAYERS]) -> (i32, i32) {
    let h = cell_hash(seed, heli.sortie as i32, heli.leg as i32, CH_HELI_WAY);
    if heli.leg % 2 == 1 {
        let n = players.iter().filter(|p| huntable(p)).count();
        if n > 0 {
            let pick = (h % n as u64) as usize;
            if let Some(p) = players.iter().filter(|p| huntable(p)).nth(pick) {
                return (
                    (p.body.qx as f32 * POS_XZ_Q * MM_PER_M) as i32,
                    (p.body.qz as f32 * POS_XZ_Q * MM_PER_M) as i32,
                );
            }
        }
    }
    let span = terrain::ISLAND_SIZE - 2.0 * WAY_MARGIN_M;
    for k in 0..WAY_TRIES {
        let h = cell_hash(
            seed,
            heli.sortie as i32,
            heli.leg as i32 * 64 + k,
            CH_HELI_WAY,
        );
        let x = WAY_MARGIN_M + unit(h, 8) * span;
        let z = WAY_MARGIN_M + unit(h, 32) * span;
        if terrain::height(seed, x, z) > terrain::BEACH_MAX_H && !terrain::in_haven(haven, x, z) {
            return ((x * MM_PER_M) as i32, (z * MM_PER_M) as i32);
        }
    }
    let c = (terrain::ISLAND_SIZE * 0.5 * MM_PER_M) as i32;
    (c, c)
}

/// A player the heli may hunt: alive, awake, standing, and outside the
/// town's safe zone (`combat::protected`, the bite's own rule).
fn huntable(p: &Player) -> bool {
    p.active && !p.dead && p.hp > 0 && !p.sleeping && !p.wounded && !combat::protected(p)
}

/// The look: confirm the target, or scan for one.
#[allow(clippy::too_many_arguments)]
fn look(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &HeliDef,
    cols: &ColIndex,
    occ: &mut Occupants,
    heli: &mut Heli,
    slot: &Mob,
    players: &[Player; MAX_PLAYERS],
) {
    let gun = gun_mm(heli);
    if heli.engaged() {
        let p = &players[heli.target as usize];
        if p.id != heli.target_id || !huntable(p) {
            heli.drop_target();
        } else if sees(seed, haven, cols, occ, gun, p) {
            heli.seen_at = tick;
            (heli.lx, heli.lz) = feet_xz_mm(p);
        } else if tick > heli.seen_at + def.lose_ticks {
            heli.drop_target();
        }
        if heli.engaged() {
            return;
        }
    }
    // Nearest first, `LOOK_TRIES` of them: a selection by repeated scan
    // rather than a sort, so nothing here allocates or needs a buffer.
    let reach2 = def.detect_mm as f32 * def.detect_mm as f32;
    let mut tried = [usize::MAX; LOOK_TRIES];
    for t in 0..LOOK_TRIES {
        let mut best: Option<(f32, usize)> = None;
        for (i, p) in players.iter().enumerate() {
            if !huntable(p) || tried[..t].contains(&i) {
                continue;
            }
            let (px, pz) = feet_xz_mm(p);
            let (dx, dz) = ((px - heli.x) as f32, (pz - heli.z) as f32);
            let d2 = dx * dx + dz * dz;
            if d2 <= reach2 && best.is_none_or(|(b, _)| d2 < b) {
                best = Some((d2, i));
            }
        }
        let Some((_, i)) = best else {
            return;
        };
        tried[t] = i;
        let p = &players[i];
        if sees(seed, haven, cols, occ, gun, p) {
            let (px, pz) = feet_xz_mm(p);
            heli.target = i as u8;
            heli.target_id = p.id;
            heli.seen_at = tick;
            (heli.lx, heli.lz) = (px, pz);
            // Start the circle from the side it is already on, so the
            // first thing it does is not cross over the target.
            heli.orbit = nav::yaw_toward((heli.x - px) as f32, (heli.z - pz) as f32, slot.yaw);
            heli.burst_left = 0;
            // A beat between being spotted and the first burst: the
            // rotor turns on you before the gun does.
            heli.next_shot = tick + def.gap_ticks as u64;
            return;
        }
    }
}

/// One tick of flight: steer toward the goal, hold the height.
fn fly(seed: u64, haven: &Haven, def: &HeliDef, heli: &mut Heli, players: &[Player; MAX_PLAYERS]) {
    let (gx, gz, speed) = if heli.engaged() {
        // Round where it last saw them, at the engage speed. The bearing
        // advances by the arc the engage speed covers on the circle.
        let r = def.orbit_mm.max(1) as f32;
        let step = def.engage_speed_mmpt as f32 * (65_536.0 / 6.283_185_5) / r;
        heli.orbit = heli.orbit.wrapping_add(step as u16);
        let (ox, oz) = yaw_dir(heli.orbit);
        let gx = heli.lx as f32 + ox * r;
        let gz = heli.lz as f32 + oz * r;
        // Far off the circle (it just spotted somebody, or they ran), it
        // closes at cruise speed.
        let (dx, dz) = (gx - heli.x as f32, gz - heli.z as f32);
        let far = dx * dx + dz * dz > 4.0 * r * r;
        let speed = if far {
            def.speed_mmpt
        } else {
            def.engage_speed_mmpt
        };
        (gx, gz, speed as f32)
    } else {
        let (dx, dz) = ((heli.wx - heli.x) as f32, (heli.wz - heli.z) as f32);
        if !heli.leaving && dx * dx + dz * dz < ARRIVE_MM * ARRIVE_MM {
            heli.leg += 1;
            (heli.wx, heli.wz) = waypoint(seed, haven, heli, players);
        }
        (heli.wx as f32, heli.wz as f32, def.speed_mmpt as f32)
    };

    // Horizontal: the velocity it wants, approached at `ACCEL`.
    let (dx, dz) = (gx - heli.x as f32, gz - heli.z as f32);
    let d = (dx * dx + dz * dz).sqrt();
    let (wvx, wvz) = if d > 1.0 {
        // Slows into a goal it would otherwise overshoot in a second.
        let s = speed.min(d / 30.0);
        (dx / d * s, dz / d * s)
    } else {
        (0.0, 0.0)
    };
    let (mut ax, mut az) = (wvx - heli.vx as f32, wvz - heli.vz as f32);
    let a = (ax * ax + az * az).sqrt();
    if a > ACCEL {
        ax = ax / a * ACCEL;
        az = az / a * ACCEL;
    }
    heli.vx = (heli.vx as f32 + ax) as i32;
    heli.vz = (heli.vz as f32 + az) as i32;

    // Vertical: its height over the higher of the ground under it and the
    // ground it is about to reach.
    let here = floor_m(seed, haven, heli.x, heli.z);
    let ahead = floor_m(
        seed,
        haven,
        heli.x + heli.vx * LOOKAHEAD_TICKS,
        heli.z + heli.vz * LOOKAHEAD_TICKS,
    );
    let alt = if heli.engaged() {
        def.engage_mm
    } else {
        def.cruise_mm
    };
    let ceil = Y_CEIL_Q * (POS_Y_Q * MM_PER_M) as i32;
    let want_y = ((here.max(ahead) * MM_PER_M) as i32 + alt as i32).min(ceil);
    let wvy = ((want_y - heli.y) as f32 / 30.0).clamp(-CLIMB, CLIMB);
    let ay = (wvy - heli.vy as f32).clamp(-VACCEL, VACCEL);
    heli.vy = (heli.vy as f32 + ay) as i32;

    let size = (terrain::ISLAND_SIZE * MM_PER_M) as i32;
    heli.x = (heli.x + heli.vx).clamp(1_000, size - 1_000);
    heli.z = (heli.z + heli.vz).clamp(1_000, size - 1_000);
    let floor = (floor_m(seed, haven, heli.x, heli.z) * MM_PER_M) as i32 + MIN_CLEAR_MM;
    heli.y = (heli.y + heli.vy).max(floor).min(ceil);
}

/// The gun: a burst while the target is in sight, in reach and on the nose.
#[allow(clippy::too_many_arguments)]
fn fire(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &HeliDef,
    cols: &ColIndex,
    occ: &mut Occupants,
    heli: &mut Heli,
    slot: &Mob,
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
) -> Option<Round> {
    if !heli.engaged() || heli.leaving {
        return None;
    }
    // In sight as of the last look. Lost from view, it holds fire and
    // starts a fresh burst when it finds them again.
    if tick > heli.seen_at + LOOK_TICKS {
        heli.burst_left = 0;
        return None;
    }
    if tick < heli.next_shot {
        return None;
    }
    let p = &players[heli.target as usize];
    // Measured from the gun on the slot's own quanta — the same origin a
    // client draws the shot from, so the picture and the hit agree.
    let o = (
        (slot.body.qx * (POS_XZ_Q * MM_PER_M) as i32) as f32,
        (slot.body.qy * (POS_Y_Q * MM_PER_M) as i32 + ARROW_EYE_MM) as f32,
        (slot.body.qz * (POS_XZ_Q * MM_PER_M) as i32) as f32,
    );
    let (px, pz) = feet_xz_mm(p);
    let t = (
        px as f32,
        p.body.qy as f32 * POS_Y_Q * MM_PER_M + CHEST_MM,
        pz as f32,
    );
    let (dx, dy, dz) = (t.0 - o.0, t.1 - o.1, t.2 - o.2);
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist > def.range_mm as f32 {
        return None;
    }
    if nav::yaw_gap(slot.yaw, nav::yaw_toward(dx, dz, slot.yaw)) > FACE_GAP {
        return None;
    }
    if heli.burst_left == 0 {
        heli.burst_left = def.burst.max(1);
    }
    heli.burst_left -= 1;
    heli.next_shot = tick
        + if heli.burst_left == 0 {
            def.gap_ticks.max(1)
        } else {
            def.rate_ticks.max(1)
        } as u64;

    // The wobble: the aim point jittered on each axis by up to
    // `spread_pm` mm per metre of range.
    let h = cell_hash(seed, heli.sortie as i32, tick as i32, CH_HELI_AIM);
    let r = dist * def.spread_pm as f32 / MM_PER_M;
    let (ax, ay, az) = (
        dx + (unit(h, 0) * 2.0 - 1.0) * r,
        dy + (unit(h, 16) * 2.0 - 1.0) * r,
        dz + (unit(h, 32) * 2.0 - 1.0) * r,
    );
    let yaw = nav::yaw_toward(ax, az, slot.yaw);
    let pitch = pitch_toward(ay, (ax * ax + az * az).sqrt());
    events.push(
        EV_SHOT,
        mob::mob_id(HELI_SLOT),
        (yaw as u32) << 8 | pitch as u32,
        def.range_mm / 100,
    );

    // The bullet's own resolution: the nearest body on the whole segment,
    // then the world walked only as far as that body.
    let reach = def.range_mm as f32;
    let (fx, fz) = yaw_dir(yaw);
    let (ch, sv) = pitch_dir(pitch);
    let s = (fx * ch * reach, sv * reach, fz * ch * reach);
    let hit = ranged::nearest_body(
        players,
        o,
        s,
        1.0,
        mob::mob_id(HELI_SLOT),
        ranged::Pose::Live,
    )?;
    let n = (def.range_mm as usize / ARROW_STEP_MM as usize + 1).min(ROUND_SAMPLES);
    let upto = (hit.t * n as f32) as usize + 1;
    let (stop_t, _, _) = ranged::world_stop(seed, haven, cols, occ, o, s, n, upto, ARROW_R_M);
    (hit.t <= stop_t).then_some(Round {
        victim: hit.slot as u8,
        damage: def.damage,
        range_cm: (reach * hit.t / 10.0) as u16,
    })
}

/// Can the gun at `gun` (mm) see this player's chest? The world walk the
/// bullet uses, over the sight line, ignoring bodies.
fn sees(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    gun: (f32, f32, f32),
    p: &Player,
) -> bool {
    let (px, pz) = feet_xz_mm(p);
    let s = (
        px as f32 - gun.0,
        p.body.qy as f32 * POS_Y_Q * MM_PER_M + CHEST_MM - gun.1,
        pz as f32 - gun.2,
    );
    let len = (s.0 * s.0 + s.1 * s.1 + s.2 * s.2).sqrt();
    let n = ((len / ARROW_STEP_MM as f32) as usize + 1).min(SIGHT_SAMPLES);
    let (_, surf, _) = ranged::world_stop(seed, haven, cols, occ, gun, s, n, n, ARROW_R_M);
    surf.is_none()
}

fn gun_mm(heli: &Heli) -> (f32, f32, f32) {
    (heli.x as f32, (heli.y + ARROW_EYE_MM) as f32, heli.z as f32)
}

fn feet_xz_mm(p: &Player) -> (i32, i32) {
    (
        (p.body.qx as f32 * POS_XZ_Q * MM_PER_M) as i32,
        (p.body.qz as f32 * POS_XZ_Q * MM_PER_M) as i32,
    )
}

/// The surface it keeps its height over, metres: the ground, or the sea.
fn floor_m(seed: u64, haven: &Haven, x_mm: i32, z_mm: i32) -> f32 {
    let size = terrain::ISLAND_SIZE;
    let x = (x_mm as f32 / MM_PER_M).clamp(0.0, size);
    let z = (z_mm as f32 / MM_PER_M).clamp(0.0, size);
    terrain::ground(seed, haven, x, z).max(terrain::SEA_LEVEL)
}

/// Sixteen bits of `h` from `shift`, as `0.0..=1.0`.
#[inline]
fn unit(h: u64, shift: u32) -> f32 {
    ((h >> shift) & 0xFFFF) as f32 / 65_535.0
}

/// The wire pitch whose direction rises `dy` over a planar `run` — the
/// pitch LUT searched for the nearest sine, since the sim may not call
/// trig. The table's sine rises monotonically from index 0 (straight down)
/// to 255 (straight up).
pub fn pitch_toward(dy: f32, run: f32) -> u8 {
    let len = (dy * dy + run * run).sqrt();
    if len <= 0.0 {
        return 128;
    }
    let want = dy / len;
    let (mut lo, mut hi) = (0usize, 255usize);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if pitch_dir(mid as u8).1 < want {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // `lo` is the first entry at or above; its neighbour below may be nearer.
    if lo > 0 && fabs(pitch_dir(lo as u8 - 1).1 - want) < fabs(pitch_dir(lo as u8).1 - want) {
        lo -= 1;
    }
    lo as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_toward_inverts_the_table() {
        for i in 0..=255u8 {
            let (c, s) = pitch_dir(i);
            assert_eq!(pitch_toward(s, c), i, "entry {i}");
        }
        assert_eq!(pitch_toward(-1.0, 0.0), 0);
        assert_eq!(pitch_toward(1.0, 0.0), 255);
    }

    #[test]
    fn the_heli_slot_is_the_heli() {
        assert_eq!(mob::kind_of(HELI_SLOT), mob::MOB_HELI);
        assert_eq!(mob::guard_site_of(HELI_SLOT), None);
        assert_eq!(mob::pack_of(HELI_SLOT), None);
    }
}
