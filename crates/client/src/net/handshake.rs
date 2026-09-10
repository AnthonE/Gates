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
    // There is no launcher in a tab, so on this platform that sentence names
    // an act the reader cannot perform. The honest one states the shard's
    // requirement and points at the door a page actually has.
    //
    // ⚠ **This sentence changed on 2026-09-10 and the old one is now wrong,
    // not merely worse.** It read *"a browser cannot sign in yet - try a shard
    // that takes guests"*, which was true while `elo::sign_siwe`'s wasm arm
    // returned `None` by construction. `elo::sign_siwe_web` gives it a body:
    // a page with a wallet signs the same SIWE message the desktop client
    // does. Telling that player to go and find a different shard would send
    // them away from a door they can open.
    //
    // It still has to cover both ways a shard says `REFUSE_AUTH` — a guest on
    // a locked shard, and a signature that recovered somebody else — so it
    // names the requirement rather than guessing which happened.
    if !launcher_reachable && code == protocol::REFUSE_AUTH {
        return "refused: this shard needs a signed identity - connect a wallet in this \
                browser and join again"
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

/// What the shard is asking this client to prove: the three inert values,
/// decoded out of its challenge.
///
/// **Three values and never a message.** The launcher (desktop) or the wallet
/// (browser) composes the sentence; we hand over the parts. See [`auth_for`]
/// for the incident that made that inversion the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    /// The SIWE domain — the host we dialled with its port stripped.
    pub domain: String,
    /// The shard's nonce, raw. Rendered as lowercase hex by
    /// [`Proof::nonce_hex`] for a signer that wants text, and passed as bytes
    /// to [`protocol::siwe_message`] by [`Proof::message`] — one value, two
    /// renderings, rather than a hex string somebody has to parse back.
    pub nonce: [u8; protocol::NONCE_BYTES],
    /// Unix seconds, the shard's own `Issued At`.
    pub issued_at: u64,
}

impl Proof {
    /// The nonce as lowercase hex — what the elo launcher's `prove` takes.
    pub fn nonce_hex(&self) -> String {
        const H: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(protocol::NONCE_BYTES * 2);
        for b in self.nonce {
            out.push(H[(b >> 4) as usize] as char);
            out.push(H[(b & 0xf) as usize] as char);
        }
        out
    }

    /// **The exact text a browser wallet is asked to sign.**
    ///
    /// Only a browser needs this, and it is the one place the desktop
    /// inversion cannot hold: the elo launcher composes its own sentence and
    /// we hand it three inert values, but `window.ethereum` has no `prove`
    /// verb — a wallet signs text or nothing. So the text is built HERE, in
    /// Rust, through [`protocol::siwe_message`] — the same function the shard
    /// calls to rebuild what it verifies (`server::auth::verify`).
    ///
    /// ⚠ **The point is that the page never composes it.** A JS
    /// reimplementation of an EIP-4361 message would be a second copy of a
    /// byte-exact format whose only symptom on drift is every login failing
    /// as `WrongSigner`, which reads as "signing is broken" rather than as
    /// one wrong space. `protocol` already compiles to wasm; the module hands
    /// the finished string to the wallet and the human reads it there.
    /// `crates/server/tests/siwe_wire.rs` runs a signature over THIS text
    /// through the real shard.
    ///
    /// The address is EIP-55 checksummed, which is load-bearing rather than
    /// cosmetic: the case is inside the signed bytes, so a lowercase spelling
    /// recovers a different digest and is refused. See
    /// [`protocol::Address::to_checksum_hex`].
    pub fn message(&self, address: protocol::Address) -> String {
        let sum = address.to_checksum_hex();
        // `to_checksum_hex` writes ASCII hex and `0x`, so this cannot fail;
        // the fallback keeps the signature total rather than panicking a page.
        let addr = core::str::from_utf8(&sum).unwrap_or("0x");
        let mut text = [0u8; protocol::SIWE_MESSAGE_MAX];
        let n = protocol::siwe_message(
            &self.domain,
            addr,
            protocol::SLUG,
            &self.nonce,
            self.issued_at,
            &mut text,
        )
        .min(protocol::SIWE_MESSAGE_MAX);
        String::from_utf8_lossy(&text[..n]).into_owned()
    }
}

