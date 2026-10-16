//! The shard status endpoint: one thread, one `TcpListener`, one JSON
//! document. This is the piece three places named as missing — `stats.rs`'s
//! header ("read by … later the status page"), `NOW.md` §0v item 2, and the
//! shard-list row in `DECISIONS.md` §open ("that endpoint is the whole of
//! what would light the counts"). `ci/shardlist.py`'s `players` field is the
//! eventual consumer; today both readers draw `?` for the absent count.
//!
//! ## The document
//!
//! `GET /status.json` answers `200` with exactly this shape, integers only
//! (`stats.rs` L5: diagnostics are numbers, not strings):
//!
//! ```json
//! {"players":3,"agents":1,"spectators":1,"max_players":100,"proto":62,"tick":123456,
//!  "dg_out_bytes":0,"dg_out_pkts":0,"dg_in_bytes":0,"dg_in_pkts":0,
//!  "stream_out_bytes":0,"stream_out_frames":0,
//!  "stream_in_bytes":0,"stream_in_frames":0,
//!  "aim_stale_samples":0,"aim_stale_sum":0,"aim_stale_max":0,
//!  "aim_stale_unacked":0,"aim_stale_refused":0,
//!  "aim_stale_hist":[0,0,0,0,0,0,0,0],
//!  "favour_granted":0,"favour_sum":0,
//!  "favour_clamped":0,"favour_disagree":0,"next_wipe":0,
//!  "wipe":1,"act":1,"works_lit":0,"works_total":6,"restart_at":0}
//! ```
//!
//! (one line on the wire; wrapped here to fit)
//!
//! - `players` — bodies with a live connection, off the `players` gauge the
//!   sim loop mirrors from `ShardCore::connected` each tick. A gauge and
//!   not `joins - leaves`, because that pair legitimately drifts
//!   (`stats.rs` says how).
//! - `agents` — how many of `players` joined as a declared agent
//!   (`HELLO_AGENT`), off the gauge the sim loop mirrors from
//!   `ShardCore::agents_connected`. A share of `players`, never a separate
//!   population: the people on the shard are `players - agents`.
//! - `spectators` — occupied spectator seats (`NETCODE.md` §2.3), off the
//!   gauge the sim loop mirrors from `ShardCore::spectators`. Never counted
//!   in `players`: a seat has no body and does not take a player slot.
//! - `proto` — `protocol::PROTO_VER`, the wire this shard speaks. What a
//!   publish reads before shipping a client (`ci/publish_web.sh`,
//!   `ci/publish_depot.py`): a client on another number is `REFUSE_VERSION`
//!   at the handshake, and nothing else relates a shard to a published build.
//! - `max_players` — `sim_core::limits::MAX_PLAYERS`, the cap the shard
//!   actually enforces (the same source `ci/shardlist.py` pins for the same
//!   stated reason: a published cap that disagrees with the enforced one is
//!   a lie the player discovers at a refused join).
//! - `tick` — `stats.current_tick`, which doubles as a liveness check: a
//!   number that stops moving is a shard that stopped ticking.
//! - the eight **lane byte** counters, straight off `ShardStats` (see the
//!   lane-bytes block in `stats.rs` for why they are four pairs). They are
//!   monotonic totals since boot, so a rate is two polls and a subtraction,
//!   and a **per-client** rate is that divided by `players` — which is why
//!   they live beside it rather than in a second document. This is the half
//!   `NOW.md` §0q item 4 was missing: the soak's 16.5 kB/s/client was a
//!   ceiling derived from budgets because nothing counted a byte.
//! - the six **aim staleness** readings, off `ShardStats` (see the
//!   aim-staleness block in `stats.rs`). `aim_stale_sum / aim_stale_samples`
//!   is the mean `T − S` in ticks, `aim_stale_max` is the tail, and
//!   `aim_stale_hist` is the distribution — index `n` is exactly `n` ticks
//!   up to the last, which is "that many **or more**"
//!   (`stats::AIM_STALE_BUCKETS`). `aim_stale_unacked` and
//!   `aim_stale_refused` are the two exclusions, published so the sample
//!   count can be checked against the frames the shard actually ran rather
//!   than trusted. **These are raw `T − S`**: the interpolation delay is
//!   not in them and `stats.rs` says why.
//!
//!   `aim_stale_hist` is the one array in the document. Integers still
//!   (`stats.rs` L5 bans strings, not shape) — the alternative was eight
//!   flat fields whose names would encode the bucket edges, which puts the
//!   thing a reader must know in a key instead of in a doc comment.
//! - the four **favour** counters — what the shard did with the readings
//!   above (the favour block in `stats.rs`). `favour_granted` is frames
//!   that got a nonzero rewind and `favour_sum / favour_granted` is their
//!   mean depth in ticks; **`favour_granted` climbing at all is how an
//!   operator sees that lag compensation is on**, which is worth a field
//!   because it was *off* for three passes with every gate green.
//!   `favour_clamped` is the fraction of those that hit
//!   `REWIND_MAX_TICKS`, i.e. how often the 233 ms ceiling is the binding
//!   constraint rather than the link — the evidence for moving that
//!   number. `favour_disagree` is the cross-check: claims corrected down
//!   to the server's own ack evidence, and one of the two favour counters
//!   in `anomaly::WATCHED` (the other, `favour_playout_high`, is in the
//!   anomaly log only).
//!
//! - `next_wipe` — unix seconds of the next wipe, 0 for none (`wipe.rs`):
//!   what a server list prints as "wipes Thu 19:00".
//!
//! - `wipe`, `act`, `works_lit`, `works_total` — the server's progression
//!   (`ARC.md`): which wipe is running, the act, and how many works are lit
//!   of how many exist. What the island banner tells a player, for a list.
//!
//! - `restart_at` — unix seconds of a pending `/restart`, 0 for none
//!   (`wipe::Restart`). The key being there at all is how
//!   `ci/deploy_shard.sh --warn` knows this shard takes SIGUSR1 as a
//!   countdown, not as the default kill.
//!
//! ## What this thread is allowed to touch
//!
//! `ShardStats` atomics and constants — nothing else. It never speaks to the
//! sim thread, holds no lock (crate `clippy.toml`), and cannot block a tick:
//! the sim stores gauges it was already storing, and this thread loads them.
//! Walls 2/3 bind the sim thread, not this one, but it is bounded anyway
//! because every client-driven byte deserves a cap (wall 4's spirit):
//!
//! - a request is read up to [`STATUS_REQUEST_CAP`] bytes and no further —
//!   past the cap the connection is closed unanswered;
//! - the whole request and its response share one deadline,
//!   [`STATUS_IO_TIMEOUT_MS`] from accept — not a timeout per read, which a
//!   client trickling a byte a call renews forever;
//! - one response per connection, then close — no keep-alive, no pipelining;
//! - connections are answered serially on the one thread, so a slow client
//!   costs the next poller a bounded wait and never a thread.
//!
//! Malformed input gets a close, never a panic: garbage bytes, a non-GET
//! method and an over-cap request line all end the connection with nothing
//! written. A well-formed GET of any other path gets a `404`, because an
//! operator pointing `curl` at the wrong path deserves an answer that says
//! so rather than a hangup that blames the network.
//!
//! ## Lifecycle
//!
//! Daemon-style: the thread blocks in `accept` and is reaped by process
//! exit. The storage thread's drain-then-flag shutdown dance (`store.rs`)
//! exists because that thread owes bytes to a disk; this one owes nothing
//! to anybody, so `bin/shard.rs` neither signals nor joins it and shutdown
//! cannot hang on it.
//!
//! Serving this to the internet is publishing, and publishing stays an
//! operator act: the knob (`status_addr`, `config.rs`) is absent by
//! default, so a live shard changes nothing until its operator says where.

