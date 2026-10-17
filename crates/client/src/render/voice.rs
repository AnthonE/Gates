//! Proximity voice (wire v113): hold the talk key and the players near you
//! hear you; the players near you who talk come out of where they stand.
//!
//! - **Talk**: hold `V` (Rust's key) or a side mouse button. The first press
//!   opens the microphone, which is when a browser asks the player for it.
//!   Samples go to 16 kHz, into 20 ms Opus frames (`crates/voice`), and out
//!   as datagrams; letting go sends the last frame with the end flag.
//!   SETTINGS > AUDIO can make the key a toggle, or open the microphone
//!   whenever it hears more than a threshold; it also sets the microphone's
//!   gain and device, or turns voice off altogether.
//! - **Hear**: the shard relays a talker's frames only to players within
//!   `VOICE_RADIUS_CM` (`ShardCore::voice_reach`), so who can hear whom is
//!   never this client's call. Each talker gets a lane of `sound::talk`: the
//!   frames decode at the output rate, and the lane's ear gains follow the
//!   talker's body, quieter with distance and panned by where they stand.
//! - **Mute**: `/mute NAME` and `/unmute NAME` in chat, or `/mute` alone for
//!   whoever is talking right now. This client stops playing them; nothing
//!   is said to anyone. MUTE MY MICROPHONE and MUTE ALL VOICES are binds
//!   (unbound by default) for the rest of the session.
//!
//! Voice is never game state: it does not pass `ClientCore`, and a frame
//! that never arrives costs a gap, never a desync.

use bevy::prelude::*;

use crate::config::TalkMode;
use crate::sound::talk::{TalkMsg, TALKERS, TALK_CHUNK};
use protocol::VoiceFrame;
use sim_core::limits::VOICE_RADIUS_CM;

use super::audio;
use super::binds::{Action, Binds};
use super::bodies::Body;
use super::chat::Chat;
use super::hud::Toast;
use super::settings::Settings;
use super::{Eye, Net};

/// Where a voice is heard from to nothing, metres: the shard's radius.
const RADIUS_M: f32 = VOICE_RADIUS_CM as f32 / 100.0;
/// Full volume inside this, metres.
const NEAR_M: f32 = 2.0;
/// A mouth above the feet, metres.
const MOUTH_M: f32 = 1.5;
/// A lane nobody has spoken on for this long is freed.
const LANE_IDLE_S: f64 = 5.0;
/// A talker counts as talking for this long after their last frame.
const TALKING_S: f64 = 0.35;
/// Speech sits a little under the game's cues at parity; this lifts it.
const VOICE_GAIN: f32 = 1.2;
/// An open microphone stays on the air this long after the last sound over
/// its threshold, so the gaps between words do not cut it.
const GATE_HOLD_S: f64 = 0.4;

/// Who this player has muted, by player id, for this session.
#[derive(Resource, Default)]
pub struct Mutes {
    ids: Vec<u32>,
}

impl Mutes {
    pub fn muted(&self, id: u32) -> bool {
        self.ids.contains(&id)
    }

    pub fn mute(&mut self, id: u32) {
        if !self.muted(id) {
            self.ids.push(id);
        }
    }

    pub fn unmute(&mut self, id: u32) {
        self.ids.retain(|&m| m != id);
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }
}

/// The microphone as the player sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MicState {
    /// Never asked for: the talk key has not been pressed yet.
    #[default]
    Off,
    /// Waiting on the browser's permission prompt.
    Asking,
    On,
    /// No microphone, or the player said no.
    None,
}

/// What the HUD draws: whether this player is on the air, and who else is.
#[derive(Resource, Default)]
pub struct VoiceHud {
    pub on_air: bool,
    pub mic: MicState,
    /// MUTE MY MICROPHONE: nothing goes out, in any talk mode.
    pub mic_muted: bool,
    /// MUTE ALL VOICES: nobody is played.
    pub deaf: bool,
    /// Talkers sounding right now, by name.
    pub talking: Vec<(u32, String)>,
    dirty: bool,
}

/// One talker, as heard.
struct Lane {
    id: u32,
    dec: ::voice::Decoder,
    /// When their last frame landed, seconds of real time.
    last: f64,
    /// Their last frame carried the end flag: the key came up.
    ended: bool,
    /// Walls between them and the ears (`ui::occlusion`), and when that was
    /// last walked: a quarter second, the reference's own re-check.
    walls: u8,
    walled_at: f64,
}

