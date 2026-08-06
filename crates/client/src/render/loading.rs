//! The loading screen: the world is built here, and this is what the player
//! looks at while it happens.
//!
//! **It is a state, not a splash, and the progress is real.** Nothing here
//! counts frames or interpolates a bar toward 100% — the three ring builders
//! each report how many of their cells are resident (`Ring`, `PropRing`,
//! `ClutterRing`), the bar is their mean, and the screen ends the instant all
//! three say full. That is the same observable-state settle the capture
//! harness uses and for the same reason: a bar driven by a clock is a bar that
//! lies on a slow box, and this repo's rule is that a gate — or a player —
//! waits on state, never on milliseconds.
//!
//! **Where it sits in the flow.** `Connecting` ends the moment the welcome
//! lands, which is when the seed exists and therefore when `WorldId` can be
//! built; but a welcome is not a world. Everything between the welcome and a
//! frame worth showing — a 5x5 ring of ground chunks, the same ring of
//! scatter, a ring of clutter tiles and the far mesh, one build of each per
//! frame by budget — used to happen with the
//! player already nominally in the world, staring at an empty near ring
//! filling in around them. This is that interval, named.
//!
//! **It covers two waits, not one, and the first one used to be invisible.**
//! The welcome names the island; the first snapshot names where on it the
//! player stands. Between those the client knows a seed and nothing else,
//! and the rings — which read `Eye::pos` — used to spend that interval
//! building the world **origin**, because an unplaced predictor reports
//! `Body::default()` and that is a real location rather than a sentinel.
//! Every connect threw those chunks away; a first snapshot slower than the
//! whole ring build would have filled the bar there.
//! `crate::ui::load::Progress::placed` is the gate for it,
//! `render::world_placed` is where the streamers stand down, and this screen
//! is what the player looks at meanwhile.
//!
//! **The world renders behind it, deliberately.** The overlay is opaque and
//! the rig is up, so the 3D pass runs for every loading frame with nothing
//! visible to the player — which is exactly when a pipeline should be paying
//! its first-use specialization cost. `CLAUDE.md`'s prewarm trap says a native
//! pipeline compile is a bigger stall than the WebGL link that cost the
//! browser client 700 ms worst-frames; a loading screen that hides a live
//! renderer is the cheapest prewarm there is.

use bevy::prelude::*;

use super::clutter::{ClutterRing, RING_TILES};
use super::menu::{Connecting, Menu, Screen};
use super::props::PropRing;
use super::terrain_mesh::{Ring, RING_CHUNKS};
use super::{ui, Eye, WorldId};
use crate::ui::load::Progress;

/// Everything this screen owns, so leaving despawns exactly it.
#[derive(Component)]
pub struct LoadingRoot;

/// The filled part of the bar. Its width is the only thing that animates.
#[derive(Component)]
pub struct Bar;

/// The three counters, redrawn in place.
#[derive(Component)]
pub struct Counts;

/// Read this frame's [`Progress`] off the rings.
///
/// The arithmetic on it is [`crate::ui::load`]'s — pure, and gated in the
/// code tier. This is the half that has to touch the render module, and it
/// is the whole of that half: four counters and the shipped totals.
///
/// `placed` is passed in rather than read from `Eye` here so the function
/// stays a plain projection of the rings, and so the caller cannot
/// accidentally answer the two questions from two different frames.
pub fn read(placed: bool, ring: &Ring, props: &PropRing, clutter: &ClutterRing) -> Progress {
    Progress {
        placed,
        ground: (ring.len(), RING_CHUNKS),
        scatter: (props.len(), RING_CHUNKS),
        clutter: (clutter.len(), RING_TILES),
        far: ring.far_ready(),
    }
}

/// Bar geometry, pixels. Not a knob in the registry sense — it is the width
/// of a rectangle, chosen to match the shard rows on the screen before it.
const BAR_W: f32 = 560.0;
const BAR_H: f32 = 10.0;

