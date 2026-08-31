//! **How stale is an aim, in ticks** — the gate for lag-compensation slice 1
//! (`findings/lagcomp-design-20260818.md` §7).
//!
//! `combat::strike` resolves a swing against present server positions and
//! nothing measured how far behind the world the swinging client was. This
//! suite holds the measurement that now does: an input datagram carries
//! `snapshot_ack = S` ("the newest world I had applied when I made these
//! frames is server tick S"), the server executes that frame at tick `T`,
//! and **raw `T − S`** is folded into `ShardStats` and published on
//! `/status.json`.
//!
//! Four things are gated here and each has a defect it was proven against:
//!
//! 1. **The arithmetic** — an ack `N` ticks old measures exactly `N`, end
//!    to end through `ShardCore::push_input` → `tick`, not through a stub.
//! 2. **The u16 wrap** — `World::tick` is a `u64` and `snapshot_ack` is its
//!    low 16 bits, so a shard crosses the boundary every 65 536 ticks
//!    (36 min 24 s at 30 Hz). A widening subtraction is green for the first
//!    half hour of every wipe and garbage afterwards, which is exactly the
//!    gate `CLAUDE.md` means by one that has not met its bug.
//! 3. **The exclusions are visible** — a client that has never acked is not
//!    a client with enormous staleness, and it is counted (`aim_stale_unacked`)
//!    rather than silently skipped, because a skip and a zero are the same
//!    thing in a mean.
//! 4. **The stamp is keep-first** — a retransmit tail must not re-stamp a
//!    frame already buffered with a fresher ack, which would understate
//!    every frame that waits a tick in the input buffer.
//!
//! No assertion here is on elapsed time (`CLAUDE.md`: a gate that waits on
//! a clock is not a gate). Every one is on a tick number or a counter.

use protocol::InputDatagram;
use server::client::ClientNetState;
use server::core::ShardCore;
use server::stats::{
    self, ShardStats, AIM_STALE_BUCKETS, AIM_STALE_CEILING_TICKS, FAVOUR_DISAGREE_BAND_TICKS,
};
use sim_core::combat::CombatContent;
use sim_core::gather::ItemStack;
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::{
    INTERP_DELAY_TICKS, REWIND_ACK_BIAS_TICKS, REWIND_MAX_TICKS, SNAPSHOT_INTERVAL_TICKS,
};
use sim_core::movement::POS_XZ_Q;
use sim_core::yaw_dir;

const SEED: u64 = 20_260_731;
/// The canonical dev spawn — the same one every other wire suite stands on.
const SPAWN: (f32, f32) = (1024.0, 1024.0);

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn frame(seq: u16) -> InputFrame {
    InputFrame {
        seq,
        ..InputFrame::default()
    }
}

/// The whole aim-staleness reading, as one tuple, so a test asserts the set
/// rather than one field of it: `(samples, sum, max, unacked, refused)`.
fn reading(s: &ShardStats) -> (u64, u64, u64, u64, u64) {
    (
        ShardStats::get(&s.aim_stale_samples),
        ShardStats::get(&s.aim_stale_sum),
        ShardStats::get(&s.aim_stale_max),
        ShardStats::get(&s.aim_stale_unacked),
        ShardStats::get(&s.aim_stale_refused),
    )
}

fn hist(s: &ShardStats) -> Vec<u64> {
    s.aim_stale_hist.iter().map(ShardStats::get).collect()
}

fn tick(core: &mut ShardCore, stats: &ShardStats) {
    core.tick_bare(stats, |_, _, _| true);
}

/// A shard with one connected client, already in the world and already
/// receiving snapshots.
fn shard() -> (Box<ShardCore>, ShardStats) {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.dev_spawn = Some(SPAWN);
    assert!(core.connect(0, id_of(0)));
    // Enough ticks for the join command to land and for the sent-snapshot
    // ring to hold something to ack.
    for _ in 0..8 {
        tick(&mut core, &stats);
    }
    (core, stats)
}

/// Advance to the next tick at which a snapshot was sent, and return that
/// snapshot's header tick — which is `world.tick` after the call that
/// incremented it (`encode_snapshot` reads `self.world.tick` post-tick and
/// `record_sent` stores exactly that).
fn advance_to_snapshot(core: &mut ShardCore, stats: &ShardStats) -> u64 {
    loop {
        tick(core, stats);
        if core.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
            return core.world.tick;
        }
    }
}

