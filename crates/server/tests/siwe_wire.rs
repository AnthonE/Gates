//! **Identity over a real socket.** A signature made the way a wallet makes
//! one, carried by the real handshake, verified by the real shard.
//!
//! `server::auth`'s unit tests own the arithmetic and `protocol::auth`'s own
//! the message bytes. This owns the thing neither can see: that the two
//! halves agree *across the wire*, where a single disagreement about what
//! gets signed — one space, one field order, the address the client used to
//! build the text versus the one the server rebuilt it with — refuses every
//! login while both sides look correct in isolation.
//!
//! That failure is not hypothetical. The first version of the shared client
//! handshake built the SIWE text with `Address::GUEST` and then asked for a
//! signature over it, so the server (which rebuilds from the *claimed*
//! address) would have computed a different digest and refused everybody as
//! `WrongSigner` — with the crypto, the nonce and the domain binding all
//! correct. Only a test that runs both halves catches that.

use k256::ecdsa::SigningKey;
use server::config::ShardConfig;
use server::net::{client_handshake, spawn_shard};
use server::stats::ShardStats;
use server::store::Saves;
use std::time::Duration;
use tiny_keccak::{Hasher, Keccak};

use server::botclient::bot_endpoint;

/// The shipped content, baked. A copy of `bot_smoke`'s helper rather than a
/// shared module: two integration test binaries cannot share a `mod` without
/// a file that belongs to neither, and this is nine lines.
fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&c).expect("shipped content bakes");
    // A test shard spawns naked: a suite that asserted on a fresh
    // inventory would be asserting on content instead of on code.
    //
    // The reason used to be that the shipped `[[spawn_kit]]` was testing
    // scaffolding. Since 2026-08-15 it is a rock and a torch and is the
    // game (DECISIONS.md), so the scaffolding half of that argument is
    // gone — the OTHER half is what this line always rested on and it is
    // unchanged: these suites count snapshot bytes and handshake
    // outcomes, and a kit edit must not move either.
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

const DOMAIN: &str = "127.0.0.1";

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut k = Keccak::v256();
    let mut out = [0u8; 32];
    k.update(data);
    k.finalize(&mut out);
    out
}

/// The address for a signing key, derived the way `server::auth` does.
fn address_of(sk: &SigningKey) -> protocol::Address {
    let pt = sk.verifying_key().to_encoded_point(false);
    let h = keccak(&pt.as_bytes()[1..]);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    protocol::Address(a)
}

/// Sign like a wallet: EIP-191 envelope, keccak, recoverable ECDSA, v+27.
fn wallet_sign(sk: &SigningKey, text: &str) -> protocol::Signature {
    let prefixed = [
        format!("\x19Ethereum Signed Message:\n{}", text.len()).as_bytes(),
        text.as_bytes(),
    ]
    .concat();
    let (sig, rec) = sk
        .sign_prehash_recoverable(&keccak(&prefixed))
        .expect("signs");
    let mut raw = [0u8; 65];
    raw[..64].copy_from_slice(&sig.to_bytes());
    raw[64] = 27 + rec.to_byte();
    protocol::Signature(raw)
}

fn key(byte: u8) -> SigningKey {
    let mut b = [0u8; 32];
    b[31] = byte;
    SigningKey::from_bytes(&b.into()).expect("valid")
}

async fn shard(require_auth: bool) -> server::net::ShardHandle {
    shard_listing(require_auth, None).await
}

/// A shard that also answers to `siwe_domains` — `None` is the key absent.
async fn shard_listing(
    require_auth: bool,
    siwe_domains: Option<&[&str]>,
) -> server::net::ShardHandle {
    let tables = baked_content();
    let mut cfg = ShardConfig::ephemeral(20_260_807);
    cfg.require_auth = require_auth;
    cfg.domain = DOMAIN.into();
    cfg.siwe_domains = siwe_domains.map(|l| l.iter().map(|d| d.to_string()).collect());
    spawn_shard(
        cfg,
        tables,
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

/// **The whole scheme over a socket.** A real signature is admitted, and the
/// shard is one that refuses guests — so the admission is the proof.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_join_is_admitted_by_a_shard_that_refuses_guests() {
    let handle = shard(true).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    let sk = key(7);
    let welcome = tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(&mut send, &mut recv, DOMAIN, address_of(&sk), |text| {
            Some(wallet_sign(&sk, std::str::from_utf8(text).expect("utf-8")))
        }),
    )
    .await
    .expect("inside 5 s")
    .expect("a real signature must be admitted");
    assert_ne!(welcome.player_id, 0, "welcomed with no player id");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 0);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A guest is refused where identity is required — the other half of the
