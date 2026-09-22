//! One ordinary player session, driven by the bot and drawn by Gates' renderer.
//! The read-only page receives captured frames; it never opens a game session.
pub mod http;

use crate::botclient::BotDriver;
use crate::explorer::{Gatherer, Phase};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use client::render::{input, Net, Screen};
use protocol::MAX_EVENT_MSG_BYTES;
use sim_core::input::InputFrame;
use sim_core::limits::EVENT_RING_CAP;
use std::time::{Duration, Instant};

// Experimental broadcast defaults, registered in DECISIONS.md.
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;
pub const FRAME_INTERVAL: Duration = Duration::from_millis(250);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(300);

type Event = ([u8; MAX_EVENT_MSG_BYTES], usize);

pub struct Controller {
    pub gatherer: Gatherer,
    events: rtrb::Consumer<Event>,
    held: InputFrame,
    last_tick: Option<u32>,
    created: Instant,
    playing: Option<Instant>,
    duration: Duration,
    scripted: bool,
}

impl Controller {
    pub fn attach(
        session: &mut client::Session,
        mut gatherer: Gatherer,
        scripted: bool,
        duration: Duration,
    ) -> Result<Self, String> {
        gatherer.welcome(&session.welcome);
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
            gatherer,
            events,
            held: InputFrame {
                pitch: 128,
                ..Default::default()
            },
            last_tick: None,
            created: Instant::now(),
            playing: None,
            duration,
            scripted,
        })
    }

    /// Bounded reliable handoff, followed by at most one new control sample
    /// per advanced client tick. The core retains taps until a tick consumes
    /// them, so a faster renderer cannot lose a one-frame jump.
    pub fn sample(&mut self, session: &client::Session, ready: bool) -> Result<InputFrame, String> {
        for _ in 0..EVENT_RING_CAP {
            let Ok((bytes, n)) = self.events.pop() else {
                break;
            };
            self.gatherer
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
                self.gatherer
                    .frame(&session.core.view, session.welcome.player_id, tick as u16);
            self.last_tick = Some(tick);
        }
        Ok(self.held)
    }

    fn status(&self, session: &client::Session) -> Status {
        Status {
            scripted: self.scripted,
            phase: if session.core.dead {
                Phase::Dead
            } else if session.core.wounded {
                Phase::Wounded
            } else {
                self.gatherer.stats.phase
            },
            seconds: self.playing.map_or(0, |t| t.elapsed().as_secs()),
            hp: session.core.hp,
            hp_max: session.core.hp_max,
            wood: self.gatherer.stats.wood,
            gained: self.gatherer.stats.wood_gained,
            trees: self.gatherer.stats.targets_completed,
            retreats: self.gatherer.stats.retreats,
            decisions: self.gatherer.explorer.stats.decisions,
            failures: self.gatherer.explorer.stats.failures,
            decode_errors: session.core.decode_errors + session.core.event_errors,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Status {
    pub scripted: bool,
    pub phase: Phase,
    pub seconds: u64,
    pub hp: u16,
    pub hp_max: u16,
    pub wood: u32,
    pub gained: u32,
    pub trees: u64,
    pub retreats: u64,
    pub decisions: u64,
    pub failures: u64,
    pub decode_errors: u64,
}

impl Status {
    pub fn action(self) -> &'static str {
        match self.phase {
            Phase::Waiting => "Getting ready",
            Phase::Exploring => "Looking for trees",
            Phase::Approaching => "Approaching a tree",
            Phase::Harvesting => "Gathering wood",
            Phase::Recovering => "Trying another route",
            Phase::Fleeing => "Retreating from danger",
            Phase::NoTool => "Stopped: no usable tool",
            Phase::Full => "Stopped: pack full",
            Phase::Dead => "Dead",
            Phase::Wounded => "Wounded",
            Phase::Sleeping => "Sleeping",
            Phase::Stale => "Waiting for the shard",
        }
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
    let ready = *screen.get() == Screen::InWorld;
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

pub fn lifetime(
    controller: NonSend<Controller>,
    screen: Res<State<Screen>>,
    net: Option<NonSend<Net>>,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(screen.get(), Screen::Menu | Screen::Disconnected) {
        eprintln!("jev-watch: the bot's connection ended");
        exit.write(AppExit::error());
    } else if controller
        .playing
        .is_some_and(|t| t.elapsed() >= controller.duration)
    {
        println!("broadcast gathering: {:?}", controller.gatherer.stats);
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
    use std::sync::atomic::Ordering;

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
        let driver = crate::jev::Driver::new(
            crate::agent_demo::Scripted,
            crate::jev::THINK_INTERVAL,
            crate::jev::REQUEST_TIMEOUT,
        )
        .unwrap();
        let mut controller = Controller::attach(
            &mut session,
            Gatherer::new(driver),
            true,
            Duration::from_secs(120),
        )
        .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(15), async {
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
                if controller.gatherer.stats.wood_gained > 0 {
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
            "no harvest: {:?}",
            controller.gatherer.stats
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
}
