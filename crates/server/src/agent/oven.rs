//! Ovens: the fire, the recycler and the research table, worked the way a
//! player works them. The sim makes them one class (`sim_core::oven`): a
//! container a press switches on, that turns what is put in it into
//! something else while it runs. So the session at one is one routine
//! ([`Session`]): open it with `E`, move stacks in and out with its panel
//! open, switch it on with the same key the human client's `C` sends, watch
//! it, switch it off, shut the panel. What goes in and what comes out is
//! the [`Work`]:
//!
//! - **Cook**: raw meat off a kill is no food until a fire has had it, and
//!   cooked meat left on burns (`content/cooking.toml`). Wood goes in, and a
//!   piece of meat on each free slot; each piece comes off as soon as it is
//!   done; the fire is put out when the last is off.
//! - **Recycle**: gears, rope and tarp go into a recycler, and junk,
//!   fragments and cloth come out; it is switched off when the last is
//!   taken apart.
//! - **Research**: a looted sample and the junk it costs go into a research
//!   table; the paper it makes comes out, to be read (`agent::build`'s
//!   blueprint op does the reading, and puts the table up).
//!
//! [`DeviceJob`] is a cook or a recycle as a goal: to its own fire or
//! recycler near here, or one it has seen, or one it puts down (a fire pit
//! crafted first), and a [`Session`] there. Everything it knows of a device
//! is what its panel shows and the lit set the server announces; what turns
//! into what is the wiki's (`agent::wiki::Page::{cooks, recycles}`).

use crate::agent::build::{aim_point, e_picks_by, item_named, recipe_for, Stations};
use crate::agent::hands::Hands;
use crate::agent::home::{belt_move, HOLD_TICKS, SPOT_TRIES};
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::route::{Route, Step};
use crate::agent::site::foundation_goes;
use crate::agent::wiki::{Book, Class};
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::build::{build_cell_of, LOC_PLANE};
use sim_core::deploy::{box_key, cell_center, REFUSE_D_COST};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::inventory::CONT_BOX;
use sim_core::limits::{BOX_SLOTS, HOTBAR_SLOTS, INV_SLOTS, MAX_BUILD_COORD, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::research::{TABLE_COIN_SLOT, TABLE_ITEM_SLOT};
use sim_core::terrain::Haven;

/// The deployables the work is done at, by catalog name.
pub const FIRE_ITEM: &str = "Fire Pit";
pub const RECYCLER_ITEM: &str = "Recycler";
pub const TABLE_ITEM: &str = "Research Table";
/// Pieces of meat on the fire at once, a slot each: they cook side by side.
pub const COOK_SLOTS: usize = 8;
/// Fuel laid in at a time, and the least it is topped up from: a unit burns
/// a few seconds, a piece of meat cooks in twenty.
pub const FUEL_LOAD: u16 = 20;
const FUEL_LOW: u32 = 4;
/// Fuel a cook wants in the pack before it is worth going.
pub const FUEL_MIN: u32 = 10;
/// Its own device this near is walked to rather than another put down; a
/// recycler it has seen, this near, is walked to.
pub const KEEP_M: f32 = 80.0;
pub const SEEN_NEAR_M: f32 = 200.0;
/// It works a device from this far off its middle.
pub const STAND_M: f32 = 1.8;
const STAND_SLACK_M: f32 = 0.6;
/// A device goes down this far from the feet, in front.
const PUT_NEAR_M: f32 = 1.5;
const PUT_FAR_M: f32 = 3.5;
/// An answer to a deploy, an open, a move or a switch comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;
/// After a move is answered, the panel catches up this soon.
const SETTLE_TICKS: u32 = 3;
/// Moves one session makes at most.
pub const MAX_MOVES: u8 = 64;
/// Presses that went unanswered or that `E` would not take, before the
/// session gives up.
const MAX_TRIES: u8 = 3;
/// A cook or a recycle that came to nothing is not offered again for this
/// long.
pub const RETRY_TICKS: u32 = 60 * TICK_HZ;
/// The drift left at a press, once the eyes have settled.
const PRESS_SLACK_M: f32 = 0.1;

/// What a session at a device is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Cook,
    Recycle,
    /// Paper for this sample, paid with `cost` of the research coin.
    Research {
        sample: u16,
        cost: u16,
    },
}

impl Work {
    /// The device it is done at.
    pub fn device(self) -> &'static str {
        match self {
            Work::Cook => FIRE_ITEM,
            Work::Recycle => RECYCLER_ITEM,
            Work::Research { .. } => TABLE_ITEM,
        }
    }

    /// The device stops on its own (a research lands); the others run until
    /// they are switched off.
    fn stops_itself(self) -> bool {
        matches!(self, Work::Research { .. })
    }
}

