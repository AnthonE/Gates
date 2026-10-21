//! Shared setup for the headless survivor and its rendered local broadcast.

use crate::agent::combat::Temperament;
use crate::agent::hands::Preset;
use crate::autopilot::Autopilot;
use crate::botclient::{
    agent_endpoint, run_agent_bot, run_guest_agent, AgentIdentity, BotDriver, BotReport,
};
use crate::explorer::SurvivorOpts;
/// Explicit offline goals (`mind::Scripted`), never a fallback for a
/// failed model request. Re-exported here for the bins that name it.
pub use crate::mind::Scripted;
use crate::mind::{DecisionSource, Mind, MindConfig};
use agentkey::AgentKey;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::Endpoint;

/// The domain an agent's lock-code signature names: never a shard's, so the
/// signature is no login anywhere, and never sent.
const LOCK_SIGN_DOMAIN: &str = "lock-code.gates.invalid";

/// Where an agent's lock code comes from (`agent::lock`): a wallet agent's
/// from what only its key signs, a guest's from its install's secret kept
/// at `guest_path` (made there the first time). `None` when the secret
/// cannot be read or made: the body then makes up a code for this run.
pub fn lock_secret(
    key: Option<&AgentKey>,
    name: &str,
    guest_path: &Path,
) -> Option<crate::agent::lock::LockSecret> {
    use crate::agent::lock::{guest_secret, LockCode, LockSecret};
    if let Some(key) = key {
        let sig = key.sign_siwe(LOCK_SIGN_DOMAIN, &[0; protocol::NONCE_BYTES], 0);
        return Some(LockSecret::Code(LockCode::derive(&sig.0, name)));
    }
    if let Some(dir) = guest_path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    guest_secret(guest_path)
        .ok()
        .map(|install| LockSecret::guest(install, name))
}

/// Where a guest agent's install keeps its lock secret: `GATES_AGENT_SECRET`
/// if set, else `~/.gates/agent-guest.secret`, else `.gates/` in the working
/// directory — never the shared temp directory, where another user could
/// create the file first and know the secret.
pub fn guest_secret_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("GATES_AGENT_SECRET") {
        return p.into();
    }
    match std::env::var_os("HOME") {
        Some(home) => std::path::Path::new(&home).join(".gates/agent-guest.secret"),
        None => std::path::Path::new(".gates").join("agent-guest.secret"),
    }
}

/// The display name a jev bot declares to its spectators when none is given.
pub const AGENT_NAME: &str = "jev";

/// The Gates web page an agent off this machine is watched from. nginx
/// sends `/games/gates/?server=…` on to the build wherever it lives, so the
/// link holds across the build's move to `/games/gates/play/`.
pub const PUBLIC_WATCH_PAGE: &str = "https://elopros.com/games/gates/";

/// Where a jev bot plays and who it says it is (`NETCODE.md` §2.3, §2.4).
///
/// Without a key it is a **guest agent**: unsigned, loopback only, declared an
/// agent and watchable, followed by "any watchable agent". With a key it is a
/// **wallet agent**: the key is loaded from a `0600` file named by path (never
/// argv), the dial validates the shard's certificate (a pin, the root store
/// off loopback), and it pays every door a person does.
pub struct Door {
    /// `host:port`, dialled by name so a real certificate can validate.
    pub server: String,
    /// A pinned certificate digest (the dotted hex a dev shard prints).
    pub cert_hash: Option<String>,
    pub key: Option<AgentKey>,
}

impl Door {
    /// Load an agent's key from its file (`0600`, 64 hex digits). The only
    /// way a jev bot gets a key: named by path, never carried in argv.
    pub fn load_key(path: &Path) -> Result<AgentKey, String> {
        AgentKey::load(path).map_err(|e| e.to_string())
    }

    pub fn new(
        server: impl Into<String>,
        cert_hash: Option<String>,
        key: Option<AgentKey>,
    ) -> Result<Self, String> {
        let server = server.into();
        let cert_hash = cert_hash.filter(|h| !h.trim().is_empty());
        let door = Self {
            server,
            cert_hash,
            key,
        };
        if door.key.is_none() {
            door.guest_addr()?;
        }
        Ok(door)
    }

    fn guest_addr(&self) -> Result<SocketAddr, String> {
        self.server
            .parse::<SocketAddr>()
            .ok()
            .filter(|a| a.ip().is_loopback())
            .ok_or_else(|| {
                "a guest agent joins only a loopback shard (127.0.0.1:PORT); pass --agent-key PATH to join any other".into()
            })
    }

