//! One local agent player. See crates/server/JEV.md for running it.

use server::agent_demo::{MindArgs, MIND_USAGE};
use server::botclient::{bot_endpoint, run_driven_bot};
use server::explorer::Survivor;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

const RUN_SECONDS: u64 = 120;

fn usage() -> String {
    format!(
        "jev-bot (--local | --server 127.0.0.1:PORT) [--seconds 120] {MIND_USAGE}\nThe bot survives in-game: it answers the death screen, crawls when wounded, gathers, crafts, eats and drinks. The process ends when the run ends or the connection fails."
    )
}

struct Options {
    local: bool,
    server: Option<SocketAddr>,
    duration: Duration,
    mind: MindArgs,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut options = Self {
            local: false,
            server: None,
            duration: Duration::from_secs(RUN_SECONDS),
            mind: MindArgs::default(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if options.mind.take(&arg, &mut args)? {
                continue;
            }
            match arg.as_str() {
                "--help" | "-h" => return Ok(None),
                "--local" => options.local = true,
                "--server" => {
                    options.server = Some(
                        args.next()
                            .ok_or("missing server address")?
                            .parse()
                            .map_err(|_| "invalid server address")?,
                    )
                }
                "--seconds" => {
                    let value: u64 = args
                        .next()
                        .ok_or("missing duration")?
                        .parse()
                        .map_err(|_| "invalid duration")?;
                    if !(1..=86400).contains(&value) {
                        return Err("run duration must be 1..86400 seconds".into());
                    }
                    options.duration = Duration::from_secs(value);
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
    // Check credentials and start the source before booting a shard.
    let mind = options.mind.build()?;
    println!("mind: {}", options.mind.label());
    let mut survivor = Survivor::new(mind);
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
    let result = tokio::select! {
        result = run_driven_bot(&endpoint, address, options.duration, &mut survivor) => result.map(Some),
        signal = tokio::signal::ctrl_c() => signal.map(|_| None).map_err(|e| format!("Ctrl-C: {e}")),
    };
    endpoint.close(0u32.into(), b"agent finished");
    if let Some(shard) = &shard {
        shard.shutdown.store(true, Ordering::Relaxed);
    }
    print_report(&survivor);
    if let Some(report) = result? {
        println!(
            "player {}: {} snapshots, {} inputs, {} actions, executed seq {}, decode errors {}, truncated {}",
            report.player_id,
            report.snapshots_applied,
            report.inputs_sent,
            report.actions_sent,
            report.last_executed_seq,
            report.decode_errors + report.event_decode_errors,
            report.walk_truncated
        );
        if report.walk_truncated
            || report.snapshots_applied == 0
            || survivor.mind.stats.decisions == 0
        {
            return Err("run did not complete a working decision/snapshot loop".into());
        }
    }
    Ok(())
}

fn print_report(survivor: &Survivor) {
    let (hour, hour_cap, day, day_cap) = survivor.mind.guard().counts();
    let m = survivor.mind.stats;
    println!(
        "mind {}: {} requests, {} decisions, {} failures, {} late, {} input tokens, {} output tokens, {} pauses; this hour {hour}/{hour_cap}, today {day}/{day_cap}",
        survivor.mind.kind().label(),
        m.requests,
        m.decisions,
        m.failures,
        m.late,
        m.input_tokens,
        m.output_tokens,
        m.pauses
    );
    let s = survivor.stats;
    println!(
        "survival: {} deaths, {} respawns ({} asks), {} retreats; goals {} done, {} failed, {} interrupted; {} targets finished, {} abandoned; crafted {}, eaten {}, drinks {}, equips {}; {} actions sent",
        s.deaths,
        s.respawns,
        s.respawn_asks,
        s.retreats,
        s.goals_done,
        s.goals_failed,
        s.goals_interrupted,
        s.targets_completed,
        s.targets_abandoned,
        s.crafted,
        s.eaten,
        s.drinks,
        s.equips,
        s.actions
    );
    if let Some(core) = survivor.core() {
        let gathered: Vec<String> = (0..usize::from(core.catalog.count))
            .filter(|&i| survivor.gathered.get(i).is_some_and(|&n| n > 0))
            .map(|i| {
                format!(
                    "{} {}",
                    String::from_utf8_lossy(core.catalog.name(i)),
                    survivor.gathered[i]
                )
            })
            .collect();
        println!("gathered: {}", gathered.join(", "));
        let mut pack: Vec<(String, u32)> = Vec::new();
        for stack in core.inv.iter().filter(|s| s.count > 0) {
            let name = String::from_utf8_lossy(core.catalog.name(stack.item as usize)).into_owned();
            match pack.iter_mut().find(|(n, _)| *n == name) {
                Some(entry) => entry.1 += u32::from(stack.count),
                None => pack.push((name, u32::from(stack.count))),
            }
        }
        let pack: Vec<String> = pack.iter().map(|(n, c)| format!("{n} {c}")).collect();
        println!(
            "pack: {}; health {}/{}, food {}/{}, water {}/{}",
            pack.join(", "),
            core.hp,
            core.hp_max,
            core.food,
            core.max_food,
            core.water,
            core.max_water
        );
    }
    let goals: Vec<String> = survivor
        .history
        .iter()
        .map(|r| {
            format!(
                "{} {}{} (+{}, {} s)",
                r.goal.label().as_str(),
                r.outcome.word(),
                r.outcome
                    .why()
                    .map(|w| format!(": {}", w.text()))
                    .unwrap_or_default(),
                r.gained,
                r.secs
            )
        })
        .collect();
    println!("recent goals: {}", goals.join(" | "));
}

#[tokio::main]
async fn main() {
    let result = match Options::parse() {
        Ok(Some(options)) => run(options).await,
        Ok(None) => {
            println!("{}", usage());
            return;
        }
        Err(e) => Err(format!("{e}\n{}", usage())),
    };
    if let Err(error) = result {
        eprintln!("jev-bot: {error}");
        std::process::exit(1);
    }
}
