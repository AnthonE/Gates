//! Lag compensation end to end: the body a shooter **drew** against the body
//! the shard **rewound to** when that shooter's frame ran.
//!
//! `ClientCore` (the struct the game drives) against `ShardCore` through
//! real encoded datagrams, with a fixed one-way delay each way and the
//! client rendering at 60 fps against the shard's 30 Hz. The victim sprints
//! in a straight line; for every frame the shooter sends, the test reads
//! where the shooter's interpolator drew the victim, then where the shard's
//! rewind put the victim for that frame (`T − favour`), and reports the gap
//! in ticks. Zero is a perfect rewind; positive means the shard judged the
//! swing against a victim further along than the one on screen.

use std::collections::VecDeque;

use client_core::core::ClientCore;
use client_core::interp::RemoteState;
use protocol::{decode_input, InputDatagram};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::input::{InputFrame, BTN_SPRINT};
use sim_core::limits::{DATAGRAM_BUDGET_BYTES, REWIND_MAX_TICKS};
use sim_core::movement::POS_XZ_Q;

const SEED: u64 = 0x6A7E5;
const TICK_MS: f64 = 1000.0 / 30.0;
/// The shooter's render rate: two frames per sim tick.
const FRAME_MS: f64 = TICK_MS / 2.0;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// A one-way pipe: a fixed delay, plus up to `jitter_ms` more per
/// datagram. In order by default (one due before the datagram queued ahead
/// of it waits for that one, as a queue does); `unordered`, each datagram
/// lands at its own time, as QUIC datagrams on a path with jitter do.
#[derive(Default)]
struct Pipe {
    q: VecDeque<(f64, Lane, Vec<u8>)>,
    jitter_ms: f64,
    unordered: bool,
    rng: u64,
}

impl Pipe {
    fn jittery(jitter_ms: f64, seed: u64) -> Self {
        Self {
            jitter_ms,
            rng: seed,
            ..Self::default()
        }
    }
    fn send(&mut self, at: f64, lane: Lane, bytes: &[u8]) {
        // xorshift64: the same jitter every run.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let u = (self.rng >> 11) as f64 / (1u64 << 53) as f64;
        self.q
            .push_back((at + u * self.jitter_ms, lane, bytes.to_vec()));
    }
    fn due(&mut self, now: f64) -> Vec<(Lane, Vec<u8>)> {
        let mut out = Vec::new();
        if self.unordered {
            // The event stream is reliable and stays in order: one not yet
            // due holds every event behind it.
            let (mut i, mut held) = (0, false);
            while i < self.q.len() {
                let (at, lane) = (self.q[i].0, self.q[i].1);
                let event = matches!(lane, Lane::Event);
                if at <= now && !(event && held) {
                    let (_, lane, b) = self.q.remove(i).expect("index checked");
                    out.push((lane, b));
                } else {
                    held |= event;
                    i += 1;
                }
            }
            return out;
        }
        while self.q.front().is_some_and(|(at, _, _)| *at <= now) {
            let (_, lane, b) = self.q.pop_front().expect("front checked");
            out.push((lane, b));
        }
        out
    }
}

/// What one run measured.
struct Run {
    /// `(T − favour) − drawn tick` for every measured frame, in ticks.
    errors: Vec<f64>,
    /// Favours minted for the shooter's measured frames.
    favours: Vec<u8>,
    /// The shooter's highest playout delay over the measured window.
    playout_peak: f64,
    /// Frames the shard counted as cut (`favour_playout_high`).
    cut_counted: u64,
}

/// The shooter's link: `one_way_ms` each way, plus up to `jitter_ms` on
/// each datagram, with the snapshots coming down `unordered` or queued.
#[derive(Clone, Copy)]
struct Link {
    one_way_ms: f64,
    jitter_ms: f64,
    unordered: bool,
}

/// What the shooter's client does to its acks on the way out.
#[derive(Clone, Copy)]
enum Lie {
    Honest,
    /// Every datagram acks this many ticks further back than it applied.
    Late(u16),
    /// For the first second of every three, the ack held `lag` ticks and
    /// then jumped to the newest: ages that saw-tooth under the round-trip
    /// bound and read as jitter. For the other two, acking `lag` late, the
    /// age held at the bound while the playout bound that jitter earned
    /// decays. Every datagram reports `playout` ticks of playout.
    FakeJitter {
        lag: u16,
        playout: u8,
    },
}

