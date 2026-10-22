//! The game guide the website's guide tab draws: every item, recipe, weapon,
//! armour piece, food, cooking step, research row and animal, read straight
//! off the validated content set, as `guide.json` (schema 1, the platform's
//! `/games/<slug>/guide/`).
//!
//! ```text
//! cargo run -p content --example guide_json -- [content_dir] [assets_dir] > guide.json
//! ```
//!
//! `ci/build_web.sh` writes it beside the browser build, so it is served at
//! `/games/gates/play/guide.json` with the icons it names. An icon path is
//! relative to that file and only written when `assets/icons/` ships one.

use content::schema::{CookStation, Rarity, Station};
use content::Content;
use std::fmt::Write as _;
use std::path::Path;

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The client's icon rule (`client/src/ui/icons.rs` `stem`).
fn stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending && !out.is_empty() {
                out.push('_');
            }
            pending = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending = true;
        }
    }
    out
}

/// One guide entry, written as it is built.
#[derive(Default)]
struct Entry {
    id: String,
    name: String,
    kind: String,
    text: String,
    icon: Option<String>,
    stats: Vec<(String, String)>,
    needs: Vec<(String, String, u32)>,
    makes: Vec<(String, String, u32)>,
    place: String,
}

impl Entry {
    fn json(&self) -> String {
        let mut j = format!("{{\"id\":{},\"name\":{}", esc(&self.id), esc(&self.name));
        if !self.kind.is_empty() {
            let _ = write!(j, ",\"kind\":{}", esc(&self.kind));
        }
        if !self.text.is_empty() {
            let _ = write!(j, ",\"text\":{}", esc(&self.text));
        }
        if let Some(i) = &self.icon {
            let _ = write!(j, ",\"icon\":{}", esc(i));
        }
        if !self.stats.is_empty() {
            let rows: Vec<String> = self
                .stats
                .iter()
                .map(|(k, v)| format!("[{},{}]", esc(k), esc(v)))
                .collect();
            let _ = write!(j, ",\"stats\":[{}]", rows.join(","));
        }
        for (key, list) in [("needs", &self.needs), ("makes", &self.makes)] {
            if !list.is_empty() {
                let rows: Vec<String> = list
                    .iter()
                    .map(|(id, name, n)| {
                        format!("{{\"id\":{},\"name\":{},\"n\":{n}}}", esc(id), esc(name))
                    })
                    .collect();
                let _ = write!(j, ",\"{key}\":[{}]", rows.join(","));
            }
        }
        if !self.place.is_empty() {
            let _ = write!(j, ",\"where\":{}", esc(&self.place));
        }
        j.push('}');
        j
    }
}

fn station(s: Station) -> &'static str {
    match s {
        Station::None => "By hand",
        Station::Workbench1 => "Workbench 1",
        Station::Workbench2 => "Workbench 2",
        Station::Workbench3 => "Workbench 3",
        Station::Furnace => "Furnace",
    }
}

fn cook_station(s: CookStation) -> &'static str {
    match s {
        CookStation::Fire => "Campfire",
        CookStation::Furnace => "Furnace",
        CookStation::Recycler => "Recycler",
    }
}

fn rarity(r: Rarity) -> &'static str {
    match r {
        Rarity::Common => "Common",
        Rarity::Uncommon => "Uncommon",
        Rarity::Rare => "Rare",
        Rarity::VeryRare => "Very rare",
    }
}

