//! **Achievements**, in Steam's shape (Achievement Sets, 2026-10-15): the
//! game's achievements grouped in sets, each set with its own progress and a
//! trophy when it is finished, progress stats behind the counting ones,
//! spoilers, and achievements that retire without being taken from anyone.
//!
//! The shard is the one that saw it happen, so the shard keeps them
//! (Steamworks' "set by official game server"). An unlock pops up in game
//! (`SUB_ACHIEVEMENT`, wire v118), a finished set goes to Discord, `/ach`
//! answers in chat, and Elo Pros reads `/achievements.json` off the status
//! thread and draws the page.
//!
//! ## Where this runs, and what it costs
//!
//! - **The tick.** `ShardCore::note_achievements` walks the tick's events
//!   once, after the standings. A tracked event finds its connection's row
//!   by slot and sets a bit or bumps a counter in place: no string, no map
//!   and no allocation. Events that name no actor (a piece placed, a charge
//!   stuck, an animal's corpse) are matched to the tick's own command or hit,
//!   and only when something tracked happened. One body a tick is swept for
//!   what it holds, wears and has learned, so a full shard is swept every
//!   [`MAX_PLAYERS`] ticks.
//! - **Every [`FLUSH_SECS`] s** the rows that moved are copied into a ring
//!   for the writer thread. Nothing is formatted on the sim thread except
//!   the popup of something just earned.
//! - **The writer thread** owns the clock (an unlock is stamped when it
//!   lands, so to within a flush), keeps every wallet the shard has seen,
//!   writes `<base>.achievements` (the record, read back at boot) and
//!   `<base>.achievements.json`, which the status thread serves, and posts a
//!   finished set to the shard's Discord webhook.
//!
//! Achievements are a career: a wipe archives the world and leaves these
//! files where they are (`wipe.rs` names what it moves).
//!
//! ## Changing the list
//!
//! A row in [`ACHIEVEMENTS`]. The record files rows by `id`, so the order is
//! free. Never remove or rename a row someone may hold: set `archived`. An
//! archived row is not awarded again, stays on the profiles that have it,
//! and stops counting towards its set. A set, once finished, stays finished
//! when rows are added to it. An icon is a stem of `assets/icons/`; the site
//! carries its own copies under `art/achievements/gates/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sim_core::build::{MAT_METAL, MAT_STONE, MAT_TWIG, SHAPE_FOUNDATION, SHAPE_TRI_FOUNDATION};
use sim_core::collide::Part;
use sim_core::gather::ItemStack;
use sim_core::limits::{MAX_PLAYERS, MOB_ID_TAG};
use sim_core::mob::MOB_WOLF;
use sim_core::world::{
    hit_part, SimEvent, EV_ARC_DID, EV_ASSIST, EV_CONSUMED, EV_CRAFT_DONE, EV_DEPLOY_PLACED,
    EV_DRANK, EV_EXTRACTED, EV_GATHER, EV_GAVE, EV_HIT, EV_RESEARCH, EV_RESPAWN, EV_STOCK,
    EV_SWIPE, EV_VEND, EV_VITALS, STRUCT_VICTIM,
};

