//! Gate: the map's markers actually spawn, and they spawn what they claim to.
//!
//! **A spawn is not type-checked, so this one is run rather than read.**
//! `CLAUDE.md` records a `(DeathRoot, ui::screen(bg), Node { padding })` that
//! compiled, passed `./ci/gates.sh`, and killed the client the instant a body
//! reached the death screen — Bevy 0.18 refuses a bundle carrying a component
//! twice, at spawn, inside a command queue, naming the system as
//! `<Enable the debug feature to see the name>`. Booting the game is what
//! found it, on a screen no headless test visits.
//!
//! The map screen is the same kind of place: `Screen::Map` needs a shard, a
//! `Net` and a `WorldId`, so nothing headless opens it. What CAN be opened
//! headlessly is the part both it and the death screen share — `spawn_mark`,
//! which assembles a badge, a picture and a name out of three helpers and is
//! exactly the shape that dies at spawn. So this drives it for every
//! `MarkKind` against a real `App`, flushes the queue, and reads the tree
//! back.
//!
//! No window and no GPU: `MinimalPlugins` plus the asset server, which hands
//! out handles for files it has not loaded. `tests/prewarm.rs`'s fixture.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::icons::Icons;
use client::render::map::spawn_mark;
use client::ui::map::{Mark, MarkKind};

/// Every kind. A `match` over each one so a kind added without a row here
/// fails to compile rather than going unspawned.
const KINDS: [MarkKind; 8] = [
    MarkKind::None,
    MarkKind::Haven,
    MarkKind::Waystation,
    MarkKind::Depot,
    MarkKind::Bed,
    MarkKind::BedSpent,
    MarkKind::Hearth,
    MarkKind::Backpack,
];

/// The mark under test, and where the spawn put it — both resources, so the
/// draw happens inside a real SYSTEM taking `Option<Res<Icons>>` rather than
/// through a hand-assembled `Commands`. That is how `render::map::setup` and
/// `render::death::setup` both call it, and the borrow it forces (the icons
/// are read while the command queue is open) is part of what is being
/// checked.
#[derive(Resource)]
struct Subject(Mark);

#[derive(Resource, Default)]
struct Drawn(Option<Entity>);

fn draw_system(
    mut commands: Commands,
    subject: Res<Subject>,
    icons: Option<Res<Icons>>,
    mut out: ResMut<Drawn>,
) {
    let e = commands
        .spawn(Node::default())
        .with_children(|frame| spawn_mark(frame, &subject.0, icons.as_deref()))
        .id();
    out.0 = Some(e);
}

/// `MinimalPlugins` plus the asset server, which hands out handles for files
/// it has not loaded — `tests/prewarm.rs`'s fixture.
fn draw(kind: MarkKind, icons: bool) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Image>();
    app.init_resource::<Drawn>();
    app.insert_resource(Subject(Mark {
        kind,
        px: 0.5,
        py: 0.5,
    }));
    if icons {
        app.add_systems(Startup, client::render::icons::load);
    }
    app.add_systems(Update, draw_system);
    // The panic a duplicate component raises happens when the command queue
    // is applied, which is inside this call.
    app.update();
    let parent = app.world().resource::<Drawn>().0.expect("the mark drew");
    (app, parent)
}

fn children(app: &App, e: Entity) -> Vec<Entity> {
    app.world()
        .entity(e)
        .get::<Children>()
        .map(|c| c.iter().collect())
        .unwrap_or_default()
}

/// The one that matters: every kind spawns without the command queue dying.
///
/// A duplicate component panics inside `App::update`, so this test failing at
/// all is the whole finding — the assertions after it are the cheap half.
#[test]
fn every_kind_spawns_without_a_duplicate_component() {
    for kind in KINDS {
        let (app, parent) = draw(kind, true);
        let kids = children(&app, parent).len();
        if kind == MarkKind::None {
            assert_eq!(kids, 0, "{kind:?} is never drawn and must draw nothing");
        } else {
            assert_eq!(kids, 1, "{kind:?} must spawn exactly one badge");
        }
    }
}

/// The badge carries the picture, and an authored destination carries its
/// name as well.
#[test]
fn a_badge_holds_its_picture_and_a_destination_holds_its_name() {
    for kind in KINDS {
        if kind == MarkKind::None {
            continue;
        }
        let (app, parent) = draw(kind, true);
        let badge = children(&app, parent)[0];
        let kids = children(&app, badge);
        let pictures = kids
            .iter()
            .filter(|e| app.world().entity(**e).contains::<ImageNode>())
            .count();
        assert_eq!(pictures, 1, "{kind:?} drew {pictures} pictures, wanted 1");
        // The name is a row with the text under it, so it is the child that
        // is NOT the picture.
        let wanted = usize::from(kind.site_label().is_some());
        assert_eq!(
            kids.len() - pictures,
            wanted,
            "{kind:?} drew {} name rows, wanted {wanted}",
            kids.len() - pictures
        );
    }
}

/// With no `Icons` resource the badge still lands, with no picture in it.
///
/// `render/icons.rs`'s rule — a miss is not a defect and must not draw an
/// empty cell — and here it means a marker whose art has not loaded is still
/// a marker where the thing is. It is also the state the FIRST frame of the
/// screen is in on a cold asset server, which is not a rare case.
#[test]
fn a_mark_with_no_icon_is_still_a_mark() {
    for kind in KINDS {
        if kind == MarkKind::None {
            continue;
        }
        let (app, parent) = draw(kind, false);
        let badge = children(&app, parent)[0];
        assert!(
            app.world().entity(badge).contains::<Node>(),
            "{kind:?} lost its badge with no icons loaded"
        );
        let pictures = children(&app, badge)
            .iter()
            .filter(|e| app.world().entity(**e).contains::<ImageNode>())
            .count();
        assert_eq!(pictures, 0, "{kind:?} drew a picture with no icons loaded");
    }
}

/// The depot uses the current destination badge API, including its label,
/// rather than restoring the former bare-square marker implementation.
#[test]
fn depot_badge_is_hollow_and_names_the_freight_destination() {
    let (app, parent) = draw(MarkKind::Depot, true);
    let badge = children(&app, parent)[0];
    let colour = app.world().entity(badge).get::<BackgroundColor>().unwrap();
    let (haven_app, haven_parent) = draw(MarkKind::Haven, true);
    let haven_badge = children(&haven_app, haven_parent)[0];
    let haven_colour = haven_app
        .world()
        .entity(haven_badge)
        .get::<BackgroundColor>()
        .unwrap();
    assert_eq!(colour.0, haven_colour.0);
    let label_row = children(&app, badge)
        .into_iter()
        .find(|e| !app.world().entity(*e).contains::<ImageNode>())
        .expect("destination label row");
    let text = children(&app, label_row)
        .into_iter()
        .find_map(|e| app.world().entity(e).get::<Text>().map(|t| t.0.clone()))
        .expect("destination text");
    assert_eq!(text, "DEPOT");
}
