//! Gate: the held-item models exist, load the way `viewmodel` loads them, and
//! answer to names `content/items.toml` actually uses.
//!
//! **The name assertion is the one that catches a real rename.** Items are
//! resolved by normalised display name and by nothing else, because the wire
//! carries no content id (`ui::hold`'s header argues it). That is cheap and it
//! has one failure mode: rename `"Stone Hatchet"` in content and the lookup
//! quietly stops matching, the model stops drawing, and every gate stays
//! green because the string was never wrong — it was only *unpaired*. So this
//! reads the content file and requires a live item behind every model.
//!
//! The structural half mirrors `tests/deploy_assets.rs`: `viewmodel::swap`
//! loads `Primitive { mesh: 0, primitive: 0 }`, which draws a fraction of a
//! multi-primitive model with no error anywhere.
//!
//! **Two sources, one law.** `HeldSrc::Glb` rows are gated off their file;
//! `HeldSrc::Gen` rows are BUILT here — the same constructors the client runs
//! at startup — and measured the same way, so a generated row whose geometry
//! drifts from its table entry is exactly as red as a regenerated asset
//! nobody re-measured. A `Gen` name with no generator panics right here,
//! which is this gate reaching the boot-time panic before a boot does.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use client::render::heldgen;
use client::ui::hold::{HeldSrc, HELD_MODELS};
use client::ui::icons::stem;

fn asset_path(rel: &str) -> std::path::PathBuf {
    std::path::Path::new("../../assets").join(rel)
}

fn glb_json(path: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&raw[0..4], b"glTF", "{}: not a GLB", path.display());
    let len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&raw[20..20 + len]).expect("GLB JSON chunk")
}

/// The `.glb` rows, which are the ones with a file to hold to.
fn glb_rows() -> impl Iterator<Item = (&'static str, &'static str)> {
    HELD_MODELS.iter().filter_map(|m| match m.src {
        HeldSrc::Glb(path) => Some((m.key, path)),
        HeldSrc::Gen(_) => None,
    })
}

#[test]
fn every_held_model_ships() {
    for (key, rel) in glb_rows() {
        assert!(
            asset_path(rel).exists(),
            "{key} declares {rel} and it is not in the tree — the item would \
             draw the generic stand-in tool forever"
        );
    }
}

#[test]
fn each_held_model_is_the_single_primitive_the_viewmodel_loads() {
    for (key, rel) in glb_rows() {
        let g = glb_json(&asset_path(rel));
        let prims: usize = g["meshes"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["primitives"].as_array())
                    .map(|p| p.len())
                    .sum()
            })
            .unwrap_or(0);
        assert_eq!(
            prims, 1,
            "{key} ({rel}) has {prims} primitives and `viewmodel::swap` loads \
             `Primitive {{ mesh: 0, primitive: 0 }}` — it would draw 1/{prims} \
             of the model and report nothing"
        );
        let mats = g["materials"].as_array().map_or(0, |m| m.len());
        assert_eq!(mats, 1, "{key} ({rel}) has {mats} materials, not 1");
    }
}

#[test]
fn nothing_held_glows() {
    // Same rule `deploy_assets.rs` states at length: the generator ships
    // `emissiveFactor = [1,1,1]` on nearly everything and its map is junk —
    // the spear here measured a 0.53 peak before the import stripped it.
    // Nothing a player carries emits light. **The torch did not become the
    // exception**, which is worth stating because this comment predicted it
    // would: torch light v0 gave its row a `PointLight` on the hand
    // (`ui::hold::TORCH_LIGHT`) and left its material black, so a carried
    // light and a carried emissive turned out to be different mechanisms
    // and this test needed neither to grow nor to go.
    // `tests/hand_light.rs` gates the other half.
    //
    // This is also why `fire.glb` — reused for the other deployables' rows —
    // has NO row of its own: the world's fire pit is lit and bakes a full
    // emissive, and a carried unlit one must not glow. Point a row at it and
    // this test is the one that goes red.
    for (key, rel) in glb_rows() {
        let g = glb_json(&asset_path(rel));
        for m in g["materials"].as_array().into_iter().flatten() {
            let lit = m
                .get("emissiveFactor")
                .is_some_and(|f| (0..3).any(|k| f[k].as_f64().unwrap_or(0.0) > 1e-6));
            assert!(
                !lit,
                "{key} ({rel}) has a non-zero emissiveFactor — it would glow \
                 in the dark. `ci/import_meshy.py` strips this unless \
                 `--emissive` is passed, so the import skipped a step."
            );
        }
    }
}

#[test]
fn nothing_generated_glows_either() {
    // The generated rows' half of `nothing_held_glows`: the material is code,
    // so the assertion runs on the very value the client will register.
    for m in &HELD_MODELS {
        let HeldSrc::Gen(name) = m.src else { continue };
        let mat = heldgen::material(name);
        let e = mat.emissive;
        assert!(
            e.red <= 1e-6 && e.green <= 1e-6 && e.blue <= 1e-6,
            "{} (generated {name:?}) has emissive {e:?} — nothing a player \
             carries emits light",
            m.key
        );
    }
}

