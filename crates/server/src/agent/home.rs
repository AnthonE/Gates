//! Somewhere to come back to: the sleeping bags this body put down, where
//! it wakes after a death, and the walk back to what it dropped.
//!
//! What a player knows here and nothing more: the bags it placed itself
//! (it watched each go down), the own-bag list the death screen shows
//! (`ClientCore::own_bags`, sent at the join and re-sent as its bags go
//! down or come up, on a wake and at each death),
//! and where its last death backpack lies (`ClientCore::own_bag`, the map
//! mark a player walks back to). Nobody else's bags, boxes or bases.
//!
//! The skills are small state machines that return what to do next
//! ([`Do`]); `explorer.rs` owns the outbox and turns a `Do` into the verb.
//! Facts reach them through `Survivor::event_with`, which stays the one
//! reader of the client's rings.

use crate::agent::hands::Hands;
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::agent::site::foundation_goes;
use crate::agent::stash::STASH_RETRY_TICKS;
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::backpack::LOOT_REACH_M;
use sim_core::build::{build_cell_of, BUILD_REACH_M, LOC_PLANE};
use sim_core::deploy::{
    cell_center, BagAnchor, ARCH_BAG, BAG_CAP, FEED_CHUNK, REFUSE_D_BAG_CAP, REFUSE_D_COST,
    UPKEEP_PERIOD_TICKS,
};
use sim_core::limits::{HEARTH_STOCK_ROWS, HOTBAR_SLOTS, INV_SLOTS, MAX_BUILD_COORD, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::{self, Haven};
use sim_core::upkeep;

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
/// A bag that failed to go down is not tried again until the body has
/// walked this far from where it failed...
pub const BAG_RETRY_M: f32 = 10.0;
/// ...or this long has passed: the same ground, claim or crowd would
/// refuse it again.
pub const BAG_RETRY_TICKS: u32 = 120 * TICK_HZ;

/// A cupboard whose last reading covers fewer upkeep periods than this
/// (hours) is fed on the next visit home.
pub const UPKEEP_LOW_PERIODS: u32 = 6;

/// The reply to the last feed: the cupboard's stock, per upkeep material,
/// against what one upkeep period charges in it, and when it was read.
/// The only stock readout the game gives, so it is what this body goes by
/// until it feeds again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub items: [u16; HEARTH_STOCK_ROWS],
    pub stock: [u32; HEARTH_STOCK_ROWS],
    pub bill: [u32; HEARTH_STOCK_ROWS],
    pub rows: u8,
    pub at: u32,
    /// Grades standing when it was read: a base that has grown since pays
    /// a bigger bill than the reading says.
    pub grades: u32,
}

impl Reading {
    /// Whole upkeep periods the stock still covers at `tick`; `None` when
    /// nothing is charged (twig, or nothing the cupboard pays for).
    pub fn periods_left(&self, tick: u32) -> Option<u32> {
        let n = usize::from(self.rows);
        let covers = upkeep::lasts(&self.stock[..n], &self.bill[..n])?;
        Some(covers.saturating_sub(self.gone(tick)))
    }

    /// Upkeep periods charged since the reading.
    fn gone(&self, tick: u32) -> u32 {
        let gone = u64::from(tick.wrapping_sub(self.at)) / UPKEEP_PERIOD_TICKS;
        gone.min(u64::from(u32::MAX)) as u32
    }

