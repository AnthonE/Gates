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
    /// Maximum condition, **hundredths of a point** (item durability v0,
    /// DECISIONS.md 2026-08-15 — taken from the reference, per item, never
    /// one constant: rock 10 000, torch 5 000, stone tools 10 000, metal
    /// tools 40 000). **Absent means 0 means never wears and can never be
    /// repaired** — the schema default IS the rule for non-tools, so wood
    /// and every consumable simply do not write the line. A nonzero value
    /// requires `stack = 1` (validation rule V7): condition is per-stack
    /// state and two conditions in one slot is a merge nobody can resolve.
    #[serde(default)]
    pub condition_max: u32,
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
    /// Tool item id → condition loss per landed hit, **hundredths of a
    /// point** (item durability v0; the reference's 0.3/hit is 30). Keyed
    /// per **(tool, node)** exactly as `yield_per_hit` is, because the
    /// table IS the wrong-tool predicate (`reference/DURABILITY.md` §2 —
    /// a metal hatchet pays 0.3 on a tree and 1.0 on flesh, one tool, two
    /// rates, chosen by what it is swung at). There is no predicate to
    /// port. Never `hand` (bare hands do not wear — V2), and every
    /// condition-carrying tool a node pays must have a row (V4).
    #[serde(default)]
    pub condition_loss: BTreeMap<String, u32>,
    /// The optional side payout. Absent on every node that pays one thing.
    #[serde(default)]
    pub secondary: Option<Secondary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Station {
    None,
    Workbench1,
    /// The bench ladder's second and third rungs (bench ladder v0, the
    /// pre-Oct-2025 scrap-era shape, operator 2026-08-15). Declared
    /// between `Workbench1` and `Furnace` so the enum's order stays the
    /// baked code's order — the furnace moved 2 → 4 with them.
    Workbench2,
    Workbench3,
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
    /// Locked behind research (`content/research.toml`, research v0):
    /// nobody may craft this until they have learned it. Defaults false —
    /// most of the ladder is open, and a gate you have to opt into is a
    /// gate you cannot apply by accident.
    #[serde(default)]
    pub blueprint: bool,
}

/// One row of `content/research.toml`: an item you can take to a table,
/// and what learning it costs in the coin the file names.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Research {
    /// The item consumed. The recipe it unlocks is the one that outputs
    /// this item — resolved at bake, so the file never names a recipe id
    /// and the two can never disagree about which thing was learned.
    pub item: String,
    /// Units of the coin burned. Zero is legal (a free unlock is a
    /// tutorial, not a mistake).
    pub cost: u32,
    /// The tech tree's edge (tech tree v0): the **item** of another
    /// research row that must be learned before this node may be bought
    /// at a bench. Absent means a root. Written as an item rather than a
    /// recipe id for the same reason `item` is — the file speaks in
    /// things a player recognises, and the bake resolves the graph.
    /// Only the tree verb reads it; the research table takes a looted
    /// sample with no questions asked.
    #[serde(default)]
    pub requires: Option<String>,
}

