//! Gate: the first-person arms are aimed at the hold point, and the arm that
//! holds nothing is out of frame.
//!
//! **The arithmetic that IS worth gating about a frame** (`CLAUDE.md`: there
//! is no visual gate and none is to be built; what may be gated is that the
//! numbers fit the volume, in Rust, in the shape of `tests/tree.rs`).
//! `render/viewmodel.rs` places the arms by *derivation* — `VIEWMODEL_ARMS` is
//! whatever puts the hold clip's right hand exactly on `VIEWMODEL_HOLD` — and
//! until now the only thing that checked the derivation survived contact with
//! the scene graph was an `info!` printed once, 45 frames in, in a running
//! client with a GPU. That is evidence a person has to go and read. This file
//! is the same claim as a test, and it is cheap for the reason `rig_asset.rs`
//! is cheap: a GLB's JSON chunk is JSON, a keyframe is four floats, and
//! forward kinematics is a chain of quaternion multiplies.
//!
//! Four things it holds, and each one has already failed or is one edit from
//! failing:
//!
//!   1. **The hold hand lands on the hold point.** Moving `VIEWMODEL_ARMS`,
//!      re-importing the rig, or retargeting the clip all move the hand, and
//!      none of them is a compile error.
//!   2. **The clip is still the two-handed grip the hide exists for.** The
//!      hidden arm is a fix for `Pistol_Idle_Loop` specifically; a swap to a
//!      one-handed clip should delete the hide rather than inherit it, and
//!      this is what says so out loud.
//!   3. **The hidden bone is the off hand's**, resolved against the shipped
//!      file. A typo in `VIEWMODEL_HIDDEN_ARM` costs the whole viewmodel at
//!      runtime — `dress_arms` requires the name — and costs nothing at build.
//!   4. **The hold clip never animates that bone's scale.** The collapse is
//!      written once and never re-applied, which is free and is only correct
//!      while the clip leaves scale alone. `Idle_Loop` does not: it animates
//!      scale on all 24 joints, so this is a real edit away.
//!
//! What it CANNOT check is whether an arm entering frame at that angle reads
//! as an arm. That is `--bin modelview <file> --eye --hide char1_body`, and a
//! person looking at the game.

#![cfg(feature = "render")]

use client::render::viewmodel::{
    VIEWMODEL_ARMS, VIEWMODEL_BOB_X, VIEWMODEL_BOB_Y, VIEWMODEL_HIDDEN_ARM, VIEWMODEL_HOLD,
    VIEWMODEL_SWING_PUSH,
};

/// Assets live beside the crate — `tests/rig_asset.rs`'s hop.
fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

const RIG: &str = "models/stumpy.glb";
/// The hand the item is in. `dress_arms` resolves this same literal.
const HOLD_BONE: &str = "RightHand";
/// The hand on the arm that gets collapsed.
const OFF_BONE: &str = "LeftHand";
/// `render::rig::FOV_DEG`, restated. Vertical, and 16:9 is the frame this is
/// judged in — `capture-native.sh`'s and the shipped window's.
const FOV_DEG: f32 = 75.0;
const ASPECT: f32 = 16.0 / 9.0;
/// `render::rig`'s near plane. A point closer than this is clipped, which is
/// one of the two ways the collapsed arm can be off screen.
const NEAR_M: f32 = 0.1;

/// How many times to sample the loop. The clip is 1.667 s of gentle idle; 24
/// steps is one every 70 ms, finer than the motion.
const STEPS: usize = 24;

type V3 = [f32; 3];
type Q = [f32; 4];

// ── The file ─────────────────────────────────────────────────────────────

struct Glb {
    json: serde_json::Value,
    bin: Vec<u8>,
}

