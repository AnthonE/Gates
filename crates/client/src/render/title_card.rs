//! THE GATE's title card: the place's name, large, on the way in — the
//! location banner an open world gives its landmarks, here for the one piece
//! of safe ground on the island. It rises over the middle of the screen as
//! you cross into the zone (or wake there), holds, and goes; leaving says so
//! too, in a warning's colour. The toast and the chime (`hud::gate_watch`)
//! still carry the rules for whoever has the card turned off by a plate run.
//!
//! HUD only: it reads where the core says you are, decides nothing, and is
//! spawned for each crossing and despawned when it has faded.

use bevy::prelude::*;

use super::ui::{font, font_bold};
use super::{Net, WorldEntity};

/// The card's beats, seconds: in, held, out.
pub const FADE_IN_S: f32 = 0.8;
pub const HOLD_S: f32 = 2.6;
pub const FADE_OUT_S: f32 = 1.3;
/// The gilt rules either side of the diamond, px at full width.
const RULE_PX: f32 = 220.0;
/// How far the card rises as it comes in, px.
const RISE_PX: f32 = 14.0;

const GILT: [f32; 3] = [0.98, 0.84, 0.52];
const LAPIS: [f32; 3] = [0.62, 0.8, 1.0];
const BONE: [f32; 3] = [0.94, 0.92, 0.86];
const WARN: [f32; 3] = [1.0, 0.52, 0.42];

#[derive(Component)]
pub struct CardRoot;

/// A piece of the card and the colour it fades from.
#[derive(Component)]
pub struct CardInk([f32; 3]);

/// One of the two rules, which grow out from the diamond.
#[derive(Component)]
pub struct CardRule;

/// What the card says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crossing {
    In,
    InHostile,
    Out,
}

/// The card's lines for a crossing: the eyebrow, the title, the line under.
pub fn lines(c: Crossing) -> (&'static str, &'static str, &'static str) {
    match c {
        Crossing::In => (
            "SAFE ZONE",
            "T H E   G A T E",
            "NO WEAPONS  \u{b7}  NO KILLING  \u{b7}  NO LOOTING  \u{b7}  NO SLEEPING",
        ),
        Crossing::InHostile => (
            "SAFE ZONE",
            "T H E   G A T E",
            "YOU ARE HOSTILE  \u{b7}  THE SENTRIES WILL FIRE",
        ),
        Crossing::Out => ("LEAVING", "T H E   G A T E", "YOU ARE NO LONGER PROTECTED"),
    }
}

/// How visible the card is `age` seconds in, and how far its rules have
/// grown: both 0 to 1.
pub fn beat(age: f32) -> (f32, f32) {
    let ease = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let grow = ease(age / FADE_IN_S);
    let alpha = if age < FADE_IN_S {
        grow
    } else if age < FADE_IN_S + HOLD_S {
        1.0
    } else {
        1.0 - ease((age - FADE_IN_S - HOLD_S) / FADE_OUT_S)
    };
    (alpha, grow)
}

#[derive(Default)]
pub struct CardState {
    was_in: Option<bool>,
    age: f32,
    showing: bool,
}

/// Every piece of the card that fades.
pub type Inks<'w, 's> = Query<
    'w,
    's,
    (
        &'static CardInk,
        Option<&'static mut TextColor>,
        Option<&'static mut BackgroundColor>,
    ),
>;

/// Fade every piece to `alpha`: a line's text, or a rule's or the diamond's
/// fill. Never both — a UI text carries a background too (`Node` requires
/// one), and inking it drew the line as a solid bar.
pub fn paint(alpha: f32, inks: &mut Inks) {
    for (ink, text, bg) in inks {
        let c = Color::srgba(ink.0[0], ink.0[1], ink.0[2], alpha);
        if let Some(mut t) = text {
            t.0 = c;
        } else if let Some(mut b) = bg {
            b.0 = c;
        }
    }
}

