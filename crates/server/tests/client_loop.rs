//! The client-loop gate (M0 "connect, predict/reconcile, interpolate"):
//! `ClientCore` — the exact struct the native client drives, out of the
//! shared `client-core` crate — against `ShardCore`, through real encoded
//! datagrams both ways. No sockets, no clocks: fully deterministic, so the
//! asserts are exact and quotable from this shared box.
//!
//! The two claims under test (DESIGN.md §5.6, NETCODE.md §3):
//! - clean delivery ⇒ prediction is **bit-exact** — zero mispredictions,
//!   because both sides sim the quantized values they transmit;
//! - loss ⇒ mispredictions happen, corrections flow, and the client
//!   re-converges to the server's exact quantized state.

use client_core::core::ClientCore;
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

/// Put every join on one real beach spawn. `dev_spawn` is the documented
/// override for exactly this — "it exists so a test can put two clients
/// inside AOI range on demand" (DECISIONS.md §open, dev spawn override).
/// Until the beach spawn ring landed, these two ids happened to hash
/// within AOI of each other; that was luck, and what this gate is about
/// is the client loop, not where the world puts a fresh player. The point
/// is the one the ring itself picked for client 0, so it stays a real
/// spawn on a real shore that no worldgen change can sink.
fn pin_together(core: &mut ShardCore) {
    let p = core.world.spawn_pos(id_of(0));
    core.world.dev_spawn = Some(p);
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
    core.tick_bare(stats, |lane, slot, bytes| {
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
    let mut core = Box::new(ShardCore::new(SEED));
    pin_together(&mut core);
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
        let mut rs = client_core::interp::RemoteState::default();
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
    let mut core = Box::new(ShardCore::new(SEED));
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

/// The interest set and interpolation survive churn: a player walks into
/// view and then disconnects — and **the body stays**, which is the whole
/// of the sleepers slice seen from the far end of the wire.
///
/// This test used to assert the opposite, and it was right to until
/// `Command::Leave` stopped clearing `active`. The old sentence — "the
/// clients drop him" — was the netcode's faithful report of a design where
/// logging off deleted your body, which `reference/SAVES.md` §1 is about
/// the reference game replacing. What the client must now observe is a body
/// that is still in the interest set, still interpolating, and *marked*: a
/// raider has to be able to see there is nobody home, and a snapshot that
/// dropped the entity would be the server telling them there is nobody
/// there at all.
///
/// The removal path is not untested by this change — it is exercised by
/// eviction and by AOI exit, which are the two ways an entity legitimately
/// leaves a client's set now.
#[test]
fn churn_keeps_the_body_and_marks_it_asleep() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    pin_together(&mut core);
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
        clients[0].1.interp.ids().any(|id| id == id_of(1)),
        "the disconnected player's body left the world — sleepers are not \
         reaching the wire"
    );
    let mut rs = client_core::interp::RemoteState::default();
    assert!(
        clients[0]
            .1
            .interp
            .sample(id_of(1), clients[0].1.render_tick(), &mut rs),
        "the body is in the set but has no samples to draw from"
    );
    assert!(
        rs.sleeping,
        "the body is still drawn as a live player — the wire's sleeping bit \
         is not arriving, so a raider cannot tell nobody is home"
    );
}
