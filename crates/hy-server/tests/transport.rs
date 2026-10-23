//! Transport tests (V1-5 contract 5 and 14.2): T-S1 pin enforcement on every
//! connection with a raw capture, T-S2 certificate replay, T-S24 rogue server by name.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, Mutex};

use common::{Harness, gated_requests, impostor_config, opts, serve_tls};
use hy_core::link::LinkClient;
use hy_core::link::flow::FlowError;
use hy_core::protocol::slip::Slip;
use hy_core::protocol::{ALPN, b64};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type Captures = Arc<Mutex<Vec<Vec<u8>>>>;

/// A recording relay in front of `target`: every connection's client-to-server bytes
/// are kept, one buffer per connection. Returns the relay's port.
async fn capture_relay(target: std::net::SocketAddr) -> (u16, Captures) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let caps: Captures = Arc::new(Mutex::new(Vec::new()));
    let caps2 = caps.clone();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let idx = {
                let mut c = caps2.lock().unwrap();
                c.push(Vec::new());
                c.len() - 1
            };
            let caps = caps2.clone();
            tokio::spawn(async move {
                let Ok(server) = TcpStream::connect(target).await else {
                    return;
                };
                let (mut cr, mut cw) = client.into_split();
                let (mut sr, mut sw) = server.into_split();
                let up = async move {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = cr.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        caps.lock().unwrap()[idx].extend_from_slice(&buf[..n]);
                        if sw.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    let _ = sw.shutdown().await;
                };
                let down = async move {
                    let _ = tokio::io::copy(&mut sr, &mut cw).await;
                    let _ = cw.shutdown().await;
                };
                tokio::join!(up, down);
            });
        }
    });
    (port, caps)
}

/// The TLS records a client sent: (content type, payload length).
fn records(bytes: &[u8]) -> Vec<(u8, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 5 <= bytes.len() {
        let len = usize::from(u16::from_be_bytes([bytes[at + 3], bytes[at + 4]]));
        out.push((bytes[at], len));
        at += 5 + len;
    }
    out
}

