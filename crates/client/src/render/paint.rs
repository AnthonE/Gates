//! **Painted skins** (wire v110): a skin that is a picture rather than a
//! tint. The shard's skin catalog names it by the first 8 bytes of its PNG's
//! sha256 (`SkinRow::texture`); this file fetches that PNG from the platform
//! (`/api/items/skin/<hex16>.png`), checks the bytes hash to the name it was
//! asked for, decodes it, builds its mip chain, and hands the image on:
//!
//! - to [`super::viewmodel::SkinMats`], which paints a worn piece with it
//!   (the tunic's UVs are its canvas, `render::canvas`);
//! - to [`super::viewmodel::Models`] as a mesh and material of their own for
//!   a held canvas row (the bat, drawn on its tint-free canvas mesh), so
//!   every hand and the ground draw it the way they draw a skin with its own
//!   model.
//!
//! A new painted skin is a catalog row and a file on the platform: no client
//! release. Fetched like `faces::Pics` — a thread and `ureq` natively,
//! `fetch` in a page — and a failure is remembered, so the skin falls back
//! to its tint (its picture's average colour) rather than asking forever.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sha2::{Digest, Sha256};

use super::mipmap;
use super::viewmodel::{Look, Models, SkinMats};
use super::Net;

/// Refused past this many bytes: the platform's cap on a painted skin.
pub const PAINT_MAX_BYTES: usize = 1024 * 1024;
/// A painted skin is a square power of two between these sides.
pub const PAINT_MIN_SIDE: u32 = 256;
pub const PAINT_MAX_SIDE: u32 = 1024;
/// Fetches out at once, and pictures kept at most.
const IN_FLIGHT: usize = 2;
const KEPT: usize = 64;

enum State {
    Pending,
    Ready(Handle<Image>),
    Failed,
}

type Landed = ([u8; 8], Option<(u32, Vec<u8>)>);
type Tx = tokio::sync::mpsc::UnboundedSender<Landed>;

/// The painted pictures. See the module header.
#[derive(Resource)]
pub struct Paints {
    rows: Vec<([u8; 8], State)>,
    in_flight: usize,
    tx: Tx,
    rx: tokio::sync::mpsc::UnboundedReceiver<Landed>,
    /// A held canvas row's tint-free mesh (`heldgen::canvas_mesh`), built
    /// once per row.
    canvas: Vec<(usize, Handle<Mesh>)>,
}

impl Default for Paints {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            rows: Vec::new(),
            in_flight: 0,
            tx,
            rx,
            canvas: Vec::new(),
        }
    }
}

/// What the platform serves a painted skin at.
pub fn url(origin: &str, hash: [u8; 8]) -> String {
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("{origin}/api/items/skin/{hex}.png")
}

impl Paints {
    /// The picture named `hash`: `Ok(Some)` once it has landed, `Ok(None)`
    /// while it is on its way, `Err(())` if it never will.
    fn want(&mut self, origin: &str, hash: [u8; 8]) -> Result<Option<Handle<Image>>, ()> {
        if let Some((_, state)) = self.rows.iter().find(|(h, _)| *h == hash) {
            return match state {
                State::Ready(img) => Ok(Some(img.clone())),
                State::Pending => Ok(None),
                State::Failed => Err(()),
            };
        }
        if self.rows.len() >= KEPT {
            return Err(());
        }
        if self.in_flight >= IN_FLIGHT {
            return Ok(None); // asked again next frame
        }
        self.rows.push((hash, State::Pending));
        self.in_flight += 1;
        fetch(url(origin, hash), hash, self.tx.clone());
        Ok(None)
    }

    /// Turn landed fetches into images: level 0 and its mip chain, wrapped
    /// around (a canvas's U is one turn).
    fn pump(&mut self, images: &mut Assets<Image>) {
        while let Ok((hash, landed)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(row) = self.rows.iter_mut().find(|(h, _)| *h == hash) else {
                continue;
            };
            row.1 = match landed {
                Some((side, level0)) => State::Ready(images.add(image(side, level0))),
                None => State::Failed,
            };
        }
    }
}

