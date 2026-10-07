//! The town's vendors (`content/sites.toml`): fixed offers at the kiosks of
//! THE GATE (`town::KIOSKS`), traded for junk. Rust's Outpost vending
//! machines are the reference: category stalls (food, building, tools,
//! components, a resource exchange), unlimited stock, and **no keycards for
//! sale** — Facepunch pulled them because buying one skipped the monument
//! ladder.
//!
//! A trade is all or nothing on a scratch copy of the pack: the payment
//! leaves and the goods arrive together, or nothing moves.

use crate::craft::{inv_count, inv_take};
use crate::gather::{inv_add, GatherContent};
use crate::limits::{INV_SLOTS, MAX_VEND_OFFERS};
use crate::world::{EventQueue, Player, EV_VEND, EV_VEND_REFUSED};

/// How far from a kiosk's counter a player may trade, metres (planar).
pub const VEND_REACH_M: f32 = 3.0;
/// The most repeats one trade action may ask for.
pub const VEND_TIMES_MAX: u8 = 20;

/// Refusals, `EV_VEND_REFUSED.b`.
pub const REFUSE_V_KIND: u32 = 1;
pub const REFUSE_V_REACH: u32 = 2;
pub const REFUSE_V_FUNDS: u32 = 3;
pub const REFUSE_V_FULL: u32 = 4;
/// The offer waits on a work no one has lit (`works.rs`): THE GATE's
/// stalls restock as the island comes back.
pub const REFUSE_V_LOCKED: u32 = 5;
pub const REFUSE_V_MAX: u32 = REFUSE_V_LOCKED;

/// One offer: at kiosk `vendor`, pay `pay_n` of `pay` for `get_n` of `get`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VendOffer {
    pub vendor: u8,
    pub pay: u16,
    pub pay_n: u16,
    pub get: u16,
    pub get_n: u16,
    /// The world unlock it waits on, `works::NO_UNLOCK` for none.
    pub unlock: u8,
}

/// Every offer on the island, in content order (the wire's offer index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VendContent {
    pub offers: [VendOffer; MAX_VEND_OFFERS],
    pub count: u16,
}

impl VendContent {
    pub const EMPTY: VendContent = VendContent {
        offers: [VendOffer {
            vendor: 0,
            pay: 0,
            pay_n: 0,
            get: 0,
            get_n: 0,
            unlock: 0,
        }; MAX_VEND_OFFERS],
        count: 0,
    };

    pub fn get(&self, k: usize) -> Option<VendOffer> {
        (k < self.count as usize).then(|| self.offers[k])
    }
}

/// Apply one trade: offer `k`, `times` over. Refusals are events.
#[allow(clippy::too_many_arguments)]
pub fn trade(
    vc: &VendContent,
    gc: &GatherContent,
    unlocks: u32,
    town: &crate::town::Town,
    p: &mut Player,
    k: usize,
    times: u8,
    events: &mut EventQueue,
) {
    let refuse = |events: &mut EventQueue, code: u32| {
        events.push(EV_VEND_REFUSED, p.id, code, k as u32);
    };
    let Some(o) = vc.get(k) else {
        refuse(events, REFUSE_V_KIND);
        return;
    };
    if times == 0 || times > VEND_TIMES_MAX || p.dead || p.sleeping || p.wounded {
        refuse(events, REFUSE_V_KIND);
        return;
    }
    let Some((kx, kz)) = crate::town::kiosk_world(town, o.vendor as usize) else {
        refuse(events, REFUSE_V_KIND);
        return;
    };
    if !town.live {
        refuse(events, REFUSE_V_KIND);
        return;
    }
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (px - kx, pz - kz);
    if dx * dx + dz * dz > VEND_REACH_M * VEND_REACH_M {
        refuse(events, REFUSE_V_REACH);
        return;
    }
    if !crate::works::holds(unlocks, o.unlock) {
        refuse(events, REFUSE_V_LOCKED);
        return;
    }
    let pay = o.pay_n as u32 * times as u32;
    let get = o.get_n as u32 * times as u32;
    if inv_count(&p.inv, o.pay) < pay {
        refuse(events, REFUSE_V_FUNDS);
        return;
    }
    let mut scratch: [crate::gather::ItemStack; INV_SLOTS] = p.inv;
    inv_take(&mut scratch, o.pay, pay);
    let cap = gc.stack_max_of(o.get);
    let cond = gc.cond_max_of(o.get);
    let mut left = get;
    while left > 0 {
        let chunk = left.min(u16::MAX as u32) as u16;
        let added = inv_add(&mut scratch, o.get, chunk, cap, cond);
        if added < chunk {
            refuse(events, REFUSE_V_FULL);
            return;
        }
        left -= chunk as u32;
    }
    p.inv = scratch;
    events.push(EV_VEND, p.id, k as u32, times as u32);
}
