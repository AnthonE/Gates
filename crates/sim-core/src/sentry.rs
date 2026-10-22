//! THE GATE's sentries: Rust's Outpost turrets. Four guns stand on the
//! watchtower roofs (`town::SENTRY_POSTS`) and shoot any **hostile** player
//! inside the safe zone — someone who attacked a player in the last
//! `combat::HOSTILE_TICKS` — so a fight at the gate cannot be ended by
//! running inside. Everyone else they leave alone, and they cannot be hurt.
//!
//! They ride the animal roster the way the heli does ([`SENTRY_SLOT0`],
//! species `mob::MOB_SENTRY`): on the wire each is a mob record whose yaw is
//! where the gun points, so a client draws the turret turning with no new
//! entity class, and its rounds are `EV_SHOT`s from the slot's id resolved
//! with the bullet's own body solve and world walk. A sentry that acquires a
//! target says so first (`EV_SENTRY_LOCK`) and fires `lock_ticks` later:
//! Rust's turret beeps before it shoots.
//!
//! Every balance number is content (`content/sites.toml` `[sentry]`).
//! Under [`SentryDef::INERT`] the posts stay empty — every test world, and
//! any shard whose boot did not arm them.

use crate::collide::ColIndex;
use crate::heli::{pitch_toward, Round};
use crate::limits::{ARROW_STEP_MM, MAX_PLAYERS};
use crate::mob::{self, Mob};
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};
use crate::nav;
use crate::occupy::Occupants;
use crate::pitch_lut::pitch_dir;
use crate::ranged::{self, ARROW_EYE_MM, ARROW_R_M, MM_PER_M};
use crate::rng::cell_hash;
use crate::terrain::Haven;
use crate::world::{EventQueue, Player, EV_SENTRY_LOCK, EV_SHOT};
use crate::yaw_lut::yaw_dir;

/// Guns on the town's towers.
pub const SENTRIES: usize = crate::town::SENTRY_POSTS.len();
/// The first sentry's roster slot: they sit just below the heli's.
pub const SENTRY_SLOT0: usize = crate::heli::HELI_SLOT - SENTRIES;

/// No player.
pub const NO_TARGET: u8 = u8::MAX;

/// Ticks between two looks (a target's sight line, or a scan for one).
pub(crate) const LOOK_TICKS: u64 = 10;
/// Candidates one scan tests for line of sight, nearest first; one more is
/// then taken in turn from the rest ([`rotating_pick`]).
const LOOK_TRIES: usize = 3;
/// A sight line is walked at most this many samples (`ARROW_STEP_MM`
/// apart, so ~100 m — the longest reach content may give the gun).
const SIGHT_SAMPLES: usize = 600;
/// A round is walked at most this many samples.
const ROUND_SAMPLES: usize = 600;
/// Where on a body it aims, above the feet.
const CHEST_MM: f32 = 1100.0;
/// Traverse, wire yaw units per tick: four LUT steps, ~170°/s.
pub(crate) const TURN_STEP: u16 = 4 << 8;
/// It fires only with the target within this of where the gun points.
const FACE_GAP: u16 = 12 << 8;

const CH_SENTRY_AIM: u32 = 210;

/// Whether a roster slot is one of the town's sentries.
#[inline]
pub const fn is_sentry_slot(slot: usize) -> bool {
    slot >= SENTRY_SLOT0 && slot < SENTRY_SLOT0 + SENTRIES
}

/// The baked `[sentry]` table (`content::bake_sentry`). Times are ticks,
/// distances millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SentryDef {
    /// How far it shoots, and how far it looks.
    pub range_mm: u32,
    /// Per round that lands, before armour.
    pub damage: u16,
    pub burst: u8,
    /// Between rounds of a burst, and between bursts.
    pub rate_ticks: u16,
    pub gap_ticks: u16,
    /// From the lock to the first round: the warning.
    pub lock_ticks: u16,
    /// Unseen this long and it lets the target go.
    pub lose_ticks: u16,
    /// Aim wobble: up to this many mm per metre of range, on each axis.
    pub spread_pm: u16,
}

