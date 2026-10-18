//! **GatesBench** (`jev-bot --bench MINUTES`): one mind, played on fresh
//! private islands and scored on the boards a wipe pays (`standings.rs`).
//!
//! Each run boots its own loopback island from the local seed, seats the
//! fleet as throwaway wallets so the standings key every bot, plays MINUTES
//! of wall clock, and shuts the island down, which writes
//! `<base>.standings`; each bot's row is read back from it. A run scores the
//! mean of its bots' THE ISLAND, and the result is the mean over runs with a
//! 95% interval. Beside the score each bot says how many decisions it took,
//! how many its own mind answered and how many the autopilot played, and how
//! long it was alive: a harness that misses its turns is scored on the
//! scripted policy's play, and the result says so.
//!
//! It measures a mind playing alone on a quiet island: no people, nobody but
//! its own fleet. A submitted result is the submitter's word.

use crate::agent_demo::{bot_name, lock_secret, Door, MindArgs};
use crate::botclient::BotDriver;
use crate::explorer::Survivor;
use crate::mind::{Choice, DecisionSource, Mind, Plan, SourceKind, Summary};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// What a result says it is.
pub const FORMAT: &str = "gates-bench-1";
/// The longest run, and the most runs, the platform takes.
pub const MINUTES_MAX: u32 = 240;
pub const RUNS_MAX: u32 = 32;
pub const RUNS_DEFAULT: u32 = 3;
/// How long a shut-down island may take to write its last standings.
const FLUSH_WAIT: Duration = Duration::from_secs(60);

/// A bot's decisions as its mind took them, counted on the decision worker
/// ([`Counted`]) and read by the run.
#[derive(Debug, Default)]
pub struct Tally {
    decisions: AtomicU32,
    autopilot: AtomicU32,
}

impl Tally {
    /// `(decisions, of those the autopilot's)` so far.
    pub fn read(&self) -> (u32, u32) {
        (
            self.decisions.load(Ordering::Relaxed),
            self.autopilot.load(Ordering::Relaxed),
        )
    }
}

/// A [`DecisionSource`] that counts every answer its source gives and who
/// gave it, then passes it on unchanged.
pub struct Counted<S> {
    inner: S,
    tally: Arc<Tally>,
}

impl<S: DecisionSource> Counted<S> {
    pub fn new(inner: S, tally: Arc<Tally>) -> Self {
        Self { inner, tally }
    }
}

impl<S: DecisionSource> DecisionSource for Counted<S> {
    fn kind(&self) -> SourceKind {
        self.inner.kind()
    }

    fn decide(&mut self, summary: &Summary) -> Result<Choice, String> {
        let answer = self.inner.decide(summary);
        if let Ok(choice) = &answer {
            self.tally.decisions.fetch_add(1, Ordering::Relaxed);
            if crate::autopilot::is_autopilot(choice.reason.as_str()) {
                self.tally.autopilot.fetch_add(1, Ordering::Relaxed);
            }
        }
        answer
    }

    fn take_note(&mut self) -> Option<crate::bugs::Note> {
        self.inner.take_note()
    }

    fn take_plan(&mut self) -> Option<Plan> {
        self.inner.take_plan()
    }
}

/// The survivor's own frames, and the ticks its body was alive for.
struct Clocked<'a> {
    survivor: &'a mut Survivor,
    alive: u32,
}

impl BotDriver for Clocked<'_> {
    fn welcome(&mut self, welcome: &protocol::Welcome) {
        self.survivor.welcome(welcome);
    }
    fn event(&mut self, bytes: &[u8]) -> Result<(), protocol::WireError> {
        self.survivor.event(bytes)
    }
    fn frame(
        &mut self,
        view: &crate::view::ClientView,
        player_id: u32,
        seq: u16,
    ) -> sim_core::input::InputFrame {
        let f = self.survivor.frame(view, player_id, seq);
        if self.survivor.core().is_some_and(|c| !c.dead && c.hp > 0) {
            self.alive += 1;
        }
        f
    }
    fn action(&mut self, out: &mut [u8]) -> Option<usize> {
        self.survivor.action(out)
    }
    fn say(&mut self, out: &mut [u8]) -> Option<usize> {
        self.survivor.say(out)
    }
    fn voice_in(&mut self, frame: &protocol::VoiceFrame) {
        self.survivor.voice_in(frame);
    }
    fn voice_out(&mut self, out: &mut [protocol::VoiceFrame]) -> usize {
        self.survivor.voice_out(out)
    }
}

/// One bot's run: its standings row and what its mind did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BotRow {
    pub island: u64,
    pub works: u64,
    pub hoard: u64,
    pub kills: u64,
    pub deaths: u64,
    pub decisions: u32,
    pub answered: u32,
    pub autopilot: u32,
    pub alive_s: u32,
}

impl BotRow {
    fn json(&self) -> Value {
        json!({
            "island": self.island,
            "works": self.works,
            "hoard": self.hoard,
            "kills": self.kills,
            "deaths": self.deaths,
            "decisions": self.decisions,
            "answered": self.answered,
            "autopilot": self.autopilot,
            "alive_s": self.alive_s,
        })
    }
}