/// How often a talker's walls are walked again, seconds.
const WALLS_EVERY_S: f64 = 0.25;

/// The voice state that cannot be a `Resource`: a desktop microphone holds
/// a `cpal::Stream`, a browser's a `JsValue`, and neither is `Send`.
pub struct Voice {
    mic: Option<mic::Mic>,
    /// The device `mic` was opened on (the setting's name; empty is the
    /// default).
    device: String,
    /// TOGGLE mode: the talk key last turned it on.
    toggled: bool,
    /// OPEN mode: on the air until this, seconds of real time.
    gate_until: f64,
    talker: ::voice::Talker,
    /// The microphone's rate down to the codec's, built when the rate is known.
    down: Option<(u32, ::voice::Resampler)>,
    raw: Vec<f32>,
    pcm16: Vec<f32>,
    lanes: [Option<Lane>; TALKERS],
    /// The rate the lanes decode at: the output's.
    out_rate: u32,
    pcm: Vec<f32>,
    /// Whether the player has been told there is no microphone.
    told: bool,
}

impl Voice {
    fn new() -> Self {
        Self {
            mic: None,
            device: String::new(),
            toggled: false,
            gate_until: f64::MIN,
            talker: ::voice::Talker::new(),
            down: None,
            raw: Vec::with_capacity(4_096),
            pcm16: Vec::with_capacity(1_024),
            lanes: Default::default(),
            out_rate: 0,
            pcm: Vec::with_capacity(4 * TALK_CHUNK),
            told: false,
        }
    }
}

/// Is a talk key or button held? The keys are the player's
/// (`binds::Action::Talk`: `` ` `` and a mouse side button by default).
pub fn talk_held(
    binds: &super::binds::Binds,
    keys: &ButtonInput<KeyCode>,
    mouse: &ButtonInput<MouseButton>,
) -> bool {
    binds.pressed(super::binds::Action::Talk, keys, mouse)
}

/// The talking half: the keys, the microphone, the encoder, the send.
#[allow(clippy::too_many_arguments)]
pub fn talk(
    mut voice: NonSendMut<Voice>,
    mut net: NonSendMut<Net>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    chat: Option<Res<Chat>>,
    mut hud: ResMut<VoiceHud>,
    mut toast: ResMut<Toast>,
    binds: Res<Binds>,
    settings: Option<Res<Settings>>,
    time: Res<Time<Real>>,
) {
    let v = &mut *voice;
    let now = time.elapsed_secs_f64();
    let (enabled, mode, gain, gate_db, device) = match settings.as_deref() {
        Some(s) => (
            s.voice_chat,
            s.talk_mode,
            s.vol_mic,
            s.mic_gate_db,
            s.mic_device.as_str(),
        ),
        None => (true, TalkMode::Hold, 1.0, -45.0, ""),
    };
    // A line being typed owns the keyboard.
    let typing = chat.is_some_and(|c| c.open());
    if !typing {
        if binds.just_pressed(Action::MuteMic, &keys, &mouse) {
            hud.mic_muted = !hud.mic_muted;
            hud.dirty = true;
            toast.say(if hud.mic_muted {
                "voice: your microphone is muted"
            } else {
                "voice: your microphone is on"
            });
        }
        if binds.just_pressed(Action::MuteVoices, &keys, &mouse) {
            hud.deaf = !hud.deaf;
            hud.dirty = true;
            toast.say(if hud.deaf {
                "voice: all voices muted"
            } else {
                "voice: voices on"
            });
        }
    }
    let pressed = !typing && binds.just_pressed(Action::Talk, &keys, &mouse);
    if pressed && !enabled {
        toast.warn("voice chat is off - turn it on in SETTINGS > AUDIO");
    } else if pressed && hud.mic_muted {
        toast.warn("voice: your microphone is muted");
    }

    // A watcher has no voice.
    let can = enabled && !hud.mic_muted && net.session.watching.is_none();
    let held = can && !typing && talk_held(&binds, &keys, &mouse);
    if mode != TalkMode::Toggle || !can {
        v.toggled = false;
    } else if pressed {
        v.toggled = !v.toggled;
    }

    // Voice off, or another device picked: let the microphone go. It opens
    // again when it is next wanted.
    if v.mic.is_some() && (!enabled || v.device != device) {
        v.mic = None;
        v.down = None;
        v.told = false;
    }
    let want = can
        && match mode {
            TalkMode::Hold => held,
            TalkMode::Toggle => v.toggled,
            // Listening for speech is what an open microphone is.
            TalkMode::Open => true,
        };
    if want && v.mic.is_none() {
        v.mic = Some(mic::Mic::open(device));
        v.device = device.to_string();
    }
    let state = v.mic.as_ref().map(|m| m.state()).unwrap_or_default();
    if state == MicState::None && want && !v.told {
        v.told = true;
        toast.warn("no microphone: you hear the players near you, but cannot talk");
    }

    v.raw.clear();
    let mut rate = 0;
    if let Some(m) = v.mic.as_mut() {
        m.take(&mut v.raw);
        rate = m.rate();
    }
    v.pcm16.clear();
    if rate > 0 && !v.raw.is_empty() {
        if v.down.as_ref().map(|(r, _)| *r) != Some(rate) {
            v.down = Some((rate, ::voice::Resampler::new(rate, ::voice::RATE)));
        }
        let (_, rs) = v.down.as_mut().expect("built above");
        let out = &mut v.pcm16;
        rs.process(&v.raw, |s| out.push(s * gain));
    }
    if mode == TalkMode::Open && level_db(&v.pcm16).is_some_and(|db| db >= gate_db) {
        v.gate_until = now + GATE_HOLD_S;
    }
    let keyed = match mode {
        TalkMode::Hold => held,
        TalkMode::Toggle => v.toggled,
        TalkMode::Open => held || now < v.gate_until,
    };
    let on_air = can && keyed && state == MicState::On;
    if on_air {
        // A frame the microphone delivered nothing on is still on the air:
        // the end flag waits for the key, or the gate, to let go.
        let session = &mut net.session;
        v.talker.push(&v.pcm16, |seq, _, packet| {
            if let Some(f) = VoiceFrame::new(seq, 0, packet) {
                session.send_voice(&f);
            }
        });
    } else {
        // Off the air: whatever is buffered goes out with the end flag
        // (nothing at all when nothing was said).
        let session = &mut net.session;
        v.talker.finish(|seq, _, packet| {
            if let Some(f) = VoiceFrame::new(seq, protocol::VOICE_END, packet) {
                session.send_voice(&f);
            }
        });
    }
    if hud.on_air != on_air || hud.mic != state {
        hud.on_air = on_air;
        hud.mic = state;
        hud.dirty = true;
    }
}

