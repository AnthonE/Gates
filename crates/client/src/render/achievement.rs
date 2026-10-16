//! The achievement popup: Steam's corner toast for an unlock (wire v118,
//! `SUB_ACHIEVEMENT`). The achievement's icon, ACHIEVEMENT UNLOCKED, its
//! name and line, and where the set stands; a finished set gets SET COMPLETE
//! in gold. It slides in at the bottom right, holds, and goes, one at a time,
//! with any that arrive together waiting their turn.
//!
//! HUD only: it reads what the server said was earned (`Feed`), decides
//! nothing, and is spawned per popup and despawned when it has faded.

use std::collections::VecDeque;

use bevy::prelude::*;

use super::feed::Feed;
use super::icons::Icons;
use super::ui::{font, font_bold};
use super::WorldEntity;

/// The popup's beats, seconds: in, held, out.
pub const IN_S: f32 = 0.35;
pub const HOLD_S: f32 = 4.5;
pub const OUT_S: f32 = 0.6;
/// How far it slides in from, px.
const SLIDE_PX: f32 = 36.0;
/// Popups waiting their turn; past this the oldest waiting is dropped.
const QUEUE_CAP: usize = 16;
const WIDTH_PX: f32 = 340.0;
const ICON_PX: f32 = 56.0;

const GILT: [f32; 3] = [0.98, 0.84, 0.52];
const BONE: [f32; 3] = [0.94, 0.92, 0.86];
const MUTED: [f32; 3] = [0.70, 0.68, 0.63];
const PLATE: [f32; 3] = [0.10, 0.095, 0.085];
const EDGE: [f32; 3] = [0.42, 0.36, 0.22];
/// The plate's opacity at full.
const PLATE_A: f32 = 0.92;

#[derive(Component)]
pub struct PopRoot;

/// A piece of the popup, the colour it fades from and its opacity at full:
/// its text, its fill or its picture.
#[derive(Component)]
pub struct PopInk([f32; 3], f32);

/// The plate's edge, which fades with it. Its own component, never a second
/// `PopInk` on the plate: two of one component in a bundle panics at spawn
/// (`CLAUDE.md` §traps), and nothing before the spawn says so.
#[derive(Component)]
pub struct PopEdge([f32; 3]);

#[derive(Default)]
pub struct PopState {
    queue: VecDeque<protocol::Achieved>,
    age: f32,
    showing: bool,
}

/// How visible the popup is `age` seconds in, and how far it has slid: both
/// 0 to 1.
pub fn beat(age: f32) -> (f32, f32) {
    let ease = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let slide = ease(age / IN_S);
    let alpha = if age < IN_S {
        slide
    } else if age < IN_S + HOLD_S {
        1.0
    } else {
        1.0 - ease((age - IN_S - HOLD_S) / OUT_S)
    };
    (alpha, slide)
}

/// The popup's three lines: the eyebrow, the name, the line under.
pub fn lines(a: &protocol::Achieved) -> (&'static str, String, String) {
    let name = String::from_utf8_lossy(a.name.as_bytes()).into_owned();
    if a.set {
        return (
            "SET COMPLETE",
            name,
            format!("All {} achievements", a.total),
        );
    }
    let desc = a
        .desc
        .map(|d| String::from_utf8_lossy(d.as_bytes()).into_owned())
        .unwrap_or_default();
    let under = if a.total > 0 {
        format!("{desc}  \u{b7}  {} / {}", a.done, a.total)
    } else {
        desc
    };
    ("ACHIEVEMENT UNLOCKED", name, under)
}

/// Take this frame's unlocks and run the popup. Unlocks that land while the
/// island is still loading wait for it: an unlock at the join (a pack that
/// already held 65 Junk) would otherwise pop up over the loading screen.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn popup(
    mut commands: Commands,
    feed: Res<Feed>,
    screen: Option<Res<State<super::Screen>>>,
    icons: Option<Res<Icons>>,
    time: Res<Time>,
    mut st: Local<PopState>,
    mut roots: Query<(Entity, &mut Node), With<PopRoot>>,
    mut inks: Query<(
        &PopInk,
        Option<&mut TextColor>,
        Option<&mut BackgroundColor>,
        Option<&mut ImageNode>,
        Option<(&PopEdge, &mut BorderColor)>,
    )>,
) {
    for a in feed.achievements() {
        if st.queue.len() == QUEUE_CAP {
            st.queue.pop_front();
        }
        st.queue.push_back(*a);
    }
    let loading = screen.is_some_and(|s| *s.get() == super::Screen::Loading);
    if !st.showing {
        if loading {
            return;
        }
        let Some(a) = st.queue.pop_front() else {
            return;
        };
        spawn_popup(&mut commands, &a, icons.as_deref());
        st.age = 0.0;
        st.showing = true;
        return;
    }
    st.age += time.delta_secs();
    if st.age >= IN_S + HOLD_S + OUT_S {
        for (e, _) in &roots {
            commands.entity(e).despawn();
        }
        st.showing = false;
        return;
    }
    let (alpha, slide) = beat(st.age);
    for (_, mut node) in &mut roots {
        node.right = Val::Px(24.0 - SLIDE_PX * (1.0 - slide));
    }
    for (ink, text, bg, image, edge) in &mut inks {
        let c = Color::srgba(ink.0[0], ink.0[1], ink.0[2], ink.1 * alpha);
        if let Some(mut t) = text {
            t.0 = c;
        }
        if let Some(mut b) = bg {
            b.0 = c;
        }
        if let Some(mut i) = image {
            i.color = c;
        }
        if let Some((edge, mut border)) = edge {
            *border = BorderColor::all(Color::srgba(edge.0[0], edge.0[1], edge.0[2], alpha));
        }
    }
}

