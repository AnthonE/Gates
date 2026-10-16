//! **Achievements**, in Steam's shape (Achievement Sets, 2026-10-15): the
//! game's achievements grouped in sets, each set with its own progress and a
//! trophy when it is finished, progress stats behind the counting ones,
//! spoilers, and achievements that retire without being taken from anyone.
//!
//! The shard is the one that saw it happen, so the shard keeps them
//! (Steamworks' "set by official game server"). Elo Pros reads
//! `/achievements.json` off the status thread and draws the page.
//!
//! ## Where this runs, and what it costs
//!
//! - **The tick.** `ShardCore::note_achievements` walks the tick's events
//!   once, after the standings. A tracked event finds its connection's row
//!   by slot and sets a bit or bumps a counter in place: no string, no map
//!   and no allocation. One body a tick is swept for what it holds and
//!   wears, so a full shard is swept every [`MAX_PLAYERS`] ticks.
//! - **Every [`FLUSH_SECS`] s** the rows that moved are copied (72 bytes
//!   each) into a ring for the writer thread. Nothing is formatted on the
//!   sim thread.
//! - **The writer thread** owns the clock (an unlock is stamped when it
//!   lands, so to within a flush), keeps every wallet the shard has seen,
//!   and writes `<base>.achievements` (the record, read back at boot) and
//!   `<base>.achievements.json`, which the status thread serves.
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
//! when rows are added to it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sim_core::gather::ItemStack;
use sim_core::limits::MAX_PLAYERS;
use sim_core::world::{
    SimEvent, EV_CONSUMED, EV_CRAFT_DONE, EV_DEPLOY_PLACED, EV_DRANK, EV_GATHER, EV_RESEARCH,
    EV_RESPAWN, EV_VEND, EV_VITALS,
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
}

impl Rule {
    /// The progress stat behind a counting row, and its target:
    /// Steamworks' "progress stat" with its range.
    pub fn progress(&self) -> Option<(String, u32)> {
        match *self {
            Rule::Gather(item, n) => Some((format!("gather:{item}"), n)),
            Rule::Hold(item, n) => Some((format!("hold:{item}"), n)),
            Rule::Wear(n) => Some(("wear".into(), n)),
            _ => None,
        }
    }
}

/// A set: Steam's Achievement Set.
#[derive(Clone, Copy, Debug)]
pub struct Set {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
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
    pub rule: Rule,
    /// Steam's "spoiler": the name and text stay hidden until earned.
    pub spoiler: bool,
    pub archived: bool,
}

const fn survival(id: &'static str, name: &'static str, desc: &'static str, rule: Rule) -> Def {
    Def {
        id,
        set: "survival",
        name,
        desc,
        rule,
        spoiler: false,
        archived: false,
    }
}

const fn spoiler(d: Def) -> Def {
    Def { spoiler: true, ..d }
}

pub const SETS: &[Set] = &[Set {
    id: "survival",
    name: "Survival",
    about: "The first hours on the island: gather, craft, and settle in.",
    archived: false,
}];

