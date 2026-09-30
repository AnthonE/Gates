//! Somewhere to come back to: the sleeping bags this body put down, where
//! it wakes after a death, and the walk back to what it dropped.
//!
//! What a player knows here and nothing more: the bags it placed itself
//! (it watched each go down), the own-bag list the death screen shows
//! (`ClientCore::own_bags`, sent at each death), and where its last death
//! backpack lies (`ClientCore::own_bag`, the map mark a player walks back
//! to). Nobody else's bags, boxes or bases.
//!
//! The skills are small state machines that return what to do next
//! ([`Do`]); `explorer.rs` owns the outbox and turns a `Do` into the verb.
//! Facts reach them through `Survivor::event_with`, which stays the one
//! reader of the client's rings.

use crate::agent::hands::Hands;
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::agent::site::foundation_goes;
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::backpack::LOOT_REACH_M;
use sim_core::build::{build_cell_of, BUILD_REACH_M, LOC_PLANE};
use sim_core::deploy::{
    cell_center, BagAnchor, ARCH_BAG, BAG_CAP, REFUSE_D_BAG_CAP, REFUSE_D_COST,
};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, MAX_BUILD_COORD, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::{self, Haven};

/// A death backpack further than this is not worth the walk back.
pub const RECOVER_M: f32 = 150.0;
/// A blow or a death this close to one of its bags means home is being
/// fought over...
pub const HOME_ALARM_M: f32 = 40.0;
/// ...for this long: waking there would walk into the same fight.
pub const HOME_ALARM_TICKS: u32 = 120 * TICK_HZ;
/// A deployable is held in hand this long before it is put down: the
/// ghost a person places by has to be on screen first.
pub const HOLD_TICKS: u32 = 6;
/// Spots tried for one bag before the goal gives up here.
pub const SPOT_TRIES: usize = 3;
/// A bag goes down this far from the feet at least (not underfoot)...
pub const BAG_NEAR_M: f32 = 1.5;
/// ...and at most this far, inside the server's reach with room for the
/// half metre a snapshot lags a body.
pub const BAG_FAR_M: f32 = BUILD_REACH_M - 1.0;
/// Where the recovery walk stops short of the backpack, metres.
pub const LOOT_STAND_M: f32 = 2.0;
/// Loot presses answered by nothing before the recovery gives up.
pub const LOOT_TRIES: u8 = 3;
/// An action's answer arrives within this long (`explorer::VERDICT_SECS`).
const VERDICT_TICKS: u32 = 3 * TICK_HZ;

/// What the server said about a deploy this body sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    Yes,
    Refused(u8),
}

/// What home has come to.
#[derive(Clone, Copy, Debug, Default)]
pub struct HomeStats {
    pub bags_placed: u64,
    pub deploy_refusals: u64,
    /// Death-screen answers sent, by choice (a lost answer asked again
    /// counts again).
    pub wakes_on_bag: u64,
    pub wakes_on_beach: u64,
    /// Loot presses sent, and recoveries that brought something home.
    pub loots: u64,
    pub recovered: u64,
}

/// The bags this body knows it has down, the alarm over them, and the
/// verdict on a deploy in flight.
#[derive(Clone, Copy, Debug)]
pub struct Home {
    /// Build cells (and storey) of its own sleeping bags.
    bags: [Option<(u16, u16, u8)>; BAG_CAP],
    /// The last blow or death near one of them.
    alarm: Option<u32>,
    /// The address a deploy was sent to, until its answer.
    asked: Option<(u16, u16, u8, u8)>,
    verdict: Option<Placed>,
    pub stats: HomeStats,
}

impl Default for Home {
    fn default() -> Self {
        Self::new()
    }
}

impl Home {
    pub const fn new() -> Self {
        Self {
            bags: [None; BAG_CAP],
            alarm: None,
            asked: None,
            verdict: None,
            stats: HomeStats {
                bags_placed: 0,
                deploy_refusals: 0,
                wakes_on_bag: 0,
                wakes_on_beach: 0,
                loots: 0,
                recovered: 0,
            },
        }
    }

    /// A new session: what it knew of the old one is learned again at the
    /// next death screen.
    pub fn reset(&mut self) {
        let stats = self.stats;
        *self = Self::new();
        self.stats = stats;
    }

    /// Sleeping bags it knows it has down.
    pub fn bags(&self) -> u8 {
        self.bags.iter().flatten().count() as u8
    }

    /// Did this body build what stands at this address?
    pub fn owns(&self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        loc == LOC_PLANE && self.bags.contains(&Some((cx, cz, level)))
    }

