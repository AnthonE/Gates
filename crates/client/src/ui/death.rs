//! What the death screen says.
//!
//! Pure and in `ui/` because it is a sentence assembled from five wire
//! fields, and a sentence assembled inside a Bevy system is one no headless
//! test can read back.
//!
//! **No position anywhere in it, and that is a rule rather than an
//! omission** (`ALPHA.md` §1, "who/what killed you — range and weapon, no
//! map position"): a screen that told you where you fell would hand the
//! raider standing over your body a pin to the base they just cleared. Who,
//! with what, from how far.

use protocol::event::ItemCatalog;
use sim_core::mob;
use sim_core::world::{
    DEATH_BY_ARROW, DEATH_BY_BULLET, DEATH_BY_CHARGE, DEATH_BY_CLOCK, DEATH_BY_HAND, DEATH_BY_MOB,
    DEATH_BY_SALT,
};

use super::craft::item_name;

/// Everything the screen needs, read straight off `ClientCore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Death {
    pub cause: u8,
    pub killer: u32,
    pub item: u16,
    pub range_cm: u16,
    /// This client's own id, so "you did it to yourself" is answerable.
    pub own_id: u32,
}

/// The one line under the title.
///
/// An unknown cause is reported as itself rather than folded into the
/// nearest sentence — `DEATH_BY_MAX`'s own doc records a judged FAIL where a
/// fourth cause would have shipped silently, so a client that quietly said
/// "you ran out" for cause 3 would be hiding exactly the bug that ledger
/// exists to expose.
pub fn sentence(d: &Death, catalog: &ItemCatalog) -> String {
    match d.cause {
        DEATH_BY_CLOCK => "you ran out".to_string(),
        DEATH_BY_SALT => "the sea is salt".to_string(),
        DEATH_BY_HAND if d.killer == d.own_id => "you did it to yourself".to_string(),
        DEATH_BY_HAND => {
            let weapon = match item_name(catalog, d.item) {
                Some(n) => format!(" with {n}"),
                None => String::new(),
            };
            format!(
                "#{} killed you{} from {:.1} m",
                d.killer,
                weapon,
                d.range_cm as f32 / 100.0
            )
        }
        // The bow gets its own verb rather than `DEATH_BY_HAND`'s, for the
        // reason `world.rs` gives for the cause existing at all: the range
        // is the whole story of a ranged kill, and "killed you from 41.3 m"
        // reads as a melee reach bug rather than as an archer.
        //
        // No self-kill arm. `ranged.rs` refuses an arrow against its own
        // shooter (`tests/shoot.rs: an_arrow_never_hits_its_owner`), so a
        // branch for it here would be unreachable code asserting a rule
        // that is already a wall one crate down.
        //
        // **Two causes, one sentence, and that is the point.** A bullet
        // shared `DEATH_BY_ARROW` outright from hitscan v0 to arrow
        // recovery v1, and the reason it went unnoticed for twenty-three
        // days is written right here: the sentence was already exact,
        // because the weapon is a wire field, so this reads "shot you with
        // the revolver from 12.4 m" and "shot you with the bow from
        // 34.0 m" off one branch either way. What was wrong was never the
        // words — it was the *cause code* underneath them, which is what
        // anything reading the cause rather than the sentence sees.
        //
        // So `DEATH_BY_BULLET` is named here rather than given an arm: the
        // fix upstream was to stop lying about which cause it was, not to
        // say something different to the player. Leaving it unnamed would
        // have been the actual regression — it falls to `other` and prints
        // "killed by cause 6".
        DEATH_BY_ARROW | DEATH_BY_BULLET => {
            let weapon = match item_name(catalog, d.item) {
                Some(n) => format!(" with {n}"),
                None => String::new(),
            };
            format!(
                "#{} shot you{} from {:.1} m",
                d.killer,
                weapon,
                d.range_cm as f32 / 100.0
            )
        }
        // The killer is a roster slot's tagged id, never a player number —
        // printing "#8388608" would be the wire's bookkeeping leaking into
        // a sentence. The species comes off that slot through `mob::kind_of`,
        // which is what the renderer picks the mesh with and what the sim
        // built the roster with: three readers, one pure function, and no
        // wire field. A death screen that named the wrong animal would be
        // the cheapest possible way to find out the three had drifted.
        DEATH_BY_MOB => match mob::slot_of_id(d.killer).map(mob::kind_of) {
            Some(mob::MOB_WOLF) => "a wolf ran you down".to_string(),
            _ => "a pig gored you".to_string(),
        },
        // The blast's whole story is the distance, an arrow's rule — and
        // the planter may be the victim, which gets the sentence a
        // self-inflicted bomb has earned since bombs existed.
        DEATH_BY_CHARGE if d.killer == d.own_id => "you blew yourself up".to_string(),
        DEATH_BY_CHARGE => format!(
            "#{}'s charge got you from {:.1} m",
            d.killer,
            d.range_cm as f32 / 100.0
        ),
        other => format!("killed by cause {other}"),
    }
}

