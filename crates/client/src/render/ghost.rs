//! The build ghost: the cell you are aiming at, coloured by whether the sim
//! would take it — and the right-click that commits it.
//!
//! `NOW.md` §0w item 1: *"No build ghost, so the wheel latches and never
//! places."* The radial menu has shipped since 2026-08-06 and its own header
//! says it "latches a piece and does not place one", because **placing a
//! piece the player cannot see the destination of spends materials on a
//! guess**. This is the destination.
//!
//! All arithmetic is [`crate::ui::place`]'s. This module owns one mesh, one
//! transform, two materials and a mouse button.
//!
//! ## What the colour promises
//!
//! Green means *nothing this client can see refuses it* — not "the server
//! will take it". `place::verdict` checks the four refusals a client can
//! check (spot taken, reach, ground, cost) and is silent on support, hearth
//! claims and world capacity. So a green ghost can still be refused, and the
//! refusal arrives on the toast like any other. What must never happen is the
//! other way round: a red ghost on something the sim would have accepted,
//! which is why every check in `verdict` is one the sim runs the same way.

use bevy::prelude::*;
use sim_core::build::{
    BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_W, SHAPE_DOORWAY, SHAPE_STAIRS, SHAPE_WALL,
};
use sim_core::collide::WALL_THICKNESS_M;
use sim_core::limits::MAX_BUILD_LEVELS;

use crate::look::yaw_u16;
use crate::ui::build::{row_for, MATERIALS, SHAPES};
use crate::ui::place::{self, Site, Target, Verdict};

use super::hud::Toast;
use super::input::Look;
use super::panels::Ui;
use super::structures::{cell_center, level_base_y, SEAM_M, SLAB_T};
use super::{Net, WorldId};

/// The ghost's translucency, and its two verdicts. Cosmetics
/// (`DECISIONS.md` §open, client cosmetics).
const GHOST_OK: Color = Color::srgba(0.42, 0.78, 0.36, 0.38);
const GHOST_NO: Color = Color::srgba(0.86, 0.28, 0.22, 0.34);

/// The one ghost entity, and what it is currently showing.
#[derive(Resource, Default)]
pub struct Ghost {
    entity: Option<Entity>,
    ok_mat: Option<Handle<StandardMaterial>>,
    no_mat: Option<Handle<StandardMaterial>>,
    mesh: Option<Handle<Mesh>>,
    /// The working level the `R`/`F` steppers move. Client-side latch, like
    /// the wheel's own shape and material.
    pub level: u8,
    /// Where it is and what it decided, so `place` acts on exactly what is
    /// drawn — the prompt-and-verb rule from `verbs`.
    pub target: Target,
    pub verdict: Verdict,
    pub row: Option<u16>,
    pub shape: u8,
}

/// `R` raises the working level, `F` lowers it.
///
/// **Only while the wheel is up**, which is what keeps `R` free for repair
/// in the world (`web/src/main.js` orders the same two branches the same way
/// and pins the ordering in a gate, because R means two things and the one
/// that wins is decided by whether build mode is on).
pub fn level_keys(keys: Res<ButtonInput<KeyCode>>, ui: Option<Res<Ui>>, mut ghost: ResMut<Ghost>) {
    let wheel_up = ui
        .map(|u| u.panel == super::panels::Panel::Wheel)
        .unwrap_or(false);
    if !wheel_up {
        return;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        ghost.level = (ghost.level + 1).min(MAX_BUILD_LEVELS as u8 - 1);
    }
    if keys.just_pressed(KeyCode::KeyF) {
        ghost.level = ghost.level.saturating_sub(1);
    }
}