    /// The shard is on this machine, so its seats reach nobody else.
    pub fn loopback(&self) -> bool {
        self.server
            .parse::<SocketAddr>()
            .map_or(self.server.starts_with("localhost:"), |a| {
                a.ip().is_loopback()
            })
    }

    /// Where a person watches this body: `page` with this door's spectator
    /// query, or — off this machine, with no page named — the public Gates
    /// page. `None` for a loopback shard with no page: there is no page
    /// this machine is known to serve.
    pub fn watch_url(&self, page: Option<&str>) -> Option<String> {
        let page = page.or((!self.loopback()).then_some(PUBLIC_WATCH_PAGE))?;
        Some(spectate_url(page, &self.spectate_query()))
    }

    /// The dial: certificate-validating, permissive only on loopback.
    pub fn endpoint(&self) -> Result<Endpoint<Client>, String> {
        agent_endpoint(&self.server, self.cert_hash.as_deref())
    }

    /// What a spectator names: `any` for a guest agent (it has no proven
    /// address), the wallet for a wallet agent.
    pub fn watch_target(&self) -> String {
        match &self.key {
            None => "any".into(),
            Some(key) => String::from_utf8_lossy(&key.address().to_hex()).into_owned(),
        }
    }

    /// The query a Gates web page takes to watch this body (`?server=…`).
    pub fn spectate_query(&self) -> String {
        let mut q = format!("server={}", self.server);
        if let Some(hash) = &self.cert_hash {
            q.push_str("&hash=");
            q.push_str(hash);
        }
        match &self.key {
            None => q.push_str("&spectate"),
            Some(_) => {
                q.push_str("&spectate=");
                q.push_str(&self.watch_target());
            }
        }
        q
    }

    /// The desktop client's command for the same seat.
    pub fn desktop_command(&self) -> String {
        let pin = self
            .cert_hash
            .as_ref()
            .map(|h| format!(" --cert-hash {h}"))
            .unwrap_or_default();
        format!(
            "cargo run -p client --features render --bin gates -- --server {}{pin} --spectate {}",
            self.server,
            self.watch_target()
        )
    }

    /// Play `duration` of ticks as `name` through this door.
    pub async fn play(
        &self,
        endpoint: &Endpoint<Client>,
        name: protocol::Name,
        duration: Duration,
        driver: &mut dyn BotDriver,
    ) -> Result<BotReport, String> {
        match &self.key {
            None => run_guest_agent(endpoint, self.guest_addr()?, name, duration, driver).await,
            Some(key) => {
                let identity = AgentIdentity {
                    key,
                    name,
                    watchable: true,
                };
                run_agent_bot(endpoint, &self.server, &identity, duration, driver).await
            }
        }
    }
}

/// The name bot `i` of `n` declares: `base`, or `base-i` in a fleet.
pub fn bot_name(base: &str, i: usize, n: usize) -> Result<protocol::Name, String> {
    let name = if n > 1 {
        format!("{base}-{}", i + 1)
    } else {
        base.to_owned()
    };
    protocol::Name::new(&name)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("agent name {name:?} must be 1-16 printable ASCII characters"))
}

/// A Gates web page to link spectators to: `http(s)://`, printable, bounded.
pub fn check_page(page: &str) -> Result<(), String> {
    let ok = (page.starts_with("http://") || page.starts_with("https://"))
        && page.len() <= 512
        && page
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"\"'<>`\\".contains(&b));
    if ok {
        Ok(())
    } else {
        Err("--spectate-page must be an http(s) URL of a Gates web page".into())
    }
}

/// `page` with a spectator query appended.
pub fn spectate_url(page: &str, query: &str) -> String {
    let sep = if page.contains('?') { '&' } else { '?' };
    format!("{page}{sep}{query}")
}

