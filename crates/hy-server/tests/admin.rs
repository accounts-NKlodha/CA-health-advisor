//! Admin page hardening, T-S12 (V1-5 contract TH15 and 14.2), and the confirmation flow
//! through the page.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    Harness, admin_login, admin_request, csrf_in, http, test_config, test_limits, urlencode,
};
use hy_core::clock::Clock;
use hy_core::protocol::hex;
use hy_core::protocol::words::{device_check_code, fingerprint_phrase};
use hy_core::secret::Secret;
use hy_server::server::DeviceState;

const PASS: &str = "correct horse battery";

async fn harness() -> Harness {
    let h = Harness::start().await;
    h.server
        .set_admin_passphrase(&Secret::new(PASS.to_owned()))
        .unwrap();
    h
}

fn host(h: &Harness) -> String {
    format!("127.0.0.1:{}", h.admin.port())
}

fn origin(h: &Harness) -> String {
    format!("http://{}", host(h))
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s12a_the_admin_page_is_never_served_on_a_lan_address() {
    let h = harness().await;
    for addr in ["0.0.0.0:0", "[::]:0"] {
        let Ok(listener) = tokio::net::TcpListener::bind(addr).await else {
            continue;
        };
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let err = hy_server::serve::serve_admin(listener, h.server.clone(), rx)
            .await
            .expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
    // The LAN API listener has no admin routes.
    let c = h.client(1);
    for path in ["/", "/login", "/users", "/slips", "/devices/confirm"] {
        let raw = c.raw(reqwest::Method::GET, path, None, None).await.unwrap();
        assert_eq!(raw.status, 404, "{path}");
        assert_eq!(common::error_of(&raw), "protocol_unsupported");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s12b_cross_origin_and_csrf_less_posts_fail() {
    let h = harness().await;
    let cookie = admin_login(&h, PASS).await;
    let (_, _, page) = http(
        h.admin,
        &admin_request("GET", "/", &host(&h), None, Some(&cookie), None),
    )
    .await;
    let csrf = csrf_in(&page);
    let form = format!("csrf={csrf}&alias=evil");
    let other = format!("http://127.0.0.1:{}", h.admin.port() + 1);
    for (name, o, f) in [
        ("another port", Some(other.as_str()), form.clone()),
        (
            "another site",
            Some("http://attacker.example"),
            form.clone(),
        ),
        ("null origin", Some("null"), form.clone()),
        ("no origin", None, form.clone()),
    ] {
        let (status, _, _) = http(
            h.admin,
            &admin_request("POST", "/users", &host(&h), o, Some(&cookie), Some(&f)),
        )
        .await;
        assert_eq!(status, 403, "{name}");
    }
    // Same origin but no or a wrong CSRF token.
    for f in ["alias=evil".to_owned(), "csrf=00&alias=evil".to_owned()] {
        let (status, _, _) = http(
            h.admin,
            &admin_request(
                "POST",
                "/users",
                &host(&h),
                Some(&origin(&h)),
                Some(&cookie),
                Some(&f),
            ),
        )
        .await;
        assert!(
            status == 403 || status == 422 || status == 400,
            "{f}: {status}"
        );
    }
    assert!(h.server.users().unwrap().is_empty(), "nothing was created");
    // The real form works.
    let (status, _, _) = http(
        h.admin,
        &admin_request(
            "POST",
            "/users",
            &host(&h),
            Some(&origin(&h)),
            Some(&cookie),
            Some(&form),
        ),
    )
    .await;
    assert_eq!(status, 303);
    assert_eq!(h.server.users().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s12c_a_foreign_host_header_is_refused() {
    let h = harness().await;
    let cookie = admin_login(&h, PASS).await;
    for bad in [
        "attacker.example",
        "attacker.example:80",
        "127.0.0.1",
        "192.168.1.5:8444",
    ] {
        let (status, head, body) = http(
            h.admin,
            &admin_request("GET", "/", bad, None, Some(&cookie), None),
        )
        .await;
        assert_eq!(status, 421, "{bad}");
        assert!(!body.contains("<table"), "no data for {bad}");
        assert!(
            head.to_ascii_lowercase()
                .contains("content-security-policy: default-src 'none'")
        );
    }
    let (status, head, _) = http(
        h.admin,
        &admin_request("GET", "/", &host(&h), None, Some(&cookie), None),
    )
    .await;
    assert_eq!(status, 200);
    let lower = head.to_ascii_lowercase();
    assert!(lower.contains("x-frame-options: deny"));
    assert!(lower.contains("cache-control: no-store"));
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s12d_wrong_passphrases_get_exponential_backoff() {
    let h = harness().await;
    let login = |p: &str| {
        admin_request(
            "POST",
            "/login",
            &host(&h),
            Some(&origin(&h)),
            None,
            Some(&format!("passphrase={}", urlencode(p))),
        )
    };
    let locked = |h: &Harness| -> (i64, i64) {
        h.server
            .db()
            .query_row(
                "SELECT failures, locked_until FROM admin WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    let now = || h.clock.now().timestamp_millis();
    let mut waits = Vec::new();
    for i in 1..=5 {
        let (status, _, _) = http(h.admin, &login("wrong passphrase!")).await;
        assert_eq!(status, 401, "attempt {i}");
        let (failures, until) = locked(&h);
        assert_eq!(failures, i);
        waits.push((until - now()) / 1000);
        // Even the right passphrase is refused while locked.
        let (status, _, body) = http(h.admin, &login(PASS)).await;
        assert_eq!(status, 429);
        assert!(body.contains("Try again"));
        h.clock
            .advance(chrono::Duration::milliseconds(until - now()));
    }
    assert_eq!(waits, vec![1, 2, 4, 8, 16]);
    // Capped at 15 minutes.
    for _ in 0..10 {
        let _ = http(h.admin, &login("wrong passphrase!")).await;
        let (_, until) = locked(&h);
        assert!(until - now() <= hy_server::admin::MAX_BACKOFF_S * 1000);
        h.clock
            .advance(chrono::Duration::milliseconds(until - now()));
    }
    // The right passphrase after the wait resets the counter.
    let cookie = admin_login(&h, PASS).await;
    assert!(!cookie.is_empty());
    assert_eq!(locked(&h).0, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s12e_simultaneous_wrong_passphrases_cannot_beat_the_backoff_or_pile_up_argon2() {
    // A costlier Argon2 than the other tests so the checks would overlap if allowed.
    let config = hy_server::Config {
        argon2: (16 * 1024, 2, 1),
        ..test_config()
    };
    let h = Harness::start_with(config, test_limits()).await;
    h.server
        .set_admin_passphrase(&Secret::new(PASS.to_owned()))
        .unwrap();
    let request = admin_request(
        "POST",
        "/login",
        &host(&h),
        Some(&origin(&h)),
        None,
        Some(&format!("passphrase={}", urlencode("wrong passphrase!"))),
    );
    let mut tasks = Vec::new();
    for _ in 0..24 {
        let (addr, req) = (h.admin, request.clone());
        tasks.push(tokio::spawn(async move { http(addr, &req).await.0 }));
    }
    let mut statuses = Vec::new();
    for t in tasks {
        statuses.push(t.await.unwrap());
    }
    let wrong = statuses.iter().filter(|s| **s == 401).count();
    let refused = statuses.iter().filter(|s| **s == 429).count();
    // The clock does not move: after the first failure everything is locked, and an
    // attempt during a running check is refused without one.
    assert_eq!((wrong, refused), (1, 23), "{statuses:?}");
    let failures: i64 = h
        .server
        .db()
        .query_row("SELECT failures FROM admin WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(failures, 1);
    assert_eq!(
        h.server.peak_login_checks(),
        1,
        "one Argon2 check at a time"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_session_nothing_is_shown_or_changed() {
    let h = harness().await;
    let u = h.user("u1");
    for (method, path, form) in [
        ("GET", "/", None),
        ("POST", "/slips", Some(format!("csrf=x&user={}", hex(&u)))),
        ("POST", "/users", Some("csrf=x&alias=a".to_owned())),
    ] {
        let (status, head, body) = http(
            h.admin,
            &admin_request(
                method,
                path,
                &host(&h),
                Some(&origin(&h)),
                None,
                form.as_deref(),
            ),
        )
        .await;
        assert_eq!(status, 303, "{path}");
        assert!(head.to_ascii_lowercase().contains("location: /login"));
        assert!(!body.contains("u1"));
    }
    // A session ends after 30 idle minutes.
    let cookie = admin_login(&h, PASS).await;
    h.clock.advance(chrono::Duration::minutes(31));
    let (status, _, _) = http(
        h.admin,
        &admin_request("GET", "/", &host(&h), None, Some(&cookie), None),
    )
    .await;
    assert_eq!(status, 303);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_issues_a_slip_once_and_confirms_only_with_the_words_and_code() {
    let h = harness().await;
    let u = h.user("u1");
    let cookie = admin_login(&h, PASS).await;
    let (_, _, page) = http(
        h.admin,
        &admin_request("GET", "/", &host(&h), None, Some(&cookie), None),
    )
    .await;
    assert!(
        page.contains(&fingerprint_phrase(&h.pin())),
        "the six words are shown"
    );
    let csrf = csrf_in(&page);
    let (status, _, slip_page) = http(
        h.admin,
        &admin_request(
            "POST",
            "/slips",
            &host(&h),
            Some(&origin(&h)),
            Some(&cookie),
            Some(&format!("csrf={csrf}&user={}", hex(&u))),
        ),
    )
    .await;
    assert_eq!(status, 200);
    let at = slip_page.find("hy1-").unwrap();
    let text: String = slip_page[at..].chars().take_while(|c| *c != '<').collect();
    let slip = hy_core::protocol::slip::Slip::parse(&text).unwrap();
    // The overview never shows the slip again.
    let (_, _, page) = http(
        h.admin,
        &admin_request("GET", "/", &host(&h), None, Some(&cookie), None),
    )
    .await;
    assert!(!page.contains(&text));

    let (r, d) = h.pair(&slip, "PC-1").await.unwrap();
    let (_, _, page) = http(
        h.admin,
        &admin_request("GET", "/", &host(&h), None, Some(&cookie), None),
    )
    .await;
    // The waiting PC shows the same check code as the companion.
    assert!(page.contains(&format!("<strong>{}</strong>", r.check_code)));
    assert!(page.contains("PC-1"));
    let confirm = |extra: &str| {
        admin_request(
            "POST",
            "/devices/confirm",
            &host(&h),
            Some(&origin(&h)),
            Some(&cookie),
            Some(&format!("csrf={csrf}&device={}{extra}", hex(&d.device_ref))),
        )
    };
    // Without ticking "the six words match", or with a wrong code: nothing changes.
    let code = device_check_code(&d.device_ref);
    let (s1, _, _) = http(h.admin, &confirm(&format!("&check_code={code}"))).await;
    let wrong = if code.starts_with('0') {
        "1111"
    } else {
        "0000"
    };
    let (s2, _, _) = http(
        h.admin,
        &confirm(&format!("&check_code={wrong}&words_match=yes")),
    )
    .await;
    assert_eq!((s1, s2), (400, 409));
    let state = |h: &Harness| h.server.devices().unwrap()[0].state;
    assert_eq!(state(&h), DeviceState::Pending);
    let (s3, _, _) = http(
        h.admin,
        &confirm(&format!("&check_code={code}&words_match=yes")),
    )
    .await;
    assert_eq!(s3, 303);
    assert_eq!(state(&h), DeviceState::Active);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s12e_a_cancelled_sign_in_keeps_the_permit_and_is_still_counted() {
    use tower::ServiceExt as _;
    // Costly enough that the check is still running when the request is cancelled.
    let config = hy_server::Config {
        argon2: (32 * 1024, 4, 1),
        ..test_config()
    };
    let h = Harness::start_with(config, test_limits()).await;
    h.server
        .set_admin_passphrase(&Secret::new(PASS.to_owned()))
        .unwrap();
    let form = format!("passphrase={}", urlencode("wrong passphrase!"));
    // The request is driven in a task we can abort, exactly as a dropped connection or
    // the connection timeout drops the handler.
    let router = hy_server::admin::router(h.server.clone(), h.admin.port());
    let req = axum::http::Request::post("/login")
        .header("host", host(&h))
        .header("origin", origin(&h))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(axum::body::Body::from(form.clone()))
        .unwrap();
    let inflight = tokio::spawn(router.oneshot(req));
    let start = std::time::Instant::now();
    while h.server.login_checks_running() == 0 {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "check never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    inflight.abort();
    assert!(inflight.await.unwrap_err().is_cancelled());
    // The cancelled check still holds the permit: another sign-in is refused at once.
    assert_eq!(
        h.server.login_checks_running(),
        1,
        "the check outlives the request"
    );
    let request = admin_request(
        "POST",
        "/login",
        &host(&h),
        Some(&origin(&h)),
        None,
        Some(&form),
    );
    let (status, _, _) = http(h.admin, &request).await;
    assert_eq!(status, 429);
    // When it finishes, the cancelled failure is counted and the backoff applies.
    while h.server.login_checks_running() != 0 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    // The permit is released just after the counter drops; let it go.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let (failures, locked_until): (i64, i64) = h
        .server
        .db()
        .query_row(
            "SELECT failures, locked_until FROM admin WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(failures, 1, "the cancelled attempt was counted");
    assert!(locked_until > h.clock.now().timestamp_millis());
    let (status, _, _) = http(h.admin, &request).await;
    assert_eq!(status, 429, "locked by the counted failure");
    assert_eq!(h.server.peak_login_checks(), 1, "never two checks at once");
}
