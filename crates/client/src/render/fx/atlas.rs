//! The particle sprites: one 256² sheet generated at startup — white, the
//! shape in the alpha — so every sprite is tinted by its particle's colour.
//!
//! The shapes are the shared `fx` crate's (`fx::atlas`); this file is Gates'
//! layout of them ([`CELLS`]), its names for the cells, and the Bevy image.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::super::mipmap;

pub use ::fx::atlas::{SPRITE_COLS, SPRITE_ROWS, SPRITE_TEX};

/// The sprites, by cell.
pub const GLOW: u8 = 0;
pub const STREAK: u8 = 1;
/// Three star flashes, cells `STAR..STAR + 3`.
pub const STAR: u8 = 2;
/// Three puffs, cells `PUFF..PUFF + 3`.
pub const PUFF: u8 = 5;
/// Two smoke wisps.
pub const SMOKE: u8 = 8;
pub const DROPLET: u8 = 10;
pub const RING: u8 = 11;
pub const SPECK: u8 = 12;
pub const LEAF: u8 = 13;
pub const SPLINTER: u8 = 14;
/// A flame tongue, tip up — drawn unrolled, or the fire leans.
pub const FLAME: u8 = 15;

/// Gates' sheet: the shared crate's default layout, cell for cell.
pub const CELLS: [::fx::atlas::Shape; ::fx::atlas::CELLS] = ::fx::atlas::DEFAULT;

/// The sheet's level 0: RGBA8 sRGB.
pub fn sprite_pixels() -> (Vec<u8>, u32, u32) {
    ::fx::atlas::sheet(&CELLS)
}

/// The sheet with its mip chain.
pub fn sprite_image() -> Image {
    let (level0, w, h) = sprite_pixels();
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    if let Some(l0) = img.data.as_ref() {
        let chain = mipmap::chain(l0, w, h, mipmap::Filter::Srgb);
        img.texture_descriptor.mip_level_count = mipmap::levels(w, h);
        img.data = Some(chain);
    }
    img.sampler = ImageSampler::linear();
    img
}
