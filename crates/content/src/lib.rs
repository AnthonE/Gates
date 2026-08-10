//! Content loading + validation (CLAUDE.md wall 7, CONTENT.md §0). Boot
//! calls `Content::load_dir`; a set that fails schema, reference, or
//! balance-band checks refuses to load — the shard does not boot on
//! drifted content. The canonical hash pins into the WAL header when the
//! WAL file format lands (until then it is logged at boot).
//!
//! This crate is boot-path, not tick-path: it allocates freely and does
//! I/O. Nothing here is called from the sim thread.

pub mod bake;
pub mod balance;
mod canon;
pub mod schema;
mod validate;

use schema::*;
use std::path::Path;

pub use balance::Anchors;

/// Every file the content set is made of — exactly these, no extras.
/// A missing file is a loud failure, never a defaulted section.
pub const FILES: [&str; 14] = [
    "items.toml",
    "gatherables.toml",
    "recipes.toml",
    "building.toml",
    "weapons.toml",
    "armor.toml",
    "consumables.toml",
    "deployables.toml",
    "cooking.toml",
    "research.toml",
    "loot.toml",
    "mobs.toml",
    "skins.toml",
    "balance.toml",
];

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemsFile {
    item: Vec<Item>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GatherablesFile {
    gatherable: Vec<Gatherable>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipesFile {
    recipe: Vec<Recipe>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildingFile {
    piece: Vec<Piece>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WeaponsFile {
    weapon: Vec<Weapon>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ArmorFile {
    armor: Vec<Armor>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsumablesFile {
    consumable: Vec<Consumable>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeployablesFile {
    deployable: Vec<Deployable>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CookingFile {
    fuel: Fuel,
    /// Absent is legal and means an oven that burns but transforms
    /// nothing — the shipped state today, and the honest one while the
    /// island grows no raw food (`content/cooking.toml` says why).
    #[serde(default)]
    cook: Vec<Cook>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResearchFile {
    coin: ResearchCoin,
    /// Absent is legal and means nothing is researchable — a shard where
    /// the whole ladder is open, which is what every shard was before
    /// research v0.
    #[serde(default)]
    research: Vec<Research>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LootFile {
    loot_table: Vec<LootTable>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MobsFile {
    #[serde(default)]
    mob: Vec<Mob>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SkinsFile {
    skin: Vec<Skin>,
}

/// The whole validated content set. Construction is the only way in, so
/// holding a `Content` means every check in `validate` and every band in
/// `balance` passed.
#[derive(Debug, Clone)]
pub struct Content {
    pub items: Vec<Item>,
    pub gatherables: Vec<Gatherable>,
    pub recipes: Vec<Recipe>,
    pub pieces: Vec<Piece>,
    pub weapons: Vec<Weapon>,
    pub armors: Vec<Armor>,
    pub consumables: Vec<Consumable>,
    pub deployables: Vec<Deployable>,
    pub fuel: Fuel,
    pub cooks: Vec<Cook>,
    /// What research is paid in, and what it teaches (research v0).
    pub research_coin: ResearchCoin,
    pub research: Vec<Research>,
    pub loot_tables: Vec<LootTable>,
    /// The animal roster's species table. Empty is legal and means a
    /// shard with no wildlife — the file still has to exist, because a
    /// missing file is a loud failure here and never a defaulted section.
    pub mobs: Vec<Mob>,
    pub skins: Vec<Skin>,
    pub balance: Balance,
    anchors: Anchors,
}

fn parse<T: serde::de::DeserializeOwned>(name: &str, text: &str) -> Result<T, String> {
    toml::from_str(text).map_err(|e| format!("{name}: {e}"))
}

impl Content {
    /// Load `FILES` from a directory. Extra `.toml` files are refused —
    /// a typo'd filename must not silently drop a section from the set.
    pub fn load_dir(dir: &Path) -> Result<Self, String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".toml") && !FILES.contains(&name.as_ref()) {
                return Err(format!(
                    "{}: unexpected content file `{name}` — the set is exactly {FILES:?}",
                    dir.display()
                ));
            }
        }
        let mut texts = Vec::with_capacity(FILES.len());
        for f in FILES {
            let path = dir.join(f);
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            texts.push((f, text));
        }
        let borrowed: Vec<(&str, &str)> = texts.iter().map(|(n, t)| (*n, t.as_str())).collect();
        Self::from_sources(&borrowed)
    }

    /// Build from named sources (the test path). Must contain every name
    /// in `FILES` exactly once and nothing else.
    pub fn from_sources(sources: &[(&str, &str)]) -> Result<Self, String> {
        for (n, _) in sources {
            if !FILES.contains(n) {
                return Err(format!("content: unexpected source `{n}`"));
            }
        }
        let get = |name: &str| -> Result<&str, String> {
            let mut found = None;
            for (n, t) in sources {
                if *n == name {
                    if found.is_some() {
                        return Err(format!("content: `{name}` given twice"));
                    }
                    found = Some(*t);
                }
            }
            found.ok_or_else(|| format!("content: missing `{name}`"))
        };
        let items: ItemsFile = parse("items.toml", get("items.toml")?)?;
        let gatherables: GatherablesFile = parse("gatherables.toml", get("gatherables.toml")?)?;
        let recipes: RecipesFile = parse("recipes.toml", get("recipes.toml")?)?;
        let building: BuildingFile = parse("building.toml", get("building.toml")?)?;
        let weapons: WeaponsFile = parse("weapons.toml", get("weapons.toml")?)?;
        let armor: ArmorFile = parse("armor.toml", get("armor.toml")?)?;
        let consumables: ConsumablesFile = parse("consumables.toml", get("consumables.toml")?)?;
        let deployables: DeployablesFile = parse("deployables.toml", get("deployables.toml")?)?;
        let cooking: CookingFile = parse("cooking.toml", get("cooking.toml")?)?;
        let research: ResearchFile = parse("research.toml", get("research.toml")?)?;
        let loot: LootFile = parse("loot.toml", get("loot.toml")?)?;
        let mobs: MobsFile = parse("mobs.toml", get("mobs.toml")?)?;
        let skins: SkinsFile = parse("skins.toml", get("skins.toml")?)?;
        let balance: Balance = parse("balance.toml", get("balance.toml")?)?;

        let mut content = Content {
            items: items.item,
            gatherables: gatherables.gatherable,
            recipes: recipes.recipe,
            pieces: building.piece,
            weapons: weapons.weapon,
            armors: armor.armor,
            consumables: consumables.consumable,
            deployables: deployables.deployable,
            fuel: cooking.fuel,
            cooks: cooking.cook,
            research_coin: research.coin,
            research: research.research,
            loot_tables: loot.loot_table,
            mobs: mobs.mob,
            skins: skins.skin,
            balance,
            anchors: Anchors::default(),
        };
        validate::structural(&content)?;
        content.anchors = balance::check(&content)?;
        Ok(content)
    }

    /// The computed balance anchors (CONTENT.md §4), for the boot log.
    pub fn anchors(&self) -> &Anchors {
        &self.anchors
    }

    /// xxh3 over the canonical serialization of the parsed set — stable
    /// against formatting and comments, moved by any value. Pins into the
    /// WAL header (CONTENT.md §0) when the WAL file format lands.
    pub fn hash(&self) -> u64 {
        canon::hash(self)
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn recipe_for(&self, output: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.output == output)
    }
}
