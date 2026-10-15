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
