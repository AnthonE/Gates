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

/// The wolf, as a box massing, facing **+Z** (predator v0).
///
/// Same construction as the pig above and deliberately so — this file's job
/// is to draw the roster, not to invent a second way of drawing. What
/// changes is the proportions, and they are the whole tell at the distance
/// the animal is first seen: **leaner, longer-legged, and the high point is
/// the shoulder**, where the pig's line runs downhill to the tail. 1.6 m
/// nose to tail and 0.85 m at the shoulder is a grey wolf, and it is still
/// well under `EYE_HEIGHT` — a player looks down at this too, which matters
/// more here than for the pig, because a silhouette at eye level reads as a
/// person and this one is going to be running at you.
///
/// The ears are the cheapest legible difference and they are why they are
/// here: two pricked cards on the skull. A boar's head has none, and at
/// 20 m the ear line is what separates the two silhouettes before the gait
/// does.
const WOLF_BODY: &[([f32; 3], [f32; 3], u32)] = &[
    // Chest and barrel — leaner than the pig's, which is most of the read.
    ([0.0, 0.62, -0.02], [0.19, 0.19, 0.42], 0x6a6258),
    // The withers: a wolf's highest point, and the reverse of the boar's
    // downhill line.
    ([0.0, 0.70, 0.16], [0.20, 0.15, 0.20], 0x5f584e),
    // Neck, carried forward rather than up.
    ([0.0, 0.62, 0.46], [0.13, 0.13, 0.16], 0x6a6258),
    ([0.0, 0.56, 0.66], [0.105, 0.105, 0.14], 0x746a5e),
    // Muzzle — long and narrow where the pig's is a blunt snout disc.
    ([0.0, 0.52, 0.80], [0.06, 0.065, 0.10], 0x8a7b6b),
    ([-0.075, 0.70, 0.62], [0.035, 0.055, 0.02], 0x5a5148),
    ([0.075, 0.70, 0.62], [0.035, 0.055, 0.02], 0x5a5148),
    ([0.0, 0.62, -0.42], [0.185, 0.175, 0.16], 0x5f584e),
    // Tail carried low and straight.
    ([0.0, 0.50, -0.60], [0.05, 0.05, 0.13], 0x554e46),
];

/// One wolf leg — longer and thinner than the pig's, hip at the local
/// origin on the same convention, hanging to y = −0.40.
const WOLF_LEG: &[([f32; 3], [f32; 3], u32)] =
    &[([0.0, -0.20, 0.0], [0.055, 0.20, 0.055], 0x4e483f)];

/// The wolf's four hips. Diagonal pairs in phase, exactly as the pig's —
/// a wolf trots too, and a species that paced would read as a camel here
/// for the same reason it would there. The stance is longer front-to-back
/// and narrower across, which is the other half of the leaner silhouette.
pub const WOLF_LEG_ANCHORS: [([f32; 3], f32); 4] = [
    ([-0.14, 0.40, 0.34], 0.0),
    ([0.14, 0.40, 0.34], std::f32::consts::PI),
    ([-0.14, 0.40, -0.40], std::f32::consts::PI),
    ([0.14, 0.40, -0.40], 0.0),
];

/// The wolf's flight gait: `flee_pct` 85 of the player's 5.5 m/s sprint.
/// Higher than the pig's 3.85, so at the same ground speed a wolf's legs
/// swing *shallower* — which is right, because it is the animal for whom
/// that speed is closer to a lope than a bolt.
pub const WOLF_LEG_FULL_MPS: f32 = 4.675;

/// What the wolf massing claims, read off the mesh by the gate.
pub const WOLF_LEN_M: f32 = 1.6;
pub const WOLF_H_M: f32 = 0.85;

/// One species' meshes. Body and leg are separate because the legs move and
/// the body does not.
pub struct SpeciesAssets {
    pub body: Handle<Mesh>,
    pub leg: Handle<Mesh>,
    pub anchors: [([f32; 3], f32); 4],
}

/// The roster's meshes and the one material, built once.
///
/// **One material across both species**, so a mixed herd still batches — the
/// hexes are vertex colours, which is what makes a second animal free at
/// draw time. Two species is the roster's whole shape and the resource
/// carries them by name rather than by index, because a `[SpeciesAssets;
/// MOB_KINDS]` would be a table this file has to keep in step with an
/// ordinal in another crate.
#[derive(Resource)]
pub struct HerdAssets {
    pub pig: SpeciesAssets,
    pub wolf: SpeciesAssets,
    pub material: Handle<StandardMaterial>,
}

