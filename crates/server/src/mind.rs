//! The agent's decision layer (PLAYERS.md, "the model does not drive at
//! frame rate"). A decision source picks a **goal** from a small vocabulary;
//! the local controller (`explorer.rs`) carries it out on client-received
//! state and reports how it went. The source is asked when a goal ends
//! (done, failed, interrupted) or on a heartbeat, never more often than
//! once per `jev::THINK_INTERVAL`, one request outstanding.
//!
//! Everything a source reads is [`Summary`]: a fixed-size, copyable value
//! built only from what this client has received (`explorer::observe`).
//! Serialization, HTTP and child-process I/O stay on the worker thread, so
//! the input loop neither waits nor allocates. Defaults: `DECISIONS.md`
//! §open, "Jev goals v0".

pub use crate::agent::build::Milestone;
use crate::agent::orders::{self, Order};
use protocol::MAX_ITEM_NAME_BYTES;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Ask again after a goal has run this long. Proposed default.
pub const JEV_HEARTBEAT_SECS: u64 = 30;
/// Spend guard: requests per rolling hour window and per day window.
/// At either ceiling the bot pauses model decisions visibly. Proposed.
pub const JEV_MAX_REQUESTS_HOUR: u32 = 600;
pub const JEV_MAX_REQUESTS_DAY: u32 = 7200;
/// Bytes kept of a decision's one-line reason. Proposed default.
pub const REASON_BYTES: usize = 96;
/// Finished goals the page and the report remember. Proposed default.
pub const GOAL_HISTORY: usize = 8;
/// Pack entries and craftable names a summary carries.
pub const SUMMARY_ITEMS: usize = 16;
pub const SUMMARY_CRAFTS: usize = 8;
/// Materials the next part of the base is short of, as a summary carries them.
pub const SUMMARY_NEEDS: usize = crate::agent::build::NEED_ROWS;
/// What its box holds, as a summary carries it.
pub const SUMMARY_STORED: usize = crate::agent::stash::STORED_ROWS;
/// What the team's boxes hold together, as a summary carries it.
pub const SUMMARY_HOME_HOLDS: usize = 12;
/// The raw materials the next milestone comes down to, as a summary
/// carries them.
pub const SUMMARY_RAW: usize = crate::agent::plan::RAW_ROWS;
/// The scripted policy goes home to put things away once the pack is down
/// to this many free slots.
pub const SCRIPTED_STASH_FREE_SLOTS: u8 = 4;
/// A raw material the pack already holds this much of is not gathered for
/// its own sake: a pack full of wood has no room for the pickaxe the next
/// stone makes. The base's needs ask for more by name.
pub const SCRIPTED_PLENTY: u32 = 3000;
/// A barrel or crate known this near (metres) is worth taking on the way,
/// whatever the base needs.
pub const SCRIPTED_LOOT_NEAR_M: u8 = 40;
/// What loot pays that the playbook goes for when the base is short of it:
/// barrels, crates and caches pay these, and before a furnace stands
/// nothing else pays fragments (`content/loot.toml`).
pub const LOOTED: [&str; 9] = [
    "Metal Fragments",
    "Low Grade Fuel",
    "Junk",
    "Rope",
    "Tarp",
    "Gears",
    "Animal Fat",
    "Cloth",
    "Sulfur",
];
/// The recycler, by catalog name (`agent::oven`), and the gears held before
/// one is worth making: two go into it, two are kept whole for the
/// revolver, the rest come apart in it.
pub const RECYCLER_ITEM: &str = crate::agent::oven::RECYCLER_ITEM;
const SCRIPTED_RECYCLER_GEARS: u32 = 5;
/// Ore, and what its furnace makes of it.
pub const SMELTS: [(&str, &str); 2] = [("Metal Ore", "Metal Fragments"), ("Sulfur Ore", "Sulfur")];
/// The deployable a body wakes on, by catalog name: what the playbook
/// crafts once it has stone tools.
pub const BAG_ITEM: &str = "Sleeping Bag";

/// What a raid plants, and how many the playbook keeps in the pack.
pub const SATCHEL_ITEM: &str = crate::agent::raid::SATCHEL_ITEM;
pub const SCRIPTED_SATCHELS: u32 = 4;
/// The scripted policy's "running low" line for food and water, percent of
/// the meter. Also the default for when a model is told a meter is low.
pub const SCRIPTED_LOW_METER_PCT: u32 = 40;
/// The scripted policy runs from a fight below this share of health (the
/// fight reflex answers one above it), and heals below the second when
/// nobody is about.
pub const SCRIPTED_FLEE_HP_PCT: u32 = 30;
pub const SCRIPTED_HEAL_HP_PCT: u32 = 60;

/// An item name as the wire catalog spells it, restricted to printable
/// ASCII so it can be shown, logged and sent without escaping surprises.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Name {
    bytes: [u8; MAX_ITEM_NAME_BYTES],
    len: u8,
}

impl Name {
    pub const EMPTY: Self = Self {
        bytes: [0; MAX_ITEM_NAME_BYTES],
        len: 0,
    };

    /// `None` for an empty, oversize or non-printable name — such an item
    /// is left out of every summary rather than sent escaped.
    pub fn new(raw: &[u8]) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_ITEM_NAME_BYTES {
            return None;
        }
        if !raw.iter().all(|b| (0x20..=0x7e).contains(b)) {
            return None;
        }
        let mut bytes = [0; MAX_ITEM_NAME_BYTES];
        bytes[..raw.len()].copy_from_slice(raw);
        Some(Self {
            bytes,
            len: raw.len() as u8,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn as_str(&self) -> &str {
        // Construction admits printable ASCII only.
        std::str::from_utf8(self.as_bytes()).unwrap_or("")
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

/// What a decision source may ask the body to do. Nothing here is a step:
/// each goal is a local skill that runs until it succeeds or fails.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Goal {
    Explore,
    GatherWood,
    GatherStone,
    GatherOre,
    /// Pick a berry bush: food and water.
    Forage,
    /// Pick hemp: cloth.
    GatherHemp,
    /// Craft one of the named item, resolved through the wire catalog and
    /// recipe table when it starts. Never an index.
    Craft(Name),
    Eat,
    Drink,
    Flee,
    Wait,
    /// Fight the nearest player in sight.
    Fight,
    /// Kill the nearest animal in sight.
    Hunt,
    /// Use meds from the pack.
    Heal,
    /// Put a sleeping bag down near here: somewhere to wake that is not
    /// the beach.
    Bag,
    /// Walk back to the backpack this body dropped where it died, and take
    /// what it held.
    Recover,
    /// Work on the base: choose a plot if there is none, then build the
    /// next milestone as far as the pack pays for it.
    Build,
    /// Walk home and in through its own doors, shutting them behind.
    GoHome,
    /// Go home and see to it: feed the cupboard when its stock runs low,
    /// put away what the belt does not keep, take out what the base needs.
    Stash,
    /// Go for the barrels, crates and caches it knows of, or the nearest
    /// place on the map that has them: smash, open, take.
    Loot,
    /// Cook the raw meat in the pack at a fire, putting one down if there
    /// is none near.
    Cook,
    /// Take salvaged parts apart at its recycler, putting it down if
    /// none stands near: junk, fragments and cloth.
    Recycle,
    /// Home is under attack or damaged: in through the doors, the fight
    /// met from inside, then the damage mended and the doors shut.
    Defend,
    /// Break into the weakest base it has seen that is worth it, with
    /// satchels or blows, and take what its boxes hold.
    Raid,
    /// Say yes to the team invite waiting on it (wire v108).
    JoinTeam,
    /// Invite the person beside it to its team: the invite a player sends
    /// with N (wire v108). Offered while it has no team or leads one with
    /// room, and a person who is not an agent stands within reach.
    Invite,
    /// Stay with its nearest teammate, and fight whoever goes for them.
    Follow,
    /// Its friend (`--friend`) is on the island and not on its team: walk
    /// to THE GATE and wait there, the place they know to find it, until
    /// they are in reach of an invite.
    Meet,
    /// Take what the pack carries past the belt to the team's chest, and
    /// put it in.
    Deliver,
    /// A supply run: gather the kind a few minutes, then deliver it.
    SupplyWood,
    SupplyStone,
    SupplyOre,
    SupplyHemp,
    /// Buy the named item with junk at THE GATE's stall that sells it.
    Buy(Name),
    /// Sell the named item for junk at THE GATE's SALVAGE stall.
    Sell(Name),
    /// Take what the nearest open work takes to it and put it in, or fuel
    /// a lit one (`ARC.md`).
    FeedWork,
    /// Walk to the nearest stone it has not read and read it: it teaches
    /// the glyphs it names, and some hold a lock's answer (`ARC.md` F6).
    ReadStone,
    /// Turn a lock's dials to the answer a stone gave it; the last turn
    /// pays (`ARC.md` F7).
    SolveLock,
    /// Swipe the keycard it carries at its door in the Black Ziggurat and
    /// walk into the room behind it.
    SwipeCard,
}

/// `craft:` plus the longest catalog name.
pub const LABEL_BYTES: usize = 6 + MAX_ITEM_NAME_BYTES;

/// A goal's wire spelling in fixed storage, e.g. `gather_wood` or
/// `craft:Stone Hatchet`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Label {
    bytes: [u8; LABEL_BYTES],
    len: u8,
}

impl Label {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl std::fmt::Debug for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Goal {
    /// Every goal but the per-item craft, in the order they are offered.
    pub const FIXED: [Goal; 36] = [
        Goal::Explore,
        Goal::GatherWood,
        Goal::GatherStone,
        Goal::GatherOre,
        Goal::Forage,
        Goal::GatherHemp,
        Goal::Eat,
        Goal::Drink,
        Goal::Flee,
        Goal::Wait,
        Goal::Fight,
        Goal::Hunt,
        Goal::Heal,
        Goal::Bag,
        Goal::Recover,
        Goal::Build,
        Goal::GoHome,
        Goal::Stash,
        Goal::Loot,
        Goal::Cook,
        Goal::Recycle,
        Goal::Defend,
        Goal::Raid,
        Goal::JoinTeam,
        Goal::Invite,
        Goal::Follow,
        Goal::Meet,
        Goal::Deliver,
        Goal::SupplyWood,
        Goal::SupplyStone,
        Goal::SupplyOre,
        Goal::SupplyHemp,
        Goal::FeedWork,
        Goal::ReadStone,
        Goal::SolveLock,
        Goal::SwipeCard,
    ];

    /// The vocabulary's base keys, for documentation and tests.
    pub fn key(self) -> &'static str {
        match self {
            Goal::Explore => "explore",
            Goal::GatherWood => "gather_wood",
            Goal::GatherStone => "gather_stone",
            Goal::GatherOre => "gather_ore",
            Goal::Forage => "forage",
            Goal::GatherHemp => "gather_hemp",
            Goal::Craft(_) => "craft",
            Goal::Eat => "eat",
            Goal::Drink => "drink",
            Goal::Flee => "flee",
            Goal::Wait => "wait",
            Goal::Fight => "fight",
            Goal::Hunt => "hunt",
            Goal::Heal => "heal",
            Goal::Bag => "bag",
            Goal::Recover => "recover",
            Goal::Build => "build",
            Goal::GoHome => "go_home",
            Goal::Stash => "stash",
            Goal::Loot => "loot",
            Goal::Cook => "cook",
            Goal::Recycle => "recycle",
            Goal::Defend => "defend",
            Goal::Raid => "raid",
            Goal::JoinTeam => "join_team",
            Goal::Invite => "invite",
            Goal::Follow => "follow",
            Goal::Meet => "meet",
            Goal::Deliver => "deliver",
            Goal::SupplyWood => "supply_wood",
            Goal::SupplyStone => "supply_stone",
            Goal::SupplyOre => "supply_ore",
            Goal::SupplyHemp => "supply_hemp",
            Goal::Buy(_) => "buy",
            Goal::Sell(_) => "sell",
            Goal::FeedWork => "feed_work",
            Goal::ReadStone => "read_stone",
            Goal::SolveLock => "solve_lock",
            Goal::SwipeCard => "swipe_card",
        }
    }

    pub fn label(self) -> Label {
        let mut bytes = [0u8; LABEL_BYTES];
        let mut len = 0;
        let mut put = |s: &[u8]| {
            let n = s.len().min(LABEL_BYTES - len);
            bytes[len..len + n].copy_from_slice(&s[..n]);
            len += n;
        };
        put(self.key().as_bytes());
        if let Goal::Craft(name) | Goal::Buy(name) | Goal::Sell(name) = self {
            put(b":");
            put(name.as_bytes());
        }
        Label {
            bytes,
            len: len as u8,
        }
    }

    /// A supply run: gather, then fill the team's chest.
    pub fn supplies(self) -> bool {
        matches!(
            self,
            Goal::SupplyWood | Goal::SupplyStone | Goal::SupplyOre | Goal::SupplyHemp
        )
    }

    /// Strict: a label must name one of the goals actually offered.
    pub fn parse(label: &str, offered: &[Goal]) -> Option<Goal> {
        offered
            .iter()
            .copied()
            .find(|g| g.label().as_str() == label)
    }

    /// What the goal means, in the words a source is given. Worker only.
    pub fn describe(self) -> String {
        match self {
            Goal::Explore => "Walk somewhere new.".into(),
            Goal::GatherWood => "Chop a tree in view; wood makes tools.".into(),
            Goal::GatherStone => "Mine a stone node in view.".into(),
            Goal::GatherOre => "Mine a metal or sulfur node in view.".into(),
            Goal::Forage => "Pick a berry bush in view: berries for food and water.".into(),
            Goal::GatherHemp => "Pick hemp in view; cloth makes bags and bandages.".into(),
            Goal::Craft(name) => format!("Craft one {}.", name.as_str()),
            Goal::Eat => "Eat from the pack until food is mostly full.".into(),
            Goal::Drink => "Drink until water is mostly full: juicy food, else the sea (costs health).".into(),
            Goal::Flee => "Run from the players or animals in view.".into(),
            Goal::Wait => "Stand still a few seconds.".into(),
            Goal::Fight => "Fight the nearest player in view; backs off if losing.".into(),
            Goal::Hunt => "Kill the nearest animal in view for meat and hide.".into(),
            Goal::Heal => "Use bandages or a medkit.".into(),
            Goal::Bag => "Put a sleeping bag down to wake beside: here, or on the team's floor if you live there.".into(),
            Goal::Recover => "Walk back to the death backpack and take what it holds.".into(),
            Goal::Build => "Build the next part of the base from the pack; choose a plot if none.".into(),
            Goal::GoHome => "Walk home and in, shutting the doors.".into(),
            Goal::Stash => {
                "Go home: feed the cupboard, box what the belt does not need, take out what the base needs.".into()
            }
            Goal::Loot => "Smash known barrels and empty crates, or go to the nearest loot place.".into(),
            Goal::Cook => "Cook raw meat at a fire, putting one down if none is near.".into(),
            Goal::Recycle => "Recycle the salvaged parts for junk.".into(),
            Goal::Defend => "Home is attacked or damaged: get inside, fight from there, then repair it.".into(),
            Goal::Raid => "Break into the weakest base worth it and take what its boxes hold.".into(),
            Goal::JoinTeam => "Accept the team invite.".into(),
            Goal::Invite => "Invite the player beside you to your team.".into(),
            Goal::Follow => "Stay with a teammate; fight their attackers.".into(),
            Goal::Meet => {
                "Walk to THE GATE and wait for your friend there, then team up.".into()
            }
            Goal::Deliver => "Put the pack in the team's box.".into(),
            Goal::SupplyWood => "Chop wood, then put it in the team's box.".into(),
            Goal::SupplyStone => "Mine stone, then put it in the team's box.".into(),
            Goal::SupplyOre => "Mine ore, then put it in the team's box.".into(),
            Goal::SupplyHemp => "Pick hemp, then put it in the team's box.".into(),
            Goal::Buy(name) => format!("Buy {} with junk at the gate.", name.as_str()),
            Goal::Sell(name) => format!("Sell {} for junk at the gate.", name.as_str()),
            Goal::FeedWork => "Give the nearest open work what it needs, or fuel a lit one.".into(),
            Goal::ReadStone => "Read the nearest unread stone: learn its glyphs; some give a lock's answer.".into(),
            Goal::SolveLock => "Turn a lock's dials to the answer a stone gave; the last turn pays.".into(),
            Goal::SwipeCard => "Swipe your keycard at its ziggurat door and walk into the room.".into(),
        }
    }
}

/// Why a goal stopped, or stopped early. Integer-like and copyable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    NoTool,
    NotFound,
    Stuck,
    PackFull,
    NoRecipe,
    MissingInputs,
    Refused,
    NoFood,
    NoWater,
    NoAnswer,
    TooHurt,
    Hit,
    Died,
    Wounded,
    Replaced,
    Fight,
    Won,
    Lost,
    Escaped,
    NoMeds,
    Hostile,
    NoSpot,
    /// The session is ending: home, to sleep behind shut doors.
    LogOff,
    /// A lock in the way, and no code for it.
    Locked,
    BoxFull,
    /// Marked hostile: the gate's guns would shoot it.
    Wanted,
}

impl Why {
    pub fn text(self) -> &'static str {
        match self {
            Why::NoTool => "no working tool in the belt",
            Why::NotFound => "none in sight",
            Why::Stuck => "could not reach a target",
            Why::PackFull => "the pack is full",
            Why::NoRecipe => "no recipe this player can use",
            Why::MissingInputs => "not enough materials",
            Why::Refused => "the server refused it",
            Why::NoFood => "nothing edible in the pack",
            Why::NoWater => "no water in reach or in sight",
            Why::NoAnswer => "no answer from the server",
            Why::TooHurt => "too hurt to drink sea water",
            Why::Hit => "took a hit",
            Why::Died => "died",
            Why::Wounded => "went down wounded",
            Why::Replaced => "a new goal replaced it",
            Why::Fight => "a fight outlasted it",
            Why::Won => "won the fight",
            Why::Lost => "lost the fight",
            Why::Escaped => "got away from a losing fight",
            Why::NoMeds => "no bandage or medkit in the pack",
            Why::Hostile => "someone dangerous is in view",
            Why::NoSpot => "no spot here takes it",
            Why::LogOff => "the session is ending, so it went home",
            Why::Locked => "a lock it has no code for is in the way",
            Why::BoxFull => "the box is full",
            Why::Wanted => "it attacked a player lately, and the gate's guns would shoot it",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Running,
    Done,
    Failed(Why),
    Interrupted(Why),
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Running => "running",
            Outcome::Done => "done",
            Outcome::Failed(_) => "failed",
            Outcome::Interrupted(_) => "interrupted",
        }
    }