impl SentryDef {
    pub const INERT: Self = Self {
        range_mm: 0,
        damage: 0,
        burst: 0,
        rate_ticks: 0,
        gap_ticks: 0,
        lock_ticks: 0,
        lose_ticks: 0,
        spread_pm: 0,
    };

    #[inline]
    pub fn armed(&self) -> bool {
        self.range_mm > 0 && self.damage > 0
    }
}

/// One gun's brain. Hashed whenever it differs from `Default`
/// (`World::state_hash`); not saved — a restart forgets who it was after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sentry {
    /// The player slot it is after, and that player's id.
    pub target: u8,
    pub target_id: u32,
    pub seen_at: u64,
    pub next_shot: u64,
    pub burst_left: u8,
    /// The roster slot a scan's rotating try starts from ([`rotating_pick`]):
    /// one past the last slot it took. Kept when the target goes, or a
    /// hidden body locked and then dropped would send it back to slot 0.
    pub scan: u8,
}

impl Default for Sentry {
    fn default() -> Self {
        Self {
            target: NO_TARGET,
            target_id: 0,
            seen_at: 0,
            next_shot: 0,
            burst_left: 0,
            scan: 0,
        }
    }
}

impl Sentry {
    /// Every field, little-endian, in declaration order.
    pub fn hash_bytes(&self) -> [u8; 23] {
        let mut b = [0u8; 23];
        b[0] = self.target;
        b[1] = self.burst_left;
        b[2..6].copy_from_slice(&self.target_id.to_le_bytes());
        b[6..14].copy_from_slice(&self.seen_at.to_le_bytes());
        b[14..22].copy_from_slice(&self.next_shot.to_le_bytes());
        b[22] = self.scan;
        b
    }

    pub(crate) fn engaged(&self) -> bool {
        self.target != NO_TARGET
    }

    /// Back to idle, the scan's place kept.
    pub(crate) fn drop_target(&mut self) {
        *self = Sentry {
            scan: self.scan,
            ..Sentry::default()
        };
    }
}

/// Who a sentry shoots: a live, awake player standing in the safe zone
/// while hostile. A downed one too — the gun finishes what it started.
pub fn wanted(town: &crate::town::Town, p: &Player) -> bool {
    if !p.active || p.dead || p.sleeping || p.hp == 0 || p.hostile == 0 {
        return false;
    }
    let x = p.body.qx as f32 * POS_XZ_Q;
    let z = p.body.qz as f32 * POS_XZ_Q;
    crate::town::safe(town, x, z)
}