/// How a row is earned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// `n` units of an item entering your pack over a career (`EV_GATHER`:
    /// a node, a pickup, a corpse, a give).
    Gather(&'static str, u32),
    /// `n` of an item in your pack at once (the pack sweep).
    Hold(&'static str, u32),
    /// Craft any one of these (`EV_CRAFT_DONE`).
    Craft(&'static [&'static str]),
    /// Place any one of these (`EV_DEPLOY_PLACED`; a lock, which mints no
    /// deployable, by the tick's own `PlaceDeploy` command).
    Place(&'static [&'static str]),
    /// Wear `n` pieces at once (the pack sweep).
    Wear(u32),
    /// Learn a recipe (`EV_RESEARCH`).
    Research,
    /// Buy from a kiosk (`EV_VEND`).
    Vend,
    /// Eat until the food bar is full. The full bars a body spawns with do
    /// not count: the bar has to climb to full after eating.
    FullFood,
    /// Drink until the water bar is full, on the same terms.
    FullWater,
    /// Place a building piece of one of these shapes (the tick's `Place`).
    Build(&'static [u8]),
    /// Place `n` building pieces over a career.
    Pieces(u32),
    /// Upgrade a piece to this material or better (`MAT_TWIG`: any upgrade).
    Upgrade(u8),
    /// Stock a hearth (`EV_STOCK`).
    Stock,
    /// Stick a charge to a structure (the tick's `Throw`).
    Throw,
    /// Kill an animal, of this kind if one is named (the killing hit, matched
    /// to the corpse it left).
    Kill(Option<u8>),
    /// Kill `n` animals over a career.
    Kills(u32),
    /// Kill another player (`EV_DEATH`).
    KillPlayer,
    /// Hit a player in the head (`EV_HIT`).
    Headshot,
    /// Kill a player from at least this many centimetres.
    LongShot(u16),
    /// Finish helping a downed player up (`EV_ASSIST`).
    Revive,
    /// Give to one of the works (`EV_GAVE`).
    Give,
    /// Light a work, this one if it is named (`EV_WORK`).
    Light(Option<&'static str>),
    /// Take coin out through the exchange (`EV_EXTRACTED`).
    Extract,
    /// Solve this mechanism (`EV_MECH_SOLVED`).
    Solve(&'static str),
    /// Talk to every speaker (`EV_ARC_DID`).
    TalkAll,
    /// Read an inscription (`EV_ARC_DID`).
    Read,
    /// Know every glyph the inscriptions teach (the pack sweep).
    Glyphs,
    /// Open a card door with this keycard (`EV_SWIPE`).
    Swipe(&'static str),
}

/// A set: Steam's Achievement Set.
#[derive(Clone, Copy, Debug)]
pub struct Set {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    /// An icon stem, or empty.
    pub icon: &'static str,
    /// Retired whole: nothing in it is awarded, and it shows only to those
    /// who earned something in it.
    pub archived: bool,
}

/// One achievement.
#[derive(Clone, Copy, Debug)]
pub struct Def {
    /// The record's name for it. Never renamed.
    pub id: &'static str,
    pub set: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    /// An icon stem, or empty.
    pub icon: &'static str,
    pub rule: Rule,
    /// Steam's "spoiler": the name and text stay hidden until earned.
    pub spoiler: bool,
    pub archived: bool,
}

const fn def(
    set: &'static str,
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    icon: &'static str,
    rule: Rule,
) -> Def {
    Def {
        id,
        set,
        name,
        desc,
        icon,
        rule,
        spoiler: false,
        archived: false,
    }
}

const fn survival(
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    icon: &'static str,
    rule: Rule,
) -> Def {
    def("survival", id, name, desc, icon, rule)
}

const fn building(
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    icon: &'static str,
    rule: Rule,
) -> Def {
    def("building", id, name, desc, icon, rule)
}

const fn combat(
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    icon: &'static str,
    rule: Rule,
) -> Def {
    def("combat", id, name, desc, icon, rule)
}

const fn arc(
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    icon: &'static str,
    rule: Rule,
) -> Def {
    def("arc", id, name, desc, icon, rule)
}

const fn spoiler(d: Def) -> Def {
    Def { spoiler: true, ..d }
}

pub const SETS: &[Set] = &[
    Set {
        id: "survival",
        name: "Survival",
        about: "The first hours on the island: gather, craft, and settle in.",
        icon: "camp_fire",
        archived: false,
    },
    Set {
        id: "building",
        name: "Building & Raiding",
        about: "Build a base worth keeping, and open other people's.",
        icon: "shape_foundation",
        archived: false,
    },
    Set {
        id: "combat",
        name: "Combat",
        about: "Hunt, fight, and keep your friends standing.",
        icon: "bat",
        archived: false,
    },
    Set {
        id: "arc",
        name: "The Arc",
        about: "Wake the island's works and learn what the stones say.",
        icon: "ui_star",
        archived: false,
    },
];

/// The list. Survival mirrors Rust's own first-hours achievements where
/// Gates has the same thing (a Tool Cupboard is a Hearth here, Scrap is
/// Junk); The Arc is Gates's own.
pub const ACHIEVEMENTS: &[Def] = &[
    // ---- Survival ----------------------------------------------------------
    survival(
        "COLLECT_100_WOOD",
        "Collect Wood",
        "Collect 100 Wood",
        "wood",
        Rule::Gather("item.wood", 100),
    ),
    survival(
        "COLLECT_700_WOOD",
        "Collect 700 Wood",
        "Collect 700 Wood",
        "wood",
        Rule::Gather("item.wood", 700),
    ),
    survival(
        "COLLECT_200_STONE",
        "Collect 200 Stone",
        "Collect 200 Stone",
        "stone",
        Rule::Gather("item.stone", 200),
    ),
    survival(
        "COLLECT_300_METAL_ORE",
        "Collect 300 Metal Ore",
        "Collect 300 Metal Ore",
        "metal_ore",
        Rule::Gather("item.metal_ore", 300),
    ),
    survival(
        "COLLECT_30_CLOTH",
        "Collect 30 Cloth",
        "Collect 30 Cloth",
        "cloth",
        Rule::Gather("item.cloth", 30),
    ),
    survival(
        "COLLECT_50_CLOTH",
        "Collect 50 Cloth",
        "Collect 50 Cloth",
        "cloth",
        Rule::Gather("item.cloth", 50),
    ),
    survival(
        "HOLD_65_JUNK",
        "Collect 65 Junk",
        "Hold 65 Junk at once",
        "junk",
        Rule::Hold("item.junk", 65),
    ),
    survival(
        "HOLD_50_LGF",
        "Acquire 50 Low Grade Fuel",
        "Hold 50 Low Grade Fuel at once",
        "low_grade_fuel",
        Rule::Hold("item.lowgrade", 50),
    ),
    survival(
        "CRAFT_STONE_HATCHET",
        "Craft Stone Hatchet",
        "Craft a Stone Hatchet",
        "stone_hatchet",
        Rule::Craft(&["item.hatchet_stone"]),
    ),
    survival(
        "CRAFT_STONE_PICKAXE",
        "Craft Stone Pickaxe",
        "Craft a Stone Pickaxe",
        "stone_pickaxe",
        Rule::Craft(&["item.pickaxe_stone"]),
    ),
    survival(
        "CRAFT_SPEAR",
        "Craft Spear",
        "Craft a Wooden Spear",
        "wooden_spear",
        Rule::Craft(&["item.spear_wood"]),
    ),
    survival(
        "CRAFT_BUILDING_PLAN",
        "Craft Building Plan",
        "Craft a Building Plan",
        "building_plan",
        Rule::Craft(&["item.building_plan"]),
    ),
    survival(
        "CRAFT_HAMMER",
        "Craft Hammer",
        "Craft a Hammer",
        "hammer",
        Rule::Craft(&["item.hammer"]),
    ),
    survival(
        "CRAFT_CAMPFIRE",
        "Craft Camp Fire",
        "Craft a Camp Fire",
        "camp_fire",
        Rule::Craft(&["item.fire_pit"]),
    ),
    survival(
        "CRAFT_SLEEPINGBAG",
        "Craft Sleeping Bag",
        "Craft a Sleeping Bag",
        "sleeping_bag",
        Rule::Craft(&["item.sleeping_bag"]),
    ),
    survival(
        "CRAFT_LOCK",
        "Craft Lock",
        "Craft a lock",
        "key_lock",
        Rule::Craft(&["item.lock_key", "item.lock_code"]),
    ),
    survival(
        "CRAFT_WOODEN_DOOR",
        "Craft Wooden Door",
        "Craft a Wooden Door",
        "wooden_door",
        Rule::Craft(&["item.door_wood"]),
    ),
    survival(
        "CRAFT_HEARTH",
        "Craft Hearth",
        "Craft a Hearth",
        "hearth",
        Rule::Craft(&["item.hearth"]),
    ),
    survival(
        "CRAFT_ARROWS",
        "Craft Arrows",
        "Craft some Arrows",
        "wooden_arrow",
        Rule::Craft(&[
            "item.arrow_wood",
            "item.arrow_bone",
            "item.arrow_fire",
            "item.arrow_metal",
            "item.arrow_hv",
        ]),
    ),
    survival(
        "CRAFT_HUNTING_BOW",
        "Craft Hunting Bow",
        "Craft a Hunting Bow",
        "hunting_bow",
        Rule::Craft(&["item.bow"]),
    ),
    survival(
        "CRAFT_BURLAP_TUNIC",
        "Craft Burlap Tunic",
        "Craft a Burlap Tunic",
        "burlap_tunic",
        Rule::Craft(&["item.armor_burlap_body"]),
    ),
    survival(
        "CRAFT_BURLAP_HOOD",
        "Craft Burlap Hood",
        "Craft a Burlap Hood",
        "burlap_hood",
        Rule::Craft(&["item.armor_burlap_head"]),
    ),
    survival(
        "CRAFT_WOODEN_BOX",
        "Craft Wooden Box",
        "Craft a Small Box",
        "small_box",
        Rule::Craft(&["item.box_small"]),
    ),
    survival(
        "CRAFT_FURNACE",
        "Craft a Furnace",
        "Craft a Furnace",
        "furnace",
        Rule::Craft(&["item.furnace"]),
    ),
    survival(
        "CRAFT_WORKBENCH",
        "Craft a Workbench",
        "Craft a Workbench",
        "workbench",
        Rule::Craft(&["item.workbench1"]),
    ),
    survival(
        "CRAFT_RESEARCH_TABLE",
        "Craft a Research Table",
        "Craft a Research Table",
        "research_table",
        Rule::Craft(&["item.research_table"]),
    ),
    survival(
        "PLACE_CAMPFIRE",
        "Place Camp Fire",
        "Place a Camp Fire",
        "camp_fire",
        Rule::Place(&["item.fire_pit"]),
    ),
    survival(
        "PLACE_SLEEPINGBAG",
        "Place Sleeping Bag",
        "Place a Sleeping Bag",
        "sleeping_bag",
        Rule::Place(&["item.sleeping_bag"]),
    ),
    survival(
        "PLACE_WOODEN_DOOR",
        "Place Wooden Door",
        "Place a Wooden Door",
        "wooden_door",
        Rule::Place(&["item.door_wood"]),
    ),
    survival(
        "PLACE_LOCK",
        "Place Lock",
        "Place a lock",
        "key_lock",
        Rule::Place(&["item.lock_key", "item.lock_code"]),
    ),
    survival(
        "PLACE_HEARTH",
        "Place Hearth",
        "Place a Hearth",
        "hearth",
        Rule::Place(&["item.hearth"]),
    ),
    survival(
        "PLACE_WOODEN_BOX",
        "Place Wooden Box",
        "Place a box",
        "small_box",
        Rule::Place(&["item.box_small", "item.box_large"]),
    ),
    survival(
        "PLACE_FURNACE",
        "Place a Furnace",
        "Place a Furnace",
        "furnace",
        Rule::Place(&["item.furnace"]),
    ),
    survival(
        "PLACE_WORKBENCH",
        "Place a Workbench",
        "Place a Workbench",
        "workbench",
        Rule::Place(&["item.workbench1", "item.workbench2", "item.workbench3"]),
    ),
    survival(
        "PLACE_RESEARCH_TABLE",
        "Place a Research Table",
        "Place a Research Table",
        "research_table",
        Rule::Place(&["item.research_table"]),
    ),
    survival(
        "RESEARCH_ITEM",
        "Research an Item",
        "Learn a recipe",
        "blueprint",
        Rule::Research,
    ),
    survival(
        "KIOSK_PURCHASE",
        "First Purchase",
        "Buy something at a kiosk in THE GATE",
        "junk",
        Rule::Vend,
    ),
    survival(
        "EQUIP_CLOTHING",
        "Equip Clothing",
        "Wear clothing in both slots",
        "burlap_tunic",
        Rule::Wear(2),
    ),
    spoiler(survival(
        "GLUTTON",
        "Glutton",
        "Completely fill your food bar",
        "vital_food",
        Rule::FullFood,
    )),
    spoiler(survival(
        "SEALBREAKER",
        "Sealbreaker",
        "Completely fill your hydration bar",
        "vital_water",
        Rule::FullWater,
    )),
    // ---- Building & Raiding --------------------------------------------------
    building(
        "BUILD_FOUNDATION",
        "Construct Base",
        "Place a foundation",
        "shape_foundation",
        Rule::Build(&[SHAPE_FOUNDATION, SHAPE_TRI_FOUNDATION]),
    ),
    building(
        "BUILD_50_PIECES",
        "Architect",
        "Place 50 building pieces",
        "shape_foundation",
        Rule::Pieces(50),
    ),
    building(
        "UPGRADE_BASE",
        "Upgrade Base",
        "Upgrade a building piece",
        "verb_upgrade",
        Rule::Upgrade(MAT_TWIG),
    ),
    building(
        "UPGRADE_STONE",
        "Set in Stone",
        "Upgrade a piece to stone",
        "stone",
        Rule::Upgrade(MAT_STONE),
    ),
    building(
        "UPGRADE_METAL",
        "Sheet Metal",
        "Upgrade a piece to metal",
        "sheet_metal",
        Rule::Upgrade(MAT_METAL),
    ),
    building(
        "STOCK_HEARTH",
        "Upkeep",
        "Stock a hearth",
        "hearth",
        Rule::Stock,
    ),
    building(
        "PLACE_CODE_LOCK",
        "Code Lock",
        "Place a code lock",
        "code_lock",
        Rule::Place(&["item.lock_code"]),
    ),
    building(
        "PLACE_METAL_DOOR",
        "Metal Door",
        "Place a metal door",
        "metal_door",
        Rule::Place(&["item.door_metal"]),
    ),
    building(
        "PLACE_LARGE_BOX",
        "Storage",
        "Place a large box",
        "large_box",
        Rule::Place(&["item.box_large"]),
    ),
    building(
        "PLACE_WORKBENCH_2",
        "Workbench Level 2",
        "Place a level 2 workbench",
        "workbench_2",
        Rule::Place(&["item.workbench2"]),
    ),
    building(
        "PLACE_WORKBENCH_3",
        "Workbench Level 3",
        "Place a level 3 workbench",
        "workbench_3",
        Rule::Place(&["item.workbench3"]),
    ),
    building(
        "PLACE_RECYCLER",
        "Recycler",
        "Place a recycler",
        "recycler",
        Rule::Place(&["item.recycler"]),
    ),
    building(
        "PLACE_PLANTER",
        "Green Thumb",
        "Place a planter box",
        "planter_box",
        Rule::Place(&["item.planter"]),
    ),
    building(
        "PLACE_TURRET",
        "Sentry Duty",
        "Place an auto turret",
        "auto_turret",
        Rule::Place(&["item.autoturret"]),
    ),
    building(
        "CRAFT_SATCHEL",
        "Breach Kit",
        "Craft a satchel charge",
        "satchel_charge",
        Rule::Craft(&["item.satchel_charge"]),
    ),
    building(
        "THROW_SATCHEL",
        "Raider",
        "Stick a satchel charge to a wall",
        "satchel_charge",
        Rule::Throw,
    ),
    // ---- Combat ------------------------------------------------------------
    combat(
        "KILL_ANIMAL",
        "Kill an Animal",
        "Kill an animal",
        "raw_meat",
        Rule::Kill(None),
    ),
    combat(
        "KILL_WOLF",
        "Wolfsbane",
        "Kill a wolf",
        "hunting_bow",
        Rule::Kill(Some(MOB_WOLF)),
    ),
    combat(
        "KILL_10_ANIMALS",
        "Hunter",
        "Kill 10 animals",
        "raw_meat",
        Rule::Kills(10),
    ),
    combat(
        "COOK_MEAT",
        "Cook",
        "Have cooked meat in your pack",
        "cooked_meat",
        Rule::Hold("item.cooked_meat", 1),
    ),
    combat(
        "FIRST_BLOOD",
        "First Blood",
        "Kill another player",
        "bat",
        Rule::KillPlayer,
    ),
    combat(
        "HEADSHOT",
        "Headshot",
        "Hit a player in the head",
        "hunting_bow",
        Rule::Headshot,
    ),
    combat(
        "LONG_SHOT",
        "Long Shot",
        "Kill a player from 50 m away",
        "crossbow",
        Rule::LongShot(5_000),
    ),
    combat(
        "REVIVE",
        "Medic",
        "Help a downed player up",
        "medkit",
        Rule::Revive,
    ),
    combat(
        "CRAFT_CROSSBOW",
        "Crossbow",
        "Craft a crossbow",
        "crossbow",
        Rule::Craft(&["item.crossbow"]),
    ),
    combat(
        "CRAFT_REVOLVER",
        "Six Shooter",
        "Craft a revolver",
        "revolver",
        Rule::Craft(&["item.revolver"]),
    ),
    // ---- The Arc -------------------------------------------------------------
    arc(
        "GIVE_TO_WORK",
        "Contributor",
        "Give to one of the island's works",
        "gears",
        Rule::Give,
    ),
    arc(
        "LIGHT_WORK",
        "Spark",
        "Light one of the works",
        "ui_star",
        Rule::Light(None),
    ),
    arc(
        "LIGHT_CRUCIBLE",
        "The Crucible",
        "Light THE CRUCIBLE",
        "furnace",
        Rule::Light(Some("THE CRUCIBLE")),
    ),
    arc(
        "LIGHT_WELLS",
        "The Lapis Wells",
        "Light THE LAPIS WELLS",
        "vital_water",
        Rule::Light(Some("THE LAPIS WELLS")),
    ),
    arc(
        "LIGHT_OBSERVATORY",
        "The Observatory",
        "Light THE OBSERVATORY",
        "ui_star",
        Rule::Light(Some("THE OBSERVATORY")),
    ),
    arc(
        "LIGHT_SPINE",
        "The Golden Spine",
        "Light THE GOLDEN SPINE",
        "gears",
        Rule::Light(Some("THE GOLDEN SPINE")),
    ),
    arc(
        "LIGHT_EXCHANGE",
        "The Exchange",
        "Light THE EXCHANGE",
        "junk",
        Rule::Light(Some("THE EXCHANGE")),
    ),
    arc(
        "EXTRACT",
        "Way Out",
        "Take Junk out through THE EXCHANGE",
        "junk",
        Rule::Extract,
    ),
    arc(
        "RING_LOCK",
        "The Ring Lock",
        "Open THE RING LOCK",
        "ui_lock",
        Rule::Solve("THE RING LOCK"),
    ),
    arc(
        "TALK_ALL",
        "Small Talk",
        "Talk to every speaker at THE GATE",
        "ui_star",
        Rule::TalkAll,
    ),
    arc(
        "READ_INSCRIPTION",
        "Stone Reader",
        "Read an inscription",
        "blueprint",
        Rule::Read,
    ),
    spoiler(arc(
        "ALL_GLYPHS",
        "Lettered",
        "Learn every glyph the stones teach",
        "blueprint",
        Rule::Glyphs,
    )),
    arc(
        "SWIPE_GREEN",
        "Green Room",
        "Open a door with a green keycard",
        "green_keycard",
        Rule::Swipe("item.keycard_green"),
    ),
    arc(
        "SWIPE_BLUE",
        "Blue Room",
        "Open a door with a blue keycard",
        "blue_keycard",
        Rule::Swipe("item.keycard_blue"),
    ),
    arc(
        "SWIPE_RED",
        "Red Room",
        "Open a door with a red keycard",
        "red_keycard",
        Rule::Swipe("item.keycard_red"),
    ),
];

/// Rows are bits of a `u128`. Past this the row type widens.
pub const MAX_DEFS: usize = 128;
const _: () = assert!(ACHIEVEMENTS.len() <= MAX_DEFS);
/// Progress stats a row can carry.
pub const MAX_COUNTERS: usize = 16;
/// Wallets kept in memory: every one the shard has seen, about 200 bytes
/// each. Past this a new wallet is not tracked and `refused` counts it.
pub const MAX_WALLETS: usize = 1 << 16;
/// How often the rows that moved go to the writer.
pub const FLUSH_SECS: u64 = 30;
/// Popups owed between pumps, at most.
pub const TELL_CAP: usize = 64;
/// Batches queued for the writer. One is written in well under a flush, so
/// a backlog means the disk is stuck; a full ring keeps the rows marked
/// moved and the next flush sends them again.
const WRITE_RING: usize = 4;
/// The most of the public file the status thread serves, and the writer's
/// bound on it.
pub const JSON_CAP: u64 = 8 * 1024 * 1024;
/// Finished sets one flush may post to Discord; the rest are only written.
const DISCORD_PER_FLUSH: usize = 5;

const NONE_U8: u8 = u8::MAX;
const NONE_U32: u32 = u32::MAX;
/// A tell for a finished set rather than a row.
const TELL_SET: u8 = 0x80;
/// Building shapes the build table indexes by.
const SHAPES: usize = 32;
/// Mob kinds the kill table indexes by.
const KINDS: usize = 8;

pub fn record_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.achievements", base.display()))
}

pub fn json_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.achievements.json", base.display()))
}

// ---- the bake ----------------------------------------------------------------

/// How a progress stat moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterKind {
    /// Summed: units gathered, pieces placed, animals killed.
    Sum,
    /// The most seen at once: held, worn, glyphs known.
    Max,
    /// Bits seen, OR-ed: the speakers talked to. Published as the count.
    Mask,
}