/// The list. Mirrors Rust's own first-hours achievements where Gates has the
/// same thing (a Tool Cupboard is a Hearth here, Scrap is Junk).
pub const ACHIEVEMENTS: &[Def] = &[
    survival(
        "COLLECT_100_WOOD",
        "Collect Wood",
        "Collect 100 Wood",
        Rule::Gather("item.wood", 100),
    ),
    survival(
        "COLLECT_700_WOOD",
        "Collect 700 Wood",
        "Collect 700 Wood",
        Rule::Gather("item.wood", 700),
    ),
    survival(
        "COLLECT_200_STONE",
        "Collect 200 Stone",
        "Collect 200 Stone",
        Rule::Gather("item.stone", 200),
    ),
    survival(
        "COLLECT_300_METAL_ORE",
        "Collect 300 Metal Ore",
        "Collect 300 Metal Ore",
        Rule::Gather("item.metal_ore", 300),
    ),
    survival(
        "COLLECT_30_CLOTH",
        "Collect 30 Cloth",
        "Collect 30 Cloth",
        Rule::Gather("item.cloth", 30),
    ),
    survival(
        "COLLECT_50_CLOTH",
        "Collect 50 Cloth",
        "Collect 50 Cloth",
        Rule::Gather("item.cloth", 50),
    ),
    survival(
        "HOLD_65_JUNK",
        "Collect 65 Junk",
        "Hold 65 Junk at once",
        Rule::Hold("item.junk", 65),
    ),
    survival(
        "HOLD_50_LGF",
        "Acquire 50 Low Grade Fuel",
        "Hold 50 Low Grade Fuel at once",
        Rule::Hold("item.lowgrade", 50),
    ),
    survival(
        "CRAFT_STONE_HATCHET",
        "Craft Stone Hatchet",
        "Craft a Stone Hatchet",
        Rule::Craft(&["item.hatchet_stone"]),
    ),
    survival(
        "CRAFT_STONE_PICKAXE",
        "Craft Stone Pickaxe",
        "Craft a Stone Pickaxe",
        Rule::Craft(&["item.pickaxe_stone"]),
    ),
    survival(
        "CRAFT_SPEAR",
        "Craft Spear",
        "Craft a Wooden Spear",
        Rule::Craft(&["item.spear_wood"]),
    ),
    survival(
        "CRAFT_BUILDING_PLAN",
        "Craft Building Plan",
        "Craft a Building Plan",
        Rule::Craft(&["item.building_plan"]),
    ),
    survival(
        "CRAFT_HAMMER",
        "Craft Hammer",
        "Craft a Hammer",
        Rule::Craft(&["item.hammer"]),
    ),
    survival(
        "CRAFT_CAMPFIRE",
        "Craft Camp Fire",
        "Craft a Camp Fire",
        Rule::Craft(&["item.fire_pit"]),
    ),
    survival(
        "CRAFT_SLEEPINGBAG",
        "Craft Sleeping Bag",
        "Craft a Sleeping Bag",
        Rule::Craft(&["item.sleeping_bag"]),
    ),
    survival(
        "CRAFT_LOCK",
        "Craft Lock",
        "Craft a lock",
        Rule::Craft(&["item.lock_key", "item.lock_code"]),
    ),
    survival(
        "CRAFT_WOODEN_DOOR",
        "Craft Wooden Door",
        "Craft a Wooden Door",
        Rule::Craft(&["item.door_wood"]),
    ),
    survival(
        "CRAFT_HEARTH",
        "Craft Hearth",
        "Craft a Hearth",
        Rule::Craft(&["item.hearth"]),
    ),
    survival(
        "CRAFT_ARROWS",
        "Craft Arrows",
        "Craft some Arrows",
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
        Rule::Craft(&["item.bow"]),
    ),
    survival(
        "CRAFT_BURLAP_TUNIC",
        "Craft Burlap Tunic",
        "Craft a Burlap Tunic",
        Rule::Craft(&["item.armor_burlap_body"]),
    ),
    survival(
        "CRAFT_BURLAP_HOOD",
        "Craft Burlap Hood",
        "Craft a Burlap Hood",
        Rule::Craft(&["item.armor_burlap_head"]),
    ),
    survival(
        "CRAFT_WOODEN_BOX",
        "Craft Wooden Box",
        "Craft a Small Box",
        Rule::Craft(&["item.box_small"]),
    ),
    survival(
        "CRAFT_FURNACE",
        "Craft a Furnace",
        "Craft a Furnace",
        Rule::Craft(&["item.furnace"]),
    ),
    survival(
        "CRAFT_WORKBENCH",
        "Craft a Workbench",
        "Craft a Workbench",
        Rule::Craft(&["item.workbench1"]),
    ),
    survival(
        "CRAFT_RESEARCH_TABLE",
        "Craft a Research Table",
        "Craft a Research Table",
        Rule::Craft(&["item.research_table"]),
    ),
    survival(
        "PLACE_CAMPFIRE",
        "Place Camp Fire",
        "Place a Camp Fire",
        Rule::Place(&["item.fire_pit"]),
    ),
    survival(
        "PLACE_SLEEPINGBAG",
        "Place Sleeping Bag",
        "Place a Sleeping Bag",
        Rule::Place(&["item.sleeping_bag"]),
    ),
    survival(
        "PLACE_WOODEN_DOOR",
        "Place Wooden Door",
        "Place a Wooden Door",
        Rule::Place(&["item.door_wood"]),
    ),
    survival(
        "PLACE_LOCK",
        "Place Lock",
        "Place a lock",
        Rule::Place(&["item.lock_key", "item.lock_code"]),
    ),
    survival(
        "PLACE_HEARTH",
        "Place Hearth",
        "Place a Hearth",
        Rule::Place(&["item.hearth"]),
    ),
    survival(
        "PLACE_WOODEN_BOX",
        "Place Wooden Box",
        "Place a box",
        Rule::Place(&["item.box_small", "item.box_large"]),
    ),
    survival(
        "PLACE_FURNACE",
        "Place a Furnace",
        "Place a Furnace",
        Rule::Place(&["item.furnace"]),
    ),
    survival(
        "PLACE_WORKBENCH",
        "Place a Workbench",
        "Place a Workbench",
        Rule::Place(&["item.workbench1", "item.workbench2", "item.workbench3"]),
    ),
    survival(
        "PLACE_RESEARCH_TABLE",
        "Place a Research Table",
        "Place a Research Table",
        Rule::Place(&["item.research_table"]),
    ),
    survival(
        "RESEARCH_ITEM",
        "Research an Item",
        "Learn a recipe",
        Rule::Research,
    ),
    survival(
        "KIOSK_PURCHASE",
        "First Purchase",
        "Buy something at a kiosk in THE GATE",
        Rule::Vend,
    ),
    survival(
        "EQUIP_CLOTHING",
        "Equip Clothing",
        "Wear clothing in both slots",
        Rule::Wear(2),
    ),
    spoiler(survival(
        "GLUTTON",
        "Glutton",
        "Completely fill your food bar",
        Rule::FullFood,
    )),
    spoiler(survival(
        "SEALBREAKER",
        "Sealbreaker",
        "Completely fill your hydration bar",
        Rule::FullWater,
    )),
];

