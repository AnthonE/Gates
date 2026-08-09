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
//! Four segments, each one an encoder a key already presses — zero new
//! wire. Upgrade is `U`'s `encode_action_upgrade`, repair is `R`'s
//! `encode_action_repair`, and demolish and pick-up are both `Backspace`'s
//! `encode_action_demolish`, because the sim makes them one command with a
//! store bit: a **piece** comes down inside its grace window and refunds
//! whole (`build::demolish`), a **deployable** comes up any time you may
//! build there and returns its item (`deploy::pick_up`). The wheel splits
//! what the key merges — a segment named PICK UP that could fell a wall
//! would be the positional-payload trap wearing a menu — so [`act`] filters
//! by store and says why instead of sending the other verb.
//!
//! **Rotate has no segment, deliberately.** A placed piece is
//! `{cx, cz, level, loc, row, hp, uh}` — no facing, nothing to turn — so a
//! rotate wedge would be a dead button teaching a verb the game does not
//! have (`NOW.md` §0p2 item 0 has the whole argument).
//!
//! Whether the grace window is still open is **not** asked here, for
//! `demolish_near`'s stated reason: it is arithmetic over a tick the client
//! does not hold, and the sim's refusal (`REFUSE_B_WINDOW`) already has a
//! sentence.

use sim_core::build::BuildContent;

use super::build::Rings;
use super::structure::{self, Store, Target};

/// The verbs on the ring. No rotate — see the module note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Upgrade,
    Repair,
    Demolish,
    PickUp,
}

/// The ring, clockwise from the top. Upgrade sits at the top as the verb a
/// base-builder reaches for most; **demolish is at the bottom** — the one
/// destructive segment goes where a flick from centre is longest, the same
/// reasoning that put the keyboard verb on `Backspace`.
pub const VERBS: [Verb; 4] = [Verb::Upgrade, Verb::Repair, Verb::Demolish, Verb::PickUp];

pub fn label(v: Verb) -> &'static str {
    match v {
        Verb::Upgrade => "Upgrade",
        Verb::Repair => "Repair",
        Verb::Demolish => "Demolish",
        Verb::PickUp => "Pick Up",
    }
}

/// The centre's one-liner, the shape wheel's `shape_blurb` for verbs.
pub fn blurb(v: Verb) -> &'static str {
    match v {
        Verb::Upgrade => "Take the piece one rung up the material ladder.",
        Verb::Repair => "Mend what stands here.",
        Verb::Demolish => "Take a fresh piece back down for a full refund.",
        Verb::PickUp => "Lift a deployable back into your bag.",
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
