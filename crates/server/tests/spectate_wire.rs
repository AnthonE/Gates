//! **Spectators over a real socket** (wire v73; `NETCODE.md` §2.3).
//!
//! A real shard, real WebTransport, the real client `Session` on both ends:
//! an agent player walking, and viewers taking read-only seats on it. What
//! each test proves is a claim the design section makes and nothing smaller
//! could check:
//!
//! 1. a watcher sees what the agent's own client sees — the same body at the
//!    same tick, bit for bit, the same inventory, the same health and vitals
//!    — including a watcher that arrived late;
//! 2. a watcher's input and actions reach nothing — a forged client's frames
//!    are refused and the agent does not move, and an action ends the seat;
//! 3. the seat caps refuse at the cap, per target and per shard;
//! 4. consent is the target's: a player who did not declare it cannot be
//!    watched, a human who opted in cannot be watched on a shard without a
//!    feed delay, and a shut door refuses at the hello;
//! 5. the target leaving closes the seat with its reason.
//!
//! Asserts are on observable state, never on elapsed time; every wait is a
//! bounded poll on a fact, with a deadline only as a hang guard.

use k256::ecdsa::SigningKey;
use server::config::{ShardConfig, Spectate};
use server::net::{client_handshake_as, spawn_shard, ShardHandle};
use server::stats::ShardStats;
use server::store::Saves;
use std::collections::BTreeMap;
use std::time::Duration;

use client::{Join, JoinError, Session};

/// The shipped content WITH its spawn kit (a rock and a torch): the one
/// wire suite that wants a non-empty inventory, because "the watcher's
/// inventory equals the agent's" means nothing when both are empty.
fn content_with_kit() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    server::net::bake_all(&c).expect("shipped content bakes")
}

async fn shard(spectate: Spectate, seed: u64) -> ShardHandle {
    let mut cfg = ShardConfig::ephemeral(seed);
    cfg.spectate = spectate;
    spawn_shard(
        cfg,
        content_with_kit(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

fn open(seats: usize, per_target: usize) -> Spectate {
    Spectate {
        seats,
        per_target,
        human_delay_s: 0,
    }
}

/// A throwaway key, generated here and never anywhere else.
fn key(byte: u8) -> client::agentkey::AgentKey {
    let mut b = [0u8; 32];
    b[31] = byte;
    b[7] = 0x5A;
    client::agentkey::AgentKey::from_secret(&b).expect("a valid scalar")
}

fn endpoint(h: &ShardHandle) -> wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client> {
    client::client_endpoint(&h.local_addr.to_string(), Some(&h.cert_hash)).expect("endpoint")
}

/// Pump every session, then yield. One call is one frame for each.
async fn frame(sessions: &mut [&mut Session]) {
    for s in sessions.iter_mut() {
        s.pump(1000.0 / 60.0);
    }
    tokio::time::sleep(Duration::from_millis(8)).await;
}

/// The own record a session's newest snapshot carries for `id`, with its tick.
fn sample(s: &Session, id: u32) -> Option<(u32, protocol::EntityState)> {
    let snap = s.core.view.newest()?;
    let e = snap.entities().iter().find(|e| e.id == id)?;
    Some((snap.header.tick, *e))
}

async fn settle_agent(agent: &mut Session) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !(agent.core.predict.started
            && agent.core.hp > 0
            && agent.core.inv.iter().any(|s| s.count > 0))
        {
            frame(&mut [&mut *agent]).await;
        }
    })
    .await
    .expect("the agent is placed, alive and holding its kit");
}