/// The decision-source flags both agent binaries accept.
pub const MIND_USAGE: &str = "[--scripted | --mcp | --student FILE | --external PROGRAM [ARG...]] [--record DIR] [--autopilot-s 45] [--friend 0xWALLET] [--friend-away autopilot|think] [--alone think|autopilot] [--voice] [--agent-harness NAME] [--agent-model NAME] [--think-ms 1000] [--timeout-ms 3000] [--heartbeat-s 30] [--max-requests-hour 600] [--max-requests-day 7200] [--skill novice|average|good|pro] [--temperament passive|defensive|opportunist|kos] [--tactics ga1:...|off]\n--tactics picks the bred brain (examples/zoo.rs --agent) that decides when to run from wolves and which hand to fight them with (default: agent::combat::DEFAULT_TACTICS; off: the hand-written sum); its eyes and hands stay a person's, and against people the sum always decides.\nJev needs TYPESAFE_API_KEY. --scripted makes no model calls. --external runs your own agent as a child speaking JSON lines (JEV.md) and must come last. --mcp makes this process an MCP server on its own stdin/stdout, so an MCP harness (Claude Code, Hermes, Codex, OpenClaw...) decides with the tools look, act and status; a turn it has not answered in --autopilot-s is played by the scripted policy. --friend names the one wallet it plays with: it meets them at THE GATE, teams up, follows; while they are away the scripted policy plays and no model is asked, unless --friend-away think, which keeps the mind deciding on its own. --alone autopilot hands every turn with no person near (no friend on the island, no human in view or heard) to the scripted policy, so a crowd of minds costs nothing while the island is empty. --voice gives an external or MCP mind ears and a voice: what players near it say out loud is transcribed into what it hears, and its lines are spoken (GATES_VOICE_KEY or OPENAI_API_KEY; JEV.md). Decisions are never more often than once a second. --skill sets how fast and steady the bot's hands aim (default good): its view turns and settles like a player's, whatever decides. --temperament sets when it fights (default opportunist): it always answers an attack, and starts a fight only with a clear edge. --record DIR writes every decision, whoever makes it, to a new JSON-lines file in DIR for distilling (ops/distill/train.py); --student FILE decides with the classifier that trainer wrote, with no model calls. --agent-harness and --agent-model say what plays it (at most 48 of A-Z a-z 0-9 space . _ : / @ + -): written into --record rows and, with --set-face, onto its Elo Pros face.";

/// Which source decides: Jev, the explicit scripted policy, or an agent
/// the operator brings. Never swapped for another at runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Jev,
    Scripted,
    External(String, Vec<String>),
    /// An MCP harness on this process's own stdio (`mcp.rs`).
    Mcp,
    /// A classifier trained on recorded decisions (`distill.rs`).
    Student(std::path::PathBuf),
}

#[derive(Clone, Debug)]
pub struct MindArgs {
    pub source: Source,
    pub cfg: MindConfig,
    /// How good the survivor's hands are; not the mind's, but chosen with it.
    pub skill: Preset,
    /// When the survivor fights, likewise.
    pub temperament: Temperament,
    /// Where the body's rendered view is served, told to an external
    /// source as `GATES_FRAME_URL` (`jev-watch` only: `jev-bot` renders
    /// nothing).
    pub frame_url: Option<String>,
    /// The one wallet it plays with (`--friend 0x…`).
    pub friend: Option<protocol::Address>,
    /// While its friend is away the mind keeps deciding on its own
    /// (`--friend-away think`) instead of handing the body to the free
    /// scripted policy (`autopilot`, the default): a character with a
    /// friend, rather than a buddy that waits for one.
    pub friend_away_think: bool,
    /// With no person near (`autopilot::alone`), the free scripted policy
    /// plays and the mind is not asked (`--alone autopilot`); the default
    /// keeps it deciding.
    pub alone_autopilot: bool,
    /// How long an MCP harness has to answer before the scripted policy
    /// plays that turn (`--autopilot-s`).
    pub autopilot: Duration,
    /// Ears and a voice (`--voice`, `agent_voice.rs`): it hears what the
    /// players near it say out loud and speaks its lines.
    pub voice: bool,
    /// Where every decision is recorded for distilling (`--record DIR`).
    pub record: Option<std::path::PathBuf>,
    /// What plays this body, as the platform labels it (`--agent-harness`,
    /// `--agent-model`; `agentkey::label_ok`): self-declared, like
    /// `HELLO_AGENT`. Written into `--record` rows and signed onto the face
    /// by `--set-face`. `Some("")` clears a face's label.
    pub harness: Option<String>,
    pub model: Option<String>,
    /// Counts what the mind decides and who decided it (`bench.rs`).
    pub tally: Option<std::sync::Arc<crate::bench::Tally>>,
    /// The bred brain for its wolf fights (`--tactics ga1:…`, `off` for
    /// none; `DEFAULT_TACTICS` unless told): every source plays through the
    /// same reflexes, so every source takes it.
    pub tactics: Option<sim_core::genome::Genome>,
    /// Decide on the caller's thread (`Mind::inline`), for a lockstep island
    /// (`--warp`, `lockstep.rs`): only a mind that never waits on anything
    /// — the scripted policy or a student — may.
    pub inline: bool,
}

