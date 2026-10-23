//! Abuse tests (V1-5 contract 4.4, 5 and 14.2): T-S5 slip guessing, T-S6 pairing replay,
//! T-S7 token misuse, T-S8 cross-user isolation, T-S22 denial of service.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Harness, error_of, opts_from, test_config, test_limits, tls_request};
use hy_core::protocol::slip::encode_secret;
use hy_core::protocol::wire::{PairRequest, StateResponse};
use hy_core::protocol::{PROTOCOL, b64};
use hy_core::secret::Secret;
use hy_server::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The load-heavy tests run one at a time: run side by side on a two-core CI runner they
/// starve each other's handshakes and timings, which tests the runner, not the server.
static HEAVY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn pair_body(secret: &[u8; 10], origin: uuid::Uuid) -> Vec<u8> {
    serde_json::to_vec(&PairRequest {
        protocol: PROTOCOL,
        secret: encode_secret(secret).expose().clone(),
        origin_device: origin,
        device_label: "PC".into(),
        app_version: "0.5.0".into(),
        windows_build: "26100".into(),
    })
    .unwrap()
}

fn random_secret() -> [u8; 10] {
    let mut s = [0u8; 10];
    getrandom::fill(&mut s).unwrap();
    s
}

/// Send `count` random-secret pairing attempts from `127.0.0.n` over `workers`
/// connections (fewer than the 8 allowed per address, so none is refused while the
/// server is still releasing the previous one); returns (slip_invalid answers, rate_limited answers).
async fn guess(h: &Arc<Harness>, n: u8, count: usize, workers: usize) -> (usize, usize) {
    let mut tasks = Vec::new();
    for w in 0..workers {
        let h = h.clone();
        let mine = count / workers + usize::from(w < count % workers);
        tasks.push(tokio::spawn(async move {
            let c = h.client(n);
            let (mut invalid, mut limited) = (0, 0);
            for _ in 0..mine {
                let body = pair_body(&random_secret(), common::new_id());
                // A connection the OS or a busy runner drops before any HTTP byte was
                // answered is retried; it never reached the pairing code.
                let mut tries = 0;
                let raw = loop {
                    match c
                        .raw(reqwest::Method::POST, "/v1/pair", None, Some(body.clone()))
                        .await
                    {
                        Ok(raw) => break raw,
                        Err(hy_core::link::LinkError::Unreachable) if tries < 3 => tries += 1,
                        Err(e) => panic!("{e}"),
                    }
                };
                match (raw.status, error_of(&raw).as_str()) {
                    (403, "slip_invalid") => invalid += 1,
                    (429, "rate_limited") => limited += 1,
                    other => panic!("unexpected {other:?}"),
                }
            }
            (invalid, limited)
        }));
    }
    let mut total = (0, 0);
    for t in tasks {
        let (a, b) = t.await.unwrap();
        total.0 += a;
        total.1 += b;
    }
    total
}

