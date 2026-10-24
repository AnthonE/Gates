//! Bullets that fly — Rust's gunplay, where a round has travel time and drop.
//!
//! A firearm used to resolve its whole reach on the tick the trigger broke
//! (`ranged::hitscan`). Rust's rounds are projectiles: a 5.56 round leaves at
//! 375 m/s, so a target at 150 m is hit 0.4 s after the shot and a metre
//! under the line, and a player leads a runner and holds over at range. A
//! firearm whose round carries ballistics (`AmmoDef::speed_mmpt > 0`) now
//! launches one of these per pellet; a round with none still resolves
//! instantly, which is every fixture written before this module and every
//! weapon that is not a gun.
//!
//! # It is an arrow's flight, in the shooter's timeline
//!
//! The integration is the arrow's exactly — millimetres and millimetres per
//! tick, `v.y -= drop; q += v`, all integers — and each tick's segment asks
//! the arrow's two questions (`world_stop`, `nearest_body`) at the arrow's
//! spacing, so a bullet and an arrow cannot disagree about what a trunk is.
//!
//! The difference is **which bodies it is asked about**. An arrow resolves
//! against present-tick bodies (`ranged::Pose::Live`); a bullet keeps the
//! lag-compensation favour its trigger was granted for its whole flight and
//! resolves every tick against bodies rewound by it (`Pose::Rewound`). That
//! is the shooter's screen: they see everyone else that many ticks late and
//! their own round fly in real time from the moment it left, so the round
//! that crosses a body on their screen crosses it here. The arrow's
//! objection to rewinding (`Pose`'s doc) is to rewinding *part* of a flight
//! launched in another timeline; a bullet is in one timeline from muzzle to
//! stop.
//!
//! # Pellets of one shot land as one blow
//!
//! A shotgun's pellets fly side by side and reach a body on the same tick.
//! Hits are gathered per tick and summed per (shooter, volley, body), so a
//! shell that puts nine pellets into a chest is one `EV_HIT` for the total
//! at the worst part any of them crossed — the hitscan's rule, kept.

use crate::collide::{ColIndex, Part};
use crate::combat::CombatContent;
use crate::limits::{
    ARROW_STEP_MM, MAX_ARROWS, MAX_BULLETS, MAX_BULLETS_PER_OWNER, MAX_BULLET_LIFE_TICKS,
    MAX_BULLET_SUBSTEPS, MAX_PLAYERS,
};
use crate::movement::POS_Y_Q;
use crate::occupy::Occupants;
use crate::ranged::{
    nearest_body, part_crossed, push_impact, world_stop, BodyHit, Boom, Chip, Ignite, Kill, Lit,
    MobShot, Pose, Quarry, ARROW_R_M, IMPACT_BULLET, MM_PER_M,
};
use crate::rewind::Rewind;
use crate::terrain;
use crate::world::{EventQueue, Player, EV_HEALTH, EV_HIT, EV_HURT};

/// One round in flight. `life == 0` ⇔ the slot is free, which is also what
/// keeps it out of `state_hash`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bullet {
    /// Position, millimetres.
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// Velocity, millimetres per tick.
    pub vx: i32,
    pub vy: i32,
    pub vz: i32,
    /// Gravity, mm/tick², carried on the round for the arrow's reason: a
    /// round in the air does not change because content was rebaked.
    pub drop: u16,
    /// The shooter's network id.
    pub owner: u32,
    /// The gun, for the death screen.
    pub item: u16,
    /// The round, for what it does where it stops (fire, blast).
    pub round: u16,
    /// Body damage per pellet, the round's `damage_pct` already applied.
    pub damage: u16,
    pub structure: u16,
    pub head_pct: u16,
    pub limb_pct: u16,
    /// The gun's falloff (`combat::Gun::falloff_mm` and `falloff_pct`),
    /// read against the distance flown.
    pub falloff_mm: [u32; 2],
    pub falloff_pct: u16,
    /// The lag-comp favour the trigger was granted, kept for the flight.
    pub back: u8,
    /// The low half of the tick it was fired on: pellets of one shot share
    /// it, and that is what merges them into one blow.
    pub volley: u32,
    /// Millimetres it may fly before it is spent (the gun's `range_m`).
    pub reach: u32,
    /// Millimetres flown so far, the arc's length.
    pub flown: u32,
    /// Ticks of flight left; zero is a free slot.
    pub life: u16,
    /// The volley's first pellet: the one that carries the round's fire or
    /// blast, so a shell of incendiary buckshot lights one fire, not twelve.
    pub lead: bool,
}