    /// The death screen's list replaces what it thought: a bag a raider
    /// cut is gone from it, one from an earlier session is on it.
    pub fn on_bags(&mut self, anchors: &[BagAnchor]) {
        self.bags = [None; BAG_CAP];
        for (slot, a) in self.bags.iter_mut().zip(anchors) {
            *slot = Some((a.cx, a.cz, a.level));
        }
    }

    /// A deploy is on its way to this address.
    pub fn asked(&mut self, cx: u16, cz: u16, level: u8, loc: u8) {
        self.asked = Some((cx, cz, level, loc));
        self.verdict = None;
    }

    /// A placement broadcast. Only the address it asked for is its own.
    pub fn on_placed(&mut self, cx: u16, cz: u16, level: u8, loc: u8, deploy: bool) {
        if deploy && self.asked == Some((cx, cz, level, loc)) {
            self.asked = None;
            self.verdict = Some(Placed::Yes);
            self.stats.bags_placed += 1;
            if let Some(slot) = self.bags.iter_mut().find(|b| b.is_none()) {
                *slot = Some((cx, cz, level));
            }
        }
    }

    /// A deploy refusal: its own, since the ring carries only the owner's.
    pub fn on_refused(&mut self, reason: u8) {
        self.stats.deploy_refusals += 1;
        if self.asked.take().is_some() {
            self.verdict = Some(Placed::Refused(reason));
        }
    }

    pub fn take_verdict(&mut self) -> Option<Placed> {
        self.verdict.take()
    }

    /// Hurt, or killed, standing here: near one of its bags that is home
    /// being fought over.
    pub fn on_hurt(&mut self, at: [f32; 2], tick: u32) {
        let near = self.bags.iter().flatten().any(|&(cx, cz, _)| {
            let (x, z) = cell_center(cx, cz);
            (x - at[0]).hypot(z - at[1]) <= HOME_ALARM_M
        });
        if near {
            self.alarm = Some(tick);
        }
    }

    pub fn under_attack(&self, tick: u32) -> bool {
        self.alarm
            .is_some_and(|at| tick.wrapping_sub(at) < HOME_ALARM_TICKS)
    }

    /// The death screen's choice: a ready bag of its own, unless home is
    /// where the fight is; otherwise the beach.
    pub fn wake_on_bag(&self, core: &ClientCore, tick: u32) -> bool {
        core.any_bag_ready() && !self.under_attack(tick)
    }

    /// The answer went out; count which one.
    pub fn woke(&mut self, on_bag: bool) {
        if on_bag {
            self.stats.wakes_on_bag += 1;
        } else {
            self.stats.wakes_on_beach += 1;
        }
    }
}

/// The deployable row that is a sleeping bag, and the item it places.
pub fn bag_row(core: &ClientCore) -> Option<(u16, u16)> {
    let defs = &core.deploy_defs;
    (0..core.deploy_defs_have.min(defs.def_count)).find_map(|row| {
        let def = defs.defs[row as usize];
        (def.arch == ARCH_BAG && def.hp > 0 && def.item != 0).then_some((row, def.item))
    })
}

/// Where the backpack from its last death lies, while it stands.
pub fn death_bag(core: &ClientCore) -> Option<[f32; 3]> {
    if core.own_bag == 0 {
        return None;
    }
    core.bags
        .entries()
        .iter()
        .find(|b| b.id == core.own_bag)
        .map(|b| {
            [
                b.qx as f32 * POS_XZ_Q,
                b.qy as f32 * POS_Y_Q,
                b.qz as f32 * POS_XZ_Q,
            ]
        })
}

/// A cell to put a bag down in, on bare ground within reach of the feet:
/// ground a foundation would take, nothing built or deployed on it, not a
/// spot the server already refused. The nearest to a stride ahead wins.
pub fn bag_cell(
    core: &ClientCore,
    seed: u64,
    haven: &Haven,
    body: &EntityState,
    refused: &[(u16, u16)],
) -> Option<(u16, u16)> {
    let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    let (fx, fz) = sim_core::yaw_dir(body.yaw);
    let (ax, az) = (x + fx * 2.5, z + fz * 2.5);
    let (bx, bz) = (build_cell_of(x), build_cell_of(z));
    let mut best: Option<(f32, u16, u16)> = None;
    for dz in -2..=2 {
        for dx in -2..=2 {
            let (cx, cz) = (bx + dx, bz + dz);
            if !(0..MAX_BUILD_COORD as i32).contains(&cx)
                || !(0..MAX_BUILD_COORD as i32).contains(&cz)
            {
                continue;
            }
            let (cx, cz) = (cx as u16, cz as u16);
            let (mx, mz) = cell_center(cx, cz);
            let d = (mx - x).hypot(mz - z);
            let score = (mx - ax).hypot(mz - az);
            if !(BAG_NEAR_M..=BAG_FAR_M).contains(&d)
                || refused.contains(&(cx, cz))
                || best.is_some_and(|(b, ..)| score >= b)
                || core.pieces.cols().get(cx, cz).planes & 1 != 0
                || core
                    .deploys
                    .entries()
                    .iter()
                    .any(|r| r.cx == cx && r.cz == cz && r.level == 0)
                || !foundation_goes(seed, haven, cx, cz)
            {
                continue;
            }
            best = Some((score, cx, cz));
        }
    }
    best.map(|(_, cx, cz)| (cx, cz))
}

