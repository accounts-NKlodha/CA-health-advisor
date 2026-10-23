//! T-S10 log canaries (V1-5 contract 11 and 14.2). One test, alone in its binary,
//! because it installs the global log subscriber and the panic hook: a full V1-5a flow
//! runs with planted values, then every log line, the audit log, the admin pages, the
//! panic output and the companion-side debug output are searched for them.
//!
//! By design the admin page shows the user's alias and the PC label (the administrator
//! needs them to confirm), and the one-time slip page shows the slip; those pages are
//! checked for the other canaries only.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Harness, admin_login, admin_request, csrf_in, http};
use hy_core::protocol::slip::encode_secret;
use hy_core::protocol::{b64, hex};
use hy_core::secret::Secret;
use hy_server::logging::{self, MemorySink};

const ALIAS: &str = "CANARY_ALIAS_7f3a";
const LABEL: &str = "CANARY_LABEL_5b1e";
const WATER: &str = "987654";
const PASS: &str = "CANARY_PASS_9c2d_long";
const WRONG_PASS: &str = "CANARY_WRONG_4e8a_long";

#[tokio::test(flavor = "multi_thread")]
async fn t_s10_no_planted_value_reaches_any_log_audit_entry_page_or_panic_output() {
    let sink = MemorySink::default();
    let writer = sink.clone();
    assert!(logging::install(move || writer.clone()));
    logging::install_panic_hook();

    let h = Harness::start().await;
    h.server
        .set_admin_passphrase(&Secret::new(PASS.to_owned()))
        .unwrap();
    let u = h.user(ALIAS);
    let host = format!("127.0.0.1:{}", h.admin.port());
    let origin = format!("http://{host}");

    // Sign-in, wrong then right.
    let (status, _, _) = http(
        h.admin,
        &admin_request(
            "POST",
            "/login",
            &host,
            Some(&origin),
            None,
            Some(&format!("passphrase={WRONG_PASS}")),
        ),
    )
    .await;
    assert_eq!(status, 401);
    h.clock.advance(chrono::Duration::seconds(2));
    let cookie = admin_login(&h, PASS).await;
    let (_, _, overview) = http(
        h.admin,
        &admin_request("GET", "/", &host, None, Some(&cookie), None),
    )
    .await;
    let csrf = csrf_in(&overview);
    let (_, _, slip_page) = http(
        h.admin,
        &admin_request(
            "POST",
            "/slips",
            &host,
            Some(&origin),
            Some(&cookie),
            Some(&format!("csrf={csrf}&user={}", hex(&u))),
        ),
    )
    .await;
    let at = slip_page.find("hy1-").unwrap();
    let slip_text: String = slip_page[at..].chars().take_while(|c| *c != '<').collect();
    let slip = hy_core::protocol::slip::Slip::parse(&slip_text).unwrap();
    let secret = encode_secret(slip.secret.expose()).expose().clone();

    // Pairing, a pending device poking at everything with a planted amount, a refused
    // replay, confirmation, state, a confirmed upload attempt, revocation.
    let (redeemed, d) = h.pair(&slip, LABEL).await.unwrap();
    let token_b64 = b64(d.token.expose());
    let token_hex = hex(d.token.expose());
    let c = h.client(1);
    let upload = format!(
        r#"{{"category":"hydration","local_date":"2026-10-22","revision":1,"payload":{{"ml":{WATER}}}}}"#
    );
    for (m, path, body) in common::gated_requests() {
        let _ = c.raw(m, path, Some(&d.token), body).await.unwrap();
    }
    let _ = c
        .raw(
            reqwest::Method::POST,
            "/v1/upload",
            Some(&d.token),
            Some(upload.clone().into_bytes()),
        )
        .await;
    let _ = h.pair(&slip, LABEL).await;
    let _ = c
        .raw(
            reqwest::Method::GET,
            &format!("/v1/state?token={token_b64}"),
            None,
            None,
        )
        .await;
    let (_, _, overview2) = http(
        h.admin,
        &admin_request("GET", "/", &host, None, Some(&cookie), None),
    )
    .await;
    h.confirm(&d);
    let mut store = d.store(h.clock.clone());
    d.sync(&mut store).await.unwrap();
    let _ = c
        .raw(
            reqwest::Method::POST,
            "/v1/upload",
            Some(&d.token),
            Some(upload.into_bytes()),
        )
        .await;
    h.server.revoke_device(&d.device_ref).unwrap();
    let _ = c
        .raw(reqwest::Method::GET, "/v1/state", Some(&d.token), None)
        .await;
    h.clock.advance(chrono::Duration::days(8));
    h.server.maintenance().unwrap();

    // A panic whose message carries every canary.
    let message = format!("{ALIAS} {LABEL} {WATER} {PASS} {secret} {token_b64}");
    let joined = std::thread::spawn(move || panic!("{message}")).join();
    assert!(joined.is_err());
    // Back to the default hook so this test's own failures are readable.
    drop(std::panic::take_hook());

    // Companion-side debug output of the pairing results.
    let debug = format!("{redeemed:?} {:?} {:?}", d.link, d.token);

    let logs = sink.contents();
    assert!(logs.contains("slip_redeemed"), "the log is being captured");
    assert!(
        logs.contains("\"event\":\"panic\""),
        "the panic was logged (location only)"
    );
    let audit: String = {
        let conn = h.server.db();
        let mut stmt = conn
            .prepare("SELECT at, event, hex(user_id), hex(device_ref), net24 FROM audit")
            .unwrap();
        stmt.query_map([], |r| {
            Ok(format!(
                "{:?} {:?} {:?} {:?} {:?}",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n")
    };
    assert!(audit.contains("slip_redeemed"));

    let all = [
        ALIAS, LABEL, WATER, PASS, WRONG_PASS, &secret, &token_b64, &token_hex,
    ];
    for (name, text) in [
        ("server log", &logs),
        ("audit log", &audit),
        ("debug output", &debug),
    ] {
        for canary in all {
            assert!(!text.contains(canary), "{name} contains a canary");
        }
    }
    // The admin pages may show the alias and label, never the rest.
    let not_on_pages = [WATER, PASS, WRONG_PASS, &secret, &token_b64, &token_hex];
    for (name, page) in [
        ("overview", &overview),
        ("overview after pairing", &overview2),
    ] {
        for canary in not_on_pages {
            assert!(!page.contains(canary), "{name} contains a canary");
        }
    }
    // The slip page shows the slip once (by design) but nothing else secret.
    for canary in [WATER, PASS, WRONG_PASS, &token_b64, &token_hex] {
        assert!(!slip_page.contains(canary));
    }
}
