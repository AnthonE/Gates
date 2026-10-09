//! What other players wear, drawn on their bodies (wire v98, `EventMsg::Worn`).
//!
//! The shard says which item sits in each wear slot of every body; this hangs
//! a piece of clothing or armour off the matching bone — a hood or a helmet
//! on `Head`, a tunic, a vest or a poncho on the spine, a tunic's skirt on
//! `Hips` — so a hide poncho reads from across a field the way it does in
//! Rust. Ids only: the wire carries no condition and no count.
//!
//! ## Placing a piece on a bone
//!
//! Every piece is authored in **body space** — metres, +Y up, the face toward
//! +Z, feet at 0 — measured off the shipped rig's bind pose. A piece rides its
//! bone exactly as a skinned vertex weighted wholly to that bone would: its
//! transform in the bone's frame is the bone's inverse bind matrix times
//! [`MESH_FROM_BODY`], read off the body's own `SkinnedMesh` at runtime. The
//! one fixed fact is that rotation — the rig's mesh space is Z-up and body
//! space is Y-up — and `tests/rig_asset.rs` pins it against the file.
//!
//! The pieces are rigid, so a swinging arm can pass through a poncho's edge;
//! that is the reference's own low-detail look and the price of no new art.

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sim_core::gather::NO_ITEM;
use sim_core::limits::WEAR_SLOTS;

use super::bodies::Body;
use super::Net;

/// Mesh space from body space: the rig's mesh is Z-up, a quarter turn about
/// X from the body's Y-up (`tests/rig_asset.rs` checks every bone's rest pose
/// times its inverse bind matrix is this turn's inverse).
pub const MESH_FROM_BODY: Mat4 = Mat4::from_cols(
    Vec4::new(1.0, 0.0, 0.0, 0.0),
    Vec4::new(0.0, 0.0, -1.0, 0.0),
    Vec4::new(0.0, 1.0, 0.0, 0.0),
    Vec4::new(0.0, 0.0, 0.0, 1.0),
);

/// The bones a piece hangs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bone {
    Head = 0,
    Spine = 1,
    Hips = 2,
}

/// Rig bone names, index-aligned with [`Bone`].
pub const BONE_NAMES: [&str; 3] = ["Head", "Spine01", "Hips"];

/// Parts one body can wear at once: two slots, two parts each.
const MAX_PARTS: usize = 4;

/// What a body's clothing hangs from, found once its scene has spawned, and
/// what it is showing.
#[derive(Component)]
pub struct WornRig {
    bones: [Option<Entity>; 3],
    /// The body's skinned mesh, whose inverse bind poses place a piece.
    pub skin: Option<Entity>,
    shown: [u16; WEAR_SLOTS],
    parts: [Option<Entity>; MAX_PARTS],
    /// Dress again next frame: the rig was just found, or a name or the
    /// bind poses had not arrived when it last tried.
    pending: bool,
}

/// Marks an entity under a body that [`super::anim::reshade`] must not
/// repaint with the body's own material — clothing and the held item.
#[derive(Component)]
pub struct KeepLook;

/// One drawn part: its mesh, its material and the bone it rides.
struct Part {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    bone: Bone,
}

