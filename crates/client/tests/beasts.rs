//! Gate: the fel beasts (`render/beasts.rs`) — the tier the arc picks, the
//! clip a gait picks, and the files `ci/rig_beast.py` wrote.
//!
//! The files are held to `tests/mob_mesh.rs`'s three facts for the box
//! massing (feet at the origin, nose to +Z, the size it claims) plus the
//! two a skinned model adds: one skin, and every clip the player resolves
//! by name.
#![cfg(feature = "render")]

use std::path::{Path, PathBuf};

use client::render::beasts::{
    gait_for, rate_of, tier_of, wants, BeastClip, BEAST_CARCASS_GLB, BEAST_GLB, BEAST_RATE_MAX,
    CLIP_MPS, FEL_PIG, FEL_WOLF,
};
use client::render::mobs::{Gait, PIG_H_M, WOLF_H_M};
use client_core::arc::ArcView;
use sim_core::works::{WORK_LIT, WORK_OPEN, WORK_SEALED};

fn asset_path(rel: &str) -> PathBuf {
    Path::new("../../assets").join(rel)
}

fn glb_json(rel: &str) -> serde_json::Value {
    let path = asset_path(rel);
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&raw[0..4], b"glTF", "{rel}: not a GLB");
    let len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&raw[20..20 + len]).expect("GLB JSON chunk")
}

