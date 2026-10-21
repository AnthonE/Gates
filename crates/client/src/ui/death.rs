//! What the death screen says, and which buttons it offers.
//!
//! Pure and in `ui/` because every string on it is assembled from wire
//! fields, and a string assembled inside a Bevy system is one no headless
//! test can read back.
//!
//! **The layout is Rust's** (operator, 2026-10-15, with the reference frame
//! pasted): a strip across the top — YOU DIED, then ALIVE FOR, KILLED BY,
//! WITH A, AT A DISTANCE OF and IN — the island in the middle with a skull
//! where you fell and your bags on it, and a bar along the bottom with one
//! button per bag, THE GATE once you have reached it, and RANDOM RESPAWN.
//! This file is the words and the order; `render/death.rs` draws them.
//!
//! **The map marks where you fell now.** This screen used to refuse any
//! position (`ALPHA.md` §1, "no map position"); the reference screen the
//! operator asked for has the skull pin, and only the body that fell ever
//! sees it.

use protocol::event::ItemCatalog;
use sim_core::deploy::{BagAnchor, DeployContent, DeployRec, ARCH_BAG, BAG_CAP};
use sim_core::mob;
use sim_core::world::{
    DEATH_BY_ARROW, DEATH_BY_BULLET, DEATH_BY_CACTUS, DEATH_BY_CHARGE, DEATH_BY_CLOCK,
    DEATH_BY_COLD, DEATH_BY_FALL, DEATH_BY_FEL, DEATH_BY_HAND, DEATH_BY_HEAT, DEATH_BY_MOB,
    DEATH_BY_SALT, DEATH_BY_SPIKES,
};

use super::craft::item_name;

/// Everything the screen needs about the death, read straight off
/// `ClientCore`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Death {
    pub cause: u8,
    pub killer: u32,
    pub item: u16,
    pub range_cm: u16,
    /// This client's own id, so "you did it to yourself" is answerable.
    pub own_id: u32,
    /// How long the body had been standing, in seconds (wire v115). `None`
    /// when the server did not time that life (a corpse restored from disk),
    /// and then the strip has no ALIVE FOR.
    pub alive_s: Option<u32>,
}

impl Death {
    /// Whether the killer is the victim.
    fn own_doing(&self) -> bool {
        self.killer == self.own_id
    }
}

/// The death as one line. The report composer's prefill
/// ([`report_title`]) is built from it.
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
        DEATH_BY_HEAT => "the heat took you".to_string(),
        DEATH_BY_FEL => "the verdigris took you".to_string(),
        DEATH_BY_FALL => "you fell too far".to_string(),
        DEATH_BY_CACTUS => "you kept walking into a cactus".to_string(),
        DEATH_BY_SPIKES => "you stayed on the spikes".to_string(),
        DEATH_BY_HAND if d.own_doing() => "you did it to yourself".to_string(),
        DEATH_BY_HAND => {
            let weapon = match item_name(catalog, d.item) {
                Some(n) => format!(" with {n}"),
                None if d.item == sim_core::gather::NO_ITEM => " with their fists".to_string(),
                None => String::new(),
            };
            format!(
                "{} killed you{} from {:.1} m",
                killer,
                weapon,
                d.range_cm as f32 / 100.0
            )
        }
        // The bow gets its own verb, for the reason `world.rs` gives for
        // the cause existing at all: the range is the whole story of a
        // ranged kill, and "killed you from 41.3 m" reads as a melee reach
        // bug rather than as an archer. A bullet shares the sentence — the
        // weapon is a wire field, so "shot you with the revolver" and "shot
        // you with the bow" come off one branch. No self-kill arm:
        // `ranged.rs` refuses an arrow against its own shooter.
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
        DEATH_BY_MOB => match beast(d.killer) {
            Some(b) => b.sentence.to_string(),
            None => "something gored you".to_string(),
        },
        // The blast's whole story is the distance, an arrow's rule — and
        // the planter may be the victim.
        DEATH_BY_CHARGE if d.own_doing() => "you blew yourself up".to_string(),
        DEATH_BY_CHARGE => format!(
            "{}'s charge got you from {:.1} m",
            killer,
            d.range_cm as f32 / 100.0
        ),
        other => format!("killed by cause {other}"),
    }
}

