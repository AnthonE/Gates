//! **What the client trusts, over a real socket.**
//!
//! The one thing neither half can prove alone. `server::net::spawn_shard`
//! self-signs an identity and prints its SHA-256; `client::client_endpoint`
//! decides which certificates that shard may present. Both compile and both
//! look right in isolation while the client trusts *anything* — which is
//! what it did until 2026-08-10, and the cost is not a stolen key.
//!
//! **SIWE has no channel binding.** The handshake proves the joiner holds
//! the key behind an address (`siwe_wire.rs` proves that end to end), and it
//! proves nothing at all about which TLS connection the proof arrived on. So
//! an on-path relay that terminates the client's QUIC and opens its own to
//! the shard forwards the challenge, forwards the signature, and is admitted
//! as the victim: live session hijack, with the key never leaving the
//! wallet and the nonce perfectly fresh. Certificate validation is the only
//! thing in the stack that refuses that relay, because it is the only thing
//! that asks *who* the far end is.
//!
//! Three claims, and the third is what stops the first two passing for the
//! wrong reason:
//!
//! 1. a non-loopback shard with a self-signed certificate is REFUSED, at the
//!    transport, before a byte of handshake;
//! 2. the same shard is admitted when the operator pins its hash
//!    (`--cert-hash`) — and a *wrong* pin is still refused, so the pin is a
//!    check and not a second permissive path;
//! 3. loopback stays permissive, which is the carve-out `DECISIONS.md`
//!    2026-08-10 spells out and the reason claim 1's shard is not simply
//!    broken.

use server::config::ShardConfig;
use server::net::{spawn_shard, ShardHandle};
use server::store::Saves;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// The shipped content, baked. A copy of `siwe_wire`'s helper rather than a
/// shared module, for the same reason it gives: two integration test binaries
/// cannot share a `mod` without a file that belongs to neither.
fn baked_content() -> server::net::SimTables {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let c = content::Content::load_dir(&dir).expect("shipped content loads");
    let mut tables = server::net::bake_all(&c).expect("shipped content bakes");
    // Naked, like every other wire suite: a fresh inventory is content, and
    // this file is asserting on transport.
    tables.spawn_kit = sim_core::inventory::SpawnKit::EMPTY;
    tables
}

/// **This box's own non-loopback IPv4 address, measured rather than
/// guessed.** A connected UDP socket sends nothing — `connect` on a datagram
/// socket only fixes the peer — but the kernel picks the source address for
/// that route immediately, and `local_addr` reports it. So this asks the
/// routing table which address the machine would speak from, without a
/// packet, a name lookup, or a crate.
///
/// **A panic, never a skip.** `CLAUDE.md`: a suite that cannot run its claim
/// must say so loudly and exit nonzero, because a pass it did not earn is
/// the worst bug class. A box with no route off loopback cannot demonstrate
/// the refusal this file exists for, and quietly asserting nothing there
/// would leave the client's whole trust posture ungated on exactly the CI
/// machine where nobody is watching.
fn non_loopback_v4() -> Ipv4Addr {
    // Two probes, both to addresses reserved for documentation (RFC 5737) so
    // that a misread of this function can never generate traffic to a real
    // host: the first exercises the default route, the second any route at
    // all. Neither is contacted.
    for probe in ["203.0.113.1:9", "198.51.100.1:9"] {
        let sock = match UdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(_) => continue,
        };
        if sock.connect(probe).is_err() {
            continue;
        }
        if let Ok(SocketAddr::V4(local)) = sock.local_addr() {
            if !local.ip().is_loopback() && !local.ip().is_unspecified() {
                return *local.ip();
            }
        }
    }
    panic!(
        "tls_posture: this box has no non-loopback IPv4 address, so the one claim \
         this suite exists to prove — that a NON-loopback shard is refused — cannot \
         be demonstrated here. Not a defect in the tree and not something to skip \
         quietly: run it on a box with a route."
    );
}

