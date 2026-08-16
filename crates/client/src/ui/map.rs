//! The island map: the palette, the hillshade, and the two positional facts
//! that make it right side up.
//!
//! Ported from `web/src/map.js`, and pure for its reasons plus one of its
//! own — **the whole file is positional payload**. `CLAUDE.md` is explicit
//! that a byte-golden is blind to what a field means and that positional
//! payloads are where the reference ecosystem actually bled; a map has no
//! golden at all, and getting a sign wrong produces a picture that is right
//! about everything and upside down. So [`paint`] is a function a headless
//! test can call and read the pixels back out of.
//!
//! ## The palette is a claim, not a starting point
//!
//! Read off the reference `mapraw.jpg`, the one reference frame that is a map
//! and nothing else. The four ground colours are close in VALUE and far apart
//! in HUE deliberately: the reference map reads as terrain because relief does
//! the work and the biome tint is a wash over it, so a palette with strong
//! value contrast would fight the hillshade and flatten the island. Water is
//! the exception and is meant to be — a coastline you cannot find at a glance
//! is the whole failure this screen exists to fix.

use protocol::event::WireBag;
use sim_core::build::BUILD_CELL_M;
use sim_core::deploy::{BagAnchor, DeployContent, DeployRec, ARCH_BAG, ARCH_HEARTH};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::{self, Haven, ISLAND_SIZE, SEA_LEVEL};

/// Metres per grid square. 2048 / 128 = 16 squares a side.
pub const GRID_M: f32 = 128.0;
/// Grid squares per side.
pub const GRID_COLS: usize = (ISLAND_SIZE / GRID_M) as usize;
/// Column letters, west to east — one per column, asserted rather than
/// trusted.
pub const GRID_LETTERS: &str = "ABCDEFGHIJKLMNOP";

/// How deep the water ramp runs before it is all floor colour, metres.
pub const DEEP_M: f32 = 12.0;

/// Beach and dune.
pub const SAND: [f32; 3] = [203.0, 185.0, 148.0];
/// Open meadow.
pub const GRASS: [f32; 3] = [109.0, 140.0, 74.0];
/// Forest litter: the same green, darker — how the reference separates wooded
/// from open ground without a second hue.
pub const LITTER: [f32; 3] = [78.0, 107.0, 58.0];
/// Rock: the alpine band and every cliff the splat law vetoes to rock.
pub const ROCK: [f32; 3] = [168.0, 152.0, 120.0];
/// The shelf, just off the beach.
pub const SEA_SHALLOW: [f32; 3] = [47.0, 106.0, 142.0];
/// Open water at [`DEEP_M`] and below.
pub const SEA_DEEP: [f32; 3] = [23.0, 50.0, 74.0];

/// The hillshade's light, as a unit vector in world axes (**x WEST**, y up,
/// z north).
///
/// Up and to the LEFT of the image, which is north-west — the convention
/// every printed relief map uses and the one `mapraw.jpg` follows. The map's
/// north is `+z` and its east is `-x` (`DECISIONS.md` 2026-08-15 — the same
/// call the compass strip rests on), so north-west in world axes is
/// `+x, +z` and **both terms here are positive**.
///
/// Both signs are the bearing readout's fact restated in a second place,
/// which is why they moved in the 2026-08-15 commit rather than after it: a
/// map whose x term flipped and whose light did not would be lit from the
/// north-EAST, and nothing but a person looking at it would say so.
pub const LIGHT: [f32; 3] = [0.5, std::f32::consts::FRAC_1_SQRT_2, 0.5];

/// The lambert term becomes a gain: `SHADE_FLOOR + SHADE_GAIN * lambert`,
/// clamped.
///
/// **Derived, not chosen.** Flat ground has normal `(0, 1, 0)` and therefore
/// lambert `LIGHT[1]`, and flat ground must come out at the palette's own
/// colour — that is what makes the palette above a measurable claim. So
/// `floor + gain * LIGHT[1] == 1`. Fixing the gain at 0.6 (enough that a 20 m
/// ridge reads, little enough that a cliff face reaches the clamp rather than
/// black) gives the floor. The identity is asserted below rather than
/// computed here, so the assertion is not a tautology.
/// `map.js` carries this as `0.5757359312880714`, which is the `f64` the
/// identity gives. This is that number at `f32`, which is the precision the
/// browser's own canvas fill sees it at anyway.
pub const SHADE_GAIN: f32 = 0.6;
pub const SHADE_FLOOR: f32 = 0.575_735_9;
pub const SHADE_CLAMP: (f32, f32) = (0.45, 1.35);

/// The grid square a world position is in — `"H11"`, or `""` off the island.
///
/// Columns are letters west to east; rows are numbers north to south from 1.
/// **Both directions are load-bearing and both count DOWN**: our north is
/// `+z` and our east is `-x` (`DECISIONS.md` 2026-08-15), so row 1 is the
/// `+z` edge and column A is the `+x` edge — the same two flips [`paint`]
/// applies to the image, which is why they live in one file.
pub fn grid_label(x: f32, z: f32) -> String {
    if !(0.0..ISLAND_SIZE).contains(&x) || !(0.0..ISLAND_SIZE).contains(&z) {
        return String::new();
    }
    let col = (((ISLAND_SIZE - x) / GRID_M) as usize).min(GRID_COLS - 1);
    let row = (((ISLAND_SIZE - z) / GRID_M) as usize).min(GRID_COLS - 1);
    let letter = GRID_LETTERS.as_bytes()[col] as char;
    format!("{letter}{}", row + 1)
}

