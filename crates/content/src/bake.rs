//! Bake: the validated content set → the pure fixed-capacity tables the
//! sim consumes (`sim_core::gather::GatherContent`). This is the one
//! bridge across CLAUDE.md wall 7 — data crosses, code never does. Item
//! indices are the rank of the item id in sorted order: canonical for a
//! given set, and the set itself is pinned by the content hash.
//!
//! Boot path only: allocation and `String` errors are fine here; nothing
//! in this module runs on the sim thread.

use crate::schema::{
    DeployArchetype, Material, NodeArchetype, Placement, Shape, Station, WeaponKind,
};
use crate::Content;
use sim_core::backpack::BackpackContent;
use sim_core::build::{
    BuildContent, PieceDef, MAT_METAL, MAT_STONE, MAT_WOOD, SHAPE_DOORWAY, SHAPE_FLOOR,
    SHAPE_FOUNDATION, SHAPE_ROOF, SHAPE_STAIRS, SHAPE_WALL,
};
use sim_core::combat::{CombatContent, MeleeDef, ThrowDef};
use sim_core::craft::{CraftContent, RecipeDef, STATION_FURNACE, STATION_NONE, STATION_WORKBENCH1};
use sim_core::deploy::{
    DeployContent, DeployDef, ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE, ARCH_HEARTH,
    ARCH_WORKBENCH, PLACE_ANY, PLACE_DOORWAY, PLACE_FOUNDATION, PLACE_GROUND,
};
use sim_core::gather::{GatherContent, NodeDef, MAX_TOOLS_PER_NODE, NO_ITEM};
use sim_core::limits::{
    HEARTH_STOCK_ROWS, MAX_DEPLOY_COSTS, MAX_DEPLOY_DEFS, MAX_ITEM_DEFS, MAX_LOOT_ENTRIES,
    MAX_LOOT_ROLLS, MAX_LOOT_TABLES, MAX_PIECE_COSTS, MAX_PIECE_DEFS, MAX_RECIPES,
    MAX_RECIPE_INPUTS, TICK_HZ,
};
use sim_core::loot::{
    LootContent, LootEntryDef, LootTableDef, LOOT_BARREL, LOOT_CACHE, LOOT_CRATE,
};
use sim_core::survival::{ConsumableDef, SurvivalContent, TICKS_PER_MIN};

/// Gatherable index (terrain `Occupant as usize - 1`) of each archetype.
fn node_slot(a: NodeArchetype) -> usize {
    match a {
        NodeArchetype::Tree => 0,
        NodeArchetype::StoneNode => 1,
        NodeArchetype::MetalNode => 2,
        NodeArchetype::SulfurNode => 3,
        NodeArchetype::Bush => 4,
    }
}

impl Content {
    /// Rank of `id` among all item ids, sorted — the sim-side item index.
    /// The wire slice will ship this same mapping in the join bundle.
    pub fn item_index(&self, id: &str) -> Option<u16> {
        let mut rank = 0u16;
        let mut found = false;
        for item in &self.items {
            if item.id.as_str() < id {
                rank += 1;
            } else if item.id == id {
                found = true;
            }
        }
        found.then_some(rank)
    }

