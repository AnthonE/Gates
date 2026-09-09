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

/// Turn a refusal frame into the sentence a player sees, or `None` if this
/// frame is not one. Shared by both steps below, because a shard may refuse
/// at either.
fn refusal(frame: &[u8]) -> Option<String> {
    match peek_kind(frame) {
        Ok(KIND_REFUSE) => {
            let r = decode_refuse(frame).ok()?;
            Some(match protocol::refuse_text(r.code) {
                Some(why) => format!("refused: {why}"),
                None => format!("refused: code {}", r.code),
            })
        }
        _ => None,
    }
}

/// Step 1 — the opening frame. Writes into `buf`, answers its length.
pub fn hello(buf: &mut [u8]) -> Result<usize, String> {
    encode_hello(
        &Hello {
            proto_ver: PROTO_VER,
            ver: protocol::version::VER,
            build: protocol::version::BUILD,
        },
        buf,
    )
    .map_err(|e| format!("encode hello: {e:?}"))
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
) -> Result<usize, String> {
    match peek_kind(frame) {
        Ok(protocol::KIND_CHALLENGE) => {}
        Ok(KIND_REFUSE) => return Err(refusal(frame).unwrap_or_else(|| "refused".into())),
        other => return Err(format!("expected a challenge, got {other:?}")),
    }
    let challenge = protocol::decode_challenge(frame).map_err(|e| format!("challenge: {e:?}"))?;

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
        let nonce = core::str::from_utf8(&hex).map_err(|_| "nonce hex".to_string())?;
        match sign(domain, nonce, challenge.issued_at) {
            Some(signature) => protocol::Auth { address, signature },
            None => protocol::Auth::default(),
        }
    };
    protocol::encode_auth(&auth, buf).map_err(|e| format!("encode auth: {e:?}"))
}

/// Step 3 — the shard's answer: a welcome, or the reason it said no.
pub fn welcome_from(frame: &[u8]) -> Result<Welcome, String> {
    match peek_kind(frame) {
        Ok(KIND_WELCOME) => decode_welcome(frame).map_err(|e| format!("welcome: {e:?}")),
        Ok(KIND_REFUSE) => Err(refusal(frame).unwrap_or_else(|| "refused".into())),
        other => Err(format!("unexpected handshake reply: {other:?}")),
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
        assert!(at_auth.starts_with("refused"), "{at_auth}");
        assert_eq!(
            at_auth, at_welcome,
            "one refusal reads the same at either step"
        );
        // The text is the shard's stated reason, not a code, when we have one.
        assert!(
            protocol::refuse_text(protocol::REFUSE_VERSION)
                .is_some_and(|why| at_auth.contains(why)),
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
        assert!(err.contains("unexpected handshake reply"), "{err}");
    }
}
