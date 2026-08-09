//! Animals — the client half (`sim-core/src/mob.rs`).
//!
//! Structurally this is `bodies.rs` with a different mesh, and the split is
//! the point rather than duplication: both read the **same** interpolator,
//! because on the wire an animal is the same class-D record a player is
//! (`protocol` v29). What separates them is one bit of the entity id
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
/// `(centre, half-extent, hex)`, hooves at y = 0. **The legs are not here**:
/// they swing, so they are a separate mesh hung as child transforms
/// ([`PIG_LEG`], [`LEG_ANCHORS`]) — [`pig_mesh`] assembles the whole animal
/// at rest for the gate to measure.
const PIG_BODY: &[([f32; 3], [f32; 3], u32)] = &[
    // Barrel body — the silhouette. Everything else hangs off it.
    ([0.0, 0.52, 0.0], [0.25, 0.22, 0.52], 0x6b5a4a),
    // Shoulders, higher than the rump: a boar's line runs downhill to the
    // tail and that wedge is the shape people recognise at distance.
    ([0.0, 0.60, -0.18], [0.26, 0.17, 0.24], 0x60513f),
    ([0.0, 0.52, 0.62], [0.17, 0.17, 0.16], 0x60513f),
    ([0.0, 0.45, 0.82], [0.085, 0.08, 0.06], 0x8a6f63),
    ([-0.12, 0.70, 0.56], [0.055, 0.06, 0.025], 0x5a4b3b),
    ([0.12, 0.70, 0.56], [0.055, 0.06, 0.025], 0x5a4b3b),
    ([0.0, 0.66, -0.55], [0.035, 0.035, 0.05], 0x5a4b3b),
];

/// One leg, authored with its **hip at the local origin** so a swing is a
/// plain rotation about x at the anchor. It hangs to y = −0.32, and each
/// anchor in [`LEG_ANCHORS`] sits at y = 0.32, so a resting leg's foot is at
/// exactly y = 0 — the same box the massing carried before the legs moved.
const PIG_LEG: &[([f32; 3], [f32; 3], u32)] = &[([0.0, -0.16, 0.0], [0.06, 0.16, 0.06], 0x4f4335)];

/// Where each leg hangs from (body space), and its place in the stride —
/// **diagonal pairs in phase**, which is a trot: front-left steps with
/// rear-right, front-right with rear-left, π apart. The reference animal is
/// a boar and a boar trots; a pace (lateral pairs) is the gait that reads as
/// a camel.
pub const LEG_ANCHORS: [([f32; 3], f32); 4] = [
    ([-0.16, 0.32, 0.34], 0.0),                   // front-left
    ([0.16, 0.32, 0.34], std::f32::consts::PI),   // front-right
    ([-0.16, 0.32, -0.34], std::f32::consts::PI), // rear-left
    ([0.16, 0.32, -0.34], 0.0),                   // rear-right
];

/// How far a leg swings from vertical at full gait, radians (`DECISIONS.md`
/// §open, "pig gait v0"). Kept under 90° by the gate: past horizontal the
/// foot would rise above its own hip, which is a cartwheel, not a stride —
/// and because the swing only ever *raises* a foot off its resting contact
/// (`hip − len·cos` ≥ 0 for any angle under π/2), no amplitude this constant
/// can hold ever pushes a hoof through the ground.
pub const PIG_LEG_SWING_RAD: f32 = 0.6;

/// Metres of ground per full stride cycle. Distance-integrated exactly like
/// the footstep odometer (`sound/steps.rs`): cadence from a clock would make
/// a fleeing pig's legs beat at a grazing pig's rate, and a hitch that
/// teleported the animal wraps the phase instead of banking swings.
pub const PIG_LEG_CYCLE_M: f32 = 1.0;

/// The speed at which the swing reaches full amplitude, m/s — the animal's
/// own flight gait (`flee_pct` 70 of the player's 5.5 m/s sprint, the
/// `mobs.toml` arithmetic `tests/content.rs` gates). Slower is a shallower
/// swing, linearly, so a grazing shuffle does not goose-step.
pub const PIG_LEG_FULL_MPS: f32 = 3.85;

/// Nose-to-tail length and shoulder height the massing above claims. The
/// gate reads these off the mesh rather than trusting them.
pub const PIG_LEN_M: f32 = 1.5;
pub const PIG_H_M: f32 = 0.78;

/// The shared meshes and material, built once. Body and leg are separate
/// meshes because the legs move and the body does not; all five parts share
/// one material, so a herd still batches.
#[derive(Resource)]
pub struct PigAssets {
    pub body: Handle<Mesh>,
    pub leg: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
}

/// The whole animal at rest — body plus all four legs at their anchors,
/// assembled from the SAME tables the draw path spawns from, so the gate
/// measures the shipped geometry rather than a copy of it. At rest the swing
/// is zero and every leg transform is a pure translation, which is why this
/// composition and the entity tree are the same shape.
pub fn pig_mesh() -> Mesh {
    let mut parts: Vec<([f32; 3], [f32; 3], u32)> = PIG_BODY.to_vec();
    for (anchor, _) in LEG_ANCHORS {
        for (c, h, hex) in PIG_LEG {
            parts.push((
                [c[0] + anchor[0], c[1] + anchor[1], c[2] + anchor[2]],
                *h,
                *hex,
            ));
        }
    }
    boxes_mesh_with(&parts, linear, 1.0)
}

/// The body alone — what the pig entity itself wears.
pub fn pig_body_mesh() -> Mesh {
    // `linear`, not the mean-1 `tint1` the authored structures use: nothing
    // is behind this material to modulate, so the hex above IS the albedo.
    // One tile per metre is the `Soup` default and means nothing here for the
    // same reason — there is no map to project.
    boxes_mesh_with(PIG_BODY, linear, 1.0)
}