/// same claim, and what makes the test above mean something.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_is_refused_where_identity_is_required() {
    let handle = shard(true).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    let err = tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(
            &mut send,
            &mut recv,
            DOMAIN,
            protocol::Address::GUEST,
            |_| None,
        ),
    )
    .await
    .expect("inside 5 s")
    .expect_err("a guest must be refused");
    assert!(err.contains("refused"), "{err}");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **The impersonation attempt, over the wire.** A real signature from one
/// key, presented under someone else's address. This is what the whole
/// handshake exists to refuse, and before SIWE it was simply admitted —
/// anyone who knew your name was you.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signature_under_someone_elses_address_is_refused() {
    let handle = shard(true).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    let victim = address_of(&key(1));
    let attacker = key(2);
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        // Claim the victim's address; sign with the attacker's key. The
        // signature is perfectly valid — for somebody else.
        client_handshake(&mut send, &mut recv, DOMAIN, victim, |text| {
            Some(wallet_sign(
                &attacker,
                std::str::from_utf8(text).expect("utf-8"),
            ))
        }),
    )
    .await
    .expect("inside 5 s")
    .expect_err("impersonation must be refused");
    assert!(err.contains("refused"), "{err}");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A signature for a **different domain** is refused, which is what stops
/// one shard collecting signatures it can present at another.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signature_for_another_shard_is_refused() {
    let handle = shard(true).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    let sk = key(3);
    // The client signs for `evil.example` while talking to this shard, which
    // rebuilds the message with its own domain.
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(
            &mut send,
            &mut recv,
            "evil.example",
            address_of(&sk),
            |text| Some(wallet_sign(&sk, std::str::from_utf8(text).expect("utf-8"))),
        ),
    )
    .await
    .expect("inside 5 s")
    .expect_err("a foreign domain must be refused");
    assert!(err.contains("refused"), "{err}");
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A guest still plays where guests are taken — the dev flow, every test in
/// this repo, and the bots.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_still_plays_where_guests_are_taken() {
    let handle = shard(false).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(
            &mut send,
            &mut recv,
            DOMAIN,
            protocol::Address::GUEST,
            |_| None,
        ),
    )
    .await
    .expect("inside 5 s")
    .expect("a guest must be welcomed on a guest shard");
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Letting guests in does not let impostors in.** On a shard that takes
/// guests, a joiner who claims an address still has to hold its key. This is
/// what makes `require_auth = false` safe on a shard that saves players
/// (`shard-public.toml`): a guest gets a fresh body, never somebody's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signature_under_someone_elses_address_is_refused_where_guests_are_taken() {
    let handle = shard(false).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    let victim = address_of(&key(1));
    let attacker = key(2);
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(&mut send, &mut recv, DOMAIN, victim, |text| {
            Some(wallet_sign(
                &attacker,
                std::str::from_utf8(text).expect("utf-8"),
            ))
        }),
    )
    .await
    .expect("inside 5 s")
    .expect_err("impersonation must be refused on a guest shard too");
    assert!(err.contains("refused"), "{err}");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// And a real signature is still admitted there, beside the guests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_join_is_admitted_where_guests_are_taken() {
    let handle = shard(false).await;
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");

    let sk = key(8);
    let welcome = tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(&mut send, &mut recv, DOMAIN, address_of(&sk), |text| {
            Some(wallet_sign(&sk, std::str::from_utf8(text).expect("utf-8")))
        }),
    )
    .await
    .expect("inside 5 s")
    .expect("a real signature must be admitted on a guest shard");
    assert_ne!(welcome.player_id, 0, "welcomed with no player id");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 0);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

// ── a shard with more than one name ──────────────────────────────────────────
//
// `siwe_domains` lets a shard accept a proof bound to a name beside `domain`:
// the browser page signs its own host (`elopros.com`) so a wallet does not
// warn that the page and the message disagree. Each case below is a real
// signature over the real handshake, and the refusals are the half that
// matter: one more name must not mean any name.

/// The page's name, as the public shard lists it (`shard-public.toml`).
const PAGE: &str = "elopros.com";

