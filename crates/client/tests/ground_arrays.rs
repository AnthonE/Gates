//! Gate: the ground's twenty photographs become two texture arrays, and they
//! become them at the right moment.
//!
//! `textures::stack_ground` exists because WebGL2 and WebGPU hold a fragment
//! stage to 16 sampled textures across every bind group in the pipeline, and
//! the ground alone was sixteen (`render/ground_splat.rs`, header). Stacking
//! is a copy — a layer samples exactly as the standalone texture did — so what
//! can go wrong is not arithmetic but PLUMBING, and each way is silent:
//!
//!  1. **Built too early.** An image is in `Assets<Image>` one frame before
//!     its `Added` event reaches `mipmap::enqueue`, so a builder keyed on
//!     "loaded and not pending" fires in that window and stacks sixteen
//!     single-level layers: the mip chains this module took a slice to build
//!     are then in the sources and in nothing that is drawn. The island reads
//!     as static at range with every gate green.
//!  2. **The wrong layer, or the wrong channel.** Roughness and AO are R
//!     and G of one layer beside the normals; a tap at the wrong offset reads
//!     a normal map, and swapped channels darken the island where a surface
//!     is smooth — neither looks like a wrong index.
//!  3. **A set that no longer fits.** A re-sourced 2K albedo beside three 1K
//!     ones cannot be one array. Refusing loudly is the design; this gate
//!     also reads the shipped files, so the refusal cannot be reached by a
//!     file swap that left the gates green. The one sanctioned resample is
//!     `textures::lift` — 512² roughness/AO into the 1024² data array — and
//!     it keeps the source chain byte for byte; `textures::pack_rg` is exact
//!     per channel.
//!  4. **Bytes in the wrong order.** wgpu reads an array's chains layer-major
//!     and the image must say so; a mip-major buffer draws every layer's
//!     level 1 where its level 0 should be.
//!
//! Headless — `MinimalPlugins` and the asset plugin, no GPU, no file for the
//! system-level legs (the images are built in code). Leg 3 reads the shipped
//! JPEG headers and nothing else.

#![cfg(feature = "render")]

use bevy::asset::{AssetPlugin, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDataOrder, TextureDimension, TextureFormat, TextureViewDimension,
};

use client::render::mipmap;
use client::render::textures::{
    self, layer_bytes, layer_ready, lift, pack_rg, stack, GroundArrays, GroundMaps, MapSet,
    StackError, FAMILY_LAYERS, ROUGH_AO_LAYER0,
};

/// The four identities, then the road's aggregate: the arrays' layer order.
const ROLES: [&str; 5] = ["sand", "grass", "litter", "rock", "aggregate"];
const FAMILIES: [&str; 4] = ["albedo", "normal", "rough", "ao"];

/// A `w × w` RGBA8 image whose every texel is `(seed, x, y, 255)`, one level,
/// the shape the loader hands `mipmap::drain`.
fn plane(w: u32, format: TextureFormat, seed: u8) -> Image {
    let mut data = Vec::with_capacity((w * w * 4) as usize);
    for y in 0..w {
        for x in 0..w {
            data.extend_from_slice(&[seed, x as u8, y as u8, 255]);
        }
    }
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: w,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::default(),
    );
    img.sampler = tiling();
    img
}

/// The descriptor `textures::tiling` builds, as far as a test can see it.
fn tiling() -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        anisotropy_clamp: 4,
        ..ImageSamplerDescriptor::linear()
    })
}

/// Give `img` its chain exactly as `mipmap::drain` does — count and buffer
/// together.
fn chained(mut img: Image, filter: mipmap::Filter) -> Image {
    let (w, h) = (img.width(), img.height());
    let level0 = img.data.clone().expect("a plane has data");
    img.data = Some(mipmap::chain(&level0, w, h, filter));
    img.texture_descriptor.mip_level_count = mipmap::levels(w, h);
    img
}

// ── 1. The stack is a layer-major copy of every chain ──────────────────────

