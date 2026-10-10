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
    DEATH_BY_ARROW, DEATH_BY_BULLET, DEATH_BY_CHARGE, DEATH_BY_CLOCK, DEATH_BY_COLD, DEATH_BY_HAND,
    DEATH_BY_MOB, DEATH_BY_SALT,
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
///
/// `killer` is what the killer is called (`ui::names::label`) — a player's
/// name, else their short address, else `#id`.
pub fn sentence(d: &Death, catalog: &ItemCatalog, killer: &str) -> String {
    match d.cause {
        DEATH_BY_CLOCK => "you ran out".to_string(),
        DEATH_BY_SALT => "the sea is salt".to_string(),
        DEATH_BY_COLD => "the cold took you".to_string(),
        sim_core::world::DEATH_BY_FALL => "you fell too far".to_string(),
        DEATH_BY_HAND if d.killer == d.own_id => "you did it to yourself".to_string(),
        DEATH_BY_HAND => {
            let weapon = match item_name(catalog, d.item) {
                Some(n) => format!(" with {n}"),
                None => String::new(),
            };
            format!(
                "{} killed you{} from {:.1} m",
                killer,
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
                "{} shot you{} from {:.1} m",
                killer,
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
        DEATH_BY_MOB if mob::slot_of_id(d.killer).is_some_and(sim_core::turret::is_turret_slot) => {
            "an auto turret gunned you down".to_string()
        }
        DEATH_BY_MOB => match mob::slot_of_id(d.killer).map(mob::kind_of) {
            Some(mob::MOB_WOLF) => "a wolf ran you down".to_string(),
            Some(mob::MOB_STAG) => "a stag gored you".to_string(),
            Some(mob::MOB_HELI) => "the attack helicopter gunned you down".to_string(),
            Some(mob::MOB_SENTRY) => "THE GATE's sentry gunned you down".to_string(),
            _ => "a pig gored you".to_string(),
        },
        // The blast's whole story is the distance, an arrow's rule — and
        // the planter may be the victim, which gets the sentence a
        // self-inflicted bomb has earned since bombs existed.
        DEATH_BY_CHARGE if d.killer == d.own_id => "you blew yourself up".to_string(),
        DEATH_BY_CHARGE => format!(
            "{}'s charge got you from {:.1} m",
            killer,
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
    /// THE GATE's respawn point (Rust's Outpost spawn point): unlocked by
    /// reaching the town, resting 30 minutes after each use.
    Gate,
    Beach,
}

/// The rows, in the reference's order: the anchor you'd rather have first.
/// `(wake, title, the small line under it)`.
pub const WAKES: [(Wake, &str, &str); 3] = [
    (
        Wake::Bag,
        "Wake on your bag",
        "the nearest one that is ready",
    ),
    (
        Wake::Gate,
        "Wake at THE GATE",
        "the town's market street - rests 30 minutes after",
    ),
    (Wake::Beach, "Wake on a beach", "the shoreline, somewhere"),
];

/// What the screen can offer: a bag you own, and THE GATE once you have
/// been there.
///
/// **Owning one, not one being ready.** A bag inside its cooldown is still
/// a bag you placed and still somewhere you might rather wake; the sim
/// picks the nearest ready one and falls back to the beach if none is, and
/// [`woke`] tells the player which answered. Hiding the row on a cooldown
/// would take the choice away for five minutes over a fact the client
/// learned at the moment of death and cannot refresh. THE GATE's row is the
/// same: its line says when it rests, and asking early is a beach.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Anchors {
    pub bag: bool,
    pub gate: bool,
}

impl Anchors {
    fn has(self, wake: Wake) -> bool {
        match wake {
            Wake::Bag => self.bag,
            Wake::Gate => self.gate,
            Wake::Beach => true,
        }
    }
}

/// The rows to draw, in order. The beach is LAST whatever else is offered,
/// so the digit that answers "just get me back in" is the last one.
pub fn rows(a: Anchors) -> impl Iterator<Item = &'static (Wake, &'static str, &'static str)> {
    WAKES.iter().filter(move |r| a.has(r.0))
}

/// The wake a **digit** selects — 1-based, over the rows actually drawn.
///
/// Position, not identity, which is the whole point: with no bag, `1` is
/// the first row on the screen, whatever it is. A player does not read a
/// table, they press the number next to the words.
pub fn wake_at(a: Anchors, n: usize) -> Option<Wake> {
    if n == 0 {
        return None;
    }
    rows(a).nth(n - 1).map(|r| r.0)
}

/// Whether a wake is on the screen at all.
pub fn offers(a: Anchors, wake: Wake) -> bool {
    rows(a).any(|r| r.0 == wake)
}

/// The footer line under the rows.
///
/// With nothing but the beach there is no choice to explain and one thing
/// worth saying instead — *why* there is only one row. A player who is not
/// told assumes the feature is broken, which is the same failure [`woke`]
/// exists to avoid one beat later.
pub fn note(a: Anchors) -> &'static str {
    if a.bag || a.gate {
        "click a row, or press its number"
    } else {
        "no bag placed - the shoreline is the only way back"
    }
}

/// THE GATE row's small line: when the point rests, or why it will not take
/// you. `left_ticks` is `ClientCore::gate_spawn_left`'s answer.
pub fn gate_line(left_ticks: u32, hostile: bool) -> String {
    if hostile {
        "not while you are hostile - this will be a beach".to_string()
    } else if left_ticks > 0 {
        let secs = left_ticks.div_ceil(sim_core::limits::TICK_HZ);
        format!(
            "resting {}:{:02} - this will be a beach",
            secs / 60,
            secs % 60
        )
    } else {
        WAKES[1].2.to_string()
    }
}

/// What the toast says once the wake lands.
///
/// `asked` is what the player pressed; `on_bag` and `in_town` are which
/// anchor actually answered — **asking for a bag inside its cooldown gets a
/// beach**, and so does THE GATE while it rests, and a player who is not
/// told that has no way to learn it except by looking around.
pub fn woke(asked: Wake, on_bag: bool, in_town: bool) -> Option<&'static str> {
    match (asked, on_bag, in_town) {
        (_, true, _) => Some("you woke on your bag"),
        (Wake::Gate, _, true) => Some("you woke at THE GATE"),
        (Wake::Bag, false, _) => Some("no bag ready - you woke on a beach"),
        (Wake::Gate, false, false) => Some("THE GATE would not take you - you woke on a beach"),
        (Wake::Beach, false, _) => None,
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
        assert_eq!(sentence(&d, &cat, &format!("#{}", d.killer)), "you ran out");
        let d = Death {
            cause: DEATH_BY_SALT,
            ..Death::default()
        };
        assert_eq!(
            sentence(&d, &cat, &format!("#{}", d.killer)),
            "the sea is salt"
        );
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
            sentence(&d, &cat, &format!("#{}", d.killer)),
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
        assert_eq!(
            sentence(&d, &cat, &format!("#{}", d.killer)),
            "#12 killed you from 1.0 m"
        );
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
        assert_eq!(
            sentence(&d, &cat, &format!("#{}", d.killer)),
            "you did it to yourself"
        );
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
                "",
            )
        };
        let wolf = (0..sim_core::limits::MAX_MOBS)
            .find(|&s| mob::kind_of(s) == mob::MOB_WOLF)
            .expect("the roster holds a predator");
        let pig = (0..sim_core::limits::MAX_MOBS)
            .find(|&s| mob::kind_of(s) == mob::MOB_PIG)
            .expect("the roster holds prey");
        let stag = (0..sim_core::limits::MAX_MOBS)
            .find(|&s| mob::kind_of(s) == mob::MOB_STAG)
            .expect("the roster holds a stag");
        assert_eq!(said(wolf), "a wolf ran you down");
        assert_eq!(said(pig), "a pig gored you");
        assert_eq!(said(stag), "a stag gored you");
        assert_eq!(
            said(mob::HELI_SLOT),
            "the attack helicopter gunned you down"
        );
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
        assert_eq!(
            sentence(&d, &cat, &format!("#{}", d.killer)),
            format!("killed by cause {unknown}")
        );
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
        assert_eq!(
            sentence(&d, &cat, &format!("#{}", d.killer)),
            "#7 shot you with BOW from 41.3 m"
        );
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
            let s = sentence(&d, &cat, &format!("#{}", d.killer));
            for bad in ["x=", "z=", "at (", "cell"] {
                assert!(!s.contains(bad), "cause {cause} leaked a position: {s}");
            }
        }
    }

    const NONE: Anchors = Anchors {
        bag: false,
        gate: false,
    };
    const BAG: Anchors = Anchors {
        bag: true,
        gate: false,
    };
    const GATE: Anchors = Anchors {
        bag: false,
        gate: true,
    };
    const BOTH: Anchors = Anchors {
        bag: true,
        gate: true,
    };

    /// **The item, in one assertion.** No bag placed and THE GATE never
    /// reached ⇒ one row, and it is the beach.
    #[test]
    fn a_player_with_no_anchor_is_offered_only_the_beach() {
        let r: Vec<_> = rows(NONE).collect();
        assert_eq!(r.len(), 1, "an anchorless death was offered a choice");
        assert_eq!(r[0].0, Wake::Beach);
        assert!(!offers(NONE, Wake::Bag) && !offers(NONE, Wake::Gate));
        assert_eq!(rows(BAG).count(), 2, "a bag owner lost the choice");
        assert!(offers(BAG, Wake::Bag) && !offers(BAG, Wake::Gate));
        assert!(offers(GATE, Wake::Gate) && !offers(GATE, Wake::Bag));
        assert_eq!(rows(BOTH).count(), 3);
    }

    /// The beach is the LAST row whatever else is offered.
    #[test]
    fn the_beach_is_the_last_row_either_way() {
        for a in [NONE, BAG, GATE, BOTH] {
            assert_eq!(rows(a).last().unwrap().0, Wake::Beach);
        }
    }

    /// A digit means the row it is drawn next to.
    #[test]
    fn a_digit_selects_by_position_and_not_by_identity() {
        assert_eq!(wake_at(BAG, 1), Some(Wake::Bag));
        assert_eq!(wake_at(BAG, 2), Some(Wake::Beach));
        assert_eq!(wake_at(NONE, 1), Some(Wake::Beach));
        assert_eq!(wake_at(GATE, 1), Some(Wake::Gate));
        assert_eq!(wake_at(BOTH, 2), Some(Wake::Gate));
        assert_eq!(wake_at(BOTH, 3), Some(Wake::Beach));
        // Past the drawn rows, and the 1-based zero, are both nothing —
        // never a wrap onto the other answer.
        assert_eq!(wake_at(NONE, 2), None);
        assert_eq!(wake_at(BOTH, 4), None);
        assert_eq!(wake_at(BAG, 0), None);
    }

    /// Every drawn row has a key that reaches it, in every shape.
    #[test]
    fn every_drawn_row_is_reachable_by_its_own_digit() {
        for a in [NONE, BAG, GATE, BOTH] {
            for (i, (wake, _, _)) in rows(a).enumerate() {
                assert_eq!(
                    wake_at(a, i + 1),
                    Some(*wake),
                    "row {i} of {a:?} has no digit"
                );
            }
        }
    }

    /// The anchorless footer says WHY there is one row.
    #[test]
    fn the_bagless_footer_explains_itself() {
        assert_ne!(note(NONE), note(BAG));
        assert_eq!(note(GATE), note(BAG));
        assert!(note(NONE).contains("bag"), "{}", note(NONE));
    }

    #[test]
    fn the_gate_row_says_when_it_rests() {
        assert_eq!(gate_line(0, false), WAKES[1].2);
        assert!(gate_line(90 * sim_core::limits::TICK_HZ, false).starts_with("resting 1:30"));
        assert!(gate_line(0, true).contains("hostile"));
    }

    #[test]
    fn the_wake_reports_the_anchor_that_answered() {
        assert_eq!(
            woke(Wake::Bag, false, false),
            Some("no bag ready - you woke on a beach")
        );
        assert_eq!(woke(Wake::Bag, true, false), Some("you woke on your bag"));
        assert_eq!(woke(Wake::Beach, false, false), None);
        assert_eq!(woke(Wake::Gate, false, true), Some("you woke at THE GATE"));
        assert!(woke(Wake::Gate, false, false).unwrap().contains("beach"));
    }
}
