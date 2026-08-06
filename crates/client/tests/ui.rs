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

use client::ui::build::{self, Hover, Rings, MATERIALS, SHAPES};
use client::ui::craft::{self, Cat, Facts};
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
    for n in [3usize, 6] {
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
    for n in [3usize, 6] {
        for i in 0..n {
            let a = build::segment_angle(i, n);
            assert_eq!(build::segment(a.sin(), a.cos(), n), i);
        }
    }
}

#[test]
fn the_rings_and_the_dead_centre() {
    let r = Rings::default();
    assert!(r.dead < r.split && r.split < r.rim);
    // Dead centre: a release here keeps what was chosen and picks nothing.
    assert_eq!(build::pick(0.0, 0.0, r), None);
    assert_eq!(build::pick(0.0, r.dead - 1.0, r), None);
    // Past the rim the pointer has left the wheel.
    assert_eq!(build::pick(0.0, r.rim + 1.0, r), None);
    // Inner ring is the material, outer is the shape — the other way round
    // would put the six-way ring where the thumb has least room.
    assert_eq!(
        build::pick(0.0, (r.dead + r.split) * 0.5, r),
        Some(Hover::Material(0))
    );
    assert_eq!(
        build::pick(0.0, (r.split + r.rim) * 0.5, r),
        Some(Hover::Shape(0))
    );
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
