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

use client::render::bodies::RETIRED_BODY_PALM;
use client::render::viewmodel::{
    bump, rig_transform, swing_pose, VIEWMODEL_ARMS, VIEWMODEL_BOB_X, VIEWMODEL_BOB_Y,
    VIEWMODEL_GRIP_M, VIEWMODEL_GRIP_Q, VIEWMODEL_GRIP_SCALE, VIEWMODEL_HIDDEN_ARM,
    VIEWMODEL_HIDDEN_BEHIND_M, VIEWMODEL_HIDDEN_OFFSET, VIEWMODEL_HOLD, VIEWMODEL_SWING_ATTACK,
    VIEWMODEL_SWING_WINDUP, VIEWMODEL_TILT,
};

/// Assets live beside the crate — `tests/rig_asset.rs`'s hop.
fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

const RIG: &str = "models/stumpy.glb";
/// The hand the item is in — `anim::HAND_BONE`, which `dress_arms` and
/// `bodies::bind_hands` both resolve. Read from the crate rather than retyped,
/// because a gate holding its own copy of a name is checking itself.
const HOLD_BONE: &str = client::render::anim::HAND_BONE;
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

    /// Every node's world TRS, with `clip` posing it — [`Glb::skeleton`]'s
    /// arithmetic, keeping the rotation and the scale it throws away.
    ///
    /// Its own function rather than a widening of `skeleton`, so the four
    /// gates that only ever wanted a point keep reading as arithmetic about
    /// points; this one exists because a GRIP is a frame, not a place.
    fn skeleton_trs(&self, clip: &str, t: f32) -> Vec<(V3, Q, V3)> {
        let over = self.pose(clip, t);
        let nodes = self.nodes();
        let mut out = vec![([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]); nodes.len()];
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
        while let Some((i, pt, pr, ps)) = stack.pop() {
            let n = &nodes[i];
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
            out[i] = (wt, wr, ws);
            for c in n["children"].as_array().into_iter().flatten() {
                stack.push((c.as_u64().unwrap() as usize, wt, wr, ws));
            }
        }
        out
    }

    /// The node's parent, or `None` for a scene root.
    fn parent_of(&self, node: usize) -> Option<usize> {
        self.nodes().iter().position(|n| {
            n["children"]
                .as_array()
                .is_some_and(|c| c.iter().any(|c| c.as_u64() == Some(node as u64)))
        })
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

/// Where the collapsed off-arm's origin actually ends up, in view space, with
/// `dress_arms`'s write applied — the joint moved to `VIEWMODEL_HIDDEN_OFFSET`
/// in its parent's frame.
///
/// Rebuilt from the parent's own world frame rather than by re-running the
/// skeleton with a patched node, because that is the composition Bevy does:
/// `world = parent_t + parent_R · (parent_S · local_t)`.
fn collapsed_off_arm(glb: &Glb, clip: &str, t: f32) -> V3 {
    let bone = glb.node(VIEWMODEL_HIDDEN_ARM).unwrap();
    let parent = glb.parent_of(bone).expect("the off arm has a parent");
    let (pt, pr, ps) = glb.skeleton_trs(clip, t)[parent];
    let (pt, pr) = (view(pt), view_rot(pr));
    let l = [
        VIEWMODEL_HIDDEN_OFFSET.x * ps[0],
        VIEWMODEL_HIDDEN_OFFSET.y * ps[1],
        VIEWMODEL_HIDDEN_OFFSET.z * ps[2],
    ];
    let r = rotate(pr, l);
    [pt[0] + r[0], pt[1] + r[1], pt[2] + r[2]]
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
    // Checked over the loop AND over the bob envelope `animate` writes onto
    // the rig, because that moves the collapse point too.
    //
    // **Scaling the joint was not enough and this gate is how that was
    // found.** Its own origin sits 0.217 m from the lens, where the frame is
    // 33 cm tall — so it cleared the bottom edge by a margin no viewmodel
    // motion could respect, and the swing walked straight through it (ndc y
    // −0.97, a speck of skin in shot). `dress_arms` moves the joint as well
    // as collapsing it, and what this now asserts is the property that buys:
    // the point is BEHIND the camera, where no rotation of the rig can bring
    // it back. `ndc` answers `None` there, which `on_screen` already reads as
    // off screen — so the assertion below did not have to change, only what
    // it is pointed at.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let envelope = [
        [0.0, 0.0, 0.0],
        [VIEWMODEL_BOB_X, 0.0, 0.0],
        [-VIEWMODEL_BOB_X, -VIEWMODEL_BOB_Y, 0.0],
        [VIEWMODEL_BOB_X, VIEWMODEL_BOB_Y, 0.0],
    ];
    let mut nearest = f32::MAX;
    for t in steps(&glb, clip) {
        let p = collapsed_off_arm(&glb, clip, t);
        for e in envelope {
            let q = [p[0] + e[0], p[1] + e[1], p[2] + e[2]];
            assert!(
                !on_screen(q),
                "{VIEWMODEL_HIDDEN_ARM} collapses to {q:?} at t={t:.2} under \
                 offset {e:?}, which is inside the frame"
            );
            assert!(
                q[2] > 0.0,
                "{VIEWMODEL_HIDDEN_ARM} collapses to {q:?} at t={t:.2}, which \
                 is IN FRONT of the camera — the whole point of the offset is \
                 that it is behind, where no swing can rotate it back"
            );
            nearest = nearest.min(q[2]);
        }
    }
    // **And the offset is load-bearing rather than decoration**, stated as its
    // own assertion: the joint's UNMOVED origin is inside the frame under the
    // same envelope. Without this the gate above passes just as well on a
    // build that never writes the offset at all — which is a gate checking its
    // own copy of the fix (`CLAUDE.md`'s naive-rebuild trap, one tier up).
    let bone = glb.node(VIEWMODEL_HIDDEN_ARM).unwrap();
    let unmoved = view(glb.skeleton(clip, 0.0)[bone]);
    let (rot, off) = swing_pose(0.10);
    let at = rig_transform(rot, off).transform_point(bevy::math::Vec3::from_array(unmoved));
    assert!(
        on_screen([at.x, at.y, at.z]),
        "the un-offset collapse point stays out of frame through the swing on \
         its own, so VIEWMODEL_HIDDEN_OFFSET is checking nothing — either the \
         rig or the arc has changed under this gate"
    );

    // The distance the constant claims, held to a centimetre. A re-import or a
    // retarget that turns the parent's frame moves this and nothing else
    // notices.
    assert!(
        (nearest - VIEWMODEL_HIDDEN_BEHIND_M).abs() < 0.05,
        "the collapse point parks {nearest:.3} m behind the eye where \
         VIEWMODEL_HIDDEN_BEHIND_M says {VIEWMODEL_HIDDEN_BEHIND_M:.3} — the \
         offset is derived from this rig and one of them has moved"
    );
}