/// The loudness of `pcm` in dBFS (RMS), or `None` for no samples.
pub fn level_db(pcm: &[f32]) -> Option<f32> {
    if pcm.is_empty() {
        return None;
    }
    let mean = pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32;
    Some(10.0 * mean.max(1e-12).log10())
}

/// The microphones this machine has, by name, for the settings screen. A
/// browser lists none: it uses the one its permission prompt allowed.
pub fn input_devices() -> Vec<String> {
    mic::devices()
}

/// The hearing half: every relayed frame decoded into its talker's lane,
/// and every lane's ear gains from where its talker stands.
#[allow(clippy::too_many_arguments)]
pub fn hear(
    mut voice: NonSendMut<Voice>,
    net: NonSend<Net>,
    mut engine: ResMut<audio::Engine>,
    mutes: Res<Mutes>,
    settings: Option<Res<Settings>>,
    eye: Res<Eye>,
    bodies: Query<(&Body, &Transform)>,
    time: Res<Time<Real>>,
    mut hud: ResMut<VoiceHud>,
    world: Option<Res<super::WorldId>>,
) {
    let now = time.elapsed_secs_f64();
    let v = &mut *voice;
    if v.out_rate != engine.out_rate {
        // The output moved (a device opened): every decoder was built for
        // the old rate.
        v.out_rate = engine.out_rate;
        for (lane, slot) in v.lanes.iter_mut().enumerate() {
            if slot.take().is_some() {
                engine.push_talk(TalkMsg::Stop { lane: lane as u8 });
            }
        }
    }

    // Voice off, or every voice muted: what arrives is dropped, and every
    // lane falls silent.
    if hud.deaf || settings.as_ref().is_some_and(|s| !s.voice_chat) {
        net.session.drain_voice(|_| {});
        for (lane, slot) in v.lanes.iter_mut().enumerate() {
            if slot.take().is_some() {
                engine.push_talk(TalkMsg::Stop { lane: lane as u8 });
            }
        }
        if !hud.talking.is_empty() {
            hud.talking.clear();
            hud.dirty = true;
        }
        return;
    }

    let lanes = &mut v.lanes;
    let pcm = &mut v.pcm;
    let out_rate = v.out_rate;
    net.session.drain_voice(|f| {
        if mutes.muted(f.from) {
            return;
        }
        let lane = match lanes
            .iter()
            .position(|l| l.as_ref().is_some_and(|l| l.id == f.from))
        {
            Some(i) => i,
            None => {
                // A free lane, else the one quiet the longest.
                let i = lanes.iter().position(Option::is_none).unwrap_or_else(|| {
                    (0..TALKERS)
                        .min_by(|&a, &b| {
                            let t = |i: usize| lanes[i].as_ref().map_or(0.0, |l| l.last);
                            t(a).total_cmp(&t(b))
                        })
                        .unwrap_or(0)
                });
                engine.push_talk(TalkMsg::Stop { lane: i as u8 });
                lanes[i] = Some(Lane {
                    id: f.from,
                    dec: ::voice::Decoder::new(out_rate),
                    last: now,
                    ended: false,
                    walls: 0,
                    walled_at: f64::MIN,
                });
                i
            }
        };
        let l = lanes[lane].as_mut().expect("filled above");
        l.last = now;
        l.ended = f.is_end();
        pcm.clear();
        l.dec.push(f.seq, f.is_end(), f.payload(), pcm);
        for chunk in pcm.chunks(TALK_CHUNK) {
            let mut msg = [0.0; TALK_CHUNK];
            msg[..chunk.len()].copy_from_slice(chunk);
            engine.push_talk(TalkMsg::Pcm {
                lane: lane as u8,
                len: chunk.len() as u16,
                pcm: msg,
            });
        }
    });

    // The gains: master and VOICE, the distance, the pan, and the walls.
    let occlude = settings.as_ref().is_none_or(|s| s.occlusion);
    let level = settings.map(|s| s.master() * s.vol_voice).unwrap_or(1.0) * VOICE_GAIN;
    let (rx, rz) = crate::look::right_dir(crate::look::yaw_u16(eye.yaw));
    let mut talking: Vec<(u32, String)> = Vec::new();
    for (i, slot) in v.lanes.iter_mut().enumerate() {
        let Some(l) = slot.as_mut() else {
            continue;
        };
        if now - l.last > LANE_IDLE_S || mutes.muted(l.id) {
            *slot = None;
            engine.push_talk(TalkMsg::Stop { lane: i as u8 });
            continue;
        }
        let at = bodies
            .iter()
            .find(|(b, _)| b.0 == l.id)
            .map(|(_, t)| t.translation + Vec3::Y * MOUTH_M);
        let (gl, gr) = match at {
            Some(at) => ears(at - eye.pos, rx, rz),
            // Not drawn (a watcher's own target, a body not streamed in yet):
            // in the middle, a little down.
            None => (0.7, 0.7),
        };
        if let (Some(at), Some(w), true) = (at, world.as_deref(), occlude) {
            if now - l.walled_at >= WALLS_EVERY_S {
                l.walled_at = now;
                let cols = net.session.core.pieces.cols();
                let (ear, mouth) = ([eye.pos.x, eye.pos.y, eye.pos.z], [at.x, at.y, at.z]);
                l.walls = crate::ui::occlusion::walls_between(w.seed, &w.haven, cols, ear, mouth);
            }
        } else {
            l.walls = 0;
        }
        let (wall_gain, lp) = match crate::ui::occlusion::muffle(l.walls) {
            Some((g, hz)) => (g, one_pole(hz, out_rate)),
            None => (1.0, 1.0),
        };
        engine.push_talk(TalkMsg::Gain {
            lane: i as u8,
            l: gl * level * wall_gain,
            r: gr * level * wall_gain,
            lp,
        });
        if !l.ended && now - l.last < TALKING_S {
            talking.push((l.id, crate::ui::names::of(&net.session.core, l.id)));
        }
    }
    if talking != hud.talking {
        hud.talking = talking;
        hud.dirty = true;
    }
}

