//! The death screen — `Screen::Dead`, laid out like Rust's.
//!
//! **Dying used to end the session**: `ClientCore::dead` was set and read by
//! nothing, and the only way back was to restart the process. This screen is
//! the exit.
//!
//! A `Screen` rather than a HUD overlay: `input::gather` runs in `InWorld`
//! only, so a corpse stops walking and swinging for free (the sim refuses all
//! of it anyway), while `world_running` still holds and the session keeps
//! pumping behind it.
//!
//! **The layout is the reference's** (operator, 2026-10-15, with Rust's own
//! frame pasted): a strip across the top — YOU DIED, then ALIVE FOR, KILLED
//! BY, WITH A, AT A DISTANCE OF and IN, and REPORT CHEAT on the right — the
//! island filling the middle with a skull where you fell and your bags on it,
//! and a bar along the bottom with one button per bag, THE GATE once reached,
//! and RANDOM RESPAWN on the right. A bag that is still cooling shows its
//! timer and does not take a click. Every word is [`crate::ui::death`]'s;
//! this file lays them out and wires the clicks.
//!
//! Opaque, and over the HUD ([`DEATH_Z`]): the HUD is still drawn under this
//! screen and is several roots rather than one, so covering it is the one
//! way to keep the hotbar from printing through the bottom bar.

use bevy::image::Image;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::ui::death::{self as model, Death, Fact, FactKind, Meters, Ready, Tone, Wake};
use crate::ui::map::{self, GRID_COLS};

use super::hud::Toast;
use super::map::{
    BADGE_HOLLOW, BADGE_SOLID, GRID_INK, GRID_LABEL_PX, MAP_MARK_PX, MARK_ICON_FRAC, SITE_LABEL_PX,
    SITE_LABEL_W,
};
use super::screen::Screen;
use super::{ui, Net, WorldId};

/// Over the HUD. The report composer's line draws over this in turn
/// (`render::report`), so REPORT CHEAT can be typed into from here.
pub const DEATH_Z: i32 = 50;

/// The strip across the top and the bar along the bottom, UI px.
const TOP_H: f32 = 64.0;
const BOTTOM_H: f32 = 80.0;
/// The YOU DIED block, top left.
const DIED_W: f32 = 172.0;
/// A fact's chip, and the picture in one.
const CHIP_H: f32 = 26.0;
const CHIP_PIC: f32 = 20.0;
/// A wake button on the bottom bar, and RANDOM RESPAWN's.
const BUTTON_W: f32 = 178.0;
const BUTTON_H: f32 = 56.0;
const RANDOM_W: f32 = 214.0;
/// The skull where you fell, and a bag of yours, on the map.
const PIN_PX: f32 = 28.0;
/// How far above the spot the skull's disc stands.
const PIN_STEM: f32 = 9.0;
const BAG_MARK_PX: f32 = MAP_MARK_PX + 4.0;

/// The ground under everything, a shade under the bars.
const SHEET: Color = Color::srgb(0.075, 0.072, 0.066);
const BAR: Color = Color::srgb(0.118, 0.114, 0.104);
/// YOU DIED's block: the reference's one red slab.
const DIED: Color = Color::srgb(0.53, 0.13, 0.10);
/// The chips: green for the life, red for what ended it, grey for the rest.
const LIFE: Color = Color::srgb(0.34, 0.47, 0.17);
const FOE: Color = Color::srgb(0.62, 0.16, 0.12);
const PLAIN: Color = Color::srgb(0.25, 0.245, 0.23);
/// Each bag's own colour, cycling as the reference's do — orange, green,
/// blue — on its button and on its ring on the map, so the button and the
/// bag it wakes you on are matched by eye rather than by reading squares.
const BAG_INKS: [Color; 3] = [
    Color::srgb(0.78, 0.34, 0.12),
    Color::srgb(0.30, 0.43, 0.19),
    Color::srgb(0.20, 0.35, 0.52),
];
/// The same three, bright: the bag on the map and the ring on its button.
const BAG_MAP_INKS: [Color; 3] = [
    Color::srgb(1.0, 0.56, 0.26),
    Color::srgb(0.58, 0.86, 0.36),
    Color::srgb(0.47, 0.72, 1.0),
];
/// THE GATE's button: its map gilt, darkened to sit under white type.
const GATE_PANEL: Color = Color::srgb(0.52, 0.40, 0.14);
/// A button that will not take a click yet: grey, whatever its colour.
const COLD: Color = Color::srgb(0.13, 0.127, 0.118);
const COLD_TEXT: Color = Color::srgba(0.62, 0.60, 0.56, 0.6);
/// RANDOM RESPAWN, the reference's green.
const GO: Color = Color::srgb(0.33, 0.47, 0.16);
const GO_HOT: Color = Color::srgb(0.41, 0.57, 0.20);
/// The skull's pin.
const SKULL: Color = Color::srgb(0.86, 0.17, 0.13);
/// THE GATE's gilt, `MarkKind::Town`'s.
const GATE_INK: Color = Color::srgb(236.0 / 255.0, 190.0 / 255.0, 96.0 / 255.0);

/// The digits a button answers to, in its place on the bar: `1`…`9`, `0`.
const DIGITS: [KeyCode; 10] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
];

