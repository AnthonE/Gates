//! The I/O shell around `ShardCore`: wtransport termination, the session
//! hello (DESIGN.md §5.9), per-connection tasks, and the pinned 30 Hz sim
//! thread. Transport config of record per NETCODE.md §2.2: keep-alive
//! 10 s, idle 30 s, 64 KiB datagram send buffer, `send_datagram`
//! (drop-oldest) and never `_wait`.

use crate::config::ShardConfig;
use crate::core::{Admitted, Lane, ShardCore};
use crate::slot::{
    generation_of, state_of, Connect, EvMsg, FileMsg, Link, SaveMsg, SlotTable, SnapMsg, VoiceMsg,
    WorldDone, WorldMsg, WriteMsg, FILE_RING_CAP, MAX_CONNS, SLOT_LEAVING, SLOT_LIVE,
};
use crate::stats::ShardStats;
use crate::store::{PlayerKey, SaveFile, SaveStore, Saves};
use protocol::{
    decode_action, decode_chat, decode_hello, decode_input, encode_refuse, encode_welcome,
    peek_kind, ActionMsg, ChatMsg, ItemCatalog, Refuse, Welcome, KIND_CHAT, KIND_INPUT,
    MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES, PROTO_VER, REFUSE_FULL, REFUSE_VERSION,
};
use rtrb::RingBuffer;
use sim_core::input::BTN_MASK;
use sim_core::limits::{
    ACTION_RING_CAP, CHAT_RING_CAP, CTRL_RING_CAP, EVENT_RING_CAP, GRAVEYARD_RING_CAP,
    INPUT_RING_CAP, MAX_PLAYERS, MAX_SPECTATORS, SAVE_RING_CAP, SNAPSHOT_RING_CAP, TICK_HZ,
    VOICE_BURST, VOICE_FPS, VOICE_OUT_RING_CAP, VOICE_PER_TICK, VOICE_RING_CAP, WORLD_RING_CAP,
};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wtransport::endpoint::endpoint_side::Server;
use wtransport::endpoint::IncomingSession;
use wtransport::error::SendDatagramError;
use wtransport::tls::Sha256DigestFmt;
use wtransport::{Connection, Endpoint, Identity, RecvStream, SendStream, ServerConfig};

/// Handshake must finish inside this or the session drops (edge flood
/// control, DESIGN.md §10). Plumbing bound, DECISIONS.md §open row.
/// Bits of the slot's claim generation that reach a player id. See the id
/// derivation below: the ceiling is `limits::MOB_ID_TAG`, not arithmetic.
/// A slot retires at it rather than wrap (`SlotTable::claim`).
const PLAYER_ID_GEN_MASK: u32 = crate::slot::ID_GEN_MAX;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// What one connection may make this shard hold for it, QUIC-side. quinn's
/// defaults (100 + 100 streams, 1.25 MB a stream, an unbounded connection
/// window) are a general server's; this one reads exactly one bidi stream
/// (`accept_bi` once) and never reads another, so every byte a client parks
/// on a stream it opened is memory nobody frees until the connection does —
/// from an unauthenticated handshake too.
///
/// Bidi: the CONNECT request plus the game stream, with headroom. Uni: the
/// browser's HTTP/3 control and two QPACK streams (fewer and a browser
/// stalls), plus GREASE. Every frame the client sends is at most
/// `protocol::MAX_STREAM_MSG_BYTES` + 2, so a 64 KiB stream window is
/// hundreds of frames.
const QUIC_MAX_BIDI_STREAMS: u32 = 4;
const QUIC_MAX_UNI_STREAMS: u32 = 8;
const QUIC_STREAM_RWND: u32 = 64 * 1024;
const QUIC_CONN_RWND: u32 = 256 * 1024;
/// Inbound datagrams buffered before quinn drops the oldest: an input
/// datagram is ~74 B, so this is several seconds of a stalled reader.
const QUIC_DATAGRAM_RBUF: usize = 64 * 1024;
/// Unacknowledged outbound stream bytes. A client that never acks holds this
/// much, not quinn's 10 MB; a burst of join sync still moves a window a
/// round trip, megabytes a second at any real RTT.
const QUIC_SEND_WINDOW: u64 = 2 * 1024 * 1024;
const _: () = assert!(QUIC_STREAM_RWND as usize >= 64 * (protocol::MAX_STREAM_MSG_BYTES + 2));
const _: () = assert!(QUIC_CONN_RWND >= QUIC_STREAM_RWND);
/// The challenge's signed reply gets its own, longer deadline, because a
/// person answers it (`protocol::SIGN_WAIT_SECS`). The wait is counted in
/// [`SignWait`], not `in_flight`, so sign-ins nobody answers cannot shut the
/// door on everybody else for a minute. The QUIC session outlives the wait
/// on the transport's 10 s keep-alive.
const SIGN_WAIT: Duration = Duration::from_secs(protocol::SIGN_WAIT_SECS);
const _: () = assert!(protocol::SIGN_WAIT_SECS > HANDSHAKE_TIMEOUT.as_secs());
/// How often `wake_relay` looks for a published tick: the latency floor
/// between the sim pushing a frame and its writer sending it. One task polls
/// for every connection, where each writer used to poll its own rings every
/// 2 ms (~100k wakeups a second at a full shard for ~30 frames each).
const WAKE_POLL: Duration = Duration::from_millis(1);
/// A writer wakes at least this often with no tick published: a stalled sim
/// still has its leaves noticed and its telemetry sampled.
const WRITER_FALLBACK: Duration = Duration::from_millis(50);
/// How long a reader waits before it retries a full ring.
const RING_RETRY: Duration = Duration::from_millis(2);
/// Sim thread abandons backlog beyond this many ticks (a debugger pause,
/// a VM freeze) instead of sprinting to catch up.
const MAX_TICK_BACKLOG: u32 = 8;

/// The ticks a catch-up sprint has run off: what the world tick gained on
/// wall time, and so on every client's acks, with no round trip changing
/// (`ShardCore::note_pace_backlog`). Clock-free, so it is tested.
///
/// The backlog alone is not that. An overloaded shard (every tick longer
/// than a tick) only grows its backlog until it is dropped, and the world
/// falls behind wall time, so acks get younger, not older. What is gained
/// is the backlog's fall since its peak: the ticks run back to back.
#[derive(Default)]
struct PaceLead {
    /// The most whole ticks behind since the loop was last on time.
    peak: u16,
}

impl PaceLead {
    /// One iteration ended `behind` whole ticks late (0: on time). Returns
    /// the ticks run off since the peak.
    fn step(&mut self, behind: u16) -> u16 {
        self.peak = self.peak.max(behind);
        let run_off = self.peak - behind;
        if behind == 0 {
            self.peak = 0;
        }
        run_off
    }

    /// The backlog was abandoned, not run: nothing was gained.
    fn drop_backlog(&mut self) {
        self.peak = 0;
    }
}

/// How long the accept loop will keep draining the save ring after a
/// shutdown is signalled, waiting for the sim thread's final flush: 200
/// tries × 5 ms = 1 s.
///
/// A backstop and not the mechanism. The real exit is `save_rx` being
/// *abandoned* — the sim thread dropping its producer, which is exact — and
/// this only bounds the case where the sim thread is wedged rather than
/// finishing. A shutdown that takes a second is a shutdown; one that hangs
/// forever is a deploy that never completes.
const SHUTDOWN_DRAIN_TRIES: u32 = 200;

/// UDP socket buffer we ask the OS for, both directions
/// (`NETCODE.md` §2.2). quinn's README is explicit that its one socket
/// serves every connection and that the usual OS defaults (~208 KiB) are
/// too small for that; this is the row that had been an intention in a
/// table headed *config of record* since it was written.
///
/// **Asking is not getting**, which is why `bind_udp` reads the value back
/// and `ShardStats` carries both numbers. Linux silently clamps the request
/// to `net.core.rmem_max`/`wmem_max`, so on an un-tuned box this call
/// succeeds and changes nothing — the failure mode is a shard that believes
/// it has 8 MiB of headroom and has 208 KiB.
const UDP_BUF_BYTES: usize = 8 * 1024 * 1024;

/// In-flight handshakes past which an **unvalidated** address is answered
/// with a QUIC Retry instead of a handshake (`NETCODE.md` §2.2's "~2× cap").
/// Arithmetic over `MAX_PLAYERS`, not a knob of its own.
///
/// Under this, joining costs one round trip as before. Over it, a source
/// that cannot receive at the address it claims — every spoofed packet in a
/// reflection flood — is answered with a token and never seen again, while a
/// real joiner pays one extra round trip during a rush and nothing at all
/// the rest of the time.
const ADMIT_RETRY_AT: usize = 2 * MAX_PLAYERS;

/// In-flight handshakes past which a connection attempt is refused outright,
/// before any crypto (`NETCODE.md` §2.2's "hard cap").
///
/// Deliberately **not** the polite refusal: `REFUSE_FULL` travels to a
/// client that completed a handshake and tells it why, which is the right
/// answer to a full shard and the wrong one to a flood. Four times the
/// player cap means a shard can be entirely full, with a queue of the same
/// size again waiting, before anybody is turned away silently.
const ADMIT_REFUSE_AT: usize = 4 * MAX_PLAYERS;

/// Between transport-telemetry samples, per connection: cheap enough to leave
/// on in production and often enough to catch a congestion episode that
/// lasts a second.
const NET_SAMPLE_EVERY: Duration = Duration::from_secs(1);

// These two are **compile-time** rather than tests, which is the stronger
// form and is available because every term is a constant: get one wrong and
// the shard does not build, so there is no version of the tree where the
// admission gate is inverted and a suite is merely red.
//
// 1. Retry must come before refuse. Inverted, a flood is refused before it
//    is ever asked to validate its address and the Retry path — the only
//    thing that separates a spoofed source from a busy one — is unreachable.
// 2. Both must sit past a full shard, or an ordinary rush to join a popular
//    server is answered as an attack.
const _: () = assert!(ADMIT_RETRY_AT < ADMIT_REFUSE_AT);
const _: () = assert!(ADMIT_RETRY_AT > MAX_PLAYERS);

/// Handshakes one source may hold at once: an IPv4 address, or an IPv6 /64
/// (one subscriber's whole allocation, so a prefix of addresses is still one
/// source). `ADMIT_REFUSE_AT` is a count over everybody, and an unanswered
/// sign-in holds its place for `SIGN_WAIT`, so without this one address
/// opening a handful of sessions a second kept every other joiner out.
///
/// Taken only from an address that has proved it receives where it claims,
/// so a spoofed flood cannot spend a real player's allowance: at the door
/// when QUIC has already validated it (under load, the Retry above makes
/// every newcomer validate first), otherwise once its QUIC handshake has
/// completed. The shard's own host is exempt — loopback, which the load bots
/// and every wire suite dial from, the address it is bound to, which its
/// population dials (`population::dial_addr`), and `own_sources`, the public
/// address its house agents arrive from when it is bound to a wildcard.
/// Eight covers a household or a café joining at once.
const ADMIT_PER_SOURCE: u16 = 8;
/// Handshakes one **network** may hold at once: an IPv4 /24 or an IPv6 /48,
/// on top of `ADMIT_PER_SOURCE`. One subscriber's /56, a free tunnel /48 or
/// a rented /24 is hundreds of "sources", and fifty of them parking unsigned
/// sessions filled `ADMIT_REFUSE_AT` and refused everyone else at the door.
/// This makes that a dozen networks, not a dozen addresses; a carrier's
/// CGNAT /24 still fits a rush of joiners.
const ADMIT_PER_PREFIX: u16 = 4 * ADMIT_PER_SOURCE;
/// Buckets in the per-source table. Sources hash in with a key drawn at boot,
/// so nobody can aim at somebody else's bucket; a collision only makes two
/// sources share one allowance. The per-network table is the same size.
const SOURCE_BUCKETS: usize = 4096;
const _: () = assert!((ADMIT_PER_SOURCE as usize) < ADMIT_RETRY_AT);
const _: () = assert!(ADMIT_PER_PREFIX > ADMIT_PER_SOURCE);
const _: () = assert!((ADMIT_PER_PREFIX as usize) < ADMIT_RETRY_AT);
const _: () = assert!(SOURCE_BUCKETS.is_power_of_two());

/// Seats one source may hold at once, guests and wallets alike — the
/// handshake allowance's household, held. With guests admitted, nothing
/// else stopped one address joining eight at a time until every seat was
/// its own and everyone else was told the shard is full. Loopback, the
/// shard's own address and `own_sources` are exempt, as at the door (bots,
/// population, the house agents).
const SEATS_PER_SOURCE: usize = ADMIT_PER_SOURCE as usize;
const _: () = assert!(SEATS_PER_SOURCE < MAX_PLAYERS);

/// In-flight handshakes per source (`ADMIT_PER_SOURCE`) and per network
/// (`ADMIT_PER_PREFIX`).
struct SourceGate {
    held: Box<[std::sync::atomic::AtomicU16]>,
    nets: Box<[std::sync::atomic::AtomicU16]>,
    key: std::collections::hash_map::RandomState,
    /// The shard's own addresses: the bound one when it is specific, and
    /// `own_sources` (the host's public ones, which a wildcard bind hides).
    own: Box<[std::net::IpAddr]>,
}

/// One source's place in [`SourceGate`], given back on drop — however the
/// handshake that holds it ends. A loopback permit holds no bucket.
struct SourcePermit {
    gate: Arc<SourceGate>,
    bucket: Option<(usize, usize)>,
}

