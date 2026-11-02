//! Who a body is beyond what it sees (`--character FILE`): a name, a kind,
//! a persona, seven traits, a few quirks and a goal for its life. The mind
//! is sent it on every turn (`character` in the request), and the traits
//! tune the scripted playbook (`Knobs`), so a fleet of free bodies stops
//! playing as one: a greedy one hoards and loots farther afield, a timid one
//! runs sooner, a sociable one keeps closer to its team.
//!
//! One file can cast a whole fleet. Its `[culture]` table is what every body
//! shares; `[members."NAME"]` overrides it for the body of that name. A body
//! the file does not name, in a fleet, gets the culture with its traits
//! moved deterministically by a hash of its wallet (else its name), so
//! twelve villagers of one culture are twelve people and the same twelve
//! after a restart. A file with a character's keys at its top level is the
//! culture too, and a lone body takes it exactly. TOML, or JSON when the
//! file ends in `.json`. Tables and keys it does not know (a persona file's
//! `[voice]`, say) are someone else's and left alone; a value out of range
//! is brought into it rather than refused, so a hand-edited file never keeps
//! a body off the island.

use crate::agent::combat::Temperament;
use serde_json::{json, Value};

/// The longest persona kept, in bytes.
pub const PERSONA_MAX: usize = 600;
/// The longest life goal and quirk kept, in bytes.
pub const GOAL_MAX: usize = 160;
pub const QUIRK_MAX: usize = 80;
/// Quirks a character carries at most, and how many a fleet body draws
/// from a culture that lists more.
pub const QUIRKS_MAX: usize = 8;
pub const QUIRKS_DRAWN: usize = 3;
/// How far a fleet body's traits stray from its culture's, either way.
pub const TRAIT_SPREAD: i32 = 20;
/// The longest character file read.
pub const FILE_MAX_BYTES: u64 = 256 * 1024;

/// The seven traits, in the order [`Traits`] keeps them.
pub const TRAIT_NAMES: [&str; 7] = [
    "ambition",
    "risk",
    "greed",
    "sociability",
    "honesty",
    "patience",
    "grit",
];

/// One trait, an index into [`Traits`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trait {
    Ambition,
    Risk,
    Greed,
    Sociability,
    Honesty,
    Patience,
    Grit,
}

/// Seven 0-100 scores; 50 is the playbook as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traits(pub [u8; 7]);

impl Default for Traits {
    fn default() -> Self {
        Self::MID
    }
}

impl Traits {
    pub const MID: Self = Self([50; 7]);

    pub fn get(&self, t: Trait) -> u8 {
        self.0[t as usize]
    }

    fn json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (i, name) in TRAIT_NAMES.iter().enumerate() {
            m.insert((*name).into(), json!(self.0[i]));
        }
        Value::Object(m)
    }

    /// What the playbook does differently for these traits. At 50 across
    /// the board every knob is the playbook's own constant.
    pub fn knobs(&self) -> Knobs {
        // A trait's lean from the middle, -50..=50.
        let lean = |t: Trait| i32::from(self.get(t)) - 50;
        let at = |base: i32, per_50: i32, by: i32| base + per_50 * by / 50;
        let (risk, grit, greed) = (lean(Trait::Risk), lean(Trait::Grit), lean(Trait::Greed));
        let (patience, social, honesty) = (
            lean(Trait::Patience),
            lean(Trait::Sociability),
            lean(Trait::Honesty),
        );
        let ambition = lean(Trait::Ambition);
        let pct = |v: i32, lo: i32, hi: i32| v.clamp(lo, hi) as u32;
        Knobs {
            // Bold and tough bodies stay in a losing fight longer.
            flee_hp_pct: pct(at(30, -10, risk) + at(0, -5, grit), 10, 50),
            heal_hp_pct: pct(at(60, -15, risk) + at(0, -5, grit), 30, 85),
            // The patient let a meter run lower before they stop for it.
            low_meter_pct: pct(at(40, -10, patience) + at(0, -5, risk), 20, 60),
            loot_near_m: at(40, 30, greed).clamp(15, 90) as u8,
            // The greedy fill the pack fuller before the walk home.
            stash_free_slots: at(4, -2, greed).clamp(1, 8) as u8,
            plenty: at(3000, 2000, greed).clamp(1000, 6000) as u32,
            satchels: at(4, 3, ambition + risk).clamp(1, 10) as u32,
            raid: match honesty - (greed + ambition) / 2 {
                h if h >= 35 => RaidWhen::Never,
                h if h <= -35 => RaidWhen::Any,
                _ => RaidWhen::Night,
            },
            follow_m: at(25, -12, social).clamp(8, 45) as f32,
        }
    }

    /// When it starts a fight, when nothing else (a society's job, a
    /// `--temperament`) says: risk and ambition together, a reckless
    /// climber shooting on sight.
    pub fn temperament(&self) -> Temperament {
        let edge = (u32::from(self.get(Trait::Risk)) + u32::from(self.get(Trait::Ambition))) / 2;
        match edge {
            80.. => Temperament::Kos,
            40.. => Temperament::Opportunist,
            20.. => Temperament::Defensive,
            _ => Temperament::Passive,
        }
    }
}