impl Bullet {
    #[inline]
    fn active(&self) -> bool {
        self.life > 0
    }
}

/// One body a tick's rounds reached, pellets of a volley summed.
#[derive(Clone, Copy, Debug)]
struct Blow {
    owner: u32,
    volley: u32,
    slot: usize,
    item: u16,
    damage: u32,
    part: Part,
    range_cm: u16,
    /// The first pellet's horizontal velocity, for the victim's hurt arc.
    dx: i32,
    dz: i32,
}

const NO_BLOW: Blow = Blow {
    owner: 0,
    volley: 0,
    slot: 0,
    item: 0,
    damage: 0,
    part: Part::Limb,
    range_cm: 0,
    dx: 0,
    dz: 0,
};

/// Every round in the air on the shard, and the tick's scratch for the
/// blows they land. Both arrays are boxed: together they are ~200 kB, and
/// wasm's stack is 1 MiB (`CLAUDE.md`'s trap list).
#[derive(Clone, Debug)]
pub struct Bullets {
    b: Box<[Bullet; MAX_BULLETS]>,
    blows: Box<[Blow; MAX_BULLETS]>,
}

impl Bullets {
    pub fn new() -> Self {
        Self {
            b: crate::boxed_array(Bullet::default()),
            blows: crate::boxed_array(NO_BLOW),
        }
    }

    /// The rounds actually in the air, in slot order.
    pub fn entries(&self) -> impl Iterator<Item = &Bullet> {
        self.b.iter().filter(|b| b.active())
    }

