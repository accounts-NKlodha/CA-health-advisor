//! Shared harness for the V1-5a integration tests: the real server (database, keys,
//! TLS listener, admin listener) on loopback ports, a manual clock, and the companion's
//! real pairing client from `hy_core::link`.
//!
//! Each test binary uses part of this module, so unused items are allowed.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use hy_core::clock::{Clock, ManualClock};
use hy_core::link::flow::{
    DeviceFacts, FlowError, Redeemed, SyncOutcome, fetch_state, redeem_slip, sync_once,
};
use hy_core::link::{LinkClient, LinkOptions, RawResponse};
use hy_core::protocol::slip::Slip;
use hy_core::protocol::wire::StateResponse;
use hy_core::secret::{EphemeralVault, Secret, TokenVault};
use hy_core::store::{NewLink, Store};
use hy_server::keys::{ServerKeys, UnprotectedForTests};
use hy_server::serve::{self, Limits};
use hy_server::server::Faults;
use hy_server::{Config, Server, db};
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// The wall time every test starts at.
pub fn start_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 23, 6, 0, 0).unwrap()
}

/// A loopback address `127.0.0.n` (each test client can be its own "LAN host").
pub fn loopback(n: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, n))
}

/// Settings for tests: cheap Argon2 and a short pairing floor.
pub fn test_config() -> Config {
    Config {
        host_name: "127.0.0.1".into(),
        api_port: 0,
        pair_floor: Duration::from_millis(20),
        argon2: (256, 1, 1),
        ..Config::default()
    }
}

/// Short limits so idle-connection tests finish quickly.
pub fn test_limits() -> Limits {
    Limits {
        handshake_timeout: Duration::from_secs(2),
        header_timeout: Duration::from_secs(1),
        connection_cap: Duration::from_secs(20),
        ..Limits::default()
    }
}

/// Client options for tests.
pub fn opts() -> LinkOptions {
    LinkOptions {
        local_address: None,
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(15),
    }
}

/// Client options binding the client to `127.0.0.n`.
pub fn opts_from(n: u8) -> LinkOptions {
    LinkOptions {
        local_address: Some(loopback(n)),
        ..opts()
    }
}

/// A running server.
pub struct Harness {
    pub server: Arc<Server>,
    pub clock: Arc<ManualClock>,
    pub api: SocketAddr,
    pub admin: SocketAddr,
    pub dir: PathBuf,
    /// The control-log mirror (a sibling folder of `dir`, standing in for another volume).
    pub mirror: PathBuf,
    /// What the startup comparison found.
    pub startup: hy_server::recovery::Startup,
    _tmp: Option<tempfile::TempDir>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl Harness {
    /// A fresh server with the test settings.
    pub async fn start() -> Self {
        Self::start_with(test_config(), test_limits()).await
    }

    /// A fresh server with these settings.
    pub async fn start_with(config: Config, limits: Limits) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data");
        std::fs::create_dir_all(&dir).unwrap();
        let clock = Arc::new(ManualClock::new(start_time()));
        let mut h = Self::open(&dir, clock, config, limits).await;
        h._tmp = Some(tmp);
        h
    }

    /// The default mirror for a data folder: `../mirror/control.mirror`.
    pub fn mirror_for(dir: &Path) -> PathBuf {
        let m = dir.parent().unwrap().join("mirror");
        std::fs::create_dir_all(&m).unwrap();
        m.join("control.mirror")
    }

    /// Open (or create) the server in `dir`, as a service start does, with the default
    /// mirror.
    pub async fn open(dir: &Path, clock: Arc<ManualClock>, config: Config, limits: Limits) -> Self {
        let mirror = Self::mirror_for(dir);
        Self::open_with(dir, &mirror, clock, config, limits)
            .await
            .unwrap()
    }

    /// Open the server in `dir` with the mirror at `mirror`; runs the startup comparison
    /// (contract 7.3) before serving and returns its refusal, if any.
    pub async fn open_with(
        dir: &Path,
        mirror: &Path,
        clock: Arc<ManualClock>,
        config: Config,
        limits: Limits,
    ) -> Result<Self, String> {
        Self::open_full(dir, mirror, clock, config, limits, Faults::default(), 0).await
    }

