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
use server::stats::{ShardStats, AIM_STALE_BUCKETS, AIM_STALE_CEILING_TICKS};
use sim_core::input::InputFrame;
use sim_core::limits::SNAPSHOT_INTERVAL_TICKS;

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
    let mut dg = InputDatagram::new(s as u16, 0, seq as u32);
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
        let mut dg = InputDatagram::new(0, 0, seq as u32);
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
    let mut dg = InputDatagram::new(0, 0, 1);
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
    // Snapshots land on even ticks (`SNAPSHOT_INTERVAL_TICKS`), so an odd
    // tick names one that cannot exist.
    let fake = (s + 1) as u16;
    let mut dg = InputDatagram::new(fake, 0, 1);
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
    let (f, view) = c.consume_input().expect("the frame is buffered");
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
    let (f, view) = c.consume_input().expect("buffered");
    assert_eq!((f.seq, view), (2, Some(20)));
}

/// **4b · Two frames consumed in one tick report the one that executes.**
/// The consume throttle takes two when the buffer runs deep; only the newer
/// reaches the world, so measuring the older would price an aim nobody
/// acted on — and the older is by definition the staler, so getting this
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
    let (f, view) = c.consume_input().expect("buffered");
    assert_eq!(
        (f.seq, view),
        (1, Some(1_001)),
        "the throttle reported a frame other than the one it executed"
    );
}
