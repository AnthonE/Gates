//! The research table and the paper it makes, as the client reads them —
//! the pure half (research table v1).
//!
//! The table is a container (`sim_core::research`'s header says why), so
//! most of what a player does to it is the move verb every box already
//! takes. What is new is decided here, headless and gated by `tests/ui.rs`
//! §V, and `render::panels::inv` only draws it:
//!
//! - **which open container is a table** ([`table_open`]) — resolved out of
//!   the deploy sync the client already draws, the way the name bar is;
//! - **where a gesture sends a stack** — one unit into the item slot
//!   whatever the drag ([`table_grab`]), the coin into the coin slot, and a
//!   right-click that knows which is which ([`table_quick_move`]);
//! - **what the line under the slots says** ([`table_line`]), which is the
//!   sim's own start check read on the client's view, so the panel says
//!   the refusal before the press rather than after it;
//! - **what a sheet is called and drawn as** ([`stack_label`],
//!   [`icon_name`]) — "Revolver Blueprint", on the revolver's picture;
//! - **what using a slot sends** ([`use_as`]): paper is read, anything else
//!   is eaten;
//! - **how far along a research is** ([`TableClock`]), from the moment this
//!   client SAW it start — never guessed, because a bar that starts at zero
//!   on a table opened halfway through is a bar that lies.
//!
//! None of it predicts anything: the sim moves the items and runs the
//! clock, and the panel redraws from what the core says came back.

use protocol::event::ItemCatalog;
use sim_core::deploy::{box_key, DeployContent, DeployRec, ARCH_RESEARCH};
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::inventory::{is_own, CONT_BOX, CONT_SELF, REFUSE_M_BUSY, REFUSE_M_TABLE};
use sim_core::limits::{INV_SLOTS, TICK_HZ};
use sim_core::research::{blueprint_target, ResearchContent, TABLE_COIN_SLOT, TABLE_ITEM_SLOT};

use crate::ui::craft::item_label;
use crate::ui::slots::{move_args, quick_move, refusal_text, Grab, Quick};

/// Is the open container a research table? `kind` and `handle` are the
/// core's open container; the answer comes from the deploy record standing
/// at that address, filtered by the watermark for `box_item_at`'s reason —
/// a def row past it is zeroes and would read as some other archetype.
pub fn table_open(
    kind: u8,
    handle: u32,
    deploys: &[DeployRec],
    defs: &DeployContent,
    defs_have: u16,
) -> bool {
    kind == CONT_BOX
        && handle != 0
        && deploys.iter().any(|d| {
            box_key(d.cx, d.cz, d.level) == handle
                && u16::from(d.row) < defs_have
                && defs.defs[d.row as usize].arch == ARCH_RESEARCH
        })
}

/// The address a box handle packs (`deploy::box_key`'s inverse), for the
/// verbs that name a place rather than a container — the table's switch is
/// `ACT_USE`, which carries an address.
pub fn table_address(handle: u32) -> (u16, u16, u8) {
    (
        (handle >> 16) as u16,
        ((handle >> 4) & 0x0FFF) as u16,
        (handle & 0xF) as u8,
    )
}

/// Is this stack a sheet of paper? True for a blank as well — it is still
/// paper, it just teaches nothing.
pub fn is_paper(rc: &ResearchContent, s: ItemStack) -> bool {
    s.count > 0 && rc.blueprint != NO_ITEM && s.item == rc.blueprint
}

/// What a stack is called: `"Revolver Blueprint"` for paper with a target,
/// the item's own label for everything else — a blank sheet included,
/// which is honestly just "Blueprint".
pub fn stack_label(catalog: &ItemCatalog, rc: &ResearchContent, s: ItemStack) -> String {
    match blueprint_target(rc, s) {
        Some(t) => format!("{} {}", item_label(catalog, t), item_label(catalog, s.item)),
        None => item_label(catalog, s.item),
    }
}

/// The name a stack's picture is looked up by. Paper with a target is drawn
/// as the thing it teaches — on a blueprint ground, which is the panel's
/// half ([`is_paper`]) — so twelve sheets are twelve pictures rather than
/// twelve identical scrolls.
pub fn icon_name(catalog: &ItemCatalog, rc: &ResearchContent, s: ItemStack) -> String {
    match blueprint_target(rc, s) {
        Some(t) => item_label(catalog, t),
        None => item_label(catalog, s.item),
    }
}

