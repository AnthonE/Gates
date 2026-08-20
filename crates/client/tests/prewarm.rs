//! Gate: every material gets one tiny draw before the thing that wears it
//! appears, and the draw goes away again.
//!
//! Two failure modes, and the second is the expensive one. A prewarm that
//! misses a material costs a pop the first time that material is used — the
//! defect it exists to remove, invisible to every other gate. A prewarm that
//! never retires its entities costs a draw call **per material, forever**,
//! which is strictly worse than not having one: `MeshMaterial3d` on a live
//! entity is extracted, batched and drawn every frame whether it covers a
//! pixel or not.
//!
//! No window and no GPU. What is being checked is what reaches the ECS and
//! what the arithmetic says the draw's size is — the pipeline itself needs a
//! GPU to observe (`PipelineCache::pipelines()` lives in the render world),
//! and no GPU has ever run this client.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use client::render::prewarm::{
    retire, setup, warm, WarmMesh, Warming, WARM_AHEAD_M, WARM_FRAMES, WARM_SIZE_M,
};
use client::render::rig::EyeCam;
use client::render::settings::FOV_MIN_DEG;

fn fixture() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.add_systems(Startup, setup);
    app.add_systems(Update, (warm, retire.after(warm)));
    app.world_mut().spawn(EyeCam);
    app
}

/// A material nobody named gets warmed anyway.
///
/// **The derived property, and the whole reason this is not a list.** A list
/// of materials-that-need-warming is the shape `CLAUDE.md` warns about twice
/// — the `props.js` citation count and the `pop_*` ring scrape — and both
/// drifted the moment somebody added one more. Here the material is created
/// by this test, which `render/prewarm.rs` has never heard of.
#[test]
fn a_material_nobody_listed_is_warmed() {
    let mut app = fixture();
    app.update();

    let handle = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.1, 0.2, 0.3),
            ..default()
        });
    // Asset events are sent in `PostUpdate` and `warm` reads them in
    // `Update`, so the warm draw appears on the frame after the material.
    app.update();
    app.update();

    let warmed: Vec<_> = app
        .world_mut()
        .query::<(&Warming, &MeshMaterial3d<StandardMaterial>)>()
        .iter(app.world())
        .map(|(_, m)| m.0.clone())
        .collect();
    assert!(
        warmed.contains(&handle),
        "a freshly created material got no warm draw — the first thing that \
         wears it will specialize its pipeline mid-play, which is the pop this \
         file exists to remove"
    );
}

/// The warm draw wears the layout the game's meshes wear.
///
/// A pipeline's key includes the mesh's vertex layout, so warming a material
/// against the wrong one specializes a pipeline the game will never ask for
/// and leaves the real one cold — a prewarm that is **all cost and no
/// benefit**, and green under every other assertion in this file.
#[test]
fn the_warm_mesh_matches_what_the_game_draws() {
    let mut app = fixture();
    app.update();
    let handle = app.world().resource::<WarmMesh>().0.clone();
    let meshes = app.world().resource::<Assets<Mesh>>();
    let mesh = meshes.get(&handle).expect("the warm mesh must exist");
    for (name, id) in [
        ("POSITION", Mesh::ATTRIBUTE_POSITION.id),
        ("NORMAL", Mesh::ATTRIBUTE_NORMAL.id),
        ("UV_0", Mesh::ATTRIBUTE_UV_0.id),
        ("TANGENT", Mesh::ATTRIBUTE_TANGENT.id),
        ("COLOR", Mesh::ATTRIBUTE_COLOR.id),
    ] {
        assert!(
            mesh.attribute(id).is_some(),
            "the warm mesh has no {name}, so it is a different vertex layout \
             than `Soup` emits for every prop, tile and tree — and therefore a \
             different pipeline than the one being warmed for"
        );
    }
}

/// …and it is too small to see.
///
/// **Computed rather than eyeballed.** The warm draw is a real, in-frustum,
/// depth-writing triangle sitting half a metre from the lens for eight
/// frames; if it were a pixel wide it would be a white speck flashing in the
/// middle of the screen every time a material was created. The angle it
/// subtends against the client's own field of view is arithmetic, so this is
/// the kind of visual claim that can be gated.
#[test]
fn the_warm_draw_is_under_a_pixel() {
    // The tallest common panel, which is the worst case: more pixels over the
    // same angle is a bigger speck. And the NARROWEST field of view a player
    // can choose, which is the other worst case — a narrow fov magnifies, so
    // the setting that makes this most visible is the bottom of the slider
    // rather than the default.
    const PIXELS_TALL: f32 = 2160.0;
    let half_fov = (FOV_MIN_DEG.to_radians() * 0.5).tan();
    let px = WARM_SIZE_M / (WARM_AHEAD_M * 2.0 * half_fov) * PIXELS_TALL;
    assert!(
        px < 0.1,
        "a warm draw covers {px:.3} px at {PIXELS_TALL:.0}p and {FOV_MIN_DEG}° — \
         it is in frustum and it writes depth, so anything approaching a pixel \
         is a speck the player sees"
    );
}

/// The warm draws go away, and not on the frame they arrive.
#[test]
fn a_warm_draw_lives_a_few_frames_and_then_leaves() {
    let mut app = fixture();
    app.update();
    // **Held, not dropped.** A material whose last handle goes out of scope
    // before `warm` reads the event is released, and `prewarm` skips it on
    // purpose — the first draft of this test dropped the handle and read the
    // resulting empty world as a broken prewarm.
    let _held = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());

    let count = |app: &mut App| {
        app.world_mut()
            .query::<&Warming>()
            .iter(app.world())
            .count()
    };

    app.update();
    app.update();
    assert_eq!(count(&mut app), 1, "the warm draw must exist at all");

    // It has to be QUEUED more than once — one frame is a single chance, and
    // a frame the renderer skipped would waste it.
    let mut alive = 1;
    for _ in 0..3 {
        app.update();
        alive += count(&mut app);
    }
    assert!(
        alive >= 4,
        "the warm draw survived {alive} of 4 frames — it must be queued \
         several times, not once"
    );

    // …and then it must go. A leaked warm entity is a draw call per material
    // for the rest of the session, which is worse than no prewarm at all.
    for _ in 0..WARM_FRAMES + 4 {
        app.update();
    }
    assert_eq!(
        count(&mut app),
        0,
        "a warm draw outlived WARM_FRAMES ({WARM_FRAMES}) — every material \
         ever created would keep drawing a triangle in front of the camera \
         forever"
    );
}

/// Before there is a camera there is no view to specialize against, and the
/// system must not spawn an orphan.
#[test]
fn nothing_is_warmed_without_an_eye() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<StandardMaterial>();
    app.add_systems(Startup, setup);
    app.add_systems(Update, (warm, retire.after(warm)));
    // No `EyeCam`.
    app.update();
    let _held = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.update();
    app.update();
    assert_eq!(
        app.world_mut()
            .query::<&Warming>()
            .iter(app.world())
            .count(),
        0,
        "a warm draw was spawned with no camera to be drawn by — it would be \
         an entity with no parent and no view, costing extraction and warming \
         nothing"
    );
}
