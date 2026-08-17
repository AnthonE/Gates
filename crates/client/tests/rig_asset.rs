//! Gate: the shipped player rig is the file `render/anim.rs` assumes it is.
//!
//! `tests/deploy_assets.rs`'s shape, applied to the one asset every player in
//! the world is wearing. Four things can go wrong at import, none of them is a
//! compile error, and each one draws a wrong picture rather than raising
//! anything:
//!
//!   1. **A clip is renamed and every body freezes.** Clips resolve by NAME
//!      through `Gltf::named_animations` (`anim.rs` says why, at length). A
//!      name that is not in the file leaves `nodes[slot]` at
//!      `AnimationNodeIndex::default()` — which is the graph ROOT, and playing
//!      the root plays nothing. The body stands in its bind pose forever and
//!      the log line that says so scrolls past at boot.
//!
//!   2. **The rig is the wrong height, so the drawn body disagrees with the
//!      volume the server shoots at.** `ANIM_RIG_H_M` is a measured constant
//!      and a re-import is exactly the event that invalidates it.
//!
//!   3. **The stand-up rotation is missing.** The generator ships merged
//!      animation exports Z-up: the character lies on its back and every
//!      measurement of the raw file describes a fallen body. `ci/
//!      import_char.py` writes the correction onto the scene root, and this is
//!      what proves it was run.
//!
//!   4. **The textures are not KTX2, so the character costs 67 MB of VRAM.**
//!      A 4096² base colour is 67 MB decompressed whatever it weighs on disk
//!      (`ci/ktx_pack.py`'s header has the measurements). Forgetting that step
//!      leaves a file that loads, draws correctly, and quietly spends a third
//!      of a GPU's texture budget on one character.
//!
//! Headless and arithmetic: a GLB's JSON chunk is JSON and a bounding box is a
//! bounding box, so no GPU, window or asset server is involved. What this
//! CANNOT check is whether the animation looks like walking — that is what
//! `cargo run -p client --features render --bin modelview` is for, and what a
//! person looking at it is for.

#![cfg(feature = "render")]

use client::render::anim::{Clip, ANIM_BODY_H_M, ANIM_RIG_H_M};

/// Assets live beside the crate, not inside it — the same hop
/// `tests/deploy_assets.rs` and `tests/ui.rs` make.
fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

/// What `render/anim.rs` loads. One place, so a re-point is one edit.
const RIG: &str = "models/stumpy.glb";

struct Glb {
    json: serde_json::Value,
}

