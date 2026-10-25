//! What other players wear, drawn on their bodies (wire v98, `EventMsg::Worn`).
//!
//! The shard says which item sits in each wear slot of every body; this hangs
//! a piece of clothing or armour off the matching bone — a hood or a helmet
//! on `Head`, a tunic, a vest or a poncho on the spine, a tunic's skirt on
//! `Hips` — so a hide poncho reads from across a field the way it does in
//! Rust. Ids and skins: the wire carries no condition and no count.
//!
//! A skinned piece (v102) is drawn in its skin's tint, the held item's rule
//! (`bodies::update_hand`): if this client's skin catalog knows the skin,
//! else plain until the catalog drip brings it.
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
use client_core::core::WornLook;
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

/// The bones a piece hangs from. The limbs since clothing v1: trousers,
/// boots, gloves and sleeves ride the bone of the segment they cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bone {
    Head = 0,
    Spine = 1,
    Hips = 2,
    LeftUpLeg = 3,
    LeftLeg = 4,
    LeftFoot = 5,
    RightUpLeg = 6,
    RightLeg = 7,
    RightFoot = 8,
    LeftArm = 9,
    LeftForeArm = 10,
    LeftHand = 11,
    RightArm = 12,
    RightForeArm = 13,
    RightHand = 14,
}

/// Rig bone names, index-aligned with [`Bone`].
pub const BONE_NAMES: [&str; 15] = [
    "Head",
    "Spine01",
    "Hips",
    "LeftUpLeg",
    "LeftLeg",
    "LeftFoot",
    "RightUpLeg",
    "RightLeg",
    "RightFoot",
    "LeftArm",
    "LeftForeArm",
    "LeftHand",
    "RightArm",
    "RightForeArm",
    "RightHand",
];

/// Parts one body can wear at once: seven slots, a shirt or a coat with its
/// sleeves the most (six), trousers five.
const MAX_PARTS: usize = 32;

