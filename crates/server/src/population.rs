//! **Resident inhabitants**: a shard's own bots, seated over its own wire.
//!
//! Three consecutive judges ranked the same gap first (`findings/
//! pass-20260813-230343-{14,16,18}-judge.md` §B.1): *"a shard has no
//! inhabitants, so none of the last four passes' work is reachable by a
//! player."* Every opponent in this tree lives inside a `#[test]` or needs an
//! operator to start `bin/bots` by hand, and a human who joins the alpha
//! shard meets a silent island. `bots.rs` already proves the raid profile
//! produces a real raid when it is run; the missing piece was a shard that
//! runs it.
//!
//! **Over the wire, deliberately.** An inhabitant is not a shortcut into
//! `ShardCore` — it dials `handle.local_addr` with the same
//! [`crate::botclient::run_bot`] a load test uses, through the full
//! QUIC/WebTransport handshake, and the server cannot tell it from a player.
//! A second producer feeding `push_input` directly would be a second
//! implementation of the input path, and the interesting bugs (a refusal
//! computed on values the client did not predict, the item-move verb that
//! fails as a disconnect) all live in the half it would skip.
//!
//! **Resident, not a one-shot fleet.** This is the whole difference from
//! `bin/bots`, which hands each bot a year and never looks again: a bot whose
//! connection dies there silently shrinks the population and nothing notices.
//! Here a slot is a *post* — it runs a bounded shift, reports, and is
//! re-manned until the shard shuts down. That also gives the shift end a
//! place to read the `BotReport` counters, which is the complaint `bots.rs`'s
//! own header makes about its year default: a bot that never stops never
//! reports.

use crate::botclient::{bot_endpoint, run_bot, BotReport};
use sim_core::bots::RaidRows;
use sim_core::limits::MAX_PLAYERS;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How long one inhabitant's shift runs before it reconnects.
///
/// The knob's two ends: short churns handshakes and reseats every plot,
/// long makes the counters unreadable and hides a wedged bot for as long as
/// it lasts. Five minutes is well over the 300-tick fuse (10 s) and over the
/// `RAID_CYCLE` the plan re-seats on, so a shift always contains whole raids
/// rather than clipping one.
///
/// Proposed default, `DECISIONS.md` §open ("shard population v0").
pub const DEFAULT_SHIFT_SECS: u64 = 300;

/// Wait before re-manning a post whose shift ended in an error.
///
/// It exists so a shard that cannot reach itself — a bind that came up half
/// way, an endpoint refusing — spins at 0.5 Hz instead of as fast as the
/// reactor allows. Not a latency number: the success path does not wait.
///
/// Proposed default, `DECISIONS.md` §open ("shard population v0").
pub const RECONNECT_BACKOFF_MS: u64 = 2_000;

/// Gap between launching one post and the next. `bots.rs`'s constant and its
/// reason: 50 handshakes at once is a TLS thundering herd.
const LAUNCH_STAGGER_MS: u64 = 20;

/// Hotbar slot the attacker selects for its charge, and the container slot
/// the two `Move` steps address. Slot **addresses**, not balance numbers —
/// the ones `sim-core/tests/raid_storm.rs` drives the same profile with.
/// `DECISIONS.md` §open carries them in case a fleet ever needs to vary them.
pub const RAID_CHARGE_SLOT: u8 = 0;
pub const RAID_GOODS_SLOT: u8 = 2;
/// The code every owner bot sets and every attacker bot must guess. One
/// shared value is the point: `lock.rs`'s shock/lockout ladder only runs on
/// a *wrong* guess, and with 10,000 codes a random draw is wrong ~99.99% of
/// the time whatever this is.
pub const RAID_OWNER_CODE: u16 = 4242;

/// The raid profile's rows, resolved out of **already-loaded** content by id.
///
/// By id and never typed: wall 7 says a row number is content, and `bake.rs`
/// ranks ids, so inserting one item renumbers its neighbours and a hard-coded
/// row would silently address the wrong piece.
///
/// It takes the content rather than a path because the shard has already
/// loaded and validated its own `content_dir`; re-reading a
/// `CARGO_MANIFEST_DIR`-relative tree, which is what `bin/bots` must do, would
/// be a *second* content set inside a process that has one.
pub fn raid_rows(content: &content::Content) -> Result<RaidRows, String> {
    let row = |what: &str, got: Option<u16>| got.ok_or_else(|| format!("content: no {what}"));
    Ok(RaidRows {
        // Twig: the editable draft tier (`reference/BUILDING.md` §7b), so a
        // bot plants what a bot can afford.
        foundation: row(
            "build.foundation_twig",
            content.piece_index("build.foundation_twig"),
        )?,
        wall: row("build.wall_twig", content.piece_index("build.wall_twig"))?,
        container: row("item.box_small", content.deploy_index("item.box_small"))?,
        lock: row("item.lock_code", content.deploy_index("item.lock_code"))?,
        charge_slot: RAID_CHARGE_SLOT,
        goods_slot: RAID_GOODS_SLOT,
        code: RAID_OWNER_CODE,
    })
}

