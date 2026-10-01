//! Defending home. A blow, a blast or shots at its own base call this body
//! home (`Home::alarmed`): in through its doors, where the fight reflex
//! answers whoever is at work on the walls from inside them
//! (`combat::Kit::home_ground`). Once home has been quiet a while, what
//! was damaged is mended the way a player mends it: the hammer in hand,
//! standing where the repair key takes that piece (the structure nearest
//! the feet, [`nearest_structure`]), one repair at a time, each waiting
//! for the piece to stand whole again or for the refusal. Then the doors
//! are shut behind it.
//!
//! What it knows is its own: the base it built (the builder's plot), the
//! damage band the client draws on each of its pieces, and the alarm its
//! ears raised. The verbs go out from `explorer.rs`.

use crate::agent::build::{
    item_named, look_point, nearest_structure, recipe_for, Act, Builder, Region, Way, HAMMER_ITEM,
    STANDS,
};
use crate::agent::hands::Hands;
use crate::agent::home::{belt_move, Home, HOLD_TICKS};
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::build::{anchor, LOC_EDGE_XLO, LOC_EDGE_ZLO, REFUSE_B_COST};
use sim_core::limits::{HOTBAR_SLOTS, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::Haven;

/// An alarm at home this fresh calls a body that is out home.
pub const DEFEND_ALARM_TICKS: u32 = 60 * TICK_HZ;
/// Home quiet this long (no blow, blast or shot there, nobody dangerous in
/// sight): the damage is mended.
pub const QUIET_TICKS: u32 = 15 * TICK_HZ;
/// A defence that came to nothing is not offered again for this long.
pub const DEFEND_RETRY_TICKS: u32 = 120 * TICK_HZ;
/// A repair's answer: the piece whole again on the mirror, or a refusal.
pub const VERDICT_TICKS: u32 = 3 * TICK_HZ;
/// Presses at one piece answered by nothing before it is let be.
pub const REPAIR_TRIES: u8 = 2;
/// A hammer's craft lands within this long.
pub const CRAFT_TICKS: u32 = 15 * TICK_HZ;
/// Pieces given up on in one defence.
pub const SKIP_ROWS: usize = 8;
/// Outside a wall, it stands this far from the wall's middle to mend it:
/// nearer than anything else the repair key could take.
pub const MEND_STAND_M: f32 = 0.7;
/// The last steps onto that spot are walked straight, to within this.
pub const MEND_NEAR_M: f32 = 0.15;
/// Stalled this long on the last steps: the spot is not to be had.
pub const MEND_STALL_TICKS: u32 = 2 * TICK_HZ;

/// What the defence wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ward {
    Go(Intent),
    /// A step of the walk through its doors (the builder's: a door's use,
    /// a code, or a walk); never `Done` or `Fail`.
    Walk(Act),
    /// Craft a hammer (`encode_action_craft`).
    Craft {
        recipe: u16,
    },
    /// Move the hammer to the belt (`encode_action_move`).
    Belt {
        from: u8,
        to: u8,
        count: u16,
    },
    /// Mend the structure at this address, the hammer in hand
    /// (`encode_action_repair`).
    Repair {
        deploy: bool,
        at: OpAddr,
        intent: Intent,
    },
    Done,
    Fail(Why),
}

/// Where a piece is mended from: a point of the walk through its doors,
/// inside, or a spot outside the wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stand {
    Inside(usize),
    Outside([f32; 2]),
}

/// One damaged structure of its own and where it is mended from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    pub deploy: bool,
    pub at: OpAddr,
    pub from: Stand,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefendStats {
    pub defences: u64,
    pub repairs: u64,
    pub refusals: u64,
    pub hammers: u64,
}