/// When the playbook raids a base worth it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaidWhen {
    Never,
    /// By night, as written.
    Night,
    Any,
}

/// The scripted playbook's numbers, from the traits ([`Traits::knobs`]).
/// [`Knobs::default`] is the playbook as written (`mind::SCRIPTED_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knobs {
    pub flee_hp_pct: u32,
    pub heal_hp_pct: u32,
    pub low_meter_pct: u32,
    pub loot_near_m: u8,
    pub stash_free_slots: u8,
    pub plenty: u32,
    pub satchels: u32,
    pub raid: RaidWhen,
    pub follow_m: f32,
}

impl Default for Knobs {
    fn default() -> Self {
        use crate::mind::*;
        Self {
            flee_hp_pct: SCRIPTED_FLEE_HP_PCT,
            heal_hp_pct: SCRIPTED_HEAL_HP_PCT,
            low_meter_pct: SCRIPTED_LOW_METER_PCT,
            loot_near_m: SCRIPTED_LOOT_NEAR_M,
            stash_free_slots: SCRIPTED_STASH_FREE_SLOTS,
            plenty: SCRIPTED_PLENTY,
            satchels: SCRIPTED_SATCHELS,
            raid: RaidWhen::Night,
            follow_m: SCRIPTED_FOLLOW_M,
        }
    }
}

/// What kind of body this is to the house (the Watch page groups by it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Protagonist,
    Teammate,
    Villager,
    Buddy,
    Solo,
    Community,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Protagonist,
        Kind::Teammate,
        Kind::Villager,
        Kind::Buddy,
        Kind::Solo,
        Kind::Community,
        Kind::Other,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Kind::Protagonist => "protagonist",
            Kind::Teammate => "teammate",
            Kind::Villager => "villager",
            Kind::Buddy => "buddy",
            Kind::Solo => "solo",
            Kind::Community => "community",
            Kind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.word() == s)
    }
}

/// One body's character.
#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub name: String,
    pub kind: Kind,
    pub persona: String,
    pub traits: Traits,
    pub quirks: Vec<String>,
    pub goal: String,
}

impl Character {
    /// The plain body: the playbook as written, nothing said about it.
    pub fn plain(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: Kind::Other,
            persona: String::new(),
            traits: Traits::MID,
            quirks: Vec::new(),
            goal: String::new(),
        }
    }

    /// The request's `character`.
    pub fn json(&self) -> Value {
        json!({
            "name": self.name,
            "kind": self.kind.word(),
            "persona": self.persona,
            "traits": self.traits.json(),
            "quirks": self.quirks,
            "goal": self.goal,
        })
    }

    pub fn knobs(&self) -> Knobs {
        self.traits.knobs()
    }
}

/// A character as a file gives it: anything unset comes from the culture.
#[derive(Clone, Debug, Default, PartialEq)]
struct Sketch {
    name: Option<String>,
    kind: Option<Kind>,
    persona: Option<String>,
    traits: [Option<u8>; 7],
    quirks: Option<Vec<String>>,
    goal: Option<String>,
}

