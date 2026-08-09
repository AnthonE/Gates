//! Gates for the status endpoint (`status.rs`): the document's three fields
//! exist and parse as integers, refusals close instead of panicking, and the
//! `players` gauge counts occupancy rather than `joins - leaves`.
//!
//! Every assertion is on observable state — a byte the responder wrote or a
//! connection it closed — never on elapsed time (CLAUDE.md: a gate that
//! waits on a clock is not a gate). The refusal cases are all shaped so the
//! responder decides immediately (a complete bad line, or more bytes than
//! the cap) rather than by waiting out its read timeout.

use server::core::ShardCore;
use server::stats::ShardStats;
use server::status::{spawn_status, STATUS_REQUEST_CAP};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;

const SEED: u64 = 0x6A7E5;

/// Send `req`, read to close, return what came back. Write and read errors
/// are swallowed deliberately: a refusal may close the socket while the
/// request is still in flight, and the assertion is on the bytes returned
/// (none, for a refusal), not on the client's own syscalls.
fn exchange(addr: SocketAddr, req: &[u8]) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).expect("connect to the responder");
    let _ = s.write_all(req);
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    out
}

/// Pull `"name":<digits>` out of a JSON body, asserting the field exists
/// and is an integer — by hand, because the shape is three fixed fields and
/// a JSON dependency would outweigh the module it tests.
fn field(body: &str, name: &str) -> u64 {
    let key = format!("\"{name}\":");
    let at = body
        .find(&key)
        .unwrap_or_else(|| panic!("no `{name}` field in body `{body}`"));
    let digits: String = body[at + key.len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    assert!(
        !digits.is_empty(),
        "`{name}` is not a bare integer in `{body}` (stats.rs L5: diagnostics are numbers)"
    );
    digits.parse().expect("checked digits")
}

fn start(stats: Arc<ShardStats>) -> SocketAddr {
    // Port 0: the OS assigns, spawn_status reports back — no clock, no race.
    spawn_status("127.0.0.1:0".parse().expect("static addr"), stats).expect("bind loopback")
}

#[test]
fn serves_the_three_fields_as_integers() {
    let stats = Arc::new(ShardStats::default());
    ShardStats::set(&stats.players, 3);
    ShardStats::set(&stats.current_tick, 123_456);
    let addr = start(stats);

    let resp = exchange(addr, b"GET /status.json HTTP/1.1\r\nHost: t\r\n\r\n");
    let text = String::from_utf8(resp).expect("the response is text");
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "got: {text}");
    assert!(
        text.contains("Content-Type: application/json\r\n"),
        "got: {text}"
    );
    let body = text.split("\r\n\r\n").nth(1).expect("a body after headers");
    assert_eq!(field(body, "players"), 3);
    assert_eq!(
        field(body, "max_players"),
        server::limits::MAX_PLAYERS as u64,
        "max_players must be the cap the shard enforces, from the same constant"
    );
    assert_eq!(field(body, "tick"), 123_456);
}

#[test]
fn refusals_close_and_the_responder_survives_them() {
    let stats = Arc::new(ShardStats::default());
    ShardStats::set(&stats.players, 7);
    let addr = start(stats);

    // Garbage with a complete line: closed with nothing written.
    let resp = exchange(addr, b"\x00\xff\xfe not http at all\r\n\r\n");
    assert!(resp.is_empty(), "garbage got an answer: {resp:?}");

    // Oversized: past the cap with no request line — refused at the cap,
    // not read further and not waited out.
    let big = vec![b'A'; STATUS_REQUEST_CAP * 4];
    let resp = exchange(addr, &big);
    assert!(
        resp.is_empty(),
        "an over-cap request got an answer: {resp:?}"
    );

    // A method this endpoint does not speak: closed.
    let resp = exchange(addr, b"POST /status.json HTTP/1.1\r\n\r\n");
    assert!(resp.is_empty(), "POST got an answer: {resp:?}");

    // A well-formed GET of the wrong path: 404, not a hangup — an operator
    // probing with curl deserves an answer that names the mistake.
    let resp = exchange(addr, b"GET /wat HTTP/1.1\r\n\r\n");
    assert!(
        String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 404 "),
        "got: {resp:?}"
    );

    // The observable state that proves none of the above panicked the
    // thread: the NEXT request is answered, correctly.
    let resp = exchange(addr, b"GET /status.json HTTP/1.1\r\n\r\n");
    let text = String::from_utf8(resp).expect("text");
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "got: {text}");
    let body = text.split("\r\n\r\n").nth(1).expect("body");
    assert_eq!(field(body, "players"), 7);
}

/// The knob: absent serves nothing (the honest default — no endpoint until
/// the operator says where), set it parses as a socket address, and a value
/// that does not parse refuses the boot rather than coercing.
#[test]
fn status_addr_is_absent_by_default_and_refuses_junk() {
    use server::config::parse_shard_toml;
    let base = "bind = \"127.0.0.1:1\"\nseed = 1\n";
    let off = parse_shard_toml(base).expect("parses");
    assert_eq!(off.status_addr, None);
    let on =
        parse_shard_toml(&format!("{base}status_addr = \"127.0.0.1:8080\"\n")).expect("parses");
    assert_eq!(
        on.status_addr,
        Some("127.0.0.1:8080".parse().expect("static addr"))
    );
    for bad in ["\"\"", "\"nope\"", "\"127.0.0.1\"", "\"1.2.3.4:notaport\""] {
        assert!(
            parse_shard_toml(&format!("{base}status_addr = {bad}\n")).is_err(),
            "accepted status_addr = {bad}"
        );
    }
}

/// The number behind `players` is occupancy, not `joins - leaves`: a
/// disconnect of an already-empty slot (the refused-install sweep bumps
/// `leaves` with no matching `join` in `net.rs`) moves the counter pair and
/// must not move the gauge's source.
#[test]
fn connected_counts_occupancy() {
    let mut core = ShardCore::new(SEED);
    assert_eq!(core.connected(), 0);
    assert!(core.connect(0, 1));
    assert!(core.connect(1, 2));
    assert_eq!(core.connected(), 2);
    let _ = core.disconnect(0);
    assert_eq!(core.connected(), 1);
    // A second disconnect of the same slot is the sweep's no-join case.
    let _ = core.disconnect(0);
    assert_eq!(core.connected(), 1);
}
