//! A town kiosk's panel (THE GATE, `sim_core::vend`) — `E` at a kiosk
//! (`ui::interact::Verb::Trade`). Rust's shop, row for row: what is FOR SALE,
//! what it COSTS, a `− n +` stepper and a BUY button; a row still waiting on
//! the island says so in red where Rust says SOLD OUT. The junk count sits in
//! the header.
//!
//! Draws from the offers the server dripped (`SUB_VEND_OFFERS`) and sends
//! the offer index and how many times; reach, funds and room are the sim's
//! verdict, and its answer lands on the status line ([`sync_status`]).

use bevy::prelude::*;

use super::{
    font, font_bold, Hover, Panel, PanelRoot, Ui, BADGE, CELL_BG, PANEL_BG, SCRIM, TEXT, TEXT_DIM,
    TEXT_SHORT,
};
use crate::render::feed::{Feed, Refused};
use crate::ui::craft::item_label;
use client_core::core::ClientCore;

const PANEL_W: f32 = 640.0;
/// A row's height and its pictures', px.
const ROW_H: f32 = 64.0;
const PIC_PX: f32 = 44.0;
/// The most times one press buys: the wire's count is a `u8`, and twenty is
/// a stack of most things.
pub const VEND_MAX: u8 = 20;

/// The column heads' strip and a row's ground — Rust's shop is darker rows
/// on the panel, a hair apart.
const HEAD_BG: Color = Color::srgba(0.118, 0.106, 0.094, 0.85);
const ROW_BG: Color = Color::srgba(0.212, 0.196, 0.176, 0.92);
/// BUY: Rust's green, a dark plate under its own green ink.
const BUY_BG: Color = Color::srgba(0.282, 0.357, 0.161, 0.95);
const BUY_HOT: Color = Color::srgba(0.345, 0.439, 0.196, 1.0);
/// What SOLD OUT is in Rust: a dark red plate under red ink.
const OUT_BG: Color = Color::srgba(0.239, 0.098, 0.082, 0.92);

/// A buy button: offer index and how many times.
#[derive(Component)]
pub struct BuyButton(pub u8, pub u8);

/// A row's stepper: offer index and which way.
#[derive(Component)]
pub struct VendStep(pub u8, pub i8);

