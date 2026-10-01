//! One bot playing its own session at the shard's tick rate (`botclient`),
//! and Gates' renderer drawing it from a spectator seat that follows it
//! (`NETCODE.md` §2.3). The read-only page receives captured frames; it never
//! opens a game session.
//!
//! **The bot never waits on a frame.** It used to drive the window's own
//! session, one decision per rendered frame, and on the software renderer a
//! frame takes most of a second: input held that long carries a body metres
//! past a 0.25 m build stand, so the bot walked back and forth across its
//! stand for a whole build goal and placed nothing. The agent's stands,
//! doors and hands want every tick (the lockstep walls run it at 30 Hz; it
//! builds at 5 Hz and stalls at 3).
pub mod http;

use crate::botclient::BotDriver;
use crate::explorer::{Phase, Survivor};
use crate::mind::{Goal, History, Mode, Name, Reason, SourceKind};
use crate::view::ClientView;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use client::render::{Net, Screen};
use sim_core::input::InputFrame;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Experimental broadcast defaults, registered in DECISIONS.md.
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;
pub const FRAME_INTERVAL: Duration = Duration::from_millis(250);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
/// Pack entries the page lists.
pub const PAGE_ITEMS: usize = 6;
/// Ticks between page records. A capture takes one at most every
/// `FRAME_INTERVAL`, which is 7.5 ticks.
const RECORD_TICKS: u16 = 4;
/// Page records in flight from the bot to the window: about four seconds of
/// them, drained every rendered frame. A full ring skips a record; the
/// window keeps the newest it has.
const RECORDS: usize = 32;

/// Run until the connection fails: death is answered in-game, so the
/// supervisor only restarts a run whose session or renderer ended.
#[derive(Resource)]
pub struct Continuous;

const RUNNING: u8 = 0;
const FINISHED: u8 = 1;
const FAILED: u8 = 2;

/// How the bot's run stands, shared with the window.
#[derive(Default)]
pub struct Live {
    joined: AtomicBool,
    end: AtomicU8,
}

impl Live {
    /// The bot has played a tick in the world.
    pub fn joined(&self) -> bool {
        self.joined.load(Ordering::Relaxed)
    }

    /// The run is over: its time ran out (`ok`) or its session failed.
    pub fn finish(&self, ok: bool) {
        self.end
            .store(if ok { FINISHED } else { FAILED }, Ordering::Relaxed);
    }

    /// `Some(ok)` once the run is over.
    pub fn ended(&self) -> Option<bool> {
        match self.end.load(Ordering::Relaxed) {
            RUNNING => None,
            end => Some(end == FINISHED),
        }
    }
}

/// The bot's driver and the window's hold on it: a ring of page records
/// between them, and the run's state.
pub fn pair(survivor: Survivor) -> (Watched, Bot) {
    let live = Arc::new(Live::default());
    let (records, inbox) = rtrb::RingBuffer::new(RECORDS);
    (
        Watched {
            survivor,
            live: live.clone(),
            records,
            playing: None,
        },
        Bot {
            live,
            inbox,
            latest: None,
            created: Instant::now(),
            drawn: false,
        },
    )
}

/// The survivor as `botclient` drives it, once a tick, sending the page
/// record as it plays.
pub struct Watched {
    pub survivor: Survivor,
    pub live: Arc<Live>,
    records: rtrb::Producer<Status>,
    playing: Option<Instant>,
}

impl BotDriver for Watched {
    fn welcome(&mut self, welcome: &protocol::Welcome) {
        self.survivor.welcome(welcome);
    }

    fn event(&mut self, bytes: &[u8]) -> Result<(), protocol::WireError> {
        self.survivor.event(bytes)
    }

    fn frame(&mut self, view: &ClientView, player_id: u32, seq: u16) -> InputFrame {
        let frame = self.survivor.frame(view, player_id, seq);
        let playing = *self.playing.get_or_insert_with(Instant::now);
        self.live.joined.store(true, Ordering::Relaxed);
        if seq.is_multiple_of(RECORD_TICKS) {
            if let Some(status) = status(&self.survivor, view, player_id, playing) {
                let _ = self.records.push(status);
            }
        }
        frame
    }

    fn action(&mut self, out: &mut [u8]) -> Option<usize> {
        self.survivor.action(out)
    }
}