#[test]
fn the_stack_is_layer_major_and_keeps_every_byte() {
    let layers: Vec<Image> = (0..4)
        .map(|k| {
            chained(
                plane(8, TextureFormat::Rgba8UnormSrgb, k),
                mipmap::Filter::Srgb,
            )
        })
        .collect();
    let refs: Vec<&Image> = layers.iter().collect();
    let arr = stack(&refs).expect("four matching chains stack");

    let d = &arr.texture_descriptor;
    assert_eq!(d.size.depth_or_array_layers, 4);
    assert_eq!((d.size.width, d.size.height), (8, 8));
    assert_eq!(d.dimension, TextureDimension::D2);
    assert_eq!(d.format, TextureFormat::Rgba8UnormSrgb);
    assert_eq!(
        d.mip_level_count,
        mipmap::levels(8, 8),
        "the chain travels whole"
    );
    assert_eq!(arr.data_order, TextureDataOrder::LayerMajor);
    assert_eq!(
        arr.texture_view_descriptor
            .as_ref()
            .and_then(|v| v.dimension),
        Some(TextureViewDimension::D2Array),
        "the view must say D2Array or the bind group refuses it at draw"
    );
    assert_eq!(arr.sampler, tiling(), "the sampler is the layers' own");
    assert_eq!(
        arr.asset_usage,
        RenderAssetUsages::RENDER_WORLD,
        "an array is upload-only; the sources hold the CPU copies"
    );

    // Layer-major: layer k's whole chain, then layer k+1's.
    let want: Vec<u8> = layers
        .iter()
        .flat_map(|l| l.data.clone().unwrap())
        .collect();
    assert_eq!(arr.data.as_deref(), Some(want.as_slice()));
    let per = layer_bytes(8, 8, mipmap::levels(8, 8), 4);
    assert_eq!(
        per,
        mipmap::chain_bytes(8, 8),
        "layer_bytes agrees with the chain"
    );
    for (k, l) in layers.iter().enumerate() {
        assert_eq!(
            &arr.data.as_ref().unwrap()[k * per..(k + 1) * per],
            l.data.as_deref().unwrap(),
            "layer {k} is not at offset {k}·{per}"
        );
    }
}

// ── 2. Readiness is the mip pass's own predicate ───────────────────────────

#[test]
fn a_layer_that_is_about_to_get_its_chain_is_not_ready() {
    let bare = plane(8, TextureFormat::Rgba8Unorm, 1);
    assert!(
        mipmap::wants(&bare),
        "the fixture is one `drain` will chain"
    );
    assert!(
        !layer_ready(&bare),
        "a one-level power-of-two image is about to be chained and must be waited for"
    );
    let done = chained(
        plane(8, TextureFormat::Rgba8Unorm, 1),
        mipmap::Filter::Linear,
    );
    assert!(layer_ready(&done), "a chained image is ready");
    // One the mip pass will never touch — not power-of-two — is ready as it
    // is, or the ground would wait forever on it.
    let odd = plane(6, TextureFormat::Rgba8Unorm, 1);
    assert!(!mipmap::wants(&odd));
    assert!(
        layer_ready(&odd),
        "an image the mip pass skips must not be waited for"
    );
}

// ── 3. A set that does not fit is refused, and the shipped set fits ────────

#[test]
fn layers_that_disagree_are_refused_whole() {
    let a = chained(
        plane(8, TextureFormat::Rgba8Unorm, 1),
        mipmap::Filter::Linear,
    );
    let big = chained(
        plane(16, TextureFormat::Rgba8Unorm, 2),
        mipmap::Filter::Linear,
    );
    assert_eq!(
        stack(&[&a, &big]).err(),
        Some(StackError::Mismatch {
            layer: 1,
            what: "size"
        })
    );
    let srgb = chained(
        plane(8, TextureFormat::Rgba8UnormSrgb, 3),
        mipmap::Filter::Srgb,
    );
    assert_eq!(
        stack(&[&a, &srgb]).err(),
        Some(StackError::Mismatch {
            layer: 1,
            what: "format"
        })
    );
    let flat = plane(8, TextureFormat::Rgba8Unorm, 4);
    assert_eq!(
        stack(&[&a, &flat]).err(),
        Some(StackError::Mismatch {
            layer: 1,
            what: "mip count"
        })
    );
    let mut clamped = chained(
        plane(8, TextureFormat::Rgba8Unorm, 5),
        mipmap::Filter::Linear,
    );
    clamped.sampler = ImageSampler::Default;
    assert_eq!(
        stack(&[&a, &clamped]).err(),
        Some(StackError::Mismatch {
            layer: 1,
            what: "sampler"
        })
    );
    let mut gone = chained(
        plane(8, TextureFormat::Rgba8Unorm, 6),
        mipmap::Filter::Linear,
    );
    gone.data = None;
    assert_eq!(stack(&[&a, &gone]).err(), Some(StackError::NoData(1)));
    assert_eq!(stack(&[]).err(), Some(StackError::Empty));
}

