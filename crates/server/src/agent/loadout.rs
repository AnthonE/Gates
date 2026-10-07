//! What this body carries where: the belt a player sets up before a fight.
//!
//! A human keeps a weapon, meds and tools on the belt, where a key selects
//! them, and the rest in the pack. The ladders below say which item is
//! better than which for each belt role, by **content id** — the wiki's
//! words for items — and [`Loadout::learn`] turns them into wire ids once
//! the catalog is in. Damage, reach and heal come from the wiki's pages;
//! what is in the pack comes from the client's own inventory mirror.
//!
//! Everything here reads fixed tables and the inventory; nothing allocates.

use super::wiki::{Book, Rules};
use client_core::core::ClientCore;
use sim_core::gather::{ItemStack, NO_ITEM};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS};

/// Swung weapons, best first. Reach first: a spear wins the fight it
/// starts against anything a metre long (`content/weapons.toml`).
pub const MELEE: [&str; 8] = [
    "item.spear_metal",
    "item.spear_wood",
    "item.hatchet_metal",
    "item.pickaxe_metal",
    "item.hatchet_stone",
    "item.pickaxe_stone",
    "item.bat",
    "item.torch",
];
/// Bows and guns, best first.
pub const RANGED: [&str; 3] = ["item.revolver", "item.crossbow", "item.bow"];
/// Health in a hurry, best first.
pub const MEDS: [&str; 2] = ["item.medkit", "item.bandage"];
/// What fells a tree and what breaks rock, best first.
pub const HATCHETS: [&str; 3] = ["item.hatchet_metal", "item.hatchet_stone", "item.bat"];
pub const PICKS: [&str; 3] = ["item.pickaxe_metal", "item.pickaxe_stone", "item.bat"];

/// The first arms the playbook makes once it has stone tools, in order,
/// with how many it wants and what it must own first: a spear, a bow and
/// arrows for it, and bandages. By catalog name, because that is all the
/// mind sees of an item (`mind::Summary`).
pub const ARM_UP: [(&str, u32, Option<&str>); 4] = [
    ("Wooden Spear", 1, None),
    ("Hunting Bow", 1, None),
    ("Wooden Arrow", 20, Some("Hunting Bow")),
    ("Bandage", 3, None),
];

/// A belt role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Melee,
    Ranged,
    Meds,
    Hatchet,
    Pick,
}

/// The belt policy: what a player keeps under the number keys, in order.
pub const BELT: [Role; 5] = [
    Role::Melee,
    Role::Ranged,
    Role::Meds,
    Role::Hatchet,
    Role::Pick,
];

const LADDER_MAX: usize = MELEE.len();

fn ladder_ids(role: Role) -> &'static [&'static str] {
    match role {
        Role::Melee => &MELEE,
        Role::Ranged => &RANGED,
        Role::Meds => &MEDS,
        Role::Hatchet => &HATCHETS,
        Role::Pick => &PICKS,
    }
}

/// The ladders, by wire id.
#[derive(Clone, Copy, Debug)]
pub struct Loadout {
    ladders: [[u16; LADDER_MAX]; BELT.len()],
    ready: bool,
}

impl Default for Loadout {
    fn default() -> Self {
        Self::new()
    }
}

impl Loadout {
    pub const fn new() -> Self {
        Self {
            ladders: [[NO_ITEM; LADDER_MAX]; BELT.len()],
            ready: false,
        }
    }

