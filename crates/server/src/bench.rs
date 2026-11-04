//! **GatesBench** (`gates-agent --bench MINUTES`): one mind, played on fresh
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
//!
//! **The island starts at [`BENCH_ARC_HOUR`] of the wipe.** Until 2026-10-21
//! it started at hour 0, and a run of up to four hours never reached the
//! first work's opening (`content/arc.toml`, hour 12): every deposit was
//! refused as sealed, so THE ISLAND scored hoard, kills and locks and never
//! the works it pays most for. At hour 48 four works take deposits and none
//! has lit itself (the first fallback is hour 72).

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
/// The hour of the wipe a bench island starts at (`dev_arc_hour`): the
/// Crucible, the Exchange, the Wells and the Observatory open and unpaid,
/// the Spine still sealed, nothing lit by its fallback.
pub const BENCH_ARC_HOUR: u32 = 48;
/// How long a shut-down island may take to write its last standings.
const FLUSH_WAIT: Duration = Duration::from_secs(60);

/// A bot's decisions as its mind took them, counted on the decision worker
/// ([`Counted`]) and read by the run.
#[derive(Debug)]
pub struct Tally {
    decisions: AtomicU32,
    autopilot: AtomicU32,
    /// Every answer by its goal's kind (`Goal::key`), and the plan steps
    /// its scripted policy saw through (`DecisionSource::plan_done`): how
    /// the bot played, beside how well (`behaviour`).
    goals: [AtomicU32; GOAL_KINDS],
    plan_done: AtomicU32,
}

/// The kinds of goal a tally counts: the fixed ones, then those that name
/// a thing or a place.
const NAMED_KINDS: [&str; 6] = ["craft", "buy", "sell", "give", "go", "walk"];
const GOAL_KINDS: usize = crate::mind::Goal::FIXED.len() + NAMED_KINDS.len();

fn kind_slot(key: &str) -> Option<usize> {
    crate::mind::Goal::FIXED
        .iter()
        .position(|g| g.key() == key)
        .or_else(|| {
            NAMED_KINDS
                .iter()
                .position(|k| *k == key)
                .map(|i| crate::mind::Goal::FIXED.len() + i)
        })
}

fn kind_key(slot: usize) -> &'static str {
    match crate::mind::Goal::FIXED.get(slot) {
        Some(g) => g.key(),
        None => NAMED_KINDS[slot - crate::mind::Goal::FIXED.len()],
    }
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            decisions: AtomicU32::new(0),
            autopilot: AtomicU32::new(0),
            goals: std::array::from_fn(|_| AtomicU32::new(0)),
            plan_done: AtomicU32::new(0),
        }
    }
}

impl Tally {
    /// Its answers by kind of goal, so far.
    pub fn goals(&self) -> std::collections::BTreeMap<&'static str, u32> {
        (0..GOAL_KINDS)
            .map(|i| (kind_key(i), self.goals[i].load(Ordering::Relaxed)))
            .filter(|&(_, n)| n > 0)
            .collect()
    }
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
        self.decide_fresh(summary, &mut || None)
    }

    fn decide_fresh(
        &mut self,
        summary: &Summary,
        fresher: &mut dyn FnMut() -> Option<Summary>,
    ) -> Result<Choice, String> {
        let answer = self.inner.decide_fresh(summary, fresher);
        if let Ok(choice) = &answer {
            self.tally.decisions.fetch_add(1, Ordering::Relaxed);
            if crate::autopilot::is_autopilot(choice.reason.as_str()) {
                self.tally.autopilot.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(i) = kind_slot(choice.goal.key()) {
                self.tally.goals[i].fetch_add(1, Ordering::Relaxed);
            }
            self.tally
                .plan_done
                .store(self.inner.plan_done(), Ordering::Relaxed);
        }
        answer
    }

    fn take_note(&mut self) -> Option<crate::bugs::Note> {
        self.inner.take_note()
    }

    fn take_plan(&mut self) -> Option<Plan> {
        self.inner.take_plan()
    }

    fn set_feared(&mut self, feared: bool) {
        self.inner.set_feared(feared);
    }

    fn set_context(&mut self, ctx: &serde_json::Map<String, serde_json::Value>) {
        self.inner.set_context(ctx);
    }

    fn take_extras(&mut self) -> Option<crate::memory::Extras> {
        self.inner.take_extras()
    }

    fn plan_done(&self) -> u32 {
        self.inner.plan_done()
    }
}

/// Shannon entropy, in bits, of a count distribution: 0 for a bot that did
/// one thing, `log2(k)` for one that spread evenly over `k`.
pub fn entropy(counts: &std::collections::BTreeMap<&str, u32>) -> f64 {
    let n: f64 = counts.values().map(|&c| f64::from(c)).sum();
    if n == 0.0 {
        return 0.0;
    }
    -counts
        .values()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / n;
            p * p.log2()
        })
        .sum::<f64>()
}

