//! Gate: a felled tree becomes a stump, and a respawned one becomes a tree.
//!
//! **Why this exists as a test rather than as a screenshot.** The sim has
//! owned chopping for a long time — ten swings, a yield table, a harvested bit
//! and a respawn timer, all authoritative and all on the wire — and the native
//! client's entire contribution was to keep drawing the tree. Proving the fix
//! end to end would mean chopping a tree over a live socket, which is not a
//! gate: it needs a shard, a tool, ten swings and an aim cone, and it fails
//! for a dozen reasons that have nothing to do with the swap.
//!
//! So the transport is behind a predicate (`props::apply_fell`) and this
//! drives it with a fixed key set. What it asserts is the whole renderer-side
//! contract: the mesh swaps, the material swaps, the stump lifts by exactly
//! half its own height, and every one of those reverses on respawn. What it
//! deliberately does NOT claim is that a real chop arrives — that is
//! `ClientCore`'s `EventMsg::SlotHarvested` path, which has its own coverage.
//!
//! No window, no GPU, no socket: `MinimalPlugins` plus the asset plugin.

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::props::{apply_fell, assets, Fellable, PropAssets, STUMP_LIFT_M};
use client::render::tree::CONIFER_POOL;

/// The variant this fixture's tree spawned as. **Deliberately not variant 0
/// and deliberately not `KEY % variants`**: the first cut of `apply_fell`
/// re-derived the variant from the key on the un-fell branch while
/// `spawn_slot` picked it from the slot's yaw, so a respawned tree came back
/// as a different tree — and the test written alongside it spawned by the
/// key too, so it agreed with the bug instead of catching it. Pinning a
/// variant that the key would NOT produce is what makes this assertion mean
/// something.
///
/// **Computed, not written down.** It was the literal `3` against a pool of 4;
/// generating the conifer took the pool to 3, which made it both an
/// out-of-range index and — at the next value down — exactly `KEY % 3`, so the
/// premise above quietly stopped holding. Both failures are silent in the
/// direction that matters: the test would still pass while testing nothing.
/// This picks the first variant that is neither 0 nor the one the key would
/// produce, and fails to compile if the pool is too small to have one.
const VARIANT: usize = distinct_variant();

const fn distinct_variant() -> usize {
    let key_derived = KEY as usize % CONIFER_POOL;
    let mut v = 1;
    while v < CONIFER_POOL {
        if v != key_derived {
            return v;
        }
        v += 1;
    }
    panic!(
        "CONIFER_POOL is too small for this gate's premise: it needs a variant \
         that is neither 0 nor KEY % CONIFER_POOL"
    )
}

/// The world an entity is spawned into, plus the shared prop assets.
fn fixture() -> (App, PropAssets) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    // The needle card is generated into this store; without it `assets` has
    // nowhere to put the image and the fixture cannot be built.
    app.init_asset::<Image>();

    // `assets` needs both stores mutably at once, which a `World` will not
    // hand out; taking them out and putting them back is the ordinary way.
    let world = app.world_mut();
    let mut meshes = world.remove_resource::<Assets<Mesh>>().unwrap();
    let mut materials = world.remove_resource::<Assets<StandardMaterial>>().unwrap();
    // The needle card is generated into `Assets<Image>` rather than loaded off
    // disk, so building the pool now needs the image store too.
    let mut images = world.remove_resource::<Assets<Image>>().unwrap();
    let a = assets(&mut meshes, &mut materials, &mut images);
    world.insert_resource(meshes);
    world.insert_resource(materials);
    world.insert_resource(images);
    (app, a)
}

const KEY: u32 = (137u32 << 16) | 42;
const GROUND_Y: f32 = 12.5;
const SCALE: f32 = 1.05;

