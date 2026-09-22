//! The trust ledger's sink: every trust row the sim mints, appended to a
//! durable log off the sim thread (`PLAYERS.md` wall 3, `NOW.md` §5d).
//!
//! ## The path, one hop per owner
//!
//! `sim_core::trust` keeps a tick's rows in a ring no tick can overflow.
//! After every `World::tick`, `ShardCore::tick` calls [`Tap::drain`] on the
//! sim thread. It resolves each party's wallet, then pushes the rows onto an
//! `rtrb` ring. The writer thread spawned by [`spawn`] owns the files: it
//! drains that ring, formats each row, and appends it to the log. The shape
//! is `store.rs`'s and `anomaly.rs`'s: SPSC rings and atomics, and the file
//! on a plain `std::thread`. The sim thread never formats, locks, allocates
//! or touches a file on this path (walls 2 and 3; `tests/trust_log.rs`
//! counts its allocations).
//!
//! ## Nothing is dropped silently
//!
//! The ring to the writer is [`TRUST_TAP_RING_CAP`] deep. When the disk
//! stalls long enough to fill it, the newest rows are dropped and
//! **counted** (`trust_ring_drops`), and the count is also written into the
//! log as a `gap` line at the place the rows went missing. Rows then wait
//! behind that gap line, so the log's order stays true. A sim-side overflow
//! (unreachable by `sim_core::trust`'s derivation) takes the same path as
//! `trust_sim_overflow`. A shard with no sink counts its rows as
//! `trust_unlogged`, so a ledger that is not being kept shows up as a
//! climbing number.
//!
//! ## The format: JSON lines, versioned in every segment's header
//!
//! The directory holds `trust-<seq>.jsonl` segments, and every boot starts a
//! new one, so a restart appends and never rewrites. Every segment opens
//! with a `header` line that names the format and version
//! ([`TRUST_LOG_FORMAT`]), the shard (domain, seed, content hash, world
//! digest, build, protocol), this boot's random id, the durability
//! contract, and the verb and presence vocabulary. So a segment read with no
//! other file still describes itself. After the header come `trust` rows,
//! `gap` lines, `pruned` lines (a segment deleted by rotation, named in the
//! log that outlived it), and on a clean shutdown a `close` line. A segment
//! whose last line has no newline was cut by a crash. The reader counts that
//! line as torn and keeps everything before it.
//!
//! ## Bounded, and durable on a stated cadence
//!
//! A segment is closed at [`TRUST_SEGMENT_BYTES`], and the oldest are
//! deleted until closed segments plus one open segment fit in
//! [`TRUST_LOG_MAX_BYTES`]. Lines reach the OS within [`TRUST_POLL_MS`] and
//! the disk within [`TRUST_SYNC_MS`] (`sync_data`). They are also synced at
//! every rotation and at shutdown. So a killed process loses nothing it
//! handed the kernel, and a power loss loses at most one sync interval.
//!
//! ## Wallets
//!
//! A party is a player id plus, when this shard knows it, the wallet the
//! handshake proved (`auth.rs`). It is resolved at drain time through
//! `ShardCore::identities` (connected players and filed sleepers). An
//! evicted body's wallet is also kept in a bounded memory, because an
//! offline raid on an evicted player's base is exactly the row the ledger
//! exists for. A body with no key is written `guest`. An id that names no
//! body and nothing remembered is written `unresolved`, never guessed.

use crate::core::ShardCore;
use crate::stats::ShardStats;
use crate::store::PlayerKey;
use sim_core::limits::{MAX_PLAYERS, MAX_TRUST_ROWS_PER_TICK, TICK_HZ};
use sim_core::world::{
    PRESENCE_ASLEEP, PRESENCE_AWAKE, PRESENCE_GONE, PRESENCE_MAX, TRUST_AUTH, TRUST_CONT,
    TRUST_DOOR, TRUST_VERB_MAX,
};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Messages in flight between the sim thread's [`Tap`] and the writer.
///
/// Eight ticks of the most trust rows a tick can make
/// (`MAX_TRUST_ROWS_PER_TICK`). The writer drains every [`TRUST_POLL_MS`],
/// about a tick and a half, so this ring only fills if the disk stalls for
/// a quarter second at the worst possible rate. Real rates are orders of
/// magnitude lower. Overflow policy: **drop newest, counted, and written
/// into the log as a gap line**. Proposed default, DECISIONS.md §open
/// ("trust ledger v1").
pub const TRUST_TAP_RING_CAP: usize = 2048;
const _: () = assert!(
    TRUST_TAP_RING_CAP >= 2 * MAX_TRUST_ROWS_PER_TICK,
    "the sink ring must hold two worst-case ticks, or a writer one poll late \
     drops rows at a rate the sim can actually produce"
);

/// A segment is closed and the next one opened once it reaches this size.
/// 16 MiB is about 80 000 rows. Proposed default, DECISIONS.md §open.
pub const TRUST_SEGMENT_BYTES: u64 = 16_777_216;

/// The most the log directory keeps: closed segments plus the open one.
/// Past it, the oldest segments are deleted, and each deletion is written
/// into the current segment as a `pruned` line. 256 MiB is about 1.3 million
/// rows. Proposed default, DECISIONS.md §open.
pub const TRUST_LOG_MAX_BYTES: u64 = 268_435_456;
const _: () = assert!(
    TRUST_LOG_MAX_BYTES >= 2 * TRUST_SEGMENT_BYTES,
    "the log must keep at least one closed segment beside the open one"
);

