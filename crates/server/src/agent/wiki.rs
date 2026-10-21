//! What a player knows from the wiki: the game's rules, read from the
//! shipped content files the way a person reads a weapons table. Arrow
//! speed, a spear's reach and a revolver's reload never ride the wire, and
//! knowing them is not knowing anything about the island or who is on it.
//!
//! The content is compiled in (`include_str!`), so the agent needs no file
//! on disk and every build knows the rules of the content it shipped with.
//! [`Rules`] is built once, off the frame path, keyed by content index;
//! [`Book`] re-keys it by **wire** item id once the catalog has dripped in,
//! matching by catalog name, and the wire wins wherever both carry a
//! number (the draw, the nock, what a mouthful heals).

use protocol::{ItemCatalog, MAX_ITEM_NAME_BYTES};
use sim_core::gather::{NO_ITEM, SWING_INTERVAL_TICKS};
use sim_core::limits::MAX_ITEM_DEFS;

/// Every content file, as shipped. `content::FILES` is the set; a test
/// holds the two together.
pub const SOURCES: [(&str, &str); 16] = [
    ("items.toml", include_str!("../../../../content/items.toml")),
    (
        "gatherables.toml",
        include_str!("../../../../content/gatherables.toml"),
    ),
    (
        "recipes.toml",
        include_str!("../../../../content/recipes.toml"),
    ),
    (
        "building.toml",
        include_str!("../../../../content/building.toml"),
    ),
    (
        "weapons.toml",
        include_str!("../../../../content/weapons.toml"),
    ),
    ("armor.toml", include_str!("../../../../content/armor.toml")),
    (
        "consumables.toml",
        include_str!("../../../../content/consumables.toml"),
    ),
    (
        "deployables.toml",
        include_str!("../../../../content/deployables.toml"),
    ),
    (
        "cooking.toml",
        include_str!("../../../../content/cooking.toml"),
    ),
    (
        "research.toml",
        include_str!("../../../../content/research.toml"),
    ),
    ("loot.toml", include_str!("../../../../content/loot.toml")),
    ("mobs.toml", include_str!("../../../../content/mobs.toml")),
    ("skins.toml", include_str!("../../../../content/skins.toml")),
    (
        "balance.toml",
        include_str!("../../../../content/balance.toml"),
    ),
    ("sites.toml", include_str!("../../../../content/sites.toml")),
    ("arc.toml", include_str!("../../../../content/arc.toml")),
];

/// What an item is for, as a player sorts a belt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Class {
    #[default]
    Other,
    /// Swung, and fells nothing: spears, the torch.
    Melee,
    /// Bows, crossbows, guns.
    Ranged,
    /// Thrown and planted: the satchel.
    Throw,
    /// Swung, and also gathers: rock, hatchets, pickaxes. Still a weapon
    /// ([`Page::melee`] is filled), just not only one.
    Tool,
    /// Eaten for food or water.
    Food,
    /// Eaten for health alone: bandage, medkit.
    Med,
}

/// A swing. Zero damage means the item does not swing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Melee {
    pub damage: u16,
    /// What one blow takes off a building piece or a deployable (on a
    /// sided piece's soft face; the hard face pays the game's floor).
    pub structure: u16,
    pub reach_cm: u16,
    /// Ticks between swings while primary is held; one shared number.
    pub cadence_ticks: u16,
    pub head_pct: u16,
    pub limb_pct: u16,
}

/// A shot. Zero damage means the item does not fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ranged {
    pub damage: u16,
    /// Ticks between shots; for a weapon that draws, the nock after a loose.
    pub rate_ticks: u16,
    /// Ticks the aim button is held before a drawn shot looses; 0 fires
    /// from the hip.
    pub draw_ticks: u16,
    /// A trace to `range_mm` rather than a flying round.
    pub hitscan: bool,
    pub range_mm: u32,
    /// The preferred round's item — a content index in [`Rules`], a wire
    /// id in [`Book`], `NO_ITEM` when there is none.
    pub round: u16,
    /// That round's flight: muzzle speed (mm per tick) and gravity (mm per
    /// tick squared). Zero for a hitscan weapon.
    pub speed_mmpt: u16,
    pub drop_mmpt2: u16,
    /// Rounds loaded at once; 0 spends straight from the pack.
    pub magazine: u16,
    pub reload_ticks: u16,
    pub head_pct: u16,
    pub limb_pct: u16,
}

impl Ranged {
    pub const NONE: Self = Self {
        damage: 0,
        rate_ticks: 0,
        draw_ticks: 0,
        hitscan: false,
        range_mm: 0,
        round: NO_ITEM,
        speed_mmpt: 0,
        drop_mmpt2: 0,
        magazine: 0,
        reload_ticks: 0,
        head_pct: 100,
        limb_pct: 100,
    };
}