    /// The gather tables. Refuses sets the sim's fixed capacities can't
    /// hold — a refused bake is a refused boot, same as failed validation.
    pub fn bake_gather(&self) -> Result<GatherContent, String> {
        if self.items.len() > MAX_ITEM_DEFS {
            return Err(format!(
                "bake: {} items exceed the sim's {MAX_ITEM_DEFS}-def table",
                self.items.len()
            ));
        }
        let mut gc = GatherContent::EMPTY;
        gc.item_count = self.items.len() as u16;
        for item in &self.items {
            let idx = self.item_index(&item.id).expect("own id") as usize;
            gc.stack_max[idx] = u16::try_from(item.stack)
                .map_err(|_| format!("bake: `{}` stack {} overflows u16", item.id, item.stack))?;
        }
        for g in &self.gatherables {
            let slot = node_slot(g.archetype);
            if gc.nodes[slot].output != NO_ITEM {
                return Err(format!(
                    "bake: duplicate gatherable for {:?} (`{}`)",
                    g.archetype, g.id
                ));
            }
            let mut def = NodeDef {
                output: self
                    .item_index(&g.output)
                    .ok_or_else(|| format!("bake: `{}` output missing", g.id))?,
                hits: u16::try_from(g.hits)
                    .map_err(|_| format!("bake: `{}` hits overflow u16", g.id))?,
                hand_yield: 0,
                weak_pct: u16::try_from(g.weak_spot_bonus_pct)
                    .map_err(|_| format!("bake: `{}` weak-spot bonus overflows u16", g.id))?,
                tools: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
                secondary: match &g.secondary {
                    None => (NO_ITEM, 0),
                    Some(s) => (
                        self.item_index(&s.output)
                            .ok_or_else(|| format!("bake: `{}` secondary output missing", g.id))?,
                        u16::try_from(s.per_hit).map_err(|_| {
                            format!("bake: `{}` secondary per_hit overflows u16", g.id)
                        })?,
                    ),
                },
            };
            let mut tool_n = 0usize;
            for (tool, per_hit) in &g.yield_per_hit {
                let per = u16::try_from(*per_hit)
                    .map_err(|_| format!("bake: `{}` yield for `{tool}` overflows u16", g.id))?;
                if tool == "hand" {
                    def.hand_yield = per;
                    continue;
                }
                if tool_n == MAX_TOOLS_PER_NODE {
                    return Err(format!(
                        "bake: `{}` has more than {MAX_TOOLS_PER_NODE} tool rows",
                        g.id
                    ));
                }
                let idx = self
                    .item_index(tool)
                    .ok_or_else(|| format!("bake: `{}` tool `{tool}` missing", g.id))?;
                def.tools[tool_n] = (idx, per);
                tool_n += 1;
            }
            gc.nodes[slot] = def;
        }
        Ok(gc)
    }

    /// Rank of `id` among all recipe ids, sorted — the wire-side recipe
    /// index (same canonical mapping as `item_index`).
    pub fn recipe_index(&self, id: &str) -> Option<u16> {
        let mut rank = 0u16;
        let mut found = false;
        for r in &self.recipes {
            if r.id.as_str() < id {
                rank += 1;
            } else if r.id == id {
                found = true;
            }
        }
        found.then_some(rank)
    }

    /// The craft table. Refuses sets the sim's fixed capacities — or the
    /// wire's field widths (seconds u16, output count u8) — can't hold.
    pub fn bake_craft(&self) -> Result<CraftContent, String> {
        if self.recipes.len() > MAX_RECIPES {
            return Err(format!(
                "bake: {} recipes exceed the sim's {MAX_RECIPES}-row table",
                self.recipes.len()
            ));
        }
        let mut cc = CraftContent::EMPTY;
        cc.recipe_count = self.recipes.len() as u16;
        for r in &self.recipes {
            let idx = self.recipe_index(&r.id).expect("own id resolves") as usize;
            if r.seconds == 0 || r.seconds > u16::MAX as u32 {
                return Err(format!(
                    "bake: `{}` seconds {} outside 1..=65535",
                    r.id, r.seconds
                ));
            }
            if r.count == 0 || r.count > u8::MAX as u32 {
                return Err(format!(
                    "bake: `{}` output count {} outside 1..=255",
                    r.id, r.count
                ));
            }
            if r.inputs.len() > MAX_RECIPE_INPUTS {
                return Err(format!(
                    "bake: `{}` has more than {MAX_RECIPE_INPUTS} inputs",
                    r.id
                ));
            }
            let mut def = RecipeDef {
                output: self
                    .item_index(&r.output)
                    .ok_or_else(|| format!("bake: `{}` output missing", r.id))?,
                out_count: r.count as u16,
                ticks: r.seconds * TICK_HZ,
                station: match r.station {
                    Station::None => STATION_NONE,
                    Station::Workbench1 => STATION_WORKBENCH1,
                    Station::Furnace => STATION_FURNACE,
                },
                n_inputs: r.inputs.len() as u8,
                inputs: [(0, 0); MAX_RECIPE_INPUTS],
            };
            for (n, input) in r.inputs.iter().enumerate() {
                let item = self
                    .item_index(&input.item)
                    .ok_or_else(|| format!("bake: `{}` input `{}` missing", r.id, input.item))?;
                let count = u16::try_from(input.count).map_err(|_| {
                    format!(
                        "bake: `{}` input `{}` count overflows u16",
                        r.id, input.item
                    )
                })?;
                def.inputs[n] = (item, count);
            }
            cc.recipes[idx] = def;
        }
        Ok(cc)
    }

