//! HealthierYou load simulator (architecture 5.1: "3 real clients plus 10 simulated
//! clients, driven by a simulator built on the shared core crate"; V1-5 slice b).
//!
//! It runs the real server in process (database, keys, mirror, pinned TLS 1.3 on a
//! loopback port) and 10 simulated companions (5 users with 2 PCs each), each with its
//! own in-memory store and the companion's real pairing and sync code. Over a simulated
//! week on a manual clock the PCs log water, sync concurrently, switch sharing on (one
//! PC per user; the other follows remotely) and off, and delete a day. Midway a server
//! backup is taken; at the end it is restored, so every user goes into recovery hold and
//! is released as their PCs check in.
//!
//! After each phase it checks that the server holds exactly what each PC should have
//! shared, and that the control log and the mirror agree. Any difference is a failure
//! (non-zero exit code). It prints only counts and timings, never data.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{NaiveDate, TimeZone, Utc};
use hy_core::clock::{Clock, ManualClock};
use hy_core::link::LinkOptions;
use hy_core::link::flow::{DeviceFacts, SyncOutcome, redeem_slip, sync_once};
use hy_core::protocol::words::device_check_code;
use hy_core::schema::{Category, DateRange, EventPayload, NewEvent};
use hy_core::secret::EphemeralVault;
use hy_core::store::Store;
use hy_server::backup::{self, BackupKey};
use hy_server::keys::{ServerKeys, UnprotectedForTests};
use hy_server::recovery::Startup;
use hy_server::serve::{self, Limits};
use hy_server::{Config, Server, db};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

const USERS: usize = 5;
const PCS_PER_USER: usize = 2;
const DAYS: i64 = 7;
const ENTRIES_PER_DAY: u32 = 3;
const NOTICE: &str = "v1-5-sharing-1";

type Fail = String;

#[allow(clippy::needless_pass_by_value)]
fn fail(what: impl std::fmt::Display) -> Fail {
    what.to_string()
}

/// One simulated companion.
struct Pc {
    user: usize,
    store: Arc<Mutex<Store>>,
    vault: Arc<EphemeralVault>,
    /// Water this PC logged, by date.
    logged: BTreeMap<NaiveDate, u32>,
    /// Its own loopback address: each office PC is its own LAN host (the server limits
    /// connections per address).
    opts: LinkOptions,
}

impl Pc {
    fn with<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut self.store.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The running server.
struct Running {
    server: Arc<Server>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
    startup: Startup,
}

fn config(port: u16) -> Config {
    Config {
        host_name: "127.0.0.1".into(),
        api_port: port,
        pair_floor: Duration::from_millis(5),
        argon2: (256, 1, 1),
        pair_limits: (1000, 1000, 1000),
    }
}

fn opts(n: usize) -> LinkOptions {
    let n = u8::try_from(10 + n).unwrap_or(250);
    LinkOptions {
        local_address: Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, n))),
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(20),
    }
}

fn key() -> BackupKey {
    BackupKey::from_bytes([7; 32])
}

async fn start(
    dir: &Path,
    mirror: &Path,
    clock: &Arc<ManualClock>,
    port: u16,
) -> Result<Running, Fail> {
    let keys = if ServerKeys::exist(dir) {
        ServerKeys::load(dir, &UnprotectedForTests).map_err(fail)?
    } else {
        ServerKeys::create(dir, &UnprotectedForTests, "127.0.0.1", 2026).map_err(fail)?
    };
    let conn = db::open(&dir.join("server.db")).map_err(fail)?;
    let mut tries = 0;
    let listener = loop {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => break l,
            Err(_) if tries < 100 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => return Err(fail(format!("binding the API port: {e}"))),
        }
    };
    let port = listener.local_addr().map_err(fail)?.port();
    let clock_dyn: Arc<dyn Clock> = clock.clone();
    let server = Arc::new(Server::new(conn, keys, clock_dyn, config(port)));
    let startup = server.startup(mirror)?;
    let (tx, rx) = watch::channel(false);
    let s = server.clone();
    let task = tokio::spawn(async move {
        let _ = serve::serve_api(listener, s, Limits::default(), rx).await;
    });
    Ok(Running {
        server,
        shutdown: tx,
        task,
        startup,
    })
}

