//! One ordinary player session, driven by the bot and drawn by Gates' renderer.
//! The read-only page receives captured frames; it never opens a game session.
pub mod http;

use crate::botclient::BotDriver;
use crate::explorer::{Phase, Survivor};
use crate::mind::{Goal, History, Mode, Name, Reason, SourceKind};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use client::render::{input, Net, Screen};
use protocol::{MAX_EVENT_MSG_BYTES, MAX_STREAM_MSG_BYTES};
use sim_core::input::InputFrame;
use sim_core::limits::EVENT_RING_CAP;
use std::time::{Duration, Instant};

// Experimental broadcast defaults, registered in DECISIONS.md.
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;
pub const FRAME_INTERVAL: Duration = Duration::from_millis(250);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
/// Pack entries the page lists.
pub const PAGE_ITEMS: usize = 6;

type Event = ([u8; MAX_EVENT_MSG_BYTES], usize);

/// Run until the connection fails: death is answered in-game, so the
/// supervisor only restarts a run whose session or renderer ended.
#[derive(Resource)]
pub struct Continuous;

pub struct Controller {
    pub survivor: Survivor,
    events: rtrb::Consumer<Event>,
    held: InputFrame,
    last_tick: Option<u32>,
    created: Instant,
    playing: Option<Instant>,
    duration: Duration,
    /// An action the session's bounded lane refused as full; retried each
    /// tick, never dropped (the renderer's own panels do the same).
    unsent: Option<([u8; MAX_STREAM_MSG_BYTES], usize)>,
}

impl Controller {
    pub fn attach(
        session: &mut client::Session,
        mut survivor: Survivor,
        duration: Duration,
    ) -> Result<Self, String> {
        survivor.welcome(&session.welcome);
        let (mut tx, events) = rtrb::RingBuffer::new(EVENT_RING_CAP);
        session.observe_events(move |bytes| {
            if bytes.len() > MAX_EVENT_MSG_BYTES {
                return false;
            }
            let mut copy = [0; MAX_EVENT_MSG_BYTES];
            copy[..bytes.len()].copy_from_slice(bytes);
            tx.push((copy, bytes.len())).is_ok()
        })?;
        Ok(Self {
            survivor,
            events,
            held: InputFrame {
                pitch: 128,
                ..Default::default()
            },
            last_tick: None,
            created: Instant::now(),
            playing: None,
            duration,
            unsent: None,
        })
    }

    /// Bounded reliable handoff, followed by at most one new control sample
    /// and one action per advanced client tick. The core retains taps until
    /// a tick consumes them, so a faster renderer cannot lose a jump.
    pub fn sample(&mut self, session: &client::Session, ready: bool) -> Result<InputFrame, String> {
        for _ in 0..EVENT_RING_CAP {
            let Ok((bytes, n)) = self.events.pop() else {
                break;
            };
            self.survivor
                .event(&bytes[..n])
                .map_err(|_| "bot rejected an observed event")?;
        }
        if session.closed() || self.events.is_abandoned() {
            return Err("the bot session or its event observer closed".into());
        }
        if !ready {
            return Ok(InputFrame {
                pitch: self.held.pitch,
                yaw: self.held.yaw,
                ..Default::default()
            });
        }
        self.playing.get_or_insert_with(Instant::now);
        let tick = session.core.clock.client_tick;
        if self.last_tick != Some(tick) {
            self.held =
                self.survivor
                    .frame(&session.core.view, session.welcome.player_id, tick as u16);
            self.last_tick = Some(tick);
            if self.unsent.is_none() {
                let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
                if let Some(n) = self.survivor.action(&mut buf) {
                    self.unsent = Some((buf, n));
                }
            }
            if let Some((buf, n)) = self.unsent {
                match session.send_action(&buf[..n]) {
                    Ok(()) => self.unsent = None,
                    Err(client::SendError::Full) => {}
                    Err(client::SendError::Closed) => {
                        return Err("the bot's action lane closed".into())
                    }
                    // A spectator seat cannot act (v73). The bot joins as a
                    // player, so this is a wiring fault: stop, never drop.
                    Err(client::SendError::ReadOnly) => {
                        return Err("the bot's session is a read-only seat".into())
                    }
                }
            }
        }
        Ok(self.held)
    }