    /// Rank of `id` among all building-piece ids, sorted — the wire-side
    /// piece row (same canonical mapping as `item_index`).
    pub fn piece_index(&self, id: &str) -> Option<u16> {
        let mut rank = 0u16;
        let mut found = false;
        for p in &self.pieces {
            if p.id.as_str() < id {
                rank += 1;
            } else if p.id == id {
                found = true;
            }
        }
        found.then_some(rank)
    }

    /// The build table. Refuses sets the sim's fixed capacities — or the
    /// wire's field widths (hp u16) — can't hold.
    pub fn bake_building(&self) -> Result<BuildContent, String> {
        if self.pieces.len() > MAX_PIECE_DEFS {
            return Err(format!(
                "bake: {} building pieces exceed the sim's {MAX_PIECE_DEFS}-row table",
                self.pieces.len()
            ));
        }
        let mut bc = BuildContent::EMPTY;
        bc.piece_count = self.pieces.len() as u16;
        for p in &self.pieces {
            let idx = self.piece_index(&p.id).expect("own id resolves") as usize;
            if p.cost.len() > MAX_PIECE_COSTS {
                return Err(format!(
                    "bake: `{}` has more than {MAX_PIECE_COSTS} cost rows",
                    p.id
                ));
            }
            let mut def = PieceDef {
                shape: match p.shape {
                    Shape::Foundation => SHAPE_FOUNDATION,
                    Shape::Wall => SHAPE_WALL,
                    Shape::Doorway => SHAPE_DOORWAY,
                    Shape::Floor => SHAPE_FLOOR,
                    Shape::Stairs => SHAPE_STAIRS,
                    Shape::Roof => SHAPE_ROOF,
                },
                material: match p.material {
                    Material::Wood => MAT_WOOD,
                    Material::Stone => MAT_STONE,
                    Material::Metal => MAT_METAL,
                },
                hp: u16::try_from(p.hp)
                    .map_err(|_| format!("bake: `{}` hp {} overflows u16", p.id, p.hp))?,
                n_costs: p.cost.len() as u8,
                costs: [(0, 0); MAX_PIECE_COSTS],
            };
            if def.hp == 0 {
                return Err(format!("bake: `{}` hp 0 is the inert sentinel", p.id));
            }
            for (n, cost) in p.cost.iter().enumerate() {
                let item = self
                    .item_index(&cost.item)
                    .ok_or_else(|| format!("bake: `{}` cost `{}` missing", p.id, cost.item))?;
                let count = u16::try_from(cost.count).map_err(|_| {
                    format!("bake: `{}` cost `{}` count overflows u16", p.id, cost.item)
                })?;
                // One row per item: the sim checks each row's
                // affordability independently, so a double-listed item
                // would pass the check yet under-collect.
                if def.costs[..n].iter().any(|&(i, _)| i == item) {
                    return Err(format!("bake: `{}` lists cost `{}` twice", p.id, cost.item));
                }
                def.costs[n] = (item, count);
            }
            bc.pieces[idx] = def;
        }
        // `validate::structural` has already pinned this to 1..=100, so the
        // narrow cannot fail on shipped content; it is written as a bake
        // refusal anyway because `bake_building` is reachable from fixtures
        // that never ran the validator.
        bc.repair_pct = u16::try_from(self.balance.globals.repair_cost_pct).map_err(|_| {
            format!(
                "bake: repair_cost_pct {} overflows u16",
                self.balance.globals.repair_cost_pct
            )
        })?;
        Ok(bc)
    }