/// A charge. Zero fuse means the item is not thrown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Throw {
    pub damage: u16,
    pub structure: u16,
    pub fuse_ticks: u16,
    pub reach_cm: u16,
    pub blast_cm: u16,
}

/// One item's entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub class: Class,
    pub melee: Melee,
    pub ranged: Ranged,
    pub throw: Throw,
    /// Health one use restores, from the wire catalog.
    pub heal: u16,
    /// What a fire makes of it, and in how many ticks (`NO_ITEM` for
    /// nothing): raw meat cooks, cooked meat burns.
    pub cooks: u16,
    pub cook_ticks: u16,
    /// A recycler takes it apart (`content/cooking.toml`'s recycler rows).
    pub recycles: bool,
}

impl Page {
    pub const EMPTY: Self = Self {
        class: Class::Other,
        melee: Melee {
            damage: 0,
            structure: 0,
            reach_cm: 0,
            cadence_ticks: 0,
            head_pct: 100,
            limb_pct: 100,
        },
        ranged: Ranged::NONE,
        throw: Throw {
            damage: 0,
            structure: 0,
            fuse_ticks: 0,
            reach_cm: 0,
            blast_cm: 0,
        },
        heal: 0,
        cooks: NO_ITEM,
        cook_ticks: 0,
        recycles: false,
    };

    pub fn swings(&self) -> bool {
        self.melee.damage > 0
    }

    pub fn fires(&self) -> bool {
        self.ranged.damage > 0
    }
}

/// Bytes kept of a content id (`item.hatchet_stone`): what a ladder names
/// an item by.
pub const ID_BYTES: usize = 40;

/// An animal's entry: what its bite does and how big a target it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Beast {
    pub hp: u16,
    pub bite: u16,
    pub reach_cm: u16,
    pub bite_ticks: u16,
    /// The hit volume a swing has to enter: a cylinder on its feet.
    pub radius_cm: u16,
    pub height_cm: u16,
}

/// The content's pages, by content index, with each item's display name
/// and content id.
pub struct Rules {
    names: [[u8; MAX_ITEM_NAME_BYTES]; MAX_ITEM_DEFS],
    lens: [u8; MAX_ITEM_DEFS],
    ids: [[u8; ID_BYTES]; MAX_ITEM_DEFS],
    id_lens: [u8; MAX_ITEM_DEFS],
    pages: [Page; MAX_ITEM_DEFS],
    count: usize,
    /// The pig's and the wolf's rows.
    pig: Beast,
    wolf: Beast,
    /// What an oven burns, by content index.
    fuel: u16,
}

impl Rules {
    /// The rules of the content this binary shipped with.
    pub fn shipped() -> Result<Box<Self>, String> {
        Self::from_content(&content::Content::from_sources(&SOURCES)?)
    }