/// One leg, hip at the origin — what each child transform wears.
pub fn pig_leg_mesh() -> Mesh {
    boxes_mesh_with(PIG_LEG, linear, 1.0)
}

pub fn load(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(PigAssets {
        body: meshes.add(pig_body_mesh()),
        leg: meshes.add(pig_leg_mesh()),
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
    mut q: Query<(&mut Transform, &mut Gait), With<Pig>>,
    time: Res<Time>,
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
        let Some(slot) = mob::slot_of_id(id) else {
            continue;
        };
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
                if let Ok((mut t, mut gait)) = q.get_mut(entity) {
                    t.translation = pos;
                    t.rotation = facing;
                    gait.observe(pos, time.delta_secs());
                }
            }
            None => {
                let entity = commands
                    .spawn((
                        super::WorldEntity,
                        Pig(id),
                        Gait::new(slot),
                        Mesh3d(assets.body.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::from_translation(pos).with_rotation(facing),
                    ))
                    // The legs: child transforms, NOT `WorldEntity` — the
                    // teardown marks roots only and a recursive despawn of
                    // the pig takes them (`render/mod.rs::WorldEntity`).
                    .with_children(|parent| {
                        for (anchor, leg_phase) in LEG_ANCHORS {
                            parent.spawn((
                                PigLeg(leg_phase),
                                Mesh3d(assets.leg.clone()),
                                MeshMaterial3d(assets.material.clone()),
                                Transform::from_translation(Vec3::from_array(anchor)),
                            ));
                        }
                    })
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

/// One drawn animal, carrying its wire id — the roster slot inside it
/// (`mob::slot_of_id`) is what keys the voice's per-animal cadence
/// (`sound::pig`), so the identity has to ride the entity rather than be
/// re-derived from a position.
#[derive(Component)]
pub struct Pig(pub u32);

/// One leg, carrying its place in the stride (its entry in [`LEG_ANCHORS`]).
#[derive(Component)]
pub struct PigLeg(pub f32);

/// The animal's stride, derived — the wire carries no velocity and no gait,
/// so this is `anim::BodyAnim`'s speed derivation (difference the
/// interpolated position, low-pass the result) plus a phase that advances by
/// GROUND COVERED, which is `sound/steps.rs`'s odometer rule: distance gets
/// flight cadence for free and a clock would not. Render state, never sim
/// state — deleting it changes how a pig looks and nothing else.
#[derive(Component)]
pub struct Gait {
    /// Metres per second, horizontal, low-passed.
    pub speed: f32,
    /// Where in the stride cycle this animal is, radians in `[0, TAU)`.
    pub phase: f32,
    /// Last interpolated position, for the difference.
    last: Option<Vec3>,
}

impl Gait {
    /// The phase origin is hashed from the roster slot — the snort's own
    /// convention (`sound::pig::hash01`), and for its reason: deterministic
    /// per animal with no OS randomness, so a herd walked into is four
    /// strides in four places rather than a chorus line.
    pub fn new(slot: usize) -> Self {
        Self {
            speed: 0.0,
            phase: crate::sound::pig::hash01(slot as u32, 0) * std::f32::consts::TAU,
            last: None,
        }
    }

    /// Fold a new interpolated position in.
    pub fn observe(&mut self, pos: Vec3, dt: f32) {
        let Some(last) = self.last.replace(pos) else {
            return; // First sight establishes the origin, strides nowhere.
        };
        if dt <= 0.0 {
            return;
        }
        // Horizontal only, both halves: a pig riding terrain uphill is
        // walking, not climbing, and height gained is not ground covered.
        let step = pos - last;
        let d = Vec2::new(step.x, step.z).length();
        // One-pole low pass on the SPEED, `anim.rs`'s own constant: the raw
        // difference lurches on a late packet, and the amplitude it drives
        // would lurch with it.
        let k = 1.0 - (-12.0 * dt).exp();
        self.speed += (d / dt - self.speed) * k;
        // The phase integrates the distance itself — wrap, never bank, so a
        // hitch that teleported the animal lands mid-cycle instead of buying
        // a flurry of catch-up swings.
        self.phase = (self.phase + d / PIG_LEG_CYCLE_M * std::f32::consts::TAU)
            .rem_euclid(std::f32::consts::TAU);
    }
}

/// A leg's swing angle about x, radians. Pure — `tests/mob_mesh.rs` gates
/// the trot's arithmetic through this without a window.
///
/// The sine keys the stride, the leg's own phase offset makes the diagonal
/// pairs agree and the lateral pairs mirror, and the amplitude scales
/// linearly with speed to [`PIG_LEG_SWING_RAD`] at [`PIG_LEG_FULL_MPS`] — so
/// a standing animal's legs rest at vertical no matter where its phase
/// stopped.
pub fn leg_swing_rad(phase: f32, leg_phase: f32, speed_mps: f32) -> f32 {
    let amp = PIG_LEG_SWING_RAD * (speed_mps / PIG_LEG_FULL_MPS).clamp(0.0, 1.0);
    (phase + leg_phase).sin() * amp
}

/// Swing every drawn animal's legs off the gait `stream` just advanced.
/// Runs right after it in the same chain, so the legs read this frame's
/// stride and not the last one's.
pub fn trot(
    herd: Query<(&Gait, &Children), With<Pig>>,
    mut legs: Query<(&PigLeg, &mut Transform)>,
) {
    for (gait, children) in herd.iter() {
        for child in children.iter() {
            if let Ok((leg, mut t)) = legs.get_mut(child) {
                t.rotation = Quat::from_rotation_x(leg_swing_rad(gait.phase, leg.0, gait.speed));
            }
        }
    }
}
