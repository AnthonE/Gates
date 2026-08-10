//! Bot load: `cargo run -p server --bin bots -- <count> [addr] [secs]`
//! (CLAUDE.md commands; addr defaults to 127.0.0.1:4433). N wtransport
//! clients random-walking, aggregate line every 5 s, and each bot's
//! `BotReport` printed when its walk ends. Nice this process on shared
//! boxes — it exists to generate load.
//!
//! `secs` bounds the walk; omitted, a bot walks for a year, which is the
//! behaviour this binary has always had. It exists because the report was
//! the point: `run_bot` has always returned eleven per-client measurements
//! — snapshots applied, deltas, staleness, decode errors, missing
//! baselines, inputs, own updates, the widest entity set seen, the event
//! lane — and this binary threw every one of them away, so the 100-bot
//! soak `NOW.md` §0 asks for could be *run* and could not be *read*. A bot
//! that never stops never reports, hence the argument.

use server::botclient::{bot_endpoint, run_bot};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or_else(|| {
        eprintln!("usage: bots <count> [addr] [secs]");
        std::process::exit(1);
    });
    let server: SocketAddr = args
        .next()
        .unwrap_or_else(|| "127.0.0.1:4433".into())
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("bots: bad addr: {e}");
            std::process::exit(1);
        });
    // A load bot runs until the process dies unless told otherwise: a year,
    // which is this binary's original constant and stays the default so an
    // existing invocation behaves exactly as it did.
    let walk = args
        .next()
        .map(|a| {
            a.parse::<u64>().unwrap_or_else(|e| {
                eprintln!("bots: bad secs: {e}");
                std::process::exit(1);
            })
        })
        .map_or(Duration::from_secs(60 * 60 * 24 * 365), Duration::from_secs);

    let endpoint = match bot_endpoint() {
        Ok(e) => Arc::new(e),
        Err(e) => {
            eprintln!("bots: {e}");
            std::process::exit(1);
        }
    };

    let connected = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicU64::new(0));
    println!("bots: launching {count} against {server}");
    for i in 0..count {
        let endpoint = endpoint.clone();
        let connected = connected.clone();
        let failed = failed.clone();
        tokio::spawn(async move {
            connected.fetch_add(1, Ordering::Relaxed);
            match run_bot(&endpoint, server, i as u64, walk).await {
                Ok(r) => {
                    connected.fetch_sub(1, Ordering::Relaxed);
                    // One line per bot, integers only, the whole struct.
                    // Grep-shaped rather than pretty because the reader is a
                    // hundred of these at the end of a soak: `stale` and
                    // `nobase` against `snaps` are the client-side half of
                    // the shard's `snap_entities_shed` / `snap_candidates`,
                    // and `maxent` is what AOI actually delivered to one
                    // client — the number `NETCODE.md` §9's budgets are
                    // stated in and have never been measured against 100
                    // real connections.
                    println!(
                        "bot {i}: id {} · snaps {} (delta {}, stale {}, nobase {}) · \
                         decode err {} · inputs {} (exec {}) · own {} · maxent {} · \
                         events {} (err {})",
                        r.player_id,
                        r.snapshots_applied,
                        r.delta_snapshots,
                        r.stale_snapshots,
                        r.no_baseline,
                        r.decode_errors,
                        r.inputs_sent,
                        r.last_executed_seq,
                        r.own_updates,
                        r.max_entities_seen,
                        r.events_received,
                        r.event_decode_errors,
                    );
                }
                Err(e) => {
                    connected.fetch_sub(1, Ordering::Relaxed);
                    failed.fetch_add(1, Ordering::Relaxed);
                    eprintln!("bot {i}: {e}");
                }
            }
        });
        // Stagger handshakes; 50 at once is a TLS thundering herd.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut report = tokio::time::interval(Duration::from_secs(5));
    loop {
        report.tick().await;
        println!(
            "bots: {} walking, {} failed",
            connected.load(Ordering::Relaxed),
            failed.load(Ordering::Relaxed)
        );
    }
}
