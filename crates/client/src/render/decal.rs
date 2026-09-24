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
//! along the normal, and bent to the trunk where it lands on one.
//!
//! # What a mark looks like
//!
//! Picked per weapon × matter ([`decal_kind`]), the way the reference picks an
//! impact effect: a bullet leaves a hole whose rim is the material's
//! (splintered wood, chipped stone with cracks, bright bare metal, a soil
//! crater), a blade a gash, a blow on metal a dent, a blast a scorch and a
//! body blood on the floor behind it. Every mark is rotated and scaled at
//! random, and fades out after its life. The atlas is grey value × alpha; the
//! matter's [`tint`] colours it, so one crater serves every soil.
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
use super::{surface, Eye, WorldId};
use sim_core::gather::NO_CELL;
use sim_core::ranged::SURF_WORLD;
use sim_core::terrain::{self, Slot};
use sim_core::yaw_dir;

/// Marks drawable at once — a *view* cap: marks are not saved, replicated or
/// known to the sim. Each costs 18 vertices of one shared mesh.
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

/// Segments across a mark, so a trunk mark can bend: eight chords over a
/// 22 cm arc on a 24 cm radius is a sagitta under a millimetre.
pub const MESH_MARK_SEGMENTS: u32 = 8;

/// Vertices per mark: two rows of `MESH_MARK_SEGMENTS + 1`.
const VERTS_PER_MARK: usize = (MESH_MARK_SEGMENTS as usize + 1) * 2;

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
pub const CELL_TEX: u32 = 128;

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

/// One vertex of a mark: position, normal, uv, linear colour.
pub type MarkVertex = ([f32; 3], [f32; 3], [f32; 2], [f32; 4]);

/// One pooled mark.
#[derive(Clone, Copy)]
struct Mark {
    /// Seconds left through life and then [`FADE_S`]; zero is a free slot.
    left: f32,
    /// The alpha step last written, so a fade rewrites only when it moves.
    step: i32,
    pos: Vec3,
    normal: Vec3,
    /// The patch's local +u axis on the surface; horizontal on a trunk.
    tangent: Vec3,
    size: f32,
    cell: u32,
    /// The trunk's radius for a bent mark, metres; zero is flat.
    bend_r: f32,
    /// Linear tint.
    tint: [f32; 3],
    /// A trunk mark cannot be spun (it is aligned to the trunk), so its
    /// variety is the atlas cell's dihedral turn: bit 0 swaps u/v, bits 1–2
    /// mirror them.
    flip: u8,
}

impl Default for Mark {
    fn default() -> Self {
        Self {
            left: 0.0,
            step: -1,
            pos: Vec3::ZERO,
            normal: Vec3::Y,
            tangent: Vec3::X,
            size: 0.0,
            cell: 0,
            bend_r: 0.0,
            tint: [1.0; 3],
            flip: 0,
        }
    }
}

/// The pool: every mark's state, and the handles of the one mesh that draws
/// them. `Default` is written out: `[T; N]` derives it only up to 32.
#[derive(Resource)]
pub struct Marks {
    slots: [Mark; MARKS],
    mesh: Handle<Mesh>,
    /// The oldest slot to recycle when every one is busy — a rotating hand.
    hand: usize,
    /// Whether the mesh no longer matches the slots.
    dirty: bool,
    rng: u32,
    /// Marks laid since the world was entered, and live marks recycled.
    pub placed: u64,
    pub stolen: u64,
    /// The weak-spot cross: its entity, material, the alpha step last
    /// written and the cell and heading it was last placed for.
    weak: Option<Entity>,
    weak_material: Handle<StandardMaterial>,
    weak_step: i32,
    weak_cell: u32,
    weak_mark8: u8,
    weak_at: Vec3,
    weak_normal: Vec3,
}

