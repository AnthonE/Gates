//! Sprint runs forward only (`movement::sprints`): backing off or stepping
//! sideways with the key held walks, as in the reference.

use sim_core::input::{InputFrame, BTN_AIM, BTN_SPRINT};
use sim_core::movement::sprints;

fn held(move_x: i8, move_z: i8) -> InputFrame {
    InputFrame {
        buttons: BTN_SPRINT,
        move_x,
        move_z,
        ..InputFrame::default()
    }
}

#[test]
fn sprint_is_forward_and_its_diagonals_only() {
    assert!(sprints(&held(0, 127)));
    assert!(sprints(&held(127, 127)), "forward diagonal runs");
    assert!(sprints(&held(-127, 127)), "forward diagonal runs");
    assert!(!sprints(&held(0, -127)), "backpedal walks");
    assert!(!sprints(&held(127, 0)), "strafe walks");
    assert!(!sprints(&held(-127, -127)), "back diagonal walks");
    assert!(!sprints(&held(127, 60)), "mostly sideways walks");
    assert!(!sprints(&held(0, 0)), "standing is not running");
    let mut drawn = held(0, 127);
    drawn.buttons |= BTN_AIM;
    assert!(!sprints(&drawn), "a drawn bow walks");
    let mut no_key = held(0, 127);
    no_key.buttons = 0;
    assert!(!sprints(&no_key));
}