/// A rig-space rotation in VIEW space. [`view`]'s companion: the arms carry a
/// yaw of π, which as a quaternion is `(0, 1, 0, 0)`, so composing it on the
/// left is the whole transform.
fn view_rot(q: Q) -> Q {
    qmul([0.0, 1.0, 0.0, 0.0], q)
}

/// The angle between two rotations, radians.
fn angle_between(a: Q, b: Q) -> f32 {
    let d = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs();
    2.0 * d.clamp(-1.0, 1.0).acos()
}

/// `Quat::from_euler(EulerRot::YXZ, a, b, c)` as this file's `(x, y, z, w)`
/// array: intrinsic Y then X then Z, so `Ry(a) · Rx(b) · Rz(c)`.
fn euler_yxz(a: f32, b: f32, c: f32) -> Q {
    let ax = |i: usize, t: f32| {
        let mut q = [0.0, 0.0, 0.0, (t / 2.0).cos()];
        q[i] = (t / 2.0).sin();
        q
    };
    qmul(qmul(ax(1, a), ax(0, b)), ax(2, c))
}

#[test]
fn the_grip_hangs_the_item_exactly_where_the_camera_used_to() {
    // **The gate that makes parenting to the hand a derivation rather than a
    // taste call.** `VIEWMODEL_GRIP_M`/`_Q`/`_SCALE` are the item's transform
    // in the `RightHand` bone's own frame, and the property that picks them is
    // that composing them onto the hand reproduces the pose the item had as a
    // child of the CAMERA — `VIEWMODEL_HOLD` with `VIEWMODEL_TILT` — so
    // nothing about the resting frame moved and only the following is new.
    //
    // Three ways this goes wrong and none of them is a compile error: a
    // re-import moves the hand bone, a retarget moves the hold clip, or
    // somebody nudges `VIEWMODEL_HOLD` and leaves the grip behind. All three
    // land here.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let bone = glb
        .node(HOLD_BONE)
        .unwrap_or_else(|| panic!("{RIG}: no {HOLD_BONE} bone"));
    // The clip's FIRST frame, which is what the constant was derived at.
    let (ht, hr, hs) = glb.skeleton_trs(clip, 0.0)[bone];
    let (ht, hr) = (view(ht), view_rot(hr));

    // The scale first, because it is the one an eye cannot check: the item's
    // own offsets are metres and a bone on this rig carries the root's 0.01.
    assert!(
        (hs[0] - hs[1]).abs() < 1e-6 && (hs[1] - hs[2]).abs() < 1e-6,
        "{HOLD_BONE} is non-uniformly scaled {hs:?} — the grip's single \
         VIEWMODEL_GRIP_SCALE cannot express that"
    );
    let want_scale = 1.0 / hs[0];
    assert!(
        (VIEWMODEL_GRIP_SCALE - want_scale).abs() / want_scale < 1e-3,
        "VIEWMODEL_GRIP_SCALE is {VIEWMODEL_GRIP_SCALE} where the hand's own \
         scale asks for {want_scale} — the item would draw {:.0}× life size",
        VIEWMODEL_GRIP_SCALE / want_scale
    );

    // Then the frame: hand ∘ grip must land on the hold pose.
    let g = [
        VIEWMODEL_GRIP_M.x * hs[0],
        VIEWMODEL_GRIP_M.y * hs[1],
        VIEWMODEL_GRIP_M.z * hs[2],
    ];
    let r = rotate(hr, g);
    let at = [ht[0] + r[0], ht[1] + r[1], ht[2] + r[2]];
    let target = [VIEWMODEL_HOLD.x, VIEWMODEL_HOLD.y, VIEWMODEL_HOLD.z];
    let off = dist(at, target);
    assert!(
        off < 0.002,
        "the grip puts the item {:.1} mm from VIEWMODEL_HOLD — it is derived \
         from the hand and one of them has moved",
        off * 1000.0
    );

    let grip_q = [
        VIEWMODEL_GRIP_Q.x,
        VIEWMODEL_GRIP_Q.y,
        VIEWMODEL_GRIP_Q.z,
        VIEWMODEL_GRIP_Q.w,
    ];
    let tilt = euler_yxz(VIEWMODEL_TILT.x, VIEWMODEL_TILT.y, VIEWMODEL_TILT.z);
    let a = angle_between(qmul(hr, grip_q), tilt);
    assert!(
        a < 0.01,
        "the grip's orientation is {:.2}° off VIEWMODEL_TILT — the item would \
         sit in the hand at the wrong angle",
        a.to_degrees()
    );
}