/// Step 2a — read the shard's challenge and say what must be signed.
///
/// `Ok(None)` is a **guest**: nothing to prove, nothing to ask a signer for.
/// A shard with `require_auth` will refuse that at step 3, which is the right
/// place to learn it.
///
/// `server` is the `host:port` **we dialled**, and the domain is taken from
/// it rather than from anything the shard said. That is the whole of SIWE's
/// domain binding: a signature collected by one shard is not valid at another
/// because the two messages differ, and letting the server name itself would
/// hand that away.
///
/// ⚠ **This is half of what used to be [`auth_for`], and the split exists for
/// one reason: a browser signature cannot be obtained synchronously.**
/// `window.ethereum.request` returns a Promise and the nonce only exists
/// mid-handshake, so it cannot be pre-signed. Making this function `async`
/// would have been the obvious fix and is the one this module's header
/// forbids — a `Send` future natively, a non-`Send` one in a page. So the
/// *awaiting* stays in whichever `connect` is driving, where `async` already
/// lives, and both halves of the rule stay pure and testable with no shard.
pub fn proof_wanted(
    frame: &[u8],
    server: &str,
    address: protocol::Address,
) -> Result<Option<Proof>, JoinError> {
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

    if address.is_guest() {
        return Ok(None);
    }

    let domain = server.rsplit_once(':').map(|(h, _)| h).unwrap_or(server);
    Ok(Some(Proof {
        domain: domain.to_string(),
        nonce: challenge.nonce,
        issued_at: challenge.issued_at,
    }))
}

/// Step 2b — the auth frame, from whatever the signer answered.
///
/// `None` means "connect as a guest", which is what a declined prompt, an
/// absent launcher, a locked wallet or a guest address all produce. It is
/// deliberately not an error here: a shard that takes guests will admit one,
/// and a shard that does not answers `REFUSE_AUTH` at step 3 with a code the
/// caller can turn into the right sentence.
pub fn auth_frame(
    address: protocol::Address,
    signature: Option<protocol::Signature>,
    buf: &mut [u8],
) -> Result<usize, JoinError> {
    let auth = match signature {
        Some(signature) => protocol::Auth { address, signature },
        None => protocol::Auth::default(),
    };
    protocol::encode_auth(&auth, buf).map_err(|e| JoinError::Failed(format!("encode auth: {e:?}")))
}

