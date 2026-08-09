//! Join links — `scry://join/gates/host:port`, and the game's own
//! `gates://host:port`.
//!
//! **The thing a player actually wants: a friend pastes a link, and the game
//! comes up on the shard they are standing on.** Everything below is in
//! service of that one sentence, and the interesting decisions are about who
//! owns the scheme and what a link is allowed to say.
//!
//! ## Two schemes, and the platform one is canonical
//!
//! - **`scry://join/<slug>/<host:port>`** — the form to share. The launcher
//!   owns it: it is the binary a player installs, it already knows how to
//!   start a title with `{server}` substituted into its launch args
//!   (`scry-depot`'s `ARG_VARS`), it already holds the identity the shard will
//!   ask to sign, and one scheme serves every title on the platform rather
//!   than one per game. A link naming a title the player has not installed is
//!   therefore still a *useful* link — the launcher can offer to install it,
//!   which a game-owned scheme could never do because the game is what is
//!   missing.
//! - **`gates://<host:port>`** — the game's own short form, parsed here for
//!   the case where the OS hands a url straight to this binary. It costs one
//!   match arm and it means a url is never a dead end: if it reaches the game,
//!   the game understands it.
//!
//! Both land in the same place — [`Join`] — and both are equivalent to having
//! typed `--server host:port`, which is deliberate. A deeplink is a way of
//! *spelling* an address, not a second way of connecting: it sets the same
//! field, skips the same menu, and gets the same shape check. There is no code
//! path a link can reach that an argv cannot.
//!
//! ## What a link may NOT say
//!
//! This is the security boundary and it is the reason this parser is a
//! whitelist rather than a url crate. A join link is **untrusted input from a
//! stranger** — that is its whole purpose, a thing pasted into a chat window —
//! so it may carry exactly two facts: which title, and which address. It may
//! not name an identity (the launcher holds that and a link that could set it
//! would be a phishing primitive), a token, a passphrase, a file path, a
//! content directory, or any other flag. Anything past the address is refused
//! rather than ignored, because "ignored" is how an unnoticed field becomes a
//! supported one.
//!
//! The address itself is then run through `shardlist::check_addr`, the same
//! shape check every other address in this client gets — so a link cannot
//! smuggle a url, a path, whitespace, or a port of zero into the transport.

use crate::shardlist::{check_addr, MAX_URL_BYTES};

/// The two schemes a join link may use. `scry` is canonical; see the module
/// docs for why `gates` exists as well.
pub const SCHEMES: [&str; 2] = ["scry", "gates"];

/// This game's slug on the platform, as `data/launcher/gates.manifest.json`
/// spells it. A `scry://join/<slug>/…` naming any other title is refused
/// here: this binary can only join Gates shards, and silently ignoring the
/// slug would make `scry://join/some-other-game/evil:1` a Gates join link.
pub const SLUG: &str = "gates";

/// What a join link resolves to. Two facts, and deliberately no more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Join {
    /// The title the link named, when it named one. `None` for the short
    /// `gates://host:port` form, which names it by scheme.
    pub slug: Option<String>,
    /// `host:port`, already through `shardlist::check_addr`.
    pub addr: String,
}

/// Is this argument a join link at all?
///
/// Cheap and total, so the argv parser can branch on it before trying to read
/// the token as an address. Deliberately does not validate — a *malformed*
/// link must reach [`parse`] and be refused with a reason, not fall through to
/// the address parser and be refused as "bad address", which names the wrong
/// thing.
pub fn is_link(s: &str) -> bool {
    let t = s.trim();
    SCHEMES
        .iter()
        .any(|scheme| t.len() > scheme.len() + 3 && t[..scheme.len() + 3].eq_ignore_ascii_case(&format!("{scheme}://")))
}

