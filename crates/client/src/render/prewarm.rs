//! Draw every material once, tiny, before the thing that wears it appears.
//!
//! **The trap this closes is `CLAUDE.md`'s oldest rendering one**, and the
//! first thing to say is that its wording is browser-era and no longer
//! describes what happens here. It reads: *median fps hides shader-compile
//! stalls… a native pipeline compile is a bigger stall than a WebGL link*.
//! The mechanism is real and the SYMPTOM is not: `bevy_render`'s
//! `RenderPlugin::synchronous_pipeline_compilation` defaults to **false**, so
//! a pipeline is built on a task pool and a draw whose pipeline is not ready
//! yet is **skipped**, not waited for. Natively the failure is therefore a
//! *pop* — a barrel, a tracer, a bullet mark or another player's body arriving
//! a few frames after the thing that caused it — rather than a 700 ms hitch.
//! Which is better, and still worth removing, and worth writing down because
//! the two failures are diagnosed differently.
//!
//! ## What was already covered, and what was not
//!
//! Most of it, by accident of a good decision: `loading.rs` says the loading
//! screen shows the world as it streams in, and `world_running` includes
//! `Screen::Loading`, so terrain, water, clutter, trees and every scatter prop
//! are drawn — and therefore specialized — while the bar is still filling.
//! `decal.rs` then hand-rolled this idea for the one material that is never
//! drawn until somebody swings: a real in-frustum draw at `PREWARM_ALPHA` for
//! `PREWARM_FRAMES`.
//!
//! What was left is everything shaped like the decal and without its own
//! solution — the tracer pool, the build ghost, the interaction highlight, a
//! mob's hide, a held item. Each is a material built in a setup system and
//! first drawn minutes later, on an event, in the middle of a fight.
//!
//! ## Why this is derived rather than a list
//!
//! A hand-kept list of "materials that need warming" is the exact shape
//! `CLAUDE.md` warns about twice — the `props.js` citation count and the
//! `pop_*` ring scrape, both of which drifted the moment somebody added one
//! more. So this warms **every `StandardMaterial` the moment it is created**,
//! off `AssetEvent::Added`. A material added by a slice nobody has written yet
//! is covered without that slice knowing this file exists.
//!
//! ## Why a sub-pixel draw is enough
//!
//! Specialization happens when a draw is QUEUED, not when a fragment runs, so
//! the warm entity only has to survive frustum culling — it does not have to
//! cover a pixel. [`WARM_SIZE_M`] at [`WARM_AHEAD_M`] is under a tenth of a
//! pixel at 1080p and the game's field of view, which `tests/prewarm.rs`
//! computes rather than asserts by eye. `decal.rs` reaches the same place from
//! the other side (full size, `PREWARM_ALPHA` alpha) because a projected decal
//! has no meaningful scale to shrink.
//!
//! ## The two residuals, both named because neither is covered
//!
//! **Skinned meshes are a different pipeline key** and this warms none of
//! them: a body's skin is a `SkinnedMesh` component, so the first remote
//! player to walk into view still specializes on arrival. **And a driver may
//! finalize a shader on first real use** whatever a queue did — the pipeline
//! is warm, the driver's copy of it may not be. Neither is measurable on this
//! box: `PipelineCache::pipelines()` is public, but it lives in the render
//! world and needs a GPU to say anything, and no GPU has ever run this client.

use bevy::prelude::*;

use super::props::Soup;
use super::rig::EyeCam;

/// Frames a warm entity stays alive. **`decal.rs`'s number, imported rather
/// than re-picked** — it is the same claim about the same renderer, and two
/// copies of it would be two things to keep true.
pub const WARM_FRAMES: u32 = super::decal::PREWARM_FRAMES;

/// How far in front of the eye a warm entity sits, metres. Past the camera's
/// 0.1 m near plane with room to spare, and close enough that nothing can get
/// between it and the lens.
pub const WARM_AHEAD_M: f32 = 0.5;

