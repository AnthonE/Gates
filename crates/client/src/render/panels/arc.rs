//! The arc's two screens (`ARC.md` F4):
//!
//! - **WORK** — `E` at a work's terminal: its state, its quota line by line
//!   with what you carry and a DEPOSIT for each, and once it burns its tank
//!   and FUEL.
//! - **ISLAND** — `O` anywhere: the act, every work's bar, what each one
//!   gives the whole server, and what the island holds now.
//!
//! Both draw from `ClientCore::arc` and send one `ACT_ARC` per press; reach,
//! state and what you carry are the sim's verdict, and its answer lands on
//! the status line ([`sync_status`]). Words come from `crate::ui::arc`.

use bevy::prelude::*;

use super::kit::{self, BAR_FILL, BAR_TANK};
use super::{Panel, Ui, LINE_HOT, TEXT, TEXT_DIM};
use crate::render::feed::{Feed, Refused};
use crate::ui::arc as words;
use crate::ui::craft::item_label;
use client_core::core::ClientCore;
use sim_core::works::{ARG_ALL, OP_DEPOSIT, OP_FUEL, WORK_LIT, WORK_OPEN};

const WORK_W: f32 = 600.0;
const ISLAND_W: f32 = 640.0;

/// A press on a work: `ACT_ARC`'s op, target and argument.
#[derive(Component, Clone, Copy)]
pub struct ArcButton {
    pub op: u8,
    pub target: u8,
    pub arg: u8,
}

/// The server tick this client believes it is, for the hours a work names.
fn now(core: &ClientCore) -> f64 {
    core.clock.server_est
}

pub fn build_work(commands: &mut Commands, ui: &Ui, core: &ClientCore) {
    let k = ui.work as usize;
    let Some(w) = core.arc.works.get(k).filter(|w| w.known).copied() else {
        kit::screen(commands, WORK_W, |p| {
            kit::header(p, "THE WORK", None, &ui.status);
            kit::line(p, "it has not woken for you yet", 13.0, TEXT_DIM);
            kit::hint(p, "[ESC] CLOSE");
        });
        return;
    };
    let have = |item: u16| sim_core::craft::inv_count(&core.inv, item);
    kit::screen(commands, WORK_W, |p| {
        kit::header(p, w.name(), Some(words::act_title(w.def.act)), &ui.status);
        kit::strong(p, words::state_line(&w, now(core)), 14.0, TEXT);
        match w.state {
            WORK_LIT if w.def.fuel_max > 0 => kit::bar(p, words::tank_pct(&w) * 10, BAR_TANK, None),
            WORK_LIT => {}
            _ => kit::bar(p, w.progress_pm(), BAR_FILL, None),
        }
        let gives = words::gives_line(&w);
        if !gives.is_empty() {
            kit::line(p, format!("GIVES EVERYONE  {gives}"), 12.0, LINE_HOT);
        }

        kit::section(p, "THE QUOTA");
        let open = w.state == WORK_OPEN;
        let mut any = false;
        for (i, input) in w
            .def
            .inputs
            .iter()
            .enumerate()
            .take(w.def.n_inputs as usize)
        {
            let got = w.got[i].min(input.need);
            let carry = have(input.item);
            let left = input.need - got;
            any |= open && carry > 0 && left > 0;
            kit::row(p, |r| {
                kit::cell(
                    r,
                    item_label(&core.catalog, input.item).to_uppercase(),
                    150.0,
                    TEXT,
                );
                kit::cell(
                    r,
                    format!("{got} / {}", input.need),
                    140.0,
                    kit::count_color(got, input.need),
                );
                kit::cell(r, format!("you carry {carry}"), 130.0, TEXT_DIM);
                kit::button(
                    r,
                    "DEPOSIT",
                    ArcButton {
                        op: OP_DEPOSIT,
                        target: k as u8,
                        arg: i as u8,
                    },
                    open && carry > 0 && left > 0,
                );
            });
        }
        if w.state != WORK_LIT {
            kit::row(p, |r| {
                kit::button(
                    r,
                    "DEPOSIT EVERYTHING IT TAKES",
                    ArcButton {
                        op: OP_DEPOSIT,
                        target: k as u8,
                        arg: ARG_ALL,
                    },
                    any,
                );
            });
        }

        if w.def.fuel_max > 0 {
            kit::section(p, "THE TANK");
            let carry = have(w.def.fuel);
            let lit = w.state == WORK_LIT;
            kit::row(p, |r| {
                kit::cell(
                    r,
                    item_label(&core.catalog, w.def.fuel).to_uppercase(),
                    150.0,
                    TEXT,
                );
                kit::cell(r, format!("{} / {}", w.fuel, w.def.fuel_max), 140.0, TEXT);
                kit::cell(r, format!("you carry {carry}"), 130.0, TEXT_DIM);
                kit::button(
                    r,
                    "FUEL",
                    ArcButton {
                        op: OP_FUEL,
                        target: k as u8,
                        arg: 0,
                    },
                    lit && carry > 0 && w.fuel < w.def.fuel_max,
                );
            });
            kit::line(
                p,
                format!(
                    "burns {} an hour on a busy island, slower on a quiet one",
                    w.def.burn_per_hour
                ),
                11.0,
                TEXT_DIM,
            );
        }
        if let Some(share) = words::share_line(&w) {
            kit::strong(p, share, 12.0, LINE_HOT);
        }
        kit::hint(p, "[ESC] CLOSE");
    });
}

/// An ISLAND tab: 0 the works, 1 the journal.
#[derive(Component, Clone, Copy)]
pub struct IslandTab(pub u8);

