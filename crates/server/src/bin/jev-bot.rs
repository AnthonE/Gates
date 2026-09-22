//! One peaceful local explorer. See crates/server/JEV.md for running it.

use server::agent_demo::Scripted;
use server::botclient::{bot_endpoint, run_driven_bot};
use server::explorer::Gatherer;
use server::jev::{Driver, Jev, REQUEST_TIMEOUT, THINK_INTERVAL};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

const RUN_SECONDS: u64 = 120;
const USAGE: &str = "jev-bot (--local | --server 127.0.0.1:PORT) [--scripted] [--gather-wood] [--seconds 120] [--think-ms 1000] [--timeout-ms 3000]\nJev mode requires TYPESAFE_API_KEY. --scripted is an offline demo. --gather-wood adds the local harvesting skill.";

struct Options {
    local: bool,
    server: Option<SocketAddr>,
    scripted: bool,
    gather_wood: bool,
    duration: Duration,
    interval: Duration,
    timeout: Duration,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut options = Self {
            local: false,
            server: None,
            scripted: false,
            gather_wood: false,
            duration: Duration::from_secs(RUN_SECONDS),
            interval: THINK_INTERVAL,
            timeout: REQUEST_TIMEOUT,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => return Ok(None),
                "--local" => options.local = true,
                "--scripted" => options.scripted = true,
                "--gather-wood" => options.gather_wood = true,
                "--server" => {
                    options.server = Some(
                        args.next()
                            .ok_or("missing server address")?
                            .parse()
                            .map_err(|_| "invalid server address")?,
                    )
                }
                "--seconds" | "--think-ms" | "--timeout-ms" => {
                    let value: u64 = args
                        .next()
                        .ok_or("missing duration")?
                        .parse()
                        .map_err(|_| "invalid duration")?;
                    if value == 0 {
                        return Err("durations must be positive".into());
                    }
                    match arg.as_str() {
                        "--seconds" if value <= 86400 => {
                            options.duration = Duration::from_secs(value)
                        }
                        "--think-ms" if value <= 60000 => {
                            options.interval = Duration::from_millis(value)
                        }
                        "--timeout-ms" if value <= 60000 => {
                            options.timeout = Duration::from_millis(value)
                        }
                        _ => {
                            return Err(
                                "run limit is 24 hours; interval/timeout limit is 60 seconds"
                                    .into(),
                            )
                        }
                    }
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }
        if options.local == options.server.is_some() {
            return Err("choose exactly one of --local and --server".into());
        }
        if options.server.is_some_and(|a| !a.ip().is_loopback()) {
            return Err("this guest prototype only joins loopback shards".into());
        }
        Ok(Some(options))
    }
}

async fn run(options: Options) -> Result<(), String> {
    // Check credentials before booting a shard or opening a player session.
    let driver = if options.scripted {
        println!("mode: SCRIPTED exploration; no model calls");
        Driver::new(Scripted, options.interval, options.timeout)?
    } else {
        let key = std::env::var("TYPESAFE_API_KEY")
            .map_err(|_| "set TYPESAFE_API_KEY, or use --scripted for the offline demo")?;
        println!("mode: Jev {}", server::jev::MODEL);
        Driver::new(
            Jev::new(key, options.timeout)?,
            options.interval,
            options.timeout,
        )?
    };
    let mut driver = Gatherer::new(driver);
    if options.gather_wood {
        println!("goal: collect wood using visible trees and ordinary player inputs");
    }
    let shard = if options.local {
        let handle = server::agent_demo::spawn_local().await?;
        println!(
            "local shard: {} · proto {}",
            handle.local_addr,
            protocol::PROTO_VER
        );
        println!("watch with the matching client: cargo run -p client --features render --bin gates -- --server https://{} --cert-hash {}", handle.local_addr, handle.cert_hash);
        Some(handle)
    } else {
        None
    };
    let address = shard
        .as_ref()
        .map(|s| s.local_addr)
        .or(options.server)
        .expect("validated destination");
    let endpoint = bot_endpoint()?;
    let controller: &mut dyn server::botclient::BotDriver = if options.gather_wood {
        &mut driver
    } else {
        &mut driver.explorer
    };
    let result = tokio::select! {
        result = run_driven_bot(&endpoint, address, options.duration, controller) => result.map(Some),
        signal = tokio::signal::ctrl_c() => signal.map(|_| None).map_err(|e| format!("Ctrl-C: {e}")),
    };
    endpoint.close(0u32.into(), b"agent finished");
    if let Some(shard) = &shard {
        shard.shutdown.store(true, Ordering::Relaxed);
    }
    println!("exploration: {:?}", driver.explorer.stats);
    if options.gather_wood {
        println!("gathering: {:?}", driver.stats);
    }
    if let Some(report) = result? {
        println!(
            "player {}: {} snapshots, {} inputs, executed seq {}, decode errors {}, truncated {}",
            report.player_id,
            report.snapshots_applied,
            report.inputs_sent,
            report.last_executed_seq,
            report.decode_errors,
            report.walk_truncated
        );
        if report.walk_truncated
            || report.snapshots_applied == 0
            || (driver.explorer.stats.decisions == 0 && driver.stats.wood_gained == 0)
        {
            return Err("run did not complete a working decision/snapshot loop".into());
        }
        if options.gather_wood && driver.stats.wood_gained == 0 {
            return Err("collect-wood run ended without a confirmed inventory gain".into());
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let result = match Options::parse() {
        Ok(Some(options)) => run(options).await,
        Ok(None) => {
            println!("{USAGE}");
            return;
        }
        Err(e) => Err(format!("{e}\n{USAGE}")),
    };
    if let Err(error) = result {
        eprintln!("jev-bot: {error}");
        std::process::exit(1);
    }
}