/// The whole-stack move that puts `item` on the belt from the pack: an
/// empty belt slot, else the last one (the stacks swap).
fn belt_move(core: &ClientCore, item: u16) -> Option<(u8, u8, u16)> {
    let from =
        (HOTBAR_SLOTS..INV_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == item)?;
    let to = (0..HOTBAR_SLOTS)
        .find(|&i| core.inv[i].count == 0)
        .unwrap_or(HOTBAR_SLOTS - 1);
    Some((from as u8, to as u8, core.inv[from].count))
}

/// What a home skill wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Do {
    /// Carry on with this intent.
    Go(Intent),
    /// Move a stack from the pack to the belt (`encode_action_move`).
    Belt {
        from: u8,
        to: u8,
        count: u16,
    },
    /// Put the held deployable down here (`encode_action_deploy`), holding
    /// this intent while it goes.
    Deploy {
        row: u16,
        cx: u16,
        cz: u16,
        intent: Intent,
    },
    /// Open the backpack in reach (`encode_action_loot`).
    Loot(Intent),
    Done,
    Fail(Why),
}

/// Putting one sleeping bag down: to the belt, in hand, eyes on the spot,
/// then the deploy and its verdict. A refused spot is crossed off and the
/// next one tried.
#[derive(Clone, Copy, Debug, Default)]
pub struct BagJob {
    cell: Option<(u16, u16)>,
    /// Since when the bag has been asked to be in hand at this spot.
    held: Option<u32>,
    belt: Option<u32>,
    sent: Option<u32>,
    refused: [(u16, u16); SPOT_TRIES],
    tries: usize,
}

impl BagJob {
    pub fn belt_sent(&mut self, tick: u32) {
        self.belt = Some(tick);
    }

    pub fn deploy_sent(&mut self, tick: u32) {
        self.sent = Some(tick);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        home: &mut Home,
        tick: u32,
    ) -> Do {
        let Some((row, item)) = bag_row(core) else {
            return Do::Fail(Why::NoRecipe);
        };
        if let Some(at) = self.sent {
            match home.take_verdict() {
                Some(Placed::Yes) => return Do::Done,
                Some(Placed::Refused(r)) => {
                    self.sent = None;
                    self.held = None;
                    match u32::from(r) {
                        REFUSE_D_COST => return Do::Fail(Why::MissingInputs),
                        REFUSE_D_BAG_CAP => return Do::Fail(Why::Refused),
                        // A spot the ground, the reach, a claim or a
                        // neighbour spoiled: another one.
                        _ => {
                            if let Some(cell) = self.cell.take() {
                                if self.tries < SPOT_TRIES {
                                    self.refused[self.tries] = cell;
                                }
                            }
                            self.tries += 1;
                            if self.tries >= SPOT_TRIES {
                                return Do::Fail(Why::NoSpot);
                            }
                        }
                    }
                }
                None if tick.wrapping_sub(at) >= VERDICT_TICKS => {
                    return Do::Fail(Why::NoAnswer);
                }
                // The bag may leave the pack before the broadcast lands.
                None => return Do::Go(Intent::IDLE),
            }
        }
        let belt = (0..HOTBAR_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == item);
        let Some(slot) = belt else {
            return match self.belt {
                Some(at) if tick.wrapping_sub(at) < VERDICT_TICKS => Do::Go(Intent::IDLE),
                Some(_) => Do::Fail(Why::NoAnswer),
                None => match belt_move(core, item) {
                    Some((from, to, count)) => Do::Belt { from, to, count },
                    None => Do::Fail(Why::MissingInputs),
                },
            };
        };
        let refused = &self.refused[..self.tries.min(SPOT_TRIES)];
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        // A spot the body has since been pushed away from is picked again.
        let cell = self.cell.filter(|&(cx, cz)| {
            let (mx, mz) = cell_center(cx, cz);
            (mx - x).hypot(mz - z) <= BAG_FAR_M
        });
        let Some((cx, cz)) = cell.or_else(|| bag_cell(core, seed, haven, body, refused)) else {
            return Do::Fail(Why::NoSpot);
        };
        if self.cell != Some((cx, cz)) {
            self.cell = Some((cx, cz));
            self.held = None;
        }
        let (mx, mz) = cell_center(cx, cz);
        let intent = Intent {
            look: Look::Point([mx, terrain::ground(seed, haven, mx, mz), mz]),
            sel: Some(slot as u8),
            ..Intent::IDLE
        };
        let held = *self.held.get_or_insert(tick);
        if tick.wrapping_sub(held) >= HOLD_TICKS && hands.settled() {
            return Do::Deploy {
                row,
                cx,
                cz,
                intent,
            };
        }
        Do::Go(intent)
    }
}