/// Park the ghost over the aimed address, or hide it when there is nothing
/// to draw.
// Eight, and each is a distinct source this frame reads: the spawner, the
// ghost's own state, the two asset stores it builds into on first use, the
// seed, the session, the view, and which panel is up.
#[allow(clippy::too_many_arguments)]
pub fn track(
    mut commands: Commands,
    mut ghost: ResMut<Ghost>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
    look: Res<Look>,
    ui: Option<Res<Ui>>,
) {
    let ghost = &mut *ghost;
    if ghost.mesh.is_none() {
        // ONE unit cube, scaled per shape. A mesh per shape would be five
        // more pipeline specializations for a thing that is drawn
        // translucent and never inspected — and a pipeline created after the
        // world is up is the prewarm trap (`CLAUDE.md`, `RENDER.md` §2).
        ghost.mesh = Some(meshes.add(Cuboid::new(1.0, 1.0, 1.0)));
        ghost.ok_mat = Some(materials.add(translucent(GHOST_OK)));
        ghost.no_mat = Some(materials.add(translucent(GHOST_NO)));
    }

    // The wheel decides what is being placed. With it down there is no
    // pending placement and therefore nothing to aim.
    let Some(ui) = ui else {
        hide(&mut commands, ghost);
        return;
    };
    if ui.panel != super::panels::Panel::Wheel {
        hide(&mut commands, ghost);
        return;
    }

    let core = &net.session.core;
    let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
    let material = MATERIALS[ui.material.min(MATERIALS.len() - 1)];
    let Some(row) = row_for(&core.piece_defs, shape, material) else {
        // The content has no piece for this pair — the wheel already draws
        // that segment dead, and a ghost would contradict it.
        hide(&mut commands, ghost);
        return;
    };

    let [x, _, z] = core.predict.render_position();
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
    let target = place::target(x, z, fx, fz, shape, ghost.level);
    let verdict = place::verdict(
        target,
        row,
        shape,
        &Site {
            seed: world.seed,
            at: (x, z),
            taken: core.pieces.entries(),
            content: &core.piece_defs,
            inv: &core.inv,
        },
    );
    ghost.target = target;
    ghost.verdict = verdict;
    ghost.row = Some(row);
    ghost.shape = shape;

    let (scale, offset) = shape_box(shape, target.loc);
    let base_y = level_base_y(world.seed, target.cx, target.cz, target.level);
    let (cxm, czm) = cell_center(target.cx, target.cz);
    let pos = match target.loc {
        LOC_EDGE_W => Vec3::new(target.cx as f32 * BUILD_CELL_M, base_y, czm),
        sim_core::build::LOC_EDGE_N => Vec3::new(cxm, base_y, target.cz as f32 * BUILD_CELL_M),
        _ => Vec3::new(cxm, base_y, czm),
    } + offset;
    // North edges are the west shape turned a quarter — the same rotation
    // `structures::spawn_piece` gives the real thing, so the ghost and the
    // piece it becomes are the same object in the same pose.
    let yaw = if target.loc == sim_core::build::LOC_EDGE_N {
        std::f32::consts::FRAC_PI_2
    } else {
        0.0
    };
    let transform = Transform::from_translation(pos)
        .with_rotation(Quat::from_rotation_y(yaw))
        .with_scale(scale);
    let mat = if verdict.ok() {
        ghost.ok_mat.clone()
    } else {
        ghost.no_mat.clone()
    }
    .expect("built above");

    match ghost.entity {
        Some(e) => {
            commands
                .entity(e)
                .insert((transform, MeshMaterial3d(mat), Visibility::Visible));
        }
        None => {
            let e = commands
                .spawn((
                    super::WorldEntity,
                    Mesh3d(ghost.mesh.clone().expect("built above")),
                    MeshMaterial3d(mat),
                    transform,
                ))
                .id();
            ghost.entity = Some(e);
        }
    }
}

/// Right-click places what the ghost is showing.
///
/// Left is the swing and always has been; right is the place, which is the
/// browser's binding and the genre's. It acts on the ghost's own latched
/// target rather than re-aiming, so what was drawn is what is sent.
pub fn place_key(
    mouse: Res<ButtonInput<MouseButton>>,
    ghost: Res<Ghost>,
    net: NonSend<Net>,
    mut toast: ResMut<Toast>,
    ui: Option<Res<Ui>>,
) {
    let wheel_up = ui
        .map(|u| u.panel == super::panels::Panel::Wheel)
        .unwrap_or(false);
    if !wheel_up || !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(row) = ghost.row else {
        return;
    };
    // The local verdict is advisory, and refusing to SEND on it would make
    // it authoritative — including its blind spots. So a red ghost still
    // sends and still gets the server's own sentence back; the colour is
    // there to stop the player wasting the press, not to veto it.
    let t = ghost.target;
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_place(row, t.cx, t.cz, t.level, t.loc, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => {
                if let Verdict::No(why) = ghost.verdict {
                    if !why.is_empty() {
                        toast.say(why);
                    }
                }
            }
            Err(e) => toast.say(e.to_string()),
        },
        Err(e) => toast.say(format!("that placement would not encode ({e:?})")),
    }
}