    pub fn why(self) -> Option<Why> {
        match self {
            Outcome::Failed(w) | Outcome::Interrupted(w) => Some(w),
            _ => None,
        }
    }
}

/// One goal as it ran. `gained` is units received (gathered, crafted,
/// eaten or drunk); `secs` is play time spent on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Report {
    pub goal: Goal,
    pub outcome: Outcome,
    pub gained: u32,
    pub secs: u32,
}

/// What prompted a request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Trigger {
    #[default]
    Start,
    Completed,
    Failed,
    Interrupted,
    Respawned,
    Heartbeat,
    Fought,
    /// Someone near it spoke (an external source only).
    Heard,
    /// Its team changed: an invite arrived, or someone joined or left.
    Team,
}

impl Trigger {
    pub fn word(self) -> &'static str {
        match self {
            Trigger::Start => "start",
            Trigger::Completed => "completed",
            Trigger::Failed => "failed",
            Trigger::Interrupted => "interrupted",
            Trigger::Respawned => "respawned",
            Trigger::Heartbeat => "heartbeat",
            Trigger::Fought => "fought",
            Trigger::Heard => "heard",
            Trigger::Team => "team",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BodyState {
    #[default]
    Alive,
    Wounded,
    Dead,
}

/// Its team (wire v108), as its client was told: how many others are on
/// it, where the nearest awake one is, how many are down, whether it leads,
/// and whether an invite waits on it. Labels for the people go to an
/// external source only ([`Summary::team_json`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TeamSense {
    pub mates: u8,
    /// Awake teammates: how many, and the nearest's distance and bearing.
    pub awake: Sighting,
    pub down: u8,
    pub leader: bool,
    pub invited: bool,
    /// A stranger is fighting near a teammate.
    pub mate_attacked: bool,
    /// The team's base, its shared home, if its eyes have seen a teammate
    /// at a box: where it is from here, how many of its boxes it knows,
    /// and whether it is under attack (`agent::supply`).
    pub home: Sighting,
    pub home_boxes: u8,
    pub home_attacked: bool,
    /// It lives there (no base of its own), and its boxes hold something
    /// the pack lacks (food, meds, a better tool).
    pub lives_there: bool,
    pub home_has_needs: bool,
    /// A teammate gave the team's lock code; the team's cupboard has it on
    /// its crew (the code entered there); its own bag is down in the base.
    pub code: bool,
    pub on_cupboard: bool,
    pub bag_there: bool,
    /// Whose invite waits, and the teammates, as a chat log names them.
    pub invite_from: Who,
    pub who: [Who; protocol::TEAM_MAX],
    pub who_len: u8,
}

/// A player as a chat log names them: name, else short address, else id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Who {
    pub id: u32,
    pub name: protocol::Name,
    pub address: protocol::Address,
}

impl Default for Who {
    fn default() -> Self {
        Self {
            id: 0,
            name: protocol::Name::EMPTY,
            address: protocol::Address::GUEST,
        }
    }
}

impl Who {
    pub fn label(&self) -> String {
        HeardLine {
            from: self.id,
            name: self.name,
            address: self.address,
            ..HeardLine::EMPTY
        }
        .label()
    }
}

impl TeamSense {
    fn json(&self) -> Value {
        json!({
            "mates": self.mates,
            "nearest": self.awake.json(),
            "down": self.down,
            "lead": self.leader,
            "invited": self.invited,
            "mate_attacked": self.mate_attacked,
            "home": self.home.json(),
            "home_boxes": self.home_boxes,
            "home_attacked": self.home_attacked,
            "i_live_there": self.lives_there,
            "home_has_what_i_need": self.home_has_needs,
            "code_known": self.code,
            "on_cupboard": self.on_cupboard,
            "my_bag_there": self.bag_there,
        })
    }
}

/// Its friend (`jev-bot --friend 0x…`): the one wallet this body plays
/// with. Whether they are on the island, on its team, in sight, in reach of
/// an invite, or inviting it. Sent to an external source beside the
/// observation, as `team` is ([`Summary::friend_json`]), never to Jev.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FriendSense {
    /// A friend is named at all.
    pub set: bool,
    /// Who, as a chat log names them; `id` 0 until they joined.
    pub who: Who,
    /// On the island now: awake on its team, or (before a team) joined or
    /// seen recently (`explorer::FRIEND_AWAY_SECS`).
    pub online: bool,
    pub on_team: bool,
    /// Where they stand, while its eyes have them.
    pub in_sight: Sighting,
    /// They are the person an `invite` would go to now.
    pub reachable: bool,
    /// The invite waiting on it is theirs.
    pub invited_me: bool,
}

impl FriendSense {
    pub const NONE: Self = Self {
        set: false,
        who: Who {
            id: 0,
            name: protocol::Name::EMPTY,
            address: protocol::Address::GUEST,
        },
        online: false,
        on_team: false,
        in_sight: Sighting::NONE,
        reachable: false,
        invited_me: false,
    };
}

/// Relative directions, clockwise from straight ahead.
pub const BEARINGS: [&str; 8] = [
    "ahead",
    "ahead-right",
    "right",
    "behind-right",
    "behind",
    "behind-left",
    "left",
    "ahead-left",
];

/// A work (`ARC.md`) as its client knows it: its name, whether it is
/// sealed, open or lit, where its terminal is, what its quota still takes
/// while open, its fuel and the room in its tank while lit, and what
/// lighting it unlocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkSense {
    pub name: Name,
    pub state: u8,
    pub at: Option<Place>,
    pub needs: [(Name, u32); sim_core::limits::MAX_WORK_INPUTS],
    pub needs_len: u8,
    pub fuel: Name,
    pub fuel_room: u32,
    pub unlocks: Name,
}

impl WorkSense {
    pub const EMPTY: Self = Self {
        name: Name::EMPTY,
        state: 0,
        at: None,
        needs: [(Name::EMPTY, 0); sim_core::limits::MAX_WORK_INPUTS],
        needs_len: 0,
        fuel: Name::EMPTY,
        fuel_room: 0,
        unlocks: Name::EMPTY,
    };

    fn json(&self) -> Value {
        json!({
            "name": self.name.as_str(),
            "state": match self.state {
                sim_core::works::WORK_OPEN => "open: takes deposits",
                sim_core::works::WORK_LIT => "lit: burns fuel",
                _ => "sealed",
            },
            "where": self.at.map(|p| p.json()),
            "still_needs": self.needs[..usize::from(self.needs_len)].iter().map(|(n, c)| json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
            "fuel": (self.state == sim_core::works::WORK_LIT).then(|| json!({"name": self.fuel.as_str(), "room": self.fuel_room})),
            "unlocks": self.unlocks.as_str(),
        })
    }
}

/// A lock (`ARC.md` F7) as its client knows it: where it is, how many
/// dials, whether it rests after a solve, and whether a stone it read
/// gave the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockSense {
    pub name: Name,
    pub at: Option<Place>,
    pub dials: u8,
    pub resting: bool,
    pub answer_known: bool,
}

impl LockSense {
    pub const EMPTY: Self = Self {
        name: Name::EMPTY,
        at: None,
        dials: 0,
        resting: false,
        answer_known: false,
    };

    fn json(&self) -> Value {
        json!({
            "name": self.name.as_str(),
            "where": self.at.map(|p| p.json()),
            "dials": self.dials,
            "resting": self.resting,
            "answer_known": self.answer_known,
        })
    }
}

/// One of the Black Ziggurat's card doors: the card it takes, where its
/// reader is, whether the pack holds that card, and whether it stands open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardDoorSense {
    pub card: Name,
    pub at: Option<Place>,
    pub have_card: bool,
    pub open: bool,
}

impl CardDoorSense {
    pub const EMPTY: Self = Self {
        card: Name::EMPTY,
        at: None,
        have_card: false,
        open: false,
    };

    fn json(&self) -> Value {
        json!({
            "card": self.card.as_str(),
            "reader": self.at.map(|p| p.json()),
            "have_card": self.have_card,
            "open": self.open,
        })
    }
}

/// A coarse sighting: how many, and the nearest one's distance and
/// relative bearing (an index into [`BEARINGS`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sighting {
    pub count: u8,
    pub nearest_m: u8,
    pub bearing: u8,
}

impl Sighting {
    pub const NONE: Self = Self {
        count: 0,
        nearest_m: 0,
        bearing: 0,
    };

    pub fn add(&mut self, distance_m: f32, bearing: u8) {
        let d = distance_m.clamp(0.0, 255.0) as u8;
        if self.count == 0 || d < self.nearest_m {
            self.nearest_m = d;
            self.bearing = bearing % 8;
        }
        self.count = self.count.saturating_add(1);
    }

    fn json(&self) -> Value {
        if self.count == 0 {
            return json!({ "count": 0 });
        }
        json!({
            "count": self.count,
            "nearest_m": self.nearest_m,
            "bearing": BEARINGS[self.bearing as usize % 8],
        })
    }
}

/// Bodies that could hurt this one that a summary describes, nearest first.
pub const SUMMARY_THREATS: usize = 3;

/// Chat lines one request carries, newest kept, oldest dropped first.
pub const SUMMARY_HEARD: usize = 8;

