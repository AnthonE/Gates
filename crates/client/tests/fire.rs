//! Gate: a fire that is lit puts light on the ground, and one that is not does
//! not.
//!
//! **Until 2026-08-25 there was no dynamic light of any kind in this client** —
//! `PointLight`, `SpotLight` and a non-black `emissive` all returned zero
//! across `crates/client/src`. A campfire cooked meat, played a crackling cue
//! and cast nothing, and the ten minutes in eighty that are night were lit by a
//! single direction-free `AmbientLight`.
//!
//! Two halves, and neither is visible to any other gate here:
//!
//! 1. **The spawn shape.** Whether a burnable deployable gets a light child at
//!    all is a claim about a bundle, and `CLAUDE.md` is explicit that a spawn is
//!    not type-checked. Delete the `with_child` and nothing else in this crate
//!    goes red.
//! 2. **The predicate.** The light is driven off `ClientCore::ovens()`, the set
//!    `EV_OVEN` fills — no wire change and no new field on `DeployRec`. If that
//!    read inverted, every unlit fire on the shard would glow and every lit one
//!    would be dark, with the same green suite.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use client::render::structures::{
    apply_fire_lights, build_kit, burns, fire_lumens, spawn_deploy, FireLight,
};
use sim_core::deploy::DeployRec;
use sim_core::terrain;
// The archetype ids come from the SIM, which is where they are defined —
// `structures.rs` imports the same constants rather than restating them.
use sim_core::deploy::{ARCH_BOX, ARCH_FIRE, ARCH_FURNACE, ARCH_RECYCLER};

/// One fire's address.
const CX: u16 = 511;
const CZ: u16 = 733;
const LEVEL: u8 = 1;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<Image>();
    app
}

/// A recycler is a machine, not a fire — and `oven::toggle` switches it through
/// the same verb, so it lands in the same lit set a campfire does. If the light
/// keyed off `is_converter` it would glow.
#[test]
fn only_the_things_that_burn_burn() {
    assert!(burns(ARCH_FIRE), "a fire pit does not burn");
    assert!(burns(ARCH_FURNACE), "a furnace does not burn");
    assert!(
        !burns(ARCH_RECYCLER),
        "a recycler burns — it is a machine with a motor in it, and it appears \
         in ClientCore::ovens() exactly as a campfire does, so this predicate \
         is the only thing keeping a warm flickering light out of one"
    );
    assert!(!burns(ARCH_BOX), "a storage box burns");
}

/// The predicate half: dark unless the sim says this address is alight.
#[test]
fn a_fire_lights_only_when_the_sim_says_it_is_lit() {
    let mut app = app();
    app.world_mut().spawn((
        FireLight {
            cx: CX,
            cz: CZ,
            level: LEVEL,
        },
        PointLight {
            intensity: 0.0,
            ..default()
        },
    ));
    app.world_mut().flush();

    let read = |app: &mut App| -> f32 {
        app.world_mut()
            .query::<&PointLight>()
            .iter(app.world())
            .next()
            .expect("no light")
            .intensity
    };

    // Nothing lit anywhere.
    app.world_mut()
        .run_system_once(|q: Query<(&FireLight, &mut PointLight)>| {
            apply_fire_lights(q, &|_, _, _| false);
        })
        .unwrap();
    assert_eq!(
        read(&mut app),
        0.0,
        "an unlit fire is casting light — every fire pit on the island would \
         glow whether or not anyone put fuel in it"
    );

    // This address lit.
    app.world_mut()
        .run_system_once(|q: Query<(&FireLight, &mut PointLight)>| {
            apply_fire_lights(q, &|cx, cz, level| (cx, cz, level) == (CX, CZ, LEVEL));
        })
        .unwrap();
    assert_eq!(
        read(&mut app),
        fire_lumens(),
        "a lit fire is casting nothing"
    );

    // A DIFFERENT address lit — the fire must not read someone else's state.
    app.world_mut()
        .run_system_once(|q: Query<(&FireLight, &mut PointLight)>| {
            apply_fire_lights(q, &|cx, cz, level| (cx, cz, level) == (CX + 1, CZ, LEVEL));
        })
        .unwrap();
    assert_eq!(
        read(&mut app),
        0.0,
        "a fire lit up for a neighbour's address — the lookup is ignoring part \
         of the key"
    );
}

/// **The spawn shape.** A burnable deployable gets a light child; nothing else
/// does. Proven red by deleting the `with_child` — no other gate in the crate
/// moves.
#[test]
fn a_burnable_deployable_spawns_with_a_light_and_a_box_does_not() {
    let mut app = app();
    // A real `Kit`, because the question is what the real spawn builds.
    let kit = {
        let world = app.world_mut();
        let assets = world.resource::<AssetServer>().clone();
        let mut meshes = world.remove_resource::<Assets<Mesh>>().unwrap();
        let mut mats = world.remove_resource::<Assets<StandardMaterial>>().unwrap();
        let kit = build_kit(&assets, &mut meshes, &mut mats);
        world.insert_resource(meshes);
        world.insert_resource(mats);
        kit
    };

    let seed = 20260731u64;
    let haven = terrain::haven(seed);
    let rec = |cx: u16, cz: u16| DeployRec {
        cx,
        cz,
        level: LEVEL,
        loc: sim_core::build::LOC_PLANE,
        row: 0,
        owner: 1,
        hp: 100,
        uh: 0,
        open: false,
        has_lock: false,
        locked: false,
        dmg: 0,
    };

    let spawn = |app: &mut App, arch: u8, cx: u16| -> Entity {
        let r = rec(cx, CZ);
        let e = {
            let mut commands = app.world_mut().commands();
            spawn_deploy(&mut commands, &kit, seed, &haven, &r, arch, 0)
        };
        app.world_mut().flush();
        e
    };

    let fire = spawn(&mut app, ARCH_FIRE, CX);
    let kids = app
        .world()
        .entity(fire)
        .get::<Children>()
        .map(|c| c.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    let lights: Vec<_> = kids
        .iter()
        .filter(|e| app.world().entity(**e).get::<FireLight>().is_some())
        .collect();
    assert_eq!(
        lights.len(),
        1,
        "a fire pit spawned {} light children — one, or a lit campfire puts no          photon on the ground beside it",
        lights.len()
    );

    // Dark at spawn. The sim announces lit-ness; a light that arrives already
    // burning would glow on every fire pit the moment it streams in.
    let l = app
        .world()
        .entity(*lights[0])
        .get::<PointLight>()
        .expect("FireLight without a PointLight");
    assert_eq!(
        l.intensity, 0.0,
        "a fire spawns already alight — it must wait for EV_OVEN"
    );
    assert!(
        !l.shadows_enabled,
        "the fire light casts shadows — six faces of re-rasterised geometry per          fire, on top of the sun's four cascades"
    );

    // And a box gets nothing.
    let boxx = spawn(&mut app, ARCH_BOX, CX + 4);
    let box_lights = app
        .world()
        .entity(boxx)
        .get::<Children>()
        .map(|c| {
            c.iter()
                .filter(|e| app.world().entity(*e).get::<FireLight>().is_some())
                .count()
        })
        .unwrap_or(0);
    assert_eq!(box_lights, 0, "a storage box is casting firelight");
}