/// **A watcher sees what the agent's own client sees.** The agent walks, a
/// watcher joins mid-walk (a late join), and from then on every tick both
/// sessions sampled carries the SAME record for the agent's body; the
/// inventory, health and vitals agree; and the watcher's label names the
/// agent. The watcher never sends a frame: the shard counts none refused and
/// its own view of the agent's body is the agent's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watcher_sees_what_the_agent_sees() {
    let h = shard(open(4, 2), 20_260_922).await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let k = key(1);
    let name = protocol::Name::new("jev-test").unwrap();
    let mut agent = Session::connect_agent(&ep, &server, &k, name)
        .await
        .expect("the agent joins");
    settle_agent(&mut agent).await;
    let id = agent.welcome.player_id;

    // Walk, and join the watcher while walking: a late join mid-motion.
    agent.core.set_input(0, 0x2000, 128, 0, 127, 0);
    for _ in 0..20 {
        frame(&mut [&mut agent]).await;
    }
    let mut watcher = Session::watch(&ep, &server, k.address())
        .await
        .expect("a watcher takes a seat");
    assert_eq!(
        watcher.welcome.player_id, id,
        "welcomed as the agent's body"
    );
    let w = watcher.watching.expect("told whose view this is");
    assert_eq!(w.address, k.address());
    assert!(w.agent);
    assert_eq!(w.name.as_str(), "jev-test");
    assert!(watcher.core.spectating());
    // **The join facts arrive before the walks do.** A player hears its
    // health and vitals once, as events at its join; a late watcher hears
    // them from the seat's catch-up, which the shard sends ahead of the
    // seat's first drip on the same ordered stream. So by the time the first
    // catalog batch has landed, they are already known — not waiting for the
    // next hunger tick to announce them.
    tokio::time::timeout(Duration::from_secs(10), async {
        while watcher.core.catalog.count == 0 {
            frame(&mut [&mut agent, &mut watcher]).await;
        }
    })
    .await
    .expect("the watcher's drips begin");
    assert!(
        watcher.core.hp > 0 && watcher.core.max_food > 0,
        "the drips began before the join facts: hp {} max_food {}",
        watcher.core.hp,
        watcher.core.max_food
    );

    let mut theirs: BTreeMap<u32, protocol::EntityState> = BTreeMap::new();
    let mut ours: BTreeMap<u32, protocol::EntityState> = BTreeMap::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut walked = 0u32;
        loop {
            if walked == 90 {
                agent.core.set_input(0, 0x2000, 128, 0, 0, 0);
            }
            walked += 1;
            frame(&mut [&mut agent, &mut watcher]).await;
            if let Some((t, e)) = sample(&agent, id) {
                theirs.insert(t, e);
            }
            if let Some((t, e)) = sample(&watcher, id) {
                ours.insert(t, e);
            }
            let common = ours.keys().filter(|t| theirs.contains_key(t)).count();
            if walked > 150
                && common > 40
                && watcher.core.inv == agent.core.inv
                && watcher.core.hp == agent.core.hp
            {
                break;
            }
        }
    })
    .await
    .expect("the watcher converged on the agent's state");

    // The same body at the same tick, bit for bit — every tick both sampled.
    let mut compared = 0;
    let mut moved = false;
    let mut last: Option<protocol::EntityState> = None;
    for (t, e) in &ours {
        if let Some(a) = theirs.get(t) {
            assert_eq!(e, a, "tick {t}: the watcher's body is not the agent's");
            compared += 1;
            moved |= last.is_some_and(|l| (l.qx, l.qz) != (e.qx, e.qz));
            last = Some(*e);
        }
    }
    assert!(compared > 40, "only {compared} common ticks");
    assert!(moved, "the comparison never covered the body moving");

    // The HUD's own-facts: kit, health and vitals — the late watcher heard
    // health and vitals from the seat's catch-up, not from the join events.
    assert!(agent.core.inv.iter().any(|s| s.count > 0));
    assert_eq!(watcher.core.inv, agent.core.inv, "inventory");
    assert_eq!(watcher.core.worn, agent.core.worn, "wear");
    assert!(agent.core.hp > 0);
    assert_eq!(
        (watcher.core.hp, watcher.core.hp_max),
        (agent.core.hp, agent.core.hp_max),
        "health"
    );
    assert_eq!(
        (watcher.core.food, watcher.core.water),
        (agent.core.food, agent.core.water),
        "vitals"
    );
    assert_eq!(watcher.core.catalog, agent.core.catalog, "item names");
    // **A mirrored fact, end to end**: the agent says something and hears
    // its own echo (the delivery receipt, `ShardCore::pump_chat`); the
    // watcher hears exactly that line, because every message the sim's
    // arms and the chat fan-out address to the agent is copied to its seat.
    let mut b = [0u8; 64];
    let n = protocol::encode_chat(b"hello, watchers", true, &mut b).unwrap();
    agent.send_action(&b[..n]).expect("the agent speaks");
    let (mut heard, mut echoed) = (None, None);
    tokio::time::timeout(Duration::from_secs(10), async {
        while heard.is_none() || echoed.is_none() {
            frame(&mut [&mut agent, &mut watcher]).await;
            if echoed.is_none() {
                echoed = agent.core.pop_chat();
            }
            if heard.is_none() {
                heard = watcher.core.pop_chat();
            }
        }
    })
    .await
    .expect("the line reaches the agent and its watcher");
    assert_eq!(heard, echoed, "the watcher heard something else");
    assert_eq!(heard.map(|(from, _, _)| from), Some(id));
    // The camera has the agent's body to follow.
    let v = watcher.core.spectate_view().expect("a sample to follow");
    assert!(v.x.is_finite() && v.z.is_finite());

    // The seat's ack-only datagrams do their one job: the shard deltas this
    // watcher's snapshots against baselines it acked, like a player's.
    assert!(
        watcher.core.snapshots_delta > 0,
        "a live seat never got a delta: its acks are not reaching the shard"
    );
    // Nothing the watcher did was refused as input, and the seat is counted.
    assert_eq!(ShardStats::get(&h.stats.spectate_input_refused), 0);
    assert_eq!(ShardStats::get(&h.stats.spectate_joins), 1);
    assert_eq!(ShardStats::get(&h.stats.spectators), 1);
    assert_eq!(
        ShardStats::get(&h.stats.players),
        1,
        "a seat is not a player"
    );
    assert_eq!(watcher.core.decode_errors + watcher.core.event_errors, 0);
    // And the local refusal: a verb a HUD offers never reaches the wire.
    assert_eq!(
        watcher.send_action(&[1, 2, 3]),
        Err(client::SendError::ReadOnly)
    );
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A forged watcher: the handshake done by hand, so it can send what our
/// client never would.
async fn raw_watch(
    h: &ShardHandle,
    target: protocol::Address,
) -> (
    wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>,
    wtransport::Connection,
    wtransport::SendStream,
    Result<server::net::Joined, String>,
) {
    let ep = server::botclient::bot_endpoint().expect("endpoint");
    let conn = ep
        .connect(&format!("https://{}", h.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = conn.open_bi().await.expect("bi").await.expect("bi");
    let hello = protocol::Hello {
        flags: protocol::HELLO_SPECTATE,
        target,
        ..protocol::Hello::this_build()
    };
    let joined = tokio::time::timeout(
        Duration::from_secs(10),
        client_handshake_as(
            &mut send,
            &mut recv,
            "127.0.0.1",
            protocol::Address::GUEST,
            &hello,
            |_, _, _| None,
        ),
    )
    .await
    .expect("a handshake answer inside 10 s");
    // The recv half is kept alive by being leaked into a task that drains it,
    // so the shard's event lane is read like a real watcher's.
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        while let Ok(Some(_)) = recv.read(&mut buf).await {}
    });
    (ep, conn, send, joined)
}