/// The walk back to the death backpack and the loot press there.
#[derive(Clone, Copy, Debug, Default)]
pub struct RecoverJob {
    sent: Option<(u32, u32)>,
    tries: u8,
}

impl RecoverJob {
    /// A loot press went out at `tick`, with `gained` units in hand so far.
    pub fn loot_sent(&mut self, tick: u32, gained: u32) {
        self.sent = Some((tick, gained));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        hands: &Hands,
        hostile: bool,
        gained: u32,
        tick: u32,
    ) -> Do {
        let Some([bx, by, bz]) = death_bag(core) else {
            // Emptied by the press, or gone before it got there.
            return if gained > 0 {
                Do::Done
            } else {
                Do::Fail(Why::NotFound)
            };
        };
        if hostile {
            return Do::Fail(Why::Hostile);
        }
        let look = Intent {
            look: Look::Point([bx, by + 0.3, bz]),
            ..Intent::IDLE
        };
        if let Some((at, before)) = self.sent {
            if tick.wrapping_sub(at) < VERDICT_TICKS {
                return Do::Go(look);
            }
            self.sent = None;
            self.tries += 1;
            // It took what fitted and the rest stays: a full pack.
            if gained > before {
                return Do::Done;
            }
            if self.tries >= LOOT_TRIES {
                return Do::Fail(Why::Refused);
            }
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let d = (bx - x).hypot(bz - z);
        if d > LOOT_STAND_M {
            match route.to(core, body, [bx, bz], LOOT_STAND_M, true, tick) {
                step @ Step::Walk { .. } => {
                    return Do::Go(step.walk().unwrap_or(Intent::IDLE));
                }
                Step::Wait => return Do::Go(Intent::walk(yaw_toward(bx - x, bz - z))),
                Step::Blocked if d > LOOT_REACH_M - 0.5 => return Do::Fail(Why::Stuck),
                Step::Arrived | Step::Blocked => {}
            }
        }
        if d > LOOT_REACH_M - 0.5 {
            return Do::Go(Intent::walk(yaw_toward(bx - x, bz - z)));
        }
        // The press a person makes once the prompt is on the bag.
        if hands.settled() {
            Do::Loot(look)
        } else {
            Do::Go(look)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_its_own_bags_and_a_fight_there_sends_it_to_the_beach() {
        let mut home = Home::new();
        home.asked(100, 100, 0, LOC_PLANE);
        // Somebody else's placement is not an answer.
        home.on_placed(101, 100, 0, LOC_PLANE, true);
        assert_eq!(home.take_verdict(), None);
        home.on_placed(100, 100, 0, LOC_PLANE, true);
        assert_eq!(home.take_verdict(), Some(Placed::Yes));
        assert_eq!(home.bags(), 1);
        assert!(home.owns(100, 100, 0, LOC_PLANE));
        let (x, z) = cell_center(100, 100);
        home.on_hurt([x + HOME_ALARM_M + 5.0, z], 10);
        assert!(!home.under_attack(10), "a blow far from home");
        home.on_hurt([x + 10.0, z], 20);
        assert!(home.under_attack(20 + HOME_ALARM_TICKS - 1));
        assert!(!home.under_attack(20 + HOME_ALARM_TICKS));
        // The death screen's list is the truth.
        home.on_bags(&[]);
        assert_eq!(home.bags(), 0);
        // A refusal answers only a deploy in flight.
        home.on_refused(3);
        assert_eq!(home.take_verdict(), None);
        home.asked(5, 5, 0, LOC_PLANE);
        home.on_refused(3);
        assert_eq!(home.take_verdict(), Some(Placed::Refused(3)));
    }
}