/// The page record: the survivor's own client state and its controller's
/// record. `None` before the welcome.
pub fn status(s: &Survivor, view: &ClientView, player: u32, playing: Instant) -> Option<Status> {
    let core = s.core()?;
    let now = Instant::now();
    let mut items = [(Name::EMPTY, 0u32); PAGE_ITEMS];
    let mut items_len = 0u8;
    if let Some(summary) = s.summary(view, player) {
        for (name, count) in summary.items().iter().take(PAGE_ITEMS) {
            items[items_len as usize] = (*name, *count);
            items_len += 1;
        }
    }
    let goal_secs = s.goal_ticks().map_or(0, |t| t / sim_core::limits::TICK_HZ);
    let (hour, hour_cap, day, day_cap) = s.mind.guard().counts();
    Some(Status {
        source: s.mind.kind(),
        mode: s.mind.mode(now),
        phase: if core.dead {
            Phase::Dead
        } else if core.wounded {
            Phase::Wounded
        } else {
            s.stats.phase
        },
        goal: s.goal(),
        goal_secs,
        reason: s.mind.last.map_or(Reason::EMPTY, |c| c.reason),
        history: s.history,
        seconds: playing.elapsed().as_secs(),
        hp: core.hp,
        hp_max: core.hp_max,
        food: (core.food, core.max_food),
        water: (core.water, core.max_water),
        items,
        items_len,
        wood: s.held("Wood"),
        deaths: s.stats.deaths,
        respawns: s.stats.respawns,
        trees: s.stats.targets_completed,
        crafted: s.stats.crafted,
        retreats: s.stats.retreats,
        requests: s.mind.stats.requests,
        decisions: s.mind.stats.decisions,
        failures: s.mind.stats.failures,
        input_tokens: s.mind.stats.input_tokens,
        output_tokens: s.mind.stats.output_tokens,
        hour: (hour, hour_cap),
        day: (day, day_cap),
        decode_errors: core.decode_errors + core.event_errors,
    })
}

/// Everything the page may show about one captured frame: received state
/// and the controller's own record. No key, prompt or response body.
#[derive(Clone, Copy)]
pub struct Status {
    pub source: SourceKind,
    pub mode: Mode,
    pub phase: Phase,
    pub goal: Option<Goal>,
    pub goal_secs: u32,
    pub reason: Reason,
    pub history: History,
    pub seconds: u64,
    pub hp: u16,
    pub hp_max: u16,
    pub food: (u16, u16),
    pub water: (u16, u16),
    pub items: [(Name, u32); PAGE_ITEMS],
    pub items_len: u8,
    pub wood: u32,
    pub deaths: u64,
    pub respawns: u64,
    pub trees: u64,
    pub crafted: u64,
    pub retreats: u64,
    pub requests: u64,
    pub decisions: u64,
    pub failures: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub hour: (u32, u32),
    pub day: (u32, u32),
    pub decode_errors: u64,
}

impl Status {
    pub fn action(&self) -> &'static str {
        self.phase.text()
    }

    /// The controller badge: which source decides, and whether it can.
    pub fn mode_line(&self) -> String {
        format!("{} · {}", self.source.label(), self.mode.text())
    }

    pub fn items(&self) -> &[(Name, u32)] {
        &self.items[..self.items_len as usize]
    }
}

pub struct Captured {
    pub image: Image,
    pub status: Status,
    pub at: Instant,
    pub id: u64,
}

pub struct Capture {
    pub frames: rtrb::Producer<Captured>,
    next: Instant,
    pending: bool,
    id: u64,
}

impl Capture {
    pub fn new(frames: rtrb::Producer<Captured>) -> Self {
        Self {
            frames,
            next: Instant::now(),
            pending: false,
            id: 0,
        }
    }
}

/// Nobody supplies gameplay input in the broadcast window: it is a seat.
pub fn clear_human_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
) {
    keys.reset_all();
    mouse.reset_all();
    motion.delta = Vec2::ZERO;
}

/// The seat's "SPECTATING" label is for a person who chose a seat; this
/// window is the bot's own view, and the page around it says whose.
pub fn hide_seat_label(
    mut labels: Query<&mut Node, With<client::render::spectate::SpectateLabel>>,
) {
    for mut node in &mut labels {
        node.display = Display::None;
    }
}

/// One line for a finished run: the survivor's own record.
pub fn report(s: &Survivor) -> String {
    format!(
        "broadcast survival: {:?}; mind {:?}; gathered wood {} stone {} ore {} cloth {}",
        s.stats,
        s.mind.stats,
        s.gathered_of("Wood"),
        s.gathered_of("Stone"),
        s.gathered_of("Metal Ore") + s.gathered_of("Sulfur Ore"),
        s.gathered_of("Cloth"),
    )
}

/// The window's hold on the run: the bot's page records as they arrive,
/// and whether the renderer has drawn the world yet.
pub struct Bot {
    pub live: Arc<Live>,
    inbox: rtrb::Consumer<Status>,
    latest: Option<Status>,
    created: Instant,
    drawn: bool,
}

