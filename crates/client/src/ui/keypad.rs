//! The code lock's keypad, as arithmetic — everything about it that a test
//! can hold (lock v1, `reference/DOORS.md`).
//!
//! `RENDER.md` §1's rule applied to the newest surface: the buffer, the
//! digit rules and the key→op mapping live here, pure and gated by
//! `tests/ui.rs`; `render/verbs.rs` reads keys and hands bytes to the
//! session, and `render/hud.rs` draws the line this produces.
//!
//! ## Why the keypad is a HUD line and not a panel
//!
//! Every other menu in this client grabs the pointer, and a keypad must
//! not: the door is in front of you, the person knocking on the other side
//! of it is not waiting, and a screen that takes your mouse to type four
//! digits is a screen you die in. The reference's own keypad is a small
//! overlay for the same reason. So this is a line under the prompt and a
//! handful of keys, and the cost of that choice is that the digits are
//! typed rather than clicked.
//!
//! ## What it deliberately does not know
//!
//! Whether the code is right, whether you may set it, and whether the lock
//! is shut. All three are the sim's verdicts and all three come back as
//! events (`EV_AUTH`, `REFUSE_D_*`). A keypad that greyed out an op it
//! predicted would be refused would be a client deciding access, which is
//! the one thing `RENDER.md` §1 forbids most plainly.

use sim_core::lock::{
    LOCK_OP_ENTER, LOCK_OP_LOCK, LOCK_OP_SET_CODE, LOCK_OP_SET_GUEST, LOCK_OP_TAKE, LOCK_OP_UNLOCK,
};

/// Digits in a code. The wire's own space (0000..=9999).
pub const DIGITS: usize = 4;

/// The keypad, aimed at one door. `None` in `at` means closed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keypad {
    /// The door this keypad is bolted to, if it is open at all.
    pub at: Option<(u16, u16, u8, u8)>,
    digits: [u8; DIGITS],
    len: usize,
}

impl Keypad {
    /// Open the keypad on a door, clearing whatever was typed at the last
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

    /// The HUD line: what is typed, dotted out to four, and the ops.
    pub fn line(&self) -> String {
        let mut code = String::with_capacity(DIGITS);
        for i in 0..DIGITS {
            if i < self.len {
                code.push((b'0' + self.digits[i]) as char);
            } else {
                code.push('_');
            }
        }
        format!(
            "CODE {code}  ·  [ENTER] TRY  [S] SET  [G] GUEST  [K] LOCK  [U] UNLOCK  [T] TAKE  [ESC] CLOSE"
        )
    }
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
        KeypadKey::Try => (LOCK_OP_ENTER, Needs::Code),
        KeypadKey::Set => (LOCK_OP_SET_CODE, Needs::Code),
        KeypadKey::Guest => (LOCK_OP_SET_GUEST, Needs::Code),
        KeypadKey::Lock => (LOCK_OP_LOCK, Needs::Nothing),
        KeypadKey::Unlock => (LOCK_OP_UNLOCK, Needs::Nothing),
        KeypadKey::Take => (LOCK_OP_TAKE, Needs::Nothing),
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

    /// The keypad's op set is the sim's op set. A verb added to `lock.rs`
    /// with no key here is a mechanic no player can reach.
    #[test]
    fn the_keypad_reaches_every_op_the_sim_has() {
        use KeypadKey::*;
        let reachable: Vec<u8> = [Try, Set, Guest, Lock, Unlock, Take]
            .iter()
            .filter_map(|k| op_for(*k))
            .map(|o| o.0)
            .collect();
        for op in 0..=sim_core::lock::LOCK_OP_MAX {
            assert!(
                reachable.contains(&op),
                "lock op {op} has no key — the sim can do it and no player can ask"
            );
        }
    }
}