/// Everything this screen owns.
#[derive(Component)]
pub struct DeathRoot;

/// A button on the bottom bar. A component wrapper around [`Wake`], which
/// lives in `ui::death` with the order that decides which buttons exist.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WakeButton(pub Wake);

/// A bag of yours on the map: a second way to press its button.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WakeMark(pub Wake);

/// The ring round a bag on the map, lit while its button is under the
/// pointer — which of three bags in one square is the one you are about to
/// press.
#[derive(Component)]
pub struct MarkRing(pub Wake);

/// A button's picture, or the padlock that stands in for it while the
/// button will not take a click.
#[derive(Component)]
pub struct WakeIcon {
    wake: Wake,
    lock: bool,
}

/// The small line under a button's name: its grid square, or its timer.
#[derive(Component)]
pub struct WakeLine {
    wake: Wake,
    square: String,
    /// What the line says now, so it is rewritten once a second rather
    /// than every frame.
    shown: Option<Ready>,
}

/// The middle of the screen, sea-coloured, and the island in it.
#[derive(Component)]
pub struct MapArea;
#[derive(Component)]
pub struct IslandFrame;

/// The killer's picture, hidden until it lands (`faces::Pics`).
#[derive(Component)]
pub struct KillerFace;

/// REPORT CHEAT, top right.
#[derive(Component)]
pub struct ReportCheat;

/// The status line beside RANDOM RESPAWN: "waking...", or why a press did
/// nothing.
#[derive(Component)]
pub struct Note;

/// The answer in flight, and what the screen was built for.
///
/// **The screen latches on the press rather than on the wake**: a second
/// press cannot send a second action into a screen the server has already
/// closed. The sim ignores the duplicate (`world.rs`), and the player should
/// not be left wondering whether the first one took.
#[derive(Resource, Default)]
pub struct Answer {
    pub chosen: Option<Wake>,
    /// True once an answer has gone out, until the wake lands.
    pub sent: bool,
    /// What was asked for, so the wake can report which anchor answered.
    pub asked: Option<Wake>,
    /// The bags the buttons were laid out for, by address, and whether THE
    /// GATE had one: a list that moves under the screen (a raider takes a
    /// bag down) rebuilds it rather than leaving a button for a bag that is
    /// gone.
    built: Vec<(u16, u16, u8)>,
    built_gate: bool,
    /// The killer's picture, while it is still on its way.
    face: Option<(protocol::Address, u32)>,
}

/// Whether a button can be pressed now: a bag's timer off the last list,
/// THE GATE's rest and your hostility, and the beach always.
fn ready_of(core: &client_core::core::ClientCore, wake: Wake) -> Ready {
    match wake {
        Wake::Bag(i) => match core.bag_wait_left(i) {
            0 => Ready::Now,
            s => Ready::In(s),
        },
        Wake::Gate => model::gate_ready(core.gate_spawn_left().unwrap_or(0), core.hostile_left()),
        Wake::Beach => Ready::Now,
    }
}

/// Raise the screen when the core says this body died. Runs in `InWorld`.
pub fn watch(net: NonSend<Net>, mut next: ResMut<NextState<Screen>>) {
    if net.session.core.dead {
        next.set(Screen::Dead);
    }
}

/// Drop the screen when the wake lands, and say which anchor answered.
///
/// Runs in `Screen::Dead`. `woke_on_bag` outlives `dead` on purpose
/// (`core.rs`): a player who asked for a bag and got a beach is told so
/// *after* the screen closes, which is the only moment the fact is worth
/// anything.
pub fn awaken(
    net: NonSend<Net>,
    answer: Res<Answer>,
    mut toast: ResMut<Toast>,
    mut next: ResMut<NextState<Screen>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let core = &net.session.core;
    if core.dead {
        return;
    }
    // Which anchor answered a gate request: the point's rest restarting is
    // the sim's own answer (`world::wake`), and it rides the same flush as
    // the respawn. Not the predicted body, which stands where it fell until
    // the next snapshot moves it.
    let in_town = core.gate_spawn_left().is_some_and(|l| {
        l as u64 + 60 * sim_core::limits::TICK_HZ as u64
            > sim_core::world::GATE_SPAWN_COOLDOWN_TICKS
    });
    if let Some(line) = model::woke(
        answer.asked.unwrap_or(Wake::Beach),
        core.woke_on_bag,
        in_town,
    ) {
        toast.warn(line);
    }
    // Take the pointer back, so waking up does not need a click that would
    // also swing the axe — `pause::act`'s rule, for the same reason.
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = super::web::in_post();
    }
    next.set(Screen::InWorld);
}

