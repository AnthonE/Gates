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
use sim_core::gather::{HIT_UNIT, SWING_INTERVAL_TICKS};
use sim_core::limits::TICK_HZ;

/// The computed anchor values, for the boot log and the test output.
#[derive(Debug, Clone, Default)]
pub struct Anchors {
    /// Banded weapons: (item id, body hits to kill, no armor).
    pub ttk: Vec<(String, u32)>,
    /// Per `farm_per_min` row: (item id, declared effective units/min,
    /// sim at-node ceiling units/min). The ceiling is the most a body
    /// standing at the node can be paid: the node's invariant payout at
    /// the best tier-≤1 tool, over the fewest swings that exhaust it
    /// (every one marked). Declared ≤ ceiling is enforced; the gap is
    /// the friction the world charges — travel, measured by the server's
    /// farmwalk, plus everything `reference/RIPLIST.md` §0 says we do
    /// not charge yet.
    pub farm_rates: Vec<(String, u32, u32)>,
    pub satchel_minutes: f64,
    /// What the starter base costs to BUILD, twig skeleton included —
    /// which is the number a raid is priced against, because the raider
    /// is destroying everything the builder paid for.
    pub starter_minutes: f64,
    /// Satchel cost to break one wall over starter cost: wood, stone, metal.
    pub raid_ratio: [f64; 3],
    /// Melee swings to break the weakest door with the best melee weapon.
    pub door_breach_swings: u32,
    /// Melee swings to break a wall, best melee weapon: wood, stone, metal.
    pub wall_breach_swings: [u32; 3],
    /// What the starter base costs to KEEP, per day. Deliberately not
    /// `starter_minutes × the rate`: since twig v0 the sweep never charges
    /// a scaffold upkeep (`deploy::upkeep_sweep`), so the twig half of the
    /// build bill has no upkeep face at all and charging the anchor for it
    /// would price a wall nobody pays for.
    pub upkeep_daily_minutes: f64,
    pub wood_wall_minutes: f64,
}

