//! The arrow you can see.
//!
//! Ranged combat has worked since 2026-08-06 and was **invisible**: the sim
//! flew a real projectile, stopped it on trunks and walls, and killed people
//! with it, while the wire said nothing until the moment of the hit. A shot
//! arrived as `EV_HIT`/`EV_HEALTH`/`EV_DEATH` and nothing else, so an archer
//! saw their arrow only by watching somebody's health drop. `EV_SHOT` (wire
//! v33) is the fact that was missing; this is what draws it.
//!
//! # It re-flies the sim's own arithmetic, and that is the point
//!
//! `EV_SHOT` carries the shooter, the two aim angles, and the round's speed
//! and drop in **millimetres per tick** — the exact integers
//! `sim-core/ranged.rs` integrates. So this does not approximate a curve or
//! interpolate between two reported positions: it runs the same
//! `v.y -= drop; q += v` in the same units at the same rate, from an origin
//! both sides derive the same way (the shooter's feet plus `ARROW_EYE_MM`).
//! The drawn arrow and the real one are the same arc, which is the
//! quantize-both-sides law (CLAUDE.md's trap list) applied to a tracer.
//!
//! That is worth more than it costs. The alternative — streaming positions —
//! would put an entity on the snapshot lane for something that exists for
//! forty ticks and changes nothing, and it would still look wrong between
//! updates.
//!
//! # Bevy draws, it does not decide
//!
//! Nothing here is gameplay state and nothing may read it. A tracer that is
//! late, dropped, or wrong costs a streak of motion and not a hit: the arrow
//! that can kill you is the server's, and its `EV_HIT` arrives whether or not
//! anything was ever drawn. **This is deliberately not the reference game's
//! model** — theirs lets the client own the projectile and audits it with a
//! thirteen-convar tolerance budget (`reference/PROJECTILES.md` §2/§3, and
//! §9.1 for why ours stays server-side). Here a forged tracer changes nothing
//! but what its author sees.
//!
//! # A fixed pool, never a per-frame spawn
//!
//! `TRACERS` entities are spawned once at startup and hidden; a shot claims a
//! free one and releases it when the flight ends. The trap list's rule is no
//! per-frame allocations on the client, and spawn-plus-despawn per arrow is
//! exactly that under a volley — with the added cost that a Bevy spawn is a
//! structural archetype move. `highlight.rs` makes the same call for the same
//! reason with a pool of one.

use bevy::prelude::*;

use super::feed::Feed;
use super::Net;
use sim_core::limits::TICK_HZ;
use sim_core::ranged::ARROW_EYE_MM;
use sim_core::{pitch_dir, yaw_dir};

/// Arrows drawable at once.
///
/// Well under the sim's `MAX_ARROWS` (128) on purpose: that is a shard-wide
/// cap and this is a *view* cap, bounded by what one player can see rather
/// than by what the island can hold. Sixteen is four archers volleying inside
/// one AOI, and the overflow policy is to refuse the newest tracer — never to
/// steal a live one, because a streak that vanishes mid-flight reads as a bug
/// where a missing one reads as nothing at all.
const TRACERS: usize = 16;

/// Millimetres in a metre — the wire speaks mm, Bevy speaks m.
const MM_PER_M: f32 = 1000.0;

/// The shaft, metres. Long enough to read as a streak at speed, short enough
/// not to look like a spear at rest.
const LEN_M: f32 = 0.55;
/// The shaft's radius, metres.
const RADIUS_M: f32 = 0.022;

/// One pooled tracer entity and the flight it is currently drawing.
#[derive(Default, Clone, Copy)]
struct Flight {
    /// Position in millimetres, exactly the sim's `Arrow` quanta.
    qx: i32,
    qy: i32,
    qz: i32,
    /// Velocity in millimetres per tick, likewise.
    vx: i32,
    vy: i32,
    vz: i32,
    drop: u16,
    /// Ticks of flight left; zero means this slot is free. Same convention
    /// as `sim_core::ranged::Arrow::life`, so the two read alike.
    life: u16,
    /// Fractional ticks carried between frames. The sim steps at `TICK_HZ`
    /// and the client renders at whatever the monitor does, so a frame is
    /// some non-integer number of ticks; this is the remainder, and keeping
    /// it is what stops a 144 Hz client drawing a slower arrow than a 60 Hz
    /// one.
    carry: f32,
}

/// The pool. Entities are spawned once and reused.
#[derive(Resource, Default)]
pub struct Tracers {
    slots: [Flight; TRACERS],
    entities: Vec<Entity>,
}

impl Tracers {
    /// The first free slot, or `None` when every tracer is busy — the
    /// refusal `TRACERS` documents, and deliberately not drop-oldest.
    fn free(&self) -> Option<usize> {
        self.slots.iter().position(|f| f.life == 0)
    }
}

