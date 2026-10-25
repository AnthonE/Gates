//! Gate: the loading screen and every settings tab spawn whole.
//!
//! **A spawn is not type-checked, so it is run rather than read** — the trap
//! `tests/spectate_label.rs` exists for: two of one component in a bundle
//! compiles and then panics inside the command flush. These screens are only
//! ever built in a windowed run, so this builds them in a headless `App` (no
//! window, no GPU) and reads a little of the tree back.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::binds::{Action, Binds};
use client::render::loading::{self, LoadingCam, LoadingRoot};
use client::render::screen::{Connecting, Direct, Menu};
use client::render::settings::{self, BindCell, Settings, SettingsRoot, CATEGORIES};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_resource::<Binds>();
    app
}

#[test]
fn the_loading_screen_spawns_with_its_own_camera() {
    let mut app = app();
    app.insert_non_send_resource(Connecting::default());
    app.insert_resource(Menu::new("127.0.0.1:4433", None));
    app.insert_resource(Direct("127.0.0.1:4433".into()));
    app.add_systems(Update, loading::setup);
    // A duplicate component in a bundle panics here, inside the flush.
    app.update();
    let world = app.world_mut();
    assert_eq!(world.query::<&LoadingRoot>().iter(world).count(), 1);
    assert_eq!(world.query::<&LoadingCam>().iter(world).count(), 1);
    let mut texts = world.query::<&Text>();
    assert!(
        texts.iter(world).any(|t| t.0 == "ENTERING 127.0.0.1:4433"),
        "the title names the address when the menu has no name for it"
    );
}

#[test]
fn every_settings_tab_spawns_and_keybinds_has_a_cell_per_slot() {
    let mut app = app();
    app.insert_resource(Settings::default());
    app.add_systems(Update, settings::setup);
    app.update();
    // Now walk the rail: `rebuild` respawns the screen on `dirty`.
    let mut app2 = self::app();
    app2.insert_resource(Settings::default());
    app2.add_systems(Update, settings::rebuild);
    for (cat, name) in CATEGORIES.iter().enumerate() {
        {
            let mut s = app2.world_mut().resource_mut::<Settings>();
            s.cat = cat;
            s.dirty = true;
        }
        app2.update();
        let world = app2.world_mut();
        assert!(
            world.query::<&SettingsRoot>().iter(world).count() >= 1,
            "{name} drew nothing"
        );
        if *name == "KEYBINDS" {
            let cells = world.query::<&BindCell>().iter(world).count();
            assert_eq!(cells, client::render::binds::COUNT * 2);
        }
    }
    // A capture in progress draws its cell differently, and still spawns.
    {
        let mut s = app2.world_mut().resource_mut::<Settings>();
        s.capturing = Some((Action::Interact, 0));
        s.dirty = true;
    }
    app2.update();
    let world = app2.world_mut();
    let mut texts = world.query::<&Text>();
    assert!(texts.iter(world).any(|t| t.0 == "PRESS A KEY"));
}

/// Bevy keys a 2D camera's depth texture by its target alone, not by its
/// sample count, so two `Camera2d`s on the window with different `Msaa`
/// share one depth texture and wgpu refuses the pass ("differing sample
/// counts", #492). Escape on the loading screen is one way to get two: the
/// loading camera stays up through its fade while the next screen spawns
/// its own.
#[test]
fn a_screen_over_the_fading_loading_camera_shares_its_msaa() {
    use bevy::state::app::StatesPlugin;
    use client::render::disconnected::{self, Reason};
    let mut app = app();
    app.add_plugins(StatesPlugin);
    app.insert_non_send_resource(Connecting::default());
    app.insert_resource(Menu::new("127.0.0.1:4433", None));
    app.insert_resource(Direct("127.0.0.1:4433".into()));
    app.init_resource::<Reason>();
    app.add_systems(Update, loading::setup.run_if(run_once));
    app.update();
    app.add_systems(
        PostUpdate,
        (loading::teardown, disconnected::setup).run_if(run_once),
    );
    app.update();
    let world = app.world_mut();
    assert_eq!(
        world.query::<&LoadingCam>().iter(world).count(),
        1,
        "still fading"
    );
    // The render plugin adds a missing `Msaa` as its default, 4×; this app
    // has no render plugin, so read a missing one the way it would.
    let msaa: Vec<Msaa> = world
        .query_filtered::<Option<&Msaa>, With<Camera2d>>()
        .iter(world)
        .map(|m| m.copied().unwrap_or_default())
        .collect();
    assert!(msaa.len() >= 2);
    assert!(msaa.iter().all(|m| *m == msaa[0]), "{msaa:?}");
}