/// One tick of the town's guns. Each sentry that landed a round on somebody
/// writes it to `rounds`, for `World::tick` to land (the heli's split: the
/// guns read the player array and cannot write it).
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &SentryDef,
    cols: &ColIndex,
    occ: &mut Occupants,
    guns: &mut [Sentry; SENTRIES],
    slots: &mut [Mob],
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
    rounds: &mut [Option<Round>; SENTRIES],
) {
    *rounds = [None; SENTRIES];
    let town = haven.town;
    if !def.armed() || !town.live {
        for (g, m) in guns.iter_mut().zip(slots.iter_mut()) {
            *g = Sentry::default();
            m.alive = false;
        }
        return;
    }
    for k in 0..SENTRIES {
        let Some((x, y, z)) = crate::town::sentry_world(&town, k) else {
            continue;
        };
        let slot = &mut slots[k];
        // The record's feet sit `ARROW_EYE_MM` under the muzzle, and the gun
        // is measured back off the record's own quanta — the origin a
        // client draws the shot from, so the picture and the hit agree.
        slot.body = Body {
            qx: (x / POS_XZ_Q) as i32,
            qy: ((y * MM_PER_M - ARROW_EYE_MM as f32) / (POS_Y_Q * MM_PER_M)) as i32,
            qz: (z / POS_XZ_Q) as i32,
            qvy: 0,
            grounded: true,
        };
        slot.alive = true;
        let gun = (
            (slot.body.qx * (POS_XZ_Q * MM_PER_M) as i32) as f32,
            (slot.body.qy * (POS_Y_Q * MM_PER_M) as i32 + ARROW_EYE_MM) as f32,
            (slot.body.qz * (POS_XZ_Q * MM_PER_M) as i32) as f32,
        );
        let s = &mut guns[k];
        if tick.is_multiple_of(LOOK_TICKS) {
            look(
                seed,
                haven,
                tick,
                def,
                cols,
                occ,
                s,
                SENTRY_SLOT0 + k,
                gun,
                players,
                events,
                &|p| wanted(&town, p),
            );
        }
        if s.engaged() {
            let p = &players[s.target as usize];
            let (px, pz) = (
                p.body.qx as f32 * POS_XZ_Q * MM_PER_M,
                p.body.qz as f32 * POS_XZ_Q * MM_PER_M,
            );
            let want = nav::yaw_toward(px - gun.0, pz - gun.2, slot.yaw);
            slot.yaw = nav::turn_toward(slot.yaw, want, TURN_STEP);
        }
        rounds[k] = fire(
            seed,
            haven,
            tick,
            def,
            cols,
            occ,
            s,
            (SENTRY_SLOT0 + k, k as i32),
            gun,
            slot,
            players,
            events,
        );
    }
}

/// The look: keep the target while it is wanted and in sight, or find the
/// nearest wanted player in sight. `slot` is the gun's roster slot (its
/// wire id); `wanted` is who it shoots — the town's hostiles, or a player
/// turret's strangers (`turret.rs`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn look(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &SentryDef,
    cols: &ColIndex,
    occ: &mut Occupants,
    s: &mut Sentry,
    slot: usize,
    gun: (f32, f32, f32),
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
    wanted: &dyn Fn(&Player) -> bool,
) {
    if s.engaged() {
        let p = &players[s.target as usize];
        if p.id != s.target_id || !wanted(p) {
            s.drop_target();
        } else if sees(seed, haven, cols, occ, gun, p) {
            s.seen_at = tick;
        } else if tick > s.seen_at + def.lose_ticks as u64 {
            s.drop_target();
        }
        if s.engaged() {
            return;
        }
    }
    let reach2 = def.range_mm as f32 * def.range_mm as f32;
    let d2_of = |p: &Player| {
        let (dx, dz) = (
            p.body.qx as f32 * POS_XZ_Q * MM_PER_M - gun.0,
            p.body.qz as f32 * POS_XZ_Q * MM_PER_M - gun.2,
        );
        dx * dx + dz * dz
    };
    // The `LOOK_TRIES` nearest, then one more in turn from the rest.
    let mut tried = [usize::MAX; LOOK_TRIES + 1];
    for t in 0..=LOOK_TRIES {
        let pick = if t < LOOK_TRIES {
            let mut best: Option<(f32, usize)> = None;
            for (i, p) in players.iter().enumerate() {
                if !wanted(p) || tried[..t].contains(&i) {
                    continue;
                }
                let d2 = d2_of(p);
                if d2 <= reach2 && best.is_none_or(|(b, _)| d2 < b) {
                    best = Some((d2, i));
                }
            }
            best.map(|(_, i)| i)
        } else {
            let i = rotating_pick(players, s.scan as usize, |i, p| {
                !tried[..t].contains(&i) && wanted(p) && d2_of(p) <= reach2
            });
            if let Some(i) = i {
                s.scan = ((i + 1) % MAX_PLAYERS) as u8;
            }
            i
        };
        let Some(i) = pick else {
            return;
        };
        tried[t] = i;
        let p = &players[i];
        if sees(seed, haven, cols, occ, gun, p) {
            *s = Sentry {
                target: i as u8,
                target_id: p.id,
                seen_at: tick,
                next_shot: tick + def.lock_ticks.max(1) as u64,
                burst_left: 0,
                scan: s.scan,
            };
            events.push(EV_SENTRY_LOCK, mob::mob_id(slot), p.id, 0);
            return;
        }
    }
}

