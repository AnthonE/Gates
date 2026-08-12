//! Admin commands, parsed out of a chat line — **and that is the whole
//! transport decision** (`ALPHA.md` §3's admin lane, admin v0).
//!
//! A command arrives as an ordinary `KIND_CHAT` frame. Nothing on the wire
//! moves: no kind code, no subtype, no `PROTO_VER` bump, no golden. The
//! client needs no build to gain the lane — typing `/kick 259` in the chat
//! box already sends everything the server needs, which also means an
//! admin on an older client is still an admin.
//!
//! It is the reference game's own shape, arrived at from the other end:
//! their console takes text, and a command is a *sentence a person types*
//! rather than a packet a UI builds. The alternative — a `KIND_ADMIN`
//! message with typed fields — would spend a wire version on making the
//! client responsible for a syntax the server has to re-validate anyway.
//!
//! ## The rules this file exists to keep
//!
//! - **Parsing is pure and total.** Any 48 bytes in, `Option` out; a
//!   malformed command is `None` and never a panic. Every field is bounded
//!   by the parse, so a dispatcher downstream is handed numbers it can
//!   use rather than strings it must check.
//! - **It says nothing about permission.** A `Some` here means "this line
//!   is shaped like a command", never "this player may run it" — the
//!   allowlist lives server-side beside the wallets (`server/src/admin.rs`)
//!   and this crate has no business knowing who anybody is.
//! - **`/bug` is in the same table on purpose.** It is not an admin verb —
//!   anyone may file one (`ALPHA.md` §4) — but it is the same *shape*: a
//!   slash line the server acts on rather than relays. Splitting the two
//!   parsers would be two places for a leading slash to mean something.
//!
//! ## Targets are ids, not wallets
//!
//! `/kick 259` names a player by the id the death screen and the chat
//! prefix already show. A 42-character wallet would not fit a 48-byte
//! chat line beside a verb, and an admin reading a name off their own
//! screen is the interaction that actually happens. A **ban** still has
//! to outlive the session, so the server resolves the id to the wallet it
//! already holds — the admin types what they can see and the server
//! records what it can enforce.

use crate::ChatText;

/// The longest note `/bug` carries, bytes. Bounded by the chat line it
/// arrives in, so this is a slice of an already-sanitized text rather
/// than a second cap to keep in step.
pub const BUG_NOTE_MAX: usize = ChatText::CAP;

/// One parsed slash line.
///
/// Deliberately small and `Copy`: it crosses from the parse to the
/// dispatcher by value, like every other record in this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminCmd {
    /// Drop this player's connection now. Their body stays as a sleeper —
    /// a kick is not a wipe.
    Kick { id: u32 },
    /// Kick, and remember the wallet so the next join is refused.
    Ban { id: u32 },
    /// A line from the house, to everybody, marked as the server's.
    Say { text: ChatText },
    /// Put the caller at another player's feet. Self-targeted movement
    /// only — an admin goes to the trouble, never the other way round,
    /// because teleporting a *player* is a verb with no counter-play.
    Teleport { id: u32 },
    /// Put `count` of item row `item` in the caller's own pockets.
    Give { item: u16, count: u16 },
    /// Write the world now rather than at the next cadence.
    SaveNow,
    /// Not an admin verb: anyone may file one. The tick and the filer's
    /// position are stamped by the server, so the note is all a player
    /// has to type (`ALPHA.md` §4).
    Bug { note: ChatText },
}

/// Is this line addressed to the server at all? Cheap enough to ask
/// before the parse, and it is the predicate the fan-out uses to decide
/// whether a line is chat.
#[inline]
pub fn is_command(text: &ChatText) -> bool {
    text.as_bytes().first() == Some(&b'/')
}