    /// Rank of `id` among all deployable ids, sorted — the wire-side
    /// deployable row (same canonical mapping as `item_index`).
    pub fn deploy_index(&self, id: &str) -> Option<u16> {
        let mut rank = 0u16;
        let mut found = false;
        for d in &self.deployables {
            if d.id.as_str() < id {
                rank += 1;
            } else if d.id == id {
                found = true;
            }
        }
        found.then_some(rank)
    }

    /// The deployable table + upkeep globals. Refuses sets the sim's
    /// fixed capacities can't hold, including a build table whose cost
    /// items outgrow the hearth stock rows.
    pub fn bake_deployables(&self) -> Result<DeployContent, String> {
        if self.deployables.len() > MAX_DEPLOY_DEFS {
            return Err(format!(
                "bake: {} deployables exceed the sim's {MAX_DEPLOY_DEFS}-row table",
                self.deployables.len()
            ));
        }
        let mut dc = DeployContent::EMPTY;
        dc.def_count = self.deployables.len() as u16;
        for d in &self.deployables {
            let idx = self.deploy_index(&d.id).expect("own id resolves") as usize;
            let hp = u16::try_from(d.hp)
                .map_err(|_| format!("bake: `{}` hp {} overflows u16", d.id, d.hp))?;
            if hp == 0 {
                return Err(format!("bake: `{}` hp 0 is the inert sentinel", d.id));
            }
            // The repair price, joined from the recipe that makes this
            // deployable's item. Placement charges one crafted item and a
            // repair cannot: a fraction of one item rounds up to the whole
            // thing, so a scratched door would cost a whole door. Pricing
            // against what the door was *made* of gives the piece formula
            // something to divide (`build::repair_units`).
            //
            // `validate.rs` enforces one recipe per output, so this join
            // is 1:1 and never picks. A deployable with no recipe at all
            // bakes `n_costs = 0` and `repair` refuses it as unpriced —
            // it is not required to be craftable, only to be priced
            // before it can be mended.
            let mut costs = [(0u16, 0u16); MAX_DEPLOY_COSTS];
            let mut n_costs = 0u8;
            if let Some(r) = self.recipe_for(&d.id) {
                // Equal widths today; the guard is what keeps them from
                // diverging into an out-of-bounds write.
                if r.inputs.len() > MAX_DEPLOY_COSTS {
                    return Err(format!(
                        "bake: `{}` recipe has more than {MAX_DEPLOY_COSTS} inputs to price a repair against",
                        d.id
                    ));
                }
                for (n, input) in r.inputs.iter().enumerate() {
                    let item = self.item_index(&input.item).ok_or_else(|| {
                        format!("bake: `{}` repair input `{}` missing", d.id, input.item)
                    })?;
                    let count = u16::try_from(input.count).map_err(|_| {
                        format!(
                            "bake: `{}` repair input `{}` count overflows u16",
                            d.id, input.item
                        )
                    })?;
                    costs[n] = (item, count);
                }
                n_costs = r.inputs.len() as u8;
            }
            dc.defs[idx] = DeployDef {
                arch: match d.archetype {
                    DeployArchetype::Bag => ARCH_BAG,
                    DeployArchetype::Hearth => ARCH_HEARTH,
                    DeployArchetype::Box => ARCH_BOX,
                    DeployArchetype::Fire => ARCH_FIRE,
                    DeployArchetype::Furnace => ARCH_FURNACE,
                    DeployArchetype::Workbench => ARCH_WORKBENCH,
                    DeployArchetype::Door => ARCH_DOOR,
                },
                placement: match d.placement {
                    Placement::Ground => PLACE_GROUND,
                    Placement::Foundation => PLACE_FOUNDATION,
                    Placement::Doorway => PLACE_DOORWAY,
                    Placement::Any => PLACE_ANY,
                },
                hp,
                item: self
                    .item_index(&d.id)
                    .ok_or_else(|| format!("bake: `{}` is not an item", d.id))?,
                n_costs,
                costs,
            };
        }
        // Upkeep materials: distinct build-cost items, ascending — the
        // hearth stock rows align to this order.
        let mut mats: Vec<u16> = Vec::new();
        for p in &self.pieces {
            for cost in &p.cost {
                let item = self
                    .item_index(&cost.item)
                    .ok_or_else(|| format!("bake: `{}` cost `{}` missing", p.id, cost.item))?;
                if !mats.contains(&item) {
                    mats.push(item);
                }
            }
        }
        mats.sort_unstable();
        if mats.len() > HEARTH_STOCK_ROWS {
            return Err(format!(
                "bake: {} distinct build-cost items exceed the {HEARTH_STOCK_ROWS} hearth stock rows",
                mats.len()
            ));
        }
        for (n, &item) in mats.iter().enumerate() {
            dc.mats[n] = item;
        }
        dc.mat_count = mats.len() as u8;
        dc.upkeep_pct_per_day =
            u16::try_from(self.balance.globals.upkeep_pct_per_day).map_err(|_| {
                format!(
                    "bake: upkeep_pct_per_day {} overflows u16",
                    self.balance.globals.upkeep_pct_per_day
                )
            })?;
        Ok(dc)
    }

