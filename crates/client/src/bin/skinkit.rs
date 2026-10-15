//! `skinkit`: paint a skin for Gates. For a person or an agent, no GPU.
//!
//! ```sh
//! cargo run -p client --features render --release --bin skinkit -- list
//! cargo run ... -- template tunic template.png        # the UV layout to paint on
//! cargo run ... -- check tunic skin.png --name "Field Medic Tunic"
//! cargo run ... -- preview tunic skin.png preview.png # front, side, back
//! ```
//!
//! The layouts are the game's own meshes (`render::canvas`), so a template
//! and a preview can never disagree with what the game draws. `check` says
//! whether the game will take the picture (`render::paint::pixels`, and the
//! art bar's `ALBEDO_LUMA_BAND`, `ART.md` §7) and prints the exact rows that
//! list it: `content/skins.toml` here, and the platform's mirror.

use client::render::canvas::{self, Canvas, Flat, CANVASES};
use client::render::paint;
use sha2::{Digest, Sha256};
use std::path::Path;

/// `ART.md` §7: a surface's mean linear luma stays in this band, or it
/// reads as a light source or a hole under the game's light.
const LUMA_BAND: (f32, f32) = (0.05, 0.55);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match words.as_slice() {
        ["list"] => {
            list();
            Ok(())
        }
        ["template", item, out, rest @ ..] => template(item, out, rest),
        ["check", item, png, rest @ ..] => check(item, png, rest),
        ["preview", item, png, out] => preview(item, png, out),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("skinkit: {e}");
        std::process::exit(1);
    }
}

const USAGE: &str = "usage: skinkit list | template ITEM OUT.png [--size 1024] | check ITEM SKIN.png [--name NAME] | preview ITEM SKIN.png OUT.png";

fn canvas_of(item: &str) -> Result<&'static Canvas, String> {
    canvas::find(item).ok_or_else(|| {
        let keys: Vec<&str> = CANVASES.iter().map(|c| c.key).collect();
        format!(
            "{item:?} cannot be painted (yet); paintable: {}",
            keys.join(", ")
        )
    })
}

fn flats(c: &Canvas) -> Vec<Flat> {
    canvas::meshes(c).iter().map(canvas::flat).collect()
}

fn list() {
    for c in &CANVASES {
        println!("{} ({})", c.key, c.item);
        println!("  wraps {}", c.wrap);
        for r in c.regions {
            println!(
                "  {:>5.1}%..{:>5.1}% from the top: {}",
                r.v.0 * 100.0,
                r.v.1 * 100.0,
                r.name
            );
        }
    }
    println!(
        "A skin is one square PNG, {}..{} px (a power of two), at most {} KiB, mean brightness inside the art bar.",
        paint::PAINT_MIN_SIDE,
        paint::PAINT_MAX_SIDE,
        paint::PAINT_MAX_BYTES / 1024
    );
}

/// Which texels of a `side`² image the canvas's triangles cover.
fn coverage(fl: &[Flat], side: usize) -> Vec<bool> {
    let mut used = vec![false; side * side];
    for f in fl {
        for t in &f.tris {
            let uv = t.map(|i| f.uv[i as usize]);
            raster_uv(uv, side, |x, y| used[y * side + x] = true);
        }
    }
    used
}

/// Call `put` for every texel centre inside the UV triangle `uv`.
fn raster_uv(uv: [[f32; 2]; 3], side: usize, mut put: impl FnMut(usize, usize)) {
    let s = side as f32;
    let p = uv.map(|[u, v]| [u * s, v * s]);
    let area =
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
    if area.abs() < 1e-9 {
        return;
    }
    let (x0, x1) = (
        p.iter()
            .map(|q| q[0])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize,
        p.iter()
            .map(|q| q[0])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(s) as usize,
    );
    let (y0, y1) = (
        p.iter()
            .map(|q| q[1])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize,
        p.iter()
            .map(|q| q[1])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(s) as usize,
    );
    for y in y0..y1 {
        for x in x0..x1 {
            let c = [x as f32 + 0.5, y as f32 + 0.5];
            let w0 =
                ((p[1][0] - c[0]) * (p[2][1] - c[1]) - (p[2][0] - c[0]) * (p[1][1] - c[1])) / area;
            let w1 =
                ((p[2][0] - c[0]) * (p[0][1] - c[1]) - (p[0][0] - c[0]) * (p[2][1] - c[1])) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                put(x, y);
            }
        }
    }
}