/// Body hits to kill: ceil(hp / damage), integer-exact. `reduction_pct`
/// is the armor's cut of every body hit.
///
/// **Public since armor v0, and only so it can be pinned.** This divides by
/// the exact rational `damage × (100 − pct) / 100`; the sim floors that
/// number once and subtracts it (`sim_core::combat::reduce`), and the two
/// are not the same function. They agree on every (weapon, armor) pair we
/// ship — `crates/content/tests/content.rs` asserts it rather than assuming
/// it — and the day they stop agreeing, the band describes a fight nobody
/// has.
pub fn hits_to_kill(hp: u32, damage: u32, reduction_pct: u32) -> u32 {
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
    //
    // ⚠ **Known-misleading since armor v0 (2026-08-19), deliberately left
    // standing, and it is not a bug that a green run hides — it is the
    // green run that is the problem.** Applying armor changes no content
    // number, so this anchor cannot go red for it; what it says just stops
    // being true. Three ways, and each is one somebody will otherwise
    // rediscover:
    //
    // 1. **Slot-blind.** The loop below runs every armor row against a
    //    *body* hits-to-kill, so `item.armor_burlap_head` — a head piece —
    //    is credited with reducing body hits. The sim happens to agree
    //    today (`combat::worn_pct` sums both slots, because a head band is
    //    not a coverage model — headshot v0 gave a *shot* a head to cross
    //    without giving anything a worn piece to look up), so this is right
    //    by coincidence and wrong the day hit areas land.
    // 2. **A ceiling with no floor.** A worn set adding *zero* hits
    //    satisfies `armor_extra_hits_max` perfectly, so armor could be
    //    entirely decorative with every gate green.
    // 3. **It cannot see a set.** The band is per piece, by its own
    //    comment, and `Player::worn` holds one head plus one body: burlap
    //    head 10 % + roadsign 25 % is **+3 hits** on rock, spear_wood,
    //    revolver and the stone tools, against a band of 2.
    //
    // Fixing (3) reddens the run, which is why it is not fixed here:
    // `armor_extra_hits_max` has to be re-spoken (2 → 3) or the ladder
    // re-priced, and both are operator acts (`DECISIONS.md` §open, "armor
    // reduction v0"; `NOW.md` §0pvp item 4). What DID land is the half
    // that moves no band — `hits_to_kill` and the sim's own reducer are
    // pinned equal for every (weapon, set) pair in
    // `crates/content/tests/content.rs`, because two arithmetics that
    // disagree describe a fight nobody has.
    for w in &c.weapons {
        let band = match w.kind {
            WeaponKind::Melee => bands.ttk_melee,
            WeaponKind::Bow => bands.ttk_bow,
            WeaponKind::Firearm => bands.ttk_firearm,
            WeaponKind::Throwable => continue, // structure damage, not TTK
        };
        let base = hits_to_kill(hp, w.damage, 0);
        in_band(base, band, &format!("ttk `{}`", w.id))?;
        if w.headshot_mult != bands.headshot_mult {
            return Err(format!(
                "band break: headshot mult on `{}` is {}, the band says exactly {}",
                w.id, w.headshot_mult, bands.headshot_mult
            ));
        }
        if w.limb_pct != bands.limb_pct {
            return Err(format!(
                "band break: limb pct on `{}` is {}, the band says exactly {}",
                w.id, w.limb_pct, bands.limb_pct
            ));
        }
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

    // Anchor 3's agreement face — the declared rate against the sim's own
    // arithmetic (the latent defect `reference/BALANCE.md` §4.3 named:
    // nothing compared the two). `farm_per_min` claims an *effective*
    // rate; the ceiling it may never cross is the most the sim can
    // sustain paying a body standing at the node. Since the mark buys
    // speed rather than yield (`NodeDef::weak_pct`), that is the node's
    // whole invariant payout over the fewest swings that can exhaust it
    // — the per-node arithmetic is below. A `secondary` pays too —
    // flat, any hand, no mark — so an item a node pays only that way is
    // still priceable here. Cadence, tick rate and the budget unit are
    // the sim's own constants, imported, so a change there moves this
    // check and not just the game. What the gap between declared and
    // ceiling *means* (travel, and the friction we do not yet charge —
    // `reference/RIPLIST.md` §0) is the operator's knob; the
    // semantics-independent half is that an effective rate can never
    // beat standing at the node.
    for (item, declared) in &c.balance.globals.farm_per_min {
        let mut at_node: u64 = 0;
        let mut payable = false;
        for g in &c.gatherables {
            if g.output == *item {
                let best_per_hit = g
                    .yield_per_hit
                    .iter()
                    .filter(|(tool, _)| {
                        *tool == "hand" || c.item(tool).is_some_and(|i| i.tier <= 1)
                    })
                    .map(|(_, per_hit)| u64::from(*per_hit).min(u64::from(u16::MAX)))
                    .max()
                    .unwrap_or(0);
                if best_per_hit > 0 {
                    payable = true;
                    // The node's payout is invariant under marking
                    // (`NodeDef::weak_pct`) — a marked swing only spends
                    // the budget faster. So the ceiling is the whole
                    // total over the FEWEST swings that can exhaust it,
                    // every one of them marked, rounding up exactly as
                    // the sim's budget arithmetic does. The finish share
                    // moves when yield arrives, never how much, so it
                    // does not enter here.
                    let total = best_per_hit * u64::from(g.hits);
                    let budget = u64::from(g.hits) * u64::from(HIT_UNIT);
                    let want = u64::from(HIT_UNIT) + u64::from(g.weak_spot_bonus_pct);
                    let swings = budget.div_ceil(want);
                    let ceiling = total * u64::from(TICK_HZ) * 60 / (swings * SWING_INTERVAL_TICKS);
                    at_node = at_node.max(ceiling);
                }
            }
            if let Some(sec) = g.secondary.as_ref().filter(|s| s.output == *item) {
                let per_swing = u64::from(sec.per_hit).min(u64::from(u16::MAX));
                if per_swing > 0 {
                    payable = true;
                    let ceiling = per_swing * u64::from(TICK_HZ) * 60 / SWING_INTERVAL_TICKS;
                    at_node = at_node.max(ceiling);
                }
            }
        }
        if !payable {
            return Err(format!(
                "`{item}` has a declared farm rate and no gatherable pays it at \
                 tier ≤ 1 — not as an output (hand or a tier-0/1 tool row) and \
                 not as a secondary — a `farm_per_min` row prices what a node \
                 pays; a barrel-only drop is `component_minutes`"
            ));
        }
        if u64::from(*declared) > at_node {
            return Err(format!(
                "farm rate break: `{item}` declares {declared}/min effective; the sim \
                 pays at most {at_node}/min standing at the node (weak mark included, \
                 at the {SWING_INTERVAL_TICKS}-tick cadence) — an effective \
                 travel-included rate that beats standing at the node prices \
                 walking as a bonus"
            ));
        }
        anchors
            .farm_rates
            .push((item.clone(), *declared, at_node as u32));
    }

    // "A full wood wall ≈ 4 min of wood at T1 tools" (CONTENT §4 anchor 3).
    // It read 7 until 2026-08-10, when the cost column was taken from the
    // reference and the band was re-spoken under `BALANCE.md` §7.
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
    // The graded half of the same bill — what upkeep is actually charged
    // on, see `Anchors::upkeep_daily_minutes`.
    let mut starter_upkept = 0.0;
    for pc in &c.balance.starter_base.pieces {
        let piece = c
            .pieces
            .iter()
            .find(|p| p.id == pc.piece)
            .expect("validated");
        for cost in &piece.cost {
            let m = farm_minutes(c, &cost.item, f64::from(cost.count * pc.count), 0)?;
            starter += m;
            if piece.material != Material::Twig {
                starter_upkept += m;
            }
        }
        // **And the twig under it.** Since twig v0 a piece cannot be
        // placed at its finished grade — it goes down as twig and the
        // hammer pays the grade on top (`reference/BUILDING.md` §7b.4) —
        // so the bill for a stone wall is the twig wall plus the stone
        // wall, and an anchor that priced only the second would understate
        // every base on the shard. Read off the table rather than added to
        // the bill by hand, so the day a shape's twig rung is re-priced
        // this moves with it and nobody has to remember.
        if piece.material != Material::Twig {
            let twig = c
                .pieces
                .iter()
                .find(|p| p.shape == piece.shape && p.material == Material::Twig)
                .ok_or_else(|| {
                    format!(
                        "starter base names `{}`, whose shape has no twig rung — \
                         nothing could ever place it",
                        pc.piece
                    )
                })?;
            for cost in &twig.cost {
                starter += farm_minutes(c, &cost.item, f64::from(cost.count * pc.count), 0)?;
            }
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
        let satchels = hp.div_ceil(satchel.structure);
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

    // Anchor 1's melee face — the raid lane by hand (`content/weapons.toml`
    // `structure`). Two laws first, both pure ordering, no number to speak:
    // a weapon is never better against a wall than against a person, and
    // the throwable raid tool out-damages every melee weapon on structure.
    let mut best_melee: Option<&Weapon> = None;
    for w in &c.weapons {
        if w.structure > w.damage {
            return Err(format!(
                "band break: `{}` structure {} exceeds its own body damage {} — a weapon may not be better against a wall than a person",
                w.id, w.structure, w.damage
            ));
        }
        if w.kind != WeaponKind::Melee {
            continue;
        }
        if w.structure >= satchel.structure {
            return Err(format!(
                "band break: melee `{}` structure {} reaches the raid tool's {} — the raid tool stays the raid tool",
                w.id, w.structure, satchel.structure
            ));
        }
        if best_melee.is_none_or(|b| w.structure > b.structure) {
            best_melee = Some(w);
        }
    }
    let best = best_melee.ok_or("no melee weapon in weapons.toml to price a hand raid with")?;
    if best.structure == 0 {
        return Err(format!(
            "band break: best melee `{}` deals no structure damage — nothing can be breached by hand",
            best.id
        ));
    }

    // The door is the breach point: band the weakest one, so it stays
    // openable by hand and never becomes a formality.
    let weakest_door = c
        .deployables
        .iter()
        .filter(|d| d.archetype == DeployArchetype::Door)
        .min_by_key(|d| d.hp)
        .ok_or("no door in deployables.toml to price a hand raid against")?;
    anchors.door_breach_swings = weakest_door.hp.div_ceil(best.structure);
    in_band(
        anchors.door_breach_swings,
        bands.door_breach_swings,
        &format!("door breach swings `{}` by `{}`", weakest_door.id, best.id),
    )?;

    // Walls are what the satchel is for: every material sits above the
    // floor, and the ladder rises.
    for (i, material) in [Material::Wood, Material::Stone, Material::Metal]
        .into_iter()
        .enumerate()
    {
        let (hp, _) = wall_cost(material)?;
        let swings = hp.div_ceil(best.structure);
        if swings < bands.wall_breach_swings_min {
            return Err(format!(
                "band break: {material:?} wall breaches in {swings} melee swings by `{}`, floor {}",
                best.id, bands.wall_breach_swings_min
            ));
        }
        anchors.wall_breach_swings[i] = swings;
    }
    let [w_sw, s_sw, m_sw] = anchors.wall_breach_swings;
    if !(w_sw < s_sw && s_sw < m_sw) {
        return Err(format!(
            "band break: melee wall breach must rise with tier, got {w_sw} / {s_sw} / {m_sw}"
        ));
    }
    if anchors.door_breach_swings >= w_sw {
        return Err(format!(
            "band break: the door ({} swings) is no weaker than the wood wall ({w_sw}) — the breach point must be the door",
            anchors.door_breach_swings
        ));
    }

    // Anchor 3's upkeep face: a solo starter's daily upkeep in farm-min,
    // charged on the graded pieces only (twig pays none).
    for it in &c.balance.starter_base.items {
        starter_upkept += farm_minutes(c, &it.item, f64::from(it.count), 0)?;
    }
    anchors.upkeep_daily_minutes =
        starter_upkept * f64::from(c.balance.globals.upkeep_pct_per_day) / 100.0;
    if anchors.upkeep_daily_minutes > f64::from(bands.upkeep_solo_daily_max_min) {
        return Err(format!(
            "band break: daily upkeep {:.1} farm-min, ceiling {}",
            anchors.upkeep_daily_minutes, bands.upkeep_solo_daily_max_min
        ));
    }

    Ok(anchors)
}
