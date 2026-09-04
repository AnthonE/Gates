//! Gate: nothing on this island is authored below a real surface's specular.
//!
//! **This catches a defect no other gate in the crate can see.** A wrong
//! `reflectance` changes no triangle, no mesh handle, no bit on the wire and no
//! replay hash — so `tests/tree.rs`, `tests/ground.rs`, `test_protocol_golden`
//! and `test_replay` are all green over a frame with no specular in it. It went
//! unnoticed across every material in the client because Bevy's `reflectance`
//! reads like a 0..1 "how shiny" slider and is actually a remap,
//! `F0 = 0.16 · reflectance²`, whose DEFAULT of 0.5 is the ordinary dielectric
//! 4%. Authored against the slider reading, the island shipped at F0 0.06%–1.1%
//! — 4 to 70 times under — and the consequence was measured a week before the
//! cause was found: `render/ground_splat.rs` wired four per-texel roughness
//! maps and recorded a null result, because roughness shapes a lobe and there
//! was no energy in the lobe.
//!
//! The proof it was a misreading rather than a style is that ONE module got it
//! right: `water.rs` derives its own in a comment, `sqrt(0.02/0.16) = 0.354`,
//! and notes the plane it replaced was "nearly two and a half times too
//! specular". §3 below holds the two definitions of that number together.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::fresnel::{self, f0_of, reflectance_for};
use client::render::props::{assets, PropAssets};
use client::render::textures::{MapSet, PropMaps};

/// The band a dielectric's normal-incidence specular has to sit in.
///
/// Real dielectrics run about 2% (water) to 5–6% (gemstone, dense mineral);
/// nothing an outdoor survival island is made of leaves it. The floor is what
/// actually matters here — everything shipped was under it — and it is set at
/// 1.5%, below water's 2%, so the sea is inside its own band rather than a
/// special case.
const F0_MIN: f32 = 0.015;
const F0_MAX: f32 = 0.06;

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

/// §1 — the constants say what they claim to say.
#[test]
fn the_constants_encode_the_f0_they_name() {
    for (name, refl, f0) in [
        ("DIELECTRIC", fresnel::DIELECTRIC, 0.04f32),
        ("FLESH", fresnel::FLESH, 0.028),
        ("WATER", fresnel::WATER, 0.02),
    ] {
        let want = reflectance_for(f0);
        assert!(
            (refl - want).abs() < 1e-5,
            "fresnel::{name} is {refl} but the F0 it documents ({f0}) wants \
             {want} — the constant and its own doc comment disagree"
        );
        assert!(
            (f0_of(refl) - f0).abs() < 1e-5,
            "fresnel::{name} round-trips to F0 {} rather than {f0}",
            f0_of(refl)
        );
    }
    // Bevy's own default IS the ordinary dielectric, which is the sentence that
    // explains the whole defect: the correct value for almost everything here
    // was what you get by not writing the field at all.
    assert!(
        (fresnel::DIELECTRIC - StandardMaterial::default().reflectance).abs() < 1e-6,
        "fresnel::DIELECTRIC has drifted from Bevy's own default"
    );
}

/// §2 — **every prop material delivers a physical specular.**
///
/// The load-bearing one. Proven red on the shipped values: `bark` at 0.08 is
/// F0 0.10%, sixteen times under the floor.
#[test]
fn every_prop_material_is_a_real_surface() {
    let (app, a) = fixture();
    let store = app.world().resource::<Assets<StandardMaterial>>();
    for (name, handle) in a.materials() {
        let m = store.get(handle).expect("material not in the store");
        let f0 = f0_of(m.reflectance);
        assert!(
            (F0_MIN..=F0_MAX).contains(&f0),
            "prop material `{name}` has reflectance {:.4}, delivering F0 \
             {:.4}% — outside the {:.1}%..{:.1}% a real dielectric occupies. \
             Bevy's `reflectance` is a remap (F0 = 0.16·r²), not a 0..1 \
             slider; see `render::fresnel`",
            m.reflectance,
            f0 * 100.0,
            F0_MIN * 100.0,
            F0_MAX * 100.0
        );
    }
}

/// §3 — the sea's own derivation and this module's constant are one number.
///
/// `water.rs` worked its out from the physics before `fresnel` existed and
/// keeps its derivation; this holds the two equal so they cannot drift into
/// two different oceans.
#[test]
fn the_sea_and_the_table_agree() {
    assert!(
        (client::render::water::WATER_REFLECTANCE - fresnel::WATER).abs() < 1e-6,
        "water.rs says {} and fresnel::WATER says {}",
        client::render::water::WATER_REFLECTANCE,
        fresnel::WATER
    );
}

/// §4 — flesh is not stone, and the gap is the right way round.
#[test]
fn hide_is_less_specular_than_granite() {
    assert!(
        f0_of(fresnel::FLESH) < f0_of(fresnel::DIELECTRIC),
        "FLESH is at least as specular as an ordinary dielectric — a pig lit \
         like wet granite is the failure this constant exists to avoid"
    );
}