/// A killer that is not a player: its KILLED BY name and its sentence.
struct Beast {
    name: &'static str,
    sentence: &'static str,
}

/// What a mob killer is, off its roster slot.
///
/// The killer is a roster slot's tagged id, never a player number —
/// printing "#8388608" would be the wire's bookkeeping leaking into a
/// sentence. The species comes off that slot through `mob::kind_of`, which
/// is what the renderer picks the mesh with and what the sim built the
/// roster with: three readers, one pure function, and no wire field.
fn beast(killer: u32) -> Option<Beast> {
    let slot = mob::slot_of_id(killer)?;
    if sim_core::turret::is_turret_slot(slot) {
        return Some(Beast {
            name: "Auto turret",
            sentence: "an auto turret gunned you down",
        });
    }
    Some(match mob::kind_of(slot) {
        mob::MOB_WOLF => Beast {
            name: "Wolf",
            sentence: "a wolf ran you down",
        },
        mob::MOB_STAG => Beast {
            name: "Stag",
            sentence: "a stag gored you",
        },
        mob::MOB_HELI => Beast {
            name: "Attack helicopter",
            sentence: "the attack helicopter gunned you down",
        },
        mob::MOB_SENTRY => Beast {
            name: "THE GATE's sentry",
            sentence: "THE GATE's sentry gunned you down",
        },
        _ => Beast {
            name: "Pig",
            sentence: "a pig gored you",
        },
    })
}

// ---------------------------------------------------------------------------
// The strip across the top
// ---------------------------------------------------------------------------

/// How a fact's chip is coloured: green for the life you had, red for what
/// ended it, grey for the rest — the reference's three.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Life,
    Foe,
    Plain,
}

/// What a fact is, so the render layer can hang a picture on the two that
/// carry one (the killer's face, the weapon's icon).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FactKind {
    Alive,
    Killer,
    Weapon,
    Distance,
    Square,
}

/// One field of the strip: the small caption over a coloured chip.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fact {
    pub kind: FactKind,
    pub caption: &'static str,
    pub value: String,
    pub tone: Tone,
}

/// The last meters the server stated, for a death by the survival clock:
/// which of them ran out is what KILLED BY names.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Meters {
    pub food: u16,
    pub water: u16,
}