/// Right-click **outside** build mode places the held deployable.
///
/// Without this a box, a bag and a furnace cannot be put down at all — which
/// keeps the container panel and respawn-on-bag unreachable however well they
/// are drawn. `DeployDef::item` is the item placement consumes, so the held
/// hotbar slot IS the choice and no second wheel is owed; the reference does
/// it exactly this way.
///
/// The aimed address is `place::target` with a plane shape, because every
/// deployable that is not a door stands on the cell body. A door goes in a
/// doorway edge, and the sim answers `REFUSE_D_DOOR` for one aimed anywhere
/// else — the client does not try to guess which, because guessing wrong
/// costs the player the item.
pub fn deploy_key(
    mouse: Res<ButtonInput<MouseButton>>,
    net: NonSend<Net>,
    look: Res<Look>,
    mut toast: ResMut<Toast>,
    ui: Option<Res<Ui>>,
    chat: Option<Res<super::chat::Chat>>,
) {
    let busy = ui
        .map(|u| u.panel != super::panels::Panel::None)
        .unwrap_or(false)
        || chat.map(|c| c.open()).unwrap_or(false);
    if busy || !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let core = &net.session.core;
    let held = core.inv[(net.sel as usize).min(core.inv.len() - 1)];
    let Some(row) =
        crate::ui::structure::row_for_item(&core.deploy_defs, core.deploy_defs_have, held.item)
    else {
        return; // not holding a deployable; nothing to say about it
    };
    let [x, _, z] = core.predict.render_position();
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
    let t = crate::ui::place::target(x, z, fx, fz, sim_core::build::SHAPE_FOUNDATION, 0);
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_deploy(row as u16, t.cx, t.cz, t.level, t.loc, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => {}
            Err(e) => toast.say(e.to_string()),
        },
        Err(e) => toast.say(format!("that deployable would not encode ({e:?})")),
    }
}

/// Drop the ghost when the world goes.
pub fn forget(mut ghost: ResMut<Ghost>) {
    *ghost = Ghost::default();
}

fn hide(commands: &mut Commands, ghost: &mut Ghost) {
    if let Some(e) = ghost.entity {
        commands.entity(e).insert(Visibility::Hidden);
    }
}

fn translucent(base_color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color,
        alpha_mode: AlphaMode::Blend,
        // Unlit: a ghost that took the sun would read as a real piece in
        // some orientations and as a stain in others, and it is a readout
        // rather than a thing in the world.
        unlit: true,
        // No shadow and no depth write, so it never darkens the ground it is
        // hovering over or occludes the piece it would replace.
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// The unit cube's scale and its offset from the address's base point, per
/// shape. Mirrors `structures::spawn_piece`'s geometry so the ghost is the
/// size of the thing it becomes.
fn shape_box(shape: u8, _loc: u8) -> (Vec3, Vec3) {
    let span = BUILD_CELL_M - SEAM_M;
    match shape {
        SHAPE_WALL | SHAPE_DOORWAY => (
            Vec3::new(WALL_THICKNESS_M, LEVEL_H_M, span),
            Vec3::new(0.0, LEVEL_H_M * 0.5, 0.0),
        ),
        SHAPE_STAIRS => (
            Vec3::new(span, SLAB_T, 4.15),
            Vec3::new(0.0, LEVEL_H_M * 0.5, 0.0),
        ),
        // Foundation / floor / roof: the slab under the level plane.
        _ => (
            Vec3::new(span, SLAB_T, span),
            Vec3::new(0.0, -SLAB_T * 0.5, 0.0),
        ),
    }
}
