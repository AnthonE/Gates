//! The roots every server-side HTTP fetch trusts: `client::tls`'s list.

pub fn ureq() -> ureq::tls::TlsConfig {
    let certs: Vec<_> = client::tls::root_certs()
        .into_iter()
        .map(ureq::tls::Certificate::from_der)
        .collect();
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::new_with_certs(&certs))
        .build()
}
