//! An agent's goals become ordinary wire inputs and actions. The decision
//! worker never touches the frame loop's allocator, and a survivor driven
//! over real WebTransport gathers from the inventory the shard announces.
use server::botclient::{bot_endpoint, run_driven_bot};
use server::explorer::Survivor;
use server::mind::{
    Choice, DecisionSource, Goal, Mind, MindConfig, Reason, Scripted, SourceKind, Summary,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

thread_local! { static ALLOCS: Cell<Option<usize>> = const { Cell::new(None) }; }
struct CountAlloc;
fn note() {
    let _ = ALLOCS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

/// Alternates a goal with an allocated error, which must be dropped on the
/// worker and never on the input loop.
struct Alternating(bool);
impl DecisionSource for Alternating {
    fn kind(&self) -> SourceKind {
        SourceKind::Jev
    }
    fn decide(&mut self, s: &Summary) -> Result<Choice, String> {
        self.0 = !self.0;
        if self.0 {
            Ok(Choice {
                goal: s.options()[0],
                confidence: 1.0,
                reason: Reason::from_text("alternating"),
                input_tokens: 7,
                output_tokens: 1,
                say: server::mind::Say::Quiet,
                code: None,
            })
        } else {
            Err("an allocated error must be dropped on the worker".to_owned())
        }
    }
}

#[test]
fn worker_handoffs_never_touch_the_frame_loops_allocator() {
    let cfg = MindConfig::default();
    let mut mind = Mind::new(Alternating(false), cfg).unwrap();
    let mut summary = Summary::EMPTY;
    summary.offer(Goal::Explore);
    summary.offer(Goal::Wait);
    // A synthetic clock keeps the once-per-second floor honest without
    // making the test wait for it; the worker runs in real time.
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(60);
    let mut step = 0u32;
    while mind.stats.decisions + mind.stats.failures < 64 {
        let now = t0 + cfg.interval * 4 * step;
        step += 1;
        ALLOCS.with(|count| count.set(Some(0)));
        let asked = mind.ask(now, &summary);
        let asks = ALLOCS.with(|count| count.replace(None).unwrap());
        assert_eq!(asks, 0, "a request allocated on the input loop");
        assert!(asked, "the floor and backoff had passed; the mind must ask");
        loop {
            ALLOCS.with(|count| count.set(Some(0)));
            let answer = mind.poll(now);
            let polls = ALLOCS.with(|count| count.replace(None).unwrap());
            assert_eq!(polls, 0, "an answer or error was freed on the input loop");
            if answer.is_some() || !mind.pending() {
                break;
            }
            assert!(Instant::now() < deadline, "the worker stopped answering");
            std::thread::yield_now();
        }
    }
    assert!(mind.stats.decisions > 0 && mind.stats.failures > 0);
    assert_eq!(mind.stats.input_tokens, 7 * mind.stats.decisions);
}

const TREE_SEED: u64 = 20260731;

/// Test-only scene placement: put the ordinary spawn kit south of a real
/// tree. The controller receives no target address or privileged World.
fn tree_scene() -> (f32, f32) {
    use sim_core::terrain::{self, ScatterTable};
    let haven = terrain::haven(TREE_SEED);
    let table = ScatterTable::alpha_default();
    // Its box first, then the island: mild ground wherever it lies.
    let hint = (40..216).flat_map(|cz| (40..216).map(move |cx| (cx, cz)));
    for (cx, cz) in sim_core::find::mild_cells(TREE_SEED, hint) {
        let slot = terrain::scatter(TREE_SEED, &table, &haven, cx, cz);
        // A grown tree that pays its row: a sapling is no wood in the time
        // this run has.
        if !sim_core::find::row_tree(TREE_SEED, cx, cz, &slot) {
            continue;
        }
        let at = (slot.x, slot.z - 5.0);
        let floor = terrain::ground(TREE_SEED, &haven, at.0, at.1);
        if floor > 1.0 && (floor - slot.y).abs() < 0.3 {
            return at;
        }
    }
    panic!("fixture seed has no tree with a level approach");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_survivor_gathers_over_real_webtransport_from_announced_inventory() {
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
    let mut bot = Survivor::new(Mind::new(Scripted::default(), MindConfig::default()).unwrap());
    let result = run_driven_bot(
        &endpoint,
        shard.local_addr,
        Duration::from_secs(20),
        &mut bot,
    )
    .await;
    shard.shutdown.store(true, Ordering::Relaxed);
    endpoint.close(0u32.into(), b"test done");
    let report = result.unwrap();
    assert!(!report.walk_truncated);
    assert!(report.snapshots_applied > 0 && report.events_received > 0);
    assert_eq!(report.decode_errors + report.event_decode_errors, 0);
    assert!(bot.mind.stats.decisions > 0, "{:?}", bot.mind.stats);
    assert!(
        bot.gathered_of("Wood") > 0 && bot.stats.gather_awards > 0,
        "{:?} {:?}",
        bot.stats,
        bot.history.iter().collect::<Vec<_>>()
    );
    assert!(
        bot.mind.stats.requests <= 20 + 1,
        "never more than one request a second: {:?}",
        bot.mind.stats
    );
}

/// Fells a tree, then crafts whatever the wood allows: the craft travels
/// the ordinary action lane and its completion comes back on the event lane.
/// It says one line on its first decision, which travels the chat lane and
/// comes back to it as any player's line does.
#[derive(Default)]
struct Crafty {
    spoke: bool,
}
impl DecisionSource for Crafty {
    fn kind(&self) -> SourceKind {
        SourceKind::External
    }
    fn decide(&mut self, s: &Summary) -> Result<Choice, String> {
        let goal = s
            .options()
            .iter()
            .copied()
            .find(|g| matches!(g, Goal::Craft(_)))
            .or_else(|| s.offers(Goal::GatherWood).then_some(Goal::GatherWood))
            .unwrap_or(Goal::Explore);
        Ok(Choice {
            goal,
            confidence: 1.0,
            reason: Reason::from_text("test: craft when possible"),
            input_tokens: 0,
            output_tokens: 0,
            say: if std::mem::replace(&mut self.spoke, true) {
                server::mind::Say::Quiet
            } else {
                server::mind::Say::Line(
                    protocol::ChatText::sanitize(b"hello island")
                        .unwrap()
                        .into(),
                )
            },
            code: None,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actions_ride_the_ordinary_action_lane_and_their_verdicts_return() {
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
    let mut bot = Survivor::new(Mind::new(Crafty::default(), MindConfig::default()).unwrap());
    let result = run_driven_bot(
        &endpoint,
        shard.local_addr,
        Duration::from_secs(45),
        &mut bot,
    )
    .await;
    shard.shutdown.store(true, Ordering::Relaxed);
    endpoint.close(0u32.into(), b"test done");
    let report = result.unwrap();
    assert!(!report.walk_truncated);
    assert!(report.actions_sent > 0, "{report:?}");
    assert_eq!(report.actions_sent, bot.stats.actions);
    assert!(
        report.chats_sent == 1 && bot.stats.said == 1 && bot.stats.echoed == 1,
        "its line went out and came back through the shard's chat: {report:?} {:?}",
        bot.stats
    );
    assert!(
        bot.stats.crafted > 0,
        "a craft's completion came back over the event lane: {:?} {:?}",
        bot.stats,
        bot.history.iter().collect::<Vec<_>>()
    );
}
