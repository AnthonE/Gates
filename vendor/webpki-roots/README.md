# webpki-roots 1.0.10, without the eMudhra (emSign) roots

Upstream [webpki-roots](https://github.com/rustls/webpki-roots) 1.0.10 with three
entries removed from `src/lib.rs`: **emSign Root CA - C1**, **emSign Root CA - G1**
and **emSign ECC Root CA - C3**. Nothing else is changed (120 → 117 anchors).

Why: in October 2026 the private keys of eight eMudhra intermediates were found to
be brute-forceable. Browsers blocked them; this list did not drop the roots, and
rustls does no revocation checking here, so a forged certificate chained to one of
them would be accepted for any host the client or server talks to.

`crates/server/tests/distrusted_roots.rs` fails if any emSign root is back in
the list the workspace resolves, including if this patch is removed. Delete this
directory and the `[patch.crates-io]` entry once an upstream release drops them.
