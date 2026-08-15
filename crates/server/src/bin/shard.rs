//! The shard server: `cargo run -p server --bin shard` (reads shard.toml,
//! CLAUDE.md commands). Prints the bound address and the dev-cert hash
//! (WebTransport `serverCertificateHashes` format), then a stats line every
//! 10 s. Runs until killed.

use server::config::parse_shard_toml;
use server::net::spawn_shard;
use server::population::{Population, PopulationStats};
use server::stats::ShardStats;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// How long a graceful shutdown will wait for the storage thread before it
/// gives up and says what it lost. A backstop, not the mechanism — the real
/// exit is `store_stopped` being raised, which is exact.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(10);

/// **"The supervisor is stopping this service", whatever the OS calls it.**
///
/// Ctrl-C is portable and `tokio::signal::ctrl_c` handles it on both
/// platforms; this is the *other* half, and it is the half that matters in
/// production, because a shard is almost never stopped by somebody typing at
/// it. On Unix that is `SIGTERM` — systemd, `docker stop`, every process
/// supervisor there is. On Windows the same event arrives as a console
/// control: `CTRL_CLOSE_EVENT` when the window is closed and
/// `CTRL_SHUTDOWN_EVENT` when the machine is going down.
///
/// It exists as a type rather than as two `#[cfg]` arms in the select below
/// because the select is the part that must not fork: the shutdown path is
/// the thing that turns a stop into a save, and a second copy of it under a
/// `cfg` is a copy that only one platform ever runs and neither reviews.
/// Here the `cfg` covers *which signal*, and the flush is one code path.
///
/// **Found by typechecking, not by review** — `tokio::signal::unix` was
/// imported unconditionally and the Windows build of this binary could not
/// compile at all, which the three-platform release workflow would have hit
/// on its first run (2026-08-11).
struct Stop {
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(windows)]
    close: tokio::signal::windows::CtrlClose,
    #[cfg(windows)]
    shutdown: tokio::signal::windows::CtrlShutdown,
}

