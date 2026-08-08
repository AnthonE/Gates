//! Keyboard and mouse into `ClientCore::set_input`, and the eye out of the
//! predictor.
//!
//! **The one door.** This is the only place in the render path that writes
//! anything the sim will read, and it writes exactly one thing: an input
//! frame. Everything else in `render/` is a pure read of the core or of
//! `sim_core`.
//!
//! Quantization is the browser's, to the bit, because **both sides must
//! quantize or prediction drifts by rounding** (`CLAUDE.md` traps, the
//! quantize-both-sides law): yaw is a `u16` where 0 faces +Z and increasing
//! turns toward +X, pitch is a `u8` with 128 level, and the move axes are
//! `i8`.
//!
//! **The signs are not this file's to choose.** Which way a mouse push and a
//! strafe key point is [`crate::look`]'s, because both were inverted and the
//! arithmetic that says so has to be callable without a window. This file
//! reads keys and calls that module; it does not know which way is right.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim_core::input::{BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};

use crate::look::{self, MOUSE_RAD_PER_PX, PITCH_LIMIT};

use super::{Eye, Net, EYE_HEIGHT};

/// Free-running view angles, radians. Not sim state: the sim gets the
/// quantized copy below and this is what the camera is drawn from.
#[derive(Resource, Default)]
pub struct Look {
    pub yaw: f32,
    pub pitch: f32,
    /// Set by capture mode, which drives the view itself.
    pub frozen: bool,
}

pub use crate::look::{pitch_u8, yaw_u16};

