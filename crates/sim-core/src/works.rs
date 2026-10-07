//! The works (`ARC.md` F1–F3): the island's broken machines, which the whole
//! server switches back on by pouring materials into them.
//!
//! A work is **sealed** until its opening hour, then **open** to deposits.
//! When every input on its quota is met it is **lit**:
//! - its *floor* unlock is set for the rest of the wipe;
//! - its tank is filled, so its *ceiling* unlock holds while there is fuel;
//! - the fuel burns every hour, slower on a quiet shard, and anyone may top
//!   it up.
//!
//! An unpaid work lights itself at its fallback hour with an empty tank, so
//! the floor is guaranteed and only the ceiling is contested
//! (`WORLD.md` §4.3).
//!
//! Every unlock is global: it is a bit in [`Works::unlocks`], and a recipe
//! names the bit it needs (`craft.rs`). Effects are content rows turning a
//! knob code ([`knob_pct`]), never a `match` on which work it was
//! (`WORLD.md` §5.4).
//!
//! The arc clock is the world's own tick: a wipe is a fresh world, which
//! starts at tick 0, and a saved world keeps its tick (`worldsave.rs`).

use crate::craft::{inv_count, inv_take};
use crate::limits::{
    MAX_ARC_EFFECTS, MAX_UNLOCKS, MAX_WORKS, MAX_WORK_CREDITS, MAX_WORK_INPUTS, TICK_HZ,
};
use crate::spot::Spot;
use crate::terrain::Haven;
use crate::world::{EventQueue, Player, EV_ARC_REFUSED, EV_WORK};

/// Not open yet.
pub const WORK_SEALED: u8 = 0;
/// Taking deposits.
pub const WORK_OPEN: u8 = 1;
/// Burning: its floor holds, and its ceiling while the tank has fuel.
pub const WORK_LIT: u8 = 2;

/// `EV_WORK.b`: it opened for deposits.
pub const WORK_EV_OPENED: u32 = 1;
/// Its quota was met; `c` is who put the last of it in.
pub const WORK_EV_LIT: u32 = 2;
/// Nobody paid, and its fallback hour lit it with an empty tank.
pub const WORK_EV_FALLBACK: u32 = 3;
/// Its tank ran dry: the ceiling is off.
pub const WORK_EV_EMBERS: u32 = 4;
/// Fuel went into a dry tank; `c` is who.
pub const WORK_EV_REKINDLED: u32 = 5;
pub const WORK_EV_MAX: u32 = WORK_EV_REKINDLED;

/// `Command::Arc` ops. One action carries every arc verb (the action lane
/// has three codes left, `protocol` `the_action_lane_has_the_room_it_claims`).
/// Put what you carry toward work `target`'s quota: input `arg`, or every
/// input with [`ARG_ALL`].
pub const OP_DEPOSIT: u8 = 0;
/// Top up lit work `target`'s tank.
pub const OP_FUEL: u8 = 1;
pub const OP_MAX: u8 = OP_FUEL;
/// `arg` for every input at once.
pub const ARG_ALL: u8 = 0xFF;

/// Refusals, `EV_ARC_REFUSED.b`. One family for every arc verb.
pub const REFUSE_A_KIND: u32 = 1;
/// Not at the work.
pub const REFUSE_A_REACH: u32 = 2;
/// Not open yet.
pub const REFUSE_A_SEALED: u32 = 3;
/// You carry nothing it still needs.
pub const REFUSE_A_NOTHING: u32 = 4;
/// Already burning: it takes fuel now, not its quota.
pub const REFUSE_A_LIT: u32 = 5;
/// Fuel for a work that is not burning yet.
pub const REFUSE_A_COLD: u32 = 6;
/// The tank is full.
pub const REFUSE_A_FULL: u32 = 7;
pub const REFUSE_A_MAX: u32 = REFUSE_A_FULL;