use crate::stats::ShardStats;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Most bytes of a request this will read before closing the connection
/// unanswered. The only line it acts on is the request line, and
/// `GET /status.json HTTP/1.1\r\n` is 26 bytes — the cap is generous
/// headroom for a real client's request line, not a buffer for headers,
/// which are never parsed. DECISIONS.md §open ("shard status endpoint v0").
pub const STATUS_REQUEST_CAP: usize = 512;

/// How long one connection may take, request and response together, in
/// milliseconds. A client still talking or still reading past it is closed;
/// at two seconds a slow-loris costs the next poller a bounded wait on the
/// serial accept loop, never a thread. DECISIONS.md §open ("shard status
/// endpoint v0").
pub const STATUS_IO_TIMEOUT_MS: u64 = 2000;

/// Bind `addr`, spawn the responder thread, and return the bound address —
/// which is the useful half when `addr` carries port 0 (the test path, the
/// same convention `ShardConfig::bind` states).
///
/// A bind failure is returned rather than logged, so a boot can refuse it:
/// an operator who set `status_addr` and got a shard silently serving
/// nothing would be the config-says-X-shard-does-Y defect `config.rs`
/// refuses everywhere else.
pub fn spawn_status(addr: SocketAddr, stats: Arc<ShardStats>) -> std::io::Result<SocketAddr> {
    spawn_status_with(addr, stats, None)
}