/// Run the shooter (slot 0, a real `ClientCore`) and the runner (slot 1,
/// raw frames with no acks, so it mints no favour of its own) for `secs`
/// with `one_way_ms` of delay on both of the shooter's directions, the
/// shard holding the shooter's acks to the round trip quinn would read for
/// that link (`ShardStats::slot_rtt_us`).
fn run(one_way_ms: f64, secs: f64) -> Run {
    run_with(one_way_ms, secs, 0, true)
}

/// [`run`], with every datagram the shooter sends acking `lie` ticks
/// further back than it had applied, and the round trip `measured` or not.
fn run_with(one_way_ms: f64, secs: f64, lie: u16, measured: bool) -> Run {
    let lie = if lie == 0 {
        Lie::Honest
    } else {
        Lie::Late(lie)
    };
    run_link(one_way_ms, 0.0, secs, lie, measured)
}

/// The whole harness: `jitter_ms` more delay, at most, on each datagram
/// each way, and the client's acks told `lie`.
fn run_link(one_way_ms: f64, jitter_ms: f64, secs: f64, lie: Lie, measured: bool) -> Run {
    let link = Link {
        one_way_ms,
        jitter_ms,
        unordered: false,
    };
    run_on(link, secs, lie, measured)
}

/// [`run_link`] on any [`Link`].
fn run_on(link: Link, secs: f64, lie: Lie, measured: bool) -> Run {
    let Link {
        one_way_ms,
        jitter_ms,
        unordered,
    } = link;
    let stats = ShardStats::default();
    if measured {
        // The pipe's round trip, its mean jitter both ways (quinn's
        // smoothed reading), and loopback's own fraction of a millisecond.
        stats
            .slot_rtt_us
            .store(0, ((2.0 * one_way_ms + jitter_ms) * 1000.0) as u64 + 100);
    }
    let mut core = Box::new(ShardCore::new(SEED));
    let p = core.world.spawn_pos(id_of(0));
    core.world.dev_spawn = Some(p);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut shooter = ClientCore::new(SEED, id_of(0), 0);
    let runner = id_of(1);

    let mut up = Pipe::jittery(jitter_ms, 0x9E37_79B9_7F4A_7C15);
    let mut down = Pipe::jittery(jitter_ms, 0xD1B5_4A32_D192_ED03);
    down.unordered = unordered;
    let mut playout_peak = 0.0f64;
    // A faked jitter's held ack and the tick it was taken at.
    let mut held: Option<(u16, u64)> = None;
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    // Where the runner stood at the end of each tick: `hist[T]` is the row
    // `World::tick` writes into the rewind ring for tick `T`.
    let mut hist: Vec<(f32, f32)> = Vec::new();
    // seq → where the shooter drew the runner when it made that frame.
    let mut drawn: Vec<Option<(f32, f32)>> = vec![None; 1 << 16];
    // (tick the frame ran at, seq, favour)
    let mut ran: Vec<(u64, u16, u8)> = Vec::new();

    let mut runner_seq: u16 = 0;
    let mut now = 0.0f64;
    let mut next_tick = 0.0f64;
    let end = secs * 1000.0;
    while now < end {
        // The shooter's frame: advance, sample the runner, send.
        shooter.set_input(0, 0, 128, 0, 0, 0);
        shooter.advance(FRAME_MS);
        if core.world.tick > 60 {
            playout_peak = playout_peak.max(shooter.playout_ticks());
        }
        let n = shooter.poll_input(&mut buf);
        if n > 0 {
            let dg = decode_input(&buf[..n]).expect("client encodes valid input");
            if let Some(last) = dg.frames().last() {
                let mut rs = RemoteState::default();
                if shooter
                    .interp
                    .sample(runner, shooter.render_tick(), &mut rs)
                {
                    drawn[last.seq as usize] = Some((rs.x, rs.z));
                }
            }
            up.send(now + one_way_ms, Lane::Snapshot, &buf[..n]);
        }
        // The shard's ticks that fall inside this frame.
        while next_tick <= now {
            for (_, b) in up.due(now) {
                let mut dg = decode_input(&b).expect("valid");
                match lie {
                    Lie::Honest => {}
                    Lie::Late(lie) => {
                        // Acking late: the newest ack and its bits slid back.
                        dg.snapshot_ack = dg.snapshot_ack.wrapping_sub(lie);
                        dg.ack_bits >>= lie;
                    }
                    Lie::FakeJitter { lag, playout } => {
                        let t = core.world.tick;
                        if t % 90 < 30 {
                            match held {
                                Some((ack, at)) if t < at + u64::from(lag) => {
                                    dg.snapshot_ack = ack;
                                    dg.ack_bits = 0;
                                }
                                _ => held = Some((dg.snapshot_ack, t)),
                            }
                        } else {
                            dg.snapshot_ack = dg.snapshot_ack.wrapping_sub(lag);
                            dg.ack_bits >>= lag;
                            held = None;
                        }
                        dg.playout_ticks = playout;
                    }
                }
                core.push_input(0, &dg);
            }
            runner_seq = runner_seq.wrapping_add(1);
            let mut rdg = InputDatagram::new(0, 0, 4);
            rdg.push(InputFrame {
                seq: runner_seq,
                buttons: BTN_SPRINT,
                yaw: 0,
                pitch: 128,
                move_x: 0,
                move_z: 127,
                sel: 0,
                pitch_fine: 0,
            })
            .expect("one frame fits");
            core.push_input(1, &rdg);

            let t = core.world.tick;
            let granted = ShardStats::get(&stats.favour_granted);
            let sum = ShardStats::get(&stats.favour_sum);
            let mut outs: Vec<(Lane, Vec<u8>)> = Vec::new();
            core.tick_bare(&stats, |lane, slot, bytes| {
                if slot == 0 {
                    outs.push((lane, bytes.to_vec()));
                }
                true
            });
            let b = core
                .world
                .players
                .iter()
                .find(|p| p.active && p.id == runner)
                .expect("runner in world")
                .body;
            assert_eq!(hist.len() as u64, t, "one row per tick");
            hist.push((b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q));
            if ShardStats::get(&stats.favour_granted) == granted + 1 {
                let f = (ShardStats::get(&stats.favour_sum) - sum) as u8;
                ran.push((t, core.clients[0].last_executed, f));
            }
            for (lane, bytes) in outs {
                down.send(now + one_way_ms, lane, &bytes);
            }
            next_tick += TICK_MS;
        }
        for (lane, b) in down.due(now) {
            match lane {
                Lane::Snapshot => {
                    shooter.on_datagram(&b);
                }
                Lane::Event => {
                    shooter.on_stream(&b).expect("server events decode");
                }
            }
        }
        now += FRAME_MS;
    }

    // Where along its run was the runner when it stood at `at`? The
    // fractional tick whose lerp is nearest, searched over the ring's
    // reach before `t`.
    let tick_of = |at: (f32, f32), t: u64| -> Option<f64> {
        let lo = t.saturating_sub(24) as usize;
        let mut best: Option<(f32, f64)> = None;
        for k in lo..(t as usize).min(hist.len() - 1) {
            let (a, b) = (hist[k], hist[k + 1]);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dz * dz;
            if len2 < 0.0025 {
                return None; // the runner was not running here
            }
            let u = (((at.0 - a.0) * dx + (at.1 - a.1) * dz) / len2).clamp(0.0, 1.0);
            let (px, pz) = (a.0 + dx * u, a.1 + dz * u);
            let d2 = (px - at.0).powi(2) + (pz - at.1).powi(2);
            if best.is_none_or(|(bd, _)| d2 < bd) {
                best = Some((d2, k as f64 + u as f64));
            }
        }
        best.filter(|(d2, _)| *d2 < 0.01).map(|(_, k)| k)
    };

    let mut out = Run {
        errors: Vec::new(),
        favours: Vec::new(),
        playout_peak,
        cut_counted: ShardStats::get(&stats.favour_playout_high),
    };
    // Skip the first two seconds: the clock, the playout estimator and the
    // runner's sprint all settle.
    for &(t, seq, f) in ran.iter().filter(|r| r.0 > 60) {
        let Some(at) = drawn[seq as usize] else {
            continue;
        };
        let Some(seen) = tick_of(at, t) else {
            continue;
        };
        out.errors.push((t - f as u64) as f64 - seen);
        out.favours.push(f);
    }
    out
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Up to 150 ms each way the shard judges the swing against the body the
/// shooter drew, to within the half tick a whole-tick rewind can promise.
/// Before this gate the rewind ran a tick short on a perfect link and the
/// 7-tick clamp cost ~3.4 ticks at 100 ms each way: a sprinting body judged
/// 60 cm from where it was on screen.
#[test]
fn the_rewind_lands_on_the_body_the_shooter_drew() {
    for one_way in [0.0, 50.0, 100.0, 150.0] {
        let r = run(one_way, 6.0);
        assert!(
            r.errors.len() > 100,
            "{one_way} ms: too few frames measured"
        );
        let m = mean(&r.errors);
        let close = r.errors.iter().filter(|e| e.abs() <= 0.55).count();
        let clamped = r.favours.iter().filter(|&&f| f == REWIND_MAX_TICKS).count();
        eprintln!(
            "one-way {one_way:>5} ms: n={:>4} mean err {m:+.2} ticks, within half a tick {close}, \
             mean favour {:.2}, clamped {clamped}",
            r.errors.len(),
            r.favours.iter().map(|&f| f as f64).sum::<f64>() / r.favours.len() as f64,
        );
        assert!(
            m.abs() < 0.3,
            "{one_way} ms each way: the rewind is off by {m:+.2} ticks on average \
             (positive: the victim is judged further along than drawn)"
        );
        assert!(
            close * 10 >= r.errors.len() * 9,
            "{one_way} ms each way: only {close} of {} frames rewound to within half a \
             tick of what was drawn",
            r.errors.len()
        );
    }
}

fn mean_favour(r: &Run) -> f64 {
    r.favours.iter().map(|&f| f as f64).sum::<f64>() / r.favours.len() as f64
}

/// A client that acks late looks as stale as a slow link and was paid like
/// one: its own ack history agrees with it, so the cross-check against
/// that history cannot see it, and on a loopback link it bought the whole
/// clamp. Held to the round trip quinn measures, it gets what the link
/// explains plus the slack, and no more.
#[test]
fn acking_late_buys_no_more_rewind_than_the_round_trip_explains() {
    let honest = mean_favour(&run_with(0.0, 4.0, 0, true));
    let unbounded = mean_favour(&run_with(0.0, 4.0, 10, false));
    let bounded = mean_favour(&run_with(0.0, 4.0, 10, true));
    eprintln!("honest {honest:.2}, late acks unbounded {unbounded:.2}, bounded {bounded:.2}");
    assert!(
        unbounded >= honest + 8.0,
        "acking 10 ticks late bought only {unbounded:.2} against {honest:.2}: the harness lies wrong"
    );
    // At most the slack and the trip's rounding up over an honest age of 0.
    assert!(
        bounded <= honest + (server::stats::RTT_AGE_SLACK_TICKS + 1) as f64,
        "acking 10 ticks late on loopback still bought {bounded:.2} against {honest:.2}"
    );
}

/// A client that saw-tooths its acks under the round-trip bound reads as
/// jitter, which raised its playout bound toward the rails' 8; holding its
/// age at the bound and claiming that playout then stacked the two, most of
/// `REWIND_MAX_TICKS` on loopback. Capped in total (`stats::favour_cap`),
/// it buys no more than a client acking late at a constant offset and
/// reporting the shipped delay.
#[test]
fn faking_jitter_buys_no_more_than_acking_late() {
    use server::stats::{favour_cap, rtt_age_bound};
    // Loopback: a frame acks the snapshot its own tick sent, so an ack
    // `bound` ticks late sits exactly on the bound.
    let bound = rtt_age_bound(100).expect("measured");
    let liar = Lie::FakeJitter {
        lag: bound,
        playout: sim_core::limits::PLAYOUT_MAX_TICKS,
    };
    let honest = mean_favour(&run_link(0.0, 0.0, 6.0, Lie::Honest, true));
    // The round trip unmeasured: no bound and no cap, which is everything
    // this liar met before the cap (its ages never pass the bound).
    let unbounded = run_link(0.0, 0.0, 6.0, liar, false);
    let bounded = run_link(0.0, 0.0, 6.0, liar, true);
    let cap = favour_cap(bound);
    eprintln!(
        "honest {honest:.2}, faked jitter unbounded {:.2}, bounded {:.2} (max {}), cap {cap}",
        mean_favour(&unbounded),
        mean_favour(&bounded),
        bounded.favours.iter().max().expect("measured"),
    );
    assert!(
        mean_favour(&unbounded) >= cap as f64 + 1.5,
        "the sawtooth bought only {:.2} uncapped: the harness lies wrong",
        mean_favour(&unbounded)
    );
    assert!(
        bounded.favours.iter().all(|&f| f <= cap),
        "faking jitter on loopback still bought past the cap of {cap}"
    );
    // It stacks two to four ticks past the cap, beyond the band an honest
    // link's tail is forgiven (`stats::FAVOUR_CAP_BAND_TICKS`).
    assert!(
        bounded.cut_counted as usize * 2 >= bounded.favours.len(),
        "faking jitter was counted on only {} frames of {}",
        bounded.cut_counted,
        bounded.favours.len()
    );
}

/// An honest client on a jittery link: its playout rises with the jitter
/// and its acks swing about the smoothed round trip, and none of it may
/// cost it rewind beyond the tail tick: the same link run with the round
/// trip unmeasured (no bound, no cap) mints the same favour, frame for
/// frame, or one tick more on a few frames of the jitteriest links. Nobody
/// is accused either way.
///
/// Every link but the last is short enough that the total cap
/// (`stats::favour_cap`) sits under `REWIND_MAX_TICKS`, and the unbounded
/// favour reaches it, so a cap cutting honest frames shows here. The last
/// is long: its cap is the rewind ceiling, and it tests the age clamp
/// alone. No link drives the client's playout near the rails: an in-order
/// arrival gap is never negative, so RFC 3550's estimate stays under two
/// ticks and the target under six, and jitter that reorders reads lower
/// still (a stale snapshot is not applied).
#[test]
fn an_honest_jittery_client_is_paid_in_full() {
    use server::stats::{favour_cap, rtt_age_bound, FAVOUR_CAP_BAND_TICKS};
    // (one way, jitter, reordered, whether the round trip's tail may cut)
    for (one_way, jitter, unordered, tail) in [
        (10.0, 100.0, true, false),
        (40.0, 80.0, true, false),
        (10.0, 140.0, false, true),
        (0.0, 160.0, true, true),
        (100.0, 100.0, false, false),
    ] {
        let link = Link {
            one_way_ms: one_way,
            jitter_ms: jitter,
            unordered,
        };
        let bounded = run_on(link, 8.0, Lie::Honest, true);
        let free = run_on(link, 8.0, Lie::Honest, false);
        let rtt = ((2.0 * one_way + jitter) * 1000.0) as u32 + 100;
        let cap = favour_cap(rtt_age_bound(rtt).expect("measured"));
        let m = mean(&bounded.errors);
        let cuts: Vec<u8> = bounded
            .favours
            .iter()
            .zip(&free.favours)
            .filter(|(b, f)| b < f)
            .map(|(b, f)| f - b)
            .collect();
        let deepest = cuts.iter().copied().max().unwrap_or(0);
        let reach = *free.favours.iter().max().expect("measured");
        eprintln!(
            "one-way {one_way} ms, jitter {jitter} ms{}: n={} mean err {m:+.2}, mean favour \
             {:.2} (unbounded {:.2}, max {reach}), cap {cap}, cut {} by up to {deepest}, \
             counted {}, playout peak {:.2}",
            if unordered { " reordered" } else { "" },
            bounded.errors.len(),
            mean_favour(&bounded),
            mean_favour(&free),
            cuts.len(),
            bounded.cut_counted,
            bounded.playout_peak,
        );
        assert_eq!(bounded.favours.len(), free.favours.len());
        if one_way < 100.0 {
            // Within a tick of the cap, and past it where the tail may be
            // cut: the harness reaches the regime it claims to test.
            let near = if tail { reach > cap } else { reach + 1 >= cap };
            assert!(
                cap < REWIND_MAX_TICKS && near,
                "{one_way} ms + {jitter} ms: the cap ({cap}) is out of the honest favour's \
                 reach ({reach}), so it cannot be seen cutting"
            );
        } else {
            assert_eq!(
                cap, REWIND_MAX_TICKS,
                "the long link tests the age clamp alone"
            );
        }
        assert_eq!(
            bounded.cut_counted, 0,
            "{one_way} ms + {jitter} ms jitter: an honest client was accused"
        );
        if tail {
            assert!(
                deepest <= FAVOUR_CAP_BAND_TICKS && cuts.len() * 8 <= free.favours.len(),
                "{one_way} ms + {jitter} ms jitter: the bounds cut {} of {} honest frames, \
                 by up to {deepest}",
                cuts.len(),
                free.favours.len(),
            );
        } else {
            assert!(
                cuts.is_empty(),
                "{one_way} ms + {jitter} ms jitter: the bounds cut an honest client"
            );
        }
    }
}
