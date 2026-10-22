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

/// A one-way pipe with a fixed delay, delivering in order.
#[derive(Default)]
struct Pipe {
    q: VecDeque<(f64, Lane, Vec<u8>)>,
}

impl Pipe {
    fn send(&mut self, at: f64, lane: Lane, bytes: &[u8]) {
        self.q.push_back((at, lane, bytes.to_vec()));
    }
    fn due(&mut self, now: f64) -> Vec<(Lane, Vec<u8>)> {
        let mut out = Vec::new();
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
    let stats = ShardStats::default();
    if measured {
        // The pipe's round trip, and loopback's own fraction of a millisecond.
        stats
            .slot_rtt_us
            .store(0, (2.0 * one_way_ms * 1000.0) as u64 + 100);
    }
    let mut core = Box::new(ShardCore::new(SEED));
    let p = core.world.spawn_pos(id_of(0));
    core.world.dev_spawn = Some(p);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut shooter = ClientCore::new(SEED, id_of(0), 0);
    let runner = id_of(1);

    let mut up = Pipe::default();
    let mut down = Pipe::default();
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
                // Acking late: the newest ack and its bits slid back.
                dg.snapshot_ack = dg.snapshot_ack.wrapping_sub(lie);
                dg.ack_bits >>= lie;
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