/// A scan's extra try, after the nearest: the first slot at or after
/// `start` (wrapping) that `eligible` passes. Nearest-only let the few
/// nearest wanted bodies blind a gun for good: hidden behind a wall, they
/// were every try of every look, and the raider in plain view further out
/// was never asked.
///
/// The caller keeps `start` (`Sentry::scan`, `Heli::scan`) one past the
/// slot this took, so it only ever moves forward past a slot it asked:
/// toggling hidden bodies in and out of range cannot jump it over the
/// raider, who is reached within one look per hidden body between the
/// cursor and them. A count-based turn (`turn % n`) could be steered off
/// the raider for good by two hidden alts taking turns at the range's
/// edge. One pass over the roster, no buffer, deterministic in slot order.
pub(crate) fn rotating_pick(
    players: &[Player; MAX_PLAYERS],
    start: usize,
    eligible: impl Fn(usize, &Player) -> bool,
) -> Option<usize> {
    (0..MAX_PLAYERS)
        .map(|j| (start + j) % MAX_PLAYERS)
        .find(|&i| eligible(i, &players[i]))
}
// The cursor is a `u8`.
const _: () = assert!(MAX_PLAYERS <= 256);

/// The gun: a burst while the target is in sight, in reach and in line.
/// `(slot, aim)` is the gun's roster slot and the key its aim wobble is
/// drawn off.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &SentryDef,
    cols: &ColIndex,
    occ: &mut Occupants,
    s: &mut Sentry,
    (slot_ix, aim): (usize, i32),
    gun: (f32, f32, f32),
    slot: &Mob,
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
) -> Option<Round> {
    if !s.engaged() || tick < s.next_shot {
        return None;
    }
    // In sight as of the last look; lost, it holds fire.
    if tick > s.seen_at + LOOK_TICKS {
        s.burst_left = 0;
        return None;
    }
    let p = &players[s.target as usize];
    let t = (
        p.body.qx as f32 * POS_XZ_Q * MM_PER_M,
        p.body.qy as f32 * POS_Y_Q * MM_PER_M + CHEST_MM,
        p.body.qz as f32 * POS_XZ_Q * MM_PER_M,
    );
    let (dx, dy, dz) = (t.0 - gun.0, t.1 - gun.1, t.2 - gun.2);
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist > def.range_mm as f32 {
        return None;
    }
    if nav::yaw_gap(slot.yaw, nav::yaw_toward(dx, dz, slot.yaw)) > FACE_GAP {
        return None;
    }
    if s.burst_left == 0 {
        s.burst_left = def.burst.max(1);
    }
    s.burst_left -= 1;
    s.next_shot = tick
        + if s.burst_left == 0 {
            def.gap_ticks.max(1)
        } else {
            def.rate_ticks.max(1)
        } as u64;

    let h = cell_hash(seed, aim, tick as i32, CH_SENTRY_AIM);
    let r = dist * def.spread_pm as f32 / MM_PER_M;
    let unit = |shift: u32| ((h >> shift) & 0xFFFF) as f32 / 65_535.0;
    let (ax, ay, az) = (
        dx + (unit(0) * 2.0 - 1.0) * r,
        dy + (unit(16) * 2.0 - 1.0) * r,
        dz + (unit(32) * 2.0 - 1.0) * r,
    );
    let yaw = nav::yaw_toward(ax, az, slot.yaw);
    let pitch = pitch_toward(ay, (ax * ax + az * az).sqrt());
    let id = mob::mob_id(slot_ix);
    events.push(
        EV_SHOT,
        id,
        (yaw as u32) << 8 | pitch as u32,
        def.range_mm / 100,
    );

    let reach = def.range_mm as f32;
    let (fx, fz) = yaw_dir(yaw);
    let (ch, sv) = pitch_dir(pitch);
    let seg = (fx * ch * reach, sv * reach, fz * ch * reach);
    let hit = ranged::nearest_body(players, gun, seg, 1.0, id, ranged::Pose::Live)?;
    let n = (def.range_mm as usize / ARROW_STEP_MM as usize + 1).min(ROUND_SAMPLES);
    let upto = (hit.t * n as f32) as usize + 1;
    let (stop_t, _, _) = ranged::world_stop(seed, haven, cols, occ, gun, seg, n, upto, ARROW_R_M);
    (hit.t <= stop_t).then_some(Round {
        victim: hit.slot as u8,
        damage: def.damage,
        range_cm: (reach * hit.t / 10.0) as u16,
    })
}

