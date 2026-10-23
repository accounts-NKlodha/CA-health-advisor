//! Pairing failure tests T-P1 to T-P6 (architecture section 11; V1-5 contract 14.1),
//! against the real server over loopback TLS with the companion's real client.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Harness, impostor_config, opts, opts_from, serve_tls};
use hy_core::link::PinnedVerifier;
use hy_core::link::flow::{FlowError, fetch_state};
use hy_core::protocol::slip::{Slip, encode_secret};
use hy_core::protocol::wire::{PairRequest, WireError};
use hy_core::protocol::{ALPN, SNI};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The same slip text pointed at another port (a MITM or a moved server).
fn slip_at(slip: &Slip, port: u16, pin: [u8; 32]) -> Slip {
    Slip::new(&slip.host, port, pin, *slip.secret.expose()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p1_a_mitm_certificate_aborts_before_the_secret_is_sent() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let slip = h.slip(&u);
    // The attacker's server answers on the slip's address with its own certificate and
    // counts every HTTP request that reaches it.
    let hits = Arc::new(AtomicUsize::new(0));
    let (cfg, _) = impostor_config();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits2 = hits.clone();
    let router = axum::Router::new().fallback(move || {
        let hits = hits2.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            "{}"
        }
    });
    let _t = serve_tls(listener, cfg, router);

    let moved = slip_at(&slip, port, h.pin());
    let err = h.pair(&moved, "PC-1").await.err().expect("must fail");
    assert!(matches!(err, FlowError::PinMismatch { .. }), "{err:?}");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "no request reached the impostor"
    );
    // The secret was never used: the genuine server still accepts the slip.
    assert_eq!(h.count("devices"), 0);
    h.pair(&slip, "PC-1").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p2_a_reused_slip_is_rejected() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let slip = h.slip(&u);
    h.pair(&slip, "PC-1").await.unwrap();
    let err = h.pair(&slip, "PC-2").await.err().unwrap();
    assert_eq!(err, FlowError::Server(WireError::SlipInvalid));
    assert_eq!(h.count("devices"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p3_simultaneous_redemptions_create_exactly_one_device() {
    let h = Arc::new(Harness::start().await);
    let u = h.user("u1");
    let slip = Arc::new(h.slip(&u));
    let mut tasks = Vec::new();
    // Ten PCs on ten addresses (so the per-IP limit is not what stops them).
    for n in 2..12u8 {
        let h = h.clone();
        let slip = slip.clone();
        tasks.push(tokio::spawn(async move {
            h.pair_as(&slip, &format!("PC-{n}"), common::new_id(), opts_from(n))
                .await
                .map(|_| ())
        }));
    }
    let mut ok = 0;
    for t in tasks {
        match t.await.unwrap() {
            Ok(()) => ok += 1,
            Err(e) => assert_eq!(e, FlowError::Server(WireError::SlipInvalid)),
        }
    }
    assert_eq!(ok, 1, "exactly one redemption wins");
    assert_eq!(h.count("devices"), 1);
    let used: i64 = h
        .server
        .db()
        .query_row(
            "SELECT COUNT(*) FROM slips WHERE used_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(used, 1);
    // One pairing entry in the user's chain.
    let entries: i64 = h
        .server
        .db()
        .query_row("SELECT COUNT(*) FROM control_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(entries, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p4_an_expired_slip_is_rejected() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let fresh = h.slip(&u);
    let old = h.slip(&u);
    h.clock
        .advance(chrono::Duration::hours(23) + chrono::Duration::minutes(59));
    h.pair(&fresh, "PC-1").await.unwrap();
    h.clock.advance(chrono::Duration::minutes(1));
    let err = h.pair(&old, "PC-2").await.err().unwrap();
    assert_eq!(err, FlowError::Server(WireError::SlipInvalid));
    assert_eq!(h.count("devices"), 1);
}

#[derive(Debug)]
struct AcceptAnything;

impl ServerCertVerifier for AcceptAnything {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        hy_core::link::provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p5_tls_below_1_3_is_refused_by_the_server() {
    let h = Harness::start().await;
    // A client that would accept any certificate but speaks only TLS 1.2: the server
    // itself must refuse.
    let mut cfg = rustls::ClientConfig::builder_with_provider(hy_core::link::provider())
        .with_protocol_versions(&[&rustls::version::TLS12])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnything))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![ALPN.to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let tcp = tokio::net::TcpStream::connect(h.api).await.unwrap();
    let res = connector
        .connect(ServerName::try_from(SNI).unwrap(), tcp)
        .await;
    assert!(res.is_err(), "a TLS 1.2 handshake must fail");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p5_no_resumption_and_no_early_data_even_for_a_client_that_asks() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    // A client with resumption and 0-RTT switched on, pinned to the real server.
    let mut cfg = rustls::ClientConfig::builder_with_provider(hy_core::link::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedVerifier::new(h.pin())))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![ALPN.to_vec()];
    cfg.enable_early_data = true;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    for round in 0..2 {
        let tcp = tokio::net::TcpStream::connect(h.api).await.unwrap();
        let mut tls = connector
            .connect(ServerName::try_from(SNI).unwrap(), tcp)
            .await
            .unwrap();
        let (_, conn) = tls.get_ref();
        assert_eq!(
            conn.handshake_kind(),
            Some(rustls::HandshakeKind::Full),
            "round {round} must be a full handshake"
        );
        assert!(!conn.is_early_data_accepted());
        let req = format!(
            "GET /v1/state HTTP/1.1\r\nHost: {SNI}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            hy_core::protocol::b64(d.token.expose())
        );
        tls.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = tls.read_to_end(&mut out).await;
        assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 200"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p5_a_captured_pairing_secret_cannot_be_used_again() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let slip = h.slip(&u);
    let body = serde_json::to_vec(&PairRequest {
        protocol: hy_core::protocol::PROTOCOL,
        secret: encode_secret(slip.secret.expose()).expose().clone(),
        origin_device: common::new_id(),
        device_label: "PC-1".into(),
        app_version: "0.5.0".into(),
        windows_build: "26100".into(),
    })
    .unwrap();
    let c = h.client(1);
    let first = c
        .raw(reqwest::Method::POST, "/v1/pair", None, Some(body.clone()))
        .await
        .unwrap();
    assert_eq!(first.status, 200);
    // The exact same bytes again (a captured request), from another address.
    let again = h
        .client(2)
        .raw(reqwest::Method::POST, "/v1/pair", None, Some(body))
        .await
        .unwrap();
    assert_eq!(again.status, 403);
    assert_eq!(common::error_of(&again), "slip_invalid");
    assert_eq!(h.count("devices"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p6_after_an_ip_change_the_pc_reconnects_by_name_with_the_pin() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());
    // The last address is now dead (the server moved); the slip's host still resolves.
    let dead = {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap()
    };
    store.set_link_last_ip(&dead.to_string()).unwrap();
    let out = d.sync(&mut store).await.unwrap().unwrap();
    assert!(matches!(
        out,
        hy_core::store::StateOutcome::Applied {
            newly_confirmed: true,
            ..
        }
    ));
    assert_eq!(
        store.server_link().unwrap().unwrap().last_ip.as_deref(),
        Some(h.api.to_string().as_str()),
        "the address that passed the pin is remembered"
    );
    // A manual address works the same way, still pinned.
    store
        .set_link_manual_address(Some(&h.api.to_string()))
        .unwrap();
    store.set_link_last_ip(&dead.to_string()).unwrap();
    assert!(d.sync(&mut store).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p6_an_impostor_at_the_old_address_is_never_trusted() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());
    let (cfg, _) = impostor_config();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let old = listener.local_addr().unwrap();
    let _t = serve_tls(listener, cfg, axum::Router::new());
    store.set_link_last_ip(&old.to_string()).unwrap();
    let err = d.sync(&mut store).await.err().unwrap();
    assert!(matches!(err, FlowError::PinMismatch { .. }), "{err:?}");
    assert!(store.head_receipt().unwrap().is_none(), "nothing applied");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_p6_a_revoked_token_gets_401_and_the_pc_offers_to_re_pair() {
    let h = Harness::start().await;
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());
    d.sync(&mut store).await.unwrap();
    h.server.revoke_device(&d.device_ref).unwrap();
    let raw = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&d.token), None)
        .await
        .unwrap();
    assert_eq!(raw.status, 401);
    assert_eq!(common::error_of(&raw), "device_revoked");
    let link = store.server_link().unwrap().unwrap();
    let err = fetch_state(&link, 0, &d.vault, opts()).await.err().unwrap();
    assert_eq!(err, FlowError::Revoked);
    assert!(store.unpair_locally().unwrap());
    assert!(store.server_link().unwrap().is_none());
    // A new slip pairs the same PC again.
    let slip = h.slip(&d.user_id);
    let (_, again) = h
        .pair_as(&slip, "PC-1", common::new_id(), opts())
        .await
        .unwrap();
    assert_ne!(again.device_ref, d.device_ref);
}
