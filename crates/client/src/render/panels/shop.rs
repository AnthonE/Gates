//! A player's vending machine (`sim_core::shop`) — `E` at one
//! (`ui::interact::Verb::Shop`). Everyone sees what is FOR SALE, its PRICE in
//! junk and how many lots are in stock, with BUY ×1 and ×5. The owner and
//! their crew also get the offer editor (an item picked from what is in the
//! machine, the lot and the price as steppers, SET and CLEAR) and STOCK,
//! which switches to the inventory page where the machine's slots take the
//! normal drag (the box stays open on the server across the switch).
//!
//! Draws from the offers the server sent (`ClientCore::shop_offers`) and the
//! open container's slots; every press is the sim's to judge, and its answer
//! lands on the status line ([`sync_status`]).

use bevy::prelude::*;

use super::kit;
use super::{Panel, Ui, TEXT, TEXT_DIM, TEXT_SHORT};
use crate::ui::craft::item_label;
use client_core::core::ClientCore;
use sim_core::limits::SHOP_ROWS;
use sim_core::shop::ShopAct;

const PANEL_W: f32 = 760.0;

/// A buy: the row and how many lots.
#[derive(Component, Clone, Copy)]
pub struct ShopBuy(pub u8, pub u8, pub sim_core::shop::ShopOffer);

/// Which part of a draft offer a stepper moves.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Item,
    Lot,
    Price,
}

/// A draft stepper: the row, the field and by how much.
#[derive(Component, Clone, Copy)]
pub struct ShopEdit(pub u8, pub Field, pub i16);

/// Send a row's draft as its offer.
#[derive(Component, Clone, Copy)]
pub struct ShopSet(pub u8);

/// Take a row's offer down.
#[derive(Component, Clone, Copy)]
pub struct ShopClear(pub u8);

/// Switch to the inventory page to stock the machine.
#[derive(Component, Clone, Copy)]
pub struct ShopStock;

/// The items in the open machine's slots, slot order, each once — what the
/// editor's item picker cycles through. Empty when the machine is not the
/// open container (yet).
pub fn stock_items(core: &ClientCore, key: u32) -> Vec<u16> {
    let mut out = Vec::new();
    if core.cont_kind != sim_core::inventory::CONT_BOX || core.cont_handle != key {
        return out;
    }
    for s in core.cont.iter().take(sim_core::limits::BOX_SLOTS) {
        if s.count > 0 && !out.contains(&s.item) {
            out.push(s.item);
        }
    }
    out
}

/// The draft rows from a machine's live offers; an empty row drafts the
/// first item in stock, one for one junk.
fn draft_from(view: &protocol::ShopView, stock: &[u16]) -> [(u16, u16, u16); SHOP_ROWS] {
    let mut d = [(0u16, 1u16, 1u16); SHOP_ROWS];
    for (r, row) in view.rows.iter().enumerate() {
        d[r] = if row.live() {
            (row.item, row.lot, row.price)
        } else {
            (stock.first().copied().unwrap_or(0), 1, 1)
        };
    }
    d
}

