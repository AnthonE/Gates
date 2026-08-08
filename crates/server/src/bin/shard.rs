//! The shard server: `cargo run -p server --bin shard` (reads shard.toml,
//! CLAUDE.md commands). Prints the bound address and the dev-cert hash
//! (WebTransport `serverCertificateHashes` format), then a stats line every
//! 10 s. Runs until killed.

use server::config::parse_shard_toml;
use server::net::spawn_shard;
use server::stats::ShardStats;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};

/// How long a graceful shutdown will wait for the storage thread before it
/// gives up and says what it lost. A backstop, not the mechanism — the real
/// exit is `store_stopped` being raised, which is exact.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shard.toml".into());
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("shard: cannot read {path}: {e}");
            eprintln!(
                "shard: shard.toml is per-deployment (gitignored); start from shard.toml.example"
            );
            std::process::exit(1);
        }
    };
    let cfg = match parse_shard_toml(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    // Content validates at boot (CLAUDE.md wall 7): a set that fails
    // schema, reference, or balance-band checks does not get a shard. The
    // hash pins into the WAL header when the WAL file format lands.
    let content = match content::Content::load_dir(std::path::Path::new(&cfg.content_dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content refused: {e}");
            std::process::exit(1);
        }
    };
    let gather = match content.bake_gather() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let craft = match content.bake_craft() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let deploy = match content.bake_deployables() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let build = match content.bake_building() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let combat = match content.bake_combat() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let backpack = match content.bake_backpack() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let survival = match content.bake_survival() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let spawn_kit = match content.bake_spawn_kit() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let loot = match content.bake_loot() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let mobs = match content.bake_mobs() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let catalog = match server::net::bake_catalog(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    let a = content.anchors();
    println!(
        "content ok: {} items · hash {:016x}",
        content.items.len(),
        content.hash()
    );
    println!(
        "content anchors: raid w/s/m {:.2}/{:.2}/{:.2} · satchel {:.1} min · starter {:.1} min · upkeep {:.1} min/day",
        a.raid_ratio[0],
        a.raid_ratio[1],
        a.raid_ratio[2],
        a.satchel_minutes,
        a.starter_minutes,
        a.upkeep_daily_minutes
    );
    println!(
        "content anchors: melee breach — door {} swings · wall w/s/m {}/{}/{}",
        a.door_breach_swings,
        a.wall_breach_swings[0],
        a.wall_breach_swings[1],
        a.wall_breach_swings[2]
    );
    let seed = cfg.seed;
    // The island gets the same boot treatment content does. `spawn_shard`
    // refuses a short one on every path; this call is here for the *counter*,
    // which is the half that has something to say on a seed that fills — the
    // shard printed its content hash and its balance anchors and then went
    // quiet about how much island it had built.
    match server::boot::check_seed(seed) {
        Ok(live) => println!(
            "island ok: {live}/{} authored sites (haven pad + {} waystations)",
            server::boot::AUTHORED_SITES,
            sim_core::terrain::WAYSTATIONS
        ),
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    }
    // The player store, opened and validated before a port is bound — the
    // posture content and the island already take (CLAUDE.md wall 7). It needs
    // the content hash, which is why it is opened HERE and handed over rather
    // than inside `spawn_shard`: a save file describes inventories as item
    // *indices*, so restoring it under moved content would hand players the
    // wrong things, and that has to be a refusal rather than a surprise.
    let saves = match cfg.save_file.as_deref() {
        None => {
            println!("saves off: no save_file — every join builds a fresh character");
            server::store::Saves::off()
        }
        Some(path) => match server::store::open(Path::new(path), cfg.seed, content.hash()) {
            Ok((saves, found)) => {
                if found.created {
                    println!("saves ok: created {path} — remembering nobody yet");
                } else {
                    // The backup depth is named at boot because it is the thing
                    // an operator needs to know BEFORE the save goes bad, not
                    // after: recovery is copying `<file>.1` over `<file>`, and
                    // `.2` exists because `.1` can share the corruption
                    // (reference/SAVES.md §6).
                    println!(
                        "saves ok: {} players remembered from {path} · {} backup(s) rotated \
                         ({path}.1 is the previous run)",
                        found.live,
                        server::store::SAVE_BACKUP_COUNT
                    );
                }
                if found.corrupt > 0 {
                    // Its own line, and said out loud rather than folded into
                    // the count: a corrupt record is somebody's base, and the
                    // operator is the only one who can judge whether N of them
                    // is one torn write or a disk going bad.
                    println!(
                        "saves WARNING: {} record(s) REFUSED as corrupt — that many players \
                         start over. One is a torn write (the shard was killed mid-save); \
                         many is the disk.",
                        found.corrupt
                    );
                }
                if !cfg.require_auth {
                    // The trap this warns about is silent: saves armed,
                    // admission open, so every joiner is a guest with no key
                    // to be filed under and the store stays empty forever
                    // while every gate is green.
                    println!(
                        "saves WARNING: require_auth is false, so joiners have no identity \
                         to save under — a guest is admitted and remembered by nobody, and \
                         this shard will write nothing to that file"
                    );
                }
                saves
            }
            Err(e) => {
                eprintln!("shard: {e}");
                std::process::exit(1);
            }
        },
    };
    // The world file, opened and validated the same way and for the same
    // reasons — before a port is bound, against this seed and this content
    // hash. It needs the baked tables as well as the hash, because
    // validating a world means *loading* one: a piece names a content row
    // and the decoder range-checks it, so a trial world is built here and
    // thrown away, and the bytes are handed to the sim thread to load into
    // the world it actually runs (`worldfile::WorldBoot` says why twice is
    // right).
    let world_boot = match cfg.world_file.as_deref() {
        None => {
            println!("world off: no world_file — the island is generated fresh every boot");
            server::worldfile::WorldBoot::off()
        }
        Some(path) => {
            let mut trial = sim_core::world::World::new(cfg.seed);
            trial.gather = gather;
            trial.craft = craft;
            trial.build = build;
            trial.deploy = deploy;
            trial.combat = combat;
            trial.backpack = backpack;
            trial.survival = survival;
            trial.loot = loot;
            match server::worldfile::open(
                Path::new(path),
                &mut trial,
                cfg.seed,
                content.hash(),
                cfg.world_save_interval_ticks,
            ) {
                Ok((boot, found)) => {
                    if found.created {
                        println!(
                            "world ok: will create {path} — a fresh island, saved every {} ticks",
                            cfg.world_save_interval_ticks
                        );
                    } else {
                        println!(
                            "world ok: resumed {path} at tick {} · {} bodies \
                             ({} claimable) · {} backup(s) rotated ({path}.1 \
                             is the previous run)",
                            found.tick,
                            found.bodies,
                            found.claimable,
                            server::store::SAVE_BACKUP_COUNT
                        );
                        if found.bodies > found.claimable {
                            // Said out loud: a body nobody can claim is
                            // somebody's base standing there as free loot,
                            // and the operator is the only one who can tell
                            // whether that is one stale identity or the key
                            // table having been lost.
                            println!(
                                "world WARNING: {} of {} bodies have no identity beside them — \
                                 those players cannot walk back into their own body and will \
                                 come back through the player store instead",
                                found.bodies - found.claimable,
                                found.bodies
                            );
                        }
                    }
                    boot
                }
                Err(e) => {
                    eprintln!("shard: {e}");
                    std::process::exit(1);
                }
            }
        }
    };
    // The `AUTH WARNING` that used to print here is **deleted, not moved**.
    // It existed because `validate_session` was a stub that accepted any
    // non-empty token and filed the save under the token's own bytes, so
    // `require_auth = true` proved a joiner carried *something*. It now
    // proves they hold the private key behind the address they claim
    // (`auth.rs`), which is the thing the warning was waiting for.
    let handle = match spawn_shard(
        cfg, gather, craft, build, deploy, combat, backpack, survival, spawn_kit, loot, mobs,
        catalog, saves, world_boot,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    println!("shard up on {} (seed {seed})", handle.local_addr);
    println!("dev cert sha256 {}", handle.cert_hash);

    // **The thing that turns a `systemctl stop` into a save.**
    //
    // Everything about the graceful shutdown existed before this and none
    // of it had ever run in production, because nothing set the flag: the
    // default SIGTERM handler kills the process where it stands, so a
    // deploy cost every player up to an autosave sweep and everybody's base
    // up to a whole save interval. The flush was real and unreachable.
    //
    // Both signals, because both are how a shard actually goes down: SIGINT
    // is an operator with a terminal, SIGTERM is systemd, docker stop, and
    // every process supervisor there is.
    let mut sigterm = signal(SignalKind::terminate())
        .map_err(|e| format!("SIGTERM handler: {e}"))
        .unwrap_or_else(|e| {
            eprintln!("shard: {e}");
            std::process::exit(1);
        });

    let mut report = tokio::time::interval(Duration::from_secs(10));
    report.tick().await; // immediate first tick consumed
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                shutdown(&handle, "SIGINT").await;
                return;
            }
            _ = sigterm.recv() => {
                shutdown(&handle, "SIGTERM").await;
                return;
            }
            _ = report.tick() => {}
        }
        let s = &handle.stats;
        println!(
            "tick {} · joins {} leaves {} · in ok/bad/drop {}/{}/{} · snap sent/skip/err {}/{}/{} · \
             refused v/full {}/{} · dropped-ticks {} · saves restored/written/lost {}/{}/{} · \
             sleepers {} (took over {}, evicted {}) · worlds written/skipped/failed {}/{}/{}",
            ShardStats::get(&s.current_tick),
            ShardStats::get(&s.joins),
            ShardStats::get(&s.leaves),
            ShardStats::get(&s.input_dg_ok),
            ShardStats::get(&s.input_dg_bad),
            ShardStats::get(&s.input_ring_drops),
            ShardStats::get(&s.snap_sent),
            ShardStats::get(&s.snap_ring_skips),
            ShardStats::get(&s.snap_send_errors),
            ShardStats::get(&s.refused_version),
            ShardStats::get(&s.refused_full),
            ShardStats::get(&s.ticks_dropped),
            ShardStats::get(&s.saves_restored),
            ShardStats::get(&s.saves_written),
            // The three failures that mean a player lost something, summed:
            // a full ring, an evicted table slot, a disk that refused.
            ShardStats::get(&s.save_ring_drops)
                + ShardStats::get(&s.saves_evicted)
                + ShardStats::get(&s.save_write_errors),
            ShardStats::get(&s.sleepers),
            ShardStats::get(&s.takeovers),
            ShardStats::get(&s.sleepers_evicted),
            ShardStats::get(&s.world_saves_written),
            ShardStats::get(&s.world_saves_skipped),
            ShardStats::get(&s.world_save_errors),
        );
    }
}

