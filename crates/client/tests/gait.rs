//! Gate: how Triple T's legs move, measured on the posed skeleton with no
//! GPU. After Kong Royale's `tools/actors/anim-test.mts` (MIT,
//! github.com/MaximilianHansen/kongroyale; ideas only) and oneshot's
//! `tests/anim.rs`, the same measurements on `stumpy.glb`:
//!
//! 1. **Strides.** Each loop's `Clip::stride_mps` is what its planted toes do,
//!    measured here every run rather than once by hand.
//! 2. **Feet.** A body travelling at a steady speed plays the loop and rate
//!    `anim::BodyAnim` picks, its legs turned as the gait turns them. A foot is
//!    down where its toes are within [`PLANT_M`] of their lowest; its speed
//!    over the ground there, over the body's, is the slide.
//! 3. **Slopes.** `render::ik` puts each foot on a slope and the hips down for
//!    the lower.
//!
//! `-- --nocapture` prints the tables to tune by.

#![cfg(feature = "render")]

use std::collections::HashMap;

use bevy::prelude::*;
use client::render::anim::{BodyAnim, Clip, ANIM_RATE_MAX, ANIM_RATE_MIN};
use client::render::ik::{foot_rise, Legs};
use sim_core::movement::{CROUCH_SPEED, SPRINT_SPEED, WADE_SPEED_MULT, WALK_SPEED};

/// A foot is down where its toes are this close to their lowest in the
/// loop, metres. The toes, not the ankle: Triple T's heel lifts and the foot
/// rolls over its toes through the stance, so the ankle never stands still.
const PLANT_M: f32 = 0.03;
const TOES: [&str; 2] = ["LeftToeBase", "RightToeBase"];
/// How far a loop's `Clip::stride_mps` may be off the speed its own planted
/// toes move at, as a share of it.
const STRIDE_OFF_MAX: f32 = 0.05;
/// The most a planted foot may slide, as a share of the body's speed.
const SLIDE_MAX: f32 = 0.10;
/// Samples per loop.
const STEPS: usize = 240;

// ---------------------------------------------------------------- the file

struct Track {
    node: usize,
    path: u8, // b't', b'r', b's'
    step: bool,
    times: Vec<f32>,
    values: Vec<f32>,
}

struct Glb {
    names: Vec<String>,
    parent: Vec<Option<usize>>,
    rest: Vec<Transform>,
    clips: HashMap<String, Vec<Track>>,
}