/// How often the writer calls `sync_data` while it has unsynced lines. Lines
/// are written to the OS every [`TRUST_POLL_MS`], so a process crash loses
/// nothing written. A power loss loses at most this much. Proposed default,
/// DECISIONS.md §open.
pub const TRUST_SYNC_MS: u64 = 1_000;

/// How long the writer sleeps when its ring is empty. Proposed default,
/// DECISIONS.md §open.
pub const TRUST_POLL_MS: u64 = 50;

/// Ticks between identity refreshes when nothing forces one. A tick with
/// rows or an eviction always refreshes. The cadence covers a body that
/// connects, sleeps and is evicted without appearing in any row. One
/// second: `ShardCore::identities` is ~10k compares at a full shard.
/// Proposed default, DECISIONS.md §open.
pub const TRUST_IDENT_REFRESH_TICKS: u64 = 30;
const _: () = assert!(TRUST_IDENT_REFRESH_TICKS == TICK_HZ as u64);

/// Evicted bodies whose wallet the tap remembers, oldest overwritten. An
/// eviction frees a slot for a join, so this is roughly the last thousand
/// joins that met a full shard. Proposed default, DECISIONS.md §open.
pub const TRUST_GONE_MEMORY: usize = 1024;

/// The on-disk format's version, in every header. Bump it with any change to
/// a line's shape, the wall-6 discipline applied to a file.
pub const TRUST_LOG_FORMAT: u32 = 1;

/// The `format` field every header carries.
pub const TRUST_LOG_NAME: &str = "gates-trust-log";

/// Formatted lines the writer batches before one `write_all`, in bytes.
const BATCH_BYTES: usize = 65_536;

// ---------------------------------------------------------------------------
// What crosses the ring
// ---------------------------------------------------------------------------

/// Who a party was, as far as this shard could tell at drain time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// The wallet the handshake proved.
    Wallet(PlayerKey),
    /// A body with no key: admitted, and remembered by nobody.
    Guest,
    /// No body and nothing remembered. Written as such, never guessed.
    Unresolved,
}

/// One side of a trust row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Party {
    pub id: u32,
    pub who: Who,
}

/// One trust row, resolved. `tick` is the tick the verb was applied in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub tick: u64,
    pub verb: u8,
    pub presence: u8,
    pub actor: Party,
    pub counterparty: Party,
}

/// Rows the log does not have, and why, over a tick range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gap {
    pub from_tick: u64,
    pub to_tick: u64,
    /// Dropped because the ring to the writer was full.
    pub ring_full: u64,
    /// Refused by the sim's own ring (`TrustLedger::overflow`).
    pub sim_overflow: u64,
}

/// What the sim thread hands the writer. `Copy` and integer-only, so it
/// crosses the ring under wall 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    Row(Row),
    Gap(Gap),
}

// ---------------------------------------------------------------------------
// The sim thread's end
// ---------------------------------------------------------------------------

/// The sim thread's end of the trust log. [`Tap::off`] when no log is
/// configured: rows are then counted as `trust_unlogged` and go nowhere.
pub struct Tap {
    live: Option<Box<Live>>,
}

struct Live {
    tx: rtrb::Producer<Msg>,
    /// `ShardCore::identities` at the last refresh, and at the one before.
    /// Twice `MAX_PLAYERS` wide: bodies are capped at `MAX_PLAYERS`, but a
    /// keyed join still in the command queue has a key and no body yet, and
    /// a table that truncated would write a known wallet as a guest.
    now: Box<[(PlayerKey, u32)]>,
    now_n: usize,
    prev: Box<[(PlayerKey, u32)]>,
    prev_n: usize,
    /// Wallets of evicted bodies, a ring overwritten oldest first.
    gone: Box<[(u32, PlayerKey)]>,
    gone_n: usize,
    gone_at: usize,
    /// `World::evictions` at the last refresh.
    evictions: u64,
    /// Loss not yet written. While it is pending, rows wait behind it.
    gap: Option<Gap>,
}

impl Default for Tap {
    fn default() -> Self {
        Self::off()
    }
}

impl Tap {
    /// No log. Allocates nothing, so `ShardCore::new` and every test can
    /// hold one for free.
    pub const fn off() -> Self {
        Self { live: None }
    }

    pub fn is_on(&self) -> bool {
        self.live.is_some()
    }

    /// A live tap and the consumer end of its ring, with no writer attached.
    /// [`spawn`] hands the consumer to the writer thread. The back-pressure
    /// tests hold it themselves, so they can stall the writer on purpose.
    pub fn channel(capacity: usize) -> (Tap, rtrb::Consumer<Msg>) {
        let (tx, rx) = rtrb::RingBuffer::<Msg>::new(capacity);
        let blank = (PlayerKey::PLACEHOLDER, 0u32);
        let live = Live {
            tx,
            now: vec![blank; 2 * MAX_PLAYERS].into_boxed_slice(),
            now_n: 0,
            prev: vec![blank; 2 * MAX_PLAYERS].into_boxed_slice(),
            prev_n: 0,
            gone: vec![(0u32, PlayerKey::PLACEHOLDER); TRUST_GONE_MEMORY].into_boxed_slice(),
            gone_n: 0,
            gone_at: 0,
            evictions: 0,
            gap: None,
        };
        (
            Tap {
                live: Some(Box::new(live)),
            },
            rx,
        )
    }

