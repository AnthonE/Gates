//! A player's auto turret (`NOW.md` §0aa 1): Rust's `AutoTurret`, unpowered
//! (there is no electricity here). A deployable with a box of rounds and a
//! gun on top that shoots anybody in reach and in sight who is not **its
//! owner or the crew of the claim it stands in**.
//!
//! ## What it reuses
//!
//! The gun is the town sentry's (`sentry.rs`): the same look, lock beep,
//! burst and bullet solve, through `sentry::look` and `sentry::fire`. And
//! like a sentry it rides the animal roster: turret `k` is roster slot
//! [`TURRET_SLOT0`]` + k`, species `mob::MOB_SENTRY`, a mob record whose yaw
//! is where the gun points. So the wire, the client's turret mesh, the
//! tracer and the lock-on beep all exist already.
//!
//! ## Who it spares, and why that is not its own list
//!
//! The reference hangs a third authorisation list on the turret
//! (`roster.rs`'s header). A list is state a save has to carry, and a new
//! saved list is a world-format change, which is a wipe. So v0 asks the
//! question building already answers: its owner, and whoever is on the crew
//! of every hearth whose claim covers it (`claim::foreign_claim`). A turret
//! outside any claim spares its owner alone.
//!
//! ## The table is derived, not kept
//!
//! [`reconcile`] walks the deploy store every look and gives each turret
//! record an entry (and a roster slot); an entry whose record is gone is
//! freed. Placement, a raid, decay, a pickup and a world load all arrive the
//! same way, with no hook in any of them. Past [`MAX_TURRETS`] a placed
//! turret stands inert until one comes down. Not saved, like a sentry's
//! brain: a restart forgets who it was after.
//!
//! ## Interference
//!
//! Rust's rule rather than a shard-wide one: of the turrets within
//! [`TURRET_INTERFERENCE_M`] of each other, the first
//! [`TURRET_INTERFERENCE`] in table order shoot and the rest stand idle,
//! guns up. A wall of turrets is a wall of twelve.

use crate::build::Pieces;
use crate::collide::ColIndex;
use crate::deploy::{box_key, DeployContent, Deploys, ARCH_TURRET};
use crate::gather::ItemStack;
use crate::heli::Round;
use crate::limits::{MAX_PLAYERS, MAX_TURRETS, TURRET_INTERFERENCE, TURRET_INTERFERENCE_M};
use crate::mob::Mob;
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};
use crate::nav;
use crate::occupy::Occupants;
use crate::ranged::{ARROW_EYE_MM, MM_PER_M};
use crate::sentry::{self, Sentry, SentryDef, LOOK_TICKS, SENTRY_SLOT0};
use crate::terrain::Haven;
use crate::world::{EventQueue, Player};

/// Turret `k`'s roster slot is `TURRET_SLOT0 + k`, just below the sentries.
pub const TURRET_SLOT0: usize = SENTRY_SLOT0 - MAX_TURRETS;

/// Whether a roster slot is a player turret's.
#[inline]
pub const fn is_turret_slot(slot: usize) -> bool {
    slot >= TURRET_SLOT0 && slot < SENTRY_SLOT0
}

/// The gun's height over the turret's base, metres.
pub const GUN_M: f32 = 0.9;

/// **Who a player's machine answers to**: its owner, and whoever is on the
/// crew of every hearth whose claim covers `(x, z)`. Outside any claim, the
/// owner alone. The turret spares exactly these; the vending machine
/// (`shop.rs`) lets exactly these stock it and set its offers.
pub fn spares(pieces: &Pieces, deploys: &Deploys, owner: u32, x: f32, z: f32, id: u32) -> bool {
    id == owner
        || spared_by_claim(
            pieces,
            deploys,
            crate::claim::any_claim(pieces, deploys, x, z),
            x,
            z,
            id,
        )
}

/// [`spares`]' crew half, with the coverage asked once by the caller (a
/// turret's look asks it of every player in range).
#[inline]
fn spared_by_claim(
    pieces: &Pieces,
    deploys: &Deploys,
    covered: bool,
    x: f32,
    z: f32,
    id: u32,
) -> bool {
    covered && !crate::claim::foreign_claim(pieces, deploys, x, z, id)
}

/// The baked `[turret]` table: the gun, and the round it spends out of its
/// own box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurretDef {
    pub gun: SentryDef,
    pub ammo: u16,
}

impl TurretDef {
    pub const INERT: Self = Self {
        gun: SentryDef::INERT,
        ammo: 0,
    };

    #[inline]
    pub fn armed(&self) -> bool {
        self.gun.armed() && self.ammo != 0
    }
}

/// One table entry: which deployable it is, and its gun's brain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Turret {
    pub live: bool,
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    pub owner: u32,
    pub gun: Sentry,
}

