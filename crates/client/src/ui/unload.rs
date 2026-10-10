//! `R`, held, empties the magazine back into the pack (`Command::Unload`,
//! `NOW.md` §0mag 2) — **a hold on `R` with nothing to reload**.
//!
//! `R` stays the reload key and still answers on the down edge, so a reload
//! costs no latency. The hold is what a press that loaded nothing turns
//! into: a full magazine, or a pack with no round to load. Which of those
//! a press was is the server's answer, so the hold watches for it rather
//! than guessing — [`ClientCore::fills`] moves when a fill lands, and a
//! fill spends the hold (there was something to reload; the player is now
//! inside its beat, and an unload would only come back `still busy`).
//!
//! One case is known before the answer: the readout says the magazine is
//! full. Then the press sends nothing, because the reload could only be
//! refused, and the key decides on release — a tap still sends the reload,
//! so the "the magazine is full" sentence is still the key's answer, and a
//! hold unloads. Deciding off the readout is safe here and only here: a
//! stale readout costs a tap's worth of delay, never a swallowed press
//! (`render::verbs`' comment at the `R` binding is why that matters).
//!
//! [`ClientCore::fills`]: client_core::core::ClientCore::fills

/// Seconds `R` must be held for an unload. `interact::GIVE_HOLD_S`'s
/// length, for its reason: long enough that a reload tap never becomes one.
pub const UNLOAD_HOLD_S: f64 = 0.5;

/// Seconds into a hold before the HUD names it. A reload's answer lands
/// inside this on any playable link, so a press that loads never flashes
/// "unloading" first, and a tap never shows it at all.
pub const UNLOAD_SHOW_S: f64 = 0.2;

/// What the reload key sends this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Reload,
    Unload,
}

/// One frame of the reload key and what the client knows of the hand.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// The key's down edge.
    pub pressed: bool,
    /// Whether it is held.
    pub down: bool,
    /// The item in hand (`ClientCore::held_item`).
    pub item: u16,
    /// The readout (`ClientCore::mag`): loaded, ceiling.
    pub mag: (u16, u16),
    /// `ClientCore::fills`.
    pub fills: u32,
    pub now: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Armed {
    t0: f64,
    item: u16,
    fills: u32,
    /// The press sent nothing (the readout said full): release sends the
    /// reload a tap owes.
    deferred: bool,
}

/// The hold on `R`. [`UnloadHold::step`] once a frame while the key is the
/// reload key; [`UnloadHold::cancel`] whenever something else takes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnloadHold {
    on: Option<Armed>,
}

impl UnloadHold {
    /// One frame. Returns what to send, if anything.
    pub fn step(&mut self, f: Frame) -> Option<Ask> {
        if f.pressed {
            let (loaded, ceiling) = f.mag;
            let full = ceiling > 0 && loaded >= ceiling;
            self.on = Some(Armed {
                t0: f.now,
                item: f.item,
                fills: f.fills,
                deferred: full,
            });
            return (!full).then_some(Ask::Reload);
        }
        let a = self.on?;
        if !f.down {
            self.on = None;
            return a.deferred.then_some(Ask::Reload);
        }
        // The hand changed, or the press loaded: either way there is no
        // unload to give this hold.
        if f.item != a.item || f.fills != a.fills {
            self.on = None;
            return None;
        }
        if f.now - a.t0 >= UNLOAD_HOLD_S {
            self.on = None;
            // Nothing loaded (or nothing stated for this hand) is nothing
            // to unload: the reload's own refusal has already said why.
            return (f.mag.0 > 0).then_some(Ask::Unload);
        }
        None
    }

    /// Whether the HUD should name the hold this frame: under way past
    /// [`UNLOAD_SHOW_S`], on the hand it began on, with nothing filled
    /// since, and rounds to empty.
    pub fn showing(&self, f: Frame) -> bool {
        self.on.is_some_and(|a| {
            f.now - a.t0 >= UNLOAD_SHOW_S && f.item == a.item && f.fills == a.fills && f.mag.0 > 0
        })
    }

    /// Drop the hold without sending (a panel took the keys, the hand
    /// became a hammer, the body went down).
    pub fn cancel(&mut self) {
        self.on = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUN: u16 = 5;

    fn frame(pressed: bool, down: bool, mag: (u16, u16), fills: u32, now: f64) -> Frame {
        Frame {
            pressed,
            down,
            item: GUN,
            mag,
            fills,
            now,
        }
    }

    #[test]
    fn a_press_reloads_at_once_and_a_fill_spends_the_hold() {
        let mut h = UnloadHold::default();
        assert_eq!(h.step(frame(true, true, (3, 8), 0, 0.0)), Some(Ask::Reload));
        // The fill lands; holding on past the hold sends nothing.
        assert_eq!(h.step(frame(false, true, (8, 8), 1, 0.1)), None);
        assert_eq!(h.step(frame(false, true, (8, 8), 1, 1.0)), None);
        assert_eq!(h.step(frame(false, false, (8, 8), 1, 1.1)), None);
    }

    #[test]
    fn a_hold_with_nothing_to_load_unloads_once() {
        // A dry pack: the reload goes out and comes back refused (no fill).
        let mut h = UnloadHold::default();
        assert_eq!(h.step(frame(true, true, (3, 8), 0, 0.0)), Some(Ask::Reload));
        assert_eq!(h.step(frame(false, true, (3, 8), 0, 0.3)), None);
        assert!(h.showing(frame(false, true, (3, 8), 0, 0.3)));
        assert_eq!(
            h.step(frame(false, true, (3, 8), 0, UNLOAD_HOLD_S)),
            Some(Ask::Unload)
        );
        assert_eq!(h.step(frame(false, true, (3, 8), 0, 2.0)), None, "once");
        assert_eq!(h.step(frame(false, false, (0, 8), 0, 2.1)), None);
    }

    #[test]
    fn a_full_magazine_waits_for_the_release() {
        // A tap still sends the reload, so the key's answer is the sim's.
        let mut h = UnloadHold::default();
        assert_eq!(h.step(frame(true, true, (8, 8), 0, 0.0)), None);
        assert!(
            !h.showing(frame(false, true, (8, 8), 0, 0.1)),
            "a tap shows nothing"
        );
        assert_eq!(
            h.step(frame(false, false, (8, 8), 0, 0.1)),
            Some(Ask::Reload)
        );
        // A hold unloads, and its release sends nothing more.
        assert_eq!(h.step(frame(true, true, (8, 8), 0, 1.0)), None);
        assert_eq!(
            h.step(frame(false, true, (8, 8), 0, 1.6)),
            Some(Ask::Unload)
        );
        assert_eq!(h.step(frame(false, false, (0, 8), 0, 1.7)), None);
    }

    #[test]
    fn nothing_loaded_or_another_hand_unloads_nothing() {
        // Empty and dry: the refusal said so, the hold adds nothing.
        let mut h = UnloadHold::default();
        h.step(frame(true, true, (0, 8), 0, 0.0));
        assert_eq!(h.step(frame(false, true, (0, 8), 0, 1.0)), None);
        // A bow (no readout): `R` picked an arrow, the hold is inert.
        h.step(frame(true, true, (0, 0), 0, 2.0));
        assert_eq!(h.step(frame(false, true, (0, 0), 0, 3.0)), None);
        // Switched hands mid-hold.
        h.step(frame(true, true, (8, 8), 0, 4.0));
        let mut other = frame(false, true, (8, 8), 0, 4.2);
        other.item = GUN + 1;
        assert_eq!(h.step(other), None);
        assert_eq!(h.step(frame(false, true, (8, 8), 0, 5.0)), None);
    }
}
