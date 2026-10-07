//! The playbook's planner: what the next milestone's shortfalls come down
//! to in things nobody crafts here.
//!
//! The base's survey says what the pack is short of for the next part of
//! the base (`build::Survey::needs`): fifty low grade fuel, say, or a
//! hundred metal fragments. A player reads the crafting menu one level
//! down: fuel is fat and cloth, fragments are ore once a furnace stands
//! and loot until then. [`raw_needs`] walks the recipe table the server
//! sent the same way, at the stations this body has, to the raw materials
//! the mind can send it for (gather, hunt, loot), net of what the pack
//! already holds. Fixed arrays, no allocation; a recipe's own inputs are
//! followed a few levels deep at most.

use crate::agent::build::{recipe_for, Stations};
use client_core::core::ClientCore;
use sim_core::limits::INV_SLOTS;

/// Raw materials one plan reports.
pub const RAW_ROWS: usize = 4;
/// How many crafting levels below a shortfall are followed.
pub const PLAN_DEPTH: u8 = 3;
/// Distinct items one walk keeps track of.
const ROWS: usize = 12;

/// Items and units, in fixed storage.
#[derive(Clone, Copy, Debug)]
struct Rows {
    rows: [(u16, u32); ROWS],
    n: usize,
}

impl Rows {
    const EMPTY: Self = Self {
        rows: [(0, 0); ROWS],
        n: 0,
    };

    fn add(&mut self, item: u16, units: u32) {
        if let Some(r) = self.rows[..self.n].iter_mut().find(|r| r.0 == item) {
            r.1 = r.1.saturating_add(units);
        } else if self.n < ROWS {
            self.rows[self.n] = (item, units);
            self.n += 1;
        }
    }

    /// Take up to `units` of `item` from what is left of it here.
    fn take(&mut self, item: u16, units: u32) -> u32 {
        self.rows[..self.n]
            .iter_mut()
            .find(|r| r.0 == item)
            .map_or(0, |r| {
                let took = r.1.min(units);
                r.1 -= took;
                took
            })
    }
}

/// Units of an item in the pack and belt.
fn carried(core: &ClientCore, item: u16) -> u32 {
    core.inv[..INV_SLOTS]
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| u32::from(s.count))
        .sum()
}

/// What `needs` (items and units still missing) come down to: each one
/// with a recipe this body can use (no station, or one it has standing)
/// is replaced by that recipe's inputs for enough crafts, less what the
/// pack holds of them, down to [`PLAN_DEPTH`] levels; what has no recipe
/// here is raw, and is what the mind must gather, hunt or loot. Writes
/// the largest shortfalls first into `out`; the rows filled.
pub fn raw_needs(
    core: &ClientCore,
    has: &Stations,
    needs: &[(u16, u32)],
    out: &mut [(u16, u32); RAW_ROWS],
) -> usize {
    let mut raw = Rows::EMPTY;
    // What the pack holds of each item the walk meets, spent as it goes so
    // one stack is not counted toward two inputs.
    let mut pool = Rows::EMPTY;
    for &(item, units) in needs {
        // A shortfall is net of the pack already; its inputs are not.
        expand(
            core, has, item, units, PLAN_DEPTH, &mut raw, &mut pool, false,
        );
    }
    let mut n = 0;
    while n < RAW_ROWS {
        let Some(best) = raw.rows[..raw.n]
            .iter_mut()
            .filter(|r| r.1 > 0)
            .max_by_key(|r| r.1)
        else {
            break;
        };
        out[n] = *best;
        best.1 = 0;
        n += 1;
    }
    n
}