/// What using an inventory slot sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseAs {
    /// `ACT_RESEARCH`: read the paper (`research::study`).
    Read,
    /// `ACT_CONSUME`: eat it, or be told why not (`survival.rs`).
    Consume,
}

/// Paper is read; everything else goes to the eat verb, which answers for
/// itself. A blank is read too, so its refusal is the research table's
/// sentence ("that cannot be researched") rather than the food one.
pub fn use_as(rc: &ResearchContent, s: ItemStack) -> UseAs {
    if is_paper(rc, s) {
        UseAs::Read
    } else {
        UseAs::Consume
    }
}

/// The gesture a drag INTO a table's item slot carries: one unit, whatever
/// the button. The slot holds one (`research::table_accepts`), and a
/// whole-stack drag of ammunition would otherwise bounce as a refusal for a
/// rule the panel already knew.
pub fn table_grab(to_table_item: bool, grab: Grab) -> Grab {
    if to_table_item {
        Grab::One
    } else {
        grab
    }
}

/// A drop the table will refuse whatever the arithmetic, said before the
/// round trip: something that is neither researchable nor coin onto the
/// working slots, anything onto the slots past them, and any move at all
/// while a research runs. `None` when only the sim can say (a merge, a
/// swap). The sentences are the sim's own (`refusal_text`).
pub fn table_drop_refusal(
    rc: &ResearchContent,
    running: bool,
    to_slot: usize,
    item: u16,
) -> Option<&'static str> {
    if running {
        return Some(refusal_text(REFUSE_M_BUSY as u8));
    }
    let fits = match to_slot {
        TABLE_ITEM_SLOT => rc.row_for(item).is_some(),
        TABLE_COIN_SLOT => item == rc.coin,
        _ => false,
    };
    (!fits).then(|| refusal_text(REFUSE_M_TABLE as u8))
}

/// Where a right-click sends a stack with a research table open.
///
/// Out of the table it is the ordinary quick-move, into the pack. Into it,
/// the stack picks its own slot — the coin into the coin slot, topped up
/// to its room, and anything researchable into the item slot, ONE unit —
/// and everything else is refused in the table's words, because "no room"
/// would be the wrong reason for a stick of wood at a research table.
#[allow(clippy::too_many_arguments)]
pub fn table_quick_move(
    handle: u32,
    running: bool,
    from_kind: u8,
    from_slot: usize,
    catalog: &ItemCatalog,
    rc: &ResearchContent,
    inv: &[ItemStack; INV_SLOTS],
    cont: &[ItemStack; INV_SLOTS],
    worn: &[ItemStack],
) -> Quick {
    if running {
        return Quick::Refused(refusal_text(REFUSE_M_BUSY as u8));
    }
    if !is_own(from_kind) {
        return quick_move(
            CONT_BOX, handle, from_kind, from_slot, catalog, inv, cont, worn,
        );
    }
    let src = if from_kind == CONT_SELF {
        inv.get(from_slot).copied().unwrap_or_default()
    } else {
        worn.get(from_slot).copied().unwrap_or_default()
    };
    if src.count == 0 {
        return Quick::Refused("there is nothing in that slot");
    }
    let (to_slot, grab) = if src.item == rc.coin {
        let there = cont[TABLE_COIN_SLOT];
        if there.count == 0 {
            (TABLE_COIN_SLOT, Grab::All)
        } else {
            let room = catalog
                .stack_max(src.item as usize)
                .saturating_sub(there.count);
            if room == 0 {
                return Quick::Refused("the table's junk slot is full");
            }
            (TABLE_COIN_SLOT, Grab::Fit(room))
        }
    } else if rc.row_for(src.item).is_some() {
        if cont[TABLE_ITEM_SLOT].count > 0 {
            return Quick::Refused("the table already holds something to research");
        }
        (TABLE_ITEM_SLOT, Grab::One)
    } else {
        return Quick::Refused(refusal_text(REFUSE_M_TABLE as u8));
    };
    match move_args(
        handle, from_kind, from_slot, CONT_BOX, to_slot, grab, inv, cont, worn,
    ) {
        Some(args) => Quick::Send(args),
        None => Quick::Refused("that move cannot be addressed from here"),
    }
}

