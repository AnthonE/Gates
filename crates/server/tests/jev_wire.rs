//! An agent's decisions become ordinary wire inputs. The captured input
//! trace must also replay identically without allocations inside the sim.
use server::botclient::{bot_endpoint, run_driven_bot, walk_ticks, BotDriver};
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
