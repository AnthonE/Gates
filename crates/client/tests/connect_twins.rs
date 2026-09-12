//! **The two `Session::connect`s, held to being the same join.** Code tier:
//! no `render` feature, no GPU, no socket, no browser.
//!
//! `src/lib.rs` carries two whole connect sequences, one per target. Each
//! compiles only on its own target, so **no build, no clippy run and no test
//! in this repo ever sees both at once** — and `cargo test --workspace` never
//! compiles the wasm one at all, because `client-web` is an empty crate
//! natively. The single gated wasm compile is `ci/gates.sh`'s browser-client
//! line, and it compiles that arm alone.
//!
//! ⚠ **`lib.rs` used to say a gate was therefore impossible.** It is not: a
//! source scan does not compile anything, so it can read both arms on any
//! target. That false reason is why nobody wrote this for a day — a
//! plausible-sounding impossibility claim is more expensive than an
//! unmentioned gap, because it stops the next person looking. The claim is
//! corrected where it was made.
//!
//! **What rustc already covers, this deliberately does not.** `Session` is one
//! non-cfg'd struct whose `wire` field is the cfg-selected type, so a missing
//! or extra field in either arm is `E0063` on that arm's own build. A second,
//! weaker copy of a compiler check is a gate somebody deletes later after
//! finding it proves nothing. What is left over — and what is gated here — is
//! everything the compiler cannot compare because it only ever sees one side:
//! the VALUES those fields are given, the lane depths, the frame ceilings, and
//! the handshake sequence itself.
//!
//! Proven red under four mutations, each applied to the wasm arm alone and
//! each of which passes every build, clippy run, test and gate in this repo:
//!
//! | mutation | caught by |
//! |---|---|
//! | `closed: false` → `closed: true` | the struct-literal comparison |
//! | the event lane `64` → `8` | the channel-depth comparison |
//! | `FrameReader::<MAX_STREAM_MSG_BYTES>` → `MAX_EVENT_MSG_BYTES` | the ceiling sequence |
//! | dropping the auth step (`auth_for`, or either half of it) | the handshake sequence |
//!
//! The third is the one worth naming: it makes the browser handshake read at
//! 320 bytes where the desktop reads at 128 — a lane ceiling silently widened
//! on one platform, which is a difference in what the two clients will ACCEPT
//! off the wire.

use std::path::Path;