/// "47 seconds", "14 minutes", "2 hours", "3 days": the largest whole unit,
/// as the reference prints it.
pub fn alive_for(secs: u32) -> String {
    let (n, unit) = match secs {
        0..60 => (secs, "second"),
        60..3_600 => (secs / 60, "minute"),
        3_600..86_400 => (secs / 3_600, "hour"),
        _ => (secs / 86_400, "day"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// What the KILLED BY chip says: the player, the animal, or the thing that
/// is not anybody (Rust's "Cold", "Fall", "Hunger").
pub fn killed_by(d: &Death, killer: &str, meters: Meters) -> String {
    match d.cause {
        DEATH_BY_CLOCK => match (meters.food == 0, meters.water == 0) {
            (true, true) => "Hunger and thirst".to_string(),
            (false, true) => "Thirst".to_string(),
            _ => "Hunger".to_string(),
        },
        DEATH_BY_SALT => "Salt water".to_string(),
        DEATH_BY_COLD => "Cold".to_string(),
        DEATH_BY_HEAT => "Heat".to_string(),
        DEATH_BY_FALL => "Fall".to_string(),
        DEATH_BY_CACTUS => "A cactus".to_string(),
        DEATH_BY_SPIKES => "Spikes".to_string(),
        DEATH_BY_FEL => "The Verdigris".to_string(),
        DEATH_BY_MOB => beast(d.killer).map_or("An animal", |b| b.name).to_string(),
        DEATH_BY_HAND | DEATH_BY_CHARGE if d.own_doing() => "Yourself".to_string(),
        DEATH_BY_HAND | DEATH_BY_ARROW | DEATH_BY_BULLET | DEATH_BY_CHARGE => killer.to_string(),
        other => format!("Cause {other}"),
    }
}

/// Whether the killer is a player other than you — the deaths that get a
/// face, a weapon and a REPORT CHEAT button.
pub fn by_another_player(d: &Death) -> bool {
    matches!(
        d.cause,
        DEATH_BY_HAND | DEATH_BY_ARROW | DEATH_BY_BULLET | DEATH_BY_CHARGE
    ) && !d.own_doing()
        && d.killer != 0
        && mob::slot_of_id(d.killer).is_none()
}

/// The WITH A chip: the weapon in the killer's hand, when there was one and
/// its name has arrived. A charge carries no item (the bomb went with the
/// plant), so it has no chip rather than a guessed one.
pub fn weapon<'a>(d: &Death, catalog: &'a ItemCatalog) -> Option<&'a str> {
    if !matches!(d.cause, DEATH_BY_HAND | DEATH_BY_ARROW | DEATH_BY_BULLET) || d.own_doing() {
        return None;
    }
    item_name(catalog, d.item)
}

/// "356 meters", or "2.5 meters" up close where the tenth is the story.
/// Integer arithmetic, rounded once: 9.99 m is "10 meters", never "10.0".
pub fn meters(range_cm: u16) -> String {
    let tenths = (range_cm as u32 + 5) / 10;
    if tenths < 100 {
        format!("{}.{} meters", tenths / 10, tenths % 10)
    } else {
        format!("{} meters", (range_cm as u32 + 50) / 100)
    }
}

/// The AT A DISTANCE OF chip, for a death something did from somewhere.
pub fn distance(d: &Death) -> Option<String> {
    let ranged = matches!(
        d.cause,
        DEATH_BY_HAND | DEATH_BY_ARROW | DEATH_BY_BULLET | DEATH_BY_CHARGE | DEATH_BY_MOB
    );
    (ranged && d.range_cm > 0 && !(d.cause == DEATH_BY_HAND && d.own_doing()))
        .then(|| meters(d.range_cm))
}

/// The strip, in the reference's order. `square` is the grid square the
/// body fell in (`ui::map::grid_label`), empty off the island.
pub fn facts(
    d: &Death,
    catalog: &ItemCatalog,
    killer: &str,
    meters: Meters,
    square: &str,
) -> Vec<Fact> {
    let mut out = Vec::new();
    if let Some(secs) = d.alive_s {
        out.push(Fact {
            kind: FactKind::Alive,
            caption: "ALIVE FOR",
            value: alive_for(secs),
            tone: Tone::Life,
        });
    }
    out.push(Fact {
        kind: FactKind::Killer,
        caption: "KILLED BY",
        value: killed_by(d, killer, meters),
        tone: Tone::Foe,
    });
    if let Some(w) = weapon(d, catalog) {
        out.push(Fact {
            kind: FactKind::Weapon,
            caption: "WITH A",
            value: w.to_string(),
            tone: Tone::Foe,
        });
    }
    if let Some(m) = distance(d) {
        out.push(Fact {
            kind: FactKind::Distance,
            caption: "AT A DISTANCE OF",
            value: m,
            tone: Tone::Plain,
        });
    }
    if !square.is_empty() {
        out.push(Fact {
            kind: FactKind::Square,
            caption: "IN",
            value: square.to_string(),
            tone: Tone::Plain,
        });
    }
    out
}

/// What REPORT CHEAT puts in the report composer for the player to finish:
/// who, with what, from how far, and where — the evidence a reader needs
/// first, with the frame of this screen attached by the composer itself.
pub fn report_title(d: &Death, catalog: &ItemCatalog, killer: &str, square: &str) -> String {
    let mut s = format!("cheat? {}", sentence(d, catalog, killer));
    if !square.is_empty() {
        s.push_str(" in ");
        s.push_str(square);
    }
    s.push_str(" - ");
    s.chars().take(crate::report::TITLE_MAX).collect()
}

// ---------------------------------------------------------------------------
// The bar along the bottom
// ---------------------------------------------------------------------------

/// Where you wake up: one button on the bottom bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wake {
    /// Your own bag, by its place in `ClientCore::own_bags`.
    Bag(usize),
    /// THE GATE's respawn point (Rust's Outpost spawn point): unlocked by
    /// reaching the town, resting 30 minutes after each use.
    Gate,
    /// RANDOM RESPAWN: the beach ring.
    Beach,
}

