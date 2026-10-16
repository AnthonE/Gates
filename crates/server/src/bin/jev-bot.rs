//! One local agent player. See crates/server/JEV.md for running it.

use server::agent::society::{self, Board, Kin, Role, Society};
use server::agent_demo::{bot_name, check_page, Door, MindArgs, Source, AGENT_NAME, MIND_USAGE};
use server::explorer::Survivor;
use sim_core::limits::TICK_HZ;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const RUN_SECONDS: u64 = 120;
/// An MCP harness's run: the longest `--seconds` allows. The harness ends
/// it sooner by closing its end.
const MCP_RUN_SECONDS: u64 = 86400;
/// Where `--set-face` signs in: the platform every Gates nametag reads.
const FACE_ORIGIN: &str = "https://elopros.com";

/// Under `--mcp` stdout is the harness's JSON-RPC, so the run's report
/// goes to stderr instead.
static MCP: AtomicBool = AtomicBool::new(false);

macro_rules! out {
    ($($arg:tt)*) => {
        if MCP.load(Ordering::Relaxed) {
            eprintln!($($arg)*);
        } else {
            println!($($arg)*);
        }
    };
}
/// Most bots one run may start on one island. Plumbing bound.
const MAX_BOTS: usize = 8;
/// A society may run two full crews: a guild of sixteen.
const MAX_SOCIETY_BOTS: usize = 2 * protocol::TEAM_MAX;
/// A shift is long enough to do something between log-offs (the walk home
/// starts `explorer::LOG_OFF_SECS` before its end), and a log-off is long
/// enough to be one.
const MIN_SHIFT_ON_S: u64 = 300;
const MIN_SHIFT_OFF_S: u64 = 60;

const SOCIETY_USAGE: &str = "[--society [--crews 1] [--roles farmer,guard,...] [--allies NAME|0xWALLET,...]] [--shift ON_S,OFF_S] [--agent-keys DIR]\n--society makes the fleet (--bots 2..16, scripted or external) a guild of crews that live as Rust clans: each crew meets at THE GATE, its first bot founds the team and builds the base, and the rest live in it with a job. --crews splits the fleet into that many crews (default: as few as the team cap allows), whose bases stand side by side and who never fight each other. --roles names the jobs after each founder, in order and repeating (farmer: supply runs for the boxes, never fights; guard: walks a ring round home and takes on whoever comes in armed or fighting, leaves visitors be; runner: keeps the crew's vending machine outside the base, stocked at half THE GATE's price, and brings the junk home); the default is farmer, guard, runner, farmer, guard, farmer. A job sets the bot's temperament. --allies names players every member spares. --shift plays ON_S seconds, goes home and logs off for OFF_S, and again until --seconds is up: the compound sleeps, as a clan's does. --agent-keys DIR gives every bot its own wallet, DIR/<name>.key (made there, owner-only, when missing), so a shift's bodies are its own again on a shard that keeps them.";

fn usage() -> String {
    format!(
        "jev-bot --new-key PATH | --set-face --agent-key PATH [--name NAME] [--pic FILE.png] [--origin URL] | (--local | --server HOST:PORT) [--agent-key PATH] [--agent-name jev] [--cert-hash HEX] [--spectate-page URL] [--seconds 120] [--bots 1] {SOCIETY_USAGE} {MIND_USAGE}\nThe bot survives in-game: it answers the death screen, crawls when wounded, gathers, crafts, eats and drinks. The process ends when the run ends or the connection fails.\nEvery bot declares itself an agent a spectator can follow (NETCODE.md §2.3). Without --agent-key it is a guest on a loopback shard; with it, a wallet agent that signs with the key in that 0600 file and may join any shard that admits it (§2.4). The key is only ever named by path.\n--new-key PATH makes a new key in that file (it must not exist yet), owner-only, prints its address and exits.\n--set-face signs this key's display name and picture (a square PNG, 64-512 px, at most 256 KiB) onto its Elo Pros profile, what every Gates nametag shows, and exits.\n--bots 2..8 starts that many scripted or external guest agents on one island (each external agent is its own child); Jev and a wallet stay one bot.\nOn a shard off this machine it prints a link to watch the agent's own view live (https://elopros.com/games/gates/, or the Gates page --spectate-page names)."
    )
}