    pub fn status(&self, session: &client::Session) -> Status {
        let s = &self.survivor;
        let now = Instant::now();
        let core = &session.core;
        let mut items = [(Name::EMPTY, 0u32); PAGE_ITEMS];
        let mut items_len = 0u8;
        if let Some(summary) = s.summary(&core.view, session.welcome.player_id) {
            for (name, count) in summary.items().iter().take(PAGE_ITEMS) {
                items[items_len as usize] = (*name, *count);
                items_len += 1;
            }
        }
        let goal_secs = s.goal_ticks().map_or(0, |t| t / sim_core::limits::TICK_HZ);
        let (hour, hour_cap, day, day_cap) = s.mind.guard().counts();
        Status {
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
            seconds: self.playing.map_or(0, |t| t.elapsed().as_secs()),
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
        }
    }
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

/// Only the bot supplies gameplay input in the broadcast window.
pub fn clear_human_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
) {
    keys.reset_all();
    mouse.reset_all();
    motion.delta = Vec2::ZERO;
}

pub fn drive(
    mut controller: NonSendMut<Controller>,
    mut net: NonSendMut<Net>,
    mut look: ResMut<input::Look>,
    screen: Res<State<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    // The death screen is played too: its answer is the bot's respawn verb.
    let ready = matches!(screen.get(), Screen::InWorld | Screen::Dead);
    let f = match controller.sample(&net.session, ready) {
        Ok(f) => f,
        Err(why) => {
            eprintln!("jev-watch: {why}");
            exit.write(AppExit::error());
            return;
        }
    };
    net.session
        .core
        .set_input(f.buttons, f.yaw, f.pitch, f.move_x, f.move_z, f.sel);
    net.sel = f.sel;
    net.light = false;
    look.yaw = f32::from(f.yaw) * std::f32::consts::TAU / 65536.0;
    look.pitch = (f32::from(f.pitch) / 255.0 - 0.5) * std::f32::consts::PI;
    look.free_yaw = 0.0;
    look.free_pitch = 0.0;
    look.frozen = true;
}

