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
///
/// **This loop used to call `c.predict.decay_error()` itself**, commented
/// "the render loop's once-per-frame call" — and so did seven other
/// `*_wire.rs` harnesses, while the actual render loop called it nowhere.
/// The drain assertion at the bottom of `own_prediction_converges_under_loss`
/// was therefore checking the harness rather than the product: it proved the
/// decay function worked when invoked and never asked whether the shipping
/// client invoked it (it did not, for the whole life of the code). The call
/// lives inside `ClientCore::advance` now, which both this harness and
/// `Session::pump` already go through, so the line below is deleted rather
/// than moved and the assertion means what it always claimed to.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    mut lose: impl FnMut() -> bool,
) {
    // One closure, both directions, called in the same order as it always
    // was (input sends first, then snapshot deliveries) so the seeded loss
    // sequences in the suites below keep their exact draws.
    pump_dir(core, stats, clients, TICK_MS, &mut |_input, _slot| lose());
}

/// `pump` with the knobs the netcode-v2 gates need: a per-call `dt_ms` (a
/// client fed two ticks of wall time produces two input frames in one
/// datagram — how the server's buffer is legitimately driven deep), and
/// loss that knows its direction (`input` true for the C→S send, false for
/// a S→C snapshot to `slot`) so a one-way stall — the stop-under-starvation
/// shape — is expressible.
fn pump_dir(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    dt_ms: f64,
    lose: &mut impl FnMut(bool, usize) -> bool,
) {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    for (slot, c) in clients.iter_mut() {
        c.advance(dt_ms);
        let n = c.poll_input(&mut buf);
        if n > 0 && !lose(true, *slot) {
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
        if !lose(false, slot) {
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

/// **The gate for the defect this whole suite was blind to: `err` accumulates
/// forever if nothing drains it, and nothing drained it in the product.**
///
/// Every other test here runs a *lossless* in-process wire, where
/// `clean_delivery_predicts_bit_exact` proves the prediction is bit-exact and
/// therefore `err` is never written at all. That is why eight harnesses could
/// hand-roll the missing `decay_error` call, and why the one assertion that
/// would have caught its absence (`error_magnitude() < 0.05`, above) passed
/// for the life of the code: with no loss there is no correction, and with a
/// short lossy burst followed by a clean tail there is time to drain. Neither
/// resembles three minutes on a real network.
///
/// So this runs the shape that found it: a **sustained** 2 % loss for 5,400
/// ticks — three minutes at 30 Hz, the length of the session that reported a
/// trunk sitting a foot to the side of the thing that stopped you — while the
/// body wanders through the real scattered world and collides with real trees.
///
/// It tracks BOTH numbers, because only one of them is the assertion and the
/// other is the evidence:
/// - `shadow` is the vector sum of every correction, which is exactly what
///   `err` held before the fix (`predict.rs` writes `err += old - new` and
///   nothing subtracted). It is measured here rather than asserted — it is
///   what the player saw.
/// - `error_magnitude()` is what `err` holds now. That is the assertion.
#[test]
fn the_correction_offset_does_not_accumulate_over_a_long_lossy_session() {
    /// Three minutes at 30 Hz — the length of the session that reported it.
    const TICKS: u32 = 5_400;

    /// Ticks of clean delivery after the lossy phase. Corrections stop, so
    /// whatever the offset holds here is what it FAILS to drain — which is
    /// the defect, as distinct from the size of any one correction.
    const CLEAN_TAIL: u32 = 120;

    /// One session at one loss rate. Returns
    /// `(confirms, mispredicts, peak_live_err, final_live_err, no_decay_err)`.
    fn session(loss_percent: u32) -> (u64, u64, f32, f32, f32) {
        let stats = ShardStats::default();
        let mut core = Box::new(ShardCore::new(SEED));
        pin_together(&mut core);
        assert!(core.connect(0, id_of(0)));
        let mut c = ClientCore::new(SEED, id_of(0), 0);
        let mut rng = Pcg32::new(SEED, 77);
        let mut loss_rng = Pcg32::new(SEED, 78);
        let mut yaw = 100u16;

        // The no-decay accumulator: every correction summed, never drained —
        // exactly what `err` held before the fix.
        let mut shadow = [0.0f32; 3];
        let mut peak_live = 0.0f32;

        for tick in 0..TICKS + CLEAN_TAIL {
            // The tail delivers everything: the offset has nothing new to
            // absorb and every frame decays it. `decay_error` runs once per
            // `advance`, which is once per 30 Hz tick HERE and once per
            // render frame in the client — so a 60–144 Hz client drains
            // faster than this harness and the bound below is conservative.
            let loss_percent = if tick < TICKS { loss_percent } else { 0 };
            steer(&mut c, &mut rng, &mut yaw, tick < TICKS);

            // `pump`'s body, opened up so the reconcile can be measured on
            // both sides of the snapshot that causes it.
            let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
            c.advance(TICK_MS);
            let n = c.poll_input(&mut buf);
            if n > 0 && loss_rng.next_u32() % 100 >= loss_percent {
                let dg = decode_input(&buf[..n]).expect("client encodes valid input");
                core.push_input(0, &dg);
            }
            let mut outs: Vec<Vec<u8>> = Vec::new();
            let mut ev_outs: Vec<Vec<u8>> = Vec::new();
            core.tick_bare(&stats, |lane, _slot, bytes| {
                match lane {
                    Lane::Snapshot => outs.push(bytes.to_vec()),
                    Lane::Event => ev_outs.push(bytes.to_vec()),
                }
                true
            });
            for bytes in ev_outs {
                c.on_stream(&bytes).expect("server events decode");
            }
            for bytes in outs {
                if loss_rng.next_u32() % 100 < loss_percent {
                    continue;
                }
                // `position()` is the sim-truth body with no smoothing, so the
                // difference across the reconcile IS the correction `err`
                // takes. Gated on `started` for the same reason `reconcile`
                // gates its own write: the FIRST snapshot is not a correction,
                // it is the spawn adoption, moving the body from the default
                // `[0,0,0]` to a real beach ~2.1 km away. Counting it reads as
                // a two-kilometre error and drowns what is being measured.
                let started = c.predict.started;
                let before = c.predict.position();
                c.on_datagram(&bytes);
                let after = c.predict.position();
                if started {
                    for i in 0..3 {
                        shadow[i] += before[i] - after[i];
                    }
                }
            }
            peak_live = peak_live.max(c.predict.error_magnitude());
        }
        let sh = (shadow[0] * shadow[0] + shadow[1] * shadow[1] + shadow[2] * shadow[2]).sqrt();
        (
            c.predict.confirmations,
            c.predict.mispredictions,
            peak_live,
            c.predict.error_magnitude(),
            sh,
        )
    }

    println!(
        "\n{TICKS} ticks (3 min at 30 Hz) + {CLEAN_TAIL} clean, wandering the real \
         scattered world\n\
         loss  reconciles  confirmed  mispredicts   peak err   settled err   \
         if never drained"
    );
    for loss in [0u32, 2, 10, 30] {
        let (ok, bad, peak, settled, shadow) = session(loss);
        let total = ok + bad;
        println!(
            "{loss:3}%  {total:10}  {:8.2}%  {bad:11}  {peak:7.4} m   {settled:9.4} m   \
             {shadow:13.4} m",
            100.0 * ok as f64 / total.max(1) as f64,
        );
        // **The assertion is about DRAINING, not about size.** A single
        // correction under heavy loss is legitimately large — 0.53 m at 30 %,
        // measured — and smoothing it is the whole job. What may never happen
        // is that it stays: with corrections stopped for `CLEAN_TAIL` ticks
        // the offset must be gone. Without the decay call this reads the
        // `if never drained` column instead, which is red at every rate that
        // mispredicts at all.
        assert!(
            settled < 0.05,
            "at {loss}% loss the smoothing offset settled at {settled:.4} m after \
             {CLEAN_TAIL} clean ticks — it is accumulating rather than decaying, \
             which is the camera sitting that far from the body the world collides \
             against (it would hold {shadow:.4} m with no decay at all)"
        );
        // A client that mispredicted constantly would also hold a small `err`
        // and would be worthless, so the offset bound is only meaningful
        // beside this.
        assert!(
            ok > bad,
            "at {loss}% loss most reconciles must confirm bit-exact; \
             got {ok} confirms to {bad} mispredicts"
        );
    }
}

/// **A body that has been killed says so on the wire** — the `dead` bit,
/// v48, seen from the far end exactly as the sleeper above is.
///
/// The defect this gate exists for is a drawing one with a netcode cause. A
/// corpse keeps its slot, its position, its facing and its interest entry
/// until its owner leaves the death screen (`sim-core/world.rs` `die` — the
/// screen is waiting on the body), and until v48 not one bit of that record
/// said the person was out of the fight. So the client drew a killed player
/// standing at idle, and "is that player still coming for me" was
/// answerable only from the kill feed, which names ids and not the body in
/// front of you.
///
/// The kill is `DEATH_BY_CLOCK` rather than a swing, and that is on
/// purpose: what is under test is the *record*, not the weapon. Starving
/// the body reaches `World::die` — the one function that sets the flag —
/// through the shortest honest path, with no combat content to arm and no
/// second player's aim to place. Any other cause writes the same bit.
#[test]
fn a_killed_body_is_marked_dead_on_the_wire() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    pin_together(&mut core);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut rng = Pcg32::new(SEED, 23);
    let mut yaws = [0u16, 9000u16];
    for _ in 0..120u32 {
        for (i, (_, c)) in clients.iter_mut().enumerate() {
            steer(c, &mut rng, &mut yaws[i], true);
        }
        pump(&mut core, &stats, &mut clients, || false);
    }

    let watcher = 0usize;
    let victim_id = id_of(1);
    let mut rs = client_core::interp::RemoteState::default();
    assert!(
        clients[watcher]
            .1
            .interp
            .sample(victim_id, clients[watcher].1.render_tick(), &mut rs),
        "the two bodies are not in each other's interest set — this gate \
         cannot see anything about how one of them is drawn"
    );
    assert!(
        !rs.dead,
        "a living player is already marked dead, so the assertion below \
         would pass without the kill"
    );

    // Arm the clock and empty the victim. `probe_fixture`'s starve and
    // dehydrate rates are set to kill inside a counted window, which is
    // exactly what this needs — no wall-clock anywhere, only ticks.
    core.world.survival = sim_core::survival::SurvivalContent::probe_fixture();
    {
        let p = core
            .world
            .players
            .iter_mut()
            .find(|p| p.active && p.id == victim_id)
            .expect("the victim is in the world");
        p.food = 0;
        p.water = 0;
        p.hp = 1;
    }
    // Both clients keep pumping. The victim is on the death screen, not
    // gone — they are still connected and the server still owes them
    // snapshots, which is the difference between this and the sleeper case
    // above and is the whole point: nobody left.
    for _ in 0..90u32 {
        for (i, (_, c)) in clients.iter_mut().enumerate() {
            steer(c, &mut rng, &mut yaws[i], false);
        }
        pump(&mut core, &stats, &mut clients, || false);
    }
    assert!(
        core.world
            .players
            .iter()
            .any(|p| p.active && p.id == victim_id && p.dead),
        "the victim never died — the arrangement, not the wire, is what \
         failed"
    );

    assert!(
        clients[watcher].1.interp.ids().any(|id| id == victim_id),
        "the corpse left the interest set — a death is not a removal, the \
         body stays until its owner leaves the death screen"
    );
    assert!(
        clients[watcher]
            .1
            .interp
            .sample(victim_id, clients[watcher].1.render_tick(), &mut rs),
        "the corpse is in the set but has no samples to draw from"
    );
    assert!(
        rs.dead,
        "the body is still drawn as a live player — the wire's `dead` bit \
         is not arriving, so a killer cannot tell from the body in front of \
         them that the fight is over"
    );
}

/// **Netcode v2, the throttle catch-up: a deep buffer drains without a
/// single misprediction.** The client runs two input ticks per server tick
/// for a stretch — a legitimate clock lead, the exact thing the consume
/// throttle exists for — so the server's buffer climbs past
/// `INPUT_THROTTLE_DEPTH` and the throttle consumes two frames per tick.
/// Both must MOVE the body (`Command::InputPair`): until netcode v2 the
/// older frame was consumed and never stepped, so every throttle tick was
/// one tick of walking silently dropped and a guaranteed reconcile
/// mismatch — this test is red by construction on that code.
#[test]
fn the_throttle_catchup_stays_bit_exact() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    pin_together(&mut core);
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    let mut rng = Pcg32::new(SEED, 23);
    let mut yaw = 7_000u16;

    // Settle: adopt the spawn, walk normally.
    for _ in 0..50u32 {
        steer(&mut clients[0].1, &mut rng, &mut yaw, true);
        pump(&mut core, &stats, &mut clients, || false);
    }
    // Sprint ahead: two client ticks of wall time per server tick. Each
    // pump posts one datagram carrying the whole unacked tail, so the
    // buffer legitimately deepens by one frame per tick.
    let mut deepest = 0u8;
    for _ in 0..10u32 {
        steer(&mut clients[0].1, &mut rng, &mut yaw, true);
        pump_dir(
            &mut core,
            &stats,
            &mut clients,
            TICK_MS * 2.0,
            &mut |_, _| false,
        );
        deepest = deepest.max(clients[0].1.view.buffered_depth);
    }
    // Drain: back to lockstep; the dilation nudge and the throttle bring
    // the buffer home.
    for _ in 0..150u32 {
        steer(&mut clients[0].1, &mut rng, &mut yaw, true);
        pump(&mut core, &stats, &mut clients, || false);
    }

    let c = &clients[0].1;
    // The throttle demonstrably fired, by pigeonhole rather than by a
    // depth reading: the sprint phase produced two frames per server tick
    // with zero loss, every produced frame ended up executed (the tail
    // below is empty — nothing pending, nothing gap-jumped), so some ticks
    // MUST have executed two — and those rode `Command::InputPair`. The
    // gauge reading beside it proves the buffer genuinely ran deep: the
    // throttle holds the post-consume plateau at 5 (pre-consume 7, past
    // the threshold of 6, minus the two it takes).
    assert!(
        deepest >= 5,
        "the buffer never ran deep (deepest {deepest}) — the throttle was \
         not exercised and this test proved nothing"
    );
    // Steady-state tail: the frame produced this tick plus at most one
    // awaiting its snapshot (15 Hz reporting of a 30 Hz clock) — the
    // sprint's ten-frame backlog is gone, so nothing was left unexecuted
    // and nothing was gap-jumped (zero loss makes a jump impossible).
    assert!(
        c.predict.tail().len() <= 2,
        "the backlog never drained ({} frames still unacked)",
        c.predict.tail().len()
    );
    assert_eq!(
        c.predict.mispredictions, 0,
        "a throttle tick diverged: the older consumed frame did not move \
         the body the way the client's ring did"
    );
    assert!(c.predict.confirmations > 100, "reconciles actually ran");
}

/// **Netcode v2, the stop-snap kill: a one-way input stall no longer
/// walks the body onward at full stride.** The player walks, stops, and
/// the release frame never arrives (C→S loss; snapshots still flow — the
/// asymmetry a real uplink stall has). The server used to re-run the last
/// frame verbatim forever: at walk speed that is 0.1 m per starved tick —
/// a metre of phantom travel over ten — all of it pulled back through the
/// reconcile as the snap this slice is named for. The decay ramp
/// (`sim_core::input::decay_frame`) spends the stale frame to zero in
/// three ticks, so the overshoot is bounded at about one tick of walking.
#[test]
fn a_stop_under_starvation_does_not_overshoot() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    pin_together(&mut core);
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];

    // Walk straight at a fixed yaw — no sprint, so one tick is 0.1 m and
    // the bound below is arithmetic rather than a tuned tolerance.
    for _ in 0..60u32 {
        clients[0].1.set_input(0, 12_000, 0, 0, 127, 0);
        pump(&mut core, &stats, &mut clients, || false);
    }
    let at_stall = server_body(&core, id_of(0));

    // The stop that never arrives: the client releases the stick, every
    // input datagram is lost, snapshots keep flowing.
    for _ in 0..12u32 {
        clients[0].1.set_input(0, 12_000, 0, 0, 0, 0);
        pump_dir(&mut core, &stats, &mut clients, TICK_MS, &mut |input, _| {
            input
        });
    }
    let stalled = server_body(&core, id_of(0));
    let drift_m = {
        let dx = (stalled.qx - at_stall.qx) as f32 * sim_core::movement::POS_XZ_Q;
        let dz = (stalled.qz - at_stall.qz) as f32 * sim_core::movement::POS_XZ_Q;
        (dx * dx + dz * dz).sqrt()
    };
    // Decay envelope: 2/3 + 1/3 of one walk tick ≈ 0.1 m, plus quanta
    // slack. Full-strength reuse is 1.2 m over these twelve ticks.
    assert!(
        drift_m < 0.3,
        "the server walked a stopped player {drift_m:.2} m past the stall \
         — starved reuse is running at full strength"
    );
    assert!(
        clients[0].1.view.repeat_count >= 3,
        "the header never reported the starve (repeat_count {}) — the \
         gauge this slice added is not flowing",
        clients[0].1.view.repeat_count
    );

    // Delivery resumes; the two ends reconverge to the same quantized
    // stop, bit for bit.
    for _ in 0..40u32 {
        clients[0].1.set_input(0, 12_000, 0, 0, 0, 0);
        pump(&mut core, &stats, &mut clients, || false);
    }
    let server = server_body(&core, id_of(0));
    let client = clients[0].1.predict.body;
    assert_eq!(
        (client.qx, client.qy, client.qz),
        (server.qx, server.qy, server.qz),
        "the ends did not reconverge after the stall"
    );
}
