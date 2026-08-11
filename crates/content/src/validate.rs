//! Structural validation (CONTENT.md §0): unique well-formed ids, no
//! orphan references anywhere, stations that exist, doors weaker than
//! their walls. Everything here is a boot refusal, not a warning.

use crate::schema::*;
use crate::Content;
use sim_core::limits::INV_SLOTS;
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
    for m in &c.mobs {
        check_id(&m.id, "mob.", "mob")?;
        unique(&m.id)?;
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
        // The finish share is a slice of the node's own payout, so 100
        // means "pays nothing until it falls" and past 100 the per-swing
        // arithmetic would owe negative yield. A one-hit node cannot
        // withhold from itself — its only swing is the finishing one —
        // so a share there is a content mistake wearing a plausible
        // number, not a harmless no-op.
        if g.finish_bonus_pct > 100 {
            return Err(format!(
                "gatherable `{}`: finish_bonus_pct {} is a share of the node's \
                 own payout and cannot exceed 100",
                g.id, g.finish_bonus_pct
            ));
        }
        if g.finish_bonus_pct > 0 && g.hits < 2 {
            return Err(format!(
                "gatherable `{}`: finish_bonus_pct {} on a {}-hit node — the \
                 only swing IS the finish, so nothing is withheld from anyone",
                g.id, g.finish_bonus_pct, g.hits
            ));
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
        if let Some(s) = &g.secondary {
            item_exists(&s.output, &format!("gatherable `{}` secondary", g.id))?;
            if s.per_hit == 0 {
                return Err(format!(
                    "gatherable `{}`: secondary `{}` pays nothing — drop the \
                     row rather than shipping a payout of zero",
                    g.id, s.output
                ));
            }
            // Two payouts of the same item is one payout with the sum
            // written twice, and it would put two `EV_GATHER` lines for one
            // item on the client's toast stack.
            if s.output == g.output {
                return Err(format!(
                    "gatherable `{}`: secondary repeats the primary output `{}`",
                    g.id, g.output
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
    // Repair is priced as a percent of the pro-rata share of the piece's
    // own build cost, so both ends of the range are a live defect rather
    // than a taste: at 0 a wall heals free and no raid can ever land, and
    // above 100 repairing costs strictly more than the damage destroyed —
    // which is a rebuild with extra steps, and makes the verb dead weight.
    // The ceiling is 100 exactly for that reason; loosening past it is an
    // operator act, not a data edit.
    let rp = c.balance.globals.repair_cost_pct;
    if rp == 0 || rp > 100 {
        return Err(format!(
            "globals: repair_cost_pct {rp} must be 1..=100 — 0 heals a base \
             for free, over 100 costs more than rebuilding"
        ));
    }
    let piece_hp = |shape: Shape, material: Material| -> Option<u32> {
        c.pieces
            .iter()
            .find(|p| p.shape == shape && p.material == material)
            .map(|p| p.hp)
    };
    for p in &c.pieces {
        for (lo, hi) in [
            // Twig is on the ladder because it is where every piece
            // starts: a shape with a wood rung and no twig rung under it
            // is a shape nothing can ever place (`build::place` takes
            // twig and nothing else), which is the same hole the two
            // rungs below describe, one step lower.
            (Material::Twig, Material::Wood),
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
                let rounds = w
                    .ammo
                    .as_deref()
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| format!("weapon `{}`: bows need ammo", w.id))?;
                // Every round is an item AND has a ballistic row. The
                // second half is the check the schema move exists for: a
                // bow used to carry its own numbers and could not miss
                // them, and now it names rounds that have to supply them.
                // A bow whose arrow has no `[[ammo]]` row would otherwise
                // fire at zero speed rather than refuse to load.
                for id in rounds {
                    item_exists(id, &format!("weapon `{}` ammo", w.id))?;
                    if !c.ammo.iter().any(|a| &a.id == id) {
                        return Err(format!(
                            "weapon `{}`: round `{id}` has no [[ammo]] row to fly by",
                            w.id
                        ));
                    }
                }
                // Preference order is the whole of the ammo policy until a
                // switch verb exists, so a duplicate is a silently dead
                // entry rather than a harmless one.
                for (i, id) in rounds.iter().enumerate() {
                    if rounds[..i].contains(id) {
                        return Err(format!("weapon `{}`: round `{id}` listed twice", w.id));
                    }
                }
            }
            WeaponKind::Firearm => {
                let rounds = w
                    .ammo
                    .as_deref()
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| format!("weapon `{}`: firearms need ammo", w.id))?;
                for id in rounds {
                    item_exists(id, &format!("weapon `{}` ammo", w.id))?;
                }
            }
            WeaponKind::Melee | WeaponKind::Throwable => {
                if w.ammo.is_some() {
                    return Err(format!(
                        "weapon `{}`: melee/throwable carries no ammo",
                        w.id
                    ));
                }
            }
        }
        // The fuse belongs to exactly one kind, checked both ways for the
        // reason `ballistic` is: a throwable without one is a charge that
        // never blows, and a fuse on a hatchet is a number nothing reads,
        // which is how a content file starts lying about what it arms.
        match w.kind {
            WeaponKind::Throwable => {
                if w.fuse_s.unwrap_or(0) == 0 {
                    return Err(format!(
                        "weapon `{}`: throwables need a nonzero fuse_s",
                        w.id
                    ));
                }
                // A throwable is the raid tool (balance.rs anchor 1) and
                // the raid ratio divides wall hp by this column. Zero here
                // would make that division meaningless *and* plant charges
                // that take nothing off a wall.
                if w.structure == 0 {
                    return Err(format!(
                        "weapon `{}`: the raid tool must deal structure damage",
                        w.id
                    ));
                }
                // A zero radius is not "no splash", it is a division no
                // falloff can do — it will be the divisor once a consumer
                // exists — so it is refused at the door rather than
                // guarded at every future use.
                if w.blast_m.unwrap_or(0) == 0 {
                    return Err(format!(
                        "weapon `{}`: throwables need a nonzero blast_m",
                        w.id
                    ));
                }
            }
            _ => {
                if w.fuse_s.is_some() {
                    return Err(format!("weapon `{}`: only throwables carry a fuse_s", w.id));
                }
                if w.blast_m.is_some() {
                    return Err(format!(
                        "weapon `{}`: only throwables carry a blast_m",
                        w.id
                    ));
                }
            }
        }
    }

    // Ammo: item-backed rounds carrying the ballistics that used to sit on
    // the bow (`reference/PROJECTILES.md` §9.3). The sampler wall — a round
    // too fast for the collision tracer — is deliberately NOT here: it is
    // arithmetic over `ARROW_STEP_MM` and `TICK_HZ`, which are sim-core's
    // constants, so `bake.rs` keeps it where it can read them.
    for a in &c.ammo {
        item_exists(&a.id, "ammo")?;
        if c.ammo.iter().filter(|o| o.id == a.id).count() > 1 {
            return Err(format!("ammo `{}`: listed twice", a.id));
        }
        // Zero speed is an arrow that never leaves the bow, and it is a
        // division below (`range_mm / speed`), so it is refused at the
        // door rather than guarded at the use.
        if a.speed_mps == 0 {
            return Err(format!("ammo `{}`: a round with no muzzle speed", a.id));
        }
        // Zero drop is legal and means a flat round — the schema has no
        // opinion about gravity, only about a round that cannot fly.
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
        // A heal with no span would be an instant heal, and the sim's ramp
        // divides by the span — refuse the row rather than let a division
        // decide (survival.rs).
        if con.health > 0 && con.seconds == 0 {
            return Err(format!(
                "consumable `{}`: heals {} hp over 0 s — health needs a span",
                con.id, con.health
            ));
        }
        // The meters are u16 in the sim and the bake refuses past them;
        // catch it here, where the message can name the file.
        if con.food > u16::MAX as u32 || con.water > u16::MAX as u32 {
            return Err(format!("consumable `{}`: food/water overflows u16", con.id));
        }
    }

    // The survival clock. Every one of these would be a division by zero,
    // a meter that never moves, or a body that cannot be hurt — each of
    // which would make the clock silently inert, which is the failure mode
    // the module's whole point is to avoid.
    {
        let s = &c.balance.survival;
        for (name, v) in [
            ("max_food", s.max_food),
            ("max_water", s.max_water),
            ("food_minutes_to_empty", s.food_minutes_to_empty),
            ("water_minutes_to_empty", s.water_minutes_to_empty),
            ("starve_hp_per_min", s.starve_hp_per_min),
            ("dehydrate_hp_per_min", s.dehydrate_hp_per_min),
        ] {
            if v == 0 {
                return Err(format!("survival `{name}`: must be ≥ 1"));
            }
        }
        if s.max_food > u16::MAX as u32 || s.max_water > u16::MAX as u32 {
            return Err("survival: meter ceiling overflows u16".to_string());
        }
        // DESIGN §2 pairs hunger with thirst and the genre puts thirst
        // first; the data may retune the gap but not invert the shape,
        // because the HUD's row order and the module's doc both read it.
        if s.water_minutes_to_empty >= s.food_minutes_to_empty {
            return Err(format!(
                "survival: water ({} min) must empty before food ({} min)",
                s.water_minutes_to_empty, s.food_minutes_to_empty
            ));
        }
        // A clock that kills a full-hp body faster than it takes to notice
        // is a bug, not a balance choice. One point per minute per meter is
        // the floor the band asserts against in `test_content`.
        let hp = c.balance.globals.player_hp;
        let worst = s.starve_hp_per_min + s.dehydrate_hp_per_min;
        if worst == 0 || hp / worst < 5 {
            return Err(format!(
                "survival: both meters empty kill {hp} hp in under 5 min ({worst} hp/min)"
            ));
        }
        // **A clock must have an answer.** The clock shipped on 2026-08-03
        // over an island that paid no food at all: five consumable rows
        // parsed, validated and hashed into the WAL header, and not one
        // node, recipe or verb that produced a single unit of any of them
        // (the merge-gate judge's ranked gap 1,
        // `findings/archive-prestamp/pass-20260803-041958-02-judge.md`).
        // Every other check in this block refuses a clock that is silently
        // *inert*; this one refuses a clock that is silently
        // **unanswerable**, which costs a player the whole session rather
        // than nothing.
        //
        // A drink that costs more hp than a body has is not a hard choice,
        // it is a suicide button; and one that costs hp while restoring
        // nothing is a tax with no purchase. Both are refused here, where
        // the message can name the file (survival.rs `drink`).
        if s.drink_water == 0 && s.drink_hp_cost > 0 {
            return Err(format!(
                "survival: the drink costs {} hp and restores no water",
                s.drink_hp_cost
            ));
        }
        if s.drink_hp_cost >= hp {
            return Err(format!(
                "survival: one drink costs {} of {hp} hp — the sea is salt, not lethal",
                s.drink_hp_cost
            ));
        }

        // Reachable means a gatherable pays it, **or the drink verb is
        // armed** — the two payout paths the sim has. The drink counts
        // because it is a verb the sim actually runs against the
        // heightfield every shard boots on, which is exactly the property
        // the loot tables lack below. Loot tables deliberately do not
        // count: nine barrel entries are parsed and hashed, and no verb
        // opens a container (that judge's ranked gap 2), so counting them
        // would be exactly the lie this check exists to catch. When the
        // open verb lands, the set widens in that commit.
        let mut gathered_food = false;
        let mut gathered_water = s.drink_water > 0;
        for g in &c.gatherables {
            for id in [Some(&g.output), g.secondary.as_ref().map(|s| &s.output)]
                .into_iter()
                .flatten()
            {
                for con in &c.consumables {
                    if &con.id != id {
                        continue;
                    }
                    gathered_food |= con.food > 0;
                    gathered_water |= con.water > 0;
                }
            }
        }
        if !gathered_food {
            return Err("survival: hunger drains and nothing on the island pays \
                 a consumable with `food` — the clock has no answer"
                .to_string());
        }
        if !gathered_water {
            return Err(
                "survival: thirst drains, `drink_water` is 0 and nothing on the \
                 island pays a consumable with `water` — the clock has no answer"
                    .to_string(),
            );
        }
    }

    // The spawn kit. Every refusal here is the same class: a kit that seats
    // a player holding something the rest of the tables disagree about.
    // An EMPTY kit is not checked and not an error — a naked beach spawn is
    // the game, and it is what content without a `[[spawn_kit]]` gives.
    {
        let kit = &c.balance.spawn_kit;
        if kit.len() > INV_SLOTS {
            return Err(format!(
                "spawn_kit: {} stacks will not fit {INV_SLOTS} inventory slots",
                kit.len()
            ));
        }
        let mut seen_items = BTreeSet::new();
        for e in kit {
            let def = c
                .item(&e.item)
                .ok_or_else(|| format!("spawn_kit: no such item `{}`", e.item))?;
            if e.count == 0 {
                return Err(format!(
                    "spawn_kit `{}`: grants 0, which is a slot that draws empty",
                    e.item
                ));
            }
            if e.count > def.stack {
                return Err(format!(
                    "spawn_kit `{}`: grants {} past its own stack size {}",
                    e.item, e.count, def.stack
                ));
            }
            // One entry per item. Two stacks of the same thing is not
            // wrong so much as a typo that silently halves what the author
            // thought they were granting — `grant_kit` writes slots in
            // order and never merges.
            if !seen_items.insert(&e.item) {
                return Err(format!("spawn_kit: `{}` granted twice", e.item));
            }
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
        // The lock and its placement class are one thing said twice, and
        // the sim indexes on both: `place_deploy` picks the lock branch
        // off the archetype and picks "the address must hold a door" off
        // the placement. A row that said one without the other would be
        // a lock that mints a deploy record on a doorway, or a door-class
        // deployable the lock store never hears about.
        match (d.archetype, d.placement) {
            (DeployArchetype::Lock, Placement::Door) => {}
            (DeployArchetype::Lock, p) => {
                return Err(format!(
                    "deployable `{}`: a lock is placement `door`, not {p:?}",
                    d.id
                ));
            }
            (a, Placement::Door) => {
                return Err(format!(
                    "deployable `{}`: placement `door` is the lock's alone, not {a:?}'s",
                    d.id
                ));
            }
            _ => {}
        }
    }
    // Exactly one lock row, or none. The sim resolves the item to give
    // back when a lock is unbolted by scanning for the archetype
    // (`deploy::lock_row`), so a second row would make that scan pick the
    // first one and hand back the wrong item — silently, and only on the
    // take verb.
    let locks = c
        .deployables
        .iter()
        .filter(|d| d.archetype == DeployArchetype::Lock)
        .count();
    if locks > 1 {
        return Err(format!(
            "deployables: {locks} lock rows, and the sim can only name one"
        ));
    }

    // The decay ladder: one rate per material, each a live percent, and
    // **monotone against toughness** — a tougher grade must not rot
    // faster than a weaker one, or upgrading would cost materials to
    // shorten a base's life. That is the reference's shape (§5's ladder
    // runs 1 h twig → 3 h wood → 5 h stone → 8 h metal) and it is the one
    // property of these four numbers a reader cannot check by eye.
    {
        let d = &c.balance.globals.decay_pct_per_period;
        for m in [
            Material::Twig,
            Material::Wood,
            Material::Stone,
            Material::Metal,
        ] {
            match d.get(&m) {
                None => return Err(format!("balance: no decay rate for {m:?}")),
                Some(0) => {
                    return Err(format!(
                        "balance: {m:?} decays 0% per period, which is a piece                          that never rots — turn upkeep off with                          upkeep_pct_per_day instead of by rounding"
                    ))
                }
                Some(p) if *p > 100 => {
                    return Err(format!("balance: {m:?} decays {p}% per period, over 100"))
                }
                Some(_) => {}
            }
        }
        let (tw, w, st, me) = (
            d[&Material::Twig],
            d[&Material::Wood],
            d[&Material::Stone],
            d[&Material::Metal],
        );
        if !(tw >= w && w >= st && st >= me) {
            return Err(format!(
                "balance: the decay ladder is not monotone (twig {tw}, wood {w},                  stone {st}, metal {me}) — a tougher grade rotting faster makes an                  upgrade cost materials to shorten a base's life"
            ));
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
        // A container nothing can open is a container that never pays.
        if l.hits == 0 {
            return Err(format!("loot `{}`: zero hits would never open", l.id));
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

    // Mobs: every band here is a *reachability* check rather than a taste
    // one — an animal that cannot be killed, cannot be caught, or cannot be
    // left behind is content that reads as a bug in the sim.
    for m in &c.mobs {
        if m.hp == 0 {
            return Err(format!(
                "mob `{}`: zero hp is how the roster says a species is disarmed — \
                 a row that means it should be deleted, not written",
                m.id
            ));
        }
        if m.name.trim().is_empty() {
            return Err(format!("mob `{}`: empty name", m.id));
        }
        if m.walk_pct == 0 || m.walk_pct > 100 || m.flee_pct == 0 || m.flee_pct > 100 {
            return Err(format!(
                "mob `{}`: speeds are 1–100 percent of the player's own",
                m.id
            ));
        }
        // The leash has to be wider than the fright radius, or the animal
        // spends its whole life being turned around at the leash while a
        // player stands inside the radius that started it.
        if m.roam_m <= m.spook_m {
            return Err(format!(
                "mob `{}`: a {}m leash inside a {}m spook radius is a treadmill",
                m.id, m.roam_m, m.spook_m
            ));
        }
        if m.spook_m == 0 || m.flee_seconds == 0 {
            return Err(format!(
                "mob `{}`: an animal that never flees is scenery",
                m.id
            ));
        }
        if m.respawn_seconds == 0 {
            return Err(format!(
                "mob `{}`: a zero respawn hatches the slot on the tick it died",
                m.id
            ));
        }
        // The bite is armed whole or not at all: `attack = 0` is the
        // pacifist row and every other field must be zero with it, while
        // an armed row needs a reach, a cadence and a courage floor —
        // half-armed states are the "reads as a bug in the sim" class
        // this block exists for (a biter with zero reach never lands one;
        // zero seconds is a bite every tick).
        if m.attack == 0 {
            if m.attack_range_m != 0 || m.attack_seconds != 0 || m.brave_pct != 0 {
                return Err(format!(
                    "mob `{}`: attack fields set on a species with zero attack",
                    m.id
                ));
            }
        } else {
            if m.attack_range_m == 0 || m.attack_seconds == 0 {
                return Err(format!(
                    "mob `{}`: an armed bite needs a range and a cadence",
                    m.id
                ));
            }
            if m.brave_pct > 100 {
                return Err(format!(
                    "mob `{}`: brave_pct is a percent of max hp, 0–100",
                    m.id
                ));
            }
            // A bite from further than the spook radius would be an animal
            // attacking players it has not even noticed.
            if m.attack_range_m > m.spook_m {
                return Err(format!(
                    "mob `{}`: a {}m bite outreaches its {}m spook radius",
                    m.id, m.attack_range_m, m.spook_m
                ));
            }
        }
        if m.drops.is_empty() {
            return Err(format!("mob `{}`: killing it pays nothing", m.id));
        }
        for d in &m.drops {
            item_exists(&d.item, &format!("mob `{}` drop", m.id))?;
            if d.count == 0 {
                return Err(format!("mob `{}`: zero count on `{}`", m.id, d.item));
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

    // The oven table (`content/cooking.toml`). Every rule here is a
    // refusal of a *silently inert* oven rather than a taste call: a fire
    // is the one deployable whose whole behaviour is content, so content
    // that disarms it leaves a placeable object with no verb behind it
    // and nothing else in the tree to notice.
    let f = &c.fuel;
    if !c.items.iter().any(|i| i.id == f.item) {
        return Err(format!("fuel: `{}` is not an item", f.item));
    }
    if !c.items.iter().any(|i| i.id == f.byproduct) {
        return Err(format!("fuel: byproduct `{}` is not an item", f.byproduct));
    }
    if f.seconds == 0 {
        return Err("fuel: a unit that burns for 0 s never runs out".to_string());
    }
    if f.byproduct_pct > 100 {
        return Err(format!(
            "fuel: byproduct_pct {} is hundredths of ONE unit per unit burned, so 100 is the \
             ceiling — a fire that pays more than it eats is a wood duplicator",
            f.byproduct_pct
        ));
    }
    if f.item == f.byproduct {
        return Err("fuel: a byproduct that IS the fuel burns forever".to_string());
    }
    // The oven has to be reachable, exactly as the survival clock has to
    // be answerable: a fuel nothing pays would arm the verb against a
    // world that cannot use it. Gather is the payout path the check
    // trusts (see the clock's own note above), and the fuel shipped
    // today — wood — is what every tree pays.
    if !c
        .gatherables
        .iter()
        .any(|g| g.output == f.item || g.secondary.as_ref().is_some_and(|s| s.output == f.item))
    {
        return Err(format!(
            "fuel: `{}` is not paid by any gatherable — an oven nothing can feed",
            f.item
        ));
    }
    // Every station a row names must have a deployable to stand in the
    // world, or the row is a transformation with nowhere to happen — the
    // same silently-inert failure the fuel checks above refuse, one level
    // out. Cheap to check and it is what catches a `recycler` row shipped
    // before the recycler itself.
    for k in &c.cooks {
        let arch = match k.station {
            CookStation::Fire => DeployArchetype::Fire,
            CookStation::Furnace => DeployArchetype::Furnace,
            CookStation::Recycler => DeployArchetype::Recycler,
        };
        if !c.deployables.iter().any(|d| d.archetype == arch) {
            return Err(format!(
                "cook: `{}` runs at {:?}, and no deployable is one",
                k.input, k.station
            ));
        }
    }
    // Rows may share a `(station, input)` — that is how one component
    // recycles into metal AND coin — so what has to be refused is not the
    // second row but a SET that cannot run as one conversion. The sim
    // fires every row over an input together off one slot timer
    // (`oven::sweep`), which imposes exactly two rules:
    //
    // - they must agree about `seconds`, because a slot holds one clock;
    // - they must pay distinct outputs, or which row is "the" payer of an
    //   item becomes an accident of file order — the positional-payload
    //   trap wearing a content hat, which is what the one-row-per-input
    //   rule that stood here used to prevent outright.
    let mut cook_seen: std::collections::BTreeMap<(u32, String), (u32, BTreeSet<String>)> =
        std::collections::BTreeMap::new();
    for k in &c.cooks {
        if !c.items.iter().any(|i| i.id == k.input) {
            return Err(format!("cook: input `{}` is not an item", k.input));
        }
        if !c.items.iter().any(|i| i.id == k.output) {
            return Err(format!("cook: output `{}` is not an item", k.output));
        }
        if k.seconds == 0 {
            return Err(format!("cook: `{}` converts in 0 s", k.input));
        }
        if k.count == 0 {
            return Err(format!(
                "cook: `{}` pays 0 units of `{}` — an inert row",
                k.input, k.output
            ));
        }
        if k.input == k.output {
            return Err(format!("cook: `{}` cooks into itself", k.input));
        }
        // The fuel is not a cook input **at a station that burns**: such
        // an oven consumes it as fuel first, so the row could never fire,
        // and the move verb would be admitting an item for a
        // transformation that does not happen. A recycler burns nothing,
        // so the conflict does not exist there and the rule does not
        // reach — it is scoped rather than global because an over-broad
        // rule with a stale reason is how a comment starts lying.
        if k.input == f.item && k.station != CookStation::Recycler {
            return Err(format!(
                "cook: `{}` is the fuel — it burns, it does not cook",
                k.input
            ));
        }
        let group = cook_seen
            .entry((k.station as u32, k.input.clone()))
            .or_insert_with(|| (k.seconds, BTreeSet::new()));
        if group.0 != k.seconds {
            return Err(format!(
                "cook: rows for `{}` at the same station disagree about seconds ({} vs {}) — \
                 they fire together off one slot timer, so they share one clock",
                k.input, group.0, k.seconds
            ));
        }
        if !group.1.insert(k.output.clone()) {
            return Err(format!(
                "cook: two rows pay `{}` for `{}` at the same station",
                k.output, k.input
            ));
        }
    }

    // The research table (`content/research.toml`, research v0). Same
    // posture as the oven rules above: every one of these refuses a
    // *silently inert* sink rather than a taste call, because a coin with
    // a faucet and no working sink is the exact failure research exists to
    // close.
    if !c.items.iter().any(|i| i.id == c.research_coin.item) {
        return Err(format!(
            "research: coin `{}` is not an item",
            c.research_coin.item
        ));
    }
    if !c.research.is_empty() {
        // A table nobody can stand at teaches nothing. The deployable is
        // what makes the verb reachable, so its absence is a bake error
        // and not a shrug — the cook rows one block up take the same
        // check for the same reason.
        if !c
            .deployables
            .iter()
            .any(|d| d.archetype == DeployArchetype::Research)
        {
            return Err(
                "research: rows exist and no deployable is a research table —                  a sink nobody can reach"
                    .to_string(),
            );
        }
    }
    let mut research_seen = BTreeSet::new();
    for r in &c.research {
        if !c.items.iter().any(|i| i.id == r.item) {
            return Err(format!("research: `{}` is not an item", r.item));
        }
        if !research_seen.insert(r.item.clone()) {
            return Err(format!("research: two rows for `{}`", r.item));
        }
        // The row must unlock something, and exactly one thing. Two
        // recipes for one item would make WHICH one a blueprint teaches an
        // accident of file order — the positional-payload trap, in the one
        // table where the player has paid for the answer.
        let n = c.recipes.iter().filter(|k| k.output == r.item).count();
        if n == 0 {
            return Err(format!(
                "research: `{}` unlocks nothing — no recipe outputs it",
                r.item
            ));
        }
        if n > 1 {
            return Err(format!(
                "research: `{}` is output by {n} recipes, so which one a \
                 blueprint teaches would be file order",
                r.item
            ));
        }
    }
    // Every gate must have a key, and this is the half a content editor
    // actually gets wrong: a recipe marked `blueprint` with no research row
    // is a recipe **nobody can ever craft**, and it fails silently — the
    // catalog lists it, the craft panel offers it, and the refusal says
    // "you have not learned this" forever.
    for k in &c.recipes {
        if k.blueprint && !c.research.iter().any(|r| r.item == k.output) {
            return Err(format!(
                "recipe `{}` is blueprint-gated and no research row teaches \
                 `{}` — nobody could ever craft it",
                k.id, k.output
            ));
        }
    }

    // The backpack despawn ladder: a base that exists, and multipliers
    // that rise strictly with rarity. Without the strict rise a rarer
    // item could make a bag despawn *sooner* than a common one — the
    // exact inversion NETCODE.md §6.4's tier shape exists to prevent.
    let bp = &c.balance.backpack;
    if bp.despawn_base_min == 0 {
        return Err("backpack: despawn_base_min must be ≥ 1 minute".to_string());
    }
    let mults = bp.mults();
    if mults[0] == 0 {
        return Err("backpack: mult_common must be ≥ 1".to_string());
    }
    for w in mults.windows(2) {
        if w[1] <= w[0] {
            return Err(format!(
                "backpack: despawn multipliers must rise strictly with rarity ({:?})",
                mults
            ));
        }
    }

    Ok(())
}
