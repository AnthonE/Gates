//! The web asset variant: a model's KTX2 textures re-embedded as PNG, so a
//! browser can draw it (`NOW.md` §0web item 6).
//!
//! ## Why a variant and not a loader
//!
//! Every model in `assets/models/` embeds its maps as KTX2/UASTC
//! (`ci/ktx_pack.py`, and `assets/models/MANIFEST.md` for why: VRAM, not
//! disk). Bevy decodes UASTC through `basis-universal`, which is C++ and
//! does not build for `wasm32-unknown-unknown` — so `client-web` is built
//! without it (`findings/web-build-20260909.md` §15), and all 47 glTF loads
//! in a browser fail as `format requires transcoding: Uastc(Rgb)`. There is
//! no pure-Rust UASTC transcoder to reach for, and a JavaScript one in the
//! page would be a second decoder for one format, which is the shape this
//! repo refuses on principle.
//!
//! So the conversion happens where the transcoder exists — natively, at
//! build time — and the browser is handed models whose images are a format
//! it has always been able to read. `ci/build_web.sh` runs [`convert_dir`]
//! over the STAGED copy of `assets/models/`, in place, after staging; the
//! tree's own files are untouched and every gate over them
//! (`tests/prop_assets.rs`, `tests/packed_maps.rs`) still reads KTX2. The
//! client needs no path seam: the page fetches `models/prop/rock_a.glb` and
//! gets the variant, because the variant has the same name.
//!
//! ## The level, and the bytes
//!
//! **Level 1 of the chain, not level 0** — 512², where the KTX2 was packed
//! at 1024². Two reasons, and the first is the one that binds: a PNG lands
//! in VRAM as raw RGBA8, 4 MB per 1024² map, 16 MB per four-map model and
//! ~400 MB across the pool, on a target whose tier is `Low` and whose
//! surface is capped at 2048 pixels a side. 512² is a quarter of that. The
//! second is the download: a 512² PNG is roughly a third of the 1024² UASTC
//! it replaces, so the staged variant is smaller than the tree's models,
//! not larger. Level 1 is read straight off the container rather than
//! resampled from level 0 — the packer already built the chain, and a
//! second box filter is a second opinion about the same texels.
//!
//! What the browser loses, said out loud: the chain. `bevy_image` builds no
//! mips for a PNG, so `render/mipmap.rs` covers `models/` as well as
//! `textures/` now — a KTX2 image is skipped by the level count it already
//! carries, so the desktop is unchanged, and a PNG one gets the same
//! derived chain a photograph gets.
//!
//! ## PNG only where there is alpha
//!
//! A map with no alpha in it — most albedos, every normal and packed
//! roughness map — is written as JPEG instead ([`encode_web`]). Measured
//! 2026-10-21 over the 153 models: 493 of 494 maps are opaque, and they went
//! 100.1 MB as PNG → 27.5 MB, the staged models 138.4 → 65.7 MB, at a median
//! PSNR of 39 dB (43 dB on the normals, 32 dB at worst, a carcass's noise).
//! The models are most of what a page downloads: 99 GLBs are asked for at
//! `Startup`. 4:4:4, because a normal or a
//! roughness map keeps data in its colour channels and chroma subsampling
//! would blur one channel into the next; quality [`JPEG_QUALITY`], and
//! [`JPEG_QUALITY_NORMAL`] for a normal map, whose error is a lighting error.
//! A map with any alpha below 255 stays PNG, which is lossless where a cut-out
//! edge is decided.
//!
//! ## What this does to the file
//!
//! A GLB is a header, a JSON chunk and one binary chunk; images point at
//! `bufferViews` into that chunk. [`rewrite`] walks every view in order,
//! copies each one it does not own byte for byte into a new chunk at a
//! 4-aligned offset, and replaces an image view's bytes with the PNG. Every
//! accessor addresses its view relatively, so nothing else in the document
//! moves. Only `image/ktx2` images are touched; a model that already embeds
//! PNG or JPEG passes through with its views re-packed and otherwise
//! identical, and a `uri`-referenced image is left alone.

use std::io::Read;

/// The mip level of the packed chain the variant is cut at: 1 is 512² for a
/// chain packed at 1024 (`ci/ktx_pack.py`). Level 0 would be the same VRAM
/// per model the packer measured the KTX2 route against.
pub const WEB_LEVEL: u32 = 1;

