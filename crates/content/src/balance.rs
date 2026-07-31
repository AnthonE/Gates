//! The balance anchors (CONTENT.md §4), computed from the data and held
//! inside the bands `content/balance.toml` declares. A `.toml` edit that
//! breaks a band fails `test_content` AND refuses to boot — the band gets
//! re-spoken (DECISIONS.md), never silently drifted.
//!
//! Farm-minutes are the cost currency: raw materials at the declared
//! tier-1 rates, barrel-only drops at their road-minute price, crafted
//! items by recursive recipe expansion. Craft seconds are ignored — farm
//! time dominates at these scales (declared in balance.toml).

use crate::schema::*;
use crate::Content;

/// The computed anchor values, for the boot log and the test output.
#[derive(Debug, Clone, Default)]
pub struct Anchors {
    /// Banded weapons: (item id, body hits to kill, no armor).
    pub ttk: Vec<(String, u32)>,
    pub satchel_minutes: f64,
    pub starter_minutes: f64,
    /// Satchel cost to break one wall over starter cost: wood, stone, metal.
    pub raid_ratio: [f64; 3],
    pub upkeep_daily_minutes: f64,
    pub wood_wall_minutes: f64,
}

/// Body hits to kill: ceil(hp / damage), integer-exact. `reduction_pct`
/// is the armor's cut of every body hit.
fn hits_to_kill(hp: u32, damage: u32, reduction_pct: u32) -> u32 {
    let effective = damage * (100 - reduction_pct);
    let scaled_hp = hp * 100;
    scaled_hp.div_ceil(effective)
}

/// Farm-minutes for `qty` of an item: declared raw rate, component
/// road-price, or recursive recipe expansion — in that order. An item
/// none of them can price is a content bug, loudly.
fn farm_minutes(c: &Content, item: &str, qty: f64, depth: u32) -> Result<f64, String> {
    if depth > 12 {
        return Err(format!(
            "cost expansion too deep at `{item}` — recipe cycle?"
        ));
    }
    let g = &c.balance.globals;
    if let Some(rate) = g.farm_per_min.get(item) {
        return Ok(qty / f64::from(*rate));
    }
    if let Some(minutes) = g.component_minutes.get(item) {
        return Ok(qty * f64::from(*minutes));
    }
    let recipe = c.recipe_for(item).ok_or_else(|| {
        format!("`{item}` is unpriceable: no farm rate, no component price, no recipe")
    })?;
    let crafts = qty / f64::from(recipe.count);
    let mut total = 0.0;
    for input in &recipe.inputs {
        total += farm_minutes(c, &input.item, crafts * f64::from(input.count), depth + 1)?;
    }
    Ok(total)
}

fn in_band(value: u32, band: [u32; 2], what: &str) -> Result<(), String> {
    if value < band[0] || value > band[1] {
        return Err(format!(
            "band break: {what} = {value}, band [{}, {}] (re-speak the band or move the data)",
            band[0], band[1]
        ));
    }
    Ok(())
}

