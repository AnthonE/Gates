//! `cfg(webgl2)`: the browser module that draws through WebGL2.
//!
//! A page ships two modules (`ci/build_web.sh`): one built with the `webgpu`
//! feature, which a browser with WebGPU loads, and one without it, which every
//! other browser loads. They are the same platform — one thread, no sockets,
//! the page owns the join — so everything about being in a tab stays
//! `target_arch = "wasm32"`. What differs is the GPU API, and Bevy decides
//! that at compile time: under `webgpu` it drops every WebGL workaround it
//! has (`all(feature = "webgl", target_arch = "wasm32", not(feature =
//! "webgpu"))`, in a dozen places). This names the same set once, so a
//! downgrade that exists because WebGL2 cannot do something is spelled
//! `cfg(webgl2)` and the WebGPU module takes the desktop's path instead.
fn main() {
    // The target and the features already key this unit; without a line
    // like this cargo reruns the script on every change under the crate.
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(webgl2)");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let webgpu = std::env::var_os("CARGO_FEATURE_WEBGPU").is_some();
    if wasm && !webgpu {
        println!("cargo::rustc-cfg=webgl2");
    }
    windows_icon();
}

/// The Gates sigil as the Windows .exe's own icon (`icon/gates.rc`). Same
/// hand-rolled shape as the elo launcher's `build.rs`: one subprocess to
/// whatever resource compiler is on the box, and a warning, never a failed
/// build, when there is none. The running window sets its icon at runtime
/// either way (`bin/gates.rs`, `window_icon`).
fn windows_icon() {
    use std::path::PathBuf;
    use std::process::Command;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icon");
    let rc = dir.join("gates.rc");
    println!("cargo::rerun-if-changed={}", rc.display());
    println!(
        "cargo::rerun-if-changed={}",
        dir.join("gates.ico").display()
    );
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(if msvc {
        "gates.res"
    } else {
        "gates.o"
    });
    let (rc, out_s) = (rc.display().to_string(), out.display().to_string());
    let tools: Vec<(&str, Vec<String>)> = if msvc {
        vec![
            (
                "rc.exe",
                vec!["/nologo".into(), "/fo".into(), out_s.clone(), rc.clone()],
            ),
            ("llvm-rc", vec![format!("/fo{out_s}"), rc.clone()]),
        ]
    } else {
        let args = vec![
            rc.clone(),
            "-O".into(),
            "coff".into(),
            "-o".into(),
            out_s.clone(),
        ];
        vec![
            ("x86_64-w64-mingw32-windres", args.clone()),
            ("windres", args),
        ]
    };
    for (tool, args) in tools {
        // The .rc names its .ico by bare filename, so run it from `icon/`.
        match Command::new(tool).args(&args).current_dir(&dir).status() {
            Ok(s) if s.success() => {
                println!("cargo::rustc-link-arg-bins={out_s}");
                return;
            }
            Ok(s) => println!("cargo::warning={tool} failed ({s}); the .exe gets the default icon"),
            Err(_) => continue,
        }
    }
    println!("cargo::warning=no resource compiler (rc.exe, llvm-rc, windres); the .exe gets the default icon");
}