/// What the population itself can say, as distinct from what the shard says
/// about it. `ShardStats` already counts joins, inputs and actions and cannot
/// tell an inhabitant from a player — which is the point — so these are the
/// supervisor's own facts: how many posts are manned right now, and what the
/// shifts that ended reported.
#[derive(Default, Debug)]
pub struct PopulationStats {
    /// Posts currently holding a live session. The number a report line
    /// should print, because it is the only one that goes *down*.
    pub live: AtomicU64,
    /// Shifts begun and shifts that returned a report.
    pub shifts_started: AtomicU64,
    pub shifts_ended: AtomicU64,
    /// Shifts that ended in a transport error instead of a report. Non-zero
    /// on a healthy shard means the population is fighting its own wire.
    pub shift_errors: AtomicU64,
    // ---- summed off every `BotReport` a finished shift returned -----------
    pub inputs_sent: AtomicU64,
    pub actions_sent: AtomicU64,
    pub pieces_placed: AtomicU64,
    pub deploys_placed: AtomicU64,
    pub charges_planted: AtomicU64,
    pub struct_hits: AtomicU64,
}

impl PopulationStats {
    /// Read one counter. `Relaxed` because every one of these is a gauge read
    /// for a human, never a value another thread orders against.
    pub fn get(c: &AtomicU64) -> u64 {
        c.load(Ordering::Relaxed)
    }

    fn absorb(&self, r: &BotReport) {
        for (c, v) in [
            (&self.inputs_sent, r.inputs_sent),
            (&self.actions_sent, r.actions_sent),
            (&self.pieces_placed, r.pieces_placed),
            (&self.deploys_placed, r.deploys_placed),
            (&self.charges_planted, r.charges_planted),
            (&self.struct_hits, r.struct_hits),
        ] {
            c.fetch_add(v, Ordering::Relaxed);
        }
    }
}

/// How many inhabitants a shard may seat.
///
/// **Bounded, and refused rather than clamped** (wall 4). A population at
/// `MAX_PLAYERS` is a shard whose every slot is a bot: a human dials it and
/// meets `REFUSE_FULL`, which is the silent island this whole module exists
/// to fix, arriving as a locked door instead. So the cap is `MAX_PLAYERS - 1`
/// — at least one seat is always a person's — and a config over it says so at
/// boot rather than seating what fits and dropping the rest.
pub const MAX_POPULATION: usize = MAX_PLAYERS - 1;

/// The address an inhabitant dials, given the address the shard **bound**.
///
/// They are not the same and the difference is a real failure: a public shard
/// binds `0.0.0.0:4433`, and `0.0.0.0` is a wildcard to listen on, not a host
/// to connect to — dialling it is `connect: … invalid remote address` on
/// Linux, so the whole population would fail every shift on the one config
/// that is not a test's. The bound port is always right; only the host has to
/// be repaired, and loopback is the correct repair because the dialler is
/// inside the process that is listening.
pub fn dial_addr(bound: SocketAddr) -> SocketAddr {
    if bound.ip().is_unspecified() {
        let host = match bound {
            SocketAddr::V4(_) => std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            SocketAddr::V6(_) => std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        };
        SocketAddr::new(host, bound.port())
    } else {
        bound
    }
}

/// A live population. Dropping it does **not** stop the posts; the shutdown
/// flag handed to [`spawn_population`] does, which is the same flag that
/// stops the sim thread, so a shard has exactly one stop.
pub struct Population {
    pub stats: Arc<PopulationStats>,
    /// The posts, so a caller that wants to *wait* for the population to
    /// drain can. The supervisor loops end on the shutdown flag.
    posts: Vec<tokio::task::JoinHandle<()>>,
}

impl Population {
    /// Wait for every post to notice the shutdown flag and end. Bounded only
    /// by the posts themselves — each checks the flag between reactor wakeups
    /// rather than on a clock.
    pub async fn join(self) {
        for p in self.posts {
            let _ = p.await;
        }
    }

    pub fn live(&self) -> u64 {
        PopulationStats::get(&self.stats.live)
    }
}

