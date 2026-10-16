//! The compound shop: a player's vending machine (`deploy::ARCH_VENDING`),
//! Rust's vending machine priced in the island's junk.
//!
//! The machine is a 12-slot container like a box (`deploy::holds_items`):
//! its slots hold the stock and the junk it takes. Only its owner and the
//! crew of the claim it stands in move items in or out, or set its offers
//! (`turret::spares`, the turret's rule); anyone in reach may open it to look
//! and may buy.
//!
//! An offer is "`lot` of `item` for `price` junk". The offers live here,
//! keyed by the machine's box address the way a lock is keyed by its door's,
//! and go with the machine (`Deploys::remove_at`). A machine with no offers
//! holds no record.
//!
//! A purchase is all or nothing on scratch copies of the machine and the
//! pack (`vend.rs`'s shape): the goods and the junk move together, or
//! nothing moves. A stack keeps its condition and skin on the way through,
//! so a shop never mends a tool.

use crate::build::Pieces;
use crate::craft::{inv_count, inv_take};
use crate::deploy::{DeployContent, Deploys, NO_SHOP_COIN};
use crate::gather::{GatherContent, ItemStack};
use crate::limits::{BOX_SLOTS, MAX_SHOPS, SHOP_ROWS};
use crate::world::{EventQueue, Player, EV_SHOP, EV_SHOP_REFUSED};

/// The most lots one purchase may ask for.
pub const SHOP_TIMES_MAX: u8 = 20;

/// The shop verb's ops (`ShopAct::op`).
pub const SHOP_OP_SET: u8 = 0;
pub const SHOP_OP_CLEAR: u8 = 1;
pub const SHOP_OP_BUY: u8 = 2;
pub const SHOP_OP_MAX: u8 = SHOP_OP_BUY;

/// Refusals, `EV_SHOP_REFUSED.c >> 24`.
/// No vending machine at the address, or a malformed ask.
pub const REFUSE_SH_KIND: u32 = 1;
/// The machine is out of reach.
pub const REFUSE_SH_REACH: u32 = 2;
/// No such offer: the row is past the table or empty.
pub const REFUSE_SH_OFFER: u32 = 3;
/// The machine holds less than the lots asked for.
pub const REFUSE_SH_STOCK: u32 = 4;
/// The buyer carries less junk than the bill.
pub const REFUSE_SH_FUNDS: u32 = 5;
/// The goods do not fit the buyer's pack.
pub const REFUSE_SH_FULL: u32 = 6;
/// The takings do not fit the machine.
pub const REFUSE_SH_TAKINGS: u32 = 7;
/// Only the owner and the crew of its claim set a machine's offers.
pub const REFUSE_SH_ALLOWED: u32 = 8;
/// Every shop record on the island is spoken for (`MAX_SHOPS`).
pub const REFUSE_SH_CAP: u32 = 9;
pub const REFUSE_SH_MAX: u32 = REFUSE_SH_CAP;

/// One offer: `lot` of `item` for `price` of the coin. `lot == 0` is an
/// empty row, and an empty row is all zeros.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShopOffer {
    pub item: u16,
    pub lot: u16,
    pub price: u16,
}

impl ShopOffer {
    pub fn live(&self) -> bool {
        self.lot > 0
    }
}

/// One machine's offers, keyed by its packed box address (`deploy::box_key`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShopRec {
    pub key: u32,
    pub rows: [ShopOffer; SHOP_ROWS],
}

impl ShopRec {
    /// Every field, little-endian: the key, then each row's item, lot, price.
    pub fn hash_bytes(&self) -> [u8; 4 + SHOP_ROWS * 6] {
        let mut b = [0u8; 4 + SHOP_ROWS * 6];
        b[0..4].copy_from_slice(&self.key.to_le_bytes());
        for (r, o) in self.rows.iter().enumerate() {
            let at = 4 + r * 6;
            b[at..at + 2].copy_from_slice(&o.item.to_le_bytes());
            b[at + 2..at + 4].copy_from_slice(&o.lot.to_le_bytes());
            b[at + 4..at + 6].copy_from_slice(&o.price.to_le_bytes());
        }
        b
    }
}

/// What a shop press asks: set a row, clear it, or buy `times` lots of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopAct {
    Set {
        row: u8,
        item: u16,
        lot: u16,
        price: u16,
    },
    Clear {
        row: u8,
    },
    Buy {
        row: u8,
        times: u8,
    },
}