/// Stop the shard and **wait for the disk**, then report what was saved.
///
/// The waiting is the point. Setting the flag and calling `exit` would
/// return the same instant the signal arrived and kill the process out from
/// under the flush — which is exactly the bug this whole path exists to
/// fix, moved one function later.
///
/// It waits on `store_stopped`, which the storage thread raises when its
/// rings are dry *and abandoned*: the sim flushes and drops its producers,
/// the accept loop drains until abandoned and drops its own, the storage
/// thread drains until abandoned and raises the flag. Every hop waits on a
/// producer being dropped, so this ends exactly when the last byte is
/// written — not after a duration somebody guessed.
///
/// `SHUTDOWN_WAIT` is a backstop for a wedged thread and not the mechanism.
/// A shutdown that hangs forever is a deploy that never completes, which is
/// worse than a shutdown that loses the last minute and says so.
async fn shutdown(handle: &server::net::ShardHandle, signal_name: &str) {
    println!("shard: {signal_name} — flushing the world and every player record");
    handle.shutdown.store(true, Ordering::Relaxed);

    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while !ShardStats::raised(&handle.stats.store_stopped) {
        if Instant::now() >= deadline {
            eprintln!(
                "shard: WARNING — the storage thread did not finish inside {}s. \
                 Up to one save interval of the world and one sweep of each \
                 player may be lost.",
                SHUTDOWN_WAIT.as_secs()
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let s = &handle.stats;
    println!(
        "shard: down · worlds written {} (failed {}) · player records written {} (failed {})",
        ShardStats::get(&s.world_saves_written),
        ShardStats::get(&s.world_save_errors),
        ShardStats::get(&s.saves_written),
        ShardStats::get(&s.save_write_errors),
    );
}
