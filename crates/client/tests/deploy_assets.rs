//! Gate: a generated deployable model is the shape its `DEPLOY` row promises,
//! and is the shape `build_kit` assumes when it loads it.
//!
//! **Two silent failures, and neither shows up as a compile error.**
//!
//! The first is structural. `build_kit` loads `GltfAssetLabel::Primitive {
//! mesh: 0, primitive: 0 }` and `Material { index: 0 }` — one mesh, one
//! primitive, one material. Every asset generated on 2026-08-11 is exactly
//! that, which is *why* the loader could stay a one-line change against the
//! cuboid it replaced. But nothing about a `.glb` guarantees it: a
//! multi-primitive model would load its FIRST primitive and draw a fraction
//! of itself with no error anywhere. That is a wrong picture, not a crash.
//!
//! The second is dimensional, and is the whole reason `ci/import_meshy.py`
//! exists. The generator's own size estimate is unreliable — measured off by
//! 9x on a spear — so scale is imposed from `DEPLOY` at import. A model
//! re-exported without that step, or imported against a stale row, is a
//! deployable that does not match its own build ghost. The ghost reads
//! `deploy_size`; this reads the file; they have to agree.
//!
//! It runs headless: a GLB header is JSON and a bounding box is arithmetic,
//! so no GPU, window or asset server is involved — the same reason
//! `tests/tree.rs` is arithmetic rather than a screenshot.

#![cfg(feature = "render")]

use client::render::structures::{deploy_size, DEPLOY_ASSET};

/// Assets live beside the crate, not inside it — `assets/` is what the depot
/// ships and `crates/client/` is what cargo builds. Same hop `tests/ui.rs`
/// makes for the baked icons.
fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

/// The JSON chunk of a GLB, parsed enough to count parts and bound vertices.
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

    fn primitives(&self) -> Vec<&serde_json::Value> {
        self.json["meshes"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["primitives"].as_array())
                    .flatten()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The union of every primitive's POSITION bounds, in metres. Read off the
    /// accessor rather than the vertex buffer: glTF requires POSITION to carry
    /// `min`/`max`, and `import_meshy.py` rewrites them when it bakes scale.
    fn extent(&self) -> [f32; 3] {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in self.primitives() {
            let a = &self.json["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(a["min"][k].as_f64().unwrap() as f32);
                hi[k] = hi[k].max(a["max"][k].as_f64().unwrap() as f32);
            }
        }
        [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
    }

    fn min_y(&self) -> f32 {
        self.primitives()
            .iter()
            .map(|p| {
                let i = p["attributes"]["POSITION"].as_u64().unwrap() as usize;
                self.json["accessors"][i]["min"][1].as_f64().unwrap() as f32
            })
            .fold(f32::MAX, f32::min)
    }
}

#[test]
fn every_declared_model_ships() {
    for (arch, path) in DEPLOY_ASSET.iter().enumerate() {
        let Some(rel) = path else { continue };
        let p = asset_path(rel);
        assert!(
            p.exists(),
            "archetype {arch} declares {rel} and it is not in the tree — \
             the deployable would draw nothing at all"
        );
    }
}

#[test]
fn each_model_is_the_single_primitive_build_kit_loads() {
    for (arch, path) in DEPLOY_ASSET.iter().enumerate() {
        let Some(rel) = path else { continue };
        let g = Glb::open(&asset_path(rel));
        let n = g.primitives().len();
        assert_eq!(
            n, 1,
            "archetype {arch} ({rel}) has {n} primitives, and `build_kit` loads \
             `Primitive {{ mesh: 0, primitive: 0 }}` — it would draw 1/{n} of \
             this model and report no error"
        );
        let mats = g.json["materials"].as_array().map_or(0, |m| m.len());
        assert_eq!(
            mats, 1,
            "archetype {arch} ({rel}) has {mats} materials and `build_kit` \
             loads `Material {{ index: 0 }}` — the rest would never be applied"
        );
    }
}

#[test]
fn each_model_fits_the_row_that_sizes_its_ghost() {
    for (arch, path) in DEPLOY_ASSET.iter().enumerate() {
        let Some(rel) = path else { continue };
        let g = Glb::open(&asset_path(rel));
        let got = g.extent();
        let want = deploy_size(arch);
        for (k, axis) in ["x", "y", "z"].iter().enumerate() {
            // A tenth of a millimetre of float slack, and no more: the importer
            // scales to fit INSIDE the row, so overflow is a real defect and
            // never rounding.
            assert!(
                got[k] <= want[k] + 1e-4,
                "archetype {arch} ({rel}) is {:.3} m on {axis} against a DEPLOY \
                 row of {:.3} — it pokes out of the volume its own build ghost \
                 previews",
                got[k],
                want[k]
            );
        }
        // Fitting inside is necessary and not sufficient: a model scaled to a
        // hundredth would also fit. It has to fill its longest axis, which is
        // the one the uniform fit solved for.
        let fill = (0..3).map(|k| got[k] / want[k]).fold(0.0f32, f32::max);
        assert!(
            fill > 0.9,
            "archetype {arch} ({rel}) fills only {:.0}% of its longest DEPLOY \
             axis — either the import skipped its scale step or the row was \
             written for a different model",
            fill * 100.0
        );
    }
}

/// Which archetypes are allowed to emit light, and it is a short list on
/// purpose. `ARCH_FIRE` is the campfire; its generated emissive map genuinely
/// carries ember glow (measured peak 0.24) and is the reason `import_meshy.py`
/// has a `--emissive` flag at all rather than always stripping.
const MAY_GLOW: [u8; 1] = [sim_core::deploy::ARCH_FIRE];

#[test]
fn only_a_fire_glows() {
    for (arch, path) in DEPLOY_ASSET.iter().enumerate() {
        let Some(rel) = path else { continue };
        let g = Glb::open(&asset_path(rel));
        let lit = g.json["materials"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|m| match m.get("emissiveFactor") {
                Some(f) => (0..3).any(|k| f[k].as_f64().unwrap_or(0.0) > 1e-6),
                // glTF's default emissiveFactor is [0,0,0], so absent is dark.
                None => false,
            });
        let allowed = MAY_GLOW.contains(&(arch as u8));
        assert_eq!(
            lit,
            allowed,
            "archetype {arch} ({rel}) emissive={lit}, allowed={allowed}. \
             The generator ships `emissiveFactor = [1,1,1]` on nearly every \
             asset and its map is usually junk — a wooden spear measured a \
             0.53 peak, i.e. a stick that glows in the dark. \
             `ci/import_meshy.py` strips it unless `--emissive` is passed, so \
             this is either an import that skipped the strip or a new thing \
             that genuinely burns and belongs in MAY_GLOW."
        );
    }
}

#[test]
fn every_model_stands_on_the_ground() {
    for (arch, path) in DEPLOY_ASSET.iter().enumerate() {
        let Some(rel) = path else { continue };
        let y = Glb::open(&asset_path(rel)).min_y();
        // `spawn_deploy` places these by their own origin, so a model centred
        // on its middle sinks half of itself into the terrain. `origin_at:
        // "bottom"` held on every asset generated so far; this is what notices
        // when the next one does not.
        assert!(
            y.abs() < 1e-3,
            "archetype {arch} ({rel}) has min.y = {y:+.4} — it is not authored \
             with its feet at zero and would sit buried or float"
        );
    }
}
