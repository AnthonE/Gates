//! The play log: the shard's own analytics, as JSON lines, one file a day.
//!
//! What a player did and how the shard ran, kept first-party on the box
//! (`ALPHA.md` §3's session metrics, no third party). Five kinds of line:
//!
//! - `boot` — the shard came up: seed, build, protocol, wipe.
//! - `door` — a connection reached the accept loop: address, client release
//!   and build, wallet, agent/spectator, and whether it was seated or why
//!   not. Written by the accept loop.
//! - `join` / `leave` — a body was seated, and left: how it came in (fresh,
//!   restored, took over its sleeper), where it stood, how long it played,
//!   its round trip, and a tally of the session (kills, deaths, crafts,
//!   builds, gathers, hits, chat, refusals, …). The leave repeats the door's
//!   address and build so a session reads off one line.
//! - one line per thing worth counting in play: deaths with cause, killer,
//!   weapon and range; crafts, builds, upgrades, deployables, raids
//!   (charges and what fell), research, the exchange, trades, shops, the
//!   casino, the arc, wounds and revives, and the refusals a player hit.
//! - `pulse` — once a minute: the world's population (awake, sleeping,
//!   animals, pieces, deployables, bags), the whole `/status.json`
//!   document, and the process's memory.
//!
//! The sim thread pushes `Copy` records onto an `rtrb` ring (wall 3: it
//! never formats, locks or allocates on this path); the accept loop pushes
//! door records onto a second ring; one plain thread owns the files, names
//! things and writes. A full ring drops the record and counts it
//! (`play_dropped`, watched by the anomaly log). Files are
//! `<path>.YYYY-MM-DD` (UTC) and are deleted after `play_log_days`.

use crate::stats::ShardStats;
use crate::store::PlayerKey;
use serde_json::{json, Map, Value};
use sim_core::limits::MAX_PLAYERS;
use std::collections::BTreeMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Records in flight from the sim thread. A tick emits at most a few per
/// player in practice; this holds seconds of a busy shard behind a slow disk.
pub const PLAY_RING_CAP: usize = 8192;
/// Door records in flight from the accept loop.
pub const DOOR_RING_CAP: usize = 512;
/// How often the sim thread states the world's population.
pub const PULSE_SECS: u64 = 60;

/// What a session did, kept per slot on the sim thread and written whole on
/// the leave. Plain counters; a reader sums them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub kills: u32,
    pub deaths: u32,
    pub animals: u32,
    pub wounded: u32,
    pub revives: u32,
    pub respawns: u32,
    pub hits: u32,
    pub damage_dealt: u32,
    pub damage_taken: u32,
    pub shots: u32,
    pub gathered: u32,
    pub crafted: u32,
    pub built: u32,
    pub upgraded: u32,
    pub deployed: u32,
    pub charges: u32,
    pub researched: u32,
    pub ate: u32,
    pub drank: u32,
    pub extracted: u32,
    pub gave: u32,
    pub traded: u32,
    pub gambled: u32,
    pub refused: u32,
    pub chat: u32,
    pub rtt_ms_sum: u64,
    pub rtt_samples: u32,
    pub rtt_ms_max: u32,
}

/// How a join came in (`core::Admitted`), as the log names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Fresh,
    Restored,
    TookOver,
}

/// What a play line is about. The sim side picks it and fills `a`/`b`/`c`
/// as each variant's comment says; the writer names the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    /// who died; a killer id (0: no hand), b cause, c item << 16 | range dm.
    Death,
    /// a 1 when the body woke on its bag.
    Respawn,
    /// a item, b units.
    Craft,
    /// a piece row, b shape, c material.
    Build,
    /// a material.
    Upgrade,
    /// a deploy row, b item.
    Deploy,
    /// No actor: a piece row fell (decay or a raid). a row.
    PieceGone,
    /// who owned it; a deploy row, b item.
    DeployGone,
    /// A charge stuck to a wall.
    Charge,
    /// a animal kind.
    Animal,
    /// a recipe, b item, c coin burned.
    Research,
    /// a exchange (work), b units of coin taken.
    Extract,
    /// a item, b work, c units.
    Gave,
    /// a offer, b times.
    Vend,
    /// a machine box, b op, c row << 8 | times.
    Shop,
    /// a card door, b 1 from the lever.
    Swipe,
    /// a colour, b stake.
    WheelBet,
    /// a staked, b paid.
    WheelPaid,
    /// a machine, b multiplier, c payout.
    Slots,
    /// a `PAID_*`, b the shard's tag.
    Paid,
    /// a work, b `WORK_EV_*`.
    Work,
    /// a mechanism.
    Solved,
    /// a op, b target.
    Arc,
    /// a chance per mille.
    Wounded,
    /// a chance per mille, b hp.
    Recovered,
    /// a the helper.
    Revived,
    /// a the sim event code refused, b its reason.
    Refused,
}

