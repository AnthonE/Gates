//! The client-loop gate (M0 "connect, predict/reconcile, interpolate"):
//! `ClientCore` — the exact struct the browser drives through the wasm
//! bridge — against `ShardCore`, through real encoded datagrams both
//! ways. No sockets, no clocks: fully deterministic, so the asserts are
//! exact and quotable from this shared box.
//!
//! The two claims under test (DESIGN.md §5.6, NETCODE.md §3):
//! - clean delivery ⇒ prediction is **bit-exact** — zero mispredictions,
//!   because both sides sim the quantized values they transmit;
//! - loss ⇒ mispredictions happen, corrections flow, and the client
//!   re-converges to the server's exact quantized state.

use client_wasm::core::ClientCore;
use protocol::decode_input;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::input::BTN_SPRINT;
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::movement::Body;
use sim_core::rng::Pcg32;

const SEED: u64 = 0x6A7E5;
const TICK_MS: f64 = 1000.0 / 30.0;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn server_body(core: &ShardCore, id: u32) -> Body {
    core.world
        .players
        .iter()
        .find(|p| p.active && p.id == id)
        .expect("player in world")
        .body
}

/// Drive the shared input state for a client: a deterministic wander.
fn steer(c: &mut ClientCore, rng: &mut Pcg32, yaw: &mut u16, moving: bool) {
    *yaw = yaw.wrapping_add((rng.next_u32() % 700) as u16);
    let buttons = if rng.next_u32().is_multiple_of(4) {
        BTN_SPRINT
    } else {
        0
    };
    let (mx, mz) = if moving {
        (((rng.next_u32() % 255) as i32 - 127) as i8, 127i8)
    } else {
        (0, 0)
    };
    c.set_input(buttons, *yaw, 0, mx, mz, 0);
}

/// One lockstep pump: clients advance one tick and post inputs, the shard
/// ticks and posts snapshots. `lose` decides per-datagram delivery.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    mut lose: impl FnMut() -> bool,
) {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    for (slot, c) in clients.iter_mut() {
        c.advance(TICK_MS);
        c.predict.decay_error(); // the render loop's once-per-frame call
        let n = c.poll_input(&mut buf);
        if n > 0 && !lose() {
            let dg = decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut outs: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut ev_outs: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => outs.push((slot, bytes.to_vec())),
            Lane::Event => ev_outs.push((slot, bytes.to_vec())),
        }
        true
    });
    // The event lane is reliable: loss never applies to it.
    for (slot, bytes) in ev_outs {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_stream(&bytes).expect("server events decode");
        }
    }
    for (slot, bytes) in outs {
        if !lose() {
            let c = clients
                .iter_mut()
                .find(|(s, _)| *s == slot)
                .map(|(_, c)| c)
                .expect("snapshot for a known client");
            c.on_datagram(&bytes);
        }
    }
}

#[test]
fn clean_delivery_predicts_bit_exact() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut rng = Pcg32::new(SEED, 11);
    let mut yaws = [100u16, 40_000u16];

    for tick in 0..600u32 {
        for (i, (_, c)) in clients.iter_mut().enumerate() {
            steer(c, &mut rng, &mut yaws[i], tick < 500);
        }
        pump(&mut core, &stats, &mut clients, || false);
    }

    for (slot, c) in &clients {
        assert!(c.predict.started, "client {slot} adopted its spawn");
        assert_eq!(
            c.predict.mispredictions, 0,
            "client {slot}: clean delivery must predict bit-exact"
        );
        assert!(c.predict.confirmations > 200, "reconciliation engaged");
        assert!(c.snapshots_delta > 100, "ack loop produced deltas");
        assert_eq!(c.decode_errors, 0);
        // Quiescent and fully acked: predicted state IS the server state.
        let sb = server_body(&core, id_of(*slot));
        assert_eq!(c.predict.body.qx, sb.qx);
        assert_eq!(c.predict.body.qy, sb.qy);
        assert_eq!(c.predict.body.qz, sb.qz);
        // Each client interpolates the other guy near his server truth.
        let other = id_of(1 - *slot);
        let ob = server_body(&core, other);
        let mut rs = client_wasm::interp::RemoteState::default();
        assert!(
            c.interp.sample(other, c.render_tick(), &mut rs),
            "remote sampled"
        );
        let dx = rs.x - ob.qx as f32 * 0.03;
        let dz = rs.z - ob.qz as f32 * 0.03;
        assert!(
            (dx * dx + dz * dz).sqrt() < 2.0,
            "client {slot} interpolates the other within the delay bound (off by {})",
            (dx * dx + dz * dz).sqrt()
        );
    }
}

#[test]
fn loss_corrects_and_reconverges() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    let mut rng = Pcg32::new(SEED, 13);
    let mut loss_rng = Pcg32::new(SEED, 17);
    let mut yaw = 7u16;

    // 400 ticks of movement under 30% independent datagram loss.
    for _ in 0..400u32 {
        steer(&mut clients[0].1, &mut rng, &mut yaw, true);
        pump(&mut core, &stats, &mut clients, || {
            loss_rng.next_u32() % 10 < 3
        });
    }
    let lossy_phase = clients[0].1.predict.mispredictions;

    // Then quiesce with clean delivery: everything must reconverge.
    for _ in 0..120u32 {
        clients[0].1.set_input(0, yaw, 0, 0, 0, 0);
        pump(&mut core, &stats, &mut clients, || false);
    }
    let c = &clients[0].1;
    assert!(c.snapshots_applied > 100, "snapshots flowed despite loss");
    let sb = server_body(&core, id_of(0));
    assert_eq!(c.predict.body.qx, sb.qx, "reconverged x");
    assert_eq!(c.predict.body.qy, sb.qy, "reconverged y");
    assert_eq!(c.predict.body.qz, sb.qz, "reconverged z");
    assert_eq!(
        c.predict.mispredictions, lossy_phase,
        "clean tail added no new mispredictions"
    );
    // The correction offset drains once corrections stop.
    assert!(
        c.predict.error_magnitude() < 0.05,
        "smoothing offset drained, at {}",
        c.predict.error_magnitude()
    );
}

/// The interest set and interpolation survive churn: a third player joins,
/// walks into view, disconnects; the clients drop him.
#[test]
fn churn_removes_remotes() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut rng = Pcg32::new(SEED, 19);
    let mut yaws = [0u16, 9000u16];
    for _ in 0..120u32 {
        for (i, (_, c)) in clients.iter_mut().enumerate() {
            steer(c, &mut rng, &mut yaws[i], true);
        }
        pump(&mut core, &stats, &mut clients, || false);
    }
    assert!(clients[0].1.interp.ids().any(|id| id == id_of(1)));

    core.disconnect(1);
    let survivors = &mut clients[..1];
    for _ in 0..30u32 {
        steer(&mut survivors[0].1, &mut rng, &mut yaws[0], false);
        pump(&mut core, &stats, survivors, || false);
    }
    assert!(
        !clients[0].1.interp.ids().any(|id| id == id_of(1)),
        "disconnected remote removed from the interp set"
    );
}