/// Watch for a crossing and run the card.
#[allow(clippy::type_complexity)]
pub fn card(
    mut commands: Commands,
    net: NonSend<Net>,
    time: Res<Time>,
    mut st: Local<CardState>,
    mut roots: Query<(Entity, &mut Node), (With<CardRoot>, Without<CardRule>)>,
    mut inks: Inks,
    mut rules: Query<&mut Node, (With<CardRule>, Without<CardRoot>)>,
) {
    let core = &net.session.core;
    if core.dead {
        st.was_in = None;
        return;
    }
    let [x, _, z] = core.eye_position();
    let in_town = sim_core::town::safe(&core.haven().town, x, z);
    let crossing = match st.was_in {
        // Waking inside counts as arriving.
        None | Some(false) if in_town => Some(if core.hostile_left() > 0 {
            Crossing::InHostile
        } else {
            Crossing::In
        }),
        Some(true) if !in_town => Some(Crossing::Out),
        _ => None,
    };
    st.was_in = Some(in_town);
    if let Some(c) = crossing {
        for (e, _) in &roots {
            commands.entity(e).despawn();
        }
        spawn_card(&mut commands, c);
        st.age = 0.0;
        st.showing = true;
        return;
    }
    if !st.showing {
        return;
    }
    st.age += time.delta_secs();
    let (alpha, grow) = beat(st.age);
    if st.age >= FADE_IN_S + HOLD_S + FADE_OUT_S {
        for (e, _) in &roots {
            commands.entity(e).despawn();
        }
        st.showing = false;
        return;
    }
    for (_, mut node) in &mut roots {
        node.margin.top = Val::Px(RISE_PX * (1.0 - grow));
    }
    paint(alpha, &mut inks);
    for mut node in &mut rules {
        node.width = Val::Px(RULE_PX * grow);
    }
}

fn spawn_card(commands: &mut Commands, c: Crossing) {
    let (eyebrow, title, under) = lines(c);
    let out = c == Crossing::Out;
    let (title_px, title_ink) = if out { (40.0, BONE) } else { (64.0, GILT) };
    let under_ink = if c == Crossing::In { BONE } else { WARN };
    let clear = |ink: [f32; 3]| TextColor(Color::srgba(ink[0], ink[1], ink[2], 0.0));
    let shadow = TextShadow {
        offset: Vec2::splat(2.0),
        color: Color::srgba(0.0, 0.0, 0.0, 0.75),
    };
    commands
        .spawn((
            WorldEntity,
            CardRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(17.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                margin: UiRect::top(Val::Px(RISE_PX)),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|card| {
            card.spawn((
                CardInk(LAPIS),
                Text::new(eyebrow),
                font_bold(15.0),
                clear(LAPIS),
                shadow,
            ));
            card.spawn((
                CardInk(title_ink),
                Text::new(title),
                font_bold(title_px),
                clear(title_ink),
                TextShadow {
                    offset: Vec2::splat(3.0),
                    color: Color::srgba(0.0, 0.0, 0.0, 0.7),
                },
            ));
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|row| {
                let rule = |row: &mut ChildSpawnerCommands| {
                    row.spawn((
                        CardRule,
                        CardInk(GILT),
                        Node {
                            width: Val::Px(0.0),
                            height: Val::Px(2.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ));
                };
                rule(row);
                row.spawn((
                    CardInk(LAPIS),
                    Node {
                        width: Val::Px(7.0),
                        height: Val::Px(7.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ));
                rule(row);
            });
            card.spawn((
                CardInk(under_ink),
                Text::new(under),
                font(17.0),
                clear(under_ink),
                shadow,
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At full, the lines are ink and the fill behind them stays clear; the
    /// rules and the diamond are the fills that draw.
    #[test]
    fn a_fade_paints_the_lines_and_never_the_fill_behind_them() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::ecs::world::CommandQueue;
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        spawn_card(&mut Commands::new(&mut queue, &world), Crossing::In);
        queue.apply(&mut world);
        world
            .run_system_once(|mut q: Inks| paint(1.0, &mut q))
            .expect("the fade runs");
        let mut lines = 0;
        for (t, bg) in world
            .query::<(&TextColor, Option<&BackgroundColor>)>()
            .iter(&world)
        {
            lines += 1;
            assert_eq!(t.0.alpha(), 1.0, "the line is drawn");
            assert!(
                bg.is_none_or(|b| b.0.alpha() == 0.0),
                "and nothing behind it"
            );
        }
        assert_eq!(lines, 3, "eyebrow, title, line under");
        let fills = world
            .query_filtered::<&BackgroundColor, (With<CardInk>, Without<TextColor>)>()
            .iter(&world)
            .filter(|b| b.0.alpha() == 1.0)
            .count();
        assert_eq!(fills, 3, "two rules and the diamond");
    }

    #[test]
    fn the_card_comes_in_holds_and_goes() {
        assert_eq!(beat(0.0), (0.0, 0.0));
        let (a, g) = beat(FADE_IN_S + HOLD_S * 0.5);
        assert_eq!((a, g), (1.0, 1.0));
        let (a, g) = beat(FADE_IN_S + HOLD_S + FADE_OUT_S);
        assert!(a.abs() < 1e-6 && g == 1.0);
        // It never brightens on the way out.
        let mut last = 1.0;
        for k in 0..20 {
            let (a, _) = beat(FADE_IN_S + HOLD_S + FADE_OUT_S * k as f32 / 20.0);
            assert!(a <= last);
            last = a;
        }
    }
}