/// One record from the sim thread. `Copy`, integers and fixed bytes only.
#[derive(Clone, Copy)]
pub enum Rec {
    Join {
        tick: u64,
        slot: u8,
        id: u32,
        key: Option<PlayerKey>,
        how: How,
        agent: bool,
    },
    Leave {
        tick: u64,
        slot: u8,
        id: u32,
        key: Option<PlayerKey>,
        ticks: u64,
        tally: Tally,
        at: (i32, i32),
    },
    Watch {
        tick: u64,
        slot: u16,
        target: u32,
        seated: bool,
    },
    Play {
        tick: u64,
        what: What,
        who: u32,
        a: u32,
        b: u32,
        c: u32,
        at: (i32, i32),
    },
    Pulse {
        tick: u64,
        awake: u16,
        sleepers: u16,
        dead: u16,
        animals: u16,
        pieces: u32,
        deploys: u32,
        bags: u32,
    },
}

/// How a door ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Door {
    Seated,
    Watching,
    Banned,
    InUse,
    Parked,
    Full,
    PerSource,
    AgentSeats,
}

/// One connection at the accept loop.
#[derive(Clone, Copy)]
pub struct DoorRec {
    pub door: Door,
    pub slot: u16,
    pub id: u32,
    pub ip: Option<IpAddr>,
    pub ver: u32,
    pub build: u64,
    pub flags: u8,
    pub key: Option<PlayerKey>,
}

/// The sim thread's end, and every session's tally.
pub struct Tap {
    tx: Option<rtrb::Producer<Rec>>,
    stats: Option<Arc<ShardStats>>,
    tally: Box<[Tally; MAX_PLAYERS]>,
    since: [u64; MAX_PLAYERS],
}

impl Tap {
    /// No log: every push is a no-op. What tests and community shards run.
    pub fn off() -> Self {
        Self {
            tx: None,
            stats: None,
            tally: Box::new([Tally::default(); MAX_PLAYERS]),
            since: [0; MAX_PLAYERS],
        }
    }

    pub fn on(&self) -> bool {
        self.tx.is_some()
    }

    pub fn push(&mut self, rec: Rec) {
        let Some(tx) = self.tx.as_mut() else {
            return;
        };
        if tx.push(rec).is_err() {
            if let Some(s) = self.stats.as_ref() {
                ShardStats::bump(&s.play_dropped);
            }
        }
    }

    /// A tally to bump, for a connected slot. `None` when the log is off.
    pub fn tally(&mut self, slot: usize) -> Option<&mut Tally> {
        self.tx.as_ref()?;
        self.tally.get_mut(slot)
    }

    pub fn join(
        &mut self,
        tick: u64,
        slot: usize,
        id: u32,
        key: Option<PlayerKey>,
        how: How,
        agent: bool,
    ) {
        if slot >= MAX_PLAYERS {
            return;
        }
        self.tally[slot] = Tally::default();
        self.since[slot] = tick;
        self.push(Rec::Join {
            tick,
            slot: slot as u8,
            id,
            key,
            how,
            agent,
        });
    }

    pub fn leave(
        &mut self,
        tick: u64,
        slot: usize,
        id: u32,
        key: Option<PlayerKey>,
        at: (i32, i32),
    ) {
        if slot >= MAX_PLAYERS {
            return;
        }
        let tally = std::mem::take(&mut self.tally[slot]);
        self.push(Rec::Leave {
            tick,
            slot: slot as u8,
            id,
            key,
            ticks: tick.saturating_sub(self.since[slot]),
            tally,
            at,
        });
    }

    /// One round-trip sample for a slot, in µs.
    pub fn rtt(&mut self, slot: usize, us: u32) {
        if us == 0 {
            return;
        }
        if let Some(t) = self.tally(slot) {
            let ms = us.div_ceil(1000);
            t.rtt_ms_sum += u64::from(ms);
            t.rtt_samples += 1;
            t.rtt_ms_max = t.rtt_ms_max.max(ms);
        }
    }
}