    /// **The hook**: hand this tick's trust rows to the log. Called by
    /// `ShardCore::tick` after `World::tick`, before the next tick clears the
    /// ring.
    ///
    /// The tap is lifted out of the core for the call, because resolving a
    /// wallet reads the rest of the core. The `Default` left behind is
    /// [`Tap::off`], a `None`, so the swap allocates nothing.
    pub fn drain(core: &mut ShardCore, stats: &ShardStats) {
        let mut tap = std::mem::take(&mut core.trust);
        tap.drain_from(core, stats);
        core.trust = tap;
    }

    fn drain_from(&mut self, core: &ShardCore, stats: &ShardStats) {
        let ledger = &core.world.trust;
        let rows = ledger.rows();
        let tick = ledger.tick();
        let overflow = ledger.overflow() as u64;
        ShardStats::add(&stats.trust_rows, rows.len() as u64);
        if overflow > 0 {
            ShardStats::add(&stats.trust_sim_overflow, overflow);
        }
        let Some(live) = self.live.as_mut() else {
            ShardStats::add(&stats.trust_unlogged, rows.len() as u64);
            return;
        };
        let evicted = core.world.evictions != live.evictions;
        if !rows.is_empty() || evicted || tick.is_multiple_of(TRUST_IDENT_REFRESH_TICKS) {
            live.refresh(core, evicted);
        }
        for r in rows {
            let row = Row {
                tick,
                verb: r.verb,
                presence: r.presence,
                actor: live.party(core, r.actor),
                counterparty: live.party(core, r.counterparty),
            };
            live.offer(Msg::Row(row), stats);
        }
        // Rows the sim itself refused were the newest of this tick, so their
        // gap goes after the rows it kept.
        if overflow > 0 {
            live.lose(tick, 0, overflow);
        }
        // A gap that missed its ring slot retries every tick, rows or not.
        live.flush_gap();
    }

    /// Whether a gap line is waiting for room on the ring.
    pub fn gap_pending(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.gap.is_some())
    }

    /// Retry a pending gap line. The drain calls this every tick, rows or
    /// not, so a gap waits at most as long as the ring stays full.
    pub fn flush_gap(&mut self) {
        if let Some(live) = self.live.as_mut() {
            live.flush_gap();
        }
    }

    /// Put one message on the ring, or count it lost into the pending gap.
    /// [`Tap::drain`] is the production caller; tests call this directly.
    pub fn offer(&mut self, msg: Msg, stats: &ShardStats) {
        match self.live.as_mut() {
            Some(live) => live.offer(msg, stats),
            None => {
                if matches!(msg, Msg::Row(_)) {
                    ShardStats::bump(&stats.trust_unlogged);
                }
            }
        }
    }
}

impl Live {
    fn offer(&mut self, msg: Msg, stats: &ShardStats) {
        self.flush_gap();
        if self.gap.is_none() && self.tx.push(msg).is_ok() {
            return;
        }
        if let Msg::Row(r) = msg {
            ShardStats::bump(&stats.trust_ring_drops);
            self.lose(r.tick, 1, 0);
        }
    }

    fn lose(&mut self, tick: u64, ring_full: u64, sim_overflow: u64) {
        let g = self.gap.get_or_insert(Gap {
            from_tick: tick,
            to_tick: tick,
            ..Gap::default()
        });
        g.to_tick = tick;
        g.ring_full += ring_full;
        g.sim_overflow += sim_overflow;
    }

    fn flush_gap(&mut self) {
        if let Some(g) = self.gap {
            if self.tx.push(Msg::Gap(g)).is_ok() {
                self.gap = None;
            }
        }
    }

    /// Re-read who everybody is. On a tick that evicted a body, remember the
    /// wallet of every body that vanished from both the identity table and
    /// the world, before the table forgets it for good.
    fn refresh(&mut self, core: &ShardCore, evicted: bool) {
        std::mem::swap(&mut self.now, &mut self.prev);
        self.prev_n = self.now_n;
        self.now_n = core.identities(&mut self.now);
        if !evicted {
            return;
        }
        self.evictions = core.world.evictions;
        for i in 0..self.prev_n {
            let (key, id) = self.prev[i];
            let still = self.now[..self.now_n].iter().any(|&(_, n)| n == id);
            if !still && core.world.presence_of(id) == PRESENCE_GONE {
                self.gone[self.gone_at] = (id, key);
                self.gone_at = (self.gone_at + 1) % TRUST_GONE_MEMORY;
                self.gone_n = (self.gone_n + 1).min(TRUST_GONE_MEMORY);
            }
        }
    }

    fn party(&self, core: &ShardCore, id: u32) -> Party {
        let find = |t: &[(PlayerKey, u32)]| t.iter().find(|&&(_, n)| n == id).map(|&(k, _)| k);
        let who = if let Some(k) = find(&self.now[..self.now_n]) {
            Who::Wallet(k)
        } else if let Some(k) = find(&self.prev[..self.prev_n]) {
            Who::Wallet(k)
        } else if let Some(&(_, k)) = self.gone[..self.gone_n].iter().find(|&&(n, _)| n == id) {
            Who::Wallet(k)
        } else if core.world.presence_of(id) != PRESENCE_GONE {
            Who::Guest
        } else {
            Who::Unresolved
        };
        Party { id, who }
    }
}

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// Who this shard is, written into every segment's header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardIdent {
    /// What the shard calls itself in the message players sign.
    pub domain: String,
    pub seed: u64,
    pub content: u64,
    /// `probe_terrain(seed)`: the island's own digest.
    pub world: u64,
    pub build: String,
    pub proto: u16,
}

