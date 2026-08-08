//! What players built: placed pieces, deployables, and the backpacks the
//! dead leave behind.
//!
//! **The gap this closes was the widest one in the native client.** Every
//! one of these three sets was decoded into `ClientCore` and drawn by
//! nothing, so a player standing inside someone's base saw bare terrain, a
//! door they could not see swung silently, and the bag holding their own
//! loot was invisible at the spot they died. `world.rs` puts them on the
//! wire, `core.rs` calls its mirrors "the renderer's truth", and until now
//! there was no renderer.
//!
//! Geometry is `web/src/scene.js`'s `setPiece`/`setDeploy`/`setBags`, carried
//! across rather than re-invented: both clients must agree about where a wall
//! stands, because the sim's collision is a third opinion and it is the one
//! that wins. The dimensions that ARE collision truth come from
//! `sim_core::collide` by import (`PIECE_LIFT_M`, `WALL_THICKNESS_M`,
//! `DOOR_POST_W_M`) rather than by copied literal — one class of drift the
//! browser could not close and this can.
//!
//! ## Reconciled, not evented
//!
//! `ClientCore` exposes both a per-message delta (`piece_changes()`) and the
//! whole mirror (`pieces`, `deploys`, `bags`). This reads the mirror. The
//! delta is cheaper per frame and wrong at exactly the moment it matters: a
//! resync walk restates the world, a removal restarts an in-progress walk,
//! and a renderer driven off deltas has to reproduce that state machine
//! correctly or leave a wall standing where the server has none. Reading the
//! set makes that desync impossible by construction, and the set is bounded
//! (`MAX_PIECES` 8192), not unbounded.
//!
//! **No per-frame allocation**, which is why the entity maps carry a
//! generation stamp instead of building a live-key set each frame and
//! diffing it: mark what the mirror still holds, then `retain` the marked.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::build::{
    BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_N, LOC_EDGE_W, SHAPE_DOORWAY, SHAPE_STAIRS, SHAPE_WALL,
};
use sim_core::collide::{DOOR_POST_W_M, PIECE_LIFT_M, WALL_THICKNESS_M};
use sim_core::deploy::{
    DeployRec, ARCH_BAG, ARCH_BOX, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE, ARCH_HEARTH, ARCH_WORKBENCH,
};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain;

use super::{Net, WorldId};

/// Plane-piece thickness, metres. Cosmetic — the sim's plane is a surface
/// height and not a slab (`collide.rs`), so this is how thick we *draw* it
/// and nothing stands on the underside.
pub const SLAB_T: f32 = 0.3;

/// The seam a drawn piece leaves at its cell boundary, metres. Without it,
/// two abutting floors z-fight along their shared edge for the whole length
/// of a base. `scene.js` carries the same 0.04 for the same reason.
pub const SEAM_M: f32 = 0.04;

/// Wood, stone, metal — colour, perceptual roughness, metallic.
///
/// Cosmetics (`DECISIONS.md` §open, client cosmetics). The response matters
/// as much as the colour: `ART.md` reads the reference's tier as *sheen* as
/// much as hue, so metal is a conductor with a real specular lobe and wood is
/// flat. A tier told apart by colour alone reads as three paints.
const TIER: [(Color, f32, f32); 3] = [
    (Color::srgb(0.541, 0.416, 0.271), 0.88, 0.0), // wood  0x8a6a45
    (Color::srgb(0.518, 0.514, 0.486), 0.72, 0.0), // stone 0x84837c
    (Color::srgb(0.373, 0.416, 0.447), 0.38, 0.85), // metal 0x5f6a72
];

