//! The in-game menus' gate.
//!
//! **Headless, and not behind `--features render`.** Every function under
//! `client::ui` is pure, so this runs in the code tier beside the other
//! walls rather than in the renderer tier where nobody looks at it. That is
//! the whole reason the arithmetic was put outside the Bevy systems: a menu
//! bug is an arithmetic bug, and arithmetic that needs a window to run needs
//! a window to test.
//!
//! What it covers, and why each one is here rather than trusted:
//!
//! - **§A the move verb.** `CLAUDE.md`'s trap list names it as the most
//!   bug-prone thing in the reference, failing as a *disconnect*, and as a
//!   positional payload no byte-golden can see. Every refusal is probed with
//!   a value that would be wrong in a distinguishable way.
//! - **§B the split gestures**, because half-of-one is the rounding a player
//!   notices.
//! - **§C the craft panel's numbers** — affordability, the ingredient table,
//!   time, and the search — because each is a number a player will act on.
//! - **§D the wheel's angles.** Off by half a segment is invisible in code
//!   and obvious in the hand.
//! - **§E the loading screen's two waits**, because the first of them —
//!   "has the server said where?" — has no natural symptom. An unplaced
//!   predictor reports the world origin rather than nothing, so the failure
//!   is a client that builds the wrong neighbourhood confidently and a bar
//!   that fills while it does.
//! - **§F one owner for the face.** The only gate here that is a grep, and
//!   for the reason `tests/sound.rs` states about the feed drain: **the
//!   defect is a call site, not a value.** A `TextFont` literal compiles,
//!   passes clippy, and draws in Bevy's debug mono next to forty words that
//!   are not — which is exactly how all forty of them got there.

use client::ui::build::{self, Rings, MATERIALS, SHAPES};
use client::ui::craft::{self, Cat, Facts};
use client::ui::load::Progress;
use client::ui::slots::{self, Grab};
use sim_core::build::{BuildContent, MAT_METAL, MAT_STONE, MAT_WOOD, SHAPE_FOUNDATION, SHAPE_WALL};
use sim_core::craft::CraftContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::ItemStack;
use sim_core::inventory::{CONT_BAG, CONT_BOX, CONT_SELF};
use sim_core::limits::{BOX_SLOTS, INV_SLOTS};

fn empty() -> [ItemStack; INV_SLOTS] {
    [ItemStack::default(); INV_SLOTS]
}

fn stocked() -> [ItemStack; INV_SLOTS] {
    let mut inv = empty();
    inv[0] = ItemStack { item: 7, count: 9 };
    inv[1] = ItemStack {
        item: 3,
        count: 100,
    };
    inv[2] = ItemStack { item: 7, count: 1 };
    inv
}

/// A real handle: large, distinct, and not confusable with a kind or a slot.
/// The browser client learned this the hard way — while `bag`, `from_kind`
/// and `to_kind` were all 0 on every legal call, transposing two of them was
/// a green mutant.
const HANDLE: u32 = 0x0031_0004;

// ---- §A · the move verb -------------------------------------------------

#[test]
fn move_marshals_every_field_to_its_own_name() {
    let inv = stocked();
    let cont = empty();
    let m = slots::move_args(0, CONT_SELF, 0, CONT_SELF, 5, Grab::All, &inv, &cont)
        .expect("a self move of a real stack");
    assert_eq!(m.from_slot, 0);
    assert_eq!(m.to_slot, 5);
    assert_eq!(m.from_kind, CONT_SELF);
    assert_eq!(m.to_kind, CONT_SELF);
    // The count comes off the INVENTORY's count word, never off the item id
    // in the even neighbour — the fixture's item (7) and count (9) differ so
    // reading the wrong one is visible.
    assert_eq!(m.count, 9);
    // Self→self normalizes the handle away rather than passing one through:
    // `world.rs` never reads the field for such a move and the encoder does
    // not range-check it, so a stray value would enter the WAL unvalidated.
    assert_eq!(m.bag, 0);
}

#[test]
fn a_ground_move_carries_the_handle_and_the_containers_own_count() {
    let inv = empty();
    let mut cont = empty();
    cont[4] = ItemStack { item: 2, count: 6 };
    let m = slots::move_args(HANDLE, CONT_BOX, 4, CONT_SELF, 0, Grab::All, &inv, &cont)
        .expect("box to self");
    assert_eq!(m.bag, HANDLE);
    assert_eq!(m.from_kind, CONT_BOX);
    // Read from the CONTAINER's view. Reading `inv` here is the label bug
    // one array over: same shape, different container, and the sim would be
    // handed a count the client never drew.
    assert_eq!(m.count, 6);
}

#[test]
fn refusals_in_order() {
    let inv = stocked();
    let mut cont = empty();
    cont[0] = ItemStack { item: 2, count: 4 };

    // 1 · a kind past CONT_MAX.
    assert!(slots::move_args(HANDLE, 9, 0, CONT_SELF, 1, Grab::All, &inv, &cont).is_none());

    // 2 · two DIFFERENT ground containers — the command carries one handle.
    assert!(slots::move_args(HANDLE, CONT_BAG, 0, CONT_BOX, 1, Grab::All, &inv, &cont).is_none());
    // ...but the same kind on both ends is rearranging one open container.
    assert!(slots::move_args(HANDLE, CONT_BOX, 0, CONT_BOX, 1, Grab::All, &inv, &cont).is_some());

    // 3 · a slot past its OWN container's width. The encoder would carry
    //     box slot 20 — `slots_in` is why the panel does not send it.
    assert_eq!(slots::slots_in(CONT_BOX), BOX_SLOTS);
    assert!(slots::move_args(
        HANDLE,
        CONT_BOX,
        0,
        CONT_BOX,
        BOX_SLOTS,
        Grab::All,
        &inv,
        &cont
    )
    .is_none());
    // The same slot number in your own inventory is fine: the bound is
    // per-kind, not one flat INV_SLOTS for everything.
    assert!(slots::move_args(
        0,
        CONT_SELF,
        0,
        CONT_SELF,
        BOX_SLOTS,
        Grab::All,
        &inv,
        &cont
    )
    .is_some());

    // 4 · the same address twice.
    assert!(slots::move_args(0, CONT_SELF, 3, CONT_SELF, 3, Grab::All, &inv, &cont).is_none());
    // Same slot NUMBER across two kinds is a different address.
    assert!(slots::move_args(HANDLE, CONT_BOX, 0, CONT_SELF, 0, Grab::All, &inv, &cont).is_some());

    // 5 · a ground end with a zero handle. `box_key(0,0,0) == 0` addresses a
    //     real box, so sending 0 for "no container known" would move items
    //     in a stranger's box rather than being refused.
    assert!(slots::move_args(0, CONT_BOX, 0, CONT_SELF, 5, Grab::All, &inv, &cont).is_none());

    // 6 · an empty source. The sim does not clamp a count, so neither does
    //     this: a stack of nothing yields nothing to send.
    assert!(slots::move_args(0, CONT_SELF, 20, CONT_SELF, 21, Grab::All, &inv, &cont).is_none());
}

#[test]
fn a_validated_move_encodes() {
    let inv = stocked();
    let cont = empty();
    let m = slots::move_args(0, CONT_SELF, 0, CONT_SELF, 5, Grab::All, &inv, &cont).unwrap();
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    let len = m
        .encode(&mut buf)
        .expect("the wire carries a validated move");
    assert!(len > 0 && len <= buf.len());
}

// ---- §B · the split gestures --------------------------------------------

#[test]
fn grabs_round_the_way_a_hand_expects() {
    assert_eq!(Grab::All.units(9), 9);
    // Rounded UP, so a stack of one still moves rather than becoming a drag
    // that silently does nothing.
    assert_eq!(Grab::Half.units(9), 5);
    assert_eq!(Grab::Half.units(1), 1);
    assert_eq!(Grab::One.units(9), 1);
    // Nothing in, nothing out — an empty slot is refused by `move_args`
    // rather than clamped here.
    assert_eq!(Grab::All.units(0), 0);
    assert_eq!(Grab::Half.units(0), 0);
    assert_eq!(Grab::One.units(0), 0);
}

#[test]
fn a_half_drag_sends_half() {
    let inv = stocked();
    let cont = empty();
    let m = slots::move_args(0, CONT_SELF, 0, CONT_SELF, 6, Grab::Half, &inv, &cont).unwrap();
    assert_eq!(m.count, 5);
    let m = slots::move_args(0, CONT_SELF, 0, CONT_SELF, 6, Grab::One, &inv, &cont).unwrap();
    assert_eq!(m.count, 1);
}

#[test]
fn the_grid_geometry_covers_every_slot_once() {
    let mut seen = std::collections::BTreeSet::new();
    for slot in 0..INV_SLOTS {
        let c = slots::cell_of(CONT_SELF, slot).expect("inside the inventory");
        assert!(seen.insert((c.col, c.row)), "slot {slot} shares a cell");
        assert!(c.col < slots::GRID_COLS);
    }
    assert_eq!(seen.len(), INV_SLOTS);
    assert!(slots::cell_of(CONT_BOX, BOX_SLOTS).is_none());
}

// ---- §C · the craft panel -----------------------------------------------

fn craft_fixture() -> CraftContent {
    // Row 0: 3 of item 0 → 2 of item 2, no station.
    // Row 1: 2 of item 1 + 1 of item 2 → 1 of item 3.
    // Row 2: 1 of item 0 → 1 of item 4, workbench.
    CraftContent::probe_fixture()
}

#[test]
fn affordability_is_the_tightest_input() {
    let recipes = craft_fixture();
    let mut inv = empty();
    inv[0] = ItemStack { item: 0, count: 7 };
    // Row 0 costs 3 of item 0, so seven pays for two and leaves one over.
    assert_eq!(craft::affordable(&recipes.recipes[0], &inv), 2);

    // Row 1 needs two items and the SCARCER one is the ceiling.
    inv[1] = ItemStack { item: 1, count: 10 };
    inv[2] = ItemStack { item: 2, count: 1 };
    assert_eq!(craft::affordable(&recipes.recipes[1], &inv), 1);

    // Nothing in the bag pays for nothing.
    assert_eq!(craft::affordable(&recipes.recipes[0], &empty()), 0);
}