/// What a result says about where it ran and what played.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub commit: String,
    pub proto: u32,
    pub seed: u64,
    pub layout: u64,
    pub minutes: u32,
    pub bots: u32,
    pub mind: &'static str,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub think_ms: u32,
}

/// The commit this build came from: `GATES_COMMIT` at build time, else the
/// one `protocol` stamped (what an F7 report names). A dirty or unknown
/// build is `unknown`, since no commit reproduces it.
pub fn commit() -> String {
    let sha = option_env!("GATES_COMMIT").unwrap_or(protocol::version::GIT_SHA);
    if (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        sha.to_ascii_lowercase()
    } else {
        "unknown".into()
    }
}

/// `(n, mean, ci95)` of the runs' scores: the 95% half-width is
/// `1.96 * sd / sqrt(n)` with the sample sd, and 0 under two runs.
pub fn score(islands: &[f64]) -> (usize, f64, f64) {
    let n = islands.len();
    if n == 0 {
        return (0, 0.0, 0.0);
    }
    let mean = islands.iter().sum::<f64>() / n as f64;
    if n < 2 {
        return (n, mean, 0.0);
    }
    let var = islands.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1) as f64;
    (n, mean, 1.96 * var.sqrt() / (n as f64).sqrt())
}

/// A run's score: the mean of its bots' THE ISLAND.
fn run_island(bots: &[BotRow]) -> f64 {
    if bots.is_empty() {
        return 0.0;
    }
    bots.iter().map(|b| b.island as f64).sum::<f64>() / bots.len() as f64
}

