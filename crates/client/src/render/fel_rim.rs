//! A green rim while the Verdigris eats at you: the edge of the frame greens
//! as its meter climbs (`ClientCore::fel_pct`, `exposure::corrode`), and goes
//! as it falls. The hurt flash's gradient node (`hurt_flash.rs`), held rather
//! than flashed — it is a condition, not a blow — and kept under the HUD.

use bevy::prelude::*;

use super::WorldEntity;

/// The rim's alpha at a full meter. **(knob)**
const RIM_MAX: f32 = 0.22;
/// The meter, per cent, where the rim starts to show and where it is full.
const RIM_FROM_PCT: f32 = 25.0;
const RIM_FULL_PCT: f32 = 100.0;
/// Where the green starts, as a share of the far-corner radius.
const RIM_CLEAR_PCT: f32 = 60.0;

/// The rim's node.
#[derive(Component)]
pub struct FelRim;

fn rim(alpha: f32) -> Color {
    Color::srgba(0.12, 0.55, 0.08, alpha)
}

/// The rim's alpha at a meter of `pct` per cent.
pub fn rim_alpha(pct: u8) -> f32 {
    let t = ((pct as f32 - RIM_FROM_PCT) / (RIM_FULL_PCT - RIM_FROM_PCT)).clamp(0.0, 1.0);
    RIM_MAX * t * t * (3.0 - 2.0 * t)
}

/// Keep the rim on the meter, and the node only while it shows.
pub fn rim_drive(
    mut commands: Commands,
    net: NonSend<super::Net>,
    mut q: Query<(Entity, &mut BackgroundGradient), With<FelRim>>,
) {
    let alpha = rim_alpha(net.session.core.fel_pct);
    let live = q.single_mut().ok();
    match (alpha > 0.0, live) {
        (false, Some((e, _))) => commands.entity(e).despawn(),
        (false, None) => {}
        (true, Some((_, mut bg))) => {
            if let Some(Gradient::Radial(r)) = bg.0.first_mut() {
                if let Some(stop) = r.stops.last_mut() {
                    if stop.color != rim(alpha) {
                        stop.color = rim(alpha);
                    }
                }
            }
        }
        (true, None) => {
            commands.spawn((
                WorldEntity,
                FelRim,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundGradient(vec![Gradient::Radial(RadialGradient::new(
                    UiPosition::CENTER,
                    RadialGradientShape::FarthestCorner,
                    vec![
                        ColorStop::new(Color::NONE, Val::Percent(RIM_CLEAR_PCT)),
                        ColorStop::new(rim(alpha), Val::Percent(100.0)),
                    ],
                ))]),
                ZIndex(-1),
                Pickable::IGNORE,
            ));
        }
    }
}
