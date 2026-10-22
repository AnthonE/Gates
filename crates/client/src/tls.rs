//! Which certificate authorities a desktop build trusts: the usual list
//! minus [`DISTRUSTED`]. rustls checks no revocation and the browsers'
//! blocklists never reach a native binary, so dropping a bad CA is on us.
//! Every native TLS connection (the shard over WebTransport, every HTTP
//! fetch in the client and the server) builds its roots through here.

/// Organizations whose roots are dropped, matched in the certificate.
/// eMudhra: intermediate CA keys brute-forceable, CVE-2026-111943 (Oct 2026).
pub const DISTRUSTED: &[&[u8]] = &[b"eMudhra"];

pub fn distrusted(der: &[u8]) -> bool {
    DISTRUSTED
        .iter()
        .any(|org| der.windows(org.len()).any(|w| w == *org))
}

/// Mozilla's roots as whole certificates, minus the distrusted. What `ureq`
/// takes as a custom root list.
pub fn root_certs() -> Vec<&'static [u8]> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .map(|c| c.as_ref())
        .filter(|der| !distrusted(der))
        .collect()
}

/// The TLS config every `ureq` agent in the client uses.
#[cfg(feature = "render")]
pub fn ureq() -> ureq::tls::TlsConfig {
    let certs: Vec<_> = root_certs()
        .into_iter()
        .map(ureq::tls::Certificate::from_der)
        .collect();
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::new_with_certs(&certs))
        .build()
}

/// A `ureq` agent with [`ureq`]'s roots, for the plain `ureq::get` calls.
#[cfg(feature = "render")]
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(self::ureq())
        .build()
        .into()
}

/// The OS root store minus the distrusted: what dialling a public shard
/// trusts (`client_endpoint`).
#[cfg(feature = "native")]
pub fn native_roots() -> wtransport::tls::rustls::RootCertStore {
    let mut store = wtransport::tls::build_native_cert_store();
    store.roots.retain(|a| !distrusted(&a.subject));
    store
}

/// WebTransport's default client TLS over [`native_roots`].
#[cfg(feature = "native")]
pub fn webtransport() -> wtransport::config::TlsClientConfig {
    wtransport::tls::client::build_default_tls_config(std::sync::Arc::new(native_roots()), None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn exactly_the_emudhra_roots_are_dropped() {
        let all = webpki_root_certs::TLS_SERVER_ROOT_CERTS.len();
        assert_eq!(
            super::root_certs().len(),
            all - 3,
            "emSign C1, G1 and ECC C3"
        );
        assert!(super::root_certs().iter().all(|d| !super::distrusted(d)));
    }
}
