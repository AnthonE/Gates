//! One local agent player. See crates/server/JEV.md for running it.

use server::agent_demo::{bot_name, Door, MindArgs, Source, AGENT_NAME, MIND_USAGE};
use server::explorer::Survivor;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const RUN_SECONDS: u64 = 120;
/// Most bots one run may start on one island. Plumbing bound.
const MAX_BOTS: usize = 8;

fn usage() -> String {
    format!(
        "jev-bot (--local | --server HOST:PORT) [--agent-key PATH] [--agent-name jev] [--cert-hash HEX] [--seconds 120] [--bots 1] {MIND_USAGE}\nThe bot survives in-game: it answers the death screen, crawls when wounded, gathers, crafts, eats and drinks. The process ends when the run ends or the connection fails.\nEvery bot declares itself an agent a spectator can follow (NETCODE.md §2.3). Without --agent-key it is a guest on a loopback shard; with it, a wallet agent that signs with the key in that 0600 file and may join any shard that admits it (§2.4). The key is only ever named by path.\n--bots 2..8 starts that many scripted or external guest agents on one island (each external agent is its own child); Jev and a wallet stay one bot."
    )
}

struct Options {
    local: bool,
    server: Option<String>,
    agent_key: Option<PathBuf>,
    agent_name: String,
    cert_hash: Option<String>,
    duration: Duration,
    bots: usize,
    mind: MindArgs,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut options = Self {
            local: false,
            server: None,
            agent_key: None,
            agent_name: AGENT_NAME.into(),
            cert_hash: None,
            duration: Duration::from_secs(RUN_SECONDS),
            bots: 1,
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
                // A name, not a resolved address: the certificate is checked
                // against the name dialled (`botclient::agent_endpoint`).
                "--server" => options.server = Some(args.next().ok_or("missing server address")?),
                // A path only: the key itself never rides argv (`agentkey`).
                "--agent-key" => {
                    options.agent_key = Some(PathBuf::from(
                        args.next().ok_or("--agent-key needs a file path")?,
                    ))
                }
                "--agent-name" => {
                    options.agent_name = args.next().ok_or("--agent-name needs a name")?
                }
                "--cert-hash" => {
                    options.cert_hash = Some(args.next().ok_or("--cert-hash needs a digest")?)
                }
                "--bots" => {
                    options.bots = args
                        .next()
                        .ok_or("--bots needs a count")?
                        .parse()
                        .map_err(|_| "invalid bot count")?;
                    if !(1..=MAX_BOTS).contains(&options.bots) {
                        return Err(format!("--bots must be 1..{MAX_BOTS}"));
                    }
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
        if options.local && options.cert_hash.is_some() {
            return Err("--cert-hash is for --server; a local shard pins its own".into());
        }
        if options.bots > 1 && options.mind.source == Source::Jev {
            return Err(
                "--bots above 1 runs scripted or external agents; Jev is billed per bot".into(),
            );
        }
        if options.bots > 1 && options.agent_key.is_some() {
            return Err("one --agent-key is one body; --bots above 1 runs guest agents".into());
        }
        for i in 0..options.bots {
            bot_name(&options.agent_name, i, options.bots)?;
        }
        Ok(Some(options))
    }
}

async fn run(options: Options) -> Result<(), String> {
    // Load the key (if any) and start every source before booting a shard:
    // a refused key or credential costs nothing.
    let key = options
        .agent_key
        .as_deref()
        .map(Door::load_key)
        .transpose()?;
    if key.is_none() {
        if let Some(server) = &options.server {
            // A guest must name a loopback shard; say so before anything runs.
            Door::new(server.clone(), None, None)?;
        }
    }
    let mut minds = Vec::with_capacity(options.bots);
    for _ in 0..options.bots {
        minds.push(options.mind.build()?);
    }
    println!("mind: {} × {}", options.mind.label(), options.bots);
    let shard = if options.local {
        let handle = server::agent_demo::spawn_local().await?;
        println!(
            "local shard: {} · proto {} · spectator seats open to this machine",
            handle.local_addr,
            protocol::PROTO_VER
        );
        Some(handle)
    } else {
        None
    };
    let door = match &shard {
        Some(h) => Door::new(h.local_addr.to_string(), Some(h.cert_hash.clone()), key)?,
        None => Door::new(
            options.server.clone().expect("validated destination"),
            options.cert_hash.clone(),
            key,
        )?,
    };
    println!(
        "{}: {}",
        if door.key.is_some() {
            "wallet agent"
        } else {
            "guest agent"
        },
        door.key
            .as_ref()
            .map(|k| format!("{:?}", k.address()))
            .unwrap_or_else(|| "unsigned, loopback only".into())
    );
    println!(
        "watch it (NETCODE.md §2.3; a loopback shard is this machine only): a Gates web page with ?{}",
        door.spectate_query()
    );
    println!("  or the desktop client: {}", door.desktop_command());
    let endpoint = Arc::new(door.endpoint()?);
    let door = Arc::new(door);
    let mut fleet = tokio::task::JoinSet::new();
    for (i, mind) in minds.into_iter().enumerate() {
        let endpoint = endpoint.clone();
        let door = door.clone();
        let duration = options.duration;
        let name = bot_name(&options.agent_name, i, options.bots)?;
        fleet.spawn(async move {
            let mut survivor = Survivor::new(mind);
            let result = door.play(&endpoint, name, duration, &mut survivor).await;
            (i, survivor, result)
        });
    }
    let mut outcomes = Vec::with_capacity(options.bots);
    let interrupted = tokio::select! {
        _ = async {
            while let Some(joined) = fleet.join_next().await {
                outcomes.push(joined);
            }
        } => false,
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|e| format!("Ctrl-C: {e}"))?;
            true
        }
    };
    // Closing the endpoint ends every bot's session promptly, so an
    // interrupted run still reports what each body did.
    endpoint.close(0u32.into(), b"agent finished");
    if interrupted {
        while let Some(joined) = fleet.join_next().await {
            outcomes.push(joined);
        }
        println!("interrupted by Ctrl-C");
    }
    if let Some(shard) = &shard {
        shard.shutdown.store(true, Ordering::Relaxed);
    }
    let mut failed = None;
    for joined in outcomes {
        let (i, survivor, result) = joined.map_err(|e| format!("bot task: {e}"))?;
        let label = if options.bots > 1 {
            format!("bot {i} ")
        } else {
            String::new()
        };
        print_report(&label, &survivor);
        match result {
            Ok(report) => {
                println!(
                    "{label}player {}: {} snapshots, {} inputs, {} actions, executed seq {}, decode errors {}, truncated {}",
                    report.player_id,
                    report.snapshots_applied,
                    report.inputs_sent,
                    report.actions_sent,
                    report.last_executed_seq,
                    report.decode_errors + report.event_decode_errors,
                    report.walk_truncated
                );
                if !interrupted
                    && (report.walk_truncated
                        || report.snapshots_applied == 0
                        || survivor.mind.stats.decisions == 0)
                {
                    failed = Some(format!(
                        "{label}run did not complete a working decision/snapshot loop"
                    ));
                }
            }
            Err(_) if interrupted => {}
            Err(e) => failed = Some(format!("{label}{e}")),
        }
    }
    failed.map_or(Ok(()), Err)
}

fn print_report(label: &str, survivor: &Survivor) {
    let (hour, hour_cap, day, day_cap) = survivor.mind.guard().counts();
    let m = survivor.mind.stats;
    println!(
        "{label}mind {}: {} requests, {} decisions, {} failures, {} late, {} input tokens, {} output tokens, {} pauses; this hour {hour}/{hour_cap}, today {day}/{day_cap}",
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
        "{label}survival: {} deaths, {} respawns ({} asks), {} retreats; goals {} done, {} failed, {} interrupted; {} targets finished, {} abandoned; crafted {}, eaten {}, drinks {}, equips {}; {} actions sent",
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
        println!("{label}gathered: {}", gathered.join(", "));
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
            "{label}pack: {}; health {}/{}, food {}/{}, water {}/{}",
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
    println!("{label}recent goals: {}", goals.join(" | "));
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