pub fn setup(mut commands: Commands, connecting: NonSend<Connecting>, world: Option<Res<WorldId>>) {
    // No camera is spawned here: `rig::setup` runs on entering this same
    // state and its `Camera3d` is the one camera the UI draws against. A
    // second (2D) camera would fight it for the frame and Bevy would warn.
    let where_to = if connecting.addr.is_empty() {
        "the world".to_string()
    } else {
        connecting.addr.clone()
    };
    let seed = match world {
        Some(w) => format!("seed {}", w.seed),
        // The menu path always has one by now; a `--server` launch inserts it
        // before the window. Saying so beats printing "seed 0".
        None => "seed pending".to_string(),
    };

    commands.spawn((
        LoadingRoot,
        ui::screen(ui::BG),
        children![
            ui::title("GATES"),
            ui::label(format!("entering {where_to}    {seed}"), 15.0, ui::DIM),
            // The track, and the fill inside it.
            (
                Node {
                    width: Val::Px(BAR_W),
                    height: Val::Px(BAR_H),
                    margin: UiRect::vertical(Val::Px(14.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(ui::ROW_IDLE),
                BorderColor::all(ui::RULE),
                children![(
                    Bar,
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(ui::ACCENT),
                )],
            ),
            (Counts, ui::label(Progress::waiting().line(), 13.0, ui::DIM)),
            ui::label(
                "the island is a pure function of the seed - nothing is downloading, \
                 this is the client building it. it waits for the server to say \
                 WHERE first",
                12.0,
                ui::FAINT,
            ),
            ui::label("Esc goes back to the server list", 12.0, ui::FAINT),
        ],
    ));
}

/// Drive the bar, and end the screen when the world is actually there.
#[allow(clippy::too_many_arguments)]
pub fn update(
    eye: Res<Eye>,
    ring: Res<Ring>,
    props: Res<PropRing>,
    clutter: Res<ClutterRing>,
    mut bar: Query<&mut Node, With<Bar>>,
    mut counts: Query<&mut Text, With<Counts>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    connecting: NonSend<Connecting>,
    mut menu: ResMut<Menu>,
    mut next: ResMut<NextState<Screen>>,
) {
    let p = read(eye.placed, &ring, &props, &clutter);

    if let Ok(mut node) = bar.single_mut() {
        node.width = Val::Percent(p.fraction() * 100.0);
    }
    if let Ok(mut text) = counts.single_mut() {
        let line = p.line();
        if text.0 != line {
            text.0 = line;
        }
    }

    // The way out. A world that never finishes building is a bug rather than
    // a wait, but the player still has to be able to leave one — the same
    // rule the connect screen obeys.
    if keyboard.just_pressed(KeyCode::Escape) {
        menu.status = format!("{}: left while the world was loading", connecting.addr);
        next.set(Screen::Menu);
        return;
    }

    if p.done() {
        next.set(Screen::InWorld);
    }
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<LoadingRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture built from the SHIPPED ring sizes rather than from numbers
    /// typed here — a test carrying its own 25 would keep passing after
    /// `NEAR_RADIUS` changed, asserting about a bar nobody draws.
    ///
    /// **Placed**, because these three are about the ring arithmetic and the
    /// placement gate is `tests/ui.rs` §E's, where it runs in the code tier.
    /// A fixture that left it false would make every assertion below pass on
    /// the gate rather than on the thing it names.
    fn p(g: usize, s: usize, c: usize, far: bool) -> Progress {
        Progress {
            placed: true,
            ground: (g, RING_CHUNKS),
            scatter: (s, RING_CHUNKS),
            clutter: (c, RING_TILES),
            far,
        }
    }
    fn full() -> Progress {
        p(RING_CHUNKS, RING_CHUNKS, RING_TILES, true)
    }

    #[test]
    fn the_bar_is_the_mean_of_three_rings() {
        // One ring at full cannot carry the bar past a third, which is the
        // whole reason the mean is taken rather than the max.
        assert!((p(RING_CHUNKS, 0, 0, true).fraction() - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(p(0, 0, 0, false).fraction(), 0.0);
        assert_eq!(full().fraction(), 1.0);
    }

    #[test]
    fn a_full_bar_is_not_a_built_world() {
        // Every ring resident but the far mesh still missing: the bar reads
        // 100% and the screen must NOT end, because the far mesh is what the
        // player sees past 160 m. `Ring::is_full` has always carried this;
        // the bar would hide it.
        let all_but_far = p(RING_CHUNKS, RING_CHUNKS, RING_TILES, false);
        assert_eq!(all_but_far.fraction(), 1.0);
        assert!(!all_but_far.done());
        assert!(full().done());
    }

    #[test]
    fn a_ring_that_over_fills_cannot_push_the_bar_past_full() {
        assert_eq!(p(999, 999, 999, true).fraction(), 1.0);
    }

    /// The seam between this module and the pure one: `read` is a projection
    /// and must not have an opinion. It is the only place the placement
    /// answer can be dropped on the floor, and dropping it would restore
    /// exactly the bug the gate exists for — silently, with every assertion
    /// above still green.
    #[test]
    fn read_carries_the_placement_answer_through() {
        let (r, pr, c) = (Ring::default(), PropRing::default(), ClutterRing::default());
        assert!(!read(false, &r, &pr, &c).placed);
        assert!(read(true, &r, &pr, &c).placed);
        assert_eq!(read(false, &r, &pr, &c).ground.1, RING_CHUNKS);
    }
}