    /// The combat table: every `kind = "melee"` weapon row keyed by the
    /// item it arms, plus `globals.player_hp` — the same number
    /// `anchors()` divides by for the TTK band, so the band the data
    /// declares and the band the sim plays cannot drift apart.
    ///
    /// Both damage columns cross: body `damage` and `structure`. A melee
    /// row with a zero in either is refused rather than baked inert — a
    /// weapon that silently cannot raid is the bug the column exists to
    /// prevent.
    ///
    /// Melee and throwable cross. Bow and firearm rows are still
    /// deliberately dropped rather than half-baked: a projectile the sim
    /// can read but not fire is a number that looks armed and is not
    /// (combat.rs's scope note; `DECISIONS.md` §open, "melee combat v0").
    /// The throwable stopped being one of those the tick `charge.rs`
    /// landed — its `structure` is what the raid ratio divides wall hp by,
    /// so leaving it out of the table was leaving the anchor unexecutable.
    pub fn bake_combat(&self) -> Result<CombatContent, String> {
        if self.items.len() > MAX_ITEM_DEFS {
            return Err(format!(
                "bake: {} items exceed the sim's {MAX_ITEM_DEFS}-def table",
                self.items.len()
            ));
        }
        let mut cc = CombatContent::EMPTY;
        cc.player_hp = u16::try_from(self.balance.globals.player_hp).map_err(|_| {
            format!(
                "bake: player_hp {} overflows u16",
                self.balance.globals.player_hp
            )
        })?;
        if cc.player_hp == 0 {
            return Err("bake: player_hp 0 would disarm combat entirely".to_string());
        }
        for w in &self.weapons {
            if w.kind != WeaponKind::Melee && w.kind != WeaponKind::Throwable {
                continue;
            }
            let idx = self
                .item_index(&w.id)
                .ok_or_else(|| format!("bake: weapon `{}` arms no item", w.id))?
                as usize;
            let reach_cm = w
                .range_m
                .checked_mul(100)
                .and_then(|cm| u16::try_from(cm).ok())
                .ok_or_else(|| {
                    format!(
                        "bake: `{}` range {} m overflows the cm reach",
                        w.id, w.range_m
                    )
                })?;
            if reach_cm == 0 {
                return Err(format!("bake: `{}` has no reach", w.id));
            }
            let structure = u16::try_from(w.structure)
                .map_err(|_| format!("bake: `{}` structure {} overflows u16", w.id, w.structure))?;
            if structure == 0 {
                return Err(format!("bake: `{}` deals no structure damage", w.id));
            }
            let damage = u16::try_from(w.damage)
                .map_err(|_| format!("bake: `{}` damage {} overflows u16", w.id, w.damage))?;
            if w.kind == WeaponKind::Throwable {
                // Seconds → ticks, the reach column's treatment: the
                // division lives here so the sim reads an integer it never
                // has to scale. A fuse that rounded to zero would blow the
                // instant it was planted, which is a different verb.
                let fuse_ticks = (w.fuse_s.unwrap_or(0))
                    .checked_mul(TICK_HZ)
                    .and_then(|t| u16::try_from(t).ok())
                    .ok_or_else(|| {
                        format!(
                            "bake: `{}` fuse {} s overflows the wire's tick field",
                            w.id,
                            w.fuse_s.unwrap_or(0)
                        )
                    })?;
                if fuse_ticks == 0 {
                    return Err(format!("bake: throwable `{}` has no fuse", w.id));
                }
                // Metres → cm, the reach column's treatment. Zero is
                // refused here as well as in `validate` because this is
                // the number a falloff will divide by once one exists, and
                // a bake that let a zero through would put that division
                // one call away from a NaN the determinism gates would
                // then have to catch for it.
                let blast_cm = (w.blast_m.unwrap_or(0))
                    .checked_mul(100)
                    .and_then(|cm| u16::try_from(cm).ok())
                    .ok_or_else(|| {
                        format!(
                            "bake: `{}` blast {} m overflows the cm radius",
                            w.id,
                            w.blast_m.unwrap_or(0)
                        )
                    })?;
                if blast_cm == 0 {
                    return Err(format!("bake: throwable `{}` has no blast radius", w.id));
                }
                if cc.throw[idx].structure != 0 {
                    return Err(format!("bake: duplicate throwable row for `{}`", w.id));
                }
                cc.throw[idx] = ThrowDef {
                    damage,
                    structure,
                    fuse_ticks,
                    reach_cm,
                    blast_cm,
                };
                continue;
            }
            if damage == 0 {
                return Err(format!("bake: melee `{}` deals no damage", w.id));
            }
            if cc.melee[idx].damage != 0 {
                return Err(format!("bake: duplicate weapon row for `{}`", w.id));
            }
            cc.melee[idx] = MeleeDef {
                damage,
                structure,
                reach_cm,
            };
        }
        Ok(cc)
    }

