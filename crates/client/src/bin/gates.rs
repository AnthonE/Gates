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
//!
//! `--server` and `--identity` are how the **scry launcher** starts this
//! binary — a depot's launch block names them (`ci/depot.py`). Parsing lives
//! in `client::args` so the two binaries cannot disagree about a flag.

use bevy::prelude::*;
use client::args::{self, Parsed};
use client::render::boot::Who;
use client::render::{GatesRenderPlugin, Net, Rt, Start, WorldId};
use client::scry::Player;
use client::{client_endpoint, Session};

fn main() -> AppExit {
    let a = match args::parse(std::env::args().skip(1)) {
        Parsed::Run(a) => a,
        Parsed::Help => {
            println!("{}", args::USAGE);
            return AppExit::Success;
        }
        Parsed::Bad(why) => {
            eprintln!("gates: {why}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    let server = a.server.clone();
    let capture = a.capture.clone();
    // **Who connects before the window, and who does not.**
    //
    // Only `--capture` does, now. The probe harness is a gate: a client that
    // draws a world it is not connected to lies for its first few frames, and
    // a harness that could photograph a half-finished handshake is a harness
    // whose frames depend on the network. So its connect stays a hard
    // precondition here.
    //
    // **A launcher join used to be the other one, and that was the defect.**
    // `--server` meant the player had already chosen, so the client connected
    // here and `exit(1)`'d on failure — into a terminal a double-clicked game
    // does not have, after a black window that had not opened yet. It goes
    // through `Screen::Boot` now: the window comes up first, the splash says
    // what it is doing, and the connect runs in `Screen::Connecting`, which
    // has owned a timeout, an Esc and a failure arm that names the reason
    // since it existed. The player is still not asked to choose twice —
    // `chosen` carries that — they are just allowed to survive a dead shard.
    let straight_in = capture.is_some();

    // The launcher handshake used to be here too, before the window, because
    // it is a blocking round trip over a local socket and must never happen
    // inside a frame. It still must not; it happens on a thread from the boot
    // splash instead (`render/boot.rs`), which is the same rule with a window
    // already up. A capture run resolves it here and asks nobody, because a
    // gate that reaches for a socket outside the repo is a gate whose result
    // depends on what else is running on the box.
    let who = match a.identity.as_deref() {
        Some(id) => Player::Declared(id.to_string()),
        None => Player::Anonymous,
    };
    if straight_in || a.no_launcher {
        println!("gates: {}", who.line());
    }

    let address = match a.address() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("gates: {e}");
            std::process::exit(1);
        }
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let session = straight_in.then(|| {
        rt.block_on(async {
            let endpoint = client_endpoint(&server, a.cert_hash.as_deref()).unwrap_or_else(|e| {
                eprintln!("gates: {e}");
                std::process::exit(1);
            });
            Session::connect(&endpoint, &server, address, client::scry::sign_siwe)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("gates: {e}");
                    std::process::exit(1);
                })
        })
    });
    if let Some(s) = &session {
        println!(
            "gates: in the world — player {} seed {} tick {}",
            s.welcome.player_id, s.welcome.seed, s.welcome.tick
        );
    }

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
    app.insert_resource(Who(who));
    // The runtime outlives every session, which is why it is its own
    // resource now: on the menu path there is no session yet to hold it.
    app.insert_non_send_resource(Rt(rt));

    if let Some(session) = session {
        app.insert_resource(WorldId::new(session.welcome.seed));
        app.insert_non_send_resource(Net { session, sel: 0 });
    }
    let start = Start {
        direct: server,
        servers_url: a.servers_url.clone(),
        connected: straight_in,
        // The bit the menu turns on: an address the launcher chose must not be
        // second-guessed by a screen asking the player to choose again. It is
        // separate from `connected` now, because the two stopped being the
        // same thing the moment the launcher's connect became a state.
        chosen: straight_in || a.server_given,
        identity: a.identity.clone(),
        no_launcher: a.no_launcher,
        no_hud: a.no_hud,
    };
    app.add_plugins(GatesRenderPlugin { start, capture });
    // **Returned, not discarded.** `App::run` hands back an `AppExit`, which
    // implements `Termination` — dropping it means a capture run that failed
    // its own file check still exits 0, and a gate reading that exit code
    // would call it a pass. Measured: with the return discarded, a capture
    // that wrote five of six vantages printed "1 of 6 vantages did not reach
    // disk" and exited 0 anyway.
    app.run()
}