// Nine, and the ninth is the sound queue. Every one is a distinct source this
// frame reads: the session, the free view, the cursor, the settings, two input
// maps, the accumulated motion, whether a panel has the pointer, and where a
// swing goes.
#[allow(clippy::too_many_arguments)]
pub fn gather(
    mut net: NonSendMut<Net>,
    mut look: ResMut<Look>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    settings: Res<super::settings::Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut sound: ResMut<super::audio::Sound>,
    // `Option`, because a capture run does not register the menus at all —
    // and a probe harness that could open one is a gate whose frames depend
    // on a keystroke (`render/panels/mod.rs`).
    ui: Option<Res<super::panels::Ui>>,
    // Same shape as `ui`, and the same reason it is optional: a capture run
    // registers neither.
    chat: Option<Res<super::chat::Chat>>,
) {
    // An in-game panel owns the pointer while it is up: the cursor comes
    // back, the view stops turning, and the movement axes go to zero. A
    // player dragging an item across a container is not also walking into a
    // wall — and the alternative, sending the keys anyway, means every letter
    // typed into the search box is also a step.
    //
    // **The panels are the only pointer owner left in this file.** The Esc
    // menu is a whole `Screen` and `gather` does not run on it; a panel is
    // drawn over a running world, so the world's own input has to stand down
    // while one is open, and this is where that happens.
    // **The chat composer counts.** Without it, typing "we should build here"
    // walks you forward, swings twice, eats whatever is in slot 3 and opens
    // the inventory. A text field is a text field whether it is inside a
    // panel or floating over the world.
    let panel_open = ui.map(|u| u.panel.grabs_pointer()).unwrap_or(false)
        || chat.map(|c| c.open()).unwrap_or(false);

    // Pointer lock on click — the browser client's contract, which players
    // already know from every other game. **Releasing it on Escape is no
    // longer here**: Escape opens the Esc menu (`pause::open`), and that
    // screen owns letting the pointer go and taking it back, because a
    // released pointer with no menu under it was a state the player could not
    // tell from a hang. A panel is the same rule one level down — it releases
    // the pointer because it has something under it to click.
    if let Ok(mut c) = cursor.single_mut() {
        if mouse.just_pressed(MouseButton::Left) && !panel_open {
            c.grab_mode = CursorGrabMode::Locked;
            c.visible = false;
        }
        if panel_open {
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
        if !look.frozen && !panel_open && c.grab_mode == CursorGrabMode::Locked {
            let d = motion.delta;
            // Sensitivity scales the free-running radians BEFORE the
            // quantization below, never the quantization itself — see
            // `settings`'s header. Invert flips the pitch delta only; a yaw
            // inversion is not a setting any reference offers.
            let rad = MOUSE_RAD_PER_PX * settings.sensitivity;
            look.yaw = crate::look::yaw_after(look.yaw, d.x, rad);
            look.pitch =
                crate::look::pitch_after(look.pitch, d.y, rad, settings.invert_look, PITCH_LIMIT);
        }
    }

    if panel_open {
        // The input frame still goes out — the sim needs one every tick and
        // a client that stopped sending would be a client standing still for
        // a different reason. It goes out EMPTY of movement and buttons,
        // with the view angles unchanged, which is exactly "standing here".
        let sel = net.sel;
        net.session
            .core
            .set_input(0, yaw_u16(look.yaw), pitch_u8(look.pitch), 0, 0, sel);
        return;
    }

    // Physical intent — forward and rightward, as the player means them.
    // Turning that into the wire's axes is `look::move_axes`, which is where
    // the sim's left-handed `move_x` is answered for.
    let mut fwd = 0i32;
    let mut right = 0i32;
    if keys.pressed(KeyCode::KeyW) {
        fwd += 1;
    }
    if keys.pressed(KeyCode::KeyS) {
        fwd -= 1;
    }
    if keys.pressed(KeyCode::KeyD) {
        right += 1;
    }
    if keys.pressed(KeyCode::KeyA) {
        right -= 1;
    }
    let (move_x, move_z) = look::move_axes(fwd, right);

    let mut buttons = 0u8;
    if keys.pressed(KeyCode::ShiftLeft) {
        buttons |= BTN_SPRINT;
    }
    if keys.pressed(KeyCode::Space) {
        buttons |= BTN_JUMP;
    }
    // **The swing is held-item modal.** Left click means "place" with a
    // building plan and "repair" with a hammer, and neither item has an
    // attack to lose — the reference lists the hammer's damage total as 0
    // and the plan has no damage stats at all. Without this the same press
    // would place a foundation AND send a swing, which the sim answers with
    // a gather attempt on whatever is in front of you.
    let core = &net.session.core;
    let hand = crate::ui::hold::held_in_hand(&core.catalog, &core.inv, net.sel);
    let swings = !hand.opens_a_wheel();
    if swings && mouse.pressed(MouseButton::Left) {
        buttons |= BTN_PRIMARY;
    }
    // The swing, heard here rather than in `render/audio.rs` because this is
    // the only place that knows a panel is not eating the click — the
    // `panel_open` return above is what makes closing an inventory not also
    // swing an axe. `just_pressed`, not `pressed`: the cue's own cooldown
    // paces a held button, and a per-frame push would spend the whole cue
    // queue on one held mouse button.
    if swings && mouse.just_pressed(MouseButton::Left) {
        sound.play(crate::sound::mixer::Request::own(crate::sound::Cue::Swing));
    }

    // Hotbar 1–6. `set_input` clamps into range, so an out-of-range key
    // cannot reach the wire.
    let mut sel = net.sel;
    for (i, k) in [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
    ]
    .iter()
    .enumerate()
    {
        if keys.just_pressed(*k) {
            sel = i as u8;
        }
    }
    net.sel = sel;

    net.session.core.set_input(
        buttons,
        yaw_u16(look.yaw),
        pitch_u8(look.pitch),
        move_x,
        move_z,
        sel,
    );
}

/// One network frame per rendered frame, then place the eye.
///
/// The local body comes from the PREDICTOR so it answers input on the frame
/// it was pressed; every other body comes from the interpolator at the render
/// tick, so it is smooth and late rather than jittery and early. `pump` never
/// awaits — a frame that waits on a socket is a dropped frame.
///
/// **The predictor is not a position until the server has given it one.**
/// `Predictor::started` is false from the welcome — which carries a seed and
/// no spawn — until the first snapshot carrying our own entity, and until
/// then `render_position()` returns `Body::default()`'s, the world origin.
/// That is a real place on this island rather than a sentinel, so writing it
/// would silently aim every ring builder at a neighbourhood the server never
/// named. The flag is what `render::world_placed` gates the whole `Stream`
/// set on; `pos` is simply left at whatever it was, which on a fresh connect
/// is `Eye::default()` and on a reconnect is `world_teardown`'s reset.
pub fn place_eye(
    mut net: NonSendMut<Net>,
    mut eye: ResMut<Eye>,
    look: Res<Look>,
    time: Res<Time>,
    // The false→true edge, logged once. A wait with no observable is a wait
    // nobody can tell from a hang, and this one gates the whole world.
    mut announced: Local<bool>,
) {
    let dt_ms = time.delta_secs_f64() * 1000.0;
    net.session.pump(dt_ms);

    eye.placed = net.session.core.predict.started;
    if !eye.placed {
        // Cleared here so a reconnect announces its own placement:
        // `world_teardown` resets `Eye`, but it cannot reach a `Local`.
        *announced = false;
        return;
    }

    let [x, y, z] = net.session.core.predict.render_position();
    if !*announced {
        *announced = true;
        info!("gates: the shard placed us at {x:.1}, {y:.1}, {z:.1} — building the world");
    }
    eye.pos = Vec3::new(x, y + EYE_HEIGHT, z);
    eye.yaw = look.yaw;
    eye.pitch = look.pitch;
}