/// One defence, from the walk home to the doors shut behind the repairs.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefendJob {
    fix: Option<Fix>,
    tries: u8,
    skipped: [Option<(bool, OpAddr)>; SKIP_ROWS],
    /// The hammer in hand and eyes on the piece since.
    held: Option<u32>,
    /// A repair in flight, and when it went.
    sent: Option<u32>,
    refused: Option<u8>,
    /// A hammer being crafted: the count before (none), and when it went.
    craft: Option<(u32, u32)>,
    belt: Option<u32>,
    /// The last straight steps: the nearest come, and when.
    approach: Option<(f32, u32)>,
    /// A door stands open behind it: out through it, shutting it, then
    /// back in.
    shut_out: bool,
    /// Mended everything it could: on the way in to shut the doors.
    finishing: bool,
    /// Repairs this defence made.
    pub mended: u32,
}

/// The pieces and deployables of its own base whose damage the client
/// draws (`dmg`, the band a player sees), in mirror order.
pub fn damaged<'a>(
    core: &'a ClientCore,
    builder: &'a Builder,
) -> impl Iterator<Item = (bool, OpAddr)> + 'a {
    let pieces = core
        .pieces
        .entries()
        .iter()
        .filter(|r| r.dmg > 0)
        .map(|r| (false, r.cx, r.cz, r.level, r.loc));
    let deploys = core
        .deploys
        .entries()
        .iter()
        .filter(|r| r.dmg > 0)
        .map(|r| (true, r.cx, r.cz, r.level, r.loc));
    pieces
        .chain(deploys)
        .filter(move |&(_, cx, cz, level, loc)| builder.owns(cx, cz, level, loc))
        .map(|(deploy, cx, cz, level, loc)| (deploy, OpAddr { cx, cz, level, loc }))
}

/// Where this structure is mended from, if anywhere: a point of the walk
/// through the doors where the repair key takes it, else a spot outside a
/// wall's middle where it does.
pub fn stand_for(core: &ClientCore, builder: &Builder, deploy: bool, at: OpAddr) -> Option<Stand> {
    let picks = |p: [f32; 2]| nearest_structure(core, p[0], p[1]) == Some((deploy, at));
    if let Some(i) = (0..STANDS).find(|&i| builder.stand_at(i).is_some_and(picks)) {
        return Some(Stand::Inside(i));
    }
    if at.level != 0 {
        return None;
    }
    let (ax, az) = anchor(at.cx, at.cz, at.loc);
    let normal = match at.loc {
        LOC_EDGE_XLO => [1.0, 0.0],
        LOC_EDGE_ZLO => [0.0, 1.0],
        _ => return None,
    };
    [1.0f32, -1.0].into_iter().find_map(|s| {
        let p = [
            ax + normal[0] * s * MEND_STAND_M,
            az + normal[1] * s * MEND_STAND_M,
        ];
        (builder.region_of(p) == Region::Outside && picks(p)).then_some(Stand::Outside(p))
    })
}

/// Is there anything of its own base to mend that the pack pays for, and
/// somewhere to mend it from?
pub fn mendable(core: &ClientCore, builder: &Builder) -> bool {
    damaged(core, builder).any(|(deploy, at)| {
        affordable(core, deploy, at) && stand_for(core, builder, deploy, at).is_some()
    })
}

/// Would the pack pay for mending this, by the game's price (each of the
/// piece's own cost rows, pro rata to the hp missing, at the table's
/// repair percent), the missing hp taken at the most the drawn band allows?
pub fn affordable(core: &ClientCore, deploy: bool, at: OpAddr) -> bool {
    let same = |r: (u16, u16, u8, u8)| r == (at.cx, at.cz, at.level, at.loc);
    if deploy {
        let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
        core.deploys
            .entries()
            .iter()
            .find(|r| same((r.cx, r.cz, r.level, r.loc)))
            .filter(|r| u16::from(r.row) < have)
            .is_some_and(|r| {
                let def = core.deploy_defs.defs[usize::from(r.row)];
                let n = usize::from(def.n_costs).min(def.costs.len());
                pays(core, r.dmg, def.hp, &def.costs[..n])
            })
    } else {
        core.pieces
            .entries()
            .iter()
            .find(|r| same((r.cx, r.cz, r.level, r.loc)))
            .filter(|r| u16::from(r.row) < core.piece_defs_have)
            .is_some_and(|r| {
                let def = core.piece_defs.pieces[usize::from(r.row)];
                let n = usize::from(def.n_costs).min(def.costs.len());
                pays(core, r.dmg, def.hp, &def.costs[..n])
            })
    }
}