/// One progress stat.
#[derive(Clone, Debug)]
struct Counter {
    /// `gather:item.wood`, `hold:item.junk`, `wear`, `talk:speakers`, …
    name: String,
    kind: CounterKind,
    /// The item the sweep counts, for a `hold:` stat.
    hold: Option<u16>,
    /// `(target, row)`, for every row this counter earns.
    steps: Vec<(u32, u8)>,
}

/// What the bake reads off the content. Built by [`Baked::bake_from`]; tests
/// build their own.
pub struct Lookup<'a> {
    pub n_items: usize,
    pub item: &'a dyn Fn(&str) -> Option<u16>,
    /// Each deploy row's item.
    pub deploy_items: &'a [u16],
    /// Work names, by work index.
    pub works: &'a [String],
    /// Mechanism names, by mechanism index.
    pub mechs: &'a [String],
    /// The keycard each card door takes, by door.
    pub cards: &'a [u16],
    /// How many speakers there are.
    pub speakers: u32,
    /// Every glyph some inscription teaches.
    pub glyphs: u64,
}

/// The one-off triggers, by what fires them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flag {
    Research,
    Vend,
    FullFood,
    FullWater,
    Stock,
    Throw,
    KillPlayer,
    Headshot,
    Revive,
    Give,
    Extract,
    Read,
    LightAny,
    KillAny,
}
const FLAGS: usize = 14;

/// The list as tables the tick indexes by number.
#[derive(Clone, Debug)]
pub struct Baked {
    /// By item index: the gather counter it feeds, or `NONE_U8`.
    gather: Vec<u8>,
    /// By item index: the rows crafting it earns.
    craft: Vec<u128>,
    /// By deploy row: the rows placing it earns.
    place: Vec<u128>,
    /// By building shape: the rows placing one earns.
    build: [u128; SHAPES],
    /// By material: the rows an upgrade to it earns (it or anything less).
    upgrade: [u128; 4],
    /// By mob kind: the rows killing one earns.
    kill: [u128; KINDS],
    /// By work index.
    light: Vec<u128>,
    /// By mechanism index.
    solve: Vec<u128>,
    /// By card door.
    swipe: Vec<u128>,
    flag: [u128; FLAGS],
    /// `(cm, row)`: a kill from this far earns it.
    long_shot: Vec<(u16, u8)>,
    counters: Vec<Counter>,
    c_pieces: u8,
    c_kills: u8,
    c_talk: u8,
    c_glyphs: u8,
    c_wear: u8,
    glyph_mask: u64,
    /// By set: its live rows.
    sets: Vec<u128>,
    /// By row: its progress stat and target, Steamworks' "progress stat".
    stats: Vec<Option<(String, u32)>>,
    /// Rows that name something the content lacks, as `id: what`. Never
    /// earnable; the boot log names them.
    pub unresolved: Vec<String>,
    /// Where a finished set is posted (`shard.toml` `discord_webhook`).
    pub discord: Option<String>,
}

impl Default for Baked {
    fn default() -> Self {
        Baked {
            gather: Vec::new(),
            craft: Vec::new(),
            place: Vec::new(),
            build: [0; SHAPES],
            upgrade: [0; 4],
            kill: [0; KINDS],
            light: Vec::new(),
            solve: Vec::new(),
            swipe: Vec::new(),
            flag: [0; FLAGS],
            long_shot: Vec::new(),
            counters: Vec::new(),
            c_pieces: NONE_U8,
            c_kills: NONE_U8,
            c_talk: NONE_U8,
            c_glyphs: NONE_U8,
            c_wear: NONE_U8,
            glyph_mask: 0,
            sets: Vec::new(),
            stats: Vec::new(),
            unresolved: Vec::new(),
            discord: None,
        }
    }
}

impl Baked {
    pub fn bake(defs: &[Def], sets: &[Set], l: &Lookup) -> Baked {
        let mut b = Baked {
            gather: vec![NONE_U8; l.n_items],
            craft: vec![0; l.n_items],
            place: vec![0; l.deploy_items.len()],
            light: vec![0; l.works.len()],
            solve: vec![0; l.mechs.len()],
            swipe: vec![0; l.cards.len()],
            sets: vec![0; sets.len()],
            stats: vec![None; defs.len().min(MAX_DEFS)],
            glyph_mask: l.glyphs,
            ..Baked::default()
        };
        for (i, d) in defs.iter().enumerate().take(MAX_DEFS) {
            let set = sets.iter().position(|s| s.id == d.set);
            if d.archived || set.is_some_and(|s| sets[s].archived) {
                continue;
            }
            let bit = 1u128 << i;
            let mut missing: Vec<String> = Vec::new();
            let item = |id: &str, missing: &mut Vec<String>| -> Option<usize> {
                let got = (l.item)(id).map(usize::from).filter(|&n| n < l.n_items);
                if got.is_none() {
                    missing.push(id.to_string());
                }
                got
            };
            let step =
                |b: &mut Baked, name: String, kind: CounterKind, hold: Option<u16>, n: u32| {
                    match counter(&mut b.counters, name.clone(), kind, hold) {
                        Some(c) => {
                            b.counters[c as usize].steps.push((n, i as u8));
                            b.stats[i] = Some((name, n));
                            Some(c)
                        }
                        None => None,
                    }
                };
            let ok = match d.rule {
                Rule::Gather(id, n) => match item(id, &mut missing) {
                    Some(it) => {
                        match step(&mut b, format!("gather:{id}"), CounterKind::Sum, None, n) {
                            Some(c) => {
                                b.gather[it] = c;
                                true
                            }
                            None => false,
                        }
                    }
                    None => false,
                },
                Rule::Hold(id, n) => match item(id, &mut missing) {
                    Some(it) => step(
                        &mut b,
                        format!("hold:{id}"),
                        CounterKind::Max,
                        Some(it as u16),
                        n,
                    )
                    .is_some(),
                    None => false,
                },
                Rule::Wear(n) => match step(&mut b, "wear".into(), CounterKind::Max, None, n) {
                    Some(c) => {
                        b.c_wear = c;
                        true
                    }
                    None => false,
                },
                Rule::Pieces(n) => {
                    match step(&mut b, "build:pieces".into(), CounterKind::Sum, None, n) {
                        Some(c) => {
                            b.c_pieces = c;
                            true
                        }
                        None => false,
                    }
                }
                Rule::Kills(n) => {
                    match step(&mut b, "kill:animals".into(), CounterKind::Sum, None, n) {
                        Some(c) => {
                            b.c_kills = c;
                            true
                        }
                        None => false,
                    }
                }
                Rule::TalkAll => {
                    l.speakers > 0
                        && match step(
                            &mut b,
                            "talk:speakers".into(),
                            CounterKind::Mask,
                            None,
                            l.speakers,
                        ) {
                            Some(c) => {
                                b.c_talk = c;
                                true
                            }
                            None => false,
                        }
                }
                Rule::Glyphs => {
                    let n = l.glyphs.count_ones();
                    n > 0
                        && match step(&mut b, "glyphs".into(), CounterKind::Max, None, n) {
                            Some(c) => {
                                b.c_glyphs = c;
                                true
                            }
                            None => false,
                        }
                }
                Rule::Craft(ids) => {
                    let mut any = false;
                    for id in ids {
                        if let Some(it) = item(id, &mut missing) {
                            b.craft[it] |= bit;
                            any = true;
                        }
                    }
                    any && missing.is_empty()
                }
                Rule::Place(ids) => {
                    let mut any = false;
                    for id in ids {
                        if let Some(it) = item(id, &mut missing) {
                            for (row, &di) in l.deploy_items.iter().enumerate() {
                                if usize::from(di) == it {
                                    b.place[row] |= bit;
                                    any = true;
                                }
                            }
                        }
                    }
                    if !any && missing.is_empty() {
                        missing.push("no deployable".into());
                    }
                    any && missing.is_empty()
                }
                Rule::Build(shapes) => {
                    for &s in shapes {
                        if let Some(m) = b.build.get_mut(usize::from(s)) {
                            *m |= bit;
                        }
                    }
                    true
                }
                Rule::Upgrade(mat) => {
                    for m in usize::from(mat)..b.upgrade.len() {
                        b.upgrade[m] |= bit;
                    }
                    true
                }
                Rule::Kill(kind) => {
                    match kind {
                        Some(k) => match b.kill.get_mut(usize::from(k)) {
                            Some(m) => *m |= bit,
                            None => missing.push(format!("mob kind {k}")),
                        },
                        None => b.flag[Flag::KillAny as usize] |= bit,
                    }
                    missing.is_empty()
                }
                Rule::Light(name) => {
                    match name {
                        Some(n) => match l.works.iter().position(|w| w == n) {
                            Some(w) => b.light[w] |= bit,
                            None => missing.push(n.to_string()),
                        },
                        None => b.flag[Flag::LightAny as usize] |= bit,
                    }
                    missing.is_empty()
                }
                Rule::Solve(name) => match l.mechs.iter().position(|m| m == name) {
                    Some(m) => {
                        b.solve[m] |= bit;
                        true
                    }
                    None => {
                        missing.push(name.to_string());
                        false
                    }
                },
                Rule::Swipe(card) => match item(card, &mut missing) {
                    Some(it) => {
                        let mut any = false;
                        for (door, &c) in l.cards.iter().enumerate() {
                            if usize::from(c) == it {
                                b.swipe[door] |= bit;
                                any = true;
                            }
                        }
                        if !any {
                            missing.push(format!("a door for {card}"));
                        }
                        any
                    }
                    None => false,
                },
                Rule::LongShot(cm) => {
                    b.long_shot.push((cm, i as u8));
                    true
                }
                Rule::Research => b.set_flag(Flag::Research, bit),
                Rule::Vend => b.set_flag(Flag::Vend, bit),
                Rule::FullFood => b.set_flag(Flag::FullFood, bit),
                Rule::FullWater => b.set_flag(Flag::FullWater, bit),
                Rule::Stock => b.set_flag(Flag::Stock, bit),
                Rule::Throw => b.set_flag(Flag::Throw, bit),
                Rule::KillPlayer => b.set_flag(Flag::KillPlayer, bit),
                Rule::Headshot => b.set_flag(Flag::Headshot, bit),
                Rule::Revive => b.set_flag(Flag::Revive, bit),
                Rule::Give => b.set_flag(Flag::Give, bit),
                Rule::Extract => b.set_flag(Flag::Extract, bit),
                Rule::Read => b.set_flag(Flag::Read, bit),
            };
            for m in &missing {
                b.unresolved.push(format!("{}: {m}", d.id));
            }
            if let (true, Some(s)) = (ok, set) {
                b.sets[s] |= bit;
            }
        }
        b
    }

