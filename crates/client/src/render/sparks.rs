//! What a blow on stone or metal throws that is HOT.
//!
//! `impact.rs` throws chips — a lit solid, a thing in the world — and a chip
//! is the right answer for wood, dirt and flesh. It is the wrong answer for
//! the two matters a pick actually strikes fire from. The operator's ask
//! (2026-09-13: *"even some nice generic sparks would be good but i need them
//! realistic ish like rust"*) is a different object with a different physics:
//! a spark is not lit, it **emits**; it is not a tumbling square, it is a
//! streak drawn by its own velocity; it does not shrink out, it **cools**
//! — white, then yellow, then orange, then a red ember — and it is gone in a
//! third of a second. Three of those four facts are one law each below, and
//! the fourth ([`SPARK_HEAT`]) is a ladder of four materials rather than a
//! per-frame colour write, for the reason `impact::fly` gives: a burst costs
//! handle copies, never an asset mutation.
//!
//! # Why emissive, and why HDR
//!
//! The camera carries `Bloom::NATURAL` (`rig.rs`) with no threshold, so what
//! makes a spark glow is not a halo texture — it is a base colour whose
//! linear value is far past 1.0 on an `unlit` material. The tone map draws
//! the core white and the bloom pass scatters the excess into the halo, which
//! is what a real spark does to a real sensor. The same ladder on an LDR
//! colour would be four flat orange dots.
//!
//! # It decides nothing
//!
//! `RENDER.md` §1, and `impact.rs`'s posture exactly: every burst here is
//! fired from a [`Contact`] the sim already announced, never from the button
//! and never from the client's own reading of what is in reach. The pool is
//! fixed ([`SPARK_POOL`]), the overflow is drop-oldest for `impact.rs`'s
//! reason (the oldest spark is the dimmest ember), and every law is on the
//! pool as a pure method a test drives with no `World` and no GPU
//! (`tests/sparks.rs`).
//!
//! [`Contact`]: super::impact::Contact

use bevy::prelude::*;

use super::impact::{Burst, Matter, CHIP_GRAVITY_MPS2};

/// Sparks drawable at once. A view cap on a client-driven path (wall 4):
/// [`SPARK_BURST_METAL`] per blow on metal, so this is nearly nine
/// simultaneous metal blows' worth, against a spark that lives well under
/// half a second and a melee cadence of one blow per player per 1.267 s.
pub const SPARK_POOL: usize = 160;
/// Sparks one blow on metal throws. Metal is the one matter a real strike
/// throws a *shower* off.
pub const SPARK_BURST_METAL: usize = 18;
/// Sparks one blow on stone throws — a pick on granite flashes a few, and a
/// few is what says "hard" without saying "metal".
pub const SPARK_BURST_STONE: usize = 6;
/// A spark's nominal life, seconds; each is rolled between half and 1.3× of
/// it. Short on purpose: a spark that outlives the blow reads as a firework.
pub const SPARK_LIFE_S: f32 = 0.32;
/// The nominal launch speed, m/s; each spark is rolled between half and one
/// and a half of it. Twice a chip's, because a spark is a fragment with no
/// mass to speak of and the eye reads speed as heat.
pub const SPARK_SPEED_MPS: f32 = 6.5;
/// The share of a spark's launch that is along the surface normal rather
/// than scattered — `CHIP_SPRAY`'s meaning, tighter, because a spark shower
/// is a cone and a chip burst is a splash.
pub const SPARK_SPRAY: f32 = 0.70;
/// Air drag, per second: the velocity decays by `e^(−drag·dt)`. A spark is
/// light, so it slows visibly inside its own life where a chip does not.
pub const SPARK_DRAG_PER_S: f32 = 2.4;
/// The streak's thickness, metres. Under a centimetre: a spark is a line,
/// and the bloom is what gives it width.
pub const SPARK_WIDTH_M: f32 = 0.008;
/// How long a streak is, as the time its velocity is drawn over, seconds —
/// motion stretch. A spark at 6.5 m/s draws 9 cm long; one that has slowed
/// to 2 m/s draws 3 cm, which is the same visual law a camera's shutter
/// applies and the reason a fast spark reads as fast.
pub const SPARK_STRETCH_S: f32 = 0.014;
/// The streak's shortest and longest draw, metres — the stretch clamped so
/// an ember is still a mark and a launch is not a lance.
pub const SPARK_LEN_MIN_M: f32 = 0.02;
pub const SPARK_LEN_MAX_M: f32 = 0.14;
/// The cooling ladder, hottest first: linear HDR colour per rung. A spark
/// walks down it over its life ([`Sparks::draw`] picks the rung), so a
/// shower is white at the blow and red at the ground. Linear and past 1.0
/// on purpose — see the header on bloom.
pub const SPARK_HEAT: [[f32; 3]; 4] = [
    [22.0, 20.0, 15.0],
    [16.0, 9.5, 2.6],
    [7.0, 2.4, 0.4],
    [2.2, 0.45, 0.08],
];
/// The last share of a spark's life over which its width shrinks to nothing
/// — the ember going out rather than popping.
const SPARK_FADE_SHARE: f32 = 0.40;

