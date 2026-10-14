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

use protocol::chat::{Channel, ChatText, CHAT_MAX_BYTES};
use protocol::Address;

/// The `/g ` prefix that sends a line to everyone instead of to the 20 m
/// radius. Consumed by the client — the server is told the channel in a
/// bit, and never sees these two characters.
pub const GLOBAL_PREFIX: &str = "/g ";
/// The `/t ` prefix that sends a line to the team (wire v108), wherever
/// they are. The client's, like `/g `.
pub const TEAM_PREFIX: &str = "/t ";

/// How many lines the scrollback holds. Bounded like every other client
/// store (wall 4); the oldest is evicted.
pub const LOG_CAP: usize = 24;

/// How long a line stays up with the composer shut, then how long it takes
/// to fade. Rust's chat: a line is there while it is news, and opening chat
/// brings the recent ones back.
pub const SHOW_S: f32 = 10.0;
pub const FADE_S: f32 = 1.5;

/// The line being typed.
#[derive(Debug, Default, Clone)]
pub struct Composer {
    pub text: String,
    pub open: bool,
    /// Where an unprefixed line goes. Sticky across opens, and `Tab` cycles
    /// it, the way Rust's chat remembers Team vs Global.
    pub channel: Channel,
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

    /// `Tab`: Local → Team → Global → Local.
    pub fn cycle(&mut self) {
        self.channel = match self.channel {
            Channel::Local => Channel::Team,
            Channel::Team => Channel::Global,
            Channel::Global => Channel::Local,
        };
    }

    /// The channel the line would go to as typed: a `/g ` or `/t ` prefix
    /// wins over the sticky one.
    pub fn target(&self) -> Channel {
        if self.text.starts_with(GLOBAL_PREFIX) || self.text.trim_end() == "/g" {
            Channel::Global
        } else if self.text.starts_with(TEAM_PREFIX) || self.text.trim_end() == "/t" {
            Channel::Team
        } else {
            self.channel
        }
    }

    /// The typed line without its channel prefix, as drawn in the bar.
    pub fn body(&self) -> &str {
        self.text
            .strip_prefix(GLOBAL_PREFIX)
            .or_else(|| self.text.strip_prefix(TEAM_PREFIX))
            .unwrap_or(&self.text)
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
    pub fn outgoing(&self) -> Option<(Channel, ChatText)> {
        let (channel, body) = if let Some(rest) = self.text.strip_prefix(GLOBAL_PREFIX) {
            (Channel::Global, rest)
        } else if let Some(rest) = self.text.strip_prefix(TEAM_PREFIX) {
            (Channel::Team, rest)
        } else if matches!(self.text.trim_end(), "/g" | "/t") {
            // `/g` alone, with nothing after it, is a channel switch with no
            // message — not a line reading "/g".
            return None;
        } else {
            (self.channel, self.text.as_str())
        };
        ChatText::sanitize(body.as_bytes()).map(|t| (channel, t))
    }
}

/// One line in the scrollback.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub speaker: u32,
    /// What the speaker is called (`ui::names::label`), settled when the
    /// line arrived so the scrollback never renames a line under a reader.
    pub name: String,
    pub channel: Channel,
    pub text: String,
    /// The speaker's wallet and picture revision, for the avatar, settled
    /// with the name. `None` for a guest or a tag that has not landed.
    pub face: Option<(Address, u32)>,
    /// When it arrived, seconds on the client's clock. The fade reads it.
    pub at: f64,
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