// ---------------------------------------------------------------------------
// Which answers the screen offers (bag choice v0)
// ---------------------------------------------------------------------------
//
// **A row for an anchor you do not have is not a choice, it is a wrong
// button.** "Wake on your bag" was drawn unconditionally, and for a player
// who has never placed one it always resolved to a beach — the sim's
// deliberate kindness (`asking_for_a_bag_you_have_not_got_is_a_beach`),
// arriving as a screen that offered two doors into one room. The list is
// data now, and `ClientCore::own_bags()` is what decides its length.
//
// The order is fixed and the beach is LAST, which is what makes
// `rows(false)` a suffix of `rows(true)` rather than a second table: the
// digit that answers "just get me back in" is the last one either way.

/// Where you wake up.
///
/// `ui` rather than `render` because the whole of "which rows exist and
/// which key reaches them" is arithmetic that a headless test can read
/// back, and it was in a Bevy file where nothing could
/// (`crate::ui`'s standing rule).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wake {
    Bag,
    Beach,
}

/// The rows, in the reference's order: the anchor you'd rather have first.
/// `(wake, title, the small line under it)`.
pub const WAKES: [(Wake, &str, &str); 2] = [
    (
        Wake::Bag,
        "Wake on your bag",
        "the nearest one that is ready",
    ),
    (Wake::Beach, "Wake on a beach", "the shoreline, somewhere"),
];

/// The rows to draw. `has_bag` is `!ClientCore::own_bags().is_empty()` —
/// **owning one, not one being ready.** A bag inside its cooldown is still
/// a bag you placed and still somewhere you might rather wake; the sim
/// picks the nearest ready one and falls back to the beach if none is, and
/// [`woke`] tells the player which answered. Hiding the row on a cooldown
/// would take the choice away for five minutes over a fact the client
/// learned at the moment of death and cannot refresh.
pub fn rows(has_bag: bool) -> &'static [(Wake, &'static str, &'static str)] {
    if has_bag {
        &WAKES
    } else {
        &WAKES[1..]
    }
}

/// The wake a **digit** selects — 1-based, over the rows actually drawn.
///
/// Position, not identity, which is the whole point: with no bag, `1` is
/// the beach, because `1` is the first row on the screen. A player does
/// not read a table, they press the number next to the words.
pub fn wake_at(has_bag: bool, n: usize) -> Option<Wake> {
    if n == 0 {
        return None;
    }
    rows(has_bag).get(n - 1).map(|r| r.0)
}

/// Whether a wake is on the screen at all — the gate for the **letter**
/// aliases, which are bound to the anchor rather than to the position.
/// `F` must do nothing for a player with no bag, or the alias is a way to
/// press a button that was deliberately not drawn.
pub fn offers(has_bag: bool, wake: Wake) -> bool {
    rows(has_bag).iter().any(|r| r.0 == wake)
}

/// The footer line under the rows.
///
/// With no bag there is no choice to explain and one thing worth saying
/// instead — *why* there is only one row. A player who is not told assumes
/// the feature is broken, which is the same failure [`woke`] exists to
/// avoid one beat later.
pub fn note(has_bag: bool) -> &'static str {
    if has_bag {
        "click a row, or press its number"
    } else {
        "no bag placed - the shoreline is the only way back"
    }
}