/// Join with a proof bound to `signed_for`, signed by `sk`, through the real
/// client handshake — which signs whatever domain it is handed, so a test can
/// sign the name a page would.
async fn join_signed_for(
    handle: &server::net::ShardHandle,
    signed_for: &str,
    sk: &SigningKey,
) -> Result<protocol::Welcome, String> {
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");
    tokio::time::timeout(
        Duration::from_secs(5),
        client_handshake(&mut send, &mut recv, signed_for, address_of(sk), |text| {
            Some(wallet_sign(sk, std::str::from_utf8(text).expect("utf-8")))
        }),
    )
    .await
    .expect("inside 5 s")
}

/// **A proof bound to a listed name is admitted**, and the shard is one that
/// refuses guests, so the admission is the proof. Both names: the page's and
/// the dialled one a desktop client signs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proof_bound_to_a_listed_name_is_admitted() {
    let handle = shard_listing(true, Some(&[DOMAIN, PAGE])).await;
    for (byte, name) in [(21u8, PAGE), (22u8, DOMAIN)] {
        let welcome = join_signed_for(&handle, name, &key(byte))
            .await
            .unwrap_or_else(|e| panic!("a proof bound to the listed `{name}` was refused: {e}"));
        assert_ne!(welcome.player_id, 0, "welcomed with no player id");
    }
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 0);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **A name outside the list is refused**, on a shard that takes guests —
/// so the refusal is the signature's, not the guest door's. The near misses
/// are the ones a sloppy match would let through: a parent of a listed name,
/// a child of one, another case, a trailing dot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proof_bound_to_an_unlisted_name_is_refused() {
    let handle = shard_listing(false, Some(&[DOMAIN, PAGE])).await;
    let unlisted = [
        "evil.example",
        "game.elopros.com",
        "com",
        "ELOPROS.COM",
        "elopros.com.",
    ];
    for (i, name) in unlisted.iter().enumerate() {
        let err = join_signed_for(&handle, name, &key(30 + i as u8))
            .await
            .expect_err("a proof bound to an unlisted name must be refused");
        assert!(err.contains("refused"), "{name}: {err}");
    }
    assert_eq!(
        ShardStats::get(&handle.stats.refused_auth),
        unlisted.len() as u64
    );
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Without the key, the shard is the one-name shard it was**: the dialled
/// name is admitted and the page's is refused, exactly as before
/// `siwe_domains` existed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_key_only_the_domain_is_accepted() {
    let cfg = ShardConfig::ephemeral(1);
    assert_eq!(cfg.siwe_domains, None);
    assert_eq!(cfg.siwe_accepts(), std::slice::from_ref(&cfg.domain));

    let handle = shard_listing(false, None).await;
    join_signed_for(&handle, DOMAIN, &key(40))
        .await
        .expect("the dialled name is admitted as it always was");
    let err = join_signed_for(&handle, PAGE, &key(41))
        .await
        .expect_err("a name the operator never listed must be refused");
    assert!(err.contains("refused"), "{err}");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **The public shard accepts the name its browser page signs, and the name
/// its desktop clients sign.** The page's rule (`page_domain`) and the
/// shard's list (`shard-public.toml`) live in two crates and one config
/// file; this is the one place that holds them to agreeing, through the
/// shard's real verifier.
#[test]
fn the_public_shard_accepts_what_its_page_and_its_desktop_sign() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../shard-public.toml");
    let text = std::fs::read_to_string(&path).expect("shard-public.toml is tracked");
    let cfg = server::config::parse_shard_toml(&text).expect("shard-public.toml parses");
    let dialled = format!("{}:61234", cfg.domain);
    let page = client::net::handshake::page_domain("https:", PAGE, &dialled)
        .expect("the page on elopros.com vouches for the shard under it");

    let sk = key(42);
    let address = address_of(&sk);
    let nonce = [0x33u8; protocol::NONCE_BYTES];
    let issued_at = 1_700_000_000;
    for (who, domain) in [
        ("the page", page),
        ("a desktop client", cfg.domain.as_str()),
    ] {
        let proof = client::net::handshake::Proof {
            domain: domain.to_string(),
            uri: cfg.domain.clone(),
            nonce,
            issued_at,
        };
        let auth = protocol::Auth {
            address,
            signature: wallet_sign(&sk, &proof.message(address)),
            solana: None,
            pass: None,
        };
        assert!(
            server::auth::verify_any(cfg.siwe_accepts(), &nonce, issued_at, &auth).is_ok(),
            "the public shard refuses what {who} signs (`{domain}`)"
        );
    }
    // ...and refuses what the same page signs for a sibling under its name:
    // the proof a relay would carry from that shard to this one.
    let sibling = client::net::handshake::Proof {
        domain: page.to_string(),
        uri: "play.elopros.com".to_string(),
        nonce,
        issued_at,
    };
    let auth = protocol::Auth {
        address,
        signature: wallet_sign(&sk, &sibling.message(address)),
        solana: None,
        pass: None,
    };
    assert!(
        server::auth::verify_any(cfg.siwe_accepts(), &nonce, issued_at, &auth).is_err(),
        "the public shard accepted a page's proof collected for another shard"
    );
}