/// Drive one measurement end to end and return the raw staleness the shard
/// recorded for it: ack a snapshot, let `age` ticks pass, then hand the
/// shard one input frame and tick once so it executes.
fn measure_one(core: &mut ShardCore, stats: &ShardStats, seq: u16, age: u64) -> u64 {
    let s = advance_to_snapshot(core, stats);
    for _ in 0..age {
        tick(core, stats);
    }
    assert_eq!(
        core.world.tick,
        s + age,
        "the fixture lost count of its own ticks"
    );
    let mut dg = InputDatagram::new(s as u16, 0, 4);
    dg.push(frame(seq)).expect("one frame fits");
    core.push_input(0, &dg);
    let before = ShardStats::get(&stats.aim_stale_sum);
    let before_n = ShardStats::get(&stats.aim_stale_samples);
    tick(core, stats);
    assert_eq!(
        ShardStats::get(&stats.aim_stale_samples),
        before_n + 1,
        "the frame did not execute, so nothing was measured"
    );
    ShardStats::get(&stats.aim_stale_sum) - before
}

/// **1 · The arithmetic, through the real path.** An ack `N` ticks behind
/// the tick that executes the frame measures exactly `N`, for every `N` the
/// histogram distinguishes and two past its top edge.
#[test]
fn an_ack_n_ticks_old_measures_exactly_n() {
    let (mut core, stats) = shard();
    let mut seq = 1u16;
    for age in [0u64, 1, 2, 3, 5, 7, 9] {
        let got = measure_one(&mut core, &stats, seq, age);
        assert_eq!(
            got, age,
            "an ack {age} ticks old measured {got} ticks of staleness"
        );
        seq = seq.wrapping_add(1);
    }
    let (samples, sum, max, unacked, refused) = reading(&stats);
    assert_eq!(samples, 7, "one sample per frame executed");
    assert_eq!(sum, 27, "0+1+2+3+5+7+9");
    assert_eq!(max, 9, "the max is the tail, not the mean");
    assert_eq!(
        (unacked, refused),
        (0, 0),
        "a client that acked real snapshots was excluded from its own measurement"
    );
    // The distribution: 0,1,2,3,5 land in their own buckets, 7 and 9 both
    // land in the top one ("7 or more" — the proposed REWIND_MAX_TICKS).
    let h = hist(&stats);
    assert_eq!(h.len(), AIM_STALE_BUCKETS);
    assert_eq!(h, vec![1, 1, 1, 1, 0, 1, 0, 2], "histogram {h:?}");
}

/// **2 · The u16 wrap**, which is the one this gate exists for. The
/// subtraction lives in `ShardStats::record_aim_stale` precisely so it can
/// be driven across the boundary without running a shard for 36 minutes.
///
/// Each row is `(T low 16, S, expected raw)`. A widening subtraction — the
/// obvious `world.tick - ack as u64` — returns the right answer for the
/// first two rows and ~65 500 for every other one, so a suite that only
/// ever ran at low tick numbers would be green over it forever.
#[test]
fn the_measurement_wraps_with_the_u16_it_is_read_from() {
    let cases: [(u16, u16, u64); 6] = [
        // No wrap: ordinary mid-wipe traffic.
        (1_000, 995, 5),
        (30, 30, 0),
        // The boundary itself: tick 65 536 is `0` in 16 bits.
        (0, 65_535, 1),
        // Server tick 65 539 against a snapshot from tick 65 534.
        (3, 65_534, 5),
        // The widest sample the ceiling still believes, straddling the wrap.
        (
            AIM_STALE_CEILING_TICKS.wrapping_sub(1),
            u16::MAX,
            AIM_STALE_CEILING_TICKS as u64,
        ),
        // And a snapshot from just before the wrap, executed just after.
        (7, 65_530, 13),
    ];
    for (now, ack, want) in cases {
        let s = ShardStats::default();
        s.record_aim_stale(now, Some(ack));
        assert_eq!(
            reading(&s),
            (1, want, want, 0, 0),
            "T(low16)={now} S={ack} should measure {want} ticks"
        );
    }
}

/// **3a · A client that has never acked is excluded, and says so.**
///
/// `ClientView::ack_fields` returns a flat `(0, 0)` until the first
/// snapshot lands, so a naive `T − S` on a shard that has been up for a
/// while reports the shard's own uptime as one player's lag — a single such
/// sample would own `aim_stale_max` and drag the mean past anything real.
/// The test runs the shard well past tick 0 first, so a zero-staleness
/// reading cannot pass by accident.
#[test]
fn a_client_that_never_acked_is_excluded_and_counted() {
    let (mut core, stats) = shard();
    for _ in 0..40 {
        tick(&mut core, &stats);
    }
    assert!(
        core.world.tick > 40,
        "the fixture must be past tick 0 or this proves nothing"
    );
    assert!(
        core.clients[0].newest_acked.is_none(),
        "this client is supposed to have acked nothing"
    );

    let mut sent = 0u64;
    for seq in 1..=6u16 {
        // ack 0 / bits 0 — exactly what a client sends before its first
        // snapshot, and `on_acks` credits none of it because tick 0 is
        // never a snapshot tick.
        let mut dg = InputDatagram::new(0, 0, 4);
        dg.push(frame(seq)).expect("one frame fits");
        core.push_input(0, &dg);
        tick(&mut core, &stats);
        sent += 1;
    }
    assert!(
        core.clients[0].newest_acked.is_none(),
        "an ack of 0 credited a snapshot the shard never sent"
    );
    let (samples, sum, max, unacked, refused) = reading(&stats);
    assert_eq!(
        (samples, sum, max, refused),
        (0, 0, 0, 0),
        "an unmeasurable frame reached the distribution"
    );
    assert_eq!(
        unacked, sent,
        "the exclusion has to be visible: {sent} frames were executed \
         unmeasured and the counter says {unacked}"
    );
    assert_eq!(hist(&stats), vec![0; AIM_STALE_BUCKETS]);
}

