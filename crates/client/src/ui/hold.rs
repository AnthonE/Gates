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
    // `lay` is wrong and the one where the scale cheat is not about
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
    // A hafted tool is held near the butt, far from the head. 0.25 of the
    // model is **36% up the haft**, which is where the number finally means
    // what this line has always claimed: the haft is the bottom 0.393 m of a
    // 0.562 m model and the head is the rest. It used to mean 48%, a choked
    // grip halfway up, because the haft was only half the model's declared
    // height when the model was measured across its own lean.
    //
    // **0.562 is not a regenerated asset, it is the same axe measured along
    // the haft instead of across the box** (`ci/stand_grip.py`, 2026-09-01).
    // The file was authored leaning 32°, so its +Y extent was the diagonal's
    // shadow — and, worse, `import_meshy.py` centres the BOUNDING BOX, which
    // on a heavy head hung off one side of a leaning handle put the haft
    // **121 mm from the +Y axis at the grip height**. `viewmodel::pose` did
    // what it says and slid thin air into the palm, so the axe hung a hand's
    // width beside the fist and pointed 32° across the frame (operator,
    // 2026-09-01, with the frame). Standing it up is a rigid rotation:
    // nothing about the axe changed size, and `scale` is still 1.0.
    //
    // **And it is neither `tool` nor plain `upright`; the pose is two
    // measured angles** (operator, 2026-09-01, twice, with a reference frame
    // each time: *"it needs to be a right angle"*, then *"it needs to tilt
    // more to the right and then back towards the character"*).
    //
    // `lay = FRAC_PI_2` — what `tool()` gives — is what a SPEAR wants. On a
    // hatchet it laid the haft to **19.5° above horizontal**, drawing at 36°
    // on screen, within a few degrees of the forearm's own diagonal: the axe
    // read as the arm continuing rather than as a thing being carried. Flat
    // upright (`lay = 0`) is 69° and 106.7° on screen — vertical, but leaning
    // 17° LEFT and only 13° back. These two angles are the pose the second
    // reference describes, solved rather than nudged. Then turned again
    // (operator, 2026-09-01: *"rotate the axe counter clockwise 22 degrees.
    // then if the bottom of the axe was pinned down the whole then need to
    // rotate forward way more"*): **22.3° CCW in the image plane — the
    // APPARENT angle, 84.6° → 106.9°, which is not the direction vector's
    // 94° because the item hangs 0.32 m right of centre and perspective
    // tilts it — and the head swung 55° forward, from 20.5° leaning back at
    // the eye to 35.0° away from it**, at 52.2° of elevation. Then 8° back
    // clockwise, then 45.8° more (*"if the rotation was in the middle of the
    // axe object then its facing at 11 oclock we need it at around 1 o
    // clock"*): butt-to-head **105.8° → 60.0°**, elevation held at 54.8°,
    // lean −10.1°.
    //
    // ⚠ **Measure that angle in PIXELS, not in NDC, and not on the point
    // cloud's principal axis.** Both wrong ways were tried here. NDC is
    // aspect-corrected and pixels are not, so a 16:9 frame stretches x by
    // 1.78 and an angle read in NDC is off by ~7° — the operator reads the
    // screen. And a PCA of the projected silhouette is not the direction the
    // axe points, because this head is 61% of the model's length and hangs
    // off one side, so the cloud's principal axis is the HEAD's axis: it
    // read 97° where the picture read 106°, and solving on it swung the axe
    // flat. Butt centroid to head centroid, projected, in pixels, is the one
    // that agreed with the frame (it put the head at 830, 348 against 835,
    // 375 measured off the capture).
    //
    // ⚠ **The elevation is the constraint nobody would guess, and it is why
    // the lean stops at 20°.** "Back towards the character" is depth, and
    // depth is foreshortening: at 27° the head starts to read as a lump and
    // at 34.5° — which was tried, and shot — the silhouette stops being an
    // axe at all. Holding elevation at 69° is what keeps the head high enough
    // in frame to still have a shape. The trade is visible in one place:
    // `posesheet.py`'s projection of these four poses, which is arithmetic
    // through the same camera and agreed with the captures it was checked
    // against.
    //
    // `pose_yaw` also decides which way the head faces, and that is the other
    // reason it is not 0: the head's long axis is the model's +X, so untwisted
    // it lies broadside — 34.5 of its 34.5 cm across the screen, on an axe
    // whose head is already 61% of its own length.
    //
    // ⚠ **Two things this row is NOT, and both are measured**: it is not the
    // thumb line — the rig's thumb runs (0.412, 0.906, −0.095) in the hand's
    // own frame and every pose that satisfies the framing above sits ~90° off
    // it, because `VIEWMODEL_GRIP_Q` is reverse-derived from a VIEW pose and
    // carries no anatomy at all; and it does not fix the swing, whose apex
    // still finishes above the horizon. `NOW.md` §0fp carries both.
    //
    // ⚠ **The lean was invisible until the model was stood up**: the file's
    // own 32° lean cancelled a third of the lay, so the wrong angle and the
    // wrong asset were hiding each other.
    HeldModelDef {
        key: "stone_hatchet",
        src: HeldSrc::Glb("models/held/stone_hatchet.glb"),
        height_m: 0.562,
        // 0.15, not the 0.25 it shipped at (operator, 2026-09-01: *"make the
        // Y value or whatever of the axe go up so the axe slides up the hand
        // more"*). This is the knob rather than a translation because it
        // slides the axe ALONG ITS OWN HAFT through the fist — 4.8 cm up,
        // drawn — instead of lifting it off the palm, which is the defect
        // this whole row exists to have fixed. The fist is 0.084 m up a
        // 0.393 m haft now; the fist band still lands on haft, which
        // `the_fist_closes_on_the_model_and_not_on_air` is what checks.
        grip_frac: 0.15,
        // Judged smaller by eye, twice (operator, 2026-09-01: *"maybe the ax
        // needs to be a little bit smaller"*, then *"if anything we should
        // scale the whole axe down if we can"*). At true size the axe is 70%
        // of the frame's height at the hold's depth; 0.85 was 60% and this is
        // 51%. The ordinary viewmodel scale cheat, the bow's and the rock's —
        // and NOT a fix for the head, which is 0.345 m wide on a 0.562 m axe
        // and stays out of proportion at every scale.
        scale: 0.72,
        lay: 0.960,
        pose_yaw: -0.663,
        light: None,
    },
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
    //
    // **"Upright" was a table entry and not a fact about the file until
    // 2026-09-01**: the bow was authored across a 45° diagonal, so keeping it
    // upright kept the diagonal, and the riser sat 165 mm off the axis the
    // fist is on — the hatchet's defect on the other modelled hold. 1.687 is
    // the limb-to-limb length, which is what 1.191 was a diagonal's shadow
    // of; the bow is the same size on screen, because `scale` is what sets
    // that and `ci/stand_grip.py` only turns the mesh.
    HeldModelDef::upright(
        "hunting_bow",
        "models/held/hunting_bow.glb",
        1.687,
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
        lay: 0.0,
        pose_yaw: 0.0,
        light: Some(TORCH_LIGHT),
    },
    // A revolver is authored barrel-up so the shared quarter-turn points it
    // forward, and gripped low, at the handle.
    HeldModelDef {
        key: "revolver",
        src: HeldSrc::Gen("revolver"),
        height_m: 0.261,
        grip_frac: 0.19,
        scale: 1.0,
        lay: core::f32::consts::FRAC_PI_2,
        pose_yaw: -0.65,
        light: None,
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
    /// How far the model is laid off its own +Y, radians, about the hold
    /// frame's X — 0 keeps it upright, `FRAC_PI_2` lays it flat forward
    /// (+Y → −Z, the quarter-turn a swung tool wants).
    ///
    /// ⚠ **This was a `bool` and the thing it controls is an ANGLE**, which
    /// cost the hatchet two passes. A rotation about the hold frame's X and
    /// `render::viewmodel::VIEWMODEL_TILT`'s pitch term are near-parallel —
    /// the item frame's own X sits 31° off the view's — so between them they
    /// decide how far off vertical a haft stands, and with one of the two
    /// pinned to 0 or 90° the row could ask for **19.5° or 69° and nothing
    /// in between**. Neither is what an axe wants. Now the row says the
    /// angle and `pose_yaw` says which way it leans, which together point
    /// the model's +Y anywhere on the sphere.
    pub lay: f32,
    /// Extra presentation yaw about the fist, radians, composed onto the
    /// pose. Zero for almost everything; the revolver turns so the frame
    /// shows its PROFILE — seen from dead behind, a gun is a stack of
    /// blocks, and the L-shape is the whole read.
    pub pose_yaw: f32,
    /// What this item puts into the world when it is in the hand, or `None`
    /// for the twelve rows that put nothing. See [`HeldLight`].
    pub light: Option<HeldLight>,
}

