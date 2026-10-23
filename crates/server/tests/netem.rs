//! **The netcode under a bad network, in numbers** (NETCODE.md §11's
//! `netem profiles`).
//!
//! `client_loop.rs` proves the loop with every datagram landing on the tick
//! it was sent. This puts a real link between the same two structs — the
//! shipping `ClientCore` and `ShardCore` through encoded bytes — on a
//! simulated clock: the client draws at 60 fps, the shard ticks at 30 Hz,
//! and each direction gets latency, jitter (which reorders datagrams), loss
//! and an optional blackout. The event lane is reliable and ordered, so it
//! is only delayed: past a blackout it lands when the link comes back, the
//! way QUIC's retransmit would. No sockets and no wall clock: every run is
//! a function of its seeds, so every number below is reproducible.
//!
//! Two players wander the real island in view of each other (sprints,
//! turns, stops). Client 0 is measured: how often prediction held, how big
//! the corrections were, how far the other player is drawn from where the
//! server had him at the moment drawn, how far behind that moment is, how
//! often the shard ran out of its inputs, and what it cost in bytes.
//!
//! `cargo test --release -p server --test netem -- --nocapture` prints the
//! table.

use client_core::core::ClientCore;
use client_core::interp::RemoteState;
use protocol::decode_input;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::input::BTN_SPRINT;
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::rng::Pcg32;

const SEED: u64 = 0x6A7E5;
/// Simulated time is integer microseconds: a 30 Hz tick does not divide a
/// millisecond, and a float clock summed over two minutes drifts.
const FRAME_US: u64 = 1_000_000 / 60;
const RUN_S: u64 = 90;
/// Nothing is measured before this: the join, the spawn adoption and the
/// clock's first settle are not the steady state.
const WARMUP_S: u64 = 5;

fn tick_at_us(k: u64) -> u64 {
    k * 1_000_000 / 30
}

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

#[derive(Clone, Copy)]
struct Link {
    name: &'static str,
    /// One way, each direction.
    lat_ms: u32,
    /// Added per datagram, uniform in `0..=jitter_ms`; datagrams reorder.
    jitter_ms: u32,
    loss_pct: u32,
    /// Everything dropped (datagrams) or held (the event lane) in
    /// `[from, from + len)` seconds.
    blackout: Option<(u64, u64)>,
}

#[rustfmt::skip]
const PROFILES: [Link; 5] = [
    Link { name: "LAN", lat_ms: 2, jitter_ms: 1, loss_pct: 0, blackout: None },
    Link { name: "50ms/0%", lat_ms: 25, jitter_ms: 5, loss_pct: 0, blackout: None },
    Link { name: "150ms/5%", lat_ms: 75, jitter_ms: 15, loss_pct: 5, blackout: None },
    Link { name: "250ms/10%", lat_ms: 125, jitter_ms: 30, loss_pct: 10, blackout: None },
    Link { name: "100ms+2s out", lat_ms: 50, jitter_ms: 10, loss_pct: 1, blackout: Some((40, 2)) },
];

struct Dg {
    at: u64,
    slot: usize,
    bytes: Vec<u8>,
}

struct Wire {
    link: Link,
    rng: Pcg32,
}

impl Wire {
    fn dark(&self, t: u64) -> bool {
        self.link
            .blackout
            .is_some_and(|(from, len)| t >= from * 1_000_000 && t < (from + len) * 1_000_000)
    }

    /// When a datagram sent at `t` lands, or `None` if the link took it.
    fn datagram(&mut self, t: u64) -> Option<u64> {
        let lost = self.rng.next_u32() % 100 < self.link.loss_pct;
        let jitter = (self.rng.next_u32() % (self.link.jitter_ms + 1)) as u64;
        if lost || self.dark(t) {
            return None;
        }
        Some(t + (self.link.lat_ms as u64 + jitter) * 1000)
    }

    /// When a reliable message sent at `t` lands: after the blackout if it
    /// was sent into one, never before the previous one on the lane.
    fn stream(&self, t: u64, prev: u64) -> u64 {
        let sent = match self.link.blackout {
            Some((from, len)) if self.dark(t) => (from + len) * 1_000_000,
            _ => t,
        };
        (sent + self.link.lat_ms as u64 * 1000).max(prev)
    }
}

/// One player's deterministic wander: a heading held for a while, sprints,
/// and stops — the stop is where a late input shows.
struct Walker {
    rng: Pcg32,
    yaw: u16,
    mx: i8,
    mz: i8,
    buttons: u8,
    hold: u32,
}

