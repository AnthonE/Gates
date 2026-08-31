//! The input frame (DESIGN.md §5.4): seq, buttons, yaw u16, pitch u8,
//! move vec 2×i8. The wire adds client_tick and redundancy; the sim
//! consumes exactly one effective frame per player per tick (two on a
//! throttle catch-up, `world.rs Command::InputPair`) and keeps the last
//! one applied — an empty server-side buffer reuses it, and replay
//! reproduces that for free because the frame is sim state. Since netcode
//! v2 (NETCODE.md §4) the SERVER does not leave the reuse at full
//! strength: it mints [`decay_frame`]'s ramp as ordinary commands, so the
//! sim rule stays one sentence and the WAL still carries everything.

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

/// How many starved reuses fully extinguish a frame's movement — the
/// length of [`decay_frame`]'s ramp. Server-side minting stops once a
/// frame is this stale (the stored frame is then already fully decayed,
/// so the sim's implicit reuse of it is identical to further mints), and
/// the wire's `repeat_count` saturation only has to say "at least this".
pub const DECAY_STEPS: u32 = 3;

/// The starved-reuse frame (netcode v2, DECISIONS.md 2026-08-31): what the
/// server feeds the sim on a tick that has no fresh frame from this player,
/// in place of re-running the last one verbatim at full strength forever.
///
/// `repeat` is how many consecutive ticks the last real frame has already
/// covered (1 on the first starved tick). Movement fades 3/3 → 2/3 → 1/3 →
/// 0 across [`DECAY_STEPS`] reuses — Rocket League's published ramp, and
/// the direction matters more than the numbers: **undershooting and
/// catching up when the real input lands reads better than overshooting
/// and rubber-banding back** (Cone, GDC 2018). The overshoot is exactly
/// what a player felt as "my client snaps when I stop": the release frame
/// is late, the server walks the body on at full speed, and the reconcile
/// drags the client to a place it never went.
///
/// What decays and what rides through is decided by kind, not by list:
/// - **Axes decay.** `move_x`/`move_z` are the only intent that
///   extrapolates badly.
/// - **Edges and holds clear from the first reuse.** A reused `BTN_JUMP`
///   is a hop on every landing (`movement.rs` — grounded re-arms the
///   button), a reused `BTN_PRIMARY` swings an arm nobody is driving.
/// - **Latches ride through untouched**: `yaw`, `pitch`, `sel`, and
///   `BTN_LIGHT` — the frame's statements about what *is* rather than
///   what to *do*. A two-tick starve that snuffed a torch for everyone
///   watching would be a new defect wearing a fix's clothes.
///
/// Integer math only, widened to i32 before the scale so ±127 cannot
/// overflow and division truncates toward zero identically on native and
/// wasm32 — an arithmetic shift would decay left-strafe harder than right
/// (`-127 >> 1` is -64 against `127 >> 1`'s 63), which is why there isn't
/// one. `repeat == 0` returns the frame bit-identical, so a caller may
/// use this total function unconditionally.
pub fn decay_frame(f: &InputFrame, repeat: u32) -> InputFrame {
    let keep = (DECAY_STEPS.saturating_sub(repeat)) as i32;
    let scale = |v: i8| ((v as i32) * keep / DECAY_STEPS as i32) as i8;
    InputFrame {
        seq: f.seq,
        buttons: if repeat == 0 {
            f.buttons
        } else {
            f.buttons & BTN_LIGHT
        },
        yaw: f.yaw,
        pitch: f.pitch,
        move_x: scale(f.move_x),
        move_z: scale(f.move_z),
        sel: f.sel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(move_x: i8, move_z: i8, buttons: u8) -> InputFrame {
        InputFrame {
            seq: 9,
            buttons,
            yaw: 0x1234,
            pitch: 77,
            move_x,
            move_z,
            sel: 3,
        }
    }

    /// repeat 0 is the frame itself, bit for bit — the total-function
    /// contract that lets a caller apply the decay unconditionally.
    #[test]
    fn repeat_zero_is_identity() {
        let f = frame(-127, 127, BTN_MASK);
        assert_eq!(decay_frame(&f, 0), f);
    }

    /// The ramp is 2/3, 1/3, 0 — and symmetric in sign, which is the
    /// reason the scale is a widened multiply-divide and not a shift.
    #[test]
    fn movement_decays_symmetrically_to_zero() {
        let f = frame(-127, 127, 0);
        let d1 = decay_frame(&f, 1);
        assert_eq!((d1.move_x, d1.move_z), (-84, 84));
        let d2 = decay_frame(&f, 2);
        assert_eq!((d2.move_x, d2.move_z), (-42, 42));
        for repeat in DECAY_STEPS.. {
            let d = decay_frame(&f, repeat);
            assert_eq!((d.move_x, d.move_z), (0, 0));
            if repeat > DECAY_STEPS + 2 {
                break;
            }
        }
    }

    /// Edges and holds clear on the first reuse — the reused-jump hop and
    /// the driverless swing — while the flame latch rides through, so a
    /// short starve cannot snuff a torch for everyone watching.
    #[test]
    fn buttons_clear_except_the_light_latch() {
        let f = frame(50, 50, BTN_MASK);
        let d = decay_frame(&f, 1);
        assert_eq!(d.buttons, BTN_LIGHT);
        let dark = frame(50, 50, BTN_SPRINT | BTN_JUMP | BTN_PRIMARY);
        assert_eq!(decay_frame(&dark, 1).buttons, 0);
    }

    /// The facts of the frame — look, hand, seq — never decay.
    #[test]
    fn latches_ride_through_every_step() {
        let f = frame(10, -10, BTN_LIGHT);
        for repeat in 0..6 {
            let d = decay_frame(&f, repeat);
            assert_eq!((d.seq, d.yaw, d.pitch, d.sel), (9, 0x1234, 77, 3));
        }
    }
}
