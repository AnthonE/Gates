//! Who owns the mouse pointer while a world is running.
//!
//! Here rather than inside `render::input` for this module's own stated
//! reason: the version of this that lived inside a Bevy system could only be
//! tested by a windowed run, and it was wrong for months in a way a windowed
//! run is exactly the worst instrument for — the defect is a *sequence*, not
//! a value, and it only shows up after you close something.
//!
//! ## The bug this exists to have fixed
//!
//! Reported by the operator, 2026-08-16, playing: *"when i was building and
//! used the window to pick pieces and close it my mouse was still up and i
//! couldnt look around right without clicking"*.
//!
//! `render::input::gather` released the pointer whenever a panel was up and
//! **never took it back**. The only path back to a locked pointer was the
//! "click to capture" arm, so after every inventory, every build wheel and
//! every hammer wheel the player had to click to look around again — and with
//! a building plan in hand that click *places a piece*. The remedy for a stuck
//! cursor was to build something.
//!
//! Two other owners in this client already got it right, which is what makes
//! this an omission rather than a design: `chat::close` re-locks with the
//! comment *"so getting out of chat does not need a click that would also
//! swing the axe"*, and the Esc menu takes the pointer back on resume. Panels
//! were the one release with no partner.
//!
//! ## Why it remembers instead of just re-locking
//!
//! An unconditional lock on close would capture a pointer the player had
//! never given. A fresh world is uncaptured until the first click — the
//! contract the settings screen advertises as *"click to capture the
//! pointer"* — so opening and closing a panel before ever clicking must leave
//! it uncaptured. [`Pointer`] records what to give back on the rising edge and
//! gives back exactly that, so both the common case and the contract hold
//! without either being a special case.

/// What the render layer should do to the cursor this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    /// Lock it and hide it — the playing state.
    Lock,
    /// Release it and show it — something on screen is being clicked.
    Release,
    /// Nobody is claiming it this frame; leave it as it is.
    Leave,
}

/// The pointer's memory across frames.
///
/// One value, stepped once per frame by [`Pointer::step`]. Deliberately not
/// four booleans in a system's `Local`s: the bug was that two of the four
/// transitions had no arm, and a type with one method makes the set of
/// transitions something a test can enumerate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointer {
    /// Whether a panel held the pointer on the previous frame.
    panel_had_it: bool,
    /// Whether the player had captured the pointer before that panel opened —
    /// the thing closing has to give back.
    locked_before_panel: bool,
    /// Whether the left button now down is the press that captured the
    /// pointer. See [`Pointer::swallow`].
    capturing: bool,
    /// Whether the left button now down began on a hand that does not
    /// swing. See [`Pointer::spent`].
    spending: bool,
}

impl Pointer {
    /// Advance one frame and say what the cursor should do.
    ///
    /// `locked` is the cursor's state as the window reports it, `panel_open`
    /// is whether anything on screen is claiming the pointer (a panel or the
    /// chat composer), and `clicked` is a left press this frame.
    ///
    /// The order matters and is the same order the system ran in: a click
    /// captures, then a panel's rising edge remembers, then the panel either
    /// holds the pointer or hands it back.
    pub fn step(&mut self, locked: bool, panel_open: bool, clicked: bool) -> Grab {
        // Click to capture — only when nothing on screen wants the click.
        let mut out = if clicked && !panel_open {
            Grab::Lock
        } else {
            Grab::Leave
        };
        // The rising edge: remember what the player had, including a capture
        // made by the very click above.
        if panel_open && !self.panel_had_it {
            self.locked_before_panel = locked || out == Grab::Lock;
        }
        if panel_open {
            out = Grab::Release;
        } else if self.panel_had_it && self.locked_before_panel {
            // The falling edge — the arm that did not exist.
            out = Grab::Lock;
        }
        self.panel_had_it = panel_open;
        out
    }

    /// Is the left button, this frame, the click that captured the pointer —
    /// or that click still held down? **That press is not also a use.** With
    /// the belt click-modal a capture click would otherwise eat a mushroom,
    /// put a sleeping bag down or place a wall wherever the ghost happened to
    /// stand, and it swung the axe long before that; a player clicking back
    /// into the window asked for none of it. Swallowed until the button is
    /// let go, so holding the capture click does not turn into a swing.
    ///
    /// `held` is the left button being down this frame; the other three are
    /// [`Pointer::step`]'s own inputs, read before it locks anything.
    pub fn swallow(&mut self, locked: bool, panel_open: bool, clicked: bool, held: bool) -> bool {
        if clicked && !locked && !panel_open {
            self.capturing = true;
        }
        if !held {
            self.capturing = false;
        }
        self.capturing
    }