    pub fn from_content(content: &content::Content) -> Result<Box<Self>, String> {
        use content::schema::WeaponKind;
        let combat = content.bake_combat()?;
        let mut rules = Box::new(Self {
            names: [[0; MAX_ITEM_NAME_BYTES]; MAX_ITEM_DEFS],
            lens: [0; MAX_ITEM_DEFS],
            ids: [[0; ID_BYTES]; MAX_ITEM_DEFS],
            id_lens: [0; MAX_ITEM_DEFS],
            pages: [Page::EMPTY; MAX_ITEM_DEFS],
            count: content.items.len().min(MAX_ITEM_DEFS),
            pig: Beast::default(),
            wolf: Beast::default(),
            fuel: content.item_index(&content.fuel.item).unwrap_or(NO_ITEM),
        });
        for m in &content.mobs {
            let n = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
            let beast = Beast {
                hp: n(m.hp),
                bite: n(m.attack),
                reach_cm: n(m.attack_range_m.saturating_mul(100)),
                bite_ticks: n(m.attack_seconds.saturating_mul(sim_core::limits::TICK_HZ)),
                radius_cm: n(m.body_r_cm),
                height_cm: n(m.body_h_cm),
            };
            match m.id.as_str() {
                "mob.pig" => rules.pig = beast,
                "mob.wolf" => rules.wolf = beast,
                _ => {}
            }
        }
        for item in &content.items {
            let Some(idx) = content.item_index(&item.id).map(usize::from) else {
                continue;
            };
            if idx >= MAX_ITEM_DEFS
                || item.name.len() > MAX_ITEM_NAME_BYTES
                || item.id.len() > ID_BYTES
            {
                return Err(format!("wiki: item `{}` does not fit the book", item.id));
            }
            rules.names[idx][..item.name.len()].copy_from_slice(item.name.as_bytes());
            rules.lens[idx] = item.name.len() as u8;
            rules.ids[idx][..item.id.len()].copy_from_slice(item.id.as_bytes());
            rules.id_lens[idx] = item.id.len() as u8;
            let page = &mut rules.pages[idx];
            let m = combat.melee[idx];
            if m.damage > 0 {
                page.melee = Melee {
                    damage: m.damage,
                    structure: m.structure,
                    reach_cm: m.reach_cm,
                    cadence_ticks: SWING_INTERVAL_TICKS as u16,
                    head_pct: m.head_pct,
                    limb_pct: m.limb_pct,
                };
            }
            let r = combat.ranged[idx];
            if r.damage > 0 {
                let round = r.ammo[0];
                let flight = combat.ammo.get(usize::from(round)).copied();
                page.ranged = Ranged {
                    damage: r.damage,
                    rate_ticks: r.rate_ticks,
                    draw_ticks: r.draw_ticks,
                    hitscan: r.hitscan,
                    range_mm: r.range_mm,
                    round,
                    speed_mmpt: flight.map_or(0, |a| a.speed_mmpt),
                    drop_mmpt2: flight.map_or(0, |a| a.drop_mmpt2),
                    magazine: r.magazine,
                    reload_ticks: r.reload_ticks,
                    head_pct: r.head_pct,
                    limb_pct: r.limb_pct,
                };
            }
            let t = combat.throw[idx];
            page.throw = Throw {
                damage: t.damage,
                structure: t.structure,
                fuse_ticks: t.fuse_ticks,
                reach_cm: t.reach_cm,
                blast_cm: t.blast_cm,
            };
        }
        for c in &content.cooks {
            let (Some(input), Some(output)) =
                (content.item_index(&c.input), content.item_index(&c.output))
            else {
                continue;
            };
            let Some(page) = rules.pages.get_mut(usize::from(input)) else {
                continue;
            };
            match c.station {
                content::schema::CookStation::Fire => {
                    page.cooks = output;
                    let ticks = c.seconds.saturating_mul(sim_core::limits::TICK_HZ);
                    page.cook_ticks = u16::try_from(ticks).unwrap_or(u16::MAX);
                }
                content::schema::CookStation::Recycler => page.recycles = true,
                content::schema::CookStation::Furnace => {}
            }
        }
        for w in &content.weapons {
            let Some(idx) = content.item_index(&w.id).map(usize::from) else {
                continue;
            };
            let gathers = content
                .gatherables
                .iter()
                .any(|g| g.yield_per_hit.contains_key(&w.id));
            rules.pages[idx].class = match w.kind {
                WeaponKind::Bow | WeaponKind::Firearm => Class::Ranged,
                WeaponKind::Throwable => Class::Throw,
                WeaponKind::Melee if gathers => Class::Tool,
                WeaponKind::Melee => Class::Melee,
            };
        }
        Ok(rules)
    }

    fn name(&self, idx: usize) -> &[u8] {
        &self.names[idx][..usize::from(self.lens[idx])]
    }

    fn index_of(&self, name: &[u8]) -> Option<usize> {
        (0..self.count).find(|&i| !name.is_empty() && self.name(i) == name)
    }

    /// The content index of an item by its content id.
    pub fn index_of_id(&self, id: &str) -> Option<usize> {
        (0..self.count).find(|&i| {
            !id.is_empty() && &self.ids[i][..usize::from(self.id_lens[i])] == id.as_bytes()
        })
    }
}

/// The rules re-keyed by wire item id: what this body looks an item up in.
/// Fixed storage, filled once, without allocating, when the catalog is
/// complete.
#[derive(Clone, Copy, Debug)]
pub struct Book {
    pages: [Page; MAX_ITEM_DEFS],
    /// Content index → wire id, `NO_ITEM` for an item the wire lacks.
    wire_of: [u16; MAX_ITEM_DEFS],
    pig: Beast,
    wolf: Beast,
    /// What an oven burns, by wire id.
    fuel: u16,
    ready: bool,
    /// Wire items no page was found for, by name.
    pub unknown: u16,
    /// Numbers the wire and the rules disagreed on (the wire's were kept).
    pub disagreements: u16,
}

