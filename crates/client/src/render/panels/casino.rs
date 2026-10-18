//! THE GATE's casino panels (`sim_core::casino`): `E` at a big wheel terminal
//! or a slot machine (`ui::interact::Verb::Wheel` / `Slots`).
//!
//! The wheel's panel is Rust's betting terminal: one row a colour, what it
//! pays and how often it comes up, your stake on it this round, and a BET
//! button that adds the chosen stake. The slot machine's is Rust's: a
//! multiplier, a lever, three reels, and the pay table.
//!
//! Both draw only what the shard told (`ClientCore::casino`); the stakes, the
//! reels and the pay are the sim's. A spin's result is told as it starts, so
//! the reels here and the wheel in the yard (`render/casino.rs`) keep it
//! until they stop.

use bevy::prelude::*;

use super::{
    font, font_bold, Hover, Panel, PanelRoot, Ui, BADGE, CELL_BG, PANEL_BG, SCRIM, TEXT, TEXT_DIM,
    TEXT_SHORT,
};
use crate::ui::craft::item_label;
use client_core::core::ClientCore;
use sim_core::casino::{
    self, SLOT_COST, SLOT_MULT_MAX, SLOT_SPIN_TICKS, SLOT_SYMBOLS, SLOT_TRIPLE, WHEEL,
    WHEEL_COLOURS, WHEEL_OPEN_TICKS, WHEEL_PAYS, WHEEL_ROUND_TICKS, WHEEL_SEGMENTS,
    WHEEL_STAKE_MAX,
};
use sim_core::limits::TICK_HZ;

const PANEL_W: f32 = 640.0;
const ROW_H: f32 = 50.0;
const HEAD_BG: Color = Color::srgba(0.118, 0.106, 0.094, 0.85);
const ROW_BG: Color = Color::srgba(0.212, 0.196, 0.176, 0.92);
const BET_BG: Color = Color::srgba(0.282, 0.357, 0.161, 0.95);
const BET_HOT: Color = Color::srgba(0.345, 0.439, 0.196, 1.0);

/// The wheel's colours as Rust paints them, in `casino::WHEEL_PAYS` order:
/// the name and an sRGB fill. The yard's wheel wears the same.
pub const COLOURS: [(&str, [f32; 3]); WHEEL_COLOURS] = [
    ("YELLOW", [0.93, 0.76, 0.16]),
    ("GREEN", [0.30, 0.66, 0.27]),
    ("BLUE", [0.20, 0.44, 0.82]),
    ("PURPLE", [0.55, 0.28, 0.72]),
    ("RED", [0.82, 0.18, 0.15]),
];

/// The stakes the stepper walks, junk.
const STAKES: [u16; 6] = [1, 5, 10, 25, 50, 100];
const _: () = assert!(STAKES[STAKES.len() - 1] as u32 == WHEEL_STAKE_MAX);

/// Each reel symbol's picture, by the item it is named for, and the word
/// drawn when no picture is baked (`casino::SYM_*` order).
const SYMBOLS: [(&str, &str); SLOT_SYMBOLS] = [
    ("Junk", "JUNK"),
    ("Rope", "ROPE"),
    ("Berries", "BERRY"),
    ("Low Grade Fuel", "LOW GRADE"),
    ("Wood", "WOOD"),
    ("Bandage", "BANDAGE"),
    ("Charcoal", "CHARCOAL"),
    ("Gunpowder", "POWDER"),
    ("", "GATE"),
    ("Cooked Meat", "MEAT"),
    ("Hammer", "HAMMER"),
    ("Sulfur", "SULFUR"),
    ("Gears", "GEARS"),
    ("Metal Fragments", "FRAGS"),
    ("Cloth", "CLOTH"),
    ("", "7 7 7"),
];

pub fn colour(c: usize) -> Color {
    let [r, g, b] = COLOURS[c % WHEEL_COLOURS].1;
    Color::srgb(r, g, b)
}

