//! The server's TLS profile (V1-5 contract 5): TLS 1.3 only, no 0-RTT, no session
//! resumption (no tickets, no session cache), ALPN `hy/1` only, aws-lc-rs provider.

use std::sync::Arc;

use hy_core::protocol::ALPN;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::keys::ServerKeys;

/// The rustls server configuration for the LAN API.
///
/// # Errors
/// A rustls error if the key does not match the certificate.
pub fn server_config(keys: &ServerKeys) -> Result<Arc<ServerConfig>, rustls::Error> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(keys.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                keys.tls_key_pkcs8().expose().clone(),
            )),
        )?;
    cfg.alpn_protocols = vec![ALPN.to_vec()];
    cfg.max_early_data_size = 0;
    cfg.send_tls13_tickets = 0;
    cfg.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_has_no_early_data_tickets_or_session_cache() {
        let keys = ServerKeys::generate("pc", 2026).unwrap();
        let cfg = server_config(&keys).unwrap();
        assert_eq!(cfg.alpn_protocols, vec![b"hy/1".to_vec()]);
        assert_eq!(cfg.max_early_data_size, 0);
        assert_eq!(cfg.send_tls13_tickets, 0);
        assert!(!cfg.session_storage.can_cache());
    }
}
