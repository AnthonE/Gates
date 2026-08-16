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
//!
//! ## Two angles, and which one everything reads
//!
//! Free look (hold `Left Alt`) is the only thing in this client that lets the
//! camera point somewhere the body is not. It is implemented as a strict
//! split rather than a mode flag consulted in several places, because the
//! failure it invites is a crosshair that disagrees with the server:
//!
//! - **[`Look::yaw`] / [`Look::pitch`] are the BODY's.** They go on the wire,
//!   and every question with a sim answer in it reads them — `set_input`
//!   here, `verbs::resolve`'s pick, the build ghost, the compass strip, the
//!   map's bearing. Free look does not move them at all.
//! - **[`Look::free_yaw`] / [`Look::free_pitch`] are the head's offset**, and
//!   exactly one line adds them to anything: [`place_eye`], on the way into
//!   [`Eye`], which is what the camera and the listener are drawn from.
//!
//! So the invariant is checkable by grep rather than by reasoning: a reader of
//! `Look` follows the body, a reader of `Eye` follows the head, and there is
//! no third case. A capture run pins `frozen`, which disables the mode
//! outright and leaves `place_eye`'s sum an identity — the visual frames
//! cannot move because of this.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim_core::input::{BTN_CROUCH, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};

use crate::look::{self, FREE_LOOK_YAW_LIMIT, MOUSE_RAD_PER_PX, PITCH_LIMIT};

use super::{Eye, Net, EYE_HEIGHT};

/// Free-running view angles, radians. Not sim state: the sim gets the
/// quantized copy below and this is what the camera is drawn from.
///
/// **[`Look::yaw`] and [`Look::pitch`] are the BODY's**, and that is the whole
/// of the free-look design. Everything that talks to the sim — `set_input`
/// below, `verbs::resolve`'s pick, the build ghost — reads those two and is
/// therefore untouched by free look and cannot drift from what the server
/// will do. The head's offset lives in the two fields under them and is added
/// in exactly once, by [`place_eye`], on its way to the camera.
#[derive(Resource, Default)]
pub struct Look {
    pub yaw: f32,
    pub pitch: f32,
    /// The head's yaw offset from the body while free look is held, radians.
    /// Zero whenever it is not.
    pub free_yaw: f32,
    /// The head's pitch offset from the body while free look is held.
    pub free_pitch: f32,
    /// Set by capture mode, which drives the view itself.
    pub frozen: bool,
}

pub use crate::look::{pitch_u8, yaw_u16};

