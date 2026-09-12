//! The web asset variant, as a command: `ci/build_web.sh` runs this over the
//! STAGED `assets/models/` so a browser is handed models whose textures it
//! can read (`client::webassets` for the whole argument).
//!
//!   cargo run -p client --features webassets --bin web_assets -- <dir>
//!
//! Converts every `.glb` under `<dir>` in place — KTX2/UASTC images become
//! PNG at `WEB_LEVEL` — and prints what it did. Refuses to run over the
//! tree's own `assets/models/`: the variant is for a staged copy, and the
//! files under git are what every gate reads.
//!
//! `webassets` is its own feature rather than part of `render`: the
//! transcoder is the only thing it needs and Bevy is not, so a build script
//! pays a minute for this rather than the renderer's ten.

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            eprintln!("usage: web_assets <staged models dir>");
            std::process::exit(2);
        }
    };
    let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|e| {
        eprintln!("web_assets: {}: {e}", dir.display());
        std::process::exit(2);
    });
    // The tree's own models are what the gates read; a variant written over
    // them would redden `tests/prop_assets.rs` and hand the desktop a PNG.
    let tree = std::fs::canonicalize(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")).ok();
    if let Some(tree) = tree {
        if canonical.starts_with(&tree) {
            eprintln!(
                "web_assets: refusing to convert the tree's own assets ({}) — point this at a staged copy",
                canonical.display()
            );
            std::process::exit(2);
        }
    }
    match client::webassets::convert_dir(&canonical) {
        Ok((files, images, before, after)) => {
            println!(
                "web_assets: {files} models, {images} images -> PNG at level {} ({:.1} MB -> {:.1} MB)",
                client::webassets::WEB_LEVEL,
                before as f64 / 1e6,
                after as f64 / 1e6
            );
            if files == 0 {
                eprintln!(
                    "web_assets: nothing under {} carried a KTX2 image",
                    canonical.display()
                );
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("web_assets: {e}");
            std::process::exit(1);
        }
    }
}