/// What a device job or session wants next. `explorer.rs` (or the builder)
/// sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tend {
    Go(Intent),
    /// Craft the device (`encode_action_craft`).
    Craft {
        recipe: u16,
    },
    /// Move a stack to the belt (`encode_action_move`).
    Belt {
        from: u8,
        to: u8,
        count: u16,
    },
    /// Put the device down, it in hand (`encode_action_deploy`).
    Deploy {
        row: u16,
        cx: u16,
        cz: u16,
        intent: Intent,
    },
    /// Open the device (`encode_action_container`).
    Open {
        key: u32,
        intent: Intent,
    },
    /// Move a stack with the device's panel open (`encode_action_move`):
    /// into it from the pack, or out of it into the pack.
    Move {
        key: u32,
        into: bool,
        from: u8,
        to: u8,
        count: u16,
        intent: Intent,
    },
    /// Switch it on or off (`encode_action_use`).
    Switch {
        at: OpAddr,
        intent: Intent,
    },
    /// Shut the panel: the session is over once this is sent.
    Close(Intent),
    Done,
    Fail(Why),
}

/// What working devices has come to.
#[derive(Clone, Copy, Debug, Default)]
pub struct OvenStats {
    pub sessions: u64,
    /// Devices put down.
    pub placed: u64,
    pub laid: u64,
    pub cooked: u64,
    pub burnt: u64,
    /// Stacks put into a recycler.
    pub recycled: u64,
    /// Paper taken out of a research table.
    pub papers: u64,
}

/// Units of an item in the pack and belt.
fn carried(core: &ClientCore, item: u16) -> u32 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

fn units(slots: &[ItemStack], item: u16) -> u32 {
    slots
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// Something the fire is wanted for: it cooks into food and is not food
/// yet (raw meat; not cooked meat, which only burns).
pub fn raw(book: &Book, item: u16) -> bool {
    let page = book.page(item);
    page.cooks != NO_ITEM && page.class != Class::Food && book.page(page.cooks).class == Class::Food
}

/// What a session puts into the device for its work, from the pack.
fn feeds(work: Work, book: &Book, item: u16) -> bool {
    match work {
        Work::Cook => raw(book, item),
        Work::Recycle => book.page(item).recycles,
        Work::Research { sample, .. } => item == sample,
    }
}

/// Units of what a work feeds a device that the pack holds.
pub fn held_for(work: Work, core: &ClientCore, book: &Book) -> u32 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count > 0 && feeds(work, book, s.item))
        .map(|s| u32::from(s.count))
        .sum()
}

/// The deployable row a device is, and its item.
pub fn device_row(core: &ClientCore, name: &str) -> Option<(u16, u16)> {
    let item = item_named(core, name)?;
    let defs = &core.deploy_defs;
    (0..core.deploy_defs_have.min(defs.def_count))
        .find(|&row| {
            let def = defs.defs[usize::from(row)];
            def.hp > 0 && def.item == item
        })
        .map(|row| (row, item))
}

/// The device a work is done at stands at this ground cell, per the mirror.
pub fn stands(core: &ClientCore, work: Work, (cx, cz): (u16, u16)) -> bool {
    let Some((row, _)) = device_row(core, work.device()) else {
        return false;
    };
    core.deploys
        .entries()
        .iter()
        .any(|d| (d.cx, d.cz, d.level, d.loc) == (cx, cz, 0, LOC_PLANE) && u16::from(d.row) == row)
}

/// The nearest of these cells holding the work's device within `within`
/// metres of (`x`, `z`).
pub fn nearest(
    core: &ClientCore,
    work: Work,
    cells: impl Iterator<Item = (u16, u16)>,
    x: f32,
    z: f32,
    within: f32,
) -> Option<(u16, u16)> {
    let mut best: Option<(f32, (u16, u16))> = None;
    for cell in cells {
        let (mx, mz) = cell_center(cell.0, cell.1);
        let d = (mx - x).hypot(mz - z);
        if d <= within && best.is_none_or(|(b, _)| d < b) && stands(core, work, cell) {
            best = Some((d, cell));
        }
    }
    best.map(|(_, cell)| cell)
}

/// Can a cook or a recycle go now: what the work takes, in the pack (and
/// fuel for a fire), and a device to do it at: one in `near` (its own or
/// one seen, already found standing near), one in the pack, or (a fire
/// pit) the wood to make one.
pub fn can_tend(core: &ClientCore, book: &Book, work: Work, near: bool) -> bool {
    if !book.ready() || held_for(work, core, book) == 0 {
        return false;
    }
    let fuel = book.fuel();
    if work == Work::Cook && (fuel == NO_ITEM || carried(core, fuel) < FUEL_MIN) {
        return false;
    }
    if near {
        return true;
    }
    let Some((_, item)) = device_row(core, work.device()) else {
        return false;
    };
    if carried(core, item) > 0 {
        return true;
    }
    // A fire pit is made from the fuel itself, anywhere: enough for the pit
    // and a fire's worth.
    work == Work::Cook
        && recipe_for(core, item, &Stations::NONE).is_some_and(|(r, ..)| {
            let def = core.recipes.recipes[usize::from(r)];
            def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())]
                .iter()
                .all(|&(input, need)| {
                    let extra = if input == fuel { FUEL_MIN } else { 0 };
                    carried(core, input) >= u32::from(need) + extra
                })
        })
}