/// **3b · And the exclusion is not permanent.** The same client, once it
/// acks a snapshot the shard actually sent, is measured from the very next
/// datagram — `push_input` credits the ack before it stamps the tail, so
/// there is no dead first frame.
#[test]
fn the_first_real_ack_is_measured_immediately() {
    let (mut core, stats) = shard();
    let mut dg = InputDatagram::new(0, 0, 4);
    dg.push(frame(1)).expect("fits");
    core.push_input(0, &dg);
    tick(&mut core, &stats);
    assert_eq!(ShardStats::get(&stats.aim_stale_unacked), 1);

    let got = measure_one(&mut core, &stats, 2, 2);
    assert_eq!(got, 2, "the first datagram carrying a real ack was skipped");
    assert_eq!(ShardStats::get(&stats.aim_stale_samples), 1);
}

/// **3c · An ack naming a snapshot this shard never sent is not a
/// measurement either.** The stamp is gated on `newest_acked`, which
/// `on_acks` sets only out of the server's own sent ring — so a client
/// claim the server cannot corroborate buys nothing.
#[test]
fn an_ack_of_a_snapshot_never_sent_is_not_measured() {
    let (mut core, stats) = shard();
    let s = advance_to_snapshot(&mut core, &stats);
    // One past the newest tick ever snapshotted: a snapshot from the
    // FUTURE, which cannot have been sent. (This read "snapshots land on
    // even ticks, so an odd tick names one that cannot exist" until
    // netcode v2 S2 put a snapshot on every tick — the future tick is the
    // uncorroborated claim that survives any cadence.)
    let fake = (s + 1) as u16;
    let mut dg = InputDatagram::new(fake, 0, 4);
    dg.push(frame(1)).expect("fits");
    core.push_input(0, &dg);
    tick(&mut core, &stats);
    assert_eq!(
        reading(&stats),
        (0, 0, 0, 1, 0),
        "an uncorroborated ack was folded into the distribution"
    );
}

/// **3d · A wildly old ack is refused and counted, not believed.**
/// `snapshot_ack` is a client claim, and one datagram naming a snapshot
/// from an hour ago would own `aim_stale_max` forever. The ceiling is
/// derived (`stats::AIM_STALE_CEILING_TICKS`); this holds both sides of it.
#[test]
fn a_staleness_past_the_ceiling_is_refused_and_counted() {
    let s = ShardStats::default();
    // Exactly at the ceiling: believed.
    s.record_aim_stale(AIM_STALE_CEILING_TICKS, Some(0));
    assert_eq!(
        reading(&s),
        (
            1,
            AIM_STALE_CEILING_TICKS as u64,
            AIM_STALE_CEILING_TICKS as u64,
            0,
            0
        ),
        "the ceiling itself must be inside the believed range"
    );
    // One past it: refused, and it touches nothing else.
    s.record_aim_stale(AIM_STALE_CEILING_TICKS + 1, Some(0));
    assert_eq!(
        reading(&s),
        (
            1,
            AIM_STALE_CEILING_TICKS as u64,
            AIM_STALE_CEILING_TICKS as u64,
            0,
            1
        ),
        "a sample past the ceiling reached the distribution"
    );
    // And the worst case a forger can reach: the whole u16 space.
    s.record_aim_stale(0, Some(1));
    assert_eq!(ShardStats::get(&s.aim_stale_refused), 2);
    assert_eq!(
        ShardStats::get(&s.aim_stale_max),
        AIM_STALE_CEILING_TICKS as u64,
        "a forged ack moved the high-water mark"
    );
}

