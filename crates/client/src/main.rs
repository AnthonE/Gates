//! `cargo run -p client -- [addr]` — the native client, headless for now.
//!
//! Slice 1 of the desktop client (DECISIONS.md 2026-08-05) and deliberately
//! the unglamorous half: connect over the real wire, drive the real
//! `ClientCore`, and report what the world says. No window and no renderer
//! yet, because the risky claim is not "Rust can draw a triangle" — it is
//! that the transport, the handshake, the snapshot lane, the event lane and
//! the predictor all run natively against an unmodified shard. That is what
//! this proves, and it proves it in a few hundred lines because the client
//! core was already pure Rust.
//!
//! What it is NOT: a gate. It needs a live shard, so it cannot run headless
//! in CI as it stands — `bot_smoke` is the gate that covers this transport
//! path, and the native session gets its own gate when it has a renderer
//! worth asserting on.

use client::args::{self, Parsed};
use client::scry::Scry;
use client::{client_endpoint, Session};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let parsed = args::parse(std::env::args().skip(1));
    let a = match parsed {
        Parsed::Run(a) => a,
        Parsed::Help => {
            println!("{}", args::USAGE);
            return;
        }
        Parsed::Bad(why) => {
            eprintln!("client: {why}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    let server = a.server.clone();

    // Who is playing. No launcher is a normal state and this line says which
    // it was, because an address off a launcher and an address off the command
    // line are equally unverified but not equally informative.
    if !a.no_launcher {
        let scry = Scry::discover(a.identity.as_deref(), env!("CARGO_PKG_VERSION"));
        println!("client: {}", scry.player.line());
    } else if let Some(id) = a.identity.as_deref() {
        println!("client: identity {id} (declared, unverified; launcher skipped)");
    }

    let endpoint = match client_endpoint() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("client: {e}");
            std::process::exit(1);
        }
    };

    let token = match a.token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("client: {e}");
            std::process::exit(1);
        }
    };
    println!("client: connecting to {server}");
    let mut session = match Session::connect(&endpoint, &server, token).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("client: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "client: in the world — player {} seed {} tick {}",
        session.welcome.player_id, session.welcome.seed, session.welcome.tick
    );

    // 60 Hz pump. The core owns the 30 Hz sim tick; this is only how often
    // wall time is handed to it, which is what a renderer's frame loop will
    // do once there is one.
    let mut last = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_millis(16));
    let mut reported = Instant::now();
    loop {
        ticker.tick().await;
        let now = Instant::now();
        let dt_ms = now.duration_since(last).as_secs_f64() * 1000.0;
        last = now;

        let frame = session.pump(dt_ms);

        if now.duration_since(reported) >= Duration::from_secs(2) {
            reported = now;
            // `steps` is how many 30 Hz sim steps this 60 Hz frame drove —
            // 0 or 1 is the correct reading, not a stalled clock.
            println!("  steps {} · snapshots {}", frame.tick, frame.snapshots);
        }
    }
}
