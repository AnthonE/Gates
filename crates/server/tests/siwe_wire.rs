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
    protocol::Signature::from_ecdsa(raw)
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
            nonce,
            issued_at,
        };
        let auth = protocol::Auth {
            address,
            signature: wallet_sign(&sk, &proof.message(address)),
        };
        assert!(
            server::auth::verify_any(cfg.siwe_accepts(), &nonce, issued_at, &auth).is_ok(),
            "the public shard refuses what {who} signs (`{domain}`)"
        );
    }
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
        nonce,
        issued_at,
    };
    let auth = protocol::Auth {
        address,
        signature: wallet_sign(&sk, &proof.message(address)),
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
        nonce,
        issued_at,
    };
    let auth = protocol::Auth {
        address,
        signature: wallet_sign(&sk, &elsewhere.message(address)),
    };
    assert_eq!(
        server::auth::verify(DOMAIN, &nonce, issued_at, &auth),
        Err(server::auth::AuthError::WrongSigner)
    );
}

// ── smart-contract wallets (v109, `chainsig.rs`) ─────────────────────────────

/// A JSON-RPC stand-in on loopback: answers every POST with `result` and
/// keeps each body it was sent. The chain, without the network.
fn fake_rpc(result: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            log.lock()
                .expect("lock")
                .push(String::from_utf8_lossy(&body).into_owned());
            let reply = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{result}"}}"#);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                reply.len(),
                reply
            );
        }
    });
    (url, seen)
}

async fn shard_with_chain(chain: Option<String>) -> server::net::ShardHandle {
    let tables = baked_content();
    let mut cfg = ShardConfig::ephemeral(20_261_014);
    cfg.require_auth = true;
    cfg.domain = DOMAIN.into();
    if let Some(url) = chain {
        cfg.chain_sig = server::chainsig::Config::new(vec![url]);
    }
    spawn_shard(
        cfg,
        tables,
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

/// Join as `address`, signing with `sign`; the welcome or the refusal.
async fn join_as(
    handle: &server::net::ShardHandle,
    address: protocol::Address,
    sign: impl FnOnce(&[u8]) -> Option<protocol::Signature>,
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
        Duration::from_secs(15),
        client_handshake(&mut send, &mut recv, DOMAIN, address, sign),
    )
    .await
    .expect("inside 15 s")
}

/// What an undeployed Coinbase Smart Wallet sends: ~640 bytes no key made.
fn smart_signature() -> protocol::Signature {
    let bytes: Vec<u8> = (0..640u32).map(|i| (i * 31 + 7) as u8).collect();
    protocol::Signature::from_slice(&bytes).expect("fits the wire")
}

const SMART: protocol::Address = protocol::Address([0x5a; 20]);

/// **A smart wallet signs in when its chain vouches for it**, on a shard
/// that refuses guests — so the welcome is the proof. And the chain was
/// asked about this wallet, through the validator.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_smart_wallet_is_admitted_when_its_chain_says_yes() {
    let (url, seen) = fake_rpc("0x01");
    let handle = shard_with_chain(Some(url)).await;
    let welcome = join_as(&handle, SMART, |_| Some(smart_signature()))
        .await
        .expect("the chain said yes");
    assert_ne!(welcome.player_id, 0);
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 0);
    let bodies = seen.lock().expect("lock").clone();
    assert_eq!(bodies.len(), 1, "one question, one name: one call");
    assert!(bodies[0].contains("\"eth_call\""), "{}", bodies[0]);
    assert!(
        bodies[0].contains(&"5a".repeat(20)),
        "the chain was asked about this wallet"
    );
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_smart_wallet_is_refused_when_its_chain_says_no() {
    let (url, _) = fake_rpc("0x00");
    let handle = shard_with_chain(Some(url)).await;
    let err = join_as(&handle, SMART, |_| Some(smart_signature()))
        .await
        .expect_err("the chain said no");
    assert!(err.contains("refused"), "{err}");
    assert_eq!(ShardStats::get(&handle.stats.refused_auth), 1);
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A shard that names no chain refuses one, as every shard did before v109.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_chain_a_smart_wallet_is_refused() {
    let handle = shard_with_chain(None).await;
    let err = join_as(&handle, SMART, |_| Some(smart_signature()))
        .await
        .expect_err("nothing can check it");
    assert!(err.contains("refused"), "{err}");
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **A plain key never waits on the network**, even on a shard that names a
/// chain: the arithmetic answers first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_key_never_asks_the_chain() {
    let (url, seen) = fake_rpc("0x00");
    let handle = shard_with_chain(Some(url)).await;
    let sk = key(11);
    join_as(&handle, address_of(&sk), |text| {
        Some(wallet_sign(&sk, std::str::from_utf8(text).expect("utf-8")))
    })
    .await
    .expect("a real key is admitted");
    assert!(
        seen.lock().expect("lock").is_empty(),
        "the chain was asked about a plain key"
    );
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