fn fast_config() -> Config {
    Config {
        pair_floor: Duration::from_millis(1),
        ..test_config()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s5_a_thousand_guesses_from_one_ip_hit_the_limit() {
    let _heavy = HEAVY.lock().await;
    let h = Arc::new(Harness::start_with(fast_config(), test_limits()).await);
    let u = h.user("u1");
    let slip = h.slip(&u);
    let (invalid, limited) = guess(&h, 2, 1000, 4).await;
    assert_eq!(
        (invalid, limited),
        (5, 995),
        "5 per IP per minute, then 429"
    );
    // A rate-limited answer says when to retry.
    let raw = h
        .client(2)
        .raw(
            reqwest::Method::POST,
            "/v1/pair",
            None,
            Some(pair_body(&random_secret(), common::new_id())),
        )
        .await
        .unwrap();
    assert_eq!(raw.status, 429);
    // The genuine slip, from the same address, works after the window.
    h.clock.advance(chrono::Duration::minutes(1));
    h.pair_as(&slip, "PC-1", common::new_id(), opts_from(2))
        .await
        .unwrap();
    assert_eq!(h.count("devices"), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s5_twenty_ips_hit_the_global_limit_which_survives_a_restart() {
    let _heavy = HEAVY.lock().await;
    let h = Arc::new(Harness::start_with(fast_config(), test_limits()).await);
    let u = h.user("u1");
    let slip = h.slip(&u);
    let mut tasks = Vec::new();
    for n in 2..22u8 {
        let h = h.clone();
        tasks.push(tokio::spawn(async move { guess(&h, n, 50, 1).await }));
    }
    let mut invalid = 0;
    for t in tasks {
        let (i, l) = t.await.unwrap();
        assert!(i <= 5, "per-IP limit");
        assert_eq!(i + l, 50);
        invalid += i;
    }
    assert_eq!(invalid, 60, "60 attempts per hour in total");
    h.clock.advance(chrono::Duration::minutes(2));
    assert_eq!(
        guess(&h, 30, 3, 1).await,
        (0, 3),
        "global limit still applies"
    );

    // Restart: the counters are in the database.
    let h = Arc::try_unwrap(h).ok().unwrap();
    let (tmp, dir, clock) = h.stop().await;
    let h = Arc::new(Harness::open(&dir, clock, fast_config(), test_limits()).await);
    assert_eq!(
        guess(&h, 31, 3, 1).await,
        (0, 3),
        "limits survive a restart"
    );
    // After the window the genuine slip still works.
    h.clock.advance(chrono::Duration::hours(1));
    let moved = hy_core::protocol::slip::Slip::new(
        "127.0.0.1",
        h.api.port(),
        h.pin(),
        *slip.secret.expose(),
    )
    .unwrap();
    h.pair(&moved, "PC-1").await.unwrap();
    drop(tmp);
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s5_unknown_used_and_expired_slips_answer_in_the_same_time() {
    let _heavy = HEAVY.lock().await;
    let floor = Duration::from_millis(150);
    let config = Config {
        pair_floor: floor,
        pair_limits: (10_000, 10_000, 10_000),
        ..test_config()
    };
    let h = Harness::start_with(config, test_limits()).await;
    let u = h.user("u1");
    let rounds = 8;
    // Expired slips: issued, then a day passes. (Three per user at most.)
    let mut expired = Vec::new();
    for i in 0..rounds {
        let user = h.user(&format!("e{i}"));
        expired.push(*h.slip(&user).secret.expose());
    }
    h.clock.advance(chrono::Duration::hours(25));
    let mut used = Vec::new();
    for _ in 0..rounds {
        let s = h.slip(&u);
        h.pair(&s, "PC").await.unwrap();
        used.push(*s.secret.expose());
    }
    let c = h.client(1);
    let time = |secrets: Vec<[u8; 10]>| {
        let c = c.clone();
        async move {
            let mut out = Vec::new();
            for s in secrets {
                let start = Instant::now();
                let raw = c
                    .raw(
                        reqwest::Method::POST,
                        "/v1/pair",
                        None,
                        Some(pair_body(&s, common::new_id())),
                    )
                    .await
                    .unwrap();
                out.push(start.elapsed());
                assert_eq!(error_of(&raw), "slip_invalid");
            }
            out
        }
    };
    // One warm-up answer, then the three kinds interleaved, so scheduling noise falls
    // on all of them alike.
    let _ = time(vec![random_secret()]).await;
    let (mut unknown_t, mut used_t, mut expired_t) = (Vec::new(), Vec::new(), Vec::new());
    for r in 0..rounds {
        unknown_t.extend(time(vec![random_secret()]).await);
        used_t.extend(time(vec![used[r]]).await);
        expired_t.extend(time(vec![expired[r]]).await);
    }
    let (unknown, used, expired) = (unknown_t, used_t, expired_t);
    for t in unknown.iter().chain(&used).chain(&expired) {
        assert!(*t >= floor, "every answer waits for the floor: {t:?}");
    }
    let m = [median(unknown), median(used), median(expired)];
    let spread = *m.iter().max().unwrap() - *m.iter().min().unwrap();
    // Generous bound: the floor hides the database work; only scheduling noise remains.
    assert!(spread < Duration::from_millis(75), "medians {m:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s6_a_replayed_pairing_body_makes_no_second_device() {
    let h = Harness::start().await;
    let u = h.user("u1");
    let slip = h.slip(&u);
    let origin = common::new_id();
    let body = pair_body(slip.secret.expose(), origin);
    let first = h
        .client(1)
        .raw(reqwest::Method::POST, "/v1/pair", None, Some(body.clone()))
        .await
        .unwrap();
    assert_eq!(first.status, 200);
    let resp: hy_core::protocol::wire::PairResponse = serde_json::from_slice(&first.body).unwrap();
    for n in 1..4 {
        let again = h
            .client(n)
            .raw(reqwest::Method::POST, "/v1/pair", None, Some(body.clone()))
            .await
            .unwrap();
        assert_eq!(
            (again.status, error_of(&again).as_str()),
            (403, "slip_invalid")
        );
    }
    assert_eq!(h.count("devices"), 1);
    assert_eq!(h.count("control_log"), 1);
    // The original result stands: the first device is still pending with its token.
    let token = Secret::new(hy_core::protocol::unb64_exact::<32>(&resp.token).unwrap());
    let st = h
        .client(1)
        .raw(reqwest::Method::GET, "/v1/state", Some(&token), None)
        .await
        .unwrap();
    assert_eq!(st.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s7_token_misuse_gets_401_with_the_right_reason() {
    let h = Harness::start().await;
    let x = h.confirmed_device("ux", "PC-X").await;
    let y = h.confirmed_device("uy", "PC-Y").await;
    let r = h.confirmed_device("ur", "PC-R").await;
    h.server.revoke_device(&r.device_ref).unwrap();
    let get = |auth: Option<String>, query: &str| {
        let mut req = format!(
            "GET /v1/state{query} HTTP/1.1\r\nHost: hy-server.invalid\r\nConnection: close\r\n"
        );
        if let Some(a) = auth {
            req.push_str(&format!("Authorization: {a}\r\n"));
        }
        req.push_str("\r\n");
        req
    };
    let mut wrong = [0u8; 32];
    getrandom::fill(&mut wrong).unwrap();
    let xt = b64(x.token.expose());
    let cases: Vec<(&str, String, u16, &str)> = vec![
        ("no header", get(None, ""), 401, "unknown_token"),
        (
            "empty",
            get(Some("Bearer ".into()), ""),
            401,
            "unknown_token",
        ),
        (
            "wrong",
            get(Some(format!("Bearer {}", b64(&wrong))), ""),
            401,
            "unknown_token",
        ),
        (
            "truncated",
            get(Some(format!("Bearer {}", &xt[..xt.len() - 2])), ""),
            401,
            "unknown_token",
        ),
        (
            "padded",
            get(Some(format!("Bearer {xt}=")), ""),
            401,
            "unknown_token",
        ),
        (
            "not bearer",
            get(Some(format!("Basic {xt}")), ""),
            401,
            "unknown_token",
        ),
        (
            "query string",
            get(None, &format!("?token={xt}&access_token={xt}")),
            401,
            "unknown_token",
        ),
        (
            "revoked",
            get(Some(format!("Bearer {}", b64(r.token.expose()))), ""),
            401,
            "device_revoked",
        ),
    ];
    for (name, req, status, error) in cases {
        let (s, body) = tls_request(&h, 1, &req).await;
        assert_eq!(s, status, "{name}: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["error"], error, "{name}");
        assert_eq!(
            v.as_object().unwrap().len(),
            1,
            "{name}: nothing else leaks"
        );
    }
    // Another user's token is that user's identity: it sees only that user's state.
    let (s, body) = tls_request(
        &h,
        1,
        &get(Some(format!("Bearer {}", b64(y.token.expose()))), ""),
    )
    .await;
    assert_eq!(s, 200);
    let st: StateResponse = serde_json::from_str(&body).unwrap();
    let StateResponse::Confirmed { alias, devices, .. } = st else {
        panic!()
    };
    assert_eq!(alias, "uy");
    assert!(devices.iter().all(|d| d.label == "PC-Y"));
    // Two Authorization headers are refused.
    let two = format!(
        "GET /v1/state HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {xt}\r\nAuthorization: Bearer {}\r\n\r\n",
        b64(y.token.expose())
    );
    assert_eq!(tls_request(&h, 1, &two).await.0, 401);
}

#[test]
fn t_s8_another_users_ids_anywhere_in_a_request_change_nothing() {
    use proptest::prelude::*;
    let _heavy = HEAVY.blocking_lock();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let (h, x, y) = rt.block_on(async {
        let h = Harness::start().await;
        let x = h.confirmed_device("ux", "PC-X").await;
        let y = h.confirmed_device("uy", "PC-Y").await;
        (h, x, y)
    });
    let state_of = |d: &common::Device| {
        rt.block_on(async {
            let raw = h
                .client(1)
                .raw(reqwest::Method::GET, "/v1/state", Some(&d.token), None)
                .await
                .unwrap();
            let StateResponse::Confirmed {
                alias,
                devices,
                entries,
                ..
            } = common::state_of(&raw)
            else {
                panic!()
            };
            (
                alias,
                devices
                    .into_iter()
                    .map(|d| (d.device_ref, d.label, d.confirmed))
                    .collect::<Vec<_>>(),
                entries,
            )
        })
    };
    let x_before = state_of(&x);
    let y_before = state_of(&y);
    let y_ids = [
        uuid::Uuid::from_bytes(y.user_id).to_string(),
        uuid::Uuid::from_bytes(y.device_ref).to_string(),
        hy_core::protocol::hex(&y.user_id),
        hy_core::protocol::hex(&y.device_ref),
    ];
    let endpoints = [
        ("GET", "/v1/state"),
        ("GET", "/v1/history"),
        ("POST", "/v1/control"),
        ("POST", "/v1/upload"),
        ("POST", "/v1/receipt"),
        ("GET", "/v1/update/manifest"),
    ];
    let names = ["user_id", "device_ref", "origin_device", "user", "device"];
    let mut runner = proptest::test_runner::TestRunner::new(proptest::test_runner::Config {
        cases: 48,
        ..proptest::test_runner::Config::default()
    });
    let xt = b64(x.token.expose());
    runner
        .run(
            &(0..endpoints.len(), 0..y_ids.len(), 0..names.len(), 0..3usize),
            |(e, id, name, place)| {
                let (method, path) = endpoints[e];
                let (key, value) = (names[name], &y_ids[id]);
                let mut target = path.to_owned();
                let mut headers = String::new();
                let mut body = String::new();
                match place {
                    0 => target.push_str(&format!("?{key}={value}&after_seq=0")),
                    1 => headers.push_str(&format!("X-{key}: {value}\r\nX-User-Id: {value}\r\n")),
                    _ => body = format!("{{\"{key}\":\"{value}\",\"category\":\"hydration\"}}"),
                }
                let req = format!(
                    "{method} {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {xt}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let (status, resp) = rt.block_on(tls_request(&h, 1, &req));
                if path == "/v1/state" {
                    prop_assert_eq!(status, 200);
                    let st: StateResponse = serde_json::from_str(&resp).unwrap();
                    let StateResponse::Confirmed { alias, devices, .. } = st else { panic!() };
                    prop_assert_eq!(&alias, &x_before.0);
                    let devs: Vec<_> = devices.into_iter().map(|d| (d.device_ref, d.label, d.confirmed)).collect();
                    prop_assert_eq!(&devs, &x_before.1);
                    prop_assert!(!resp.contains(&y_ids[0]) && !resp.contains(&y_ids[1]));
                } else if path == "/v1/update/manifest" {
                    // The update manifest arrives in V1-6.
                    prop_assert_eq!(status, 404);
                    prop_assert!(resp.contains("protocol_unsupported"));
                } else {
                    // Strict schemas: a request naming another user's ids is refused as a
                    // whole, before anything is read or written, and says nothing of Y.
                    prop_assert_eq!(status, 400, "{} {}", method, resp);
                    prop_assert!(!resp.contains(&y_ids[0]) && !resp.contains(&y_ids[1]));
                }
                Ok(())
            },
        )
        .unwrap();
    let y_after = state_of(&y);
    assert_eq!(y_after.0, y_before.0);
    assert_eq!(y_after.1, y_before.1);
    assert_eq!(y_after.2, y_before.2, "Y's chain is unchanged");
    let x_after = state_of(&x);
    assert_eq!(x_after.2, x_before.2, "nothing was recorded for X either");
    // Unpair with Y's ids in the body revokes only X.
    let body = format!("{{\"device_ref\":\"{}\"}}", y_ids[1]);
    let req = format!(
        "POST /v1/unpair HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {xt}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(rt.block_on(tls_request(&h, 1, &req)).0, 200);
    let devices = h.server.devices().unwrap();
    let state = |r: &[u8; 16]| devices.iter().find(|d| &d.device_ref == r).unwrap().state;
    assert_eq!(
        state(&x.device_ref),
        hy_server::server::DeviceState::Revoked
    );
    assert_eq!(state(&y.device_ref), hy_server::server::DeviceState::Active);
    drop(rt);
}

#[tokio::test(flavor = "multi_thread")]
async fn t_s22_idle_connections_slow_headers_huge_bodies_and_floods_leave_a_genuine_client_working()
{
    let _heavy = HEAVY.lock().await;
    let h = Arc::new(Harness::start().await);
    let d = h.confirmed_device("u1", "PC-1").await;
    let mut store = d.store(h.clock.clone());

    // 200 idle connections from one address: at most 8 are held.
    let mut idle = Vec::new();
    for _ in 0..200 {
        if let Ok(s) = common::connect_from(h.api, 2).await {
            idle.push(s);
        }
    }
    // A slowloris: a finished handshake, then one header byte every 300 ms.
    let mut slow = common::tls_connect(h.api, h.pin(), 3).await;
    let slow_task = tokio::spawn(async move {
        let start = Instant::now();
        slow.write_all(b"GET /v1/state HTTP/1.1\r\n").await.unwrap();
        for b in b"X-Drip: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".iter() {
            if slow.write_all(&[*b]).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut buf = [0u8; 256];
            if let Ok(Ok(0) | Err(_)) =
                tokio::time::timeout(Duration::from_millis(1), slow.read(&mut buf)).await
            {
                break;
            }
            let _ = &buf;
        }
        start.elapsed()
    });
    // A flood: 1,000 requests from one address as fast as they go.
    let flood = {
        let h = h.clone();
        tokio::spawn(async move {
            let mut tasks = Vec::new();
            for _ in 0..1000 {
                let c = h.client(4);
                tasks.push(tokio::spawn(async move {
                    let _ = c.raw(reqwest::Method::GET, "/v1/state", None, None).await;
                }));
            }
            for t in tasks {
                let _ = t.await;
            }
        })
    };
    // The genuine client still syncs within 5 s.
    let start = Instant::now();
    let out = d.sync(&mut store).await.unwrap();
    assert!(out.is_some());
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );

    // A 10 MB body is refused without being read.
    let mut big = common::tls_connect(h.api, h.pin(), 5).await;
    big.write_all(b"POST /v1/pair HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 10485760\r\n\r\n")
        .await
        .unwrap();
    // The answer comes from the declared length alone, before any body byte is sent.
    // (Reading after streaming the body would race the server's close: on Windows the
    // reset discards the 413 the client had not read yet.)
    let mut resp = vec![0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(5), big.read(&mut resp))
        .await
        .unwrap()
        .unwrap_or(0);
    let text = String::from_utf8_lossy(&resp[..n]);
    assert!(text.starts_with("HTTP/1.1 413"), "{text}");
    assert!(text.contains("payload_too_large"));
    // Sending the body anyway goes nowhere: the connection is closed.
    let chunk = vec![b'a'; 64 * 1024];
    let mut sent = 0usize;
    for _ in 0..160 {
        if big.write_all(&chunk).await.is_err() {
            break;
        }
        sent += chunk.len();
    }
    assert!(sent < 10 * 1024 * 1024, "the 10 MB body was accepted");

    // The slow sender was cut off near the header limit (1 s in tests, 10 s in service).
    let slow_for = slow_task.await.unwrap();
    assert!(
        slow_for < Duration::from_secs(5),
        "slowloris lasted {slow_for:?}"
    );
    // Idle connections that never start a handshake are closed after its limit.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut open = 0;
    for s in &mut idle {
        let mut b = [0u8; 1];
        if tokio::time::timeout(Duration::from_millis(50), s.read(&mut b))
            .await
            .is_err()
        {
            open += 1;
        }
    }
    assert_eq!(open, 0, "idle connections are closed");
    flood.await.unwrap();
    // And the genuine client is still fine.
    assert!(d.sync(&mut store).await.unwrap().is_some());
}