// ── the browser's half ───────────────────────────────────────────────────────
//
// Everything above signs text composed by `server::net::client_handshake`.
// A page cannot use that: `window.ethereum` has no `prove` verb, so the text
// is built in the client crate (`client::net::handshake::Proof::message`) and
// handed to the wallet as a finished string. That is a SECOND construction of
// the same bytes, which is the shape this file's own header says refuses every
// login while both sides look correct alone — so it is gated here, against the
// shard's real verifier rather than against a rebuild of it.

/// **A signature over the text a browser wallet is shown verifies at the
/// shard.** The whole browser identity path in one assertion.
///
/// `Proof::message` is what a page hands `personal_sign`; `wallet_sign` is
/// EIP-191 exactly as a wallet does it; `server::auth::verify` is the shard's
/// own function, unmodified. Nothing here rebuilds the message a second time,
/// which is the trap `CLAUDE.md` records under `lattice.rs`: a naive rebuild
/// that calls the thing under test carries the same mutant on both sides.
#[test]
fn a_signature_over_the_browsers_own_text_verifies_at_the_shard() {
    let sk = key(11);
    let address = address_of(&sk);
    let nonce = [0x5Au8; protocol::NONCE_BYTES];
    let issued_at = 1_700_000_000;

    let proof = client::net::handshake::Proof {
        domain: DOMAIN.to_string(),
        uri: DOMAIN.to_string(),
        nonce,
        issued_at,
    };
    let auth = protocol::Auth {
        address,
        signature: wallet_sign(&sk, &proof.message(address)),
        solana: None,
        pass: None,
    };
    assert!(
        server::auth::verify(DOMAIN, &nonce, issued_at, &auth).is_ok(),
        "the shard must accept a signature over the text a page shows its wallet"
    );
}

/// **The case in the address is inside the signed bytes, and this proves it.**
///
/// EIP-4361 wants the EIP-55 spelling and the shard rebuilds with it
/// (`Address::to_checksum_hex`), so a page that signed the lowercase form
/// recovers a different digest and is refused — as `WrongSigner`, which reads
/// as "somebody else signed this" and not as "one character is the wrong
/// case". That is the whole reason `to_checksum_hex` moved into `protocol`
/// instead of being reimplemented for the browser.
///
/// The mutant, run rather than described: build the message the wrong way and
/// watch the real verifier refuse it.
#[test]
fn the_lowercase_address_is_refused_which_is_why_the_checksum_is_shared() {
    let sk = key(11);
    let address = address_of(&sk);
    let nonce = [0x5Au8; protocol::NONCE_BYTES];
    let issued_at = 1_700_000_000;

    let lower = address.to_hex();
    let mut text = [0u8; protocol::SIWE_MESSAGE_MAX];
    let n = protocol::siwe_message(
        DOMAIN,
        std::str::from_utf8(&lower).unwrap(),
        protocol::SLUG,
        &nonce,
        issued_at,
        &mut text,
    );
    let auth = protocol::Auth {
        address,
        signature: wallet_sign(&sk, std::str::from_utf8(&text[..n]).unwrap()),
        solana: None,
        pass: None,
    };
    assert_eq!(
        server::auth::verify(DOMAIN, &nonce, issued_at, &auth),
        Err(server::auth::AuthError::WrongSigner),
        "a lowercase address must not verify — if it does, the case stopped \
         reaching the signed bytes and every wallet spelling would pass"
    );
}

/// The domain binding, from the browser's side: a page that signed for one
/// shard cannot present it at another. Same rule as
/// `a_signature_for_another_shard_is_refused` above, over the text a wallet
/// actually sees.
#[test]
fn the_browsers_text_is_bound_to_the_shard_it_was_collected_for() {
    let sk = key(11);
    let address = address_of(&sk);
    let nonce = [0x5Au8; protocol::NONCE_BYTES];
    let issued_at = 1_700_000_000;

    let elsewhere = client::net::handshake::Proof {
        domain: "evil.example".to_string(),
        uri: "evil.example".to_string(),
        nonce,
        issued_at,
    };
    let auth = protocol::Auth {
        address,
        signature: wallet_sign(&sk, &elsewhere.message(address)),
        solana: None,
        pass: None,
    };
    assert_eq!(
        server::auth::verify(DOMAIN, &nonce, issued_at, &auth),
        Err(server::auth::AuthError::WrongSigner)
    );
}