/// How close to a work's terminal a deposit must be, metres.
pub const WORK_REACH_M: f32 = 4.0;
/// How often works open, light and burn: every ten seconds.
pub const WORKS_PERIOD_TICKS: u64 = 10 * TICK_HZ as u64;
/// Which tick of the period, so the sweep does not share one with the
/// oven's stride.
const WORKS_PHASE: u64 = 7;
/// Periods an hour.
const PERIODS_PER_HOUR: u32 = (3600 * TICK_HZ as u64 / WORKS_PERIOD_TICKS) as u32;

/// An unlock code: bit + 1, so a recipe's zero means "needs nothing".
pub const NO_UNLOCK: u8 = 0;

/// Knob codes a ceiling can turn, by per cent (100 = unchanged).
/// Furnace smelting speed (`oven.rs`).
pub const KNOB_SMELT_PCT: u8 = 0;
pub const KNOB_MAX: u8 = KNOB_SMELT_PCT;

/// One quota line: `need` of `item`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkInput {
    pub item: u16,
    pub need: u32,
}

/// One work, baked from `content/arc.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkDef {
    /// Where its terminal stands.
    pub spot: Spot,
    /// The act it belongs to (`ARC.md` §2).
    pub act: u8,
    /// The tick deposits open.
    pub opens_at: u64,
    /// The tick it lights itself if nobody paid.
    pub fallback_at: u64,
    /// Unlock codes, `NO_UNLOCK` for none.
    pub floor: u8,
    pub ceiling: u8,
    /// What its tank burns, how much it holds, and how much an hour at a
    /// busy shard.
    pub fuel: u16,
    pub fuel_max: u32,
    pub burn_per_hour: u32,
    pub n_inputs: u8,
    pub inputs: [WorkInput; MAX_WORK_INPUTS],
}

/// One effect: while `unlock` is set, `knob` runs at `pct` per cent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArcEffect {
    pub unlock: u8,
    pub knob: u8,
    pub pct: u16,
}

/// Every work and effect on the shard. Construction input like every
/// content table: `EMPTY` holds no works, so nothing is ever gated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorksContent {
    pub defs: [WorkDef; MAX_WORKS],
    pub count: u8,
    pub effects: [ArcEffect; MAX_ARC_EFFECTS],
    pub n_effects: u8,
    /// At or below this many players a tank burns at `quiet_burn_pct`; at
    /// or above `busy_players`, at full rate; linear between.
    pub quiet_players: u16,
    pub busy_players: u16,
    pub quiet_burn_pct: u16,
}

impl WorksContent {
    pub const EMPTY: WorksContent = WorksContent {
        defs: [WorkDef {
            spot: Spot {
                site: 0,
                nth: 0,
                x_cm: 0,
                y_cm: 0,
                z_cm: 0,
            },
            act: 0,
            opens_at: 0,
            fallback_at: 0,
            floor: NO_UNLOCK,
            ceiling: NO_UNLOCK,
            fuel: 0,
            fuel_max: 0,
            burn_per_hour: 0,
            n_inputs: 0,
            inputs: [WorkInput { item: 0, need: 0 }; MAX_WORK_INPUTS],
        }; MAX_WORKS],
        count: 0,
        effects: [ArcEffect {
            unlock: NO_UNLOCK,
            knob: 0,
            pct: 100,
        }; MAX_ARC_EFFECTS],
        n_effects: 0,
        quiet_players: 10,
        busy_players: 100,
        quiet_burn_pct: 100,
    };

    /// One work over the gather fixture's items, for the gates: it opens at
    /// once, takes 6 of item 0 and 2 of item 1, falls back after an hour,
    /// floors unlock 1 and ceilings unlock 2, and burns item 1.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.count = 1;
        c.defs[0] = WorkDef {
            spot: Spot {
                site: crate::spot::SITE_TOWN,
                nth: 0,
                x_cm: 0,
                y_cm: 0,
                z_cm: 0,
            },
            act: 2,
            opens_at: 0,
            fallback_at: 3600 * TICK_HZ as u64,
            floor: 1,
            ceiling: 2,
            fuel: 1,
            fuel_max: 10,
            burn_per_hour: 6,
            n_inputs: 2,
            inputs: [
                WorkInput { item: 0, need: 6 },
                WorkInput { item: 1, need: 2 },
                WorkInput::default(),
                WorkInput::default(),
            ],
        };
        c.n_effects = 1;
        c.effects[0] = ArcEffect {
            unlock: 2,
            knob: KNOB_SMELT_PCT,
            pct: 200,
        };
        c
    }

    pub fn get(&self, k: usize) -> Option<&WorkDef> {
        (k < self.count as usize).then(|| &self.defs[k])
    }
}

