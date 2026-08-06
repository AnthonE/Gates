//! Other players.
//!
//! The local body is not drawn — the camera is inside it (first person, eye
//! at 1.6 m). Everyone else comes from the INTERPOLATOR at the render tick,
//! which is smooth and late rather than jittery and early: the predictor is
//! the local body's alone, and using it for a remote would be predicting
//! someone else's input.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::Net;

/// One networked body, keyed by the entity id the wire uses.
#[derive(Component)]
pub struct Body(pub u32);

#[derive(Resource, Default)]
pub struct Bodies {
    live: HashMap<u32, Entity>,
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
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

    let ids: Vec<u32> = core.interp.ids().collect();
    for id in &ids {
        if *id == core.player_id || !core.interp.sample(*id, at, &mut rs) {
            continue;
        }
        // The capsule's origin is its middle; the wire's y is the feet.
        let pos = Vec3::new(rs.x, rs.y + 0.9, rs.z);
        match store.live.get(id) {
            Some(e) => {
                if let Ok((_, mut t)) = q.get_mut(*e) {
                    t.translation = pos;
                }
            }
            None => {
                let e = commands
                    .spawn((
                        super::WorldEntity,
                        Body(*id),
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(pos),
                    ))
                    .id();
                store.live.insert(*id, e);
            }
        }
    }

    // Anyone the interpolator has dropped has left AOI or the world.
    store.live.retain(|id, e| {
        if ids.contains(id) {
            return true;
        }
        commands.entity(*e).despawn();
        false
    });
}