/// [`spawn_status`], also serving the files kept beside the world `base`
/// (the `world_file`, else the `save_file`), read from disk so this thread
/// still never speaks to the sim thread:
///
/// - `GET /standings.json` (`standings::json_path`) — a website's or a
///   bot's leaderboard;
/// - `GET /online.json` (`online::json_path`) — who is connected, rewritten
///   once a minute: the platform's "playing now" and hours on record;
/// - `GET /payout/<wipe>.json` (`standings::payout_path`) — what a closed
///   wipe owes each wallet, which the platform's cashier reads and pays.
///   The shard still holds no keys: it says who earned what, and the
///   operator names which shards the cashier believes.
pub fn spawn_status_with(
    addr: SocketAddr,
    stats: Arc<ShardStats>,
    base: Option<PathBuf>,
) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    std::thread::Builder::new()
        .name("status".into())
        .spawn(move || serve(listener, stats, base))?;
    Ok(bound)
}

/// The most of the standings file this will serve.
pub const STANDINGS_JSON_CAP: u64 = 256 * 1024;

/// The most of a payout file this will serve.
pub const PAYOUT_JSON_CAP: u64 = 256 * 1024;

/// `/payout/<wipe>.json` → the wipe, or `None` for any other path.
fn payout_wipe(path: &str) -> Option<u32> {
    let n = path.strip_prefix("/payout/")?.strip_suffix(".json")?;
    if n.is_empty() || n.len() > 10 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// Up to `cap` bytes of `p` as text, or `None` when it cannot be read.
fn read_capped(p: &Path, cap: u64) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    let mut text = String::new();
    f.take(cap).read_to_string(&mut text).ok()?;
    Some(text)
}

const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// The thread body: accept, answer, close, forever. An accept error backs
/// off briefly rather than spinning — the errors that recur (fd
/// exhaustion) are not fixed by asking again in the same microsecond.
fn serve(listener: TcpListener, stats: Arc<ShardStats>, base: Option<PathBuf>) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Refusals and dead sockets end the connection; neither is
                // worth a counter, let alone a panic. The one observable
                // that matters is that the NEXT request is answered, and
                // `tests/status.rs` asserts exactly that.
                let _ = answer(stream, &stats, base.as_deref());
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// A connection with one deadline for everything said on it. A socket
/// timeout is per call: a client sending a byte at a time renewed it on
/// every read, 512 reads of two seconds each, and held the one responder
/// thread — and `/payout`, which the cashier reads — for a quarter of an
/// hour. Each call here gets only the time that is left.
struct Deadlined {
    s: TcpStream,
    until: Instant,
}

impl Deadlined {
    fn arm(&self, read: bool) -> std::io::Result<()> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "status request deadline",
            ));
        }
        if read {
            self.s.set_read_timeout(Some(left))
        } else {
            self.s.set_write_timeout(Some(left))
        }
    }
}

impl Read for Deadlined {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.arm(true)?;
        self.s.read(buf)
    }
}

impl Write for Deadlined {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.arm(false)?;
        self.s.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.s.flush()
    }
}

