//! **Who is this, proved.** Recover the signer of a SIWE signature and turn
//! it into the key a save is filed under.
//!
//! ## This used to be a stub and the stub was worse than it looked
//!
//! It accepted any non-empty session token and returned the token's own
//! bytes as the player key. Two consequences, and the second is the one that
//! justified replacing the whole model rather than filling in the middle:
//!
//! - Anyone who knew your token was you. That was written down honestly and
//!   was survivable only while nobody was playing.
//! - **A session token rotates.** The key rotated with it, so every login
//!   would have minted a new character and persistence would have silently
//!   done nothing for every real player — with every gate green, because no
//!   gate can see that two runs of a stub disagree about who somebody is.
//!
//! ## What replaced it, and why it is smaller
//!
//! `protocol::auth` carries the wire half: a server nonce, a SIWE message
//! built by one shared function, an address and a 65-byte signature. This
//! module is the arithmetic: keccak the EIP-191 envelope, `ecrecover` the
//! public key, hash it to an address, and compare.
//!
//! No network call. No issuer to be unreachable. No "what does a shard do
//! when it cannot reach scry" policy question, which is the one the token
//! model could not answer and the reason it kept not landing. The identity
//! is a public key hash and the proof is arithmetic over bytes we already
//! have.
//!
//! ## The key is the address, lowercase, and that is forever
//!
//! Once players have bases, the key is load-bearing in a way nothing else
//! here is: changing how it is derived orphans every save on the shard.
//! So it is pinned two ways — `protocol::auth` has a golden on the address
//! derivation against a published vector (private key 1), and this module
//! has a golden on the full recover-and-compare path.

use protocol::{siwe_message, Address, Auth, Signature, NONCE_BYTES, SIWE_MESSAGE_MAX};

use crate::store::PlayerKey;
use k256::ecdsa::{RecoveryId, Signature as EcdsaSig, VerifyingKey};
use tiny_keccak::{Hasher, Keccak};

/// Why a signature was not accepted. Integer-shaped and never carrying the
/// bytes it refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// The address was the guest sentinel — nothing to prove, and nothing
    /// is claimed. Only an error on a shard that requires identity.
    Guest,
    /// `v` was not 27, 28, 0 or 1 — not a `personal_sign` recovery id.
    BadRecoveryId,
    /// The signature is not a point on the curve / not a valid ECDSA pair.
    Malformed,
    /// It verified, but against a **different** address than the one
    /// claimed. The interesting failure: somebody signed something, just not
    /// as who they said they were.
    WrongSigner,
}

impl AuthError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Guest => "no address was offered",
            Self::BadRecoveryId => "the signature's recovery id is not a personal_sign one",
            Self::Malformed => "the signature is not a valid secp256k1 signature",
            Self::WrongSigner => "the signature is valid but belongs to a different address",
        }
    }
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut k = Keccak::v256();
    let mut out = [0u8; 32];
    k.update(data);
    k.finalize(&mut out);
    out
}

/// An address from a public key: the last 20 bytes of `keccak256` over the
/// uncompressed point *without* its `0x04` tag.
///
/// **The single most consequential line in this file.** Derive it any other
/// way — keep the tag, take the first 20 bytes, hash the compressed form —
/// and every player is filed under a key that is not theirs, consistently,
/// forever, with every other test green. `protocol::auth`'s golden against
/// the published vector for private key 1 is what pins it.
fn address_of(vk: &VerifyingKey) -> Address {
    let point = vk.to_encoded_point(false);
    let bytes = point.as_bytes();
    let h = keccak256(&bytes[1..]);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    Address(a)
}