fn spawn_popup(commands: &mut Commands, a: &protocol::Achieved, icons: Option<&Icons>) {
    let (eyebrow, name, under) = lines(a);
    let stem = a
        .icon
        .map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned())
        .unwrap_or_default();
    let icon = icons.and_then(|i| i.glyph(&stem));
    // A glyph is white and takes the gilt; an item is a picture, drawn as is.
    let icon_ink = if crate::ui::icons::is_glyph(&stem) {
        GILT
    } else {
        [1.0, 1.0, 1.0]
    };
    let eyebrow_ink = if a.set { GILT } else { MUTED };
    let clear = |ink: [f32; 3]| TextColor(Color::srgba(ink[0], ink[1], ink[2], 0.0));
    commands
        .spawn((
            WorldEntity,
            PopRoot,
            PopInk(PLATE, PLATE_A),
            PopEdge(EDGE),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(24.0),
                right: Val::Px(24.0 - SLIDE_PX),
                width: Val::Px(WIDTH_PX),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            Pickable::IGNORE,
        ))
        .with_children(|pop| {
            match icon {
                Some(h) => {
                    pop.spawn((
                        PopInk(icon_ink, 1.0),
                        ImageNode {
                            image: h,
                            color: Color::NONE,
                            ..default()
                        },
                        Node {
                            width: Val::Px(ICON_PX),
                            height: Val::Px(ICON_PX),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                }
                None => {
                    pop.spawn(Node {
                        width: Val::Px(ICON_PX),
                        height: Val::Px(ICON_PX),
                        flex_shrink: 0.0,
                        ..default()
                    });
                }
            }
            pop.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                flex_grow: 1.0,
                min_width: Val::Px(0.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn((
                    PopInk(eyebrow_ink, 1.0),
                    Text::new(eyebrow),
                    font_bold(11.0),
                    clear(eyebrow_ink),
                ));
                col.spawn((
                    PopInk(BONE, 1.0),
                    Text::new(name),
                    font_bold(17.0),
                    clear(BONE),
                ));
                if !under.is_empty() {
                    col.spawn((
                        PopInk(MUTED, 1.0),
                        Text::new(under),
                        font(13.0),
                        clear(MUTED),
                    ));
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> protocol::ChatText {
        protocol::ChatText::sanitize(s.as_bytes()).unwrap()
    }

    /// The popup spawns — a bundle with two of one component is a panic at
    /// spawn that the compiler and clippy both pass — and every piece of it
    /// fades.
    #[test]
    fn the_popup_spawns_whole() {
        use bevy::ecs::world::CommandQueue;
        for set in [false, true] {
            let mut world = World::new();
            let mut queue = CommandQueue::default();
            let a = protocol::Achieved {
                set,
                name: text("Collect Wood"),
                desc: Some(text("Collect 100 Wood")),
                icon: Some(text("wood")),
                done: 3,
                total: 40,
            };
            spawn_popup(&mut Commands::new(&mut queue, &world), &a, None);
            queue.apply(&mut world);
            let roots = world.query::<&PopRoot>().iter(&world).count();
            assert_eq!(roots, 1);
            assert_eq!(world.query::<&PopEdge>().iter(&world).count(), 1);
            let texts = world.query::<(&PopInk, &Text)>().iter(&world).count();
            assert_eq!(texts, 3, "eyebrow, name, line under");
        }
    }

    #[test]
    fn the_popup_slides_in_holds_and_goes() {
        assert_eq!(beat(0.0), (0.0, 0.0));
        assert_eq!(beat(IN_S + HOLD_S * 0.5), (1.0, 1.0));
        let (a, s) = beat(IN_S + HOLD_S + OUT_S);
        assert!(a.abs() < 1e-6 && s == 1.0);
        let mut last = 1.0;
        for k in 0..20 {
            let (a, _) = beat(IN_S + HOLD_S + OUT_S * k as f32 / 20.0);
            assert!(a <= last, "it never brightens on the way out");
            last = a;
        }
    }

    #[test]
    fn an_unlock_says_what_and_where_the_set_stands() {
        let a = protocol::Achieved {
            set: false,
            name: text("Collect Wood"),
            desc: Some(text("Collect 100 Wood")),
            icon: Some(text("wood")),
            done: 3,
            total: 40,
        };
        let (eyebrow, name, under) = lines(&a);
        assert_eq!(eyebrow, "ACHIEVEMENT UNLOCKED");
        assert_eq!(name, "Collect Wood");
        assert_eq!(under, "Collect 100 Wood  \u{b7}  3 / 40");
        let set = protocol::Achieved {
            set: true,
            desc: None,
            name: text("Survival"),
            done: 40,
            ..a
        };
        assert_eq!(
            lines(&set),
            (
                "SET COMPLETE",
                "Survival".to_string(),
                "All 40 achievements".to_string()
            )
        );
    }
}