impl ShopAct {
    pub fn op(&self) -> u8 {
        match self {
            ShopAct::Set { .. } => SHOP_OP_SET,
            ShopAct::Clear { .. } => SHOP_OP_CLEAR,
            ShopAct::Buy { .. } => SHOP_OP_BUY,
        }
    }

    pub fn row(&self) -> u8 {
        match *self {
            ShopAct::Set { row, .. } | ShopAct::Clear { row } | ShopAct::Buy { row, .. } => row,
        }
    }

    /// The lots a buy asks for, 0 for the other ops.
    pub fn times(&self) -> u8 {
        match *self {
            ShopAct::Buy { times, .. } => times,
            _ => 0,
        }
    }

    /// `EV_SHOP.c`'s low 24 bits: op << 16 | row << 8 | times.
    pub fn packed(&self) -> u32 {
        ((self.op() as u32) << 16) | ((self.row() as u32) << 8) | self.times() as u32
    }
}

/// Every machine's offers: a dense list, swap-removed, boxed like the box
/// store.
pub struct Shops {
    entries: Box<[ShopRec; MAX_SHOPS]>,
    len: usize,
}

impl Default for Shops {
    fn default() -> Self {
        Self::new()
    }
}

impl Shops {
    pub fn new() -> Self {
        Self {
            entries: crate::boxed_array(ShopRec::default()),
            len: 0,
        }
    }

