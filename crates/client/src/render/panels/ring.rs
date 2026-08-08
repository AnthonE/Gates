//! The build wheel's rings, drawn as textures.
//!
//! **Why a texture and not nodes.** A wedge is not a rectangle, and Bevy UI
//! has no arc. The old wheel dodged that by drawing a flat dark disc with
//! rounded label boxes floating on it, which is why it read as programmer
//! art: the reference's wheel is a **cream annulus cut into wedges**, and
//! nothing rectangular approximates that. Rasterising the annulus once is the
//! whole fix, and it costs a texture.
//!
//! **Baked at plugin-build time, never per frame.** Ten images total — one
//! base ring pair, one highlight per shape segment, one per material segment
//! — generated beside `ui::build_fonts` and `audio::build_bank`. The wheel
//! rebuilds whenever the pointer crosses a segment boundary, which while
//! sweeping is several times a second; generating a megabyte there would be
//! an allocation on a hot path for a thing that has ten possible states. So
//! the ten are made once and a rebuild only picks handles.
//!
//! **Geometry comes from [`Rings`]**, the same struct `ui::build::pick`
//! resolves the pointer with, so what is drawn and what is picked cannot
//! disagree — the wheel's oldest rule, kept.
//!
//! Colours are measured off `Rust Images/building.jpeg` by locating the ring
//! programmatically (bright, low-saturation pixels; centre and radii from
//! their distribution) rather than by eye: the band is `#f5e7dd` and the
//! chosen wedge is `#d13f28`.

use bevy::asset::{uuid_handle, RenderAssetUsages};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::ui::build::{Rings, MATERIALS, SHAPES};

/// The cream of the ring band, `#f5e7dd`.
pub const CREAM: [u8; 3] = [245, 231, 221];
/// The chosen wedge, `#d13f28`.
pub const CHOSEN: [u8; 3] = [209, 63, 40];
/// How opaque the ring is over the world. The reference's sits over a
/// *blurred* world, which we do not have; a touch of translucency keeps it
/// from reading as a sticker without costing legibility.
const RING_ALPHA: u8 = 242;

/// Texture edge, px. The wheel is drawn at `2 * rim` ≈ 428 px, so 512 gives
/// a little better than 1:1 and the wedge edges stay clean when a display
/// scales it.
const TEX: u32 = 512;

/// Half the angular gap between two wedges, radians. Small — the reference's
/// dividers are hairlines, not gutters.
const GAP: f32 = 0.010;

/// Supersampling per axis. Wedge edges are radial lines at every angle, so
/// they alias badly at 1 spp; 3×3 is cheap once and clean.
const SS: u32 = 3;

pub const BASE: Handle<Image> = uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e0");

/// One highlight per shape segment, then one per material segment. Indexed
/// the same way `SHAPES` and `MATERIALS` are, because the caller has an index
/// and nothing else.
pub const SHAPE_HI: [Handle<Image>; 6] = [
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e1"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e2"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e3"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e4"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e5"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5e6"),
];

pub const MAT_HI: [Handle<Image>; 3] = [
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5f1"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5f2"),
    uuid_handle!("7a1c4e20-91b3-4f6a-8c2d-11a0b3c4d5f3"),
];

/// Which band a pixel is in, in texture units where `rim` maps to `TEX/2`.
#[derive(Clone, Copy)]
struct Band {
    inner: f32,
    outer: f32,
    segments: usize,
}

/// Is `(r, theta)` inside this band's segment `seg`? `theta` is 0 at straight
/// up and grows clockwise — [`crate::ui::build::pick`]'s convention, so the
/// arithmetic here is the arithmetic the pointer is resolved with.
fn covers(b: Band, r: f32, theta: f32, seg: Option<usize>) -> bool {
    if r < b.inner || r > b.outer {
        return false;
    }
    let step = std::f32::consts::TAU / b.segments as f32;
    // Segment i is centred on i*step, so its span is ±step/2 around that.
    let shifted = (theta + step * 0.5).rem_euclid(std::f32::consts::TAU);
    let idx = (shifted / step) as usize % b.segments;
    // The gap is cut at both edges of every wedge, whether or not this call
    // is drawing that wedge — otherwise the base ring and a highlight would
    // disagree about where the divider is.
    let within = shifted - idx as f32 * step;
    if within < GAP || within > step - GAP {
        return false;
    }
    match seg {
        None => true,
        Some(s) => idx == s,
    }
}

