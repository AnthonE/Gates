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

/// The viewmodel a held item draws, keyed by the same normalised name the
/// icons and the mouse modality key off.
///
/// **This is the third reader of `stem`, and that is the point.** The wire
/// carries display names and nothing else — no content id, no tag — so a
/// rename in `content/items.toml` has exactly one place to break rather than
/// three, and `tests/ui.rs` drives all three off the same table. The
/// alternative was an item→model id on the wire, which is wall 6 for a fact
/// the client can already derive.
///
/// Order is the model-handle order `render::viewmodel` loads, so an index
/// here is an index there. Paths are relative to the asset root.
pub const HELD_MODELS: [HeldModelDef; 7] = [
    // A stone is gripped around its middle — there is no handle to hold.
    HeldModelDef::new("rock", "models/held/rock.glb", 0.20, 0.50),
    // A hafted tool is held near the butt, far from the head. A quarter up
    // the haft is where a hand actually sits on an axe.
    HeldModelDef::new("stone_hatchet", "models/held/stone_hatchet.glb", 0.50, 0.25),
    HeldModelDef::new("stone_pickaxe", "models/held/stone_pickaxe.glb", 0.60, 0.22),
    HeldModelDef::new("hammer", "models/held/hammer.glb", 0.35, 0.25),
    // A rolled document is carried in the middle, like the rock.
    HeldModelDef::new("building_plan", "models/held/building_plan.glb", 0.30, 0.50),
    // **The spear is the reason this table has a fraction at all.** At 1.8 m
    // it is nine times the rock, and one shared offset put its butt through
    // the camera. A third back from the point is a carry, not a thrust.
    HeldModelDef::new("wooden_spear", "models/held/wooden_spear.glb", 1.80, 0.35),
    // A bow is held at the riser, dead centre between the limbs.
    HeldModelDef::new("hunting_bow", "models/held/hunting_bow.glb", 1.20, 0.50),
];

/// One held model: the item it answers to, the file, and where the hand goes.
///
/// **The pose lives here rather than in `render` for this crate's standing
/// rule** — arithmetic in `crate::ui`, headless and gated; nodes and handles
/// in `render`. A grip offset is arithmetic in the sense that matters: it can
/// be silently wrong, and being wrong puts the butt of a spear through the
/// player's eye.
pub struct HeldModelDef {
    /// The normalised display name this draws for.
    pub key: &'static str,
    /// Asset path, relative to the asset root.
    pub path: &'static str,
    /// The model's long axis in metres — the size `ci/import_meshy.py` gave
    /// it, restated so the grip can be a fraction rather than seven separate
    /// hand-tuned offsets that drift when an asset is regenerated.
    pub length_m: f32,
    /// Where along that axis the hand sits, from the foot: 0.0 is the butt,
    /// 1.0 the tip, 0.5 the middle.
    pub grip_frac: f32,
}

impl HeldModelDef {
    const fn new(key: &'static str, path: &'static str, length_m: f32, grip_frac: f32) -> Self {
        Self {
            key,
            path,
            length_m,
            grip_frac,
        }
    }

    /// How far to slide the model DOWN its own long axis so the grip lands at
    /// the hand, metres. Negative by construction: the asset is authored with
    /// its foot at zero, so the hand is always somewhere above that.
    pub fn grip_offset_m(&self) -> f32 {
        -self.length_m * self.grip_frac
    }
}

/// Which [`HELD_MODELS`] row an item draws, or `None` for an empty hand and
/// for everything we have no model for.
///
/// `None` is not a failure and must not draw a fallback tool: an empty hand
/// is the commonest state in the game, and a stone standing in for a
/// revolver is worse than an empty hand, because it tells the player
/// something false about what they are carrying.
pub fn held_model(catalog: &ItemCatalog, stack: ItemStack) -> Option<usize> {
    if stack.count == 0 {
        return None;
    }
    let name = core::str::from_utf8(catalog.name(stack.item as usize)).ok()?;
    let s = stem(name);
    HELD_MODELS.iter().position(|m| m.key == s)
}