pub fn check(c: &Content) -> Result<Anchors, String> {
    let hp = c.balance.globals.player_hp;
    let bands = &c.balance.bands;
    let mut anchors = Anchors::default();

    // Anchor 2 — TTK per weapon kind, body hits, no armor; then every
    // single armor piece may add at most `armor_extra_hits_max` hits.
    for w in &c.weapons {
        let band = match w.kind {
            WeaponKind::Melee => bands.ttk_melee,
            WeaponKind::Bow => bands.ttk_bow,
            WeaponKind::Firearm => bands.ttk_firearm,
            WeaponKind::Throwable => continue, // structure damage, not TTK
        };
        let base = hits_to_kill(hp, w.damage, 0);
        in_band(base, band, &format!("ttk `{}`", w.id))?;
        for a in &c.armors {
            let with = hits_to_kill(hp, w.damage, a.reduction_pct);
            if with - base > bands.armor_extra_hits_max {
                return Err(format!(
                    "band break: `{}` vs `{}` adds +{} hits, max {}",
                    a.id,
                    w.id,
                    with - base,
                    bands.armor_extra_hits_max
                ));
            }
        }
        anchors.ttk.push((w.id.clone(), base));
    }

    // Anchor 3 — the farm rate: every banded node totals `node_yield`
    // over `node_hits` swings with a tier-1 tool.
    for g in &c.gatherables {
        if !c.balance.banded_nodes.contains(&g.archetype) {
            continue;
        }
        let t1_yield = g
            .yield_per_hit
            .iter()
            .filter(|(tool, _)| c.item(tool).map(|i| i.tier) == Some(1))
            .map(|(_, per_hit)| *per_hit)
            .max()
            .ok_or_else(|| format!("gatherable `{}`: banded but no tier-1 tool row", g.id))?;
        in_band(g.hits, bands.node_hits, &format!("hits `{}`", g.id))?;
        in_band(
            t1_yield * g.hits,
            bands.node_yield,
            &format!("node yield `{}`", g.id),
        )?;
    }

    // "A full wood wall ≈ 7 min of wood at T1 tools" (CONTENT §3).
    let wall_cost = |material: Material| -> Result<(u32, f64), String> {
        let wall = c
            .pieces
            .iter()
            .find(|p| p.shape == Shape::Wall && p.material == material)
            .ok_or_else(|| format!("no {material:?} wall"))?;
        let mut minutes = 0.0;
        for cost in &wall.cost {
            minutes += farm_minutes(c, &cost.item, f64::from(cost.count), 0)?;
        }
        Ok((wall.hp, minutes))
    };
    let (_, wood_minutes) = wall_cost(Material::Wood)?;
    anchors.wood_wall_minutes = wood_minutes;
    let rounded = wood_minutes.round();
    if rounded < f64::from(bands.wood_wall_minutes[0])
        || rounded > f64::from(bands.wood_wall_minutes[1])
    {
        return Err(format!(
            "band break: wood wall = {wood_minutes:.1} farm-min, band {:?}",
            bands.wood_wall_minutes
        ));
    }

    // Anchor 1 — the raid ratio. Satchel chain expanded to farm-minutes;
    // starter base (balance.toml bill) likewise; walls per tier.
    let satchel = c
        .weapons
        .iter()
        .find(|w| w.kind == WeaponKind::Throwable)
        .ok_or("no throwable raid tool in weapons.toml")?;
    anchors.satchel_minutes = farm_minutes(c, &satchel.id, 1.0, 0)?;

    let mut starter = 0.0;
    for pc in &c.balance.starter_base.pieces {
        let piece = c
            .pieces
            .iter()
            .find(|p| p.id == pc.piece)
            .expect("validated");
        for cost in &piece.cost {
            starter += farm_minutes(c, &cost.item, f64::from(cost.count * pc.count), 0)?;
        }
    }
    for it in &c.balance.starter_base.items {
        starter += farm_minutes(c, &it.item, f64::from(it.count), 0)?;
    }
    anchors.starter_minutes = starter;

    for (i, material) in [Material::Wood, Material::Stone, Material::Metal]
        .into_iter()
        .enumerate()
    {
        let (hp, _) = wall_cost(material)?;
        let satchels = hp.div_ceil(satchel.damage);
        anchors.raid_ratio[i] = f64::from(satchels) * anchors.satchel_minutes / starter;
    }
    let [wood, stone, metal] = anchors.raid_ratio;
    let band = bands.raid_ratio_stone_pct;
    if stone * 100.0 < f64::from(band[0]) || stone * 100.0 > f64::from(band[1]) {
        return Err(format!(
            "band break: stone raid ratio {stone:.2}, band [{:.1}, {:.1}]",
            f64::from(band[0]) / 100.0,
            f64::from(band[1]) / 100.0
        ));
    }
    if !(wood < stone && stone < metal) {
        return Err(format!(
            "band break: raid ratio must rise with tier, got {wood:.2} / {stone:.2} / {metal:.2}"
        ));
    }

    // Anchor 3's upkeep face: a solo starter's daily upkeep in farm-min.
    anchors.upkeep_daily_minutes =
        starter * f64::from(c.balance.globals.upkeep_pct_per_day) / 100.0;
    if anchors.upkeep_daily_minutes > f64::from(bands.upkeep_solo_daily_max_min) {
        return Err(format!(
            "band break: daily upkeep {:.1} farm-min, ceiling {}",
            anchors.upkeep_daily_minutes, bands.upkeep_solo_daily_max_min
        ));
    }

    Ok(anchors)
}
