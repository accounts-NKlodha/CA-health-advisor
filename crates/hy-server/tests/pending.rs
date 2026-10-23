//! Pending devices and confirmation (V1-5 contract 4.2, 4.5 and 14.2): T-S3 tampered slip
//! with a relay MITM, T-S4 stolen slip redeemed first, T-S18 administrator visibility.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use common::{Device, Harness, error_of, gated_requests, impostor_config, keys_of, serve_tls};
use hy_core::link::LinkClient;
use hy_core::protocol::chain::EntryBody;
use hy_core::protocol::slip::Slip;
use hy_core::protocol::wire::{StateResponse, WireError};
use hy_core::protocol::words::{fingerprint_phrase, fingerprint_words};
use hy_core::protocol::{b64, unb64, unb64_exact};
use hy_core::secret::Secret;
use hy_core::store::StateOutcome;
use hy_server::server::{AdminError, DeviceState, PENDING_EXPIRY_MS};

/// Every gated endpoint answers 403 `device_unconfirmed`, and `GET /v1/state` holds
/// only `{status, alias}`.
async fn assert_pending_sees_nothing(h: &Harness, d: &Device) {
    let c = h.client(1);
    for (method, path, body) in gated_requests() {
        let raw = c.raw(method, path, Some(&d.token), body).await.unwrap();
        assert_eq!(raw.status, 403, "{path}");
        assert_eq!(error_of(&raw), "device_unconfirmed", "{path}");
    }
    let raw = c
        .raw(
            reqwest::Method::GET,
            "/v1/state?after_seq=0",
            Some(&d.token),
            None,
        )
        .await
        .unwrap();
    assert_eq!(raw.status, 200);
    assert_eq!(keys_of(&raw), ["alias", "status"]);
    let body = String::from_utf8(raw.body.clone()).unwrap();
    for word in [
        "entries",
        "head",
        "devices",
        "policy",
        "tombstone",
        "confirmed_on",
    ] {
        assert!(!body.contains(word), "pending state leaks {word}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s3_a_tampered_slip_through_a_relay_leaves_the_device_pending_with_different_words() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let genuine = h.slip(&u);

    // The attacker terminates TLS with its own certificate and relays each request to
    // the real server, rewriting the fingerprint in the pairing answer.
    let (cfg, attacker_fp) = impostor_config();
    let real = LinkClient::new(h.pin(), h.api, common::opts()).unwrap();
    let attacker_b64 = b64(&attacker_fp);
    let relay = axum::Router::new().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
            let real = real.clone();
            let attacker_b64 = attacker_b64.clone();
            async move {
                let token = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .and_then(unb64_exact::<32>)
                    .map(Secret::new);
                let m = reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap();
                let path = uri.path_and_query().map_or("/", |p| p.as_str()).to_owned();
                let body = (!body.is_empty()).then(|| body.to_vec());
                let resp = real.raw(m, &path, token.as_ref(), body).await.unwrap();
                let mut out = resp.body;
                if path == "/v1/pair" && resp.status == 200 {
                    let mut v: serde_json::Value = serde_json::from_slice(&out).unwrap();
                    v["server_fingerprint"] = serde_json::Value::String(attacker_b64);
                    out = serde_json::to_vec(&v).unwrap();
                }
                (StatusCode::from_u16(resp.status).unwrap(), out).into_response()
            }
        },
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let _t = serve_tls(listener, cfg, relay);
    let tampered = Slip::new("127.0.0.1", port, attacker_fp, *genuine.secret.expose()).unwrap();

    // Pairing completes on the attacker's path...
    let (redeemed, d) = h.pair(&tampered, "PC-1").await.unwrap();
    // ...but the companion shows the attacker's words, not the admin page's.
    assert_eq!(redeemed.words, fingerprint_words(&attacker_fp));
    assert_ne!(redeemed.words, fingerprint_words(&h.pin()));
    assert_ne!(redeemed.words.join(" "), fingerprint_phrase(&h.pin()));
    // The device is pending, and pending devices see nothing on the real server.
    let row = h
        .server
        .devices()
        .unwrap()
        .into_iter()
        .find(|r| r.device_ref == d.device_ref)
        .unwrap();
    assert_eq!(row.state, DeviceState::Pending);
    assert_pending_sees_nothing(&h, &d).await;
    // The administrator, seeing the words differ, rejects it: the token is dead.
    h.server.revoke_device(&d.device_ref).unwrap();
    let raw = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&d.token), None)
        .await
        .unwrap();
    assert_eq!(
        (raw.status, error_of(&raw).as_str()),
        (401, "device_revoked")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s4_a_stolen_slip_redeemed_first_stays_pending_until_rejected() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let slip = h.slip(&u);
    // The thief redeems the unaltered slip first: its six words are even correct.
    let (thief_view, thief) = h.pair(&slip, "THIEF").await.unwrap();
    assert_eq!(thief_view.words, fingerprint_words(&h.pin()));
    // The genuine user gets slip_invalid (and reports it).
    let err = h.pair(&slip, "PC-1").await.err().unwrap();
    assert_eq!(
        err,
        hy_core::link::flow::FlowError::Server(WireError::SlipInvalid)
    );
    assert_pending_sees_nothing(&h, &thief).await;
    // The administrator rejects every pending PC of the user.
    assert_eq!(h.server.reject_pending_for_user(&u).unwrap(), 1);
    let raw = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&thief.token), None)
        .await
        .unwrap();
    assert_eq!(error_of(&raw), "device_revoked");
    // A revoked device can never be confirmed.
    let code = hy_core::protocol::words::device_check_code(&thief.device_ref);
    assert_eq!(
        h.server.confirm_device(&thief.device_ref, &code),
        Err(AdminError::NotPending)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s4_an_unconfirmed_device_is_revoked_automatically_at_day_seven() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let (_, thief) = h.pair(&h.slip(&u), "THIEF").await.unwrap();
    h.clock
        .advance(chrono::Duration::milliseconds(PENDING_EXPIRY_MS - 60_000));
    assert_eq!(h.server.maintenance().unwrap(), 0);
    assert_pending_sees_nothing(&h, &thief).await;
    h.clock.advance(chrono::Duration::minutes(1));
    assert_eq!(h.server.maintenance().unwrap(), 1);
    let row = h
        .server
        .devices()
        .unwrap()
        .into_iter()
        .find(|r| r.device_ref == thief.device_ref)
        .unwrap();
    assert_eq!(row.state, DeviceState::Revoked);
    assert_eq!(row.revoke_reason.as_deref(), Some("unconfirmed_expired"));
    let raw = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&thief.token), None)
        .await
        .unwrap();
    assert_eq!(error_of(&raw), "device_revoked");
}

