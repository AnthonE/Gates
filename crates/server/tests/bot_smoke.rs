//! The 50-bot smoke (DESIGN.md §11 M0): real wtransport connections
//! against a real shard on loopback — handshake, inputs, snapshots, acks,
//! deltas, all through the actual net stack. Asserts are structural
//! (connected counts, decode integrity, ack loop engagement), never timed:
//! this box shares cores, so wall-clock is not a claim here.

use server::botclient::{bot_endpoint, run_bot};
use server::config::ShardConfig;
use server::net::spawn_shard;
use server::stats::ShardStats;
use server::store::Saves;
use std::time::Duration;

const BOTS: usize = 50;

/// The shipped content set, baked — the smoke runs the same boot path the
/// shard binary does, so gather and the catalog are live under the herd.
fn baked_content() -> (
    sim_core::gather::GatherContent,
    sim_core::craft::CraftContent,
    sim_core::build::BuildContent,
    sim_core::deploy::DeployContent,
    sim_core::combat::CombatContent,
    sim_core::backpack::BackpackContent,
    sim_core::survival::SurvivalContent,
    sim_core::oven::CookContent,
    sim_core::loot::LootContent,
    sim_core::mob::MobContent,
    protocol::ItemCatalog,
) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    let gather = content.bake_gather().expect("shipped content bakes");
    let craft = content.bake_craft().expect("shipped recipes bake");
    let build = content.bake_building().expect("shipped building set bakes");
    let deploy = content
        .bake_deployables()
        .expect("shipped deployables bake");
    let combat = content.bake_combat().expect("shipped weapons bake");
    let backpack = content
        .bake_backpack()
        .expect("shipped despawn ladder bakes");
    let survival = content
        .bake_survival()
        .expect("shipped survival clock bakes");
    let cook = content.bake_cooking().expect("shipped oven table bakes");
    let loot = content.bake_loot().expect("shipped loot tables bake");
    let mobs = content.bake_mobs().expect("shipped animals bake");
    let catalog = server::net::bake_catalog(&content).expect("shipped catalog bakes");
    (
        gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_bot_smoke_50() {
    let (gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog) =
        baked_content();
    let handle = spawn_shard(
        ShardConfig::ephemeral(0xC0FFEE),
        gather,
        craft,
        build,
        deploy,
        combat,
        backpack,
        survival,
        cook,
        // A test shard spawns naked: the alpha `[[spawn_kit]]` is scaffolding
        // for a human looking at the game, and a suite that asserted on a
        // fresh inventory would be asserting on content instead of on code.
        sim_core::inventory::SpawnKit::EMPTY,
        loot,
        mobs,
        catalog,
        // Persistence off: these shards write no file, so the suite stays
        // hermetic and every join is a fresh character (`store::Saves::off`).
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("shard boots");
    let addr = handle.local_addr;

    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    let mut tasks = Vec::with_capacity(BOTS);
    for i in 0..BOTS {
        let endpoint = endpoint.clone();
        tasks.push(tokio::spawn(async move {
            // Rows `None`: this suite measures the snapshot/ack pipeline, and
            // it asserted its numbers before the raid lane existed. The raid
            // gets its own test below rather than perturbing this one.
            run_bot(&endpoint, addr, i as u64, Duration::from_secs(4), None).await
        }));
        // Stagger the TLS herd; the walk windows still overlap heavily.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The `players` gauge is a mirror — one `ShardStats::set` off
    // `ShardCore::connected` per tick in the net loop (`net.rs`) — and a
    // mirror with no gate is the round-1 hole: delete that line and every
    // suite stayed green while `/status.json` said 0 forever. This suite is
    // the only one that runs the loop the mirror lives in, so the claim is
    // made here, on observable state bounded by a timeout (never on elapsed
    // time): the gauge must leave 0 while bots are connected...
    tokio::time::timeout(Duration::from_secs(20), async {
        while ShardStats::get(&handle.stats.players) == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the players gauge never tracked a join — the net-loop mirror is gone");

    let mut reports = Vec::with_capacity(BOTS);
    for (i, t) in tasks.into_iter().enumerate() {
        let report = t
            .await
            .expect("bot task join")
            .unwrap_or_else(|e| panic!("bot {i} failed: {e}"));
        reports.push(report);
    }

    // ...and return to 0 once every bot has left, which is what separates a
    // gauge (set each tick off occupancy) from a counter that only climbs.
    tokio::time::timeout(Duration::from_secs(20), async {
        while ShardStats::get(&handle.stats.players) != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the players gauge never tracked the leaves back to 0");

    // Every bot got welcomed with the shard's seed and a unique id.
    let mut ids: Vec<u32> = reports.iter().map(|r| r.player_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), BOTS, "player ids must be unique");
    for r in &reports {
        let w = r.welcome.expect("welcomed");
        assert_eq!(w.seed, 0xC0FFEE, "welcome carries the world seed");
    }

    // The pipeline moved: snapshots applied, own entity seen, the input
    // ack loop engaged (last_executed advanced from 0).
    for (i, r) in reports.iter().enumerate() {
        assert!(
            r.snapshots_applied >= 10,
            "bot {i}: only {} snapshots applied",
            r.snapshots_applied
        );
        assert!(r.own_updates > 0, "bot {i}: never saw own entity");
        assert!(
            r.last_executed_seq > 0,
            "bot {i}: server never executed an input"
        );
        assert_eq!(r.decode_errors, 0, "bot {i}: decode errors");
        assert_eq!(r.no_baseline, 0, "bot {i}: baseline anomalies");
        assert!(r.inputs_sent > 0, "bot {i}: sent no inputs");
        // The event lane spoke: with content loaded every client gets at
        // least the catalog drip, and every message must decode.
        assert!(r.events_received > 0, "bot {i}: event lane silent");
        assert_eq!(r.event_decode_errors, 0, "bot {i}: event decode errors");
    }

    // Interest works: 50 spawns scattered over the island won't all share
    // AOI, but at least some bots must see a neighbour.
    let bots_with_others = reports.iter().filter(|r| r.max_entities_seen > 1).count();
    assert!(
        bots_with_others > 0,
        "no bot ever saw another entity — AOI or spawn scatter broken"
    );

    // The ack→baseline loop closed: deltas engaged for most bots (a bot
    // whose whole window rode zero-states would mean acks never landed).
    let bots_with_deltas = reports.iter().filter(|r| r.delta_snapshots > 0).count();
    assert!(
        bots_with_deltas >= BOTS * 8 / 10,
        "deltas engaged for only {bots_with_deltas}/{BOTS} bots"
    );

    // Server-side integrity: all bots joined, nothing malformed arrived,
    // and the encoder never met an out-of-range body.
    let s = &handle.stats;
    assert_eq!(ShardStats::get(&s.joins), BOTS as u64, "joins");
    assert_eq!(ShardStats::get(&s.input_dg_bad), 0, "malformed inputs");
    assert_eq!(ShardStats::get(&s.encode_range_errors), 0, "range errors");
    assert_eq!(ShardStats::get(&s.forced_resyncs), 0, "forced resyncs");
    assert!(ShardStats::get(&s.ticks) > 60, "sim thread ticked");

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **The bots raid over the wire** (`NOW.md` §0rs item 1, off the merge
/// judge's repeated gap 1 *"a player has no opponent"*).
///
/// `test_raid_storm` proved the caps by pushing `raid_step` straight into
/// `World::tick`; nothing proved the profile could reach a shard as traffic.
/// This closes the round trip end to end and asserts on each link of it:
/// the bot derives its plot from **its own body** off the snapshot stream
/// (`raid_cycles`, and the plots must not all be the same cell — a constant
/// would satisfy every count here and only the spread can tell them apart),
/// every command the profile emits has a wire form the encoder accepts
/// (`actions_unencodable == 0`, which reddens the day a `raid_step` variant
/// lands with no arm in `encode_raid`), the frames decode server-side
/// (`actions_bad == 0` — a frame that did not would have *dropped the
/// session*), and the sim answers on the event lane (`*_refused`).
///
/// That last one is the measurement, not a fault. Roughly half of what
/// `raid_step` claims is meant to be refused, and a naked spawn can afford
/// none of the rest; what had never happened before this test is those
/// refusal paths being reached **through the net stack** rather than by a
/// direct `World::tick`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_bots_raid_over_the_wire() {
    /// Even, so the fleet is half attackers and half owners (`run_bot`
    /// splits on stream parity). Smaller than `BOTS`: this suite is about
    /// the action lane's round trip, and the herd size is `test_bot_smoke_50`'s
    /// claim, not this one's.
    const RAIDERS: usize = 8;

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    // Resolved by id, exactly as `bin/bots` resolves them — a row typed here
    // would pass while the shipped table said otherwise (wall 7).
    let rows = sim_core::bots::RaidRows {
        foundation: content
            .piece_index("build.foundation_twig")
            .expect("shipped foundation"),
        wall: content
            .piece_index("build.wall_twig")
            .expect("shipped wall"),
        container: content.deploy_index("item.box_small").expect("shipped box"),
        lock: content
            .deploy_index("item.lock_code")
            .expect("shipped lock"),
        charge_slot: 0,
        goods_slot: 2,
        code: 4242,
    };

    let (gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog) =
        baked_content();
    let handle = spawn_shard(
        ShardConfig::ephemeral(0x5A1D),
        gather,
        craft,
        build,
        deploy,
        combat,
        backpack,
        survival,
        cook,
        // A test shard spawns naked: the alpha `[[spawn_kit]]` is scaffolding
        // for a human looking at the game, and a suite that asserted on a
        // fresh inventory would be asserting on content instead of on code.
        // It also makes the claim sharper — a bot that can afford nothing
        // still has to have its claims *heard and refused* on the real path.
        sim_core::inventory::SpawnKit::EMPTY,
        loot,
        mobs,
        catalog,
        // Persistence off: these shards write no file, so the suite stays
        // hermetic and every join is a fresh character (`store::Saves::off`).
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("shard boots");
    let addr = handle.local_addr;

    let endpoint = std::sync::Arc::new(bot_endpoint().expect("client endpoint"));
    let mut tasks = Vec::with_capacity(RAIDERS);
    for i in 0..RAIDERS {
        let endpoint = endpoint.clone();
        tasks.push(tokio::spawn(async move {
            run_bot(
                &endpoint,
                addr,
                i as u64,
                Duration::from_secs(4),
                Some(rows),
            )
            .await
        }));
        // Stagger the TLS herd; the raid windows still overlap heavily.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut reports = Vec::with_capacity(RAIDERS);
    for (i, t) in tasks.into_iter().enumerate() {
        let report = t
            .await
            .expect("bot task join")
            .unwrap_or_else(|e| panic!("raider {i} failed: {e}"));
        reports.push(report);
    }

    for (i, r) in reports.iter().enumerate() {
        // The plot came off the snapshot stream: no body, no raid.
        assert!(
            r.raid_cycles > 0,
            "raider {i}: never seated a plot from its own body"
        );
        assert!(r.last_plot.is_some(), "raider {i}: no plot recorded");
        assert!(r.actions_sent > 0, "raider {i}: sent no action");
        // Every verb in the profile encodes. A variant added to `raid_step`
        // with no arm in `encode_raid` lands here, not on a shard.
        assert_eq!(
            r.actions_unencodable, 0,
            "raider {i}: {} raid commands had no wire form",
            r.actions_unencodable
        );
        // A refused *write* means the session died mid-raid — the shape a
        // malformed frame produces, and the one thing the encoder guard
        // above exists to prevent.
        assert_eq!(
            r.action_lane_errors, 0,
            "raider {i}: the action stream died mid-raid"
        );
        assert_eq!(r.event_decode_errors, 0, "raider {i}: event decode errors");
        // The round trip closed: the sim heard this bot's claims and
        // answered them. A naked spawn can afford none of them, so the
        // answer is a refusal — which is the half of wall 4 that had never
        // been driven over a socket.
        let verdicts = r.build_refused + r.deploy_refused + r.move_refused;
        assert!(
            verdicts > 0,
            "raider {i}: sent {} actions and the sim answered none of them",
            r.actions_sent
        );
    }

    // The plots are derived, not constant: bots spawn scattered, so their
    // cells must be too. This is the assertion that a hard-coded cell fails.
    let mut plots: Vec<(u16, u16)> = reports.iter().filter_map(|r| r.last_plot).collect();
    plots.sort_unstable();
    plots.dedup();
    assert!(
        plots.len() > 1,
        "every raider claimed the same plot {:?} — the body-to-cell derivation is a constant",
        plots.first()
    );

    // Fleet-wide: the build verbs specifically were heard, and the server
    // survived the storm with nothing malformed and no forced resync.
    let build_refusals: u64 = reports.iter().map(|r| r.build_refused).sum();
    assert!(build_refusals > 0, "no raider's Place ever reached the sim");
    let s = &handle.stats;
    assert!(ShardStats::get(&s.actions_ok) > 0, "no action decoded");
    assert_eq!(
        ShardStats::get(&s.actions_bad),
        0,
        "a raid frame failed to decode — that drops the session"
    );
    assert_eq!(ShardStats::get(&s.input_dg_bad), 0, "malformed inputs");
    assert_eq!(ShardStats::get(&s.encode_range_errors), 0, "range errors");
    assert_eq!(ShardStats::get(&s.forced_resyncs), 0, "forced resyncs");

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The C→S action lane over a real socket: a framed craft request crosses
/// the bidi stream, reaches the sim, and its verdict rides back on the
/// event lane; a malformed action frame drops the session and counts.
/// (`craft_wire.rs` covers the lane's semantics with decoded structs; this
/// is the read_frame → decode → sim → event round trip itself.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_action_lane_over_socket() {
    use protocol::{decode_event, encode_action_craft, EventMsg, MAX_STREAM_MSG_BYTES};
    use server::net::{client_handshake, read_event_frame, write_frame};

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    // A station-none recipe a naked spawn cannot afford: the sim must
    // answer the framed request with a refused event (missing inputs).
    let recipe = content
        .recipe_index("recipe.hatchet_stone")
        .expect("shipped recipe");

    let (gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog) =
        baked_content();
    let handle = spawn_shard(
        ShardConfig::ephemeral(11),
        gather,
        craft,
        build,
        deploy,
        combat,
        backpack,
        survival,
        cook,
        // A test shard spawns naked: the alpha `[[spawn_kit]]` is scaffolding
        // for a human looking at the game, and a suite that asserted on a
        // fresh inventory would be asserting on content instead of on code.
        sim_core::inventory::SpawnKit::EMPTY,
        loot,
        mobs,
        catalog,
        // Persistence off: these shards write no file, so the suite stays
        // hermetic and every join is a fresh character (`store::Saves::off`).
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let opening = connection.open_bi().await.expect("open_bi");
    let (mut send, mut recv) = opening.await.expect("bi");
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    // A guest handshake through the one shared implementation
    // (`net::client_handshake`) rather than a fourth hand-rolled copy.
    tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(
            &mut send,
            &mut recv,
            "test",
            protocol::Address::GUEST,
            |_| None,
        ),
    )
    .await
    .expect("welcome inside 5 s")
    .expect("welcomed");

    let len = encode_action_craft(recipe, 1, &mut buf).expect("action encodes");
    write_frame(&mut send, &buf[..len]).await.expect("action");

    // The event lane drips catalog/recipes/sync too; scan until the
    // refusal arrives. A hang here is the failure, bounded by the timeout.
    let refused = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (ev, ev_len) = read_event_frame(&mut recv).await.expect("event frame");
            if let Ok(EventMsg::CraftRefused { reason }) = decode_event(&ev[..ev_len]) {
                return reason;
            }
        }
    })
    .await
    .expect("craft verdict inside 10 s");
    assert_eq!(
        refused as u32,
        sim_core::craft::REFUSE_INPUTS,
        "naked spawn crafting a hatchet must refuse on inputs"
    );
    assert_eq!(ShardStats::get(&handle.stats.actions_ok), 1);
    assert_eq!(ShardStats::get(&handle.stats.actions_bad), 0);

    // A malformed action frame (KIND_ACTION with an unknown subtype)
    // counts and ends the session — framing trust is gone.
    write_frame(&mut send, &[0x1E]).await.expect("bad frame");
    tokio::time::timeout(Duration::from_secs(10), async {
        while ShardStats::get(&handle.stats.actions_bad) == 0
            || ShardStats::get(&handle.stats.leaves) == 0
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("malformed frame must count and drop the session inside 10 s");
    assert_eq!(ShardStats::get(&handle.stats.actions_bad), 1);

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A wrong protocol version is refused with a posted reason, not a hang
/// (DESIGN.md §5.9) — exercised through the real handshake path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_version_gate_refuses() {
    use protocol::{encode_hello, Hello, MAX_STREAM_MSG_BYTES};
    use server::net::{read_frame, write_frame};

    let (gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog) =
        baked_content();
    let handle = spawn_shard(
        ShardConfig::ephemeral(7),
        gather,
        craft,
        build,
        deploy,
        combat,
        backpack,
        survival,
        cook,
        // A test shard spawns naked: the alpha `[[spawn_kit]]` is scaffolding
        // for a human looking at the game, and a suite that asserted on a
        // fresh inventory would be asserting on content instead of on code.
        sim_core::inventory::SpawnKit::EMPTY,
        loot,
        mobs,
        catalog,
        // Persistence off: these shards write no file, so the suite stays
        // hermetic and every join is a fresh character (`store::Saves::off`).
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots");
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let opening = connection.open_bi().await.expect("open_bi");
    let (mut send, mut recv) = opening.await.expect("bi");
    let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
    // Version wrong, everything else this build's own — so the refusal can
    // only be about the number under test.
    let len = encode_hello(
        &Hello {
            proto_ver: 999,
            ver: protocol::version::VER,
            build: protocol::version::BUILD,
        },
        &mut buf,
    )
    .expect("encode");
    write_frame(&mut send, &buf[..len]).await.expect("write");
    let (reply, reply_len) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv))
        .await
        .expect("reply inside 5 s")
        .expect("a refusal frame, not silence");
    let refuse = protocol::decode_refuse(&reply[..reply_len]).expect("refuse decodes");
    assert_eq!(refuse.code, protocol::REFUSE_VERSION);
    assert_eq!(ShardStats::get(&handle.stats.refused_version), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **The release floor, in both directions** — because the direction that is
/// easy to get wrong is the one that admits.
///
/// A client below `min_client` is refused with `REFUSE_BUILD` and counted
/// apart from `refused_version`, since the two mean different things about a
/// shard. A client *above* it is admitted, and that half is the point of the
/// test: the gate is `<` and not `!=` on purpose (`config.rs` says why), so a
/// player who updated before the shard operator restarted must still get in.
/// An `!=` written here by mistake passes every "too old is refused" test
/// ever written and locks out exactly the people who did the right thing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_min_client_refuses_below_and_admits_above() {
    use protocol::{encode_hello, Hello, MAX_STREAM_MSG_BYTES};
    use server::net::{read_frame, write_frame};

    // The floor sits one minor above this build, so this build's own release
    // is genuinely below it — no hand-picked constant that could drift apart
    // from what the client actually sends.
    let (fma, fmi, fpa) = protocol::version::unpack(protocol::version::VER);
    let floor = protocol::version::pack(fma, fmi + 1, fpa);

    for (ver, want) in [
        (protocol::version::VER, Some(protocol::REFUSE_BUILD)), // below the floor
        (floor, None),                                          // exactly the floor: admitted
        (protocol::version::pack(fma, fmi + 2, fpa), None),     // above it: admitted
    ] {
        let (gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog) =
            baked_content();
        let mut cfg = ShardConfig::ephemeral(11);
        cfg.min_client = floor;
        let handle = spawn_shard(
            cfg,
            gather,
            craft,
            build,
            deploy,
            combat,
            backpack,
            survival,
            cook,
            sim_core::inventory::SpawnKit::EMPTY,
            loot,
            mobs,
            catalog,
            Saves::off(),
            server::worldfile::WorldBoot::off(),
        )
        .await
        .expect("boots");
        let endpoint = bot_endpoint().expect("endpoint");
        let connection = endpoint
            .connect(&format!("https://{}", handle.local_addr))
            .await
            .expect("connects");
        let opening = connection.open_bi().await.expect("open_bi");
        let (mut send, mut recv) = opening.await.expect("bi");
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        // The wire version is this build's, always: the floor is a statement
        // about the RELEASE and must be reachable with the bytes agreeing.
        let len = encode_hello(
            &Hello {
                proto_ver: protocol::PROTO_VER,
                ver,
                build: protocol::version::BUILD,
            },
            &mut buf,
        )
        .expect("encode");
        write_frame(&mut send, &buf[..len]).await.expect("write");

        let (reply, reply_len) =
            tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv))
                .await
                .expect("reply inside 5 s")
                .expect("a frame, not silence");
        let reply = &reply[..reply_len];

        match want {
            Some(code) => {
                let refuse = protocol::decode_refuse(reply).expect("refuse decodes");
                assert_eq!(
                    refuse.code, code,
                    "a client below the floor must be refused"
                );
                assert_eq!(
                    ShardStats::get(&handle.stats.refused_build),
                    1,
                    "the build refusal must be counted under its own name"
                );
                assert_eq!(
                    ShardStats::get(&handle.stats.refused_version),
                    0,
                    "a release refusal is not a wire refusal"
                );
                // A player meets a sentence, never a bare number.
                assert!(protocol::refuse_text(code).is_some());
            }
            None => {
                // Admitted means the handshake CONTINUES — the next thing a
                // shard sends is the SIWE challenge, not a refusal. Asserting
                // on the kind rather than on a full join keeps this test about
                // the gate under test and nothing else.
                assert_eq!(
                    protocol::peek_kind(reply).expect("a kind"),
                    protocol::KIND_CHALLENGE,
                    "a client at or above the floor must not be refused"
                );
                assert_eq!(ShardStats::get(&handle.stats.refused_build), 0);
            }
        }
        handle
            .shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The dev gate at its source: a shard's welcome carries `dev` true iff it
/// is running a dev override (`shard.toml dev_spawn`). That bit is a
/// client's only dev gate — dev affordances hang off it and nothing else —
/// so a public shard reporting true would offer a dev surface to every
/// player who connects.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_welcome_dev_bit_tracks_dev_spawn() {
    use server::net::client_handshake;

    // Same shard, same everything, one config key apart.
    for (dev_spawn, want) in [(None, false), (Some((1024.0, 1024.0)), true)] {
        let (gather, craft, build, deploy, combat, backpack, survival, cook, loot, mobs, catalog) =
            baked_content();
        let mut cfg = ShardConfig::ephemeral(13);
        cfg.dev_spawn = dev_spawn;
        let handle = spawn_shard(
            cfg,
            gather,
            craft,
            build,
            deploy,
            combat,
            backpack,
            survival,
            cook,
            // A test shard spawns naked: the alpha `[[spawn_kit]]` is scaffolding
            // for a human looking at the game, and a suite that asserted on a
            // fresh inventory would be asserting on content instead of on code.
            sim_core::inventory::SpawnKit::EMPTY,
            loot,
            mobs,
            catalog,
            Saves::off(),
            server::worldfile::WorldBoot::off(),
        )
        .await
        .expect("boots");
        let endpoint = bot_endpoint().expect("endpoint");
        let connection = endpoint
            .connect(&format!("https://{}", handle.local_addr))
            .await
            .expect("connects");
        let opening = connection.open_bi().await.expect("open_bi");
        let (mut send, mut recv) = opening.await.expect("bi");
        let welcome = tokio::time::timeout(
            Duration::from_secs(5),
            client_handshake(
                &mut send,
                &mut recv,
                "test",
                protocol::Address::GUEST,
                |_| None,
            ),
        )
        .await
        .expect("welcome inside 5 s")
        .expect("welcomed");
        assert_eq!(
            welcome.dev, want,
            "dev_spawn {dev_spawn:?} must welcome with dev = {want}"
        );
        handle
            .shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
