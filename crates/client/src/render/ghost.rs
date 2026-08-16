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

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use client_core::core::ClientCore;
use sim_core::build::{SHAPE_FOUNDATION, SHAPE_TRI_FOUNDATION};
use sim_core::limits::MAX_BUILD_LEVELS;

use crate::look::yaw_u16;
use crate::ui::build::{row_for, PLACE_MATERIAL, SHAPES};
use crate::ui::place::{self, DeploySite, DeployVerdict, Site, Target, Verdict};

use super::hud::Toast;
use super::input::{pitch_u8, Look};
use super::panels::Ui;
use super::structures::{self, base_transform, deploy_transform, shape_parts};
use super::{Net, WorldId, EYE_HEIGHT};

/// The planar point the ghost aims at: the LOOK ray — eye, yaw AND pitch,
/// the tracer's own ray convention, so the ghost sits where a shot would
/// land — marched into the world by [`place::aim_from_look`] against
/// terrain and the predictor's piece surfaces.
///
/// Until 2026-08-15 the aim was `feet + yaw · 3.5 m`, pitch discarded: the
/// crosshair rested on one cell and the ghost stood on another, which is
/// how that day's playtest read `SPOT TAKEN` off a gap. The ray is the
/// crosshair now; the fixed projection survives inside `aim_from_look` as
/// the sky/out-of-range fallback.
fn aim_point(seed: u64, core: &ClientCore, look: &Look, feet: [f32; 3]) -> (f32, f32) {
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(look.yaw));
    let (ch, sv) = sim_core::pitch_dir(pitch_u8(look.pitch));
    place::aim_from_look(
        seed,
        core.cols(),
        [feet[0], feet[1] + EYE_HEIGHT, feet[2]],
        [fx * ch, sv, fz * ch],
        (feet[0], feet[2]),
    )
}

/// The ghost's translucency, and its two verdicts. Cosmetics
/// (`DECISIONS.md` §open, client cosmetics).
// **Blue, not green, and that is measured off the reference's own
// behaviour rather than chosen**: its building guides describe the ghost as
// lighting up "bright blue" when it is ready to place and red or orange when
// it is not. Ours was green, which is the generic engine answer.
const GHOST_OK: Color = Color::srgba(0.34, 0.62, 0.96, 0.40);
const GHOST_NO: Color = Color::srgba(0.86, 0.28, 0.22, 0.34);
/// The deploy preview's colour. **Neutral on purpose, and re-picked after a
/// merge that silently invalidated the first choice.**
///
/// It has to be neither of the two above, because those two mean something:
/// blue is "ready to place" and red is "refused" — and while the deploy
/// ghost now computes a verdict (`place::deploy_verdict`), that verdict can
/// only ever say NO or UNKNOWN, never yes: the claim and the capacity caps
/// are the server's alone, so "nothing visible refuses it" may not wear the
/// ready colour. Neutral is the honest face of Unknown; a refused deploy
/// wears [`GHOST_NO`] like a refused piece. The first version of this
/// constant was a pale blue-grey chosen to contrast with a build ghost that
/// was GREEN at the time. The reference-blue change landed on `main` in the
/// same window and the two merged without touching a common line — leaving a
/// deploy preview a shade off the build ghost's own blue, silently promising
/// the readiness it explicitly must not. Warm bone grey instead: adjacent to
/// neither hue.
const GHOST_DEPLOY: Color = Color::srgba(0.87, 0.84, 0.76, 0.32);