#[test]
fn t_s4_only_the_confirm_action_can_make_a_device_active() {
    // Search the server's sources, settings and admin routes for any other path to
    // 'active': there must be exactly one SQL statement that sets it, inside
    // `Server::confirm_device`, called only by the admin page's Confirm form.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut setters = Vec::new();
    let mut callers = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for (i, _) in flat.match_indices("state = 'active'") {
            setters.push((name.clone(), i, flat.clone()));
        }
        for _ in flat.matches("confirm_device(") {
            callers.push(name.clone());
        }
        // No environment switch, flag or setting can confirm or skip confirmation.
        assert!(
            !text.contains("auto_confirm") && !text.contains("AUTO_CONFIRM"),
            "{name}"
        );
        assert!(!text.contains("skip_confirm"), "{name}");
    }
    assert_eq!(setters.len(), 1, "exactly one statement sets 'active'");
    let (file, at, flat) = &setters[0];
    assert_eq!(file, "server.rs");
    let owner = flat[..*at].rsplit("pub fn ").next().unwrap();
    assert!(
        owner.starts_with("confirm_device("),
        "set inside {}",
        &owner[..40.min(owner.len())]
    );
    assert_eq!(
        callers.iter().filter(|f| *f == "admin.rs").count(),
        1,
        "one admin caller"
    );
    assert!(
        callers.iter().all(|f| f == "admin.rs" || f == "server.rs"),
        "callers: {callers:?}"
    );
    let main = std::fs::read_to_string(src.join("main.rs")).unwrap();
    assert!(
        !main.contains("confirm"),
        "the command line has no confirm path"
    );
    // The database refuses any other transition into 'active'.
    let migration = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/0001_server.sql"),
    )
    .unwrap();
    assert!(migration.contains("devices_revocation_final"));
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s4_the_database_refuses_activating_a_device_outside_confirmation() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let (_, d) = h.pair(&h.slip(&u), "PC-1").await.unwrap();
    // Without confirmed_on (which only confirm_device sets with its chain entry) the
    // schema refuses the change; and a revoked device can never come back.
    let res = h.server.db().execute(
        "UPDATE devices SET state = 'active' WHERE device_ref = ?1",
        [d.device_ref.as_slice()],
    );
    assert!(
        res.is_err(),
        "activation without confirmation data must fail"
    );
    h.server.revoke_device(&d.device_ref).unwrap();
    let res = h.server.db().execute(
        "UPDATE devices SET state = 'active', confirmed_at = 1, confirmed_on = '2026-10-23' WHERE device_ref = ?1",
        [d.device_ref.as_slice()],
    );
    assert!(res.is_err(), "revoked is final");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s4_confirmation_needs_the_check_code_that_was_read_out() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let (r, d) = h.pair(&h.slip(&u), "PC-1").await.unwrap();
    let wrong = if r.check_code.starts_with('0') {
        "1111"
    } else {
        "0000"
    };
    assert_eq!(
        h.server.confirm_device(&d.device_ref, wrong),
        Err(AdminError::CheckCodeMismatch)
    );
    assert_pending_sees_nothing(&h, &d).await;
    // The code the companion shows is the one the admin page expects (typed loosely).
    let typed = r.check_code.to_lowercase();
    h.server.confirm_device(&d.device_ref, &typed).unwrap();
}

