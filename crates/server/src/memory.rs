//! What a body remembers past one look (`--memory FILE`): the places it has
//! seen, the people it has met and how they treated it, what happened to it
//! that mattered, and the life story and strategy its mind keeps.
//!
//! Two halves, split on the agent walls. The **frame side** ([`Perceived`])
//! is a fixed-size, copyable record the body files as things happen (a blow
//! and from whom, a kill, its own death) and fills at each decision with what
//! is in view and where its places are; it rides to the decision worker in
//! the [`Summary`] and is never serialized as it stands. The **worker side**
//! ([`Memory`], kept by [`Remembering`]) folds each one in, allocating as it
//! likes off the frame path, keeps it bounded (places, people and events are
//! capped, newest kept) and saves it atomically when a file is named.
//!
//! Places are stored in world metres here, on our box. A mind is shown them
//! only as a bearing and a distance from where the body stands and faces now,
//! as every other place in its turn is: no position, no seed. People are
//! named as a human sees them (the nametag, else the chat log's label), and
//! what it remembers of a person is what the body itself saw and heard.

use crate::character::Character;
use crate::mind::{Choice, DecisionSource, Goal, Name, Outcome, Plan, SourceKind, Summary};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// People, places and events one look carries (frame side).
pub const SEEN_PEOPLE: usize = 8;
pub const SEEN_PLACES: usize = 16;
pub const SEEN_EVENTS: usize = 6;
/// What the memory keeps, newest first out when full (the contract's caps).
pub const PLACES_MAX: usize = 24;
pub const PEOPLE_MAX: usize = 24;
pub const EVENTS_MAX: usize = 16;
/// Text bounds, in bytes.
pub const SUMMARY_MAX: usize = 600;
pub const THOUGHT_MAX: usize = 280;
pub const SAID_MAX: usize = 120;
pub const EVENT_MAX: usize = 120;
pub const STRATEGY_TEXT_MAX: usize = 200;
pub const STANDING_ORDERS_MAX: usize = 4;
/// How often a named file is rewritten, at most (and once more at the end).
pub const SAVE_EVERY: Duration = Duration::from_secs(30);
/// The longest memory file read back.
pub const FILE_MAX_BYTES: u64 = 512 * 1024;
/// A place of a kind seen within this of one remembered is that one.
const SAME_PLACE_M: f32 = 30.0;
/// Seen again after this long is another time seen.
const NEW_SIGHTING_S: u64 = 120;
/// What a save says it is.
const FORMAT: &str = "gates-memory-1";

/// What kind of place a remembered one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Home,
    Base,
    Resource,
    Loot,
    Landmark,
    Danger,
    Work,
    Gate,
    Other,
}

impl PlaceKind {
    const ALL: [PlaceKind; 9] = [
        PlaceKind::Home,
        PlaceKind::Base,
        PlaceKind::Resource,
        PlaceKind::Loot,
        PlaceKind::Landmark,
        PlaceKind::Danger,
        PlaceKind::Work,
        PlaceKind::Gate,
        PlaceKind::Other,
    ];

    pub fn word(self) -> &'static str {
        match self {
            PlaceKind::Home => "home",
            PlaceKind::Base => "base",
            PlaceKind::Resource => "resource",
            PlaceKind::Loot => "loot",
            PlaceKind::Landmark => "landmark",
            PlaceKind::Danger => "danger",
            PlaceKind::Work => "work",
            PlaceKind::Gate => "gate",
            PlaceKind::Other => "other",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.word() == s)
    }

    /// One of a kind on the island, or one per name: a newer sighting moves
    /// it rather than adding another.
    fn unique(self) -> bool {
        matches!(self, PlaceKind::Home | PlaceKind::Gate | PlaceKind::Work)
    }
}

/// A place the body saw this look, in world metres (frame side).
#[derive(Clone, Copy, Debug)]
pub struct PlaceSeen {
    pub kind: PlaceKind,
    pub name: Name,
    pub at: [f32; 2],
}

impl PlaceSeen {
    const EMPTY: Self = Self {
        kind: PlaceKind::Other,
        name: Name::EMPTY,
        at: [0.0; 2],
    };
}

/// A person the body met since it last asked (frame side): who, as its
/// nametag says, and what passed between them.
#[derive(Clone, Copy, Debug)]
pub struct PersonSeen {
    /// The wire id: only to fold two notes on one body together, never
    /// shown (a guest's label is `#id`, as a chat log writes it).
    pub from: u32,
    pub name: protocol::Name,
    pub address: protocol::Address,
    pub agent: bool,
    pub teammate: bool,
    /// In sight at this look.
    pub in_view: bool,
    pub armed: bool,
    pub hits_on_me: u16,
    pub my_hits: u16,
    pub killed_me: u8,
    pub i_killed: u8,
}

impl PersonSeen {
    const EMPTY: Self = Self {
        from: 0,
        name: protocol::Name::EMPTY,
        address: protocol::Address::GUEST,
        agent: false,
        teammate: false,
        in_view: false,
        armed: false,
        hits_on_me: 0,
        my_hits: 0,
        killed_me: 0,
        i_killed: 0,
    };