/// True if a connection carried nothing but a ClientHello, an optional compatibility
/// ChangeCipherSpec, and at most an alert (an encrypted alert is 2 + 1 + 16 bytes).
/// Any HTTP request (and so any `Authorization` header or secret) would need an
/// application-data record far larger than that, and only after the handshake.
fn no_application_data(bytes: &[u8]) -> bool {
    let recs = records(bytes);
    !recs.is_empty()
        && recs[0].0 == 0x16
        && recs[1..].iter().all(|&(t, len)| match t {
            0x14 => len == 1,
            0x15 => len <= 2,
            0x17 => len <= 19,
            _ => false,
        })
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s1_every_connection_to_a_different_certificate_fails_before_any_application_data() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let u = h.user("u2");
    let slip = h.slip(&u);

    // The impostor has its own self-signed certificate; the relay records the bytes.
    let (cfg, _) = impostor_config();
    let imp = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let imp_addr = imp.local_addr().unwrap();
    let _t = serve_tls(imp, cfg, axum::Router::new());
    let (port, caps) = capture_relay(imp_addr).await;
    let relay = std::net::SocketAddr::from(([127, 0, 0, 1], port));

    // Pairing.
    let moved = Slip::new("127.0.0.1", port, h.pin(), *slip.secret.expose()).unwrap();
    let err = h.pair(&moved, "PC-2").await.err().unwrap();
    assert!(matches!(err, FlowError::PinMismatch { .. }), "{err:?}");
    // State, unpair, history, control, upload, receipt and the manifest.
    let client = LinkClient::new(h.pin(), relay, opts()).unwrap();
    let mut calls = vec![
        (reqwest::Method::GET, "/v1/state", None),
        (reqwest::Method::POST, "/v1/unpair", None),
    ];
    calls.extend(gated_requests());
    for (method, path, body) in calls.clone() {
        let err = client.raw(method, path, Some(&d.token), body).await.err();
        assert!(
            matches!(err, Some(hy_core::link::LinkError::PinMismatch { .. })),
            "{path}: {err:?}"
        );
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let caps = caps.lock().unwrap().clone();
    assert_eq!(caps.len(), 1 + calls.len(), "one connection per request");
    let secret_text = hy_core::protocol::slip::encode_secret(slip.secret.expose());
    let token_text = b64(d.token.expose());
    for (i, c) in caps.iter().enumerate() {
        assert!(
            no_application_data(c),
            "connection {i}: records {:?}",
            records(c)
        );
        let text = String::from_utf8_lossy(c);
        assert!(!text.contains(secret_text.expose().as_str()));
        assert!(!text.contains(&token_text));
        assert!(!text.contains("Authorization"));
    }
    // The genuine slip is untouched.
    h.pair(&slip, "PC-2").await.unwrap();
}

/// Presents the genuine certificate with a different private key.
#[derive(Debug)]
struct ReplayedCert(Arc<CertifiedKey>);

impl ResolvesServerCert for ReplayedCert {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s2_a_replayed_certificate_without_its_key_fails_the_handshake() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let slip = h.slip(&u);
    let provider = hy_core::link::provider();
    let other = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            other.serialize_der(),
        )))
        .unwrap();
    let ck = CertifiedKey::new(
        vec![CertificateDer::from(h.server.keys.cert_der.clone())],
        key,
    );
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(ReplayedCert(Arc::new(ck))));
    cfg.alpn_protocols = vec![ALPN.to_vec()];
    let imp = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let imp_addr = imp.local_addr().unwrap();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits2 = hits.clone();
    let router = axum::Router::new().fallback(move || {
        let hits = hits2.clone();
        async move {
            hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            "{}"
        }
    });
    let _t = serve_tls(imp, Arc::new(cfg), router);
    let (port, caps) = capture_relay(imp_addr).await;

    let moved = Slip::new("127.0.0.1", port, h.pin(), *slip.secret.expose()).unwrap();
    let err = h.pair(&moved, "PC-1").await.err().unwrap();
    // The certificate matches the pin, so this is the signature check failing.
    assert_eq!(err, FlowError::Tls);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    for c in caps.lock().unwrap().iter() {
        assert!(no_application_data(c), "records {:?}", records(c));
    }
    assert_eq!(h.count("devices"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s24_a_rogue_server_answering_to_the_name_gets_no_token() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());
    d.sync(&mut store).await.unwrap();
    // The rogue host answers at the address the name now resolves to (here: the last
    // address the PC remembers, which it tries first).
    let (cfg, _) = impostor_config();
    let imp = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let imp_addr = imp.local_addr().unwrap();
    let _t = serve_tls(imp, cfg, axum::Router::new());
    let (port, caps) = capture_relay(imp_addr).await;
    store
        .set_link_last_ip(&format!("127.0.0.1:{port}"))
        .unwrap();
    let before = store.head_receipt().unwrap().unwrap();
    let err = d.sync(&mut store).await.err().unwrap();
    assert!(matches!(err, FlowError::PinMismatch { .. }));
    assert!(
        err.to_string().contains("not the office server"),
        "the user is told: {err}"
    );
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let caps = caps.lock().unwrap().clone();
    assert_eq!(caps.len(), 1, "it aborts; no other address is tried");
    assert!(no_application_data(&caps[0]));
    assert!(!String::from_utf8_lossy(&caps[0]).contains(&b64(d.token.expose())));
    assert_eq!(
        store.head_receipt().unwrap().unwrap(),
        before,
        "nothing applied"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_closes_connections_without_alpn_hy1() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut cfg = rustls::ClientConfig::builder_with_provider(hy_core::link::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(hy_core::link::PinnedVerifier::new(h.pin())))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let tcp = TcpStream::connect(h.api).await.unwrap();
    let res = connector
        .connect(
            rustls::pki_types::ServerName::try_from(hy_core::protocol::SNI).unwrap(),
            tcp,
        )
        .await;
    // rustls refuses a mismatched ALPN offer outright; if a handshake ever completed,
    // the server must still close without answering.
    if let Ok(mut tls) = res {
        let req = format!(
            "GET /v1/state HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\n\r\n",
            b64(d.token.expose())
        );
        let _ = tls.write_all(req.as_bytes()).await;
        let mut out = Vec::new();
        let _ = tls.read_to_end(&mut out).await;
        assert!(out.is_empty());
    }
}
