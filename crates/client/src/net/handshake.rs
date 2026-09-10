//! The join handshake, with the transport taken out of it.
//!
//! Three pure functions that read and write byte frames in a caller's
//! buffer. **No socket, no `async`, no transport type, and deliberately no
//! trait** — the sequencing (send this, then read that) stays in whichever
//! `connect` is driving, because that is the part each transport does
//! differently and the part that cannot be shared anyway.
//!
//! **Why this is not an async trait**, which was the obvious first design:
//! the native `connect` runs under `tokio::spawn` (`render/menu.rs`), which
//! requires a `Send` future, and a web one must run under
//! `wasm_bindgen_futures::spawn_local`, which must NOT — `JsValue` is not
//! `Send`. An `async fn` in a trait cannot express "Send on one target, not
//! the other" without `trait_variant` or duplicated bounds, and a generic
//! caller cannot assume it either. Making the shared half *incapable* of I/O
//! sidesteps the question rather than answering it, and it buys a second
//! thing: every rule below is now reachable from a code-tier test with no
//! shard, no socket and no runtime.
//!
//! The measurement this comes from (`findings/web-build-20260909.md` §10.3):
//! the handshake was ~85 lines of which **six** touched the transport. A web
//! build that copied the other seventy-nine would be the THIRD copy — there
//! is already a second at `server/src/net.rs`, and ⚠ **that one has diverged
//! on purpose**: its `sign` takes the composed message, ours takes three
//! inert values, and the difference is the fix documented below. Do not
//! "unify" them.

use protocol::{
    decode_refuse, decode_welcome, encode_hello, peek_kind, Hello, Welcome, KIND_REFUSE,
    KIND_WELCOME, PROTO_VER,
};

/// Why a join did not happen.
///
/// **The refusal carries its CODE, and that is the whole point of this type.**
/// It used to be flattened into a `String` at the moment it was decoded, which
/// left every caller with only prose to work with — and a caller that has only
/// prose has to match on prose. That is not hypothetical: the browser page
/// shipped on 2026-09-10 branching on `String(e).includes("auth")`, against a
/// sentence reading *"this shard needs a signed identity — sign in through the
/// elo launcher"*, which contains no `auth` anywhere. The branch was dead, so a
/// player in a tab was told to open a launcher that cannot exist in a tab —
/// exactly the misdirection `findings/web-build-20260909.md` §5 says must not
/// be got wrong, shipped in the same slice that quoted it.
///
/// Prose-matching is the defect, so the fix is not a better substring. A code
/// is a number the shard actually sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinError {
    /// The shard said no, with the `REFUSE_*` code it sent.
    Refused(u8),
    /// Everything else — a transport failure, a malformed frame, an encoder
    /// that refused its own buffer. No code, because none was received.
    Failed(String),
}

/// Whether a player on THIS build could reach an elo launcher at all.
///
/// A property of the platform, not of whether one happens to be running: a
/// desktop player with no launcher installed can install one, and a player in
/// a tab cannot. That distinction is the only axis on which the two sentence
/// tables below differ, and it is why this is a `cfg!` rather than a runtime
/// probe — `Display` has nothing to probe with, and the answer never changes
/// for a given binary.
pub const LAUNCHER_REACHABLE: bool = cfg!(not(target_arch = "wasm32"));

