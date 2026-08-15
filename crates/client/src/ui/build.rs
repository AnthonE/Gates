//! The radial build menu — the wheel in the reference frame, over the table
//! `content/building.toml` actually is.
//!
//! ## Why two rings rather than one
//!
//! The reference wheel is one ring of shapes because its building plan is a
//! flat list of them. Ours is not: `content/building.toml` is **6 shapes ×
//! 3 materials**, eighteen rows, and the material is not a cosmetic — it is
//! the hp ladder and the upgrade path (wood → stone → metal). Flattening
//! that onto one ring would be eighteen segments a thumb cannot hit; putting
//! the material on a second ring says what the content says, and it means
//! the two choices are independent, which is how a player thinks about them
//! ("a wall, in stone").
//!
//! So: **outer ring picks the shape, middle ring picks the material, the
//! centre is the readout** — name, blurb, hp, and the cost of the piece the
//! two rings currently name. The centre is a dead zone by design; the
//! reference's is too, and a wheel whose middle is clickable is a wheel that
//! fires when a player lets go without choosing.
//!
//! ## The one number that crosses the wall
//!
//! Everything here resolves to a **piece row index**, which is the first
//! argument of `encode_action_place`. [`row_for`] is the only thing that
//! produces one, and it produces it by searching the baked table for the
//! (shape, material) pair rather than by arithmetic on the wheel's own
//! indices — `shape * 3 + material` would be right for today's file and
//! wrong the first time someone adds a shape without all three materials,
//! and the failure would be *placing a different piece than the one drawn*.
//! That is the positional-payload class, one table over.

use sim_core::build::{
    BuildContent, MAT_METAL, MAT_STONE, MAT_TWIG, MAT_WOOD, SHAPE_DOORWAY, SHAPE_FLOOR,
    SHAPE_FOUNDATION, SHAPE_FRAME, SHAPE_ROOF, SHAPE_STAIRS, SHAPE_WALL, SHAPE_WINDOW,
};
use sim_core::craft::inv_count;
use sim_core::gather::ItemStack;
use sim_core::limits::INV_SLOTS;

/// The outer ring, clockwise from the top. Order is the build order — you
/// need a foundation before a wall and a wall before a roof — so the wheel
/// reads as the sequence a base is actually built in; the two openings
/// (catalogue v1) sit between the doorway and the floor, which is where a
/// builder reaches for them.
pub const SHAPES: [u8; 8] = [
    SHAPE_FOUNDATION,
    SHAPE_WALL,
    SHAPE_DOORWAY,
    SHAPE_WINDOW,
    SHAPE_FRAME,
    SHAPE_FLOOR,
    SHAPE_STAIRS,
    SHAPE_ROOF,
];

/// The middle ring, clockwise from the top: the upgrade ladder in order, so
/// "further round" is always "stronger". Twig leads it since twig v0 — it is
/// the rung every piece is placed at, so a ladder that started at wood would
/// be missing the only rung the blueprint can actually reach.
pub const MATERIALS: [u8; 4] = [MAT_TWIG, MAT_WOOD, MAT_STONE, MAT_METAL];

pub fn shape_label(shape: u8) -> &'static str {
    match shape {
        SHAPE_FOUNDATION => "Foundation",
        SHAPE_WALL => "Wall",
        SHAPE_DOORWAY => "Doorway",
        SHAPE_WINDOW => "Window",
        SHAPE_FRAME => "Wall Frame",
        SHAPE_FLOOR => "Floor",
        SHAPE_STAIRS => "Stairs",
        SHAPE_ROOF => "Roof",
        _ => "Piece",
    }
}

/// The centre's one-line blurb. The reference wheel carries one per shape
/// and it is the only place a new player learns what a piece is *for*.
pub fn shape_blurb(shape: u8) -> &'static str {
    match shape {
        SHAPE_FOUNDATION => "The ground floor. Everything else stands on one.",
        SHAPE_WALL => "Secure your base by enclosing it in walls.",
        SHAPE_DOORWAY => "A wall with a hole for a door. The way in.",
        SHAPE_WINDOW => "A wall with an eye. Arrows pass; bodies never do.",
        SHAPE_FRAME => "A wall that is mostly hole, until an insert fills it.",
        SHAPE_FLOOR => "A storey's ceiling and the next one's ground.",
        SHAPE_STAIRS => "Reach the storey above.",
        SHAPE_ROOF => "Cap the top so nothing builds above you.",
        _ => "",
    }
}