/// JPEG quality for an opaque map that is not a normal map.
pub const JPEG_QUALITY: u8 = 90;
/// JPEG quality for a normal map: its error bends the lighting.
pub const JPEG_QUALITY_NORMAL: u8 = 95;

/// Why a file could not be converted.
#[derive(Debug)]
pub enum Error {
    /// Not a GLB, or a GLB this reader does not understand.
    Malformed(&'static str),
    /// The document is not the JSON it claims to be.
    Json(serde_json::Error),
    /// A KTX2 image could not be decoded.
    Decode(String),
    /// The PNG encoder refused.
    Png(String),
    /// The JPEG encoder refused.
    Jpeg(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Malformed(why) => write!(f, "malformed GLB: {why}"),
            Error::Json(e) => write!(f, "GLB JSON: {e}"),
            Error::Decode(why) => write!(f, "KTX2 decode: {why}"),
            Error::Png(why) => write!(f, "PNG encode: {why}"),
            Error::Jpeg(why) => write!(f, "JPEG encode: {why}"),
        }
    }
}

const MAGIC: u32 = 0x4654_6C67; // "glTF"
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

fn u32_at(d: &[u8], at: usize) -> Result<u32, Error> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(Error::Malformed("truncated"))
}

/// Split a GLB into its JSON document and its binary chunk.
pub fn split(glb: &[u8]) -> Result<(serde_json::Value, Vec<u8>), Error> {
    if u32_at(glb, 0)? != MAGIC {
        return Err(Error::Malformed("not a GLB"));
    }
    if u32_at(glb, 4)? != 2 {
        return Err(Error::Malformed("not GLB version 2"));
    }
    let mut at = 12;
    let mut json = None;
    let mut bin = Vec::new();
    while at + 8 <= glb.len() {
        let len = u32_at(glb, at)? as usize;
        let kind = u32_at(glb, at + 4)?;
        let body = glb
            .get(at + 8..at + 8 + len)
            .ok_or(Error::Malformed("chunk runs past the end"))?;
        match kind {
            CHUNK_JSON => json = Some(serde_json::from_slice(body).map_err(Error::Json)?),
            CHUNK_BIN => bin = body.to_vec(),
            _ => {}
        }
        at += 8 + len;
    }
    Ok((json.ok_or(Error::Malformed("no JSON chunk"))?, bin))
}

