//! The title manifest — where the launcher says this game's own pages live.
//!
//! **The launcher hands out URLs; it does not proxy documents.** That rule is
//! stated twice in the vendored SDK, once on `Overlay::servers_url` and once
//! on `Overlay::title` ("⚠ It returns a url, not a title object — the launcher
//! does not proxy a game's own documents"), and it is the whole shape of this
//! file: `title(slug)` answers with a URL, the game fetches that URL itself,
//! and what comes back is this.
//!
//! **What it is for.** The reference's main menu carries NEWS, ITEM STORE and
//! WORKSHOP beside PLAY GAME, and every one of the three is a live service
//! rather than a screen the game draws from nothing. Ours are the scry-works
//! launcher's (operator, 2026-08-09: *"we have NEWS thats part of the
//! community on the scry-works launcher … we do have an item store and
//! workshop. that again is part of the scry-works setup"*). This document is
//! how the game learns where they are.
//!
//! ## Three rules it inherits, and one it adds
//!
//! Inherited from [`crate::shardlist`], because this is the same kind of
//! input — bytes off a network, from a host named by another host:
//!
//! - **Bounded.** Wall 4: the read is capped and one byte over is an error,
//!   not a truncation.
//! - **Refuse, don't guess.** A field that is not shaped like a URL costs that
//!   field, never the document.
//! - **Additive and forward-compatible.** An unknown key is ignored rather
//!   than refused: the platform will grow this file, and an older game must
//!   keep working against a newer manifest.
//!
//! The one it adds is about *money*, and it is the reason the store is a link
//! rather than a screen. The SDK says it in full on `Overlay::overlay`:
//!
//! > ⚠ **Display freely; never approve.** A game controls its own pixels, so
//! > it can draw a convincing fake of any dialog.
//!
//! So a game must never take a payment in its own window. `ITEM STORE` here
//! resolves to a URL the *launcher* opens (`Overlay::open_url`), in the
//! launcher's own window, where the player can tell a real checkout from one
//! the game drew. `BUSINESS.md` owns what is sold; this owns only the door.
//!
//! **PROPOSED — `DECISIONS.md` §open, "title manifest v0".** `servers.url` is
//! the key the platform already publishes (it is what
//! `Overlay::servers_url` answers from). The other three are named here and
//! nothing publishes them yet, which is a *stated* pending state and not a
//! pretend one: a section whose link is absent says so on screen, in the same
//! words the shard list uses when nobody has published one.

use serde::Deserialize;

/// The most bytes a title manifest may be. Generous against a document that
/// is a handful of URLs, and bounded because it arrives over a network.
///
/// Written out rather than as `16 * 1024`: `ci/knob_registry.mjs` pins the
/// number this ships against the row in `DECISIONS.md` §open, and it reads a
/// literal rather than evaluating an expression.
pub const MAX_MANIFEST_BYTES: usize = 16384;

/// The most bytes any one URL in it may be — the same cap the shard list
/// puts on its own (`shardlist::MAX_URL_BYTES`), because they are the same
/// kind of value going to the same kind of place.
pub use crate::shardlist::MAX_URL_BYTES;

/// One section of the launcher's own site, as this document names it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Section {
    /// Where it lives. Absent is normal and is not an error.
    #[serde(default)]
    pub url: Option<String>,
    /// What to call it, if the platform wants to override our label. The
    /// reference renames its own entries over time (`RUST+` did not always
    /// exist); a name in the document costs nothing and means a rename is not
    /// a client release.
    #[serde(default)]
    pub label: Option<String>,
}

impl Section {
    /// The URL, if there is one and it is usable. A section whose link is
    /// junk reads as a section with no link — which is a state the screen
    /// already draws honestly — rather than as a button that opens nothing.
    pub fn link(&self) -> Option<&str> {
        let u = self.url.as_deref()?;
        check_url(u).ok()?;
        Some(u)
    }
}

/// What the launcher's manifest says about this title.
///
/// Every field optional, because every one of them is a service that may not
/// be stood up yet, and a manifest that must name all four to parse is a
/// manifest that breaks the game the day one is taken down for an hour.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    #[serde(default)]
    pub servers: Section,
    #[serde(default)]
    pub news: Section,
    #[serde(default)]
    pub store: Section,
    #[serde(default)]
    pub workshop: Section,
}

