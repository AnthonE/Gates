//! Write an authored kit's boxes and anchors as JSON for the Blender dresser
//! (`ci/site_kit.py`): `cargo run -p sim-core --example kit_dump -- town` (or `ziggurat`,
//! `mark_ruin`, …)
//! prints `ci/kits/town.json`. `tests/kit_dump.rs` holds the committed files
//! to the source.

// Host-side tool: printing and a String are its job, not the sim's.
#![allow(clippy::disallowed_macros, clippy::disallowed_types)]

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "town".into());
    let mut out = String::new();
    match which.as_str() {
        "town" => sim_core::town::dump(&mut out).unwrap(),
        "ziggurat" => sim_core::monument::dump(&mut out).unwrap(),
        other => {
            let kind = sim_core::landmark::DRESSED
                .into_iter()
                .find(|&k| format!("mark_{}", sim_core::landmark::slug(k)) == other)
                .unwrap_or_else(|| panic!("no kit named {other}"));
            sim_core::landmark::dump(kind, &mut out).unwrap()
        }
    }
    print!("{out}");
}