/// One credited giver: a player id and their share in basis points of the
/// quota (a whole quota is 10,000).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Credit {
    pub id: u32,
    pub points: u32,
}

/// One work's state. Sim state: hashed and saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub state: u8,
    pub fuel: u32,
    /// The burn's remainder, in fuel × 100 per cent × periods an hour, so a
    /// tank burning a fraction of a unit a period loses nothing to rounding.
    pub burn_acc: u32,
    pub lit_at: u64,
    pub got: [u32; MAX_WORK_INPUTS],
    pub credits: [Credit; MAX_WORK_CREDITS],
}

/// Bytes [`Work::to_bytes`] writes: the save and the hash.
pub const WORK_BYTES: usize = 1 + 4 + 4 + 8 + 4 * MAX_WORK_INPUTS + 8 * MAX_WORK_CREDITS;

impl Work {
    pub fn to_bytes(&self) -> [u8; WORK_BYTES] {
        let mut b = [0u8; WORK_BYTES];
        b[0] = self.state;
        b[1..5].copy_from_slice(&self.fuel.to_le_bytes());
        b[5..9].copy_from_slice(&self.burn_acc.to_le_bytes());
        b[9..17].copy_from_slice(&self.lit_at.to_le_bytes());
        let mut at = 17;
        for g in self.got {
            b[at..at + 4].copy_from_slice(&g.to_le_bytes());
            at += 4;
        }
        for c in self.credits {
            b[at..at + 4].copy_from_slice(&c.id.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&c.points.to_le_bytes());
            at += 8;
        }
        b
    }

    pub fn from_bytes(b: &[u8; WORK_BYTES]) -> Work {
        let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"));
        let mut w = Work {
            state: b[0],
            fuel: u32_at(1),
            burn_acc: u32_at(5),
            lit_at: u64::from_le_bytes(b[9..17].try_into().expect("8 bytes")),
            ..Work::default()
        };
        let mut at = 17;
        for g in w.got.iter_mut() {
            *g = u32_at(at);
            at += 4;
        }
        for c in w.credits.iter_mut() {
            c.id = u32_at(at);
            c.points = u32_at(at + 4);
            at += 8;
        }
        w
    }

    /// The share `id` is credited with, basis points.
    pub fn points_of(&self, id: u32) -> u32 {
        self.credits
            .iter()
            .find(|c| c.points > 0 && c.id == id)
            .map_or(0, |c| c.points)
    }

    fn credit(&mut self, id: u32, points: u32) {
        if points == 0 {
            return;
        }
        if let Some(c) = self.credits.iter_mut().find(|c| c.points > 0 && c.id == id) {
            c.points = c.points.saturating_add(points);
            return;
        }
        if let Some(c) = self.credits.iter_mut().find(|c| c.points == 0) {
            *c = Credit { id, points };
            return;
        }
        let mut min = 0;
        for (i, c) in self.credits.iter().enumerate() {
            if c.points < self.credits[min].points {
                min = i;
            }
        }
        if self.credits[min].points < points {
            self.credits[min] = Credit { id, points };
        }
    }
}

/// Every work's state, slot for slot with [`WorksContent::defs`], plus the
/// unlocks they hold. Boxed in `World` (the stack posture `mobs` takes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Works {
    pub w: [Work; MAX_WORKS],
    /// Derived from `w` by [`Works::refresh`]: bit `u - 1` for every unlock
    /// code `u` a lit work holds. Not hashed or saved separately.
    pub unlocks: u32,
}

