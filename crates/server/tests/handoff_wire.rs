//! **Moving a player between two shards, over real sockets** (wire v152).
//!
//! `server::handoff`'s unit tests own the text, the checks and the platform's
//! vector. This owns what they cannot see: two real shards and the real
//! client `Session`, where the arena admits a player on the island's
//! handoff with no signature, refuses it a second time, and `/leave` sends
//! them back on a handoff the island admits. Both shards refuse guests, so an
//! admission is the proof.
//!
//! The addresses a handoff names are labels (`island.test:7000`): the client
//! dials each shard's real socket, and the shard checks the label it was
//! configured with.

use server::config::ShardConfig;
use server::handoff;
use server::net::{spawn_shard, ShardHandle};
use server::store::Saves;
use std::time::Duration;

use client::agentkey::AgentKey;
use client::{JoinError, Session};

const ISLAND: &str = "island.test:7000";
const ARENA: &str = "arena.test:7100";

fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&c).expect("shipped content bakes");
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

fn seed(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn public(seed: &[u8; 32]) -> [u8; 32] {
    use ring::signature::KeyPair;
    let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed).expect("a seed");
    let mut out = [0u8; 32];
    out.copy_from_slice(pair.public_key().as_ref());
    out
}

/// The handoff half of `shard.toml` for the island and the arena: each
/// admits the other's key and signs with its own.
fn handoff_cfg(me: &str) -> handoff::Config {
    let (island, arena) = (seed(0x11), seed(0x22));
    let (mine, theirs) = if me == ISLAND {
        (("island-1", island), ("arena-1", arena))
    } else {
        (("arena-1", arena), ("island-1", island))
    };
    handoff::Config {
        addr: Some(me.into()),
        keys: vec![(theirs.0.into(), public(&theirs.1))],
        signer: Some((mine.0.into(), mine.1)),
        arena: Some(ARENA.into()),
    }
}

async fn shard(me: &str, seed: u64) -> ShardHandle {
    let mut cfg = ShardConfig::ephemeral(seed);
    cfg.require_auth = true;
    cfg.handoff = handoff_cfg(me);
    spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

type Ep = wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>;

fn endpoint(h: &ShardHandle) -> Ep {
    client::client_endpoint(&h.local_addr.to_string(), Some(&h.cert_hash)).expect("endpoint")
}

fn key(byte: u8) -> AgentKey {
    let mut b = [0u8; 32];
    b[31] = byte;
    b[9] = 0x5A;
    AgentKey::from_secret(&b).expect("a valid scalar")
}

fn wallet(k: &AgentKey) -> String {
    String::from_utf8(k.address().to_hex().to_vec()).expect("ascii")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The travel message a shard configured as `from` would send `player` to `to`.
fn travel(from: &str, player: &str, to: &str) -> protocol::Travel {
    let tok = handoff::issue(&handoff_cfg(from), player, to, b"", now()).expect("issues");
    protocol::Travel::new(to, protocol::Handoff::new(tok.as_bytes(), b"").unwrap()).unwrap()
}

async fn frame(s: &mut Session) {
    s.pump(1000.0 / 60.0);
    tokio::time::sleep(Duration::from_millis(8)).await;
}

async fn placed(mut s: Session) -> Session {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !(s.core.predict.started && s.core.hp > 0) {
            frame(&mut s).await;
        }
    })
    .await
    .expect("placed and alive");
    s
}

fn type_line(s: &Session, line: &str) {
    let mut b = [0u8; 64];
    let n = protocol::encode_chat(line.as_bytes(), protocol::Channel::Global, &mut b).unwrap();
    s.send_action(&b[..n]).expect("the line is queued");
}

/// The next line `s` hears from the house, as text.
async fn hear(s: &mut Session) -> String {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            while let Some((who, _, text)) = s.core.pop_chat() {
                if who == 0 {
                    return String::from_utf8(text.as_bytes().to_vec()).unwrap();
                }
            }
            frame(s).await;
        }
    })
    .await
    .expect("a line arrives")
}