/// An ephemeral shard bound to `ip`, self-signing the usual dev identity.
///
/// Note what it does NOT do: the certificate's SANs are `localhost`,
/// `127.0.0.1` and `::1` whatever this binds to (`net::spawn_shard`), which
/// is correct — a dev certificate has no business claiming an address the
/// operator happened to run it on, and the pin below does not consult a name.
async fn shard(ip: IpAddr) -> ShardHandle {
    let tables = baked_content();
    let mut cfg = ShardConfig::ephemeral(20_260_810);
    cfg.bind = SocketAddr::new(ip, 0);
    spawn_shard(
        cfg,
        tables,
        Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("boots")
}

/// Join as a guest — the identity half is `siwe_wire`'s, and pulling a
/// signature in here would let a TLS failure hide behind an auth one.
async fn join(
    endpoint_addr: &str,
    endpoint: &wtransport::Endpoint<wtransport::endpoint::endpoint_side::Client>,
) -> Result<client::Session, String> {
    tokio::time::timeout(
        Duration::from_secs(10),
        client::Session::connect(
            endpoint,
            endpoint_addr,
            protocol::Address::GUEST,
            |_, _, _| None,
        ),
    )
    .await
    .map_err(|_| "the connect never resolved".to_string())?
}

/// **Claim 1.** A shard the client did not self-sign for, at an address that
/// is not this machine's own loopback, must not be admitted on the strength
/// of presenting *a* certificate. This is the relay, modelled: an attacker's
/// endpoint can always produce a valid self-signed chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_loopback_shard_with_an_unpinned_certificate_is_refused() {
    let ip = non_loopback_v4();
    let handle = shard(IpAddr::V4(ip)).await;
    let addr = format!("{}:{}", ip, handle.local_addr.port());

    let endpoint = client::client_endpoint(&addr, None).expect("endpoint");
    let err = join(&addr, &endpoint)
        .await
        .err()
        .expect("an unpinned self-signed certificate must not be trusted off loopback");

    // **The refusal has to come from the transport, not the handshake.** A
    // shard that refused this join for its own reasons would satisfy an
    // `is_err` and prove nothing about what the client trusts, so the prefix
    // is load-bearing: `Session::connect` stamps `connect:` only on the
    // `endpoint.connect` failure, before a Hello is written.
    assert!(
        err.starts_with("connect:"),
        "refused, but not at the certificate: {err}"
    );

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Claim 2.** The dev path the refusal above is not allowed to cost us:
/// an operator who has the shard's printed hash may still reach it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pinned_certificate_hash_admits_the_same_shard() {
    let ip = non_loopback_v4();
    let handle = shard(IpAddr::V4(ip)).await;
    let addr = format!("{}:{}", ip, handle.local_addr.port());

    let endpoint = client::client_endpoint(&addr, Some(&handle.cert_hash)).expect("endpoint");
    let session = join(&addr, &endpoint)
        .await
        .expect("the shard's own hash must admit the shard");
    assert_ne!(session.welcome.player_id, 0, "welcomed with no player id");

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Claim 2, the other half.** A pin that does not match is refused — which
/// is what makes the test above evidence of a check rather than of a second
/// way to trust anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pin_that_does_not_match_is_refused() {
    let ip = non_loopback_v4();
    let handle = shard(IpAddr::V4(ip)).await;
    let addr = format!("{}:{}", ip, handle.local_addr.port());

    // A well-formed digest of the right shape and the wrong value.
    let wrong = (0..32)
        .map(|_| "ab".to_string())
        .collect::<Vec<_>>()
        .join(":");
    assert_ne!(wrong, handle.cert_hash);

    let endpoint = client::client_endpoint(&addr, Some(&wrong)).expect("endpoint");
    let err = join(&addr, &endpoint)
        .await
        .err()
        .expect("a pin for another certificate must not admit this one");
    assert!(
        err.starts_with("connect:"),
        "refused, but not at the certificate: {err}"
    );

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// **Claim 3, the carve-out.** Loopback stays permissive, and this is the
/// test that says the two refusals above are about trust rather than about a
/// shard that never worked: the identical dev shard, on `127.0.0.1`, with no
/// pin, is joined.
///
/// Why the carve-out is not a hole: an attacker able to sit on the path
/// between two sockets on this machine is already executing code on this
/// machine, so it costs nothing against the threat that motivates the file
/// (an on-path relay on a public shard) — and it is what keeps every local
/// dev flow and the capture harness working without a flag.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_stays_permissive() {
    let handle = shard(IpAddr::V4(Ipv4Addr::LOCALHOST)).await;
    let addr = format!("127.0.0.1:{}", handle.local_addr.port());

    let endpoint = client::client_endpoint(&addr, None).expect("endpoint");
    let session = join(&addr, &endpoint)
        .await
        .expect("a loopback dev shard must still be joinable with no pin");
    assert_ne!(session.welcome.player_id, 0, "welcomed with no player id");

    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
}
