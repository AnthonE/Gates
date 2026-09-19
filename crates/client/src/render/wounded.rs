//! The crawl's screen (wounded v0; `reference/WOUNDED.md` §4, §9.5).
//!
//! What a downed player sees, in three parts, and each has one owner:
//!
//! - **the camera drops** — `input::place_eye` eases `Eye::down` toward one
//!   and lowers the eye from `EYE_HEIGHT` to [`CRAWL_EYE_M`]; `rig::follow_eye`
//!   rolls the view by [`WOUND_ROLL_RAD`] times the same fraction, so the
//!   horizon tilts as the body goes down and rights itself as it gets up;
//! - **the edges darken** — a radial vignette over the whole frame,
//!   transparent at the centre and near-black at the rim, spawned when the
//!   body falls and despawned when it stands or dies. A UI node and not a
//!   post-process, because this client has none (`RENDER.md`), and a
//!   gradient node costs nothing per frame once it is up;
//! - **the two numbers** — the seconds to the roll and the odds of getting
//!   up, the line the reference added in 2023, counted down here from
//!   `EventMsg::Wounded`'s tick count at the tick rate the client already
//!   runs. The odds are as the sim read them at the fall; the sim re-reads
//!   the meters at the roll, so the number can drift a point over the minute
//!   and `world.rs`'s `EV_WOUNDED` says so.
//!
//! **What this file does not do is decide anything.** Whether a body is
//! down is `ClientCore::wounded`, set by the wire; whether it may swing is
//! the sim's (`live_slot_of`) and the predictor's (`Predictor::crawl`).
//! Bevy draws, it does not decide (`CLAUDE.md`).
//!
//! ⚠ **The look is unsourced.** No page reached on 2026-09-13 describes the
//! reference's wounded camera or colour (`WOUNDED.md` §4 separates what a
//! source says from what a player remembers). The three knobs below are a
//! first cut to be judged by booting the game, which is the visual gate.

use bevy::prelude::*;
use sim_core::limits::TICK_HZ;

use super::feed::Feed;
use super::{Net, WorldEntity};

/// Eye height above the feet while down, metres — a body on its side has
/// its eyes about a forearm off the ground. Against `EYE_HEIGHT = 1.6`.
/// Proposed default, `DECISIONS.md` §open ("wounded v0").
pub const CRAWL_EYE_M: f32 = 0.45;
/// The view's roll while down, radians (~12.6°): enough that the horizon
/// is visibly wrong, not so much that the door you are crawling to is hard
/// to keep in frame. Same row.
pub const WOUND_ROLL_RAD: f32 = 0.22;
/// Seconds for the camera to fall and to rise — a one-pole ease, so the
/// last centimetre takes as long as it wants and the first half-metre is
/// fast, which is what a fall looks like. Same row.
pub const WOUND_DROP_S: f32 = 0.45;
/// The vignette's darkness at the rim. Same row.
pub const WOUND_RIM_ALPHA: f32 = 0.86;
/// Where the vignette starts biting, as a share of the far-corner radius.
/// Same row.
pub const WOUND_CLEAR_PCT: f32 = 30.0;

/// Everything the crawl's screen owns. `WorldEntity` too, so a teardown
/// takes it with the world.
#[derive(Component)]
pub struct WoundedRoot;

/// The readout line under the vignette.
#[derive(Component)]
pub struct WoundedLine;

/// The client's own countdown and the odds it was told.
#[derive(Resource, Default)]
pub struct Crawl {
    /// Seconds left to the roll, counted down from the fall.
    pub secs_left: f32,
    /// The odds as `EventMsg::Wounded` stated them, per mille.
    pub chance_pm: u16,
    /// The whole second last drawn, so the line is re-set once a second
    /// and not once a frame (a `format!` per frame is an allocation per
    /// frame, and the client is held to the sim thread's discipline).
    shown: u32,
}

/// One system: spawn the screen when the body goes down, count while it is
/// down, despawn when it is not. Runs after `feed::drain` (it reads the
/// frame's `Feed::wounded`) under `world_running`, so a fall that lands
/// while the Esc menu is up still starts the clock.
pub fn overlay(
    mut commands: Commands,
    net: NonSend<Net>,
    feed: Res<Feed>,
    time: Res<Time>,
    mut crawl: ResMut<Crawl>,
    roots: Query<Entity, With<WoundedRoot>>,
    mut lines: Query<&mut Text, With<WoundedLine>>,
) {
    if let Some((ticks, chance_pm)) = feed.wounded {
        crawl.secs_left = ticks as f32 / TICK_HZ as f32;
        crawl.chance_pm = chance_pm;
        crawl.shown = u32::MAX;
    }
    let down = net.session.core.wounded;
    let root = roots.iter().next();
    match (down, root) {
        (false, Some(e)) => {
            commands.entity(e).despawn();
        }
        (false, None) => {}
        (true, None) => spawn(&mut commands, &crawl),
        (true, Some(_)) => {
            let (_, target, ticks) = net.session.core.assist;
            let helping = target == net.session.core.player_id
                && ticks > 0
                && ticks < sim_core::assist::ASSIST_TICKS;
            if !helping {
                crawl.secs_left = (crawl.secs_left - time.delta_secs()).max(0.0);
            }
            let whole = crawl.secs_left.ceil() as u32;
            if whole != crawl.shown {
                crawl.shown = whole;
                let want = crate::ui::wounded::readout(crawl.secs_left, crawl.chance_pm);
                for mut t in &mut lines {
                    t.0 = want.clone();
                }
            }
        }
    }
}

fn spawn(commands: &mut Commands, crawl: &Crawl) {
    let rim = Color::srgba(0.05, 0.015, 0.012, WOUND_RIM_ALPHA);
    commands
        .spawn((
            WorldEntity,
            WoundedRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexEnd,
                padding: UiRect::bottom(Val::Px(96.0)),
                ..default()
            },
            BackgroundGradient(vec![Gradient::Radial(RadialGradient::new(
                UiPosition::CENTER,
                RadialGradientShape::FarthestCorner,
                vec![
                    ColorStop::new(Color::NONE, Val::Percent(WOUND_CLEAR_PCT)),
                    ColorStop::new(rim, Val::Percent(100.0)),
                ],
            ))]),
            // Under the HUD's own overlays (they carry no `ZIndex` and so
            // sit at zero, spawned earlier); over the world, which is not
            // UI at all.
            ZIndex(-1),
            Pickable::IGNORE,
        ))
        .with_children(|root| {
            root.spawn((
                WoundedLine,
                Text::new(crate::ui::wounded::readout(
                    crawl.secs_left,
                    crawl.chance_pm,
                )),
                super::ui::font_bold(26.0),
                TextColor(Color::srgb(0.93, 0.86, 0.80)),
                super::ui::TEXT_SHADOW,
                Pickable::IGNORE,
            ));
        });
}
