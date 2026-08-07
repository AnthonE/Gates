//! Does this session token name a player, and **who**?
//!
//! **The seam, and today it is a stub that says so.** The shape is the Steam
//! one: the launcher issues a session token, the game relays it in `Hello`,
//! and the shard asks the ISSUER whether it is good — one call to scry,
//! answered with a stable player key. Rust-the-game does exactly this against
//! Steam's Web API, and the reason to copy it is that no part of it needs a
//! wallet, a nonce round-trip or a chain lookup to answer "who is this".
//!
//! ## It returns a key, and that is what dissolved the argument
//!
//! This used to answer `bool`, which was enough to admit somebody and not
//! enough to remember them. A shard that only knows "this joiner carried a
//! credential" builds a fresh character on every connection, so any amount
//! of authentication ceremony — signatures, expiry windows, on-chain ticket
//! checks — bought nothing: you could prove who you were perfectly and still
//! lose everything on a dropped connection.
//!
//! `Option<PlayerKey>` is the whole fix. The key is the string a save is
//! filed under (`store.rs`), and **which identity scheme produces it is this
//! function's business alone**. Session token, EIP-191 signature, scry player
//! id, dev flag: each is a different body here, and none of them moves the
//! wire, the store, the sim, or a single test. That is why persistence did
//! not have to wait for the identity question to be settled.
//!
//! ## The key must never be a credential
//!
//! `store.rs` writes it to disk verbatim, and a save file is not a secret
//! store. So whatever lands here must return a *name*, not a token — a
//! stable public identifier the issuer resolves the credential to. The stub
//! below returns the token's own bytes, and that is only safe because of what
//! the stub is: with no validation, a token IS a bare name, and **anyone who
//! knows your name is you**. That is already true of a shard running this
//! stub. Persistence does not make it worse; it makes it visible, which is
//! the argument for landing the honest stub rather than a plausible one.
//!
//! ## What this does NOT do, stated plainly
//!
//! It does not talk to scry. [`validate_session`] accepts any non-empty token
//! and rejects an empty one, so with `require_auth = true` a shard proves a
//! client CARRIED a credential and nothing more. That is not authentication
//! and must not be armed on a public shard as though it were — the knob's
//! default is `false` for that reason and `config.rs` repeats it.
//!
//! It is landed as a stub rather than left out because the wire, the refusal,
//! the counter and the policy knob are the parts that need a `PROTO_VER` bump
//! and goldens, and they are auth-agnostic: whatever validator lands behind
//! this function, none of that moves again. The validator is one function and
//! one HTTP call.
//!
//! ## What the real one needs
//!
//! An endpoint that takes a token and returns a stable key, plus the failure
//! posture: a shard that cannot REACH the issuer must decide between refusing
//! everyone and admitting everyone, and that is an operator call, not a
//! default. Neither is written here because scry's session API is not
//! published yet (`sdk/PROTOCOL.md`).

use crate::store::PlayerKey;
use protocol::AuthToken;

/// Who this token names, or `None` if it names nobody. See the header — this
/// is a shape check standing where an issuer lookup goes.
///
/// `None` is a normal, playable state on a shard that takes guests: a joiner
/// with no key is admitted and simply remembered by nobody, because there is
/// no stable string to file them under. That is not a downgrade of anything —
/// it is exactly what every shard did before the store existed.
pub fn validate_session(token: &AuthToken) -> Option<PlayerKey> {
    // The token's own bytes, capped at `PLAYER_KEY_MAX_BYTES` by
    // `PlayerKey::new`. A token longer than the key cap therefore names
    // nobody rather than being truncated into somebody else's key — the same
    // refuse-never-truncate rule `AuthToken::new` follows, and here it is
    // load-bearing: two players sharing a key would share a save.
    PlayerKey::new(token.as_bytes())
}

/// Whether a token is admissible at all — what `require_auth` gates on.
///
/// Separate from the key on purpose: admission and identity are different
/// questions, and a real validator will one day admit a joiner it cannot
/// name (a shard that trusts its own LAN) or name one it will not admit (a
/// banned key). Today both answers come from the same call, and this says so
/// in one line rather than by having two callers each reinvent it.
pub fn admits(token: &AuthToken) -> bool {
    validate_session(token).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_carries_nothing_and_a_holder_carries_something() {
        assert!(!admits(&AuthToken::NONE));
        assert_eq!(validate_session(&AuthToken::NONE), None);
        assert!(admits(&AuthToken::new(b"a-session-handle").expect("fits")));
    }

    /// The stub must not be mistaken for validation: any non-empty token
    /// passes, including obvious nonsense. This test exists to FAIL the day
    /// someone wires the real validator without updating it, which is the
    /// moment the stub stops being honest.
    #[test]
    fn the_stub_accepts_nonsense_and_that_is_the_point() {
        assert!(admits(&AuthToken::new(b"x").expect("fits")));
    }

    /// The property persistence rests on, and the only one it needs: the same
    /// token resolves to the same key every time. Everything else about
    /// identity here is a stub; this is a contract, and a validator that
    /// returned a fresh key per connection would silently give every player a
    /// new character on every join while every gate stayed green.
    #[test]
    fn one_token_is_always_one_key() {
        let t = AuthToken::new(b"dev-anthone").expect("fits");
        let a = validate_session(&t).expect("names somebody");
        let b = validate_session(&AuthToken::new(b"dev-anthone").expect("fits"))
            .expect("names somebody");
        assert_eq!(a, b, "the same token must name the same player");
        let other = validate_session(&AuthToken::new(b"dev-someone-else").expect("fits"))
            .expect("names somebody");
        assert_ne!(a, other, "two tokens must not share one save");
    }

    /// **Every token the wire admits must name somebody.** The failure this
    /// guards is silent and total: a key cap under the token cap would admit a
    /// launcher's 40-byte session and then file it under nobody, so
    /// persistence would do nothing for every real player while every gate
    /// stayed green and the dev shard's short tokens worked fine. `store.rs`
    /// has a static assert for the constants; this is the same claim through
    /// the actual call, at the largest token that exists.
    #[test]
    fn a_token_at_the_wire_cap_still_names_somebody() {
        let long = AuthToken::new(&[b'k'; protocol::AUTH_TOKEN_MAX_BYTES]).expect("fits the wire");
        let key = validate_session(&long).expect("a token the wire admits must name somebody");
        assert_eq!(key.as_bytes().len(), protocol::AUTH_TOKEN_MAX_BYTES);
    }
}
