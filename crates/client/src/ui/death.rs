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
    DEATH_BY_ARROW, DEATH_BY_CHARGE, DEATH_BY_CLOCK, DEATH_BY_HAND, DEATH_BY_MOB, DEATH_BY_SALT,
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
        DEATH_BY_ARROW => {
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
        c.set(idx, name.as_bytes()).unwrap();
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
