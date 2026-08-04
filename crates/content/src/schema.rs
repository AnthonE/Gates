//! The content schemas (CONTENT.md §1) as serde shapes. Every struct is
//! `deny_unknown_fields`: a field that can't be written can't be sold —
//! the never-table (DESIGN.md §3.3) enforced at the schema layer. All
//! numbers are integers so the canonical hash is exact.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    VeryRare,
}

impl Rarity {
    pub fn canon(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipSlot {
    Hand,
    Head,
    Body,
    None,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub stack: u32,
    pub tier: u32,
    pub rarity: Rarity,
    pub slot: EquipSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeArchetype {
    Tree,
    StoneNode,
    MetalNode,
    SulfurNode,
    Bush,
}

/// A second thing one node pays, flat — the bush's berries beside its
/// cloth (DECISIONS.md §open, "food you can get"). Flat on purpose: no
/// tool row and no weak-spot bonus, because picking is not chopping.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Secondary {
    pub output: String,
    /// Units per landed swing, whatever is in the hand.
    pub per_hit: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gatherable {
    pub id: String,
    pub archetype: NodeArchetype,
    /// Item this node yields.
    pub output: String,
    /// Hits to exhaust the node.
    pub hits: u32,
    /// Extra yield % paid on a weak-spot hit (the glint / the X).
    pub weak_spot_bonus_pct: u32,
    /// Tool item id (or `hand`) → units per hit. BTreeMap: canonical order.
    pub yield_per_hit: BTreeMap<String, u32>,
    /// The optional side payout. Absent on every node that pays one thing.
    #[serde(default)]
    pub secondary: Option<Secondary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Station {
    None,
    Workbench1,
    Furnace,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stack {
    pub item: String,
    pub count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub id: String,
    pub output: String,
    /// Output quantity per craft.
    pub count: u32,
    pub station: Station,
    pub seconds: u32,
    pub inputs: Vec<Stack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Foundation,
    Wall,
    Doorway,
    Floor,
    Stairs,
    Roof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    Wood,
    Stone,
    Metal,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    pub id: String,
    pub shape: Shape,
    pub material: Material,
    pub hp: u32,
    /// Direct build cost, and the upgrade-into cost (wood→stone→metal).
    pub cost: Vec<Stack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponKind {
    Melee,
    Bow,
    Firearm,
    Throwable,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ballistic {
    pub speed_mps: u32,
    pub drop_mps2: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weapon {
    /// Item id this row arms — weapons are items first.
    pub id: String,
    pub kind: WeaponKind,
    /// Damage per body hit.
    pub damage: u32,
    /// Damage per hit against a building piece or a deployable — its own
    /// column, never `damage` scaled (weapons.toml's header states the two
    /// laws that hold it; balance.rs asserts them).
    pub structure: u32,
    pub headshot_mult: u32,
    pub rate_per_min: u32,
    pub range_m: u32,
    /// Required for projectile kinds; absent on a firearm = hitscan.
    pub ballistic: Option<Ballistic>,
    pub ammo: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmorSlot {
    Head,
    Body,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Armor {
    pub id: String,
    pub slot: ArmorSlot,
    pub reduction_pct: u32,
    pub move_penalty_pct: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consumable {
    pub id: String,
    pub health: u32,
    pub food: u32,
    pub water: u32,
    pub seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Ground,
    Foundation,
    Doorway,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployArchetype {
    Bag,
    Hearth,
    Box,
    Fire,
    Furnace,
    Workbench,
    Door,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployable {
    pub id: String,
    pub archetype: DeployArchetype,
    pub placement: Placement,
    /// Doors only: pairs the door under its material's wall hp.
    pub material: Option<Material>,
    pub hp: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LootEntry {
    pub item: String,
    pub weight: u32,
    pub count_min: u32,
    pub count_max: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LootTable {
    pub id: String,
    pub container: String,
    pub rolls_min: u32,
    pub rolls_max: u32,
    /// Swings to smash the container open. Content, not a code literal:
    /// re-pricing how long a barrel takes is a balance pass, and balance
    /// passes are `content/*.toml` only (CLAUDE.md wall 7). See
    /// DECISIONS.md §open, "barrel smash hits".
    pub hits: u32,
    pub entries: Vec<LootEntry>,
}

/// Bare tickers only (CLAUDE.md wall 8) — the enum cannot spell `$SCRY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Coin {
    #[serde(rename = "SCRY")]
    Scry,
    #[serde(rename = "MYRRH")]
    Myrrh,
}

/// Appearance only: no stat field exists to write (DESIGN.md §3.3).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skin {
    pub id: String,
    pub covers: String,
    pub coin: Coin,
    pub price: u32,
    pub season: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Globals {
    pub player_hp: u32,
    pub farm_per_min: BTreeMap<String, u32>,
    pub component_minutes: BTreeMap<String, u32>,
    pub upkeep_pct_per_day: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bands {
    pub ttk_melee: [u32; 2],
    pub ttk_bow: [u32; 2],
    pub ttk_firearm: [u32; 2],
    /// Every banded weapon carries exactly this headshot multiplier.
    pub headshot_mult: u32,
    pub armor_extra_hits_max: u32,
    pub node_yield: [u32; 2],
    pub node_hits: [u32; 2],
    pub wood_wall_minutes: [u32; 2],
    /// Percent so the file stays integer-only: [100, 300] = 1.0×–3.0×.
    pub raid_ratio_stone_pct: [u32; 2],
    /// Melee swings to break the weakest door with the best melee weapon.
    /// The door is the intended breach point, so this band is the one
    /// place a hand raid is *meant* to be possible.
    pub door_breach_swings: [u32; 2],
    /// Floor on melee swings to break *any* wall at any material — the
    /// wall is what the satchel is for, and melee must never undercut it.
    pub wall_breach_swings_min: u32,
    pub upkeep_solo_daily_max_min: u32,
}

/// The death backpack's despawn ladder (`content/balance.toml`
/// `[backpack]`; NETCODE.md §6.4 — one base constant × a rarity
/// multiplier). Lifetime = base × the multiplier of the rarest item the
/// bag holds; an empty bag rides the base.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backpack {
    pub despawn_base_min: u32,
    pub mult_common: u32,
    pub mult_uncommon: u32,
    pub mult_rare: u32,
    pub mult_very_rare: u32,
}

impl Backpack {
    /// The ladder in `Rarity::canon()` order — the one place the four
    /// named fields become an indexable row.
    pub fn mults(&self) -> [u32; 4] {
        [
            self.mult_common,
            self.mult_uncommon,
            self.mult_rare,
            self.mult_very_rare,
        ]
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PieceCount {
    pub piece: String,
    pub count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StarterBase {
    pub pieces: Vec<PieceCount>,
    pub items: Vec<Stack>,
}

/// The survival clock (`content/balance.toml` `[survival]`; DESIGN.md §2
/// "hunger/thirst minimal — a slow health drain past a timer, food to reset
/// it"). Minutes and per-minute rates, because those are the units the
/// design speaks; `bake_survival` converts them to ticks once so the sim
/// never multiplies a clock. Proposed defaults, DECISIONS.md §open
/// ("survival clock v0").
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Survival {
    pub max_food: u32,
    pub max_water: u32,
    /// Minutes from a full meter to an empty one, at rest.
    pub food_minutes_to_empty: u32,
    pub water_minutes_to_empty: u32,
    /// Hit points a minute while the matching meter reads zero.
    pub starve_hp_per_min: u32,
    pub dehydrate_hp_per_min: u32,
    /// The drink verb (wire v15, `survival::drink`): water units one
    /// mouthful of the sea restores, and the hit points swallowing it
    /// costs. The sea is salt — that is the design, not a tax. Zero
    /// `drink_water` disarms the verb, and `validate::structural` then
    /// requires a gatherable to answer thirst instead.
    pub drink_water: u32,
    pub drink_hp_cost: u32,
}

/// The declared bands + globals the anchors compute against
/// (`content/balance.toml`; DECISIONS.md §open "balance bands").
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balance {
    pub globals: Globals,
    pub bands: Bands,
    pub banded_nodes: Vec<NodeArchetype>,
    pub starter_base: StarterBase,
    pub backpack: Backpack,
    pub survival: Survival,
}