/// The accept loop's end.
pub struct DoorTap {
    tx: Option<rtrb::Producer<DoorRec>>,
    stats: Option<Arc<ShardStats>>,
}

impl DoorTap {
    pub fn off() -> Self {
        Self {
            tx: None,
            stats: None,
        }
    }

    pub fn push(&mut self, rec: DoorRec) {
        let Some(tx) = self.tx.as_mut() else {
            return;
        };
        if tx.push(rec).is_err() {
            if let Some(s) = self.stats.as_ref() {
                ShardStats::bump(&s.play_dropped);
            }
        }
    }
}

/// What the `boot` line says about this shard.
pub struct Boot {
    pub seed: u64,
    pub domain: String,
    pub wipe: u32,
}

/// Open the log and start its thread. The directory is made if missing.
pub fn spawn(
    path: PathBuf,
    keep_days: u32,
    boot: Boot,
    items: Vec<String>,
    stats: Arc<ShardStats>,
) -> std::io::Result<(Tap, DoorTap)> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    // Opened here so a path that cannot be written refuses the boot.
    let mut files = Files::new(path, keep_days);
    files.open(now_ms())?;
    let (tx, rx) = rtrb::RingBuffer::<Rec>::new(PLAY_RING_CAP);
    let (door_tx, door_rx) = rtrb::RingBuffer::<DoorRec>::new(DOOR_RING_CAP);
    let mut w = Writer {
        files,
        items,
        stats: stats.clone(),
        doors: BTreeMap::new(),
    };
    std::thread::Builder::new()
        .name("playlog".into())
        .spawn(move || {
            w.line(json!({
                "ev": "boot",
                "seed": boot.seed,
                "domain": boot.domain,
                "wipe": boot.wipe,
                "build": protocol::version::BUILD_ID,
                "proto": protocol::PROTO_VER,
                "max_players": MAX_PLAYERS,
            }));
            w.run(rx, door_rx);
        })?;
    Ok((
        Tap {
            tx: Some(tx),
            stats: Some(stats.clone()),
            ..Tap::off()
        },
        DoorTap {
            tx: Some(door_tx),
            stats: Some(stats),
        },
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The day files.
struct Files {
    path: PathBuf,
    keep_days: u32,
    day: i64,
    file: Option<std::io::BufWriter<std::fs::File>>,
}

impl Files {
    fn new(path: PathBuf, keep_days: u32) -> Self {
        Self {
            path,
            keep_days,
            day: i64::MIN,
            file: None,
        }
    }

    fn open(&mut self, ms: u64) -> std::io::Result<()> {
        let day = (ms / 86_400_000) as i64;
        if day == self.day && self.file.is_some() {
            return Ok(());
        }
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
        }
        self.day = day;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(crate::scribe::day_file(&self.path, day))?;
        self.file = Some(std::io::BufWriter::new(f));
        crate::scribe::prune(&self.path, day, self.keep_days);
        Ok(())
    }

    fn write(&mut self, line: &str) {
        if let Err(e) = self.open(now_ms()) {
            eprintln!("playlog: {}: {e}", self.path.display());
            return;
        }
        if let Some(f) = self.file.as_mut() {
            if f.write_all(line.as_bytes()).is_err() || f.write_all(b"\n").is_err() {
                self.file = None;
            }
        }
    }

    fn flush(&mut self) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }
}

/// What the door said about a seated id, repeated on its leave.
#[derive(Clone)]
struct Seen {
    ip: Option<IpAddr>,
    ver: u32,
    build: u64,
}

struct Writer {
    files: Files,
    items: Vec<String>,
    stats: Arc<ShardStats>,
    doors: BTreeMap<u32, Seen>,
}

