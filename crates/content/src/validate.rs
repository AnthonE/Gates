//! Structural validation (CONTENT.md §0): unique well-formed ids, no
//! orphan references anywhere, stations that exist, doors weaker than
//! their walls. Everything here is a boot refusal, not a warning.

use crate::schema::*;
use crate::Content;
use std::collections::BTreeSet;

fn check_id(id: &str, prefix: &str, what: &str) -> Result<(), String> {
    let rest = id
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{what} `{id}`: id must start with `{prefix}`"))?;
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "{what} `{id}`: ids are lowercase [a-z0-9_] after the prefix"
        ));
    }
    Ok(())
}

pub fn structural(c: &Content) -> Result<(), String> {
    // Ids: well-formed, and unique across the whole set.
    let mut seen = BTreeSet::new();
    let mut unique = |id: &str| -> Result<(), String> {
        if !seen.insert(id.to_string()) {
            return Err(format!("duplicate id `{id}`"));
        }
        Ok(())
    };
    for i in &c.items {
        check_id(&i.id, "item.", "item")?;
        unique(&i.id)?;
        if i.stack == 0 {
            return Err(format!("item `{}`: stack must be ≥ 1", i.id));
        }
        if i.tier > 2 {
            return Err(format!("item `{}`: tier is 0–2 (CONTENT §1)", i.id));
        }
        if i.name.trim().is_empty() {
            return Err(format!("item `{}`: empty name", i.id));
        }
    }
    for g in &c.gatherables {
        check_id(&g.id, "gather.", "gatherable")?;
        unique(&g.id)?;
    }
    for r in &c.recipes {
        check_id(&r.id, "recipe.", "recipe")?;
        unique(&r.id)?;
    }
    for p in &c.pieces {
        check_id(&p.id, "build.", "building piece")?;
        unique(&p.id)?;
    }
    for l in &c.loot_tables {
        check_id(&l.id, "loot.", "loot table")?;
        unique(&l.id)?;
    }
    for s in &c.skins {
        check_id(&s.id, "skin.", "skin")?;
        unique(&s.id)?;
    }

    let item_exists = |id: &str, what: &str| -> Result<(), String> {
        if c.item(id).is_none() {
            return Err(format!("{what}: `{id}` is not an item"));
        }
        Ok(())
    };

    // Gatherables: outputs and tools exist; tools are held items.
    for g in &c.gatherables {
        item_exists(&g.output, &format!("gatherable `{}` output", g.id))?;
        if g.hits == 0 {
            return Err(format!("gatherable `{}`: hits must be ≥ 1", g.id));
        }
        if g.yield_per_hit.is_empty() {
            return Err(format!("gatherable `{}`: empty yield table", g.id));
        }
        for (tool, per_hit) in &g.yield_per_hit {
            if *per_hit == 0 {
                return Err(format!("gatherable `{}`: zero yield for `{tool}`", g.id));
            }
            if tool == "hand" {
                continue;
            }
            item_exists(tool, &format!("gatherable `{}` tool", g.id))?;
            if c.item(tool).map(|i| i.slot) != Some(EquipSlot::Hand) {
                return Err(format!(
                    "gatherable `{}`: tool `{tool}` is not a hand item",
                    g.id
                ));
            }
        }
    }

    // Recipes: outputs/inputs exist, one recipe per output (the cost
    // expansion in balance.rs needs a single well-defined chain), and
    // crafting stations exist as deployables.
    let mut outputs = BTreeSet::new();
    for r in &c.recipes {
        item_exists(&r.output, &format!("recipe `{}` output", r.id))?;
        if !outputs.insert(r.output.clone()) {
            return Err(format!(
                "recipe `{}`: `{}` already has a recipe — one per output",
                r.id, r.output
            ));
        }
        if r.count == 0 {
            return Err(format!("recipe `{}`: count must be ≥ 1", r.id));
        }
        if r.inputs.is_empty() {
            return Err(format!("recipe `{}`: no inputs", r.id));
        }
        for input in &r.inputs {
            item_exists(&input.item, &format!("recipe `{}` input", r.id))?;
            if input.count == 0 {
                return Err(format!("recipe `{}`: zero-count input", r.id));
            }
            if input.item == r.output {
                return Err(format!("recipe `{}` consumes its own output", r.id));
            }
        }
        let station_item = match r.station {
            Station::None => None,
            Station::Workbench1 => Some("item.workbench1"),
            Station::Furnace => Some("item.furnace"),
        };
        if let Some(id) = station_item {
            if !c.deployables.iter().any(|d| d.id == id) {
                return Err(format!(
                    "recipe `{}`: station `{id}` is not a deployable",
                    r.id
                ));
            }
        }
    }

    // Building pieces: one row per shape × material, costs exist, hp
    // strictly rising with material tier per shape.
    let mut combos = BTreeSet::new();
    for p in &c.pieces {
        if !combos.insert((p.shape as u8, p.material as u8)) {
            return Err(format!("building `{}`: duplicate shape × material", p.id));
        }
        if p.hp == 0 {
            return Err(format!("building `{}`: hp must be ≥ 1", p.id));
        }
        if p.cost.is_empty() {
            return Err(format!("building `{}`: no cost", p.id));
        }
        for cost in &p.cost {
            item_exists(&cost.item, &format!("building `{}` cost", p.id))?;
            if cost.count == 0 {
                return Err(format!("building `{}`: zero-count cost", p.id));
            }
        }
    }
    let piece_hp = |shape: Shape, material: Material| -> Option<u32> {
        c.pieces
            .iter()
            .find(|p| p.shape == shape && p.material == material)
            .map(|p| p.hp)
    };
    for p in &c.pieces {
        for (lo, hi) in [
            (Material::Wood, Material::Stone),
            (Material::Stone, Material::Metal),
        ] {
            if let (Some(a), Some(b)) = (piece_hp(p.shape, lo), piece_hp(p.shape, hi)) {
                if a >= b {
                    return Err(format!(
                        "building `{}`: {:?} hp must rise {:?} < {:?}",
                        p.id, p.shape, lo, hi
                    ));
                }
            }
            // The ladder has no holes: upgrade-in-place climbs shape by
            // shape (sim-core build.rs), so a stone rung with no wood one
            // under it is a piece nothing can ever be upgraded into.
            if piece_hp(p.shape, hi).is_some() && piece_hp(p.shape, lo).is_none() {
                return Err(format!(
                    "building `{}`: {:?} has a {:?} rung but no {:?} — the upgrade ladder must be whole",
                    p.id, p.shape, hi, lo
                ));
            }
        }
    }

    // Weapons: item-backed hand items; projectile kinds carry ballistic +
    // existing ammo; melee carries neither.
    for w in &c.weapons {
        item_exists(&w.id, "weapon")?;
        if c.item(&w.id).map(|i| i.slot) != Some(EquipSlot::Hand) {
            return Err(format!("weapon `{}`: not a hand item", w.id));
        }
        if w.damage == 0 || w.headshot_mult == 0 || w.rate_per_min == 0 {
            return Err(format!("weapon `{}`: zero damage/mult/rate", w.id));
        }
        match w.kind {
            WeaponKind::Bow => {
                if w.ballistic.is_none() {
                    return Err(format!("weapon `{}`: bows are ballistic", w.id));
                }
                let ammo = w
                    .ammo
                    .as_deref()
                    .ok_or_else(|| format!("weapon `{}`: bows need ammo", w.id))?;
                item_exists(ammo, &format!("weapon `{}` ammo", w.id))?;
            }
            WeaponKind::Firearm => {
                let ammo = w
                    .ammo
                    .as_deref()
                    .ok_or_else(|| format!("weapon `{}`: firearms need ammo", w.id))?;
                item_exists(ammo, &format!("weapon `{}` ammo", w.id))?;
            }
            WeaponKind::Melee | WeaponKind::Throwable => {
                if w.ammo.is_some() || w.ballistic.is_some() {
                    return Err(format!(
                        "weapon `{}`: melee/throwable carries no ammo or ballistic",
                        w.id
                    ));
                }
            }
        }
    }

    // Armor: item-backed, worn in the slot the item declares, sane range.
    for a in &c.armors {
        item_exists(&a.id, "armor")?;
        let item_slot = c.item(&a.id).map(|i| i.slot);
        let matches = matches!(
            (a.slot, item_slot),
            (ArmorSlot::Head, Some(EquipSlot::Head)) | (ArmorSlot::Body, Some(EquipSlot::Body))
        );
        if !matches {
            return Err(format!("armor `{}`: item slot disagrees", a.id));
        }
        if a.reduction_pct > 90 {
            return Err(format!("armor `{}`: reduction over 90%", a.id));
        }
    }

    // Consumables and deployables: item-backed; doors declare material
    // and stay strictly weaker than that material's wall (the intended
    // breach point); non-doors declare none.
    for con in &c.consumables {
        item_exists(&con.id, "consumable")?;
        if con.health == 0 && con.food == 0 && con.water == 0 {
            return Err(format!("consumable `{}`: does nothing", con.id));
        }
    }
    for d in &c.deployables {
        item_exists(&d.id, "deployable")?;
        if c.item(&d.id).map(|i| i.slot) != Some(EquipSlot::Hand) {
            return Err(format!("deployable `{}`: not a hand item", d.id));
        }
        if d.hp == 0 {
            return Err(format!("deployable `{}`: hp must be ≥ 1", d.id));
        }
        match (d.archetype, d.material) {
            (DeployArchetype::Door, None) => {
                return Err(format!("deployable `{}`: doors declare material", d.id));
            }
            (DeployArchetype::Door, Some(m)) => {
                let wall = piece_hp(Shape::Wall, m)
                    .ok_or_else(|| format!("deployable `{}`: no {m:?} wall exists", d.id))?;
                if d.hp >= wall {
                    return Err(format!(
                        "deployable `{}`: door hp {} must stay under the {m:?} wall's {wall}",
                        d.id, d.hp
                    ));
                }
            }
            (_, Some(_)) => {
                return Err(format!(
                    "deployable `{}`: only doors declare material",
                    d.id
                ));
            }
            (_, None) => {}
        }
    }

    // Loot: every entry exists, weights and count ranges sane.
    let mut containers = BTreeSet::new();
    for l in &c.loot_tables {
        if !containers.insert(l.container.clone()) {
            return Err(format!("loot `{}`: duplicate container", l.id));
        }
        if l.rolls_min == 0 || l.rolls_min > l.rolls_max {
            return Err(format!("loot `{}`: bad rolls range", l.id));
        }
        if l.entries.is_empty() {
            return Err(format!("loot `{}`: empty table", l.id));
        }
        for e in &l.entries {
            item_exists(&e.item, &format!("loot `{}` entry", l.id))?;
            if e.weight == 0 {
                return Err(format!("loot `{}`: zero weight on `{}`", l.id, e.item));
            }
            if e.count_min == 0 || e.count_min > e.count_max {
                return Err(format!("loot `{}`: bad count range on `{}`", l.id, e.item));
            }
        }
    }

    // Skins: appearance rows only (the schema already can't carry stats);
    // covered items must exist, prices are nonzero bare-ticker amounts.
    for s in &c.skins {
        item_exists(&s.covers, &format!("skin `{}` covers", s.id))?;
        if s.price == 0 {
            return Err(format!("skin `{}`: zero price", s.id));
        }
    }

    // Balance globals point at real items.
    for id in c
        .balance
        .globals
        .farm_per_min
        .keys()
        .chain(c.balance.globals.component_minutes.keys())
    {
        item_exists(id, "balance globals")?;
    }
    for (id, rate) in &c.balance.globals.farm_per_min {
        if *rate == 0 {
            return Err(format!("balance: zero farm rate for `{id}`"));
        }
    }
    for pc in &c.balance.starter_base.pieces {
        if !c.pieces.iter().any(|p| p.id == pc.piece) {
            return Err(format!("starter base: `{}` is not a piece", pc.piece));
        }
        if pc.count == 0 {
            return Err(format!("starter base: zero count for `{}`", pc.piece));
        }
    }
    for it in &c.balance.starter_base.items {
        item_exists(&it.item, "starter base")?;
        if it.count == 0 {
            return Err(format!("starter base: zero count for `{}`", it.item));
        }
    }
    for arch in &c.balance.banded_nodes {
        if !c.gatherables.iter().any(|g| g.archetype == *arch) {
            return Err(format!("balance: banded node {arch:?} has no gatherable"));
        }
    }

    Ok(())
}
