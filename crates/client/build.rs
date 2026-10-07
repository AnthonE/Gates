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
    println!("cargo::rustc-check-cfg=cfg(webgl2)");
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let webgpu = std::env::var_os("CARGO_FEATURE_WEBGPU").is_some();
    if wasm && !webgpu {
        println!("cargo::rustc-cfg=webgl2");
    }
}
