//! The pinned TLS 1.3 client for the office server (V1-5 contract 5), shared by the
//! companion and the simulator. Built only with the `link` feature.
//!
//! **Pinning.** [`PinnedVerifier`] accepts a connection only if SHA-256 of the
//! end-entity certificate's DER equals the 32-byte pin, compared in constant time.
//! Intermediates, names, validity dates and OCSP are ignored: the pin is the identity.
//! The TLS 1.3 `CertificateVerify` signature check is **delegated** to the provider, so a
//! server that replays the genuine certificate without its private key fails the
//! handshake (T-S2). TLS 1.2 signatures always fail. Because the HTTP request is written
//! only after the handshake completes, a pin mismatch aborts before any application byte
//! (no token, no secret) is sent (T-P1, T-S1).
//!
//! **Profile.** TLS 1.3 only, no 0-RTT, no session resumption (every connection makes a
//! full handshake and a fresh pin check), ALPN `hy/1`, the constant SNI
//! `hy-server.invalid` (the real address is supplied separately, never looked up by that
//! name), HTTP/1.1, no connection reuse, and no system proxy.

pub mod flow;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rustls::client::Resumption;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::protocol::wire::{
    ControlRequest, ControlResponse, ErrorBody, HistoryResponse, PairRequest, PairResponse,
    RECEIPT_HEADER, ReceiptRequest, ReceiptResponse, StateResponse, UploadRequest, UploadResponse,
    WireError,
};
use crate::protocol::{ALPN, SNI, TOKEN_LEN, b64, ct_eq32, fingerprint, prefix8};
use crate::secret::Secret;

/// Largest response body the client reads.
pub const MAX_RESPONSE: usize = 4 * 1024 * 1024;

/// The crypto provider: aws-lc-rs with its defaults (contract 5 and C4).
#[must_use]
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// The certificate verifier that pins the full SHA-256 of the server's certificate.
#[derive(Debug)]
pub struct PinnedVerifier {
    pin: [u8; 32],
    provider: Arc<CryptoProvider>,
    last_seen: Mutex<Option<[u8; 32]>>,
    checks: AtomicU64,
}

impl PinnedVerifier {
    /// A verifier for `pin`.
    #[must_use]
    pub fn new(pin: [u8; 32]) -> Self {
        Self {
            pin,
            provider: provider(),
            last_seen: Mutex::new(None),
            checks: AtomicU64::new(0),
        }
    }

    /// The fingerprint of the last certificate presented, if any.
    pub fn last_seen(&self) -> Option<[u8; 32]> {
        *self
            .last_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// How many certificates this verifier has checked (one per full handshake).
    pub fn checks(&self) -> u64 {
        self.checks.load(Ordering::SeqCst)
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let seen = fingerprint(end_entity.as_ref());
        *self
            .last_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(seen);
        if ct_eq32(&seen, &self.pin) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 is refused".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Load-bearing: without this check a replayed certificate would be accepted
        // without its private key (T-S2).
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The client TLS configuration for a verifier (contract 5).
///
/// # Errors
/// [`LinkError::Tls`] if the provider cannot do TLS 1.3 (never with aws-lc-rs).
pub fn client_config(verifier: Arc<PinnedVerifier>) -> Result<ClientConfig, LinkError> {
    let mut cfg = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| LinkError::Tls)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![ALPN.to_vec()];
    cfg.resumption = Resumption::disabled();
    cfg.enable_early_data = false;
    Ok(cfg)
}

/// Why a request to the server failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The server's certificate is not the pinned one. Nothing was sent. Only the first 8
    /// hex characters of each fingerprint are kept, for the log.
    PinMismatch {
        /// Prefix of the pin.
        expected: String,
        /// Prefix of what the server presented.
        seen: String,
    },
    /// The TLS handshake failed for another reason (old TLS version, no shared ALPN, a
    /// bad signature).
    Tls,
    /// No connection could be made (no route, refused, timed out).
    Unreachable,
    /// The server answered with an error.
    Server {
        /// HTTP status.
        status: u16,
        /// The error enum.
        error: WireError,
    },
    /// The server's answer was not a valid response.
    Protocol,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PinMismatch { expected, seen } => {
                write!(f, "pin_mismatch expected={expected} seen={seen}")
            }
            Self::Tls => f.write_str("tls_failed"),
            Self::Unreachable => f.write_str("unreachable"),
            Self::Server { status, error } => write!(f, "server status={status} error={error}"),
            Self::Protocol => f.write_str("protocol_error"),
        }
    }
}

impl std::error::Error for LinkError {}

/// Options for [`LinkClient`].
#[derive(Debug, Clone, Copy)]
pub struct LinkOptions {
    /// Bind the client socket to this local address (tests and the simulator use several
    /// loopback addresses).
    pub local_address: Option<IpAddr>,
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            local_address: None,
            connect_timeout: Duration::from_secs(10),
            timeout: Duration::from_secs(30),
        }
    }
}