/// What the toast says once the wake lands.
///
/// `asked_for_bag` is what the player pressed and `on_bag` is which anchor
/// actually answered — **asking for a bag inside its cooldown gets a beach**,
/// and a player who is not told that has no way to learn it except by
/// looking around.
pub fn woke(asked_for_bag: bool, on_bag: bool) -> Option<&'static str> {
    match (asked_for_bag, on_bag) {
        (true, false) => Some("no bag ready - you woke on a beach"),
        (_, true) => Some("you woke on your bag"),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_with(idx: usize, name: &str) -> ItemCatalog {
        let mut c = ItemCatalog::EMPTY;
        c.set(idx, name.as_bytes(), protocol::ItemRow::EMPTY)
            .unwrap();
        c.count = (idx + 1) as u16;
        c
    }

    #[test]
    fn the_world_kills_get_their_own_sentence() {
        let cat = ItemCatalog::EMPTY;
        let d = Death {
            cause: DEATH_BY_CLOCK,
            ..Death::default()
        };
        assert_eq!(sentence(&d, &cat), "you ran out");
        let d = Death {
            cause: DEATH_BY_SALT,
            ..Death::default()
        };
        assert_eq!(sentence(&d, &cat), "the sea is salt");
    }

    #[test]
    fn a_players_hand_names_who_what_and_how_far() {
        let cat = catalog_with(4, "STONE HATCHET");
        let d = Death {
            cause: DEATH_BY_HAND,
            killer: 12,
            item: 4,
            range_cm: 250,
            own_id: 7,
        };
        assert_eq!(
            sentence(&d, &cat),
            "#12 killed you with STONE HATCHET from 2.5 m"
        );
    }

    /// The catalog arrives in batches, so an unnamed weapon is a real state
    /// for the first frames of a session — the sentence drops the clause
    /// rather than printing an index at a moment like this one.
    #[test]
    fn an_unnamed_weapon_drops_the_clause() {
        let cat = ItemCatalog::EMPTY;
        let d = Death {
            cause: DEATH_BY_HAND,
            killer: 12,
            item: 4,
            range_cm: 100,
            own_id: 7,
        };
        assert_eq!(sentence(&d, &cat), "#12 killed you from 1.0 m");
    }

    #[test]
    fn your_own_id_is_your_own_fault() {
        let cat = ItemCatalog::EMPTY;
        let d = Death {
            cause: DEATH_BY_HAND,
            killer: 7,
            own_id: 7,
            ..Death::default()
        };
        assert_eq!(sentence(&d, &cat), "you did it to yourself");
    }

    /// **Each species gets its own sentence, off the roster slot.** The
    /// killer id is tagged (`MOB_ID_TAG`) and its low bits are the slot, so
    /// the same `mob::kind_of` the sim built the roster with and the
    /// renderer picks the mesh with also picks the verb here. Three readers
    /// of one pure function and no wire field between them — this is the
    /// cheapest of the three to check, so it is the one that reddens first
    /// if they ever drift.
    #[test]
    fn the_death_screen_names_the_animal_that_killed_you() {
        let cat = ItemCatalog::EMPTY;
        let said = |slot: usize| {
            sentence(
                &Death {
                    cause: DEATH_BY_MOB,
                    killer: mob::mob_id(slot),
                    ..Death::default()
                },
                &cat,
            )
        };
        let wolf = (0..sim_core::limits::MAX_MOBS)
            .find(|&s| mob::kind_of(s) == mob::MOB_WOLF)
            .expect("the roster holds a predator");
        let pig = (0..sim_core::limits::MAX_MOBS)
            .find(|&s| mob::kind_of(s) == mob::MOB_PIG)
            .expect("the roster holds prey");
        assert_eq!(said(wolf), "a wolf ran you down");
        assert_eq!(said(pig), "a pig gored you");
        assert_ne!(
            said(wolf),
            said(pig),
            "both species share one sentence — the screen is guessing"
        );
    }

    /// `DEATH_BY_MAX`'s doc records a judged FAIL where a fourth cause would
    /// have shipped silently. A client that folded an unknown cause into the
    /// nearest sentence would hide it here too.
    #[test]
    fn an_unknown_cause_says_so() {
        let cat = ItemCatalog::EMPTY;
        // One past `DEATH_BY_MAX`, not a literal: this test moves every time
        // a cause is added, which is the point of it.
        let unknown = sim_core::world::DEATH_BY_MAX + 1;
        let d = Death {
            cause: unknown,
            ..Death::default()
        };
        assert_eq!(sentence(&d, &cat), format!("killed by cause {unknown}"));
    }

    /// The bow's own sentence — the range is the story, so it must survive
    /// into the line the player reads.
    #[test]
    fn an_arrow_says_who_shot_and_how_far() {
        let cat = catalog_with(1, "BOW");
        let d = Death {
            cause: DEATH_BY_ARROW,
            killer: 7,
            own_id: 3,
            item: 1,
            range_cm: 4130,
        };
        assert_eq!(sentence(&d, &cat), "#7 shot you with BOW from 41.3 m");
    }

    /// No sentence may contain a coordinate. Asserted structurally rather
    /// than by eye, because `ALPHA.md` §1 is the kind of rule that gets
    /// broken by someone adding a helpful debug field.
    #[test]
    fn no_sentence_carries_a_position() {
        let cat = catalog_with(1, "ROCK");
        for cause in 0..=4u8 {
            let d = Death {
                cause,
                killer: 3,
                item: 1,
                range_cm: 512,
                own_id: 9,
            };
            let s = sentence(&d, &cat);
            for bad in ["x=", "z=", "at (", "cell"] {
                assert!(!s.contains(bad), "cause {cause} leaked a position: {s}");
            }
        }
    }

    /// **The item, in one assertion.** No bag placed ⇒ one row, and it is
    /// the beach.
    #[test]
    fn a_player_with_no_bag_is_offered_no_bag() {
        let r = rows(false);
        assert_eq!(r.len(), 1, "a bagless death was offered a choice");
        assert_eq!(r[0].0, Wake::Beach);
        assert!(!offers(false, Wake::Bag));
        assert_eq!(rows(true).len(), 2, "a bag owner lost the choice");
        assert!(offers(true, Wake::Bag) && offers(true, Wake::Beach));
    }

    /// The beach is the LAST row either way, so the bagless list is a
    /// suffix of the other. That is what keeps the two in one table
    /// instead of two that can disagree about wording.
    #[test]
    fn the_beach_is_the_last_row_either_way() {
        assert_eq!(rows(false), &WAKES[1..]);
        assert_eq!(rows(true).last().unwrap().0, Wake::Beach);
    }

    /// A digit means the row it is drawn next to. With no bag, `1` is the
    /// beach — the number the player can see, not the number the enum
    /// happens to have.
    #[test]
    fn a_digit_selects_by_position_and_not_by_identity() {
        assert_eq!(wake_at(true, 1), Some(Wake::Bag));
        assert_eq!(wake_at(true, 2), Some(Wake::Beach));
        assert_eq!(wake_at(false, 1), Some(Wake::Beach));
        // Past the drawn rows, and the 1-based zero, are both nothing —
        // never a wrap onto the other answer.
        assert_eq!(wake_at(false, 2), None);
        assert_eq!(wake_at(true, 3), None);
        assert_eq!(wake_at(true, 0), None);
        assert_eq!(wake_at(false, 0), None);
    }

    /// Every drawn row has a key that reaches it, in both shapes —
    /// `render::death`'s old assertion, moved here where it can be
    /// written against the list rather than against a length.
    #[test]
    fn every_drawn_row_is_reachable_by_its_own_digit() {
        for has_bag in [false, true] {
            for (i, (wake, _, _)) in rows(has_bag).iter().enumerate() {
                assert_eq!(
                    wake_at(has_bag, i + 1),
                    Some(*wake),
                    "row {i} of has_bag={has_bag} has no digit"
                );
            }
        }
    }

    /// The bagless footer says WHY there is one row. Silence there reads
    /// as a broken screen.
    #[test]
    fn the_bagless_footer_explains_itself() {
        assert_ne!(note(false), note(true));
        assert!(note(false).contains("bag"), "{}", note(false));
    }

    #[test]
    fn the_wake_reports_the_anchor_that_answered() {
        assert_eq!(
            woke(true, false),
            Some("no bag ready - you woke on a beach")
        );
        assert_eq!(woke(true, true), Some("you woke on your bag"));
        assert_eq!(woke(false, false), None);
    }
}