/// Rows are bits of a `u64`. Past this the row type widens.
pub const MAX_DEFS: usize = 64;
const _: () = assert!(ACHIEVEMENTS.len() <= MAX_DEFS);
/// Progress stats a row can carry.
pub const MAX_COUNTERS: usize = 16;
/// Wallets kept in memory: every one the shard has seen, about 150 bytes
/// each. Past this a new wallet is not tracked and `refused` counts it.
pub const MAX_WALLETS: usize = 1 << 16;
/// How often the rows that moved go to the writer.
pub const FLUSH_SECS: u64 = 30;
/// Unlock lines owed to chat, at most, between pumps.
pub const TELL_CAP: usize = 64;
/// Batches queued for the writer. One is written in well under a flush, so
/// a backlog means the disk is stuck; a full ring keeps the rows marked
/// moved and the next flush sends them again.
const WRITE_RING: usize = 4;
/// The most of the public file the status thread serves, and the writer's
/// bound on it.
pub const JSON_CAP: u64 = 8 * 1024 * 1024;

const NONE_U8: u8 = u8::MAX;
const NONE_U32: u32 = u32::MAX;
/// A tell for a finished set rather than a row.
const TELL_SET: u8 = 0x80;

pub fn record_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.achievements", base.display()))
}

pub fn json_path(base: &Path) -> PathBuf {
    PathBuf::from(format!("{}.achievements.json", base.display()))
}

// ---- the bake ----------------------------------------------------------------

