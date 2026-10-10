//! **The admin lane's socket half, over a real socket** (`NOW.md` §0ad2).
//!
//! `admin_wire.rs` stops at the ring: it proves a `/kick` from the right
//! wallet leaves the sim. This drives what happens after it, on a real shard
//! with the real client `Session` at both ends:
//!
//! 1. a kicked player's connection closes with `REFUSE_ADMIN`, as its close
//!    code and as the reason posted on the event lane, and the admin hears
//!    that it happened;
//! 2. a banned wallet that dials again is refused at the door with the same
//!    code (the check that was never written: `bans.contains` had no
//!    caller), the ban file holds it, and `/unban` lets it back in;
//! 3. a stranger's `/kick` closes nothing.
//!
//! Asserts are on observable state; every wait is a bounded poll on a fact,
//! with a deadline only as a hang guard.

use server::admin::Admins;
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
    b[9] = 0x3C;
    AgentKey::from_secret(&b).expect("a valid scalar")
}

/// The wallet as the shard files it: `0x` and 40 lowercase hex.
fn wallet(k: &AgentKey) -> String {
    String::from_utf8(k.address().to_hex().to_vec()).expect("ascii")
}

/// A shard whose one admin is `admin`.
async fn shard(admin: &AgentKey, seed: u64, ban_file: Option<String>) -> ShardHandle {
    let mut cfg = ShardConfig::ephemeral(seed);
    cfg.admins = Admins::parse(&wallet(admin)).expect("a legal list");
    cfg.ban_file = ban_file;
    spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

fn endpoint(h: &ShardHandle) -> Ep {
    client::client_endpoint(&h.local_addr.to_string(), Some(&h.cert_hash)).expect("endpoint")
}

/// Pump every session, then yield. One call is one frame for each.
async fn frame(sessions: &mut [&mut Session]) {
    for s in sessions.iter_mut() {
        s.pump(1000.0 / 60.0);
    }
    tokio::time::sleep(Duration::from_millis(8)).await;
}

type Ep = wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>;

/// Join as `k` and wait until the body is placed and alive. `ep` outlives
/// the session: it holds the socket.
async fn join(ep: &Ep, h: &ShardHandle, k: &AgentKey, name: &str) -> Session {
    let mut s = Session::connect_agent(
        ep,
        &h.local_addr.to_string(),
        k,
        protocol::Name::new(name).unwrap(),
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

/// Type `line` into chat, which is all an admin verb is on the wire.
fn type_line(s: &Session, line: &str) {
    let mut b = [0u8; 64];
    let n = protocol::encode_chat(line.as_bytes(), true, &mut b).unwrap();
    s.send_action(&b[..n]).expect("the line is queued");
}

/// The next line `s` hears from `from` (0 is the house), as text.
async fn hear(s: &mut Session, from: u32) -> String {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            while let Some((who, _, text)) = s.core.pop_chat() {
                if who == from {
                    return String::from_utf8(text.as_bytes().to_vec()).unwrap();
                }
            }
            frame(&mut [&mut *s]).await;
        }
    })
    .await
    .expect("a line arrives")
}

/// Pump `victim` (and `admin`, whose chat must keep flowing) until the
/// shard hangs up on it, then assert it was told why.
async fn closed_by_admin(admin: &mut Session, victim: &mut Session) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !victim.closed() {
            frame(&mut [&mut *admin, &mut *victim]).await;
        }
    })
    .await
    .expect("the shard closes the target's connection");
    assert_eq!(
        victim.close_code(),
        Some(protocol::REFUSE_ADMIN as u64),
        "closed without the admin's reason"
    );
    assert_eq!(
        victim.posted_refusal(),
        Some(protocol::REFUSE_ADMIN),
        "the reason must also arrive on the event lane: a browser cannot \
         read the connection's close code"
    );
}