/// A one-pole low-pass coefficient for `hz` at `rate` (1.0 is open).
fn one_pole(hz: f32, rate: u32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / rate.max(1) as f32).exp()
}

/// Ear gains for a voice at `to` (from the listener's eye), with the
/// listener's right vector `(rx, rz)`: full inside [`NEAR_M`], down to
/// nothing at the shard's radius, and an equal-power pan that never takes
/// a voice wholly out of one ear.
///
/// **Cubed, over the reference's 100 m**: −7 dB at 25 m, −14 dB at 40 m,
/// −23 dB at 60 m, −35 dB at 80 m. Clear across a compound, a murmur from
/// the next ridge — the reference's players put plain speech at 20–40 m,
/// and its server still sends the packet to 100 m.
pub fn ears(to: Vec3, rx: f32, rz: f32) -> (f32, f32) {
    let d = to.length();
    let g = (1.0 - (d - NEAR_M).max(0.0) / (RADIUS_M - NEAR_M)).clamp(0.0, 1.0);
    let g = g * g * g;
    let flat = Vec2::new(to.x, to.z);
    let side = if flat.length() > 0.01 {
        flat.normalize().dot(Vec2::new(rx, rz))
    } else {
        0.0
    };
    let theta = (side * 0.7 + 1.0) * std::f32::consts::FRAC_PI_4;
    let k = std::f32::consts::SQRT_2 * g;
    (theta.cos() * k, theta.sin() * k)
}