/// The writer's sizes and cadences. `Default` is the knobs above; tests
/// shrink them to reach rotation in kilobytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub segment_bytes: u64,
    pub max_bytes: u64,
    pub sync_ms: u64,
    pub poll_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            segment_bytes: TRUST_SEGMENT_BYTES,
            max_bytes: TRUST_LOG_MAX_BYTES,
            sync_ms: TRUST_SYNC_MS,
            poll_ms: TRUST_POLL_MS,
        }
    }
}

/// The writer's counters. Its own `Arc` rather than `ShardStats`, because
/// the log is opened before the shard exists (`bin/shard.rs` refuses a
/// boot whose log cannot be opened, before a port is bound).
#[derive(Debug, Default)]
pub struct WriterStats {
    pub rows_written: AtomicU64,
    pub gaps_written: AtomicU64,
    /// Rows in a batch the disk refused. Each failed batch is also a
    /// `write_errors` bump.
    pub rows_unwritten: AtomicU64,
    pub write_errors: AtomicU64,
    pub segments_opened: AtomicU64,
    pub segments_pruned: AtomicU64,
    pub bytes_pruned: AtomicU64,
    /// Raised once the writer has written its `close` line and synced.
    pub stopped: AtomicBool,
}

impl WriterStats {
    pub fn get(v: &AtomicU64) -> u64 {
        v.load(Ordering::Relaxed)
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

/// The handle `spawn` returns: where the log is, and how it is doing.
pub struct TrustLog {
    pub dir: PathBuf,
    /// The segment this boot opened first.
    pub segment: u64,
    /// This boot's random id, as written in its headers.
    pub boot: String,
    pub stats: Arc<WriterStats>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl TrustLog {
    /// Wait for the writer to finish, up to `deadline`. It finishes once its
    /// [`Tap`] is dropped. True if it did.
    pub fn join_within(&mut self, deadline: Duration) -> bool {
        let end = Instant::now() + deadline;
        while !self.stats.stopped() {
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        true
    }
}

/// Open (or create) the log directory, start this boot's segment, and start
/// the writer. Everything that can fail on a disk fails here, before the
/// shard binds a port.
pub fn spawn(dir: &Path, ident: ShardIdent, limits: Limits) -> std::io::Result<(Tap, TrustLog)> {
    if limits.max_bytes < 2 * limits.segment_bytes || limits.segment_bytes == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "trust log limits must keep one closed segment beside the open one",
        ));
    }
    std::fs::create_dir_all(dir)?;
    let seq = segments(dir)?.last().map(|s| s.0 + 1).unwrap_or(1);
    let boot = boot_id();
    let stats = Arc::new(WriterStats::default());
    let mut w = Writer {
        dir: dir.to_path_buf(),
        ident,
        boot: boot.clone(),
        limits,
        file: open_segment(dir, seq)?,
        seq,
        seg_bytes: 0,
        rows_in_buf: 0,
        stats: stats.clone(),
        line: String::with_capacity(512),
        buf: Vec::with_capacity(BATCH_BYTES + 1024),
        dirty: false,
        last_sync: Instant::now(),
    };
    w.start_segment()?;
    let (tap, rx) = Tap::channel(TRUST_TAP_RING_CAP);
    let handle = std::thread::Builder::new()
        .name("trustlog".into())
        .spawn(move || w.run(rx))?;
    Ok((
        tap,
        TrustLog {
            dir: dir.to_path_buf(),
            segment: seq,
            boot,
            stats,
            handle: Some(handle),
        },
    ))
}

/// A segment's file name. Ten digits, so name order is sequence order.
pub fn segment_name(seq: u64) -> String {
    format!("trust-{seq:010}.jsonl")
}

fn segment_seq(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("trust-")?.strip_suffix(".jsonl")?;
    if digits.len() != 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Every segment in `dir` as `(seq, bytes)`, oldest first.
fn segments(dir: &Path) -> std::io::Result<Vec<(u64, u64)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name();
        if let Some(seq) = name.to_str().and_then(segment_seq) {
            out.push((seq, e.metadata()?.len()));
        }
    }
    out.sort_unstable();
    Ok(out)
}

fn open_segment(dir: &Path, seq: u64) -> std::io::Result<File> {
    OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(dir.join(segment_name(seq)))
}