    fn set_flag(&mut self, f: Flag, bit: u128) -> bool {
        self.flag[f as usize] |= bit;
        true
    }

    /// The shipped list against the shipped content.
    pub fn bake_from(c: &content::Content, deploy: &sim_core::deploy::DeployContent) -> Baked {
        let rows = &deploy.defs[..usize::from(deploy.def_count)];
        let deploy_items: Vec<u16> = rows.iter().map(|d| d.item).collect();
        let works = c.bake_work_names();
        let mechs = c.bake_arc_text().map(|t| t.mech_names).unwrap_or_default();
        let cards = c.bake_cards().map(|c| c.to_vec()).unwrap_or_default();
        let (speakers, glyphs) = c.bake_lore().map_or((0, 0), |lore| {
            let n = usize::from(lore.n_inscriptions);
            let glyphs = lore.inscriptions[..n].iter().fold(0, |m, i| m | i.teaches);
            (u32::from(lore.n_speakers), glyphs)
        });
        let item = |id: &str| c.item_index(id);
        let l = Lookup {
            n_items: c.items.len(),
            item: &item,
            deploy_items: &deploy_items,
            works: &works,
            mechs: &mechs,
            cards: &cards,
            speakers,
            glyphs,
        };
        Baked::bake(ACHIEVEMENTS, SETS, &l)
    }

    /// The stats by counter index: what the writer files them under.
    pub fn counter_meta(&self) -> Vec<(String, CounterKind)> {
        self.counters
            .iter()
            .map(|c| (c.name.clone(), c.kind))
            .collect()
    }

    /// Each row's progress stat and target, by row.
    pub fn stats(&self) -> Vec<Option<(String, u32)>> {
        self.stats.clone()
    }
}

fn counter(
    cs: &mut Vec<Counter>,
    name: String,
    kind: CounterKind,
    hold: Option<u16>,
) -> Option<u8> {
    if let Some(i) = cs.iter().position(|c| c.name == name) {
        return Some(i as u8);
    }
    if cs.len() == MAX_COUNTERS {
        return None;
    }
    cs.push(Counter {
        name,
        kind,
        hold,
        steps: Vec::new(),
    });
    Some((cs.len() - 1) as u8)
}

// ---- the sim side ------------------------------------------------------------

/// One wallet's achievements, as the tick keeps them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Bit `i` is `defs[i]` earned.
    pub got: u128,
    /// By counter index.
    pub stats: [u32; MAX_COUNTERS],
}

/// What a connection's bars last read, for the climb to full.
#[derive(Clone, Copy, Debug, Default)]
struct Bars {
    ate: bool,
    drank: bool,
    /// Below full at the last `EV_VITALS`.
    food_low: bool,
    water_low: bool,
}

/// What the writer is handed: absolute rows, so sending one twice is
/// harmless, each with the name the standings know the wallet by.
pub struct Batch {
    rows: Vec<(String, String, Row)>,
}

/// `ShardCore::achievements`.
pub struct Achievements {
    defs: &'static [Def],
    sets: &'static [Set],
    baked: Baked,
    rows: Vec<Row>,
    keys: Vec<String>,
    index: BTreeMap<String, u32>,
    /// By connection slot: its row, or `NONE_U32`.
    slot: [u32; MAX_PLAYERS],
    tracked: usize,
    bars: [Bars; MAX_PLAYERS],
    moved: Vec<u32>,
    moved_flag: Vec<bool>,
    tells: [(u8, u8); TELL_CAP],
    tells_n: usize,
    sweep: usize,
    out: Option<rtrb::Producer<Batch>>,
    writer: Option<std::thread::JoinHandle<()>>,
    /// Wallets not tracked because the table was full.
    pub refused: u64,
    /// Popups dropped because the tell queue was full.
    pub tells_dropped: u64,
}

impl Default for Achievements {
    fn default() -> Self {
        Self::off()
    }
}

impl Achievements {
    /// Tracks nothing: a shard with no world file, and tests that do not
    /// care.
    pub fn off() -> Self {
        Self::new(ACHIEVEMENTS, SETS, Baked::default())
    }

    /// Tracking `defs` in memory, writing nothing.
    pub fn new(defs: &'static [Def], sets: &'static [Set], baked: Baked) -> Self {
        Achievements {
            defs,
            sets,
            baked,
            rows: Vec::new(),
            keys: Vec::new(),
            index: BTreeMap::new(),
            slot: [NONE_U32; MAX_PLAYERS],
            tracked: 0,
            bars: [Bars::default(); MAX_PLAYERS],
            moved: Vec::with_capacity(4 * MAX_PLAYERS),
            moved_flag: Vec::new(),
            tells: [(0, 0); TELL_CAP],
            tells_n: 0,
            sweep: 0,
            out: None,
            writer: None,
            refused: 0,
            tells_dropped: 0,
        }
    }

    /// The shipped list, read back from `<base>.achievements`, with a writer
    /// thread for the flushes. No `base`: kept in memory only.
    pub fn open(baked: Baked, base: Option<&Path>) -> Self {
        let mut a = Self::new(ACHIEVEMENTS, SETS, baked);
        let Some(base) = base else {
            return a;
        };
        let record = std::fs::read_to_string(record_path(base))
            .map(|t| parse_record(&t))
            .unwrap_or_default();
        a.seed(&record);
        let (tx, mut rx) = rtrb::RingBuffer::<Batch>::new(WRITE_RING);
        let w = Writer {
            defs: a.defs,
            sets: a.sets,
            counters: a.baked.counter_meta(),
            stats: a.baked.stats(),
            discord: a.baked.discord.clone(),
        };
        let (rec_path, json) = (record_path(base), json_path(base));
        let spawned = std::thread::Builder::new()
            .name("achievements".into())
            .spawn(move || {
                let mut mirror = record;
                loop {
                    match rx.pop() {
                        Ok(batch) => {
                            let now = unix_now();
                            let finished = w.apply(&mut mirror, &batch, now);
                            if let Err(e) = write_atomic(&rec_path, &render_record(&mirror)) {
                                eprintln!("achievements {}: {e}", rec_path.display());
                            }
                            if let Err(e) = write_atomic(&json, &w.render_json(&mirror, now)) {
                                eprintln!("achievements {}: {e}", json.display());
                            }
                            w.post(&finished);
                        }
                        Err(_) if rx.is_abandoned() => return,
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
                    }
                }
            });
        if let Ok(h) = spawned {
            a.out = Some(tx);
            a.writer = Some(h);
        }
        a
    }

    /// Rows for every wallet the record names.
    fn seed(&mut self, record: &BTreeMap<String, Rec>) {
        let meta = self.baked.counter_meta();
        for (key, rec) in record {
            let Some(i) = self.row_of(key) else {
                continue;
            };
            let row = &mut self.rows[i as usize];
            for (bit, d) in self.defs.iter().enumerate().take(MAX_DEFS) {
                if rec.got.contains_key(d.id) {
                    row.got |= 1 << bit;
                }
            }
            for (c, (name, _)) in meta.iter().enumerate() {
                if let Some(&v) = rec.stats.get(name) {
                    row.stats[c] = v.min(u64::from(u32::MAX)) as u32;
                }
            }
        }
    }

    /// The row for `key`, made if new. `None` when the table is full.
    fn row_of(&mut self, key: &str) -> Option<u32> {
        if let Some(&i) = self.index.get(key) {
            return Some(i);
        }
        if self.rows.len() >= MAX_WALLETS {
            self.refused += 1;
            return None;
        }
        let i = self.rows.len() as u32;
        self.rows.push(Row::default());
        self.keys.push(key.to_string());
        self.moved_flag.push(false);
        self.index.insert(key.to_string(), i);
        Some(i)
    }

    /// Connection `slot` is `key` now (`None`: a guest, tracked as nobody).
    /// The one place this allocates, and only for a wallet it has not seen.
    pub fn join(&mut self, slot: usize, key: Option<&str>) {
        self.leave(slot);
        let Some(key) = key.filter(|k| crate::standings::is_wallet(k)) else {
            return;
        };
        let Some(i) = self.row_of(key) else {
            return;
        };
        self.slot[slot] = i;
        self.tracked += 1;
        // A first join is a player the page should count.
        self.mark(i);
    }

    pub fn leave(&mut self, slot: usize) {
        if self.slot[slot] != NONE_U32 {
            self.tracked -= 1;
        }
        self.slot[slot] = NONE_U32;
        self.bars[slot] = Bars::default();
    }

    /// Anyone connected with a wallet: when nobody is, the tick skips the
    /// walk entirely.
    pub fn tracking(&self) -> bool {
        self.tracked > 0
    }

    pub fn tracks(&self, slot: usize) -> bool {
        self.slot[slot] != NONE_U32
    }

    /// The row on `slot`, for tests and the admin's eye.
    pub fn row(&self, slot: usize) -> Option<&Row> {
        self.rows.get(self.slot[slot] as usize)
    }

    fn mark(&mut self, i: u32) {
        let f = &mut self.moved_flag[i as usize];
        if !*f {
            *f = true;
            self.moved.push(i);
        }
    }

    /// Earn `bits` on `slot`'s row: whatever is new is marked, told, and
    /// may finish a set.
    fn earn(&mut self, slot: usize, bits: u128) {
        if bits == 0 {
            return;
        }
        let i = self.slot[slot];
        let Some(row) = self.rows.get_mut(i as usize) else {
            return;
        };
        let new = bits & !row.got;
        if new == 0 {
            return;
        }
        row.got |= new;
        let got = row.got;
        self.mark(i);
        let mut left = new;
        while left != 0 {
            let bit = left.trailing_zeros() as u8;
            left &= left - 1;
            self.tell(slot, bit);
        }
        for s in 0..self.baked.sets.len() {
            let m = self.baked.sets[s];
            if m != 0 && m & new != 0 && got & m == m {
                self.tell(slot, TELL_SET | s as u8);
            }
        }
    }