/// `--autopilot-s`'s default: a long think for a big model, well inside the
/// minute a decision may wait.
pub const AUTOPILOT_DEFAULT: Duration = Duration::from_secs(45);
/// How much longer than the harness's own deadline (`autopilot`) an MCP
/// decision may wait, so the autopilot's answer lands inside it.
const MCP_DECISION_SLACK: Duration = Duration::from_secs(15);

impl Default for MindArgs {
    fn default() -> Self {
        Self {
            source: Source::Jev,
            cfg: MindConfig::default(),
            skill: Preset::default(),
            temperament: Temperament::default(),
            frame_url: None,
            friend: None,
            friend_away_think: false,
            alone_autopilot: false,
            autopilot: AUTOPILOT_DEFAULT,
            voice: false,
            record: None,
            harness: None,
            model: None,
            tally: None,
            tactics: Some(crate::agent::combat::default_tactics()),
            inline: false,
        }
    }
}

impl MindArgs {
    /// Take one flag if it is a mind flag. `Ok(false)`: not ours.
    pub fn take(
        &mut self,
        arg: &str,
        args: &mut impl Iterator<Item = String>,
    ) -> Result<bool, String> {
        let mut number = |what: &str| -> Result<u64, String> {
            args.next()
                .ok_or_else(|| format!("{what} needs a value"))?
                .parse()
                .map_err(|_| format!("invalid {what}"))
        };
        match arg {
            "--scripted" => self.source = Source::Scripted,
            "--think-ms" => self.cfg.interval = Duration::from_millis(number(arg)?),
            "--timeout-ms" => self.cfg.timeout = Duration::from_millis(number(arg)?),
            "--heartbeat-s" => self.cfg.heartbeat = Duration::from_secs(number(arg)?),
            "--max-requests-hour" => {
                self.cfg.per_hour = u32::try_from(number(arg)?).map_err(|_| "ceiling too large")?
            }
            "--max-requests-day" => {
                self.cfg.per_day = u32::try_from(number(arg)?).map_err(|_| "ceiling too large")?
            }
            "--skill" => {
                let name = args.next().ok_or("--skill needs a value")?;
                self.skill =
                    Preset::parse(&name).ok_or("--skill is one of novice, average, good, pro")?;
            }
            "--temperament" => {
                let name = args.next().ok_or("--temperament needs a value")?;
                self.temperament = Temperament::parse(&name)
                    .ok_or("--temperament is one of passive, defensive, opportunist, kos")?;
            }
            "--mcp" => self.source = Source::Mcp,
            "--voice" => self.voice = true,
            "--record" => {
                self.record = Some(args.next().ok_or("--record needs a directory")?.into())
            }
            "--student" => {
                self.source = Source::Student(args.next().ok_or("--student needs a file")?.into())
            }
            "--tactics" => {
                let text = args.next().ok_or("--tactics needs a ga1: genome or off")?;
                if text == "off" {
                    self.tactics = None;
                    return Ok(true);
                }
                let genome = sim_core::genome::Genome::decode(
                    crate::agent::combat::TACTICS_PREFIX,
                    text.as_bytes(),
                )
                .map_err(|e| format!("--tactics is not a ga1: genome ({e:?})"))?;
                self.tactics = Some(genome);
            }
            "--autopilot-s" => {
                let secs = number(arg)?;
                if !(5..=165).contains(&secs) {
                    return Err("--autopilot-s must be 5..165".into());
                }
                self.autopilot = Duration::from_secs(secs);
            }
            "--friend" => {
                let wallet = args.next().ok_or("--friend needs a wallet (0x + 40 hex)")?;
                let address = protocol::Address::from_hex(wallet.trim().as_bytes())
                    .filter(|a| !a.is_guest())
                    .ok_or("--friend needs a wallet (0x + 40 hex)")?;
                self.friend = Some(address);
            }
            "--friend-away" => {
                self.friend_away_think = match args.next().as_deref() {
                    Some("think") => true,
                    Some("autopilot") => false,
                    _ => return Err("--friend-away is autopilot or think".into()),
                };
            }
            "--alone" => {
                self.alone_autopilot = match args.next().as_deref() {
                    Some("autopilot") => true,
                    Some("think") => false,
                    _ => return Err("--alone is autopilot or think".into()),
                };
            }
            "--agent-harness" | "--agent-model" => {
                let label = args.next().ok_or_else(|| format!("{arg} needs a name"))?;
                if !agentkey::label_ok(&label) {
                    return Err(format!(
                        "{arg} is at most {} of A-Z a-z 0-9 space . _ : / @ + -",
                        agentkey::LABEL_MAX
                    ));
                }
                match arg {
                    "--agent-harness" => self.harness = Some(label),
                    _ => self.model = Some(label),
                }
            }
            "--external" => {
                let program = args.next().ok_or("--external needs a program")?;
                self.source = Source::External(program, args.collect());
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The survivor's setup beyond its mind.
    pub fn opts(&self) -> SurvivorOpts {
        SurvivorOpts {
            skill: self.skill.skill(),
            temperament: self.temperament,
            lock: None,
            friend: self.friend,
            society: None,
            code_doors: false,
            tactics: self.tactics,
        }
    }

    /// The source in one word, as a bug report and a bench result name it.
    pub fn word(&self) -> &'static str {
        match self.source {
            Source::Jev => "jev",
            Source::Scripted => "scripted",
            Source::External(..) => "external",
            Source::Mcp => "mcp",
            Source::Student(_) => "student",
        }
    }

    pub fn label(&self) -> String {
        match &self.source {
            Source::Jev => format!("Jev {}", crate::jev::MODEL),
            Source::Scripted => "SCRIPTED goals; no model calls".into(),
            Source::External(program, _) => format!("EXTERNAL agent: {program}"),
            Source::Mcp => format!(
                "MCP harness on stdio; autopilot after {} s",
                self.autopilot.as_secs()
            ),
            Source::Student(file) => format!("STUDENT {}; no model calls", file.display()),
        }
    }

    /// Check the configuration and credentials, then start the source.
    pub fn build(&self) -> Result<Mind, String> {
        self.build_with_speech(None)
    }

    /// [`Self::build`], with a voice for an external or MCP mind's lines
    /// (`agent_voice::AgentVoice::spawn`'s producer).
    pub fn build_with_speech(
        &self,
        speech: Option<crate::agent_voice::SpeechTx>,
    ) -> Result<Mind, String> {
        if self.inline && !matches!(self.source, Source::Scripted | Source::Student(_)) {
            return Err(
                "--warp plays the scripted policy or a --student: a model or a \
                        harness thinks on the wall clock, and a warped island does not wait"
                    .into(),
            );
        }
        let voiced = |door: crate::external::External| match speech {
            Some(tx) => door.with_speech(tx),
            None => door,
        };
        self.cfg.check()?;
        let buddy = self.friend.is_some();
        // A buddy's mind costs nothing while its friend is away, nor any
        // mind with `--alone autopilot` while no person is near: those turns
        // are the autopilot's and never reach the spend guard. `--friend-away
        // think` keeps a buddy's mind deciding instead.
        let gate = if buddy && !self.friend_away_think {
            Some(crate::autopilot::FRIEND_AWAY)
        } else if self.alone_autopilot {
            Some(crate::autopilot::ALONE)
        } else {
            None
        };
        let priced = |mind: Mind| match gate {
            Some(g) => mind.free_when(g.rule),
            None => mind,
        };
        match &self.source {
            Source::Scripted => self.start(Scripted::default(), self.cfg),
            Source::Student(file) => self.start(crate::distill::Student::load(file)?, self.cfg),
            Source::Mcp => {
                let cfg = MindConfig {
                    timeout: self.autopilot + MCP_DECISION_SLACK,
                    ..self.cfg
                };
                cfg.check()?;
                let (door, presence) = crate::mcp::spawn(self.autopilot)?;
                let quiet = self.autopilot + crate::mcp::AWAY_GRACE;
                let door = crate::mcp::McpDoor::new(voiced(door), presence, quiet);
                self.start(Autopilot::new(door, gate), cfg).map(priced)
            }
            Source::External(program, args) => {
                let env: Vec<(&str, String)> = self
                    .frame_url
                    .iter()
                    .map(|url| ("GATES_FRAME_URL", url.clone()))
                    .collect();
                if buddy || gate.is_some() {
                    // The autopilot's answer to a request the agent missed
                    // must land inside the mind's own deadline.
                    let inner = self.cfg.timeout.mul_f32(0.8);
                    let door = voiced(crate::external::External::spawn_with_env(
                        program, args, &env, inner,
                    )?);
                    return self.start(Autopilot::new(door, gate), self.cfg).map(priced);
                }
                self.start(
                    voiced(crate::external::External::spawn_with_env(
                        program,
                        args,
                        &env,
                        self.cfg.timeout,
                    )?),
                    self.cfg,
                )
            }
            Source::Jev => {
                let key = std::env::var("TYPESAFE_API_KEY").map_err(|_| {
                    "set TYPESAFE_API_KEY, or choose --scripted or --external explicitly"
                })?;
                self.start(crate::jev::Jev::new(key, self.cfg.timeout)?, self.cfg)
            }
        }
    }

    /// `--agent-harness` and `--agent-model` as a row names them: `None`
    /// when unset or cleared.
    pub fn labels(&self) -> (Option<&str>, Option<&str>) {
        fn set(l: &Option<String>) -> Option<&str> {
            l.as_deref().filter(|l| !l.is_empty())
        }
        (set(&self.harness), set(&self.model))
    }

    /// The decision worker for `source`, counted when a bench is counting
    /// and recording what it decides when `--record` named a directory.
    fn start(&self, source: impl DecisionSource, cfg: MindConfig) -> Result<Mind, String> {
        match &self.tally {
            Some(t) => self.recorded(crate::bench::Counted::new(source, t.clone()), cfg),
            None => self.recorded(source, cfg),
        }
    }

    fn recorded(&self, source: impl DecisionSource, cfg: MindConfig) -> Result<Mind, String> {
        match &self.record {
            Some(dir) => {
                let (harness, model) = self.labels();
                let source = crate::distill::Recorder::open(source, dir, harness, model)?;
                if self.inline {
                    Mind::inline(source, cfg)
                } else {
                    Mind::new(source, cfg)
                }
            }
            None if self.inline => Mind::inline(source, cfg),
            None => Mind::new(source, cfg),
        }
    }
}

pub async fn spawn_local() -> Result<crate::net::ShardHandle, String> {
    spawn_local_with_content(&local_content()).await
}

/// The content a local island boots with: `content/` beside this program,
/// where a release archive puts it, else the source tree it was built
/// from. A downloaded `jev-bot --local` has no source tree to point at.
pub fn local_content() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("content")))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"))
}