#[test]
fn the_swing_keeps_the_item_in_frame_and_the_dead_arm_out_of_it() {
    // **The one thing about a swing arc that is checkable without a GPU**, and
    // it is the thing that actually decided the arc. The obvious way to
    // animate a first-person swing is to play the rig's own `Sword_Attack` on
    // the viewmodel arms — and measured, that clip carries the right hand
    // BEHIND the camera for 40% of its length and to ndc y −2.36 at the
    // strike, because it is authored for a body seen from outside. So the
    // stroke is `swing_pose`, and what makes it a design rather than a dial is
    // that the whole of it stays in shot.
    //
    // Both halves are asserted, because they fail in opposite directions: too
    // small an arc and nothing moves (the defect), too large and the item
    // leaves the frame — or drags the collapsed off-arm INTO it, which is the
    // speck-of-skin failure `the_hidden_arm_collapses_to_a_point_off_screen`
    // exists for, now that the collapse point rides a rotation rather than
    // three centimetres of bob.
    let glb = Glb::open(&asset_path(RIG));
    let clip = client::render::anim::ARMS_HOLD_CLIP;
    let hold = [VIEWMODEL_HOLD.x, VIEWMODEL_HOLD.y, VIEWMODEL_HOLD.z];

    const N: usize = 120;
    let mut path = 0.0f32;
    let mut prev: Option<V3> = None;
    let (mut lo, mut hi, mut wide) = (f32::MAX, f32::MIN, 0.0f32);
    let mut near_dead = f32::MAX;
    // The collapse point over the hold loop — the swing rides on top of
    // whatever the clip is doing with that bone.
    let dead_pts: Vec<V3> = steps(&glb, clip)
        .into_iter()
        .map(|t| collapsed_off_arm(&glb, clip, t))
        .collect();

    for i in 0..=N {
        let s = i as f32 / N as f32;
        let (rot, off) = swing_pose(s);
        let rig = rig_transform(rot, off);
        let place = |p: V3| {
            let v = rig.transform_point(bevy::math::Vec3::new(p[0], p[1], p[2]));
            [v.x, v.y, v.z]
        };

        let at = place(hold);
        if let Some(p) = prev {
            path += dist(at, p);
        }
        prev = Some(at);
        let (x, y) = ndc(at).unwrap_or_else(|| {
            panic!("the swing takes the item to {at:?} at s={s:.2} — behind the near plane")
        });
        assert!(
            x.abs() <= 1.20 && (-1.15..=1.05).contains(&y),
            "the swing takes the item to ndc ({x:.2}, {y:.2}) at s={s:.2}, \
             which is outside the frame it is supposed to sweep across"
        );
        lo = lo.min(y);
        hi = hi.max(y);
        wide = wide.max(x.abs());

        for p in &dead_pts {
            let q = place(*p);
            assert!(
                !on_screen(q),
                "{VIEWMODEL_HIDDEN_ARM}'s collapse point is dragged to {q:?} \
                 at s={s:.2}, which is inside the frame"
            );
            // Behind the camera the whole way. Measured as a DEPTH rather than
            // as an ndc margin, because ndc has no answer behind the lens —
            // and the depth is the property that makes the arc safe at all.
            near_dead = near_dead.min(q[2]);
        }
    }

    // **The floor is the defect, stated as a number.** *"the swing animation
    // is the most underwhelming thing ever. hardly anything moves"* — the arc
    // it replaced rotated the item 1.15 rad about its own grip and pushed the
    // rig 13 cm, so the grip itself travelled ~26 cm over the whole stroke.
    // Half a metre is comfortably past that and comfortably under what the
    // frame allows.
    assert!(
        path > 0.5,
        "the whole swing moves the grip {path:.2} m — that is the arc this \
         replaced, not a swing"
    );
    assert!(
        hi - lo > 0.8,
        "the swing spans only {:.2} of the frame vertically",
        hi - lo
    );
    assert!(
        near_dead > 1.0,
        "the swing brings the collapsed arm to {near_dead:.2} m — it has to \
         stay well behind the camera for the whole stroke"
    );
}