/// The file with whole-line comments removed, `tests/tls_callsite.rs`'s
/// `code_of` verbatim and for its reason: comments may name any of these
/// freely — every one of them is *about* this rule — and the rule is about
/// code. Whole-line only is sufficient here and checked: every comment inside
/// both blocks is its own line, and stripping trailing text would have to
/// understand that `format!("https://{server}")` is not one.
fn code_of(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text between the braces of the first block whose header matches
/// `header`, brace-matched rather than line-counted.
fn block_after(code: &str, header: &str) -> String {
    let at = code.find(header).unwrap_or_else(|| {
        panic!("src/lib.rs no longer contains `{header}` - this gate is watching a ghost")
    });
    let open = code[at..]
        .find('{')
        .expect("a block header is followed by a brace")
        + at;
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    for (i, b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return code[open + 1..i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("`{header}`'s block is unterminated");
}

const NATIVE: &str = "#[cfg(feature = \"native\")]\nimpl Session {";
const WEB: &str = "#[cfg(target_arch = \"wasm32\")]\nimpl Session {";

fn twins() -> (String, String) {
    let code = code_of(Path::new("src/lib.rs"));
    (block_after(&code, NATIVE), block_after(&code, WEB))
}

/// Every occurrence of `needle` followed by an identifier, in source order.
fn names_after(block: &str, needle: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(at) = rest.find(needle) {
        let tail = &rest[at + needle.len()..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
        rest = &rest[at + needle.len()..];
    }
    out
}

/// Consecutive duplicates collapsed, order kept — so "read the stream ceiling,
/// then the event ceiling" is one fact whether the stream is read once or
/// twice. The desktop path reads two handshake frames and the browser path
/// reads them through one buffered reader; that difference is real, forced,
/// and not what this gate is about.
fn squash(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in v {
        if out.last() != Some(&x) {
            out.push(x);
        }
    }
    out
}

/// `auth_for` written out as the two halves it is composed of.
///
/// **The one asymmetry this gate tolerates, and it is forced.** A browser
/// signature is a Promise and the shard's nonce only exists mid-handshake, so
/// it cannot be pre-signed — the page must decode the challenge, `await` a
/// wallet, and then encode. `net::handshake::auth_for` is the synchronous
/// composition of exactly those two functions, for a desktop signer that
/// answers without awaiting.
///
/// So this expansion is not a licence for the two paths to differ: it says
/// they drive the same rules in the same order and lets one of them stop in
/// the middle. That the two assemblies produce the SAME BYTES is a separate,
/// exact claim, and it is gated where the functions live —
/// `net::handshake::tests::the_two_step_path_and_the_composition_agree` builds
/// an auth frame both ways from one challenge and compares them.
fn expand(steps: Vec<String>) -> Vec<String> {
    steps
        .into_iter()
        .flat_map(|s| match s.as_str() {
            "auth_for" => vec!["proof_wanted".to_string(), "auth_frame".to_string()],
            _ => vec![s],
        })
        .collect()
}

/// **The same rules, in the same order.**
///
/// `net::handshake` is the pure half of the join — the version check, the SIWE
/// domain binding, the refusal decode — extracted precisely so both transports
/// drive one copy of it. That is only true while both actually call all of it.
#[test]
fn both_twins_drive_the_same_handshake_in_the_same_order() {
    let (native, web) = twins();
    let a = expand(names_after(&native, "net::handshake::"));
    let b = expand(names_after(&web, "net::handshake::"));
    assert_eq!(
        a,
        vec!["hello", "proof_wanted", "auth_frame", "welcome_from"],
        "the desktop connect no longer drives the handshake this gate knows about"
    );
    assert_eq!(
        a, b,
        "the two connects drive different handshake steps, or drive them in a different \
         order.\n  desktop: {a:?}\n  browser: {b:?}\nEach target compiles only its own arm, \
         so nothing else in this repo can see this. (`auth_for` counts as \
         `proof_wanted` then `auth_frame` — see `expand`.)"
    );
}

/// **The same lane depths.**
///
/// Both are wall-4 caps with stated policies — the event lane buffers what the
/// shard has already sent, and the action lane is bounded at the sim's own
/// burst depth so a client cannot hold more in flight than the server will
/// take. A platform that quietly runs a shallower one drops a shard's last
/// words on a kick, on that platform only.
#[test]
fn both_twins_open_their_lanes_at_the_same_depth() {
    let (native, web) = twins();
    let needle = "mpsc::channel::<Vec<u8>>(";
    let depths = |b: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = b;
        while let Some(at) = rest.find(needle) {
            let tail = &rest[at + needle.len()..];
            out.push(
                tail[..tail.find(')').expect("a call is closed")]
                    .trim()
                    .to_string(),
            );
            rest = tail;
        }
        out
    };
    let a = depths(&native);
    let b = depths(&web);
    assert_eq!(
        a.len(),
        2,
        "the desktop connect opens {} lanes, not 2",
        a.len()
    );
    assert_eq!(
        a, b,
        "the two connects open their lanes at different depths.\n  desktop: {a:?}\n  \
         browser: {b:?}"
    );
}

/// **The same frame ceilings, in the same order.**
///
/// The handshake lane is `MAX_STREAM_MSG_BYTES` and the event lane is
/// `MAX_EVENT_MSG_BYTES`; the ceiling belongs to the lane, not to the
/// transport. Widening one on one platform changes what that client will
/// ACCEPT off the wire — a difference in the parsing surface, not in a
/// buffer size.
#[test]
fn both_twins_read_each_lane_at_the_same_ceiling() {
    let (native, web) = twins();
    let a = squash(names_after(&native, "MAX_"));
    let b = squash(names_after(&web, "MAX_"));
    assert_eq!(
        a,
        vec!["STREAM_MSG_BYTES", "EVENT_MSG_BYTES"],
        "the desktop connect no longer reads the handshake at the stream ceiling and the \
         event lane at the event ceiling"
    );
    assert_eq!(
        a, b,
        "the two connects use different frame ceilings, or use them in a different order.\n  \
         desktop: {a:?}\n  browser: {b:?}"
    );
}

/// **The same session, field for field.**
///
/// rustc guarantees the field SET on either arm's own build; what it cannot
/// see is that the two arms give a field the same VALUE, because it never
/// compiles both. `closed: true` on one platform is a client that believes it
/// is disconnected the instant it joins, and every gate in this repo is green.
///
/// `wire` is exempt BY NAME — the way `tests/sound.rs` exempts `pop_chat` —
/// because it is the one field whose type is cfg-selected and whose whole
/// purpose is to differ. A second legitimate divergence has to be argued into
/// this list in the same commit rather than slipping past a rule.
#[test]
fn both_twins_build_the_same_session() {
    const EXEMPT: &[&str] = &["wire"];
    let (native, web) = twins();

    let fields = |b: &str| -> Vec<(String, String)> {
        let lit = block_after(b, "Ok(Self {");
        let mut out = Vec::new();
        let (mut depth, mut cur) = (0usize, String::new());
        for c in lit.chars() {
            match c {
                '(' | '[' | '{' | '<' => depth += 1,
                ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        out.push(cur);
        out.into_iter()
            .map(|f| f.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|f| !f.is_empty())
            .map(|f| match f.split_once(':') {
                // Shorthand (`welcome`) is its own value, and that is the
                // point: `wire,` on one side and `wire: X::new(c)` on the
                // other are different initialisers of the same field.
                None => (f.clone(), f),
                Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
            })
            .collect()
    };

    let a = fields(&native);
    let b = fields(&web);
    assert!(
        a.len() >= 11,
        "the desktop session literal has only {} fields - this gate expects the whole struct",
        a.len()
    );
    assert_eq!(
        a.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        b.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        "the two session literals name different fields, or name them in a different order"
    );
    for ((ka, va), (_, vb)) in a.iter().zip(b.iter()) {
        if EXEMPT.contains(&ka.as_str()) {
            continue;
        }
        assert_eq!(
            va, vb,
            "`{ka}` is built differently by the two connects.\n  desktop: {va}\n  browser: {vb}\n\
             Each target compiles one arm, so this difference is invisible to every other \
             gate. If it is deliberate, name the field in this test's EXEMPT list with the \
             argument for it."
        );
    }
}

/// **And the headers this gate keys on are still the ones in the file.**
///
/// Without this, renaming a cfg predicate empties every scan above and all
/// four tests pass by reading nothing — the failure `tls_callsite.rs` guards
/// against with its own "watching a ghost" assertion.
#[test]
fn the_two_twins_are_still_where_this_gate_looks() {
    let code = code_of(Path::new("src/lib.rs"));
    for header in [NATIVE, WEB] {
        assert_eq!(
            code.matches(header).count(),
            1,
            "`{header}` appears {} times in src/lib.rs - this gate reads the first",
            code.matches(header).count()
        );
    }
}
