//! The shard server: `cargo run -p server --bin shard` (reads shard.toml,
//! CLAUDE.md commands). Prints the bound address and the dev-cert hash
//! (WebTransport `serverCertificateHashes` format), then a stats line every
//! 10 s. Runs until killed.
//!
//! `shard --close-wipe N shard.toml` serves nothing: it closes wipe N into
//! the hall on a stopped shard's files and exits (`ci/release.sh --wipe`).

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

/// The countdown SIGUSR1 starts, minutes: a deploy's warning (`--warn`).
const RESTART_SIGNAL_MIN: u16 = 2;

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
    // `shard [config]` serves. `shard --close-wipe N [config]` closes wipe N
    // on a stopped shard's files and exits (`close_only`). `shard
    // --check-saves [config]` says whether this build loads the live saves,
    // touching nothing (`check_saves`).
    let mut path = None;
    let mut close = None;
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--check-saves" {
            check = true;
        } else if a == "--close-wipe" {
            match args.next().and_then(|n| n.parse::<u32>().ok()) {
                Some(n) if n > 0 => close = Some(n),
                _ => {
                    eprintln!(
                        "shard: --close-wipe takes the wipe number: --close-wipe 3 shard.toml"
                    );
                    std::process::exit(2);
                }
            }
        } else if path.is_none() {
            path = Some(a);
        }
    }
    let path = path.unwrap_or_else(|| "shard.toml".into());
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
    if let Some(n) = close {
        std::process::exit(close_only(&cfg, n));
    }
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
    if check {
        std::process::exit(check_saves(&cfg, &content));
    }
    // Every table in one call, refusing the boot on the first that fails.
    // It was twelve separate `match` arms here and one of them was simply
    // never written — see `net::SimTables`.
    let mut tables = match server::net::bake_all(&content) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("shard: content bake refused: {e}");
            std::process::exit(1);
        }
    };
    // The dev spawn-kit override, `dev_spawn`'s shape and `dev_spawn`'s
    // caveats (`config.rs` states them in full). Unset — the shipping
    // default and the only path a public shard takes — this is a no-op and
    // the kit stays the content's rock and torch.
    //
    // Loud on stdout when it fires, because a shard silently handing every
    // joiner a different kit than `content/` declares is the exact shape of
    // "the doc reads as covered while nothing checks it": the next person to
    // wonder why their spawn is fat should find the answer in the boot log,
    // not in a diff.
    match server::config::dev_spawn_kit(cfg.dev_spawn_kit.as_deref(), &content) {
        Ok(None) => {}
        Ok(Some(kit)) => {
            println!(
                "⚠ dev_spawn_kit: {} stacks replace content's [[spawn_kit]] — \
                 a DEV shard, never a public one",
                kit.count
            );
            tables.spawn_kit = kit;
        }
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    }
    // A finished set of achievements is posted where the wipe is.
    tables.achievements.discord = cfg.discord_webhook.clone();
    if !cfg.dev_heli {
        println!("⚠ dev_heli = false: no patrol heli — a DEV shard, never a public one");
        tables.heli = sim_core::heli::HeliDef::INERT;
    }
    if cfg.dev_works_lit {
        println!(
            "⚠ dev_works_lit: every work opens and lights on the first sweep — the arc's \
             top and fel tier C at boot, a DEV shard, never a public one"
        );
        for d in tables.works.defs.iter_mut() {
            d.opens_at = 0;
            d.fallback_at = 0;
        }
    }
    if let Some(h) = cfg.dev_arc_hour {
        println!(
            "⚠ dev_arc_hour = {h}: a fresh world starts {h} hours into its wipe (a saved \
             world keeps its own clock) — a DEV shard, never a public one"
        );
    }
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
            "island ok: {live}/{} authored sites (haven pad + {} waystations + {} inland)",
            server::boot::AUTHORED_SITES,
            sim_core::terrain::WAYSTATIONS,
            sim_core::terrain::INLAND_SITES
        ),
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    }
    // **The wipe, before any file is opened** (`server::wipe`). A shard that
    // was down when its scheduled wipe came applies it now; one meeting a
    // schedule for the first time starts counting from this boot.
    let now = unix_now();
    let wipe_state_path =
        server::wipe::state_path(cfg.world_file.as_deref(), cfg.save_file.as_deref());
    let mut wipe_state = match wipe_state_path.as_deref().map(server::wipe::read_state) {
        None => None,
        Some(Ok(st)) => st,
        Some(Err(e)) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    if let (Some(path), Some(sched)) = (wipe_state_path.as_deref(), cfg.wipe) {
        let missed = match wipe_state {
            None => false,
            Some(st) => sched
                .last_at_or_before(now)
                .is_some_and(|due| st.last < due),
        };
        if missed {
            let st = wipe_state.unwrap_or_default();
            let n = st.wipes + 1;
            let bp = server::wipe::blueprints_due(n, cfg.blueprint_wipe_every);
            println!(
                "wipe: the scheduled wipe came while the shard was down — applying wipe {n} now"
            );
            end_world(&cfg, now, n, bp);
            wipe_state = server::wipe::read_state(path).ok().flatten();
        } else if wipe_state.is_none() {
            let st = server::wipe::State {
                wipes: 0,
                last: now,
            };
            if let Err(e) = server::wipe::write_state(path, &st) {
                eprintln!("shard: wipe state {}: {e}", path.display());
                std::process::exit(1);
            }
            wipe_state = Some(st);
        }
    }
    let wipes_so_far = wipe_state.map_or(0, |s| s.wipes);
    let wipe_clock =
        server::wipe::Clock::new(cfg.wipe, cfg.blueprint_wipe_every, wipes_so_far, now);
    match wipe_clock.next() {
        Some(p) => println!(
            "wipe: next {} · wipe {} · {}",
            server::wipe::fmt_utc(p.at),
            wipes_so_far + 1,
            if p.blueprints {
                "blueprints too"
            } else {
                "blueprints kept"
            }
        ),
        None => println!("wipe: no schedule (an admin's /wipe still works)"),
    }

    // The player store, opened and validated before a port is bound — the
    // posture content and the island already take (CLAUDE.md wall 7). It needs
    // the content hash, which is why it is opened HERE and handed over rather
    // than inside `spawn_shard`: a save file describes inventories as item
    // *indices*, so restoring it under moved content would hand players the
    // wrong things, and that has to be a refusal rather than a surprise.
    let mut saves = match cfg.save_file.as_deref() {
        None => {
            println!("saves off: no save_file — every join builds a fresh character");
            server::store::Saves::off()
        }
        Some(path) => match server::store::open(
            Path::new(path),
            cfg.seed,
            content.hash(),
            content.layout_hash(),
            // For the condition wall (`server::cond`): the one record check
            // the decoder cannot make without the baked ceilings.
            &tables.gather,
        ) {
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
                if found.balance_moved {
                    // Said once: the file was written under other numbers
                    // on this layout, and the header now names these.
                    println!(
                        "saves: content was rebalanced since the last boot — every save \
                         kept ({} clamped to a lowered condition ceiling)",
                        found.clamped
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
                    // Not a warning since SIWE: a joiner who signs in is
                    // proved and saved whatever this knob says. It only lets
                    // guests in, and a guest is filed under nothing.
                    println!(
                        "saves: require_auth is false, so guests are admitted and \
                         remembered by nobody; players who sign in are saved as usual"
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
    // The island's own digest, computed once: the world file's header pins
    // it, and the trust log's header names the island its rows are about.
    let world_digest = cfg
        .world_file
        .as_ref()
        .map(|_| sim_core::probe::probe_terrain(cfg.seed));
    let mut world_boot = match cfg.world_file.as_deref() {
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
            let world_digest = world_digest.unwrap_or_default();
            match server::worldfile::open(
                Path::new(path),
                &mut trial,
                cfg.seed,
                content.hash(),
                content.layout_hash(),
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
                        if found.balance_moved {
                            println!(
                                "world: content was rebalanced since the last save — the \
                                 world kept ({} stacks clamped to a lowered condition ceiling)",
                                found.clamped
                            );
                        }
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

    // **The trust ledger's log** (`server::trustlog`), beside the world file
    // it records: `<world_file>.trust/`. Tied to the world file and not a key
    // of its own because its rows are stamped with the world's tick, and that
    // tick only continues across a restart when the world does. Opened here,
    // before a port is bound, so a directory it cannot write refuses the boot
    // the way a save file does.
    let mut trust_log = match (cfg.world_file.as_deref(), world_digest) {
        (Some(world_path), Some(world)) => {
            let dir = format!("{world_path}.trust");
            let ident = server::trustlog::ShardIdent {
                domain: cfg.domain.clone(),
                seed: cfg.seed,
                content: content.hash(),
                world,
                build: protocol::version::BUILD_ID.to_string(),
                proto: protocol::PROTO_VER,
            };
            match server::trustlog::spawn(
                Path::new(&dir),
                ident,
                server::trustlog::Limits::default(),
            ) {
                Ok((tap, log)) => {
                    println!(
                        "trust log ok: {dir}/{} · boot {}",
                        server::trustlog::segment_name(log.segment),
                        log.boot
                    );
                    saves.trust = tap;
                    Some(log)
                }
                Err(e) => {
                    eprintln!("shard: trust log {dir}: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!(
                "trust log off: no world_file — trust rows are counted (trust_unlogged) \
                 and kept nowhere"
            );
            None
        }
    };
    let status_addr = cfg.status_addr;
    let status_base = cfg
        .world_file
        .as_deref()
        .or(cfg.save_file.as_deref())
        .map(|b| Path::new(b).to_path_buf());
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
    // The spectator door (`NETCODE.md` §2.3), said once at boot so the
    // counters on the periodic line below have their other half.
    let spectate = cfg.spectate;
    if spectate.open() {
        println!(
            "spectate open: {} seats, {} per player · agents live · humans {}",
            spectate.seats,
            spectate.per_target,
            if spectate.human_delay_s == 0 {
                "refused".to_string()
            } else {
                format!("behind a {} s delay", spectate.human_delay_s)
            }
        );
    }
    // Who this shard seats at all, said once at boot either way, so an
    // operator reading the log knows which kind of island this is.
    println!(
        "agents_only: {}",
        if cfg.agents_only {
            "true — declared agents and spectators only; a person is refused at the hello"
        } else {
            "false — people and agents alike"
        }
    );
    // The names a sign-in may be bound to, said once where the operator
    // listed more than `domain` (`config.rs`). Without the key this prints
    // nothing, and the shard is the one-name shard it always was.
    if cfg.siwe_domains.is_some() {
        println!(
            "sign-in: proofs bound to {} are accepted",
            cfg.siwe_accepts().join(", ")
        );
    }
    world_boot.wipe = wipe_clock;
    let handle = match spawn_shard(cfg, tables, saves, world_boot).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("shard: {e}");
            std::process::exit(1);
        }
    };
    println!("shard up on {} (seed {seed})", handle.local_addr);
    // ⚠ **This line said "dev cert" unconditionally**, including on a shard
    // serving a real Let's Encrypt chain — measured 2026-08-23 on the public
    // shard, whose boot log read `dev cert sha256 2d:c0:…` while it was in
    // fact serving the `game.elopros.com` leaf. That is the wrong thing to say
    // in the one log line somebody reads while debugging TLS: it invites the
    // conclusion that the shard is self-signing, which sends the search in the
    // opposite direction from the fault. The hash is worth printing either way
    // — for a dev chain it is the `serverCertificateHashes` a browser needs —
    // so name which of the two it is instead of assuming the dev case.
    match boot_cfg.cert_pem.as_deref() {
        Some(path) => println!("cert sha256 {} (chain {path})", handle.cert_hash),
        None => println!(
            "dev cert sha256 {} (self-signed, this boot only)",
            handle.cert_hash
        ),
    }
    // **SIGUSR1 is "restart in two minutes"**: the `/restart` countdown for an
    // operator with no admin in game, which `ci/deploy_shard.sh --warn` sends
    // after it stages a build. Installed before the status endpoint, because
    // that endpoint's `restart_at` key is the deploy's word that this shard
    // has the handler (SIGUSR1's default is to kill). Unix only.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::user_defined1()) {
            Ok(mut usr1) => {
                let stats = handle.stats.clone();
                tokio::spawn(async move {
                    while usr1.recv().await.is_some() {
                        ShardStats::set(&stats.restart_asked, RESTART_SIGNAL_MIN as u64 + 1);
                        println!("shard: SIGUSR1 — restarting in {RESTART_SIGNAL_MIN} min");
                    }
                });
            }
            Err(e) => {
                eprintln!("shard: SIGUSR1 handler: {e}");
                std::process::exit(1);
            }
        }
    }
    // The status endpoint, only where the operator said (config.rs says why
    // absent-serves-nothing is the default). A bind failure refuses the boot
    // rather than running without it: a shard whose config names an endpoint
    // it silently does not serve is the config-says-X-shard-does-Y defect
    // the parser refuses everywhere else. Daemon thread — shutdown neither
    // signals nor joins it, so the flush path above cannot hang on it.
    if let Some(addr) = status_addr {
        match server::status::spawn_status_with(addr, handle.stats.clone(), status_base) {
            Ok(bound) => println!(
                "status on http://{bound}/status.json, /standings.json and /payout/<wipe>.json"
            ),
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
    let mut wipe_watch = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            // The sim thread's wipe clock came due and it has already raised
            // the shutdown flag; wait out the flush like any stop, then end
            // the world on the closed files and exit for the supervisor to
            // bring up the fresh island.
            _ = wipe_watch.tick() => {
                if !ShardStats::raised(&handle.stats.wipe_due) {
                    // A `/restart` came due: the same flush, no wipe, and an
                    // exit the supervisor answers by starting the shard again
                    // (on whatever build `ci/deploy_shard.sh` staged).
                    if ShardStats::raised(&handle.stats.restart_due) {
                        shutdown(&handle, "RESTART").await;
                        trust_down(trust_log.as_mut()).await;
                        drain(pop.take()).await;
                        println!("shard: restarting — exiting; the supervisor starts it again");
                        return;
                    }
                    continue;
                }
                shutdown(&handle, "WIPE").await;
                trust_down(trust_log.as_mut()).await;
                drain(pop.take()).await;
                let bp = ShardStats::raised(&handle.stats.wipe_blueprints);
                end_world(&boot_cfg, unix_now(), wipes_so_far + 1, bp);
                println!("shard: wiped — exiting; the supervisor brings up the new island");
                return;
            }
            _ = tokio::signal::ctrl_c() => {
                shutdown(&handle, "SIGINT").await;
                trust_down(trust_log.as_mut()).await;
                drain(pop.take()).await;
                return;
            }
            name = stop.recv() => {
                shutdown(&handle, name).await;
                trust_down(trust_log.as_mut()).await;
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
             refused v/full/agents/people {}/{}/{}/{} · handshake errors {} · dropped-ticks {} · saves restored/written/lost {}/{}/{} · \
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
            ShardStats::get(&s.refused_agents),
            ShardStats::get(&s.refused_agents_only),
            ShardStats::get(&s.handshake_errors),
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
        // Its own line, and only on a shard whose door is open: a seat is not
        // a player, so none of it is in the line above.
        if spectate.open() {
            println!(
                "spectators {}/{} · seated {} · refused {} full {} ended {} · forged in/act {}/{}",
                ShardStats::get(&s.spectators),
                spectate.seats,
                ShardStats::get(&s.spectate_joins),
                ShardStats::get(&s.spectate_refused),
                ShardStats::get(&s.spectate_full),
                ShardStats::get(&s.spectate_ended),
                ShardStats::get(&s.spectate_input_refused),
                ShardStats::get(&s.spectate_actions_refused),
            );
        }
        trust_line(s, trust_log.as_ref());
        if let Some(p) = &pop {
            let g = &p.stats;
            // What was placed, armed and hit is the shard's own count, once
            // per fact: every post hears every broadcast, so the posts'
            // tallies summed would count each one once per post.
            println!(
                "population {}/{} live · shifts {}/{} started/ended, {} errored · \
                 sent in/act {}/{} · top storey {} · on the island: placed \
                 pieces/deploys {}/{} · charges {} · struct hits {}",
                p.live(),
                population,
                PopulationStats::get(&g.shifts_started),
                PopulationStats::get(&g.shifts_ended),
                PopulationStats::get(&g.shift_errors),
                PopulationStats::get(&g.inputs_sent),
                PopulationStats::get(&g.actions_sent),
                PopulationStats::get(&g.top_storey),
                ShardStats::get(&s.pieces_placed),
                ShardStats::get(&s.deploys_placed),
                ShardStats::get(&s.charges_armed),
                ShardStats::get(&s.struct_hits),
            );
        }
    }
}

/// Wipe `n`'s last word: each board's podium and everyone's placing, read
/// from the standings the shutdown wrote, into the hall; what is owed to whom
/// (`<world>.payout-<n>.csv`), for the operator to send; and the results to
/// Discord if the shard posts there. Once per wipe: a wipe the hall already
/// holds is not closed again, so a retried wipe never pays a podium twice.
/// `false` if the hall could not be written.
fn close_world(cfg: &server::config::ShardConfig, now: u64, n: u32, base: &Path) -> bool {
    if server::standings::is_closed(base, n) {
        println!("wipe {n}: already in the hall — it is closed once, not again");
        return true;
    }
    // No standings is a close that writes nothing (`close_wipe`), so it is
    // not reported as one.
    let kept = server::standings::standings_path(base).exists();
    match server::standings::close_wipe(base, n, &cfg.prizes, &cfg.exits) {
        Ok(closed) => {
            if let Some(w) = &closed.winner {
                println!("wipe {n}: the island goes to {w}");
            }
            if !closed.paid.is_empty() {
                // `paid` is one row per (wallet, coin): prizes and extraction
                // can be different coins, so total each coin on its own and
                // count wallets, not rows.
                let mut coins: std::collections::BTreeMap<&str, u64> = Default::default();
                for p in &closed.paid {
                    *coins.entry(p.3.as_str()).or_default() += p.2;
                }
                let owed: Vec<String> = coins.iter().map(|(t, a)| format!("{a} {t}")).collect();
                let wallets = closed
                    .paid
                    .iter()
                    .map(|p| p.0.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                println!(
                    "wipe {n}: {} owed to {wallets} wallets — {} (the operator sends it)",
                    owed.join(" + "),
                    server::standings::payout_path(base, n, "csv").display()
                );
            }
            if kept {
                println!(
                    "wipe {n}: closed into {} ({} podium places)",
                    server::standings::hall_path(base).display(),
                    closed.podiums.len()
                );
            }
            if let Some(url) = cfg.discord_webhook.as_deref() {
                let next = cfg.wipe.map(|s| server::wipe::fmt_utc(s.next_after(now)));
                let text = server::standings::discord_text(n, &closed, next.as_deref());
                if let Err(e) = server::standings::post_discord(url, &text) {
                    eprintln!("shard: wipe {n}: {e}");
                }
            }
            true
        }
        Err(e) => {
            eprintln!("shard: wipe {n}: the hall was not written: {e}");
            false
        }
    }
}

/// `shard --close-wipe N <config>`: [`close_world`] on a STOPPED shard's
/// files, and nothing else — no file moves, no store rewrite, no new state.
/// `ci/release.sh --wipe` runs it before it moves the world aside itself: a
/// release that changes the save layout cannot let a shard wipe in place
/// (`wipe::apply` rewrites the store), and without this a release wipe would
/// end a world without its podium or its payouts. `N` must be the world the
/// wipe state says is running. The exit code: 0 closed (or already was), 1 not.
fn close_only(cfg: &server::config::ShardConfig, n: u32) -> i32 {
    let Some(base) = cfg
        .world_file
        .as_deref()
        .or(cfg.save_file.as_deref())
        .map(Path::new)
    else {
        println!("wipe {n}: this shard keeps nothing on disk, so there is nothing to close");
        return 0;
    };
    if let Some(path) =
        server::wipe::state_path(cfg.world_file.as_deref(), cfg.save_file.as_deref())
    {
        match server::wipe::read_state(&path) {
            Ok(st) => {
                let running = st.map_or(1, |s| s.wipes + 1);
                if n != running {
                    eprintln!(
                        "shard: --close-wipe {n}, but {} says the world on disk is wipe {running}",
                        path.display()
                    );
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("shard: {e}");
                return 1;
            }
        }
    }
    if !server::standings::standings_path(base).exists() {
        println!(
            "wipe {n}: no standings beside {} — the shard that ran this world kept none, \
             so the hall gets no entry for it",
            base.display()
        );
    }
    if close_world(cfg, unix_now(), n, base) {
        0
    } else {
        1
    }
}

/// `shard --check-saves [config]`: would THIS build's shard load the saves
/// the config names? The header checks a boot makes (formats, seed, content
/// layout, the island's digest), read-only: no backup rotates and nothing is
/// written, so `ci/release.sh` and `ci/deploy_shard.sh` ask it of the live
/// files under the running shard, before anything moves. A deploy that fails
/// it is a wipe, and finding that out from a shard refusing to boot means
/// finding it out with the island down.
///
/// A scheduled wipe that has come due is a boot that ends the world before it
/// opens a file (`main`'s missed-wipe path), so then the world file need not
/// load. The player store still must: the wipe rewrites its records under
/// this build and keeps its header.
///
/// Exit 0: a boot loads them, or starts fresh. 3: it would refuse (a wipe).
/// 1: the question could not be asked (a file this user cannot read, or a
/// wipe state the shard refuses too).
fn check_saves(cfg: &server::config::ShardConfig, content: &content::Content) -> i32 {
    let (hash, layout) = (content.hash(), content.layout_hash());
    println!(
        "check-saves: content {hash:016x} · layout {layout:016x} · seed {}",
        cfg.seed
    );
    let due = match server::wipe::state_path(cfg.world_file.as_deref(), cfg.save_file.as_deref())
        .map(|p| server::wipe::read_state(&p))
    {
        Some(Err(e)) => {
            eprintln!("shard: {e}");
            return 1;
        }
        Some(Ok(Some(st))) => cfg
            .wipe
            .and_then(|s| s.last_at_or_before(unix_now()))
            .is_some_and(|at| st.last < at)
            .then_some(st.wipes + 1),
        _ => None,
    };
    let how = |m: server::store::ContentMatch| match m {
        server::store::ContentMatch::Same => "",
        server::store::ContentMatch::Balance => " (rebalanced, which loads)",
        server::store::ContentMatch::Upgrade => " (its layout is pinned on this boot)",
    };
    // A file this user cannot read is a question not asked, never a wipe:
    // the deploy is run as the shard's user or it is not run.
    for path in [cfg.world_file.as_deref(), cfg.save_file.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Err(e) = std::fs::File::open(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "shard: --check-saves cannot read {path}: {e} (run it as the shard's user)"
                );
                return 1;
            }
        }
    }
    let mut wipe = false;
    if let Some(path) = cfg.world_file.as_deref() {
        if let Some(n) = due {
            println!(
                "world {path}: scheduled wipe {n} is due, so a boot ends that world \
                 before opening it"
            );
        } else {
            let digest = sim_core::probe::probe_terrain(cfg.seed);
            match server::worldfile::check(Path::new(path), cfg.seed, hash, layout, digest) {
                Ok(None) => println!("world {path}: none yet, a fresh island"),
                Ok(Some(m)) => println!("world {path}: ok{}", how(m)),
                Err(e) => {
                    wipe = true;
                    println!("world {path}: WIPE: {e}");
                }
            }
        }
    }
    if let Some(path) = cfg.save_file.as_deref() {
        match server::store::check(Path::new(path), cfg.seed, hash, layout) {
            Ok(None) => println!("saves {path}: none yet, everyone starts fresh"),
            Ok(Some(m)) => println!("saves {path}: ok{}", how(m)),
            Err(e) => {
                wipe = true;
                println!("saves {path}: WIPE: {e}");
            }
        }
    }
    if wipe {
        println!("check-saves: this build cannot load the live saves; deploying it is a wipe");
        3
    } else {
        println!("check-saves: this build loads the live saves; no wipe");
        0
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Wipe number `n` on the closed files, then record it. A failure exits
/// non-zero: the files are as the archive step left them, and the operator
/// reads the line before the supervisor's restart does anything else.
fn end_world(cfg: &server::config::ShardConfig, now: u64, n: u32, blueprints: bool) {
    let world = cfg.world_file.as_deref().map(Path::new);
    let save = cfg.save_file.as_deref().map(Path::new);
    if world.is_none() && save.is_none() {
        // A shard that keeps nothing: the restart is the whole wipe.
        println!("wipe {n}: nothing on disk — the next boot is a fresh island anyway");
        return;
    }
    // The hall first, before the archive moves the standings. A hall that
    // could not be written does not stop the wipe; the line says so.
    if let Some(base) = world.or(save) {
        close_world(cfg, now, n, base);
    }
    match server::wipe::apply(world, save, now, blueprints) {
        Ok(r) => println!(
            "wipe {n}: {} · players: {} kept their {}, {} cleared · archived in {}",
            if r.world_archived {
                "world archived"
            } else {
                "no world file"
            },
            r.kept,
            if blueprints {
                "nothing (blueprint wipe)"
            } else {
                "blueprints and glyphs"
            },
            r.cleared,
            r.archive.display()
        ),
        Err(e) => {
            eprintln!("shard: wipe {n} failed: {e}");
            std::process::exit(1);
        }
    }
    if let Some(path) =
        server::wipe::state_path(cfg.world_file.as_deref(), cfg.save_file.as_deref())
    {
        let st = server::wipe::State {
            wipes: n,
            last: now,
        };
        if let Err(e) = server::wipe::write_state(&path, &st) {
            eprintln!(
                "shard: wipe {n} done, but its state {}: {e}",
                path.display()
            );
            std::process::exit(1);
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

/// The trust ledger's line on the 10 s report. Silent until there is a row,
/// a loss or a stopped writer, so a quiet shard's console stays quiet.
fn trust_line(s: &ShardStats, log: Option<&server::trustlog::TrustLog>) {
    use server::trustlog::WriterStats as W;
    let rows = ShardStats::get(&s.trust_rows);
    match log {
        Some(l)
            if rows > 0
                || l.stats.stopped()
                || W::get(&l.stats.write_errors) > 0
                || ShardStats::get(&s.trust_sim_overflow) > 0 =>
        {
            let w = &l.stats;
            println!(
                "trust rows {rows} · written {} · gaps {} · dropped {} · sim overflow {} · \
                 write errors {} · segments {} (pruned {})",
                W::get(&w.rows_written),
                W::get(&w.gaps_written),
                ShardStats::get(&s.trust_ring_drops),
                ShardStats::get(&s.trust_sim_overflow),
                W::get(&w.write_errors),
                W::get(&w.segments_opened),
                W::get(&w.segments_pruned),
            );
            // The writer only stops when the sim thread lets go of its tap,
            // which a running shard never does. Said out loud rather than
            // inferred: this is the ledger silently not being kept.
            if w.stopped() {
                println!(
                    "trust WARNING: the log writer has stopped while the shard runs — \
                     the sim thread is not holding its tap, and no row is being kept"
                );
            }
        }
        None if rows > 0 => println!(
            "trust rows {rows} · unlogged {} (no world_file, so no trust log)",
            ShardStats::get(&s.trust_unlogged)
        ),
        _ => {}
    }
}

/// Wait for the trust log's writer to write its `close` line. It stops once
/// the sim thread drops its tap, which `shutdown` has already waited for.
/// Polled like `shutdown`'s store flag, so no runtime thread sleeps in it.
async fn trust_down(log: Option<&mut server::trustlog::TrustLog>) {
    let Some(log) = log else {
        return;
    };
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while !log.stats.stopped() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if log.join_within(Duration::ZERO) {
        println!(
            "shard: trust log closed · {} rows written this boot ({} gaps, {} write errors)",
            server::trustlog::WriterStats::get(&log.stats.rows_written),
            server::trustlog::WriterStats::get(&log.stats.gaps_written),
            server::trustlog::WriterStats::get(&log.stats.write_errors),
        );
    } else {
        eprintln!(
            "shard: WARNING — the trust log writer did not finish inside {}s; \
             its last lines may be unsynced",
            SHUTDOWN_WAIT.as_secs()
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