/// The head of `content/research.toml`: what research is paid in.
///
/// One coin for the whole table, named here rather than assumed in code —
/// `sim-core/research.rs` receives an item index and never learns that it
/// is currency, which is what keeps `DESIGN.md` §3.1 out of `crates/`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchCoin {
    pub item: String,
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
    /// The two socket shapes (catalogue v1, `reference/BUILDING.md`
    /// §9.13): openings priced by what they deny net of the insert you
    /// still owe — window 0.7 of the wall, frame 0.5 (§7b.3).
    Window,
    WallFrame,
    /// The triangle footprint (triangles v0, §9.14): the half-cell along
    /// a diagonal, at §7b.3's own ratios — tri foundation and tri roof
    /// 0.5 of the wall, tri floor 0.25. The diagonal WALL that closes a
    /// hypotenuse is not a shape: it is the wall, on a diagonal slot.
    TriFoundation,
    TriFloor,
    TriRoof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    /// The placement state, not a grade: everything is built as twig and
    /// a hammer commits it upward (`reference/BUILDING.md` §7b.4). First
    /// in the enum because the order IS the upgrade ladder — `Ord` here
    /// is what `decay_pct_per_period`'s map and the ladder checks read.
    Twig,
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
    /// Direct build cost, and the upgrade-into cost. Only a twig row is
    /// ever paid as a *placement*; the rest are paid on top of it, by the
    /// hammer (sim-core `build::place` refuses anything else).
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
    /// A recycler (recycler v0): a container that converts without
    /// burning. An oven in the sim (`sim-core/oven.rs`) minus the fuel,
    /// and the economy's first faucet — what it pays is rows in
    /// `content/cooking.toml`, never code.
    Recycler,
    /// A research table (research v0). A **station** rather than a
    /// container — checked by proximity like the workbench, holding
    /// nothing (`sim-core/research.rs` says why) — and the coin's sink.
    Research,
    /// The bench ladder's upper rungs (bench ladder v0): stations like
    /// `Workbench`, one tier each. Their own archetypes rather than a
    /// tier field, because the archetype is what the wire carries, the
    /// client silhouettes and `bench_near` scans — `sim-core/deploy.rs`
    /// `bench_tier` is the one place the rung order is written.
    Workbench2,
    Workbench3,
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

/// Which container runs a cook row. The archetype names of
/// `content/deployables.toml`, narrowed to the three that convert — a row
/// that named `box` would be a transformation with no station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookStation {
    Fire,
    Furnace,
    /// Converts without burning (recycler v0). The station that makes this
    /// table the economy's arming point: a row here is a faucet.
    Recycler,
}

/// One transformation a container performs: one unit in, `count` units of
/// `output` out, over `seconds`, at `station`.
///
/// **Several rows may share one `(station, input)`** — that is how a
/// component recycles into metal *and* coin — and when they do, the sim
/// fires all of them together on one clock. `validate::structural`
/// therefore holds such a set to a single `seconds` and refuses two rows
/// that pay the same output.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cook {
    pub input: String,
    pub output: String,
    /// Units of `output` one conversion pays. Defaults to 1, which is
    /// every cooking row — the field exists for the recycler, where a
    /// component is worth eight fragments.
    #[serde(default = "one")]
    pub count: u32,
    pub seconds: u32,
    pub station: CookStation,
}

/// `Cook::count`'s default. A free function because `serde(default = …)`
/// takes a path and not a literal.
fn one() -> u32 {
    1
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
    /// Flight-and-charge speed, percent of `SPRINT_SPEED` — one fast gait
    /// for both directions of a rousing.
    pub flee_pct: u32,
    /// How long one rousing lasts — the fright's span and the charge's
    /// commitment.
    pub flee_seconds: u32,
    /// Damage per bite. Zero = this species never fights (the pacifist
    /// row stays expressible; validate refuses the half-armed states).
    pub attack: u32,
    /// Bite reach, metres.
    pub attack_range_m: u32,
    /// Seconds between bites.
    pub attack_seconds: u32,
    /// Percent of max hp at which courage runs out: at or above it a
    /// roused animal charges its tormentor, below it the same rousing is
    /// a flight. The reference boar's rule — fights whole, flees hurt.
    pub brave_pct: u32,
    /// Leash radius from the home the seed chose.
    pub roam_m: u32,
    /// A player closer than this starts a flight — in daylight.
    pub spook_m: u32,
    /// The same radius after dusk. Required, like every other field here:
    /// a species that did not say what it does at night would be defaulted
    /// into an answer, and the whole point of the field is that the hour is
    /// a content decision. Free to be larger, smaller or equal — validate
    /// holds the reachability bands at *both* hours and takes no view on
    /// the direction.
    pub night_spook_m: u32,
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
