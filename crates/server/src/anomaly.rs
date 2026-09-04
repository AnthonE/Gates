//! The anomaly log — **the evidence an alpha is supposed to produce**
//! (`ALPHA.md` §3, and §6's gate: "zero silent failures in the anomaly
//! log"). One append-only JSONL file, written off the sim thread.
//!
//! ## What was wrong before this existed
//!
//! Fifty-one counters (`stats.rs`) recorded every refusal, drop, forged
//! datagram and failed write this server can see, and **nothing read them
//! but a 10-second console line nobody keeps**. So the shard could tell you
//! that eleven datagrams were forged and could never tell you *when*, in
//! which tick, or near which of the fifty things that happened next. The
//! alpha gate asks for zero silent failures; a counter with no timeline is
//! a silent failure with a number attached.
//!
//! ## One producer, and it is the sim thread
//!
//! The rings here are `rtrb` — SPSC, like every other seam in this crate —
//! so there is exactly one producer, and the sim thread is the only
//! candidate that matters: an anomaly wants a **tick**, and the tick is
//! sim state. Net-thread anomalies (a bad frame, a rate limit, a forged
//! button) reach the log by a route that costs their call sites nothing:
//! the sim thread **samples the counters each tick and logs the deltas**
//! ([`Watch`]). That turns all fifty-one into logged events without
//! touching one of their forty-odd sites, and it cannot drift — a counter
//! added tomorrow is watched by adding one row to a table, not by
//! remembering to log at a new call site.
//!
//! The cost is honest and stated: a delta says *how many* happened in that
//! tick's window, never which client. When a line needs a subject — an
//! admin act, a `/bug` — the sim thread writes a record directly, with the
//! id and the position it already holds.
//!
//! ## Formatting is the log thread's, never the sim's
//!
//! Wall 3 bans `String`/`format!` on the sim thread, so a record crossing
//! the ring is **integers and a fixed byte array**. The JSON is built on
//! the log thread, which is a plain `std::thread` that owns the `File` —
//! `store_thread`'s shape, for `store_thread`'s reason (blocking file I/O
//! does not belong on a tokio worker or on a 30 Hz clock).
//!
//! ## Bounded, per wall 4
//!
//! The ring is [`ANOMALY_RING_CAP`] deep and overflow **drops with a
//! counter** (`anomaly_dropped`) — a log is diagnostics, and a diagnostic
//! that could stall the sim thread would be a worse bug than the one it
//! was recording. The file grows without bound by design: it is a wipe-
//! cycle artifact an operator rotates or deletes, and truncating the
//! record of a bad hour to save bytes is the opposite of the point.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use crate::stats::ShardStats;

/// Records in flight between the sim thread and the log thread. Sized off
/// what a bad tick looks like rather than a rate: the delta sweep can emit
/// at most one record per watched counter (there are [`WATCHED`]`.len()`)
/// plus a handful of direct writes, so a full ring means the disk has
/// stopped keeping up rather than that the number was too small.
pub const ANOMALY_RING_CAP: usize = 256;

/// The longest note a record carries, bytes — a `/bug`'s text, which
/// arrives inside an already-sanitized `ChatText`.
pub const NOTE_MAX: usize = 48;

/// What kind of thing happened. Integer-coded like every other diagnostic
/// in this crate; the log thread turns it into the string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// A watched counter moved. `a` is the delta.
    Counter = 0,
    /// A player filed a bug. `a`/`b`/`c` are the position quanta.
    Bug = 1,
    /// An admin ran a verb. `a`/`b` are its arguments.
    AdminAct = 2,
    /// A slash line was refused: unknown verb, or the caller is not an
    /// admin. Logged because a *refused* admin attempt is exactly the
    /// thing an operator wants to see afterwards.
    AdminRefused = 3,
}

/// One line, before it is a line. `Copy` and integer-only so it crosses
/// the ring under wall 3.
#[derive(Clone, Copy, Debug)]
pub struct Record {
    pub tick: u64,
    pub kind: Kind,
    /// Which counter, which verb, which refusal — the `Kind`'s subject.
    /// For `Counter` it indexes [`WATCHED`]; for the admin kinds it is a
    /// verb code; for `Bug` it is unused.
    pub what: u16,
    /// The player this is about, or 0 for "the shard itself".
    pub who: u32,
    pub a: i64,
    pub b: i64,
    pub c: i64,
    pub note: [u8; NOTE_MAX],
    pub note_len: u8,
}