/// What a body's clothing hangs from, found once its scene has spawned, and
/// what it is showing.
#[derive(Component)]
pub struct WornRig {
    bones: [Option<Entity>; BONE_NAMES.len()],
    /// The body's skinned mesh, whose inverse bind poses place a piece.
    pub skin: Option<Entity>,
    /// What is drawn: a skin this client could not draw is shown as zero,
    /// so the catalog landing it later reads as a change.
    shown: WornLook,
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
/// Cloth blacked with charcoal, the Ash Mask's and the Ash Suit's.
const ASH: [f32; 3] = [0.045, 0.045, 0.040];
/// Hazmat yellow, rubberised.
const HAZMAT: [f32; 3] = [0.52, 0.38, 0.03];
/// Washed-out cotton, the tank top's.
const COTTON: [f32; 3] = [0.36, 0.36, 0.33];
/// Clothing v1: a grey hoodie, blue work trousers, an olive jacket, a
/// white snow jacket, a red bandana and beenie, dark stitched leather.
const FLEECE: [f32; 3] = [0.20, 0.22, 0.25];
const DENIM: [f32; 3] = [0.08, 0.12, 0.22];
const OLIVE: [f32; 3] = [0.12, 0.14, 0.06];
const SNOW: [f32; 3] = [0.70, 0.74, 0.80];
const RED: [f32; 3] = [0.40, 0.05, 0.03];
const LEATHER: [f32; 3] = [0.12, 0.06, 0.03];

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
    let ash = materials.add(cloth(ASH, 0.95));
    let hazmat = materials.add(cloth(HAZMAT, 0.55));
    let cotton = materials.add(cloth(COTTON, 0.9));
    let fleece = materials.add(cloth(FLEECE, 0.95));
    let denim = materials.add(cloth(DENIM, 0.9));
    let olive = materials.add(cloth(OLIVE, 0.85));
    let snow = materials.add(cloth(SNOW, 0.8));
    let red = materials.add(cloth(RED, 0.9));
    let leather = materials.add(cloth(LEATHER, 0.7));
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
    // The Ash Mask: a band of blacked cloth tied round the lower face, over
    // the mouth and nose, standing a finger off the head.
    let mask = add(lathe(
        Vec2::ZERO,
        &[
            (1.53, 0.192, 0.245),
            (1.60, 0.197, 0.252),
            (1.67, 0.192, 0.246),
        ],
        None,
    ));
    let crest = add(Cuboid::new(0.035, 0.06, 0.42)
        .mesh()
        .build()
        .translated_by(Vec3::new(0.0, 1.94, -0.01)));
    let [top, skirt] = tunic_meshes();
    let tunic_top = add(top);
    let tunic_skirt = add(skirt);
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
    // Clothing v1's limbs, measured off the rig's bind pose (body space):
    // hip, knee, ankle and toe; shoulder, elbow and wrist. Each segment's
    // tube stands a finger off the skin, a coat's a finger off a shirt's.
    let m = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
    let (hip, knee, ankle) = (
        Vec3::new(0.083, 0.74, -0.038),
        Vec3::new(0.114, 0.489, -0.015),
        Vec3::new(0.133, 0.16, -0.039),
    );
    let (shoulder, elbow, wrist) = (
        Vec3::new(0.150, 1.215, -0.055),
        Vec3::new(0.329, 1.054, -0.084),
        Vec3::new(0.490, 0.910, -0.030),
    );
    let tip = wrist + (wrist - elbow).normalize() * 0.15;
    let mut limb = |a: Vec3, b: Vec3, ra: f32, rb: f32| {
        [add(tube(a, b, ra, rb)), add(tube(m(a), m(b), ra, rb))]
    };
    let thigh = limb(hip, knee, 0.072, 0.064);
    let shin = limb(knee + Vec3::Y * 0.01, ankle, 0.07, 0.06);
    let shaft = limb(ankle - Vec3::Y * 0.05, ankle + Vec3::Y * 0.17, 0.072, 0.068);
    let glove = limb(
        wrist - (wrist - elbow).normalize() * 0.04,
        tip,
        0.068,
        0.078,
    );
    let sleeve = limb(shoulder, elbow, 0.068, 0.048);
    let cuff = limb(elbow, wrist, 0.05, 0.045);
    let coat_sleeve = limb(shoulder, elbow, 0.078, 0.058);
    let coat_cuff = limb(elbow, wrist, 0.06, 0.054);
    let mut foot = |x: f32| {
        add(Cuboid::new(0.22, 0.11, 0.38)
            .mesh()
            .build()
            .translated_by(Vec3::new(x, 0.052, 0.07)))
    };
    let feet = [foot(0.163), foot(-0.168)];
    // Up under a shirt's hem, so no bark shows between the two.
    let band = add(lathe(
        Vec2::new(0.0, -0.076),
        &[(0.64, 0.178, 0.164), (0.95, 0.176, 0.162)],
        None,
    ));
    // A bandana over this rig's mouth and chin, under the Ash Mask's band.
    let kerchief = add(lathe(
        Vec2::ZERO,
        &[
            (1.32, 0.19, 0.245),
            (1.40, 0.197, 0.252),
            (1.48, 0.19, 0.246),
        ],
        None,
    ));
    let coat = add(lathe(
        Vec2::new(0.0, -0.076),
        &[
            (0.86, 0.205, 0.185),
            (1.26, 0.198, 0.178),
            (1.36, 0.135, 0.125),
        ],
        None,
    ));
    let parka = add(lathe(
        Vec2::new(0.0, -0.076),
        &[
            (0.66, 0.225, 0.205),
            (0.98, 0.215, 0.195),
            (1.27, 0.212, 0.192),
            (1.38, 0.15, 0.14),
        ],
        None,
    ));
    let cap = add(lathe(
        Vec2::ZERO,
        &[
            (1.69, 0.232, 0.283),
            (1.75, 0.235, 0.286),
            (1.84, 0.21, 0.26),
            (1.91, 0.15, 0.18),
            (1.96, 0.0, 0.0),
        ],
        None,
    ));
    // Every part of a trouser, a boot, a glove or a coat's arms, on its bone.
    let pair = |pieces: [Handle<Mesh>; 2], mat: &Handle<StandardMaterial>, l: Bone, r: Bone| {
        vec![
            part(pieces[0].clone(), mat, l),
            part(pieces[1].clone(), mat, r),
        ]
    };
    let trousers = |mat: &Handle<StandardMaterial>| {
        let mut v = vec![part(band.clone(), mat, Bone::Hips)];
        v.extend(pair(thigh.clone(), mat, Bone::LeftUpLeg, Bone::RightUpLeg));
        v.extend(pair(shin.clone(), mat, Bone::LeftLeg, Bone::RightLeg));
        v
    };
    let boots = |mat: &Handle<StandardMaterial>| {
        let mut v = pair(shaft.clone(), mat, Bone::LeftLeg, Bone::RightLeg);
        v.extend(pair(feet.clone(), mat, Bone::LeftFoot, Bone::RightFoot));
        v
    };
    let gloves =
        |mat: &Handle<StandardMaterial>| pair(glove.clone(), mat, Bone::LeftHand, Bone::RightHand);
    let sleeves = |mat: &Handle<StandardMaterial>, coat: bool| {
        let (a, b) = if coat {
            (coat_sleeve.clone(), coat_cuff.clone())
        } else {
            (sleeve.clone(), cuff.clone())
        };
        let mut v = pair(a, mat, Bone::LeftArm, Bone::RightArm);
        v.extend(pair(b, mat, Bone::LeftForeArm, Bone::RightForeArm));
        v
    };
    let with = |mut v: Vec<Part>, more: Vec<Part>| {
        v.extend(more);
        v
    };
    commands.insert_resource(WornKit {
        pieces: vec![
            ("Burlap Trousers", trousers(&burlap)),
            ("Hide Pants", trousers(&hide)),
            ("Pants", trousers(&denim)),
            ("Burlap Shoes", boots(&burlap)),
            ("Hide Boots", boots(&hide)),
            ("Boots", boots(&leather)),
            ("Burlap Gloves", gloves(&burlap)),
            ("Leather Gloves", gloves(&leather)),
            (
                "Hoodie",
                with(
                    vec![part(tunic_top.clone(), &fleece, Bone::Spine)],
                    sleeves(&fleece, false),
                ),
            ),
            (
                "Hide Vest",
                vec![part(tunic_top.clone(), &hide, Bone::Spine)],
            ),
            (
                "Jacket",
                with(
                    vec![part(coat.clone(), &olive, Bone::Spine)],
                    sleeves(&olive, true),
                ),
            ),
            (
                "Snow Jacket",
                with(
                    vec![part(parka.clone(), &snow, Bone::Spine)],
                    sleeves(&snow, true),
                ),
            ),
            ("Bandana", vec![part(kerchief, &red, Bone::Head)]),
            (
                "Improvised Balaclava",
                vec![part(hood.clone(), &burlap, Bone::Head)],
            ),
            ("Beenie Hat", vec![part(cap.clone(), &red, Bone::Head)]),
            ("Burlap Hood", vec![part(hood.clone(), &burlap, Bone::Head)]),
            ("Hide Hood", vec![part(hood.clone(), &hide, Bone::Head)]),
            ("Ash Mask", vec![part(mask, &ash, Bone::Head)]),
            // The suit is the tunic's cut in the mask's blacked cloth: two
            // parts, so a helmet over it still fits `MAX_PARTS`.
            (
                "Ash Suit",
                vec![
                    part(tunic_top.clone(), &ash, Bone::Spine),
                    part(tunic_skirt.clone(), &ash, Bone::Hips),
                ],
            ),
            (
                "Bone Helmet",
                vec![
                    part(dome.clone(), &bone, Bone::Head),
                    part(crest, &bone, Bone::Head),
                ],
            ),
            (
                "Burlap Tunic",
                vec![
                    part(tunic_top.clone(), &burlap, Bone::Spine),
                    part(tunic_skirt.clone(), &burlap, Bone::Hips),
                ],
            ),
            ("Hide Poncho", vec![part(poncho, &hide, Bone::Spine)]),
            // Rust's hazmat: hood and suit in one, so three parts, worn
            // alone (`combat::wear_clashes`).
            (
                "Hazmat Suit",
                vec![
                    part(hood.clone(), &hazmat, Bone::Head),
                    part(tunic_top.clone(), &hazmat, Bone::Spine),
                    part(tunic_skirt.clone(), &hazmat, Bone::Hips),
                ],
            ),
            (
                "Coffee Can Helmet",
                vec![part(dome.clone(), &sign, Bone::Head)],
            ),
            (
                "Tank Top",
                vec![part(tunic_top.clone(), &cotton, Bone::Spine)],
            ),
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
        commands
            .entity(body)
            .insert(WornRig::under(body, &children, &names, &skinned));
    }
}

impl WornRig {
    /// The bones and the skinned mesh under `body`, once its scene has
    /// spawned. [`bind`]'s walk, and the portrait's (`portrait.rs`).
    pub fn under(
        body: Entity,
        children: &Query<&Children>,
        names: &Query<&Name>,
        skinned: &Query<(), With<SkinnedMesh>>,
    ) -> Self {
        let mut rig = WornRig {
            bones: [None; BONE_NAMES.len()],
            skin: None,
            shown: WornLook::NONE,
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
        rig
    }

    /// Take everything off: the portrait's, back on the menu.
    pub fn strip(&mut self, commands: &mut Commands) {
        for p in self.parts.iter_mut() {
            if let Some(e) = p.take() {
                commands.entity(e).despawn();
            }
        }
        self.shown = WornLook::NONE;
        self.pending = false;
    }
}

/// What of `look` this client can draw, and how: each slot's skin tint if
/// the skin catalog knows the skin, and the look with every skin it does
/// not know zeroed — what [`WornRig::shown`] records.
pub fn drawable(
    look: WornLook,
    cat: &protocol::SkinCatalog,
) -> (WornLook, [Option<[f32; 3]>; WEAR_SLOTS]) {
    let mut out = look;
    let mut tints = [None; WEAR_SLOTS];
    for (skin, tint) in out.skins.iter_mut().zip(tints.iter_mut()) {
        *tint = crate::ui::skins::tint_of(cat, *skin);
        if tint.is_none() {
            *skin = 0;
        }
    }
    (out, tints)
}

/// Keep each remote body dressed in what the shard says it wears. Work only
/// when a `Worn` landed (`ClientCore::worn_gen`), the skin catalog moved
/// (`skins_gen`: a skin may have become drawable) or a body is waiting.
#[allow(clippy::too_many_arguments)]
pub fn dress(
    mut commands: Commands,
    net: NonSend<Net>,
    kit: Option<Res<WornKit>>,
    binds: Res<Assets<SkinnedMeshInverseBindposes>>,
    skins: Query<&SkinnedMesh>,
    mut skin_mats: ResMut<super::viewmodel::SkinMats>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bodies: Query<(&Body, &mut WornRig)>,
    mut last_gen: Local<(u32, u32)>,
) {
    let Some(kit) = kit else { return };
    let core = &net.session.core;
    let moved = (core.worn_gen, core.skins_gen) != *last_gen;
    *last_gen = (core.worn_gen, core.skins_gen);
    for (body, mut rig) in &mut bodies {
        if !moved && !rig.pending {
            continue;
        }
        put_on(
            &mut commands,
            &kit,
            (&binds, &skins),
            (&mut skin_mats, &mut materials),
            core,
            &mut rig,
            core.worn_of(body.0),
        );
    }
}

/// Dress one rig in `look`, if it is not already showing it: [`dress`]'s
/// body, and the portrait's (`portrait.rs`), which wears your own.
pub fn put_on(
    commands: &mut Commands,
    kit: &WornKit,
    (binds, skins): (&Assets<SkinnedMeshInverseBindposes>, &Query<&SkinnedMesh>),
    (skin_mats, materials): (
        &mut super::viewmodel::SkinMats,
        &mut Assets<StandardMaterial>,
    ),
    core: &client_core::core::ClientCore,
    rig: &mut WornRig,
    look: WornLook,
) {
    let (want, tints) = drawable(look, &core.skins);
    if want == rig.shown && !rig.pending {
        return;
    }
    let Some(skin) = rig.skin.and_then(|e| skins.get(e).ok()) else {
        return;
    };
    let Some(ibm) = binds.get(&skin.inverse_bindposes) else {
        rig.pending = true;
        return;
    };
    let mut pending = false;
    let mut names: [&str; WEAR_SLOTS] = [""; WEAR_SLOTS];
    for (name, &item) in names.iter_mut().zip(want.items.iter()) {
        if item == sim_core::gather::NO_ITEM {
            continue;
        }
        match crate::ui::craft::item_name(&core.catalog, item) {
            Some(n) => *name = n,
            None => pending = true, // the catalog has not dripped this row yet
        }
    }
    // A skin whose material is not ready (a painted one on its way, v110)
    // hangs plain this time and keeps the rig pending, so it is dressed
    // again until the picture is on it.
    let mut unready = false;
    hang(
        commands,
        kit,
        skin,
        ibm,
        rig,
        &names,
        |slot, base| match tints[slot] {
            Some(t) => skin_mats
                .for_part(want.skins[slot], t, base, materials)
                .unwrap_or_else(|| {
                    unready = true;
                    base.clone()
                }),
            None => base.clone(),
        },
    );
    rig.shown = want;
    rig.pending = pending || unready;
}

/// Take off whatever `rig` shows and hang the pieces named in `names` (the
/// catalog's display names; an empty or unknown name hangs nothing) on its
/// bones, each part in the material `look` gives for its slot (the index in
/// `names`) and its own material — that, or a skin's tinted copy of it.
/// [`dress`]'s body, and `examples/worn_look.rs`'s.
pub fn hang(
    commands: &mut Commands,
    kit: &WornKit,
    skin: &SkinnedMesh,
    ibm: &[Mat4],
    rig: &mut WornRig,
    names: &[&str],
    mut look: impl FnMut(usize, &Handle<StandardMaterial>) -> Handle<StandardMaterial>,
) {
    for p in rig.parts.iter_mut() {
        if let Some(e) = p.take() {
            commands.entity(e).despawn();
        }
    }
    let mut n = 0;
    for (slot, name) in names.iter().enumerate() {
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
                    MeshMaterial3d(look(slot, &part.material)),
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
/// The Burlap Tunic: its top (Spine) and its skirt (Hips). **One painted
/// skin covers both** (`ui::skins::CANVASES`): the top takes the image's
/// upper half, collar at the top edge, and the skirt the lower half, hem at
/// the bottom edge, so a design runs on across the waist. A public contract
/// (`DECISIONS.md:432`), pinned by `tests/skin_templates.rs`.
pub fn tunic_meshes() -> [Mesh; 2] {
    let c = Vec2::new(0.0, -0.076);
    let top = lathe(
        c,
        &[(0.93, 0.19, 0.17), (1.26, 0.185, 0.165), (1.34, 0.13, 0.12)],
        None,
    );
    let skirt = lathe(c, &[(0.58, 0.215, 0.195), (0.97, 0.19, 0.17)], None);
    [banded(top, 0.0, 0.5), banded(skirt, 0.5, 1.0)]
}

/// A lathe's V (0 at its lowest ring, 1 at its highest) moved into the band
/// `v0..v1` of the image, upside down: the highest ring at `v0`, the top.
fn banded(mut m: Mesh, v0: f32, v1: f32) -> Mesh {
    if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) =
        m.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    {
        for t in uv.iter_mut() {
            t[1] = v0 + (v1 - v0) * (1.0 - t[1]);
        }
    }
    m
}

fn lathe(c: Vec2, profile: &[Ring], window: Option<(f32, f32, f32)>) -> Mesh {
    const SEG: usize = 28;
    let rings = profile.len();
    // V by arc length up the profile (0 at the lowest ring, 1 at the
    // highest), so painted cloth is not squeezed at a short shoulder.
    let mut arc = Vec::with_capacity(rings);
    let mut run = 0.0f32;
    for i in 0..rings {
        if i > 0 {
            let (y0, ax, az) = profile[i - 1];
            let (y1, bx, bz) = profile[i];
            let dr = (bx + bz - ax - az) * 0.5;
            run += ((y1 - y0).powi(2) + dr * dr).sqrt();
        }
        arc.push(run);
    }
    let total = run.max(1e-6);
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
            uv.push([u, arc[i] / total]);
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

/// A tapering tube in body space from `a` to `b` (clothing v1: a trouser
/// leg, a sleeve, a boot's shaft), radius `ra` at `a` and `rb` at `b`,
/// capped at neither end. The rig's limbs are not vertical (its arms hang
/// in an A), so this is the lathe's ring laid along any axis: the ring's
/// frame is the axis and the two directions square to it.
pub fn tube(a: Vec3, b: Vec3, ra: f32, rb: f32) -> Mesh {
    const SEG: usize = 14;
    let axis = (b - a).normalize_or(Vec3::Y);
    let side = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::Z };
    let u = axis.cross(side).normalize_or(Vec3::X);
    let v = axis.cross(u);
    let mut pos = Vec::with_capacity((SEG + 1) * 2);
    let mut nor = Vec::with_capacity(pos.capacity());
    let mut uv = Vec::with_capacity(pos.capacity());
    // The slope of the taper leans every normal toward the narrow end.
    let lean = (ra - rb) / (b - a).length().max(1e-4);
    for (t, (c, r)) in [(0.0, (a, ra)), (1.0, (b, rb))] {
        for k in 0..=SEG {
            let th = k as f32 / SEG as f32 * std::f32::consts::TAU;
            let (st, ct) = th.sin_cos();
            let out = u * ct + v * st;
            pos.push((c + out * r).to_array());
            nor.push((out + axis * lean).normalize_or(out).to_array());
            uv.push([k as f32 / SEG as f32, t]);
        }
    }
    let mut idx = Vec::with_capacity(SEG * 6);
    for k in 0..SEG as u32 {
        let (p, q) = (k, k + SEG as u32 + 1);
        idx.extend_from_slice(&[p, q, p + 1, p + 1, q, q + 1]);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A worn skin is drawn only if the catalog knows it, and `shown`
    /// records an unknown one as plain — so the catalog landing it later
    /// is a change [`dress`] acts on, not a look it thinks it already drew.
    #[test]
    fn an_unknown_worn_skin_is_drawn_plain_until_the_catalog_knows_it() {
        let mut look = WornLook::NONE;
        look.items[0] = 41;
        look.items[1] = 40;
        look.skins[1] = 0x0B72;
        let mut cat = protocol::SkinCatalog::EMPTY;
        let (shown, tints) = drawable(look, &cat);
        assert_eq!(shown.items, look.items);
        assert_eq!((shown.skins, tints), ([0; WEAR_SLOTS], [None; WEAR_SLOTS]));

        cat.count = 1;
        cat.set(
            0,
            b"Ember",
            protocol::SkinRow {
                catalog: 0x0B72,
                covers: 40,
                tint: [255, 0, 0],
                coin: protocol::COIN_NONE,
                price: 0,
                texture: [0; 8],
                unlock: false,
            },
        )
        .unwrap();
        let (now, tints) = drawable(look, &cat);
        assert_eq!(now, look, "drawn in its skin");
        let mut want = [None; WEAR_SLOTS];
        want[1] = Some([1.0, 0.0, 0.0]);
        assert_eq!(tints, want);
        assert_ne!(now, shown, "and that reads as a change");
    }
}
