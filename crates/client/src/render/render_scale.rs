//! Render the world below window resolution, then compose the UI at full size.
//!
//! The eye keeps its projection and post effects. Only its target changes.
//! The presentation camera draws no scene geometry and performs no second
//! tonemap; its background image is already the eye's finished frame. At 100%
//! the original direct-to-window path is restored, including its camera order.

use bevy::camera::{visibility::RenderLayers, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;

use super::{rig::EyeCam, Settings, WorldEntity};

/// Proposed defaults, `DECISIONS.md` §open, render scale v0. Percentage of
/// each dimension, not pixel count: 50% renders a quarter as many pixels.
pub const RENDER_SCALE_MIN: u8 = 50;
pub const RENDER_SCALE_STEP: u8 = 5;
/// Identity: every existing preset and settings file keeps its resolution.
pub const RENDER_SCALE_MAX: u8 = 100;

/// Physical pixels, including a high-DPI window's device scale. Zero-sized
/// windows are deferred by the caller; even a one-pixel viewport stays valid.
pub fn extent(window: UVec2, percent: u8) -> UVec2 {
    let percent = u64::from(percent.clamp(RENDER_SCALE_MIN, RENDER_SCALE_MAX));
    let dim = |n| ((u64::from(n) * percent / 100) as u32).max(1);
    UVec2::new(dim(window.x), dim(window.y))
}

/// The world image is a completed opaque frame. Native atmosphere preserves
/// the skybox's zero alpha between clouds; UI alpha blending must not turn
/// those already-lit pixels into holes in the presentation camera's clear.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct OpaqueFrame {
    #[texture(0)]
    #[sampler(1)]
    pub image: Handle<Image>,
}

impl UiMaterial for OpaqueFrame {
    fn fragment_shader() -> ShaderRef {
        "shaders/render_scale.wgsl".into()
    }
}

#[derive(Component)]
pub struct Presentation;

/// Lives with the eye, so leaving a world releases its image as well as the
/// two `WorldEntity` entities that present it. No persistent resource retains
/// a render target between joins.
#[derive(Component)]
pub struct ScaledSurface {
    pub image: Handle<Image>,
    pub camera: Entity,
    pub backdrop: Entity,
    original_target: RenderTarget,
    original_order: isize,
}

/// Before `CameraUpdateSystems`: a changed image size and newly spawned
/// camera must reach projection and UI layout in the same frame.
#[allow(clippy::type_complexity)]
pub fn apply(
    mut commands: Commands,
    settings: Res<Settings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut eyes: Query<
        (
            Entity,
            &mut Camera,
            &mut RenderTarget,
            Option<&ScaledSurface>,
        ),
        With<EyeCam>,
    >,
    mut images: ResMut<Assets<Image>>,
    mut frames: ResMut<Assets<OpaqueFrame>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let window_size = window.resolution.physical_size();
    if window_size.x == 0 || window_size.y == 0 {
        return;
    }
    let percent = settings
        .gfx
        .render_scale
        .clamp(RENDER_SCALE_MIN, RENDER_SCALE_MAX);
    for (eye, mut camera, mut target, surface) in &mut eyes {
        if percent == RENDER_SCALE_MAX {
            if let Some(surface) = surface {
                *target = surface.original_target.clone();
                camera.order = surface.original_order;
                commands.entity(surface.backdrop).despawn();
                commands.entity(surface.camera).despawn();
                commands.entity(eye).remove::<ScaledSurface>();
            }
            continue;
        }
        let size = extent(window_size, percent);
        if let Some(surface) = surface {
            // Reading through `get` first keeps settled frames from marking
            // the image changed and uploading it again.
            if images.get(&surface.image).is_some_and(|i| i.size() != size) {
                if let Some(image) = images.get_mut(&surface.image) {
                    image.resize(bevy::render::render_resource::Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    });
                }
            }
            continue;
        }

        let mut image =
            Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
        image.sampler = ImageSampler::linear();
        let image = images.add(image);
        let presentation = commands
            .spawn((
                WorldEntity,
                Presentation,
                Camera2d,
                Camera {
                    order: camera.order + 1,
                    ..default()
                },
                target.clone(),
                IsDefaultUiCamera,
                RenderLayers::none(),
                Msaa::Off,
                Tonemapping::None,
                DebandDither::Disabled,
            ))
            .id();
        let backdrop = commands
            .spawn((
                WorldEntity,
                Presentation,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                MaterialNode(frames.add(OpaqueFrame {
                    image: image.clone(),
                })),
                GlobalZIndex(i32::MIN),
                UiTargetCamera(presentation),
                FocusPolicy::Pass,
                Pickable::IGNORE,
            ))
            .id();
        commands.entity(eye).insert(ScaledSurface {
            image: image.clone(),
            camera: presentation,
            backdrop,
            original_target: target.clone(),
            original_order: camera.order,
        });
        *target = RenderTarget::Image(image.into());
    }
}
