//! Gate: a chopped tree topples, stays down, and stands back up on respawn.
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
//! contract: the stump appears, the trunk and its canopy go down together on
//! one bearing, the mesh never changes, and every one of those reverses on
//! respawn. What it deliberately does NOT claim is that a real chop arrives —
//! that is `ClientCore`'s `EventMsg::SlotHarvested` path, which has its own
//! coverage.
//!
//! **`RENDER.md`'s framing is why the curve is in here at all.** There is no
//! visual gate in this repo and there must not be one (`CLAUDE.md`); what may
//! be gated about a frame is *arithmetic*. A fall is an angle over time, an
//! angle is arithmetic, and `fell_rotation` is split out of the system so it
//! can be driven at both ends and in the middle without a window.
//!
//! No window, no GPU, no socket: `MinimalPlugins` plus the asset plugin.

use std::time::Duration;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::props::{
    apply_fell, assets, fall, fell_bearing, fell_rotation, FellPart, Fellable, PropAssets, Topple,
    FELL_FALL_S, STUMP_LIFT_M,
};
use client::render::textures::{MapSet, PropMaps};
use client::render::tree::CONIFER_POOL;

/// The variant this fixture's tree spawned as. **Deliberately not variant 0
/// and deliberately not `KEY % variants`**: the first cut of `apply_fell`
/// re-derived the variant from the key on the un-fell branch while
/// `spawn_slot` picked it from the slot's yaw, so a respawned tree came back
/// as a different tree.
///
/// That bug is now structurally impossible rather than merely tested against
/// — a toppling trunk keeps the mesh it already had, so `apply_fell` touches
/// no mesh handle on any path. The pinning stays because the *accessor* it
/// guards (`pine_mesh`) is still what `spawn_slot` and the respawn path read,
/// and because a gate that quietly stops being able to fail is the worst bug
/// class in `CLAUDE.md`'s list.
///
/// **Computed, not written down.** It was the literal `3` against a pool of 4;
/// generating the conifer took the pool to 3, which made it both an
/// out-of-range index and — at the next value down — exactly `KEY % 3`, so the
/// premise quietly stopped holding.
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
    let mut images = world.remove_resource::<Assets<Image>>().unwrap();

    // The prop maps are handles, never resolved: this fixture has no asset
    // files and does not need them. `assets` only clones them into materials,
    // so a default handle exercises exactly the same code path a loaded one
    // would — and keeps this gate in the tier that runs without a GPU or a
    // filesystem, which is the whole point of it.
    let maps = PropMaps {
        rock: MapSet::default(),
        bark: MapSet::default(),
        wood: MapSet::default(),
        stone: MapSet::default(),
        metal: MapSet::default(),
    };
    let a = assets(&mut meshes, &mut materials, &mut images, &maps);
    world.insert_resource(meshes);
    world.insert_resource(materials);
    world.insert_resource(images);
    (app, a)
}

const KEY: u32 = (137u32 << 16) | 42;
const GROUND_Y: f32 = 12.5;
const SCALE: f32 = 1.05;
/// The yaw the fixture's tree stands at. Non-zero on purpose: the fall
/// COMPOSES onto the yaw, so a fixture at yaw 0 cannot tell a correct
/// composition from one that discards it.
const YAW: f32 = 0.9;

