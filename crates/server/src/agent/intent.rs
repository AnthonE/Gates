//! What a skill wants the body to do this frame, before hands turn it into
//! an input frame. A skill says where to look and how to move; it never
//! writes yaw or pitch itself, so one place (the hands) can hold every
//! turn to a person's speed and aim.

use sim_core::input::InputFrame;

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
    /// legs). Resolving it needs what perception tracked of that body, so
    /// the plain resolver below keeps the current view.
    Body { id: u32, part: u8 },
}

/// One frame's worth of a skill's wishes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    pub look: Look,
    pub move_x: i8,
    pub move_z: i8,
    pub buttons: u8,
    pub sel: u8,
}

impl Intent {
    /// Stand still, look where you are looking, hold slot 0.
    pub const IDLE: Self = Self {
        look: Look::Keep,
        move_x: 0,
        move_z: 0,
        buttons: 0,
        sel: 0,
    };

    /// Walk forward along a heading.
    pub const fn walk(yaw: u16) -> Self {
        Self {
            look: Look::Heading(yaw),
            move_z: 127,
            ..Self::IDLE
        }
    }

    /// The frame this intent asks for, laid over `base` (which carries the
    /// sequence number and the view to keep). `eye` is where this body
    /// looks from, for [`Look::Point`].
    pub fn frame(&self, base: InputFrame, eye: [f32; 3]) -> InputFrame {
        let (yaw, pitch) = match self.look {
            Look::Keep | Look::Body { .. } => (base.yaw, base.pitch),
            Look::Heading(yaw) => (yaw, LEVEL_PITCH),
            Look::Point([x, y, z]) => {
                let (dx, dy, dz) = (x - eye[0], y - eye[1], z - eye[2]);
                (yaw_toward(dx, dz), pitch_toward(dy, dx.hypot(dz)))
            }
        };
        InputFrame {
            yaw,
            pitch,
            move_x: self.move_x,
            move_z: self.move_z,
            buttons: self.buttons,
            sel: self.sel,
            ..base
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

    fn base() -> InputFrame {
        InputFrame {
            seq: 7,
            yaw: 0x4000,
            pitch: 90,
            sel: 3,
            ..InputFrame::default()
        }
    }

    #[test]
    fn keep_holds_the_view_and_takes_the_rest_from_the_intent() {
        let f = Intent {
            buttons: sim_core::input::BTN_PRIMARY,
            sel: 2,
            ..Intent::IDLE
        }
        .frame(base(), [0.0; 3]);
        assert_eq!((f.seq, f.yaw, f.pitch), (7, 0x4000, 90));
        assert_eq!(
            (f.buttons, f.sel, f.move_z),
            (sim_core::input::BTN_PRIMARY, 2, 0)
        );
    }

    #[test]
    fn a_heading_looks_level_and_a_body_keeps_the_view_until_tracked() {
        let f = Intent::walk(0x8000).frame(base(), [0.0; 3]);
        assert_eq!((f.yaw, f.pitch, f.move_z), (0x8000, LEVEL_PITCH, 127));
        let f = Intent {
            look: Look::Body { id: 9, part: 0 },
            ..Intent::IDLE
        }
        .frame(base(), [0.0; 3]);
        assert_eq!((f.yaw, f.pitch), (0x4000, 90));
    }

    #[test]
    fn a_point_is_faced_on_the_wire_grid() {
        let eye = [10.0, 2.0, 10.0];
        let look = |p| {
            Intent {
                look: Look::Point(p),
                ..Intent::IDLE
            }
            .frame(base(), eye)
        };
        // +Z is yaw 0 and +X a quarter turn (the sim's `yaw_dir`).
        let ahead = look([10.0, 2.0, 20.0]);
        assert_eq!((ahead.yaw, ahead.pitch), (0, LEVEL_PITCH));
        let (fx, fz) = sim_core::yaw_dir(look([20.0, 2.0, 10.0]).yaw);
        assert!(fx > 0.99 && fz.abs() < 0.01);
        let up = look([10.0, 12.0, 20.0]);
        assert!(up.pitch > LEVEL_PITCH && up.pitch < 255);
        assert!(look([10.0, -8.0, 20.0]).pitch < LEVEL_PITCH);
    }
}