impl Drop for SourcePermit {
    fn drop(&mut self) {
        if let Some((b, n)) = self.bucket {
            self.gate.held[b].fetch_sub(1, Ordering::Relaxed);
            self.gate.nets[n].fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl SourceGate {
    fn new(bound: Option<std::net::IpAddr>, own_sources: &[std::net::IpAddr]) -> Self {
        let table = || {
            (0..SOURCE_BUCKETS)
                .map(|_| std::sync::atomic::AtomicU16::new(0))
                .collect()
        };
        Self {
            held: table(),
            nets: table(),
            key: std::collections::hash_map::RandomState::new(),
            own: bound
                .into_iter()
                .chain(own_sources.iter().copied())
                .map(|ip| ip.to_canonical())
                .filter(|ip| !ip.is_unspecified())
                .collect(),
        }
    }

    /// The bucket `ip` counts against: the whole IPv4 address, or the /64 of
    /// an IPv6 one (a v4-mapped v6 address is its v4 address).
    fn bucket(&self, ip: std::net::IpAddr) -> usize {
        use std::hash::BuildHasher;
        let h = match ip.to_canonical() {
            std::net::IpAddr::V4(v4) => self.key.hash_one(v4.octets()),
            std::net::IpAddr::V6(v6) => self.key.hash_one(&v6.octets()[..8]),
        };
        h as usize & (SOURCE_BUCKETS - 1)
    }

    /// The network bucket `ip` counts against: its IPv4 /24 or IPv6 /48.
    fn net_bucket(&self, ip: std::net::IpAddr) -> usize {
        use std::hash::BuildHasher;
        let h = match ip.to_canonical() {
            std::net::IpAddr::V4(v4) => self.key.hash_one(&v4.octets()[..3]),
            std::net::IpAddr::V6(v6) => self.key.hash_one(&v6.octets()[..6]),
        };
        h as usize & (SOURCE_BUCKETS - 1)
    }

    /// `ip` as this gate counts it: canonical, or `None` for the shard's own
    /// host — loopback or one of its own addresses — which is never counted.
    fn counted(&self, ip: std::net::IpAddr) -> Option<std::net::IpAddr> {
        let ip = ip.to_canonical();
        (!ip.is_loopback() && !self.own.contains(&ip)).then_some(ip)
    }

    /// A place for one more handshake from `ip`, or `None` if its source
    /// already holds `ADMIT_PER_SOURCE` or its network `ADMIT_PER_PREFIX`.
    /// The shard's own host always gets one, uncounted.
    fn take(gate: &Arc<Self>, ip: std::net::IpAddr) -> Option<SourcePermit> {
        let bucket = if let Some(ip) = gate.counted(ip) {
            let b = gate.bucket(ip);
            if gate.held[b].fetch_add(1, Ordering::Relaxed) >= ADMIT_PER_SOURCE {
                gate.held[b].fetch_sub(1, Ordering::Relaxed);
                return None;
            }
            let n = gate.net_bucket(ip);
            if gate.nets[n].fetch_add(1, Ordering::Relaxed) >= ADMIT_PER_PREFIX {
                gate.nets[n].fetch_sub(1, Ordering::Relaxed);
                gate.held[b].fetch_sub(1, Ordering::Relaxed);
                return None;
            }
            Some((b, n))
        } else {
            None
        };
        Some(SourcePermit {
            gate: gate.clone(),
            bucket,
        })
    }
}

/// Sign-ins that may wait on their signature at once (`SIGN_WAIT`). Past it
/// a waiter is dropped for the newcomer: the oldest of **the source holding
/// the most seats** ([`SignWait::join`]), a source here being the network
/// its handshake counts against (`ADMIT_PER_PREFIX`'s /24 or /48). A flood
/// of challenges nobody answers churns its own seats, and a slow signer — a
/// wallet prompt and a person reading it — is dropped only once every
/// waiting network holds as few seats as its own. Oldest-first alone let
/// fifty addresses drop every human signer, every time. And a flood can no
/// longer refuse the shard to everybody, which it did while every unanswered
/// challenge sat in `in_flight` for a minute.
const SIGN_WAITERS: usize = ADMIT_REFUSE_AT;

/// Low bits of a seat's word: its waiter's source, a [`SourceGate`] network
/// bucket, or [`SIGN_HOST`] for the shard's own host (uncounted at the door, and one
/// source here).
const SIGN_SOURCE_BITS: u32 = SOURCE_BUCKETS.trailing_zeros() + 1;
const SIGN_HOST: usize = SOURCE_BUCKETS;
const _: () = assert!(SIGN_HOST < 1 << SIGN_SOURCE_BITS);

/// The sign-ins waiting on a signature ([`SIGN_WAITERS`] seats). Lock-free
/// like the rest of the net side: each seat holds one word, its waiter's
/// join number over its source (`number << SIGN_SOURCE_BITS | source`; 0 =
/// empty), so a seat changes hands in one compare-and-swap and the lowest
/// word held is the oldest waiter.
struct SignWait {
    seats: Box<[SignSeat]>,
    /// Seats held per source ([`SIGN_HOST`] + 1 of them), plus that source's
    /// joiners still looking: counted before a seat is taken and uncounted
    /// after it is lost, so a count can run ahead of its seats but never
    /// behind them, and never wraps.
    per: Box<[std::sync::atomic::AtomicU32]>,
    /// The next join number; starts at 1.
    next: std::sync::atomic::AtomicU64,
}

struct SignSeat {
    held: std::sync::atomic::AtomicU64,
    /// Woken when a newer waiter takes this seat.
    taken: tokio::sync::Notify,
}

/// One waiter's seat in [`SignWait`], given back on drop.
struct SignTicket {
    pool: Arc<SignWait>,
    seat: usize,
    word: u64,
}

impl SignWait {
    fn new() -> Self {
        Self {
            seats: (0..SIGN_WAITERS)
                .map(|_| SignSeat {
                    held: std::sync::atomic::AtomicU64::new(0),
                    taken: tokio::sync::Notify::new(),
                })
                .collect(),
            per: (0..=SIGN_HOST)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect(),
            next: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// The source a seat's word names.
    fn source(word: u64) -> usize {
        (word & ((1 << SIGN_SOURCE_BITS) - 1)) as usize
    }

    /// Join the wait from `source` — the network bucket of the joiner's
    /// [`SourcePermit`], `None` for the shard's own host: an empty seat, or,
    /// when all are taken, the oldest seat of the source holding the most
    /// (counting this joiner, so a source at the top drops its own oldest).
    /// That waiter is woken to leave.
    fn join(pool: &Arc<Self>, source: Option<usize>) -> SignTicket {
        let source = source.unwrap_or(SIGN_HOST);
        let number = pool.next.fetch_add(1, Ordering::Relaxed);
        let word = number << SIGN_SOURCE_BITS | source as u64;
        // Before any seat is taken: see `per`.
        pool.per[source].fetch_add(1, Ordering::Relaxed);
        let ticket = |seat| SignTicket {
            pool: pool.clone(),
            seat,
            word,
        };
        loop {
            // (its source's count, its word, its seat) of the waiter to drop.
            let mut victim: Option<(u32, u64, usize)> = None;
            for (i, s) in pool.seats.iter().enumerate() {
                let held = s.held.load(Ordering::Acquire);
                if held == 0 {
                    let took =
                        s.held
                            .compare_exchange(0, word, Ordering::AcqRel, Ordering::Acquire);
                    if took.is_ok() {
                        return ticket(i);
                    }
                    continue;
                }
                let count = pool.per[Self::source(held)].load(Ordering::Relaxed);
                if victim.is_none_or(|(c, w, _)| count > c || (count == c && held < w)) {
                    victim = Some((count, held, i));
                }
            }
            if let Some((_, held, i)) = victim {
                let seat = &pool.seats[i];
                let took =
                    seat.held
                        .compare_exchange(held, word, Ordering::AcqRel, Ordering::Acquire);
                if took.is_ok() {
                    pool.per[Self::source(held)].fetch_sub(1, Ordering::Relaxed);
                    seat.taken.notify_waiters();
                    return ticket(i);
                }
            }
            // Raced another joiner or a leaver: look again.
        }
    }
}

impl SignTicket {
    /// Resolves once a newer waiter has taken this one's seat.
    async fn bumped(&self) {
        let seat = &self.pool.seats[self.seat];
        loop {
            // Armed before the look, so a take between the two still wakes.
            let taken = seat.taken.notified();
            tokio::pin!(taken);
            taken.as_mut().enable();
            if seat.held.load(Ordering::Acquire) != self.word {
                return;
            }
            taken.await;
        }
    }
}

impl Drop for SignTicket {
    fn drop(&mut self) {
        // A seat already taken by a newer waiter is theirs, and was
        // uncounted by the one who took it: left alone.
        let left = self.pool.seats[self.seat].held.compare_exchange(
            self.word,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if left.is_ok() {
            self.pool.per[SignWait::source(self.word)].fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Sign-ins past their signature at once: its check, the ticket read on a
/// blocking thread (`entitle::check_one`, up to its timeout) and the hand-off
/// to the accept loop. `in_flight` bounded this work until the wait left it;
/// a gate of its own rather than back in `in_flight`, so signers nobody
/// answers still cannot shut the door. Well under tokio's 512 blocking
/// threads, which the skin and face reads share.
const POST_SIGN_PERMITS: usize = MAX_PLAYERS;

/// One source as `SourceGate` counts it: the same IPv4 address, or the same
/// IPv6 /64.
fn same_source(a: std::net::IpAddr, b: std::net::IpAddr) -> bool {
    match (a, b) {
        (std::net::IpAddr::V4(a), std::net::IpAddr::V4(b)) => a == b,
        (std::net::IpAddr::V6(a), std::net::IpAddr::V6(b)) => a.octets()[..8] == b.octets()[..8],
        _ => false,
    }
}

/// Watcher seats one source may hold at once: a quarter of the ceiling, so
/// a single address cannot take every `?spectate=` seat on the shard. The
/// player cap (`SEATS_PER_SOURCE`) counts only player seats, and a watcher's
/// handshake permit is given back the moment it is seated.
const WATCH_SEATS_PER_SOURCE: usize = MAX_SPECTATORS / 4;
const _: () = assert!(WATCH_SEATS_PER_SOURCE >= 1 && WATCH_SEATS_PER_SOURCE < MAX_SPECTATORS);

/// Watcher seats held by connections from `ip`'s source
/// (`WATCH_SEATS_PER_SOURCE`).
fn watch_seats_held(seats: &[SeatSlot; MAX_SPECTATORS], ip: std::net::IpAddr) -> usize {
    seats
        .iter()
        .filter(|s| s.occupied && s.source.is_some_and(|x| same_source(x, ip)))
        .count()
}

/// Live seats held by connections from `ip`'s source (`SEATS_PER_SOURCE`).
fn seats_held(keys: &[KeySlot; MAX_PLAYERS], slots: &SlotTable, ip: std::net::IpAddr) -> usize {
    keys.iter()
        .enumerate()
        .filter(|(s, k)| {
            let w = slots.load(*s);
            state_of(w) == SLOT_LIVE
                && generation_of(w) == k.generation
                && k.source.is_some_and(|x| same_source(x, ip))
        })
        .count()
}

/// Binds the shard's UDP socket with the buffer sizes §2.2 asks for, then
/// **reads back what the OS actually granted** and records both.
///
/// The readback is the point. `setsockopt(SO_RCVBUF)` does not fail when the
/// kernel decides you may not have that much — it clamps to `rmem_max` and
/// returns success — so the only way to know what a shard is running on is
/// to ask it afterwards. (Linux also reports back roughly double what it
/// granted, its own bookkeeping overhead included; the number recorded here
/// is what the OS reports, unadjusted, because inventing a halving would be
/// a second guess layered on the first.)
///
/// A failure to set a buffer is **not** a boot failure: a shard that runs
/// with a small socket is a shard that runs. A failure to *bind* is.
fn bind_udp(addr: SocketAddr, stats: &ShardStats) -> std::io::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let sock = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    // Dual-stack when bound to `[::]`, matching what `with_bind_address`
    // did before this function existed — otherwise a v6 bind would stop
    // accepting the v4 clients it used to.
    if addr.is_ipv6() && addr.ip().is_unspecified() {
        let _ = sock.set_only_v6(false);
    }
    // Before `bind`, which is where the kernel sizes the buffers.
    let _ = sock.set_recv_buffer_size(UDP_BUF_BYTES);
    let _ = sock.set_send_buffer_size(UDP_BUF_BYTES);
    sock.bind(&addr.into())?;
    sock.set_nonblocking(true)?;

    stats
        .net_rcvbuf_asked
        .store(UDP_BUF_BYTES as u64, Ordering::Relaxed);
    stats
        .net_sndbuf_asked
        .store(UDP_BUF_BYTES as u64, Ordering::Relaxed);
    if let Ok(got) = sock.recv_buffer_size() {
        stats.net_rcvbuf_bytes.store(got as u64, Ordering::Relaxed);
    }
    if let Ok(got) = sock.send_buffer_size() {
        stats.net_sndbuf_bytes.store(got as u64, Ordering::Relaxed);
    }
    Ok(sock.into())
}
const SHUTDOWN_DRAIN_POLL: Duration = Duration::from_millis(5);

/// Chat rate limit (ALPHA.md §1: "rate-limited server-side"), a token
/// bucket per connection: `CHAT_BURST` lines may go back to back, then
/// one more every `CHAT_REFILL` — a sustained ~30 lines/min with room to
/// answer a question fast. Proposed defaults, DECISIONS.md §open
/// ("chat v0"). Refused lines are counted, not announced: the limiter is
/// the shard's business, not a conversation with the spammer.
const CHAT_BURST: u32 = protocol::chat::CHAT_BURST;
const CHAT_REFILL: Duration = Duration::from_millis(protocol::chat::CHAT_REFILL_MS);

/// One connection's chat token bucket. Lives in the reader task, so it is
/// per stream — a reconnect buys a fresh bucket, and the handshake costs
/// far more than the three lines that would buy.
struct ChatLimiter {
    tokens: u32,
    /// When the bucket was last accounted; None until the first line.
    last: Option<Instant>,
}

impl ChatLimiter {
    fn new() -> Self {
        Self {
            tokens: CHAT_BURST,
            last: None,
        }
    }

    /// Spend a token if one has accrued. `now` is a parameter, not a
    /// `Instant::now()` call inside, so the policy is testable without
    /// sleeping.
    fn allow(&mut self, now: Instant) -> bool {
        let last = *self.last.get_or_insert(now);
        let steps = now.saturating_duration_since(last).as_nanos() / CHAT_REFILL.as_nanos();
        if steps >= CHAT_BURST as u128 {
            // Idle long enough to fill the bucket. The clock restarts
            // here on purpose: leaving `last` in the deep past would hand
            // out a full bucket on every call from then on — a limiter
            // that stops limiting, which is the failure mode worth
            // naming.
            self.tokens = CHAT_BURST;
            self.last = Some(now);
        } else if steps > 0 {
            let steps = steps as u32;
            self.tokens = (self.tokens + steps).min(CHAT_BURST);
            self.last = Some(last + CHAT_REFILL * steps);
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

pub struct ShardHandle {
    pub local_addr: SocketAddr,
    /// SHA-256 of the dev certificate, `serverCertificateHashes` format.
    pub cert_hash: String,
    pub stats: Arc<ShardStats>,
    pub shutdown: Arc<AtomicBool>,
}

/// Bake the item-name catalog from validated content (the same
/// index-is-sorted-rank mapping `bake_gather` uses). Boot path: a name
/// the wire can't carry refuses the boot, same as any other bake error.
/// Each row carries its condition ceiling (wire v46) in the u16 hundredths
/// the sim runs on — the same conversion `bake::bake_gather` performs, and
/// the same refusal when a `condition_max` overflows it.
///
/// The armor columns (wire v52) are read off the **already-baked**
/// `CombatContent` rather than off `content.armors`, and the parameter
/// exists only to force that. A second walk of the authored rows here
/// would be a second derivation of the same fact — the `head`/`body`
/// string mapping, the item-index resolution and the reduction — and the
/// two would agree until the day one of them was edited, at which point
/// the client would draw a protection total off table A while `hurt`
/// charged off table B. Every gate would be green: the golden pins what
/// this function produced, and the sim's own armor suite never reads the
/// catalog. One table, one reader.
///
/// `gather` (wire v64) is there for exactly the same reason and it is the
/// stronger case of the two, because the number is one the sim *charges*:
/// `stack_max` is what `inventory::plan_move` measures every merge
/// against, and the client now aims a quick-move with it. Read off the
/// baked table rather than off `item.stack`, so a client that decides
/// "this slot has room for 40 more" is reading the same `u16` the sim will
/// refuse it against, down to the conversion — `bake_gather`'s
/// `u16::try_from` is the one place that narrowing happens.
///
/// `survival` (wire v81) is the third baked table read here, for the eat
/// columns: the client makes a left click with food in hand a meal off
/// them, so they are the rows `survival::consume` will charge.
///
/// `cook` (wire v89) is the fourth, for the oven roles: which section of a
/// camp fire each item goes in, the same `CookContent::roles` the move verb
/// refuses against.
pub fn bake_catalog(
    content: &content::Content,
    combat: &sim_core::combat::CombatContent,
    gather: &sim_core::gather::GatherContent,
    survival: &sim_core::survival::SurvivalContent,
    cook: &sim_core::oven::CookContent,
) -> Result<ItemCatalog, String> {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = content.items.len() as u16;
    for item in &content.items {
        let idx = content.item_index(&item.id).expect("own id resolves") as usize;
        let cond_max = u16::try_from(item.condition_max).map_err(|_| {
            format!(
                "catalog: item `{}` condition_max {} overflows u16 hundredths",
                item.id, item.condition_max
            )
        })?;
        let armor = combat.armor[idx];
        let eat = survival.row(idx as u16).unwrap_or_default();
        // The draw: what a right mouse does with this item and when the bow
        // is ready (`ItemRow::draw_ticks`). Bytes on the wire, so a draw or
        // a cadence past 255 ticks is refused here rather than truncated.
        let ranged = combat.ranged[idx];
        let (draw_ticks, nock_ticks) = if ranged.draw_ticks > 0 {
            (
                u8::try_from(ranged.draw_ticks),
                u8::try_from(ranged.rate_ticks),
            )
        } else {
            (Ok(0), Ok(0))
        };
        let (Ok(draw_ticks), Ok(nock_ticks)) = (draw_ticks, nock_ticks) else {
            return Err(format!(
                "catalog: `{}` draws for {} ticks and nocks for {}, past a byte",
                item.id, ranged.draw_ticks, ranged.rate_ticks
            ));
        };
        // A gun's handling (v126): its cadence a byte of ticks, refused past
        // one as the draw's is; the rest `content::validate` bounded.
        let gun = if ranged.damage > 0 && ranged.hitscan {
            let g = ranged.gun;
            let rate_ticks = u8::try_from(ranged.rate_ticks).map_err(|_| {
                format!(
                    "catalog: `{}` fires every {} ticks, past a byte",
                    item.id, ranged.rate_ticks
                )
            })?;
            Some(protocol::GunRow {
                rate_ticks,
                reload_ticks: ranged.reload_ticks,
                semi: g.semi,
                pellets: g.pellets,
                aim: g.aim,
                hip: g.hip,
                recoil_pitch: g.recoil_pitch,
                recoil_yaw: g.recoil_yaw,
                ads_recoil_pct: g.ads_recoil_pct,
                rate_sub: u16::try_from(ranged.rate_sub)
                    .map_err(|_| format!("catalog: `{}` cadence overflows the wire", item.id))?,
                moving: g.moving,
                per_shot: g.per_shot,
                max: g.max,
                recover_ticks: g.recover_ticks,
                delay_ticks: g.delay_ticks,
                pellet: g.pellet,
                speed_mmpt: combat
                    .ammo_def(ranged.ammo[0])
                    .filter(|a| a.speed_mmpt > 0)
                    .map_or(0, |a| g.muzzle(a.speed_mmpt)),
                drop_mmpt2: combat
                    .ammo_def(ranged.ammo[0])
                    .map_or(0, |a| a.drop_mmpt2.min(63) as u8),
                mods: g.mods,
            })
        } else {
            None
        };
        cat.set(
            idx,
            item.name.as_bytes(),
            protocol::ItemRow {
                cond_max,
                armor_pct: armor.reduction_pct,
                wear_slot: armor.slot,
                // Per mille in the sim, whole percent on the wire; only a
                // worn piece carries them (`ItemRow::coherent`).
                warmth_pct: if armor.slot == sim_core::combat::WEAR_NONE {
                    0
                } else {
                    (survival.exposure.warmth[idx] / 10).clamp(-100, 100) as i8
                },
                ward_pct: if armor.slot == sim_core::combat::WEAR_NONE {
                    0
                } else {
                    (survival.exposure.ward[idx] / 10).min(100) as u8
                },
                stack_max: gather.stack_max_of(idx as u16),
                food: eat.food,
                water: eat.water,
                health: eat.health,
                draw_ticks,
                nock_ticks,
                oven: cook.packed_roles(idx as u16),
                holster: sim_core::combat::drawn_weapon(combat, gather, idx as u16),
                revive: survival.revives(idx as u16),
                use_ticks: eat.use_ticks,
                pour: survival.vessel(idx as u16).is_some(),
                class: item.class.code(),
                // What one hit takes off a body (v131), the order the sim
                // reads a hand in: a gun or a bow, else a swing, else a
                // throw.
                damage: combat
                    .held_ranged(idx as u16)
                    .map(|d| d.damage)
                    .or_else(|| combat.held_melee(idx as u16).map(|d| d.damage))
                    .or_else(|| combat.held_throw(idx as u16).map(|d| d.damage))
                    .unwrap_or(0),
                gun,
                // An attachment (v144): its bit and slot off the sim's
                // table, the view's half (kick, zoom, laser, light) off the
                // content row, which the sim never reads.
                attach: combat.mod_bit(idx as u16).and_then(|bit| {
                    let m = combat.mods[bit as usize];
                    let a = content.attachments.get(bit as usize)?;
                    Some(protocol::ModRow {
                        bit,
                        group: m.group,
                        aim_pct: m.aim_pct,
                        hip_pct: m.hip_pct,
                        recoil_pct: a.recoil_pct as u16,
                        zoom_pct: a.zoom_pct as u8,
                        quiet: m.quiet,
                        laser: a.laser,
                        light: a.light,
                    })
                }),
            },
        )
        .map_err(|_| {
            format!(
                "catalog: item `{}` name `{}` is empty or over {} bytes, or its \
                 row (armor {} % in slot {}, condition {} on a stack of {}) is one \
                 the sim cannot mean",
                item.id,
                item.name,
                protocol::MAX_ITEM_NAME_BYTES,
                armor.reduction_pct,
                armor.slot,
                cond_max,
                gather.stack_max_of(idx as u16),
            )
        })?;
    }
    Ok(cat)
}

/// Every content table a shard runs on, in one value.
///
/// **This is a struct because it was thirteen positional arguments and one
/// of them went missing for the whole life of the feature.** `bake_research`
/// was written, validated, tested and never called: `sim_thread` installed
/// eleven tables, `World::research` stayed `ResearchContent::EMPTY`, every
/// `Command::Research` refused `REFUSE_R_ITEM`, and the six
/// `blueprint = true` recipes were uncraftable by anyone on a live shard —
/// with every gate in the tree green, because no gate boots a shard and
/// asks what it installed. A twelfth positional argument threaded through
/// two signatures and ten call sites is not a fix, it is the same
/// conditions with one more chance to miss.
///
/// So: one struct, one constructor, and adding a table is a field the
/// compiler makes every caller acknowledge. `tests/boot_tables.rs` holds
/// the other half — that every `bake_*` the content crate exposes has a
/// home here at all.
pub struct SimTables {
    pub gather: sim_core::gather::GatherContent,
    pub craft: sim_core::craft::CraftContent,
    pub build: sim_core::build::BuildContent,
    pub deploy: sim_core::deploy::DeployContent,
    pub combat: sim_core::combat::CombatContent,
    pub backpack: sim_core::backpack::BackpackContent,
    pub survival: sim_core::survival::SurvivalContent,
    pub cook: sim_core::oven::CookContent,
    pub spawn_kit: sim_core::inventory::SpawnKit,
    pub loot: sim_core::loot::LootContent,
    pub mobs: sim_core::mob::MobContent,
    /// Bred brains for the roster (`mobs.toml` `genome`); empty leaves every
    /// animal on its rule table.
    pub nets: sim_core::brain::Nets,
    /// The attack helicopter (`heli.rs`). Armed by the boot alone: test
    /// shards assign their tables field by field and leave it inert.
    pub heli: sim_core::heli::HeliDef,
    /// THE GATE's sentries (`sentry.rs`), armed by the boot alone like the
    /// heli.
    pub sentry: sim_core::sentry::SentryDef,
    /// A player auto turret's guns (`deployables.toml` `[turret]`).
    pub turret: sim_core::turret::TurretDef,
    /// Electricity's numbers (`deployables.toml` `[power]`).
    pub power: sim_core::power::PowerContent,
    pub research: sim_core::research::ResearchContent,
    pub catalog: ItemCatalog,
    /// Each item's description line (`content/items.toml` `description`).
    pub item_descs: Box<protocol::ItemDescs>,
    /// The skin catalog, twice: the sim's half (what fits what) and the
    /// wire's (names, tints, prices), both from `content/skins.toml`.
    pub skins: sim_core::skin::SkinContent,
    pub skin_catalog: Box<protocol::SkinCatalog>,
    /// The town's vendor offers (`content/sites.toml`) and their names.
    pub vend: sim_core::vend::VendContent,
    pub vendor_names: Vec<String>,
    /// The keycard each ziggurat door takes (`monument::DOORS`).
    pub cards: [u16; sim_core::monument::CARD_DOORS],
    /// The arc (`content/arc.toml`): the works, and the names the work drip
    /// carries.
    pub works: sim_core::works::WorksContent,
    pub work_names: Vec<String>,
    pub unlock_names: Vec<String>,
    /// Speakers, inscriptions and mechanisms, and the words the server
    /// answers with (`ARC.md` F5–F7).
    pub lore: sim_core::lore::LoreContent,
    pub mechs: sim_core::mech::MechContent,
    pub arc_text: content::bake::ArcText,
    /// What the standings weigh (`standings.rs`, `arc.toml` `[standings]`).
    pub standing_rules: crate::standings::Rules,
    /// The achievement list as tables (`achievements.rs`).
    pub achievements: crate::achievements::Baked,
}

/// Bake every table a shard needs, or refuse the boot naming the one that
/// failed. The single place the list of tables is written down.
pub fn bake_all(content: &content::Content) -> Result<SimTables, String> {
    // Combat and gather are baked into locals first because the catalog
    // reads their rows rather than re-deriving them (`bake_catalog`).
    let combat = content.bake_combat()?;
    let gather = content.bake_gather()?;
    let survival = content.bake_survival()?;
    let cook = content.bake_cooking()?;
    let deploy = content.bake_deployables()?;
    let achievements = crate::achievements::Baked::bake_from(content, &deploy);
    Ok(SimTables {
        craft: content.bake_craft()?,
        build: content.bake_building()?,
        deploy,
        achievements,
        backpack: content.bake_backpack()?,
        spawn_kit: content.bake_spawn_kit()?,
        loot: content.bake_loot()?,
        mobs: content.bake_mobs()?,
        nets: content.bake_nets()?,
        heli: content.bake_heli()?,
        sentry: content.bake_sentry()?,
        turret: content.bake_turret()?,
        power: content.bake_power()?,
        research: content.bake_research()?,
        catalog: bake_catalog(content, &combat, &gather, &survival, &cook)?,
        item_descs: bake_item_descs(content)?,
        skins: content.bake_skins()?,
        skin_catalog: bake_skin_catalog(content)?,
        vend: content.bake_vend()?,
        vendor_names: content.bake_vendor_names(),
        cards: content.bake_cards()?,
        works: content.bake_arc()?,
        work_names: content.bake_work_names(),
        unlock_names: content.bake_unlock_names(),
        lore: content.bake_lore()?,
        mechs: content.bake_mechs()?,
        arc_text: content.bake_arc_text()?,
        standing_rules: crate::standings::Rules::bake_from(content, content.bake_worth()),
        combat,
        gather,
        survival,
        cook,
    })
}

/// Every baked table into a fresh core, as a shard's boot installs them:
/// the sim thread's list and a lockstep island's (`lockstep.rs`), one list.
/// What belongs to neither the world nor the core comes back — the
/// standings' rules and the achievements, which open beside files the
/// caller owns.
pub fn install_tables(
    core: &mut ShardCore,
    tables: SimTables,
) -> (crate::standings::Rules, crate::achievements::Baked) {
    // Destructured rather than field-by-field off `tables`, so that adding a
    // table and forgetting to install it is a compile error naming the field
    // — which is the whole reason this is a struct (`SimTables`).
    let SimTables {
        gather,
        craft,
        build,
        deploy,
        combat,
        backpack,
        survival,
        cook,
        spawn_kit,
        loot,
        mobs,
        nets,
        heli,
        sentry,
        turret,
        power,
        research,
        catalog,
        item_descs,
        skins,
        skin_catalog,
        vend,
        vendor_names,
        cards,
        works,
        work_names,
        unlock_names,
        lore,
        mechs,
        arc_text,
        standing_rules,
        achievements,
    } = tables;
    core.world.gather = gather;
    core.world.craft = craft;
    core.world.build = build;
    core.world.deploy = deploy;
    core.world.combat = combat;
    core.world.backpack = backpack;
    core.world.survival = survival;
    core.world.cook = cook;
    core.world.spawn_kit = spawn_kit;
    core.world.loot = loot;
    core.world.mob = mobs;
    *core.world.nets = nets;
    core.world.heli_def = heli;
    core.world.sentry_def = sentry;
    core.world.turret_def = turret;
    core.world.power_def = power;
    core.world.research = research;
    core.world.skins = skins;
    core.catalog = catalog;
    core.item_descs = item_descs;
    core.skin_catalog = skin_catalog;
    core.world.vend = vend;
    core.vendor_names = vendor_names;
    core.world.cards = cards;
    core.world.works_def = works;
    core.world.works.refresh(&core.world.works_def);
    core.work_names = work_names;
    core.unlock_names = unlock_names;
    core.world.lore_def = lore;
    core.world.mech_def = mechs;
    core.arc_text = arc_text;
    (standing_rules, achievements)
}

/// Each item's description line, by item index (`SUB_ITEM_DESC`). A line
/// over `MAX_ITEM_DESC_BYTES` refuses the boot.
pub fn bake_item_descs(content: &content::Content) -> Result<Box<protocol::ItemDescs>, String> {
    let mut d = Box::new(protocol::ItemDescs::EMPTY);
    for item in &content.items {
        let idx = content.item_index(&item.id).expect("own id resolves") as usize;
        d.set(idx, item.description.as_bytes()).map_err(|_| {
            format!(
                "item `{}`: description is over {} bytes",
                item.id,
                protocol::MAX_ITEM_DESC_BYTES
            )
        })?;
    }
    Ok(d)
}

/// A painted skin's texture as the wire names it: the first 8 bytes of the
/// PNG's sha256 (content validation has made it 64 lowercase hex).
fn texture_prefix(hex: Option<&str>) -> [u8; 8] {
    let mut out = [0u8; 8];
    if let Some(h) = hex.filter(|h| h.len() >= 16) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap_or(0);
        }
    }
    out
}

/// The wire's skin catalog (skins v0), in the same row order as
/// `Content::bake_skins`, which is the order an owned set indexes.
pub fn bake_skin_catalog(content: &content::Content) -> Result<Box<protocol::SkinCatalog>, String> {
    let mut cat = Box::new(protocol::SkinCatalog::EMPTY);
    for (i, s) in content.skins.iter().enumerate() {
        let covers = content
            .item_index(&s.covers)
            .ok_or_else(|| format!("skin `{}` covers `{}`: not an item", s.id, s.covers))?;
        let (coin, price) = match (s.coin, s.price) {
            (Some(content::schema::Coin::Elo), Some(p)) => (protocol::COIN_ELO, p),
            (Some(content::schema::Coin::Orbs), Some(p)) => (protocol::COIN_ORBS, p),
            (Some(content::schema::Coin::Junk), Some(p)) => (protocol::COIN_JUNK, p),
            _ => (protocol::COIN_NONE, 0),
        };
        cat.set(
            i,
            s.name.as_bytes(),
            protocol::SkinRow {
                catalog: s.catalog,
                covers,
                tint: s.tint,
                coin,
                price,
                texture: texture_prefix(s.texture.as_deref()),
                unlock: s.unlock,
            },
        )
        .map_err(|_| {
            format!(
                "skin `{}`: name `{}` is empty or over {} bytes, or its row is one the \
                 wire cannot carry",
                s.id,
                s.name,
                protocol::MAX_ITEM_NAME_BYTES
            )
        })?;
    }
    cat.count = content.skins.len() as u16;
    Ok(cat)
}

/// Stop threads if startup fails or its future is cancelled before handoff.
struct PendingStartup(Option<Arc<AtomicBool>>);

impl Drop for PendingStartup {
    fn drop(&mut self) {
        if let Some(shutdown) = &self.0 {
            shutdown.store(true, Ordering::Relaxed);
        }
    }
}

/// Boot a shard: bind, initialize the sim, then start accepting connections.
/// The caller owns process lifetime; `shutdown` stops the sim thread.
/// `tables` is the content bake (CLAUDE.md wall 7) — data the world runs
/// on, handed over before the first tick like the seed.
///
/// `saves` is the player store, already opened and validated against this
/// seed and content (`store::open`) — or `Saves::off()`, which is a shard
/// that was told to remember nobody and is what every test here runs. It is
/// a parameter rather than something opened in here because validating it
/// needs the *content hash*, which this function is never handed: the
/// binary bakes content and therefore the binary opens the file, and a
/// refusal lands before a port is bound.
pub async fn spawn_shard(
    cfg: ShardConfig,
    tables: SimTables,
    saves: Saves,
    world_boot: crate::worldfile::WorldBoot,
) -> Result<ShardHandle, String> {
    // The island validates at boot the way content does (CLAUDE.md wall 7),
    // and here rather than in `bin/shard.rs` so that every path that raises a
    // shard — the binary, the bots, the wire tests — refuses the same seed.
    // First, before an identity is loaded or a port is bound: a refusal this
    // cheap should cost neither.
    crate::boot::check_seed(cfg.seed)?;

    // A PUBLIC shard serves a real certificate chain and browsers trust it
    // outright; the dev flow self-signs for loopback and the page passes the
    // hash below through `serverCertificateHashes`. Both or neither is
    // enforced at parse (config.rs), so one `is_some` decides it.
    let identity = match (&cfg.cert_pem, &cfg.key_pem) {
        (Some(cert), Some(key)) => Identity::load_pemfiles(cert, key)
            .await
            .map_err(|e| format!("loading {cert} / {key}: {e}"))?,
        _ => Identity::self_signed(["localhost", "127.0.0.1", "::1"])
            .map_err(|e| format!("self-signed identity: {e}"))?,
    };
    let cert_hash = identity
        .certificate_chain()
        .as_slice()
        .first()
        .ok_or("empty certificate chain")?
        .hash()
        .fmt(Sha256DigestFmt::DottedHex);

    // Built before the endpoint, because the socket bind below records what
    // the OS granted and needs somewhere to put it.
    let stats = Arc::new(ShardStats::default());
    // A fact about the shard, said once: the status endpoint reads it off
    // the stats, the only thing that thread may touch.
    if cfg.agents_only {
        ShardStats::raise(&stats.agents_only);
    }

    let mut transport = wtransport::config::QuicTransportConfig::default();
    // NETCODE.md §2.2: bound worst-case queued staleness; snapshots
    // replace, never accumulate.
    transport.datagram_send_buffer_size(64 * 1024);
    transport
        .max_concurrent_bidi_streams(QUIC_MAX_BIDI_STREAMS.into())
        .max_concurrent_uni_streams(QUIC_MAX_UNI_STREAMS.into())
        .stream_receive_window(QUIC_STREAM_RWND.into())
        .receive_window(QUIC_CONN_RWND.into())
        .send_window(QUIC_SEND_WINDOW)
        .datagram_receive_buffer_size(Some(QUIC_DATAGRAM_RBUF));
    // Congestion control, selectable because §2.1 names it as the real risk
    // and §2.2 promised an A/B path that had never been built. CUBIC halves
    // cwnd on loss and a collapsed window at 100 ms RTT can fall under our
    // 30 Hz send rate; BBR is loss-insensitive and is labelled experimental
    // in quinn, which is exactly why this is a shard flag and not a default.
    // `net_congestion_events` is the reading that makes the A/B mean
    // something.
    if cfg.congestion == crate::config::Congestion::Bbr {
        transport.congestion_controller_factory(Arc::new(
            wtransport::quinn::congestion::BbrConfig::default(),
        ));
    }
    let socket = bind_udp(cfg.bind, &stats).map_err(|e| format!("bind {}: {e}", cfg.bind))?;
    let server_config = ServerConfig::builder()
        .with_bind_socket(socket)
        .with_custom_transport(identity, transport)
        .max_idle_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("idle timeout: {e}"))?
        .keep_alive_interval(Some(Duration::from_secs(10)))
        .build();

    let endpoint =
        Endpoint::server(server_config).map_err(|e| format!("endpoint bind {}: {e}", cfg.bind))?;
    let local_addr = endpoint
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut pending = PendingStartup(Some(shutdown.clone()));
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    // Players, then spectator seats (`slot::MAX_CONNS`). Started past every
    // id the saved world still holds, before anyone can connect.
    let slots = Arc::new(SlotTable::new(MAX_CONNS));
    slots.start_past(&world_boot.id_floors);
    tokio::spawn(wake_relay(slots.clone()));

    let (ctrl_tx, ctrl_rx) = RingBuffer::<Connect>::new(CTRL_RING_CAP);
    let (grave_tx, grave_rx) = RingBuffer::<Link>::new(GRAVEYARD_RING_CAP);
    // The save path, two hops and one direction: the sim takes records and
    // knows no keys; the accept loop owns the key table and the index; the
    // storage thread owns the file and decides nothing. Each hop is a bounded
    // SPSC ring, so nothing on the way to a disk can block a tick.
    let (save_tx, save_rx) = RingBuffer::<SaveMsg>::new(SAVE_RING_CAP);
    let (write_tx, write_rx) = RingBuffer::<WriteMsg>::new(SAVE_RING_CAP);
    // The world path, one hop and two directions: full buffers down to the
    // store thread, emptied ones back. Depth 2 because that is what a
    // double buffer is — one being written while one is being filled — and
    // a deeper queue would only let stale worlds pile up behind a slow disk
    // when the correct answer to a slow disk is to skip a save.
    let (world_tx, world_rx) = RingBuffer::<WorldMsg>::new(WORLD_RING_CAP);
    let (world_done_tx, world_done_rx) = RingBuffer::<WorldDone>::new(WORLD_RING_CAP);
    // The side files, sim → storage thread: the gift ledger and the teams
    // (`slot::FileMsg`), each message a whole small file. On the store
    // thread because it is the one the binary waits on at shutdown: the last
    // gift lands before exit, and the teams before a wipe archives them.
    let (file_tx, file_rx) = RingBuffer::<FileMsg>::new(FILE_RING_CAP);
    // The admin path, sim → accept, one direction: a kick needs a socket
    // and the sim thread holds none (`admin.rs`'s split). Shallow because
    // an admin is a person typing — `CTRL_RING_CAP` is the same size for
    // the same reason — and a full ring refuses the act out loud rather
    // than queueing a kick nobody remembers ordering.
    let (admin_tx, admin_rx) = RingBuffer::<crate::admin::AdminAct>::new(CTRL_RING_CAP);
    // And back, accept → sim: how each act went, for the admin's screen.
    // One answer per act, so the same depth; a full ring drops the line.
    let (answer_tx, answer_rx) = RingBuffer::<crate::admin::AdminReply>::new(CTRL_RING_CAP);
    // The skins path, accept → sim, one direction: what the platform said a
    // player owns (`skins.rs`), read off the sim thread and entered as a
    // command. One read in flight per slot, so a slot's worth of depth is a
    // ring that cannot fill; a full one drops the answer and counts it.
    let (skins_tx, skins_rx) = RingBuffer::<crate::slot::SkinsMsg>::new(MAX_PLAYERS);
    // The item store's prices, same direction (`skins::prices_of`). One read
    // in flight at a time, so two deep cannot fill; a full ring drops the
    // answer and the next sweep brings it again.
    let (prices_tx, prices_rx) = RingBuffer::<crate::slot::SkinPricesMsg>::new(2);
    // Names and pictures (`faces.rs`), same direction: one read per join,
    // so a slot's worth of depth cannot fill.
    let (faces_tx, faces_rx) = RingBuffer::<crate::slot::FaceMsg>::new(MAX_PLAYERS);
    let skin_content = Arc::new(tables.skins);
    let crate::worldfile::WorldBoot {
        file: world_file,
        idents: world_idents,
        blob: world_blob,
        interval_ticks: world_interval,
        clamp: world_clamp,
        wipe,
        id_floors: _,
    } = world_boot;

    // The anomaly log, opened before the sim thread that writes to it —
    // and a failure to open is a **boot** failure, not a silent downgrade:
    // an operator who configured a log and got none would be reading an
    // empty file after the incident they wanted it for (`anomaly.rs`).
    let log = match cfg.anomaly_file.as_deref() {
        Some(path) => {
            let (sink, _thread) = crate::anomaly::spawn(std::path::Path::new(path), stats.clone())
                .map_err(|e| format!("anomaly log `{path}`: {e}"))?;
            // The handle is deliberately dropped: the thread ends when the
            // sim thread's `Sink` drops at shutdown, which is the signal
            // that every record has been written — joining here would mean
            // waiting for it before the shard has started.
            sink
        }
        None => crate::anomaly::Sink::off(),
    };
    // The ban list, read before anyone can knock: a file that does not
    // parse is a boot failure, never a shard that forgot whom it banned.
    let bans = match cfg.ban_file.as_deref() {
        Some(path) => crate::admin::Bans::load(std::path::Path::new(path))
            .map_err(|e| format!("ban file `{path}`: {e}"))?,
        None => crate::admin::Bans::new(),
    };
    // The sim's copy, for the companion (`ShardCore::install_bans`).
    let banned: Vec<PlayerKey> = bans.keys().to_vec();

    // The companion (`companion.rs`), only where the operator said. A bind
    // failure refuses the boot, as the status endpoint's does.
    let companion = match cfg.companion_addr {
        Some(addr) => {
            let (c, bound) = crate::companion::spawn(
                addr,
                cfg.seed,
                cfg.domain.clone(),
                cfg.siwe_accepts().to_vec(),
            )
            .map_err(|e| format!("companion bind {addr}: {e}"))?;
            println!("companion on http://{bound}/");
            c
        }
        None => crate::companion::Companion::off(),
    };

    // Captions (`scribe.rs`), only where the operator said. A shard told to
    // caption without a transcriber's key refuses the boot rather than
    // captioning nothing.
    let scribe = if cfg.captions {
        let scribe_cfg = crate::scribe::ScribeConfig::from_env(
            cfg.transcript_file.as_deref().map(std::path::PathBuf::from),
            cfg.transcript_days,
        )?;
        println!(
            "captions on: {} transcribes, {} translates",
            scribe_cfg.voice.stt_model, scribe_cfg.translate_model
        );
        crate::scribe::spawn(scribe_cfg, stats.clone())?
    } else {
        crate::scribe::Scribe::off()
    };

    {
        let stats = stats.clone();
        let shutdown = shutdown.clone();
        let slots = slots.clone();
        let seed = cfg.seed;
        let dev_spawn = cfg.dev_spawn;
        let dev_env = cfg.dev_env;
        let dev_arc_hour = cfg.dev_arc_hour;
        let admins = cfg.admins.clone();
        let team_max = cfg.team_max;
        let standings_base = cfg
            .world_file
            .as_deref()
            .or(cfg.save_file.as_deref())
            .map(std::path::PathBuf::from);
        let prizes = cfg.prizes.clone();
        let exits = cfg.exits;
        let mut motd = cfg.motd.clone();
        // Captions say so at every join: what is transcribed, why, for how
        // long, and how to choose the language.
        if scribe.on() {
            motd.push(match cfg.transcript_file {
                Some(_) => format!(
                    "Voice and chat here are transcribed for captions, translation and \
                     reports, kept {} days.",
                    cfg.transcript_days
                ),
                None => "Voice and chat here are transcribed for captions and translation.".into(),
            });
            motd.push(
                "Captions come in your system's language: /lang <code> changes it, /lang off \
                 stops translating."
                    .into(),
            );
        }
        // The trust log's tap rides the save boot artifact (`store::Saves`).
        let trust = saves.trust;
        std::thread::Builder::new()
            .name("sim".into())
            .spawn(move || {
                sim_thread(
                    seed,
                    dev_spawn,
                    dev_env,
                    dev_arc_hour,
                    tables,
                    world_blob,
                    world_idents,
                    world_interval,
                    world_clamp,
                    wipe,
                    standings_base,
                    prizes,
                    exits,
                    motd,
                    ctrl_rx,
                    skins_rx,
                    prices_rx,
                    faces_rx,
                    grave_tx,
                    save_tx,
                    world_tx,
                    world_done_rx,
                    file_tx,
                    admin_tx,
                    answer_rx,
                    log,
                    admins,
                    banned,
                    team_max,
                    trust,
                    companion,
                    scribe,
                    slots,
                    stats,
                    shutdown,
                    ready_tx,
                )
            })
            .map_err(|e| format!("sim thread spawn: {e}"))?;
    }

    {
        // Blocking file I/O, on a thread of its own: DESIGN.md §8's "the
        // storage thread serializes". Not a tokio task, because a `sync_data`
        // inside the runtime would stall whatever else that worker owed —
        // including an accept.
        let stats = stats.clone();
        let file = saves.file;
        std::thread::Builder::new()
            .name("store".into())
            .spawn(move || {
                store_thread(
                    file,
                    write_rx,
                    world_file,
                    world_rx,
                    world_done_tx,
                    file_rx,
                    stats,
                )
            })
            .map_err(|e| format!("store thread spawn: {e}"))?;
    }

    // The control ring bounds arrivals between ticks, not arrivals during
    // world construction. Do not admit clients until its consumer is ready.
    let blight_salt = ready_rx
        .await
        .map_err(|_| "sim thread stopped during initialization".to_owned())?;

    tokio::spawn(accept_loop(
        endpoint,
        ShardFacts {
            seed: cfg.seed,
            blight_salt,
            // A shard running a dev override is a dev shard, and says so
            // in every welcome — that bit is the client's only dev gate.
            // A shard with a fake network is a dev shard as surely as one
            // with a pinned spawn — the client's dev affordances key on this.
            dev: cfg.dev_spawn.is_some() || cfg.netsim.is_some() || cfg.dev_arc_hour.is_some(),
            require_auth: cfg.require_auth,
            siwe_domains: cfg.siwe_accepts().into(),
            pass_key: cfg.elo_account_key,
            entitle: cfg.entitle.clone(),
            skins: cfg.skins.clone(),
            faces: cfg.faces.clone(),
            skin_content,
            min_client: cfg.min_client,
            netsim: cfg.netsim,
            spectate: cfg.spectate,
            agent_seats: cfg.agent_seats,
            agents_only: cfg.agents_only,
            own_sources: cfg.own_sources.clone(),
            minted: cfg
                .world_file
                .as_deref()
                .or(cfg.save_file.as_deref())
                .map(|w| crate::mints::minted_path(std::path::Path::new(w))),
        },
        bans,
        ctrl_tx,
        skins_tx,
        prices_tx,
        faces_tx,
        grave_rx,
        save_rx,
        write_tx,
        admin_rx,
        answer_tx,
        saves.store,
        slots,
        stats.clone(),
        shutdown.clone(),
    ));

    pending.0 = None;
    Ok(ShardHandle {
        local_addr,
        cert_hash,
        stats,
        shutdown,
    })
}

// ---------------------------------------------------------------------------
// Accept side
// ---------------------------------------------------------------------------

/// What every joiner is told about the shard itself: the seed its whole
/// world derives from, and whether this is a dev shard — the bit the
/// client gates its dev affordances on.
#[derive(Clone)]
struct ShardFacts {
    seed: u64,
    /// This wipe's blight salt, as the sim thread drew or loaded it.
    blight_salt: u32,
    dev: bool,
    /// `shard.toml require_auth`. See `config.rs` for why it is a knob.
    require_auth: bool,
    /// The SIWE domains: every name this shard answers to in the message
    /// players sign — `domain` alone unless `siwe_domains` lists more
    /// (`ShardConfig::siwe_accepts`). Shared, because every handshake task
    /// gets a handle and none of them needs its own copy.
    siwe_domains: Arc<[String]>,
    /// elo's account key (`shard.toml elo_account_pubkey`): with it, an Elo
    /// Pros pass is a sign-in (`auth::verify_pass`); without, none is.
    pass_key: Option<[u8; 32]>,
    /// The ticket door (`entitle.rs`). `Config::off()` — the default — checks
    /// nothing, which is what every test and every community shard runs.
    entitle: crate::entitle::Config,
    /// Where skin ownership is read (`skins.rs`), and the baked rows an
    /// answer's catalog ids map onto.
    skins: crate::skins::Config,
    /// Where names and pictures are read (`faces.rs`).
    faces: crate::faces::Config,
    skin_content: Arc<sim_core::skin::SkinContent>,
    /// `shard.toml min_client`, packed. 0 — the default — admits every client
    /// whose `PROTO_VER` already matched, which is every client that could
    /// have got this far.
    min_client: u32,
    /// The dev fake-network knob (`config.rs`), applied per connection at
    /// the two datagram tasks. `None` on every shipping shard.
    netsim: Option<crate::config::NetSim>,
    /// The spectator door (`config.rs`, `NETCODE.md` §2.3). Shut by default.
    spectate: crate::config::Spectate,
    /// `shard.toml agent_seats`: the most declared agents seated at once.
    /// 0 — the default — caps nothing.
    agent_seats: u32,
    /// `shard.toml agents_only`: declared agents and spectators only.
    agents_only: bool,
    /// `shard.toml own_sources`: this host's public addresses, which no
    /// per-source cap counts.
    own_sources: Vec<std::net::IpAddr>,
    /// `<world>.minted`, the minter's log of the tokens it sent
    /// (`mints.rs`): a wallet named there is read again. `None` with no
    /// world file.
    minted: Option<std::path::PathBuf>,
}

/// Is there a seat for this joiner under `agent_seats`? People always pass;
/// an agent passes while the connected-agent gauge is below the cap. The
/// gauge is the sim's, mirrored each tick, so a burst of agents inside one
/// tick can overshoot by that burst — the cap keeps seats for people, it is
/// not a hard count.
fn agent_seat_free(agent_seats: u32, agents_now: u64, is_agent: bool) -> bool {
    !is_agent || agent_seats == 0 || agents_now < u64::from(agent_seats)
}

/// Does this shard seat this kind of session at all? Every shard seats
/// everyone, except an agent-only one (`agents_only`), which seats declared
/// agents and spectators and nobody else. The declaration is the hello's
/// `HELLO_AGENT`: self-declared, as the `(agent)` mark is.
fn admits_kind(agents_only: bool, is_agent: bool, is_spectator: bool) -> bool {
    !agents_only || is_agent || is_spectator
}

/// What a handshake task hands back once the client said a valid hello.
/// The recv half stays: after the hello it is the C→S action lane.
struct Handshaken {
    connection: Connection,
    send: SendStream,
    recv: RecvStream,
    /// Who the admission seam said this is, if it could say. `None` ⇒ a
    /// guest: admitted (on a shard that takes guests) and remembered by
    /// nobody, because there is no stable string to file them under.
    key: Option<PlayerKey>,
    /// The Elo Pros account a pass naming `key`'s wallet came from
    /// (`Connect::alias`).
    alias: Option<PlayerKey>,
    /// The PROVEN address behind `key`, or `Address::GUEST` — the form a
    /// spectator names its target in and the `Watch` message reports.
    address: protocol::Address,
    /// What the session declared about itself (v73): agent, consent to be
    /// watched, a spectator's target, a display name.
    hello: protocol::Hello,
}

/// The accept loop's own id→key table, one entry per connection slot.
///
/// **This is where identity meets the world's ids, and it is deliberately
/// the only place.** The sim hands back saves labelled with a player id;
/// this turns an id into the key the record is filed under. It stores the id
/// alongside the key and matches it exactly, because a slot is reused: a
/// save for the previous tenant paired against the current one's key would
/// hand somebody else's inventory to whoever claimed the slot next. Ordering
/// makes that unreachable (the sim pushes a leave's record before it frees
/// the slot, and this loop drains the ring before it installs anyone), and
/// the id check is what makes it unreachable *by construction* rather than
/// by argument.
///
/// **`conn` is the roster sweep's only reach into a live connection.** A
/// player who sells their copy mid-session is not doing anything the sim can
/// see, so the kick cannot come from the sim thread — it comes from here,
/// and this is the handle it closes. Held as an `Option` because a guest
/// slot has no wallet to sweep and a freed slot has nothing at all.
///
/// No longer `Copy`: a `Connection` is refcounted and cloning it is a
/// decision, not a memcpy. The two read sites take `.key` and `.id` by value
/// as before.
#[derive(Clone, Default)]
struct KeySlot {
    key: Option<PlayerKey>,
    id: u32,
    conn: Option<Connection>,
    /// The slot generation this tenant claimed — what a spectator seat is
    /// bound to, because a reused slot has a new one (`SeatSlot`).
    generation: u32,
    /// The proven address, or `GUEST` (`Handshaken::address`).
    address: protocol::Address,
    /// Declared `HELLO_AGENT` (v73): consent to be watched, fed undelayed.
    agent: bool,
    /// Declared `HELLO_WATCHABLE`: a human's opt-in, fed only behind the
    /// shard's human delay.
    watchable: bool,
    /// The self-declared display name its watchers see.
    name: protocol::Name,
    /// The address it joined from, as `SourceGate` counts it, or `None` for
    /// the shard's own host (`SEATS_PER_SOURCE`).
    source: Option<std::net::IpAddr>,
}

/// The accept loop's half of a spectator seat (`NETCODE.md` §2.3): the
/// connection it may have to close, and exactly which tenant it watches.
/// Seat `i` is connection slot `MAX_PLAYERS + i`.
#[derive(Clone, Default)]
struct SeatSlot {
    /// Granted and not yet swept. A flag of its own rather than
    /// `conn.is_some()`, so the door (`pick_target`) is a pure function a
    /// test can drive without a socket.
    occupied: bool,
    conn: Option<Connection>,
    /// This seat's own claim generation, so a sweep never closes the next
    /// tenant of the seat in the previous one's name.
    generation: u32,
    /// The watched player's connection slot and the generation it held when
    /// the seat was granted. The target is gone the moment either changes.
    target: usize,
    target_gen: u32,
    /// The address it watches from, as `SourceGate` counts it, or `None`
    /// for the shard's own host (`WATCH_SEATS_PER_SOURCE`).
    source: Option<std::net::IpAddr>,
}

/// Wallets the per-join platform reads remember (`PlatformReads`): a full
/// shard and as many again who just left. Past it the stalest is forgotten,
/// which costs that wallet one more read and nothing else.
const READ_MEMORY: usize = 2 * MAX_PLAYERS;
/// Skin and face reads out at once. The rest wait on a task, never on the
/// accept loop and never on a blocking thread: the pool is shared with the
/// ticket door's join read.
const READ_PERMITS: usize = 32;

/// How often the minter's log is looked at (`mints.rs`): a token sent is
/// read into its buyer's skins within this, and a few seconds more.
const MINTED_POLL: Duration = Duration::from_secs(5);
/// When a minted wallet is read again, after its line is seen. The
/// platform lists tokens off an index that can trail the chain, so one read
/// is not enough; three reads per sale is what the metered route spends.
const REREAD_AFTER: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(30),
    Duration::from_secs(120),
];
/// Reads waiting to come due, oldest dropped past this.
const REREADS_MAX: usize = 96;

type SkinsAnswer = (usize, u32, Option<PlayerKey>, crate::skins::Owned);
type FaceAnswer = (usize, u32, PlayerKey, crate::faces::Face);

/// One wallet's last platform read.
#[derive(Clone, Copy)]
struct LastRead<A> {
    key: PlayerKey,
    /// When it was last asked; the cooldown runs from here.
    at: Instant,
    /// That read has not answered yet.
    out: bool,
    /// The last answer that knew something, and when ITS read was asked.
    /// Told again only while younger than the cooldown, so a skin sold since
    /// is never lent back for longer than a client refresh could see it.
    known: Option<(A, Instant)>,
    /// A join came inside the cooldown and was told the remembered answer:
    /// read again once the cooldown has run ([`PlatformReads::owed`]), so a
    /// skin bought in that window still arrives without the player asking.
    owed: bool,
}

/// Skin-ownership and face reads, remembered **by wallet**, so a reconnect
/// loop costs the platform one read per cooldown rather than one per join.
/// Bounded (`READ_MEMORY`) and clock-free: every instant is handed in.
struct PlatformReads<A> {
    recent: Vec<LastRead<A>>,
    cooldown: Duration,
}

impl<A: Copy> PlatformReads<A> {
    fn new(cooldown: Duration) -> Self {
        Self {
            recent: Vec::with_capacity(READ_MEMORY),
            cooldown,
        }
    }

    /// The answer to state again with `key`'s join: a known one younger than
    /// the cooldown.
    fn known(&self, key: &PlayerKey, now: Instant) -> Option<A> {
        let e = self.recent.iter().find(|e| e.key == *key)?;
        e.known
            .filter(|&(_, at)| now.duration_since(at) < self.cooldown)
            .map(|(a, _)| a)
    }

    /// Should `key` be read now? Never while a read is out or inside the
    /// cooldown. `door` is a join's ask: refused, it owes a read for when
    /// the cooldown has run. A client's ask is simply ignored, as before.
    fn ask(&mut self, key: &PlayerKey, door: bool, now: Instant) -> bool {
        if let Some(e) = self.recent.iter_mut().find(|e| e.key == *key) {
            if e.out || now.duration_since(e.at) < self.cooldown {
                e.owed |= door;
                return false;
            }
            e.at = now;
            e.out = true;
            e.owed = false;
            return true;
        }
        if self.recent.len() >= READ_MEMORY {
            // The stalest idle one first: forgetting a wallet with a read
            // out or owed would lose the read the join was promised.
            let busy = |e: &LastRead<A>| e.out || e.owed;
            if let Some(i) =
                (0..self.recent.len()).min_by_key(|&i| (busy(&self.recent[i]), self.recent[i].at))
            {
                self.recent.swap_remove(i);
            }
        }
        self.recent.push(LastRead {
            key: *key,
            at: now,
            out: true,
            known: None,
            owed: false,
        });
        true
    }

    /// A read about `key` came back; `None` is "could not look", which
    /// changes nothing remembered (the sim keeps its set too).
    fn done(&mut self, key: &PlayerKey, answer: Option<A>) {
        if let Some(e) = self.recent.iter_mut().find(|e| e.key == *key) {
            e.out = false;
            if let Some(a) = answer {
                e.known = Some((a, e.at));
            }
        }
    }

    /// A wallet owed a read whose cooldown has run, handed out once.
    fn owed(&mut self, now: Instant) -> Option<PlayerKey> {
        let cooldown = self.cooldown;
        let e = self
            .recent
            .iter_mut()
            .find(|e| e.owed && !e.out && now.duration_since(e.at) >= cooldown)?;
        e.owed = false;
        Some(e.key)
    }
}

/// One blocking platform read behind `gate` (`READ_PERMITS`). A read that
/// died answers `failed`, so an answer always comes back and a wallet's read
/// is never left out for ever.
fn spawn_read<M: Send + 'static>(
    gate: &Arc<tokio::sync::Semaphore>,
    tx: &tokio::sync::mpsc::Sender<M>,
    failed: M,
    read: impl FnOnce() -> M + Send + 'static,
) {
    let gate = gate.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let answer = match gate.acquire_owned().await {
            // `_permit`, not `_`: held until the read is done.
            Ok(_permit) => tokio::task::spawn_blocking(read).await.unwrap_or(failed),
            Err(_) => failed,
        };
        let _ = tx.send(answer).await;
    });
}

/// The skin-ownership read for tenant `gen` of `slot` (`skins.rs`). The
/// answer carries the wallet, so it lands on whoever holds it by then
/// ([`landing`]). A guest (`None`) owns what the config says; nothing is
/// remembered for one.
fn read_skins(
    slot: usize,
    gen: u32,
    key: Option<PlayerKey>,
    facts: &ShardFacts,
    gate: &Arc<tokio::sync::Semaphore>,
    tx: &tokio::sync::mpsc::Sender<SkinsAnswer>,
) {
    let cfg = facts.skins.clone();
    let sc = facts.skin_content.clone();
    let failed = (slot, gen, key, crate::skins::Owned::Unknown);
    spawn_read(gate, tx, failed, move || {
        // The key IS the wallet (`auth::key_of`), as at the ticket door.
        let wallet = key
            .as_ref()
            .and_then(|k| std::str::from_utf8(k.as_bytes()).ok());
        (slot, gen, key, crate::skins::owned_of(&cfg, wallet, &sc))
    });
}

/// The name-and-picture read for tenant `gen` of `slot` (`faces.rs`).
fn read_face(
    slot: usize,
    gen: u32,
    key: PlayerKey,
    facts: &ShardFacts,
    gate: &Arc<tokio::sync::Semaphore>,
    tx: &tokio::sync::mpsc::Sender<FaceAnswer>,
) {
    let cfg = facts.faces.clone();
    let failed = (slot, gen, key, crate::faces::Face::Unknown);
    spawn_read(gate, tx, failed, move || {
        // The key IS the wallet, lowercase; a non-utf8 key reads as none.
        let wallet = std::str::from_utf8(key.as_bytes()).unwrap_or_default();
        (slot, gen, key, crate::faces::face_of(&cfg, wallet))
    });
}

/// Where a platform answer about tenant `gen` of `slot` lands: that tenant
/// while it is live, otherwise (for a wallet) whoever holds that wallet live
/// now, because a reconnect while the read was out is the same person. An
/// answer about a guest who left lands nowhere, and a stranger who took the
/// slot is never told.
fn landing(
    slot: usize,
    gen: u32,
    key: Option<&PlayerKey>,
    keys: &[KeySlot; MAX_PLAYERS],
    slots: &SlotTable,
) -> Option<usize> {
    if slot < MAX_PLAYERS {
        let w = slots.load(slot);
        if generation_of(w) == gen && state_of(w) == SLOT_LIVE {
            return Some(slot);
        }
    }
    key.and_then(|k| live_seat(k, keys, slots))
}

/// The live player slot holding wallet `key`, if any ([`seated`], live only).
fn live_seat(key: &PlayerKey, keys: &[KeySlot; MAX_PLAYERS], slots: &SlotTable) -> Option<usize> {
    seated(key, keys, slots).filter(|&s| state_of(slots.load(s)) == SLOT_LIVE)
}

#[allow(clippy::too_many_arguments)]
async fn accept_loop(
    endpoint: Endpoint<Server>,
    facts: ShardFacts,
    mut bans: crate::admin::Bans,
    mut ctrl_tx: rtrb::Producer<Connect>,
    mut skins_tx: rtrb::Producer<crate::slot::SkinsMsg>,
    mut prices_tx: rtrb::Producer<crate::slot::SkinPricesMsg>,
    mut faces_tx: rtrb::Producer<crate::slot::FaceMsg>,
    mut grave_rx: rtrb::Consumer<Link>,
    mut save_rx: rtrb::Consumer<SaveMsg>,
    mut write_tx: rtrb::Producer<WriteMsg>,
    mut admin_rx: rtrb::Consumer<crate::admin::AdminAct>,
    mut answer_tx: rtrb::Producer<crate::admin::AdminReply>,
    mut store: SaveStore,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
) {
    // Net-side plumbing between handshake tasks and this loop; the sim
    // thread never touches it (L3 is about the sim thread, not tokio).
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<Handshaken>(MAX_PLAYERS);
    // `[T; N]` initialiser syntax needs `Copy` and `KeySlot` stopped being
    // `Copy` when it started holding a `Connection`. Built on the heap and
    // converted, which is also the shape `boxed_array` uses next door for a
    // different reason (wasm's shadow stack) — here it is simply the way to
    // fill a fixed array with a clonable value.
    let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
    // The spectator seats' connections and their targets (`SeatSlot`).
    let mut seats: [SeatSlot; MAX_SPECTATORS] = std::array::from_fn(|_| SeatSlot::default());
    let mut sweep = tokio::time::interval(Duration::from_millis(100));
    // ---- the roster sweep -------------------------------------------------
    //
    // A join check alone is a door with no lock behind it: a player can sell
    // the ticket and keep playing. This re-asks about everybody on an
    // interval (`entitle::DEFAULT_SWEEP`), and the interval IS the security
    // property — it is how long a sold copy can linger, which is a posted
    // knob rather than a hole.
    //
    // Results come back through a channel rather than being awaited inline,
    // because this loop is also the accept path: a blocked sweep would be a
    // shard that stops taking players while elo is slow. `in_flight` is the
    // no-stacking rule — one round at a time, whatever the origin does, the
    // same refusal `status.rs`'s poller makes.
    // Handshakes started and not yet finished. The admission gate's only
    // input, and the reason it is a count rather than a rate: a rate needs a
    // clock and a window, and what actually protects the shard is "how much
    // unfinished crypto am I holding right now".
    let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let bound = endpoint.local_addr().ok().map(|a| a.ip());
    let sources = Arc::new(SourceGate::new(bound, &facts.own_sources));
    // The sign-ins waiting on a person.
    let signing = Arc::new(SignWait::new());
    // The work a signature starts — its check, the ticket read — a few at a
    // time ([`POST_SIGN_PERMITS`]).
    let post_sign = Arc::new(tokio::sync::Semaphore::new(POST_SIGN_PERMITS));
    let (kick_tx, mut kick_rx) = tokio::sync::mpsc::channel::<Vec<(usize, u32)>>(1);
    let mut entitle_sweep = tokio::time::interval(facts.entitle.sweep);
    let mut sweep_in_flight = false;
    // ---- skin ownership (`skins.rs`) ------------------------------------
    //
    // Asked at the door and when a client says it bought something, never
    // on a timer. Requests come from the action readers, answers from
    // blocking reads, both through channels so this loop never waits on
    // the platform — the ticket sweep's shape. Remembered per wallet
    // (`PlatformReads`): a join inside the cooldown is told the last answer
    // and read again when the cooldown has run, so a reconnect loop costs
    // the platform nothing extra.
    let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::channel::<(usize, u32)>(MAX_PLAYERS);
    let (owned_tx, mut owned_rx) = tokio::sync::mpsc::channel::<SkinsAnswer>(MAX_PLAYERS);
    let mut skin_reads =
        PlatformReads::<sim_core::skin::SkinSet>::new(crate::skins::REFRESH_COOLDOWN);
    // Names and pictures (`faces.rs`): once per join and cooldown, answered
    // on a channel for the same reason — a slow platform never slows the door.
    let (face_tx, mut face_rx) = tokio::sync::mpsc::channel::<FaceAnswer>(MAX_PLAYERS);
    let mut face_reads =
        PlatformReads::<(protocol::Name, u32)>::new(crate::faces::REFRESH_COOLDOWN);
    let faces_armed = facts.faces.origin.is_some();
    // Every skin and face read waits its turn here (`READ_PERMITS`).
    let read_gate = Arc::new(tokio::sync::Semaphore::new(READ_PERMITS));
    // The item store's prices: at boot, then every `PRICES_EVERY`. Only a
    // shard that reads ownership from the platform reads prices from it.
    let prices_armed = facts.skins.origin.is_some() && !facts.skins.all;
    let mut prices_sweep = tokio::time::interval(crate::skins::PRICES_EVERY);
    prices_sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let (prices_done_tx, mut prices_done_rx) =
        tokio::sync::mpsc::channel::<crate::skins::Prices>(1);
    let mut prices_in_flight = false;
    // The minter's log (`mints.rs`): looked at every `MINTED_POLL`, read
    // when it grew, and each wallet it names read again now and twice more,
    // because the platform's token index can trail the chain.
    let mut minted_poll = tokio::time::interval(MINTED_POLL);
    minted_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let (minted_tx, mut minted_rx) = tokio::sync::mpsc::channel::<(u64, String)>(1);
    let mut minted_in_flight = false;
    let (mut minted_len, mut minted_seen) = (0u64, None::<usize>);
    let mut rereads: Vec<(PlayerKey, Instant)> = Vec::with_capacity(REREADS_MAX);
    let mut parked: Vec<Parked> = Vec::with_capacity(PARK_MAX);
    let mut park_poll = tokio::time::interval(PARK_POLL);
    park_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The door's tail, for a fresh arrival and a parked one alike.
    macro_rules! admit {
        ($done:expr) => {{
            let mut done = $done;
            // Before the claim, never after: a record for the slot's
            // previous tenant has to be filed under the key it was
            // written for, and installing first would overwrite that key.
            drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
            // An account that linked a wallet: its record is the wallet's
            // now, unless the wallet has its own (`adopt_account_save`).
            if let (Some(to), Some(from)) = (done.key, done.alias) {
                let free = seated(&from, &keys, &slots).is_none();
                if !(free && adopt_account_save(&mut store, &mut write_tx, &from, &to, &stats)) {
                    done.alias = None;
                }
            }
            if done.hello.is_spectator() {
                let source = sources.counted(done.connection.remote_address().ip());
                install_spectator(
                    done,
                    source,
                    &facts,
                    &mut ctrl_tx,
                    &keys,
                    &mut seats,
                    &slots,
                    &stats,
                );
            } else {
                // A wallet read inside the cooldown is not read again: what
                // it said rides the join itself (`Connect::said`). The same
                // `now` for `known` and `ask`, so "told" and "not read"
                // always agree.
                let now = Instant::now();
                // A guest owns what the config says, which asks nobody
                // (`skins::owned_of` with no wallet): said with the join too.
                let said = crate::slot::Said {
                    skins: match done.key {
                        Some(k) => skin_reads.known(&k, now),
                        None => {
                            match crate::skins::owned_of(&facts.skins, None, &facts.skin_content) {
                                crate::skins::Owned::Known(set) => Some(set),
                                crate::skins::Owned::Unknown => None,
                            }
                        }
                    },
                    face: done
                        .key
                        .filter(|_| faces_armed)
                        .and_then(|k| face_reads.known(&k, now)),
                };
                let source = sources.counted(done.connection.remote_address().ip());
                if let Some((slot, gen)) = install(
                    done,
                    source,
                    said,
                    &facts,
                    &mut ctrl_tx,
                    &refresh_tx,
                    &mut keys,
                    &store,
                    &slots,
                    &stats,
                ) {
                    if said.skins.is_some() && keys[slot].key.is_some() {
                        ShardStats::bump(&stats.skins_replayed);
                    }
                    if said.face.is_some() {
                        ShardStats::bump(&stats.faces_replayed);
                    }
                    // The door's reads: what this player owns, before they
                    // reach a bench, and what they are called.
                    if let Some(k) = keys[slot].key {
                        if skin_reads.ask(&k, true, now) {
                            read_skins(slot, gen, Some(k), &facts, &read_gate, &owned_tx);
                        }
                        if faces_armed && face_reads.ask(&k, true, now) {
                            read_face(slot, gen, k, &facts, &read_gate, &face_tx);
                        }
                    }
                }
            }
        }};
    }
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                // Admission, before any crypto (NETCODE.md §2.2). Every
                // refusal we had before this fired *after* a completed
                // handshake — QUIC, TLS, CONNECT, SIWE, and an entitlement
                // round trip to elo — so a full shard was an amplifier and
                // a spoofed source cost us more than it cost the attacker.
                //
                // Order matters: refuse is the harder answer and is checked
                // first, so a flood past the hard cap is not merely retried
                // forever.
                let busy = in_flight.load(Ordering::Relaxed);
                if busy >= ADMIT_REFUSE_AT {
                    ShardStats::bump(&stats.admit_refused);
                    incoming.refuse();
                    continue;
                }
                // `retry()` PANICS if the address is already validated —
                // wtransport's own doc comment says so, and it is the reason
                // this reads as a guard rather than an optimisation. A
                // validated address has already proved it can receive at the
                // address it claims, which is the entire thing a Retry buys.
                if busy >= ADMIT_RETRY_AT && !incoming.remote_address_validated() {
                    ShardStats::bump(&stats.admit_retried);
                    incoming.retry();
                    continue;
                }
                // An address QUIC has validated is charged against its
                // source here, before any crypto (`ADMIT_PER_SOURCE`).
                let permit = if incoming.remote_address_validated() {
                    let Some(p) = SourceGate::take(&sources, incoming.remote_address().ip()) else {
                        ShardStats::bump(&stats.refused_per_source);
                        incoming.refuse();
                        continue;
                    };
                    Some(p)
                } else {
                    None
                };
                in_flight.fetch_add(1, Ordering::Relaxed);
                let stats = stats.clone();
                let done_tx = done_tx.clone();
                let in_flight = in_flight.clone();
                let sources = sources.clone();
                tokio::spawn(handshake_task(
                    incoming,
                    done_tx,
                    stats,
                    facts.require_auth,
                    facts.siwe_domains.clone(),
                    facts.pass_key,
                    facts.entitle.clone(),
                    facts.min_client,
                    facts.spectate.open(),
                    facts.agents_only,
                    in_flight,
                    sources,
                    permit,
                    signing.clone(),
                    post_sign.clone(),
                ));
            }
            Some(done) = done_rx.recv() => {
                // A banned wallet stops here, before it claims anything,
                // refused with the code its ban closed it on — so the
                // screen says what it said then. (A watcher is anonymous:
                // its auth is never verified, so it carries no key a ban
                // could match.)
                if done.key.as_ref().is_some_and(|k| bans.contains(k)) {
                    ShardStats::bump(&stats.refused_banned);
                    spawn_refusal(done.connection, done.send, protocol::REFUSE_ADMIN);
                    continue;
                }
                // One wallet, one body. A second session for a key whose body
                // is still awake would miss the sleeper (there is none) and
                // be seated from the store's record: two bodies, one
                // inventory. So the session already in is closed
                // (`REFUSE_IN_USE`) and the newcomer waits, unclaimed, until
                // the sim has put that body to sleep — then it is seated on
                // it. Never in this pass: the old leave has not reached the
                // sim yet. A second newcomer for the same key, or one past
                // the park's room, is refused outright.
                if !done.hello.is_spectator() {
                    // A wallet whose account (`Handshaken::alias`) is still
                    // in is the same player: that session is closed too, so
                    // its body is asleep for the wallet to take.
                    let seat = done.key.and_then(|k| seated(&k, &keys, &slots)).or_else(|| {
                        done.key.and(done.alias).and_then(|k| seated(&k, &keys, &slots))
                    });
                    if let Some(s) = seat {
                        let twice = parked.iter().any(|p| p.done.key == done.key);
                        if twice || parked.len() >= PARK_MAX {
                            ShardStats::bump(&stats.refused_in_use);
                            spawn_refusal(done.connection, done.send, protocol::REFUSE_IN_USE);
                        } else {
                            kick_tenant(s, &mut keys, &slots, protocol::REFUSE_IN_USE);
                            ShardStats::bump(&stats.parked_in_use);
                            parked.push(Parked {
                                done,
                                slot: s,
                                generation: keys[s].generation,
                                until: tokio::time::Instant::now() + PARK_WAIT,
                            });
                        }
                        continue;
                    }
                }
                admit!(done);
            }
            _ = park_poll.tick(), if !parked.is_empty() => {
                let now = tokio::time::Instant::now();
                let mut i = 0;
                while i < parked.len() {
                    let w = slots.load(parked[i].slot);
                    let freed = crate::slot::state_of(w) == crate::slot::SLOT_EMPTY
                        || crate::slot::generation_of(w) != parked[i].generation;
                    if !freed && now < parked[i].until {
                        i += 1;
                        continue;
                    }
                    let p = parked.swap_remove(i);
                    // Freed, and nothing else seated the key meanwhile: the
                    // body is asleep and the join wakes it.
                    if freed && p.done.key.and_then(|k| seated(&k, &keys, &slots)).is_none() {
                        admit!(p.done);
                    } else {
                        ShardStats::bump(&stats.refused_in_use);
                        spawn_refusal(p.done.connection, p.done.send, protocol::REFUSE_IN_USE);
                    }
                }
            }
            Some((slot, gen)) = refresh_rx.recv() => {
                if slot < MAX_PLAYERS
                    && crate::slot::generation_of(slots.load(slot)) == gen
                    && crate::slot::state_of(slots.load(slot)) == crate::slot::SLOT_LIVE
                {
                    // A guest's answer is the shard's config, the same every
                    // time: there is nothing to ask again.
                    if let Some(k) = keys[slot].key {
                        if skin_reads.ask(&k, false, Instant::now()) {
                            read_skins(slot, gen, Some(k), &facts, &read_gate, &owned_tx);
                        }
                    }
                }
            }
            _ = minted_poll.tick(), if facts.minted.is_some() && !minted_in_flight => {
                minted_in_flight = true;
                let path = facts.minted.clone().unwrap_or_default();
                let tx = minted_tx.clone();
                let known = minted_len;
                tokio::task::spawn_blocking(move || {
                    let len = std::fs::metadata(&path).map_or(0, |m| m.len());
                    let text = if len != known {
                        std::fs::read_to_string(&path).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let _ = tx.blocking_send((len, text));
                });
                // Reads come due on the same beat.
                let now = Instant::now();
                rereads.retain(|(k, due)| {
                    if *due > now {
                        return true;
                    }
                    if let Some(s) = live_seat(k, &keys, &slots) {
                        if skin_reads.ask(k, true, now) {
                            read_skins(s, keys[s].generation, Some(*k), &facts, &read_gate, &owned_tx);
                        }
                    }
                    false
                });
            }
            Some((len, text)) = minted_rx.recv() => {
                minted_in_flight = false;
                if len != minted_len {
                    minted_len = len;
                    // The first look is the baseline: what was minted before
                    // this boot was read at each join since.
                    let (wallets, n) = crate::mints::minted_since(&text, minted_seen.unwrap_or(usize::MAX));
                    minted_seen = Some(n);
                    let now = Instant::now();
                    for w in wallets {
                        let Some(k) = PlayerKey::new(w.as_bytes()) else {
                            continue;
                        };
                        for after in REREAD_AFTER {
                            if rereads.len() == REREADS_MAX {
                                rereads.remove(0);
                            }
                            rereads.push((k, now + after));
                        }
                    }
                }
            }
            _ = prices_sweep.tick(), if prices_armed && !prices_in_flight => {
                prices_in_flight = true;
                let cfg = facts.skins.clone();
                let sc = facts.skin_content.clone();
                let tx = prices_done_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = tx.blocking_send(crate::skins::prices_of(&cfg, &sc));
                });
            }
            Some(prices) = prices_done_rx.recv() => {
                prices_in_flight = false;
                match prices {
                    crate::skins::Prices::Known(prices) => {
                        if prices_tx.push(crate::slot::SkinPricesMsg { prices: *prices }).is_ok() {
                            ShardStats::bump(&stats.skin_prices_read);
                        }
                    }
                    crate::skins::Prices::Unknown => ShardStats::bump(&stats.skin_prices_unknown),
                }
            }
            Some((slot, gen, key, face)) = face_rx.recv() => {
                face_reads.done(&key, match face {
                    crate::faces::Face::Known { name, pic } => Some((name, pic)),
                    crate::faces::Face::Unknown => None,
                });
                // The generation guard, widened to the wallet (`landing`).
                if let Some(slot) = landing(slot, gen, Some(&key), &keys, &slots) {
                    match face {
                        crate::faces::Face::Known { name, pic } => {
                            let msg = crate::slot::FaceMsg { slot, id: keys[slot].id, name, pic };
                            if faces_tx.push(msg).is_ok() {
                                ShardStats::bump(&stats.faces_read);
                            } else {
                                ShardStats::bump(&stats.faces_dropped);
                            }
                        }
                        crate::faces::Face::Unknown => ShardStats::bump(&stats.faces_unknown),
                    }
                }
            }
            Some((slot, gen, key, owned)) = owned_rx.recv() => {
                if let Some(k) = key.as_ref() {
                    skin_reads.done(k, match owned {
                        crate::skins::Owned::Known(set) => Some(set),
                        crate::skins::Owned::Unknown => None,
                    });
                }
                // The generation guard, widened to the wallet: an answer
                // about a tenant who has left lands on the same wallet's new
                // seat, never on whoever else holds the slot now (`landing`).
                if let Some(slot) = landing(slot, gen, key.as_ref(), &keys, &slots) {
                    match owned {
                        crate::skins::Owned::Known(set) => {
                            let msg = crate::slot::SkinsMsg { slot, id: keys[slot].id, owned: set };
                            if skins_tx.push(msg).is_ok() {
                                ShardStats::bump(&stats.skins_read);
                            } else {
                                ShardStats::bump(&stats.skins_dropped);
                            }
                        }
                        crate::skins::Owned::Unknown => ShardStats::bump(&stats.skins_unknown),
                    }
                }
            }
            _ = entitle_sweep.tick(), if facts.entitle.armed() && !sweep_in_flight => {
                // Snapshot who to ask about, with the generation each answer
                // must still match when it lands. A slot that turned over
                // while the round was out is a DIFFERENT player, and kicking
                // them on the previous tenant's verdict is the same class of
                // bug the save path's `id` check exists to prevent.
                let roster: Vec<(usize, u32, String)> = keys
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, ks)| {
                        // An account has no copy to check (the join refuses it
                        // too), and one in the batch would make `check_many`
                        // answer `Unknown` for everybody.
                        let k = ks.key.as_ref()?;
                        if crate::auth::is_account_key(k) {
                            return None;
                        }
                        let wallet = std::str::from_utf8(k.as_bytes()).ok()?.to_string();
                        let gen = crate::slot::generation_of(slots.load(slot));
                        Some((slot, gen, wallet))
                    })
                    .collect();
                if !roster.is_empty() {
                    sweep_in_flight = true;
                    let cfg = facts.entitle.clone();
                    let tx = kick_tx.clone();
                    let stats2 = stats.clone();
                    tokio::task::spawn_blocking(move || {
                        let wallets: Vec<String> =
                            roster.iter().map(|(_, _, w)| w.clone()).collect();
                        let verdicts = crate::entitle::check_many(&cfg, &wallets);
                        let mut kicks = Vec::new();
                        for ((slot, gen, _), v) in roster.iter().zip(verdicts) {
                            match v {
                                crate::entitle::Verdict::Nope => kicks.push((*slot, *gen)),
                                crate::entitle::Verdict::Unknown => {
                                    ShardStats::bump(&stats2.entitle_unknown);
                                }
                                crate::entitle::Verdict::Owns => {}
                            }
                        }
                        // Send even when empty: it is what clears the
                        // in-flight flag, so a quiet round cannot wedge the
                        // sweep off permanently.
                        let _ = tx.blocking_send(kicks);
                    });
                }
            }
            Some(kicks) = kick_rx.recv() => {
                sweep_in_flight = false;
                for (slot, gen) in kicks {
                    // The generation guard: only kick the tenant we asked
                    // about. A reconnect between the ask and the answer gets
                    // its own join check, so nothing is skipped by waiting.
                    if slot >= MAX_PLAYERS
                        || crate::slot::generation_of(slots.load(slot)) != gen
                        || crate::slot::state_of(slots.load(slot)) != crate::slot::SLOT_LIVE
                    {
                        continue;
                    }
                    ShardStats::bump(&stats.entitle_kicked);
                    slots.mark_leaving_why(slot, gen, protocol::REFUSE_TICKET);
                    if let Some(conn) = keys[slot].conn.take() {
                        // Closed with the refusal code rather than dropped,
                        // so the player is told WHY by the same table a join
                        // refusal uses. A silent close reads as a network
                        // fault, and "my internet is broken" is the wrong
                        // thing to believe when the fix is to buy a copy.
                        close_with_reason(conn, protocol::REFUSE_TICKET);
                    }
                }
            }
            _ = sweep.tick() => {
                while let Ok(link) = grave_rx.pop() {
                    drop(link); // net side deallocates, never the sim
                }
                // Reads owed to joins that were told a remembered answer
                // (`PlatformReads::owed`), now that the cooldown has run: at
                // most one per wallet per cooldown, however often it rejoined.
                let now = Instant::now();
                while let Some(k) = skin_reads.owed(now) {
                    if let Some(s) = live_seat(&k, &keys, &slots) {
                        if skin_reads.ask(&k, false, now) {
                            read_skins(s, keys[s].generation, Some(k), &facts, &read_gate, &owned_tx);
                        }
                    }
                }
                while let Some(k) = face_reads.owed(now) {
                    if let Some(s) = live_seat(&k, &keys, &slots) {
                        if face_reads.ask(&k, false, now) {
                            read_face(s, keys[s].generation, k, &facts, &read_gate, &face_tx);
                        }
                    }
                }
                // A tenant that ended by any road but a kick — a lane
                // failed, the sim sent it away, it would not take its
                // welcome — still has a QUIC connection, and its datagram
                // reader keeps that alive for as long as the peer acks. A
                // session the shard ended must not outlive it uncounted, so
                // it is closed here, with its reason when one was recorded.
                for (s, k) in keys.iter_mut().enumerate() {
                    let w = slots.load(s);
                    if state_of(w) == SLOT_LIVE && generation_of(w) == k.generation {
                        continue;
                    }
                    if let Some(conn) = k.conn.take() {
                        end_session(conn, slots.why(s, k.generation));
                    }
                }
                // Spectator seats whose target left (`NETCODE.md` §2.3). The
                // sim stopped feeding them the tick the target's slot turned
                // over; this closes the connection with the reason, on the
                // cadence the admin kick uses. A seat whose OWN session
                // ended is closed the way a player's is, above.
                for (i, seat) in seats.iter_mut().enumerate() {
                    if !seat.occupied {
                        continue;
                    }
                    let own = slots.load(MAX_PLAYERS + i);
                    if state_of(own) != SLOT_LIVE || generation_of(own) != seat.generation {
                        if let Some(conn) = seat.conn.take() {
                            end_session(conn, slots.why(MAX_PLAYERS + i, seat.generation));
                        }
                        *seat = SeatSlot::default();
                        continue;
                    }
                    let t = slots.load(seat.target);
                    if state_of(t) == SLOT_LIVE && generation_of(t) == seat.target_gen {
                        continue;
                    }
                    ShardStats::bump(&stats.spectate_ended);
                    slots.mark_leaving_why(
                        MAX_PLAYERS + i,
                        seat.generation,
                        protocol::REFUSE_WATCH_ENDED,
                    );
                    if let Some(conn) = seat.conn.take() {
                        close_with_reason(conn, protocol::REFUSE_WATCH_ENDED);
                    }
                    *seat = SeatSlot::default();
                }
                // Admin kicks, bans and unbans, on the same cadence as the
                // graveyard: an admin is a person typing, so 100 ms is
                // immediate and the arm costs a pop on an empty ring. Each
                // act answers its admin, because only this loop knows how
                // it went.
                while let Ok(act) = admin_rx.pop() {
                    let (by, id, ban_key) = match act {
                        crate::admin::AdminAct::Kick { by, id } => (by, id, None),
                        crate::admin::AdminAct::Ban { by, id, key } => (by, id, Some(key)),
                        crate::admin::AdminAct::Unban { by, prefix } => {
                            let how = bans.remove_prefix(&prefix);
                            match how {
                                Ok(_) => {
                                    ShardStats::bump(&stats.admin_unbanned);
                                    // Written whole, like a ban: the lift
                                    // has to outlive a restart too.
                                    if bans.save().is_err() {
                                        ShardStats::bump(&stats.save_write_errors);
                                    }
                                }
                                Err(_) => ShardStats::bump(&stats.admin_refused),
                            }
                            let line = crate::admin::unban_line(&prefix, &how);
                            answer_admin(&mut answer_tx, by, &line, &stats);
                            continue;
                        }
                    };
                    if let Some(key) = ban_key {
                        // Recorded before the kick, so a full list refuses
                        // the BAN rather than kicking and forgetting why.
                        if !bans.insert(key) {
                            ShardStats::bump(&stats.admin_refused);
                            let line = crate::admin::ban_full_line(id);
                            answer_admin(&mut answer_tx, by, &line, &stats);
                            continue;
                        }
                        // Written whole on each ban: an admin typing, a few
                        // KiB at most. A failed write keeps the ban for the
                        // uptime and counts like the player store's.
                        if bans.save().is_err() {
                            ShardStats::bump(&stats.save_write_errors);
                        }
                    }
                    let mut kicked = admin_kick(id, &mut keys, &slots);
                    // A ban is the wallet's, wherever it sits now: a session
                    // that reconnected after the sim named `id` holds a new
                    // id on the same key, and one parked waiting for its old
                    // body would be seated by the next poll.
                    if let Some(key) = ban_key {
                        if let Some(s) = seated(&key, &keys, &slots) {
                            kicked |= kick_tenant(s, &mut keys, &slots, protocol::REFUSE_ADMIN);
                        }
                        let mut i = 0;
                        while i < parked.len() {
                            if parked[i].done.key == Some(key) {
                                let p = parked.swap_remove(i);
                                ShardStats::bump(&stats.refused_banned);
                                spawn_refusal(p.done.connection, p.done.send, protocol::REFUSE_ADMIN);
                            } else {
                                i += 1;
                            }
                        }
                    }
                    let counter = if kicked { &stats.admin_kicked } else { &stats.admin_refused };
                    ShardStats::bump(counter);
                    // A ban whose target already left still took: the door
                    // refuses them next time, which is what the admin wants
                    // to hear.
                    let line = match ban_key {
                        Some(key) => crate::admin::banned_line(id, &key),
                        None => crate::admin::kicked_line(id, kicked),
                    };
                    answer_admin(&mut answer_tx, by, &line, &stats);
                }
                drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);
                if shutdown.load(Ordering::Relaxed) {
                    // The sim thread is flushing every connected player's
                    // record right now, and this loop is the only thing that
                    // can carry those to the store. Returning on the first
                    // look at the flag — which is what this did — threw them
                    // away and left the shutdown flush writing into a ring
                    // nobody would ever read.
                    drain_until_abandoned(&mut save_rx, &mut write_tx, &mut store, &keys, &stats)
                        .await;
                    endpoint.close(wtransport::VarInt::from_u32(0), b"shutdown");
                    return;
                }
            }
        }
    }
}