fn line(img: &mut [u8], side: usize, a: [f32; 2], b: [f32; 2], rgb: [u8; 3]) {
    let s = side as f32;
    let (ax, ay, bx, by) = (a[0] * s, a[1] * s, b[0] * s, b[1] * s);
    let n = ((bx - ax).abs().max((by - ay).abs()).ceil() as usize).max(1);
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, y) = ((ax + (bx - ax) * t) as usize, (ay + (by - ay) * t) as usize);
        if x < side && y < side {
            let o = (y * side + x) * 4;
            img[o..o + 3].copy_from_slice(&rgb);
        }
    }
}

fn template(item: &str, out: &str, rest: &[&str]) -> Result<(), String> {
    let c = canvas_of(item)?;
    let side = match rest {
        [] => 1024,
        ["--size", n] => n.parse::<usize>().map_err(|_| "--size is a number")?,
        _ => return Err(USAGE.into()),
    };
    if !side.is_power_of_two() || !(256..=2048).contains(&side) {
        return Err("--size is a power of two, 256..2048".into());
    }
    let fl = flats(c);
    // Regions in alternating light tones, off-canvas dark, the wireframe on
    // top, region edges and quarter turns as guides.
    const TONES: [[u8; 3]; 4] = [
        [214, 205, 186],
        [186, 200, 214],
        [206, 214, 186],
        [214, 190, 196],
    ];
    let used = coverage(&fl, side);
    let mut img = vec![0u8; side * side * 4];
    for y in 0..side {
        let v = (y as f32 + 0.5) / side as f32;
        let tone = c
            .regions
            .iter()
            .position(|r| v >= r.v.0 && v < r.v.1)
            .map_or([200, 200, 200], |i| TONES[i % TONES.len()]);
        for x in 0..side {
            let o = (y * side + x) * 4;
            let rgb = if used[y * side + x] {
                tone
            } else {
                [40, 40, 44]
            };
            img[o..o + 3].copy_from_slice(&rgb);
            img[o + 3] = 255;
        }
    }
    for f in &fl {
        for t in &f.tris {
            let uv = t.map(|i| f.uv[i as usize]);
            for k in 0..3 {
                line(&mut img, side, uv[k], uv[(k + 1) % 3], [120, 120, 128]);
            }
        }
    }
    for q in 1..4 {
        let u = q as f32 / 4.0;
        line(&mut img, side, [u, 0.0], [u, 0.999], [60, 140, 220]);
    }
    for r in c.regions.iter().skip(1) {
        line(&mut img, side, [0.0, r.v.0], [0.999, r.v.0], [220, 60, 60]);
    }
    write_png(Path::new(out), side as u32, &img)?;
    println!("wrote {out} ({side}x{side}) for {} ({})", c.key, c.item);
    println!(
        "  wraps {}; blue lines are quarter turns, red lines region edges",
        c.wrap
    );
    for r in c.regions {
        println!(
            "  rows {}..{}: {}",
            (r.v.0 * side as f32) as usize,
            (r.v.1 * side as f32) as usize,
            r.name
        );
    }
    println!("  dark texels are off the item: paint there is never seen");
    Ok(())
}

