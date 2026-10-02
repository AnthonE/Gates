//! Living in the base: the box, the belt and the hearth.
//!
//! A player comes home to put things away. It walks in through its own
//! doors (the builder's [`Passage`](crate::agent::build)), feeds the
//! cupboard when the last stock it read is running low, opens its box,
//! and moves stacks between the box and the pack under a belt loadout:
//! weapons, tools, armour, the building kit and a couple of stacks of food
//! and meds stay on the body; what the base still needs to be built stays
//! in the pack, and is
//! taken out of the box when the pack is short of it; the rest goes in the
//! box. Then it shuts the panel. [`StashJob`] is that visit.
//!
//! What it knows of the box is what the box's panel showed the last time
//! it was open ([`Ledger`]), the way a player remembers what is in their
//! box. What it knows of the upkeep is the reply to its last feed (the only
//! stock readout the game gives, kept in [`Home`]).

use crate::agent::build::{self, aim_point, Act, Builder, Way, PRESS_SLACK_M};
use crate::agent::hands::Hands;
use crate::agent::home::{Home, HOLD_TICKS};
use crate::agent::intent::{Intent, Look};
use crate::agent::oven;
use crate::agent::route::Route;
use crate::agent::wiki::{Book, Class};
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::deploy::box_key;
use sim_core::gather::ItemStack;
use sim_core::inventory::CONT_BOX;
use sim_core::limits::{BOX_SLOTS, HEARTH_STOCK_ROWS, HOTBAR_SLOTS, INV_SLOTS, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::Haven;

/// An answer to a feed, an open or a move comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;
/// After a move is answered, the box's panel catches up this soon: the
/// next move is planned from what it shows then.
const SETTLE_TICKS: u32 = 3;
/// Moves one visit makes at most: a box and a pack are 36 slots, and a
/// visit that has not finished by then is not converging.
pub const MAX_MOVES: u8 = 32;
/// Presses that went unanswered, or that `E` would not take, before the
/// visit gives up on that part.
const MAX_TRIES: u8 = 3;
/// A visit that failed is not offered again for this long.
pub const STASH_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// Stacks of each food and med kept on the body.
pub const PROVISION_STACKS: u32 = 2;
/// Distinct items the summary reports from the box.
pub const STORED_ROWS: usize = 6;

/// The box as its panel last showed it.
#[derive(Clone, Copy, Debug)]
pub struct Ledger {
    slots: [ItemStack; BOX_SLOTS],
    known: bool,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Ledger {
    pub const EMPTY: Self = Self {
        slots: [ItemStack {
            item: 0,
            count: 0,
            cond: 0,
            skin: 0,
        }; BOX_SLOTS],
        known: false,
    };

    /// The box is gone, or a new session: nothing is known of it.
    pub fn clear(&mut self) {
        *self = Self::EMPTY;
    }

    /// The panel on the box at `key` is open: what it shows is what the
    /// box holds. True when it was this box.
    pub fn on_cont(&mut self, core: &ClientCore, key: u32) -> bool {
        if !box_open(core, key) {
            return false;
        }
        self.slots.copy_from_slice(&core.cont[..BOX_SLOTS]);
        self.known = true;
        true
    }

    /// The panel has been open at least once.
    pub fn known(&self) -> bool {
        self.known
    }

    pub fn slots(&self) -> &[ItemStack] {
        &self.slots
    }

    /// Units of an item in the box, as last seen.
    pub fn units(&self, item: u16) -> u32 {
        self.slots
            .iter()
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum()
    }

    /// What the box holds, item by item, into `out`; the rows filled.
    pub fn totals(&self, out: &mut [(u16, u32)]) -> usize {
        let mut n = 0;
        for s in self.slots.iter().filter(|s| s.count > 0) {
            if let Some(row) = out[..n].iter_mut().find(|r| r.0 == s.item) {
                row.1 = row.1.saturating_add(u32::from(s.count));
            } else if n < out.len() {
                out[n] = (s.item, u32::from(s.count));
                n += 1;
            }
        }
        n
    }
}

/// Is the box at `key` the container this client has open?
pub fn box_open(core: &ClientCore, key: u32) -> bool {
    core.cont_kind == CONT_BOX && core.cont_handle == key
}

/// The belt loadout: how many of an item a player keeps on the body
/// whatever else it puts away. Weapons and tools by what the game's rules
/// say they are for, armour by the slot it is worn in, the plan and hammer
/// it builds with, and the rounds a weapon it carries fires: all of them.
/// Salvage and the recycler that takes it apart stay on the body too: the
/// recycle works from the pack, and nothing takes them out of the box for
/// it. Food and meds: [`PROVISION_STACKS`] stacks' worth, not a pack full
/// of mushrooms. Nothing else.
pub fn loadout(core: &ClientCore, book: &Book, item: u16) -> u32 {
    let row = core.catalog.row(usize::from(item));
    let page = book.page(item);
    match page.class {
        Class::Food | Class::Med => return PROVISION_STACKS * u32::from(row.stack_max.max(1)),
        Class::Other => {}
        _ => return u32::MAX,
    }
    let name = core.catalog.name(usize::from(item));
    let kit = name == build::PLAN_ITEM.as_bytes()
        || name == build::HAMMER_ITEM.as_bytes()
        || name == oven::RECYCLER_ITEM.as_bytes()
        || page.recycles;
    let round = core.inv[..INV_SLOTS]
        .iter()
        .any(|s| s.count > 0 && book.page(s.item).ranged.round == item);
    if kit || round || row.wear_slot != 0 {
        u32::MAX
    } else {
        0
    }
}

/// One stack's move, by slot: the only moves a visit makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    /// Box slot to pack slot.
    Take { from: u8, to: u8, count: u16 },
    /// Pack slot to box slot.
    Put { from: u8, to: u8, count: u16 },
    /// Pack slot to an empty belt slot.
    Belt { from: u8, to: u8, count: u16 },
}

/// Units of an item in the pack and belt.
fn carried(core: &ClientCore, item: u16) -> u32 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// Where `count` of `stack` lands among `slots` (tried in `order`): onto a
/// stack of the same item with room, else an empty slot. The count may
/// shrink to the room there; `plan_move` refuses a merge past the stack's
/// ceiling rather than clamping it. `None` when nothing takes any.
fn land(
    slots: &[ItemStack],
    order: impl Iterator<Item = usize> + Clone,
    stack: ItemStack,
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
        (d.count > 0 && d.item == stack.item && d.skin == stack.skin && room > 0)
            .then(|| (i as u8, count.min(room)))
    });
    merge.or_else(|| {
        order
            .clone()
            .find(|&i| slots[i].count == 0)
            .map(|i| (i as u8, count))
    })
}

