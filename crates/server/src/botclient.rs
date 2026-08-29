//! The headless load client (DESIGN.md §11 M0 "bots bin"): a real
//! wtransport connection driving `sim_core::bots` random-walk inputs at
//! 30 Hz and reconstructing snapshots through `ClientView` — the same
//! snapshot contract the native client's `ClientCore` implements, out of
//! the same shared crate (`client-core`). Used by `bin/bots` and the
//! 50-bot smoke gate.
//!
//! **And it raids** (`NOW.md` §0rs item 1, off the judge's repeated gap 1
//! *"a player has no opponent"*). `sim_core::bots::raid_step` landed with
//! `test_raid_storm` driving it straight into `World::tick`, which proved
//! the caps and nothing about the wire: this file is where the same profile
//! becomes traffic. The bot derives its plot from **its own body** through
//! `build_cell_of` — the identical function `ui/place.rs:77` uses to turn a
//! look-at point into a cell — feeds `raid_step`, and writes the result over
//! the `SendStream` the handshake already left open. Every frame goes out
//! through the public `encode_action_*` the native client calls, so the
//! server cannot tell a raiding bot from a player, which is the whole point:
//! a refusal counted here was earned on the real path.

use crate::net::{client_handshake, read_event_frame, write_frame, FRAME_PREFIX_BYTES};
use crate::view::{Applied, ClientView};
use protocol::{
    decode_event, encode_action_access, encode_action_demolish, encode_action_deploy,
    encode_action_loot, encode_action_move, encode_action_pickup, encode_action_place,
    encode_action_repair, encode_action_throw, encode_input, peek_kind, EventMsg, InputDatagram,
    Welcome, WireError, KIND_SNAPSHOT, MAX_STREAM_MSG_BYTES,
};
use sim_core::bots::{bot_frame, raid_step, RaidPlan, RaidRows, RAID_CYCLE};
use sim_core::build::build_cell_of;
use sim_core::input::InputFrame;
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, MAX_BUILD_COORD, MAX_INPUT_FRAMES, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::rng::Pcg32;
use sim_core::world::Command;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use wtransport::endpoint::endpoint_side::Client;
use wtransport::error::{ConnectingError, ConnectionError};
use wtransport::{Connection, Endpoint};

#[derive(Debug, Default)]
pub struct BotReport {
    pub player_id: u32,
    pub welcome: Option<Welcome>,
    /// Times the peer's HTTP/3 layer shed this bot's connect before the
    /// shard ever answered, and we dialled again. Reported rather than
    /// swallowed: a retry that nothing counts turns a real capacity
    /// regression into a slower green run (`bot_smoke` asserts a ceiling).
    pub connect_sheds: u32,
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
    /// the lane like any real client must — an unread lane backpressures).
    pub events_received: u64,
    pub event_decode_errors: u64,

    // ---- what this client actually cost the wire (NOW.md §0q item 4) -----
    //
    // The **per-client** half of the byte measurement. The shard counts its
    // lanes in aggregate (`ShardStats`, and see the lane-bytes block in
    // `stats.rs` for why it cannot do better without inventing a per-client
    // table); one bot already is a client, so the same four lanes measured
    // here are a *distribution* rather than a mean — which is the thing a
    // shard total divided by `players` can never be.
    //
    // Only bytes are new: the frame and datagram counts they pair with are
    // already above (`inputs_sent`, `events_received`, `actions_sent`), and
    // a second count of the same events would be a number that can disagree
    // with itself.
    /// Snapshot datagram bytes that arrived, counted before the decode and
    /// regardless of it — the path charged for them either way.
    pub dg_in_bytes: u64,
    /// Snapshot datagrams that arrived. Not the same as
    /// `snapshots_applied`: a stale or baseline-less one still crossed.
    pub dg_in_count: u64,
    /// Input datagram bytes the socket accepted (pairs with `inputs_sent`).
    pub dg_out_bytes: u64,
    /// Event-lane bytes read off the reliable stream, length prefix
    /// included (pairs with `events_received` + `event_decode_errors`).
    pub ev_in_bytes: u64,
    /// Action-lane bytes written, prefix included (pairs with
    /// `actions_sent`); a write that failed is not counted here and is
    /// `action_lane_errors` instead.
    pub act_out_bytes: u64,

