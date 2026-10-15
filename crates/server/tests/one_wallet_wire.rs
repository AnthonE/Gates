//! **One wallet, one body, over a real socket.**
//!
//! A second session for a wallet whose body was awake used to miss the
//! sleeper (there was none) and be seated from the store's record: two
//! bodies carrying one inventory, each free to stash its copy. The accept
//! loop now refuses the newcomer with `REFUSE_IN_USE` and closes the session
//! already in with the same code, so the player's next join wakes the body
//! the first one left asleep.

use server::config::ShardConfig;
use server::net::{spawn_shard, ShardHandle};
use server::stats::ShardStats;
use server::store::Saves;
use std::time::Duration;

use client::agentkey::AgentKey;
use client::{JoinError, Session};

fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&c).expect("shipped content bakes");
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

/// A throwaway key, generated here and never anywhere else.
fn key(byte: u8) -> AgentKey {
    let mut b = [0u8; 32];
    b[31] = byte;
    b[9] = 0x5D;
    AgentKey::from_secret(&b).expect("a valid scalar")
}

type Ep = wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>;

/// Pump every session, then yield. One call is one frame for each.
async fn frame(sessions: &mut [&mut Session]) {
    for s in sessions.iter_mut() {
        s.pump(1000.0 / 60.0);
    }
    tokio::time::sleep(Duration::from_millis(8)).await;
}

/// Join as `k` and wait until the body is placed and alive.
async fn join(ep: &Ep, h: &ShardHandle, k: &AgentKey) -> Session {
    let mut s = Session::connect_agent(
        ep,
        &h.local_addr.to_string(),
        k,
        protocol::Name::new("twin").unwrap(),
    )
    .await
    .expect("joins");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !(s.core.predict.started && s.core.hp > 0) {
            frame(&mut [&mut s]).await;
        }
    })
    .await
    .expect("placed and alive");
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_session_for_a_seated_wallet_is_refused_and_takes_over() {
    let h = spawn_shard(
        ShardConfig::ephemeral(20_261_015),
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let ep =
        client::client_endpoint(&h.local_addr.to_string(), Some(&h.cert_hash)).expect("endpoint");
    let k = key(7);
    let mut first = join(&ep, &h, &k).await;

    // The same wallet again, while the first body is awake: refused at the
    // door, before it can claim a slot.
    let name = protocol::Name::new("twin").unwrap();
    match Session::connect_agent(&ep, &h.local_addr.to_string(), &k, name).await {
        Err(JoinError::Refused(code)) => assert_eq!(code, protocol::REFUSE_IN_USE),
        Err(e) => panic!("expected a refusal with a code, got {e}"),
        Ok(_) => panic!("a second session was seated for a wallet already playing"),
    }
    assert_eq!(ShardStats::get(&h.stats.refused_in_use), 1);

    // ...and the session already in is closed with the same reason.
    tokio::time::timeout(Duration::from_secs(20), async {
        while !first.closed() {
            frame(&mut [&mut first]).await;
        }
    })
    .await
    .expect("the first session is closed");
    assert_eq!(first.close_code(), Some(protocol::REFUSE_IN_USE as u64));
    drop(first);

    // Joining again lands on the body the first session left asleep — a
    // takeover, never a second body seated from a record.
    let back = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let name = protocol::Name::new("twin").unwrap();
            match Session::connect_agent(&ep, &h.local_addr.to_string(), &k, name).await {
                Ok(s) => return s,
                // The old slot frees once the sim has applied its leave.
                Err(JoinError::Refused(code)) if code == protocol::REFUSE_IN_USE => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                Err(e) => panic!("rejoin failed: {e}"),
            }
        }
    })
    .await
    .expect("the wallet gets back in");
    assert!(!back.closed());
    assert_eq!(ShardStats::get(&h.stats.takeovers), 1, "woke the sleeper");
    assert_eq!(
        ShardStats::get(&h.stats.saves_restored),
        0,
        "no body from a record"
    );
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}