/// The first primitive's POSITION bounds, glTF frame.
fn bounds(j: &serde_json::Value) -> ([f64; 3], [f64; 3]) {
    let acc = j["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
        .as_u64()
        .unwrap() as usize;
    let a = &j["accessors"][acc];
    let v = |k: &str| {
        let s = a[k].as_array().unwrap();
        [
            s[0].as_f64().unwrap(),
            s[1].as_f64().unwrap(),
            s[2].as_f64().unwrap(),
        ]
    };
    (v("min"), v("max"))
}

fn arc(states: &[(u8, u8)]) -> ArcView {
    let mut a = ArcView {
        count: states.len() as u8,
        ..ArcView::default()
    };
    for (i, (act, state)) in states.iter().enumerate() {
        a.works[i].known = true;
        a.works[i].def.act = *act;
        a.works[i].state = *state;
    }
    a
}

#[test]
fn the_tier_climbs_with_the_arc() {
    // Wipe day: nothing open, act I.
    assert_eq!(tier_of(&arc(&[(2, WORK_SEALED), (2, WORK_SEALED)])), 0);
    assert_eq!(tier_of(&ArcView::default()), 0, "no rows yet is act I");
    // The first work opens: act II.
    assert_eq!(tier_of(&arc(&[(2, WORK_OPEN), (2, WORK_SEALED)])), 1);
    assert_eq!(tier_of(&arc(&[(2, WORK_LIT), (2, WORK_OPEN)])), 1);
    // Every act-II work has burned.
    assert_eq!(tier_of(&arc(&[(2, WORK_LIT), (2, WORK_LIT)])), 2);
    // Act III is heavy whatever act II looks like.
    assert_eq!(tier_of(&arc(&[(2, WORK_OPEN), (3, WORK_OPEN)])), 2);
}

#[test]
fn the_gait_picks_the_clip_with_hysteresis() {
    let mps = CLIP_MPS[FEL_WOLF];
    assert_eq!(gait_for(0.0, mps, None), BeastClip::Idle);
    assert_eq!(gait_for(mps[0], mps, None), BeastClip::Walk);
    assert_eq!(gait_for(mps[1], mps, None), BeastClip::Trot);
    assert_eq!(gait_for(mps[2], mps, None), BeastClip::Run);
    // Just past the walk/trot boundary, a walking animal keeps walking...
    let edge = 0.5 * (mps[0] + mps[1]);
    assert_eq!(
        gait_for(edge + 0.05, mps, Some(BeastClip::Walk)),
        BeastClip::Walk
    );
    // ...and one clear of it trots.
    assert_eq!(
        gait_for(edge + 0.5, mps, Some(BeastClip::Walk)),
        BeastClip::Trot
    );
    // A bite or howl clip is no gait to hold on to.
    assert_eq!(gait_for(mps[2], mps, Some(BeastClip::Bite)), BeastClip::Run);

    let mut g = Gait::new(0);
    g.speed = mps[2];
    assert_eq!(wants(&g, mps, None), BeastClip::Run);
    g.bite = 0.2;
    assert_eq!(
        wants(&g, mps, None),
        BeastClip::Bite,
        "a bite plays over the run"
    );
    g.lie = 1.0;
    assert_eq!(
        wants(&g, mps, None),
        BeastClip::Lie,
        "a sleeper lies, bite or not"
    );

    assert_eq!(rate_of(BeastClip::Trot, mps[1], mps), 1.0);
    assert_eq!(rate_of(BeastClip::Run, 99.0, mps), BEAST_RATE_MAX);
    assert_eq!(rate_of(BeastClip::Idle, 3.0, mps), 1.0);
}

/// `CLIP_MPS` is the rigging script's table; a re-tuned gait in one and not
/// the other slides every paw.
#[test]
fn clip_speeds_match_the_rig_script() {
    let py = std::fs::read_to_string("../../ci/rig_beast.py").expect("ci/rig_beast.py");
    for (sp, name) in [(FEL_PIG, "pig"), (FEL_WOLF, "wolf")] {
        let at = py.find(&format!("\"{name}\": {{")).expect("species block");
        let block = &py[at..];
        for (i, clip) in ["Walk", "Trot", "Run"].iter().enumerate() {
            let k = block.find(&format!("\"{clip}\": (")).expect("gait row") + clip.len() + 5;
            let num: String = block[k..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let v: f32 = num.parse().unwrap();
            assert_eq!(
                v, CLIP_MPS[sp][i],
                "{name} {clip}: rig_beast.py says {v} m/s, beasts.rs says {}",
                CLIP_MPS[sp][i]
            );
        }
    }
}

#[test]
fn every_fel_model_stands_faces_and_plays() {
    for (sp, h_m) in [(FEL_PIG, PIG_H_M as f64), (FEL_WOLF, WOLF_H_M as f64)] {
        for rel in BEAST_GLB[sp] {
            let j = glb_json(rel);
            assert_eq!(
                j["skins"].as_array().map(Vec::len),
                Some(1),
                "{rel}: one skin"
            );
            let names: Vec<&str> = j["animations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["name"].as_str().unwrap())
                .collect();
            for c in BeastClip::ALL {
                assert!(
                    names.contains(&c.name()),
                    "{rel}: no {:?} clip in {names:?}",
                    c
                );
            }
            for img in j["images"].as_array().unwrap() {
                assert_eq!(img["mimeType"], "image/ktx2", "{rel}: maps not packed");
            }
            let (lo, hi) = bounds(&j);
            assert!(lo[1].abs() < 0.01, "{rel}: feet at y = {:.3}, not 0", lo[1]);
            assert!(hi[2] > -lo[2], "{rel}: the long end is behind — facing -Z?");
            // The withers are scaled to the species' height; ears, horns and
            // spines stand above it, never more than a quarter.
            assert!(
                hi[1] > 0.95 * h_m && hi[1] < 1.25 * h_m,
                "{rel}: top {:.2} m against {h_m} at the withers",
                hi[1]
            );
        }
    }
}

#[test]
fn every_fel_carcass_lies_in_the_carcass_frame() {
    for row in BEAST_CARCASS_GLB {
        for rel in row {
            let j = glb_json(rel);
            assert!(j.get("skins").is_none(), "{rel}: a carcass is static");
            let (lo, hi) = bounds(&j);
            assert!(
                lo[1].abs() < 0.01,
                "{rel}: not on the ground ({:.3})",
                lo[1]
            );
            assert!(
                hi[0].abs() < 0.01,
                "{rel}: back not at x = 0 ({:.3})",
                hi[0]
            );
            assert!(lo[0] < -0.5, "{rel}: legs do not reach out toward -x");
        }
    }
}