/// A light a held item casts. One row carries one today — the torch.
///
/// **It is a table entry rather than a `match` in `render`** for the reason
/// the module header states about the grip: the pose lives with the item
/// because it can be silently wrong, and a light is the same shape of fact.
/// A `match arch` in the renderer is how `structures::burns` had to be
/// written — the deployables have no per-row table to hang it on — and this
/// one does.
///
/// **The colour is deliberately NOT here.** There is exactly one flame
/// colour in this client (`render::structures::FIRE_COLOR`) and a second
/// copy of it in a second module is the drift `CLAUDE.md` warns about on
/// every mirror: a torch and a campfire burn the same thing, so they read
/// the same constant. Lumens and metres are arithmetic and live here;
/// a `Color` is a render type and could not live here anyway.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HeldLight {
    /// Luminous flux, lumens — the unit Bevy's `PointLight::intensity` is
    /// in. **(knob)**, registered in `DECISIONS.md` §open.
    pub lumens: f32,
    /// Where the light is cut off, metres. **(knob)** Past this the fragment
    /// is not touched at all; it is a budget, not a look, and it wants to be
    /// set where the contribution has already fallen under the night ambient
    /// — see [`pool_radius_m`].
    pub range_m: f32,
}

/// The torch's light. **(knob)** — `DECISIONS.md` §open, "torch light v0".
///
/// **Priced ordinally, which is what the emission rule in
/// `.claude/skills/threejs-procedural-vfx` actually asks for**: its numbers
/// "are evidence of relative hierarchy inside that scene, not universal
/// exposure-independent constants… preserve the relationship." The ladder
/// this client has is `sun 100 000 lx → campfire 900 lm → torch → night
/// ambient 60 lux`, and the torch's whole job is to sit between the last
/// two. So it is **600 lm — deliberately under `structures::FIRE_LUMENS`**,
/// because a hand torch that out-lit a fire pit standing next to one is the
/// inverted hierarchy that rule exists to forbid, and no amount of "it
/// reads better" is allowed to buy it.
///
/// **What it buys, in metres.** [`pool_radius_m`] is where the source stops
/// beating the ambient: 600 lm against 60 lux is **0.89 m**. The flame is
/// carried about 1.4 m up, so the ground at the player's feet takes ~24 lux
/// of orange on top of 60 lux of blue-white — a 1.4× warm pool with an
/// inverse-square edge, against an ambient that is direction-free and has
/// no edge at all. The chroma is doing as much work as the ratio:
/// `rig`'s night ambient is `srgb(0.80, 0.85, 0.95)` and this is
/// `structures::FIRE_COLOR`.
///
/// **The photometry says 600 is already generous and the ambient is the
/// real problem.** Nobody has ever put a torch in an integrating sphere;
/// the defensible chain is a candle at 4π ≈ 12.6 lm and ~0.16 lm/W for a
/// sooting flame, giving a large pitch torch **~250 lm**. We are 2.4× that
/// — and `rig::NIGHT_AMBIENT_LUX` is **240× moonlight**, by its own
/// admission. Fixing that ratio properly is one owner over `rig`'s coupled
/// set (`CLAUDE.md` §traps: tonemap, sky, exposure and fog are one owner),
/// not a bigger number here. `NOW.md` §0tl carries it.
///
/// **The exposure trap in that skill pack does not apply to us, and saying
/// so is the point.** It warns that a torch filling an unmasked 64×36
/// luminance meter pulls exposure down and darkens the world *because* you
/// lit a torch. This client has no meter: `rig` runs a fixed
/// `Exposure { ev100: 14.2 }`. So the failure mode is absent — and so is
/// the recovery, which is why the ambient has to carry night rather than an
/// adaptation curve.
///
/// Range matches the campfire's 6 m for the same reason it has one: past
/// there the contribution is a rounding error on the ambient (5 lux at 3 m,
/// 1.3 at 6) and all a longer reach buys is fragments touched.
pub const TORCH_LIGHT: HeldLight = HeldLight {
    lumens: 600.0,
    range_m: 6.0,
};