    pub fn entries(&self) -> &[ShopRec] {
        &self.entries[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn index_of(&self, key: u32) -> Option<usize> {
        if key == 0 {
            return None;
        }
        self.entries().iter().position(|r| r.key == key)
    }

    /// The machine's offers, all empty when it has none.
    pub fn offers(&self, key: u32) -> [ShopOffer; SHOP_ROWS] {
        self.index_of(key)
            .map_or([ShopOffer::default(); SHOP_ROWS], |i| self.entries[i].rows)
    }

    /// Forget a machine's offers (it is gone). A key with none is a no-op.
    pub(crate) fn drop_key(&mut self, key: u32) {
        if let Some(i) = self.index_of(key) {
            self.len -= 1;
            self.entries[i] = self.entries[self.len];
            self.entries[self.len] = ShopRec::default();
        }
    }

    /// Write one row; false when the machine has no record and the store is
    /// full.
    fn set(&mut self, key: u32, row: usize, offer: ShopOffer) -> bool {
        let i = match self.index_of(key) {
            Some(i) => i,
            None => {
                if self.len == MAX_SHOPS {
                    return false;
                }
                self.entries[self.len] = ShopRec {
                    key,
                    rows: [ShopOffer::default(); SHOP_ROWS],
                };
                self.len += 1;
                self.len - 1
            }
        };
        self.entries[i].rows[row] = offer;
        true
    }

    /// Empty one row, and drop the record when no row is left.
    fn clear(&mut self, key: u32, row: usize) {
        let Some(i) = self.index_of(key) else {
            return;
        };
        self.entries[i].rows[row] = ShopOffer::default();
        if !self.entries[i].rows.iter().any(ShopOffer::live) {
            self.drop_key(key);
        }
    }

    /// Replace every record from a decoded world save. Boot-only.
    pub(crate) fn restore(&mut self, recs: &[ShopRec]) {
        self.len = recs.len().min(MAX_SHOPS);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
        for r in self.entries[self.len..].iter_mut() {
            *r = ShopRec::default();
        }
    }
}

/// How many of `item` the slots hold.
pub fn count_in(slots: &[ItemStack], item: u16) -> u32 {
    slots
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| s.count as u32)
        .sum()
}

/// Add `amount` of `item` (with `cond` and `skin` on fresh stacks) to the
/// slots: top-ups first, then empty slots. Returns what went in.
fn add_to(
    slots: &mut [ItemStack],
    item: u16,
    amount: u32,
    stack_max: u16,
    cond: u16,
    skin: u16,
) -> u32 {
    if stack_max == 0 {
        return 0;
    }
    let mut left = amount;
    for s in slots.iter_mut() {
        if left == 0 {
            return amount;
        }
        if s.count > 0 && s.item == item && s.skin == skin && s.count < stack_max {
            let take = ((stack_max - s.count) as u32).min(left);
            s.count += take as u16;
            left -= take;
        }
    }
    for s in slots.iter_mut() {
        if left == 0 {
            return amount;
        }
        if s.count == 0 {
            let take = (stack_max as u32).min(left);
            *s = ItemStack {
                item,
                count: take as u16,
                cond,
                skin,
            };
            left -= take;
        }
    }
    amount - left
}

/// Move `n` of `item` from `src` to `dst` stack by stack, each keeping its
/// condition and skin. False when `dst` ran out of room (the scratch copies
/// are then thrown away).
fn move_units(
    src: &mut [ItemStack],
    dst: &mut [ItemStack],
    item: u16,
    n: u32,
    stack_max: u16,
) -> bool {
    let mut left = n;
    for s in src.iter_mut() {
        if left == 0 {
            break;
        }
        if s.count == 0 || s.item != item {
            continue;
        }
        let k = (s.count as u32).min(left);
        if add_to(dst, item, k, stack_max, s.cond, s.skin) < k {
            return false;
        }
        s.count -= k as u16;
        if s.count == 0 {
            *s = ItemStack::default();
        }
        left -= k;
    }
    left == 0
}

/// Whether `id` may stock machine `i` and set its offers.
pub fn may_edit(pieces: &Pieces, deploys: &Deploys, i: usize, id: u32) -> bool {
    let Some(b) = deploys.boxes().get(i) else {
        return false;
    };
    let (x, z) = b.xz();
    crate::turret::spares(pieces, deploys, b.owner, x, z, id)
}

/// Apply one shop press at the machine with box address `key`. `reach` is
/// the world's verdict: `None` when no vending machine stands there, else
/// whether the presser reaches it. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    dc: &DeployContent,
    gc: &GatherContent,
    pieces: &Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    key: u32,
    act: ShopAct,
    reach: Option<bool>,
    events: &mut EventQueue,
) {
    let pid = p.id;
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(EV_SHOP_REFUSED, pid, key, (code << 24) | act.packed());
    };
    let Some(i) = deploys.vending_index(key) else {
        refuse(events, REFUSE_SH_KIND);
        return;
    };
    if p.dead || p.sleeping || p.wounded {
        refuse(events, REFUSE_SH_KIND);
        return;
    }
    if reach != Some(true) {
        refuse(events, REFUSE_SH_REACH);
        return;
    }
    let row = act.row() as usize;
    if row >= SHOP_ROWS {
        refuse(events, REFUSE_SH_OFFER);
        return;
    }
    match act {
        ShopAct::Set {
            item, lot, price, ..
        } => {
            if lot == 0 || item >= gc.item_count {
                refuse(events, REFUSE_SH_KIND);
                return;
            }
            if !may_edit(pieces, deploys, i, pid) {
                refuse(events, REFUSE_SH_ALLOWED);
                return;
            }
            let offer = ShopOffer { item, lot, price };
            if !deploys.shops_mut().set(key, row, offer) {
                refuse(events, REFUSE_SH_CAP);
                return;
            }
        }
        ShopAct::Clear { .. } => {
            if !may_edit(pieces, deploys, i, pid) {
                refuse(events, REFUSE_SH_ALLOWED);
                return;
            }
            deploys.shops_mut().clear(key, row);
        }
        ShopAct::Buy { times, .. } => {
            let coin = dc.shop_coin;
            if times == 0 || times > SHOP_TIMES_MAX || coin == NO_SHOP_COIN {
                refuse(events, REFUSE_SH_KIND);
                return;
            }
            let o = deploys.shops().offers(key)[row];
            if !o.live() {
                refuse(events, REFUSE_SH_OFFER);
                return;
            }
            let goods = o.lot as u32 * times as u32;
            let bill = o.price as u32 * times as u32;
            let mut mach = deploys.boxes()[i].items;
            if count_in(&mach, o.item) < goods {
                refuse(events, REFUSE_SH_STOCK);
                return;
            }
            if inv_count(&p.inv, coin) < bill {
                refuse(events, REFUSE_SH_FUNDS);
                return;
            }
            let mut inv = p.inv;
            inv_take(&mut inv, coin, bill);
            if !move_units(&mut mach, &mut inv, o.item, goods, gc.stack_max_of(o.item)) {
                refuse(events, REFUSE_SH_FULL);
                return;
            }
            let into = add_to(
                &mut mach,
                coin,
                bill,
                gc.stack_max_of(coin),
                gc.cond_max_of(coin),
                crate::skin::NO_SKIN,
            );
            if into < bill {
                refuse(events, REFUSE_SH_TAKINGS);
                return;
            }
            p.inv = inv;
            for (s, st) in mach.iter().enumerate().take(BOX_SLOTS) {
                deploys.set_box_slot(i, s, *st);
            }
        }
    }
    events.push(EV_SHOP, pid, key, act.packed());
}
