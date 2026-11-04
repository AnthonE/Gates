//! A fixed pool of particles, and the quads that draw it.
//!
//! State only, and every law is a method on [`Pool`]: `spawn`, `step` and
//! `fill` take plain numbers, so the cap, the drop-oldest policy, the motion
//! and the vertex layout are drivable from a test with no engine and no GPU.
//! A game's adapter owns the mesh: [`indices`] once, then [`Pool::fill`] into
//! its position, uv and colour attributes every frame something is alive.

use crate::atlas::{SPRITE_COLS, SPRITE_ROWS, SPRITE_TEX};
use crate::v3::{self, V3};

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

/// The normal every vertex of a pool's mesh carries: up, so a lit pool is
/// shaded like the ground under it.
pub const NORMAL: V3 = v3::Y;

/// One particle. `left <= 0` is a free slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct Particle {
    pub left: f32,
    pub life: f32,
    pub pos: V3,
    pub vel: V3,
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
    pub plane_p: V3,
    pub plane_n: V3,
}

/// One particle's quad: four corners, their uvs, and its colour.
pub type Quad = ([[f32; 3]; 4], [[f32; 2]; 4], [f32; 4]);

/// The camera, as a quad needs it.
#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub pos: V3,
    pub right: V3,
    pub up: V3,
}

/// A mesh's vertex attributes, four vertices per particle slot, as the
/// adapter borrows them from its mesh.
pub struct Verts<'a> {
    pub pos: &'a mut [[f32; 3]],
    pub uv: &'a mut [[f32; 2]],
    pub col: &'a mut [[f32; 4]],
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

    /// Start the rolls from `seed` (zero is the default seed): the fx stage
    /// shoots one effect under several seeds to see what is luck and what is
    /// the recipe.
    pub fn seed(&mut self, seed: u32) {
        self.rng = seed;
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
            p.vel[1] -= p.gravity * dt;
            p.vel = v3::scale(p.vel, (-p.drag * dt).exp());
            p.pos = v3::add(p.pos, v3::scale(p.vel, dt));
            p.roll += p.spin * dt;
            // One bounce off the surface it came from, losing most of its
            // speed — sparks skitter, they do not fall through the wall.
            if p.plane_n != v3::ZERO {
                let d = v3::dot(v3::sub(p.pos, p.plane_p), p.plane_n);
                if d < 0.0 {
                    p.pos = v3::sub(p.pos, v3::scale(p.plane_n, d));
                    let vn = v3::dot(p.vel, p.plane_n);
                    if vn < 0.0 {
                        p.vel = v3::scale(v3::sub(p.vel, v3::scale(p.plane_n, 1.35 * vn)), 0.45);
                    }
                    p.plane_n = v3::ZERO;
                }
            }
        }
    }

    /// Whether the mesh has to be written this frame: something is alive, or
    /// the last write still showed something that has since died.
    pub fn needs_write(&self) -> bool {
        self.drawn || self.parts.iter().any(|p| p.left > 0.0)
    }

    /// Move every live particle by `d`: the frame they are simulated in
    /// moved under them (a flame carried in a hand).
    pub fn shift(&mut self, d: V3) {
        for p in self.parts.iter_mut().filter(|p| p.left > 0.0) {
            p.pos = v3::add(p.pos, d);
        }
    }

    /// Forget every particle.
    pub fn clear(&mut self) {
        for p in self.parts.iter_mut() {
            p.left = 0.0;
        }
    }

    /// The four corners of particle `p` relative to `anchor`, with their uvs,
    /// and its colour — or `None` for a free slot.
    pub fn quad(p: &Particle, cam: &Cam, anchor: V3) -> Option<Quad> {
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
                let r = v3::add(v3::scale(cam.right, co), v3::scale(cam.up, s));
                let u = v3::sub(v3::scale(cam.up, co), v3::scale(cam.right, s));
                (v3::scale(r, size), v3::scale(u, size), p.pos)
            }
            Orient::Flat => {
                let (s, co) = p.roll.sin_cos();
                (
                    v3::scale([co, 0.0, s], size),
                    v3::scale([-s, 0.0, co], size),
                    p.pos,
                )
            }
            Orient::Stretch => {
                let view = v3::norm_or(v3::sub(p.pos, cam.pos), v3::Z);
                let dist = v3::len(v3::sub(p.pos, cam.pos));
                let across = v3::sub(p.vel, v3::scale(view, v3::dot(p.vel, view)));
                let max = if p.len_max > 0.0 {
                    p.len_max
                } else {
                    STREAK_MAX_M
                };
                let len = (v3::len(p.vel) * p.stretch).clamp(STREAK_MIN_M, max);
                let axis = v3::norm_or(across, cam.up);
                let side = v3::norm_or(v3::cross(view, axis), cam.right);
                let w = size.max(dist * MIN_PX);
                // The head leads; the tail trails back along the motion.
                (
                    v3::scale(axis, len * 0.5),
                    v3::scale(side, w),
                    v3::sub(p.pos, v3::scale(axis, len * 0.5)),
                )
            }
        };
        let o = v3::sub(centre, anchor);
        let pos = [
            v3::sub(v3::sub(o, a), b),
            v3::sub(v3::add(o, a), b),
            v3::add(v3::add(o, a), b),
            v3::add(v3::sub(o, a), b),
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

    /// Copy the pool into a mesh's attributes, in place, relative to
    /// `anchor` — or do nothing when it is empty and the mesh already is.
    /// Returns whether it wrote. No allocation; the slices hold four vertices
    /// per slot (`cap() * 4`), as [`indices`] numbers them.
    pub fn fill(&mut self, out: Verts, cam: &Cam, anchor: V3) -> bool {
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
                        v3::dist2(p.pos, cam.pos)
                    }
                };
                key(pj).total_cmp(&key(pi))
            });
        }
        let Verts { pos, uv, col } = out;
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

/// A pool mesh's fixed triangle list: two triangles per slot, four vertices
/// each, built once with the mesh.
pub fn indices(cap: usize) -> Vec<u32> {
    let mut idx = Vec::with_capacity(cap * 6);
    for q in 0..cap as u32 {
        let b = q * 4;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    idx
}
