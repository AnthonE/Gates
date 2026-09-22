//! **The jev bot through its real door** (`NETCODE.md` §2.3, §2.4): the same
//! `agent_demo::Door` `jev-bot` builds from its flags, a real shard, real
//! WebTransport, and a survivor playing with the explicit scripted mind.
//!
//! 1. On a spectate-enabled local shard (`agent_demo::spawn_local`, what
//!    `jev-bot --local` boots) the guest bot is a watchable agent: a seat
//!    asking for "any agent" is admitted, is told it watches `jev`, is
//!    welcomed as the bot's body, and sees that body move.
//! 2. A wallet bot is followed by its own address.
//! 3. On a `require_auth` shard, the signed path — a throwaway key written to
//!    a `0600` file and loaded by path, as `--agent-key` does — is admitted
//!    and plays; the guest path is refused as a guest.
//!
//! Asserts are on observable state; every wait is a bounded poll on a fact,
//! with a deadline only as a hang guard.

use server::agent_demo::{bot_name, Door};
use server::botclient::BotReport;
use server::explorer::Survivor;
use server::mind::{Mind, MindConfig, Scripted};
use server::stats::ShardStats;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use client::Session;

fn survivor() -> Survivor {
    Survivor::new(Mind::new(Scripted::default(), MindConfig::default()).expect("a mind"))
}

/// A throwaway secret, built here and written nowhere but a temp file.
fn secret(byte: u8) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[31] = byte;
    b[3] = 0x6D;
    b
}

/// The key file `--agent-key` names: 64 hex digits, mode 0600.
fn key_file(byte: u8) -> PathBuf {
    let path = std::env::temp_dir().join(format!("jev-door-key-{}-{byte}", std::process::id()));
    let hex: String = secret(byte).iter().map(|b| format!("{b:02x}")).collect();
    let mut f = std::fs::File::create(&path).expect("temp key file");
    f.write_all(hex.as_bytes()).expect("written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("0600");
    }
    path
}

/// Play `seconds` through `door` on a task; the survivor comes back with the
/// report, so its own counters can be read after.
fn play(
    door: std::sync::Arc<Door>,
    name: &str,
    seconds: u64,
) -> tokio::task::JoinHandle<(Survivor, Result<BotReport, String>)> {
    let name = bot_name(name, 0, 1).expect("a name");
    tokio::spawn(async move {
        let ep = door.endpoint().expect("endpoint");
        let mut bot = survivor();
        let result = door
            .play(&ep, name, Duration::from_secs(seconds), &mut bot)
            .await;
        (bot, result)
    })
}

/// Take a seat watching `target`, retrying until the bot has joined.
async fn seat(server: &str, hash: &str, target: protocol::Address) -> Session {
    let ep = client::client_endpoint(server, Some(hash)).expect("endpoint");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(s) = Session::watch(&ep, server, target).await {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("a seat on the bot inside 20 s")
}

/// Pump the seat until the watched body has moved `metres` from where the
/// seat first saw it.
async fn sees_it_move(watcher: &mut Session, id: u32, metres: f32) {
    let q = sim_core::movement::POS_XZ_Q;
    let mut first: Option<(i32, i32)> = None;
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            watcher.pump(1000.0 / 60.0);
            if let Some(e) = watcher.core.view.get(id) {
                let at = (e.qx, e.qz);
                let (x0, z0) = *first.get_or_insert(at);
                let moved = ((at.0 - x0) as f32 * q).hypot((at.1 - z0) as f32 * q);
                if moved >= metres {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    })
    .await
    .expect("the seat saw the bot's body move");
}

/// **Claim 1.** The guest bot on the shard `jev-bot --local` boots is a
/// watchable agent, found by "any agent", and followed as it plays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spectator_follows_the_local_guest_bot() {
    let shard = server::agent_demo::spawn_local().await.expect("boots");
    let server = shard.local_addr.to_string();
    let door = Door::new(server.clone(), Some(shard.cert_hash.clone()), None).expect("a door");
    assert_eq!(door.watch_target(), "any");
    assert!(
        door.spectate_query().ends_with("&spectate"),
        "bare: no address to name"
    );
    let bot = play(std::sync::Arc::new(door), "jev", 25);

    let mut watcher = seat(&server, &shard.cert_hash, protocol::Address::GUEST).await;
    let w = watcher.watching.expect("told whose view this is");
    assert!(w.agent, "declared an agent");
    assert!(w.address.is_guest(), "a guest agent has no proven address");
    assert_eq!(w.name.as_str(), "jev");
    assert!(watcher.core.spectating());
    let id = watcher.welcome.player_id;
    sees_it_move(&mut watcher, id, 2.0).await;

    let (bot, result) = bot.await.expect("the bot task");
    let report = result.expect("the bot played");
    assert_eq!(
        report.player_id, id,
        "the seat was welcomed as the bot's body"
    );
    assert!(bot.mind.stats.decisions > 0);
    assert_eq!(report.decode_errors + report.event_decode_errors, 0);
    assert!(
        ShardStats::get(&shard.stats.spectate_joins) > 0,
        "the shard seated the watcher"
    );
    shard.shutdown.store(true, Ordering::Relaxed);
}

