//! Does this session token name a player?
//!
//! **The seam, and today it is a stub that says so.** The shape is the Steam
//! one: the launcher issues a session token, the game relays it in `Hello`,
//! and the shard asks the ISSUER whether it is good — one call to scry,
//! answered with a stable player key. Rust-the-game does exactly this against
//! Steam's Web API, and the reason to copy it is that no part of it needs a
//! wallet, a nonce round-trip or a chain lookup to answer "who is this".
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

use protocol::AuthToken;

/// Whether a token is admissible. See the header — this is a shape check.
pub fn validate_session(token: &AuthToken) -> bool {
    !token.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_carries_nothing_and_a_holder_carries_something() {
        assert!(!validate_session(&AuthToken::NONE));
        assert!(validate_session(
            &AuthToken::new(b"a-session-handle").expect("fits")
        ));
    }

    /// The stub must not be mistaken for validation: any non-empty token
    /// passes, including obvious nonsense. This test exists to FAIL the day
    /// someone wires the real validator without updating it, which is the
    /// moment the stub stops being honest.
    #[test]
    fn the_stub_accepts_nonsense_and_that_is_the_point() {
        assert!(validate_session(&AuthToken::new(b"x").expect("fits")));
    }
}