/// Spawn the pool, hidden. One-time cost at startup, so the frame path never
/// spawns.
pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Tracers>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cylinder::new(RADIUS_M, LEN_M));
    // Unlit and pale: a tracer is a readability affordance, not a lit object
    // in the world, and one that took the scene's lighting would disappear
    // against a bright sky exactly when a shot matters most.
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.93, 0.90, 0.78),
        unlit: true,
        ..default()
    });
    pool.entities = (0..TRACERS)
        .map(|_| {
            commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
}

/// Claim a slot for every shot the feed reports.
///
/// Reads `Res<Feed>` — never `pop_shot` — because the drain is
/// single-consumer and a second caller would silently starve the first. That
/// is CLAUDE.md's clean-merge trap, and this module is exactly the kind of
/// second reader that caused it.
pub fn launch(mut pool: ResMut<Tracers>, feed: Res<Feed>, net: NonSend<Net>) {
    if feed.shots().is_empty() {
        return;
    }
    let core = &net.session.core;
    let at = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();

    for &(shooter, yaw, pitch, speed_mmpt, drop_mmpt2) in feed.shots() {
        // Where the bow was. The local player's own shot comes off the
        // predictor — the interpolator does not hold you — and everyone
        // else's off the same sample `bodies::stream` draws them at, so a
        // tracer leaves the bow the player can actually see rather than the
        // server's older copy of it.
        let feet = if shooter == core.player_id {
            core.predict.position()
        } else if core.interp.sample(shooter, at, &mut rs) {
            [rs.x, rs.y, rs.z]
        } else {
            // A shot from someone outside AOI, or one that arrived before
            // their first snapshot. Nothing to hang it on, so it is dropped
            // rather than drawn from the origin.
            continue;
        };
        let Some(ix) = pool.free() else {
            return;
        };

        let (fx, fz) = yaw_dir(yaw);
        let (ch, sv) = pitch_dir(pitch);
        let speed = speed_mmpt as f32;
        pool.slots[ix] = Flight {
            qx: (feet[0] * MM_PER_M) as i32,
            qy: (feet[1] * MM_PER_M) as i32 + ARROW_EYE_MM,
            qz: (feet[2] * MM_PER_M) as i32,
            vx: (fx * ch * speed) as i32,
            vy: (sv * speed) as i32,
            vz: (fz * ch * speed) as i32,
            drop: drop_mmpt2,
            // The sim derives this from the weapon's reach and the round's
            // speed; the wire does not carry reach, so the tracer runs on
            // the store's own backstop instead. A streak that outlives the
            // arrow by a few ticks is invisible — it is already past
            // anything it could have hit — where one that dies early reads
            // as the arrow vanishing.
            life: sim_core::limits::MAX_ARROW_LIFE_TICKS,
            carry: 0.0,
        };
    }
}

/// Fly every live tracer and write its transform.
pub fn fly(
    mut pool: ResMut<Tracers>,
    time: Res<Time>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
) {
    let ticks = time.delta_secs() * TICK_HZ as f32;
    let entities = pool.entities.clone();
    for (ix, entity) in entities.iter().enumerate() {
        let Ok((mut tf, mut vis)) = q.get_mut(*entity) else {
            continue;
        };
        let f = &mut pool.slots[ix];
        if f.life == 0 {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        }

        // Whole ticks this frame, remainder carried. Integer steps only —
        // the same arithmetic the sim runs, so the curve cannot diverge.
        f.carry += ticks;
        let mut steps = f.carry as u32;
        f.carry -= steps as f32;
        while steps > 0 && f.life > 0 {
            f.vy -= f.drop as i32;
            f.qx += f.vx;
            f.qy += f.vy;
            f.qz += f.vz;
            f.life -= 1;
            steps -= 1;
        }

        if f.life == 0 {
            *vis = Visibility::Hidden;
            continue;
        }
        let pos = Vec3::new(
            f.qx as f32 / MM_PER_M,
            f.qy as f32 / MM_PER_M,
            f.qz as f32 / MM_PER_M,
        );
        // Point the shaft along travel. A cylinder's axis is +Y, so the
        // rotation is from +Y onto the velocity — an arrow drawn upright
        // while flying flat is the tell that this was skipped.
        let dir = Vec3::new(f.vx as f32, f.vy as f32, f.vz as f32);
        tf.translation = pos;
        if dir.length_squared() > 0.0 {
            tf.rotation = Quat::from_rotation_arc(Vec3::Y, dir.normalize());
        }
        if *vis != Visibility::Visible {
            *vis = Visibility::Visible;
        }
    }
}
