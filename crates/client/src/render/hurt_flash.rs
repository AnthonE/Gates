//! A red rim when you are hit: the edge of the frame reddens for a moment and
//! fades. Where the blow came from is the hurt arc's (`hud::hurt_arc`); this is
//! the "that hurt" half, and it stays at the rim and stays faint — Rust's 2022
//! combat pass replaced its full-screen blood with a small splat and a little
//! shake, and later made its hurt effects milder still.
//!
//! A gradient node like the wounded vignette (`wounded.rs`), spawned on the
//! first blow and despawned when it has faded; while it is up only the rim
//! stop's alpha moves, in place.

use bevy::prelude::*;

use super::feed::Feed;
use super::WorldEntity;

/// The rim's alpha for the lightest blow, and how much more per point of
/// damage, capped.
const FLASH_MIN: f32 = 0.16;
const FLASH_PER_HP: f32 = 0.008;
const FLASH_MAX: f32 = 0.42;
/// Alpha lost per second: the heaviest blow is gone in about half a second.
const FLASH_DECAY_PER_S: f32 = 0.85;
/// Where the red starts, as a share of the far-corner radius.
const FLASH_CLEAR_PCT: f32 = 52.0;

/// The rim's node.
#[derive(Component)]
pub struct HurtFlash;

fn rim(alpha: f32) -> Color {
    Color::srgba(0.55, 0.02, 0.02, alpha)
}

/// Raise the rim on a blow, fade it, and keep the node only while it shows.
pub fn flash(
    mut commands: Commands,
    feed: Res<Feed>,
    time: Res<Time>,
    mut level: Local<f32>,
    mut q: Query<(Entity, &mut BackgroundGradient), With<HurtFlash>>,
) {
    if feed.hurts > 0 {
        let want = (FLASH_MIN + feed.hurt_damage as f32 * FLASH_PER_HP).min(FLASH_MAX);
        *level = level.max(want);
    } else {
        *level = (*level - FLASH_DECAY_PER_S * time.delta_secs()).max(0.0);
    }
    let live = q.single_mut().ok();
    match (*level > 0.0, live) {
        (false, Some((e, _))) => commands.entity(e).despawn(),
        (false, None) => {}
        (true, Some((_, mut bg))) => {
            if let Some(Gradient::Radial(r)) = bg.0.first_mut() {
                if let Some(stop) = r.stops.last_mut() {
                    stop.color = rim(*level);
                }
            }
        }
        (true, None) => {
            commands.spawn((
                WorldEntity,
                HurtFlash,
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
                        ColorStop::new(Color::NONE, Val::Percent(FLASH_CLEAR_PCT)),
                        ColorStop::new(rim(*level), Val::Percent(100.0)),
                    ],
                ))]),
                // Under the HUD, over the world — the wounded vignette's place.
                ZIndex(-1),
                Pickable::IGNORE,
            ));
        }
    }
}