// Ten sources and two `Local`s. Every source is distinct: the session, the
// free view, the cursor, the settings, two input maps, the accumulated motion,
// the accumulated scroll, the sound queue, whether a panel has the pointer,
// and where a swing goes. The two locals are the pointer's edge memory — see
// the cursor block.
#[allow(clippy::too_many_arguments)]
pub fn gather(
    mut net: NonSendMut<Net>,
    mut look: ResMut<Look>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    settings: Res<super::settings::Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut sound: ResMut<super::audio::Sound>,
    // `Option`, because a capture run does not register the menus at all —
    // and a probe harness that could open one is a gate whose frames depend
    // on a keystroke (`render/panels/mod.rs`).
    ui: Option<Res<super::panels::Ui>>,
    // Same shape as `ui`, and the same reason it is optional: a capture run
    // registers neither.
    chat: Option<Res<super::chat::Chat>>,
    // The pointer's memory across frames. A `Local` holding a pure type from
    // `ui::` rather than loose booleans, for that module's reason: the defect
    // it fixes is a SEQUENCE, and a sequence inside a system can only be
    // checked by a windowed run.
    mut pointer: Local<crate::ui::pointer::Pointer>,
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

    // **Free look: the head turns and the body does not.** Held `Left Alt`
    // parks the wire angles where they are and spends the mouse on a camera
    // offset instead, so a player can look over a shoulder while still
    // running — and, the half that matters, while still aiming where the body
    // faces. That is not a compromise forced by the wire; it is what the verb
    // means. A free look that moved the aim would just be a slower look.
    //
    // **The rule is one line: everything that talks to the sim reads the
    // body, and only the camera reads the head.** `Look`'s header states
    // which fields are which, and `place_eye` is the single place the offset
    // is added in.
    //
    // Not frozen (a capture run drives the view itself) and not while a panel
    // owns the pointer. The offset is dropped the instant any of that stops
    // being true, so releasing returns the view to the body rather than
    // stranding a player looking sideways with no key held.
    let free = !look.frozen && !panel_open && keys.pressed(KeyCode::AltLeft);
    if !free {
        look.free_yaw = 0.0;
        look.free_pitch = 0.0;
    }

    // Pointer lock on click — the browser client's contract, which players
    // already know from every other game. **Releasing it on Escape is no
    // longer here**: Escape opens the Esc menu (`pause::open`), and that
    // screen owns letting the pointer go and taking it back, because a
    // released pointer with no menu under it was a state the player could not
    // tell from a hang. A panel is the same rule one level down — it releases
    // the pointer because it has something under it to click.
    //
    // **And it has to TAKE IT BACK, which is what this block was missing.**
    // Reported by the operator, 2026-08-16, on the build wheel: *"used the
    // window to pick pieces and close it my mouse was still up and i couldnt
    // look around right without clicking"*. The release above had no matching
    // restore anywhere, so the only route back to mouse-look was the
    // `just_pressed(Left)` arm at the top — and with a building plan in hand
    // that click **places a piece**. The workaround for a stuck cursor was to
    // build something.
    //
    // The other two pointer owners already got this right and are the
    // precedent rather than an invention: `chat::close` re-locks with the
    // comment *"so getting out of chat does not need a click that would also
    // swing the axe"*, and `pause` takes it back on resume. Panels were the
    // one release in the client with no partner.
    //
    // **Restored on the edge, and only if the player had it.** A fresh world
    // is uncaptured until the first click — the contract the settings screen
    // advertises as *"click to capture the pointer"* — so opening and closing
    // a panel before ever clicking must not silently capture it.
    //
    // The whole decision is `ui::pointer::Pointer::step`, which is pure and
    // enumerates its transitions in a test. This block does exactly what it
    // is told and knows nothing about the rule.
    //
    // **Stepped before the window is asked for, not inside the `if let`.**
    // The state machine tracks an edge, so it has to advance on every frame
    // this system runs — including a frame where the query finds no window.
    // Stepping inside would stall the edge exactly when it is hardest to
    // notice, and the pointer would come back stuck on the next real frame.
    let locked = cursor
        .single()
        .map(|c| c.grab_mode == CursorGrabMode::Locked)
        .unwrap_or(false);
    let want = pointer.step(locked, panel_open, mouse.just_pressed(MouseButton::Left));
    if let Ok(mut c) = cursor.single_mut() {
        match want {
            crate::ui::pointer::Grab::Lock => {
                c.grab_mode = CursorGrabMode::Locked;
                c.visible = false;
            }
            crate::ui::pointer::Grab::Release => {
                c.grab_mode = CursorGrabMode::None;
                c.visible = true;
            }
            crate::ui::pointer::Grab::Leave => {}
        }
        if !look.frozen && !panel_open && c.grab_mode == CursorGrabMode::Locked {
            let d = motion.delta;
            // Sensitivity scales the free-running radians BEFORE the
            // quantization below, never the quantization itself — see
            // `settings`'s header. Invert flips the pitch delta only; a yaw
            // inversion is not a setting any reference offers.
            let rad = MOUSE_RAD_PER_PX * settings.sensitivity;
            if free {
                // The head. Same sensitivity and the same sign as the body —
                // `look::free_yaw_after` shares `yaw_after`'s subtraction and
                // a test pins the two together, because a mode that turned
                // the other way would be this module's original bug hiding
                // somewhere nobody re-checks.
                look.free_yaw =
                    crate::look::free_yaw_after(look.free_yaw, d.x, rad, FREE_LOOK_YAW_LIMIT);
                look.free_pitch = crate::look::free_pitch_after(
                    look.free_pitch,
                    look.pitch,
                    d.y,
                    rad,
                    settings.invert_look,
                    PITCH_LIMIT,
                );
            } else {
                look.yaw = crate::look::yaw_after(look.yaw, d.x, rad);
                look.pitch = crate::look::pitch_after(
                    look.pitch,
                    d.y,
                    rad,
                    settings.invert_look,
                    PITCH_LIMIT,
                );
            }
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
    // **Crouch crosses the wire and the sim ignores it, deliberately.**
    // `BTN_CROUCH` has been declared and inside `BTN_MASK` since v0 — so the
    // server accepts the bit rather than refusing the frame — and
    // `movement::step` does not read it (`sim-core/input.rs`: *"crouch has no
    // sim effect yet — it lands with the combat pass"*). Binding the key now
    // costs nothing and moves nothing: both sides run the same `movement::step`
    // over a bit neither consults, so prediction cannot drift on it and no
    // golden moves.
    //
    // It is bound anyway because the alternative is worse. The key belongs to
    // crouch in every reference a player arrives from; leaving it unbound
    // means the combat pass has to remember to add it, and until then Ctrl is
    // free for something else to take and then have to give back. What must
    // NOT happen is this being advertised as a working verb — the settings
    // screen's bind row says so in as many words.
    //
    // `ControlRight` too: `panels/inv.rs` already treats the two as one
    // modifier, and a split would be a keyboard the player has to think about.
    if keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight) {
        buttons |= BTN_CROUCH;
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
        // The score's *small* bump, and it is here for the same reason the
        // cue is: this is the only place that knows a swing was a swing and
        // not a click that closed a panel. `reference/AUDIO.md` §8's
        // published order puts a weapon in play at the bottom — ours is the
        // swing rather than the equip because a swing is an event and an
        // equipped-weapon state is not (`sound::music::BUMP_SWING`).
        sound.music.bump(crate::sound::music::BUMP_SWING);
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
    // The wheel walks the same six slots, and it wraps
    // (`ui::slots::hotbar_scrolled` owns the arithmetic and the direction).
    //
    // **The units are not interchangeable and that is the whole subtlety.**
    // A mouse reports `Line` — one notch is `y = ±1`, and a fast spin really
    // is several slots, so the magnitude is honoured. A trackpad reports
    // `Pixel`, tens of units per flick, and honouring THAT magnitude would
    // sweep the whole bar off one gesture; it gets one slot per frame in the
    // direction of travel instead. `panels/craft.rs` converts the other way —
    // lines into pixels — because it is scrolling a list and cares about
    // distance. This cares about counting.
    //
    // Below the panel guard above, so a wheel over an open crafting list
    // still scrolls the list rather than also changing what is in your hand.
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y.round() as i32,
        MouseScrollUnit::Pixel => scroll.delta.y.signum() as i32,
    };
    if notches != 0 {
        // Negated: a wheel pushed away from the player reads `+y` and walks
        // toward slot 1.
        sel = crate::ui::slots::hotbar_scrolled(sel, -notches);
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
    // **The one place the head's offset is added to the body's angles.**
    // Both are zero unless free look is held, so this is the identity it has
    // always been the rest of the time. Adding it anywhere else — or reading
    // `Eye` for anything the sim will answer for — is what would put the
    // crosshair somewhere the server does not agree with.
    eye.yaw = look.yaw + look.free_yaw;
    eye.pitch = look.pitch + look.free_pitch;
}