impl Default for Book {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Book {
    pub const EMPTY: Self = Self {
        pages: [Page::EMPTY; MAX_ITEM_DEFS],
        wire_of: [NO_ITEM; MAX_ITEM_DEFS],
        pig: Beast {
            hp: 0,
            bite: 0,
            reach_cm: 0,
            bite_ticks: 0,
            radius_cm: 0,
            height_cm: 0,
        },
        wolf: Beast {
            hp: 0,
            bite: 0,
            reach_cm: 0,
            bite_ticks: 0,
            radius_cm: 0,
            height_cm: 0,
        },
        fuel: NO_ITEM,
        ready: false,
        unknown: 0,
        disagreements: 0,
    };

    pub fn ready(&self) -> bool {
        self.ready
    }

    /// One wire item's page; the empty page for anything unknown.
    pub fn page(&self, item: u16) -> &Page {
        self.pages.get(usize::from(item)).unwrap_or(&Page::EMPTY)
    }

    /// The wire id of an item named by its content id, once learned.
    pub fn wire(&self, rules: &Rules, id: &str) -> Option<u16> {
        let w = *self.wire_of.get(rules.index_of_id(id)?)?;
        (w != NO_ITEM).then_some(w)
    }

    pub fn pig(&self) -> &Beast {
        &self.pig
    }

    /// What an oven burns (`NO_ITEM` before the catalog is in).
    pub fn fuel(&self) -> u16 {
        self.fuel
    }

    pub fn wolf(&self) -> &Beast {
        &self.wolf
    }

    /// Fill from the rules and a complete wire catalog.
    pub fn learn(&mut self, rules: &Rules, catalog: &ItemCatalog) {
        *self = Self::EMPTY;
        let count = usize::from(catalog.count).min(MAX_ITEM_DEFS);
        // Content index → wire id, for the rounds a weapon names.
        let mut wire_of = [NO_ITEM; MAX_ITEM_DEFS];
        let (mut unknown, mut disagreements) = (0, 0);
        for (w, page) in self.pages[..count].iter_mut().enumerate() {
            let row = catalog.row(w);
            match rules.index_of(catalog.name(w)) {
                Some(c) => {
                    wire_of[c] = w as u16;
                    *page = rules.pages[c];
                }
                None => unknown += 1,
            }
            // The wire's draw and nock are what the sim runs; the rules'
            // copy is a cross-check.
            let draw = u16::from(row.draw_ticks);
            if page.ranged.draw_ticks != draw {
                disagreements += 1;
                page.ranged.draw_ticks = draw;
            }
            if row.draws() && page.ranged.rate_ticks != u16::from(row.nock_ticks) {
                disagreements += 1;
                page.ranged.rate_ticks = u16::from(row.nock_ticks);
            }
            page.heal = row.health;
            if page.class == Class::Other && row.eats() {
                page.class = if row.food == 0 && row.water == 0 {
                    Class::Med
                } else {
                    Class::Food
                };
            }
        }
        for page in &mut self.pages[..count] {
            let round = usize::from(page.ranged.round);
            page.ranged.round = wire_of.get(round).copied().unwrap_or(NO_ITEM);
            let cooked = usize::from(page.cooks);
            page.cooks = wire_of.get(cooked).copied().unwrap_or(NO_ITEM);
        }
        self.fuel = wire_of
            .get(usize::from(rules.fuel))
            .copied()
            .unwrap_or(NO_ITEM);
        self.wire_of = wire_of;
        self.pig = rules.pig;
        self.wolf = rules.wolf;
        self.unknown = unknown;
        self.disagreements = disagreements;
        self.ready = count > 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content::schema::WeaponKind;

    fn shipped() -> (content::Content, Book) {
        let content = content::Content::from_sources(&SOURCES).unwrap();
        let catalog = crate::net::bake_all(&content).unwrap().catalog;
        let rules = Rules::from_content(&content).unwrap();
        let mut book = Book::EMPTY;
        book.learn(&rules, &catalog);
        (content, book)
    }

    #[test]
    fn the_compiled_in_sources_are_the_content_set() {
        let names: Vec<&str> = SOURCES.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, content::FILES);
        assert!(Rules::shipped().is_ok());
    }

