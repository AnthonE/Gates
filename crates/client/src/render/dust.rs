//! The puff a blow knocks out of a surface.
//!
//! A chip burst says *something came off*. It cannot say *the thing is
//! solid*, because eight tumbling flakes are a punctuation mark and not a
//! cloud, and it is the cloud — grey off granite, tan off a trunk, brown off
//! the ground — that a player reads as the blow having weight. The reference
//! game puts one under every pick strike and every hatchet blow, and on
//! 2026-09-13 the operator named the gap (*"we dont really have any good FX
//! effect around combat or hitting rocks or hitting woods"*). This is the
//! soft half of the answer; `sparks.rs` is the hot half.
//!
//! # A billboard, lit like the ground it came off
//!
//! Each puff is one quad turned to the camera every frame, drawn through a
//! soft round mask. It is **lit**, not `unlit`, for the reason `impact.rs`
//! gives about chips — a cloud that ignored the sun would glow at night —
//! and the quad's vertex normal is its own local **+Y** rather than the +Z
//! it faces the camera with, so after the billboard turn the normal is the
//! camera's up, which is world-up to within the pitch the player is
//! looking down at. A puff is then shaded like the ground under it, which
//! is what dust in a clearing is: bright in sun, dim under a canopy.
//!
//! # Per-slot materials, and the write they cost
//!
//! A puff fades by alpha and grows by scale, and the growth alone would not
//! do — a cloud that spread without thinning reads as a balloon. Alpha on a
//! `StandardMaterial` is a uniform, so every slot owns a material and the
//! fade is a write to it, `decal.rs`'s exact trade and its exact
//! mitigation: quantized to [`DUST_ALPHA_STEPS`] so the whole fade is that
//! many uploads per puff and not one per frame.
//!
//! # It decides nothing
//!
//! `RENDER.md` §1 and `impact.rs`'s posture: fired from a [`Contact`] the sim
//! announced, fixed pool, drop-oldest, every law a pure method on the pool
//! (`tests/dust.rs`).
//!
//! [`Contact`]: super::impact::Contact

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::impact::{Burst, Matter};
use super::Eye;

/// Puffs drawable at once. Three per blow on stone, so this is thirteen
/// blows inside one puff's life — a crowded clearing cannot reach it and it
/// is still one array (wall 4).
pub const DUST_POOL: usize = 40;
/// A puff's nominal life, seconds; each is rolled between 0.8× and 1.2× so
/// three puffs off one blow do not vanish on one frame.
pub const DUST_LIFE_S: f32 = 0.9;
/// A puff's radius at birth and at death, metres. It grows between them
/// with an ease-out — fast at the blow, drifting at the end.
pub const DUST_R0_M: f32 = 0.10;
pub const DUST_R1_M: f32 = 0.42;
/// A puff's opacity at birth. Well under one: dust is a veil, and a solid
/// disc reads as a smoke grenade.
pub const DUST_ALPHA: f32 = 0.42;
/// The kick a puff leaves the surface with, m/s — along the normal, spread
/// by [`DUST_SPRAY`]. Slow: this is what stays behind after the chips have
/// gone.
pub const DUST_KICK_MPS: f32 = 1.1;
/// How fast a puff rises once its kick has decayed, m/s. Warm air off a
/// struck surface, and the one motion that separates dust from debris.
pub const DUST_RISE_MPS: f32 = 0.30;
/// Air drag on the kick, per second (`e^(−drag·dt)`), so the cloud stops
/// where it was thrown and hangs.
pub const DUST_DRAG_PER_S: f32 = 3.0;
/// The share of a puff's kick along the surface normal rather than
/// scattered — half, because a cloud is round.
pub const DUST_SPRAY: f32 = 0.5;
/// Alpha steps a fade is quantized to — `decal::ALPHA_STEPS`'s reason.
pub const DUST_ALPHA_STEPS: f32 = 24.0;
/// Puffs one blow knocks out of each matter. Zero for flesh: `ART.md` has
/// no blood in it, and a cloud off a body would be that.
pub const DUST_BURST: [usize; super::impact::MATTER_COUNT] = [2, 3, 2, 0, 1, 3, 4, 2, 0];
/// The mask's side, texels.
const DUST_TEX: u32 = 64;

/// How many puffs a blow on `matter` knocks loose.
pub fn burst_size(matter: Matter) -> usize {
    DUST_BURST[matter.slot()]
}

/// A puff's colour, before the alpha. Values chosen to sit near the surface
/// they come off — dust is that surface, powdered — and lighter than it,
/// because powder scatters more than the solid it was.
pub fn tint(matter: Matter) -> Color {
    match matter {
        Matter::Wood => Color::srgb(0.66, 0.56, 0.40),
        Matter::Stone => Color::srgb(0.60, 0.58, 0.54),
        Matter::Metal => Color::srgb(0.52, 0.49, 0.44),
        Matter::Flesh => Color::srgb(0.46, 0.10, 0.10),
        Matter::Plant => Color::srgb(0.52, 0.56, 0.36),
        Matter::Dirt => Color::srgb(0.50, 0.42, 0.30),
        Matter::Sand => Color::srgb(0.78, 0.71, 0.56),
        Matter::Grass => Color::srgb(0.52, 0.50, 0.36),
        Matter::Water => Color::srgb(0.80, 0.86, 0.90),
    }
}

