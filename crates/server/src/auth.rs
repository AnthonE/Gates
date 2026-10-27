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
//! when it cannot reach elo" policy question, which is the one the token
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

use protocol::{Address, Auth, Signature, NONCE_BYTES, SIWE_MESSAGE_MAX};

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
    /// An Elo Pros pass this shard takes none of: no `elo_account_pubkey`.
    NoPassKey,
    /// A pass for another game, or bound to a name this shard does not
    /// answer to — one a player handed some other server.
    PassElsewhere,
    /// A pass past its `exp`.
    PassExpired,
    /// A pass for another join: its nonce is not the one this join was
    /// handed, so it was asked for elsewhere or is being replayed.
    PassReplayed,
}

impl AuthError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Guest => "no address was offered",
            Self::BadRecoveryId => "the signature's recovery id is not a personal_sign one",
            Self::Malformed => "the signature is not a valid secp256k1 signature",
            Self::WrongSigner => "the signature is valid but belongs to a different address",
            Self::NoPassKey => "this shard takes no Elo Pros pass",
            Self::PassElsewhere => "the pass is for another game or another shard",
            Self::PassExpired => "the pass has run out",
            Self::PassReplayed => "the pass was asked for another join",
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
    verify_any(&[domain], nonce, issued_at, auth)
}

/// [`verify`] against a shard that answers to more than one name
/// (`ShardConfig::siwe_accepts`): the proof is good if it is bound to ANY of
/// `domains`, and to nothing else.
///
/// **The client still never says which name it signed.** The wire carries an
/// address and a signature and no text, so the shard rebuilds the message
/// once per listed name and keeps the one whose recovered signer is the
/// claimed address. A name outside the list is never rebuilt, so a proof
/// bound to it recovers nobody it could match — which is the whole of the
/// domain binding, widened by exactly the names the operator wrote.
///
/// **And bound to this shard.** A page on `elopros.com` signs its own name
/// in the domain line for every shard under it, so the `URI:` line names the
/// shard it dialled (`protocol::siwe_message_for`), and a proof is good only
/// if that too is one of `domains` — the names THIS shard answers to. A
/// sibling shard's proof names the sibling and is refused, so it cannot be
/// passed on here. Every listed name against every listed name: the
/// launcher's own form (URI = domain), a page's, and a name kept listed
/// through a rename alike.
///
/// One name is [`verify`] exactly: the same checks in the same order, the
/// same refusal for each failure. An empty list proves nothing and refuses —
/// there is no message the signature could have been over.
pub fn verify_any<S: AsRef<str>>(
    domains: &[S],
    nonce: &[u8; NONCE_BYTES],
    issued_at: u64,
    auth: &Auth,
) -> Result<PlayerKey, AuthError> {
    verify_text(domains, auth, |domain, uri, addr, text| {
        protocol::siwe_message_for(domain, uri, addr, protocol::SLUG, nonce, issued_at, text)
    })
}

/// [`verify_any`] over the companion page's own text
/// (`protocol::companion_message`), which the game never accepts: a sign-in
/// collected for the page is no game login, and a game login is no sign-in.
pub fn verify_companion<S: AsRef<str>>(
    domains: &[S],
    nonce: &[u8; NONCE_BYTES],
    issued_at: u64,
    auth: &Auth,
) -> Result<PlayerKey, AuthError> {
    verify_text(domains, auth, |domain, uri, addr, text| {
        protocol::companion_message(domain, uri, addr, nonce, issued_at, text)
    })
}