impl Turret {
    /// Every field, little-endian, in declaration order.
    pub fn hash_bytes(&self) -> [u8; 33] {
        let mut b = [0u8; 33];
        b[0] = self.live as u8;
        b[1..3].copy_from_slice(&self.cx.to_le_bytes());
        b[3..5].copy_from_slice(&self.cz.to_le_bytes());
        b[5] = self.level;
        b[6] = self.loc;
        b[7..11].copy_from_slice(&self.owner.to_le_bytes());
        b[11..33].copy_from_slice(&self.gun.hash_bytes());
        b
    }

    fn holds(&self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        self.live && self.cx == cx && self.cz == cz && self.level == level && self.loc == loc
    }
}

/// Bring the table in line with the deploy store: free an entry whose
/// turret is gone, then give every turret record without one the first free
/// entry, in store order. Bounded: `MAX_TURRETS` finds and one store walk.
pub fn reconcile(
    dc: &DeployContent,
    deploys: &Deploys,
    table: &mut [Turret; MAX_TURRETS],
    slots: &mut [Mob],
) {
    let is_turret =
        |row: u8| (row as usize) < dc.defs.len() && dc.defs[row as usize].arch == ARCH_TURRET;
    for (t, m) in table.iter_mut().zip(slots.iter_mut()) {
        if t.live
            && !deploys
                .find(t.cx, t.cz, t.level, t.loc)
                .is_some_and(|d| is_turret(d.row))
        {
            *t = Turret::default();
            m.alive = false;
        }
    }
    for d in deploys.entries() {
        if !is_turret(d.row) || table.iter().any(|t| t.holds(d.cx, d.cz, d.level, d.loc)) {
            continue;
        }
        let Some(k) = table.iter().position(|t| !t.live) else {
            return;
        };
        table[k] = Turret {
            live: true,
            cx: d.cx,
            cz: d.cz,
            level: d.level,
            loc: d.loc,
            owner: d.owner,
            gun: Sentry::default(),
        };
        // It starts facing the way it was placed.
        slots[k].yaw = (d.pose.yaw as u16) << 8;
    }
}