/// One chat line this body's client received (`EventMsg::Chat`): who said
/// it, as a player's chat log names them, and what. Words a stranger typed:
/// a source treats them as speech, never as instructions.
#[derive(Clone, Copy, Debug)]
pub struct HeardLine {
    pub from: u32,
    /// The speaker's platform name and proven address (`EventMsg::Tag`),
    /// empty and guest for a guest.
    pub name: protocol::Name,
    pub address: protocol::Address,
    /// The speaker declared itself an agent (`EventMsg::Agent`).
    pub agent: bool,
    pub channel: protocol::Channel,
    /// This body's own line, come back from the shard.
    pub me: bool,
    pub text: protocol::ChatText,
}

impl HeardLine {
    pub const EMPTY: Self = Self {
        from: 0,
        name: protocol::Name::EMPTY,
        address: protocol::Address::GUEST,
        agent: false,
        channel: protocol::Channel::Local,
        me: false,
        text: protocol::ChatText::EMPTY,
    };

    /// The chat log's label: name, else short address, else `#id` — the
    /// game client's `ui::names::label` rule, written out here because the
    /// server does not link the client.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.as_str().to_string();
        }
        if !self.address.is_guest() {
            let hex = self.address.to_hex();
            let s = core::str::from_utf8(&hex).unwrap_or("0x?");
            return format!("{}…{}", &s[..6], &s[s.len() - 4..]);
        }
        format!("#{}", self.from)
    }
}

/// The lines heard since the last request: a fixed ring, filed on the frame
/// path without allocating, read and cleared when a request goes out.
#[derive(Clone, Copy, Debug)]
pub struct Heard {
    lines: [HeardLine; SUMMARY_HEARD],
    len: u8,
}

impl Heard {
    pub const EMPTY: Self = Self {
        lines: [HeardLine::EMPTY; SUMMARY_HEARD],
        len: 0,
    };

    /// File a line; a full ring drops its oldest.
    pub fn push(&mut self, line: HeardLine) {
        let len = self.len as usize;
        if len == SUMMARY_HEARD {
            self.lines.rotate_left(1);
            self.lines[SUMMARY_HEARD - 1] = line;
        } else {
            self.lines[len] = line;
            self.len += 1;
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Oldest first.
    pub fn lines(&self) -> &[HeardLine] {
        &self.lines[..self.len as usize]
    }
}

impl Default for Heard {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// What a decision source has its body say out loud: one local chat line,
/// sent with the human chat box's own encoder. An external source says
/// what it likes; the scripted policy only answers its team's orders with
/// fixed lines (`agent::orders`); Jev is a classifier and stays quiet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Say {
    #[default]
    Quiet,
    Line(protocol::ChatText),
    /// A line for this body's team (wire v108), wherever they are. Said
    /// locally when it has no team.
    Team(protocol::ChatText),
    /// Asked for a line the chat rule refuses (too long, a control
    /// character, a `/command`): counted, never repaired.
    Refused,
}

/// What a body in sight is holding, as a threat: never which item.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Arms {
    #[default]
    Unarmed,
    Tool,
    Melee,
    Ranged,
    Explosive,
    /// An animal's bite.
    Teeth,
}

impl Arms {
    pub fn word(self) -> &'static str {
        match self {
            Arms::Unarmed => "unarmed",
            Arms::Tool => "tool",
            Arms::Melee => "melee",
            Arms::Ranged => "ranged",
            Arms::Explosive => "explosive",
            Arms::Teeth => "teeth",
        }
    }
}

/// Where home stands for this body.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HomeState {
    /// No plot chosen.
    #[default]
    None,
    /// A plot chosen, no cupboard standing on it yet.
    Plot,
    /// Its cupboard stands.
    Built,
    /// Built, and this body is inside it.
    Inside,
}

impl HomeState {
    pub fn word(self) -> &'static str {
        match self {
            HomeState::None => "none",
            HomeState::Plot => "plot_chosen",
            HomeState::Built => "built",
            HomeState::Inside => "inside",
        }
    }
}

/// How far off, coarsely.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Range {
    /// Within a spear's reach or two.
    #[default]
    Close,
    Near,
    Mid,
    Far,
}

impl Range {
    pub fn of(m: f32) -> Range {
        if m < 5.0 {
            Range::Close
        } else if m < 20.0 {
            Range::Near
        } else if m < 60.0 {
            Range::Mid
        } else {
            Range::Far
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Range::Close => "close",
            Range::Near => "near",
            Range::Mid => "mid",
            Range::Far => "far",
        }
    }
}

/// One body in sight that could hurt this one, anonymous: what it holds,
/// how far, which way (an index into [`BEARINGS`]), whether it is badly
/// hurt, and whether it is aiming at me.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Threat {
    pub arms: Arms,
    pub range: Range,
    pub bearing: u8,
    pub wounded: bool,
    pub aiming_at_me: bool,
}

impl Threat {
    fn json(&self) -> Value {
        json!({
            "arms": self.arms.word(),
            "range": self.range.word(),
            "bearing": BEARINGS[self.bearing as usize % 8],
            "wounded": self.wounded,
            "aiming_at_me": self.aiming_at_me,
        })
    }
}

/// A coarse distance: here (under 10 m), near (50), mid (200), far.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Distance {
    #[default]
    Here,
    Near,
    Mid,
    Far,
}

impl Distance {
    pub fn of(metres: f32) -> Self {
        if metres < 10.0 {
            Distance::Here
        } else if metres < 50.0 {
            Distance::Near
        } else if metres < 200.0 {
            Distance::Mid
        } else {
            Distance::Far
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Distance::Here => "here",
            Distance::Near => "near",
            Distance::Mid => "mid",
            Distance::Far => "far",
        }
    }
}

/// Home as a summary carries it: no position, a relative bearing (an index
/// into [`BEARINGS`]) and a distance band.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HomeSense {
    pub state: HomeState,
    pub distance: Distance,
    pub bearing: u8,
    /// Blows, breaks or blasts at home a moment ago.
    pub attacked: bool,
    /// Something of it is damaged that a repair would mend.
    pub damaged: bool,
}

impl HomeSense {
    fn json(&self) -> Value {
        if self.state == HomeState::None {
            return json!({ "state": "none" });
        }
        json!({
            "state": self.state.word(),
            "distance": self.distance.word(),
            "bearing": BEARINGS[self.bearing as usize % 8],
            "under_attack": self.attacked,
            "damaged": self.damaged,
        })
    }
}

/// A place on the map, as a summary carries it: a distance band and a
/// relative bearing (an index into [`BEARINGS`]), never where.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Place {
    pub distance: Distance,
    pub bearing: u8,
}

impl Place {
    fn json(&self) -> Value {
        json!({
            "distance": self.distance.word(),
            "bearing": BEARINGS[self.bearing as usize % 8],
        })
    }
}

/// The scripted policy takes the pack to the team's chest with this few
/// slots free.
pub const SCRIPTED_DELIVER_FREE: u8 = 4;
/// The scripted policy catches up with a teammate farther than this.
pub const SCRIPTED_FOLLOW_M: f32 = 25.0;

/// The most goals one request can offer: the fixed set and every craft.
pub const MAX_OPTIONS: usize = Goal::FIXED.len() + SUMMARY_CRAFTS + crate::agent::gate::TRADE_ROWS;

/// Everything a decision source may know, as a fixed-size value. Built only
/// from this client's received state (`explorer::observe`); it carries no
/// seed, no absolute position and no identity of another body beyond the
/// label a player's own chat log shows, in `heard` — which only an
/// external source is sent ([`Summary::heard_json`]), never Jev.
#[derive(Clone, Copy, Debug)]
pub struct Summary {
    pub tick: u32,
    pub body: BodyState,
    pub hp: u16,
    pub hp_max: u16,
    pub food: u16,
    pub food_max: u16,
    pub water: u16,
    pub water_max: u16,
    pub items: [(Name, u32); SUMMARY_ITEMS],
    pub items_len: u8,
    pub free_slots: u8,
    pub craftable: [Name; SUMMARY_CRAFTS],
    pub craftable_len: u8,
    pub trees: Sighting,
    pub stone_nodes: Sighting,
    pub ore_nodes: Sighting,
    /// Berry bushes; hemp is `hemp`.
    pub bushes: Sighting,
    pub hemp: Sighting,
    pub players: Sighting,
    pub animals: Sighting,
    pub water_near: Sighting,
    pub threats: [Threat; SUMMARY_THREATS],
    pub threats_len: u8,
    /// How the last fight went: won, lost or escaped.
    pub last_fight: Option<Why>,
    pub last: Option<Report>,
    /// The goal still running when this was asked (a heartbeat), with what
    /// it has gained so far — so a source can let it finish.
    pub current: Option<Report>,
    pub hits: u16,
    /// Chat lines since the last request, its own echoed back included.
    pub heard: Heard,
    pub deaths: u32,
    pub respawns: u32,
    /// Sleeping bags this body knows it has down.
    pub bags: u8,
    /// The backpack it dropped at its last death, while it stands.
    pub backpack: Sighting,
    /// Its base, if it has one.
    pub home: HomeSense,
    /// One of its sleeping bags would take it now.
    pub bag_ready: bool,
    pub night: bool,
    /// The next part of the base, and what the pack is short of for it.
    pub milestone: Milestone,
    pub needs: [(Name, u32); SUMMARY_NEEDS],
    pub needs_len: u8,
    /// What those come down to that nobody crafts here: to gather, hunt
    /// or loot.
    pub raw: [(Name, u32); SUMMARY_RAW],
    pub raw_len: u8,
    /// Its own workbench and furnace stand.
    pub bench: bool,
    pub furnace: bool,
    /// Barrels and crates it knows of and has not emptied.
    pub loot: Sighting,
    /// The nearest place on the map with crates it has not found bare.
    pub loot_place: Option<Place>,
    /// Bases it has seen that are worth raiding now: how many, and the
    /// nearest's distance and bearing.
    pub raid: Sighting,
    /// What its box held when last opened.
    pub stored: [(Name, u32); SUMMARY_STORED],
    pub stored_len: u8,
    /// The box holds something the pack is short of for the base.
    pub take_out: bool,
    /// The cupboard's stock is running low, and the pack can feed it.
    pub feed: bool,
    /// Its team (wire v108).
    pub team: TeamSense,
    /// Its friend (`--friend`), if it has one.
    pub friend: FriendSense,
    /// What the team's boxes held when their panels were last open.
    pub home_holds: [(Name, u32); SUMMARY_HOME_HOLDS],
    pub home_holds_len: u8,
    /// THE GATE, while the town stands, and the works its client knows.
    pub gate: Option<Place>,
    pub works: [WorkSense; sim_core::limits::MAX_WORKS],
    pub works_len: u8,
    /// The ancients' stones (how many it has read of those it knows of,
    /// the nearest unread one in walking range, the glyphs it reads of the
    /// alphabet), their locks, and the ziggurat's card doors.
    pub stones_read: u8,
    pub stones_known: u8,
    pub unread_stone: Option<Place>,
    pub glyphs: u8,
    pub alphabet: u8,
    pub locks: [LockSense; sim_core::limits::MAX_MECHS],
    pub locks_len: u8,
    pub card_doors: [CardDoorSense; sim_core::monument::CARD_DOORS],
    pub card_doors_len: u8,
    pub trigger: Trigger,
    pub options: [Goal; MAX_OPTIONS],
    pub options_len: u8,
}

impl Summary {
    pub const EMPTY: Self = Self {
        tick: 0,
        body: BodyState::Alive,
        hp: 0,
        hp_max: 0,
        food: 0,
        food_max: 0,
        water: 0,
        water_max: 0,
        items: [(Name::EMPTY, 0); SUMMARY_ITEMS],
        items_len: 0,
        free_slots: 0,
        craftable: [Name::EMPTY; SUMMARY_CRAFTS],
        craftable_len: 0,
        trees: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        stone_nodes: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        ore_nodes: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        bushes: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        hemp: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        players: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        animals: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        water_near: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        threats: [Threat {
            arms: Arms::Unarmed,
            range: Range::Close,
            bearing: 0,
            wounded: false,
            aiming_at_me: false,
        }; SUMMARY_THREATS],
        threats_len: 0,
        last_fight: None,
        last: None,
        current: None,
        hits: 0,
        heard: Heard::EMPTY,
        deaths: 0,
        respawns: 0,
        bags: 0,
        backpack: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        home: HomeSense {
            state: HomeState::None,
            distance: Distance::Here,
            bearing: 0,
            attacked: false,
            damaged: false,
        },
        bag_ready: false,
        night: false,
        milestone: Milestone::Shell,
        needs: [(Name::EMPTY, 0); SUMMARY_NEEDS],
        needs_len: 0,
        raw: [(Name::EMPTY, 0); SUMMARY_RAW],
        raw_len: 0,
        bench: false,
        furnace: false,
        loot: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        loot_place: None,
        raid: Sighting {
            count: 0,
            nearest_m: 0,
            bearing: 0,
        },
        stored: [(Name::EMPTY, 0); SUMMARY_STORED],
        stored_len: 0,
        take_out: false,
        feed: false,
        team: TeamSense {
            mates: 0,
            awake: Sighting {
                count: 0,
                nearest_m: 0,
                bearing: 0,
            },
            down: 0,
            leader: false,
            invited: false,
            mate_attacked: false,
            home: Sighting {
                count: 0,
                nearest_m: 0,
                bearing: 0,
            },
            home_boxes: 0,
            home_attacked: false,
            lives_there: false,
            home_has_needs: false,
            code: false,
            on_cupboard: false,
            bag_there: false,
            invite_from: Who {
                id: 0,
                name: protocol::Name::EMPTY,
                address: protocol::Address::GUEST,
            },
            who: [Who {
                id: 0,
                name: protocol::Name::EMPTY,
                address: protocol::Address::GUEST,
            }; protocol::TEAM_MAX],
            who_len: 0,
        },
        friend: FriendSense::NONE,
        home_holds: [(Name::EMPTY, 0); SUMMARY_HOME_HOLDS],
        home_holds_len: 0,
        gate: None,
        works: [WorkSense::EMPTY; sim_core::limits::MAX_WORKS],
        works_len: 0,
        stones_read: 0,
        stones_known: 0,
        unread_stone: None,
        glyphs: 0,
        alphabet: 0,
        locks: [LockSense::EMPTY; sim_core::limits::MAX_MECHS],
        locks_len: 0,
        card_doors: [CardDoorSense::EMPTY; sim_core::monument::CARD_DOORS],
        card_doors_len: 0,
        trigger: Trigger::Start,
        options: [Goal::Wait; MAX_OPTIONS],
        options_len: 0,
    };