/// A BET button: the colour.
#[derive(Component)]
pub struct WheelBet(pub u8);
/// The stake stepper, or the slot multiplier's: which way.
#[derive(Component)]
pub struct CasinoStep(pub i8);
/// The lever.
#[derive(Component)]
pub struct SlotsSpin;
/// The wheel's clock line, rewritten every frame ([`tick`]).
#[derive(Component)]
pub struct CasinoClock;
/// Reel `r`'s picture and its word, turned every frame while it spins.
#[derive(Component)]
pub struct ReelPic(pub usize);
#[derive(Component)]
pub struct ReelWord(pub usize);

fn header(
    p: &mut ChildSpawnerCommands,
    title: &str,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
    let junk = core.research.coin;
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(4.0), Val::Px(10.0)),
        ..default()
    })
    .with_children(|h| {
        h.spawn((Text::new(title), font_bold(22.0), TextColor(TEXT)));
        if junk == sim_core::gather::NO_ITEM {
            return;
        }
        h.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|j| {
            if let Some(handle) = icons.item(&item_label(&core.catalog, junk)) {
                j.spawn((
                    ImageNode::new(handle),
                    Node {
                        width: Val::Px(26.0),
                        height: Val::Px(26.0),
                        ..default()
                    },
                ));
            }
            j.spawn((
                Text::new(format!("{}", sim_core::craft::inv_count(&core.inv, junk))),
                font_bold(16.0),
                TextColor(TEXT),
            ));
        });
    });
}

fn footer(p: &mut ChildSpawnerCommands, ui: &Ui, fine: &str) {
    p.spawn((
        Text::new(fine.to_string()),
        font(11.0),
        TextColor(TEXT_DIM),
        Node {
            margin: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(6.0), Val::Px(0.0)),
            ..default()
        },
    ));
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
        f.spawn((Text::new("[ESC] CLOSE"), font(11.0), TextColor(TEXT_DIM)));
    });
}