/// A local island's config: `shard.toml.example`'s seed on a loopback port,
/// guests welcome, nobody seated but who dials, everyone landing at one
/// spawn.
pub fn local_config() -> Result<crate::config::ShardConfig, String> {
    let mut cfg = crate::config::parse_shard_toml(include_str!("../../../shard.toml.example"))?;
    cfg.bind = "127.0.0.1:0".parse().expect("loopback");
    cfg.require_auth = false;
    cfg.population = 0;
    cfg.dev_spawn = Some(sim_core::world::World::new(cfg.seed).spawn_pos(0));
    // Spectator seats open (`NETCODE.md` §2.3). The bind is loopback, so
    // only a viewer on this machine can reach them; a remote viewer needs a
    // routable bind and a fixed UDP port, which is an operator's call.
    cfg.spectate = crate::config::Spectate::on();
    Ok(cfg)
}

pub async fn spawn_local_with_content(
    content_dir: &std::path::Path,
) -> Result<crate::net::ShardHandle, String> {
    boot_local(local_config()?, content_dir).await
}

/// A bench island (`bench.rs`): the local one, with its standings kept
/// beside `base` (`<base>.standings`, written when it shuts down). Nothing
/// else is kept: the world and the players are not saved.
pub async fn spawn_bench(
    content_dir: &std::path::Path,
    base: &std::path::Path,
) -> Result<crate::net::ShardHandle, String> {
    let mut cfg = local_config()?;
    cfg.world_file = Some(base.display().to_string());
    cfg.dev_arc_hour = Some(crate::bench::BENCH_ARC_HOUR);
    boot_local(cfg, content_dir).await
}

