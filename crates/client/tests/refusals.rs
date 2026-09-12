//! **What a refused player is told, held against the codes the protocol
//! actually declares.** Code tier: no `render` feature, no GPU, no socket.
//!
//! This gate exists because of a defect that shipped, and the defect is worth
//! stating precisely because the shape recurs. On 2026-09-10 the browser page
//! branched on `String(e).includes("auth")` to decide whether a refusal meant
//! *this shard needs an account* — against a sentence reading *"this shard
//! needs a signed identity — sign in through the elo launcher"*, which
//! contains no `auth` anywhere. The branch was dead on arrival, so a player in
//! a tab was handed the desktop sentence and told to sign in through a
//! launcher that cannot exist in a tab. `findings/web-build-20260909.md` §5
//! names that exact misdirection as the thing not to get wrong, and the slice
//! that quoted it shipped it.
//!
//! **The lesson is not "test the substring".** Prose-matching IS the defect:
//! any gate that pinned the sentence would have gone green the moment somebody
//! reworded either side, and would have taught the next author that matching
//! on prose is a supported thing to do. So the fix carried the shard's own
//! `REFUSE_*` code out of the handshake (`net::handshake::JoinError`), and
//! what is gated here is the sentence TABLE — exhaustively, against the set of
//! codes `protocol` declares, derived by reading that file rather than by
//! listing them here.
//!
//! Deriving matters: `CLAUDE.md` records a hand-kept mirror of another crate's
//! surface going stale twice (the `props.js` count, the destructive-ring verb
//! list), and a refusal code added to `protocol` with no sentence on one of
//! the two platforms is precisely that failure — it would land in front of a
//! player as `refused: code 7`.

use std::path::Path;

use client::refusal_sentence;

/// Codes whose two sentences are allowed to differ, and why.
///
/// **Deliberately a list rather than a rule.** The desktop and browser tables
/// are the same table with overrides, so a divergence is a claim that a player
/// on one platform cannot do what the other is being told to do — and that
/// claim should cost somebody an entry here and a sentence of argument, the
/// way `tests/sound.rs` makes `pop_chat`'s exemption cost one.
const MAY_DIFFER: &[(&str, &str)] = &[(
    "REFUSE_AUTH",
    "the shared sentence says `sign in through the elo launcher`. A page has no local \
     launcher and never will — the launcher is a desktop process reached over a local \
     socket — so on the browser that sentence names an act the reader cannot perform. \
     The browser sentence points at the door a page DOES have: a wallet extension, \
     signing the same SIWE message through `elo::sign_siwe_web`.",
)];

/// Every `pub const REFUSE_<NAME>: u8 = <n>;` the protocol declares, read out
/// of its source.
///
/// **Read, not listed.** The whole value of this gate is that a code added to
/// `protocol` tomorrow is covered by it today; a copy of the list here would
/// be one more hand-kept mirror to go stale.
fn declared_codes() -> Vec<(String, u8)> {
    let path = Path::new("../protocol/src/lib.rs");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const REFUSE_") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(": u8 = ") else {
            continue;
        };
        let Some(value) = value.strip_suffix(';') else {
            continue;
        };
        let code: u8 = value
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("REFUSE_{name} has a value this gate cannot read: {value}"));
        out.push((format!("REFUSE_{name}"), code));
    }
    assert!(
        out.len() >= 4,
        "found only {} REFUSE_* constants in protocol — this gate reads them out of the \
         source, so a change to how they are written blinds it. Fix the scrape.",
        out.len()
    );
    out
}

/// **Every declared code says something on both platforms.**
///
/// Red under adding a `REFUSE_*` constant to `protocol` without a sentence:
/// `refuse_text` returns `None`, `refusal_sentence` falls through to
/// `refused: code N`, and this names the constant that has no words.
#[test]
fn every_refusal_code_has_a_sentence_on_both_platforms() {
    for (name, code) in declared_codes() {
        for (platform, launcher) in [("desktop", true), ("browser", false)] {
            let said = refusal_sentence(code, launcher);
            assert!(
                !said.contains(&format!("code {code}")),
                "{name} ({code}) has no {platform} sentence — a refused player would read \
                 {said:?}. Add it to `protocol::refuse_text`, or to the browser overrides \
                 in `net::handshake::refusal_sentence` if the shared wording does not fit \
                 a tab."
            );
            assert!(
                said.starts_with("refused: ") && said.len() > "refused: ".len() + 8,
                "{name}'s {platform} sentence is not one: {said:?}"
            );
        }
    }
}