/// The next move of a visit, or `None` when the box and the pack are where
/// they should be: first what the base's `bill` is short of in the pack,
/// out of the box; then what is carried past both the loadout (`keep`,
/// units of an item) and the bill, into the box; then the loadout from the
/// pack onto empty belt slots. Every move makes progress on one of the
/// three, and none undoes another's, so the visit ends.
pub fn plan(
    core: &ClientCore,
    keep: impl Fn(u16) -> u32,
    bill: &[(u16, u32)],
    boxed: &[ItemStack],
) -> Option<Transfer> {
    let cap = |item: u16| core.catalog.row(usize::from(item)).stack_max;
    let billed = |item: u16| {
        bill.iter()
            .filter(|r| r.0 == item)
            .map(|r| r.1)
            .sum::<u32>()
    };
    let pack = &core.inv[..INV_SLOTS];
    // The pack before the belt: the belt is for what is used.
    let into_pack = (HOTBAR_SLOTS..INV_SLOTS).chain(0..HOTBAR_SLOTS);
    for &(item, want) in bill {
        let short = want.saturating_sub(carried(core, item));
        if short == 0 {
            continue;
        }
        for (b, s) in boxed.iter().enumerate() {
            if s.count == 0 || s.item != item {
                continue;
            }
            let n = u32::from(s.count).min(short) as u16;
            if let Some((to, count)) = land(pack, into_pack.clone(), *s, n, cap(item)) {
                return Some(Transfer::Take {
                    from: b as u8,
                    to,
                    count,
                });
            }
        }
    }
    for (i, s) in pack.iter().enumerate().rev() {
        if s.count == 0 {
            continue;
        }
        let spare = carried(core, s.item).saturating_sub(billed(s.item).max(keep(s.item)));
        let n = u32::from(s.count).min(spare) as u16;
        if let Some((to, count)) = land(boxed, 0..boxed.len(), *s, n, cap(s.item)) {
            return Some(Transfer::Put {
                from: i as u8,
                to,
                count,
            });
        }
    }
    let to = (0..HOTBAR_SLOTS).find(|&i| pack[i].count == 0)?;
    (HOTBAR_SLOTS..INV_SLOTS)
        .find(|&i| pack[i].count > 0 && keep(pack[i].item) > 0)
        .map(|i| Transfer::Belt {
            from: i as u8,
            to: to as u8,
            count: pack[i].count,
        })
}

