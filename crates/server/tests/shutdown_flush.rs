//! **A SIGTERM is a save** (`NOW.md` §0y item 4): the shard binary, a real
//! signal, and the files it leaves behind.
//!
//! The flush crosses three threads and a process boundary: `bin/shard.rs`
//! turns the signal into the shutdown flag, the sim thread saves the world
//! while everyone is still connected and then every player's record, the
//! accept loop files those records until the sim lets go of its ring, and
//! the storage thread writes both until both rings are abandoned and raises
//! `store_stopped`, which the binary waits on before it exits. Pieces are
//! gated alone — the flush order on `ShardCore` (`world_persist.rs`), the
//! accept loop's drain and `KeySlot`'s id match (`net.rs`) — and this is the
//! one test that runs them as one path, against the files a restart reads.
//!
//! So this drives the real binary rather than `spawn_shard`: the signal
//! handler and the wait on the store are in `main`, and a test that set the
//! flag itself would skip exactly the glue that was missing the first time
//! (`bin/shard.rs` on why a SIGTERM used to cost a save interval). Unix,
//! because that is where the signal is SIGTERM.
//!
//! What a SIGKILL leaves is gated in `tests/world_persist.rs`
//! (`a_temp_file_a_kill_left_behind_is_neither_read_nor_kept`): no test can
//! time a kill inside a write, so the property gated there is that what a
//! kill leaves costs nothing.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use client::Session;

/// The seed `shard.toml.example` ships, so the island is one a shard boots.
const SEED: u64 = 20260731;

/// The shard process, killed outright if the test leaves early — a failed
/// assert must not leave a server running on the box.
struct Shard(Child);

impl Drop for Shard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A throwaway key, generated here and never anywhere else.
fn key() -> client::agentkey::AgentKey {
    let mut b = [0u8; 32];
    b[31] = 0x5E;
    b[3] = 0x77;
    client::agentkey::AgentKey::from_secret(&b).expect("a valid scalar")
}

/// Every `.tmp` under `dir`, the trust log's directory included.
fn temp_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("scratch dir reads").flatten() {
        let p = entry.path();
        if p.is_dir() {
            temp_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "tmp") {
            out.push(p);
        }
    }
}

/// The next line either stream printed, or a panic naming everything printed
/// so far.
async fn line(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    seen: &mut Vec<String>,
    deadline: Instant,
) -> String {
    let left = deadline.saturating_duration_since(Instant::now());
    match tokio::time::timeout(left, rx.recv()).await {
        Ok(Some(l)) => {
            seen.push(l.clone());
            l
        }
        _ => panic!("the shard went quiet; it printed:\n{}", seen.join("\n")),
    }
}