    // ---- the raid lane (`None` rows = this bot only walks) ----------------
    /// Action frames written over the reliable stream.
    pub actions_sent: u64,
    /// `raid_step` commands with no wire form, or whose encoder refused the
    /// arguments. Counted rather than sent: a malformed action frame is the
    /// one thing that **drops the session** (`net.rs` `action_reader_task`),
    /// so the bot must never hand the server something it cannot decode.
    pub actions_unencodable: u64,
    /// Writes the action lane refused. Non-zero means the stream died; the
    /// bot keeps walking afterwards rather than ending the run.
    pub action_lane_errors: u64,
    /// Times the plan was re-seated from a live body — so `> 0` is the
    /// proof the plot came from the snapshot stream and not from a literal.
    pub raid_cycles: u64,
    /// The last plot cell this bot raided. Reported because a fleet whose
    /// plots are all *identical* would still satisfy every count above while
    /// the body-to-cell derivation was a constant; the smoke asserts the
    /// spread, which is the only thing that can tell those two apart.
    pub last_plot: Option<(u16, u16)>,
    /// The sim's verdicts on this bot's claims, off the event lane. Roughly
    /// half of `raid_step` is *meant* to be refused (a stranger's code, a
    /// stranger's box, a foundation on somebody else's claim), so these are
    /// the measurement, not a fault: they are the refusal paths wall 4 had
    /// never seen driven over a socket at population.
    pub build_refused: u64,
    pub deploy_refused: u64,
    pub move_refused: u64,
    /// A charge that actually landed damage on a structure, and a code that
    /// was actually accepted — the two events that say the raid connected.
    pub struct_hits: u64,
    pub auths: u64,
    /// The **success** half of the same lane: claims the sim granted rather
    /// than refused. Counted because the refusal counters above cannot tell
    /// "the rule ran and said no" apart from "the bot could not afford to
    /// ask" — a fleet that never places anything scores identically to one
    /// whose every claim was rejected on the interesting rule. These are
    /// what a raid *cost*, and they were 0 for all 8 raiders until the
    /// fleet was given something to spend.
    pub pieces_placed: u64,
    pub deploys_placed: u64,
    /// Charges that actually armed. Separate from `struct_hits` because a
    /// charge is **planted and fused**, not thrown: those are two different
    /// failures ten seconds apart, and one counter cannot tell "the throw
    /// was refused" from "the run ended before the fuse did".
    pub charges_planted: u64,
}

/// Event-lane counters, shared with the drain task. One allocation, so a
/// bot fleet does not pay an `Arc` per counter.
#[derive(Default)]
struct EventTally {
    received: AtomicU64,
    decode_errors: AtomicU64,
    /// Bytes off the event stream, prefix included. Lives here rather than
    /// on `BotReport` because the lane is drained on its own task — the
    /// same reason `received` does.
    bytes: AtomicU64,
    build_refused: AtomicU64,
    deploy_refused: AtomicU64,
    move_refused: AtomicU64,
    struct_hits: AtomicU64,
    auths: AtomicU64,
    pieces_placed: AtomicU64,
    deploys_placed: AtomicU64,
    charges_planted: AtomicU64,
}

impl EventTally {
    fn note(&self, ev: &EventMsg) {
        let c = match ev {
            EventMsg::BuildRefused { .. } => &self.build_refused,
            EventMsg::DeployRefused { .. } => &self.deploy_refused,
            EventMsg::MoveRefused { .. } => &self.move_refused,
            EventMsg::StructHit { .. } => &self.struct_hits,
            EventMsg::Auth { .. } => &self.auths,
            EventMsg::PiecePlaced { .. } => &self.pieces_placed,
            EventMsg::DeployPlaced { .. } => &self.deploys_placed,
            EventMsg::ChargePlaced { .. } => &self.charges_planted,
            _ => return,
        };
        c.fetch_add(1, Ordering::Relaxed);
    }
}