/// Parse a fetched manifest.
///
/// Never partially trusted: a document that is not an object at all is an
/// error, and a document that is one keeps whichever sections parse. The
/// error strings are written for a reader, the way `shardlist::parse`'s are,
/// because they end up on a menu in front of a player.
pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "title manifest: over {MAX_MANIFEST_BYTES} bytes - refused rather than truncated"
        ));
    }
    // **It has to be an OBJECT, and serde will not insist on that for us.**
    // A struct whose every field is `#[serde(default)]` also deserializes
    // from a JSON *sequence* — positionally — so `[]` came back as a
    // perfectly good manifest naming nothing. That is the worst possible
    // reading of a document that is plainly not ours: whatever the host
    // actually served (a list, an error page shaped as JSON) would present as
    // "the platform has published nothing yet", and the screen would say so
    // in a sentence that is a lie. Caught by this module's own test.
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err("title manifest: the document is not a JSON object".into());
    }
    serde_json::from_slice::<Manifest>(bytes)
        .map_err(|e| format!("title manifest: not readable JSON ({e})"))
}

/// Is this something we are willing to hand to the launcher to open?
///
/// **`https` only, and that is not paranoia about the transport.** The verb
/// on the other end is "open this on the player's desktop" — the launcher's
/// `open_url`, which reaches the real browser — so a URL from a document is
/// an instruction to the player's machine. A scheme allowlist is the cheapest
/// complete answer to `file://`, `javascript:` and everything else somebody
/// might put in a manifest the game did not write.
pub fn check_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("empty url".into());
    }
    if url.len() > MAX_URL_BYTES {
        return Err(format!("url over {MAX_URL_BYTES} bytes"));
    }
    if !url.starts_with("https://") {
        return Err("url is not https".into());
    }
    // A host has to be there. `https://` alone parses as a prefix and would
    // otherwise sail through the check above.
    let rest = &url["https://".len()..];
    if rest.is_empty() || rest.starts_with('/') {
        return Err("url names no host".into());
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("url contains whitespace or control characters".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_with_one_section_keeps_the_others_empty() {
        let m = parse(br#"{"news":{"url":"https://scry.works/gates/news"}}"#).unwrap();
        assert_eq!(m.news.link(), Some("https://scry.works/gates/news"));
        assert_eq!(m.store.link(), None);
        assert_eq!(m.workshop.link(), None);
    }

    #[test]
    fn an_unknown_key_rides_rather_than_refusing_the_document() {
        // The platform will grow this file and an older game must keep
        // working against a newer manifest.
        let m = parse(br#"{"news":{"url":"https://a.example/n"},"clans":{"url":"x"}}"#).unwrap();
        assert!(m.news.link().is_some());
    }

    #[test]
    fn a_junk_link_costs_its_own_section_and_nothing_else() {
        let m = parse(
            br#"{"news":{"url":"javascript:alert(1)"},
                 "store":{"url":"https://scry.works/gates/store"}}"#,
        )
        .unwrap();
        assert_eq!(m.news.link(), None, "a non-https link must not be opened");
        assert!(m.store.link().is_some(), "and must not cost its neighbour");
    }

    #[test]
    fn the_scheme_allowlist_is_the_whole_point() {
        // The verb on the other end reaches the player's desktop.
        for bad in [
            "",
            "http://scry.works/x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "scry://join/gates/h:1",
            "https://",
            "https:///nohost",
            "https://a.example/ has a space",
        ] {
            assert!(check_url(bad).is_err(), "{bad} must be refused");
        }
        assert!(check_url("https://scry.works/gates/news").is_ok());
    }

    #[test]
    fn an_oversize_document_is_refused_rather_than_truncated() {
        let big = vec![b'x'; MAX_MANIFEST_BYTES + 1];
        assert!(parse(&big).unwrap_err().contains("refused"));
    }

    #[test]
    fn a_document_that_is_not_an_object_is_an_error() {
        // `[]` is the one that mattered: serde reads a sequence into a struct
        // whose fields all default, so this parsed CLEANLY as a manifest
        // naming nothing, and the screen would have reported a host serving
        // the wrong document as a platform that had published nothing.
        for bad in [
            &b"[]"[..],
            b"[{}]",
            b"\"a string\"",
            b"42",
            b"null",
            b"not json",
            b"",
        ] {
            assert!(
                parse(bad).is_err(),
                "{:?} must not read as an empty manifest",
                String::from_utf8_lossy(bad)
            );
        }
        // ...but an empty object is a perfectly good "nothing published yet",
        // and leading whitespace is not a defect.
        assert_eq!(parse(b"{}").unwrap(), Manifest::default());
        assert_eq!(parse(b"  \n {} ").unwrap(), Manifest::default());
    }
}