/// A raw response, for callers (and tests) that handle the status themselves.
#[derive(Debug, Clone)]
pub struct RawResponse {
    /// HTTP status.
    pub status: u16,
    /// The body, at most [`MAX_RESPONSE`] bytes.
    pub body: Vec<u8>,
}

/// A client bound to one server address and one pin.
#[derive(Debug, Clone)]
pub struct LinkClient {
    http: reqwest::Client,
    verifier: Arc<PinnedVerifier>,
    pin: [u8; 32],
    receipt: Option<String>,
}

/// Resolve a slip's host with the OS resolver (DNS, LLMNR or `.local`, as Windows does).
pub async fn resolve(host: &str, port: u16) -> Vec<SocketAddr> {
    tokio::net::lookup_host((host, port))
        .await
        .map(Iterator::collect)
        .unwrap_or_default()
}

impl LinkClient {
    /// A client for the server at `addr` whose certificate must hash to `pin`.
    ///
    /// # Errors
    /// [`LinkError::Tls`] if the HTTP client cannot be built.
    pub fn new(pin: [u8; 32], addr: SocketAddr, opts: LinkOptions) -> Result<Self, LinkError> {
        let verifier = Arc::new(PinnedVerifier::new(pin));
        let tls = client_config(verifier.clone())?;
        let mut builder = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .resolve(SNI, addr)
            .no_proxy()
            .http1_only()
            .pool_max_idle_per_host(0)
            .connect_timeout(opts.connect_timeout)
            .timeout(opts.timeout)
            .redirect(reqwest::redirect::Policy::none());
        if let Some(ip) = opts.local_address {
            builder = builder.local_address(ip);
        }
        let http = builder.build().map_err(|_| LinkError::Tls)?;
        Ok(Self {
            http,
            verifier,
            pin,
            receipt: None,
        })
    }

    /// Echo this stored receipt (wire form) in `HY-Receipt` on every request (contract
    /// 7.4); `None` sends none (a PC that has no receipt yet).
    #[must_use]
    pub fn with_receipt(mut self, receipt: Option<String>) -> Self {
        self.receipt = receipt;
        self
    }

    /// The verifier, for checks that every connection was pinned.
    #[must_use]
    pub fn verifier(&self) -> &PinnedVerifier {
        &self.verifier
    }

    fn url(path: &str) -> String {
        format!("https://{SNI}{path}")
    }

    fn classify(&self, e: &reqwest::Error) -> LinkError {
        if let Some(seen) = self.verifier.last_seen()
            && !ct_eq32(&seen, &self.pin)
        {
            return LinkError::PinMismatch {
                expected: prefix8(&self.pin),
                seen: prefix8(&seen),
            };
        }
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(e);
        while let Some(s) = source {
            if s.downcast_ref::<rustls::Error>().is_some() {
                return LinkError::Tls;
            }
            // `io::Error::source` skips the wrapped error itself, and hyper wraps the
            // TLS error in two layers of `io::Error`, so look inside each layer.
            if let Some(io) = s.downcast_ref::<std::io::Error>()
                && let Some(inner) = io.get_ref()
            {
                source = Some(inner);
                continue;
            }
            source = s.source();
        }
        if e.is_connect() || e.is_timeout() {
            LinkError::Unreachable
        } else {
            LinkError::Protocol
        }
    }

