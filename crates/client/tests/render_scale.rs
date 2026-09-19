#![cfg(feature = "render")]

use bevy::asset::AssetPlugin;
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowResolution};
use client::render::render_scale::{apply, extent, Presentation, ScaledSurface};
use client::render::rig::EyeCam;
use client::render::settings::Knob;
use client::render::{Settings, WorldEntity};

fn app(scale: u8) -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<Settings>()
        .add_systems(PostUpdate, apply);
    app.world_mut().resource_mut::<Settings>().gfx.render_scale = scale;
    let window = app
        .world_mut()
        .spawn((
            PrimaryWindow,
            Window {
                resolution: WindowResolution::new(1920, 1080).with_scale_factor_override(2.0),
                ..default()
            },
        ))
        .id();
    let eye = app
        .world_mut()
        .spawn((WorldEntity, EyeCam, Camera3d::default()))
        .id();
    (app, window, eye)
}

#[test]
fn dimensions_are_physical_and_never_degenerate() {
    assert_eq!(extent(UVec2::new(1920, 1080), 50), UVec2::new(960, 540));
    assert_eq!(extent(UVec2::new(1920, 1080), 75), UVec2::new(1440, 810));
    assert_eq!(extent(UVec2::new(801, 601), 100), UVec2::new(801, 601));
    assert_eq!(extent(UVec2::ONE, 50), UVec2::ONE);
    assert_eq!(extent(UVec2::ZERO, 50), UVec2::ONE);
    assert_eq!(extent(UVec2::splat(u32::MAX), 100), UVec2::splat(u32::MAX));
}

#[test]
fn saved_scale_applies_on_join_and_ui_keeps_the_window() {
    let (mut app, window, eye) = app(75);
    app.update();
    let w = app.world();
    let surface = w.get::<ScaledSurface>(eye).unwrap();
    let image = w.resource::<Assets<Image>>().get(&surface.image).unwrap();
    assert_eq!(image.size(), UVec2::new(1440, 810));
    assert!(matches!(
        w.get::<RenderTarget>(eye),
        Some(RenderTarget::Image(_))
    ));
    assert!(matches!(
        w.get::<RenderTarget>(surface.camera),
        Some(RenderTarget::Window(_))
    ));
    assert!(w.get::<IsDefaultUiCamera>(surface.camera).is_some());
    assert_eq!(
        w.get::<Tonemapping>(surface.camera),
        Some(&Tonemapping::None)
    );
    assert!(w.get::<Camera>(surface.camera).unwrap().order > w.get::<Camera>(eye).unwrap().order);
    assert_eq!(
        w.get::<Window>(window).unwrap().resolution.scale_factor(),
        2.0
    );
    assert_eq!(
        w.get::<UiTargetCamera>(surface.backdrop).unwrap().entity(),
        surface.camera
    );
    assert_eq!(w.get::<GlobalZIndex>(surface.backdrop).unwrap().0, i32::MIN);
}

#[test]
fn resizing_reuses_the_target_and_full_resolution_restores_the_original_path() {
    let (mut app, window, eye) = app(50);
    app.world_mut().get_mut::<Camera>(eye).unwrap().order = -3;
    app.update();
    let s = app.world().get::<ScaledSurface>(eye).unwrap();
    let (image, presentation, backdrop) = (s.image.clone(), s.camera, s.backdrop);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set_physical_resolution(1280, 720);
    app.update();
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&image)
            .unwrap()
            .size(),
        UVec2::new(640, 360)
    );
    for _ in 0..8 {
        app.update();
        let s = app.world().get::<ScaledSurface>(eye).unwrap();
        assert_eq!(s.image, image);
        assert_eq!(s.camera, presentation);
        assert_eq!(s.backdrop, backdrop);
    }
    app.world_mut().resource_mut::<Settings>().gfx.render_scale = 100;
    app.update();
    assert!(app.world().get::<ScaledSurface>(eye).is_none());
    assert!(app.world().get_entity(presentation).is_err());
    assert!(app.world().get_entity(backdrop).is_err());
    assert!(matches!(
        app.world().get::<RenderTarget>(eye),
        Some(RenderTarget::Window(_))
    ));
    assert_eq!(app.world().get::<Camera>(eye).unwrap().order, -3);
}

#[test]
fn the_default_does_not_create_an_offscreen_pass_and_the_stepper_is_bounded() {
    let (mut app, _, eye) = app(100);
    app.update();
    assert!(app.world().get::<ScaledSurface>(eye).is_none());
    assert_eq!(
        app.world_mut()
            .query::<&Presentation>()
            .iter(app.world())
            .count(),
        0
    );
    let mut settings = app.world_mut().resource_mut::<Settings>();
    settings.adjust(Knob::RenderScale, -1);
    assert_eq!(settings.gfx.render_scale, 95);
    assert_eq!(settings.preset_name(), None);
    for _ in 0..100 {
        settings.adjust(Knob::RenderScale, -1);
    }
    assert_eq!(settings.gfx.render_scale, 50);
    for _ in 0..100 {
        settings.adjust(Knob::RenderScale, 1);
    }
    assert_eq!(settings.gfx.render_scale, 100);
}
