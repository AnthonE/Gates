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
//! `i8`. The sim's movement only ever reads the top 8 bits of yaw
//! (`yaw_lut.rs`), so a client that sent a finer bearing would be predicting
//! on a heading the server cannot face.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim_core::input::{BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};

use super::{Eye, Net, EYE_HEIGHT};

/// Radians per pixel of mouse travel (`DECISIONS.md` §open, client cosmetics).
pub const MOUSE_RAD_PER_PX: f32 = 0.0022;
/// How close to straight up or down the view may get, radians from level.
/// The browser's `PITCH_LIMIT`; a pitch that reaches the pole makes the yaw
/// basis degenerate.
pub const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.02;

/// Free-running view angles, radians. Not sim state: the sim gets the
/// quantized copy below and this is what the camera is drawn from.
#[derive(Resource, Default)]
pub struct Look {
    pub yaw: f32,
    pub pitch: f32,
    /// Set by capture mode, which drives the view itself.
    pub frozen: bool,
}

/// Wire yaw from a free-running radian yaw.
pub fn yaw_u16(yaw: f32) -> u16 {
    let t = (yaw / std::f32::consts::TAU).rem_euclid(1.0);
    (t * 65536.0) as u32 as u16
}

/// Wire pitch: 128 level, 255 straight up.
pub fn pitch_u8(pitch: f32) -> u8 {
    let v = ((pitch / std::f32::consts::PI + 0.5) * 255.0).round();
    v.clamp(0.0, 255.0) as u8
}

pub fn gather(
    mut net: NonSendMut<Net>,
    mut look: ResMut<Look>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    // `Option`, because a capture run does not register the menus at all —
    // and a probe harness that could open one is a gate whose frames depend
    // on a keystroke (`render/ui/mod.rs`).
    ui: Option<Res<super::ui::Ui>>,
) {
    // A panel owns the pointer while it is up: the cursor comes back, the
    // view stops turning, and the movement axes go to zero. A player
    // dragging an item across a container is not also walking into a wall —
    // and the alternative, sending the keys anyway, means every letter typed
    // into the search box is also a step.
    let menu_open = ui.map(|u| u.panel.grabs_pointer()).unwrap_or(false);

    // Pointer lock on click, released on Escape — the browser client's
    // contract, which players already know from every other game.
    if let Ok(mut c) = cursor.single_mut() {
        if mouse.just_pressed(MouseButton::Left) && !menu_open {
            c.grab_mode = CursorGrabMode::Locked;
            c.visible = false;
        }
        if keys.just_pressed(KeyCode::Escape) || menu_open {
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
        if !look.frozen && !menu_open && c.grab_mode == CursorGrabMode::Locked {
            let d = motion.delta;
            look.yaw += d.x * MOUSE_RAD_PER_PX;
            look.pitch = (look.pitch - d.y * MOUSE_RAD_PER_PX).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
    }

    if menu_open {
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

    let mut mx = 0i32;
    let mut mz = 0i32;
    if keys.pressed(KeyCode::KeyW) {
        mz += 1;
    }
    if keys.pressed(KeyCode::KeyS) {
        mz -= 1;
    }
    if keys.pressed(KeyCode::KeyD) {
        mx += 1;
    }
    if keys.pressed(KeyCode::KeyA) {
        mx -= 1;
    }

    let mut buttons = 0u8;
    if keys.pressed(KeyCode::ShiftLeft) {
        buttons |= BTN_SPRINT;
    }
    if keys.pressed(KeyCode::Space) {
        buttons |= BTN_JUMP;
    }
    if mouse.pressed(MouseButton::Left) {
        buttons |= BTN_PRIMARY;
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
        (mx * 127) as i8,
        (mz * 127) as i8,
        sel,
    );
}

/// One network frame per rendered frame, then place the eye.
///
/// The local body comes from the PREDICTOR so it answers input on the frame
/// it was pressed; every other body comes from the interpolator at the render
/// tick, so it is smooth and late rather than jittery and early. `pump` never
/// awaits — a frame that waits on a socket is a dropped frame.
pub fn place_eye(mut net: NonSendMut<Net>, mut eye: ResMut<Eye>, look: Res<Look>, time: Res<Time>) {
    let dt_ms = time.delta_secs_f64() * 1000.0;
    net.session.pump(dt_ms);

    let [x, y, z] = net.session.core.predict.render_position();
    eye.pos = Vec3::new(x, y + EYE_HEIGHT, z);
    eye.yaw = look.yaw;
    eye.pitch = look.pitch;
}
