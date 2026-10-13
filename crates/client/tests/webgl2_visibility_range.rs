//! Gate: no entity carries a `VisibilityRange` in the WebGL2 browser module.
//!
//! A mesh with one makes Bevy's vertex shader read the visibility-range table
//! at view binding 14. WebGL2 has no storage buffers, so the shader declares
//! it a 1024-byte uniform array, while the layout entry says 16 bytes, and
//! wgpu refuses `pbr_opaque_mesh_pipeline` at creation: the page panics the
//! moment such a mesh is in view. It shipped twice — the trees (fixed by
//! `tree::lod_band`) and then the ring's power poles (`poles::fade_band`) —
//! and every other gate is green over it, because only a browser running
//! WebGL2 ever builds that pipeline.
//!
//! The rule, as a source scan: every `VisibilityRange { … }` literal in
//! `src/` sits in a fn behind `#[cfg(not(webgl2))]` or
//! `#[cfg(not(target_arch = "wasm32"))]`, or in a fn named in [`VALUES_ONLY`].

use std::path::{Path, PathBuf};

/// Fns that build `VisibilityRange` values that never reach an entity
/// directly, with the reason. Adding a row is a claim someone has to check.
const VALUES_ONLY: &[(&str, &str, &str)] = &[(
    "render/tree.rs",
    "bands",
    "TreeLod's two bands; an entity gets them only through `lod_band`, \
     which is nothing under webgl2",
)];

const GUARDS: &[&str] = &[
    "#[cfg(not(webgl2))]",
    "#[cfg(not(target_arch = \"wasm32\"))]",
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read src") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The name of a `fn` declared on this line, if one is.
fn fn_name(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let t = t
        .strip_prefix("pub(crate) ")
        .or_else(|| t.strip_prefix("pub "))
        .unwrap_or(t);
    let t = t.strip_prefix("fn ")?;
    Some(&t[..t.find(|c: char| !(c.is_alphanumeric() || c == '_'))?])
}

/// Every literal in `src/` as (file, line, enclosing fn, guarded).
fn literals() -> Vec<(String, usize, String, bool)> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    let mut out = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).expect("read file");
        let lines: Vec<&str> = text.lines().collect();
        let rel = f
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for (i, l) in lines.iter().enumerate() {
            let code = l.split("//").next().unwrap_or("");
            // A literal, not a return type (`-> VisibilityRange {`).
            if !code.contains("VisibilityRange {") || code.contains("->") {
                continue;
            }
            let Some(at) = (0..=i).rev().find(|&k| fn_name(lines[k]).is_some()) else {
                out.push((rel.clone(), i + 1, String::new(), false));
                continue;
            };
            // The attribute and doc lines directly above the fn.
            let guarded = lines[..at]
                .iter()
                .rev()
                .take_while(|a| {
                    let a = a.trim_start();
                    a.starts_with("#[") || a.starts_with("//")
                })
                .any(|a| GUARDS.iter().any(|g| a.contains(g)));
            out.push((
                rel.clone(),
                i + 1,
                fn_name(lines[at]).unwrap().to_string(),
                guarded,
            ));
        }
    }
    out
}

#[test]
fn no_visibility_range_reaches_a_webgl2_entity() {
    let bad: Vec<_> = literals()
        .into_iter()
        .filter(|(file, _, func, guarded)| {
            !guarded && !VALUES_ONLY.iter().any(|(f, n, _)| f == file && n == func)
        })
        .map(|(file, line, func, _)| format!("src/{file}:{line} in fn {func}"))
        .collect();
    assert!(
        bad.is_empty(),
        "a VisibilityRange literal outside a #[cfg(not(webgl2))] fn — under WebGL2 \
         the pipeline panics the moment the mesh is in view. Route it through a \
         cfg-split helper like plants::fade_band or poles::fade_band:\n  {}",
        bad.join("\n  ")
    );
}

/// The scan is not aimed at nothing: it sees the guarded helpers it exists
/// for, and the values-only fn it excuses.
#[test]
fn the_scan_still_finds_the_known_sites() {
    let found = literals();
    for (file, func) in [
        ("render/plants.rs", "fade_band"),
        ("render/poles.rs", "fade_band"),
        ("render/tree.rs", "bands"),
    ] {
        assert!(
            found.iter().any(|(f, _, n, _)| f == file && n == func),
            "the scan no longer sees the VisibilityRange literal in {file}::{func}"
        );
    }
    assert!(
        found
            .iter()
            .any(|(f, _, n, g)| f == "render/poles.rs" && n == "fade_band" && *g),
        "poles::fade_band reads as unguarded — the attribute scan is broken"
    );
}