    /// The label a person's chat line carries ([`crate::mind::HeardLine::label`]).
    pub fn label(&self) -> String {
        crate::mind::HeardLine {
            from: self.from,
            name: self.name,
            address: self.address,
            ..crate::mind::HeardLine::EMPTY
        }
        .label()
    }
}

/// Something that happened to the body that only the frame side saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// It died; `who` killed it, if a player did.
    Died,
    /// It killed `who`.
    Killed,
}

#[derive(Clone, Copy, Debug)]
pub struct EventSeen {
    pub kind: EventKind,
    /// The person, an index into the look's `people`; `None` for the wild.
    pub who: Option<u8>,
}

/// What the body perceived since it last asked, and its pose now: filed on
/// the frame path without allocating, carried by the [`Summary`], read on
/// the worker. `pos` and every place's `at` are world metres for the
/// memory's own book-keeping: [`Summary::to_json`] never reads this.
#[derive(Clone, Copy, Debug)]
pub struct Perceived {
    pub pos: Option<[f32; 2]>,
    pub yaw: u16,
    /// Players this body has killed, all run (the kill feed's word).
    pub kills: u32,
    pub people: [PersonSeen; SEEN_PEOPLE],
    pub people_len: u8,
    pub places: [PlaceSeen; SEEN_PLACES],
    pub places_len: u8,
    pub events: [EventSeen; SEEN_EVENTS],
    pub events_len: u8,
}

impl Default for Perceived {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Perceived {
    pub const EMPTY: Self = Self {
        pos: None,
        yaw: 0,
        kills: 0,
        people: [PersonSeen::EMPTY; SEEN_PEOPLE],
        people_len: 0,
        places: [PlaceSeen::EMPTY; SEEN_PLACES],
        places_len: 0,
        events: [EventSeen {
            kind: EventKind::Died,
            who: None,
        }; SEEN_EVENTS],
        events_len: 0,
    };

    pub fn people(&self) -> &[PersonSeen] {
        &self.people[..usize::from(self.people_len)]
    }

    pub fn places(&self) -> &[PlaceSeen] {
        &self.places[..usize::from(self.places_len)]
    }

    pub fn events(&self) -> &[EventSeen] {
        &self.events[..usize::from(self.events_len)]
    }

    /// Player `from`'s row, made if there is room: its index.
    pub fn person(
        &mut self,
        from: u32,
        name: protocol::Name,
        address: protocol::Address,
        agent: bool,
    ) -> Option<usize> {
        if let Some(i) = self.people().iter().position(|p| p.from == from) {
            return Some(i);
        }
        let i = usize::from(self.people_len);
        if i >= SEEN_PEOPLE {
            return None;
        }
        self.people[i] = PersonSeen {
            from,
            name,
            address,
            agent,
            ..PersonSeen::EMPTY
        };
        self.people_len += 1;
        Some(i)
    }

    /// A place seen this look; a full look drops it.
    pub fn place(&mut self, kind: PlaceKind, name: Name, at: [f32; 2]) {
        let i = usize::from(self.places_len);
        if i < SEEN_PLACES && at.iter().all(|v| v.is_finite()) {
            self.places[i] = PlaceSeen { kind, name, at };
            self.places_len += 1;
        }
    }

    pub fn event(&mut self, kind: EventKind, who: Option<usize>) {
        let i = usize::from(self.events_len);
        if i < SEEN_EVENTS {
            self.events[i] = EventSeen {
                kind,
                who: who.map(|w| w as u8),
            };
            self.events_len += 1;
        }
    }

    /// A request went out with it: what happened is told, the count stays.
    pub fn clear(&mut self) {
        *self = Self {
            kills: self.kills,
            ..Self::EMPTY
        };
    }
}

/// How a mind marks a person (`stance` in a reply).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Friend,
    Neutral,
    Wary,
    Enemy,
    Rival,
}

impl Stance {
    const ALL: [Stance; 5] = [
        Stance::Friend,
        Stance::Neutral,
        Stance::Wary,
        Stance::Enemy,
        Stance::Rival,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Stance::Friend => "friend",
            Stance::Neutral => "neutral",
            Stance::Wary => "wary",
            Stance::Enemy => "enemy",
            Stance::Rival => "rival",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.word() == s)
    }
}

/// What a strategist left: echoed to every later turn (`strategy`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strategy {
    pub ambition: String,
    pub project: String,
    pub standing_orders: Vec<String>,
    /// Unix seconds it was set.
    pub set_at: u64,
}

impl Strategy {
    pub fn json(&self, now: u64) -> Value {
        json!({
            "ambition": self.ambition,
            "project": self.project,
            "standing_orders": self.standing_orders,
            "set_ago_s": now.saturating_sub(self.set_at),
        })
    }
}

/// What a reply said beyond its goal (`thought`, `strategy`, `stance`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub thought: Option<String>,
    pub strategy: Option<Strategy>,
    /// The strategy's `summary`: the body's life story, kept in memory.
    pub summary: Option<String>,
    pub stance: Vec<(String, Stance)>,
}

/// Printable, one line, cut at `max` bytes on a character boundary.
pub fn clean(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max));
    for c in text.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if c.is_control() || (c == ' ' && (out.is_empty() || out.ends_with(' '))) {
            continue;
        }
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out.trim_end().to_owned()
}

