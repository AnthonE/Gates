//! Hand revive: the six-second hold is server time, never a client claim.
//! Fixed player slots carry the intent and progress. A released/decayed hold,
//! movement, damage, an occluder or a changed target interrupts it.
use crate::collide::{CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M};
use crate::input::{InputFrame, BTN_ASSIST, BTN_JUMP, BTN_PRIMARY};
use crate::melee::{self, Ray};
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};

/// Reference default, `reference/WOUNDED.md` §2.6; registered in DECISIONS.
pub const ASSIST_TICKS: u16 = 6 * crate::limits::TICK_HZ as u16;
/// **(knob)** Proposed hand reach, `DECISIONS.md` §open, hand revive v0.
pub const ASSIST_REACH_M: f32 = 3.0;

pub fn holding(frame: &InputFrame) -> bool {
    frame.buttons & BTN_ASSIST != 0
        && frame.buttons & (BTN_PRIMARY | BTN_JUMP) == 0
        && frame.move_x == 0
        && frame.move_z == 0
}

pub fn ray(body: &Body, frame: &InputFrame) -> Ray {
    melee::ray(body, frame.yaw, frame.pitch, ASSIST_REACH_M * 1000.0)
}

/// Same cylinder and quantized ray on both targets. The client uses this to
/// offer a prompt; the server additionally checks the whole ray for cover.
pub fn aimed(ray: &Ray, target: &Body) -> Option<f32> {
    let o = ray.at_m(0.0);
    let u = (ray.s.0 / 1000.0, ray.s.1 / 1000.0, ray.s.2 / 1000.0);
    let base = target.qy as f32 * POS_Y_Q;
    melee::cylinder_span(
        o,
        u,
        (target.qx as f32 * POS_XZ_Q, target.qz as f32 * POS_XZ_Q),
        CAPSULE_RADIUS_M,
        base,
        base + CAPSULE_HEIGHT_M,
    )
    .map(|(enter, _)| enter)
}
