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
/// shared verbatim by the server and `client-core`'s predictor, so a v22
/// client against a v21 server would predict an arc the server never runs and
/// be hard-snapped back to the ground on every single press — a permanent,
/// silent misprediction, which is precisely the class NETCODE's
/// quantize-both-sides law and `PROTO_VER` exist to make impossible. The
/// handshake refusing the pairing outright is the correct outcome.
///
/// The coverage hole this doc used to name is **closed** (2026-08-18, NOW.md
/// §5c): `goldens.rs` drew `buttons` from `rng.next_bounded(4)`, so from v0 to
/// v46 the protocol golden's fuzz exercised bits 0–1 only — `BTN_PRIMARY` was
/// outside it too, since M1. It now draws the whole octet, and
/// `the_input_golden_fuzzes_the_whole_button_octet` reads the fixture bytes
/// back so the coverage cannot narrow again unnoticed. It changed fixture
/// bytes and turned **no `PROTO_VER`**, on the rule written there: what a test
/// feeds an encoder is the test's coverage, not the wire's meaning.
pub const BTN_SPRINT: u8 = 1 << 0;
pub const BTN_CROUCH: u8 = 1 << 1;
pub const BTN_PRIMARY: u8 = 1 << 2;
pub const BTN_JUMP: u8 = 1 << 3;
/// **Hold the light in your hand up, lit** (torch fuel v0, `light.rs`).
///
/// A *latch*, not an edge: the client keeps the bit set for as long as the
/// flame should burn and clears it to put the flame out, exactly as
/// [`InputFrame::sel`] is a latch for which slot is in the hand rather
/// than a "switch slots" verb. That is what makes this a button bit
/// instead of an `ActionMsg`, and the reason is prediction rather than
/// wire economy: both sides have to agree, every tick, on whether a
/// torch is burning, and a one-shot toggle would leave the two ends
/// holding separate copies of a flag that a single dropped datagram could
/// invert (`EV_OVEN`'s own argument, one layer down).
///
/// The sim never stores it. Whether a flame is actually burning is
/// `light::is_lit` — this bit **and** a held stack whose content row
/// declares a `light_burn` **and** condition left to spend — so the
/// authority stays with the server while the *intent* stays with the
/// hand that pressed the key, and the client can compute the same
/// predicate from facts it already mirrors (its own latch, its own
/// `HELD_MODELS` row, and the `cond` `SUB_INV` gives it).
pub const BTN_LIGHT: u8 = 1 << 4;

/// Every button bit the sim means — the closed set of the five above.
///
/// The wire carries `buttons` as a full unmasked octet (see the JUMP note),
/// so bits 5–7 cross intact and mean nothing: no verb reads them, but
/// `state_hash` hashes the stored frame, so an unmasked garbage bit would be
/// client-writable state that no rule owns (NOW.md §5b's forgery slack).
/// The server refuses a wire frame carrying one (`net.rs` `accept_input`);
/// `world::apply` masks non-wire frames instead — `sel`'s fallback rule.
/// A new button joins this mask in the same commit that declares its bit,
/// or every press of it is refused at the door;
/// `tests/domain_ledger.rs` fails if the two drift apart.
pub const BTN_MASK: u8 = BTN_SPRINT | BTN_CROUCH | BTN_PRIMARY | BTN_JUMP | BTN_LIGHT;

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