impl Extras {
    /// A reply's `thought`, `strategy` and `stance`. Missing or null is
    /// none; text is cleaned and cut to its bound; a wrong shape is refused
    /// whole (the goal beside it still stands).
    pub fn parse(reply: &Value) -> Result<Extras, String> {
        let mut x = Extras::default();
        match &reply["thought"] {
            Value::Null => {}
            Value::String(t) => x.thought = Some(clean(t, THOUGHT_MAX)).filter(|t| !t.is_empty()),
            _ => return Err("thought is a string".into()),
        }
        match &reply["strategy"] {
            Value::Null => {}
            Value::Object(o) => {
                let text = |k: &str, max: usize| match o.get(k) {
                    None | Some(Value::Null) => Ok(String::new()),
                    Some(Value::String(s)) => Ok(clean(s, max)),
                    Some(_) => Err(format!("strategy.{k} is a string")),
                };
                let mut orders = Vec::new();
                match o.get("standing_orders") {
                    None | Some(Value::Null) => {}
                    Some(Value::Array(a)) if a.len() <= STANDING_ORDERS_MAX => {
                        for order in a {
                            let t = order.as_str().ok_or("standing_orders are strings")?;
                            let t = clean(t, STRATEGY_TEXT_MAX);
                            if !t.is_empty() {
                                orders.push(t);
                            }
                        }
                    }
                    Some(_) => {
                        return Err(format!(
                            "standing_orders is a list of at most {STANDING_ORDERS_MAX} strings"
                        ))
                    }
                }
                x.strategy = Some(Strategy {
                    ambition: text("ambition", STRATEGY_TEXT_MAX)?,
                    project: text("project", STRATEGY_TEXT_MAX)?,
                    standing_orders: orders,
                    set_at: 0,
                });
                let summary = text("summary", SUMMARY_MAX)?;
                x.summary = (!summary.is_empty()).then_some(summary);
            }
            _ => return Err("strategy is an object".into()),
        }
        match &reply["stance"] {
            Value::Null => {}
            Value::Array(a) if a.len() <= PEOPLE_MAX => {
                for row in a {
                    let name = row["name"].as_str().map(|n| clean(n, 64));
                    let stance = row["stance"].as_str().and_then(Stance::parse);
                    match (name, stance) {
                        (Some(n), Some(s)) if !n.is_empty() => x.stance.push((n, s)),
                        _ => {
                            return Err(
                                "stance rows are {\"name\", \"stance\": friend|neutral|wary|enemy|rival}"
                                    .into(),
                            )
                        }
                    }
                }
            }
            Value::Array(_) => return Err(format!("stance marks at most {PEOPLE_MAX} people")),
            _ => return Err("stance is a list".into()),
        }
        Ok(x)
    }