impl HerdAssets {
    /// The meshes for a roster slot — the client's half of `mob::kind_of`.
    ///
    /// **This is the whole reason the wire carries no species field.** The
    /// sim asks `kind_of` at world construction and the renderer asks it
    /// here, off the slot inside the entity id, so the two sides cannot
    /// disagree about what is on screen without disagreeing about a pure
    /// function. `protocol` v29 rejected a `kind` on `EntityState` on cost;
    /// this is the thing that makes the rejection free rather than a debt.
    pub fn of(&self, slot: usize) -> &SpeciesAssets {
        match mob::kind_of(slot) {
            mob::MOB_WOLF => &self.wolf,
            _ => &self.pig,
        }
    }
}

/// The stride speed at which a slot's species reaches full swing.
pub fn full_mps_of(slot: usize) -> f32 {
    match mob::kind_of(slot) {
        mob::MOB_WOLF => WOLF_LEG_FULL_MPS,
        _ => PIG_LEG_FULL_MPS,
    }
}

/// How high above a slot's feet its voice comes from, metres.
///
/// The head, near enough — 0.6 of standing height for both species, which is
/// where a snout is on a four-legged animal carrying its skull forward. It
/// matters at all because the cue is positional: an emitter at the hooves of
/// an animal on a rise is metres from the one the player is looking at, and
/// the panner hears the difference before the eye forgives it.
///
/// Beside [`full_mps_of`] and reading the same `mob::kind_of`, so the three
/// per-species facts the render half needs — meshes, stride, voice height —
/// are one lookup pattern rather than three conventions.
/// Where a blow lands on an animal, metres above its own origin.
///
/// **Read off the shipped mesh table rather than taken as a fraction of the
/// standing height**, which is the `ANIM_RIG_H_M` habit applied to a
/// four-legged body: the first box in each species' table is documented as
/// the barrel — *"the silhouette. Everything else hangs off it"* for the pig,
/// *"chest and barrel"* for the wolf — so its centre IS the flank, and a
/// re-shaped animal moves this with it instead of leaving a constant behind.
///
/// Beside [`voice_h_of`] and [`full_mps_of`], reading the same
/// `mob::kind_of`, so the per-species facts the render half needs stay one
/// lookup pattern. It is deliberately NOT `voice_h_of`: that is the snout,
/// and a snout is where a sound comes from rather than where a hatchet
/// lands.
pub fn flank_h_of(slot: usize) -> f32 {
    match mob::kind_of(slot) {
        mob::MOB_WOLF => WOLF_BODY[0].0[1],
        _ => PIG_BODY[0].0[1],
    }
}