/// **No sentence a browser player reads may send them to a launcher.**
///
/// This is the defect, stated as a law rather than as a string. It is red
/// today under reverting the `REFUSE_AUTH` override — which is how it was
/// proven, since the sentence it refuses is the one that shipped.
///
/// `elo` and `wallet` are deliberately NOT refused: buying a copy on elo is
/// something a browser player can do, and a wallet is how a page proves an
/// identity — since 2026-09-10 that is built (`elo::sign_siwe_web`), so a
/// sentence naming one points at a real door. The word that names an
/// impossible act is `launcher`.
#[test]
fn no_browser_refusal_points_a_player_at_a_launcher() {
    for (name, code) in declared_codes() {
        let said = refusal_sentence(code, false);
        assert!(
            !said.to_ascii_lowercase().contains("launcher"),
            "{name}'s browser sentence tells a player in a tab to use the elo launcher: \
             {said:?}. There is no launcher in a page — a page's signer is a wallet \
             extension — so this names an act the reader cannot perform: the exact \
             misdirection findings/web-build-20260909.md §5 is about."
        );
    }
}

/// **The two tables differ only where somebody argued they should.**
///
/// Red both ways on purpose: an unlisted divergence names the code, and a
/// listed one that has converged names the stale entry — because an exemption
/// nobody re-reads is how a list stops meaning anything.
#[test]
fn the_two_sentence_tables_diverge_only_where_argued() {
    for (name, code) in declared_codes() {
        let desktop = refusal_sentence(code, true);
        let browser = refusal_sentence(code, false);
        let argued = MAY_DIFFER.iter().find(|(n, _)| *n == name);
        match (desktop == browser, argued) {
            (false, None) => panic!(
                "{name} says different things to a desktop player and a browser one, and \
                 nothing says why.\n  desktop: {desktop:?}\n  browser: {browser:?}\n\
                 Add it to MAY_DIFFER with the reason, or make the sentences agree."
            ),
            (true, Some((_, why))) => panic!(
                "{name} is listed in MAY_DIFFER but both platforms now say {desktop:?}. \
                 The exemption's reason was: {why}\nRemove the entry."
            ),
            _ => {}
        }
    }
}

/// A code no build has words for still shows the NUMBER rather than swallowing
/// it — a shard refusing for a reason we cannot name is a build skew, and the
/// number is the only thing that identifies it.
#[test]
fn an_unknown_code_is_reported_as_a_number() {
    let said = refusal_sentence(203, true);
    assert!(
        said.contains("203"),
        "an unknown refusal code must reach the player as a number: {said:?}"
    );
}

/// **The page does not decide what a refusal MEANS by reading it.**
///
/// A source scan over `crates/client-web/web/app.js` (the page's script, a
/// file because a `script-src 'self'` CSP refuses an inline module), in
/// `tests/tls_callsite.rs`'s shape and for its reason: the defect is a call
/// site, not a value, so the instrument is a grep for the call site. The page
/// is not Rust and no compiler will ever look at it, which makes it the one
/// place in this repo where a dead branch can live indefinitely.
///
/// ⚠ **Scoped to the join's `catch` block, and that scope is the gate's
/// correctness rather than a convenience.** The first draft banned string
/// matching anywhere in the file and immediately failed on
/// `k.startsWith("Key")` — the page filtering DOM key codes, which is a
/// browser API contract and has nothing to do with our sentences. A gate that
/// reports correct code as the defect is a gate the next author deletes, and
/// it would have taken this one with it. What is forbidden is narrower and
/// exact: reading the error to work out what happened.
#[test]
fn the_browser_page_reads_no_refusal_to_decide_what_it_was() {
    let path = Path::new("../client-web/web/app.js");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // Comments may quote these freely — every one of them is *about* this
    // rule — so whole-line comments go first, exactly as `code_of` does it.
    let code: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let at = code.find("catch (e) {").unwrap_or_else(|| {
        panic!(
            "app.js no longer catches the join's failure where this gate looks - it is \
             watching a ghost. Re-point it at whatever handles a refused join."
        )
    });
    let bytes = code.as_bytes();
    let open = at + code[at..].find('{').expect("the catch opens a block");
    let mut depth = 0usize;
    let mut close = open;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let handler = &code[open..close];
    assert!(close > open, "the catch block is unterminated");

    // ⚠ **A blocklist of spellings, not a proof, and saying so is the point.**
    // The first version of this list missed `/auth/.test(e)` — the obvious way
    // round a substring ban — which a mutant found. `.test(` and `.exec(` are
    // there now, and something will get past it eventually; what makes it
    // worth having anyway is that the mechanism it protects (a `REFUSE_*` code
    // on `e.code`) makes text-matching the harder thing to reach for, and this
    // catches the easy regression. A gate that overstated itself here would be
    // worse than one that names its limit.
    for banned in [
        ".includes(",
        ".indexOf(",
        ".match(",
        ".startsWith(",
        ".endsWith(",
        ".search(",
        ".test(",
        ".exec(",
        "RegExp",
    ] {
        assert!(
            !handler.contains(banned),
            "app.js's join handler calls {banned} - it is testing a message's TEXT to \
             decide what happened. That is the defect this file exists for: the sentence it \
             matched was reworded out from under it and the branch died silently, so a \
             player in a tab was told to open a launcher that cannot exist in one. \
             `Gates.join` throws an error carrying the shard's own `REFUSE_*` code as \
             `e.code`; branch on that.\n\nThe handler:\n{handler}"
        );
    }
}