impl Bot {
    /// The newest page record; `None` before the bot's first.
    pub fn latest(&mut self) -> Option<Status> {
        while let Ok(status) = self.inbox.pop() {
            self.latest = Some(status);
        }
        self.latest
    }
}

pub fn lifetime(
    mut bot: NonSendMut<Bot>,
    continuous: Option<Res<Continuous>>,
    screen: Res<State<Screen>>,
    net: Option<NonSend<Net>>,
    mut exit: MessageWriter<AppExit>,
) {
    bot.drawn |= matches!(screen.get(), Screen::InWorld | Screen::Dead);
    match bot.live.ended() {
        // A timed run's clock ran out.
        Some(true) if continuous.is_none() => {
            if let Some(net) = net {
                println!(
                    "broadcast snapshots: {}, decode errors: {}",
                    net.session.core.snapshots_applied,
                    net.session.core.decode_errors + net.session.core.event_errors
                );
            }
            exit.write(AppExit::Success);
        }
        Some(_) => {
            eprintln!("jev-watch: the bot's connection ended");
            exit.write(AppExit::error());
        }
        None if matches!(screen.get(), Screen::Menu | Screen::Disconnected)
            || net.as_ref().is_some_and(|n| n.session.closed()) =>
        {
            eprintln!("jev-watch: the window's spectator seat ended");
            exit.write(AppExit::error());
        }
        None if !bot.drawn && bot.created.elapsed() > STARTUP_TIMEOUT => {
            eprintln!("jev-watch: the renderer did not finish loading");
            exit.write(AppExit::error());
        }
        None => {}
    }
}

