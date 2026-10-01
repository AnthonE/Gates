//! Cooking: raw meat at a fire pit, the way a player cooks it.
//!
//! Meat off a kill is not food until a fire has had it (`content/cooking.
//! toml`: raw meat cooks on a fire, and cooked meat left on burns). So a
//! player with meat in the pack walks to its fire, or puts a fire pit down
//! (crafting one from wood first), opens it, lays wood in and a piece of
//! meat on each free slot, lights it, and takes each piece off as soon as
//! it is done, before it burns; then puts the fire out and shuts it.
//! [`CookJob`] is that session. Everything it knows of the fire is what the
//! fire's panel shows and the lit set the server announces; what cooks to
//! what is the wiki's (`agent::wiki::Page::cooks`).

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
use sim_core::terrain::Haven;

/// The deployable a cook puts down, by catalog name.
pub const FIRE_ITEM: &str = "Fire Pit";
/// Pieces of meat on the fire at once, a slot each: they cook side by side.
pub const COOK_SLOTS: usize = 8;
/// Fuel laid in at a time, and the least it is topped up from: a unit burns
/// a few seconds, a piece of meat cooks in twenty.
pub const FUEL_LOAD: u16 = 20;
const FUEL_LOW: u32 = 4;
/// Fuel a cook wants in the pack before it is worth going.
pub const FUEL_MIN: u32 = 10;
/// Its own fire this near is walked to rather than another put down.
pub const FIRE_KEEP_M: f32 = 80.0;
/// It works the fire from this far off its middle.
pub const STAND_M: f32 = 1.8;
const STAND_SLACK_M: f32 = 0.6;
/// A fire pit goes down this far from the feet, in front.
const FIRE_NEAR_M: f32 = 1.5;
const FIRE_FAR_M: f32 = 3.5;
/// An answer to a deploy, an open, a move or a switch comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;
/// After a move is answered, the panel catches up this soon.
const SETTLE_TICKS: u32 = 3;
/// Moves one session makes at most.
pub const MAX_MOVES: u8 = 64;
/// Presses that went unanswered or that `E` would not take, before the
/// session gives up.
const MAX_TRIES: u8 = 3;
/// A cook that came to nothing is not offered again for this long.
pub const COOK_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// The drift left at a press, once the eyes have settled.
const PRESS_SLACK_M: f32 = 0.1;

/// What a cook wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cook {
    Go(Intent),
    /// Craft a fire pit (`encode_action_craft`).
    Craft {
        recipe: u16,
    },
    /// Move a stack to the belt (`encode_action_move`).
    Belt {
        from: u8,
        to: u8,
        count: u16,
    },
    /// Put the fire pit down, it in hand (`encode_action_deploy`).
    Deploy {
        row: u16,
        cx: u16,
        cz: u16,
        intent: Intent,
    },
    /// Open the fire (`encode_action_container`).
    Open {
        key: u32,
        intent: Intent,
    },
    /// Move a stack with the fire's panel open (`encode_action_move`).
    Move {
        key: u32,
        to_fire: bool,
        from: u8,
        to: u8,
        count: u16,
        intent: Intent,
    },
    /// Light the fire, or put it out (`encode_action_use`).
    Switch {
        at: OpAddr,
        intent: Intent,
    },
    /// Shut the panel: the session is over once this is sent.
    Close(Intent),
    Done,
    Fail(Why),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    Craft,
    Belt,
    Deploy,
    Open,
    Move,
    Switch,
    Close,
}

/// What cooking has come to.
#[derive(Clone, Copy, Debug, Default)]
pub struct CookStats {
    pub sessions: u64,
    pub fires: u64,
    pub laid: u64,
    pub cooked: u64,
    pub burnt: u64,
}

/// One cooking session at a fire.
#[derive(Clone, Copy, Debug, Default)]
pub struct CookJob {
    /// The cell of the fire this session works.
    fire: Option<(u16, u16)>,
    /// The cell a fire pit was asked to go down on.
    placing: Option<(u16, u16)>,
    placed: bool,
    refused: [(u16, u16); SPOT_TRIES],
    spots: usize,
    /// What the last step asked to send, until `sent` confirms it...
    want: Option<Wait>,
    /// ...and what went out, when.
    waiting: Option<(Wait, u32)>,
    /// The item crafted for, and how many were held before.
    making: Option<(u16, u32)>,
    held: Option<u32>,
    /// The move's answer, refused or not, and when.
    moved: Option<(bool, u32)>,
    deploy_refused: Option<u8>,
    opened: bool,
    fresh: bool,
    closed: bool,
    moves: u8,
    tries: u8,
}