/// One line for a finished run: the survivor's own record.
pub fn report(controller: &Controller) -> String {
    let s = &controller.survivor;
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

pub fn lifetime(
    controller: NonSend<Controller>,
    continuous: Option<Res<Continuous>>,
    screen: Res<State<Screen>>,
    net: Option<NonSend<Net>>,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(screen.get(), Screen::Menu | Screen::Disconnected)
        || net.as_ref().is_some_and(|n| n.session.closed())
    {
        eprintln!("jev-watch: the bot's connection ended");
        println!("{}", report(&controller));
        exit.write(AppExit::error());
    } else if continuous.is_none()
        && controller
            .playing
            .is_some_and(|t| t.elapsed() >= controller.duration)
    {
        println!("{}", report(&controller));
        if let Some(net) = net {
            println!(
                "broadcast snapshots: {}, decode errors: {}",
                net.session.core.snapshots_applied,
                net.session.core.decode_errors + net.session.core.event_errors
            );
        }
        exit.write(AppExit::Success);
    } else if controller.playing.is_none() && controller.created.elapsed() > STARTUP_TIMEOUT {
        eprintln!("jev-watch: the renderer did not finish loading");
        exit.write(AppExit::error());
    }
}

pub fn capture(
    mut commands: Commands,
    mut capture: NonSendMut<Capture>,
    controller: NonSend<Controller>,
    net: NonSend<Net>,
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
    capture.pending = true;
    capture.next = Instant::now() + FRAME_INTERVAL;
    capture.id = capture.id.saturating_add(1);
    let id = capture.id;
    let at = Instant::now();
    let status = controller.status(&net.session);
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
    use std::sync::atomic::Ordering;

    fn scripted() -> Survivor {
        Survivor::new(Mind::new(Scripted::default(), MindConfig::default()).unwrap())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rendered_session_controls_and_observes_an_actual_harvest() {
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
        let wood = content.item_index("item.wood").unwrap();
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
        let endpoint = client::client_endpoint(&server, Some(&shard.cert_hash)).unwrap();
        let mut session =
            client::Session::connect(&endpoint, &server, protocol::Address::GUEST, |_, _, _| None)
                .await
                .unwrap();
        let mut controller =
            Controller::attach(&mut session, scripted(), Duration::from_secs(120)).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(30), async {
            let mut cadence = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
            let mut last = Instant::now();
            let mut moved = false;
            let mut swung = false;
            loop {
                cadence.tick().await;
                let frame = controller.sample(&session, true).unwrap();
                // A second draw at the same client tick must hold the input,
                // not consume another decision or advance the search scan.
                assert_eq!(frame, controller.sample(&session, true).unwrap());
                moved |= frame.move_z != 0;
                swung |= frame.buttons & sim_core::input::BTN_PRIMARY != 0;
                session.core.set_input(
                    frame.buttons,
                    frame.yaw,
                    frame.pitch,
                    frame.move_x,
                    frame.move_z,
                    frame.sel,
                );
                let now = Instant::now();
                session.pump(now.duration_since(last).as_secs_f64() * 1000.0);
                last = now;
                if controller.survivor.gathered_of("Wood") > 0 {
                    let inventory: u32 = session
                        .core
                        .inv
                        .iter()
                        .filter(|s| s.item == wood)
                        .map(|s| u32::from(s.count))
                        .sum();
                    assert_eq!(controller.status(&session).wood, inventory);
                    assert!(moved && swung);
                    break;
                }
            }
        })
        .await;
        shard.shutdown.store(true, Ordering::Relaxed);
        assert!(
            outcome.is_ok(),
            "no harvest: {:?} {:?}",
            controller.survivor.stats,
            controller.survivor.history.iter().collect::<Vec<_>>()
        );
        assert_eq!(session.core.decode_errors + session.core.event_errors, 0);
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
        let mut session =
            client::Session::connect(&endpoint, &server, protocol::Address::GUEST, |_, _, _| None)
                .await
                .unwrap();
        let mut controller = Controller::attach(&mut session, scripted(), Duration::ZERO).unwrap();
        controller.playing = Some(Instant::now());
        let mut world = World::new();
        world.insert_resource(State::new(Screen::InWorld));
        world.init_resource::<Messages<AppExit>>();
        world.insert_resource(Continuous);
        world.insert_non_send_resource(controller);
        world.insert_non_send_resource(Net {
            session,
            sel: 0,
            light: false,
        });

        world.run_system_once(lifetime).unwrap();
        assert!(world.resource::<Messages<AppExit>>().is_empty());

        // Death is answered in-game now: a continuous run keeps its island.
        world.non_send_resource_mut::<Net>().session.core.dead = true;
        world.insert_resource(State::new(Screen::Dead));
        world.run_system_once(lifetime).unwrap();
        assert!(world.resource::<Messages<AppExit>>().is_empty());

        // The finite CLI mode keeps its duration bound.
        world.non_send_resource_mut::<Net>().session.core.dead = false;
        world.insert_resource(State::new(Screen::InWorld));
        world.remove_resource::<Continuous>();
        world.run_system_once(lifetime).unwrap();
        assert_eq!(
            world
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .collect::<Vec<_>>(),
            [AppExit::Success]
        );

        world.insert_resource(Continuous);
        world.insert_resource(State::new(Screen::Disconnected));
        world.run_system_once(lifetime).unwrap();
        assert_eq!(
            world
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .collect::<Vec<_>>(),
            [AppExit::error()]
        );

        // Continuous mode must not hide a renderer that never finishes loading.
        world.insert_resource(State::new(Screen::Loading));
        let mut controller = world.non_send_resource_mut::<Controller>();
        controller.playing = None;
        controller.created = Instant::now() - STARTUP_TIMEOUT - Duration::from_secs(1);
        world.run_system_once(lifetime).unwrap();
        assert_eq!(
            world
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .collect::<Vec<_>>(),
            [AppExit::error()]
        );
        shard.shutdown.store(true, Ordering::Relaxed);
    }
}