/// **A watcher's input reaches nothing.** A forged watcher sends movement
/// frames at the agent's body: every datagram is refused and counted, the
/// agent does not move, and the frames never became input anywhere. Then it
/// sends an action on the reliable lane and the seat is closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watchers_input_and_actions_reach_nothing() {
    let h = shard(open(4, 2), 20_260_923).await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let k = key(2);
    let mut agent = Session::connect_agent(&ep, &server, &k, protocol::Name::new("still").unwrap())
        .await
        .expect("the agent joins");
    settle_agent(&mut agent).await;
    let id = agent.welcome.player_id;
    // Standing still, and settled: its own view of its body is stable.
    for _ in 0..30 {
        frame(&mut [&mut agent]).await;
    }
    let before = sample(&agent, id).expect("a sample").1;
    let ok_before = ShardStats::get(&h.stats.input_dg_ok);

    let (_ep, conn, mut send, joined) = raw_watch(&h, k.address()).await;
    let joined = joined.expect("the forged watcher is seated");
    assert_eq!(joined.welcome.player_id, id);
    // Movement frames, as the agent's own client would send them.
    let mut frames = protocol::InputDatagram::new(0, 0, 4);
    for seq in 1..=10u16 {
        frames
            .push(sim_core::input::InputFrame {
                seq,
                buttons: sim_core::input::BTN_SPRINT,
                yaw: 0,
                pitch: 128,
                move_x: 0,
                move_z: 127,
                sel: 0,
            })
            .unwrap();
    }
    let mut buf = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
    let n = protocol::encode_input(&frames, &mut buf).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while ShardStats::get(&h.stats.spectate_input_refused) < 20 {
            let _ = conn.send_datagram(&buf[..n]);
            frame(&mut [&mut agent]).await;
        }
    })
    .await
    .expect("the forged frames are counted as refused");
    // The agent has not moved: many ticks later its body is where it stood.
    for _ in 0..30 {
        frame(&mut [&mut agent]).await;
    }
    let after = sample(&agent, id).expect("a sample").1;
    assert_eq!(
        (after.qx, after.qz),
        (before.qx, before.qz),
        "a watcher's frames moved the body it watches"
    );
    // The only accepted input datagrams in that window were the agent's own
    // (the forged ones were refused before the input ring): the count of
    // refusals is what went missing from `input_dg_ok`, not a subset of it.
    assert!(ShardStats::get(&h.stats.input_dg_ok) >= ok_before);
    assert_eq!(ShardStats::get(&h.stats.spectators), 1);

    // An action on the reliable lane ends the seat.
    let mut act = [0u8; 16];
    let len = protocol::encode_action_loot(&mut act).unwrap();
    server::net::write_frame(&mut send, &act[..len])
        .await
        .expect("write");
    tokio::time::timeout(Duration::from_secs(10), async {
        while ShardStats::get(&h.stats.spectators) != 0 {
            frame(&mut [&mut agent]).await;
        }
    })
    .await
    .expect("the seat is closed after an action");
    assert_eq!(ShardStats::get(&h.stats.spectate_actions_refused), 1);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn refused(r: Result<Session, JoinError>) -> u8 {
    match r {
        Err(JoinError::Refused(code)) => code,
        Err(e) => panic!("expected a refusal with a code, got {e}"),
        Ok(_) => panic!("expected a refusal, got a seat"),
    }
}

