//! The clip key: `F9` makes a video of the last half minute.
//!
//! The session keeps its last minute (`crate::clip`); this hands it on. In a
//! page the tape goes to the page (`window.gatesClip`), which draws it with
//! this same client in a second canvas (`client-web`'s `Gates::film`) and
//! encodes it on the player's GPU. On the desktop it is written beside the
//! screenshots and a second `gates --clip` draws it, so the game the player
//! is in never waits on a frame of it. Nothing leaves the machine either way.

use bevy::prelude::*;

use super::Net;

/// The key. Next to the screenshot's F12 and reserved with it
/// (`binds::RESERVED`).
pub const KEY: KeyCode = KeyCode::F9;

pub fn take(
    keys: Res<ButtonInput<KeyCode>>,
    net: Option<NonSend<Net>>,
    monitors: Query<&bevy::window::Monitor>,
    mut toast: ResMut<super::hud::Toast>,
) {
    if !keys.just_pressed(KEY) {
        return;
    }
    let Some(net) = net else {
        toast.warn("clip: join a world first");
        return;
    };
    let Some(tape) = net.session.clip_tape(crate::clip::CLIP_S) else {
        toast.warn("clip: nothing to clip yet");
        return;
    };
    let held = net.session.clip_held_s().min(crate::clip::CLIP_S);
    // The film draws in a window of its own; on a screen shorter than the
    // video it draws smaller and is scaled up (`--max-height`).
    let screen = monitors.iter().map(|m| m.physical_height).max();
    hand_over(tape, held, screen, &mut toast);
}

#[cfg(target_arch = "wasm32")]
fn hand_over(tape: Vec<u8>, held: f64, _screen: Option<u32>, toast: &mut super::hud::Toast) {
    use wasm_bindgen::{JsCast, JsValue};
    let global = js_sys::global();
    let f = js_sys::Reflect::get(&global, &JsValue::from_str("gatesClip"))
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    let Some(f) = f else {
        toast.warn("clip: this page cannot make videos");
        return;
    };
    let bytes = js_sys::Uint8Array::from(&tape[..]);
    match f.call2(&global, &bytes, &JsValue::from(held)) {
        Ok(_) => toast.say(format!("clip: the last {held:.0} s — making your video")),
        Err(e) => {
            error!("gates: the page's gatesClip threw: {e:?}");
            toast.warn("clip failed");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn hand_over(tape: Vec<u8>, held: f64, screen: Option<u32>, toast: &mut super::hud::Toast) {
    let Some(dir) = crate::shot::dir() else {
        toast.warn("no screenshot directory — set GATES_SHOTS_DIR");
        return;
    };
    let stamp = crate::shot::stamp(crate::shot::now_secs());
    let path = dir.join(format!("{}clip-{stamp}.rec", crate::shot::PREFIX));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &tape)) {
        error!("gates: cannot write {}: {e}", path.display());
        toast.warn("clip failed to save");
        return;
    }
    info!("gates: clip → {}", path.display());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // Drawn by a second copy of this binary: the film owns its window and its
    // frame clock, and this one is still playing.
    let drawn = std::env::current_exe().and_then(|exe| {
        let mut film = std::process::Command::new(exe);
        film.arg("--clip").arg(&path);
        if let Some(h) = screen {
            // Room for a title bar and a task bar.
            film.arg("--max-height")
                .arg(h.saturating_sub(160).max(240).to_string());
        }
        film.stdout(std::process::Stdio::null()).spawn()
    });
    match drawn {
        Ok(_) => toast.say(format!("clip: the last {held:.0} s — {name} → .mp4")),
        Err(e) => {
            error!("gates: cannot start the clip's film: {e}");
            toast.say(format!(
                "clip saved: {name} (gates --clip to make the video)"
            ));
        }
    }
}