pub fn build_screen(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
    let key = ui.shop_key;
    let view = core.shop_offers(key).copied();
    let coin = view.map_or(sim_core::gather::NO_ITEM, |v| v.coin);
    let have = |item: u16| sim_core::craft::inv_count(&core.inv, item);
    let purse = format!(
        "{} {}",
        have(coin),
        item_label(&core.catalog, coin).to_uppercase()
    );
    kit::screen(commands, PANEL_W, |p| {
        kit::header(
            p,
            "VENDING MACHINE",
            view.is_some().then_some(purse.as_str()),
            &ui.status,
        );
        let Some(view) = view else {
            kit::line(p, "reading the machine…", 13.0, TEXT_DIM);
            kit::hint(p, "[ESC] CLOSE");
            return;
        };
        kit::section(p, "FOR SALE");
        let mut any = false;
        for (r, row) in view.rows.iter().enumerate() {
            if !row.live() {
                continue;
            }
            any = true;
            let lots = row.lots();
            kit::row(p, |line| {
                super::vendor::ware(line, core, icons, row.item, row.lot as u32, TEXT);
                super::vendor::ware(
                    line,
                    core,
                    icons,
                    view.coin,
                    row.price as u32,
                    kit::count_color(have(view.coin), row.price as u32),
                );
                kit::cell(
                    line,
                    if lots == 0 {
                        "SOLD OUT".to_string()
                    } else {
                        format!("{lots} in stock")
                    },
                    110.0,
                    if lots == 0 { TEXT_SHORT } else { TEXT_DIM },
                );
                for n in [1u8, 5] {
                    let afford = have(view.coin) >= row.price as u32 * n as u32;
                    kit::button(
                        line,
                        &format!("BUY ×{n}"),
                        ShopBuy(r as u8, n, row.terms()),
                        afford && lots >= n as u16,
                    );
                }
            });
        }
        if !any {
            kit::line(p, "nothing for sale yet", 13.0, TEXT_DIM);
        }
        if view.may_edit {
            kit::section(p, "YOUR OFFERS");
            for r in 0..SHOP_ROWS {
                let (item, lot, price) = ui.shop_draft[r];
                let live = view.rows[r].live();
                kit::row(p, |line| {
                    kit::cell(line, format!("#{}", r + 1), 24.0, TEXT_DIM);
                    kit::button(
                        line,
                        &item_label(&core.catalog, item),
                        ShopEdit(r as u8, Field::Item, 1),
                        true,
                    );
                    kit::cell(line, format!("×{lot}"), 44.0, TEXT);
                    kit::button(line, "-", ShopEdit(r as u8, Field::Lot, -1), true);
                    kit::button(line, "+", ShopEdit(r as u8, Field::Lot, 1), true);
                    kit::cell(line, format!("{price} junk"), 70.0, TEXT);
                    kit::button(line, "-10", ShopEdit(r as u8, Field::Price, -10), true);
                    kit::button(line, "-", ShopEdit(r as u8, Field::Price, -1), true);
                    kit::button(line, "+", ShopEdit(r as u8, Field::Price, 1), true);
                    kit::button(line, "+10", ShopEdit(r as u8, Field::Price, 10), true);
                    kit::button(line, "SET", ShopSet(r as u8), true);
                    kit::button(line, "CLEAR", ShopClear(r as u8), live);
                });
            }
            kit::row(p, |line| {
                kit::button(line, "STOCK THE MACHINE", ShopStock, true);
                kit::cell(
                    line,
                    "its slots hold the stock and the junk it takes",
                    360.0,
                    TEXT_DIM,
                );
            });
        }
        kit::hint(p, "[ESC] CLOSE");
    });
}

/// Keep the panel honest about the machine: the editor's drafts start from
/// its offers when they first arrive, and a machine the server stopped
/// showing (gone, out of reach) shuts the panel.
pub fn follow(mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    if ui.panel != Panel::Shop {
        return;
    }
    let core = &net.session.core;
    let key = ui.shop_key;
    let open = core.cont_kind == sim_core::inventory::CONT_BOX && core.cont_handle == key;
    if open {
        ui.shop_open_seen = true;
    } else if ui.shop_open_seen {
        ui.panel = Panel::None;
        ui.dirty = true;
        return;
    }
    if ui.shop_draft_key != key {
        if let Some(view) = core.shop_offers(key) {
            let stock = stock_items(core, key);
            ui.shop_draft = draft_from(view, &stock);
            ui.shop_draft_key = key;
            ui.dirty = true;
        }
    }
}

/// Send one shop press and say so.
fn send(ui: &mut Ui, net: &super::super::Net, act: ShopAct, said: &str) {
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_shop(ui.shop_key, act, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => ui.say(said.to_string()),
            Err(e) => ui.say(e.to_string()),
        },
        Err(e) => ui.say(format!("that would not encode ({e:?})")),
    }
}