async fn boot_local(
    cfg: crate::config::ShardConfig,
    content_dir: &std::path::Path,
) -> Result<crate::net::ShardHandle, String> {
    let content = content::Content::load_dir(content_dir).map_err(|e| format!("content: {e}"))?;
    crate::net::spawn_shard(
        cfg,
        crate::net::bake_all(&content)?,
        crate::store::Saves::off(),
        crate::worldfile::WorldBoot::off(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mind_flags_parse_strictly_and_keep_the_once_a_second_floor() {
        let parse = |line: &str| {
            let mut args = line.split_whitespace().map(str::to_owned);
            let mut m = MindArgs::default();
            while let Some(arg) = args.next() {
                assert!(m.take(&arg, &mut args)?, "{arg} is a mind flag");
            }
            m.cfg.check().map(|_| m)
        };
        let m = parse("--max-requests-hour 60 --heartbeat-s 45 --external python3 agent.py --x")
            .unwrap();
        assert_eq!(
            m.source,
            Source::External("python3".into(), vec!["agent.py".into(), "--x".into()])
        );
        assert_eq!((m.cfg.per_hour, m.cfg.heartbeat.as_secs()), (60, 45));
        assert!(
            parse("--think-ms 999").is_err(),
            "never under once a second"
        );
        assert!(parse("--max-requests-day 0").is_err());
        assert!(parse("--heartbeat-s").is_err());
        assert_eq!(parse("--scripted").unwrap().source, Source::Scripted);
        assert_eq!(parse("--scripted").unwrap().skill, Preset::Good);
        let taught = parse("--record runs --student brain.json").unwrap();
        assert_eq!(taught.source, Source::Student("brain.json".into()));
        assert_eq!(taught.record, Some("runs".into()));
        assert!(parse("--student").is_err() && parse("--record").is_err());
        let pro = parse("--skill pro --scripted").unwrap();
        assert_eq!(pro.opts().skill, crate::agent::hands::Skill::PRO);
        assert!(parse("--skill").is_err());
        assert!(parse("--skill godlike").is_err());
        assert_eq!(
            parse("--scripted").unwrap().opts().temperament,
            Temperament::Opportunist
        );
        let kos = parse("--temperament kos --scripted").unwrap();
        assert_eq!(kos.opts().temperament, Temperament::Kos);
        assert!(parse("--temperament").is_err());
        assert!(parse("--temperament berserk").is_err());
        assert!(
            !parse("--scripted").unwrap().friend_away_think,
            "autopilot by default"
        );
        assert!(
            parse("--friend-away think --scripted")
                .unwrap()
                .friend_away_think
        );
        assert!(
            !parse("--friend-away autopilot --scripted")
                .unwrap()
                .friend_away_think
        );
        assert!(parse("--friend-away nap").is_err());
        assert!(parse("--friend-away").is_err());
        assert!(
            !parse("--scripted").unwrap().alone_autopilot,
            "think by default"
        );
        assert!(
            parse("--alone autopilot --scripted")
                .unwrap()
                .alone_autopilot
        );
        assert!(parse("--alone nap").is_err());
        let labelled =
            parse("--agent-harness claude-code/2.1 --agent-model test-model --scripted").unwrap();
        assert_eq!(
            labelled.labels(),
            (Some("claude-code/2.1"), Some("test-model"))
        );
        assert!(parse("--agent-harness my\"harness").is_err());
        assert!(parse(&format!("--agent-model {}", "m".repeat(49))).is_err());
        assert_eq!(parse("--scripted").unwrap().labels(), (None, None));
    }

    /// Off this machine the owner is handed the public page, which takes
    /// the shard and the wallet; on it, only a page they name.
    #[test]
    fn an_owner_is_handed_a_watch_link_that_reaches_their_agent() {
        let key = AgentKey::from_secret(&[0x11; 32]).unwrap();
        let wallet = String::from_utf8_lossy(&key.address().to_hex()).into_owned();
        let public = Door::new("game.elopros.com:61234", None, Some(key)).unwrap();
        assert!(!public.loopback());
        assert_eq!(
            public.watch_url(None).unwrap(),
            format!(
                "https://elopros.com/games/gates/?server=game.elopros.com:61234&spectate={wallet}"
            )
        );
        let local = Door::new("127.0.0.1:4433", Some("aa:bb".into()), None).unwrap();
        assert!(local.loopback());
        assert_eq!(local.watch_url(None), None);
        assert_eq!(
            local.watch_url(Some("http://127.0.0.1:8000/")).unwrap(),
            "http://127.0.0.1:8000/?server=127.0.0.1:4433&hash=aa:bb&spectate"
        );
        assert!(
            Door::new("localhost:4433", None, None).is_err(),
            "a guest names an address"
        );
    }
}