fn part(app: &mut App, a: &PropAssets, part: FellPart) -> Entity {
    let (mesh, vis) = match part {
        FellPart::Stump => (a.stump_mesh().clone(), Visibility::Hidden),
        FellPart::Canopy => (a.needle_mesh(VARIANT).clone(), Visibility::Inherited),
        _ => (a.pine_mesh(VARIANT).clone(), Visibility::Inherited),
    };
    let mut ent = app.world_mut().spawn((
        Fellable {
            key: KEY,
            variant: VARIANT,
            base_y: GROUND_Y,
            yaw: YAW,
            part,
            felled: false,
        },
        Mesh3d(mesh),
        MeshMaterial3d(a.bark_material(KEY).clone()),
        Transform {
            // The stump sits `STUMP_LIFT_M` above the slot's ground because
            // its mesh is centred on its own axis while the slot's `y` is the
            // surface — and the lift scales with the instance, exactly as
            // `spawn_slot` writes it. Every other part is authored with its
            // base at y = 0 and needs no lift.
            translation: Vec3::new(
                3.0,
                GROUND_Y
                    + if matches!(part, FellPart::Stump) {
                        STUMP_LIFT_M * SCALE
                    } else {
                        0.0
                    },
                -7.0,
            ),
            rotation: Quat::from_rotation_y(YAW),
            scale: Vec3::splat(SCALE),
        },
        // Explicit, because the query matches on it. In the client Bevy
        // inserts it as a required component of `Mesh3d`; a fixture that
        // omitted it did not match the query at all, and the respawn test
        // then passed VACUOUSLY — it asserted a pine was drawn on an
        // entity nothing had ever touched.
        vis,
    ));
    // Exactly what `spawn_slot` does: the two parts that move carry the
    // state for moving, and nothing else does.
    if matches!(part, FellPart::Trunk | FellPart::Canopy) {
        ent.insert(Topple { t: -1.0 });
    }
    ent.id()
}

/// Run the discrete half — the state change `harvest` drives.
fn run(app: &mut App, harvested: &dyn Fn(u32) -> bool) {
    let world = app.world_mut();
    let mut state = world.query::<(
        &mut Fellable,
        Option<&mut Topple>,
        &mut Transform,
        &mut Visibility,
    )>();
    apply_fell(state.query_mut(world), harvested);
}

/// Run the continuous half — the real [`fall`] system, on a clock this test
/// owns. Driving the shipped system rather than re-deriving the pose here is
/// the point: a correct curve wired to nothing is the failure a hand-rolled
/// assertion would miss.
fn tick(app: &mut App, dt: f32) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(dt));
    app.world_mut().run_system_cached(fall).unwrap();
}

/// How far off vertical this entity's trunk axis is, radians. The measurement
/// the whole slice is about, taken off the shipped transform.
fn tilt(app: &App, e: Entity) -> f32 {
    let up = app.world().get::<Transform>(e).unwrap().rotation * Vec3::Y;
    up.angle_between(Vec3::Y)
}

