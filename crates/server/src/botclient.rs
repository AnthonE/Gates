//! The headless load client (DESIGN.md §11 M0 "bots bin"): a real
//! wtransport connection driving `sim_core::bots` random-walk inputs at
//! 30 Hz and reconstructing snapshots through `ClientView` — the same
//! contract the web client will implement. Used by `bin/bots` and the
//! 50-bot smoke gate.

use crate::net::{read_event_frame, read_frame, write_frame};
use crate::view::{Applied, ClientView};
use protocol::{
    decode_event, decode_refuse, decode_welcome, encode_hello, encode_input, peek_kind, Hello,
    InputDatagram, Welcome, KIND_REFUSE, KIND_SNAPSHOT, KIND_WELCOME, MAX_STREAM_MSG_BYTES,
    PROTO_VER,
};
use sim_core::bots::bot_frame;
use sim_core::input::InputFrame;
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, MAX_INPUT_FRAMES, TICK_HZ};
use sim_core::rng::Pcg32;
use std::net::SocketAddr;
use std::time::Duration;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::Endpoint;

#[derive(Debug, Default)]
pub struct BotReport {
    pub player_id: u32,
    pub welcome: Option<Welcome>,
    pub snapshots_applied: u64,
    pub delta_snapshots: u64,
    pub stale_snapshots: u64,
    pub decode_errors: u64,
    pub no_baseline: u64,
    pub inputs_sent: u64,
    /// Newest input seq the server confirmed executing.
    pub last_executed_seq: u16,
    pub own_updates: u64,
    pub max_entities_seen: usize,
    /// Event-lane messages decoded off the reliable stream (a bot drains
    /// the lane like the browser must — an unread lane backpressures).
    pub events_received: u64,
    pub event_decode_errors: u64,
}