/// Parse a join link.
///
/// Every refusal names what was wrong and, where there is one, the fix — this
/// string is printed to a player who was handed a link by a friend and has no
/// idea what a shard is.
pub fn parse(link: &str) -> Result<Join, String> {
    let raw = link.trim();
    if raw.len() > MAX_URL_BYTES {
        return Err(format!(
            "join link is {} bytes, over the {MAX_URL_BYTES}-byte cap",
            raw.len()
        ));
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| format!("{raw:?} is not a join link — expected scry://join/{SLUG}/host:port"))?;
    let scheme = scheme.to_ascii_lowercase();
    if !SCHEMES.contains(&scheme.as_str()) {
        return Err(format!(
            "{scheme:?} is not a join scheme — expected {}",
            SCHEMES
                .iter()
                .map(|s| format!("{s}://"))
                .collect::<Vec<_>>()
                .join(" or ")
        ));
    }

    // A query or fragment is dropped, not refused: a link that survived a
    // chat client often arrives with tracking junk stapled to it, and that is
    // not the player's fault. Nothing past the address is ever *read* — see
    // the module docs on what a link may not say.
    let rest = rest
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    // `scry://join/gates/h:1` → ["join", "gates", "h:1"]
    // `gates://h:1`           → ["h:1"]
    // `gates://join/h:1`      → ["join", "h:1"]
    //
    // The VERB is matched case-insensitively and the address is not touched:
    // a url's scheme and path are conventionally lowercase but a player pastes
    // what they were sent, while `addr` is a DNS name whose case belongs to
    // whoever wrote the shard list — lowercasing it here would be this parser
    // editing an address on its way to the transport.
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    let verb_is = |v: &str| parts.first().is_some_and(|p| p.eq_ignore_ascii_case(v));
    let bad_shape = || {
        format!(
            "{raw:?} has nothing joinable in it — expected \
             scry://join/{SLUG}/host:port or {SLUG}://host:port"
        )
    };
    let (slug, addr) = match scheme.as_str() {
        // The canonical form. The verb is required on `scry://` because that
        // scheme is the whole platform's and `join` is one of the things it
        // could mean.
        "scry" if verb_is("join") => match parts.as_slice() {
            [_, slug, addr] => (Some((*slug).to_string()), *addr),
            _ => {
                return Err(format!(
                    "a scry join link names a title and an address — \
                     expected scry://join/{SLUG}/host:port"
                ))
            }
        },
        "scry" => {
            return Err(match parts.first() {
                Some(verb) => format!(
                    "scry://{verb}/… is not a join link — \
                     expected scry://join/{SLUG}/host:port"
                ),
                None => bad_shape(),
            })
        }
        // The game's own scheme names the title by being itself, so the verb
        // is optional.
        "gates" if verb_is("join") => match parts.as_slice() {
            [_, addr] => (None, *addr),
            _ => return Err(bad_shape()),
        },
        "gates" => match parts.as_slice() {
            [addr] => (None, *addr),
            _ => return Err(bad_shape()),
        },
        _ => return Err(bad_shape()),
    };

    if let Some(s) = &slug {
        // Refused, never ignored: ignoring it would make a link for any other
        // title a Gates join link pointing wherever that link said.
        if !s.eq_ignore_ascii_case(SLUG) {
            return Err(format!(
                "this is a join link for {s:?}, and this game is {SLUG:?}"
            ));
        }
    }

    let addr = percent_decode(addr)?;
    check_addr(&addr).map_err(|why| format!("join link: {why}"))?;
    Ok(Join {
        slug: slug.map(|s| s.to_ascii_lowercase()),
        addr,
    })
}

/// Undo percent-encoding in the address segment.
///
/// Worth the twenty lines because a link's one special character is the colon
/// in `host:port`, and a chat client that encodes it turns a working link into
/// a refusal the player cannot diagnose. **Decoding is safe here only because
/// `check_addr` runs after it** — a `%2F` that becomes a slash, or a `%20`
/// that becomes a space, is caught by the same rules that catch a typed one.
/// Decode-then-validate; never the reverse.
fn percent_decode(s: &str) -> Result<String, String> {
    if !s.contains('%') {
        return Ok(s.to_string());
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b
                .get(i + 1..i + 3)
                .ok_or_else(|| format!("join link: {s:?} ends in a half-written % escape"))?;
            let hex = std::str::from_utf8(hex)
                .map_err(|_| format!("join link: {s:?} has a bad % escape"))?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|_| format!("join link: {s:?} has a bad % escape %{hex}"))?;
            out.push(byte);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("join link: {s:?} decodes to invalid utf-8"))
}

