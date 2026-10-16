//! The crew's shop (`sim_core::shop`, `agent::society`): a vending machine
//! outside the crew's base, on the side THE GATE is, selling what its
//! farmers bring in for junk. This is what makes a small island's economy:
//! a player who would rather not chop wood for an hour buys it here, at half
//! what THE GATE asks, and the crew earns several times what THE GATE's
//! SALVAGE stall would have paid it.
//!
//! The work at it is a device session (`agent::oven`, `Work::Shop`): open
//! it, take the junk it took out, put the wares in (a slot always left for
//! the next takings), then set an offer for each ware it holds a lot of.
//! Prices are read off THE GATE's own stalls, as a player reads them, so a
//! change to `content/sites.toml` moves them too.

use crate::agent::build::item_named;
use client_core::core::ClientCore;
use protocol::ShopAct;
use sim_core::gather::ItemStack;
use sim_core::limits::{BOX_SLOTS, SHOP_ROWS};

/// The machine, by catalog name.
pub const SHOP_ITEM: &str = "Vending Machine";
/// What a crew sells, row by row: the raw stuff its farmers bring in by the
/// thousand, and THE GATE sells too.
pub const WARES: [&str; 4] = ["Wood", "Stone", "Metal Fragments", "Cloth"];
const _: () = assert!(WARES.len() <= SHOP_ROWS);
/// A crew's price, percent of THE GATE's for the same lot.
pub const PRICE_PCT: u32 = 50;
/// The machine stands this far from home's middle, towards THE GATE:
/// outside a starter base and its doors, inside its cupboard's claim.
pub const FROM_HOME_M: f32 = 12.0;
/// A machine near the spot is the crew's, within this many metres.
pub const SPOT_NEAR_M: f32 = 8.0;
/// A body stocks the shop once it carries this many lots of one ware.
pub const STOCK_LOTS: u32 = 2;
/// A shop is looked in on this often for its takings, with nothing to put
/// in it.
pub const VISIT_SECS: u32 = 10 * 60;

/// What the crew asks for `item`: `(lot, price)`, THE GATE's own lot of it
/// at [`PRICE_PCT`] of its price, and never less than one more than THE
/// GATE's SALVAGE stall pays for that lot — a shop that undersold the
/// salvage stall would be a gift. `None` for what the crew does not sell or
/// THE GATE does not price.
pub fn price(core: &ClientCore, item: u16) -> Option<(u16, u16)> {
    let junk = item_named(core, crate::agent::gate::JUNK)?;
    if !ware(core, item) {
        return None;
    }
    let offers = (0..usize::from(core.vend.count)).filter_map(|k| core.vend.get(k));
    let (lot, gate) = offers
        .clone()
        .filter(|o| o.pay == junk && o.get == item && o.get_n > 0)
        .map(|o| (o.get_n, u32::from(o.pay_n)))
        .min_by_key(|&(n, pay)| pay * 1000 / u32::from(n))?;
    // What SALVAGE pays for one lot, at its own rate.
    let salvage = offers
        .filter(|o| o.pay == item && o.get == junk && o.pay_n > 0)
        .map(|o| u32::from(o.get_n) * u32::from(lot) / u32::from(o.pay_n))
        .max()
        .unwrap_or(0);
    let ask = (gate * PRICE_PCT / 100)
        .max(salvage + 1)
        .min(u32::from(u16::MAX));
    Some((lot, ask as u16))
}

/// Is `item` one of the crew's [`WARES`]?
pub fn ware(core: &ClientCore, item: u16) -> bool {
    let name = core.catalog.name(usize::from(item));
    WARES.iter().any(|w| w.as_bytes() == name)
}

/// The row a ware is offered on.
fn row_of(core: &ClientCore, item: u16) -> Option<u8> {
    let name = core.catalog.name(usize::from(item));
    WARES
        .iter()
        .position(|w| w.as_bytes() == name)
        .map(|r| r as u8)
}

/// Where the crew's machine stands: [`FROM_HOME_M`] from home's middle
/// towards THE GATE (east of it, with no town to face).
pub fn spot(core: &ClientCore, home: [f32; 2]) -> [f32; 2] {
    let [dx, dz] = match crate::agent::gate::gate(core) {
        Some([gx, gz]) => {
            let (dx, dz) = (gx - home[0], gz - home[1]);
            let d = dx.hypot(dz);
            if d > 1.0 {
                [dx / d, dz / d]
            } else {
                [1.0, 0.0]
            }
        }
        None => [1.0, 0.0],
    };
    [home[0] + dx * FROM_HOME_M, home[1] + dz * FROM_HOME_M]
}

/// What to do next at an open machine about its offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Set or clear one row.
    Press(ShopAct),
    /// Its view has not come yet.
    Wait,
    /// Not the crew's: its view says this body may not edit it.
    NotOurs,
    /// Every ware it holds a lot of is on offer at the crew's price.
    Done,
}

