//! The input frame (DESIGN.md §5.4): seq, buttons, yaw u16, pitch u8,
//! move vec 2×i8. The wire adds client_tick and redundancy; the sim
//! consumes exactly one effective frame per player per tick and keeps the
//! last one applied — an empty server-side buffer reuses it (NETCODE.md §4),
//! and replay reproduces that for free because the frame is sim state.

/// Button bits (ALPHA.md §1 sizes sprint/crouch into the field; crouch has
/// no sim effect yet — it lands with the combat pass). PRIMARY is the
/// swing/use button: gather now (M1), attack with M2. A new bit in an
/// already-sized field — the wire layout does not move.
///
/// JUMP is the fourth, and it moves **no bit and no byte**: `buttons` is
/// written and read as a full unmasked octet (`protocol/lib.rs` —
/// `w.write(f.buttons as u32, 8)` / `r.read(8)`), so bit 3 has been crossing
/// intact since v0 and was merely ignored on arrival.
///
/// **It still turns `PROTO_VER` (21 ⇒ 22), and that is wall 6 working rather
/// than an exception to it.** The precedent is v18's, stated at
/// `protocol/lib.rs`: *a widened meaning is a wire change even when the
/// layout is byte-identical.* Here the consequence is worse than v18's
/// declined drag, because this bit feeds prediction. `movement::step` is
/// shared verbatim by the server and `client-wasm`'s predictor, so a v22
/// client against a v21 server would predict an arc the server never runs and
/// be hard-snapped back to the ground on every single press — a permanent,
/// silent misprediction, which is precisely the class NETCODE's
/// quantize-both-sides law and `PROTO_VER` exist to make impossible. The
/// handshake refusing the pairing outright is the correct outcome.
///
/// One coverage hole worth naming and NOT closing here: `goldens.rs` draws
/// `buttons` from `rng.next_bounded(4)`, so the protocol golden's fuzz has
/// only ever exercised bits 0–1 — `BTN_PRIMARY` is outside it too, and has
/// been since M1. Widening that draw would change fixture bytes for a reason
/// unrelated to this version's meaning, which muddies exactly the signal wall
/// 6 reads. It is filed in `NOW.md` as its own item.
pub const BTN_SPRINT: u8 = 1 << 0;
pub const BTN_CROUCH: u8 = 1 << 1;
pub const BTN_PRIMARY: u8 = 1 << 2;
pub const BTN_JUMP: u8 = 1 << 3;

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
    /// Selected hotbar slot 0..HOTBAR_SLOTS — the held item (client keys
    /// 1–6). 3 bits on the wire (decode refuses 6–7); `world::apply`
    /// falls invalid non-wire values back to slot 0.
    pub sel: u8,
}
