//! Shutdown with work in flight (V1-5 acceptance follow-up): stopping the listeners must
//! leave no task or blocking operation of theirs holding the server, so the caller's
//! reference is the last one and the database can be closed, as before an in-process
//! restore (`hy-sim`) replaces it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use common::{Harness, admin_request, tls_connect, urlencode};
use hy_core::secret::Secret;
use tokio::io::AsyncWriteExt;

const PASS: &str = "correct horse battery";

async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < Duration::from_secs(20), "{what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_waits_for_blocking_operations_and_admin_connections_in_flight() {
    let h = Harness::start().await;
    h.server
        .set_admin_passphrase(&Secret::new(PASS.to_owned()))
        .unwrap();
    let server = h.server.clone();

    // Hold the database, so every operation that reaches it blocks there and stays in
    // flight until released.
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = {
        let s = server.clone();
        std::thread::spawn(move || {
            let guard = s.db();
            locked_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            drop(guard);
        })
    };
    locked_rx.recv().unwrap();

    // 1. A LAN API request whose blocking operation (the pairing rate count) waits on the
    // database.
    let mut tls = tls_connect(h.api, h.pin(), 1).await;
    let body = "{}";
    let request = format!(
        "POST /v1/pair HTTP/1.1\r\nHost: hy\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(request.as_bytes()).await.unwrap();
    tls.flush().await.unwrap();
    // 2. An admin sign-in whose blocking check waits on the database.
    let host = format!("127.0.0.1:{}", h.admin.port());
    let origin = format!("http://{host}");
    let form = format!("passphrase={}", urlencode(PASS));
    let login = admin_request("POST", "/login", &host, Some(&origin), None, Some(&form));
    let mut admin = tokio::net::TcpStream::connect(h.admin).await.unwrap();
    admin.write_all(login.as_bytes()).await.unwrap();
    // 3. An admin connection that has sent nothing yet (its task holds the router).
    let idle_admin = tokio::net::TcpStream::connect(h.admin).await.unwrap();

    wait_for("both blocking operations start", || {
        server.blocking_in_flight() == 2
    })
    .await;

    // Shut down with all three in flight: the listeners must not return while the
    // blocking operations still run.
    let stopping = tokio::spawn(h.stop());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !stopping.is_finished(),
        "the listeners returned while blocking operations were still running"
    );
    assert_eq!(server.blocking_in_flight(), 2);

    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let (_tmp, _dir, _clock) = tokio::time::timeout(Duration::from_secs(20), stopping)
        .await
        .expect("the listeners return once the operations end")
        .unwrap();
    assert_eq!(server.blocking_in_flight(), 0);
    // Nothing either listener started holds the server: this reference is the last, so
    // the database closes here.
    assert_eq!(
        Arc::strong_count(&server),
        1,
        "a task or blocking operation still references the server after shutdown"
    );
    Arc::try_unwrap(server)
        .map(drop)
        .expect("the last reference");
    drop((tls, admin, idle_admin));
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_with_nothing_in_flight_returns_at_once() {
    let h = Harness::start().await;
    let server = h.server.clone();
    let _idle_admin = tokio::net::TcpStream::connect(h.admin).await.unwrap();
    let _idle_api = tls_connect(h.api, h.pin(), 2).await;
    tokio::time::timeout(Duration::from_secs(5), h.stop())
        .await
        .expect("no wait without work in flight");
    assert_eq!(Arc::strong_count(&server), 1);
}
