//! Chat, drawn: the scrollback, the composer, and the keys that reach them.
//!
//! `KIND_CHAT` is the one C→S message kind that is not an `ACT_*`, and the
//! native client could send neither it nor read `pop_chat` — the island was
//! silent in both directions.
//!
//! All arithmetic is [`crate::ui::chat`]'s: what `/g` does, and what the
//! composer does at the byte cap. This file owns nodes and keys.
//!
//! ## The composer takes the keyboard, and says so
//!
//! While it is open the world's own binds stand down — `input::gather` reads
//! `Chat::open` the same way it reads a panel's `grabs_pointer`. Without that,
//! typing "we should build here" walks you forward, swings twice, eats
//! whatever is in slot 3 and opens the inventory. The pointer comes back too,
//! which is the browser's behaviour and the reason a player can see what they
//! are typing.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use protocol::chat::Channel;
use protocol::Address;

use crate::ui::chat::{Composer, Line, Log};

use super::binds::Action;
use super::faces::Pics;
use super::hud::Toast;
use super::panels::Panel;
use super::Net;

/// Rust's chat shape: a narrow column bottom left, a square avatar per line,
/// bold type, and the input on a bar tinted by its channel. 440 px keeps a
/// centred toast clear of it at the 1280 px the client opens at.
const CHAT_W: f32 = 440.0;
const CHAT_LEFT: f32 = 22.0;
const BAR_BOTTOM: f32 = 70.0;
const BAR_H: f32 = 26.0;
const LOG_BOTTOM: f32 = BAR_BOTTOM + BAR_H + 6.0;
const AVATAR_PX: f32 = 22.0;
const LINE_PX: f32 = 15.0;
/// How many lines are drawn. The log keeps more (`LOG_CAP`); a column
/// taller than this would climb into the team panel.
const SHOWN: usize = 10;
const MESSAGE_INK: Color = Color::srgba(0.97, 0.97, 0.95, 1.0);
/// A name on words said out loud (a caption, v135): the voice HUD's
/// lavender rather than a channel's colour, since nobody chose a channel.
const VOICE_INK: Color = Color::srgba(0.80, 0.72, 1.0, 1.0);
const AVATAR_BLANK: Color = Color::srgba(0.12, 0.12, 0.13, 0.75);
/// A clickable line under the pointer while the composer is open.
const LINE_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.10);
const SHADOW_A: f32 = 0.85;

/// A channel's colour: the tag and name on a line, and the input bar.
fn ink(channel: Channel) -> Color {
    match channel {
        // Rust's team chat green.
        Channel::Team => Color::srgba(0.62, 0.85, 0.36, 1.0),
        Channel::Global => Color::srgba(0.98, 0.80, 0.38, 1.0),
        // Rust's default name blue, for the everyday channel.
        Channel::Local => Color::srgba(0.55, 0.76, 1.0, 1.0),
    }
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Team => "[Team]",
        Channel::Global => "[Global]",
        Channel::Local => "[Local]",
    }
}

/// The scrollback and the line being typed.
#[derive(Resource, Default)]
pub struct Chat {
    pub composer: Composer,
    pub log: Log,
    /// Redraw the log's node tree next frame. A log that rebuilt every frame
    /// would allocate a node per line per frame for a screen that changes
    /// when somebody speaks.
    pub dirty: bool,
    /// This client's language has gone to the shard (`/lang … auto`, v135),
    /// or it has none to send.
    pub lang_sent: bool,
}

impl Chat {
    /// Does the composer own the keyboard this frame?
    pub fn open(&self) -> bool {
        self.composer.open
    }
}

/// The scrollback's root.
#[derive(Component)]
pub struct LogRoot;

/// The input bar, and its three texts.
#[derive(Component)]
pub struct ComposerBar;
#[derive(Component)]
pub struct ComposerChannel;
#[derive(Component)]
pub struct ComposerLine;
#[derive(Component)]
pub struct ComposerHint;

/// Something in a line that fades with it: arrival time and full alpha.
/// `bg` fades its `BackgroundColor` too — only the avatar tile's: every
/// `Node` carries a transparent one, and fading that up draws a black box.
#[derive(Component)]
pub struct ChatFade {
    at: f64,
    base: f32,
    bg: bool,
}

/// A line that opens its speaker's profile card when clicked: only while
/// the composer is open, which is the only time the pointer is free over
/// the log (`panels::profile::chat_opens`).
#[derive(Component)]
pub struct ChatWho(pub super::panels::profile::ProfileOpen);

