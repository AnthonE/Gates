//! **One wallet, one body, over a real socket.**
//!
//! A second session for a wallet whose body was awake used to miss the
//! sleeper (there was none) and be seated from the store's record: two
//! bodies carrying one inventory, each free to stash its copy. The accept
//! loop now closes the session already in (`REFUSE_IN_USE`), holds the
//! newcomer until the sim has put that body to sleep, and seats it there.

use server::config::ShardConfig;
use server::net::{spawn_shard, ShardHandle};
use server::stats::ShardStats;
use server::store::Saves;
use std::time::Duration;

use client::agentkey::AgentKey;
use client::Session;

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
async fn a_second_session_for_a_seated_wallet_takes_over_the_one_body() {
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

    // The same wallet again, while the first body is awake. It is seated —
    // once the first session has been closed and its body put to sleep, and
    // on that body: a takeover, never a second body from the record.
    let second = tokio::time::timeout(
        Duration::from_secs(20),
        Session::connect_agent(
            &ep,
            &h.local_addr.to_string(),
            &k,
            protocol::Name::new("twin").unwrap(),
        ),
    )
    .await
    .expect("an answer")
    .expect("the second session is seated");
    assert!(!second.closed());
    assert_eq!(ShardStats::get(&h.stats.parked_in_use), 1);
    assert_eq!(ShardStats::get(&h.stats.refused_in_use), 0);
    assert_eq!(ShardStats::get(&h.stats.takeovers), 1, "woke the sleeper");
    assert_eq!(
        ShardStats::get(&h.stats.saves_restored),
        0,
        "no body from a record"
    );

    // ...and the session that was in is closed, told why.
    tokio::time::timeout(Duration::from_secs(20), async {
        while !first.closed() {
            frame(&mut [&mut first]).await;
        }
    })
    .await
    .expect("the first session is closed");
    assert_eq!(first.close_code(), Some(protocol::REFUSE_IN_USE as u64));
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **A rejoin inside the read cooldown is told its skins without a second
/// platform read**: what the wallet's last read said rides the join, so a
/// reconnect loop costs the platform nothing and the player loses nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rejoin_inside_the_cooldown_is_told_its_skins_without_a_second_read() {
    let mut cfg = ShardConfig::ephemeral(20_261_016);
    cfg.skins.all = true;
    let h = spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let ep =
        client::client_endpoint(&h.local_addr.to_string(), Some(&h.cert_hash)).expect("endpoint");
    let k = key(8);
    let mut first = join(&ep, &h, &k).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        while first.core.skins_owned.is_empty() {
            frame(&mut [&mut first]).await;
        }
    })
    .await
    .expect("the door's read lands");
    let owned = first.core.skins_owned;
    assert_eq!(ShardStats::get(&h.stats.skins_read), 1);

    let mut second = tokio::time::timeout(
        Duration::from_secs(20),
        Session::connect_agent(
            &ep,
            &h.local_addr.to_string(),
            &k,
            protocol::Name::new("twin").unwrap(),
        ),
    )
    .await
    .expect("an answer")
    .expect("the second session is seated");
    tokio::time::timeout(Duration::from_secs(20), async {
        while second.core.skins_owned != owned {
            frame(&mut [&mut second]).await;
        }
    })
    .await
    .expect("told what it owns");
    assert_eq!(ShardStats::get(&h.stats.skins_read), 1, "no second read");
    assert_eq!(ShardStats::get(&h.stats.skins_replayed), 1);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}
