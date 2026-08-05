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

fn main() {
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
    app.add_plugins(DefaultPlugins);
    app.insert_non_send_resource(Net {
        _rt: rt,
        session,
        sel: 0,
    });
    app.add_plugins(GatesRenderPlugin { seed, capture });
    app.run();
}
