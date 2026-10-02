//! Marks the world keeps: bullet holes, gashes, dents, blood and scorch.
//!
//! # One mesh, both builds
//!
//! Every mark is a small patch in ONE dynamic mesh drawn with ONE lit,
//! alpha-blended `StandardMaterial` over a generated atlas — one draw call on
//! the desktop and in the browser alike. Bevy's `ForwardDecal` cannot run on
//! WebGL2 (its 4-byte uniform is refused outright), and drawing marks two ways
//! meant the two builds looked different; the mesh path the browser proved is
//! now the only path. A patch is lifted [`MESH_MARK_LIFT_M`] off its surface
//! along the normal.
//!
//! # On the drawn surface, not the sim's
//!
//! A world contact arrives already moved onto the mesh it struck
//! (`impact::snap`), and a mark there is a [`MARK_GRID`]² grid whose every
//! vertex is projected onto that mesh ([`Marks::conform`]): it follows the
//! lumps of a rock and the taper of a trunk, and fades out over an edge it
//! cannot reach rather than hanging in the air past it. Off a mesh (the
//! ground, a wall) it is flat, or bent about the trunk's axis.
//!
//! # What a mark looks like
//!
//! Picked per weapon × matter ([`decal_kind`]), the way the reference picks an
//! impact effect: a bullet leaves a hole whose rim is the material's
//! (splintered wood, chipped stone with cracks, bright bare metal, a soil
//! crater), a blade a gash, a blow on metal a dent, a blast a scorch and a
//! body blood on the floor behind it. Every mark is rotated and scaled at
//! random, and fades out after its life. The atlas is grey value × alpha; the
//! matter's [`tint`] colours it, so one crater serves every soil. A second
//! atlas is each mark's relief as a normal map, so a gash is a groove the sun
//! lights one side of and a chip a pit, not a sticker.
//!
//! # A fixed pool
//!
//! [`MARKS`] slots, drop-oldest — a mark has no motion to interrupt and the
//! oldest is the faintest. The mesh is rewritten only when a mark is placed,
//! a fade step moves or the pool is forgotten; a free slot is a collapsed
//! patch, so the mesh never changes size.
//!
//! # The weak spot
//!
//! `EV_WEAK_MARK`'s cross on the node the sim marked is its own entity on the
//! same mesh-mark path: it pulses and moves every hit, which a pooled mark
//! does not.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::impact::{skin_radius, strike_height, Contacts, Matter, Weapon};
use super::mipmap;
use super::props::hash01;
use super::skin::{ray_mesh, Patch, Skin};
use super::{surface, Eye, WorldId};
use sim_core::gather::NO_CELL;
use sim_core::ranged::SURF_WORLD;
use sim_core::terrain::{self, Slot};
use sim_core::yaw_dir;

/// Marks drawable at once — a *view* cap: marks are not saved, replicated or
/// known to the sim. Each costs [`MARK_GRID`]² vertices of one shared mesh.
pub const MARKS: usize = 256;

/// How long a mark stays at full strength, seconds.
pub const LIFE_S: f32 = 45.0;

/// Blood's life, seconds — shorter: a fight's blood should not outlast it.
pub const BLOOD_LIFE_S: f32 = 30.0;

/// The fade-out tail, seconds, once a mark's life is up. A mark that vanished
/// on a frame boundary would read as a pop in the corner of the eye.
pub const FADE_S: f32 = 6.0;

/// A melee scuff's nominal footprint, metres; every other kind is sized
/// relative to what made it ([`decal_kind`]).
pub const SIZE_M: f32 = 0.22;

/// Marks further than this from the eye are not laid, metres — the reference
/// caps impact decals at 30 m; ours are cheaper and a firefight reads further.
pub const MARK_RANGE_M: f32 = 50.0;

/// How far a mark stands off its surface along the normal, metres — at zero
/// separation the depth test is a coin toss per pixel.
pub const MESH_MARK_LIFT_M: f32 = 0.01;

/// Segments across the weak spot's curved patch ([`curved_patch`]): eight
/// chords over a 30 cm arc on a 24 cm radius is a sagitta under a centimetre.
pub const MESH_MARK_SEGMENTS: u32 = 8;

/// Vertices across and up one mark — a grid, so a mark can follow a rock in
/// both directions.
pub const MARK_GRID: usize = 5;

/// Vertices per mark.
const VERTS_PER_MARK: usize = MARK_GRID * MARK_GRID;

/// The weak-spot cross's footprint, metres — a target read from where a swing
/// is taken, larger than a scuff.
pub const WEAK_MARK_SIZE_M: f32 = 0.30;

/// How fast the weak-spot cross breathes outside its sector, Hz.
pub const WEAK_MARK_PULSE_HZ: f32 = 1.4;

/// The cross's alpha at the bottom and top of the pulse; it holds at the top
/// while the player stands in the sector.
pub const WEAK_MARK_ALPHA_LO: f32 = 0.45;
pub const WEAK_MARK_ALPHA_HI: f32 = 0.95;

/// The weak-spot mask's side, texels.
const MARK_TEX: u32 = 64;

/// Alpha steps a fade is quantized to, so a fading mark rewrites the mesh 32
/// times over [`FADE_S`] and not every frame.
const ALPHA_STEPS: f32 = 32.0;

/// The atlas: [`ATLAS_COLS`] × [`ATLAS_ROWS`] cells of [`CELL_TEX`]² texels.
pub const ATLAS_COLS: u32 = 8;
pub const ATLAS_ROWS: u32 = 4;
pub const CELL_TEX: u32 = 192;

/// What a mark is — an atlas cell, or a run of variant cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    HoleSoil,
    HoleWood,
    HoleStone,
    HoleMetal,
    GashWood,
    ChipStone,
    DentMetal,
    ScuffSoil,
    ArrowHole,
    Scorch,
    Blood,
}

impl Kind {
    /// Every kind, for the generator and the gates.
    pub const ALL: [Kind; 11] = [
        Kind::HoleSoil,
        Kind::HoleWood,
        Kind::HoleStone,
        Kind::HoleMetal,
        Kind::GashWood,
        Kind::ChipStone,
        Kind::DentMetal,
        Kind::ScuffSoil,
        Kind::ArrowHole,
        Kind::Scorch,
        Kind::Blood,
    ];

    /// The first atlas cell and how many variants follow it.
    pub fn cells(self) -> (u32, u32) {
        match self {
            Kind::HoleSoil => (0, 2),
            Kind::HoleWood => (2, 2),
            Kind::HoleStone => (4, 2),
            Kind::HoleMetal => (6, 2),
            Kind::GashWood => (8, 2),
            Kind::ChipStone => (10, 2),
            Kind::DentMetal => (12, 2),
            Kind::ScuffSoil => (14, 2),
            Kind::ArrowHole => (16, 2),
            Kind::Scorch => (18, 2),
            Kind::Blood => (20, 4),
        }
    }

    fn life(self) -> f32 {
        match self {
            Kind::Blood => BLOOD_LIFE_S,
            _ => LIFE_S,
        }
    }
}

/// Which mark a blow leaves and how wide it is, metres — `None` for a blow
/// that leaves nothing (water, a bush). A bullet's hole is small and its rim
/// is the material's; a blade's gash and a blunt dent are the width of the
/// tool; blood is a splat; a blast is a scorch.
pub fn decal_kind(weapon: Weapon, matter: Matter) -> Option<(Kind, f32)> {
    use Matter::*;
    Some(match (weapon, matter) {
        (_, Water | Plant) => return None,
        (_, Flesh) => (Kind::Blood, 0.55),
        (Weapon::Blast, _) => (Kind::Scorch, 2.2),
        (Weapon::Melee, Wood) => (Kind::GashWood, SIZE_M * 1.2),
        (Weapon::Melee, Stone) => (Kind::ChipStone, SIZE_M),
        (Weapon::Melee, Metal) => (Kind::DentMetal, SIZE_M),
        (Weapon::Melee, Dirt | Sand | Grass) => (Kind::ScuffSoil, SIZE_M * 1.3),
        (Weapon::Arrow, _) => (Kind::ArrowHole, 0.16),
        (Weapon::Bullet, Wood) => (Kind::HoleWood, 0.14),
        (Weapon::Bullet, Stone) => (Kind::HoleStone, 0.14),
        (Weapon::Bullet, Metal) => (Kind::HoleMetal, 0.11),
        (Weapon::Bullet, Dirt | Sand | Grass) => (Kind::HoleSoil, 0.18),
    })
}