/// The one ghost entity, and what it is currently showing.
#[derive(Resource, Default)]
pub struct Ghost {
    entity: Option<Entity>,
    /// Which shape the current children were built for, so they are rebuilt on
    /// a shape CHANGE and not every frame. A doorway is three children; a wall
    /// is one. Respawning them per frame would churn entities on the client's
    /// hot path for a value that changes when the player turns a wheel.
    built_shape: Option<u8>,
    /// The deploy preview — a separate entity from the build ghost because the
    /// two are never up at once but are driven by different systems, and one
    /// entity shared between them would need a mode flag that could disagree.
    deploy_entity: Option<Entity>,
    deploy_mat: Option<Handle<StandardMaterial>>,
    ok_mat: Option<Handle<StandardMaterial>>,
    no_mat: Option<Handle<StandardMaterial>>,
    mesh: Option<Handle<Mesh>>,
    /// The unit half-cell prism beside the unit cube (triangles v0) —
    /// `structures::part_mesh` builds both, so the preview's triangle is
    /// the piece's.
    tri_mesh: Option<Handle<Mesh>>,
    /// The working level the `R`/`F` steppers move. Client-side latch, like
    /// the wheel's own shape and material.
    pub level: u8,
    /// Where it is and what it decided, so `place` acts on exactly what is
    /// drawn — the prompt-and-verb rule from `verbs`.
    pub target: Target,
    pub verdict: Verdict,
    pub row: Option<u16>,
    pub shape: u8,
    /// The deploy ghost's own latched pair, for the same rule: `deploy_key`
    /// sends the address that was drawn, and the HUD says the reason the
    /// drawing is red. Defaults are the empty address and `Unknown` —
    /// neutral, never red, on a frame nobody computed.
    pub deploy_target: Target,
    pub deploy_verdict: DeployVerdict,
}