/// How many sparks a blow on `matter` throws. Zero for everything that is
/// not stone or metal — wood does not spark, and neither does a body.
pub fn burst_size(matter: Matter) -> usize {
    match matter {
        Matter::Metal => SPARK_BURST_METAL,
        Matter::Stone => SPARK_BURST_STONE,
        Matter::Wood | Matter::Flesh | Matter::Plant | Matter::Dirt => 0,
    }
}

/// A pool entity — `impact::ChipOf`'s reason: without it the draw query
/// declares write access to every transform on the island to reach a
/// hundred and sixty it addresses by id anyway.
#[derive(Component)]
pub struct SparkOf;

/// One spark in flight. `left == 0` is a free slot.
#[derive(Clone, Copy)]
struct Spark {
    left: f32,
    /// The life it was born with, so the heat rung is a fraction of THIS
    /// spark's span and a short-lived one still walks the whole ladder.
    life: f32,
    pos: Vec3,
    vel: Vec3,
}

impl Default for Spark {
    fn default() -> Self {
        Self {
            left: 0.0,
            life: 1.0,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
        }
    }
}

/// The pool. State only — `impact::Chips`'s shape, so every law is drivable
/// headless.
#[derive(Resource)]
pub struct Sparks {
    slots: [Spark; SPARK_POOL],
    entities: Vec<Entity>,
    next: usize,
    /// Bursts that threw at least one spark, and sparks refused a free slot.
    /// The observables `tests/sparks.rs` asserts on.
    pub bursts: u64,
    pub stolen: u64,
    rng: u32,
}

impl Default for Sparks {
    fn default() -> Self {
        Self {
            slots: [Spark::default(); SPARK_POOL],
            entities: Vec::new(),
            next: 0,
            bursts: 0,
            stolen: 0,
            rng: 0,
        }
    }
}

impl Sparks {
    fn claim(&mut self) -> usize {
        if let Some(i) = self.slots.iter().position(|s| s.left <= 0.0) {
            return i;
        }
        self.stolen += 1;
        let i = self.next;
        self.next = (self.next + 1) % SPARK_POOL;
        i
    }