#[test]
fn every_model_answers_to_an_item_that_exists() {
    let toml = std::fs::read_to_string("../../content/items.toml").expect("content/items.toml");
    // `name = "Stone Hatchet"` → `stone_hatchet`, the same normalisation the
    // lookup runs at runtime. Parsed by hand rather than by pulling a TOML
    // dependency into a render-feature test: one key, one shape.
    let names: Vec<String> = toml
        .lines()
        .filter_map(|l| l.trim().strip_prefix("name = "))
        .map(|v| stem(v.trim().trim_matches('"')))
        .collect();
    assert!(
        names.len() > 20,
        "parsed only {} names out of items.toml — the parse is wrong, not the \
         content, and a broken parse would pass every assertion below",
        names.len()
    );
    for m in &HELD_MODELS {
        let key = m.key;
        assert!(
            names.iter().any(|n| n == key),
            "a held row answers to {key:?} and no item in content/items.toml \
             normalises to that. Either the item was renamed — in which case \
             the model silently stopped drawing — or the key is a typo."
        );
    }
}

/// The +Y extent of a built mesh's positions, for the generated rows.
fn mesh_height(mesh: &bevy::prelude::Mesh) -> f32 {
    let Some(VertexAttributeValues::Float32x3(pos)) =
        mesh.attribute(bevy::prelude::Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("a generated held mesh has no Float32x3 POSITION attribute");
    };
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    hi[1] - lo[1]
}

#[test]
fn the_declared_height_is_the_models_own_height() {
    // The grip is a fraction of `height_m`, so `height_m` being a
    // *restatement* of what the geometry measures is the whole assumption.
    // Regenerate an asset at a different size, forget this table, and the
    // hand silently moves to the wrong place along the haft — no error, just
    // a spear held by its point. This is the gate for that, and it holds the
    // generated rows to the same number their table line declares.
    //
    // **It measures +Y and not the longest axis, and that is the fix rather
    // than a tidy-up.** `grip_m` spends the fraction up +Y; measuring some
    // other axis here made the gate green on a rock whose fist sat 97% of
    // the way up it and on a building plan whose grip point was 8 cm off the
    // object altogether — both models are wider than they are tall, so the
    // number this test compared was never the number the grip was spent on.
    // With +Y on both sides, `every_grip_is_on_the_object`'s 0..=1 assertion
    // finally means what its name says.
    for m in &HELD_MODELS {
        let height = match m.src {
            HeldSrc::Glb(rel) => {
                let g = glb_json(&asset_path(rel));
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for p in g["meshes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x["primitives"].as_array())
                    .flatten()
                {
                    let a = &g["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
                    for k in 0..3 {
                        lo[k] = lo[k].min(a["min"][k].as_f64().unwrap() as f32);
                        hi[k] = hi[k].max(a["max"][k].as_f64().unwrap() as f32);
                    }
                }
                hi[1] - lo[1]
            }
            HeldSrc::Gen(name) => mesh_height(&heldgen::mesh(name)),
        };
        let err = (height - m.height_m).abs() / m.height_m;
        assert!(
            err < 0.02,
            "{} declares height_m = {:.3} and the geometry measures {:.3} \
             ({:+.0}%). The grip offset is derived from that number, so the \
             hand is now {:.3} m off along the haft.",
            m.key,
            m.height_m,
            height,
            err * 100.0,
            (height - m.height_m).abs() * m.grip_frac
        );
    }
}

#[test]
fn every_model_stands_on_its_own_feet() {
    // `ci/import_meshy.py`'s convention — authored standing, feet at y = 0 —
    // and the other half of the assumption `grip_m` rests on: the grip is
    // measured UP from the model's origin, so an asset whose origin sits in
    // its middle puts the fist half a model away from where the table says,
    // with `the_declared_height_is_the_models_own_height` still green because
    // an extent says nothing about where it sits.
    for (key, rel) in glb_rows() {
        let g = glb_json(&asset_path(rel));
        let mut lo = f32::MAX;
        for p in g["meshes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["primitives"].as_array())
            .flatten()
        {
            let a = &g["accessors"][p["attributes"]["POSITION"].as_u64().unwrap() as usize];
            lo = lo.min(a["min"][1].as_f64().unwrap() as f32);
        }
        assert!(
            lo.abs() < 0.01,
            "{key} ({rel}) has its lowest vertex at y = {lo:+.3} rather than \
             on 0. The grip is measured up from the origin, so the fist is \
             {:.3} m out of the model.",
            lo.abs()
        );
    }
}

#[test]
fn every_grip_is_on_the_object() {
    for m in &HELD_MODELS {
        assert!(
            (0.0..=1.0).contains(&m.grip_frac),
            "{} grips at {} of its own height — outside the model, so the item \
             would float beside the hand rather than in it",
            m.key,
            m.grip_frac
        );
        // A viewmodel may shrink a world-sized object into the hand; nothing
        // is ever drawn LARGER than it was authored, and zero would collapse
        // the mesh while `swap` still pointed a handle at it.
        assert!(
            m.scale > 0.0 && m.scale <= 1.0,
            "{} draws at scale {} — a held model shrinks or ships as-is",
            m.key,
            m.scale
        );
        // Positive by construction: the point `swap` lands on the fist is
        // measured up the model from its own foot.
        assert!(m.grip_m() >= 0.0, "{} grip point is below the foot", m.key);
    }
}

#[test]
fn the_lookup_is_unambiguous() {
    for (i, m) in HELD_MODELS.iter().enumerate() {
        let key = m.key;
        assert!(
            !HELD_MODELS[..i].iter().any(|o| o.key == key),
            "{key:?} appears twice in HELD_MODELS — `position` takes the first \
             and the second model is unreachable"
        );
    }
}
