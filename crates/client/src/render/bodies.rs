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
use super::viewmodel::Models;
use super::Net;
use crate::ui::hold::{held_model_of, lit_model_of, HELD_MODELS};
use sim_core::mob;

/// Where a remote body's right fist is, in metres, in that body's own
/// frame — the third-person twin of `viewmodel::VIEWMODEL_PALM`.
/// **(knob)**, registered in `DECISIONS.md` §open ("remote hands v0").
///
/// The rig stands on its own origin facing +Z with +Y up
/// (`bodies::stream`'s note on `facing`), so right is +X: 22 cm out, 1.25 m
/// up and 18 cm forward is a hand carrying something in front of the chest
/// on a 1.8 m figure (`anim::ANIM_BODY_H_M`).
///
/// ⚠ **A fixed offset and not a bone**, and the compromise is deliberate.
/// The rig has one bound bone in this client — `anim::HEAD_BONE` — and
/// binding a second means a name that must exist in `models/stumpy.glb` or
/// the whole feature is invisible; the local viewmodel does not attach to
/// its hand bone either (`viewmodel.rs`, the `"RightHand"` lookup it
/// deliberately does not parent to). So the item does not swing with the
/// arm: it rides the body's root, which reads correctly at the distance
/// this exists for — across a clearing — and reads as floating up close.
/// `NOW.md` §0tl carries the bone bind.
pub const BODY_PALM: Vec3 = Vec3::new(0.22, 1.25, 0.18);

/// Wire yaw is `0..65536` over a full turn (`interp::RemoteState`), and this
/// is the one place it becomes radians. The sim's convention is yaw 0 facing
/// +Z increasing toward +X — the same one `rig::follow_eye` builds the local
/// camera's direction from, and the two must agree or a remote faces one way
/// while its aim cone points another.
fn wire_yaw_to_radians(q: f32) -> f32 {
    q * (std::f32::consts::TAU / 65536.0)
}

/// Wire pitch is a `u8` with **128 level and 255 straight up** — the exact
/// inverse of `look::pitch_u8`, which is the one place the local client
/// encodes it. Positive is up.
///
/// **Quantize both sides or it drifts** (`CLAUDE.md` traps). The wire carries
/// 255 steps over π, so a head can only point at one of 255 angles and this
/// says so rather than pretending to a precision the packet does not have —
/// which is also why the value is interpolated as a float before it gets
/// here (`interp::RemoteState::pitch`) and not rounded again.
fn wire_pitch_to_radians(p: f32) -> f32 {
    (p / 255.0 - 0.5) * std::f32::consts::PI
}

/// One networked body, keyed by the entity id the wire uses.
#[derive(Component)]
pub struct Body(pub u32);

/// The item in a remote body's hand — one child entity per body, spawned
/// empty and hidden, and given geometry the first time that body holds
/// something with a model.
///
/// **One entity that outlives every swap**, `viewmodel::HandLight`'s shape
/// exactly: a body that changes hotbar slots swaps two handles and a
/// transform, and never spawns or despawns anything. The alternative —
/// spawn on pick-up, despawn on empty — puts a command queue round trip
/// between a player raising a weapon and the weapon appearing, on the one
/// event this whole record exists to disclose.
#[derive(Component)]
pub struct HeldOnBody;

/// The light that item casts when the sim says it is burning.
///
/// A sibling of [`HeldOnBody`] rather than a child of it, because the
/// item's transform carries `def.scale` and a light parented under it
/// would have its offset scaled by the model's in-hand cheat — the
/// deployables are down at 0.2, so a flame 4 cm above a box's crown would
/// sit 8 mm above it. Both hang off the body root and neither reads the
/// other.
#[derive(Component)]
pub struct BodyFlame;

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
    /// The two hand entities, kept rather than looked up: this system
    /// already owns the map from wire id to `Entity` and a `Children`
    /// walk per body per frame would be a scan on the client's hot path
    /// for a value that never moves.
    hand: Entity,
    flame: Entity,
    /// Which `HELD_MODELS` row the hand and the flame are currently
    /// showing, `sleeping`'s reason exactly — written on a TRANSITION.
    /// Assigning `Mesh3d` unconditionally marks it changed 60 times a
    /// second for every remote, and a `PointLight` written every frame is
    /// re-extracted into the render world every frame.
    held: Option<usize>,
    lit: Option<usize>,
}