fn secs(ms: u32) -> String {
    format!("{:.1} s", f64::from(ms) / 1000.0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().map_or("content", String::as_str);
    let assets = args.get(1).map_or("assets", String::as_str);
    let c = Content::load_dir(Path::new(dir)).unwrap_or_else(|e| {
        eprintln!("guide_json: {e}");
        std::process::exit(1);
    });
    let name_of = |id: &str| -> String {
        c.items
            .iter()
            .find(|i| i.id == id)
            .map_or_else(|| id.to_string(), |i| i.name.clone())
    };
    let icon_of = |id: &str| -> Option<String> {
        let name = c.items.iter().find(|i| i.id == id)?.name.clone();
        let rel = format!("icons/{}.png", stem(&name));
        Path::new(assets)
            .join(&rel)
            .is_file()
            .then(|| format!("assets/{rel}"))
    };
    let stack = |id: &str, n: u32| (id.to_string(), name_of(id), n);
    let item_entry = |id: &str| Entry {
        id: id.to_string(),
        name: name_of(id),
        icon: icon_of(id),
        ..Entry::default()
    };

    let mut sections: Vec<(&str, &str, &str, Vec<Entry>)> = Vec::new();

    let items = c
        .items
        .iter()
        .map(|i| {
            let mut e = item_entry(&i.id);
            e.kind = format!("{:?}", i.class);
            e.text = i.description.clone();
            if i.stack > 1 {
                e.stats.push(("Stack".into(), i.stack.to_string()));
            }
            e.stats.push(("Tier".into(), i.tier.to_string()));
            e.stats.push(("Rarity".into(), rarity(i.rarity).into()));
            e
        })
        .collect();
    sections.push(("items", "Items", "Everything on the island.", items));

    let crafting = c
        .recipes
        .iter()
        .map(|r| {
            let mut e = item_entry(&r.output);
            e.id = r.id.clone();
            e.kind = station(r.station).into();
            e.needs = r.inputs.iter().map(|s| stack(&s.item, s.count)).collect();
            e.makes = vec![stack(&r.output, r.count)];
            e.stats.push(("Time".into(), format!("{} s", r.seconds)));
            let mut notes = Vec::new();
            if r.blueprint {
                notes.push("Learn its blueprint first.");
            }
            if r.unlock.is_some() {
                notes.push("The island has to unlock it.");
            }
            e.text = notes.join(" ");
            e
        })
        .collect();
    sections.push((
        "crafting",
        "Crafting",
        "What you can make, and from what.",
        crafting,
    ));

    let weapons = c
        .weapons
        .iter()
        .map(|w| {
            let mut e = item_entry(&w.id);
            e.id = format!("weapon.{}", w.id);
            e.kind = format!("{:?}", w.kind);
            e.stats.push(("Damage".into(), w.damage.to_string()));
            if w.rate_per_min > 0 {
                e.stats
                    .push(("Rate".into(), format!("{}/min", w.rate_per_min)));
            }
            if w.range_m > 0 {
                e.stats.push(("Range".into(), format!("{} m", w.range_m)));
            }
            if let Some(m) = w.magazine {
                e.stats.push(("Magazine".into(), m.to_string()));
            }
            if let Some(r) = w.reload_ms {
                e.stats.push(("Reload".into(), secs(r)));
            }
            if let Some(ammo) = &w.ammo {
                e.needs = ammo.iter().map(|a| stack(a, 1)).collect();
                e.text = "Fires the rounds listed.".into();
            }
            e
        })
        .collect();
    sections.push(("weapons", "Weapons", "", weapons));

    let armor = c
        .armors
        .iter()
        .map(|a| {
            let mut e = item_entry(&a.id);
            e.id = format!("armor.{}", a.id);
            e.kind = format!("{:?}", a.slot);
            e.stats
                .push(("Protection".into(), format!("{}%", a.reduction_pct)));
            if a.move_penalty_pct > 0 {
                e.stats
                    .push(("Slows you".into(), format!("{}%", a.move_penalty_pct)));
            }
            if a.cold_pct != 0 {
                e.stats.push(("Warmth".into(), format!("{}%", a.cold_pct)));
            }
            e
        })
        .collect();
    sections.push(("armor", "Armor", "", armor));

    let food = c
        .consumables
        .iter()
        .map(|f| {
            let mut e = item_entry(&f.id);
            e.id = format!("food.{}", f.id);
            for (k, v) in [("Health", f.health), ("Food", f.food), ("Water", f.water)] {
                if v > 0 {
                    e.stats.push((k.into(), format!("+{v}")));
                }
            }
            e
        })
        .collect();
    sections.push(("food", "Food and medicine", "", food));

    let cooking = c
        .cooks
        .iter()
        .map(|k| {
            let mut e = item_entry(&k.output);
            e.id = format!("cook.{}.{}", k.input, k.output);
            e.kind = cook_station(k.station).into();
            e.needs = vec![stack(&k.input, 1)];
            e.makes = vec![stack(&k.output, k.count)];
            e.stats.push(("Time".into(), format!("{} s", k.seconds)));
            e
        })
        .collect();
    sections.push((
        "cooking",
        "Cooking and smelting",
        "Fires, furnaces and the recycler.",
        cooking,
    ));

    let crops = c
        .crops
        .iter()
        .map(|k| {
            let mut e = item_entry(&k.yield_item);
            e.id = format!("crop.{}", k.id);
            e.kind = "Planter".into();
            e.needs = vec![stack(&k.seed, 1)];
            e.makes = vec![stack(&k.yield_item, k.count)];
            e.stats.push(("Time".into(), format!("{} s", k.seconds)));
            e
        })
        .collect();
    sections.push((
        "crops",
        "Crops",
        "One seed is one plant in a planter. Pick it ripe.",
        crops,
    ));

    let coin = name_of(&c.research_coin.item);
    let research = c
        .research
        .iter()
        .map(|r| {
            let mut e = item_entry(&r.item);
            e.id = format!("research.{}", r.item);
            e.place = "Research table".into();
            e.stats.push(("Cost".into(), format!("{} {coin}", r.cost)));
            if let Some(req) = &r.requires {
                e.text = format!("Learn {} first.", name_of(req));
            }
            e
        })
        .collect();
    sections.push((
        "research",
        "Research",
        "Blueprints, and what they cost.",
        research,
    ));

    let animals = c
        .mobs
        .iter()
        .map(|m| {
            let mut e = Entry {
                id: m.id.clone(),
                name: m.name.clone(),
                ..Entry::default()
            };
            e.stats.push(("Health".into(), m.hp.to_string()));
            e.stats.push(("Bite".into(), m.attack.to_string()));
            e.makes = m.drops.iter().map(|s| stack(&s.item, s.count)).collect();
            e.text = "What it drops is listed.".into();
            e
        })
        .collect();
    sections.push(("animals", "Animals", "", animals));

    let mut out = String::from("{\"game\":\"gates\",\"schema\":1,");
    let built = std::env::var("GUIDE_BUILT").unwrap_or_default();
    let _ = write!(out, "\"built\":{},\"sections\":[", esc(&built));
    let mut first = true;
    for (id, title, blurb, entries) in sections {
        if entries.is_empty() {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        let _ = write!(out, "{{\"id\":{},\"title\":{}", esc(id), esc(title));
        if !blurb.is_empty() {
            let _ = write!(out, ",\"blurb\":{}", esc(blurb));
        }
        let rows: Vec<String> = entries.iter().map(Entry::json).collect();
        let _ = write!(out, ",\"entries\":[{}]}}", rows.join(","));
    }
    out.push_str("]}\n");
    print!("{out}");
}