impl Glb {
    fn load() -> Glb {
        let raw = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/models/stumpy.glb"
        ))
        .expect("assets/models/stumpy.glb");
        let u32_at = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap()) as usize;
        let json_len = u32_at(12);
        let json: serde_json::Value = serde_json::from_slice(&raw[20..20 + json_len]).unwrap();
        let bin = &raw[20 + json_len + 8..];
        let floats = |acc: usize| -> Vec<f32> {
            let a = &json["accessors"][acc];
            assert_eq!(a["componentType"], 5126, "float accessors only");
            let n = a["count"].as_u64().unwrap() as usize
                * match a["type"].as_str().unwrap() {
                    "SCALAR" => 1,
                    "VEC3" => 3,
                    "VEC4" => 4,
                    t => panic!("{t}"),
                };
            let v = &json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
            assert!(v.get("byteStride").is_none(), "packed views only");
            let o = v["byteOffset"].as_u64().unwrap_or(0) as usize
                + a["byteOffset"].as_u64().unwrap_or(0) as usize;
            (0..n)
                .map(|i| f32::from_le_bytes(bin[o + 4 * i..o + 4 * i + 4].try_into().unwrap()))
                .collect()
        };
        let nodes = json["nodes"].as_array().unwrap();
        let mut parent = vec![None; nodes.len()];
        let mut names = Vec::new();
        let mut rest = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            for c in n["children"].as_array().into_iter().flatten() {
                parent[c.as_u64().unwrap() as usize] = Some(i);
            }
            names.push(n["name"].as_str().unwrap_or("").to_string());
            let f = |k: &str, d: &[f32]| -> Vec<f32> {
                n[k].as_array()
                    .map(|a| a.iter().map(|x| x.as_f64().unwrap() as f32).collect())
                    .unwrap_or_else(|| d.to_vec())
            };
            let (t, r, s) = (
                f("translation", &[0.0; 3]),
                f("rotation", &[0.0, 0.0, 0.0, 1.0]),
                f("scale", &[1.0; 3]),
            );
            rest.push(Transform {
                translation: Vec3::from_slice(&t),
                rotation: Quat::from_slice(&r),
                scale: Vec3::from_slice(&s),
            });
        }
        let mut clips = HashMap::new();
        for a in json["animations"].as_array().unwrap() {
            let samplers = a["samplers"].as_array().unwrap();
            let mut tracks = Vec::new();
            for ch in a["channels"].as_array().unwrap() {
                let s = &samplers[ch["sampler"].as_u64().unwrap() as usize];
                let interp = s["interpolation"].as_str().unwrap_or("LINEAR");
                assert_ne!(interp, "CUBICSPLINE");
                tracks.push(Track {
                    node: ch["target"]["node"].as_u64().unwrap() as usize,
                    path: match ch["target"]["path"].as_str().unwrap() {
                        "translation" => b't',
                        "rotation" => b'r',
                        "scale" => b's',
                        p => panic!("{p}"),
                    },
                    step: interp == "STEP",
                    times: floats(s["input"].as_u64().unwrap() as usize),
                    values: floats(s["output"].as_u64().unwrap() as usize),
                });
            }
            clips.insert(a["name"].as_str().unwrap().to_string(), tracks);
        }
        Glb {
            names,
            parent,
            rest,
            clips,
        }
    }

    fn node(&self, name: &str) -> usize {
        self.names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("no {name}"))
    }

    fn duration(&self, clip: &str) -> f32 {
        self.clips[clip]
            .iter()
            .filter_map(|t| t.times.last().copied())
            .fold(0.0, f32::max)
    }

    /// Every node's own transform at `t` seconds into `clip`.
    fn pose(&self, clip: &str, t: f32) -> Vec<Transform> {
        let mut out = self.rest.clone();
        for tr in &self.clips[clip] {
            let k = tr
                .times
                .partition_point(|&x| x <= t)
                .clamp(1, tr.times.len())
                - 1;
            let n = if tr.path == b'r' { 4 } else { 3 };
            let at = |i: usize| &tr.values[i * n..i * n + n];
            let (a, b, u) = if k + 1 < tr.times.len() && !tr.step {
                let (t0, t1) = (tr.times[k], tr.times[k + 1]);
                (at(k), at(k + 1), ((t - t0) / (t1 - t0)).clamp(0.0, 1.0))
            } else {
                (at(k), at(k), 0.0)
            };
            let o = &mut out[tr.node];
            match tr.path {
                b'r' => {
                    o.rotation = Quat::from_slice(a)
                        .slerp(Quat::from_slice(b), u)
                        .normalize()
                }
                b't' => o.translation = Vec3::from_slice(a).lerp(Vec3::from_slice(b), u),
                _ => o.scale = Vec3::from_slice(a).lerp(Vec3::from_slice(b), u),
            }
        }
        out
    }

    /// `node`'s frame in the scene's, under `locals`.
    fn frame(&self, locals: &[Transform], node: usize) -> Transform {
        let mut out = locals[node];
        let mut at = node;
        while let Some(p) = self.parent[at] {
            out = locals[p].mul_transform(out);
            at = p;
        }
        out
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
}

// ---------------------------------------------------------------- 1. strides

