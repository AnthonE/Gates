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

/// Where a held row's geometry comes from.
///
/// Data only, deliberately: this module is the headless arithmetic tier, so
/// a generated row carries a NAME and `render::heldgen` owns the mesh behind
/// it. An unknown name there is a panic at boot and a red render-tier test
/// (`tests/held_assets.rs`), never a silent empty hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldSrc {
    /// A `.glb` under the asset root: single primitive, single material,
    /// authored standing (+Y up, feet at y = 0) — `ci/import_meshy.py`'s
    /// convention, gated per file by `tests/held_assets.rs`.
    Glb(&'static str),
    /// Built at startup by `render::heldgen`, same authoring convention.
    Gen(&'static str),
}

/// The viewmodel a held item draws, keyed by the same normalised name the
/// icons and the mouse modality key off.
///
/// **This is the third reader of `stem`, and that is the point.** The wire
/// carries display names (and, since v46, condition ceilings — a number,
/// not an identity) — no content id, no tag — so a
/// rename in `content/items.toml` has exactly one place to break rather than
/// three, and `tests/ui.rs` drives all three off the same table. The
/// alternative was an item→model id on the wire, which is wall 6 for a fact
/// the client can already derive.
///
/// Order is the model-handle order `render::viewmodel` loads, so an index
/// here is an index there. Paths are relative to the asset root.
///
/// **The deployables reuse the world's own `models/deploy/*.glb`**, scaled
/// down — holding a box shows the box you would place, out of assets already
/// shipped. Absent on purpose: the fire pit (`fire.glb` bakes a lit
/// `emissiveFactor` and a carried unlit one must not glow —
/// `tests/held_assets.rs::nothing_held_glows`), and every greybox archetype
/// (furnace, workbench 2/3, research table, lock), because a scaled cuboid in
/// the hand tells the player less than the stand-in tool does.
pub const HELD_MODELS: [HeldModelDef; 14] = [
    // **A stone is palmed, not hafted, which makes this the one row where
    // `lay_forward` is wrong and the one where the scale cheat is not about
    // frame area.** Laid forward the model's 16 cm axis stands up and its
    // flat authored bottom faces the eye — a slab, edge-on. Upright it sits
    // the way it was authored to sit on the ground, straddling the fist
    // front-to-back by its own Z extent because the mesh is centred on that
    // axis, and `grip_frac` becomes a HEIGHT: low, so the stone rests on the
    // palm rather than being skewered by it.
    //
    // **0.60 is the number the capture argued for.** At full size the model
    // is 0.200 across and this rig's hand is not — a stone twice the width of
    // the hand holding it cannot read as held, at any offset, and the frames
    // showed exactly that: the fingers vanished BEHIND a boulder whose near
    // face stood 8 cm proud of the palm. Shrunk to the hand it is a stone in
    // an open palm with the fingers around it, which is the picture. Not the
    // deployables' cheat (a token of a world-sized object) — this is a held
    // prop authored bigger than the hand that holds it.
    HeldModelDef::upright("rock", "models/held/rock.glb", 0.103, 0.15, 0.60),
    // A hafted tool is held near the butt, far from the head. A quarter up
    // the haft is where a hand actually sits on an axe.
    HeldModelDef::tool(
        "stone_hatchet",
        "models/held/stone_hatchet.glb",
        0.500,
        0.25,
    ),
    HeldModelDef::tool(
        "stone_pickaxe",
        "models/held/stone_pickaxe.glb",
        0.600,
        0.22,
    ),
    HeldModelDef::tool("hammer", "models/held/hammer.glb", 0.350, 0.25),
    // A rolled document is carried in the middle — no haft either, so the
    // fist closes around the roll rather than beside it. 0.50 of 0.069 is
    // 3.5 cm; the same fraction used to mean 15 cm up a model 6.9 cm tall.
    HeldModelDef::tool(
        "building_plan",
        "models/held/building_plan.glb",
        0.069,
        0.50,
    ),
    // **The spear is the reason this table has a fraction at all.** At 1.8 m
    // it is seventeen times the rock's height, and one shared offset put its
    // butt through the camera. A third back from the point is a carry, not a
    // thrust; the butt behind the grip leaves the frame at the near plane,
    // which is what every first-person spear does.
    HeldModelDef::tool("wooden_spear", "models/held/wooden_spear.glb", 1.800, 0.35),
    // A bow is held at the riser, dead centre between the limbs — and held
    // UPRIGHT, which is what a bow looks like in a hand and what laying it
    // flat never did. Drawn at 0.8: a viewmodel's scale cheat, so the top
    // limb stays inside the frame instead of leaving it at the hold's depth.
    HeldModelDef::upright(
        "hunting_bow",
        "models/held/hunting_bow.glb",
        1.191,
        0.50,
        0.80,
    ),
    // The two generated rows — `render::heldgen` owns the geometry, this
    // table owns where the hand goes, same split as the glb rows.
    // A torch stays upright: its whole read is the head above the fist.
    HeldModelDef {
        key: "torch",
        src: HeldSrc::Gen("torch"),
        height_m: 0.455,
        grip_frac: 0.35,
        scale: 1.0,
        lay_forward: false,
        pose_yaw: 0.0,
    },
    // A revolver is authored barrel-up so the shared quarter-turn points it
    // forward, and gripped low, at the handle.
    HeldModelDef {
        key: "revolver",
        src: HeldSrc::Gen("revolver"),
        height_m: 0.261,
        grip_frac: 0.19,
        scale: 1.0,
        lay_forward: true,
        pose_yaw: -0.65,
    },
    // The deployables, palmed level. `height_m` restates each FILE's +Y
    // extent (the gate measures it); the grip fraction puts the palm near the
    // TOP of the shown object — carried by an upper edge, hanging below the
    // fist — which is now what the number SAYS, 0.80-odd of the object's own
    // height, rather than a fraction of a different axis that happened to
    // land there.
    // Scales are small on purpose and were judged off a capture: the first
    // cut at 0.26 put a 29 cm crate at 43 cm from the eye and it ate a
    // quarter of the frame — a held deployable is a token of the thing, and
    // the placement ghost at the reticle is the actual size claim.
    HeldModelDef::upright("small_box", "models/deploy/box.glb", 0.610, 0.83, 0.16),
    HeldModelDef::upright("large_box", "models/deploy/box.glb", 0.610, 0.81, 0.22),
    HeldModelDef::upright("sleeping_bag", "models/deploy/bag.glb", 0.308, 0.80, 0.12),
    HeldModelDef::upright(
        "workbench",
        "models/deploy/workbench.glb",
        0.822,
        0.81,
        0.15,
    ),
    HeldModelDef::upright("hearth", "models/deploy/hearth.glb", 1.000, 0.80, 0.20),
];

/// One held model: the item it answers to, its geometry source, and how the
/// hand carries it.
///
/// **The pose lives here rather than in `render` for this crate's standing
/// rule** — arithmetic in `crate::ui`, headless and gated; nodes and handles
/// in `render`. A grip offset is arithmetic in the sense that matters: it can
/// be silently wrong, and being wrong puts the butt of a spear through the
/// player's eye — or, as shipped for a while, hangs every tool `grip_m` BELOW
/// the fist, because the offset was applied on the parent's Y after the model
/// had been quarter-turned onto −Z. The offset is now a point (`grip_m` up
/// the model's own +Y) and `render::viewmodel::swap` rotates it with the
/// model, so the axis cannot be wrong twice.
pub struct HeldModelDef {
    /// The normalised display name this draws for.
    pub key: &'static str,
    /// Where the geometry comes from — a shipped `.glb` or a generated mesh.
    pub src: HeldSrc,
    /// The model's own **+Y extent** in metres — the size
    /// `ci/import_meshy.py` gave it (or `render::heldgen` builds), restated
    /// so the grip can be a fraction rather than hand-tuned offsets that
    /// drift when an asset is regenerated. `tests/held_assets.rs` holds both
    /// kinds to the file.
    ///
    /// ⚠ **The +Y extent and not the longest axis, and the difference was a
    /// shipped defect.** This said `length_m` and measured whichever axis was
    /// longest, while [`grip_frac`](Self::grip_frac) has always been applied
    /// up +Y. For a haft that is the same number — a spear is 1.8 m of Y and
    /// nothing else — so the two agreed on every row anyone was looking at.
    /// They do not agree on a model that is WIDER than it is tall: the rock is
    /// 0.200 across and 0.103 up, so "half its length" put the fist at 97% of
    /// its height, on the far face, and the whole stone hung between the hand
    /// and the eye with the hand hidden behind it (operator, 2026-08-26:
    /// *"can we make sure the rock is in the hand better somehow?"*). The
    /// building plan was worse and unreported: 0.150 up a model 0.069 tall,
    /// a grip point 8 cm off the object entirely. Declaring the axis the grip
    /// actually slides along is what makes a fraction mean one thing.
    pub height_m: f32,
    /// Where along the model's own +Y the hand sits, as a fraction of
    /// `height_m`: 0.0 is the foot, 1.0 the crown.
    pub grip_frac: f32,
    /// Uniform in-hand scale. 1.0 for the hafted tools, which are modelled at
    /// hand size; below 1.0 for the deployables, which are modelled at WORLD
    /// size and would fill the frame — the ordinary viewmodel scale cheat —
    /// and for the rock, which is not that case: it is a held prop simply
    /// authored wider than the hand can close on, and a thing bigger than the
    /// hand around it reads as floating however it is placed.
    pub scale: f32,
    /// `true` lays the model forward (+Y → −Z, the quarter-turn a swung tool
    /// wants); `false` keeps it upright, for the things a hand carries level
    /// — a bow at the riser, a torch, a box.
    pub lay_forward: bool,
    /// Extra presentation yaw about the fist, radians, composed onto the
    /// pose. Zero for almost everything; the revolver turns so the frame
    /// shows its PROFILE — seen from dead behind, a gun is a stack of
    /// blocks, and the L-shape is the whole read.
    pub pose_yaw: f32,
}

impl HeldModelDef {
    /// A swung tool: full scale, laid forward.
    const fn tool(key: &'static str, path: &'static str, height_m: f32, grip_frac: f32) -> Self {
        Self {
            key,
            src: HeldSrc::Glb(path),
            height_m,
            grip_frac,
            scale: 1.0,
            lay_forward: true,
            pose_yaw: 0.0,
        }
    }

    /// A carried object: stays upright, drawn at `scale`.
    const fn upright(
        key: &'static str,
        path: &'static str,
        height_m: f32,
        grip_frac: f32,
        scale: f32,
    ) -> Self {
        Self {
            key,
            src: HeldSrc::Glb(path),
            height_m,
            grip_frac,
            scale,
            lay_forward: false,
            pose_yaw: 0.0,
        }
    }

    /// How far up the model's own +Y the grip point sits once scaled, metres.
    /// `swap` rotates this with the model's pose and slides the model so the
    /// point lands in the fist — the one place the item is attached to.
    ///
    /// A fraction of [`height_m`](Self::height_m), which is the +Y extent,
    /// because +Y is the axis this offset is spent on. That reads as a
    /// tautology and it is the whole bug fix — see the field.
    pub fn grip_m(&self) -> f32 {
        self.height_m * self.scale * self.grip_frac
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
            c.set(i, n.as_bytes(), 0).expect("a short name fits");
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
        // The revolver used to be this test's example and then grew a model,
        // which is the point of the table. The metal hatchet is the honest
        // example now: `reference/ARMOR.md`'s shape of gap — content priced
        // and validated with no picture behind it.
        let c = catalog_with(&["Metal Hatchet"]);
        // `None` here means "wear the generic stand-in", which is a different
        // picture from an empty hand and from a modelled item. Returning
        // `Some(0)` would put a rock in the player's hand instead of a tool.
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