/// [`held_model`] on the selected hotbar slot. Clamped for [`held_in_hand`]'s
/// reason: `sel` is a keypress, not a checked index.
pub fn held_model_in_hand(catalog: &ItemCatalog, inv: &[ItemStack], sel: u8) -> Option<usize> {
    if inv.is_empty() {
        return None;
    }
    held_model(catalog, inv[(sel as usize).min(inv.len() - 1)])
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
        assert_eq!(
            held(
                &c,
                ItemStack {
                    item: 1,
                    count: 1,
                    cond: 0,
                }
            ),
            Held::Plan
        );
        assert_eq!(
            held(
                &c,
                ItemStack {
                    item: 2,
                    count: 1,
                    cond: 0,
                }
            ),
            Held::Hammer
        );
        assert_eq!(
            held(
                &c,
                ItemStack {
                    item: 0,
                    count: 5,
                    cond: 0,
                }
            ),
            Held::Other
        );
    }

    #[test]
    fn an_empty_hand_is_not_a_building_item() {
        let c = catalog_with(&["Wood", "Building Plan"]);
        // Count zero is an empty slot whatever the item word says — the
        // inventory leaves the id behind when a stack drains.
        assert_eq!(
            held(
                &c,
                ItemStack {
                    item: 1,
                    count: 0,
                    cond: 0,
                }
            ),
            Held::Other
        );
    }

    #[test]
    fn an_unnamed_item_is_other_rather_than_a_panic() {
        // A catalog row the server has not sent yet reads empty. Deciding
        // "not a building item" is the safe answer: the mouse keeps its
        // ordinary meaning until the content arrives.
        let c = catalog_with(&["Building Plan"]);
        assert_eq!(
            held(
                &c,
                ItemStack {
                    item: 40,
                    count: 1,
                    cond: 0,
                }
            ),
            Held::Other
        );
    }

    #[test]
    fn the_selection_is_clamped_to_the_inventory() {
        let c = catalog_with(&["Wood", "Hammer"]);
        let inv = [
            ItemStack {
                item: 1,
                count: 1,
                cond: 0,
            },
            ItemStack {
                item: 0,
                count: 9,
                cond: 0,
            },
        ];
        assert_eq!(held_in_hand(&c, &inv, 0), Held::Hammer);
        // Past the end clamps to the last slot rather than indexing out.
        assert_eq!(held_in_hand(&c, &inv, 200), Held::Other);
        assert_eq!(held_in_hand(&c, &[], 0), Held::Other);
    }

    #[test]
    fn an_empty_hand_draws_no_model_at_all() {
        let c = catalog_with(&["Rock"]);
        // The commonest state in the game, and the one worth stating: a count
        // of zero is not "hold a rock", it is hold nothing. `viewmodel::swap`
        // hides both the model and the stand-in on `None`, so a tool must
        // never appear over an empty cell.
        assert_eq!(
            held_model(
                &c,
                ItemStack {
                    item: 0,
                    count: 0,
                    cond: 0,
                }
            ),
            None
        );
        assert_eq!(
            held_model(
                &c,
                ItemStack {
                    item: 0,
                    count: 1,
                    cond: 0,
                }
            ),
            Some(0)
        );
    }

    #[test]
    fn an_item_with_no_model_is_none_and_not_a_default() {
        let c = catalog_with(&["Revolver"]);
        // `None` here means "wear the generic stand-in", which is a different
        // picture from an empty hand and from a modelled item. Returning
        // `Some(0)` would put a rock in the player's hand instead of a gun.
        assert_eq!(
            held_model(
                &c,
                ItemStack {
                    item: 0,
                    count: 1,
                    cond: 0,
                }
            ),
            None
        );
        // An id past the catalog is the same answer, never a panic: `sel` and
        // the stack both arrive from outside this function.
        assert_eq!(
            held_model(
                &c,
                ItemStack {
                    item: 900,
                    count: 1,
                    cond: 0,
                }
            ),
            None
        );
    }

    #[test]
    fn the_model_lookup_keys_off_the_display_name() {
        // The whole point of `stem`: the catalog says "Stone Hatchet" and the
        // file is `stone_hatchet.glb`, with no id table between them.
        let c = catalog_with(&["Stone Hatchet", "Wooden Spear"]);
        assert_eq!(
            held_model(
                &c,
                ItemStack {
                    item: 0,
                    count: 1,
                    cond: 0,
                }
            ),
            Some(
                HELD_MODELS
                    .iter()
                    .position(|m| m.key == "stone_hatchet")
                    .unwrap()
            )
        );
        assert_eq!(
            held_model_in_hand(
                &c,
                &[ItemStack {
                    item: 1,
                    count: 1,
                    cond: 0,
                }],
                0
            ),
            Some(
                HELD_MODELS
                    .iter()
                    .position(|m| m.key == "wooden_spear")
                    .unwrap()
            )
        );
        assert_eq!(held_model_in_hand(&c, &[], 0), None);
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