#[test]
fn a_chopped_tree_topples_and_stays_down() {
    let (mut app, a) = fixture();
    let trunk = part(&mut app, &a, FellPart::Trunk);
    let mesh_before = app.world().get::<Mesh3d>(trunk).unwrap().0.clone();

    // Nothing harvested: upright, and the clock does not move it.
    run(&mut app, &|_| false);
    tick(&mut app, 0.5);
    assert!(tilt(&app, trunk) < 1e-4, "a standing tree must not lean");
    assert!(!app.world().get::<Fellable>(trunk).unwrap().felled);

    // Chopped. The topple is armed but has not been integrated yet, so the
    // tree is still upright on this frame — the fall is a motion, not a jump.
    run(&mut app, &|k| k == KEY);
    assert!(app.world().get::<Fellable>(trunk).unwrap().felled);
    assert!(
        tilt(&app, trunk) < 1e-4,
        "the cut arms the fall; it must not teleport the trunk flat, which is \
         the pop this slice exists to remove"
    );

    // Halfway through the duration it is leaning and not yet down. Both
    // bounds matter: `> 0` catches a fall that never starts, `< 90°` catches
    // one that snaps to the end on its first integration.
    tick(&mut app, FELL_FALL_S * 0.5);
    let mid = tilt(&app, trunk);
    assert!(
        mid > 0.05 && mid < std::f32::consts::FRAC_PI_2 - 0.05,
        "half a duration in, the trunk must be mid-fall — got {mid} rad"
    );

    // Past the duration it is flat, and stays flat however long the clock runs.
    tick(&mut app, FELL_FALL_S);
    let down = tilt(&app, trunk);
    assert!(
        (down - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
        "a felled trunk must come to rest exactly flat — got {down} rad"
    );
    tick(&mut app, 60.0);
    assert!(
        (tilt(&app, trunk) - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
        "the trunk must STAY down — a log that keeps rotating past flat is a \
         clamp that was never applied"
    );

    // **The invariant that makes persistence free.** A downed tree is the same
    // mesh lying down, so a felled forest costs exactly what a standing one
    // costs. If this ever swaps, the triangle argument in `DECISIONS.md`
    // felling v0 stops being true and the log has to earn its budget again.
    assert_eq!(
        app.world().get::<Mesh3d>(trunk).unwrap().0,
        mesh_before,
        "felling must not swap the trunk's mesh"
    );
}

#[test]
fn the_canopy_rides_the_trunk_down() {
    let (mut app, a) = fixture();
    let trunk = part(&mut app, &a, FellPart::Trunk);
    let canopy = part(&mut app, &a, FellPart::Canopy);

    run(&mut app, &|k| k == KEY);
    // One integration for both, exactly as the client does it: one system,
    // one `delta_secs`, two entities.
    tick(&mut app, FELL_FALL_S * 0.4);

    let (rt, rc) = {
        let w = app.world();
        (
            w.get::<Transform>(trunk).unwrap().rotation,
            w.get::<Transform>(canopy).unwrap().rotation,
        )
    };
    // Compared through the axis they both rotate, **not** with
    // `Quat::angle_between`, which is `acos` of a dot product and returns NaN
    // when the two are identical and the dot rounds a hair above 1.0 — the
    // exact case this assertion is about. It bit: the first cut of this test
    // used `angle_between` and went red under an unrelated mutation because
    // the quats agreed *too* exactly. A comparison that fails when its
    // premise holds perfectly is not a comparison.
    assert!(
        (rt * Vec3::Y).distance(rc * Vec3::Y) < 1e-6,
        "the canopy is a SIBLING of the trunk carrying its own transform, not \
         a child — if the two ever integrate differently the needles tear off \
         the tree mid-fall, and nothing but a screenshot would say so"
    );
    assert!(tilt(&app, canopy) > 0.05, "the canopy must actually move");
}

#[test]
fn the_stump_appears_with_the_cut_and_hides_on_respawn() {
    let (mut app, a) = fixture();
    let stump = part(&mut app, &a, FellPart::Stump);

    assert_eq!(
        *app.world().get::<Visibility>(stump).unwrap(),
        Visibility::Hidden,
        "a stump under a standing tree must not be drawn"
    );

    run(&mut app, &|k| k == KEY);
    assert_eq!(
        *app.world().get::<Visibility>(stump).unwrap(),
        Visibility::Inherited,
        "the stump is what the cut leaves; it appears with the cut rather than \
         when the trunk lands"
    );

    run(&mut app, &|_| false);
    assert_eq!(
        *app.world().get::<Visibility>(stump).unwrap(),
        Visibility::Hidden,
        "a respawned slot has no stump"
    );

    // **The lift, which no longer has a swap to prove it.** While felling was
    // a mesh swap, `apply_fell` raised the trunk entity by `STUMP_LIFT_M` and
    // this gate asserted the arithmetic on the way through. Nothing swaps now
    // — `spawn_slot` places a stump entity already lifted — so the constant
    // would have gone uncovered, and the first cut of this file "kept" it with
    // `assert!(STUMP_LIFT_M > 0.0)`. Clippy refused that as an assertion with
    // a constant value, and it was right for a better reason than it knew: an
    // assertion that cannot fail is not coverage. This is the real one.
    let y = app.world().get::<Transform>(stump).unwrap().translation.y;
    assert!(
        (y - (GROUND_Y + STUMP_LIFT_M * SCALE)).abs() < 1e-5,
        "a stump stands half its own height above the slot's ground, scaled \
         with the instance — got {y}"
    );
    // And it does not move when the tree comes down on top of it.
    run(&mut app, &|k| k == KEY);
    tick(&mut app, FELL_FALL_S * 2.0);
    let after = app.world().get::<Transform>(stump).unwrap().translation.y;
    assert!(
        (after - y).abs() < 1e-6,
        "the stump is not part of the topple — it must not be animated"
    );
}

#[test]
fn a_respawned_tree_stands_back_up_as_the_same_tree() {
    let (mut app, a) = fixture();
    let trunk = part(&mut app, &a, FellPart::Trunk);

    run(&mut app, &|k| k == KEY);
    tick(&mut app, FELL_FALL_S * 2.0);
    // Precondition. Without it this test passes on an entity the system never
    // touched, which is exactly how it passed while the fixture was missing a
    // component the query needed.
    assert!(
        tilt(&app, trunk) > 1.0,
        "precondition: the tree must be down before respawn is tested"
    );

    run(&mut app, &|_| false);
    assert!(
        tilt(&app, trunk) < 1e-4,
        "a respawned tree must stand back up"
    );
    let rot = app.world().get::<Transform>(trunk).unwrap().rotation;
    assert!(
        rot.angle_between(Quat::from_rotation_y(YAW)) < 1e-5,
        "it must stand up at the yaw it was SPAWNED at — restoring identity \
         instead would turn every respawn into a silent re-facing"
    );

    // And the next chop starts from the top of the curve rather than from
    // wherever the last one ended.
    run(&mut app, &|k| k == KEY);
    tick(&mut app, FELL_FALL_S * 0.5);
    let mid = tilt(&app, trunk);
    assert!(
        mid > 0.05 && mid < std::f32::consts::FRAC_PI_2 - 0.05,
        "a second chop must fall like the first — got {mid} rad"
    );

    assert_eq!(
        app.world().get::<Mesh3d>(trunk).unwrap().0,
        *a.pine_mesh(VARIANT),
        "a respawned slot must draw THE SAME tree again"
    );
    assert_ne!(
        *a.pine_mesh(VARIANT),
        *a.pine_mesh(KEY as usize),
        "the fixture's variant must not be the one a key-derived bug would \
         also produce, or this test agrees with the bug"
    );
}

/// **The topple must not look like a state change, because something listens
/// for state changes.**
///
/// `audio::fell` fires the tree-fall cue off `Ref<Fellable>::is_changed()`.
/// The first cut of this slice kept `fall_t` ON `Fellable`, so [`fall`] took
/// `&mut Fellable` and marked it changed on every frame of a ~96-frame topple
/// — one chop would have played the cue ninety-six times. Nothing else in
/// this repo would have caught it: every other assertion about felling is
/// about geometry, and this defect is audible.
///
/// So the animating field lives on [`Topple`] and this pins that split. The
/// general rule, which is `CLAUDE.md`'s silent-merge trap one turn on: **a
/// component something else change-detects may not carry a per-frame field.**
/// **A named fn, never a closure — and this cost a red run to learn.**
/// `run_system_cached` keys on the system's TYPE, and two textually identical
/// closures are two distinct types. Calling it with a fresh closure each time
/// registers a fresh system whose `last_run` is zero, so the second call sees
/// every change since the world began — including the transition the first
/// call was supposed to have consumed. One fn item is one cached system with
/// one advancing `last_run`, which is what change detection needs to mean
/// anything here.
fn count_changed(q: Query<Entity, Changed<Fellable>>) -> usize {
    q.iter().count()
}

#[test]
fn falling_does_not_mark_the_slot_changed() {
    let (mut app, a) = fixture();
    let trunk = part(&mut app, &a, FellPart::Trunk);

    run(&mut app, &|k| k == KEY);
    // Consume the transition's own change flag, which is legitimate — the slot
    // really did change state on that frame. Everything after is the topple.
    let at_cut = app.world_mut().run_system_cached(count_changed).unwrap();
    assert_eq!(
        at_cut, 1,
        "precondition: the CUT itself must mark the slot changed, or the \
         tree-fall cue would never fire at all and this gate would pass by \
         asserting nothing"
    );

    for _ in 0..8 {
        tick(&mut app, FELL_FALL_S / 16.0);
    }
    // Precondition: it really is mid-fall, or this asserts nothing.
    let mid = tilt(&app, trunk);
    assert!(mid > 0.01, "precondition: the trunk must be moving");

    let changed = app.world_mut().run_system_cached(count_changed).unwrap();
    assert_eq!(
        changed, 0,
        "a tree in mid-topple must not re-fire `Changed<Fellable>` — that is \
         the tree-fall cue playing once per frame for the length of the fall"
    );
}

/// The curve itself, without a world. Three points and a direction.
#[test]
fn the_fall_curve_starts_upright_ends_flat_and_only_goes_one_way() {
    let b = fell_bearing(KEY);
    let up = |t: f32| (fell_rotation(YAW, b, t) * Vec3::Y).angle_between(Vec3::Y);

    assert!(up(0.0) < 1e-6, "t=0 is standing");
    assert!(
        (up(FELL_FALL_S) - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "the duration is when it is flat, exactly"
    );
    assert!(
        (up(FELL_FALL_S * 4.0) - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "past the duration it is clamped, not continuing"
    );

    // Monotonic, and accelerating: a tree pivots slowest off the stump. The
    // second assertion is what makes the curve `p²` rather than linear, and a
    // linear curve would pass the first one.
    let mut prev = 0.0;
    for i in 1..=32 {
        let a = up(FELL_FALL_S * i as f32 / 32.0);
        assert!(a > prev, "the fall must never reverse");
        prev = a;
    }
    // **The margin is the assertion.** Written as a bare `<` against
    // `FRAC_PI_2 * 0.5` this passes on a LINEAR curve, because linear hits
    // that value exactly and float rounding lands it a hair under — proven by
    // mutating `p * p` to `p` and watching this test stay green. `p²` puts
    // half the duration at a QUARTER of the sweep, so the honest bound is the
    // midpoint between the two curves, and a linear fall now fails here.
    let half = up(FELL_FALL_S * 0.5);
    assert!(
        half < std::f32::consts::FRAC_PI_2 * 0.375,
        "at half the duration a falling tree is a quarter of the way over, not \
         half — an even sweep reads as a barrier arm, not as gravity. Got {half}"
    );

    // It falls TOWARD its bearing: at rest the trunk axis lies along it.
    let flat = fell_rotation(YAW, b, FELL_FALL_S) * Vec3::Y;
    let want = Vec3::new(b.sin(), 0.0, b.cos());
    assert!(
        flat.distance(want) < 1e-5,
        "the trunk must come to rest along its own bearing, got {flat:?} want {want:?}"
    );
}

/// Two entities and two clients have to agree on which way a tree goes down,
/// and nothing carries the answer between them.
#[test]
fn the_fall_bearing_is_a_pure_function_of_the_cell_key() {
    assert_eq!(
        fell_bearing(KEY),
        fell_bearing(KEY),
        "the bearing is what the trunk and the canopy each derive independently"
    );
    // Not all one direction. A hash that collapsed would give a forest that
    // falls like dominoes, and every assertion above would still pass.
    let spread: Vec<f32> = (0..64).map(|k| fell_bearing(k * 7919)).collect();
    let lo = spread.iter().cloned().fold(f32::MAX, f32::min);
    let hi = spread.iter().cloned().fold(f32::MIN, f32::max);
    assert!(
        hi - lo > 5.0,
        "64 keys must not all fall the same way — spread was {lo}..{hi}"
    );
    for b in &spread {
        assert!((0.0..=std::f32::consts::TAU).contains(b));
    }
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
    let e = part(&mut app, &a, FellPart::Vanish);

    run(&mut app, &|k| k == KEY);
    assert_eq!(
        *app.world().get::<Visibility>(e).unwrap(),
        Visibility::Hidden,
        "a harvested node must stop being drawn"
    );
    // And it does not try to topple: a boulder has no bearing.
    tick(&mut app, FELL_FALL_S);
    assert!(
        tilt(&app, e) < 1e-6,
        "a `Vanish` slot must not be animated — it has no trunk to pivot about"
    );

    run(&mut app, &|_| false);
    assert_eq!(
        *app.world().get::<Visibility>(e).unwrap(),
        Visibility::Inherited,
        "a respawned node comes back"
    );
}
