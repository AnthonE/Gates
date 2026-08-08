//! Animals — the client half (`sim-core/src/mob.rs`).
//!
//! Structurally this is `bodies.rs` with a different mesh, and the split is
//! the point rather than duplication: both read the **same** interpolator,
//! because on the wire an animal is the same class-D record a player is
//! (`protocol` v28). What separates them is one bit of the entity id
//! (`limits::MOB_ID_TAG`), so each streamer takes the half it draws and
//! neither has to know the other exists.
//!
//! Nothing here decides anything (`CLAUDE.md`: Bevy draws, it does not
//! decide). The pig's heading, position, gait and life are all sim state
//! arriving on snapshots; this file owns a mesh, a material, and a
//! transform.
//!
//! **The origin is the feet.** `bodies.rs` records what the alternative
//! costs — the rig's own predecessor added 0.9 m to centre a capsule and
//! floated every player a metre off the ground — so the massing below is
//! authored with its hooves at y = 0 and `tests/mob_mesh.rs` fails if that
//! ever stops being true.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::props::{boxes_mesh_with, linear};
use super::Net;
use sim_core::mob;

/// Wire yaw is `0..65536` over a full turn; the sim's convention is yaw 0
/// facing +Z increasing toward +X. Same conversion `bodies.rs` does, and
/// the two must agree or a pig walks sideways.
fn wire_yaw_to_radians(q: f32) -> f32 {
    q * (std::f32::consts::TAU / 65536.0)
}

/// The pig, as a box massing, facing **+Z** — the sim's yaw-0 direction, so
/// the animal walks nose-first rather than shoulder-first.
///
/// Measured against the real thing rather than eyeballed, because there is a
/// number downstream that cares: 0.78 m at the shoulder and 1.5 m nose to
/// tail is a wild boar, and it is comfortably under `movement::STEP_UP +
/// EYE_HEIGHT` — a player looks *down* at this, which is most of why it
/// reads as an animal and not as a crouching person.
///
/// `(centre, half-extent, hex)`, hooves at y = 0.
const PIG: &[([f32; 3], [f32; 3], u32)] = &[
    // Barrel body — the silhouette. Everything else hangs off it.
    ([0.0, 0.52, 0.0], [0.25, 0.22, 0.52], 0x6b5a4a),
    // Shoulders, higher than the rump: a boar's line runs downhill to the
    // tail and that wedge is the shape people recognise at distance.
    ([0.0, 0.60, -0.18], [0.26, 0.17, 0.24], 0x60513f),
    ([0.0, 0.52, 0.62], [0.17, 0.17, 0.16], 0x60513f),
    ([0.0, 0.45, 0.82], [0.085, 0.08, 0.06], 0x8a6f63),
    ([-0.12, 0.70, 0.56], [0.055, 0.06, 0.025], 0x5a4b3b),
    ([0.12, 0.70, 0.56], [0.055, 0.06, 0.025], 0x5a4b3b),
    ([-0.16, 0.16, 0.34], [0.06, 0.16, 0.06], 0x4f4335),
    ([0.16, 0.16, 0.34], [0.06, 0.16, 0.06], 0x4f4335),
    ([-0.16, 0.16, -0.34], [0.06, 0.16, 0.06], 0x4f4335),
    ([0.16, 0.16, -0.34], [0.06, 0.16, 0.06], 0x4f4335),
    ([0.0, 0.66, -0.55], [0.035, 0.035, 0.05], 0x5a4b3b),
];

/// Nose-to-tail length and shoulder height the massing above claims. The
/// gate reads these off the mesh rather than trusting them.
pub const PIG_LEN_M: f32 = 1.5;
pub const PIG_H_M: f32 = 0.78;

/// The shared mesh and material, built once.
#[derive(Resource)]
pub struct PigAssets {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// The massing as a mesh — public so the gate can measure the *shipped*
/// geometry instead of a copy of it.
pub fn pig_mesh() -> Mesh {
    // `linear`, not the mean-1 `tint1` the authored structures use: nothing
    // is behind this material to modulate, so the hex above IS the albedo.
    // One tile per metre is the `Soup` default and means nothing here for the
    // same reason — there is no map to project.
    boxes_mesh_with(PIG, linear, 1.0)
}

pub fn load(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(PigAssets {
        mesh: meshes.add(pig_mesh()),
        // Untextured, like the bush and the crate: `assets/textures/` has no
        // hide map and a bristled animal wearing the bark photograph would
        // be worse than one wearing its own vertex colours. The roughness is
        // the same register the foliage material uses — hair is not shiny.
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.88,
            reflectance: 0.06,
            ..default()
        }),
    });
}

/// One drawn animal, keyed by its wire id.
struct Live {
    entity: Entity,
    seen: u64,
}

#[derive(Resource, Default)]
pub struct Herd {
    live: HashMap<u32, Live>,
    /// Bumped once per frame; anything the interpolator still holds is
    /// stamped with it and `retain` drops the rest — `bodies.rs`'s
    /// generation stamp, and for its reason: the first cut of that file
    /// allocated a `Vec` per frame and scanned it per body, which is a
    /// per-frame allocation on the client's hot path.
    gen: u64,
}

pub fn stream(
    mut commands: Commands,
    mut herd: ResMut<Herd>,
    mut q: Query<&mut Transform, With<Pig>>,
    assets: Option<Res<PigAssets>>,
    net: NonSend<Net>,
) {
    let Some(assets) = assets else {
        return; // Startup has not run yet.
    };
    let core = &net.session.core;
    let at = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();

    herd.gen = herd.gen.wrapping_add(1);
    let gen = herd.gen;

    for id in core.interp.ids() {
        // The one line that divides this file from `bodies.rs`.
        if mob::slot_of_id(id).is_none() {
            continue;
        }
        // Stamped on PRESENCE, not on a successful sample, for the reason
        // `bodies.rs` states at length: `sample` briefly has no bracketing
        // pair when an entity first enters AOI, and despawning across that
        // gap is a flicker that looks like an optimisation.
        let known = herd.live.get_mut(&id).map(|live| {
            live.seen = gen;
            live.entity
        });
        if !core.interp.sample(id, at, &mut rs) {
            continue;
        }
        let pos = Vec3::new(rs.x, rs.y, rs.z);
        let facing = Quat::from_rotation_y(wire_yaw_to_radians(rs.yaw));
        match known {
            Some(entity) => {
                if let Ok(mut t) = q.get_mut(entity) {
                    t.translation = pos;
                    t.rotation = facing;
                }
            }
            None => {
                let entity = commands
                    .spawn((
                        super::WorldEntity,
                        Pig,
                        Mesh3d(assets.mesh.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::from_translation(pos).with_rotation(facing),
                    ))
                    .id();
                herd.live.insert(id, Live { entity, seen: gen });
            }
        }
    }

    // Gone from the interpolator: out of AOI, or killed. Both arrive as the
    // server's removal and both mean the same thing to a renderer.
    herd.live.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });
}

/// One drawn animal.
#[derive(Component)]
pub struct Pig;
