//! The headless load client (DESIGN.md §11 M0 "bots bin"): a real
//! wtransport connection driving `sim_core::bots` random-walk inputs at
//! 30 Hz and reconstructing snapshots through `ClientView` — the same
//! contract the web client will implement. Used by `bin/bots` and the
//! 50-bot smoke gate.

use crate::net::{client_handshake, read_event_frame};
use crate::view::{Applied, ClientView};
use protocol::{decode_event, encode_input, peek_kind, InputDatagram, Welcome, KIND_SNAPSHOT};
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

    // A bot is a guest and stays one (`net::client_handshake` says why).
    let welcome = client_handshake(
        &mut send,
        &mut recv,
        "bot",
        protocol::Address::GUEST,
        |_| None,
    )
    .await?;

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
    // **Bind IPv4 first, and fall back to the dual-stack default.**
    // `with_bind_default()` is `INADDR_ANY` dual-stack, and on a container
    // with no IPv6 it fails outright — `Address family not supported by
    // protocol (os error 97)`. `CLAUDE.md`'s trap list records that exact
    // failure taking all four `bot_smoke` tests down on a CLEAN tree and
    // names it correctly: a missing capability, not a defect in the diff.
    //
    // What that entry could not say, because no fix was known, is that the
    // capability is not actually needed. Every shard this fleet loads is
    // reachable over v4 — `shard.toml` binds `127.0.0.1:4433` and so does
    // every gate — so asking for v4 makes the wall RUN instead of skipping
    // it, which is the same resolution `CLAUDE.md` prescribes for the
    // `wasm32-unknown-unknown` case. The dual-stack path is kept for a v6
    // shard, and both failures are reported if neither binds.
    //
    // `client::client_endpoint` carries the identical fix for the identical
    // reason; the native client hit it first, because a client that cannot
    // bind cannot draw.
    let build = |ip: wtransport::config::IpBindConfig| {
        Endpoint::client(
            wtransport::ClientConfig::builder()
                .with_bind_config(ip)
                .with_no_cert_validation()
                .build(),
        )
    };
    match build(wtransport::config::IpBindConfig::InAddrAnyV4) {
        Ok(e) => Ok(e),
        Err(v4) => build(wtransport::config::IpBindConfig::InAddrAnyDual)
            .map_err(|dual| format!("client endpoint: v4 {v4}; dual-stack {dual}")),
    }
}