/// Assemble a GLB from a document and a binary chunk.
pub fn join(json: &serde_json::Value, bin: &[u8]) -> Vec<u8> {
    let mut doc = serde_json::to_vec(json).expect("a Value serializes");
    while doc.len() % 4 != 0 {
        doc.push(b' ');
    }
    let mut body = bin.to_vec();
    while body.len() % 4 != 0 {
        body.push(0);
    }
    let total = 12 + 8 + doc.len() + 8 + body.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(doc.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&doc);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Rewrite a GLB so every `image/ktx2` image holds what `encode` makes of
/// its bytes instead: `(bytes, mime type)`, given the image's index and its
/// KTX2. Returns the new GLB and how many images were converted.
pub fn rewrite(
    glb: &[u8],
    mut encode: impl FnMut(usize, &[u8]) -> Result<(Vec<u8>, &'static str), Error>,
) -> Result<(Vec<u8>, usize), Error> {
    let (mut json, bin) = split(glb)?;
    // Which bufferView each KTX2 image owns.
    let mut image_views = Vec::new();
    if let Some(images) = json.get("images").and_then(|v| v.as_array()) {
        for (i, im) in images.iter().enumerate() {
            let ktx = im.get("mimeType").and_then(|m| m.as_str()) == Some("image/ktx2");
            if let (true, Some(view)) = (ktx, im.get("bufferView").and_then(|v| v.as_u64())) {
                image_views.push((i, view as usize));
            }
        }
    }
    let views = json
        .get("bufferViews")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut new_bin = Vec::with_capacity(bin.len());
    let mut new_views = Vec::with_capacity(views.len());
    let mut mimes = Vec::with_capacity(image_views.len());
    let mut converted = 0;
    for (vi, view) in views.iter().enumerate() {
        let offset = view.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let length =
            view.get("byteLength")
                .and_then(|v| v.as_u64())
                .ok_or(Error::Malformed("bufferView without byteLength"))? as usize;
        let bytes = bin
            .get(offset..offset + length)
            .ok_or(Error::Malformed("bufferView runs past the binary chunk"))?;
        let replaced;
        let bytes: &[u8] = if let Some(&(image, _)) = image_views.iter().find(|(_, v)| *v == vi) {
            let (out, mime) = encode(image, bytes)?;
            replaced = out;
            mimes.push((image, mime));
            converted += 1;
            &replaced
        } else {
            bytes
        };
        while new_bin.len() % 4 != 0 {
            new_bin.push(0);
        }
        let mut nv = view.clone();
        nv["byteOffset"] = serde_json::Value::from(new_bin.len());
        nv["byteLength"] = serde_json::Value::from(bytes.len());
        new_bin.extend_from_slice(bytes);
        new_views.push(nv);
    }
    if let Some(images) = json.get_mut("images").and_then(|v| v.as_array_mut()) {
        for (i, mime) in &mimes {
            images[*i]["mimeType"] = serde_json::Value::from(*mime);
        }
    }
    // The chunk `join` writes is padded to four bytes; the buffer's declared
    // length is that chunk, exactly, rather than three bytes short of it.
    while new_bin.len() % 4 != 0 {
        new_bin.push(0);
    }
    json["bufferViews"] = serde_json::Value::Array(new_views);
    if let Some(buffers) = json.get_mut("buffers").and_then(|v| v.as_array_mut()) {
        if let Some(b0) = buffers.first_mut() {
            b0["byteLength"] = serde_json::Value::from(new_bin.len());
        }
    }
    Ok((join(&json, &new_bin), converted))
}

/// Decode one level of a KTX2/UASTC container to RGBA8: `(w, h, rgba)`.
///
/// The same route `tests/packed_maps.rs` takes and Bevy takes — container →
/// zstd → UASTC transcode — with the row pitch in PIXELS for the
/// uncompressed target, because `basis-universal 0.3.1`'s safe wrapper
/// computes it in blocks and segfaults (`CLAUDE.md`'s packer entry).
#[cfg(not(target_arch = "wasm32"))]
pub fn decode_level(ktx: &[u8], level: u32) -> Result<(u32, u32, Vec<u8>), Error> {
    use basis_universal::{sys, DecodeFlags, TranscoderBlockFormat};
    let reader = ktx2::Reader::new(ktx).map_err(|e| Error::Decode(format!("{e:?}")))?;
    let h = reader.header();
    let want = level.min(h.level_count.saturating_sub(1));
    let lv = reader
        .levels()
        .nth(want as usize)
        .ok_or_else(|| Error::Decode(format!("no level {want}")))?;
    let raw: Vec<u8> = match h.supercompression_scheme {
        Some(ktx2::SupercompressionScheme::Zstandard) => {
            let mut cursor = std::io::Cursor::new(lv.data);
            let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut cursor)
                .map_err(|e| Error::Decode(format!("zstd: {e}")))?;
            let mut out = Vec::new();
            dec.read_to_end(&mut out)
                .map_err(|e| Error::Decode(format!("zstd: {e}")))?;
            out
        }
        None => lv.data.to_vec(),
        other => return Err(Error::Decode(format!("supercompression {other:?}"))),
    };
    let (w, hg) = (
        (h.pixel_width >> want).max(1),
        (h.pixel_height >> want).max(1),
    );
    let (bx, by) = (w.div_ceil(4), hg.div_ceil(4));
    if raw.len() != (bx * by * 16) as usize {
        return Err(Error::Decode(format!(
            "level {want} is {} bytes, not {bx}x{by} UASTC blocks",
            raw.len()
        )));
    }
    basis_universal::transcoder_init();
    let t = unsafe { sys::low_level_uastc_transcoder_new() };
    let mut rgba = vec![0u8; (w * hg * 4) as usize];
    let ok = unsafe {
        sys::low_level_uastc_transcoder_transcode_slice(
            t,
            rgba.as_mut_ptr() as _,
            bx,
            by,
            raw.as_ptr(),
            raw.len() as u32,
            TranscoderBlockFormat::RGBA32.into(),
            4,
            false,
            true,
            w,
            hg,
            w,
            std::ptr::null_mut(),
            hg,
            0,
            3,
            DecodeFlags::HIGH_QUALITY.bits(),
        )
    };
    unsafe { sys::low_level_uastc_transcoder_delete(t) };
    if !ok {
        return Err(Error::Decode("UASTC transcode failed".into()));
    }
    Ok((w, hg, rgba))
}

/// RGBA8 to PNG bytes.
pub fn png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| Error::Png(e.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| Error::Png(e.to_string()))?;
    }
    Ok(out)
}