    fn tell(&mut self, slot: usize, what: u8) {
        if self.tells_n == TELL_CAP {
            self.tells_dropped += 1;
            return;
        }
        self.tells[self.tells_n] = (slot as u8, what);
        self.tells_n += 1;
    }

    fn bump(&mut self, slot: usize, c: u8, v: u32) {
        if c == NONE_U8 {
            return;
        }
        let i = self.slot[slot];
        let Some(row) = self.rows.get_mut(i as usize) else {
            return;
        };
        let counter = &self.baked.counters[c as usize];
        let s = &mut row.stats[c as usize];
        let was = *s;
        *s = match counter.kind {
            CounterKind::Sum => was.saturating_add(v),
            CounterKind::Max => was.max(v),
            CounterKind::Mask => was | v,
        };
        if *s == was {
            return;
        }
        let now = match counter.kind {
            CounterKind::Mask => s.count_ones(),
            _ => *s,
        };
        self.mark(i);
        let mut earned = 0u128;
        for &(n, bit) in &self.baked.counters[c as usize].steps {
            if now >= n {
                earned |= 1 << bit;
            }
        }
        self.earn(slot, earned);
    }

    fn flag(&self, f: Flag) -> u128 {
        self.baked.flag[f as usize]
    }

    /// One of the tick's events whose `a` is this connection's player.
    /// Events this does not track cost a match arm.
    pub fn event(&mut self, slot: usize, ev: SimEvent) {
        if !self.tracks(slot) {
            return;
        }
        match ev.code {
            EV_GATHER => {
                let (item, units) = ((ev.b >> 16) as usize, ev.b & 0xffff);
                let c = self.baked.gather.get(item).copied().unwrap_or(NONE_U8);
                if units > 0 {
                    self.bump(slot, c, units);
                }
            }
            EV_CRAFT_DONE => {
                let bits = self.baked.craft.get((ev.b >> 16) as usize).copied();
                self.earn(slot, bits.unwrap_or(0));
            }
            EV_DEPLOY_PLACED => self.placed(slot, (ev.b & 0xff) as u16),
            EV_RESEARCH => self.earn(slot, self.flag(Flag::Research)),
            EV_VEND => self.earn(slot, self.flag(Flag::Vend)),
            EV_STOCK => self.earn(slot, self.flag(Flag::Stock)),
            EV_GAVE => self.earn(slot, self.flag(Flag::Give)),
            EV_EXTRACTED => self.earn(slot, self.flag(Flag::Extract)),
            EV_ARC_DID => {
                let target = (ev.c >> 8) & 0xff;
                match ev.b as u8 {
                    sim_core::lore::OP_TALK if target < 32 => {
                        self.bump(slot, self.baked.c_talk, 1 << target)
                    }
                    sim_core::lore::OP_READ => self.earn(slot, self.flag(Flag::Read)),
                    _ => {}
                }
            }
            // A card, not the inside lever.
            EV_SWIPE if ev.c == 0 => {
                let bits = self.baked.swipe.get(ev.b as usize).copied();
                self.earn(slot, bits.unwrap_or(0));
            }
            EV_ASSIST if ev.c == u32::from(sim_core::assist::ASSIST_TICKS) => {
                self.earn(slot, self.flag(Flag::Revive))
            }
            EV_HIT => {
                let player = ev.b & MOB_ID_TAG == 0 && ev.b != STRUCT_VICTIM && ev.b != ev.a;
                if player && hit_part(ev.c) == Part::Head {
                    self.earn(slot, self.flag(Flag::Headshot));
                }
            }
            EV_CONSUMED => {
                // A meal may carry water too; the bar decides.
                self.bars[slot].ate = true;
                self.bars[slot].drank = true;
            }
            EV_DRANK => self.bars[slot].drank = true,
            EV_RESPAWN => self.bars[slot] = Bars::default(),
            EV_VITALS => {
                let (food, water) = ((ev.b >> 16) as u16, ev.b as u16);
                let (max_food, max_water) = ((ev.c >> 16) as u16, ev.c as u16);
                let b = self.bars[slot];
                let mut bits = 0;
                if max_food > 0 && food >= max_food && b.food_low && b.ate {
                    bits |= self.flag(Flag::FullFood);
                }
                if max_water > 0 && water >= max_water && b.water_low && b.drank {
                    bits |= self.flag(Flag::FullWater);
                }
                let b = &mut self.bars[slot];
                b.food_low = food < max_food;
                b.water_low = water < max_water;
                self.earn(slot, bits);
            }
            _ => {}
        }
    }

    /// `slot` placed deploy row `row`.
    pub fn placed(&mut self, slot: usize, row: u16) {
        let bits = self.baked.place.get(usize::from(row)).copied();
        self.earn(slot, bits.unwrap_or(0));
    }

    /// `slot` placed a building piece of `shape`.
    pub fn built(&mut self, slot: usize, shape: u8) {
        if !self.tracks(slot) {
            return;
        }
        let bits = self.baked.build.get(usize::from(shape)).copied();
        self.earn(slot, bits.unwrap_or(0));
        self.bump(slot, self.baked.c_pieces, 1);
    }

    /// `slot` upgraded a piece to `material`.
    pub fn upgraded(&mut self, slot: usize, material: u8) {
        let bits = self.baked.upgrade.get(usize::from(material)).copied();
        self.earn(slot, bits.unwrap_or(0));
    }

    /// `slot` stuck a charge to a structure.
    pub fn thrown(&mut self, slot: usize) {
        self.earn(slot, self.flag(Flag::Throw));
    }

    /// `slot` lit work `work`.
    pub fn lit(&mut self, slot: usize, work: usize) {
        let bits = self.baked.light.get(work).copied().unwrap_or(0);
        self.earn(slot, bits | self.flag(Flag::LightAny));
    }

    /// `slot` solved mechanism `mech`.
    pub fn solved(&mut self, slot: usize, mech: usize) {
        let bits = self.baked.solve.get(mech).copied();
        self.earn(slot, bits.unwrap_or(0));
    }

    /// `slot` killed another player from `range_cm`.
    pub fn killed_player(&mut self, slot: usize, range_cm: u16) {
        let mut bits = self.flag(Flag::KillPlayer);
        for &(cm, bit) in &self.baked.long_shot {
            if range_cm >= cm {
                bits |= 1 << bit;
            }
        }
        self.earn(slot, bits);
    }

    /// `slot` killed an animal of `kind`.
    pub fn killed_animal(&mut self, slot: usize, kind: u8) {
        if !self.tracks(slot) {
            return;
        }
        let bits = self.baked.kill.get(usize::from(kind)).copied().unwrap_or(0);
        self.earn(slot, bits | self.flag(Flag::KillAny));
        self.bump(slot, self.baked.c_kills, 1);
    }

    /// Which connection slot the pack sweep looks at this tick.
    pub fn sweep_next(&mut self) -> usize {
        let s = self.sweep;
        self.sweep = (self.sweep + 1) % MAX_PLAYERS;
        s
    }

    /// The pack sweep's look at one body: what it holds, wears and has
    /// learned from the stones.
    pub fn swept(&mut self, slot: usize, inv: &[ItemStack], worn: &[ItemStack], glyphs: u64) {
        if !self.tracks(slot) {
            return;
        }
        for c in 0..self.baked.counters.len() {
            if let Some(item) = self.baked.counters[c].hold {
                let n: u32 = inv
                    .iter()
                    .filter(|s| s.count > 0 && s.item == item)
                    .map(|s| u32::from(s.count))
                    .sum();
                if n > 0 {
                    self.bump(slot, c as u8, n);
                }
            }
        }
        let worn = worn.iter().filter(|s| s.count > 0).count() as u32;
        if worn > 0 {
            self.bump(slot, self.baked.c_wear, worn);
        }
        let known = (glyphs & self.baked.glyph_mask).count_ones();
        if known > 0 {
            self.bump(slot, self.baked.c_glyphs, known);
        }
    }

    /// The popups owed since the last pump, as `(slot, what to draw)`. Built
    /// here, at most [`TELL_CAP`] a tick, and only when something was earned.
    pub fn take_tells(&mut self, mut f: impl FnMut(usize, protocol::Achieved)) {
        let n = std::mem::take(&mut self.tells_n);
        for &(slot, what) in &self.tells[..n] {
            let slot = usize::from(slot);
            let got = self.rows.get(self.slot[slot] as usize).map_or(0, |r| r.got);
            let text = |s: &str| protocol::ChatText::sanitize(s.as_bytes());
            if what & TELL_SET != 0 {
                let s = usize::from(what & !TELL_SET);
                let (Some(set), Some(&m)) = (self.sets.get(s), self.baked.sets.get(s)) else {
                    continue;
                };
                let Some(name) = text(set.name) else {
                    continue;
                };
                let n = m.count_ones().min(255) as u8;
                f(
                    slot,
                    protocol::Achieved {
                        set: true,
                        name,
                        desc: None,
                        icon: text(set.icon),
                        done: n,
                        total: n,
                    },
                );
            } else if let Some(d) = self.defs.get(usize::from(what)) {
                let Some(name) = text(d.name) else {
                    continue;
                };
                let m = self
                    .sets
                    .iter()
                    .position(|s| s.id == d.set)
                    .and_then(|s| self.baked.sets.get(s))
                    .copied()
                    .unwrap_or(0);
                f(
                    slot,
                    protocol::Achieved {
                        set: false,
                        name,
                        desc: text(d.desc),
                        icon: text(d.icon),
                        done: (got & m).count_ones().min(255) as u8,
                        total: m.count_ones().min(255) as u8,
                    },
                );
            }
        }
    }

    /// What `/ach` says to `slot`: each set's count, and the counting row
    /// nearest done. Short lines, for chat.
    pub fn lines(&self, slot: usize) -> Vec<String> {
        let Some(row) = self.row(slot) else {
            return vec!["Achievements need a wallet sign-in".into()];
        };
        let mut out = Vec::new();
        for (s, set) in self.sets.iter().enumerate() {
            let m = self.baked.sets.get(s).copied().unwrap_or(0);
            if m == 0 {
                continue;
            }
            let done = (row.got & m).count_ones();
            let tick = if done == m.count_ones() {
                " - done"
            } else {
                ""
            };
            out.push(format!("{}: {done}/{}{tick}", set.name, m.count_ones()));
        }
        let mut best: Option<(u64, String)> = None;
        for (i, d) in self.defs.iter().enumerate().take(MAX_DEFS) {
            let live = self.baked.sets.iter().any(|m| m & (1 << i) != 0);
            if !live || row.got & (1 << i) != 0 || d.spoiler {
                continue;
            }
            let Some((stat, max)) = self.baked.stats.get(i).cloned().flatten() else {
                continue;
            };
            let Some(c) = self.baked.counters.iter().position(|c| c.name == stat) else {
                continue;
            };
            let raw = row.stats[c];
            let v = match self.baked.counters[c].kind {
                CounterKind::Mask => raw.count_ones(),
                _ => raw,
            }
            .min(max);
            let per_mille = u64::from(v) * 1000 / u64::from(max.max(1));
            if v > 0 && best.as_ref().is_none_or(|(b, _)| per_mille > *b) {
                best = Some((per_mille, format!("Next: {} {v}/{max}", d.name)));
            }
        }
        out.extend(best.map(|(_, l)| l));
        out
    }

