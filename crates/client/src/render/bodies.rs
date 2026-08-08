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

use super::anim::{BodyAnim, Reshade, Rig};
use super::Net;
use sim_core::mob;

/// Wire yaw is `0..65536` over a full turn (`interp::RemoteState`), and this
/// is the one place it becomes radians. The sim's convention is yaw 0 facing
/// +Z increasing toward +X — the same one `rig::follow_eye` builds the local
/// camera's direction from, and the two must agree or a remote faces one way
/// while its aim cone points another.
fn wire_yaw_to_radians(q: f32) -> f32 {
    q * (std::f32::consts::TAU / 65536.0)
}

/// One networked body, keyed by the entity id the wire uses.
#[derive(Component)]
pub struct Body(pub u32);

/// One drawn remote, with the frame it was last seen on.
struct Live {
    entity: Entity,
    seen: u64,
    /// Which of the two materials this body is currently wearing. Kept so
    /// the swap below is written on a *transition* and not every frame:
    /// assigning `MeshMaterial3d` unconditionally would mark the component
    /// changed 60 times a second for every remote, which is a per-frame
    /// cost on the client's hot path for a value that changes twice in a
    /// body's life.
    sleeping: bool,
}

#[derive(Resource, Default)]
pub struct Bodies {
    live: HashMap<u32, Live>,
    /// Bumped once per frame; a body still in the interpolator is stamped
    /// with it, and `retain` drops whatever the stamp missed.
    gen: u64,
}

pub fn stream(
    mut commands: Commands,
    mut store: ResMut<Bodies>,
    mut q: Query<(&Body, &mut Transform, &mut BodyAnim)>,
    time: Res<Time>,
    rig: Res<Rig>,
    net: NonSend<Net>,
) {
    // Nothing is drawn until the rig has loaded. A body spawned before it
    // would get no scene and no player, and the bind below only ever runs on
    // `Added<AnimationPlayer>` — so it would stay an invisible entity forever
    // rather than catching up.
    if !rig.ready() {
        return;
    }
    let scene = rig.scene.clone().expect("rig.ready() checked above");
    let core = &net.session.core;
    let at = core.render_tick();
    let mut rs = client_wasm::interp::RemoteState::default();

    store.gen = store.gen.wrapping_add(1);
    let gen = store.gen;

    for id in core.interp.ids() {
        if id == core.player_id {
            continue;
        }
        // **Animals are on this lane too, and they are not people.** An
        // animal is the same class-D record a player is (`protocol` v28);
        // the only thing separating them is the high bit of the id, and
        // this loop's `!= player_id` was the whole filter until there was
        // something else on the wire. Without it every pig also grew a
        // humanoid rig standing in it — caught in a capture, invisible to
        // every gate, because a mannequin at a pig's coordinates is a
        // perfectly well-formed remote body. `mobs::stream` takes the half
        // this skips, and the two conditions are exact complements.
        if mob::slot_of_id(id).is_some() {
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
            (live.entity, live.sleeping)
        });
        if !core.interp.sample(id, at, &mut rs) {
            continue;
        }
        // **The rig's origin is its FEET, and the capsule's was its middle.**
        // The old draw added 0.9 m to the wire's y to centre a pill; a glTF
        // humanoid stands on its own origin, so adding it again floats every
        // player a metre off the ground. The wire's y IS the feet — no offset.
        let pos = Vec3::new(rs.x, rs.y, rs.z);
        // The wire has carried yaw since the first snapshot and nothing ever
        // read it: bodies faced +Z no matter where they were looking or
        // walking. A capsule hid that; a figure with a face cannot.
        let facing = Quat::from_rotation_y(wire_yaw_to_radians(rs.yaw));
        match known {
            Some((entity, was_sleeping)) => {
                if let Ok((_, mut t, mut anim)) = q.get_mut(entity) {
                    t.translation = pos;
                    t.rotation = facing;
                    // The clip choice, off state the sim already sent.
                    anim.observe(pos, time.delta_secs(), rs.sleeping);
                }
                if was_sleeping != rs.sleeping {
                    // The shade lives on the scene's descendants now, so the
                    // swap is a marker the walk consumes rather than a
                    // component on this entity — see `anim::Reshade`.
                    commands.entity(entity).insert(Reshade(rs.sleeping));
                    if let Some(live) = store.live.get_mut(&id) {
                        live.sleeping = rs.sleeping;
                    }
                }
            }
            None => {
                let mut anim = BodyAnim::default();
                anim.observe(pos, 0.0, rs.sleeping);
                let entity = commands
                    .spawn((
                        super::WorldEntity,
                        Body(id),
                        anim,
                        // Painted as soon as the scene's meshes exist; until
                        // then the body wears the library's own preview
                        // colours, which is one or two frames.
                        Reshade(rs.sleeping),
                        SceneRoot(scene.clone()),
                        Transform::from_translation(pos)
                            .with_rotation(facing)
                            .with_scale(Vec3::splat(rig.scale)),
                    ))
                    .id();
                store.live.insert(
                    id,
                    Live {
                        entity,
                        seen: gen,
                        sleeping: rs.sleeping,
                    },
                );
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
