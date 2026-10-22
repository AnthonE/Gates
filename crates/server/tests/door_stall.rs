//! **One client that never reads cannot shut the door.**
//!
//! The welcome used to be written on the accept loop itself, and `write_all`
//! waits for stream credit only the client grants. A client that shrank its
//! receive window to fit the challenge and nothing more, sent its hello and a
//! guest answer, and never read again held that write — and the whole accept
//! loop behind it — for good: no joins, kicks, bans, save drains or shutdown.
//! The welcome is now written on the connection's own task, bounded; this
//! pins that a stalled joiner and a stalled watcher leave the door open.

use server::config::ShardConfig;
use server::net::{client_handshake, spawn_shard, write_frame};
use server::store::Saves;
use std::sync::Arc;
use std::time::Duration;
use wtransport::{ClientConfig, Endpoint};

use server::botclient::bot_endpoint;

fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&c).expect("shipped content bakes");
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

/// An endpoint whose streams take 64 bytes and no more until the app reads:
/// the challenge fits, the welcome after it does not.
fn starved_endpoint() -> Endpoint<wtransport::endpoint::endpoint_side::Client> {
    let mut cfg = ClientConfig::builder()
        .with_bind_config(wtransport::config::IpBindConfig::InAddrAnyV4)
        .with_no_cert_validation()
        .build();
    let mut transport = wtransport::quinn::TransportConfig::default();
    transport.stream_receive_window(64u32.into());
    cfg.quic_config_mut().transport_config(Arc::new(transport));
    Endpoint::client(cfg).expect("endpoint")
}

/// A session that said its piece and reads nothing more. Both halves are
/// held: dropping the receive half would send STOP_SENDING, which fails the
/// shard's write instead of stalling it.
type Stalled = (
    wtransport::Connection,
    wtransport::SendStream,
    wtransport::RecvStream,
);

/// Opens a stream, says hello and a guest answer, and never reads again.
async fn stall(
    endpoint: &Endpoint<wtransport::endpoint::endpoint_side::Client>,
    addr: std::net::SocketAddr,
    hello: &protocol::Hello,
) -> Stalled {
    let connection = endpoint
        .connect(&format!("https://{addr}"))
        .await
        .expect("connects");
    let (mut send, recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    let len = protocol::encode_hello(hello, &mut buf).expect("hello");
    write_frame(&mut send, &buf[..len])
        .await
        .expect("write hello");
    let len = protocol::encode_auth(&protocol::Auth::default(), &mut buf).expect("auth");
    write_frame(&mut send, &buf[..len])
        .await
        .expect("write auth");
    (connection, send, recv)
}

async fn joins(addr: std::net::SocketAddr) {
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{addr}"))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");
    let welcome = tokio::time::timeout(
        Duration::from_secs(4),
        client_handshake(
            &mut send,
            &mut recv,
            "127.0.0.1",
            protocol::Address::GUEST,
            |_| None,
        ),
    )
    .await
    .expect("the door stayed open behind a client that never reads")
    .expect("an honest guest is admitted");
    assert_ne!(welcome.player_id, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_joiner_that_never_reads_does_not_hold_the_door() {
    let cfg = ShardConfig::ephemeral(20_261_022);
    let handle = spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let starved = starved_endpoint();
    let _held = stall(&starved, handle.local_addr, &protocol::Hello::this_build()).await;
    // Let the door reach the stalled welcome before the honest client knocks.
    tokio::time::sleep(Duration::from_millis(300)).await;
    joins(handle.local_addr).await;
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watcher_that_never_reads_does_not_hold_the_door() {
    let mut cfg = ShardConfig::ephemeral(20_261_022);
    cfg.spectate = server::config::Spectate {
        seats: 4,
        per_target: 2,
        human_delay_s: 0,
    };
    let handle = spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    // A watchable agent to be watched: an honest join that declares itself.
    let endpoint = bot_endpoint().expect("endpoint");
    let agent = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = agent.open_bi().await.expect("open_bi").await.expect("bi");
    let hello = protocol::Hello {
        flags: protocol::HELLO_AGENT,
        name: protocol::Name::new("jev-test").expect("name"),
        ..protocol::Hello::this_build()
    };
    server::net::client_handshake_as(
        &mut send,
        &mut recv,
        "127.0.0.1",
        protocol::Address::GUEST,
        &hello,
        |_, _, _| None,
    )
    .await
    .expect("the agent joins");

    let starved = starved_endpoint();
    let watch = protocol::Hello {
        flags: protocol::HELLO_SPECTATE,
        target: protocol::Address::GUEST,
        ..protocol::Hello::this_build()
    };
    let _held = stall(&starved, handle.local_addr, &watch).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    joins(handle.local_addr).await;
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