/// The accept loop's half of the shutdown flush: file records until the sim
/// thread lets go of the ring.
///
/// Waited on the *producer being dropped*, not on a duration: the sim thread
/// drops `save_tx` when it is finished, so this is an exact signal. The try
/// count is a backstop for a sim thread that is wedged rather than finishing,
/// because "no bound is wait" applies here too. Its own function so a test
/// can hold the producer open (`NOW.md` §0y item 4).
async fn drain_until_abandoned(
    save_rx: &mut rtrb::Consumer<SaveMsg>,
    write_tx: &mut rtrb::Producer<WriteMsg>,
    store: &mut SaveStore,
    keys: &[KeySlot; MAX_PLAYERS],
    stats: &Arc<ShardStats>,
) {
    for _ in 0..SHUTDOWN_DRAIN_TRIES {
        drain_saves(save_rx, write_tx, store, keys, stats);
        if save_rx.is_abandoned() {
            break;
        }
        tokio::time::sleep(SHUTDOWN_DRAIN_POLL).await;
    }
    drain_saves(save_rx, write_tx, store, keys, stats);
}

/// An Elo Pros account that has linked a wallet comes in as the wallet, on a
/// pass that still names the account (v151). Unless the wallet has a record
/// of its own, the account's record is the wallet's now, rewritten in its
/// row: a restart, or the account coming in again, finds it under the wallet
/// only, so it is never two characters. Returns whether the wallet takes the
/// account's character; the sim moves the rest (`ShardCore::adopt_account`).
fn adopt_account_save(
    store: &mut SaveStore,
    write_tx: &mut rtrb::Producer<WriteMsg>,
    from: &PlayerKey,
    to: &PlayerKey,
    stats: &Arc<ShardStats>,
) -> bool {
    if store.find(to).is_some() {
        return false;
    }
    if let Some((index, stamp, save)) = store.rekey(from, to) {
        let key = *to;
        if write_tx
            .push(WriteMsg {
                index,
                key,
                stamp,
                save,
            })
            .is_err()
        {
            // The index has it; the wallet's next record lands in that row.
            ShardStats::bump(&stats.save_ring_drops);
        }
    }
    true
}