/// The shipped files can be the arrays the code builds: one size per family,
/// roughness the size of AO, and both the normals' size or exactly half it
/// (`textures::lift`), because all three share an array.
///
/// **Read off the headers, not typed.** `assets/textures/MANIFEST.md` says
/// "drop a better source in with the same name", and a 2K re-source of one
/// identity beside three 1K ones is exactly what `stack` refuses at boot —
/// here rather than there, so the refusal is never first met by a player.
#[test]
fn every_family_ships_at_one_size() {
    let dims = |role: &str, family: &str| {
        let path = format!(
            "{}/../../assets/textures/{role}_{family}.jpg",
            env!("CARGO_MANIFEST_DIR")
        );
        image::image_dimensions(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    for family in FAMILIES {
        let first = dims(ROLES[0], family);
        for role in &ROLES[1..] {
            assert_eq!(
                dims(role, family),
                first,
                "{role}_{family}.jpg is not the size of sand_{family}.jpg — the \
                 {family} maps are one texture array and every layer \
                 must share one size. Re-source the SET."
            );
        }
    }
    assert_eq!(
        dims("sand", "rough"),
        dims("sand", "ao"),
        "roughness and AO are packed into one layer (textures::pack_rg), so \
         the two families must ship at one size"
    );
    let (nw, nh) = dims("sand", "normal");
    let (rw, rh) = dims("sand", "rough");
    assert!(
        (nw, nh) == (rw, rh) || (nw, nh) == (2 * rw, 2 * rh),
        "normals ship at {nw}x{nh} and roughness/AO at {rw}x{rh}: they share \
         the data array, which takes roughness and AO at the normals' size or \
         exactly half it (textures::lift) and nothing else"
    );
}

/// `lift` doubles a map without changing what a minifying tap reads: its
/// chain IS the source's, one level down, and its new top level is the
/// source's own bilinear surface (wrapped — every ground map tiles) at the new
/// texel centres, to the byte.
#[test]
fn a_lifted_layer_keeps_its_chain_and_tops_it_with_its_own_surface() {
    let src = chained(
        plane(4, TextureFormat::Rgba8Unorm, 9),
        mipmap::Filter::Linear,
    );
    let up = lift(&src).expect("a linear chained plane lifts");
    let d = &up.texture_descriptor;
    assert_eq!((d.size.width, d.size.height), (8, 8));
    assert_eq!(d.format, TextureFormat::Rgba8Unorm);
    assert_eq!(d.mip_level_count, mipmap::levels(8, 8), "a full chain");
    assert_eq!(up.sampler, src.sampler, "the sampler travels");
    let data = up.data.as_deref().unwrap();
    let top = 8 * 8 * 4;
    assert_eq!(
        &data[top..],
        src.data.as_deref().unwrap(),
        "levels 1.. must BE the source's chain, or a minified tap moves"
    );
    // The hardware's bilinear under `Repeat`, at new texel (x2, y2)'s centre.
    let s = src.data.as_deref().unwrap();
    let at = |x: i64, y: i64, c: usize| {
        f64::from(s[((y.rem_euclid(4) * 4 + x.rem_euclid(4)) * 4) as usize + c])
    };
    for y2 in 0..8 {
        for x2 in 0..8 {
            let (u, v) = (x2 as f64 / 2.0 - 0.25, y2 as f64 / 2.0 - 0.25);
            let (x0, y0) = (u.floor() as i64, v.floor() as i64);
            let (fx, fy) = (u - u.floor(), v - v.floor());
            for c in 0..4 {
                let want = (at(x0, y0, c) * (1.0 - fx) + at(x0 + 1, y0, c) * fx) * (1.0 - fy)
                    + (at(x0, y0 + 1, c) * (1.0 - fx) + at(x0 + 1, y0 + 1, c) * fx) * fy;
                let got = f64::from(data[(y2 * 8 + x2) * 4 + c]);
                assert!(
                    (got - want).abs() <= 0.5,
                    "texel ({x2},{y2}) channel {c}: {got}, the source's surface is {want}"
                );
            }
        }
    }
    // Upsampling gamma-encoded bytes would be the wrong arithmetic.
    let srgb = chained(
        plane(4, TextureFormat::Rgba8UnormSrgb, 9),
        mipmap::Filter::Srgb,
    );
    assert!(lift(&srgb).is_none(), "an sRGB map must not be lifted");
}

/// A `w × w` greyscale map as `bevy_image` loads one (`r = g = b`, A 255),
/// a different value at every texel so a channel mix-up shows.
fn grey(w: u32, salt: u32) -> Image {
    let mut img = plane(w, TextureFormat::Rgba8Unorm, 0);
    for (i, t) in img.data.as_mut().unwrap().chunks_exact_mut(4).enumerate() {
        let v = ((i as u32 * 37 + salt * 101) % 256) as u8;
        t.copy_from_slice(&[v, v, v, 255]);
    }
    img
}

/// `pack_rg` is the two chains interleaved, every level, byte for byte: R the
/// roughness's, G the AO's, B 0 and A 255 — and the lift keeps B and A
/// constant at the level it adds.
#[test]
fn roughness_and_ao_pack_per_channel_through_the_whole_chain() {
    let rough = chained(grey(4, 1), mipmap::Filter::Linear);
    let ao = chained(grey(4, 2), mipmap::Filter::Linear);
    let packed = pack_rg(&rough, &ao).expect("two matching linear chains pack");
    let (dp, dr) = (&packed.texture_descriptor, &rough.texture_descriptor);
    assert_eq!(
        (dp.size, dp.mip_level_count, dp.format),
        (dr.size, dr.mip_level_count, TextureFormat::Rgba8Unorm)
    );
    assert_eq!(packed.sampler, rough.sampler, "the sampler travels");
    let (p, r, a) = (
        packed.data.as_deref().unwrap(),
        rough.data.as_deref().unwrap(),
        ao.data.as_deref().unwrap(),
    );
    assert_eq!(p.len(), r.len(), "the whole chain is packed");
    for (i, ((pt, rt), at)) in p
        .chunks_exact(4)
        .zip(r.chunks_exact(4))
        .zip(a.chunks_exact(4))
        .enumerate()
    {
        assert_eq!(pt, [rt[0], at[0], 0, 255], "texel {i} of the chain");
    }
    let up = lift(&packed).expect("a packed layer lifts");
    for t in up.data.as_deref().unwrap().chunks_exact(4) {
        assert_eq!((t[2], t[3]), (0, 255), "B and A are constants, lifted too");
    }
    // Two maps that cannot share a texel are refused, not resampled.
    let big = chained(grey(8, 2), mipmap::Filter::Linear);
    assert!(pack_rg(&rough, &big).is_none(), "sizes differ");
    assert!(pack_rg(&rough, &grey(4, 2)).is_none(), "mip counts differ");
    let srgb = chained(
        plane(4, TextureFormat::Rgba8UnormSrgb, 3),
        mipmap::Filter::Srgb,
    );
    assert!(pack_rg(&rough, &srgb).is_none(), "an sRGB map is not data");
}

// ── 4. The system: waits for every chain, one family a frame, rough/AO at 5 ─

/// Twenty fixtures shaped like the real set: 1K/512 become 8/4 here.
fn maps(images: &mut Assets<Image>) -> GroundMaps {
    let mut set = |k: u8| MapSet {
        albedo: images.add(plane(8, TextureFormat::Rgba8UnormSrgb, k)),
        normal: images.add(plane(8, TextureFormat::Rgba8Unorm, 16 + k)),
        rough: images.add(plane(4, TextureFormat::Rgba8Unorm, 32 + k)),
        ao: Some(images.add(plane(4, TextureFormat::Rgba8Unorm, 48 + k))),
    };
    GroundMaps {
        sand: set(0),
        grass: set(1),
        litter: set(2),
        rock: set(3),
        aggregate: set(4),
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Image>();
    app.add_systems(Update, textures::stack_ground);
    app
}

/// Chain every source in `maps`, as `drain` would have.
fn chain_all(app: &mut App, maps: &[Handle<Image>]) {
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    for h in maps {
        let img = images.get(h).unwrap().clone();
        let filter = mipmap::Filter::pick("", img.texture_descriptor.format, false);
        images.insert(h.id(), chained(img, filter)).unwrap();
    }
}

#[test]
fn the_ground_is_stacked_only_after_every_chain_lands_one_family_a_frame() {
    let mut app = app();
    let maps = {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        maps(&mut images)
    };
    let all: Vec<Handle<Image>> = [
        &maps.sand,
        &maps.grass,
        &maps.litter,
        &maps.rock,
        &maps.aggregate,
    ]
    .iter()
    .flat_map(|m| {
        [
            m.albedo.clone(),
            m.normal.clone(),
            m.rough.clone(),
            m.ao.clone().unwrap(),
        ]
    })
    .collect();
    // The one held back is sand's ALBEDO — a layer of the family built
    // first — so that while it waits, nothing behind it is built either, and
    // when it lands both families are still owed.
    let (held, rest) = all.split_first().unwrap();
    app.insert_resource(maps);

    // Loaded, not chained: the frame-wide window of trap 1. Nothing may be
    // built here however long it lasts.
    for _ in 0..10 {
        app.update();
    }
    assert!(
        !app.world().contains_resource::<GroundArrays>(),
        "arrays were built from sources that have no mip chain yet"
    );

    // All but one chained: still nothing, because the missing one is
    // in the family built first and the builder never skips past a family
    // that is not ready — the order is fixed, so the data array (whose layers
    // ARE all ready) waits behind it.
    chain_all(&mut app, rest);
    for _ in 0..10 {
        app.update();
    }
    assert!(
        !app.world().contains_resource::<GroundArrays>(),
        "arrays were built while one source still had no chain"
    );
    assert!(app.world().contains_resource::<GroundMaps>());

    // The last chain lands. One family a frame: albedo, then data — so the
    // resource exists after two updates and not after one.
    chain_all(&mut app, std::slice::from_ref(held));
    app.update();
    assert!(
        !app.world().contains_resource::<GroundArrays>(),
        "two arrays were built in one frame — the one-a-frame budget is gone"
    );
    app.update();
    let arrays = app
        .world()
        .get_resource::<GroundArrays>()
        .expect("two frames after the last chain, the arrays exist")
        .clone();
    assert!(
        !app.world().contains_resource::<GroundMaps>(),
        "the sources are still resident beside the arrays that copied them"
    );

    let images = app.world().resource::<Assets<Image>>();
    let albedo = images.get(&arrays.albedo).expect("albedo array");
    let data_arr = images.get(&arrays.data).expect("data array");
    assert_eq!(
        albedo.texture_descriptor.size.depth_or_array_layers,
        FAMILY_LAYERS
    );
    assert_eq!(
        albedo.texture_descriptor.format,
        TextureFormat::Rgba8UnormSrgb
    );
    assert_eq!(
        data_arr.texture_descriptor.size.depth_or_array_layers,
        2 * FAMILY_LAYERS
    );
    assert_eq!(
        data_arr.texture_descriptor.format,
        TextureFormat::Rgba8Unorm
    );
    for arr in [albedo, data_arr] {
        assert_eq!(
            arr.texture_descriptor.mip_level_count,
            mipmap::levels(8, 8),
            "the arrays carry the chain the sources were given"
        );
    }

    // Trap 2: the layer order, read off the bytes. Every layer is 8×8 with a
    // full chain now — roughness/AO packed and lifted from 4×4 — and a
    // source's R channel is its seed at every texel, so the upsample keeps it
    // and the seeds say which source landed where, and in which channel.
    let per = layer_bytes(8, 8, mipmap::levels(8, 8), 4);
    let data = data_arr.data.as_ref().unwrap();
    for k in 0..ROLES.len() {
        assert_eq!(albedo.data.as_ref().unwrap()[k * per], k as u8);
        assert_eq!(
            data[k * per],
            16 + k as u8,
            "data layer {k} is not {}'s normal",
            ROLES[k]
        );
        let layer = ROUGH_AO_LAYER0 as usize + k;
        let (r, g) = (32 + k as u8, 48 + k as u8);
        assert_eq!(
            data[layer * per..layer * per + 4],
            [r, g, 0, 255],
            "data layer {layer} is not {}'s roughness in R and AO in G",
            ROLES[k]
        );
        // Its level 1 IS the two 4×4 sources' level 0, packed.
        let packed = pack_rg(
            &plane(4, TextureFormat::Rgba8Unorm, r),
            &plane(4, TextureFormat::Rgba8Unorm, g),
        )
        .unwrap();
        let level1 = layer * per + 8 * 8 * 4;
        assert_eq!(
            &data[level1..level1 + 4 * 4 * 4],
            packed.data.as_deref().unwrap(),
            "data layer {layer}'s level 1 is not {}'s maps as shipped",
            ROLES[k]
        );
    }
    // B and A are constants on every texel of every packed level.
    for t in data[ROUGH_AO_LAYER0 as usize * per..].chunks_exact(4) {
        assert_eq!((t[2], t[3]), (0, 255));
    }
}