/// The icon file for a shape. Separate from [`shape_label`] because a label
/// is prose and a stem is an asset name.
///
/// **Here rather than beside the draw** (where it lived until the hammer
/// wheel got its own glyphs) for `ui::icons`' stated reason: a stem map can
/// be silently wrong, and being wrong draws an empty wedge, so it belongs
/// where a headless test can check it against what `assets/icons/` ships.
/// `tests/ui.rs` §G does.
pub fn shape_icon(shape: u8) -> &'static str {
    match shape {
        SHAPE_FOUNDATION => "shape_foundation",
        SHAPE_WALL => "shape_wall",
        SHAPE_DOORWAY => "shape_doorway",
        SHAPE_WINDOW => "shape_window",
        SHAPE_FRAME => "shape_wall_frame",
        SHAPE_FLOOR => "shape_floor",
        SHAPE_STAIRS => "shape_stairs",
        _ => "shape_roof",
    }
}

pub fn material_label(material: u8) -> &'static str {
    match material {
        MAT_TWIG => "Twig",
        MAT_WOOD => "Wood",
        MAT_STONE => "Stone",
        MAT_METAL => "Metal",
        _ => "?",
    }
}

/// **The material a blueprint places in, and it is not a choice.**
///
/// **Twig since twig v0 (2026-08-10).** This constant did not move — it is
/// still `MATERIALS[0]` — but the rung it names is now a real scaffold grade
/// rather than the wood one, and `sim_core::build::place` refuses anything
/// else, so the pin below stopped being a client-side courtesy and became
/// the rule. `reference/BUILDING.md` §7b.4.
///
/// The reference game's building plan places one thing — the cheapest rung —
/// and the *hammer* is what walks a standing piece up the ladder. Ours had a
/// second ring on the wheel for picking the material at placement time, which
/// is a verb the reference does not have and which merged two of its menus
/// into one (operator, 2026-08-07: *"you dont upgrade from holding blueprint
/// … upgrade is the hammer you hold"*, then *"just remove the inner ring"*).
///
/// **Nothing is lost by pinning it.** `sim_core::build::upgrade` has existed
/// since the build slice, `ACT_UPGRADE` is on the wire, and the client
/// already sends it — `U` takes the piece you are looking at one rung up via
/// `structure::next_material`. So stone and metal are reached the way the
/// reference reaches them, by upgrading, rather than by being placed
/// directly. What is still owed is that `U` should be a hammer wheel;
/// `NOW.md` §0p2 carries it.
pub const PLACE_MATERIAL: u8 = MATERIALS[0];

/// The wheel's two radii, in pixels of the drawn wheel. Proposed defaults
/// (`DECISIONS.md` §open, "build wheel v0").
///
/// **One ring since 2026-08-07** — see [`PLACE_MATERIAL`]. The inner radius
/// is `0.66 × rim`, which is the ratio measured off the reference
/// `building.jpeg` (the ring located programmatically, not by eye), and the
/// dead centre it leaves is far roomier than the old 96 — which the readout
/// needed, because five lines in a 192 px circle was already tight.
#[derive(Clone, Copy, Debug)]
pub struct Rings {
    /// Inside this is the readout, and dead.
    pub dead: f32,
    /// The rim. Outside it the pointer has left the wheel.
    pub rim: f32,
}

impl Default for Rings {
    fn default() -> Self {
        Self {
            dead: 141.0,
            rim: 214.0,
        }
    }
}

/// Which shape segment the pointer is in, or `None` for the dead centre and
/// anywhere past the rim — both of which release without choosing.
///
/// `dx` is right-positive and **`dy` is up-positive** — mathematical
/// orientation, not Bevy UI's downward y. The render layer negates once at
/// the call site, which keeps the trigonometry here readable and testable
/// without a windowing convention baked into it.
///
/// Segment 0 is centred on **straight up**, and indices increase
/// **clockwise**, which is what `dx.atan2(dy)` gives directly.
pub fn pick(dx: f32, dy: f32, rings: Rings) -> Option<usize> {
    pick_in(dx, dy, rings, SHAPES.len())
}