/// A cell a device goes down on, in front of the body: bare ground the
/// foundation's terrain rule takes, nothing built or put down on it, clear
/// of `avoid` (its own base), and not one refused already.
fn put_cell(
    core: &ClientCore,
    seed: u64,
    haven: &Haven,
    body: &EntityState,
    refused: &[(u16, u16)],
    avoid: impl Fn(u16, u16) -> bool,
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
            if !(PUT_NEAR_M..=PUT_FAR_M).contains(&d)
                || refused.contains(&(cx, cz))
                || best.is_some_and(|(b, ..)| score >= b)
                || avoid(cx, cz)
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

/// Where `count` of `item` lands among `slots` (tried in `order`): onto a
/// stack of the same item with room, else an empty slot.
fn land(
    slots: &[ItemStack],
    order: impl Iterator<Item = usize> + Clone,
    item: u16,
    count: u16,
    cap: u16,
) -> Option<(u8, u16)> {
    if cap == 0 || count == 0 {
        return None;
    }
    let count = count.min(cap);
    let merge = order.clone().find_map(|i| {
        let d = slots[i];
        let room = cap.saturating_sub(d.count);
        (d.count > 0 && d.item == item && room > 0).then(|| (i as u8, count.min(room)))
    });
    merge.or_else(|| {
        order
            .clone()
            .find(|&i| slots[i].count == 0)
            .map(|i| (i as u8, count))
    })
}

/// One move at an open device: `(into the device, from slot, to slot,
/// count)`.
pub type Shift = (bool, u8, u8, u16);

/// The next move at an open device for its work, or `None` when there is
/// nothing to move. Whatever the device made comes off first.
///
/// - Cook: off whatever is done (cooked food, burnt food, charcoal), before
///   it burns; then fuel when it runs low; then a piece of raw food onto
///   each free slot, up to [`COOK_SLOTS`].
/// - Recycle: off whatever it took apart; then each stack of what it
///   takes apart onto a free slot.
/// - Research: off the paper; nothing while it runs (the table is locked);
///   the sample, one unit, into the item slot and the coin it costs into
///   the coin slot while there is no paper for it yet; the coin left over
///   back out once there is.
pub fn plan(
    work: Work,
    core: &ClientCore,
    book: &Book,
    slots: &[ItemStack],
    lit: bool,
) -> Option<Shift> {
    let cap = |item: u16| core.catalog.row(usize::from(item)).stack_max;
    let pack = &core.inv[..INV_SLOTS];
    let into_pack = (HOTBAR_SLOTS..INV_SLOTS).chain(0..HOTBAR_SLOTS);
    let take = |i: usize, s: &ItemStack| {
        land(pack, into_pack.clone(), s.item, s.count, cap(s.item))
            .map(|(to, count)| (false, i as u8, to, count))
    };
    let from_pack = |want: &dyn Fn(u16) -> bool| {
        (0..INV_SLOTS).find(|&i| pack[i].count > 0 && want(pack[i].item))
    };
    match work {
        Work::Cook => {
            let fuel = book.fuel();
            for (i, s) in slots.iter().enumerate() {
                if s.count > 0 && s.item != fuel && !raw(book, s.item) {
                    if let Some(shift) = take(i, s) {
                        return Some(shift);
                    }
                }
            }
            if units(slots, fuel) < FUEL_LOW {
                if let Some(from) = from_pack(&|item| item == fuel) {
                    let want = FUEL_LOAD
                        .saturating_sub(units(slots, fuel) as u16)
                        .min(pack[from].count);
                    if let Some((to, count)) = land(slots, 0..slots.len(), fuel, want, cap(fuel)) {
                        return Some((true, from as u8, to, count));
                    }
                }
            }
            let cooking = slots
                .iter()
                .filter(|s| s.count > 0 && raw(book, s.item))
                .count();
            if cooking >= COOK_SLOTS {
                return None;
            }
            let from = from_pack(&|item| raw(book, item))?;
            let to = slots.iter().position(|s| s.count == 0)?;
            Some((true, from as u8, to as u8, 1))
        }
        Work::Recycle => {
            for (i, s) in slots.iter().enumerate() {
                if s.count > 0 && !book.page(s.item).recycles {
                    if let Some(shift) = take(i, s) {
                        return Some(shift);
                    }
                }
            }
            let from = from_pack(&|item| book.page(item).recycles)?;
            let to = slots.iter().position(|s| s.count == 0)?;
            Some((true, from as u8, to as u8, pack[from].count))
        }
        Work::Research { sample, cost } => {
            let rc = &core.research;
            let paper =
                |s: &ItemStack| sim_core::research::blueprint_target(rc, *s) == Some(sample);
            if lit {
                return None;
            }
            let item = slots[TABLE_ITEM_SLOT];
            let coin = slots[TABLE_COIN_SLOT];
            if item.count > 0 && (paper(&item) || item.item != sample) {
                return take(TABLE_ITEM_SLOT, &item);
            }
            let done = pack.iter().any(paper);
            if !done && item.count == 0 {
                if let Some(from) = from_pack(&|i| i == sample) {
                    return Some((true, from as u8, TABLE_ITEM_SLOT as u8, 1));
                }
            }
            let have = if coin.item == rc.coin { coin.count } else { 0 };
            if !done && item.count > 0 && have < cost {
                let from = from_pack(&|i| i == rc.coin)?;
                let count = (cost - have).min(pack[from].count);
                return Some((true, from as u8, TABLE_COIN_SLOT as u8, count));
            }
            if coin.count > 0 && (done || item.count == 0) {
                return take(TABLE_COIN_SLOT, &coin);
            }
            None
        }
    }
}

/// The device should be running: what it works on is in it (and, at a
/// research table, paid for).
pub fn should_run(work: Work, core: &ClientCore, book: &Book, slots: &[ItemStack]) -> bool {
    match work {
        Work::Cook => slots.iter().any(|s| s.count > 0 && raw(book, s.item)),
        Work::Recycle => slots
            .iter()
            .any(|s| s.count > 0 && book.page(s.item).recycles),
        Work::Research { sample, cost } => {
            let (item, coin) = (slots[TABLE_ITEM_SLOT], slots[TABLE_COIN_SLOT]);
            item.count == 1
                && item.item == sample
                && coin.item == core.research.coin
                && coin.count >= cost
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    Craft,
    Belt,
    Deploy,
    Open,
    Move,
    /// A switch, wanting it on or off.
    Switch(bool),
    Close,
}

/// One session at a device, with the body standing in reach of it: open,
/// move, switch, watch, switch off, shut.
#[derive(Clone, Copy, Debug)]
pub struct Session {
    at: OpAddr,
    want: Option<Wait>,
    waiting: Option<(Wait, u32)>,
    held: Option<u32>,
    moved: Option<(bool, u32)>,
    opened: bool,
    fresh: bool,
    moves: u8,
    tries: u8,
}

impl Session {
    pub fn new(at: OpAddr) -> Self {
        Self {
            at,
            want: None,
            waiting: None,
            held: None,
            moved: None,
            opened: false,
            fresh: false,
            moves: 0,
            tries: 0,
        }
    }

    pub fn at(&self) -> OpAddr {
        self.at
    }

    /// The device's container handle.
    pub fn key(&self) -> u32 {
        box_key(self.at.cx, self.at.cz, self.at.level)
    }

    /// Nothing is in flight.
    pub fn at_checkpoint(&self) -> bool {
        self.waiting.is_none() && self.want.is_none()
    }

    /// The last turn's verb went out.
    pub fn sent(&mut self, tick: u32) {
        if let Some(w) = self.want.take() {
            self.waiting = Some((w, tick));
            match w {
                Wait::Move => {
                    self.moved = None;
                    self.moves = self.moves.saturating_add(1);
                }
                Wait::Open => self.fresh = false,
                _ => {}
            }
        }
    }

    /// A panel showed this device.
    pub fn on_panel(&mut self, handle: u32) {
        if handle == self.key() {
            self.fresh = true;
        }
    }

    /// A move landed or was refused.
    pub fn on_moved(&mut self, refused: bool, tick: u32) {
        if matches!(self.waiting, Some((Wait::Move, _))) {
            self.moved = Some((refused, tick));
        }
    }

    /// The next turn of the session.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        book: &Book,
        work: Work,
        stats: &mut OvenStats,
        tick: u32,
    ) -> Tend {
        let turn = self.next(core, seed, haven, body, hands, book, work, stats, tick);
        self.want = match turn {
            Tend::Open { .. } => Some(Wait::Open),
            Tend::Move { .. } => Some(Wait::Move),
            Tend::Switch { .. } => {
                let lit = core.ovens().is_lit(self.at.cx, self.at.cz, self.at.level);
                Some(Wait::Switch(!lit))
            }
            Tend::Close(_) => Some(Wait::Close),
            _ => None,
        };
        turn
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        book: &Book,
        work: Work,
        stats: &mut OvenStats,
        tick: u32,
    ) -> Tend {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let at = self.at;
        let key = self.key();
        let look = Intent {
            look: Look::Point(aim_point(core, seed, haven, at, x, z)),
            ..Intent::IDLE
        };
        let open = core.cont_kind == CONT_BOX && core.cont_handle == key;
        let lit = core.ovens().is_lit(at.cx, at.cz, at.level);
        if let Some((wait, since)) = self.waiting {
            let late = tick.wrapping_sub(since) >= VERDICT_TICKS;
            match wait {
                Wait::Open => {
                    if self.fresh && open {
                        self.waiting = None;
                        self.opened = true;
                        self.tries = 0;
                        stats.sessions += 1;
                    } else if late {
                        self.waiting = None;
                        self.tries += 1;
                    } else {
                        return Tend::Go(look);
                    }
                }
                Wait::Move => match self.moved {
                    Some((refused, when)) => {
                        if tick.wrapping_sub(when) < SETTLE_TICKS {
                            return Tend::Go(look);
                        }
                        self.waiting = None;
                        self.moved = None;
                        if refused {
                            self.tries += 1;
                        }
                    }
                    None if late => {
                        self.waiting = None;
                        self.tries += 1;
                    }
                    None => return Tend::Go(look),
                },
                // The lit set says when it caught (or went out); a research
                // table may land and go quiet before the news of its start.
                Wait::Switch(on) => {
                    if lit == on || late || (on && work.stops_itself() && self.landed(core, work)) {
                        self.waiting = None;
                        if lit != on && late {
                            self.tries += 1;
                        }
                    } else {
                        return Tend::Go(look);
                    }
                }
                Wait::Close => {
                    self.waiting = None;
                    return Tend::Done;
                }
                Wait::Craft | Wait::Belt | Wait::Deploy => self.waiting = None,
            }
        }
        if self.tries >= MAX_TRIES {
            return Tend::Fail(Why::Refused);
        }
        if self.opened && !open {
            // It was open and the server shut it: out of reach, or gone.
            return Tend::Fail(Why::Stuck);
        }
        if !self.opened {
            return if self.press(core, hands, x, z, tick) {
                Tend::Open { key, intent: look }
            } else {
                Tend::Go(look)
            };
        }
        if self.moves >= MAX_MOVES {
            return Tend::Close(look);
        }
        let slots = &core.cont[..BOX_SLOTS];
        if let Some((into, from, to, count)) = plan(work, core, book, slots, lit) {
            if into {
                match work {
                    Work::Cook if raw(book, core.inv[usize::from(from)].item) => stats.laid += 1,
                    Work::Recycle => stats.recycled += 1,
                    _ => {}
                }
            } else {
                let s = slots[usize::from(from)];
                let page = book.page(s.item);
                if page.class == Class::Food && page.cooks == NO_ITEM {
                    stats.burnt += u64::from(count);
                } else if page.class == Class::Food {
                    stats.cooked += u64::from(count);
                }
                if sim_core::research::blueprint_target(&core.research, s).is_some() {
                    stats.papers += 1;
                }
            }
            return Tend::Move {
                key,
                into,
                from,
                to,
                count,
                intent: look,
            };
        }
        let run = should_run(work, core, book, slots);
        let switch = (run && !lit) || (!run && lit && !work.stops_itself());
        if switch {
            return if self.press(core, hands, x, z, tick) {
                Tend::Switch { at, intent: look }
            } else {
                Tend::Go(look)
            };
        }
        if run || lit {
            // At work: watch it, and take off what it makes.
            return Tend::Go(look);
        }
        Tend::Close(look)
    }

    /// A research table has made the paper: it sits in the item slot.
    fn landed(&self, core: &ClientCore, work: Work) -> bool {
        let Work::Research { sample, .. } = work else {
            return false;
        };
        sim_core::research::blueprint_target(&core.research, core.cont[TABLE_ITEM_SLOT])
            == Some(sample)
    }

    /// The press at the device, once the eyes have settled on it for a
    /// moment and it is what a player's `E` (or the switch's key) would
    /// take. A device `E` will not take from here, settled on this long,
    /// counts a try.
    fn press(&mut self, core: &ClientCore, hands: &Hands, x: f32, z: f32, tick: u32) -> bool {
        let held = *self.held.get_or_insert(tick);
        let waited = tick.wrapping_sub(held);
        if waited >= HOLD_TICKS
            && hands.settled()
            && e_picks_by(core, x, z, hands.view().0, self.at, PRESS_SLACK_M)
        {
            self.held = None;
            return true;
        }
        if waited >= VERDICT_TICKS {
            self.held = None;
            self.tries += 1;
        }
        false
    }
}

fn ground_addr((cx, cz): (u16, u16)) -> OpAddr {
    OpAddr {
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }
}

/// A cook or a recycle as a goal: to a device, putting one down when there
/// is none to go to, and a session there.
#[derive(Clone, Copy, Debug)]
pub struct DeviceJob {
    work: Work,
    /// The ground cell of the device this job works.
    device: Option<(u16, u16)>,
    /// The cell one was asked to go down on, and whether it did.
    placing: Option<(u16, u16)>,
    placed: bool,
    refused: [(u16, u16); SPOT_TRIES],
    spots: usize,
    want: Option<Wait>,
    waiting: Option<(Wait, u32)>,
    /// The device crafted for, and how many were held before.
    making: Option<(u16, u32)>,
    held: Option<u32>,
    deploy_refused: Option<u8>,
    session: Option<Session>,
}

impl DeviceJob {
    pub fn new(work: Work) -> Self {
        Self {
            work,
            device: None,
            placing: None,
            placed: false,
            refused: [(0, 0); SPOT_TRIES],
            spots: 0,
            want: None,
            waiting: None,
            making: None,
            held: None,
            deploy_refused: None,
            session: None,
        }
    }

    pub fn work(&self) -> Work {
        self.work
    }

    /// Nothing is in flight: switching now leaves at worst a panel open,
    /// which walking away shuts, and a device running, which runs down.
    pub fn at_checkpoint(&self) -> bool {
        self.waiting.is_none()
            && self.want.is_none()
            && self.session.is_none_or(|s| s.at_checkpoint())
    }

    /// The device this job put down, once the server said so.
    pub fn placed(&self) -> Option<(u16, u16)> {
        self.placed.then_some(self.device).flatten()
    }

    /// The last step's verb went out.
    pub fn sent(&mut self, tick: u32) {
        if let Some(w) = self.want.take() {
            self.waiting = Some((w, tick));
            if w == Wait::Deploy {
                self.deploy_refused = None;
            }
        } else if let Some(s) = self.session.as_mut() {
            s.sent(tick);
        }
    }

    /// A placement broadcast: the device went down where it was asked.
    pub fn on_placed(&mut self, cx: u16, cz: u16, level: u8, loc: u8, deploy: bool) {
        if deploy && level == 0 && loc == LOC_PLANE && self.placing == Some((cx, cz)) {
            self.placing = None;
            self.placed = true;
            self.device = Some((cx, cz));
        }
    }

    /// A deploy refusal, while a device is in flight.
    pub fn on_refused(&mut self, reason: u8) {
        if matches!(self.waiting, Some((Wait::Deploy, _))) {
            self.deploy_refused = Some(reason);
        }
    }

    /// A panel showed a container.
    pub fn on_panel(&mut self, handle: u32) {
        if let Some(s) = self.session.as_mut() {
            s.on_panel(handle);
        }
    }

    /// A move landed or was refused.
    pub fn on_moved(&mut self, refused: bool, tick: u32) {
        if let Some(s) = self.session.as_mut() {
            s.on_moved(refused, tick);
        }
    }

    /// The next step of the job. `go_to` is the device to go to when there
    /// is one (its own near here, or one seen); `avoid` the cells a new one
    /// may not go on.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        book: &Book,
        go_to: Option<(u16, u16)>,
        avoid: impl Fn(u16, u16) -> bool,
        stats: &mut OvenStats,
        tick: u32,
    ) -> Tend {
        let step = self.next(
            core, seed, haven, body, hands, route, book, go_to, avoid, stats, tick,
        );
        self.want = match step {
            Tend::Craft { .. } => Some(Wait::Craft),
            Tend::Belt { .. } => Some(Wait::Belt),
            Tend::Deploy { .. } => Some(Wait::Deploy),
            _ => None,
        };
        if let Tend::Deploy { cx, cz, .. } = step {
            self.placing = Some((cx, cz));
        }
        step
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        book: &Book,
        go_to: Option<(u16, u16)>,
        avoid: impl Fn(u16, u16) -> bool,
        stats: &mut OvenStats,
        tick: u32,
    ) -> Tend {
        if !book.ready() {
            return Tend::Fail(Why::NotFound);
        }
        let work = self.work;
        let Some((row, item)) = device_row(core, work.device()) else {
            return Tend::Fail(Why::NoRecipe);
        };
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        // What went out is waiting on its answer.
        if let Some((wait, since)) = self.waiting {
            let late = tick.wrapping_sub(since) >= VERDICT_TICKS;
            match wait {
                Wait::Craft => match self.making {
                    Some((made, before)) if carried(core, made) > before => {
                        self.waiting = None;
                        self.making = None;
                    }
                    // It takes its craft time: waited for, up to a point.
                    _ if tick.wrapping_sub(since) < VERDICT_TICKS + 10 * TICK_HZ => {
                        return Tend::Go(Intent::IDLE);
                    }
                    _ => return Tend::Fail(Why::NoAnswer),
                },
                Wait::Belt => {
                    if (0..HOTBAR_SLOTS).any(|i| core.inv[i].count > 0 && core.inv[i].item == item)
                    {
                        self.waiting = None;
                    } else if late {
                        return Tend::Fail(Why::NoAnswer);
                    } else {
                        return Tend::Go(Intent::IDLE);
                    }
                }
                Wait::Deploy => {
                    if self.placed {
                        self.waiting = None;
                        stats.placed += 1;
                    } else if let Some(reason) = self.deploy_refused.take() {
                        self.waiting = None;
                        if u32::from(reason) == REFUSE_D_COST {
                            return Tend::Fail(Why::MissingInputs);
                        }
                        // The ground, the reach or a claim spoiled this
                        // spot: another one.
                        if let Some(cell) = self.placing.take() {
                            if self.spots < SPOT_TRIES {
                                self.refused[self.spots] = cell;
                            }
                        }
                        self.spots += 1;
                        if self.spots >= SPOT_TRIES {
                            return Tend::Fail(Why::NoSpot);
                        }
                    } else if late {
                        return Tend::Fail(Why::NoAnswer);
                    } else {
                        return Tend::Go(Intent::IDLE);
                    }
                }
                _ => self.waiting = None,
            }
        }
        // A device: the one this job works, the one to go to, or one put
        // down now.
        let device = self
            .device
            .filter(|&d| stands(core, work, d))
            .or_else(|| go_to.filter(|&d| stands(core, work, d)));
        let Some(device) = device else {
            if self.session.is_some() {
                // It went out of the world under the session.
                return Tend::Fail(Why::NotFound);
            }
            return self.put_down(core, seed, haven, body, hands, row, item, avoid, tick);
        };
        self.device = Some(device);
        // Beside it, at arm's length.
        let (mx, mz) = cell_center(device.0, device.1);
        let d = (mx - x).hypot(mz - z);
        let session = self
            .session
            .get_or_insert_with(|| Session::new(ground_addr(device)));
        if !session.opened && (d - STAND_M).abs() > STAND_SLACK_M {
            self.held = None;
            let (ux, uz) = if d > f32::EPSILON {
                ((x - mx) / d, (z - mz) / d)
            } else {
                (1.0, 0.0)
            };
            let spot = [mx + ux * STAND_M, mz + uz * STAND_M];
            return match route.to(core, body, spot, 0.5, false, tick) {
                step @ Step::Walk { .. } => Tend::Go(step.walk().unwrap_or(Intent::IDLE)),
                Step::Blocked => Tend::Fail(Why::Stuck),
                Step::Arrived | Step::Wait => {
                    Tend::Go(Intent::walk(yaw_toward(spot[0] - x, spot[1] - z)))
                }
            };
        }
        session.step(core, seed, haven, body, hands, book, work, stats, tick)
    }

    /// Put the device down in front of the body: crafted first from the
    /// pack (a fire pit; a recycler is made at a bench, by the craft goal),
    /// onto the belt, held up while the eyes find the spot.
    #[allow(clippy::too_many_arguments)]
    fn put_down(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        row: u16,
        item: u16,
        avoid: impl Fn(u16, u16) -> bool,
        tick: u32,
    ) -> Tend {
        if carried(core, item) == 0 {
            let Some((recipe, ..)) = recipe_for(core, item, &Stations::NONE) else {
                return Tend::Fail(Why::NoRecipe);
            };
            self.making = Some((item, 0));
            return Tend::Craft { recipe };
        }
        let Some(slot) =
            (0..HOTBAR_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == item)
        else {
            return match belt_move(core, item) {
                Some((from, to, count)) => Tend::Belt { from, to, count },
                None => Tend::Fail(Why::MissingInputs),
            };
        };
        let refused = &self.refused[..self.spots.min(SPOT_TRIES)];
        let cell = self
            .placing
            .or_else(|| put_cell(core, seed, haven, body, refused, avoid));
        let Some((cx, cz)) = cell else {
            return Tend::Fail(Why::NoSpot);
        };
        let (mx, mz) = cell_center(cx, cz);
        let intent = Intent {
            look: Look::Point([mx, sim_core::terrain::ground(seed, haven, mx, mz), mz]),
            sel: Some(slot as u8),
            ..Intent::IDLE
        };
        let held = *self.held.get_or_insert(tick);
        if tick.wrapping_sub(held) >= HOLD_TICKS && hands.settled() {
            self.held = None;
            return Tend::Deploy {
                row,
                cx,
                cz,
                intent,
            };
        }
        Tend::Go(intent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::wiki::{Rules, SOURCES};

    /// The shipped rules and a client holding the shipped catalog.
    fn shipped() -> (Box<ClientCore>, Book, impl Fn(&str) -> u16) {
        let content = content::Content::from_sources(&SOURCES).unwrap();
        let baked = crate::net::bake_all(&content).unwrap();
        let rules = Rules::from_content(&content).unwrap();
        let mut book = Book::EMPTY;
        book.learn(&rules, &baked.catalog);
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        core.catalog = baked.catalog;
        core.research = baked.research;
        let ids: Vec<(String, u16)> = content
            .items
            .iter()
            .map(|i| (i.id.clone(), content.item_index(&i.id).unwrap()))
            .collect();
        let id = move |name: &str| ids.iter().find(|(n, _)| n == name).unwrap().1;
        (core, book, id)
    }

    fn stack(item: u16, count: u16) -> ItemStack {
        ItemStack {
            item,
            count,
            ..Default::default()
        }
    }

    /// Off the fire first whatever is done, before it burns; then wood when
    /// it runs low; then a piece of meat a slot; then nothing.
    #[test]
    fn a_cook_takes_off_the_done_then_fuels_then_lays_meat_a_slot_each() {
        let (mut core, book, id) = shipped();
        let (raw_meat, cooked, wood) =
            (id("item.raw_meat"), id("item.cooked_meat"), id("item.wood"));
        assert!(raw(&book, raw_meat) && !raw(&book, cooked) && !raw(&book, wood));
        core.inv[HOTBAR_SLOTS] = stack(raw_meat, 10);
        core.inv[HOTBAR_SLOTS + 1] = stack(wood, 100);
        let mut fire = [ItemStack::default(); BOX_SLOTS];
        fire[3] = stack(cooked, 1);
        let plan =
            |core: &ClientCore, fire: &[ItemStack]| plan(Work::Cook, core, &book, fire, true);
        // The cooked piece comes off, into the pack.
        let (into, from, _, count) = plan(&core, &fire).unwrap();
        assert_eq!((into, from, count), (false, 3, 1));
        fire[3] = ItemStack::default();
        // No wood on: a load of it.
        let (into, from, to, count) = plan(&core, &fire).unwrap();
        assert_eq!(
            (into, usize::from(from), count),
            (true, HOTBAR_SLOTS + 1, FUEL_LOAD)
        );
        fire[usize::from(to)] = stack(wood, count);
        // Then a piece of meat on each free slot, up to the cap.
        for laid in 0..COOK_SLOTS {
            let (into, from, to, count) = plan(&core, &fire).unwrap();
            assert_eq!((into, usize::from(from), count), (true, HOTBAR_SLOTS, 1));
            assert_eq!(fire[usize::from(to)].count, 0, "a free slot, piece {laid}");
            fire[usize::from(to)] = stack(raw_meat, 1);
        }
        assert_eq!(plan(&core, &fire), None, "the fire is full");
        assert!(should_run(Work::Cook, &core, &book, &fire));
    }

    /// Meat, wood and a fire to cook on: a cook can go; no wood, no meat,
    /// or no way to a fire, it cannot.
    #[test]
    fn a_cook_wants_meat_fuel_and_a_fire() {
        let (mut core, book, id) = shipped();
        let (raw_meat, wood, pit) = (id("item.raw_meat"), id("item.wood"), id("item.fire_pit"));
        let can = |core: &ClientCore| can_tend(core, &book, Work::Cook, false);
        assert!(!can(&core), "nothing to cook");
        core.inv[HOTBAR_SLOTS] = stack(raw_meat, 5);
        assert!(!can(&core), "no wood");
        core.inv[HOTBAR_SLOTS + 1] = stack(wood, FUEL_MIN as u16);
        // The recipe table has not arrived, so wood alone makes no pit.
        assert!(!can(&core), "no fire to cook on");
        assert!(can_tend(&core, &book, Work::Cook, true), "one near");
        core.inv[HOTBAR_SLOTS + 2] = stack(pit, 1);
        // Its deploy row has not arrived either: no fire it knows of.
        assert!(!can(&core));
        core.deploy_defs.defs[0].item = pit;
        core.deploy_defs.defs[0].hp = 50;
        core.deploy_defs.def_count = 1;
        core.deploy_defs_have = 1;
        assert!(can(&core));
    }

    /// A recycle takes off what came apart first, then puts in each stack
    /// that comes apart, whole; it runs while one is in.
    #[test]
    fn a_recycle_takes_off_the_scrap_then_feeds_whole_stacks() {
        let (mut core, book, id) = shipped();
        let (gears, junk, wood) = (id("item.gears"), id("item.junk"), id("item.wood"));
        assert!(book.page(gears).recycles && !book.page(wood).recycles);
        core.inv[HOTBAR_SLOTS] = stack(wood, 10);
        core.inv[HOTBAR_SLOTS + 1] = stack(gears, 3);
        let mut rec = [ItemStack::default(); BOX_SLOTS];
        rec[2] = stack(junk, 7);
        let (into, from, _, count) = plan(Work::Recycle, &core, &book, &rec, true).unwrap();
        assert_eq!((into, from, count), (false, 2, 7));
        rec[2] = ItemStack::default();
        assert!(!should_run(Work::Recycle, &core, &book, &rec));
        let (into, from, to, count) = plan(Work::Recycle, &core, &book, &rec, false).unwrap();
        assert_eq!(
            (into, usize::from(from), count),
            (true, HOTBAR_SLOTS + 1, 3)
        );
        rec[usize::from(to)] = stack(gears, 3);
        core.inv[HOTBAR_SLOTS + 1] = ItemStack::default();
        assert_eq!(plan(Work::Recycle, &core, &book, &rec, true), None);
        assert!(should_run(Work::Recycle, &core, &book, &rec));
        assert_eq!(held_for(Work::Recycle, &core, &book), 0);
    }

    /// A research puts one unit of the sample in the item slot and its
    /// price in the coin slot, runs, and the paper and the coin left over
    /// come out.
    #[test]
    fn a_research_loads_one_sample_and_its_price_then_takes_the_paper() {
        let (mut core, book, id) = shipped();
        let (revolver, junk) = (id("item.revolver"), id("item.junk"));
        let rc = core.research;
        let cost = rc.row_for(revolver).unwrap().cost;
        assert_eq!(rc.coin, junk);
        let work = Work::Research {
            sample: revolver,
            cost,
        };
        core.inv[HOTBAR_SLOTS] = stack(revolver, 1);
        core.inv[HOTBAR_SLOTS + 1] = stack(junk, 100);
        let mut table = [ItemStack::default(); BOX_SLOTS];
        let (into, from, to, count) = plan(work, &core, &book, &table, false).unwrap();
        assert_eq!(
            (into, usize::from(from), usize::from(to), count),
            (true, HOTBAR_SLOTS, TABLE_ITEM_SLOT, 1)
        );
        table[TABLE_ITEM_SLOT] = stack(revolver, 1);
        core.inv[HOTBAR_SLOTS] = ItemStack::default();
        let (into, from, to, count) = plan(work, &core, &book, &table, false).unwrap();
        assert_eq!(
            (into, usize::from(from), usize::from(to), count),
            (true, HOTBAR_SLOTS + 1, TABLE_COIN_SLOT, cost)
        );
        table[TABLE_COIN_SLOT] = stack(junk, cost);
        assert_eq!(plan(work, &core, &book, &table, false), None);
        assert!(should_run(work, &core, &book, &table));
        // Running: nothing moves. Landed: the paper comes out.
        assert_eq!(plan(work, &core, &book, &table, true), None);
        table[TABLE_ITEM_SLOT] = sim_core::research::blueprint_of(&rc, revolver);
        table[TABLE_COIN_SLOT] = ItemStack::default();
        let (into, from, ..) = plan(work, &core, &book, &table, false).unwrap();
        assert_eq!((into, usize::from(from)), (false, TABLE_ITEM_SLOT));
        assert!(!should_run(work, &core, &book, &table));
    }
}