    /// The charged materials running low at `tick`.
    fn low(&self, tick: u32) -> impl Iterator<Item = u16> + '_ {
        let gone = self.gone(tick);
        (0..usize::from(self.rows))
            .filter(move |&i| {
                self.bill[i] > 0
                    && (u64::from(self.stock[i]) * 24 / u64::from(self.bill[i]))
                        .saturating_sub(u64::from(gone))
                        < u64::from(UPKEEP_LOW_PERIODS)
            })
            .map(|i| self.items[i])
    }
}

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
    /// The last blow or death near one of them, and where it was.
    alarm: Option<u32>,
    alarm_at: Option<[f32; 2]>,
    /// The last blow, blast, shot or hurt at the base itself (not at a bag
    /// put down elsewhere): what calls it home to defend it.
    fought: Option<u32>,
    /// The last blow, break or blast on the base itself: not a bite taken
    /// standing beside it.
    raid: Option<u32>,
    /// The address a deploy was sent to, until its answer.
    asked: Option<(u16, u16, u8, u8)>,
    verdict: Option<Placed>,
    /// The server said it has all the bags it may have down: none of the
    /// ones it knows of this session, maybe, but the cap stands until the
    /// next own-bag list names them.
    capped: bool,
    /// Where and when the last bag failed to go down.
    bag_failed: Option<([f32; 2], u32)>,
    /// The death backpack (`ClientCore::own_bag`) whose recovery came to
    /// nothing more, and the free pack slots then: not walked to again
    /// until the pack has more room, or a new death leaves a new one.
    pack_tried: Option<(u32, u8)>,
    /// Where its base stands (the spot in its core), once it has one.
    base: Option<[f32; 2]>,
    /// The last feed's reply.
    upkeep: Option<Reading>,
    /// When the last visit to the box came to nothing.
    stash_failed: Option<u32>,
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
            alarm_at: None,
            fought: None,
            raid: None,
            asked: None,
            verdict: None,
            capped: false,
            bag_failed: None,
            pack_tried: None,
            base: None,
            upkeep: None,
            stash_failed: None,
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

    /// A new session: what it knew of the old one is learned again from
    /// the own-bag list the server sends at the join.
    pub fn reset(&mut self) {
        let stats = self.stats;
        *self = Self::new();
        self.stats = stats;
    }

    /// Sleeping bags it knows it has down.
    pub fn bags(&self) -> u8 {
        self.bags.iter().flatten().count() as u8
    }

    /// Sleeping bags it counts as down when choosing: all it may have
    /// once the server has said so.
    pub fn bags_known(&self) -> u8 {
        if self.capped {
            BAG_CAP as u8
        } else {
            self.bags()
        }
    }

    /// A bag failed to go down with the body standing here.
    pub fn bag_failed(&mut self, at: [f32; 2], tick: u32) {
        self.bag_failed = Some((at, tick));
    }

    /// Is a bag not worth trying here and now: it failed close by, a
    /// moment ago? Moving on or waiting a while gives it another go.
    pub fn bag_held(&self, at: [f32; 2], tick: u32) -> bool {
        self.bag_failed.is_some_and(|(p, t)| {
            tick.wrapping_sub(t) < BAG_RETRY_TICKS
                && (p[0] - at[0]).hypot(p[1] - at[1]) <= BAG_RETRY_M
        })
    }

    /// The recovery of this death backpack came to all it will.
    pub fn pack_tried(&mut self, core: &ClientCore) {
        if core.own_bag != 0 {
            self.pack_tried = Some((core.own_bag, free_slots(core)));
        }
    }

    /// Is the walk back not worth it: no free slot for what it holds, or
    /// this backpack was tried already and the pack has no more room.
    pub fn recover_held(&self, core: &ClientCore) -> bool {
        let free = free_slots(core);
        free == 0
            || self
                .pack_tried
                .is_some_and(|(id, then)| id == core.own_bag && free <= then)
    }

    /// Did this body build what stands at this address?
    pub fn owns(&self, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
        loc == LOC_PLANE && self.bags.contains(&Some((cx, cz, level)))
    }

    /// The own-bag list (at the join, as its bags change, at each death)
    /// replaces what it thought: a bag a raider cut is gone from it, one
    /// from an earlier session is on it.
    pub fn on_bags(&mut self, anchors: &[BagAnchor]) {
        self.bags = [None; BAG_CAP];
        self.capped = false;
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
        // The join's own-bag list names bags from an earlier session, so
        // this is the backstop: a refusal that beats that list (or a list
        // a resync is still owed) still says the cap is full.
        if u32::from(reason) == REFUSE_D_BAG_CAP {
            self.capped = true;
        }
        if self.asked.take().is_some() {
            self.verdict = Some(Placed::Refused(reason));
        }
    }

    pub fn take_verdict(&mut self) -> Option<Placed> {
        self.verdict.take()
    }

    /// Where its base stands, if it has one (the builder's plot).
    pub fn set_base(&mut self, at: Option<[f32; 2]>) {
        self.base = at;
    }

    /// Is this spot home ground: near its base or one of its bags?
    fn near_home(&self, at: [f32; 2]) -> bool {
        let near = |x: f32, z: f32| (x - at[0]).hypot(z - at[1]) <= HOME_ALARM_M;
        self.near_base(at)
            || self.bags.iter().flatten().any(|&(cx, cz, _)| {
                let (x, z) = cell_center(cx, cz);
                near(x, z)
            })
    }

    fn near_base(&self, at: [f32; 2]) -> bool {
        self.base
            .is_some_and(|[x, z]| (x - at[0]).hypot(z - at[1]) <= HOME_ALARM_M)
    }

    /// Hurt, or killed, standing here: near its base or one of its bags
    /// that is home being fought over.
    pub fn on_hurt(&mut self, at: [f32; 2], tick: u32) {
        if self.near_home(at) {
            self.sound(at, tick);
        }
    }

    fn sound(&mut self, at: [f32; 2], tick: u32) {
        self.alarm = Some(tick);
        self.alarm_at = Some(at);
        if self.near_base(at) {
            self.fought = Some(tick);
        }
    }

    /// A blast heard here: near home, somebody is blowing their way in.
    pub fn on_blast(&mut self, at: [f32; 2], tick: u32) {
        if self.near_home(at) {
            self.sound(at, tick);
        }
        if self.near_base(at) {
            self.raid = Some(tick);
        }
    }

    /// A blow landed on a piece or deployable of its own base.
    pub fn on_struck(&mut self, tick: u32) {
        self.alarm = Some(tick);
        self.alarm_at = self.base;
        self.fought = Some(tick);
        self.raid = Some(tick);
    }

    /// Something of its own base that does not rot came down: while the
    /// cupboard's stock covers the upkeep nothing decays, so somebody
    /// broke it. Without a reading (or with the stock run out) it may be
    /// rot, and says nothing; so does a reading taken before the base grew
    /// past `grades` then, since a bigger bill burns the stock faster than
    /// it says.
    pub fn on_removed(&mut self, tick: u32, grades: u32) {
        if self
            .upkeep
            .filter(|r| grades <= r.grades)
            .and_then(|r| r.periods_left(tick))
            .is_some_and(|p| p > 0)
        {
            self.alarm = Some(tick);
            self.alarm_at = self.base;
            self.fought = Some(tick);
            self.raid = Some(tick);
        }
    }

    /// Its own cupboard came down: what the last feed read was that
    /// cupboard's stock, not the one built in its place.
    pub fn lost_hearth(&mut self) {
        self.upkeep = None;
    }

    /// A stock readout of its own cupboard arrived: a feed's reply, or the
    /// crew vital's push while it stands in the claim.
    pub fn on_stock(&mut self, core: &ClientCore, tick: u32, grades: u32) {
        let n = usize::from(core.stock_count).min(HEARTH_STOCK_ROWS);
        let mut r = Reading {
            rows: n as u8,
            at: tick,
            grades,
            ..Reading::default()
        };
        for (i, &(item, units, bill)) in core.stock[..n].iter().enumerate() {
            (r.items[i], r.stock[i], r.bill[i]) = (item, units, bill);
        }
        self.upkeep = Some(r);
    }

    pub fn upkeep(&self) -> Option<Reading> {
        self.upkeep
    }

    /// Does the cupboard want feeding: a base with `grades` standing pays
    /// upkeep, and the last reading is missing, running low, or was taken
    /// before the base grew into paying anything.
    pub fn upkeep_due(&self, tick: u32, grades: u32) -> bool {
        if grades == 0 {
            // Twig is never charged.
            return false;
        }
        match self.upkeep {
            None => true,
            Some(r) => match r.periods_left(tick) {
                Some(left) => left < UPKEEP_LOW_PERIODS,
                None => grades > r.grades,
            },
        }
    }

    /// Would a feed now top up what runs low, with what `has` says is to
    /// hand (the pack, or the box too)? A feed takes a chunk of every
    /// material the cupboard eats, charged or not, so it waits for one
    /// that is both charged and running low: a pack of wood fed to a
    /// cupboard short of stone is wood lost. Before any reading (or once
    /// the base has grown past what the last one charged) the charges are
    /// not known, and a feed is how a player learns them.
    pub fn can_feed(&self, has: impl Fn(u16) -> bool, tick: u32) -> bool {
        match self.upkeep {
            None => true,
            Some(r) if r.periods_left(tick).is_none() => {
                r.items[..usize::from(r.rows)].iter().any(|&i| has(i))
            }
            Some(r) => r.low(tick).any(has),
        }
    }

    /// Would a feed take `item` for nothing? The last reading names it
    /// among what the cupboard eats with nothing charged in it, while
    /// something else is, and the base has not grown since (`grades`): fed,
    /// it sits in the stock for good, the fragments a bench was to cost.
    /// Without a current reading nothing is known to be wasted.
    pub fn feed_wastes(&self, item: u16, grades: u32) -> bool {
        self.upkeep.is_some_and(|r| {
            let n = usize::from(r.rows);
            grades <= r.grades
                && r.bill[..n].iter().any(|&b| b > 0)
                && (0..n).any(|i| r.items[i] == item && r.bill[i] == 0)
        })
    }

    /// What a feed wants in the pack while the cupboard is due one: a
    /// feed's worth of each material running low (of each it eats, when
    /// the charges are not known), into `out`; the rows filled. Nothing
    /// without a reading, which names no material.
    pub fn feed_rows(&self, tick: u32, grades: u32, out: &mut [(u16, u32)]) -> usize {
        let Some(r) = self.upkeep.filter(|_| self.upkeep_due(tick, grades)) else {
            return 0;
        };
        let mut n = 0;
        let mut push = |item: u16| {
            if n < out.len() {
                out[n] = (item, FEED_CHUNK);
                n += 1;
            }
        };
        if r.periods_left(tick).is_none() {
            r.items[..usize::from(r.rows)].iter().for_each(|&i| push(i));
        } else {
            r.low(tick).for_each(push);
        }
        n
    }

    /// A visit to the box came to nothing here and now.
    pub fn stash_failed(&mut self, tick: u32) {
        self.stash_failed = Some(tick);
    }

    /// Is a visit to the box not worth another try yet?
    pub fn stash_held(&self, tick: u32) -> bool {
        self.stash_failed
            .is_some_and(|t| tick.wrapping_sub(t) < STASH_RETRY_TICKS)
    }

    /// Where its base stands, while a blow, break or blast on it is
    /// fresher than `within`: where whoever is doing it is to be found.
    /// Being hurt beside it is not one: that blow has its own author.
    pub fn raided(&self, tick: u32, within: u32) -> Option<[f32; 2]> {
        self.base
            .filter(|_| self.raid.is_some_and(|at| tick.wrapping_sub(at) < within))
    }

    /// A blow, blast, shot or death at the base itself fresher than
    /// `within`. A fight by a bag put down elsewhere is not one: nothing
    /// there for a defence to save.
    pub fn alarmed(&self, tick: u32, within: u32) -> bool {
        self.fought.is_some_and(|at| tick.wrapping_sub(at) < within)
    }

    /// A defence saw the base through: the fight that called it home is
    /// over, and only a new one calls it back.
    pub fn settle(&mut self) {
        self.fought = None;
    }

    pub fn under_attack(&self, tick: u32) -> bool {
        self.alarm
            .is_some_and(|at| tick.wrapping_sub(at) < HOME_ALARM_TICKS)
    }

    /// Shots landing here, heard: near home, somebody is fighting over it.
    pub fn on_shot(&mut self, at: [f32; 2], tick: u32) {
        if self.near_home(at) {
            self.sound(at, tick);
        }
    }

    /// The death screen's choice: a ready bag of its own, unless home is
    /// where the fight is; otherwise the beach. The game wakes a body on
    /// the ready bag nearest where it fell (`fell`), so while home is
    /// fought over that bag is taken only when it lies away from the base:
    /// a bag elsewhere is a safe start, the one at home is the fight again.
    pub fn wake_on_bag(&self, core: &ClientCore, tick: u32, fell: [f32; 2]) -> bool {
        self.wake_on(core.own_bags(), tick, fell)
    }

    /// [`Home::wake_on_bag`] over a list of its bags (the own-bag list).
    pub fn wake_on(&self, bags: &[BagAnchor], tick: u32, fell: [f32; 2]) -> bool {
        let ready = bags.iter().filter(|b| b.ready);
        if !self.under_attack(tick) {
            return ready.count() > 0;
        }
        let mut nearest: Option<(f32, [f32; 2])> = None;
        for b in ready {
            let (x, z) = cell_center(b.cx, b.cz);
            let d = (x - fell[0]).hypot(z - fell[1]);
            if nearest.is_none_or(|(n, _)| d < n) {
                nearest = Some((d, [x, z]));
            }
        }
        let fought = self.alarm_at;
        nearest.is_some_and(|(_, [x, z])| {
            fought.is_some_and(|[fx, fz]| (fx - x).hypot(fz - z) > HOME_ALARM_M)
        })
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

/// Empty pack and belt slots.
fn free_slots(core: &ClientCore) -> u8 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count == 0)
        .count() as u8
}

