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
    /// Unmarked hits to exhaust the node. The node's whole payout is
    /// `hits × yield_per_hit[tool]` however it is struck.
    pub hits: u32,
    /// Extra **budget** % a weak-spot hit (the glint / the X) spends.
    /// The mark buys SPEED, not yield: a marked swing takes a bigger
    /// bite of the same pool and is paid pro rata, so a skilled player
    /// empties the node in fewer swings and everyone banks the same
    /// total. `sim_core::gather::NodeDef::weak_pct` has the model.
    pub weak_spot_bonus_pct: u32,
    /// % of the node's whole payout withheld from the per-swing pay and
    /// handed to whoever lands the exhausting swing. A redistribution,
    /// never a bonus on top — it prices abandoning a half-struck node.
    /// 0 pays evenly.
    #[serde(default)]
    pub finish_bonus_pct: u32,
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

/// One round, and **the object its own ballistics belong to**.
///
/// This used to be a `[weapon.ballistic]` block on the bow, and moving it
/// here is `reference/PROJECTILES.md` §9.3 (operator, 2026-08-10). The
/// reference game hangs ballistics off `ItemModProjectile` — a mod on the
/// *ammo item* — which is why one bow there fires four arrows that differ
/// in speed, drop and impact while the bow stays one object. With the
/// numbers on the weapon, an arrow variant is unreachable at any values:
/// the bow decides how fast its arrow flies, so every arrow it fires flies
/// the same.
///
/// Damage stays on the weapon. That is not the reference's split — theirs
/// scales the weapon's damage by the round's — but the multiplier is a
/// number nobody has spoken, and inventing one is `DECISIONS.md` §open's
/// job rather than this struct's.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ammo {
    /// Item id this row arms — ammo is an item first, exactly as weapons
    /// are. An `[[ammo]]` row naming no item is refused at boot.
    pub id: String,
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
    /// The rounds this weapon can fire, in **preference order** — the sim
    /// spends the first one the shooter is actually carrying. Required on
    /// `bow`, refused on melee and throwable.
    ///
    /// A list rather than one id because the reference game's bow is a
    /// `BaseProjectile` with `SwitchAmmoTo`: one weapon, several rounds.
    /// We have the *capacity* here and not yet the verb — there is no way
    /// to ask for a particular arrow, so order in this list is the whole
    /// of the policy. Every shipped row names exactly one round today, so
    /// nothing about what a bow fires has changed with the schema.
    pub ammo: Option<Vec<String>>,
    /// Fuse seconds — required on `throwable`, refused on every other
    /// kind (`validate.rs`), because it is the one column a swing has no
    /// meaning for. The bake turns it into ticks against `TICK_HZ` the
    /// way `range_m` becomes `reach_cm`, so no float rounding of a
    /// content number reaches the sim.
    pub fuse_s: Option<u32>,
    /// Blast radius in metres — required on `throwable`, refused on every
    /// other kind, `fuse_s`'s treatment for `fuse_s`'s reason. Baked to cm
    /// like `range_m`, so no float from a content file reaches the sim.
    ///
    /// **No sim code reads the baked value yet.** It is intended to be the
    /// number that makes a breach a *hole* rather than a wall taken down
    /// one address at a time, scaling both damage columns — the
    /// `structure` a neighbouring piece takes and the `damage` a player
    /// standing in it takes, both falling off linearly to zero at this
    /// distance. That consumer does not exist; see `combat::ThrowDef`.
    pub blast_m: Option<u32>,
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
    /// On a door — the only class whose target must be **occupied**, and
    /// occupied by one specific archetype (lock v1).
    Door,
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
    /// A code lock. The one archetype that becomes no deploy record: it
    /// bolts onto a door's address and lives in the sim's lock store
    /// (`sim-core/lock.rs`, `reference/DOORS.md` §9.1).
    Lock,
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

/// What an oven burns (`content/cooking.toml`, `sim-core/oven.rs`).
/// One row for the whole game: the reference carries an
/// `ItemModBurnable` per item, and a second burnable here is a schema
/// change we will make when a second one exists rather than a `Vec` that
/// has held one element since the day it was written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fuel {
    pub item: String,
    /// How long one unit burns.
    pub seconds: u32,
    /// What a burned unit leaves in the oven.
    pub byproduct: String,
    /// Hundredths of a byproduct unit banked per unit burned, paid whole
    /// at 100. An integer rather than a roll so the fire's yield is in
    /// `state_hash` without an RNG draw in the tick.
    pub byproduct_pct: u32,
}

/// Which oven runs a cook row. The archetype names of
/// `content/deployables.toml`, narrowed to the two that burn — a row that
/// named `box` would be a transformation with no station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookStation {
    Fire,
    Furnace,
}

/// One transformation an oven performs: one unit in, one unit out, over
/// `seconds`, at `station`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cook {
    pub input: String,
    pub output: String,
    pub seconds: u32,
    pub station: CookStation,
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

/// One animal species (`sim-core/src/mob.rs`).
///
/// Speeds are **percentages of the player's own**, not metres per second,
/// and that is the schema being honest about the sim rather than being
/// friendly: an animal drives `movement::step` through the same
/// `InputFrame` a player does, so the only speed it can express is a
/// fraction of `WALK_SPEED` (or of `SPRINT_SPEED` while it runs). A
/// m/s field here would be a number the bake had to quietly round to the
/// nearest 1/127th, and a content author would never learn which way.
///
/// Distances are metres and times are seconds — the units a balance pass
/// is argued in. The bake converts both, so nothing in `mob.rs` knows what
/// a second is.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mob {
    /// `mob.<species>` — the ordinal the sim knows it by is resolved from
    /// this name at bake, the way a loot table's container is.
    pub id: String,
    pub name: String,
    pub hp: u32,
    /// Amble speed, percent of `WALK_SPEED`.
    pub walk_pct: u32,
    /// Flight speed, percent of `SPRINT_SPEED`.
    pub flee_pct: u32,
    /// How long one fright lasts.
    pub flee_seconds: u32,
    /// Leash radius from the home the seed chose.
    pub roam_m: u32,
    /// A player closer than this starts a flight.
    pub spook_m: u32,
    /// Time between a death and the same slot standing up again at the
    /// same home.
    pub respawn_seconds: u32,
    /// What the killing blow pays. Straight into the killer's inventory
    /// (`mob::strike`), so these are stacks and not a weighted table —
    /// butchering an animal is not opening a barrel.
    pub drops: Vec<Stack>,
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
    /// Repair price as a percent of the pro-rata share of the piece's own
    /// build cost (100 = the damage's worth exactly). Percent so the file
    /// stays integer-only, like `raid_ratio_stone_pct`.
    pub repair_cost_pct: u32,
    /// Unpaid decay per upkeep period, % of max hp, keyed by material.
    /// A map rather than three fields so a fourth grade is a data change
    /// (`Material`'s own set is what validate checks it against).
    pub decay_pct_per_period: BTreeMap<Material, u32>,
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
    /// What a fresh character spawns holding (`[[spawn_kit]]`).
    ///
    /// **Defaulted, because a naked spawn is the game.** Content that
    /// authors no kit still boots and grants nothing — which is what a
    /// public shard wants and what every test fixture already assumes. The
    /// alpha's kit exists so the build and hammer verbs can be exercised at
    /// all; `DECISIONS.md` §open "spawn kit v0" is where it gets armed or
    /// emptied for a real shard, and that is an operator call.
    #[serde(default)]
    pub spawn_kit: Vec<Stack>,
}