#[test]
fn the_swing_pulse_is_smooth_at_both_ends_and_at_its_peak() {
    // `bump`'s doc claims C¹ at three points, and a slope step in a stroke
    // this size is a visible flick. Same shape as `sim-core`'s `contour.rs`:
    // gate the MECHANISM (a derivative), not a picture.
    //
    // Proven red under the shape it replaced — a bare `sin(π u)`, whose
    // derivative at u = 0 is π rather than 0.
    let h = 1e-3;
    for attack in [0.5, VIEWMODEL_SWING_ATTACK] {
        let d = |u: f32| (bump(u + h, attack) - bump(u - h, attack)) / (2.0 * h);
        for (u, what) in [(0.0, "the start"), (1.0, "the end"), (attack, "the peak")] {
            assert!(
                d(u).abs() < 0.05,
                "bump(attack {attack}) has slope {:.3} at {what} (u={u}) — a \
                 step there is a flick in the middle of a stroke",
                d(u)
            );
        }
        assert!(
            (bump(attack, attack) - 1.0).abs() < 1e-5,
            "the pulse must reach exactly 1 at its peak"
        );
        assert_eq!(bump(0.0, attack), 0.0);
        assert_eq!(bump(1.0, attack), 0.0);
    }
    // The two pulses meet at the wind-up boundary, and both are zero there —
    // which is what makes the composed stroke C¹ across the join too.
    assert_eq!(bump(1.0, 0.5), 0.0, "the wind-up ends at rest");
    assert_eq!(
        bump(0.0, VIEWMODEL_SWING_ATTACK),
        0.0,
        "the strike starts at rest"
    );
    let (rot, off) = swing_pose(VIEWMODEL_SWING_WINDUP);
    // Componentwise rather than `Quat::angle_between`, which is
    // `acos(|dot|)` — and the dot of a float quaternion with an exact copy
    // of itself can land a rounding step above 1, where `acos` is NaN and
    // every comparison against it is false. `tests/remote_hand.rs` hit that
    // for real.
    assert!(
        rot.to_array()
            .iter()
            .zip([0.0, 0.0, 0.0, 1.0])
            .all(|(a, b)| (a.abs() - b).abs() < 1e-4)
            && off.length() < 1e-4,
        "the rig passes back through its rest pose at the wind-up/strike join"
    );
}