/// How fast a loop's planted toes go back under the body, m/s of the clip's
/// own time: the ground speed it was made for (both feet's together).
fn stride(glb: &Glb, clip: Clip) -> f32 {
    let d = glb.duration(clip.name());
    let h = d / STEPS as f32;
    let toes = TOES.map(|t| glb.node(t));
    let s: Vec<[Vec3; 2]> = (0..STEPS)
        .map(|i| {
            let p = glb.pose(clip.name(), i as f32 * h);
            toes.map(|t| glb.frame(&p, t).translation)
        })
        .collect();
    let mut v = Vec::new();
    for f in 0..2 {
        let low = s.iter().map(|x| x[f].y).fold(f32::INFINITY, f32::min);
        for i in 0..STEPS {
            if s[i][f].y <= low + PLANT_M {
                let d = (s[(i + 1) % STEPS][f] - s[(i + STEPS - 1) % STEPS][f]) / (2.0 * h);
                v.push(Vec2::new(d.x, d.z).length());
            }
        }
    }
    median(v)
}

#[test]
fn every_stride_is_what_its_clip_does() {
    let glb = Glb::load();
    let mut off = Vec::new();
    for c in Clip::ALL {
        let Some(want) = c.stride_mps() else { continue };
        let got = stride(&glb, c);
        println!(
            "{:<11} measured {got:.3} m/s, stride_mps {want:.3}",
            format!("{c:?}")
        );
        if (want - got).abs() > STRIDE_OFF_MAX * got {
            off.push(format!("{c:?}: {want} vs {got:.3}"));
        }
    }
    assert!(off.is_empty(), "stride_mps is off its clip: {off:?}");
}

// ---------------------------------------------------------------- 2. feet

/// A body that has travelled at `speed` on `bearing` (radians off its facing,
/// the sim's yaw sense) long enough to settle, its gait read back.
fn settled(speed: f32, bearing: f32, crouched: bool) -> BodyAnim {
    let mut a = BodyAnim {
        crouched,
        ..default()
    };
    let step = Vec3::new(bearing.sin(), 0.0, bearing.cos()) * speed / 60.0;
    let mut pos = Vec3::ZERO;
    for _ in 0..600 {
        pos += step;
        a.observe(pos, 1.0 / 60.0, false, false, false);
    }
    a
}

struct Slide {
    clip: Clip,
    rate: f32,
    slide: f32,
}

fn slide(glb: &Glb, speed: f32, bearing: f32, crouched: bool) -> Slide {
    let a = settled(speed, bearing, crouched);
    let clip = a.clip.unwrap();
    let rate = a.rate();
    let d = glb.duration(clip.name());
    let toes = TOES.map(|t| glb.node(t));
    let h = d / STEPS as f32;
    let s: Vec<[Vec3; 2]> = (0..STEPS)
        .map(|i| {
            let p = glb.pose(clip.name(), i as f32 * h);
            toes.map(|t| glb.frame(&p, t).translation)
        })
        .collect();
    // The travel in the legs' frame: the clips face +Z, and what the twist
    // can't take up of the bearing is left over as a sideways error.
    let want = if a.backward {
        bearing - core::f32::consts::PI
    } else {
        bearing
    };
    let err = want - a.hip_yaw;
    let sign = if a.backward { -1.0 } else { 1.0 };
    let body = Vec3::new(err.sin(), 0.0, err.cos()) * speed * sign;
    let mut slid = Vec::new();
    for f in 0..2 {
        let low = s.iter().map(|x| x[f].y).fold(f32::INFINITY, f32::min);
        for i in 0..STEPS {
            if s[i][f].y > low + PLANT_M {
                continue;
            }
            let v = (s[(i + 1) % STEPS][f] - s[(i + STEPS - 1) % STEPS][f]) / (2.0 * h);
            slid.push((body + Vec3::new(v.x, 0.0, v.z) * rate).length() / speed);
        }
    }
    Slide {
        clip,
        rate,
        slide: median(slid),
    }
}