/// Where a light of `lumens` stops beating a uniform `ambient_lux`, metres.
///
/// A point light of `L` lumens radiates `L / 4π` candela, so its
/// illuminance at `d` metres is `L / (4π d²)`; set that equal to the ambient
/// and solve. **This is the number that decides whether a held light is a
/// mechanic or a decoration**, and it is a function rather than a comment
/// so a gate can read it — `tests/hand_light.rs` binds it to the
/// renderer's own `NIGHT_AMBIENT_LUX`, which is the coupling a written
/// constant would lose the moment night changed.
///
/// Returns 0.0 for a non-positive ambient rather than an infinity: an
/// ambient of zero means every radius beats it, which is true and is not a
/// radius.
pub fn pool_radius_m(lumens: f32, ambient_lux: f32) -> f32 {
    if ambient_lux <= 0.0 || lumens <= 0.0 {
        return 0.0;
    }
    (lumens / (4.0 * core::f32::consts::PI * ambient_lux)).sqrt()
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
            lay: core::f32::consts::FRAC_PI_2,
            pose_yaw: 0.0,
            light: None,
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
            lay: 0.0,
            pose_yaw: 0.0,
            light: None,
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

    /// How far above the fist this row's light sits, metres — the crown of
    /// the model plus [`FLAME_LIFT_M`].
    ///
    /// **Derived from the mesh rather than typed**, for the reason
    /// [`grip_m`](Self::grip_m) is: a hand-tuned offset drifts the moment
    /// the geometry is regenerated, and a light that has drifted INSIDE the
    /// head lights nothing at all while looking like a dead constant. The
    /// crown is `height_m · scale` up the model's own +Y and the fist is
    /// `grip_m` up the same axis, so the flame is the difference.
    ///
    /// Only meaningful for an upright row, and every row that declares a
    /// light is one — `tests/hand_light.rs` refuses a lit row with
    /// a non-zero `lay`, because a laid-forward model's +Y is the view's −Z
    /// and this offset would push the flame out of the frame instead of up.
    pub fn flame_m(&self) -> f32 {
        self.height_m * self.scale - self.grip_m() + FLAME_LIFT_M
    }
}