/// The link that would bring someone else to `addr` — what a "copy join link"
/// button puts on the clipboard, and what the in-game menu shows.
///
/// The canonical scheme, always: a shared link should work for a player who
/// has the launcher and not this game, and only `scry://` can offer to install
/// it.
pub fn link_for(addr: &str) -> String {
    format!("scry://join/{SLUG}/{addr}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canonical_form_is_what_a_friend_shares() {
        let j = parse("scry://join/gates/game.moreright.xyz:61234").expect("canonical");
        assert_eq!(j.slug.as_deref(), Some("gates"));
        assert_eq!(j.addr, "game.moreright.xyz:61234");
        // ...and it round-trips through the thing that writes it.
        assert_eq!(
            link_for("game.moreright.xyz:61234"),
            "scry://join/gates/game.moreright.xyz:61234"
        );
        assert_eq!(
            parse(&link_for("game.moreright.xyz:61234")).unwrap().addr,
            "game.moreright.xyz:61234"
        );
    }

    #[test]
    fn the_games_own_scheme_needs_no_verb() {
        assert_eq!(parse("gates://h:1").unwrap().addr, "h:1");
        assert_eq!(parse("gates://join/h:1").unwrap().addr, "h:1");
        // It names the title by being itself, so there is no slug to carry.
        assert_eq!(parse("gates://h:1").unwrap().slug, None);
    }

    #[test]
    fn a_link_for_another_title_is_refused_and_not_ignored() {
        // The one that matters. Ignoring the slug would make every other
        // title's join link a Gates join link pointing wherever it said.
        let why = parse("scry://join/some-other-game/evil.test:1").expect_err("wrong slug");
        assert!(why.contains("some-other-game"), "{why}");
        assert!(why.contains("gates"), "{why}");
    }

    #[test]
    fn a_link_may_say_a_title_and_an_address_and_nothing_else() {
        // Every one of these is a field a link must not be able to set. They
        // are refused rather than dropped: a silently ignored segment is how
        // an unnoticed field becomes a supported one.
        for bad in [
            "scry://join/gates/h:1/0xdeadbeef",
            "scry://join/gates/h:1/--identity/0x1",
            "scry://join/gates",
            "scry://join",
            "scry://install/gates/h:1",
            "scry://gates/h:1",
            "gates://join/h:1/extra",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_query_string_is_dropped_rather_than_refused() {
        // A link that survived a chat client arrives with junk stapled on,
        // and that is not the player's fault. Dropped — never read.
        assert_eq!(parse("scry://join/gates/h:1?utm=x").unwrap().addr, "h:1");
        assert_eq!(parse("scry://join/gates/h:1#frag").unwrap().addr, "h:1");
        assert_eq!(parse("scry://join/gates/h:1/").unwrap().addr, "h:1");
    }

    #[test]
    fn an_encoded_colon_still_joins() {
        // The practical half: some chat clients encode the one special
        // character a host:port has.
        assert_eq!(parse("scry://join/gates/h%3A1").unwrap().addr, "h:1");
        // ...and decoding cannot smuggle anything past check_addr, because
        // check_addr runs after it.
        assert!(parse("scry://join/gates/h%2Fx:1").is_err()); // a slash
        assert!(parse("scry://join/gates/h%20x:1").is_err()); // a space
        assert!(parse("scry://join/gates/h:1%00").is_err()); // a NUL
        assert!(parse("scry://join/gates/h:%").is_err()); // half an escape
    }

    #[test]
    fn the_address_gets_the_same_shape_check_as_a_typed_one() {
        // A link is a way of spelling an address, not a second way in.
        for bad in [
            "scry://join/gates/nonsense",
            "scry://join/gates/h:0",
            "scry://join/gates/h:port",
            "scry://join/gates/::1:4433",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(parse("scry://join/gates/[::1]:4433").is_ok());
    }

    #[test]
    fn junk_is_refused_without_panicking() {
        for junk in [
            "",
            "://",
            "scry://",
            "gates://",
            "http://example.test/join",
            "javascript:alert(1)",
            "scry:/join/gates/h:1",
            "not a link at all",
            "scry://join/gates/",
        ] {
            assert!(parse(junk).is_err(), "{junk:?} should be refused");
        }
        // Over the cap, and it must say so rather than work on a truncation.
        let long = format!("scry://join/gates/{}:1", "h".repeat(MAX_URL_BYTES));
        assert!(parse(&long).is_err());
    }

    #[test]
    fn recognising_a_link_is_separate_from_accepting_one() {
        // `is_link` decides which error the player sees. A malformed link
        // must reach `parse` and be refused as a bad LINK, not fall through
        // to the address parser and be refused as a bad address.
        assert!(is_link("scry://join/gates/h:1"));
        assert!(is_link("gates://h:1"));
        assert!(is_link("SCRY://join/gates/h:1"));
        assert!(is_link("scry://nonsense")); // malformed, but still a link
        assert!(!is_link("h:1"));
        assert!(!is_link("game.moreright.xyz:61234"));
        assert!(!is_link("--server"));
        assert!(!is_link(""));
        // The one that would misfire on a naive `contains("://")`.
        assert!(!is_link("https://example.test/servers.json"));
    }

    #[test]
    fn a_scheme_is_case_insensitive_because_a_url_is() {
        assert_eq!(parse("SCRY://JOIN/GATES/h:1").unwrap().addr, "h:1");
        assert_eq!(parse("Gates://h:1").unwrap().addr, "h:1");
    }
}
