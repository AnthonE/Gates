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
    fn opt_s(&mut self, v: Option<&str>) {
        match v {
            None => self.u(0),
            Some(s) => {
                self.u(1);
                self.s(s);
            }
        }
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
    }

    h.s("gatherables");
    for g in sorted(&c.gatherables, |g| &g.id) {
        h.s(&g.id);
        h.u(g.archetype as u32);
        h.s(&g.output);
        h.u(g.hits);
        h.u(g.weak_spot_bonus_pct);
        h.u(g.yield_per_hit.len() as u32);
        for (tool, per_hit) in &g.yield_per_hit {
            h.s(tool);
            h.u(*per_hit);
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
        match w.ballistic {
            None => h.u(0),
            Some(b) => {
                h.u(1);
                h.u(b.speed_mps);
                h.u(b.drop_mps2);
            }
        }
        h.opt_s(w.ammo.as_deref());
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
