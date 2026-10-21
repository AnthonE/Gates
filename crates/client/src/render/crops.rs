//! Farming v2's plants, drawn: one root per plant the server has told this
//! client of (`ClientCore::plants`), at its soil point, turned by a hash of
//! its id and scaled by its stage, with the children its stage shows — a
//! patch of turned soil for a seed, a sprout for a seedling, the crop's own
//! mesh from sapling on, its fruit while fruiting and ripe (corn's ears, the
//! pumpkins on the vine; a bush's berries only once ripe, and a potato's
//! haulm yellowing instead), and the wilt once it is dying — over a mound of
//! heaped soil when it grows in the open ground.
//!
//! **Bevy draws, it does not decide.** Every fact here is the wire record;
//! nothing is predicted. The reconcile runs only on frames the plant set
//! moved (`APPLIED2_PLANTS`), never on the deploys' flag.

use std::f32::consts::TAU;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use protocol::WirePlant;
use sim_core::farm::{
    LOOK_BERRY, LOOK_CORN, LOOK_COUNT, LOOK_HEMP, LOOK_POTATO, LOOK_PUMPKIN, SPOT_GROUND,
    STAGE_DYING, STAGE_FRUITING, STAGE_RIPE, STAGE_SCALE_PM, STAGE_SEED, STAGE_SEEDLING,
};

use super::feed::Feed;
use super::{fresnel, plants, Net, WorldEntity};

/// A bed plant draws this much of its full size: a planter's soil is
/// shallow, and six full-size crops in 1.6 m would be a hedge.
pub const BED_SCALE: f32 = 0.8;

/// The meshes and materials every crop is drawn from, built once at
/// `Startup`.
#[derive(Resource)]
pub struct CropKit {
    /// Per look: the grown plant's wood (stalk, vine or twigs) and leaves.
    stalk: [Handle<Mesh>; LOOK_COUNT as usize],
    leaves: [Handle<Mesh>; LOOK_COUNT as usize],
    /// Per look, healthy and dry.
    leaf_mat: [[Handle<StandardMaterial>; 2]; LOOK_COUNT as usize],
    /// Per look, the wood's material: a crop's stalk bends on hemp's sway, a
    /// berry bush's twigs on the bush's, so each stays inside its leaves.
    wood_mat: [Handle<StandardMaterial>; LOOK_COUNT as usize],
    /// A ripe potato's haulm, yellowing as the tubers set (a potato shows no
    /// fruit: what it pays is under the soil).
    potato_ripe: Handle<StandardMaterial>,
    sprout: Handle<Mesh>,
    sprout_mat: Handle<StandardMaterial>,
    disc: Handle<Mesh>,
    disc_mat: Handle<StandardMaterial>,
    /// A ground plant's heaped soil, under it at every stage.
    mound: Handle<Mesh>,
    mound_mat: Handle<StandardMaterial>,
    /// Corn's ears.
    fruit: Handle<Mesh>,
    fruit_mat: Handle<StandardMaterial>,
    /// Pumpkins on the vine, two variants.
    pumpkin: [Handle<Mesh>; 2],
    pumpkin_mat: Handle<StandardMaterial>,
    /// A ripe berry bush's berries.
    berries: Handle<Mesh>,
    berry_mat: Handle<StandardMaterial>,
    /// A dying plant's brown: one for the solid parts and corn's straps, and
    /// per look one wearing that look's leaf card (a card's shape is its
    /// alpha).
    wilt_solid: Handle<StandardMaterial>,
    wilt_card: [Handle<StandardMaterial>; LOOK_COUNT as usize],
    /// The planting ghost's sprout (`render::farm_ghost`), and its red for
    /// ground the sim would refuse.
    pub ghost: Handle<StandardMaterial>,
    pub ghost_no: Handle<StandardMaterial>,
    pub ghost_mesh: Handle<Mesh>,
}