    /// How many sparks are drawing right now.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|s| s.left > 0.0).count()
    }

    /// One roll in `[0, 1)` — `impact::Chips::roll`'s xorshift on its own
    /// seed, so the three pools of one blow do not roll in lockstep.
    fn roll(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x7F4A_7C15;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }

    /// Throw one blow's sparks. A matter that does not spark throws nothing
    /// and counts nothing, so `bursts` is "bursts that sparked".
    pub fn ignite(&mut self, b: &Burst) {
        let n = burst_size(b.matter);
        if n == 0 {
            return;
        }
        let away = b.away.normalize_or(Vec3::Y);
        self.bursts += 1;
        for _ in 0..n {
            let scatter =
                Vec3::new(self.signed(), self.signed(), self.signed()).normalize_or(Vec3::Y);
            let dir = (away * SPARK_SPRAY + scatter * (1.0 - SPARK_SPRAY)).normalize_or(away);
            let speed = SPARK_SPEED_MPS * (0.5 + self.roll());
            let life = SPARK_LIFE_S * (0.5 + 0.8 * self.roll());
            let ix = self.claim();
            self.slots[ix] = Spark {
                left: life,
                life,
                // Born just off the surface, `impact`'s reason: a streak
                // born exactly on a wall spends its first frame inside it.
                pos: b.at + dir * (SPARK_LEN_MIN_M * 2.0),
                // A little lift, so a shower off a vertical face arcs up
                // and out instead of only out.
                vel: dir * speed + Vec3::Y * (SPARK_SPEED_MPS * 0.15),
            };
        }
    }

    /// Advance every live spark by `dt` and retire the ones whose time is up.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let drag = (-SPARK_DRAG_PER_S * dt).exp();
        for s in &mut self.slots {
            if s.left <= 0.0 {
                continue;
            }
            s.left -= dt;
            if s.left <= 0.0 {
                s.left = 0.0;
                continue;
            }
            s.vel.y -= CHIP_GRAVITY_MPS2 * dt;
            s.vel *= drag;
            s.pos += s.vel * dt;
        }
    }

    /// What slot `i` draws as: position, the streak's rotation (its own
    /// local +X laid along the velocity), its scale (length, width, width),
    /// and the heat rung into [`SPARK_HEAT`]. `None` when the slot is free.
    pub fn draw(&self, i: usize) -> Option<(Vec3, Quat, Vec3, usize)> {
        let s = self.slots.get(i)?;
        if s.left <= 0.0 {
            return None;
        }
        let speed = s.vel.length();
        let len = (speed * SPARK_STRETCH_S).clamp(SPARK_LEN_MIN_M, SPARK_LEN_MAX_M);
        let fade = (s.left / (s.life * SPARK_FADE_SHARE)).min(1.0);
        let width = SPARK_WIDTH_M * fade;
        let rot = Quat::from_rotation_arc(Vec3::X, s.vel.normalize_or(Vec3::Y));
        // Cooling: the rung is how much of THIS spark's life is spent.
        let spent = 1.0 - s.left / s.life;
        let rung = ((spent * SPARK_HEAT.len() as f32) as usize).min(SPARK_HEAT.len() - 1);
        Some((s.pos, rot, Vec3::new(len, width, width), rung))
    }
}

/// The ladder's materials, built once so a burst never touches `Assets`.
#[derive(Resource)]
pub struct SparkAssets {
    mats: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Sparks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A unit cuboid scaled per frame to (length, width, width): the streak
    // is its scale, and orienting a box along a vector is one quaternion.
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mats: Vec<Handle<StandardMaterial>> = SPARK_HEAT
        .iter()
        .map(|[r, g, b]| {
            materials.add(StandardMaterial {
                // Linear and far past 1.0 — the whole of the glow; see the
                // header. `unlit` because a spark is its own light source
                // and the sun has nothing to add to it.
                base_color: Color::linear_rgb(*r, *g, *b),
                unlit: true,
                ..default()
            })
        })
        .collect();
    pool.entities = (0..SPARK_POOL)
        .map(|_| {
            commands
                .spawn((
                    SparkOf,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mats[0].clone()),
                    Transform::from_scale(Vec3::splat(SPARK_WIDTH_M)),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
    commands.insert_resource(SparkAssets { mats });
}

/// Advance the pool and copy it onto the entities — the draw, and nothing
/// but the draw; the motion is [`Sparks::step`].
pub fn fly(
    time: Res<Time>,
    mut pool: ResMut<Sparks>,
    assets: Option<Res<SparkAssets>>,
    mut q: Query<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        With<SparkOf>,
    >,
) {
    pool.step(time.delta_secs());
    let Some(assets) = assets else { return };
    for i in 0..SPARK_POOL {
        let Some(&entity) = pool.entities.get(i) else {
            continue;
        };
        let Ok((mut tf, mut vis, mut m)) = q.get_mut(entity) else {
            continue;
        };
        match pool.draw(i) {
            None => {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
            Some((pos, rot, scale, rung)) => {
                tf.translation = pos;
                tf.rotation = rot;
                tf.scale = scale;
                *vis = Visibility::Visible;
                if let Some(want) = assets.mats.get(rung) {
                    if m.0 != *want {
                        m.0 = want.clone();
                    }
                }
            }
        }
    }
}

/// Retire every spark on the way out of a world — `impact::forget`'s twin.
pub fn forget(mut pool: ResMut<Sparks>, mut q: Query<&mut Visibility, With<SparkOf>>) {
    for i in 0..SPARK_POOL {
        pool.slots[i] = Spark::default();
        if let Some(&e) = pool.entities.get(i) {
            if let Ok(mut v) = q.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
    }
    pool.bursts = 0;
    pool.stolen = 0;
}
