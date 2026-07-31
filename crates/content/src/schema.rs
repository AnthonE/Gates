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
    /// Damage per body hit; for the satchel, structure damage.
    pub damage: u32,
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
    pub armor_extra_hits_max: u32,
    pub node_yield: [u32; 2],
    pub node_hits: [u32; 2],
    pub wood_wall_minutes: [u32; 2],
    /// Percent so the file stays integer-only: [100, 300] = 1.0×–3.0×.
    pub raid_ratio_stone_pct: [u32; 2],
    pub upkeep_solo_daily_max_min: u32,
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

/// The declared bands + globals the anchors compute against
/// (`content/balance.toml`; DECISIONS.md §open "balance bands").
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balance {
    pub globals: Globals,
    pub bands: Bands,
    pub banded_nodes: Vec<NodeArchetype>,
    pub starter_base: StarterBase,
}
