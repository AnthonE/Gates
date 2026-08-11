//! Gate: the held-item models exist, load the way `viewmodel` loads them, and
//! answer to names `content/items.toml` actually uses.
//!
//! **The third assertion is the one that catches a real rename.** Items are
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

#![cfg(feature = "render")]

use client::ui::hold::HELD_MODELS;
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

#[test]
fn every_held_model_ships() {
    for m in &HELD_MODELS {
        let (key, rel) = (m.key, m.path);
        assert!(
            asset_path(rel).exists(),
            "{key} declares {rel} and it is not in the tree — the item would \
             draw the generic stand-in tool forever"
        );
    }
}

#[test]
fn each_held_model_is_the_single_primitive_the_viewmodel_loads() {
    for m in &HELD_MODELS {
        let (key, rel) = (m.key, m.path);
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
    // Nothing a player carries emits light. A torch would be the first, and
    // it would need this list to grow rather than this test to go.
    for m in &HELD_MODELS {
        let (key, rel) = (m.key, m.path);
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
        let (key, rel) = (m.key, m.path);
        assert!(
            names.iter().any(|n| n == key),
            "{rel} answers to {key:?} and no item in content/items.toml \
             normalises to that. Either the item was renamed — in which case \
             the model silently stopped drawing — or the key is a typo."
        );
    }
}

#[test]
fn the_declared_length_is_the_models_own_length() {
    // The grip is `-length_m * grip_frac`, so `length_m` being a *restatement*
    // of what the file measures is the whole assumption. Regenerate an asset
    // at a different size, forget this table, and the hand silently moves to
    // the wrong place along the haft — no error, just a spear held by its
    // point. This is the gate for that.
    for m in &HELD_MODELS {
        let g = glb_json(&asset_path(m.path));
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
        let longest = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
        let err = (longest - m.length_m).abs() / m.length_m;
        assert!(
            err < 0.02,
            "{} declares length_m = {:.3} and the file measures {:.3} ({:+.0}%). \
             The grip offset is derived from that number, so the hand is now \
             {:.3} m off along the haft.",
            m.key,
            m.length_m,
            longest,
            err * 100.0,
            (longest - m.length_m).abs() * m.grip_frac
        );
    }
}

#[test]
fn every_grip_is_on_the_object() {
    for m in &HELD_MODELS {
        assert!(
            (0.0..=1.0).contains(&m.grip_frac),
            "{} grips at {} of its own length — outside the model, so the item \
             would float beside the hand rather than in it",
            m.key,
            m.grip_frac
        );
        // Negative by construction, and worth asserting because the sign is
        // what puts the model below the hand rather than above it.
        assert!(
            m.grip_offset_m() <= 0.0,
            "{} grip offset is positive",
            m.key
        );
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