#[test]
fn the_ingredient_table_scales_with_the_stepper() {
    let recipes = craft_fixture();
    let mut inv = empty();
    inv[0] = ItemStack { item: 0, count: 5 };
    let (lines, n) = craft::ingredients(&recipes.recipes[0], 3, &inv);
    assert_eq!(n, 1);
    // AMOUNT is per craft and does NOT scale; TOTAL is what three cost.
    assert_eq!(lines[0].amount, 3);
    assert_eq!(lines[0].total, 9);
    assert_eq!(lines[0].have, 5);
    // Five of nine is short, which is what the reference paints red.
    assert!(lines[0].short());

    let (lines, _) = craft::ingredients(&recipes.recipes[0], 1, &inv);
    assert_eq!(lines[0].total, 3);
    assert!(!lines[0].short());
}

#[test]
fn craft_time_counts_the_sims_own_ticks() {
    let recipes = craft_fixture();
    // Row 0 is 2 ticks at 30 Hz.
    let one = craft::seconds(&recipes.recipes[0], 1);
    assert!((one - 2.0 / 30.0).abs() < 1e-6, "one craft: {one}");
    let four = craft::seconds(&recipes.recipes[0], 4);
    assert!((four - one * 4.0).abs() < 1e-6);
}

#[test]
fn the_station_badge_is_absent_rather_than_empty() {
    let recipes = craft_fixture();
    assert!(craft::station_label(recipes.recipes[0].station).is_none());
    assert!(craft::station_label(recipes.recipes[2].station).is_some());
}

#[test]
fn search_is_case_insensitive_and_an_empty_query_matches() {
    assert!(craft::name_matches(b"Metal Fragments", ""));
    assert!(craft::name_matches(b"Metal Fragments", "metal"));
    assert!(craft::name_matches(b"Metal Fragments", "FRAG"));
    assert!(craft::name_matches(b"Metal Fragments", " frag "));
    assert!(!craft::name_matches(b"Wood", "stone"));
    // A query longer than the name cannot be inside it.
    assert!(!craft::name_matches(b"Wood", "woodwork"));
}

#[test]
fn the_rail_buckets_are_computed_and_not_guessed() {
    let recipes = craft_fixture();
    let deploys = DeployContent::probe_fixture();
    let facts = Facts::build(&recipes, &deploys);

    // BY HAND and WORKBENCH partition by the sim's own station field.
    assert!(craft::in_category(
        Cat::ByHand,
        0,
        &recipes.recipes[0],
        &facts,
        &[]
    ));
    assert!(!craft::in_category(
        Cat::ByHand,
        2,
        &recipes.recipes[2],
        &facts,
        &[]
    ));
    assert!(craft::in_category(
        Cat::Workbench,
        2,
        &recipes.recipes[2],
        &facts,
        &[]
    ));

    // COMPONENT is "this output feeds another recipe": row 0 makes item 2,
    // which row 1 consumes.
    assert!(facts.is_component(2));
    assert!(craft::in_category(
        Cat::Component,
        0,
        &recipes.recipes[0],
        &facts,
        &[]
    ));

    // FAVOURITE is the local latch and nothing else.
    assert!(!craft::in_category(
        Cat::Favourite,
        0,
        &recipes.recipes[0],
        &facts,
        &[]
    ));
    assert!(craft::in_category(
        Cat::Favourite,
        0,
        &recipes.recipes[0],
        &facts,
        &[0]
    ));

    // ALL takes everything, which is what makes it the safe default.
    assert!(craft::in_category(
        Cat::All,
        2,
        &recipes.recipes[2],
        &facts,
        &[]
    ));
}

#[test]
fn the_browser_skips_inert_rows() {
    let recipes = craft_fixture();
    let deploys = DeployContent::probe_fixture();
    let facts = Facts::build(&recipes, &deploys);
    let catalog = protocol::event::ItemCatalog::EMPTY;
    let mut out = Vec::new();
    craft::rows(
        &recipes,
        &empty(),
        &catalog,
        &facts,
        &[],
        0,
        Cat::All,
        "",
        &mut out,
    );
    // The fixture declares three; the rest of the table is `INERT`
    // (`out_count == 0`) and an inert row is not a recipe.
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|r| r.out_count > 0));

    // An empty table is empty rather than full of blanks.
    let mut out2 = Vec::new();
    craft::rows(
        &CraftContent::EMPTY,
        &empty(),
        &catalog,
        &facts,
        &[],
        0,
        Cat::All,
        "",
        &mut out2,
    );
    assert!(out2.is_empty());
}

#[test]
fn an_unnamed_item_prints_its_index_rather_than_nothing() {
    let mut catalog = protocol::event::ItemCatalog::EMPTY;
    catalog.set(4, b"Wood").unwrap();
    assert_eq!(craft::item_name(&catalog, 4), Some("Wood"));
    assert_eq!(craft::item_name(&catalog, 9), None);
    assert_eq!(craft::item_label(&catalog, 4), "Wood");
    assert_eq!(craft::item_label(&catalog, 9), "#9");
}

// ---- §D · the build wheel -----------------------------------------------

#[test]
fn segment_zero_is_centred_on_up() {
    let n = SHAPES.len();
    // Straight up is segment 0, and so is a nudge to either side of it —
    // "centred" is the claim, and a ring whose first segment STARTS at up
    // would fail the second of these.
    assert_eq!(build::segment(0.0, 1.0, n), 0);
    assert_eq!(build::segment(0.10, 1.0, n), 0);
    assert_eq!(build::segment(-0.10, 1.0, n), 0);
}

#[test]
fn segments_increase_clockwise() {
    let n = SHAPES.len();
    // Six segments: 60° each. A quarter turn to the RIGHT is segment 1 or 2,
    // never 4 or 5 — a wheel that numbered anticlockwise would highlight the
    // mirror of what the labels say.
    let right = build::segment(1.0, 0.0, n);
    assert_eq!(right, n.div_ceil(4), "90 deg right: {right}");
    let left = build::segment(-1.0, 0.0, n);
    assert!(
        left > n / 2,
        "90 deg left should be late in the ring: {left}"
    );
    // Straight down is the segment opposite 0.
    assert_eq!(build::segment(0.0, -1.0, n), n / 2);
}

#[test]
fn every_direction_lands_in_exactly_one_segment() {
    // 4 is the hammer wheel's count (§K rides this arithmetic).
    for n in [3usize, 4, 6] {
        let mut hits = vec![0u32; n];
        // A full sweep at a degree a step: no direction may fall outside the
        // ring, and every segment must be reachable.
        for deg in 0..360 {
            let t = (deg as f32).to_radians();
            let seg = build::segment(t.sin(), t.cos(), n);
            assert!(seg < n, "{deg} deg fell outside a ring of {n}");
            hits[seg] += 1;
        }
        assert!(
            hits.iter().all(|h| *h > 0),
            "a segment of {n} was unreachable"
        );
    }
}

#[test]
fn the_labels_sit_in_the_segments_they_name() {
    // The drawing code places a chip at `segment_angle` and the pointer is
    // resolved by `segment`. If those two ever disagree the wheel highlights
    // one thing and selects another, so they are checked against each other
    // rather than each against a constant.
    for n in [3usize, 4, 6] {
        for i in 0..n {
            let a = build::segment_angle(i, n);
            assert_eq!(build::segment(a.sin(), a.cos(), n), i);
        }
    }
}

#[test]
fn the_ring_and_the_dead_centre() {
    let r = Rings::default();
    assert!(r.dead < r.rim);
    // Dead centre: a release here keeps what was chosen and picks nothing.
    assert_eq!(build::pick(0.0, 0.0, r), None);
    assert_eq!(build::pick(0.0, r.dead - 1.0, r), None);
    // Past the rim the pointer has left the wheel.
    assert_eq!(build::pick(0.0, r.rim + 1.0, r), None);
    // **One ring, and it is the shapes.** The material ring was cut on
    // 2026-08-07: the reference's blueprint places one rung and its hammer
    // climbs the ladder, so picking a material at placement time was a verb
    // the reference does not have. Anywhere in the band is a shape.
    let mid = (r.dead + r.rim) * 0.5;
    assert_eq!(build::pick(0.0, mid, r), Some(0));
    assert_eq!(build::pick(0.0, r.dead + 1.0, r), Some(0));
    assert_eq!(build::pick(0.0, r.rim - 1.0, r), Some(0));
}

/// The band matches the reference's proportion.
///
/// Not decoration: the ring texture is baked from these same radii
/// (`render/panels/ring.rs`), so a change here silently moves what is drawn
/// as well as what is picked, and the two staying equal is the wheel's
/// oldest rule.
#[test]
fn the_band_is_the_references_proportion() {
    let r = Rings::default();
    let ratio = r.dead / r.rim;
    assert!(
        (ratio - 0.66).abs() < 0.02,
        "inner/outer is {ratio:.3}, measured off building.jpeg as 0.66"
    );
}

/// A blueprint places one material and it is the bottom rung.
#[test]
fn the_blueprint_places_the_first_rung() {
    assert_eq!(build::PLACE_MATERIAL, MATERIALS[0]);
    assert_eq!(build::PLACE_MATERIAL, MAT_WOOD);
    // The ladder above it is reachable, or pinning the placement would have
    // made stone and metal unbuildable rather than upgrade-only.
    assert!(MATERIALS.len() > 1, "there is a ladder to climb");
}

#[test]
fn a_piece_row_is_searched_for_and_never_computed() {
    let content = BuildContent::probe_fixture();
    // The fixture is deliberately NOT a full shape × material grid, which is
    // the case `shape * 3 + material` would get wrong — and getting it wrong
    // means placing a different piece than the wheel drew.
    let wood_foundation = build::row_for(&content, SHAPE_FOUNDATION, MAT_WOOD);
    assert_eq!(wood_foundation, Some(0));
    let wood_wall = build::row_for(&content, SHAPE_WALL, MAT_WOOD);
    assert_eq!(wood_wall, Some(1));
    // A pair the content has no piece for is `None`, which the wheel draws
    // as a dead segment rather than a live one over the wrong row.
    assert_eq!(build::row_for(&content, SHAPE_FOUNDATION, MAT_METAL), None);
    // And an empty table has nothing at all.
    assert_eq!(
        build::row_for(&BuildContent::EMPTY, SHAPE_FOUNDATION, MAT_WOOD),
        None
    );
    let _ = MAT_STONE;
}