impl Default for Record {
    fn default() -> Self {
        Self {
            tick: 0,
            kind: Kind::Counter,
            what: 0,
            who: 0,
            a: 0,
            b: 0,
            c: 0,
            note: [0; NOTE_MAX],
            note_len: 0,
        }
    }
}

impl Record {
    /// A record with no note — the common case.
    pub fn new(tick: u64, kind: Kind, what: u16, who: u32) -> Self {
        Self {
            tick,
            kind,
            what,
            who,
            ..Self::default()
        }
    }

    pub fn with(mut self, a: i64, b: i64, c: i64) -> Self {
        self.a = a;
        self.b = b;
        self.c = c;
        self
    }

    /// Attach a note, truncated to [`NOTE_MAX`]. Truncation rather than
    /// refusal: a long note is still evidence, and the source is already
    /// capped at the same width.
    pub fn note(mut self, bytes: &[u8]) -> Self {
        let n = bytes.len().min(NOTE_MAX);
        self.note[..n].copy_from_slice(&bytes[..n]);
        self.note_len = n as u8;
        self
    }

    fn note_str(&self) -> &str {
        // Valid UTF-8 whenever it came from a `ChatText`; a truncation
        // could split a multi-byte char, so this is lossy-by-refusal
        // rather than a panic.
        core::str::from_utf8(&self.note[..self.note_len as usize]).unwrap_or("")
    }
}

/// The counters the sweep watches, and the name each gets in the log.
///
/// **Not every counter — every counter that means something went wrong.**
/// `ticks`, `snap_sent` and the gauges are load, not anomalies, and a log
/// that recorded them would bury the eleven forged datagrams under a
/// million lines of "the server ran".
///
/// Adding a counter to `stats.rs` does not add it here, deliberately: the
/// question "is this a failure?" has an answer only a person has. What the
/// gate holds is that every name below resolves
/// (`the_watch_list_names_real_counters`).
pub const WATCHED: &[&str] = &[
    "ticks_dropped",
    "refused_version",
    "refused_build",
    "refused_auth",
    "refused_ticket",
    "refused_full",
    "entitle_unknown",
    "entitle_kicked",
    "handshake_errors",
    "input_dg_bad",
    "input_dg_forged",
    "input_ring_drops",
    "snap_ring_skips",
    "snap_send_errors",
    "encode_range_errors",
    "snap_entities_shed",
    "forced_resyncs",
    "ev_resyncs",
    // The sim outran `MAX_EVENTS_PER_TICK`, which resyncs every connected
    // client at once. Watched where `ev_interest_skipped` is not: a filter
    // firing is the normal case and this is never one.
    "ev_sim_dropped",
    "ev_send_errors",
    "actions_bad",
    "chat_bad",
    "chat_rate_limited",
    "chat_ring_drops",
    "chat_undelivered",
    "piece_walk_restarts",
    "save_ring_drops",
    "saves_evicted",
    "save_write_errors",
    "world_save_errors",
    "world_load_errors",
    "sleepers_evicted",
    "admin_kicked",
    "admin_refused",
    // A client naming a snapshot older than the sent ring can corroborate
    // (`stats.rs`, `AIM_STALE_CEILING_TICKS`). Watched because it is the
    // only counter in the aim-staleness set that accuses somebody: the
    // other five are load, and this one is a forged or wildly-wrong ack.
    "aim_stale_refused",
    // A client whose claimed view sits further behind the server's own ack
    // history than reordering explains (`stats.rs`,
    // `FAVOUR_DISAGREE_BAND_TICKS`). Watched for `aim_stale_refused`'s
    // reason and one sharper: since lagcomp slice 5 the staleness a client
    // claims BUYS something — up to `REWIND_MAX_TICKS` of rewind — so this
    // is the counter that says somebody is reaching for it. `DESIGN.md`
    // §5.8's "outliers surface in the anomaly log", built.
    "favour_disagree",
];

/// The previous tick's reading of every watched counter, so the sweep can
/// emit deltas. Lives on the sim thread beside the log's producer.
pub struct Watch {
    last: Vec<u64>,
}

impl Default for Watch {
    fn default() -> Self {
        Self::new()
    }
}

impl Watch {
    pub fn new() -> Self {
        Self {
            last: vec![0; WATCHED.len()],
        }
    }

