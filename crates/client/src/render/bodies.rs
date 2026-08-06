//! Other players.
//!
//! The local body is not drawn — the camera is inside it (first person, eye
//! at 1.6 m). Everyone else comes from the INTERPOLATOR at the render tick,
//! which is smooth and late rather than jittery and early: the predictor is
//! the local body's alone, and using it for a remote would be predicting
//! someone else's input.
//!
//! **No per-frame allocation, and no per-frame scan** — the entity map carries
//! a generation stamp, exactly as `structures::stream` does and for the same
//! reason. The first cut collected `interp.ids()` into a `Vec` every frame and
//! then asked `ids.contains(id)` inside `retain`, which is two costs on the
//! client's hot path: one heap allocation per frame, and a linear scan per
//! live body, so retiring N remotes was O(N²). `CLAUDE.md`'s client trap says
//! a client-side hitch feels identical to a server blip to the player, so the
//! client is held to the sim thread's discipline even though it is not the
//! sim. Mark what the interpolator still holds, then `retain` the marked.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::Net;

/// One networked body, keyed by the entity id the wire uses.
#[derive(Component)]
pub struct Body(pub u32);

/// One drawn remote, with the frame it was last seen on.
struct Live {
    entity: Entity,
    seen: u64,
}

#[derive(Resource, Default)]
pub struct Bodies {
    live: HashMap<u32, Live>,
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
    /// Bumped once per frame; a body still in the interpolator is stamped
    /// with it, and `retain` drops whatever the stamp missed.
    gen: u64,
}

pub fn stream(
    mut commands: Commands,
    mut store: ResMut<Bodies>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&Body, &mut Transform)>,
    net: NonSend<Net>,
) {
    let mesh = store
        .mesh
        .get_or_insert_with(|| meshes.add(Capsule3d::new(0.4, 1.0)))
        .clone();
    let material = store
        .material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.42, 0.36, 0.28),
                perceptual_roughness: 0.75,
                ..default()
            })
        })
        .clone();

    let core = &net.session.core;
    let at = core.render_tick();
    let mut rs = client_wasm::interp::RemoteState::default();

    store.gen = store.gen.wrapping_add(1);
    let gen = store.gen;

    for id in core.interp.ids() {
        if id == core.player_id {
            continue;
        }
        // **Stamp on PRESENCE, not on a successful sample**, and the two are
        // not the same frame. The retired `ids.contains(id)` test asked only
        // whether the interpolator still held the body; `sample` additionally
        // needs two snapshots bracketing the render tick, which it briefly
        // does not have when a remote first enters AOI or when a packet is
        // late. Stamping on the sample would despawn and respawn the body
        // across that gap — a flicker this refactor would have introduced
        // while looking like a pure optimisation.
        let known = store.live.get_mut(&id).map(|live| {
            live.seen = gen;
            live.entity
        });
        if !core.interp.sample(id, at, &mut rs) {
            continue;
        }
        // The capsule's origin is its middle; the wire's y is the feet.
        let pos = Vec3::new(rs.x, rs.y + 0.9, rs.z);
        match known {
            Some(entity) => {
                if let Ok((_, mut t)) = q.get_mut(entity) {
                    t.translation = pos;
                }
            }
            None => {
                let entity = commands
                    .spawn((
                        super::WorldEntity,
                        Body(id),
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(pos),
                    ))
                    .id();
                store.live.insert(id, Live { entity, seen: gen });
            }
        }
    }

    // Anyone the interpolator has dropped has left AOI or the world.
    store.live.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });
}
