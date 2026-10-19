//! The certificate trust every native HTTPS client of moly uses: the Mozilla
//! root program as `webpki-roots` ships it, verified by rustls' WebPKI
//! verifier on every platform. The operating system's trust store is not
//! read, so every platform trusts the same roots and checks chains the same
//! way; the roots change with the dependency's releases.

use ureq::tls::{RootCerts, TlsConfig};

/// The TLS settings for an HTTPS agent.
pub fn tls_config() -> TlsConfig {
    TlsConfig::builder().root_certs(RootCerts::WebPki).build()
}
