//! **An agent player joins with a wallet, and pays every door a person
//! does** (`NETCODE.md` §2.4).
//!
//! A real shard with `require_auth` on and a ticket door armed against a
//! stub origin that says which wallets own a copy. Throwaway keys, generated
//! here. Both connect paths the agents use are driven: `botclient`'s signed
//! dial (`run_agent_bot`, what `jev-bot` runs) and the client `Session`
//! (`Session::connect_agent`, what `jev-watch` runs). What is proved:
//!
//! 1. a signed, entitled agent is admitted — on both paths;
//! 2. a guest is refused (`REFUSE_AUTH`) — an agent cannot skip the key;
//! 3. an agent whose wallet owns no copy is refused (`REFUSE_TICKET`) —
//!    entitlement is not bypassed for a bot;
//! 4. a key that is not the claimed address's is refused (`REFUSE_AUTH`);
//! 5. the agent endpoint validates certificates off loopback: an unpinned
//!    self-signed shard is refused at the transport, its own pin admits it,
//!    and a wrong pin is refused.

use server::botclient::{agent_endpoint, run_agent_bot, AgentIdentity, BotDriver, BotReport};
use server::config::ShardConfig;
use server::net::{spawn_shard, ShardHandle};
use server::stats::ShardStats;
use server::store::Saves;
use server::view::ClientView;
use sim_core::input::InputFrame;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::time::Duration;

use client::agentkey::AgentKey;
use client::{Join, JoinError, Session};

fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&c).expect("shipped content bakes");
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

/// A throwaway key. Never a real one: the scalar is built here.
fn key(byte: u8) -> AgentKey {
    let mut b = [0u8; 32];
    b[31] = byte;
    b[5] = 0xA9;
    AgentKey::from_secret(&b).expect("a valid scalar")
}

fn wallet(k: &AgentKey) -> String {
    String::from_utf8(k.address().to_hex().to_vec()).expect("ascii")
}

/// A stub of elo's ticket route: `GET /api/ticket/gates/of/<wallet>` answers
/// `{"entitled": true}` for the wallets it was given and `false` for every
/// other — the definite on-chain zero, the only answer that refuses.
fn ticket_origin(owners: Vec<String>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("");
            let who = path.rsplit('/').next().unwrap_or("");
            let body = if owners.iter().any(|w| w == who) {
                "{\"entitled\": true}"
            } else {
                "{\"entitled\": false}"
            };
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    format!("http://{addr}")
}

