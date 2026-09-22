//! An agent's decisions become ordinary wire inputs. The captured input
//! trace must also replay identically without allocations inside the sim.
use server::botclient::{bot_endpoint, run_driven_bot, walk_ticks, BotDriver};
use server::explorer::Gatherer;
use server::jev::{
    Action, Decision, DecisionSource, Driver, Observation, REQUEST_TIMEOUT, THINK_INTERVAL,
};
use server::view::ClientView;
use sim_core::input::{InputFrame, BTN_JUMP};
use sim_core::world::{Command, World};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::time::Duration;

thread_local! { static ALLOCS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct CountAlloc;
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = ALLOCS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
        System.realloc(ptr, layout, size)
    }
}
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

struct Alternating(bool);
impl DecisionSource for Alternating {
    fn decide(&mut self, observation: Observation) -> Result<Decision, String> {
        self.0 = !self.0;
        if self.0 {
            Forward.decide(observation)
        } else {
            Err("an allocated error must be dropped on the worker".to_owned())
        }
    }
}

#[test]
fn repeated_worker_handoffs_never_touch_the_frame_loops_allocator() {
    let mut driver =
        Driver::new(Alternating(false), Duration::from_nanos(1), REQUEST_TIMEOUT).unwrap();
    let mut view = ClientView::new();
    view.entities.push((
        1,
        protocol::EntityState {
            id: 1,
            grounded: true,
            ..Default::default()
        },
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    // Enough round trips to cross several Tokio mpsc allocation blocks;
    // completion is a count, never an assertion about throughput.
    while driver.stats.decisions + driver.stats.failures < 128 {
        view.newest_applied = Some(view.newest_applied.unwrap_or_default() + 1);
        ALLOCS.with(|count| count.set(Some(0)));
        driver.frame(&view, 1, 1);
        let heap_ops = ALLOCS.with(|count| count.replace(None).unwrap());
        assert_eq!(
            heap_ops, 0,
            "worker handoff allocated or freed on the input loop"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "worker stopped answering"
        );
        std::thread::yield_now();
    }
    assert!(driver.stats.decisions > 0 && driver.stats.failures > 0);
}

const TREE_SEED: u64 = 20260731;

/// Test-only scene placement: put the ordinary spawn kit south of a real
/// tree. The controller receives no target address or privileged World.
fn tree_scene() -> (f32, f32) {
    use sim_core::terrain::{self, Occupant, ScatterTable};
    let haven = terrain::haven(TREE_SEED);
    let table = ScatterTable::alpha_default();
    for cz in 40..216 {
        for cx in 40..216 {
            let slot = terrain::scatter(TREE_SEED, &table, &haven, cx, cz);
            if slot.occupant != Occupant::Tree {
                continue;
            }
            let at = (slot.x, slot.z - 5.0);
            let floor = terrain::ground(TREE_SEED, &haven, at.0, at.1);
            if floor > 1.0 && (floor - slot.y).abs() < 0.3 {
                return at;
            }
        }
    }
    panic!("fixture seed has no tree with a level approach");
}

#[test]
fn collector_finds_walks_and_fells_using_wire_observations_without_frame_allocations() {
    use protocol::{encode_input, InputDatagram, Welcome};
    use server::core::{Lane, ShardCore};
    use server::stats::ShardStats;
    let content = content::Content::load_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
    )
    .unwrap();
    let start = tree_scene();
    let make = || {
        let tables = server::net::bake_all(&content).unwrap();
        let mut core = Box::new(ShardCore::new(TREE_SEED));
        core.world.gather = tables.gather;
        core.world.spawn_kit = tables.spawn_kit;
        core.world.dev_spawn = Some(start);
        core.catalog = tables.catalog;
        assert!(core.connect(0, 256));
        core
    };
    let mut shard = make();
    let mut replay = make();
    let stats = ShardStats::default();
    let mut bot = Gatherer::new(Driver::new(Forward, THINK_INTERVAL, REQUEST_TIMEOUT).unwrap());
    bot.welcome(&Welcome {
        seed: TREE_SEED,
        player_id: 256,
        tick: 0,
        dev: true,
    });
    let mut view = ClientView::new();
    let mut moved = false;
    let mut primary = false;
    let mut bytes = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
    for seq in 1..=1500u16 {
        ALLOCS.with(|count| count.set(Some(0)));
        let frame = bot.frame(&view, 256, seq);
        let ops = ALLOCS.with(|count| count.replace(None).unwrap());
        assert_eq!(ops, 0, "gather controller touched the frame allocator");
        moved |= frame.move_z != 0;
        primary |= frame.buttons & sim_core::input::BTN_PRIMARY != 0;
        assert_eq!(
            frame.buttons
                & !(BTN_JUMP | sim_core::input::BTN_PRIMARY | sim_core::input::BTN_SPRINT),
            0
        );
        let (ack, ack_bits) = view.ack_fields();
        let mut input = InputDatagram::new(ack, ack_bits, sim_core::limits::INTERP_DELAY_TICKS);
        input.push(frame).unwrap();
        let len = encode_input(&input, &mut bytes).unwrap();
        let decoded = protocol::decode_input(&bytes[..len]).unwrap();
        shard.push_input(0, &decoded);
        replay.push_input(0, &decoded);
        shard.tick_bare(&stats, |lane, _, bytes| {
            match lane {
                Lane::Snapshot => {
                    view.apply(bytes).unwrap();
                }
                Lane::Event => bot.event(bytes).unwrap(),
            }
            true
        });
        replay.tick_bare(&stats, |_, _, _| true);
        assert_eq!(shard.world.state_hash(), replay.world.state_hash());
        if bot.stats.targets_completed > 0 {
            break;
        }
    }
    assert!(moved && primary, "the bot must approach and then swing");
    assert!(bot.stats.wood_gained > 0, "{:?}", bot.stats);
    assert!(
        bot.stats.gather_awards > 0 && bot.stats.targets_completed > 0,
        "{:?}",
        bot.stats
    );
    let wood = content.item_index("item.wood").unwrap();
    let actual: u32 = shard
        .world
        .players
        .iter()
        .find(|p| p.active && p.id == 256)
        .unwrap()
        .inv
        .iter()
        .filter(|s| s.item == wood)
        .map(|s| u32::from(s.count))
        .sum();
    assert_eq!(
        bot.stats.wood, actual,
        "success is the server's inventory, not a predicted swing"
    );
    // Exercise the retreat's additional perception ray under the same
    // allocator wall, after the ordinary harvesting episode has completed.
    let mut pursuer = *view.get(256).unwrap();
    pursuer.id = 257;
    pursuer.qz += sim_core::movement::quant_xz(1.0);
    view.entities.push((257, pursuer));
    let n = protocol::event::encode_event_hurt(0, 15, &mut bytes).unwrap();
    bot.event(&bytes[..n]).unwrap();
    ALLOCS.with(|count| count.set(Some(0)));
    let frame = bot.frame(&view, 256, 1501);
    let ops = ALLOCS.with(|count| count.replace(None).unwrap());
    assert_eq!(ops, 0, "retreat perception touched the frame allocator");
    assert_eq!(frame.buttons, sim_core::input::BTN_SPRINT);
    assert_eq!(frame.move_z, -127);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collector_receives_inventory_and_gathers_over_real_webtransport() {
    let content = content::Content::load_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
    )
    .unwrap();
    let mut config = server::config::ShardConfig::ephemeral(TREE_SEED);
    config.dev_spawn = Some(tree_scene());
    let shard = server::net::spawn_shard(
        config,
        server::net::bake_all(&content).unwrap(),
        server::store::Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .unwrap();
    let endpoint = bot_endpoint().unwrap();
    let mut bot = Gatherer::new(Driver::new(Forward, THINK_INTERVAL, REQUEST_TIMEOUT).unwrap());
    let result = run_driven_bot(
        &endpoint,
        shard.local_addr,
        Duration::from_secs(8),
        &mut bot,
    )
    .await;
    shard.shutdown.store(true, Ordering::Relaxed);
    endpoint.close(0u32.into(), b"test done");
    let report = result.unwrap();
    assert!(!report.walk_truncated);
    assert!(report.snapshots_applied > 0 && report.events_received > 0);
    assert_eq!(report.decode_errors + report.event_decode_errors, 0);
    assert!(
        bot.stats.wood_gained > 0 && bot.stats.gather_awards > 0,
        "{:?}",
        bot.stats
    );
}

struct Forward;
impl DecisionSource for Forward {
    fn decide(&mut self, _: Observation) -> Result<Decision, String> {
        Ok(Decision {
            action: Action::Forward,
            confidence: 1.0,
            input_tokens: 0,
        })
    }
}
struct Recorded {
    driver: Driver,
    frames: Vec<InputFrame>,
}
impl BotDriver for Recorded {
    fn frame(&mut self, view: &ClientView, player: u32, seq: u16) -> InputFrame {
        let frame = self.driver.frame(view, player, seq);
        assert!(self.frames.len() < self.frames.capacity());
        self.frames.push(frame);
        frame
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_moves_over_the_wire_and_its_inputs_replay_without_allocations() {
    let duration = Duration::from_secs(3);
    let content = content::Content::load_dir(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
    )
    .unwrap();
    let config = server::config::ShardConfig::ephemeral(20260731);
    let shard = server::net::spawn_shard(
        config,
        server::net::bake_all(&content).unwrap(),
        server::store::Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .unwrap();
    let endpoint = bot_endpoint().unwrap();
    let mut recorded = Recorded {
        driver: Driver::new(Forward, THINK_INTERVAL, REQUEST_TIMEOUT).unwrap(),
        frames: Vec::with_capacity(walk_ticks(duration) as usize),
    };
    let result = run_driven_bot(&endpoint, shard.local_addr, duration, &mut recorded).await;
    shard.shutdown.store(true, Ordering::Relaxed);
    endpoint.close(0u32.into(), b"test done");
    let report = result.unwrap();
    assert!(!report.walk_truncated);
    assert!(report.snapshots_applied > 0 && report.last_executed_seq > 0);
    assert_eq!(report.decode_errors, 0);
    assert_eq!(report.event_decode_errors, 0);
    assert!(recorded.driver.stats.decisions > 0);
    assert!(recorded.driver.stats.moving_frames > 0);
    assert_ne!(
        recorded.driver.stats.first_position, recorded.driver.stats.last_position,
        "the shard must actually move the body"
    );
    assert_eq!(
        report.actions_sent, 0,
        "peaceful movement has no action lane"
    );
    for frame in &recorded.frames {
        assert_eq!(
            frame.buttons & !BTN_JUMP,
            0,
            "strict subset of human buttons"
        );
        assert_eq!(frame.move_x, 0);
        assert!([0, 127].contains(&frame.move_z));
    }

    let mut a = World::new(20260731);
    let mut b = World::new(20260731);
    for world in [&mut a, &mut b] {
        world.tick(&[Command::Join { id: 1 }]);
        world.tick(&[]); // Construction and warmup stay outside the count.
    }
    for frame in recorded.frames {
        let commands = [Command::Input {
            id: 1,
            frame,
            favour: 0,
        }];
        ALLOCS.with(|count| count.set(Some(0)));
        a.tick(&commands);
        b.tick(&commands);
        let allocations = ALLOCS.with(|count| count.replace(None).unwrap());
        assert_eq!(allocations, 0, "model-origin inputs allocated in the sim");
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "same agent inputs must replay"
        );
    }
}
