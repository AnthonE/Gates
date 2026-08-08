//! What is in your hand, and therefore what the mouse means.
//!
//! **The reference's whole building interface is held-item modal**, and ours
//! was not: `B` opened the shape wheel whether you were holding a building
//! plan, a revolver or nothing at all, and upgrade/repair/pick-up were bare
//! keys that worked the same way. Facepunch's own wiki is explicit for both
//! items — *"right click when equipped for more options"* — and the two
//! items own different wheels:
//!
//! | held | hold right | left |
//! |---|---|---|
//! | building plan | the shape wheel, which **latches** | place the ghost |
//! | hammer | the action wheel, which **fires** | repair swing |
//!
//! **Neither item has an attack**, which is what makes this safe: the
//! reference lists the hammer's Damage Total as **0** and the building plan
//! has no damage stats at all, so binding left click away from the swing
//! costs a player nothing. That was checked before the binding moved —
//! `DECISIONS.md` "the mouse is held-item modal".
//!
//! ## Why this identifies items by NAME
//!
//! Because the wire carries nothing else. `protocol::ItemCatalog` is display
//! names and lengths; there is no content id, no tag, and no "this is a
//! building plan" flag on any table the client receives. Adding one is a
//! wire change (wall 6) for a fact the client can already derive, so the
//! same normalisation the icons key off ([`crate::ui::icons::stem`]) does
//! this too — one rule, one gate, and a rename in `content/items.toml`
//! breaks both in the same place rather than one of them silently.
//!
//! `crates/client/tests/ui.rs` §H holds the content to it: an item whose
//! normalised name is `building_plan` and one that is `hammer` must exist,
//! or this module quietly decides nothing is ever held and the mouse
//! reverts to a swing.

use crate::ui::icons::stem;
use protocol::ItemCatalog;
use sim_core::gather::ItemStack;

/// The file stem of the item that opens the shape wheel and places.
pub const PLAN_STEM: &str = "building_plan";
/// The file stem of the item that opens the action wheel and repairs.
pub const HAMMER_STEM: &str = "hammer";

/// What the selected hotbar slot is holding, as far as the mouse cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Held {
    /// Anything with no building verb attached — a weapon, a resource, an
    /// empty hand. The mouse keeps its ordinary meaning.
    #[default]
    Other,
    /// The building plan.
    Plan,
    /// The hammer.
    Hammer,
}

impl Held {
    /// Does holding this open a wheel on right-click?
    pub fn opens_a_wheel(self) -> bool {
        matches!(self, Held::Plan | Held::Hammer)
    }

    /// Does left click place a building piece?
    pub fn places(self) -> bool {
        matches!(self, Held::Plan)
    }

    /// Does left click swing a repair?
    pub fn repairs(self) -> bool {
        matches!(self, Held::Hammer)
    }

    /// Should the build ghost be drawn?
    ///
    /// Only for the plan. The hammer acts on a piece that already stands, so
    /// a ghost under it would be offering a placement the left button does
    /// not make.
    pub fn shows_ghost(self) -> bool {
        matches!(self, Held::Plan)
    }
}

/// What the given stack is, by the catalog's name for it.
///
/// An empty stack is [`Held::Other`], not a special case: nothing in hand
/// and a rock in hand mean the same thing to the building verbs.
pub fn held(catalog: &ItemCatalog, stack: ItemStack) -> Held {
    if stack.count == 0 {
        return Held::Other;
    }
    let raw = catalog.name(stack.item as usize);
    let Ok(name) = core::str::from_utf8(raw) else {
        return Held::Other;
    };
    match stem(name).as_str() {
        PLAN_STEM => Held::Plan,
        HAMMER_STEM => Held::Hammer,
        _ => Held::Other,
    }
}

/// The stack in the selected hotbar slot, then [`held`] on it.
///
/// `sel` is the client-side hotbar latch, which is why it is clamped rather
/// than trusted: it is a `u8` off a keypress and the inventory is 30 slots.
pub fn held_in_hand(catalog: &ItemCatalog, inv: &[ItemStack], sel: u8) -> Held {
    if inv.is_empty() {
        return Held::Other;
    }
    let i = (sel as usize).min(inv.len() - 1);
    held(catalog, inv[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_with(names: &[&str]) -> ItemCatalog {
        let mut c = ItemCatalog::EMPTY;
        for (i, n) in names.iter().enumerate() {
            c.set(i, n.as_bytes()).expect("a short name fits");
        }
        c.count = names.len() as u16;
        c
    }

    #[test]
    fn the_two_building_items_are_recognised_by_their_display_name() {
        let c = catalog_with(&["Wood", "Building Plan", "Hammer"]);
        assert_eq!(held(&c, ItemStack { item: 1, count: 1 }), Held::Plan);
        assert_eq!(held(&c, ItemStack { item: 2, count: 1 }), Held::Hammer);
        assert_eq!(held(&c, ItemStack { item: 0, count: 5 }), Held::Other);
    }

    #[test]
    fn an_empty_hand_is_not_a_building_item() {
        let c = catalog_with(&["Wood", "Building Plan"]);
        // Count zero is an empty slot whatever the item word says — the
        // inventory leaves the id behind when a stack drains.
        assert_eq!(held(&c, ItemStack { item: 1, count: 0 }), Held::Other);
    }

    #[test]
    fn an_unnamed_item_is_other_rather_than_a_panic() {
        // A catalog row the server has not sent yet reads empty. Deciding
        // "not a building item" is the safe answer: the mouse keeps its
        // ordinary meaning until the content arrives.
        let c = catalog_with(&["Building Plan"]);
        assert_eq!(held(&c, ItemStack { item: 40, count: 1 }), Held::Other);
    }

    #[test]
    fn the_selection_is_clamped_to_the_inventory() {
        let c = catalog_with(&["Wood", "Hammer"]);
        let inv = [
            ItemStack { item: 1, count: 1 },
            ItemStack { item: 0, count: 9 },
        ];
        assert_eq!(held_in_hand(&c, &inv, 0), Held::Hammer);
        // Past the end clamps to the last slot rather than indexing out.
        assert_eq!(held_in_hand(&c, &inv, 200), Held::Other);
        assert_eq!(held_in_hand(&c, &[], 0), Held::Other);
    }

    #[test]
    fn the_verbs_each_item_owns() {
        assert!(Held::Plan.opens_a_wheel() && Held::Hammer.opens_a_wheel());
        assert!(!Held::Other.opens_a_wheel());
        // Place and repair are exclusive: one left button, one meaning.
        assert!(Held::Plan.places() && !Held::Plan.repairs());
        assert!(Held::Hammer.repairs() && !Held::Hammer.places());
        assert!(!Held::Other.places() && !Held::Other.repairs());
        // Only the plan previews, because only the plan places.
        assert!(Held::Plan.shows_ghost() && !Held::Hammer.shows_ghost());
    }
}