/// Sixteen hex digits from the OS, or from the clock and pid if the OS
/// cannot say. It tells two boots apart; nothing trusts it for more.
fn boot_id() -> String {
    let mut b = [0u8; 8];
    if getrandom::getrandom(&mut b).is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        b = (t ^ ((std::process::id() as u64) << 32)).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Writer {
    dir: PathBuf,
    ident: ShardIdent,
    boot: String,
    limits: Limits,
    file: File,
    seq: u64,
    seg_bytes: u64,
    rows_in_buf: u64,
    stats: Arc<WriterStats>,
    line: String,
    buf: Vec<u8>,
    dirty: bool,
    last_sync: Instant,
}

impl Writer {
    fn run(mut self, mut rx: rtrb::Consumer<Msg>) {
        let poll = Duration::from_millis(self.limits.poll_ms);
        let sync = Duration::from_millis(self.limits.sync_ms);
        loop {
            let mut got = false;
            while let Ok(m) = rx.pop() {
                self.take(&m);
                got = true;
            }
            self.write_out();
            if self.dirty && self.last_sync.elapsed() >= sync {
                self.sync();
            }
            if rx.is_abandoned() {
                // Messages pushed between the drain above and the tap's drop
                // are still owed a line.
                while let Ok(m) = rx.pop() {
                    self.take(&m);
                }
                // The close line counts rows that reached the OS, so it is
                // written after them.
                self.write_out();
                self.line.clear();
                self.line.push_str("{\"kind\":\"close\",\"rows\":");
                push_u64(&mut self.line, WriterStats::get(&self.stats.rows_written));
                self.line.push_str("}\n");
                self.buf.extend_from_slice(self.line.as_bytes());
                self.write_out();
                self.sync();
                self.stats.stopped.store(true, Ordering::Release);
                return;
            }
            if !got {
                std::thread::sleep(poll);
            }
        }
    }

    fn take(&mut self, m: &Msg) {
        self.line.clear();
        format_msg(m, &mut self.line);
        self.buf.extend_from_slice(self.line.as_bytes());
        match m {
            Msg::Row(_) => self.rows_in_buf += 1,
            Msg::Gap(_) => {
                self.stats.gaps_written.fetch_add(1, Ordering::Relaxed);
            }
        }
        if self.buf.len() >= BATCH_BYTES {
            self.write_out();
        }
    }

    /// Hand the batch to the OS, then rotate if the segment is full.
    fn write_out(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        match self.file.write_all(&self.buf) {
            Ok(()) => {
                self.seg_bytes += self.buf.len() as u64;
                self.stats
                    .rows_written
                    .fetch_add(self.rows_in_buf, Ordering::Relaxed);
                self.dirty = true;
            }
            Err(_) => {
                self.stats.write_errors.fetch_add(1, Ordering::Relaxed);
                self.stats
                    .rows_unwritten
                    .fetch_add(self.rows_in_buf, Ordering::Relaxed);
            }
        }
        self.buf.clear();
        self.rows_in_buf = 0;
        if self.seg_bytes >= self.limits.segment_bytes {
            self.rotate();
        }
    }

    fn sync(&mut self) {
        if self.file.sync_data().is_err() {
            self.stats.write_errors.fetch_add(1, Ordering::Relaxed);
        }
        self.dirty = false;
        self.last_sync = Instant::now();
    }

    fn rotate(&mut self) {
        self.sync();
        match open_segment(&self.dir, self.seq + 1) {
            Ok(f) => {
                self.file = f;
                self.seq += 1;
                self.seg_bytes = 0;
                if self.start_segment().is_err() {
                    self.stats.write_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            // Keep appending to the full segment rather than lose rows: the
            // size bound bends, and the error is counted.
            Err(_) => {
                self.stats.write_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Write the header into the (new, empty) current segment, sync it, and
    /// prune: the oldest segments go until everything closed, plus one full
    /// open segment, fits in `max_bytes`. Each deletion is named in a
    /// `pruned` line here, in the segment that outlived it.
    fn start_segment(&mut self) -> std::io::Result<()> {
        let opened = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut head = String::with_capacity(1024);
        format_header(
            &self.ident,
            &self.boot,
            self.seq,
            opened,
            &self.limits,
            &mut head,
        );
        let segs = segments(&self.dir)?;
        let mut closed: u64 = segs.iter().filter(|s| s.0 != self.seq).map(|s| s.1).sum();
        for &(seq, bytes) in segs.iter().filter(|s| s.0 != self.seq) {
            if closed + self.limits.segment_bytes <= self.limits.max_bytes {
                break;
            }
            let name = segment_name(seq);
            if std::fs::remove_file(self.dir.join(&name)).is_ok() {
                closed -= bytes;
                self.stats.segments_pruned.fetch_add(1, Ordering::Relaxed);
                self.stats.bytes_pruned.fetch_add(bytes, Ordering::Relaxed);
                head.push_str("{\"kind\":\"pruned\",\"segment\":\"");
                head.push_str(&name);
                head.push_str("\",\"bytes\":");
                push_u64(&mut head, bytes);
                head.push_str("}\n");
            }
        }
        self.file.write_all(head.as_bytes())?;
        self.file.sync_data()?;
        self.seg_bytes += head.len() as u64;
        self.stats.segments_opened.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Formatting: the writer thread's, never the sim's
// ---------------------------------------------------------------------------

/// A verb's name in the log. `None` for a code this build has no name for,
/// which `every_code_has_a_log_name` refuses at test time.
pub fn verb_name(v: u8) -> Option<&'static str> {
    match v {
        TRUST_DOOR => Some("door"),
        TRUST_AUTH => Some("auth"),
        TRUST_CONT => Some("cont"),
        _ => None,
    }
}

/// A presence's name in the log.
pub fn presence_name(p: u8) -> Option<&'static str> {
    match p {
        PRESENCE_AWAKE => Some("awake"),
        PRESENCE_ASLEEP => Some("asleep"),
        PRESENCE_GONE => Some("gone"),
        _ => None,
    }
}

fn push_u64(out: &mut String, v: u64) {
    use std::fmt::Write as _;
    let _ = write!(out, "{v}");
}

/// A wallet as a JSON string. An address is printable ASCII and goes in as
/// it is; anything else is written as `hex:` plus its bytes, so an opaque
/// key can never break a line.
fn push_key(out: &mut String, k: &PlayerKey) {
    let b = k.as_bytes();
    out.push('"');
    if b.iter()
        .all(|&c| c.is_ascii_graphic() && c != b'"' && c != b'\\')
    {
        out.push_str(core::str::from_utf8(b).unwrap_or(""));
    } else {
        use std::fmt::Write as _;
        out.push_str("hex:");
        for c in b {
            let _ = write!(out, "{c:02x}");
        }
    }
    out.push('"');
}

fn push_party(out: &mut String, p: &Party) {
    out.push_str("{\"id\":");
    push_u64(out, p.id as u64);
    match p.who {
        Who::Wallet(k) => {
            out.push_str(",\"wallet\":");
            push_key(out, &k);
        }
        Who::Guest => out.push_str(",\"guest\":true"),
        Who::Unresolved => out.push_str(",\"unresolved\":true"),
    }
    out.push('}');
}

/// One line, newline included.
pub fn format_msg(m: &Msg, out: &mut String) {
    match m {
        Msg::Row(r) => {
            out.push_str("{\"kind\":\"trust\",\"tick\":");
            push_u64(out, r.tick);
            out.push_str(",\"verb\":\"");
            match verb_name(r.verb) {
                Some(n) => out.push_str(n),
                None => {
                    out.push_str("code-");
                    push_u64(out, r.verb as u64);
                }
            }
            out.push_str("\",\"presence\":\"");
            match presence_name(r.presence) {
                Some(n) => out.push_str(n),
                None => {
                    out.push_str("code-");
                    push_u64(out, r.presence as u64);
                }
            }
            out.push_str("\",\"actor\":");
            push_party(out, &r.actor);
            out.push_str(",\"counterparty\":");
            push_party(out, &r.counterparty);
            out.push_str("}\n");
        }
        Msg::Gap(g) => {
            out.push_str("{\"kind\":\"gap\",\"from_tick\":");
            push_u64(out, g.from_tick);
            out.push_str(",\"to_tick\":");
            push_u64(out, g.to_tick);
            out.push_str(",\"ring_full\":");
            push_u64(out, g.ring_full);
            out.push_str(",\"sim_overflow\":");
            push_u64(out, g.sim_overflow);
            out.push_str("}\n");
        }
    }
}

/// A segment's first line. Everything a reader needs to interpret the
/// segment without any other file: format and version, the shard, the
/// boot, the durability contract, and the vocabulary.
pub fn format_header(
    ident: &ShardIdent,
    boot: &str,
    seq: u64,
    opened_unix: u64,
    limits: &Limits,
    out: &mut String,
) {
    use std::fmt::Write as _;
    let _ = write!(
        out,
        "{{\"kind\":\"header\",\"format\":\"{TRUST_LOG_NAME}\",\"version\":{TRUST_LOG_FORMAT},\
         \"segment\":{seq},\"boot\":\"{boot}\",\"opened_unix\":{opened_unix},\"shard\":{{\"domain\":"
    );
    push_json_str(out, &ident.domain);
    let _ = write!(
        out,
        ",\"seed\":{},\"content\":\"{:016x}\",\"world\":\"{:016x}\",\"build\":",
        ident.seed, ident.content, ident.world
    );
    push_json_str(out, &ident.build);
    let _ = write!(
        out,
        ",\"proto\":{}}},\"segment_bytes\":{},\"max_bytes\":{},\"sync_ms\":{},\"verbs\":{{",
        ident.proto, limits.segment_bytes, limits.max_bytes, limits.sync_ms
    );
    for v in 1..=TRUST_VERB_MAX {
        if v > 1 {
            out.push(',');
        }
        let _ = write!(out, "\"{v}\":\"{}\"", verb_name(v).unwrap_or("unnamed"));
    }
    out.push_str("},\"presences\":{");
    for p in 1..=PRESENCE_MAX {
        if p > 1 {
            out.push(',');
        }
        let _ = write!(out, "\"{p}\":\"{}\"", presence_name(p).unwrap_or("unnamed"));
    }
    out.push_str("}}\n");
}

/// A config string as JSON: quotes, backslashes and control characters
/// escaped.
fn push_json_str(out: &mut String, s: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// The reader (`bin/trust-log.rs` and the tests)
// ---------------------------------------------------------------------------

/// One party, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogParty {
    pub id: u32,
    pub wallet: Option<String>,
    pub guest: bool,
}

/// One `trust` line, read back, with the segment and boot it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub segment: u64,
    pub boot: String,
    pub tick: u64,
    pub verb: String,
    pub presence: String,
    pub actor: LogParty,
    pub counterparty: LogParty,
}

/// One `gap` line, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogGap {
    pub segment: u64,
    pub from_tick: u64,
    pub to_tick: u64,
    pub ring_full: u64,
    pub sim_overflow: u64,
}

/// What one segment held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentInfo {
    pub seq: u64,
    pub boot: String,
    /// The header's `shard` object, verbatim.
    pub shard: String,
    pub rows: usize,
    /// It ended in a `close` line: a clean shutdown.
    pub closed: bool,
    /// The `close` line's count: rows this boot handed the OS, across every
    /// segment the boot wrote (so it equals `rows` only when the boot never
    /// rotated).
    pub close_rows: Option<u64>,
    /// Its last line had no newline: a crash cut it.
    pub torn: bool,
}

/// A whole log directory, read back in segment order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    pub segments: Vec<SegmentInfo>,
    pub rows: Vec<LogRow>,
    pub gaps: Vec<LogGap>,
    /// `(segment name, bytes)` for every `pruned` line.
    pub pruned: Vec<(String, u64)>,
}

/// Read every segment in `dir`, oldest first. Refuses a segment whose
/// header names another format or a version this reader does not know,
/// rather than misreading it.
pub fn read_dir(dir: &Path) -> Result<Log, String> {
    let segs = segments(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut log = Log::default();
    for (seq, _) in segs {
        let path = dir.join(segment_name(seq));
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        read_segment(seq, &bytes, &mut log).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(log)
}

/// Read one segment's bytes into `log`.
pub fn read_segment(seq: u64, bytes: &[u8], log: &mut Log) -> Result<(), String> {
    let torn = !bytes.is_empty() && !bytes.ends_with(b"\n");
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The piece after the last newline is empty on a clean segment and the
    // torn line on a cut one; either way it is not a line.
    lines.pop();
    let mut info = SegmentInfo {
        seq,
        boot: String::new(),
        shard: String::new(),
        rows: 0,
        closed: false,
        close_rows: None,
        torn,
    };
    for (i, line) in lines.iter().enumerate() {
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 1))?;
        let kind = v["kind"].as_str().unwrap_or("");
        if i == 0 {
            if kind != "header" || v["format"].as_str() != Some(TRUST_LOG_NAME) {
                return Err("the first line is not a trust log header".into());
            }
            let ver = v["version"].as_u64().unwrap_or(0);
            if ver != TRUST_LOG_FORMAT as u64 {
                return Err(format!(
                    "format version {ver}, and this reader knows version {TRUST_LOG_FORMAT}"
                ));
            }
            info.boot = v["boot"].as_str().unwrap_or("").to_string();
            info.shard = v["shard"].to_string();
            continue;
        }
        match kind {
            "trust" => {
                let party = |p: &serde_json::Value| LogParty {
                    id: p["id"].as_u64().unwrap_or(0) as u32,
                    wallet: p["wallet"].as_str().map(str::to_string),
                    guest: p["guest"].as_bool().unwrap_or(false),
                };
                log.rows.push(LogRow {
                    segment: seq,
                    boot: info.boot.clone(),
                    tick: v["tick"].as_u64().unwrap_or(0),
                    verb: v["verb"].as_str().unwrap_or("").to_string(),
                    presence: v["presence"].as_str().unwrap_or("").to_string(),
                    actor: party(&v["actor"]),
                    counterparty: party(&v["counterparty"]),
                });
                info.rows += 1;
            }
            "gap" => log.gaps.push(LogGap {
                segment: seq,
                from_tick: v["from_tick"].as_u64().unwrap_or(0),
                to_tick: v["to_tick"].as_u64().unwrap_or(0),
                ring_full: v["ring_full"].as_u64().unwrap_or(0),
                sim_overflow: v["sim_overflow"].as_u64().unwrap_or(0),
            }),
            "pruned" => log.pruned.push((
                v["segment"].as_str().unwrap_or("").to_string(),
                v["bytes"].as_u64().unwrap_or(0),
            )),
            "close" => {
                info.closed = true;
                info.close_rows = v["rows"].as_u64();
            }
            other => return Err(format!("line {}: unknown kind `{other}`", i + 1)),
        }
    }
    if !lines.is_empty() || torn {
        log.segments.push(info);
    }
    Ok(())
}

/// Which side of a row a wallet or player filter must match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    Either,
    Actor,
    Counterparty,
}

/// What `trust-log` selects. Every field is optional, and an empty filter
/// matches every row.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Case-insensitive, matched against the side `role` names.
    pub wallet: Option<String>,
    pub player: Option<u32>,
    pub verb: Option<String>,
    pub presence: Option<String>,
    pub from_tick: Option<u64>,
    pub to_tick: Option<u64>,
    /// A boot id, or a prefix of one.
    pub boot: Option<String>,
    pub role: Role,
}