fn kinds_after(state: &StateResponse) -> Vec<(u8, EntryBody)> {
    let StateResponse::Confirmed { entries, .. } = state else {
        panic!("pending")
    };
    entries
        .iter()
        .map(|e| {
            (
                e.kind,
                EntryBody::decode(e.kind, &unb64(&e.body, 256).unwrap()).unwrap(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s18_and_t_s4_a_second_confirmed_device_is_visible_to_the_users_other_pc() {
    let h = Harness::start().await;
    let a = h.confirmed_device("u1", "PC-A").await;
    let mut store_a = a.store(h.clock.clone());
    let first = a.sync(&mut store_a).await.unwrap().unwrap();
    let StateOutcome::Applied {
        seq: seq_a,
        newly_confirmed: true,
        new_devices,
    } = first
    else {
        panic!("{first:?}")
    };
    assert!(
        new_devices.is_empty(),
        "devices present at confirmation are not new"
    );

    // The administrator pairs a second device for the same user and confirms it.
    let (_, b) = h.pair(&h.slip(&a.user_id), "PC-B").await.unwrap();
    // While B is pending, A's list shows it as unconfirmed but it is not announced.
    h.confirm(&b);

    // The new entries are in U's chain, under a signed head A verifies.
    let raw = h
        .client(1)
        .raw(
            reqwest::Method::GET,
            &format!("/v1/state?after_seq={seq_a}"),
            Some(&a.token),
            None,
        )
        .await
        .unwrap();
    let kinds = kinds_after(&common::state_of(&raw));
    assert_eq!(
        kinds,
        vec![
            (3, kinds[0].1),
            (
                4,
                EntryBody::DeviceConfirmed {
                    device_ref: b.device_ref
                }
            ),
        ]
    );
    assert!(
        matches!(kinds[0].1, EntryBody::Pairing { device_ref, .. } if device_ref == b.device_ref)
    );

    let out = a.sync(&mut store_a).await.unwrap().unwrap();
    let StateOutcome::Applied {
        newly_confirmed: false,
        new_devices,
        seq,
    } = out
    else {
        panic!("{out:?}")
    };
    assert_eq!(seq, seq_a + 2);
    assert_eq!(new_devices, vec![uuid::Uuid::from_bytes(b.device_ref)]);
    let listed = store_a.paired_devices().unwrap();
    let b_row = listed
        .iter()
        .find(|p| p.device_ref.as_bytes() == &b.device_ref)
        .unwrap();
    assert!(b_row.confirmed && !b_row.notice_shown);
    assert_eq!(b_row.label, "PC-B");
    // Showing the notice is remembered.
    store_a.mark_device_notice_shown(b_row.device_ref).unwrap();
    let again = a.sync(&mut store_a).await.unwrap().unwrap();
    assert!(
        matches!(again, StateOutcome::Applied { ref new_devices, .. } if new_devices.is_empty())
    );
}