/// EIP-55: the same 42 characters as [`Address::to_hex`], re-cased.
///
/// A hex digit goes uppercase when the matching nibble of `keccak256` over
/// the **lowercase hex text** (no `0x`) is 8 or more. Digits `0-9` have no
/// case and are left alone.
///
/// **Why a shard needs this at all.** The identity we store stays lowercase —
/// `Address::to_hex` says why, and a save filed under a checksummed key would
/// depend on how the address was capitalised where it came from. But the
/// *message* is scry's, and scry's launcher binds the checksummed spelling
/// into the bytes it signs, because EIP-4361 requires it and reference SIWE
/// parsers reject an all-lowercase address outright. So the case matters in
/// exactly one place — recomputing what was signed — and nowhere else.
pub(crate) fn checksum_hex(address: &Address) -> [u8; 42] {
    let lower = address.to_hex();
    let h = keccak256(&lower[2..]); // the hex text, without `0x`
    let mut out = lower;
    for i in 0..40 {
        let c = out[2 + i];
        if !c.is_ascii_alphabetic() {
            continue;
        }
        // Nibble i of the digest: high nibble for even i, low for odd.
        let nib = if i % 2 == 0 {
            h[i / 2] >> 4
        } else {
            h[i / 2] & 0xf
        };
        if nib >= 8 {
            out[2 + i] = c.to_ascii_uppercase();
        }
    }
    out
}

/// The EIP-191 `personal_sign` digest: the message under its length-prefixed
/// envelope, keccaked.
///
/// The envelope is not decoration — it is what stops a signature obtained
/// here from being a valid *transaction* signature. Every wallet applies it
/// to `personal_sign`, so a verifier that skipped it would reject every real
/// signature while accepting hand-made ones, which is the wrong way round.
fn personal_sign_digest(message: &[u8]) -> [u8; 32] {
    let mut envelope = [0u8; SIWE_MESSAGE_MAX + 32];
    let prefix = b"\x19Ethereum Signed Message:\n";
    let mut at = 0;
    envelope[at..at + prefix.len()].copy_from_slice(prefix);
    at += prefix.len();
    // The length in decimal, written without allocating.
    let mut digits = [0u8; 20];
    let mut n = 0;
    let mut v = message.len();
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        envelope[at] = digits[i];
        at += 1;
    }
    let end = at + message.len();
    envelope[at..end].copy_from_slice(message);
    keccak256(&envelope[..end])
}

/// Verify a client's [`Auth`] against the nonce this connection issued.
///
/// The message is **rebuilt here** from the same inputs the client had,
/// through `protocol::siwe_message` — the one implementation both sides
/// call. The client never sends the text, so it cannot get a signature over
/// something else accepted by telling us what it signed.
pub fn verify(
    domain: &str,
    nonce: &[u8; NONCE_BYTES],
    issued_at: u64,
    auth: &Auth,
) -> Result<PlayerKey, AuthError> {
    if auth.address.is_guest() {
        return Err(AuthError::Guest);
    }
    let mut text = [0u8; SIWE_MESSAGE_MAX];
    // EIP-55, not `Address::to_hex`'s lowercase: the launcher binds the
    // checksummed spelling into the bytes it signs, so recomputing with a
    // lowercase address recovers a different digest and every honest login
    // fails as `WrongSigner`. The case is part of the message.
    let checksummed = checksum_hex(&auth.address);
    let addr = core::str::from_utf8(&checksummed).map_err(|_| AuthError::Malformed)?;
    let len = siwe_message(domain, addr, protocol::SLUG, nonce, issued_at, &mut text);
    let len = len.min(SIWE_MESSAGE_MAX);
    let digest = personal_sign_digest(&text[..len]);

    let Signature(raw) = auth.signature;
    // `v` is 27/28 from every wallet and 0/1 from some libraries. Both are
    // accepted because both are in the wild and the difference is a
    // historical accident, not a security property.
    let rec = match raw[64] {
        27 | 0 => 0u8,
        28 | 1 => 1u8,
        _ => return Err(AuthError::BadRecoveryId),
    };
    let sig = EcdsaSig::from_slice(&raw[..64]).map_err(|_| AuthError::Malformed)?;
    let rec_id = RecoveryId::from_byte(rec).ok_or(AuthError::BadRecoveryId)?;
    let vk = VerifyingKey::recover_from_prehash(&digest, &sig, rec_id)
        .map_err(|_| AuthError::Malformed)?;

    let signer = address_of(&vk);
    if signer != auth.address {
        return Err(AuthError::WrongSigner);
    }
    // The key is the lowercase `0x…` text of the address. Text rather than
    // the 20 raw bytes because `PlayerKey` is what an operator reads out of
    // a save file when they have to support it, and 20 bytes of binary in a
    // hexdump is a worse afternoon than 42 characters that are obviously an
    // address.
    let hex = signer.to_hex();
    PlayerKey::new(&hex).ok_or(AuthError::Malformed)
}