impl Writer {
    fn run(&mut self, mut rx: rtrb::Consumer<Rec>, mut door_rx: rtrb::Consumer<DoorRec>) {
        loop {
            // Doors first: a join's door is pushed before its `Connect`.
            while let Ok(d) = door_rx.pop() {
                let v = self.door(&d);
                self.line(v);
            }
            while let Ok(r) = rx.pop() {
                let v = self.rec(&r);
                self.line(v);
            }
            self.files.flush();
            if rx.is_abandoned() {
                while let Ok(r) = rx.pop() {
                    let v = self.rec(&r);
                    self.line(v);
                }
                self.line(json!({"ev": "close"}));
                self.files.flush();
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn line(&mut self, mut v: Value) {
        if let Some(o) = v.as_object_mut() {
            // `t` first on the line, for a reader eyeballing it.
            let mut out = Map::with_capacity(o.len() + 1);
            out.insert("t".into(), json!(now_ms()));
            out.append(o);
            v = Value::Object(out);
        }
        self.files.write(&v.to_string());
    }

    fn item(&self, i: u32) -> Value {
        match self.items.get(i as usize) {
            Some(n) if !n.is_empty() => json!(n),
            _ => json!(i),
        }
    }

    fn door(&mut self, d: &DoorRec) -> Value {
        let (ma, mi, pa) = protocol::version::unpack(d.ver);
        let door = match d.door {
            Door::Seated => "seated",
            Door::Watching => "watching",
            Door::Banned => "banned",
            Door::InUse => "in_use",
            Door::Parked => "parked",
            Door::Full => "full",
            Door::PerSource => "per_source",
            Door::AgentSeats => "agent_seats",
        };
        if d.door == Door::Seated {
            // Bounded by the seats: a leave takes its entry, and a seat
            // whose leave was dropped is overwritten by its next tenant's
            // id only if the ids collide, so cap it outright.
            if self.doors.len() >= 4 * MAX_PLAYERS {
                self.doors.clear();
            }
            self.doors.insert(
                d.id,
                Seen {
                    ip: d.ip,
                    ver: d.ver,
                    build: d.build,
                },
            );
        }
        json!({
            "ev": "door",
            "door": door,
            "slot": d.slot,
            "id": d.id,
            "ip": d.ip.map(|ip| ip.to_string()),
            "client": format!("{ma}.{mi}.{pa}"),
            "client_build": format!("{:016x}", d.build),
            "agent": d.flags & protocol::HELLO_AGENT != 0,
            "watchable": d.flags & protocol::HELLO_WATCHABLE != 0,
            "spectate": d.flags & protocol::HELLO_SPECTATE != 0,
            "wallet": wallet(d.key.as_ref()),
        })
    }

    fn rec(&mut self, r: &Rec) -> Value {
        match *r {
            Rec::Join {
                tick,
                slot,
                id,
                key,
                how,
                agent,
            } => json!({
                "ev": "join",
                "tick": tick,
                "slot": slot,
                "id": id,
                "wallet": wallet(key.as_ref()),
                "how": match how {
                    How::Fresh => "fresh",
                    How::Restored => "restored",
                    How::TookOver => "took_over",
                },
                "agent": agent,
            }),
            Rec::Leave {
                tick,
                slot,
                id,
                key,
                ticks,
                tally,
                at,
            } => {
                let seen = self.doors.remove(&id);
                let mut v = json!({
                    "ev": "leave",
                    "tick": tick,
                    "slot": slot,
                    "id": id,
                    "wallet": wallet(key.as_ref()),
                    "secs": ticks / sim_core::limits::TICK_HZ as u64,
                    "x": metres(at.0),
                    "z": metres(at.1),
                    "rtt_ms_avg": (tally.rtt_ms_sum / u64::from(tally.rtt_samples.max(1))),
                    "rtt_ms_max": tally.rtt_ms_max,
                    "tally": tally_json(&tally),
                });
                if let (Some(s), Some(o)) = (seen, v.as_object_mut()) {
                    let (ma, mi, pa) = protocol::version::unpack(s.ver);
                    o.insert("ip".into(), json!(s.ip.map(|ip| ip.to_string())));
                    o.insert("client".into(), json!(format!("{ma}.{mi}.{pa}")));
                    o.insert("client_build".into(), json!(format!("{:016x}", s.build)));
                }
                v
            }
            Rec::Watch {
                tick,
                slot,
                target,
                seated,
            } => json!({
                "ev": "watch",
                "tick": tick,
                "slot": slot,
                "target": target,
                "seated": seated,
            }),
            Rec::Play {
                tick,
                what,
                who,
                a,
                b,
                c,
                at,
            } => {
                let mut v = self.play(what, a, b, c);
                if let Some(o) = v.as_object_mut() {
                    o.insert("tick".into(), json!(tick));
                    if who != 0 {
                        o.insert("who".into(), json!(who));
                    }
                    if at != (0, 0) {
                        o.insert("x".into(), json!(metres(at.0)));
                        o.insert("z".into(), json!(metres(at.1)));
                    }
                }
                v
            }
            Rec::Pulse {
                tick,
                awake,
                sleepers,
                dead,
                animals,
                pieces,
                deploys,
                bags,
            } => {
                let status: Value = serde_json::from_str(&crate::status::status_body(&self.stats))
                    .unwrap_or(Value::Null);
                json!({
                    "ev": "pulse",
                    "tick": tick,
                    "awake": awake,
                    "sleepers": sleepers,
                    "dead": dead,
                    "animals": animals,
                    "pieces": pieces,
                    "deploys": deploys,
                    "bags": bags,
                    "rss_kb": rss_kb(),
                    "net_lost_packets": ShardStats::get(&self.stats.net_lost_packets),
                    "net_sent_packets": ShardStats::get(&self.stats.net_sent_packets),
                    "net_rtt_us_max": ShardStats::get(&self.stats.net_rtt_us_max),
                    "joins": ShardStats::get(&self.stats.joins),
                    "leaves": ShardStats::get(&self.stats.leaves),
                    "ticks_dropped": ShardStats::get(&self.stats.ticks_dropped),
                    "play_dropped": ShardStats::get(&self.stats.play_dropped),
                    "status": status,
                })
            }
        }
    }

    fn play(&self, what: What, a: u32, b: u32, c: u32) -> Value {
        use sim_core::world::*;
        match what {
            What::Death => json!({
                "ev": "death",
                "killer": (a != 0).then_some(a),
                "by_animal": a & sim_core::limits::MOB_ID_TAG != 0,
                "cause": cause(b as u8),
                "weapon": (c >> 16 != u32::from(sim_core::gather::NO_ITEM)).then(|| self.item(c >> 16)),
                "range_m": f64::from(c & 0xffff) / 10.0,
            }),
            What::Respawn => json!({"ev": "respawn", "on_bag": a == 1}),
            What::Craft => json!({"ev": "craft", "item": self.item(a), "units": b}),
            What::Build => json!({"ev": "build", "piece": a, "shape": b, "material": c}),
            What::Upgrade => json!({"ev": "upgrade", "material": a}),
            What::Deploy => json!({"ev": "deploy", "row": a, "item": self.item(b)}),
            What::PieceGone => json!({"ev": "piece_gone", "piece": a}),
            What::DeployGone => json!({"ev": "deploy_gone", "row": a, "item": self.item(b)}),
            What::Charge => json!({"ev": "charge"}),
            What::Animal => json!({"ev": "animal_kill", "kind": a}),
            What::Research => {
                json!({"ev": "research", "recipe": a, "item": self.item(b), "cost": c})
            }
            What::Extract => json!({"ev": "extract", "exchange": a, "coin": b}),
            What::Gave => json!({"ev": "gave", "item": self.item(a), "work": b, "units": c}),
            What::Vend => json!({"ev": "vend", "offer": a, "times": b}),
            What::Shop => json!({
                "ev": "shop", "box": a, "op": b, "row": c >> 8, "times": c & 0xff,
            }),
            What::Swipe => json!({"ev": "swipe", "door": a, "lever": b == 1}),
            What::WheelBet => json!({"ev": "wheel_bet", "colour": a, "stake": b}),
            What::WheelPaid => json!({"ev": "wheel_paid", "staked": a, "paid": b}),
            What::Slots => {
                json!({"ev": "slots", "machine": a, "multiplier": b, "payout": c})
            }
            What::Paid => json!({
                "ev": "paid",
                "result": match a {
                    PAID_DONE => "done",
                    PAID_SHORT => "short",
                    PAID_NO_BODY => "no_body",
                    _ => "unknown",
                },
                "tag": b,
            }),
            What::Work => json!({"ev": "work", "work": a, "state": b}),
            What::Solved => json!({"ev": "mech_solved", "mech": a}),
            What::Arc => json!({"ev": "arc", "op": a, "target": b}),
            What::Wounded => json!({"ev": "wounded", "chance_pm": a}),
            What::Recovered => json!({"ev": "recovered", "chance_pm": a, "hp": b}),
            What::Revived => json!({"ev": "revived", "helper": a}),
            What::Refused => json!({"ev": "refused", "verb": refused_verb(a as u8), "reason": b}),
        }
    }
}

fn tally_json(t: &Tally) -> Value {
    json!({
        "kills": t.kills, "deaths": t.deaths, "animals": t.animals,
        "wounded": t.wounded, "revives": t.revives, "respawns": t.respawns,
        "hits": t.hits, "damage_dealt": t.damage_dealt, "damage_taken": t.damage_taken,
        "shots": t.shots, "gathered": t.gathered, "crafted": t.crafted,
        "built": t.built, "upgraded": t.upgraded, "deployed": t.deployed,
        "charges": t.charges, "researched": t.researched, "ate": t.ate,
        "drank": t.drank, "extracted": t.extracted, "gave": t.gave,
        "traded": t.traded, "gambled": t.gambled, "refused": t.refused,
        "chat": t.chat,
    })
}

/// A key as the log writes it: the wallet's text, lowercased.
fn wallet(key: Option<&PlayerKey>) -> Option<String> {
    key.and_then(|k| core::str::from_utf8(k.as_bytes()).ok())
        .map(str::to_ascii_lowercase)
}

/// Body quanta to metres, to the decimetre.
fn metres(q: i32) -> f64 {
    (f64::from(q) * f64::from(sim_core::movement::POS_XZ_Q) * 10.0).round() / 10.0
}

fn cause(c: u8) -> &'static str {
    const NAMES: [&str; 15] = [
        "hand", "clock", "salt", "arrow", "animal", "charge", "bullet", "cold", "fall", "heat",
        "fel", "cactus", "spikes", "bleed", "drown",
    ];
    NAMES.get(c as usize).copied().unwrap_or("other")
}

fn refused_verb(code: u8) -> &'static str {
    use sim_core::world::*;
    match code {
        EV_CRAFT_REFUSED => "craft",
        EV_BUILD_REFUSED => "build",
        EV_DEPLOY_REFUSED => "deploy",
        EV_RESEARCH_REFUSED => "research",
        EV_VEND_REFUSED => "vend",
        EV_SWIPE_REFUSED => "swipe",
        EV_ARC_REFUSED => "arc",
        EV_SHOP_REFUSED => "shop",
        EV_WHEEL_REFUSED => "wheel",
        EV_SLOTS_REFUSED => "slots",
        _ => "other",
    }
}

/// Resident memory, from `/proc` (0 where there is none).
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer() -> Writer {
        Writer {
            files: Files::new(PathBuf::from("/nonexistent/playlog"), 1),
            items: vec!["Wood".into(), "Stone".into()],
            stats: Arc::new(ShardStats::default()),
            doors: BTreeMap::new(),
        }
    }