/// The sentence a player sees for a refusal code.
///
/// `protocol::refuse_text` is the shared table and stays the authority; this
/// wraps it and overrides only where the shared wording assumes a door the
/// reader does not have. `crates/client/tests/refusals.rs` holds it to that:
/// every `REFUSE_*` constant the protocol declares must have a sentence on
/// both sides, no browser sentence may mention a launcher, and the two tables
/// may differ only for codes listed there with a reason.
pub fn refusal_sentence(code: u8, launcher_reachable: bool) -> String {
    // The overrides. There is exactly one today, so this is an `if` rather
    // than a `match` with a lone arm (clippy's `single_match`, and it is
    // right — a one-armed match here would be a shape claiming to be a table).
    // The second one turns it into a `match` and that is the moment to write
    // the table.
    //
    // `REFUSE_AUTH`'s shared wording is "sign in through the elo launcher".
    // There is no launcher in a tab, and no browser wallet either
    // (`elo::sign_siwe`'s wasm arm returns `None` by construction), so on this
    // platform that sentence names an act the reader cannot perform. The
    // honest one states the shard's requirement and points somewhere real.
    if !launcher_reachable && code == protocol::REFUSE_AUTH {
        return "refused: this shard needs an account, and a browser cannot sign in yet \
                - try a shard that takes guests"
            .to_string();
    }
    match protocol::refuse_text(code) {
        Some(why) => format!("refused: {why}"),
        // A code this build has no sentence for. Still shows the number rather
        // than swallowing it, because a shard refusing for a reason we cannot
        // name is a build skew and the number is what identifies it.
        None => format!("refused: code {code}"),
    }
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::Refused(code) => f.write_str(&refusal_sentence(*code, LAUNCHER_REACHABLE)),
            JoinError::Failed(why) => f.write_str(why),
        }
    }
}

impl From<String> for JoinError {
    fn from(why: String) -> Self {
        JoinError::Failed(why)
    }
}

/// The `REFUSE_*` code in a refusal frame, or `None` if this frame is not one.
/// Shared by both steps below, because a shard may refuse at either.
fn refusal(frame: &[u8]) -> Option<u8> {
    match peek_kind(frame) {
        Ok(KIND_REFUSE) => Some(decode_refuse(frame).ok()?.code),
        _ => None,
    }
}

/// Step 1 — the opening frame. Writes into `buf`, answers its length.
pub fn hello(buf: &mut [u8]) -> Result<usize, JoinError> {
    encode_hello(
        &Hello {
            proto_ver: PROTO_VER,
            ver: protocol::version::VER,
            build: protocol::version::BUILD,
        },
        buf,
    )
    .map_err(|e| JoinError::Failed(format!("encode hello: {e:?}")))
}

/// Step 2 — read the shard's challenge, answer with the auth frame.
///
/// `server` is the `host:port` **we dialled**, and the domain is taken from
/// it rather than from anything the shard said. That is the whole of SIWE's
/// domain binding: a signature collected by one shard is not valid at another
/// because the two messages differ, and letting the server name itself would
/// hand that away.
pub fn auth_for(
    frame: &[u8],
    server: &str,
    address: protocol::Address,
    // `(domain, nonce hex, issued_at)` — the three inert values the launcher
    // needs, never a message this process composed. See below for why that
    // inversion is the whole fix.
    sign: impl FnOnce(&str, &str, u64) -> Option<protocol::Signature>,
    buf: &mut [u8],
) -> Result<usize, JoinError> {
    match peek_kind(frame) {
        Ok(protocol::KIND_CHALLENGE) => {}
        // A frame that says REFUSE but does not decode is still a refusal, and
        // its code is the one thing we could not read — so it reports as a
        // failure with the reason, never as a refusal with an invented code.
        Ok(KIND_REFUSE) => {
            return Err(match refusal(frame) {
                Some(code) => JoinError::Refused(code),
                None => JoinError::Failed("refused, but the frame did not decode".into()),
            })
        }
        other => {
            return Err(JoinError::Failed(format!(
                "expected a challenge, got {other:?}"
            )))
        }
    }
    let challenge = protocol::decode_challenge(frame)
        .map_err(|e| JoinError::Failed(format!("challenge: {e:?}")))?;

    let domain = server.rsplit_once(':').map(|(h, _)| h).unwrap_or(server);
    let auth = if address.is_guest() {
        protocol::Auth::default()
    } else {
        // **This process does not compose the message, and that is the fix.**
        // It used to build the SIWE text here and hand it to the launcher's
        // `sign`, which refused every one: `sign` classifies a message by its
        // first line (`elo <family>`) and EIP-4361 begins with a domain. The
        // refusal became `None`, `None` means "connect as a guest", and a
        // `require_auth` shard answered REFUSE_AUTH — so every login failed as
        // if the signature were wrong, when the message was one the launcher
        // would never sign.
        //
        // `prove` is the verb: the LAUNCHER writes every word, which is what
        // stops a game smuggling a sentence into a signature, and it needs no
        // consent prompt for the same reason. We hand over three inert values
        // and the shard rebuilds the text from the ones it already knows
        // (`protocol::siwe_message`).
        let mut hex = [0u8; protocol::NONCE_BYTES * 2];
        for (i, b) in challenge.nonce.iter().enumerate() {
            const H: &[u8; 16] = b"0123456789abcdef";
            hex[i * 2] = H[(b >> 4) as usize];
            hex[i * 2 + 1] = H[(b & 0xf) as usize];
        }
        let nonce =
            core::str::from_utf8(&hex).map_err(|_| JoinError::Failed("nonce hex".to_string()))?;
        match sign(domain, nonce, challenge.issued_at) {
            Some(signature) => protocol::Auth { address, signature },
            None => protocol::Auth::default(),
        }
    };
    protocol::encode_auth(&auth, buf).map_err(|e| JoinError::Failed(format!("encode auth: {e:?}")))
}