/// The wearable items this client can draw, by the catalog's display name.
#[derive(Resource)]
pub struct WornKit {
    pieces: Vec<(&'static str, Vec<Part>)>,
}

impl WornKit {
    /// The parts a wearable of display name `name` draws, none if unknown.
    fn parts_of(&self, name: &str) -> &[Part] {
        self.pieces
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(&[], |(_, p)| p.as_slice())
    }
}

/// The burlap's colour, linear: undyed sacking.
const BURLAP: [f32; 3] = [0.33, 0.25, 0.14];
/// Bone, weathered.
const BONE: [f32; 3] = [0.50, 0.46, 0.36];
/// Tanned hide.
const HIDE: [f32; 3] = [0.26, 0.15, 0.08];

/// Build the meshes and materials once (`Startup`).
pub fn load(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cloth = |c: [f32; 3], rough: f32| StandardMaterial {
        base_color: Color::linear_rgb(c[0], c[1], c[2]),
        perceptual_roughness: rough,
        // Seen from inside through a hood's face or a poncho's hem.
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    let burlap = materials.add(cloth(BURLAP, 0.95));
    let bone = materials.add(cloth(BONE, 0.7));
    let hide = materials.add(cloth(HIDE, 0.8));
    let metal = super::textures::MapSet::load(&assets, "metal");
    let sign = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.62, 0.64, 0.66),
        base_color_texture: Some(metal.albedo),
        perceptual_roughness: 0.6,
        metallic: 0.6,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let mut add = |m: Mesh| meshes.add(m);
    let part = |mesh, material: &Handle<StandardMaterial>, bone| Part {
        mesh,
        material: material.clone(),
        bone,
    };
    // The head is the top of the log: x ±0.18, z ±0.23 (the nose), its flat
    // crown at 1.80; the torso below it, centre z −0.076.
    let hood = add(lathe(
        Vec2::ZERO,
        &[
            (1.10, 0.20, 0.25),
            (1.18, 0.225, 0.275),
            (1.74, 0.225, 0.275),
            (1.82, 0.20, 0.245),
            (1.88, 0.13, 0.16),
            (1.91, 0.0, 0.0),
        ],
        Some((0.95, 1.24, 1.70)),
    ));
    let dome = add(lathe(
        Vec2::ZERO,
        &[
            (1.55, 0.235, 0.285),
            (1.60, 0.225, 0.275),
            (1.78, 0.215, 0.265),
            (1.86, 0.18, 0.22),
            (1.92, 0.10, 0.12),
            (1.94, 0.0, 0.0),
        ],
        None,
    ));
    let crest = add(Cuboid::new(0.035, 0.06, 0.42)
        .mesh()
        .build()
        .translated_by(Vec3::new(0.0, 1.94, -0.01)));
    let tunic_top = add(lathe(
        Vec2::new(0.0, -0.076),
        &[(0.93, 0.19, 0.17), (1.26, 0.185, 0.165), (1.34, 0.13, 0.12)],
        None,
    ));
    let tunic_skirt = add(lathe(
        Vec2::new(0.0, -0.076),
        &[(0.58, 0.215, 0.195), (0.97, 0.19, 0.17)],
        None,
    ));
    let poncho = add(lathe(
        Vec2::new(0.0, -0.076),
        &[(0.80, 0.36, 0.30), (1.30, 0.17, 0.155), (1.36, 0.12, 0.11)],
        None,
    ));
    let plate_front = add(Cuboid::new(0.30, 0.34, 0.025)
        .mesh()
        .build()
        .translated_by(Vec3::new(0.0, 1.12, 0.095)));
    let plate_back = add(Cuboid::new(0.30, 0.34, 0.025)
        .mesh()
        .build()
        .translated_by(Vec3::new(0.0, 1.12, -0.245)));
    commands.insert_resource(WornKit {
        pieces: vec![
            ("Burlap Hood", vec![part(hood, &burlap, Bone::Head)]),
            (
                "Bone Helmet",
                vec![
                    part(dome, &bone, Bone::Head),
                    part(crest, &bone, Bone::Head),
                ],
            ),
            (
                "Burlap Tunic",
                vec![
                    part(tunic_top, &burlap, Bone::Spine),
                    part(tunic_skirt, &burlap, Bone::Hips),
                ],
            ),
            ("Hide Poncho", vec![part(poncho, &hide, Bone::Spine)]),
            (
                "Roadsign Vest",
                vec![
                    part(plate_front, &sign, Bone::Spine),
                    part(plate_back, &sign, Bone::Spine),
                ],
            ),
        ],
    });
}

/// Find a remote body's bones and skin once its scene has spawned: the same
/// trigger and climb as `bodies::bind_hands`.
pub fn bind(
    mut commands: Commands,
    added: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    names: Query<&Name>,
    skinned: Query<(), With<SkinnedMesh>>,
    ids: Query<(), With<Body>>,
) {
    for player in &added {
        let mut at = player;
        let mut body = None;
        for _ in 0..16 {
            if ids.get(at).is_ok() {
                body = Some(at);
                break;
            }
            match parents.get(at) {
                Ok(p) => at = p.0,
                Err(_) => break,
            }
        }
        let Some(body) = body else { continue };
        let mut rig = WornRig {
            bones: [None; 3],
            skin: None,
            shown: [NO_ITEM; WEAR_SLOTS],
            parts: [None; MAX_PARTS],
            pending: true,
        };
        let mut stack = vec![body];
        let mut seen = 0usize;
        while let Some(e) = stack.pop() {
            seen += 1;
            if seen > 512 {
                break;
            }
            if rig.skin.is_none() && skinned.get(e).is_ok() {
                rig.skin = Some(e);
            }
            if let Ok(n) = names.get(e) {
                if let Some(i) = BONE_NAMES.iter().position(|b| *b == n.as_str()) {
                    rig.bones[i] = Some(e);
                }
            }
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
        }
        commands.entity(body).insert(rig);
    }
}

/// Keep each remote body dressed in what the shard says it wears. Work only
/// when a `Worn` landed (`ClientCore::worn_gen`) or a body is waiting.
pub fn dress(
    mut commands: Commands,
    net: NonSend<Net>,
    kit: Option<Res<WornKit>>,
    binds: Res<Assets<SkinnedMeshInverseBindposes>>,
    skins: Query<&SkinnedMesh>,
    mut bodies: Query<(&Body, &mut WornRig)>,
    mut last_gen: Local<u32>,
) {
    let Some(kit) = kit else { return };
    let core = &net.session.core;
    let moved = core.worn_gen != *last_gen;
    *last_gen = core.worn_gen;
    for (body, mut rig) in &mut bodies {
        if !moved && !rig.pending {
            continue;
        }
        let want = core.worn_of(body.0);
        if want == rig.shown && !rig.pending {
            continue;
        }
        let Some(skin) = rig.skin.and_then(|e| skins.get(e).ok()) else {
            continue;
        };
        let Some(ibm) = binds.get(&skin.inverse_bindposes) else {
            rig.pending = true;
            continue;
        };
        let mut pending = false;
        let mut names: [&str; WEAR_SLOTS] = [""; WEAR_SLOTS];
        for (name, &item) in names.iter_mut().zip(want.iter()) {
            if item == NO_ITEM {
                continue;
            }
            match crate::ui::craft::item_name(&core.catalog, item) {
                Some(n) => *name = n,
                None => pending = true, // the catalog has not dripped this row yet
            }
        }
        hang(&mut commands, &kit, skin, ibm, &mut rig, &names);
        rig.shown = want;
        rig.pending = pending;
    }
}

/// Take off whatever `rig` shows and hang the pieces named in `names` (the
/// catalog's display names; an empty or unknown name hangs nothing) on its
/// bones. [`dress`]'s body, and `examples/worn_look.rs`'s.
pub fn hang(
    commands: &mut Commands,
    kit: &WornKit,
    skin: &SkinnedMesh,
    ibm: &[Mat4],
    rig: &mut WornRig,
    names: &[&str],
) {
    for p in rig.parts.iter_mut() {
        if let Some(e) = p.take() {
            commands.entity(e).despawn();
        }
    }
    let mut n = 0;
    for name in names {
        for part in kit.parts_of(name) {
            let Some(bone) = rig.bones[part.bone as usize] else {
                continue;
            };
            let Some(j) = skin.joints.iter().position(|&e| e == bone) else {
                continue;
            };
            let Some(&inv) = ibm.get(j) else { continue };
            if n == MAX_PARTS {
                return;
            }
            let e = commands
                .spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(part.material.clone()),
                    Transform::from_matrix(inv * MESH_FROM_BODY),
                    KeepLook,
                    ChildOf(bone),
                ))
                .id();
            rig.parts[n] = Some(e);
            n += 1;
        }
    }
}

