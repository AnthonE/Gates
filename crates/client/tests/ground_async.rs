//! Gate: the ground streamer's move onto `AsyncComputeTaskPool`.
//!
//! `heightfield` is pure — it reads `sim_core::terrain`, touches no ECS and
//! allocates only what it returns — so moving it off the main thread changes
//! no vertex. What it changes is the SHAPE of `terrain_mesh::stream`, and
//! `tests/ground.rs` cannot see any of that: it calls `heightfield` directly
//! and never runs a system. Two things went from true-by-construction to
//! claims the moment the build stopped finishing inside the statement that
//! started it, and both are the kind that produce no error:
//!
//!  1. **`far_done` was set before the build.** It is what `far_ready` reports
//!     to the loading bar, which is what ends the loading screen — so the flag
//!     that used to mean "the island is up" would have come to mean "the
//!     island is queued", and a player would be dropped into a world with no
//!     island in it. Split into `far_started` (guards the spawn) and
//!     `far_done` (set when the mesh reaches the world).
//!  2. **`built` was the only test for "is this chunk handled"**, and it is
//!     written when the mesh exists. An async build leaves a window in which a
//!     key is in neither map, and the ring loop would queue a fresh task for
//!     the same chunk on every frame of it — a task storm that grows with how
//!     long the pool takes, which is exactly when it can least afford one.
//!
//! No window, no GPU, no socket: `MinimalPlugins` plus the asset plugin, which
//! is `tests/music.rs`'s posture. `MinimalPlugins` carries `TaskPoolPlugin`,
//! so the compute pool this depends on is real rather than stubbed.
//!
//! Proven red: setting `far_done` where `far_started` is set fails claim 1;
//! dropping the `near_tasks.contains_key` half of the queue guard fails
//! claim 2.

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use client::render::ground_splat::GroundMaterial;
use client::render::terrain_mesh::{
    self, Chunk, Ring, Static, CHUNK_BUILDS_PER_FRAME, RING_CHUNKS,
};
use client::render::textures::GroundArrays;
use client::render::{Eye, WorldId};

/// The seed the shard ships and every capture is shot on.
const SEED: u64 = 20260731;