/// Printable, one line, cut at `max` bytes on a character boundary.
fn line(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max));
    for c in text.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if c.is_control() || (c == ' ' && out.ends_with(' ')) {
            continue;
        }
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

impl Sketch {
    fn parse(v: &Value, at: &str) -> Result<Self, String> {
        let Some(o) = v.as_object() else {
            return Err(format!("{at} is a table of character keys"));
        };
        let text = |key: &str, max: usize| -> Result<Option<String>, String> {
            match o.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(line(s, max))),
                Some(_) => Err(format!("{at}.{key} is a string")),
            }
        };
        let mut s = Sketch {
            name: text("name", 64)?,
            persona: text("persona", PERSONA_MAX)?,
            goal: text("goal", GOAL_MAX)?,
            ..Sketch::default()
        };
        if let Some(k) = o.get("kind").filter(|k| !k.is_null()) {
            let word = k.as_str().ok_or_else(|| format!("{at}.kind is a string"))?;
            s.kind = Some(Kind::parse(word).unwrap_or(Kind::Other));
        }
        if let Some(t) = o.get("traits").filter(|t| !t.is_null()) {
            let t = t
                .as_object()
                .ok_or_else(|| format!("{at}.traits is a table"))?;
            for (key, value) in t {
                let Some(i) = TRAIT_NAMES.iter().position(|n| n == key) else {
                    continue;
                };
                let n = value
                    .as_f64()
                    .ok_or_else(|| format!("{at}.traits.{key} is a number, 0-100"))?;
                s.traits[i] = Some(n.round().clamp(0.0, 100.0) as u8);
            }
        }
        if let Some(q) = o.get("quirks").filter(|q| !q.is_null()) {
            let q = q
                .as_array()
                .ok_or_else(|| format!("{at}.quirks is a list"))?;
            let mut quirks = Vec::new();
            for item in q.iter().take(QUIRKS_MAX * 4) {
                let text = item
                    .as_str()
                    .ok_or_else(|| format!("{at}.quirks are strings"))?;
                let text = line(text, QUIRK_MAX);
                if !text.is_empty() {
                    quirks.push(text);
                }
            }
            s.quirks = Some(quirks);
        }
        Ok(s)
    }

    /// `self` over `under`: each key it sets wins.
    fn over(&self, under: &Sketch) -> Sketch {
        let mut traits = under.traits;
        for (i, t) in self.traits.iter().enumerate() {
            if t.is_some() {
                traits[i] = *t;
            }
        }
        Sketch {
            name: self.name.clone().or_else(|| under.name.clone()),
            kind: self.kind.or(under.kind),
            persona: self.persona.clone().or_else(|| under.persona.clone()),
            traits,
            quirks: self.quirks.clone().or_else(|| under.quirks.clone()),
            goal: self.goal.clone().or_else(|| under.goal.clone()),
        }
    }
}

/// The keys a character table may hold.
const CHARACTER_KEYS: [&str; 6] = ["name", "kind", "persona", "traits", "quirks", "goal"];

/// A character file: the culture, and the bodies it names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cast {
    culture: Sketch,
    bodies: Vec<(String, Sketch)>,
}

impl Cast {
    /// Read `path`: JSON when it ends in `.json`, TOML otherwise.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut text = String::new();
        file.take(FILE_MAX_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if text.len() as u64 > FILE_MAX_BYTES {
            return Err(format!("{} is over {FILE_MAX_BYTES} bytes", path.display()));
        }
        let json = path.extension().is_some_and(|e| e == "json");
        Self::parse(&text, json).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str, json: bool) -> Result<Self, String> {
        let v: Value = if json {
            serde_json::from_str(text).map_err(|e| e.to_string())?
        } else {
            let t: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
            serde_json::to_value(t).map_err(|e| e.to_string())?
        };
        let o = v.as_object().ok_or("a character file is a table")?;
        let mut top = serde_json::Map::new();
        for (k, val) in o {
            if CHARACTER_KEYS.contains(&k.as_str()) {
                top.insert(k.into(), val.clone());
            }
        }
        let mut culture = Sketch::parse(&Value::Object(top), "the file")?;
        if let Some(c) = o.get("culture") {
            culture = Sketch::parse(c, "culture")?.over(&culture);
        }
        let mut bodies = Vec::new();
        if let Some(b) = o.get("members") {
            let b = b.as_object().ok_or("members is a table of names")?;
            for (name, sketch) in b {
                bodies.push((
                    name.clone(),
                    Sketch::parse(sketch, &format!("members.{name}"))?,
                ));
            }
        }
        Ok(Self { culture, bodies })
    }