    #[test]
    fn a_session_reads_off_its_leave_line() {
        let mut w = writer();
        let key = PlayerKey::new(b"0xABCDEF").unwrap();
        let door = w.door(&DoorRec {
            door: Door::Seated,
            slot: 3,
            id: 259,
            ip: Some("203.0.113.9".parse().unwrap()),
            ver: protocol::version::pack(1, 2, 3),
            build: 0xfeed,
            flags: 0,
            key: Some(key),
        });
        assert_eq!(door["door"], "seated");
        assert_eq!(door["client"], "1.2.3");
        let tally = Tally {
            kills: 2,
            crafted: 5,
            rtt_ms_sum: 90,
            rtt_samples: 3,
            rtt_ms_max: 40,
            ..Tally::default()
        };
        let leave = w.rec(&Rec::Leave {
            tick: 900,
            slot: 3,
            id: 259,
            key: Some(key),
            ticks: 600,
            tally,
            at: (0, 0),
        });
        assert_eq!(leave["wallet"], "0xabcdef");
        assert_eq!(leave["secs"], 600 / sim_core::limits::TICK_HZ as u64);
        assert_eq!(leave["ip"], "203.0.113.9");
        assert_eq!(leave["client"], "1.2.3");
        assert_eq!(leave["rtt_ms_avg"], 30);
        assert_eq!(leave["tally"]["kills"], 2);
        assert_eq!(leave["tally"]["crafted"], 5);
        assert!(w.doors.is_empty(), "the leave spends its door");
    }

    #[test]
    fn play_lines_name_their_items() {
        let w = writer();
        let v = w.play(What::Craft, 1, 4, 0);
        assert_eq!(v["ev"], "craft");
        assert_eq!(v["item"], "Stone");
        assert_eq!(w.play(What::Craft, 99, 1, 0)["item"], 99);
        let d = w.play(What::Death, 5, 3, (1 << 16) | 125);
        assert_eq!(d["cause"], "arrow");
        assert_eq!(d["weapon"], "Stone");
        assert_eq!(d["range_m"], 12.5);
    }

    #[test]
    fn an_off_tap_keeps_no_tally() {
        let mut t = Tap::off();
        assert!(t.tally(0).is_none());
        t.rtt(0, 5000);
        t.join(1, 0, 1, None, How::Fresh, false);
    }
}
