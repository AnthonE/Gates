//! Write an authored kit's boxes and anchors as JSON for the Blender dresser
//! (`ci/site_kit.py`): `cargo run -p sim-core --example kit_dump -- town`
//! prints `ci/kits/town.json`. `tests/kit_dump.rs` holds the committed files
//! to the source.

// Host-side tool: printing and a String are its job, not the sim's.
#![allow(clippy::disallowed_macros, clippy::disallowed_types)]

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "town".into());
    let mut out = String::new();
    match which.as_str() {
        "town" => sim_core::town::dump(&mut out).unwrap(),
        other => panic!("no kit named {other}"),
    }
    print!("{out}");
}