/// Jensen-Shannon divergence, in bits (0 the same, 1 nothing shared),
/// between two count distributions.
pub fn jsd(
    a: &std::collections::BTreeMap<&str, u32>,
    b: &std::collections::BTreeMap<&str, u32>,
) -> f64 {
    let (na, nb): (f64, f64) = (
        a.values().map(|&c| f64::from(c)).sum(),
        b.values().map(|&c| f64::from(c)).sum(),
    );
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    let keys: std::collections::BTreeSet<&&str> = a.keys().chain(b.keys()).collect();
    let mut d = 0.0;
    for k in keys {
        let p = a.get(*k).map_or(0.0, |&c| f64::from(c) / na);
        let q = b.get(*k).map_or(0.0, |&c| f64::from(c) / nb);
        let m = (p + q) / 2.0;
        if p > 0.0 {
            d += 0.5 * p * (p / m).log2();
        }
        if q > 0.0 {
            d += 0.5 * q * (q / m).log2();
        }
    }
    d.max(0.0)
}

/// How a fleet played, over every run (`gates-bench-behaviour-1`): each
/// bot's goal mix (entropy in bits, the kinds it used, plan steps seen
/// through) and the fleet's spread, the mean Jensen-Shannon divergence
/// between every two bots' mixes: near 0, a fleet of one bot many times.
/// Beside the score, not in it: the platform's `gates-bench-1` takes no
/// keys beyond its own (`meter/bench.py`).
pub fn behaviour<'a>(tallies: impl IntoIterator<Item = &'a Tally>) -> Value {
    let mixes: Vec<(std::collections::BTreeMap<&str, u32>, u32)> = tallies
        .into_iter()
        .map(|t| (t.goals(), t.plan_done.load(Ordering::Relaxed)))
        .collect();
    let mut pairs = 0u32;
    let mut spread = 0.0;
    for i in 0..mixes.len() {
        for j in i + 1..mixes.len() {
            spread += jsd(&mixes[i].0, &mixes[j].0);
            pairs += 1;
        }
    }
    let used: std::collections::BTreeSet<&str> =
        mixes.iter().flat_map(|(g, _)| g.keys().copied()).collect();
    let n = mixes.len().max(1) as f64;
    json!({
        "format": "gates-bench-behaviour-1",
        "bots": mixes.iter().map(|(g, plan)| json!({
            "entropy_bits": entropy(g),
            "distinct_goals": g.len(),
            "decisions": g.values().sum::<u32>(),
            "plan_steps_done": plan,
            "goals": g,
        })).collect::<Vec<_>>(),
        "mean_entropy_bits": mixes.iter().map(|(g, _)| entropy(g)).sum::<f64>() / n,
        "mean_pairwise_jsd_bits": if pairs > 0 { spread / f64::from(pairs) } else { 0.0 },
        "distinct_goals": used.len(),
        "plan_steps_done": mixes.iter().map(|(_, p)| p).sum::<u32>(),
    })
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
    /// Play each run on a lockstep island (`lockstep.rs`) as fast as the
    /// box runs it, not on a loopback shard in wall-clock time (`--warp`).
    /// The minds must decide inline (`MindArgs::inline`).
    pub warp: bool,
}

/// Every bot's mind, built once and played through every run, and the tally
/// beside each.
pub fn minds(mind: &MindArgs, name: &str, bots: usize) -> Result<Vec<(Mind, Arc<Tally>)>, String> {
    (0..bots)
        .map(|i| {
            let tally = Arc::new(Tally::default());
            let mut m = mind.clone();
            m.tally = Some(tally.clone());
            m.cast_body(bot_name(name, i, bots)?.as_str(), None, bots > 1);
            Ok((m.build()?, tally))
        })
        .collect()
}

/// Bot `i`'s setup beyond its mind: its character's temperament included.
fn opts_of(mind: &MindArgs, name: &str, i: usize, bots: usize) -> crate::explorer::SurvivorOpts {
    let mut m = mind.clone();
    if let Ok(n) = bot_name(name, i, bots) {
        m.cast_body(n.as_str(), None, bots > 1);
    }
    m.opts()
}