    /// The channel tag drawn before a name: Rust's `[Team]`. Local is the
    /// default and goes bare.
    pub fn tag(channel: Channel) -> &'static str {
        match channel {
            Channel::Global => "[Global]",
            Channel::Team => "[Team]",
            Channel::Local => "",
        }
    }

    /// The coloured half of a line: tag and name, then a colon. The name is
    /// the one the shard told everyone (`EventMsg::Tag`), so two players
    /// reading the same conversation read the same names.
    pub fn head(line: &Line) -> String {
        match Self::tag(line.channel) {
            "" => format!("{}: ", line.name),
            tag => format!("{tag} {}: ", line.name),
        }
    }

    /// How visible a line `age_s` old is: whole while it is news or while
    /// the composer is open, then a fade to nothing.
    pub fn alpha(age_s: f32, open: bool) -> f32 {
        if open {
            return 1.0;
        }
        (1.0 - (age_s - SHOW_S) / FADE_S).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composed(s: &str) -> Composer {
        Composer {
            text: s.to_string(),
            open: true,
            channel: Channel::Local,
        }
    }

    #[test]
    fn a_plain_line_is_local() {
        let (channel, text) = composed("hello").outgoing().expect("sends");
        assert_eq!(channel, Channel::Local);
        assert_eq!(text.as_bytes(), b"hello");
    }

    #[test]
    fn the_team_prefix_is_stripped_like_the_global_one() {
        let (channel, text) = composed("/t on my way").outgoing().expect("sends");
        assert_eq!(channel, Channel::Team);
        assert_eq!(text.as_bytes(), b"on my way");
        assert!(composed("/t").outgoing().is_none());
        assert!(composed("/t ").outgoing().is_none());
    }

    /// The prefix is the client's and never reaches the wire — the channel
    /// crosses as a bit.
    #[test]
    fn the_global_prefix_is_stripped_and_costs_no_bytes() {
        let (channel, text) = composed("/g hello").outgoing().expect("sends");
        assert_eq!(channel, Channel::Global);
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
                name: format!("#{i}"),
                channel: Channel::Local,
                text: format!("line {i}"),
                face: None,
                at: 0.0,
            });
        }
        assert_eq!(log.lines.len(), LOG_CAP);
        assert_eq!(log.lines[0].text, format!("line {}", 5));
    }

    #[test]
    fn a_line_names_its_channel_and_its_speaker() {
        let l = Line {
            speaker: 4,
            name: "Ash".into(),
            channel: Channel::Team,
            text: "hi".into(),
            face: None,
            at: 0.0,
        };
        assert_eq!(Log::head(&l), "[Team] Ash: ");
        let l = Line {
            channel: Channel::Global,
            ..l
        };
        assert_eq!(Log::head(&l), "[Global] Ash: ");
        let l = Line {
            channel: Channel::Local,
            ..l
        };
        assert_eq!(Log::head(&l), "Ash: ");
    }

    /// `Tab` picks where an unprefixed line goes; a typed prefix still wins.
    #[test]
    fn the_sticky_channel_sends_unprefixed_lines_and_a_prefix_overrides_it() {
        let mut c = composed("hello");
        c.cycle();
        assert_eq!(c.channel, Channel::Team);
        assert_eq!(c.outgoing().map(|(ch, _)| ch), Some(Channel::Team));
        c.text = "/g hello".into();
        assert_eq!(c.target(), Channel::Global);
        assert_eq!(c.body(), "hello");
        assert_eq!(c.outgoing().map(|(ch, _)| ch), Some(Channel::Global));
        c.cycle();
        c.cycle();
        assert_eq!(c.channel, Channel::Local, "three steps go round");
        c.clear();
        c.cycle();
        assert_eq!(
            c.channel,
            Channel::Team,
            "clearing the line keeps the channel"
        );
    }

    #[test]
    fn a_line_is_whole_then_fades_unless_chat_is_open() {
        assert_eq!(Log::alpha(0.0, false), 1.0);
        assert_eq!(Log::alpha(SHOW_S, false), 1.0);
        let mid = Log::alpha(SHOW_S + FADE_S * 0.5, false);
        assert!(mid > 0.0 && mid < 1.0);
        assert_eq!(Log::alpha(SHOW_S + FADE_S, false), 0.0);
        assert_eq!(Log::alpha(1000.0, true), 1.0, "open chat shows the log");
    }
}