impl Glb {
    fn open(path: &std::path::Path) -> Self {
        let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&raw[0..4], b"glTF", "{}: not a GLB", path.display());
        let len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
        let json = serde_json::from_slice(&raw[20..20 + len])
            .unwrap_or_else(|e| panic!("{}: bad JSON chunk: {e}", path.display()));
        let at = 20 + len.next_multiple_of(4);
        assert!(
            at + 8 <= raw.len() && &raw[at + 4..at + 8] == b"BIN\0",
            "{RIG}: no BIN chunk — this file carries no keyframes to read"
        );
        let n = u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as usize;
        Self {
            json,
            bin: raw[at + 8..at + 8 + n].to_vec(),
        }
    }

    /// One accessor's f32 rows. Refuses anything it cannot decode rather than
    /// guessing — `rig_asset.rs`'s reader and its reason: a silently wrong
    /// decode makes every assertion below pass on numbers it invented.
    fn floats(&self, i: usize) -> Vec<Vec<f32>> {
        let a = &self.json["accessors"][i];
        assert_eq!(
            a["componentType"].as_u64(),
            Some(5126),
            "{RIG}: accessor {i} is not f32"
        );
        let n = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            other => panic!("{RIG}: accessor {i} is {other:?}"),
        };
        let bv = &self.json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let base = bv["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = match bv["byteStride"].as_u64().unwrap_or(0) as usize {
            0 => 4 * n,
            s => s,
        };
        let count = a["count"].as_u64().unwrap() as usize;
        (0..count)
            .map(|k| {
                (0..n)
                    .map(|c| {
                        let o = base + k * stride + 4 * c;
                        f32::from_le_bytes(self.bin[o..o + 4].try_into().unwrap())
                    })
                    .collect()
            })
            .collect()
    }

    fn nodes(&self) -> &Vec<serde_json::Value> {
        self.json["nodes"].as_array().expect("no nodes")
    }

    /// A node index by name, or `None`. The lookup `dress_arms` does at
    /// runtime, done against the file at build time.
    fn node(&self, name: &str) -> Option<usize> {
        self.nodes()
            .iter()
            .position(|n| n["name"].as_str() == Some(name))
    }

    fn clip(&self, name: &str) -> &serde_json::Value {
        self.json["animations"]
            .as_array()
            .expect("no animations")
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("{RIG}: no clip named {name}"))
    }

    fn duration(&self, name: &str) -> f32 {
        let a = self.clip(name);
        a["samplers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let t = self.floats(s["input"].as_u64().unwrap() as usize);
                t.last().map_or(0.0, |v| v[0])
            })
            .fold(0.0, f32::max)
    }

    /// Every `(node, path)` this clip writes.
    fn channels(&self, name: &str) -> Vec<(usize, String)> {
        self.clip(name)["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["target"]["node"].as_u64().unwrap() as usize,
                    c["target"]["path"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// The clip's local TRS overrides at time `t`, per node.
    ///
    /// Linear between the bracketing keys, and **slerp for rotations**: a
    /// componentwise lerp of two quaternions is not a rotation, and the error
    /// is largest exactly where the pose is moving fastest.
    fn pose(&self, name: &str, t: f32) -> std::collections::HashMap<usize, Trs> {
        let a = self.clip(name);
        let samplers = a["samplers"].as_array().unwrap();
        let mut out: std::collections::HashMap<usize, Trs> = Default::default();
        for c in a["channels"].as_array().unwrap() {
            let s = &samplers[c["sampler"].as_u64().unwrap() as usize];
            let node = c["target"]["node"].as_u64().unwrap() as usize;
            let path = c["target"]["path"].as_str().unwrap();
            let times = self.floats(s["input"].as_u64().unwrap() as usize);
            let vals = self.floats(s["output"].as_u64().unwrap() as usize);
            let tt = t.clamp(times[0][0], times[times.len() - 1][0]);
            let mut i = 0;
            while i + 1 < times.len() && times[i + 1][0] < tt {
                i += 1;
            }
            let j = (i + 1).min(times.len() - 1);
            let span = times[j][0] - times[i][0];
            let u = if span <= 0.0 {
                0.0
            } else {
                (tt - times[i][0]) / span
            };
            let e = out.entry(node).or_default();
            match path {
                "rotation" => {
                    let (a, b) = (quat(&vals[i]), quat(&vals[j]));
                    e.rotation = Some(slerp(a, b, u));
                }
                "translation" => e.translation = Some(lerp3(&vals[i], &vals[j], u)),
                "scale" => e.scale = Some(lerp3(&vals[i], &vals[j], u)),
                _ => {}
            }
        }
        out
    }

    /// World-space translation of every node, with `clip` posing it.
    ///
    /// Depth-first from the scene roots, composing parent · local. The scene
    /// root's own `scale 0.01` is in here and belongs in here: joint
    /// translations on this rig are in centimetres (`rig_asset.rs`
    /// `root_transform` says why at length), so the chain only measures metres
    /// once that scale has been applied to it.
    fn skeleton(&self, clip: &str, t: f32) -> Vec<V3> {
        let over = self.pose(clip, t);
        let nodes = self.nodes();
        let mut out = vec![[0.0; 3]; nodes.len()];
        let mut parent = vec![usize::MAX; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            for c in n["children"].as_array().into_iter().flatten() {
                parent[c.as_u64().unwrap() as usize] = i;
            }
        }
        let roots: Vec<usize> = (0..nodes.len())
            .filter(|i| parent[*i] == usize::MAX)
            .collect();
        let mut stack: Vec<(usize, V3, Q, V3)> = roots
            .iter()
            .map(|&r| (r, [0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]))
            .collect();
        // Bounded: a node is pushed once, by its one parent.
        while let Some((i, pt, pr, ps)) = stack.pop() {
            let n = &nodes[i];
            assert!(
                n["matrix"].is_null(),
                "{RIG}: node {i} carries a matrix, not TRS — this reader cannot decode it"
            );
            let o = over.get(&i);
            let tr = o
                .and_then(|o| o.translation)
                .unwrap_or_else(|| vec3(&n["translation"], [0.0; 3]));
            let ro = o
                .and_then(|o| o.rotation)
                .unwrap_or_else(|| vec4(&n["rotation"], [0.0, 0.0, 0.0, 1.0]));
            let sc = o
                .and_then(|o| o.scale)
                .unwrap_or_else(|| vec3(&n["scale"], [1.0; 3]));
            let scaled = [tr[0] * ps[0], tr[1] * ps[1], tr[2] * ps[2]];
            let rotated = rotate(pr, scaled);
            let wt = [pt[0] + rotated[0], pt[1] + rotated[1], pt[2] + rotated[2]];
            let wr = qmul(pr, ro);
            let ws = [ps[0] * sc[0], ps[1] * sc[1], ps[2] * sc[2]];
            out[i] = wt;
            for c in n["children"].as_array().into_iter().flatten() {
                stack.push((c.as_u64().unwrap() as usize, wt, wr, ws));
            }
        }
        out
    }

    /// Whether `ancestor` is on `node`'s parent chain (or is it).
    fn descends_from(&self, node: usize, ancestor: usize) -> bool {
        let nodes = self.nodes();
        let mut parent = vec![usize::MAX; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            for c in n["children"].as_array().into_iter().flatten() {
                parent[c.as_u64().unwrap() as usize] = i;
            }
        }
        let mut at = node;
        let mut hops = 0;
        while at != usize::MAX {
            if at == ancestor {
                return true;
            }
            at = parent[at];
            hops += 1;
            assert!(hops <= nodes.len(), "{RIG}: the node graph has a cycle");
        }
        false
    }
}

#[derive(Default, Clone, Copy)]
struct Trs {
    translation: Option<V3>,
    rotation: Option<Q>,
    scale: Option<V3>,
}

// ── The arithmetic, written out so a reader can check it ─────────────────

fn vec3(v: &serde_json::Value, dflt: V3) -> V3 {
    v.as_array().map_or(dflt, |a| {
        [
            a[0].as_f64().unwrap() as f32,
            a[1].as_f64().unwrap() as f32,
            a[2].as_f64().unwrap() as f32,
        ]
    })
}

fn vec4(v: &serde_json::Value, dflt: Q) -> Q {
    v.as_array().map_or(dflt, |a| {
        [
            a[0].as_f64().unwrap() as f32,
            a[1].as_f64().unwrap() as f32,
            a[2].as_f64().unwrap() as f32,
            a[3].as_f64().unwrap() as f32,
        ]
    })
}

fn quat(v: &[f32]) -> Q {
    [v[0], v[1], v[2], v[3]]
}

fn lerp3(a: &[f32], b: &[f32], u: f32) -> V3 {
    [
        a[0] + (b[0] - a[0]) * u,
        a[1] + (b[1] - a[1]) * u,
        a[2] + (b[2] - a[2]) * u,
    ]
}

fn slerp(a: Q, b: Q, u: f32) -> Q {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let mut b = b;
    if d < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        d = -d;
    }
    let out = if d > 0.9995 {
        [
            a[0] + (b[0] - a[0]) * u,
            a[1] + (b[1] - a[1]) * u,
            a[2] + (b[2] - a[2]) * u,
            a[3] + (b[3] - a[3]) * u,
        ]
    } else {
        let th = d.clamp(-1.0, 1.0).acos();
        let s = th.sin();
        let (wa, wb) = (((1.0 - u) * th).sin() / s, (u * th).sin() / s);
        [
            a[0] * wa + b[0] * wb,
            a[1] * wa + b[1] * wb,
            a[2] * wa + b[2] * wb,
            a[3] * wa + b[3] * wb,
        ]
    };
    let n = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2] + out[3] * out[3]).sqrt();
    [out[0] / n, out[1] / n, out[2] / n, out[3] / n]
}

