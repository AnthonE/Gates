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
    encode_action_reload, encode_action_repair, encode_action_throw, encode_input, peek_kind,
    EventMsg, InputDatagram, Welcome, WireError, KIND_SNAPSHOT, MAX_STREAM_MSG_BYTES,
};
use sim_core::bots::{bot_frame, raid_step, RaidPlan, RaidRows, RAID_CYCLE};
use sim_core::build::build_cell_of;
use sim_core::input::InputFrame;
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, MAX_BUILD_COORD, MAX_INPUT_FRAMES, TICK_HZ};
use sim_core::movement::POS_XZ_Q;
use sim_core::ranged::{REFUSE_RL_BUSY, REFUSE_RL_EMPTY};
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

    // ---- the reload lane (NOW.md §0mag item 6) ---------------------------
    /// Dry clicks heard — a trigger pulled on an empty magazine.
    pub dry_clicks: u64,
    /// `Command::Reload` frames this bot wrote.
    pub reloads_sent: u64,
    /// Magazines the sim actually filled in answer.
    pub reloads_confirmed: u64,
    /// Rounds that left the pack across every confirmed fill.
    pub rounds_loaded: u64,
    /// Asks that arrived while the arm was busy — the retries. Expected to
    /// be nonzero, and that is the mechanic rather than a defect: see the
    /// reload lane in `run_bot` for why the first ask after a dry click
    /// cannot succeed.
    pub reloads_busy: u64,
    /// Asks refused for a reason that is neither dry nor busy.
    pub reloads_refused: u64,

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

    // ---- how much walking actually happened -------------------------------
    //
    // **The three counters that make a red diagnosable instead of a mystery.**
    // Every assertion in `bot_smoke` is about how deep into the raid profile
    // the fleet got, and until these existed a failure could not say whether
    // the bot never took the step or took it and never heard the answer —
    // which are opposite bugs with one symptom (`charges_planted == 0`).
    /// Cadence ticks this bot actually took. The walk ends on this reaching
    /// [`walk_ticks`], never on a clock, so it is the same on a loaded box
    /// as on an idle one and a suite that reads it is reading WORK.
    pub ticks_walked: u64,
    /// `raid_step` calls issued (one per cadence tick once a plot is
    /// seated). `ticks_walked - raid_steps` is time spent waiting for a body
    /// off the snapshot stream, which is the other way a profile starves.
    pub raid_steps: u64,
    /// Polls the settle spent waiting for the event lane to go quiet after
    /// the walk. Non-zero always; at the ceiling means the lane never
    /// quiesced, so a counter read here may still be short.
    pub settle_polls: u32,
    /// The wall-clock backstop fired before the tick budget was spent — the
    /// box stalled this bot for longer than [`WALK_CEILING`] times its
    /// nominal walk. **Reported rather than silent**, because a truncated
    /// walk is the one condition under which a low count is the box's fault
    /// and not the shard's, and a gate that cannot tell those apart is the
    /// flaky gate this field was added to retire.
    pub walk_truncated: bool,
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
    /// Trigger pulls on an empty magazine — `REFUSE_RL_EMPTY`, the dry
    /// click. **This one is not a statistic: it is the bot's only sense
    /// organ for its own ammunition**, and the frame loop watches it grow.
    ///
    /// A snapshot carries no round count (`EntityState` has `held` and no
    /// count, deliberately — a broadcast magazine is a wallhack), so the
    /// event lane is the only place a client of any kind learns it is
    /// empty. The native client keeps a running count off `EV_SHOT`; a bot
    /// with no HUD has exactly this, which is also what a *person* has when
    /// they are not watching the number. You hear the click.
    dry_clicks: AtomicU64,
    /// `REFUSE_RL_BUSY` — the arm is mid-cadence, mid-swing or mid-reload.
    /// **Its own counter because it is the only refusal that means *ask
    /// again*.** Full, wrong-hand and no-rounds all mean stop, and a lane
    /// that could not tell them apart would either give up on a full
    /// magazine or spin forever on an empty pack.
    reloads_busy: AtomicU64,
    /// Magazines actually filled — `EventMsg::Reload`, the sim's answer.
    reloads: AtomicU64,
    /// Rounds that left the pack, summed. `took` and not a difference this
    /// side computed: a partial fill off a nearly empty pack is the case
    /// that makes them differ, and it is the one worth counting.
    rounds_loaded: AtomicU64,
    /// Reloads refused for a reason that is neither dry nor busy — full,
    /// wrong hand, no rounds. The lane stops asking on these.
    reloads_refused: AtomicU64,
}

