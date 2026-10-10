//! The hammer's wheel — the reference's second radial, over the verbs the
//! sim already answers (`NOW.md` §0p2 item 1).
//!
//! The mouse is held-item modal (`DECISIONS.md` 2026-08-07,
//! [`super::hold`]): the building plan's right-hold opens the shape wheel,
//! which **latches**; the hammer's right-hold opens this one, which
//! **fires on release**. The difference is the verbs' own: a shape is a
//! selection you keep, an action is a thing you do once, and a wheel that
//! latched "demolish" would arm a destructive verb into a field nothing
//! shows.
//!
//! ## The segment set is the live-command set
//!
//! Upgrade is `U`'s `encode_action_upgrade`, repair is `R`'s
//! `encode_action_repair`, and demolish and pick-up are both `Backspace`'s
//! `encode_action_demolish`, because the sim makes them one command with a
//! store bit: a **piece** comes down inside its grace window and refunds
//! whole (`build::demolish`), a **deployable** comes up any time you may
//! build there and returns its item (`deploy::pick_up`). The wheel splits
//! what the key merges — a segment named PICK UP that could fell a wall
//! would be the positional-payload trap wearing a menu — so [`act`] filters
//! by store and says why instead of sending the other verb.
//!
//! Rotate turns stairs clockwise or flips an edge's hard/soft facing. Its
//! grace window, permission and collision checks belong to the server.
//!
//! Whether the grace window is still open is **not** asked here, for
//! `demolish_near`'s stated reason: it is arithmetic over a tick the client
//! does not hold, and the sim's refusal (`REFUSE_B_WINDOW`) already has a
//! sentence.

use sim_core::build::{repair_quote, shape_has_facing, BuildContent, MAX_REPAIR_COSTS};
use sim_core::craft::inv_count;
use sim_core::deploy::DeployContent;
use sim_core::gather::ItemStack;
use sim_core::limits::INV_SLOTS;

use super::build::{Cost, Rings};
use super::structure::{self, Store, Target};

/// The verbs on the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Upgrade,
    Repair,
    Demolish,
    PickUp,
    Rotate,
}

/// The ring, clockwise from the top. Upgrade sits at the top as the verb a
/// base-builder reaches for most; demolish remains on the lower half,
/// separated from Upgrade by Repair.
pub const VERBS: [Verb; 5] = [
    Verb::Upgrade,
    Verb::Repair,
    Verb::Demolish,
    Verb::PickUp,
    Verb::Rotate,
];

pub fn label(v: Verb) -> &'static str {
    match v {
        Verb::Upgrade => "Upgrade",
        Verb::Repair => "Repair",
        Verb::Demolish => "Demolish",
        Verb::PickUp => "Pick Up",
        Verb::Rotate => "Rotate",
    }
}

/// The icon file for a verb — the shape wheel's
/// [`super::build::shape_icon`] for the second ring.
///
/// A stem is an asset name and a label is prose, so they are two functions
/// even though today they cover the same verb set. The gate is
/// `tests/ui.rs` §G: every stem named here has to be a file `assets/icons/`
/// actually ships, because a miss here is not a crash — `Icons::verb`
/// returns `None` and the wedge quietly falls back to its label, which is
/// the wheel it had before these were baked.
pub fn verb_icon(v: Verb) -> &'static str {
    match v {
        Verb::Upgrade => "verb_upgrade",
        Verb::Repair => "verb_repair",
        Verb::Demolish => "verb_demolish",
        Verb::PickUp => "verb_pick_up",
        Verb::Rotate => "verb_rotate",
    }
}

/// The centre's one-liner, the shape wheel's `shape_blurb` for verbs.
pub fn blurb(v: Verb) -> &'static str {
    match v {
        Verb::Upgrade => "Take the piece one rung up the material ladder.",
        Verb::Repair => "Mend what stands here.",
        Verb::Demolish => "Take a fresh piece back down for a full refund.",
        Verb::PickUp => "Lift a deployable back into your bag.",
        Verb::Rotate => "Turn stairs or flip a wall's hard and soft faces.",
    }
}

/// Which verb segment the pointer is in — [`super::build::pick`]'s
/// arithmetic over this ring's own count, so the two wheels cannot disagree
/// about where the band is.
pub fn pick(dx: f32, dy: f32, rings: Rings) -> Option<usize> {
    super::build::pick_in(dx, dy, rings, VERBS.len())
}