impl Filter {
    pub fn matches(&self, r: &LogRow) -> bool {
        let sides: &[&LogParty] = match self.role {
            Role::Either => &[&r.actor, &r.counterparty],
            Role::Actor => &[&r.actor],
            Role::Counterparty => &[&r.counterparty],
        };
        if let Some(w) = &self.wallet {
            let hit = sides.iter().any(|p| {
                p.wallet
                    .as_deref()
                    .is_some_and(|x| x.eq_ignore_ascii_case(w))
            });
            if !hit {
                return false;
            }
        }
        if let Some(id) = self.player {
            if !sides.iter().any(|p| p.id == id) {
                return false;
            }
        }
        if self.verb.as_deref().is_some_and(|v| v != r.verb) {
            return false;
        }
        if self.presence.as_deref().is_some_and(|p| p != r.presence) {
            return false;
        }
        if self.from_tick.is_some_and(|t| r.tick < t) || self.to_tick.is_some_and(|t| r.tick > t) {
            return false;
        }
        if self.boot.as_deref().is_some_and(|b| !r.boot.starts_with(b)) {
            return false;
        }
        true
    }
}

/// Counts per verb and per (verb, presence) over a selection. Deliberately
/// nothing per player: `PLAYERS.md` wall 2 forbids a ranking, and a table of
/// players sorted by betrayals would be one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub rows: u64,
    pub by_verb: BTreeMap<String, u64>,
    pub by_verb_presence: BTreeMap<(String, String), u64>,
}

