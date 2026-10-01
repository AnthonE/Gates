//! The committed kit JSON the Blender dresser reads (`ci/kits/*.json`) is
//! exactly what the sim's tables say, so a model is never dressed around
//! boxes the game no longer collides with. On a red: re-run
//! `cargo run -p sim-core --example kit_dump -- <kit> > ci/kits/<kit>.json`
//! and `ci/site_kit.py gen` for the model.
#![allow(clippy::disallowed_types)]

#[test]
fn the_town_kit_file_matches_the_source() {
    let mut out = String::new();
    sim_core::town::dump(&mut out).unwrap();
    let file = include_str!("../../../ci/kits/town.json");
    assert!(file == out, "ci/kits/town.json is stale — regenerate it");
}

#[test]
fn the_ziggurat_kit_file_matches_the_source() {
    let mut out = String::new();
    sim_core::monument::dump(&mut out).unwrap();
    let file = include_str!("../../../ci/kits/ziggurat.json");
    assert!(
        file == out,
        "ci/kits/ziggurat.json is stale — regenerate it"
    );
}