/// Let go of the pointer and stop the corpse walking.
pub fn enter(
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut report: Query<&mut Node, With<super::report::ReportLine>>,
    look: Res<super::input::Look>,
    mut answer: ResMut<Answer>,
    mut ui: Option<ResMut<super::panels::Ui>>,
    net: Option<NonSendMut<Net>>,
) {
    *answer = Answer::default();
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
    // REPORT CHEAT's composer types above the bottom bar, not over its
    // buttons; `teardown` puts it back.
    for mut node in &mut report {
        node.bottom = Val::Px(BOTTOM_H + 14.0);
    }
    // The bag you were reading is not yours any more — it is lying on the
    // ground with your body in it. Close the panel rather than leave it
    // showing a corpse's slots behind the buttons.
    if let Some(ui) = ui.as_mut() {
        ui.panel = super::panels::Panel::None;
        ui.drag = None;
        ui.dirty = true;
    }
    // `pause::enter`'s rule: the predictor keeps applying the last frame it
    // was given, so dying mid-stride would otherwise walk the corpse through
    // the screen. Yaw and pitch are kept — where you are looking is not
    // something you are doing.
    //
    // And the torch latch lets go: the body that held the flame is gone, and
    // a latch carried through the respawn lit the kit's fresh torch with no
    // click the moment slot 2 was in hand.
    if let Some(mut net) = net {
        net.light = false;
        let sel = net.sel;
        net.session.core.set_input(
            0,
            super::input::yaw_u16(look.yaw),
            super::input::pitch_u8(look.pitch),
            0,
            0,
            sel,
        );
    }
}

/// The colour a fact's chip is.
fn tone(t: Tone) -> Color {
    match t {
        Tone::Life => LIFE,
        Tone::Foe => FOE,
        Tone::Plain => PLAIN,
    }
}

/// A button's colour at rest: a bag's own, THE GATE's gilt, the beach's
/// green.
fn ink(wake: Wake) -> Color {
    match wake {
        Wake::Bag(i) => BAG_INKS[i % BAG_INKS.len()],
        Wake::Gate => GATE_PANEL,
        Wake::Beach => GO,
    }
}

/// The same colour bright enough to read on the map's dark badges and on the
/// button's ring: the bag's, or THE GATE's own gilt.
fn map_ink(wake: Wake) -> Color {
    match wake {
        Wake::Bag(i) => BAG_MAP_INKS[i % BAG_MAP_INKS.len()],
        Wake::Gate | Wake::Beach => GATE_INK,
    }
}

/// A colour under the pointer: an eighth of the way to white.
fn lit(c: Color) -> Color {
    let s = c.to_srgba();
    let up = |v: f32| v + (1.0 - v) * 0.125;
    Color::srgba(up(s.red), up(s.green), up(s.blue), s.alpha)
}

/// The node every absolutely placed child of the island frame starts from:
/// centred on a map fraction, `px` square.
fn on_map(fx: f32, fy: f32, px: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(fx * 100.0),
        top: Val::Percent(fy * 100.0),
        width: Val::Px(px),
        height: Val::Px(px),
        margin: UiRect::axes(Val::Px(-px * 0.5), Val::Px(-px * 0.5)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    }
}

