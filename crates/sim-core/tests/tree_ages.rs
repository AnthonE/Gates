//! Trees come in two sizes and a fresh wipe's forest is not all one age
//! (tree size classes v0, tree ages v0): about 30 % of trees are the small
//! class and pay wood by their size, about 40 % stand young at the wipe and
//! grow up over `WILD_GROW_TICKS`, and a felled young tree's sapling never
//! outgrows the stand it came from.

use sim_core::gather::{
    self, SlotLives, GROW_STAGES, SAPLING_PM, TREE_GROW_TICKS, WILD_GROW_TICKS, WILD_YOUNG_PM,
};
use sim_core::terrain::{
    self, Occupant, ScatterTable, TREE_LARGE_SCALE, TREE_SCALE_MEAN, TREE_SMALL_BYTES,
    TREE_SMALL_SCALE,
};
use sim_core::world::EventQueue;

const SEED: u64 = 20_260_731;

#[test]
fn the_mean_tree_scale_is_the_byte_draws_mean() {
    let sum: f32 = (0..=255u8)
        .map(|b| terrain::slot_scale(Occupant::Tree, b))
        .sum();
    let mean = sum / 256.0;
    assert!(
        (mean - TREE_SCALE_MEAN).max(TREE_SCALE_MEAN - mean) < 0.0005,
        "TREE_SCALE_MEAN {TREE_SCALE_MEAN} against the draw's {mean}"
    );
    // The two classes do not overlap, and each spans what it says.
    let small = terrain::slot_scale(Occupant::Tree, TREE_SMALL_BYTES - 1);
    let large = terrain::slot_scale(Occupant::Tree, TREE_SMALL_BYTES);
    assert!(small <= TREE_SMALL_SCALE.1 + 1e-6 && large >= TREE_LARGE_SCALE.0 - 1e-6);
    assert_eq!(terrain::slot_scale(Occupant::Tree, 0), TREE_SMALL_SCALE.0);
    assert!(terrain::slot_scale(Occupant::Tree, 255) <= terrain::SLOT_SCALE_MAX);
    // Everything else keeps its ±10 %.
    assert_eq!(terrain::slot_scale(Occupant::StoneNode, 0), 0.9);
}

#[test]
fn a_third_of_the_island_is_small_trees_and_the_wood_is_unchanged() {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    let (mut trees, mut small, mut wood) = (0u32, 0u32, 0u64);
    for cz in 0..terrain::CELLS_PER_SIDE {
        for cx in 0..terrain::CELLS_PER_SIDE {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant != Occupant::Tree {
                continue;
            }
            trees += 1;
            if s.scale < TREE_LARGE_SCALE.0 {
                small += 1;
            }
            wood += gather::tree_wood_pm(s.scale) as u64;
        }
    }
    assert!(trees > 1000, "only {trees} trees on the shipped seed");
    let small_pm = small * 1000 / trees;
    assert!(
        (250..=350).contains(&small_pm),
        "{small_pm}‰ of trees are small"
    );
    let mean_wood = wood / trees as u64;
    assert!(
        (970..=1030).contains(&mean_wood),
        "the island's mean tree pays {mean_wood}‰ of the row"
    );
    // A small tree pays less than a large one, at the same pay per hit.
    assert!(gather::tree_wood_pm(TREE_SMALL_SCALE.1) < gather::tree_wood_pm(TREE_LARGE_SCALE.0));
}

#[test]
fn a_fresh_wipe_stands_at_every_age_and_is_grown_a_day_later() {
    let (mut cells, mut young, mut saplings, mut nearly) = (0u32, 0u32, 0u32, 0u32);
    for cz in 0..200u16 {
        for cx in 0..200u16 {
            cells += 1;
            let at = gather::wild_grown_at(SEED, cx, cz);
            if at == 0 {
                assert_eq!(gather::wild_pm(SEED, cx, cz, 0), 1000);
                continue;
            }
            young += 1;
            assert!(at <= WILD_GROW_TICKS);
            let pm = gather::wild_pm(SEED, cx, cz, 0);
            assert!(pm < 1000, "a young tree stood full-grown at the wipe");
            if pm == SAPLING_PM {
                saplings += 1;
            }
            if pm >= 900 {
                nearly += 1;
            }
            // It only ever grows, and is grown by its tick.
            let mut last = pm;
            for k in 1..=GROW_STAGES {
                let now = WILD_GROW_TICKS * k / GROW_STAGES;
                let p = gather::wild_pm(SEED, cx, cz, now);
                assert!(p >= last, "a wild tree shrank");
                last = p;
            }
            assert_eq!(gather::wild_pm(SEED, cx, cz, at), 1000);
            assert_eq!(gather::wild_pm(SEED, cx, cz, WILD_GROW_TICKS), 1000);
        }
    }
    let young_pm = young as u64 * 1000 / cells as u64;
    assert!(
        young_pm.abs_diff(WILD_YOUNG_PM) <= 20,
        "{young_pm}‰ young against {WILD_YOUNG_PM}‰"
    );
    assert!(
        saplings > 0 && nearly > 0,
        "the wipe's ages do not span sapling to grown"
    );
}

#[test]
fn a_felled_young_tree_does_not_outgrow_its_stand() {
    let mut lives = SlotLives::new();
    let mut ev = EventQueue::default();
    // Two felled trees, due back at tick 100: one in a young stand that
    // is grown by 2,000,000, one in a grown stand.
    lives.harvest(5, 0, Occupant::Tree, 100, &mut ev);
    lives.harvest(7, 0, Occupant::Tree, 100, &mut ev);
    let wild = |cx: u16, _cz: u16| if cx == 5 { 2_000_000 } else { 0 };
    lives.respawn_due_unless(100, &mut ev, &|_, _| false, &wild);
    assert_eq!(lives.find(5, 0).unwrap().grown_at, 2_000_000);
    assert_eq!(lives.find(7, 0).unwrap().grown_at, 100 + TREE_GROW_TICKS);
    // The young stand's sapling waits small, then grows its last hour.
    assert_eq!(lives.tree_pm_at(SEED, 5, 0, 1_000_000), SAPLING_PM);
    assert_eq!(lives.tree_pm_at(SEED, 5, 0, 2_000_000), 1000);
}

#[test]
fn a_tree_the_store_has_not_felled_is_the_stands_age() {
    // Find a young cell; with no entry, the store answers the stand's age.
    let (cx, cz) = (0..200u16)
        .flat_map(|cz| (0..200u16).map(move |cx| (cx, cz)))
        .find(|&(cx, cz)| gather::wild_pm(SEED, cx, cz, 0) < 500)
        .expect("no young tree");
    let lives = SlotLives::new();
    assert_eq!(
        lives.tree_pm_at(SEED, cx, cz, 0),
        gather::wild_pm(SEED, cx, cz, 0)
    );
    assert_eq!(lives.tree_pm_at(SEED, cx, cz, WILD_GROW_TICKS), 1000);
}