struct Options {
    new_key: Option<PathBuf>,
    local: bool,
    server: Option<String>,
    agent_key: Option<PathBuf>,
    agent_name: String,
    cert_hash: Option<String>,
    spectate_page: Option<String>,
    duration: Duration,
    bots: usize,
    mind: MindArgs,
    set_face: bool,
    face_name: Option<String>,
    face_pic: Option<PathBuf>,
    origin: String,
    seconds_given: bool,
    society: bool,
    crews: Option<usize>,
    roles: Vec<Role>,
    allies: Vec<Kin>,
    shift: Option<(Duration, Duration)>,
    agent_keys: Option<PathBuf>,
}

impl Options {
    fn parse() -> Result<Option<Self>, String> {
        let mut options = Self {
            new_key: None,
            local: false,
            server: None,
            agent_key: None,
            agent_name: AGENT_NAME.into(),
            cert_hash: None,
            spectate_page: None,
            duration: Duration::from_secs(RUN_SECONDS),
            bots: 1,
            mind: MindArgs::default(),
            set_face: false,
            face_name: None,
            face_pic: None,
            origin: FACE_ORIGIN.into(),
            seconds_given: false,
            society: false,
            crews: None,
            roles: Vec::new(),
            allies: Vec::new(),
            shift: None,
            agent_keys: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if options.mind.take(&arg, &mut args)? {
                continue;
            }
            match arg.as_str() {
                "--help" | "-h" => return Ok(None),
                "--local" => options.local = true,
                "--set-face" => options.set_face = true,
                "--name" => options.face_name = Some(args.next().ok_or("--name needs a name")?),
                "--pic" => {
                    options.face_pic =
                        Some(PathBuf::from(args.next().ok_or("--pic needs a PNG file")?))
                }
                "--origin" => options.origin = args.next().ok_or("--origin needs a URL")?,
                // A new wallet for an agent, written owner-only (`agentkey`).
                "--new-key" => {
                    options.new_key = Some(PathBuf::from(
                        args.next().ok_or("--new-key needs a file path")?,
                    ))
                }
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
                "--spectate-page" => {
                    let page = args.next().ok_or("--spectate-page needs a URL")?;
                    check_page(&page)?;
                    options.spectate_page = Some(page);
                }
                "--bots" => {
                    options.bots = args
                        .next()
                        .ok_or("--bots needs a count")?
                        .parse()
                        .map_err(|_| "invalid bot count")?;
                    if !(1..=MAX_SOCIETY_BOTS).contains(&options.bots) {
                        return Err(format!(
                            "--bots must be 1..{MAX_BOTS} (..{MAX_SOCIETY_BOTS} with --society)"
                        ));
                    }
                }
                "--society" => options.society = true,
                "--crews" => {
                    let n: usize = args
                        .next()
                        .ok_or("--crews needs a count")?
                        .parse()
                        .map_err(|_| "invalid crew count")?;
                    if !(1..=society::MAX_CREWS).contains(&n) {
                        return Err(format!("--crews must be 1..{}", society::MAX_CREWS));
                    }
                    options.crews = Some(n);
                }
                "--roles" => {
                    let list = args.next().ok_or("--roles needs a list")?;
                    for name in list.split(',') {
                        let role = Role::parse(name.trim())
                            .filter(|r| *r != Role::Founder)
                            .ok_or("--roles takes farmer, guard and runner (the founder is each crew's first bot)")?;
                        options.roles.push(role);
                    }
                }
                "--allies" => {
                    let list = args.next().ok_or("--allies needs a list")?;
                    for who in list.split(',') {
                        options.allies.push(
                            Kin::parse(who)
                                .ok_or("--allies takes names and 0x wallets, comma-separated")?,
                        );
                    }
                }
                "--shift" => {
                    let spec = args.next().ok_or("--shift needs ON_S,OFF_S")?;
                    let (on, off) = spec
                        .split_once(',')
                        .and_then(|(a, b)| {
                            Some((a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?))
                        })
                        .ok_or("--shift needs ON_S,OFF_S in seconds")?;
                    if on < MIN_SHIFT_ON_S || off < MIN_SHIFT_OFF_S {
                        return Err(format!(
                            "--shift: on at least {MIN_SHIFT_ON_S} s, off at least {MIN_SHIFT_OFF_S} s"
                        ));
                    }
                    options.shift = Some((Duration::from_secs(on), Duration::from_secs(off)));
                }
                "--agent-keys" => {
                    options.agent_keys = Some(PathBuf::from(
                        args.next().ok_or("--agent-keys needs a directory")?,
                    ))
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
                    options.seconds_given = true;
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }
        if options.set_face {
            if options.agent_key.is_none() {
                return Err("--set-face signs with --agent-key PATH".into());
            }
            if options.face_name.is_none() && options.face_pic.is_none() {
                return Err("--set-face needs --name, --pic or both".into());
            }
            if options.local || options.server.is_some() || options.new_key.is_some() {
                return Err("--set-face sets the face and exits; run the bot without it".into());
            }
            return Ok(Some(options));
        }
        if options.face_name.is_some() || options.face_pic.is_some() {
            return Err("--name and --pic go with --set-face (in game, --agent-name)".into());
        }
        if options.new_key.is_some() {
            if options.local || options.server.is_some() || options.agent_key.is_some() {
                return Err("--new-key makes a key and exits; run the bot with --agent-key".into());
            }
            return Ok(Some(options));
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
        if options.mind.source == Source::Mcp {
            if options.bots > 1 {
                return Err("--mcp is one body: one harness on this process's stdio".into());
            }
            if !options.seconds_given {
                options.duration = Duration::from_secs(MCP_RUN_SECONDS);
            }
        }
        if options.bots > 1 && options.agent_key.is_some() {
            return Err("one --agent-key is one body; --bots above 1 runs guest agents, or --agent-keys DIR".into());
        }
        if options.agent_keys.is_some() && options.agent_key.is_some() {
            return Err("choose one of --agent-key and --agent-keys".into());
        }
        if options.bots > MAX_BOTS && !options.society {
            return Err(format!("--bots above {MAX_BOTS} is a guild: add --society"));
        }
        let society_flags =
            options.crews.is_some() || !options.roles.is_empty() || !options.allies.is_empty();
        if society_flags && !options.society {
            return Err("--crews, --roles and --allies shape a --society".into());
        }
        if options.society {
            if options.bots < 2 {
                return Err("--society needs a fleet: --bots 2..16".into());
            }
            let crews = crew_count(&options);
            if options.bots.div_ceil(crews) > protocol::TEAM_MAX {
                return Err(format!(
                    "{} bots in {crews} crews is over the team cap of {}: add --crews",
                    options.bots,
                    protocol::TEAM_MAX
                ));
            }
            if crews > options.bots {
                return Err("--crews is more than the bots to found them".into());
            }
        }
        for i in 0..options.bots {
            bot_name(&options.agent_name, i, options.bots)?;
        }
        Ok(Some(options))
    }
}

/// How many crews a society's fleet makes: as asked, else as few as the
/// team cap allows.
fn crew_count(options: &Options) -> usize {
    options
        .crews
        .unwrap_or_else(|| options.bots.div_ceil(protocol::TEAM_MAX))
}

/// Which crew bot `i` is in, and its place there: crews are contiguous runs
/// of the fleet, as even as they go.
fn crew_of(options: &Options, i: usize) -> (usize, usize) {
    let per = options.bots.div_ceil(crew_count(options));
    (i / per, i % per)
}

/// Each bot's place in the society (`agent::society`): its job, its crew
/// by name and wallet with the founder first, the guild beyond it, and
/// the founder's lock secret, which is the code on the crew's doors.
fn plan_society(
    options: &Options,
    names: &[protocol::Name],
    wallets: &[Option<protocol::Address>],
    locks: &[Option<server::agent::lock::LockSecret>],
    board: &'static Board,
) -> Result<Vec<Society>, String> {
    let n = options.bots;
    let mut plans = Vec::with_capacity(n);
    for i in 0..n {
        let (crew_no, me) = crew_of(options, i);
        let mut soc = Society {
            role: match me {
                0 => Role::Founder,
                j if options.roles.is_empty() => Role::nth(j),
                j => options.roles[(j - 1) % options.roles.len()],
            },
            me: me as u8,
            crew: [Kin::NONE; protocol::TEAM_MAX],
            crew_len: 0,
            allies: [Kin::NONE; society::MAX_ALLIES],
            allies_len: 0,
            code: None,
            crew_no: crew_no as u8,
            board: Some(board),
        };
        for j in 0..n {
            let (their_crew, their_place) = crew_of(options, j);
            let kin = Kin::member(names[j], wallets[j], their_crew as u8, their_place as u8);
            if their_crew == crew_no {
                soc.crew[usize::from(soc.crew_len)] = kin;
                soc.crew_len += 1;
                if their_place == 0 {
                    soc.code = locks[j];
                }
            } else {
                if usize::from(soc.allies_len) >= society::MAX_ALLIES {
                    return Err(format!(
                        "a guild member spares at most {} others",
                        society::MAX_ALLIES
                    ));
                }
                soc.allies[usize::from(soc.allies_len)] = kin;
                soc.allies_len += 1;
            }
        }
        for kin in &options.allies {
            if usize::from(soc.allies_len) >= society::MAX_ALLIES {
                return Err(format!(
                    "--allies: a guild member spares at most {} others, its other crews included",
                    society::MAX_ALLIES
                ));
            }
            soc.allies[usize::from(soc.allies_len)] = *kin;
            soc.allies_len += 1;
        }
        plans.push(soc);
    }
    Ok(plans)
}

/// `--agent-keys DIR`: bot `name`'s own wallet, made there owner-only the
/// first time.
fn bot_key(dir: &std::path::Path, name: &str) -> Result<agentkey::AgentKey, String> {
    let path = dir.join(format!("{name}.key"));
    if path.exists() {
        return Door::load_key(&path);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let key = agentkey::AgentKey::create(&path).map_err(|e| e.to_string())?;
    out!(
        "new agent key for {name} in {} (owner-only): {}",
        path.display(),
        String::from_utf8_lossy(&key.address().to_checksum_hex())
    );
    Ok(key)
}

/// `--set-face`: this key's name and picture on its Elo Pros profile,
/// signed by the key itself (`server::face_set`).
fn set_face(options: &Options) -> Result<(), String> {
    let path = options.agent_key.as_deref().expect("validated");
    let key = Door::load_key(path)?;
    let png = match &options.face_pic {
        Some(pic) => {
            let bytes = std::fs::read(pic).map_err(|e| format!("{}: {e}", pic.display()))?;
            if bytes.len() > server::face_set::FACE_PIC_MAX_BYTES {
                return Err(format!(
                    "{} is over the platform's {}-byte cap",
                    pic.display(),
                    server::face_set::FACE_PIC_MAX_BYTES
                ));
            }
            Some(bytes)
        }
        None => None,
    };
    let face = server::face_set::set_face(
        &key,
        &options.origin,
        options.face_name.as_deref(),
        png.as_deref(),
    )?;
    let hex = key.address().to_hex();
    let wallet = String::from_utf8_lossy(&hex);
    out!(
        "face set for {wallet}: name {}, picture {}\nsee it: {}/api/face/{wallet}",
        face["name"]
            .as_str()
            .map_or("(none)".into(), |n| format!("{n:?}")),
        face["picture"].as_str().unwrap_or("(none)"),
        options.origin.trim_end_matches('/')
    );
    Ok(())
}

/// `--new-key`: a fresh wallet for an agent, its address printed. The
/// secret stays in the file.
fn new_key(path: &std::path::Path) -> Result<(), String> {
    let key = agentkey::AgentKey::create(path).map_err(|e| e.to_string())?;
    let address = key.address().to_checksum_hex();
    out!(
        "new agent key in {} (owner-only; keep it safe)\naddress: {}\nrun: jev-bot --server game.elopros.com:61234 --agent-key {} --scripted",
        path.display(),
        String::from_utf8_lossy(&address),
        path.display()
    );
    Ok(())
}

async fn run(options: Options) -> Result<(), String> {
    if let Some(path) = &options.new_key {
        return new_key(path);
    }
    if options.set_face {
        // Blocking HTTP: off the runtime's worker, which it would stall.
        return tokio::task::block_in_place(|| set_face(&options));
    }
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
    if options.mind.source == Source::Mcp {
        MCP.store(true, Ordering::Relaxed);
        server::mcp::set_me(server::mcp::Me {
            name: options.agent_name.clone(),
            wallet: key.as_ref().map(|k| {
                let hex = k.address().to_hex();
                String::from_utf8_lossy(&hex).into_owned()
            }),
        });
    }
    // Ears and a voice (`--voice`): the first body gets them, and its mind's
    // lines are spoken through them.
    let mut voice = None;
    let mut speech = None;
    if options.mind.voice {
        if !matches!(options.mind.source, Source::External(..) | Source::Mcp) {
            return Err("--voice needs an external or MCP mind (--external or --mcp)".into());
        }
        // No key is a body without a voice, said once, never a body that
        // does not start: a service gains its voice when the key lands.
        match server::agent_voice::VoiceConfig::from_env() {
            Ok(cfg) => {
                out!(
                    "voice: {}, speaks with {}",
                    if cfg.hears() {
                        format!("hears through {} ({})", cfg.base, cfg.stt_model)
                    } else {
                        "no ears (no GATES_VOICE_KEY or OPENAI_API_KEY)".into()
                    },
                    match &cfg.eleven {
                        Some((_, v, m)) => format!("ElevenLabs voice {v} ({m})"),
                        None => format!("{} voice {}", cfg.tts_model, cfg.tts_voice),
                    }
                );
                let (v, tx) = server::agent_voice::AgentVoice::spawn(cfg)?;
                voice = Some(Box::new(v));
                speech = Some(tx);
            }
            Err(e) => out!("voice: off - {e}"),
        }
    }
    let mut minds = Vec::with_capacity(options.bots);
    for _ in 0..options.bots {
        minds.push(options.mind.build_with_speech(speech.take())?);
    }
    out!(
        "mind: {} × {} · {} hands · {}",
        options.mind.label(),
        options.bots,
        options.mind.skill.name(),
        options.mind.temperament.name()
    );
    let shard = if options.local {
        let handle = server::agent_demo::spawn_local().await?;
        out!(
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
    out!(
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
    // Where the owner watches its own view (NETCODE.md §2.3): the public
    // page off this machine, a named page on it, else the bare query.
    if let Some(url) = door.watch_url(options.spectate_page.as_deref()) {
        server::mcp::set_watch(url);
    }
    match door.watch_url(options.spectate_page.as_deref()) {
        Some(url) if door.loopback() => out!("watch it (this machine only): {url}"),
        Some(url) => out!("watch it live: {url}"),
        None => out!(
            "watch it (this machine only): a Gates web page with ?{}",
            door.spectate_query()
        ),
    }
    out!("  or the desktop client: {}", door.desktop_command());
    let endpoint = Arc::new(door.endpoint()?);
    // Every bot's name, door and lock: one wallet each with --agent-keys,
    // else the run's one door.
    let mut names = Vec::with_capacity(options.bots);
    for i in 0..options.bots {
        names.push(bot_name(&options.agent_name, i, options.bots)?);
    }
    let door = Arc::new(door);
    let mut doors = Vec::with_capacity(options.bots);
    for name in &names {
        doors.push(match &options.agent_keys {
            Some(dir) => Arc::new(Door::new(
                door.server.clone(),
                door.cert_hash.clone(),
                Some(bot_key(dir, name.as_str())?),
            )?),
            None => door.clone(),
        });
    }
    let locks: Vec<_> = names
        .iter()
        .zip(&doors)
        .map(|(name, d)| {
            server::agent_demo::lock_secret(
                d.key.as_ref(),
                name.as_str(),
                &server::agent_demo::guest_secret_path(),
            )
        })
        .collect();
    let plans = if options.society {
        let wallets: Vec<_> = doors
            .iter()
            .map(|d| d.key.as_ref().map(|k| k.address()))
            .collect();
        let board: &'static Board = Box::leak(Box::new(Board::new()));
        let plans = plan_society(&options, &names, &wallets, &locks, board)?;
        for c in 0..crew_count(&options) {
            let crew: Vec<String> = plans
                .iter()
                .zip(&names)
                .filter(|(p, _)| usize::from(p.crew_no) == c)
                .map(|(p, n)| format!("{} {}", n.as_str(), p.role.name()))
                .collect();
            if !crew.is_empty() {
                out!("society: crew {} — {}", c + 1, crew.join(", "));
            }
        }
        plans.into_iter().map(Some).collect()
    } else {
        vec![None; options.bots]
    };
    if let Some((on, off)) = options.shift {
        out!(
            "shifts: {} s on, {} s off{}",
            on.as_secs(),
            off.as_secs(),
            if options.agent_keys.is_none() && door.key.is_none() {
                " (guests come back as new bodies; --agent-keys DIR keeps them)"
            } else {
                ""
            }
        );
    }
    let mut fleet = tokio::task::JoinSet::new();
    for (i, mind) in minds.into_iter().enumerate() {
        let endpoint = endpoint.clone();
        let door = doors[i].clone();
        let duration = options.duration;
        let shift = options.shift;
        let name = names[i];
        let mut opts = options.mind.opts();
        opts.lock = locks[i];
        opts.society = plans[i];
        if let Some(soc) = plans[i] {
            opts.temperament = soc.role.temperament();
        }
        let voice = if i == 0 { voice.take() } else { None };
        fleet.spawn(async move {
            let mut survivor = Survivor::with(mind, opts);
            survivor.voice = voice;
            // The run in shifts (`--shift`), else one: each session's end
            // has the body home with its doors shut when it comes, counted
            // in the shard's ticks from that session's welcome.
            let mut left = duration;
            loop {
                let on = shift.map_or(left, |(on, _)| on.min(left));
                let ticks = on.as_secs().saturating_mul(u64::from(TICK_HZ));
                survivor.new_shift(ticks.min(u64::from(u32::MAX)) as u32);
                let result = door.play(&endpoint, name, on, &mut survivor).await;
                left = left.saturating_sub(on);
                let off = match shift {
                    Some((_, off)) if result.is_ok() && !left.is_zero() => off.min(left),
                    _ => return (i, survivor, result),
                };
                out!("{}: off shift for {} s", name.as_str(), off.as_secs());
                tokio::time::sleep(off).await;
                left = left.saturating_sub(off);
                if left.is_zero() {
                    return (i, survivor, result);
                }
            }
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
        out!("interrupted by Ctrl-C");
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
                out!(
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
    out!(
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
    out!(
        "{label}survival: {} deaths, {} respawns ({} asks), {} retreats; goals {} done, {} failed, {} interrupted; {} targets finished, {} abandoned, {} given way; crafted {}, eaten {}, drinks {}, equips {}; {} actions sent",
        s.deaths,
        s.respawns,
        s.respawn_asks,
        s.retreats,
        s.goals_done,
        s.goals_failed,
        s.goals_interrupted,
        s.targets_completed,
        s.targets_abandoned,
        s.gave_way,
        s.crafted,
        s.eaten,
        s.drinks,
        s.equips,
        s.actions
    );
    out!(
        "{label}chat: said {} ({} heard back), heard {} from others, {} refused, {} replaced unsent",
        s.said, s.echoed, s.heard, s.say_refused, s.say_dropped
    );
    if let Some(v) = survivor.voice.as_ref() {
        let v = v.stats;
        out!(
            "{label}voice: heard {} frames, {} utterances, {} transcribed ({} over the cap); spoke {} lines in {} frames ({} over the cap); {} service failures",
            v.frames_heard, v.utterances, v.transcribed, v.heard_dropped, v.lines_spoken, v.frames_sent, v.said_dropped, v.failures
        );
    }
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
        out!("{label}gathered: {}", gathered.join(", "));
        if survivor.delivered > 0 || survivor.took > 0 {
            out!(
                "{label}team's boxes: {} units put in, {} taken out",
                survivor.delivered,
                survivor.took
            );
        }
        let mut pack: Vec<(String, u32)> = Vec::new();
        for stack in core.inv.iter().filter(|s| s.count > 0) {
            let name = String::from_utf8_lossy(core.catalog.name(stack.item as usize)).into_owned();
            match pack.iter_mut().find(|(n, _)| *n == name) {
                Some(entry) => entry.1 += u32::from(stack.count),
                None => pack.push((name, u32::from(stack.count))),
            }
        }
        let pack: Vec<String> = pack.iter().map(|(n, c)| format!("{n} {c}")).collect();
        out!(
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
    out!("{label}recent goals: {}", goals.join(" | "));
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