impl Default for Marks {
    fn default() -> Self {
        Self {
            slots: [Mark::default(); MARKS],
            mesh: Handle::default(),
            hand: 0,
            dirty: false,
            rng: 0,
            placed: 0,
            stolen: 0,
            weak: None,
            weak_material: Handle::default(),
            weak_step: -1,
            weak_cell: NO_CELL,
            weak_mark8: 0,
            weak_at: Vec3::ZERO,
            weak_normal: Vec3::Y,
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
            (
                Vec3::Y.cross(n).normalize_or(Vec3::X),
                (self.roll() * 8.0) as u8,
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
            tangent,
            size,
            cell,
            bend_r,
            tint: [lin.red * j, lin.green * j, lin.blue * j],
            flip,
        };
        self.placed += 1;
        self.dirty = true;
        ix
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
        let mut out = [([0.0; 3], [0.0, 1.0, 0.0], [0.0; 2], [0.0; 4]); VERTS_PER_MARK];
        let m = &self.slots[ix];
        if m.left <= 0.0 {
            return out;
        }
        let a = (alpha_step(m.left) as f32 / ALPHA_STEPS).clamp(0.0, 1.0);
        let col = [m.tint[0], m.tint[1], m.tint[2], a];
        let segs = MESH_MARK_SEGMENTS as usize;
        let b = m.normal.cross(m.tangent);
        let (cu, cv) = (m.cell % ATLAS_COLS, m.cell / ATLAS_COLS);
        // Half a texel in from the cell's edge so a mip never reads the
        // neighbour.
        let inset = 0.5 / CELL_TEX as f32;
        for c in 0..=segs {
            let u = c as f32 / segs as f32;
            let (off, n) = if m.bend_r > 0.0 {
                let theta = (u - 0.5) * m.size / m.bend_r;
                let (s, co) = theta.sin_cos();
                (
                    m.tangent * (m.bend_r * s) + m.normal * (m.bend_r * (co - 1.0)),
                    m.tangent * s + m.normal * co,
                )
            } else {
                (m.tangent * ((u - 0.5) * m.size), m.normal)
            };
            for r in 0..2 {
                let v = r as f32;
                let up = if m.bend_r > 0.0 { Vec3::Y } else { b };
                let p = m.pos + off + up * ((v - 0.5) * m.size) + n * MESH_MARK_LIFT_M;
                let (mut tu, mut tv) = (u, v);
                if m.flip & 1 != 0 {
                    std::mem::swap(&mut tu, &mut tv);
                }
                if m.flip & 2 != 0 {
                    tu = 1.0 - tu;
                }
                if m.flip & 4 != 0 {
                    tv = 1.0 - tv;
                }
                let uv = [
                    (cu as f32 + inset + tu * (1.0 - 2.0 * inset)) / ATLAS_COLS as f32,
                    (cv as f32 + inset + tv * (1.0 - 2.0 * inset)) / ATLAS_ROWS as f32,
                ];
                out[c * 2 + r] = (p.to_array(), n.to_array(), uv, col);
            }
        }
        out
    }

    /// Copy every slot into the mesh's attributes, in place — no allocation.
    fn write(&self, mesh: &mut Mesh) {
        let (mut pos, mut nrm, mut uv, mut col) = (None, None, None, None);
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
                VertexAttributeValues::Float32x4(v) if id == Mesh::ATTRIBUTE_COLOR.id => {
                    col = Some(v);
                }
                _ => {}
            }
        }
        let (Some(pos), Some(nrm), Some(uv), Some(col)) = (pos, nrm, uv, col) else {
            return;
        };
        for ix in 0..MARKS {
            for (k, (p, n, t, c)) in self.vertices(ix).into_iter().enumerate() {
                let i = ix * VERTS_PER_MARK + k;
                pos[i] = p;
                nrm[i] = n;
                uv[i] = t;
                col[i] = c;
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
    pool.slots = [Mark::default(); MARKS];
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

/// The empty mark mesh: [`MARKS`] collapsed patches and their fixed indices.
pub fn mark_mesh() -> Mesh {
    let n = MARKS * VERTS_PER_MARK;
    let segs = MESH_MARK_SEGMENTS;
    let mut idx = Vec::with_capacity(MARKS * segs as usize * 6);
    for m in 0..MARKS as u32 {
        let base = m * VERTS_PER_MARK as u32;
        for s in 0..segs {
            let a = base + s * 2;
            let (b, c, d) = (a + 1, a + 2, a + 3);
            idx.extend_from_slice(&[a, b, c, b, d, c]);
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
    mesh.insert_indices(Indices::U32(idx));
    mesh
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
    let atlas = images.add(atlas_image());
    let cross = images.add(cross_texture());
    pool.mesh = meshes.add(mark_mesh());
    let material = standard.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas),
        // A mark is dirt, splinters and soot — never a highlight the surface
        // under it did not have.
        perceptual_roughness: 0.95,
        metallic: 0.0,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
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

    let curved = meshes.add(curved_patch(
        terrain::occupant_volume(terrain::Occupant::Tree).0 / WEAK_MARK_SIZE_M,
        MESH_MARK_SEGMENTS,
    ));
    let weak_mat = standard.add(StandardMaterial {
        base_color: WEAK_TINT.with_alpha(0.0),
        base_color_texture: Some(cross),
        perceptual_roughness: 0.95,
        metallic: 0.0,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    pool.weak_material = weak_mat.clone();
    pool.weak = Some(
        commands
            .spawn((
                WeakSpot,
                Mesh3d(curved),
                MeshMaterial3d(weak_mat),
                Transform::from_scale(Vec3::splat(WEAK_MARK_SIZE_M)),
                Visibility::Hidden,
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

/// Value (grey, sRGB 0..1) and alpha of cell `cell` at `(u, v) ∈ [-1, 1]²`.
fn texel(cell: u32, u: f32, v: f32) -> (f32, f32) {
    let seed = 0x5EED_0000 ^ cell.wrapping_mul(0x0101_0101);
    let r = (u * u + v * v).sqrt();
    let a = v.atan2(u);
    let n = fbm(u * 5.0, v * 5.0, seed);
    let (value, alpha) = match cell {
        // A soil crater: a dark pit, the slope lighter, clods thrown out.
        0 | 1 => {
            let rn = r / (0.6 + 0.3 * (around(a, 1.6, seed) - 0.5));
            let pit = smooth(0.4, 0.0, rn);
            let body = 1.0 - smooth(0.5, 0.95, rn);
            let clods = smooth(0.62, 0.74, fbm(u * 9.0 + 3.0, v * 9.0, seed ^ 7))
                * smooth(0.6, 0.85, rn)
                * (1.0 - smooth(0.95, 1.35, rn));
            (
                (0.7 - 0.55 * pit + 0.25 * (n - 0.5)).clamp(0.05, 1.0),
                (body * (0.7 + 0.3 * n)).max(clods * 0.9),
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
            (value, pit.max(chip * 0.95).max(cracks * 0.9).max(dust))
        }
        // Metal: a dark centre, a bright ring of bare metal, a scuffed halo.
        6 | 7 => {
            let hole = smooth(0.1, 0.06, r);
            let rim = 1.0 - smooth(0.17, 0.24, r / (0.9 + 0.2 * around(a, 3.0, seed)));
            let scratch = vnoise(a * 9.0, r * 2.0, seed ^ 4);
            let halo = (1.0 - smooth(0.24, 0.58, r)) * (0.35 + 0.65 * scratch);
            let value = if hole > 0.5 { 0.05 } else { 0.8 + 0.2 * rim };
            (value, hole.max(rim).max(halo * 0.75))
        }
        // A blade's gash in wood: a dark cut, torn pale lips, a few chips.
        8 | 9 => {
            // The gash narrows to nothing at its ends; past them there is no
            // cut at all (and a zero width must not reach `smooth`).
            let w = 0.13 * (1.0 - (u / 0.85).powi(2)).max(0.0) + 1e-4;
            let d = v - 0.06 * (vnoise(u * 3.0, 0.5, seed) - 0.5);
            let ends = 1.0 - smooth(0.7, 0.85, u.abs());
            let cut = smooth(w * 0.55, w * 0.2, d.abs()) * ends;
            let lip = smooth(w * 1.7, w * 1.1, d.abs()) * ends;
            let chips = smooth(0.7, 0.8, fbm(u * 8.0, v * 8.0, seed ^ 3))
                * (1.0 - smooth(0.2, 0.5, d.abs()));
            let fibre = 0.84 + 0.16 * vnoise(u * 26.0, v * 3.0, seed ^ 9);
            (
                fibre * (1.0 - cut) + 0.08 * cut,
                cut.max(lip * 0.9).max(chips * 0.8),
            )
        }
        // A pick on stone: a pale chipped scar with darker grit in it.
        10 | 11 => {
            let rn = r / (0.55 + 0.35 * (around(a, 2.2, seed) - 0.5));
            let body = 1.0 - smooth(0.7, 1.0, rn);
            let grit = smooth(0.5, 0.7, fbm(u * 7.0 + 1.3, v * 7.0, seed ^ 6));
            let flake = smooth(0.35, 0.8, rn)
                * (1.0 - smooth(1.0, 1.25, rn))
                * smooth(0.55, 0.62, fbm(u * 4.0, v * 4.0, seed ^ 8));
            (
                0.9 - 0.35 * grit - 0.15 * n,
                (body * (0.7 + 0.3 * n)).max(flake * 0.8),
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
            (value, scratch.max(dent * 0.6))
        }
        // A blunt or bladed blow on soil: a smear of turned earth.
        14 | 15 => {
            let rn = (u * u * 0.5 + v * v * 2.2).sqrt() / (0.7 + 0.25 * around(a, 1.8, seed));
            let body = 1.0 - smooth(0.6, 1.0, rn);
            (0.6 - 0.3 * n, body * (0.55 + 0.45 * n))
        }
        // An arrow's hole: a small dark bore with a pale bruise and a split.
        16 | 17 => {
            let hole = smooth(0.1, 0.05, r);
            let ring = 1.0 - smooth(0.16, 0.3, r / (0.85 + 0.3 * around(a, 3.0, seed)));
            let split = line(v, 0.03) * (1.0 - smooth(0.25, 0.45, u.abs()));
            let value = if hole.max(split) > 0.5 { 0.07 } else { 0.85 };
            (value, hole.max(ring * 0.85).max(split * 0.9))
        }
        // Scorch: soot, darkest at the centre, ragged at the edge.
        18 | 19 => {
            let rn = r / (0.82 + 0.3 * (around(a, 2.0, seed) - 0.5));
            let body = 1.0 - smooth(0.35, 1.0, rn);
            (0.25 + 0.5 * rn.min(1.0) * n, body * (0.6 + 0.4 * n))
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
            (0.75 + 0.25 * n, body.max(drops) * 0.92)
        }
        _ => (0.0, 0.0),
    };
    // The cell's own transparent border: nothing reaches its edge, so no mip
    // and no bilinear tap bleeds a neighbour in.
    let pad = 1.0 - smooth(0.88, 0.97, u.abs().max(v.abs()));
    (value.clamp(0.0, 1.0), (alpha * pad).clamp(0.0, 1.0))
}

/// The atlas's level 0: RGBA8 sRGB, grey value in RGB, coverage in A.
pub fn atlas_pixels() -> (Vec<u8>, u32, u32) {
    let (w, h) = (ATLAS_COLS * CELL_TEX, ATLAS_ROWS * CELL_TEX);
    let mut data = vec![0u8; (w * h * 4) as usize];
    let used = Kind::ALL
        .iter()
        .map(|k| k.cells().0 + k.cells().1)
        .max()
        .unwrap_or(0);
    for cell in 0..used {
        let (cx, cy) = (cell % ATLAS_COLS, cell / ATLAS_COLS);
        for y in 0..CELL_TEX {
            for x in 0..CELL_TEX {
                let u = (x as f32 + 0.5) / CELL_TEX as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / CELL_TEX as f32 * 2.0 - 1.0;
                let (val, a) = texel(cell, u, v);
                let px = (cx * CELL_TEX + x) as usize;
                let py = (cy * CELL_TEX + y) as usize;
                let i = (py * w as usize + px) * 4;
                let g = (val * 255.0 + 0.5) as u8;
                data[i] = g;
                data[i + 1] = g;
                data[i + 2] = g;
                data[i + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
    }
    (data, w, h)
}

/// The atlas image, with its whole mip chain — a mark at 40 m is a few
/// texels and must not shimmer.
pub fn atlas_image() -> Image {
    let (level0, w, h) = atlas_pixels();
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    if let Some(l0) = img.data.as_ref() {
        let chain = mipmap::chain(l0, w, h, mipmap::Filter::Srgb);
        img.texture_descriptor.mip_level_count = mipmap::levels(w, h);
        img.data = Some(chain);
    }
    img.sampler = ImageSampler::linear();
    img
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

/// Draw the weak-spot cross on the node the sim marked for this player, or
/// hide it. Reads the core's latched `mark_cell`/`mark8` — not a ring.
pub fn weak_spot(
    mut pool: ResMut<Marks>,
    net: Option<NonSend<super::Net>>,
    world: Option<Res<WorldId>>,
    in_weak: Res<super::verbs::InWeak>,
    time: Res<Time>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(&mut Transform, &mut Visibility), With<WeakSpot>>,
) {
    let Some(entity) = pool.weak else { return };
    let Ok((mut tf, mut vis)) = q.get_mut(entity) else {
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
        pool.weak_cell = core.mark_cell;
        pool.weak_mark8 = core.mark8;
        pool.weak_at = at;
        pool.weak_normal = n;
        pool.weak_step = -1;
    }
    let (at, n) = (pool.weak_at, pool.weak_normal);
    let (_, rot) = mesh_pose(SURF_WORLD, n);
    tf.translation = at + n * MESH_MARK_LIFT_M;
    tf.rotation = rot;
    tf.scale = Vec3::splat(WEAK_MARK_SIZE_M);
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
        for (p, _, _, _) in pool.vertices(ix) {
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
        for (p, _, _, _) in pool.vertices(ix) {
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
