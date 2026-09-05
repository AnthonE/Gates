//! Gate: what a packed map DECODES to, read off the shipped file through
//! the same path Bevy takes — ktx2 container → zstd → basis-universal UASTC
//! → RGBA8 — and measured as arithmetic.
//!
//! **Why a decode and not a header.** `tests/prop_assets.rs` already holds
//! every prop's textures to KTX2, and `ci/ktx_pack.py --self-test`-shaped
//! checks read the container. Neither can see the VALUES, and on 2026-09-05
//! the values were the defect: every normal map in the tree — 23 files,
//! every prop, deployable, held item and both sites — decoded to X and Y
//! means of **0.212** where a tangent-space normal map centres on 0.5. The
//! packer had asked `ktx create` for a linear format and let its documented
//! default run the sRGB→linear curve over data that was never a colour
//! (0.5 through that curve is 0.214). Every vector was bent ~41° in tangent
//! space, in a world direction that changes at every UV seam, which is the
//! polygon-edged shading patchwork on the boulders. Three weeks, every gate
//! green, found by decoding one file.
//!
//! **Two things are measured, and both carry a known-defect list rather than
//! an `#[ignore]`.** A listed file must still measure DEFECTIVE — its entry
//! is removed when it is re-packed, and a re-pack that forgets fails here
//! rather than leaving a stale list that reads as coverage (`CLAUDE.md`'s
//! dead-citation trap, applied to a test). An unlisted file must measure
//! correct. So the list can shrink and cannot rot.
//!
//!   1. **Normal-map neutrality**: mean X and Y in 0.45..0.55 and at least
//!      90 % of texels decoding to a unit vector.
//!   2. **Albedo chart contrast** on the scatter props and sites: the UV
//!      islands agreeing with each other to `CHART_CONTRAST_MAX`, read from
//!      `ci/measure_glb.py` so the triage and the gate cannot disagree. The
//!      generator bakes each island under its own light; the seams follow
//!      polygon edges and read as fractures. `ci/flatten_charts.py` is the
//!      repair, and this is the gate on its EFFECT on what ships — the
//!      lesson of `water_carry.rs`, that a repair with no observable is
//!      deleted by accident with every gate green.
//!
//! The transcode calls the C function directly: `basis_universal 0.3.1`'s
//! safe `transcode_slice` computes the row pitch in blocks for every format,
//! and an uncompressed output needs it in pixels — the safe call segfaults
//! on RGBA32, measured on these very files. Bevy itself only ever asks that
//! transcoder for a block-compressed format, so it never trips it.

#![cfg(feature = "render")]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use basis_universal::{sys, DecodeFlags, TranscoderBlockFormat};
use client::render::props::{prop_models, OCCUPANTS};
use client::render::structures::DEPLOY_ASSET;
use client::ui::hold::{HeldSrc, HELD_MODELS};

/// Every packed model with a normal map, as it stood on 2026-09-05: bent by
/// the packer, X/Y means 0.212. Remove an entry when its file is re-packed
/// (`ci/ktx_pack.py`, which now refuses to produce this) — the test then
/// holds it to neutral.
const BENT_BY_THE_PACKER: &[&str] = &[
    "models/deploy/bag.glb",
    "models/deploy/box.glb",
    "models/deploy/fire.glb",
    "models/deploy/hearth.glb",
    "models/deploy/workbench.glb",
    "models/held/building_plan.glb",
    "models/held/hammer.glb",
    "models/held/hunting_bow.glb",
    "models/held/rock.glb",
    "models/held/stone_hatchet.glb",
    "models/held/stone_pickaxe.glb",
    "models/held/wooden_spear.glb",
    "models/prop/barrel.glb",
    "models/prop/cache.glb",
    "models/prop/crate.glb",
    "models/prop/node_metal.glb",
    "models/prop/node_stone.glb",
    "models/prop/node_sulfur.glb",
    "models/prop/rock_a.glb",
    "models/prop/rock_b.glb",
    "models/prop/rock_c.glb",
    "models/site/canopy.glb",
    "models/site/shelter.glb",
];