/// Where a world position lands on a `size`-pixel map, in pixels.
///
/// By continuous EXTENT, where [`paint`]'s flip is by sample INDEX — and the
/// only thing that makes the two agree is where the samples sit. See
/// [`paint`].
pub fn world_to_map(x: f32, z: f32, size: usize) -> (f32, f32) {
    let s = size as f32;
    (
        // East is -x and image columns grow east, so this flips too — a
        // north-up map has east on the right only if east is `-x`.
        (ISLAND_SIZE - x) / ISLAND_SIZE * s,
        // North is +z and image rows grow south, so this flips.
        (ISLAND_SIZE - z) / ISLAND_SIZE * s,
    )
}

/// Paint the island into an RGBA buffer, `size × size`.
///
/// **Three positional facts, each of which has cost a pass somewhere.**
///
/// 1. Sample row `j` grows with `+z`, which is NORTH, and image row `py`
///    grows DOWNWARD, which is SOUTH. So `py = size - 1 - j`. Get it wrong
///    and the map is right about everything and upside down.
/// 2. Sample column `i` grows with `+x`, which is **WEST**
///    (`DECISIONS.md` 2026-08-15), and image column `px` grows RIGHTWARD,
///    which is east. So `px = size - 1 - i`, the same flip on the other
///    axis. Get *this* one wrong and the map is right about everything and
///    mirrored — which is what it was until 2026-08-15, undetectably,
///    because it agreed with a compass that was mirrored too.
/// 3. **Samples sit at pixel CENTRES.** A fill starting at 0 puts sample
///    `(i, j)` on the low-x, low-z corner of the pixel it fills; the pixel's
///    extent then runs from that sample to the next, so the painted island is
///    half a cell out on both axes — and on the flipped axis `world_to_map`
///    returns the boundary between two rows, and `floor` takes the southern
///    one every time. Sampling at centres puts `(i, j)` strictly inside the
///    pixel it painted, on both axes, with no boundary to tip over.
///
/// One pass, ~`size²` height taps plus an apron. Called once per session on
/// the first open, never per frame: the island is a function of the seed and
/// the seed does not change inside a session.
pub fn paint(seed: u64, size: usize, out: &mut [u8]) {
    assert!(out.len() >= size * size * 4, "buffer too small for the map");
    let step = ISLAND_SIZE / size as f32;
    let half = step * 0.5;
    let inv2s = 1.0 / (2.0 * step);

    for j in 0..size {
        let z = half + j as f32 * step;
        let py = size - 1 - j;
        for i in 0..size {
            let x = half + i as f32 * step;
            let px = size - 1 - i;
            let h = terrain::height(seed, x, z);

            // Central differences at the map's own step. A smoothed slope
            // rather than `terrain::slope`'s ±1 m one — which is correct for a
            // map, and is also what the ground mesh feeds the splat law at its
            // own step, so the map and the world disagree no more than two
            // chunk resolutions of the world already do.
            let dhdx =
                (terrain::height(seed, x + step, z) - terrain::height(seed, x - step, z)) * inv2s;
            let dhdz =
                (terrain::height(seed, x, z + step) - terrain::height(seed, x, z - step)) * inv2s;

            let (r, g, b) = if h <= SEA_LEVEL {
                // Water: one ramp from shelf to floor, and NO hillshade. The
                // sea bed has relief and drawing it would put a second,
                // competing terrain inside the shape the eye is trying to read
                // as the coastline.
                let t = (-h / DEEP_M).clamp(0.0, 1.0);
                (
                    SEA_SHALLOW[0] + (SEA_DEEP[0] - SEA_SHALLOW[0]) * t,
                    SEA_SHALLOW[1] + (SEA_DEEP[1] - SEA_SHALLOW[1]) * t,
                    SEA_SHALLOW[2] + (SEA_DEEP[2] - SEA_SHALLOW[2]) * t,
                )
            } else {
                let slope = (dhdx * dhdx + dhdz * dhdz).sqrt();
                let w = terrain::splat_from(h, terrain::moisture(seed, x, z), slope);
                // They sum to ~255 by construction, and "~" is the splat law's
                // own word for it — dividing by the actual sum keeps the blend
                // a true average whatever the rounding did.
                let sum = (w[0] as f32 + w[1] as f32 + w[2] as f32 + w[3] as f32).max(1.0);
                let mix = |c: usize| {
                    (SAND[c] * w[0] as f32
                        + GRASS[c] * w[1] as f32
                        + LITTER[c] * w[2] as f32
                        + ROCK[c] * w[3] as f32)
                        / sum
                };
                // Hillshade. A heightfield's normal is `(-dh/dx, 1, -dh/dz)`
                // before normalising; the lambert term against `LIGHT` becomes
                // a multiplier, so the biome tint survives it as a tint.
                let inv = 1.0 / (dhdx * dhdx + 1.0 + dhdz * dhdz).sqrt();
                let lambert = (-dhdx * LIGHT[0] + LIGHT[1] + -dhdz * LIGHT[2]) * inv;
                let shade =
                    (SHADE_FLOOR + SHADE_GAIN * lambert).clamp(SHADE_CLAMP.0, SHADE_CLAMP.1);
                (mix(0) * shade, mix(1) * shade, mix(2) * shade)
            };

            let o = (py * size + px) * 4;
            out[o] = r.clamp(0.0, 255.0) as u8;
            out[o + 1] = g.clamp(0.0, 255.0) as u8;
            out[o + 2] = b.clamp(0.0, 255.0) as u8;
            out[o + 3] = 255;
        }
    }
}