fn button(r: &mut ChildSpawnerCommands, tag: impl Bundle, w: f32, label: &str, live: bool) {
    let (bg, ink) = if live {
        (BET_BG, BADGE)
    } else {
        (CELL_BG, TEXT_DIM)
    };
    let mut b = r.spawn((
        Button,
        tag,
        Node {
            width: Val::Px(w),
            height: Val::Px(30.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(bg),
    ));
    if live {
        b.insert(Hover::on(bg).hot(BET_HOT));
    }
    b.with_child((
        Text::new(label.to_string()),
        font_bold(14.0),
        TextColor(ink),
        Pickable::IGNORE,
    ));
}

/// `− n +`, the value in a dark well between two small buttons.
fn stepper(r: &mut ChildSpawnerCommands, value: String) {
    let step = |r: &mut ChildSpawnerCommands, by: i8, glyph: &str| {
        r.spawn((
            Button,
            CasinoStep(by),
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
        .with_child((
            Text::new(glyph.to_string()),
            font_bold(16.0),
            TextColor(TEXT_DIM),
            Pickable::IGNORE,
        ));
    };
    step(r, -1, "-");
    r.spawn((
        Node {
            width: Val::Px(64.0),
            height: Val::Px(30.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(HEAD_BG),
    ))
    .with_child((Text::new(value), font_bold(15.0), TextColor(TEXT)));
    step(r, 1, "+");
}

fn root(commands: &mut Commands, body: impl FnOnce(&mut ChildSpawnerCommands)) {
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
            .with_children(body);
            super::spawn_tip(root);
        });
}

/// The wheel's betting terminal.
pub fn build_wheel(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
    let stake = STAKES[(ui.wheel_stake as usize).min(STAKES.len() - 1)];
    let have = sim_core::craft::inv_count(&core.inv, core.research.coin);
    root(commands, |p| {
        header(p, "THE BIG WHEEL", core, icons);
        p.spawn((
            Text::new(clock_line(core)),
            font_bold(14.0),
            TextColor(super::LINE_HOT),
            CasinoClock,
            Node {
                margin: UiRect::horizontal(Val::Px(8.0)),
                ..default()
            },
        ));
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(4.0),
            padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(4.0), Val::Px(6.0)),
            ..default()
        })
        .with_children(|r| {
            r.spawn((
                Text::new("STAKE"),
                font_bold(12.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::right(Val::Px(8.0)),
                    ..default()
                },
            ));
            stepper(r, format!("{stake}"));
        });
        for c in 0..WHEEL_COLOURS {
            let hits = WHEEL.iter().filter(|&&s| s as usize == c).count();
            let mine = core.casino.stakes[c];
            let room = (mine as u32 + stake as u32) <= WHEEL_STAKE_MAX;
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
                row.spawn((
                    Node {
                        width: Val::Px(26.0),
                        height: Val::Px(26.0),
                        ..default()
                    },
                    BackgroundColor(colour(c)),
                ));
                row.spawn((
                    Text::new(format!("{}  ×{}", COLOURS[c].0, WHEEL_PAYS[c])),
                    font_bold(15.0),
                    TextColor(TEXT),
                    Node {
                        width: Val::Px(130.0),
                        ..default()
                    },
                ));
                row.spawn((
                    Text::new(format!("{hits} in {WHEEL_SEGMENTS}")),
                    font(12.0),
                    TextColor(TEXT_DIM),
                    Node {
                        width: Val::Px(70.0),
                        ..default()
                    },
                ));
                row.spawn((
                    Text::new(if mine > 0 {
                        format!("ON IT: {mine}")
                    } else {
                        String::new()
                    }),
                    font_bold(13.0),
                    TextColor(BADGE),
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
                let live = casino::wheel_open(core.server_tick_est().max(0.0) as u64)
                    && room
                    && have >= stake as u32;
                button(row, WheelBet(c as u8), 96.0, &format!("BET {stake}"), live);
            });
        }
        footer(
            p,
            ui,
            "A win pays the stake back and its colour's multiple on top. Bets shut 15 s before \
             the wheel stops; at most 100 junk a colour each spin.",
        );
    });
}

/// A slot machine.
pub fn build_slots(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
    let mult = ui.slot_mult.clamp(1, SLOT_MULT_MAX);
    let cost = SLOT_COST * mult as u32;
    let have = sim_core::craft::inv_count(&core.inv, core.research.coin);
    let shown = reels_now(core, ui.slot_machine).0;
    root(commands, |p| {
        header(
            p,
            &format!("SLOT MACHINE {}", ui.slot_machine + 1),
            core,
            icons,
        );
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(10.0),
            padding: UiRect::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|reels| {
            for (r, &sym) in shown.iter().enumerate() {
                reels
                    .spawn((
                        Node {
                            width: Val::Px(150.0),
                            height: Val::Px(120.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.93, 0.91, 0.86, 0.96)),
                    ))
                    .with_children(|cell| {
                        let (name, word) = SYMBOLS[sym as usize % SLOT_SYMBOLS];
                        let pic = icons.item(name).unwrap_or_default();
                        cell.spawn((
                            ImageNode::new(pic),
                            ReelPic(r),
                            Node {
                                width: Val::Px(64.0),
                                height: Val::Px(64.0),
                                ..default()
                            },
                        ));
                        cell.spawn((
                            Text::new(word),
                            font_bold(14.0),
                            TextColor(Color::srgb(0.16, 0.13, 0.10)),
                            ReelWord(r),
                        ));
                    });
            }
        });
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(4.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            ..default()
        })
        .with_children(|r| {
            r.spawn((
                Text::new("BET"),
                font_bold(12.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::right(Val::Px(8.0)),
                    ..default()
                },
            ));
            stepper(r, format!("×{mult}"));
            r.spawn((
                Text::new(format!("{cost} JUNK A SPIN")),
                font_bold(13.0),
                TextColor(if have >= cost { TEXT } else { TEXT_SHORT }),
                Node {
                    flex_grow: 1.0,
                    margin: UiRect::left(Val::Px(10.0)),
                    ..default()
                },
            ));
            button(r, SlotsSpin, 110.0, "SPIN", have >= cost);
        });
        // The pay table at this multiplier, best first.
        let mut rows: Vec<(usize, u32)> = (0..SLOT_SYMBOLS)
            .filter(|&s| SLOT_TRIPLE[s] > 0)
            .map(|s| (s, SLOT_TRIPLE[s]))
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        p.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(16.0),
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(HEAD_BG),
        ))
        .with_children(|t| {
            for (s, pay) in rows {
                t.spawn((
                    Text::new(format!("3 × {}  {}", SYMBOLS[s].1, pay * mult as u32)),
                    font(12.0),
                    TextColor(TEXT),
                ));
            }
            t.spawn((
                Text::new(format!(
                    "each {}  +{}",
                    SYMBOLS[casino::SLOT_FACE as usize].1,
                    casino::SLOT_FACE_PAY * mult as u32
                )),
                font(12.0),
                TextColor(TEXT),
            ));
        });
        footer(
            p,
            ui,
            "Three of a kind pays the table times your bet; every low grade showing pays on top.",
        );
    });
}