/// **4 · The stamp is keep-first.**
///
/// `findings/lagcomp-design-20260818.md` §2.2 claims `push_frame` "drops a
/// frame it has already seen", so the first datagram's ack would be kept
/// for free. **It does not**: the guard there drops a frame already
/// *executed* or ancient, and a frame sitting unexecuted in the buffer is
/// overwritten by the retransmit tail of the next datagram. Without the
/// keep-first rule the stamp would follow the newest datagram to mention
/// the frame — a fresher `S`, so a smaller `T − S` — and every frame that
/// waits a tick in the buffer would understate its own staleness.
#[test]
fn a_retransmitted_frame_keeps_the_ack_it_first_arrived_under() {
    let mut c = ClientNetState::new();
    c.reset(1);
    // seq 7 first appears in a datagram acking snapshot 100…
    c.push_frame(frame(7), Some(100));
    // …and again in the redundancy tail of one acking 140, before it has
    // been executed.
    c.push_frame(frame(7), Some(140));
    let got = c.consume_input().expect("the frame is buffered");
    let (f, view) = (got.frame, got.view);
    assert_eq!(f.seq, 7);
    assert_eq!(
        view,
        Some(100),
        "the retransmit tail re-stamped a frame the client made earlier"
    );

    // The other half: a DIFFERENT seq landing in the same ring slot takes
    // the new stamp, so keep-first is about one frame and not about a slot.
    let mut c = ClientNetState::new();
    c.reset(1);
    c.push_frame(frame(1), Some(10));
    let _ = c.consume_input();
    c.push_frame(frame(2), Some(20));
    let got = c.consume_input().expect("buffered");
    assert_eq!((got.frame.seq, got.view), (2, Some(20)));
}

/// **4b · Two frames consumed in one tick report the newer's stamp.**
/// The consume throttle takes two when the buffer runs deep; both move the
/// body since netcode v2 (`Command::InputPair`), but only the newer's
/// buttons ACT, so measuring the older's aim would price a swing nobody
/// swung — and the older is by definition the staler, so getting this
/// backwards would inflate the number this whole slice exists to produce.
#[test]
fn the_throttle_reports_the_frame_that_executes() {
    let mut c = ClientNetState::new();
    c.reset(1);
    // Deep enough to trip INPUT_THROTTLE_DEPTH: distinct stamps, oldest
    // first, so "the newer" is unambiguous.
    for s in 0..10u16 {
        c.push_frame(frame(s), Some(1_000 + s));
    }
    let got = c.consume_input().expect("buffered");
    assert_eq!(
        (got.frame.seq, got.view),
        (1, Some(1_001)),
        "the throttle reported a frame other than the one whose buttons act"
    );
    // And the older frame rides back beside it, owed its movement step.
    assert_eq!(got.prev.expect("throttle carries the older frame").seq, 0);
}

// ─────────────────────────────────────────────────────────────────────────
// Slice 5 — the mint. `findings/lagcomp-design-20260818.md` §7.
//
// Everything above measures. Everything below spends the measurement, and
// it exists because the thing that shipped broken here was not a wrong
// value anywhere — it was a `favour: 0` literal at the one site that mints
// the number, with the ring, the clamp, the type, `combat::strike`'s
// rewound scan and `ranged::hitscan`'s all landed, all gated, and all
// unreachable. Three judge reports in a row ranked it first.
//
// So the shape of the gate matters more than usual: an assertion that
// `favour_for` returns the right number proves the arithmetic and NOT that
// anything calls it. `the_shard_mints_a_favour_through_the_real_path`
// is the one that would have been red for those three passes, and it works
// by reading a counter written from the same binding the command carries
// (`core.rs` says why it is bound once).
//
// What is deliberately NOT gated here: that a rewound body changes a hit.
// `sim-core/tests/{combat,gun}.rs` own that — a target hit at favour 7 and
// missed at favour 0, both asserted — and duplicating it against a shard
// would be a second, weaker copy of somebody else's gate. The chain is
// mint (here) → clamp (`world.rs`'s `Input` arm) → reader (those suites),
// and each link is held by the crate that owns it.

/// The four favour counters as one tuple, `reading`'s shape and for its
/// reason: `(granted, sum, clamped, disagree)`.
fn favour_reading(s: &ShardStats) -> (u64, u64, u64, u64) {
    (
        ShardStats::get(&s.favour_granted),
        ShardStats::get(&s.favour_sum),
        ShardStats::get(&s.favour_clamped),
        ShardStats::get(&s.favour_disagree),
    )
}

/// Ack a snapshot and nothing else — a datagram with no frame tail, so it
/// moves `newest_acked` without buffering anything to execute.
fn ack_only(core: &mut ShardCore, tick_acked: u64) {
    let dg = InputDatagram::new(tick_acked as u16, 0, 4);
    core.push_input(0, &dg);
}