/// The key for an address that has already been proved — the one place that
/// decides the on-disk spelling, so nothing else has to agree with it.
pub fn key_of(address: &Address) -> Option<PlayerKey> {
    if address.is_guest() {
        return None;
    }
    PlayerKey::new(&address.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    const DOMAIN: &str = "gates.test";

    fn signer_for(byte: u8) -> SigningKey {
        let mut sk = [0u8; 32];
        sk[31] = byte;
        SigningKey::from_bytes(&sk.into()).expect("a small scalar is a valid key")
    }

    /// Sign the challenge exactly as a wallet would, so these tests exercise
    /// the real path rather than a convenient one.
    fn sign_as(sk: &SigningKey, nonce: &[u8; NONCE_BYTES], issued_at: u64, v_style: u8) -> Auth {
        let address = address_of(sk.verifying_key());
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        // Checksummed, exactly as `verify` recomputes it and a launcher signs
        // it — a test that signed the lowercase form would pass against a
        // verifier that did the same and fail against every real wallet.
        let checksummed = checksum_hex(&address);
        let addr = core::str::from_utf8(&checksummed).expect("utf-8");
        let len = siwe_message(DOMAIN, addr, protocol::SLUG, nonce, issued_at, &mut text);
        let digest = personal_sign_digest(&text[..len]);
        let (sig, rec) = sk.sign_prehash_recoverable(&digest).expect("signs");
        let mut raw = [0u8; 65];
        raw[..64].copy_from_slice(&sig.to_bytes());
        raw[64] = if v_style == 27 {
            27 + rec.to_byte()
        } else {
            rec.to_byte()
        };
        Auth {
            address,
            signature: Signature(raw),
        }
    }

    /// **The whole scheme, end to end**, and the assertion that matters is
    /// the key: it is the address, lowercase, and it is what a save is filed
    /// under forever.
    #[test]
    fn a_real_signature_recovers_its_own_address() {
        let sk = signer_for(1);
        let nonce = [7u8; NONCE_BYTES];
        let auth = sign_as(&sk, &nonce, 1_770_000_000, 27);
        let key = verify(DOMAIN, &nonce, 1_770_000_000, &auth).expect("a real signature verifies");
        assert_eq!(
            key.as_bytes(),
            b"0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
            "the player key is not the lowercase address — every save on the \
             shard would be filed under the wrong name"
        );
    }

    /// Both `v` conventions are in the wild. Rejecting either would refuse
    /// half the wallets on earth for a historical accident.
    #[test]
    fn both_recovery_id_conventions_are_accepted() {
        let sk = signer_for(9);
        let nonce = [3u8; NONCE_BYTES];
        for style in [27u8, 0u8] {
            let auth = sign_as(&sk, &nonce, 42, style);
            assert!(
                verify(DOMAIN, &nonce, 42, &auth).is_ok(),
                "v-style {style} was refused"
            );
        }
    }

    /// **The attack this exists to stop**: a real signature, presented with
    /// somebody else's address. It verifies as *someone*, just not as who it
    /// claims — and that is the difference between authentication and a
    /// spelling check.
    #[test]
    fn a_signature_from_the_wrong_key_is_refused() {
        let me = signer_for(1);
        let them = signer_for(2);
        let nonce = [5u8; NONCE_BYTES];
        let mut auth = sign_as(&them, &nonce, 100, 27);
        // Keep their signature, claim my address.
        auth.address = address_of(me.verifying_key());
        assert_eq!(
            verify(DOMAIN, &nonce, 100, &auth),
            Err(AuthError::WrongSigner)
        );
    }

    /// **Replay.** A signature is over a nonce this connection chose, so one
    /// captured from another connection recovers a different signer than the
    /// address claims and is refused. This is the property that makes the
    /// per-connection nonce worth having.
    #[test]
    fn a_signature_for_another_nonce_is_refused() {
        let sk = signer_for(4);
        let auth = sign_as(&sk, &[1u8; NONCE_BYTES], 100, 27);
        assert!(
            verify(DOMAIN, &[2u8; NONCE_BYTES], 100, &auth).is_err(),
            "a signature was accepted against a nonce it did not sign"
        );
    }

    /// And over the timestamp, and over the domain — so a signature
    /// collected by one shard cannot be presented at another.
    #[test]
    fn a_signature_for_another_shard_or_time_is_refused() {
        let sk = signer_for(6);
        let nonce = [8u8; NONCE_BYTES];
        let auth = sign_as(&sk, &nonce, 1_000, 27);
        assert!(verify(DOMAIN, &nonce, 1_001, &auth).is_err(), "time");
        assert!(
            verify("other.shard", &nonce, 1_000, &auth).is_err(),
            "domain"
        );
    }

    /// Garbage never panics, it refuses. This runs on the handshake path,
    /// where the bytes are a stranger's.
    #[test]
    fn arbitrary_bytes_are_refused_and_never_panic() {
        let mut rng = sim_core::rng::Pcg32::new(0x5157_4553, 3);
        for _ in 0..500 {
            let mut raw = [0u8; 65];
            for b in raw.iter_mut() {
                *b = rng.next_bounded(256) as u8;
            }
            let mut addr = [0u8; 20];
            for b in addr.iter_mut() {
                *b = rng.next_bounded(256) as u8;
            }
            let auth = Auth {
                address: Address(addr),
                signature: Signature(raw),
            };
            let _ = verify(DOMAIN, &[0; NONCE_BYTES], 0, &auth);
        }
    }

    /// A guest is refused by name rather than by accident, so a shard that
    /// requires identity can say which of the two happened.
    #[test]
    fn a_guest_is_refused_as_a_guest() {
        assert_eq!(
            verify(DOMAIN, &[0; NONCE_BYTES], 0, &Auth::default()),
            Err(AuthError::Guest)
        );
        assert_eq!(key_of(&Address::GUEST), None);
    }

    /// The two paths that produce a key must produce the same one, or a
    /// player verified at the door would be filed under a different name
    /// than one restored from a file.
    #[test]
    fn the_verified_key_and_the_derived_key_agree() {
        let sk = signer_for(11);
        let nonce = [2u8; NONCE_BYTES];
        let auth = sign_as(&sk, &nonce, 7, 27);
        let verified = verify(DOMAIN, &nonce, 7, &auth).expect("verifies");
        assert_eq!(Some(verified), key_of(&auth.address));
    }
}

#[cfg(test)]
mod checksum_tests {
    use super::*;

    /// The four canonical EIP-55 vectors from the spec itself.
    ///
    /// Pinned against published values rather than against our own output,
    /// because a checksum that is self-consistent and wrong re-cases every
    /// address the same way and fails every login identically — which reads
    /// as "signing is broken" and not as "this function is wrong".
    #[test]
    fn checksum_matches_the_eip55_vectors() {
        for want in [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ] {
            let a = Address::from_hex(want.as_bytes()).expect("a valid address");
            let got = checksum_hex(&a);
            assert_eq!(
                core::str::from_utf8(&got).expect("utf-8"),
                want,
                "EIP-55 vector"
            );
        }
    }

    /// The address the launcher reports for the dev wallet, re-cased here.
    /// This is the one that actually has to match in production.
    #[test]
    fn the_dev_wallet_rechecksums_to_what_the_launcher_reports() {
        let a = Address::from_hex(b"0x24445efddb08d4938e3e3627042b2cf4063d9092").unwrap();
        assert_eq!(
            core::str::from_utf8(&checksum_hex(&a)).unwrap(),
            "0x24445EFddB08d4938E3E3627042B2Cf4063d9092"
        );
    }
}