/// One progress stat.
#[derive(Clone, Debug)]
struct Counter {
    /// `gather:item.wood`, `hold:item.junk`, `wear`.
    name: String,
    kind: CounterKind,
    /// `(target, row)`, for every row this counter earns.
    steps: Vec<(u32, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CounterKind {
    /// Summed from `EV_GATHER`.
    Gather,
    /// The most of an item held at once.
    Hold(u16),
    /// The most pieces worn at once.
    Wear,
}

/// The list as tables the tick indexes by number.
#[derive(Clone, Debug, Default)]
pub struct Baked {
    /// By item index: the gather counter it feeds, or `NONE_U8`.
    gather: Vec<u8>,
    /// By item index: the rows crafting it earns.
    craft: Vec<u64>,
    /// By deploy row: the rows placing it earns.
    place: Vec<u64>,
    counters: Vec<Counter>,
    research: u64,
    vend: u64,
    full_food: u64,
    full_water: u64,
    /// By set: its live rows.
    sets: Vec<u64>,
    /// Rows that name something the content lacks, as `id: item`. Never
    /// earnable; the boot log names them.
    pub unresolved: Vec<String>,
}

impl Baked {
    /// `item` resolves an item id to its index, `deploy_items` is each
    /// deploy row's item, and `n_items` bounds the item tables.
    pub fn bake(
        defs: &[Def],
        sets: &[Set],
        n_items: usize,
        item: impl Fn(&str) -> Option<u16>,
        deploy_items: &[u16],
    ) -> Baked {
        let mut b = Baked {
            gather: vec![NONE_U8; n_items],
            craft: vec![0; n_items],
            place: vec![0; deploy_items.len()],
            sets: vec![0; sets.len()],
            ..Baked::default()
        };
        for (i, d) in defs.iter().enumerate().take(MAX_DEFS) {
            let set = sets.iter().position(|s| s.id == d.set);
            if d.archived || set.is_some_and(|s| sets[s].archived) {
                continue;
            }
            let bit = 1u64 << i;
            let mut ok = true;
            let mut resolve = |id: &str| -> Option<usize> {
                let got = item(id).map(usize::from).filter(|&n| n < n_items);
                if got.is_none() {
                    b.unresolved.push(format!("{}: {id}", d.id));
                }
                got
            };
            match d.rule {
                Rule::Gather(id, n) => match resolve(id) {
                    Some(it) => {
                        let c =
                            counter(&mut b.counters, format!("gather:{id}"), CounterKind::Gather);
                        if let Some(c) = c {
                            b.gather[it] = c;
                            b.counters[c as usize].steps.push((n, i as u8));
                        } else {
                            ok = false;
                        }
                    }
                    None => ok = false,
                },
                Rule::Hold(id, n) => match resolve(id) {
                    Some(it) => {
                        let kind = CounterKind::Hold(it as u16);
                        match counter(&mut b.counters, format!("hold:{id}"), kind) {
                            Some(c) => b.counters[c as usize].steps.push((n, i as u8)),
                            None => ok = false,
                        }
                    }
                    None => ok = false,
                },
                Rule::Wear(n) => match counter(&mut b.counters, "wear".into(), CounterKind::Wear) {
                    Some(c) => b.counters[c as usize].steps.push((n, i as u8)),
                    None => ok = false,
                },
                Rule::Craft(ids) => {
                    for id in ids {
                        match resolve(id) {
                            Some(it) => b.craft[it] |= bit,
                            None => ok = false,
                        }
                    }
                }
                Rule::Place(ids) => {
                    for id in ids {
                        match resolve(id) {
                            Some(it) => {
                                for (row, &di) in deploy_items.iter().enumerate() {
                                    if usize::from(di) == it {
                                        b.place[row] |= bit;
                                    }
                                }
                            }
                            None => ok = false,
                        }
                    }
                }
                Rule::Research => b.research |= bit,
                Rule::Vend => b.vend |= bit,
                Rule::FullFood => b.full_food |= bit,
                Rule::FullWater => b.full_water |= bit,
            }
            if let (true, Some(s)) = (ok, set) {
                b.sets[s] |= bit;
            }
        }
        b
    }

    /// The shipped list against the shipped content.
    pub fn bake_from(c: &content::Content, deploy: &sim_core::deploy::DeployContent) -> Baked {
        let rows = &deploy.defs[..usize::from(deploy.def_count)];
        let items: Vec<u16> = rows.iter().map(|d| d.item).collect();
        Baked::bake(
            ACHIEVEMENTS,
            SETS,
            c.items.len(),
            |id| c.item_index(id),
            &items,
        )
    }

    /// The stat names, by counter index: what the writer files them under.
    pub fn counter_names(&self) -> Vec<String> {
        self.counters.iter().map(|c| c.name.clone()).collect()
    }
}

fn counter(cs: &mut Vec<Counter>, name: String, kind: CounterKind) -> Option<u8> {
    if let Some(i) = cs.iter().position(|c| c.name == name) {
        return Some(i as u8);
    }
    if cs.len() == MAX_COUNTERS {
        return None;
    }
    cs.push(Counter {
        name,
        kind,
        steps: Vec::new(),
    });
    Some((cs.len() - 1) as u8)
}

// ---- the sim side ------------------------------------------------------------

/// One wallet's achievements, as the tick keeps them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Bit `i` is `defs[i]` earned.
    pub got: u64,
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
/// harmless.
pub struct Batch {
    rows: Vec<(String, Row)>,
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
    /// Unlock lines dropped because the tell queue was full.
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
        let names = a.baked.counter_names();
        let (defs, sets) = (a.defs, a.sets);
        let (rec_path, json) = (record_path(base), json_path(base));
        let spawned = std::thread::Builder::new()
            .name("achievements".into())
            .spawn(move || {
                let mut mirror = record;
                loop {
                    match rx.pop() {
                        Ok(batch) => {
                            let now = unix_now();
                            apply(&mut mirror, &batch, now, defs, sets, &names);
                            if let Err(e) = write_atomic(&rec_path, &render_record(&mirror)) {
                                eprintln!("achievements {}: {e}", rec_path.display());
                            }
                            let text = render_json(&mirror, now, defs, sets);
                            if let Err(e) = write_atomic(&json, &text) {
                                eprintln!("achievements {}: {e}", json.display());
                            }
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
        let names = self.baked.counter_names();
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
            for (c, name) in names.iter().enumerate() {
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
    fn earn(&mut self, slot: usize, bits: u64) {
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

    fn bump(&mut self, slot: usize, c: u8, units: u32, max: bool) {
        let i = self.slot[slot];
        let Some(row) = self.rows.get_mut(i as usize) else {
            return;
        };
        let s = &mut row.stats[c as usize];
        let was = *s;
        *s = if max {
            was.max(units)
        } else {
            was.saturating_add(units)
        };
        if *s == was {
            return;
        }
        let now = *s;
        self.mark(i);
        let mut earned = 0u64;
        for &(n, bit) in &self.baked.counters[c as usize].steps {
            if now >= n {
                earned |= 1 << bit;
            }
        }
        if earned != 0 {
            self.earn(slot, earned);
        }
    }

    /// One of the tick's events, already resolved to its player's
    /// connection. Events this does not track cost a match arm.
    pub fn event(&mut self, slot: usize, ev: SimEvent) {
        if !self.tracks(slot) {
            return;
        }
        match ev.code {
            EV_GATHER => {
                let (item, units) = ((ev.b >> 16) as usize, ev.b & 0xffff);
                let c = self.baked.gather.get(item).copied().unwrap_or(NONE_U8);
                if c != NONE_U8 && units > 0 {
                    self.bump(slot, c, units, false);
                }
            }
            EV_CRAFT_DONE => {
                let bits = self.baked.craft.get((ev.b >> 16) as usize).copied();
                if let Some(bits @ 1..) = bits {
                    self.earn(slot, bits);
                }
            }
            EV_DEPLOY_PLACED => self.placed(slot, (ev.b & 0xff) as u16),
            EV_RESEARCH if self.baked.research != 0 => self.earn(slot, self.baked.research),
            EV_VEND if self.baked.vend != 0 => self.earn(slot, self.baked.vend),
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
                    bits |= self.baked.full_food;
                }
                if max_water > 0 && water >= max_water && b.water_low && b.drank {
                    bits |= self.baked.full_water;
                }
                let b = &mut self.bars[slot];
                b.food_low = food < max_food;
                b.water_low = water < max_water;
                if bits != 0 {
                    self.earn(slot, bits);
                }
            }
            _ => {}
        }
    }

    /// `slot` placed deploy row `row`.
    pub fn placed(&mut self, slot: usize, row: u16) {
        if let Some(bits @ 1..) = self.baked.place.get(usize::from(row)).copied() {
            self.earn(slot, bits);
        }
    }

    /// Which connection slot the pack sweep looks at this tick.
    pub fn sweep_next(&mut self) -> usize {
        let s = self.sweep;
        self.sweep = (self.sweep + 1) % MAX_PLAYERS;
        s
    }

    /// The pack sweep's look at one body: what it holds and wears.
    pub fn swept(&mut self, slot: usize, inv: &[ItemStack], worn: &[ItemStack]) {
        if !self.tracks(slot) {
            return;
        }
        for c in 0..self.baked.counters.len() {
            let n = match self.baked.counters[c].kind {
                CounterKind::Gather => continue,
                CounterKind::Hold(item) => inv
                    .iter()
                    .filter(|s| s.count > 0 && s.item == item)
                    .map(|s| u32::from(s.count))
                    .sum(),
                CounterKind::Wear => worn.iter().filter(|s| s.count > 0).count() as u32,
            };
            if n > 0 {
                self.bump(slot, c as u8, n, true);
            }
        }
    }

    /// The unlock lines owed since the last pump, as `(slot, line)`.
    /// Formatting happens here, at most [`TELL_CAP`] lines a tick, and only
    /// when something was earned.
    pub fn take_tells(&mut self, mut f: impl FnMut(usize, String)) {
        let n = std::mem::take(&mut self.tells_n);
        for &(slot, what) in &self.tells[..n] {
            let slot = usize::from(slot);
            if what & TELL_SET != 0 {
                let s = usize::from(what & !TELL_SET);
                if let Some(set) = self.sets.get(s) {
                    f(slot, format!("Set complete: {}", set.name));
                }
            } else if let Some(d) = self.defs.get(usize::from(what)) {
                f(slot, format!("Achievement: {}", d.name));
            }
        }
    }

    /// Hand the rows that moved to the writer. Called every
    /// [`FLUSH_SECS`]; allocates one batch, sized by what moved.
    pub fn flush(&mut self) {
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
            .map(|&i| (self.keys[i as usize].clone(), self.rows[i as usize]))
            .collect();
        if out.push(Batch { rows }).is_ok() {
            for &i in &self.moved {
                self.moved_flag[i as usize] = false;
            }
            self.moved.clear();
        }
    }

    /// The shutdown's last word: flush, and wait for the writer to land it.
    pub fn close(&mut self) {
        // A full ring is the writer mid-write: give it up to two seconds to
        // make room, so the last flush is not the one that gets skipped.
        for _ in 0..40 {
            self.flush();
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
    /// Stat name → value.
    pub stats: BTreeMap<String, u64>,
    /// Set id → unix seconds finished.
    pub sets: BTreeMap<String, u64>,
}

/// Fold a batch into the mirror at `now`: new rows are stamped, stats
/// replaced, and a set stamped the first time its live rows are all held.
/// A stamp is never moved or removed.
pub fn apply(
    mirror: &mut BTreeMap<String, Rec>,
    batch: &Batch,
    now: u64,
    defs: &[Def],
    sets: &[Set],
    names: &[String],
) {
    for (key, row) in &batch.rows {
        let rec = mirror.entry(key.clone()).or_default();
        for (bit, d) in defs.iter().enumerate().take(MAX_DEFS) {
            if row.got & (1 << bit) != 0 {
                rec.got.entry(d.id.to_string()).or_insert(now);
            }
        }
        for (c, name) in names.iter().enumerate() {
            let v = u64::from(row.stats[c]);
            if v > 0 {
                let s = rec.stats.entry(name.clone()).or_insert(0);
                *s = (*s).max(v);
            }
        }
        for set in sets {
            if !rec.sets.contains_key(set.id) && set_done(rec, set, defs) {
                rec.sets.insert(set.id.to_string(), now);
            }
        }
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

/// `/achievements.json`: the list, how many have each, and every wallet's
/// own. Wallets past [`JSON_CAP`] are left out and `truncated` says so.
pub fn render_json(mirror: &BTreeMap<String, Rec>, now: u64, defs: &[Def], sets: &[Set]) -> String {
    let sets_v: Vec<serde_json::Value> = sets
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id, "name": s.name, "about": s.about, "archived": s.archived,
                "earned": mirror.values().filter(|r| r.sets.contains_key(s.id)).count(),
            })
        })
        .collect();
    let defs_v: Vec<serde_json::Value> = defs
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id, "set": d.set, "name": d.name, "desc": d.desc,
                "spoiler": d.spoiler, "archived": d.archived,
                "progress": d.rule.progress().map(|(stat, max)| serde_json::json!({"stat": stat, "max": max})),
                "earned": mirror.values().filter(|r| r.got.contains_key(d.id)).count(),
            })
        })
        .collect();
    let head = serde_json::json!({
        "v": 1, "at": now, "players": mirror.len(), "sets": sets_v, "achievements": defs_v,
    })
    .to_string();
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
        let w =
            serde_json::json!({"got": rec.got, "stats": rec.stats, "sets": rec.sets}).to_string();
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

    /// Items 0..6: wood, junk, axe, bag, cloth, shirt. Deploy rows: bag, box.
    const ITEMS: [&str; 6] = ["wood", "junk", "axe", "bag", "cloth", "shirt"];
    const DEPLOYS: [u16; 2] = [3, 99];

    const T_SETS: &[Set] = &[
        Set {
            id: "a",
            name: "Alpha",
            about: "",
            archived: false,
        },
        Set {
            id: "b",
            name: "Beta",
            about: "",
            archived: false,
        },
    ];
    const T_DEFS: &[Def] = &[
        Def {
            id: "WOOD_10",
            set: "a",
            name: "Wood",
            desc: "",
            rule: Rule::Gather("wood", 10),
            spoiler: false,
            archived: false,
        },
        Def {
            id: "WOOD_20",
            set: "a",
            name: "More Wood",
            desc: "",
            rule: Rule::Gather("wood", 20),
            spoiler: false,
            archived: false,
        },
        Def {
            id: "AXE",
            set: "a",
            name: "Axe",
            desc: "",
            rule: Rule::Craft(&["axe"]),
            spoiler: false,
            archived: false,
        },
        Def {
            id: "BAG",
            set: "b",
            name: "Bag",
            desc: "",
            rule: Rule::Place(&["bag"]),
            spoiler: false,
            archived: false,
        },
        Def {
            id: "JUNK",
            set: "b",
            name: "Junk",
            desc: "",
            rule: Rule::Hold("junk", 5),
            spoiler: false,
            archived: false,
        },
        Def {
            id: "OLD",
            set: "b",
            name: "Old",
            desc: "",
            rule: Rule::Craft(&["shirt"]),
            spoiler: false,
            archived: true,
        },
        Def {
            id: "FED",
            set: "b",
            name: "Fed",
            desc: "",
            rule: Rule::FullFood,
            spoiler: true,
            archived: false,
        },
        Def {
            id: "GONE",
            set: "b",
            name: "Gone",
            desc: "",
            rule: Rule::Craft(&["nope"]),
            spoiler: false,
            archived: false,
        },
    ];

    fn baked() -> Baked {
        Baked::bake(
            T_DEFS,
            T_SETS,
            ITEMS.len(),
            |id| ITEMS.iter().position(|i| *i == id).map(|i| i as u16),
            &DEPLOYS,
        )
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

    fn lines(a: &mut Achievements) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        a.take_tells(|s, l| out.push((s, l)));
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

    #[test]
    fn a_counter_climbs_through_its_steps_and_each_unlock_is_told_once() {
        let mut a = ach();
        a.join(3, Some(W1));
        a.event(3, ev(EV_GATHER, 7, 6, 0));
        assert_eq!(a.row(3).unwrap().got, 0);
        assert_eq!(a.row(3).unwrap().stats[0], 6);
        a.event(3, ev(EV_GATHER, 7, 6, 0));
        assert_eq!(a.row(3).unwrap().got, 0b01);
        a.event(3, ev(EV_GATHER, 7, 30, 0));
        assert_eq!(a.row(3).unwrap().got, 0b11);
        assert_eq!(a.row(3).unwrap().stats[0], 42, "the stat keeps counting");
        assert_eq!(
            lines(&mut a),
            vec![
                (3, "Achievement: Wood".into()),
                (3, "Achievement: More Wood".into())
            ]
        );
        a.event(3, ev(EV_GATHER, 7, 30, 0));
        assert!(lines(&mut a).is_empty(), "nothing new, nothing said");
    }

    #[test]
    fn crafting_placing_and_holding_earn_and_a_finished_set_is_told() {
        let mut a = ach();
        a.join(0, Some(W1));
        a.event(0, ev(EV_GATHER, 1, 25, 0));
        a.event(0, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        let told = lines(&mut a);
        assert!(
            told.contains(&(0, "Set complete: Alpha".into())),
            "{told:?}"
        );
        a.event(0, ev(EV_DEPLOY_PLACED, 0, 0, 1));
        assert_eq!(a.row(0).unwrap().got & (1 << 3), 1 << 3, "row 0 is the bag");
        a.event(0, ev(EV_DEPLOY_PLACED, 0, 1, 1));
        let inv = [stack(1, 3), stack(1, 2), stack(0, 9)];
        a.swept(0, &inv, &[]);
        assert_eq!(
            a.row(0).unwrap().got & (1 << 4),
            1 << 4,
            "5 junk across two stacks"
        );
    }

    #[test]
    fn archived_and_unresolved_rows_are_never_awarded_and_never_block_a_set() {
        let b = baked();
        assert_eq!(b.unresolved, vec!["GONE: nope".to_string()]);
        let mut a = ach();
        a.join(1, Some(W1));
        a.event(1, ev(EV_CRAFT_DONE, 1, pack(5, 1), 0));
        assert_eq!(a.row(1).unwrap().got & (1 << 5), 0, "archived");
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
        assert_eq!(a.row(2).unwrap().got, 0);
        a.event(2, ev(EV_VITALS, 1, (60 << 16) | 100, full));
        a.event(2, ev(EV_CONSUMED, 1, 0, 0));
        a.event(2, ev(EV_VITALS, 1, (80 << 16) | 100, full));
        assert_eq!(a.row(2).unwrap().got, 0);
        a.event(2, ev(EV_VITALS, 1, full, full));
        assert_eq!(a.row(2).unwrap().got, 1 << 6);
    }

    #[test]
    fn guests_and_unknown_slots_are_nobody() {
        let mut a = ach();
        a.join(4, None);
        a.join(5, Some("guest"));
        assert!(!a.tracking());
        a.event(4, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        a.event(9, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        assert!(a.rows.is_empty());
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
        assert_eq!(a.row(9).unwrap().got, 1 << 2);
        assert_eq!(a.row(7).unwrap().got, 0);
    }

    #[test]
    fn the_writer_stamps_once_and_a_finished_set_stays_finished() {
        let names = baked().counter_names();
        let mut mirror = BTreeMap::new();
        let mut row = Row {
            got: 0b0111,
            ..Row::default()
        };
        row.stats[0] = 25;
        let batch = Batch {
            rows: vec![(W1.into(), row)],
        };
        apply(&mut mirror, &batch, 100, T_DEFS, T_SETS, &names);
        apply(&mut mirror, &batch, 200, T_DEFS, T_SETS, &names);
        let rec = &mirror[W1];
        assert_eq!(rec.got["WOOD_10"], 100, "a stamp never moves");
        assert_eq!(rec.sets["a"], 100);
        assert_eq!(rec.stats["gather:wood"], 25);
        // A row added to a finished set later does not unfinish it.
        const MORE: &[Def] = &[
            T_DEFS[0],
            T_DEFS[1],
            T_DEFS[2],
            Def {
                id: "NEW",
                set: "a",
                name: "New",
                desc: "",
                rule: Rule::Research,
                spoiler: false,
                archived: false,
            },
        ];
        apply(&mut mirror, &batch, 300, MORE, T_SETS, &names);
        assert!(!set_done(&mirror[W1], &T_SETS[0], MORE));
        assert_eq!(mirror[W1].sets["a"], 100, "finished stays finished");
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
        assert_eq!(a.row(0).unwrap().got, 1);
    }

    #[test]
    fn the_json_parses_and_counts_who_has_what() {
        let mut mirror = BTreeMap::new();
        let names = baked().counter_names();
        let mut row = Row {
            got: 0b0111,
            ..Row::default()
        };
        row.stats[0] = 25;
        apply(
            &mut mirror,
            &Batch {
                rows: vec![(W1.into(), row), (W2.into(), Row::default())],
            },
            9,
            T_DEFS,
            T_SETS,
            &names,
        );
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&mirror, 9, T_DEFS, T_SETS)).expect("json");
        assert_eq!(v["players"], 2);
        assert_eq!(v["achievements"][0]["earned"], 1);
        assert_eq!(v["achievements"][0]["progress"]["stat"], "gather:wood");
        assert_eq!(v["achievements"][0]["progress"]["max"], 10);
        assert_eq!(v["achievements"][6]["spoiler"], true);
        assert_eq!(v["sets"][0]["earned"], 1);
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
        a.flush();
        a.event(0, ev(EV_CRAFT_DONE, 1, pack(2, 1), 0));
        a.flush();
        assert_eq!(a.moved.len(), 1, "the ring was full: kept");
        let first = rx.pop().unwrap();
        assert_eq!(first.rows[0].1.got, 0);
        a.flush();
        assert_eq!(rx.pop().unwrap().rows[0].1.got, 1 << 2);
    }

    /// The shipped list against the shipped content: every row resolves,
    /// every unlock line fits chat, and ids are unique.
    #[test]
    fn the_shipped_list_resolves_against_the_shipped_content() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        let c = content::Content::load_dir(&dir).expect("shipped content loads");
        let deploy = c.bake_deployables().expect("deployables bake");
        let b = Baked::bake_from(&c, &deploy);
        assert!(
            b.unresolved.is_empty(),
            "rows naming missing items: {:?}",
            b.unresolved
        );
        let live: u64 = b.sets.iter().fold(0, |m, s| m | s);
        assert_eq!(
            live.count_ones() as usize,
            ACHIEVEMENTS.len(),
            "every row is in a live set"
        );
        let mut ids: Vec<_> = ACHIEVEMENTS.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ACHIEVEMENTS.len(), "ids are unique");
        for d in ACHIEVEMENTS {
            let line = crate::admin::server_line(
                &protocol::ChatText::sanitize(format!("Achievement: {}", d.name).as_bytes())
                    .expect("chat-safe"),
            );
            assert!(
                line.as_bytes().ends_with(d.name.as_bytes()),
                "{} does not fit a chat line",
                d.id
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
            let line = format!("Set complete: {}", s.name);
            assert!(line.len() + 9 <= protocol::ChatText::CAP, "{}", s.id);
        }
    }

    /// The tick's cost, measured: a full shard of wallets, a full event
    /// ring of tracked events every tick. `cargo test -p server --release
    /// -- --ignored --nocapture achievements::tests::cost`.
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
        let evs: Vec<(usize, SimEvent)> = (0..sim_core::limits::MAX_EVENTS_PER_TICK)
            .map(|i| {
                let code = [EV_GATHER, EV_CRAFT_DONE, EV_VITALS, EV_DEPLOY_PLACED][i % 4];
                let b = match code {
                    EV_GATHER => pack(wood, 3),
                    EV_CRAFT_DONE => pack(axe, 1),
                    EV_VITALS => (50 << 16) | 50,
                    _ => 0,
                };
                (i % MAX_PLAYERS, ev(code, 0, b, (100 << 16) | 100))
            })
            .collect();
        let inv = [stack(0, 5); sim_core::limits::INV_SLOTS];
        let ticks = 30 * 60 * 10;
        let t = std::time::Instant::now();
        for tick in 0..ticks {
            for &(s, e) in &evs {
                a.event(s, e);
            }
            let s = a.sweep_next();
            a.swept(s, &inv, &inv[..sim_core::limits::WEAR_SLOTS]);
            a.take_tells(|_, l| drop(l));
            if tick % (30 * FLUSH_SECS as usize) == 0 {
                a.flush();
            }
        }
        let per = t.elapsed() / ticks as u32;
        println!("achievements: {per:?} a tick at {} events", evs.len());
    }
}