    pub fn items(&self) -> &[(Name, u32)] {
        &self.items[..self.items_len as usize]
    }

    pub fn craftable(&self) -> &[Name] {
        &self.craftable[..self.craftable_len as usize]
    }

    pub fn needs(&self) -> &[(Name, u32)] {
        &self.needs[..self.needs_len as usize]
    }

    pub fn raw(&self) -> &[(Name, u32)] {
        &self.raw[..self.raw_len as usize]
    }

    pub fn stored(&self) -> &[(Name, u32)] {
        &self.stored[..self.stored_len as usize]
    }

    pub fn home_holds(&self) -> &[(Name, u32)] {
        &self.home_holds[..self.home_holds_len as usize]
    }

    pub fn options(&self) -> &[Goal] {
        &self.options[..self.options_len as usize]
    }

    pub fn threats(&self) -> &[Threat] {
        &self.threats[..self.threats_len as usize]
    }

    pub fn offers(&self, goal: Goal) -> bool {
        self.options().contains(&goal)
    }

    /// Offer a goal once. Silently full past `MAX_OPTIONS`, which the
    /// constant's arithmetic makes unreachable.
    pub fn offer(&mut self, goal: Goal) {
        if !self.offers(goal) && (self.options_len as usize) < MAX_OPTIONS {
            self.options[self.options_len as usize] = goal;
            self.options_len += 1;
        }
    }

    pub fn count_of(&self, name: &str) -> u32 {
        self.items()
            .iter()
            .filter(|(n, _)| n.as_str() == name)
            .map(|(_, c)| *c)
            .sum()
    }

    /// What it heard since the last request, oldest first: who (the chat
    /// log's label), whether that is itself or a declared agent, the
    /// channel and the words. Sent to an external source beside the
    /// observation, never inside it, so Jev is never handed player text.
    /// Worker only (it allocates).
    pub fn heard_json(&self) -> Value {
        Value::Array(
            self.heard
                .lines()
                .iter()
                .map(|l| {
                    json!({
                        "from": l.label(),
                        "me": l.me,
                        "agent": l.agent,
                        "friend": self.friend.set
                            && !l.address.is_guest()
                            && l.address == self.friend.who.address,
                        "channel": match l.channel {
                            protocol::Channel::Local => "local",
                            protocol::Channel::Global => "global",
                            protocol::Channel::Team => "team",
                        },
                        "text": core::str::from_utf8(l.text.as_bytes()).unwrap_or(""),
                    })
                })
                .collect(),
        )
    }

    /// THE GATE and the works, the stones, their locks and the ziggurat's
    /// card doors, as its client knows them. Sent to an external source
    /// beside the observation (Jev's request stays small).
    /// Worker only (it allocates).
    pub fn places_json(&self) -> Value {
        json!({
            "the_gate": self.gate.map(|p| p.json()),
            "works": self.works[..usize::from(self.works_len)].iter().map(WorkSense::json).collect::<Vec<_>>(),
            "stones": {
                "read": self.stones_read,
                "known": self.stones_known,
                "nearest_unread": self.unread_stone.map(|p| p.json()),
                "glyphs_i_read": format!("{} of {}", self.glyphs, self.alphabet),
            },
            "locks": self.locks[..usize::from(self.locks_len)].iter().map(LockSense::json).collect::<Vec<_>>(),
            "ziggurat_doors": self.card_doors[..usize::from(self.card_doors_len)].iter().map(CardDoorSense::json).collect::<Vec<_>>(),
        })
    }

    /// Its friend (`--friend`), by the label a chat log shows: `null` with
    /// none named. Sent beside the observation, as `team` is. Worker only.
    pub fn friend_json(&self) -> Value {
        let f = &self.friend;
        if !f.set {
            return Value::Null;
        }
        let hex = f.who.address.to_hex();
        json!({
            "name": f.who.label(),
            "wallet": core::str::from_utf8(&hex).unwrap_or(""),
            "online": f.online,
            "on_my_team": f.on_team,
            "in_sight": f.in_sight.json(),
            "in_invite_reach": f.reachable,
            "invited_me": f.invited_me,
        })
    }

    /// Who its team is and whose invite waits, by the labels a chat log
    /// shows. Sent to an external source beside the observation, as
    /// `heard` is, never inside it. Worker only (it allocates).
    pub fn team_json(&self) -> Value {
        let t = &self.team;
        json!({
            "invite_from": t.invited.then(|| t.invite_from.label()),
            "teammates": t.who[..t.who_len as usize].iter().map(Who::label).collect::<Vec<_>>(),
        })
    }

    /// The model-visible state. Worker only (it allocates).
    pub fn to_json(&self) -> Value {
        let meter = |v: u16, max: u16| {
            if max == 0 {
                Value::Null
            } else {
                json!([v, max])
            }
        };
        json!({
            "body": match self.body {
                BodyState::Alive => "alive",
                BodyState::Wounded => "wounded",
                BodyState::Dead => "dead",
            },
            "health": meter(self.hp, self.hp_max),
            "food": meter(self.food, self.food_max),
            "water": meter(self.water, self.water_max),
            "pack": {
                "items": self.items().iter().map(|(n, c)| json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
                "free_slots": self.free_slots,
            },
            "craftable_now": self.craftable().iter().map(|n| n.as_str()).collect::<Vec<_>>(),
            "in_view": {
                "trees": self.trees.json(),
                "stone_nodes": self.stone_nodes.json(),
                "ore_nodes": self.ore_nodes.json(),
                "berry_bushes": self.bushes.json(),
                "hemp": self.hemp.json(),
                "players": self.players.json(),
                "animals": self.animals.json(),
            },
            "water_nearby": self.water_near.json(),
            "threats": self.threats().iter().map(Threat::json).collect::<Vec<_>>(),
            "last_fight": self.last_fight.map(Why::text),
            "last_goal": self.last.map(|r| json!({
                "goal": r.goal.label().as_str(),
                "outcome": r.outcome.word(),
                "why": r.outcome.why().map(Why::text),
                "gained": r.gained,
                "seconds": r.secs,
            })),
            "current_goal": self.current.map(|r| json!({
                "goal": r.goal.label().as_str(),
                "gained": r.gained,
                "seconds": r.secs,
            })),
            "hits_taken_since_last_decision": self.hits,
            "deaths": self.deaths,
            "respawns": self.respawns,
            "sleeping_bags_down": self.bags,
            "my_death_backpack": self.backpack.json(),
            "home": self.home.json(),
            "a_sleeping_bag_is_ready": self.bag_ready,
            "night": self.night,
            "base_next": {
                "milestone": self.milestone.word(),
                "missing": self.needs().iter().map(|(n, c)| json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
                "raw": self.raw().iter().map(|(n, c)| json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
            },
            "stations": { "workbench": self.bench, "furnace": self.furnace },
            "loot_known": self.loot.json(),
            "loot_place": self.loot_place.map(|p| p.json()),
            "raid_targets": self.raid.json(),
            "my_box": {
                "holds": self.stored().iter().map(|(n, c)| json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
                "has_what_the_base_needs": self.take_out,
            },
            "cupboard_needs_feeding": self.feed,
            "team": self.team.json(),
            "team_home_holds": self.home_holds().iter().map(|(n, c)| json!({"name": n.as_str(), "count": c})).collect::<Vec<_>>(),
            "asked_because": self.trigger.word(),
        })
    }
}

/// A decision's one-line reason: printable ASCII, whitespace collapsed,
/// truncated. Never a prompt, a key or a raw response body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Reason {
    bytes: [u8; REASON_BYTES],
    len: u8,
}

impl Reason {
    pub const EMPTY: Self = Self {
        bytes: [0; REASON_BYTES],
        len: 0,
    };

    pub fn from_text(text: &str) -> Self {
        let mut out = Self::EMPTY;
        let mut space = false;
        for &b in text.as_bytes() {
            if out.len as usize == REASON_BYTES {
                break;
            }
            let b = if b.is_ascii_whitespace() { b' ' } else { b };
            if !(0x20..=0x7e).contains(&b) {
                continue;
            }
            if b == b' ' && (space || out.len == 0) {
                continue;
            }
            space = b == b' ';
            out.bytes[out.len as usize] = b;
            out.len += 1;
        }
        while out.len > 0 && out.bytes[out.len as usize - 1] == b' ' {
            out.len -= 1;
        }
        out
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl std::fmt::Debug for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

/// One accepted answer.
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub goal: Goal,
    pub confidence: f64,
    pub reason: Reason,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// A line to say out loud: an external source's own, or the scripted
    /// policy's fixed answer to an order. Jev's is always [`Say::Quiet`],
    /// and the body ignores a line from it.
    pub say: Say,
    /// The team's lock code, as a teammate gave it (an external agent's
    /// `"code"`, or a number on team chat for the scripted policy): kept
    /// for the locks on the way to the team's chest.
    pub code: Option<crate::agent::lock::LockCode>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    Jev,
    Scripted,
    External,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Jev => "Jev 1.13.0",
            SourceKind::Scripted => "Scripted",
            SourceKind::External => "External agent",
        }
    }
}

/// Blocking by design: invoked on the decision worker (or inline for a
/// non-blocking, allocation-free source). Sources bound their own I/O; the
/// mind rejects late replies but cannot cancel a source.
pub trait DecisionSource: Send + 'static {
    fn kind(&self) -> SourceKind;
    fn decide(&mut self, summary: &Summary) -> Result<Choice, String>;
}

/// Fixed request windows. A window opens at its first request and closes
/// `span` later; the next request opens a new one.
#[derive(Clone, Copy, Debug)]
struct Window {
    start: Option<Instant>,
    count: u32,
    span: Duration,
}

impl Window {
    fn roll(&mut self, now: Instant) {
        if self
            .start
            .is_some_and(|start| now.saturating_duration_since(start) >= self.span)
        {
            self.start = None;
            self.count = 0;
        }
    }

    fn record(&mut self, now: Instant) {
        self.roll(now);
        self.start.get_or_insert(now);
        self.count = self.count.saturating_add(1);
    }
}

/// The request ceiling. Counts requests sent, answered or not: a provider
/// may bill a request whose answer could not be used.
#[derive(Clone, Copy, Debug)]
pub struct SpendGuard {
    per_hour: u32,
    per_day: u32,
    hour: Window,
    day: Window,
}

impl SpendGuard {
    pub fn new(per_hour: u32, per_day: u32) -> Self {
        Self {
            per_hour,
            per_day,
            hour: Window {
                start: None,
                count: 0,
                span: Duration::from_secs(3600),
            },
            day: Window {
                start: None,
                count: 0,
                span: Duration::from_secs(86400),
            },
        }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        self.hour.roll(now);
        self.day.roll(now);
        self.hour.count < self.per_hour && self.day.count < self.per_day
    }

    pub fn record(&mut self, now: Instant) {
        self.hour.record(now);
        self.day.record(now);
    }