/// The warm entity's drawn size, metres — applied as a transform SCALE over a
/// one-metre mesh, never baked into the vertices.
///
/// **Both halves of that were measured rather than chosen.** The size is
/// what `tests/prewarm.rs` needs to keep the draw under a tenth of a pixel at
/// the worst case a player can produce — a 4K panel at the bottom of the
/// field-of-view slider, where a narrow fov magnifies. The first cut was
/// 1e-4, which is 0.374 px there: a white speck in the middle of the screen
/// every time a material is created. And it is a scale rather than a vertex
/// coordinate because `Soup::mesh` generates tangents, and mikktspace refuses
/// a degenerate triangle — a mesh authored at 2e-5 with UVs to match is a
/// panic at boot waiting for a rounding error.
///
/// Written out rather than as `2.0e-5`: `ci/knob_registry.mjs` reads a
/// declaration's initializer and cannot parse an exponent, and a knob the
/// registry cannot pin is a knob the registry is not checking.
pub const WARM_SIZE_M: f32 = 0.000_02;

/// A material being warmed, and how many frames it has left.
#[derive(Component)]
pub struct Warming(pub u32);

/// The shared mesh every warm draw uses.
///
/// **Built with [`Soup`], which is the point rather than a convenience.** A
/// pipeline's key includes the mesh's vertex layout, so warming a material
/// against a layout the game does not use would specialize a pipeline the game
/// will never ask for and leave the real one cold. `Soup` is what `props.rs`,
/// `clutter.rs` and `tree.rs` all emit, so this cannot drift from them.
#[derive(Resource)]
pub struct WarmMesh(pub Handle<Mesh>);

/// Build the shared mesh once.
pub fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let mut s = Soup::default();
    // One metre. [`WARM_SIZE_M`] shrinks the DRAW, not the mesh.
    let h = 1.0f32;
    // A single triangle: three corners, non-degenerate, with a normal facing
    // the eye. `Soup::mesh` generates tangents and refuses a degenerate one.
    s.tri(
        Vec3::new(-h, -h, 0.0),
        Vec3::new(h, -h, 0.0),
        Vec3::new(0.0, h, 0.0),
        |_| [1.0, 1.0, 1.0, 1.0],
        None,
        0.0,
    );
    commands.insert_resource(WarmMesh(meshes.add(s.mesh())));
}

/// Spawn a warm draw for every material that has just been created.
pub fn warm(
    mut commands: Commands,
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mesh: Option<Res<WarmMesh>>,
    cam: Query<Entity, With<EyeCam>>,
) {
    let (Some(mesh), Ok(cam)) = (mesh, cam.single()) else {
        // No camera yet means no view to specialize against, so there is
        // nothing this could warm. The events are still drained: a material
        // created before the rig exists is one the world is not drawing
        // either, and it will be warmed by the draw that first uses it — the
        // behaviour without this file, for that one case.
        events.clear();
        return;
    };
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        // `None` means nothing holds this material any more — it was created
        // and dropped before this system ran. Warming it would specialize a
        // pipeline for something that cannot be drawn, so the skip is the
        // behaviour and not a fallback.
        let Some(handle) = materials.get_strong_handle(*id) else {
            continue;
        };
        commands.entity(cam).with_child((
            Warming(WARM_FRAMES),
            Mesh3d(mesh.0.clone()),
            MeshMaterial3d(handle),
            // Parented to the camera, so it stays in frustum for its whole
            // life without a system moving it. `-Z` is forward in Bevy's
            // camera space.
            Transform::from_xyz(0.0, 0.0, -WARM_AHEAD_M).with_scale(Vec3::splat(WARM_SIZE_M)),
        ));
    }
}

/// Retire a warm draw once it has been queued enough times.
///
/// **Counted in frames rather than seconds** for the reason `CLAUDE.md` gives
/// about gates: a wall-clock wait is a race on a loaded box, and what this
/// actually needs is a number of QUEUED DRAWS. A frame is that unit.
pub fn retire(mut commands: Commands, mut warming: Query<(Entity, &mut Warming)>) {
    for (e, mut w) in warming.iter_mut() {
        match w.0.checked_sub(1) {
            Some(0) | None => commands.entity(e).despawn(),
            Some(left) => w.0 = left,
        }
    }
}