/// A pool entity — `impact::ChipOf`'s reason.
#[derive(Component)]
pub struct PuffOf;

/// One puff. `left == 0` is a free slot.
#[derive(Clone, Copy)]
struct Puff {
    left: f32,
    life: f32,
    pos: Vec3,
    vel: Vec3,
    /// A turn about the view axis so three puffs off one blow do not
    /// present one mask three times.
    roll: f32,
    matter: Matter,
    /// The alpha step last written to this slot's material.
    step: i32,
}

impl Default for Puff {
    fn default() -> Self {
        Self {
            left: 0.0,
            life: 1.0,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            roll: 0.0,
            matter: Matter::Dirt,
            step: -1,
        }
    }
}

/// The pool — state only, `impact::Chips`'s shape.
#[derive(Resource)]
pub struct Dust {
    slots: [Puff; DUST_POOL],
    entities: Vec<Entity>,
    next: usize,
    pub bursts: u64,
    pub stolen: u64,
    rng: u32,
}

impl Default for Dust {
    fn default() -> Self {
        Self {
            slots: [Puff::default(); DUST_POOL],
            entities: Vec::new(),
            next: 0,
            bursts: 0,
            stolen: 0,
            rng: 0,
        }
    }
}

/// What one slot draws as this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PuffDraw {
    pub pos: Vec3,
    pub radius: f32,
    pub alpha: f32,
    pub roll: f32,
    pub matter: Matter,
}

impl Dust {
    fn claim(&mut self) -> usize {
        if let Some(i) = self.slots.iter().position(|p| p.left <= 0.0) {
            return i;
        }
        self.stolen += 1;
        let i = self.next;
        self.next = (self.next + 1) % DUST_POOL;
        i
    }

    /// How many puffs are drawing right now.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|p| p.left > 0.0).count()
    }

    fn roll(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x2545_F491;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }

    /// Knock one blow's dust loose. A matter with no dust knocks nothing and
    /// counts nothing.
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
            let dir = (away * DUST_SPRAY + scatter * (1.0 - DUST_SPRAY)).normalize_or(away);
            let life = DUST_LIFE_S * (0.8 + 0.4 * self.roll());
            let roll = self.signed() * std::f32::consts::PI;
            let ix = self.claim();
            self.slots[ix] = Puff {
                left: life,
                life,
                // Born its own birth radius off the surface, so the disc's
                // centre is never inside the wall it came off.
                pos: b.at + dir * DUST_R0_M,
                vel: dir * (DUST_KICK_MPS * (0.6 + 0.8 * self.roll())),
                roll,
                matter: b.matter,
                step: -1,
            };
        }
    }

    /// Advance every live puff by `dt`.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let drag = (-DUST_DRAG_PER_S * dt).exp();
        for p in &mut self.slots {
            if p.left <= 0.0 {
                continue;
            }
            p.left -= dt;
            if p.left <= 0.0 {
                p.left = 0.0;
                continue;
            }
            p.vel *= drag;
            p.pos += (p.vel + Vec3::Y * DUST_RISE_MPS) * dt;
        }
    }

    /// What slot `i` draws as, or `None` when it is free.
    ///
    /// The radius eases out from [`DUST_R0_M`] to [`DUST_R1_M`] and the alpha
    /// falls from [`DUST_ALPHA`] to zero along a curve that is steepest at
    /// the end — the cloud thins slowly and then is gone, rather than
    /// vanishing at half strength.
    pub fn draw(&self, i: usize) -> Option<PuffDraw> {
        let p = self.slots.get(i)?;
        if p.left <= 0.0 {
            return None;
        }
        let u = (1.0 - p.left / p.life).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - u) * (1.0 - u);
        let radius = DUST_R0_M + (DUST_R1_M - DUST_R0_M) * ease;
        let alpha = DUST_ALPHA * (1.0 - u) * (1.0 - u * 0.5);
        Some(PuffDraw {
            pos: p.pos,
            radius,
            alpha,
            roll: p.roll,
            matter: p.matter,
        })
    }

    /// The quantized alpha step for slot `i` this frame, and whether it
    /// moved since the last write — the draw writes the material only when
    /// this says so.
    pub fn alpha_step(&mut self, i: usize, alpha: f32) -> (f32, bool) {
        let step = (alpha * DUST_ALPHA_STEPS).ceil() as i32;
        let moved = self.slots[i].step != step;
        self.slots[i].step = step;
        (step as f32 / DUST_ALPHA_STEPS, moved)
    }
}

/// The shared mask and mesh, built once.
#[derive(Resource)]
pub struct DustAssets {
    materials: Vec<Handle<StandardMaterial>>,
}