/// sRGB byte → linear.
fn lin(c: u8) -> f32 {
    let x = c as f32 / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear → sRGB byte.
fn srgb(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let s = if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

fn check(item: &str, png: &str, rest: &[&str]) -> Result<(), String> {
    let c = canvas_of(item)?;
    let name = match rest {
        [] => None,
        ["--name", n] => Some(*n),
        _ => return Err(USAGE.into()),
    };
    let bytes = std::fs::read(png).map_err(|e| format!("{png}: {e}"))?;
    let (side, px) = paint::pixels(&bytes).map_err(|e| format!("{png}: {e}"))?;
    let used = coverage(&flats(c), side as usize);
    let (mut sum, mut n) = ([0f64; 3], 0usize);
    for (i, &u) in used.iter().enumerate() {
        if u {
            for k in 0..3 {
                sum[k] += f64::from(lin(px[i * 4 + k]));
            }
            n += 1;
        }
    }
    let mean = sum.map(|s| (s / n.max(1) as f64) as f32);
    let luma = 0.2126 * mean[0] + 0.7152 * mean[1] + 0.0722 * mean[2];
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let tint = mean.map(srgb);
    println!("{png}: {side}x{side}, {} bytes, sha256 {sha}", bytes.len());
    println!(
        "  mean linear colour {:.3} {:.3} {:.3}, luma {luma:.3} (the art bar: {}..{})",
        mean[0], mean[1], mean[2], LUMA_BAND.0, LUMA_BAND.1
    );
    if !(LUMA_BAND.0..=LUMA_BAND.1).contains(&luma) {
        return Err(format!(
            "luma {luma:.3} is outside {}..{}: {} under the game's light. Real cloth, wood and paint sit inside it",
            LUMA_BAND.0,
            LUMA_BAND.1,
            if luma > LUMA_BAND.1 { "darken it; it would glow" } else { "lighten it; it would read as a hole" }
        ));
    }
    let name = name.unwrap_or("NAME");
    let slug: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let next = next_catalog();
    println!("ok: the game takes it. To list it, one pull request to AnthonE/Gates:");
    println!("  1. the PNG at assets/skins/{slug}.png, and a row in assets/skins/MANIFEST.md (who made it, CC0 or your own work)");
    println!("  2. this row in content/skins.toml:\n");
    println!("[[skin]]");
    println!("id = \"skin.{}_{slug}\"", c.key);
    println!(
        "catalog = {}",
        next.map_or("NEXT_FREE_ID".to_string(), |n| n.to_string())
    );
    println!("name = \"{name}\"");
    println!("covers = \"{}\"", c.item);
    println!("tint = [{}, {}, {}]", tint[0], tint[1], tint[2]);
    println!("texture = \"{sha}\"");
    println!("season = \"community\"");
    println!(
        "\nThe platform mirrors it when it is listed: the PNG at data/items/skins/{sha}.png and a catalogs.json row."
    );
    Ok(())
}

/// One past the highest `catalog` in `content/skins.toml`, run from a Gates
/// checkout; `None` elsewhere.
fn next_catalog() -> Option<u32> {
    let text = std::fs::read_to_string("content/skins.toml").ok()?;
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("catalog = "))
        .filter_map(|n| n.trim().parse::<u32>().ok())
        .max()
        .map(|m| m + 1)
}

fn preview(item: &str, png: &str, out: &str) -> Result<(), String> {
    let c = canvas_of(item)?;
    let bytes = std::fs::read(png).map_err(|e| format!("{png}: {e}"))?;
    let (side, tex) = paint::pixels(&bytes).map_err(|e| format!("{png}: {e}"))?;
    let fl = flats(c);
    const W: usize = 384;
    const H: usize = 512;
    let views = [0.0f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI];
    let mut sheet = vec![0u8; W * 3 * H * 4];
    // The frame: every view the same scale, the item's own bounds.
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for f in &fl {
        for p in &f.pos {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let centre = [
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ];
    let reach = ((hi[0] - lo[0]).max(hi[2] - lo[2])).max(hi[1] - lo[1]) * 0.55;
    let scale = (H as f32 / 2.0) / reach.max(1e-6);
    for (vi, &yaw) in views.iter().enumerate() {
        let (sin, cos) = yaw.sin_cos();
        let mut depth = vec![f32::MIN; W * H];
        let mut px = vec![[52u8, 56, 62]; W * H];
        let light = {
            let l = [-0.4f32, 0.6, 0.7];
            let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
            l.map(|x| x / n)
        };
        for f in &fl {
            let view = |p: [f32; 3]| {
                let (x, y, z) = (p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]);
                [x * cos - z * sin, y, x * sin + z * cos]
            };
            let rot = |n: [f32; 3]| [n[0] * cos - n[2] * sin, n[1], n[0] * sin + n[2] * cos];
            for t in &f.tris {
                let v = t.map(|i| view(f.pos[i as usize]));
                let nn = t.map(|i| rot(f.nrm[i as usize]));
                let uv = t.map(|i| f.uv[i as usize]);
                let s = v.map(|p| [W as f32 / 2.0 + p[0] * scale, H as f32 / 2.0 - p[1] * scale]);
                let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1])
                    - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let x0 = s
                    .iter()
                    .map(|q| q[0])
                    .fold(f32::MAX, f32::min)
                    .floor()
                    .max(0.0) as usize;
                let x1 = s
                    .iter()
                    .map(|q| q[0])
                    .fold(f32::MIN, f32::max)
                    .ceil()
                    .min(W as f32) as usize;
                let y0 = s
                    .iter()
                    .map(|q| q[1])
                    .fold(f32::MAX, f32::min)
                    .floor()
                    .max(0.0) as usize;
                let y1 = s
                    .iter()
                    .map(|q| q[1])
                    .fold(f32::MIN, f32::max)
                    .ceil()
                    .min(H as f32) as usize;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let c = [x as f32 + 0.5, y as f32 + 0.5];
                        let w0 = ((s[1][0] - c[0]) * (s[2][1] - c[1])
                            - (s[2][0] - c[0]) * (s[1][1] - c[1]))
                            / area;
                        let w1 = ((s[2][0] - c[0]) * (s[0][1] - c[1])
                            - (s[0][0] - c[0]) * (s[2][1] - c[1]))
                            / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let z = w0 * v[0][2] + w1 * v[1][2] + w2 * v[2][2];
                        let i = y * W + x;
                        if z <= depth[i] {
                            continue;
                        }
                        depth[i] = z;
                        let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
                        let vv = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
                        let tx = ((u.rem_euclid(1.0)) * side as f32) as usize % side as usize;
                        let ty = ((vv.clamp(0.0, 0.9999)) * side as f32) as usize;
                        let o = (ty * side as usize + tx) * 4;
                        let mut n = [0f32; 3];
                        for k in 0..3 {
                            n[k] = w0 * nn[0][k] + w1 * nn[1][k] + w2 * nn[2][k];
                        }
                        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                        // Facing away (the inside of cloth): its own normal flipped.
                        let mut d = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / len;
                        if n[2] < 0.0 {
                            d = -d;
                        }
                        let shade = 0.35 + 0.65 * d.max(0.0);
                        px[i] = [0, 1, 2].map(|k| srgb(lin(tex[o + k]) * shade));
                    }
                }
            }
        }
        for y in 0..H {
            for x in 0..W {
                let o = (y * W * 3 + vi * W + x) * 4;
                sheet[o..o + 3].copy_from_slice(&px[y * W + x]);
                sheet[o + 3] = 255;
            }
        }
    }
    write_png_wh(Path::new(out), (W * 3) as u32, H as u32, &sheet)?;
    println!("wrote {out}: {} three ways (front, side, back)", c.key);
    Ok(())
}

fn write_png(path: &Path, side: u32, rgba: &[u8]) -> Result<(), String> {
    write_png_wh(path, side, side, rgba)
}

fn write_png_wh(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}