async fn armed_shard(owners: Vec<String>) -> ShardHandle {
    let mut cfg = ShardConfig::ephemeral(20_260_922);
    cfg.require_auth = true;
    cfg.entitle = server::entitle::Config {
        origin: Some(ticket_origin(owners)),
        ..server::entitle::Config::off()
    };
    spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

/// Stands still. The point is the door, not the walk.
struct Idle;
impl BotDriver for Idle {
    fn frame(&mut self, _: &ClientView, _: u32, seq: u16) -> InputFrame {
        InputFrame {
            seq,
            pitch: 128,
            ..Default::default()
        }
    }
}

async fn bot(h: &ShardHandle, k: &AgentKey) -> Result<BotReport, String> {
    let server = h.local_addr.to_string();
    let ep = agent_endpoint(&server, None).expect("endpoint");
    let identity = AgentIdentity {
        key: k,
        name: protocol::Name::new("agent").unwrap(),
        watchable: true,
    };
    tokio::time::timeout(
        Duration::from_secs(20),
        run_agent_bot(
            &ep,
            &server,
            &identity,
            Duration::from_millis(400),
            &mut Idle,
        ),
    )
    .await
    .expect("an answer inside 20 s")
}

fn said(err: &str, code: u8) -> bool {
    err.contains(protocol::refuse_text(code).expect("a sentence"))
}

/// **Claim 1, both paths.** A signed, entitled agent is admitted by a shard
/// that refuses guests and checks copies.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_signed_entitled_agent_is_admitted_on_both_paths() {
    let owner = key(1);
    let h = armed_shard(vec![wallet(&owner)]).await;
    // botclient's signed dial — what `jev-bot` runs.
    let report = bot(&h, &owner).await.expect("admitted");
    assert!(report.welcome.is_some(), "welcomed");
    assert!(
        report.snapshots_applied > 0,
        "and played: snapshots arrived"
    );
    // The client Session — what `jev-watch` runs.
    let server = h.local_addr.to_string();
    let ep = client::client_endpoint(&server, None).expect("endpoint");
    let s = Session::connect_agent(&ep, &server, &owner, protocol::Name::new("agent").unwrap())
        .await
        .expect("admitted");
    assert_ne!(s.welcome.player_id, 0);
    assert_eq!(ShardStats::get(&h.stats.refused_auth), 0);
    assert_eq!(ShardStats::get(&h.stats.refused_ticket), 0);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Claim 2.** A guest is refused — an agent without its key is a guest,
/// and a shard that requires identity takes no guest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_is_refused_where_identity_is_required() {
    let h = armed_shard(vec![]).await;
    let server = h.local_addr.to_string();
    let ep = client::client_endpoint(&server, None).expect("endpoint");
    let r = Session::connect_as(
        &ep,
        &server,
        protocol::Address::GUEST,
        &Join::Agent {
            name: protocol::Name::new("keyless").unwrap(),
        },
        |_, _, _| None,
    )
    .await;
    assert!(
        matches!(r, Err(JoinError::Refused(c)) if c == protocol::REFUSE_AUTH),
        "a keyless agent must be refused as a guest"
    );
    assert_eq!(ShardStats::get(&h.stats.refused_auth), 1);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Claim 3.** An agent is entitled like any player: a proven wallet that
/// owns no copy is refused, on both paths, with the ticket's own code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unentitled_agent_is_refused_on_both_paths() {
    let owner = key(2);
    let stranger = key(3);
    let h = armed_shard(vec![wallet(&owner)]).await;
    let err = bot(&h, &stranger).await.expect_err("refused");
    assert!(said(&err, protocol::REFUSE_TICKET), "{err}");
    let server = h.local_addr.to_string();
    let ep = client::client_endpoint(&server, None).expect("endpoint");
    let r =
        Session::connect_agent(&ep, &server, &stranger, protocol::Name::new("x").unwrap()).await;
    assert!(
        matches!(r, Err(JoinError::Refused(c)) if c == protocol::REFUSE_TICKET),
        "an agent wallet with no copy must be refused"
    );
    assert_eq!(ShardStats::get(&h.stats.refused_ticket), 2);
    assert_eq!(
        ShardStats::get(&h.stats.refused_auth),
        0,
        "it proved who it was"
    );
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Claim 4.** A key that is not the claimed address's: the entitled
/// owner's address, signed by a different key. The signature is valid — for
/// somebody else — and the shard refuses it as an auth failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_that_is_not_the_claimed_addresss_is_refused() {
    let owner = key(4);
    let impostor = key(5);
    let h = armed_shard(vec![wallet(&owner)]).await;
    let server = h.local_addr.to_string();
    let ep = client::client_endpoint(&server, None).expect("endpoint");
    let r = Session::connect_as(
        &ep,
        &server,
        owner.address(),
        &Join::Agent {
            name: protocol::Name::new("impostor").unwrap(),
        },
        |d, n, i| impostor.sign_proof_hex(d, n, i),
    )
    .await;
    assert!(
        matches!(r, Err(JoinError::Refused(c)) if c == protocol::REFUSE_AUTH),
        "a signature by another key must not prove the owner's address"
    );
    assert_eq!(ShardStats::get(&h.stats.refused_auth), 1);
    assert_eq!(ShardStats::get(&h.stats.refused_ticket), 0);
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// This box's own non-loopback IPv4 address (`tls_posture.rs`'s probe: a
/// connected UDP socket sends nothing, and the kernel picks the source).
/// A panic, never a skip — see that file for why.
fn non_loopback_v4() -> Ipv4Addr {
    for probe in ["203.0.113.1:9", "198.51.100.1:9"] {
        let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if sock.connect(probe).is_err() {
            continue;
        }
        if let Ok(SocketAddr::V4(local)) = sock.local_addr() {
            if !local.ip().is_loopback() && !local.ip().is_unspecified() {
                return *local.ip();
            }
        }
    }
    panic!("agent_join: no non-loopback IPv4 address on this box to prove the refusal on");
}

/// **Claim 5.** The agent endpoint asks who the far end is. Off loopback, an
/// unpinned self-signed shard is refused before a byte of handshake; its own
/// pin admits it; a pin for another certificate is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_agent_endpoint_validates_certificates_off_loopback() {
    let ip = non_loopback_v4();
    let mut cfg = ShardConfig::ephemeral(20_260_928);
    cfg.bind = SocketAddr::new(IpAddr::V4(ip), 0);
    let h = spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let server = format!("{}:{}", ip, h.local_addr.port());
    let url = format!("https://{server}");
    let dial = |pin: Option<String>| {
        let server = server.clone();
        let url = url.clone();
        async move {
            let ep = agent_endpoint(&server, pin.as_deref()).expect("endpoint");
            tokio::time::timeout(Duration::from_secs(10), ep.connect(&url))
                .await
                .expect("a connect answer inside 10 s")
                .map(|_| ())
        }
    };
    assert!(
        dial(None).await.is_err(),
        "an unpinned self-signed certificate must not be trusted off loopback"
    );
    dial(Some(h.cert_hash.clone()))
        .await
        .expect("the shard's own pin admits it");
    let wrong = (0..32).map(|_| "cd").collect::<Vec<_>>().join(":");
    assert!(
        dial(Some(wrong)).await.is_err(),
        "a pin for another certificate must be refused"
    );
    h.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
}