/// **A kick closes the connection with its reason.** The admin types
/// `/kick <id>` into chat; the target's connection ends with `REFUSE_ADMIN`
/// and the admin is told it worked. The admin stays connected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_kick_closes_the_target_with_the_admin_reason() {
    let (ka, kv) = (key(1), key(2));
    let h = shard(&ka, 20_261_010, None).await;
    let ep = endpoint(&h);
    let mut admin = join(&ep, &h, &ka, "admin").await;
    let mut victim = join(&ep, &h, &kv, "victim").await;
    let id = victim.welcome.player_id;

    type_line(&admin, &format!("/kick {id}"));
    closed_by_admin(&mut admin, &mut victim).await;
    assert_eq!(hear(&mut admin, 0).await, format!("[server] kicked {id}"));
    assert_eq!(ShardStats::get(&h.stats.admin_kicked), 1);
    assert!(!admin.closed(), "the admin kicked themself");
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **A ban is refused at the door, and `/unban` lifts it.** `/ban <id>`
/// closes the target like a kick, writes the wallet to the ban file and
/// tells the admin the wallet's front. The same wallet dialling again is
/// refused `REFUSE_ADMIN` before it reaches a slot. `/unban` with the front
/// the answer printed lifts it — out of the file too — and the wallet joins.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_banned_wallet_is_refused_at_the_door_until_unbanned() {
    let (ka, kv) = (key(3), key(4));
    let file = std::env::temp_dir().join(format!("gates-ban-wire-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let h = shard(&ka, 20_261_011, Some(file.display().to_string())).await;
    let ep = endpoint(&h);
    let mut admin = join(&ep, &h, &ka, "admin").await;
    let mut victim = join(&ep, &h, &kv, "victim").await;
    let id = victim.welcome.player_id;
    let w = wallet(&kv);

    type_line(&admin, &format!("/ban {id}"));
    closed_by_admin(&mut admin, &mut victim).await;
    assert_eq!(
        hear(&mut admin, 0).await,
        format!("[server] banned {id} ({})", &w[..14])
    );
    assert_eq!(ShardStats::get(&h.stats.admin_kicked), 1);
    assert!(
        std::fs::read_to_string(&file).unwrap().contains(&w),
        "the ban is in its file"
    );
    drop(victim);

    let server = h.local_addr.to_string();
    let name = protocol::Name::new("victim").unwrap();
    match Session::connect_agent(&ep, &server, &kv, name).await {
        Err(JoinError::Refused(code)) => assert_eq!(code, protocol::REFUSE_ADMIN),
        Err(e) => panic!("expected a refusal with a code, got {e}"),
        Ok(_) => panic!("a banned wallet was let back in"),
    }
    assert_eq!(ShardStats::get(&h.stats.refused_banned), 1);

    // The front the answer printed, without its `0x`: what an admin types.
    type_line(&admin, &format!("/unban {}", &w[2..14]));
    assert_eq!(
        hear(&mut admin, 0).await,
        format!("[server] unbanned {}", &w[..14])
    );
    assert_eq!(ShardStats::get(&h.stats.admin_unbanned), 1);
    assert!(
        !std::fs::read_to_string(&file).unwrap().contains(&w),
        "the lift is in the file too"
    );
    let back = join(&ep, &h, &kv, "victim").await;
    assert!(!back.closed());
    assert_eq!(ShardStats::get(&h.stats.refused_banned), 1);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = std::fs::remove_file(&file);
}

/// **A stranger's `/kick` closes nothing.** A player not on the list types
/// the admin's id. The barrier that proves nothing is still in flight: the
/// stranger's next line is echoed (so the kick line was handled), and then
/// an admin act goes through the accept loop's same FIFO ring and is
/// answered — a kick queued ahead of it would have landed first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_strangers_kick_closes_nothing() {
    let (ka, ks) = (key(5), key(6));
    let h = shard(&ka, 20_261_012, None).await;
    let ep = endpoint(&h);
    let mut admin = join(&ep, &h, &ka, "admin").await;
    let mut stranger = join(&ep, &h, &ks, "stranger").await;
    let target = admin.welcome.player_id;

    type_line(&stranger, &format!("/kick {target}"));
    type_line(&stranger, "after");
    let me = stranger.welcome.player_id;
    assert_eq!(hear(&mut stranger, me).await, "after");

    type_line(&admin, "/unban abcdef");
    assert_eq!(hear(&mut admin, 0).await, "[server] no ban starts abcdef");
    assert_eq!(ShardStats::get(&h.stats.admin_kicked), 0);
    for _ in 0..30 {
        frame(&mut [&mut admin, &mut stranger]).await;
    }
    assert!(!admin.closed(), "a stranger's kick closed the admin");
    assert!(!stranger.closed());
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}
