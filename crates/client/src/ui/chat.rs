//! The chat composer and the scrollback, as arithmetic.
//!
//! Chat was the one C→S message kind the native client could not send:
//! `KIND_CHAT` had no key and `pop_chat` had no reader, so the island was
//! silent in both directions. `ALPHA.md` §1 has it as global text plus a
//! 20 m local radius, and the server decides what those mean — the client
//! chooses a channel and types.
//!
//! Pure and in `ui/` because the two things that can be wrong here are both
//! arithmetic: **what `/g` does to the line that gets sent**, and **what the
//! composer does at the byte cap**. Both are testable without a window.
//!
//! ## The cap is bytes, not characters
//!
//! `CHAT_MAX_BYTES` is 48 UTF-8 *bytes* and `sanitize` refuses anything over
//! it outright — it does not truncate. A composer that counted characters
//! would let a player type 48 emoji, refuse to encode, and report nothing;
//! one that truncated bytes could split a codepoint and produce a line that
//! is not UTF-8, which `sanitize` also refuses. So [`Composer::push`] admits
//! a character only when the whole of it fits, which is the only rule that
//! keeps "what you can type" and "what will send" the same set.

use protocol::chat::{ChatText, CHAT_MAX_BYTES};

/// The `/g ` prefix that sends a line to everyone instead of to the 20 m
/// radius. Consumed by the client — the server is told the channel in a
/// bit, and never sees these two characters.
pub const GLOBAL_PREFIX: &str = "/g ";

/// How many lines the scrollback holds. Bounded like every other client
/// store (wall 4); the oldest is evicted.
pub const LOG_CAP: usize = 24;

/// The line being typed.
#[derive(Debug, Default, Clone)]
pub struct Composer {
    pub text: String,
    pub open: bool,
}

impl Composer {
    /// Add one character if the whole of it fits inside the byte cap.
    ///
    /// Returns whether it went in, so a caller can decide whether anything
    /// needs redrawing. Control characters are refused here rather than at
    /// encode: `sanitize` would refuse the whole line for one of them, and
    /// silently losing a typed line is worse than never accepting the key.
    pub fn push(&mut self, c: char) -> bool {
        if c.is_control() {
            return false;
        }
        if self.text.len() + c.len_utf8() > CHAT_MAX_BYTES {
            return false;
        }
        self.text.push(c);
        true
    }

    /// Remove the last character. Pops a whole codepoint, never a byte.
    pub fn backspace(&mut self) -> bool {
        self.text.pop().is_some()
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    /// How many bytes are left. Drawn beside the composer, because a cap the
    /// player cannot see is a cap that feels like a broken keyboard.
    pub fn remaining(&self) -> usize {
        CHAT_MAX_BYTES.saturating_sub(self.text.len())
    }

    /// What the composer would send: the channel, and the sanitized line.
    ///
    /// `None` when there is nothing to send — an empty line, a line that is
    /// only the prefix, or one `sanitize` refuses. The prefix is stripped
    /// **before** sanitizing so that `/g` costs the player none of their 48
    /// bytes.
    pub fn outgoing(&self) -> Option<(bool, ChatText)> {
        let (global, body) = match self.text.strip_prefix(GLOBAL_PREFIX) {
            Some(rest) => (true, rest),
            // `/g` alone, with nothing after it, is a channel switch with no
            // message — not a line reading "/g".
            None if self.text.trim_end() == "/g" => return None,
            None => (false, self.text.as_str()),
        };
        ChatText::sanitize(body.as_bytes()).map(|t| (global, t))
    }
}

/// One line in the scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub speaker: u32,
    pub global: bool,
    pub text: String,
    /// True for the client's own lines, so they can be drawn differently.
    pub own: bool,
}

/// The scrollback: bounded, oldest evicted.
#[derive(Debug, Default)]
pub struct Log {
    pub lines: Vec<Line>,
}