/// The pose and the two handles for one held row, in the body's own
/// (rig-scaled) frame.
///
/// **Everything is divided by the rig's uniform scale**, and it has to be
/// here rather than at the call site: these are children of a root that
/// carries `Transform::with_scale(rig.scale)`, so a child's local metre is
/// `rig.scale` world metres. `viewmodel::pose` answers in world metres —
/// it is shared with the first-person hand, which hangs off an unscaled
/// camera — so this is the one place the two frames meet. The ratio is
/// 1.0 today (`ANIM_BODY_H_M / ANIM_RIG_H_M`, 1.8 / 1.8) and writing the
/// division anyway is what keeps a re-measured rig from silently moving
/// every held item.
pub fn hand_pose(row: usize, scale: f32) -> Transform {
    let mut t = super::viewmodel::pose(&HELD_MODELS[row], BODY_PALM);
    t.translation /= scale;
    t.scale /= scale;
    t
}

#[derive(Resource, Default)]
pub struct Bodies {
    live: HashMap<u32, Live>,
    /// Bumped once per frame; a body still in the interpolator is stamped
    /// with it, and `retain` drops whatever the stamp missed.
    gen: u64,
}

/// Eight parameters, which clippy counts — `viewmodel::dress_arms` carries
/// the same allow and the same argument. A `SystemParam` struct would exist
/// only to satisfy the count: `models` is the eighth and it is the held
/// item's geometry, which is as distinct from the rig's scene as `feed` is
/// from `time`. Bundling two of them would hide which of the eight a future
/// reader has to think about.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut store: ResMut<Bodies>,
    mut q: Query<(&Body, &mut Transform, &mut BodyAnim)>,
    time: Res<Time>,
    rig: Res<Rig>,
    models: Res<Models>,
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
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
    let mut rs = client_core::interp::RemoteState::default();

    store.gen = store.gen.wrapping_add(1);
    let gen = store.gen;

    for id in core.interp.ids() {
        if id == core.player_id {
            continue;
        }
        // **Animals are on this lane too, and they are not people.** An
        // animal is the same class-D record a player is (`protocol` v29);
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
                    // Compared before written, and the guard is not a micro-
                    // optimisation: a `Transform` written through `DerefMut`
                    // is MARKED CHANGED whether or not the value moved, and a
                    // changed root re-propagates its whole skeleton — 55 nodes
                    // for this rig. A camp of sleepers, a corpse, or anyone
                    // standing still costs that every frame for a value that
                    // is bit-identical to the one already there.
                    if t.translation != pos {
                        t.translation = pos;
                    }
                    if t.rotation != facing {
                        t.rotation = facing;
                    }
                    // The clip choice, off state the sim already sent.
                    // `dead` is the v48 bit: a corpse keeps its slot until
                    // its owner leaves the death screen, so without it a
                    // killed player is drawn standing at idle.
                    anim.observe(pos, time.delta_secs(), rs.sleeping, rs.dead);
                    anim.pitch = wire_pitch_to_radians(rs.pitch);
                    // **The one thing the sim sends that state cannot
                    // imply.** Everything else here is derived — the gait
                    // comes out of two positions — but a swing is an input
                    // fact, and a client never receives another player's
                    // input frame. So it arrives as its own broadcast
                    // (`EV_SWING`, wire v47) and is applied AFTER `observe`,
                    // which resets nothing but the gait and would otherwise
                    // eat a swing heard on the same frame the body started
                    // moving.
                    if feed.swings().contains(&id) {
                        anim.swing();
                    }
                    // **And the blow you just landed on them** — the other
                    // fact no amount of interpolated state can imply.
                    // Attacker-only, because `EV_HIT` is unicast to the
                    // attacker: `Clip::Flinch`'s doc comment and the
                    // `DECISIONS.md` row carry the asymmetry in full.
                    // After the swing, so a body hit on the same frame it
                    // swung flinches — `BodyAnim::flinch` is the newest of
                    // the two transients here and clears the other.
                    if feed.hit_victims().contains(&id) {
                        anim.flinch();
                    }
                }
                // **The hand, and it is the one thing on this record a
                // client cannot work out for itself.** Everything else
                // above is either interpolated position or a bit; this is
                // an id the wire started carrying at v56 precisely because
                // the holder's inventory and the holder's latch are not
                // ours to read.
                update_hand(&mut commands, &mut store, id, &models, rig.scale, &rs, core);
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
                anim.observe(pos, 0.0, rs.sleeping, rs.dead);
                anim.pitch = wire_pitch_to_radians(rs.pitch);
                // A body that enters AOI on the same frame it swings still
                // gets its arc; without this the first swing of every
                // newly-visible raider is the one nobody sees.
                if feed.swings().contains(&id) {
                    anim.swing();
                }
                // Same for the blow: a body that enters AOI on the frame
                // your arrow reaches it still flinches.
                if feed.hit_victims().contains(&id) {
                    anim.flinch();
                }
                let entity = commands
                    .spawn((
                        super::WorldEntity,
                        Body(id),
                        anim,
                        // The step odometer, fresh: a body that re-enters
                        // AOI must not measure the gap as ground covered
                        // (`audio::RemoteSteps` — the producer reads the
                        // transform this system writes).
                        super::audio::RemoteSteps::default(),
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
                // Both hand entities exist from the body's first frame,
                // dark and empty — see `HeldOnBody`. Children of the root,
                // so they inherit its position, its facing and its
                // despawn, and `hand_pose` divides out its scale.
                let mut hand = Entity::PLACEHOLDER;
                let mut flame = Entity::PLACEHOLDER;
                commands.entity(entity).with_children(|b| {
                    hand = b
                        .spawn((
                            HeldOnBody,
                            Mesh3d::default(),
                            MeshMaterial3d::<StandardMaterial>::default(),
                            Transform::default(),
                            Visibility::Hidden,
                        ))
                        .id();
                    flame = b
                        .spawn((
                            BodyFlame,
                            PointLight {
                                color: super::structures::FIRE_COLOR,
                                intensity: 0.0,
                                range: 0.0,
                                // Off for `HandLight`'s reason and one
                                // more: there can be a torch per remote
                                // in the interest set, so this is the
                                // light most likely to arrive in bulk.
                                shadows_enabled: false,
                                ..default()
                            },
                            Transform::IDENTITY,
                        ))
                        .id();
                });
                store.live.insert(
                    id,
                    Live {
                        entity,
                        seen: gen,
                        sleeping: rs.sleeping,
                        hand,
                        flame,
                        // Deliberately not resolved here. The body is
                        // spawned this frame and its children are queued
                        // commands; `update_hand` runs on the NEXT frame
                        // against a `None` it can honestly compare
                        // against, which is one frame of empty hand and
                        // no ordering assumption about a command queue.
                        held: None,
                        lit: None,
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

/// Point a remote body's hand and flame at what the wire says it is
/// holding, writing only on a transition.
///
/// **Two questions, not one**, and they are asked in this order because
/// the second is a narrowing of the first: `held_model_of` says what item
/// this body carries, and `lit_model_of` says whether that item is
/// burning — a fact the server resolved (`sim-core/light.rs` `is_lit`)
/// because two of its three inputs are the holder's own. An item with no
/// model draws nothing and still lights nothing, which is the honest
/// pairing: a glow with no source is worse than neither.
///
/// Writes go through `Commands` rather than a second `Query`, and that is
/// not laziness — a `Query<&mut Transform, With<HeldOnBody>>` beside this
/// system's `Query<&mut Transform>` over bodies is not provably disjoint
/// to Bevy's scheduler and would need a `Without` on the body query to
/// compile, i.e. a filter on the hot query to serve the cold one. These
/// inserts fire on a hotbar switch and a flame edge, so a command per
/// transition is a command per second at worst.
/// Which rows a remote body's hand and flame should be showing — the half
/// of [`update_hand`] that is arithmetic, split out for
/// `viewmodel::apply_hand_light`'s reason: the socket is the only thing
/// the system adds, and a decision a gate cannot call is a decision
/// nothing checks.
///
/// **A corpse drops what it was holding, and this is the only place that
/// happens.** The wire sends the hand of a dead body because that is what
/// is true (`server/core.rs` `held_of` — hiding it there would put a
/// render policy inside the sim's answer), and this is the policy over
/// it: the death clip lays the rig out flat and an item posed off a
/// standing chest would hang in the air above the body. A **sleeper keeps
/// theirs** — that body is still upright, and a sleeping player with a
/// weapon in hand is a fact a raider should be able to read.
pub fn hand_wants(
    catalog: &protocol::ItemCatalog,
    rs: &client_core::interp::RemoteState,
) -> (Option<usize>, Option<usize>) {
    if rs.dead {
        return (None, None);
    }
    (
        held_model_of(catalog, rs.held),
        lit_model_of(catalog, rs.held, rs.lit),
    )
}

fn update_hand(
    commands: &mut Commands,
    store: &mut Bodies,
    id: u32,
    models: &Models,
    scale: f32,
    rs: &client_core::interp::RemoteState,
    core: &client_core::core::ClientCore,
) {
    let Some(live) = store.live.get_mut(&id) else {
        return;
    };
    let (want, want_lit) = hand_wants(&core.catalog, rs);
    if live.held != want {
        let mut e = commands.entity(live.hand);
        match want {
            Some(row) => {
                let (mesh, mat) = models.row(row);
                e.insert((
                    Mesh3d(mesh),
                    MeshMaterial3d(mat),
                    hand_pose(row, scale),
                    Visibility::Inherited,
                ));
            }
            // Hidden AND cleared. Hiding alone leaves the last item's
            // mesh and material handles alive on a body that put them
            // down, which keeps an asset resident for a hand that is
            // empty; clearing alone leaves a visible nothing that still
            // costs a draw-call cull.
            None => {
                e.insert((
                    Mesh3d::default(),
                    MeshMaterial3d::<StandardMaterial>::default(),
                    Visibility::Hidden,
                ));
            }
        }
        live.held = want;
    }
    if live.lit != want_lit {
        let (lumens, range, lift) = match want_lit.and_then(|row| {
            HELD_MODELS[row]
                .light
                .map(|l| (l, HELD_MODELS[row].flame_m()))
        }) {
            Some((l, flame)) => (l.lumens, l.range_m, flame),
            None => (0.0, 0.0, 0.0),
        };
        commands.entity(live.flame).insert((
            PointLight {
                color: super::structures::FIRE_COLOR,
                intensity: lumens,
                range,
                shadows_enabled: false,
                ..default()
            },
            // Up the body's own +Y from the fist, divided by the rig
            // scale for `hand_pose`'s reason. An unlit hand parks the
            // emitter back at the origin rather than leaving it where the
            // last flame was — a zero-intensity light is invisible, but a
            // stale transform is a value a later reader could trust.
            Transform::from_translation((BODY_PALM + Vec3::Y * lift) / scale),
        ));
        live.lit = want_lit;
    }
}
