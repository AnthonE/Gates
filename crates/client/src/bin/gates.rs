//! `cargo run -p client --features render --bin gates -- [addr] [--capture <dir>]`
//! — the desktop client with a window on it (`RENDER.md`).
//!
//! **Bevy draws; it does not decide.** `sim-core` is the authority and carries
//! the walls — zero allocation in the tick, no `HashMap` iteration, replay
//! determinism — and `ClientCore` owns prediction and interpolation. Every
//! system under `client::render` reads those and writes transforms. The day
//! world state starts living in Bevy components is the day those walls stop
//! meaning anything, and nothing in CI would catch it, so the boundary is a
//! rule rather than a preference: **no gameplay state in the ECS.**
//!
//! `--capture <dir>` runs the probe harness instead of a player: settle on
//! observable state, warm the pipelines, shoot a fixed vantage list, exit.

use bevy::prelude::*;
use client::render::{GatesRenderPlugin, Net};
use client::{client_endpoint, Session};
use std::net::SocketAddr;
use std::path::PathBuf;

fn main() -> AppExit {
    let mut addr = String::from("127.0.0.1:4433");
    let mut capture: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--capture" => {
                capture = Some(PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("gates: --capture needs a directory");
                    std::process::exit(1);
                })));
            }
            other => addr = other.to_string(),
        }
    }
    let server: SocketAddr = addr.parse().unwrap_or_else(|e| {
        eprintln!("gates: bad addr: {e}");
        std::process::exit(1);
    });

    // Connect before the window opens. A client that draws a world it is not
    // connected to is a client that lies for its first few frames.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let session = rt.block_on(async {
        let endpoint = client_endpoint().unwrap_or_else(|e| {
            eprintln!("gates: {e}");
            std::process::exit(1);
        });
        Session::connect(&endpoint, server)
            .await
            .unwrap_or_else(|e| {
                eprintln!("gates: {e}");
                std::process::exit(1);
            })
    });
    let seed = session.welcome.seed;
    println!(
        "gates: in the world — player {} seed {} tick {}",
        session.welcome.player_id, seed, session.welcome.tick
    );

    let mut app = App::new();
    // **The asset root is the executable's directory, not the working one.**
    // Bevy resolved `textures/rock_albedo.jpg` to `target/debug/assets/...`
    // and logged `Path not found` — while the renderer carried on drawing a
    // white fallback, so three successive material changes measured
    // BYTE-IDENTICAL statistics and looked like physics. That is the shape of
    // this whole class: a missing texture is not an error the image shows you.
    //
    // Resolved here rather than by a symlink so the same binary works from a
    // repo checkout and from a shipped layout with `assets/` beside the exe.
    let mut assets = AssetPlugin::default();
    if let Ok(cwd) = std::env::current_dir() {
        let repo = cwd.join("assets");
        if repo.is_dir() {
            assets.file_path = repo.to_string_lossy().into_owned();
        }
    }
    app.add_plugins(DefaultPlugins.set(assets));
    app.insert_non_send_resource(Net {
        _rt: rt,
        session,
        sel: 0,
    });
    app.add_plugins(GatesRenderPlugin { seed, capture });
    // **Returned, not discarded.** `App::run` hands back an `AppExit`, which
    // implements `Termination` — dropping it means a capture run that failed
    // its own file check still exits 0, and a gate reading that exit code
    // would call it a pass. Measured: with the return discarded, a capture
    // that wrote five of six vantages printed "1 of 6 vantages did not reach
    // disk" and exited 0 anyway.
    app.run()
}
