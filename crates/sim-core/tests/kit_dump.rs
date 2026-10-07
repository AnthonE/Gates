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

#[test]
fn the_landmark_kit_files_match_the_source() {
    use sim_core::landmark::{self, LandmarkKind};
    let files: [(LandmarkKind, &str); 5] = [
        (
            LandmarkKind::Mast,
            include_str!("../../../ci/kits/mark_mast.json"),
        ),
        (
            LandmarkKind::Ruin,
            include_str!("../../../ci/kits/mark_ruin.json"),
        ),
        (
            LandmarkKind::Tower,
            include_str!("../../../ci/kits/mark_tower.json"),
        ),
        (
            LandmarkKind::Stones,
            include_str!("../../../ci/kits/mark_stones.json"),
        ),
        (
            LandmarkKind::Yard,
            include_str!("../../../ci/kits/mark_yard.json"),
        ),
    ];
    assert_eq!(files.len(), landmark::DRESSED.len());
    for (kind, file) in files {
        assert!(landmark::DRESSED.contains(&kind));
        let mut out = String::new();
        landmark::dump(kind, &mut out).unwrap();
        assert!(
            file == out,
            "ci/kits/mark_{}.json is stale — regenerate it",
            landmark::slug(kind)
        );
    }
}