fn qmul(a: Q, b: Q) -> Q {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

/// Rotate a point by an `(x, y, z, w)` quaternion — `rig_asset.rs`'s, restated
/// so this file is arithmetic a reader can check without a second tab.
fn rotate(q: Q, v: V3) -> V3 {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let t = [
        2.0 * (y * v[2] - z * v[1]),
        2.0 * (z * v[0] - x * v[2]),
        2.0 * (x * v[1] - y * v[0]),
    ];
    [
        v[0] + w * t[0] + y * t[2] - z * t[1],
        v[1] + w * t[1] + z * t[0] - x * t[2],
        v[2] + w * t[2] + x * t[1] - y * t[0],
    ]
}

/// A rig-space point in VIEW space, which is what `spawn_arms` builds: the
/// arms hang off the camera at `VIEWMODEL_ARMS`, yawed 180° so a rig that
/// faces +Z faces the way a camera looking down −Z does. A yaw of π negates x
/// and z and leaves y, so the whole transform is three signs and an add.
fn view(p: V3) -> V3 {
    [
        VIEWMODEL_ARMS.x - p[0],
        VIEWMODEL_ARMS.y + p[1],
        VIEWMODEL_ARMS.z - p[2],
    ]
}

/// Normalised device coordinates, or `None` for a point at or behind the near
/// plane — which is off screen for the purposes below, not an error.
fn ndc(p: V3) -> Option<(f32, f32)> {
    if p[2] > -NEAR_M {
        return None;
    }
    let z = -p[2];
    let tan_v = (FOV_DEG.to_radians() / 2.0).tan();
    Some((p[0] / (tan_v * ASPECT * z), p[1] / (tan_v * z)))
}

fn on_screen(p: V3) -> bool {
    ndc(p).is_some_and(|(x, y)| x.abs() <= 1.0 && y.abs() <= 1.0)
}

fn dist(a: V3, b: V3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Every time the loop is sampled at.
fn steps(glb: &Glb, clip: &str) -> Vec<f32> {
    let d = glb.duration(clip);
    (0..=STEPS).map(|i| d * i as f32 / STEPS as f32).collect()
}

// ── The gates ────────────────────────────────────────────────────────────

#[test]
fn the_arms_offset_lands_the_hold_hand_on_the_hold_point() {
    // The claim `VIEWMODEL_ARMS`'s doc comment makes, and the one the client
    // prints once at boot: the offset is not dialled in, it is whatever puts
    // the hold clip's right hand on `VIEWMODEL_HOLD`. If that stops being
    // true, the item stops being in the hand — and everything else about the
    // viewmodel still works, so nothing else notices.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let bone = glb
        .node(HOLD_BONE)
        .unwrap_or_else(|| panic!("{RIG}: no {HOLD_BONE} bone"));
    let target = [VIEWMODEL_HOLD.x, VIEWMODEL_HOLD.y, VIEWMODEL_HOLD.z];
    let mut worst = 0.0f32;
    for t in steps(&glb, clip) {
        let at = view(glb.skeleton(clip, t)[bone]);
        worst = worst.max(dist(at, target));
    }
    // 10 mm. The running client measured 1 mm at rest; the loop's own breathing
    // is most of the rest, so this is a bound on the DERIVATION rather than on
    // the animation, and it fails on a typo without failing on a re-retarget
    // that keeps the pose.
    assert!(
        worst < 0.010,
        "{HOLD_BONE} strays {:.1} mm from VIEWMODEL_HOLD over {clip} — \
         VIEWMODEL_ARMS is derived from this clip and one of them has moved",
        worst * 1000.0
    );
}

#[test]
fn the_hold_clip_is_the_two_handed_grip_the_hidden_arm_exists_for() {
    // Not a property anybody wants — a statement of the defect the hide is a
    // fix for, so that swapping to a one-handed clip is a RED gate telling you
    // to delete the hide rather than a silent inheritance of it.
    //
    // Two facts, and the second is the one the operator actually saw: the
    // hands are close enough to tangle, and the idle one is NEARER the eye, so
    // it draws in front of the item rather than behind it.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let (hold, off) = (glb.node(HOLD_BONE).unwrap(), glb.node(OFF_BONE).unwrap());
    let (mut closest, mut nearest_gap) = (f32::MAX, f32::MAX);
    for t in steps(&glb, clip) {
        let s = glb.skeleton(clip, t);
        let (h, o) = (view(s[hold]), view(s[off]));
        closest = closest.min(dist(h, o));
        // View space looks down −z, so a LARGER z is nearer the eye.
        nearest_gap = nearest_gap.min(o[2] - h[2]);
    }
    assert!(
        closest < 0.10,
        "{clip}'s hands are {:.0} mm apart — that is a one-handed pose, so \
         VIEWMODEL_HIDDEN_ARM is deleting an arm for a reason that no longer \
         holds. Delete the hide (and re-derive VIEWMODEL_ARMS if the hold hand \
         changed) rather than widening this bound",
        closest * 1000.0
    );
    assert!(
        nearest_gap > 0.0,
        "{OFF_BONE} is no longer in front of {HOLD_BONE} ({:.0} mm) — the hide \
         was for an open palm drawn over the held item",
        nearest_gap * 1000.0
    );
}

#[test]
fn the_hidden_arm_is_the_one_holding_nothing() {
    // `dress_arms` resolves VIEWMODEL_HIDDEN_ARM by name and REQUIRES it: a
    // name the file does not carry leaves the whole viewmodel undressed — a
    // body wrapped around the camera and no arms — with every other gate
    // green. And collapsing a joint takes its whole child chain, so naming the
    // wrong shoulder deletes the hand the item is in.
    let glb = Glb::open(&asset_path(RIG));
    let hidden = glb.node(VIEWMODEL_HIDDEN_ARM).unwrap_or_else(|| {
        panic!("{RIG}: no bone named {VIEWMODEL_HIDDEN_ARM} — dress_arms would never dress")
    });
    let (hold, off) = (glb.node(HOLD_BONE).unwrap(), glb.node(OFF_BONE).unwrap());
    assert!(
        glb.descends_from(off, hidden),
        "{VIEWMODEL_HIDDEN_ARM} is not above {OFF_BONE} — collapsing it would \
         leave the idle hand on screen"
    );
    assert!(
        !glb.descends_from(hold, hidden),
        "{VIEWMODEL_HIDDEN_ARM} is above {HOLD_BONE} — collapsing it takes the \
         hand the item is in with it"
    );
}

#[test]
fn the_hidden_arm_collapses_to_a_point_off_screen() {
    // A collapsed joint is not gone, it is a heap of zero-area triangles at
    // that joint's own origin. Off screen it costs nothing and shows nothing;
    // ON screen it is a speck of skin in the middle of the frame, which is a
    // worse defect than the one being fixed.
    //
    // Checked over the loop AND over the motion envelope `animate` writes onto
    // the arms — the bob's two amplitudes and the swing's push — because those
    // move the collapse point too, and the swing's is 13 cm, which is not
    // small next to how far outside the frame this sits.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let bone = glb.node(VIEWMODEL_HIDDEN_ARM).unwrap();
    let envelope = [
        [0.0, 0.0, 0.0],
        [VIEWMODEL_BOB_X, 0.0, 0.0],
        [-VIEWMODEL_BOB_X, -VIEWMODEL_BOB_Y, 0.0],
        // Mid-swing: `animate`'s `arc` peaks at 1 and writes both terms.
        [0.0, 0.05, VIEWMODEL_SWING_PUSH],
        [
            VIEWMODEL_BOB_X,
            0.05 - VIEWMODEL_BOB_Y,
            VIEWMODEL_SWING_PUSH,
        ],
    ];
    let mut worst = f32::MAX;
    for t in steps(&glb, clip) {
        let p = view(glb.skeleton(clip, t)[bone]);
        for e in envelope {
            let q = [p[0] + e[0], p[1] + e[1], p[2] + e[2]];
            assert!(
                !on_screen(q),
                "{VIEWMODEL_HIDDEN_ARM} collapses to {q:?} at t={t:.2} under \
                 offset {e:?}, which is inside the frame"
            );
            // How much margin, in ndc — reported so a shrinking one is visible
            // in the failure rather than only in a future red run.
            if let Some((x, y)) = ndc(q) {
                worst = worst.min(x.abs().max(y.abs()));
            }
        }
    }
    assert!(
        worst > 1.2,
        "the collapse point is only {worst:.2} of the way past the frame edge \
         — that is inside a bob of visible"
    );
}

#[test]
fn the_hold_clip_never_animates_the_hidden_bones_scale() {
    // The collapse is written ONCE, by `dress_arms`, and never re-applied.
    // That is free and it is correct only while the clip playing on these arms
    // leaves scale alone. `Pistol_Idle_Loop` writes rotation on 22 joints and
    // translation on the hips; `Idle_Loop`, three names away in the same file,
    // writes translation, rotation AND scale on all 24. So a clip swap here
    // pops the arm back with no compile error, no panic and no log line — the
    // exact class `CLAUDE.md`'s trap list is made of.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let hidden = glb.node(VIEWMODEL_HIDDEN_ARM).unwrap();
    let scaled: Vec<usize> = glb
        .channels(clip)
        .into_iter()
        .filter(|(_, path)| path == "scale")
        .map(|(n, _)| n)
        .filter(|&n| glb.descends_from(n, hidden))
        .collect();
    assert!(
        scaled.is_empty(),
        "{clip} animates scale on {scaled:?}, under {VIEWMODEL_HIDDEN_ARM} — \
         the one-shot collapse in dress_arms would be overwritten on the next \
         frame. Re-apply it after AnimationSystems (anim::head_look's shape) \
         or pick a clip that leaves scale alone"
    );
}