/// **5 · The formula, exhaustively, at the one function that owns it.**
///
/// `favour = min((T − S) + INTERP_DELAY_TICKS − REWIND_ACK_BIAS_TICKS,
/// REWIND_MAX_TICKS)`, which at the shipped constants is `min(raw + 4, 7)`
/// (the flat add grew by one at netcode v2 S2: the bias floored to 0 with
/// the 30 Hz interval while the interp delay deliberately held at 4).
/// Swept across the clamp rather than sampled either side of it, because
/// the two defects this catches are an off-by-one at the ceiling and a
/// dropped term — and a two-point test is green under both if the points
/// are chosen badly.
#[test]
fn the_favour_is_the_measurement_plus_four_clamped_at_seven() {
    let bias = INTERP_DELAY_TICKS - REWIND_ACK_BIAS_TICKS;
    assert_eq!(
        bias, 4,
        "the shipped constants moved; the sweep below is written against them"
    );
    for raw in 0..=20u16 {
        let want = (raw + bias as u16).min(REWIND_MAX_TICKS as u16) as u8;
        assert_eq!(
            stats::favour_for(raw, Some(0), INTERP_DELAY_TICKS),
            want,
            "a {raw}-tick-old aim minted the wrong favour"
        );
    }
    // The clamp is reached at raw 3 and never exceeded, which is the whole
    // promise `REWIND_MAX_TICKS` makes to the victim.
    assert_eq!(stats::favour_for(2, Some(0), INTERP_DELAY_TICKS), 6);
    assert_eq!(
        stats::favour_for(3, Some(0), INTERP_DELAY_TICKS),
        REWIND_MAX_TICKS
    );
    assert_eq!(
        stats::favour_for(AIM_STALE_CEILING_TICKS, Some(0), INTERP_DELAY_TICKS),
        REWIND_MAX_TICKS
    );
}

/// **5b · The two inputs that mint nothing.** Both are cases where the
/// server does not know, and a favour of 0 is the pre-lag-compensation sim
/// bit for bit — so "no measurement" costs the shooter help rather than
/// handing out an unearned rewind.
///
/// The ceiling half is the one with an attacker behind it: `snapshot_ack`
/// is a client claim, and it is now a claim worth *money* — an hour-old ack
/// asks for the deepest rewind the shard can give. It gets none.
#[test]
fn an_unmeasurable_aim_mints_no_favour() {
    // Never acked a snapshot this shard sent.
    assert_eq!(stats::favour_for(9_000, None, INTERP_DELAY_TICKS), 0);
    // At the ceiling: still believed, so still paid.
    assert_eq!(
        stats::favour_for(AIM_STALE_CEILING_TICKS, Some(0), INTERP_DELAY_TICKS),
        REWIND_MAX_TICKS,
        "the ceiling itself must stay inside the paid range, as it is for the statistic"
    );
    // One past it, and the worst a forger can reach: nothing.
    assert_eq!(
        stats::favour_for(AIM_STALE_CEILING_TICKS + 1, Some(0), INTERP_DELAY_TICKS),
        0
    );
    assert_eq!(
        stats::favour_for(0, Some(1), INTERP_DELAY_TICKS),
        0,
        "a whole u16 of forged staleness paid out"
    );
}

/// **6 · The mint reaches the command — through the shard, not a stub.**
///
/// **This is the gate the slice exists for.** Under the `favour: 0` literal
/// that stood at `core.rs` for three passes, every assertion here is red
/// and every other gate in this repo is green, which is exactly the
/// arrangement that let a finished feature sit switched off.
///
/// It reads `favour_granted`/`favour_sum` rather than the command buffer
/// because `cmd_buf` is private — and the counter is written from the same
/// binding `Command::Input` carries, one line apart, so it cannot report a
/// rewind the sim was not told about.
#[test]
fn the_shard_mints_a_favour_through_the_real_path() {
    let (mut core, stats) = shard();
    // A fresh aim — one tick of staleness — is still worth four ticks of
    // rewind, because the client drew its remotes INTERP_DELAY_TICKS back
    // even on a perfect link. This is the case a "favour == staleness"
    // mistake gets wrong while looking sane.
    let raw = measure_one(&mut core, &stats, 1, 1);
    assert_eq!(raw, 1);
    assert_eq!(
        favour_reading(&stats),
        (1, 5, 0, 0),
        "the shard executed a frame and granted it no rewind — lag compensation is off"
    );

    // A slower link: raw 2 ⇒ 6, still under the clamp, so `favour_clamped`
    // must stay put. A mint that clamped everything would pass the line
    // above and fail here.
    let raw = measure_one(&mut core, &stats, 2, 2);
    assert_eq!(raw, 2);
    assert_eq!(favour_reading(&stats), (2, 11, 0, 0));

    // Past the clamp: granted 7, and counted as clamped.
    let raw = measure_one(&mut core, &stats, 3, 9);
    assert_eq!(raw, 9);
    assert_eq!(
        favour_reading(&stats),
        (3, 18, 1, 0),
        "a 9-tick-old aim either got more than REWIND_MAX_TICKS or was not counted as clamped"
    );
}