    /// Every shipped weapon has a page under its wire id, with the numbers
    /// its kind needs, and the wire and the rules agree.
    #[test]
    fn every_shipped_weapon_resolves() {
        let (content, book) = shipped();
        assert!(book.ready());
        assert_eq!((book.unknown, book.disagreements), (0, 0));
        assert!(!content.weapons.is_empty());
        for w in &content.weapons {
            // Content order and wire order are one bake apart; the book is
            // keyed by the wire's, which `bake_catalog` sets.
            let wire = content.item_index(&w.id).unwrap();
            let page = book.page(wire);
            match w.kind {
                WeaponKind::Melee => {
                    assert!(matches!(page.class, Class::Melee | Class::Tool), "{}", w.id);
                    assert!(page.swings() && page.melee.reach_cm > 0, "{}", w.id);
                    assert_eq!(page.melee.cadence_ticks, SWING_INTERVAL_TICKS as u16);
                }
                WeaponKind::Bow => {
                    assert_eq!(page.class, Class::Ranged, "{}", w.id);
                    assert!(page.fires() && !page.ranged.hitscan, "{}", w.id);
                    assert!(page.ranged.speed_mmpt > 0 && page.ranged.drop_mmpt2 > 0);
                    assert!(page.ranged.rate_ticks > 0);
                    assert_ne!(page.ranged.round, NO_ITEM, "{}", w.id);
                }
                WeaponKind::Firearm => {
                    assert_eq!(page.class, Class::Ranged, "{}", w.id);
                    assert!(page.fires() && page.ranged.hitscan && page.ranged.range_mm > 0);
                    assert!(page.ranged.magazine > 0 && page.ranged.reload_ticks > 0);
                    assert_ne!(page.ranged.round, NO_ITEM, "{}", w.id);
                }
                WeaponKind::Throwable => {
                    assert_eq!(page.class, Class::Throw, "{}", w.id);
                    assert!(page.throw.fuse_ticks > 0 && page.throw.structure > 0);
                }
            }
        }
        let by_name = |name: &str| {
            let item = content.items.iter().find(|i| i.name == name).unwrap();
            *book.page(content.item_index(&item.id).unwrap())
        };
        assert!(by_name("Hunting Bow").ranged.draw_ticks > 0);
        assert_eq!(by_name("Bat").class, Class::Tool);
        assert_eq!(by_name("Wooden Spear").class, Class::Melee);
        assert!(by_name("Wooden Spear").melee.reach_cm > by_name("Bat").melee.reach_cm);
        let bandage = by_name("Bandage");
        assert_eq!((bandage.class, bandage.heal > 0), (Class::Med, true));
        assert_eq!(by_name("Corn").class, Class::Food);
        assert_eq!(by_name("Wood").class, Class::Other);
    }

    /// Ladders name items by content id, and animals have entries too.
    #[test]
    fn content_ids_and_animals_resolve() {
        let content = content::Content::from_sources(&SOURCES).unwrap();
        let catalog = crate::net::bake_all(&content).unwrap().catalog;
        let rules = Rules::from_content(&content).unwrap();
        let mut book = Book::EMPTY;
        assert_eq!(
            book.wire(&rules, "item.spear_wood"),
            None,
            "not learned yet"
        );
        book.learn(&rules, &catalog);
        let spear = book.wire(&rules, "item.spear_wood").unwrap();
        assert_eq!(catalog.name(usize::from(spear)), b"Wooden Spear");
        assert_eq!(book.wire(&rules, "item.no_such_thing"), None);
        let (wolf, pig) = (book.wolf(), book.pig());
        assert!(wolf.hp > 0 && wolf.bite > 0 && wolf.reach_cm > 0 && wolf.bite_ticks > 0);
        assert!(wolf.radius_cm > 0 && wolf.height_cm > 0 && pig.hp > 0);
        // What a fire makes of meat, what it burns, what a recycler takes.
        let wire = |id| book.wire(&rules, id).unwrap();
        let raw = book.page(wire("item.raw_meat"));
        assert_eq!(raw.cooks, wire("item.cooked_meat"));
        assert!(raw.cook_ticks > 0 && raw.class != Class::Food);
        let cooked = book.page(wire("item.cooked_meat"));
        assert_eq!(cooked.cooks, wire("item.burnt_meat"));
        assert_eq!(cooked.class, Class::Food);
        assert_eq!(book.fuel(), wire("item.wood"));
        assert!(book.page(wire("item.gears")).recycles);
        assert_eq!(book.page(wire("item.wood")).cooks, NO_ITEM);
    }

    /// Where the wire disagrees, the wire is kept and the difference
    /// counted.
    #[test]
    fn the_wire_wins_a_disagreement() {
        let content = content::Content::from_sources(&SOURCES).unwrap();
        let mut catalog = crate::net::bake_all(&content).unwrap().catalog;
        let bow = content.item_index("item.bow").unwrap();
        let mut row = catalog.row(usize::from(bow));
        row.draw_ticks += 3;
        catalog.rows[usize::from(bow)] = row;
        let mut book = Book::EMPTY;
        book.learn(&Rules::from_content(&content).unwrap(), &catalog);
        assert_eq!(book.disagreements, 1);
        assert_eq!(book.page(bow).ranged.draw_ticks, u16::from(row.draw_ticks));
    }
}
