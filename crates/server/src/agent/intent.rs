//! What a skill wants the body to do this frame, before hands turn it into
//! an input frame. A skill says where to look and how to move; it never
//! writes yaw or pitch itself, so one place ([`super::hands`]) can hold
//! every turn to a person's speed and aim.

/// Where the eyes go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// Leave the view where it is.
    Keep,
    /// Face this wire yaw, eyes level.
    Heading(u16),
    /// Look at a point in the world, metres.
    Point([f32; 3]),
    /// Look at a body this agent has seen, at a part of it (head, chest,
    /// legs, as `sim_core::collide::Part` bits), where perception last
    /// had it in sight.
    Body { id: u32, part: u8 },
}

/// One frame's worth of a skill's wishes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    pub look: Look,
    /// Strafe and forward, relative to the view the hands hold.
    pub move_x: i8,
    pub move_z: i8,
    /// Walk along this world bearing (wire yaw) at full stride, whatever
    /// the view: the hands turn it into strafe and forward for the view
    /// they actually hold this frame. Backing away from a pursuer while
    /// still turning to face it must not walk into it. Overrides the axes.
    pub travel: Option<u16>,
    /// The buttons held this frame. A frame states every held button, as a
    /// human's does, so a skill that keeps a bow drawn says so each frame.
    pub buttons: u8,
    /// The hotbar slot to hold, or `None` to keep the one in hand: the sim
    /// equips by slot every tick, so a skill that only moves must not
    /// swap the weapon out from under the skill that chose it.
    pub sel: Option<u8>,
}

impl Intent {
    /// Stand still, look where you are looking, keep the slot in hand.
    pub const IDLE: Self = Self {
        look: Look::Keep,
        move_x: 0,
        move_z: 0,
        travel: None,
        buttons: 0,
        sel: None,
    };

    /// Walk along a heading, facing it. The legs take the bearing at once
    /// and the eyes follow at the hands' pace, so a body that turns away
    /// from deep water or a blow does not first walk on into it while the
    /// view comes round.
    pub const fn walk(yaw: u16) -> Self {
        Self {
            look: Look::Heading(yaw),
            travel: Some(yaw),
            ..Self::IDLE
        }
    }
}

/// The wire pitch of a level gaze.
pub const LEVEL_PITCH: u8 = 128;

/// The wire yaw that faces along `(dx, dz)`, on the 256-step grid the sim
/// reads (`yaw >> 8`).
pub fn yaw_toward(dx: f32, dz: f32) -> u16 {
    (((dx.atan2(dz) / std::f32::consts::TAU * 256.0).round() as i32).rem_euclid(256) as u16) << 8
}

/// The wire pitch that rises `dy` over a horizontal `distance`: 0 straight
/// down, 255 straight up.
pub fn pitch_toward(dy: f32, distance: f32) -> u8 {
    ((dy.atan2(distance) / std::f32::consts::PI + 0.5) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearings_are_on_the_wire_grid() {
        // +Z is yaw 0 and +X a quarter turn (the sim's `yaw_dir`).
        assert_eq!(yaw_toward(0.0, 1.0), 0);
        assert_eq!(yaw_toward(1.0, 0.0), 1 << 14);
        assert_eq!(yaw_toward(-1.0, -0.001) & 0xff, 0);
        assert_eq!(pitch_toward(0.0, 5.0), LEVEL_PITCH);
        assert!(pitch_toward(10.0, 10.0) > LEVEL_PITCH);
        assert!(pitch_toward(-8.0, 10.0) < LEVEL_PITCH);
    }
}