    /// Hand the rows that moved to the writer, each with the name `label`
    /// knows its wallet by. Called every [`FLUSH_SECS`]; allocates one
    /// batch, sized by what moved.
    pub fn flush(&mut self, label: impl Fn(&str) -> Option<String>) {
        let Some(out) = self.out.as_mut() else {
            // Nothing to write to: forget what moved so the list stays small.
            for &i in &self.moved {
                self.moved_flag[i as usize] = false;
            }
            self.moved.clear();
            return;
        };
        if self.moved.is_empty() || out.is_full() {
            return;
        }
        let rows = self
            .moved
            .iter()
            .map(|&i| {
                let key = &self.keys[i as usize];
                let name = label(key).unwrap_or_else(|| crate::standings::short_label(key));
                (key.clone(), name, self.rows[i as usize])
            })
            .collect();
        if out.push(Batch { rows }).is_ok() {
            for &i in &self.moved {
                self.moved_flag[i as usize] = false;
            }
            self.moved.clear();
        }
    }

    /// The shutdown's last word: flush, and wait for the writer to land it.
    pub fn close(&mut self, label: impl Fn(&str) -> Option<String>) {
        // A full ring is the writer mid-write: give it up to two seconds to
        // make room, so the last flush is not the one that gets skipped.
        for _ in 0..40 {
            self.flush(&label);
            if self.moved.is_empty() || self.out.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.out = None;
        if let Some(h) = self.writer.take() {
            let _ = h.join();
        }
    }
}

// ---- the writer side ---------------------------------------------------------

/// One wallet's achievements as the files keep them, by name. Names the
/// shipped list no longer has are kept as they are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rec {
    /// Row id → unix seconds earned.
    pub got: BTreeMap<String, u64>,
    /// Stat name → value (a speaker mask is kept as the mask).
    pub stats: BTreeMap<String, u64>,
    /// Set id → unix seconds finished.
    pub sets: BTreeMap<String, u64>,
}

/// What the writer thread knows: the list, the stats' names and kinds, and
/// where to post.
pub struct Writer {
    pub defs: &'static [Def],
    pub sets: &'static [Set],
    pub counters: Vec<(String, CounterKind)>,
    pub stats: Vec<Option<(String, u32)>>,
    pub discord: Option<String>,
}

impl Writer {
    /// Fold a batch into the mirror at `now`: new rows are stamped, stats
    /// replaced, and a set stamped the first time its live rows are all
    /// held. A stamp is never moved or removed. Returns who finished which
    /// set just now, as `(name, set name)`.
    pub fn apply(
        &self,
        mirror: &mut BTreeMap<String, Rec>,
        batch: &Batch,
        now: u64,
    ) -> Vec<(String, &'static str)> {
        let mut finished = Vec::new();
        for (key, name, row) in &batch.rows {
            let rec = mirror.entry(key.clone()).or_default();
            for (bit, d) in self.defs.iter().enumerate().take(MAX_DEFS) {
                if row.got & (1 << bit) != 0 {
                    rec.got.entry(d.id.to_string()).or_insert(now);
                }
            }
            for (c, (stat, kind)) in self.counters.iter().enumerate() {
                let v = u64::from(row.stats[c]);
                if v > 0 {
                    let s = rec.stats.entry(stat.clone()).or_insert(0);
                    *s = match kind {
                        CounterKind::Mask => *s | v,
                        _ => (*s).max(v),
                    };
                }
            }
            for set in self.sets {
                if !rec.sets.contains_key(set.id) && set_done(rec, set, self.defs) {
                    rec.sets.insert(set.id.to_string(), now);
                    finished.push((name.clone(), set.name));
                }
            }
        }
        finished
    }

    /// Post finished sets to Discord, a few a flush. Blocking and bounded
    /// (`standings::post_discord`), on this thread and never the sim's.
    pub fn post(&self, finished: &[(String, &'static str)]) {
        let Some(url) = self.discord.as_deref() else {
            return;
        };
        for (name, set) in finished.iter().take(DISCORD_PER_FLUSH) {
            let text = format!("{name} finished the {set} achievements in Gates.");
            if let Err(e) = crate::standings::post_discord(url, &text) {
                eprintln!("achievements: {e}");
            }
        }
    }

    /// `/achievements.json`: the list, how many have each, and every
    /// wallet's own. Wallets past [`JSON_CAP`] are left out and `truncated`
    /// says so.
    pub fn render_json(&self, mirror: &BTreeMap<String, Rec>, now: u64) -> String {
        let icon = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let sets_v: Vec<serde_json::Value> = self
            .sets
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id, "name": s.name, "about": s.about, "icon": icon(s.icon),
                    "archived": s.archived,
                    "earned": mirror.values().filter(|r| r.sets.contains_key(s.id)).count(),
                })
            })
            .collect();
        let defs_v: Vec<serde_json::Value> = self
            .defs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let progress = self.stats.get(i).cloned().flatten();
                serde_json::json!({
                    "id": d.id, "set": d.set, "name": d.name, "desc": d.desc, "icon": icon(d.icon),
                    "spoiler": d.spoiler, "archived": d.archived,
                    "progress": progress.map(|(stat, max)| serde_json::json!({"stat": stat, "max": max})),
                    "earned": mirror.values().filter(|r| r.got.contains_key(d.id)).count(),
                })
            })
            .collect();
        let head = serde_json::json!({
            "v": 1, "at": now, "players": mirror.len(), "sets": sets_v, "achievements": defs_v,
        })
        .to_string();
        let masks: Vec<&str> = self
            .counters
            .iter()
            .filter(|(_, k)| *k == CounterKind::Mask)
            .map(|(n, _)| n.as_str())
            .collect();
        // `{"v":…}` opened up to take the wallets as one more key.
        let mut out = String::with_capacity(head.len() + 64 * mirror.len());
        out.push_str(&head[..head.len() - 1]);
        out.push_str(",\"wallets\":{");
        let mut first = true;
        let mut truncated = false;
        for (key, rec) in mirror {
            if rec.got.is_empty() && rec.stats.is_empty() {
                continue;
            }
            // A mask is published as how many: the page draws a count.
            let stats: BTreeMap<&str, u64> = rec
                .stats
                .iter()
                .map(|(n, &v)| {
                    let v = if masks.contains(&n.as_str()) {
                        u64::from(v.count_ones())
                    } else {
                        v
                    };
                    (n.as_str(), v)
                })
                .collect();
            let w =
                serde_json::json!({"got": rec.got, "stats": stats, "sets": rec.sets}).to_string();
            let k = serde_json::Value::String(key.clone()).to_string();
            if (out.len() + k.len() + w.len() + 64) as u64 > JSON_CAP {
                truncated = true;
                break;
            }
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&k);
            out.push(':');
            out.push_str(&w);
        }
        out.push_str(&format!("}},\"truncated\":{truncated}}}"));
        out
    }
}

/// Every live row of `set` is held. A set with no live rows is never done.
pub fn set_done(rec: &Rec, set: &Set, defs: &[Def]) -> bool {
    if set.archived {
        return false;
    }
    let mut live = defs
        .iter()
        .filter(|d| d.set == set.id && !d.archived)
        .peekable();
    live.peek().is_some() && live.all(|d| rec.got.contains_key(d.id))
}

/// `<base>.achievements`: one wallet a line,
/// `0x… a:ID@unix … c:stat=value … s:set@unix …`.
pub fn render_record(mirror: &BTreeMap<String, Rec>) -> String {
    let mut out = String::from("# gates achievements v1\n");
    for (key, rec) in mirror {
        out.push_str(key);
        for (id, at) in &rec.got {
            out.push_str(&format!(" a:{id}@{at}"));
        }
        for (name, v) in &rec.stats {
            out.push_str(&format!(" c:{name}={v}"));
        }
        for (id, at) in &rec.sets {
            out.push_str(&format!(" s:{id}@{at}"));
        }
        out.push('\n');
    }
    out
}