/// One side of a row: the picture and how many, under a column head.
fn ware(
    row: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
    item: u16,
    n: u32,
    ink: Color,
) {
    let name = item_label(&core.catalog, item);
    row.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            width: Val::Px(150.0),
            ..default()
        },
        super::Tip(name.clone()),
        Interaction::default(),
    ))
    .with_children(|c| {
        match icons.item(&name) {
            Some(handle) => {
                c.spawn((
                    ImageNode {
                        color: super::super::icons::PICTURE,
                        ..ImageNode::new(handle.clone())
                    },
                    Node {
                        width: Val::Px(PIC_PX),
                        height: Val::Px(PIC_PX),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            None => {
                c.spawn((
                    Text::new(name.to_uppercase()),
                    font_bold(11.0),
                    TextColor(TEXT_DIM),
                    Pickable::IGNORE,
                ));
            }
        }
        c.spawn((
            Text::new(format!("{n}")),
            font_bold(20.0),
            TextColor(ink),
            Pickable::IGNORE,
        ));
    });
}

/// A small square button in a row's stepper.
fn step(row: &mut ChildSpawnerCommands, offer: u8, by: i8, glyph: &str) {
    row.spawn((
        Button,
        VendStep(offer, by),
        Node {
            width: Val::Px(30.0),
            height: Val::Px(30.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(CELL_BG),
        Hover::on(CELL_BG),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(glyph.to_string()),
            font_bold(16.0),
            TextColor(TEXT_DIM),
            Pickable::IGNORE,
        ));
    });
}

pub fn build_screen(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
    let v = ui.vendor as usize;
    let name = String::from_utf8_lossy(core.vendor_name(v)).into_owned();
    // The coin research is paid in is junk (`content/research.toml`).
    let junk = core.research.coin;
    let have = |item: u16| sim_core::craft::inv_count(&core.inv, item);
    commands
        .spawn((
            PanelRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|root| {
            root.spawn((
                super::PanelBody,
                Node {
                    width: Val::Px(PANEL_W),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(12.0)),
                    row_gap: Val::Px(4.0),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|p| {
                // The header: the stall's name, and what you carry to pay it.
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(4.0), Val::Px(10.0)),
                    ..default()
                })
                .with_children(|h| {
                    h.spawn((
                        Text::new(if name.is_empty() {
                            "SHOP".into()
                        } else {
                            name.to_uppercase()
                        }),
                        font_bold(22.0),
                        TextColor(TEXT),
                    ));
                    if junk != sim_core::gather::NO_ITEM {
                        h.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|j| {
                            if let Some(handle) = icons.item(&item_label(&core.catalog, junk)) {
                                j.spawn((
                                    ImageNode::new(handle.clone()),
                                    Node {
                                        width: Val::Px(26.0),
                                        height: Val::Px(26.0),
                                        ..default()
                                    },
                                ));
                            }
                            j.spawn((
                                Text::new(format!("{}", have(junk))),
                                font_bold(16.0),
                                TextColor(TEXT),
                            ));
                        });
                    }
                });
                // The column heads.
                p.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(14.0),
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(HEAD_BG),
                ))
                .with_children(|h| {
                    for (label, w) in [("FOR SALE", 150.0), ("COST", 150.0)] {
                        h.spawn((
                            Text::new(label),
                            font_bold(12.0),
                            TextColor(TEXT_DIM),
                            Node {
                                width: Val::Px(w),
                                ..default()
                            },
                        ));
                    }
                });
                let mut any = false;
                for (k, o) in core.vend.offers[..core.vend.count as usize]
                    .iter()
                    .enumerate()
                {
                    if o.vendor as usize != v {
                        continue;
                    }
                    any = true;
                    let n = ui.vend_n.get(k).copied().unwrap_or(1).clamp(1, VEND_MAX);
                    let bill = o.pay_n as u32 * n as u32;
                    let afford = have(o.pay) >= bill;
                    // SALVAGE's rows pay junk out: you are selling to it.
                    let sells = o.get == junk;
                    p.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(14.0),
                            height: Val::Px(ROW_H),
                            padding: UiRect::axes(Val::Px(14.0), Val::Px(0.0)),
                            ..default()
                        },
                        BackgroundColor(ROW_BG),
                    ))
                    .with_children(|row| {
                        ware(row, core, icons, o.get, o.get_n as u32 * n as u32, TEXT);
                        ware(
                            row,
                            core,
                            icons,
                            o.pay,
                            bill,
                            if afford { TEXT } else { TEXT_SHORT },
                        );
                        // Under the counter until the island can make it
                        // (`ARC.md` §1.3): the row says which work must burn
                        // instead of offering a press the sim would refuse —
                        // where Rust's shop says SOLD OUT.
                        if let Some(why) = crate::ui::arc::locked_line(&core.arc, o.unlock) {
                            row.spawn((
                                Node {
                                    flex_grow: 1.0,
                                    height: Val::Px(ROW_H - 12.0),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    padding: UiRect::horizontal(Val::Px(8.0)),
                                    ..default()
                                },
                                BackgroundColor(OUT_BG),
                            ))
                            .with_child((
                                Text::new(why.to_uppercase()),
                                font_bold(12.0),
                                TextColor(TEXT_SHORT),
                            ));
                            return;
                        }
                        // The stepper and the button, pushed right.
                        row.spawn(Node {
                            flex_grow: 1.0,
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::FlexEnd,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(4.0),
                            ..default()
                        })
                        .with_children(|r| {
                            step(r, k as u8, -1, "-");
                            r.spawn((
                                Node {
                                    width: Val::Px(52.0),
                                    height: Val::Px(30.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(HEAD_BG),
                            ))
                            .with_child((
                                Text::new(format!("{n}")),
                                font_bold(15.0),
                                TextColor(TEXT),
                            ));
                            step(r, k as u8, 1, "+");
                            let (bg, ink) = if afford {
                                (BUY_BG, BADGE)
                            } else {
                                (CELL_BG, TEXT_DIM)
                            };
                            let mut buy = r.spawn((
                                Button,
                                BuyButton(k as u8, n),
                                Node {
                                    width: Val::Px(84.0),
                                    height: Val::Px(30.0),
                                    margin: UiRect::left(Val::Px(8.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(bg),
                            ));
                            // A press it cannot pay for does not light.
                            if afford {
                                buy.insert(Hover::on(bg).hot(BUY_HOT));
                            }
                            buy.with_child((
                                Text::new(if sells { "SELL" } else { "BUY" }),
                                font_bold(14.0),
                                TextColor(ink),
                                Pickable::IGNORE,
                            ));
                        });
                    });
                }
                if !any {
                    p.spawn((
                        Text::new("this stall has nothing to trade yet"),
                        font(13.0),
                        TextColor(TEXT_DIM),
                        Node {
                            margin: UiRect::all(Val::Px(10.0)),
                            ..default()
                        },
                    ));
                }
                // The sim's last word, and how to leave.
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(8.0), Val::Px(2.0)),
                    ..default()
                })
                .with_children(|f| {
                    f.spawn((
                        Text::new(ui.status.clone()),
                        font(12.0),
                        TextColor(super::STATUS),
                    ));
                    // Esc only: with a panel up `verbs::keys` never sees `E`.
                    f.spawn((Text::new("[ESC] CLOSE"), font(11.0), TextColor(TEXT_DIM)));
                });
            });
            super::spawn_tip(root);
        });
}