/// `/mute NAME`, `/unmute NAME`, `/mute` (whoever is talking) and `/unmute`
/// (everyone): handled here, never sent. `Some(reply)` when `line` was one.
pub fn command(line: &str, mutes: &mut Mutes, net: &Net, hud: &VoiceHud) -> Option<String> {
    let line = line.trim();
    let (mute, rest) = match line.strip_prefix("/mute") {
        Some(r) => (true, r),
        None => (false, line.strip_prefix("/unmute")?),
    };
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None; // `/muted`, `/mutex`: not ours
    }
    let who = rest.trim().to_lowercase();
    let core = &net.session.core;
    if who.is_empty() {
        if !mute {
            mutes.clear();
            return Some("voice: everyone unmuted".into());
        }
        if hud.talking.is_empty() {
            return Some("voice: nobody is talking (/mute NAME)".into());
        }
        for (id, _) in &hud.talking {
            mutes.mute(*id);
        }
        let names: Vec<&str> = hud.talking.iter().map(|(_, n)| n.as_str()).collect();
        return Some(format!("voice: muted {}", names.join(", ")));
    }
    let found = core
        .interp
        .ids()
        .filter(|&id| id != core.player_id)
        .find(|&id| crate::ui::names::of(core, id).to_lowercase().contains(&who));
    let Some(id) = found else {
        return Some(format!("voice: no player near you called {who}"));
    };
    let name = crate::ui::names::of(core, id);
    if mute {
        mutes.mute(id);
        Some(format!("voice: muted {name}"))
    } else {
        mutes.unmute(id);
        Some(format!("voice: unmuted {name}"))
    }
}

/// The HUD's voice corner.
#[derive(Component)]
struct VoiceRoot;

fn setup(mut commands: Commands, mut hud: ResMut<VoiceHud>) {
    commands.spawn((
        super::WorldEntity,
        VoiceRoot,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(30.0),
            left: Val::Px(14.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
    hud.dirty = true;
}

/// Who is talking, and whether you are.
fn draw(mut commands: Commands, mut hud: ResMut<VoiceHud>, root: Query<Entity, With<VoiceRoot>>) {
    if !hud.dirty {
        return;
    }
    let Ok(root) = root.single() else {
        return;
    };
    hud.dirty = false;
    commands.entity(root).despawn_related::<Children>();
    let mut rows: Vec<(String, Color)> = Vec::new();
    match (hud.mic, hud.on_air) {
        (MicState::On, true) => rows.push(("YOU ARE TALKING".into(), Color::srgb(0.95, 0.35, 0.3))),
        (MicState::Asking, _) => rows.push((
            "MICROPHONE: allow it in your browser".into(),
            Color::srgb(0.95, 0.8, 0.4),
        )),
        _ => {}
    }
    if hud.mic_muted {
        rows.push(("MICROPHONE MUTED".into(), Color::srgb(0.95, 0.6, 0.3)));
    }
    if hud.deaf {
        rows.push(("VOICES MUTED".into(), Color::srgb(0.95, 0.6, 0.3)));
    }
    for (_, name) in &hud.talking {
        rows.push((format!("{name} is talking"), Color::srgb(0.55, 0.95, 0.55)));
    }
    commands.entity(root).with_children(|c| {
        for (text, color) in rows {
            c.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
                Pickable::IGNORE,
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(text),
                    super::ui::font_bold(15.0),
                    TextColor(color),
                    Pickable::IGNORE,
                ));
            });
        }
    });
}