/// Is the cupboard worth a feed now: its stock is running low by the last
/// reading (or there is none yet for a base that pays upkeep), and the pack
/// or the box (as `stored` last showed it) holds something it is fed.
pub fn feed_due(core: &ClientCore, home: &Home, stored: &Ledger, grades: u32, tick: u32) -> bool {
    home.upkeep_due(tick, grades)
        && home.can_feed(|i| carried(core, i) > 0 || stored.units(i) > 0, tick)
}

/// Rows a visit's bill holds: the base's, and a feed's worth per upkeep
/// material.
pub const VISIT_ROWS: usize = build::BILL_ROWS + HEARTH_STOCK_ROWS;

/// What a visit keeps in the pack, into `out`: the base's `bill`, and a
/// feed's worth of each upkeep material running low while the cupboard
/// is due one, so the box gives up what the cupboard eats instead of
/// taking it in. The rows filled.
pub fn visit_bill(
    bill: &[(u16, u32)],
    home: &Home,
    grades: u32,
    tick: u32,
    out: &mut [(u16, u32); VISIT_ROWS],
) -> usize {
    let mut n = bill.len().min(VISIT_ROWS);
    out[..n].copy_from_slice(&bill[..n]);
    let mut feed = [(0u16, 0u32); HEARTH_STOCK_ROWS];
    let fed = home.feed_rows(tick, grades, &mut feed);
    for &(item, units) in &feed[..fed] {
        // One row per item: `plan` takes out what each row is short of.
        if let Some(row) = out[..n].iter_mut().find(|r| r.0 == item) {
            row.1 = row.1.saturating_add(units);
        } else if n < VISIT_ROWS {
            out[n] = (item, units);
            n += 1;
        }
    }
    n
}

/// What a visit wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chore {
    /// The walk in through the doors: the builder's airlock walk.
    Walk(Act),
    Go(Intent),
    /// Feed the cupboard at this address (`encode_action_feed`).
    Feed {
        at: OpAddr,
        intent: Intent,
    },
    /// Open the box (`encode_action_container`).
    Open {
        key: u32,
        intent: Intent,
    },
    /// Move a stack with the box's panel open (`encode_action_move`).
    Move {
        key: u32,
        transfer: Transfer,
        intent: Intent,
    },
    /// Shut the panel (`encode_action_container` with no container): the
    /// visit is over once this is sent.
    Close(Intent),
    Done,
    Fail(Why),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    Feed,
    Open,
    Move,
    Close,
}

/// One visit home: in through the doors, the feed, the box.
#[derive(Clone, Copy, Debug, Default)]
pub struct StashJob {
    inside: bool,
    /// The feed is over: answered, refused, skipped or given up.
    fed: bool,
    answered: bool,
    /// The move's answer, refused or not, and when.
    moved: Option<(bool, u32)>,
    /// What the last chore asked to send, until `sent` confirms it...
    want: Option<Wait>,
    /// ...and what went out, when.
    waiting: Option<(Wait, u32)>,
    /// Eyes held on this address since, at this point.
    held: Option<(OpAddr, u32, [f32; 3])>,
    /// Where the eyes were at the last press: they stay there until it
    /// is answered, since a press can wait a tick or two for its pace.
    pressed: Option<[f32; 3]>,
    moves: u8,
    tries: u8,
    /// Presses `E` would not take here, at the feed.
    feed_tries: u8,
    /// The feed was given up without a reply, or refused.
    feed_missed: bool,
    /// The server refused the feed in flight (reach, or no cupboard).
    refused: bool,
    /// The panel is shut: what is left is a feed of what the box gave up.
    closed: bool,
    /// This visit opened the box, and its panel has shown the box since
    /// the open went out. The client's panel is not cleared when the
    /// agent shuts it (the server sends nothing back), so what an earlier
    /// visit left there is never taken for the box as it is now.
    opened: bool,
    fresh: bool,
    /// The visit stopped short: moves refused, or too many of them.
    cut_short: bool,
}

