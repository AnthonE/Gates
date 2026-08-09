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
//! {"players":3,"max_players":100,"tick":123456}
//! ```
//!
//! - `players` — bodies with a live connection, off the `players` gauge the
//!   sim loop mirrors from `ShardCore::connected` each tick. A gauge and
//!   not `joins - leaves`, because that pair legitimately drifts
//!   (`stats.rs` says how).
//! - `max_players` — `sim_core::limits::MAX_PLAYERS`, the cap the shard
//!   actually enforces (the same source `ci/shardlist.py` pins for the same
//!   stated reason: a published cap that disagrees with the enforced one is
//!   a lie the player discovers at a refused join).
//! - `tick` — `stats.current_tick`, which doubles as a liveness check: a
//!   number that stops moving is a shard that stopped ticking.
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
//! - reads and writes both time out at [`STATUS_IO_TIMEOUT_MS`];
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
use std::sync::Arc;
use std::time::Duration;

/// Most bytes of a request this will read before closing the connection
/// unanswered. The only line it acts on is the request line, and
/// `GET /status.json HTTP/1.1\r\n` is 26 bytes — the cap is generous
/// headroom for a real client's request line, not a buffer for headers,
/// which are never parsed. DECISIONS.md §open ("shard status endpoint v0").
pub const STATUS_REQUEST_CAP: usize = 512;

/// Read and write timeout per connection, in milliseconds. A client that
/// stalls past it is closed; at two seconds a slow-loris costs the next
/// poller a bounded wait on the serial accept loop, never a thread.
/// DECISIONS.md §open ("shard status endpoint v0").
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
    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    std::thread::Builder::new()
        .name("status".into())
        .spawn(move || serve(listener, stats))?;
    Ok(bound)
}

/// The thread body: accept, answer, close, forever. An accept error backs
/// off briefly rather than spinning — the errors that recur (fd
/// exhaustion) are not fixed by asking again in the same microsecond.
fn serve(listener: TcpListener, stats: Arc<ShardStats>) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Refusals and dead sockets end the connection; neither is
                // worth a counter, let alone a panic. The one observable
                // that matters is that the NEXT request is answered, and
                // `tests/status.rs` asserts exactly that.
                let _ = answer(stream, &stats);
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Read one request, bounded; answer it or close. Every early `Ok(())` is
/// a deliberate refusal — the connection closes with nothing written.
fn answer(mut stream: TcpStream, stats: &ShardStats) -> std::io::Result<()> {
    let timeout = Some(Duration::from_millis(STATUS_IO_TIMEOUT_MS));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;

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
    if path != "/status.json" {
        return stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }

    // The response string is this thread's one allocation per request, and
    // this thread is not the sim thread — correctness and boundedness over
    // cleverness (the fixed-buffer alternative buys nothing measurable
    // against a poller that asks every few seconds).
    let body = format!(
        "{{\"players\":{},\"max_players\":{},\"tick\":{}}}",
        ShardStats::get(&stats.players),
        sim_core::limits::MAX_PLAYERS,
        ShardStats::get(&stats.current_tick),
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())
}
