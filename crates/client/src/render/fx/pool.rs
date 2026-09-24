//! A fixed pool of particles and the one mesh that draws them.
//!
//! State only, and every law is a method on [`Pool`] — `ignite`/`step`/
//! `write` take plain numbers — so the cap, the drop-oldest policy, the
//! motion and the vertex layout are drivable from a test with no `World` and
//! no GPU (`tests/fx.rs`). The Bevy half (`fx::draw`) is a copy into a mesh.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

use super::atlas::{SPRITE_COLS, SPRITE_ROWS, SPRITE_TEX};

/// How a particle's quad is turned.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Orient {
    /// Faces the camera, rolled by its own angle.
    #[default]
    Billboard,
    /// Drawn along its own velocity as seen from the camera — a spark's
    /// streak — at least [`MIN_PX`] wide so it reads with bloom off.
    Stretch,
    /// Lies flat on the ground (a ring on the water).
    Flat,
}

/// A streak's narrowest drawn width as a share of its distance from the eye:
/// about 1.5 px at 1080p and a 70° field, so a spark never drops below a
/// pixel and vanishes — the web build has no bloom to widen it.
pub const MIN_PX: f32 = 1.6e-3;

/// The shortest and longest streak drawn, metres.
pub const STREAK_MIN_M: f32 = 0.02;
pub const STREAK_MAX_M: f32 = 0.18;

/// One particle. `left <= 0` is a free slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct Particle {
    pub left: f32,
    pub life: f32,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Half-extent at birth and at death, metres; a streak's width.
    pub size0: f32,
    pub size1: f32,
    /// Linear colour and alpha at birth and at death. HDR allowed.
    pub c0: [f32; 4],
    pub c1: [f32; 4],
    /// Downward acceleration, m/s² (negative drifts up, like warm dust).
    pub gravity: f32,
    /// Velocity decay per second, `e^(−drag·dt)`.
    pub drag: f32,
    pub roll: f32,
    pub spin: f32,
    pub cell: u8,
    pub orient: Orient,
    /// Seconds of velocity a streak is drawn over.
    pub stretch: f32,
    /// The longest this streak may draw, metres; zero is [`STREAK_MAX_M`].
    /// A tracer is metres long where a spark is centimetres.
    pub len_max: f32,
    /// A plane it bounces off once (a spark off the surface it came from):
    /// a point on it and its normal; a zero normal is no plane.
    pub plane_p: Vec3,
    pub plane_n: Vec3,
}

/// One particle's quad: four corners, their uvs, and its colour.
pub type Quad = ([[f32; 3]; 4], [[f32; 2]; 4], [f32; 4]);

/// The camera, as a quad needs it.
#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub pos: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

/// The pool.
pub struct Pool {
    parts: Vec<Particle>,
    /// Draw order scratch, reused every frame.
    order: Vec<u16>,
    next: usize,
    rng: u32,
    /// Whether the mesh last written held a live particle — a pool that
    /// died out needs one last write to collapse, and an empty one none.
    drawn: bool,
    /// Sort back to front (the alpha pool) or not (the additive one).
    sorted: bool,
    /// Particles spawned, and live ones recycled for want of a slot.
    pub spawned: u64,
    pub stolen: u64,
}

impl Pool {
    /// A pool of `cap` particles — allocated here, once, and never again.
    pub fn new(cap: usize, sorted: bool) -> Self {
        Self {
            parts: vec![Particle::default(); cap],
            order: (0..cap as u16).collect(),
            next: 0,
            rng: 0,
            drawn: true,
            sorted,
            spawned: 0,
            stolen: 0,
        }
    }

    pub fn cap(&self) -> usize {
        self.parts.len()
    }

    pub fn live(&self) -> usize {
        self.parts.iter().filter(|p| p.left > 0.0).count()
    }

    pub fn particles(&self) -> impl Iterator<Item = &Particle> {
        self.parts.iter().filter(|p| p.left > 0.0)
    }

    /// One roll in `[0, 1)` — a 32-bit xorshift, seeded on first use.
    pub fn roll(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x1B87_3593;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    /// A roll in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.roll() * 2.0 - 1.0
    }

    /// A free slot, or the oldest — drop-oldest: the oldest particle is the
    /// faintest, and refusing the newest would drop the blow the player just
    /// landed.
    fn claim(&mut self) -> usize {
        if let Some(i) = self.parts.iter().position(|p| p.left <= 0.0) {
            return i;
        }
        self.stolen += 1;
        let i = self.next;
        self.next = (self.next + 1) % self.parts.len();
        i
    }

    pub fn spawn(&mut self, p: Particle) {
        let i = self.claim();
        self.parts[i] = p;
        self.spawned += 1;
    }