/// Deployable stand-ins by archetype (`sim_core::deploy` order: bag, hearth,
/// box, fire, furnace, workbench, door): full size `w × h × d` in metres,
/// colour, roughness, metallic. Cosmetics, same registry row.
const DEPLOY: [([f32; 3], Color, f32, f32); 7] = [
    (
        [1.2, 0.25, 0.7],
        Color::srgb(0.478, 0.612, 0.306),
        0.92,
        0.0,
    ), // bag
    ([0.9, 0.9, 0.9], Color::srgb(0.549, 0.231, 0.180), 0.80, 0.0), // hearth
    ([1.0, 0.7, 1.0], Color::srgb(0.478, 0.361, 0.227), 0.85, 0.0), // box
    ([0.7, 0.4, 0.7], Color::srgb(0.816, 0.439, 0.188), 0.75, 0.0), // fire
    ([1.1, 1.5, 1.1], Color::srgb(0.310, 0.290, 0.271), 0.70, 0.0), // furnace
    ([1.6, 0.9, 0.9], Color::srgb(0.631, 0.475, 0.247), 0.85, 0.0), // workbench
    (
        [0.12, 2.1, 0.9],
        Color::srgb(0.420, 0.290, 0.169),
        0.82,
        0.0,
    ), // door
];

/// A locked door wears banded iron: the one bit of door state a passer-by
/// can read off the outside, and the thing they would have to break.
const DOOR_LOCKED: Color = Color::srgb(0.235, 0.247, 0.267);

/// The death backpack (`backpack.rs`) — a low canvas bundle where a body
/// fell, in the sleeping bag's cloth.
const BAG_SIZE: [f32; 3] = [0.6, 0.35, 0.45];
const BAG_COLOR: Color = Color::srgb(0.627, 0.416, 0.235);

/// A grid address: the key both placed stores are addressed by.
pub type Addr = (u16, u16, u8, u8);

/// One drawn thing, and enough of what it was drawn *as* to know when the
/// drawing is stale. An upgrade keeps the address and changes the row; a
/// door swing keeps the row and changes the pose. Both must redraw, and
/// neither shows up as an address appearing or vanishing.
struct Live {
    entity: Entity,
    seen: u64,
    row: u8,
    open: bool,
    locked: bool,
}

/// Shared meshes and materials, built once on first use. A base is hundreds
/// of pieces over five shapes and three materials; one `StandardMaterial`
/// per piece would be one draw call per piece.
struct Kit {
    slab: Handle<Mesh>,
    wall: Handle<Mesh>,
    post: Handle<Mesh>,
    lintel: Handle<Mesh>,
    stairs: Handle<Mesh>,
    tier: [Handle<StandardMaterial>; 3],
    deploy_mesh: [Handle<Mesh>; 7],
    deploy_mat: [Handle<StandardMaterial>; 7],
    door_locked: Handle<StandardMaterial>,
    bag_mesh: Handle<Mesh>,
    bag_mat: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
pub struct StructRing {
    pieces: HashMap<Addr, Live>,
    deploys: HashMap<Addr, Live>,
    bags: HashMap<u32, Live>,
    kit: Option<Kit>,
    gen: u64,
}

impl StructRing {
    /// The entity drawing the piece or deployable at `addr`, if one stands.
    ///
    /// Exists for the hammer's highlight, which needs the thing the player is
    /// looking at rather than a second derivation of where it would be. The
    /// addressing arithmetic in `spawn_piece` is subtle enough — edge pieces
    /// are canonical to a cell's west or north boundary, so the same physical
    /// edge is never addressable twice — that a highlight computing its own
    /// transform would be a second implementation of it, and the wheel's
    /// oldest rule says what that costs.
    pub fn entity_at(&self, addr: Addr, deploy: bool) -> Option<Entity> {
        let map = if deploy { &self.deploys } else { &self.pieces };
        map.get(&addr).map(|l| l.entity)
    }

    /// Standing counts: pieces, deployables, bags. For the gates and for
    /// nothing on the hot path.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.pieces.len(), self.deploys.len(), self.bags.len())
    }
}

