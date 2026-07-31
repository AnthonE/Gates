//! The input frame (DESIGN.md §5.4): seq, buttons, yaw u16, pitch u8,
//! move vec 2×i8. The wire adds client_tick and redundancy; the sim
//! consumes exactly one effective frame per player per tick and keeps the
//! last one applied — an empty server-side buffer reuses it (NETCODE.md §4),
//! and replay reproduces that for free because the frame is sim state.

/// Button bits (ALPHA.md §1 sizes sprint/crouch into the field; crouch has
/// no sim effect yet — it lands with the combat pass). PRIMARY is the
/// swing/use button: gather now (M1), attack with M2. A new bit in an
/// already-sized field — the wire layout does not move.
pub const BTN_SPRINT: u8 = 1 << 0;
pub const BTN_CROUCH: u8 = 1 << 1;
pub const BTN_PRIMARY: u8 = 1 << 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InputFrame {
    pub seq: u16,
    pub buttons: u8,
    /// View yaw; the high byte indexes the shared yaw LUT for movement.
    pub yaw: u16,
    pub pitch: u8,
    /// Strafe axis, +right, -127..=127.
    pub move_x: i8,
    /// Forward axis, +forward, -127..=127.
    pub move_z: i8,
}