    /// Is the left button, this frame, a press that began on a hand that
    /// does not swing (a lock or a box put down, a piece placed, a meal, a
    /// read), or that press still held? **It is not also a swing.** Putting
    /// down the last lock empties the slot under a button still held, and
    /// an empty hand punches, so the press that put a lock on a box punched
    /// the box (operator, 2026-10-17: *"placing a lock makes me swing"*).
    /// Swallowed until the button is let go.
    ///
    /// `swings` is what the hand in use does on a left click this frame
    /// (`ui::hold::Click::swings`).
    pub fn spent(&mut self, clicked: bool, held: bool, swings: bool) -> bool {
        if clicked && !swings {
            self.spending = true;
        }
        if !held {
            self.spending = false;
        }
        self.spending
    }
}

/// Frames a fresh lock request may stay ungranted before it counts as
/// refused. A browser grants within a frame or two; a refusal (no user
/// gesture behind it, or asked again too soon after the player pressed Esc)
/// never arrives as anything Bevy can see.
pub const LOCK_GRACE_FRAMES: u32 = 30;
/// Frames after a request before a granted lock counts as settled, so the
/// tail of the lock just let go (a panel opened and closed inside a frame or
/// two) is not mistaken for the player's Esc.
pub const LOCK_SETTLE_FRAMES: u32 = 4;

/// What [`BrowserLock::step`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockFix {
    /// The window's grab mode and the page agree, or may still come to.
    Keep,
    /// The game let go but the page still holds the lock: ask again.
    Unlock,
    /// The lock asked for never came. Let the cursor go, so it is not an
    /// invisible arrow over a view that does not turn, and so the next click
    /// is a capture (and swallowed as one).
    Refused,
    /// The page held the lock and the player took it back: Esc, or the tab
    /// losing focus. The browser ate that Esc, so this is the game's Esc.
    Taken,
}

/// The browser's half of the pointer.
///
/// Bevy never hears that a page let go of the pointer: winit asks for the
/// lock and never listens for `pointerlockchange`, so after Esc the window
/// still says `Locked` and hides the cursor, no menu opens, and the click
/// that recaptures is not swallowed (the window thought it was locked all
/// along) and swings. Stepped once a frame with what the window wants and
/// what `document.pointerLockElement` says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrowserLock {
    /// Frames the window has wanted the lock without a break.
    wanting: u32,
    /// Whether the page has held a settled lock since the window asked.
    held: bool,
}