impl Walker {
    fn new(stream: u64) -> Self {
        Self {
            rng: Pcg32::new(SEED, stream),
            yaw: 0,
            mx: 0,
            mz: 0,
            buttons: 0,
            hold: 0,
        }
    }

    fn frame(&mut self, c: &mut ClientCore) {
        if self.hold == 0 {
            self.hold = 20 + self.rng.next_u32() % 70;
            let r = self.rng.next_u32() % 10;
            self.yaw = self.yaw.wrapping_add((self.rng.next_u32() % 20_000) as u16);
            (self.mx, self.mz) = if r < 2 {
                (0, 0)
            } else {
                (((self.rng.next_u32() % 255) as i32 - 127) as i8, 127)
            };
            self.buttons = if r >= 6 { BTN_SPRINT } else { 0 };
        }
        self.hold -= 1;
        // A mouse: the look drifts a little every frame.
        self.yaw = self.yaw.wrapping_add((self.rng.next_u32() % 200) as u16);
        c.set_input(self.buttons, self.yaw, 0, self.mx, self.mz, 0);
    }
}

#[derive(Default, Debug)]
struct Report {
    confirmed: u64,
    minor: u64,
    mispredicted: u64,
    correction_n: u64,
    correction_sum_m: f64,
    correction_max_m: f32,
    offset_max_m: f32,
    remote_err_cm: Vec<f32>,
    remote_behind_ms: Vec<f32>,
    starved_ticks: u64,
    ticks: u64,
    snap_bytes: u64,
    input_bytes: u64,
    seconds: f64,
    playout_ticks: f64,
    recover_ms: Option<f64>,
    decode_errors: u64,
}

fn pct(v: &mut [f32], p: usize) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[(v.len() * p / 100).min(v.len() - 1)]
}