/// Parse a slash line, or `None` if it is not one this server knows.
///
/// **An unknown verb is `None` and the line is NOT relayed as chat** —
/// see the dispatcher. Typing `/kik 259` at a griefer should read as a
/// command that failed, never as a public message announcing what you
/// were trying to do.
pub fn parse(text: &ChatText) -> Option<AdminCmd> {
    if !is_command(text) {
        return None;
    }
    // Valid UTF-8 by `ChatText`'s construction, and control characters are
    // already refused there, so a split on spaces is total. Slicing at 1 is
    // safe on a char boundary because `is_command` just proved byte 0 is an
    // ASCII slash.
    let line = core::str::from_utf8(text.as_bytes()).ok()?;
    // **The verb and its tail are split ONCE, and the tail is what is left
    // over rather than a byte offset computed from the verb's length.**
    // That arithmetic was a bug: with a space after the slash — `/ say hi`,
    // which `ChatText::sanitize` permits because it only trims the ENDS —
    // `1 + verb.len()` pointed into the middle of the tail and `/ say hi`
    // broadcast "y hi". Deriving the remainder cannot drift that way, and
    // it also cannot land mid-character.
    let body = line[1..].trim_start_matches(' ');
    let (verb, rest) = match body.split_once(' ') {
        Some((v, r)) => (v, r),
        None => (body, ""),
    };
    let mut parts = rest.split(' ').filter(|s| !s.is_empty());
    match verb {
        "kick" => Some(AdminCmd::Kick {
            id: parts.next()?.parse().ok()?,
        }),
        "ban" => Some(AdminCmd::Ban {
            id: parts.next()?.parse().ok()?,
        }),
        "tp" => Some(AdminCmd::Teleport {
            id: parts.next()?.parse().ok()?,
        }),
        "give" => {
            let item = parts.next()?.parse().ok()?;
            // A bare `/give 7` means one, which is what a person means.
            let count = match parts.next() {
                Some(c) => c.parse().ok()?,
                None => 1,
            };
            // Zero is a no-op dressed as a verb; refuse it at the parse so
            // the dispatcher never has to have an opinion.
            (count > 0).then_some(AdminCmd::Give { item, count })
        }
        "save" => Some(AdminCmd::SaveNow),
        // The tail of these two is free text, so it is the remainder
        // whole — never rejoined from the split, which would eat the runs
        // of spaces a person typed. Re-sanitized so it is a `ChatText` by
        // the same rule the whole line was; `None` for an empty tail,
        // because a `/say` with nothing to say is not a command.
        "say" => ChatText::sanitize(rest.as_bytes()).map(|text| AdminCmd::Say { text }),
        "bug" => ChatText::sanitize(rest.as_bytes()).map(|note| AdminCmd::Bug { note }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ChatText {
        ChatText::sanitize(s.as_bytes()).expect("a legal chat line")
    }

    #[test]
    fn the_verbs_parse_with_their_arguments() {
        assert_eq!(parse(&text("/kick 259")), Some(AdminCmd::Kick { id: 259 }));
        assert_eq!(parse(&text("/ban 260")), Some(AdminCmd::Ban { id: 260 }));
        assert_eq!(
            parse(&text("/tp 261")),
            Some(AdminCmd::Teleport { id: 261 })
        );
        assert_eq!(
            parse(&text("/give 7 30")),
            Some(AdminCmd::Give { item: 7, count: 30 })
        );
        assert_eq!(parse(&text("/save")), Some(AdminCmd::SaveNow));
    }

    /// A bare `/give` means one of the thing, and a zero count is refused
    /// rather than executed as nothing.
    #[test]
    fn give_defaults_to_one_and_refuses_zero() {
        assert_eq!(
            parse(&text("/give 7")),
            Some(AdminCmd::Give { item: 7, count: 1 })
        );
        assert_eq!(parse(&text("/give 7 0")), None);
    }

    /// The free-text tail keeps what was typed, spaces included, and an
    /// empty one is not a command.
    #[test]
    fn the_text_verbs_keep_their_tail() {
        let Some(AdminCmd::Say { text: t }) = parse(&text("/say wipe in 5 minutes")) else {
            panic!("say did not parse");
        };
        assert_eq!(t.as_bytes(), b"wipe in 5 minutes");
        let Some(AdminCmd::Bug { note }) = parse(&text("/bug fell through the floor")) else {
            panic!("bug did not parse");
        };
        assert_eq!(note.as_bytes(), b"fell through the floor");
        assert_eq!(parse(&text("/say")), None);
        assert_eq!(parse(&text("/bug   ")), None);
    }

    /// Every rejection path, and the property that matters about all of
    /// them: a line that is not a command this server knows parses to
    /// `None` rather than to something.
    #[test]
    fn nonsense_is_refused_and_never_panics() {
        for s in [
            "hello",             // not a command at all
            "/",                 // no verb
            "/kik 259",          // unknown verb
            "/kick",             // missing argument
            "/kick abc",         // unparseable id
            "/kick 99999999999", // past a u32
            "/give abc 1",
            "/give 7 99999",
            "//",
            "/ 259",
        ] {
            assert_eq!(parse(&text(s)), None, "{s:?} should not parse");
        }
    }

    /// Extra spaces between a verb and its argument are a person typing,
    /// not a syntax error.
    #[test]
    fn runs_of_spaces_between_arguments_are_forgiven() {
        assert_eq!(
            parse(&text("/kick   259")),
            Some(AdminCmd::Kick { id: 259 })
        );
    }

    /// **A space after the slash is a person typing too, and it used to
    /// corrupt the tail.** The first cut computed the free-text tail as
    /// `1 + verb.len()` bytes into the line, which is only right when the
    /// verb starts at byte 1 — so `/ say hi` broadcast "y hi" and
    /// `/  say hello` broadcast "ay hello". Found by review; the
    /// regression is cheap to pin.
    #[test]
    fn a_space_after_the_slash_does_not_eat_the_tail() {
        for line in ["/say hi", "/ say hi", "/   say hi"] {
            let Some(AdminCmd::Say { text: t }) = parse(&text(line)) else {
                panic!("{line:?} did not parse as a say");
            };
            assert_eq!(t.as_bytes(), b"hi", "{line:?} mangled its tail");
        }
        assert_eq!(parse(&text("/ kick 259")), Some(AdminCmd::Kick { id: 259 }));
    }

    /// Multi-byte UTF-8 anywhere in the line must not panic — this parse
    /// runs on a `ChatText`, which permits any non-control UTF-8, and a
    /// panic on the sim thread is a shard crash any player could trigger
    /// by typing in chat.
    #[test]
    fn multibyte_text_never_panics() {
        for s in [
            "/say h\u{e9}llo w\u{f6}rld",
            "/\u{65e5}\u{672c}\u{8a9e}",
            "/ \u{65e5}\u{672c}\u{8a9e} \u{5f15}\u{6570}",
            "/say \u{1f525}\u{1f525}",
            "/kick \u{65e5}\u{672c}\u{8a9e}",
            "/give \u{4e03} 7",
            "/\u{65e5}",
            "/ ",
            "/  ",
        ] {
            // The assertion is that this RETURNS rather than panicking;
            // what it returns is each verb's own business above.
            let _ = parse(&text(s));
        }
        // And the one that must still work with a multi-byte tail.
        let Some(AdminCmd::Say { text: t }) = parse(&text("/say h\u{e9}llo")) else {
            panic!("a multi-byte tail must survive");
        };
        assert_eq!(t.as_bytes(), "h\u{e9}llo".as_bytes());
    }

    /// `is_command` and `parse` must agree about the leading slash, or a
    /// line could be withheld from chat by one and refused by the other —
    /// which is how a command would silently vanish.
    #[test]
    fn is_command_brackets_the_parser() {
        assert!(is_command(&text("/kick 259")));
        assert!(is_command(&text("/nonsense")));
        assert!(!is_command(&text("hello")));
        assert!(!is_command(&text("no /slash here")));
        // Anything the parser accepts, `is_command` must have admitted.
        assert!(parse(&text("hello")).is_none());
    }
}
