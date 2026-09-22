//! Shared setup for the headless survivor and its rendered local broadcast.

use crate::botclient::{
    agent_endpoint, run_agent_bot, run_guest_agent, AgentIdentity, BotDriver, BotReport,
};
/// Explicit offline goals (`mind::Scripted`), never a fallback for a
/// failed model request. Re-exported here for the bins that name it.
pub use crate::mind::Scripted;
use crate::mind::{Mind, MindConfig};
use agentkey::AgentKey;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::Endpoint;

/// The display name a jev bot declares to its spectators when none is given.
pub const AGENT_NAME: &str = "jev";

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
pub const MIND_USAGE: &str = "[--scripted | --external PROGRAM [ARG...]] [--think-ms 1000] [--timeout-ms 3000] [--heartbeat-s 30] [--max-requests-hour 600] [--max-requests-day 7200]\nJev needs TYPESAFE_API_KEY. --scripted makes no model calls. --external runs your own agent as a child speaking JSON lines (JEV.md) and must come last. Decisions are never more often than once a second.";

/// Which source decides: Jev, the explicit scripted policy, or an agent
/// the operator brings. Never swapped for another at runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Jev,
    Scripted,
    External(String, Vec<String>),
}

#[derive(Clone, Debug)]
pub struct MindArgs {
    pub source: Source,
    pub cfg: MindConfig,
}

impl Default for MindArgs {
    fn default() -> Self {
        Self {
            source: Source::Jev,
            cfg: MindConfig::default(),
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
            "--external" => {
                let program = args.next().ok_or("--external needs a program")?;
                self.source = Source::External(program, args.collect());
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub fn label(&self) -> String {
        match &self.source {
            Source::Jev => format!("Jev {}", crate::jev::MODEL),
            Source::Scripted => "SCRIPTED goals; no model calls".into(),
            Source::External(program, _) => format!("EXTERNAL agent: {program}"),
        }
    }

    /// Check the configuration and credentials, then start the source.
    pub fn build(&self) -> Result<Mind, String> {
        self.cfg.check()?;
        match &self.source {
            Source::Scripted => Mind::new(Scripted::default(), self.cfg),
            Source::External(program, args) => Mind::new(
                crate::external::External::spawn(program, args, self.cfg.timeout)?,
                self.cfg,
            ),
            Source::Jev => {
                let key = std::env::var("TYPESAFE_API_KEY").map_err(|_| {
                    "set TYPESAFE_API_KEY, or choose --scripted or --external explicitly"
                })?;
                Mind::new(crate::jev::Jev::new(key, self.cfg.timeout)?, self.cfg)
            }
        }
    }
}

pub async fn spawn_local() -> Result<crate::net::ShardHandle, String> {
    spawn_local_with_content(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
    )
    .await
}

pub async fn spawn_local_with_content(
    content_dir: &std::path::Path,
) -> Result<crate::net::ShardHandle, String> {
    let mut cfg = crate::config::parse_shard_toml(include_str!("../../../shard.toml.example"))?;
    cfg.bind = "127.0.0.1:0".parse().expect("loopback");
    cfg.require_auth = false;
    cfg.population = 0;
    cfg.dev_spawn = Some(sim_core::world::World::new(cfg.seed).spawn_pos(0));
    // Spectator seats open (`NETCODE.md` §2.3). The bind is loopback, so
    // only a viewer on this machine can reach them; a remote viewer needs a
    // routable bind and a fixed UDP port, which is an operator's call.
    cfg.spectate = crate::config::Spectate::on();
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
    }
}