/// **The caps refuse at the cap.** Two seats per target: the third watcher of
/// one agent is refused FULL. Three seats per shard: a fourth watcher of a
/// second agent is refused FULL too, though that agent has room — the shard
/// is out of seats. A watcher that leaves frees its seat for the next.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_seat_caps_refuse_at_the_cap() {
    let h = shard(open(3, 2), 20_260_924).await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let (k1, k2) = (key(3), key(4));
    let n = protocol::Name::new("a").unwrap();
    let mut a1 = Session::connect_agent(&ep, &server, &k1, n).await.unwrap();
    let mut a2 = Session::connect_agent(&ep, &server, &k2, n).await.unwrap();
    settle_agent(&mut a1).await;
    settle_agent(&mut a2).await;

    let w1 = Session::watch(&ep, &server, k1.address())
        .await
        .expect("seat 1");
    let w2 = Session::watch(&ep, &server, k1.address())
        .await
        .expect("seat 2");
    assert_eq!(
        refused(Session::watch(&ep, &server, k1.address()).await),
        protocol::REFUSE_WATCH_FULL,
        "a third watcher of one agent is over the per-target cap"
    );
    let w3 = Session::watch(&ep, &server, k2.address())
        .await
        .expect("seat 3");
    assert_eq!(
        refused(Session::watch(&ep, &server, k2.address()).await),
        protocol::REFUSE_WATCH_FULL,
        "a fourth watcher is over the shard's seats"
    );
    assert_eq!(ShardStats::get(&h.stats.spectate_full), 2);
    // "Any" is refused FULL too once every consenting agent is full or the
    // seats are gone, rather than being told nobody can be watched.
    assert_eq!(
        refused(Session::watch(&ep, &server, protocol::Address::GUEST).await),
        protocol::REFUSE_WATCH_FULL
    );

    // A seat that leaves is a seat for the next watcher.
    drop(w3);
    let again = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            frame(&mut [&mut a1, &mut a2]).await;
            if let Ok(s) = Session::watch(&ep, &server, k2.address()).await {
                return s;
            }
        }
    })
    .await
    .expect("the freed seat is taken");
    assert!(again.watching.is_some());
    drop((w1, w2));
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Sign like a wallet for a person: the client's launcher-shaped signer over
/// `protocol::siwe_message`, through a throwaway key.
fn person(byte: u8) -> (SigningKey, client::agentkey::AgentKey) {
    let mut b = [0u8; 32];
    b[31] = byte;
    b[3] = 0x11;
    (
        SigningKey::from_bytes(&b.into()).expect("valid"),
        client::agentkey::AgentKey::from_secret(&b).expect("valid"),
    )
}