    /// The survival table: `[survival]`'s meters and rates, plus every
    /// `content/consumables.toml` row keyed by the item it feeds.
    ///
    /// **Minutes become ticks here and nowhere else.** The sim owns a tick
    /// counter and no clock (wall 1), so a span stated in minutes has to
    /// cross into ticks exactly once, at the boundary — doing it in the sim
    /// would put `TICK_HZ` arithmetic on the hot path and doing it twice
    /// would be two chances to disagree.
    ///
    /// `validate::structural` has already refused a zero meter, a zero
    /// rate, an inverted water/food ordering and a heal with no span; what
    /// this adds is the arithmetic refusal — a span whose tick count
    /// overflows u32, and a consumable that arms no item.
    pub fn bake_survival(&self) -> Result<SurvivalContent, String> {
        if self.items.len() > MAX_ITEM_DEFS {
            return Err(format!(
                "bake: {} items exceed the sim's {MAX_ITEM_DEFS}-def table",
                self.items.len()
            ));
        }
        let s = &self.balance.survival;
        let mut sc = SurvivalContent::EMPTY;
        let u16f = |v: u32, what: &str| {
            u16::try_from(v).map_err(|_| format!("bake: survival {what} {v} overflows u16"))
        };
        sc.max_food = u16f(s.max_food, "max_food")?;
        sc.max_water = u16f(s.max_water, "max_water")?;
        sc.starve_hp_per_min = u16f(s.starve_hp_per_min, "starve_hp_per_min")?;
        sc.dehydrate_hp_per_min = u16f(s.dehydrate_hp_per_min, "dehydrate_hp_per_min")?;
        sc.drink_water = u16f(s.drink_water, "drink_water")?;
        sc.drink_hp_cost = u16f(s.drink_hp_cost, "drink_hp_cost")?;
        let span = |min: u32, what: &str| {
            min.checked_mul(TICKS_PER_MIN)
                .ok_or_else(|| format!("bake: survival {what} {min} min overflows the tick span"))
        };
        sc.food_span_ticks = span(s.food_minutes_to_empty, "food_minutes_to_empty")?;
        sc.water_span_ticks = span(s.water_minutes_to_empty, "water_minutes_to_empty")?;
        if sc.max_food == 0 || sc.max_water == 0 {
            return Err("bake: a zero meter would disarm the survival clock".to_string());
        }
        for con in &self.consumables {
            let idx = self
                .item_index(&con.id)
                .ok_or_else(|| format!("bake: consumable `{}` feeds no item", con.id))?
                as usize;
            if sc.consumable[idx].is_food() {
                return Err(format!("bake: duplicate consumable row for `{}`", con.id));
            }
            let health = u16f(con.health, "health")?;
            let seconds = u16f(con.seconds, "seconds")?;
            if health > 0 && seconds == 0 {
                return Err(format!("bake: consumable `{}` heals over 0 s", con.id));
            }
            // The ramp's span in ticks must fit the sim's u32 too.
            (seconds as u32).checked_mul(TICK_HZ).ok_or_else(|| {
                format!("bake: consumable `{}` span {seconds} s overflows", con.id)
            })?;
            sc.consumable[idx] = ConsumableDef {
                health,
                food: u16f(con.food, "food")?,
                water: u16f(con.water, "water")?,
                seconds,
            };
        }
        Ok(sc)
    }