// ---------------------------------------------------------------------------
// The markers: the destinations, and the things on the island that are yours.
//
// A port of `web/src/map.js`'s `resolveMarks` (in git history; the layer was
// lost when the map was), plus the tier the browser could not reach: the
// haven pad and the waystations. The browser's comment says why it could not
// mark them — `terrain::haven(seed)` had no bridge export — and that blocker
// dissolved when the client became Rust: `render::WorldId` memoizes the same
// `Haven` the server resolves, so the authored tier costs a read, not a wire
// change. `NOW.md` §0a's "systems lane, one export please" is this, answered.
//
// Everything else here comes off data the client already holds: the deploy
// mirror and the standing death bags, both streamed island-wide for the 3D
// scene and `interact::resolve`. Nothing new crosses the wire.
//
// What it deliberately does NOT mark: boxes, doors, fires, furnaces,
// workbenches, locks. A base is a dozen boxes and a door per room, and
// marking them buries the two anchors a player is actually looking for
// (`DECISIONS.md`, map marker cap v0 — retired with the browser, defaults
// carried forward here). The death marker stays unbuilt: an operator call
// (`ALPHA.md` §1 has the death screen's position rule), not a lane's.

/// The most markers the map will draw. A wall (`CLAUDE.md` §4: bounded
/// everything, with a stated overflow policy), not a budget a player meets —
/// the marked archetypes are the two a base has one of, so a real map draws
/// a handful. Overflow policy: drop-newest, and the refused count is kept on
/// [`Marks`] rather than discarded — a cap that truncates silently reads as
/// "everything is drawn" when it is not. `DECISIONS.md` §open, map markers v1.
pub const MAP_MARKS_MAX: usize = 64;
/// …and a full rack of bags fits inside it with room to spare, so
/// [`resolve_wake_marks`]' drop is unreachable by an ordinary player.
/// Compile-time rather than a test for `protocol`'s reason: a cap that is
/// only correct while another crate's constant stays put is not a cap, and
/// raising `BAG_CAP` past this would make the death screen silently stop
/// drawing one of a player's own beds.
const _: () = assert!(
    sim_core::deploy::BAG_CAP <= MAP_MARKS_MAX,
    "BAG_CAP outgrew the map's marker cap — a player's own bag would be dropped"
);

/// What a mark is a mark OF. `None` is the zero so a stale array slot cannot
/// draw a bed — [`Marks::a`] is fixed-size and reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkKind {
    #[default]
    None,
    /// The haven pad: the island's one authored destination.
    Haven,
    /// A waystation: the lesser tier of the same search.
    Waystation,
    /// A deployed sleeping bag: where you WAKE.
    Bed,
    /// One of **your own** bags whose cooldown has not lapsed — a bed the
    /// next death will skip over.
    ///
    /// Its own kind rather than a flag on [`MarkKind::Bed`] because the
    /// renderer's `match` is the gate: a state with no draw branch fails
    /// to compile, and a bool would have drawn a ready bag over a spent
    /// one with nothing to notice. Only [`resolve_wake_marks`] can produce
    /// it — readiness is an own-fact and the island-wide deploy mirror
    /// does not carry it (`protocol`'s `SUB_BAGS`).
    BedSpent,
    /// A hearth: the base's anchor, and what its upkeep is paid into.
    Hearth,
    /// A standing death backpack: where your stuff is.
    Backpack,
}

impl MarkKind {
    /// The fill, 0–255 RGB. NOT the ground palette's colours — a marker that
    /// shared one would vanish over exactly that biome, and the test below
    /// holds every fill a measured distance off every ground colour.
    ///
    /// Haven and waystation share one fill on purpose: they are one class of
    /// thing (an authored destination, tiered), and the renderer separates
    /// the tiers by size. Bed, hearth and bag are the browser layer's own
    /// colours: cool blue for the thing you sleep on, ember for fire, straw
    /// for loot.
    pub fn fill(self) -> [f32; 3] {
        match self {
            // Never drawn: `Marks::count` bounds every reader. Black, so a
            // bug that draws it anyway is visible instead of plausible.
            MarkKind::None => [0.0, 0.0, 0.0],
            MarkKind::Haven | MarkKind::Waystation => [232.0, 228.0, 218.0],
            // A spent bag is the SAME blue as a ready one: the colour says
            // "this is a bed of yours", and the renderer's shape says
            // whether it will answer. Two blues would be the one channel a
            // player reading a small panel in a hurry cannot tell apart,
            // which is the rule this whole table is built on.
            MarkKind::Bed | MarkKind::BedSpent => [127.0, 179.0, 255.0],
            MarkKind::Hearth => [255.0, 157.0, 92.0],
            MarkKind::Backpack => [232.0, 215.0, 106.0],
        }
    }
}

/// One marker, in map FRACTIONS (0..1 across the painted island) — the
/// output of [`world_to_map`] at size 1, which is how the render layer places
/// the player too, so a marker and the player cannot disagree about the
/// projection.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mark {
    pub kind: MarkKind,
    pub px: f32,
    pub py: f32,
}

/// A reusable marker set: fixed storage, a live count, and the overflow
/// count the cap refused — kept, never silent.
pub struct Marks {
    pub a: [Mark; MAP_MARKS_MAX],
    /// How many entries of `a` are live.
    pub count: usize,
    /// How many marks the cap refused. Not drawn as marks; drawn as a number.
    pub dropped: usize,
}

impl Default for Marks {
    fn default() -> Self {
        Self {
            a: [Mark::default(); MAP_MARKS_MAX],
            count: 0,
            dropped: 0,
        }
    }
}

impl Marks {
    fn push(&mut self, kind: MarkKind, x: f32, z: f32) {
        if self.count >= MAP_MARKS_MAX {
            self.dropped += 1;
            return;
        }
        // THE projection — the same call the player's own marker goes
        // through. A second copy of the north-is-up flip here is the
        // positional defect class `CLAUDE.md` names: the island correct, the
        // player correct, and your bed drawn as far south as it is north.
        let (px, py) = world_to_map(x, z, 1);
        self.a[self.count] = Mark { kind, px, py };
        self.count += 1;
    }
}