/// The world y a piece at `level` sits at, given the terrain under its cell.
///
/// **This is collision truth, not a look.** `collide.rs`'s header states the
/// same expression — `terrain height + PIECE_LIFT_M + level·LEVEL_H_M` — and
/// calls it "the renderer's formula", because the sim walks players on a
/// surface derived from it. A renderer that drew the floor 10 cm off would
/// put every player ankle-deep in it or hovering above it, and no gate would
/// say so: the sim would be right and the picture wrong.
pub fn level_base_y(seed: u64, cx: u16, cz: u16, level: u8) -> f32 {
    let (cxm, czm) = cell_center(cx, cz);
    terrain::height(seed, cxm, czm) + PIECE_LIFT_M + level as f32 * LEVEL_H_M
}

/// The world XZ of a cell's centre.
pub fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    )
}

fn build_kit(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Kit {
    let tier = std::array::from_fn(|i| {
        let (base_color, perceptual_roughness, metallic) = TIER[i];
        materials.add(StandardMaterial {
            base_color,
            perceptual_roughness,
            metallic,
            ..default()
        })
    });
    let deploy_mesh = std::array::from_fn(|i| {
        let [w, h, d] = DEPLOY[i].0;
        meshes.add(Cuboid::new(w, h, d))
    });
    let deploy_mat = std::array::from_fn(|i| {
        let (_, base_color, perceptual_roughness, metallic) = DEPLOY[i];
        materials.add(StandardMaterial {
            base_color,
            perceptual_roughness,
            metallic,
            ..default()
        })
    });
    let span = BUILD_CELL_M - SEAM_M;
    Kit {
        slab: meshes.add(Cuboid::new(span, SLAB_T, span)),
        wall: meshes.add(Cuboid::new(WALL_THICKNESS_M, LEVEL_H_M, span)),
        post: meshes.add(Cuboid::new(WALL_THICKNESS_M, LEVEL_H_M, DOOR_POST_W_M)),
        // The lintel spans what the two posts leave: the doorway's opening is
        // the intended breach point and it has to read as one.
        lintel: meshes.add(Cuboid::new(
            WALL_THICKNESS_M,
            0.9,
            (span - 2.0 * DOOR_POST_W_M).max(0.1),
        )),
        stairs: meshes.add(Cuboid::new(span, SLAB_T, 4.15)),
        tier,
        deploy_mesh,
        deploy_mat,
        door_locked: materials.add(StandardMaterial {
            base_color: DOOR_LOCKED,
            perceptual_roughness: 0.45,
            metallic: 0.8,
            ..default()
        }),
        bag_mesh: meshes.add(Cuboid::new(BAG_SIZE[0], BAG_SIZE[1], BAG_SIZE[2])),
        bag_mat: materials.add(StandardMaterial {
            base_color: BAG_COLOR,
            perceptual_roughness: 0.95,
            ..default()
        }),
    }
}

/// Reconcile all three stores against the core's mirrors.
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<StructRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
) {
    // One reborrow, then field-level borrows. `ResMut`'s `DerefMut` hands out
    // a borrow of the WHOLE resource, so reading `kit` while inserting into
    // `pieces` is a conflict until the struct is split like this.
    let ring = &mut *ring;
    if ring.kit.is_none() {
        ring.kit = Some(build_kit(&mut meshes, &mut materials));
    }
    let kit = ring.kit.as_ref().expect("built above");
    let core = &net.session.core;
    ring.gen = ring.gen.wrapping_add(1);
    let gen = ring.gen;
    let seed = world.seed;

    // ---- pieces ---------------------------------------------------------
    // A row past `piece_defs_have` has not dripped in yet: its shape and
    // material are unknown, and `PieceDef::INERT` would draw it as a wooden
    // foundation. Skip it — the frame after the defs arrive draws it right,
    // and a wrong wall is worse than a late one.
    let have = core.piece_defs_have.min(core.piece_defs.piece_count);
    for rec in core.pieces.entries() {
        if (rec.row as u16) >= have {
            continue;
        }
        let key = (rec.cx, rec.cz, rec.level, rec.loc);
        if let Some(live) = ring.pieces.get_mut(&key) {
            live.seen = gen;
            if live.row == rec.row {
                continue;
            }
            // An upgrade in place: same address, new material.
            commands.entity(live.entity).despawn();
            ring.pieces.remove(&key);
        }
        let def = core.piece_defs.pieces[rec.row as usize];
        let entity = spawn_piece(&mut commands, kit, seed, key, def.shape, def.material);
        ring.pieces.insert(
            key,
            Live {
                entity,
                seen: gen,
                row: rec.row,
                open: false,
                locked: false,
            },
        );
    }
    ring.pieces.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });

    // ---- deployables ----------------------------------------------------
    let dhave = core.deploy_defs_have.min(core.deploy_defs.def_count);
    for rec in core.deploys.entries() {
        if (rec.row as u16) >= dhave {
            continue;
        }
        let key = (rec.cx, rec.cz, rec.level, rec.loc);
        if let Some(live) = ring.deploys.get_mut(&key) {
            live.seen = gen;
            // A door swing and a lock are both redraws at one address.
            if live.row == rec.row && live.open == rec.open && live.locked == rec.locked {
                continue;
            }
            commands.entity(live.entity).despawn();
            ring.deploys.remove(&key);
        }
        let arch = core.deploy_defs.defs[rec.row as usize].arch;
        let entity = spawn_deploy(&mut commands, kit, seed, rec, arch);
        ring.deploys.insert(
            key,
            Live {
                entity,
                seen: gen,
                row: rec.row,
                open: rec.open,
                locked: rec.locked,
            },
        );
    }
    ring.deploys.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });

    // ---- backpacks ------------------------------------------------------
    // A bag never moves, so a known id is left entirely alone.
    for bag in core.bags.entries() {
        if let Some(live) = ring.bags.get_mut(&bag.id) {
            live.seen = gen;
            continue;
        }
        // The sim drops it at the body's FEET, so half its height lifts it
        // onto the ground rather than leaving it sunk to the waist.
        let pos = Vec3::new(
            bag.qx as f32 * POS_XZ_Q,
            bag.qy as f32 * POS_Y_Q + BAG_SIZE[1] * 0.5,
            bag.qz as f32 * POS_XZ_Q,
        );
        let entity = commands
            .spawn((
                super::WorldEntity,
                Mesh3d(kit.bag_mesh.clone()),
                MeshMaterial3d(kit.bag_mat.clone()),
                Transform::from_translation(pos),
            ))
            .id();
        ring.bags.insert(
            bag.id,
            Live {
                entity,
                seen: gen,
                row: 0,
                open: false,
                locked: false,
            },
        );
    }
    ring.bags.retain(|_, live| {
        if live.seen == gen {
            return true;
        }
        commands.entity(live.entity).despawn();
        false
    });
}