    /// Body `name`'s character, its wallet (hex) when it has one. A body
    /// the file names takes its entry over the culture, exactly; one it
    /// does not, in a `fleet`, takes the culture with its traits moved by
    /// its own hash ([`TRAIT_SPREAD`]) and its quirks drawn from the
    /// culture's ([`QUIRKS_DRAWN`]).
    pub fn for_body(&self, name: &str, wallet: Option<&str>, fleet: bool) -> Character {
        let named = self.bodies.iter().find(|(n, _)| n == name);
        let sketch = match named {
            Some((_, s)) => s.over(&self.culture),
            None => self.culture.clone(),
        };
        let mut traits = Traits::MID;
        for (i, t) in sketch.traits.iter().enumerate() {
            traits.0[i] = t.unwrap_or(50);
        }
        let mut quirks = sketch.quirks.unwrap_or_default();
        if fleet && named.is_none() {
            let seed = crate::character::seed(wallet.unwrap_or(name));
            for (i, t) in traits.0.iter_mut().enumerate() {
                let roll = ((seed >> (i * 8)) & 0xFF) as i32;
                let shift = roll * (2 * TRAIT_SPREAD + 1) / 256 - TRAIT_SPREAD;
                *t = (i32::from(*t) + shift).clamp(0, 100) as u8;
            }
            if quirks.len() > QUIRKS_DRAWN {
                let mut drawn = Vec::with_capacity(QUIRKS_DRAWN);
                let mut pool = quirks;
                let mut s = seed.rotate_left(17);
                while drawn.len() < QUIRKS_DRAWN {
                    let i = (s % pool.len() as u64) as usize;
                    drawn.push(pool.swap_remove(i));
                    s = s.rotate_left(13) ^ 0x9E37_79B9_7F4A_7C15;
                }
                quirks = drawn;
            }
        }
        quirks.truncate(QUIRKS_MAX);
        Character {
            // The name a human sees is the body's: a file's `name` is only
            // kept for a lone body named after it.
            name: sketch
                .name
                .filter(|_| !fleet || named.is_some())
                .unwrap_or_else(|| name.to_owned()),
            kind: sketch.kind.unwrap_or(Kind::Other),
            persona: sketch.persona.unwrap_or_default(),
            traits,
            quirks,
            goal: sketch.goal.unwrap_or_default(),
        }
    }
}