    /// Send a request and return the status and body.
    ///
    /// # Errors
    /// [`LinkError`] for anything but a received response.
    pub async fn raw(
        &self,
        method: reqwest::Method,
        path: &str,
        token: Option<&Secret<[u8; TOKEN_LEN]>>,
        body: Option<Vec<u8>>,
    ) -> Result<RawResponse, LinkError> {
        let mut req = self.http.request(method, Self::url(path));
        if let Some(t) = token {
            req = req.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", b64(t.expose())),
            );
        }
        if let Some(r) = &self.receipt {
            req = req.header(RECEIPT_HEADER, r);
        }
        if let Some(b) = body {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(b);
        }
        let mut resp = req.send().await.map_err(|e| self.classify(&e))?;
        let status = resp.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| self.classify(&e))? {
            if body.len() + chunk.len() > MAX_RESPONSE {
                return Err(LinkError::Protocol);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(RawResponse { status, body })
    }

    async fn call<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        token: Option<&Secret<[u8; TOKEN_LEN]>>,
        body: Option<&Req>,
    ) -> Result<Resp, LinkError> {
        let bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| LinkError::Protocol)?;
        let raw = self.raw(method, path, token, bytes).await?;
        if (200..300).contains(&raw.status) {
            serde_json::from_slice(&raw.body).map_err(|_| LinkError::Protocol)
        } else {
            let err: ErrorBody =
                serde_json::from_slice(&raw.body).map_err(|_| LinkError::Protocol)?;
            Err(LinkError::Server {
                status: raw.status,
                error: err.error,
            })
        }
    }

    /// `POST /v1/pair`.
    ///
    /// # Errors
    /// [`LinkError`]; `Server { error: SlipInvalid, .. }` for any unusable slip.
    pub async fn pair(&self, req: &PairRequest) -> Result<PairResponse, LinkError> {
        self.call(reqwest::Method::POST, "/v1/pair", None, Some(req))
            .await
    }

    /// `GET /v1/state?after_seq=`.
    ///
    /// # Errors
    /// [`LinkError`]; `device_revoked` or `unknown_token` as `Server` errors.
    pub async fn state(
        &self,
        token: &Secret<[u8; TOKEN_LEN]>,
        after_seq: u64,
    ) -> Result<StateResponse, LinkError> {
        self.call::<(), _>(
            reqwest::Method::GET,
            &format!("/v1/state?after_seq={after_seq}"),
            Some(token),
            None,
        )
        .await
    }

    /// `POST /v1/control` (contract 8.3 to 8.5).
    ///
    /// # Errors
    /// [`LinkError`]; `device_regressed`, `cannot_record` and `device_unconfirmed` as
    /// `Server` errors.
    pub async fn control(
        &self,
        token: &Secret<[u8; TOKEN_LEN]>,
        req: &ControlRequest,
    ) -> Result<ControlResponse, LinkError> {
        self.call(reqwest::Method::POST, "/v1/control", Some(token), Some(req))
            .await
    }

    /// `POST /v1/upload` (contract 8.6).
    ///
    /// # Errors
    /// [`LinkError`].
    pub async fn upload(
        &self,
        token: &Secret<[u8; TOKEN_LEN]>,
        req: &UploadRequest,
    ) -> Result<UploadResponse, LinkError> {
        self.call(reqwest::Method::POST, "/v1/upload", Some(token), Some(req))
            .await
    }

    /// `GET /v1/history?from=&to=` ("Restore from server", at most 90 days).
    ///
    /// # Errors
    /// [`LinkError`]; `recovery_hold` while the server holds the user.
    pub async fn history(
        &self,
        token: &Secret<[u8; TOKEN_LEN]>,
        from: &str,
        to: &str,
    ) -> Result<HistoryResponse, LinkError> {
        self.call::<(), _>(
            reqwest::Method::GET,
            &format!("/v1/history?from={from}&to={to}"),
            Some(token),
            None,
        )
        .await
    }

    /// `POST /v1/receipt`: present this PC's stored receipt as recovery evidence
    /// (contract 9).
    ///
    /// # Errors
    /// [`LinkError`].
    pub async fn receipt(
        &self,
        token: &Secret<[u8; TOKEN_LEN]>,
        head: &str,
    ) -> Result<ReceiptResponse, LinkError> {
        let req = ReceiptRequest {
            head: head.to_owned(),
        };
        self.call(
            reqwest::Method::POST,
            "/v1/receipt",
            Some(token),
            Some(&req),
        )
        .await
    }

    /// `POST /v1/unpair`.
    ///
    /// # Errors
    /// [`LinkError`].
    pub async fn unpair(&self, token: &Secret<[u8; TOKEN_LEN]>) -> Result<(), LinkError> {
        let _: serde_json::Value = self
            .call::<(), _>(reqwest::Method::POST, "/v1/unpair", Some(token), None)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_profile_is_tls13_only_without_resumption_or_early_data() {
        let v = Arc::new(PinnedVerifier::new([1; 32]));
        let cfg = client_config(v).unwrap();
        assert_eq!(cfg.alpn_protocols, vec![b"hy/1".to_vec()]);
        assert!(!cfg.enable_early_data);
        assert!(matches!(cfg.resumption, ref r if format!("{r:?}").contains("Disabled")));
    }

    #[test]
    fn tls12_signatures_always_fail() {
        // T-S2 (unit part).
        use rustls::internal::msgs::codec::{Codec, Reader};
        let v = PinnedVerifier::new([1; 32]);
        // scheme 0x0807 (Ed25519), a 64-byte signature.
        let mut wire = vec![0x08, 0x07, 0x00, 0x40];
        wire.extend_from_slice(&[0u8; 64]);
        let dss = DigitallySignedStruct::read(&mut Reader::init(&wire)).unwrap();
        let cert = CertificateDer::from(vec![0u8; 8]);
        assert!(v.verify_tls12_signature(b"m", &cert, &dss).is_err());
        assert!(v.verify_tls13_signature(b"m", &cert, &dss).is_err());
    }

    #[test]
    fn the_verifier_accepts_only_the_exact_pin() {
        let der = b"certificate der bytes".to_vec();
        let pin = fingerprint(&der);
        let v = PinnedVerifier::new(pin);
        let name = ServerName::try_from(SNI).unwrap();
        let ok = v.verify_server_cert(
            &CertificateDer::from(der.clone()),
            &[],
            &name,
            &[],
            UnixTime::now(),
        );
        assert!(ok.is_ok());
        let mut other = der;
        other.push(0);
        let bad = v.verify_server_cert(
            &CertificateDer::from(other),
            &[],
            &name,
            &[],
            UnixTime::now(),
        );
        assert!(bad.is_err());
        assert_eq!(v.checks(), 2);
        assert_ne!(v.last_seen(), Some(pin));
    }
}