/// How far the flame sits above the model's crown, metres. **(knob)**
///
/// Not zero, and the reason is the one thing a point light cannot do: it
/// does not light the surface it is standing on. At the crown exactly, the
/// torch's own wrapped head is at the light's origin and stays black while
/// everything around it brightens — a lamp with a dark bulb. Four
/// centimetres up puts the head *below* the source, so it is lit from above
/// like anything else the torch lights, and the player sees where the light
/// is coming from without a flame mesh existing yet (`NOW.md` §0tl).
pub const FLAME_LIFT_M: f32 = 0.04;

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

/// **The client's copy of `sim_core::light::is_lit`**, returning the
/// [`HELD_MODELS`] row that is actually alight (torch fuel v0).
///
/// The same three facts the sim reads, in the same order, from data both
/// ends already hold — which is the whole reason a flame is derived on
/// both sides instead of stored on one and shipped to the other:
///
/// 1. `latch` — the player's own `BTN_LIGHT`, `render::Net::light`, which
///    is what crosses to the sim;
/// 2. the row declaring a [`HeldLight`] at all, this side's spelling of
///    the content row's `light_burn`;
/// 3. `cond > 0` — the fuel, mirrored here by `SUB_INV`, which is how a
///    torch that burned out goes dark on this side without a message
///    existing to say "your torch went out".
///
/// It lags the sim by exactly one round trip on the third fact and by
/// nothing on the other two, and lagging is the correct failure: the
/// flame dies a few frames late, never lights something that is not
/// burning.
///
/// **`cond` is only asked of a row that declares a light.** A zero
/// condition means *never wears* for everything else in the game
/// (`GatherContent::cond_max`), and asking it of a hatchet would put out
/// a light no hatchet has. Content rule V8 is what makes the question
/// safe here — a light always has a ceiling to spend.
pub fn lit_model_in_hand(
    catalog: &ItemCatalog,
    inv: &[ItemStack],
    sel: u8,
    latch: bool,
) -> Option<usize> {
    if !latch || inv.is_empty() {
        return None;
    }
    let stack = inv[(sel as usize).min(inv.len() - 1)];
    let row = held_model(catalog, stack)?;
    if HELD_MODELS[row].light.is_none() || stack.cond == 0 {
        return None;
    }
    Some(row)
}