/// Hand one record to the store's index. Called only from the sim thread, so
/// it does exactly one thing that can fail and counts it: a full ring drops
/// the newest record (limits.rs `SAVE_RING_CAP`), which costs freshness and
/// never correctness — the sweep comes round again and a record is filed by
/// key, in place.
///
/// One caller passes `key`: the eviction save, whose ring-drop story is
/// different. The store keeps the record the victim's leave filed, stale by
/// the raid, but the sim kept its own copy at the pick (`core.rs`
/// `EvictMemo`) and a returning victim rejoins from that, not from the
/// store's. A drop still costs the raid if a restart, or `MAX_PLAYERS`
/// later evictions, come before the victim does.
fn push_save(
    save_tx: &mut rtrb::Producer<SaveMsg>,
    id: u32,
    key: Option<PlayerKey>,
    save: sim_core::persist::PlayerSave,
    stats: &Arc<ShardStats>,
) {
    if save_tx.push(SaveMsg { id, key, save }).is_err() {
        ShardStats::bump(&stats.save_ring_drops);
    }
}

/// Unix seconds. Read here and nowhere near the sim thread — a save's stamp
/// orders eviction across restarts, which the world's tick counter cannot do
/// because it begins again at 0.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// File every record the sim has handed over, and pass each on to the disk.
///
/// Bounded by the ring's own capacity, so this cannot run long however far
/// behind it fell. A record for an id whose slot has already moved on is
/// dropped: see [`KeySlot`] for why that is the safe direction, and why the
/// ordering above keeps it from happening at all.
fn drain_saves(
    save_rx: &mut rtrb::Consumer<SaveMsg>,
    write_tx: &mut rtrb::Producer<WriteMsg>,
    store: &mut SaveStore,
    keys: &[KeySlot; MAX_PLAYERS],
    stats: &Arc<ShardStats>,
) {
    while let Ok(msg) = save_rx.pop() {
        let key = match msg.key {
            // The sim named the key itself: an eviction save. The victim's
            // connection ended long ago, so the id below names a slot some
            // later tenant may hold — the table here cannot resolve it, and
            // the sim's own sleeper index was the last pairing standing
            // (`SaveMsg::key`).
            Some(key) => key,
            None => {
                // `id = generation << 8 | slot` (see `install`), so the
                // slot is the low byte and the generation check is the id
                // equality below.
                let slot = (msg.id & 0xFF) as usize;
                if slot >= MAX_PLAYERS || keys[slot].id != msg.id {
                    continue;
                }
                let Some(key) = keys[slot].key else {
                    continue; // a guest: admitted, remembered by nobody
                };
                key
            }
        };
        let stamp = now_secs();
        ShardStats::bump(&stats.saves_taken);
        // A full table refused a newcomer it could not fit without pushing
        // out somebody worth more (`SaveStore::put`): remembered by nobody,
        // as a guest is. Its body is still in the world. An eviction record
        // worth anything is refused only past the table's budget
        // (`put_evicted`), and then its one copy is the sim's memo
        // (`core::EvictMemo`) until a restart: said out loud, as well as
        // counted.
        let evicted = msg.key.is_some();
        let put = if evicted {
            store.put_evicted(&key, stamp, msg.save)
        } else {
            store.put(&key, stamp, msg.save)
        };
        let Some(put) = put else {
            ShardStats::bump(&stats.saves_refused);
            if evicted && crate::store::worth(&msg.save) > 0 {
                eprintln!(
                    "store: an eviction record was refused (table full, eviction budget \
                     spent); it is kept in memory only"
                );
            }
            continue;
        };
        if put.evicted {
            ShardStats::bump(&stats.saves_evicted);
        }
        if write_tx
            .push(WriteMsg {
                index: put.index,
                key,
                stamp,
                save: msg.save,
            })
            .is_err()
        {
            // The index has it; the disk does not. Freshness, not
            // correctness — the sweep re-takes this player, and the record
            // is idempotent (filed by key, written in place).
            ShardStats::bump(&stats.save_ring_drops);
        }
    }
}

/// A side file the store thread failed to write (`side_file_errors`) is
/// handed over again: the teams at the save that follows, the ledger on the
/// next tick. On the world-save cadence, so a disk that keeps failing costs
/// one retry per save, not one per tick.
fn retry_side_files(core: &mut ShardCore, stats: &ShardStats, seen: &mut u64) {
    let failed = ShardStats::get(&stats.side_file_errors);
    if failed != *seen {
        *seen = failed;
        core.teams.forget_written();
        core.gifts.mark_dirty();
        core.mints.mark_dirty();
    }
}

/// Take one world save: fill a pooled buffer and hand it to the store
/// thread, or skip and count.
///
/// **The whole of the sim thread's cost is in here**, and it is two linear
/// passes over state that is already in cache: `encode_world` writes
/// integers into a buffer that already exists, and `identities` fills a
/// `Vec` whose capacity was reserved at boot. No allocation (wall 2), no
/// lock or syscall (wall 3), and a ceiling that does not move with the size
/// of the world (`WORLD_SAVE_MAX_BYTES`, wall 4).
///
/// That is the whole answer to `reference/SAVES.md` §4 — a stop-the-world
/// freeze the reference game has not fixed in thirteen years, whose only
/// mitigation is a convar with a bad end on both sides. The freeze is not
/// caused by saving being expensive; it is caused by *serialising an object
/// graph on the thread that runs the game*. Split those two and the knob
/// stops having a second end.
///
/// A skip is the stated overflow policy and not a failure: the cadence
/// comes around again with a fresher world, so nothing is lost that a later
/// save does not carry.
fn take_world_save(
    core: &mut ShardCore,
    pool: &mut Vec<Box<[u8]>>,
    idents: &mut Vec<Vec<(PlayerKey, u32)>>,
    world_tx: &mut rtrb::Producer<WorldMsg>,
    file_tx: &mut rtrb::Producer<FileMsg>,
    stats: &ShardStats,
) {
    let (Some(mut buf), Some(mut ids)) = (pool.pop(), idents.pop()) else {
        ShardStats::bump(&stats.world_saves_skipped);
        return;
    };
    let Some(len) = core.encode_world(&mut buf) else {
        // Unreachable: the buffer is `WORLD_SAVE_MAX_BYTES` and that is the
        // ceiling by construction. Counted rather than unwrapped, and the
        // buffer goes back to the pool either way.
        ShardStats::bump(&stats.world_save_errors);
        pool.push(buf);
        idents.push(ids);
        return;
    };
    ids.resize(MAX_PLAYERS, (PlayerKey::PLACEHOLDER, 0));
    let n = core.identities(&mut ids);
    ids.truncate(n);
    // The teams by the same keys, so a restart finds them again. The text is
    // made here, only when it changed; the store thread writes it. No room
    // on the ring leaves it for the next save (`FILE_RING_CAP`); this thread
    // is the ring's only producer, so room now is room at the push.
    if !file_tx.is_full() {
        if let Some(msg) = core.teams.take_write(&ids) {
            let _ = file_tx.push(msg);
        }
    }
    // And the ids evicted bodies wore, so their owners' bases are still
    // theirs after a restart (`core::EvictIds`). They ride this world and
    // land after it (`WorldMsg::side`), never ahead of or without it.
    let side = core.evict_ids.take_write();
    let tick = core.world.tick;
    if let Err(rtrb::PushError::Full(msg)) = world_tx.push(WorldMsg {
        tick,
        len,
        buf,
        idents: ids,
        side,
    }) {
        ShardStats::bump(&stats.world_saves_skipped);
        if msg.side.is_some() {
            core.evict_ids.mark_dirty();
        }
        pool.push(msg.buf);
        let mut back = msg.idents;
        back.clear();
        idents.push(back);
    }
}

/// The storage thread: the only thing in this process that touches the save
/// file, the world file and the side files (`slot::FileMsg`), and it makes
/// no decisions — the index owner already chose the slot.
///
/// Exits when its producer is gone AND the ring is dry, which is exact rather
/// than timed: the accept loop dropping `write_tx` is the shutdown signal, and
/// a sleep-then-guess would either lose the last records or hold the process
/// open for a fixed pause.
fn store_thread(
    mut file: SaveFile,
    mut write_rx: rtrb::Consumer<WriteMsg>,
    mut world_file: crate::worldfile::WorldFile,
    mut world_rx: rtrb::Consumer<WorldMsg>,
    mut world_done_tx: rtrb::Producer<WorldDone>,
    mut file_rx: rtrb::Consumer<FileMsg>,
    stats: Arc<ShardStats>,
) {
    loop {
        // Read BEFORE the drains: a producer seen gone here has pushed all it
        // ever will, and the drains below take every bit of it. Read after,
        // a push landing between a ring's drain and the check would be left
        // on the ring.
        let gone = write_rx.is_abandoned() && world_rx.is_abandoned() && file_rx.is_abandoned();
        std::sync::atomic::fence(Ordering::Acquire);
        let mut idle = true;
        // The world first: it is the bigger write and the rarer one, and
        // taking it before the player records means a shutdown flush lands
        // the world before the thread notices its producers are gone.
        while let Ok(msg) = world_rx.pop() {
            idle = false;
            let (ok, landed) = match world_file.write(msg.tick, &msg.buf[..msg.len], &msg.idents) {
                Ok(true) => {
                    ShardStats::bump(&stats.world_saves_written);
                    (true, true)
                }
                // No file: nothing written, nothing counted — the same rule
                // `SaveFile::write` states, and for the same reason.
                Ok(false) => (true, false),
                Err(_) => {
                    ShardStats::bump(&stats.world_save_errors);
                    (false, false)
                }
            };
            // The evicted ids, only beside a world that landed: a crash
            // between the two leaves the newer world with the older ids,
            // which only ever names ids the boot filters or a wake spends.
            // No world file, no world for them to name: dropped, not owed.
            let side_lost = match msg.side.as_ref() {
                None => false,
                Some(_) if !landed => !ok,
                Some(side) => match crate::gifts::write_atomic(&side.path, &side.text) {
                    Ok(()) => false,
                    Err(e) => {
                        eprintln!("store: writing {}: {e}", side.path.display());
                        ShardStats::bump(&stats.side_file_errors);
                        true
                    }
                },
            };
            // The buffer goes home whatever happened. A write that failed
            // must not also cost the pool a buffer, or a shard with a full
            // disk would stop being *able* to save once the disk was fixed.
            let WorldMsg {
                buf, mut idents, ..
            } = msg;
            idents.clear();
            let _ = world_done_tx.push(WorldDone {
                buf,
                idents,
                ok,
                side_lost,
            });
        }
        // The side files (`slot::FileMsg`): each a whole file, temp + fsync
        // + rename. The teams are handed over just before the world they
        // were keyed against, so they may land a pass ahead of it — the order
        // the old synchronous write had. A failed write is logged; the
        // in-memory copy stands and the next change writes it again.
        while let Ok(msg) = file_rx.pop() {
            idle = false;
            if let Err(e) = crate::gifts::write_atomic(&msg.path, &msg.text) {
                eprintln!("store: writing {}: {e}", msg.path.display());
                ShardStats::bump(&stats.side_file_errors);
            }
        }
        while let Ok(msg) = write_rx.pop() {
            idle = false;
            match file.write(msg.index, &msg.key, msg.stamp, &msg.save) {
                Ok(true) => ShardStats::bump(&stats.saves_written),
                // No file: nothing was written and nothing is counted. The
                // index still has the record, which is the documented
                // in-memory case (`config.rs` on `save_file`) — counting it as
                // written would report persistence a shard does not have.
                Ok(false) => {}
                // A shard that cannot persist keeps running: dropping every
                // player because a disk filled would be a worse outcome than
                // a shard that forgets. Counted, and that counter is the only
                // place this is visible.
                Err(_) => ShardStats::bump(&stats.save_write_errors),
            }
        }
        // Every ring, read by `gone` above: the drains in this pass ran after
        // all three producers were seen dropped, so nothing pushed can still
        // be waiting.
        if gone {
            // Everything this process will ever write has been written.
            // `bin/shard.rs` waits on this before it exits, which is what
            // turns a SIGTERM into a save instead of a lost hour.
            ShardStats::raise(&stats.store_stopped);
            return;
        }
        if idle {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Session + hello, off the accept loop so a slow client can't
/// head-of-line-block accepts. Version gate lives here; the cap gate needs
/// the slot table and lives in `install`.
#[allow(clippy::too_many_arguments)]
async fn handshake_task(
    incoming: IncomingSession,
    done_tx: tokio::sync::mpsc::Sender<Handshaken>,
    stats: Arc<ShardStats>,
    require_auth: bool,
    siwe_domains: Arc<[String]>,
    pass_key: Option<[u8; 32]>,
    entitle: crate::entitle::Config,
    min_client: u32,
    spectate_open: bool,
    agents_only: bool,
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
    sources: Arc<SourceGate>,
    permit: Option<SourcePermit>,
    signing: Arc<SignWait>,
    post_sign: Arc<tokio::sync::Semaphore>,
) {
    // Decremented however this task leaves — the timeout arm, any of the
    // refusal arms, or success. A guard rather than a line at the end,
    // because `handshake_task` has more than one exit and a leaked count
    // would ratchet the admission gate shut permanently: the shard would
    // start refusing everyone and no counter would explain why.
    struct InFlight(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for InFlight {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let guard = InFlight(in_flight);

    let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let request = incoming.await.map_err(|_| ())?;
        // The QUIC handshake is done, so the address is proven: count it
        // against its source (`ADMIT_PER_SOURCE`) for the rest of this task,
        // unless the door already did.
        let permit = match permit {
            Some(p) => p,
            None => {
                let Some(p) = SourceGate::take(&sources, request.remote_address().ip()) else {
                    ShardStats::bump(&stats.refused_per_source);
                    request.too_many_requests().await;
                    return Err(());
                };
                p
            }
        };
        let connection = request.accept().await.map_err(|_| ())?;
        let (send, mut recv) = connection.accept_bi().await.map_err(|_| ())?;
        let (hello_buf, hello_len) = read_frame(&mut recv).await.ok_or(())?;
        Ok::<_, ()>((connection, send, recv, hello_buf, hello_len, permit))
    })
    .await;
    let Ok(Ok((connection, send, mut recv, hello_buf, hello_len, permit))) = result else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };
    // **The version first, read on its own** (`protocol::peek_hello_version`).
    // The hello grew at v73; decoding the whole message before comparing the
    // version would drop every older client's shorter hello as a handshake
    // error — a timeout on their screen — where it is owed a posted "update
    // the game".
    match protocol::peek_hello_version(&hello_buf[..hello_len]) {
        Ok(v) if v == PROTO_VER => {}
        Ok(_) => {
            ShardStats::bump(&stats.refused_version);
            spawn_refusal(connection, send, REFUSE_VERSION);
            return;
        }
        Err(_) => {
            ShardStats::bump(&stats.handshake_errors);
            return;
        }
    }
    let Ok(hello) = decode_hello(&hello_buf[..hello_len]) else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };
    // The release floor, second because it is only meaningful once the two
    // sides agree on what the bytes are — `hello.ver` is not a number until
    // `proto_ver` says the layout it was read from is this one.
    //
    // `<`, never `!=`: a client NEWER than the shard is admitted. That is
    // deliberate and it is the direction that keeps a release shippable — a
    // player who updated first must not be locked out of a shard its operator
    // has not restarted yet, and the wire compatibility that would actually
    // break is `PROTO_VER`'s to refuse, one gate up.
    if hello.ver < min_client {
        // The counter is the whole record, and that is this crate's shape
        // rather than a shortcut: there is no logger in `server`'s lib — the
        // bins own every line of output — so a refusal is observable the way
        // every other refusal here is, through `ShardStats`. The shard binary
        // prints the floor and its own build at boot, which is the other half
        // an operator needs to read this number.
        ShardStats::bump(&stats.refused_build);
        spawn_refusal(connection, send, protocol::REFUSE_BUILD);
        return;
    }
    // A watcher at a shut door is told so before any crypto: there is no
    // identity question worth asking a session this shard will not seat.
    if hello.is_spectator() && !spectate_open {
        ShardStats::bump(&stats.spectate_refused);
        spawn_refusal(connection, send, protocol::REFUSE_WATCH);
        return;
    }
    // A person at an agent-only world is told so before any crypto too: no
    // signature would change the answer.
    if !admits_kind(agents_only, hello.is_agent(), hello.is_spectator()) {
        ShardStats::bump(&stats.refused_agents_only);
        spawn_refusal(connection, send, protocol::REFUSE_AGENTS);
        return;
    }
    // ---- SIWE, and the nonce never leaves this stack frame --------------
    //
    // The server picks a nonce, the client signs a message containing it,
    // and the server recovers the signer. There is no nonce table to size,
    // expire or sweep: the value lives in this task's local and nothing else
    // in the process can see it, so a signature captured on one connection
    // is worthless on every other — no other connection ever chose it.
    //
    // Sent to a stranger before anything about them exists, which is safe
    // precisely because it is random and means nothing: the server has
    // nothing to lose by challenging someone it will go on to refuse.
    let mut nonce = [0u8; protocol::NONCE_BYTES];
    if getrandom::getrandom(&mut nonce).is_err() {
        // No OS entropy is not a thing to work around with a weaker nonce —
        // a guessable one is a signature anybody can replay. Refuse the
        // connection instead.
        ShardStats::bump(&stats.handshake_errors);
        spawn_refusal(connection, send, protocol::REFUSE_AUTH);
        return;
    }
    let issued_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let challenge = protocol::Challenge { nonce, issued_at };

    let mut send = send;
    // Two deadlines, because two different things answer. Writing the
    // challenge is this machine's work and keeps the handshake's 5 s; the
    // reply waits on a wallet prompt and a person reading it, which is
    // `SIGN_WAIT`. One budget for both refused every human signer.
    let wrote =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, write_challenge(&mut send, &challenge)).await;
    let Ok(Ok(_)) = wrote else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };
    // The shard's own work is done until the reply: the wait leaves the
    // admission gate (`in_flight`) for the sign-in pool, where the network
    // holding the most seats makes room for a newer waiter ([`SIGN_WAITERS`]).
    // Its network is the one its handshake counts against, which `permit`
    // goes on holding to the end of this task.
    drop(guard);
    let ticket = SignWait::join(&signing, permit.bucket.map(|(_, net)| net));
    let exchange = tokio::select! {
        r = tokio::time::timeout(SIGN_WAIT, async {
            let (buf, len) = read_auth_frame(&mut recv).await.ok_or(())?;
            protocol::decode_auth(&buf[..len]).map_err(|_| ())
        }) => r.ok(),
        _ = ticket.bumped() => None,
    };
    drop(ticket);
    let Some(Ok(auth)) = exchange else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };

    // **A watcher is anonymous by construction** (`NETCODE.md` §2.3): it has
    // no body, keeps nothing and can change nothing, so the three doors below
    // — `require_auth`, the signature, the ticket — guard things it cannot
    // touch. Its auth frame is read (the handshake has one shape for every
    // session) and discarded unverified; what it may watch is decided by the
    // TARGET's consent, in `install_spectator`, not by who the watcher is.
    if hello.is_spectator() {
        let _ = done_tx
            .send(Handshaken {
                connection,
                send,
                recv,
                key: None,
                alias: None,
                address: protocol::Address::GUEST,
                hello,
            })
            .await;
        return;
    }

    // From here the work is the shard's again — the signature's check, the
    // ticket read, the hand-off — so it waits its turn ([`POST_SIGN_PERMITS`]).
    // Waited for, never skipped: the ticket door fails open, so a turn
    // refused for want of room could only become a join nobody checked.
    let Ok(_turn) = post_sign.acquire().await else {
        ShardStats::bump(&stats.handshake_errors);
        return;
    };

    // A guest offers no address and is admitted only where guests are.
    // Everyone else is *proved*: the signature is recovered against the
    // message this server built from the nonce it chose, so the key below is
    // an address somebody holds the private key for and not a string they
    // typed.
    //
    // **A Solana account (v142)** proves its key the same way, over the
    // Solana form of the same text (`auth::verify_solana_any`), and then elo
    // says which player it is: linked to an Ethereum account, it IS that
    // player — the same save, the same copy — and unlinked it is its own,
    // filed under its base58 address. A shard with no origin asks nobody.
    // When elo cannot be asked, the join is refused (`REFUSE_LINK`) rather
    // than filed under a guess: a linked player seated on their Solana key
    // for the length of an outage would be a second character.
    //
    // **An Elo Pros pass (v151)** is elo saying who is signed in on its site:
    // checked against elo's pinned key, this game, these same names and this
    // join's nonce, with no call out. A pass naming a wallet IS that wallet — the same save and
    // copy as signing with it, and nothing asked it to sign. One naming no
    // wallet is the account itself, filed under its `acct_…` id, and one
    // naming both is how an account that played before it linked a wallet
    // gets its character back as the wallet (`alias`, `adopt_account_save`).
    let mut alias = None;
    let (key, proven) = if let Some(pass) = auth.pass {
        let passed = pass_key
            .ok_or(crate::auth::AuthError::NoPassKey)
            .and_then(|k| {
                crate::auth::verify_pass(pass.as_bytes(), &k, &siwe_domains, &nonce, issued_at)
            });
        match passed {
            Ok(p) => match p.wallet {
                Some(addr) => {
                    alias = Some(p.account);
                    (crate::auth::key_of(&addr), addr)
                }
                None => (Some(p.account), protocol::Address::GUEST),
            },
            Err(_) => {
                ShardStats::bump(&stats.refused_auth);
                spawn_refusal(connection, send, protocol::REFUSE_AUTH);
                return;
            }
        }
    } else if auth.solana.is_some() {
        let sol = match crate::auth::verify_solana_any(&siwe_domains, &nonce, issued_at, &auth) {
            Ok(sol) => sol,
            Err(_) => {
                ShardStats::bump(&stats.refused_auth);
                spawn_refusal(connection, send, protocol::REFUSE_AUTH);
                return;
            }
        };
        let (b58, n) = sol.to_base58();
        let text = core::str::from_utf8(&b58[..n])
            .unwrap_or_default()
            .to_string();
        let cfg = entitle.clone();
        let link = tokio::task::spawn_blocking(move || crate::entitle::link_of(&cfg, &text))
            .await
            .unwrap_or(crate::entitle::Link::Unknown);
        match link {
            crate::entitle::Link::To(addr) => (crate::auth::key_of(&addr), addr),
            crate::entitle::Link::Unlinked => {
                (crate::auth::solana_key_of(&sol), protocol::Address::GUEST)
            }
            crate::entitle::Link::Unknown => {
                ShardStats::bump(&stats.refused_auth);
                spawn_refusal(connection, send, protocol::REFUSE_LINK);
                return;
            }
        }
    } else if auth.address.is_guest() {
        (None, protocol::Address::GUEST)
    } else {
        match crate::auth::verify_any(&siwe_domains, &nonce, issued_at, &auth) {
            Ok(key) => (Some(key), auth.address),
            Err(_) => {
                // Counted, never explained to the caller: which of the four
                // ways a signature can be wrong is not a stranger's business.
                ShardStats::bump(&stats.refused_auth);
                spawn_refusal(connection, send, protocol::REFUSE_AUTH);
                return;
            }
        }
    };
    if require_auth && key.is_none() {
        ShardStats::bump(&stats.refused_auth);
        spawn_refusal(connection, send, protocol::REFUSE_AUTH);
        return;
    }

    // ---- the ticket door ------------------------------------------------
    //
    // Asked only of a PROVED address, and after `require_auth`, because a
    // guest has no wallet to ask about — `config.rs` refuses the armed-over-
    // open pairing at boot so this cannot silently check nobody.
    //
    // On its own blocking thread rather than inline: `ureq` is synchronous
    // and this task shares a tokio worker with every other handshake in
    // flight, so a slow origin would stall strangers who are not waiting on
    // it. `entitle::Config::timeout` bounds the call itself, and it has to:
    // no handshake deadline covers this step.
    //
    // **`Unknown` admits.** The only value that refuses is a definite
    // on-chain zero; an outage must not become a shard nobody can join.
    // `entitle::Verdict::admits` is the one place that decision lives.
    if entitle.armed() {
        // An account with no wallet has no copy to check, and a free Elo Pros
        // login is not a bought copy: it is refused until it links the wallet
        // that holds one. The door is closed, not an outage, so this is not
        // the `Unknown` that admits.
        if key.as_ref().is_some_and(crate::auth::is_account_key) {
            ShardStats::bump(&stats.refused_ticket);
            spawn_refusal(connection, send, protocol::REFUSE_TICKET);
            return;
        }
        if let Some(k) = key.as_ref() {
            // The key IS the wallet: `auth::key_of` builds it from
            // `Address::to_hex`, which is ASCII `0x…` lowercase (or, for an
            // unlinked Solana player, `auth::solana_key_of`'s base58). Decoded
            // rather than transmuted, and a key that somehow is not utf8
            // becomes an empty string that `entitle::is_wallet` refuses —
            // which is `Unknown`, which admits.
            let wallet = std::str::from_utf8(k.as_bytes())
                .unwrap_or_default()
                .to_string();
            let cfg = entitle.clone();
            let verdict =
                tokio::task::spawn_blocking(move || crate::entitle::check_one(&cfg, &wallet))
                    .await
                    // A panicked or cancelled blocking task is "we could not look",
                    // and reaches the same door every other failure does.
                    .unwrap_or(crate::entitle::Verdict::Unknown);

            match verdict {
                crate::entitle::Verdict::Nope => {
                    ShardStats::bump(&stats.refused_ticket);
                    spawn_refusal(connection, send, protocol::REFUSE_TICKET);
                    return;
                }
                // Counted, not logged: the point of the counter is that a
                // fail-open is VISIBLE to the operator. An address in a log
                // line would be the other thing.
                crate::entitle::Verdict::Unknown => {
                    ShardStats::bump(&stats.entitle_unknown);
                }
                crate::entitle::Verdict::Owns => {}
            }
        }
    }
    // The proven address rides along for the spectator door: `key` is only
    // `Some` when `verify` recovered this very address from the signature —
    // or, for a Solana account, the Ethereum address it is linked to (an
    // unlinked one has none, and is watched by nobody by address).
    let address = if key.is_some() {
        proven
    } else {
        protocol::Address::GUEST
    };
    let _ = done_tx
        .send(Handshaken {
            connection,
            send,
            recv,
            key,
            alias,
            address,
            hello,
        })
        .await;
}