    /// The backpack despawn ladder: `[backpack]`'s base minutes and the
    /// per-rarity multipliers, resolved against every item's declared
    /// rarity into one lifetime-in-ticks row per item index. The sim then
    /// never sees a rarity, a minute, or a multiplier — only "this item
    /// keeps a bag alive this many ticks", which is the whole reason the
    /// max-over-contents rule is two comparisons and no table lookup
    /// chain (CLAUDE.md wall 7).
    ///
    /// `validate::structural` has already refused a zero base and a
    /// ladder that does not rise strictly with rarity; what this adds is
    /// the arithmetic refusal — a product that overflows the sim's u32
    /// tick field is a content bug, not a saturated bag.
    pub fn bake_backpack(&self) -> Result<BackpackContent, String> {
        if self.items.len() > MAX_ITEM_DEFS {
            return Err(format!(
                "bake: {} items exceed the sim's {MAX_ITEM_DEFS}-def table",
                self.items.len()
            ));
        }
        let bp = &self.balance.backpack;
        let base_ticks = bp
            .despawn_base_min
            .checked_mul(60)
            .and_then(|s| s.checked_mul(TICK_HZ))
            .ok_or_else(|| {
                format!(
                    "bake: backpack base {} min overflows the tick field",
                    bp.despawn_base_min
                )
            })?;
        if base_ticks == 0 {
            return Err("bake: a zero backpack base would disarm the drop".to_string());
        }
        let mults = bp.mults();
        let mut bc = BackpackContent::EMPTY;
        bc.base_ticks = base_ticks;
        for item in &self.items {
            let idx = self.item_index(&item.id).expect("own id") as usize;
            let mult = mults[item.rarity.canon() as usize];
            bc.despawn_ticks[idx] = base_ticks.checked_mul(mult).ok_or_else(|| {
                format!(
                    "bake: `{}` lifetime ({base_ticks} ticks × {mult}) overflows the tick field",
                    item.id
                )
            })?;
        }
        Ok(bc)
    }

