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
    /// A true line in the world, from its head back along its velocity — a
    /// spark's streak — turned to face the camera, at least [`MIN_PX`] wide so
    /// it reads with bloom off. Seen head on it is a dot, not a line.
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
    /// A flipbook: this many cells from `cell`, row-major across the pool's
    /// [`Sheet`], played once over its life; 0 or 1 is a still sprite.
    pub frames: u8,
    /// A roll in `[0, 1)` at birth: a flipbook above a half is drawn
    /// mirrored, so two of the same cell don't read as twins.
    pub seed: f32,
    /// Which of the game's emissive ramps a baked sheet's heat takes (0 the
    /// first), and how bright, for the game's flipbook shader.
    pub tone: f32,
    pub glow: f32,
}

/// One particle's quad: four corners, their uvs, and its colour.
pub type Quad = ([[f32; 3]; 4], [[f32; 2]; 4], [f32; 4]);

/// What a flipbook's quad carries besides: the next frame's uvs, the
/// sprite's own right axis (unit, world: the game's shader lights a baked
/// sheet in the sprite's frame), and `[blend, age, tone, glow]` — how far
/// into the crossfade to that frame, how far through its life (0 born, 1
/// gone), its ramp and its brightness.
pub type Extra = ([[f32; 2]; 4], V3, [f32; 4]);

/// How a pool's sprite sheet is cut: `cols × rows` cells of `tex` texels a
/// side, numbered row-major from the top left. Up to 256 cells (`cell` is a
/// byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub cols: u32,
    pub rows: u32,
    pub tex: u32,
}

impl Sheet {
    /// The generated sheet's cut ([`crate::atlas`]): 4 × 4 of 64.
    pub const DEFAULT: Sheet = Sheet {
        cols: SPRITE_COLS,
        rows: SPRITE_ROWS,
        tex: SPRITE_TEX,
    };

    /// The uv of `(u, v) ∈ [0, 1]²` inside `cell`, inset half a texel so a
    /// cell's edge never bleeds its neighbour's.
    pub fn uv(&self, cell: u32, u: f32, v: f32) -> [f32; 2] {
        let cols = self.cols.max(1);
        let rows = self.rows.max(1);
        let cell = cell.min(cols * rows - 1);
        let (cu, cv) = ((cell % cols) as f32, (cell / cols) as f32);
        let inset = 0.5 / self.tex.max(1) as f32;
        [
            (cu + inset + u * (1.0 - 2.0 * inset)) / cols as f32,
            (cv + inset + v * (1.0 - 2.0 * inset)) / rows as f32,
        ]
    }

    /// The four corners' uvs of `cell`, in the quad's corner order.
    pub fn corners(&self, cell: u32) -> [[f32; 2]; 4] {
        [
            self.uv(cell, 0.0, 1.0),
            self.uv(cell, 1.0, 1.0),
            self.uv(cell, 1.0, 0.0),
            self.uv(cell, 0.0, 0.0),
        ]
    }
}

/// Where a flipbook of `frames` cells stands `age` (0..1) through its life:
/// the frame drawn, the frame it is fading into, and how far into that fade.
/// The last frame holds (no frame after it to fade to); a still sprite is
/// its one cell at a blend of 0.
pub fn frame_at(frames: u8, age: f32) -> (u32, u32, f32) {
    let n = frames.max(1) as u32;
    let f = age.clamp(0.0, 1.0) * n as f32;
    let i = (f as u32).min(n - 1);
    if i + 1 < n {
        (i, i + 1, f - i as f32)
    } else {
        (i, i, 0.0)
    }
}

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
    /// A flipbook pool's second uv set, its sprites' right axes (its
    /// normals) and its `[blend, age, tone, glow]` ([`Extra`]); `None` for
    /// a mesh that draws stills only.
    pub uv2: Option<&'a mut [[f32; 2]]>,
    pub nrm: Option<&'a mut [[f32; 3]]>,
    pub aux: Option<&'a mut [[f32; 4]]>,
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
    /// How its sprite sheet is cut.
    sheet: Sheet,
    /// Particles spawned, and live ones recycled for want of a slot.
    pub spawned: u64,
    pub stolen: u64,
}