// Each is a distinct input: the session, the world's seed and sites, the
// painted island, the icons, the answer it records its layout in, and the
// window it sizes the map against.
#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Res<WorldId>,
    mut island: ResMut<super::map::Island>,
    mut images: ResMut<Assets<Image>>,
    icons: Option<Res<super::icons::Icons>>,
    mut answer: ResMut<Answer>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
) {
    let core = &net.session.core;
    let icons = icons.as_deref();
    let glyph = |k: &str| icons.and_then(|i| i.glyph(k));

    let d = Death {
        cause: core.own_death_cause,
        killer: core.own_death_killer,
        item: core.own_death_item,
        range_cm: core.own_death_range_cm,
        own_id: core.player_id,
        alive_s: core.own_death_alive_s,
    };
    let killer = crate::ui::names::of(core, d.killer);
    // While dead the predicted body IS the death position: a corpse neither
    // moves nor predicts (`ClientCore::own_bag`'s note).
    let [dx, _, dz] = core.predict.position();
    let square = map::grid_label(dx, dz);
    let (fx, fy) = map::world_to_map(dx, dz, 1);
    let meters = Meters {
        food: core.food,
        water: core.water,
    };
    let facts = model::facts(&d, &core.catalog, &killer, meters, &square);
    let player_kill = model::by_another_player(&d);
    answer.face = core
        .tag(d.killer)
        .filter(|_| player_kill)
        .map(|t| (t.address, t.pic));

    // The buttons: every bag you own, THE GATE once reached, the beach last.
    let bags = core.own_bags();
    let gate = core.gate_spawn_left().is_some();
    answer.built = bags.iter().map(|b| (b.cx, b.cz, b.level)).collect();
    answer.built_gate = gate;
    let half = sim_core::build::BUILD_CELL_M * 0.5;
    let bag_square = |b: &sim_core::deploy::BagAnchor| {
        map::grid_label(
            b.cx as f32 * sim_core::build::BUILD_CELL_M + half,
            b.cz as f32 * sim_core::build::BUILD_CELL_M + half,
        )
    };
    let town = &world.haven.town;
    let gate_square = map::grid_label(town.x, town.z);

    // The map: the named places for orientation, your bags as buttons, and
    // the skull over everything.
    let mut arc = map::ArcMarks::default();
    for (_, w) in core.arc.known() {
        arc.add_work(w.def.spot, w.name());
    }
    let mut sites = map::Marks::default();
    map::resolve_sites(&mut sites, &world.haven, &arc);
    let mut beds = map::Marks::default();
    map::resolve_wake_marks(&mut beds, bags);
    // Painted once per seed and shared with `Screen::Map`.
    let texture = island.texture(&mut images, world.seed, &world.haven);
    // The first frame's guess at the island's side; `fit` keeps it right
    // from the laid-out area after that.
    let (w, h) = windows
        .single()
        .map(|win| (win.width() / ui_scale.0, win.height() / ui_scale.0))
        .unwrap_or((1280.0, 720.0));
    let side = w.min(h - TOP_H - BOTTOM_H).max(64.0);
    let named = map::clear_labels(
        &sites.a[..sites.count],
        side,
        MAP_MARK_PX,
        SITE_LABEL_PX,
        SITE_LABEL_W,
    );

    commands
        .spawn((
            DeathRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(SHEET),
            GlobalZIndex(DEATH_Z),
        ))
        .with_children(|root| {
            // ---- the strip across the top ------------------------------
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(TOP_H),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::right(Val::Px(16.0)),
                    ..default()
                },
                BackgroundColor(BAR),
            ))
            .with_children(|top| {
                top.spawn((
                    Node {
                        width: Val::Px(DIED_W),
                        height: Val::Percent(100.0),
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(DIED),
                ))
                .with_children(|b| {
                    b.spawn(ui::strong("YOU DIED", 30.0, ui::TITLE));
                });

                top.spawn(Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::left(Val::Px(22.0)),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|row| {
                    for (i, f) in facts.iter().enumerate() {
                        if i > 0 {
                            // The rule between two facts, at the chips' middle.
                            row.spawn((
                                Node {
                                    width: Val::Px(18.0),
                                    height: Val::Px(1.0),
                                    flex_shrink: 0.0,
                                    margin: UiRect::top(Val::Px(14.0)),
                                    ..default()
                                },
                                BackgroundColor(ui::RULE),
                            ));
                        }
                        let picture = match f.kind {
                            FactKind::Weapon => icons.and_then(|i| i.item(&f.value)),
                            _ => None,
                        };
                        spawn_fact(row, f, picture, player_kill);
                    }
                });

                if player_kill {
                    top.spawn((
                        ReportCheat,
                        Button,
                        Node {
                            height: Val::Px(32.0),
                            flex_shrink: 0.0,
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(0.0)),
                            column_gap: Val::Px(8.0),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(ui::ROW_IDLE),
                        ui::Hover::default(),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Node {
                                width: Val::Px(18.0),
                                height: Val::Px(18.0),
                                border_radius: BorderRadius::MAX,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            BackgroundColor(ui::ACCENT),
                        ))
                        .with_children(|q| {
                            q.spawn(ui::strong("?", 12.0, ui::TITLE));
                        });
                        b.spawn(ui::strong("REPORT CHEAT", 13.0, ui::DIM));
                    });
                }
            });

            // ---- the island ---------------------------------------------
            root.spawn((
                MapArea,
                Node {
                    flex_grow: 1.0,
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(super::map::sea()),
            ))
            .with_children(|area| {
                area.spawn((
                    IslandFrame,
                    Node {
                        width: Val::Px(side),
                        height: Val::Px(side),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    ImageNode::new(texture),
                ))
                .with_children(|frame| {
                    spawn_grid(frame);
                    // The named places, under everything that is yours —
                    // the map screen's own badges and names.
                    for (i, m) in sites.a[..sites.count].iter().enumerate().rev() {
                        super::map::spawn_mark_named(frame, m, icons, named[i]);
                    }
                    // Where you fell: the reference's pin, its tip on the
                    // spot and the skull standing above it.
                    spawn_skull(frame, fx, fy, glyph("map_death"));
                    // Your bags over it, a button each, in `own_bags` order
                    // (`resolve_wake_marks`' push order): a bag placed where
                    // you died covers the pin's tip, not the other way
                    // round, so it still shows and still takes a click.
                    for (i, m) in beds.a[..beds.count].iter().enumerate() {
                        spawn_bag_mark(frame, m, Wake::Bag(i), glyph("map_bed"));
                    }
                });
            });

            // ---- the bar along the bottom ---------------------------------
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(BOTTOM_H),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(0.0)),
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(BAR),
            ))
            .with_children(|bar| {
                let marks = ButtonGlyphs {
                    lock: glyph("ui_lock"),
                    pin: glyph("ui_pin"),
                };
                let mut n = 0;
                // The bags and THE GATE in a row that gives way before
                // RANDOM RESPAWN does: eight bags and THE GATE are wider
                // than a small window, and the buttons narrow (to the badge
                // alone at the limit) rather than push the beach off the
                // edge. Their digits and the map still reach every one.
                bar.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_shrink: 1.0,
                    min_width: Val::Px(0.0),
                    column_gap: Val::Px(8.0),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|row| {
                    for (i, b) in bags.iter().enumerate() {
                        n += 1;
                        let title = model::bag_title(
                            b,
                            core.deploys.entries(),
                            &core.deploy_defs,
                            core.deploy_defs_have,
                            &core.catalog,
                        );
                        // The deployable's own picture, which reads as a
                        // sleeping bag at button size where the map's white
                        // glyph does not; the glyph if it has none.
                        let picture = icons
                            .and_then(|i| i.item(&title))
                            .or_else(|| glyph("map_bed"));
                        spawn_wake_button(
                            row,
                            Wake::Bag(i),
                            &title,
                            bag_square(b),
                            n,
                            picture,
                            &marks,
                        );
                    }
                    if gate {
                        n += 1;
                        spawn_wake_button(
                            row,
                            Wake::Gate,
                            "THE GATE",
                            gate_square.clone(),
                            n,
                            glyph("map_site"),
                            &marks,
                        );
                    }
                });
                // The space between, which says why it is empty when it is.
                bar.spawn((
                    Node {
                        flex_grow: 1.0,
                        padding: UiRect::horizontal(Val::Px(12.0)),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|gap| {
                    if bags.is_empty() {
                        gap.spawn(ui::label(model::NO_BAGS, 13.0, ui::FAINT));
                    }
                });
                bar.spawn((Note, ui::label("", 13.0, ui::DIM)));
                n += 1;
                spawn_random(bar, n, glyph("ui_respawn"));
            });
        });
}

