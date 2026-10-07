//! The arc's banner (`ARC.md` F4): top centre, under the compass — the act,
//! the work that matters most right now and its bar. It is how a player
//! learns, from the first minute, that this island is broken and that the
//! whole server is switching it back on.
//!
//! Spawned once with the HUD and rewritten in place when its words change;
//! the words are `crate::ui::arc::banner`'s, so `tests/ui.rs` holds them.

use bevy::prelude::*;

use super::feed::Feed;
use super::panels::kit::BAR_FILL;
use super::Net;

/// The banner's root, its two lines, and its bar's fill.
#[derive(Component)]
pub struct ArcBanner;
#[derive(Component)]
pub struct ArcTitle;
#[derive(Component)]
pub struct ArcLine;
#[derive(Component)]
pub struct ArcTrough;
#[derive(Component)]
pub struct ArcFill;

const BAR_W: f32 = 220.0;

pub fn setup(mut commands: Commands) {
    commands
        .spawn((
            super::WorldEntity,
            ArcBanner,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(36.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                display: Display::None,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|b| {
            b.spawn((
                ArcTitle,
                Text::new(""),
                super::ui::font_bold(12.0),
                TextColor(Color::srgba(0.84, 0.62, 0.27, 0.9)),
                super::ui::TEXT_SHADOW,
                Pickable::IGNORE,
            ));
            b.spawn((
                ArcLine,
                Text::new(""),
                super::ui::font_bold(13.0),
                TextColor(Color::srgba(0.90, 0.88, 0.82, 0.85)),
                super::ui::TEXT_SHADOW,
                Pickable::IGNORE,
            ));
            b.spawn((
                ArcTrough,
                Node {
                    width: Val::Px(BAR_W),
                    height: Val::Px(4.0),
                    ..default()
                },
                BackgroundColor(super::panels::VITAL_TROUGH),
                Pickable::IGNORE,
            ))
            .with_children(|t| {
                t.spawn((
                    ArcFill,
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(BAR_FILL),
                    Pickable::IGNORE,
                ));
            });
        });
}

/// Rewrite the banner when its words or its bar move.
#[allow(clippy::type_complexity)]
pub fn update(
    net: NonSend<Net>,
    feed: Res<Feed>,
    mut root: Query<&mut Node, (With<ArcBanner>, Without<ArcFill>, Without<ArcTrough>)>,
    mut title: Query<&mut Text, (With<ArcTitle>, Without<ArcLine>)>,
    mut line: Query<&mut Text, (With<ArcLine>, Without<ArcTitle>)>,
    mut trough: Query<&mut Node, (With<ArcTrough>, Without<ArcBanner>, Without<ArcFill>)>,
    mut fill: Query<&mut Node, (With<ArcFill>, Without<ArcBanner>, Without<ArcTrough>)>,
) {
    let banner = crate::ui::arc::banner(&net.session.core.arc, feed.server_tick_est);
    let Ok(mut node) = root.single_mut() else {
        return;
    };
    let Some(b) = banner else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    if let Ok(mut t) = title.single_mut() {
        if t.0 != b.title {
            t.0 = b.title.clone();
        }
    }
    if let Ok(mut t) = line.single_mut() {
        if t.0 != b.line {
            t.0 = b.line.clone();
        }
    }
    if let Ok(mut t) = trough.single_mut() {
        let want = if b.pm.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if t.display != want {
            t.display = want;
        }
    }
    if let (Ok(mut f), Some(pm)) = (fill.single_mut(), b.pm) {
        let want = Val::Percent(pm.min(1000) as f32 / 10.0);
        if f.width != want {
            f.width = want;
        }
    }
}