/// Rasterise the wheel. `pick` selects which band and segment is drawn; `None`
/// draws both bands whole, which is the base.
fn bake(rings: Rings, pick: Option<(bool, usize)>, rgb: [u8; 3]) -> Image {
    let n = TEX as usize;
    let mut data = vec![0u8; n * n * 4];
    let half = TEX as f32 * 0.5;
    // Texture units per wheel pixel: the texture spans the rim's diameter.
    let scale = rings.rim / half;

    let shape_band = Band {
        inner: rings.split,
        outer: rings.rim,
        segments: SHAPES.len(),
    };
    let mat_band = Band {
        inner: rings.dead,
        outer: rings.split,
        segments: MATERIALS.len(),
    };

    for py in 0..n {
        for px in 0..n {
            let mut hits = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32 - half) * scale;
                    // Texture y grows downward; the pick convention is y-up,
                    // and this is the single place the flip happens.
                    let y = (half - (py as f32 + (sy as f32 + 0.5) / SS as f32)) * scale;
                    let r = (x * x + y * y).sqrt();
                    let theta = x.atan2(y).rem_euclid(std::f32::consts::TAU);
                    let inside = match pick {
                        None => {
                            covers(shape_band, r, theta, None) || covers(mat_band, r, theta, None)
                        }
                        Some((is_shape, seg)) => {
                            let b = if is_shape { shape_band } else { mat_band };
                            covers(b, r, theta, Some(seg))
                        }
                    };
                    if inside {
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                continue;
            }
            let cover = hits as f32 / (SS * SS) as f32;
            let o = (py * n + px) * 4;
            data[o] = rgb[0];
            data[o + 1] = rgb[1];
            data[o + 2] = rgb[2];
            data[o + 3] = (RING_ALPHA as f32 * cover) as u8;
        }
    }

    Image::new(
        Extent3d {
            width: TEX,
            height: TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        // sRGB: these are colours off a reference frame, not data.
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Bake all ten, at plugin-build time.
pub fn build_rings(app: &mut App) {
    let rings = Rings::default();
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();

    images
        .insert(BASE.id(), bake(rings, None, CREAM))
        .expect("the wheel's base ring");
    for (i, h) in SHAPE_HI.iter().enumerate() {
        images
            .insert(h.id(), bake(rings, Some((true, i)), CHOSEN))
            .expect("a shape highlight");
    }
    for (i, h) in MAT_HI.iter().enumerate() {
        images
            .insert(h.id(), bake(rings, Some((false, i)), CHOSEN))
            .expect("a material highlight");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(segments: usize) -> Band {
        Band {
            inner: 100.0,
            outer: 200.0,
            segments,
        }
    }

    #[test]
    fn segment_zero_is_straight_up() {
        // Straight up is theta 0, and it must land in segment 0 — the same
        // claim `tests/ui.rs` §D makes about `pick`. If these two ever
        // disagree the wheel highlights one wedge and selects another.
        assert!(covers(band(6), 150.0, 0.0, Some(0)));
        assert!(!covers(band(6), 150.0, 0.0, Some(1)));
    }

    #[test]
    fn indices_grow_clockwise() {
        let step = std::f32::consts::TAU / 6.0;
        for i in 0..6usize {
            assert!(
                covers(band(6), 150.0, step * i as f32, Some(i)),
                "segment {i} did not cover its own centre angle"
            );
        }
    }

    #[test]
    fn the_bands_bound_the_radius() {
        assert!(!covers(band(6), 99.0, 0.0, None), "inside the inner edge");
        assert!(!covers(band(6), 201.0, 0.0, None), "outside the rim");
        assert!(covers(band(6), 150.0, 0.0, None));
    }

    #[test]
    fn a_divider_is_cut_at_every_boundary() {
        let step = std::f32::consts::TAU / 6.0;
        // Just inside the boundary between segment 0 and segment 1.
        let edge = step * 0.5;
        assert!(!covers(band(6), 150.0, edge - GAP * 0.5, None));
        assert!(!covers(band(6), 150.0, edge + GAP * 0.5, None));
    }
}