/// The next offer to set at open machine `key`, from what it holds (`slots`,
/// its panel) and its view: a ware it holds a lot of and does not offer at
/// the crew's price is set; a ware's row with none left to sell is cleared.
/// Rows past the wares are a crewmate's own and are left be.
pub fn next_offer(core: &ClientCore, key: u32, slots: &[ItemStack]) -> Next {
    let Some(view) = core.shop_offers(key) else {
        return Next::Wait;
    };
    if !view.may_edit {
        return Next::NotOurs;
    }
    for name in WARES {
        let Some(item) = item_named(core, name) else {
            continue;
        };
        let (Some(row), Some((lot, price))) = (row_of(core, item), price(core, item)) else {
            continue;
        };
        let have: u32 = slots[..BOX_SLOTS.min(slots.len())]
            .iter()
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum();
        let now = view.rows[usize::from(row)];
        if have >= u32::from(lot) {
            if (now.item, now.lot, now.price) != (item, lot, price) {
                return Next::Press(ShopAct::Set {
                    row,
                    item,
                    lot,
                    price,
                });
            }
        } else if now.live() && now.item == item {
            return Next::Press(ShopAct::Clear { row });
        }
    }
    Next::Done
}

/// Lots of the most-carried ware the pack holds, as a reason to go.
pub fn lots_carried(core: &ClientCore) -> u32 {
    let mut best = 0;
    for name in WARES {
        let Some(item) = item_named(core, name) else {
            continue;
        };
        let Some((lot, _)) = price(core, item) else {
            continue;
        };
        let held: u32 = core.inv[..sim_core::limits::INV_SLOTS]
            .iter()
            .filter(|s| s.count > 0 && s.item == item)
            .map(|s| u32::from(s.count))
            .sum();
        best = best.max(held / u32::from(lot.max(1)));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{ItemRow, ShopRow, ShopView};
    use sim_core::vend::VendOffer;

    const WOOD: u16 = 0;
    const JUNK: u16 = 1;
    const STONE: u16 = 2;
    const BAT: u16 = 3;
    const KEY: u32 = 77;

    /// Wood, junk and stone, and THE GATE's stalls: wood sold at 15 junk the
    /// 500 and bought at 4 the 1000, stone sold at 20 the 500.
    fn core() -> Box<ClientCore> {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let names = ["Wood", "Junk", "Stone", "Bat"];
        core.catalog.count = names.len() as u16;
        for (i, name) in names.iter().enumerate() {
            let row = ItemRow {
                stack_max: 1000,
                ..ItemRow::EMPTY
            };
            core.catalog.set(i, name.as_bytes(), row).unwrap();
        }
        let offer = |pay, pay_n, get, get_n| VendOffer {
            vendor: 0,
            pay,
            pay_n,
            get,
            get_n,
            unlock: sim_core::works::NO_UNLOCK,
        };
        core.vend.offers[0] = offer(JUNK, 15, WOOD, 500);
        core.vend.offers[1] = offer(WOOD, 1000, JUNK, 4);
        core.vend.offers[2] = offer(JUNK, 20, STONE, 500);
        core.vend.count = 3;
        core
    }

    fn view(core: &mut ClientCore, may_edit: bool, rows: &[(usize, ShopRow)]) {
        let mut v = ShopView {
            key: KEY,
            coin: JUNK,
            may_edit,
            rows: [ShopRow::default(); SHOP_ROWS],
        };
        for &(i, r) in rows {
            v.rows[i] = r;
        }
        let mut buf = [0u8; protocol::event::MAX_EVENT_MSG_BYTES];
        let n = protocol::encode_event_shop(&v, &mut buf).unwrap();
        core.on_stream(&buf[..n]).unwrap();
    }

    fn stock(item: u16, count: u16) -> [ItemStack; BOX_SLOTS] {
        let mut slots = [ItemStack::default(); BOX_SLOTS];
        slots[0] = ItemStack {
            item,
            count,
            ..ItemStack::default()
        };
        slots
    }

    #[test]
    fn a_crew_asks_half_the_gates_price_and_more_than_salvage_pays() {
        let core = core();
        assert_eq!(price(&core, WOOD), Some((500, 7)));
        assert_eq!(price(&core, STONE), Some((500, 10)));
        assert_eq!(price(&core, BAT), None, "not a ware");
        assert!(ware(&core, WOOD) && !ware(&core, JUNK));
    }

    #[test]
    fn a_lot_in_stock_is_offered_and_an_empty_row_cleared() {
        let mut core = core();
        let wood = stock(WOOD, 900);
        assert_eq!(next_offer(&core, KEY, &wood), Next::Wait, "no view yet");
        view(&mut core, false, &[]);
        assert_eq!(next_offer(&core, KEY, &wood), Next::NotOurs);
        view(&mut core, true, &[]);
        let set = ShopAct::Set {
            row: 0,
            item: WOOD,
            lot: 500,
            price: 7,
        };
        assert_eq!(next_offer(&core, KEY, &wood), Next::Press(set));
        let offered = ShopRow {
            item: WOOD,
            lot: 500,
            price: 7,
            have: 900,
        };
        view(&mut core, true, &[(0, offered)]);
        assert_eq!(next_offer(&core, KEY, &wood), Next::Done);
        // Sold down below a lot: the row comes off.
        let low = stock(WOOD, 200);
        assert_eq!(
            next_offer(&core, KEY, &low),
            Next::Press(ShopAct::Clear { row: 0 })
        );
    }
}