/// **SIGTERM flushes the world and the player, and nothing leaves a `.tmp`.**
/// A keyed agent joins a shard that keeps both files and is standing in the
/// world when the signal lands. The binary must exit cleanly, having written
/// a world whose one body is claimable by that agent's key and a player
/// record filed under it — the two halves of walking back in after a deploy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sigterm_writes_the_world_and_every_player_and_leaves_no_temp_file() {
    let dir = std::env::temp_dir().join(format!("gates-shutdown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (save, world) = (dir.join("save.bin"), dir.join("world.bin"));
    let toml = dir.join("shard.toml");
    std::fs::write(
        &toml,
        format!(
            "bind = \"127.0.0.1:0\"\nseed = {SEED}\ncontent_dir = \"{}\"\n\
             save_file = \"{}\"\nworld_file = \"{}\"\n",
            root.join("content").display(),
            save.display(),
            world.display()
        ),
    )
    .expect("shard.toml");

    let mut child = Command::new(env!("CARGO_BIN_EXE_shard"))
        .arg(&toml)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shard binary starts");
    let pid = child.id();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let out = child.stdout.take().expect("piped");
    let err = child.stderr.take().expect("piped");
    let mut shard = Shard(child);
    for stream in [
        Box::new(out) as Box<dyn std::io::Read + Send>,
        Box::new(err),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(stream).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        });
    }
    drop(tx);

    // The bound port and the dev certificate's hash, off the boot banner.
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    let (mut addr, mut hash) = (None, None);
    while addr.is_none() || hash.is_none() {
        let l = line(&mut rx, &mut seen, deadline).await;
        if let Some(rest) = l.strip_prefix("shard up on ") {
            addr = rest.split_whitespace().next().map(str::to_owned);
        } else if let Some(rest) = l.strip_prefix("dev cert sha256 ") {
            hash = rest.split_whitespace().next().map(str::to_owned);
        }
    }
    let (addr, hash) = (addr.unwrap(), hash.unwrap());

    // A keyed agent, placed in the world. Its first snapshot is the proof the
    // sim holds its body, and it takes far longer than the binary's next step
    // after the banner, which is installing the SIGTERM handler.
    let k = key();
    let ep = client::client_endpoint(&addr, Some(&hash)).expect("endpoint");
    let mut agent = Session::connect_agent(
        &ep,
        &addr,
        &k,
        protocol::Name::new("sleeper").expect("a name"),
    )
    .await
    .expect("admitted");
    tokio::time::timeout(Duration::from_secs(30), async {
        while !(agent.core.predict.started && agent.core.hp > 0) {
            agent.pump(1000.0 / 60.0);
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    })
    .await
    .expect("the agent is placed and alive");

    let sent = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("kill runs");
    assert!(sent.success(), "SIGTERM was not delivered");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = shard.0.try_wait().expect("waits") {
            break s;
        }
        assert!(
            Instant::now() < deadline,
            "the shard did not exit on SIGTERM"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    drop(agent);
    // Both pipes close at exit, so this ends.
    while let Some(l) = rx.recv().await {
        seen.push(l);
    }
    let log = seen.join("\n");
    assert!(
        status.success(),
        "the shard died on the signal instead of flushing ({status}):\n{log}"
    );
    assert!(
        seen.iter()
            .any(|l| l == "shard: SIGTERM — flushing the world and every player record"),
        "the signal did not reach the flush:\n{log}"
    );
    // `worlds written N (failed N) · player records written N (failed N)`.
    let down = seen
        .iter()
        .find(|l| l.starts_with("shard: down"))
        .unwrap_or_else(|| panic!("no shutdown report:\n{log}"));
    let n: Vec<u64> = down
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|t| t.parse().ok())
        .collect();
    let [worlds, world_errs, records, record_errs] = n[..] else {
        panic!("the shutdown report changed shape: {down}");
    };
    assert!(worlds >= 1 && records >= 1, "nothing was flushed: {down}");
    assert_eq!((world_errs, record_errs), (0, 0), "{down}");
    assert!(
        !seen.iter().any(|l| l.contains("WARNING")),
        "the flush did not finish clean:\n{log}"
    );

    let mut tmp = Vec::new();
    temp_files(&dir, &mut tmp);
    assert!(tmp.is_empty(), "the shutdown left {tmp:?} behind");

    // What a restart reads: the world with the agent's body in it, claimable
    // by its key, and the agent's own record in the player store.
    let me = server::auth::key_of(&k.address()).expect("a proven address has a key");
    let content = content::Content::load_dir(&root.join("content")).expect("content loads");
    let tables = server::net::bake_all(&content).expect("content bakes");
    let mut trial = sim_core::world::World::new(SEED);
    trial.gather = tables.gather;
    trial.craft = tables.craft;
    trial.build = tables.build;
    trial.deploy = tables.deploy;
    trial.combat = tables.combat;
    trial.backpack = tables.backpack;
    trial.survival = tables.survival;
    trial.cook = tables.cook;
    trial.loot = tables.loot;
    let (boot, found) = server::worldfile::open(
        &world,
        &mut trial,
        SEED,
        content.hash(),
        content.layout_hash(),
        sim_core::probe::probe_terrain(SEED),
        u64::MAX,
    )
    .expect("the flushed world boots");
    assert!(!found.created, "no world was written");
    assert_eq!(
        (found.bodies, found.claimable),
        (1, 1),
        "the agent's body is not in the world under a name"
    );
    assert!(
        boot.idents.iter().any(|(key, _)| *key == me),
        "the body is claimable, but not by the agent's key"
    );
    let (saves, loaded) = server::store::open(
        &save,
        SEED,
        content.hash(),
        content.layout_hash(),
        &tables.gather,
    )
    .expect("the flushed store boots");
    assert_eq!((loaded.live, loaded.corrupt), (1, 0));
    let record = saves
        .store
        .find(&me)
        .expect("the agent's record was flushed");
    assert!(record.hp_max > 0, "the record filed is not a character");

    drop(shard);
    let _ = std::fs::remove_dir_all(&dir);
}