/// Seat `count` inhabitants against `server` and keep them seated.
///
/// One shared endpoint (one UDP socket, many QUIC connections) and one task
/// per post. `raid` supplies the content rows the raid profile addresses;
/// `None` walks and sends no action, which is a population that is scenery
/// rather than an opponent.
///
/// Returns an error only for the things that make the whole population
/// impossible — a count over [`MAX_POPULATION`], or an endpoint that will not
/// bind. A single post that cannot connect is not an error: it is counted in
/// `shift_errors` and re-manned, because the shard it dials may simply not
/// have finished coming up.
pub fn spawn_population(
    count: usize,
    server: SocketAddr,
    raid: Option<RaidRows>,
    shift: Duration,
    shutdown: Arc<AtomicBool>,
) -> Result<Population, String> {
    if count > MAX_POPULATION {
        return Err(format!(
            "population {count} is over the {MAX_POPULATION} cap — a shard \
             whose every slot is a bot refuses the player it was seated for"
        ));
    }
    let stats = Arc::new(PopulationStats::default());
    let endpoint = Arc::new(bot_endpoint()?);
    let mut posts = Vec::with_capacity(count);
    for i in 0..count {
        let endpoint = endpoint.clone();
        let stats = stats.clone();
        let shutdown = shutdown.clone();
        posts.push(tokio::spawn(async move {
            // Stagger inside the task rather than between spawns: the caller
            // is a boot path and must not block for `count * 20 ms`.
            sleep_unless_stopped(
                Duration::from_millis(LAUNCH_STAGGER_MS * i as u64),
                &shutdown,
            )
            .await;
            // The stream index decides attacker-or-owner inside `run_bot`
            // (`seed_stream % 2 == 1`), so a post keeps its role across
            // shifts. A population is therefore half attackers and half
            // owners however many posts you seat — and the same half every
            // time, which is what makes a base outlive the raider who is
            // shooting at it.
            man_the_post(i as u64, &endpoint, server, raid, shift, &stats, &shutdown).await;
        }));
    }
    Ok(Population { stats, posts })
}

/// A seated population, plus the two facts the boot line has to print.
pub struct Seated {
    pub population: Population,
    /// Where the posts actually dialled — [`dial_addr`] of the bound address,
    /// which is not the bound address on any shard that is not a test's.
    pub dial: SocketAddr,
    /// `None` = the inhabitants raid. `Some(why)` = they walk, and this is
    /// the reason, for the caller to say out loud. Surfaced rather than
    /// logged in here because a library that printed would print in every
    /// test too, and because a **silent** downgrade to scenery is exactly
    /// the "pass it didn't earn" shape the trap list warns about.
    pub no_raid: Option<String>,
}

/// Seat whatever `cfg.population` asks for against a shard that has just
/// bound `bound`. `Ok(None)` is the shipping default: no key, no inhabitants.
///
/// This is the whole of the boot decision — the count, the content rows, the
/// wildcard-bind repair — in one function rather than in `bin/shard.rs`,
/// because a binary is the one thing in this crate no test can call.
pub fn seat(
    cfg: &crate::config::ShardConfig,
    content: &content::Content,
    bound: SocketAddr,
    shutdown: Arc<AtomicBool>,
) -> Result<Option<Seated>, String> {
    if cfg.population == 0 {
        return Ok(None);
    }
    let (rows, no_raid) = match raid_rows(content) {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e)),
    };
    let dial = dial_addr(bound);
    let population = spawn_population(
        cfg.population,
        dial,
        rows,
        Duration::from_secs(DEFAULT_SHIFT_SECS),
        shutdown,
    )?;
    Ok(Some(Seated {
        population,
        dial,
        no_raid,
    }))
}

/// One post, for the life of the shard: shift, report, re-man.
async fn man_the_post(
    stream: u64,
    endpoint: &wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>,
    server: SocketAddr,
    raid: Option<RaidRows>,
    shift: Duration,
    stats: &PopulationStats,
    shutdown: &AtomicBool,
) {
    while !shutdown.load(Ordering::Relaxed) {
        stats.shifts_started.fetch_add(1, Ordering::Relaxed);
        stats.live.fetch_add(1, Ordering::Relaxed);
        // Raced against the flag rather than run to completion: a shift is
        // five minutes and a `systemctl stop` may not wait for one. Dropping
        // the future closes the QUIC connection, which the shard sees as the
        // leave it would see from any client that walked away.
        let outcome = tokio::select! {
            r = run_bot(endpoint, server, stream, shift, raid) => Some(r),
            () = stopped(shutdown) => None,
        };
        stats.live.fetch_sub(1, Ordering::Relaxed);
        match outcome {
            Some(Ok(report)) => {
                stats.shifts_ended.fetch_add(1, Ordering::Relaxed);
                stats.absorb(&report);
            }
            Some(Err(_)) => {
                // Not printed per post: `count` posts failing the same way
                // would be `count` identical lines a second. The shard's
                // report line carries the count, which is the readable form.
                stats.shift_errors.fetch_add(1, Ordering::Relaxed);
                sleep_unless_stopped(Duration::from_millis(RECONNECT_BACKOFF_MS), shutdown).await;
            }
            // Shut down mid-shift: no report, no backoff, no re-man.
            None => return,
        }
    }
}

/// Resolve when the shutdown flag is set. Polled rather than notified because
/// the flag is an `AtomicBool` shared with the sim thread, which is not async
/// and must not grow a channel to satisfy this file.
async fn stopped(shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(SHUTDOWN_POLL_MS)).await;
    }
}

/// Sleep, but wake early if the shard is stopping.
async fn sleep_unless_stopped(d: Duration, shutdown: &AtomicBool) {
    tokio::select! {
        () = tokio::time::sleep(d) => {}
        () = stopped(shutdown) => {}
    }
}

/// How often a post looks at the shutdown flag while it is otherwise idle.
/// A shutdown-latency number and nothing else: it bounds how long a
/// `systemctl stop` waits on the population, not anything a player sees.
const SHUTDOWN_POLL_MS: u64 = 50;
