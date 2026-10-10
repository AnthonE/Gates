//! Gate: the world-space tags (NOW §0x 2) spawn, one per fact, hidden.
//!
//! **A spawn is not type-checked, so it is run rather than read** — the
//! trap `tests/spectate_label.rs` exists for. The tags spawn on entering a
//! world, and a world needs a shard, so nothing headless would otherwise
//! ever build them: this runs `render::anchor::setup` in a real `App` (no
//! window, no GPU) and reads the tree back.

#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use client::render::anchor::{setup, Tag};

#[test]
fn the_tags_spawn_whole_and_hidden() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.add_systems(Update, setup);
    // A duplicate component in a bundle panics here, inside the flush.
    app.update();
    let world = app.world_mut();
    let mut q = world.query::<(&Tag, &Visibility, &Text)>();
    let mut tags: Vec<(Tag, Visibility, String)> = q
        .iter(world)
        .map(|(t, v, x)| (*t, *v, x.0.clone()))
        .collect();
    tags.sort_by_key(|(t, ..)| *t as u8);
    assert_eq!(
        tags,
        vec![
            (Tag::Charge, Visibility::Hidden, String::new()),
            (Tag::Wall, Visibility::Hidden, String::new()),
        ],
        "one tag per fact, and nothing drawn until there is something to say"
    );
}