/// RGBA8 with its alpha dropped to a 4:4:4 JPEG at `quality`.
pub fn jpeg(w: u32, h: u32, rgba: &[u8], quality: u8) -> Result<Vec<u8>, Error> {
    let (w, h) = (
        u16::try_from(w).map_err(|_| Error::Jpeg("wider than 65535".into()))?,
        u16::try_from(h).map_err(|_| Error::Jpeg("taller than 65535".into()))?,
    );
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    enc.set_optimized_huffman_tables(true);
    enc.encode(rgba, w, h, jpeg_encoder::ColorType::Rgba)
        .map_err(|e| Error::Jpeg(e.to_string()))?;
    Ok(out)
}

/// One decoded map as the browser gets it: PNG where any texel has alpha
/// below 255, JPEG otherwise (the module header has why), as `(bytes, mime)`.
pub fn encode_web(
    w: u32,
    h: u32,
    rgba: &[u8],
    normal: bool,
) -> Result<(Vec<u8>, &'static str), Error> {
    if rgba.chunks_exact(4).any(|t| t[3] != 255) {
        return Ok((png(w, h, rgba)?, "image/png"));
    }
    let q = if normal {
        JPEG_QUALITY_NORMAL
    } else {
        JPEG_QUALITY
    };
    Ok((jpeg(w, h, rgba, q)?, "image/jpeg"))
}

/// The images a GLB's materials use as normal maps, by image index —
/// through `KHR_texture_basisu` where the texture names its KTX2 there.
pub fn normal_images(json: &serde_json::Value) -> Vec<usize> {
    let textures = json.get("textures").and_then(|v| v.as_array());
    let image_of = |t: u64| -> Option<usize> {
        let tex = textures?.get(t as usize)?;
        tex.pointer("/extensions/KHR_texture_basisu/source")
            .or_else(|| tex.get("source"))
            .and_then(|v| v.as_u64())
            .map(|i| i as usize)
    };
    json.get("materials")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.pointer("/normalTexture/index").and_then(|v| v.as_u64()))
        .filter_map(image_of)
        .collect()
}

/// Convert one GLB's KTX2 images at [`WEB_LEVEL`], each to the format
/// [`encode_web`] picks for it.
#[cfg(not(target_arch = "wasm32"))]
pub fn convert(glb: &[u8]) -> Result<(Vec<u8>, usize), Error> {
    let normals = normal_images(&split(glb)?.0);
    rewrite(glb, |image, ktx| {
        let (w, h, rgba) = decode_level(ktx, WEB_LEVEL)?;
        encode_web(w, h, &rgba, normals.contains(&image))
    })
}