/// Step 3 — the shard's answer: a welcome, or the reason it said no.
pub fn welcome_from(frame: &[u8]) -> Result<Welcome, JoinError> {
    match peek_kind(frame) {
        Ok(KIND_WELCOME) => {
            decode_welcome(frame).map_err(|e| JoinError::Failed(format!("welcome: {e:?}")))
        }
        Ok(KIND_REFUSE) => Err(match refusal(frame) {
            Some(code) => JoinError::Refused(code),
            None => JoinError::Failed("refused, but the frame did not decode".into()),
        }),
        other => Err(JoinError::Failed(format!(
            "unexpected handshake reply: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{
        encode_challenge, encode_refuse, encode_welcome, Address, Challenge, Refuse,
        MAX_STREAM_MSG_BYTES,
    };

    fn challenge_frame(nonce: [u8; protocol::NONCE_BYTES], issued_at: u64) -> Vec<u8> {
        let mut b = [0u8; MAX_STREAM_MSG_BYTES];
        let n = encode_challenge(&Challenge { nonce, issued_at }, &mut b).expect("encode");
        b[..n].to_vec()
    }

    /// A non-guest address. Any non-zero byte does it — `is_guest` is
    /// all-zeroes — and the value is otherwise arbitrary.
    fn someone() -> Address {
        let mut a = [0u8; protocol::ADDRESS_BYTES];
        a[0] = 0xAB;
        Address(a)
    }

    #[test]
    fn the_opening_frame_is_a_hello_at_this_builds_protocol_version() {
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let n = hello(&mut buf).expect("hello encodes");
        assert!(n > 0);
        assert_eq!(peek_kind(&buf[..n]), Ok(protocol::KIND_HELLO));
        let h = protocol::decode_hello(&buf[..n]).expect("round trips");
        // Wall 6: the wire version is the one this build was compiled at, so
        // a stale client is refused at the door rather than quietly misread.
        assert_eq!(h.proto_ver, PROTO_VER);
    }

    /// **The load-bearing rule in this file, and until now it needed a shard
    /// to exercise.** SIWE's domain binding is that the signature covers the
    /// host we DIALLED — a signature collected by one shard must not be
    /// valid at another. If the domain ever came from the server's own
    /// message instead, every shard could harvest signatures for every other,
    /// and nothing about the connection would look wrong.
    #[test]
    fn the_signed_domain_is_the_host_we_dialled_with_the_port_stripped() {
        let frame = challenge_frame([7u8; protocol::NONCE_BYTES], 1_700_000_000);
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let mut seen = None;
        let _ = auth_for(
            &frame,
            "game.elopros.com:4433",
            someone(),
            |domain, nonce, issued| {
                seen = Some((domain.to_string(), nonce.to_string(), issued));
                None
            },
            &mut buf,
        )
        .expect("encodes");
        let (domain, nonce, issued) = seen.expect("sign was asked");
        assert_eq!(
            domain, "game.elopros.com",
            "the port is not part of the domain"
        );
        assert_eq!(
            nonce,
            "07".repeat(protocol::NONCE_BYTES),
            "lowercase hex, byte for byte"
        );
        assert_eq!(issued, 1_700_000_000, "the shard's own clock, not ours");
    }

    /// A host with no port is still a domain — `--server` accepts one, and
    /// `rsplit_once(':')` must not eat a bare name.
    #[test]
    fn a_server_with_no_port_is_its_own_domain() {
        let frame = challenge_frame([0u8; protocol::NONCE_BYTES], 1);
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let mut seen = None;
        let _ = auth_for(
            &frame,
            "localhost",
            someone(),
            |d, _, _| {
                seen = Some(d.to_string());
                None
            },
            &mut buf,
        );
        assert_eq!(seen.as_deref(), Some("localhost"));
    }

    /// A declined prompt or an absent launcher answers `None`, and `None`
    /// means **connect as a guest** rather than fail the connection
    /// (`Session::connect`'s contract). A page has no local launcher at all,
    /// so this is the web build's ordinary path, not its error path.
    #[test]
    fn a_refused_signature_connects_as_a_guest_rather_than_failing() {
        let frame = challenge_frame([1u8; protocol::NONCE_BYTES], 9);
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let n =
            auth_for(&frame, "h:1", someone(), |_, _, _| None, &mut buf).expect("still encodes");
        let auth = protocol::decode_auth(&buf[..n]).expect("round trips");
        assert!(auth.address.is_guest(), "no signature means guest");
    }

    /// A guest never reaches the launcher at all — asking would be a consent
    /// prompt for a player who has not claimed an identity.
    #[test]
    fn a_guest_is_never_asked_to_sign() {
        let frame = challenge_frame([2u8; protocol::NONCE_BYTES], 9);
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let mut asked = false;
        let _ = auth_for(
            &frame,
            "h:1",
            Address::GUEST,
            |_, _, _| {
                asked = true;
                None
            },
            &mut buf,
        )
        .expect("encodes");
        assert!(!asked, "a guest address must not open a signing prompt");
    }

    /// Both steps can be answered with a refusal instead, and the player is
    /// owed the reason rather than a decode error.
    #[test]
    fn a_refusal_at_either_step_carries_its_reason() {
        let mut b = [0u8; MAX_STREAM_MSG_BYTES];
        let n = encode_refuse(
            &Refuse {
                code: protocol::REFUSE_VERSION,
            },
            &mut b,
        )
        .expect("encode");
        let frame = &b[..n];

        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let at_auth = auth_for(frame, "h:1", Address::GUEST, |_, _, _| None, &mut buf)
            .expect_err("a refusal is not an auth");
        let at_welcome = welcome_from(frame).expect_err("a refusal is not a welcome");
        // **The CODE, not the sentence.** This used to assert
        // `at_auth.starts_with("refused")` and that the shard's stated reason
        // appeared somewhere in the text — a test matching on prose, checking
        // a function whose whole job had become flattening a code into prose.
        // It passed happily while the browser page's own prose-match was dead.
        assert_eq!(
            at_auth,
            JoinError::Refused(protocol::REFUSE_VERSION),
            "a refusal must carry the code the shard sent"
        );
        assert_eq!(
            at_auth, at_welcome,
            "one refusal reads the same at either step"
        );
        // And it still reaches a player as that shard's stated reason.
        assert!(
            protocol::refuse_text(protocol::REFUSE_VERSION)
                .is_some_and(|why| at_auth.to_string().contains(why)),
            "{at_auth}"
        );
    }

    #[test]
    fn a_welcome_round_trips_and_anything_else_is_named_in_the_error() {
        let w = Welcome {
            seed: 20260731,
            player_id: 7,
            tick: 42,
            dev: false,
        };
        let mut b = [0u8; MAX_STREAM_MSG_BYTES];
        let n = encode_welcome(&w, &mut b).expect("encode");
        let got = welcome_from(&b[..n]).expect("decodes");
        assert_eq!(
            (got.seed, got.player_id, got.tick),
            (w.seed, w.player_id, w.tick)
        );

        // A challenge arriving where a welcome belongs is a protocol error,
        // and the message has to say what actually came so a report is useful.
        let stray = challenge_frame([0u8; protocol::NONCE_BYTES], 0);
        let err = welcome_from(&stray).expect_err("not a welcome");
        assert!(
            matches!(err, JoinError::Failed(_)),
            "a protocol error is not a refusal - there is no code to report: {err}"
        );
        assert!(
            err.to_string().contains("unexpected handshake reply"),
            "{err}"
        );
    }
}