/// The panel shows the fire at `key`.
fn fire_open(core: &ClientCore, key: u32) -> bool {
    core.cont_kind == CONT_BOX && core.cont_handle == key
}

fn addr((cx, cz): (u16, u16)) -> OpAddr {
    OpAddr {
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }
}

/// Units of an item in the pack and belt.
fn carried(core: &ClientCore, item: u16) -> u32 {
    core.inv[..INV_SLOTS]
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

/// Raw food the pack holds.
pub fn raw_held(core: &ClientCore, book: &Book) -> u32 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count > 0 && raw(book, s.item))
        .map(|s| u32::from(s.count))
        .sum()
}

/// The deployable row a fire pit is, and its item.
pub fn fire_row(core: &ClientCore) -> Option<(u16, u16)> {
    let item = item_named(core, FIRE_ITEM)?;
    let defs = &core.deploy_defs;
    (0..core.deploy_defs_have.min(defs.def_count))
        .find(|&row| {
            let def = defs.defs[usize::from(row)];
            def.hp > 0 && def.item == item
        })
        .map(|row| (row, item))
}

/// A fire stands at this cell, per the mirror.
pub fn fire_stands(core: &ClientCore, (cx, cz): (u16, u16)) -> bool {
    let Some((row, _)) = fire_row(core) else {
        return false;
    };
    core.deploys
        .entries()
        .iter()
        .any(|d| (d.cx, d.cz, d.level, d.loc) == (cx, cz, 0, LOC_PLANE) && u16::from(d.row) == row)
}

/// Where its own fire is worth walking to from (`x`, `z`): standing, and
/// near enough.
pub fn fire_near(
    core: &ClientCore,
    fire: Option<(u16, u16)>,
    x: f32,
    z: f32,
) -> Option<(u16, u16)> {
    fire.filter(|&f| {
        let (fx, fz) = cell_center(f.0, f.1);
        fire_stands(core, f) && (fx - x).hypot(fz - z) <= FIRE_KEEP_M
    })
}

/// Can a session go now: raw food and fuel in the pack, and a fire to cook
/// on (its own near here, a pit in the pack, or the wood to make one).
pub fn can_cook(core: &ClientCore, book: &Book, fire: Option<(u16, u16)>, x: f32, z: f32) -> bool {
    let fuel = book.fuel();
    if !book.ready() || fuel == NO_ITEM || raw_held(core, book) == 0 {
        return false;
    }
    let wood = carried(core, fuel);
    if wood < FUEL_MIN {
        return false;
    }
    if fire_near(core, fire, x, z).is_some() {
        return true;
    }
    let Some((_, item)) = fire_row(core) else {
        return false;
    };
    if carried(core, item) > 0 {
        return true;
    }
    // Made from the fuel itself: enough for the pit and a fire's worth.
    recipe_for(core, item, &Stations::NONE).is_some_and(|(r, ..)| {
        let def = core.recipes.recipes[usize::from(r)];
        def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())]
            .iter()
            .all(|&(input, need)| {
                let extra = if input == fuel { FUEL_MIN } else { 0 };
                carried(core, input) >= u32::from(need) + extra
            })
    })
}