/// The scatter props' and sites' albedo chart contrast as shipped, measured
/// 2026-09-05. A listed file must still read within `PIN_TOL` of its pin;
/// remove the entry when it is re-packed through `ci/flatten_charts.py`.
const PATCHY_AS_SHIPPED: &[(&str, f64)] = &[
    ("models/prop/rock_a.glb", 0.139),
    ("models/prop/rock_b.glb", 0.147),
    ("models/prop/rock_c.glb", 0.089),
    ("models/prop/node_stone.glb", 0.309),
    ("models/prop/node_metal.glb", 0.178),
    ("models/prop/node_sulfur.glb", 0.143),
    ("models/prop/barrel.glb", 0.328),
    ("models/prop/crate.glb", 0.179),
    ("models/prop/cache.glb", 0.132),
    ("models/site/shelter.glb", 0.210),
    ("models/site/canopy.glb", 0.082),
];
const PIN_TOL: f64 = 0.01;

/// The one packed model with no normal map: the character, whose delivery
/// carried an albedo only (`assets/models/MANIFEST.md`). Named so the census
/// below cannot pass by skipping a file nobody listed.
const NO_NORMAL_MAP: &[&str] = &["models/stumpy.glb"];

fn asset_path(rel: &str) -> PathBuf {
    Path::new("../../assets").join(rel)
}

/// Every model path a table in this crate loads, deduplicated — props and
/// sites, deployables, and the held items (which reuse some deployables).
fn packed_models() -> Vec<&'static str> {
    let mut set = BTreeSet::new();
    for o in OCCUPANTS {
        for p in prop_models(o) {
            set.insert(*p);
        }
    }
    for p in DEPLOY_ASSET.iter().flatten() {
        set.insert(*p);
    }
    for m in HELD_MODELS.iter() {
        if let HeldSrc::Glb(p) = &m.src {
            set.insert(*p);
        }
    }
    set.into_iter().collect()
}

struct Glb {
    json: serde_json::Value,
    bin: Vec<u8>,
}

impl Glb {
    fn open(rel: &str) -> Self {
        let path = asset_path(rel);
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&raw[0..4], b"glTF", "{rel}: not a GLB");
        let mut off = 12;
        let (mut json, mut bin) = (None, Vec::new());
        while off + 8 <= raw.len() {
            let len = u32::from_le_bytes(raw[off..off + 4].try_into().unwrap()) as usize;
            let kind = u32::from_le_bytes(raw[off + 4..off + 8].try_into().unwrap());
            let body = &raw[off + 8..off + 8 + len];
            match kind {
                0x4E4F_534A => json = Some(serde_json::from_slice(body).expect("bad JSON chunk")),
                0x004E_4942 => bin = body.to_vec(),
                _ => {}
            }
            off += 8 + len;
        }
        Self {
            json: json.expect("GLB has no JSON chunk"),
            bin,
        }
    }

    fn view(&self, i: usize) -> &[u8] {
        let bv = &self.json["bufferViews"][i];
        let st = bv["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = bv["byteLength"].as_u64().unwrap() as usize;
        &self.bin[st..st + len]
    }

    /// The image bytes behind one material slot, or `None` if the material
    /// has no such slot.
    fn slot_image(&self, slot: &str) -> Option<&[u8]> {
        let mat = &self.json["materials"][0];
        let tex = mat[slot]
            .as_object()
            .or_else(|| mat["pbrMetallicRoughness"][slot].as_object())?;
        let ti = tex["index"].as_u64()? as usize;
        let src = self.json["textures"][ti]["source"].as_u64()? as usize;
        let img = &self.json["images"][src];
        assert_eq!(
            img["mimeType"].as_str(),
            Some("image/ktx2"),
            "{slot}: not KTX2 — `prop_assets.rs` should have caught this"
        );
        Some(self.view(img["bufferView"].as_u64()? as usize))
    }

    fn accessor_f32(&self, i: usize, n: usize) -> Vec<f32> {
        let a = &self.json["accessors"][i];
        assert_eq!(a["componentType"].as_u64(), Some(5126));
        let bv = &self.json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let st = bv["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        assert_eq!(
            bv["byteStride"].as_u64().unwrap_or((4 * n) as u64),
            (4 * n) as u64
        );
        let count = a["count"].as_u64().unwrap() as usize;
        (0..count * n)
            .map(|k| f32::from_le_bytes(self.bin[st + k * 4..st + k * 4 + 4].try_into().unwrap()))
            .collect()
    }

    fn indices(&self) -> Vec<u32> {
        let p = &self.json["meshes"][0]["primitives"][0];
        let i = p["indices"].as_u64().expect("no indices") as usize;
        let a = &self.json["accessors"][i];
        let bv = &self.json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let st = bv["byteOffset"].as_u64().unwrap_or(0) as usize
            + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let count = a["count"].as_u64().unwrap() as usize;
        match a["componentType"].as_u64() {
            Some(5123) => (0..count)
                .map(|k| {
                    u16::from_le_bytes(self.bin[st + k * 2..st + k * 2 + 2].try_into().unwrap())
                        as u32
                })
                .collect(),
            Some(5125) => (0..count)
                .map(|k| {
                    u32::from_le_bytes(self.bin[st + k * 4..st + k * 4 + 4].try_into().unwrap())
                })
                .collect(),
            other => panic!("indices componentType {other:?}"),
        }
    }

    fn uvs(&self) -> Vec<[f32; 2]> {
        let p = &self.json["meshes"][0]["primitives"][0];
        let i = p["attributes"]["TEXCOORD_0"]
            .as_u64()
            .expect("no TEXCOORD_0") as usize;
        self.accessor_f32(i, 2)
            .chunks(2)
            .map(|c| [c[0], c[1]])
            .collect()
    }

    fn vertex_count(&self) -> usize {
        let p = &self.json["meshes"][0]["primitives"][0];
        let i = p["attributes"]["POSITION"].as_u64().unwrap() as usize;
        self.json["accessors"][i]["count"].as_u64().unwrap() as usize
    }
}