/// Every gait at the sim's own speeds that its loop's rate band covers,
/// forward, backwards and on a diagonal: the planted feet hold.
#[test]
fn planted_feet_hold_at_every_gait() {
    let glb = Glb::load();
    let wade = WALK_SPEED * WADE_SPEED_MULT;
    let cases: &[(&str, f32, f32)] = &[
        ("wade", wade, 0.0),
        ("walk", WALK_SPEED, 0.0),
        ("walk-diag", WALK_SPEED, 0.785),
        ("sprint", SPRINT_SPEED, 0.0),
        ("sprint-diag", SPRINT_SPEED, -0.785),
        ("backpedal", WALK_SPEED, core::f32::consts::PI),
        ("backpedal-wade", wade, core::f32::consts::PI),
    ];
    let mut failed = Vec::new();
    println!(
        "{:<15} {:>6} {:<11} {:>6} {:>7}",
        "case", "m/s", "clip", "rate", "slide"
    );
    for &(name, speed, bearing) in cases {
        let s = slide(&glb, speed, bearing, false);
        let in_band = s.rate.abs() > ANIM_RATE_MIN + 1e-3 && s.rate.abs() < ANIM_RATE_MAX - 1e-3;
        println!(
            "{name:<15} {speed:>6.2} {:<11} {:>6.2} {:>6.1}%",
            format!("{:?}", s.clip),
            s.rate,
            s.slide * 100.0
        );
        if in_band && s.slide > SLIDE_MAX {
            failed.push(format!("{name}: {:.1}%", s.slide * 100.0));
        }
    }
    assert!(
        failed.is_empty(),
        "planted feet slide past {}%: {failed:?}",
        SLIDE_MAX * 100.0
    );
}

/// What the twist and the rate band leave sliding, printed and never gated:
/// a side-on strafe past the legs' turn; and the crouch at the sim's speed,
/// past the crouch loop's top rate, whose two feet also go back at different
/// speeds (the clip wants fixing, not the rate). The numbers to beat.
#[test]
fn what_still_slides() {
    let glb = Glb::load();
    for (name, speed, bearing, crouched) in [
        ("strafe", WALK_SPEED, core::f32::consts::FRAC_PI_2, false),
        (
            "strafe-sprint",
            SPRINT_SPEED,
            core::f32::consts::FRAC_PI_2,
            false,
        ),
        ("crouch", CROUCH_SPEED, 0.0, true),
    ] {
        let s = slide(&glb, speed, bearing, crouched);
        println!(
            "{name:<14} {speed:>5.2} m/s {:?} at {:.2}: {:.1}% slide",
            s.clip,
            s.rate,
            s.slide * 100.0
        );
        assert!(s.slide.is_finite());
    }
}

// ---------------------------------------------------------------- 3. slopes

/// Standing across a 20° slope, each foot meets the ground under it and the
/// hips come down for the lower one.
#[test]
fn feet_meet_a_slope() {
    let glb = Glb::load();
    let hips = glb.node("Hips");
    let bones = |side: &str| {
        [
            format!("{side}UpLeg"),
            format!("{side}Leg"),
            format!("{side}Foot"),
        ]
    };
    let slope = 20f32.to_radians().tan();
    for i in 0..8 {
        let p = glb.pose(
            Clip::Idle.name(),
            glb.duration(Clip::Idle.name()) * i as f32 / 8.0,
        );
        let leg = |side: &str| bones(side).map(|b| p[glb.node(&b)]);
        let legs = Legs {
            hips_parent: glb.frame(&p, glb.parent[hips].unwrap()),
            hips: p[hips],
            left: leg("Left"),
            right: leg("Right"),
        };
        let a = legs.ankles();
        let rises = a.map(|v| foot_rise(v.x * slope, 0.0));
        let planted = legs.plant(rises, 1.0);
        let mut k = legs;
        k.hips.translation = planted.hips;
        for (b, q) in [(&mut k.left, planted.left), (&mut k.right, planted.right)] {
            let q = q.expect("a standing leg is bent");
            for (t, q) in b.iter_mut().zip(q) {
                t.rotation = q;
            }
        }
        let ka = k.ankles();
        for f in 0..2 {
            let want = a[f] + Vec3::Y * rises[f];
            assert!(
                (ka[f] - want).length() < 0.005,
                "foot {f}: {} vs {want}",
                ka[f]
            );
        }
    }
}
