//! A material created before its maps' chains land is re-prepared when they
//! do — `mipmap::retouch`, the half of the mip pass the direct-connect paths
//! were missing (`NOW.md` §0web item 3; `mipmap.rs`'s header).
//!
//! **The defect this gates is invisible to every value test in the tree.**
//! `drain` gives an image its chain and re-uploads it; `bevy_pbr` prepares a
//! material's bind group once, from the GPU image as it stood, and never
//! again on `AssetEvent<Image>::Modified`. So a `StandardMaterial` that
//! existed the frame before its albedo got a chain keeps binding the
//! one-level texture forever, with the image itself, its descriptor and
//! every gate over `chain()` all correct. The only observable is that the
//! MATERIAL was marked modified after the image was — which is what this
//! reads, off Bevy's own `AssetEvent<StandardMaterial>` stream.
//!
//! Driven through a headless `App` because the claim is about scheduling:
//! `retouch` runs after `drain` in the same frame, sees the ids `drain` just
//! chained, and touches exactly the materials bound to them. Three mutants
//! were run: `retouch` touching every material (the unbound one goes red),
//! `drain` not recording the id (the bound one goes red), and `Chained` not
//! being cleared (the second frame goes red).
#![cfg(feature = "render")]

use bevy::asset::{AssetEvent, AssetPlugin, Assets, Handle};
use bevy::ecs::message::MessageCursor;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use client::render::mipmap::{self, binds, Chained, Pending};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Image>();
    app.init_asset::<StandardMaterial>();
    app.init_resource::<Pending>();
    app.init_resource::<Chained>();
    app.add_systems(
        Update,
        (mipmap::drain, mipmap::retouch.after(mipmap::drain)),
    );
    app
}

/// A one-level power-of-two RGBA8 image, as a loaded photograph arrives.
fn photo(n: u32) -> Image {
    let data = (0..n * n)
        .flat_map(|i| [(i % 251) as u8, (i % 241) as u8, (i % 239) as u8, 255])
        .collect::<Vec<u8>>();
    Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    )
}

/// `Modified` events for materials, drained since the last call.
fn modified(
    app: &mut App,
    cursor: &mut MessageCursor<AssetEvent<StandardMaterial>>,
) -> Vec<AssetId<StandardMaterial>> {
    let events = app
        .world()
        .resource::<Messages<AssetEvent<StandardMaterial>>>();
    cursor
        .read(events)
        .filter_map(|e| match e {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn a_material_bound_to_a_freshly_chained_image_is_touched_and_only_that_one() {
    let mut app = app();
    let (image, other): (Handle<Image>, Handle<Image>) = {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        (images.add(photo(64)), images.add(photo(64)))
    };
    let (bound, unbound, bound_by_normal): (
        Handle<StandardMaterial>,
        Handle<StandardMaterial>,
        Handle<StandardMaterial>,
    ) = {
        let mut mats = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
        (
            mats.add(StandardMaterial {
                base_color_texture: Some(image.clone()),
                ..default()
            }),
            mats.add(StandardMaterial {
                base_color_texture: Some(other.clone()),
                ..default()
            }),
            mats.add(StandardMaterial {
                normal_map_texture: Some(image.clone()),
                ..default()
            }),
        )
    };
    // One frame with nothing pending: the `Added` events flush, nothing is
    // modified.
    let mut cursor = app
        .world()
        .resource::<Messages<AssetEvent<StandardMaterial>>>()
        .get_cursor();
    app.update();
    assert!(
        modified(&mut app, &mut cursor).is_empty(),
        "a material was touched with nothing chained"
    );

    // The image is queued as `enqueue` would queue it, and the frame chains
    // it and touches the two materials bound to it — by any slot.
    app.world_mut().resource_mut::<Pending>().0.push(image.id());
    app.update();
    let touched = modified(&mut app, &mut cursor);
    assert!(
        touched.contains(&bound.id()),
        "the material bound by albedo was not touched"
    );
    assert!(
        touched.contains(&bound_by_normal.id()),
        "the material bound by normal map was not touched"
    );
    assert!(
        !touched.contains(&unbound.id()),
        "a material bound to a different image was touched — the walk is not keyed on the chained set"
    );
    assert!(
        app.world().resource::<Chained>().0.is_empty(),
        "`Chained` was not cleared after the touch"
    );
    let chained = app.world().resource::<Assets<Image>>().get(&image).unwrap();
    assert!(
        chained.texture_descriptor.mip_level_count > 1,
        "drain did not chain the image"
    );

    // The next frame touches nothing: the record is one frame's.
    app.update();
    assert!(
        modified(&mut app, &mut cursor).is_empty(),
        "a material was touched again on a frame nothing was chained"
    );
}

#[test]
fn binds_reads_every_slot_the_standard_shader_samples() {
    let images = {
        let mut app = app();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        (0..6).map(|_| images.add(photo(4))).collect::<Vec<_>>()
    };
    let mat = StandardMaterial {
        base_color_texture: Some(images[0].clone()),
        normal_map_texture: Some(images[1].clone()),
        metallic_roughness_texture: Some(images[2].clone()),
        occlusion_texture: Some(images[3].clone()),
        emissive_texture: Some(images[4].clone()),
        depth_map: Some(images[5].clone()),
        ..default()
    };
    for (i, h) in images.iter().enumerate() {
        assert!(binds(&mat, h.id()), "slot {i} is not read by `binds`");
    }
    assert!(!binds(&StandardMaterial::default(), images[0].id()));
}