impl Stop {
    fn new() -> Result<Self, String> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Stop {
                sigterm: signal(SignalKind::terminate())
                    .map_err(|e| format!("SIGTERM handler: {e}"))?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Stop {
                close: tokio::signal::windows::ctrl_close()
                    .map_err(|e| format!("CTRL_CLOSE handler: {e}"))?,
                shutdown: tokio::signal::windows::ctrl_shutdown()
                    .map_err(|e| format!("CTRL_SHUTDOWN handler: {e}"))?,
            })
        }
    }

    /// Resolves when the supervisor says stop, naming the signal for the log.
    async fn recv(&mut self) -> &'static str {
        #[cfg(unix)]
        {
            self.sigterm.recv().await;
            "SIGTERM"
        }
        #[cfg(windows)]
        {
            tokio::select! {
                _ = self.close.recv() => "CTRL_CLOSE",
                _ = self.shutdown.recv() => "CTRL_SHUTDOWN",
            }
        }
    }
}

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
    // Every table in one call, refusing the boot on the first that fails.
    // It was twelve separate `match` arms here and one of them was simply
    // never written — see `net::SimTables`.
    let tables = match server::net::bake_all(&content) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    // What this shard IS, before what it loaded. Three numbers and they are
    // three different questions (`protocol::version`): the build is what to
    // quote in a bug report, `proto` is the exact wire gate, and the floor is
    // this operator's own policy about releases. Printed together because the
    // counter that records a `REFUSE_BUILD` is a bare integer — this line is
    // the other half of reading it.
    let (fma, fmi, fpa) = protocol::version::unpack(cfg.min_client);
    println!(
        "shard {} · proto v{} · admits clients {}",
        protocol::version::BUILD_ID,
        protocol::PROTO_VER,
        if cfg.min_client == 0 {
            "of any release".to_string()
        } else {
            format!("{fma}.{fmi}.{fpa} and newer")
        }
    );
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
            trial.gather = tables.gather;
            trial.craft = tables.craft;
            trial.build = tables.build;
            trial.deploy = tables.deploy;
            trial.combat = tables.combat;
            trial.backpack = tables.backpack;
            trial.survival = tables.survival;
            trial.cook = tables.cook;
            trial.loot = tables.loot;
            // The island's own digest, computed once here and carried into
            // every header this process writes. It is the number
            // `test_terrain_golden` pins, so a build whose worldgen moved
            // refuses the file rather than putting bases on ground that
            // changed shape (`worldfile::H_WORLD`).
            let world_digest = sim_core::probe::probe_terrain(cfg.seed);
            match server::worldfile::open(
                Path::new(path),
                &mut trial,
                cfg.seed,
                content.hash(),
                world_digest,
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
    let status_addr = cfg.status_addr;
    // **The shard's own inhabitants** (`population.rs`), resolved before the
    // config moves into `spawn_shard`. The rows come off the content this
    // process already loaded and validated, by id — `bin/bots` has to read a
    // second copy of the tree because it is a separate process, and that is
    // the one thing an in-shard population does not have to do.
    // Cloned because `cfg` moves into `spawn_shard` and the population is
    // seated after the bind — it needs the bound port, which does not exist
    // until that call returns. One clone at boot, never in a loop.
    let boot_cfg = cfg.clone();
    let population = cfg.population;
    let handle = match spawn_shard(cfg, tables, saves, world_boot).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    println!("shard up on {} (seed {seed})", handle.local_addr);
    println!("dev cert sha256 {}", handle.cert_hash);
    // The status endpoint, only where the operator said (config.rs says why
    // absent-serves-nothing is the default). A bind failure refuses the boot
    // rather than running without it: a shard whose config names an endpoint
    // it silently does not serve is the config-says-X-shard-does-Y defect
    // the parser refuses everywhere else. Daemon thread — shutdown neither
    // signals nor joins it, so the flush path above cannot hang on it.
    if let Some(addr) = status_addr {
        match server::status::spawn_status(addr, handle.stats.clone()) {
            Ok(bound) => println!("status on http://{bound}/status.json"),
            Err(e) => {
                eprintln!("shard: status endpoint bind {addr}: {e}");
                std::process::exit(1);
            }
        }
    }

    // **The thing that turns a `systemctl stop` into a save.**
    //
    // Everything about the graceful shutdown existed before this and none
    // of it had ever run in production, because nothing set the flag: the
    // default SIGTERM handler kills the process where it stands, so a
    // deploy cost every player up to an autosave sweep and everybody's base
    // up to a whole save interval. The flush was real and unreachable.
    //
    // Both signals, because both are how a shard actually goes down: SIGINT
    // is an operator with a terminal, and `Stop` is the supervisor — systemd,
    // docker stop, or the Windows console-control equivalents.
    let mut stop = Stop::new().unwrap_or_else(|e| {
        eprintln!("shard: {e}");
        std::process::exit(1);
    });

    // Seated after the bind and after the status endpoint, because a post
    // that dials before the accept loop is up just burns its backoff. The
    // shutdown flag is the shard's own, so `systemctl stop` ends the
    // population and the sim thread with one store.
    let mut pop = match server::population::seat(
        &boot_cfg,
        &content,
        handle.local_addr,
        handle.shutdown.clone(),
    ) {
        Ok(Some(s)) => {
            // Loud downgrade, never silent: a population that walks is
            // scenery, and an operator who asked for opponents has to be
            // told they got bodies instead.
            match &s.no_raid {
                Some(why) => {
                    eprintln!("shard: population {population} walks, it cannot raid — {why}")
                }
                None => println!("population {population} seated on {} — raiding", s.dial),
            }
            Some(s.population)
        }
        Ok(None) => None,
        Err(e) => {
            // A refusal here is the count or the endpoint, both of which the
            // operator can fix and neither of which is worth running a wrong
            // world for.
            eprintln!("shard: population: {e}");
            std::process::exit(1);
        }
    };

    let mut report = tokio::time::interval(Duration::from_secs(10));
    report.tick().await; // immediate first tick consumed
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                shutdown(&handle, "SIGINT").await;
                drain(pop.take()).await;
                return;
            }
            name = stop.recv() => {
                shutdown(&handle, name).await;
                drain(pop.take()).await;
                return;
            }
            _ = report.tick() => {}
        }
        let s = &handle.stats;
        println!(
            // `aoi offered/carried/shed` is why the fill counts at all: the
            // question a 100-bot soak is run to answer — does the linear
            // interest scan need a spatial structure — is `offered / ticks`,
            // and whether the shard is degrading is `shed / offered`. Neither
            // is readable from a shed count alone, and neither is readable
            // from a status endpoint the operator has not turned on, so the
            // three ride the line that always prints.
            "tick {} · joins {} leaves {} · in ok/bad/drop {}/{}/{} · snap sent/skip/err {}/{}/{} · \
             aoi offered/carried/shed {}/{}/{} · \
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
            ShardStats::get(&s.snap_candidates),
            ShardStats::get(&s.snap_entities_sent),
            ShardStats::get(&s.snap_entities_shed),
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
        // Its own line, and only when there is one: the shard's counters
        // cannot tell an inhabitant from a player — which is the point of
        // seating them over the wire — so this is the only place the split
        // is readable. `live` against the configured count is the health
        // number; `errors` climbing is a population fighting its own wire.
        if let Some(p) = &pop {
            let g = &p.stats;
            println!(
                "population {}/{} live · shifts {}/{} started/ended, {} errored · \
                 sent in/act {}/{} · placed pieces/deploys {}/{} · \
                 charges {} · struct hits {}",
                p.live(),
                population,
                PopulationStats::get(&g.shifts_started),
                PopulationStats::get(&g.shifts_ended),
                PopulationStats::get(&g.shift_errors),
                PopulationStats::get(&g.inputs_sent),
                PopulationStats::get(&g.actions_sent),
                PopulationStats::get(&g.pieces_placed),
                PopulationStats::get(&g.deploys_placed),
                PopulationStats::get(&g.charges_planted),
                PopulationStats::get(&g.struct_hits),
            );
        }
    }
}

/// Wait for the inhabitants to leave. `shutdown` has already set the flag
/// they watch, so this is a join and not a signal — and it is a join rather
/// than a `return` straight into process exit because a post dropped
/// mid-handshake leaves the accept loop holding a half-open connection the
/// shard then has to time out.
async fn drain(pop: Option<Population>) {
    if let Some(p) = pop {
        p.join().await;
        println!("population left");
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