/// Connect, handshake, then walk for `duration`. Any transport failure is
/// an `Err` with a short reason.
pub async fn run_bot(
    endpoint: &Endpoint<Client>,
    server: SocketAddr,
    seed_stream: u64,
    duration: Duration,
) -> Result<BotReport, String> {
    let url = format!("https://{server}");
    let connection = endpoint
        .connect(&url)
        .await
        .map_err(|e| format!("connect: {e}"))?;

    let opening = connection
        .open_bi()
        .await
        .map_err(|e| format!("open_bi: {e}"))?;
    let (mut send, mut recv) = opening.await.map_err(|e| format!("open_bi await: {e}"))?;

    let mut msg_buf = [0u8; MAX_STREAM_MSG_BYTES];
    let len = encode_hello(
        &Hello {
            proto_ver: PROTO_VER,
        },
        &mut msg_buf,
    )
    .map_err(|e| format!("encode hello: {e:?}"))?;
    write_frame(&mut send, &msg_buf[..len])
        .await
        .map_err(|_| "write hello".to_string())?;

    let (reply, reply_len) = read_frame(&mut recv)
        .await
        .ok_or("no handshake reply".to_string())?;
    let reply = &reply[..reply_len];
    let welcome = match peek_kind(reply) {
        Ok(KIND_WELCOME) => decode_welcome(reply).map_err(|e| format!("welcome: {e:?}"))?,
        Ok(KIND_REFUSE) => {
            let r = decode_refuse(reply).map_err(|e| format!("refuse: {e:?}"))?;
            return Err(format!("refused: code {}", r.code));
        }
        other => return Err(format!("unexpected handshake reply: {other:?}")),
    };

    let mut report = BotReport {
        player_id: welcome.player_id,
        welcome: Some(welcome),
        ..BotReport::default()
    };

    // Drain the event lane on its own task (a `select!` read would drop a
    // half-read frame on cancellation and desync the stream). The browser
    // client does the same: the lane pump is independent of the RAF loop.
    let events_received = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let event_decode_errors = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    {
        let (got, bad) = (events_received.clone(), event_decode_errors.clone());
        tokio::spawn(async move {
            let mut recv = recv;
            while let Some((buf, len)) = read_event_frame(&mut recv).await {
                match decode_event(&buf[..len]) {
                    Ok(_) => got.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    Err(_) => bad.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                };
            }
        });
    }

    let mut view = ClientView::new();
    let mut rng = Pcg32::new(welcome.seed ^ 0xB07B_07B0, seed_stream);
    let mut seq: u16 = 1;
    let mut yaw: u16 = (seed_stream as u16).wrapping_mul(2557);
    let mut tail: Vec<InputFrame> = Vec::with_capacity(MAX_INPUT_FRAMES);

    let mut cadence = tokio::time::interval(Duration::from_nanos(1_000_000_000 / TICK_HZ as u64));
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::Instant::now() + duration;
    let mut dg_buf = [0u8; DATAGRAM_BUDGET_BYTES];

    loop {
        tokio::select! {
            _ = cadence.tick() => {
                let f = bot_frame(&mut rng, yaw, seq);
                yaw = f.yaw;
                seq = seq.wrapping_add(1);
                tail.push(f);
                // Drop what the server confirmed, then cap to the wire's
                // redundancy window (drop-oldest, limits.rs).
                let confirmed = view.last_executed_seq;
                tail.retain(|t| {
                    let ahead = t.seq.wrapping_sub(confirmed);
                    (1..0x8000).contains(&ahead)
                });
                while tail.len() > MAX_INPUT_FRAMES {
                    tail.remove(0);
                }
                let (ack, ack_bits) = view.ack_fields();
                let mut dg = InputDatagram::new(ack, ack_bits, tail[0].seq as u32);
                for t in &tail {
                    if dg.push(*t).is_err() {
                        break;
                    }
                }
                if let Ok(len) = encode_input(&dg, &mut dg_buf) {
                    // send_datagram, never _wait (the trap list).
                    if connection.send_datagram(&dg_buf[..len]).is_ok() {
                        report.inputs_sent += 1;
                    }
                }
                report.last_executed_seq = view.last_executed_seq;
            }
            dg = connection.receive_datagram() => {
                let dg = dg.map_err(|e| format!("receive: {e}"))?;
                if peek_kind(&dg) != Ok(KIND_SNAPSHOT) {
                    report.decode_errors += 1;
                    continue;
                }
                match view.apply(&dg) {
                    Ok(Applied::Ok { delta }) => {
                        report.snapshots_applied += 1;
                        if delta {
                            report.delta_snapshots += 1;
                        }
                        if view.get(report.player_id).is_some() {
                            report.own_updates += 1;
                        }
                        report.max_entities_seen =
                            report.max_entities_seen.max(view.entities.len());
                    }
                    Ok(Applied::Stale) => report.stale_snapshots += 1,
                    Ok(Applied::NoBaseline) => report.no_baseline += 1,
                    Err(_) => report.decode_errors += 1,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                report.last_executed_seq = view.last_executed_seq;
                report.events_received =
                    events_received.load(std::sync::atomic::Ordering::Relaxed);
                report.event_decode_errors =
                    event_decode_errors.load(std::sync::atomic::Ordering::Relaxed);
                return Ok(report);
            }
        }
    }
}

/// The shared client endpoint for a fleet of bots: one UDP socket, many
/// QUIC connections. Dev-only certificate trust (`with_no_cert_validation`)
/// — bots are a load tool for shards we run, never a browser substitute.
pub fn bot_endpoint() -> Result<Endpoint<Client>, String> {
    let config = wtransport::ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();
    match Endpoint::client(config) {
        Ok(e) => Ok(e),
        Err(dual_stack) => {
            // `with_bind_default` binds `[::]:0` — a dual-stack v6 socket —
            // which a machine with IPv6 disabled cannot create at all. This is
            // the failure `CLAUDE.md` documents for `bot_smoke`: all four of
            // its tests die here with `Address family not supported by
            // protocol (os error 97)`, on a clean tree, and the entry says to
            // believe the box rather than the diff.
            //
            // That reading was right and the conclusion was one step short. A
            // wall that cannot run is not a wall — and unlike a missing wasm
            // target, this one is OUR code binding a socket we did not need.
            // The shard under test is IPv4, so falling back costs nothing and
            // turns a permanently-red gate into one that runs. Same fix, same
            // reason, as `client::client_endpoint`.
            //
            // Dual-stack stays FIRST: it is the better socket where it exists,
            // and it is the one that reaches a v6-only shard.
            let v4 = wtransport::ClientConfig::builder()
                .with_bind_address(std::net::SocketAddr::from(([0, 0, 0, 0], 0)))
                .with_no_cert_validation()
                .build();
            Endpoint::client(v4).map_err(|v4_err| {
                format!("client endpoint: {dual_stack} (and IPv4-only: {v4_err})")
            })
        }
    }
}
