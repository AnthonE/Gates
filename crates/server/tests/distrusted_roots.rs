//! The eMudhra (emSign) roots stay out of the trust list.
//!
//! In October 2026 the keys of eight eMudhra intermediates turned out to be
//! brute-forceable. ureq's rustls does no revocation checking, so a root in
//! `TLS_SERVER_ROOTS` is the whole decision: a forged certificate chained to
//! one would pass for any host the client or server fetches from.
//!
//! This reads the `webpki-roots` the workspace actually resolves — the same
//! one every ureq agent in `client` and `server` links — so it fails if the
//! vendored list regains an emSign root *or* if the `[patch.crates-io]` entry
//! goes away.

fn mentions(der: &[u8], needle: &[u8]) -> bool {
    der.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn no_emsign_root_is_trusted() {
    let emsign: Vec<String> = webpki_roots::TLS_SERVER_ROOTS
        .iter()
        .filter(|a| mentions(&a.subject, b"emSign") || mentions(&a.subject, b"eMudhra"))
        .map(|a| String::from_utf8_lossy(&a.subject).into_owned())
        .collect();
    assert!(
        emsign.is_empty(),
        "distrusted eMudhra roots are trusted: {emsign:?}"
    );
}