/// The wheel's line: how long bets stay open, or that it is turning, and
/// where it last came to rest.
fn clock_line(core: &ClientCore) -> String {
    let now = core.server_tick_est().max(0.0);
    let (round, phase) = casino::wheel_phase(now as u64);
    let secs = |t: u64| t.div_ceil(TICK_HZ as u64);
    let last = match core.casino.spin {
        Some((r, seg)) if (r as u64) < round => {
            let c = WHEEL[seg as usize % WHEEL_SEGMENTS] as usize;
            format!("   ·   LAST SPIN: {} ×{}", COLOURS[c].0, WHEEL_PAYS[c])
        }
        _ => String::new(),
    };
    if phase < WHEEL_OPEN_TICKS {
        format!("BETS CLOSE IN {}s{last}", secs(WHEEL_OPEN_TICKS - phase))
    } else {
        format!("SPINNING — STOPS IN {}s", secs(WHEEL_ROUND_TICKS - phase))
    }
}

/// The reels machine `m` shows now, and whether they are still turning: the
/// last spin's, each stopping a third of the spin after the one before.
fn reels_now(core: &ClientCore, m: u8) -> ([u8; 3], bool) {
    let Some(r) = core.casino.slots.filter(|r| r.machine == m && r.code == 0) else {
        return ([casino::SYM_SEVEN; 3], false);
    };
    let t = core.server_tick_est() - core.casino.slots_at;
    let spin = SLOT_SPIN_TICKS as f64;
    let mut out = r.reels;
    let mut turning = false;
    for (i, s) in out.iter_mut().enumerate() {
        if t < spin * (i + 1) as f64 / 3.0 {
            turning = true;
            *s = ((t.max(0.0) * 0.5) as usize + i * 5) as u8 % SLOT_SYMBOLS as u8;
        }
    }
    (out, turning)
}

/// What a stopped spin paid, in the player's words.
fn slots_line(r: &client_core::core::SlotsResult) -> String {
    let [a, b, c] = r.reels.map(|s| s as usize % SLOT_SYMBOLS);
    if r.pay == 0 {
        return "no luck".into();
    }
    if a == b && b == c {
        format!("three {} — won {} junk", SYMBOLS[a].1, r.pay)
    } else {
        format!("won {} junk", r.pay)
    }
}