/// Claim a slot, build the rings, hand the sim its ends, spawn the tasks
/// that welcome the client and then serve it. Any refusal is posted, never
/// a hang (DESIGN.md §5.9). **Not `async`, on purpose**: this runs inside
/// the accept loop, and the one await it used to make — the welcome's write
/// — waited on the peer, so a client that never read its stream froze every
/// join, kick and save behind it. The welcome is written by the
/// connection's own task now ([`write_opening`]).
#[allow(clippy::too_many_arguments)]
fn install(
    done: Handshaken,
    source: Option<std::net::IpAddr>,
    said: crate::slot::Said,
    facts: &ShardFacts,
    ctrl_tx: &mut rtrb::Producer<Connect>,
    refresh_tx: &tokio::sync::mpsc::Sender<(usize, u32)>,
    keys: &mut [KeySlot; MAX_PLAYERS],
    store: &SaveStore,
    slots: &Arc<SlotTable>,
    stats: &Arc<ShardStats>,
) -> Option<(usize, u32)> {
    let Handshaken {
        connection,
        send,
        recv,
        key,
        alias,
        address,
        hello,
    } = done;
    if !agent_seat_free(
        facts.agent_seats,
        ShardStats::get(&stats.agents),
        hello.is_agent(),
    ) {
        ShardStats::bump(&stats.refused_agents);
        spawn_refusal(connection, send, REFUSE_FULL);
        return None;
    }
    if source.is_some_and(|ip| seats_held(keys, slots, ip) >= SEATS_PER_SOURCE) {
        ShardStats::bump(&stats.refused_per_source);
        spawn_refusal(connection, send, REFUSE_FULL);
        return None;
    }
    let Some((slot, generation)) = (0..MAX_PLAYERS).find_map(|s| slots.claim(s).map(|g| (s, g)))
    else {
        ShardStats::bump(&stats.refused_full);
        spawn_refusal(connection, send, REFUSE_FULL);
        return None;
    };
    // Player id: slot in the low byte, claim generation above — unique
    // across slot reuse, stable for the connection's life.
    //
    // The generation is masked to 23 bits **so bit 31 is never set**, and
    // that is a wire contract rather than a tidiness: `limits::MOB_ID_TAG`
    // is the high bit and it means "this class-D entity is an animal, draw
    // it as one". `SlotTable::claim` retires a slot at this mask rather than
    // wrap — a wrapped id is an old tenant's, and a reconnect loop 2²³ long
    // would have been handed whatever still names it — so the mask is a
    // backstop; it costs nothing and `tests/mob.rs` asserts it.
    let id = ((generation & PLAYER_ID_GEN_MASK) << 8) | slot as u32;
    // Who this connection is, for the whole of its life. Recorded before the
    // sim is told anything, so a record coming back from the very first tick
    // already has a key to be filed under.
    keys[slot] = KeySlot {
        key,
        id,
        // Cloned for the sweep. Every other clone of this handle lives in a
        // reader/writer task; this one lives exactly as long as the slot
        // does, and `install` overwrites it on the next tenant.
        conn: Some(connection.clone()),
        generation,
        address,
        agent: hello.is_agent(),
        watchable: hello.consents_to_watch(),
        name: hello.name,
        source,
    };
    // Does this shard remember them? A miss is the ordinary case and it is
    // not a failure: a guest, a first visit, or a shard with no save file.
    // A hit can be stale — an eviction record still on the save ring — and
    // the sim, which kept that record, decides (`ShardCore::connect_as`).
    let save = key.and_then(|k| store.find(&k));

    let (input_tx, input_rx) = RingBuffer::new(INPUT_RING_CAP);
    let (action_tx, action_rx) = RingBuffer::<ActionMsg>::new(ACTION_RING_CAP);
    let (chat_tx, chat_rx) = RingBuffer::<ChatMsg>::new(CHAT_RING_CAP);
    let (voice_tx, voice_rx) = RingBuffer::<protocol::VoiceFrame>::new(VOICE_RING_CAP);
    let (voice_out_tx, voice_out_rx) = RingBuffer::<VoiceMsg>::new(VOICE_OUT_RING_CAP);
    let (snap_tx, snap_rx) = RingBuffer::<SnapMsg>::new(SNAPSHOT_RING_CAP);
    let (ev_tx, ev_rx) = RingBuffer::<EvMsg>::new(EVENT_RING_CAP);
    let link = Link {
        generation,
        input: input_rx,
        actions: action_rx,
        chats: chat_rx,
        voice_in: voice_rx,
        voice_out: voice_out_tx,
        snaps: snap_tx,
        events: ev_tx,
    };
    if ctrl_tx
        .push(Connect {
            slot,
            id,
            save,
            key,
            alias,
            watch: None,
            said,
            agent: hello.is_agent(),
            name: if hello.is_agent() {
                hello.name
            } else {
                protocol::Name::EMPTY
            },
            link,
        })
        .is_err()
    {
        // Control ring full: refuse rather than wait (L4 — no bound is
        // "wait"). The claim reverts; the client may retry.
        slots.unclaim(slot, generation);
        ShardStats::bump(&stats.refused_full);
        spawn_refusal(connection, send, REFUSE_FULL);
        return None;
    }
    // `saves_restored` is **not** counted here, and it used to be. A record
    // existing is no longer the same fact as a record being used: since
    // sleepers, the world outranks the store at the door, so a player whose
    // body is still standing is admitted by `Command::Wake` and never reads
    // the record this task just fetched. The sim thread counts it, because
    // the sim thread is where the choice is made (`ShardCore::connect_as`
    // → `Admitted`). The comment this replaced had the right principle — a
    // counter that reports restores which never reached a world is worse
    // than no counter — and this is that principle applied one seam later.

    let welcome = Welcome {
        player_id: id,
        seed: facts.seed,
        tick: ShardStats::get(&stats.current_tick) as u32,
        dev: facts.dev,
        blight_salt: facts.blight_salt,
    };

    tokio::spawn(reader_task(
        connection.clone(),
        input_tx,
        voice_tx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
        facts.netsim,
    ));
    // The bidi recv half stays open for the connection's life: after the
    // hello it is the C→S action lane (craft requests) and the chat lane,
    // demultiplexed by kind.
    tokio::spawn(action_reader_task(
        recv,
        action_tx,
        chat_tx,
        refresh_tx.clone(),
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
    // The bidi send half stays open for the connection's life: after the
    // welcome it becomes the reliable event lane (protocol::event). The
    // welcome itself is written there too, never here (`write_opening`).
    let (s, st) = (slots.clone(), stats.clone());
    tokio::spawn(async move {
        let mut send = send;
        if write_opening(&mut send, &welcome, None).await.is_err() {
            ShardStats::bump(&st.handshake_errors);
            s.mark_leaving(slot, generation);
            return;
        }
        event_writer_task(send, ev_rx, s, st, slot, generation).await;
    });
    tokio::spawn(writer_task(
        connection,
        snap_rx,
        voice_out_rx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
        facts.netsim,
    ));
    Some((slot, generation))
}

// ---------------------------------------------------------------------------
// Spectator seats (`NETCODE.md` §2.3)
// ---------------------------------------------------------------------------

/// Which player a watcher gets, or the refusal code it earns.
///
/// Pure over the accept loop's two tables so the whole door — consent, the
/// target naming, both caps — is reachable by a test without a socket
/// (`accept_chat`'s precedent). `want` is the hello's target: an address
/// names one player (matched against the address the shard PROVED, never a
/// claim), and `GUEST` means "any watchable agent", taken in slot order,
/// skipping one whose own seats are full.
///
/// **Eligible** means the tenant is live and consented: it declared
/// `HELLO_AGENT`, or it opted in as a human and this shard runs human feeds
/// (`human_feeds`, which is `spectate_human_delay_s > 0`). A human who opted
/// in on a shard that runs none is refused exactly like a stranger — one
/// code, because "online but private" is itself a fact about a person.
fn pick_target(
    keys: &[KeySlot; MAX_PLAYERS],
    live: impl Fn(usize) -> Option<u32>,
    seats: &[SeatSlot; MAX_SPECTATORS],
    want: protocol::Address,
    human_feeds: bool,
    per_target: usize,
) -> Result<usize, u8> {
    let mut full = false;
    for (t, k) in keys.iter().enumerate() {
        // The tenant is this slot's CURRENT one: live, and the generation
        // it was installed under (a slot is reused, and a stale key row
        // must not be mistaken for the player now in it).
        let Some(gen) = live(t) else {
            continue;
        };
        if k.generation != gen {
            continue;
        }
        if !(k.agent || (k.watchable && human_feeds)) {
            continue;
        }
        if !want.is_guest() && k.address != want {
            continue;
        }
        let watching = seats
            .iter()
            .filter(|s| s.occupied && s.target == t && s.target_gen == gen)
            .count();
        if watching >= per_target {
            full = true;
            if want.is_guest() {
                continue; // "any": try the next consenting player
            }
            return Err(protocol::REFUSE_WATCH_FULL);
        }
        return Ok(t);
    }
    Err(if full {
        protocol::REFUSE_WATCH_FULL
    } else {
        protocol::REFUSE_WATCH
    })
}

/// Seat a watcher: pick its target, claim a seat slot, build its rings, hand
/// the sim its ends, then welcome it as the target's body and tell it whose.
/// Every refusal is posted, never a hang (DESIGN.md §5.9) — `install`'s shape
/// with the player half taken out, and not `async` for the same reason.
#[allow(clippy::too_many_arguments)]
fn install_spectator(
    done: Handshaken,
    source: Option<std::net::IpAddr>,
    facts: &ShardFacts,
    ctrl_tx: &mut rtrb::Producer<Connect>,
    keys: &[KeySlot; MAX_PLAYERS],
    seats: &mut [SeatSlot; MAX_SPECTATORS],
    slots: &Arc<SlotTable>,
    stats: &Arc<ShardStats>,
) {
    let Handshaken {
        connection,
        send,
        recv,
        hello,
        ..
    } = done;
    let live = |t: usize| {
        let w = slots.load(t);
        (state_of(w) == SLOT_LIVE).then(|| generation_of(w))
    };
    let target = match pick_target(
        keys,
        live,
        seats,
        hello.target,
        facts.spectate.human_delay_s > 0,
        facts.spectate.per_target,
    ) {
        Ok(t) => t,
        Err(code) => {
            let counter = if code == protocol::REFUSE_WATCH_FULL {
                &stats.spectate_full
            } else {
                &stats.spectate_refused
            };
            ShardStats::bump(counter);
            spawn_refusal(connection, send, code);
            return;
        }
    };
    if source.is_some_and(|ip| watch_seats_held(seats, ip) >= WATCH_SEATS_PER_SOURCE) {
        ShardStats::bump(&stats.spectate_full);
        spawn_refusal(connection, send, protocol::REFUSE_WATCH_FULL);
        return;
    }
    // A seat, within the shard's configured count (`spectate_seats` may be
    // below `MAX_SPECTATORS`; the tables are sized to the ceiling).
    let Some((i, generation)) =
        (0..facts.spectate.seats).find_map(|i| slots.claim(MAX_PLAYERS + i).map(|g| (i, g)))
    else {
        ShardStats::bump(&stats.spectate_full);
        spawn_refusal(connection, send, protocol::REFUSE_WATCH_FULL);
        return;
    };
    let slot = MAX_PLAYERS + i;
    let target_gen = generation_of(slots.load(target));
    let k = &keys[target];
    // An agent's feed is live; a human's rides the shard's delay. Decided
    // here, once, from what the target declared — never from the watcher.
    let delay = (!k.agent).then(|| Duration::from_secs(facts.spectate.human_delay_s as u64));

    let (input_tx, input_rx) = RingBuffer::new(INPUT_RING_CAP);
    // A watcher has no reliable C→S lane: these two rings exist because
    // `Link` names them, and nothing ever pushes into either.
    let (_action_tx, action_rx) = RingBuffer::<ActionMsg>::new(1);
    let (_chat_tx, chat_rx) = RingBuffer::<ChatMsg>::new(1);
    // Nor does it talk; it does hear, when it watches an agent
    // (`ShardCore::voice_reach`).
    let (_voice_tx, voice_rx) = RingBuffer::<protocol::VoiceFrame>::new(1);
    let (voice_out_tx, voice_out_rx) = RingBuffer::<VoiceMsg>::new(VOICE_OUT_RING_CAP);
    let (snap_tx, snap_rx) = RingBuffer::<SnapMsg>::new(SNAPSHOT_RING_CAP);
    let (ev_tx, ev_rx) = RingBuffer::<EvMsg>::new(EVENT_RING_CAP);
    let link = Link {
        generation,
        input: input_rx,
        actions: action_rx,
        chats: chat_rx,
        voice_in: voice_rx,
        voice_out: voice_out_tx,
        snaps: snap_tx,
        events: ev_tx,
    };
    if ctrl_tx
        .push(Connect {
            slot,
            id: k.id,
            save: None,
            key: None,
            alias: None,
            watch: Some(target),
            said: crate::slot::Said::default(),
            agent: false,
            name: protocol::Name::EMPTY,
            link,
        })
        .is_err()
    {
        slots.unclaim(slot, generation);
        ShardStats::bump(&stats.spectate_full);
        spawn_refusal(connection, send, protocol::REFUSE_WATCH_FULL);
        return;
    }
    seats[i] = SeatSlot {
        occupied: true,
        conn: Some(connection.clone()),
        generation,
        target,
        target_gen,
        source,
    };

    // Welcomed AS the target's body: `player_id` is whose view this is, which
    // is the one field a client core keys its own-facts on.
    let welcome = Welcome {
        player_id: k.id,
        seed: facts.seed,
        tick: ShardStats::get(&stats.current_tick) as u32,
        dev: facts.dev,
        blight_salt: facts.blight_salt,
    };
    let watch = protocol::Watch {
        address: k.address,
        agent: k.agent,
        name: k.name,
    };

    tokio::spawn(spectator_reader_task(
        connection.clone(),
        input_tx,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
    tokio::spawn(spectator_stream_task(
        recv,
        slots.clone(),
        stats.clone(),
        slot,
        generation,
    ));
    let (s, st) = (slots.clone(), stats.clone());
    match delay {
        None => {
            tokio::spawn(async move {
                let mut send = send;
                if write_opening(&mut send, &welcome, Some(&watch))
                    .await
                    .is_err()
                {
                    ShardStats::bump(&st.handshake_errors);
                    s.mark_leaving(slot, generation);
                    return;
                }
                event_writer_task(send, ev_rx, s, st, slot, generation).await;
            });
            tokio::spawn(writer_task(
                connection,
                snap_rx,
                voice_out_rx,
                slots.clone(),
                stats.clone(),
                slot,
                generation,
                None,
            ));
        }
        Some(delay) => {
            // A delayed feed hears nothing live (`ShardCore::voice_reach`).
            drop(voice_out_rx);
            tokio::spawn(async move {
                let mut send = send;
                if write_opening(&mut send, &welcome, Some(&watch))
                    .await
                    .is_err()
                {
                    ShardStats::bump(&st.handshake_errors);
                    s.mark_leaving(slot, generation);
                    return;
                }
                delayed_writer_task(
                    connection, send, snap_rx, ev_rx, s, st, slot, generation, delay,
                )
                .await;
            });
        }
    }
}

/// The welcome, and a watcher's `Watch` after it, written on the
/// connection's own task and **never on the accept loop**. `write_all` waits
/// for stream credit that only the client grants, so a client that shrinks
/// its receive window and never reads held this write — and with it the
/// whole door: no joins, kicks, bans, save drains or shutdown — for good,
/// keep-alives keeping the connection itself alive. Bounded like the
/// challenge; a client that will not take its welcome in that time is sent
/// away, and the accept loop's sweep closes its connection (`end_session`).
async fn write_opening(
    send: &mut SendStream,
    welcome: &Welcome,
    watch: Option<&protocol::Watch>,
) -> Result<(), ()> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        write_welcome(send, welcome).await?;
        if let Some(watch) = watch {
            write_watch(send, watch).await?;
        }
        Ok(())
    })
    .await
    .unwrap_or(Err(()))
}

async fn write_watch(send: &mut SendStream, msg: &protocol::Watch) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = protocol::encode_watch(msg, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

/// A watcher's datagram, accepted or refused — `accept_input`'s wiring for a
/// seat, split out for the same reason: reachable by a test with no socket.
///
/// Only the acks are wanted (they are how the shard keeps delta-coding this
/// seat's snapshots), so a datagram carrying a frame is refused whole and
/// counted: our client never builds one in watch mode, so it is forged, and
/// refusing the datagram rather than stripping the frame keeps a forger from
/// learning anything by the difference. Never a disconnect — the datagram
/// lane's policy is loss, not framing trust.
fn accept_spectator_input(
    dg: &[u8],
    input_tx: &mut rtrb::Producer<protocol::InputDatagram>,
    stats: &ShardStats,
) {
    ShardStats::add_msg(&stats.net_dg_in_count, &stats.net_dg_in_bytes, dg.len());
    let Ok(decoded) = decode_input(dg) else {
        ShardStats::bump(&stats.input_dg_bad);
        return;
    };
    if !decoded.frames().is_empty() {
        ShardStats::bump(&stats.spectate_input_refused);
        return;
    }
    if input_tx.push(decoded).is_err() {
        ShardStats::bump(&stats.input_ring_drops);
    }
}

async fn spectator_reader_task(
    connection: Connection,
    mut input_tx: rtrb::Producer<protocol::InputDatagram>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    while let Ok(dg) = connection.receive_datagram().await {
        accept_spectator_input(&dg, &mut input_tx, &stats);
    }
    slots.mark_leaving(slot, generation);
}

/// A watcher's C→S stream. **There is no reliable lane for a spectator**:
/// the first frame on it — an action, a chat line, anything — is counted and
/// ends the seat, because a read-only session that writes is a forged
/// client. The stream closing ends the seat too, which is how a watcher that
/// simply leaves is noticed.
async fn spectator_stream_task(
    mut recv: RecvStream,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    if read_frame(&mut recv).await.is_some() {
        ShardStats::bump(&stats.spectate_actions_refused);
    }
    slots.mark_leaving(slot, generation);
}

/// A FIFO of messages each due at a wall-clock instant — the human feed's
/// delay (`config::Spectate::human_delay_s`). Bounded (`cap`): the caller
/// states what overflow means for its lane. Pure over the instants it is
/// handed, so its arithmetic is testable without waiting on a clock.
struct DelayLine<T> {
    q: std::collections::VecDeque<(tokio::time::Instant, T)>,
    cap: usize,
}

impl<T> DelayLine<T> {
    fn new(cap: usize) -> Self {
        Self {
            q: std::collections::VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Queue `msg` due at `due`. `Err(msg)` when full — the caller decides.
    fn push(&mut self, due: tokio::time::Instant, msg: T) -> Result<(), T> {
        if self.q.len() >= self.cap {
            return Err(msg);
        }
        self.q.push_back((due, msg));
        Ok(())
    }

    /// The oldest message due by `now`, if any. Dues are pushed in order
    /// (each is enqueue time plus one fixed delay), so the head is always
    /// the next due.
    fn pop_due(&mut self, now: tokio::time::Instant) -> Option<T> {
        if self.q.front().is_some_and(|(due, _)| *due <= now) {
            return self.q.pop_front().map(|(_, m)| m);
        }
        None
    }

    fn drop_oldest(&mut self) {
        self.q.pop_front();
    }
}

/// Snapshot datagrams a delay line holds per second of delay: every part
/// of one per tick at the snapshot cadence, plus a second's slack.
const DELAY_SNAPS_PER_S: usize = TICK_HZ as usize * sim_core::limits::SNAPSHOT_PARTS;
/// Event-lane messages a delay line holds per second of delay — the drips
/// send at most a handful per tick, so eight a tick is a generous bound, and
/// a watcher that outruns it is closed rather than shown a gap.
const DELAY_EVENTS_PER_S: usize = 8 * TICK_HZ as usize;

/// Both of a delayed seat's output lanes, one task (`NETCODE.md` §2.3): every
/// snapshot and every event the sim hands this seat is held for `delay`
/// before it goes out, so a human who opted in to being watched is never
/// watched live. Snapshots keep their superseding rule after the delay (only
/// the newest due one is sent) and drop their oldest when the line is full;
/// events never drop — a full event line ends the seat, the reliable lane's
/// own policy. The acks this seat sends back are `delay` stale, so the shard
/// falls back to zero-state snapshots for it: correct, and more bytes, which
/// is the price of the delay and one reason human feeds are off by default.
/// A writer's wake: the sim's next published tick (`SlotTable::publish`,
/// carried by [`wake_relay`]), or [`WRITER_FALLBACK`] with none.
struct Wake {
    rx: tokio::sync::watch::Receiver<u64>,
    fallback: tokio::time::Interval,
}

impl Wake {
    fn new(slots: &SlotTable) -> Self {
        let mut fallback = tokio::time::interval(WRITER_FALLBACK);
        fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self {
            rx: slots.subscribe(),
            fallback,
        }
    }

    /// The first call returns at once, so a writer drains what was pushed
    /// before it started.
    async fn next(&mut self) {
        tokio::select! {
            r = self.rx.changed() => {
                if r.is_err() {
                    self.fallback.tick().await;
                }
            }
            _ = self.fallback.tick() => {}
        }
        self.fallback.reset();
    }
}

/// Carries the sim's published ticks to every writer: one task polling one
/// atomic, so the sim thread wakes nobody itself (it takes no lock, L3) and
/// the writers sleep until there is something to send.
async fn wake_relay(slots: Arc<SlotTable>) {
    let mut poll = tokio::time::interval(WAKE_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut seen = 0;
    loop {
        poll.tick().await;
        let n = slots.published();
        if n != seen {
            seen = n;
            slots.wake_writers(n);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn delayed_writer_task(
    connection: Connection,
    mut send: SendStream,
    mut snap_rx: rtrb::Consumer<SnapMsg>,
    mut ev_rx: rtrb::Consumer<EvMsg>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
    delay: Duration,
) {
    let secs = delay.as_secs() as usize + 1;
    let mut snaps = DelayLine::<SnapMsg>::new(secs * DELAY_SNAPS_PER_S);
    let mut events = DelayLine::<EvMsg>::new(secs * DELAY_EVENTS_PER_S);
    let mut wake = Wake::new(&slots);
    loop {
        wake.next().await;
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            return;
        }
        let now = tokio::time::Instant::now();
        while let Ok(msg) = snap_rx.pop() {
            if let Err(msg) = snaps.push(now + delay, msg) {
                snaps.drop_oldest();
                ShardStats::bump(&stats.snap_ring_skips);
                let _ = snaps.push(now + delay, msg);
            }
        }
        while let Ok(msg) = ev_rx.pop() {
            if events.push(now + delay, msg).is_err() {
                ShardStats::bump(&stats.ev_send_errors);
                slots.mark_leaving(slot, generation);
                return;
            }
        }
        let mut newest = crate::slot::TickGroup::default();
        while let Some(msg) = snaps.pop_due(now) {
            newest.push(msg);
        }
        let mut gone = false;
        for msg in newest.take() {
            if !send_snap(&connection, &msg, &stats) {
                gone = true;
                break;
            }
        }
        if gone {
            break;
        }
        while let Some(msg) = events.pop_due(now) {
            let n = FRAME_PREFIX_BYTES + msg.bytes().len();
            if write_frame(&mut send, msg.bytes()).await.is_err() {
                ShardStats::bump(&stats.ev_send_errors);
                slots.mark_leaving(slot, generation);
                return;
            }
            ShardStats::add_msg(&stats.net_stream_out_frames, &stats.net_stream_out_bytes, n);
        }
    }
    slots.mark_leaving(slot, generation);
}

/// The input half of the datagram lane, split out of the async task the
/// way `accept_chat` is: the **wiring** — peek, decode, the domain
/// refusal, then the ring, and which counter each exit moves — is
/// reachable by a test without a socket.
///
/// The domain refusal is NOW.md §5b's: `buttons` is a full octet on the
/// wire and the sim means only the bits in `BTN_MASK`, so a frame carrying
/// an unknown bit is a value no client of this version can have built —
/// forged, not mistyped. The whole datagram is refused, exactly as decode
/// already refuses a forged `sel` (`Malformed` refuses the datagram, not
/// the frame), **before** anything reaches the ring: the refusal is
/// ordered ahead of every mutation, so a forged head cannot smuggle a
/// valid tail into the sim (the item-move trap's lesson, applied here).
/// Counted (`input_dg_forged`), dropped, never a disconnect — the datagram
/// lane's loss policy is redundancy, not framing trust.
fn accept_input(
    dg: &[u8],
    input_tx: &mut rtrb::Producer<protocol::InputDatagram>,
    voice: &mut VoiceIn,
    stats: &ShardStats,
) {
    // **First, before any verdict on it.** These bytes crossed the path
    // whatever the decoder goes on to think of them, and a bandwidth number
    // that only counted the datagrams we liked would read lowest exactly
    // when a shard is being fed garbage (NOW.md §0q item 4).
    ShardStats::add_msg(&stats.net_dg_in_count, &stats.net_dg_in_bytes, dg.len());
    let kind = peek_kind(dg).ok();
    if kind == Some(protocol::KIND_VOICE) {
        voice.accept(dg, stats);
        return;
    }
    let ok = kind == Some(KIND_INPUT);
    if !ok {
        ShardStats::bump(&stats.input_dg_bad);
        return;
    }
    match decode_input(dg) {
        Ok(decoded) => {
            if decoded.frames().iter().any(|f| f.buttons & !BTN_MASK != 0) {
                ShardStats::bump(&stats.input_dg_forged);
                return;
            }
            ShardStats::bump(&stats.input_dg_ok);
            if input_tx.push(decoded).is_err() {
                // Ring full: drop newest — the next datagram
                // re-carries the unacked tail (limits.rs).
                ShardStats::bump(&stats.input_ring_drops);
            }
        }
        Err(_) => ShardStats::bump(&stats.input_dg_bad),
    }
}

/// A talker's voice frames on their way in (v113): decoded, rate-limited
/// by a token bucket of [`VOICE_FPS`] a second and [`VOICE_BURST`] deep,
/// and ringed for the sim thread's range check. Every refusal is counted
/// and none is a disconnect: voice is the datagram lane's, and a lost
/// frame is the codec's to conceal.
struct VoiceIn {
    tx: rtrb::Producer<protocol::VoiceFrame>,
    tokens: u32,
    last: Option<Instant>,
}

impl VoiceIn {
    fn new(tx: rtrb::Producer<protocol::VoiceFrame>) -> Self {
        Self {
            tx,
            tokens: VOICE_BURST,
            last: None,
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        let last = *self.last.get_or_insert(now);
        let per = Duration::from_secs(1) / VOICE_FPS;
        let steps = (now.saturating_duration_since(last).as_nanos() / per.as_nanos()) as u64;
        if steps >= VOICE_BURST as u64 {
            self.tokens = VOICE_BURST;
            self.last = Some(now);
        } else if steps > 0 {
            self.tokens = (self.tokens + steps as u32).min(VOICE_BURST);
            self.last = Some(last + per * steps as u32);
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }

    fn accept(&mut self, dg: &[u8], stats: &ShardStats) {
        let Ok(frame) = protocol::decode_voice_up(dg) else {
            ShardStats::bump(&stats.voice_bad);
            return;
        };
        if !self.allow(Instant::now()) {
            ShardStats::bump(&stats.voice_rate_limited);
            return;
        }
        if self.tx.push(frame).is_err() {
            ShardStats::bump(&stats.voice_ring_drops);
            return;
        }
        ShardStats::bump(&stats.voice_ok);
    }
}

#[allow(clippy::too_many_arguments)]
async fn reader_task(
    connection: Connection,
    mut input_tx: rtrb::Producer<protocol::InputDatagram>,
    voice_tx: rtrb::Producer<protocol::VoiceFrame>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
    netsim: Option<crate::config::NetSim>,
) {
    if let Some(ns) = netsim {
        // The dev fake-network shim (`config.rs netsim`), ingress half: a
        // FIFO delay line between the socket and the sim's input ring, so
        // the netcode can be FELT at the RTT it is tuned for without
        // leaving loopback. Lives entirely on the tokio side — the sim
        // thread sees ordinary late datagrams and nothing else, so walls
        // 1–3 are untouched. Bounded (`NETSIM_QUEUE`, drop-newest at the
        // line, counted): a dev tool still states its overflow policy.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(
            tokio::time::Instant,
            wtransport::datagram::Datagram,
        )>(NETSIM_QUEUE);
        {
            let stats = stats.clone();
            let mut voice = VoiceIn::new(voice_tx);
            tokio::spawn(async move {
                while let Some((due, dg)) = rx.recv().await {
                    tokio::time::sleep_until(due).await;
                    accept_input(&dg, &mut input_tx, &mut voice, &stats);
                }
            });
        }
        let mut rng = netsim_rng(&ns, slot);
        while let Ok(dg) = connection.receive_datagram().await {
            match netsim_verdict(&ns, &mut rng) {
                None => ShardStats::bump(&stats.netsim_dropped),
                Some(delay) => {
                    ShardStats::bump(&stats.netsim_delayed);
                    let due = tokio::time::Instant::now() + delay;
                    if tx.try_send((due, dg)).is_err() {
                        ShardStats::bump(&stats.netsim_dropped);
                    }
                }
            }
        }
        slots.mark_leaving(slot, generation);
        return;
    }
    let mut voice = VoiceIn::new(voice_tx);
    while let Ok(dg) = connection.receive_datagram().await {
        accept_input(&dg, &mut input_tx, &mut voice, &stats);
    }
    slots.mark_leaving(slot, generation);
}

/// The delay line's bound, in datagrams. At 30 Hz per lane even the knob's
/// 2 s latency ceiling holds ~60 in flight, so 64 is the honest working
/// depth rather than a guess; past it the newest is dropped and counted
/// (`netsim_dropped`) — freshness-over-completeness, the lane's own policy.
const NETSIM_QUEUE: usize = 64;

/// One datagram's fate under the shim: `None` ⇒ the loss roll took it;
/// `Some(delay)` ⇒ deliver after `lat ± jitter` (clamped at zero). Seeded
/// off the knob and the slot (`netsim_rng`), so a "feels wrong at 30,10,1"
/// report reproduces exactly.
fn netsim_verdict(
    ns: &crate::config::NetSim,
    rng: &mut sim_core::rng::Pcg32,
) -> Option<std::time::Duration> {
    if ns.loss_pct > 0 && rng.next_u32() % 100 < ns.loss_pct as u32 {
        return None;
    }
    let jitter = if ns.jitter_ms > 0 {
        rng.next_bounded(2 * ns.jitter_ms + 1) as i64 - ns.jitter_ms as i64
    } else {
        0
    };
    let ms = (ns.lat_ms as i64 + jitter).max(0) as u64;
    Some(std::time::Duration::from_millis(ms))
}

fn netsim_rng(ns: &crate::config::NetSim, slot: usize) -> sim_core::rng::Pcg32 {
    let key = ((ns.lat_ms as u64) << 40) | ((ns.jitter_ms as u64) << 8) | ns.loss_pct as u64;
    sim_core::rng::Pcg32::new(0x4E45_5453_494D ^ key, slot as u64)
}

/// Reads length-prefixed C→S frames off the bidi stream for the
/// connection's life and demultiplexes them by kind: actions to the
/// action ring, chat to the chat ring.
///
/// The two lanes have deliberately opposite pressure policies. A full
/// **action** ring backpressures: the task stops reading until the sim
/// drains, and QUIC flow control holds the client (limits.rs
/// `ACTION_RING_CAP` — the reliable lane never drops a transaction). A
/// full **chat** ring drops the line, because backpressuring the shared
/// stream on chat would stall the sender's own transactions behind their
/// own typing — the spammer's punishment would land on their building.
///
/// An action frame that fails to decode drops the session: framing trust
/// is gone. A *chat* frame that fails to decode does not — chat text is
/// the one payload a client bug can plausibly malform, and it is counted
/// and swallowed instead (`chat_bad`). Neither ever panics.
/// The chat half of that demux, split out of the async task so the
/// **wiring** — decode, then limiter, then ring, then which counter moves
/// — is reachable by a test without a socket. A limiter that is never
/// called is a limiter that stopped limiting, and that is not something a
/// unit test of the bucket alone can notice.
fn accept_chat(
    frame: &[u8],
    limiter: &mut ChatLimiter,
    now: Instant,
    chat_tx: &mut rtrb::Producer<ChatMsg>,
    stats: &ShardStats,
) {
    let Ok(chat) = decode_chat(frame) else {
        ShardStats::bump(&stats.chat_bad);
        return;
    };
    if !limiter.allow(now) {
        ShardStats::bump(&stats.chat_rate_limited);
        return;
    }
    if chat_tx.push(chat).is_err() {
        ShardStats::bump(&stats.chat_ring_drops);
        return;
    }
    // Counted only once it is actually ringed — `chat_ok` says "decoded
    // and ringed", so a dropped line must not also read as an accepted
    // one.
    ShardStats::bump(&stats.chat_ok);
}

#[allow(clippy::too_many_arguments)]
async fn action_reader_task(
    mut recv: RecvStream,
    mut action_tx: rtrb::Producer<ActionMsg>,
    mut chat_tx: rtrb::Producer<ChatMsg>,
    refresh_tx: tokio::sync::mpsc::Sender<(usize, u32)>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    let mut limiter = ChatLimiter::new();
    // A client hammering "refresh my skins" is answered at most once a
    // second from here, before it can crowd the accept loop's bounded queue
    // that every other player's refresh shares; the accept loop's own
    // cooldown (`skins::REFRESH_COOLDOWN`) then decides whether to read.
    let mut last_refresh: Option<Instant> = None;
    loop {
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            return; // slot moved on; sim (or another task) already knows
        }
        let Some((buf, len)) = read_frame(&mut recv).await else {
            break; // stream closed or oversize frame: the session is done
        };
        // Before the demux: an action, a chat line and a chat line the
        // limiter is about to refuse all cost the same bytes.
        ShardStats::add_msg(
            &stats.net_stream_in_frames,
            &stats.net_stream_in_bytes,
            FRAME_PREFIX_BYTES + len,
        );
        if peek_kind(&buf[..len]) == Ok(KIND_CHAT) {
            accept_chat(
                &buf[..len],
                &mut limiter,
                Instant::now(),
                &mut chat_tx,
                &stats,
            );
            continue;
        }
        match decode_action(&buf[..len]) {
            // "I bought something": answered here, by the accept loop's
            // platform read, and never by the sim (`skins.rs`). A full
            // channel drops the ask; the cooldown would have anyway.
            Ok(ActionMsg::SkinsRefresh) => {
                ShardStats::bump(&stats.actions_ok);
                let now = Instant::now();
                if last_refresh.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1)) {
                    last_refresh = Some(now);
                    let _ = refresh_tx.try_send((slot, generation));
                }
            }
            Ok(mut act) => {
                ShardStats::bump(&stats.actions_ok);
                loop {
                    match action_tx.push(act) {
                        Ok(()) => break,
                        Err(rtrb::PushError::Full(back)) => {
                            act = back;
                            let word = slots.load(slot);
                            if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
                                return;
                            }
                            tokio::time::sleep(RING_RETRY).await;
                        }
                    }
                }
            }
            Err(_) => {
                ShardStats::bump(&stats.actions_bad);
                break;
            }
        }
    }
    slots.mark_leaving(slot, generation);
}

#[allow(clippy::too_many_arguments)]
async fn writer_task(
    connection: Connection,
    mut snap_rx: rtrb::Consumer<SnapMsg>,
    mut voice_rx: rtrb::Consumer<VoiceMsg>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
    netsim: Option<crate::config::NetSim>,
) {
    // The dev fake-network shim, egress half (the ingress twin is in
    // `reader_task`): snapshots detour through a FIFO delay line before
    // the one real `send_datagram`. The drain-to-newest below still runs
    // first, so what gets delayed is exactly what would have been sent.
    let mut netsim_lane = netsim.map(|ns| {
        let (tx, mut rx) =
            tokio::sync::mpsc::channel::<(tokio::time::Instant, SnapMsg)>(NETSIM_QUEUE);
        let conn = connection.clone();
        let st = stats.clone();
        tokio::spawn(async move {
            while let Some((due, msg)) = rx.recv().await {
                tokio::time::sleep_until(due).await;
                if !send_snap(&conn, &msg, &st) {
                    return;
                }
            }
        });
        (ns, netsim_rng(&ns, slot ^ 0x80), tx)
    });
    let mut wake = Wake::new(&slots);
    // The dev shim's two delay lines, at their worst, which quinn's round
    // trip never sees: added back so a netsim player keeps the rewind its
    // shaped link needs (`ClientNetState::rtt_bounded_view`).
    let shim_us = netsim.map_or(0, |ns| 2_000 * u64::from(ns.lat_ms + ns.jitter_ms));
    // Per-connection previous readings, so what reaches `ShardStats` is a
    // delta. Local to the task on purpose: a shared table keyed by
    // connection would be a map with a lifetime problem and a lock, to hold
    // two numbers that only this task ever reads.
    // **Now, not `NET_SAMPLE_EVERY` out**: the first wake samples. Starting
    // a full period out would mean a connection that lives under a second
    // contributes nothing at all — and a shard being hammered by clients
    // that connect and die is exactly when the loss counters matter most.
    // The first sample's `prev_*` are zero, so it correctly reports
    // everything since the connection opened.
    let mut sample_at = tokio::time::Instant::now();
    let (mut prev_lost, mut prev_sent, mut prev_cong, mut prev_holes) = (0u64, 0u64, 0u64, 0u64);
    loop {
        wake.next().await;
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            return; // slot moved on; sim (or reader) already knows
        }
        // This connection's round trip, every wake: the sim thread holds its
        // acks to it (`ShardStats::slot_rtt_us`), so it must track a link
        // that worsens within the tick, not the second.
        if slot < MAX_PLAYERS {
            let rtt = connection.rtt().as_micros() as u64;
            stats.slot_rtt_us.store(slot, rtt.max(1) + shim_us);
        }
        // Transport telemetry (NETCODE.md §2.2). Reads quinn's own counters
        // through wtransport's passthrough — no wire field, no PROTO_VER,
        // no golden: the numbers already exist and nothing was asking for
        // them.
        let now = tokio::time::Instant::now();
        if now >= sample_at {
            sample_at = now + NET_SAMPLE_EVERY;
            let s = connection.quic_connection().stats();
            let p = s.path;
            // `saturating_sub` rather than `-`: these are monotonic per
            // connection, but a reset path or a stat that goes backwards
            // under us must not underflow a u64 into billions.
            ShardStats::add(
                &stats.net_lost_packets,
                p.lost_packets.saturating_sub(prev_lost),
            );
            ShardStats::add(
                &stats.net_sent_packets,
                p.sent_packets.saturating_sub(prev_sent),
            );
            ShardStats::add(
                &stats.net_congestion_events,
                p.congestion_events.saturating_sub(prev_cong),
            );
            ShardStats::add(
                &stats.net_black_holes,
                p.black_holes_detected.saturating_sub(prev_holes),
            );
            prev_lost = p.lost_packets;
            prev_sent = p.sent_packets;
            prev_cong = p.congestion_events;
            prev_holes = p.black_holes_detected;
            stats
                .net_rtt_us_max
                .fetch_max(p.rtt.as_micros() as u64, Ordering::Relaxed);
            stats
                .net_mtu_last
                .store(p.current_mtu as u64, Ordering::Relaxed);
        }
        // Voice goes out whole and in order: unlike a snapshot, a frame
        // is not superseded by the next one (v113). `send_datagram` drops
        // the oldest itself when the path is congested.
        while let Ok(v) = voice_rx.pop() {
            match connection.send_datagram(v.bytes()) {
                Ok(()) => ShardStats::add_msg(
                    &stats.net_dg_out_count,
                    &stats.net_dg_out_bytes,
                    v.bytes().len(),
                ),
                Err(SendDatagramError::NotConnected) => break,
                Err(_) => ShardStats::bump(&stats.voice_out_drops),
            }
        }
        // Drain to the newest tick — snapshots replace, never accumulate —
        // and send every datagram of it: a snapshot and its overflow
        // (wire v121) are one tick.
        let mut newest = crate::slot::TickGroup::default();
        while let Ok(msg) = snap_rx.pop() {
            newest.push(msg);
        }
        let mut gone = false;
        for msg in newest.take() {
            if let Some((ns, rng, tx)) = netsim_lane.as_mut() {
                match netsim_verdict(ns, rng) {
                    None => ShardStats::bump(&stats.netsim_dropped),
                    Some(delay) => {
                        ShardStats::bump(&stats.netsim_delayed);
                        let due = tokio::time::Instant::now() + delay;
                        if tx.try_send((due, msg)).is_err() {
                            ShardStats::bump(&stats.netsim_dropped);
                        }
                    }
                }
                continue;
            }
            if !send_snap(&connection, &msg, &stats) {
                gone = true;
                break;
            }
        }
        if gone {
            break;
        }
    }
    slots.mark_leaving(slot, generation);
}

/// The one real snapshot send, factored so the netsim delay line and the
/// direct path cannot drift: the live-MTU clamp (the trap list: oversize
/// sends fail, and in a browser they failed silently), and the counters —
/// on `Ok` and nowhere else, because the socket accepting the datagram is
/// the event, not our intent to hand it one. `false` ⇒ the connection is
/// gone and the caller should stop.
fn send_snap(connection: &Connection, msg: &SnapMsg, stats: &ShardStats) -> bool {
    let max = connection.max_datagram_size().unwrap_or(0);
    if msg.bytes().len() > max {
        ShardStats::bump(&stats.snap_send_errors);
        return true;
    }
    let n = msg.bytes().len();
    match connection.send_datagram(msg.bytes()) {
        Ok(()) => {
            ShardStats::add_msg(&stats.net_dg_out_count, &stats.net_dg_out_bytes, n);
            true
        }
        Err(SendDatagramError::NotConnected) => false,
        Err(_) => {
            ShardStats::bump(&stats.snap_send_errors);
            true
        }
    }
}

/// Drains the per-connection event ring onto the reliable bidi stream —
/// in order, every message (this lane is the one that never drops; the
/// bound lives at the ring, whose refusal the sim converts to a resync).
/// A stalled peer parks this task on stream flow control; the keep-alive/
/// idle-timeout pair reaps dead ones.
async fn event_writer_task(
    mut send: SendStream,
    mut ev_rx: rtrb::Consumer<EvMsg>,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    slot: usize,
    generation: u32,
) {
    let mut wake = Wake::new(&slots);
    loop {
        wake.next().await;
        let word = slots.load(slot);
        if state_of(word) != SLOT_LIVE || generation_of(word) != generation {
            // Sent away with a reason: post it as the session's last word,
            // inside the grace `close_with_reason` waits out. A browser
            // cannot read the QUIC close code, so this is how it learns why.
            // `why` matches the whole generation, so this holds even when
            // the slot was freed and claimed again before this writer woke
            // (a takeover's newcomer takes the slot it left).
            if let Some(code) = slots.why(slot, generation) {
                let _ = tokio::time::timeout(REFUSE_POST_TIMEOUT, async {
                    write_refuse(&mut send, code).await?;
                    send.finish().await.map_err(|_| ())
                })
                .await;
            }
            return; // slot moved on; sim (or reader) already knows
        }
        while let Ok(msg) = ev_rx.pop() {
            let n = FRAME_PREFIX_BYTES + msg.bytes().len();
            if write_frame(&mut send, msg.bytes()).await.is_err() {
                ShardStats::bump(&stats.ev_send_errors);
                slots.mark_leaving(slot, generation);
                return;
            }
            // The prefix is on the wire, so it is in the count. A frame
            // that failed half-written is not: `write_all` reports the
            // failure and not how far it got, and a guessed byte is worse
            // than a missing one.
            ShardStats::add_msg(&stats.net_stream_out_frames, &stats.net_stream_out_bytes, n);
        }
    }
}

// ---------------------------------------------------------------------------
// Stream framing (u16 LE length prefix per message)
// ---------------------------------------------------------------------------

/// The length prefix every stream frame carries, in bytes. Named because
/// the byte counters have to include it — it is on the wire — and a `+ 2`
/// at four call sites is a magic number that silently lies the day the
/// prefix widens.
pub const FRAME_PREFIX_BYTES: usize = 2;
const _: () = assert!(FRAME_PREFIX_BYTES == core::mem::size_of::<u16>());

/// One length-prefixed message off a stream: `(buffer, len)`, or None on
/// EOF/oversize (the caller drops the session). The 64 B cap is the
/// server's C→S acceptance — a hello has no business being big.
pub async fn read_frame(recv: &mut RecvStream) -> Option<([u8; MAX_STREAM_MSG_BYTES], usize)> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf).await.ok()?;
    let len = u16::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_STREAM_MSG_BYTES {
        return None;
    }
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    recv.read_exact(&mut buf[..len]).await.ok()?;
    Some((buf, len))
}

/// The auth frame's read: [`read_frame`] at `MAX_AUTH_MSG_BYTES`, the one
/// handshake frame that may be longer (an Elo Pros pass, v151). Read once per
/// join, after the challenge.
pub async fn read_auth_frame(
    recv: &mut RecvStream,
) -> Option<([u8; protocol::MAX_AUTH_MSG_BYTES], usize)> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf).await.ok()?;
    let len = u16::from_le_bytes(len_buf) as usize;
    if len == 0 || len > protocol::MAX_AUTH_MSG_BYTES {
        return None;
    }
    let mut buf = [0u8; protocol::MAX_AUTH_MSG_BYTES];
    recv.read_exact(&mut buf[..len]).await.ok()?;
    Some((buf, len))
}