const _: () = assert!(MAX_UNLOCKS <= 32, "Works::unlocks is a u32");

impl Default for Works {
    fn default() -> Self {
        Works {
            w: [Work::default(); MAX_WORKS],
            unlocks: 0,
        }
    }
}

impl Works {
    /// Whether any work has moved off its fresh state: the hash folds the
    /// table only then, so a world with no works hashes as it always did.
    pub fn is_fresh(&self) -> bool {
        self.w.iter().all(|w| *w == Work::default())
    }

    /// Recompute `unlocks` from the works.
    pub fn refresh(&mut self, wc: &WorksContent) {
        let mut bits = 0u32;
        for (def, w) in wc.defs.iter().zip(self.w.iter()).take(wc.count as usize) {
            if w.state != WORK_LIT {
                continue;
            }
            if def.floor != NO_UNLOCK {
                bits |= 1 << (def.floor - 1);
            }
            if def.ceiling != NO_UNLOCK && w.fuel > 0 {
                bits |= 1 << (def.ceiling - 1);
            }
        }
        self.unlocks = bits;
    }

    /// The act the shard is in: the highest act of any work that has opened,
    /// and act I before any has.
    pub fn act(&self, wc: &WorksContent) -> u8 {
        let mut act = 1;
        for (def, w) in wc.defs.iter().zip(self.w.iter()).take(wc.count as usize) {
            if w.state != WORK_SEALED {
                act = act.max(def.act);
            }
        }
        act
    }
}

/// Whether unlock code `u` is held. `NO_UNLOCK` always is.
#[inline]
pub fn holds(unlocks: u32, u: u8) -> bool {
    u == NO_UNLOCK || (u as usize <= MAX_UNLOCKS && unlocks & (1 << (u - 1)) != 0)
}

/// What `knob` runs at under `unlocks`, per cent: the largest effect any
/// held unlock gives it, 100 when none does.
pub fn knob_pct(wc: &WorksContent, unlocks: u32, knob: u8) -> u32 {
    let mut pct = 100u32;
    for e in wc.effects.iter().take(wc.n_effects as usize) {
        if e.knob == knob && e.unlock != NO_UNLOCK && holds(unlocks, e.unlock) {
            pct = pct.max(e.pct as u32);
        }
    }
    pct
}

/// The burn rate at `players` online, per cent of full.
fn burn_pct(wc: &WorksContent, players: u32) -> u32 {
    let (quiet, busy) = (wc.quiet_players as u32, wc.busy_players as u32);
    let low = (wc.quiet_burn_pct as u32).min(100);
    if players <= quiet || busy <= quiet {
        return if players >= busy { 100 } else { low };
    }
    if players >= busy {
        return 100;
    }
    low + (100 - low) * (players - quiet) / (busy - quiet)
}

/// Open, light and burn every work, once a period. `players` is how many
/// are online, for the quiet-shard burn.
pub fn sweep(
    wc: &WorksContent,
    works: &mut Works,
    tick: u64,
    players: u32,
    events: &mut EventQueue,
) {
    if wc.count == 0 || tick % WORKS_PERIOD_TICKS != WORKS_PHASE {
        return;
    }
    let pct = burn_pct(wc, players);
    for (k, (def, w)) in wc
        .defs
        .iter()
        .zip(works.w.iter_mut())
        .take(wc.count as usize)
        .enumerate()
    {
        if w.state == WORK_SEALED && tick >= def.opens_at {
            w.state = WORK_OPEN;
            events.push(EV_WORK, k as u32, WORK_EV_OPENED, 0);
        }
        if w.state != WORK_LIT && tick >= def.fallback_at {
            w.state = WORK_LIT;
            w.lit_at = tick;
            w.fuel = 0;
            events.push(EV_WORK, k as u32, WORK_EV_FALLBACK, 0);
        }
        if w.state == WORK_LIT && w.fuel > 0 && def.burn_per_hour > 0 {
            w.burn_acc = w
                .burn_acc
                .saturating_add(def.burn_per_hour.saturating_mul(pct));
            let unit = 100 * PERIODS_PER_HOUR;
            let burnt = w.burn_acc / unit;
            w.burn_acc %= unit;
            w.fuel = w.fuel.saturating_sub(burnt);
            if w.fuel == 0 {
                w.burn_acc = 0;
                events.push(EV_WORK, k as u32, WORK_EV_EMBERS, 0);
            }
        }
    }
    works.refresh(wc);
}