/// [`render_record`] backwards. A line it cannot read is skipped whole.
pub fn parse_record(text: &str) -> BTreeMap<String, Rec> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.split_ascii_whitespace();
        let Some(key) = parts.next().filter(|k| !k.starts_with('#')) else {
            continue;
        };
        let mut rec = Rec::default();
        let mut ok = true;
        for p in parts {
            let parsed = if let Some(t) = p.strip_prefix("a:") {
                t.rsplit_once('@')
                    .and_then(|(id, at)| Some(rec.got.insert(id.into(), at.parse().ok()?)))
            } else if let Some(t) = p.strip_prefix("c:") {
                t.rsplit_once('=')
                    .and_then(|(n, v)| Some(rec.stats.insert(n.into(), v.parse().ok()?)))
            } else if let Some(t) = p.strip_prefix("s:") {
                t.rsplit_once('@')
                    .and_then(|(id, at)| Some(rec.sets.insert(id.into(), at.parse().ok()?)))
            } else {
                None
            };
            if parsed.is_none() {
                ok = false;
                break;
            }
        }
        if ok {
            out.insert(key.to_string(), rec);
        }
    }
    out
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W1: &str = "0x00000000000000000000000000000000000000a1";
    const W2: &str = "0x00000000000000000000000000000000000000b2";

    /// Items 0..6: wood, junk, axe, bag, cloth, shirt; 6 is the red card.
    const ITEMS: [&str; 7] = ["wood", "junk", "axe", "bag", "cloth", "shirt", "red"];
    /// Deploy rows: bag, box.
    const DEPLOYS: [u16; 2] = [3, 99];

    const T_SETS: &[Set] = &[
        Set {
            id: "a",
            name: "Alpha",
            about: "",
            icon: "ui_star",
            archived: false,
        },
        Set {
            id: "b",
            name: "Beta",
            about: "",
            icon: "",
            archived: false,
        },
        Set {
            id: "c",
            name: "Gamma",
            about: "",
            icon: "",
            archived: false,
        },
    ];
    const T_DEFS: &[Def] = &[
        def(
            "a",
            "WOOD_10",
            "Wood",
            "Ten wood",
            "wood",
            Rule::Gather("wood", 10),
        ),
        def(
            "a",
            "WOOD_20",
            "More Wood",
            "",
            "wood",
            Rule::Gather("wood", 20),
        ),
        def("a", "AXE", "Axe", "", "", Rule::Craft(&["axe"])),
        def("b", "BAG", "Bag", "", "", Rule::Place(&["bag"])),
        def("b", "JUNK", "Junk", "", "", Rule::Hold("junk", 5)),
        Def {
            archived: true,
            ..def("b", "OLD", "Old", "", "", Rule::Craft(&["shirt"]))
        },
        spoiler(def("b", "FED", "Fed", "", "", Rule::FullFood)),
        def("b", "GONE", "Gone", "", "", Rule::Craft(&["nope"])),
        def(
            "c",
            "FOUND",
            "Found",
            "",
            "",
            Rule::Build(&[SHAPE_FOUNDATION]),
        ),
        def("c", "PIECES_3", "Pieces", "", "", Rule::Pieces(3)),
        def("c", "STONE", "Stone", "", "", Rule::Upgrade(MAT_STONE)),
        def("c", "WOLF", "Wolf", "", "", Rule::Kill(Some(MOB_WOLF))),
        def("c", "KILLS_2", "Kills", "", "", Rule::Kills(2)),
        def("c", "PVP", "Pvp", "", "", Rule::KillPlayer),
        def("c", "FAR", "Far", "", "", Rule::LongShot(5_000)),
        def("c", "HEAD", "Head", "", "", Rule::Headshot),
        def("c", "TALK", "Talk", "", "", Rule::TalkAll),
        def("c", "GLYPHS", "Glyphs", "", "", Rule::Glyphs),
        def("c", "LIT_B", "Lit B", "", "", Rule::Light(Some("WORK B"))),
        def("c", "LIT", "Lit", "", "", Rule::Light(None)),
        def("c", "SOLVE", "Solve", "", "", Rule::Solve("LOCK")),
        def("c", "RED", "Red", "", "", Rule::Swipe("red")),
        def("c", "THROW", "Throw", "", "", Rule::Throw),
        def("c", "REVIVE", "Revive", "", "", Rule::Revive),
    ];

    fn idx(id: &str) -> u32 {
        T_DEFS.iter().position(|d| d.id == id).unwrap() as u32
    }

    fn bit(id: &str) -> u128 {
        1 << idx(id)
    }

    fn baked() -> Baked {
        let item = |id: &str| ITEMS.iter().position(|i| *i == id).map(|i| i as u16);
        let works = ["WORK A".to_string(), "WORK B".to_string()];
        let mechs = ["LOCK".to_string()];
        let l = Lookup {
            n_items: ITEMS.len(),
            item: &item,
            deploy_items: &DEPLOYS,
            works: &works,
            mechs: &mechs,
            cards: &[6, 4],
            speakers: 3,
            glyphs: 0b1111,
        };
        Baked::bake(T_DEFS, T_SETS, &l)
    }

    fn ach() -> Achievements {
        Achievements::new(T_DEFS, T_SETS, baked())
    }

    fn pack(item: u32, units: u32) -> u32 {
        (item << 16) | units
    }

    fn ev(code: u8, a: u32, b: u32, c: u32) -> SimEvent {
        SimEvent { code, a, b, c }
    }

    fn told(a: &mut Achievements) -> Vec<(usize, String, bool)> {
        let mut out = Vec::new();
        a.take_tells(|s, t| {
            out.push((
                s,
                String::from_utf8_lossy(t.name.as_bytes()).into_owned(),
                t.set,
            ))
        });
        out
    }

    fn stack(item: u16, count: u16) -> ItemStack {
        ItemStack {
            item,
            count,
            cond: 0,
            skin: 0,
        }
    }

    fn got(a: &Achievements, slot: usize) -> u128 {
        a.row(slot).unwrap().got
    }

    #[test]
    fn a_counter_climbs_through_its_steps_and_each_unlock_pops_once() {
        let mut a = ach();
        a.join(3, Some(W1));
        a.event(3, ev(EV_GATHER, 7, pack(0, 6), 0));
        assert_eq!(got(&a, 3), 0);
        assert_eq!(a.row(3).unwrap().stats[0], 6);
        a.event(3, ev(EV_GATHER, 7, pack(0, 6), 0));
        assert_eq!(got(&a, 3), bit("WOOD_10"));
        a.event(3, ev(EV_GATHER, 7, pack(0, 30), 0));
        assert_eq!(a.row(3).unwrap().stats[0], 42, "the stat keeps counting");
        assert_eq!(
            told(&mut a),
            vec![(3, "Wood".into(), false), (3, "More Wood".into(), false)]
        );
        a.event(3, ev(EV_GATHER, 7, pack(0, 30), 0));
        assert!(told(&mut a).is_empty(), "nothing new, nothing said");
    }

    #[test]
    fn the_popup_carries_the_line_the_icon_and_the_sets_progress() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.event(0, ev(EV_GATHER, 1, pack(0, 10), 0));
        let mut seen = Vec::new();
        a.take_tells(|_, t| seen.push(t));
        let t = seen[0];
        assert_eq!(
            t.desc.map(|d| d.as_bytes().to_vec()),
            Some(b"Ten wood".to_vec())
        );
        assert_eq!(
            t.icon.map(|d| d.as_bytes().to_vec()),
            Some(b"wood".to_vec())
        );
        assert_eq!((t.done, t.total), (1, 3));
        a.event(0, ev(EV_GATHER, 1, pack(0, 10), 0));
        a.event(0, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        let mut seen = Vec::new();
        a.take_tells(|_, t| seen.push(t));
        let set = seen.iter().find(|t| t.set).expect("the set is told");
        assert_eq!((set.done, set.total), (3, 3));
        assert_eq!(
            set.icon.map(|d| d.as_bytes().to_vec()),
            Some(b"ui_star".to_vec())
        );
    }

    #[test]
    fn crafting_placing_and_holding_earn() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.event(0, ev(EV_DEPLOY_PLACED, 0, 0, 1));
        assert_eq!(got(&a, 0) & bit("BAG"), bit("BAG"), "row 0 is the bag");
        a.event(0, ev(EV_DEPLOY_PLACED, 0, 1, 1));
        let inv = [stack(1, 3), stack(1, 2), stack(0, 9)];
        a.swept(0, &inv, &[], 0);
        assert_eq!(
            got(&a, 0) & bit("JUNK"),
            bit("JUNK"),
            "5 junk across two stacks"
        );
    }

    #[test]
    fn building_raiding_and_the_arc_earn_through_their_joins() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.built(0, SHAPE_FOUNDATION);
        a.built(0, 1);
        assert_eq!(got(&a, 0) & bit("FOUND"), bit("FOUND"));
        assert_eq!(got(&a, 0) & bit("PIECES_3"), 0);
        a.built(0, 1);
        assert_eq!(got(&a, 0) & bit("PIECES_3"), bit("PIECES_3"));
        a.upgraded(0, sim_core::build::MAT_WOOD);
        assert_eq!(got(&a, 0) & bit("STONE"), 0);
        a.upgraded(0, MAT_METAL);
        assert_eq!(
            got(&a, 0) & bit("STONE"),
            bit("STONE"),
            "metal is stone or better"
        );
        a.thrown(0);
        a.lit(0, 0);
        assert_eq!(got(&a, 0) & (bit("LIT") | bit("LIT_B")), bit("LIT"));
        a.lit(0, 1);
        assert_eq!(got(&a, 0) & bit("LIT_B"), bit("LIT_B"));
        a.solved(0, 0);
        a.event(0, ev(EV_SWIPE, 1, 1, 0));
        assert_eq!(got(&a, 0) & bit("RED"), 0, "door 1 takes the cloth card");
        a.event(0, ev(EV_SWIPE, 1, 0, 1));
        assert_eq!(got(&a, 0) & bit("RED"), 0, "the inside lever is no card");
        a.event(0, ev(EV_SWIPE, 1, 0, 0));
        let want = bit("THROW") | bit("SOLVE") | bit("RED");
        assert_eq!(got(&a, 0) & want, want);
    }

    #[test]
    fn combat_kills_shots_and_revives() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.killed_animal(0, sim_core::mob::MOB_PIG);
        assert_eq!(got(&a, 0) & bit("WOLF"), 0);
        a.killed_animal(0, MOB_WOLF);
        assert_eq!(
            got(&a, 0) & (bit("WOLF") | bit("KILLS_2")),
            bit("WOLF") | bit("KILLS_2")
        );
        a.killed_player(0, 1_200);
        assert_eq!(got(&a, 0) & (bit("PVP") | bit("FAR")), bit("PVP"));
        a.killed_player(0, 6_000);
        assert_eq!(got(&a, 0) & bit("FAR"), bit("FAR"));
        let head = sim_core::world::hit_c(Part::Head, 30);
        a.event(0, ev(EV_HIT, 1, STRUCT_VICTIM, head));
        a.event(0, ev(EV_HIT, 1, MOB_ID_TAG | 4, head));
        assert_eq!(
            got(&a, 0) & bit("HEAD"),
            0,
            "a wall or an animal is no headshot"
        );
        a.event(0, ev(EV_HIT, 1, 2, sim_core::world::hit_c(Part::Chest, 30)));
        assert_eq!(got(&a, 0) & bit("HEAD"), 0);
        a.event(0, ev(EV_HIT, 1, 2, head));
        assert_eq!(got(&a, 0) & bit("HEAD"), bit("HEAD"));
        a.event(0, ev(EV_ASSIST, 1, 2, 30));
        assert_eq!(got(&a, 0) & bit("REVIVE"), 0, "half a revive");
        a.event(
            0,
            ev(EV_ASSIST, 1, 2, u32::from(sim_core::assist::ASSIST_TICKS)),
        );
        assert_eq!(got(&a, 0) & bit("REVIVE"), bit("REVIVE"));
    }

    #[test]
    fn talking_to_every_speaker_and_learning_every_glyph() {
        let mut a = ach();
        a.join(0, Some(W1));
        let talk = |t: u32| ev(EV_ARC_DID, 1, u32::from(sim_core::lore::OP_TALK), t << 8);
        a.event(0, talk(0));
        a.event(0, talk(0));
        a.event(0, talk(2));
        assert_eq!(got(&a, 0) & bit("TALK"), 0, "two of three, however often");
        a.event(0, talk(1));
        assert_eq!(got(&a, 0) & bit("TALK"), bit("TALK"));
        a.swept(0, &[], &[], 0b0111 | 1 << 40);
        assert_eq!(
            got(&a, 0) & bit("GLYPHS"),
            0,
            "a glyph no stone teaches does not count"
        );
        a.swept(0, &[], &[], 0b1111);
        assert_eq!(got(&a, 0) & bit("GLYPHS"), bit("GLYPHS"));
    }

    #[test]
    fn archived_and_unresolved_rows_are_never_awarded_and_never_block_a_set() {
        let b = baked();
        assert_eq!(b.unresolved, vec!["GONE: nope".to_string()]);
        let mut a = ach();
        a.join(1, Some(W1));
        a.event(1, ev(EV_CRAFT_DONE, 1, pack(5, 1), 0));
        assert_eq!(got(&a, 1) & bit("OLD"), 0, "archived");
        let rec = Rec {
            got: ["BAG", "JUNK", "FED", "GONE"]
                .iter()
                .map(|s| (s.to_string(), 1))
                .collect(),
            ..Rec::default()
        };
        assert!(
            set_done(&rec, &T_SETS[1], T_DEFS),
            "OLD is archived and does not count"
        );
    }

    #[test]
    fn a_full_bar_counts_only_after_eating_up_to_it() {
        let mut a = ach();
        a.join(2, Some(W1));
        let full = (100 << 16) | 100;
        // The join's own announcement: full, nothing eaten.
        a.event(2, ev(EV_VITALS, 1, full, full));
        a.event(2, ev(EV_VITALS, 1, (60 << 16) | 100, full));
        // Dying hungry and waking full is not a meal.
        a.event(2, ev(EV_RESPAWN, 1, 0, 0));
        a.event(2, ev(EV_VITALS, 1, full, full));
        assert_eq!(got(&a, 2), 0);
        a.event(2, ev(EV_VITALS, 1, (60 << 16) | 100, full));
        a.event(2, ev(EV_CONSUMED, 1, 0, 0));
        a.event(2, ev(EV_VITALS, 1, (80 << 16) | 100, full));
        assert_eq!(got(&a, 2), 0);
        a.event(2, ev(EV_VITALS, 1, full, full));
        assert_eq!(got(&a, 2), bit("FED"));
    }

    #[test]
    fn guests_and_unknown_slots_are_nobody() {
        let mut a = ach();
        a.join(4, None);
        a.join(5, Some("guest"));
        assert!(!a.tracking());
        a.event(4, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        a.event(9, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        a.built(9, 0);
        a.killed_animal(9, 0);
        assert!(a.rows.is_empty());
        assert_eq!(
            a.lines(4),
            vec!["Achievements need a wallet sign-in".to_string()]
        );
        a.join(4, Some(W1));
        assert!(a.tracking());
        a.leave(4);
        assert!(!a.tracking());
    }

    #[test]
    fn a_wallet_keeps_its_row_across_connections() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.event(0, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        a.leave(0);
        a.join(7, Some(W2));
        a.join(9, Some(W1));
        assert_eq!(got(&a, 9), bit("AXE"));
        assert_eq!(got(&a, 7), 0);
    }

    #[test]
    fn ach_says_each_set_and_the_nearest_counting_row() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.event(0, ev(EV_GATHER, 1, pack(0, 14), 0));
        a.built(0, 1);
        let lines = a.lines(0);
        assert_eq!(lines[0], "Alpha: 1/3");
        assert_eq!(lines[1], "Beta: 0/3");
        assert_eq!(lines.last().unwrap(), "Next: More Wood 14/20");
        assert!(
            lines.iter().all(|l| l.len() + 9 <= protocol::ChatText::CAP),
            "{lines:?}"
        );
    }

    fn writer() -> Writer {
        let b = baked();
        Writer {
            defs: T_DEFS,
            sets: T_SETS,
            counters: b.counter_meta(),
            stats: b.stats(),
            discord: None,
        }
    }

    #[test]
    fn the_writer_stamps_once_and_a_finished_set_stays_finished() {
        let w = writer();
        let mut mirror = BTreeMap::new();
        let mut row = Row {
            got: bit("WOOD_10") | bit("WOOD_20") | bit("AXE"),
            ..Row::default()
        };
        row.stats[0] = 25;
        let batch = Batch {
            rows: vec![(W1.into(), "Ash".into(), row)],
        };
        let first = w.apply(&mut mirror, &batch, 100);
        assert_eq!(
            first,
            vec![("Ash".to_string(), "Alpha")],
            "who to post, once"
        );
        assert!(w.apply(&mut mirror, &batch, 200).is_empty());
        let rec = &mirror[W1];
        assert_eq!(rec.got["WOOD_10"], 100, "a stamp never moves");
        assert_eq!(rec.sets["a"], 100);
        assert_eq!(rec.stats["gather:wood"], 25);
        // A row added to a finished set later does not unfinish it.
        const MORE: &[Def] = &[
            T_DEFS[0],
            T_DEFS[1],
            T_DEFS[2],
            def("a", "NEW", "New", "", "", Rule::Research),
        ];
        let w2 = Writer {
            defs: MORE,
            ..writer()
        };
        w2.apply(&mut mirror, &batch, 300);
        assert!(!set_done(&mirror[W1], &T_SETS[0], MORE));
        assert_eq!(mirror[W1].sets["a"], 100, "finished stays finished");
    }

    #[test]
    fn a_speaker_mask_is_kept_whole_and_published_as_a_count() {
        let w = writer();
        let talk = w
            .counters
            .iter()
            .position(|(n, _)| n == "talk:speakers")
            .unwrap();
        let mut mirror = BTreeMap::new();
        let mut row = Row::default();
        row.stats[talk] = 0b101;
        w.apply(
            &mut mirror,
            &Batch {
                rows: vec![(W1.into(), "".into(), row)],
            },
            1,
        );
        row.stats[talk] = 0b011;
        w.apply(
            &mut mirror,
            &Batch {
                rows: vec![(W1.into(), "".into(), row)],
            },
            2,
        );
        assert_eq!(mirror[W1].stats["talk:speakers"], 0b111, "OR-ed, not maxed");
        let v: serde_json::Value = serde_json::from_str(&w.render_json(&mirror, 3)).unwrap();
        assert_eq!(v["wallets"][W1]["stats"]["talk:speakers"], 3);
    }

    #[test]
    fn the_record_reads_back_what_it_wrote() {
        let mut mirror = BTreeMap::new();
        let rec = Rec {
            got: [("WOOD_10".to_string(), 100), ("RETIRED".to_string(), 5)].into(),
            stats: [
                ("gather:item.wood".to_string(), 412),
                ("wear".to_string(), 3),
            ]
            .into(),
            sets: [("a".to_string(), 100)].into(),
        };
        mirror.insert(W1.to_string(), rec);
        mirror.insert(W2.to_string(), Rec::default());
        let text = render_record(&mirror);
        assert_eq!(parse_record(&text), mirror);
        let bad = format!("{text}0xdead a:X@notanumber\n");
        assert_eq!(parse_record(&bad), mirror, "a bad line is skipped whole");

        // And the sim side seeds from it.
        let mut a = ach();
        a.seed(&parse_record(&text));
        a.join(0, Some(W1));
        assert_eq!(got(&a, 0), bit("WOOD_10"));
    }

    #[test]
    fn the_json_parses_and_counts_who_has_what() {
        let w = writer();
        let mut mirror = BTreeMap::new();
        let mut row = Row {
            got: bit("WOOD_10") | bit("WOOD_20") | bit("AXE"),
            ..Row::default()
        };
        row.stats[0] = 25;
        let batch = Batch {
            rows: vec![
                (W1.into(), "".into(), row),
                (W2.into(), "".into(), Row::default()),
            ],
        };
        w.apply(&mut mirror, &batch, 9);
        let v: serde_json::Value = serde_json::from_str(&w.render_json(&mirror, 9)).expect("json");
        assert_eq!(v["players"], 2);
        assert_eq!(v["achievements"][0]["earned"], 1);
        assert_eq!(v["achievements"][0]["icon"], "wood");
        assert_eq!(v["achievements"][2]["icon"], serde_json::Value::Null);
        assert_eq!(v["achievements"][0]["progress"]["stat"], "gather:wood");
        assert_eq!(v["achievements"][0]["progress"]["max"], 10);
        assert_eq!(
            v["achievements"][idx("TALK") as usize]["progress"]["max"],
            3
        );
        assert_eq!(v["achievements"][6]["spoiler"], true);
        assert_eq!(v["sets"][0]["earned"], 1);
        assert_eq!(v["sets"][0]["icon"], "ui_star");
        assert_eq!(v["wallets"][W1]["got"]["AXE"], 9);
        assert!(
            v["wallets"].get(W2).is_none(),
            "nothing to show is left out"
        );
        assert_eq!(v["truncated"], false);
    }

    #[test]
    fn a_full_ring_keeps_the_rows_for_the_next_flush() {
        let (tx, mut rx) = rtrb::RingBuffer::<Batch>::new(1);
        let mut a = ach();
        a.out = Some(tx);
        a.join(0, Some(W1));
        a.flush(|_| Some("Ash".into()));
        a.event(0, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        a.flush(|_| None);
        assert_eq!(a.moved.len(), 1, "the ring was full: kept");
        let first = rx.pop().unwrap();
        assert_eq!(first.rows[0].1, "Ash");
        assert_eq!(first.rows[0].2.got, 0);
        a.flush(|_| None);
        let second = rx.pop().unwrap();
        assert_eq!(second.rows[0].2.got, bit("AXE"));
        assert_eq!(
            second.rows[0].1,
            crate::standings::short_label(W1),
            "no name: the short wallet"
        );
    }

    /// The shipped list against the shipped content: every row resolves,
    /// every row is in a live set, ids are unique, every name fits the popup
    /// and every icon is a file the client and the site carry.
    #[test]
    fn the_shipped_list_resolves_against_the_shipped_content() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        let c = content::Content::load_dir(&dir).expect("shipped content loads");
        let deploy = c.bake_deployables().expect("deployables bake");
        let b = Baked::bake_from(&c, &deploy);
        assert!(
            b.unresolved.is_empty(),
            "rows naming what the content lacks: {:?}",
            b.unresolved
        );
        let live: u128 = b.sets.iter().fold(0, |m, s| m | s);
        assert_eq!(
            live.count_ones() as usize,
            ACHIEVEMENTS.len(),
            "every row is in a live set"
        );
        let mut ids: Vec<_> = ACHIEVEMENTS.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ACHIEVEMENTS.len(), "ids are unique");
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
        for d in ACHIEVEMENTS {
            assert!(
                SETS.iter().any(|s| s.id == d.set),
                "{}: no set {}",
                d.id,
                d.set
            );
            for t in [d.name, d.desc] {
                assert!(
                    protocol::ChatText::sanitize(t.as_bytes()).is_some(),
                    "{}: {t:?} is not popup-safe",
                    d.id
                );
            }
            assert!(
                icons.join(format!("{}.png", d.icon)).is_file(),
                "{}: no icon {}",
                d.id,
                d.icon
            );
            if let Rule::Craft(items) = d.rule {
                for id in items {
                    assert!(
                        c.recipes.iter().any(|r| r.output == *id),
                        "{}: nothing crafts {id}",
                        d.id
                    );
                }
            }
        }
        for s in SETS {
            assert!(
                icons.join(format!("{}.png", s.icon)).is_file(),
                "set {}: no icon",
                s.id
            );
            assert!(
                format!("{}: 99/99 - done", s.name).len() + 9 <= protocol::ChatText::CAP,
                "{}",
                s.id
            );
        }
    }

    /// The tick's cost, measured: a full shard of wallets, a full event
    /// ring of tracked events every tick. `cargo test -p server --release
    /// --lib -- --ignored --nocapture achievements::tests::cost`.
    #[test]
    #[ignore]
    fn cost() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        let c = content::Content::load_dir(&dir).expect("shipped content loads");
        let deploy = c.bake_deployables().expect("deployables bake");
        let wood = c.item_index("item.wood").unwrap() as u32;
        let axe = c.item_index("item.hatchet_stone").unwrap() as u32;
        let mut a = Achievements::new(ACHIEVEMENTS, SETS, Baked::bake_from(&c, &deploy));
        for s in 0..MAX_PLAYERS {
            a.join(s, Some(&format!("0x{s:040x}")));
        }
        let head = sim_core::world::hit_c(Part::Head, 30);
        let evs: Vec<(usize, SimEvent)> = (0..sim_core::limits::MAX_EVENTS_PER_TICK)
            .map(|i| {
                let code = [
                    EV_GATHER,
                    EV_CRAFT_DONE,
                    EV_VITALS,
                    EV_DEPLOY_PLACED,
                    EV_HIT,
                ][i % 5];
                let (b, c) = match code {
                    EV_GATHER => (pack(wood, 3), 0),
                    EV_CRAFT_DONE => (pack(axe, 1), 0),
                    EV_VITALS => ((50 << 16) | 50, (100 << 16) | 100),
                    EV_HIT => (7, head),
                    _ => (0, 0),
                };
                (i % MAX_PLAYERS, ev(code, 0, b, c))
            })
            .collect();
        let inv = [stack(0, 5); sim_core::limits::INV_SLOTS];
        let ticks = 30 * 60 * 10;
        let t = std::time::Instant::now();
        for tick in 0..ticks {
            for &(s, e) in &evs {
                a.event(s, e);
            }
            a.built(tick % MAX_PLAYERS, 1);
            let s = a.sweep_next();
            a.swept(s, &inv, &inv[..sim_core::limits::WEAR_SLOTS], u64::MAX);
            a.take_tells(|_, t| {
                std::hint::black_box(t);
            });
            if tick % (30 * FLUSH_SECS as usize) == 0 {
                a.flush(|_| None);
            }
        }
        let per = t.elapsed() / ticks as u32;
        println!("achievements: {per:?} a tick at {} events", evs.len());
    }
}