/// A mark's colour on this matter; the atlas carries the value, this the hue.
///
/// Pale where the blow exposes something pale (heartwood, fresh stone, bare
/// metal), dark where it turns something dark over (earth). Measured against
/// the surfaces they land on by `tests/decal.rs`.
pub fn tint(kind: Kind, matter: Matter) -> Color {
    match kind {
        Kind::Blood => return Color::srgb(0.40, 0.03, 0.03),
        Kind::Scorch => return Color::srgb(0.17, 0.15, 0.13),
        _ => {}
    }
    match matter {
        Matter::Wood => Color::srgb(0.82, 0.66, 0.46),
        Matter::Stone => Color::srgb(0.80, 0.78, 0.74),
        Matter::Metal => Color::srgb(0.84, 0.84, 0.86),
        Matter::Sand => Color::srgb(0.56, 0.48, 0.35),
        Matter::Grass => Color::srgb(0.27, 0.25, 0.16),
        Matter::Dirt | Matter::Plant | Matter::Water | Matter::Flesh => {
            Color::srgb(0.30, 0.23, 0.16)
        }
    }
}

/// One vertex of a mark: position, normal, uv, linear colour, tangent.
pub type MarkVertex = ([f32; 3], [f32; 3], [f32; 2], [f32; 4], [f32; 4]);

/// One grid vertex of a mark, on its surface (the lift is added at write).
#[derive(Clone, Copy)]
struct MarkVert {
    p: Vec3,
    n: Vec3,
    /// The world direction of the atlas's +u here.
    t: Vec3,
    /// The tangent's handedness, Bevy's `w`.
    w: f32,
    /// The cell-local uv, 0..1.
    uv: Vec2,
    /// How much of the mark shows here: 0 past an edge it could not reach.
    a: f32,
}

impl Default for MarkVert {
    fn default() -> Self {
        Self {
            p: Vec3::ZERO,
            n: Vec3::Y,
            t: Vec3::X,
            w: 1.0,
            uv: Vec2::ZERO,
            a: 1.0,
        }
    }
}

/// One pooled mark.
#[derive(Clone, Copy)]
struct Mark {
    /// Seconds left through life and then [`FADE_S`]; zero is a free slot.
    left: f32,
    /// The alpha step last written, so a fade rewrites only when it moves.
    step: i32,
    /// The centre, for [`Marks::forget_near`].
    pos: Vec3,
    /// The mark's own normal and size, which [`Marks::conform`] casts along.
    normal: Vec3,
    size: f32,
    cell: u32,
    /// Linear tint.
    tint: [f32; 3],
    grid: [MarkVert; VERTS_PER_MARK],
}

impl Default for Mark {
    fn default() -> Self {
        Self {
            left: 0.0,
            step: -1,
            pos: Vec3::ZERO,
            normal: Vec3::Y,
            size: 0.0,
            cell: 0,
            tint: [1.0; 3],
            grid: [MarkVert::default(); VERTS_PER_MARK],
        }
    }
}

/// A mark's grid, laid flat or bent about a trunk of radius `bend_r`: the
/// patch's +u along `tangent`, its +v up the trunk (bent) or along
/// `normal × tangent` (flat), and the atlas cell turned by `flip`'s dihedral
/// bits (bit 0 swaps u/v, bits 1–2 mirror them).
fn grid_of(
    at: Vec3,
    normal: Vec3,
    tangent: Vec3,
    size: f32,
    bend_r: f32,
    flip: u8,
) -> [MarkVert; VERTS_PER_MARK] {
    let mut out = [MarkVert::default(); VERTS_PER_MARK];
    let b = normal.cross(tangent);
    let last = (MARK_GRID - 1) as f32;
    for j in 0..MARK_GRID {
        for i in 0..MARK_GRID {
            let (u, v) = (i as f32 / last, j as f32 / last);
            let (off, n, du) = if bend_r > 0.0 {
                let theta = (u - 0.5) * size / bend_r;
                let (s, c) = theta.sin_cos();
                (
                    tangent * (bend_r * s) + normal * (bend_r * (c - 1.0)),
                    tangent * s + normal * c,
                    tangent * c - normal * s,
                )
            } else {
                (tangent * ((u - 0.5) * size), normal, tangent)
            };
            let dv = if bend_r > 0.0 { Vec3::Y } else { b };
            let p = at + off + dv * ((v - 0.5) * size);
            let (mut tu, mut tv, mut au, mut av) = (u, v, du, dv);
            if flip & 1 != 0 {
                std::mem::swap(&mut tu, &mut tv);
                std::mem::swap(&mut au, &mut av);
            }
            if flip & 2 != 0 {
                tu = 1.0 - tu;
                au = -au;
            }
            if flip & 4 != 0 {
                tv = 1.0 - tv;
                av = -av;
            }
            // Bevy's bitangent is `cross(n, t) · w` and points toward −v.
            let w = if n.cross(au).dot(av) > 0.0 { -1.0 } else { 1.0 };
            out[j * MARK_GRID + i] = MarkVert {
                p,
                n,
                t: au,
                w,
                uv: Vec2::new(tu, tv),
                a: 1.0,
            };
        }
    }
    out
}

/// Project a grid onto the surface `cast` answers — `cast(origin, dir, max)`
/// is a ray against one drawn mesh — vertex by vertex, along `normal`. A
/// vertex the surface does not meet within `reach`, or meets turned away,
/// fades to nothing: the mark ends at an edge rather than hanging past it.
fn conform_grid(
    grid: &mut [MarkVert; VERTS_PER_MARK],
    normal: Vec3,
    reach: f32,
    cast: &mut dyn FnMut(Vec3, Vec3, f32) -> Option<(Vec3, Vec3)>,
) {
    for v in grid.iter_mut() {
        let o = v.p + normal * reach;
        match cast(o, -normal, reach * 2.0) {
            Some((at, n)) => {
                let facing = n.dot(normal);
                v.a *= smooth(0.15, 0.45, facing);
                v.p = at;
                v.n = n;
                let t = v.t - n * n.dot(v.t);
                v.t = t.normalize_or(v.t);
            }
            None => v.a = 0.0,
        }
    }
}

/// How far a mark's vertices are searched for its surface, metres: past the
/// lumps of a rock and the taper of a trunk, not as far as the next face.
fn conform_reach(size: f32) -> f32 {
    (size * 0.3).clamp(0.05, 0.25)
}

/// The pool: every mark's state, and the handles of the one mesh that draws
/// them. On the heap: [`MARKS`] grids is too big for a browser's stack.
#[derive(Resource)]
pub struct Marks {
    slots: Vec<Mark>,
    mesh: Handle<Mesh>,
    /// The oldest slot to recycle when every one is busy — a rotating hand.
    hand: usize,
    /// Whether the mesh no longer matches the slots.
    dirty: bool,
    rng: u32,
    /// Marks laid since the world was entered, and live marks recycled.
    pub placed: u64,
    pub stolen: u64,
    /// The weak-spot cross: its entity, mesh, material, the alpha step last
    /// written and the cell and heading it was last placed for.
    weak: Option<Entity>,
    weak_mesh: Handle<Mesh>,
    weak_material: Handle<StandardMaterial>,
    weak_step: i32,
    weak_cell: u32,
    weak_mark8: u8,
    /// Clears asked for this frame ([`Marks::forget_later`]), applied by
    /// [`fade`] — which runs after [`mark`], so a killing blow's own mark,
    /// laid this same frame, goes with the thing it hit whichever system
    /// ran first.
    pending: [(Vec3, f32); FORGET_QUEUE],
    n_pending: usize,
    /// The triangles a mark is being fitted to, reused.
    patch: Patch,
}

/// Deferred clears one frame can queue: a full collapse's removals
/// (`client_core::core::REMOVED_RING`) with room for the trunks, nodes and
/// doors of the same frame. Past it a clear runs at once rather than never.
const FORGET_QUEUE: usize = client_core::core::REMOVED_RING + 32;

impl Default for Marks {
    fn default() -> Self {
        Self {
            slots: vec![Mark::default(); MARKS],
            mesh: Handle::default(),
            hand: 0,
            dirty: false,
            rng: 0,
            placed: 0,
            stolen: 0,
            weak: None,
            weak_mesh: Handle::default(),
            weak_material: Handle::default(),
            weak_step: -1,
            weak_cell: NO_CELL,
            weak_mark8: 0,
            pending: [(Vec3::ZERO, 0.0); FORGET_QUEUE],
            n_pending: 0,
            patch: Patch::default(),
        }
    }
}

impl Marks {
    /// A free slot, or the oldest live one — drop-oldest.
    fn claim(&mut self) -> usize {
        if let Some(ix) = self.slots.iter().position(|m| m.left <= 0.0) {
            return ix;
        }
        self.stolen += 1;
        let ix = self.hand;
        self.hand = (self.hand + 1) % MARKS;
        ix
    }