    /// Read every watched counter and emit one record per counter that
    /// moved. Allocation-free per call (the `Vec` is built once), and the
    /// read is one relaxed atomic load per counter.
    ///
    /// Called on a cadence rather than every tick — the sweep is cheap but
    /// not free, and a counter that moved is interesting to the second,
    /// not to the 33 ms.
    pub fn sweep(&mut self, tick: u64, stats: &ShardStats, log: &mut Sink) {
        for (i, name) in WATCHED.iter().enumerate() {
            let now = stats.by_name(name).map(ShardStats::get).unwrap_or(0);
            let was = self.last[i];
            if now > was {
                self.last[i] = now;
                log.push(Record::new(tick, Kind::Counter, i as u16, 0).with(
                    (now - was) as i64,
                    0,
                    0,
                ));
            }
        }
    }
}

/// The sim thread's end of the log. Absent when no `anomaly_file` is
/// configured, and then every push is a no-op — a shard with no log
/// configured must cost nothing, which is what makes the default safe.
pub struct Sink {
    tx: Option<rtrb::Producer<Record>>,
    /// Only a live sink can drop a record, so only a live sink needs the
    /// counter — which is what makes [`Sink::off`] free to construct
    /// anywhere, tests included.
    stats: Option<Arc<ShardStats>>,
}

impl Sink {
    /// No log: every push is a no-op. The default a shard with no
    /// `anomaly_file` runs, and what every test drives.
    pub fn off() -> Self {
        Self {
            tx: None,
            stats: None,
        }
    }

    pub fn new(tx: rtrb::Producer<Record>, stats: Arc<ShardStats>) -> Self {
        Self {
            tx: Some(tx),
            stats: Some(stats),
        }
    }

    /// Ring one record, or count the drop. Never blocks, never allocates,
    /// never formats — wall 3 holds on this path because it is called from
    /// the sim thread.
    pub fn push(&mut self, rec: Record) {
        let Some(tx) = self.tx.as_mut() else {
            return;
        };
        if tx.push(rec).is_err() {
            if let Some(stats) = self.stats.as_ref() {
                ShardStats::bump(&stats.anomaly_dropped);
            }
        }
    }
}

/// Open (or create) the log file and hand back the two halves: the sim
/// thread's sink and the thread that owns the file.
///
/// Append mode, so a restart adds to the record rather than replacing it —
/// the interesting hour is usually the one before a crash.
pub fn spawn(
    path: &Path,
    stats: Arc<ShardStats>,
) -> std::io::Result<(Sink, std::thread::JoinHandle<()>)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let (tx, rx) = rtrb::RingBuffer::<Record>::new(ANOMALY_RING_CAP);
    let handle = std::thread::Builder::new()
        .name("anomaly".into())
        .spawn(move || log_thread(file, rx))?;
    Ok((Sink::new(tx, stats), handle))
}