/// `R` raises the working level, `F` lowers it.
///
/// **Only while the wheel is up**, which is what keeps `R` free for repair
/// in the world (`web/src/main.js` orders the same two branches the same way
/// and pins the ordering in a gate, because R means two things and the one
/// that wins is decided by whether build mode is on).
pub fn level_keys(keys: Res<ButtonInput<KeyCode>>, ui: Option<Res<Ui>>, mut ghost: ResMut<Ghost>) {
    // Level nudges belong to the wheel, which is the plan's. Held-item
    // modality means they are dead keys the rest of the time.
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
    children: Query<&Children>,
) {
    let ghost = &mut *ghost;
    if ghost.mesh.is_none() {
        // ONE unit cube, scaled per shape — plus the one unit prism the
        // triangle shapes need (triangles v0). Two meshes, not one per
        // shape: both carry the standard vertex layout, so the second is
        // the same pipeline, not a new specialization — the prewarm trap
        // (`CLAUDE.md`, `RENDER.md` §2) is about pipelines, not meshes.
        ghost.mesh = Some(meshes.add(Cuboid::new(1.0, 1.0, 1.0)));
        ghost.tri_mesh = Some(meshes.add(structures::part_mesh(&structures::Part {
            size: Vec3::ONE,
            offset: Vec3::ZERO,
            x_rot: 0.0,
            kind: structures::PartKind::Tri,
        })));
        ghost.ok_mat = Some(materials.add(translucent(GHOST_OK)));
        ghost.no_mat = Some(materials.add(translucent(GHOST_NO)));
    }

    // The wheel decides what is being placed. With it down there is no
    // pending placement and therefore nothing to aim.
    let Some(ui) = ui else {
        hide(&mut commands, ghost);
        return;
    };
    // **Shown while the PLAN is held, wheel open or not.** It used to be
    // shown only while the wheel was open, which paired with a right-click
    // place to make building a sequence of menus: hold the wheel, click
    // through it, release, repeat. The reference keeps the ghost up for as
    // long as the plan is out and places with repeated left clicks, and the
    // ghost is what makes that flow legible.
    //
    // The inventory still hides it: that screen owns the pointer, and a
    // preview of a placement you cannot make is noise.
    let core = &net.session.core;
    let hand = crate::ui::hold::held_in_hand(&core.catalog, &core.inv, net.sel);
    if !hand.shows_ghost() || ui.panel == super::panels::Panel::Inventory {
        hide(&mut commands, ghost);
        return;
    }

    let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
    let material = PLACE_MATERIAL;
    let Some(row) = row_for(&core.piece_defs, shape, material) else {
        // The content has no piece for this pair — the wheel already draws
        // that segment dead, and a ghost would contradict it.
        hide(&mut commands, ghost);
        return;
    };

    let [x, y, z] = core.predict.render_position();
    let aim = aim_point(world.seed, core, &look, [x, y, z]);
    let target = place::target_at(aim.0, aim.1, shape, ghost.level);
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

    // The same base point and quarter-turn `structures::spawn_piece` gives
    // the real thing, from the same function, so the ghost and the piece it
    // becomes are the same object in the same pose.
    let transform = base_transform(world.seed, (target.cx, target.cz, target.level, target.loc));
    let mat = if verdict.ok() {
        ghost.ok_mat.clone()
    } else {
        ghost.no_mat.clone()
    }
    .expect("built above");

    let root = match ghost.entity {
        Some(e) => {
            commands.entity(e).insert((transform, Visibility::Visible));
            e
        }
        None => {
            let e = commands
                .spawn((super::WorldEntity, transform, Visibility::Visible))
                .id();
            ghost.entity = Some(e);
            e
        }
    };

    // Rebuild the children only when the shape changes. `despawn_related` drops
    // the previous set; a shape that keeps its part count would still be
    // rebuilt, which is fine because the trigger is a wheel turn.
    // A foundation's one part is per-ADDRESS, not per shape: the skirt
    // depth follows the terrain under the aimed cell
    // (`structures::foundation_part` — the same emit the standing piece
    // draws, so the preview promises exactly the object the click buys).
    let footing = matches!(shape, SHAPE_FOUNDATION | SHAPE_TRI_FOUNDATION).then(|| {
        structures::foundation_part(
            world.seed,
            target.cx,
            target.cz,
            shape == SHAPE_TRI_FOUNDATION,
        )
    });

    if ghost.built_shape != Some(shape) {
        ghost.built_shape = Some(shape);
        commands.entity(root).despawn_related::<Children>();
        let mesh = ghost.mesh.clone().expect("built above");
        let tri = ghost.tri_mesh.clone().expect("built above");
        // The shared table (`structures::shape_parts`): one unit mesh —
        // the cube, or the prism for a Tri part — scaled per part, where
        // the piece will use a real-size mesh per part. Same sizes, same
        // offsets, same pitch, one emit site. The foundations' one part is
        // the per-address footing computed above instead of the table's
        // fixed-thickness fallback arm.
        let (mut parts, n) = shape_parts(shape);
        if let Some(part) = footing {
            parts[0] = part;
        }
        commands.entity(root).with_children(|c| {
            for part in &parts[..n] {
                let unit = match part.kind {
                    structures::PartKind::Box => mesh.clone(),
                    structures::PartKind::Tri => tri.clone(),
                };
                c.spawn((
                    Mesh3d(unit),
                    MeshMaterial3d(mat.clone()),
                    // **The header has always claimed "no shadow" and the code
                    // never said so.** A translucent mesh still casts one in
                    // Bevy unless this component is present, so the ghost was
                    // laying a hard shadow on the ground it hovers over —
                    // exactly the "darkens the ground" the comment promised it
                    // would not do. A comment is not an implementation.
                    NotShadowCaster,
                    part.transform().with_scale(part.size),
                ));
            }
        });
    } else {
        // Same shape, so the verdict colour can have moved — and for a
        // foundation, the footing's depth with the aimed cell.
        let kids: Vec<Entity> = children
            .get(root)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        for k in kids {
            commands.entity(k).insert(MeshMaterial3d(mat.clone()));
            if let Some(part) = footing {
                commands
                    .entity(k)
                    .insert(part.transform().with_scale(part.size));
            }
        }
    }
}