    /// One roll in `[0, 1)` — a 32-bit xorshift, seeded on first use.
    fn roll(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x2545_F491;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1 << 24) as f32
    }

    /// How many marks are drawing.
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|m| m.left > 0.0).count()
    }

    /// Lay one mark: rotated and scaled at random, a random variant of its
    /// kind. `bend_r` is the trunk radius for a mark on a standing thing, or
    /// zero. Pure — no `World`, no assets.
    pub fn place(
        &mut self,
        at: Vec3,
        normal: Vec3,
        kind: Kind,
        size: f32,
        matter: Matter,
        bend_r: f32,
    ) -> usize {
        let n = normal.normalize_or(Vec3::Y);
        let (first, variants) = kind.cells();
        let cell = first + ((self.roll() * variants as f32) as u32).min(variants - 1);
        let size = size * (0.8 + 0.4 * self.roll());
        let (tangent, flip) = if bend_r > 0.0 {
            // On a trunk the grain runs up it, so the cell keeps its own
            // up and only mirrors — a gash across a trunk stays across it.
            (
                Vec3::Y.cross(n).normalize_or(Vec3::X),
                (self.roll() * 4.0) as u8 * 2,
            )
        } else {
            let base = if n.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
            let t = base.cross(n).normalize_or(Vec3::X);
            let spin = Quat::from_axis_angle(n, self.roll() * std::f32::consts::TAU);
            (spin * t, 0)
        };
        // A little brightness jitter so a row of holes is not one sticker.
        let lin = tint(kind, matter).to_linear();
        let j = 0.88 + 0.24 * self.roll();
        let ix = self.claim();
        self.slots[ix] = Mark {
            left: kind.life() + FADE_S,
            step: -1,
            pos: at,
            normal: n,
            size,
            cell,
            tint: [lin.red * j, lin.green * j, lin.blue * j],
            grid: grid_of(at, n, tangent, size, bend_r, flip),
        };
        self.placed += 1;
        self.dirty = true;
        ix
    }

    /// Fit slot `ix` to the surface `cast` answers (see `conform_grid`).
    pub fn conform(
        &mut self,
        ix: usize,
        cast: &mut dyn FnMut(Vec3, Vec3, f32) -> Option<(Vec3, Vec3)>,
    ) {
        let Some(m) = self.slots.get_mut(ix) else {
            return;
        };
        if m.left <= 0.0 {
            return;
        }
        let (normal, reach) = (m.normal, conform_reach(m.size));
        conform_grid(&mut m.grid, normal, reach, cast);
        self.dirty = true;
    }

    /// Drop every mark within `r` of `at` — the wall or the trunk it was on
    /// has gone, and a hole must not hang in the air where it stood.
    /// Returns how many went.
    pub fn forget_near(&mut self, at: Vec3, r: f32) -> usize {
        let mut n = 0;
        for m in self.slots.iter_mut() {
            if m.left > 0.0 && m.pos.distance_squared(at) <= r * r {
                *m = Mark::default();
                n += 1;
            }
        }
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    /// [`Marks::forget_near`], at the end of this frame's marking: the
    /// surface is gone (a wall down, a trunk falling, a node mined out, a
    /// door swung open), and a mark laid on it this same frame must go too.
    pub fn forget_later(&mut self, at: Vec3, r: f32) {
        if self.n_pending == FORGET_QUEUE {
            self.forget_near(at, r);
            return;
        }
        self.pending[self.n_pending] = (at, r);
        self.n_pending += 1;
    }

    /// Run every clear [`Marks::forget_later`] queued. Returns how many
    /// marks went.
    pub fn forget_pending(&mut self) -> usize {
        let mut n = 0;
        for i in 0..self.n_pending {
            let (at, r) = self.pending[i];
            n += self.forget_near(at, r);
        }
        self.n_pending = 0;
        n
    }

    /// Age every mark by `dt`: release the dead, step the fading. Returns
    /// whether the mesh needs rewriting.
    pub fn age(&mut self, dt: f32) -> bool {
        for m in self.slots.iter_mut() {
            if m.left <= 0.0 {
                continue;
            }
            m.left -= dt;
            if m.left <= 0.0 {
                *m = Mark::default();
                self.dirty = true;
                continue;
            }
            let step = alpha_step(m.left);
            if step != m.step {
                m.step = step;
                self.dirty = true;
            }
        }
        std::mem::take(&mut self.dirty)
    }

    /// The vertices of slot `ix` — a collapsed patch for a free slot.
    pub fn vertices(&self, ix: usize) -> [MarkVertex; VERTS_PER_MARK] {
        let mut out = [(
            [0.0; 3],
            [0.0, 1.0, 0.0],
            [0.0; 2],
            [0.0; 4],
            [1.0, 0.0, 0.0, 1.0],
        ); VERTS_PER_MARK];
        let Some(m) = self.slots.get(ix) else {
            return out;
        };
        if m.left <= 0.0 {
            return out;
        }
        let a = (alpha_step(m.left) as f32 / ALPHA_STEPS).clamp(0.0, 1.0);
        let (cu, cv) = (m.cell % ATLAS_COLS, m.cell / ATLAS_COLS);
        // Half a texel in from the cell's edge so a mip never reads the
        // neighbour.
        let inset = 0.5 / CELL_TEX as f32;
        for (k, g) in m.grid.iter().enumerate() {
            let p = g.p + g.n * MESH_MARK_LIFT_M;
            let uv = [
                (cu as f32 + inset + g.uv.x * (1.0 - 2.0 * inset)) / ATLAS_COLS as f32,
                (cv as f32 + inset + g.uv.y * (1.0 - 2.0 * inset)) / ATLAS_ROWS as f32,
            ];
            out[k] = (
                p.to_array(),
                g.n.to_array(),
                uv,
                [m.tint[0], m.tint[1], m.tint[2], a * g.a],
                [g.t.x, g.t.y, g.t.z, g.w],
            );
        }
        out
    }

    /// Copy every slot into the mesh's attributes, in place — no allocation.
    fn write(&self, mesh: &mut Mesh) {
        let (mut pos, mut nrm, mut uv, mut col, mut tan) = (None, None, None, None, None);
        for (attr, values) in mesh.attributes_mut() {
            let id = attr.id;
            match values {
                VertexAttributeValues::Float32x3(v) => {
                    if id == Mesh::ATTRIBUTE_POSITION.id {
                        pos = Some(v);
                    } else if id == Mesh::ATTRIBUTE_NORMAL.id {
                        nrm = Some(v);
                    }
                }
                VertexAttributeValues::Float32x2(v) if id == Mesh::ATTRIBUTE_UV_0.id => {
                    uv = Some(v);
                }
                VertexAttributeValues::Float32x4(v) => {
                    if id == Mesh::ATTRIBUTE_COLOR.id {
                        col = Some(v);
                    } else if id == Mesh::ATTRIBUTE_TANGENT.id {
                        tan = Some(v);
                    }
                }
                _ => {}
            }
        }
        let (Some(pos), Some(nrm), Some(uv), Some(col), Some(tan)) = (pos, nrm, uv, col, tan)
        else {
            return;
        };
        for ix in 0..MARKS {
            for (k, (p, n, t, c, g)) in self.vertices(ix).into_iter().enumerate() {
                let i = ix * VERTS_PER_MARK + k;
                pos[i] = p;
                nrm[i] = n;
                uv[i] = t;
                col[i] = c;
                tan[i] = g;
            }
        }
    }
}

/// The quantized alpha step of a mark with `left` seconds to go.
fn alpha_step(left: f32) -> i32 {
    ((left / FADE_S).clamp(0.0, 1.0) * ALPHA_STEPS) as i32
}

/// Drop every mark, because the world they were made in has gone. The mesh
/// catches up on the next frame's [`fade`].
pub fn forget_in(commands: &mut Commands, pool: &mut Marks) {
    if let Some(e) = pool.weak {
        commands.entity(e).insert(Visibility::Hidden);
    }
    for m in pool.slots.iter_mut() {
        *m = Mark::default();
    }
    pool.hand = 0;
    pool.dirty = true;
    pool.weak_cell = NO_CELL;
    pool.weak_step = -1;
}

/// The mark mesh's entity.
#[derive(Component)]
pub struct MarkMesh;

/// The weak-spot cross's entity.
#[derive(Component)]
pub struct WeakSpot;