    /// As [`Harness::open_with`], with faults injected before the startup comparison,
    /// listening on `port` for the API (0: any free port). A restart keeps the port, as
    /// the real service does, so paired PCs reach the server again at their known
    /// address.
    pub async fn open_full(
        dir: &Path,
        mirror: &Path,
        clock: Arc<ManualClock>,
        mut config: Config,
        limits: Limits,
        faults: Faults,
        port: u16,
    ) -> Result<Self, String> {
        let keys = if ServerKeys::exist(dir) {
            ServerKeys::load(dir, &UnprotectedForTests).unwrap()
        } else {
            ServerKeys::create(dir, &UnprotectedForTests, &config.host_name, 2026).unwrap()
        };
        let conn = db::open(&dir.join("server.db")).unwrap();
        let mut tries = 0;
        let api_listener = loop {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => break l,
                Err(e) if tries < 50 => {
                    tries += 1;
                    let _ = e;
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(e) => panic!("rebinding the API port: {e}"),
            }
        };
        let admin_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let api = api_listener.local_addr().unwrap();
        let admin = admin_listener.local_addr().unwrap();
        config.api_port = api.port();
        let clock_dyn: Arc<dyn Clock> = clock.clone();
        let server = Arc::new(Server::new(conn, keys, clock_dyn, config));
        server.inject_faults(faults);
        let startup = server.startup(mirror)?;
        let (tx, rx) = watch::channel(false);
        let s1 = server.clone();
        let rx1 = rx.clone();
        let t1 = tokio::spawn(async move {
            serve::serve_api(api_listener, s1, limits, rx1)
                .await
                .unwrap();
        });
        let s2 = server.clone();
        let t2 = tokio::spawn(async move {
            serve::serve_admin(admin_listener, s2, rx).await.unwrap();
        });
        Ok(Self {
            server,
            clock,
            api,
            admin,
            dir: dir.to_path_buf(),
            mirror: mirror.to_path_buf(),
            startup,
            _tmp: None,
            shutdown: tx,
            tasks: vec![t1, t2],
        })
    }

    /// The test backup key.
    pub fn backup_key() -> hy_server::backup::BackupKey {
        hy_server::backup::BackupKey::from_bytes([7; 32])
    }

    /// Write a server backup now (into `../backups`).
    pub fn backup(&self) -> PathBuf {
        let dir = self.dir.parent().unwrap().join("backups");
        hy_server::backup::write(&self.server, &dir, &Self::backup_key())
            .unwrap()
            .path
    }

    /// Stop, restore `file` as `hy-server restore` does, and start again (the startup
    /// comparison runs against the same mirror).
    pub async fn restore(self, file: &Path) -> Self {
        self.restore_with(file, Faults::default()).await
    }

    /// As [`Harness::restore`], with faults injected before the startup comparison.
    pub async fn restore_with(self, file: &Path, faults: Faults) -> Self {
        self.restore_full(file, faults, |_| {}).await
    }

    /// As [`Harness::restore`], letting `change` alter the mirror first (an old disk
    /// image restored with the database).
    pub async fn restore_after(self, file: &Path, change: impl FnOnce(&Path)) -> Self {
        self.restore_full(file, Faults::default(), change).await
    }

    async fn restore_full(self, file: &Path, faults: Faults, change: impl FnOnce(&Path)) -> Self {
        let mirror = self.mirror.clone();
        let config = self.server.config.clone();
        let port = self.api.port();
        let (tmp, dir, clock) = self.stop().await;
        let now = clock.now().timestamp_millis();
        hy_server::backup::restore(&dir, file, &Self::backup_key(), now).unwrap();
        change(&mirror);
        let mut h = Self::open_full(&dir, &mirror, clock, config, test_limits(), faults, port)
            .await
            .unwrap();
        h._tmp = tmp;
        h
    }

    /// Stop, let `change` alter the folders (data folder, mirror path), and start again.
    pub async fn restart_after(self, change: impl FnOnce(&Path, &Path)) -> Self {
        let mirror = self.mirror.clone();
        let config = self.server.config.clone();
        let port = self.api.port();
        let (tmp, dir, clock) = self.stop().await;
        change(&dir, &mirror);
        let mut h = Self::open_full(
            &dir,
            &mirror,
            clock,
            config,
            test_limits(),
            Faults::default(),
            port,
        )
        .await
        .unwrap();
        h._tmp = tmp;
        h
    }