/// One tick of every player turret. A round that landed on somebody is
/// written to `rounds` for `World::tick` to land (`sentry::step`'s split);
/// each round fired, hit or miss, spends one of `def.ammo` from the box.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    haven: &Haven,
    tick: u64,
    def: &TurretDef,
    dc: &DeployContent,
    pieces: &Pieces,
    occ: &mut Occupants,
    deploys: &mut Deploys,
    table: &mut [Turret; MAX_TURRETS],
    slots: &mut [Mob],
    players: &[Player; MAX_PLAYERS],
    events: &mut EventQueue,
    rounds: &mut [Option<Round>; MAX_TURRETS],
) {
    *rounds = [None; MAX_TURRETS];
    if !def.armed() {
        for (t, m) in table.iter_mut().zip(slots.iter_mut()) {
            *t = Turret::default();
            m.alive = false;
        }
        return;
    }
    let cols: &ColIndex = pieces.cols();
    if tick.is_multiple_of(LOOK_TICKS) {
        reconcile(dc, deploys, table, slots);
    }
    let mut recs = [None; MAX_TURRETS];
    for (r, t) in recs.iter_mut().zip(table.iter()) {
        if t.live {
            *r = deploys.find(t.cx, t.cz, t.level, t.loc).copied();
        }
    }
    for k in 0..MAX_TURRETS {
        let t = table[k];
        if !t.live {
            continue;
        }
        let Some(rec) = recs[k] else {
            continue;
        };
        let (x, z) = rec.xz();
        let gun_y = crate::deploy::body_base_y(seed, haven, cols, &rec, ARCH_TURRET) + GUN_M;
        let slot = &mut slots[k];
        // The record's feet sit `ARROW_EYE_MM` under the muzzle, the
        // sentry's convention, so the client draws the round from the gun.
        slot.body = Body {
            qx: (x / POS_XZ_Q) as i32,
            qy: ((gun_y * MM_PER_M - ARROW_EYE_MM as f32) / (POS_Y_Q * MM_PER_M)) as i32,
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
        let bi = deploys.box_index(box_key(t.cx, t.cz, t.level, t.loc));
        let rounds_in = |deploys: &Deploys| {
            bi.map_or(0u32, |i| {
                (0..crate::limits::BOX_SLOTS)
                    .map(|s| deploys.box_slot(i, s))
                    .filter(|s| s.count > 0 && s.item == def.ammo)
                    .map(|s| s.count as u32)
                    .sum()
            })
        };
        let s = &mut table[k].gun;
        // A dry gun lets its target go and looks at nobody; so does one
        // jammed by the turrets around it (module doc).
        if rounds_in(deploys) == 0 || jammed(&recs, k, (x, z)) {
            *s = Sentry::default();
            continue;
        }
        // Looks are spread over the cadence by index, so the turrets do not
        // all walk their sight lines on one tick.
        if (tick + k as u64).is_multiple_of(LOOK_TICKS) {
            let (owner, dref) = (t.owner, &*deploys);
            let covered = crate::claim::any_claim(pieces, dref, x, z);
            let wanted = |p: &Player| {
                p.active
                    && !p.dead
                    && !p.sleeping
                    && p.hp > 0
                    && !crate::combat::protected(p)
                    && p.id != owner
                    && !spared_by_claim(pieces, dref, covered, x, z, p.id)
            };
            sentry::look(
                seed,
                haven,
                tick,
                &def.gun,
                cols,
                occ,
                s,
                TURRET_SLOT0 + k,
                gun,
                players,
                events,
                &wanted,
            );
        }
        if s.engaged() {
            let p = &players[s.target as usize];
            let (px, pz) = (
                p.body.qx as f32 * POS_XZ_Q * MM_PER_M,
                p.body.qz as f32 * POS_XZ_Q * MM_PER_M,
            );
            let want = nav::yaw_toward(px - gun.0, pz - gun.2, slot.yaw);
            slot.yaw = nav::turn_toward(slot.yaw, want, sentry::TURN_STEP);
        }
        let before = s.next_shot;
        rounds[k] = sentry::fire(
            seed,
            haven,
            tick,
            &def.gun,
            cols,
            occ,
            s,
            (TURRET_SLOT0 + k, (TURRET_SLOT0 + k) as i32),
            gun,
            slot,
            players,
            events,
        );
        if s.next_shot != before {
            if let Some(i) = bi {
                spend_one(deploys, i, def.ammo);
            }
        }
    }
}

/// Whether turret `k` is past [`TURRET_INTERFERENCE`] among the turrets
/// within [`TURRET_INTERFERENCE_M`] of it: the ones before it in table order
/// win, which is stable for as long as they all stand.
fn jammed(
    recs: &[Option<crate::deploy::DeployRec>; MAX_TURRETS],
    k: usize,
    at: (f32, f32),
) -> bool {
    let r2 = TURRET_INTERFERENCE_M * TURRET_INTERFERENCE_M;
    recs[..k]
        .iter()
        .flatten()
        .filter(|r| {
            let (x, z) = r.xz();
            let (dx, dz) = (x - at.0, z - at.1);
            dx * dx + dz * dz <= r2
        })
        .count()
        >= TURRET_INTERFERENCE
}

/// Take one `item` out of box `i`, from the first stack holding it.
fn spend_one(deploys: &mut Deploys, i: usize, item: u16) {
    for s in 0..crate::limits::BOX_SLOTS {
        let mut st = deploys.box_slot(i, s);
        if st.count > 0 && st.item == item {
            st.count -= 1;
            if st.count == 0 {
                st = ItemStack::default();
            }
            deploys.set_box_slot(i, s, st);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_turret_slots_sit_under_the_sentries_and_are_guns() {
        for k in 0..MAX_TURRETS {
            let slot = TURRET_SLOT0 + k;
            assert!(is_turret_slot(slot));
            assert!(!sentry::is_sentry_slot(slot));
            assert_eq!(crate::mob::kind_of(slot), crate::mob::MOB_SENTRY);
            assert_eq!(crate::mob::ordinal(slot), None);
        }
        assert!(!is_turret_slot(TURRET_SLOT0 - 1));
        assert!(!is_turret_slot(SENTRY_SLOT0));
    }

    /// Rust's interference: fourteen turrets in one base, and the first
    /// twelve in table order shoot; a fifteenth across the island is its
    /// own base and shoots too.
    #[test]
    fn past_twelve_turrets_within_forty_metres_the_rest_stand_idle() {
        let mut recs = [None; MAX_TURRETS];
        for (k, r) in recs.iter_mut().take(14).enumerate() {
            *r = Some(crate::deploy::DeployRec {
                cx: 500 + (k % 4) as u16,
                cz: 500 + (k / 4) as u16,
                ..Default::default()
            });
        }
        recs[20] = Some(crate::deploy::DeployRec {
            cx: 900,
            cz: 900,
            ..Default::default()
        });
        let at = |k: usize| recs[k].unwrap().xz();
        for k in 0..14 {
            assert_eq!(
                jammed(&recs, k, at(k)),
                k >= TURRET_INTERFERENCE,
                "turret {k}"
            );
        }
        assert!(!jammed(&recs, 20, at(20)), "a far base is its own");
    }
}