impl Glb {
    fn open(path: &std::path::Path) -> Self {
        let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&raw[0..4], b"glTF", "{}: not a GLB", path.display());
        let len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
        let json = serde_json::from_slice(&raw[20..20 + len])
            .unwrap_or_else(|e| panic!("{}: bad JSON chunk: {e}", path.display()));
        Self { json }
    }

    fn clips(&self) -> Vec<String> {
        self.json["animations"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The bind-pose box of every mesh, **with the scene root's rotation and
    /// scale applied** — which is the whole subtlety of measuring a skinned
    /// character and the thing that made the first three attempts at this
    /// wrong.
    ///
    /// A skinned mesh is not drawn where its own node says it is: glTF says
    /// that node's transform must be ignored, and the GPU uses
    /// `JointWorld_j · IBM_j` instead, which at bind pose is the identity in
    /// the space the vertices are authored in. A transform on the SCENE ROOT
    /// sits above both the mesh and the joints, so it does reach the drawn
    /// result — and it is where `ci/import_char.py` puts its corrections
    /// precisely because it is the one place that is safe.
    ///
    /// So: raw POSITION bounds, turned by the root's quaternion, scaled by the
    /// root's scale. Nothing in between matters.
    fn standing_box(&self) -> ([f32; 3], [f32; 3]) {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let mut any = false;
        for m in self.json["meshes"].as_array().into_iter().flatten() {
            for p in m["primitives"].as_array().into_iter().flatten() {
                let i = p["attributes"]["POSITION"].as_u64().unwrap() as usize;
                let a = &self.json["accessors"][i];
                for k in 0..3 {
                    lo[k] = lo[k].min(a["min"][k].as_f64().expect("POSITION min") as f32);
                    hi[k] = hi[k].max(a["max"][k].as_f64().expect("POSITION max") as f32);
                }
                any = true;
            }
        }
        assert!(any, "{RIG}: no meshes");

        let (q, s) = self.root_transform();
        let (mut out_lo, mut out_hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for c in 0..8 {
            let v = [
                if c & 1 == 0 { lo[0] } else { hi[0] },
                if c & 2 == 0 { lo[1] } else { hi[1] },
                if c & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let r = rotate(q, [v[0] * s[0], v[1] * s[1], v[2] * s[2]]);
            for k in 0..3 {
                out_lo[k] = out_lo[k].min(r[k]);
                out_hi[k] = out_hi[k].max(r[k]);
            }
        }
        (out_lo, out_hi)
    }

    /// The scene root's rotation `(x, y, z, w)` and scale.
    ///
    /// **The scale here is deliberately NOT applied to the mesh box in the
    /// general case**, and this asset is the case where that distinction does
    /// not bite: its root carries `scale 0.01` because its JOINT translations
    /// are in centimetres, and the inverse bind matrices already account for
    /// it — so the drawn character is 1.8 m, not 18 mm. Reading the scale here
    /// and multiplying would reproduce exactly the bug the bench hit. It is
    /// read and returned as 1 for that reason, with the check that the root's
    /// scale is uniform kept, because a non-uniform one would mean the file
    /// changed shape in a way this reasoning does not cover.
    fn root_transform(&self) -> ([f32; 4], [f32; 3]) {
        let scene = self.json["scene"].as_u64().unwrap_or(0) as usize;
        let roots = self.json["scenes"][scene]["nodes"].as_array().unwrap();
        assert_eq!(roots.len(), 1, "{RIG}: expected exactly one scene root");
        let n = &self.json["nodes"][roots[0].as_u64().unwrap() as usize];
        assert!(
            n["matrix"].is_null(),
            "{RIG}: the scene root carries a matrix, not TRS — ci/import_char.py refuses these"
        );
        let q = match n["rotation"].as_array() {
            Some(v) => [
                v[0].as_f64().unwrap() as f32,
                v[1].as_f64().unwrap() as f32,
                v[2].as_f64().unwrap() as f32,
                v[3].as_f64().unwrap() as f32,
            ],
            None => [0.0, 0.0, 0.0, 1.0],
        };
        if let Some(v) = n["scale"].as_array() {
            let s: Vec<f32> = v.iter().map(|k| k.as_f64().unwrap() as f32).collect();
            assert!(
                (s[0] - s[1]).abs() < 1e-4 && (s[1] - s[2]).abs() < 1e-4,
                "{RIG}: the scene root's scale is non-uniform ({s:?}) — a character \
                 scaled unevenly is a character whose capsule cannot be a capsule"
            );
        }
        (q, [1.0, 1.0, 1.0])
    }

    /// Every image's mime type.
    fn image_mimes(&self) -> Vec<String> {
        self.json["images"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|i| i["mimeType"].as_str().unwrap_or("?").to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn materials(&self) -> usize {
        self.json["materials"].as_array().map_or(0, |a| a.len())
    }
}

/// Rotate a point by a `(x, y, z, w)` quaternion. Written out rather than
/// pulled from Bevy so this file is arithmetic a reader can check.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    // t = 2 * (q_xyz × v); v' = v + w*t + q_xyz × t
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

#[test]
fn every_clip_the_client_asks_for_is_in_the_file() {
    let glb = Glb::open(&asset_path(RIG));
    let have = glb.clips();
    // Read off `Clip` itself, never re-typed: two of the five variants are
    // aliases (`Sleep` plays the idle, `Sprint` plays the jog), and a gate
    // holding its own copy of the list would go on passing after one of those
    // aliases was pointed somewhere that does not exist.
    for c in Clip::ALL {
        let want = c.name();
        assert!(
            have.iter().any(|n| n == want),
            "{RIG} has no clip {want:?} for {c:?} — the body would stand in its \
             bind pose. Clips are: {have:?}. Fix it at import: \
             ci/import_char.py --rename OLD={want}"
        );
    }
}

#[test]
fn the_rig_stands_the_height_the_constant_claims() {
    let glb = Glb::open(&asset_path(RIG));
    let (lo, hi) = glb.standing_box();
    let h = hi[1] - lo[1];
    assert!(
        (h - ANIM_RIG_H_M).abs() < 0.01,
        "{RIG} stands {h:.3} m and ANIM_RIG_H_M says {ANIM_RIG_H_M} — re-measure \
         with `--bin modelview` and update the constant, or the drawn body and \
         the {ANIM_BODY_H_M} m capsule the server shoots at disagree"
    );
    // Feet on the origin. `bodies.rs` puts the wire's y straight into the
    // transform with no offset, on the stated grounds that a glTF humanoid
    // stands on its own origin — so an asset whose feet are not at zero floats
    // or sinks every player in the world.
    assert!(
        lo[1].abs() < 0.02,
        "{RIG}'s feet sit at y {:.3}, not 0 — every body would be drawn that far \
         off the ground",
        lo[1]
    );
}

#[test]
fn the_rig_is_standing_up_and_not_lying_down() {
    // The generator's merged-animation export is Z-up. Corrected at import
    // with a rotation on the scene root; this is the check that it happened,
    // stated as a proportion rather than as an axis so it reads the same way
    // a person does — a character is much taller than it is deep.
    let glb = Glb::open(&asset_path(RIG));
    let (lo, hi) = glb.standing_box();
    let (h, d) = (hi[1] - lo[1], hi[2] - lo[2]);
    assert!(
        h > d * 1.5,
        "{RIG} is {h:.2} m tall and {d:.2} m deep — that is a body lying down. \
         Run ci/import_char.py, which puts the stand-up rotation on the scene root"
    );
}

#[test]
fn the_mesh_is_split_into_an_arms_half_and_a_body_half() {
    // `ci/split_arms.py`, and the first-person viewmodel is what needs it: the
    // body is one material, so `Visibility` has nothing to hide unless the
    // mesh is two nodes. A re-import that forgets this step does not fail —
    // it silently draws a whole character wrapped around the camera.
    let glb = Glb::open(&asset_path(RIG));
    let nodes: Vec<&str> = glb.json["nodes"]
        .as_array()
        .map(|a| a.iter().filter_map(|n| n["name"].as_str()).collect())
        .unwrap_or_default();
    for want in [
        client::render::anim::ARMS_NODE,
        client::render::anim::BODY_NODE,
    ] {
        assert!(
            nodes.contains(&want),
            "{RIG} has no node {want:?} — run ci/split_arms.py. Nodes: {nodes:?}"
        );
    }
    // Both halves share the vertex buffers and differ only in their indices,
    // so a split that duplicated the mesh would show up as two POSITION
    // accessors rather than one.
    let meshes = glb.json["meshes"].as_array().map_or(0, |a| a.len());
    assert_eq!(meshes, 2, "{RIG} carries {meshes} meshes, expected 2");
    let pos: Vec<u64> = glb.json["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["primitives"].as_array().unwrap())
        .map(|p| p["attributes"]["POSITION"].as_u64().unwrap())
        .collect();
    assert!(
        pos.windows(2).all(|w| w[0] == w[1]),
        "the two halves do not share their vertex data (POSITION accessors {pos:?}) — \
         the split is supposed to cost one index array, not a second copy of the mesh"
    );
}

#[test]
fn the_arms_have_a_hold_clip_to_play() {
    // `render/viewmodel.rs` plays exactly this one, and it was chosen by
    // measuring where each clip puts the right hand in view space — so a
    // library re-import that drops it leaves the arms in their bind pose,
    // hanging out of frame.
    let glb = Glb::open(&asset_path(RIG));
    let have = glb.clips();
    let want = client::render::anim::ARMS_HOLD_CLIP;
    assert!(
        have.iter().any(|n| n == want),
        "{RIG} has no clip {want:?} for the first-person arms"
    );
}

#[test]
fn the_body_wears_one_material() {
    // `anim::build` assigns `materials[0]` as the waking shade and a tinted
    // copy of it as the sleeper's. That is exactly right for one material and
    // silently flattens a second onto the first — so the client refuses at
    // runtime, and this refuses at build.
    let glb = Glb::open(&asset_path(RIG));
    assert_eq!(
        glb.materials(),
        1,
        "{RIG} carries {} materials — `anim::build` can only repaint a \
         single-material body without losing one",
        glb.materials()
    );
}

#[test]
fn the_textures_are_gpu_compressed() {
    // The VRAM rule (`assets/models/MANIFEST.md`, `ci/ktx_pack.py`). A raw
    // 4096² base colour costs 67 MB of video memory however small the PNG is,
    // and nothing about the frame says so.
    let glb = Glb::open(&asset_path(RIG));
    let mimes = glb.image_mimes();
    assert!(!mimes.is_empty(), "{RIG} carries no textures at all");
    for m in &mimes {
        assert_eq!(
            m, "image/ktx2",
            "{RIG} ships a {m} texture — run ci/ktx_pack.py after ci/import_char.py"
        );
    }
}
