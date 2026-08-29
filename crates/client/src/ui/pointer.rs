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
}

#[cfg(test)]
mod tests {
    use super::*;

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