/// What the line under a table's slots says — the sim's start check
/// (`research::begin`) read on the client's view of the two slots, in
/// order, so the panel names the refusal before the press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableLine {
    /// A research is running.
    Running,
    /// The item slot holds finished paper, teaching this item.
    Done { target: u16 },
    /// Nothing in the item slot.
    Empty,
    /// Something in the item slot that no row researches.
    NotResearchable,
    /// Researchable, and the coin slot is short.
    Short { cost: u16, have: u16 },
    /// Everything is in: pressing BEGIN starts it.
    Ready { cost: u16 },
}

/// Read the table. `cont` is the open container's view, `running` the lit
/// bit the core heard for this table's address.
pub fn table_line(rc: &ResearchContent, cont: &[ItemStack], running: bool) -> TableLine {
    if running {
        return TableLine::Running;
    }
    let sample = cont.get(TABLE_ITEM_SLOT).copied().unwrap_or_default();
    if let Some(target) = blueprint_target(rc, sample) {
        return TableLine::Done { target };
    }
    if sample.count == 0 {
        return TableLine::Empty;
    }
    let Some(row) = rc.row_for(sample.item) else {
        return TableLine::NotResearchable;
    };
    let coin = cont.get(TABLE_COIN_SLOT).copied().unwrap_or_default();
    let have = if coin.item == rc.coin { coin.count } else { 0 };
    if have < row.cost {
        TableLine::Short {
            cost: row.cost,
            have,
        }
    } else {
        TableLine::Ready { cost: row.cost }
    }
}

/// The line, in words. `coin` is the coin's label.
pub fn table_text(catalog: &ItemCatalog, line: TableLine, coin: &str) -> String {
    let coin = coin.to_uppercase();
    match line {
        TableLine::Running => "RESEARCHING...".to_string(),
        TableLine::Done { target } => format!(
            "DONE - TAKE THE {} BLUEPRINT",
            item_label(catalog, target).to_uppercase()
        ),
        TableLine::Empty => "PUT SOMETHING TO RESEARCH IN THE ITEM SLOT".to_string(),
        TableLine::NotResearchable => "THAT CANNOT BE RESEARCHED".to_string(),
        TableLine::Short { cost, have } => {
            format!("NEEDS {cost} {coin} - {have} IN THE {coin} SLOT")
        }
        TableLine::Ready { cost } => format!("COSTS {cost} {coin} - PRESS BEGIN"),
    }
}

/// When this client saw the open table start, so the panel can draw the
/// wait — and only then. Fed once a frame with the open table's handle
/// (0 for none) and whether it is running.
///
/// **The start has to be SEEN, from idle.** A table opened while it is
/// already running has no start this client knows (`EV_OVEN` carries the
/// fact, not the time), so it reads [`TableClock::fraction`] `None` and the
/// panel says "researching" with no bar, rather than a bar that begins at
/// zero and is wrong by however long the research had already run.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TableClock {
    handle: u32,
    /// This table was seen idle since it was opened — the only state from
    /// which a start can be timed.
    idle_seen: bool,
    started: Option<f32>,
}

impl TableClock {
    /// One frame's observation, `now` in the caller's seconds.
    pub fn observe(&mut self, handle: u32, running: bool, now: f32) {
        if handle != self.handle {
            *self = TableClock {
                handle,
                ..Default::default()
            };
        }
        if handle == 0 {
            return;
        }
        if !running {
            self.idle_seen = true;
            self.started = None;
        } else if self.started.is_none() && self.idle_seen {
            self.started = Some(now);
        }
    }

    /// Whether this client saw the running research start — the panel's
    /// question about whether to draw a bar at all.
    pub fn timed(&self) -> bool {
        self.started.is_some()
    }

    /// How far along the research is, `0..=1`, or `None` when this client
    /// did not see it start. `table_ticks` is the dripped wait.
    pub fn fraction(&self, now: f32, table_ticks: u16) -> Option<f32> {
        let started = self.started?;
        let total = table_ticks as f32 / TICK_HZ as f32;
        if total <= 0.0 {
            return None;
        }
        Some(((now - started) / total).clamp(0.0, 1.0))
    }
}