/// One fact: the caption over its chip, the chip carrying a picture where
/// the fact has one — the killer's face for a player, the weapon's icon.
fn spawn_fact(
    row: &mut ChildSpawnerCommands,
    f: &Fact,
    picture: Option<Handle<Image>>,
    player_kill: bool,
) {
    row.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        row_gap: Val::Px(4.0),
        ..default()
    })
    .with_children(|col| {
        col.spawn(ui::strong(f.caption, 10.0, ui::FAINT));
        col.spawn((
            Node {
                height: Val::Px(CHIP_H),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(0.0)),
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(tone(f.tone)),
        ))
        .with_children(|chip| {
            if f.kind == FactKind::Killer && player_kill {
                // The face: the name's first letter on a dark square until
                // the picture lands, and the picture over it once it does.
                let initial: String = f.value.chars().take(1).collect::<String>().to_uppercase();
                chip.spawn((
                    Node {
                        width: Val::Px(CHIP_PIC),
                        height: Val::Px(CHIP_PIC),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                ))
                .with_children(|face| {
                    face.spawn(ui::strong(initial, 12.0, ui::TEXT));
                    face.spawn((
                        KillerFace,
                        ImageNode::default(),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(CHIP_PIC),
                            height: Val::Px(CHIP_PIC),
                            ..default()
                        },
                        Visibility::Hidden,
                    ));
                });
            }
            if let Some(h) = picture {
                chip.spawn((
                    ImageNode::new(h),
                    Node {
                        width: Val::Px(CHIP_PIC),
                        height: Val::Px(CHIP_PIC),
                        ..default()
                    },
                ));
            }
            chip.spawn(ui::strong(f.value.clone(), 14.0, ui::TITLE));
        });
    });
}

/// The skull pin where you fell: a zero-size anchor on the spot, a dot on
/// it, a stem up from it and the skull's disc on top.
fn spawn_skull(frame: &mut ChildSpawnerCommands, fx: f32, fy: f32, skull: Option<Handle<Image>>) {
    let at = |left: f32, top: f32, w: f32, h: f32| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(left),
        top: Val::Px(top),
        width: Val::Px(w),
        height: Val::Px(h),
        ..default()
    };
    frame
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(fx * 100.0),
                top: Val::Percent(fy * 100.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|pin| {
            pin.spawn((
                Node {
                    border_radius: BorderRadius::MAX,
                    ..at(-3.0, -3.0, 6.0, 6.0)
                },
                BackgroundColor(SKULL),
                Pickable::IGNORE,
            ));
            pin.spawn((
                at(-1.0, -PIN_STEM, 2.0, PIN_STEM),
                BackgroundColor(SKULL),
                Pickable::IGNORE,
            ));
            pin.spawn((
                Node {
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::MAX,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..at(-PIN_PX * 0.5, -PIN_STEM - PIN_PX, PIN_PX, PIN_PX)
                },
                BackgroundColor(SKULL),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.9)),
                Pickable::IGNORE,
            ))
            .with_children(|disc| {
                if let Some(h) = skull {
                    disc.spawn((
                        ImageNode::new(h).with_color(Color::WHITE),
                        Node {
                            width: Val::Px(PIN_PX * 0.62),
                            height: Val::Px(PIN_PX * 0.62),
                            ..default()
                        },
                    ));
                }
            });
        });
}