/// Every frame, in place: the wheel's clock and the turning reels; and as a
/// spin stops, its line on the status bar.
pub fn tick(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    icons: Option<Res<super::super::icons::Icons>>,
    mut clock: Query<&mut Text, (With<CasinoClock>, Without<ReelWord>)>,
    mut pics: Query<(&ReelPic, &mut ImageNode)>,
    mut words: Query<(&ReelWord, &mut Text), Without<CasinoClock>>,
) {
    let core = &net.session.core;
    match ui.panel {
        Panel::BigWheel => {
            let line = clock_line(core);
            for mut t in clock.iter_mut() {
                if t.0 != line {
                    t.0 = line.clone();
                }
            }
        }
        Panel::Slots => {
            let (shown, turning) = reels_now(core, ui.slot_machine);
            if let Some(icons) = icons.as_deref() {
                for (r, mut img) in pics.iter_mut() {
                    let pic = icons
                        .item(SYMBOLS[shown[r.0] as usize].0)
                        .unwrap_or_default();
                    if img.image != pic {
                        img.image = pic;
                    }
                }
            }
            for (r, mut t) in words.iter_mut() {
                let word = SYMBOLS[shown[r.0] as usize].1;
                if t.0 != word {
                    t.0 = word.to_string();
                }
            }
            let seq = core.casino.slots_seq;
            if !turning && ui.casino_seq.2 != seq {
                ui.casino_seq.2 = seq;
                if let Some(r) = core.casino.slots.filter(|r| r.code == 0) {
                    ui.say(slots_line(&r));
                }
            }
        }
        _ => {}
    }
}

/// A BET adds the stake to its colour; the lever pulls at the multiplier; a
/// stepper walks the stake or the multiplier.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    bets: Query<(&Interaction, &WheelBet), Changed<Interaction>>,
    steps: Query<(&Interaction, &CasinoStep), Changed<Interaction>>,
    levers: Query<&Interaction, (Changed<Interaction>, With<SlotsSpin>)>,
) {
    if !matches!(ui.panel, Panel::BigWheel | Panel::Slots) {
        return;
    }
    for (interaction, st) in steps.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if ui.panel == Panel::BigWheel {
            ui.wheel_stake =
                (ui.wheel_stake as i16 + st.0 as i16).clamp(0, STAKES.len() as i16 - 1) as u8;
        } else {
            ui.slot_mult = (ui.slot_mult as i16 + st.0 as i16).clamp(1, SLOT_MULT_MAX as i16) as u8;
        }
        ui.dirty = true;
    }
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    for (interaction, bet) in bets.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let stake = STAKES[(ui.wheel_stake as usize).min(STAKES.len() - 1)];
        match protocol::encode_action_wheel(bet.0, stake, &mut buf) {
            Ok(len) => {
                if let Err(e) = net.session.send_action(&buf[..len]) {
                    ui.say(e.to_string());
                }
            }
            Err(e) => ui.say(format!("that bet would not encode ({e:?})")),
        }
    }
    for interaction in levers.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mult = ui.slot_mult.clamp(1, SLOT_MULT_MAX);
        match protocol::encode_action_slots(ui.slot_machine, mult, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say("spinning…".to_string()),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that spin would not encode ({e:?})")),
        }
    }
}

/// The status line hears the shard: a bet taken or refused, what the wheel
/// paid, a lever refused. A spin's result waits for its reels ([`tick`]).
pub fn sync_status(mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    if !matches!(ui.panel, Panel::BigWheel | Panel::Slots) {
        return;
    }
    let c = &net.session.core.casino;
    if ui.casino_seq.0 != c.bet_seq {
        ui.casino_seq.0 = c.bet_seq;
        if let Some((colour, stake, code)) = c.bet {
            let name = COLOURS[colour as usize % WHEEL_COLOURS].0.to_lowercase();
            ui.say(if code == 0 {
                format!("{stake} junk on {name}")
            } else {
                crate::ui::refusals::casino(code)
            });
        }
    }
    if ui.casino_seq.1 != c.paid_seq {
        ui.casino_seq.1 = c.paid_seq;
        if let Some((staked, paid)) = c.paid {
            let landed = c
                .spin
                .map(|(_, seg)| COLOURS[WHEEL[seg as usize % WHEEL_SEGMENTS] as usize].0)
                .unwrap_or("?")
                .to_lowercase();
            ui.say(if paid > 0 {
                format!("{landed} — the wheel paid you {paid} junk")
            } else {
                format!("{landed} — you lost {staked} junk")
            });
        }
    }
    if let Some(r) = c.slots.filter(|r| r.code != 0) {
        if ui.casino_seq.2 != c.slots_seq {
            ui.casino_seq.2 = c.slots_seq;
            ui.say(crate::ui::refusals::casino(r.code));
        }
    }
}