/// Forget the last shard's talkers; keep the microphone.
fn forget(mut voice: NonSendMut<Voice>, mut mutes: ResMut<Mutes>, mut hud: ResMut<VoiceHud>) {
    voice.lanes = Default::default();
    voice.talker = ::voice::Talker::new();
    voice.toggled = false;
    mutes.clear();
    hud.talking.clear();
    hud.on_air = false;
}

/// Wire voice in, beside chat (and like it, never on a capture run).
pub fn register(app: &mut App) {
    app.insert_non_send_resource(Voice::new())
        .init_resource::<Mutes>()
        .init_resource::<VoiceHud>()
        .add_systems(OnEnter(super::Screen::Loading), setup)
        .add_systems(
            Update,
            (talk, hear, draw)
                .chain()
                .after(super::chat::keys)
                .run_if(super::world_running),
        )
        .add_systems(OnEnter(super::Screen::Menu), forget);
}

/// The desktop microphone: cpal's default input, downmixed to mono on its
/// own callback thread into a ring the game thread drains.
#[cfg(not(target_arch = "wasm32"))]
mod mic {
    use super::MicState;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample};

    pub struct Mic {
        live: Option<(cpal::Stream, rtrb::Consumer<f32>, u32)>,
    }

    impl Mic {
        pub fn open(device: &str) -> Self {
            match open(device) {
                Ok(live) => Self { live: Some(live) },
                Err(why) => {
                    bevy::log::info!("voice: no microphone - {why}");
                    Self { live: None }
                }
            }
        }

        pub fn state(&self) -> MicState {
            if self.live.is_some() {
                MicState::On
            } else {
                MicState::None
            }
        }

        pub fn rate(&self) -> u32 {
            self.live.as_ref().map_or(0, |l| l.2)
        }

        pub fn take(&mut self, out: &mut Vec<f32>) {
            if let Some((_, rx, _)) = self.live.as_mut() {
                while let Ok(s) = rx.pop() {
                    out.push(s);
                }
            }
        }
    }

    pub fn devices() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Ok(all) = cpal::default_host().input_devices() {
            for name in all.filter_map(|d| d.name().ok()) {
                let name = crate::config::device_name(&name);
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// The device called `want`, else the default (`want` empty, or that
    /// device gone).
    fn open(want: &str) -> Result<(cpal::Stream, rtrb::Consumer<f32>, u32), String> {
        let host = cpal::default_host();
        let named = if want.is_empty() {
            None
        } else {
            let found = host.input_devices().ok().and_then(|mut all| {
                // By the form the settings file keeps (`config::device_name`).
                all.find(|d| {
                    d.name()
                        .is_ok_and(|n| crate::config::device_name(&n) == want)
                })
            });
            if found.is_none() {
                bevy::log::info!("voice: no microphone called {want:?}; the default instead");
            }
            found
        };
        let device = match named {
            Some(d) => d,
            None => host.default_input_device().ok_or("no input device")?,
        };
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate().0;
        let channels = config.channels().max(1) as usize;
        // A second of samples: the game thread drains it every frame.
        let (tx, rx) = rtrb::RingBuffer::new(rate as usize);
        let cfg = config.config();
        let stream = match config.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &cfg, channels, tx),
            SampleFormat::I16 => build::<i16>(&device, &cfg, channels, tx),
            SampleFormat::U16 => build::<u16>(&device, &cfg, channels, tx),
            SampleFormat::I32 => build::<i32>(&device, &cfg, channels, tx),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        bevy::log::info!("voice: microphone at {rate} Hz, {channels} channel(s)");
        Ok((stream, rx, rate))
    }

    fn build<T: SizedSample>(
        device: &cpal::Device,
        cfg: &cpal::StreamConfig,
        channels: usize,
        mut tx: rtrb::Producer<f32>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        f32: FromSample<T>,
    {
        device.build_input_stream::<T, _, _>(
            cfg,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                for frame in data.chunks_exact(channels) {
                    let sum: f32 = frame.iter().map(|s| s.to_sample::<f32>()).sum();
                    // Full ring: the game thread is behind; drop the newest.
                    let _ = tx.push(sum / channels as f32);
                }
            },
            |_e: cpal::StreamError| {},
            None,
        )
    }
}