/// The recovery both texts share: `compose` writes the message for one
/// (domain, URI) pair and the checksummed address.
fn verify_text<S: AsRef<str>>(
    domains: &[S],
    auth: &Auth,
    compose: impl Fn(&str, &str, &str, &mut [u8; SIWE_MESSAGE_MAX]) -> usize,
) -> Result<PlayerKey, AuthError> {
    if auth.address.is_guest() {
        return Err(AuthError::Guest);
    }
    // EIP-55, not `Address::to_hex`'s lowercase: the launcher binds the
    // checksummed spelling into the bytes it signs, so recomputing with a
    // lowercase address recovers a different digest and every honest login
    // fails as `WrongSigner`. The case is part of the message.
    let checksummed = auth.address.to_checksum_hex();
    let addr = core::str::from_utf8(&checksummed).map_err(|_| AuthError::Malformed)?;

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

    // `WrongSigner` outranks `Malformed`: if any name recovered somebody,
    // somebody signed something, which is the more useful count.
    let mut refused = None;
    let pairs = domains
        .iter()
        .flat_map(|d| domains.iter().map(move |u| (d.as_ref(), u.as_ref())));
    for (domain, uri) in pairs {
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        let len = compose(domain, uri, addr, &mut text).min(SIWE_MESSAGE_MAX);
        let digest = personal_sign_digest(&text[..len]);
        match VerifyingKey::recover_from_prehash(&digest, &sig, rec_id) {
            Ok(vk) => {
                let signer = address_of(&vk);
                if signer == auth.address {
                    // The key is the lowercase `0x…` text of the address.
                    // Text rather than the 20 raw bytes because `PlayerKey`
                    // is what an operator reads out of a save file when they
                    // have to support it, and 20 bytes of binary in a hexdump
                    // is a worse afternoon than 42 characters that are
                    // obviously an address.
                    let hex = signer.to_hex();
                    return PlayerKey::new(&hex).ok_or(AuthError::Malformed);
                }
                refused = Some(AuthError::WrongSigner);
            }
            Err(_) => {
                refused.get_or_insert(AuthError::Malformed);
            }
        }
    }
    Err(refused.unwrap_or(AuthError::WrongSigner))
}

/// Verify a **Solana** account's [`Auth`] (v142): an Ed25519 signature by the
/// key it names over [`protocol::siws_message_for`], rebuilt here for every
/// (domain, URI) pair this shard answers to — [`verify_any`]'s binding,
/// unchanged. Ed25519 is `ring`'s (already linked under rustls), and the key
/// is the identity: there is nothing to recover, only one key to check.
pub fn verify_solana_any<S: AsRef<str>>(
    domains: &[S],
    nonce: &[u8; NONCE_BYTES],
    issued_at: u64,
    auth: &Auth,
) -> Result<protocol::SolanaKey, AuthError> {
    let Some(sol) = auth.solana else {
        return Err(AuthError::Guest);
    };
    if sol.key.0 == [0u8; protocol::SOLANA_KEY_BYTES] {
        return Err(AuthError::Guest);
    }
    let (b58, n) = sol.key.to_base58();
    let addr = core::str::from_utf8(&b58[..n]).map_err(|_| AuthError::Malformed)?;
    let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &sol.key.0);
    let pairs = domains
        .iter()
        .flat_map(|d| domains.iter().map(move |u| (d.as_ref(), u.as_ref())));
    for (domain, uri) in pairs {
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        let len = protocol::siws_message_for(
            domain,
            uri,
            addr,
            protocol::SLUG,
            nonce,
            issued_at,
            &mut text,
        )
        .min(SIWE_MESSAGE_MAX);
        if key.verify(&text[..len], &sol.signature.0).is_ok() {
            return Ok(sol.key);
        }
    }
    Err(AuthError::WrongSigner)
}

/// Who an Elo Pros pass says the player is (v151).
#[derive(Debug)]
pub struct Passed {
    /// `acct_` and 20 hex: the save key of a player whose account links no
    /// wallet.
    pub account: PlayerKey,
    /// The wallet the account links, lowercase on the wire. With one, the
    /// player IS that wallet: the same save, copy, skins and payouts as
    /// signing in with it, and nothing asked it to sign.
    pub wallet: Option<Address>,
}