/// A painted skin's level 0 as the image the game draws: its mip chain,
/// sRGB, wrapped around in U. Public for the benches (`examples/worn_look`).
pub fn image(side: u32, level0: Vec<u8>) -> Image {
    let data = mipmap::chain(&level0, side, side, mipmap::Filter::Srgb);
    let mut img = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // The count and the buffer together (`mipmap::drain`'s warning).
    img.texture_descriptor.mip_level_count = mipmap::levels(side, side);
    img.data = Some(data);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    img
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(url: String, hash: [u8; 8], tx: Tx) {
    std::thread::spawn(move || {
        let px = super::faces::get_bytes(&url, PAINT_MAX_BYTES).and_then(|b| decode(&b, hash));
        let _ = tx.send((hash, px));
    });
}

#[cfg(target_arch = "wasm32")]
fn fetch(url: String, hash: [u8; 8], tx: Tx) {
    wasm_bindgen_futures::spawn_local(async move {
        let px = crate::net::web::fetch_bytes(&url, PAINT_MAX_BYTES)
            .await
            .and_then(|b| decode(&b, hash));
        let _ = tx.send((hash, px));
    });
}

/// A painted skin's PNG → (side, RGBA8 level 0). Refused unless its bytes
/// hash to the name it was fetched by (a platform cannot hand this skin a
/// different picture), and unless [`pixels`] takes it.
pub fn decode(bytes: &[u8], hash: [u8; 8]) -> Option<(u32, Vec<u8>)> {
    if Sha256::digest(bytes)[..8] != hash {
        return None;
    }
    pixels(bytes).ok()
}

/// A painted skin's PNG → (side, opaque RGBA8), or why not: a square power
/// of two from [`PAINT_MIN_SIDE`] to [`PAINT_MAX_SIDE`], at most
/// [`PAINT_MAX_BYTES`], 8-bit grey or RGB(A). `skinkit check` says the same
/// words to a painter.
pub fn pixels(bytes: &[u8]) -> Result<(u32, Vec<u8>), String> {
    if bytes.len() > PAINT_MAX_BYTES {
        return Err(format!("over {PAINT_MAX_BYTES} bytes"));
    }
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| format!("not a PNG: {e}"))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w != h || !w.is_power_of_two() || !(PAINT_MIN_SIDE..=PAINT_MAX_SIDE).contains(&w) {
        return Err(format!(
            "{w}x{h}: a painted skin is square, a power of two, {PAINT_MIN_SIDE} to {PAINT_MAX_SIDE} px"
        ));
    }
    let size = reader.output_buffer_size().ok_or("too large")?;
    let mut buf = vec![0; size];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("unreadable: {e}"))?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("indexed colour: save it as RGB".into()),
    };
    let side = w as usize;
    let mut out = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        let line = &buf[y * frame.line_size..];
        for x in 0..side {
            let p = &line[x * channels..x * channels + channels];
            // Opaque: a skin is a surface, not a cut-out.
            out.extend_from_slice(&match channels {
                1 | 2 => [p[0], p[0], p[0], 255],
                _ => [p[0], p[1], p[2], 255],
            });
        }
    }
    Ok((w, out))
}

/// Every frame: learn the catalog's looks, ask for each painted skin's
/// picture, and once one lands give its holders what they draw.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    net: Option<NonSend<Net>>,
    pics: Option<Res<super::faces::Pics>>,
    mut paints: ResMut<Paints>,
    mut skin_mats: ResMut<SkinMats>,
    mut models: ResMut<Models>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    paints.pump(&mut images);
    let Some(n) = net.as_deref() else {
        return;
    };
    let core = &n.session.core;
    let origin = pics
        .as_deref()
        .map_or(crate::ui::names::PLATFORM_ORIGIN, |p| p.origin.as_str());
    for row in core.skins.rows() {
        if !row.painted() {
            skin_mats.learn(row.catalog, Look::Tint);
            continue;
        }
        let img = match paints.want(origin, row.texture) {
            Ok(Some(img)) => img,
            Ok(None) => {
                skin_mats.learn(row.catalog, Look::Waiting);
                continue;
            }
            Err(()) => {
                skin_mats.learn(row.catalog, Look::Failed);
                continue;
            }
        };
        // A held canvas row (the bat): its tint-free generated mesh and that
        // mesh's own material with the picture as its colour, registered
        // once. Not the row's material: the row draws a file whose normal and
        // roughness maps are laid out for its own UVs, not the canvas's.
        let name = core::str::from_utf8(core.catalog.name(row.covers as usize)).unwrap_or("");
        let stem = crate::ui::icons::stem(name);
        if let Some(r) = crate::ui::hold::HELD_MODELS
            .iter()
            .position(|m| m.key == stem)
        {
            if !models.painted(r, row.catalog) {
                let mesh = match paints.canvas.iter().find(|(i, _)| *i == r) {
                    Some((_, m)) => Some(m.clone()),
                    None => super::heldgen::canvas_mesh(&stem).map(|m| {
                        let h = meshes.add(m);
                        paints.canvas.push((r, h.clone()));
                        h
                    }),
                };
                if let Some(mesh) = mesh {
                    let base = super::heldgen::material(&stem);
                    let mat = materials.add(painted(&base, img.clone()));
                    models.paint(r, row.catalog, mesh, mat);
                }
            }
        }
        skin_mats.learn(row.catalog, Look::Ready(img));
    }
}

/// `base` with `img` as the whole of its colour.
pub fn painted(base: &StandardMaterial, img: Handle<Image>) -> StandardMaterial {
    let mut m = base.clone();
    m.base_color = Color::WHITE;
    m.base_color_texture = Some(img);
    m
}

pub fn register(app: &mut App) {
    app.init_resource::<Paints>().add_systems(Update, paint);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(side: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, side, side);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            let px: Vec<u8> = (0..side * side).flat_map(|_| rgb).collect();
            w.write_image_data(&px).unwrap();
        }
        out
    }

    fn name(bytes: &[u8]) -> [u8; 8] {
        let d = Sha256::digest(bytes);
        let mut h = [0u8; 8];
        h.copy_from_slice(&d[..8]);
        h
    }

    #[test]
    fn a_painted_skin_is_its_named_bytes_and_a_square_power_of_two() {
        let ok = png(256, [200, 20, 20]);
        let (side, px) = decode(&ok, name(&ok)).expect("a 256 px square");
        assert_eq!((side, px.len()), (256, 256 * 256 * 4));
        assert_eq!(&px[..4], &[200, 20, 20, 255]);
        // Some other picture under its name: refused.
        let other = png(256, [0, 0, 0]);
        assert!(decode(&other, name(&ok)).is_none(), "the hash is the check");
        for bad in [
            png(128, [1, 2, 3]),
            png(300, [1, 2, 3]),
            png(2048, [1, 2, 3]),
        ] {
            assert!(decode(&bad, name(&bad)).is_none(), "size refused");
        }
        assert_eq!(
            url("https://elopros.com", [0xab, 0xcd, 0, 1, 2, 3, 4, 0xff]),
            "https://elopros.com/api/items/skin/abcd0001020304ff.png"
        );
    }
}