/// **Consent is the target's.** A proven player who declared nothing cannot
/// be watched by address; a human who opted in cannot be watched on a shard
/// that runs no human feeds (the default); "any" skips both and finds no
/// one. Every refusal is the same code — "online but private" is itself a
/// fact about a person.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_target_that_did_not_consent_cannot_be_watched() {
    let h = shard(open(4, 2), 20_260_925).await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let (_sk, plain) = person(5);
    let (_sk2, opted) = person(6);
    let mut p1 = Session::connect_as(&ep, &server, plain.address(), &Join::PLAYER, |d, n, i| {
        plain.sign_proof_hex(d, n, i)
    })
    .await
    .expect("a proven player joins");
    let mut p2 = Session::connect_as(
        &ep,
        &server,
        opted.address(),
        &Join::Player { watchable: true },
        |d, n, i| opted.sign_proof_hex(d, n, i),
    )
    .await
    .expect("a proven player who opted in joins");
    settle_agent(&mut p1).await;
    settle_agent(&mut p2).await;

    for target in [plain.address(), opted.address(), protocol::Address::GUEST] {
        assert_eq!(
            refused(Session::watch(&ep, &server, target).await),
            protocol::REFUSE_WATCH,
            "{target:?} did not consent (or runs no human feed) and must not be watched"
        );
    }
    assert_eq!(ShardStats::get(&h.stats.spectate_refused), 3);
    assert_eq!(ShardStats::get(&h.stats.spectators), 0);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **A shut door refuses at the hello**, before any crypto — the default for
/// every shard, and the one a public shard runs unless its operator opens it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shard_with_the_door_shut_refuses_every_watcher() {
    let h = shard(Spectate::OFF, 20_260_926).await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let k = key(7);
    let mut agent = Session::connect_agent(&ep, &server, &k, protocol::Name::new("a").unwrap())
        .await
        .expect("an agent still plays");
    settle_agent(&mut agent).await;
    assert_eq!(
        refused(Session::watch(&ep, &server, k.address()).await),
        protocol::REFUSE_WATCH
    );
    assert_eq!(ShardStats::get(&h.stats.spectate_refused), 1);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **The target leaving ends the seat, with its reason.** The seat stops