/// An avatar still waiting on its picture.
#[derive(Component)]
pub struct ChatPic {
    address: Address,
    pic: u32,
}

pub fn setup(mut commands: Commands) {
    // Bottom left, above the build plan. Not centred and not large: chat is
    // read at a glance and must never be the thing in front of the world.
    commands.spawn((
        super::WorldEntity,
        LogRoot,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(LOG_BOTTOM),
            left: Val::Px(CHAT_LEFT),
            width: Val::Px(CHAT_W),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(5.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
    commands
        .spawn((
            super::WorldEntity,
            ComposerBar,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(BAR_BOTTOM),
                left: Val::Px(CHAT_LEFT),
                width: Val::Px(CHAT_W),
                height: Val::Px(BAR_H),
                padding: UiRect::horizontal(Val::Px(8.0)),
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .with_children(|bar| {
            bar.spawn((
                ComposerChannel,
                Text::new(""),
                super::ui::font_bold(LINE_PX),
                TextColor(Color::WHITE),
                super::ui::TEXT_SHADOW,
                Pickable::IGNORE,
            ));
            bar.spawn((
                ComposerLine,
                Text::new(""),
                super::ui::font_bold(LINE_PX),
                TextColor(MESSAGE_INK),
                super::ui::TEXT_SHADOW,
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
                Pickable::IGNORE,
            ));
            bar.spawn((
                ComposerHint,
                Text::new(""),
                super::ui::font(12.0),
                TextColor(Color::srgba(0.92, 0.92, 0.88, 0.60)),
                Pickable::IGNORE,
            ));
        });
}

/// `T` and `Enter` open the composer; `Enter` sends; `Esc` abandons.
///
/// `keyboard` is `ResMut` for `panels::keys`' reason: **a key this consumed
/// must not reach the system after it.** Every press while the composer is
/// open is consumed, which is what stops a typed line also being a walk, a
/// swing and an inventory toggle.
// Eight, and every one is a distinct owner of the keyboard this frame: the
// composer's own state, the wire it sends on, the toast, the key state it
// consumes from, the character stream, the pointer it releases, and the two
// other text fields that must not be typed into at the same time.
#[allow(clippy::too_many_arguments)]
pub fn keys(
    mut chat: ResMut<Chat>,
    net: NonSend<Net>,
    mut toast: ResMut<Toast>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut chars: MessageReader<KeyboardInput>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    ui: Option<Res<super::panels::Ui>>,
    reports: Option<Res<super::report::Reports>>,
    (mouse, binds): (Res<ButtonInput<MouseButton>>, Res<super::binds::Binds>),
    pad: Option<Res<super::verbs::Pad>>,
    mut voice: (
        Option<ResMut<super::voice::Mutes>>,
        Option<Res<super::voice::VoiceHud>>,
    ),
) {
    // The report composer is a text field too, and it is the one that draws on
    // every screen — so it can be up here, where a panel cannot. Same rule as
    // the panels: two text fields open at once is one of them eating the
    // other's keystrokes, and this pair bites hardest because typing the
    // letter `t` into a bug report would otherwise open chat mid-sentence.
    //
    // The door keypad counts too: it types with `T` (TAKE) and Enter (TRY),
    // and chat opening on those keys ate them before the keypad saw them.
    let panel_up = ui.map(|u| u.panel != Panel::None).unwrap_or(false)
        || reports.is_some_and(|r| r.open)
        || pad.is_some_and(|p| p.0.is_open());

    if !chat.composer.open {
        // A panel already owns the keyboard — the search box is a text field
        // too, and two of them open at once is one of them eating the other's
        // keystrokes.
        if panel_up {
            chars.clear();
            return;
        }
        if binds.just_pressed(Action::Chat, &keyboard, &mouse) {
            chat.composer.open = true;
            chat.composer.clear();
            chat.dirty = true;
            binds.clear_just_pressed(Action::Chat, &mut keyboard);
            super::ui::free_cursor(&mut cursor);
        }
        // Not open: drain, so a keystroke pressed with the composer shut
        // does not arrive in it the moment it opens.
        chars.clear();
        return;
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        close(&mut chat, &mut cursor);
        keyboard.clear_just_pressed(KeyCode::Escape);
        chars.clear();
        return;
    }

    if keyboard.just_pressed(KeyCode::Enter) {
        // `/mute` and `/unmute` are this client's (`render/voice.rs`): never
        // sent, so nobody learns who muted them.
        let line = chat.composer.text.clone();
        let reply = match (voice.0.as_deref_mut(), voice.1.as_deref()) {
            (Some(mutes), Some(hud)) => super::voice::command(&line, mutes, &net, hud),
            _ => None,
        };
        match reply {
            Some(reply) => toast.say(reply),
            None => send(&mut chat, &net, &mut toast),
        }
        close(&mut chat, &mut cursor);
        keyboard.clear_just_pressed(KeyCode::Enter);
        chars.clear();
        return;
    }

    for ev in chars.read() {
        if !ev.state.is_pressed() {
            continue;
        }
        match &ev.logical_key {
            // Rust's channel switch.
            Key::Tab => {
                chat.composer.cycle();
                chat.dirty = true;
            }
            Key::Backspace => {
                if chat.composer.backspace() {
                    chat.dirty = true;
                }
            }
            Key::Space => {
                if chat.composer.push(' ') {
                    chat.dirty = true;
                }
            }
            Key::Character(s) => {
                let chars_in: Vec<char> = s.chars().collect();
                for c in chars_in {
                    if chat.composer.push(c) {
                        chat.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }
    // Everything else this frame belonged to the composer.
    keyboard.clear();
}

fn close(chat: &mut Chat, cursor: &mut Query<&mut CursorOptions, With<PrimaryWindow>>) {
    chat.composer.open = false;
    chat.composer.clear();
    chat.dirty = true;
    // Take the pointer back, so getting out of chat does not need a click
    // that would also swing the axe — `pause::act`'s rule.
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = super::web::in_post();
    }
}

fn send(chat: &mut Chat, net: &Net, toast: &mut Toast) {
    let Some((channel, text)) = chat.composer.outgoing() else {
        return; // nothing to say, or only a channel switch
    };
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::chat::encode_chat(text.as_bytes(), channel, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            // **The line is not echoed locally.** The server fans it back to
            // this client like any other, and echoing would print it twice —
            // or, worse, print it once when the server rate-limited it away
            // and never sent it at all, which is a client telling a player
            // their message went out when it did not.
            Ok(()) => {}
            Err(e) => toast.warn(e.to_string()),
        },
        // `sanitize` already passed inside `outgoing`, so this is a client
        // bug rather than a player one — reported, never swallowed.
        Err(e) => toast.warn(format!("that line would not encode ({e:?})")),
    }
}

/// Drain the core's chat and caption rings into the log, and tell the shard
/// which language this player reads, once a session.
pub fn drain(
    mut net: NonSendMut<Net>,
    mut chat: ResMut<Chat>,
    time: Res<Time<Real>>,
    friends: Option<Res<super::friends::Friends>>,
    mutes: Option<Res<super::voice::Mutes>>,
) {
    if !chat.lang_sent {
        chat.lang_sent = send_lang(&net);
    }
    let now = time.elapsed_secs_f64();
    // Who a line is from, settled as it arrives.
    let who = |core: &client_core::core::ClientCore, speaker: u32| {
        let name = crate::ui::names::of(core, speaker);
        let face = core.tag(speaker).map(|t| (t.address, t.pic));
        let friend = face
            .zip(friends.as_deref())
            .is_some_and(|((address, _), f)| f.set.has(&address));
        (name, face, friend)
    };
    while let Some((speaker, channel, text)) = net.session.core.pop_chat() {
        // Valid UTF-8 by construction (`ChatText` is only built by
        // `sanitize`), but a decode that somehow was not is dropped rather
        // than drawn as replacement characters — this is the one payload on
        // the wire that another player authored.
        let Ok(s) = std::str::from_utf8(text.as_bytes()) else {
            continue;
        };
        let (name, face, friend) = who(&net.session.core, speaker);
        chat.log.push(Line {
            speaker,
            name,
            channel,
            text: s.to_string(),
            face,
            friend,
            at: now,
            key: protocol::caption_key(&text),
            ..Line::default()
        });
        chat.dirty = true;
    }
    // Captions (v135): words said out loud nearby, and translations. A
    // talker this player muted stays muted in text too.
    while let Some(c) = net.session.core.pop_caption() {
        if c.voice && mutes.as_deref().is_some_and(|m| m.muted(c.from)) {
            continue;
        }
        let Ok(s) = std::str::from_utf8(c.text.as_bytes()) else {
            continue;
        };
        let (name, face, friend) = who(&net.session.core, c.from);
        chat.log.caption(
            Line {
                speaker: c.from,
                name,
                channel: c.channel,
                text: s.to_string(),
                face,
                friend,
                at: now,
                voice: c.voice,
                lang: c.lang,
                key: c.line,
            },
            c.more,
        );
        chat.dirty = true;
    }
}

/// Say `/lang <code> auto` for the system's language, so captions and chat
/// arrive translated into it without the player asking. `false` when the
/// line could not go yet; a system language not in the list sends nothing.
fn send_lang(net: &Net) -> bool {
    let Some(code) = system_lang() else {
        return true;
    };
    let line = format!("/lang {code} auto");
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::chat::encode_chat(line.as_bytes(), Channel::Local, &mut buf) {
        Ok(len) => net.session.send_action(&buf[..len]).is_ok(),
        Err(_) => true,
    }
}

/// The system's language as a `/lang` code: the OS locale natively, the
/// browser's language on the web.
fn system_lang() -> Option<&'static str> {
    #[cfg(target_arch = "wasm32")]
    let locale = web_sys::window().and_then(|w| w.navigator().language());
    #[cfg(not(target_arch = "wasm32"))]
    let locale = sys_locale::get_locale();
    protocol::lang_of(&locale?).and_then(protocol::lang_code)
}

/// Redraw the log and the composer when either changed.
#[allow(clippy::type_complexity)]
pub fn draw(
    mut commands: Commands,
    mut chat: ResMut<Chat>,
    root: Query<Entity, With<LogRoot>>,
    mut bar: Query<(&mut Visibility, &mut BackgroundColor), With<ComposerBar>>,
    mut label: Query<(&mut Text, &mut TextColor), With<ComposerChannel>>,
    mut line: Query<&mut Text, (With<ComposerLine>, Without<ComposerChannel>)>,
    mut hint: Query<
        &mut Text,
        (
            With<ComposerHint>,
            Without<ComposerLine>,
            Without<ComposerChannel>,
        ),
    >,
) {
    if !chat.dirty {
        return;
    }
    chat.dirty = false;

    let open = chat.composer.open;
    let target = chat.composer.target();
    if let Ok((mut v, mut bg)) = bar.single_mut() {
        *v = if open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        bg.0 = ink(target).darker(0.25).with_alpha(0.45);
    }
    if let Ok((mut text, mut color)) = label.single_mut() {
        text.0 = channel_name(target).to_string();
        color.0 = ink(target);
    }
    if let Ok(mut text) = line.single_mut() {
        text.0 = format!("{}|", chat.composer.shown());
    }
    if let Ok(mut text) = hint.single_mut() {
        // The cap is drawn because a cap the player cannot see is a cap
        // that feels like a broken keyboard.
        text.0 = format!("TAB channel · click a name · {}", chat.composer.remaining());
    }

    let Ok(root) = root.single() else {
        return;
    };
    commands.entity(root).despawn_related::<Children>();
    let skip = chat.log.lines.len().saturating_sub(SHOWN);
    let lines: Vec<Line> = chat.log.lines[skip..].to_vec();
    commands.entity(root).with_children(|c| {
        for l in lines {
            let fades = |base: f32| ChatFade {
                at: l.at,
                base,
                bg: false,
            };
            let mut line = c.spawn((
                Node {
                    align_items: AlignItems::FlexStart,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                Pickable::IGNORE,
            ));
            // With the composer open the pointer is free over the log, and a
            // signed-in speaker's line is a button to their profile card.
            if let Some((wallet, _)) = l.face.filter(|(a, _)| open && !a.is_guest()) {
                let rest = Color::NONE;
                line.insert((
                    Button,
                    ChatWho(super::panels::profile::ProfileOpen {
                        wallet,
                        name: l.name.clone(),
                    }),
                    BackgroundColor(rest),
                    super::panels::Hover::on(rest).hot(LINE_HOVER),
                ));
            }
            line.with_children(|row| {
                // The avatar: the speaker's picture once it lands, their
                // initial on a dark tile until then (or for good, for a
                // player with no picture).
                let mut avatar = row.spawn((
                    Node {
                        width: Val::Px(AVATAR_PX),
                        height: Val::Px(AVATAR_PX),
                        flex_shrink: 0.0,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(AVATAR_BLANK),
                    ChatFade {
                        at: l.at,
                        base: AVATAR_BLANK.alpha(),
                        bg: true,
                    },
                    Pickable::IGNORE,
                ));
                if let Some((address, pic)) = l.face.filter(|&(_, pic)| pic != 0) {
                    avatar.insert(ChatPic { address, pic });
                }
                let initial: String = l
                    .name
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_else(|| "?".into());
                avatar.with_children(|a| {
                    a.spawn((
                        Text::new(initial),
                        super::ui::font_bold(12.0),
                        TextColor(ink(l.channel)),
                        fades(1.0),
                        Pickable::IGNORE,
                    ));
                });

                row.spawn((
                    Text::new(Log::head(&l)),
                    super::ui::font_bold(LINE_PX),
                    // A friend's name in the friend colour, whatever the
                    // channel: the channel is still the tag before it.
                    TextColor(if l.friend {
                        super::ui::FRIEND
                    } else if l.voice {
                        VOICE_INK
                    } else {
                        ink(l.channel)
                    }),
                    // The chat log is the one floating string a stranger
                    // writes the content of, so it most has to stay readable
                    // over whatever the player is facing (`ui::TEXT_SHADOW`).
                    super::ui::TEXT_SHADOW,
                    fades(1.0),
                    Node {
                        flex_shrink: 1.0,
                        // Line the first baseline up with the avatar.
                        margin: UiRect::top(Val::Px(2.0)),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|t| {
                    t.spawn((
                        TextSpan::new(l.text.clone()),
                        super::ui::font_bold(LINE_PX),
                        TextColor(MESSAGE_INK),
                        fades(1.0),
                    ));
                });
            });
        }
    });
}

/// Fade each line out once it is old news, Rust's way; an open composer
/// brings them all back.
#[allow(clippy::type_complexity)]
pub fn fade(
    chat: Res<Chat>,
    time: Res<Time<Real>>,
    mut parts: Query<(
        &ChatFade,
        Option<&mut TextColor>,
        Option<&mut TextShadow>,
        Option<&mut BackgroundColor>,
        Option<&mut ImageNode>,
    )>,
) {
    let now = time.elapsed_secs_f64();
    let open = chat.composer.open;
    for (f, text, shadow, bg, img) in &mut parts {
        let a = Log::alpha((now - f.at) as f32, open);
        let want = f.base * a;
        if let Some(mut c) = text {
            if c.0.alpha() != want {
                c.0.set_alpha(want);
            }
        }
        if let Some(mut s) = shadow {
            let sa = SHADOW_A * a;
            if s.color.alpha() != sa {
                s.color.set_alpha(sa);
            }
        }
        if let Some(mut c) = bg.filter(|_| f.bg) {
            if c.0.alpha() != want {
                c.0.set_alpha(want);
            }
        }
        if let Some(mut i) = img {
            if i.color.alpha() != want {
                i.color.set_alpha(want);
            }
        }
    }
}

/// Swap an avatar's initial for the speaker's picture once it lands.
pub fn fill_pics(
    mut commands: Commands,
    pics: Option<ResMut<Pics>>,
    waiting: Query<(Entity, &ChatPic, &ChatFade)>,
) {
    let Some(mut pics) = pics else {
        return;
    };
    for (e, p, f) in &waiting {
        if let Some(h) = pics.want(p.address, p.pic) {
            commands
                .entity(e)
                .despawn_related::<Children>()
                .remove::<ChatPic>()
                .insert((
                    ImageNode::new(h),
                    BackgroundColor(Color::NONE),
                    ChatFade {
                        at: f.at,
                        base: 1.0,
                        bg: false,
                    },
                ));
        }
    }
}

/// Forget the last shard's conversation.
pub fn forget(mut chat: ResMut<Chat>) {
    *chat = Chat::default();
}

/// Wire chat in. Registered beside the panels and, like them, **not on a
/// capture run**: a visual gate whose frames depend on whether a composer is
/// open is not a gate.
pub fn register(app: &mut App) {
    app.init_resource::<Chat>()
        // The nodes are `WorldEntity`, so they are built with the world and
        // `world_teardown` takes them.
        .add_systems(OnEnter(super::Screen::Loading), setup)
        .add_systems(
            Update,
            keys
                // Before the panels, so `T` cannot be read by both, and well
                // before `input::gather`, which reads `Chat::open` to decide
                // whether the world's binds stand down this frame.
                .before(super::panels::keys)
                .before(super::input::gather)
                .run_if(in_state(super::Screen::InWorld)),
        )
        // `drain` and `draw` run wherever the world does: a line that arrived
        // while the Esc menu was up is still owed to the player, and the ring
        // it sits in is bounded and drops the newest when it overflows.
        .add_systems(
            Update,
            (drain, draw, fill_pics, fade)
                .chain()
                .run_if(super::world_running),
        )
        .add_systems(OnEnter(super::Screen::Menu), forget);
}
