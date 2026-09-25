//! The particle sprites: one 256² sheet generated at startup — white, the
//! shape in the alpha — so every sprite is tinted by its particle's colour.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::super::mipmap;
use super::super::props::hash01;

pub const SPRITE_COLS: u32 = 4;
pub const SPRITE_ROWS: u32 = 4;
/// One sprite's side, texels.
pub const SPRITE_TEX: u32 = 64;

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

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (xi, yi) = (xf as i32 as u32, yf as i32 as u32);
    let h = |a: u32, b: u32| hash01(a.wrapping_mul(0x9E37_79B1) ^ seed, b ^ seed.rotate_left(16));
    let (a, b) = (h(xi, yi), h(xi.wrapping_add(1), yi));
    let (c, d) = (
        h(xi, yi.wrapping_add(1)),
        h(xi.wrapping_add(1), yi.wrapping_add(1)),
    );
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sy
}

fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    vnoise(x, y, seed) * 0.55
        + vnoise(x * 2.1, y * 2.1, seed ^ 0x5A) * 0.3
        + vnoise(x * 4.3, y * 4.3, seed ^ 0xC3) * 0.15
}

/// Coverage of sprite `cell` at `(u, v) ∈ [-1, 1]²` (value, alpha).
fn sprite(cell: u8, u: f32, v: f32) -> (f32, f32) {
    let seed = 0xF00D_0000 ^ (cell as u32).wrapping_mul(0x0101_0101);
    let r = (u * u + v * v).sqrt();
    let (value, alpha) = match cell {
        // A glow: a hot core and a long soft skirt, so a spark reads as light
        // without a bloom pass.
        GLOW => {
            let core = smooth(0.35, 0.0, r);
            let skirt = (1.0 - r).max(0.0).powi(3);
            (1.0, (core * 0.9 + skirt * 0.6).min(1.0))
        }
        // A streak along u, soft across v, tapering at both ends.
        STREAK => {
            let across = smooth(1.0, 0.0, v.abs()).powi(2);
            let ends = smooth(1.0, 0.7, u.abs());
            (1.0, across * ends)
        }
        // Star flashes: 4–7 spikes of rolled length around a hot centre.
        2..=4 => {
            let a = v.atan2(u);
            let spikes = 5 + (cell as u32 - 2);
            let mut s: f32 = 0.0;
            for k in 0..spikes {
                let ang =
                    (k as f32 + 0.3 * hash01(k, seed)) / spikes as f32 * std::f32::consts::TAU;
                let len = 0.6 + 0.4 * hash01(k, seed ^ 0x11);
                let d = (a - ang + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                let w = 0.18 * (1.0 - r / len).max(0.0);
                s = s
                    .max(smooth(w, 0.0, d.abs() * r.max(0.05)) * (1.0 - smooth(len * 0.6, len, r)));
            }
            let core = smooth(0.3, 0.0, r);
            (1.0, s.max(core))
        }
        // Puffs: a ragged soft blob with some body in it.
        5..=7 => {
            let a = v.atan2(u);
            let edge = 0.75 + 0.25 * vnoise(a.cos() * 1.8 + 3.0, a.sin() * 1.8 + 3.0, seed);
            let body = smooth(edge, edge * 0.35, r);
            let n = fbm(u * 3.0, v * 3.0, seed);
            (0.8 + 0.2 * n, body * (0.55 + 0.45 * n))
        }
        // Smoke: softer and wispier than a puff.
        8 | 9 => {
            let n = fbm(u * 2.2 + 5.0, v * 2.2, seed);
            let body = smooth(1.0, 0.2, r + 0.25 * (n - 0.5));
            (0.85 + 0.15 * n, body * (0.35 + 0.65 * n))
        }
        // A droplet: round, a little brighter at its top.
        DROPLET => {
            let body = smooth(0.9, 0.6, r);
            let hi = smooth(0.45, 0.0, ((u + 0.25).powi(2) + (v + 0.3).powi(2)).sqrt());
            (0.8 + 0.2 * hi, body)
        }
        RING => (
            1.0,
            smooth(0.18, 0.0, (r - 0.72).abs()) * smooth(1.0, 0.9, r),
        ),
        SPECK => (1.0, smooth(0.85, 0.55, r)),
        // A leaf: an ellipse with a midrib.
        LEAF => {
            let e = (u * u + (v * 2.4).powi(2)).sqrt();
            let rib = 1.0 - 0.25 * smooth(0.06, 0.0, v.abs());
            (rib, smooth(0.95, 0.8, e))
        }
        // A splinter: a long thin sliver.
        SPLINTER => {
            let e = (u * u + (v * 5.0).powi(2)).sqrt();
            (1.0, smooth(0.95, 0.75, e))
        }
        _ => (0.0, 0.0),
    };
    // Nothing reaches the sprite's own edge.
    let pad = 1.0 - smooth(0.9, 0.98, u.abs().max(v.abs()));
    (value.clamp(0.0, 1.0), (alpha * pad).clamp(0.0, 1.0))
}

/// The sheet's level 0: RGBA8 sRGB.
pub fn sprite_pixels() -> (Vec<u8>, u32, u32) {
    let (w, h) = (SPRITE_COLS * SPRITE_TEX, SPRITE_ROWS * SPRITE_TEX);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for cell in 0..(SPRITE_COLS * SPRITE_ROWS) as u8 {
        let (cx, cy) = (cell as u32 % SPRITE_COLS, cell as u32 / SPRITE_COLS);
        for y in 0..SPRITE_TEX {
            for x in 0..SPRITE_TEX {
                let u = (x as f32 + 0.5) / SPRITE_TEX as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / SPRITE_TEX as f32 * 2.0 - 1.0;
                let (val, a) = sprite(cell, u, v);
                let i = (((cy * SPRITE_TEX + y) * w + cx * SPRITE_TEX + x) * 4) as usize;
                let g = (val * 255.0 + 0.5) as u8;
                data[i] = g;
                data[i + 1] = g;
                data[i + 2] = g;
                data[i + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
    }
    (data, w, h)
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