impl StashJob {
    /// Nothing is in flight: no door walk, no press waiting on its answer.
    /// Switching now leaves the panel open at worst, which walking away
    /// shuts.
    pub fn at_checkpoint(&self) -> bool {
        self.waiting.is_none() && self.want.is_none()
    }

    /// Part of the visit came to nothing (the cupboard would not take a
    /// feed, or the box would not take the moves), or all of it did (no
    /// move and no feed: what called it home could not be done, a full
    /// pack and a full box say): not worth offering again straight away.
    pub fn gave_up(&self) -> bool {
        self.feed_missed || self.cut_short || (self.moves == 0 && !self.answered)
    }

    /// The box's panel showed the box (`APPLIED2_CONT` with it open).
    pub fn on_panel(&mut self) {
        self.fresh = true;
    }

    /// The last chore's verb went out.
    pub fn sent(&mut self, tick: u32) {
        if let Some(w) = self.want.take() {
            self.waiting = Some((w, tick));
            if w == Wait::Move {
                self.moved = None;
                self.moves = self.moves.saturating_add(1);
            }
            if w == Wait::Feed {
                self.answered = false;
                self.refused = false;
            }
            if w == Wait::Open {
                self.fresh = false;
            }
        }
    }

    /// The cupboard's stock came back: the feed is answered.
    pub fn on_stock(&mut self) {
        if matches!(self.waiting, Some((Wait::Feed, _))) {
            self.answered = true;
        }
    }

    /// A deploy refusal: a feed in flight was refused (reach, or no
    /// cupboard there). The reading is as it was, so the feed would be
    /// due again at once: the visit counts it missed, and waits.
    pub fn on_refused(&mut self) {
        if matches!(self.waiting, Some((Wait::Feed, _))) {
            self.refused = true;
        }
    }

    /// A move landed or was refused.
    pub fn on_moved(&mut self, refused: bool, tick: u32) {
        if matches!(self.waiting, Some((Wait::Move, _))) {
            self.moved = Some((refused, tick));
        }
    }