fn refused(r: Result<Session, JoinError>) -> Option<u8> {
    match r {
        Err(JoinError::Refused(code)) => Some(code),
        _ => None,
    }
}

/// **There and back.** The arena admits the island's handoff with no
/// signature, once; `/leave` there sends the player back on a handoff the
/// island admits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_player_goes_to_the_arena_and_back_with_no_signature() {
    let island = shard(ISLAND, 20_261_027).await;
    let arena = shard(ARENA, 20_261_028).await;
    let (ie, ae) = (endpoint(&island), endpoint(&arena));
    let me = wallet(&key(3));

    let go = travel(ISLAND, &me, ARENA);
    let s = Session::connect_handoff(&ae, &arena.local_addr.to_string(), &go)
        .await
        .expect("the arena admits the island's handoff, and refuses guests");
    let mut s = placed(s).await;

    assert_eq!(
        refused(Session::connect_handoff(&ae, &arena.local_addr.to_string(), &go).await),
        Some(protocol::REFUSE_AUTH),
        "a handoff is good once"
    );

    type_line(&s, "/leave");
    let back = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(t) = s.take_travel() {
                return t;
            }
            frame(&mut s).await;
        }
    })
    .await
    .expect("the arena sends the player back");
    assert_eq!(back.addr(), ISLAND, "back to where the handoff came from");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !s.closed() {
            frame(&mut s).await;
        }
    })
    .await
    .expect("the arena closes the session it sent away");

    let home = Session::connect_handoff(&ie, &island.local_addr.to_string(), &back)
        .await
        .expect("the island admits the arena's handoff");
    let _home = placed(home).await;

    for h in [&island, &arena] {
        h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A handoff for another shard, or signed by a key nobody named, admits
/// nobody.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handoff_for_elsewhere_or_from_a_stranger_is_refused() {
    let arena = shard(ARENA, 20_261_029).await;
    let ae = endpoint(&arena);
    let me = wallet(&key(4));

    let elsewhere = travel(ISLAND, &me, "other.test:7200");
    assert_eq!(
        refused(Session::connect_handoff(&ae, &arena.local_addr.to_string(), &elsewhere).await),
        Some(protocol::REFUSE_AUTH)
    );

    let mut stranger = handoff_cfg(ISLAND);
    stranger.signer = Some(("island-1".into(), seed(0x33)));
    let tok = handoff::issue(&stranger, &me, ARENA, b"", now()).unwrap();
    let forged =
        protocol::Travel::new(ARENA, protocol::Handoff::new(tok.as_bytes(), b"").unwrap()).unwrap();
    assert_eq!(
        refused(Session::connect_handoff(&ae, &arena.local_addr.to_string(), &forged).await),
        Some(protocol::REFUSE_AUTH)
    );
    arena
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// `/leave` with nowhere to go back to, and `/arena` outside town, are told
/// why and stay put.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_doors_say_why_when_they_do_not_open() {
    let island = shard(ISLAND, 20_261_030).await;
    let arena = shard(ARENA, 20_261_031).await;
    let (ie, ae) = (endpoint(&island), endpoint(&arena));

    let direct = Session::connect_agent(
        &ae,
        &arena.local_addr.to_string(),
        &key(5),
        protocol::Name::new("direct").unwrap(),
    )
    .await
    .expect("joins");
    let mut direct = placed(direct).await;
    type_line(&direct, "/leave");
    let line = hear(&mut direct).await;
    assert!(line.contains("came straight here"), "{line}");
    assert!(!direct.closed());

    let walker = Session::connect_agent(
        &ie,
        &island.local_addr.to_string(),
        &key(6),
        protocol::Name::new("walker").unwrap(),
    )
    .await
    .expect("joins");
    let mut walker = placed(walker).await;
    type_line(&walker, "/arena");
    let line = hear(&mut walker).await;
    assert!(line.contains("in town"), "{line}");
    assert!(!walker.closed());

    for h in [&island, &arena] {
        h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