impl BrowserLock {
    pub fn step(&mut self, want: bool, has: bool) -> LockFix {
        if !want {
            self.wanting = 0;
            self.held = false;
            return if has { LockFix::Unlock } else { LockFix::Keep };
        }
        self.wanting = self.wanting.saturating_add(1);
        if has {
            if self.wanting > LOCK_SETTLE_FRAMES {
                self.held = true;
            }
            return LockFix::Keep;
        }
        if self.held {
            LockFix::Taken
        } else if self.wanting > LOCK_GRACE_FRAMES {
            LockFix::Refused
        } else {
            LockFix::Keep
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Esc in a browser: the lock was held, then the page let go while the
    /// window still wants it. That is the player's Esc, once.
    #[test]
    fn a_browser_esc_is_seen() {
        let mut b = BrowserLock::default();
        for _ in 0..2 {
            assert_eq!(b.step(true, false), LockFix::Keep, "the request is pending");
        }
        for _ in 0..10 {
            assert_eq!(b.step(true, true), LockFix::Keep);
        }
        assert_eq!(b.step(true, false), LockFix::Taken);
        // The game answers by letting go; nothing more is owed.
        assert_eq!(b.step(false, false), LockFix::Keep);
    }

    /// A request that is never granted gives the cursor back after the grace,
    /// and is not taken for an Esc.
    #[test]
    fn a_refused_lock_frees_the_cursor() {
        let mut b = BrowserLock::default();
        for _ in 0..LOCK_GRACE_FRAMES {
            assert_eq!(b.step(true, false), LockFix::Keep);
        }
        assert_eq!(b.step(true, false), LockFix::Refused);
    }

    /// The old lock still draining when the new request goes out (a panel
    /// shut a frame after it opened) is not an Esc.
    #[test]
    fn the_tail_of_the_last_lock_is_not_an_esc() {
        let mut b = BrowserLock::default();
        assert_eq!(
            b.step(true, true),
            LockFix::Keep,
            "the old lock, still held"
        );
        assert_eq!(b.step(true, false), LockFix::Keep, "its release lands");
        assert_eq!(b.step(true, true), LockFix::Keep, "the new one is granted");
    }

    /// The game let go and the page still holds it: ask again until it does.
    #[test]
    fn a_lock_the_game_let_go_is_let_go() {
        let mut b = BrowserLock::default();
        assert_eq!(b.step(false, true), LockFix::Unlock);
        assert_eq!(b.step(false, false), LockFix::Keep);
    }

    /// The press that put down the last lock empties the slot under it, and
    /// the empty hand it leaves must not punch until the button comes up.
    #[test]
    fn a_press_that_placed_something_does_not_punch() {
        let mut p = Pointer::default();
        assert!(p.spent(true, true, false), "the lock goes down");
        assert!(p.spent(false, true, true), "the empty hand, still held");
        assert!(!p.spent(false, false, true), "let go");
        assert!(!p.spent(true, true, true), "the next press punches");
        assert!(!p.spent(false, true, true), "and keeps punching");
    }

    /// **The operator's own report, as a gate.** Lock the pointer, open the
    /// build wheel, pick a piece, close it — and be able to look around
    /// without clicking.
    #[test]
    fn closing_a_panel_gives_the_pointer_back() {
        let mut p = Pointer::default();
        assert_eq!(p.step(false, false, true), Grab::Lock, "click captures");
        // The wheel is up for a few frames while a wedge is chosen.
        for _ in 0..3 {
            assert_eq!(p.step(true, true, false), Grab::Release);
        }
        assert_eq!(
            p.step(false, false, false),
            Grab::Lock,
            "letting go of the wheel must hand the pointer back, or looking \
             around needs a click that also places a piece"
        );
    }

    /// The contract the settings screen advertises: a world nobody has
    /// clicked in yet stays uncaptured, panels or no panels.
    #[test]
    fn a_panel_cannot_capture_a_pointer_the_player_never_gave() {
        let mut p = Pointer::default();
        assert_eq!(p.step(false, true, false), Grab::Release);
        assert_eq!(
            p.step(false, false, false),
            Grab::Leave,
            "closing a panel captured a pointer the player never clicked for"
        );
    }

    /// The steady states claim nothing, so no other owner is fought with —
    /// the keypad draws over the world and deliberately grabs nothing.
    #[test]
    fn a_quiet_frame_leaves_the_cursor_alone() {
        let mut p = Pointer::default();
        assert_eq!(p.step(true, false, false), Grab::Leave);
        assert_eq!(p.step(true, false, false), Grab::Leave);
    }

    /// A click that lands on the same frame a panel opens belongs to the
    /// panel, not to the world — and the capture it would have made is still
    /// what gets handed back on close.
    #[test]
    fn a_click_into_an_opening_panel_does_not_capture() {
        let mut p = Pointer::default();
        assert_eq!(p.step(false, true, true), Grab::Release);
        assert_eq!(
            p.step(false, false, false),
            Grab::Leave,
            "a click the panel ate must not count as the player capturing"
        );
    }

    /// The click that captures is swallowed for as long as it is held, and
    /// the next click — the pointer locked by then — is a click again.
    #[test]
    fn the_capturing_click_is_not_a_use() {
        let mut p = Pointer::default();
        assert!(p.swallow(false, false, true, true), "the capture click");
        assert!(p.swallow(true, false, false, true), "still held after lock");
        assert!(!p.swallow(true, false, false, false), "released");
        assert!(!p.swallow(true, false, true, true), "a click once locked");
        assert!(
            !p.swallow(false, true, true, true),
            "a click into a panel belongs to the panel, not to this"
        );
    }

    /// Re-opening does not lose the memory: three panels in a row, closed
    /// once, still ends locked.
    #[test]
    fn the_memory_survives_a_second_panel() {
        let mut p = Pointer::default();
        p.step(false, false, true);
        p.step(true, true, false);
        assert_eq!(p.step(false, false, false), Grab::Lock);
        p.step(true, true, false);
        assert_eq!(p.step(false, false, false), Grab::Lock);
    }
}