// ── a Solana account (v142) ──────────────────────────────────────────────────

/// Join as a Solana account through the real handshake: a real Ed25519
/// signature over `protocol::siws_message_for`, bound to `domain`.
async fn join_solana(
    handle: &server::net::ShardHandle,
    domain: &str,
    seed: u8,
    forge: bool,
) -> Result<protocol::Welcome, String> {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("a seed");
    let mut pk = [0u8; 32];
    pk.copy_from_slice(pair.public_key().as_ref());
    let key = protocol::SolanaKey(pk);
    // a forged join signs with another key under this one's name
    let signer = Ed25519KeyPair::from_seed_unchecked(&[seed ^ if forge { 0xff } else { 0 }; 32])
        .expect("a seed");
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");
    tokio::time::timeout(
        Duration::from_secs(5),
        server::net::client_handshake_with(
            &mut send,
            &mut recv,
            &protocol::Hello::this_build(),
            |c| {
                let (b58, n) = key.to_base58();
                let addr = std::str::from_utf8(&b58[..n]).expect("ascii");
                let mut text = [0u8; protocol::SIWE_MESSAGE_MAX];
                let len = protocol::siws_message_for(
                    domain,
                    domain,
                    addr,
                    protocol::SLUG,
                    &c.nonce,
                    c.issued_at,
                    &mut text,
                );
                let mut sig = [0u8; protocol::SOLANA_SIGNATURE_BYTES];
                sig.copy_from_slice(signer.sign(&text[..len]).as_ref());
                protocol::Auth {
                    address: protocol::Address::GUEST,
                    signature: protocol::Signature::NONE,
                    solana: Some(protocol::SolanaProof {
                        key,
                        signature: protocol::SolanaSignature(sig),
                    }),
                    pass: None,
                }
            },
        ),
    )
    .await
    .expect("inside 5 s")
    .map(|j| j.welcome)
}

/// **A Solana account joins a shard that refuses guests**, so the admission is
/// the proof; a signature by another key under its name, or one bound to
/// another shard, is refused as `REFUSE_AUTH`. A shard with no origin asks
/// nobody about links, so the account is its own player.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_solana_account_joins_and_a_forged_one_does_not() {
    let handle = shard(true).await;
    let welcome = join_solana(&handle, DOMAIN, 31, false)
        .await
        .expect("a real Solana signature is admitted");
    assert_ne!(welcome.player_id, 0);
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 0);

    let forged = join_solana(&handle, DOMAIN, 32, true).await;
    assert!(forged.is_err(), "a signature by another key was admitted");
    let elsewhere = join_solana(&handle, "another.shard", 33, false).await;
    assert!(
        elsewhere.is_err(),
        "a proof bound to another shard was admitted"
    );
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 2);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **The browser's own Solana text verifies at the shard** — the twin of
/// `a_signature_over_the_browsers_own_text_verifies_at_the_shard`: the page
/// hands `Proof::solana_message` to the wallet, and the shard's verifier,
/// unmodified, takes the signature over it for the page's name and refuses it
/// for a sibling's.
#[test]
fn a_solana_signature_over_the_browsers_own_text_verifies_at_the_shard() {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let pair = Ed25519KeyPair::from_seed_unchecked(&[44u8; 32]).unwrap();
    let mut pk = [0u8; 32];
    pk.copy_from_slice(pair.public_key().as_ref());
    let key = protocol::SolanaKey(pk);
    let nonce = [0x44u8; protocol::NONCE_BYTES];
    let sign = |uri: &str| {
        let proof = client::net::handshake::Proof {
            domain: PAGE.to_string(),
            uri: uri.to_string(),
            nonce,
            issued_at: 1_700_000_000,
        };
        let mut sig = [0u8; protocol::SOLANA_SIGNATURE_BYTES];
        sig.copy_from_slice(pair.sign(proof.solana_message(key).as_bytes()).as_ref());
        protocol::Auth {
            address: protocol::Address::GUEST,
            signature: protocol::Signature::NONE,
            solana: Some(protocol::SolanaProof {
                key,
                signature: protocol::SolanaSignature(sig),
            }),
            pass: None,
        }
    };
    let names = [PAGE, DOMAIN];
    assert_eq!(
        server::auth::verify_solana_any(&names, &nonce, 1_700_000_000, &sign(DOMAIN)),
        Ok(key)
    );
    assert!(server::auth::verify_solana_any(
        &names,
        &nonce,
        1_700_000_000,
        &sign("play.elopros.com")
    )
    .is_err());
}