    pub fn is_empty(&self) -> bool {
        self.thought.is_none()
            && self.strategy.is_none()
            && self.summary.is_none()
            && self.stance.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PlaceMem {
    kind: PlaceKind,
    name: String,
    at: [f32; 2],
    seen_at: u64,
    note: String,
}

#[derive(Clone, Debug, PartialEq)]
struct PersonMem {
    name: String,
    agent: bool,
    teammate: bool,
    met_at: u64,
    /// Last in sight; 0 for heard but never seen.
    seen_at: u64,
    /// Last seen or heard: who is kept when the book is full.
    known_at: u64,
    times_seen: u32,
    armed_seen: bool,
    hits_on_me: u32,
    my_hits: u32,
    killed_me: u32,
    i_killed: u32,
    stance: Option<Stance>,
    last_said: String,
    hours: [u16; 24],
    /// The unix hour last counted into `hours`.
    hour_at: u64,
}

impl PersonMem {
    fn new(name: &str, now: u64) -> Self {
        Self {
            name: name.into(),
            agent: false,
            teammate: false,
            met_at: now,
            seen_at: 0,
            known_at: now,
            times_seen: 0,
            armed_seen: false,
            hits_on_me: 0,
            my_hits: 0,
            killed_me: 0,
            i_killed: 0,
            stance: None,
            last_said: String::new(),
            hours: [0; 24],
            hour_at: u64::MAX,
        }
    }

    /// Seen or heard at `now`: the hour counted once.
    fn about(&mut self, now: u64) {
        self.known_at = now;
        let hour = now / 3600;
        if self.hour_at != hour {
            self.hour_at = hour;
            let h = &mut self.hours[(hour % 24) as usize];
            *h = h.saturating_add(1);
        }
    }

    /// The mind's mark, else what the body's own record says.
    fn stance(&self) -> Stance {
        self.stance.unwrap_or(if self.teammate {
            Stance::Friend
        } else if self.killed_me > 0 || self.hits_on_me >= 3 {
            Stance::Enemy
        } else if self.hits_on_me > 0 {
            Stance::Wary
        } else {
            Stance::Neutral
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EventMem {
    at: u64,
    what: String,
}

/// The body's memory (worker side): bounded, saved by [`Remembering`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Memory {
    places: Vec<PlaceMem>,
    people: Vec<PersonMem>,
    events: VecDeque<EventMem>,
    summary: String,
    strategy: Option<Strategy>,
    /// The last look folded in, for what changed between two: home struck,
    /// a teammate gone, a trade done, a death the feed did not name.
    prev: Option<Prev>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Prev {
    home_attacked: bool,
    mates: u8,
    deaths: u32,
    last: Option<crate::mind::Report>,
}

impl Memory {
    fn person(&mut self, label: &str, now: u64) -> &mut PersonMem {
        let i = match self.people.iter().position(|p| p.name == label) {
            Some(i) => i,
            None => {
                if self.people.len() >= PEOPLE_MAX {
                    // The one longest out of mind goes.
                    if let Some(old) = self
                        .people
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, p)| p.known_at)
                        .map(|(i, _)| i)
                    {
                        self.people.swap_remove(old);
                    }
                }
                self.people.push(PersonMem::new(label, now));
                self.people.len() - 1
            }
        };
        &mut self.people[i]
    }

    fn event(&mut self, what: String, now: u64) {
        if self.events.len() >= EVENTS_MAX {
            self.events.pop_front();
        }
        self.events.push_back(EventMem {
            at: now,
            what: clean(&what, EVENT_MAX),
        });
    }

    fn place(&mut self, kind: PlaceKind, name: &str, at: [f32; 2], now: u64, note: &str) {
        let same = |p: &PlaceMem| {
            p.kind == kind
                && if kind.unique() {
                    p.name == name
                } else {
                    (p.at[0] - at[0]).hypot(p.at[1] - at[1]) <= SAME_PLACE_M
                }
        };
        if let Some(p) = self.places.iter_mut().find(|p| same(p)) {
            p.at = at;
            p.seen_at = now;
            if !name.is_empty() {
                p.name = name.into();
            }
            if !note.is_empty() {
                p.note = note.into();
            }
            return;
        }
        if self.places.len() >= PLACES_MAX {
            if let Some(old) = self
                .places
                .iter()
                .enumerate()
                .min_by_key(|(_, p)| p.seen_at)
                .map(|(i, _)| i)
            {
                self.places.swap_remove(old);
            }
        }
        self.places.push(PlaceMem {
            kind,
            name: name.into(),
            at,
            seen_at: now,
            note: note.into(),
        });
    }

    /// Fold one look in, at unix second `now`.
    pub fn ingest(&mut self, s: &Summary, now: u64) {
        let seen = &s.seen;
        let labels: Vec<String> = seen.people().iter().map(PersonSeen::label).collect();
        for (p, label) in seen.people().iter().zip(&labels) {
            let m = self.person(label, now);
            m.agent |= p.agent;
            m.teammate = p.teammate;
            if p.in_view {
                if now.saturating_sub(m.seen_at) > NEW_SIGHTING_S || m.times_seen == 0 {
                    m.times_seen += 1;
                }
                m.seen_at = now;
                m.armed_seen |= p.armed;
            }
            m.hits_on_me += u32::from(p.hits_on_me);
            m.my_hits += u32::from(p.my_hits);
            m.killed_me += u32::from(p.killed_me);
            m.i_killed += u32::from(p.i_killed);
            m.about(now);
        }
        for line in s.heard.lines().iter().filter(|l| !l.me) {
            let label = line.label();
            let said = String::from_utf8_lossy(line.text.as_bytes()).into_owned();
            let m = self.person(&label, now);
            m.agent |= line.agent;
            // A teammate's line is private, and so stays out of the book:
            // what a person said where anyone could hear is what is kept.
            if line.channel != protocol::Channel::Team {
                m.last_said = clean(&said, SAID_MAX);
            }
            m.about(now);
        }
        let who = |e: &EventSeen| e.who.and_then(|w| labels.get(usize::from(w))).cloned();
        let mut death_told = false;
        for e in seen.events() {
            match (e.kind, who(e)) {
                (EventKind::Died, Some(name)) => {
                    death_told = true;
                    self.event(format!("I was killed by {name}"), now)
                }
                (EventKind::Died, None) => {
                    death_told = true;
                    self.event("I died, and no player did it".into(), now)
                }
                (EventKind::Killed, Some(name)) => self.event(format!("I killed {name}"), now),
                (EventKind::Killed, None) => {}
            }
        }
        for p in seen.places() {
            let note = match p.kind {
                PlaceKind::Danger => "I died here",
                _ => "",
            };
            self.place(p.kind, p.name.as_str(), p.at, now, note);
        }
        if let Some(prev) = self.prev {
            let attacked = s.home.attacked || s.team.home_attacked;
            if attacked && !prev.home_attacked {
                self.event("my home was attacked".into(), now);
            }
            if s.team.mates < prev.mates && s.team.mates > 0
                || (prev.mates > 0 && s.team.mates == 0)
            {
                self.event("a teammate left my team".into(), now);
            }
            if s.deaths > prev.deaths && !death_told {
                self.event("I died".into(), now);
            }
            if let Some(r) = s.last.filter(|r| Some(*r) != prev.last) {
                if matches!(r.goal, Goal::Buy(_) | Goal::Sell(_)) && r.outcome == Outcome::Done {
                    self.event(
                        format!("I traded at THE GATE: {}", r.goal.label().as_str()),
                        now,
                    );
                }
                if r.goal == Goal::Raid && r.outcome == Outcome::Done {
                    self.event("I raided a base".into(), now);
                }
            }
        }
        self.prev = Some(Prev {
            home_attacked: s.home.attacked || s.team.home_attacked,
            mates: s.team.mates,
            deaths: s.deaths,
            last: s.last,
        });
    }

    /// A reply's strategy, summary and stance, kept.
    pub fn apply(&mut self, x: &Extras, now: u64) {
        if let Some(st) = &x.strategy {
            self.strategy = Some(Strategy {
                set_at: now,
                ..st.clone()
            });
        }
        if let Some(summary) = &x.summary {
            self.summary = clean(summary, SUMMARY_MAX);
        }
        for (name, stance) in &x.stance {
            // Only a person it knows: a mark is never a way to add a name.
            if let Some(p) = self.people.iter_mut().find(|p| p.name == *name) {
                p.stance = Some(*stance);
            }
        }
    }

    pub fn strategy(&self) -> Option<&Strategy> {
        self.strategy.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
            && self.people.is_empty()
            && self.events.is_empty()
            && self.summary.is_empty()
    }

    /// The request's `memory`, from where the body stands and faces in `s`:
    /// places as a bearing and metres, never a position.
    pub fn json(&self, s: &Summary, now: u64) -> Value {
        let pose = s.seen.pos.map(|p| (p, s.seen.yaw));
        let mut places: Vec<(f32, Value)> = self
            .places
            .iter()
            .map(|p| {
                let (d, bearing) = match pose {
                    Some((at, yaw)) => {
                        let (d, b) = relative(at, yaw, p.at);
                        (d, json!(crate::mind::BEARINGS[usize::from(b) % 8]))
                    }
                    None => (f32::INFINITY, Value::Null),
                };
                let mut v = json!({
                    "name": p.name,
                    "kind": p.kind.word(),
                    "bearing": bearing,
                    "distance_m": d.is_finite().then(|| (d / 5.0).round() as u32 * 5),
                    "seen_ago_s": now.saturating_sub(p.seen_at),
                });
                if !p.note.is_empty() {
                    v["note"] = json!(p.note);
                }
                (d, v)
            })
            .collect();
        places.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut people: Vec<&PersonMem> = self.people.iter().collect();
        people.sort_by_key(|p| std::cmp::Reverse(p.known_at));
        let ago = |t: u64| (t > 0).then(|| now.saturating_sub(t));
        json!({
            "places": places.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
            "people": people.iter().map(|p| json!({
                "name": p.name,
                "agent": p.agent,
                "teammate": p.teammate,
                "met_ago_s": now.saturating_sub(p.met_at),
                "seen_ago_s": ago(p.seen_at),
                "times_seen": p.times_seen,
                "armed_seen": p.armed_seen,
                "hits_on_me": p.hits_on_me,
                "my_hits_on_them": p.my_hits,
                "killed_me": p.killed_me,
                "i_killed": p.i_killed,
                "stance": p.stance().word(),
                "last_said": (!p.last_said.is_empty()).then_some(&p.last_said),
                "online_hours_utc": p.hours,
            })).collect::<Vec<_>>(),
            "events": self.events.iter().map(|e| json!({
                "ago_s": now.saturating_sub(e.at),
                "what": e.what,
            })).collect::<Vec<_>>(),
            "summary": self.summary,
        })
    }

    /// The whole memory as it is saved: world positions included, so only
    /// ever written to this box.
    fn save_json(&self) -> Value {
        json!({
            "format": FORMAT,
            "places": self.places.iter().map(|p| json!({
                "kind": p.kind.word(), "name": p.name, "x": p.at[0], "z": p.at[1],
                "seen_at": p.seen_at, "note": p.note,
            })).collect::<Vec<_>>(),
            "people": self.people.iter().map(|p| json!({
                "name": p.name, "agent": p.agent, "teammate": p.teammate,
                "met_at": p.met_at, "seen_at": p.seen_at, "known_at": p.known_at,
                "times_seen": p.times_seen, "armed_seen": p.armed_seen,
                "hits_on_me": p.hits_on_me, "my_hits": p.my_hits,
                "killed_me": p.killed_me, "i_killed": p.i_killed,
                "stance": p.stance.map(Stance::word), "last_said": p.last_said,
                "hours": p.hours,
            })).collect::<Vec<_>>(),
            "events": self.events.iter().map(|e| json!({"at": e.at, "what": e.what})).collect::<Vec<_>>(),
            "summary": self.summary,
            "strategy": self.strategy.as_ref().map(|st| json!({
                "ambition": st.ambition, "project": st.project,
                "standing_orders": st.standing_orders, "set_at": st.set_at,
            })),
        })
    }

    /// A saved memory read back; anything that does not fit is an error, and
    /// the caller starts fresh.
    fn from_save(v: &Value) -> Result<Memory, String> {
        if v["format"] != FORMAT {
            return Err(format!("not {FORMAT}"));
        }
        let text = |v: &Value, max: usize| clean(v.as_str().unwrap_or(""), max);
        let n = |v: &Value| v.as_u64().ok_or("a number is missing");
        let mut m = Memory::default();
        for p in v["places"]
            .as_array()
            .ok_or("no places")?
            .iter()
            .take(PLACES_MAX)
        {
            let kind = PlaceKind::parse(p["kind"].as_str().unwrap_or("")).ok_or("a place kind")?;
            let (x, z) = (p["x"].as_f64(), p["z"].as_f64());
            let (Some(x), Some(z)) = (x, z) else {
                return Err("a place has no position".into());
            };
            m.places.push(PlaceMem {
                kind,
                name: text(&p["name"], 64),
                at: [x as f32, z as f32],
                seen_at: n(&p["seen_at"])?,
                note: text(&p["note"], EVENT_MAX),
            });
        }
        for p in v["people"]
            .as_array()
            .ok_or("no people")?
            .iter()
            .take(PEOPLE_MAX)
        {
            let mut hours = [0u16; 24];
            for (i, h) in p["hours"]
                .as_array()
                .ok_or("no hours")?
                .iter()
                .take(24)
                .enumerate()
            {
                hours[i] = u16::try_from(n(h)?).unwrap_or(u16::MAX);
            }
            let count = |k: &str| n(&p[k]).map(|c| u32::try_from(c).unwrap_or(u32::MAX));
            m.people.push(PersonMem {
                name: text(&p["name"], 64),
                agent: p["agent"].as_bool().unwrap_or(false),
                teammate: p["teammate"].as_bool().unwrap_or(false),
                met_at: n(&p["met_at"])?,
                seen_at: n(&p["seen_at"])?,
                known_at: n(&p["known_at"])?,
                times_seen: count("times_seen")?,
                armed_seen: p["armed_seen"].as_bool().unwrap_or(false),
                hits_on_me: count("hits_on_me")?,
                my_hits: count("my_hits")?,
                killed_me: count("killed_me")?,
                i_killed: count("i_killed")?,
                stance: p["stance"].as_str().and_then(Stance::parse),
                last_said: text(&p["last_said"], SAID_MAX),
                hours,
                hour_at: u64::MAX,
            });
        }
        for e in v["events"]
            .as_array()
            .ok_or("no events")?
            .iter()
            .rev()
            .take(EVENTS_MAX)
            .rev()
        {
            m.events.push_back(EventMem {
                at: n(&e["at"])?,
                what: text(&e["what"], EVENT_MAX),
            });
        }
        m.summary = text(&v["summary"], SUMMARY_MAX);
        if let Some(st) = v["strategy"].as_object() {
            m.strategy = Some(Strategy {
                ambition: text(&st["ambition"], STRATEGY_TEXT_MAX),
                project: text(&st["project"], STRATEGY_TEXT_MAX),
                standing_orders: st["standing_orders"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(STANDING_ORDERS_MAX)
                    .map(|o| text(o, STRATEGY_TEXT_MAX))
                    .collect(),
                set_at: st["set_at"].as_u64().unwrap_or(0),
            });
        }
        Ok(m)
    }

    /// Read `path`: a missing file is a fresh memory; a corrupt one is said
    /// once and a fresh memory too, since a body that forgets is better than
    /// one that never plays.
    pub fn load(path: &Path) -> Memory {
        use std::io::Read;
        let mut text = String::new();
        let read = std::fs::File::open(path)
            .and_then(|f| f.take(FILE_MAX_BYTES + 1).read_to_string(&mut text));
        match read {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Memory::default(),
            Err(e) => {
                eprintln!(
                    "memory: {} unreadable ({e}); starting fresh",
                    path.display()
                );
                return Memory::default();
            }
            Ok(_) => {}
        }
        let parsed = (text.len() as u64 <= FILE_MAX_BYTES)
            .then_some(())
            .ok_or_else(|| "too big".to_string())
            .and_then(|()| serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()))
            .and_then(|v| Memory::from_save(&v));
        match parsed {
            Ok(m) => {
                eprintln!(
                    "memory: {} places, {} people, {} events from {}",
                    m.places.len(),
                    m.people.len(),
                    m.events.len(),
                    path.display()
                );
                m
            }
            Err(e) => {
                // Set aside, not overwritten: what went wrong stays to look at.
                let mut aside = path.as_os_str().to_owned();
                aside.push(".corrupt");
                let _ = std::fs::rename(path, &aside);
                eprintln!(
                    "memory: {} is corrupt ({e}); set aside as .corrupt, starting fresh",
                    path.display()
                );
                Memory::default()
            }
        }
    }

    /// Write `path` whole and atomically: a temporary beside it, renamed
    /// over it, so a crash leaves the old memory or the new, never half.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, self.save_json().to_string())
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The distance and bearing sector (0 ahead, clockwise in eighths) of `to`
/// from a body at `at` facing `yaw`: `explorer::relative`'s rule.
fn relative(at: [f32; 2], yaw: u16, to: [f32; 2]) -> (f32, u8) {
    let (dx, dz) = (to[0] - at[0], to[1] - at[1]);
    let (fx, fz) = sim_core::yaw_dir(yaw);
    let (rx, rz) = sim_core::yaw_dir(yaw.wrapping_add(1 << 14));
    let ahead = dx * fx + dz * fz;
    let right = dx * rx + dz * rz;
    let angle = right.atan2(ahead).rem_euclid(std::f32::consts::TAU);
    let sector = ((angle / (std::f32::consts::TAU / 8.0)).round() as i32).rem_euclid(8);
    (dx.hypot(dz), sector as u8)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A [`DecisionSource`] with a memory and a character: every look is folded
/// into its [`Memory`], and the source it wraps is handed the body's
/// `character`, `memory` and `strategy` before it is asked
/// ([`DecisionSource::set_context`]). What the answer says beyond its goal
/// is kept (strategy, life story, stances) and passed on for the journal
/// ([`DecisionSource::take_extras`]). Runs on the decision worker only.
pub struct Remembering<S> {
    inner: S,
    memory: Memory,
    character: Option<Value>,
    file: Option<PathBuf>,
    saved: Option<Instant>,
    extras: Option<Extras>,
}

impl<S: DecisionSource> Remembering<S> {
    /// `file` (`--memory`) is read now and written as the body goes.
    pub fn new(inner: S, character: Option<&Character>, file: Option<&Path>) -> Self {
        Self {
            inner,
            memory: file.map(Memory::load).unwrap_or_default(),
            character: character.map(Character::json),
            file: file.map(Path::to_owned),
            saved: None,
            extras: None,
        }
    }

    /// The keys beside the observation: absent when empty.
    fn context(&self, s: &Summary, now: u64) -> Map<String, Value> {
        let mut ctx = Map::new();
        if let Some(c) = &self.character {
            ctx.insert("character".into(), c.clone());
        }
        if !self.memory.is_empty() {
            ctx.insert("memory".into(), self.memory.json(s, now));
        }
        if let Some(st) = self.memory.strategy() {
            ctx.insert("strategy".into(), st.json(now));
        }
        ctx
    }

    fn save(&mut self, now: Instant, force: bool) {
        let Some(path) = &self.file else {
            return;
        };
        if !force
            && self
                .saved
                .is_some_and(|t| now.duration_since(t) < SAVE_EVERY)
        {
            return;
        }
        self.saved = Some(now);
        if let Err(e) = self.memory.save(path) {
            eprintln!("memory: {e}");
        }
    }
}

impl<S: DecisionSource> DecisionSource for Remembering<S> {
    fn kind(&self) -> SourceKind {
        self.inner.kind()
    }

    fn decide(&mut self, summary: &Summary) -> Result<Choice, String> {
        self.decide_fresh(summary, &mut || None)
    }

    fn decide_fresh(
        &mut self,
        summary: &Summary,
        fresher: &mut dyn FnMut() -> Option<Summary>,
    ) -> Result<Choice, String> {
        let now = unix_now();
        self.memory.ingest(summary, now);
        let ctx = self.context(summary, now);
        self.inner.set_context(&ctx);
        let Self { inner, memory, .. } = self;
        let answer = inner.decide_fresh(summary, &mut || {
            let newer = fresher();
            if let Some(n) = &newer {
                memory.ingest(n, now);
            }
            newer
        });
        if let Some(x) = self.inner.take_extras() {
            self.memory.apply(&x, now);
            self.extras = Some(x);
        }
        self.save(Instant::now(), false);
        answer
    }

    fn take_note(&mut self) -> Option<crate::bugs::Note> {
        self.inner.take_note()
    }

    fn take_plan(&mut self) -> Option<Plan> {
        self.inner.take_plan()
    }

    /// The answer's thought and stances, with the strategy as the memory
    /// keeps it now (the newest a mind left, this run or one before), so a
    /// journal row always says what the body is set on.
    fn take_extras(&mut self) -> Option<Extras> {
        let mut x = self.extras.take().unwrap_or_default();
        x.strategy = self.memory.strategy().cloned();
        (!x.is_empty()).then_some(x)
    }

    fn plan_done(&self) -> u32 {
        self.inner.plan_done()
    }
}

impl<S> Drop for Remembering<S> {
    fn drop(&mut self) {
        if let Some(path) = &self.file {
            if let Err(e) = self.memory.save(path) {
                eprintln!("memory: {e}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mind::HeardLine;

    fn tag(name: &str) -> protocol::Name {
        protocol::Name::new(name).unwrap()
    }

    /// A memory filled to every cap, every text at its bound.
    pub(crate) fn full() -> Memory {
        let mut m = Memory::default();
        for i in 0..PLACES_MAX {
            m.place(
                PlaceKind::Danger,
                &"p".repeat(24),
                [i as f32 * 100.0, 0.0],
                5,
                &"n".repeat(EVENT_MAX),
            );
        }
        for i in 0..PEOPLE_MAX {
            let p = m.person(&format!("Person{i:010}"), 1);
            p.last_said = "s".repeat(SAID_MAX);
            p.hours = [u16::MAX; 24];
            p.stance = Some(Stance::Rival);
            p.seen_at = 1;
        }
        for _ in 0..EVENTS_MAX {
            m.event("e".repeat(EVENT_MAX), 3);
        }
        m.summary = "s".repeat(SUMMARY_MAX);
        m.strategy = Some(Strategy {
            ambition: "a".repeat(STRATEGY_TEXT_MAX),
            project: "p".repeat(STRATEGY_TEXT_MAX),
            standing_orders: vec!["o".repeat(STRATEGY_TEXT_MAX); STANDING_ORDERS_MAX],
            set_at: 2,
        });
        m
    }

    #[test]
    fn it_remembers_people_places_and_what_happened_and_shows_no_position() {
        let mut s = Summary::EMPTY;
        s.seen.pos = Some([1000.0, 2000.0]);
        s.seen.yaw = 0;
        let ash = s
            .seen
            .person(9, tag("Ash"), protocol::Address::GUEST, false)
            .unwrap();
        s.seen.people[ash].in_view = true;
        s.seen.people[ash].armed = true;
        s.seen.people[ash].hits_on_me = 2;
        s.seen.people[ash].killed_me = 1;
        s.seen.event(EventKind::Died, Some(ash));
        s.seen
            .place(PlaceKind::Danger, Name::EMPTY, [1000.0, 2100.0]);
        s.seen.place(
            PlaceKind::Gate,
            Name::new(b"THE GATE").unwrap(),
            [1234.5, 2345.5],
        );
        s.heard.push(HeardLine {
            from: 9,
            name: tag("Ash"),
            text: protocol::ChatText::sanitize(b"gg ez").unwrap(),
            ..HeardLine::EMPTY
        });
        let mut m = Memory::default();
        m.ingest(&s, 7200);
        let v = m.json(&s, 7230);
        let text = v.to_string();
        for banned in ["1234", "2345", "2100", "\"x\"", "\"z\"", "seed", "position"] {
            assert!(!text.contains(banned), "{banned} in {text}");
        }
        let ash = &v["people"][0];
        assert_eq!(ash["name"], "Ash");
        assert_eq!(ash["killed_me"], 1);
        assert_eq!(ash["armed_seen"], true);
        assert_eq!(ash["stance"], "enemy");
        assert_eq!(ash["last_said"], "gg ez");
        assert_eq!(ash["seen_ago_s"], 30);
        assert_eq!(ash["online_hours_utc"][2], 1);
        assert_eq!(v["events"][0]["what"], "I was killed by Ash");
        let danger = &v["places"][0];
        assert_eq!(danger["kind"], "danger");
        assert_eq!(danger["distance_m"], 100);
        assert!(danger["bearing"].is_string());
        // The mind's mark wins over the record, but adds nobody.
        let x = Extras::parse(&json!({"stance": [{"name": "Ash", "stance": "rival"},
            {"name": "Nobody", "stance": "friend"}],
            "strategy": {"ambition": "rule", "project": "a stone base", "standing_orders": ["kill Ash"], "summary": "I came ashore."},
            "thought": "Ash again."})).unwrap();
        m.apply(&x, 7300);
        let v = m.json(&s, 7300);
        assert_eq!(v["people"][0]["stance"], "rival");
        assert_eq!(v["people"].as_array().unwrap().len(), 1);
        assert_eq!(v["summary"], "I came ashore.");
        assert_eq!(m.strategy().unwrap().json(7310)["set_ago_s"], 10);
        // A home struck and a death the feed did not name are events too.
        let mut later = s;
        later.seen.clear();
        later.heard.clear();
        later.home.attacked = true;
        later.deaths = 1;
        m.ingest(&later, 7400);
        let what: Vec<&str> = m.events.iter().map(|e| e.what.as_str()).collect();
        assert_eq!(
            what,
            ["I was killed by Ash", "my home was attacked", "I died"]
        );
    }

    #[test]
    fn a_reply_s_extras_parse_strictly_and_are_cut_to_their_bounds() {
        let x = Extras::parse(&json!({"thought": "t".repeat(1000)})).unwrap();
        assert_eq!(x.thought.unwrap().len(), THOUGHT_MAX);
        assert!(Extras::parse(&json!({})).unwrap().is_empty());
        for bad in [
            json!({"thought": 5}),
            json!({"strategy": "win"}),
            json!({"strategy": {"standing_orders": ["a", "b", "c", "d", "e"]}}),
            json!({"strategy": {"ambition": 1}}),
            json!({"stance": [{"name": "Ash", "stance": "lover"}]}),
            json!({"stance": {"Ash": "enemy"}}),
        ] {
            assert!(Extras::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_memory_is_capped_saved_whole_and_survives_a_corrupt_file() {
        let mut m = full();
        m.place(PlaceKind::Loot, "one more", [9999.0, 0.0], 9, "");
        m.person("one more", 9);
        m.event("one more".into(), 9);
        assert_eq!(m.places.len(), PLACES_MAX);
        assert_eq!(m.people.len(), PEOPLE_MAX);
        assert_eq!(m.events.len(), EVENTS_MAX);
        assert_eq!(m.events.back().unwrap().what, "one more", "the newest kept");
        let dir = std::env::temp_dir().join(format!("gates-memory-{}", std::process::id()));
        let path = dir.join("body.json");
        m.save(&path).unwrap();
        let back = Memory::load(&path);
        assert_eq!(back.places.len(), PLACES_MAX);
        assert_eq!(back.people, {
            let mut p = m.people.clone();
            for q in &mut p {
                q.hour_at = u64::MAX;
            }
            p
        });
        assert_eq!(back.summary, m.summary);
        assert_eq!(back.strategy, m.strategy);
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size < FILE_MAX_BYTES, "{size}");
        std::fs::write(&path, "{\"format\": \"gates-memory-1\", \"places\": 7").unwrap();
        assert!(
            Memory::load(&path).is_empty(),
            "a corrupt file starts fresh"
        );
        assert!(Memory::load(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
