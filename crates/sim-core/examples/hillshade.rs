//! Dev probe: dumps a hillshade + crease map of the height field as PPM.
//!
//! Not a gate. It exists to make a SHADING defect visible without a GPU: the
//! renderer takes its normal analytically from `terrain::ground`, so any
//! crease in that function's gradient is a crease in the frame, and this
//! draws exactly that with nothing else in the way.
//!
//! Usage: hillshade <seed> <cx> <cz> <span_m> <px> <out_prefix>

// Host-side probe: printing and file I/O are its job, not the sim's.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::terrain;

fn arg<T: std::str::FromStr>(i: usize, d: T) -> T {
    std::env::args()
        .nth(i)
        .and_then(|s| s.parse().ok())
        .unwrap_or(d)
}

fn main() {
    let seed: u64 = arg(1, 20260731);
    let cx: f32 = arg(2, 1024.0);
    let cz: f32 = arg(3, 1024.0);
    let span: f32 = arg(4, 900.0);
    let px: usize = arg(5, 720);
    let prefix: String = std::env::args().nth(6).unwrap_or_else(|| "hs".into());

    let step = span / px as f32;
    let x0 = cx - span * 0.5;
    let z0 = cz - span * 0.5;
    let d = (step * 0.5).max(0.25);

    // Sun: low and from the NW, which is what makes a contour crease pop.
    let (lx, ly, lz) = (-0.55f32, 0.62, -0.56);

    let mut h = vec![0.0f32; px * px];
    for iz in 0..px {
        for ix in 0..px {
            h[iz * px + ix] = terrain::height(seed, x0 + ix as f32 * step, z0 + iz as f32 * step);
        }
    }

    // Analytic gradient at each pixel, exactly the renderer's arms.
    let mut shade = vec![0u8; px * px * 3];
    let mut grad = vec![0.0f32; px * px];
    for iz in 0..px {
        for ix in 0..px {
            let x = x0 + ix as f32 * step;
            let z = z0 + iz as f32 * step;
            let hx = terrain::height(seed, x + d, z) - terrain::height(seed, x - d, z);
            let hz = terrain::height(seed, x, z + d) - terrain::height(seed, x, z - d);
            let (nx, ny, nz) = (-hx, 2.0 * d, -hz);
            let il = 1.0 / (nx * nx + ny * ny + nz * nz).sqrt();
            grad[iz * px + ix] = (hx * hx + hz * hz).sqrt() / (2.0 * d);
            let ndl = ((nx * il) * lx + (ny * il) * ly + (nz * il) * lz).max(0.0);
            let v = (0.12 + 0.88 * ndl).min(1.0);
            let y = h[iz * px + ix];
            let (r, g, b) = if y < 0.0 {
                (0.16 * v, 0.28 * v, 0.42 * v)
            } else {
                (0.78 * v, 0.73 * v, 0.62 * v)
            };
            let o = (iz * px + ix) * 3;
            shade[o] = (r * 255.0) as u8;
            shade[o + 1] = (g * 255.0) as u8;
            shade[o + 2] = (b * 255.0) as u8;
        }
    }

    // Crease map: |grad of the gradient magnitude|. A C1 crease in `height`
    // is a step in `grad`, which is a bright line here. Smooth relief is dark.
    let mut crease = vec![0u8; px * px * 3];
    let mut peak = 0.0f32;
    let mut sum = 0.0f64;
    for iz in 1..px - 1 {
        for ix in 1..px - 1 {
            let gx = (grad[iz * px + ix + 1] - grad[iz * px + ix - 1]) * 0.5;
            let gz = (grad[(iz + 1) * px + ix] - grad[(iz - 1) * px + ix]) * 0.5;
            let m = (gx * gx + gz * gz).sqrt();
            if m > peak {
                peak = m;
            }
            sum += m as f64;
            let v = (m * 26.0).min(1.0);
            let o = (iz * px + ix) * 3;
            crease[o] = (v * 255.0) as u8;
            crease[o + 1] = (v * 255.0) as u8;
            crease[o + 2] = (v * 255.0) as u8;
        }
    }

    for (name, buf) in [("shade", &shade), ("crease", &crease)] {
        let path = format!("{prefix}-{name}.ppm");
        let mut out = format!("P6\n{px} {px}\n255\n").into_bytes();
        out.extend_from_slice(buf);
        std::fs::write(&path, out).unwrap();
        println!("wrote {path}");
    }
    println!(
        "crease: peak {:.4}  mean {:.5}  (per-metre change in rise/run)",
        peak,
        sum / ((px - 2) * (px - 2)) as f64
    );
}