/// `marks` collapsed grids and their fixed indices.
fn grid_mesh(marks: usize) -> Mesh {
    let n = marks * VERTS_PER_MARK;
    let g = MARK_GRID as u32;
    let mut idx = Vec::with_capacity(marks * (MARK_GRID - 1) * (MARK_GRID - 1) * 6);
    for m in 0..marks as u32 {
        let base = m * VERTS_PER_MARK as u32;
        for j in 0..g - 1 {
            for i in 0..g - 1 {
                // Counter-clockwise seen from the normal's side: +u × +v is
                // the normal (`grid_of`), so the front face is the one shown.
                let a = base + j * g + i;
                let (b, c, d) = (a + 1, a + g, a + g + 1);
                idx.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0f32, 0.0, 0.0, 1.0]; n]);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// The empty mark mesh: [`MARKS`] collapsed grids and their fixed indices.
pub fn mark_mesh() -> Mesh {
    grid_mesh(MARKS)
}

/// Spawn the mark mesh and the weak-spot cross. The mesh is always drawn —
/// its patches collapsed until used — so its pipeline compiles at load and
/// not on the first shot of a fight.
pub fn setup(
    mut commands: Commands,
    mut pool: ResMut<Marks>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let (albedo, relief) = atlas_images();
    let atlas = images.add(albedo);
    let relief = images.add(relief);
    let cross = images.add(cross_texture());
    pool.mesh = meshes.add(mark_mesh());
    let material = standard.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas),
        normal_map_texture: Some(relief),
        // A mark is dirt, splinters and soot — never a highlight the surface
        // under it did not have.
        perceptual_roughness: 0.9,
        metallic: 0.0,
        reflectance: super::fresnel::DIELECTRIC,
        alpha_mode: AlphaMode::Blend,
        // Not `double_sided`: that flips the normal on a back face, and a
        // patch whose winding a conform bent is still lit by the normal it
        // was given. Both faces are drawn all the same.
        double_sided: false,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        MarkMesh,
        Mesh3d(pool.mesh.clone()),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Visible,
        NoFrustumCulling,
        NotShadowCaster,
    ));

    pool.weak_mesh = meshes.add(grid_mesh(1));
    let weak_mat = standard.add(StandardMaterial {
        base_color: WEAK_TINT.with_alpha(0.0),
        base_color_texture: Some(cross),
        perceptual_roughness: 0.95,
        metallic: 0.0,
        alpha_mode: AlphaMode::Blend,
        double_sided: false,
        cull_mode: None,
        ..default()
    });
    pool.weak_material = weak_mat.clone();
    pool.weak = Some(
        commands
            .spawn((
                WeakSpot,
                Mesh3d(pool.weak_mesh.clone()),
                MeshMaterial3d(weak_mat),
                Transform::IDENTITY,
                Visibility::Hidden,
                NoFrustumCulling,
                NotShadowCaster,
            ))
            .id(),
    );
}

/// The curved patch: a unit quad bent about its local **Z** axis to a
/// cylinder of radius `r` (mesh units), so laid on a trunk with local Z up the
/// trunk it follows the bark. Local +Y is the outward normal at the centre.
pub fn curved_patch(r: f32, segments: u32) -> Mesh {
    let n = segments.max(1) as usize;
    let mut pos = Vec::with_capacity((n + 1) * 2);
    let mut nrm = Vec::with_capacity((n + 1) * 2);
    let mut uv = Vec::with_capacity((n + 1) * 2);
    for i in 0..=n {
        let u = i as f32 / n as f32;
        let theta = (u - 0.5) / r;
        let (sin, cos) = theta.sin_cos();
        let x = r * sin;
        let y = r * (cos - 1.0);
        for (z, v) in [(-0.5f32, 1.0f32), (0.5, 0.0)] {
            pos.push([x, y, z]);
            nrm.push([sin, cos, 0.0]);
            uv.push([u, v]);
        }
    }
    let mut idx = Vec::with_capacity(n * 6);
    for i in 0..n as u32 {
        let a = i * 2;
        let (b, c, d) = (a + 1, a + 2, a + 3);
        idx.extend_from_slice(&[a, b, c, b, d, c]);
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m
}

/// Which mesh a curved patch on `surf` takes and the rotation that lays it:
/// a world hit is a trunk, so it is turned with local Z up the trunk's axis.
pub fn mesh_pose(surf: u8, normal: Vec3) -> (bool, Quat) {
    if surf == SURF_WORLD && normal.y.abs() < 0.99 {
        let n = normal.with_y(0.0).normalize_or(Vec3::Z);
        let x = n.cross(Vec3::Y).normalize_or(Vec3::X);
        (true, Quat::from_mat3(&Mat3::from_cols(x, n, Vec3::Y)))
    } else {
        (false, Quat::from_rotation_arc(Vec3::Y, normal))
    }
}

/// A ray against one drawn skin, as `conform_grid` asks for it.
fn skin_cast<'a>(
    mesh: &'a Mesh,
    tf: &'a GlobalTransform,
) -> impl FnMut(Vec3, Vec3, f32) -> Option<(Vec3, Vec3)> + 'a {
    move |o, d, max| ray_mesh(mesh, tf, o, d, max).map(|h| (h.at, h.normal))
}

/// Lay a mark for every contact the frame resolved that leaves one.
///
/// Reads `Res<Contacts>` — the one resolution of where and what — so the
/// mark, the debris and the sound of a blow cannot disagree about it.
pub fn mark(
    mut pool: ResMut<Marks>,
    contacts: Res<Contacts>,
    world: Option<Res<WorldId>>,
    net: Option<NonSend<super::Net>>,
    eye: Res<Eye>,
    meshes: Res<Assets<Mesh>>,
    skins: Query<(&Skin, &Mesh3d, &GlobalTransform)>,
) {
    let (Some(world), Some(net)) = (world, net) else {
        return;
    };
    let cols = net.session.core.pieces.cols();
    for c in contacts.iter().filter(|c| c.mark) {
        if c.at.distance_squared(eye.pos) > MARK_RANGE_M * MARK_RANGE_M {
            continue;
        }
        let Some((kind, size)) = decal_kind(c.weapon, c.matter) else {
            continue;
        };
        if matches!(kind, Kind::Blood | Kind::Scorch) {
            // Blood behind the victim, a scorch under the charge — both on
            // whatever is underfoot there.
            let p = if kind == Kind::Blood {
                let back = (-c.normal).with_y(0.0).normalize_or(Vec3::X);
                c.at + back * (0.2 + 0.5 * pool.roll())
            } else {
                c.at
            };
            let y = surface::floor_below(&world, cols, p);
            if y <= terrain::SEA_LEVEL {
                continue;
            }
            pool.place(Vec3::new(p.x, y, p.z), Vec3::Y, kind, size, c.matter, 0.0);
            continue;
        }
        // On a drawn skin: bent about a trunk's own axis at the radius it was
        // struck at, then fitted to the mesh vertex by vertex.
        let skin = c
            .skin
            .and_then(|e| skins.get(e).ok())
            .and_then(|(s, m, tf)| meshes.get(&m.0).map(|m| (s, m, tf)));
        if let Some((s, mesh, tf)) = skin {
            let bend_r = if s.tree && c.normal.y.abs() < 0.5 {
                let o = tf.translation();
                Vec2::new(c.at.x - o.x, c.at.z - o.z).length()
            } else {
                0.0
            };
            let ix = pool.place(c.at, c.normal, kind, size, c.matter, bend_r);
            let mut patch = std::mem::take(&mut pool.patch);
            patch.gather(mesh, tf, c.at, size * 0.9 + conform_reach(size) * 2.0);
            pool.conform(ix, &mut |o, d, max| patch.cast(o, d, max));
            pool.patch = patch;
            continue;
        }
        // A mark on a standing thing bends to its skin.
        let bend_r = if c.surf == SURF_WORLD && c.normal.y.abs() < 0.5 {
            surface::world_slot(&world, c.at)
                .map_or(0.0, |s| skin_radius(s.occupant as u8) * s.scale)
        } else {
            0.0
        };
        pool.place(c.at, c.normal, kind, size, c.matter, bend_r);
    }
}

/// Age every mark and, when anything visible changed, rewrite the mesh.
pub fn fade(mut pool: ResMut<Marks>, time: Res<Time>, mut meshes: ResMut<Assets<Mesh>>) {
    pool.forget_pending();
    if !pool.age(time.delta_secs()) {
        return;
    }
    if let Some(mesh) = meshes.get_mut(&pool.mesh) {
        pool.write(mesh);
    }
}

// ─── The atlas ───────────────────────────────────────────────────────────────

/// Lattice hash in `[0, 1)`.
fn lattice(x: i32, y: i32, seed: u32) -> f32 {
    hash01(
        (x as u32).wrapping_mul(0x9E37_79B1) ^ seed,
        (y as u32) ^ seed.rotate_left(16),
    )
}

/// Smooth value noise in `[0, 1)`.
fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (xf as i32, yf as i32);
    let a = lattice(xi, yi, seed);
    let b = lattice(xi + 1, yi, seed);
    let c = lattice(xi, yi + 1, seed);
    let d = lattice(xi + 1, yi + 1, seed);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sy
}