/// The client-side read: one S→C event-lane frame, sized for
/// `MAX_EVENT_MSG_BYTES` (the bot client here; the native client's framer
/// lives in `crates/client/src/lib.rs` with the same cap).
pub async fn read_event_frame(recv: &mut RecvStream) -> Option<([u8; MAX_EVENT_MSG_BYTES], usize)> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf).await.ok()?;
    let len = u16::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_EVENT_MSG_BYTES {
        return None;
    }
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
    recv.read_exact(&mut buf[..len]).await.ok()?;
    Some((buf, len))
}

/// How long a kicked session stays open so its event writer can post the
/// reason (`SlotTable::why`) before the close. Longer than
/// [`REFUSE_POST_TIMEOUT`], so the post finishes or gives up first.
const KICK_GRACE: Duration = Duration::from_millis(400);
/// The most the event writer spends posting a leave reason.
const REFUSE_POST_TIMEOUT: Duration = Duration::from_millis(300);

/// Close a connection the shard is sending away, with the refusal code (the
/// desktop reads it off the close), after [`KICK_GRACE`] so the reason
/// posted on the event lane arrives first. Detached: it never holds up the
/// accept loop.
fn close_with_reason(conn: Connection, code: u8) {
    tokio::spawn(async move {
        tokio::time::sleep(KICK_GRACE).await;
        close_now(&conn, code);
    });
}

/// Close the connection of a session that already ended: with its posted
/// reason after the grace when it has one, at once otherwise (a code
/// outside the refusal table reads as "closed").
fn end_session(conn: Connection, why: Option<u8>) {
    match why {
        Some(code) => close_with_reason(conn, code),
        None => close_now(&conn, u8::MAX),
    }
}

/// Close with the refusal code and its text, which the desktop reads.
fn close_now(conn: &Connection, code: u8) {
    conn.close(
        wtransport::VarInt::from_u32(code as u32),
        protocol::refuse_text(code).unwrap_or("closed").as_bytes(),
    );
}