/// Level 0 of a KTX2, as `(width, height, rgba8)`.
fn decode_level0(ktx: &[u8]) -> (u32, u32, Vec<u8>) {
    let reader = ktx2::Reader::new(ktx).expect("ktx2 parse");
    let h = reader.header();
    let level = reader.levels().next().expect("level 0");
    let raw: Vec<u8> = match h.supercompression_scheme {
        Some(ktx2::SupercompressionScheme::Zstandard) => {
            let mut cursor = std::io::Cursor::new(level.data);
            let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut cursor).expect("zstd frame");
            let mut out = Vec::new();
            dec.read_to_end(&mut out).expect("zstd read");
            out
        }
        None => level.data.to_vec(),
        other => panic!("unhandled supercompression {other:?}"),
    };
    let (w, hg) = (h.pixel_width, h.pixel_height);
    let (bx, by) = (w.div_ceil(4), hg.div_ceil(4));
    assert_eq!(
        raw.len(),
        (bx * by * 16) as usize,
        "level 0 is not {bx}x{by} UASTC blocks"
    );
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
            w, // row pitch in PIXELS for an uncompressed output — see the header
            std::ptr::null_mut(),
            hg,
            0,
            3,
            DecodeFlags::HIGH_QUALITY.bits(),
        )
    };
    unsafe { sys::low_level_uastc_transcoder_delete(t) };
    assert!(ok, "UASTC transcode failed");
    (w, hg, rgba)
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// One number read off `ci/measure_glb.py`, so the triage and this gate
/// have one owner for the band.
fn python_const(name: &str) -> f64 {
    let src = std::fs::read_to_string("../../ci/measure_glb.py").expect("ci/measure_glb.py");
    let line = src
        .lines()
        .find(|l| l.starts_with(&format!("{name} = ")))
        .unwrap_or_else(|| panic!("{name} is not defined at top level in ci/measure_glb.py"));
    line.split('=').nth(1).unwrap().trim().parse().unwrap()
}