fn spawn_tree(app: &mut App, a: &PropAssets) -> Entity {
    app.world_mut()
        .spawn((
            Fellable {
                key: KEY,
                variant: VARIANT,
                base_y: GROUND_Y,
                stumps: true,
                felled: false,
            },
            Mesh3d(a.pine_mesh(VARIANT).clone()),
            MeshMaterial3d(a.foliage_material().clone()),
            Transform {
                translation: Vec3::new(3.0, GROUND_Y, -7.0),
                scale: Vec3::splat(SCALE),
                ..default()
            },
            // Explicit, because the query matches on it. In the client Bevy
            // inserts it as a required component of `Mesh3d`; a fixture that
            // omitted it did not match the query at all, and the respawn test
            // then passed VACUOUSLY — it asserted a pine was drawn on an
            // entity nothing had ever touched.
            Visibility::Inherited,
        ))
        .id()
}

fn run(app: &mut App, a: &PropAssets, harvested: &dyn Fn(u32) -> bool) {
    let world = app.world_mut();
    let mut state = world.query::<(
        &mut Fellable,
        &mut Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Transform,
        &mut Visibility,
    )>();
    let q = state.query_mut(world);
    apply_fell(a, q, harvested);
}

#[test]
fn a_felled_tree_becomes_a_stump_and_lifts() {
    let (mut app, a) = fixture();
    let e = spawn_tree(&mut app, &a);

    // Nothing harvested: the tree stays a tree, and the transform is untouched.
    run(&mut app, &a, &|_| false);
    {
        let w = app.world();
        assert_eq!(w.get::<Mesh3d>(e).unwrap().0, *a.pine_mesh(VARIANT));
        assert_eq!(w.get::<Transform>(e).unwrap().translation.y, GROUND_Y);
        assert!(!w.get::<Fellable>(e).unwrap().felled);
    }

    // Harvested: stump mesh, wood material, lifted by half the stump's height
    // times the instance scale.
    run(&mut app, &a, &|k| k == KEY);
    {
        let w = app.world();
        assert_eq!(
            w.get::<Mesh3d>(e).unwrap().0,
            *a.stump_mesh(),
            "a harvested slot must stop drawing a tree"
        );
        assert_eq!(
            w.get::<MeshMaterial3d<StandardMaterial>>(e).unwrap().0,
            *a.wood_material()
        );
        let y = w.get::<Transform>(e).unwrap().translation.y;
        assert!(
            (y - (GROUND_Y + STUMP_LIFT_M * SCALE)).abs() < 1e-5,
            "stump lift is half its own height times the instance scale, got {y}"
        );
        assert!(w.get::<Fellable>(e).unwrap().felled);
    }

    // Idempotent: a second frame with the same state must not lift it again.
    run(&mut app, &a, &|k| k == KEY);
    {
        let y = app.world().get::<Transform>(e).unwrap().translation.y;
        assert!(
            (y - (GROUND_Y + STUMP_LIFT_M * SCALE)).abs() < 1e-5,
            "the swap must fire on the transition, not every frame — got {y}"
        );
    }
}

#[test]
fn a_respawned_slot_becomes_a_tree_again() {
    let (mut app, a) = fixture();
    let e = spawn_tree(&mut app, &a);

    run(&mut app, &a, &|k| k == KEY);
    // Assert it actually went down first. Without this the test passes on an
    // entity the system never touched, which is exactly how it passed while
    // the fixture was missing a component the query needed.
    assert_eq!(
        app.world().get::<Mesh3d>(e).unwrap().0,
        *a.stump_mesh(),
        "precondition: the tree must be a stump before respawn is tested"
    );

    run(&mut app, &a, &|_| false);

    let w = app.world();
    assert_eq!(
        w.get::<Mesh3d>(e).unwrap().0,
        *a.pine_mesh(VARIANT),
        "a respawned slot must draw THE SAME tree again — re-deriving the \
         variant from the key instead of remembering it swaps the tree out \
         from under the player on every respawn"
    );
    assert_ne!(
        *a.pine_mesh(VARIANT),
        *a.pine_mesh(KEY as usize),
        "the fixture's variant must not be the one a key-derived bug would \
         also produce, or this test agrees with the bug"
    );
    assert_eq!(
        w.get::<MeshMaterial3d<StandardMaterial>>(e).unwrap().0,
        *a.foliage_material()
    );
    let y = w.get::<Transform>(e).unwrap().translation.y;
    assert!(
        (y - GROUND_Y).abs() < 1e-5,
        "the lift must reverse exactly, or a slot that fells and respawns walks up the map — got {y}"
    );
    assert!(!w.get::<Fellable>(e).unwrap().felled);
}