    /// Requests in the current hour and day windows, and the two ceilings.
    pub fn counts(&self) -> (u32, u32, u32, u32) {
        (self.hour.count, self.per_hour, self.day.count, self.per_day)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MindConfig {
    /// The floor between two requests: the operator's once-per-second
    /// ceiling (`jev::THINK_INTERVAL`), also the backoff unit.
    pub interval: Duration,
    /// How long an answer stays usable, and how stale a snapshot may be.
    pub timeout: Duration,
    pub heartbeat: Duration,
    pub per_hour: u32,
    pub per_day: u32,
}

impl Default for MindConfig {
    fn default() -> Self {
        Self {
            interval: crate::jev::THINK_INTERVAL,
            timeout: crate::jev::REQUEST_TIMEOUT,
            heartbeat: Duration::from_secs(JEV_HEARTBEAT_SECS),
            per_hour: JEV_MAX_REQUESTS_HOUR,
            per_day: JEV_MAX_REQUESTS_DAY,
        }
    }
}

impl MindConfig {
    pub fn check(&self) -> Result<(), String> {
        let limit = Duration::from_secs(60);
        if self.interval < crate::jev::THINK_INTERVAL
            || self.interval > limit
            || self.timeout.is_zero()
            || self.timeout > limit
        {
            return Err(
                "decision interval must be 1..60 s (never under the once-per-second ceiling) and the timeout (0, 60 s]".into(),
            );
        }
        if self.heartbeat < self.interval || self.heartbeat > Duration::from_secs(3600) {
            return Err("heartbeat must be between the decision interval and one hour".into());
        }
        if self.per_hour == 0 || self.per_day == 0 {
            return Err("request ceilings must be at least one".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MindStats {
    pub requests: u64,
    pub decisions: u64,
    pub failures: u64,
    /// Answers that arrived after their deadline and were refused.
    pub late: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Times the spend guard paused decisions.
    pub pauses: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Ready,
    Asking,
    BackingOff,
    Paused,
}

impl Mode {
    pub fn text(self) -> &'static str {
        match self {
            Mode::Ready => "ready",
            Mode::Asking => "deciding",
            Mode::BackingOff => "backing off after a failed decision",
            Mode::Paused => "paused: spend cap",
        }
    }
}

type Ask = (u32, Summary);
type Reply = (u32, Option<Choice>);

/// One request outstanding, a bounded answer, strict acceptance, explicit
/// failure, and no substitute decisions: a failed or refused request leaves
/// the body without a goal until a real answer arrives.
pub struct Mind {
    kind: SourceKind,
    inline: Option<Box<dyn DecisionSource>>,
    inline_reply: Option<Reply>,
    asks: Option<rtrb::Producer<Ask>>,
    replies: Option<rtrb::Consumer<Reply>>,
    worker: Option<std::thread::JoinHandle<()>>,
    cfg: MindConfig,
    next_allowed: Instant,
    pending: Option<(u32, Instant)>,
    next_id: u32,
    failures: u32,
    guard: SpendGuard,
    paused: bool,
    /// Requests its source answers without a model call (the autopilot's
    /// own turns, `autopilot.rs`): never held by the spend guard.
    free_when: Option<fn(&Summary) -> bool>,
    pub stats: MindStats,
    /// The last accepted answer, for display.
    pub last: Option<Choice>,
}

impl Mind {
    /// Decisions on a dedicated worker thread: the shape for any source
    /// that blocks or allocates (Jev's HTTP, an external agent's pipes).
    pub fn new(source: impl DecisionSource, cfg: MindConfig) -> Result<Self, String> {
        cfg.check()?;
        let kind = source.kind();
        let (asks, mut inbox) = rtrb::RingBuffer::<Ask>::new(1);
        let (mut outbox, replies) = rtrb::RingBuffer::<Reply>::new(1);
        let mut source = source;
        let worker = std::thread::Builder::new()
            .name("jev-decisions".into())
            .spawn(move || loop {
                let (id, summary) = match inbox.pop() {
                    Ok(ask) => ask,
                    Err(_) if inbox.is_abandoned() => break,
                    Err(_) => {
                        std::thread::park();
                        continue;
                    }
                };
                let start = Instant::now();
                let answer = source.decide(&summary);
                match &answer {
                    Ok(c) => eprintln!(
                        "mind[{}]: {} ({:.3}), {} ms, {} input tokens; {}",
                        source.kind().label(),
                        c.goal.label().as_str(),
                        c.confidence,
                        start.elapsed().as_millis(),
                        c.input_tokens,
                        c.reason.as_str()
                    ),
                    Err(e) => eprintln!(
                        "mind[{}]: {e}; no goal until the next answer",
                        source.kind().label()
                    ),
                }
                // Error text is dropped here, never on the input loop.
                if outbox.push((id, answer.ok())).is_err() {
                    break;
                }
            })
            .map_err(|e| format!("decision worker: {e}"))?;
        Ok(Self::with(
            kind,
            None,
            Some(asks),
            Some(replies),
            Some(worker),
            cfg,
        ))
    }

    /// Decisions on the caller's thread. Only for a source whose `decide`
    /// never blocks and never allocates (the scripted policy); the lockstep
    /// tests use it so a run is a function of its inputs alone.
    pub fn inline(source: impl DecisionSource, cfg: MindConfig) -> Result<Self, String> {
        cfg.check()?;
        let kind = source.kind();
        Ok(Self::with(
            kind,
            Some(Box::new(source)),
            None,
            None,
            None,
            cfg,
        ))
    }

    fn with(
        kind: SourceKind,
        inline: Option<Box<dyn DecisionSource>>,
        asks: Option<rtrb::Producer<Ask>>,
        replies: Option<rtrb::Consumer<Reply>>,
        worker: Option<std::thread::JoinHandle<()>>,
        cfg: MindConfig,
    ) -> Self {
        let now = Instant::now();
        Self {
            kind,
            inline,
            inline_reply: None,
            asks,
            replies,
            worker,
            cfg,
            next_allowed: now,
            pending: None,
            next_id: 1,
            failures: 0,
            guard: SpendGuard::new(cfg.per_hour, cfg.per_day),
            paused: false,
            free_when: None,
            stats: MindStats::default(),
            last: None,
        }
    }

    /// Requests `rule` says cost nothing are never held by, or counted
    /// against, the spend guard: a buddy whose friend is away plays all day
    /// on the scripted policy without spending the day's ceiling.
    pub fn free_when(mut self, rule: fn(&Summary) -> bool) -> Self {
        self.free_when = Some(rule);
        self
    }

    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    pub fn config(&self) -> MindConfig {
        self.cfg
    }

    pub fn guard(&self) -> &SpendGuard {
        &self.guard
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn mode(&self, now: Instant) -> Mode {
        if self.pending.is_some() {
            Mode::Asking
        } else if self.paused {
            Mode::Paused
        } else if self.failures > 0 && now < self.next_allowed {
            Mode::BackingOff
        } else {
            Mode::Ready
        }
    }

    /// Accept at most one answer. A reply to an expired or superseded
    /// request is a failure, never a goal.
    pub fn poll(&mut self, now: Instant) -> Option<Choice> {
        let (id, answer) = match self.inline_reply.take() {
            Some(reply) => reply,
            None => self.replies.as_mut()?.pop().ok()?,
        };
        let Some((pending, sent)) = self.pending else {
            self.stats.late += 1;
            return None;
        };
        if pending != id {
            self.stats.late += 1;
            return None;
        }
        self.pending = None;
        let fresh = now.saturating_duration_since(sent) < self.cfg.timeout;
        match answer {
            Some(choice) if fresh => {
                self.stats.decisions += 1;
                self.stats.input_tokens += choice.input_tokens;
                self.stats.output_tokens += choice.output_tokens;
                self.failures = 0;
                self.last = Some(choice);
                Some(choice)
            }
            other => {
                if other.is_some() {
                    self.stats.late += 1;
                }
                self.stats.failures += 1;
                self.failures = (self.failures + 1).min(6);
                self.next_allowed = now + self.cfg.interval * (1 << self.failures);
                None
            }
        }
    }

    /// Send one request if the floor, the backoff, the spend guard and the
    /// single outstanding slot all allow it. True when a request went out.
    /// The scripted policy costs nothing to ask, so the spend guard never
    /// holds it: a house bot asking every few seconds would otherwise reach
    /// the day's ceiling hours before its run ends and stand still.
    pub fn ask(&mut self, now: Instant, summary: &Summary) -> bool {
        if self.pending.is_some() || now < self.next_allowed {
            return false;
        }
        let free = self.kind == SourceKind::Scripted || self.free_when.is_some_and(|f| f(summary));
        if !free && !self.guard.allow(now) {
            if !self.paused {
                self.paused = true;
                self.stats.pauses += 1;
            }
            return false;
        }
        self.paused = false;
        let id = self.next_id;
        if let Some(source) = self.inline.as_mut() {
            let answer = source.decide(summary);
            self.inline_reply = Some((id, answer.ok()));
        } else {
            let Some(tx) = self.asks.as_mut() else {
                return false;
            };
            if tx.push((id, *summary)).is_err() {
                return false;
            }
            if let Some(worker) = &self.worker {
                worker.thread().unpark();
            }
        }
        self.next_id = self.next_id.wrapping_add(1);
        self.pending = Some((id, now));
        if !free {
            self.guard.record(now);
        }
        self.stats.requests += 1;
        self.next_allowed = now + self.cfg.interval;
        true
    }

    /// Forget an outstanding request whose answer can no longer be used
    /// (it has aged past the deadline). The worker's reply, if it comes,
    /// is refused as late. Counts as a failure.
    pub fn expire(&mut self, now: Instant) {
        if let Some((_, sent)) = self.pending {
            if now.saturating_duration_since(sent) >= self.cfg.timeout + self.cfg.interval {
                self.pending = None;
                self.stats.failures += 1;
                self.failures = (self.failures + 1).min(6);
                self.next_allowed = now + self.cfg.interval * (1 << self.failures);
            }
        }
    }
}

impl Drop for Mind {
    fn drop(&mut self) {
        // Close the inbox before joining; sources bound their own I/O.
        self.asks.take();
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

/// Explicit offline goals, never a fallback for a failed model request.
/// Deterministic from the summary and what its team told it;
/// allocation-free.
#[derive(Default)]
pub struct Scripted {
    rotation: u8,
    /// A teammate's standing order (`agent::orders`): follow them, or hold
    /// here, until another order or until it has no team.
    standing: Option<Order>,
    /// A run or the walk home a teammate asked for, not yet started.
    errand: Option<Order>,
    /// A person near it asked to team up, in local chat.
    asked: bool,
    /// The answer to say with the next choice, and a code a teammate gave.
    say: Say,
    code: Option<crate::agent::lock::LockCode>,
}

fn pct(value: u16, max: u16) -> u32 {
    if max == 0 {
        100
    } else {
        u32::from(value) * 100 / u32::from(max)
    }
}

impl DecisionSource for Scripted {
    fn kind(&self) -> SourceKind {
        SourceKind::Scripted
    }

    fn decide(&mut self, s: &Summary) -> Result<Choice, String> {
        self.hear(s);
        let (goal, why) = self.pick(s);
        Ok(Choice {
            goal,
            confidence: 1.0,
            reason: Reason::from_text(why),
            input_tokens: 0,
            output_tokens: 0,
            say: std::mem::take(&mut self.say),
            code: self.code.take(),
        })
    }
}

impl Scripted {
    /// What its team typed since it was last asked, read as orders
    /// (`agent::orders`), and whether someone near it asked to team up.
    /// A line it said itself or an agent said is never an order.
    fn hear(&mut self, s: &Summary) {
        if s.team.mates == 0 {
            self.standing = None;
            self.errand = None;
        }
        for line in s.heard.lines() {
            if line.me || line.agent {
                continue;
            }
            let text = line.text.as_bytes();
            match line.channel {
                protocol::Channel::Team => {
                    if let Some(code) = orders::code(text) {
                        self.code = Some(code);
                        self.reply("got the code");
                    }
                    match orders::order(text) {
                        Some(Order::Help) => self.reply(orders::HELP),
                        Some(Order::Follow) => {
                            self.standing = Some(Order::Follow);
                            self.errand = None;
                            self.reply("following you");
                        }
                        Some(Order::Stay) => {
                            self.standing = Some(Order::Stay);
                            self.errand = None;
                            self.reply("holding here");
                        }
                        Some(errand) => {
                            self.standing = None;
                            self.errand = Some(errand);
                        }
                        None => {}
                    }
                }
                protocol::Channel::Local => self.asked |= orders::asks_to_team(text),
                protocol::Channel::Global => {}
            }
        }
    }

    /// The next answer: on team chat, or out loud with no team.
    fn reply(&mut self, text: &str) {
        if let Some(line) = protocol::ChatText::sanitize(text.as_bytes()) {
            self.say = Say::Team(line);
        }
    }

    fn pick(&mut self, s: &Summary) -> (Goal, &'static str) {
        // Survive now. The fight reflex answers blows on its own; the
        // policy only calls a retreat once staying would be the end.
        let threat = s.players.count > 0 || s.animals.count > 0;
        let hp = pct(s.hp, s.hp_max);
        if s.hits > 0 && threat && hp < SCRIPTED_FLEE_HP_PCT && s.offers(Goal::Flee) {
            return (Goal::Flee, "scripted: badly hurt with a body in view");
        }
        if hp < SCRIPTED_HEAL_HP_PCT && s.threats_len == 0 && s.offers(Goal::Heal) {
            return (Goal::Heal, "scripted: hurt, and nobody about");
        }
        // Home fought over: back inside, the fight met from there, then
        // the damage mended.
        if (s.home.attacked || s.team.home_attacked) && s.offers(Goal::Defend) {
            return (Goal::Defend, "scripted: home is under attack");
        }
        // A raid done: home with what it took, before anyone comes after it.
        let raided = s
            .last
            .is_some_and(|r| r.goal == Goal::Raid && r.outcome == Outcome::Done);
        if raided && s.offers(Goal::GoHome) {
            return (Goal::GoHome, "scripted: home with the raid's take");
        }
        // A team (wire v108): say yes, and go to them when they are set on.
        if s.offers(Goal::JoinTeam) {
            return (Goal::JoinTeam, "scripted: a team invite is waiting");
        }
        // Its friend (`--friend`) on the island and not on its team: an
        // invite once they are in reach, else the walk to THE GATE, where
        // they know to find it.
        if s.friend.online && !s.friend.on_team {
            if s.friend.reachable && s.offers(Goal::Invite) {
                self.reply("invite sent: press Y");
                return (Goal::Invite, "scripted: my friend is here");
            }
            if s.offers(Goal::Meet) {
                return (Goal::Meet, "scripted: meeting my friend at THE GATE");
            }
        }
        if std::mem::take(&mut self.asked) {
            if s.offers(Goal::Invite) {
                self.reply("invite sent: press Y");
                return (Goal::Invite, "scripted: a player asked to team up");
            }
            if s.team.mates == 0 {
                self.reply("come closer, or press N on me");
            }
        }
        if s.team.mate_attacked && s.offers(Goal::Follow) {
            return (Goal::Follow, "scripted: my teammate is under attack");
        }
        let low = SCRIPTED_LOW_METER_PCT;
        let bush = s.offers(Goal::Forage) && s.bushes.count > 0;
        if pct(s.water, s.water_max) < low {
            if s.offers(Goal::Drink) {
                return (Goal::Drink, "scripted: water is low");
            }
            // No sea in sight and no food known to carry water: eating is
            // how the body learns which of its food does.
            if s.offers(Goal::Eat) {
                return (
                    Goal::Eat,
                    "scripted: water is low, eating to find juicy food",
                );
            }
            if bush {
                return (Goal::Forage, "scripted: water is low, berries are in view");
            }
        }
        if pct(s.food, s.food_max) < low {
            if s.offers(Goal::Eat) {
                return (Goal::Eat, "scripted: food is low");
            }
            if s.offers(Goal::Cook) {
                return (Goal::Cook, "scripted: food is low, cook the raw meat");
            }
            if bush {
                return (Goal::Forage, "scripted: food is low, a bush is in view");
            }
        }
        // A teammate's word: a run or the walk home once, then a follow
        // or a hold until they say otherwise.
        if let Some(errand) = self.errand.take() {
            let (goal, said) = match errand {
                Order::Supply(goal) => (
                    goal,
                    match goal {
                        Goal::SupplyStone => "on it: stone",
                        Goal::SupplyOre => "on it: ore",
                        Goal::SupplyHemp => "on it: cloth",
                        _ => "on it: wood",
                    },
                ),
                _ => (Goal::GoHome, "heading home"),
            };
            if s.offers(goal) {
                self.reply(said);
                return (goal, "scripted: a teammate asked");
            }
            self.reply(if goal.supplies() && s.team.home.count == 0 {
                "show me your box first"
            } else {
                "can't right now"
            });
        }
        match self.standing {
            Some(Order::Follow) if s.offers(Goal::Follow) => {
                return (Goal::Follow, "scripted: my teammate said follow");
            }
            Some(Order::Stay) if s.offers(Goal::Wait) => {
                return (Goal::Wait, "scripted: my teammate said stay");
            }
            _ => {}
        }
        // What it died holding is worth a walk while nobody is about: the
        // offer says it is near and no one dangerous is in view.
        if s.offers(Goal::Recover) {
            return (
                Goal::Recover,
                "scripted: my backpack is near and nobody is about",
            );
        }
        // Home quiet but damaged: mended once the body is seen to.
        if s.offers(Goal::Defend) {
            return (Goal::Defend, "scripted: home is damaged");
        }
        // A teammate wandering off is caught up with; near them, the work
        // goes on.
        if s.offers(Goal::Follow) && f32::from(s.team.awake.nearest_m) > SCRIPTED_FOLLOW_M {
            return (Goal::Follow, "scripted: catching up with my teammate");
        }
        // A heartbeat with no urgent need lets a working goal finish: the
        // rotation below is for choosing a new one, not for abandoning a
        // tree half felled.
        if let Some(current) = s.current.filter(|c| s.offers(c.goal)) {
            return (current.goal, "scripted: keep going");
        }
        // Better tools first, named by the controller's own tool ladders.
        for ladder in [crate::explorer::TREE_TOOLS, crate::explorer::NODE_TOOLS] {
            let owned = ladder.iter().position(|t| s.count_of(t) > 0);
            for (rank, tool) in ladder.iter().enumerate() {
                if owned.is_some_and(|o| o <= rank) {
                    break;
                }
                if let Some(name) = s.craftable().iter().find(|n| n.as_str() == *tool) {
                    return (Goal::Craft(*name), "scripted: a better tool is craftable");
                }
            }
        }
        // With stone tools in hand, somewhere to wake that is not the beach:
        // a sleeping bag, crafted and put down where it works.
        let tooled = [crate::explorer::TREE_TOOLS, crate::explorer::NODE_TOOLS]
            .iter()
            .all(|ladder| ladder[..ladder.len() - 1].iter().any(|t| s.count_of(t) > 0));
        if tooled && s.bags == 0 {
            if s.offers(Goal::Bag) {
                return (Goal::Bag, "scripted: put a sleeping bag down");
            }
            if s.count_of(BAG_ITEM) == 0 {
                if let Some(name) = s.craftable().iter().find(|n| n.as_str() == BAG_ITEM) {
                    return (Goal::Craft(*name), "scripted: a sleeping bag is craftable");
                }
                // Not craftable for want of cloth, unless it is for want of
                // room, which hemp does not make.
                if s.offers(Goal::GatherHemp) && s.hemp.count > 0 && s.free_slots > 1 {
                    return (Goal::GatherHemp, "scripted: cloth for a sleeping bag");
                }
            }
        }
        // On a team, with the chest it saw a teammate at: fill it. A pack
        // getting full goes in; else a supply run of what it holds least
        // of, wood or stone.
        if s.team.home.count > 0 {
            if s.offers(Goal::Deliver)
                && (s.free_slots <= SCRIPTED_DELIVER_FREE || s.team.home_has_needs)
            {
                return (
                    Goal::Deliver,
                    "scripted: home to put things away and stock up",
                );
            }
            let held = |name: &str| -> u32 {
                s.home_holds()
                    .iter()
                    .filter(|(n, _)| n.as_str() == name)
                    .map(|(_, c)| *c)
                    .sum()
            };
            let mut runs = [
                (Goal::SupplyWood, held("Wood")),
                (Goal::SupplyStone, held("Stone")),
            ];
            runs.sort_by_key(|r| r.1);
            if let Some((goal, _)) = runs.into_iter().find(|r| s.offers(r.0)) {
                return (goal, "scripted: a supply run for the team's chest");
            }
        }
        // THE GATE and the works, once its own base is done or it lives in
        // the team's (before then the junk and the walk are the base's): a
        // metal tool it can buy is bought, surplus is sold, and plenty goes
        // to the works, which unlock things for everyone.
        if s.milestone == Milestone::Done || s.team.lives_there {
            for tool in ["Metal Hatchet", "Metal Pickaxe"] {
                if let Some(goal) = Name::new(tool.as_bytes())
                    .map(Goal::Buy)
                    .filter(|g| s.offers(*g) && s.count_of(tool) == 0)
                {
                    return (goal, "scripted: a metal tool at the gate");
                }
            }
            let plenty = |name: &str| s.count_of(name) >= SCRIPTED_PLENTY;
            if s.offers(Goal::FeedWork) && (plenty("Wood") || plenty("Stone")) {
                return (Goal::FeedWork, "scripted: plenty to give a work");
            }
            for name in ["Wood", "Stone"] {
                if let Some(goal) = Name::new(name.as_bytes())
                    .map(Goal::Sell)
                    .filter(|g| s.offers(*g) && s.count_of(name) >= 2 * SCRIPTED_PLENTY)
                {
                    return (goal, "scripted: selling the surplus at the gate");
                }
            }
            // The ancients, the same way: a card it carries opens its door
            // (the room's crates are a loot run's), a lock whose answer it
            // read is turned to it, and a stone near it is read.
            for (goal, why) in [
                (Goal::SwipeCard, "scripted: a keycard for its door"),
                (Goal::SolveLock, "scripted: a lock whose answer it read"),
                (Goal::ReadStone, "scripted: a stone it has not read"),
            ] {
                if s.offers(goal) {
                    return (goal, why);
                }
            }
        }
        // Home chores: the box holds what the base needs, the cupboard runs
        // low, or the pack is nearly full.
        if tooled
            && s.offers(Goal::Stash)
            && (s.take_out || s.feed || s.free_slots <= SCRIPTED_STASH_FREE_SLOTS)
        {
            return (Goal::Stash, "scripted: home to the box and the cupboard");
        }
        // Night, and a base worth breaking into is known: what it holds is
        // worth more than another hour of gathering.
        if s.night && s.offers(Goal::Raid) {
            return (Goal::Raid, "scripted: night, and a base worth raiding");
        }
        // Raw meat off a kill is no food until it is cooked.
        if s.offers(Goal::Cook) {
            return (Goal::Cook, "scripted: cook the raw meat");
        }
        // Salvage the base does not want whole is junk for research and
        // fragments for the base, at its own recycler.
        if s.offers(Goal::Recycle) {
            return (Goal::Recycle, "scripted: recycle the salvage");
        }
        // Gears enough for a recycler and some over for it to take apart,
        // and none to take them to: one is made at the bench.
        if tooled
            && s.count_of("Gears") >= SCRIPTED_RECYCLER_GEARS
            && s.count_of(RECYCLER_ITEM) == 0
        {
            if let Some(name) = s.craftable().iter().find(|n| n.as_str() == RECYCLER_ITEM) {
                return (Goal::Craft(*name), "scripted: a recycler for the gears");
            }
        }
        // Then arms, once stone tools make gathering cheap and home's chores
        // are seen to (the bag's cloth first): a spear, a bow and arrows for
        // it, and bandages. Gear decides early fights.
        let started = [crate::explorer::TREE_TOOLS, crate::explorer::NODE_TOOLS]
            .iter()
            .any(|ladder| ladder[..ladder.len() - 1].iter().any(|t| s.count_of(t) > 0));
        // While the bag is still to be made, its cloth is not spent on a bow
        // or bandages: only the spear, which takes none, goes first.
        let bag_wanted = tooled && s.bags == 0 && s.count_of(BAG_ITEM) == 0;
        if started {
            let [(spear, ..), ..] = crate::agent::loadout::ARM_UP;
            for (item, want, needs) in crate::agent::loadout::ARM_UP {
                if s.count_of(item) >= want
                    || needs.is_some_and(|n| s.count_of(n) == 0)
                    || (bag_wanted && item != spear)
                {
                    continue;
                }
                if let Some(name) = s.craftable().iter().find(|n| n.as_str() == item) {
                    return (Goal::Craft(*name), "scripted: arming up");
                }
            }
        }
        // Satchels for the raids, once the bench makes them.
        if s.count_of(SATCHEL_ITEM) < SCRIPTED_SATCHELS {
            if let Some(name) = s.craftable().iter().find(|n| n.as_str() == SATCHEL_ITEM) {
                return (Goal::Craft(*name), "scripted: satchels for a raid");
            }
        }
        // Ore in the pack and its own furnace standing: smelted, a batch at
        // a time (the craft walks home to it).
        if s.furnace {
            for (ore, out) in SMELTS {
                if s.count_of(ore) == 0 {
                    continue;
                }
                if let Some(name) = s.craftable().iter().find(|n| n.as_str() == out) {
                    return (Goal::Craft(*name), "scripted: smelt the ore at my furnace");
                }
            }
        }
        // Then a home and its stations: the next milestone's materials,
        // gathered where they are in view, then the building. What the pack
        // can make of what it holds is made (fuel, fragments at the
        // furnace); what it comes down to is gathered, hunted or looted.
        // Short of something nobody can see, it builds what the pack pays
        // for, and otherwise looks round.
        if tooled && s.milestone != Milestone::Done {
            if s.needs_len == 0 && s.offers(Goal::Build) {
                return (Goal::Build, "scripted: build the next part of the base");
            }
            for (name, _) in s.needs() {
                if let Some(name) = s.craftable().iter().find(|n| *n == name) {
                    return (Goal::Craft(*name), "scripted: make what the base needs");
                }
            }
            let raw = if s.raw_len > 0 { s.raw() } else { s.needs() };
            for (name, _) in raw {
                let (goal, seen) = match name.as_str() {
                    "Wood" => (Goal::GatherWood, s.trees.count > 0),
                    "Stone" => (Goal::GatherStone, s.stone_nodes.count > 0),
                    "Cloth" => (Goal::GatherHemp, s.hemp.count > 0),
                    "Metal Ore" | "Sulfur Ore" => (Goal::GatherOre, s.ore_nodes.count > 0),
                    "Animal Fat" => (Goal::Hunt, s.animals.count > 0),
                    _ => (Goal::Loot, false),
                };
                if seen && s.offers(goal) {
                    return (goal, "scripted: materials for the base");
                }
                // A loot run is a long way to go: what the pack already
                // pays for goes up first.
                if LOOTED.contains(&name.as_str()) && s.offers(Goal::Loot) && !s.offers(Goal::Build)
                {
                    return (Goal::Loot, "scripted: loot for what the base needs");
                }
            }
            if s.offers(Goal::Build) {
                return (Goal::Build, "scripted: build what the pack pays for");
            }
        }
        // Everything made: a metal door in a wooden one's place, once the
        // fragments are spare.
        if tooled && s.offers(Goal::Build) {
            return (Goal::Build, "scripted: the spare pays for a better door");
        }
        // A barrel or a crate close by is worth the detour.
        if tooled
            && s.offers(Goal::Loot)
            && s.loot.count > 0
            && s.loot.nearest_m <= SCRIPTED_LOOT_NEAR_M
        {
            return (Goal::Loot, "scripted: a barrel or crate is near");
        }
        // Alternate the resources, preferring one that is in view, while
        // the pack keeps room for what the base and its benches make: a
        // pack full of everything makes nothing.
        let room = s.free_slots > SCRIPTED_STASH_FREE_SLOTS;
        let order = [
            Goal::GatherWood,
            Goal::GatherStone,
            Goal::GatherWood,
            Goal::GatherOre,
        ];
        let seen = |g: Goal| match g {
            Goal::GatherWood => s.trees.count > 0 && s.count_of("Wood") < SCRIPTED_PLENTY,
            Goal::GatherStone => s.stone_nodes.count > 0 && s.count_of("Stone") < SCRIPTED_PLENTY,
            Goal::GatherOre => s.ore_nodes.count > 0,
            _ => false,
        };
        for step in 0..order.len() {
            let goal = order[(self.rotation as usize + step) % order.len()];
            if room && s.offers(goal) && seen(goal) {
                self.rotation = self.rotation.wrapping_add(step as u8 + 1);
                return (goal, "scripted: next resource in the rotation is in view");
            }
        }
        // Topping up food from a bush, with the same room kept.
        if room && s.offers(Goal::Forage) && s.bushes.count > 0 && pct(s.food, s.food_max) < 100 {
            return (Goal::Forage, "scripted: a bush is in view");
        }
        (Goal::Explore, "scripted: nothing useful in view")
    }
}

/// Recent goals, oldest first, in fixed storage.
#[derive(Clone, Copy, Debug)]
pub struct History {
    reports: [Option<Report>; GOAL_HISTORY],
    head: usize,
    len: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            reports: [None; GOAL_HISTORY],
            head: 0,
            len: 0,
        }
    }
}

impl History {
    pub fn push(&mut self, report: Report) {
        if self.len == GOAL_HISTORY {
            self.head = (self.head + 1) % GOAL_HISTORY;
            self.len -= 1;
        }
        self.reports[(self.head + self.len) % GOAL_HISTORY] = Some(report);
        self.len += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = Report> + '_ {
        (0..self.len).filter_map(move |i| self.reports[(self.head + i) % GOAL_HISTORY])
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Name {
        Name::new(s.as_bytes()).unwrap()
    }

    #[test]
    fn labels_round_trip_only_through_offered_goals() {
        let hatchet = Goal::Craft(name("Stone Hatchet"));
        let mut s = Summary::EMPTY;
        for g in Goal::FIXED {
            s.offer(g);
        }
        s.offer(hatchet);
        assert_eq!(hatchet.label().as_str(), "craft:Stone Hatchet");
        for g in s.options() {
            assert_eq!(Goal::parse(g.label().as_str(), s.options()), Some(*g));
        }
        assert_eq!(Goal::parse("craft:Metal Pickaxe", s.options()), None);
        assert_eq!(Goal::parse("teleport", s.options()), None);
        assert_eq!(Goal::parse("gather_wood", &[Goal::Explore]), None);
        assert!(Name::new(b"").is_none() && Name::new(b"bad\nname").is_none());
        assert!(Name::new(&[b'x'; MAX_ITEM_NAME_BYTES + 1]).is_none());
    }

    #[test]
    fn reasons_are_bounded_printable_single_lines() {
        let r = Reason::from_text("  two\nlines\tand \u{1b}[31m escapes  ");
        assert_eq!(r.as_str(), "two lines and [31m escapes");
        let long = "x".repeat(REASON_BYTES * 3);
        assert_eq!(Reason::from_text(&long).as_str().len(), REASON_BYTES);
    }

    #[test]
    fn the_summary_json_carries_no_seed_position_or_identity() {
        let mut s = Summary::EMPTY;
        s.items[0] = (name("Wood"), 40);
        s.items_len = 1;
        s.trees.add(12.4, 7);
        let v = s.to_json();
        let text = v.to_string();
        for banned in ["seed", "\"x\"", "\"z\"", "qx", "player_id", "position"] {
            assert!(!text.contains(banned), "{banned} in {text}");
        }
        assert_eq!(v["in_view"]["trees"]["bearing"], "ahead-left");
        assert_eq!(v["pack"]["items"][0]["name"], "Wood");
    }

    /// Heard lines reach an external source labelled the way a player's
    /// chat log labels them, the ring keeps the newest, and none of it is
    /// in the observation Jev is sent.
    #[test]
    fn heard_lines_are_labelled_capped_and_kept_out_of_the_observation() {
        let line = |from: u32, text: &str| HeardLine {
            from,
            text: protocol::ChatText::sanitize(text.as_bytes()).unwrap(),
            ..HeardLine::EMPTY
        };
        let mut s = Summary::EMPTY;
        s.heard.push(HeardLine {
            name: protocol::Name::new("Ash").unwrap(),
            ..line(257, "hi bot")
        });
        s.heard.push(HeardLine {
            agent: true,
            channel: protocol::Channel::Global,
            ..line(258, "raid north")
        });
        let v = s.heard_json();
        assert_eq!(v[0]["from"], "Ash");
        assert_eq!(v[0]["channel"], "local");
        assert_eq!(v[0]["text"], "hi bot");
        assert_eq!(
            (v[1]["from"].as_str(), v[1]["agent"].as_bool()),
            (Some("#258"), Some(true))
        );
        assert_eq!(v[1]["channel"], "global");
        assert!(!s.to_json().to_string().contains("hi bot"), "Jev's request");
        for i in 0..SUMMARY_HEARD as u32 {
            s.heard.push(line(300 + i, "x"));
        }
        assert_eq!(s.heard.lines().len(), SUMMARY_HEARD);
        assert_eq!(s.heard.lines()[0].from, 300, "the oldest went first");
    }

    #[test]
    fn the_spend_guard_pauses_at_either_ceiling_and_reopens_with_the_window() {
        let t0 = Instant::now();
        let mut g = SpendGuard::new(3, 5);
        for i in 0..3 {
            assert!(g.allow(t0 + Duration::from_secs(i)));
            g.record(t0 + Duration::from_secs(i));
        }
        assert!(!g.allow(t0 + Duration::from_secs(10)), "hour ceiling");
        let later = t0 + Duration::from_secs(3600);
        assert!(g.allow(later), "a new hour window");
        g.record(later);
        g.record(later);
        assert!(!g.allow(later + Duration::from_secs(3600)), "day ceiling");
        assert!(g.allow(t0 + Duration::from_secs(86400)));
        assert_eq!(g.counts().1, 3);
    }

    struct Counting(u32);
    impl DecisionSource for Counting {
        fn kind(&self) -> SourceKind {
            SourceKind::Jev
        }
        fn decide(&mut self, s: &Summary) -> Result<Choice, String> {
            self.0 += 1;
            Ok(Choice {
                goal: s.options()[0],
                confidence: 0.5,
                reason: Reason::EMPTY,
                input_tokens: 10,
                output_tokens: 1,
                say: Say::Quiet,
                code: None,
            })
        }
    }

    /// The free scripted policy is never paused by the spend cap; a source
    /// that costs money still is.
    #[test]
    fn the_scripted_policy_is_never_held_by_the_spend_cap() {
        let cfg = MindConfig {
            per_hour: 1,
            per_day: 1,
            ..MindConfig::default()
        };
        let mut mind = Mind::inline(Scripted::default(), cfg).unwrap();
        let mut s = Summary::EMPTY;
        s.offer(Goal::Explore);
        let t0 = Instant::now();
        for i in 0..10 {
            let t = t0 + cfg.interval * i;
            assert!(mind.ask(t, &s), "request {i}");
            assert!(mind.poll(t).is_some());
            assert_ne!(mind.mode(t), Mode::Paused);
        }
        let counts = mind.guard().counts();
        assert_eq!((counts.0, counts.2), (0, 0), "nothing counted against it");
        let mut paid = Mind::inline(Counting(0), cfg).unwrap();
        let t1 = Instant::now();
        assert!(paid.ask(t1, &s));
        assert!(paid.poll(t1).is_some());
        assert!(
            !paid.ask(t1 + cfg.interval, &s),
            "a paid source still stops"
        );
    }

    #[test]
    fn requests_never_exceed_the_floor_and_the_cap_pauses_visibly() {
        let cfg = MindConfig {
            per_hour: 2,
            ..MindConfig::default()
        };
        let mut mind = Mind::inline(Counting(0), cfg).unwrap();
        let mut s = Summary::EMPTY;
        s.offer(Goal::Explore);
        let t0 = Instant::now();
        assert!(mind.ask(t0, &s));
        assert!(!mind.ask(t0, &s), "one outstanding");
        assert_eq!(mind.poll(t0).unwrap().goal, Goal::Explore);
        assert!(
            !mind.ask(t0 + cfg.interval / 2, &s),
            "the once-per-second floor"
        );
        assert!(mind.ask(t0 + cfg.interval, &s));
        assert!(mind.poll(t0 + cfg.interval).is_some());
        assert!(!mind.ask(t0 + cfg.interval * 3, &s), "hour ceiling");
        assert_eq!(mind.mode(t0 + cfg.interval * 3), Mode::Paused);
        assert_eq!((mind.stats.requests, mind.stats.pauses), (2, 1));
        assert_eq!(mind.stats.input_tokens, 20);
        let next_hour = t0 + Duration::from_secs(3601);
        assert!(mind.ask(next_hour, &s));
        assert_eq!(mind.mode(next_hour), Mode::Asking);
    }

    #[test]
    fn late_answers_are_failures_and_back_off() {
        let cfg = MindConfig::default();
        let mut mind = Mind::inline(Counting(0), cfg).unwrap();
        let mut s = Summary::EMPTY;
        s.offer(Goal::Wait);
        let t0 = Instant::now();
        assert!(mind.ask(t0, &s));
        assert!(mind.poll(t0 + cfg.timeout).is_none(), "expired answer");
        assert_eq!((mind.stats.failures, mind.stats.late), (1, 1));
        let backoff = t0 + cfg.timeout;
        assert_eq!(mind.mode(backoff), Mode::BackingOff);
        assert!(!mind.ask(backoff + cfg.interval, &s));
        assert!(mind.ask(backoff + cfg.interval * 2, &s));
    }

    #[test]
    fn the_scripted_policy_only_picks_offered_goals() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        s.offer(Goal::Explore);
        s.offer(Goal::GatherWood);
        s.food_max = 100;
        s.water_max = 100;
        s.food = 100;
        s.water = 100;
        s.free_slots = 20;
        assert_eq!(scripted.pick(&s).0, Goal::Explore, "no tree in view");
        s.trees.add(10.0, 0);
        assert_eq!(scripted.pick(&s).0, Goal::GatherWood);
        // A pack nearly full keeps its last slots for what it makes.
        s.free_slots = SCRIPTED_STASH_FREE_SLOTS;
        assert_eq!(scripted.pick(&s).0, Goal::Explore, "no room to spare");
        s.free_slots = 20;
        s.water = 10;
        assert_eq!(scripted.pick(&s).0, Goal::GatherWood, "drink not offered");
        s.offer(Goal::Drink);
        assert_eq!(scripted.pick(&s).0, Goal::Drink);
        s.water = 100;
        let hatchet = name("Stone Hatchet");
        s.craftable[0] = hatchet;
        s.craftable_len = 1;
        s.offer(Goal::Craft(hatchet));
        assert_eq!(scripted.pick(&s).0, Goal::Craft(hatchet));
        s.items[0] = (hatchet, 1);
        s.items_len = 1;
        assert_ne!(scripted.pick(&s).0, Goal::Craft(hatchet), "already owned");
        // A heartbeat keeps a working goal, unless something is urgent.
        s.current = Some(Report {
            goal: Goal::GatherWood,
            outcome: Outcome::Running,
            gained: 150,
            secs: 30,
        });
        s.trigger = Trigger::Heartbeat;
        assert_eq!(scripted.pick(&s).0, Goal::GatherWood);
        s.water = 10;
        assert_eq!(scripted.pick(&s).0, Goal::Drink, "thirst outranks the work");
    }

    fn heard(channel: protocol::Channel, text: &str, agent: bool) -> HeardLine {
        HeardLine {
            from: 7,
            agent,
            channel,
            text: protocol::ChatText::sanitize(text.as_bytes()).unwrap(),
            ..HeardLine::EMPTY
        }
    }

    fn said(choice: &Choice) -> Option<String> {
        match choice.say {
            Say::Team(line) => Some(String::from_utf8_lossy(line.as_bytes()).into_owned()),
            _ => None,
        }
    }

    /// A teammate's words on team chat are orders: follow and stay stand
    /// until the next order, a run happens once, help is answered, and a
    /// number is the team's code. An agent's line and a local line are not.
    #[test]
    fn the_scripted_policy_takes_its_teams_orders() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        for g in [Goal::Explore, Goal::Wait, Goal::Follow, Goal::SupplyWood] {
            s.offer(g);
        }
        (s.food_max, s.water_max, s.food, s.water) = (100, 100, 100, 100);
        s.free_slots = 20;
        s.team.mates = 1;
        s.team.awake.add(6.0, 0);
        let mut ask = |s: &mut Summary, line: Option<HeardLine>| {
            s.heard.clear();
            if let Some(line) = line {
                s.heard.push(line);
            }
            scripted.decide(s).unwrap()
        };
        use protocol::Channel::{Local, Team};
        assert_eq!(
            ask(&mut s, None).goal,
            Goal::Explore,
            "near its teammate, it works"
        );

        let c = ask(&mut s, Some(heard(Team, "follow me", false)));
        assert_eq!(
            (c.goal, said(&c).as_deref()),
            (Goal::Follow, Some("following you"))
        );
        assert_eq!(ask(&mut s, None).goal, Goal::Follow, "a follow stands");
        assert_eq!(
            ask(&mut s, Some(heard(Team, "follow me", true))).goal,
            Goal::Follow
        );
        assert!(
            said(&ask(&mut s, Some(heard(Local, "stay", false)))).is_none(),
            "local is no order"
        );

        let c = ask(&mut s, Some(heard(Team, "stay here", false)));
        assert_eq!(
            (c.goal, said(&c).as_deref()),
            (Goal::Wait, Some("holding here"))
        );
        assert_eq!(
            ask(&mut s, Some(heard(Team, "stay", true))).goal,
            Goal::Wait
        );

        let c = ask(&mut s, Some(heard(Team, "go get wood", false)));
        assert_eq!(
            (c.goal, said(&c).as_deref()),
            (Goal::SupplyWood, Some("on it: wood"))
        );
        assert_eq!(
            ask(&mut s, None).goal,
            Goal::Explore,
            "a run is once, and ends the hold"
        );

        let c = ask(&mut s, Some(heard(Team, "get stone", false)));
        assert_eq!(
            said(&c).as_deref(),
            Some("show me your box first"),
            "not offered"
        );

        let c = ask(&mut s, Some(heard(Team, "help", false)));
        assert_eq!(said(&c).as_deref(), Some(orders::HELP));

        let c = ask(&mut s, Some(heard(Team, "code is 1234", false)));
        assert_eq!(c.code.map(crate::agent::lock::LockCode::wire), Some(1234));
        assert_eq!(said(&c).as_deref(), Some("got the code"));
        assert!(
            ask(&mut s, Some(heard(Team, "9999", true))).code.is_none(),
            "an agent's number"
        );

        // Off the team, an order is forgotten.
        ask(&mut s, Some(heard(Team, "follow", false)));
        s.team.mates = 0;
        assert_eq!(ask(&mut s, None).goal, Goal::Explore);
    }

    /// Its friend (`--friend`): met at THE GATE while online and not on its
    /// team, invited first once in reach, and named in what an external
    /// source is sent — their lines marked as theirs.
    #[test]
    fn the_scripted_policy_meets_and_invites_its_friend() {
        let friend =
            protocol::Address::from_hex(b"0x00000000000000000000000000000000000000f1").unwrap();
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        s.offer(Goal::Explore);
        (s.food_max, s.water_max, s.food, s.water) = (100, 100, 100, 100);
        s.free_slots = 20;
        assert_eq!(s.friend_json(), Value::Null, "no friend named");
        s.friend = FriendSense {
            set: true,
            who: Who {
                id: 7,
                name: protocol::Name::new("Ash").unwrap(),
                address: friend,
            },
            ..FriendSense::NONE
        };
        assert_eq!(scripted.decide(&s).unwrap().goal, Goal::Explore, "away");
        s.friend.online = true;
        s.offer(Goal::Meet);
        assert_eq!(scripted.decide(&s).unwrap().goal, Goal::Meet);
        s.friend.reachable = true;
        s.offer(Goal::Invite);
        let c = scripted.decide(&s).unwrap();
        assert_eq!(
            (c.goal, said(&c).as_deref()),
            (Goal::Invite, Some("invite sent: press Y"))
        );
        s.friend.on_team = true;
        assert_eq!(scripted.decide(&s).unwrap().goal, Goal::Explore, "teamed");
        let j = s.friend_json();
        assert_eq!(j["name"], "Ash");
        assert_eq!(j["wallet"], "0x00000000000000000000000000000000000000f1");
        assert_eq!(
            (j["online"].clone(), j["on_my_team"].clone()),
            (json!(true), json!(true))
        );
        s.heard.push(HeardLine {
            from: 7,
            name: protocol::Name::new("Ash").unwrap(),
            address: friend,
            text: protocol::ChatText::sanitize(b"hey doc").unwrap(),
            ..HeardLine::EMPTY
        });
        s.heard.push(heard(protocol::Channel::Local, "hi", false));
        let lines = s.heard_json();
        assert_eq!(
            (lines[0]["friend"].clone(), lines[1]["friend"].clone()),
            (json!(true), json!(false))
        );
    }

    /// A person near it asking to team up in local chat is invited when
    /// someone is in reach; else it says how.
    #[test]
    fn the_scripted_policy_invites_who_asks() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        s.offer(Goal::Explore);
        (s.food_max, s.water_max, s.food, s.water) = (100, 100, 100, 100);
        s.free_slots = 20;
        s.heard
            .push(heard(protocol::Channel::Local, "wanna team?", false));
        let c = scripted.decide(&s).unwrap();
        assert_eq!(c.goal, Goal::Explore);
        assert_eq!(said(&c).as_deref(), Some("come closer, or press N on me"));
        s.offer(Goal::Invite);
        let c = scripted.decide(&s).unwrap();
        assert_eq!(
            (c.goal, said(&c).as_deref()),
            (Goal::Invite, Some("invite sent: press Y"))
        );
        s.heard.clear();
        assert_eq!(
            scripted.decide(&s).unwrap().goal,
            Goal::Explore,
            "nobody asked"
        );
        s.heard.push(heard(protocol::Channel::Local, "team?", true));
        assert_eq!(
            scripted.decide(&s).unwrap().goal,
            Goal::Explore,
            "an agent asked"
        );
    }

    /// Hurt: heal when nobody is about, run only when nearly done, and
    /// otherwise leave the fight to the reflex. With stone tools, arm up
    /// in order, arrows only for a bow owned.
    #[test]
    fn the_scripted_policy_heals_runs_late_and_arms_up() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        for g in [Goal::Explore, Goal::Flee, Goal::Heal] {
            s.offer(g);
        }
        (s.hp, s.hp_max) = (50, 100);
        assert_eq!(scripted.pick(&s).0, Goal::Heal);
        s.threats_len = 1;
        s.players.add(4.0, 0);
        s.hits = 1;
        assert_eq!(scripted.pick(&s).0, Goal::Explore, "the reflex fights");
        s.hp = 20;
        assert_eq!(scripted.pick(&s).0, Goal::Flee);
        s = Summary::EMPTY;
        s.offer(Goal::Explore);
        let craft = |s: &mut Summary, n: &str| {
            let n = name(n);
            s.craftable[s.craftable_len as usize] = n;
            s.craftable_len += 1;
            s.offer(Goal::Craft(n));
        };
        for n in ["Wooden Spear", "Hunting Bow", "Wooden Arrow", "Bandage"] {
            craft(&mut s, n);
        }
        assert_eq!(scripted.pick(&s).0, Goal::Explore, "no stone tool yet");
        s.items[0] = (name("Stone Hatchet"), 1);
        s.items_len = 1;
        assert_eq!(scripted.pick(&s).0, Goal::Craft(name("Wooden Spear")));
        s.items[1] = (name("Wooden Spear"), 1);
        s.items_len = 2;
        assert_eq!(scripted.pick(&s).0, Goal::Craft(name("Hunting Bow")));
        // No cloth for the bow: arrows wait for one, bandages do not.
        s.craftable[1] = name("Bandage");
        s.craftable_len = 1;
        s.craftable[0] = name("Wooden Arrow");
        s.craftable_len = 2;
        assert_eq!(scripted.pick(&s).0, Goal::Craft(name("Bandage")));
        s.items[2] = (name("Hunting Bow"), 1);
        s.items_len = 3;
        assert_eq!(scripted.pick(&s).0, Goal::Craft(name("Wooden Arrow")));
    }

    #[test]
    fn with_stone_tools_the_playbook_makes_a_bag_and_puts_it_down() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        (s.food, s.food_max, s.water, s.water_max) = (100, 100, 100, 100);
        s.free_slots = 20;
        s.offer(Goal::Explore);
        s.offer(Goal::GatherHemp);
        s.hemp.add(10.0, 0);
        let bag = name(BAG_ITEM);
        s.craftable[0] = bag;
        s.craftable_len = 1;
        s.offer(Goal::Craft(bag));
        s.items[0] = (name("Stone Hatchet"), 1);
        s.items_len = 1;
        assert_ne!(scripted.pick(&s).0, Goal::Craft(bag), "a pickaxe first");
        s.items[1] = (name("Stone Pickaxe"), 1);
        s.items_len = 2;
        assert_eq!(scripted.pick(&s).0, Goal::Craft(bag));
        s.craftable_len = 0;
        assert_eq!(scripted.pick(&s).0, Goal::GatherHemp, "cloth for one");
        // A full pack is not why it was short of cloth: hemp does not help.
        s.free_slots = 0;
        assert_ne!(scripted.pick(&s).0, Goal::GatherHemp, "no room for one");
        s.free_slots = 20;
        s.items[2] = (bag, 1);
        s.items_len = 3;
        s.offer(Goal::Bag);
        assert_eq!(scripted.pick(&s).0, Goal::Bag);
        s.bags = 1;
        assert_ne!(scripted.pick(&s).0, Goal::Bag, "one is enough for now");
        // The backpack from a death, near and unwatched, comes before work.
        s.offer(Goal::Recover);
        s.current = Some(Report {
            goal: Goal::Forage,
            outcome: Outcome::Running,
            gained: 0,
            secs: 3,
        });
        assert_eq!(scripted.pick(&s).0, Goal::Recover);
    }

    #[test]
    fn with_a_bag_down_the_playbook_gathers_for_the_base_then_builds_it() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        (s.food, s.food_max, s.water, s.water_max) = (100, 100, 100, 100);
        s.free_slots = 20;
        s.items[0] = (name("Stone Hatchet"), 1);
        s.items[1] = (name("Stone Pickaxe"), 1);
        s.items_len = 2;
        s.bags = 1;
        for g in [
            Goal::Explore,
            Goal::GatherWood,
            Goal::GatherStone,
            Goal::Build,
        ] {
            s.offer(g);
        }
        s.trees.add(10.0, 0);
        s.stone_nodes.add(12.0, 2);
        s.needs[0] = (name("Stone"), 900);
        s.needs_len = 1;
        s.milestone = Milestone::Stone;
        assert_eq!(scripted.pick(&s).0, Goal::GatherStone, "the missing stone");
        // None in sight: what the pack pays for goes up meanwhile.
        s.stone_nodes = Sighting::default();
        assert_eq!(scripted.pick(&s).0, Goal::Build);
        s.needs_len = 0;
        assert_eq!(scripted.pick(&s).0, Goal::Build, "all in the pack");
        // Everything made: what the spare pays for (a metal door) goes up
        // while an op is on offer, and nothing more is built when none is.
        s.milestone = Milestone::Done;
        assert_eq!(scripted.pick(&s).0, Goal::Build);
        s.options_len = 3;
        assert!(!s.offers(Goal::Build));
        assert_eq!(scripted.pick(&s).0, Goal::GatherWood);
        assert_eq!(Goal::GoHome.label().as_str(), "go_home");
        assert_eq!(Goal::Stash.label().as_str(), "stash");
        assert!(Goal::FIXED
            .iter()
            .all(|g| g.label().as_str().len() <= LABEL_BYTES));
    }

    #[test]
    fn the_playbook_goes_home_for_the_box_and_the_cupboard() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        (s.food, s.food_max, s.water, s.water_max) = (100, 100, 100, 100);
        s.items[0] = (name("Stone Hatchet"), 1);
        s.items[1] = (name("Stone Pickaxe"), 1);
        s.items_len = 2;
        s.bags = 1;
        s.free_slots = 20;
        for g in [Goal::Explore, Goal::GatherWood, Goal::Build, Goal::Stash] {
            s.offer(g);
        }
        s.trees.add(10.0, 0);
        s.milestone = Milestone::Upstairs;
        assert_eq!(
            scripted.pick(&s).0,
            Goal::Build,
            "nothing to see to at home"
        );
        s.take_out = true;
        assert_eq!(
            scripted.pick(&s).0,
            Goal::Stash,
            "the box has what the base needs"
        );
        s.take_out = false;
        s.feed = true;
        assert_eq!(scripted.pick(&s).0, Goal::Stash, "the cupboard runs low");
        s.feed = false;
        s.free_slots = SCRIPTED_STASH_FREE_SLOTS;
        assert_eq!(scripted.pick(&s).0, Goal::Stash, "the pack is nearly full");
        // Not offered (no box, or a visit just failed): not chosen.
        s.options_len = 0;
        s.offer(Goal::Build);
        assert_eq!(scripted.pick(&s).0, Goal::Build);
        let v = s.to_json();
        assert_eq!(v["home"]["state"], "none");
        assert_eq!(v["cupboard_needs_feeding"], false);
    }