    /// The user's control-log tail and `mirror_hwm` (T-J6: equal before uploads).
    pub fn tail_and_hwm(&self, user: &[u8; 16]) -> (i64, i64) {
        let conn = self.server.db();
        let tail = conn
            .query_row(
                "SELECT coalesce(max(seq), 0) FROM control_log WHERE user_id = ?1",
                [user.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        let hwm = conn
            .query_row(
                "SELECT coalesce((SELECT seq FROM mirror_hwm WHERE user_id = ?1), 0)",
                [user.as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        (tail, hwm)
    }

    /// Audit rows of one event.
    pub fn audits(&self, event: &str) -> i64 {
        self.server
            .db()
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE event = ?1",
                [event],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Stop and open again on the same folders (a service restart).
    pub async fn restart(self) -> Self {
        let mirror = self.mirror.clone();
        let config = self.server.config.clone();
        let port = self.api.port();
        let (tmp, dir, clock) = self.stop().await;
        let mut h = Self::open_full(
            &dir,
            &mirror,
            clock,
            config,
            test_limits(),
            Faults::default(),
            port,
        )
        .await
        .unwrap();
        h._tmp = tmp;
        h
    }

    /// Keep `tmp` (returned by [`Harness::stop`]) alive as long as this server runs.
    pub fn keep(&mut self, tmp: Option<tempfile::TempDir>) {
        self._tmp = tmp;
    }

    /// Stop the listeners and close the database; returns the folder (kept alive) and
    /// the clock so the server can be opened again.
    pub async fn stop(mut self) -> (Option<tempfile::TempDir>, PathBuf, Arc<ManualClock>) {
        let _ = self.shutdown.send(true);
        for t in self.tasks.drain(..) {
            let _ = t.await;
        }
        (self._tmp.take(), self.dir.clone(), self.clock.clone())
    }

    /// The pin.
    pub fn pin(&self) -> [u8; 32] {
        self.server.keys.fingerprint
    }

    /// A new user.
    pub fn user(&self, alias: &str) -> [u8; 16] {
        self.server.create_user(alias).unwrap()
    }

    /// A slip for a user.
    pub fn slip(&self, user: &[u8; 16]) -> Slip {
        self.server.issue_slip(user).unwrap()
    }

    /// A client pinned to this server.
    pub fn client(&self, n: u8) -> LinkClient {
        LinkClient::new(self.pin(), self.api, opts_from(n)).unwrap()
    }

    /// Redeem a slip with the companion's flow from `127.0.0.1`.
    pub async fn pair(&self, slip: &Slip, label: &str) -> Result<(Redeemed, Device), FlowError> {
        self.pair_as(slip, label, new_id(), opts()).await
    }

    /// Redeem with a given origin device and client options.
    pub async fn pair_as(
        &self,
        slip: &Slip,
        label: &str,
        origin: Uuid,
        options: LinkOptions,
    ) -> Result<(Redeemed, Device), FlowError> {
        let vault = EphemeralVault::new();
        let facts = DeviceFacts {
            origin_device: origin,
            label: label.into(),
            app_version: "0.5.0".into(),
            windows_build: "26100".into(),
        };
        let r = redeem_slip(slip, &facts, &vault, options).await?;
        let token = vault.open(&r.link.token_blob).unwrap();
        let device = Device {
            device_ref: *r.link.device_ref.as_bytes(),
            user_id: *r.link.user_id.as_bytes(),
            token,
            link: r.link.clone(),
            vault: Arc::new(vault),
        };
        Ok((r, device))
    }

    /// Confirm a device as the administrator would (the check code read out).
    pub fn confirm(&self, d: &Device) {
        let code = hy_core::protocol::words::device_check_code(&d.device_ref);
        self.server.confirm_device(&d.device_ref, &code).unwrap();
    }

    /// A user with one paired and confirmed device.
    pub async fn confirmed_device(&self, alias: &str, label: &str) -> Device {
        let u = self.user(alias);
        let (_, d) = self.pair(&self.slip(&u), label).await.unwrap();
        self.confirm(&d);
        d
    }

    /// A companion PC for `user`: a store whose device id is the origin it pairs with,
    /// paired and confirmed (not yet synced).
    pub async fn pc(&self, user: &[u8; 16], label: &str) -> Pc {
        let pc = self.pending_pc(user, label).await;
        self.confirm(&pc.dev);
        pc
    }

    /// As [`Harness::pc`], left pending.
    pub async fn pending_pc(&self, user: &[u8; 16], label: &str) -> Pc {
        self.pending_pc_with(&self.slip(user), label).await
    }

    /// A pending PC paired with a given slip.
    pub async fn pending_pc_with(&self, slip: &Slip, label: &str) -> Pc {
        let clock: Arc<dyn Clock> = self.clock.clone();
        let mut store = Store::open_in_memory(clock).unwrap();
        let (_, dev) = self
            .pair_as(slip, label, store.device_id(), opts())
            .await
            .unwrap();
        store.save_pairing(&dev.link, "v1-5-notice-1").unwrap();
        Pc {
            dev,
            store: std::sync::Mutex::new(store),
        }
    }

    /// Count rows of a server table.
    pub fn count(&self, table: &str) -> i64 {
        self.server
            .db()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

/// A paired device as the tests hold it.
pub struct Device {
    pub device_ref: [u8; 16],
    pub user_id: [u8; 16],
    pub token: Secret<[u8; 32]>,
    pub link: NewLink,
    pub vault: Arc<EphemeralVault>,
}

impl Device {
    /// A companion store holding this device's pairing.
    pub fn store(&self, clock: Arc<ManualClock>) -> Store {
        let clock: Arc<dyn Clock> = clock;
        let mut store = Store::open_in_memory(clock).unwrap();
        store.save_pairing(&self.link, "v1-5-notice-1").unwrap();
        store
    }

    /// Fetch and apply state for this device's store, as one companion sync does.
    pub async fn sync(
        &self,
        store: &mut Store,
    ) -> Result<Option<hy_core::store::StateOutcome>, FlowError> {
        let link = store.server_link().unwrap().expect("paired");
        let after = store.head_receipt().unwrap().map_or(0, |r| r.head.seq);
        let fetched = fetch_state(&link, after, &*self.vault, opts()).await?;
        store.set_link_last_ip(&fetched.addr.to_string()).unwrap();
        Ok(store.apply_confirmed_state(&fetched.state).unwrap())
    }
}

/// A companion PC: its pairing and its store.
pub struct Pc {
    pub dev: Device,
    pub store: std::sync::Mutex<Store>,
}

impl Pc {
    /// Run `f` with the store.
    pub fn with<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut self.store.lock().unwrap())
    }

    /// One whole companion sync.
    pub async fn sync(&self) -> Result<SyncOutcome, FlowError> {
        let today = self.with(|s| s.today().unwrap());
        sync_once(&self.store, &*self.dev.vault, opts(), today).await
    }

    /// One whole companion sync that must succeed.
    pub async fn synced(&self) -> SyncOutcome {
        self.sync().await.unwrap()
    }

    /// Log water on this PC at `at`.
    pub fn water(&self, at: chrono::DateTime<Utc>, ml: u32) {
        self.with(|s| {
            s.append_event(
                hy_core::schema::NewEvent::manual(hy_core::schema::EventPayload::WaterLogged {
                    amount_ml: ml,
                })
                .at(at),
            )
            .unwrap();
        });
    }

    /// A client for this PC that echoes its stored receipt.
    pub fn client(&self, h: &Harness) -> LinkClient {
        let receipt = self.with(|s| s.head_receipt().unwrap().map(|r| r.to_wire()));
        h.client(1).with_receipt(receipt)
    }

    /// This PC's origin-device id.
    pub fn origin(&self) -> Uuid {
        self.with(|s| s.device_id())
    }
}

/// The JSON object keys of a response body.
pub fn keys_of(raw: &RawResponse) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_slice(&raw.body).unwrap();
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

/// The `error` enum of an error body.
pub fn error_of(raw: &RawResponse) -> String {
    let v: serde_json::Value = serde_json::from_slice(&raw.body).unwrap();
    v["error"].as_str().unwrap_or_default().to_owned()
}

/// Parse a state body.
pub fn state_of(raw: &RawResponse) -> StateResponse {
    serde_json::from_slice(&raw.body).unwrap()
}

/// Every endpoint other than pairing, with a body for the POSTs: (method, path, body).
pub fn gated_requests() -> Vec<(reqwest::Method, &'static str, Option<Vec<u8>>)> {
    use reqwest::Method;
    let control = |kind: &str| {
        Some(
            serde_json::json!({"kind": kind, "category": "hydration", "op_id": new_id()})
                .to_string()
                .into_bytes(),
        )
    };
    vec![
        (Method::GET, "/v1/history?category=hydration", None),
        (Method::POST, "/v1/control", control("enable")),
        (Method::POST, "/v1/control", control("disable")),
        (Method::POST, "/v1/control", control("delete")),
        (
            Method::POST,
            "/v1/upload",
            Some(br#"{"category":"hydration","local_date":"2026-10-22","revision":1,"payload":{"ml":987654}}"#.to_vec()),
        ),
        (Method::POST, "/v1/receipt", Some(br#"{"head":"AAAA"}"#.to_vec())),
        (Method::GET, "/v1/update/manifest", None),
    ]
}

/// A TLS server config with a fresh self-signed certificate (an impostor), ALPN `hy/1`.
pub fn impostor_config() -> (Arc<ServerConfig>, [u8; 32]) {
    let keys = ServerKeys::generate("127.0.0.1", 2026).unwrap();
    let fp = keys.fingerprint;
    (hy_server::tls::server_config(&keys).unwrap(), fp)
}

/// Serve `router` over TLS with `config` on `listener` (an attacker's or impostor's
/// server). Returns the task.
pub fn serve_tls(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    router: axum::Router,
) -> JoinHandle<()> {
    use hyper_util::rt::{TokioIo, TokioTimer};
    use hyper_util::service::TowerToHyperService;
    use tower::ServiceExt;
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let router = router.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let svc = router.map_request(|r: hyper::Request<hyper::body::Incoming>| {
                    r.map(axum::body::Body::new)
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .timer(TokioTimer::new())
                    .serve_connection(TokioIo::new(tls), TowerToHyperService::new(svc))
                    .await;
            });
        }
    })
}

/// A minimal HTTP/1.1 exchange over plain TCP, for the admin page (so tests control
/// every header, including `Host`). Returns (status, headers, body).
pub async fn http(addr: SocketAddr, request: &str) -> (u16, String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, head.to_owned(), body.to_owned())
}

/// Build an admin request. `form` is sent as `application/x-www-form-urlencoded`.
pub fn admin_request(
    method: &str,
    path: &str,
    host: &str,
    origin: Option<&str>,
    cookie: Option<&str>,
    form: Option<&str>,
) -> String {
    let mut r = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(o) = origin {
        r.push_str(&format!("Origin: {o}\r\n"));
    }
    if let Some(c) = cookie {
        r.push_str(&format!("Cookie: hy_admin={c}\r\n"));
    }
    match form {
        Some(f) => r.push_str(&format!(
            "Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{f}",
            f.len()
        )),
        None => r.push_str("\r\n"),
    }
    r
}

/// Sign in to the admin page; returns the session cookie value.
pub async fn admin_login(h: &Harness, passphrase: &str) -> String {
    let host = format!("127.0.0.1:{}", h.admin.port());
    let origin = format!("http://{host}");
    let form = format!("passphrase={}", urlencode(passphrase));
    let (status, head, _) = http(
        h.admin,
        &admin_request("POST", "/login", &host, Some(&origin), None, Some(&form)),
    )
    .await;
    assert_eq!(status, 303, "sign-in should redirect: {head}");
    head.lines()
        .find_map(|l| {
            let l = l
                .strip_prefix("set-cookie: ")
                .or_else(|| l.strip_prefix("Set-Cookie: "))?;
            l.strip_prefix("hy_admin=")?
                .split(';')
                .next()
                .map(str::to_owned)
        })
        .expect("session cookie")
}

/// The CSRF token in a page.
pub fn csrf_in(html: &str) -> String {
    let at = html.find("name=\"csrf\" value=\"").expect("csrf field") + 19;
    html[at..at + 64].to_owned()
}

/// Percent-encode for a form body.
pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A random id (an origin device or an operation).
pub fn new_id() -> Uuid {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).unwrap();
    Uuid::from_bytes(b)
}

/// A pinned TLS 1.3 connection with ALPN `hy/1` from `127.0.0.n`, for tests that write
/// raw HTTP (malformed headers, slow senders, huge bodies).
pub async fn tls_connect(
    addr: SocketAddr,
    pin: [u8; 32],
    n: u8,
) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
    let tcp = connect_from(addr, n).await.unwrap();
    let mut cfg = rustls::ClientConfig::builder_with_provider(hy_core::link::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(hy_core::link::PinnedVerifier::new(pin)))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![hy_core::protocol::ALPN.to_vec()];
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
        .connect(
            rustls::pki_types::ServerName::try_from(hy_core::protocol::SNI).unwrap(),
            tcp,
        )
        .await
        .unwrap()
}

/// A TCP connection from `127.0.0.n`.
pub async fn connect_from(addr: SocketAddr, n: u8) -> std::io::Result<tokio::net::TcpStream> {
    let sock = tokio::net::TcpSocket::new_v4()?;
    sock.bind(SocketAddr::new(loopback(n), 0))?;
    sock.connect(addr).await
}

/// Send one raw HTTP request over pinned TLS and return (status, body).
pub async fn tls_request(h: &Harness, n: u8, request: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tls = tls_connect(h.api, h.pin(), n).await;
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), tls.read_to_end(&mut out)).await;
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |x| x.1).to_owned();
    (status, body)
}
