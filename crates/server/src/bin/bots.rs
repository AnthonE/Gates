//! Bot load: `cargo run -p server --bin bots -- <count> [addr]`
//! (CLAUDE.md commands; addr defaults to 127.0.0.1:4433). N wtransport
//! clients random-walking forever, aggregate line every 5 s. Nice this
//! process on shared boxes — it exists to generate load.

use server::botclient::{bot_endpoint, run_bot};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or_else(|| {
        eprintln!("usage: bots <count> [addr]");
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
            // A load bot runs until the process dies.
            let forever = Duration::from_secs(60 * 60 * 24 * 365);
            connected.fetch_add(1, Ordering::Relaxed);
            match run_bot(&endpoint, server, i as u64, forever).await {
                Ok(_) => {}
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