impl EventTally {
    fn note(&self, ev: &EventMsg) {
        // The reload arms read a *field*, so they cannot ride the table
        // below: a refusal's meaning is its `reason`, and one counter for
        // all five would leave the bot unable to tell "you are empty" from
        // "you are already full" — opposite instructions.
        if let EventMsg::ReloadRefused { reason, .. } = ev {
            let c = match *reason as u32 {
                REFUSE_RL_EMPTY => &self.dry_clicks,
                REFUSE_RL_BUSY => &self.reloads_busy,
                _ => &self.reloads_refused,
            };
            c.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if let EventMsg::Reload { took, .. } = ev {
            self.reloads.fetch_add(1, Ordering::Relaxed);
            self.rounds_loaded
                .fetch_add(*took as u64, Ordering::Relaxed);
            return;
        }
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
        // Not a raid step — the reload lane's, and here because this is the
        // one table in this file that turns a `Command` into the bytes a
        // real client writes. Payloadless on the wire on purpose: the sim
        // picks the weapon and the amount, so there is nothing in the frame
        // to forge (`Command::Reload`'s own doc says why).
        Command::Reload { .. } => encode_action_reload(buf),
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

/// Wall-clock headroom the walk gets over its nominal length before the
/// backstop fires, as a multiple. Plumbing bound, `DECISIONS.md` §open row.
///
/// **This is a backstop, not the schedule.** The walk ends on a COUNT of
/// cadence ticks ([`walk_ticks`]); this exists only because "no bound is
/// wait" — a shard that stops ticking would otherwise hang the suite
/// forever rather than failing it. When it fires the report says so
/// ([`BotReport::walk_truncated`]) instead of returning a short count that
/// reads exactly like a shard which refused everything.
///
/// 2 rather than something generous on purpose: `bot_smoke`'s raid gate
/// asserts that the shard completes fewer ticks than the satchel's fuse
/// (`content/weapons.toml` `fuse_s = 10`, so 300 ticks at `TICK_HZ` 30). A
/// 4 s walk stretched to its 8 s ceiling plus the settle's 500 ms worst case
/// is ~255 shard ticks — inside the fuse with room, where a 3x ceiling's
/// 12.5 s would be ~375 and would redden that assertion instead.
const WALK_CEILING: u32 = 2;

/// Gap between settle polls, and how many quiet ones end it / cap it.
/// Plumbing bounds, `DECISIONS.md` §open row.
///
/// `SETTLE_QUIET_POLLS` consecutive polls with no new event ends the settle;
/// `SETTLE_MAX_POLLS` caps it at 500 ms for a lane that never quiesces
/// because the rest of the fleet is still walking. Same shape as
/// `net.rs`'s `SHUTDOWN_DRAIN_TRIES` / `SHUTDOWN_DRAIN_POLL`: the exit is
/// observable state and the count is only the bound on waiting for it.
const SETTLE_POLL_MS: u64 = 20;
const SETTLE_POLL: Duration = Duration::from_millis(SETTLE_POLL_MS);
const SETTLE_QUIET_POLLS: u32 = 5;
const SETTLE_MAX_POLLS: u32 = 25;

// **Compile-time rather than a test**, which is the stronger form and is
// available because every term is a constant — `net.rs`'s admission gate
// makes the same three checks the same way and says why: get one wrong and
// the shard does not build, so there is no version of the tree where the
// settle is a fixed sleep and a suite is merely red.
//
// 1. The quiet exit must be reachable before the cap, or the settle is a
//    fixed sleep wearing a predicate's clothes and the walk has simply been
//    widened by half a second.
// 2. Its worst case is wall-clock time every bot pays, so it is bounded at
//    500 ms — and that bound is load-bearing arithmetic, not taste: it is
//    inside `WALK_CEILING`'s headroom against the satchel fuse.
// 3. A backstop under 2x is a schedule, and a schedule is the clock this
//    module was just taken off.
const _: () = assert!(SETTLE_QUIET_POLLS < SETTLE_MAX_POLLS);
const _: () = assert!(SETTLE_POLL_MS * SETTLE_MAX_POLLS as u64 <= 500);
const _: () = assert!(WALK_CEILING >= 2);

/// The hotbar slot a raid step selected, **held across frames** until the
/// cycle re-seats it.
///
/// A type rather than an `Option<u8>` local because the defect it exists to
/// prevent is a *call site* and not a value: `Option::take()` reads
/// identically to this at a glance, compiles, and silently reduces the
/// selection to a single frame — which is the whole of the 2026-08-30 flaky
/// gate (see `sel_held` in `run_bot` for the mechanism). There is no `take`
/// on this type, so the one-shot form cannot be written by accident.
#[derive(Default, Clone, Copy)]
struct HeldSel(Option<u8>);

impl HeldSel {
    /// A raid step chose a slot. Every frame from here carries it.
    fn set(&mut self, sel: u8) {
        self.0 = Some(sel);
    }

    /// The cycle re-seated; the next step re-selects.
    fn clear(&mut self) {
        self.0 = None;
    }

    /// Stamp the held slot onto an outgoing frame, leaving `bot_frame`'s own
    /// wandering selection alone when nothing is held.
    fn apply(&self, f: &mut InputFrame) {
        if let Some(sel) = self.0 {
            f.sel = sel;
        }
    }
}

/// Cadence ticks a walk of `duration` is worth.
///
/// **The caller's `Duration` is read as WORK, not as wall-clock time**, and
/// that re-reading is the whole of the flaky-gate fix from 2026-08-30.
/// `run_bot` walks at `TICK_HZ`, one raid step per tick, and every assertion
/// in `bot_smoke` is about how far down the profile that got — so on a box
/// that stalls the bot, a wall-clock window silently bought fewer steps and
/// the gate read "the sim refused everything". `CLAUDE.md`: *assert on
/// observable state, never on elapsed milliseconds.* A count of ticks is
/// observable state; four seconds is not.
///
/// Saturating, and never zero: a caller asking for a walk gets at least one
/// tick of it, so no suite can be handed a bot that returns before sending
/// anything at all.
pub fn walk_ticks(duration: Duration) -> u64 {
    let nanos = duration.as_nanos();
    let per_tick = 1_000_000_000u128 / TICK_HZ as u128;
    ((nanos / per_tick) as u64).max(1)
}

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
    // **Held, not one-shot, and that is the second half of the 2026-08-30
    // flaky-gate fix.** `raid_step` selects the satchel on one step and
    // throws it on the next, but `bot_frame` re-rolls `sel` at random every
    // single frame on purpose ("wander the hotbar too, so held-item
    // selection is inside the alloc/replay/parity surface"). Those two
    // intents collide: a selection applied to exactly one frame is
    // overwritten by the next frame's random slot, and the server applies
    // every input frame it has before the tick's action (`core.rs`: "pending
    // actions ride after inputs"), so a tick that receives two frames runs
    // the throw with a random slot in hand and refuses it as `REFUSE_B_COST`.
    //
    // On an idle box one frame arrives per tick and roughly half the throws
    // land. Under load the redundancy tail delivers several at once and
    // **none** do — 8 raiders spending all 960 of their ticks and arming
    // zero charges, which is how this read as a starved walk when it was a
    // clobbered selection.
    //
    // Holding it is also what a player does: pressing 3 stays on slot 3.
    // Cleared when the cycle re-seats, so each cycle re-selects, and never
    // set at all for a `raid: None` bot — the 50-bot smoke still wanders the
    // hotbar, because that surface is worth covering and is not this one.
    let mut sel_held = HeldSel::default();
    let mut act_buf = [0u8; MAX_STREAM_MSG_BYTES];
    // Reload-lane events this loop has already answered with an `R`. The
    // tally is written by the drain task and read here, so the pair is a
    // monotonic counter and a high-water mark rather than a flag — a flag
    // would need clearing from two tasks, which is the one shape that can
    // lose an edge.
    let mut asks_answered: u64 = 0;
    // Fills this loop has already seen answer an ask. Same shape and same
    // reason as `asks_answered` — a high-water mark over a counter the drain
    // task writes, never a flag.
    let mut fills_seen: u64 = 0;
    // The action stream is gone. `raid` served this for the raid lane and
    // cannot serve it here, because a bot with no raid rows still shoots.
    let mut reload_lane = true;

    let mut cadence = tokio::time::interval(Duration::from_nanos(1_000_000_000 / TICK_HZ as u64));
    // **`Delay`, not `Skip`, and the difference is the bug.** `Skip` throws
    // away a tick the box was too busy to deliver, so a stalled bot walks
    // permanently less far — the amount of work a run does became a function
    // of the load on the machine, which is precisely what `CLAUDE.md`'s clock
    // rule forbids a gate to depend on. `Delay` owes every tick and pays it
    // late instead, so the walk below is bounded by a COUNT and a slow box
    // costs wall-clock time rather than coverage.
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let budget = walk_ticks(duration);
    // The backstop, never the schedule — see `WALK_CEILING`.
    let ceiling = tokio::time::Instant::now() + duration * WALK_CEILING;
    let mut dg_buf = [0u8; DATAGRAM_BUDGET_BYTES];

    loop {
        tokio::select! {
            _ = cadence.tick() => {
                report.ticks_walked += 1;
                let mut f = bot_frame(&mut rng, yaw, seq);
                // A raid's selection step rides the input lane, because that
                // is the only place a hotbar slot exists on the wire.
                sel_held.apply(&mut f);
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
                // A bot renders nothing, so its playout report is a
                // DECISION, not a measurement: the shipped default
                // (`INTERP_DELAY_TICKS`), the same value the server
                // assumed for every client before wire v61 carried one.
                let mut dg = InputDatagram::new(
                    ack,
                    ack_bits,
                    sim_core::limits::INTERP_DELAY_TICKS,
                );
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

                // ---- the reload lane (NOW.md §0mag item 6) ------------
                // **The bot hears the click and presses R.** That is the
                // whole policy, and it has to be a policy rather than a
                // cadence for a reason `EntityState` makes unavoidable: a
                // snapshot carries no round count, so a bot cannot know it
                // is empty until the sim tells it. The one message that
                // does is `REFUSE_RL_EMPTY` — the dry click, which
                // `hitscan` raises when `bot_frame`'s `BTN_PRIMARY` finds
                // an empty magazine. So the bot uses the sense a player
                // uses with their eyes off the HUD, and nothing here
                // invents a fact the wire did not carry.
                //
                // **It has to keep asking, and that is arithmetic rather
                // than a taste for realism.** `hitscan` pays `next_swing =
                // tick + rate_ticks` *before* it refuses — the same line
                // that bounds the dry click to one per cadence — and
                // `reload` answers `REFUSE_RL_BUSY` while `tick <
                // next_swing`. So the reply to a dry click is late by
                // construction: one round trip is ~1 tick and the
                // revolver's cadence is 12, so a bot that asked once per
                // click would be refused **every time, forever**, with a
                // green gate and a gun that never reloads. Measured: with
                // the retry removed, four shooters over 150 ticks confirm
                // zero fills.
                //
                // Self-clocking, so it needs no timer and no knob: one ask
                // per reload-lane event heard. A dry click asks, a BUSY
                // asks again, and the retry rate is therefore the
                // round-trip rate rather than a number somebody chose. It
                // converges because the sim eventually crosses `next_swing`
                // on a tick whose input frame did not pull the trigger —
                // `bot_frame` presses `BTN_PRIMARY` a third of the time and
                // inputs are applied before actions, so two boundary ticks
                // in three take the fill. Measured: 8–11 asks per click,
                // one confirmed fill of exactly a cylinder.
                //
                // One action per tick is the server's ceiling, not a
                // choice: `core::wants_action` takes one per client and
                // `push_action` drops the rest in silence. So a reload
                // *takes* the tick and the raid step is deferred rather
                // than sent beside it — sending both would leave which one
                // survives up to the server, and a raid step lost that way
                // is invisible to every counter in this file.
                //
                // **A confirmed fill retires every ask outstanding when it
                // landed, and without this the lane asks for a magazine it
                // has already been given.** The ordering three paragraphs up
                // is the cause: inputs are applied before actions, so on the
                // tick the fill happens `BTN_PRIMARY` is spent against the
                // magazine while it is still empty. That raises a dry click
                // and the confirm follows it in the same batch — one ask
                // whose reason was gone before the loop could read it, and
                // `reload` answers the retry `REFUSE_RL_FULL`. It needs both
                // events in one batch and a trigger pull on that exact tick,
                // which `bot_frame` presses a third of the time, so it fired
                // about once in twelve runs and the suite read it as
                // `reloads_refused` — the counter whose whole meaning is
                // *the lane asked for something impossible*. Syncing the
                // mark against the fill is the fix rather than tolerating a
                // refusal, because a lane that can ask for a full cylinder
                // cannot be told apart from a sim that fills the wrong one.
                let mut took_the_tick = false;
                let asks = tally.dry_clicks.load(Ordering::Relaxed)
                    + tally.reloads_busy.load(Ordering::Relaxed);
                let fills = tally.reloads.load(Ordering::Relaxed);
                if fills > fills_seen {
                    fills_seen = fills;
                    asks_answered = asks;
                }
                if reload_lane && asks > asks_answered {
                    asks_answered = asks;
                    let cmd = Command::Reload { id: report.player_id };
                    match encode_raid(&cmd, &mut act_buf) {
                        Some(Ok(len)) => {
                            if write_frame(&mut send, &act_buf[..len]).await.is_ok() {
                                report.reloads_sent += 1;
                                report.actions_sent += 1;
                                report.act_out_bytes += (FRAME_PREFIX_BYTES + len) as u64;
                                took_the_tick = true;
                            } else {
                                // Keep walking: a dead action lane is a
                                // finding, not a reason to stop measuring
                                // the snapshot one (the raid lane's rule).
                                report.action_lane_errors += 1;
                                reload_lane = false;
                                raid = None;
                            }
                        }
                        // Counted, never dropped — the raid lane's arm, for
                        // the raid lane's reason. A `Reload` with no wire
                        // form would otherwise be a lane that asks for
                        // nothing in perfect silence, which is this item's
                        // own failure mode one level down.
                        Some(Err(_)) | None => report.actions_unencodable += 1,
                    }
                }

                // ---- the raid lane -----------------------------------
                // One action per cadence tick, which is not a chosen
                // number: `core::wants_action` hands the sim at most one
                // action per client per tick and `push_action` *silently
                // drops* the rest, so the ceiling the server already
                // enforces is the rate. A real client cannot do better,
                // and a load tool that pretended to would be measuring a
                // pressure no player can apply.
                if let Some(rows) = raid.filter(|_| !took_the_tick) {
                    // Re-seat the plot from the live body every cycle. The
                    // bot walks, so a plan pinned at spawn would spend the
                    // whole run out of reach of its own foundation and
                    // measure nothing but `REFUSE_B_REACH`.
                    if steps_in_cycle >= RAID_CYCLE {
                        plan = None;
                        sel_held.clear();
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
                        report.raid_steps += 1;
                        match cmd {
                            Command::Input { frame, .. } => sel_held.set(frame.sel),
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
                // The walk ends on the budget of ticks being spent, which is
                // the same amount of walking on every box.
                if report.ticks_walked >= budget {
                    break;
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
            _ = tokio::time::sleep_until(ceiling) => {
                // Not a schedule and not a widened window: the box stalled
                // this bot past `WALK_CEILING` times its nominal walk, which
                // is a finding about the box. Recorded so the suite's red
                // says that instead of "the sim refused everything".
                report.walk_truncated = true;
                break;
            }
        }
    }

    // ---- settle: the answers to the last actions are still in flight -----
    //
    // **The counters below are fed by a task this one does not join**, and
    // the walk's last raid step is answered a tick later by the sim and a
    // round trip after that by the event lane. Reading the tally at the
    // instant the walk stops therefore reports a state the bot had not yet
    // been told about — a `charges_planted` of 0 for a charge that armed —
    // and that is how `test_bots_raid_over_the_wire` went red on a loaded box
    // and green on an idle one with nothing changed (2026-08-30).
    //
    // The exit is observable state: the event lane stopped advancing. The
    // poll count is only the bound on waiting for it, because the rest of the
    // fleet is still walking and a shared broadcast lane may never fall
    // silent at all.
    let mut quiet = 0u32;
    let mut last_seen = tally.received.load(Ordering::Relaxed);
    while quiet < SETTLE_QUIET_POLLS && report.settle_polls < SETTLE_MAX_POLLS {
        tokio::time::sleep(SETTLE_POLL).await;
        report.settle_polls += 1;
        let seen = tally.received.load(Ordering::Relaxed);
        if seen == last_seen {
            quiet += 1;
        } else {
            quiet = 0;
            last_seen = seen;
        }
    }

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
    report.dry_clicks = tally.dry_clicks.load(Ordering::Relaxed);
    report.reloads_confirmed = tally.reloads.load(Ordering::Relaxed);
    report.rounds_loaded = tally.rounds_loaded.load(Ordering::Relaxed);
    report.reloads_busy = tally.reloads_busy.load(Ordering::Relaxed);
    report.reloads_refused = tally.reloads_refused.load(Ordering::Relaxed);
    Ok(report)
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

    /// A walk is a count of ticks, and the count is what a suite asserts on.
    ///
    /// The property that matters is the one the flaky gate broke: the same
    /// `Duration` always buys the same amount of walking. A box cannot change
    /// this function's answer, which is the entire point of it existing —
    /// `TICK_HZ` and the caller's request are the only inputs.
    #[test]
    fn a_walk_is_measured_in_ticks_and_never_in_milliseconds() {
        // The two windows `bot_smoke` uses, at `TICK_HZ` 30.
        assert_eq!(walk_ticks(Duration::from_secs(4)), 120);
        assert_eq!(walk_ticks(Duration::from_secs(3)), 90);
        // Exactly `TICK_HZ` ticks in a second, whatever `TICK_HZ` is: read
        // off the constant rather than typed, so a rate change moves this
        // with it instead of leaving a literal behind.
        assert_eq!(walk_ticks(Duration::from_secs(1)), TICK_HZ as u64);
        // Never zero: a caller asking for a walk gets one, so no suite can
        // be handed a bot that returns before it has sent anything.
        assert_eq!(walk_ticks(Duration::ZERO), 1);
        assert_eq!(walk_ticks(Duration::from_nanos(1)), 1);
        // Monotone in the request — a longer window is never less walking.
        let mut prev = 0;
        for ms in [10u64, 100, 500, 1_000, 4_000, 60_000] {
            let t = walk_ticks(Duration::from_millis(ms));
            assert!(t >= prev, "{ms} ms bought {t} ticks, less than {prev}");
            prev = t;
        }
    }

    /// A selected slot survives the frames after the one it was chosen on.
    ///
    /// **Proven red under the old body**: with `Option::take()` in place of
    /// this type, frame 2 carries `bot_frame`'s random slot again and the
    /// second assertion fails. That is exactly the production failure — the
    /// server applies every input frame it holds before the tick's action
    /// (`core.rs`: "pending actions ride after inputs"), so a throw issued
    /// one tick after the selection ran with a random slot in hand and was
    /// refused, and eight raiders spending all 960 of their ticks armed zero
    /// charges on a loaded box.
    #[test]
    fn a_selected_slot_is_held_until_the_cycle_re_seats_it() {
        let mut held = HeldSel::default();
        // Nothing held: `bot_frame`'s own wandering selection is untouched,
        // which is what a `raid: None` bot walks with.
        let mut f = InputFrame {
            sel: 3,
            ..InputFrame::default()
        };
        held.apply(&mut f);
        assert_eq!(f.sel, 3, "an unheld frame must keep its own slot");

        // A raid step selects the charge slot.
        held.set(5);
        // Every frame from here, not just the next one. Four is past the
        // one-tick gap between `raid_step`'s select and its throw, which is
        // the gap the one-shot form covered and nothing else.
        for tick in 0..4 {
            let mut f = InputFrame {
                sel: tick as u8,
                ..InputFrame::default()
            };
            held.apply(&mut f);
            assert_eq!(f.sel, 5, "frame {tick} dropped the held slot");
        }

        // The cycle re-seats and the hotbar wanders again until the next
        // step selects — a stale hold would raid the wrong slot forever.
        held.clear();
        let mut f = InputFrame {
            sel: 2,
            ..InputFrame::default()
        };
        held.apply(&mut f);
        assert_eq!(f.sel, 2, "a cleared hold must stop stamping");
    }
}
