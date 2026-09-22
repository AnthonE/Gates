//! Gate: the spectator's label spawns, and says whose view it is.
//!
//! **A spawn is not type-checked, so it is run rather than read** — the
//! trap `tests/map_marks.rs` exists for. The label only appears on a seat,
//! and a seat needs a shard, so nothing headless would otherwise ever build
//! it: this drives `render::spectate::spawn_label` from a real system in a
//! real `App` (no window, no GPU) and reads the tree back.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::spectate::{spawn_label, SpectateLabel};

#[derive(Resource)]
struct Subject(protocol::Watch);

fn draw(mut commands: Commands, subject: Res<Subject>) {
    spawn_label(&mut commands, &subject.0);
}

#[test]
fn the_label_spawns_whole_and_names_the_watched_player() {
    let w = protocol::Watch {
        address: protocol::Address::from_hex(b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf")
            .unwrap(),
        agent: true,
        name: protocol::Name::new("jev").unwrap(),
    };
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.insert_resource(Subject(w));
    app.add_systems(Update, draw);
    // A duplicate component in the bundle panics here, inside the flush.
    app.update();
    let world = app.world_mut();
    let mut q = world.query_filtered::<&Text, With<SpectateLabel>>();
    let texts: Vec<String> = q.iter(world).map(|t| t.0.clone()).collect();
    assert_eq!(texts, vec![client::ui::spectate::label(&w)]);
    assert!(texts[0].starts_with("SPECTATING jev"), "{texts:?}");
}