    /// Advance every live particle by `dt`.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for p in self.parts.iter_mut() {
            if p.left <= 0.0 {
                continue;
            }
            p.left -= dt;
            if p.left <= 0.0 {
                p.left = 0.0;
                continue;
            }
            p.vel.y -= p.gravity * dt;
            p.vel *= (-p.drag * dt).exp();
            p.pos += p.vel * dt;
            p.roll += p.spin * dt;
            // One bounce off the surface it came from, losing most of its
            // speed — sparks skitter, they do not fall through the wall.
            if p.plane_n != Vec3::ZERO {
                let d = (p.pos - p.plane_p).dot(p.plane_n);
                if d < 0.0 {
                    p.pos -= p.plane_n * d;
                    let vn = p.vel.dot(p.plane_n);
                    if vn < 0.0 {
                        p.vel = (p.vel - p.plane_n * (1.35 * vn)) * 0.45;
                    }
                    p.plane_n = Vec3::ZERO;
                }
            }
        }
    }

    /// Whether the mesh has to be written this frame: something is alive, or
    /// the last write still showed something that has since died.
    pub fn needs_write(&self) -> bool {
        self.drawn || self.parts.iter().any(|p| p.left > 0.0)
    }

    /// Forget every particle.
    pub fn clear(&mut self) {
        for p in self.parts.iter_mut() {
            p.left = 0.0;
        }
    }

    /// The four corners of particle `p` relative to `anchor`, with their uvs,
    /// and its colour — or `None` for a free slot.
    pub fn quad(p: &Particle, cam: &Cam, anchor: Vec3) -> Option<Quad> {
        if p.left <= 0.0 {
            return None;
        }
        let t = 1.0 - (p.left / p.life.max(1e-4)).clamp(0.0, 1.0);
        let size = p.size0 + (p.size1 - p.size0) * t;
        let mut c = [0.0; 4];
        for (k, v) in c.iter_mut().enumerate() {
            *v = p.c0[k] + (p.c1[k] - p.c0[k]) * t;
        }
        let (a, b, centre) = match p.orient {
            Orient::Billboard => {
                let (s, co) = p.roll.sin_cos();
                let r = cam.right * co + cam.up * s;
                let u = cam.up * co - cam.right * s;
                (r * size, u * size, p.pos)
            }
            Orient::Flat => {
                let (s, co) = p.roll.sin_cos();
                (
                    Vec3::new(co, 0.0, s) * size,
                    Vec3::new(-s, 0.0, co) * size,
                    p.pos,
                )
            }
            Orient::Stretch => {
                let view = (p.pos - cam.pos).normalize_or(Vec3::Z);
                let dist = (p.pos - cam.pos).length();
                let across = p.vel - view * p.vel.dot(view);
                let max = if p.len_max > 0.0 {
                    p.len_max
                } else {
                    STREAK_MAX_M
                };
                let len = (p.vel.length() * p.stretch).clamp(STREAK_MIN_M, max);
                let axis = across.normalize_or(cam.up);
                let side = view.cross(axis).normalize_or(cam.right);
                let w = size.max(dist * MIN_PX);
                // The head leads; the tail trails back along the motion.
                (axis * (len * 0.5), side * w, p.pos - axis * (len * 0.5))
            }
        };
        let o = centre - anchor;
        let pos = [
            (o - a - b).to_array(),
            (o + a - b).to_array(),
            (o + a + b).to_array(),
            (o - a + b).to_array(),
        ];
        let (cu, cv) = (
            (p.cell as u32 % SPRITE_COLS) as f32,
            (p.cell as u32 / SPRITE_COLS) as f32,
        );
        let inset = 0.5 / SPRITE_TEX as f32;
        let uv = |u: f32, v: f32| {
            [
                (cu + inset + u * (1.0 - 2.0 * inset)) / SPRITE_COLS as f32,
                (cv + inset + v * (1.0 - 2.0 * inset)) / SPRITE_ROWS as f32,
            ]
        };
        Some((
            pos,
            [uv(0.0, 1.0), uv(1.0, 1.0), uv(1.0, 0.0), uv(0.0, 0.0)],
            c,
        ))
    }

    /// Copy the pool into `mesh`, in place, relative to `anchor` — or do
    /// nothing when it is empty and the mesh already is. Returns whether it
    /// wrote. No allocation: the attributes are the ones [`pool_mesh`] made.
    pub fn write(&mut self, mesh: &mut Mesh, cam: &Cam, anchor: Vec3) -> bool {
        let live = self.live();
        if live == 0 && !self.drawn {
            return false;
        }
        self.drawn = live > 0;
        if self.sorted {
            // Back to front, so overlapping puffs blend in order. The dead
            // sort to the end; `sort_unstable_by` does not allocate.
            let parts = &self.parts;
            self.order.sort_unstable_by(|&i, &j| {
                let (pi, pj) = (&parts[i as usize], &parts[j as usize]);
                let key = |p: &Particle| {
                    if p.left <= 0.0 {
                        f32::MIN
                    } else {
                        p.pos.distance_squared(cam.pos)
                    }
                };
                key(pj).total_cmp(&key(pi))
            });
        }
        let (mut pos, mut uv, mut col) = (None, None, None);
        for (attr, values) in mesh.attributes_mut() {
            let id = attr.id;
            match values {
                VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_POSITION.id => {
                    pos = Some(v);
                }
                VertexAttributeValues::Float32x2(v) if id == Mesh::ATTRIBUTE_UV_0.id => {
                    uv = Some(v);
                }
                VertexAttributeValues::Float32x4(v) if id == Mesh::ATTRIBUTE_COLOR.id => {
                    col = Some(v);
                }
                _ => {}
            }
        }
        let (Some(pos), Some(uv), Some(col)) = (pos, uv, col) else {
            return false;
        };
        for slot in 0..self.parts.len() {
            let i = if self.sorted {
                self.order[slot] as usize
            } else {
                slot
            };
            let base = slot * 4;
            match Self::quad(&self.parts[i], cam, anchor) {
                Some((p, t, c)) => {
                    for k in 0..4 {
                        pos[base + k] = p[k];
                        uv[base + k] = t[k];
                        col[base + k] = c;
                    }
                }
                None => {
                    for k in 0..4 {
                        pos[base + k] = [0.0; 3];
                        col[base + k] = [0.0; 4];
                    }
                }
            }
        }
        true
    }
}

/// An empty mesh for a pool of `cap` quads, with its fixed indices. Normals
/// point up: the lit pool is shaded like the ground under it.
pub fn pool_mesh(cap: usize) -> Mesh {
    let n = cap * 4;
    let mut idx = Vec::with_capacity(cap * 6);
    for q in 0..cap as u32 {
        let b = q * 4;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; n]);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}