/// One arc verb from `p`. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn act(
    wc: &WorksContent,
    works: &mut Works,
    haven: &Haven,
    tick: u64,
    p: &mut Player,
    op: u8,
    target: u8,
    arg: u8,
    events: &mut EventQueue,
) {
    let pid = p.id;
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(EV_ARC_REFUSED, pid, code, (op as u32) << 8 | target as u32);
    };
    let k = target as usize;
    let Some(def) = wc.get(k).copied() else {
        refuse(events, REFUSE_A_KIND);
        return;
    };
    if op > OP_MAX || p.dead || p.sleeping || p.wounded {
        refuse(events, REFUSE_A_KIND);
        return;
    }
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let feet = p.body.qy as f32 * crate::movement::POS_Y_Q;
    if !crate::spot::within(haven, &def.spot, px, feet, pz, WORK_REACH_M) {
        refuse(events, REFUSE_A_REACH);
        return;
    }
    let w = &mut works.w[k];
    match op {
        OP_DEPOSIT => {
            match w.state {
                WORK_SEALED => return refuse(events, REFUSE_A_SEALED),
                WORK_LIT => return refuse(events, REFUSE_A_LIT),
                _ => {}
            }
            if arg != ARG_ALL && arg >= def.n_inputs {
                return refuse(events, REFUSE_A_KIND);
            }
            let mut points = 0u32;
            for (i, input) in def.inputs.iter().enumerate().take(def.n_inputs as usize) {
                if arg != ARG_ALL && arg as usize != i {
                    continue;
                }
                let left = input.need.saturating_sub(w.got[i]);
                let take = inv_count(&p.inv, input.item).min(left);
                if take == 0 {
                    continue;
                }
                inv_take(&mut p.inv, input.item, take);
                w.got[i] += take;
                let share = (take as u64 * 10_000 / input.need.max(1) as u64).max(1);
                points = points.saturating_add(share as u32);
            }
            if points == 0 {
                return refuse(events, REFUSE_A_NOTHING);
            }
            w.credit(p.id, points);
            let met = def
                .inputs
                .iter()
                .zip(w.got.iter())
                .take(def.n_inputs as usize)
                .all(|(i, g)| *g >= i.need);
            if met {
                w.state = WORK_LIT;
                w.lit_at = tick;
                w.fuel = def.fuel_max;
                w.burn_acc = 0;
                events.push(EV_WORK, k as u32, WORK_EV_LIT, p.id);
                works.refresh(wc);
            }
        }
        OP_FUEL => {
            if w.state != WORK_LIT || def.fuel_max == 0 {
                return refuse(events, REFUSE_A_COLD);
            }
            let room = def.fuel_max.saturating_sub(w.fuel);
            if room == 0 {
                return refuse(events, REFUSE_A_FULL);
            }
            let take = inv_count(&p.inv, def.fuel).min(room);
            if take == 0 {
                return refuse(events, REFUSE_A_NOTHING);
            }
            inv_take(&mut p.inv, def.fuel, take);
            let dry = w.fuel == 0;
            w.fuel += take;
            // A full tank is worth a quarter of a quota's credit.
            let share = (take as u64 * 2_500 / def.fuel_max as u64).max(1);
            w.credit(p.id, share as u32);
            if dry {
                events.push(EV_WORK, k as u32, WORK_EV_REKINDLED, p.id);
                works.refresh(wc);
            }
        }
        _ => refuse(events, REFUSE_A_KIND),
    }
}
