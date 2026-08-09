//! The code lock's keypad, as arithmetic — everything about it that a test
//! can hold (lock v1, `reference/DOORS.md`).
//!
//! `RENDER.md` §1's rule applied to the newest surface: the buffer, the
//! digit rules and the key→op mapping live here, pure and gated by
//! `tests/ui.rs`; `render/verbs.rs` reads keys and hands bytes to the
//! session, and `render/hud.rs` draws the line this produces.
//!
//! ## Why the keypad is an overlay and not a `Panel`
//!
//! Every other menu in this client grabs the pointer, and a keypad must
//! not: the door is in front of you, the person knocking on the other side
//! of it is not waiting, and a screen that takes your mouse to type four
//! digits is a screen you die in. The reference's own keypad is a small
//! overlay for the same reason. So this is a small on-screen panel
//! (`render/hud.rs::pad_overlay`) that owns nothing — the digits are typed
//! rather than clicked, and the panel is presentation only. It began as a
//! bare HUD line; [`Keypad::line`] is that line, kept as the one-string
//! rendering the overlay's pieces ([`Keypad::cells`], [`OPS_HINT`]) compose
//! into, so the two surfaces cannot drift apart.
//!
//! ## What it deliberately does not know
//!
//! Whether the code is right, whether you may set it, and whether the lock
//! is shut. All three are the sim's verdicts and all three come back as
//! events (`EV_AUTH`, `REFUSE_D_*`). A keypad that greyed out an op it
//! predicted would be refused would be a client deciding access, which is
//! the one thing `RENDER.md` §1 forbids most plainly.

use sim_core::deploy::{
    ACCESS_OP_ENTER, ACCESS_OP_LOCK, ACCESS_OP_SET_CODE, ACCESS_OP_SET_GUEST, ACCESS_OP_TAKE,
    ACCESS_OP_UNLOCK,
};

/// Digits in a code. The wire's own space (0000..=9999).
pub const DIGITS: usize = 4;

/// The keypad, aimed at one lock — a door's or a box's, one flow for both
/// (locks on boxes, `DOORS.md` §9.8). `None` in `at` means closed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keypad {
    /// The address of the lock this keypad speaks to, if it is open at
    /// all: a door's edge address, or a box's plane address.
    pub at: Option<(u16, u16, u8, u8)>,
    digits: [u8; DIGITS],
    len: usize,
}

impl Keypad {
    /// Open the keypad on a lock, clearing whatever was typed at the last
    /// one. Retyping is the right default: a code left in the buffer would
    /// be entered at a *different* lock by the next press of `Enter`, and
    /// a wrong code costs hp.
    pub fn open(&mut self, cx: u16, cz: u16, level: u8, loc: u8) {
        self.at = Some((cx, cz, level, loc));
        self.clear();
    }

    pub fn close(&mut self) {
        self.at = None;
        self.clear();
    }

    pub fn is_open(&self) -> bool {
        self.at.is_some()
    }

    fn clear(&mut self) {
        self.digits = [0; DIGITS];
        self.len = 0;
    }