    pub fn len(&self) -> usize {
        self.entries().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `owner` may put `n` more rounds in the air: the store has
    /// `n` free slots and the owner stays inside `MAX_BULLETS_PER_OWNER`.
    pub fn room_for(&self, owner: u32, n: usize) -> bool {
        let mut free = 0usize;
        let mut theirs = 0usize;
        for b in self.b.iter() {
            if !b.active() {
                free += 1;
            } else if b.owner == owner {
                theirs += 1;
            }
        }
        free >= n && theirs + n <= MAX_BULLETS_PER_OWNER
    }

    /// Put a round in the first free slot. The caller has asked
    /// [`Bullets::room_for`]; a full store drops it.
    pub fn launch(&mut self, b: Bullet) {
        if let Some(slot) = self.b.iter_mut().find(|s| !s.active()) {
            *slot = b;
        }
    }
}

impl Default for Bullets {
    fn default() -> Self {
        Self::new()
    }
}

/// A round's starting state: from `o` (millimetres) along the unit `dir` at
/// `speed` mm/tick. Rounded to the millimetre, once, here.
pub fn velocity(dir: (f32, f32, f32), speed: u16) -> (i32, i32, i32) {
    let s = f32::from(speed);
    (
        crate::fmath::floor_i32(dir.0 * s + 0.5),
        crate::fmath::floor_i32(dir.1 * s + 0.5),
        crate::fmath::floor_i32(dir.2 * s + 0.5),
    )
}

/// One tick of every round in the air. Returns how many entries of `kills`
/// and `chips` were written; animals a round met go to `quarry`, fire and
/// blast rounds that came down go to `lit` for `World` to light and blow.
#[allow(clippy::too_many_arguments)]
pub fn step(
    seed: u64,
    tick: u64,
    haven: &terrain::Haven,
    cols: &ColIndex,
    occ: &mut Occupants,
    rewind: &Rewind,
    cc: &CombatContent,
    bullets: &mut Bullets,
    lit: &mut Lit,
    players: &mut [Player; MAX_PLAYERS],
    events: &mut EventQueue,
    kills: &mut [Kill; MAX_ARROWS],
    chips: &mut [Chip; MAX_ARROWS],
    mut quarry: Option<&mut Quarry>,
) -> (usize, usize) {
    let mut n_chips = 0usize;
    let mut n_blows = 0usize;
    for ix in 0..MAX_BULLETS {
        let mut b = bullets.b[ix];
        if !b.active() {
            continue;
        }
        b.vy -= i32::from(b.drop);
        let (dx, dy, dz) = (b.vx, b.vy, b.vz);
        let full = {
            let (fx, fy, fz) = (dx as f32, dy as f32, dz as f32);
            (fx * fx + fy * fy + fz * fz).sqrt()
        };
        // The last tick of a round's reach flies only what is left of it.
        let left = b.reach.saturating_sub(b.flown) as f32;
        let (frac, spent) = if full > left {
            (left / full.max(1.0), true)
        } else {
            (1.0, false)
        };
        let len = full * frac;
        let need = (len / ARROW_STEP_MM as f32) as usize + 1;
        if need > MAX_BULLET_SUBSTEPS {
            // Faster than the sampler can be honest about; `bake_ammo`
            // refuses such a round, so this is unreachable with content.
            bullets.b[ix].life = 0;
            continue;
        }
        let o = (b.qx as f32, b.qy as f32, b.qz as f32);
        let s = (dx as f32 * frac, dy as f32 * frac, dz as f32 * frac);
        let (stop_t, surf, built) = world_stop(seed, haven, cols, occ, o, s, need, need, ARROW_R_M);
        let pose = Pose::Rewound {
            rewind,
            tick,
            back: b.back,
        };
        let best = nearest_body(players, o, s, stop_t, b.owner, pose);
        let at = |t: f32| (o.0 + s.0 * t, o.1 + s.1 * t, o.2 + s.2 * t);
        let dist = |t: f32| b.flown + (len * t) as u32;

        if let Some(q) = quarry.as_deref_mut() {
            let beast = q
                .first(o, s)
                .filter(|m| m.t <= stop_t && best.is_none_or(|h| m.t < h.enter));
            if let Some(m) = beast {
                q.push(MobShot {
                    slot: m.slot,
                    by: b.owner,
                    damage: crate::combat::fall_off(
                        b.falloff_mm,
                        b.falloff_pct,
                        b.damage,
                        dist(m.t),
                    ),
                });
                come_down(cc, lit, seed, tick, ix, &b, at(m.t), None, false);
                bullets.b[ix].life = 0;
                continue;
            }
        }

        if let Some(BodyHit {
            t,
            slot: j,
            enter,
            exit,
            qy: feet_q,
            crouched: struck_low,
            part: lying_part,
        }) = best
        {
            let shooter = players.iter().position(|p| p.active && p.id == b.owner);
            let shield = match shooter {
                Some(sh) if sh != j => {
                    let (a, v) = crate::combat::pair_mut(players, sh, j);
                    crate::combat::shielded(a, v)
                }
                _ => crate::combat::protected(&players[j]),
            };
            if !shield {
                let feet_mm = feet_q as f32 * (POS_Y_Q * MM_PER_M);
                let part = lying_part.unwrap_or_else(|| {
                    part_crossed(o.1, s.1, feet_mm, enter, exit.min(stop_t), struck_low)
                });
                let raw = crate::combat::part_damage(b.damage, part, b.head_pct, b.limb_pct);
                let dmg = crate::combat::fall_off(b.falloff_mm, b.falloff_pct, raw, dist(t));
                let range_cm = (dist(t) / 10).min(u32::from(u16::MAX)) as u16;
                let same = bullets.blows[..n_blows]
                    .iter_mut()
                    .find(|w| w.owner == b.owner && w.volley == b.volley && w.slot == j);
                match same {
                    Some(w) => {
                        w.damage = w.damage.saturating_add(u32::from(dmg));
                        w.part = w.part.max(part);
                        w.range_cm = w.range_cm.min(range_cm);
                    }
                    None => {
                        bullets.blows[n_blows] = Blow {
                            owner: b.owner,
                            volley: b.volley,
                            slot: j,
                            item: b.item,
                            damage: u32::from(dmg),
                            part,
                            range_cm,
                            dx,
                            dz,
                        };
                        n_blows += 1;
                    }
                }
            }
            come_down(cc, lit, seed, tick, ix, &b, at(t), None, false);
            bullets.b[ix].life = 0;
            continue;
        }

        if let Some(kind) = surf {
            push_impact(events, kind, IMPACT_BULLET, o, s, stop_t);
            if let (Some(hit), true) = (built, b.structure > 0) {
                let from = (o.0 / MM_PER_M, o.2 / MM_PER_M);
                let cap = chips.len();
                let same = chips[..n_chips]
                    .iter_mut()
                    .find(|c| c.by == b.owner && c.hit == hit.at && c.deploy == hit.deploy);
                match same {
                    Some(c) => c.structure = c.structure.saturating_add(b.structure),
                    None if n_chips < cap => {
                        chips[n_chips] = Chip {
                            hit: hit.at,
                            deploy: hit.deploy,
                            structure: b.structure,
                            from_x: from.0,
                            from_z: from.1,
                            by: b.owner,
                        };
                        n_chips += 1;
                    }
                    None => {}
                }
            }
            // Just short of the face, so a blast or a flame starts on the
            // shooter's side of it.
            let back = (stop_t - 1.0 / need as f32).max(0.0);
            come_down(cc, lit, seed, tick, ix, &b, at(back), built, true);
            bullets.b[ix].life = 0;
            continue;
        }

        b.qx += crate::fmath::floor_i32(s.0 + 0.5);
        b.qy += crate::fmath::floor_i32(s.1 + 0.5);
        b.qz += crate::fmath::floor_i32(s.2 + 0.5);
        b.flown = b.flown.saturating_add(len as u32);
        b.life -= 1;
        if spent {
            b.life = 0;
        }
        bullets.b[ix] = b;
    }

    // The tick's blows, in the order their first pellet landed.
    let mut n_kills = 0usize;
    let mut downed = [false; MAX_PLAYERS];
    for k in 0..n_blows {
        let w = bullets.blows[k];
        let j = w.slot;
        if downed[j] {
            continue;
        }
        let v = &mut players[j];
        let dmg = w.damage.min(u32::from(u16::MAX)) as u16;
        let hurt = crate::combat::hurt(cc, v, dmg);
        let (vid, left, vmax) = (v.id, hurt.left as u32, v.hp_max as u32);
        events.push(EV_HIT, w.owner, vid, crate::world::hit_c(w.part, dmg));
        events.push(
            EV_HURT,
            vid,
            crate::combat::bearing_sector(-(w.dx as i64), -(w.dz as i64)) as u32,
            crate::world::hurt_c(w.part, dmg),
        );
        events.push(EV_HEALTH, vid, left, vmax);
        if hurt.died && n_kills < kills.len() {
            downed[j] = true;
            kills[n_kills] = Kill {
                victim: j,
                by: w.owner,
                item: w.item,
                range_cm: w.range_cm,
                head: w.part == Part::Head,
            };
            n_kills += 1;
        }
    }
    (n_kills, n_chips)
}

/// What a round does where it stops, past the hole it makes: an explosive
/// round blows (anywhere it stops), an incendiary one lights a fire (only
/// on the world — a round in a body burns nobody's floor).
#[allow(clippy::too_many_arguments)]
fn come_down(
    cc: &CombatContent,
    lit: &mut Lit,
    seed: u64,
    tick: u64,
    slot: usize,
    b: &Bullet,
    at: (f32, f32, f32),
    struck: Option<crate::ranged::Struck>,
    on_world: bool,
) {
    let Some(def) = cc.ammo_def(b.round).filter(|_| b.lead) else {
        return;
    };
    let (qx, qy, qz) = (
        crate::fmath::floor_i32(at.0),
        crate::fmath::floor_i32(at.1),
        crate::fmath::floor_i32(at.2),
    );
    if def.blast.blast_cm > 0 {
        lit.boom(Boom {
            qx,
            qy,
            qz,
            owner: b.owner,
            item: b.item,
            blast: def.blast,
            struck,
        });
    } else if on_world && def.fire_ticks[1] > 0 {
        lit.push(Ignite {
            qx,
            qy,
            qz,
            owner: b.owner,
            item: b.item,
            ticks: crate::ranged::fire_ticks(seed, tick, slot, def.fire_ticks),
            range_cm: (b.flown / 10).min(u32::from(u16::MAX)) as u16,
        });
    }
}

/// Ticks a round at `speed` mm/tick needs to fly `reach` mm, its whole
/// life, capped at [`MAX_BULLET_LIFE_TICKS`].
pub fn life_ticks(reach: u32, speed: u16) -> u16 {
    let ticks = reach.div_ceil(u32::from(speed.max(1))) + 1;
    ticks.min(u32::from(MAX_BULLET_LIFE_TICKS)) as u16
}