async fn write_refuse(send: &mut SendStream, code: u8) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_refuse(&Refuse { code }, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

/// Close player `id`'s connection for an admin, with `REFUSE_ADMIN` posted.
/// False ⇒ nobody live holds that id (they left between the verb and this).
///
/// The slot is found by id rather than carried, because the sim named a
/// player and slots are the accept loop's business — and a reconnect between
/// the two must not be kicked in the first one's name.
fn admin_kick(id: u32, keys: &mut [KeySlot; MAX_PLAYERS], slots: &SlotTable) -> bool {
    // The tenant with that id, wallet or guest: a guest griefs as well as
    // anyone, and a kick needs no wallet (a ban does — `core.rs` refuses
    // that one before it reaches here). `kick_tenant` checks it is live.
    let Some(slot) = (0..MAX_PLAYERS).find(|&s| keys[s].id == id && keys[s].conn.is_some()) else {
        return false;
    };
    kick_tenant(slot, keys, slots, protocol::REFUSE_ADMIN)
}

/// End `slot`'s live tenant with `code`: marked leaving (the sim's sweep
/// puts the body to sleep) and closed with the reason posted. False when the
/// slot is not live — already leaving, or empty.
fn kick_tenant(
    slot: usize,
    keys: &mut [KeySlot; MAX_PLAYERS],
    slots: &SlotTable,
    code: u8,
) -> bool {
    let word = slots.load(slot);
    if crate::slot::state_of(word) != crate::slot::SLOT_LIVE {
        return false;
    }
    slots.mark_leaving_why(slot, crate::slot::generation_of(word), code);
    if let Some(conn) = keys[slot].conn.take() {
        // Closed with a posted reason, the entitle kick's rule: a silent
        // close reads as a network fault, and "my internet broke" is the
        // wrong thing for a kicked player to believe.
        close_with_reason(conn, code);
    }
    true
}

/// A newcomer waiting for its own wallet's old body to fall asleep (one
/// wallet, one body): the old session has been closed, and once the sim has
/// applied its leave the slot frees and the newcomer is seated — on the
/// sleeper, by `connect_as`'s first door. One handshake and one signature,
/// however the old session ended.
struct Parked {
    done: Handshaken,
    slot: usize,
    generation: u32,
    until: tokio::time::Instant,
}

/// Newcomers that may wait at once, and how long one waits before it is
/// refused `REFUSE_IN_USE` after all. A leave lands within a tick or two of
/// the close; the wait only runs long for a body the sim cannot let go of.
const PARK_MAX: usize = 16;
const PARK_WAIT: Duration = Duration::from_secs(5);
const PARK_POLL: Duration = Duration::from_millis(20);

/// The player slot whose tenant is `key` and still has a body awake: live,
/// or leaving but not yet freed (the sim frees a slot only after it has
/// applied the leave). A stale `KeySlot` from an earlier generation is not
/// a tenant — `install` overwrites them lazily.
fn seated(key: &PlayerKey, keys: &[KeySlot; MAX_PLAYERS], slots: &SlotTable) -> Option<usize> {
    (0..MAX_PLAYERS).find(|&s| {
        let word = slots.load(s);
        keys[s].key.as_ref() == Some(key)
            && crate::slot::state_of(word) != crate::slot::SLOT_EMPTY
            && crate::slot::generation_of(word) == keys[s].generation
    })
}

/// Tell admin `to` how an act went (`admin::AdminReply`). A full ring loses
/// the line, never the act, and counts it like any other undelivered line.
fn answer_admin(
    tx: &mut rtrb::Producer<crate::admin::AdminReply>,
    to: u32,
    line: &str,
    stats: &ShardStats,
) {
    let Some(reply) = crate::admin::AdminReply::new(to, line) else {
        return;
    };
    if tx.push(reply).is_err() {
        ShardStats::bump(&stats.chat_undelivered);
    }
}

/// Refusals posting at once, process-wide (one socket, one set of
/// connections). Past it a refusal is a bare close carrying the code, so a
/// loop of doomed handshakes (a stale build, a banned wallet) cannot hold
/// more connections open than the admission gate lets in.
const REFUSALS_MAX: usize = ADMIT_REFUSE_AT;
/// How long a refusal waits for the peer to ack its frame. A client that
/// read it acks within a round trip.
const REFUSAL_WAIT: Duration = Duration::from_secs(2);

/// The count of refusals posting ([`REFUSALS_MAX`]).
struct RefusalGate(std::sync::atomic::AtomicUsize);

/// One refusal's place in a [`RefusalGate`], given back on drop.
struct RefusalOut(&'static RefusalGate);

impl RefusalGate {
    const fn new() -> Self {
        Self(std::sync::atomic::AtomicUsize::new(0))
    }

    fn take(&'static self) -> Option<RefusalOut> {
        if self.0.fetch_add(1, Ordering::Relaxed) >= REFUSALS_MAX {
            self.0.fetch_sub(1, Ordering::Relaxed);
            return None;
        }
        Some(RefusalOut(self))
    }
}

impl Drop for RefusalOut {
    fn drop(&mut self) {
        self.0 .0.fetch_sub(1, Ordering::Relaxed);
    }
}

static REFUSALS: RefusalGate = RefusalGate::new();

/// Refusals are posted, never hung (DESIGN.md §5.9) — and never block the
/// accept loop. A buffered write dies with the dropped connection, so
/// delivery needs `finish` (retransmit-until-acked) before the drop; that
/// waits on the peer, so it runs detached, bounded by [`REFUSAL_WAIT`] so a
/// non-acking client can't pin the task, and at most [`REFUSALS_MAX`] at
/// once: past that the code rides a bare close.
fn spawn_refusal(connection: Connection, mut send: SendStream, code: u8) {
    let Some(out) = REFUSALS.take() else {
        close_now(&connection, code);
        return;
    };
    tokio::spawn(async move {
        let _out = out; // moved in, so an unpolled task still gives it back
        let _ = tokio::time::timeout(REFUSAL_WAIT, async {
            write_refuse(&mut send, code).await?;
            send.finish().await.map_err(|_| ())
        })
        .await;
        drop(connection);
    });
}

async fn write_challenge(send: &mut SendStream, msg: &protocol::Challenge) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = protocol::encode_challenge(msg, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

async fn write_welcome(send: &mut SendStream, msg: &Welcome) -> Result<(), ()> {
    let mut payload = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_welcome(msg, &mut payload).map_err(|_| ())?;
    write_frame(send, &payload[..len]).await
}

/// **The client half of the handshake, in one place.**
///
/// Hello → read the challenge → answer it → read the welcome. Four steps
/// that have to agree exactly with `handshake_task`, and they are written
/// once because the alternative is what this repo already had: the bot
/// client, three fixtures in `bot_smoke` and the real client each spelling
/// the same exchange out, so a wire turn is five edits and the one that gets
/// missed fails as a hang rather than as a compile error.
///
/// `sign` is asked to sign the SIWE message this shard chose; returning
/// `None` is how a **guest** connects — no address, no signature, admitted
/// only where `require_auth` is off. Bots and every test in this repo are
/// guests, deliberately: giving a load harness a credential would be a lie
/// about what is being measured and a standing reason for somebody to put a
/// real key in a fixture.
pub async fn client_handshake(
    send: &mut SendStream,
    recv: &mut RecvStream,
    domain: &str,
    address: protocol::Address,
    sign: impl FnOnce(&[u8]) -> Option<protocol::Signature>,
) -> Result<Welcome, String> {
    // The bots are this build, so they state this build — which is also what
    // makes them a real exercise of the floor: a shard with `min_client`
    // above this release refuses its own bot fleet, and that is the correct
    // answer rather than a special case. They declare nothing (v73): a load
    // bot is not an agent player and consents to nobody watching it.
    client_handshake_as(
        send,
        recv,
        domain,
        address,
        &protocol::Hello::this_build(),
        |domain, nonce, issued_at| {
            // **The address goes in before the signing, not after**, and the
            // first version of this function got that backwards: it built the
            // text with `Address::GUEST` and then asked for a signature. The
            // server rebuilds the message from the address the client
            // *claims*, so the two texts would have differed by 42 characters
            // and every real login would have been refused as `WrongSigner` —
            // with the crypto, the nonce and the domain binding all correct.
            //
            // Checksummed, because that is what the shard recomputes and what
            // a real launcher signs — see `Address::to_checksum_hex`.
            let mut text = [0u8; protocol::SIWE_MESSAGE_MAX];
            let checksummed = address.to_checksum_hex();
            let addr = core::str::from_utf8(&checksummed).ok()?;
            let tlen =
                protocol::siwe_message(domain, addr, protocol::SLUG, nonce, issued_at, &mut text);
            sign(&text[..tlen.min(protocol::SIWE_MESSAGE_MAX)])
        },
    )
    .await
    .map(|j| j.welcome)
}

/// What a join handed back: the welcome, and — to a spectator only — who
/// it is watching (v73, `protocol::Watch`).
#[derive(Clone, Copy, Debug)]
pub struct Joined {
    pub welcome: Welcome,
    pub watch: Option<protocol::Watch>,
}

/// [`client_handshake`] with the hello spelled out (v73): an agent player
/// declares itself (`HELLO_AGENT`, a name), and a spectator asks for a seat
/// (`HELLO_SPECTATE`, a target). The sequence is the same four steps; a
/// spectator reads one more frame after the welcome, the `Watch`.
///
/// `sign` is handed the three inert values — the domain this client dialled,
/// the shard's nonce, its `Issued At` — and composes nothing it did not
/// build itself: an agent's key signs the `protocol::siwe_message` it
/// composes from them (`agentkey::AgentKey::sign_siwe`), which is the same
/// inversion the desktop client made for the launcher.
pub async fn client_handshake_as(
    send: &mut SendStream,
    recv: &mut RecvStream,
    domain: &str,
    address: protocol::Address,
    hello: &protocol::Hello,
    sign: impl FnOnce(&str, &[u8; protocol::NONCE_BYTES], u64) -> Option<protocol::Signature>,
) -> Result<Joined, String> {
    // The message is built from the domain **this client dialled**, never
    // from anything the server said — that is the whole of SIWE's domain
    // binding, and handing the server the choice would let one shard collect
    // a signature valid at another. The address is a parameter for the
    // reason `client_handshake` states: there is no order in which it can be
    // learned late.
    client_handshake_with(send, recv, hello, |challenge| {
        if address.is_guest() {
            return protocol::Auth::default();
        }
        match sign(domain, &challenge.nonce, challenge.issued_at) {
            Some(signature) => protocol::Auth {
                address,
                signature,
                solana: None,
                pass: None,
            },
            // The signer refused, is not running, or handed the player a
            // consent prompt they declined. That is a *guest*, not an error:
            // a shard that takes guests should still take this one.
            None => protocol::Auth::default(),
        }
    })
    .await
}

/// The handshake's frames, with the auth frame built by `auth_for` from the
/// shard's challenge — [`client_handshake_as`]'s Ethereum proof, or a Solana
/// account's (v142), which is what a test of that path builds here.
pub async fn client_handshake_with(
    send: &mut SendStream,
    recv: &mut RecvStream,
    hello: &protocol::Hello,
    auth_for: impl FnOnce(&protocol::Challenge) -> protocol::Auth,
) -> Result<Joined, String> {
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    let len =
        protocol::encode_hello(hello, &mut buf).map_err(|e| format!("encode hello: {e:?}"))?;
    write_frame(send, &buf[..len])
        .await
        .map_err(|_| "write hello".to_string())?;

    let (frame, n) = read_frame(recv).await.ok_or("no challenge")?;
    // A refusal can arrive here instead of a challenge — a version mismatch
    // is caught before the server ever challenges — so both are handled and
    // the refusal is reported by code rather than as "unexpected".
    match peek_kind(&frame[..n]) {
        Ok(protocol::KIND_REFUSE) => {
            let r = protocol::decode_refuse(&frame[..n]).map_err(|e| format!("refuse: {e:?}"))?;
            return Err(match protocol::refuse_text(r.code) {
                Some(why) => format!("refused: {why}"),
                None => format!("refused: code {}", r.code),
            });
        }
        Ok(protocol::KIND_CHALLENGE) => {}
        other => return Err(format!("expected a challenge, got {other:?}")),
    }
    let challenge =
        protocol::decode_challenge(&frame[..n]).map_err(|e| format!("challenge: {e:?}"))?;
    let auth = auth_for(&challenge);
    // The auth frame alone may carry an Elo Pros pass (v151).
    let mut abuf = [0u8; protocol::MAX_AUTH_MSG_BYTES];
    let len = protocol::encode_auth(&auth, &mut abuf).map_err(|e| format!("encode auth: {e:?}"))?;
    write_frame(send, &abuf[..len])
        .await
        .map_err(|_| "write auth".to_string())?;

    let (frame, n) = read_frame(recv).await.ok_or("no handshake reply")?;
    let welcome = match peek_kind(&frame[..n]) {
        Ok(protocol::KIND_WELCOME) => {
            protocol::decode_welcome(&frame[..n]).map_err(|e| format!("welcome: {e:?}"))?
        }
        Ok(protocol::KIND_REFUSE) => {
            let r = protocol::decode_refuse(&frame[..n]).map_err(|e| format!("refuse: {e:?}"))?;
            return Err(match protocol::refuse_text(r.code) {
                Some(why) => format!("refused: {why}"),
                None => format!("refused: code {}", r.code),
            });
        }
        other => return Err(format!("unexpected handshake reply: {other:?}")),
    };
    if !hello.is_spectator() {
        return Ok(Joined {
            welcome,
            watch: None,
        });
    }
    let (frame, n) = read_frame(recv).await.ok_or("no watch after the welcome")?;
    if peek_kind(&frame[..n]) != Ok(protocol::KIND_WATCH) {
        return Err("a spectator's welcome must be followed by a watch".into());
    }
    let watch = protocol::decode_watch(&frame[..n]).map_err(|e| format!("watch: {e:?}"))?;
    Ok(Joined {
        welcome,
        watch: Some(watch),
    })
}

pub async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<(), ()> {
    let len = (payload.len() as u16).to_le_bytes();
    send.write_all(&len).await.map_err(|_| ())?;
    send.write_all(payload).await.map_err(|_| ())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The sim thread
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn sim_thread(
    seed: u64,
    dev_spawn: Option<(f32, f32)>,
    dev_env: Option<(u8, u16)>,
    dev_arc_hour: Option<u32>,
    tables: SimTables,
    world_blob: Vec<u8>,
    world_idents: crate::worldfile::Identities,
    world_interval: u64,
    world_clamp: bool,
    wipe: crate::wipe::Clock,
    standings_base: Option<std::path::PathBuf>,
    prizes: crate::standings::Prizes,
    exits: crate::standings::Exits,
    motd: Vec<String>,
    mut ctrl_rx: rtrb::Consumer<Connect>,
    mut skins_rx: rtrb::Consumer<crate::slot::SkinsMsg>,
    mut prices_rx: rtrb::Consumer<crate::slot::SkinPricesMsg>,
    mut faces_rx: rtrb::Consumer<crate::slot::FaceMsg>,
    mut grave_tx: rtrb::Producer<Link>,
    mut save_tx: rtrb::Producer<SaveMsg>,
    mut world_tx: rtrb::Producer<WorldMsg>,
    mut world_done_rx: rtrb::Consumer<WorldDone>,
    mut file_tx: rtrb::Producer<FileMsg>,
    mut admin_tx: rtrb::Producer<crate::admin::AdminAct>,
    mut answer_rx: rtrb::Consumer<crate::admin::AdminReply>,
    mut log: crate::anomaly::Sink,
    admins: crate::admin::Admins,
    banned: Vec<PlayerKey>,
    team_max: u8,
    trust: crate::trustlog::Tap,
    companion: crate::companion::Companion,
    scribe: crate::scribe::Scribe,
    slots: Arc<SlotTable>,
    stats: Arc<ShardStats>,
    shutdown: Arc<AtomicBool>,
    ready: tokio::sync::oneshot::Sender<u32>,
) {
    let mut core = ShardCore::new(seed);
    core.world.dev_spawn = dev_spawn;
    let (standing_rules, achievements) = install_tables(&mut core, tables);
    core.install_admins(admins);
    core.install_bans(&banned);
    // The teams, kept beside the world save across a restart.
    core.teams = crate::team::Teams::open(
        team_max,
        standings_base.as_deref().map(crate::team::teams_path),
    );
    // And the ids evicted bodies wore, beside them (`core::EvictIds`).
    core.evict_ids =
        crate::core::EvictIds::open(standings_base.as_deref().map(crate::core::evict_ids_path));
    core.trust = trust;
    // The standings, beside the files the wipe archives with them.
    core.standings = crate::standings::Standings::open(
        standing_rules,
        prizes,
        standings_base.as_deref(),
        wipe.number(),
    );
    core.standings.exits = exits;
    // Achievements: a career, so beside the hall and not in what a wipe moves.
    if !achievements.unresolved.is_empty() {
        eprintln!(
            "shard: achievements naming items the content lacks (never earnable): {}",
            achievements.unresolved.join(", ")
        );
    }
    core.achievements =
        crate::achievements::Achievements::open(achievements, standings_base.as_deref());
    // The skins this shard gave away, beside the hall: the wipe keeps both.
    core.gifts = crate::gifts::Gifts::open(standings_base.as_deref());
    core.mints = crate::mints::Mints::open(standings_base.as_deref());
    core.online = crate::online::Online::open(standings_base.as_deref());
    core.companion = companion;
    core.scribe = scribe;
    core.motd = motd.iter().filter_map(|l| crate::wipe::chat(l)).collect();
    core.wipe = wipe;
    // The counter sweep's memory, beside the sink it feeds (`anomaly.rs`).
    let mut watch = crate::anomaly::Watch::new();
    // **The load, and this is the only place it may happen**: after the
    // content tables above are installed — the decoder range-checks every
    // row against them — and before the first tick, because a loaded world
    // is the origin of a run and not a mutation inside one (`worldsave.rs`
    // has the wall-5 argument in full).
    //
    // A refusal here cannot happen: `bin/shard.rs` already loaded these same
    // bytes into a trial world before it bound a port, and refused the boot
    // if they did not take. It is still handled rather than unwrapped,
    // because "cannot happen" and "panics the sim thread if it does" is a
    // trade nothing in this file makes.
    // `dev_arc_hour`: a fresh world's arc clock starts late, before the
    // first tick, so the first works sweep opens and lights what that hour
    // would have — the real path — and the blight reads the same hour.
    if let (true, Some(h)) = (world_blob.is_empty(), dev_arc_hour) {
        core.world.tick = u64::from(h) * sim_core::blight::TICKS_PER_HOUR;
    }
    if !world_blob.is_empty() {
        match core.world.load(&world_blob) {
            Ok(()) => {
                // A balance edit lowered a ceiling under a saved stack: the
                // trial load in `bin/shard.rs` clamped its copy, and this is
                // the world that runs (`worldfile::WorldBoot::clamp`).
                if world_clamp {
                    core.world.clamp_conditions();
                }
                core.adopt_identities(&world_idents);
                // The teams its sleepers were on, by their keys.
                if let Err(e) = core.load_teams() {
                    eprintln!("shard: teams not restored: {e}");
                }
                // Who its evicted bodies were, so their bases come back
                // to them (`ShardCore::load_evict_ids`).
                if let Err(e) = core.load_evict_ids() {
                    eprintln!("shard: evicted ids not restored: {e}");
                }
                ShardStats::set(&stats.current_tick, core.world.tick);
            }
            Err(_) => ShardStats::bump(&stats.world_load_errors),
        }
    }
    drop(world_blob);
    // A world nobody has salted draws its secret now (`sim_core::mech`): a
    // fresh world, or one saved before the arc had one. The puzzles' answers
    // follow from it, so this wipe's are not last wipe's and no client that
    // holds only the seed can work them out.
    if core.world.arc.salt == 0 {
        core.world.arc.salt = fresh_salt();
    }
    // And its blight salt (`sim_core::blight`), on the same terms: a fresh
    // world draws where this wipe's outbreaks erupt. Public, unlike the arc's
    // — every welcome carries it, because every client draws the outbreaks.
    if core.world.blight_salt == 0 {
        core.world.set_blight_salt((fresh_salt() >> 32) as u32 | 1);
    }
    // The town's public stations (`World::seed_authored`), after the load and
    // before the first tick: a saved world already holds them and this is a
    // no-op there.
    core.world.seed_authored();
    // `dev_env`: the admin lane's sky/clock verb, queued so it lands on the
    // first tick and in the WAL like any other.
    if let Some((weather, time_pm)) = dev_env {
        core.queue_env(weather, time_pm);
    }
    // Footage only (`film.rs`): a powered, loaded auto turret, built before
    // the first player joins.
    if let Some(spec) = std::env::var("GATES_FILM_TURRET")
        .ok()
        .filter(|s| !s.is_empty())
    {
        match crate::film::stage_turret_spec(&mut core.world, &spec) {
            Ok(said) => println!("⚠ film: {said} — a DEV shard, never a public one"),
            Err(e) => eprintln!("film: {e}"),
        }
    }
    // The world-save buffer pool. Allocated here, once, and never again:
    // every later save fills one of these and hands the box to the store
    // thread, which hands the box back. Wall 2 counts the tick, and the tick
    // only writes into a buffer that already exists.
    let mut world_pool: Vec<Box<[u8]>> = (0..WORLD_RING_CAP)
        .map(|_| vec![0u8; WORLD_SAVE_MAX_BYTES].into_boxed_slice())
        .collect();
    let mut ident_pool: Vec<Vec<(PlayerKey, u32)>> = (0..WORLD_RING_CAP)
        .map(|_| Vec::with_capacity(MAX_PLAYERS))
        .collect();
    let mut next_world_save = core.world.tick + world_interval.min(u64::MAX / 2);
    // Side-file failures already answered (`retry_side_files`).
    let mut side_failures_seen = 0u64;
    // Every connection slot: players, then spectator seats (`MAX_CONNS`).
    let mut links: Vec<Option<Link>> = Vec::with_capacity(MAX_CONNS);
    links.resize_with(MAX_CONNS, || None);
    let mut links = links.into_boxed_slice();
    // Who hears the talker being relayed (`ShardCore::voice_reach`), reused.
    let mut voice_hears = [false; MAX_CONNS];

    // Startup-only notification: no channel wake or drop enters the tick.
    // A cancelled startup has no accept loop and must not leave a sim alive.
    // The ready signal carries the blight salt to the accept loop, which
    // writes it into every welcome.
    // The welcome's tick reads this: a loaded world set it above, and a
    // `dev_arc_hour` world starts late, so both are mirrored before anyone
    // is admitted.
    ShardStats::set(&stats.current_tick, core.world.tick);
    if ready.send(core.world.blight_salt).is_err() {
        return;
    }
    let tick_dur = Duration::from_nanos(1_000_000_000 / TICK_HZ as u64);
    let mut next = Instant::now();
    let mut lead = PaceLead::default();
    // The boundary loop: clock read and sleep live here and only here;
    // the tick body makes no syscalls (L1 — the sleep IS the boundary).
    // The phase marks below read the clock from inside `tick_marked`, which
    // is this loop's closure doing it: a vDSO read into an atomic, nothing
    // the tick itself decides on.
    let us = |d: Duration| d.as_micros().min(u64::MAX as u128) as u64;
    while !shutdown.load(Ordering::Relaxed) {
        let t_iter = Instant::now();
        // Install fresh connections.
        while let Ok(c) = ctrl_rx.pop() {
            // A spectator seat: no body, no command, no save — a connection's
            // netcode bound to its target's body (`ShardCore::connect_spectator`).
            // Refused if the target left in the window since the accept loop
            // checked; the link then rides the LEAVING sweep out like any
            // refused install, and the watcher's connection dies with it.
            if let Some(target) = c.watch {
                let generation = c.link.generation;
                links[c.slot] = Some(c.link);
                if core.connect_spectator(c.slot, c.id, target) {
                    ShardStats::bump(&stats.spectate_joins);
                } else {
                    slots.mark_leaving(c.slot, generation);
                    ShardStats::bump(&stats.spectate_refused);
                }
                continue;
            }
            let key = c.key;
            // An account's character, taken by the wallet it linked
            // (`Connect::alias`), moves before the join looks for it. A
            // wallet the sim already knows keeps its own, and the account's
            // record the store handed over is not one to wake it in.
            let save = match (c.key, c.alias) {
                (Some(to), Some(from)) if !core.adopt_account(&from, &to) => None,
                _ => c.save,
            };
            if let Some((how, evicted)) = core.connect_as(c.slot, c.id, c.key, save) {
                links[c.slot] = Some(c.link);
                ShardStats::bump(&stats.joins);
                // Who this is, for everyone's chat and nametags: the proven
                // address now, the platform name when its read lands.
                core.tag_join(c.slot, c.id, key.as_ref());
                core.name_agent(c.slot, c.name);
                core.note_agent(c.slot, c.agent);
                // A wallet read inside the cooldown, stated with the join
                // (`Connect::said`): after `tag_join`, which owes the gifts
                // (`skins_owned` unions them back in), and after
                // `name_agent`, whose name an empty face keeps.
                if let Some(owned) = c.said.skins {
                    core.skins_owned(c.slot, c.id, owned);
                }
                if let Some((name, pic)) = c.said.face {
                    core.set_face(c.slot, c.id, name, pic);
                }
                // Two-phase eviction, the filing half: this join is about
                // to cost a sleeper its slot, and this record is that body
                // as it stands NOW — raid included — not as its leave left
                // it. Keyed by the sim (the victim has no connection slot
                // for `drain_saves` to resolve), and pushed before the
                // tick below applies the `Evict`, on the same ring every
                // other record rides.
                if let Some((key, save)) = evicted {
                    push_save(&mut save_tx, 0, Some(key), save, &stats);
                }
                // Counted here and nowhere else, because here is the only
                // place that knows which door opened. The accept task can
                // see that a record *exists*; it cannot see that the world
                // still had the body and the record went unread.
                match how {
                    Admitted::TookOver => ShardStats::bump(&stats.takeovers),
                    Admitted::Restored => ShardStats::bump(&stats.saves_restored),
                    Admitted::Fresh => {}
                }
            } else {
                // Refused: no world slot for the join (every body awake, or
                // the few sleepers left are held, `ShardCore::evict_victim`),
                // or the command queue full (unreachable by arithmetic: ctrl
                // cap + leave cap < queue reserve). Park the link and route
                // it through the normal LEAVING sweep — the sim thread never
                // drops a ring handle itself — with the reason, so the
                // client reads "full" and not a network fault.
                let generation = c.link.generation;
                links[c.slot] = Some(c.link);
                slots.mark_leaving_why(c.slot, generation, REFUSE_FULL);
                ShardStats::bump(&stats.refused_full);
            }
        }
        // What the platform said players own (`skins.rs`), after the joins
        // above so a set read for a joiner finds the client it belongs to.
        while let Ok(m) = skins_rx.pop() {
            core.skins_owned(m.slot, m.id, m.owned);
        }
        // The names and pictures players set on the platform (`faces.rs`).
        while let Ok(m) = faces_rx.pop() {
            core.set_face(m.slot, m.id, m.name, m.pic);
        }
        // What the platform's store charges for each skin.
        while let Ok(m) = prices_rx.pop() {
            core.skin_prices(&m.prices);
        }
        // Clean up dead connections — every connection slot, seats included.
        for slot in 0..MAX_CONNS {
            let word = slots.load(slot);
            if state_of(word) != SLOT_LEAVING {
                continue;
            }
            let matches = links[slot]
                .as_ref()
                .map(|l| l.generation == generation_of(word))
                .unwrap_or(false);
            if !matches {
                continue; // install still in flight; next tick
            }
            let link = links[slot].take().expect("checked above");
            let generation = link.generation;
            match grave_tx.push(link) {
                Ok(()) => {
                    // The exact save: taken off the live body, before the
                    // `Leave` is queued and — the part that matters —
                    // **before the slot is freed**. A freed slot is
                    // claimable, so pushing after would race a new tenant's
                    // key into the accept loop's table ahead of this record
                    // (`KeySlot`). The order here is what closes that.
                    if let Some((id, save)) = core.disconnect(slot) {
                        push_save(&mut save_tx, id, None, save, &stats);
                    }
                    slots.free(slot, generation);
                    // `leaves` is a PLAYER counter (it pairs with `joins`);
                    // a seat closing is not a player leaving.
                    if slot < MAX_PLAYERS {
                        ShardStats::bump(&stats.leaves);
                    }
                }
                Err(rtrb::PushError::Full(link)) => {
                    // Graveyard full: hold the handles, retry next tick.
                    links[slot] = Some(link);
                }
            }
        }
        // Drain inputs, and at most one action per client per tick — the
        // ring buffers the burst, the stream backpressures past it.
        for slot in 0..MAX_CONNS {
            if let Some(link) = links[slot].as_mut() {
                // A seat's datagrams are its acks (`push_input` takes nothing
                // else from one), so they drain like a player's.
                while let Ok(dg) = link.input.pop() {
                    core.push_input(slot, &dg);
                }
                if slot >= MAX_PLAYERS {
                    continue; // a watcher has no action or chat lane
                }
                if core.wants_action(slot) {
                    if let Ok(act) = link.actions.pop() {
                        core.push_action(slot, act);
                    }
                }
                // Chat drains unconditionally, one line per client per
                // tick: the fan-out always takes the line (it is said or
                // it is lost), so unlike an action there is no hand to
                // check for room.
                if let Ok(chat) = link.chats.pop() {
                    core.push_chat(slot, chat);
                }
            }
        }
        // Proximity voice (v113): each talker's frames go to whoever is in
        // range now, measured here because positions live on this thread.
        // Relayed, never simulated — nothing below touches the world.
        for slot in 0..MAX_PLAYERS {
            let mut frames = [protocol::VoiceFrame::EMPTY; VOICE_PER_TICK];
            let mut n = 0;
            if let Some(link) = links[slot].as_mut() {
                while n < VOICE_PER_TICK {
                    let Ok(f) = link.voice_in.pop() else {
                        break;
                    };
                    frames[n] = f;
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let Some(from) = core.voice_reach(slot, &mut voice_hears) else {
                ShardStats::add(&stats.voice_unheard, n as u64);
                continue;
            };
            // To the scribe too, for captions (v138).
            core.scribe_hear(slot, &frames[..n], &voice_hears, &stats);
            for f in &mut frames[..n] {
                f.from = from;
                let mut msg = VoiceMsg {
                    len: 0,
                    buf: [0; protocol::VOICE_DATAGRAM_MAX],
                };
                let Ok(len) = protocol::encode_voice(f, &mut msg.buf) else {
                    ShardStats::bump(&stats.encode_range_errors);
                    continue;
                };
                msg.len = len as u16;
                for (to, &hears) in voice_hears.iter().enumerate() {
                    let Some(link) = links[to].as_mut().filter(|_| hears) else {
                        continue;
                    };
                    if link.voice_out.push(msg).is_ok() {
                        ShardStats::bump(&stats.voice_relayed);
                    } else {
                        ShardStats::bump(&stats.voice_out_drops);
                    }
                }
            }
        }
        // One step of the autosave sweep, before the tick rather than after
        // it for a reason worth stating: the record is then the state the
        // *previous* tick published, which is the state a client has actually
        // been shown. It costs one tick of staleness against a sweep that is
        // already `MAX_PLAYERS` ticks coarse, and it keeps the save read out
        // of the tick's own borrow of the world.
        if let Some((id, save)) = core.autosave() {
            push_save(&mut save_tx, id, None, save, &stats);
        }
        // Buffers coming home from the store thread — and with them the
        // standings taken beside that save, now safe to write: a crash
        // rolls the world and the credits back together.
        while let Ok(done) = world_done_rx.pop() {
            world_pool.push(done.buf);
            ident_pool.push(done.idents);
            if done.ok {
                core.standings.release();
            }
            if done.side_lost {
                core.evict_ids.mark_dirty();
            }
        }
        // The world, on its cadence. Before the tick for the reason the
        // autosave sweep is: the blob is then the state the *previous* tick
        // published, which is a state clients have actually been shown.
        if core.world.tick >= next_world_save {
            next_world_save = core.world.tick.saturating_add(world_interval);
            retry_side_files(&mut core, &stats, &mut side_failures_seen);
            take_world_save(
                &mut core,
                &mut world_pool,
                &mut ident_pool,
                &mut world_tx,
                &mut file_tx,
                &stats,
            );
            core.standings.snapshot();
        }
        // The wipe clock, once a second off the wall clock (this loop is the
        // boundary: the clock read lives here, never in the tick). Before the
        // tick, so a countdown line rides this tick's chat pump.
        if core
            .world
            .tick
            .is_multiple_of(sim_core::limits::TICK_HZ as u64)
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            core.wipe_poll(now);
            ShardStats::set(&stats.next_wipe, core.wipe.next().map_or(0, |p| p.at));
            // The operator's SIGUSR1 (`bin/shard.rs`), as minutes + 1.
            let asked = stats.restart_asked.swap(0, Ordering::Relaxed);
            if asked > 0 {
                core.restart_in(now, (asked - 1).min(60) as u16);
            }
            core.restart_poll(now);
            ShardStats::set(&stats.restart_at, core.restart.pending().unwrap_or(0));
            let def = &core.world.works_def;
            let lit = core
                .world
                .works
                .w
                .iter()
                .take(def.count as usize)
                .filter(|w| w.state == sim_core::works::WORK_LIT)
                .count();
            ShardStats::set(&stats.wipe, core.wipe.number() as u64);
            ShardStats::set(&stats.act, core.world.works.act(def) as u64);
            ShardStats::set(&stats.works_lit, lit as u64);
            ShardStats::set(&stats.works_total, def.count as u64);
        }
        // The standings, once a minute: minutes played, every base
        // recounted, the boards rebuilt (`standings.rs`). The file itself
        // rides the world save, below.
        if core.world.tick % (60 * sim_core::limits::TICK_HZ as u64) == 13 {
            core.standings_minute();
            core.standings_recount();
            core.standings.rebuild();
            core.publish_online();
        }
        // The companion's view of the island, once a second.
        if core.companion.on() && core.world.tick % crate::companion::SNAP_TICKS == 7 {
            core.publish_companion();
        }
        // Tick + publish. `Ops` is the tick's side channels (admin v0) —
        // the anomaly log, the kick ring and its answers, `/save`'s flag.
        let mut save_now = false;
        let mut ops = crate::core::Ops {
            log: &mut log,
            admin_tx: Some(&mut admin_tx),
            admin_answers: Some(&mut answer_rx),
            save_now: &mut save_now,
        };
        let t_core = Instant::now();
        stats.record_phase(crate::core::Phase::Net as usize, us(t_core - t_iter));
        let mut last = t_core;
        let mut cur = crate::core::Phase::Prep as usize;
        let mark = |phase: crate::core::Phase| {
            let now = Instant::now();
            stats.record_phase(cur, us(now - last));
            last = now;
            cur = phase as usize;
        };
        core.tick_marked(
            &stats,
            &mut ops,
            |lane, slot, bytes| {
                let Some(link) = links[slot].as_mut() else {
                    return false;
                };
                match lane {
                    Lane::Snapshot => {
                        let mut msg = SnapMsg {
                            len: bytes.len() as u16,
                            buf: [0; sim_core::limits::DATAGRAM_BUDGET_BYTES],
                        };
                        msg.buf[..bytes.len()].copy_from_slice(bytes);
                        if link.snaps.push(msg).is_err() {
                            ShardStats::bump(&stats.snap_ring_skips);
                            return false; // counted; snapshots supersede anyway
                        }
                        true
                    }
                    Lane::Event => {
                        let mut msg = EvMsg {
                            len: bytes.len() as u16,
                            buf: [0; MAX_EVENT_MSG_BYTES],
                        };
                        msg.buf[..bytes.len()].copy_from_slice(bytes);
                        link.events.push(msg).is_ok()
                    }
                }
            },
            mark,
        );
        stats.record_phase(cur, us(last.elapsed()));
        // This tick's snapshots and events, and the voice relayed above,
        // are on the rings: wake the writers (`wake_relay`).
        slots.publish();
        ShardStats::bump(&stats.ticks);
        // A gift given this tick: the whole ledger to the store thread, which
        // owns every write (`gifts.rs`). Room is checked before the text is
        // built; a full ring leaves the ledger dirty for the next tick.
        if core.gifts.is_dirty() && !file_tx.is_full() {
            if let Some(msg) = core.gifts.take_write() {
                let _ = file_tx.push(msg);
            }
        }
        // A sale or a settled one (`mints.rs`): the queue the minter reads.
        if core.mints.is_dirty() && !file_tx.is_full() {
            if let Some(msg) = core.mints.take_write() {
                let _ = file_tx.push(msg);
            }
        }
        // `/save` asked for the world now rather than at the cadence. Set
        // the deadline to this tick and let the cadence check above take
        // it on the next pass — the admin verb moves the deadline, it does
        // not perform the write, so there is one world-save path and not
        // two (`take_world_save` is still the only writer).
        if save_now {
            next_world_save = core.world.tick;
        }
        // The wipe came due: stop the shard the way a SIGTERM does — the
        // flush below writes the world and every player — and say so to
        // `bin/shard.rs`, which ends the world on disk once the files close.
        if let Some(blueprints) = core.take_wipe() {
            if blueprints {
                ShardStats::raise(&stats.wipe_blueprints);
            }
            ShardStats::raise(&stats.wipe_due);
            shutdown.store(true, Ordering::Relaxed);
        }
        // The restart came due: the same stop without the wipe. `bin/shard.rs`
        // exits once the flush is down and the supervisor starts it again.
        if core.take_restart() {
            ShardStats::raise(&stats.restart_due);
            shutdown.store(true, Ordering::Relaxed);
        }
        // The anomaly log's counter sweep, once a second rather than once a
        // tick: a counter that moved is interesting to the second, and the
        // sweep is ~30 atomic loads (`anomaly::Watch`).
        if core
            .world
            .tick
            .is_multiple_of(sim_core::limits::TICK_HZ as u64)
        {
            watch.sweep(core.world.tick, &stats, &mut log);
        }
        stats.current_tick.store(core.world.tick, Ordering::Relaxed);
        // The gauges, mirrored off the state rather than accumulated here:
        // the eviction policy lives in `World::seat` and nothing on this
        // thread is told when it fires, so the counter is read, not bumped.
        // `sleepers()`, `connected()` and `agents_connected()` are each an
        // O(MAX_PLAYERS) scan of a 100-element array on a thread that has
        // just done a tick's work — measured against the alternative, which
        // is a second copy of the count that can drift from the array it
        // describes.
        ShardStats::set(&stats.sleepers_evicted, core.world.evictions);
        ShardStats::set(&stats.sleepers, core.world.sleepers() as u64);
        ShardStats::set(&stats.players, core.connected() as u64);
        ShardStats::set(&stats.agents, core.agents_connected() as u64);
        ShardStats::set(&stats.spectators, core.spectators() as u64);
        stats.record_phase(crate::stats::PHASE_TICK, us(t_iter.elapsed()));

        // Pace (the boundary).
        next += tick_dur;
        let now = Instant::now();
        if now < next {
            // On time, so any sprint is run off whole.
            core.note_pace_backlog(lead.step(0));
            std::thread::sleep(next - now);
        } else {
            let behind = now - next;
            if behind > tick_dur * MAX_TICK_BACKLOG {
                let missed = (behind.as_nanos() / tick_dur.as_nanos()) as u64;
                stats.ticks_dropped.fetch_add(missed, Ordering::Relaxed);
                next = now;
                lead.drop_backlog();
            } else {
                // The ticks run back to back so far: the world tick has
                // outrun every client's acks by that many, and the favour
                // bounds must know (`ShardCore::note_pace_backlog`).
                let behind = (behind.as_nanos() / tick_dur.as_nanos()) as u16;
                core.note_pace_backlog(lead.step(behind));
            }
        }
    }

    // ---- shutdown, and this is the whole of `NOW.md` §0y item 6 ----------
    //
    // A kill used to cost up to `MAX_PLAYERS` ticks of every player's
    // progress — the autosave sweep's own coarseness, 3.3 s at 30 Hz — plus,
    // since world persistence, up to a whole `world_save_interval_ticks` of
    // everybody's *base*. Neither is necessary on a shutdown the shard can
    // see coming, and an operator restarting a shard to deploy is the most
    // common restart there is.
    //
    // **The world goes first, while everyone is still connected**, and the
    // order is load-bearing rather than tidy: `identities` reads connected
    // clients out of the key table and sleepers out of the sleeper index, so
    // taking the world *after* the disconnects below would find the key
    // table cleared and the `Leave` commands still queued behind a tick that
    // is never going to run — a world full of bodies with nobody's name on
    // them. The encoder puts an awake body to sleep on the way out anyway
    // (`worldsave.rs`), so nothing is lost by saving them awake.
    while let Ok(done) = world_done_rx.pop() {
        world_pool.push(done.buf);
        ident_pool.push(done.idents);
        if done.side_lost {
            core.evict_ids.mark_dirty();
        }
    }
    // The standings' last count, while everyone is still connected (the
    // world save's reason, below): a wipe reads this file for its hall.
    core.standings_recount();
    core.standings.rebuild();
    if let Err(e) = core.standings.save_now() {
        eprintln!("shard: standings at shutdown: {e}");
    }
    // The achievements' last flush, landed before the process ends.
    let standings = &core.standings;
    core.achievements
        .close(|k| standings.label_of(k).map(str::to_string));
    // Room for the shutdown's side files — the teams the save below takes,
    // and a gift ledger a full ring left dirty — on the ring the store thread
    // is draining right now. Bounded like the accept loop's drain; past the
    // bound the teams keep their last file and a gift is asked for again.
    // (The evicted ids ride the world itself, `WorldMsg::side`.)
    for _ in 0..SHUTDOWN_DRAIN_TRIES {
        if file_tx.slots() >= 2 {
            break;
        }
        std::thread::sleep(SHUTDOWN_DRAIN_POLL);
    }
    // The last teams go over whatever the last write did: a failed one
    // reaches the store thread's count only after the fact.
    core.teams.forget_written();
    retry_side_files(&mut core, &stats, &mut side_failures_seen);
    take_world_save(
        &mut core,
        &mut world_pool,
        &mut ident_pool,
        &mut world_tx,
        &mut file_tx,
        &stats,
    );
    if !file_tx.is_full() {
        if let Some(msg) = core.gifts.take_write() {
            let _ = file_tx.push(msg);
        }
    }
    if !file_tx.is_full() {
        if let Some(msg) = core.mints.take_write() {
            let _ = file_tx.push(msg);
        }
    }
    // Then every connected player's exact record, which is the same read a
    // leave takes and is what makes a clean restart cost nobody anything.
    for slot in 0..MAX_PLAYERS {
        if let Some((id, save)) = core.disconnect(slot) {
            push_save(&mut save_tx, id, None, save, &stats);
        }
    }
    // Every producer drops here — `save_tx`, `world_tx`, `file_tx` — which is
    // the signal the other two threads are already written to wait for: the
    // accept loop drains `save_rx` until it is abandoned, and the store
    // thread exits when its rings are dry and abandoned. Nothing is timed and
    // nothing is guessed.
}

/// A random `u64` for a world's arc salt, from the OS-seeded hasher keys
/// std draws per process. Never 0, which means "unsalted".
fn fresh_salt() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(std::process::id() as u64);
    h.finish().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Only a backlog run off is a lead.** An overloaded loop's backlog
    /// grows until it is dropped and gains on nobody, so it grants no pace
    /// slack; a stall of `k` followed by fast ticks grants 1..=k as the
    /// sprint runs them off, then starts over.
    #[test]
    fn only_a_sprint_run_off_grants_pace_slack() {
        let mut lead = PaceLead::default();
        for behind in 0..=MAX_TICK_BACKLOG as u16 {
            for _ in 0..3 {
                assert_eq!(lead.step(behind), 0, "overload at {behind} behind");
            }
        }
        lead.drop_backlog();
        assert_eq!(lead.step(0), 0, "a dropped backlog was never run");
        for k in 1..=MAX_TICK_BACKLOG as u16 {
            assert_eq!(lead.step(k), 0, "the stall itself gains nothing");
            for run in 1..=k {
                assert_eq!(lead.step(k - run), run, "stall {k}, {run} run off");
            }
            assert_eq!(lead.step(0), 0, "on time: the next sprint starts over");
        }
        // A sprint that turns back into overload grants what it ran off.
        assert_eq!(lead.step(5), 0);
        assert_eq!(lead.step(3), 2);
        assert_eq!(lead.step(4), 1);
        assert_eq!(lead.step(6), 0);
        lead.drop_backlog();
        assert_eq!(lead.step(2), 0, "a dropped backlog leaves no peak");
    }

    /// **One source holds at most `ADMIT_PER_SOURCE` handshakes, and gets
    /// each back when it ends.** An IPv6 /64 is one source; loopback is
    /// never counted (the bots and the wire suites dial from it).
    #[test]
    fn a_source_holds_at_most_its_allowance_of_handshakes() {
        let gate = Arc::new(SourceGate::new(Some("192.0.2.1".parse().unwrap()), &[]));
        let ip: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        let mut held: Vec<_> = (0..ADMIT_PER_SOURCE)
            .map(|_| SourceGate::take(&gate, ip).expect("within the allowance"))
            .collect();
        assert!(
            SourceGate::take(&gate, ip).is_none(),
            "one past the allowance"
        );
        held.pop();
        assert!(
            SourceGate::take(&gate, ip).is_some(),
            "a finished handshake gives its place back"
        );

        let a: std::net::IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: std::net::IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        assert_eq!(gate.bucket(a), gate.bucket(b), "a /64 is one source");
        let mapped: std::net::IpAddr = "::ffff:203.0.113.7".parse().unwrap();
        assert_eq!(
            gate.bucket(mapped),
            gate.bucket(ip),
            "v4-mapped is its v4 address"
        );

        let lo: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let many: Vec<_> = (0..4 * ADMIT_PER_SOURCE)
            .map(|_| SourceGate::take(&gate, lo).expect("loopback is never refused"))
            .collect();
        assert!(many.iter().all(|p| p.bucket.is_none()));
        let own: std::net::IpAddr = "192.0.2.1".parse().unwrap();
        let mine: Vec<_> = (0..4 * ADMIT_PER_SOURCE)
            .map(|_| {
                SourceGate::take(&gate, own).expect("the shard's own address is never refused")
            })
            .collect();
        assert!(mine.iter().all(|p| p.bucket.is_none()));

        // A wildcard bind names no address of its own, so the house agents,
        // dialling the public name from the box, arrive from its public
        // address: `own_sources` exempts it, and nobody else.
        let public: std::net::IpAddr = "198.51.100.4".parse().unwrap();
        let wild = Arc::new(SourceGate::new(Some("0.0.0.0".parse().unwrap()), &[public]));
        let house: Vec<_> = (0..4 * ADMIT_PER_SOURCE)
            .map(|_| SourceGate::take(&wild, public).expect("the host's own address"))
            .collect();
        assert!(house.iter().all(|p| p.bucket.is_none()));
        assert_eq!(wild.counted(public), None, "nor its seats");
        let strangers: Vec<_> = (0..ADMIT_PER_SOURCE)
            .map(|_| SourceGate::take(&wild, ip).expect("within the allowance"))
            .collect();
        assert!(
            SourceGate::take(&wild, ip).is_none(),
            "a stranger is still capped"
        );
        drop(strangers);
    }

    /// **Sign-ins nobody answers make room, oldest first within a source,
    /// and never block the door.** A full pool of one source (here the
    /// shard's own host) drops its oldest waiter for the newcomer, and a
    /// waiter that finishes gives its place back.
    #[tokio::test]
    async fn the_sign_in_wait_drops_its_oldest_when_full() {
        let waiting = |p: &SignWait| {
            p.seats
                .iter()
                .filter(|s| s.held.load(Ordering::Acquire) != 0)
                .count()
        };
        let pool = Arc::new(SignWait::new());
        let first = Arc::new(SignWait::join(&pool, None));
        let mut rest: Vec<_> = (1..SIGN_WAITERS)
            .map(|_| SignWait::join(&pool, None))
            .collect();
        assert_eq!(waiting(&pool), SIGN_WAITERS);
        // The oldest is already waiting on its signature when it is bumped.
        let told = tokio::spawn({
            let first = first.clone();
            async move { first.bumped().await }
        });
        tokio::task::yield_now().await;
        let newest = SignWait::join(&pool, None);
        assert_eq!(waiting(&pool), SIGN_WAITERS, "still bounded");
        tokio::time::timeout(Duration::from_secs(1), told)
            .await
            .expect("the oldest is told to go")
            .expect("joins");
        drop(first);
        assert_eq!(
            waiting(&pool),
            SIGN_WAITERS,
            "its seat stays the newcomer's"
        );
        // A waiter that is answered leaves, and the next newcomer drops nobody.
        rest.pop();
        let _again = SignWait::join(&pool, None);
        assert_eq!(waiting(&pool), SIGN_WAITERS);
        let second = rest.remove(0);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), second.bumped())
                .await
                .is_err(),
            "room was free, so the next oldest stays"
        );
        // One past full again: now the second-oldest goes, even unpolled.
        let _later = SignWait::join(&pool, None);
        tokio::time::timeout(Duration::from_secs(1), second.bumped())
            .await
            .expect("the next oldest is told to go");
        drop(newest);
    }

    /// **A flood from fifty networks churns its own seats, not a slow
    /// signer's.** Each address holds its in-flight allowance and rejoins as
    /// fast as its waiters are dropped (a dropped waiter's connection closes
    /// and gives its place back), which kept the pool full forever: under
    /// oldest-first the honest waiter was the next to go after 400 joins.
    #[test]
    fn a_flood_of_fifty_sources_never_drops_the_one_honest_waiter() {
        let pool = Arc::new(SignWait::new());
        let seated = |t: &SignTicket| pool.seats[t.seat].held.load(Ordering::Acquire) == t.word;
        let honest = SignWait::join(&pool, Some(4000));
        let mut flood: Vec<SignTicket> = Vec::new();
        let mut next = 0;
        // Ten pools' worth: past the first fill, every join drops somebody.
        for _ in 0..10 * SIGN_WAITERS {
            flood.retain(&seated);
            let mut holds = [0usize; 50];
            for t in &flood {
                holds[SignWait::source(t.word)] += 1;
            }
            // The next address under its allowance, round robin: there is
            // always one, as fifty allowances are more than the pool.
            let source = (0..50)
                .map(|k| (next + k) % 50)
                .find(|&s| holds[s] < ADMIT_PER_SOURCE as usize)
                .expect("a dropped waiter gave its place back");
            next = source + 1;
            flood.push(SignWait::join(&pool, Some(source)));
            assert!(seated(&honest), "the one waiter from its own source stays");
        }

        // The counts name exactly the seats held, now nobody is mid-join.
        let mut want = vec![0u32; SIGN_HOST + 1];
        for s in pool.seats.iter() {
            let w = s.held.load(Ordering::Acquire);
            if w != 0 {
                want[SignWait::source(w)] += 1;
            }
        }
        let have: Vec<u32> = pool.per.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        assert_eq!(have, want);
        assert_eq!(want.iter().sum::<u32>() as usize, SIGN_WAITERS);
        drop(flood);
        drop(honest);
        assert!(pool.per.iter().all(|c| c.load(Ordering::Relaxed) == 0));
    }

    /// **One network holds at most `ADMIT_PER_PREFIX` handshakes**, however
    /// many of its addresses ask: a /24 or a /48 is not a crowd of sources.
    /// A refused network leaves the source table as it found it.
    #[test]
    fn a_network_holds_at_most_its_allowance_across_its_addresses() {
        let gate = Arc::new(SourceGate::new(None, &[]));
        let mut held = Vec::new();
        for host in 1..=u8::MAX {
            let ip: std::net::IpAddr = format!("198.51.100.{host}").parse().unwrap();
            match SourceGate::take(&gate, ip) {
                Some(p) => held.push(p),
                None => break,
            }
        }
        assert_eq!(
            held.len(),
            ADMIT_PER_PREFIX as usize,
            "one /24, one allowance"
        );
        let next: std::net::IpAddr = "198.51.100.250".parse().unwrap();
        assert!(SourceGate::take(&gate, next).is_none());
        assert_eq!(
            gate.held[gate.bucket(next)].load(Ordering::Relaxed),
            0,
            "a network refusal gives the source's place back"
        );
        // Another /24 (one whose keyed bucket differs: the key is random).
        let elsewhere = (101..=u8::MAX)
            .map(|c| format!("198.51.{c}.1").parse::<std::net::IpAddr>().unwrap())
            .find(|ip| gate.net_bucket(*ip) != gate.net_bucket(next))
            .unwrap();
        assert!(
            SourceGate::take(&gate, elsewhere).is_some(),
            "another /24 is its own"
        );
        held.pop();
        assert!(
            SourceGate::take(&gate, next).is_some(),
            "and a finished one is given back"
        );

        let a: std::net::IpAddr = "2001:db8:aa:1::1".parse().unwrap();
        let b: std::net::IpAddr = "2001:db8:aa:ff00::1".parse().unwrap();
        assert_eq!(gate.net_bucket(a), gate.net_bucket(b), "but one /48");
    }

    /// **One address cannot take every watcher seat** (`WATCH_SEATS_PER_SOURCE`):
    /// a watcher's handshake permit is gone once it is seated, so the seats
    /// themselves are counted by source, a /64 as one, a swept seat as none.
    #[test]
    fn one_source_holds_at_most_its_share_of_the_watcher_seats() {
        let mut seats: [SeatSlot; MAX_SPECTATORS] = std::array::from_fn(|_| SeatSlot::default());
        let a: std::net::IpAddr = "203.0.113.9".parse().unwrap();
        let v6: std::net::IpAddr = "2001:db8:5:6::1".parse().unwrap();
        let v6_sibling: std::net::IpAddr = "2001:db8:5:6:abcd::2".parse().unwrap();
        for (i, ip) in [Some(a), Some(a), Some(v6), None].into_iter().enumerate() {
            seats[i] = SeatSlot {
                occupied: true,
                source: ip,
                ..SeatSlot::default()
            };
        }
        assert_eq!(watch_seats_held(&seats, a), 2);
        assert_eq!(
            watch_seats_held(&seats, v6_sibling),
            1,
            "a /64 is one source"
        );
        // A swept seat holds nothing.
        seats[0] = SeatSlot::default();
        assert_eq!(watch_seats_held(&seats, a), 1);
    }

    /// **A source's seats are its live tenants from that address (or /64)**:
    /// a leaving tenant, a stale generation and the shard's own host do not
    /// count (`SEATS_PER_SOURCE`).
    #[test]
    fn seats_held_counts_one_sources_live_tenants() {
        let slots = SlotTable::new(MAX_CONNS);
        let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
        let a: std::net::IpAddr = "203.0.113.9".parse().unwrap();
        let v6: std::net::IpAddr = "2001:db8:5:6::1".parse().unwrap();
        let v6_sibling: std::net::IpAddr = "2001:db8:5:6:abcd::2".parse().unwrap();
        for (s, ip) in [(0, Some(a)), (1, Some(a)), (2, Some(v6)), (3, None)] {
            let generation = slots.claim(s).unwrap();
            keys[s] = KeySlot {
                generation,
                source: ip,
                ..KeySlot::default()
            };
        }
        assert_eq!(seats_held(&keys, &slots, a), 2);
        assert_eq!(
            seats_held(&keys, &slots, v6_sibling),
            1,
            "a /64 is one source"
        );
        assert_eq!(
            seats_held(&keys, &slots, "203.0.113.10".parse().unwrap()),
            0
        );
        slots.mark_leaving(1, keys[1].generation);
        assert_eq!(
            seats_held(&keys, &slots, a),
            1,
            "a leaving tenant holds no seat"
        );
        let gate = SourceGate::new(Some("192.0.2.1".parse().unwrap()), &[]);
        assert_eq!(gate.counted("127.0.0.1".parse().unwrap()), None);
        assert_eq!(gate.counted("192.0.2.1".parse().unwrap()), None);
        assert_eq!(gate.counted("::ffff:203.0.113.9".parse().unwrap()), Some(a));
    }

    /// **The human feed's delay line holds every message for exactly its
    /// delay, in order, and refuses past its bound** — clock-free: every
    /// instant is handed in, none is read.
    #[test]
    fn the_delay_line_releases_each_message_at_its_due_and_not_before() {
        let t0 = tokio::time::Instant::now();
        let d = Duration::from_secs(5);
        let mut line = DelayLine::<u32>::new(3);
        assert!(line.push(t0 + d, 1).is_ok());
        assert!(line.push(t0 + Duration::from_secs(1) + d, 2).is_ok());
        assert!(line.push(t0 + Duration::from_secs(2) + d, 3).is_ok());
        assert_eq!(line.push(t0 + d, 4), Err(4), "a full line refuses");
        // Nothing is due before the delay has passed for it.
        assert_eq!(line.pop_due(t0), None);
        assert_eq!(line.pop_due(t0 + d - Duration::from_millis(1)), None);
        // Released in order, each at its own due, never early.
        assert_eq!(line.pop_due(t0 + d), Some(1));
        assert_eq!(line.pop_due(t0 + d), None, "the next is not due yet");
        assert_eq!(line.pop_due(t0 + Duration::from_secs(10)), Some(2));
        assert_eq!(line.pop_due(t0 + Duration::from_secs(10)), Some(3));
        assert_eq!(line.pop_due(t0 + Duration::from_secs(10)), None);
        // Dropping the oldest makes room for the newest (the snapshot lane's
        // policy; the event lane refuses instead).
        for i in 0..3 {
            line.push(t0, 10 + i).unwrap();
        }
        line.drop_oldest();
        assert!(line.push(t0, 20).is_ok());
        assert_eq!(line.pop_due(t0), Some(11));
    }

    /// **The spectator door, decided without a socket** (`NETCODE.md` §2.3):
    /// consent, naming by proven address, the per-target cap, "any" skipping
    /// a full target, and a stale key row never standing in for the slot's
    /// current tenant. `pick_target` is the whole door, so each rule is one
    /// case here; `tests/spectate_wire.rs` drives the same rules over a
    /// socket.
    #[test]
    fn the_spectator_door_picks_only_consenting_current_tenants_under_the_cap() {
        let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
        let mut seats: [SeatSlot; MAX_SPECTATORS] = std::array::from_fn(|_| SeatSlot::default());
        let a = protocol::Address([0xA; protocol::ADDRESS_BYTES]);
        let b = protocol::Address([0xB; protocol::ADDRESS_BYTES]);
        let h = protocol::Address([0xC; protocol::ADDRESS_BYTES]);
        let z = protocol::Address([0xD; protocol::ADDRESS_BYTES]);
        // 0: a player who declared nothing. 1: an agent. 2: a human who
        // opted in. 3: an agent whose slot turned over (stale row).
        for (s, address, agent, watchable) in [
            (0, b, false, false),
            (1, a, true, true),
            (2, h, false, true),
            (3, z, true, true),
        ] {
            keys[s] = KeySlot {
                generation: 7,
                address,
                agent,
                watchable,
                ..KeySlot::default()
            };
        }
        let live = |t: usize| match t {
            0..=2 => Some(7),
            3 => Some(8), // re-claimed: the row above is the previous tenant's
            _ => None,
        };
        use protocol::{REFUSE_WATCH, REFUSE_WATCH_FULL};
        assert_eq!(pick_target(&keys, live, &seats, a, false, 2), Ok(1));
        assert_eq!(
            pick_target(&keys, live, &seats, protocol::Address::GUEST, false, 2),
            Ok(1)
        );
        assert_eq!(
            pick_target(&keys, live, &seats, b, false, 2),
            Err(REFUSE_WATCH),
            "no consent"
        );
        assert_eq!(
            pick_target(&keys, live, &seats, h, false, 2),
            Err(REFUSE_WATCH),
            "no human feeds"
        );
        assert_eq!(
            pick_target(&keys, live, &seats, h, true, 2),
            Ok(2),
            "behind a delay"
        );
        assert_eq!(
            pick_target(&keys, live, &seats, z, false, 2),
            Err(REFUSE_WATCH),
            "stale row"
        );
        // Fill the agent's two seats: by name it is FULL; "any" moves on to
        // the next consenting player when human feeds run, and is FULL when
        // there is none.
        for (i, seat) in seats.iter_mut().take(2).enumerate() {
            *seat = SeatSlot {
                occupied: true,
                generation: i as u32 + 1,
                target: 1,
                target_gen: 7,
                ..SeatSlot::default()
            };
        }
        assert_eq!(
            pick_target(&keys, live, &seats, a, false, 2),
            Err(REFUSE_WATCH_FULL)
        );
        assert_eq!(
            pick_target(&keys, live, &seats, protocol::Address::GUEST, false, 2),
            Err(REFUSE_WATCH_FULL)
        );
        assert_eq!(
            pick_target(&keys, live, &seats, protocol::Address::GUEST, true, 2),
            Ok(2)
        );
        // A seat on a PREVIOUS tenant of slot 1 does not count against this one.
        seats[1].target_gen = 6;
        assert_eq!(pick_target(&keys, live, &seats, a, false, 2), Ok(1));
    }

    // The admission-gate ordering and the sample period are asserted at
    // COMPILE time beside their constants (`const _: () = assert!(…)`), not
    // here. They were tests until clippy pointed out that an assertion over
    // constants is one the compiler can make — which is the better gate:
    // a wrong constant fails the build rather than a suite.

    /// **The readback is the feature, so the gate is on the readback.**
    ///
    /// It deliberately does NOT assert we got 8 MiB: on an un-tuned box
    /// (this one, and CI) `net.core.rmem_max` clamps the request to ~208 KiB
    /// and that is the honest answer. What must hold is that both numbers
    /// are recorded, so an operator can see the gap rather than trusting
    /// that a `setsockopt` which cannot fail did something. A test that
    /// demanded 8 MiB would fail on every box that has not run the sysctl,
    /// which is the "a wall that cannot run is not a wall" trap in reverse:
    /// a wall that only passes on a tuned box teaches people to skip it.
    #[test]
    fn the_socket_buffer_records_what_it_got_not_what_it_asked() {
        let stats = ShardStats::default();
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("static addr");
        let sock = bind_udp(addr, &stats).expect("bind loopback");
        assert!(sock.local_addr().is_ok(), "socket must be bound");

        let asked = ShardStats::get(&stats.net_rcvbuf_asked);
        let got = ShardStats::get(&stats.net_rcvbuf_bytes);
        assert_eq!(
            asked, UDP_BUF_BYTES as u64,
            "asked-for size must be recorded"
        );
        assert!(
            got > 0,
            "granted receive buffer must be read back, got {got}"
        );
        assert!(
            ShardStats::get(&stats.net_sndbuf_asked) == UDP_BUF_BYTES as u64
                && ShardStats::get(&stats.net_sndbuf_bytes) > 0,
            "the send side records both numbers too"
        );
    }

    #[test]
    fn agent_seats_hold_the_rest_for_people() {
        // Off: anyone, however many agents are in.
        assert!(agent_seat_free(0, 99, true));
        // Under the cap an agent sits; at it, the next is refused.
        assert!(agent_seat_free(80, 79, true));
        assert!(!agent_seat_free(80, 80, true));
        assert!(!agent_seat_free(80, 81, true));
        // A person is never counted against it.
        assert!(agent_seat_free(80, 80, false));
    }

    #[test]
    fn an_agent_only_world_seats_agents_and_spectators_and_no_person() {
        // Every other shard seats every kind.
        for (agent, spectator) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(admits_kind(false, agent, spectator));
        }
        assert!(!admits_kind(true, false, false), "a person is refused");
        assert!(admits_kind(true, true, false), "an agent plays");
        assert!(admits_kind(true, false, true), "a spectator watches");
        assert!(admits_kind(true, true, true));
    }

    /// The chat limiter is the only wall between one client and everyone
    /// else's screen, so its shape is pinned: a burst of `CHAT_BURST`
    /// goes, the next is refused, and tokens come back one per
    /// `CHAT_REFILL` — never faster, and never *all at once* after a long
    /// silence, which is the bug a naive "top up from `last`" writes.
    #[test]
    fn chat_limiter_spends_a_burst_then_drips() {
        let t0 = Instant::now();
        let mut lim = ChatLimiter::new();
        for i in 0..CHAT_BURST {
            assert!(lim.allow(t0), "burst line {i} refused");
        }
        assert!(!lim.allow(t0), "the line past the burst must be refused");

        // Half a refill buys nothing.
        assert!(!lim.allow(t0 + CHAT_REFILL / 2));
        // A whole one buys exactly one.
        assert!(lim.allow(t0 + CHAT_REFILL));
        assert!(!lim.allow(t0 + CHAT_REFILL));

        // A long silence fills the bucket and no further — and, crucially,
        // the next long-silence check does not refill it again from a
        // stale timestamp.
        let quiet = t0 + CHAT_REFILL * 1_000;
        for i in 0..CHAT_BURST {
            assert!(lim.allow(quiet), "post-silence line {i} refused");
        }
        assert!(
            !lim.allow(quiet),
            "a stale accounting timestamp is a limiter that stopped limiting"
        );
        assert!(!lim.allow(quiet + CHAT_REFILL / 2));
    }

    /// And the limiter is actually *wired*: the reader's chat path runs
    /// decode → limit → ring in that order, and each refusal moves its own
    /// counter. Deleting the limiter call would leave the bucket test above
    /// green and this one red, which is the point of testing the wiring
    /// rather than the struct.
    #[test]
    fn accept_chat_decodes_then_limits_then_rings() {
        let stats = ShardStats::default();
        let mut limiter = ChatLimiter::new();
        let (mut tx, mut rx) = RingBuffer::<ChatMsg>::new(CHAT_RING_CAP);
        let t0 = Instant::now();
        let mut frame = [0u8; MAX_STREAM_MSG_BYTES];
        let n = protocol::encode_chat(b"hearth is up", protocol::Channel::Local, &mut frame)
            .expect("encodes");
        // The demux itself: this frame is chat, and an action frame is not.
        assert_eq!(peek_kind(&frame[..n]), Ok(KIND_CHAT));
        let mut act = [0u8; MAX_STREAM_MSG_BYTES];
        let an = protocol::encode_action_cancel(0, &mut act).expect("encodes");
        assert_eq!(peek_kind(&act[..an]), Ok(protocol::KIND_ACTION));

        // The burst is accepted and ringed; everything past it is refused
        // by the limiter, not by the ring.
        for _ in 0..CHAT_BURST {
            accept_chat(&frame[..n], &mut limiter, t0, &mut tx, &stats);
        }
        assert_eq!(ShardStats::get(&stats.chat_ok), CHAT_BURST as u64);
        assert_eq!(ShardStats::get(&stats.chat_rate_limited), 0);
        for _ in 0..5 {
            accept_chat(&frame[..n], &mut limiter, t0, &mut tx, &stats);
        }
        assert_eq!(
            ShardStats::get(&stats.chat_ok),
            CHAT_BURST as u64,
            "a line past the burst was ringed — the limiter is not wired in"
        );
        assert_eq!(ShardStats::get(&stats.chat_rate_limited), 5);

        // Nothing but the burst reached the ring.
        let mut ringed = 0;
        while rx.pop().is_ok() {
            ringed += 1;
        }
        assert_eq!(ringed, CHAT_BURST as usize);

        // A full ring drops the line and counts it as dropped, never as ok.
        let (mut full_tx, _full_rx) = RingBuffer::<ChatMsg>::new(CHAT_RING_CAP);
        let mut fresh = ChatLimiter::new();
        let ok_before = ShardStats::get(&stats.chat_ok);
        for i in 0..CHAT_RING_CAP + 2 {
            accept_chat(
                &frame[..n],
                &mut fresh,
                t0 + CHAT_REFILL * i as u32,
                &mut full_tx,
                &stats,
            );
        }
        assert_eq!(
            ShardStats::get(&stats.chat_ok) - ok_before,
            CHAT_RING_CAP as u64
        );
        assert_eq!(ShardStats::get(&stats.chat_ring_drops), 2);

        // Bytes that fail the text rules are counted and swallowed — the
        // session is not dropped the way a bad action frame drops it.
        let bad = [KIND_CHAT as u8 | 0x08, 0xff, 0xff];
        accept_chat(&bad, &mut fresh, t0, &mut tx, &stats);
        assert_eq!(ShardStats::get(&stats.chat_bad), 1);
    }

    /// **The bytes a lane counts are the bytes that crossed it, not the
    /// bytes it approved of** (NOW.md §0q item 4, "real bytes").
    ///
    /// The naive place to count an arriving datagram is beside
    /// `input_dg_ok`, which is one line lower and reads identically at a
    /// glance — and it would produce a bandwidth number that falls when a
    /// shard is fed garbage, i.e. lowest under exactly the load the number
    /// exists to describe. So the pair is bumped *first*, and this asserts
    /// the two properties that says: a malformed datagram still costs its
    /// bytes, and the arrival count reconciles with the three verdict
    /// counters that partition it.
    /// A voice ring nobody drains, for the input-lane tests.
    fn voice_sink() -> VoiceIn {
        VoiceIn::new(RingBuffer::<protocol::VoiceFrame>::new(VOICE_RING_CAP).0)
    }

    /// Voice is its own datagram on the input lane (v113): decoded, ringed,
    /// rate-limited past the burst, and none of it reaches the input ring.
    #[test]
    fn voice_datagrams_ring_apart_from_input_and_are_rate_limited() {
        let stats = ShardStats::default();
        let (mut tx, mut rx) = RingBuffer::<protocol::InputDatagram>::new(INPUT_RING_CAP);
        let (vtx, mut vrx) = RingBuffer::<protocol::VoiceFrame>::new(64);
        let mut voice = VoiceIn::new(vtx);
        let mut buf = [0u8; protocol::VOICE_DATAGRAM_MAX];
        for seq in 0..(VOICE_BURST + 5) as u16 {
            let f = protocol::VoiceFrame::new(seq, 0, &[0x0b, 1, 2]).unwrap();
            let n = protocol::encode_voice_up(&f, &mut buf).unwrap();
            accept_input(&buf[..n], &mut tx, &mut voice, &stats);
        }
        // A truncated frame: the kind says voice, the body is short.
        let f = protocol::VoiceFrame::new(1, 0, &[0x0b, 1, 2]).unwrap();
        let n = protocol::encode_voice_up(&f, &mut buf).unwrap();
        accept_input(&buf[..n - 1], &mut tx, &mut voice, &stats);
        assert!(rx.pop().is_err(), "voice never reaches the input ring");
        assert_eq!(ShardStats::get(&stats.voice_ok), VOICE_BURST as u64);
        assert_eq!(ShardStats::get(&stats.voice_rate_limited), 5);
        assert_eq!(ShardStats::get(&stats.voice_bad), 1);
        assert_eq!(ShardStats::get(&stats.input_dg_bad), 0);
        assert_eq!(vrx.pop().unwrap().seq, 0);
    }

    #[test]
    fn the_lane_counts_the_bytes_that_arrived_not_the_ones_it_liked() {
        use protocol::{encode_input, InputDatagram};
        use sim_core::input::InputFrame;

        let stats = ShardStats::default();
        let (mut tx, _rx) = RingBuffer::<InputDatagram>::new(INPUT_RING_CAP);

        let mut dg = InputDatagram::new(0, 0, 0);
        dg.push(InputFrame::default()).expect("one frame fits");
        let mut buf = [0u8; 256];
        let good = encode_input(&dg, &mut buf).expect("encodes");
        accept_input(&buf[..good], &mut tx, &mut voice_sink(), &stats);
        assert_eq!(ShardStats::get(&stats.net_dg_in_count), 1);
        assert_eq!(
            ShardStats::get(&stats.net_dg_in_bytes),
            good as u64,
            "the counter must carry the datagram's real length, not a \
             budget, a header size, or a count of frames"
        );

        // Line noise: `peek_kind` refuses it. It crossed the path anyway.
        let junk = [0xEEu8; 37];
        accept_input(&junk, &mut tx, &mut voice_sink(), &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_bad), 1);
        assert_eq!(
            ShardStats::get(&stats.net_dg_in_bytes),
            good as u64 + junk.len() as u64,
            "a datagram the decoder refused still cost its bytes"
        );

        // A forged in-layout value: decodes, refused, and still counted.
        // 0x20 became the hand-revive hold in v66. Derive a spare bit so
        // this fixture keeps testing a value the current wire cannot mean.
        let unmeant = 1u8 << BTN_MASK.trailing_ones();
        assert_ne!(unmeant, 0, "the octet is full — this probe needs a new bit");
        assert_eq!(unmeant & BTN_MASK, 0, "the probe bit must be unmeant");
        let mut dg = InputDatagram::new(0, 0, 0);
        dg.push(InputFrame {
            buttons: BTN_MASK | unmeant,
            ..InputFrame::default()
        })
        .expect("one frame fits");
        let forged = encode_input(&dg, &mut buf).expect("encodes");
        accept_input(&buf[..forged], &mut tx, &mut voice_sink(), &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 1);

        // The reconciliation: every arrival lands in exactly one verdict
        // counter, so the arrival count is their sum. A byte counter that
        // agrees with nothing else is a number nobody can check.
        assert_eq!(ShardStats::get(&stats.net_dg_in_count), 3);
        assert_eq!(
            ShardStats::get(&stats.net_dg_in_count),
            ShardStats::get(&stats.input_dg_ok)
                + ShardStats::get(&stats.input_dg_bad)
                + ShardStats::get(&stats.input_dg_forged),
            "the arrival count must partition into the verdict counters"
        );
        assert_eq!(
            ShardStats::get(&stats.net_dg_in_bytes),
            (good + junk.len() + forged) as u64
        );
    }

    /// The stream lanes count the **prefix** too, because it is on the wire.
    ///
    /// A frame counter that reported `payload.len()` would understate the
    /// event lane by two bytes per frame — and the event lane is the one
    /// that sends many small frames, so the error is largest where the lane
    /// is busiest. Pinned against `write_frame`'s own arithmetic rather
    /// than against the literal 2.
    #[test]
    fn a_stream_frame_costs_its_prefix() {
        assert_eq!(
            FRAME_PREFIX_BYTES,
            (0u16).to_le_bytes().len(),
            "the byte counters add the prefix write_frame actually makes"
        );
        let stats = ShardStats::default();
        ShardStats::add_msg(
            &stats.net_stream_out_frames,
            &stats.net_stream_out_bytes,
            FRAME_PREFIX_BYTES + 7,
        );
        assert_eq!(ShardStats::get(&stats.net_stream_out_frames), 1);
        assert_eq!(ShardStats::get(&stats.net_stream_out_bytes), 9);
    }

    /// NOW.md §5b: the wire carries `buttons` as a full octet and the sim
    /// means only `BTN_MASK` — a frame carrying an unknown bit is refused
    /// at the accept boundary, through the **real** encode → decode path,
    /// and nothing of that datagram reaches the ring (the refusal is
    /// ordered before the mutation). The value just inside the boundary —
    /// every meaningful bit at once — still crosses.
    ///
    /// **The outside probe is derived from `BTN_MASK`, not typed**, since
    /// torch fuel v0: it used the literal `0x10`, and bit 4 became
    /// `BTN_LIGHT`, so the "just outside" case quietly became a legal
    /// frame. It failed loudly here only because the accept counter moved
    /// — which is luck, the same shape as `SUB_MAX`'s probe. The lowest
    /// bit the mask does not claim cannot go stale that way.
    #[test]
    fn accept_input_refuses_buttons_the_sim_cannot_mean() {
        use protocol::{encode_input, InputDatagram};
        use sim_core::input::InputFrame;

        let stats = ShardStats::default();
        let (mut tx, mut rx) = RingBuffer::<InputDatagram>::new(INPUT_RING_CAP);
        let encode = |buttons: u8| {
            let mut dg = InputDatagram::new(0, 0, 0);
            dg.push(InputFrame {
                buttons,
                ..InputFrame::default()
            })
            .expect("one frame fits");
            let mut buf = [0u8; 256];
            let n = encode_input(&dg, &mut buf).expect("encodes");
            (buf, n)
        };

        // Just inside: every meaningful bit at once.
        let (buf, n) = encode(BTN_MASK);
        accept_input(&buf[..n], &mut tx, &mut voice_sink(), &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_ok), 1);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 0);
        let ringed = rx.pop().expect("the in-domain datagram is ringed");
        assert_eq!(ringed.frames()[0].buttons, BTN_MASK);

        // Just outside: the lowest bit `BTN_MASK` does not claim, taken
        // FROM the mask so declaring a new button walks the probe along
        // instead of retiring it. The encoder writes it happily (the field
        // is 8 wide since v0), which is exactly the slack being closed
        // here.
        let unmeant = 1u8 << BTN_MASK.trailing_ones();
        assert_ne!(unmeant, 0, "the octet is full — this probe needs a new bit");
        assert_eq!(unmeant & BTN_MASK, 0, "the probe bit must be unmeant");
        let (buf, n) = encode(BTN_MASK | unmeant);
        accept_input(&buf[..n], &mut tx, &mut voice_sink(), &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 1);
        assert_eq!(
            ShardStats::get(&stats.input_dg_ok),
            1,
            "a forged datagram must not also count as accepted"
        );
        assert_eq!(
            ShardStats::get(&stats.input_dg_bad),
            0,
            "forged is not malformed — the bytes decode fine"
        );
        assert!(
            rx.pop().is_err(),
            "the forged datagram reached the ring — the refusal is not \
             ordered before the mutation"
        );

        // A valid tail does not ride a forged head: one datagram, two
        // frames, only the second in-domain — the whole datagram drops,
        // exactly as decode's own `sel` refusal drops it.
        let mut dg = InputDatagram::new(0, 0, 0);
        dg.push(InputFrame {
            buttons: 0x80,
            ..InputFrame::default()
        })
        .expect("first frame fits");
        dg.push(InputFrame {
            seq: 1,
            buttons: BTN_MASK,
            ..InputFrame::default()
        })
        .expect("second frame fits");
        let mut buf = [0u8; 256];
        let n = encode_input(&dg, &mut buf).expect("encodes");
        accept_input(&buf[..n], &mut tx, &mut voice_sink(), &stats);
        assert_eq!(ShardStats::get(&stats.input_dg_forged), 2);
        assert!(rx.pop().is_err(), "no frame of a forged datagram survives");
    }

    /// **`KeySlot`'s id match** (`NOW.md` §0y item 4): a record the sim hands
    /// back is filed under a slot's key only when it carries that slot's
    /// *current* id. The ring here holds the case the ordering argument says
    /// cannot happen — the previous tenant's record arriving after the slot
    /// was re-claimed — so the id check is the only thing between it and the
    /// new tenant's file. Matching on the slot byte alone files it under the
    /// new key, last, and hands them the old tenant's inventory.
    #[test]
    fn a_save_is_filed_only_under_the_tenant_whose_id_it_carries() {
        use sim_core::persist::PlayerSave;
        let (mut save_tx, mut save_rx) = RingBuffer::<SaveMsg>::new(8);
        let (mut write_tx, mut write_rx) = RingBuffer::<WriteMsg>::new(8);
        let mut store = SaveStore::new();
        let stats = Arc::new(ShardStats::default());
        let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
        let id = |gen: u32, slot: usize| (gen << 8) | slot as u32;
        let tenant = PlayerKey::new(b"slot-3-generation-2").expect("fits");
        let evicted = PlayerKey::new(b"an-evicted-sleeper").expect("fits");
        // Slot 3 was re-claimed: generation 2 holds it now. Slot 4 is a guest.
        keys[3] = KeySlot {
            key: Some(tenant),
            id: id(2, 3),
            ..KeySlot::default()
        };
        keys[4] = KeySlot {
            key: None,
            id: id(1, 4),
            ..KeySlot::default()
        };
        let save = |qx: i32| {
            let mut s = PlayerSave::EMPTY;
            s.body.qx = qx;
            s
        };
        for (id, key, qx) in [
            (id(2, 3), None, 1),           // slot 3's current tenant
            (id(1, 3), None, 2),           // its previous tenant, late
            (id(1, 4), None, 3),           // a guest: remembered by nobody
            (id(7, 5), None, 4),           // a slot nobody holds
            (id(1, MAX_PLAYERS), None, 5), // a spectator seat's slot
            (id(1, 3), Some(evicted), 6),  // an eviction: the sim named the key
        ] {
            assert!(save_tx
                .push(SaveMsg {
                    id,
                    key,
                    save: save(qx)
                })
                .is_ok());
        }
        drain_saves(&mut save_rx, &mut write_tx, &mut store, &keys, &stats);

        // Which record is told by `qx` alone: the saves differ in nothing else,
        // and a whole `PlayerSave` in a failure message is a page of zeros.
        let first = write_rx.pop().expect("the current tenant's record");
        assert_eq!((first.key, first.save.body.qx), (tenant, 1));
        let second = write_rx.pop().expect("the eviction record, filed as named");
        assert_eq!((second.key, second.save.body.qx), (evicted, 6));
        assert!(
            write_rx.pop().is_err(),
            "a record for a tenant no slot holds reached the disk"
        );
        assert_eq!(
            store.find(&tenant).map(|s| s.body.qx),
            Some(1),
            "the previous tenant's record was filed under the new one's key"
        );
        assert_eq!(store.live(), 2);
        assert_eq!(ShardStats::get(&stats.saves_taken), 2);
        assert_eq!(ShardStats::get(&stats.save_ring_drops), 0);
    }

    /// **The accept loop waits out the sim's flush** (`NOW.md` §0y item 4):
    /// the shutdown drain files every record pushed until the sim thread
    /// drops its producer, including the ones that land after the flag was
    /// seen. Returning on the first look, which is what the loop once did,
    /// files the first record here and loses the rest. The producer is held
    /// open on a thread, the way the sim thread holds it while it disconnects
    /// everyone.
    #[tokio::test]
    async fn the_shutdown_drain_files_records_until_the_sim_lets_go() {
        let (mut save_tx, mut save_rx) = RingBuffer::<SaveMsg>::new(8);
        let (mut write_tx, mut write_rx) = RingBuffer::<WriteMsg>::new(8);
        let mut store = SaveStore::new();
        let stats = Arc::new(ShardStats::default());
        let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
        let who: Vec<PlayerKey> = (0..3)
            .map(|s| PlayerKey::new(format!("player-{s}").as_bytes()).expect("fits"))
            .collect();
        for (s, key) in who.iter().enumerate() {
            keys[s] = KeySlot {
                key: Some(*key),
                id: (1 << 8) | s as u32,
                ..KeySlot::default()
            };
        }
        let rec = |s: u32| SaveMsg {
            id: (1 << 8) | s,
            key: None,
            save: sim_core::persist::PlayerSave::EMPTY,
        };
        assert!(save_tx.push(rec(0)).is_ok());
        let sim = std::thread::spawn(move || {
            for s in 1..3 {
                std::thread::sleep(Duration::from_millis(50));
                assert!(save_tx.push(rec(s)).is_ok());
            }
            // `save_tx` drops here: the flush is over.
        });
        drain_until_abandoned(&mut save_rx, &mut write_tx, &mut store, &keys, &stats).await;
        sim.join().expect("the sim half finished");
        let filed: Vec<PlayerKey> = std::iter::from_fn(|| write_rx.pop().ok())
            .map(|w| w.key)
            .collect();
        assert_eq!(filed, who, "the drain stopped before the sim let go");
    }

    /// **A wallet is read once per cooldown, however often it rejoins.** A
    /// join inside the cooldown is told the last answer and owes one read
    /// for when it has run; an answer older than the cooldown is never told.
    #[test]
    fn a_wallet_is_read_once_per_cooldown_however_often_it_rejoins() {
        let mut r = PlatformReads::<u8>::new(Duration::from_secs(15));
        let k = PlayerKey::new(b"0x00000000000000000000000000000000000000a1").unwrap();
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        assert!(r.ask(&k, true, t0), "the first join reads");
        assert!(!r.ask(&k, true, s(1)), "a read is out");
        assert_eq!(r.known(&k, s(1)), None);
        r.done(&k, Some(7));
        assert_eq!(r.known(&k, s(2)), Some(7), "a rejoin is told it");
        assert!(!r.ask(&k, true, s(2)), "inside the cooldown");
        assert!(!r.ask(&k, false, s(3)), "a client ask inside it too");
        assert_eq!(r.owed(s(14)), None, "owed only once the cooldown has run");
        assert_eq!(r.owed(s(15)), Some(k));
        assert_eq!(r.owed(s(15)), None, "handed out once");
        assert!(r.ask(&k, false, s(15)));
        assert_eq!(r.known(&k, s(16)), None, "older than the cooldown");
        r.done(&k, None);
        assert!(r.ask(&k, false, s(31)), "could-not-look leaves nothing out");
    }

    /// **The read memory is bounded and forgets the stalest wallet.**
    #[test]
    fn the_read_memory_is_bounded_and_forgets_the_stalest() {
        let mut r = PlatformReads::<u8>::new(Duration::from_secs(15));
        let key = |i: usize| PlayerKey::new(format!("0x{i:040x}").as_bytes()).unwrap();
        let t0 = Instant::now();
        for i in 0..=READ_MEMORY {
            assert!(r.ask(&key(i), true, t0 + Duration::from_millis(i as u64)));
        }
        assert_eq!(r.recent.len(), READ_MEMORY);
        let late = t0 + Duration::from_secs(1);
        assert!(!r.ask(&key(READ_MEMORY), true, late), "the newest is kept");
        assert!(r.ask(&key(0), true, late), "the stalest was forgotten");
    }

    /// **A platform answer lands on whoever holds the wallet now**: the
    /// tenant it was asked for, or the same wallet's new seat after a
    /// reconnect, and never a stranger who took the old slot.
    #[test]
    fn a_platform_answer_lands_on_whoever_holds_the_wallet_now() {
        fn seat(
            slots: &SlotTable,
            keys: &mut [KeySlot; MAX_PLAYERS],
            slot: usize,
            key: Option<PlayerKey>,
        ) -> u32 {
            let generation = slots.claim(slot).unwrap();
            keys[slot] = KeySlot {
                key,
                generation,
                ..KeySlot::default()
            };
            generation
        }
        let slots = SlotTable::new(MAX_CONNS);
        let mut keys: [KeySlot; MAX_PLAYERS] = std::array::from_fn(|_| KeySlot::default());
        let w = PlayerKey::new(b"0x00000000000000000000000000000000000000b2").unwrap();
        let g1 = seat(&slots, &mut keys, 2, Some(w));
        assert_eq!(landing(2, g1, Some(&w), &keys, &slots), Some(2));
        slots.mark_leaving(2, g1);
        slots.free(2, g1);
        seat(&slots, &mut keys, 5, Some(w));
        assert_eq!(
            landing(2, g1, Some(&w), &keys, &slots),
            Some(5),
            "reconnected"
        );
        seat(&slots, &mut keys, 2, None);
        assert_eq!(landing(2, g1, None, &keys, &slots), None, "a guest's, gone");
        assert_eq!(
            landing(2, g1, Some(&w), &keys, &slots),
            Some(5),
            "never the stranger now on slot 2"
        );
    }

    /// **Refusals posting at once are capped**, and each gives its place
    /// back when it ends.
    #[test]
    fn refusals_posting_at_once_are_capped_and_give_their_place_back() {
        static GATE: RefusalGate = RefusalGate::new();
        let mut out: Vec<_> = (0..REFUSALS_MAX).map(|_| GATE.take().unwrap()).collect();
        assert!(GATE.take().is_none(), "one past the cap");
        out.pop();
        assert!(
            GATE.take().is_some(),
            "an ended refusal gives its place back"
        );
    }

    /// **A side file pushed as the sim lets go is on disk when the store
    /// thread returns.** The ledger rides the same exit as the world: the
    /// thread drains every ring after it sees all three producers dropped.
    #[test]
    fn the_store_thread_writes_a_side_file_before_it_stops() {
        let dir = std::env::temp_dir().join(format!("gates-sidefile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("island.world.gifts");
        let _ = std::fs::remove_file(&path);
        let (write_tx, write_rx) = RingBuffer::<WriteMsg>::new(1);
        let (world_tx, world_rx) = RingBuffer::<WorldMsg>::new(1);
        let (world_done_tx, _world_done_rx) = RingBuffer::<WorldDone>::new(1);
        let (mut file_tx, file_rx) = RingBuffer::<FileMsg>::new(FILE_RING_CAP);
        let store = std::thread::spawn(move || {
            store_thread(
                SaveFile::closed(),
                write_rx,
                crate::worldfile::WorldFile::closed(),
                world_rx,
                world_done_tx,
                file_rx,
                Arc::new(ShardStats::default()),
            )
        });
        file_tx
            .push(FileMsg {
                path: path.clone(),
                text: "the ledger\n".into(),
            })
            .unwrap();
        drop((write_tx, world_tx, file_tx));
        store.join().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "the ledger\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The evicted ids land only beside a world that landed** (the r5
    /// session#3 finding): written ahead of the world, a crash paired an
    /// older world with a newer id map. A world write that fails writes no
    /// `.evicted` and hands it back; with no world file it is dropped, not
    /// owed; a world that lands takes it with it.
    #[test]
    fn the_evicted_ids_land_only_with_their_world() {
        let dir = std::env::temp_dir().join(format!("gates-evictside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let side = dir.join("island.world.evicted");
        let run = |world: crate::worldfile::WorldFile, text: &str| -> WorldDone {
            let (write_tx, write_rx) = RingBuffer::<WriteMsg>::new(1);
            let (mut world_tx, world_rx) = RingBuffer::<WorldMsg>::new(1);
            let (world_done_tx, mut world_done_rx) = RingBuffer::<WorldDone>::new(1);
            let (file_tx, file_rx) = RingBuffer::<FileMsg>::new(FILE_RING_CAP);
            let msg = WorldMsg {
                tick: 1,
                len: 0,
                buf: vec![0u8; 8].into_boxed_slice(),
                idents: Vec::new(),
                side: Some(FileMsg {
                    path: side.clone(),
                    text: text.into(),
                }),
            };
            assert!(world_tx.push(msg).is_ok());
            let store = std::thread::spawn(move || {
                store_thread(
                    SaveFile::closed(),
                    write_rx,
                    world,
                    world_rx,
                    world_done_tx,
                    file_rx,
                    Arc::new(ShardStats::default()),
                )
            });
            drop((write_tx, world_tx, file_tx));
            store.join().unwrap();
            world_done_rx.pop().unwrap()
        };
        let world_at = |path: &std::path::Path| {
            let mut w = sim_core::world::World::new(1);
            crate::worldfile::open(path, &mut w, 1, 0, 0, 0, 1)
                .expect("a fresh path opens")
                .0
                .file
        };

        let done = run(crate::worldfile::WorldFile::closed(), "none\n");
        assert!(done.ok && !done.side_lost, "no world: dropped, not owed");
        assert!(!side.exists());

        let done = run(world_at(&dir.join("gone").join("island.world")), "lost\n");
        assert!(!done.ok && done.side_lost, "a failed world hands it back");
        assert!(!side.exists(), "written without its world");

        let done = run(world_at(&dir.join("island.world")), "landed\n");
        assert!(done.ok && !done.side_lost);
        assert!(dir.join("island.world").exists());
        assert_eq!(std::fs::read_to_string(&side).unwrap(), "landed\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