/// A body's own number, from its wallet or name: the same every start.
pub fn seed(of: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(of.to_ascii_lowercase().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_is_the_playbook_as_written() {
        assert_eq!(Traits::MID.knobs(), Knobs::default());
        assert_eq!(Traits::MID.temperament(), Temperament::Opportunist);
        let mut reckless = Traits::MID;
        reckless.0[Trait::Risk as usize] = 95;
        reckless.0[Trait::Ambition as usize] = 90;
        assert_eq!(reckless.temperament(), Temperament::Kos);
        let k = reckless.knobs();
        assert!(k.flee_hp_pct < Knobs::default().flee_hp_pct);
        assert!(k.satchels > Knobs::default().satchels);
        let mut meek = Traits([10; 7]);
        meek.0[Trait::Honesty as usize] = 100;
        assert_eq!(meek.temperament(), Temperament::Passive);
        assert_eq!(meek.knobs().raid, RaidWhen::Never);
    }

    #[test]
    fn a_culture_casts_a_fleet_of_different_people_and_a_named_body_exactly() {
        let toml = r#"
            kind = "villager"

            [voice]
            id = "someone else's"

            [culture]
            persona = "A farmer of the river village."
            goal = "Keep the granary full"
            quirks = ["hums", "counts stones", "hates wolves", "trusts nobody new", "boasts"]

            [culture.traits]
            ambition = 40
            risk = 30
            greed = 50
            sociability = 70
            honesty = 70
            patience = 60
            grit = 50
            luck = 99

            [members."Ryuzen"]
            kind = "protagonist"
            persona = "Grinds like it is his anime arc."
            traits = { ambition = 100, risk = 90 }
        "#;
        let cast = Cast::parse(toml, false).unwrap();
        let ryu = cast.for_body("Ryuzen", None, true);
        assert_eq!(ryu.kind, Kind::Protagonist);
        assert_eq!(ryu.traits.get(Trait::Ambition), 100);
        assert_eq!(
            ryu.traits.get(Trait::Sociability),
            70,
            "the rest is the culture's"
        );
        assert_eq!(ryu.goal, "Keep the granary full");
        let villagers: Vec<Character> = (1..=12)
            .map(|i| cast.for_body(&format!("oak-{i}"), None, true))
            .collect();
        let distinct: std::collections::BTreeSet<[u8; 7]> =
            villagers.iter().map(|c| c.traits.0).collect();
        assert!(distinct.len() >= 11, "twelve villagers, twelve people");
        for v in &villagers {
            assert_eq!(v.kind, Kind::Villager);
            assert_eq!(v.quirks.len(), QUIRKS_DRAWN);
            for (i, t) in v.traits.0.iter().enumerate() {
                let base = [40, 30, 50, 70, 70, 60, 50][i];
                assert!(
                    (i32::from(*t) - base).abs() <= TRAIT_SPREAD,
                    "{t} vs {base}"
                );
            }
        }
        assert_eq!(
            cast.for_body("oak-3", None, true),
            villagers[2],
            "the same body is the same person after a restart"
        );
        assert_ne!(
            cast.for_body("oak-3", Some("0xabc"), true).traits,
            villagers[2].traits,
            "a wallet, when it has one, is what it is hashed by"
        );
        let lone = cast.for_body("oak-3", None, false);
        assert_eq!(
            lone.traits.get(Trait::Risk),
            30,
            "a lone body takes it exactly"
        );
        assert_eq!(lone.quirks.len(), 5);
    }

    #[test]
    fn a_file_is_toml_or_json_and_refuses_what_it_does_not_know() {
        let json = r#"{"name": "Ryuzen", "kind": "protagonist", "persona": "x",
            "traits": {"grit": 99}, "quirks": ["shouts his attacks"], "goal": "rule the island"}"#;
        let c = Cast::parse(json, true)
            .unwrap()
            .for_body("body", None, false);
        assert_eq!(c.name, "Ryuzen");
        assert_eq!(c.traits.get(Trait::Grit), 99);
        assert_eq!(c.json()["traits"]["grit"], 99);
        assert_eq!(c.json()["kind"], "protagonist");
        for bad in [
            r#"{"quirks": "one"}"#,
            r#"{"traits": {"grit": "lots"}}"#,
            "[1]",
        ] {
            assert!(Cast::parse(bad, true).is_err(), "{bad}");
        }
        let odd = r#"{"kind": "god", "mood": "x", "traits": {"grit": 101, "luck": 5}}"#;
        let c = Cast::parse(odd, true).unwrap().for_body("b", None, false);
        assert_eq!((c.kind, c.traits.get(Trait::Grit)), (Kind::Other, 100));
        let file = r#"
            name = "Doctor Wood"
            kind = "buddy"
            goal = "Keep my friend alive"
            [traits]
            patience = 80
            [voice]
            eleven = "abc"
        "#;
        let c = Cast::parse(file, false)
            .unwrap()
            .for_body("Doctor Wood", None, false);
        assert_eq!((c.kind, c.traits.get(Trait::Patience)), (Kind::Buddy, 80));
        let long = format!(r#"{{"persona": "{}"}}"#, "é".repeat(PERSONA_MAX));
        let c = Cast::parse(&long, true).unwrap().for_body("b", None, false);
        assert!(c.persona.len() <= PERSONA_MAX);
        assert_eq!(Character::plain("b").knobs(), Knobs::default());
    }
}