// ── an Elo Pros pass (v151) ──────────────────────────────────────────────────

/// elo's account key in these tests; the shard pins its public half.
const ELO_SEED: [u8; 32] = [61; 32];

/// A pass as `meter/accounts.py` issues one, signed by `seed`.
fn issue_pass(seed: &[u8; 32], shard: &str, nonce: &[u8; protocol::NONCE_BYTES]) -> String {
    use base64::Engine;
    use ring::signature::Ed25519KeyPair;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let text = format!(
        "elo account\nid: acct_0123456789abcdef0123\nfor: play:{}\nshard: {shard}\n\
         nonce: {nonce}\nwallet: none\nexp: 4000000000",
        protocol::SLUG
    );
    let pair = Ed25519KeyPair::from_seed_unchecked(seed).expect("a seed");
    format!(
        "{}.{}",
        b64.encode(&text),
        b64.encode(pair.sign(text.as_bytes()))
    )
}

/// A shard that refuses guests, pinning elo's key when `pinned`.
async fn shard_taking_passes(pinned: bool) -> server::net::ShardHandle {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let mut cfg = ShardConfig::ephemeral(20_260_807);
    cfg.require_auth = true;
    cfg.domain = DOMAIN.into();
    if pinned {
        let pair = Ed25519KeyPair::from_seed_unchecked(&ELO_SEED).expect("a seed");
        cfg.elo_account_key = Some(pair.public_key().as_ref().try_into().expect("32 bytes"));
    }
    spawn_shard(
        cfg,
        baked_content(),
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

/// Join with the pass `pass_for` makes from this join's challenge; it also
/// hands the pass back, so a test can replay it.
async fn join_pass(
    handle: &server::net::ShardHandle,
    pass_for: impl FnOnce(&protocol::Challenge) -> String,
) -> (Result<protocol::Welcome, String>, String) {
    let endpoint = bot_endpoint().expect("endpoint");
    let connection = endpoint
        .connect(&format!("https://{}", handle.local_addr))
        .await
        .expect("connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open_bi")
        .await
        .expect("bi");
    let mut sent = String::new();
    let joined = tokio::time::timeout(
        Duration::from_secs(5),
        server::net::client_handshake_with(
            &mut send,
            &mut recv,
            &protocol::Hello::this_build(),
            |c| {
                sent = pass_for(c);
                protocol::Auth {
                    pass: protocol::Pass::new(sent.as_bytes()),
                    ..protocol::Auth::default()
                }
            },
        ),
    )
    .await
    .expect("inside 5 s")
    .map(|j| j.welcome);
    (joined, sent)
}

/// **A player signed in to Elo Pros joins a shard that refuses guests** with
/// the pass elo issued for this shard and this join, and nothing signs. The
/// same pass at a later join is refused (its nonce was this join's), as is a
/// pass for another shard and one signed by any key but elo's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pass_joins_once_and_only_here() {
    let handle = shard_taking_passes(true).await;
    let (welcome, pass) = join_pass(&handle, |c| issue_pass(&ELO_SEED, DOMAIN, &c.nonce)).await;
    assert_ne!(welcome.expect("elo's pass is admitted").player_id, 0);
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 0);

    let (replayed, _) = join_pass(&handle, |_| pass.clone()).await;
    assert!(replayed.is_err(), "a pass was admitted at a second join");
    let (elsewhere, _) =
        join_pass(&handle, |c| issue_pass(&ELO_SEED, "evil.example", &c.nonce)).await;
    assert!(elsewhere.is_err(), "a pass for another shard was admitted");
    let (forged, _) = join_pass(&handle, |c| issue_pass(&[62; 32], DOMAIN, &c.nonce)).await;
    assert!(forged.is_err(), "a pass by another key was admitted");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 3);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A shard that pins no elo key takes no pass, however good.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shard_without_elos_key_takes_no_pass() {
    let handle = shard_taking_passes(false).await;
    let (welcome, _) = join_pass(&handle, |c| issue_pass(&ELO_SEED, DOMAIN, &c.nonce)).await;
    assert!(welcome.is_err(), "a pass was admitted with no key pinned");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