/// A buy click sends the offer; the pack and the status line answer. A
/// stepper click moves that row's count.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    buys: Query<(&Interaction, &BuyButton), Changed<Interaction>>,
    steps: Query<(&Interaction, &VendStep), Changed<Interaction>>,
) {
    if ui.panel != Panel::Vendor {
        return;
    }
    for (interaction, st) in steps.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(n) = ui.vend_n.get_mut(st.0 as usize) {
            *n = (*n as i16 + st.1 as i16).clamp(1, VEND_MAX as i16) as u8;
            ui.dirty = true;
        }
    }
    for (interaction, buy) in buys.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_vend(buy.0, buy.1, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say("trading…".to_string()),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that trade would not encode ({e:?})")),
        }
    }
}

/// The status line hears the sim: what you bought, or why not. Reads the
/// frame's [`Feed`]; pops nothing.
pub fn sync_status(feed: Res<Feed>, mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    if ui.panel != Panel::Vendor {
        return;
    }
    let core = &net.session.core;
    for &(offer, times) in feed.traded() {
        if let Some(o) = core.vend.get(offer as usize) {
            ui.say(trade_line(core, o, times));
            ui.dirty = true;
        }
    }
    for (which, code, _) in feed.refusals() {
        if which == Refused::Vend {
            ui.say(crate::ui::refusals::vend(code));
        }
    }
}

/// What a trade that went through did, in the player's words: SALVAGE buys
/// from you, every other stall sells to you.
pub fn trade_line(core: &ClientCore, o: sim_core::vend::VendOffer, times: u8) -> String {
    let (pay, get) = (o.pay_n as u32 * times as u32, o.get_n as u32 * times as u32);
    if o.get == core.research.coin {
        format!(
            "sold {pay} {} for {get} {}",
            item_label(&core.catalog, o.pay),
            item_label(&core.catalog, o.get)
        )
    } else {
        format!(
            "bought {get} {} for {pay} {}",
            item_label(&core.catalog, o.get),
            item_label(&core.catalog, o.pay)
        )
    }
}