/// Step 2 — read the shard's challenge, answer with the auth frame.
///
/// **The synchronous composition of [`proof_wanted`] and [`auth_frame`]**, for
/// a caller whose signer answers without awaiting — which is the desktop
/// client, where the elo launcher is reached over a blocking local socket. A
/// browser calls the two halves with an `await` between them; there is one
/// implementation of each rule either way, and this function holds no rule of
/// its own.
///
/// `server` is the `host:port` **we dialled** — see [`proof_wanted`] for the
/// domain binding that depends on it.
///
/// **This process does not compose the message, and that is the fix.**
/// It used to build the SIWE text here and hand it to the launcher's
/// `sign`, which refused every one: `sign` classifies a message by its
/// first line (`elo <family>`) and an EIP-4361 message begins with a domain.
/// The refusal became `None`, `None` means "connect as a guest", and a
/// `require_auth` shard answered REFUSE_AUTH — so every login failed as
/// if the signature were wrong, when the message was one the launcher
/// would never sign.
///
/// `prove` is the verb: the LAUNCHER writes every word, which is what
/// stops a game smuggling a sentence into a signature, and it needs no
/// consent prompt for the same reason. We hand over three inert values
/// and the shard rebuilds the text from the ones it already knows
/// (`protocol::siwe_message`).
pub fn auth_for(
    frame: &[u8],
    server: &str,
    address: protocol::Address,
    // `(domain, nonce hex, issued_at)` — the three inert values the launcher
    // needs, never a message this process composed.
    sign: impl FnOnce(&str, &str, u64) -> Option<protocol::Signature>,
    buf: &mut [u8],
) -> Result<usize, JoinError> {
    let signature = proof_wanted(frame, server, address)?
        .and_then(|p| sign(&p.domain, &p.nonce_hex(), p.issued_at));
    auth_frame(address, signature, buf)
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

    /// **The guard on the split, and it is the one that matters.** The desktop
    /// path composes ([`auth_for`]); a page runs the same two halves with an
    /// `await` between them because a wallet signature is a Promise. Two call
    /// sequences over one pair of rules is exactly the shape
    /// `tests/connect_twins.rs` exists to watch, and this is its code-tier
    /// half: given the same challenge and the same signature, the frame is
    /// byte-identical whichever way it was assembled.
    ///
    /// Proven by construction rather than by inspection — `auth_for` is
    /// written as the composition — but a future edit that "optimises" one
    /// side reddens here, which is the point.
    #[test]
    fn the_two_step_path_and_the_composition_agree() {
        let frame = challenge_frame([7; protocol::NONCE_BYTES], 1_700_000_000);
        let sig = protocol::Signature([9; protocol::SIGNATURE_BYTES]);

        let mut composed = [0u8; MAX_STREAM_MSG_BYTES];
        let a = auth_for(
            &frame,
            "shard.example:4433",
            someone(),
            |_, _, _| Some(sig),
            &mut composed,
        )
        .expect("composed");

        // The way a page must do it: decode, hand the three values out, come
        // back with a signature, encode.
        let want = proof_wanted(&frame, "shard.example:4433", someone())
            .expect("decodes")
            .expect("not a guest");
        assert_eq!(want.domain, "shard.example");
        assert_eq!(want.issued_at, 1_700_000_000);
        let mut stepped = [0u8; MAX_STREAM_MSG_BYTES];
        let b = auth_frame(someone(), Some(sig), &mut stepped).expect("stepped");

        assert_eq!(composed[..a], stepped[..b], "the two paths must agree");

        // ⚠ **The equality above is satisfied by both paths being equally
        // wrong**, because both run through `auth_frame`. That is
        // `CLAUDE.md`'s `lattice.rs` trap — a rebuild that calls the function
        // under test carries the same mutant on both sides — and it was
        // committed here and caught by running the mutant: `auth_frame`
        // ignoring its signature and always encoding a guest passed every
        // assertion in this file.
        //
        // So the frame is DECODED, through the protocol's own reader, and the
        // values are the ones that went in. A browser that silently joined as
        // a guest with a signature in hand would reach a `require_auth` shard
        // as `REFUSE_AUTH` — a refusal naming the shard for a defect in the
        // client.
        let got = protocol::decode_auth(&composed[..a]).expect("the frame decodes");
        assert_eq!(got.address, someone(), "the claimed address must survive");
        assert_eq!(got.signature.0, sig.0, "the signature must reach the wire");
        assert!(!got.address.is_guest(), "a signed join is not a guest join");
    }

    /// The other half of the same claim: **no signature means a guest frame**,
    /// and the address is dropped with it rather than travelling as a claim
    /// nobody proved.
    #[test]
    fn an_unsigned_join_carries_no_address_at_all() {
        let mut buf = [0u8; MAX_STREAM_MSG_BYTES];
        let n = auth_frame(someone(), None, &mut buf).expect("encodes");
        let got = protocol::decode_auth(&buf[..n]).expect("decodes");
        assert!(
            got.address.is_guest(),
            "an unsigned join must not carry the address it could not prove"
        );
    }

    /// A guest has nothing to prove, so no signer is consulted and no nonce is
    /// rendered. `None` rather than an error: a shard that takes guests admits
    /// one, and a shard that does not says so at step 3 with a code.
    #[test]
    fn a_guest_has_nothing_to_prove() {
        let frame = challenge_frame([1; protocol::NONCE_BYTES], 42);
        assert_eq!(
            proof_wanted(&frame, "shard.example:4433", Address::GUEST).expect("decodes"),
            None
        );
    }

    /// The nonce reaches the signer as lowercase hex of the full 32 bytes —
    /// the shape `protocol::siwe_message` writes, so a signer that passes it
    /// through unchanged produces bytes the shard can rebuild.
    #[test]
    fn the_nonce_is_lowercase_hex_of_every_byte() {
        let mut nonce = [0u8; protocol::NONCE_BYTES];
        nonce[0] = 0xAB;
        nonce[protocol::NONCE_BYTES - 1] = 0x0F;
        let frame = challenge_frame(nonce, 1);
        let p = proof_wanted(&frame, "s:1", someone()).unwrap().unwrap();
        let hex = p.nonce_hex();
        assert_eq!(hex.len(), protocol::NONCE_BYTES * 2);
        assert!(hex.starts_with("ab"), "{hex}");
        assert!(hex.ends_with("0f"), "{hex}");
        assert!(!hex.chars().any(|c| c.is_ascii_uppercase()));
    }
}