/// The billboard: a unit quad in its own XY plane facing +Z, whose vertex
/// normal is +Y. See the header — the normal is what lights it like the
/// ground, and the facing is what turns it to the camera.
pub fn billboard_mesh() -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-0.5f32, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
    );
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 4]);
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
    );
    m.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    m
}

/// The mask: three soft discs off centre, white, transparent past the edge.
/// Three rather than one so a puff is a lump and not a lens flare; the
/// offsets are fixed, and the per-puff `roll` is what varies them.
pub fn puff_texture() -> Image {
    let n = DUST_TEX as usize;
    let mut data = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) * 0.5;
    // (centre x, centre y, radius) as fractions of the half-side.
    let lobes = [
        (0.0f32, 0.0f32, 0.62f32),
        (0.28, 0.18, 0.48),
        (-0.24, -0.22, 0.44),
    ];
    for y in 0..n {
        for x in 0..n {
            let (px, py) = ((x as f32 - c) / c, (y as f32 - c) / c);
            let mut a = 0.0f32;
            for (lx, ly, lr) in lobes {
                let d = ((px - lx) * (px - lx) + (py - ly) * (py - ly)).sqrt() / lr;
                let t = (1.0 - d).clamp(0.0, 1.0);
                a += t * t * (3.0 - 2.0 * t);
            }
            // The outer 5% is transparent padding whatever the lobes say,
            // so the quad's edge is never a visible line.
            let edge = (px * px + py * py).sqrt();
            let pad = ((0.95 - edge) / 0.15).clamp(0.0, 1.0);
            let a = (a.min(1.0) * pad * 255.0) as u8;
            let i = (y * n + x) * 4;
            data[i] = 255;
            data[i + 1] = 255;
            data[i + 2] = 255;
            data[i + 3] = a;
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: DUST_TEX,
            height: DUST_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::linear();
    img
}

pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Dust>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(billboard_mesh());
    let mask = images.add(puff_texture());
    let mut mats = Vec::with_capacity(DUST_POOL);
    pool.entities = (0..DUST_POOL)
        .map(|_| {
            let mat = materials.add(StandardMaterial {
                base_color: tint(Matter::Dirt).with_alpha(0.0),
                base_color_texture: Some(mask.clone()),
                // Powder: no highlight, no metal, and no back-face cull
                // because a billboard's winding is whatever the last turn
                // left it.
                perceptual_roughness: 1.0,
                metallic: 0.0,
                reflectance: 0.0,
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                cull_mode: None,
                ..default()
            });
            let entity = commands
                .spawn((
                    PuffOf,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_scale(Vec3::splat(DUST_R0_M * 2.0)),
                    Visibility::Hidden,
                ))
                .id();
            mats.push(mat);
            entity
        })
        .collect();
    commands.insert_resource(DustAssets { materials: mats });
}

/// The quaternion that turns a billboard to face the eye — the camera's own
/// rotation, `rig::follow_eye`'s look vector exactly, so the quad's +Z
/// looks back down the view axis and its +Y is the camera's up.
pub fn facing(eye: &Eye) -> Quat {
    let cp = eye.pitch.cos();
    let dir = Vec3::new(eye.yaw.sin() * cp, eye.pitch.sin(), eye.yaw.cos() * cp);
    Transform::default().looking_to(dir, Vec3::Y).rotation
}

/// Advance the pool and copy it onto the entities.
///
/// Slot `i`'s entity wears slot `i`'s material — both lists were filled in
/// one pass by `setup`, so the alpha write below addresses the material the
/// entity is actually drawn with.
pub fn fly(
    time: Res<Time>,
    eye: Res<Eye>,
    mut pool: ResMut<Dust>,
    assets: Option<Res<DustAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&mut Transform, &mut Visibility), With<PuffOf>>,
) {
    pool.step(time.delta_secs());
    let Some(assets) = assets else { return };
    let face = facing(&eye);
    for i in 0..DUST_POOL {
        let Some(&entity) = pool.entities.get(i) else {
            continue;
        };
        let Ok((mut tf, mut vis)) = q.get_mut(entity) else {
            continue;
        };
        match pool.draw(i) {
            None => {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
            }
            Some(d) => {
                tf.translation = d.pos;
                tf.rotation = face * Quat::from_rotation_z(d.roll);
                tf.scale = Vec3::splat(d.radius * 2.0);
                *vis = Visibility::Visible;
                let (alpha, moved) = pool.alpha_step(i, d.alpha);
                if moved {
                    if let Some(m) = materials.get_mut(&assets.materials[i]) {
                        m.base_color = tint(d.matter).with_alpha(alpha);
                    }
                }
            }
        }
    }
}

/// Retire every puff on the way out of a world — `impact::forget`'s twin.
pub fn forget(mut pool: ResMut<Dust>, mut q: Query<&mut Visibility, With<PuffOf>>) {
    for i in 0..DUST_POOL {
        pool.slots[i] = Puff::default();
        if let Some(&e) = pool.entities.get(i) {
            if let Ok(mut v) = q.get_mut(e) {
                *v = Visibility::Hidden;
            }
        }
    }
    pool.bursts = 0;
    pool.stolen = 0;
}