/// **6b · A client that has never acked executes its frames and buys
/// nothing.** The frame still runs — this is not a refusal — and the
/// counters must show the granted set is empty rather than showing nothing
/// at all, which is how "the feature is off" and "nobody is playing" get
/// told apart on `/status.json`.
#[test]
fn a_frame_from_an_unacked_client_runs_with_no_favour() {
    let (mut core, stats) = shard();
    for _ in 0..40 {
        tick(&mut core, &stats);
    }
    let mut dg = InputDatagram::new(0, 0, 4);
    dg.push(frame(1)).expect("one frame fits");
    core.push_input(0, &dg);
    tick(&mut core, &stats);
    assert_eq!(
        ShardStats::get(&stats.aim_stale_unacked),
        1,
        "the fixture did not reach the unacked path"
    );
    assert_eq!(
        favour_reading(&stats),
        (0, 0, 0, 0),
        "a client with no acked snapshot was granted a rewind"
    );
}

/// **7 · A client acking backwards is corrected down to the evidence, and
/// counted.**
///
/// The attack the mint creates: staleness now buys rewind depth, so a
/// client on a fast link can ack an *old* snapshot to look slow and collect
/// `REWIND_MAX_TICKS` of peeker's advantage on demand. `newest_acked` is
/// the server's own record of the newest snapshot it watched this client
/// ack, so the two readings are independent and the smaller staleness wins
/// (`findings/lagcomp-design-20260818.md` §6.2's stated rule, built without
/// the wall clock that bullet asks for — `core.rs::push_input` says why).
#[test]
fn an_ack_that_regresses_past_the_band_is_corrected_and_counted() {
    let (mut core, stats) = shard();
    let old = advance_to_snapshot(&mut core, &stats);
    ack_only(&mut core, old);
    // Walk forward well past the band, acking honestly, so the server's
    // evidence is unambiguous.
    let mut newest = old;
    for _ in 0..5 {
        newest = advance_to_snapshot(&mut core, &stats);
        ack_only(&mut core, newest);
    }
    assert!(
        newest - old > FAVOUR_DISAGREE_BAND_TICKS as u64,
        "the fixture did not open a gap wider than the band"
    );

    // Now claim the OLD view, which is worth `newest - old` extra ticks of
    // rewind if believed.
    let at = core.world.tick;
    let mut dg = InputDatagram::new(old as u16, 0, 4);
    dg.push(frame(9)).expect("one frame fits");
    core.push_input(0, &dg);
    let before = ShardStats::get(&stats.aim_stale_sum);
    tick(&mut core, &stats);

    let honest = at - newest;
    assert_eq!(
        ShardStats::get(&stats.aim_stale_sum) - before,
        honest,
        "the shard measured the claim instead of its own evidence"
    );
    let (granted, sum, _, disagree) = favour_reading(&stats);
    assert_eq!(granted, 1);
    assert_eq!(
        sum,
        stats::favour_for(honest as u16, Some(0), INTERP_DELAY_TICKS) as u64,
        "the forged ack bought rewind depth the server had not seen it earn"
    );
    assert_eq!(disagree, 1, "the correction was silent");
}

/// **7b · Reordering is absorbed, and that is what the band is for.**
///
/// QUIC datagrams are unordered, so one acking snapshot `N-1` legitimately
/// lands after one acking `N`; the client has lied about nothing. The claim
/// is still corrected — the fresher reading is still the true one — but no
/// accusation is recorded, because a counter that fires on ordinary jitter
/// is a counter an operator learns to ignore.
#[test]
fn a_reordered_ack_is_corrected_without_an_accusation() {
    let (mut core, stats) = shard();
    let first = advance_to_snapshot(&mut core, &stats);
    ack_only(&mut core, first);
    let second = advance_to_snapshot(&mut core, &stats);
    ack_only(&mut core, second);
    assert!(
        second - first <= FAVOUR_DISAGREE_BAND_TICKS as u64,
        "consecutive snapshots are {} ticks apart, wider than the band — \
         this test's premise is gone",
        second - first
    );

    let at = core.world.tick;
    let mut dg = InputDatagram::new(first as u16, 0, 4);
    dg.push(frame(4)).expect("one frame fits");
    core.push_input(0, &dg);
    let before = ShardStats::get(&stats.aim_stale_sum);
    tick(&mut core, &stats);

    assert_eq!(
        ShardStats::get(&stats.aim_stale_sum) - before,
        at - second,
        "the reordered datagram was measured against its own stale claim"
    );
    assert_eq!(
        ShardStats::get(&stats.favour_disagree),
        0,
        "ordinary datagram reordering was logged as an accusation"
    );
}