/// One ring of a [`lathe`] profile: its height and its x and z radii, body
/// space metres.
type Ring = (f32, f32, f32);

/// A surface of revolution in body space, centred on `c` (x, z), through the
/// `profile` rings bottom to top — elliptic, so a piece can be deeper than
/// wide like the head it covers. `window` cuts a hole facing +Z: half-width
/// in radians and the height band it spans, for a hood's face.
fn lathe(c: Vec2, profile: &[Ring], window: Option<(f32, f32, f32)>) -> Mesh {
    const SEG: usize = 28;
    let rings = profile.len();
    let mut pos = Vec::with_capacity((SEG + 1) * rings);
    let mut nor = Vec::with_capacity(pos.capacity());
    let mut uv = Vec::with_capacity(pos.capacity());
    for (i, &(y, sx, sz)) in profile.iter().enumerate() {
        // The profile's own slope here, for the normal: outward in the
        // radial plane, leaned up where the radius is closing.
        let (lo, hi) = (
            profile[i.saturating_sub(1)],
            profile[(i + 1).min(rings - 1)],
        );
        let dy = hi.0 - lo.0;
        let dr = (hi.1 + hi.2 - lo.1 - lo.2) * 0.5;
        let (nr, ny) = {
            let l = (dy * dy + dr * dr).sqrt().max(1e-6);
            (dy / l, -dr / l)
        };
        let rm = (sx + sz) * 0.5;
        for k in 0..=SEG {
            let u = k as f32 / SEG as f32;
            // θ = 0 faces +Z, the face; θ runs toward +X.
            let (st, ct) = (u * std::f32::consts::TAU).sin_cos();
            pos.push([c.x + sx * st, y, c.y + sz * ct]);
            let n = Vec3::new(st / sx.max(1e-3) * rm * nr, ny, ct / sz.max(1e-3) * rm * nr);
            nor.push(n.normalize_or(Vec3::Y).to_array());
            uv.push([u, i as f32 / (rings - 1).max(1) as f32]);
        }
    }
    let mut idx = Vec::with_capacity(SEG * rings * 6);
    for i in 0..rings - 1 {
        let ym = (profile[i].0 + profile[i + 1].0) * 0.5;
        for k in 0..SEG {
            if let Some((half, lo, hi)) = window {
                let th = (k as f32 + 0.5) / SEG as f32 * std::f32::consts::TAU;
                let off = (th + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                if off.abs() < half && ym > lo && ym < hi {
                    continue;
                }
            }
            let a = (i * (SEG + 1) + k) as u32;
            let b = a + (SEG + 1) as u32;
            idx.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    build(pos, nor, uv, idx)
}

/// Counter-clockwise seen from outside (θ runs toward +X, rings upward),
/// so a double-sided material flips the normal on the INSIDE only.
fn build(pos: Vec<[f32; 3]>, nor: Vec<[f32; 3]>, uv: Vec<[f32; 2]>, idx: Vec<u32>) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, Default::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nor)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}