fn run(link: Link) -> Report {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.dev_spawn = Some(core.world.spawn_pos(id_of(0)));
    let mut clients: Vec<ClientCore> = (0..2)
        .map(|s| {
            assert!(core.connect(s, id_of(s)));
            ClientCore::new(SEED, id_of(s), 0)
        })
        .collect();
    let mut walkers = [Walker::new(31), Walker::new(32)];
    let mut up = Wire {
        link,
        rng: Pcg32::new(SEED, 41),
    };
    let mut down = Wire {
        link,
        rng: Pcg32::new(SEED, 42),
    };

    let mut to_server: Vec<Dg> = Vec::new();
    let mut to_client: Vec<Dg> = Vec::new();
    // Reliable lane, per client, in send order.
    let mut lane: [Vec<(u64, Vec<u8>)>; 2] = [Vec::new(), Vec::new()];
    let mut lane_last = [0u64; 2];
    // Where the server had each player at the end of each tick.
    let mut truth: [Vec<[f32; 3]>; 2] = [Vec::new(), Vec::new()];
    let tick0 = core.world.tick;

    let mut r = Report::default();
    let warm = WARMUP_S * 1_000_000;
    let end = RUN_S * 1_000_000;
    let blackout_end = link.blackout.map(|(f, l)| (f + l) * 1_000_000);
    let mut confirmed_at_end = None;
    // The predictor's ledgers as they stood when measuring began.
    let mut at_warm: Option<(u64, u64, u64)> = None;
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let (mut frame, mut tick) = (0u64, 1u64);
    loop {
        let tf = frame * FRAME_US;
        let tt = tick_at_us(tick);
        if tf.min(tt) >= end {
            break;
        }
        if tt <= tf {
            // --- a shard tick.
            to_server.sort_by_key(|d| d.at);
            let due = to_server.iter().take_while(|d| d.at <= tt).count();
            for d in to_server.drain(..due) {
                let dg = decode_input(&d.bytes).expect("client encodes valid input");
                core.push_input(d.slot, &dg);
            }
            let mut outs: Vec<(Lane, usize, Vec<u8>)> = Vec::new();
            core.tick_bare(&stats, |l, slot, bytes| {
                outs.push((l, slot, bytes.to_vec()));
                true
            });
            for (l, slot, bytes) in outs {
                if slot >= 2 {
                    continue;
                }
                match l {
                    Lane::Snapshot => {
                        if tt >= warm && slot == 0 {
                            r.snap_bytes += bytes.len() as u64;
                        }
                        if let Some(at) = down.datagram(tt) {
                            to_client.push(Dg { at, slot, bytes });
                        }
                    }
                    Lane::Event => {
                        let at = down.stream(tt, lane_last[slot]);
                        lane_last[slot] = at;
                        lane[slot].push((at, bytes));
                    }
                }
            }
            for (s, t) in truth.iter_mut().enumerate() {
                let p = core
                    .world
                    .players
                    .iter()
                    .find(|p| p.active && p.id == id_of(s))
                    .expect("player in world");
                let at = (core.world.tick - tick0) as usize;
                t.resize(at + 1, [0.0; 3]);
                t[at] = [
                    p.body.qx as f32 * POS_XZ_Q,
                    p.body.qy as f32 * POS_Y_Q,
                    p.body.qz as f32 * POS_XZ_Q,
                ];
            }
            if tt >= warm {
                r.ticks += 1;
                if core.clients[0].repeat_report() > 0 {
                    r.starved_ticks += 1;
                }
            }
            tick += 1;
            continue;
        }

        // --- a client frame.
        for (s, c) in clients.iter_mut().enumerate() {
            let due = lane[s].iter().take_while(|(at, _)| *at <= tf).count();
            for (_, bytes) in lane[s].drain(..due) {
                c.on_stream(&bytes).expect("server events decode");
            }
        }
        to_client.sort_by_key(|d| d.at);
        let due = to_client.iter().take_while(|d| d.at <= tf).count();
        for d in to_client.drain(..due) {
            let c = &mut clients[d.slot];
            let measured = d.slot == 0 && tf >= warm && c.predict.started;
            let before = c.predict.position();
            let (ok0, mis0) = (
                c.predict.confirmations,
                c.predict.mispredictions + c.predict.corrections_minor,
            );
            c.on_datagram(&d.bytes);
            if measured && c.predict.mispredictions + c.predict.corrections_minor > mis0 {
                let after = c.predict.position();
                let jump = ((before[0] - after[0]).powi(2)
                    + (before[1] - after[1]).powi(2)
                    + (before[2] - after[2]).powi(2))
                .sqrt();
                r.correction_n += 1;
                r.correction_sum_m += jump as f64;
                r.correction_max_m = r.correction_max_m.max(jump);
            }
            if d.slot == 0 && blackout_end.is_some_and(|e| tf >= e) && c.predict.confirmations > ok0
            {
                confirmed_at_end.get_or_insert(tf);
            }
        }
        for (s, c) in clients.iter_mut().enumerate() {
            walkers[s].frame(c);
            c.advance(FRAME_US as f64 / 1000.0);
            let n = c.poll_input(&mut buf);
            if n > 0 {
                if s == 0 && tf >= warm {
                    r.input_bytes += n as u64;
                }
                if let Some(at) = up.datagram(tf) {
                    to_server.push(Dg {
                        at,
                        slot: s,
                        bytes: buf[..n].to_vec(),
                    });
                }
            }
        }
        let c = &clients[0];
        if tf >= warm && at_warm.is_none() {
            at_warm = Some((
                c.predict.confirmations,
                c.predict.corrections_minor,
                c.predict.mispredictions,
            ));
        }
        if tf >= warm && c.predict.started {
            r.offset_max_m = r.offset_max_m.max(c.predict.error_magnitude());
            // Client 0 draws client 1 at `render_tick`; the server had him
            // there at that tick (between two, linearly — the same model
            // the client interpolates with, so the error is the netcode's).
            let at = c.render_tick() - tick0 as f64;
            let mut rs = RemoteState::default();
            let t = &truth[1];
            if at >= 0.0
                && (at as usize + 1) < t.len()
                && c.interp.sample(id_of(1), c.render_tick(), &mut rs)
            {
                let (i, f) = (at as usize, (at - at.floor()) as f32);
                let w = |k: usize| t[i][k] + (t[i + 1][k] - t[i][k]) * f;
                let err = ((rs.x - w(0)).powi(2) + (rs.z - w(2)).powi(2)).sqrt();
                r.remote_err_cm.push(err * 100.0);
                let now = (core.world.tick - tick0) as f64;
                r.remote_behind_ms.push(((now - at) * 1000.0 / 30.0) as f32);
            }
        }
        if let (Some(e), Some(_)) = (blackout_end, confirmed_at_end) {
            if r.recover_ms.is_none() && tf >= e && c.predict.error_magnitude() < 0.05 {
                r.recover_ms = Some((tf - e) as f64 / 1000.0);
            }
        }
        frame += 1;
    }
    let c = &clients[0];
    let (ok0, minor0, mis0) = at_warm.expect("the run outlasts the warmup");
    r.confirmed = c.predict.confirmations - ok0;
    r.minor = c.predict.corrections_minor - minor0;
    r.mispredicted = c.predict.mispredictions - mis0;
    r.playout_ticks = c.playout_ticks();
    r.decode_errors = c.decode_errors;
    r.seconds = (RUN_S - WARMUP_S) as f64;
    r
}