impl Log {
    pub fn push(&mut self, line: Line) {
        if self.lines.len() >= LOG_CAP {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }

    /// How a line reads on screen.
    ///
    /// The speaker is an id, not a name: nothing on our wire carries a
    /// display name, and inventing one client-side would be a name that
    /// differs between two players watching the same conversation.
    pub fn render(line: &Line) -> String {
        let who = if line.own {
            "you".to_string()
        } else {
            format!("#{}", line.speaker)
        };
        let tag = if line.global { "[G] " } else { "" };
        format!("{tag}{who}: {}", line.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composed(s: &str) -> Composer {
        Composer {
            text: s.to_string(),
            open: true,
        }
    }

    #[test]
    fn a_plain_line_is_local() {
        let (global, text) = composed("hello").outgoing().expect("sends");
        assert!(!global);
        assert_eq!(text.as_bytes(), b"hello");
    }

    /// The prefix is the client's and never reaches the wire — the channel
    /// crosses as a bit.
    #[test]
    fn the_global_prefix_is_stripped_and_costs_no_bytes() {
        let (global, text) = composed("/g hello").outgoing().expect("sends");
        assert!(global);
        assert_eq!(text.as_bytes(), b"hello");
    }

    #[test]
    fn a_bare_channel_switch_sends_nothing() {
        assert!(composed("/g").outgoing().is_none());
        assert!(composed("/g   ").outgoing().is_none());
        assert!(composed("/g ").outgoing().is_none());
    }

    #[test]
    fn an_empty_line_sends_nothing() {
        assert!(composed("").outgoing().is_none());
        assert!(composed("    ").outgoing().is_none());
    }

    /// The cap is bytes. A composer that counted characters would let a
    /// player type a line that then refuses to encode with nothing said.
    #[test]
    fn the_cap_is_bytes_and_admits_only_whole_characters() {
        let mut c = Composer::default();
        // 'é' is two bytes; fill to one byte short of the cap.
        while c.text.len() < CHAT_MAX_BYTES - 1 {
            assert!(c.push('a'));
        }
        assert_eq!(c.remaining(), 1);
        // Two bytes will not fit in one, and must not go in half.
        assert!(!c.push('é'));
        assert_eq!(c.text.len(), CHAT_MAX_BYTES - 1);
        // One byte still does.
        assert!(c.push('a'));
        assert_eq!(c.remaining(), 0);
        assert!(!c.push('a'));
    }

    /// Whatever the composer let you type must be something `sanitize`
    /// accepts — the whole point of enforcing the cap at the keystroke.
    #[test]
    fn anything_the_composer_accepts_will_encode() {
        let mut c = Composer::default();
        for ch in "héllo wörld ¡buenos días! ✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓✓".chars()
        {
            c.push(ch);
        }
        assert!(c.text.len() <= CHAT_MAX_BYTES);
        let (_, text) = c.outgoing().expect("a non-empty line must send");
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
    }

    #[test]
    fn backspace_pops_a_codepoint_not_a_byte() {
        let mut c = composed("aé");
        assert_eq!(c.text.len(), 3);
        assert!(c.backspace());
        assert_eq!(c.text, "a");
        assert!(c.backspace());
        assert!(!c.backspace());
    }

    #[test]
    fn control_characters_are_refused_at_the_key() {
        let mut c = Composer::default();
        assert!(!c.push('\n'));
        assert!(!c.push('\t'));
        assert!(c.text.is_empty());
    }

    #[test]
    fn the_log_is_bounded_and_evicts_the_oldest() {
        let mut log = Log::default();
        for i in 0..LOG_CAP + 5 {
            log.push(Line {
                speaker: i as u32,
                global: false,
                text: format!("line {i}"),
                own: false,
            });
        }
        assert_eq!(log.lines.len(), LOG_CAP);
        assert_eq!(log.lines[0].text, format!("line {}", 5));
    }

    #[test]
    fn a_line_names_its_channel_and_its_speaker() {
        let l = Line {
            speaker: 4,
            global: true,
            text: "hi".into(),
            own: false,
        };
        assert_eq!(Log::render(&l), "[G] #4: hi");
        let l = Line {
            speaker: 4,
            global: false,
            text: "hi".into(),
            own: true,
        };
        assert_eq!(Log::render(&l), "you: hi");
    }
}