/// What releasing over a segment does with the nearest structure: the send
/// it names, or the sentence saying why not.
///
/// The addresses ride verbatim and the store bit is [`Store::is_deploy`]'s
/// — `ui::structure`'s one conversion site, kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Rotate {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// `encode_action_demolish` — piece demolish when `deploy` is false,
    /// deployable pick-up when true. One wire verb, two meanings, split by
    /// the same bit the sim branches on.
    Demolish {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// `encode_action_upgrade`, to the rung `structure::next_material`
    /// picked — the same one press `U` sends.
    Upgrade {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        material: u8,
    },
    /// `encode_action_repair`, either store.
    Repair {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Nothing to send; say this instead. The sentences match the keyboard
    /// paths' where the situation is the same, so one refusal never reads
    /// two ways.
    Say(&'static str),
}

/// Resolve a released verb against the nearest structure. Pure, gated in
/// `tests/ui.rs` §K; `render/verbs.rs::hammer_fire` translates the answer
/// into the same `send` the keys use.
pub fn act(verb: Verb, near: Option<&Target>, piece_defs: &BuildContent, have: u16) -> Act {
    match verb {
        Verb::Rotate => match near {
            None => Act::Say("nothing to rotate in reach"),
            Some(t) if t.store != Store::Piece => Act::Say("that is not a building piece"),
            Some(t) if t.row as u16 >= have.min(piece_defs.piece_count) => {
                Act::Say("waiting for building details")
            }
            Some(t) => {
                let shape = piece_defs.pieces[t.row as usize].shape;
                if !shape_has_facing(shape) && !sim_core::circulation::is_riser(shape) {
                    Act::Say("that piece has no rotation")
                } else {
                    Act::Rotate {
                        cx: t.cx,
                        cz: t.cz,
                        level: t.level,
                        loc: t.loc,
                    }
                }
            }
        },
        Verb::Demolish => match near {
            None => Act::Say("nothing to take down in reach"),
            Some(t) if t.store != Store::Piece => Act::Say("that comes up with PICK UP"),
            Some(t) => Act::Demolish {
                deploy: false,
                cx: t.cx,
                cz: t.cz,
                level: t.level,
                loc: t.loc,
            },
        },
        Verb::PickUp => match near {
            None => Act::Say("nothing to pick up in reach"),
            Some(t) if t.store != Store::Deploy => {
                Act::Say("a building piece comes down with DEMOLISH")
            }
            Some(t) => Act::Demolish {
                deploy: true,
                cx: t.cx,
                cz: t.cz,
                level: t.level,
                loc: t.loc,
            },
        },
        Verb::Upgrade => match near {
            None => Act::Say("nothing to upgrade in reach"),
            Some(t) if t.store != Store::Piece => Act::Say("that is not a building piece"),
            Some(t) => match structure::next_material(piece_defs, have, t.row) {
                None => Act::Say("nothing to upgrade into"),
                Some(material) => Act::Upgrade {
                    cx: t.cx,
                    cz: t.cz,
                    level: t.level,
                    loc: t.loc,
                    material,
                },
            },
        },
        Verb::Repair => match near {
            None => Act::Say("nothing to repair in reach"),
            // `repair_near`'s own guard: an undripped row (`hp_max == 0`)
            // sends and lets the sim answer, because the client does not
            // know the maximum.
            Some(t) if !t.damaged() && t.hp_max > 0 => Act::Say("not damaged"),
            Some(t) => Act::Repair {
                deploy: t.store.is_deploy(),
                cx: t.cx,
                cz: t.cz,
                level: t.level,
                loc: t.loc,
            },
        },
    }
}

/// The row an upgrade actually purchases. Restrict the lookup to received
/// rows: an undripped row must never look like a free upgrade in the wheel.
pub fn upgrade_row(near: &Target, defs: &BuildContent, have: u16) -> Option<u16> {
    if near.store != Store::Piece || near.row as u16 >= have.min(defs.piece_count) {
        return None;
    }
    let material = structure::next_material(defs, have, near.row)?;
    let shape = defs.pieces[near.row as usize].shape;
    (0..have.min(defs.piece_count)).find(|&row| {
        let def = defs.pieces[row as usize];
        def.shape == shape && def.material == material
    })
}

/// What mending `t` takes, row by row against the pack: its own cost rows,
/// pro rata to the hp it is missing, at the table's repair percent. That is
/// `sim_core::build::repair_quote`, the function `build::repair` charges
/// with, so the wheel names the bill rather than "cost depends on damage"
/// (wire v102 carries each record's hp and the percent) — for the hp the
/// client was last told, which decay can have lowered since. The rows are
/// therefore a floor on the bill, and [`repair_line`] says so.
///
/// `None` while the client cannot name it: the row or the percent has not
/// dripped, or the hp is unknown. Zero rows is an answer, not a wait —
/// nothing missing, or a row that quotes no price, `repair`'s refusals.
pub fn repair_rows(
    t: &Target,
    pieces: &BuildContent,
    piece_have: u16,
    deploys: &DeployContent,
    deploy_have: u16,
    inv: &[ItemStack; INV_SLOTS],
) -> Option<([Cost; MAX_REPAIR_COSTS], usize)> {
    let mut rows = [(0u16, 0u16); MAX_REPAIR_COSTS];
    let (hp_full, n) = match t.store {
        Store::Piece if (t.row as u16) < piece_have.min(pieces.piece_count) => {
            let def = pieces.pieces[t.row as usize];
            let n = (def.n_costs as usize).min(def.costs.len());
            rows[..n].copy_from_slice(&def.costs[..n]);
            (def.hp, n)
        }
        Store::Deploy if (t.row as u16) < deploy_have.min(deploys.def_count) => {
            let def = deploys.defs[t.row as usize];
            let n = (def.n_costs as usize).min(def.costs.len());
            rows[..n].copy_from_slice(&def.costs[..n]);
            (def.hp, n)
        }
        _ => return None,
    };
    if hp_full == 0 || t.hp == 0 || pieces.repair_pct == 0 {
        return None;
    }
    let mut quote = [(0u16, 0u32); MAX_REPAIR_COSTS];
    let n = repair_quote(&rows[..n], t.hp, hp_full, pieces.repair_pct, &mut quote);
    let mut out = [Cost {
        item: 0,
        units: 0,
        have: 0,
    }; MAX_REPAIR_COSTS];
    for (slot, &(item, units)) in out.iter_mut().zip(quote.iter()).take(n) {
        *slot = Cost {
            item,
            units,
            have: inv_count(inv, item),
        };
    }
    Some((out, n))
}

/// The repair readout's headline, over [`repair_rows`]' row count.
///
/// **A floor, and it says so.** Decay drains a structure's hp with no
/// event (a broadcast per decay step would be every unpaid wall in the
/// world, every period), so between one record and the next `StructHit`
/// or `PieceRepaired` the mirror's hp can stand above the store's. Never
/// below it: every edge that raises hp (a repair, an upgrade, a placement)
/// is sent. `repair_quote` never falls as the hp missing grows, so the
/// quoted rows are the bill or less, row by row, and the hp restored is
/// this or more. The line promises that much and no more, rather than an
/// exact price the server may raise.
pub fn repair_line(t: Option<&Target>, rows: Option<usize>) -> String {
    match (t, rows) {
        // `repair`'s own refusal for a row with no price.
        (_, Some(0)) => "cannot be repaired".into(),
        (Some(t), Some(_)) => format!(
            "Restores {}+ HP to {} · costs at least",
            t.hp_max.saturating_sub(t.hp),
            t.hp_max
        ),
        _ => "Waiting for repair details".into(),
    }
}

/// Human-readable target identity, read from the same rows as the action.
pub fn target_name(
    near: Option<&Target>,
    pieces: &BuildContent,
    piece_have: u16,
    deploys: &DeployContent,
    deploy_have: u16,
    catalog: &protocol::ItemCatalog,
) -> String {
    let Some(t) = near else {
        return "Nothing in reach".into();
    };
    match t.store {
        Store::Piece if (t.row as u16) < piece_have.min(pieces.piece_count) => {
            let def = pieces.pieces[t.row as usize];
            format!(
                "{} {}",
                super::build::material_label(def.material),
                super::build::shape_label(def.shape)
            )
        }
        Store::Deploy if (t.row as u16) < deploy_have.min(deploys.def_count) => {
            super::craft::item_label(catalog, deploys.defs[t.row as usize].item)
        }
        _ => "Waiting for building details".into(),
    }
}