/// A cell a fire pit goes down on, in front of the body: bare ground the
/// foundation's terrain rule takes, nothing built or put down on it, clear
/// of `avoid` (its own base), and not one refused already.
fn fire_cell(
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
            if !(FIRE_NEAR_M..=FIRE_FAR_M).contains(&d)
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

/// Where `count` of `stack` lands among `slots` (tried in `order`): onto a
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

/// The next move at an open fire, as `(to_fire, from, to, count)`: first
/// whatever is done off it (cooked food, burnt food, charcoal), before it
/// burns; then fuel when it runs low; then a piece of raw food onto each
/// free slot, up to [`COOK_SLOTS`].
pub fn plan(core: &ClientCore, book: &Book, fire: &[ItemStack]) -> Option<(bool, u8, u8, u16)> {
    let fuel = book.fuel();
    let cap = |item: u16| core.catalog.row(usize::from(item)).stack_max;
    let pack = &core.inv[..INV_SLOTS];
    let into_pack = (HOTBAR_SLOTS..INV_SLOTS).chain(0..HOTBAR_SLOTS);
    for (i, s) in fire.iter().enumerate() {
        if s.count == 0 || s.item == fuel || raw(book, s.item) {
            continue;
        }
        if let Some((to, count)) = land(pack, into_pack.clone(), s.item, s.count, cap(s.item)) {
            return Some((false, i as u8, to, count));
        }
    }
    let units = |item: u16| {
        fire.iter()
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum::<u32>()
    };
    if units(fuel) < FUEL_LOW {
        if let Some(from) = (0..INV_SLOTS).find(|&i| pack[i].count > 0 && pack[i].item == fuel) {
            let want = FUEL_LOAD
                .saturating_sub(units(fuel) as u16)
                .min(pack[from].count);
            if let Some((to, count)) = land(fire, 0..fire.len(), fuel, want, cap(fuel)) {
                return Some((true, from as u8, to, count));
            }
        }
    }
    let cooking = fire
        .iter()
        .filter(|s| s.count > 0 && raw(book, s.item))
        .count();
    if cooking < COOK_SLOTS {
        let from = (0..INV_SLOTS).find(|&i| pack[i].count > 0 && raw(book, pack[i].item))?;
        let to = fire.iter().position(|s| s.count == 0)?;
        return Some((true, from as u8, to as u8, 1));
    }
    None
}

impl CookJob {
    /// Nothing is in flight: switching now leaves at worst a panel open,
    /// which walking away shuts, and a fire burning, which burns out.
    pub fn at_checkpoint(&self) -> bool {
        self.waiting.is_none() && self.want.is_none()
    }

    /// The cell of the fire this session put down, once the server said so.
    pub fn placed(&self) -> Option<(u16, u16)> {
        self.placed.then_some(self.fire).flatten()
    }

    /// The last step's verb went out.
    pub fn sent(&mut self, tick: u32) {
        if let Some(w) = self.want.take() {
            self.waiting = Some((w, tick));
            match w {
                Wait::Move => {
                    self.moved = None;
                    self.moves = self.moves.saturating_add(1);
                }
                Wait::Open => self.fresh = false,
                Wait::Deploy => self.deploy_refused = None,
                _ => {}
            }
        }
    }

    /// A placement broadcast: the fire pit went down where it was asked.
    pub fn on_placed(&mut self, cx: u16, cz: u16, level: u8, loc: u8, deploy: bool) {
        if deploy && level == 0 && loc == LOC_PLANE && self.placing == Some((cx, cz)) {
            self.placing = None;
            self.placed = true;
            self.fire = Some((cx, cz));
        }
    }

    /// A deploy refusal, while a fire pit is in flight.
    pub fn on_refused(&mut self, reason: u8) {
        if matches!(self.waiting, Some((Wait::Deploy, _))) {
            self.deploy_refused = Some(reason);
        }
    }

    /// The fire's panel showed it.
    pub fn on_panel(&mut self, handle: u32) {
        if self
            .fire
            .is_some_and(|(cx, cz)| box_key(cx, cz, 0) == handle)
        {
            self.fresh = true;
        }
    }

    /// A move landed or was refused.
    pub fn on_moved(&mut self, refused: bool, tick: u32) {
        if matches!(self.waiting, Some((Wait::Move, _))) {
            self.moved = Some((refused, tick));
        }
    }

    /// The next step of the session. `own` is the fire it put down before,
    /// if it has one; `avoid` the cells a new one may not go on.
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
        own: Option<(u16, u16)>,
        avoid: impl Fn(u16, u16) -> bool,
        stats: &mut CookStats,
        tick: u32,
    ) -> Cook {
        let step = self.next(
            core, seed, haven, body, hands, route, book, own, avoid, stats, tick,
        );
        self.want = match step {
            Cook::Craft { .. } => Some(Wait::Craft),
            Cook::Belt { .. } => Some(Wait::Belt),
            Cook::Deploy { .. } => Some(Wait::Deploy),
            Cook::Open { .. } => Some(Wait::Open),
            Cook::Move { .. } => Some(Wait::Move),
            Cook::Switch { .. } => Some(Wait::Switch),
            Cook::Close(_) => Some(Wait::Close),
            _ => None,
        };
        if let Cook::Deploy { cx, cz, .. } = step {
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
        own: Option<(u16, u16)>,
        avoid: impl Fn(u16, u16) -> bool,
        stats: &mut CookStats,
        tick: u32,
    ) -> Cook {
        let fuel = book.fuel();
        if !book.ready() || fuel == NO_ITEM {
            return Cook::Fail(Why::NotFound);
        }
        let Some((row, pit)) = fire_row(core) else {
            return Cook::Fail(Why::NoRecipe);
        };
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        // What went out is waiting on its answer.
        if let Some((wait, since)) = self.waiting {
            let late = tick.wrapping_sub(since) >= VERDICT_TICKS;
            match wait {
                Wait::Craft => match self.making {
                    Some((item, before)) if carried(core, item) > before => {
                        self.waiting = None;
                        self.making = None;
                    }
                    // The pit takes its craft time: waited for, up to a point.
                    _ if tick.wrapping_sub(since) < VERDICT_TICKS + 10 * TICK_HZ => {
                        return Cook::Go(Intent::IDLE);
                    }
                    _ => return Cook::Fail(Why::NoAnswer),
                },
                Wait::Belt => {
                    if (0..HOTBAR_SLOTS).any(|i| core.inv[i].count > 0 && core.inv[i].item == pit) {
                        self.waiting = None;
                    } else if late {
                        return Cook::Fail(Why::NoAnswer);
                    } else {
                        return Cook::Go(Intent::IDLE);
                    }
                }
                Wait::Deploy => {
                    if self.placed {
                        self.waiting = None;
                        stats.fires += 1;
                    } else if let Some(reason) = self.deploy_refused.take() {
                        self.waiting = None;
                        if u32::from(reason) == REFUSE_D_COST {
                            return Cook::Fail(Why::MissingInputs);
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
                            return Cook::Fail(Why::NoSpot);
                        }
                    } else if late {
                        return Cook::Fail(Why::NoAnswer);
                    } else {
                        return Cook::Go(Intent::IDLE);
                    }
                }
                Wait::Open => {
                    let key = self.fire.map(|(cx, cz)| box_key(cx, cz, 0));
                    if self.fresh && key.is_some_and(|k| fire_open(core, k)) {
                        self.waiting = None;
                        self.opened = true;
                        self.tries = 0;
                        stats.sessions += 1;
                    } else if late {
                        self.waiting = None;
                        self.tries += 1;
                        if self.tries >= MAX_TRIES {
                            return Cook::Fail(Why::NoAnswer);
                        }
                    } else {
                        return Cook::Go(self.look(core, seed, haven, x, z));
                    }
                }
                Wait::Move => match self.moved {
                    Some((refused, at)) => {
                        if tick.wrapping_sub(at) < SETTLE_TICKS {
                            return Cook::Go(self.look(core, seed, haven, x, z));
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
                    None => return Cook::Go(self.look(core, seed, haven, x, z)),
                },
                Wait::Switch => {
                    // The lit set says when it caught (or went out).
                    let lit = self
                        .fire
                        .is_some_and(|(cx, cz)| core.ovens().is_lit(cx, cz, 0));
                    let want_lit = self.cooking(core, book);
                    if lit == want_lit || late {
                        self.waiting = None;
                        if lit != want_lit {
                            self.tries += 1;
                        }
                    } else {
                        return Cook::Go(self.look(core, seed, haven, x, z));
                    }
                }
                Wait::Close => {
                    self.waiting = None;
                    self.closed = true;
                    return Cook::Done;
                }
            }
        }
        if self.tries >= MAX_TRIES {
            return Cook::Fail(Why::Refused);
        }
        // A fire: the one this session works, its own near here, or one put
        // down now.
        let fire = self
            .fire
            .filter(|&f| fire_stands(core, f))
            .or_else(|| fire_near(core, own, x, z));
        let Some(fire) = fire else {
            if self.opened {
                // It went out of the world under the session.
                return Cook::Fail(Why::NotFound);
            }
            return self.put_down(core, seed, haven, body, hands, row, pit, avoid, tick);
        };
        self.fire = Some(fire);
        let key = box_key(fire.0, fire.1, 0);
        // Beside it, at arm's length.
        let (mx, mz) = cell_center(fire.0, fire.1);
        let d = (mx - x).hypot(mz - z);
        if (d - STAND_M).abs() > STAND_SLACK_M {
            self.held = None;
            let (ux, uz) = if d > f32::EPSILON {
                ((x - mx) / d, (z - mz) / d)
            } else {
                (1.0, 0.0)
            };
            let spot = [mx + ux * STAND_M, mz + uz * STAND_M];
            return match route.to(core, body, spot, 0.5, false, tick) {
                step @ Step::Walk { .. } => Cook::Go(step.walk().unwrap_or(Intent::IDLE)),
                Step::Blocked => Cook::Fail(Why::Stuck),
                Step::Arrived | Step::Wait => {
                    Cook::Go(Intent::walk(yaw_toward(spot[0] - x, spot[1] - z)))
                }
            };
        }
        if self.opened && !fire_open(core, key) {
            // It was open and the server shut it: out of reach, or gone.
            return Cook::Fail(Why::Stuck);
        }
        let look = self.look(core, seed, haven, x, z);
        if !self.opened {
            return if self.press(core, hands, fire, x, z, tick) {
                Cook::Open { key, intent: look }
            } else {
                Cook::Go(look)
            };
        }
        if self.moves >= MAX_MOVES {
            return Cook::Close(look);
        }
        let slots = &core.cont[..BOX_SLOTS];
        if let Some((to_fire, from, to, count)) = plan(core, book, slots) {
            if to_fire && raw(book, core.inv[usize::from(from)].item) {
                stats.laid += 1;
            }
            if !to_fire {
                // Taken off done, or past done: what burns no further is
                // burnt.
                let page = book.page(slots[usize::from(from)].item);
                if page.class == Class::Food && page.cooks == NO_ITEM {
                    stats.burnt += u64::from(count);
                } else if page.class == Class::Food {
                    stats.cooked += u64::from(count);
                }
            }
            return Cook::Move {
                key,
                to_fire,
                from,
                to,
                count,
                intent: look,
            };
        }
        let lit = core.ovens().is_lit(fire.0, fire.1, 0);
        let cooking = self.cooking(core, book);
        if cooking != lit {
            // Lit with meat on it; put out once the last is off.
            return if self.press(core, hands, fire, x, z, tick) {
                Cook::Switch {
                    at: addr(fire),
                    intent: look,
                }
            } else {
                Cook::Go(look)
            };
        }
        if cooking {
            // Cooking: watch it, and take each piece off when it is done.
            return Cook::Go(look);
        }
        Cook::Close(look)
    }

    /// Raw food is on the fire.
    fn cooking(&self, core: &ClientCore, book: &Book) -> bool {
        self.opened
            && core.cont[..BOX_SLOTS]
                .iter()
                .any(|s| s.count > 0 && raw(book, s.item))
    }

    /// Eyes on the fire.
    fn look(&self, core: &ClientCore, seed: u64, haven: &Haven, x: f32, z: f32) -> Intent {
        match self.fire {
            Some(f) => Intent {
                look: Look::Point(aim_point(core, seed, haven, addr(f), x, z)),
                ..Intent::IDLE
            },
            None => Intent::IDLE,
        }
    }

    /// The press at the fire, once the eyes have settled on it for a moment
    /// and it is what a player's `E` (or the switch's key) would take. A
    /// fire `E` will not take from here, settled on this long, counts a try.
    fn press(
        &mut self,
        core: &ClientCore,
        hands: &Hands,
        fire: (u16, u16),
        x: f32,
        z: f32,
        tick: u32,
    ) -> bool {
        let held = *self.held.get_or_insert(tick);
        let waited = tick.wrapping_sub(held);
        if waited >= HOLD_TICKS
            && hands.settled()
            && e_picks_by(core, x, z, hands.view().0, addr(fire), PRESS_SLACK_M)
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

    /// Put a fire pit down in front of the body: crafted first from the
    /// pack, onto the belt, held up while the eyes find the spot.
    #[allow(clippy::too_many_arguments)]
    fn put_down(
        &mut self,
        core: &ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        row: u16,
        pit: u16,
        avoid: impl Fn(u16, u16) -> bool,
        tick: u32,
    ) -> Cook {
        if carried(core, pit) == 0 {
            let Some((recipe, ..)) = recipe_for(core, pit, &Stations::NONE) else {
                return Cook::Fail(Why::NoRecipe);
            };
            self.making = Some((pit, 0));
            return Cook::Craft { recipe };
        }
        let Some(slot) =
            (0..HOTBAR_SLOTS).find(|&i| core.inv[i].count > 0 && core.inv[i].item == pit)
        else {
            return match belt_move(core, pit) {
                Some((from, to, count)) => Cook::Belt { from, to, count },
                None => Cook::Fail(Why::MissingInputs),
            };
        };
        let refused = &self.refused[..self.spots.min(SPOT_TRIES)];
        let cell = self
            .placing
            .or_else(|| fire_cell(core, seed, haven, body, refused, avoid));
        let Some((cx, cz)) = cell else {
            return Cook::Fail(Why::NoSpot);
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
            self.placed = false;
            return Cook::Deploy {
                row,
                cx,
                cz,
                intent,
            };
        }
        Cook::Go(intent)
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
        // The cooked piece comes off, into the pack.
        let (to_fire, from, _, count) = plan(&core, &book, &fire).unwrap();
        assert_eq!((to_fire, from, count), (false, 3, 1));
        fire[3] = ItemStack::default();
        // No wood on: a load of it.
        let (to_fire, from, to, count) = plan(&core, &book, &fire).unwrap();
        assert_eq!(
            (to_fire, usize::from(from), count),
            (true, HOTBAR_SLOTS + 1, FUEL_LOAD)
        );
        fire[usize::from(to)] = stack(wood, count);
        // Then a piece of meat on each free slot, up to the cap.
        for laid in 0..COOK_SLOTS {
            let (to_fire, from, to, count) = plan(&core, &book, &fire).unwrap();
            assert_eq!((to_fire, usize::from(from), count), (true, HOTBAR_SLOTS, 1));
            assert_eq!(fire[usize::from(to)].count, 0, "a free slot, piece {laid}");
            fire[usize::from(to)] = stack(raw_meat, 1);
        }
        assert_eq!(plan(&core, &book, &fire), None, "the fire is full");
    }

    /// Meat, wood and a fire to cook on: a cook can go; no wood, no meat,
    /// or no way to a fire, it cannot.
    #[test]
    fn a_cook_wants_meat_fuel_and_a_fire() {
        let (mut core, book, id) = shipped();
        let (raw_meat, wood, pit) = (id("item.raw_meat"), id("item.wood"), id("item.fire_pit"));
        assert!(!can_cook(&core, &book, None, 0.0, 0.0), "nothing to cook");
        core.inv[HOTBAR_SLOTS] = stack(raw_meat, 5);
        assert!(!can_cook(&core, &book, None, 0.0, 0.0), "no wood");
        core.inv[HOTBAR_SLOTS + 1] = stack(wood, FUEL_MIN as u16);
        // A pit in the pack (the recipe table has not arrived, so wood alone
        // makes none).
        assert!(
            !can_cook(&core, &book, None, 0.0, 0.0),
            "no fire to cook on"
        );
        core.inv[HOTBAR_SLOTS + 2] = stack(pit, 1);
        // Its deploy row has not arrived either: no fire it knows of.
        assert!(!can_cook(&core, &book, None, 0.0, 0.0));
        core.deploy_defs.defs[0].item = pit;
        core.deploy_defs.defs[0].hp = 50;
        core.deploy_defs.def_count = 1;
        core.deploy_defs_have = 1;
        assert!(can_cook(&core, &book, None, 0.0, 0.0));
    }
}
