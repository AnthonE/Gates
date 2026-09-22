//! `trust-log`: read the trust ledger a shard wrote (`server::trustlog`).
//!
//! ```text
//! trust-log <dir> [--wallet ADDR] [--player ID] [--as actor|counterparty]
//!                 [--verb door|auth|cont] [--presence awake|asleep|gone]
//!                 [--from TICK] [--to TICK] [--boot HEX]
//!                 [--summary | --jsonl]
//! ```
//!
//! `<dir>` is the `<world_file>.trust` directory beside a shard's world
//! file. Rows print in log order, which is the order they happened. The
//! summary counts rows **per verb** and per verb × presence over whatever
//! the filters selected.
//!
//! There is no per-player total and no ordering of players by anything, and
//! that is deliberate (`PLAYERS.md` wall 2: no global leaderboard). A
//! player's own history is `--wallet` or `--player`, printed in log order.

use server::trustlog::{read_dir, summarize, Filter, LogParty, LogRow, Role};
use std::path::PathBuf;

const USAGE: &str = "usage: trust-log <dir> [--wallet ADDR] [--player ID] \
[--as actor|counterparty] [--verb door|auth|cont] [--presence awake|asleep|gone] \
[--from TICK] [--to TICK] [--boot HEX] [--summary | --jsonl]";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Rows,
    Summary,
    Jsonl,
}

fn main() {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("trust-log: {e}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut dir: Option<PathBuf> = None;
    let mut f = Filter::default();
    let mut mode = Mode::Rows;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--wallet" => f.wallet = Some(val("--wallet")?),
            "--player" => {
                f.player = Some(
                    val("--player")?
                        .parse()
                        .map_err(|_| "--player takes a number")?,
                )
            }
            "--as" => {
                f.role = match val("--as")?.as_str() {
                    "actor" => Role::Actor,
                    "counterparty" => Role::Counterparty,
                    other => return Err(format!("--as {other}: actor or counterparty")),
                }
            }
            "--verb" => f.verb = Some(val("--verb")?),
            "--presence" => f.presence = Some(val("--presence")?),
            "--from" => {
                f.from_tick = Some(val("--from")?.parse().map_err(|_| "--from takes a tick")?)
            }
            "--to" => f.to_tick = Some(val("--to")?.parse().map_err(|_| "--to takes a tick")?),
            "--boot" => f.boot = Some(val("--boot")?),
            "--summary" => mode = Mode::Summary,
            "--jsonl" => mode = Mode::Jsonl,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            s => {
                if dir.replace(PathBuf::from(s)).is_some() {
                    return Err("one directory only".into());
                }
            }
        }
    }
    let dir = dir.ok_or("no log directory given")?;
    let log = read_dir(&dir)?;
    let rows: Vec<&LogRow> = log.rows.iter().filter(|r| f.matches(r)).collect();
    match mode {
        Mode::Rows => {
            for r in &rows {
                println!("{}", row_line(r));
            }
        }
        Mode::Jsonl => {
            for r in &rows {
                println!("{}", row_json(r));
            }
        }
        Mode::Summary => {}
    }
    let s = summarize(rows.iter().copied());
    println!(
        "-- {} of {} rows · {} segments ({} closed, {} torn) · {} gap lines ({} rows lost) · {} segments pruned",
        s.rows,
        log.rows.len(),
        log.segments.len(),
        log.segments.iter().filter(|x| x.closed).count(),
        log.segments.iter().filter(|x| x.torn).count(),
        log.gaps.len(),
        log.gaps
            .iter()
            .map(|g| g.ring_full + g.sim_overflow)
            .sum::<u64>(),
        log.pruned.len(),
    );
    for (verb, n) in &s.by_verb {
        let split: Vec<String> = s
            .by_verb_presence
            .iter()
            .filter(|((v, _), _)| v == verb)
            .map(|((_, p), k)| format!("{p} {k}"))
            .collect();
        println!("   {verb:<5} {n:>8}   ({})", split.join(", "));
    }
    Ok(())
}

fn who(p: &LogParty) -> String {
    match (&p.wallet, p.guest) {
        (Some(w), _) => format!("{} {w}", p.id),
        (None, true) => format!("{} guest", p.id),
        (None, false) => format!("{} unresolved", p.id),
    }
}

fn row_line(r: &LogRow) -> String {
    format!(
        "tick {:>10}  {:<4}  owner {:<6}  actor {}  ->  counterparty {}",
        r.tick,
        r.verb,
        r.presence,
        who(&r.actor),
        who(&r.counterparty)
    )
}

fn row_json(r: &LogRow) -> String {
    let party = |p: &LogParty| {
        serde_json::json!({ "id": p.id, "wallet": p.wallet, "guest": p.guest })
    };
    serde_json::json!({
        "segment": r.segment,
        "boot": r.boot,
        "tick": r.tick,
        "verb": r.verb,
        "presence": r.presence,
        "actor": party(&r.actor),
        "counterparty": party(&r.counterparty),
    })
    .to_string()
}
