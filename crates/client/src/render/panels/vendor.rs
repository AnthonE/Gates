//! A town kiosk's panel (THE GATE, `sim_core::vend`) — `E` at a kiosk
//! (`ui::interact::Verb::Trade`). One row per offer: what it takes, what it
//! gives, and BUY ×1 / ×5 / ×20. The junk count sits in the header.
//!
//! Draws from the offers the server dripped (`SUB_VEND_OFFERS`) and sends
//! the offer index alone; reach, funds and room are the sim's verdict, and
//! its answer lands on the status line ([`sync_status`]).

use bevy::prelude::*;

use super::{
    font, font_bold, Hover, Panel, PanelRoot, Ui, BADGE, CELL_BG, CELL_FULL, LINE, LINE_HOT,
    PANEL_BG, SCRIM, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::feed::{Feed, Refused};
use crate::ui::craft::item_label;
use client_core::core::ClientCore;

const PANEL_W: f32 = 560.0;

/// A buy button: offer index and how many times.
#[derive(Component)]
pub struct BuyButton(pub u8, pub u8);

fn icon(
    row: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
    item: u16,
    n: u16,
) {
    let name = item_label(&core.catalog, item);
    row.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(6.0),
        width: Val::Px(190.0),
        ..default()
    })
    .with_children(|c| {
        if let Some(handle) = icons.item(&name) {
            c.spawn((
                ImageNode {
                    color: super::super::icons::PICTURE,
                    ..ImageNode::new(handle.clone())
                },
                Node {
                    width: Val::Px(30.0),
                    height: Val::Px(30.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        c.spawn((
            Text::new(format!("{n} × {}", name.to_uppercase())),
            font(13.0),
            TextColor(TEXT),
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
                Node {
                    width: Val::Px(PANEL_W),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(14.0)),
                    row_gap: Val::Px(6.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(LINE),
            ))
            .with_children(|p| {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                })
                .with_children(|h| {
                    h.spawn((
                        Text::new(if name.is_empty() {
                            "VENDOR".into()
                        } else {
                            name
                        }),
                        font_bold(18.0),
                        TextColor(BADGE),
                    ));
                    if junk != sim_core::gather::NO_ITEM {
                        h.spawn((
                            Text::new(format!("JUNK  {}", have(junk))),
                            font_bold(14.0),
                            TextColor(TEXT),
                        ));
                    }
                });
                p.spawn((
                    Text::new(ui.status.clone()),
                    font(12.0),
                    TextColor(TEXT_DIM),
                ));
                let mut any = false;
                for (k, o) in core.vend.offers[..core.vend.count as usize]
                    .iter()
                    .enumerate()
                {
                    if o.vendor as usize != v {
                        continue;
                    }
                    any = true;
                    let can = have(o.pay) >= o.pay_n as u32;
                    // SALVAGE's rows pay junk out: you are selling to it.
                    let sells = o.get == junk;
                    p.spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(4.0)),
                            border: UiRect::bottom(Val::Px(1.0)),
                            ..default()
                        },
                        BorderColor::all(LINE),
                    ))
                    .with_children(|row| {
                        icon(row, core, icons, o.pay, o.pay_n);
                        row.spawn((Text::new("→"), font_bold(16.0), TextColor(TEXT_DIM)));
                        icon(row, core, icons, o.get, o.get_n);
                        for times in [1u8, 5, 20] {
                            let afford = have(o.pay) >= o.pay_n as u32 * times as u32;
                            let bg = if afford { CELL_FULL } else { CELL_BG };
                            row.spawn((
                                Button,
                                BuyButton(k as u8, times),
                                Node {
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(bg),
                                Hover::on(bg),
                                BorderColor::all(if afford { LINE_HOT } else { LINE }),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(match (times, sells) {
                                        (1, true) => "SELL".to_string(),
                                        (1, false) => "BUY".to_string(),
                                        // The bill, so ×20 says what it costs.
                                        _ => {
                                            format!("×{times} · {}", o.pay_n as u32 * times as u32)
                                        }
                                    }),
                                    font_bold(12.0),
                                    TextColor(if afford {
                                        LINE_HOT
                                    } else if can {
                                        TEXT_DIM
                                    } else {
                                        TEXT_SHORT
                                    }),
                                    Pickable::IGNORE,
                                ));
                            });
                        }
                    });
                }
                if !any {
                    p.spawn((
                        Text::new("this stall has nothing to trade yet"),
                        font(13.0),
                        TextColor(TEXT_DIM),
                    ));
                }
                p.spawn((
                    // Esc only: with a panel up `verbs::keys` never sees `E`.
                    Text::new("[ESC] CLOSE"),
                    font(11.0),
                    TextColor(TEXT_DIM),
                ));
            });
        });
}

/// A buy click sends the offer; the pack and the status line answer.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    buys: Query<(&Interaction, &BuyButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Vendor {
        return;
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