#[allow(clippy::too_many_arguments)]
fn expand(
    core: &ClientCore,
    has: &Stations,
    item: u16,
    units: u32,
    depth: u8,
    raw: &mut Rows,
    pool: &mut Rows,
    net: bool,
) {
    let mut units = units;
    if net {
        if pool.rows[..pool.n].iter().all(|r| r.0 != item) {
            pool.add(item, carried(core, item));
        }
        units -= pool.take(item, units);
    }
    if units == 0 {
        return;
    }
    let recipe = (depth > 0)
        .then(|| recipe_for(core, item, has))
        .flatten()
        .and_then(|(r, ..)| core.recipes.recipes.get(usize::from(r)).copied());
    let Some(def) = recipe.filter(|d| d.out_count > 0) else {
        raw.add(item, units);
        return;
    };
    let crafts = units.div_ceil(u32::from(def.out_count));
    for &(input, per) in &def.inputs[..usize::from(def.n_inputs).min(def.inputs.len())] {
        expand(
            core,
            has,
            input,
            u32::from(per).saturating_mul(crafts),
            depth - 1,
            raw,
            pool,
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::ItemRow;
    use sim_core::craft::{RecipeDef, STATION_FURNACE, STATION_NONE};
    use sim_core::gather::ItemStack;

    const FAT: u16 = 1;
    const CLOTH: u16 = 2;
    const FUEL: u16 = 3;
    const ORE: u16 = 4;
    const FRAGS: u16 = 5;

    fn core() -> Box<ClientCore> {
        let mut core = Box::new(ClientCore::new(1, 1, 0));
        let names = [
            "Bat",
            "Animal Fat",
            "Cloth",
            "Low Grade Fuel",
            "Metal Ore",
            "Metal Fragments",
        ];
        core.catalog.count = names.len() as u16;
        for (i, name) in names.iter().enumerate() {
            let row = ItemRow {
                stack_max: 1000,
                ..ItemRow::EMPTY
            };
            core.catalog.set(i, name.as_bytes(), row).unwrap();
        }
        let recipe = |output, out_count, station, inputs: &[(u16, u16)]| {
            let mut def = RecipeDef {
                output,
                out_count,
                station,
                ticks: 30,
                n_inputs: inputs.len() as u8,
                ..RecipeDef::INERT
            };
            def.inputs[..inputs.len()].copy_from_slice(inputs);
            def
        };
        core.recipes.recipes[0] = recipe(FUEL, 4, STATION_NONE, &[(FAT, 3), (CLOTH, 1)]);
        core.recipes.recipes[1] = recipe(FRAGS, 1, STATION_FURNACE, &[(ORE, 1)]);
        core.recipes.recipe_count = 2;
        core.recipes_have = 2;
        core
    }

    fn give(core: &mut ClientCore, slot: usize, item: u16, count: u16) {
        core.inv[slot] = ItemStack {
            item,
            count,
            cond: 0,
            skin: 0,
        };
    }

    #[test]
    fn a_shortfall_comes_down_to_what_nobody_crafts_here() {
        let mut core = core();
        let mut out = [(0, 0); RAW_ROWS];
        // Fifty fuel is thirteen crafts: 39 fat and 13 cloth, less the
        // cloth already carried.
        give(&mut core, 0, CLOTH, 5);
        let n = raw_needs(&core, &Stations::NONE, &[(FUEL, 50)], &mut out);
        assert_eq!(&out[..n], &[(FAT, 39), (CLOTH, 8)]);
        // Fragments are loot until a furnace stands, then ore.
        let n = raw_needs(&core, &Stations::NONE, &[(FRAGS, 100)], &mut out);
        assert_eq!(&out[..n], &[(FRAGS, 100)]);
        let furnace = Stations {
            furnace: Some([0.0, 0.0]),
            ..Stations::NONE
        };
        give(&mut core, 1, ORE, 30);
        let n = raw_needs(&core, &furnace, &[(FRAGS, 100)], &mut out);
        assert_eq!(&out[..n], &[(ORE, 70)]);
        // One stack is not spent on two shortfalls.
        let n = raw_needs(&core, &furnace, &[(FRAGS, 20), (FRAGS, 20)], &mut out);
        assert_eq!(&out[..n], &[(ORE, 10)]);
    }
}