/// Fill `out` with every marker the map draws.
///
/// Push ORDER is the drop policy: authored destinations first, so the cap
/// (drop-newest) can never cost the haven; then the standing bags; then the
/// anchors off the deploy mirror.
///
/// Bags outrank the anchor tier deliberately (`NOW.md` §0die): the deploy
/// mirror is island-wide, so on a busy shard beds and hearths alone can fill
/// the cap — and a bag is the one mark on a clock, dropped newest, which
/// made your own bag the first thing the cap ate. `WireBag` carries no owner
/// (the server's broadcast drops the victim id at `WireBag::of`), so the map
/// cannot rank a STRANGER'S bag by whose it is — but the client can tag its
/// OWN (`ClientCore::own_bag`, the death-position join), and `own_bag` here
/// is that tag: the one bag pushed ahead of every other, directly behind the
/// authored tier, so it survives any cap the authored marks leave room in —
/// which is all of them, `AUTHORED + 1 < MAP_MARKS_MAX` by a wide margin.
/// Stranger bags still pay the cap against each other, newest first among
/// the marks behind them. The cap itself does not move and drop-newest
/// stays stated.
///
/// `have` is `ClientCore::deploy_defs_have`, and the guard is load-bearing:
/// an undripped row reads as `DeployDef::INERT`, whose arch is `ARCH_BAG` —
/// so without it a not-yet-known deployable is drawn as somebody's bed.
/// `interact::resolve` skips for the same reason.
pub fn resolve_marks(
    out: &mut Marks,
    haven: &Haven,
    deploys: &[DeployRec],
    defs: &DeployContent,
    have: u16,
    bags: &[WireBag],
    own_bag: u32,
) {
    out.count = 0;
    out.dropped = 0;

    out.push(MarkKind::Haven, haven.x, haven.z);
    for w in &haven.minor {
        out.push(MarkKind::Waystation, w.x, w.z);
    }

    // The own bag first (zero = none, the store's own sentinel), so the cap
    // can never eat it behind a shard's worth of strangers.
    if own_bag != 0 {
        if let Some(bag) = bags.iter().find(|b| b.id == own_bag) {
            out.push(
                MarkKind::Backpack,
                bag.qx as f32 * POS_XZ_Q,
                bag.qz as f32 * POS_XZ_Q,
            );
        }
    }
    for bag in bags {
        if own_bag != 0 && bag.id == own_bag {
            continue; // already pushed, ahead of the cap
        }
        // A bag carries a quantized world position, not a grid cell — it is
        // dropped where its owner died. y is the one term a map has no axis
        // for.
        out.push(
            MarkKind::Backpack,
            bag.qx as f32 * POS_XZ_Q,
            bag.qz as f32 * POS_XZ_Q,
        );
    }

    let have = have.min(defs.def_count);
    let half = BUILD_CELL_M * 0.5;
    for rec in deploys {
        if (rec.row as u16) >= have {
            continue;
        }
        let kind = match defs.defs[rec.row as usize].arch {
            ARCH_BAG => MarkKind::Bed,
            ARCH_HEARTH => MarkKind::Hearth,
            // Everything else is deliberately unmarked — see the module note.
            _ => continue,
        };
        // The cell CENTRE — where `interact::resolve` measures reach to and
        // where the mesh stands. A marker on the corner sits up to half a
        // cell off the thing it names.
        out.push(
            kind,
            rec.cx as f32 * BUILD_CELL_M + half,
            rec.cz as f32 * BUILD_CELL_M + half,
        );
    }
}