#[test]
fn the_hand_bone_is_where_the_rig_says_and_the_retired_offset_was_not() {
    // **A gate about the `RightHand` bone rather than about the viewmodel**,
    // and it lives here because this is the file that reads the GLB. A second
    // JSON-and-forward-kinematics decoder in `tests/remote_hand.rs` is the
    // two-decoders trap `CLAUDE.md` records for JPEG, one format over — so
    // that file gates the arithmetic that composes onto this bone, and this
    // one gates where the bone actually is.
    //
    // ## The retraction
    //
    // `bodies::BODY_PALM` put a remote player's held item at (0.22, 1.25,
    // 0.18) in the body's own frame, derived from a stated convention:
    // *"the rig stands facing +Z with +Y up, so right is +X"*. The file says
    // otherwise, and `render/viewmodel.rs` had already measured it —
    // **right is −X on this skeleton**. So every remote in the game carried
    // its axe on the wrong shoulder, 48 cm too high, for the whole life of
    // the feature, and no gate could see it because every assertion about
    // `BODY_PALM` measured it against itself.
    let glb = Glb::open(&asset_path(RIG));
    let bone = glb
        .node(HOLD_BONE)
        .unwrap_or_else(|| panic!("{RIG}: no {HOLD_BONE} bone"));

    // The body's own frame: the rig at the origin, unrotated and unscaled by
    // anything the client adds — which is what `bodies::stream` spawns
    // (`Transform::from_translation(pos).with_rotation(facing)` about the
    // body's own origin), so a point here is a point in that body's space.
    let (at, _, sc) = glb.skeleton_trs(client::render::anim::ARMS_HOLD_CLIP, 0.0)[bone];
    let idle = glb.skeleton_trs("Idle_Loop", 0.0)[bone].0;
    let _ = at;

    assert!(
        idle[0] < 0.0,
        "{HOLD_BONE} sits at x {:.3} — if the rig's right hand has moved to \
         +X, `viewmodel.rs`'s measured chirality and every grip derived from \
         it have to be re-derived, not just this assertion relaxed",
        idle[0]
    );
    let retired = [
        RETIRED_BODY_PALM.x,
        RETIRED_BODY_PALM.y,
        RETIRED_BODY_PALM.z,
    ];
    let off = dist(idle, retired);
    assert!(
        off > 0.5,
        "the retired offset is {off:.3} m from the hand — if it is that close \
         now, the rig moved and this retraction needs re-reading rather than \
         re-asserting"
    );
    assert!(
        retired[0] * idle[0] < 0.0,
        "the retired offset is on the same side as the hand ({:.3} vs \
         {:.3}); the wrong-shoulder half of the retraction is stale",
        retired[0],
        idle[0]
    );

    // And the bone's scale is the glTF root's centimetres, which is what
    // `VIEWMODEL_GRIP_SCALE` cancels for BOTH hands — the property
    // `remote_hand.rs` composes onto without opening this file.
    assert!(
        (VIEWMODEL_GRIP_SCALE * sc[0] - 1.0).abs() < 1e-3,
        "the bone's own scale is {} and the grip cancels {VIEWMODEL_GRIP_SCALE} \
         — a remote's item would draw {:.0}× life size",
        sc[0],
        VIEWMODEL_GRIP_SCALE * sc[0]
    );
}