impl Pool {
    /// A pool of `cap` particles over the generated sheet — allocated here,
    /// once, and never again.
    pub fn new(cap: usize, sorted: bool) -> Self {
        Self::with_sheet(cap, sorted, Sheet::DEFAULT)
    }

    /// A pool of `cap` particles over a sheet cut as `sheet` (a baked
    /// flipbook sheet).
    pub fn with_sheet(cap: usize, sorted: bool, sheet: Sheet) -> Self {
        Self {
            parts: vec![Particle::default(); cap],
            order: (0..cap as u16).collect(),
            next: 0,
            rng: 0,
            drawn: true,
            sorted,
            sheet,
            spawned: 0,
            stolen: 0,
        }
    }

    pub fn sheet(&self) -> Sheet {
        self.sheet
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

    /// The four corners of particle `p` relative to `anchor`, with their uvs
    /// on the generated sheet, and its colour — or `None` for a free slot.
    pub fn quad(p: &Particle, cam: &Cam, anchor: V3) -> Option<Quad> {
        Self::quad_on(p, cam, anchor, &Sheet::DEFAULT).map(|(q, _)| q)
    }

    /// [`Pool::quad`] on `sheet`, with the flipbook's [`Extra`]: the frame
    /// its age lands on, the next one's uvs and the blend between them.
    pub fn quad_on(p: &Particle, cam: &Cam, anchor: V3, sheet: &Sheet) -> Option<(Quad, Extra)> {
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
                let max = if p.len_max > 0.0 {
                    p.len_max
                } else {
                    STREAK_MAX_M
                };
                // A longest under the shortest is the shortest, not a panic.
                let len = (v3::len(p.vel) * p.stretch).clamp(STREAK_MIN_M, max.max(STREAK_MIN_M));
                let dir = v3::norm_or(p.vel, cam.up);
                let toward = v3::dot(dir, view);
                let across = v3::sub(dir, v3::scale(view, toward));
                let axis = v3::norm_or(across, cam.up);
                let side = v3::norm_or(v3::cross(view, axis), cam.right);
                let w = size.max(dist * MIN_PX);
                // The head leads; the tail trails back along the motion, in
                // depth too: one flying from the eye is short, not its whole
                // length sideways. Head on, at least `w` across, or the quad
                // is edge on to the eye and gone.
                let half = len * 0.5;
                (
                    v3::add(
                        v3::scale(view, toward * half),
                        v3::scale(axis, (v3::len(across) * half).max(w)),
                    ),
                    v3::scale(side, w),
                    v3::sub(p.pos, v3::scale(dir, half)),
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
        let (fa, fb, blend) = frame_at(p.frames, t);
        let mut uv = sheet.corners(p.cell as u32 + fa);
        let mut uv2 = sheet.corners(p.cell as u32 + fb);
        if p.frames > 1 && p.seed >= 0.5 {
            uv.swap(0, 1);
            uv.swap(2, 3);
            uv2.swap(0, 1);
            uv2.swap(2, 3);
        }
        let right = v3::norm_or(a, cam.right);
        Some(((pos, uv, c), (uv2, right, [blend, t, p.tone, p.glow])))
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
        let Verts {
            pos,
            uv,
            col,
            mut uv2,
            mut nrm,
            mut aux,
        } = out;
        for slot in 0..self.parts.len() {
            let i = if self.sorted {
                self.order[slot] as usize
            } else {
                slot
            };
            let base = slot * 4;
            match Self::quad_on(&self.parts[i], cam, anchor, &self.sheet) {
                Some(((p, t, c), (t2, r, a))) => {
                    for k in 0..4 {
                        pos[base + k] = p[k];
                        uv[base + k] = t[k];
                        col[base + k] = c;
                    }
                    if let Some(uv2) = uv2.as_deref_mut() {
                        uv2[base..base + 4].copy_from_slice(&t2);
                    }
                    if let Some(nrm) = nrm.as_deref_mut() {
                        for v in &mut nrm[base..base + 4] {
                            *v = r;
                        }
                    }
                    if let Some(aux) = aux.as_deref_mut() {
                        for v in &mut aux[base..base + 4] {
                            *v = a;
                        }
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