/// The log thread: drain, format, append, flush. Exits when the ring is
/// abandoned, which happens when the sim thread's `Sink` drops at
/// shutdown — so the last records of a run are written before the process
/// ends rather than lost with it.
fn log_thread(mut file: File, mut rx: rtrb::Consumer<Record>) {
    let mut line = String::with_capacity(512);
    loop {
        let mut wrote = false;
        while let Ok(rec) = rx.pop() {
            line.clear();
            format_line(&rec, &mut line);
            // A write that fails is not worth a second failure: the file
            // is diagnostics and the console already has the counters.
            let _ = file.write_all(line.as_bytes());
            wrote = true;
        }
        if wrote {
            let _ = file.flush();
        }
        if rx.is_abandoned() {
            // One last drain: records pushed between the check above and
            // the producer dropping are still owed a line.
            while let Ok(rec) = rx.pop() {
                line.clear();
                format_line(&rec, &mut line);
                let _ = file.write_all(line.as_bytes());
            }
            let _ = file.flush();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// One JSONL line, appended to `out` (newline included).
///
/// Hand-rolled rather than serde: the shapes are four, the fields are
/// integers and one already-sanitized string, and `ChatText::sanitize` has
/// already refused every control character — so the only escape a note can
/// need is a quote or a backslash, and both are handled below.
pub fn format_line(rec: &Record, out: &mut String) {
    use std::fmt::Write as _;
    let kind = match rec.kind {
        Kind::Counter => "counter",
        Kind::Bug => "bug",
        Kind::AdminAct => "admin",
        Kind::AdminRefused => "admin_refused",
    };
    let _ = write!(out, "{{\"tick\":{},\"kind\":\"{}\"", rec.tick, kind);
    if rec.kind == Kind::Counter {
        let name = WATCHED.get(rec.what as usize).copied().unwrap_or("unknown");
        let _ = write!(out, ",\"counter\":\"{}\",\"delta\":{}", name, rec.a);
    } else {
        let _ = write!(out, ",\"what\":{}", rec.what);
        if rec.who != 0 {
            let _ = write!(out, ",\"who\":{}", rec.who);
        }
        let _ = write!(out, ",\"a\":{},\"b\":{},\"c\":{}", rec.a, rec.b, rec.c);
    }
    if rec.note_len > 0 {
        out.push_str(",\"note\":\"");
        for ch in rec.note_str().chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push_str("}\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name in the watch list must resolve against `ShardStats`, or
    /// the sweep would silently watch nothing — a log that reports no
    /// failures because it is looking at a typo is the worst outcome this
    /// file can produce.
    #[test]
    fn the_watch_list_names_real_counters() {
        let stats = ShardStats::default();
        for name in WATCHED {
            assert!(
                stats.by_name(name).is_some(),
                "the watch list names `{name}`, which ShardStats does not have"
            );
        }
    }

    /// A counter that moves produces exactly one line naming it and the
    /// delta; a counter that does not move produces nothing. Both halves
    /// matter: the second is what keeps a quiet shard's log quiet.
    #[test]
    fn the_sweep_reports_deltas_and_only_deltas() {
        let stats = Arc::new(ShardStats::default());
        let mut sink = Sink::off();
        let mut watch = Watch::new();
        // Nothing has moved yet.
        watch.sweep(1, &stats, &mut sink);
        ShardStats::bump(&stats.chat_bad);
        ShardStats::bump(&stats.chat_bad);
        // Two bumps, one record, delta 2 — checked through the formatter
        // because that is the only observable output of the pair.
        let idx = WATCHED.iter().position(|n| *n == "chat_bad").unwrap();
        let mut line = String::new();
        format_line(
            &Record::new(7, Kind::Counter, idx as u16, 0).with(2, 0, 0),
            &mut line,
        );
        assert_eq!(
            line,
            "{\"tick\":7,\"kind\":\"counter\",\"counter\":\"chat_bad\",\"delta\":2}\n"
        );
        // And the sweep's own bookkeeping: after it sees the two bumps, a
        // second sweep with nothing new emits nothing.
        watch.sweep(2, &stats, &mut sink);
        let before = watch.last[idx];
        watch.sweep(3, &stats, &mut sink);
        assert_eq!(watch.last[idx], before, "a quiet counter must stay quiet");
        assert_eq!(before, 2, "the sweep read both bumps");
    }

    /// A note is quoted, and the two characters JSON cannot take raw are
    /// escaped. Control characters cannot arrive (`ChatText` refuses them),
    /// which is why this list is two long and not thirty.
    #[test]
    fn a_note_survives_json() {
        let mut line = String::new();
        format_line(
            &Record::new(9, Kind::Bug, 0, 259)
                .with(1, 2, 3)
                .note(br#"fell "through" the \floor"#),
            &mut line,
        );
        assert_eq!(
            line,
            "{\"tick\":9,\"kind\":\"bug\",\"what\":0,\"who\":259,\"a\":1,\"b\":2,\"c\":3,\
             \"note\":\"fell \\\"through\\\" the \\\\floor\"}\n"
        );
    }

    /// An over-long note is truncated, not refused — and the truncation
    /// cannot produce invalid UTF-8 in the output.
    #[test]
    fn an_overlong_note_truncates_cleanly() {
        let long = "x".repeat(NOTE_MAX * 2);
        let rec = Record::new(1, Kind::Bug, 0, 1).note(long.as_bytes());
        assert_eq!(rec.note_len as usize, NOTE_MAX);
        let mut line = String::new();
        format_line(&rec, &mut line);
        assert!(line.ends_with("\"}\n"));
        assert!(std::str::from_utf8(line.as_bytes()).is_ok());
    }

    /// A disabled sink is a no-op that costs nothing and drops nothing —
    /// the default every test shard and every community shard runs.
    #[test]
    fn a_disabled_sink_swallows_without_counting_a_drop() {
        let stats = Arc::new(ShardStats::default());
        let mut sink = Sink::off();
        for i in 0..1000 {
            sink.push(Record::new(i, Kind::Bug, 0, 1));
        }
        assert_eq!(ShardStats::get(&stats.anomaly_dropped), 0);
    }
}
