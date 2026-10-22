//! Canonical serialization → xxh3 (CONTENT.md §0). Stable against TOML
//! formatting, comments, and row order (rows hash sorted by id); moved by
//! any actual value. Strings hash length-prefixed so adjacent fields
//! can't alias. All content numbers are integers, so the digest is exact.

use crate::schema::*;
use crate::Content;
use xxhash_rust::xxh3::Xxh3;

struct Canon(Xxh3);

impl Canon {
    fn s(&mut self, v: &str) {
        self.0.update(&(v.len() as u32).to_le_bytes());
        self.0.update(v.as_bytes());
    }
    fn u(&mut self, v: u32) {
        self.0.update(&v.to_le_bytes());
    }
    fn stacks(&mut self, v: &[Stack]) {
        self.u(v.len() as u32);
        for s in v {
            self.s(&s.item);
            self.u(s.count);
        }
    }
}

fn sorted<T, F: Fn(&T) -> &str>(rows: &[T], id: F) -> Vec<&T> {
    let mut v: Vec<&T> = rows.iter().collect();
    v.sort_by_key(|r| id(r).to_string());
    v
}

pub fn hash(c: &Content) -> u64 {
    let mut h = Canon(Xxh3::new());

    h.s("items");
    for i in sorted(&c.items, |i| &i.id) {
        h.s(&i.id);
        h.s(&i.name);
        h.u(i.stack);
        h.u(i.tier);
        h.u(i.rarity.canon());
        h.u(i.slot as u32);
        h.u(i.class.code() as u32);
        // The condition ceiling reaches the sim (`bake_gather`'s
        // `cond_max`), so it walks — a value the sim reads and the digest
        // cannot see lets two contents whose tools die at different rates
        // canonicalise identically, and a WAL then replays under a wear
        // table it was not played under (item durability v0).
        h.u(i.condition_max);
        // The burn rate walks for `condition_max`'s reason exactly: it
        // reaches the sim (`bake_gather`'s `light_burn`), it decides both
        // whether an item is a light and how fast it eats itself, and two
        // contents that disagree on it would run the same WAL to different
        // inventories (torch fuel v0).
        h.u(i.light_burn);
        // What a data `cond` carries sets the save walls' ceiling.
        h.u(match i.cond_carries {
            None => 0,
            Some(crate::schema::CondCarries::Genes) => 1,
            Some(crate::schema::CondCarries::Target) => 2,
        });
    }

    h.s("gatherables");
    for g in sorted(&c.gatherables, |g| &g.id) {
        h.s(&g.id);
        h.u(g.archetype as u32);
        h.s(&g.output);
        h.u(g.hits);
        h.u(g.weak_spot_bonus_pct);
        h.u(g.finish_bonus_pct);
        h.u(g.yield_per_hit.len() as u32);
        for (tool, per_hit) in &g.yield_per_hit {
            h.s(tool);
            h.u(*per_hit);
        }
        // The wear table walks for `condition_max`'s reason: it reaches
        // `NodeDef::wear` and prices every landed hit.
        h.u(g.condition_loss.len() as u32);
        for (tool, loss) in &g.condition_loss {
            h.s(tool);
            h.u(*loss);
        }
        // The side payout is hashed like everything else the sim reads. A
        // value that reaches the sim and not the hash lets two contents
        // that play differently canonicalise identically, so a replay is
        // handed a WAL header claiming a match it does not have (the same
        // defect `[backpack]`'s ladder carried until 2026-08-03).
        match &g.secondary {
            None => h.u(0),
            Some(s) => {
                h.u(1);
                h.s(&s.output);
                h.u(s.per_hit);
            }
        }
    }

    h.s("recipes");
    for r in sorted(&c.recipes, |r| &r.id) {
        h.s(&r.id);
        h.s(&r.output);
        h.u(r.count);
        h.u(r.station as u32);
        h.u(r.seconds);
        h.u(r.blueprint as u32);
        h.stacks(&r.inputs);
        h.s(r.unlock.as_deref().unwrap_or(""));
    }

    h.s("building");
    for p in sorted(&c.pieces, |p| &p.id) {
        h.s(&p.id);
        h.u(p.shape as u32);
        h.u(p.material as u32);
        h.u(p.hp);
        h.stacks(&p.cost);
    }

    h.s("weapons");
    for w in sorted(&c.weapons, |w| &w.id) {
        h.s(&w.id);
        h.u(w.kind as u32);
        h.u(w.damage);
        h.u(w.structure);
        h.u(w.headshot_pct);
        h.u(w.limb_pct);
        h.u(w.rate_per_min);
        h.u(w.range_m);
        h.u(w.reach_cm);
        h.u(w.lit_damage.unwrap_or(0));
        h.u(w.condition_loss.unwrap_or(0));
        // The round list walks in **declared order, not sorted**, and that
        // is deliberate: order is the ammo policy (the sim spends the first
        // round the shooter carries), so two bows differing only in which
        // arrow they prefer play differently and must digest differently.
        // Sorting here would be the same defect the `[survival]` comment
        // above describes, one level down.
        match w.ammo.as_deref() {
            None => h.u(0),
            Some(rounds) => {
                h.u(1);
                h.u(rounds.len() as u32);
                for id in rounds {
                    h.s(id);
                }
            }
        }
        // The fuse walks here for the reason the `[survival]` and
        // `[backpack]` comments above give: a field that reaches the sim
        // and not the digest lets two contents that play differently
        // canonicalise identically. This one reaches `ThrowDef` (bake.rs),
        // so it walks.
        match w.fuse_s {
            None => h.u(0),
            Some(f) => {
                h.u(1);
                h.u(f);
            }
        }
        // The radius walks for the fuse's reason: it reaches `ThrowDef`
        // (bake.rs) and scales both damage columns, so two contents whose
        // satchels clear different holes must not canonicalise the same.
        match w.blast_m {
            None => h.u(0),
            Some(b) => {
                h.u(1);
                h.u(b);
            }
        }
        match w.dud_pct {
            None => h.u(0),
            Some(d) => {
                h.u(1);
                h.u(d);
            }
        }
        // The draw reaches `RangedDef` and decides when a bow may loose.
        match w.draw_ms {
            None => h.u(0),
            Some(d) => {
                h.u(1);
                h.u(d);
            }
        }
        // The magazine and its reload reach `RangedDef` too.
        h.u(w.magazine.unwrap_or(0));
        h.u(w.reload_ms.unwrap_or(0));
        match w.gun {
            None => h.u(0),
            Some(g) => {
                h.u(1);
                for v in [
                    u32::from(g.auto),
                    g.pellets,
                    g.aim_cdeg,
                    g.hip_cdeg,
                    g.move_cdeg,
                    g.bloom_cdeg,
                    g.bloom_max_cdeg,
                    g.bloom_recover_ms,
                    g.bloom_delay_ms,
                    g.pellet_cdeg,
                    g.falloff_m[0],
                    g.falloff_m[1],
                    g.falloff_pct,
                    g.recoil_pitch_cdeg[0],
                    g.recoil_pitch_cdeg[1],
                    g.recoil_yaw_cdeg[0] as u32,
                    g.recoil_yaw_cdeg[1] as u32,
                    g.ads_recoil_pct,
                ] {
                    h.u(v);
                }
            }
        }
    }

    // The ballistics, on the object they belong to. These reach
    // `AmmoDef` (bake.rs) and decide where every arrow lands, so two
    // contents whose arrows fly differently must not digest the same.
    // Sorted, unlike the round list above: this table is a keyed lookup
    // and its file order means nothing.
    h.s("ammo");
    for a in sorted(&c.ammo, |a| &a.id) {
        h.s(&a.id);
        h.u(a.speed_mps);
        h.u(a.drop_mps2);
        h.u(a.damage_pct);
        h.u(a.fire_s[0]);
        h.u(a.fire_s[1]);
        match a.blast {
            None => h.u(0),
            Some(b) => {
                h.u(1);
                h.u(b.blast_m);
                h.u(b.damage);
                h.u(b.structure);
            }
        }
    }

    h.s("armor");
    for a in sorted(&c.armors, |a| &a.id) {
        h.s(&a.id);
        h.u(a.slot as u32);
        h.u(a.reduction_pct);
        h.u(a.move_penalty_pct);
        h.u(a.cold_pct as u32);
        h.u(a.fel_pct);
        h.u(u32::from(a.full_body));
    }

    h.s("consumables");
    for con in sorted(&c.consumables, |c| &c.id) {
        h.s(&con.id);
        h.u(con.health);
        h.u(con.food);
        h.u(con.water);
        h.u(con.seconds);
        h.u(con.fel as u32);
        h.u(u32::from(con.belt_recovery));
        h.u(u32::from(con.revive));
        match &con.leaves {
            None => h.u(0),
            Some(l) => {
                h.u(1);
                h.s(l);
            }
        }
        h.u(con.leaves_pm);
    }

    h.s("deployables");
    for d in sorted(&c.deployables, |d| &d.id) {
        h.s(&d.id);
        h.u(d.archetype as u32);
        h.u(d.placement as u32);
        h.u(d.matter as u32);
        match d.material {
            None => h.u(0),
            Some(m) => h.u(1 + m as u32),
        }
        h.u(d.hp);
        h.u(u32::from(d.keyed));
        h.u(d.comfort);
        h.u(d.comfort_radius_cm);
    }

    h.s("loot");
    for l in sorted(&c.loot_tables, |l| &l.id) {
        h.s(&l.id);
        h.s(&l.container);
        h.u(l.rolls_min);
        h.u(l.rolls_max);
        h.u(l.hits);
        h.u(l.entries.len() as u32);
        for e in &l.entries {
            h.s(&e.item);
            h.u(e.weight);
            h.u(e.count_min);
            h.u(e.count_max);
        }
        // The guaranteed rows reach the sim through `bake_loot`, so two
        // contents that disagree about one play differently.
        h.u(l.guaranteed.len() as u32);
        for g in &l.guaranteed {
            h.s(&g.item);
            h.u(g.count_min);
            h.u(g.count_max);
        }
    }

    // Animals. Hashed like everything else the sim reads: two content sets
    // whose pigs differ must not canonicalise identically, or a replay is
    // handed a WAL header claiming a match it does not have.
    h.s("mobs");
    for m in sorted(&c.mobs, |m| &m.id) {
        h.s(&m.id);
        h.s(&m.name);
        h.u(m.hp);
        h.u(m.walk_pct);
        h.u(m.flee_pct);
        h.u(m.flee_seconds);
        h.u(m.attack);
        h.u(m.attack_range_m);
        h.u(m.attack_seconds);
        h.u(m.brave_pct);
        h.u(m.fel_bite);
        h.u(m.roam_m);
        h.u(m.spook_m);
        h.u(m.night_spook_m);
        h.u(m.respawn_seconds);
        // `per_km2` and `habitat` are not hashed: they decide how many
        // animals live and where, and no saved record indexes through
        // either — the heli's reason below.
        h.u(m.sight_deg);
        h.u(m.pack_m);
        h.u(m.fire_fear_m);
        // The brain's numbers decide what an animal does on a tick, so a
        // WAL replays under them like the rest of the row.
        h.u(m.pack_biters);
        h.u(m.give_up_tries);
        h.u(m.heal_after_seconds);
        h.u(m.heal_seconds);
        h.u(m.howl_seconds);
        h.u(m.orbit_m);
        h.u(m.sleep_pct);
        h.u(m.hear_m.gun);
        h.u(m.hear_m.bow);
        h.u(m.hear_m.strike);
        h.u(m.hear_m.blast);
        h.stacks(&m.drops);
        // A bred brain decides as much as the numbers above, so it replays
        // under its own text. Absent folds nothing: a set with no genome
        // hashes as it did.
        if let Some(g) = &m.genome {
            h.s("genome");
            h.s(g.trim());
            h.u(m.genome_tier.unwrap_or(crate::bake::GENOME_TIER_DEFAULT));
        }
    }

    // The fel tiers move every row above while they hold, so they are hashed
    // with them: a balance edit, which a save survives (`store::match_content`).
    h.s("fel_tier");
    for t in &c.fel_tiers {
        h.u(t.hp_pct);
        h.u(t.attack_pct);
        h.u(t.notice_pct);
        h.u(t.brave_min_pct);
    }

    // **The heli is not hashed either**, for the skins' reason below: no
    // saved stack or record indexes through it, so tuning it must not
    // refuse a save the way a moved item row has to.

    // **Skins are not hashed.** This digest is what a save file is refused
    // on (`server::store`, `server::worldfile`), because item and recipe
    // rows are indices a saved stack points through. A skin is not: a
    // stack carries its skin's catalog id, and an id the running catalog
    // does not know draws as the plain item. Hashing the rows made a new
    // look, a tint or a name a wipe, for a catalog that is meant to grow.

    // The bands are content: moving one is a visible balance change.
    h.s("balance");
    let g = &c.balance.globals;
    h.u(g.player_hp);
    h.u(g.farm_per_min.len() as u32);
    for (id, rate) in &g.farm_per_min {
        h.s(id);
        h.u(*rate);
    }
    h.u(g.component_minutes.len() as u32);
    for (id, minutes) in &g.component_minutes {
        h.s(id);
        h.u(*minutes);
    }
    h.u(g.upkeep_pct_per_day);
    h.u(g.repair_cost_pct);
    h.u(g.arrow_break_pct);
    h.u(g.arrow_lodge_s);
    if let Some(f) = &g.fist {
        h.s("fist");
        h.u(f.damage);
        h.u(f.range_m);
        h.u(f.self_damage);
    }
    // The decay ladder reaches the sim (`bake_deployables` → `decay_pct`) and
    // was the fourth field caught outside this walk: two contents rotting a
    // stone base in five hours and in fifty canonicalised identically. A
    // `BTreeMap`, so the walk order is the key order and needs no sort.
    h.s("decay");
    h.u(g.decay_pct_per_period.len() as u32);
    for (m, pct) in &g.decay_pct_per_period {
        h.u(*m as u32);
        h.u(*pct);
    }
    // Upkeep v2's three knobs, each read by the sweep. The ladder in file
    // order, which validation holds ascending, so order is meaning.
    h.s("upkeep_v2");
    h.u(g.upkeep_steps.len() as u32);
    for [after, permille] in &g.upkeep_steps {
        h.u(*after);
        h.u(*permille);
    }
    h.u(g.inside_decay_pct.unwrap_or(0));
    h.u(g.grief_protection_h);
    let b = &c.balance.bands;
    for pair in [
        b.ttk_melee,
        b.ttk_light,
        b.ttk_bow,
        b.ttk_firearm,
        b.node_yield,
        b.node_hits,
        b.wood_wall_minutes,
        b.raid_ratio_stone_pct,
        b.door_breach_swings,
    ] {
        h.u(pair[0]);
        h.u(pair[1]);
    }
    h.u(b.headshot_pct[0]);
    h.u(b.headshot_pct[1]);
    h.u(b.limb_pct);
    h.u(b.armor_extra_hits_max);
    h.u(b.wall_breach_swings_min);
    h.u(b.upkeep_solo_daily_max_min);
    h.u(c.balance.banded_nodes.len() as u32);
    for n in &c.balance.banded_nodes {
        h.u(*n as u32);
    }
    h.u(c.balance.starter_base.pieces.len() as u32);
    for pc in &c.balance.starter_base.pieces {
        h.s(&pc.piece);
        h.u(pc.count);
    }
    h.stacks(&c.balance.starter_base.items);

    // The backpack ladder was reaching the sim (it sets every death bag's
    // lifetime) and was NOT reaching this hash — so two contents that
    // despawned bags at different rates canonicalised identically, and a
    // replay could be handed a WAL whose header said the ladder matched
    // when it did not. Hashed now, alongside the clock it sits next to.
    let bp = &c.balance.backpack;
    h.s("backpack");
    h.u(bp.despawn_base_min);
    for m in bp.mults() {
        h.u(m);
    }
    h.stacks(&bp.body_drops);

    // The oven table, whole. Every field here reaches the sim
    // (`bake_cooking`), so every field is here — the `[backpack]` defect
    // above is the reason this paragraph exists rather than the four
    // lines it needs: a value the sim reads and the digest cannot see
    // lets two contents that burn wood at different rates canonicalise
    // identically, and the WAL header then claims a match that is not
    // one. The cook rows sort by input id, which is unique across the
    // table (`validate::structural` refuses two rows for one input at one
    // station, and a row's station is part of what makes it distinct —
    // so the sort key is the pair, spelled as one string).
    h.s("cooking");
    h.s(&c.fuel.item);
    h.u(c.fuel.seconds);
    h.s(&c.fuel.byproduct);
    h.u(c.fuel.byproduct_pct);
    // The farm, whole (every field reaches `bake_farm`), and the crops in
    // file order: a plant stores its crop as an index, so the order is the
    // layout (`layout_hash`) and the rows are state.
    let fm = &c.farm;
    h.s("farm");
    h.s(&fm.water);
    h.s(&fm.emptied);
    h.u(fm.skin_water);
    h.u(fm.planter_water);
    h.u(fm.rain_water);
    for p in fm.stage_pct {
        h.u(p as u32);
    }
    h.u(fm.ripe_seconds);
    h.u(fm.dying_seconds);
    for v in [
        fm.good_gene_pm,
        fm.bad_w_pm,
        fm.g_pct,
        fm.y_pct,
        fm.h_pct,
        fm.w_pct,
        fm.clone_y_per,
        fm.ground_water,
        fm.ground_rain,
        fm.catcher_water,
        fm.catcher_rain,
        fm.catcher_flow,
        fm.catcher_reach_cm,
    ] {
        h.u(v);
    }
    h.u(fm.soil.len() as u32);
    for (biome, v) in &fm.soil {
        h.s(biome);
        h.u(*v);
    }
    h.u(c.crops.len() as u32);
    for k in &c.crops {
        h.s(&k.id);
        h.s(&k.look);
        h.s(&k.seed);
        h.s(&k.yield_item);
        h.u(k.count);
        h.u(k.seconds);
        h.u(k.clones);
    }
    h.u(c.cooks.len() as u32);
    let mut cooks: Vec<&Cook> = c.cooks.iter().collect();
    cooks.sort_by_key(|k| format!("{}\u{0}{}", k.station as u32, k.input));
    for k in cooks {
        h.s(&k.input);
        h.s(&k.output);
        h.u(k.count);
        h.u(k.seconds);
        h.u(k.station as u32);
    }

    // The research table, whole, and for `cooking`'s reason one block up:
    // every field here reaches the sim (`bake_research`), so a price the
    // digest cannot see would let two contents that charge different
    // amounts for the same blueprint canonicalise identically. Rows sort
    // by item id, which `validate::structural` refuses to repeat.
    h.s("research");
    h.s(&c.research_coin.item);
    // The table's paper and its wait (research table v1): both reach the
    // sim, and a ten-second table and a sixty-second one play differently.
    h.s(&c.research_table.blueprint);
    h.u(c.research_table.seconds);
    h.u(c.research.len() as u32);
    let mut research: Vec<&Research> = c.research.iter().collect();
    research.sort_by(|a, b| a.item.cmp(&b.item));
    for r in research {
        h.s(&r.item);
        h.u(r.cost);
        // The tree edge (tech tree v0). An absent parent hashes as the
        // empty string rather than being skipped: skipping would make
        // `requires = "x"` on the LAST row and no requires at all
        // canonicalise identically-prefixed streams, and the empty
        // string is a value no item id can be.
        h.s(r.requires.as_deref().unwrap_or(""));
    }

    let sv = &c.balance.survival;
    h.s("survival");
    h.u(sv.max_food);
    h.u(sv.max_water);
    h.u(sv.food_minutes_to_empty);
    h.u(sv.water_minutes_to_empty);
    h.u(sv.starve_hp_per_min);
    h.u(sv.dehydrate_hp_per_min);
    h.u(sv.spawn_hp);
    h.u(sv.spawn_food);
    h.u(sv.spawn_water);
    h.s(sv.vessel.as_deref().unwrap_or(""));
    h.s(sv.vessel_full.as_deref().unwrap_or(""));

    let ex = &c.balance.exposure;
    h.s("exposure");
    for v in [
        ex.wet_rain_per_s,
        ex.dry_per_s,
        ex.dry_fire_per_s,
        ex.soak_depth_cm,
        ex.night_cold,
        ex.rain_cold,
        ex.wind_cold,
        ex.wet_cold,
        ex.fire_warmth,
        ex.torch_warmth,
        ex.heat_radius_cm,
        ex.chill_rise_per_s,
        ex.chill_fall_per_s,
        ex.hurt_at,
        ex.hurt_hp_per_min,
        ex.cold_food_pct,
        ex.cold_water_pct,
        ex.fire_comfort,
        ex.company_comfort,
        ex.comfort_per_s,
        ex.regen_hp_per_min,
        ex.regen_fed_pct,
        ex.regen_comfort_pct,
        ex.regen_cap_pct,
        ex.regen_food_min,
        ex.regen_water_min,
        ex.regen_chill_max,
        ex.regen_calm_s,
        ex.regen_food_pct,
        ex.regen_water_pct,
        ex.cold_below_c as u32,
        ex.cold_per_c,
        ex.hot_above_c as u32,
        ex.heat_per_c,
        ex.shade_c as u32,
        ex.wet_cool,
        ex.clothes_heat_pm,
        ex.heat_water_pct,
        ex.heat_hurt_at,
        ex.heat_hp_per_min,
        ex.fel_rise_per_s,
        ex.fel_vent_per_s,
        ex.fel_fall_per_s,
        ex.fel_hurt_at,
        ex.fel_hp_per_min,
        ex.fel_regen_max,
        ex.fel_ward_max,
        ex.fel_pocket_per_s,
        ex.snow_wet_pm,
    ] {
        h.u(v);
    }
    let cl = &c.balance.climate;
    h.s("climate");
    for v in [
        cl.temperate_low_c as u32,
        cl.temperate_high_c as u32,
        cl.arid_low_c as u32,
        cl.arid_high_c as u32,
        cl.lapse_from_m,
        cl.lapse_m,
        cl.cloud_flatten_pm,
        cl.rain_cool_c as u32,
        cl.crop_frost_c as u32,
        cl.crop_warm_c as u32,
        cl.crop_hot_c as u32,
        cl.crop_scorch_c as u32,
    ] {
        h.u(v);
    }

    // The arc (`arc.toml`). Works hash in **file order**, not sorted: a
    // work's index is its row, and the world save files progress under that
    // index, so a reorder must read as different content.
    h.s("arc");
    h.u(c.arc.quiet_players as u32);
    h.u(c.arc.busy_players as u32);
    h.u(c.arc.quiet_burn_pct as u32);
    h.u(c.works.len() as u32);
    for w in &c.works {
        h.s(&w.id);
        h.s(&w.name);
        h.s(&w.site);
        h.u(w.nth as u32);
        for m in w.at {
            h.u(m.to_bits());
        }
        h.u(w.act as u32);
        h.u(w.opens_hour);
        h.u(w.fallback_hour);
        h.s(w.floor_unlock.as_deref().unwrap_or(""));
        h.s(w.ceiling_unlock.as_deref().unwrap_or(""));
        h.s(&w.fuel);
        h.u(w.fuel_max);
        h.u(w.burn_per_hour);
        h.stacks(&w.inputs);
    }
    h.u(c.arc_effects.len() as u32);
    for e in &c.arc_effects {
        h.s(&e.unlock);
        h.s(&e.knob);
        h.u(e.pct as u32);
    }
    // What saves point into: a glyph mask is bits over the alphabet, and a
    // mechanism's dials are filed under its row. File order, the works'
    // reason. The words themselves never reach a save.
    h.s(&c.glyphs.alphabet);
    h.u(c.inscriptions.len() as u32);
    for i in &c.inscriptions {
        h.s(&i.id);
        h.s(&i.teaches);
    }
    h.u(c.mechanisms.len() as u32);
    for m in &c.mechanisms {
        h.s(&m.id);
        h.u(m.dials as u32);
        h.u(m.values as u32);
        h.s(&m.reward);
        h.u(m.reward_n as u32);
        h.u(m.rest_minutes);
    }

    h.0.digest()
}