/// The browser microphone: the page's `gatesMic` (`client-web/web/app.js`),
/// which asks for the microphone, taps it with the `gates-mic` processor and
/// hands over what it captured each frame.
#[cfg(target_arch = "wasm32")]
mod mic {
    use super::MicState;
    use js_sys::{Float32Array, Function, Reflect};
    use wasm_bindgen::{JsCast, JsValue};

    pub struct Mic {
        obj: Option<JsValue>,
    }

    fn get(obj: &JsValue, key: &str) -> Option<JsValue> {
        Reflect::get(obj, &JsValue::from_str(key))
            .ok()
            .filter(|v| !v.is_undefined() && !v.is_null())
    }

    fn call(obj: &JsValue, name: &str) -> Option<JsValue> {
        let f = get(obj, name)?.dyn_into::<Function>().ok()?;
        f.call0(obj).ok()
    }

    pub fn devices() -> Vec<String> {
        Vec::new()
    }

    /// Voice off lets the microphone go, so the browser stops showing it in
    /// use.
    impl Drop for Mic {
        fn drop(&mut self) {
            if let Some(o) = self.obj.as_ref() {
                call(o, "stop");
            }
        }
    }

    impl Mic {
        pub fn open(_device: &str) -> Self {
            let obj = get(&js_sys::global(), "gatesMic");
            match obj.as_ref() {
                Some(o) => {
                    call(o, "start");
                }
                None => bevy::log::info!("voice: the page has no gatesMic"),
            }
            Self { obj }
        }

        pub fn state(&self) -> MicState {
            let Some(o) = self.obj.as_ref() else {
                return MicState::None;
            };
            match get(o, "state").and_then(|s| s.as_string()).as_deref() {
                Some("on") => MicState::On,
                Some("asking") => MicState::Asking,
                Some("off") => MicState::Off,
                _ => MicState::None,
            }
        }

        pub fn rate(&self) -> u32 {
            self.obj
                .as_ref()
                .and_then(|o| get(o, "rate"))
                .and_then(|r| r.as_f64())
                .filter(|r| *r >= 8_000.0 && *r <= 384_000.0)
                .map_or(0, |r| r as u32)
        }

        pub fn take(&mut self, out: &mut Vec<f32>) {
            let Some(o) = self.obj.as_ref() else {
                return;
            };
            if let Some(a) = call(o, "take").and_then(|v| v.dyn_into::<Float32Array>().ok()) {
                let at = out.len();
                out.resize(at + a.length() as usize, 0.0);
                a.copy_to(&mut out[at..]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ears_fall_off_with_distance_and_pan_by_side() {
        // Right vector +x: a talker to the right is louder in the right ear.
        let (l, r) = ears(Vec3::new(5.0, 0.0, 0.0), 1.0, 0.0);
        assert!(r > l * 2.0, "{l} {r}");
        let (l2, r2) = ears(Vec3::new(0.0, 0.0, -1.0), 1.0, 0.0);
        assert!((l2 - r2).abs() < 1e-4 && (l2 - 1.0).abs() < 1e-3);
        let (fl, fr) = ears(Vec3::new(0.0, 0.0, -40.0), 1.0, 0.0);
        assert!(fl < 0.25 && fl > 0.15, "40 m is -14 dB: {fl} {fr}");
        let (fl, _) = ears(Vec3::new(0.0, 0.0, -80.0), 1.0, 0.0);
        assert!(fl < 0.03 && fl > 0.0, "80 m is a murmur: {fl}");
        assert_eq!(ears(Vec3::new(0.0, 0.0, -RADIUS_M), 1.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn level_reads_full_scale_and_silence() {
        assert_eq!(level_db(&[]), None);
        let full = level_db(&[1.0, -1.0, 1.0, -1.0]).unwrap();
        assert!(full.abs() < 1e-3, "{full}");
        let tenth = level_db(&[0.1, -0.1]).unwrap();
        assert!((tenth + 20.0).abs() < 1e-3, "{tenth}");
        assert!(level_db(&[0.0; 8]).unwrap() < -100.0);
    }

    #[test]
    fn mutes_hold_ids() {
        let mut m = Mutes::default();
        m.mute(7);
        m.mute(7);
        assert!(m.muted(7));
        m.unmute(7);
        assert!(!m.muted(7));
    }
}