/// Read one request, bounded; answer it or close. Every early `Ok(())` is
/// a deliberate refusal — the connection closes with nothing written.
fn answer(stream: TcpStream, stats: &ShardStats, base: Option<&Path>) -> std::io::Result<()> {
    let mut stream = Deadlined {
        s: stream,
        until: Instant::now() + Duration::from_millis(STATUS_IO_TIMEOUT_MS),
    };

    // Read until the request line is complete or the cap is hit. Headers
    // past the first `\r\n` may or may not have arrived; they are never
    // waited for and never read past the cap.
    let mut buf = [0u8; STATUS_REQUEST_CAP];
    let mut have = 0usize;
    let line_end = loop {
        if let Some(at) = buf[..have].windows(2).position(|w| w == b"\r\n") {
            break at;
        }
        if have == buf.len() {
            return Ok(()); // over cap with no request line: refused
        }
        let n = stream.read(&mut buf[have..])?;
        if n == 0 {
            return Ok(()); // EOF before a request line: refused
        }
        have += n;
    };

    // `GET /status.json HTTP/1.1` — method and path; the version is not
    // checked because nothing here depends on it.
    let Ok(line) = std::str::from_utf8(&buf[..line_end]) else {
        return Ok(()); // not text: refused
    };
    let mut parts = line.split(' ');
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Ok(()); // no space-separated method + path: refused
    };
    if method != "GET" {
        return Ok(()); // this endpoint speaks GET and nothing else
    }
    if let Some(wipe) = payout_wipe(path) {
        let body = base.and_then(|b| {
            read_capped(
                &crate::standings::payout_path(b, wipe, "json"),
                PAYOUT_JSON_CAP,
            )
        });
        let Some(body) = body else {
            return stream.write_all(NOT_FOUND);
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        return stream.write_all(body.as_bytes());
    }
    if path == "/standings.json" {
        let body =
            base.and_then(|b| read_capped(&crate::standings::json_path(b), STANDINGS_JSON_CAP));
        let Some(body) = body else {
            return stream.write_all(NOT_FOUND);
        };
        // Open to any origin: a leaderboard is for a website to show.
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Access-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        return stream.write_all(body.as_bytes());
    }
    if path == "/online.json" {
        // Who is on right now, for the platform (`online.rs`).
        let body = base.and_then(|b| {
            read_capped(&crate::online::json_path(b), crate::online::ONLINE_JSON_CAP)
        });
        let Some(body) = body else {
            return stream.write_all(NOT_FOUND);
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Access-Control-Allow-Origin: *\r\nCache-Control: no-store\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        return stream.write_all(body.as_bytes());
    }
    if path != "/status.json" {
        return stream.write_all(NOT_FOUND);
    }

    // The response string is this thread's one allocation per request, and
    // this thread is not the sim thread — correctness and boundedness over
    // cleverness (the fixed-buffer alternative buys nothing measurable
    // against a poller that asks every few seconds).
    // The histogram, `[a,b,…]` — built here so the one format! below stays
    // a flat list of integers like every other field.
    let mut hist = String::from("[");
    for (i, b) in stats.aim_stale_hist.iter().enumerate() {
        if i > 0 {
            hist.push(',');
        }
        hist.push_str(&ShardStats::get(b).to_string());
    }
    hist.push(']');
    let body = format!(
        "{{\"players\":{},\"agents\":{},\"spectators\":{},\"max_players\":{},\"proto\":{},\"tick\":{},\
         \"dg_out_bytes\":{},\"dg_out_pkts\":{},\
         \"dg_in_bytes\":{},\"dg_in_pkts\":{},\
         \"stream_out_bytes\":{},\"stream_out_frames\":{},\
         \"stream_in_bytes\":{},\"stream_in_frames\":{},\
         \"aim_stale_samples\":{},\"aim_stale_sum\":{},\"aim_stale_max\":{},\
         \"aim_stale_unacked\":{},\"aim_stale_refused\":{},\
         \"aim_stale_hist\":{},\
         \"favour_granted\":{},\"favour_sum\":{},\
         \"favour_clamped\":{},\"favour_disagree\":{},\"next_wipe\":{},\
         \"wipe\":{},\"act\":{},\"works_lit\":{},\"works_total\":{},\"restart_at\":{}}}",
        ShardStats::get(&stats.players),
        ShardStats::get(&stats.agents),
        ShardStats::get(&stats.spectators),
        sim_core::limits::MAX_PLAYERS,
        protocol::PROTO_VER,
        ShardStats::get(&stats.current_tick),
        ShardStats::get(&stats.net_dg_out_bytes),
        ShardStats::get(&stats.net_dg_out_count),
        ShardStats::get(&stats.net_dg_in_bytes),
        ShardStats::get(&stats.net_dg_in_count),
        ShardStats::get(&stats.net_stream_out_bytes),
        ShardStats::get(&stats.net_stream_out_frames),
        ShardStats::get(&stats.net_stream_in_bytes),
        ShardStats::get(&stats.net_stream_in_frames),
        ShardStats::get(&stats.aim_stale_samples),
        ShardStats::get(&stats.aim_stale_sum),
        ShardStats::get(&stats.aim_stale_max),
        ShardStats::get(&stats.aim_stale_unacked),
        ShardStats::get(&stats.aim_stale_refused),
        hist,
        ShardStats::get(&stats.favour_granted),
        ShardStats::get(&stats.favour_sum),
        ShardStats::get(&stats.favour_clamped),
        ShardStats::get(&stats.favour_disagree),
        ShardStats::get(&stats.next_wipe),
        ShardStats::get(&stats.wipe),
        ShardStats::get(&stats.act),
        ShardStats::get(&stats.works_lit),
        ShardStats::get(&stats.works_total),
        ShardStats::get(&stats.restart_at),
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let client = TcpStream::connect(l.local_addr().unwrap()).expect("connect");
        let (server, _) = l.accept().expect("accept");
        (client, server)
    }

    /// **A spent deadline refuses the next call outright**, however lively
    /// the peer: the time a connection gets is counted once, from accept,
    /// not renewed by every byte that arrives.
    #[test]
    fn a_spent_deadline_ends_the_connection_whatever_the_peer_sends() {
        let (mut client, server) = pair();
        client.write_all(b"G").unwrap();
        let mut live = Deadlined {
            s: server.try_clone().unwrap(),
            until: Instant::now() + Duration::from_secs(10),
        };
        let mut b = [0u8; 4];
        assert_eq!(
            live.read(&mut b).unwrap(),
            1,
            "inside the deadline a byte reads"
        );
        client.write_all(b"E").unwrap();
        let mut spent = Deadlined {
            s: server,
            until: Instant::now(),
        };
        let e = spent
            .read(&mut b)
            .expect_err("a spent deadline reads nothing");
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        let e = spent.write_all(b"HTTP/1.1").expect_err("nor writes");
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
    }
}
