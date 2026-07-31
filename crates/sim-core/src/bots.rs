//! Deterministic synthetic players: the input source for `test_alloc_zero`,
//! `test_replay`, `test_parity_wasm`, and later the server's `bots` bin.
//! Lives in sim-core so native and wasm drive byte-identical inputs.

use crate::input::{InputFrame, BTN_SPRINT};
use crate::rng::Pcg32;

/// Random-walk input frame: `yaw` drifts around the previous heading,
/// mostly-forward movement, bursts of sprint and strafe. Allocation-free.
pub fn bot_frame(rng: &mut Pcg32, prev_yaw: u16, seq: u16) -> InputFrame {
    let yaw_step = rng.next_bounded(8192) as i32 - 4096;
    let yaw = (prev_yaw as i32).wrapping_add(yaw_step) as u16;
    let forward = 40 + rng.next_bounded(88) as i32; // 40..=127, keeps moving
    let strafe = rng.next_bounded(255) as i32 - 127;
    let buttons = if rng.next_bounded(4) == 0 {
        BTN_SPRINT
    } else {
        0
    };
    InputFrame {
        seq,
        buttons,
        yaw,
        pitch: rng.next_bounded(256) as u8,
        move_x: strafe as i8,
        move_z: forward as i8,
    }
}