/// [`pick`] over a band of `n` segments. The shape wheel and the hammer's
/// wheel ([`super::hammer`]) are the same annulus with different segment
/// counts, so the radius test lives once — two copies of "is the pointer on
/// the ring" is how two wheels come to disagree about where the ring is.
pub fn pick_in(dx: f32, dy: f32, rings: Rings, n: usize) -> Option<usize> {
    let r = (dx * dx + dy * dy).sqrt();
    if r < rings.dead || r > rings.rim {
        return None;
    }
    Some(segment(dx, dy, n))
}

/// The segment index of a direction, in a ring of `n`, segment 0 centred up
/// and increasing clockwise.
///
/// Exposed because the drawing code needs the same mapping to place labels,
/// and two implementations of "which slice is this" is precisely how a wheel
/// ends up highlighting one segment and placing another.
pub fn segment(dx: f32, dy: f32, n: usize) -> usize {
    debug_assert!(n > 0);
    let step = std::f32::consts::TAU / n as f32;
    // atan2(x, y) is 0 at +y (up) and grows clockwise, which is the wheel's
    // own convention — so nothing here has to be flipped or offset except
    // the half-step that centres segment 0 rather than starting it at up.
    let theta = dx.atan2(dy) + step * 0.5;
    let turns = theta.rem_euclid(std::f32::consts::TAU) / step;
    (turns as usize).min(n - 1)
}

/// The angle a segment's centre points at, radians clockwise from up. What
/// the drawing code places an icon along.
pub fn segment_angle(index: usize, n: usize) -> f32 {
    let step = std::f32::consts::TAU / n.max(1) as f32;
    index as f32 * step
}

/// The baked piece row for a (shape, material) pair, or `None` if the
/// content has no such piece.
///
/// **A search, not arithmetic** — see the module note. `None` is a real
/// answer: a shard whose content ships wood walls and no metal ones has a
/// wheel segment with nothing behind it, and the right thing to draw is a
/// dead segment rather than a live one that places the wrong piece.
pub fn row_for(content: &BuildContent, shape: u8, material: u8) -> Option<u16> {
    content
        .pieces
        .iter()
        .take(content.piece_count as usize)
        .position(|p| p.hp > 0 && p.shape == shape && p.material == material)
        .and_then(|i| u16::try_from(i).ok())
}

/// One line of the centre readout's price. Same shape as the craft panel's
/// ingredient row and for the same reason: a cost the player cannot pay is
/// drawn short rather than hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub item: u16,
    pub units: u16,
    pub have: u32,
}

impl Cost {
    pub fn short(&self) -> bool {
        self.have < self.units as u32
    }
}

/// The costs of a piece row against the inventory. Fixed array plus a live
/// count — `MAX_PIECE_COSTS` is two, so there is nothing here to allocate.
pub fn costs(
    content: &BuildContent,
    row: u16,
    inv: &[ItemStack; INV_SLOTS],
) -> ([Cost; sim_core::limits::MAX_PIECE_COSTS], usize) {
    let mut out = [Cost {
        item: 0,
        units: 0,
        have: 0,
    }; sim_core::limits::MAX_PIECE_COSTS];
    let Some(def) = content.pieces.get(row as usize) else {
        return (out, 0);
    };
    let n = (def.n_costs as usize).min(sim_core::limits::MAX_PIECE_COSTS);
    for (slot, (item, units)) in out.iter_mut().zip(def.costs.iter()).take(n) {
        *slot = Cost {
            item: *item,
            units: *units,
            have: inv_count(inv, *item),
        };
    }
    (out, n)
}

/// Can the player pay for this row right now? The wheel dims a segment it
/// cannot afford, exactly as the craft panel dims a recipe — the sim is
/// still the authority, and a placement it refuses comes back
/// `REFUSE_B_COST`.
pub fn affordable(content: &BuildContent, row: u16, inv: &[ItemStack; INV_SLOTS]) -> bool {
    let (costs, n) = costs(content, row, inv);
    costs.iter().take(n).all(|c| !c.short())
}