pub fn build_island(commands: &mut Commands, ui: &Ui, core: &ClientCore) {
    let arc = &core.arc;
    kit::screen(commands, ISLAND_W, |p| {
        kit::header(
            p,
            "THE ISLAND",
            Some(words::act_title(arc.act())),
            &ui.status,
        );
        kit::row(p, |r| {
            kit::button(r, "THE WORKS", IslandTab(0), ui.island_tab == 0);
            kit::button(r, "JOURNAL", IslandTab(1), ui.island_tab == 1);
        });
        if ui.island_tab == 1 {
            journal(p, core);
        } else {
            works(p, core);
        }
        kit::hint(p, "[O] OR [ESC] CLOSE");
    });
}

fn works(p: &mut ChildSpawnerCommands, core: &ClientCore) {
    let arc = &core.arc;
    kit::line(
        p,
        "The island is broken. The whole server switches it back on, \
         and what one work gives, it gives everybody.",
        12.0,
        TEXT_DIM,
    );
    let mut any = false;
    for (_, w) in arc.known() {
        any = true;
        kit::section(p, w.name());
        kit::strong(p, words::state_line(w, now(core)), 13.0, TEXT);
        match w.state {
            WORK_LIT if w.def.fuel_max > 0 => kit::bar(p, words::tank_pct(w) * 10, BAR_TANK, None),
            WORK_LIT => {}
            _ => kit::bar(p, w.progress_pm(), BAR_FILL, None),
        }
        let gives = words::gives_line(w);
        if !gives.is_empty() {
            kit::line(p, format!("gives everyone  {gives}"), 12.0, LINE_HOT);
        }
        if let Some(share) = words::share_line(w) {
            kit::line(p, share, 11.0, TEXT_DIM);
        }
    }
    if !any {
        kit::line(p, "no work has woken on this island yet", 13.0, TEXT_DIM);
    }
    kit::section(p, "THE ISLAND HOLDS");
    let mut held: Vec<&str> = Vec::new();
    for (_, w) in arc.known() {
        if w.def.floor != 0 && arc.holds(w.def.floor) {
            held.push(w.floor_name());
        }
        if w.def.ceiling != 0 && arc.holds(w.def.ceiling) {
            held.push(w.ceiling_name());
        }
    }
    if held.is_empty() {
        kit::line(p, "nothing yet — it starts broken", 12.0, TEXT_DIM);
    } else {
        kit::strong(p, held.join("  ·  "), 13.0, TEXT);
    }
}

/// What this player has learned: the glyphs, and the stones read.
fn journal(p: &mut ChildSpawnerCommands, core: &ClientCore) {
    let lore = &*core.lore;
    kit::section(p, "THE GLYPHS");
    kit::line(p, super::lore::reading_line(lore), 12.0, TEXT_DIM);
    let alphabet = core::str::from_utf8(lore.alphabet()).unwrap_or("");
    // The whole alphabet, spaced: what you read shows its letter.
    let spaced: String = alphabet.chars().flat_map(|c| [c, ' ']).collect();
    super::lore::glyph_text(p, spaced.trim_end(), lore);
    kit::section(p, "STONES YOU HAVE READ");
    let mut any = false;
    for (_, ins) in lore.known_inscriptions().filter(|(_, i)| i.len > 0) {
        any = true;
        super::lore::glyph_text(p, ins.text(), lore);
    }
    if !any {
        kit::line(p, "none yet — the old stones are out there", 12.0, TEXT_DIM);
    }
    let locks: Vec<&str> = lore.known_mechs().map(|(_, m)| m.name()).collect();
    if !locks.is_empty() {
        kit::section(p, "LOCKS ON THE ISLAND");
        kit::line(p, locks.join("  ·  "), 12.0, TEXT);
    }
}

/// A tab press switches the island page.
pub fn tabs(mut ui: ResMut<Ui>, presses: Query<(&Interaction, &IslandTab), Changed<Interaction>>) {
    if ui.panel != Panel::Island {
        return;
    }
    for (interaction, t) in presses.iter() {
        if *interaction == Interaction::Pressed && ui.island_tab != t.0 {
            ui.island_tab = t.0;
            ui.dirty = true;
        }
    }
}

/// A press sends the verb; the work's bars and the status line answer.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    presses: Query<(&Interaction, &ArcButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Work {
        return;
    }
    for (interaction, b) in presses.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_arc(b.op, b.target, b.arg, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say(if b.op == OP_FUEL {
                    "feeding it…"
                } else {
                    "giving it what you carry…"
                }),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that would not encode ({e:?})")),
        }
    }
}

/// The status line hears the sim: a refusal, or the work changing.
pub fn sync_status(feed: Res<Feed>, mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    if !matches!(ui.panel, Panel::Work | Panel::Island) {
        return;
    }
    for (which, code, _) in feed.refusals() {
        if which == Refused::Arc {
            ui.say(crate::ui::refusals::arc(code));
        }
    }
    let core = &net.session.core;
    for &(index, what, _) in feed.work_events() {
        if ui.panel == Panel::Island || index == ui.work {
            if let Some(line) = words::event_line(&core.arc, index, what, None) {
                ui.say(line);
            }
        }
    }
}

/// Whether the open work is still in reach — the panel shuts when it is not,
/// the tree's posture (`panels::keys`).
pub fn work_in_reach(core: &ClientCore, k: u8) -> bool {
    let [x, y, z] = core.predict.position();
    core.arc.works.get(k as usize).is_some_and(|w| {
        w.known
            && sim_core::spot::within(
                core.haven(),
                &w.def.spot,
                x,
                y,
                z,
                sim_core::works::WORK_REACH_M + 1.0,
            )
    })
}