/// Corn's straps are solid geometry; every other look's leaves are cards.
fn card_leaves(look: u8) -> bool {
    look != LOOK_CORN
}

/// Where a vine's pumpkins lie, in the plant's frame: out from the root
/// along the ground, each turned its own way.
pub(super) const PUMPKIN_AT: [(f32, f32, f32); 2] = [(0.2, 0.05, 0.4), (-0.14, -0.17, 2.1)];

/// Build the kit and register the swaying materials with the foliage
/// shader (`props.rs`'s pattern).
pub fn build_kit(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    server: Option<Res<AssetServer>>,
    foliage: Option<ResMut<super::foliage::Foliages>>,
) {
    // In look order: the arrays below index by look.
    const _: () = assert!(
        LOOK_CORN == 0
            && LOOK_HEMP == 1
            && LOOK_POTATO == 2
            && LOOK_PUMPKIN == 3
            && LOOK_BERRY == 4
    );
    let looks: [(Mesh, Mesh); LOOK_COUNT as usize] = [
        plants::corn_stalk(0),
        plants::hemp(1),
        plants::leafy_mound(2),
        plants::pumpkin_vine(3),
        (plants::berry_wood(4), plants::berry_leaves(4)),
    ];
    let looks = looks.map(|(s, l)| (meshes.add(s), meshes.add(l)));
    let stalk = std::array::from_fn(|k| looks[k].0.clone());
    let leaves = std::array::from_fn(|k| looks[k].1.clone());
    // Each look's card: hemp's, the potato's and the pumpkin's drawn here,
    // the berry bush's the wild bush's photographed atlas (an unresolved
    // handle where nothing loads, as `props::assets` takes it).
    let bush_card = server
        .map(|s| s.load_with_settings(super::props::BUSH_CARD_ATLAS, super::textures::atlas(true)))
        .unwrap_or_default();
    let maps: [Option<Handle<Image>>; LOOK_COUNT as usize] = [
        None,
        Some(images.add(plants::hemp_leaf_image())),
        Some(images.add(plants::potato_leaf_image())),
        Some(images.add(plants::pumpkin_leaf_image())),
        Some(bush_card),
    ];
    let sheen = Color::linear_rgb(
        super::tree::CANOPY_SPECULAR_TINT,
        super::tree::CANOPY_SPECULAR_TINT,
        super::tree::CANOPY_SPECULAR_TINT,
    );
    // A thirsty plant's leaves pale toward straw.
    let tint = |dry: bool| -> Color {
        if dry {
            Color::linear_rgb(1.25, 1.0, 0.45)
        } else {
            Color::WHITE
        }
    };
    let mut leaf = |look: usize, base: Color| {
        let card = card_leaves(look as u8);
        materials.add(StandardMaterial {
            base_color: base,
            base_color_texture: maps[look].clone(),
            perceptual_roughness: 0.9,
            reflectance: fresnel::DIELECTRIC,
            specular_tint: sheen,
            alpha_mode: if card {
                AlphaMode::Mask(super::props::BUSH_CARD_ALPHA_CUT)
            } else {
                AlphaMode::Opaque
            },
            cull_mode: None,
            double_sided: true,
            ..default()
        })
    };
    let leaf_mat: [[Handle<StandardMaterial>; 2]; LOOK_COUNT as usize] =
        std::array::from_fn(|k| [leaf(k, tint(false)), leaf(k, tint(true))]);
    let potato_ripe = leaf(LOOK_POTATO as usize, Color::linear_rgb(1.55, 1.2, 0.3));
    let wilt_card: [Handle<StandardMaterial>; LOOK_COUNT as usize] =
        std::array::from_fn(|k| leaf(k, Color::linear_rgb(0.5, 0.3, 0.12)));
    let solid = |materials: &mut Assets<StandardMaterial>, c: Color, rough: f32| {
        materials.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: rough,
            reflectance: fresnel::DIELECTRIC,
            cull_mode: None,
            double_sided: true,
            ..default()
        })
    };
    let stalk_mat = solid(&mut materials, Color::WHITE, 0.8);
    let bush_wood = solid(&mut materials, Color::WHITE, 0.85);
    let wood_mat: [Handle<StandardMaterial>; LOOK_COUNT as usize] = std::array::from_fn(|k| {
        if k as u8 == LOOK_BERRY {
            bush_wood.clone()
        } else {
            stalk_mat.clone()
        }
    });
    let sprout_mat = solid(&mut materials, Color::WHITE, 0.85);
    let disc_mat = solid(&mut materials, Color::linear_rgb(0.035, 0.024, 0.014), 0.95);
    let mound_mat = solid(&mut materials, Color::linear_rgb(0.05, 0.032, 0.018), 0.97);
    let fruit_mat = solid(&mut materials, Color::linear_rgb(0.62, 0.42, 0.04), 0.55);
    // A pumpkin's skin is in its vertex colours; it has a waxy sheen.
    let pumpkin_mat = solid(&mut materials, Color::WHITE, 0.5);
    let berry_mat = solid(&mut materials, Color::WHITE, 0.32);
    let wilt_solid = solid(&mut materials, Color::linear_rgb(0.16, 0.10, 0.045), 0.95);
    let ghost = materials.add(StandardMaterial {
        base_color: Color::srgba(0.55, 0.95, 0.5, 0.45),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    let ghost_no = materials.add(StandardMaterial {
        base_color: Color::srgba(0.86, 0.28, 0.22, 0.4),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    if let Some(mut f) = foliage {
        use super::foliage::Kind;
        for (look, mats) in leaf_mat.iter().enumerate() {
            let kind = match look as u8 {
                // Corn's straps are solid: they bend with the stalk and are
                // never cut edge-on.
                LOOK_CORN => Kind::HempStem,
                // The berry bush is the wild bush, on the bush's bend.
                LOOK_BERRY => Kind::BushLeaf,
                _ => Kind::HempLeaf,
            };
            for m in mats {
                f.register(m, kind);
            }
        }
        f.register(&potato_ripe, Kind::HempLeaf);
        f.register(&stalk_mat, Kind::HempStem);
        f.register(&bush_wood, Kind::Stem);
        f.register(&berry_mat, Kind::Stem);
        f.register(&sprout_mat, Kind::HempStem);
    }
    let sprout = meshes.add(plants::sprout(0));
    commands.insert_resource(CropKit {
        stalk,
        leaves,
        leaf_mat,
        wood_mat,
        potato_ripe,
        sprout: sprout.clone(),
        sprout_mat,
        disc: meshes.add(plants::soil_disc()),
        disc_mat,
        mound: meshes.add(plants::soil_mound()),
        mound_mat,
        fruit: meshes.add(Capsule3d::new(0.035, 0.11)),
        fruit_mat,
        pumpkin: [
            meshes.add(plants::pumpkin(0)),
            meshes.add(plants::pumpkin(1)),
        ],
        pumpkin_mat,
        berries: meshes.add(plants::berries(4, true)),
        berry_mat,
        wilt_solid,
        wilt_card,
        ghost,
        ghost_no,
        ghost_mesh: sprout,
    });
}

/// One drawn plant: what it was drawn as, so a change redraws it, and the
/// reconcile that last saw it in the mirror.
pub struct CropLive {
    pub entity: Entity,
    pub look: u8,
    pub stage: u8,
    pub dry: bool,
    seen: u32,
}

/// Every drawn plant by id, whether the set moved on a frame this
/// reconcile could not run, and the reconcile's generation.
#[derive(Resource, Default)]
pub struct CropRing {
    pub live: HashMap<u32, CropLive>,
    pub pending: bool,
    gen: u32,
}

/// The set moved this frame. Runs wherever the world does, so a sync that
/// lands while the world is still being placed is not lost.
pub fn note_plants(feed: Res<Feed>, mut ring: ResMut<CropRing>) {
    if feed.applied2 & client_core::core::APPLIED2_PLANTS != 0 && !ring.pending {
        ring.pending = true;
    }
}

/// [`stream_crops`]' run condition: only on a frame the plant set moved.
pub fn crops_changed(feed: Res<Feed>, ring: Res<CropRing>) -> bool {
    ring.pending || feed.applied2 & client_core::core::APPLIED2_PLANTS != 0
}

/// Forget the last world's plants (their roots went with its entities).
pub fn teardown(mut ring: ResMut<CropRing>) {
    *ring = CropRing::default();
}

/// A plant's turn: a hash of its id, so every client turns it alike.
fn yaw_of(id: u32) -> f32 {
    let h = id.wrapping_mul(0x9E37_79B9) >> 24;
    h as f32 / 256.0 * TAU
}

/// The root's uniform scale.
fn scale_of(p: &WirePlant) -> f32 {
    let s = STAGE_SCALE_PM
        .get(p.stage as usize)
        .copied()
        .unwrap_or(1000) as f32
        / 1000.0;
    if p.spot != SPOT_GROUND {
        s * BED_SCALE
    } else {
        s
    }
}

/// Reconcile the drawn plants against the mirror: despawn what left, spawn
/// what arrived, respawn what changed look, stage or dryness.
pub fn stream_crops(
    mut commands: Commands,
    mut ring: ResMut<CropRing>,
    kit: Option<Res<CropKit>>,
    net: Option<NonSend<Net>>,
) {
    let (Some(kit), Some(net)) = (kit, net) else {
        return;
    };
    let ring = &mut *ring;
    ring.pending = false;
    ring.gen = ring.gen.wrapping_add(1);
    let gen = ring.gen;
    for p in net.session.core.plants.entries() {
        if let Some(live) = ring.live.get_mut(&p.id) {
            if live.look == p.look && live.stage == p.stage && live.dry == p.dry {
                live.seen = gen;
                continue;
            }
            commands.entity(live.entity).despawn();
        }
        let entity = spawn_crop(&mut commands, &kit, p);
        ring.live.insert(
            p.id,
            CropLive {
                entity,
                look: p.look,
                stage: p.stage,
                dry: p.dry,
                seen: gen,
            },
        );
    }
    ring.live.retain(|_, live| {
        if live.seen != gen {
            commands.entity(live.entity).despawn();
            return false;
        }
        true
    });
}

/// One plant's root and its stage's children.
fn spawn_crop(commands: &mut Commands, kit: &CropKit, p: &WirePlant) -> Entity {
    let (x, y, z) = p.pos();
    let s = scale_of(p);
    let root = Transform::from_xyz(x, y, z)
        .with_rotation(Quat::from_rotation_y(yaw_of(p.id)))
        .with_scale(Vec3::splat(s));
    let mut e = commands.spawn((WorldEntity, root, Visibility::Inherited));
    let look = (p.look as usize).min(LOOK_COUNT as usize - 1);
    // A seed and a sprout are drawn at their own size, not the stage's:
    // the turned soil and two leaves are what a seed looks like at any
    // scale, and 6% of a corn plant is a speck.
    let absolute = Transform::from_scale(Vec3::splat(1.0 / s.max(1e-3)));
    // A ground plant stands in its own heaped soil, at its own size.
    if p.spot == SPOT_GROUND {
        e.with_child((
            Mesh3d(kit.mound.clone()),
            MeshMaterial3d(kit.mound_mat.clone()),
            absolute,
        ));
    }
    match p.stage {
        STAGE_SEED => {
            e.with_child((
                Mesh3d(kit.disc.clone()),
                MeshMaterial3d(kit.disc_mat.clone()),
                absolute,
            ));
        }
        STAGE_SEEDLING => {
            e.with_children(|c| {
                c.spawn((
                    Mesh3d(kit.disc.clone()),
                    MeshMaterial3d(kit.disc_mat.clone()),
                    absolute,
                ));
                c.spawn((
                    Mesh3d(kit.sprout.clone()),
                    MeshMaterial3d(kit.sprout_mat.clone()),
                    Transform::from_scale(Vec3::splat(1.6 / s.max(1e-3))),
                ));
            });
        }
        _ => {
            let dying = p.stage == STAGE_DYING;
            let leaf = if dying {
                if card_leaves(p.look) {
                    kit.wilt_card[look].clone()
                } else {
                    kit.wilt_solid.clone()
                }
            } else if p.look == LOOK_POTATO && p.stage == STAGE_RIPE {
                kit.potato_ripe.clone()
            } else {
                kit.leaf_mat[look][usize::from(p.dry)].clone()
            };
            let wood = if dying {
                kit.wilt_solid.clone()
            } else {
                kit.wood_mat[look].clone()
            };
            let fruiting = matches!(p.stage, STAGE_FRUITING | STAGE_RIPE);
            e.with_children(|c| {
                c.spawn((
                    Mesh3d(kit.stalk[look].clone()),
                    MeshMaterial3d(wood),
                    Transform::IDENTITY,
                ));
                c.spawn((
                    Mesh3d(kit.leaves[look].clone()),
                    MeshMaterial3d(leaf),
                    Transform::IDENTITY,
                ));
                if fruiting && p.look == LOOK_CORN {
                    // Two ears on the stalk, leaning out from it.
                    let h = sim_core::farm::LOOK_H_M[LOOK_CORN as usize];
                    for (k, y) in [(0.0f32, 0.48f32), (TAU * 0.5, 0.58)] {
                        let out = Vec3::new(k.sin(), 0.0, k.cos());
                        c.spawn((
                            Mesh3d(kit.fruit.clone()),
                            MeshMaterial3d(kit.fruit_mat.clone()),
                            Transform::from_translation(Vec3::Y * (h * y) + out * 0.05)
                                .with_rotation(Quat::from_axis_angle(
                                    Vec3::new(out.z, 0.0, -out.x),
                                    0.45,
                                )),
                        ));
                    }
                }
                if fruiting && p.look == LOOK_PUMPKIN {
                    // Two pumpkins on the ground among the leaves: green
                    // fruit at six tenths their size, then grown.
                    let k = if p.stage == STAGE_RIPE { 1.0 } else { 0.6 };
                    for (i, &(x, z, yaw)) in PUMPKIN_AT.iter().enumerate() {
                        c.spawn((
                            Mesh3d(kit.pumpkin[i].clone()),
                            MeshMaterial3d(kit.pumpkin_mat.clone()),
                            Transform::from_xyz(x, 0.0, z)
                                .with_rotation(Quat::from_rotation_y(yaw))
                                .with_scale(Vec3::splat(k)),
                        ));
                    }
                }
                if p.stage == STAGE_RIPE && p.look == LOOK_BERRY {
                    c.spawn((
                        Mesh3d(kit.berries.clone()),
                        MeshMaterial3d(kit.berry_mat.clone()),
                        Transform::IDENTITY,
                    ));
                }
            });
        }
    }
    e.id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bed_plant_draws_smaller_and_every_stage_has_a_size() {
        let mut p = WirePlant {
            id: 3,
            stage: STAGE_RIPE,
            spot: 2,
            ..WirePlant::default()
        };
        assert!((scale_of(&p) - BED_SCALE).abs() < 1e-6);
        p.spot = SPOT_GROUND;
        assert!((scale_of(&p) - 1.0).abs() < 1e-6);
        for stage in 0..=STAGE_DYING {
            p.stage = stage;
            assert!(scale_of(&p) > 0.0);
        }
        // Two ids, two turns.
        assert_ne!(yaw_of(1), yaw_of(2));
    }
}