pub fn voice_h_of(slot: usize) -> f32 {
    let stand = match mob::kind_of(slot) {
        mob::MOB_WOLF => WOLF_H_M,
        _ => PIG_H_M,
    };
    stand * 0.6
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

/// The whole wolf at rest, assembled from the shipped tables — `pig_mesh`'s
/// contract for the other species, so the gate measures the geometry that
/// draws rather than a copy of it.
pub fn wolf_mesh() -> Mesh {
    let mut parts: Vec<([f32; 3], [f32; 3], u32)> = WOLF_BODY.to_vec();
    for (anchor, _) in WOLF_LEG_ANCHORS {
        for (c, h, hex) in WOLF_LEG {
            parts.push((
                [c[0] + anchor[0], c[1] + anchor[1], c[2] + anchor[2]],
                *h,
                *hex,
            ));
        }
    }
    boxes_mesh_with(&parts, linear, 1.0)
}

/// The wolf's body alone — what the entity itself wears.
pub fn wolf_body_mesh() -> Mesh {
    boxes_mesh_with(WOLF_BODY, linear, 1.0)
}

/// One wolf leg, hip at the origin.
pub fn wolf_leg_mesh() -> Mesh {
    boxes_mesh_with(WOLF_LEG, linear, 1.0)
}

pub fn load(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(HerdAssets {
        pig: SpeciesAssets {
            body: meshes.add(pig_body_mesh()),
            leg: meshes.add(pig_leg_mesh()),
            anchors: LEG_ANCHORS,
        },
        wolf: SpeciesAssets {
            body: meshes.add(wolf_body_mesh()),
            leg: meshes.add(wolf_leg_mesh()),
            anchors: WOLF_LEG_ANCHORS,
        },
        // Untextured, like the bush and the crate: `assets/textures/` has no
        // hide map and a bristled animal wearing the bark photograph would
        // be worse than one wearing its own vertex colours. The roughness is
        // the same register the foliage material uses — hair is not shiny.
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.88,
            // Hide, not stone — `fresnel::FLESH` is 2.8% where the island
            // is 4%. It shipped at 0.06, i.e. F0 0.06%, the darkest specular
            // anywhere in the client.
            reflectance: super::fresnel::FLESH,
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
    mut q: Query<(&mut Transform, &mut Gait), With<Animal>>,
    time: Res<Time>,
    assets: Option<Res<HerdAssets>>,
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
                // Which animal this is, off the slot — the same pure
                // function the sim built the roster with.
                let species = assets.of(slot);
                let entity = commands
                    .spawn((
                        super::WorldEntity,
                        Animal(id),
                        Gait::new(slot),
                        Mesh3d(species.body.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::from_translation(pos).with_rotation(facing),
                    ))
                    // The legs: child transforms, NOT `WorldEntity` — the
                    // teardown marks roots only and a recursive despawn of
                    // the animal takes them (`render/mod.rs::WorldEntity`).
                    .with_children(|parent| {
                        for (anchor, leg_phase) in species.anchors {
                            parent.spawn((
                                AnimalLeg(leg_phase),
                                Mesh3d(species.leg.clone()),
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
/// (`sound::voice`), so the identity has to ride the entity rather than be
/// re-derived from a position.
///
/// Named for what it is rather than for the one species that used to be in
/// the world: a `Pig` component on a wolf is the same class of lie as a
/// wolf wearing the pig's mesh, and both were true of this file until
/// predator v0.
#[derive(Component)]
pub struct Animal(pub u32);

/// One leg, carrying its place in the stride (its entry in its species'
/// anchor table).
#[derive(Component)]
pub struct AnimalLeg(pub f32);

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
    /// The speed at which THIS animal's swing reaches full amplitude — its
    /// species' flight gait, taken from the slot at spawn. Carried on the
    /// component rather than looked up in `trot`, because `trot` walks
    /// children and no longer has the slot in hand.
    pub full_mps: f32,
    /// Last interpolated position, for the difference.
    last: Option<Vec3>,
}

impl Gait {
    /// The phase origin is hashed from the roster slot — the snort's own
    /// convention (`sound::voice::hash01`), and for its reason: deterministic
    /// per animal with no OS randomness, so a herd walked into is four
    /// strides in four places rather than a chorus line.
    pub fn new(slot: usize) -> Self {
        Self {
            speed: 0.0,
            phase: crate::sound::voice::hash01(slot as u32, 0) * std::f32::consts::TAU,
            full_mps: full_mps_of(slot),
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
/// linearly with speed to [`PIG_LEG_SWING_RAD`] at `full_mps` — so a
/// standing animal's legs rest at vertical no matter where its phase
/// stopped.
///
/// `full_mps` is the species' flight gait and is a parameter rather than a
/// constant since predator v0: a wolf's 4.675 against a pig's 3.85 means the
/// same ground speed is a shallower swing on the faster animal, which is
/// the arithmetic saying that speed is a lope for one and a bolt for the
/// other. The swing ceiling stays shared — that one is anatomy, not gait.
pub fn leg_swing_rad(phase: f32, leg_phase: f32, speed_mps: f32, full_mps: f32) -> f32 {
    let amp = PIG_LEG_SWING_RAD * (speed_mps / full_mps).clamp(0.0, 1.0);
    (phase + leg_phase).sin() * amp
}

/// Swing every drawn animal's legs off the gait `stream` just advanced.
/// Runs right after it in the same chain, so the legs read this frame's
/// stride and not the last one's.
pub fn trot(
    herd: Query<(&Gait, &Children), With<Animal>>,
    mut legs: Query<(&AnimalLeg, &mut Transform)>,
) {
    for (gait, children) in herd.iter() {
        for child in children.iter() {
            if let Ok((leg, mut t)) = legs.get_mut(child) {
                t.rotation = Quat::from_rotation_x(leg_swing_rad(
                    gait.phase,
                    leg.0,
                    gait.speed,
                    gait.full_mps,
                ));
            }
        }
    }
}