/// **Left**-click places what the ghost is showing, while the plan is held.
///
/// Two things about this were backwards until 2026-08-07, and both were
/// checked against the reference before moving:
///
/// - it was **right**-click, which is the reference's *menu* button, not its
///   place button. Its building guides are unambiguous: hold right for the
///   radial, left click to place.
/// - it only fired **while the wheel was open**, so placing meant holding
///   the wheel open and clicking through it. The reference closes the wheel,
///   keeps the ghost, and places with repeated left clicks — which is what
///   makes building a base a flow rather than a sequence of menus.
///
/// Left click is free to mean this because the building plan has no attack.
///
/// It acts on the ghost's own latched target rather than re-aiming, so what
/// was drawn is what is sent.
pub fn place_key(
    mouse: Res<ButtonInput<MouseButton>>,
    ghost: Res<Ghost>,
    net: NonSend<Net>,
    mut toast: ResMut<Toast>,
    ui: Option<Res<Ui>>,
) {
    // Not while any panel owns the pointer: a left click on the wheel is a
    // wedge being chosen, and a left click in the inventory is a drag.
    let busy = ui
        .map(|u| u.panel != super::panels::Panel::None)
        .unwrap_or(false);
    let holding_plan =
        crate::ui::hold::held_in_hand(&net.session.core.catalog, &net.session.core.inv, net.sel)
            .places();
    if busy || !holding_plan || !mouse.just_pressed(MouseButton::Left) {
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
                        toast.warn(why);
                    }
                }
            }
            Err(e) => toast.warn(e.to_string()),
        },
        Err(e) => toast.warn(format!("that placement would not encode ({e:?})")),
    }
}

/// Park a translucent preview over where the held deployable would land —
/// and say WHETHER, where that is honestly computable (`NOW.md` §0u item 2).
///
/// **The deploy path had no ghost at all, and `deploy_key`'s own header says
/// why that matters**: "the client does not try to guess which, because
/// guessing wrong costs the player the item." A build placement the sim
/// refuses costs a click; a deployable placed at the wrong address costs the
/// box. The build ghost has existed since `NOW.md` §0w item 1 and this half
/// was never built, so the riskier of the two verbs was the blind one.
///
/// **The colour split, and the honesty rule behind it.** The verdict is
/// [`place::deploy_verdict`] — the sim's own predicates run on the client's
/// own mirror, never a copy of them (its doc states which `REFUSE_D_*`
/// reasons are mirrored and which the mirror genuinely cannot hold: the
/// hearth claim needs crew lists the wire never carries, the caps are the
/// server's store lengths). A mirrored NO draws [`GHOST_NO`] with the
/// refusal's own sentence on the HUD line; everything else stays the neutral
/// [`GHOST_DEPLOY`], because "nothing visible refuses it" may never wear the
/// ready blue — that would promise a claim check nobody can run, the exact
/// mirror of the module header's forbidden direction.
///
/// **A door previews in its edge** (`NOW.md` §0u item 3): a doorway-class
/// deployable aims an edge address (`place::deploy_target`) and the box is
/// posed by `structures::deploy_transform` — the one emit site the standing
/// deployable uses — so the preview stands exactly where the placed door
/// will, in the doorway rather than on the cell body.
// Nine, and each is a distinct source this frame reads — the same shape and
// the same justification `track` above carries.
#[allow(clippy::too_many_arguments)]
pub fn deploy_track(
    mut commands: Commands,
    mut ghost: ResMut<Ghost>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
    look: Res<Look>,
    ui: Option<Res<Ui>>,
    chat: Option<Res<super::chat::Chat>>,
) {
    let ghost = &mut *ghost;
    if ghost.deploy_mat.is_none() {
        ghost.deploy_mat = Some(materials.add(translucent(GHOST_DEPLOY)));
        if ghost.mesh.is_none() {
            ghost.mesh = Some(meshes.add(Cuboid::new(1.0, 1.0, 1.0)));
        }
        // The red is shared with the build ghost, but either system can be
        // the first to need it (a player can hold a box before ever holding
        // the plan), so both build it on first use.
        if ghost.no_mat.is_none() {
            ghost.no_mat = Some(materials.add(translucent(GHOST_NO)));
        }
    }
    let busy = ui
        .map(|u| u.panel != super::panels::Panel::None)
        .unwrap_or(false)
        || chat.map(|c| c.open()).unwrap_or(false);
    if busy {
        hide_deploy(&mut commands, ghost);
        return;
    }
    let core = &net.session.core;
    let held = core.inv[(net.sel as usize).min(core.inv.len() - 1)];
    let Some(row) =
        crate::ui::structure::row_for_item(&core.deploy_defs, core.deploy_defs_have, held.item)
    else {
        // Not holding a deployable — the same silence `deploy_key` keeps.
        hide_deploy(&mut commands, ghost);
        return;
    };
    // `row_for_item` returns a u8 row; the def table is a slice. Bounded
    // rather than trusted: `deploy_defs` is drip-fed on join (`RENDER.md` §8),
    // so a row can name a def that has not arrived yet.
    let Some(def) = core.deploy_defs.defs.get(row as usize) else {
        hide_deploy(&mut commands, ghost);
        return;
    };
    let arch = def.arch as usize;
    let size = super::structures::deploy_size(arch);

    let [x, y, z] = core.predict.render_position();
    let aim = aim_point(world.seed, core, &look, [x, y, z]);
    // A doorway-class deployable resolves an edge (the level is the build
    // ghost's working latch — placing a doorway at L1 leaves the latch
    // there, so the door that follows it aims the same storey); everything
    // else keeps `deploy_key`'s original plane target at level 0.
    let t = place::deploy_target_at(aim.0, aim.1, def.placement, ghost.level);
    let verdict = place::deploy_verdict(
        t,
        row,
        &DeploySite {
            seed: world.seed,
            at: (x, z),
            pieces: core.pieces.entries(),
            piece_defs: &core.piece_defs,
            piece_have: core.piece_defs_have,
            deploys: core.deploys.entries(),
            deploy_defs: &core.deploy_defs,
            deploy_have: core.deploy_defs_have,
            inv: &core.inv,
        },
    );
    ghost.deploy_target = t;
    ghost.deploy_verdict = verdict;

    // The one pose site (`structures::deploy_transform`, closed): the ghost
    // and the deployable it becomes are the same box in the same place —
    // for a door, in the doorway's edge.
    let transform = deploy_transform(world.seed, (t.cx, t.cz, t.level, t.loc), arch as u8, false)
        .with_scale(size);
    let mat = if verdict.refused() {
        ghost.no_mat.clone()
    } else {
        ghost.deploy_mat.clone()
    }
    .expect("built above");

    match ghost.deploy_entity {
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
                    NotShadowCaster,
                    transform,
                ))
                .id();
            ghost.deploy_entity = Some(e);
        }
    }
}