async fn stop(r: Running) -> Result<(), Fail> {
    let _ = r.shutdown.send(true);
    r.task.await.map_err(fail)?;
    // The database must be closed before a restore replaces it. `serve_api` returns only
    // after its connections and the blocking operations they started have ended
    // (`hy-server/tests/shutdown.rs`), and no admin listener runs here, so this is the
    // last reference; anything else is refused rather than restored over.
    Arc::try_unwrap(r.server)
        .map(drop)
        .map_err(|_| fail("the server is still referenced after shutdown"))
}

/// Sync every PC at once; returns each outcome and each sync's wall time.
async fn sync_all(pcs: &[Pc]) -> Result<(Vec<SyncOutcome>, Vec<Duration>), Fail> {
    let mut tasks = Vec::new();
    for pc in pcs {
        let store = pc.store.clone();
        let vault = pc.vault.clone();
        let opts = pc.opts;
        tasks.push(tokio::spawn(async move {
            let today = store
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .today()
                .map_err(fail)?;
            let t = Instant::now();
            let out = sync_once(&*store, &*vault, opts, today).await;
            out.map(|o| (o, t.elapsed()))
                .map_err(|e| fail(format!("sync: {e:?}")))
        }));
    }
    let mut outs = Vec::new();
    let mut times = Vec::new();
    for t in tasks {
        let (o, d) = t.await.map_err(fail)??;
        outs.push(o);
        times.push(d);
    }
    Ok((outs, times))
}

fn p95(times: &mut [Duration]) -> Duration {
    times.sort();
    times[(times.len() * 95 / 100).min(times.len() - 1)]
}

/// Water per entry for PC `i` on day `d` (deterministic, varied).
fn water(i: usize, d: i64) -> u32 {
    let i = u32::try_from(i % 64).unwrap_or(0);
    let d = u32::try_from(d.rem_euclid(64)).unwrap_or(0);
    150 + 50 * ((i * 7 + d * 3) % 9)
}

/// What the server must not hold.
struct Expect {
    /// (user, date) pairs deleted.
    deleted: Vec<(usize, NaiveDate)>,
    /// A user whose hydration sharing was switched off, and the first date not shared.
    off: Option<(usize, NaiveDate)>,
}

