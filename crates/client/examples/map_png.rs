//! Dev probe: dumps the MAP SCREEN's island as a PPM, so the one screen in
//! this client that is a picture can be looked at without a GPU.
//!
//! Not a gate — `CLAUDE.md` forbids a pixel gate and says the visual gate is
//! a person looking. This is what that person looks at when the machine they
//! are on has no GPU: `ui::map::paint` is the whole of the map's colour, it
//! is pure, and it is the only part of the client that can be rendered by a
//! `cargo run` with no window, no shard and no `--features render`.
//!
//! What it deliberately does NOT draw is everything the render layer puts ON
//! the island: the grid lines and their 256 labels, the marker badges, the
//! site names, the player arrow. Those are Bevy nodes and drawing a second
//! copy of them here would be a second implementation of the screen, which
//! is the hand-kept-mirror failure `CLAUDE.md` records twice. Boot the game
//! and hold G for those.
//!
//! Usage: `cargo run -p client --example map_png -- [seed] [px] [out.ppm]`

// Host-side probe: printing and file I/O are its job, not the sim's.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use client::ui::map;
use sim_core::terrain;

fn arg<T: std::str::FromStr>(i: usize, d: T) -> T {
    std::env::args()
        .nth(i)
        .and_then(|s| s.parse().ok())
        .unwrap_or(d)
}

fn main() {
    // The shard's own seed by default, so what you look at is the island a
    // player actually walks rather than an arbitrary one.
    let seed: u64 = arg(1, 20260731);
    let px: usize = arg(2, 512);
    let out: String = arg(3, "map.ppm".to_string());

    let haven = terrain::haven(seed);
    let mut rgba = vec![0u8; px * px * 4];
    map::paint(seed, &haven, px, &mut rgba);

    // PPM for `biome_map`'s reason: no image crate in this dependency tree,
    // and every viewer reads it.
    let mut rgb = Vec::with_capacity(px * px * 3);
    for p in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    use std::io::Write as _;
    let mut f = std::fs::File::create(&out).expect("create");
    write!(f, "P6\n{px} {px}\n255\n").expect("hdr");
    f.write_all(&rgb).expect("body");

    // Where the things the render layer marks actually are, as grid squares
    // — so a frame can be checked against a list rather than against memory.
    println!("seed {seed}  {px}px  -> {out}");
    let row = |what: &str, x: f32, z: f32| {
        // Map fractions as well as metres: they are what `render::map` places
        // a badge by, so a frame can be checked against this list with a
        // ruler rather than against somebody's memory of where the pad is.
        let (fx, fy) = map::world_to_map(x, z, 1);
        println!(
            "  {what:<12} {:<4}  ({x:7.1}, {z:7.1}) m   frac ({fx:.3}, {fy:.3})",
            map::grid_label(x, z)
        );
    };
    row("HAVEN", haven.x, haven.z);
    for (i, w) in haven.minor.iter().enumerate() {
        if w.live {
            row(&format!("site {i} {:?}", w.kind), w.x, w.z);
        }
    }
}
