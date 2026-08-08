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
fn baked_content() -> (
    sim_core::gather::GatherContent,
    sim_core::craft::CraftContent,
    sim_core::build::BuildContent,
    sim_core::deploy::DeployContent,
    sim_core::combat::CombatContent,
    sim_core::backpack::BackpackContent,
    sim_core::survival::SurvivalContent,
    sim_core::loot::LootContent,
    protocol::ItemCatalog,
) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    (
        c.bake_gather().expect("gather"),
        c.bake_craft().expect("craft"),
        c.bake_building().expect("building"),
        c.bake_deployables().expect("deployables"),
        c.bake_combat().expect("combat"),
        c.bake_backpack().expect("backpack"),
        c.bake_survival().expect("survival"),
        c.bake_loot().expect("loot"),
        server::net::bake_catalog(&c).expect("catalog"),
    )
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
    let (gather, craft, build, deploy, combat, backpack, survival, loot, catalog) = baked_content();
    let mut cfg = ShardConfig::ephemeral(20_260_807);
    cfg.require_auth = require_auth;
    cfg.domain = DOMAIN.into();
    spawn_shard(
        cfg,
        gather,
        craft,
        build,
        deploy,
        combat,
        backpack,
        survival,
        // A test shard spawns naked: the alpha `[[spawn_kit]]` is scaffolding
        // for a human looking at the game, and a suite that asserted on a
        // fresh inventory would be asserting on content instead of on code.
        sim_core::inventory::SpawnKit::EMPTY,
        loot,
        catalog,
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