/// The panel's presses: a buy, a draft stepper, set, clear, and stock.
#[allow(clippy::type_complexity)]
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    buys: Query<(&Interaction, &ShopBuy), Changed<Interaction>>,
    edits: Query<(&Interaction, &ShopEdit), Changed<Interaction>>,
    sets: Query<(&Interaction, &ShopSet), Changed<Interaction>>,
    clears: Query<(&Interaction, &ShopClear), Changed<Interaction>>,
    stocks: Query<&Interaction, (Changed<Interaction>, With<ShopStock>)>,
) {
    if ui.panel != Panel::Shop {
        return;
    }
    let pressed = |i: &Interaction| *i == Interaction::Pressed;
    for (i, b) in buys.iter() {
        if pressed(i) {
            send(
                &mut ui,
                &net,
                ShopAct::Buy {
                    row: b.0,
                    times: b.1,
                    terms: b.2,
                },
                "buying…",
            );
        }
    }
    for (i, e) in edits.iter() {
        if !pressed(i) {
            continue;
        }
        let r = e.0 as usize;
        if r >= SHOP_ROWS {
            continue;
        }
        let (item, lot, price) = ui.shop_draft[r];
        let step =
            |v: u16, by: i16, min: i32| (v as i32 + by as i32).clamp(min, u16::MAX as i32) as u16;
        ui.shop_draft[r] = match e.1 {
            Field::Item => {
                let stock = stock_items(&net.session.core, ui.shop_key);
                if stock.is_empty() {
                    ui.say("stock the machine first — the offer is picked from its slots");
                    continue;
                }
                let next = stock
                    .iter()
                    .position(|&s| s == item)
                    .map_or(0, |k| (k + 1) % stock.len());
                (stock[next], lot, price)
            }
            Field::Lot => (item, step(lot, e.2, 1), price),
            Field::Price => (item, lot, step(price, e.2, 0)),
        };
        ui.dirty = true;
    }
    for (i, s) in sets.iter() {
        if !pressed(i) {
            continue;
        }
        let r = s.0 as usize;
        let Some(&(item, lot, price)) = ui.shop_draft.get(r) else {
            continue;
        };
        send(
            &mut ui,
            &net,
            ShopAct::Set {
                row: s.0,
                item,
                lot,
                price,
            },
            "setting the offer…",
        );
    }
    for (i, c) in clears.iter() {
        if pressed(i) {
            send(
                &mut ui,
                &net,
                ShopAct::Clear { row: c.0 },
                "taking it down…",
            );
        }
    }
    for i in stocks.iter() {
        if pressed(i) {
            // The box stays open across the switch (`keys`); the inventory
            // page draws its slots beside the pack.
            ui.panel = Panel::Inventory;
            ui.drag = None;
            ui.dirty = true;
        }
    }
}

/// The status line hears the sim: what a press did, or why not. Reads the
/// core's last answer; pops nothing.
pub fn sync_status(mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    let core = &net.session.core;
    let (seq, result) = core.shop_last_result();
    if seq == ui.shop_seq {
        return;
    }
    ui.shop_seq = seq;
    let Some(r) = result else {
        return;
    };
    if ui.panel != Panel::Shop || r.key != ui.shop_key {
        return;
    }
    let line = if r.code != 0 {
        crate::ui::refusals::shop(r.code)
    } else {
        match r.op {
            sim_core::shop::SHOP_OP_BUY => {
                let row = core
                    .shop_offers(r.key)
                    .and_then(|v| v.rows.get(r.row as usize).copied())
                    .unwrap_or_default();
                format!(
                    "bought {} {} for {} junk",
                    row.lot as u32 * r.times as u32,
                    item_label(&core.catalog, row.item),
                    row.price as u32 * r.times as u32
                )
            }
            sim_core::shop::SHOP_OP_SET => format!("offer #{} is up", r.row + 1),
            _ => format!("offer #{} is down", r.row + 1),
        }
    };
    ui.say(line);
}