/// being fed the tick the agent's slot turns over, and the connection is
/// closed with `REFUSE_WATCH_ENDED` as its code — so a page can say "the
/// player you were watching left" rather than "connection lost".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_target_leaving_ends_the_seat_with_its_reason() {
    let h = shard(open(4, 2), 20_260_927).await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let k = key(8);
    let mut agent = Session::connect_agent(&ep, &server, &k, protocol::Name::new("a").unwrap())
        .await
        .expect("joins");
    settle_agent(&mut agent).await;
    let (_ep, conn, _send, joined) = raw_watch(&h, k.address()).await;
    joined.expect("seated");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), async {
            while ShardStats::get(&h.stats.spectators) != 1 {
                frame(&mut [&mut agent]).await;
            }
        })
        .await,
        Ok(())
    );
    drop(agent);
    let why = tokio::time::timeout(Duration::from_secs(20), conn.closed())
        .await
        .expect("the seat is closed when its target leaves");
    match why {
        wtransport::error::ConnectionError::ApplicationClosed(close) => assert_eq!(
            close.code().into_inner(),
            protocol::REFUSE_WATCH_ENDED as u64,
            "closed for the wrong reason"
        ),
        other => panic!("closed without the reason: {other:?}"),
    }
    assert_eq!(ShardStats::get(&h.stats.spectate_ended), 1);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **A human who opted in is watchable only behind the shard's delay.** With
/// `spectate_human_delay_s` set, the seat is granted, the label says the feed
/// is a person's (not an agent's), and the delayed stream still carries the
/// target's body — the delay line is on the shard's egress, so it is a fact
/// about what the shard sends and not something a modified client can skip.
/// (The refusal at the default delay of 0 is in
/// `a_target_that_did_not_consent_cannot_be_watched`.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_humans_feed_is_admitted_only_behind_the_delay() {
    // Longer than the shard's sent-snapshot ring covers, which is what makes
    // the delay OBSERVABLE without a clock: every ack this watcher sends
    // names a snapshot the shard has already forgotten, so every snapshot it
    // is sent is a zero-state keyframe and never a delta.
    let ring_s = (sim_core::limits::SENT_SNAPSHOT_RING as u64
        * sim_core::limits::SNAPSHOT_INTERVAL_TICKS
        / sim_core::limits::TICK_HZ as u64) as u32;
    let h = shard(
        Spectate {
            seats: 2,
            per_target: 2,
            human_delay_s: ring_s + 1,
        },
        20_260_929,
    )
    .await;
    let server = h.local_addr.to_string();
    let ep = endpoint(&h);
    let (_sk, opted) = person(9);
    let mut p = Session::connect_as(
        &ep,
        &server,
        opted.address(),
        &Join::Player { watchable: true },
        |d, n, i| opted.sign_proof_hex(d, n, i),
    )
    .await
    .expect("a proven player who opted in joins");
    settle_agent(&mut p).await;
    let id = p.welcome.player_id;
    let mut w = Session::watch(&ep, &server, opted.address())
        .await
        .expect("seated behind the delay");
    let watch = w.watching.expect("told whose view");
    assert!(!watch.agent, "a person's feed, not an agent's");
    assert!(client::ui::spectate::label(&watch).ends_with("delayed feed"));
    // The delayed stream arrives, and carries the watched body.
    tokio::time::timeout(Duration::from_secs(30), async {
        while w.core.snapshots_applied < 30 || w.core.catalog.count == 0 {
            frame(&mut [&mut p, &mut w]).await;
        }
    })
    .await
    .expect("the delayed feed arrives");
    assert!(sample(&w, id).is_some(), "the watched body is in the feed");
    // The delay, seen in the bytes: acks older than the shard's ring buy no
    // baseline, so the whole feed is keyframes.
    assert_eq!(
        w.core.snapshots_delta, 0,
        "a delayed seat got a delta — its feed is not behind the ring"
    );
    assert_eq!(w.core.decode_errors + w.core.event_errors, 0);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}