/// The pack holds each row's share of a repair of `dmg` bands on `max` hp.
fn pays(core: &ClientCore, dmg: u8, max: u16, rows: &[(u16, u16)]) -> bool {
    if max == 0 {
        return false;
    }
    let pct = match core.piece_defs.repair_pct {
        0 => 100,
        p => u64::from(p),
    };
    let missing = (u64::from(dmg) * u64::from(max))
        .div_ceil(7)
        .min(u64::from(max));
    rows.iter().all(|&(item, units)| {
        let need = (u64::from(units) * missing * pct)
            .div_ceil(u64::from(max) * 100)
            .max(1);
        u64::from(count(core, item)) >= need
    })
}

/// The damage band the client draws at this address; `None` once nothing
/// stands there.
fn dmg_at(core: &ClientCore, deploy: bool, at: OpAddr) -> Option<u8> {
    let same = |cx: u16, cz: u16, level: u8, loc: u8| {
        (cx, cz, level, loc) == (at.cx, at.cz, at.level, at.loc)
    };
    if deploy {
        core.deploys
            .entries()
            .iter()
            .find(|r| same(r.cx, r.cz, r.level, r.loc))
            .map(|r| r.dmg)
    } else {
        core.pieces
            .entries()
            .iter()
            .find(|r| same(r.cx, r.cz, r.level, r.loc))
            .map(|r| r.dmg)
    }
}