fn spawn_piece(
    commands: &mut Commands,
    kit: &Kit,
    seed: u64,
    (cx, cz, level, loc): Addr,
    shape: u8,
    material: u8,
) -> Entity {
    let mat = kit.tier[(material as usize).min(2)].clone();
    let base_y = level_base_y(seed, cx, cz, level);
    let (cxm, czm) = cell_center(cx, cz);

    if shape == SHAPE_WALL || shape == SHAPE_DOORWAY {
        // Edge pieces stand on the cell's west (x = cx·3) or north
        // (z = cz·3) boundary — canonical, so one physical edge is never
        // addressable twice (`build.rs`).
        let transform = if loc == LOC_EDGE_W {
            Transform::from_xyz(cx as f32 * BUILD_CELL_M, base_y + LEVEL_H_M * 0.5, czm)
        } else {
            Transform::from_xyz(cxm, base_y + LEVEL_H_M * 0.5, cz as f32 * BUILD_CELL_M)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
        };
        if shape == SHAPE_WALL {
            return commands
                .spawn((
                    super::WorldEntity,
                    Mesh3d(kit.wall.clone()),
                    MeshMaterial3d(mat),
                    transform,
                ))
                .id();
        }
        // A doorway keeps its opening: two posts and a lintel over the gap.
        let gap = (BUILD_CELL_M - SEAM_M - DOOR_POST_W_M) * 0.5;
        return commands
            .spawn((super::WorldEntity, transform, Visibility::default()))
            .with_children(|c| {
                c.spawn((
                    Mesh3d(kit.post.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(0.0, 0.0, -gap),
                ));
                c.spawn((
                    Mesh3d(kit.post.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(0.0, 0.0, gap),
                ));
                c.spawn((
                    Mesh3d(kit.lintel.clone()),
                    MeshMaterial3d(mat.clone()),
                    Transform::from_xyz(0.0, LEVEL_H_M * 0.5 - 0.45, 0.0),
                ));
            })
            .id();
    }

    if shape == SHAPE_STAIRS {
        // A ramp through the level. The grid stores no facing, so the ramp
        // always rises toward +Z (cosmetic v0 — the browser's choice too).
        return commands
            .spawn((
                super::WorldEntity,
                Mesh3d(kit.stairs.clone()),
                MeshMaterial3d(mat),
                Transform::from_xyz(cxm, base_y + LEVEL_H_M * 0.5, czm)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)),
            ))
            .id();
    }

    // Foundation / floor / roof: a slab whose TOP is the level plane, which
    // is the surface the sim stands players on.
    commands
        .spawn((
            super::WorldEntity,
            Mesh3d(kit.slab.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(cxm, base_y - SLAB_T * 0.5, czm),
        ))
        .id()
}

fn spawn_deploy(
    commands: &mut Commands,
    kit: &Kit,
    seed: u64,
    rec: &DeployRec,
    arch: u8,
) -> Entity {
    let idx = (arch as usize).min(DEPLOY.len() - 1);
    let [_, h, d] = DEPLOY[idx].0;
    let base_y = level_base_y(seed, rec.cx, rec.cz, rec.level);
    let (cxm, czm) = cell_center(rec.cx, rec.cz);
    let x0 = rec.cx as f32 * BUILD_CELL_M;
    let z0 = rec.cz as f32 * BUILD_CELL_M;
    let y = base_y + h * 0.5;
    let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);

    // A door fills its doorway edge, oriented like the wall there; open, it
    // swings off the hinge end and lies across the cell — the same read the
    // sim's collision has, so a player never walks through a leaf that still
    // looks shut. Everything else stands on the level plane at cell centre.
    let transform = match (rec.loc, rec.open) {
        (LOC_EDGE_W, false) => Transform::from_xyz(x0, y, czm),
        (LOC_EDGE_W, true) => {
            Transform::from_xyz(x0 + d * 0.5, y, z0 + BUILD_CELL_M * 0.5 - d * 0.5)
                .with_rotation(quarter)
        }
        (LOC_EDGE_N, false) => Transform::from_xyz(cxm, y, z0).with_rotation(quarter),
        (LOC_EDGE_N, true) => {
            Transform::from_xyz(x0 + BUILD_CELL_M * 0.5 - d * 0.5, y, z0 + d * 0.5)
        }
        _ => Transform::from_xyz(cxm, y, czm),
    };

    let mat = if arch == ARCH_DOOR && rec.locked {
        kit.door_locked.clone()
    } else {
        kit.deploy_mat[idx].clone()
    };

    commands
        .spawn((
            super::WorldEntity,
            Mesh3d(kit.deploy_mesh[idx].clone()),
            MeshMaterial3d(mat),
            transform,
        ))
        .id()
}

/// Which archetypes a player can open. Stated here because it is a property
/// of the archetype table, not of the key that opens one.
pub fn is_container(arch: u8) -> bool {
    matches!(arch, ARCH_BOX | ARCH_BAG)
}

/// Which archetypes are craft stations — the proximity tokens `craft.rs`
/// gates recipes on.
pub fn is_station(arch: u8) -> bool {
    matches!(
        arch,
        ARCH_WORKBENCH | ARCH_FURNACE | ARCH_FIRE | ARCH_HEARTH
    )
}