/// Three octaves of [`vnoise`], still in `[0, 1)`.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    vnoise(x, y, seed) * 0.5
        + vnoise(x * 2.03, y * 2.03, seed ^ 0x51) * 0.3
        + vnoise(x * 4.1, y * 4.1, seed ^ 0xA7) * 0.2
}

/// Noise around a circle, so a ragged rim closes on itself.
fn around(a: f32, k: f32, seed: u32) -> f32 {
    vnoise(a.cos() * k + 7.0, a.sin() * k + 7.0, seed)
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A thin line's coverage: 1 on it, 0 past `w`.
fn line(d: f32, w: f32) -> f32 {
    smooth(w, w * 0.35, d.abs())
}

/// Radial rays: coverage of `n` jittered spokes whose length is rolled per
/// spoke and stretched along `grain` (1 = along ±u), tapering to their tips.
fn spokes(u: f32, v: f32, n: f32, len: f32, width: f32, grain: f32, seed: u32) -> f32 {
    let r = (u * u + v * v).sqrt();
    let a = v.atan2(u);
    let t = (a / std::f32::consts::TAU + 0.5) * n;
    let id = t.floor();
    let f = t - id;
    let jit = hash01(id as u32, seed);
    let l = len
        * (0.45 + 0.55 * hash01(id as u32, seed ^ 0x33))
        * (1.0 - grain + grain * a.cos().abs());
    if r >= l {
        return 0.0;
    }
    let w = width * (1.0 - r / l);
    let centre = 0.3 + 0.4 * jit;
    smooth(w, w * 0.4, (f - centre).abs() * r.max(0.05) * 6.0)
}

/// Value (grey, sRGB 0..1), alpha and relief of cell `cell` at
/// `(u, v) ∈ [-1, 1]²`. Relief is a height in cell widths — 0.05 on a 26 cm
/// gash is 1.3 cm deep — and becomes the normal atlas.
fn texel(cell: u32, u: f32, v: f32) -> (f32, f32, f32) {
    let seed = 0x5EED_0000 ^ cell.wrapping_mul(0x0101_0101);
    let r = (u * u + v * v).sqrt();
    let a = v.atan2(u);
    let n = fbm(u * 5.0, v * 5.0, seed);
    let (value, alpha, height) = match cell {
        // A soil crater: a dark pit, the slope lighter, clods thrown out.
        0 | 1 => {
            let rn = r / (0.6 + 0.3 * (around(a, 1.6, seed) - 0.5));
            let pit = smooth(0.4, 0.0, rn);
            let body = 1.0 - smooth(0.5, 0.95, rn);
            let clods = smooth(0.62, 0.74, fbm(u * 9.0 + 3.0, v * 9.0, seed ^ 7))
                * smooth(0.6, 0.85, rn)
                * (1.0 - smooth(0.95, 1.35, rn));
            let rim = smooth(0.3, 0.5, rn) * (1.0 - smooth(0.5, 0.85, rn));
            (
                (0.7 - 0.55 * pit + 0.25 * (n - 0.5)).clamp(0.05, 1.0),
                (body * (0.7 + 0.3 * n)).max(clods * 0.9),
                -0.06 * (1.0 - smooth(0.0, 0.5, rn)) + 0.012 * rim + 0.015 * clods,
            )
        }
        // Wood: a dark bore, a ring of torn pale fibre, splinters along the
        // grain.
        2 | 3 => {
            let hole_r = 0.11 + 0.04 * around(a, 2.0, seed);
            let hole = smooth(hole_r + 0.03, hole_r - 0.02, r);
            let torn = 1.0 - smooth(0.18, 0.34, r / (0.8 + 0.4 * around(a, 3.0, seed ^ 1)));
            let splinter = spokes(u, v, 13.0, 0.95, 0.5, 0.7, seed);
            let fibre = 0.82 + 0.18 * vnoise(u * 3.0, v * 28.0, seed ^ 9);
            (
                fibre * (1.0 - hole) + 0.07 * hole,
                hole.max(torn * 0.9).max(splinter * 0.95),
                -0.08 * hole + 0.01 * splinter + 0.006 * torn * (fibre - 0.82) / 0.18,
            )
        }
        // Stone: a dark pit in a pale chipped crater, with cracks running out.
        4 | 5 => {
            let pit = smooth(0.14, 0.07, r / (0.8 + 0.4 * around(a, 2.5, seed)));
            let chip = 1.0 - smooth(0.26, 0.42, r / (0.75 + 0.5 * around(a, 2.0, seed ^ 2)));
            let cracks = spokes(u, v, 7.0, 0.95, 0.12, 0.0, seed ^ 5);
            let dust = (1.0 - smooth(0.3, 0.75, r)) * 0.3;
            let crack_v = 0.22;
            let base = 0.9 - 0.2 * n;
            let value = if pit > 0.5 {
                0.08
            } else if cracks > chip {
                crack_v
            } else {
                base
            };
            (
                value,
                pit.max(chip * 0.95).max(cracks * 0.9).max(dust),
                -0.07 * pit - 0.025 * chip * (1.0 - pit) - 0.006 * cracks,
            )
        }
        // Metal: a dark centre, a bright ring of bare metal, a scuffed halo.
        6 | 7 => {
            let hole = smooth(0.1, 0.06, r);
            let rim = 1.0 - smooth(0.17, 0.24, r / (0.9 + 0.2 * around(a, 3.0, seed)));
            let scratch = vnoise(a * 9.0, r * 2.0, seed ^ 4);
            let halo = (1.0 - smooth(0.24, 0.58, r)) * (0.35 + 0.65 * scratch);
            let value = if hole > 0.5 { 0.05 } else { 0.8 + 0.2 * rim };
            (
                value,
                hole.max(rim).max(halo * 0.75),
                -0.06 * hole + 0.012 * rim * (1.0 - hole) - 0.02 * (1.0 - smooth(0.0, 0.5, r)),
            )
        }
        // A chop across the grain: a notch whose upper face is the steep one,
        // pale fresh wood shadowed toward the groove, a lip of crushed bark,
        // and splinters lifting off the lips along the grain (+v is up the
        // trunk, so the grain runs along v and the cut along u).
        8 | 9 => {
            // The bit went in at one end and tore out at the other: blunt
            // where it entered, tapered where it left, ragged all round.
            let len = 0.86;
            let ue = u / len;
            let lean = 1.0 + 0.35 * ue;
            let span = ((1.0 - ue * ue) * lean).max(0.0);
            let rag = 0.75
                + 0.25 * vnoise(u * 4.0, 1.7, seed)
                + 0.2 * vnoise(u * 17.0, v * 3.0, seed ^ 0x2b);
            let w = 0.19 * span.sqrt() * span * rag + 1e-4;
            let d = v - 0.05 * (vnoise(u * 2.5, 0.5, seed) - 0.5);
            let ad = d.abs();
            let ends = 1.0 - smooth(0.8, 0.92, u.abs());
            let inside = smooth(w, w * 0.8, ad) * ends;
            let face = (1.0 - ad / w).clamp(0.0, 1.0);
            let upper = d < 0.0;
            let depth = 0.075 * span.min(1.0);
            let h_cut = -depth * face.powf(if upper { 0.6 } else { 1.5 });
            // Torn bark round the cut: a dark crushed edge, then ragged
            // flakes of the paler inner bark lifted off it.
            let tear = fbm(u * 7.0, v * 7.0, seed ^ 0x51);
            let edge = smooth(w * 1.45, w * 1.05, ad) * (1.0 - inside) * ends;
            let flakes = smooth(w * (1.6 + 1.6 * tear), w * 1.2, ad)
                * smooth(0.45, 0.6, tear)
                * (1.0 - inside)
                * ends;
            let mut spl: f32 = 0.0;
            for k in 0..12u32 {
                let su = (hash01(k, seed) - 0.5) * 1.4;
                let se = su / len;
                let sspan = ((1.0 - se * se) * (1.0 + 0.35 * se)).max(0.0);
                let sw = 0.19 * sspan.sqrt() * sspan;
                let side = if hash01(k, seed ^ 0x5) < 0.5 {
                    -1.0
                } else {
                    1.0
                };
                let l = 0.05 + 0.17 * hash01(k, seed ^ 0x9);
                let wid = 0.01 + 0.012 * hash01(k, seed ^ 0x13);
                let tilt = (hash01(k, seed ^ 0x21) - 0.5) * 0.6;
                let out = (v * side) - sw * 0.85;
                if out < 0.0 || out > l {
                    continue;
                }
                let taper = 1.0 - out / l;
                let bow = tilt * out + 0.6 * tilt * out * out / l;
                spl = spl.max(line(u - su - bow, wid * taper + 1e-4));
            }
            let chips = smooth(0.74, 0.8, fbm(u * 10.0, v * 10.0, seed ^ 3))
                * (1.0 - smooth(0.22, 0.5, ad))
                * ends
                * (1.0 - inside);
            // Fresh wood, fibre along the grain; the overhanging face is in
            // its own shadow and the groove darkest.
            let fibre = 0.74 + 0.26 * vnoise(u * 34.0, v * 2.5, seed ^ 9);
            let shade = if upper { 0.55 } else { 1.0 };
            let cut_v = fibre * shade * (1.0 - 0.6 * face.powi(4));
            let mut out_v = 0.2 + 0.1 * tear;
            out_v += (0.55 - out_v) * flakes;
            out_v += (0.9 - out_v) * spl.max(chips);
            (
                out_v + (cut_v - out_v) * inside,
                inside
                    .max(edge * 0.9)
                    .max(flakes * 0.85)
                    .max(spl * 0.95)
                    .max(chips * 0.75),
                h_cut * inside + 0.008 * edge + 0.012 * flakes + 0.01 * spl + 0.004 * chips,
            )
        }
        // A pick on stone: a shallow conchoidal scar — rippled, faceted, a
        // crushed white heart — with cracks running out past it and flakes
        // thrown round it.
        10 | 11 => {
            let rn = r
                / (0.5
                    + 0.35 * (around(a, 2.2, seed) - 0.5)
                    + 0.06 * (around(a, 7.0, seed ^ 0x77) - 0.5));
            let scar = 1.0 - smooth(0.9, 1.0, rn);
            let bowl = (1.0 - rn * rn).max(0.0);
            let ripple = 0.5 + 0.5 * (rn * 14.0 + 3.0 * vnoise(u * 3.0, v * 3.0, seed)).sin();
            let facet = ((a / std::f32::consts::TAU + 0.5) * 5.0
                + 0.8 * vnoise(u * 2.0, v * 2.0, seed ^ 4))
            .floor();
            let tilt = hash01(facet as i32 as u32, seed ^ 0x3c) - 0.5;
            let grit = fbm(u * 15.0, v * 15.0, seed ^ 0x6d) - 0.5;
            let powder = 1.0 - smooth(0.0, 0.4, rn);
            let cracks = spokes(u, v, 6.0, 1.0, 0.07, 0.0, seed ^ 5) * smooth(0.55, 0.9, rn);
            let flake = smooth(0.62, 0.7, fbm(u * 6.0, v * 6.0, seed ^ 8))
                * smooth(0.8, 1.0, rn)
                * (1.0 - smooth(1.0, 1.35, rn));
            let inner = 0.74 + 0.26 * powder - 0.14 * (n - 0.5) - 0.07 * ripple;
            let value = if cracks > scar.max(flake) {
                0.18
            } else if flake > scar {
                0.86
            } else {
                inner
            };
            (
                value,
                (scar * (0.85 + 0.15 * n))
                    .max(cracks * 0.85)
                    .max(flake * 0.7),
                (-0.03 * bowl - 0.016 - 0.002 * ripple + 0.006 * tilt * rn + 0.005 * grit) * scar
                    - 0.008 * cracks
                    + 0.004 * flake,
            )
        }
        // A blow on metal: bright scratches across a darker dent.
        12 | 13 => {
            let (s, c) = (0.35f32).sin_cos();
            let (ru, rv) = (u * c - v * s, u * s + v * c);
            let mut scratch: f32 = 0.0;
            for k in 0..5 {
                let o = (hash01(k, seed) - 0.5) * 0.7;
                let l = 0.3 + 0.55 * hash01(k, seed ^ 0x77);
                let cover = line(rv - o, 0.025) * (1.0 - smooth(l * 0.8, l, ru.abs()));
                scratch = scratch.max(cover);
            }
            let dent = 1.0 - smooth(0.12, 0.3, (ru * ru * 0.5 + rv * rv * 2.0).sqrt());
            let value = if scratch > dent * 0.8 { 1.0 } else { 0.35 };
            (
                value,
                scratch.max(dent * 0.6),
                -0.03 * dent - 0.003 * scratch,
            )
        }
        // A blunt or bladed blow on soil: a smear of turned earth.
        14 | 15 => {
            let rn = (u * u * 0.5 + v * v * 2.2).sqrt() / (0.7 + 0.25 * around(a, 1.8, seed));
            let body = 1.0 - smooth(0.6, 1.0, rn);
            let lumps = fbm(u * 6.0, v * 6.0, seed ^ 2) - 0.5;
            (
                0.6 - 0.3 * n,
                body * (0.55 + 0.45 * n),
                (0.03 * lumps - 0.012) * body,
            )
        }
        // An arrow's hole: a small dark bore with a pale bruise and a split.
        16 | 17 => {
            let hole = smooth(0.1, 0.05, r);
            let ring = 1.0 - smooth(0.16, 0.3, r / (0.85 + 0.3 * around(a, 3.0, seed)));
            let split = line(v, 0.03) * (1.0 - smooth(0.25, 0.45, u.abs()));
            let value = if hole.max(split) > 0.5 { 0.07 } else { 0.85 };
            (
                value,
                hole.max(ring * 0.85).max(split * 0.9),
                -0.08 * hole - 0.008 * ring - 0.012 * split,
            )
        }
        // Scorch: soot, darkest at the centre, ragged at the edge.
        18 | 19 => {
            let rn = r / (0.82 + 0.3 * (around(a, 2.0, seed) - 0.5));
            let body = 1.0 - smooth(0.35, 1.0, rn);
            (0.25 + 0.5 * rn.min(1.0) * n, body * (0.6 + 0.4 * n), 0.0)
        }
        // Blood: a splat with satellite drops; the last variant is a spray.
        20..=23 => {
            let spray = cell == 23;
            let (su, sv) = if spray { (u * 0.55, v * 1.6) } else { (u, v) };
            let sr = (su * su + sv * sv).sqrt();
            let rn = sr / (0.4 + 0.22 * (around(sv.atan2(su), 2.4, seed) - 0.5));
            let body = 1.0 - smooth(0.8, 1.0, rn);
            let mut drops: f32 = 0.0;
            for k in 0..12 {
                let ang = hash01(k, seed) * std::f32::consts::TAU;
                let dist = 0.45 + 0.45 * hash01(k, seed ^ 0x11);
                let rad = 0.02 + 0.05 * hash01(k, seed ^ 0x22);
                let (cx, cy) = if spray {
                    (dist * ang.cos().signum() * 0.95, 0.12 * ang.sin())
                } else {
                    (dist * ang.cos(), dist * ang.sin())
                };
                let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt();
                drops = drops.max(smooth(rad, rad * 0.6, d));
            }
            let wet = body.max(drops);
            (0.75 + 0.25 * n, wet * 0.92, 0.004 * wet)
        }
        _ => (0.0, 0.0, 0.0),
    };
    // The cell's own transparent border: nothing reaches its edge, so no mip
    // and no bilinear tap bleeds a neighbour in.
    let pad = 1.0 - smooth(0.88, 0.97, u.abs().max(v.abs()));
    (
        value.clamp(0.0, 1.0),
        (alpha * pad).clamp(0.0, 1.0),
        height * pad,
    )
}

/// Both atlases' level 0: the value one (RGBA8 sRGB, grey value in RGB,
/// coverage in A) and the relief one (RGBA8 linear, a tangent-space normal
/// in Bevy's convention: +x along +u, +y toward −v).
pub fn atlas_maps() -> (Vec<u8>, Vec<u8>, u32, u32) {
    let (w, h) = (ATLAS_COLS * CELL_TEX, ATLAS_ROWS * CELL_TEX);
    let mut albedo = vec![0u8; (w * h * 4) as usize];
    let mut relief = vec![0u8; (w * h * 4) as usize];
    for px in relief.chunks_exact_mut(4) {
        px.copy_from_slice(&[128, 128, 255, 255]);
    }
    let used = Kind::ALL
        .iter()
        .map(|k| k.cells().0 + k.cells().1)
        .max()
        .unwrap_or(0);
    let n = CELL_TEX as usize;
    let mut heights = vec![0.0f32; n * n];
    for cell in 0..used {
        let (cx, cy) = (cell % ATLAS_COLS, cell / ATLAS_COLS);
        for y in 0..CELL_TEX {
            for x in 0..CELL_TEX {
                let u = (x as f32 + 0.5) / CELL_TEX as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / CELL_TEX as f32 * 2.0 - 1.0;
                let (val, a, ht) = texel(cell, u, v);
                heights[y as usize * n + x as usize] = ht;
                let px = (cx * CELL_TEX + x) as usize;
                let py = (cy * CELL_TEX + y) as usize;
                let i = (py * w as usize + px) * 4;
                let g = (val * 255.0 + 0.5) as u8;
                albedo[i] = g;
                albedo[i + 1] = g;
                albedo[i + 2] = g;
                albedo[i + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
        // Slope per cell width: a central difference spans two texels.
        let k = CELL_TEX as f32 * 0.5;
        for y in 0..n {
            for x in 0..n {
                let at = |xx: usize, yy: usize| heights[yy.min(n - 1) * n + xx.min(n - 1)];
                let sx = (at(x + 1, y) - at(x.saturating_sub(1), y)) * k;
                let sy = (at(x, y + 1) - at(x, y.saturating_sub(1))) * k;
                let nv = Vec3::new(-sx, sy, 1.0).normalize();
                let px = cx as usize * n + x;
                let py = cy as usize * n + y;
                let i = (py * w as usize + px) * 4;
                relief[i] = ((nv.x * 0.5 + 0.5) * 255.0 + 0.5) as u8;
                relief[i + 1] = ((nv.y * 0.5 + 0.5) * 255.0 + 0.5) as u8;
                relief[i + 2] = ((nv.z * 0.5 + 0.5) * 255.0 + 0.5) as u8;
            }
        }
    }
    (albedo, relief, w, h)
}

/// The value atlas's level 0 alone — [`atlas_maps`]'s first half.
pub fn atlas_pixels() -> (Vec<u8>, u32, u32) {
    let (albedo, _, w, h) = atlas_maps();
    (albedo, w, h)
}

/// The two atlas images, value and relief, each with its whole mip chain — a
/// mark at 40 m is a few texels and must not shimmer.
pub fn atlas_images() -> (Image, Image) {
    let (albedo, relief, w, h) = atlas_maps();
    let image = |level0: Vec<u8>, format: TextureFormat, filter: mipmap::Filter| {
        let mut img = Image::new(
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            level0,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        if let Some(l0) = img.data.as_ref() {
            let chain = mipmap::chain(l0, w, h, filter);
            img.texture_descriptor.mip_level_count = mipmap::levels(w, h);
            img.data = Some(chain);
        }
        img.sampler = ImageSampler::linear();
        img
    };
    (
        image(albedo, TextureFormat::Rgba8UnormSrgb, mipmap::Filter::Srgb),
        image(relief, TextureFormat::Rgba8Unorm, mipmap::Filter::Normal),
    )
}

// ─── The weak spot ───────────────────────────────────────────────────────────

/// The weak-spot mask: two soft bars crossed at right angles, white — the
/// reference's own glyph, and the one shape nothing else on a trunk makes.
pub fn cross_texture() -> Image {
    let n = MARK_TEX as usize;
    let mut data = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) * 0.5;
    const HALF_W: f32 = 0.13;
    const SOFT: f32 = 0.08;
    for y in 0..n {
        for x in 0..n {
            let (px, py) = ((x as f32 - c) / c, (y as f32 - c) / c);
            let d1 = (px - py).abs() * std::f32::consts::FRAC_1_SQRT_2;
            let d2 = (px + py).abs() * std::f32::consts::FRAC_1_SQRT_2;
            let bar = |d: f32| ((HALF_W + SOFT - d) / SOFT).clamp(0.0, 1.0);
            let a = bar(d1).max(bar(d2));
            let edge = (px * px + py * py).sqrt();
            let pad = ((0.95 - edge) / 0.15).clamp(0.0, 1.0);
            let i = (y * n + x) * 4;
            data[i] = 255;
            data[i + 1] = 255;
            data[i + 2] = 255;
            data[i + 3] = (a * pad * 255.0) as u8;
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: MARK_TEX,
            height: MARK_TEX,
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

/// The cross's colour: pale and warm, off bark, granite, ore and the marks
/// the player has made.
pub const WEAK_TINT: Color = Color::srgb(0.98, 0.86, 0.52);

/// Where the weak-spot cross for `slot` sits and which way it faces: on the
/// occupant's collision skin, on the side the sector faces, at the height a
/// swing lands. `None` for an occupant with no skin (a bush).
pub fn weak_spot_pose(slot: &Slot, mark8: u8) -> Option<(Vec3, Vec3)> {
    let occ = slot.occupant as u8;
    let r = skin_radius(occ) * slot.scale;
    if r <= 0.0 {
        return None;
    }
    let (wx, wz) = yaw_dir((mark8 as u16) << 8);
    let (_, top) = terrain::occupant_volume(slot.occupant);
    let h = strike_height(occ).min(top * slot.scale - 0.15).max(0.15);
    let n = Vec3::new(wx, 0.0, wz);
    Some((Vec3::new(slot.x, slot.y + h, slot.z) + n * r, n))
}

/// The cross's alpha at time `t`: breathing outside the sector, held bright
/// inside it.
pub fn weak_spot_alpha(t: f32, in_sector: bool) -> f32 {
    if in_sector {
        return WEAK_MARK_ALPHA_HI;
    }
    let phase = (t * WEAK_MARK_PULSE_HZ * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    WEAK_MARK_ALPHA_LO + (WEAK_MARK_ALPHA_HI - WEAK_MARK_ALPHA_LO) * phase
}

/// Write one grid into a one-mark mesh (the weak spot's): the cell-local uv,
/// white, and `g.a` as alpha.
fn write_grid(mesh: &mut Mesh, grid: &[MarkVert; VERTS_PER_MARK]) {
    for (attr, values) in mesh.attributes_mut() {
        let id = attr.id;
        match values {
            VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_POSITION.id => {
                for (o, g) in v.iter_mut().zip(grid) {
                    *o = (g.p + g.n * MESH_MARK_LIFT_M).to_array();
                }
            }
            VertexAttributeValues::Float32x3(v) if id == Mesh::ATTRIBUTE_NORMAL.id => {
                for (o, g) in v.iter_mut().zip(grid) {
                    *o = g.n.to_array();
                }
            }
            VertexAttributeValues::Float32x2(v) if id == Mesh::ATTRIBUTE_UV_0.id => {
                for (o, g) in v.iter_mut().zip(grid) {
                    *o = g.uv.to_array();
                }
            }
            VertexAttributeValues::Float32x4(v) if id == Mesh::ATTRIBUTE_COLOR.id => {
                for (o, g) in v.iter_mut().zip(grid) {
                    *o = [1.0, 1.0, 1.0, g.a];
                }
            }
            VertexAttributeValues::Float32x4(v) if id == Mesh::ATTRIBUTE_TANGENT.id => {
                for (o, g) in v.iter_mut().zip(grid) {
                    *o = [g.t.x, g.t.y, g.t.z, g.w];
                }
            }
            _ => {}
        }
    }
}

/// Draw the weak-spot cross on the node the sim marked for this player, or
/// hide it. Reads the core's latched `mark_cell`/`mark8` — not a ring.
///
/// Placed where [`weak_spot_pose`] says on the sim's skin, then moved onto
/// the drawn mesh there and fitted to it, as a mark is.
#[allow(clippy::too_many_arguments)]
pub fn weak_spot(
    mut pool: ResMut<Marks>,
    net: Option<NonSend<super::Net>>,
    world: Option<Res<WorldId>>,
    in_weak: Res<super::verbs::InWeak>,
    time: Res<Time>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    skins: Query<(Entity, &Skin, &Mesh3d, &GlobalTransform)>,
    mut q: Query<&mut Visibility, With<WeakSpot>>,
) {
    let Some(entity) = pool.weak else { return };
    let Ok(mut vis) = q.get_mut(entity) else {
        return;
    };
    let hide = |vis: &mut Visibility, pool: &mut Marks| {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
        pool.weak_cell = NO_CELL;
    };
    let (Some(net), Some(world)) = (net, world) else {
        hide(&mut vis, &mut pool);
        return;
    };
    let core = &net.session.core;
    if core.mark_cell == NO_CELL {
        hide(&mut vis, &mut pool);
        return;
    }
    if pool.weak_cell != core.mark_cell || pool.weak_mark8 != core.mark8 {
        let (cx, cz) = (
            (core.mark_cell >> 16) as i32,
            (core.mark_cell & 0xFFFF) as i32,
        );
        let slot = terrain::scatter(world.seed, &world.table, &world.haven, cx, cz);
        let Some((at, n)) = weak_spot_pose(&slot, core.mark8) else {
            hide(&mut vis, &mut pool);
            return;
        };
        let candidates = skins
            .iter()
            .filter_map(|(e, s, m, tf)| meshes.get(&m.0).map(|m| (e, s, m, tf)));
        let snapped = super::skin::snap(at, None, candidates).and_then(|(e, h)| {
            let (_, s, m, tf) = skins.get(e).ok()?;
            Some((h, *s, m.0.clone(), *tf))
        });
        let tangent = Vec3::Y.cross(n).normalize_or(Vec3::X);
        let grid = match snapped {
            Some((h, s, mesh, tf)) => {
                let bend_r = if s.tree && h.normal.y.abs() < 0.5 {
                    let o = tf.translation();
                    Vec2::new(h.at.x - o.x, h.at.z - o.z).length()
                } else {
                    0.0
                };
                let t = Vec3::Y.cross(h.normal).normalize_or(tangent);
                let mut grid = grid_of(h.at, h.normal, t, WEAK_MARK_SIZE_M, bend_r, 0);
                if let Some(m) = meshes.get(&mesh) {
                    conform_grid(
                        &mut grid,
                        h.normal,
                        conform_reach(WEAK_MARK_SIZE_M),
                        &mut skin_cast(m, &tf),
                    );
                }
                grid
            }
            None => {
                let bend_r = skin_radius(slot.occupant as u8) * slot.scale;
                grid_of(at, n, tangent, WEAK_MARK_SIZE_M, bend_r, 0)
            }
        };
        if let Some(m) = meshes.get_mut(&pool.weak_mesh) {
            write_grid(m, &grid);
        }
        pool.weak_cell = core.mark_cell;
        pool.weak_mark8 = core.mark8;
        pool.weak_step = -1;
    }
    *vis = Visibility::Visible;
    let alpha = weak_spot_alpha(time.elapsed_secs(), in_weak.0);
    let step = (alpha * ALPHA_STEPS) as i32;
    if step == pool.weak_step {
        return;
    }
    pool.weak_step = step;
    if let Some(m) = standard.get_mut(&pool.weak_material) {
        m.base_color = WEAK_TINT.with_alpha(step as f32 / ALPHA_STEPS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full pool recycles its oldest rather than refusing the newest, and
    /// spreads the recycling over every slot.
    #[test]
    fn the_mark_pool_is_bounded_and_recycles_rather_than_refusing() {
        let mut pool = Marks::default();
        for _ in 0..MARKS {
            pool.place(Vec3::ZERO, Vec3::Y, Kind::HoleSoil, 0.2, Matter::Dirt, 0.0);
        }
        assert_eq!(pool.live(), MARKS);
        let mut seen = [0u32; MARKS];
        for _ in 0..MARKS * 2 {
            seen[pool.claim()] += 1;
        }
        assert!(seen.iter().all(|&n| n == 2), "recycling must walk the pool");
        pool.slots[7].left = 0.0;
        assert_eq!(pool.claim(), 7, "a free slot is taken before a live one");
    }

    /// A mark lives its life, fades, and frees its slot; a free slot is a
    /// collapsed patch.
    #[test]
    fn a_mark_fades_out_and_collapses() {
        let mut pool = Marks::default();
        let ix = pool.place(
            Vec3::new(3.0, 1.0, 2.0),
            Vec3::Y,
            Kind::HoleWood,
            0.14,
            Matter::Wood,
            0.0,
        );
        assert!(pool.age(0.0) || pool.vertices(ix)[0].3[3] > 0.99);
        let full = pool.vertices(ix);
        assert!(
            full.iter().all(|v| (v.3[3] - 1.0).abs() < 1e-6),
            "a new mark is opaque"
        );
        pool.age(LIFE_S + FADE_S * 0.5);
        let half = pool.vertices(ix)[0].3[3];
        assert!(
            half > 0.3 && half < 0.7,
            "halfway through the fade, alpha {half}"
        );
        pool.age(FADE_S);
        assert_eq!(pool.live(), 0);
        assert!(pool
            .vertices(ix)
            .iter()
            .all(|v| v.0 == [0.0; 3] && v.3[3] == 0.0));
    }

    /// A flat mark lies on its surface, lifted along the normal, inside its
    /// footprint; a bent one stays on its trunk.
    #[test]
    fn a_mark_lies_on_its_surface() {
        let mut pool = Marks::default();
        let (at, n) = (Vec3::new(10.0, 2.0, 5.0), Vec3::new(1.0, 0.0, 0.0));
        let ix = pool.place(at, n, Kind::HoleStone, 0.2, Matter::Stone, 0.0);
        for (p, ..) in pool.vertices(ix) {
            let d = Vec3::from_array(p) - at;
            assert!(
                (d.dot(n) - MESH_MARK_LIFT_M).abs() < 1e-4,
                "lifted along the normal"
            );
            assert!(d.length() < 0.2 * 1.2 * 0.75, "inside the footprint");
        }
        let r = 0.3;
        let axis = at - n * r;
        let ix = pool.place(at, n, Kind::GashWood, 0.26, Matter::Wood, r);
        for (p, ..) in pool.vertices(ix) {
            let d = (Vec3::from_array(p) - axis).with_y(0.0).length();
            assert!((d - r - MESH_MARK_LIFT_M).abs() < 2e-3, "on the trunk: {d}");
        }
    }

    /// Wood never takes a metal or stone hole, a bullet leaves a hole and a
    /// blade a gash, and water and bushes keep no mark.
    #[test]
    fn a_blow_leaves_the_mark_of_what_it_hit() {
        assert_eq!(
            decal_kind(Weapon::Bullet, Matter::Wood).map(|k| k.0),
            Some(Kind::HoleWood)
        );
        assert_eq!(
            decal_kind(Weapon::Bullet, Matter::Metal).map(|k| k.0),
            Some(Kind::HoleMetal)
        );
        assert_eq!(
            decal_kind(Weapon::Bullet, Matter::Stone).map(|k| k.0),
            Some(Kind::HoleStone)
        );
        assert_eq!(
            decal_kind(Weapon::Bullet, Matter::Sand).map(|k| k.0),
            Some(Kind::HoleSoil)
        );
        assert_eq!(
            decal_kind(Weapon::Melee, Matter::Wood).map(|k| k.0),
            Some(Kind::GashWood)
        );
        assert_eq!(
            decal_kind(Weapon::Blast, Matter::Metal).map(|k| k.0),
            Some(Kind::Scorch)
        );
        assert_eq!(
            decal_kind(Weapon::Bullet, Matter::Flesh).map(|k| k.0),
            Some(Kind::Blood)
        );
        assert!(decal_kind(Weapon::Bullet, Matter::Water).is_none());
        assert!(decal_kind(Weapon::Melee, Matter::Plant).is_none());
    }

    /// Every triangle of a mark winds counter-clockwise seen from its normal,
    /// flat or bent, whatever its uv turn — the face shown is the front.
    #[test]
    fn a_mark_faces_its_normal() {
        let mesh = mark_mesh();
        let Some(Indices::U32(idx)) = mesh.indices() else {
            panic!("u32 indices");
        };
        let mut pool = Marks::default();
        for (n, bend) in [(Vec3::Y, 0.0), (Vec3::new(0.6, 0.0, 0.8), 0.25)] {
            for _ in 0..8 {
                let ix = pool.place(Vec3::ONE, n, Kind::GashWood, 0.26, Matter::Wood, bend);
                let v = pool.vertices(ix);
                let base = ix * VERTS_PER_MARK;
                let per = (MARK_GRID - 1) * (MARK_GRID - 1) * 6;
                for t in idx[ix * per..(ix + 1) * per].chunks_exact(3) {
                    let p = |k: u32| Vec3::from_array(v[k as usize - base].0);
                    let face = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                    let nv = Vec3::from_array(v[t[0] as usize - base].1);
                    assert!(face.dot(nv) > 0.0, "a back face toward the normal");
                }
            }
        }
    }

    /// Every cell a kind uses is drawn, is transparent at its border (so no
    /// mip bleeds a neighbour in) and has a solid body somewhere.
    #[test]
    fn every_atlas_cell_is_padded_and_drawn() {
        let (data, w, _) = atlas_pixels();
        for k in Kind::ALL {
            let (first, n) = k.cells();
            for cell in first..first + n {
                let (cx, cy) = (cell % ATLAS_COLS, cell / ATLAS_COLS);
                let alpha = |x: u32, y: u32| {
                    data[(((cy * CELL_TEX + y) * w + cx * CELL_TEX + x) * 4 + 3) as usize]
                };
                for i in 0..CELL_TEX {
                    for (x, y) in [(i, 0), (i, CELL_TEX - 1), (0, i), (CELL_TEX - 1, i)] {
                        assert_eq!(alpha(x, y), 0, "{k:?} cell {cell} bleeds at its edge");
                    }
                }
                let max = (0..CELL_TEX * CELL_TEX)
                    .map(|i| alpha(i % CELL_TEX, i / CELL_TEX))
                    .max()
                    .unwrap_or(0);
                assert!(max > 200, "{k:?} cell {cell} is empty (max alpha {max})");
            }
        }
    }
}