    /// Resolve every ladder through the wiki; an item the wire lacks stays
    /// off its ladder.
    pub fn learn(&mut self, rules: &Rules, book: &Book) {
        for (i, role) in BELT.iter().enumerate() {
            let ids = ladder_ids(*role);
            self.ladders[i] = [NO_ITEM; LADDER_MAX];
            for (rung, id) in ids.iter().enumerate() {
                self.ladders[i][rung] = book.wire(rules, id).unwrap_or(NO_ITEM);
            }
        }
        self.ready = book.ready();
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    fn ladder(&self, role: Role) -> &[u16] {
        let i = BELT.iter().position(|r| *r == role).unwrap_or(0);
        &self.ladders[i][..ladder_ids(role).len()]
    }

    /// Where an item stands on a role's ladder: 0 is the best.
    pub fn rank(&self, role: Role, item: u16) -> Option<usize> {
        if item == NO_ITEM {
            return None;
        }
        self.ladder(role).iter().position(|&w| w == item)
    }

    /// The best working item for a role on the belt: its slot and item.
    pub fn on_belt(&self, core: &ClientCore, role: Role) -> Option<(u8, u16)> {
        best_in(self, core, role, 0..HOTBAR_SLOTS).map(|(slot, item, _)| (slot as u8, item))
    }

    /// The best working item for a role anywhere on the body.
    pub fn best(&self, core: &ClientCore, role: Role) -> Option<(usize, u16)> {
        best_in(self, core, role, 0..INV_SLOTS).map(|(slot, item, _)| (slot, item))
    }

    /// Is this belt slot worth keeping there, were `extra` on the belt too:
    /// the best of some role among the belt and it (ties go to the lower
    /// slot).
    fn kept(&self, core: &ClientCore, slot: usize, extra: u16) -> bool {
        let s = core.inv[slot];
        if !usable(core, &s) {
            return false;
        }
        BELT.iter().any(|&role| {
            let Some(rank) = self.rank(role, s.item) else {
                return false;
            };
            let beaten_by_extra = self.rank(role, extra).is_some_and(|r| r < rank);
            let beaten_on_belt = (0..HOTBAR_SLOTS).any(|other| {
                let o = core.inv[other];
                other != slot
                    && usable(core, &o)
                    && self
                        .rank(role, o.item)
                        .is_some_and(|r| r < rank || (r == rank && other < slot))
            });
            !beaten_by_extra && !beaten_on_belt
        })
    }

    /// The whole-stack move that puts `item` on the belt, when it is in the
    /// pack, not on the belt already, and better at some role than what the
    /// belt holds for it: into an empty belt slot, else over the last belt
    /// slot the policy would not keep (the two stacks swap). `(from, to,
    /// count)`. The skill that wants it then selects its slot.
    pub fn belt_move(&self, core: &ClientCore, item: u16) -> Option<(u8, u8, u16)> {
        let held = |s: &ItemStack| s.count > 0 && s.item == item;
        if item == NO_ITEM || core.inv[..HOTBAR_SLOTS].iter().any(held) {
            return None;
        }
        let from = (HOTBAR_SLOTS..INV_SLOTS)
            .find(|&i| held(&core.inv[i]) && usable(core, &core.inv[i]))?;
        let improves = BELT.iter().any(|&role| {
            self.rank(role, item).is_some_and(|rank| {
                self.on_belt(core, role)
                    .is_none_or(|(_, have)| self.rank(role, have).is_none_or(|r| rank < r))
            })
        });
        if !improves {
            return None;
        }
        let to = (0..HOTBAR_SLOTS)
            .find(|&i| core.inv[i].count == 0)
            .or_else(|| (0..HOTBAR_SLOTS).rev().find(|&i| !self.kept(core, i, item)))?;
        Some((from as u8, to as u8, core.inv[from].count))
    }
}

/// Worth holding: something there, and not broken.
fn usable(core: &ClientCore, s: &ItemStack) -> bool {
    s.count > 0 && (core.catalog.row(s.item as usize).cond_max == 0 || s.cond > 0)
}

fn best_in(
    loadout: &Loadout,
    core: &ClientCore,
    role: Role,
    slots: std::ops::Range<usize>,
) -> Option<(usize, u16, usize)> {
    let mut best: Option<(usize, u16, usize)> = None;
    for slot in slots {
        let s = core.inv[slot];
        if !usable(core, &s) {
            continue;
        }
        if let Some(rank) = loadout.rank(role, s.item) {
            if best.is_none_or(|(_, _, r)| rank < r) {
                best = Some((slot, s.item, rank));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> (Box<Rules>, Book, ClientCore) {
        let content = content::Content::from_sources(&super::super::wiki::SOURCES).unwrap();
        let catalog = crate::net::bake_all(&content).unwrap().catalog;
        let rules = Rules::from_content(&content).unwrap();
        let mut book = Book::EMPTY;
        book.learn(&rules, &catalog);
        let mut core = ClientCore::new(1, 1, 0);
        core.catalog = catalog;
        (rules, book, core)
    }

    fn stack(core: &ClientCore, item: u16) -> ItemStack {
        ItemStack {
            item,
            count: 1,
            cond: core.catalog.cond_max(item as usize),
            skin: 0,
        }
    }

    /// Every ladder resolves through the wiki, and the belt policy moves a
    /// better weapon in over what it beats, never over a keeper.
    #[test]
    fn ladders_resolve_and_a_better_weapon_takes_a_belt_slot() {
        let (rules, book, mut core) = shipped();
        let mut kit = Loadout::new();
        kit.learn(&rules, &book);
        assert!(kit.ready());
        for role in BELT {
            for id in ladder_ids(role) {
                assert!(book.wire(&rules, id).is_some(), "{id}");
            }
        }
        let id = |s: &str| book.wire(&rules, s).unwrap();
        let (rock, hatchet, pick, spear, bandage) = (
            id("item.bat"),
            id("item.hatchet_stone"),
            id("item.pickaxe_stone"),
            id("item.spear_wood"),
            id("item.bandage"),
        );
        // A full belt of keepers and junk: rock, hatchet, pick, three junk.
        let wood = book.wire(&rules, "item.wood").unwrap();
        core.inv[0] = stack(&core, rock);
        core.inv[1] = stack(&core, hatchet);
        core.inv[2] = stack(&core, pick);
        for i in 3..HOTBAR_SLOTS {
            core.inv[i] = ItemStack {
                item: wood,
                count: 10,
                cond: 0,
                skin: 0,
            };
        }
        core.inv[10] = stack(&core, spear);
        core.inv[11] = stack(&core, bandage);
        assert_eq!(kit.on_belt(&core, Role::Melee), Some((1, hatchet)));
        // The spear goes over the last junk slot, not a tool.
        let (from, to, count) = kit.belt_move(&core, spear).unwrap();
        assert_eq!((from, to, count), (10, HOTBAR_SLOTS as u8 - 1, 1));
        core.inv.swap(10, usize::from(to));
        assert_eq!(kit.on_belt(&core, Role::Melee), Some((to, spear)));
        assert_eq!(kit.belt_move(&core, spear), None, "already on the belt");
        // A bandage is the meds role's first holder.
        assert!(kit.belt_move(&core, bandage).is_some());
        // Junk improves nothing.
        core.inv[12] = stack(&core, wood);
        assert_eq!(kit.belt_move(&core, wood), None);
        // A belt of keepers only: a second rock improves nothing, and the
        // rock itself is no keeper once hatchet, pick and spear are there.
        for i in 3..HOTBAR_SLOTS - 1 {
            core.inv[i] = ItemStack::default();
        }
        assert!(
            !kit.kept(&core, 0, NO_ITEM),
            "the rock is beaten at every role"
        );
        assert!(kit.kept(&core, 1, NO_ITEM) && kit.kept(&core, 2, NO_ITEM));
    }
}
