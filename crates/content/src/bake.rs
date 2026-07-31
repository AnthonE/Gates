//! Bake: the validated content set → the pure fixed-capacity tables the
//! sim consumes (`sim_core::gather::GatherContent`). This is the one
//! bridge across CLAUDE.md wall 7 — data crosses, code never does. Item
//! indices are the rank of the item id in sorted order: canonical for a
//! given set, and the set itself is pinned by the content hash.
//!
//! Boot path only: allocation and `String` errors are fine here; nothing
//! in this module runs on the sim thread.

use crate::schema::{Material, NodeArchetype, Shape, Station};
use crate::Content;
use sim_core::build::{
    BuildContent, PieceDef, MAT_METAL, MAT_STONE, MAT_WOOD, SHAPE_DOORWAY, SHAPE_FLOOR,
    SHAPE_FOUNDATION, SHAPE_ROOF, SHAPE_STAIRS, SHAPE_WALL,
};
use sim_core::craft::{CraftContent, RecipeDef, STATION_FURNACE, STATION_NONE, STATION_WORKBENCH1};
use sim_core::gather::{GatherContent, NodeDef, MAX_TOOLS_PER_NODE, NO_ITEM};
use sim_core::limits::{
    MAX_ITEM_DEFS, MAX_PIECE_COSTS, MAX_PIECE_DEFS, MAX_RECIPES, MAX_RECIPE_INPUTS, TICK_HZ,
};

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
                def.costs[n] = (item, count);
            }
            bc.pieces[idx] = def;
        }
        Ok(bc)
    }
}