    /// Type a digit. Past [`DIGITS`] it is **dropped**, not shifted: a
    /// buffer that scrolled would let a fifth keystroke silently change
    /// the code a player is looking at.
    pub fn push(&mut self, d: u8) {
        if self.len < DIGITS && d < 10 {
            self.digits[self.len] = d;
            self.len += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.len > 0 {
            self.len -= 1;
            self.digits[self.len] = 0;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The typed code, or `None` until all four digits are in. Ops that
    /// need a code refuse to send without one — a partial code sent as a
    /// number would be a *different* code (`12` is not `0012` to a player
    /// who typed two digits and meant to type four).
    pub fn code(&self) -> Option<u16> {
        if self.len < DIGITS {
            return None;
        }
        let mut v = 0u16;
        for d in self.digits {
            v = v * 10 + d as u16;
        }
        Some(v)
    }

    /// What each of the four digit cells shows: the typed digit, or `None`
    /// for a cell not reached yet. The overlay draws these as cells and
    /// [`Self::line`] draws them as characters — one source for both.
    pub fn cells(&self) -> [Option<u8>; DIGITS] {
        std::array::from_fn(|i| (i < self.len).then(|| self.digits[i]))
    }

    /// The one-line rendering: what is typed, dotted out to four, and the
    /// ops. Composed from [`Self::cells`] and [`OPS_HINT`] so it cannot say
    /// something the overlay does not.
    pub fn line(&self) -> String {
        let mut code = String::with_capacity(DIGITS);
        for c in self.cells() {
            code.push(match c {
                Some(d) => (b'0' + d) as char,
                None => '_',
            });
        }
        format!("CODE {code}  ·  {OPS_HINT}")
    }
}

/// The ops the surface offers, key by key, in the order drawn — one string
/// shared by [`Keypad::line`] and the overlay, so what is offered cannot be
/// said two ways. The letters are [`op_for`]'s; the reachability test below
/// holds the two lists together.
pub const OPS_HINT: &str =
    "[ENTER] TRY  [S] SET  [G] GUEST  [K] LOCK  [U] UNLOCK  [T] TAKE  [ESC] CLOSE";

/// What the `L` press addresses, resolved from the pick.
///
/// **The set of things a lock bolts to is the sim's, by import.** The pick
/// carries the archetype the deploy sync named ([`Pick::arch`]), and this
/// asks `sim_core::deploy::lockable` — the exact predicate the sim's lock
/// verb gates on one crate down — instead of matching verbs. A third
/// lockable archetype reaches the keypad the day the sim grows one, not a
/// release later; that is the same no-second-list rule `interact` holds
/// for reach (locks on boxes, `DOORS.md` §9.8).
///
/// [`Pick::arch`]: crate::ui::interact::Pick::arch
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockTarget {
    /// Open the keypad at this address — a door's edge, a box's plane;
    /// the six ops speak from there and the sim answers refusals.
    Pad(u16, u16, u8, u8),
    /// Lockable, but nothing bolted on: prompt to deploy a lock rather
    /// than opening an empty pad.
    Bare,
    /// Nothing a lock bolts to.
    None,
}

/// Resolve what `L` does with a pick. Pure, and gated in `tests/ui.rs` §J
/// — including the agreement sweep against `deploy::lockable` itself.
pub fn lock_target(pick: &crate::ui::interact::Pick) -> LockTarget {
    if !sim_core::deploy::lockable(pick.arch) {
        return LockTarget::None;
    }
    if !pick.has_lock {
        return LockTarget::Bare;
    }
    LockTarget::Pad(pick.cx, pick.cz, pick.level, pick.loc)
}

/// What a keypad op needs from the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// Send the op with the typed code; refuse to send without four
    /// digits.
    Code,
    /// Send the op with no code — the buffer is irrelevant.
    Nothing,
}

/// The op a key means, and whether it needs the buffer filled. `None` for
/// a key the keypad does not claim, which is how every other binding in
/// the client keeps working while it is open.
///
/// The letters are chosen against the reference's own verb list
/// (`DOORS.md` §7): **S**et, **G**uest, loc**K**, **U**nlock, **T**ake.
/// `Enter` is the one a player will actually use, so it is the one key
/// they do not have to learn.
pub fn op_for(key: KeypadKey) -> Option<(u8, Needs)> {
    Some(match key {
        KeypadKey::Try => (ACCESS_OP_ENTER, Needs::Code),
        KeypadKey::Set => (ACCESS_OP_SET_CODE, Needs::Code),
        KeypadKey::Guest => (ACCESS_OP_SET_GUEST, Needs::Code),
        KeypadKey::Lock => (ACCESS_OP_LOCK, Needs::Nothing),
        KeypadKey::Unlock => (ACCESS_OP_UNLOCK, Needs::Nothing),
        KeypadKey::Take => (ACCESS_OP_TAKE, Needs::Nothing),
    })
}

/// The six keys that send. A tiny enum rather than `KeyCode` so this
/// module never learns what a keyboard is — the same reason `Pick` does
/// not know what a mouse is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeypadKey {
    Try,
    Set,
    Guest,
    Lock,
    Unlock,
    Take,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_digits_make_a_code_and_three_do_not() {
        let mut k = Keypad::default();
        k.open(3, 4, 0, 1);
        assert_eq!(k.code(), None, "an empty buffer is not the code 0");
        for d in [1u8, 2, 3] {
            k.push(d);
        }
        assert_eq!(
            k.code(),
            None,
            "123 is not 0123 — a partial code must not send"
        );
        k.push(4);
        assert_eq!(k.code(), Some(1234));
    }

    #[test]
    fn a_leading_zero_survives() {
        let mut k = Keypad::default();
        for d in [0u8, 0, 4, 2] {
            k.push(d);
        }
        assert_eq!(k.code(), Some(42), "0042 is 42 on the wire");
        assert!(k.line().starts_with("CODE 0042"), "{}", k.line());
    }

    #[test]
    fn a_fifth_digit_is_dropped_not_shifted() {
        let mut k = Keypad::default();
        for d in [1u8, 2, 3, 4, 9] {
            k.push(d);
        }
        assert_eq!(
            k.code(),
            Some(1234),
            "a scrolling buffer would change the code under the player's eyes"
        );
        k.backspace();
        assert_eq!(k.code(), None);
        k.push(9);
        assert_eq!(k.code(), Some(1239));
    }

    #[test]
    fn opening_at_a_new_door_clears_the_buffer() {
        let mut k = Keypad::default();
        k.open(1, 1, 0, 1);
        for d in [1u8, 2, 3, 4] {
            k.push(d);
        }
        k.open(2, 2, 0, 1);
        assert_eq!(k.at, Some((2, 2, 0, 1)));
        assert_eq!(
            k.code(),
            None,
            "a code left over from another lock would be entered at this one, \
             and a wrong code costs hp"
        );
    }

    #[test]
    fn the_line_dots_out_what_is_not_typed() {
        let mut k = Keypad::default();
        assert!(k.line().starts_with("CODE ____"), "{}", k.line());
        k.push(7);
        assert!(k.line().starts_with("CODE 7___"), "{}", k.line());
        assert!(k.line().contains("[ENTER] TRY"), "the ops are named");
        assert!(k.line().contains("[T] TAKE"));
    }

    /// The overlay's cells and the line are one rendering: cell `i` is the
    /// typed digit exactly up to `len`, and the line is the cells plus the
    /// shared ops string — composition, not agreement by luck.
    #[test]
    fn the_cells_are_the_line_in_pieces() {
        let mut k = Keypad::default();
        assert_eq!(k.cells(), [None; DIGITS]);
        for d in [0u8, 4] {
            k.push(d);
        }
        assert_eq!(k.cells(), [Some(0), Some(4), None, None]);
        assert!(k.line().starts_with("CODE 04__"), "{}", k.line());
        assert!(k.line().ends_with(OPS_HINT), "{}", k.line());
    }

    /// Every key the pad answers is offered on the hint, and the hint
    /// offers nothing the pad does not answer — the surface must not
    /// advertise a dead key or hide a live one.
    #[test]
    fn the_hint_names_every_key_and_only_those() {
        for tag in [
            "[ENTER] TRY",
            "[S] SET",
            "[G] GUEST",
            "[K] LOCK",
            "[U] UNLOCK",
            "[T] TAKE",
            "[ESC] CLOSE",
        ] {
            assert!(OPS_HINT.contains(tag), "{tag} missing from the hint");
        }
        // Seven brackets: the six ops plus close, and nothing else.
        assert_eq!(OPS_HINT.matches('[').count(), 7);
    }

    #[test]
    fn every_key_maps_to_a_distinct_op_and_the_three_that_need_a_code_say_so() {
        use KeypadKey::*;
        let all = [Try, Set, Guest, Lock, Unlock, Take];
        let mut ops: Vec<u8> = all.iter().filter_map(|k| op_for(*k)).map(|o| o.0).collect();
        assert_eq!(ops.len(), all.len(), "every key sends something");
        ops.sort_unstable();
        ops.dedup();
        assert_eq!(ops.len(), all.len(), "two keys share an op");
        assert_eq!(op_for(Try).unwrap().1, Needs::Code);
        assert_eq!(op_for(Set).unwrap().1, Needs::Code);
        assert_eq!(op_for(Guest).unwrap().1, Needs::Code);
        assert_eq!(op_for(Lock).unwrap().1, Needs::Nothing);
        assert_eq!(op_for(Unlock).unwrap().1, Needs::Nothing);
        assert_eq!(op_for(Take).unwrap().1, Needs::Nothing);
    }

    /// The keypad reaches every **lock** op the sim has. A door verb added
    /// to `lock.rs` with no key here is a mechanic no player can reach.
    ///
    /// Scoped to the lock half deliberately: since hearth crew v1 the op
    /// space also carries the three crew ops, and those are the *hearth's*
    /// surface (`E` at a hearth, `render/verbs.rs`) rather than the
    /// keypad's — a keypad offering "join crew" at a door would be
    /// offering a verb that store cannot answer. `op_is_crew` is the same
    /// split the sim dispatches on, so this cannot drift from it.
    #[test]
    fn the_keypad_reaches_every_lock_op_the_sim_has() {
        use KeypadKey::*;
        let reachable: Vec<u8> = [Try, Set, Guest, Lock, Unlock, Take]
            .iter()
            .filter_map(|k| op_for(*k))
            .map(|o| o.0)
            .collect();
        for op in 0..=sim_core::deploy::ACCESS_OP_MAX {
            if sim_core::deploy::op_is_crew(op) {
                assert!(
                    !reachable.contains(&op),
                    "crew op {op} is on the keypad, and a door cannot answer it"
                );
                continue;
            }
            assert!(
                reachable.contains(&op),
                "lock op {op} has no key — the sim can do it and no player can ask"
            );
        }
    }
}