/// One run on a fresh island: every bot's row. `minds` come back for the
/// next run.
pub async fn run(
    b: &Bench<'_>,
    minds: &mut Vec<(Mind, Arc<Tally>)>,
    run: u32,
) -> Result<Vec<BotRow>, String> {
    let dir = std::env::temp_dir().join(format!("gates-bench-{}-{run}", std::process::id()));
    if b.warp {
        return play_warp(b, minds, run);
    }
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
        let mut opts = opts_of(b.mind, b.name, i, b.bots);
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

/// One run on a lockstep island (`--warp`): the same seed, arc hour and
/// spawn as a loopback bench island, the fleet seated as made-up wallets,
/// played for the run's minutes of island time as fast as the box goes.
fn play_warp(
    b: &Bench<'_>,
    minds: &mut Vec<(Mind, Arc<Tally>)>,
    run: u32,
) -> Result<Vec<BotRow>, String> {
    use crate::agent::lock::{LockCode, LockSecret};
    let cfg = crate::agent_demo::local_config()?;
    let content = content::Content::load_dir(&b.content).map_err(|e| format!("content: {e}"))?;
    let haven = sim_core::terrain::haven(cfg.seed);
    // Each run's clock starts an hour past where the last one could have
    // ended, so a mind carried across never sees time go back.
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let t0 = start + (b.duration + Duration::from_secs(3_600)) * run;
    let mut island =
        crate::lockstep::Island::new(&content, &cfg, haven, BENCH_ARC_HOUR, t0, u64::from(run))?;
    let mut before = Vec::with_capacity(minds.len());
    let mut tallies = Vec::with_capacity(minds.len());
    for (i, (mind, tally)) in minds.drain(..).enumerate() {
        let name = bot_name(b.name, i, b.bots)?;
        let mut opts = opts_of(b.mind, b.name, i, b.bots);
        opts.lock = Some(LockSecret::Code(LockCode::derive(b"warp", name.as_str())));
        before.push(tally.read());
        tallies.push(tally);
        island.seat(Survivor::with(mind, opts), &crate::lockstep::wallet(i))?;
    }
    island.run(b.duration.as_secs() * u64::from(sim_core::limits::TICK_HZ));
    let rules = island.core.standings.rules.clone();
    let mut out = Vec::with_capacity(tallies.len());
    for (i, tally) in tallies.iter().enumerate() {
        let (d0, a0) = before[i];
        let (d1, a1) = tally.read();
        let (decisions, autopilot) = (d1.wrapping_sub(d0), a1.wrapping_sub(a0));
        let row = island.row(i).cloned();
        let at = |b: u8| row.as_ref().map_or(0, |r| r.score(b, &rules));
        out.push(BotRow {
            island: at(crate::standings::BOARD_ISLAND),
            works: at(crate::standings::BOARD_WORKS),
            hoard: at(crate::standings::BOARD_HOARD),
            kills: at(crate::standings::BOARD_FIGHT),
            deaths: row.as_ref().map_or(0, |r| u64::from(r.deaths)),
            decisions,
            answered: decisions.saturating_sub(autopilot),
            autopilot,
            alive_s: island.seats[i].alive_ticks / sim_core::limits::TICK_HZ,
        });
    }
    for (seat, tally) in island.seats.drain(..).zip(tallies) {
        minds.push((seat.bot.into_mind(), tally));
    }
    Ok(out)
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
        assert_eq!(
            v["runs"][0].as_object().unwrap().len(),
            2,
            "island and bots"
        );
        assert_eq!(v["score"].as_object().unwrap().len(), 3, "n, mean, ci95");
        // What the platform recomputes on submit: a run is its bots' mean,
        // the score the runs'.
        let runs = v["runs"].as_array().unwrap();
        for r in runs {
            let bots = r["bots"].as_array().unwrap();
            let mean = bots
                .iter()
                .map(|b| b["island"].as_f64().unwrap())
                .sum::<f64>()
                / bots.len() as f64;
            assert!((r["island"].as_f64().unwrap() - mean).abs() < 1e-9);
        }
        let mean = runs
            .iter()
            .map(|r| r["island"].as_f64().unwrap())
            .sum::<f64>()
            / runs.len() as f64;
        assert!((v["score"]["mean"].as_f64().unwrap() - mean).abs() < 1e-9);
        assert!(v.to_string().len() <= 64 * 1024);
    }

    #[test]
    fn the_spread_of_a_fleet_is_zero_for_clones_and_one_for_strangers() {
        let mix = |pairs: &[(&'static str, u32)]| pairs.iter().copied().collect();
        let a = mix(&[("gather_wood", 5), ("build", 5)]);
        let b = mix(&[("raid", 3), ("loot", 3)]);
        assert!((entropy(&a) - 1.0).abs() < 1e-12);
        assert_eq!(entropy(&mix(&[("explore", 9)])), 0.0);
        assert!(jsd(&a, &a).abs() < 1e-12);
        assert!((jsd(&a, &b) - 1.0).abs() < 1e-12);
        let t = [Tally::default(), Tally::default()];
        for (tally, mix) in t.iter().zip([&a, &b]) {
            for (k, n) in mix {
                tally.goals[kind_slot(k).unwrap()].store(*n, Ordering::Relaxed);
            }
        }
        t[1].plan_done.store(3, Ordering::Relaxed);
        let v = behaviour(&t);
        assert_eq!(v["distinct_goals"], 4);
        assert_eq!(v["plan_steps_done"], 3);
        assert!((v["mean_pairwise_jsd_bits"].as_f64().unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(v["bots"][0]["goals"]["build"], 5);
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
