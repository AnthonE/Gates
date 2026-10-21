//! A private island stepped in lockstep, as fast as the box runs it.
//!
//! GatesBench's islands are real shards on loopback, played over the wire
//! in wall-clock time (`bench.rs`): ten minutes of play is ten minutes. A
//! mind that answers on the caller's thread — the scripted policy, a
//! distilled student (`distill.rs`) — does not need the wall clock, and a
//! breeding run that scores hundreds of islands cannot afford it. This is
//! the same island without the network: one `ShardCore` assembled as the
//! shard's boot assembles it (`net::install_tables`, the standings, the
//! salts, the town's stations), the agents' `Survivor`s seated in its
//! slots, and every tick their frames in, the tick, and the snapshots and
//! events back out — the arena rig's lockstep (`tests/arena/rig.rs`) with a
//! whole island and a whole fleet. A tenth of a second of play costs what
//! the sim and the agents cost, and nothing else.
//!
//! **What it keeps of the shard's loop**: the once-a-minute standings pass
//! (minutes played, every base recounted, the boards rebuilt), because
//! THE ISLAND is read off it. What it leaves: saves, the wipe clock, admin,
//! chat and voice — a bench island has nobody to hear them.
//!
//! **A run is a function of its inputs.** The clock is synthetic, the salts
//! are the seed's, the minds answer inline: the same fleet on the same seed
//! scores the same.

use crate::botclient::BotDriver;
use crate::core::{Lane, ShardCore};
use crate::explorer::Survivor;
use crate::stats::ShardStats;
use crate::store::PlayerKey;
use crate::view::ClientView;
use protocol::{decode_action, decode_input, encode_input, InputDatagram};
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, MAX_PLAYERS, TICK_HZ};
use std::time::{Duration, Instant};

/// The standings pass's phase in the minute: the shard's loop runs it on
/// tick 13 of every 60 s (`net.rs`).
const MINUTE_PHASE: u64 = 13;

/// One seated agent.
pub struct Seat {
    pub bot: Survivor,
    pub view: ClientView,
    pub id: u32,
    /// Its wallet as the standings key it (`0x` and forty hex digits).
    pub key: String,
    /// Ticks it spent standing (alive, not on the death screen).
    pub alive_ticks: u32,
}

/// A lockstep island and its fleet.
pub struct Island {
    pub core: Box<ShardCore>,
    pub seats: Vec<Seat>,
    stats: ShardStats,
    seed: u64,
    variant: u64,
    haven: sim_core::terrain::Haven,
    t0: Instant,
    /// Ticks stepped since the island was made.
    pub ticks: u64,
}

impl Island {
    /// A fresh island from `content` on `cfg`'s seed, its arc clock at
    /// `arc_hour` of the wipe, everyone spawning at `cfg.dev_spawn`, its
    /// synthetic clock starting at `t0`. `variant` picks the wipe's salts
    /// (where the outbreaks erupt, the puzzles' answers) and the bodies'
    /// ids, which seed each agent's hands and routes: the same variant is
    /// the same island, and another plays out otherwise. `haven` must be
    /// `terrain::haven(cfg.seed)`.
    ///
    /// `t0` is the caller's because minds outlive islands: a mind carried
    /// from one island to the next keeps its last decision's time, and an
    /// island whose clock started earlier than that would hold the mind
    /// back until its clock caught up.
    pub fn new(
        content: &content::Content,
        cfg: &crate::config::ShardConfig,
        haven: sim_core::terrain::Haven,
        arc_hour: u32,
        t0: Instant,
        variant: u64,
    ) -> Result<Self, String> {
        let tables = crate::net::bake_all(content)?;
        let mut core = Box::new(ShardCore::with_haven(cfg.seed, haven));
        core.world.dev_spawn = cfg.dev_spawn;
        let (rules, achievements) = crate::net::install_tables(&mut core, tables);
        core.teams = crate::team::Teams::open(cfg.team_max, None);
        core.standings = crate::standings::Standings::open(rules, cfg.prizes.clone(), None, 0);
        core.standings.exits = cfg.exits;
        core.achievements = crate::achievements::Achievements::open(achievements, None);
        core.world.tick = u64::from(arc_hour) * sim_core::blight::TICKS_PER_HOUR;
        // Salts from the seed and the variant rather than fresh ones: a
        // warped island replays.
        core.world.arc.salt = salt(cfg.seed, 2 * variant + 1);
        core.world
            .set_blight_salt(salt(cfg.seed, 2 * variant + 2) as u32 | 1);
        core.world.seed_authored();
        Ok(Self {
            core,
            seats: Vec::new(),
            stats: ShardStats::default(),
            seed: cfg.seed,
            variant,
            haven,
            t0,
            ticks: 0,
        })
    }