/// The whole-stack move that puts `item` on the belt from the pack: an
/// empty belt slot, else the last one (the stacks swap).
pub(crate) fn belt_move(core: &ClientCore, item: u16) -> Option<(u8, u8, u16)> {
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

    /// The next move; a failure is remembered where it happened (in
    /// `Home`, which outlives the job) so the goal is not chosen again on
    /// the same spot.
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
        let step = self.next(core, seed, haven, body, hands, home, tick);
        if let Do::Fail(_) = step {
            home.bag_failed([body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q], tick);
        }
        step
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
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

    /// The next move. A recovery that ends with the backpack still
    /// standing (the pack took what fitted, the presses went unanswered,
    /// no way there) marks it tried in `Home`, so it is not chosen again
    /// until there is more room.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        route: &mut Route,
        hands: &Hands,
        home: &mut Home,
        hostile: bool,
        gained: u32,
        tick: u32,
    ) -> Do {
        let step = self.next(core, body, route, hands, hostile, gained, tick);
        let ended = matches!(step, Do::Done | Do::Fail(Why::Refused | Why::Stuck));
        if ended && death_bag(core).is_some() {
            home.pack_tried(core);
        }
        step
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
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
        // A fight by a bag is no call to defend a base: there is none here.
        home.on_shot([x + 5.0, z], 30);
        assert!(home.under_attack(30) && !home.alarmed(30, 10));
        // Nor by a bag put down away from the base.
        home.set_base(Some([x + 200.0, z]));
        home.on_hurt([x + 5.0, z], 40);
        assert!(home.under_attack(40) && !home.alarmed(40, 10));
        home.on_shot([x + 205.0, z], 50);
        assert!(home.alarmed(50, 10), "shots at the base itself");
        // The own-bag list is the truth.
        home.on_bags(&[]);
        assert_eq!(home.bags(), 0);
        // A refusal answers only a deploy in flight.
        home.on_refused(3);
        assert_eq!(home.take_verdict(), None);
        home.asked(5, 5, 0, LOC_PLANE);
        home.on_refused(3);
        assert_eq!(home.take_verdict(), Some(Placed::Refused(3)));
    }

    #[test]
    fn under_attack_it_wakes_on_a_bag_away_from_the_fight_or_on_the_beach() {
        let mut home = Home::new();
        let (hx, hz) = cell_center(100, 100);
        home.set_base(Some([hx, hz]));
        let bag = |cx: u16, ready: bool| BagAnchor {
            cx,
            cz: 100,
            level: 0,
            ready,
        };
        let (near, far) = (bag(100, true), bag(140, true));
        assert!(home.wake_on(&[near], 10, [hx, hz]), "home is safe");
        assert!(
            !home.wake_on(&[bag(100, false)], 10, [hx, hz]),
            "none ready"
        );
        home.on_struck(20);
        // The game wakes it on the ready bag nearest where it fell.
        assert!(!home.wake_on(&[near], 30, [hx, hz]), "only the home bag");
        assert!(
            !home.wake_on(&[near, far], 30, [hx, hz]),
            "the home bag is nearer"
        );
        let (fx, _) = cell_center(140, 100);
        assert!(
            home.wake_on(&[near, far], 30, [fx - 20.0, hz]),
            "the bag elsewhere"
        );
        assert!(
            home.wake_on(&[near], 20 + HOME_ALARM_TICKS, [hx, hz]),
            "quiet again"
        );
    }

    #[test]
    fn a_bag_that_failed_here_waits_for_a_walk_or_a_while_and_the_cap_sticks() {
        let mut home = Home::new();
        home.bag_failed([100.0, 100.0], 50);
        assert!(home.bag_held([103.0, 100.0], 51), "the same ground again");
        assert!(!home.bag_held([100.0 + BAG_RETRY_M + 1.0, 100.0], 51));
        assert!(!home.bag_held([100.0, 100.0], 50 + BAG_RETRY_TICKS));
        // Bags from an earlier session fill the cap: the server's word
        // stands until the next own-bag list (the join's, a bag placed or
        // taken down, a wake, a death) names them.
        home.asked(5, 5, 0, LOC_PLANE);
        home.on_refused(REFUSE_D_BAG_CAP as u8);
        assert_eq!((home.bags(), home.bags_known()), (0, BAG_CAP as u8));
        home.on_bags(&[]);
        assert_eq!(home.bags_known(), 0);
    }

    #[test]
    fn a_backpack_that_gave_all_it_could_waits_for_room_or_a_new_death() {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let mut home = Home::new();
        for s in core.inv.iter_mut() {
            (s.item, s.count) = (1, 1);
        }
        core.own_bag = 7;
        assert!(home.recover_held(&core), "no room for anything");
        core.inv[3].count = 0;
        assert!(!home.recover_held(&core));
        // The press took what fitted; the rest stays in the backpack.
        home.pack_tried(&core);
        assert!(home.recover_held(&core), "the same pack, no more room");
        core.inv[4].count = 0;
        assert!(!home.recover_held(&core), "room was made");
        home.pack_tried(&core);
        core.own_bag = 8;
        assert!(!home.recover_held(&core), "a new death, a new backpack");
    }

    #[test]
    fn blows_blasts_and_breaks_at_home_raise_the_alarm_and_upkeep_goes_by_the_last_feed() {
        let mut home = Home::new();
        home.set_base(Some([300.0, 300.0]));
        home.on_blast([300.0 + HOME_ALARM_M + 1.0, 300.0], 10);
        assert!(!home.under_attack(10), "a blast somewhere else");
        home.on_blast([310.0, 300.0], 20);
        assert!(home.under_attack(20), "a blast at home");
        assert_eq!(home.raided(20, 5), Some([300.0, 300.0]));
        home.on_hurt([305.0, 305.0], 30);
        assert_eq!(home.alarm, Some(30), "a blow by the base, no bag needed");
        assert_eq!(home.raided(30, 5), None, "a bite by the base is no raid");
        home.on_struck(31);
        assert_eq!(home.raided(31, 5), Some([300.0, 300.0]), "a blow on it");
        assert!(home.alarmed(31, 5));
        home.settle();
        assert!(!home.alarmed(31, 5), "seen through");
        assert!(home.under_attack(31), "still no bag to wake on there");
        let mut home = Home::new();
        home.on_struck(5);
        assert!(home.under_attack(5), "a hit on its own wall");
        // Upkeep: twig is never charged; a graded base never read is fed.
        let mut home = Home::new();
        assert!(!home.upkeep_due(0, 0));
        assert!(home.upkeep_due(0, 3));
        // Without a reading, a piece coming down may be rot.
        home.on_removed(5, 3);
        assert!(!home.under_attack(5));
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        // 500 held against a day's bill of 240: 50 hours.
        core.stock[0] = (50, 500, 240);
        core.stock[1] = (58, 0, 0);
        core.stock_count = 2;
        home.on_stock(&core, 100, 3);
        let r = home.upkeep().unwrap();
        assert_eq!(r.periods_left(100), Some(50));
        assert!(!home.upkeep_due(100, 3));
        let period = UPKEEP_PERIOD_TICKS as u32;
        assert!(!home.upkeep_due(100 + (50 - UPKEEP_LOW_PERIODS) * period - 1, 3));
        assert!(home.upkeep_due(100 + (50 - UPKEEP_LOW_PERIODS + 1) * period, 3));
        // Fed, nothing rots: what comes down was broken. Not so once the
        // base grew past the reading: a bigger bill may have eaten it.
        home.on_removed(200, 4);
        assert!(!home.under_attack(200));
        home.on_removed(200, 3);
        assert!(home.under_attack(200));
        // The reading names what it eats, and what of that is charged:
        // wood the cupboard would eat uncharged is not fed while the
        // stone is not in the pack.
        let low = 100 + (50 - UPKEEP_LOW_PERIODS + 1) * period;
        let carried = |core: &ClientCore| {
            let inv = core.inv;
            move |item: u16| inv.iter().any(|s| s.count > 0 && s.item == item)
        };
        assert!(!home.can_feed(carried(&core), low));
        // What it wants in the pack for that feed: the low stone.
        let mut rows = [(0, 0); HEARTH_STOCK_ROWS];
        assert_eq!(home.feed_rows(100, 3, &mut rows), 0, "not due yet");
        assert_eq!(home.feed_rows(low, 3, &mut rows), 1);
        assert_eq!(rows[0], (50, FEED_CHUNK));
        core.inv[7] = sim_core::gather::ItemStack {
            item: 58,
            count: 5,
            ..Default::default()
        };
        assert!(
            !home.can_feed(carried(&core), low),
            "only the uncharged wood"
        );
        core.inv[8] = sim_core::gather::ItemStack {
            item: 50,
            count: 5,
            ..Default::default()
        };
        assert!(
            !home.can_feed(carried(&core), 100),
            "the stone is not low yet"
        );
        assert!(home.can_feed(carried(&core), low));
        // A reading that charged nothing is taken again once the base grew.
        core.stock[0] = (50, 500, 0);
        home.on_stock(&core, 300, 3);
        assert!(!home.upkeep_due(300, 3));
        assert!(home.upkeep_due(300, 4));
        assert_eq!(home.feed_rows(300, 4, &mut rows), 2, "all it eats");
        // Its cupboard came down: a new one there starts unread.
        home.lost_hearth();
        assert_eq!(home.upkeep(), None);
        assert!(home.upkeep_due(300, 3));
        // A visit that failed waits a while.
        home.stash_failed(1000);
        assert!(home.stash_held(1000 + STASH_RETRY_TICKS - 1));
        assert!(!home.stash_held(1000 + STASH_RETRY_TICKS));
    }

    /// A feed takes a chunk of all the cupboard eats: what a current
    /// reading charges nothing in, while it charges something, is fed for
    /// nothing (the kit's fragments under a base of wood and stone). Never
    /// read, read as charging nothing, or read before the base grew: not
    /// known.
    #[test]
    fn a_feed_wastes_what_a_current_reading_charges_nothing_in() {
        const FRAGS: u16 = 49;
        const STONE: u16 = 66;
        const WOOD: u16 = 77;
        const CLOTH: u16 = 22;
        let mut home = Home::new();
        assert!(!home.feed_wastes(FRAGS, 22), "never read");
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        core.stock[0] = (FRAGS, 0, 0);
        core.stock[1] = (STONE, 0, 297);
        core.stock[2] = (WOOD, 0, 60);
        core.stock_count = 3;
        home.on_stock(&core, 100, 22);
        assert!(home.feed_wastes(FRAGS, 22));
        assert!(!home.feed_wastes(STONE, 22) && !home.feed_wastes(WOOD, 22));
        assert!(!home.feed_wastes(CLOTH, 22), "not eaten at all");
        assert!(!home.feed_wastes(FRAGS, 23), "the base grew since");
        core.stock[1].2 = 0;
        core.stock[2].2 = 0;
        home.on_stock(&core, 200, 22);
        assert!(!home.feed_wastes(FRAGS, 22), "a reading charging nothing");
    }
}