#[test]
fn every_packed_model_is_in_a_table_or_named() {
    // The census runs the other way from `every_declared_model_ships`: a
    // file on disk that no table loads is a file no gate here reads.
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                // Vendor deliveries under review are gitignored and not ours
                // to measure (`.gitignore`).
                if p.file_name().and_then(|n| n.to_str()) != Some("To Examine") {
                    walk(&p, out);
                }
            } else if p.extension().and_then(|x| x.to_str()) == Some("glb") {
                out.push(
                    p.strip_prefix("../../assets")
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut on_disk = Vec::new();
    walk(&asset_path("models"), &mut on_disk);
    let tables: BTreeSet<&str> = packed_models().into_iter().collect();
    for f in &on_disk {
        assert!(
            tables.contains(f.as_str()) || NO_NORMAL_MAP.contains(&f.as_str()),
            "{f} is on disk and in no table this test reads — add it to a table or name it"
        );
    }
    assert!(on_disk.len() >= 20, "walked only {} models", on_disk.len());
}

#[test]
fn every_packed_normal_map_decodes_neutral_or_is_on_the_bent_list() {
    let bent: BTreeSet<&str> = BENT_BY_THE_PACKER.iter().copied().collect();
    let mut seen = BTreeSet::new();
    println!(
        "{:28} {:>6} {:>6} {:>6}  state",
        "file", "mean x", "mean y", "unit%"
    );
    for rel in packed_models() {
        let g = Glb::open(rel);
        let Some(ktx) = g.slot_image("normalTexture") else {
            assert!(
                NO_NORMAL_MAP.contains(&rel),
                "{rel} has no normal map and is not on NO_NORMAL_MAP"
            );
            continue;
        };
        seen.insert(rel);
        let (w, h, rgba) = decode_level0(ktx);
        let n = (w * h) as usize;
        let (mut sx, mut sy, mut unit) = (0.0f64, 0.0f64, 0usize);
        for px in rgba.chunks_exact(4) {
            let x = px[0] as f64 / 255.0;
            let y = px[1] as f64 / 255.0;
            let z = px[2] as f64 / 255.0;
            sx += x;
            sy += y;
            let (nx, ny, nz) = (x * 2.0 - 1.0, y * 2.0 - 1.0, z * 2.0 - 1.0);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            if (0.9..=1.1).contains(&len) {
                unit += 1;
            }
        }
        let (mx, my, uf) = (sx / n as f64, sy / n as f64, unit as f64 / n as f64);
        let listed = bent.contains(rel);
        println!(
            "{rel:28} {mx:6.3} {my:6.3} {:5.1}%  {}",
            uf * 100.0,
            if listed { "bent (listed)" } else { "neutral" }
        );
        if listed {
            assert!(
                (0.19..=0.24).contains(&mx) && (0.19..=0.24).contains(&my),
                "{rel} is on BENT_BY_THE_PACKER but decodes to x {mx:.3} y {my:.3} — \
                 it has been re-packed; remove it from the list so this gate holds it neutral"
            );
        } else {
            assert!(
                (0.45..=0.55).contains(&mx) && (0.45..=0.55).contains(&my),
                "{rel}: normal map X/Y means {mx:.3}/{my:.3}, not the 0.5 a tangent-space map \
                 centres on — a transfer function was applied to data (ci/ktx_pack.py's header)"
            );
            assert!(
                uf >= 0.9,
                "{rel}: only {:.1}% of decoded normals are unit length",
                uf * 100.0
            );
        }
    }
    for rel in BENT_BY_THE_PACKER {
        assert!(
            seen.contains(rel),
            "{rel} is listed as bent but was not measured"
        );
    }
}

/// `glbcharts.charts` + `rasterize` + `chart_contrast`, in Rust, the same
/// algorithm texel for texel: a chart is a connected component of the index
/// buffer, a texel belongs to the chart of the first triangle whose edge
/// functions agree on its centre, and the contrast is the texel-weighted mean
/// of |chart mean / global mean − 1| over covered texels.
fn chart_contrast(g: &Glb, w: u32, h: u32, lum: &[f64]) -> (usize, f64) {
    let idx = g.indices();
    let nv = g.vertex_count();
    let mut parent: Vec<usize> = (0..nv).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        let mut root = x;
        while p[root] != root {
            root = p[root];
        }
        while p[x] != root {
            let next = p[x];
            p[x] = root;
            x = next;
        }
        root
    }
    for t in idx.chunks_exact(3) {
        for (u, v) in [(t[0], t[1]), (t[1], t[2])] {
            let (ru, rv) = (find(&mut parent, u as usize), find(&mut parent, v as usize));
            if ru != rv {
                parent[ru] = rv;
            }
        }
    }
    let mut label = BTreeMap::new();
    let mut vert_chart = vec![0usize; nv];
    for (v, slot) in vert_chart.iter_mut().enumerate() {
        let r = find(&mut parent, v);
        let n = label.len();
        *slot = *label.entry(r).or_insert(n);
    }
    let n_charts = label.len();

    let uv = g.uvs();
    let (wf, hf) = (w as f64, h as f64);
    let mut map = vec![-1i64; (w * h) as usize];
    for t in idx.chunks_exact(3) {
        let chart = vert_chart[t[0] as usize] as i64;
        let xs: Vec<f64> = t.iter().map(|&i| uv[i as usize][0] as f64 * wf).collect();
        let ys: Vec<f64> = t.iter().map(|&i| uv[i as usize][1] as f64 * hf).collect();
        let x0 = xs
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as i64;
        let x1 = xs
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .min(wf - 1.0) as i64;
        let y0 = ys
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as i64;
        let y1 = ys
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .min(hf - 1.0) as i64;
        for py in y0..=y1 {
            for px in x0..=x1 {
                let (gx, gy) = (px as f64 + 0.5, py as f64 + 0.5);
                let e0 = (xs[1] - xs[0]) * (gy - ys[0]) - (ys[1] - ys[0]) * (gx - xs[0]);
                let e1 = (xs[2] - xs[1]) * (gy - ys[1]) - (ys[2] - ys[1]) * (gx - xs[1]);
                let e2 = (xs[0] - xs[2]) * (gy - ys[2]) - (ys[0] - ys[2]) * (gx - xs[2]);
                let inside =
                    (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0);
                if inside {
                    map[(py as u32 * w + px as u32) as usize] = chart;
                }
            }
        }
    }
    let mut sum = vec![0.0f64; n_charts];
    let mut cnt = vec![0.0f64; n_charts];
    for (i, &c) in map.iter().enumerate() {
        if c >= 0 {
            sum[c as usize] += lum[i];
            cnt[c as usize] += 1.0;
        }
    }
    let total: f64 = cnt.iter().sum();
    let global = sum.iter().sum::<f64>() / total;
    let mut dev = 0.0;
    for c in 0..n_charts {
        if cnt[c] > 0.0 {
            dev += ((sum[c] / cnt[c]) / global - 1.0).abs() * cnt[c];
        }
    }
    (n_charts, dev / total)
}