#[test]
fn dress_arms_still_writes_both_halves_of_the_collapse() {
    // **The one thing the arithmetic above cannot see.** Every other gate in
    // this file reads the shipped GLB and the shipped constants; none of them
    // runs `dress_arms`, so deleting the line that applies
    // `VIEWMODEL_HIDDEN_OFFSET` leaves all of them green and puts a speck of
    // skin back in the frame. Same shape as `tests/sound.rs`'s call-site grep
    // and `tests/ui.rs` §H: when the defect is a CALL SITE and not a value,
    // the gate has to read the source.
    let src = std::fs::read_to_string("src/render/viewmodel.rs").expect("viewmodel.rs");
    let body = src
        .split_once("pub fn dress_arms(")
        .expect("dress_arms is gone — this gate is stale")
        .1;
    let body = body.split_once("\npub fn ").map_or(body, |(b, _)| b);
    // **Comments stripped first, and finding that out cost the mutant.** The
    // body carries `// see [VIEWMODEL_HIDDEN_OFFSET] for why the scale alone
    // does not do it` two lines above the write, so a bare `contains` was
    // green with the write deleted — a gate satisfied by the prose describing
    // the thing it is checking for.
    let code: String = body
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    for want in [
        "t.scale = Vec3::splat(VIEWMODEL_HIDDEN_SCALE)",
        "t.translation = VIEWMODEL_HIDDEN_OFFSET",
    ] {
        assert!(
            code.contains(want),
            "dress_arms no longer does `{want}` — the off arm is only half \
             collapsed, which is invisible to every other gate here"
        );
    }
}

#[test]
fn the_hold_clip_never_animates_the_hidden_bones_pose() {
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
    //
    // **Both channels, since `VIEWMODEL_HIDDEN_OFFSET`.** The collapse is a
    // scale AND a translation now, and the translation is the load-bearing
    // half — it is what parks the joint behind the camera. `Pistol_Idle_Loop`
    // does write translation, on the hips, so "this clip animates no
    // translation" is not true in general and has to be asked about THIS bone.
    for path in ["scale", "translation"] {
        let written: Vec<usize> = glb
            .channels(clip)
            .into_iter()
            .filter(|(_, p)| p == path)
            .map(|(n, _)| n)
            .filter(|&n| glb.descends_from(n, hidden))
            .collect();
        assert!(
            written.is_empty(),
            "{clip} animates {path} on {written:?}, under \
             {VIEWMODEL_HIDDEN_ARM} — the one-shot collapse in dress_arms \
             would be overwritten on the next frame. Re-apply it after \
             AnimationSystems (anim::head_look's shape) or pick a clip that \
             leaves it alone"
        );
    }
}