/// The buttons, left to right: every bag you own, THE GATE once reached,
/// and RANDOM RESPAWN last, so every digit is a button's place on the bar
/// and the beach's is always the last one.
pub fn wakes(bags: usize, gate: bool) -> impl Iterator<Item = Wake> {
    (0..bags.min(BAG_CAP))
        .map(Wake::Bag)
        .chain(gate.then_some(Wake::Gate))
        .chain(std::iter::once(Wake::Beach))
}

/// The wake a **digit** selects — 1-based, over the buttons actually drawn
/// (`0` is the tenth). Position, not identity: a player presses the number
/// on the button, not a number they remember from a table.
pub fn wake_at(bags: usize, gate: bool, n: usize) -> Option<Wake> {
    if n == 0 {
        return None;
    }
    wakes(bags, gate).nth(n - 1)
}

/// The digit printed on button `n` (1-based): `1`…`9`, then `0`.
pub fn digit(n: usize) -> Option<char> {
    match n {
        1..=9 => char::from_digit(n as u32, 10),
        10 => Some('0'),
        _ => None,
    }
}

/// `4:32`, or `1:02:03` past the hour.
pub fn clock(secs: u32) -> String {
    if secs >= 3_600 {
        format!("{}:{:02}:{:02}", secs / 3_600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// Whether a button can be pressed now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    Now,
    /// Cooling down (a bag) or resting (THE GATE), seconds left.
    In(u32),
    /// THE GATE will not take a hostile player, seconds of hostility left.
    Hostile(u32),
}

/// THE GATE's state from `ClientCore::gate_spawn_left` and `hostile_left`,
/// both in ticks.
pub fn gate_ready(left_ticks: u32, hostile_ticks: u32) -> Ready {
    let secs = |t: u32| t.div_ceil(sim_core::limits::TICK_HZ);
    if hostile_ticks > 0 {
        Ready::Hostile(secs(hostile_ticks))
    } else if left_ticks > 0 {
        Ready::In(secs(left_ticks))
    } else {
        Ready::Now
    }
}

/// The small line under a button's name: the grid square, or why it will
/// not take you yet.
pub fn wake_line(square: &str, ready: Ready) -> String {
    match ready {
        Ready::Now => square.to_string(),
        Ready::In(s) => clock(s),
        Ready::Hostile(s) => format!("hostile {}", clock(s)),
    }
}

/// A bag's name on its button: the deployable it is (`SLEEPING BAG`), off
/// the deploy mirror's record at its address, and the bag's own word when
/// the mirror or the catalog has not said yet. Rust prints the name the
/// owner gave it; ours have none, and the grid square under it is what
/// tells two of them apart.
pub fn bag_title(
    b: &BagAnchor,
    deploys: &[DeployRec],
    defs: &DeployContent,
    have: u16,
    catalog: &ItemCatalog,
) -> String {
    let have = have.min(defs.def_count);
    deploys
        .iter()
        .find(|r| r.cx == b.cx && r.cz == b.cz && r.level == b.level && (r.row as u16) < have)
        .map(|r| &defs.defs[r.row as usize])
        .filter(|def| def.arch == ARCH_BAG)
        .and_then(|def| item_name(catalog, def.item))
        .map_or_else(|| "SLEEPING BAG".to_string(), str::to_uppercase)
}

/// What the bar says when there is nothing on it but RANDOM RESPAWN: why.
/// A player who is not told assumes the feature is broken.
pub const NO_BAGS: &str = "no sleeping bag placed - place one and you can wake on it";

/// What the toast says once the wake lands.
///
/// `asked` is what the player pressed; `on_bag` and `in_town` are which
/// anchor actually answered — **asking for a bag that went cold since the
/// list was sent gets a beach**, and so does THE GATE while it rests, and a
/// player who is not told that has no way to learn it except by looking
/// around.
pub fn woke(asked: Wake, on_bag: bool, in_town: bool) -> Option<&'static str> {
    match (asked, on_bag, in_town) {
        (_, true, _) => Some("you woke on your bag"),
        (Wake::Gate, _, true) => Some("you woke at THE GATE"),
        (Wake::Bag(_), false, _) => Some("that bag was not ready - you woke on a beach"),
        (Wake::Gate, false, false) => Some("THE GATE would not take you - you woke on a beach"),
        (Wake::Beach, false, _) => None,
    }
}

// ---------------------------------------------------------------------------
// The marker on the compass (Rust's death marker)
// ---------------------------------------------------------------------------

/// How far either side of the bearing the compass reaches, degrees: the
/// tape's numbers end here and a mark this far round sits at the row's end.
/// Rust's tape shows about this much (`render::hud`'s compass).
pub const COMPASS_HALF_SPAN_DEG: f32 = 60.0;

/// Where a world point sits on the compass's marker row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompassMark {
    /// Across the row: -1 at the left end, 0 dead ahead, 1 at the right end.
    pub across: f32,
    /// Further round than the row reaches (behind you, mostly): the mark is
    /// held at the end it is nearer, which is the way to turn.
    pub off_row: bool,
    /// How far away, metres, on the ground.
    pub dist_m: f32,
}