fn count(core: &ClientCore, item: u16) -> u32 {
    core.inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

impl DefendJob {
    /// Between two ops: no repair or craft awaiting its answer, no door
    /// left open mid-walk (the builder says that half).
    pub fn at_checkpoint(&self) -> bool {
        self.sent.is_none() && self.craft.is_none() && self.belt.is_none()
    }

    /// The action asked for went out.
    pub fn sent(&mut self, tick: u32, what: &Ward) {
        match what {
            Ward::Repair { .. } => {
                self.sent = Some(tick);
                self.refused = None;
                self.held = None;
            }
            Ward::Craft { .. } => self.craft = Some((0, tick)),
            Ward::Belt { .. } => self.belt = Some(tick),
            _ => {}
        }
    }

    /// A build refusal: a repair's, while one is in flight.
    pub fn on_refused(&mut self, reason: u8) {
        if self.sent.is_some() {
            self.refused = Some(reason);
        }
    }

    fn skip(&mut self, deploy: bool, at: OpAddr) {
        if let Some(slot) = self.skipped.iter_mut().find(|s| s.is_none()) {
            *slot = Some((deploy, at));
        }
        self.fix = None;
        self.tries = 0;
        self.held = None;
        self.approach = None;
    }

    fn skipped(&self, deploy: bool, at: OpAddr) -> bool {
        self.skipped.contains(&Some((deploy, at)))
    }

    /// The builder's walk, as a ward: its end is the caller's to read.
    fn walk(act: Act) -> Result<Ward, Option<Why>> {
        match act {
            Act::Done => Err(None),
            Act::Fail(why) => Err(Some(why)),
            act => Ok(Ward::Walk(act)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        builder: &mut Builder,
        home: &Home,
        hostile: bool,
        stats: &mut DefendStats,
        tick: u32,
    ) -> Ward {
        if builder.plan().is_none() || !builder.survey().hearth {
            return Ward::Fail(Why::NotFound);
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        // A repair in flight is answered first: the piece whole on the
        // mirror, a refusal, or nothing.
        if let (Some(at), Some(fix)) = (self.sent, self.fix) {
            let whole = dmg_at(core, fix.deploy, fix.at).is_none_or(|d| d == 0);
            if whole {
                stats.repairs += 1;
                self.mended += 1;
                self.sent = None;
                self.fix = None;
                self.tries = 0;
            } else if let Some(reason) = self.refused.take() {
                stats.refusals += 1;
                self.sent = None;
                if u32::from(reason) == REFUSE_B_COST {
                    // Short of what it is mended with: so is every other.
                    return Ward::Fail(Why::MissingInputs);
                }
                self.skip(fix.deploy, fix.at);
            } else if tick.wrapping_sub(at) >= VERDICT_TICKS {
                self.sent = None;
                self.tries += 1;
                if self.tries >= REPAIR_TRIES {
                    self.skip(fix.deploy, fix.at);
                }
            } else {
                return Ward::Go(Intent::IDLE);
            }
        }
        // While home is fought over: inside, where the reflex answers the
        // raider from behind its walls.
        if hostile || home.alarmed(tick, QUIET_TICKS) {
            self.held = None;
            if builder.region(body) != Region::Room || builder.passing() {
                return match Self::walk(
                    builder.pass_to(core, seed, haven, body, hands, route, 0, tick),
                ) {
                    Ok(w) => w,
                    Err(None) => Ward::Go(Intent::IDLE),
                    Err(Some(why)) => Ward::Fail(why),
                };
            }
            return Ward::Go(Intent::IDLE);
        }
        if self.finishing {
            return self.finish(core, seed, haven, body, hands, route, builder, tick);
        }
        if self.fix.is_none() {
            self.fix = damaged(core, builder)
                .filter(|&(deploy, at)| !self.skipped(deploy, at) && affordable(core, deploy, at))
                .find_map(|(deploy, at)| {
                    stand_for(core, builder, deploy, at).map(|from| Fix { deploy, at, from })
                });
            self.tries = 0;
            self.approach = None;
        }
        let Some(fix) = self.fix else {
            self.finishing = true;
            return self.finish(core, seed, haven, body, hands, route, builder, tick);
        };
        // The piece came down meanwhile, or somebody mended it.
        if dmg_at(core, fix.deploy, fix.at).is_none_or(|d| d == 0) {
            self.fix = None;
            return Ward::Go(Intent::IDLE);
        }
        // The hammer: made if there is none, then on the belt.
        let Some(hammer) = item_named(core, HAMMER_ITEM) else {
            return Ward::Fail(Why::NoTool);
        };
        if let Some((before, at)) = self.craft {
            if count(core, hammer) > before {
                self.craft = None;
                stats.hammers += 1;
            } else if tick.wrapping_sub(at) >= CRAFT_TICKS {
                self.craft = None;
                return Ward::Fail(Why::NoTool);
            } else {
                return Ward::Go(Intent::IDLE);
            }
        }
        if count(core, hammer) == 0 {
            let stations = builder.stations(core);
            let Some((recipe, ..)) = recipe_for(core, hammer, &stations) else {
                return Ward::Fail(Why::NoTool);
            };
            return Ward::Craft { recipe };
        }
        // To where the repair key takes it.
        match fix.from {
            Stand::Inside(i) => {
                let Some(p) = builder.stand_at(i) else {
                    self.skip(fix.deploy, fix.at);
                    return Ward::Go(Intent::IDLE);
                };
                let there = (p[0] - x).hypot(p[1] - z) <= MEND_NEAR_M * 2.0;
                if !there || builder.passing() {
                    match Self::walk(
                        builder.pass_to(core, seed, haven, body, hands, route, i, tick),
                    ) {
                        Ok(w) => return w,
                        Err(None) => {}
                        Err(Some(_)) => {
                            self.skip(fix.deploy, fix.at);
                            return Ward::Go(Intent::IDLE);
                        }
                    }
                }
            }
            Stand::Outside(p) => {
                if builder.region(body) != Region::Outside || builder.passing() {
                    return match Self::walk(builder.pass(
                        core,
                        seed,
                        haven,
                        body,
                        hands,
                        route,
                        Way::Out,
                        tick,
                    )) {
                        Ok(w) => w,
                        Err(None) => Ward::Go(Intent::IDLE),
                        Err(Some(why)) => Ward::Fail(why),
                    };
                }
                let left = (p[0] - x).hypot(p[1] - z);
                if left > 0.6 {
                    self.approach = None;
                    return match route.to(core, body, p, 0.5, false, tick) {
                        step @ Step::Walk { .. } => Ward::Go(step.walk().unwrap_or(Intent::IDLE)),
                        Step::Wait | Step::Arrived => {
                            Ward::Go(Intent::walk(yaw_toward(p[0] - x, p[1] - z)))
                        }
                        Step::Blocked => {
                            self.skip(fix.deploy, fix.at);
                            Ward::Go(Intent::IDLE)
                        }
                    };
                }
                if left > MEND_NEAR_M && nearest_structure(core, x, z) != Some((fix.deploy, fix.at))
                {
                    match self.approach {
                        Some((best, since)) if left > best - 0.02 => {
                            if tick.wrapping_sub(since) >= MEND_STALL_TICKS {
                                self.skip(fix.deploy, fix.at);
                                return Ward::Go(Intent::IDLE);
                            }
                        }
                        _ => self.approach = Some((left, tick)),
                    }
                    return Ward::Go(Intent::walk(yaw_toward(p[0] - x, p[1] - z)));
                }
            }
        }
        if nearest_structure(core, x, z) != Some((fix.deploy, fix.at)) {
            // Not what the key takes from where the feet ended up.
            self.skip(fix.deploy, fix.at);
            return Ward::Go(Intent::IDLE);
        }
        // The hammer in hand, eyes on the piece, then the key.
        let slot = (0..HOTBAR_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == hammer);
        let Some(slot) = slot else {
            if let Some(at) = self.belt {
                if tick.wrapping_sub(at) < VERDICT_TICKS {
                    return Ward::Go(Intent::IDLE);
                }
                self.belt = None;
                return Ward::Fail(Why::NoTool);
            }
            return match belt_move(core, hammer) {
                Some((from, to, count)) => Ward::Belt { from, to, count },
                None => Ward::Fail(Why::NoTool),
            };
        };
        self.belt = None;
        let intent = Intent {
            look: Look::Point(look_point(seed, haven, core, fix.at)),
            sel: Some(slot as u8),
            ..Intent::IDLE
        };
        let held = *self.held.get_or_insert(tick);
        if tick.wrapping_sub(held) >= HOLD_TICKS && hands.settled() {
            return Ward::Repair {
                deploy: fix.deploy,
                at: fix.at,
                intent,
            };
        }
        Ward::Go(intent)
    }

    /// Everything mended that could be: back to the stand spot through the
    /// doors, shutting each behind; a door left standing open is shut by
    /// going out through it and back in.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        builder: &mut Builder,
        tick: u32,
    ) -> Ward {
        if !builder.walled(core) {
            return Ward::Done;
        }
        let inside = builder.region(body) != Region::Outside;
        if inside && !builder.passing() && builder.door_open(core) {
            self.shut_out = true;
        }
        if self.shut_out {
            return match Self::walk(builder.pass(
                core,
                seed,
                haven,
                body,
                hands,
                route,
                Way::Out,
                tick,
            )) {
                Ok(w) => w,
                Err(None) => {
                    self.shut_out = false;
                    Ward::Go(Intent::IDLE)
                }
                Err(Some(why)) => Ward::Fail(why),
            };
        }
        match Self::walk(builder.pass_to(core, seed, haven, body, hands, route, 0, tick)) {
            Ok(w) => w,
            Err(None) => Ward::Done,
            Err(Some(why)) => Ward::Fail(why),
        }
    }
}