    /// Home under attack comes before everything but running and healing,
    /// mending a quiet one after the meters; a raid goes at night, ahead of
    /// the base's own work.
    #[test]
    fn the_playbook_defends_home_first_and_raids_by_night() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        (s.food, s.food_max, s.water, s.water_max) = (30, 100, 30, 100);
        s.items[0] = (name("Stone Hatchet"), 1);
        s.items[1] = (name("Stone Pickaxe"), 1);
        s.items_len = 2;
        s.bags = 1;
        s.free_slots = 20;
        for g in [Goal::Explore, Goal::Eat, Goal::Build, Goal::Raid] {
            s.offer(g);
        }
        s.milestone = Milestone::Upstairs;
        assert_eq!(scripted.pick(&s).0, Goal::Eat, "hungry, home is fine");
        s.offer(Goal::Defend);
        s.home.damaged = true;
        assert_eq!(scripted.pick(&s).0, Goal::Eat, "food before mending");
        s.home.attacked = true;
        assert_eq!(scripted.pick(&s).0, Goal::Defend, "home before food");
        s.options_len = 0;
        (s.food, s.water) = (100, 100);
        for g in [Goal::Explore, Goal::Build, Goal::Raid] {
            s.offer(g);
        }
        assert_eq!(scripted.pick(&s).0, Goal::Build, "by day, the base");
        s.night = true;
        assert_eq!(scripted.pick(&s).0, Goal::Raid, "by night, a raid");
        s.last = Some(Report {
            goal: Goal::Raid,
            outcome: Outcome::Done,
            gained: 300,
            secs: 40,
        });
        s.offer(Goal::GoHome);
        assert_eq!(scripted.pick(&s).0, Goal::GoHome, "then home with the take");
        s.raid.add(80.0, 2);
        let v = s.to_json();
        assert_eq!(v["raid_targets"]["count"], 1);
        assert_eq!(v["raid_targets"]["bearing"], "right");
    }