/// **Claim 2.** A wallet bot proves its address, so a viewer can name it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spectator_names_a_wallet_bot_by_its_address() {
    let shard = server::agent_demo::spawn_local().await.expect("boots");
    let server = shard.local_addr.to_string();
    let path = key_file(21);
    let key = Door::load_key(&path).expect("the 0600 key loads");
    let address = key.address();
    let door = Door::new(server.clone(), Some(shard.cert_hash.clone()), Some(key)).expect("door");
    assert!(door.spectate_query().contains("&spectate=0x"));
    let bot = play(std::sync::Arc::new(door), "jev", 20);

    let mut watcher = seat(&server, &shard.cert_hash, address).await;
    let w = watcher.watching.expect("told whose view this is");
    assert_eq!(w.address, address, "the proven wallet");
    assert!(w.agent);
    assert_eq!(w.name.as_str(), "jev");
    let id = watcher.welcome.player_id;
    sees_it_move(&mut watcher, id, 2.0).await;
    let (_, result) = bot.await.expect("the bot task");
    assert_eq!(result.expect("played").player_id, id);
    let _ = std::fs::remove_file(path);
    shard.shutdown.store(true, Ordering::Relaxed);
}

/// A stub of elo's ticket route that entitles exactly `owner`.
fn ticket_origin(owner: String) -> String {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let who = req
                .split_whitespace()
                .nth(1)
                .and_then(|p| p.rsplit('/').next())
                .unwrap_or("");
            let body = if who == owner {
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

/// **Claim 3.** `require_auth` admits jev-bot's signed path and refuses its
/// guest path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn require_auth_admits_the_signed_bot_and_refuses_the_guest() {
    let path = key_file(22);
    let key = Door::load_key(&path).expect("the 0600 key loads");
    let wallet = String::from_utf8(key.address().to_hex().to_vec()).expect("ascii");
    let mut cfg = server::config::ShardConfig::ephemeral(20_260_930);
    cfg.require_auth = true;
    cfg.entitle = server::entitle::Config {
        origin: Some(ticket_origin(wallet)),
        ..server::entitle::Config::off()
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("content");
    let shard = server::net::spawn_shard(
        cfg,
        server::net::bake_all(&content).expect("bakes"),
        server::store::Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let server = shard.local_addr.to_string();

    let signed = Door::new(server.clone(), Some(shard.cert_hash.clone()), Some(key)).expect("door");
    let (bot, result) = play(std::sync::Arc::new(signed), "jev", 3)
        .await
        .expect("the bot task");
    let report = result.expect("a signed, entitled bot is admitted");
    assert!(report.snapshots_applied > 0, "and it played");
    assert!(bot.core().is_some(), "welcomed");

    let guest = Door::new(server, Some(shard.cert_hash.clone()), None).expect("door");
    let (_, refused) = play(std::sync::Arc::new(guest), "jev", 3)
        .await
        .expect("the bot task");
    let err = refused.expect_err("a guest is refused where identity is required");
    let auth = protocol::refuse_text(protocol::REFUSE_AUTH).expect("a sentence");
    assert!(err.contains(auth), "{err}");
    assert_eq!(ShardStats::get(&shard.stats.refused_auth), 1);
    assert_eq!(ShardStats::get(&shard.stats.refused_ticket), 0);
    let _ = std::fs::remove_file(path);
    shard.shutdown.store(true, Ordering::Relaxed);
}

/// The key file's rule is the crate's: a key others can read is refused by
/// path, before anything is dialled.
#[cfg(unix)]
#[test]
fn an_open_key_file_is_refused_before_anything_dials() {
    use std::os::unix::fs::PermissionsExt;
    let path = key_file(23);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let Err(why) = Door::load_key(&path) else {
        panic!("a world-readable key must not load");
    };
    assert!(why.contains("chmod 600"), "{why}");
    let _ = std::fs::remove_file(path);
}