/// The grid, the map screen's: faint lines and every cell's label, so the
/// square on a button and the square on the map can be read against each
/// other.
fn spawn_grid(frame: &mut ChildSpawnerCommands) {
    for i in 1..GRID_COLS {
        let t = i as f32 / GRID_COLS as f32 * 100.0;
        frame.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(t),
                top: Val::Px(0.0),
                width: Val::Px(1.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.18)),
            Pickable::IGNORE,
        ));
        frame.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(t),
                left: Val::Px(0.0),
                height: Val::Px(1.0),
                width: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.18)),
            Pickable::IGNORE,
        ));
    }
    let cell = 100.0 / GRID_COLS as f32;
    for row in 0..GRID_COLS {
        for col in 0..GRID_COLS {
            frame.spawn((
                ui::label(map::grid_cell_label(col, row), GRID_LABEL_PX, GRID_INK),
                ui::TEXT_SHADOW,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(col as f32 * cell),
                    top: Val::Percent(row as f32 * cell),
                    padding: UiRect::axes(Val::Px(3.0), Val::Px(1.0)),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
    }
}

/// A bag of yours on the map: the map screen's bed badge — solid when it
/// will answer, hollow while it cools — in the bag's own colour, made a
/// button, with a ring that lights while it or its button is under the
/// pointer.
fn spawn_bag_mark(
    frame: &mut ChildSpawnerCommands,
    m: &map::Mark,
    wake: Wake,
    picture: Option<Handle<Image>>,
) {
    let spent = m.kind == map::MarkKind::BedSpent;
    let ink = map_ink(wake);
    frame
        .spawn((
            WakeMark(wake),
            Button,
            Node {
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..on_map(m.px, m.py, BAG_MARK_PX)
            },
            BackgroundColor(if spent { BADGE_HOLLOW } else { BADGE_SOLID }),
            BorderColor::all(ink),
        ))
        .with_children(|badge| {
            if let Some(h) = picture {
                let inner = BAG_MARK_PX * MARK_ICON_FRAC;
                badge.spawn((
                    ImageNode::new(h).with_color(ink),
                    Node {
                        width: Val::Px(inner),
                        height: Val::Px(inner),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            badge.spawn((
                MarkRing(wake),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(-6.0),
                    top: Val::Px(-6.0),
                    right: Val::Px(-6.0),
                    bottom: Val::Px(-6.0),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BorderColor::all(Color::WHITE),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        });
}

/// The digit in a button's corner, the key that presses it.
fn spawn_digit(b: &mut ChildSpawnerCommands, n: usize) {
    if let Some(c) = model::digit(n) {
        b.spawn((
            ui::strong(c.to_string(), 11.0, ui::FAINT),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(3.0),
                right: Val::Px(6.0),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
}

/// The two glyphs every wake button carries beside its own picture: the
/// padlock that stands in for it while it cools, and the pin before its
/// grid square.
struct ButtonGlyphs {
    lock: Option<Handle<Image>>,
    pin: Option<Handle<Image>>,
}

/// A bag's (or THE GATE's) button: the round badge, the name, and the line
/// under it with the pin and the grid square — or the timer while it cools.
fn spawn_wake_button(
    bar: &mut ChildSpawnerCommands,
    wake: Wake,
    title: &str,
    square: String,
    n: usize,
    badge: Option<Handle<Image>>,
    marks: &ButtonGlyphs,
) {
    bar.spawn((
        WakeButton(wake),
        Button,
        Node {
            width: Val::Px(BUTTON_W),
            min_width: Val::Px(58.0),
            height: Val::Px(BUTTON_H),
            flex_shrink: 1.0,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(10.0), Val::Px(0.0)),
            column_gap: Val::Px(10.0),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(ink(wake)),
    ))
    .with_children(|b| {
        // The round badge, ringed in the colour the map draws this bag in:
        // the bag's picture while it will answer, a padlock while it cools
        // (the reference's swap), toggled by `refresh`.
        b.spawn((
            Node {
                width: Val::Px(36.0),
                height: Val::Px(36.0),
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.4)),
            BorderColor::all(map_ink(wake)),
            Pickable::IGNORE,
        ))
        .with_children(|disc| {
            if let Some(h) = badge {
                disc.spawn((
                    WakeIcon { wake, lock: false },
                    ImageNode::new(h),
                    Node {
                        width: Val::Px(26.0),
                        height: Val::Px(26.0),
                        ..default()
                    },
                ));
            }
            if let Some(h) = marks.lock.clone() {
                disc.spawn((
                    WakeIcon { wake, lock: true },
                    ImageNode::new(h).with_color(COLD_TEXT),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(18.0),
                        height: Val::Px(18.0),
                        ..default()
                    },
                    Visibility::Hidden,
                ));
            }
        });
        b.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|col| {
            col.spawn(ui::strong(title, 15.0, ui::TEXT));
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|line| {
                if let Some(h) = marks.pin.clone() {
                    line.spawn((
                        ImageNode::new(h).with_color(ui::DIM),
                        Node {
                            width: Val::Px(11.0),
                            height: Val::Px(11.0),
                            ..default()
                        },
                    ));
                }
                line.spawn((
                    WakeLine {
                        wake,
                        square: square.clone(),
                        shown: None,
                    },
                    ui::strong(square, 12.0, ui::DIM),
                ));
            });
        });
        spawn_digit(b, n);
    });
}

/// RANDOM RESPAWN, bottom right: the beach, always there.
fn spawn_random(bar: &mut ChildSpawnerCommands, n: usize, turn: Option<Handle<Image>>) {
    bar.spawn((
        WakeButton(Wake::Beach),
        Button,
        Node {
            width: Val::Px(RANDOM_W),
            height: Val::Px(BUTTON_H),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(10.0),
            ..default()
        },
        BackgroundColor(GO),
    ))
    .with_children(|b| {
        if let Some(h) = turn {
            b.spawn((
                ImageNode::new(h).with_color(ui::TITLE),
                Node {
                    width: Val::Px(20.0),
                    height: Val::Px(20.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        b.spawn(ui::strong("RANDOM RESPAWN", 17.0, ui::TITLE));
        spawn_digit(b, n);
    });
}

/// Keep the island square and as large as the middle of the screen allows,
/// off the laid-out area — `render::map::view`'s rule, so a `UiScale` (the
/// browser build sets one) is already divided out.
pub fn fit(
    area: Query<&ComputedNode, With<MapArea>>,
    mut frame: Query<&mut Node, With<IslandFrame>>,
) {
    let Ok(c) = area.single() else {
        return;
    };
    let size = c.size() * c.inverse_scale_factor();
    let side = size.x.min(size.y).floor();
    if side < 2.0 {
        return;
    }
    if let Ok(mut node) = frame.single_mut() {
        if node.width != Val::Px(side) {
            node.width = Val::Px(side);
            node.height = Val::Px(side);
        }
    }
}

/// Take a press: a button or a bag on the map that is ready becomes the
/// answer, and one that is not says why in the note.
fn choose(
    core: &client_core::core::ClientCore,
    wake: Wake,
    answer: &mut Answer,
    notes: &mut Query<&mut Text, With<Note>>,
) {
    if answer.sent {
        return;
    }
    let why = match ready_of(core, wake) {
        Ready::Now => {
            answer.chosen = Some(wake);
            return;
        }
        Ready::In(s) if wake == Wake::Gate => format!("THE GATE is resting - {}", model::clock(s)),
        Ready::In(s) => format!("that bag is still cooling - {}", model::clock(s)),
        Ready::Hostile(s) => format!(
            "THE GATE will not take you while you are hostile - {}",
            model::clock(s)
        ),
    };
    if let Ok(mut text) = notes.single_mut() {
        text.0 = why;
    }
}

// The three clickable kinds and REPORT CHEAT's two ends, the composer and
// the facts it is filled from.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn click(
    buttons: Query<(&Interaction, &WakeButton), Changed<Interaction>>,
    marks: Query<(&Interaction, &WakeMark), Changed<Interaction>>,
    cheat: Query<&Interaction, (Changed<Interaction>, With<ReportCheat>)>,
    net: NonSend<Net>,
    mut answer: ResMut<Answer>,
    reports: Option<ResMut<super::report::Reports>>,
    mut notes: Query<&mut Text, With<Note>>,
) {
    let core = &net.session.core;
    let pressed = buttons
        .iter()
        .filter(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, w)| w.0)
        .chain(
            marks
                .iter()
                .filter(|(i, _)| **i == Interaction::Pressed)
                .map(|(_, w)| w.0),
        )
        .last();
    if let Some(wake) = pressed {
        choose(core, wake, &mut answer, &mut notes);
    }

    // REPORT CHEAT: the report composer, opened with the kill already
    // written (`ui::death::report_title`) and this screen as its picture.
    if cheat.iter().any(|i| *i == Interaction::Pressed) {
        if let Some(mut reports) = reports {
            let d = Death {
                cause: core.own_death_cause,
                killer: core.own_death_killer,
                item: core.own_death_item,
                range_cm: core.own_death_range_cm,
                own_id: core.player_id,
                alive_s: core.own_death_alive_s,
            };
            let [x, _, z] = core.predict.position();
            let title = model::report_title(
                &d,
                &core.catalog,
                &crate::ui::names::of(core, d.killer),
                &map::grid_label(x, z),
            );
            reports.open_with(&title);
        }
    }
}

pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    net: NonSend<Net>,
    mut answer: ResMut<Answer>,
    reports: Option<Res<super::report::Reports>>,
    mut notes: Query<&mut Text, With<Note>>,
) {
    if answer.sent {
        return;
    }
    // The report composer owns the keyboard while it is open: a digit typed
    // into it is a word, not a respawn.
    if reports.is_some_and(|r| r.open) {
        return;
    }
    let core = &net.session.core;
    let (bags, gate) = (core.own_bags().len(), core.gate_spawn_left().is_some());
    // **Digits only, and POSITIONAL**: a digit means the button it is
    // printed on. No letters — `G` is the map everywhere else in the game,
    // and a key somebody already knows for something else is the one most
    // likely to be pressed here by mistake. **Escape is not bound either**:
    // there is nothing to back out to.
    for (i, key) in DIGITS.iter().enumerate() {
        if keyboard.just_pressed(*key) {
            if let Some(wake) = model::wake_at(bags, gate, i + 1) {
                choose(core, wake, &mut answer, &mut notes);
            }
        }
    }
}

/// Send the answer, once.
pub fn act(mut answer: ResMut<Answer>, net: NonSend<Net>, mut notes: Query<&mut Text, With<Note>>) {
    let Some(wake) = answer.chosen.take() else {
        return;
    };
    if answer.sent {
        return;
    }
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    let bags = net.session.core.own_bags();
    let encoded = match wake {
        Wake::Gate => protocol::encode_action_respawn_gate(&mut buf),
        // The bag by its address. One that went cold since the list was
        // sent falls back to the nearest ready one, then the beach — the
        // sim's rule (`Deploys::claim_bag_at`), and the toast says which.
        Wake::Bag(i) => match bags.get(i) {
            Some(b) => protocol::encode_action_respawn_at(b.cx, b.cz, b.level, &mut buf),
            None => protocol::encode_action_respawn(true, &mut buf),
        },
        Wake::Beach => protocol::encode_action_respawn(false, &mut buf),
    };
    let say = |notes: &mut Query<&mut Text, With<Note>>, what: String| {
        if let Ok(mut text) = notes.single_mut() {
            text.0 = what;
        }
    };
    match encoded {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => {
                answer.sent = true;
                answer.asked = Some(wake);
                say(&mut notes, "waking...".to_string());
            }
            // A full lane means the respawn was NOT sent (wall 4). Leaving
            // `sent` false is what lets the player press again, which is the
            // whole point of reporting rather than dropping.
            Err(e) => say(&mut notes, e.to_string()),
        },
        Err(e) => say(&mut notes, format!("respawn would not encode ({e:?})")),
    }
}

/// Every frame the screen is up: the timers, which buttons light, which bag
/// on the map is ringed, and the killer's face once it lands. And a bag list
/// that moved under the screen rebuilds it.
// The buttons, the map's bags and rings, the lines, the face: five node
// kinds, each written here and nowhere else.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn refresh(
    net: NonSend<Net>,
    mut answer: ResMut<Answer>,
    mut next: ResMut<NextState<Screen>>,
    mut buttons: Query<(&WakeButton, &Interaction, &mut BackgroundColor), Without<WakeMark>>,
    mut marks: Query<(&WakeMark, &Interaction, &mut BackgroundColor), Without<WakeButton>>,
    mut rings: Query<(&MarkRing, &mut Visibility), (Without<KillerFace>, Without<WakeIcon>)>,
    mut icons: Query<(&WakeIcon, &mut Visibility), (Without<KillerFace>, Without<MarkRing>)>,
    mut lines: Query<(&mut WakeLine, &mut Text, &mut TextColor)>,
    mut face: Query<(&mut ImageNode, &mut Visibility), (With<KillerFace>, Without<WakeIcon>)>,
    pics: Option<ResMut<super::faces::Pics>>,
) {
    let core = &net.session.core;
    if !answer.sent {
        let bags = core.own_bags();
        let moved = bags.len() != answer.built.len()
            || bags
                .iter()
                .zip(&answer.built)
                .any(|(b, k)| (b.cx, b.cz, b.level) != *k);
        if moved || core.gate_spawn_left().is_some() != answer.built_gate {
            // OnExit then OnEnter again: `set` runs the transition even for
            // the state already current (`set_if_neq` is the one that
            // does not).
            next.set(Screen::Dead);
            return;
        }
    }

    let hot = buttons
        .iter()
        .find(|(_, i, _)| **i != Interaction::None)
        .map(|(w, _, _)| w.0)
        .or_else(|| {
            marks
                .iter()
                .find(|(_, i, _)| **i != Interaction::None)
                .map(|(w, _, _)| w.0)
        });
    let on = |w: Wake| hot == Some(w) || answer.asked == Some(w);
    for (w, _, mut bg) in &mut buttons {
        let want = match (w.0, ready_of(core, w.0)) {
            (Wake::Beach, _) if on(w.0) => GO_HOT,
            (_, Ready::Now) if on(w.0) => lit(ink(w.0)),
            (_, Ready::Now) => ink(w.0),
            _ => COLD,
        };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    // The bag on the map follows its button: solid once it will answer,
    // hollow while it cools, on the same clock.
    for (m, _, mut bg) in &mut marks {
        let want = if ready_of(core, m.0) == Ready::Now {
            BADGE_SOLID
        } else {
            BADGE_HOLLOW
        };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    for (icon, mut v) in &mut icons {
        let cold = ready_of(core, icon.wake) != Ready::Now;
        let want = if icon.lock == cold {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    for (r, mut v) in &mut rings {
        let want = if on(r.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    // Rewritten when the second changes, not every frame.
    for (mut l, mut text, mut color) in &mut lines {
        let ready = ready_of(core, l.wake);
        if l.shown == Some(ready) {
            continue;
        }
        l.shown = Some(ready);
        text.0 = model::wake_line(&l.square, ready);
        color.0 = if ready == Ready::Now {
            ui::DIM
        } else {
            COLD_TEXT
        };
    }
    if let (Some((address, pic)), Some(mut pics)) = (answer.face, pics) {
        if let Some(h) = pics.want(address, pic) {
            for (mut img, mut v) in &mut face {
                img.image = h.clone();
                *v = Visibility::Inherited;
            }
            answer.face = None;
        }
    }
}

pub fn teardown(
    mut commands: Commands,
    roots: Query<Entity, With<DeathRoot>>,
    mut report: Query<&mut Node, With<super::report::ReportLine>>,
) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    // The report composer's line back where every other screen has it.
    for mut node in &mut report {
        node.bottom = Val::Px(super::report::LINE_BOTTOM);
    }
}

#[cfg(test)]
mod tests {
    use super::DIGITS;

    /// A button the keyboard cannot reach is a button half the players will
    /// not use: every bag a player can own, THE GATE and RANDOM RESPAWN each
    /// need a digit, and the digit list is hand-written.
    #[test]
    fn the_digits_cover_every_button_the_bar_can_hold() {
        let most = crate::ui::death::wakes(sim_core::deploy::BAG_CAP, true).count();
        assert!(
            DIGITS.len() >= most,
            "{most} buttons and {} digits",
            DIGITS.len()
        );
    }
}
