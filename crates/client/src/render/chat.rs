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

use crate::ui::chat::{Composer, Line, Log};

use super::hud::Toast;
use super::panels::Panel;
use super::Net;

/// The scrollback and the line being typed.
#[derive(Resource, Default)]
pub struct Chat {
    pub composer: Composer,
    pub log: Log,
    /// Redraw the log's node tree next frame. A log that rebuilt every frame
    /// would allocate a node per line per frame for a screen that changes
    /// when somebody speaks.
    pub dirty: bool,
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

/// The composer's line.
#[derive(Component)]
pub struct ComposerLine;

pub fn setup(mut commands: Commands) {
    // Bottom left, above the build plan. Not centred and not large: chat is
    // read at a glance and must never be the thing in front of the world.
    commands.spawn((
        super::WorldEntity,
        LogRoot,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(96.0),
            left: Val::Px(22.0),
            width: Val::Px(520.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
    commands.spawn((
        super::WorldEntity,
        ComposerLine,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(74.0),
            left: Val::Px(22.0),
            ..default()
        },
        Text::new(""),
        super::ui::font(15.0),
        TextColor(Color::srgba(0.98, 0.94, 0.80, 0.95)),
        Pickable::IGNORE,
    ));
}

/// `T` and `Enter` open the composer; `Enter` sends; `Esc` abandons.
///
/// `keyboard` is `ResMut` for `panels::keys`' reason: **a key this consumed
/// must not reach the system after it.** Every press while the composer is
/// open is consumed, which is what stops a typed line also being a walk, a
/// swing and an inventory toggle.
pub fn keys(
    mut chat: ResMut<Chat>,
    net: NonSend<Net>,
    mut toast: ResMut<Toast>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut chars: MessageReader<KeyboardInput>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    ui: Option<Res<super::panels::Ui>>,
) {
    let panel_up = ui.map(|u| u.panel != Panel::None).unwrap_or(false);

    if !chat.composer.open {
        // A panel already owns the keyboard — the search box is a text field
        // too, and two of them open at once is one of them eating the other's
        // keystrokes.
        if panel_up {
            chars.clear();
            return;
        }
        if keyboard.just_pressed(KeyCode::KeyT) || keyboard.just_pressed(KeyCode::Enter) {
            chat.composer.open = true;
            chat.composer.clear();
            chat.dirty = true;
            keyboard.clear_just_pressed(KeyCode::KeyT);
            keyboard.clear_just_pressed(KeyCode::Enter);
            if let Ok(mut c) = cursor.single_mut() {
                c.grab_mode = CursorGrabMode::None;
                c.visible = true;
            }
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
        send(&mut chat, &net, &mut toast);
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
        c.visible = false;
    }
}

fn send(chat: &mut Chat, net: &Net, toast: &mut Toast) {
    let Some((global, text)) = chat.composer.outgoing() else {
        return; // nothing to say, or only a channel switch
    };
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::chat::encode_chat(text.as_bytes(), global, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            // **The line is not echoed locally.** The server fans it back to
            // this client like any other, and echoing would print it twice —
            // or, worse, print it once when the server rate-limited it away
            // and never sent it at all, which is a client telling a player
            // their message went out when it did not.
            Ok(()) => {}
            Err(e) => toast.say(e.to_string()),
        },
        // `sanitize` already passed inside `outgoing`, so this is a client
        // bug rather than a player one — reported, never swallowed.
        Err(e) => toast.say(format!("that line would not encode ({e:?})")),
    }
}

/// Drain the core's chat ring into the log.
pub fn drain(mut net: NonSendMut<Net>, mut chat: ResMut<Chat>) {
    let own = net.session.core.player_id;
    while let Some((speaker, global, text)) = net.session.core.pop_chat() {
        // Valid UTF-8 by construction (`ChatText` is only built by
        // `sanitize`), but a decode that somehow was not is dropped rather
        // than drawn as replacement characters — this is the one payload on
        // the wire that another player authored.
        let Ok(s) = std::str::from_utf8(text.as_bytes()) else {
            continue;
        };
        chat.log.push(Line {
            speaker,
            global,
            text: s.to_string(),
            own: speaker == own,
        });
        chat.dirty = true;
    }
}

/// Redraw the log and the composer when either changed.
pub fn draw(
    mut commands: Commands,
    mut chat: ResMut<Chat>,
    root: Query<Entity, With<LogRoot>>,
    mut line: Query<&mut Text, With<ComposerLine>>,
) {
    if !chat.dirty {
        return;
    }
    chat.dirty = false;

    if let Ok(mut text) = line.single_mut() {
        text.0 = if chat.composer.open {
            // The remaining count is drawn because a cap the player cannot
            // see is a cap that feels like a broken keyboard.
            format!(
                "say: {}_    ({} left, /g for global, Esc cancels)",
                chat.composer.text,
                chat.composer.remaining()
            )
        } else {
            String::new()
        };
    }

    let Ok(root) = root.single() else {
        return;
    };
    commands.entity(root).despawn_related::<Children>();
    let lines: Vec<(String, bool)> = chat
        .log
        .lines
        .iter()
        .map(|l| (Log::render(l), l.global))
        .collect();
    commands.entity(root).with_children(|c| {
        for (text, global) in lines {
            c.spawn((
                Text::new(text),
                super::ui::font(14.0),
                TextColor(if global {
                    Color::srgba(0.86, 0.88, 0.96, 0.90)
                } else {
                    Color::srgba(0.92, 0.90, 0.84, 0.85)
                }),
                Pickable::IGNORE,
            ));
        }
    });
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
        .add_systems(Update, (drain, draw).chain().run_if(super::world_running))
        .add_systems(OnEnter(super::Screen::Menu), forget);
}