/// **7c · The evidence never makes a client look *staler* than it claims.**
/// The rule is "the smaller of the two estimates", and a one-directional
/// implementation is easy to write backwards — `evidence >= claim` keeping
/// the claim is the whole of it. A client honestly reporting a fresh view
/// while the server's newest ack is older (its newer acks are still in
/// flight) must keep its own, fresher number.
#[test]
fn the_correction_only_ever_runs_one_way() {
    let (mut core, stats) = shard();
    let acked = advance_to_snapshot(&mut core, &stats);
    ack_only(&mut core, acked);
    let fresh = advance_to_snapshot(&mut core, &stats);
    assert!(fresh > acked);

    // Claim the newer snapshot in the same datagram that carries the frame:
    // `on_acks` runs first, so this is also the ordinary path.
    let at = core.world.tick;
    let mut dg = InputDatagram::new(fresh as u16, 0, 4);
    dg.push(frame(3)).expect("one frame fits");
    core.push_input(0, &dg);
    let before = ShardStats::get(&stats.aim_stale_sum);
    tick(&mut core, &stats);
    assert_eq!(
        ShardStats::get(&stats.aim_stale_sum) - before,
        at - fresh,
        "a client was charged staleness its own ack disproved"
    );
    assert_eq!(ShardStats::get(&stats.favour_disagree), 0);
}