pub fn capture(
    mut commands: Commands,
    mut capture: NonSendMut<Capture>,
    mut bot: NonSendMut<Bot>,
    mut exit: MessageWriter<AppExit>,
) {
    if capture.frames.is_abandoned() {
        eprintln!("jev-watch: the broadcast worker stopped");
        exit.write(AppExit::error());
        return;
    }
    if capture.pending || capture.frames.is_full() || Instant::now() < capture.next {
        return;
    }
    let Some(status) = bot.latest() else {
        return;
    };
    capture.pending = true;
    capture.next = Instant::now() + FRAME_INTERVAL;
    capture.id = capture.id.saturating_add(1);
    let id = capture.id;
    let at = Instant::now();
    commands.spawn(Screenshot::primary_window()).observe(
        move |mut event: On<ScreenshotCaptured>, mut capture: NonSendMut<Capture>| {
            capture.pending = false;
            // Transfer the readback to the encoder; no image encoding or HTTP
            // runs on the control/render thread. One readback and one queued
            // image are the complete bound, shared by all viewers.
            let image = std::mem::take(&mut event.image);
            let _ = capture.frames.push(Captured {
                image,
                status,
                at,
                id,
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::{Mind, MindConfig, Scripted};

    fn scripted() -> Survivor {
        Survivor::new(Mind::new(Scripted::default(), MindConfig::default()).unwrap())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_bot_plays_its_own_session_and_keeps_the_page_record() {
        use sim_core::terrain::{self, Occupant, ScatterTable};
        let seed = 20260731;
        let haven = terrain::haven(seed);
        let table = ScatterTable::alpha_default();
        // Fixture placement belongs to the shard. The controller has only
        // the same Welcome, snapshots and reliable messages as a player.
        let at = (40..216)
            .find_map(|cz| {
                (40..216).find_map(|cx| {
                    let slot = terrain::scatter(seed, &table, &haven, cx, cz);
                    let at = (slot.x, slot.z - 5.0);
                    let floor = terrain::ground(seed, &haven, at.0, at.1);
                    (slot.occupant == Occupant::Tree && floor > 1.0 && (floor - slot.y).abs() < 0.3)
                        .then_some(at)
                })
            })
            .expect("fixture seed has a tree with a level approach");
        let content = content::Content::load_dir(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content"),
        )
        .unwrap();
        let mut config = crate::config::ShardConfig::ephemeral(seed);
        config.dev_spawn = Some(at);
        let shard = crate::net::spawn_shard(
            config,
            crate::net::bake_all(&content).unwrap(),
            crate::store::Saves::off(),
            crate::worldfile::WorldBoot::off(),
        )
        .await
        .unwrap();
        let server = shard.local_addr.to_string();
        let endpoint = crate::botclient::agent_endpoint(&server, Some(&shard.cert_hash)).unwrap();
        let (mut watched, mut bot) = pair(scripted());
        let name = protocol::Name::new("jev").unwrap();
        // The bot plays its own session; the page record is all the window
        // reads of it.
        let record = tokio::select! {
            result = crate::botclient::run_guest_agent(
                &endpoint,
                shard.local_addr,
                name,
                Duration::from_secs(120),
                &mut watched,
            ) => panic!("the run ended before a harvest: {result:?}"),
            record = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if let Some(s) = bot.latest().filter(|s| s.wood > 0) {
                        break s;
                    }
                }
            } => record,
        };
        shard.shutdown.store(true, Ordering::Relaxed);
        assert!(watched.survivor.gathered_of("Wood") > 0);
        assert!(record
            .items()
            .iter()
            .any(|(n, c)| n.as_str() == "Wood" && *c == record.wood));
        assert!(record.seconds < 60 && record.hp > 0);
        assert_eq!(record.decode_errors, 0);
        assert!(shard.stats.input_dg_ok.load(Ordering::Relaxed) > 0);
    }

    #[tokio::test]
    async fn observer_failure_closes_a_real_session_and_stops_both_output_lanes() {
        let shard = crate::agent_demo::spawn_local().await.unwrap();
        let server = shard.local_addr.to_string();
        let endpoint = client::client_endpoint(&server, Some(&shard.cert_hash)).unwrap();
        let mut session =
            client::Session::connect(&endpoint, &server, protocol::Address::GUEST, |_, _, _| None)
                .await
                .unwrap();
        session.observe_events(|_| false).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !session.closed() {
                session.pump(0.0);
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        session.core.set_input(0, 0, 128, 0, 127, 0);
        session.pump(100.0);
        assert_eq!(session.send_action(&[1]), Err(client::SendError::Closed));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(shard.stats.input_dg_ok.load(Ordering::Relaxed), 0);
        shard.shutdown.store(true, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn continuous_runs_outlive_death_and_the_timer_but_not_a_lost_session() {
        use bevy::ecs::system::RunSystemOnce;

        let shard = crate::agent_demo::spawn_local().await.unwrap();
        let server = shard.local_addr.to_string();
        let endpoint = client::client_endpoint(&server, Some(&shard.cert_hash)).unwrap();
        let session =
            client::Session::connect(&endpoint, &server, protocol::Address::GUEST, |_, _, _| None)
                .await
                .unwrap();
        let mut world = World::new();
        world.insert_resource(State::new(Screen::InWorld));
        world.init_resource::<Messages<AppExit>>();
        world.insert_resource(Continuous);
        world.insert_non_send_resource(pair(scripted()).1);
        world.insert_non_send_resource(Net {
            session,
            sel: 0,
            light: false,
        });
        let exits = |world: &mut World| {
            world
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .collect::<Vec<_>>()
        };

        world.run_system_once(lifetime).unwrap();
        assert!(exits(&mut world).is_empty());

        // Death is answered in-game: a continuous run keeps its island.
        world.insert_resource(State::new(Screen::Dead));
        world.run_system_once(lifetime).unwrap();
        assert!(exits(&mut world).is_empty());

        // The finite CLI mode ends when the bot's time is up; a continuous
        // run's bot never finishes, so one that did is restarted.
        world.insert_resource(State::new(Screen::InWorld));
        world.non_send_resource::<Bot>().live.finish(true);
        world.run_system_once(lifetime).unwrap();
        assert_eq!(exits(&mut world), [AppExit::error()]);
        world.remove_resource::<Continuous>();
        world.run_system_once(lifetime).unwrap();
        assert_eq!(exits(&mut world), [AppExit::Success]);

        // A bot whose session failed ends either kind of run.
        world.insert_non_send_resource(pair(scripted()).1);
        world.non_send_resource::<Bot>().live.finish(false);
        world.run_system_once(lifetime).unwrap();
        assert_eq!(exits(&mut world), [AppExit::error()]);

        // So does the window's own seat.
        world.insert_resource(Continuous);
        world.insert_non_send_resource(pair(scripted()).1);
        world.insert_resource(State::new(Screen::Disconnected));
        world.run_system_once(lifetime).unwrap();
        assert_eq!(exits(&mut world), [AppExit::error()]);

        // Continuous mode must not hide a renderer that never finishes loading.
        world.insert_resource(State::new(Screen::Loading));
        let mut bot = pair(scripted()).1;
        bot.created = Instant::now() - STARTUP_TIMEOUT - Duration::from_secs(1);
        world.insert_non_send_resource(bot);
        world.run_system_once(lifetime).unwrap();
        assert_eq!(exits(&mut world), [AppExit::error()]);
        shard.shutdown.store(true, Ordering::Relaxed);
    }
}