/// One `raid_step` command as the frame a real client would write.
///
/// `Command::Input` is deliberately absent: a hotbar selection has no action
/// encoder because it rides the **input datagram**, so the caller folds its
/// `sel` into the next `bot_frame` instead — which is exactly how the native
/// client changes slots. Everything else maps one-to-one onto the encoder
/// `crates/client/src/ui/` already calls for that verb.
///
/// `None` = no wire form for this variant. `Some(Err(..))` = the encoder
/// refused the arguments; both are counted and neither is written, because
/// `action_reader_task` drops the session on a frame it cannot decode.
fn encode_raid(cmd: &Command, buf: &mut [u8]) -> Option<Result<usize, WireError>> {
    Some(match *cmd {
        Command::Place {
            row,
            cx,
            cz,
            level,
            loc,
            freehand,
            ..
        } => encode_action_place(row, cx, cz, level, loc, freehand, buf),
        Command::PlaceDeploy {
            row,
            cx,
            cz,
            level,
            loc,
            ..
        } => encode_action_deploy(row, cx, cz, level, loc, buf),
        Command::Throw {
            deploy,
            cx,
            cz,
            level,
            loc,
            ..
        } => encode_action_throw(deploy, cx, cz, level, loc, buf),
        Command::Demolish {
            deploy,
            cx,
            cz,
            level,
            loc,
            ..
        } => encode_action_demolish(deploy, cx, cz, level, loc, buf),
        Command::Repair {
            deploy,
            cx,
            cz,
            level,
            loc,
            ..
        } => encode_action_repair(deploy, cx, cz, level, loc, buf),
        Command::Access {
            cx,
            cz,
            level,
            loc,
            op,
            code,
            ..
        } => encode_action_access(cx, cz, level, loc, op, code, buf),
        Command::Move {
            cont,
            from_kind,
            from_slot,
            to_kind,
            to_slot,
            count,
            ..
        } => encode_action_move(cont, from_kind, from_slot, to_kind, to_slot, count, buf),
        Command::Loot { .. } => encode_action_loot(buf),
        Command::Pickup { .. } => encode_action_pickup(buf),
        _ => return None,
    })
}

/// The build cell a quantized body coordinate stands in.
///
/// `build_cell_of` is the sim's own function and the clamp is the one
/// `ui/place.rs:77` applies to a look-at point, so the bot addresses a cell
/// on the same grid the server validates against — the quantize-both-sides
/// law from the trap list, applied to a cell index. The island is
/// all-positive (`limits::MAX_BUILD_COORD`: a 2,048 m world over ~683 3 m
/// cells), so the clamp only ever bites on a body outside the playfield.
fn body_cell(q: i32) -> u16 {
    build_cell_of(q as f32 * POS_XZ_Q).clamp(0, MAX_BUILD_COORD as i32 - 1) as u16
}

/// `H3_EXCESSIVE_LOAD` — the HTTP/3 code a peer sends when it is shedding
/// load rather than answering (RFC 9114 §8.1, `wtransport_proto`'s
/// `H3_EXCESSIVE_LOAD`).
const H3_EXCESSIVE_LOAD: u64 = 0x0107;

/// How many times a dial may be shed before the bot gives up.
const CONNECT_TRIES: u32 = 4;

/// Is this failure the box saying "not now", rather than the shard saying no?
///
/// The distinction is the whole point and it is exact, not a heuristic.
/// **Our** refusals are `REFUSE_VERSION..=REFUSE_ADMIN`, 0..=5, closed with
/// the refusal text beside them (`net.rs`) — those are ANSWERS and are
/// returned on the first try, because retrying one would let a suite sleep
/// through a version gate that had begun rejecting everybody. `0x107` is
/// emitted by the transport beneath both ends' application code and means
/// only that the peer was too busy to start a session.
fn is_load_shed(e: &ConnectingError) -> bool {
    match e {
        ConnectingError::ConnectionError(ConnectionError::ApplicationClosed(close)) => {
            code_is_load_shed(close.code().into_inner())
        }
        _ => false,
    }
}

/// The decidable half, split out so it can be gated.
///
/// `wtransport`'s `ApplicationClose::new` is `pub(crate)`, so a test cannot
/// build the error value — but the DISCRIMINATION is what can go wrong, and
/// it is a predicate over a `u64`. Widening this by one digit is how a
/// refusal starts being retried, so it is checked directly.
fn code_is_load_shed(code: u64) -> bool {
    code == H3_EXCESSIVE_LOAD
}