    /// The loot tables: `content/loot.toml`'s weighted rows resolved into
    /// item indices, with the weight sum baked once so the sim's roll is a
    /// multiply-shift and a walk rather than a sum-then-walk.
    ///
    /// The container **name** is content and the container **index** is
    /// code (`loot::LOOT_*`), the same split `bake_deployables` makes for
    /// archetypes: content may re-price a barrel, and it may not invent a
    /// container the sim has no verb for. A table naming an unknown
    /// container is therefore a bake refusal, not a silently ignored row —
    /// silently ignoring it is how you ship a crate table that never
    /// spawns and never says so.
    ///
    /// `validate::structural` has already refused a zero-weight row, an
    /// empty table, a bad count range and zero hits; what this adds is the
    /// arithmetic refusal — a weight or count past the sim's `u16` field,
    /// and a weight sum past `u32`.
    pub fn bake_loot(&self) -> Result<LootContent, String> {
        if self.loot_tables.len() > MAX_LOOT_TABLES {
            return Err(format!(
                "bake: {} loot tables exceed the sim's {MAX_LOOT_TABLES}-table store",
                self.loot_tables.len()
            ));
        }
        let mut lc = LootContent::EMPTY;
        for l in &self.loot_tables {
            let which = match l.container.as_str() {
                "barrel" => LOOT_BARREL,
                "crate" => LOOT_CRATE,
                "cache" => LOOT_CACHE,
                other => {
                    return Err(format!(
                        "bake: loot `{}` names container `{other}`, which the sim has no verb for",
                        l.id
                    ))
                }
            };
            if l.entries.len() > MAX_LOOT_ENTRIES {
                return Err(format!(
                    "bake: loot `{}` has {} rows, past the sim's {MAX_LOOT_ENTRIES}",
                    l.id,
                    l.entries.len()
                ));
            }
            let small = |v: u32, what: &str| -> Result<u16, String> {
                u16::try_from(v)
                    .map_err(|_| format!("bake: loot `{}` {what} {v} overflows the sim", l.id))
            };
            // Wall 4: the roll loop is per-tick work driven by a content
            // number, so the number needs a cap and a stated policy.
            // Policy is refusal at boot, `MAX_LOOT_ENTRIES`'s policy — a
            // clamp would pay a table that silently rolls fewer times than
            // it reads as rolling, which is worse than not booting.
            if l.rolls_max as usize > MAX_LOOT_ROLLS {
                return Err(format!(
                    "bake: loot `{}` rolls up to {}, past the sim's {MAX_LOOT_ROLLS} per smash",
                    l.id, l.rolls_max
                ));
            }
            let mut t = LootTableDef::INERT;
            t.rolls_min = small(l.rolls_min, "rolls_min")?;
            t.rolls_max = small(l.rolls_max, "rolls_max")?;
            t.hits = small(l.hits, "hits")?;
            t.len = l.entries.len() as u16;
            for (i, e) in l.entries.iter().enumerate() {
                let item = self
                    .item_index(&e.item)
                    .ok_or_else(|| format!("bake: loot `{}` names unknown `{}`", l.id, e.item))?;
                t.entries[i] = LootEntryDef {
                    item,
                    weight: small(e.weight, "weight")?,
                    count_min: small(e.count_min, "count_min")?,
                    count_max: small(e.count_max, "count_max")?,
                };
                t.total_weight = t
                    .total_weight
                    .checked_add(e.weight)
                    .ok_or_else(|| format!("bake: loot `{}` weight sum overflows", l.id))?;
            }
            lc.tables[which] = t;
        }
        Ok(lc)
    }
}