/// The key a renderer tests with must be the key the server and `ClientCore`
/// use. A second packing would silently never match, and every tree would
/// stay standing with nothing red.
#[test]
fn the_cell_key_is_sim_cores_own() {
    assert_eq!(sim_core::gather::cell_key(137, 42), KEY);
}

/// A smashed barrel and a mined ore node carry the same harvested bit a tree
/// does, and the browser hid them with no replacement shape. Before this they
/// were not tagged at all and stayed on screen after the server removed them.
#[test]
fn a_node_that_leaves_no_stump_simply_disappears() {
    let (mut app, a) = fixture();
    let e = app
        .world_mut()
        .spawn((
            Fellable {
                key: KEY,
                variant: 0,
                base_y: GROUND_Y,
                stumps: false,
                felled: false,
            },
            Mesh3d(a.pine_mesh(0).clone()),
            MeshMaterial3d(a.foliage_material().clone()),
            Transform::from_xyz(0.0, GROUND_Y, 0.0),
            Visibility::Inherited,
        ))
        .id();

    run(&mut app, &a, &|k| k == KEY);
    assert_eq!(
        *app.world().get::<Visibility>(e).unwrap(),
        Visibility::Hidden,
        "a harvested node with no stump must stop being drawn"
    );
    let y = app.world().get::<Transform>(e).unwrap().translation.y;
    assert!((y - GROUND_Y).abs() < 1e-6, "and must not be lifted");

    run(&mut app, &a, &|_| false);
    assert_eq!(
        *app.world().get::<Visibility>(e).unwrap(),
        Visibility::Inherited,
        "and must come back when the slot respawns"
    );
}

/// The lift must be ABSOLUTE, not accumulated.
///
/// `y += lift` / `y -= lift` is correct only while every transition is
/// observed. Anything that despawns, re-seeds or teleports an entity without
/// resetting `Fellable::felled` leaves the flag disagreeing with what is
/// drawn, and an accumulating swap then walks the prop 0.17 m per missed
/// pair — up a hillside over a session, and invisibly, because each single
/// step looks right.
///
/// The fixture below is that disagreement made explicit: a slot whose flag
/// says felled while its transform sits at the un-felled height, which is
/// what a chunk streaming back in during a harvest produces.
#[test]
fn the_lift_is_absolute_so_a_missed_transition_cannot_drift_it() {
    let (mut app, a) = fixture();
    let e = app
        .world_mut()
        .spawn((
            Fellable {
                key: KEY,
                variant: VARIANT,
                base_y: GROUND_Y,
                stumps: true,
                // Stale: says felled, but the transform below is at the
                // un-felled height.
                felled: true,
            },
            Mesh3d(a.stump_mesh().clone()),
            MeshMaterial3d(a.wood_material().clone()),
            Transform {
                translation: Vec3::new(3.0, GROUND_Y, -7.0),
                scale: Vec3::splat(SCALE),
                ..default()
            },
            Visibility::Inherited,
        ))
        .id();

    run(&mut app, &a, &|_| false);

    let y = app.world().get::<Transform>(e).unwrap().translation.y;
    assert!(
        (y - GROUND_Y).abs() < 1e-5,
        "un-felling must SET the ground height, not subtract a lift that was \
         never added — got {y}, expected {GROUND_Y}"
    );
}