/// Can the gun see this player's chest? The bullet's world walk over the
/// sight line, ignoring bodies.
fn sees(
    seed: u64,
    haven: &Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    gun: (f32, f32, f32),
    p: &Player,
) -> bool {
    let s = (
        p.body.qx as f32 * POS_XZ_Q * MM_PER_M - gun.0,
        p.body.qy as f32 * POS_Y_Q * MM_PER_M + CHEST_MM - gun.1,
        p.body.qz as f32 * POS_XZ_Q * MM_PER_M - gun.2,
    );
    let len = (s.0 * s.0 + s.1 * s.1 + s.2 * s.2).sqrt();
    let n = ((len / ARROW_STEP_MM as f32) as usize + 1).min(SIGHT_SAMPLES);
    let (_, surf, _) = ranged::world_stop(seed, haven, cols, occ, gun, s, n, n, ARROW_R_M);
    surf.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentry_slots_are_sentries() {
        for k in 0..SENTRIES {
            let slot = SENTRY_SLOT0 + k;
            assert!(is_sentry_slot(slot));
            assert_eq!(mob::kind_of(slot), mob::MOB_SENTRY);
            assert_eq!(mob::guard_site_of(slot), None);
            assert_eq!(mob::pack_of(slot), None);
            assert_eq!(mob::ordinal(slot), None);
        }
        assert!(!is_sentry_slot(crate::heli::HELI_SLOT));
        assert!(!is_sentry_slot(SENTRY_SLOT0 - 1));
    }

    /// A gun and wanted players along +x from it, `(metres off, in the
    /// open)` in slot order: the hidden ones sunk under the ground (a wall's
    /// back in the field), on a spot where exactly the open ones are in
    /// sight. Looks until the gun locks or `max` looks pass, asking
    /// `wanted(look, player)`. The id it locked on (slot `k` is id
    /// `10 + k`), and the looks it took.
    fn scan_field(
        layout: &[(f32, bool)],
        max: u64,
        wanted: impl Fn(u64, &Player) -> bool,
    ) -> (Option<u32>, u64) {
        const SEED: u64 = 0x0FEE_0FEE;
        let haven = crate::terrain::haven(SEED);
        let cols = ColIndex::new();
        let mut scratch = crate::occupy::Scratch::barren();
        let def = SentryDef {
            range_mm: 30_000,
            damage: 20,
            burst: 8,
            rate_ticks: 12,
            gap_ticks: 100,
            lock_ticks: 30,
            lose_ticks: 90,
            spread_pm: 0,
        };
        let mut players: [Player; MAX_PLAYERS] = std::array::from_fn(|_| Player::default());
        let mut found = None;
        'search: for i in 0..400 {
            let (gx, gz) = (
                1500.0 + (i % 20) as f32 * 37.0,
                1500.0 + (i / 20) as f32 * 41.0,
            );
            let gy = crate::terrain::height(SEED, gx, gz) + 2.0;
            let gun = (gx * MM_PER_M, gy * MM_PER_M, gz * MM_PER_M);
            for (k, &(d, open)) in layout.iter().enumerate() {
                let p = &mut players[k];
                p.id = 10 + k as u32;
                p.active = true;
                p.body = Body::at(SEED, &haven, gx + d, gz);
                if !open {
                    p.body.qy -= (20.0 / POS_Y_Q) as i32;
                }
            }
            let mut occ = scratch.occupants();
            if layout
                .iter()
                .enumerate()
                .all(|(k, &(_, open))| sees(SEED, &haven, &cols, &mut occ, gun, &players[k]) == open)
            {
                found = Some(gun);
                break 'search;
            }
        }
        let gun = found.expect("no spot on this seed with exactly the open ones in sight");
        let mut s = Sentry::default();
        let mut events = EventQueue::default();
        let mut looks = 0;
        while !s.engaged() && looks < max {
            look(
                SEED,
                &haven,
                looks * LOOK_TICKS,
                &def,
                &cols,
                &mut scratch.occupants(),
                &mut s,
                SENTRY_SLOT0,
                gun,
                &players,
                &mut events,
                &|p: &Player| p.active && wanted(looks, p),
            );
            looks += 1;
        }
        (s.engaged().then_some(s.target_id), looks)
    }

    /// **Hidden bodies nearer than a target in plain view do not blind the
    /// gun** (exploit sweep 2, pve#0). Three wanted players out of sight a
    /// few metres off were the nearest three, so every try of every look
    /// was spent on them and the one in the open nine metres out was never
    /// asked.
    #[test]
    fn hidden_bodies_nearer_than_the_target_do_not_blind_the_gun() {
        let layout = [(3.0, false), (4.0, false), (5.0, false), (9.0, true)];
        let (locked, looks) = scan_field(&layout, 8, |_, _| true);
        assert_eq!(locked, Some(13), "three hidden bodies blinded the gun");
        assert_eq!(looks, 1, "{looks} looks to find the one in the open");
    }

    /// **The nearest are all asked on every look** (review 5, claims#4):
    /// the extra try comes on top of them, never in place of the last one.
    /// The third nearest in plain view, two hidden nearer, and hidden ones
    /// further out on slots either side of it: locked on the first look.
    #[test]
    fn a_target_among_the_nearest_is_asked_every_look() {
        let layout = [
            (8.0, false),
            (3.0, false),
            (4.0, false),
            (6.0, true),
            (10.0, false),
        ];
        let (locked, looks) = scan_field(&layout, 8, |_, _| true);
        assert_eq!((locked, looks), (Some(13), 1));
    }

    /// **Hidden alts taking turns at the range's edge do not steer the
    /// extra try off the raider** (review 5, claims#3). Past the three
    /// nearest, all hidden, the raider in the open has one hidden alt on a
    /// lower slot and one on a higher, and each look one of the two steps
    /// into range and the other out. A count-based turn (`turn % n`) took
    /// the alt every look, for good; the cursor only moves past a slot it
    /// asked, so the raider is reached on the second.
    #[test]
    fn hidden_alts_at_the_range_edge_do_not_steer_the_extra_try() {
        let layout = [
            (3.0, false),
            (4.0, false),
            (5.0, false),
            (6.0, false), // the low alt, id 13
            (9.0, true),  // the raider, id 14
            (7.0, false), // the high alt, id 15
        ];
        let (locked, looks) = scan_field(&layout, 8, |look, p| match p.id {
            13 => look % 2 == 0,
            15 => look % 2 == 1,
            _ => true,
        });
        assert_eq!(locked, Some(14), "the alts kept the raider unasked");
        assert!(looks <= 2, "{looks} looks to reach the raider");
    }
}