    /// Seat `bot` in the next slot as wallet `key` (a standings key,
    /// `0x` and forty hex digits). Its index in [`Island::seats`].
    pub fn seat(&mut self, mut bot: Survivor, key: &str) -> Result<usize, String> {
        let slot = self.seats.len();
        if slot >= MAX_PLAYERS {
            return Err(format!("a lockstep island seats at most {MAX_PLAYERS}"));
        }
        let id = 1 + slot as u32 + (self.variant % 1_000_000) as u32 * MAX_PLAYERS as u32;
        let pk = PlayerKey::new(key.as_bytes()).ok_or("a wallet key is 0x and hex")?;
        self.core
            .connect_as(slot, id, Some(pk), None)
            .ok_or("the island refused the seat")?;
        bot.know_haven(self.seed, self.haven);
        bot.welcome(&protocol::Welcome {
            seed: self.seed,
            player_id: id,
            tick: self.core.world.tick as u32,
            dev: true,
            blight_salt: self.core.world.blight_salt,
        });
        self.seats.push(Seat {
            bot,
            view: ClientView::new(),
            id,
            key: key.to_string(),
            alive_ticks: 0,
        });
        Ok(slot)
    }

    /// One tick: every agent's frame (and action, when its slot is free)
    /// in, the standings pass on its minute, the tick, and what each client
    /// is sent back out to it.
    pub fn step(&mut self) {
        let now = self.now();
        let mut act = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        for (slot, s) in self.seats.iter_mut().enumerate() {
            let frame = s.bot.frame_at(&s.view, s.id, self.ticks as u16, now);
            if s.bot.core().is_some_and(|c| !c.dead && c.hp > 0) {
                s.alive_ticks += 1;
            }
            let (seq, ack) = s.view.ack_fields();
            let mut dg = InputDatagram::new(seq, ack, crate::botclient::AGENT_PLAYOUT_REPORT);
            if dg.push(frame).is_ok() {
                let mut bytes = [0u8; DATAGRAM_BUDGET_BYTES];
                if let Ok(len) = encode_input(&dg, &mut bytes) {
                    if let Ok(d) = decode_input(&bytes[..len]) {
                        self.core.push_input(slot, &d);
                    }
                }
            }
            if self.core.wants_action(slot) {
                if let Some(len) = s.bot.action(&mut act) {
                    if let Ok(msg) = decode_action(&act[..len]) {
                        self.core.push_action(slot, msg);
                    }
                }
            }
        }
        if self.core.world.tick % (60 * TICK_HZ as u64) == MINUTE_PHASE {
            self.core.standings_minute();
            self.core.standings_recount();
            self.core.standings.rebuild();
        }
        let seats = &mut self.seats;
        self.core.tick_bare(&self.stats, |lane, slot, bytes| {
            if let Some(s) = seats.get_mut(slot) {
                match lane {
                    Lane::Snapshot => {
                        let _ = s.view.apply(bytes);
                    }
                    Lane::Event => {
                        let _ = s.bot.event(bytes);
                    }
                }
            }
            true
        });
        self.ticks += 1;
    }

    /// Where the synthetic clock stands.
    pub fn now(&self) -> Instant {
        self.t0 + Duration::from_secs_f64(self.ticks as f64 / f64::from(TICK_HZ))
    }

    /// Step `ticks` ticks.
    pub fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    /// The boards as they stand: a last pass, then seat `i`'s row, if the
    /// standings have one for it.
    pub fn row(&mut self, i: usize) -> Option<&crate::standings::Row> {
        self.core.standings_recount();
        self.core.standings.rebuild();
        self.core.standings.row_of(&self.seats.get(i)?.key)
    }
}

/// A salt from the seed: never zero, which is "not salted yet".
fn salt(seed: u64, k: u64) -> u64 {
    let mut z = seed ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) | 1
}

/// Bot `i`'s wallet on a lockstep island: made up, and the same every run.
pub fn wallet(i: usize) -> String {
    format!("0x{:040x}", 0x10c_4570_0000u64 + i as u64)
}