fn high_waters_agree(server: &Server, users: &[[u8; 16]]) -> Result<(), Fail> {
    let conn = server.db();
    for u in users {
        let (tail, hwm): (i64, i64) = conn
            .query_row(
                "SELECT coalesce((SELECT max(seq) FROM control_log WHERE user_id = ?1), 0),
                        coalesce((SELECT seq FROM mirror_hwm WHERE user_id = ?1), 0)",
                [u.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(fail)?;
        if tail != hwm {
            return Err(fail("the control log and the mirror differ"));
        }
    }
    Ok(())
}

/// Every row the server holds is a PC's own total for a shared day, and every shared
/// day is there. Returns the rows checked.
fn check(server: &Server, pcs: &[Pc], users: &[[u8; 16]], exp: &Expect) -> Result<usize, Fail> {
    high_waters_agree(server, users)?;
    let conn = server.db();
    let mut checked = 0;
    for pc in pcs {
        let origin = pc.with(|s| s.device_id());
        let rows: BTreeMap<String, u64> = conn
            .prepare(
                "SELECT local_date, payload FROM summaries WHERE user_id = ?1 AND category = 'hydration'
                 AND origin_device = ?2",
            )
            .map_err(fail)?
            .query_map(
                rusqlite::params![users[pc.user].as_slice(), origin.as_bytes().as_slice()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(fail)?
            .map(|r| {
                let (d, p) = r.map_err(fail)?;
                let v: serde_json::Value = serde_json::from_str(&p).map_err(fail)?;
                Ok((d, v["ml"].as_u64().unwrap_or(u64::MAX)))
            })
            .collect::<Result<_, Fail>>()?;
        for (date, ml) in &pc.logged {
            let unshared = exp.deleted.contains(&(pc.user, *date))
                || exp
                    .off
                    .is_some_and(|(u, from)| u == pc.user && *date >= from);
            match (rows.get(&date.to_string()), unshared) {
                (Some(_), true) => {
                    return Err(fail(format!(
                        "user {} has a row for a day it must not share",
                        pc.user
                    )));
                }
                (None, false) => {
                    return Err(fail(format!("user {} is missing a shared day", pc.user)));
                }
                (Some(got), false) if *got != u64::from(*ml) => {
                    return Err(fail(format!("user {} has a wrong total", pc.user)));
                }
                (Some(_), false) => checked += 1,
                (None, true) => {}
            }
        }
        if rows.len() > pc.logged.len() {
            return Err(fail(format!(
                "user {} has rows for days it never logged",
                pc.user
            )));
        }
    }
    Ok(checked)
}

#[allow(clippy::too_many_lines)]
async fn run(tmp: &Path) -> Result<String, Fail> {
    let dir = tmp.join("data");
    let mirror_dir = tmp.join("mirror");
    let backups = tmp.join("backups");
    for d in [&dir, &mirror_dir] {
        std::fs::create_dir_all(d).map_err(fail)?;
    }
    let mirror = mirror_dir.join("control.mirror");
    let start_at = Utc
        .with_ymd_and_hms(2026, 10, 23, 6, 0, 0)
        .single()
        .ok_or("start time")?;
    let clock = Arc::new(ManualClock::new(start_at));
    let mut srv = start(&dir, &mirror, &clock, 0).await?;
    let port = srv.server.config.api_port;
    let mut report = Vec::new();

    // Pair and confirm 10 PCs.
    let mut users = Vec::new();
    let mut pcs = Vec::new();
    for u in 0..USERS {
        let user = srv
            .server
            .create_user(&format!("sim-user-{u}"))
            .map_err(fail)?;
        users.push(user);
        for p in 0..PCS_PER_USER {
            let clock_dyn: Arc<dyn Clock> = clock.clone();
            let mut store = Store::open_in_memory(clock_dyn).map_err(fail)?;
            let slip = srv.server.issue_slip(&user).map_err(fail)?;
            let vault = EphemeralVault::new();
            let facts = DeviceFacts {
                origin_device: store.device_id(),
                label: format!("SIM-{u}-{p}"),
                app_version: env!("CARGO_PKG_VERSION").into(),
                windows_build: "26100".into(),
            };
            let opts = opts(pcs.len());
            let r = redeem_slip(&slip, &facts, &vault, opts)
                .await
                .map_err(|e| fail(format!("pairing: {e:?}")))?;
            store.save_pairing(&r.link, "v1-5-notice-1").map_err(fail)?;
            let device_ref = *r.link.device_ref.as_bytes();
            srv.server
                .confirm_device(&device_ref, &device_check_code(&device_ref))
                .map_err(fail)?;
            pcs.push(Pc {
                user: u,
                store: Arc::new(Mutex::new(store)),
                vault: Arc::new(vault),
                logged: BTreeMap::new(),
                opts,
            });
        }
    }
    let (outs, _) = sync_all(&pcs).await?;
    if !outs.iter().all(|o| matches!(o, SyncOutcome::Synced { .. })) {
        return Err(fail("a PC did not sync after confirmation"));
    }
    report.push(format!("{} PCs paired and confirmed", pcs.len()));

    // Day 0: the first PC of each user switches hydration on; the other follows remotely.
    for pc in pcs.iter().step_by(PCS_PER_USER) {
        pc.with(|s| s.request_enable(Category::Hydration, NOTICE))
            .map_err(fail)?;
    }
    let mut exp = Expect {
        deleted: Vec::new(),
        off: None,
    };
    let mut times = Vec::new();
    let mut file = None;
    for day in 0..DAYS {
        let today = clock.now().date_naive();
        if day == 3 {
            // User 1 deletes yesterday (on its first PC); user 2 switches hydration off
            // (on its second PC), so today and later are never shared.
            let y = today - chrono::Duration::days(1);
            pcs[PCS_PER_USER]
                .with(|s| s.record_deletion(Category::Hydration, DateRange { from: y, to: y }))
                .map_err(fail)?;
            exp.deleted.push((1, y));
            pcs[2 * PCS_PER_USER + 1]
                .with(|s| s.disable_sharing(Category::Hydration))
                .map_err(fail)?;
            exp.off = Some((2, today));
            // These two PCs send their changes before anyone logs today's water.
            let movers = [PCS_PER_USER, 2 * PCS_PER_USER + 1];
            for &i in &movers {
                sync_all(std::slice::from_ref(&pcs[i])).await?;
            }
        }
        let morning = today.and_hms_opt(4, 0, 0).ok_or("time")?.and_utc();
        for (i, pc) in pcs.iter_mut().enumerate() {
            for k in 0..ENTRIES_PER_DAY {
                let at = morning + chrono::Duration::minutes(10 * i64::from(k));
                pc.with(|s| {
                    s.append_event(
                        NewEvent::manual(EventPayload::WaterLogged {
                            amount_ml: water(i, day),
                        })
                        .at(at),
                    )
                })
                .map_err(fail)?;
            }
            pc.logged.insert(today, ENTRIES_PER_DAY * water(i, day));
        }
        // Rounds: control and remote changes settle, then uploads.
        for _ in 0..3 {
            let (_, mut t) = sync_all(&pcs).await?;
            times.append(&mut t);
        }
        if day == 2 {
            file = Some(
                backup::write(&srv.server, &backups, &key())
                    .map_err(fail)?
                    .path,
            );
        }
        clock.advance(chrono::Duration::days(1));
    }
    let n = check(&srv.server, &pcs, &users, &exp).map_err(|e| format!("after the week: {e}"))?;
    report.push(format!(
        "{DAYS} days synced: {n} rows verified, sync p95 {} ms",
        p95(&mut times).as_millis()
    ));

    // Restore the day-2 backup: every user is held until each of their PCs checks in.
    let file = file.ok_or("no backup was written")?;
    stop(srv).await?;
    backup::restore(&dir, &file, &key(), clock.now().timestamp_millis()).map_err(fail)?;
    srv = start(&dir, &mirror, &clock, port).await?;
    let Startup::Hold { replayed } = srv.startup else {
        return Err(fail(format!(
            "expected a hold after the restore, got {:?}",
            srv.startup
        )));
    };
    srv.server.witness_users(true).map_err(fail)?;
    for u in &users {
        srv.server.witness_user(u, true).map_err(fail)?;
    }
    let (outs, _) = sync_all(&pcs).await?;
    if !outs
        .iter()
        .all(|o| matches!(o, SyncOutcome::Hold { evidence: true, .. }))
    {
        return Err(fail("a PC's receipt was not counted during the hold"));
    }
    if !srv.server.hold_rows().map_err(fail)?.is_empty() {
        return Err(fail("a user is still held after every PC checked in"));
    }
    // Back to normal: the PCs upload again what the restore lost.
    for _ in 0..3 {
        sync_all(&pcs).await?;
    }
    let n =
        check(&srv.server, &pcs, &users, &exp).map_err(|e| format!("after the restore: {e}"))?;
    report.push(format!(
        "restore: {replayed} mirror entries replayed, {} users released, {n} rows verified",
        users.len()
    ));
    stop(srv).await?;
    Ok(format!(
        "hy-sim {}: {}",
        env!("CARGO_PKG_VERSION"),
        report.join("; ")
    ))
}

fn main() -> std::process::ExitCode {
    let tmp = std::env::temp_dir().join(format!("hy-sim-{}", std::process::id()));
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("hy-sim: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let out = rt.block_on(run(&tmp));
    let _ = std::fs::remove_dir_all(&tmp);
    match out {
        Ok(line) => {
            println!("{line}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hy-sim: FAILED: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ten_clients_sync_a_week_and_survive_a_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let line = rt.block_on(super::run(tmp.path())).unwrap();
        assert!(line.contains("10 PCs paired"), "{line}");
        assert!(line.contains("users released"), "{line}");
    }
}