/// Fill `out` with **your own bags** and nothing else — the death
/// screen's map (bag choice v0).
///
/// A different function from [`resolve_marks`] rather than a flag on it,
/// and the difference is the whole point of the screen:
///
///   · [`resolve_marks`] draws the island — every bed and hearth anyone
///     placed, off the broadcast deploy mirror, which cannot say whose
///     they are;
///   · this draws the **answers to the question on screen**, off the
///     own-fact bag list, which is the only thing that can.
///
/// So no haven, no waystations, no other player's bed, and — the one that
/// is a rule rather than a choice — **no marker for where you fell.**
/// `ALPHA.md` §1: "who/what killed you — range and weapon, no map
/// position". A raider standing over your body is looking at your screen's
/// twin on their own machine; the reason the death screen never carried a
/// position is that yours would pin the base they just cleared. Adding a
/// map here does not repeal that, and `no_wake_map_marks_the_corpse` is
/// what keeps someone from helpfully adding it back.
///
/// The cap is [`resolve_marks`]'s and cannot be reached: `BAG_CAP` is 8
/// against [`MAP_MARKS_MAX`]'s 64. It is checked anyway, because a cap
/// that is only correct as long as another crate's constant does not move
/// is not a cap.
pub fn resolve_wake_marks(out: &mut Marks, bags: &[BagAnchor]) {
    out.count = 0;
    out.dropped = 0;
    let half = BUILD_CELL_M * 0.5;
    for b in bags {
        out.push(
            if b.ready {
                MarkKind::Bed
            } else {
                MarkKind::BedSpent
            },
            // The cell CENTRE, `resolve_marks`' metric — the two maps put
            // the same bag on the same pixel or one of them is lying.
            b.cx as f32 * BUILD_CELL_M + half,
            b.cz as f32 * BUILD_CELL_M + half,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_is_one_letter_per_column() {
        assert_eq!(GRID_LETTERS.len(), GRID_COLS);
        assert_eq!(GRID_COLS, 16);
    }

    /// The identity `SHADE_FLOOR` is derived from, asserted rather than
    /// computed — flat ground must come out at the palette's own colour, or
    /// the palette is not a claim about anything.
    #[test]
    fn flat_ground_shades_to_exactly_one() {
        let flat = SHADE_FLOOR + SHADE_GAIN * LIGHT[1];
        assert!((flat - 1.0).abs() < 1e-6, "flat ground shades to {flat}");
    }

    /// West is `+x` and north is `+z` (`DECISIONS.md` 2026-08-15), so both
    /// terms are positive — and the assertion names the compass point rather
    /// than the sign, so it goes red if the axes are ever re-decided without
    /// this file being read.
    #[test]
    fn the_light_is_a_unit_vector_from_the_north_west() {
        let len = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "light is not unit: {len}");
        // A point one metre toward the light, read back as a bearing: the
        // light must stand in the north-west quadrant of the same card the
        // compass strip prints. This is `LIGHT` checked against the
        // convention's owner, not against a remembered sign.
        let deg = crate::look::bearing_of(LIGHT[0], LIGHT[2]);
        assert!(
            (270.0..360.0).contains(&deg),
            "the light bears {deg}°, which is not north-west"
        );
    }

    /// Row 1 is the `+z` edge and column A is the `+x` edge — both count
    /// DOWN, because north is `+z` and east is `-x`. Both halves of
    /// `grid_label` are load-bearing.
    #[test]
    fn a_one_is_the_north_west_corner() {
        // North-west: the largest z AND the largest x.
        assert_eq!(
            grid_label(ISLAND_SIZE - 1.0, ISLAND_SIZE - 1.0),
            "A1",
            "A1 is the north-west corner"
        );
        // Due south of it: same column, last row.
        assert_eq!(grid_label(ISLAND_SIZE - 1.0, 1.0), format!("A{GRID_COLS}"));
        // Due east of it: last column, first row.
        assert_eq!(grid_label(1.0, ISLAND_SIZE - 1.0), "P1");
    }

    /// The lettering agrees with the compass: walking east must walk up the
    /// alphabet, and the direction "east" comes from `look::bearing_of`
    /// rather than from a sign written down twice.
    #[test]
    fn the_columns_run_west_to_east() {
        let mid = ISLAND_SIZE * 0.5;
        // Two steps in the direction the compass calls east (90°).
        let east = (-1.0f32, 0.0f32);
        assert!((crate::look::bearing_of(east.0, east.1) - 90.0).abs() < 0.01);
        let a = grid_label(mid, mid);
        let b = grid_label(mid + east.0 * GRID_M, mid + east.1 * GRID_M);
        assert!(
            b.as_bytes()[0] > a.as_bytes()[0],
            "a step east went {a} → {b}, which runs the alphabet backwards"
        );
    }

    #[test]
    fn a_position_off_the_island_has_no_square() {
        assert_eq!(grid_label(-1.0, 10.0), "");
        assert_eq!(grid_label(10.0, ISLAND_SIZE), "");
    }

    /// North is up and east is right — **stated as compass points and
    /// resolved through `look::bearing_of`**, not as two world coordinates
    /// with directional variable names.
    ///
    /// The line this replaces was `px_west < px_east` over `x = 1` and
    /// `x = ISLAND_SIZE - 1`, which is a monotonicity claim wearing a
    /// compass's clothes: it is green whichever way the x term runs, so it
    /// held while the map was mirrored and would have held after a fix that
    /// mirrored it back. `NOW.md` §0gj named it; this is the version that
    /// can fail.
    #[test]
    fn the_map_is_north_up_and_east_right() {
        let c = ISLAND_SIZE * 0.5;
        let step = 100.0;
        for (name, want) in [
            ("north", 0.0f32),
            ("east", 90.0),
            ("south", 180.0),
            ("west", 270.0),
        ] {
            // The unit direction the compass gives that name, taken from the
            // owner rather than assumed here.
            let (dx, dz) = match name {
                "north" => (0.0, 1.0),
                "east" => (-1.0, 0.0),
                "south" => (0.0, -1.0),
                _ => (1.0, 0.0),
            };
            assert!(
                (crate::look::bearing_of(dx, dz) - want).abs() < 0.01,
                "{name} is not {want}° on the card"
            );
            let (px0, py0) = world_to_map(c, c, 256);
            let (px1, py1) = world_to_map(c + dx * step, c + dz * step, 256);
            match name {
                "north" => assert!(py1 < py0, "north must move UP the image"),
                "south" => assert!(py1 > py0, "south must move DOWN the image"),
                "east" => assert!(px1 > px0, "east must move RIGHT across the image"),
                _ => assert!(px1 < px0, "west must move LEFT across the image"),
            }
            // And the other axis must not move at all.
            if dx == 0.0 {
                assert_eq!(px0, px1, "{name} moved the column");
            } else {
                assert_eq!(py0, py1, "{name} moved the row");
            }
        }
    }

    /// The painted image and `world_to_map` must agree about which pixel a
    /// position is in — the half-cell bug the browser paid a pass for. A
    /// position sampled at a pixel centre must round-trip to that pixel.
    #[test]
    fn a_pixel_centre_round_trips_to_its_own_pixel() {
        let size = 64;
        let step = ISLAND_SIZE / size as f32;
        let half = step * 0.5;
        for (i, j) in [(0usize, 0usize), (17, 5), (63, 63), (31, 32)] {
            let (x, z) = (half + i as f32 * step, half + j as f32 * step);
            let (px, py) = world_to_map(x, z, size);
            assert_eq!(
                px.floor() as usize,
                size - 1 - i,
                "column for sample ({i},{j})"
            );
            assert_eq!(
                py.floor() as usize,
                size - 1 - j,
                "row for sample ({i},{j})"
            );
        }
    }

    /// The map is not upside down. Painted at a real seed, the northern half
    /// and the southern half must not be interchangeable — and specifically,
    /// the pixel `world_to_map` names for a known-land position must not be
    /// sea colour while the mirrored one is land.
    #[test]
    fn the_painted_island_matches_where_world_to_map_says_things_are() {
        let seed = 42u64;
        let size = 96usize;
        let mut buf = vec![0u8; size * size * 4];
        paint(seed, size, &mut buf);

        // Find a land sample and a sea sample by asking the terrain directly,
        // then check the painted pixel at each agrees about which it is.
        let step = ISLAND_SIZE / size as f32;
        let half = step * 0.5;
        let mut checked_land = 0;
        let mut checked_sea = 0;
        for j in (0..size).step_by(7) {
            for i in (0..size).step_by(7) {
                let (x, z) = (half + i as f32 * step, half + j as f32 * step);
                let h = terrain::height(seed, x, z);
                let (px, py) = world_to_map(x, z, size);
                let o = ((py as usize).min(size - 1) * size + (px as usize).min(size - 1)) * 4;
                let (r, g, b) = (buf[o] as f32, buf[o + 1] as f32, buf[o + 2] as f32);
                // Sea is the only thing here that is bluer than it is red.
                let bluish = b > r + 10.0;
                if h <= SEA_LEVEL {
                    assert!(bluish, "sea at ({x},{z}) painted {r},{g},{b}");
                    checked_sea += 1;
                } else {
                    assert!(!bluish, "land at ({x},{z}) painted {r},{g},{b}");
                    checked_land += 1;
                }
            }
        }
        // A vacuous pass is the failure mode: an island with no sea, or a
        // frame with no land, would satisfy every assertion above.
        assert!(checked_land > 20, "only {checked_land} land samples");
        assert!(checked_sea > 20, "only {checked_sea} sea samples");
    }

    #[test]
    fn every_pixel_is_opaque_and_none_is_left_black() {
        let size = 48;
        let mut buf = vec![0u8; size * size * 4];
        paint(7, size, &mut buf);
        for p in buf.chunks_exact(4) {
            assert_eq!(p[3], 255, "a transparent pixel");
            assert!(
                p[0] as u16 + p[1] as u16 + p[2] as u16 > 30,
                "an unpainted pixel"
            );
        }
    }

    // ---- the markers ----------------------------------------------------

    use sim_core::deploy::{DeployDef, ARCH_BOX, ARCH_DOOR, ARCH_FIRE};
    use sim_core::terrain::WAYSTATIONS;

    /// `1 + WAYSTATIONS` authored marks lead every resolve.
    const AUTHORED: usize = 1 + WAYSTATIONS;

    fn defs_with(arches: &[u8]) -> (DeployContent, u16) {
        let mut d = DeployContent::EMPTY;
        for (i, &arch) in arches.iter().enumerate() {
            d.defs[i] = DeployDef {
                arch,
                ..DeployDef::INERT
            };
        }
        d.def_count = arches.len() as u16;
        (d, arches.len() as u16)
    }

    fn rec_at(cx: u16, cz: u16, row: u8) -> DeployRec {
        DeployRec {
            cx,
            cz,
            row,
            ..DeployRec::default()
        }
    }

    /// The authored tier is real, first, and lands where the terrain says the
    /// pad is — through the SAME projection everything else uses. This is the
    /// `NOW.md` §0a item: the one authored destination was unfindable.
    #[test]
    fn the_haven_leads_and_lands_on_its_own_pixel() {
        let haven = terrain::haven(42);
        let (defs, have) = defs_with(&[]);
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &[], &defs, have, &[], 0);

        assert_eq!(out.count, AUTHORED, "haven + {WAYSTATIONS} waystations");
        assert_eq!(out.dropped, 0);
        assert_eq!(out.a[0].kind, MarkKind::Haven);
        let (px, py) = world_to_map(haven.x, haven.z, 1);
        assert_eq!((out.a[0].px, out.a[0].py), (px, py));
        // On the island, strictly — a fraction at 0 or 1 is a pad in the sea.
        for m in &out.a[..out.count] {
            assert!(
                m.px > 0.0 && m.px < 1.0,
                "{:?} off the map: {}",
                m.kind,
                m.px
            );
            assert!(
                m.py > 0.0 && m.py < 1.0,
                "{:?} off the map: {}",
                m.kind,
                m.py
            );
        }
        for (k, m) in out.a[1..AUTHORED].iter().enumerate() {
            assert_eq!(m.kind, MarkKind::Waystation);
            let w = &haven.minor[k];
            assert_eq!((m.px, m.py), world_to_map(w.x, w.z, 1));
        }
    }

    /// A bed marks the cell CENTRE — `interact::resolve`'s metric — and a bag
    /// marks its dequantized drop position, both through `world_to_map`. The
    /// bag lands FIRST after the authored tier — the rank the cap enforces.
    #[test]
    fn anchors_mark_where_the_thing_stands() {
        let haven = terrain::haven(42);
        let (defs, have) = defs_with(&[ARCH_BAG, ARCH_HEARTH]);
        let deploys = [rec_at(100, 200, 0), rec_at(300, 400, 1)];
        let bags = [WireBag {
            id: 9,
            qx: 40_000,
            qy: 123_456, // y has no map axis; a wild value must not matter
            qz: 20_000,
        }];
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &deploys, &defs, have, &bags, 0);

        assert_eq!(out.count, AUTHORED + 3);
        let half = BUILD_CELL_M * 0.5;
        let bag = &out.a[AUTHORED];
        assert_eq!(bag.kind, MarkKind::Backpack);
        assert_eq!(
            (bag.px, bag.py),
            world_to_map(40_000.0 * POS_XZ_Q, 20_000.0 * POS_XZ_Q, 1)
        );
        let bed = &out.a[AUTHORED + 1];
        assert_eq!(bed.kind, MarkKind::Bed);
        assert_eq!(
            (bed.px, bed.py),
            world_to_map(100.0 * BUILD_CELL_M + half, 200.0 * BUILD_CELL_M + half, 1)
        );
        assert_eq!(out.a[AUTHORED + 2].kind, MarkKind::Hearth);
    }

    /// Boxes, doors and fires stay off the map — the marked set is the two
    /// anchors a base has one of, not everything a base is made of.
    #[test]
    fn unmarked_archetypes_stay_off_the_map() {
        let haven = terrain::haven(42);
        let (defs, have) = defs_with(&[ARCH_BOX, ARCH_DOOR, ARCH_FIRE]);
        let deploys = [rec_at(10, 10, 0), rec_at(11, 10, 1), rec_at(12, 10, 2)];
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &deploys, &defs, have, &[], 0);
        assert_eq!(out.count, AUTHORED, "only the authored tier");
    }

    /// An undripped row reads as `DeployDef::INERT`, whose arch is `ARCH_BAG`
    /// — so without the `have` guard a deployable this client has not been
    /// told about yet is drawn as somebody's bed.
    #[test]
    fn a_row_past_the_drip_is_not_guessed_at() {
        let haven = terrain::haven(42);
        // The defs SAY bag at row 0, but the drip has delivered nothing.
        let (defs, _) = defs_with(&[ARCH_BAG]);
        let deploys = [rec_at(10, 10, 0)];
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &deploys, &defs, 0, &[], 0);
        assert_eq!(out.count, AUTHORED, "an unknown row must not mark");
    }

    /// The cap drops NEWEST, counts what it dropped, and can never cost the
    /// authored tier, which pushes first.
    #[test]
    fn the_cap_drops_newest_and_says_so() {
        let haven = terrain::haven(42);
        let (defs, have) = defs_with(&[]);
        let over = 20;
        let bags: Vec<WireBag> = (0..MAP_MARKS_MAX + over)
            .map(|i| WireBag {
                id: i as u32,
                qx: 30_000 + i as i32,
                qy: 0,
                qz: 30_000,
            })
            .collect();
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &[], &defs, have, &bags, 0);

        assert_eq!(out.count, MAP_MARKS_MAX);
        assert_eq!(out.dropped, AUTHORED + over);
        assert_eq!(out.a[0].kind, MarkKind::Haven, "authored is never dropped");
        // A second resolve on the same `Marks` starts clean — the reuse bug
        // the browser gated (an accumulating set draws yesterday's bags).
        resolve_marks(&mut out, &haven, &[], &defs, have, &[], 0);
        assert_eq!(out.count, AUTHORED);
        assert_eq!(out.dropped, 0);
    }

    /// `NOW.md` §0die: on a busy shard the deploy mirror alone can fill the
    /// cap, and bags used to push LAST — so your own bag, arriving newest,
    /// was the first mark the cap ate. `WireBag` carries no owner (the
    /// server's broadcast drops the victim id at `WireBag::of`), so the map
    /// cannot rank YOUR bag above a stranger's; what it can do is rank every
    /// bag above the anchor tier, and this holds it: the cap full of
    /// strangers' beds, the owner's bag arriving last, and the bag survives.
    #[test]
    fn a_bag_outranks_a_wall_of_strangers_beds() {
        let haven = terrain::haven(42);
        let (defs, have) = defs_with(&[ARCH_BAG]);
        // Enough beds to fill the cap on their own.
        let deploys: Vec<DeployRec> = (0..MAP_MARKS_MAX).map(|i| rec_at(i as u16, 7, 0)).collect();
        // The owner's bag: dropped newest, handed to the resolve last.
        let bags = [WireBag {
            id: 1,
            qx: 40_000,
            qy: 0,
            qz: 20_000,
        }];
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &deploys, &defs, have, &bags, 0);

        assert_eq!(out.count, MAP_MARKS_MAX);
        assert_eq!(out.dropped, AUTHORED + 1 + deploys.len() - MAP_MARKS_MAX);
        let n = out.a[..out.count]
            .iter()
            .filter(|m| m.kind == MarkKind::Backpack)
            .count();
        assert_eq!(n, 1, "the bag must survive a cap full of beds");
    }

    /// The whole of §0die's ask, now that the client can tag its own bag:
    /// a cap-full of STRANGER BAGS — the one flood the bags-above-anchors
    /// rank cannot beat — and the owner's, dropped newest and handed to
    /// the resolve last, still survives, because the tagged bag is pushed
    /// directly behind the authored tier. Red under the untagged ordering
    /// (own_bag = 0 drops it: the newest bag is the first one eaten).
    #[test]
    fn the_owners_own_bag_outranks_a_shard_of_strangers_bags() {
        let haven = terrain::haven(42);
        let (defs, _) = defs_with(&[]);
        let mut bags: Vec<WireBag> = (0..MAP_MARKS_MAX)
            .map(|i| WireBag {
                id: 100 + i as u32,
                qx: 30_000 + i as i32,
                qy: 0,
                qz: 30_000,
            })
            .collect();
        bags.push(WireBag {
            id: 7,
            qx: 40_000,
            qy: 0,
            qz: 20_000,
        });
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &[], &defs, 0, &bags, 7);

        assert_eq!(out.count, MAP_MARKS_MAX);
        let (px, py) = world_to_map(40_000.0 * POS_XZ_Q, 20_000.0 * POS_XZ_Q, 1);
        assert!(
            out.a[..out.count].iter().any(|m| (m.px, m.py) == (px, py)),
            "the owner's bag must survive a cap full of strangers' bags"
        );
    }

    /// The rank is not an exemption: bags still pay the cap against each
    /// other. A shard with more standing bags than slots drops the newest
    /// bag — a stranger's, the owner's, the policy cannot tell and does not
    /// bend (wall 4: the cap does not move, drop-newest stays stated).
    #[test]
    fn a_strangers_bag_is_still_cappable() {
        let haven = terrain::haven(42);
        let (defs, have) = defs_with(&[]);
        let bags: Vec<WireBag> = (0..MAP_MARKS_MAX + 1)
            .map(|i| WireBag {
                id: i as u32,
                qx: 30_000 + i as i32,
                qy: 0,
                qz: 30_000,
            })
            .collect();
        let mut out = Marks::default();
        resolve_marks(&mut out, &haven, &[], &defs, have, &bags, 0);

        assert_eq!(out.count, MAP_MARKS_MAX);
        assert_eq!(out.dropped, AUTHORED + 1, "the overflow is counted");
        // Drop-newest with the authored tier pushed first: the last
        // AUTHORED + 1 bags are all refused; the very last is the cheapest
        // to name, so the assert names it.
        let last = bags.last().unwrap();
        let (px, py) = world_to_map(last.qx as f32 * POS_XZ_Q, last.qz as f32 * POS_XZ_Q, 1);
        assert!(
            !out.a[..out.count].iter().any(|m| (m.px, m.py) == (px, py)),
            "drop-newest means the LAST bag is the one refused"
        );
    }

    /// No marker fill may sit near a ground colour — a marker that shares
    /// one vanishes over exactly that biome. Measured, not eyeballed: every
    /// live kind keeps at least one channel 40 steps off every ground colour.
    #[test]
    fn marker_fills_are_not_the_ground_palette() {
        let grounds = [SAND, GRASS, LITTER, ROCK, SEA_SHALLOW, SEA_DEEP];
        for kind in [
            MarkKind::Haven,
            MarkKind::Waystation,
            MarkKind::Bed,
            MarkKind::BedSpent,
            MarkKind::Hearth,
            MarkKind::Backpack,
        ] {
            let f = kind.fill();
            for g in grounds {
                let apart = (0..3).any(|c| (f[c] - g[c]).abs() >= 40.0);
                assert!(apart, "{kind:?} fill {f:?} vanishes over {g:?}");
            }
        }
    }

    /// The zero of the marker array is `None` — a stale slot cannot draw a
    /// bed, and `None` is black so drawing it anyway would be visible.
    #[test]
    fn a_stale_slot_is_nothing() {
        assert_eq!(MarkKind::default(), MarkKind::None);
        assert_eq!(Mark::default().kind, MarkKind::None);
        assert_eq!(MarkKind::None.fill(), [0.0, 0.0, 0.0]);
    }

    // ---- the death screen's map (bag choice v0) --------------------------

    fn anchor(cx: u16, cz: u16, ready: bool) -> BagAnchor {
        BagAnchor {
            cx,
            cz,
            level: 0,
            ready,
        }
    }

    /// A ready bag and a spent one are different KINDS, so the renderer's
    /// exhaustive match has to draw them differently — a bool would have
    /// drawn one over the other with nothing to notice.
    #[test]
    fn a_spent_bag_is_not_the_same_mark_as_a_ready_one() {
        let mut out = Marks::default();
        resolve_wake_marks(&mut out, &[anchor(10, 20, true), anchor(11, 20, false)]);
        assert_eq!(out.count, 2);
        assert_eq!(out.a[0].kind, MarkKind::Bed);
        assert_eq!(out.a[1].kind, MarkKind::BedSpent);
        // …and the same blue either way: shape is the channel, not colour.
        assert_eq!(MarkKind::Bed.fill(), MarkKind::BedSpent.fill());
    }

    /// **`ALPHA.md` §1: no map position.** The death screen grew a map and
    /// that rule did not move — a marker for where you fell would hand the
    /// raider standing over your body a pin to the base they just cleared.
    /// Structural, because the way this breaks is somebody helpfully adding
    /// a "you died here" dot.
    #[test]
    fn no_wake_map_marks_the_corpse() {
        let mut out = Marks::default();
        resolve_wake_marks(&mut out, &[anchor(10, 20, true)]);
        assert_eq!(out.count, 1, "the wake map drew something besides the bags");
        assert!(
            out.a[..out.count]
                .iter()
                .all(|m| matches!(m.kind, MarkKind::Bed | MarkKind::BedSpent)),
            "the wake map drew a kind that is not one of your own bags"
        );
        // No bags is an empty map and not a haven ring: this screen answers
        // "where can I wake", and with no answer there is nothing to draw.
        resolve_wake_marks(&mut out, &[]);
        assert_eq!(out.count, 0);
        assert_eq!(out.dropped, 0);
    }

    /// The two maps put the same bag on the same pixel. They are separate
    /// functions over separate data — the island map reads the broadcast
    /// deploy mirror, this reads the own-fact list — and a projection that
    /// disagreed between them would be right on one screen and wrong on
    /// the other, which is the positional-payload defect class exactly.
    #[test]
    fn the_two_maps_agree_about_where_a_bag_is() {
        let (defs, have) = defs_with(&[ARCH_BAG]);
        let haven = terrain::haven(42);
        let mut island = Marks::default();
        resolve_marks(
            &mut island,
            &haven,
            &[rec_at(37, 91, 0)],
            &defs,
            have,
            &[],
            0,
        );
        let bed = island.a[..island.count]
            .iter()
            .find(|m| m.kind == MarkKind::Bed)
            .expect("the island map drew the bed");

        let mut wake = Marks::default();
        resolve_wake_marks(&mut wake, &[anchor(37, 91, true)]);
        assert_eq!(wake.count, 1);
        assert_eq!(
            (wake.a[0].px, wake.a[0].py),
            (bed.px, bed.py),
            "the death screen and the island map disagree about where a bag is"
        );
    }

    /// The cap holds here too. It cannot be reached today — `BAG_CAP` is 8
    /// against `MAP_MARKS_MAX`'s 64 — and it is asserted anyway, because a
    /// cap that is only correct while another crate's constant stays put is
    /// not a cap (wall 4).
    #[test]
    fn the_wake_map_is_bounded_and_says_so() {
        let over = 5;
        let bags: Vec<BagAnchor> = (0..MAP_MARKS_MAX + over)
            .map(|i| anchor((i % 1000) as u16, 7, i % 2 == 0))
            .collect();
        let mut out = Marks::default();
        resolve_wake_marks(&mut out, &bags);
        assert_eq!(out.count, MAP_MARKS_MAX);
        assert_eq!(out.dropped, over);
    }
}
