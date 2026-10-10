//! World-space anchors (NOW §0x item 2): the live charge's clock drawn on
//! the charge, and the wall readout drawn at the wall it names — the half
//! the pinned readout (`hud::readout`) left unbuilt. Text that tracks a
//! world point rather than geometry: one UI node per tag, moved each frame
//! to where the eye projects the point, so it stays the HUD's font at the
//! HUD's size however far off the wall is.
//!
//! **Bevy draws, it does not decide.** What is live is `hud::Readout`'s,
//! latched off `ClientCore`; the clock is `ClientCore::charge_left`, on the
//! server-tick estimate; where a tag hangs and whether it draws is
//! `crate::ui::anchor`'s arithmetic, tested in the code tier.
//!
//! No occlusion: a tag draws through the wall between you and it. For a
//! defender that is the point — the charge worth seeing is on the outside —
//! and an honest hide would need the sim's geometry query, not a raycast
//! against render meshes.

use bevy::prelude::*;

use super::hud::Readout;
use super::rig::EyeCam;
use super::{Net, WorldId};
use crate::ui::anchor::{
    clock_secs, range_alpha, screen_frac, structure_point, CHARGE_LIFT_M, WALL_LIFT_M,
};

/// Which fact a tag draws.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// The live charge's clock, `7s`.
    Charge,
    /// Your own blows' wall fraction, `412/500 · 83%`.
    Wall,
}

/// The clock on the charge: bigger than the HUD's lines and a warning red,
/// since it is the one tag a player may have seconds to read.
const CHARGE_FONT_PX: f32 = 22.0;
const CHARGE_RGB: (f32, f32, f32) = (1.0, 0.36, 0.24);
/// The wall's number: the readout line's own size and colour, because it is
/// that line's number, standing where the wall is.
const WALL_FONT_PX: f32 = 14.0;
const WALL_RGB: (f32, f32, f32) = (0.98, 0.82, 0.55);

/// The two tags, hidden until they have something to say. A `WorldEntity`,
/// so leaving the world takes them down with everything else. Public so a
/// headless `App` can spawn them (`tests/anchor_tags.rs`): a bundle is not
/// type-checked for duplicate components, so it is run rather than read.
pub fn setup(mut commands: Commands) {
    for (tag, px, (r, g, b)) in [
        (Tag::Charge, CHARGE_FONT_PX, CHARGE_RGB),
        (Tag::Wall, WALL_FONT_PX, WALL_RGB),
    ] {
        commands.spawn((
            super::WorldEntity,
            tag,
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            // Centred on its point whatever the text's width: a translate
            // by half its own size, resolved after layout.
            UiTransform::from_translation(Val2::percent(-50.0, -50.0)),
            Text::new(""),
            super::ui::font_bold(px),
            TextColor(Color::srgba(r, g, b, 0.0)),
            super::ui::TEXT_SHADOW,
            TextLayout::new_with_justify(Justify::Center),
            Visibility::Hidden,
            Pickable::IGNORE,
        ));
    }
}

/// Move each tag to its structure and say what it holds, or hide it.
///
/// Scheduled in `PostUpdate` after the camera's projection is computed and
/// before UI layout reads `Node`, so the tag sits where this frame's camera
/// sees the wall. The eye is a root entity, so its `Transform` IS its world
/// pose; its `GlobalTransform` would not be propagated yet and would trail
/// a turning head by a frame.
///
/// Writes into one reused buffer and copies only on a change, `hud::update`'s
/// rule (NOW §0pf 3): a frame where neither number moved allocates nothing.
#[allow(clippy::type_complexity)]
pub fn draw(
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    ro: Res<Readout>,
    eye: Query<(&Camera, &Transform), With<EyeCam>>,
    mut tags: Query<(&Tag, &mut Node, &mut Text, &mut TextColor, &mut Visibility)>,
    mut line: Local<String>,
) {
    use std::fmt::Write;
    let (Some(world), Ok((camera, eye_t))) = (world, eye.single()) else {
        return;
    };
    let core = &net.session.core;
    let pose = GlobalTransform::from(*eye_t);
    for (tag, mut node, mut text, mut color, mut vis) in &mut tags {
        line.clear();
        // What the tag says, where it hangs, and the fade it carries.
        let shown = match tag {
            Tag::Charge => ro.charge().and_then(|at| {
                let secs = core.charge_left() as f32 / sim_core::limits::TICK_HZ as f32;
                let _ = write!(line, "{}s", clock_secs(secs)?);
                Some((at, CHARGE_LIFT_M, 1.0))
            }),
            // The clock outranks the wall at one address, as on the pinned
            // line: the charge on the wall you are swinging at is the news.
            Tag::Wall => ro
                .wall()
                .filter(|&(at, ..)| ro.charge() != Some(at))
                .and_then(|(at, left, max, fade)| {
                    super::hud::push_struct_hit(&mut line, left, max).then_some((
                        at,
                        WALL_LIFT_M,
                        fade,
                    ))
                }),
        };
        let placed = shown.and_then(|(at, lift, fade)| {
            let p = Vec3::from(structure_point(world.seed, &world.haven, core, at, lift));
            let ndc = camera.world_to_ndc(&pose, p)?;
            let [fx, fy] = screen_frac(ndc.to_array())?;
            let alpha = fade * range_alpha(p.distance(eye_t.translation));
            (alpha > 0.0).then_some((fx, fy, alpha))
        });
        let Some((fx, fy, alpha)) = placed else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        let (left, top) = (Val::Percent(fx * 100.0), Val::Percent(fy * 100.0));
        if node.left != left || node.top != top {
            node.left = left;
            node.top = top;
        }
        if text.0 != *line {
            text.0.clone_from(&line);
        }
        color.set_if_neq(TextColor(color.0.with_alpha(alpha)));
        vis.set_if_neq(Visibility::Inherited);
    }
}