/// Convert every `.glb` under `dir`, in place. Returns `(files, images)`
/// converted and the bytes before and after.
#[cfg(not(target_arch = "wasm32"))]
pub fn convert_dir(dir: &std::path::Path) -> Result<(usize, usize, u64, u64), String> {
    let mut files = 0;
    let mut images = 0;
    let (mut before, mut after) = (0u64, 0u64);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("glb") {
                continue;
            }
            let glb = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let (out, n) = convert(&glb).map_err(|e| format!("{}: {e}", p.display()))?;
            if n == 0 {
                continue;
            }
            before += glb.len() as u64;
            after += out.len() as u64;
            std::fs::write(&p, &out).map_err(|e| format!("{}: {e}", p.display()))?;
            files += 1;
            images += n;
        }
    }
    Ok((files, images, before, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GLB with two views: a "mesh" view and an image view marked KTX2.
    fn glb() -> Vec<u8> {
        let mesh: Vec<u8> = (0..37u8).collect();
        let fake_ktx: Vec<u8> = (100..110u8).collect();
        let mut bin = mesh.clone();
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let img_off = bin.len();
        bin.extend_from_slice(&fake_ktx);
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": mesh.len(), "target": 34962},
                {"buffer": 0, "byteOffset": img_off, "byteLength": fake_ktx.len()}
            ],
            "accessors": [{"bufferView": 0, "byteOffset": 4, "componentType": 5126, "count": 2, "type": "VEC3"}],
            "images": [{"mimeType": "image/ktx2", "bufferView": 1}],
            "textures": [{"source": 0}]
        });
        join(&json, &bin)
    }

    #[test]
    fn the_image_view_is_replaced_and_every_other_view_is_byte_identical() {
        let src = glb();
        let (out, n) = rewrite(&src, |image, ktx| {
            assert_eq!(image, 0);
            assert_eq!(ktx, &(100..110u8).collect::<Vec<u8>>());
            Ok((vec![7u8; 23], "image/jpeg"))
        })
        .unwrap();
        assert_eq!(n, 1);
        let (json, bin) = split(&out).unwrap();
        assert_eq!(json["images"][0]["mimeType"], "image/jpeg");
        let views = json["bufferViews"].as_array().unwrap();
        let (o0, l0) = (
            views[0]["byteOffset"].as_u64().unwrap() as usize,
            views[0]["byteLength"].as_u64().unwrap() as usize,
        );
        assert_eq!(&bin[o0..o0 + l0], &(0..37u8).collect::<Vec<u8>>()[..]);
        assert_eq!(views[0]["target"], 34962, "a view's other fields travel");
        let (o1, l1) = (
            views[1]["byteOffset"].as_u64().unwrap() as usize,
            views[1]["byteLength"].as_u64().unwrap() as usize,
        );
        assert_eq!(&bin[o1..o1 + l1], &[7u8; 23][..]);
        assert_eq!(o1 % 4, 0, "views are 4-aligned");
        assert_eq!(
            json["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
            bin.len()
        );
        assert_eq!(
            json["accessors"][0]["byteOffset"], 4,
            "accessors address their view relatively and do not move"
        );
        // A round trip through `split`/`join` is exact.
        assert_eq!(split(&join(&json, &bin)).unwrap().1, bin);
    }

    #[test]
    fn a_model_with_no_ktx2_passes_through() {
        let src = glb();
        let (mut json, bin) = split(&src).unwrap();
        json["images"][0]["mimeType"] = serde_json::Value::from("image/png");
        let plain = join(&json, &bin);
        let (out, n) = rewrite(&plain, |_, _| panic!("nothing to convert")).unwrap();
        assert_eq!(n, 0);
        assert_eq!(split(&out).unwrap().0["images"][0]["mimeType"], "image/png");
    }

    #[test]
    fn a_png_round_trips_its_pixels() {
        let rgba: Vec<u8> = (0..4 * 4 * 4).map(|i| (i * 7 % 256) as u8).collect();
        let bytes = png(4, 4, &rgba).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (4, 4));
        assert_eq!(&buf[..info.buffer_size()], &rgba[..]);
    }

    /// Alpha anywhere keeps the map lossless; an opaque one becomes a JPEG
    /// that decodes to the same picture within the quantizer's reach.
    #[test]
    fn alpha_picks_png_and_an_opaque_map_is_a_close_jpeg() {
        let (w, h) = (16u32, 16u32);
        let mut rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % w * 16) as u8, (i / w * 16) as u8, 128, 255])
            .collect();
        let (bytes, mime) = encode_web(w, h, &rgba, false).unwrap();
        assert_eq!(mime, "image/jpeg");
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
            .unwrap()
            .to_rgba8();
        assert_eq!(img.dimensions(), (w, h));
        let worst = img
            .as_raw()
            .chunks_exact(4)
            .zip(rgba.chunks_exact(4))
            .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
            .max()
            .unwrap();
        assert!(worst <= 24, "a 4:4:4 q90 JPEG is off by {worst} somewhere");

        rgba[7] = 254;
        assert_eq!(encode_web(w, h, &rgba, false).unwrap().1, "image/png");
    }

    #[test]
    fn normal_maps_are_found_through_the_basisu_extension() {
        let json = serde_json::json!({
            "materials": [
                {"normalTexture": {"index": 1}, "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}},
                {"normalTexture": {"index": 2}}
            ],
            "textures": [
                {"extensions": {"KHR_texture_basisu": {"source": 0}}},
                {"extensions": {"KHR_texture_basisu": {"source": 3}}},
                {"source": 4}
            ]
        });
        assert_eq!(normal_images(&json), vec![3, 4]);
    }
}