/// Verify an **Elo Pros pass** (v151): what elo hands a player signed in on
/// its site, in place of a wallet signature. `base64url(text).base64url(sig)`,
/// an Ed25519 signature by elo's account key over the text's bytes, and the
/// text exactly:
///
/// ```text
/// elo account
/// id: acct_<20 hex>
/// for: play:gates
/// shard: <a name this shard answers to>
/// nonce: <this join's challenge nonce, lowercase hex>
/// wallet: <0x… lowercase, or none>
/// exp: <unix seconds>
/// ```
///
/// `shard` is checked against the same names a SIWE proof binds to, and
/// `nonce` against this join's challenge, for SIWE's reasons: a pass a player
/// handed another server, or this one for an earlier join, is no good here.
/// `now` is this shard's clock (the challenge's `Issued At`). The vector is the
/// platform's `sdk/testdata/account_token.json`, pinned by the test below.
pub fn verify_pass<S: AsRef<str>>(
    pass: &[u8],
    key: &[u8; 32],
    domains: &[S],
    nonce: &[u8; NONCE_BYTES],
    now: u64,
) -> Result<Passed, AuthError> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let text = core::str::from_utf8(pass).map_err(|_| AuthError::Malformed)?;
    let (body, sig) = text.split_once('.').ok_or(AuthError::Malformed)?;
    let body = b64.decode(body).map_err(|_| AuthError::Malformed)?;
    let sig = b64.decode(sig).map_err(|_| AuthError::Malformed)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(&body, &sig)
        .map_err(|_| AuthError::WrongSigner)?;
    let body = core::str::from_utf8(&body).map_err(|_| AuthError::Malformed)?;
    let mut lines = body.split('\n');
    if lines.next() != Some("elo account") {
        return Err(AuthError::Malformed);
    }
    let mut take = |name: &str| {
        lines
            .next()
            .and_then(|l| l.strip_prefix(name))
            .and_then(|l| l.strip_prefix(": "))
            .ok_or(AuthError::Malformed)
    };
    let id = take("id")?;
    let game = take("for")?;
    let shard = take("shard")?;
    let said = take("nonce")?;
    let wallet = take("wallet")?;
    let exp = take("exp")?;
    if lines.next().is_some() {
        return Err(AuthError::Malformed);
    }
    if !is_account_str(id) {
        return Err(AuthError::Malformed);
    }
    let exp: u64 = exp.parse().map_err(|_| AuthError::Malformed)?;
    let wallet = match wallet {
        "none" => None,
        w if w.len() == 42 && w.bytes().all(|b| !b.is_ascii_uppercase()) => {
            Some(Address::from_hex(w.as_bytes()).ok_or(AuthError::Malformed)?)
        }
        _ => return Err(AuthError::Malformed),
    };
    if game.strip_prefix("play:") != Some(protocol::SLUG)
        || !domains.iter().any(|d| d.as_ref() == shard)
    {
        return Err(AuthError::PassElsewhere);
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let ours = nonce
        .iter()
        .flat_map(|b| [HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]);
    if said.len() != NONCE_BYTES * 2 || !said.bytes().eq(ours) {
        return Err(AuthError::PassReplayed);
    }
    if exp <= now {
        return Err(AuthError::PassExpired);
    }
    Ok(Passed {
        account: PlayerKey::new(id.as_bytes()).ok_or(AuthError::Malformed)?,
        wallet: wallet.filter(|a| !a.is_guest()),
    })
}

/// Whether a save key is an Elo Pros account's (`acct_…`) rather than a
/// wallet's: it has no copy to check, no wallet to read skins from, and what
/// it unlocks is held on the platform until it links one.
pub fn is_account_key(key: &PlayerKey) -> bool {
    core::str::from_utf8(key.as_bytes()).is_ok_and(is_account_str)
}