#[test]
fn the_wheel_prices_a_piece_against_the_bag() {
    let content = BuildContent::probe_fixture();
    let row = build::row_for(&content, SHAPE_FOUNDATION, MAT_WOOD).unwrap();
    let mut inv = empty();
    inv[0] = ItemStack { item: 0, count: 4 };
    let (lines, n) = build::costs(&content, row, &inv);
    assert_eq!(n, 1);
    // The fixture's foundation costs 5 of item 0; four is short by one.
    assert_eq!(lines[0].units, 5);
    assert_eq!(lines[0].have, 4);
    assert!(lines[0].short());
    assert!(!build::affordable(&content, row, &inv));

    inv[0] = ItemStack { item: 0, count: 5 };
    assert!(build::affordable(&content, row, &inv));
}

#[test]
fn every_ring_entry_has_a_label() {
    // A blank chip is the dark-panel defect at wheel scale.
    for s in SHAPES {
        assert!(!build::shape_label(s).is_empty());
        assert!(!build::shape_blurb(s).is_empty());
    }
    for m in MATERIALS {
        assert!(!build::material_label(m).is_empty());
    }
}

// ===========================================================================
// §E · The loading screen: nothing is loaded until the server says what
// ===========================================================================
//
// The welcome carries `player_id`, `seed` and `tick` — and no position. Where
// the player stands arrives one snapshot later, when the first packet
// carrying our own entity lets `Predictor::adopt` take the authoritative
// spawn. The bug this section pins is that the interval between those two is
// not visibly a wait: `Predictor::body` before its first snapshot is
// `Body::default()`, whose position is the **world origin**, and the world
// origin is a real place on this island. Every ring builder reads that
// position; handed it, none of them faults. They build a good neighbourhood
// of the wrong place, report themselves full, and the bar reaches 100%.
//
// So the assertions below are all about the same thing from different sides:
// an unplaced `Progress` is not progress, whatever its counters say.

/// A ring set that is entirely, unambiguously finished. The only thing left
/// to vary is whether the server ever said where to build it.
fn built(placed: bool) -> Progress {
    Progress {
        placed,
        ground: (25, 25),
        scatter: (25, 25),
        clutter: (81, 81),
        far: true,
    }
}

#[test]
fn a_world_built_around_an_unplaced_eye_is_not_a_loaded_world() {
    // The exact shape of the bug: every counter full, the far mesh up, and
    // the client still has no idea whether any of it is where the player is.
    // `done()` is what puts a player into `Screen::InWorld`, so this single
    // assertion is the difference between a loading screen and a spawn at
    // the wrong end of the island.
    assert!(!built(false).done());
    assert!(built(true).done());
}

#[test]
fn the_bar_does_not_move_before_the_server_places_you() {
    // Not cosmetic. A bar that climbs while the rings fill around the origin
    // is a bar reporting real work toward the wrong answer, and it would
    // reach full — the player would watch it finish and then wait again.
    assert_eq!(built(false).fraction(), 0.0);
    assert_eq!(built(true).fraction(), 1.0);

    // Partial fills too: the gate is in front of the mean, not one term of it.
    let half = Progress {
        placed: false,
        ground: (25, 25),
        scatter: (0, 25),
        clutter: (0, 81),
        far: false,
    };
    assert_eq!(half.fraction(), 0.0);

    // The same counters, placed: now the mean is the mean.
    let mut placed = half;
    placed.placed = true;
    assert!((placed.fraction() - 1.0 / 3.0).abs() < 1e-6);
}

#[test]
fn the_counts_line_names_which_wait_it_is_in() {
    // Three honest zeroes would be a lie of framing: no ring has been ASKED
    // for anything yet, so "GROUND 0/25" describes work that is not late,
    // it describes work that has not been ordered.
    let waiting = Progress::waiting();
    assert!(!waiting.placed);
    assert_eq!(waiting.fraction(), 0.0);
    assert!(!waiting.done());
    assert!(waiting.line().contains("WAITING FOR THE SERVER"));
    assert!(!waiting.line().contains("GROUND"));

    assert!(built(true).line().contains("GROUND 25/25"));
}

// ---------------------------------------------------------------------------
// The swing resolver. These assertions are the ones `ci/ui_smoke.mjs` used to
// make about `resolveSwing` before the browser client was deleted; they are
// re-earned here in Rust, against the mesh and the scan this client actually
// ships, rather than left as a hole (`NOW.md` §0y).

use client::ui::interact::{resolve_swing, swing_label, Island, SwingAim, SwingPick};
use sim_core::gather::{CONE_COS, DY_MAX_M, POINT_BLANK_M2, REACH_M as SWING_REACH_M};
use sim_core::occupy::{Harvested, Pristine};
use sim_core::terrain::{scatter, Occupant, ScatterTable, CELL_SIZE};

/// A harvested set naming exactly one cell, so "the sim already took it" is
/// testable without standing a server up.
struct OneHarvested(u16, u16);
impl Harvested for OneHarvested {
    fn is_harvested(&self, cx: u16, cz: u16) -> bool {
        cx == self.0 && cz == self.1
    }
}

/// Walk the island for a seed/cell that actually holds each swingable kind,
/// so the test aims at real worldgen instead of a fixture that agrees with it.
fn find_slot(seed: u64, want: Occupant) -> Option<(i32, i32, f32, f32, f32)> {
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    for cz in -160..160 {
        for cx in -160..160 {
            let s = scatter(seed, &table, &haven, cx, cz);
            if s.occupant == want {
                return Some((cx, cz, s.x, s.y, s.z));
            }
        }
    }
    None
}

/// Standing on a node and looking at it names it; the label is the kind.
#[test]
fn a_swing_names_what_it_would_hit() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let mut named = 0;
    let mut kinds = Vec::new();
    for want in [
        Occupant::Tree,
        Occupant::StoneNode,
        Occupant::MetalNode,
        Occupant::SulfurNode,
        Occupant::Bush,
    ] {
        let Some((cx, cz, sx, sy, sz)) = find_slot(seed, want) else {
            continue;
        };
        // Stand one metre short of it, looking straight at it.
        let px = sx - 1.0;
        let pz = sz;
        let pick = resolve_swing(
            SwingAim {
                x: px,
                y: sy,
                z: pz,
                fx: 1.0,
                fz: 0.0,
            },
            Island {
                seed,
                table: &table,
                haven: &haven,
                harvested: &Pristine,
            },
        );
        assert_eq!(
            pick.occupant, want as u8,
            "{want:?} at cell ({cx},{cz}) was not picked from 1 m in front of it"
        );
        assert_eq!((pick.cx, pick.cz), (cx as u16, cz as u16));
        assert!(
            !swing_label(pick.occupant).is_empty(),
            "{want:?} has no label"
        );
        named += 1;
        kinds.push(want);
    }
    // A test that silently found nothing to aim at would pass every
    // assertion above it. Name what was covered so a shrinking scatter
    // table shows up as a smaller list rather than as continued green.
    assert!(
        named >= 3,
        "worldgen offered only {named} swingable kinds to aim at: {kinds:?}"
    );
    println!("swing prompt covered {named} kinds: {kinds:?}");
}

/// A rock is scenery. The sim never harvests one, so the crosshair must not
/// offer a swing at it — `gather::target_index` makes the same split.
#[test]
fn a_rock_is_not_a_target() {
    assert_eq!(swing_label(Occupant::Rock as u8), "");
    assert_eq!(swing_label(Occupant::None as u8), "");
    // 8 is the stump — a consequence of a harvest, never a target.
    assert_eq!(swing_label(8), "");
}