/// Where `to` sits on the compass seen from `from` (world XZ) facing
/// `facing_deg`, a bearing in `crate::look::bearing_deg`'s degrees — the
/// same function the bearing number above it is printed from, so the mark
/// and the number cannot disagree about which way is which. Clockwise of
/// your heading is to the right, as on any compass strip.
pub fn compass_mark(facing_deg: f32, from: (f32, f32), to: (f32, f32)) -> CompassMark {
    let (dx, dz) = (to.0 - from.0, to.1 - from.1);
    let bearing = crate::look::bearing_of(dx, dz);
    let rel = (bearing - facing_deg + 540.0).rem_euclid(360.0) - 180.0;
    let across = rel / COMPASS_HALF_SPAN_DEG;
    CompassMark {
        across: across.clamp(-1.0, 1.0),
        off_row: across.abs() > 1.0,
        dist_m: (dx * dx + dz * dz).sqrt(),
    }
}

/// The compass's short distance: `143m`, `1.2km`.
pub fn short_distance(m: f32) -> String {
    if m >= 1000.0 {
        format!("{:.1}km", m / 1000.0)
    } else {
        format!("{m:.0}m")
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
        assert_eq!(sentence(&d, &cat, "#0"), "you ran out");
        let d = Death {
            cause: DEATH_BY_SALT,
            ..Death::default()
        };
        assert_eq!(sentence(&d, &cat, "#0"), "the sea is salt");
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
            alive_s: Some(0),
        };
        assert_eq!(
            sentence(&d, &cat, "#12"),
            "#12 killed you with STONE HATCHET from 2.5 m"
        );
    }

    /// The catalog arrives in batches, so an unnamed weapon is a real state
    /// for the first frames of a session — the sentence drops the clause and
    /// the strip drops the chip, rather than printing an index.
    #[test]
    fn an_unnamed_weapon_drops_the_clause() {
        let cat = ItemCatalog::EMPTY;
        let d = Death {
            cause: DEATH_BY_HAND,
            killer: 12,
            item: 4,
            range_cm: 100,
            own_id: 7,
            alive_s: Some(0),
        };
        assert_eq!(sentence(&d, &cat, "#12"), "#12 killed you from 1.0 m");
        assert_eq!(weapon(&d, &cat), None);
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
        assert_eq!(sentence(&d, &cat, "#7"), "you did it to yourself");
        assert_eq!(killed_by(&d, "#7", Meters::default()), "Yourself");
        assert!(!by_another_player(&d), "nobody to report");
    }

    /// **Each species gets its own words, off the roster slot**, and the
    /// strip and the sentence read the same table.
    #[test]
    fn the_death_screen_names_the_animal_that_killed_you() {
        let cat = ItemCatalog::EMPTY;
        let of = |slot: usize| Death {
            cause: DEATH_BY_MOB,
            killer: mob::mob_id(slot),
            ..Death::default()
        };
        let find = |kind| {
            (0..sim_core::limits::MAX_MOBS)
                .find(|&s| mob::kind_of(s) == kind)
                .expect("the roster holds the kind")
        };
        let (wolf, pig, stag) = (find(mob::MOB_WOLF), find(mob::MOB_PIG), find(mob::MOB_STAG));
        assert_eq!(sentence(&of(wolf), &cat, ""), "a wolf ran you down");
        assert_eq!(sentence(&of(pig), &cat, ""), "a pig gored you");
        assert_eq!(sentence(&of(stag), &cat, ""), "a stag gored you");
        assert_eq!(
            sentence(&of(mob::HELI_SLOT), &cat, ""),
            "the attack helicopter gunned you down"
        );
        assert_eq!(killed_by(&of(wolf), "", Meters::default()), "Wolf");
        assert_eq!(killed_by(&of(pig), "", Meters::default()), "Pig");
        assert_eq!(
            killed_by(&of(mob::HELI_SLOT), "", Meters::default()),
            "Attack helicopter"
        );
        assert!(!by_another_player(&of(wolf)), "a wolf is not reportable");
    }

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
            sentence(&d, &cat, "#0"),
            format!("killed by cause {unknown}")
        );
        assert_eq!(
            killed_by(&d, "#0", Meters::default()),
            format!("Cause {unknown}")
        );
    }

    #[test]
    fn every_cause_has_a_name_on_the_chip() {
        // A cause added on one branch and a chip table on another met in a
        // merge once, and the Verdigris read "Cause 10".
        for cause in 0..=sim_core::world::DEATH_BY_MAX {
            let d = Death {
                cause,
                ..Death::default()
            };
            let chip = killed_by(&d, "#0", Meters::default());
            assert!(!chip.starts_with("Cause"), "cause {cause} reads {chip:?}");
        }
    }

    #[test]
    fn an_arrow_says_who_shot_and_how_far() {
        let cat = catalog_with(1, "BOW");
        let d = Death {
            cause: DEATH_BY_ARROW,
            killer: 7,
            own_id: 3,
            item: 1,
            range_cm: 4130,
            alive_s: Some(0),
        };
        assert_eq!(sentence(&d, &cat, "#7"), "#7 shot you with BOW from 41.3 m");
    }

    #[test]
    fn alive_for_prints_the_largest_whole_unit() {
        assert_eq!(alive_for(0), "0 seconds");
        assert_eq!(alive_for(1), "1 second");
        assert_eq!(alive_for(59), "59 seconds");
        assert_eq!(alive_for(60), "1 minute");
        assert_eq!(alive_for(14 * 60 + 59), "14 minutes");
        assert_eq!(alive_for(2 * 3_600 + 5), "2 hours");
        assert_eq!(alive_for(3 * 86_400 + 7_200), "3 days");
        assert_eq!(alive_for(86_400), "1 day");
    }

    /// The reference's strip, in its order, for the reference's death: a
    /// player with a rifle from across a valley.
    #[test]
    fn a_shot_from_far_fills_every_chip_in_order() {
        let cat = catalog_with(9, "Semi-Automatic Rifle");
        let d = Death {
            cause: DEATH_BY_BULLET,
            killer: 40,
            item: 9,
            range_cm: 35_600,
            own_id: 3,
            alive_s: Some(3 * 86_400 + 600),
        };
        let f = facts(&d, &cat, "Garry", Meters::default(), "X3");
        let got: Vec<_> = f.iter().map(|f| (f.caption, f.value.as_str())).collect();
        assert_eq!(
            got,
            [
                ("ALIVE FOR", "3 days"),
                ("KILLED BY", "Garry"),
                ("WITH A", "Semi-Automatic Rifle"),
                ("AT A DISTANCE OF", "356 meters"),
                ("IN", "X3"),
            ]
        );
        assert_eq!(f[0].tone, Tone::Life);
        assert_eq!(f[1].tone, Tone::Foe);
        assert_eq!(f[3].tone, Tone::Plain);
        assert!(by_another_player(&d));
    }

    /// A death nobody did has no weapon and no distance — and no report.
    #[test]
    fn the_cold_has_no_weapon_and_no_distance() {
        let cat = catalog_with(1, "BOW");
        let d = Death {
            cause: DEATH_BY_COLD,
            killer: 3,
            item: 1,
            range_cm: 500,
            own_id: 3,
            alive_s: Some(90),
        };
        let f = facts(&d, &cat, "#3", Meters::default(), "");
        let kinds: Vec<_> = f.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [FactKind::Alive, FactKind::Killer]);
        assert_eq!(f[1].value, "Cold");
        assert!(!by_another_player(&d));
    }

    /// A corpse restored from disk comes back without its life's length
    /// (wire v116): the strip leaves ALIVE FOR out rather than say "0".
    #[test]
    fn an_untimed_life_has_no_alive_for() {
        let d = Death {
            cause: DEATH_BY_COLD,
            killer: 3,
            own_id: 3,
            alive_s: None,
            ..Death::default()
        };
        let f = facts(&d, &ItemCatalog::default(), "#3", Meters::default(), "");
        let kinds: Vec<_> = f.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [FactKind::Killer]);
    }

    #[test]
    fn the_clock_names_the_meter_that_ran_out() {
        let d = Death {
            cause: DEATH_BY_CLOCK,
            ..Death::default()
        };
        let m = |food, water| killed_by(&d, "", Meters { food, water });
        assert_eq!(m(0, 40), "Hunger");
        assert_eq!(m(40, 0), "Thirst");
        assert_eq!(m(0, 0), "Hunger and thirst");
    }

    #[test]
    fn meters_keep_the_tenth_only_up_close() {
        assert_eq!(meters(250), "2.5 meters");
        assert_eq!(meters(40), "0.4 meters");
        assert_eq!(meters(999), "10 meters");
        assert_eq!(meters(1_000), "10 meters");
        assert_eq!(meters(35_649), "356 meters");
    }

    #[test]
    fn the_report_names_the_killer_and_fits_the_title() {
        let cat = catalog_with(9, "REVOLVER");
        let d = Death {
            cause: DEATH_BY_BULLET,
            killer: 40,
            item: 9,
            range_cm: 1_240,
            own_id: 3,
            alive_s: Some(0),
        };
        let t = report_title(&d, &cat, "Garry", "H7");
        assert_eq!(
            t,
            "cheat? Garry shot you with REVOLVER from 12.4 m in H7 - "
        );
        let long = "x".repeat(400);
        assert!(report_title(&d, &cat, &long, "H7").chars().count() <= crate::report::TITLE_MAX);
    }

    #[test]
    fn the_beach_is_the_last_button_whatever_else_is_offered() {
        for bags in 0..=BAG_CAP {
            for gate in [false, true] {
                let all: Vec<_> = wakes(bags, gate).collect();
                assert_eq!(all.len(), bags + gate as usize + 1);
                assert_eq!(*all.last().unwrap(), Wake::Beach);
                assert!(
                    all.len() <= 10,
                    "more buttons than digits: {bags} bags, gate {gate}"
                );
            }
        }
        // Past the cap the list is the cap's: the wire never carries more.
        assert_eq!(wakes(BAG_CAP + 3, false).count(), BAG_CAP + 1);
    }

    /// A digit means the button it is printed on.
    #[test]
    fn a_digit_selects_by_position_and_not_by_identity() {
        assert_eq!(wake_at(0, false, 1), Some(Wake::Beach));
        assert_eq!(wake_at(2, false, 1), Some(Wake::Bag(0)));
        assert_eq!(wake_at(2, false, 2), Some(Wake::Bag(1)));
        assert_eq!(wake_at(2, false, 3), Some(Wake::Beach));
        assert_eq!(wake_at(1, true, 2), Some(Wake::Gate));
        assert_eq!(wake_at(0, true, 1), Some(Wake::Gate));
        assert_eq!(wake_at(0, true, 2), Some(Wake::Beach));
        // Past the drawn buttons, and the 1-based zero, are nothing.
        assert_eq!(wake_at(0, false, 2), None);
        assert_eq!(wake_at(1, false, 0), None);
        // Every drawn button has a digit, and the digit reaches it.
        for bags in 0..=BAG_CAP {
            for (i, w) in wakes(bags, true).enumerate() {
                assert!(digit(i + 1).is_some(), "button {i} has no digit");
                assert_eq!(wake_at(bags, true, i + 1), Some(w));
            }
        }
        assert_eq!(digit(10), Some('0'));
        assert_eq!(digit(11), None);
    }

    #[test]
    fn a_cooling_button_counts_down_and_a_ready_one_says_where() {
        assert_eq!(clock(272), "4:32");
        assert_eq!(clock(5), "0:05");
        assert_eq!(clock(3_723), "1:02:03");
        assert_eq!(wake_line("H7", Ready::Now), "H7");
        assert_eq!(wake_line("H7", Ready::In(90)), "1:30");
        let hz = sim_core::limits::TICK_HZ;
        assert_eq!(gate_ready(0, 0), Ready::Now);
        assert_eq!(gate_ready(90 * hz, 0), Ready::In(90));
        assert_eq!(gate_ready(0, 10 * hz), Ready::Hostile(10));
        assert!(wake_line("", gate_ready(0, 10 * hz)).starts_with("hostile"));
    }

    #[test]
    fn the_wake_reports_the_anchor_that_answered() {
        assert!(woke(Wake::Bag(0), false, false).unwrap().contains("beach"));
        assert_eq!(
            woke(Wake::Bag(1), true, false),
            Some("you woke on your bag")
        );
        assert_eq!(woke(Wake::Beach, false, false), None);
        assert_eq!(woke(Wake::Gate, false, true), Some("you woke at THE GATE"));
        assert!(woke(Wake::Gate, false, false).unwrap().contains("beach"));
    }

    /// The death marker rides the compass the way the bearing number reads:
    /// dead ahead is the middle, clockwise is right, and behind you is held
    /// at the nearer end rather than dropped.
    #[test]
    fn the_compass_puts_the_death_where_you_would_turn_to() {
        let here = (1000.0, 1000.0);
        // Ten metres away in each of the four directions the bearing names.
        let north = (here.0, here.1 + 10.0);
        let east = (here.0 - 10.0, here.1); // east is -x
        let west = (here.0 + 10.0, here.1);
        let south = (here.0, here.1 - 10.0);
        let at = |facing, to| compass_mark(facing, here, to);

        let ahead = at(0.0, north);
        assert!(ahead.across.abs() < 1e-3 && !ahead.off_row, "{ahead:?}");
        assert!((ahead.dist_m - 10.0).abs() < 1e-3);
        assert!(
            (at(0.0, east).across - 1.0).abs() < 1e-3,
            "east of north is right"
        );
        assert!(
            (at(0.0, west).across + 1.0).abs() < 1e-3,
            "west of north is left"
        );
        // Turn to face it and it comes to the middle.
        assert!(at(90.0, east).across.abs() < 1e-3);
        // Part-way round to the right, in proportion to the span.
        let ne = at(45.0, east);
        assert!(
            (ne.across - 45.0 / COMPASS_HALF_SPAN_DEG).abs() < 1e-3,
            "{ne:?}"
        );
        // Behind, a little to the right: held at the right end, marked off.
        let behind = at(0.0, (here.0 - 1.0, here.1 - 10.0));
        assert!(
            behind.off_row && (behind.across - 1.0).abs() < 1e-3,
            "{behind:?}"
        );
        // Dead behind falls to one end and says so, never the middle.
        let back = at(0.0, south);
        assert!(back.off_row && back.across.abs() > 0.99, "{back:?}");
        // Across the 0/360 seam: facing 350, a mark at 10 is just right.
        let seam = at(350.0, (here.0 - 1.763, here.1 + 10.0));
        assert!(seam.across > 0.0 && seam.across < 0.3, "{seam:?}");
    }

    #[test]
    fn the_compass_distance_is_short() {
        assert_eq!(short_distance(143.4), "143m");
        assert_eq!(short_distance(1_234.0), "1.2km");
    }

    #[test]
    fn a_bag_is_named_by_what_it_is_and_falls_back_to_its_own_word() {
        let cat = catalog_with(5, "Sleeping Bag");
        let mut defs = DeployContent::EMPTY;
        defs.def_count = 1;
        defs.defs[0].arch = ARCH_BAG;
        defs.defs[0].item = 5;
        let rec = DeployRec {
            cx: 10,
            cz: 20,
            level: 0,
            row: 0,
            ..DeployRec::default()
        };
        let b = BagAnchor {
            cx: 10,
            cz: 20,
            level: 0,
            ready: true,
            wait_s: 0,
        };
        assert_eq!(bag_title(&b, &[rec], &defs, 1, &cat), "SLEEPING BAG");
        // No record at that address, or no row dripped yet: still a name.
        assert_eq!(bag_title(&b, &[], &defs, 1, &cat), "SLEEPING BAG");
        assert_eq!(bag_title(&b, &[rec], &defs, 0, &cat), "SLEEPING BAG");
    }
}