    /// The next move of the visit.
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
        book: &Book,
        tick: u32,
    ) -> Chore {
        let chore = self.next(
            core, seed, haven, body, hands, route, builder, home, book, tick,
        );
        self.want = match chore {
            Chore::Feed { .. } => Some(Wait::Feed),
            Chore::Open { .. } => Some(Wait::Open),
            Chore::Move { .. } => Some(Wait::Move),
            Chore::Close(_) => Some(Wait::Close),
            _ => None,
        };
        chore
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
        builder: &mut Builder,
        home: &Home,
        book: &Book,
        tick: u32,
    ) -> Chore {
        if builder.plan().is_none() || !book.ready() {
            return Chore::Fail(Why::NotFound);
        }
        let hearth = builder.hearth_addr(core);
        let chest = builder.box_addr(core);
        let key = chest.map(|b| box_key(b.cx, b.cz, b.level));
        // What went out is waiting on its answer.
        if let Some((wait, since)) = self.waiting {
            let late = tick.wrapping_sub(since) >= VERDICT_TICKS;
            match wait {
                Wait::Feed => {
                    if self.answered || self.refused || late {
                        self.waiting = None;
                        self.fed = true;
                        self.feed_missed |= !self.answered;
                    } else if let Some(h) = hearth {
                        return Chore::Go(self.stay(seed, haven, core, h));
                    }
                }
                Wait::Open => {
                    if self.fresh && key.is_some_and(|k| box_open(core, k)) {
                        self.waiting = None;
                        self.opened = true;
                        self.tries = 0;
                    } else if late {
                        self.waiting = None;
                        self.tries += 1;
                        if self.tries >= MAX_TRIES {
                            return Chore::Fail(Why::NoAnswer);
                        }
                    } else if let Some(b) = chest {
                        return Chore::Go(self.stay(seed, haven, core, b));
                    }
                }
                Wait::Move => match self.moved {
                    Some((refused, at)) => {
                        if tick.wrapping_sub(at) < SETTLE_TICKS {
                            if let Some(b) = chest {
                                return Chore::Go(build::look_at(seed, haven, core, b));
                            }
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
                    None => {
                        if let Some(b) = chest {
                            return Chore::Go(build::look_at(seed, haven, core, b));
                        }
                        self.waiting = None;
                    }
                },
                // The panel is shut (or the close waits on its pace, which
                // `explorer.rs` sees to before ending the visit). A feed
                // put off for what the box held comes now.
                Wait::Close => {
                    self.waiting = None;
                    self.closed = true;
                    if self.fed {
                        return Chore::Done;
                    }
                }
            }
        }
        // In through the doors, onto the spot in the core that both the
        // cupboard and the box are in reach of.
        if !self.inside {
            match builder.pass(core, seed, haven, body, hands, route, Way::In, tick) {
                Act::Done => self.inside = true,
                Act::Fail(why) => return Chore::Fail(why),
                act => return Chore::Walk(act),
            }
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let yaw = hands.view().0;
        // The cupboard first, while the pack still carries what it eats;
        // short of that, once the box has given it up and the panel is
        // shut.
        let grades = builder.charged();
        if !self.fed {
            let due = hearth.filter(|_| home.upkeep_due(tick, grades));
            let carries = home.can_feed(|i| carried(core, i) > 0, tick);
            match due {
                None => self.fed = true,
                Some(_) if !carries && chest.is_some() && !self.closed => {}
                Some(_) if !carries => self.fed = true,
                Some(h) => match self.aim(core, seed, haven, hands, h, x, z, yaw, tick) {
                    Aim::Press(intent) => return Chore::Feed { at: h, intent },
                    Aim::Wait(intent) => return Chore::Go(intent),
                    Aim::Missed => {
                        self.feed_tries += 1;
                        if self.feed_tries >= MAX_TRIES {
                            self.fed = true;
                            self.feed_missed = true;
                        }
                    }
                },
            }
            if !self.fed && (carries || self.closed) {
                return Chore::Go(Intent::IDLE);
            }
        }
        let (Some(b), Some(key), false) = (chest, key, self.closed) else {
            // No box to visit (a feed was all there was), or the visit to
            // it is over.
            return Chore::Done;
        };
        let look = build::look_at(seed, haven, core, b);
        if self.opened && !box_open(core, key) {
            // It was open and the server shut it: out of reach, or gone.
            // A feed put off for the box is tried with what the pack has.
            self.closed = true;
            return if self.fed {
                Chore::Done
            } else {
                Chore::Go(look)
            };
        }
        if !self.opened {
            return match self.aim(core, seed, haven, hands, b, x, z, yaw, tick) {
                Aim::Press(intent) => Chore::Open { key, intent },
                Aim::Wait(intent) => Chore::Go(intent),
                Aim::Missed => {
                    self.tries += 1;
                    if self.tries >= MAX_TRIES {
                        Chore::Fail(Why::Refused)
                    } else {
                        Chore::Go(look)
                    }
                }
            };
        }
        let next = if self.moves >= MAX_MOVES || self.tries >= MAX_TRIES {
            self.cut_short = true;
            None
        } else {
            let mut bill = [(0, 0); VISIT_ROWS];
            let n = visit_bill(builder.survey().bill(), home, grades, tick, &mut bill);
            plan(
                core,
                |item| loadout(core, book, item),
                &bill[..n],
                &core.cont[..BOX_SLOTS],
            )
        };
        match next {
            Some(transfer) => Chore::Move {
                key,
                transfer,
                intent: look,
            },
            None => Chore::Close(look),
        }
    }

    /// Eyes on the address, then the press once they have settled there
    /// for a moment and the thing is what a player's `E` would take, with
    /// room to spare. The eyes go where it is taken: its middle, or a
    /// little to one side of it when something nearer sits on the edge of
    /// the view there (the cupboard, beside the spot the box is opened
    /// from).
    #[allow(clippy::too_many_arguments)]
    fn aim(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        hands: &Hands,
        at: OpAddr,
        x: f32,
        z: f32,
        yaw: u16,
        tick: u32,
    ) -> Aim {
        let (since, point) = match self.held {
            Some((a, since, point)) if a == at => (since, point),
            _ => {
                let point = aim_point(core, seed, haven, at, x, z);
                self.held = Some((at, tick, point));
                (tick, point)
            }
        };
        let intent = Intent {
            look: Look::Point(point),
            ..Intent::IDLE
        };
        let waited = tick.wrapping_sub(since);
        if waited >= HOLD_TICKS
            && hands.settled()
            && build::e_picks_by(core, x, z, yaw, at, PRESS_SLACK_M)
        {
            self.held = None;
            self.pressed = Some(point);
            return Aim::Press(intent);
        }
        if waited >= VERDICT_TICKS {
            // Settled on it this long and `E` still takes something else.
            self.held = None;
            return Aim::Missed;
        }
        Aim::Wait(intent)
    }
}

impl StashJob {
    /// The look while a press waits on its answer: where it was pressed.
    fn stay(&self, seed: u64, haven: &Haven, core: &ClientCore, at: OpAddr) -> Intent {
        match self.pressed {
            Some(point) => Intent {
                look: Look::Point(point),
                ..Intent::IDLE
            },
            None => build::look_at(seed, haven, core, at),
        }
    }
}

enum Aim {
    Wait(Intent),
    Press(Intent),
    Missed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::event::ItemRow;

    const WOOD: u16 = 1;
    const STONE: u16 = 2;
    const ORE: u16 = 3;
    const SPEAR: u16 = 4;

    fn core() -> Box<ClientCore> {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        for (i, name) in [
            (WOOD, "Wood"),
            (STONE, "Stone"),
            (ORE, "Ore"),
            (SPEAR, "Spear"),
        ] {
            let row = ItemRow {
                stack_max: if i == SPEAR { 1 } else { 1000 },
                ..ItemRow::EMPTY
            };
            core.catalog
                .set(usize::from(i), name.as_bytes(), row)
                .unwrap();
        }
        core.catalog.count = 5;
        core
    }

    fn stack(item: u16, count: u16) -> ItemStack {
        ItemStack {
            item,
            count,
            ..Default::default()
        }
    }

    /// Runs `plan` to the end, applying each move the way the server
    /// would, and returns how many moves it took.
    fn visit(core: &mut ClientCore, bill: &[(u16, u32)], boxed: &mut [ItemStack]) -> usize {
        let keep = |item: u16| if item == SPEAR { u32::MAX } else { 0 };
        for n in 0..64 {
            let Some(t) = plan(core, keep, bill, boxed) else {
                return n;
            };
            let (src, dst): (&mut ItemStack, &mut ItemStack) = match t {
                Transfer::Take { from, to, .. } => (
                    &mut boxed[usize::from(from)],
                    &mut core.inv[usize::from(to)],
                ),
                Transfer::Put { from, to, .. } => (
                    &mut core.inv[usize::from(from)],
                    &mut boxed[usize::from(to)],
                ),
                Transfer::Belt { from, to, .. } => {
                    let (lo, hi) = core.inv.split_at_mut(usize::from(from));
                    (&mut hi[0], &mut lo[usize::from(to)])
                }
            };
            let count = match t {
                Transfer::Take { count, .. }
                | Transfer::Put { count, .. }
                | Transfer::Belt { count, .. } => count,
            };
            assert!(count > 0 && count <= src.count, "{t:?}");
            assert!(dst.count == 0 || (dst.item == src.item && dst.count + count <= 1000));
            dst.item = src.item;
            dst.count += count;
            src.count -= count;
        }
        panic!("the visit never ended");
    }

    #[test]
    fn a_visit_keeps_the_loadout_and_what_the_base_needs_and_boxes_the_rest() {
        let mut core = core();
        let mut boxed = [ItemStack::default(); BOX_SLOTS];
        core.inv[HOTBAR_SLOTS] = stack(SPEAR, 1);
        core.inv[HOTBAR_SLOTS + 1] = stack(WOOD, 900);
        core.inv[HOTBAR_SLOTS + 2] = stack(ORE, 40);
        core.inv[HOTBAR_SLOTS + 3] = stack(STONE, 200);
        boxed[0] = stack(STONE, 700);
        boxed[1] = stack(ORE, 990);
        // The base wants 300 wood and 600 stone.
        let bill = [(WOOD, 300), (STONE, 600)];
        let moves = visit(&mut core, &bill, &mut boxed);
        assert!(moves > 0);
        let carried = |item| carried(&core, item);
        assert_eq!(carried(WOOD), 300, "the spare wood went in the box");
        assert_eq!(carried(STONE), 600, "the short stone came out");
        assert_eq!(carried(ORE), 0, "the loot went in the box");
        let boxed_units = |item| {
            boxed
                .iter()
                .filter(|s| s.item == item)
                .map(|s| u32::from(s.count))
                .sum::<u32>()
        };
        assert_eq!(
            boxed_units(ORE),
            1030,
            "onto the ore there, then a new slot"
        );
        assert_eq!(boxed_units(WOOD), 600);
        assert_eq!(boxed_units(STONE), 300);
        // The spear came out of the pack onto the belt.
        assert!(core.inv[..HOTBAR_SLOTS]
            .iter()
            .any(|s| s.item == SPEAR && s.count == 1));
        // Nothing left to do: a second visit makes no move.
        assert_eq!(visit(&mut core, &bill, &mut boxed), 0);
    }

    #[test]
    fn the_box_gives_up_what_the_cupboard_eats_and_a_visit_that_did_nothing_waits() {
        use sim_core::deploy::FEED_CHUNK;
        let mut core = core();
        // The last feed read 5 periods of stone (50 against 240 a day):
        // running low.
        core.stock[0] = (STONE, 50, 240);
        core.stock_count = 1;
        let mut home = Home::new();
        home.on_stock(&core, 0, 3);
        let mut bill = [(0, 0); VISIT_ROWS];
        let n = visit_bill(&[(WOOD, 300), (STONE, 100)], &home, 3, 0, &mut bill);
        assert_eq!(&bill[..n], &[(WOOD, 300), (STONE, 100 + FEED_CHUNK)]);
        // None in the pack, some in the box: a feed is due, from the box.
        let mut ledger = Ledger::EMPTY;
        assert!(!feed_due(&core, &home, &ledger, 3, 0));
        ledger.slots[0] = stack(STONE, 2000);
        assert!(feed_due(&core, &home, &ledger, 3, 0));
        let mut boxed = ledger.slots;
        visit(&mut core, &bill[..n], &mut boxed);
        assert_eq!(carried(&core, STONE), 100 + FEED_CHUNK, "out for the feed");
        // A full pack and a full box: nothing moves, and a visit that made
        // no move and fed nothing is held off.
        let mut boxed = [stack(ORE, 1000); BOX_SLOTS];
        for s in core.inv[..INV_SLOTS].iter_mut() {
            *s = stack(WOOD, 1000);
        }
        let keep = |_| 0;
        assert_eq!(plan(&core, keep, &bill[..n], &boxed), None);
        boxed[0] = stack(STONE, 1000);
        assert_eq!(
            plan(&core, keep, &bill[..n], &boxed),
            None,
            "no room for it"
        );
        assert!(StashJob::default().gave_up());
    }

    #[test]
    fn a_full_box_takes_nothing_and_the_visit_still_ends() {
        let mut core = core();
        let mut boxed = [stack(STONE, 1000); BOX_SLOTS];
        core.inv[HOTBAR_SLOTS] = stack(ORE, 5);
        for s in core.inv[..HOTBAR_SLOTS].iter_mut() {
            *s = stack(WOOD, 1);
        }
        assert_eq!(visit(&mut core, &[], &mut boxed), 0);
        let mut ledger = Ledger::EMPTY;
        ledger.slots = boxed;
        assert_eq!(ledger.units(STONE), 12_000);
        let mut out = [(0, 0); STORED_ROWS];
        assert_eq!(ledger.totals(&mut out), 1);
        assert_eq!(out[0], (STONE, 12_000));
    }
}