/// [`is_account_key`] over a key's text: `acct_` and 20 lowercase hex.
pub fn is_account_str(key: &str) -> bool {
    key.len() == 25
        && key.starts_with("acct_")
        && key[5..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The save key for a Solana account that is linked to no Ethereum one: its
/// base58 address, exactly as written. It cannot collide with an Ethereum key
/// (those start `0x`, and base58 has no `0`), and it never changes — the
/// property `key_of`'s spelling exists for.
pub fn solana_key_of(key: &protocol::SolanaKey) -> Option<PlayerKey> {
    let (b58, n) = key.to_base58();
    PlayerKey::new(&b58[..n])
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

    /// The platform's vector (`sdk/testdata/account_token.json` in
    /// AnthonE/scry-forge, minted by `meter/accounts.py` with a test key):
    /// the same pass, byte for byte, must verify here, or every Elo Pros
    /// join fails with both sides looking right.
    const PASS_KEY_B64: &str = "H0SHD2PwtN2Lb2tZhyrcrQCHGFIs+qy9VvR4aISFwlE=";
    const PASS_WITH_WALLET: &str = "ZWxvIGFjY291bnQKaWQ6IGFjY3RfMDEyMzQ1Njc4OWFiY2RlZjAxMjMKZm9yOiBwbGF5OmdhdGVzCnNoYXJkOiBlbG9wcm9zLmNvbQpub25jZTogMDAwMTAyMDMwNDA1MDYwNzA4MDkwYTBiMGMwZDBlMGYxMDExMTIxMzE0MTUxNjE3MTgxOTFhMWIxYzFkMWUxZgp3YWxsZXQ6IDB4MjQ0NDVlZmRkYjA4ZDQ5MzhlM2UzNjI3MDQyYjJjZjQwNjNkOTA5MgpleHA6IDE4MDAwMDAwMDA.YbX6fWS4o2cHhFNXK6sExcLsm1M53Y0vlpH5clP4BDcFMZGEXP-sPCa39s70wgikSQvIi9viXHJ3KrRoSb0TBw";
    const PASS_NO_WALLET: &str = "ZWxvIGFjY291bnQKaWQ6IGFjY3RfMDEyMzQ1Njc4OWFiY2RlZjAxMjMKZm9yOiBwbGF5OmdhdGVzCnNoYXJkOiBlbG9wcm9zLmNvbQpub25jZTogMDAwMTAyMDMwNDA1MDYwNzA4MDkwYTBiMGMwZDBlMGYxMDExMTIxMzE0MTUxNjE3MTgxOTFhMWIxYzFkMWUxZgp3YWxsZXQ6IG5vbmUKZXhwOiAxODAwMDAwMDAw.eOk7HA9cxLiFGRkyVXgITBLcWitPOavP-i3h_beyuuLhRMSRaRXWP8vGGs5hPojdht0KFvZN9ZVb5MPYSuVEAw";
    const PASS_EXP: u64 = 1_800_000_000;
    /// The vector's nonce: bytes 0 to 31.
    const PASS_NONCE: [u8; NONCE_BYTES] = {
        let mut n = [0u8; NONCE_BYTES];
        let mut i = 0;
        while i < NONCE_BYTES {
            n[i] = i as u8;
            i += 1;
        }
        n
    };

    fn pass_key() -> [u8; 32] {
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(PASS_KEY_B64)
            .unwrap();
        raw.try_into().unwrap()
    }

    #[test]
    fn the_platform_pass_vector_verifies_as_its_wallet_or_its_account() {
        let key = pass_key();
        let p = verify_pass(
            PASS_WITH_WALLET.as_bytes(),
            &key,
            &["elopros.com"],
            &PASS_NONCE,
            PASS_EXP - 1,
        )
        .unwrap();
        assert_eq!(p.account.as_bytes(), b"acct_0123456789abcdef0123");
        assert_eq!(
            p.wallet,
            Address::from_hex(b"0x24445efddb08d4938e3e3627042b2cf4063d9092")
        );
        let p = verify_pass(
            PASS_NO_WALLET.as_bytes(),
            &key,
            &["elopros.com"],
            &PASS_NONCE,
            PASS_EXP - 1,
        )
        .unwrap();
        assert!(p.wallet.is_none());
        assert!(is_account_key(&p.account));
    }

    #[test]
    fn a_pass_for_another_shard_another_join_a_stale_one_or_another_key_is_refused() {
        let key = pass_key();
        let pass = PASS_WITH_WALLET.as_bytes();
        let here = ["elopros.com"];
        assert_eq!(
            verify_pass(pass, &key, &["evil.example"], &PASS_NONCE, PASS_EXP - 1).unwrap_err(),
            AuthError::PassElsewhere
        );
        let mut later = PASS_NONCE;
        later[0] ^= 1;
        assert_eq!(
            verify_pass(pass, &key, &here, &later, PASS_EXP - 1).unwrap_err(),
            AuthError::PassReplayed
        );
        assert_eq!(
            verify_pass(pass, &key, &here, &PASS_NONCE, PASS_EXP).unwrap_err(),
            AuthError::PassExpired
        );
        assert_eq!(
            verify_pass(pass, &[7u8; 32], &here, &PASS_NONCE, PASS_EXP - 1).unwrap_err(),
            AuthError::WrongSigner
        );
        // One character of the text moved: the signature no longer covers it.
        let mut moved = PASS_WITH_WALLET.to_string();
        moved.replace_range(10..11, if &moved[10..11] == "A" { "B" } else { "A" });
        assert!(verify_pass(moved.as_bytes(), &key, &here, &PASS_NONCE, PASS_EXP - 1).is_err());
        assert_eq!(
            verify_pass(b"not a pass", &key, &here, &PASS_NONCE, 0).unwrap_err(),
            AuthError::Malformed
        );
    }
    use k256::ecdsa::SigningKey;

    /// A Solana sign-in, signed with a real Ed25519 key over the rebuilt text,
    /// verifies; another key, another shard and another nonce do not.
    #[test]
    fn a_solana_sign_in_verifies_against_the_siws_text_only() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let pair = Ed25519KeyPair::from_seed_unchecked(&[42u8; 32]).unwrap();
        let mut pk = [0u8; 32];
        pk.copy_from_slice(pair.public_key().as_ref());
        let key = protocol::SolanaKey(pk);
        let (b58, n) = key.to_base58();
        let addr = core::str::from_utf8(&b58[..n]).unwrap();
        let nonce = [5u8; NONCE_BYTES];
        let sign = |domain: &str, pair: &Ed25519KeyPair, nonce: &[u8; NONCE_BYTES]| {
            let mut text = [0u8; SIWE_MESSAGE_MAX];
            let len = protocol::siws_message_for(
                domain,
                domain,
                addr,
                protocol::SLUG,
                nonce,
                77,
                &mut text,
            );
            let mut sig = [0u8; 64];
            sig.copy_from_slice(pair.sign(&text[..len]).as_ref());
            Auth {
                address: protocol::Address::GUEST,
                signature: protocol::Signature::NONE,
                solana: Some(protocol::SolanaProof {
                    key,
                    signature: protocol::SolanaSignature(sig),
                }),
                pass: None,
                handoff: None,
            }
        };
        let good = sign(DOMAIN, &pair, &nonce);
        assert_eq!(verify_solana_any(&[DOMAIN], &nonce, 77, &good), Ok(key));
        let other = Ed25519KeyPair::from_seed_unchecked(&[43u8; 32]).unwrap();
        let mut forged = sign(DOMAIN, &other, &nonce);
        forged.solana.as_mut().unwrap().key = key;
        assert_eq!(
            verify_solana_any(&[DOMAIN], &nonce, 77, &forged),
            Err(AuthError::WrongSigner)
        );
        assert!(
            verify_solana_any(&[DOMAIN], &[6u8; NONCE_BYTES], 77, &good).is_err(),
            "another nonce"
        );
        assert!(
            verify_solana_any(&["other.shard"], &nonce, 77, &good).is_err(),
            "another shard"
        );
        // an Ethereum verifier never takes a Solana proof
        assert_eq!(
            verify_any(&[DOMAIN], &nonce, 77, &good),
            Err(AuthError::Guest)
        );
        let k = solana_key_of(&key).unwrap();
        assert_eq!(
            k.as_bytes(),
            addr.as_bytes(),
            "the save key is the base58 address, as written"
        );
    }

    const DOMAIN: &str = "gates.test";

    fn signer_for(byte: u8) -> SigningKey {
        let mut sk = [0u8; 32];
        sk[31] = byte;
        SigningKey::from_bytes(&sk.into()).expect("a small scalar is a valid key")
    }

    /// Sign the challenge exactly as a wallet would, so these tests exercise
    /// the real path rather than a convenient one.
    fn sign_as(sk: &SigningKey, nonce: &[u8; NONCE_BYTES], issued_at: u64, v_style: u8) -> Auth {
        sign_for(sk, DOMAIN, nonce, issued_at, v_style)
    }

    /// [`sign_as`] for a page on another name.
    fn sign_for(
        sk: &SigningKey,
        domain: &str,
        nonce: &[u8; NONCE_BYTES],
        issued_at: u64,
        v_style: u8,
    ) -> Auth {
        let address = address_of(sk.verifying_key());
        let mut text = [0u8; SIWE_MESSAGE_MAX];
        // Checksummed, exactly as `verify` recomputes it and a launcher signs
        // it — a test that signed the lowercase form would pass against a
        // verifier that did the same and fail against every real wallet.
        let checksummed = address.to_checksum_hex();
        let addr = core::str::from_utf8(&checksummed).expect("utf-8");
        let len = protocol::siwe_message(domain, addr, protocol::SLUG, nonce, issued_at, &mut text);
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
            solana: None,
            pass: None,
            handoff: None,
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
                solana: None,
                pass: None,
                handoff: None,
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

    /// **A shard with two names takes a proof bound to either, and to
    /// nothing else.** The second name is a page's (`elopros.com`) beside the
    /// dialled one, and the refusal is the half that matters: widening the
    /// list by one name must not widen it by any other.
    #[test]
    fn a_listed_name_verifies_and_an_unlisted_one_is_refused() {
        let listed = ["game.test", "test"];
        let sk = signer_for(12);
        let nonce = [6u8; NONCE_BYTES];
        for domain in listed {
            let auth = sign_for(&sk, domain, &nonce, 9, 27);
            assert_eq!(
                verify_any(&listed, &nonce, 9, &auth),
                Ok(key_of(&auth.address).expect("a key")),
                "a proof bound to the listed `{domain}` was refused"
            );
        }
        for domain in ["evil.test", "sub.game.test", "game.test.", "TEST", ""] {
            let auth = sign_for(&sk, domain, &nonce, 9, 27);
            assert_eq!(
                verify_any(&listed, &nonce, 9, &auth),
                Err(AuthError::WrongSigner),
                "a proof bound to the unlisted `{domain}` was accepted"
            );
        }
    }

    /// **A page's proof names the shard it was collected for.** A page on
    /// `test` signs its own name in the domain line for any shard under it;
    /// the `URI:` line is the dialled shard, so `game.test` takes the proof a
    /// page collected for it and refuses the one collected for `play.test` —
    /// the sibling that would otherwise pass a player's sign-in straight on.
    #[test]
    fn a_page_proof_for_a_sibling_shard_is_refused() {
        let listed = ["game.test", "test"];
        let sk = signer_for(15);
        let nonce = [8u8; NONCE_BYTES];
        let page = |uri: &str| {
            let address = address_of(sk.verifying_key());
            let sum = address.to_checksum_hex();
            let addr = core::str::from_utf8(&sum).expect("utf-8");
            let mut text = [0u8; SIWE_MESSAGE_MAX];
            let len =
                protocol::siwe_message_for("test", uri, addr, protocol::SLUG, &nonce, 9, &mut text);
            let digest = personal_sign_digest(&text[..len]);
            let (sig, rec) = sk.sign_prehash_recoverable(&digest).expect("signs");
            let mut raw = [0u8; 65];
            raw[..64].copy_from_slice(&sig.to_bytes());
            raw[64] = 27 + rec.to_byte();
            Auth {
                address,
                signature: Signature(raw),
                solana: None,
                pass: None,
                handoff: None,
            }
        };
        assert!(verify_any(&listed, &nonce, 9, &page("game.test")).is_ok());
        assert_eq!(
            verify_any(&listed, &nonce, 9, &page("play.test")),
            Err(AuthError::WrongSigner),
            "a proof a page collected for a sibling shard was accepted"
        );
    }

    /// One name is `verify`, refusal for refusal — the shard without
    /// `siwe_domains` is the shard before it.
    #[test]
    fn one_listed_name_is_verify_exactly() {
        let sk = signer_for(13);
        let nonce = [4u8; NONCE_BYTES];
        let good = sign_for(&sk, DOMAIN, &nonce, 5, 27);
        let mut bad_v = good;
        bad_v.signature.0[64] = 30;
        let mut zero_sig = good;
        zero_sig.signature.0[..64].fill(0);
        let mut wrong = good;
        wrong.address = address_of(signer_for(14).verifying_key());
        for auth in [good, bad_v, zero_sig, wrong, Auth::default()] {
            assert_eq!(
                verify_any(&[DOMAIN], &nonce, 5, &auth),
                verify(DOMAIN, &nonce, 5, &auth)
            );
        }
        assert_eq!(
            verify_any::<&str>(&[], &nonce, 5, &good),
            Err(AuthError::WrongSigner),
            "an empty list proves nothing"
        );
    }
}
