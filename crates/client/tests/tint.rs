//! Gate: the prop tint pool is mean-neutral, deterministic, and actually used.
//!
//! **`ART.md` rule 7 — "no two identical instances adjacent at the same
//! rotation and scale" — was recorded as unmet in `props.rs`'s own header.**
//! The browser tinted per instance because it drew through an `InstancedMesh`
//! with a colour attribute; natively a shared material is what makes a forest
//! one draw call, so variation came from yaw, a ±10% scale and a six-mesh pool
//! and nothing else. At the ring's measured p90 of 328 trees, and one boulder
//! mesh for every granite outcrop on the island, that is the "looks procedural"
//! failure with the cause written down beside it.
//!
//! Three things here, and the first is the one that could silently ruin a
//! frame:
//!
//! 1. **The pool's mean is 1.** A tint is a value multiplier over an albedo
//!    that `prop albedo v1` measured into `ALBEDO_LUMA_BAND = [0.05, 0.55]`. A
//!    pool whose mean drifted off 1 would move the whole island's brightness as
//!    a side effect of a variation feature — and the coupled lighting owner
//!    would then be tuning against a debt nobody declared.
//! 2. **It is grey.** A per-instance HUE shift would fight the identity work
//!    `GROUND_ALBEDO` and the prop maps were measured into.
//! 3. **It is keyed on the cell key**, so two players in one clearing see the
//!    same boulder and walking away and back does not reshuffle the forest.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::props::{assets, PropAssets, TINT_POOL, TINT_SWING};
use client::render::textures::{MapSet, PropMaps};

fn fixture() -> (App, PropAssets) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<Image>();
    let world = app.world_mut();
    let mut meshes = world.remove_resource::<Assets<Mesh>>().unwrap();
    let mut materials = world.remove_resource::<Assets<StandardMaterial>>().unwrap();
    let mut images = world.remove_resource::<Assets<Image>>().unwrap();
    let maps = PropMaps {
        rock: MapSet::default(),
        bark: MapSet::default(),
        wood: MapSet::default(),
        stone: MapSet::default(),
        metal: MapSet::default(),
    };
    let a = assets(
        &mut meshes,
        &mut materials,
        &mut images,
        &maps,
        // Unresolved, like every `MapSet::default()` above it: this tier has no
        // filesystem and a material clones the handle either way.
        Handle::default(),
        client::render::props::PropModels::default(),
    );
    world.insert_resource(meshes);
    world.insert_resource(materials);
    world.insert_resource(images);
    (app, a)
}

/// The four tinted classes, and how many distinct base colours each must show.
#[test]
fn each_tinted_class_is_a_pool_of_distinct_greys_averaging_one() {
    let (app, a) = fixture();
    let store = app.world().resource::<Assets<StandardMaterial>>();

    for class in ["foliage", "needle", "rock", "bark"] {
        let greys: Vec<f32> = a
            .materials()
            .into_iter()
            .filter(|(n, _)| *n == class)
            .map(|(_, h)| {
                let c = store.get(h).expect("material not in store").base_color;
                let lin = c.to_linear();
                // Grey, not a hue — all three channels must agree.
                assert!(
                    (lin.red - lin.green).abs() < 1e-5 && (lin.green - lin.blue).abs() < 1e-5,
                    "`{class}` carries a COLOURED tint ({:?}) — a per-instance \
                     hue shift fights the identity work the maps were measured \
                     into; the modifier is a value multiplier",
                    c
                );
                lin.red
            })
            .collect();

        assert_eq!(
            greys.len(),
            TINT_POOL,
            "`{class}` published {} materials, not TINT_POOL ({TINT_POOL}) — \
             either the class is not pooled or `materials()` is not reporting \
             every entry, and the fresnel gate reads the same list",
            greys.len()
        );

        // 1 — the mean, and the assertion that keeps a variation feature from
        // being a brightness change.
        let mean = greys.iter().sum::<f32>() / greys.len() as f32;
        assert!(
            (mean - 1.0).abs() < 1e-5,
            "`{class}`'s tint pool averages {mean:.6}, not 1.0 — it would move \
             the island's brightness by {:.2}% as a side effect of adding \
             variation",
            (mean - 1.0) * 100.0
        );

        // 2 — the entries are actually distinct, and inside the declared swing.
        let mut sorted = greys.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for w in sorted.windows(2) {
            assert!(
                w[1] - w[0] > 1e-4,
                "`{class}` has two identical tints ({:.6}) — the pool is \
                 smaller than it claims and rule 7 is unmet by that much",
                w[0]
            );
        }
        let span = sorted[sorted.len() - 1] - sorted[0];
        assert!(
            (span - 2.0 * TINT_SWING).abs() < 1e-4,
            "`{class}`'s pool spans {span:.4} against the declared \
             2 x TINT_SWING = {:.4}",
            2.0 * TINT_SWING
        );
    }
}

/// The choice is a pure function of the cell key: same key, same tint, forever.
#[test]
fn a_slots_tint_is_stable_and_spread() {
    use client::render::props::tint_of;

    // Deterministic.
    for key in [0u32, 1, 7, 4_242, 91_137, u32::MAX] {
        assert_eq!(
            tint_of(key),
            tint_of(key),
            "tint_of is not a function of its argument"
        );
        assert!(tint_of(key) < TINT_POOL, "tint_of returned out of range");
    }

    // …and it uses the whole pool rather than collapsing onto one entry, which
    // is what a hash with a bad low-bit mix would do and what would leave the
    // forest exactly as uniform as before with every other assertion green.
    let mut seen = [0usize; TINT_POOL];
    for key in 0..4_096u32 {
        seen[tint_of(key)] += 1;
    }
    for (i, n) in seen.iter().enumerate() {
        assert!(
            *n > 4_096 / (TINT_POOL * 3),
            "tint {i} was chosen {n} times in 4,096 keys — the pool is \
             collapsing onto a subset and the forest is more uniform than the \
             pool size suggests: {seen:?}"
        );
    }
}