pub fn summarize<'a>(rows: impl IntoIterator<Item = &'a LogRow>) -> Summary {
    let mut s = Summary::default();
    for r in rows {
        s.rows += 1;
        *s.by_verb.entry(r.verb.clone()).or_default() += 1;
        *s.by_verb_presence
            .entry((r.verb.clone(), r.presence.clone()))
            .or_default() += 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> PlayerKey {
        PlayerKey::new(s.as_bytes()).expect("a legal key")
    }

    /// The format is a contract with every reader, so one line of each kind
    /// is pinned byte for byte. A change here is a `TRUST_LOG_FORMAT` bump.
    #[test]
    fn a_row_and_a_gap_are_pinned_byte_for_byte() {
        let mut out = String::new();
        format_msg(
            &Msg::Row(Row {
                tick: 123_456,
                verb: TRUST_DOOR,
                presence: PRESENCE_ASLEEP,
                actor: Party {
                    id: 259,
                    who: Who::Wallet(key("0x00112233445566778899aabbccddeeff00112233")),
                },
                counterparty: Party {
                    id: 514,
                    who: Who::Guest,
                },
            }),
            &mut out,
        );
        assert_eq!(
            out,
            "{\"kind\":\"trust\",\"tick\":123456,\"verb\":\"door\",\"presence\":\"asleep\",\
             \"actor\":{\"id\":259,\"wallet\":\"0x00112233445566778899aabbccddeeff00112233\"},\
             \"counterparty\":{\"id\":514,\"guest\":true}}\n"
        );
        out.clear();
        format_msg(
            &Msg::Gap(Gap {
                from_tick: 7,
                to_tick: 9,
                ring_full: 3,
                sim_overflow: 0,
            }),
            &mut out,
        );
        assert_eq!(
            out,
            "{\"kind\":\"gap\",\"from_tick\":7,\"to_tick\":9,\"ring_full\":3,\"sim_overflow\":0}\n"
        );
    }

    /// `event_roles.rs`'s closed-ledger discipline, on the writer's side:
    /// every verb and presence the sim can mint has a name in the log, so a
    /// new `TRUST_*` cannot reach disk as `code-4` without this going red.
    #[test]
    fn every_code_has_a_log_name() {
        for v in 1..=TRUST_VERB_MAX {
            assert!(verb_name(v).is_some(), "TRUST_* value {v} has no log name");
        }
        for p in 1..=PRESENCE_MAX {
            assert!(
                presence_name(p).is_some(),
                "PRESENCE_* value {p} has no log name"
            );
        }
        assert_eq!(verb_name(0), None, "zero is not a verb");
        assert_eq!(presence_name(0), None, "zero is not a presence");
    }

    /// An opaque key can never break a line: anything that is not printable
    /// ASCII goes in as hex.
    #[test]
    fn an_odd_key_is_written_as_hex() {
        let mut out = String::new();
        push_key(&mut out, &key("a\"b"));
        assert_eq!(out, "\"hex:612262\"");
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v, "hex:612262");
    }

    /// The header parses, names the format and version, and carries the
    /// vocabulary the rows use.
    #[test]
    fn the_header_describes_itself() {
        let mut out = String::new();
        let ident = ShardIdent {
            domain: "game.\"x\"".into(),
            seed: 20_260_731,
            content: 0xdead_beef,
            world: 0x1234,
            build: "0.8.0-gabc".into(),
            proto: 72,
        };
        format_header(
            &ident,
            "00ff",
            7,
            1_790_000_000,
            &Limits::default(),
            &mut out,
        );
        assert!(out.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(out.trim_end()).expect("valid JSON");
        assert_eq!(v["kind"], "header");
        assert_eq!(v["format"], TRUST_LOG_NAME);
        assert_eq!(v["version"], TRUST_LOG_FORMAT);
        assert_eq!(v["segment"], 7);
        assert_eq!(v["shard"]["domain"], "game.\"x\"");
        assert_eq!(v["shard"]["content"], "00000000deadbeef");
        assert_eq!(v["shard"]["proto"], 72);
        assert_eq!(v["verbs"][TRUST_DOOR.to_string()], "door");
        assert_eq!(v["presences"][PRESENCE_GONE.to_string()], "gone");
        assert_eq!(v["sync_ms"], TRUST_SYNC_MS);
    }

    #[test]
    fn segment_names_sort_as_their_sequence() {
        assert_eq!(segment_name(42), "trust-0000000042.jsonl");
        assert_eq!(segment_seq("trust-0000000042.jsonl"), Some(42));
        assert_eq!(segment_seq("trust-42.jsonl"), None);
        assert_eq!(segment_seq("trust-0000000042.jsonl.1"), None);
        assert!(segment_name(9) < segment_name(10));
    }
}