/// Looking away from a node in reach must whiff: the cone is the aim, and a
/// prompt that ignored it would name a thing behind the player.
#[test]
fn the_cone_is_the_aim() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let (_, _, sx, sy, sz) = find_slot(seed, Occupant::Tree).expect("seed 7 has a tree");
    let px = sx - 1.0;

    let toward = resolve_swing(
        SwingAim {
            x: px,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(toward.occupant, Occupant::Tree as u8);

    let away = resolve_swing(
        SwingAim {
            x: px,
            y: sy,
            z: sz,
            fx: -1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(
        away.occupant, 0,
        "a tree behind the player was offered as a swing"
    );
}

/// Point blank beats the cone — you can hit what you are standing in.
#[test]
fn point_blank_ignores_the_cone() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let (_, _, sx, sy, sz) = find_slot(seed, Occupant::Tree).expect("seed 7 has a tree");
    // Inside POINT_BLANK_M2, facing the wrong way entirely.
    let off = (POINT_BLANK_M2.sqrt()) * 0.5;
    let pick = resolve_swing(
        SwingAim {
            x: sx + off,
            y: sy,
            z: sz,
            fx: -1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(
        pick.occupant,
        Occupant::Tree as u8,
        "point blank must ignore the cone"
    );
}

/// Past `REACH_M` is a whiff however well aimed — the reach is a wall, and a
/// prompt that outran the sim's own scan would invite a swing that refuses.
#[test]
fn reach_bounds_the_prompt() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let (_, _, sx, sy, sz) = find_slot(seed, Occupant::Tree).expect("seed 7 has a tree");

    let inside = resolve_swing(
        SwingAim {
            x: sx - SWING_REACH_M * 0.9,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(
        inside.occupant,
        Occupant::Tree as u8,
        "0.9 x reach must hit"
    );

    let outside = resolve_swing(
        SwingAim {
            x: sx - SWING_REACH_M * 1.1,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(outside.occupant, 0, "1.1 x reach must whiff");
}

/// A harvested cell is not a target. The browser needed `hidden || fellAt`
/// because it animated the fall over 93 ticks; this client swaps the mesh on
/// the event, so the harvested set is the whole truth — and if that ever stops
/// being so, this test is where it shows.
#[test]
fn the_sim_having_taken_it_ends_the_prompt() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let (cx, cz, sx, sy, sz) = find_slot(seed, Occupant::Tree).expect("seed 7 has a tree");
    let px = sx - 1.0;

    let standing = resolve_swing(
        SwingAim {
            x: px,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(standing.occupant, Occupant::Tree as u8);

    let taken = OneHarvested(cx as u16, cz as u16);
    let gone = resolve_swing(
        SwingAim {
            x: px,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &taken,
        },
    );
    assert_eq!(
        gone.occupant, 0,
        "a harvested cell was still offered as a swing"
    );
}

/// The vertical window. A node far above or below the eye is out of the
/// swing even when its planar distance is nothing.
#[test]
fn the_vertical_window_bounds_it() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let (_, _, sx, sy, sz) = find_slot(seed, Occupant::Tree).expect("seed 7 has a tree");
    let px = sx - 1.0;

    let level = resolve_swing(
        SwingAim {
            x: px,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(level.occupant, Occupant::Tree as u8);

    let below = resolve_swing(
        SwingAim {
            x: px,
            y: sy + DY_MAX_M * 1.5,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(
        below.occupant, 0,
        "a node {DY_MAX_M} m below the eye must whiff"
    );
}

/// The pick is a pure function of its inputs: the prompt drawn on one frame
/// and the verb run on the click cannot differ while the world stands still.
#[test]
fn the_swing_pick_is_stable() {
    let seed = 7;
    let table = ScatterTable::alpha_default();
    let haven = sim_core::terrain::haven(seed);
    let (_, _, sx, sy, sz) = find_slot(seed, Occupant::Tree).expect("seed 7 has a tree");
    let a = resolve_swing(
        SwingAim {
            x: sx - 1.0,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    let b = resolve_swing(
        SwingAim {
            x: sx - 1.0,
            y: sy,
            z: sz,
            fx: 1.0,
            fz: 0.0,
        },
        Island {
            seed,
            table: &table,
            haven: &haven,
            harvested: &Pristine,
        },
    );
    assert_eq!(a, b);
    assert_eq!(SwingPick::default().occupant, 0);
}

/// Every bound this resolver uses is `gather`'s own, not a copy. If the sim
/// moves one, this fails rather than the crosshair quietly disagreeing with
/// the swing it invites.
#[test]
fn the_swing_reads_the_sims_own_bounds() {
    assert_eq!(SWING_REACH_M, 2.0);
    assert_eq!(CONE_COS, 0.866_025_4);
    assert_eq!(DY_MAX_M, 3.0);
    assert_eq!(POINT_BLANK_M2, 0.04);
    assert_eq!(CELL_SIZE, 8.0);
}

// ---------------------------------------------------------------------------
// The weak spot. These mirror `gather::swing`'s sector test; if the sim moves
// `WEAK_COS`, the LUT, or the point-blank exemption, these go red rather than
// the HUD quietly promising a bonus the server refuses.

use client::ui::interact::{in_weak_sector, mark_is_for};
use sim_core::gather::{cell_key, weak_mark8, NO_CELL, POINT_BLANK_M2 as PB, WEAK_COS};
use sim_core::yaw_dir;

/// Standing exactly on the announced bearing is inside; the opposite side is
/// out. The offset is node->player, so mark 0 (north) means stand north.
#[test]
fn the_weak_sector_is_the_sims_own() {
    for mark8 in [0u8, 32, 64, 96, 128, 160, 192, 224] {
        let (wx, wz) = yaw_dir((mark8 as u16) << 8);
        // Two metres out along the mark's own bearing.
        assert!(
            in_weak_sector(wx * 2.0, wz * 2.0, 0.0, 0.0, mark8),
            "mark {mark8}: standing on the bearing must be inside"
        );
        // And directly opposite.
        assert!(
            !in_weak_sector(-wx * 2.0, -wz * 2.0, 0.0, 0.0, mark8),
            "mark {mark8}: the far side must be outside"
        );
    }
}

/// The half-angle is `WEAK_COS`, not a number of our own. Just inside passes
/// and just outside fails, both measured against the sim's constant.
#[test]
fn the_sector_half_angle_is_weak_cos() {
    let mark8 = 0u8;
    let (wx, wz) = yaw_dir(0);
    let r = 3.0f32;
    let half = WEAK_COS.acos();
    for (delta, want) in [(half * 0.9, true), (half * 1.1, false)] {
        // Rotate the stand point by `delta` about the node.
        let (s, c) = (delta.sin(), delta.cos());
        let (px, pz) = ((wx * c - wz * s) * r, (wx * s + wz * c) * r);
        assert_eq!(
            in_weak_sector(px, pz, 0.0, 0.0, mark8),
            want,
            "at {delta} rad from the bearing (half-angle {half}) expected {want}"
        );
    }
}

/// Point blank has no bearing to judge and the sim never bonuses it, so the
/// cue must not light up there either — a prompt promising a bonus the server
/// refuses is worse than no prompt.
#[test]
fn point_blank_never_promises_a_bonus() {
    let inside = PB.sqrt() * 0.5;
    for mark8 in [0u8, 64, 128, 192] {
        assert!(
            !in_weak_sector(inside, 0.0, 0.0, 0.0, mark8),
            "mark {mark8}: point blank must never read as a weak spot"
        );
    }
}

/// The mark belongs to one node. A mark for another cell, or no chase at all,
/// must not decorate the prompt for the thing actually aimed at.
#[test]
fn the_mark_belongs_to_one_node() {
    let pick = SwingPick {
        occupant: 1,
        cx: 40,
        cz: 90,
        d2: 1.0,
    };
    assert!(
        mark_is_for(cell_key(40, 90), &pick),
        "its own cell must match"
    );
    assert!(
        !mark_is_for(cell_key(41, 90), &pick),
        "a neighbour must not"
    );
    assert!(!mark_is_for(NO_CELL, &pick), "no chase must not");
    // And a whiff is never decorated, whatever the mark says.
    let whiff = SwingPick::default();
    assert!(!mark_is_for(cell_key(0, 0), &whiff));
}

/// The mark the server announces is a function of the chase, so it moves as
/// hits land — a player who found it once cannot stand still and farm it.
#[test]
fn the_mark_moves_with_the_chase() {
    let (seed, cx, cz, pid) = (11u64, 40u16, 90u16, 7u32);
    let marks: Vec<u8> = (1..=8u16)
        .map(|n| weak_mark8(seed, cx, cz, pid, n))
        .collect();
    assert!(
        marks.iter().collect::<std::collections::HashSet<_>>().len() > 1,
        "the weak mark never moved across 8 hits: {marks:?}"
    );
}

// ---------------------------------------------------------------------------
// §F · one owner for the face
//
// `render/ui.rs` owns the palette because three screens that are supposed to
// read as one product had three copies of the same six colours. It owns the
// FACE for a harder version of the same reason, and the harder version is
// what actually happened: until 2026-08-07 nothing owned it, all forty-two
// `TextFont` sites in the render path were `..default()`, and every screen
// this client has drew in Bevy's embedded debug mono.
//
// A grep rather than a value, exactly as `tests/sound.rs`'s feed-drain gate
// is: a `TextFont { font_size: 12.0, ..default() }` is legal Rust, survives
// `clippy -D warnings`, and is invisible in review because it looks like
// every other bundle field. The only place it shows up is on the screen, and
// nothing in this repo photographs a panel.

/// Every `.rs` under `src/render`, path and text.
fn render_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).expect("src/render must exist") {
            let p = e.expect("readable entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push((
                    p.display().to_string(),
                    std::fs::read_to_string(&p).expect("readable source"),
                ));
            }
        }
    }
    let mut files = Vec::new();
    walk(std::path::Path::new("src/render"), &mut files);
    assert!(
        files.len() > 10,
        "only found {} render sources",
        files.len()
    );
    files
}

/// No `TextFont` literal outside the file that owns the face.
///
/// The escape hatch is deliberately not a comment marker: a site that needs
/// something `ui::font`/`ui::font_bold` cannot express should widen the
/// vocabulary, because the next screen will want it too.
#[test]
fn only_ui_rs_builds_a_textfont() {
    let mut offenders = Vec::new();
    for (path, text) in render_sources() {
        if path.ends_with("render/ui.rs") {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            // Comments may name the type freely — the rule is about
            // construction, and every comment about it says so.
            if code.starts_with("//") || code.starts_with("*") {
                continue;
            }
            if code.contains("TextFont {") {
                offenders.push(format!("{path}:{}: {}", n + 1, code));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a TextFont built outside render/ui.rs draws in a different typeface \
         from the rest of the screen — use ui::font (prose) or ui::font_bold \
         (labels, the reference's own default weight):\n{}",
        offenders.join("\n")
    );
}

/// Both faces, and the notice that lets us ship them.
///
/// Apache-2.0 is a *notice* licence: the fonts may be redistributed and the
/// licence text has to travel with them. `include_bytes!` makes a missing
/// `.ttf` a build error, which is why this only has to check the third file
/// — the one nothing links against and everything depends on legally.
#[test]
fn the_faces_ship_with_their_licence() {
    for name in [
        "RobotoCondensed-Regular.ttf",
        "RobotoCondensed-Bold.ttf",
        "LICENSE-ROBOTO.txt",
    ] {
        let p = std::path::Path::new("fonts").join(name);
        let n = std::fs::metadata(&p)
            .unwrap_or_else(|e| panic!("{} is missing: {e}", p.display()))
            .len();
        assert!(
            n > 1024,
            "{} is {n} bytes, which is not a file",
            p.display()
        );
    }
    let licence = std::fs::read_to_string("fonts/LICENSE-ROBOTO.txt").expect("readable licence");
    assert!(
        licence.contains("Apache License") && licence.contains("Version 2.0"),
        "fonts/LICENSE-ROBOTO.txt is not the Apache 2.0 text the fonts are under"
    );
}

/// The vocabulary is two functions and both are used.
///
/// A weight nobody asks for is a weight that rots: if `ui::font` loses its
/// last call site, prose has quietly become bold everywhere and the
/// hierarchy the reference actually has is gone. Counted rather than
/// asserted-nonzero so the number moving is visible in a diff.
#[test]
fn both_weights_are_in_use() {
    let (mut prose, mut label) = (0usize, 0usize);
    for (path, text) in render_sources() {
        if path.ends_with("render/ui.rs") {
            continue;
        }
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            // Disjoint needles: `font_bold(` does not contain `font(`.
            label += code.matches("font_bold(").count();
            prose += code.matches("font(").count();
        }
    }
    // 14 and 26 as this landed. The floors are loose on purpose — this
    // catches a *collapse* (one weight losing its last call sites), not the
    // ordinary drift of a screen gaining or losing a line, which would make
    // it a gate that cries wolf at every UI commit.
    assert!(
        prose >= 5 && label >= 15,
        "the weight hierarchy has collapsed: {prose} prose sites, {label} label sites"
    );
}

// ---------------------------------------------------------------------------
// §G · the icons ship, and the two lists agree
//
// The icons are CC BY 3.0 from game-icons.net. That licence is a *notice*
// licence: the files may be redistributed and the credit has to travel with
// them. The credit lives in `assets/icons/CREDITS.md`, which nothing links
// against and nothing would notice the loss of — the same shape as the font
// licence in §F, and the same reason for a gate.
//
// The second half is the drift that would actually bite: `icons::STEMS` is a
// const list the render path reads, and `assets/icons/` is what ships. A stem
// with no file draws an empty cell; a file with no stem is dead weight in the
// depot. Neither is visible without opening the game.

/// Where the client's assets live, from the crate root this test runs in.
fn icon_dir() -> std::path::PathBuf {
    std::path::Path::new("../../assets/icons").to_path_buf()
}

#[test]
fn the_icons_ship_with_their_credit() {
    let credits = icon_dir().join("CREDITS.md");
    let text = std::fs::read_to_string(&credits)
        .unwrap_or_else(|e| panic!("{} is missing: {e}", credits.display()));
    // The three things CC BY 3.0 actually asks for: the work, the licence,
    // and the authors.
    assert!(
        text.contains("game-icons.net"),
        "the credit does not name the source"
    );
    assert!(
        text.contains("Attribution 3.0") || text.contains("CC BY 3.0"),
        "the credit does not name the licence"
    );
    for author in ["lorc", "delapouite"] {
        assert!(
            text.contains(author),
            "the credit does not name {author}, whose icons ship"
        );
    }
}

#[test]
fn every_stem_has_a_file_and_every_file_has_a_stem() {
    let dir = icon_dir();
    let on_disk: std::collections::BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} is missing: {e}", dir.display()))
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "png").then(|| p.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    let declared: std::collections::BTreeSet<String> = client::ui::icons::STEMS
        .iter()
        .map(|s| s.to_string())
        .collect();

    let missing: Vec<_> = declared.difference(&on_disk).collect();
    let orphan: Vec<_> = on_disk.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "declared in icons::STEMS but not in assets/icons — these draw an \
         empty cell: {missing:?}"
    );
    assert!(
        orphan.is_empty(),
        "in assets/icons but not declared — dead weight in the depot: {orphan:?}"
    );
    assert_eq!(
        declared.len(),
        client::ui::icons::STEMS.len(),
        "icons::STEMS has a duplicate"
    );
}

/// Every wedge on both wheels has a picture.
///
/// The item half below reads `content/items.toml` because content grows; this
/// half reads the two verb/shape maps because *code* grows, and the failure
/// is the same one from the other end. Add a seventh shape or a fifth verb,
/// forget the bake, and nothing breaks loudly: `Icons::shape`/`verb` return
/// `None`, `glyph` falls back to the label, and the wheel quietly goes back
/// to being the text wheel these icons replaced.
#[test]
fn every_wedge_on_both_wheels_has_an_icon() {
    let declared: std::collections::BTreeSet<&str> =
        client::ui::icons::STEMS.iter().copied().collect();

    let mut missing = Vec::new();
    for shape in client::ui::build::SHAPES {
        let stem = client::ui::build::shape_icon(shape);
        if !declared.contains(stem) {
            missing.push(format!("shape {shape} -> {stem}"));
        }
    }
    for verb in client::ui::hammer::VERBS {
        let stem = client::ui::hammer::verb_icon(verb);
        if !declared.contains(stem) {
            missing.push(format!("{} -> {stem}", client::ui::hammer::label(verb)));
        }
    }
    assert!(
        missing.is_empty(),
        "wheel segments whose icon stem is not in icons::STEMS — these draw \
         their label instead of a glyph:\n{}",
        missing.join("\n")
    );
}

/// Every item the shard can send has a picture.
///
/// Reads `content/items.toml` directly rather than trusting a list: the whole
/// failure this prevents is content growing an item that the client then
/// draws as a clipped word in a 44 px cell.
#[test]
fn every_item_in_the_content_has_an_icon() {
    let toml = std::fs::read_to_string("../../content/items.toml").expect("content/items.toml");
    let declared: std::collections::BTreeSet<&str> =
        client::ui::icons::STEMS.iter().copied().collect();

    let mut missing = Vec::new();
    for line in toml.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("name = \"") else {
            continue;
        };
        let Some(name) = rest.strip_suffix('"') else {
            continue;
        };
        let stem = client::ui::icons::stem(name);
        if !declared.contains(stem.as_str()) {
            missing.push(format!("{name} -> {stem}"));
        }
    }
    assert!(
        missing.is_empty(),
        "content items with no icon (bake them into assets/icons and add the \
         stem to icons::STEMS):\n{}",
        missing.join("\n")
    );
}

// ---------------------------------------------------------------------------
// §H · the two items the mouse is modal on must exist
//
// `crate::ui::hold` decides what left and right click mean by normalising the
// held item's DISPLAY NAME, because that is the only thing on the wire —
// `protocol::ItemCatalog` carries names and nothing else. That works, and it
// has one failure mode worth a gate: rename the item in `content/items.toml`
// and nothing breaks loudly. `held` simply returns `Other` forever, the ghost
// stops appearing, right-click stops opening the wheel, and left click goes
// back to swinging. Every one of those is a silent revert to the old
// behaviour, which is the worst shape a regression can have.

#[test]
fn the_content_still_names_the_plan_and_the_hammer() {
    let toml = std::fs::read_to_string("../../content/items.toml").expect("content/items.toml");
    let stems: std::collections::BTreeSet<String> = toml
        .lines()
        .filter_map(|l| l.trim().strip_prefix("name = \"")?.strip_suffix('"'))
        .map(client::ui::icons::stem)
        .collect();

    for want in [client::ui::hold::PLAN_STEM, client::ui::hold::HAMMER_STEM] {
        assert!(
            stems.contains(want),
            "no item in the content normalises to `{want}` — `ui::hold` would \
             decide nothing is ever held, and the mouse would silently revert \
             to a swing. Rename the constant with the item, in one commit."
        );
    }
}

// ---------------------------------------------------------------------------
// §I · the 44 px cell's fallback label
//
// A cell with no baked icon prints the item's name (the dark-panel rule: an
// empty cell says nothing). The cell clips, so nothing bleeds over borders —
// but a clip cuts mid-glyph and `Gunpowde` reads as a typo, which is the
// operator-found half of `NOW.md` §0p2 item 3. `cell_abbrev` cuts on purpose
// instead: within budget passes through, over budget is cut with a trailing
// `.` that says the cut was chosen. Pure arithmetic, so it is gated here in
// the code tier rather than trusted behind a window.

#[test]
fn a_name_that_fits_the_cell_is_left_alone() {
    for name in ["Wood", "Stone", "Torch", "#12", "#65535"] {
        assert_eq!(
            craft::cell_abbrev(name, craft::CELL_LINE_CHARS),
            name,
            "a fitting name must pass through untouched"
        );
    }
}

#[test]
fn the_operators_three_names_read_as_abbreviations_not_typos() {
    // The three names the first look found bleeding (`NOW.md` §0p2 item 3).
    // Each cut ends in `.`, so the cell reads as deliberate.
    assert_eq!(craft::cell_abbrev("Gunpowder", 7), "Gunpow.");
    assert_eq!(craft::cell_abbrev("Workbench", 7), "Workbe.");
    assert_eq!(craft::cell_abbrev("Metal Fragments", 7), "Metal Fragme.");
}

#[test]
fn no_emitted_word_exceeds_the_line_budget() {
    // Names off the content's own shape: multi-word, long single words, and
    // the degenerate budgets. Every word of every result fits its line, so
    // the clip never gets to cut mid-glyph. Budget 1 is in the sweep since
    // the guard landed: it used to emit a 2-char `X.`, and the sweep used
    // to start at 2 — the contract and its gate had the same blind spot.
    for name in [
        "Metal Fragments",
        "Low Grade Fuel",
        "Sheet Metal Door",
        "Antidisestablishmentarianism",
        "a",
    ] {
        for budget in [1usize, 2, 4, 7, 12] {
            let out = craft::cell_abbrev(name, budget);
            for word in out.split(' ') {
                assert!(
                    word.chars().count() <= budget,
                    "`{word}` (of `{out}`, budget {budget}) still overflows the line"
                );
            }
        }
    }
}

#[test]
fn a_one_character_budget_keeps_the_glyph_and_drops_the_marker() {
    // No room for `.`: the first glyph alone is the only emission that
    // holds the budget, and it beats a lone `.` that says nothing.
    assert_eq!(craft::cell_abbrev("Gunpowder", 1), "G");
    assert_eq!(craft::cell_abbrev("Metal Fragments", 1), "M F");
    // Chars, not bytes, in the degenerate case too.
    assert_eq!(craft::cell_abbrev("Ébénisterie", 1), "É");
    // A word that fits a budget of one still passes through untouched.
    assert_eq!(craft::cell_abbrev("a", 1), "a");
}

// ---------------------------------------------------------------------------
// §J · the L verb's target — a lock bolts where the SIM says it bolts
//
// Locks-on-boxes landed in the sim (`DOORS.md` §9.8) while the client's `L`
// kept targeting doors alone. The fix is deliberately not a second list: the
// pick carries the archetype the deploy sync named (`Pick::arch`), and
// `keypad::lock_target` asks `sim_core::deploy::lockable` — the predicate
// the sim's own lock verb gates on. So the load-bearing test here is the
// agreement sweep, not the box case: a lockable set restated in the client
// would pass a box test today and drift silently on the next archetype.

use client::ui::interact as pick_mod;
use client::ui::keypad::{lock_target, LockTarget};
use sim_core::deploy::{DeployDef, DeployRec, ARCH_BOX, ARCH_DOOR, ARCH_LOCK};

/// A pick as `resolve` would fill it for a deploy record: address, sync'd
/// archetype, and the wire's lock mirror bit.
fn lock_pick(arch: u8, has_lock: bool) -> pick_mod::Pick {
    pick_mod::Pick {
        arch,
        has_lock,
        cx: 3,
        cz: 4,
        level: 1,
        loc: 2,
        ..Default::default()
    }
}

#[test]
fn the_resolver_hands_l_the_arch_the_sync_named() {
    // One box record, aimed at. `resolve` must stamp the pick with the
    // archetype from the def table — the fact `lock_target` runs on.
    let mut defs = DeployContent::EMPTY;
    defs.defs[0] = DeployDef {
        arch: ARCH_BOX,
        ..DeployDef::INERT
    };
    defs.def_count = 1;
    let recs = [DeployRec {
        cx: 2,
        cz: 2,
        row: 0,
        owner: 1,
        hp: 100,
        has_lock: true,
        locked: true,
        ..DeployRec::default()
    }];
    let cell = 2.0 * sim_core::build::BUILD_CELL_M + sim_core::build::BUILD_CELL_M * 0.5;
    let p = pick_mod::resolve(
        pick_mod::Aim::new(cell, cell - 2.0, 0.0, 1.0),
        &recs,
        &defs,
        1,
        &[],
    );
    assert_eq!(p.verb, pick_mod::Verb::Box);
    assert_eq!(p.arch, ARCH_BOX, "the sync's archetype rides the pick");
    assert!(p.has_lock && p.locked, "the wire's mirror bits ride it too");
    // And the whole chain: this pick opens the keypad at the box's own
    // plane address, exactly as a door's would at its edge.
    assert_eq!(lock_target(&p), LockTarget::Pad(2, 2, 0, 0));
}

#[test]
fn a_box_with_a_lock_opens_the_keypad_where_a_door_does() {
    // Same flow, both archetypes, addresses preserved verbatim — the sim
    // answers refusals at the address the record carries, so the client
    // must not normalize it.
    assert_eq!(
        lock_target(&lock_pick(ARCH_DOOR, true)),
        LockTarget::Pad(3, 4, 1, 2)
    );
    assert_eq!(
        lock_target(&lock_pick(ARCH_BOX, true)),
        LockTarget::Pad(3, 4, 1, 2)
    );
    // Bare lockables prompt for a lock rather than opening an empty pad.
    assert_eq!(lock_target(&lock_pick(ARCH_DOOR, false)), LockTarget::Bare);
    assert_eq!(lock_target(&lock_pick(ARCH_BOX, false)), LockTarget::Bare);
    // A default pick (nothing in reach) offers nothing, lock bit or not.
    assert_eq!(lock_target(&pick_mod::Pick::default()), LockTarget::None);
}

#[test]
fn the_lockable_set_is_the_sims_not_a_second_list() {
    // The agreement sweep, over every archetype the schema could name
    // (`ARCH_LOCK` is the highest today; the headroom past it costs
    // nothing and catches an addition that skips a number). If the sim
    // grows or shrinks `lockable`, this fails HERE rather than shipping a
    // keypad the sim refuses — the mirror property itself is the gate.
    for arch in 0..=(ARCH_LOCK + 8) {
        let opens = lock_target(&lock_pick(arch, true)) == LockTarget::Pad(3, 4, 1, 2);
        assert_eq!(
            opens,
            sim_core::deploy::lockable(arch),
            "arch {arch}: the client and `deploy::lockable` disagree"
        );
        // And nothing bare ever opens a pad, lockable or not.
        assert!(
            !matches!(lock_target(&lock_pick(arch, false)), LockTarget::Pad(..)),
            "arch {arch}: an empty pad opened with no lock bolted on"
        );
    }
}

#[test]
fn a_box_prompt_advertises_its_keypad_like_a_door() {
    let mut p = pick_mod::Pick {
        verb: pick_mod::Verb::Box,
        arch: ARCH_BOX,
        ..Default::default()
    };
    // Bare: the verb alone — no keypad to name (lock v1's door rule).
    assert_eq!(p.prompt(), "[E] OPEN BOX");
    // Bolted but not armed: the keypad is named, LOCKED is not claimed.
    p.has_lock = true;
    assert!(p.prompt().contains("[L] KEYPAD"), "{}", p.prompt());
    assert!(
        !p.prompt().contains("LOCKED"),
        "an unarmed lock is not a locked box: {}",
        p.prompt()
    );
    // Armed: both, and `E` still offers the open — whether it succeeds is
    // the sim's verdict, not the prompt's.
    p.locked = true;
    assert!(p.prompt().contains("LOCKED"), "{}", p.prompt());
    assert!(p.prompt().contains("[L] KEYPAD"), "{}", p.prompt());
    assert!(p.prompt().starts_with("[E] OPEN BOX"), "{}", p.prompt());
}

#[test]
fn a_cut_is_marked_and_a_multibyte_name_is_cut_on_chars_not_bytes() {
    let out = craft::cell_abbrev("Gunpowder", 7);
    assert!(out.ends_with('.'), "a cut word must end in `.`");
    // chars, not bytes: a name with multibyte glyphs must not split one.
    let out = craft::cell_abbrev("Ébénisterie", 7);
    assert_eq!(out, "Ébénis.");
}

// ---------------------------------------------------------------------------
// §K · the hammer's wheel — four live verbs, firing on release
//
// The second radial (`NOW.md` §0p2 item 1). What is gated here is what could
// be silently wrong: the segment set (a rotate wedge would be a dead button;
// a missing verb would be a mechanic no wheel reaches), the pick arithmetic
// (§D's, at this ring's own count), and `hammer::act` — the store filters
// that keep a release named PICK UP from felling a wall, since pick-up and
// demolish are ONE wire verb split by the store bit (`ACT_DEMOLISH`), which
// is exactly the positional-payload class `CLAUDE.md` warns about.

use client::ui::hammer::{self, Act, Verb as HVerb};
use client::ui::structure::{Store, Target};
use sim_core::build::PieceDef;
use sim_core::limits::MAX_PIECE_COSTS;

/// A wood→stone→metal ladder, one shape — enough for the rung question.
fn hammer_piece_defs() -> (BuildContent, u16) {
    let mut c = BuildContent::EMPTY;
    for (i, material) in [MAT_WOOD, MAT_STONE, MAT_METAL].into_iter().enumerate() {
        c.pieces[i] = PieceDef {
            shape: SHAPE_WALL,
            material,
            hp: 500,
            n_costs: 0,
            costs: [(0, 0); MAX_PIECE_COSTS],
        };
    }
    c.piece_count = 3;
    (c, 3)
}

fn target(store: Store, row: u8, hp: u16, hp_max: u16) -> Target {
    Target {
        store,
        cx: 7,
        cz: 9,
        level: 1,
        loc: 2,
        row,
        hp,
        hp_max,
    }
}

#[test]
fn the_ring_is_four_distinct_verbs_and_rotate_is_not_one() {
    // Four segments, each verb exactly once. Rotate has no segment and no
    // variant to give one to — a placed piece has no facing, so a rotate
    // wedge would teach a verb the game does not have (§0p2 item 0).
    assert_eq!(hammer::VERBS.len(), 4);
    for v in [
        HVerb::Upgrade,
        HVerb::Repair,
        HVerb::Demolish,
        HVerb::PickUp,
    ] {
        assert_eq!(
            hammer::VERBS.iter().filter(|w| **w == v).count(),
            1,
            "{v:?} must appear exactly once"
        );
        assert!(
            !hammer::label(v).is_empty(),
            "a blank wedge is the dark-panel defect"
        );
        assert!(!hammer::blurb(v).is_empty());
    }
}

#[test]
fn the_hammer_pick_is_the_shape_wheels_arithmetic_at_four() {
    let r = Rings::default();
    // Dead centre and past the rim release without firing.
    assert_eq!(hammer::pick(0.0, 0.0, r), None);
    assert_eq!(hammer::pick(0.0, r.dead - 1.0, r), None);
    assert_eq!(hammer::pick(0.0, r.rim + 1.0, r), None);
    // Four quadrants, clockwise from up — and each segment's own centre
    // angle picks it, the same drawn-vs-picked agreement §D holds for the
    // shapes.
    let mid = (r.dead + r.rim) * 0.5;
    assert_eq!(hammer::pick(0.0, mid, r), Some(0));
    assert_eq!(hammer::pick(mid, 0.0, r), Some(1));
    assert_eq!(hammer::pick(0.0, -mid, r), Some(2));
    assert_eq!(hammer::pick(-mid, 0.0, r), Some(3));
    for i in 0..hammer::VERBS.len() {
        let a = build::segment_angle(i, hammer::VERBS.len());
        assert_eq!(hammer::pick(a.sin() * mid, a.cos() * mid, r), Some(i));
    }
}

#[test]
fn each_verb_acts_only_through_its_own_store() {
    let (defs, have) = hammer_piece_defs();
    let piece = target(Store::Piece, 0, 250, 500);
    let deploy = target(Store::Deploy, 0, 250, 500);

    // Demolish fells a PIECE (deploy bit false) and refuses a deployable —
    // the same four numbers with the bit flipped would silently pick the
    // box up instead, which is the whole reason the filter exists.
    assert_eq!(
        hammer::act(HVerb::Demolish, Some(&piece), &defs, have),
        Act::Demolish {
            deploy: false,
            cx: 7,
            cz: 9,
            level: 1,
            loc: 2
        }
    );
    assert!(matches!(
        hammer::act(HVerb::Demolish, Some(&deploy), &defs, have),
        Act::Say(_)
    ));

    // Pick-up lifts a DEPLOYABLE (deploy bit true) and refuses a piece.
    assert_eq!(
        hammer::act(HVerb::PickUp, Some(&deploy), &defs, have),
        Act::Demolish {
            deploy: true,
            cx: 7,
            cz: 9,
            level: 1,
            loc: 2
        }
    );
    assert!(matches!(
        hammer::act(HVerb::PickUp, Some(&piece), &defs, have),
        Act::Say(_)
    ));

    // Repair addresses both stores, and the bit follows the store.
    assert_eq!(
        hammer::act(HVerb::Repair, Some(&piece), &defs, have),
        Act::Repair {
            deploy: false,
            cx: 7,
            cz: 9,
            level: 1,
            loc: 2
        }
    );
    assert_eq!(
        hammer::act(HVerb::Repair, Some(&deploy), &defs, have),
        Act::Repair {
            deploy: true,
            cx: 7,
            cz: 9,
            level: 1,
            loc: 2
        }
    );
}

#[test]
fn upgrade_picks_the_next_rung_the_way_u_does() {
    let (defs, have) = hammer_piece_defs();
    // Wood (row 0) upgrades into stone — `structure::next_material`'s
    // answer, the same one press `U` sends.
    assert_eq!(
        hammer::act(
            HVerb::Upgrade,
            Some(&target(Store::Piece, 0, 500, 500)),
            &defs,
            have
        ),
        Act::Upgrade {
            cx: 7,
            cz: 9,
            level: 1,
            loc: 2,
            material: MAT_STONE
        }
    );
    // Metal (row 2) is the top of the ladder.
    assert_eq!(
        hammer::act(
            HVerb::Upgrade,
            Some(&target(Store::Piece, 2, 500, 500)),
            &defs,
            have
        ),
        Act::Say("nothing to upgrade into")
    );
    // A deployable has no ladder — `upgrade_near`'s own sentence.
    assert_eq!(
        hammer::act(
            HVerb::Upgrade,
            Some(&target(Store::Deploy, 0, 500, 500)),
            &defs,
            have
        ),
        Act::Say("that is not a building piece")
    );
}

#[test]
fn refusals_before_the_round_trip_match_the_key_paths() {
    let (defs, have) = hammer_piece_defs();
    // Nothing in reach: every verb says so instead of sending.
    for v in hammer::VERBS {
        assert!(
            matches!(hammer::act(v, None, &defs, have), Act::Say(_)),
            "{v:?} with no target must refuse, not send"
        );
    }
    // An intact piece has nothing to mend — `repair_near`'s guard.
    assert_eq!(
        hammer::act(
            HVerb::Repair,
            Some(&target(Store::Piece, 0, 500, 500)),
            &defs,
            have
        ),
        Act::Say("not damaged")
    );
    // An undripped row (max 0) sends and lets the sim answer, because the
    // client does not know the maximum — also `repair_near`'s rule.
    assert!(matches!(
        hammer::act(
            HVerb::Repair,
            Some(&target(Store::Piece, 0, 100, 0)),
            &defs,
            have
        ),
        Act::Repair { .. }
    ));
}

// ---------------------------------------------------------------------------
// §L · the front door — the splash's handoff, and the browser's filters
//
// Two screens' arithmetic, both of it new, and both of it the kind that has
// no natural symptom.
//
// The splash's is one bit wide and decides which screen a launch opens on. It
// used to be a branch in `main` before any window existed, where the only way
// to check it was to launch the game two ways and watch. Get it wrong in the
// `chosen` direction and a launcher join asks the player to pick a shard they
// already picked; get it wrong the other way and a bare `gates` connects to
// whatever default address was compiled in, with no menu ever shown.
//
// The browser's is worse, because every one of its failures LOOKS like data.
// A sort that puts unknown counts first reads as a shard with nobody on it
// being the busiest. A filter that hides the Direct row reads as a fetch
// failure. A favourite keyed on an empty id lights up several rows at once.
// None of those crash, none of them warn, and all of them are a screenshot
// away from looking correct.

use client::ui::boot::{Boot, Next};
// `Cat` is aliased because `ui::craft` has one too and this file already
// imports it — two rails, two vocabularies, one module namespace.
use client::ui::servers::{self, Cat as SCat, Favourites, Filter, Listing, MAX_FAVOURITES};
use client::ui::MAX_QUERY_CHARS;

fn shard(name: &str, players: Option<u32>) -> Listing {
    Listing {
        id: format!("id-{name}"),
        name: name.to_string(),
        addr: format!("{name}.example:4433"),
        map: None,
        players,
        max_players: Some(100),
        ping_ms: Some(20),
        status_url: None,
        direct: false,
    }
}

/// The Direct row plus three shards at three populations.
fn listing() -> Vec<Listing> {
    vec![
        Listing::direct("127.0.0.1:4433"),
        shard("quiet", Some(0)),
        shard("busy", Some(90)),
        shard("full", Some(100)),
        shard("unknown", None),
    ]
}

fn names(rows: &[Listing], picked: &[usize]) -> Vec<String> {
    picked.iter().map(|i| rows[*i].name.clone()).collect()
}

// ---- the splash ----------------------------------------------------------

#[test]
fn the_splash_lifts_on_both_waits_and_not_on_either() {
    // The asset server and the launcher socket know nothing about each other
    // and finish in either order, which is why this is an `&&` and not a
    // sequence — and why both orders are driven here.
    for (first, second) in [(true, false), (false, true)] {
        let mut b = Boot::new(false);
        b.warm = first;
        b.greeted = second;
        assert!(b.next().is_none(), "one wait answered is not ready");
        assert_eq!(b.fraction(), 0.5, "and the bar says exactly that");
    }
    let mut b = Boot::new(false);
    b.warm = true;
    b.greeted = true;
    assert_eq!(b.fraction(), 1.0);
    assert_eq!(b.next(), Some(Next::Menu));
}

#[test]
fn a_launcher_join_is_never_asked_to_choose_twice() {
    // `chosen` is `args::server_given`: the launcher's Servers window already
    // picked, so the splash hands to the connect screen and the menu is never
    // drawn. The inverse matters just as much — a bare `gates` must NOT
    // connect to its default address without ever showing a list.
    let ready = |chosen| {
        let mut b = Boot::new(chosen);
        b.warm = true;
        b.greeted = true;
        b
    };
    assert_eq!(ready(true).next(), Some(Next::Connect));
    assert_eq!(ready(false).next(), Some(Next::Menu));
    // And the bit does not leak into readiness: a chosen start still waits.
    assert!(Boot::new(true).next().is_none());
}

#[test]
fn the_splash_names_the_wait_it_is_actually_in() {
    // A stage that reports nothing is a stage nobody can diagnose from a
    // screenshot — `Progress::line`'s rule, one screen earlier. The four
    // states must be four distinct sentences.
    let line = |warm, greeted| {
        Boot {
            warm,
            greeted,
            chosen: false,
        }
        .line()
    };
    let all = [
        line(false, false),
        line(false, true),
        line(true, false),
        line(true, true),
    ];
    for (i, a) in all.iter().enumerate() {
        for b in all.iter().skip(i + 1) {
            assert_ne!(a, b, "two boot stages say the same thing");
        }
    }
}

// ---- the browser ---------------------------------------------------------

#[test]
fn the_list_is_sorted_busy_first_with_unknown_last() {
    // The reference's own order (its list reads 145, 110, 101, 54, …) and the
    // question the screen answers: where is there a game happening. The
    // failure this pins is `Option`'s natural ordering, which puts `None`
    // FIRST — a shard nobody could poll would head the list as if it were the
    // busiest thing on it.
    let rows = listing();
    let shown = servers::visible(&rows, &Filter::default(), &Favourites::default());
    assert_eq!(
        names(&rows, &shown),
        ["Direct", "full", "busy", "quiet", "unknown"],
        "busy first, unknown last, Direct pinned above all of it"
    );
}

#[test]
fn the_direct_row_survives_every_filter_except_the_search_box() {
    // The invariant the intro screen has carried since it existed: a shard
    // list that fails to load must never leave nothing to click. "Hide empty"
    // alone would take the Direct row away, because it has no population —
    // an artefact of it not being a published shard, not a fact about it.
    let rows = listing();
    let favs = Favourites::default();
    for filter in [
        Filter {
            hide_empty: true,
            ..default_filter()
        },
        Filter {
            hide_full: true,
            ..default_filter()
        },
        Filter {
            cat: SCat::Favourites,
            ..default_filter()
        },
    ] {
        let shown = servers::visible(&rows, &filter, &favs);
        assert_eq!(
            rows[shown[0]].name, "Direct",
            "{filter:?} hid the direct row"
        );
    }

    // The query is the deliberate exception: a row that survived every search
    // reads as a broken search box. The safety net is one keystroke away and
    // `empty_reason` is what says so.
    let typed = Filter {
        query: "busy".into(),
        ..default_filter()
    };
    let shown = servers::visible(&rows, &typed, &favs);
    assert_eq!(names(&rows, &shown), ["busy"]);
    assert!(servers::empty_reason(&typed, rows.len(), shown.len()).is_none());
}

fn default_filter() -> Filter {
    Filter::default()
}

#[test]
fn empty_and_unknown_are_not_the_same_thing() {
    // The honesty rule `shardlist` states, applied to a filter: a shard that
    // failed its status poll has `None`, not zero, and "hide empty" must not
    // treat the two alike — that would hide every shard the network happened
    // to miss and read as them all being deserted.
    let rows = listing();
    let favs = Favourites::default();
    let shown = servers::visible(
        &rows,
        &Filter {
            hide_empty: true,
            ..Filter::default()
        },
        &favs,
    );
    assert_eq!(names(&rows, &shown), ["Direct", "full", "busy", "unknown"]);

    let shown = servers::visible(
        &rows,
        &Filter {
            hide_full: true,
            ..Filter::default()
        },
        &favs,
    );
    assert_eq!(names(&rows, &shown), ["Direct", "busy", "quiet", "unknown"]);
}

#[test]
fn a_search_reads_only_what_the_row_prints() {
    // A search that matched on a hidden field would hide rows on evidence the
    // player cannot see. Name, address and map — the three things drawn.
    let mut rows = listing();
    rows[2].map = Some("island 20260731".into());
    let favs = Favourites::default();
    let hits = |q: &str| {
        let f = Filter {
            query: q.into(),
            ..Filter::default()
        };
        names(&rows, &servers::visible(&rows, &f, &favs))
    };
    assert_eq!(hits("BUSY"), ["busy"], "case-insensitive");
    assert_eq!(hits("island"), ["busy"], "the map is searched");
    assert_eq!(hits("example:4433").len(), 4, "the address is searched");
    assert!(hits("nothing-here").is_empty());
}

#[test]
fn a_star_binds_to_a_row_and_survives_a_round_trip() {
    let rows = listing();
    let mut favs = Favourites::default();
    assert!(favs.toggle(&rows[2].id));
    assert!(favs.has("id-busy") && !favs.has("id-quiet"));

    // The category is a filter like any other, and the Direct row is still
    // exempt from it.
    let f = Filter {
        cat: SCat::Favourites,
        ..Filter::default()
    };
    assert_eq!(
        names(&rows, &servers::visible(&rows, &f, &favs)),
        ["Direct", "busy"]
    );
    // The rail counts the whole list, never the filtered one: a tab that read
    // `0` beside the thing you have to click to see anything is the opposite
    // of what a count on a tab is for.
    assert_eq!(servers::counts(&rows, &favs), [5, 1]);

    // Off the disk and back, which is the shape `config::parse` hands over.
    let round = Favourites::from_disk(favs.ids().to_vec());
    assert_eq!(round, favs);
    // Un-starring sticks, including down to empty — the case that needs the
    // writer to emit `favourites = ""` rather than omit the key.
    assert!(favs.toggle("id-busy"));
    assert!(favs.is_empty());
}

#[test]
fn a_hand_edited_favourites_list_cannot_reach_a_state_a_click_cannot() {
    // `config::parse` deliberately does no range work (its header says so),
    // so this is where the bound and the deduplication live. A file with a
    // thousand ids, blanks and repeats must load as something the star could
    // have produced.
    let junk: Vec<String> = std::iter::repeat_n("dup".to_string(), 5)
        .chain((0..MAX_FAVOURITES * 2).map(|i| format!("id{i}")))
        .chain(["".to_string(), "".to_string()])
        .collect();
    let favs = Favourites::from_disk(junk);
    assert_eq!(favs.len(), MAX_FAVOURITES);
    assert!(!favs.has(""), "a blank id would light up every unnamed row");
    assert_eq!(
        favs.ids().iter().filter(|k| *k == "dup").count(),
        1,
        "a duplicate would need two clicks to un-star"
    );
}

#[test]
fn a_row_with_no_id_keys_its_star_on_its_address() {
    // The failure this prevents is one star lighting several rows: a document
    // that omits `id` would otherwise give every such row the SAME empty key.
    let mut s = shardlist_shard("a.example:4433");
    s.id = String::new();
    let row = Listing::from_shard(&s);
    assert_eq!(row.id, "a.example:4433");

    let mut favs = Favourites::default();
    favs.toggle(&row.id);
    let mut other = shardlist_shard("b.example:4433");
    other.id = String::new();
    assert!(!favs.has(&Listing::from_shard(&other).id));
}

fn shardlist_shard(addr: &str) -> client::shardlist::Shard {
    client::shardlist::Shard {
        id: "x".into(),
        name: "X".into(),
        addr: addr.into(),
        players: None,
        max_players: None,
        map: None,
        ping_ms: None,
        status_url: None,
    }
}

#[test]
fn a_poll_rewrites_the_count_and_the_direct_row_takes_none() {
    // Carried over from the row type this replaced, because the property is
    // the one that decides a redraw: the same count twice must not rebuild
    // the screen, and the Direct row has no published count to accept.
    let mut row = Listing::from_shard(&shardlist_shard("h:1"));
    let st = client::shardlist::parse_status(br#"{"players":3,"max_players":100}"#).unwrap();
    assert!(row.apply_status(&st), "the line changed and must redraw");
    assert_eq!(row.population(), "3/100");
    assert!(!row.apply_status(&st), "an unchanged line must not redraw");

    let mut direct = Listing::direct("127.0.0.1:4433");
    assert!(!direct.apply_status(&st));
    assert_eq!(direct.population(), "?", "never a count nobody published");
    assert_eq!(direct.ping(), "?");
}

#[test]
fn a_typed_query_is_bounded_and_clearable() {
    // Wall 4 applied to a field a player types into — the same cap the craft
    // panel's search box has, and for the same reason.
    let mut f = Filter::default();
    for _ in 0..MAX_QUERY_CHARS * 2 {
        f.push('x');
    }
    assert_eq!(f.query.chars().count(), MAX_QUERY_CHARS);
    assert!(
        !f.push('x'),
        "a full box reports no change and must not redraw"
    );
    assert!(!f.push('\n'), "a control character is not a search term");

    assert!(f.narrowed());
    f.clear();
    assert!(!f.narrowed() && f.query.is_empty() && f.cat == SCat::All);
    assert!(!f.backspace(), "an empty box reports no change");
}

#[test]
fn an_empty_list_says_which_kind_of_empty_it_is() {
    // A player who cannot tell a failed fetch from an over-narrow filter is
    // stuck. The fetch's own status line owns the first; this owns the rest.
    // A fetch that returned nothing: the status line owns that sentence.
    assert_eq!(servers::empty_reason(&Filter::default(), 0, 0), None);
    // Nothing narrowed, so an empty list is not the filter's doing.
    assert_eq!(servers::empty_reason(&Filter::default(), 5, 0), None);
    // ...and a list with rows on it is never described as empty, whatever
    // the filter says. The guard used to live in the one caller.
    let narrowed = Filter {
        query: "zz".into(),
        ..Filter::default()
    };
    assert_eq!(servers::empty_reason(&narrowed, 5, 3), None);
    let starred = Filter {
        cat: SCat::Favourites,
        ..Filter::default()
    };
    assert!(servers::empty_reason(&starred, 5, 0)
        .unwrap()
        .contains("star"));
    let typed = Filter {
        query: "zz".into(),
        ..Filter::default()
    };
    assert!(servers::empty_reason(&typed, 5, 0).unwrap().contains("zz"));
}

/// The claimed address must not depend on which door the client came through.
///
/// This is the regression the boot split introduced and it is exactly the
/// class `CLAUDE.md` warns about: a *field* that is right on one path and
/// silently default on another, with nothing failing. `Connecting::address`
/// was declared, documented, and written by nothing — the pre-window connect
/// passed `args::address()` straight to the session, so every join that went
/// through the menu sent `Address::GUEST` however the player had identified
/// themselves. Moving the launcher join onto that screen would have made a
/// menu-only bug the launcher's bug too, with a successful join either way.
///
/// `render/menu.rs::begin_connect` now resolves it from `Who`, which is one
/// call and covers both provenances. This pins the VALUE half — that the two
/// resolvers agree — and `the_connect_screen_still_claims_an_identity` below
/// pins the CALL SITE half, because the defect was never a wrong value. It
/// was a field nobody assigned.
#[test]
fn a_declared_identity_reaches_the_wire_by_either_route() {
    const ID: &str = "0x1111111111111111111111111111111111111111";
    let args = match client::args::parse(["--identity", ID].iter().map(|s| s.to_string())) {
        client::args::Parsed::Run(a) => a,
        other => panic!("expected Run, got {other:?}"),
    };
    // The command-line half, which is what the pre-window connect used.
    let from_args = args.address().expect("a well-formed identity parses");
    // The `Who` half, which is what every screen-driven connect uses now.
    let player = client::scry::Player::Declared(ID.to_string());
    let from_who =
        protocol::Address::from_hex(player.address().expect("declared carries one").as_bytes())
            .expect("the same string parses the same way");
    assert_eq!(from_args, from_who, "two doors, two different players");
    assert_ne!(
        from_args,
        protocol::Address::GUEST,
        "an identity that resolves to GUEST is the bug this pins"
    );

    // And no identity is a guest on both halves, rather than a parse failure.
    let bare = match client::args::parse(std::iter::empty()) {
        client::args::Parsed::Run(a) => a,
        other => panic!("expected Run, got {other:?}"),
    };
    assert_eq!(bare.address().unwrap(), protocol::Address::GUEST);
    assert_eq!(client::scry::Player::Anonymous.address(), None);
}

/// The other half, and a grep — because the defect is a call site.
///
/// The same instrument `tests/sound.rs` uses for the feed drain and §F uses
/// for the face, for the same stated reason: `connecting.address` compiles,
/// passes clippy and joins a shard successfully whether or not anything ever
/// writes it, and the only place a `GUEST` that should have been a player
/// shows up is on a shard's own records. A value test cannot see an
/// assignment that stopped happening; this can.
#[test]
fn the_connect_screen_still_claims_an_identity() {
    let src = std::fs::read_to_string("src/render/menu.rs").expect("readable menu source");
    let start = src
        .find("pub fn begin_connect")
        .expect("begin_connect must exist - it is the one connect the screens make");
    // To the end of the function: the next item at column zero.
    let body = &src[start..];
    let end = body[1..]
        .find("\npub fn ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];
    assert!(
        body.contains("connecting.address = "),
        "begin_connect no longer sets the address it sends. It compiles, it \
         joins, and every player on this path becomes a guest - which is \
         exactly how the field went unwritten the first time:\n{body}"
    );
    assert!(
        body.contains("who.0.address()"),
        "the address must come from `Who`, which carries BOTH provenances - \
         a declared `--identity` and a launcher-reported one. Reading the \
         flag alone is what left a signed-in player anonymous"
    );
}