fn hide_deploy(commands: &mut Commands, ghost: &mut Ghost) {
    if let Some(e) = ghost.deploy_entity {
        commands.entity(e).insert(Visibility::Hidden);
    }
    // A hidden ghost has no verdict: the HUD line must not keep saying a
    // reason about an aim that is no longer drawn.
    ghost.deploy_verdict = DeployVerdict::Unknown;
}

/// Right-click **outside** build mode places the held deployable.
///
/// Without this a box, a bag and a furnace cannot be put down at all — which
/// keeps the container panel and respawn-on-bag unreachable however well they
/// are drawn. `DeployDef::item` is the item placement consumes, so the held
/// hotbar slot IS the choice and no second wheel is owed; the reference does
/// it exactly this way.
///
/// It acts on the ghost's own latched target rather than re-aiming —
/// `place_key`'s rule, "what was drawn is what is sent" — which is what lets
/// a door be sent at the doorway EDGE its preview stands in
/// (`place::deploy_target`; `deploy_track` runs earlier in the same chain
/// under the same guards, so the latch is this frame's). The local verdict
/// stays advisory exactly as the build one is: a red ghost still sends, and
/// the sentence rides the toast so the press teaches the rule.
pub fn deploy_key(
    mouse: Res<ButtonInput<MouseButton>>,
    net: NonSend<Net>,
    ghost: Res<Ghost>,
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
    let t = ghost.deploy_target;
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_deploy(row as u16, t.cx, t.cz, t.level, t.loc, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => {
                if let DeployVerdict::No(why) = ghost.deploy_verdict {
                    if !why.is_empty() {
                        toast.warn(why);
                    }
                }
            }
            Err(e) => toast.warn(e.to_string()),
        },
        Err(e) => toast.warn(format!("that deployable would not encode ({e:?})")),
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
