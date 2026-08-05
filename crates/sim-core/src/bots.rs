//! Deterministic synthetic players: the input source for `test_alloc_zero`,
//! `test_replay`, `test_parity_wasm`, and later the server's `bots` bin.
//! Lives in sim-core so native and wasm drive byte-identical inputs.

use crate::input::{InputFrame, BTN_JUMP, BTN_PRIMARY, BTN_SPRINT};
use crate::rng::Pcg32;

/// One jump every `JUMP_PERIOD` frames, and the reason it is a period rather
/// than a roll.
///
/// Every other button here spends an `rng` draw, so adding a fourth would
/// shift the stream and move `parity`, `combat`, `bags`, `test_replay` and
/// `test_alloc_zero` all at once — leaving no way to tell a digest that moved
/// because bodies now leave the ground from a digest that moved because every
/// bot re-rolled its whole life. Keyed off `seq` instead, the stream is
/// byte-identical to before this bit existed and the digests move for exactly
/// one reason.
///
/// 128 frames is ~4.3 s at 30 Hz against a ~0.7 s arc (`2·JUMP_SPEED/GRAVITY`
/// = 21 ticks), so a bot is airborne roughly a sixth of the time: enough that
/// launch, flight and landing are all permanently on the parity surface,
/// little enough that it does not hollow out the coverage the other probes
/// depend on — `probe_parity`'s bots still have to stand still enough to fill
/// inventories, stand pieces and reach the upgrade rung. It also divides
/// `PARITY_TICKS`' 16-tick window at `seq` 0, so all 10,000 of that probe's
/// sequences carry a launch even though none of them is long enough to carry
/// a landing; `probe_combat` (256) and `probe_bags` (600) carry whole arcs.
const JUMP_PERIOD: u16 = 128;

/// Random-walk input frame: `yaw` drifts around the previous heading,
/// mostly-forward movement, bursts of sprint and strafe, and the primary
/// button held about a third of the time — bots swing at whatever they
/// wander past, so the alloc/replay/parity gates walk the gather path
/// too. They jump on a fixed period for the same reason, so those gates
/// walk the vertical too. Allocation-free.
pub fn bot_frame(rng: &mut Pcg32, prev_yaw: u16, seq: u16) -> InputFrame {
    let yaw_step = rng.next_bounded(8192) as i32 - 4096;
    let yaw = (prev_yaw as i32).wrapping_add(yaw_step) as u16;
    let forward = 40 + rng.next_bounded(88) as i32; // 40..=127, keeps moving
    let strafe = rng.next_bounded(255) as i32 - 127;
    let mut buttons = if rng.next_bounded(4) == 0 {
        BTN_SPRINT
    } else {
        0
    };
    if rng.next_bounded(3) == 0 {
        buttons |= BTN_PRIMARY;
    }
    // Deliberately after the draws and deliberately not one: see JUMP_PERIOD.
    if seq.is_multiple_of(JUMP_PERIOD) {
        buttons |= BTN_JUMP;
    }
    InputFrame {
        seq,
        buttons,
        yaw,
        pitch: rng.next_bounded(256) as u8,
        move_x: strafe as i8,
        move_z: forward as i8,
        // Wander the hotbar too, so held-item selection is inside the
        // alloc/replay/parity surface.
        sel: rng.next_bounded(6) as u8,
    }
}