/// The `gates-bench-1` result.
pub fn result(h: &Header, runs: &[Vec<BotRow>]) -> Value {
    let islands: Vec<f64> = runs.iter().map(|r| run_island(r)).collect();
    let (n, mean, ci95) = score(&islands);
    json!({
        "format": FORMAT,
        "commit": h.commit,
        "proto": h.proto,
        "seed": h.seed,
        "layout": format!("{:016x}", h.layout),
        "minutes": h.minutes,
        "bots": h.bots,
        "mind": h.mind,
        "harness": h.harness,
        "model": h.model,
        "think_ms": h.think_ms,
        "runs": runs.iter().zip(&islands).map(|(bots, island)| json!({
            "island": island,
            "bots": bots.iter().map(BotRow::json).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "score": {"n": n, "mean": mean, "ci95": ci95},
    })
}

/// What one bench plays: `bots` bodies of `mind` for `duration` a run.
pub struct Bench<'a> {
    pub mind: &'a MindArgs,
    pub bots: usize,
    pub duration: Duration,
    pub name: &'a str,
    pub content: PathBuf,
}

/// Every bot's mind, built once and played through every run, and the tally
/// beside each.
pub fn minds(mind: &MindArgs, bots: usize) -> Result<Vec<(Mind, Arc<Tally>)>, String> {
    (0..bots)
        .map(|_| {
            let tally = Arc::new(Tally::default());
            let mut m = mind.clone();
            m.tally = Some(tally.clone());
            Ok((m.build()?, tally))
        })
        .collect()
}

/// One run on a fresh island: every bot's row. `minds` come back for the
/// next run.
pub async fn run(
    b: &Bench<'_>,
    minds: &mut Vec<(Mind, Arc<Tally>)>,
    run: u32,
) -> Result<Vec<BotRow>, String> {
    let dir = std::env::temp_dir().join(format!("gates-bench-{}-{run}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let out = play(b, minds, &dir).await;
    let _ = std::fs::remove_dir_all(&dir);
    out
}

async fn play(
    b: &Bench<'_>,
    minds: &mut Vec<(Mind, Arc<Tally>)>,
    dir: &Path,
) -> Result<Vec<BotRow>, String> {
    let base = dir.join("island.world");
    let shard = crate::agent_demo::spawn_bench(&b.content, &base).await?;
    let server = shard.local_addr.to_string();
    let mut fleet = tokio::task::JoinSet::new();
    let mut wallets = Vec::with_capacity(minds.len());
    let mut before = Vec::with_capacity(minds.len());
    let endpoint = Arc::new(crate::botclient::agent_endpoint(
        &server,
        Some(&shard.cert_hash),
    )?);
    for (i, (mind, tally)) in minds.drain(..).enumerate() {
        let name = bot_name(b.name, i, b.bots)?;
        // Throwaway: made in the run's own directory and gone with it.
        let key = agentkey::AgentKey::create(&dir.join(format!("bot-{i}.key")))
            .map_err(|e| e.to_string())?;
        wallets.push(String::from_utf8_lossy(&key.address().to_hex()).into_owned());
        let mut opts = b.mind.opts();
        opts.lock = lock_secret(Some(&key), name.as_str(), &dir.join("guest.secret"));
        let door = Door::new(server.clone(), Some(shard.cert_hash.clone()), Some(key))?;
        before.push(tally.read());
        let (endpoint, duration) = (endpoint.clone(), b.duration);
        fleet.spawn(async move {
            let mut survivor = Survivor::with(mind, opts);
            let mut clocked = Clocked {
                survivor: &mut survivor,
                alive: 0,
            };
            let played = door.play(&endpoint, name, duration, &mut clocked).await;
            let alive = clocked.alive;
            (i, survivor.into_mind(), tally, alive, played)
        });
    }
    let mut back: Vec<Option<(Mind, Arc<Tally>, u32)>> = (0..wallets.len()).map(|_| None).collect();
    let mut failed = None;
    while let Some(joined) = fleet.join_next().await {
        let (i, mind, tally, alive, played) = joined.map_err(|e| format!("bot task: {e}"))?;
        if let Err(e) = played {
            failed.get_or_insert(format!("bot {i}: {e}"));
        }
        back[i] = Some((mind, tally, alive));
    }
    endpoint.close(0u32.into(), b"bench run over");
    // The island's last word: its standings, written as it shuts down.
    shard.shutdown.store(true, Ordering::Relaxed);
    let flushed = tokio::time::Instant::now() + FLUSH_WAIT;
    while !crate::stats::ShardStats::raised(&shard.stats.store_stopped) {
        if tokio::time::Instant::now() > flushed {
            return Err("the bench island did not write its standings in time".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let rows = crate::standings::saved_rows(&base);
    let mut out = Vec::with_capacity(back.len());
    for (i, slot) in back.into_iter().enumerate() {
        let (mind, tally, alive) = slot.ok_or("a bot never came back")?;
        let (d0, a0) = before[i];
        let (d1, a1) = tally.read();
        let (decisions, autopilot) = (d1.wrapping_sub(d0), a1.wrapping_sub(a0));
        // No row is a bot that scored nothing and played under a minute.
        let s = rows.get(&wallets[i]);
        let at = |b: usize| s.map_or(0, |s| s.scores[b]);
        out.push(BotRow {
            island: at(crate::standings::BOARD_ISLAND as usize),
            works: at(crate::standings::BOARD_WORKS as usize),
            hoard: at(crate::standings::BOARD_HOARD as usize),
            kills: at(crate::standings::BOARD_FIGHT as usize),
            deaths: s.map_or(0, |s| u64::from(s.deaths)),
            decisions,
            answered: decisions.saturating_sub(autopilot),
            autopilot,
            alive_s: alive / sim_core::limits::TICK_HZ,
        });
        minds.push((mind, tally));
    }
    failed.map_or(Ok(out), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_is_the_sample_sds_and_zero_for_one_run() {
        assert_eq!(score(&[]), (0, 0.0, 0.0));
        assert_eq!(score(&[12.0]), (1, 12.0, 0.0));
        let (n, mean, ci) = score(&[10.0, 20.0, 30.0]);
        assert_eq!((n, mean), (3, 20.0));
        // sd 10, so 1.96 * 10 / sqrt(3).
        assert!((ci - 11.316_065).abs() < 1e-5, "{ci}");
    }

    #[test]
    fn a_result_is_gates_bench_1() {
        let h = Header {
            commit: "unknown".into(),
            proto: u32::from(protocol::PROTO_VER),
            seed: 20_260_731,
            layout: 0xab,
            minutes: 2,
            bots: 2,
            mind: "scripted",
            harness: None,
            model: Some("test-model".into()),
            think_ms: 1000,
        };
        let bot = |island| BotRow {
            island,
            decisions: 10,
            answered: 8,
            autopilot: 2,
            alive_s: 100,
            ..BotRow::default()
        };
        let v = result(&h, &[vec![bot(10), bot(20)], vec![bot(30), bot(30)]]);
        assert_eq!(v["format"], FORMAT);
        assert_eq!(v["layout"], "00000000000000ab");
        assert_eq!(v["seed"], 20_260_731);
        assert!(v["harness"].is_null() && v["model"] == "test-model");
        assert_eq!(v["runs"][0]["island"], 15.0);
        assert_eq!(v["runs"][1]["bots"][0]["island"], 30);
        assert_eq!(v["runs"][1]["bots"][0]["autopilot"], 2);
        assert_eq!(v["score"]["n"], 2);
        assert_eq!(v["score"]["mean"], 22.5);
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in [
            "format", "commit", "proto", "seed", "layout", "minutes", "bots", "mind", "harness",
            "model", "think_ms", "runs", "score",
        ] {
            assert!(keys.contains(&k), "{k}");
        }
        assert_eq!(keys.len(), 13, "nothing beyond C3: {keys:?}");
        let bot_keys = v["runs"][0]["bots"][0].as_object().unwrap().len();
        assert_eq!(bot_keys, 9);
        assert!(v.to_string().len() <= 64 * 1024);
    }

    #[test]
    fn a_commit_is_hex_or_unknown() {
        let c = commit();
        assert!(
            c == "unknown" || c.bytes().all(|b| b.is_ascii_hexdigit()),
            "{c}"
        );
    }
}
