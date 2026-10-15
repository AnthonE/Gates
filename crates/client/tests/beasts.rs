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
    CLIP_MPS, FEL_PIG, FEL_STAG, FEL_WOLF,
};
use client::render::mobs::{Gait, PIG_H_M, WOLF_H_M};

/// The stag is scaled at its withers; `mobs::STAG_H_M` is to the top of the
/// skull, and the rack stands above that.
const STAG_WITHERS_M: f64 = 1.05;
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
    for (sp, name) in [(FEL_PIG, "pig"), (FEL_WOLF, "wolf"), (FEL_STAG, "stag")] {
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
    // (species, withers height, how far above it ears, spines or a rack may stand)
    for (sp, h_m, over) in [
        (FEL_PIG, PIG_H_M as f64, 1.25),
        (FEL_WOLF, WOLF_H_M as f64, 1.25),
        (FEL_STAG, STAG_WITHERS_M, 2.0),
    ] {
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
            // The withers are scaled to the species' height; ears, horns,
            // spines and antlers stand above it, by no more than `over`.
            assert!(
                hi[1] > 0.95 * h_m && hi[1] < over * h_m,
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

/// **An animal on a hillside leans with it**, and the right way round: on
/// ground rising toward +Z a beast facing +Z raises its nose, one facing −Z
/// dips it, and one facing +X rolls its left (+Z, uphill) side up — the
/// signs are the whole risk here. Off the terrain (on a foundation) it
/// stands level.
#[test]
fn a_beast_leans_with_the_slope_it_stands_on() {
    use bevy::math::{Quat, Vec2, Vec3};
    use client::render::mobs::{ground_tilt, tilt_quat, TILT_MAX_PITCH_RAD};
    use std::f32::consts::FRAC_PI_2;
    let hill = |_x: f32, z: f32| 0.5 * z;
    let feet = [0.34, -0.34];
    let at = Vec3::new(0.0, 0.0, 0.0);

    let up = ground_tilt(hill, at, 0.0, feet);
    assert!((up.x - 0.5f32.atan()).abs() < 1e-4, "facing uphill: {up:?}");
    assert!(up.y.abs() < 1e-4);
    // The drawn nose really goes up.
    assert!((tilt_quat(up) * Vec3::Z).y > 0.4);

    let down = ground_tilt(hill, at, std::f32::consts::PI, feet);
    assert!(down.x < -0.4, "facing downhill: {down:?}");

    // Facing +X, the slope is across it: its left (+X in body space) is
    // world −Z, downhill, so that side rolls down.
    let across = ground_tilt(hill, at, FRAC_PI_2, feet);
    assert!(
        across.x.abs() < 1e-4 && across.y < -0.3,
        "across: {across:?}"
    );
    let left = Quat::from_rotation_y(FRAC_PI_2) * tilt_quat(across) * Vec3::X;
    assert!(
        left.y < 0.0 && left.z < 0.0,
        "the downhill side dips: {left:?}"
    );

    // A cliff is clamped, and an animal standing on a floor stands level.
    let cliff = ground_tilt(|_, z| 5.0 * z, at, 0.0, feet);
    assert_eq!(cliff.x, TILT_MAX_PITCH_RAD);
    let floor = ground_tilt(hill, Vec3::new(0.0, 1.0, 0.0), 0.0, feet);
    assert_eq!(floor, Vec2::ZERO);
    assert_eq!(ground_tilt(|_, _| f32::NAN, at, 0.0, feet), Vec2::ZERO);
}

/// The GLB's JSON and its binary chunk.
fn glb(rel: &str) -> (serde_json::Value, Vec<u8>) {
    let raw = std::fs::read(asset_path(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let jlen = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    let j = serde_json::from_slice(&raw[20..20 + jlen]).expect("GLB JSON chunk");
    let at = 20 + jlen;
    let blen = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;
    (j, raw[at + 8..at + 8 + blen].to_vec())
}

/// An accessor's floats, tightly packed (what `ci/rig_beast.py` writes).
fn floats(j: &serde_json::Value, bin: &[u8], acc: u64) -> Vec<f32> {
    let a = &j["accessors"][acc as usize];
    assert_eq!(a["componentType"], 5126, "float accessors only");
    let v = &j["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
    let comp = match a["type"].as_str().unwrap() {
        "SCALAR" => 1,
        "VEC3" => 3,
        "VEC4" => 4,
        t => panic!("accessor type {t}"),
    };
    let off = v["byteOffset"].as_u64().unwrap_or(0) as usize
        + a["byteOffset"].as_u64().unwrap_or(0) as usize;
    let n = a["count"].as_u64().unwrap() as usize * comp;
    (0..n)
        .map(|i| f32::from_le_bytes(bin[off + 4 * i..off + 4 * i + 4].try_into().unwrap()))
        .collect()
}

/// **No clip puts a foot in the ground.** Every keyframe of every clip of
/// every fel model, posed bone by bone, with each toe joint held above
/// [`FOOT_FLOOR_M`]. The lie `ci/rig_beast.py` authored folded the wolves'
/// hind legs through the ground — the paws up to 27 cm under it — which no
/// one saw while wolves never lay down; `ci/fix_lie.py` re-solved it.
#[test]
fn no_clip_puts_a_foot_in_the_ground() {
    use bevy::math::{Mat4, Quat, Vec3};
    /// A toe joint may dip this far: a gait's planted paw sits a few
    /// centimetres either side of zero, a buried leg does not.
    const FOOT_FLOOR_M: f32 = -0.06;
    for rel in BEAST_GLB.iter().flatten() {
        let (j, bin) = glb(rel);
        let nodes = j["nodes"].as_array().unwrap();
        let mut parent = vec![None; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            for c in n["children"].as_array().into_iter().flatten() {
                parent[c.as_u64().unwrap() as usize] = Some(i);
            }
        }
        let v3 = |n: &serde_json::Value, k: &str, d: f32| {
            n[k].as_array().map_or(Vec3::splat(d), |a| {
                Vec3::new(
                    a[0].as_f64().unwrap() as f32,
                    a[1].as_f64().unwrap() as f32,
                    a[2].as_f64().unwrap() as f32,
                )
            })
        };
        let toes: Vec<usize> = (0..nodes.len())
            .filter(|&i| {
                nodes[i]["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("Toe."))
            })
            .collect();
        assert_eq!(toes.len(), 4, "{rel}: four toe joints");
        for anim in j["animations"].as_array().unwrap() {
            let name = anim["name"].as_str().unwrap();
            let samplers = anim["samplers"].as_array().unwrap();
            let chans: Vec<(usize, &str, Vec<f32>)> = anim["channels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    let s = &samplers[c["sampler"].as_u64().unwrap() as usize];
                    (
                        c["target"]["node"].as_u64().unwrap() as usize,
                        c["target"]["path"].as_str().unwrap(),
                        floats(&j, &bin, s["output"].as_u64().unwrap()),
                    )
                })
                .collect();
            let frames = floats(&j, &bin, samplers[0]["input"].as_u64().unwrap()).len();
            for k in 0..frames {
                let mut t: Vec<Vec3> = nodes.iter().map(|n| v3(n, "translation", 0.0)).collect();
                let mut s: Vec<Vec3> = nodes.iter().map(|n| v3(n, "scale", 1.0)).collect();
                let mut r: Vec<Quat> = nodes
                    .iter()
                    .map(|n| {
                        n["rotation"].as_array().map_or(Quat::IDENTITY, |a| {
                            let f = |i: usize| a[i].as_f64().unwrap() as f32;
                            Quat::from_xyzw(f(0), f(1), f(2), f(3))
                        })
                    })
                    .collect();
                for (node, path, v) in &chans {
                    match *path {
                        "translation" => t[*node] = Vec3::from_slice(&v[3 * k..]),
                        "scale" => s[*node] = Vec3::from_slice(&v[3 * k..]),
                        "rotation" => r[*node] = Quat::from_slice(&v[4 * k..]).normalize(),
                        _ => {}
                    }
                }
                for &toe in &toes {
                    let mut m = Mat4::IDENTITY;
                    let mut at = Some(toe);
                    while let Some(i) = at {
                        m = Mat4::from_scale_rotation_translation(s[i], r[i], t[i]) * m;
                        at = parent[i];
                    }
                    let y = m.w_axis.y;
                    assert!(
                        y > FOOT_FLOOR_M,
                        "{rel} {name} frame {k}: {} at y = {y:.3} m, under the ground",
                        nodes[toe]["name"]
                    );
                }
            }
        }
    }
}