    /// The stations: fragments for the bench come from loot until a
    /// furnace stands; the furnace's fuel is crafted from what the pack
    /// holds, its fat hunted where an animal is in view; ore is smelted at
    /// the furnace before anything else is built; and a barrel or crate
    /// close by is taken on the way.
    #[test]
    fn the_playbook_loots_for_the_bench_and_smelts_at_the_furnace() {
        let mut scripted = Scripted::default();
        let mut s = Summary::EMPTY;
        (s.food, s.food_max, s.water, s.water_max) = (100, 100, 100, 100);
        s.items[0] = (name("Stone Hatchet"), 1);
        s.items[1] = (name("Stone Pickaxe"), 1);
        s.items_len = 2;
        s.bags = 1;
        s.free_slots = 20;
        for g in [Goal::Explore, Goal::GatherWood, Goal::Hunt, Goal::Loot] {
            s.offer(g);
        }
        s.trees.add(10.0, 0);
        s.milestone = Milestone::Bench;
        s.needs[0] = (name("Metal Fragments"), 100);
        s.needs_len = 1;
        s.raw[0] = s.needs[0];
        s.raw_len = 1;
        assert_eq!(scripted.pick(&s).0, Goal::Loot, "fragments are loot");
        // The furnace: fuel made from the pack when it can be...
        s.milestone = Milestone::Furnace;
        s.needs[0] = (name("Low Grade Fuel"), 50);
        s.raw[0] = (name("Animal Fat"), 39);
        s.raw[1] = (name("Cloth"), 13);
        s.raw_len = 2;
        let fuel = name("Low Grade Fuel");
        s.craftable[0] = fuel;
        s.craftable_len = 1;
        s.offer(Goal::Craft(fuel));
        assert_eq!(scripted.pick(&s).0, Goal::Craft(fuel));
        // ...else its fat hunted, with an animal in view, or looted.
        s.craftable_len = 0;
        s.animals.add(20.0, 1);
        assert_eq!(scripted.pick(&s).0, Goal::Hunt);
        s.animals = Sighting::default();
        assert_eq!(scripted.pick(&s).0, Goal::Loot);
        // Ore and a furnace: smelted first.
        s.furnace = true;
        s.items[2] = (name("Metal Ore"), 60);
        s.items_len = 3;
        let frags = name("Metal Fragments");
        s.craftable[0] = frags;
        s.craftable_len = 1;
        s.offer(Goal::Craft(frags));
        assert_eq!(scripted.pick(&s).0, Goal::Craft(frags));
        // A finished base, a barrel close by: taken on the way.
        s.items_len = 2;
        s.milestone = Milestone::Done;
        assert_eq!(scripted.pick(&s).0, Goal::GatherWood);
        s.loot.add(25.0, 0);
        assert_eq!(scripted.pick(&s).0, Goal::Loot);
        let v = s.to_json();
        assert_eq!(v["stations"]["furnace"], true);
        assert_eq!(v["loot_known"]["count"], 1);
        assert_eq!(Goal::Loot.label().as_str(), "loot");
    }
}