/// Dial, retrying only a transport-level shed, with a widening gap.
///
/// Why this exists: `test_bot_smoke_50` opens fifty QUIC connections in a
/// burst, and on 2026-08-28 bot 24 of 50 was shed with `0x107` on a box that
/// was simultaneously running a release build. The suite went red, then green
/// on a re-run with no change, and the loop runner stopped itself — correctly
/// — because an oracle that flips is not an oracle. The burst is a timing
/// assertion made by the transport rather than by us, which is the same class
/// as the clock rule in `CLAUDE.md`: assert on observable state, never on the
/// box being fast enough.
async fn connect_retrying_a_shed(
    endpoint: &Endpoint<Client>,
    url: &str,
    sheds: &mut u32,
) -> Result<Connection, String> {
    let mut gap = Duration::from_millis(40);
    for attempt in 1..=CONNECT_TRIES {
        match endpoint.connect(url).await {
            Ok(c) => return Ok(c),
            Err(e) if attempt < CONNECT_TRIES && is_load_shed(&e) => {
                *sheds += 1;
                tokio::time::sleep(gap).await;
                gap *= 2;
            }
            Err(e) => return Err(format!("connect: {e}")),
        }
    }
    unreachable!("the loop returns on the last attempt")
}

/// Connect, handshake, then walk for `duration`. Any transport failure is
/// an `Err` with a short reason.
///
/// `raid` supplies the content rows the raid profile addresses; `None` walks
/// and sends no action, which is what every gate written before the raid
/// lane existed still expects. The caller owns the rows because they are
/// **content**, resolved by id through `Content::piece_index` and friends —
/// a row number compiled in here would be wall 7 broken by a load tool.
pub async fn run_bot(
    endpoint: &Endpoint<Client>,
    server: SocketAddr,
    seed_stream: u64,
    duration: Duration,
    raid: Option<RaidRows>,
) -> Result<BotReport, String> {
    let url = format!("https://{server}");
    let mut connect_sheds = 0;
    let connection = connect_retrying_a_shed(endpoint, &url, &mut connect_sheds).await?;

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
        connect_sheds,
        ..BotReport::default()
    };

    // Drain the event lane on its own task (a `select!` read would drop a
    // half-read frame on cancellation and desync the stream). The native
    // client does the same (`Session::connect` spawns its lane reader):
    // the pump is independent of the frame loop.
    let tally = Arc::new(EventTally::default());
    {
        let tally = tally.clone();
        tokio::spawn(async move {
            let mut recv = recv;
            while let Some((buf, len)) = read_event_frame(&mut recv).await {
                tally
                    .bytes
                    .fetch_add((FRAME_PREFIX_BYTES + len) as u64, Ordering::Relaxed);
                match decode_event(&buf[..len]) {
                    Ok(ev) => {
                        tally.received.fetch_add(1, Ordering::Relaxed);
                        tally.note(&ev);
                    }
                    Err(_) => {
                        tally.decode_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
    }

    let mut view = ClientView::new();
    let mut rng = Pcg32::new(welcome.seed ^ 0xB07B_07B0, seed_stream);
    let mut seq: u16 = 1;
    let mut yaw: u16 = (seed_stream as u16).wrapping_mul(2557);
    let mut tail: Vec<InputFrame> = Vec::with_capacity(MAX_INPUT_FRAMES);

    // The raid rides its **own** rng stream, seeded the way `raid_storm.rs`
    // seeds the storm's. `bot_frame` therefore draws exactly what it drew
    // before this lane existed, so arming the raid moves no walk and no
    // digest that depended on one.
    let mut raid = raid;
    let mut raid_rng = Pcg32::new(welcome.seed ^ 0x5A1D_C0DE, seed_stream);
    // Odd streams raid, even streams own — the storm's `i % 2 == 1`, so a
    // fleet is half attackers and half owners however many bots you start.
    let attacker = seed_stream % 2 == 1;
    let mut plan: Option<RaidPlan> = None;
    let mut steps_in_cycle: u16 = 0;
    let mut sel_override: Option<u8> = None;
    let mut act_buf = [0u8; MAX_STREAM_MSG_BYTES];

    let mut cadence = tokio::time::interval(Duration::from_nanos(1_000_000_000 / TICK_HZ as u64));
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::Instant::now() + duration;
    let mut dg_buf = [0u8; DATAGRAM_BUDGET_BYTES];

    loop {
        tokio::select! {
            _ = cadence.tick() => {
                let mut f = bot_frame(&mut rng, yaw, seq);
                // A raid's selection step rides the input lane, because that
                // is the only place a hotbar slot exists on the wire.
                if let Some(sel) = sel_override.take() {
                    f.sel = sel;
                }
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
                        // On `Ok` only: what the socket took, not what we
                        // handed it (`net.rs` counts its side the same way).
                        report.dg_out_bytes += len as u64;
                    }
                }
                report.last_executed_seq = view.last_executed_seq;

                // ---- the raid lane -----------------------------------
                // One action per cadence tick, which is not a chosen
                // number: `core::wants_action` hands the sim at most one
                // action per client per tick and `push_action` *silently
                // drops* the rest, so the ceiling the server already
                // enforces is the rate. A real client cannot do better,
                // and a load tool that pretended to would be measuring a
                // pressure no player can apply.
                if let Some(rows) = raid {
                    // Re-seat the plot from the live body every cycle. The
                    // bot walks, so a plan pinned at spawn would spend the
                    // whole run out of reach of its own foundation and
                    // measure nothing but `REFUSE_B_REACH`.
                    if steps_in_cycle >= RAID_CYCLE {
                        plan = None;
                    }
                    if plan.is_none() {
                        if let Some(body) = view.get(report.player_id) {
                            let (cx, cz) = (body_cell(body.qx), body_cell(body.qz));
                            plan = Some(RaidPlan::new(report.player_id, cx, cz, attacker));
                            steps_in_cycle = 0;
                            report.raid_cycles += 1;
                            report.last_plot = Some((cx, cz));
                        }
                    }
                    if let Some(p) = plan.as_mut() {
                        let cmd = raid_step(p, &mut raid_rng, rows);
                        steps_in_cycle += 1;
                        match cmd {
                            Command::Input { frame, .. } => sel_override = Some(frame.sel),
                            other => match encode_raid(&other, &mut act_buf) {
                                Some(Ok(len)) => {
                                    if write_frame(&mut send, &act_buf[..len]).await.is_ok() {
                                        report.actions_sent += 1;
                                        report.act_out_bytes +=
                                            (FRAME_PREFIX_BYTES + len) as u64;
                                    } else {
                                        // The stream is gone. Keep walking
                                        // rather than ending the run: a
                                        // dead action lane is a finding,
                                        // not a reason to stop measuring
                                        // the snapshot one.
                                        report.action_lane_errors += 1;
                                        raid = None;
                                    }
                                }
                                Some(Err(_)) | None => report.actions_unencodable += 1,
                            },
                        }
                    }
                }
            }
            dg = connection.receive_datagram() => {
                let dg = dg.map_err(|e| format!("receive: {e}"))?;
                report.dg_in_count += 1;
                report.dg_in_bytes += dg.len() as u64;
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
                report.events_received = tally.received.load(Ordering::Relaxed);
                report.event_decode_errors = tally.decode_errors.load(Ordering::Relaxed);
                report.ev_in_bytes = tally.bytes.load(Ordering::Relaxed);
                report.build_refused = tally.build_refused.load(Ordering::Relaxed);
                report.deploy_refused = tally.deploy_refused.load(Ordering::Relaxed);
                report.move_refused = tally.move_refused.load(Ordering::Relaxed);
                report.struct_hits = tally.struct_hits.load(Ordering::Relaxed);
                report.auths = tally.auths.load(Ordering::Relaxed);
                report.pieces_placed = tally.pieces_placed.load(Ordering::Relaxed);
                report.deploys_placed = tally.deploys_placed.load(Ordering::Relaxed);
                report.charges_planted = tally.charges_planted.load(Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property that must not drift: a shed is retried, an ANSWER is
    /// not. Proven over both sets rather than the happy case, because the
    /// failure mode is a predicate widened until it swallows a refusal — and
    /// a suite that retried `REFUSE_VERSION` would sleep through a version
    /// gate that had begun rejecting everybody, then pass.
    #[test]
    fn a_refusal_is_an_answer_and_only_a_shed_is_retried() {
        for code in protocol::REFUSE_VERSION..=protocol::REFUSE_ADMIN {
            assert!(
                !code_is_load_shed(u64::from(code)),
                "REFUSE code {code} is the shard answering — it must never be retried"
            );
        }
        assert!(code_is_load_shed(H3_EXCESSIVE_LOAD), "0x107 is the shed");
        // Its neighbours are other HTTP/3 errors and are not about load:
        // 0x106 is FRAME_ERROR, 0x108 is ID_ERROR. A predicate widened to a
        // range would take these too.
        assert!(!code_is_load_shed(0x0106));
        assert!(!code_is_load_shed(0x0108));
        assert!(!code_is_load_shed(0));
    }
}
