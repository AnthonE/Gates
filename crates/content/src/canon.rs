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
        // The condition ceiling reaches the sim (`bake_gather`'s
        // `cond_max`), so it walks — a value the sim reads and the digest
        // cannot see lets two contents whose tools die at different rates
        // canonicalise identically, and a WAL then replays under a wear
        // table it was not played under (item durability v0).
        h.u(i.condition_max);
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
        h.u(w.headshot_mult);
        h.u(w.rate_per_min);
        h.u(w.range_m);
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
    }

    h.s("armor");
    for a in sorted(&c.armors, |a| &a.id) {
        h.s(&a.id);
        h.u(a.slot as u32);
        h.u(a.reduction_pct);
        h.u(a.move_penalty_pct);
    }

    h.s("consumables");
    for con in sorted(&c.consumables, |c| &c.id) {
        h.s(&con.id);
        h.u(con.health);
        h.u(con.food);
        h.u(con.water);
        h.u(con.seconds);
    }

    h.s("deployables");
    for d in sorted(&c.deployables, |d| &d.id) {
        h.s(&d.id);
        h.u(d.archetype as u32);
        h.u(d.placement as u32);
        match d.material {
            None => h.u(0),
            Some(m) => h.u(1 + m as u32),
        }
        h.u(d.hp);
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
        h.u(m.roam_m);
        h.u(m.spook_m);
        h.u(m.night_spook_m);
        h.u(m.respawn_seconds);
        h.stacks(&m.drops);
    }

    h.s("skins");
    for s in sorted(&c.skins, |s| &s.id) {
        h.s(&s.id);
        h.s(&s.covers);
        h.u(s.coin as u32);
        h.u(s.price);
        h.s(&s.season);
    }

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
    let b = &c.balance.bands;
    for pair in [
        b.ttk_melee,
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
    h.u(b.headshot_mult);
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

    h.0.digest()
}