/// **8 · The disagreement relay has exactly one reader.**
///
/// `ack_regressions` is a destructive hand-over: `take_ack_regressions`
/// zeroes it, so a second caller silently halves the shard's count of the
/// only lag-compensation signal that accuses anybody, and nothing fails.
/// That is `CLAUDE.md`'s clean-merge trap in miniature — two lanes each
/// adding a reader, no conflicting line, a green build and a broken number.
///
/// The gate is a grep for the call site, not a value assertion, because the
/// defect is a call site: a value test passes with one reader and passes
/// again with two, since each drain is individually correct.
#[test]
fn the_disagreement_relay_has_one_reader() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sites = Vec::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).expect("the server crate has a src/") {
            let p = e.expect("readable entry").path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let src = std::fs::read_to_string(&p).expect("readable source");
            for (i, line) in src.lines().enumerate() {
                // A CALL site, which is what the defect is. The
                // declaration is not one, and neither is a doc comment
                // pointing at the rule — this file's own comment about the
                // single-consumer contract would otherwise fail the gate
                // that enforces it. A trailing comment on a real call is
                // still caught: the line does not START with `//`.
                let code = line.trim_start();
                if code.starts_with("//") || code.starts_with("*") {
                    continue;
                }
                if line.contains("take_ack_regressions")
                    && !line.contains("fn take_ack_regressions")
                {
                    sites.push(format!("{}:{}", p.display(), i + 1));
                }
            }
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "the disagreement relay is drained from {} places, and a destructive \
         read with two readers loses half of what it counts: {sites:#?}",
        sites.len()
    );
    assert!(
        sites[0].contains("core.rs"),
        "the drain moved out of the tick loop that mints the favour: {sites:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// 9 — the effect, not the arithmetic.
//
// Everything above this line can be satisfied without the sim ever hearing
// a favour, and **that was proven rather than assumed**: restoring the
// shipped `favour: 0` at `core.rs`'s command construction — the exact
// literal three judge reports ranked first — leaves all sixteen tests above
// GREEN. `record_favour` reads the same binding one line earlier, so it
// keeps reporting a rewind the sim was never told about.
//
// That is `CLAUDE.md`'s naive-rebuild trap arriving in a counter, and the
// only fix is to observe a **consequence**: a swing that lands solely
// because the server rewound the victim. This gate is the one that goes red
// under that mutant, and the counters above become what they should have
// been from the start — a diagnostic, not a proof.

/// The fixture geometry, mirroring `sim-core/tests/combat.rs`'s: one tick
/// inside the 2 m fixture reach, and far enough outside it that no rounding
/// in the quantized body can close the gap.
const NEAR_M: f32 = 1.0;
const FAR_M: f32 = 4.5;

/// A shard with two connected clients, the combat fixture loaded, and a
/// spear each. `remote_hand.rs`'s `pair` for the layout, `combat.rs`'s
/// `duel_world` for the armament.
fn duel_shard() -> (Box<ShardCore>, ShardStats) {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.dev_spawn = Some(SPAWN);
    core.world.combat = CombatContent::probe_fixture();
    for slot in 0..2 {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
    }
    for _ in 0..8 {
        tick(&mut core, &stats);
    }
    for p in core.world.players.iter_mut().take(2) {
        assert!(p.active, "both fixture players must be in the world");
        p.inv[0] = ItemStack {
            item: 0,
            count: 1,
            cond: 0,
        };
    }
    (core, stats)
}

/// Teleport the victim `dist` metres in front of the attacker along yaw 0,
/// at the attacker's own height so the vertical band is never the variable
/// under test. `combat.rs::place_in_front`, written against the shard's
/// quantized body because `ShardCore::world` is what a server test holds.
fn place_victim(core: &mut ShardCore, dist: f32) {
    let (fx, fz) = yaw_dir(0);
    let a = core.world.players[0].body;
    let v = &mut core.world.players[1].body;
    v.qx = a.qx + (fx * dist / POS_XZ_Q) as i32;
    v.qz = a.qz + (fz * dist / POS_XZ_Q) as i32;
    v.qy = a.qy;
}

/// Stand the victim near for four ticks and far for four, so the rewind
/// ring's eight rows split cleanly: `T-1..T-4` is out of reach and
/// `T-5..T-8` is inside it. A favour of 4 or less therefore misses and 5 or
/// more hits, with a whole tick of margin on each side of the boundary —
/// wide enough that the gate is about the favour and not about counting
/// ticks.
fn victim_walked_out_of_reach(core: &mut ShardCore, stats: &ShardStats) {
    place_victim(core, NEAR_M);
    for _ in 0..4 {
        tick(core, stats);
    }
    place_victim(core, FAR_M);
    for _ in 0..4 {
        tick(core, stats);
    }
}

/// One swing, through the wire path, acking `snapshot` — which is what
/// mints the favour. Returns whether the victim lost hp.
fn swing_acking(core: &mut ShardCore, stats: &ShardStats, snapshot: u64) -> bool {
    let before = core.world.players[1].hp;
    let mut dg = InputDatagram::new(snapshot as u16, 0, 4);
    dg.push(InputFrame {
        seq: 21,
        buttons: BTN_PRIMARY,
        yaw: 0,
        // `InputFrame::default()`'s pitch 0 is straight DOWN; level is 128
        // (`snapshot_budget.rs` pays for this one too).
        pitch: 128,
        ..InputFrame::default()
    })
    .expect("one frame fits");
    core.push_input(0, &dg);
    tick(core, stats);
    core.world.players[1].hp < before
}

/// **9 · A stale aim lands a swing the live world would have missed.**
///
/// The whole feature, end to end and through the real path: a datagram
/// arrives acking an old snapshot, the shard mints a favour from it, the
/// sim rewinds the victim, and a spear that would have swung through empty
/// air takes hp off somebody. Nothing here reads a favour counter — the
/// assertion is on `hp`, which is the only thing a player experiences.
///
/// The control is the same fixture and the same swing under a **fresh**
/// ack, which mints `0 + 3 = 3` and reaches only as far back as the victim
/// was already out of reach. Both halves are asserted, because "it hits"
/// alone is satisfied by a fixture whose victim never left.
#[test]
fn a_stale_aim_lands_a_swing_the_live_world_would_have_missed() {
    // ── Stale: ack a snapshot, then let the fixture's eight ticks age it.
    let (mut core, stats) = duel_shard();
    let old = advance_to_snapshot(&mut core, &stats);
    ack_only(&mut core, old);
    victim_walked_out_of_reach(&mut core, &stats);
    let raw = core.world.tick - old;
    assert!(
        raw >= 4,
        "the fixture must age the ack past the clamp's knee; it aged {raw} ticks"
    );
    assert!(
        swing_acking(&mut core, &stats, old),
        "the victim stood {FAR_M} m away and {NEAR_M} m away four ticks earlier, the shard \
         minted a favour of {} for a {raw}-tick-old ack, and the swing still missed — \
         nothing rewound",
        stats::favour_for(raw as u16, Some(0), INTERP_DELAY_TICKS)
    );

    // ── Fresh: identical geometry, identical swing, an ack from this tick.
    let (mut core, stats) = duel_shard();
    let old = advance_to_snapshot(&mut core, &stats);
    ack_only(&mut core, old);
    victim_walked_out_of_reach(&mut core, &stats);
    // Every tick is a snapshot tick since netcode v2 S2 put the interval
    // at 1, so "the most recent snapshot tick" is the current one — the
    // interval-rounding this line used to do died with the cadence (and
    // clippy's modulo_one rightly called the leftover arithmetic dead).
    let recent = core.world.tick;
    ack_only(&mut core, recent);
    let raw = core.world.tick - recent;
    let favour = stats::favour_for(raw as u16, Some(0), INTERP_DELAY_TICKS);
    assert!(
        favour <= 4,
        "a {raw}-tick-old ack minted {favour}, which reaches the near window — \
         this control cannot fail for the reason it exists to test"
    );
    assert!(
        !swing_acking(&mut core, &stats, recent),
        "a swing at a victim {FAR_M} m away landed on a favour of {favour}, which reaches \
         only ticks the victim was already out of reach for — the fixture is not falsifiable"
    );
}