#[test]
fn the_netcode_holds_under_a_bad_network() {
    println!(
        "\n{RUN_S} s, two players wandering in view, client at 60 fps, measured after {WARMUP_S} s\n\
         {:<14} {:>7} {:>7} {:>8} {:>9} {:>9} {:>8} {:>8} {:>8} {:>9} {:>7} {:>9} {:>8}",
        "link", "exact%", "minor%", "mis/min", "corr avg", "corr max", "rem p99", "rem max",
        "behind", "starved%", "playout", "down", "up"
    );
    for link in PROFILES {
        let mut r = run(link);
        let total = (r.confirmed + r.minor + r.mispredicted).max(1) as f64;
        let exact = 100.0 * r.confirmed as f64 / total;
        let mis_min = r.mispredicted as f64 / (r.seconds / 60.0);
        let p99 = pct(&mut r.remote_err_cm, 99);
        let rmax = pct(&mut r.remote_err_cm, 100);
        let behind = pct(&mut r.remote_behind_ms, 50);
        let starved = 100.0 * r.starved_ticks as f64 / r.ticks.max(1) as f64;
        let corr_max_cm = 100.0 * r.correction_max_m;
        println!(
            "{:<14} {:>6.1}% {:>6.1}% {:>8.1} {:>7.1}cm {:>7.1}cm {:>6.1}cm {:>6.1}cm {:>6.0}ms {:>8.2}% {:>5.1}tk {:>5.1}kB/s {:>4.1}kB/s{}",
            link.name,
            exact,
            100.0 * r.minor as f64 / total,
            mis_min,
            100.0 * r.correction_sum_m / r.correction_n.max(1) as f64,
            corr_max_cm,
            p99,
            rmax,
            behind,
            starved,
            r.playout_ticks,
            r.snap_bytes as f64 / r.seconds / 1000.0,
            r.input_bytes as f64 / r.seconds / 1000.0,
            r.recover_ms.map_or(String::new(), |ms| format!("  recovered {ms:.0} ms after"))
        );
        assert_eq!(r.decode_errors, 0, "{}: every datagram decodes", link.name);
        assert!(
            r.remote_err_cm.len() > 4000,
            "{}: the remote was drawn",
            link.name
        );
        // The bar, per link. Measured 2026-10-23 with headroom; a red here is
        // a feel regression on that link, so read the table before moving one.
        match link.name {
            // Clean links: prediction never wrong, the remote drawn exactly
            // where the server had him.
            "LAN" | "50ms/0%" => {
                assert_eq!(
                    r.mispredicted, 0,
                    "{}: a clean link mispredicted",
                    link.name
                );
                assert!(rmax < 1.0, "{}: remote drawn {rmax:.1} cm off", link.name);
                assert_eq!(
                    r.starved_ticks, 0,
                    "{}: the shard ran out of inputs",
                    link.name
                );
            }
            // A bad but common link: still feels local.
            "150ms/5%" => {
                assert!(
                    mis_min < 10.0,
                    "150/5: {mis_min:.1} mispredictions a minute"
                );
                assert!(
                    corr_max_cm < 25.0,
                    "150/5: a {corr_max_cm:.1} cm correction"
                );
                assert!(p99 < 10.0, "150/5: remote p99 {p99:.1} cm");
                assert!(starved < 1.0, "150/5: starved {starved:.2}% of ticks");
            }
            // Degrades honestly: corrections stay nudges, never a snap.
            "250ms/10%" => {
                assert!(exact > 95.0, "250/10: only {exact:.1}% exact");
                assert!(
                    corr_max_cm < 100.0,
                    "250/10: a {corr_max_cm:.1} cm correction"
                );
                assert!(rmax < 50.0, "250/10: remote off by {rmax:.1} cm");
            }
            // Two seconds of nothing, then the link back: settled inside a
            // second, without a snap.
            _ => {
                let ms = r.recover_ms.expect("recovered from the blackout");
                assert!(ms < 1000.0, "blackout: {ms:.0} ms to settle");
                assert!(
                    corr_max_cm < 100.0,
                    "blackout: a {corr_max_cm:.1} cm correction"
                );
            }
        }
    }
}