#[test]
fn every_scatter_albedo_is_chart_flat_or_on_the_patchy_list() {
    let ceiling = python_const("CHART_CONTRAST_MAX");
    let pins: BTreeMap<&str, f64> = PATCHY_AS_SHIPPED.iter().copied().collect();
    let mut seen = BTreeSet::new();
    println!("{:28} {:>6} {:>8}  state", "file", "charts", "contrast");
    for o in OCCUPANTS {
        for rel in prop_models(o) {
            let g = Glb::open(rel);
            let ktx = g
                .slot_image("baseColorTexture")
                .expect("a prop has an albedo");
            let (w, h, rgba) = decode_level0(ktx);
            let lum: Vec<f64> = rgba
                .chunks_exact(4)
                .map(|p| {
                    0.2126 * srgb_to_linear(p[0] as f64 / 255.0)
                        + 0.7152 * srgb_to_linear(p[1] as f64 / 255.0)
                        + 0.0722 * srgb_to_linear(p[2] as f64 / 255.0)
                })
                .collect();
            let (charts, contrast) = chart_contrast(&g, w, h, &lum);
            seen.insert(*rel);
            match pins.get(rel) {
                Some(pin) => {
                    println!("{rel:28} {charts:6} {contrast:8.3}  patchy (pinned {pin:.3})");
                    assert!(
                        (contrast - pin).abs() <= PIN_TOL,
                        "{rel} is on PATCHY_AS_SHIPPED at {pin:.3} but measures {contrast:.3} — \
                         re-packed? remove its entry so this gate holds it to {ceiling}"
                    );
                }
                None => {
                    println!("{rel:28} {charts:6} {contrast:8.3}  flat");
                    assert!(
                        contrast <= ceiling,
                        "{rel}: albedo chart contrast {contrast:.3} over {ceiling} — the UV islands \
                         disagree with each other; ci/flatten_charts.py before ci/ktx_pack.py"
                    );
                }
            }
        }
    }
    for (rel, _) in PATCHY_AS_SHIPPED {
        assert!(
            seen.contains(rel),
            "{rel} is pinned as patchy but was not measured"
        );
    }
}
