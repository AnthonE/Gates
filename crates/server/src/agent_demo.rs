//! Shared setup for the headless survivor and its rendered local broadcast.

/// Explicit offline goals (`mind::Scripted`), never a fallback for a
/// failed model request. Re-exported here for the bins that name it.
pub use crate::mind::Scripted;
use crate::mind::{Mind, MindConfig};
use std::time::Duration;

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
        let m = parse("--max-requests-hour 60 --heartbeat-s 45 --external python3 agent.py --x").unwrap();
        assert_eq!(
            m.source,
            Source::External("python3".into(), vec!["agent.py".into(), "--x".into()])
        );
        assert_eq!((m.cfg.per_hour, m.cfg.heartbeat.as_secs()), (60, 45));
        assert!(parse("--think-ms 999").is_err(), "never under once a second");
        assert!(parse("--max-requests-day 0").is_err());
        assert!(parse("--heartbeat-s").is_err());
        assert_eq!(parse("--scripted").unwrap().source, Source::Scripted);
    }
}
