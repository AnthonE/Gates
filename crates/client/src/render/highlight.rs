//! The blue wash over the piece a hammer is aimed at.
//!
//! **What it is for.** The reference's hammer acts on *the block you are
//! looking at*, and it tells you which one before you commit: hold the
//! hammer, look at a wall you own, and it lights up. Ours resolved the same
//! target — `verbs::Near` has done it since the build slice — and drew
//! nothing, so every hammer verb was a keypress into the dark: you found out
//! which piece you had repaired by watching its hit points move.
//!
//! **It reuses the piece's own mesh and transform rather than deriving
//! them.** `structures::spawn_piece`'s addressing is subtle — edge pieces
//! are canonical to a cell's west or north boundary, so one physical edge is
//! never addressable twice, and a wall on a north edge carries a quarter-turn
//! that a west one does not. A highlight that computed its own placement
//! would be a second implementation of that, and `panels/wheel.rs` states
//! what a second implementation of "which one is this" costs: the thing you
//! see and the thing you act on stop being the same thing. So this reads
//! `Transform` and `Mesh3d` off the entity `StructRing` already spawned.
//!
//! **One entity, moved.** Not spawn-and-despawn per frame: the target changes
//! every time the player turns, and a per-frame spawn is an allocation on the
//! frame path for something with one instance.

use bevy::prelude::*;

use super::structures::StructRing;
use super::Net;
use crate::ui::hold::held_in_hand;
use crate::ui::structure::Store;

/// How much larger than the piece the wash is drawn, as a fraction.
///
/// Small and non-zero on purpose. Zero z-fights with the piece it covers —
/// two coplanar surfaces at the same depth — and anything much larger reads
/// as a box around the piece rather than the piece being lit.
const SWELL: f32 = 1.012;

/// The wash. Blue for the reason the placement ghost is blue: it is the
/// reference's colour for "this is the thing", and the two are the same
/// statement about different verbs.
const WASH: Color = Color::srgba(0.34, 0.62, 0.96, 0.26);

/// The single highlight entity, and what it is currently over.
#[derive(Resource, Default)]
pub struct Highlight {
    entity: Option<Entity>,
    /// The material, made once. A `StandardMaterial` per frame would leak
    /// handles into `Assets` for the lifetime of the session.
    material: Option<Handle<StandardMaterial>>,
    /// What it is on, so a frame that changes nothing does nothing.
    on: Option<Entity>,
}

/// Wash the aimed piece while a hammer is held.
///
/// Runs after `verbs::resolve`, which is what puts the target in `Near` —
/// the same ordering `verbs::keys` needs and for the same reason: what is
/// lit has to be this frame's answer, or the player lights one piece and
/// repairs another.
#[allow(clippy::too_many_arguments)]
pub fn track(
    mut commands: Commands,
    mut hi: ResMut<Highlight>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    net: NonSend<Net>,
    near: Res<super::verbs::Near>,
    ring: Res<StructRing>,
    pieces: Query<(&Transform, &Mesh3d)>,
    ui: Option<Res<super::panels::Ui>>,
) {
    let core = &net.session.core;
    let hand = held_in_hand(&core.catalog, &core.inv, net.sel);
    // A panel owns the pointer, and a wash under an open inventory is a
    // highlight on something the player cannot act on.
    let busy = ui
        .map(|u| u.panel != super::panels::Panel::None)
        .unwrap_or(false);

    let want = if hand.repairs() && !busy {
        near.0.as_ref().and_then(|t| {
            ring.entity_at(
                (t.cx, t.cz, t.level, t.loc),
                matches!(t.store, Store::Deploy),
            )
        })
    } else {
        None
    };

    if want == hi.on {
        return;
    }
    hi.on = want;

    let Some(target) = want else {
        hide(&mut commands, &mut hi);
        return;
    };
    // The piece may have streamed out between `resolve` and here.
    let Ok((transform, mesh)) = pieces.get(target) else {
        hide(&mut commands, &mut hi);
        return;
    };

    let material = hi
        .material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: WASH,
                alpha_mode: AlphaMode::Blend,
                // Unlit: the wash says "this one", and a lit one would read
                // as a material on the piece — dark on its shaded faces,
                // which is the opposite of a highlight.
                unlit: true,
                // Not written to depth, so the piece behind it still reads.
                depth_bias: 1.0,
                ..default()
            })
        })
        .clone();

    let placement = transform.with_scale(transform.scale * SWELL);
    match hi.entity {
        Some(e) => {
            commands
                .entity(e)
                .insert((mesh.clone(), placement, Visibility::Visible));
        }
        None => {
            let e = commands
                .spawn((
                    super::WorldEntity,
                    mesh.clone(),
                    MeshMaterial3d(material),
                    placement,
                ))
                .id();
            hi.entity = Some(e);
        }
    }
}

fn hide(commands: &mut Commands, hi: &mut Highlight) {
    if let Some(e) = hi.entity {
        commands.entity(e).insert(Visibility::Hidden);
    }
}

/// Drop the highlight when the world goes.
///
/// `WorldEntity` already despawns the entity with everything else; this is
/// the resource's half, and without it the next world starts holding a stale
/// `Entity` that `pieces.get` would answer `Err` for forever — which reads
/// as a highlight that never appears again.
pub fn forget_in(hi: &mut Highlight) {
    hi.entity = None;
    hi.on = None;
}