fn arrays() -> GroundArrays {
    // Handles, never files: `stream` puts them in a material and nothing here
    // renders, so a default handle is as good as a stacked array and costs no
    // asset server round trip. Present from the first frame, which is what
    // makes every claim below about the MESHES: `stream` lands nothing until
    // this resource exists (`tests/ground_arrays.rs` is where that wait is
    // gated), so a fixture without it would measure the wait and not the ring.
    GroundArrays {
        albedo: Handle::default(),
        normal: Handle::default(),
        rough_ao: Handle::default(),
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<GroundMaterial>();
    app.insert_resource(WorldId::new(SEED));
    app.insert_resource(arrays());
    app.init_resource::<Ring>();
    app.insert_resource(Eye {
        pos: Vec3::new(1024.0, 10.0, 1024.0),
        ..default()
    });
    app.add_systems(Update, terrain_mesh::stream);
    app
}

/// A runaway backstop, NOT a timeout.
///
/// `CLAUDE.md` is explicit that a gate which waits on elapsed milliseconds is
/// not a gate on this box — and the corollary bit here on the first draft,
/// which used 4,000 frames. A frame in this app is one system polling a task:
/// four thousand of them are ~80 ms of spinning, against a far mesh that takes
/// the better part of a second to build in the DEBUG profile `ci/gates.sh`
/// runs the renderer tier in. The budget was not measuring anything; it was a
/// timeout wearing a count's clothes, and it would have gone red on the gate
/// box and nowhere else.
///
/// So this number is deliberately three orders of magnitude past anything real
/// — it exists to turn "the task never completes" into a failure instead of a
/// hang. The assertion is always [`step_until`]'s predicate.
const RUNAWAY_FRAMES: usize = 3_000_000;

/// Step until `f` holds, yielding to the pool between frames.
///
/// The yield is what stops this thread from spinning against the pool thread
/// doing the work — a scheduler hint, not a clock, and nothing here asserts
/// how long anything took.
fn step_until(app: &mut App, mut f: impl FnMut(&App) -> bool) -> usize {
    for i in 0..RUNAWAY_FRAMES {
        if f(app) {
            return i;
        }
        app.update();
        std::thread::yield_now();
    }
    panic!(
        "the condition never held in {RUNAWAY_FRAMES} frames — the build is not \
         finishing at all, which is a different failure from a slow one"
    );
}

// ── 1. The loading bar is told the truth ───────────────────────────────────

#[test]
fn the_far_mesh_is_not_ready_until_it_is_in_the_world() {
    let mut app = app();
    // The first frame queues it and cannot possibly have finished it.
    app.update();
    assert!(
        !app.world().resource::<Ring>().far_ready(),
        "far_ready() is true on the frame the build was QUEUED — that flag ends \
         the loading screen, so this is a player dropped into an island-less world"
    );

    // Every frame until it lands, `far_ready()` and "a Static entity exists"
    // agree. That is the claim, not "it eventually becomes true": a flag that
    // ran ahead of the entity by even one frame is the defect.
    let mut frames = 0usize;
    loop {
        let ready = app.world().resource::<Ring>().far_ready();
        let drawn = app
            .world_mut()
            .query_filtered::<Entity, With<Static>>()
            .iter(app.world())
            .count();
        assert_eq!(
            ready,
            drawn == 1,
            "frame {frames}: far_ready() is {ready} while {drawn} far meshes exist"
        );
        if ready {
            break;
        }
        frames += 1;
        assert!(
            frames < RUNAWAY_FRAMES,
            "the far mesh never landed in {frames} frames"
        );
        app.update();
        std::thread::yield_now();
    }
    assert!(
        frames > 0,
        "the far mesh landed on the frame it was queued — it is not off the \
         main thread at all"
    );
}

// ── 2. One task per chunk, ever ────────────────────────────────────────────

#[test]
fn a_chunk_is_never_queued_twice() {
    let mut app = app();
    // Let the far mesh clear first: `stream` queues it on the first frame and
    // the near ring is what this test is about.
    step_until(&mut app, |a| a.world().resource::<Ring>().far_ready());

    // The ring fills. `CHUNK_BUILDS_PER_FRAME` queues and `CHUNK_LANDS_PER_FRAME`
    // lands, so the floor on frames is the chunk count — the ceiling is what
    // says nothing is being re-queued.
    let mut frames = 0usize;
    let mut peak_in_flight = 0usize;
    while app.world().resource::<Ring>().len() < RING_CHUNKS {
        app.update();
        frames += 1;
        peak_in_flight = peak_in_flight.max(app.world().resource::<Ring>().in_flight());
        std::thread::yield_now();
        assert!(
            frames < RUNAWAY_FRAMES,
            "the near ring never filled: {} of {RING_CHUNKS} after {frames} frames",
            app.world().resource::<Ring>().len()
        );
    }

    // The bound that catches the storm. Queueing is rationed at
    // `CHUNK_BUILDS_PER_FRAME` a frame, so a loop that re-queued a chunk it had
    // already handed to the pool would still only add one per frame — but it
    // would never RETIRE them, so the in-flight set would climb past the ring
    // itself. It cannot exceed the ring: every chunk is queued at most once.
    assert!(
        peak_in_flight <= RING_CHUNKS,
        "{peak_in_flight} builds were in flight at once against a {RING_CHUNKS}-chunk \
         ring — a chunk is being queued more than once"
    );
    assert_eq!(
        app.world().resource::<Ring>().in_flight(),
        0,
        "the ring is full and builds are still in flight"
    );

    // And every chunk is drawn exactly once.
    let drawn = app
        .world_mut()
        .query_filtered::<Entity, With<Chunk>>()
        .iter(app.world())
        .count();
    assert_eq!(
        drawn, RING_CHUNKS,
        "{drawn} chunk entities for a {RING_CHUNKS}-chunk ring"
    );

    // Standing still costs nothing more. A full ring that keeps queueing is the
    // same defect wearing a steady state.
    for _ in 0..30 {
        app.update();
    }
    assert_eq!(app.world().resource::<Ring>().in_flight(), 0);
    let drawn = app
        .world_mut()
        .query_filtered::<Entity, With<Chunk>>()
        .iter(app.world())
        .count();
    assert_eq!(
        drawn, RING_CHUNKS,
        "a settled ring is still spawning chunks"
    );
}

/// The queue budget is still a budget. It bounds how fast work is HANDED to
/// the pool, which is a different claim from the one it used to make ("how
/// much of the frame it costs"), and the constant's own doc comment now says
/// so — this is that sentence as a gate.
///
/// Counted as a DELTA from wherever the app already is, because the far mesh
/// takes hundreds of frames on this box and the near ring is queued through
/// every one of them: a test that waited for the far mesh and then started
/// counting from zero would read the whole ring as one frame's work. That is
/// the first draft of this test, and it failed reporting 25 against a budget
/// of 1 — a true statement about a premise nobody had checked.
#[test]
fn the_ring_queues_at_most_its_budget_a_frame() {
    let mut app = app();
    let handled = |a: &App| {
        let r = a.world().resource::<Ring>();
        r.len() + r.in_flight()
    };
    let mut prev = handled(&app);
    for _ in 0..RING_CHUNKS + 8 {
        app.update();
        let now = handled(&app);
        assert!(
            now <= prev + CHUNK_BUILDS_PER_FRAME + 1,
            "{now} chunks handled after a frame that started at {prev} — the queue \
             budget is {CHUNK_BUILDS_PER_FRAME} (+1 for the far mesh's own slot)"
        );
        prev = now;
    }
}

/// A chunk that leaves the ring before its build finishes is cancelled rather
/// than landed. Dropping a `Task` cancels it, and `retain` is the whole
/// teardown — without it a player walking a straight line accumulates one dead
/// task per chunk crossed, each still holding the pool.
#[test]
fn a_chunk_that_leaves_the_ring_takes_its_build_with_it() {
    let mut app = app();
    // A few frames, deliberately WITHOUT waiting for the far mesh: by the time
    // it lands the near ring is already full, and a full ring has nothing in
    // flight to cancel.
    let before = step_until(&mut app, |a| a.world().resource::<Ring>().in_flight() >= 4);
    assert!(before > 0);
    let in_flight = app.world().resource::<Ring>().in_flight();
    assert!(
        in_flight >= 4,
        "only {in_flight} builds in flight to cancel"
    );

    // Walk far enough that no chunk of the old ring is in the new one.
    app.world_mut().resource_mut::<Eye>().pos.x += terrain_mesh::CHUNK_M * 16.0;
    app.update();

    // What may survive: the far mesh, which is the whole island and is not the
    // ring's to cancel, plus the one chunk this frame queued.
    let after = app.world().resource::<Ring>().in_flight();
    assert!(
        after <= CHUNK_BUILDS_PER_FRAME + 1,
        "{after} builds still in flight after the ring moved away from all {in_flight} \
         of them — a task per chunk crossed is a pool that fills up as the player walks"
    );
}

#[test]
fn far_shadows_follow_resident_chunks_through_loading_walking_and_teleporting() {
    use bevy::ecs::message::MessageCursor;
    use bevy::image::ImageSampler;
    use bevy::light::NotShadowCaster;
    use bevy::render::render_resource::TextureFormat;
    use sim_core::terrain::ISLAND_SIZE;

    let mut app = app();
    let mut chunks = app
        .world_mut()
        .query::<(&Chunk, &MeshMaterial3d<GroundMaterial>)>();
    let mut far = app
        .world_mut()
        .query_filtered::<(&MeshMaterial3d<GroundMaterial>, Has<NotShadowCaster>), With<Static>>();
    let side = (ISLAND_SIZE / terrain_mesh::CHUNK_M) as usize;
    let mut last_keys = Vec::new();
    let mut last_mask = Vec::new();
    let mut checked = 0;
    let mut material_events = MessageCursor::<AssetEvent<GroundMaterial>>::default();

    // Includes a walk across a chunk boundary, a disjoint ring, and the
    // island's opposite edges: pending chunks and budgeted teardown must not punch
    // holes in the far shadows before their replacements actually exist.
    for (cx, cz) in [(16, 16), (17, 16), (24, 24), (31, 31), (0, 0)] {
        app.world_mut().resource_mut::<Eye>().pos = Vec3::new(
            cx as f32 * terrain_mesh::CHUNK_M,
            10.0,
            cz as f32 * terrain_mesh::CHUNK_M,
        );
        app.update();
        step_until(&mut app, |a| {
            let world = a.world();
            let materials = world.resource::<Assets<GroundMaterial>>();
            let Some((masked_id, masked)) = materials
                .iter()
                .find(|(_, m)| m.base.base_color_texture.is_some())
            else {
                panic!("the far mesh has no shadow mask");
            };
            assert_eq!(masked.base.alpha_mode, AlphaMode::Mask(0.5));
            let mask = world
                .resource::<Assets<Image>>()
                .get(masked.base.base_color_texture.as_ref().unwrap())
                .unwrap();
            assert_eq!((mask.width(), mask.height()), (side as u32, side as u32));
            assert_eq!(mask.texture_descriptor.format, TextureFormat::Rgba8Unorm);
            assert_eq!(mask.texture_descriptor.mip_level_count, 1);
            assert_eq!(mask.sampler, ImageSampler::nearest());
            let data = mask.data.as_ref().unwrap();
            let refreshed = material_events
                .read(world.resource::<Messages<AssetEvent<GroundMaterial>>>())
                .any(|event| {
                    matches!(event,
                    AssetEvent::Added { id } | AssetEvent::Modified { id }
                        if *id == masked_id)
                });
            if *data != last_mask {
                assert!(
                    refreshed,
                    "a changed mask must refresh the GPU material binding"
                );
            }
            let mut keys: Vec<_> = chunks
                .iter(world)
                .map(|(chunk, handle)| {
                    let material = materials.get(&handle.0).unwrap();
                    assert_eq!(material.base.alpha_mode, AlphaMode::Opaque);
                    assert!(material.base.base_color_texture.is_none());
                    (chunk.0, chunk.1)
                })
                .collect();
            keys.sort_unstable();
            if keys != last_keys || *data != last_mask {
                for z in 0..side {
                    for x in 0..side {
                        let expected = if keys.contains(&(x as i32, z as i32)) {
                            0
                        } else {
                            255
                        };
                        assert_eq!(
                            data[(z * side + x) * 4 + 3],
                            expected,
                            "far shadow at ({x}, {z}) must follow resident chunks {keys:?}"
                        );
                        // Derive the shader's lookup from world metres, not
                        // from the mask writer's indexing expression.
                        let uv = Vec2::new(x as f32 + 0.5, z as f32 + 0.5)
                            * terrain_mesh::CHUNK_M
                            * terrain_mesh::UV_PER_M;
                        let mapped = masked.base.uv_transform.transform_point2(uv) * side as f32;
                        assert_eq!((mapped.x as usize, mapped.y as usize), (x, z));
                    }
                }
                last_keys.clone_from(&keys);
                last_mask.clone_from(data);
                checked += 1;
            }
            for (handle, disabled) in far.iter(world) {
                assert!(!disabled, "the distant terrain must still cast shadows");
                assert_eq!(
                    materials.get(&handle.0).unwrap().base.base_color_texture,
                    masked.base.base_color_texture
                );
            }
            world.resource::<Ring>().is_full()
                && world.resource::<Ring>().in_flight() == 0
                && keys.iter().all(|&(x, z)| {
                    (x - cx).abs() <= terrain_mesh::NEAR_RADIUS
                        && (z - cz).abs() <= terrain_mesh::NEAR_RADIUS
                })
        });
    }
    assert!(
        checked >= RING_CHUNKS,
        "the test must see streaming transitions"
    );
    let mut image_events = MessageCursor::<AssetEvent<Image>>::default();
    for _ in 0..3 {
        material_events
            .read(
                app.world()
                    .resource::<Messages<AssetEvent<GroundMaterial>>>(),
            )
            .for_each(drop);
        image_events
            .read(app.world().resource::<Messages<AssetEvent<Image>>>())
            .for_each(drop);
        app.update();
        assert!(!material_events
            .read(
                app.world()
                    .resource::<Messages<AssetEvent<GroundMaterial>>>()
            )
            .any(|e| matches!(e, AssetEvent::Modified { .. })));
        assert!(!image_events
            .read(app.world().resource::<Messages<AssetEvent<Image>>>())
            .any(|e| matches!(e, AssetEvent::Modified { .. })));
    }
}
