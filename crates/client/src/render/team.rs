//! The team on screen (wire v108): the panel on the left, a green mark
//! over each teammate's head within a hundred metres, and the line an
//! invite waiting on you shows. What is shown is `ui::team`'s; this places
//! it. The map's dots are `render::map::mates`.

use bevy::prelude::*;

use super::rig::EyeCam;
use super::{ui, Net};
use protocol::{TEAM_ASLEEP, TEAM_DEAD, TEAM_MAX};

/// The team's green: on the panel, the marks, the map and team chat.
pub const MATE_INK: Color = Color::srgba(0.45, 0.92, 0.45, 0.95);
const ASLEEP_INK: Color = Color::srgba(0.66, 0.66, 0.66, 0.85);
const DOWN_INK: Color = Color::srgba(0.95, 0.38, 0.32, 0.95);

/// The ink a member's state is drawn in.
pub fn ink(state: u8) -> Color {
    match state {
        TEAM_ASLEEP => ASLEEP_INK,
        TEAM_DEAD => DOWN_INK,
        _ => MATE_INK,
    }
}

/// How far over the feet a mark rides, metres: just over a head.
const MARK_LIFT_M: f32 = 2.1;

#[derive(Component)]
struct Panel;
/// One mark, for slot `i` of the team view.
#[derive(Component)]
struct Mark(usize);

/// What the panel shows now, so it is only rebuilt when that changes.
#[derive(Resource, Default)]
struct Shown {
    lines: Vec<(String, u8)>,
}

fn setup(mut commands: Commands) {
    // Left, a third of the way down: clear of the chat log at the bottom
    // and of the vitals.
    commands.spawn((
        super::WorldEntity,
        Panel,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(22.0),
            top: Val::Percent(32.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
    for i in 0..TEAM_MAX {
        commands.spawn((
            super::WorldEntity,
            Mark(i),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Text::new(""),
            ui::font_bold(13.0),
            TextColor(MATE_INK),
            ui::TEXT_SHADOW,
            Visibility::Hidden,
            Pickable::IGNORE,
        ));
    }
}

fn forget(mut shown: ResMut<Shown>) {
    shown.lines.clear();
}

/// The panel: the roster, and an invite waiting on you.
fn panel(
    mut commands: Commands,
    net: NonSend<Net>,
    mut shown: ResMut<Shown>,
    root: Query<Entity, With<Panel>>,
) {
    let core = &net.session.core;
    let team = core.team();
    let [x, _, z] = core.predict.render_position();
    let mut lines: Vec<(String, u8)> = crate::ui::team::rows(team, core.player_id, [x, z], |id| {
        crate::ui::names::of(core, id)
    })
    .into_iter()
    .map(|r| (r.text, r.state))
    .collect();
    if team.invite != 0 && team.count == 0 {
        let who = crate::ui::names::of(core, team.invite);
        lines.push((crate::ui::team::invited_line(&who), 0));
    }
    if lines == shown.lines {
        return;
    }
    shown.lines = lines.clone();
    let Ok(root) = root.single() else {
        return;
    };
    commands.entity(root).despawn_related::<Children>();
    commands.entity(root).with_children(|c| {
        for (text, state) in lines {
            c.spawn((
                Text::new(text),
                ui::font(14.0),
                TextColor(ink(state)),
                ui::TEXT_SHADOW,
                Pickable::IGNORE,
            ));
        }
    });
}

/// The marks: each teammate within reach gets their name over their head,
/// placed on the screen where the camera sees them, whatever stands between
/// (Rust's green dot). One behind the camera, or not in the snapshot, gets
/// none — the map has them.
fn marks(
    net: NonSend<Net>,
    cam: Query<(&Camera, &GlobalTransform), With<EyeCam>>,
    mut marks: Query<(&Mark, &mut Node, &mut Text, &mut TextColor, &mut Visibility)>,
) {
    let core = &net.session.core;
    let team = core.team();
    let [x, _, z] = core.predict.render_position();
    let cam = cam.single().ok();
    let at = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();
    for (mark, mut node, mut text, mut color, mut vis) in &mut marks {
        let place = team
            .members()
            .get(mark.0)
            .filter(|m| m.id != core.player_id)
            .and_then(|m| {
                let d = crate::ui::team::distance([x, z], m.x_m, m.z_m);
                if !crate::ui::team::marked(m.state, d) || !core.interp.sample(m.id, at, &mut rs) {
                    return None;
                }
                let (camera, xf) = cam?;
                let head = Vec3::new(rs.x, rs.y + MARK_LIFT_M, rs.z);
                let px = camera.world_to_viewport(xf, head).ok()?;
                let d = (rs.x - x).hypot(rs.z - z);
                Some((px, *m, d))
            });
        match place {
            Some((px, m, d)) => {
                let label =
                    crate::ui::team::mark_label(&crate::ui::names::of(core, m.id), m.state, d);
                if text.0 != label {
                    text.0 = label;
                }
                color.0 = ink(m.state);
                // Centred over the head: the label's width is not known
                // here, so it hangs from a point a little left of it.
                node.left = Val::Px(px.x - 24.0);
                node.top = Val::Px(px.y - 16.0);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<Shown>()
        // The nodes are `WorldEntity`, so they are built with the world and
        // `world_teardown` takes them.
        .add_systems(OnEnter(super::Screen::Loading), (forget, setup))
        .add_systems(
            Update,
            (panel, marks).run_if(in_state(super::Screen::InWorld)),
        );
}