/// xxh3 over **what a save holds by position**, and nothing else: the row
/// ids every index a save carries points through, in index order, plus the
/// few authored lists whose order a save files state under.
///
/// [`hash`] moves with any value, so pinning a save to it made every
/// balance edit a wipe. A save holds item, recipe, piece and deployable
/// *ranks* (sorted ids), work and mechanism *rows* (file order), a work's
/// progress per *input slot*, a hearth's stock per *upkeep material*, a
/// glyph mask over the *alphabet*. Those move when a row is added, removed
/// or renamed, and then a save would be read as different things. Every
/// number on every row can move freely: a save holds none of them.
///
/// Conservative where it costs nothing: a piece's shape and material and a
/// deployable's archetype walk too, because a standing base was placed
/// under them and changing one is never a tuning pass.
pub fn layout_hash(c: &Content) -> u64 {
    let mut h = Canon(Xxh3::new());
    h.s("layout");
    h.s("items");
    for i in sorted(&c.items, |i| &i.id) {
        h.s(&i.id);
    }
    h.s("recipes");
    for r in sorted(&c.recipes, |r| &r.id) {
        h.s(&r.id);
    }
    h.s("pieces");
    for p in sorted(&c.pieces, |p| &p.id) {
        h.s(&p.id);
        h.u(p.shape as u32);
        h.u(p.material as u32);
    }
    h.s("deployables");
    for d in sorted(&c.deployables, |d| &d.id) {
        h.s(&d.id);
        h.u(d.archetype as u32);
    }
    // The hearth's stock rows: distinct build-cost items in rank order,
    // which is sorted-id order (`bake_deployables`).
    h.s("upkeep");
    let mut mats: Vec<&str> = c
        .pieces
        .iter()
        .flat_map(|p| p.cost.iter().map(|s| s.item.as_str()))
        .collect();
    mats.sort_unstable();
    mats.dedup();
    for m in mats {
        h.s(m);
    }
    h.s("works");
    for w in &c.works {
        h.s(&w.id);
        h.u(w.inputs.len() as u32);
        for s in &w.inputs {
            h.s(&s.item);
        }
    }
    h.s("mechanisms");
    for m in &c.mechanisms {
        h.s(&m.id);
        h.u(m.dials as u32);
    }
    h.s("glyphs");
    h.s(&c.glyphs.alphabet);
    // A plant saves its crop as an index into this order.
    h.s("crops");
    for k in &c.crops {
        h.s(&k.id);
    }
    h.0.digest()
}