/// The [`HELD_MODELS`] row **another player's** hand draws, from the item
/// id wire v56 puts on their entity record.
///
/// The remote twin of [`held_model_in_hand`], and the asymmetry between
/// them is the point: your own hand is an index into an inventory this
/// client mirrors, and theirs is a single id, because their inventory is
/// not yours to see. One id is also all a remote hand can honestly show —
/// `count` is theirs, `cond` is theirs, and the wire deliberately carries
/// neither.
///
/// `None` covers an empty hand and an item with no model, exactly as
/// [`held_model`] does and for the same reason: a stand-in tool on someone
/// else's body is a *worse* lie than an empty one, because it is the fact
/// a fight opens by reading.
pub fn held_model_of(catalog: &ItemCatalog, held: Option<u16>) -> Option<usize> {
    // `count: 1` because the wire already answered the question `count`
    // exists to answer here — the server sends `None` for an empty slot
    // (`server/core.rs` `held_of`), so a stack that reaches this call is
    // one somebody is holding. `cond` is unread by `held_model`.
    held_model(
        catalog,
        ItemStack {
            item: held?,
            count: 1,
            cond: 1,
        },
    )
}

/// The row **another player's** hand is burning, or `None`.
///
/// The remote twin of [`lit_model_in_hand`], and it reads ONE fact where
/// that one reads three: the latch and the fuel are the holder's own, so
/// the server resolved all three once (`sim-core/light.rs` `is_lit`) and
/// sent the answer. This side only re-asks the question that is about
/// *drawing* — does this row declare a light at all — which is why a
/// content row with `light_burn` and no [`HeldLight`] shows a lit item
/// and no glow rather than a glow from nowhere.
///
/// **Not derived from `held` alone, and that is the disclosure.** A torch
/// with no fuel left is still a torch in the hand; if this returned a
/// light for every torch, `ALPHA.md` §1's *light = visibility = target*
/// would run backwards — the one thing another player must not be able
/// to fake is being dark.
pub fn lit_model_of(catalog: &ItemCatalog, held: Option<u16>, lit: bool) -> Option<usize> {
    if !lit {
        return None;
    }
    let row = held_model_of(catalog, held)?;
    HELD_MODELS[row].light.is_some().then_some(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_with(names: &[&str]) -> ItemCatalog {
        let mut c = ItemCatalog::EMPTY;
        for (i, n) in names.iter().enumerate() {
            c.set(i, n.as_bytes(), protocol::ItemRow::EMPTY)
                .expect("a short name fits");
        }
        c.count = names.len() as u16;
        c
    }

    /// The client's half of the derived flame, one fact at a time.
    ///
    /// This is the mirror of `sim_core::light::tests::
    /// a_flame_needs_the_latch_the_row_and_the_fuel`, deliberately shaped
    /// the same way, because the two are one predicate computed twice —
    /// the quantize-both-sides law applied to a flag. A drift between them
    /// is a torch that glows on one screen and burns on the other, and
    /// neither side's own suite can see it alone.
    #[test]
    fn a_drawn_flame_needs_the_latch_the_row_and_the_fuel() {
        let c = catalog_with(&["Torch", "Rock"]);
        let full = ItemStack {
            item: 0,
            count: 1,
            cond: 5_000,
        };
        let inv = |s: ItemStack| [s, ItemStack::default()];

        assert_eq!(
            lit_model_in_hand(&c, &inv(full), 0, true),
            Some(torch_row()),
            "latch, row and fuel all hold"
        );
        assert_eq!(
            lit_model_in_hand(&c, &inv(full), 0, false),
            None,
            "the latch is off — a torch in the hand is not a lit torch"
        );
        assert_eq!(
            lit_model_in_hand(&c, &inv(ItemStack { cond: 0, ..full }), 0, true),
            None,
            "spent: `SUB_INV` is how this side hears the flame died"
        );
        let rock = ItemStack {
            item: 1,
            count: 1,
            cond: 0,
        };
        assert_eq!(
            lit_model_in_hand(&c, &inv(rock), 0, true),
            None,
            "a rock declares no light, and its zero `cond` means *never \
             wears* rather than *spent* — the row is what refuses it"
        );
        assert_eq!(
            lit_model_in_hand(&c, &inv(ItemStack::default()), 0, true),
            None,
            "an empty hand"
        );
        assert_eq!(
            lit_model_in_hand(&c, &[], 0, true),
            None,
            "and an inventory that has not arrived yet"
        );
    }

    /// The row index the assertions above want, found by the same lookup
    /// the client uses rather than typed — `HELD_MODELS` is reordered by
    /// anyone adding a model.
    fn torch_row() -> usize {
        HELD_MODELS
            .iter()
            .position(|m| m.light.is_some())
            .expect("exactly one row declares a light")
    }

    /// **The seam this design creates, gated at the seam.**
    ///
    /// A flame is derived on both sides, and the two sides spell "this is
    /// a light" differently: over here it is a `HELD_MODELS` row with a
    /// [`HeldLight`], over there it is a `light_burn` in `content/
    /// items.toml`. Nothing links them — the client links no content
    /// crate (`PROTO_VER`'s own note on why the catalog drips) — so the
    /// two lists can drift apart in either direction and every other gate
    /// in both crates stays green: a content row losing `light_burn`
    /// leaves a torch that glows and never burns, and a `HELD_MODELS` row
    /// losing its `light` leaves one that burns and never glows.
    ///
    /// So this reads the TOML as text, which is the only thing this crate
    /// can do and is the right shape anyway (`CLAUDE.md`: read the
    /// surface, do not keep a mirror of it). Both directions, by name.
    #[test]
    fn the_lit_rows_and_the_burning_content_rows_are_the_same_set() {
        let toml = include_str!("../../../../content/items.toml");
        // `[[item]]` blocks, each reduced to (stem of `name`, has a
        // `light_burn` line). A whole TOML parser is not owed here: the
        // file is generated by nobody and read by `crates/content` for
        // real, so what this needs is the two fields, spelled the way the
        // file spells them.
        let mut burning: Vec<String> = Vec::new();
        for block in toml.split("[[item]]").skip(1) {
            let body = block.split("[[").next().unwrap_or(block);
            let name = body.lines().find_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("name")?.trim_start().strip_prefix('=')?;
                Some(rest.trim().trim_matches('"').to_string())
            });
            let burns = body.lines().any(|l| {
                let l = l.trim();
                l.starts_with("light_burn") && !l.starts_with('#')
            });
            if burns {
                burning.push(stem(&name.expect("every item row has a name")).to_string());
            }
        }
        assert!(
            !burning.is_empty(),
            "no `[[item]]` row in content/items.toml declares `light_burn` — \
             either the scrape stopped matching the file or every light in \
             the game became free"
        );
        let mut drawn: Vec<String> = HELD_MODELS
            .iter()
            .filter(|m| m.light.is_some())
            .map(|m| m.key.to_string())
            .collect();
        burning.sort();
        drawn.sort();
        assert_eq!(
            drawn, burning,
            "the set of held models that DRAW a light and the set of item \
             rows that BURN for one have drifted apart. A row on the left \
             only glows for free; a row on the right only burns in the dark."
        );
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
