//! `hy-server`: the HealthierYou office LAN server (architecture 5).
//!
//! ```text
//! hy-server init            [--data-dir DIR] [--host NAME]   create keys and the database
//! hy-server set-passphrase  [--data-dir DIR]                 read the admin passphrase from stdin
//! hy-server fingerprint     [--data-dir DIR]                 print the fingerprint and six words
//! hy-server serve           [--data-dir DIR] --mirror FILE [--backups DIR] [--host NAME] [--api ADDR] [--admin ADDR]
//! hy-server restore         [--data-dir DIR] --backup FILE    restore a server backup (service stopped)
//! hy-server --version
//! hy-server verify-release  --file INSTALLER --current V [--previous V]
//!                                                            check a server update's signature
//!                                                            and the version policy
//! hy-server service         [--data-dir DIR] --mirror FILE [...]  run as the Windows service
//! hy-server check-mirror    [--data-dir DIR] --mirror FILE   is FILE the data's own mirror?
//! hy-server check-acl       [--data-dir DIR]                 is the data folder closed to others?
//! ```
//!
//! `--mirror` is the append-only copy of the control log (contract 7.3). It must be on
//! another volume than the data folder (a second disk or a share), so that restoring an
//! old database is detected at the next start. Server backups go to `--backups`
//! (default `<data-dir>\backups`; copy them off the PC too). Backups open only with this
//! server's `backup.key` (keys folder): if that key is lost every existing backup is
//! permanently unrecoverable. The keys are protected with machine-scope DPAPI, so a copy
//! of the keys folder opens only on this same Windows installation: it is not a way to move
//! or rebuild the server. A replacement server, an OS reinstall or keys that cannot be
//! opened take the full reset: new keys (`init` on a fresh data folder), every PC re-pairs
//! with a new slip, and the old backups cannot be opened (`docs/Server_Operations.md`).
//!
//! The data directory defaults to `%ProgramData%\HealthierYou\Server` on Windows and must
//! be given elsewhere. Running as a Windows service under a virtual account, the folder
//! ACLs and the firewall rule are set up by the server installer (slice V1-6).

use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use hy_core::clock::SystemClock;
use hy_core::protocol::words::fingerprint_phrase;
use hy_core::protocol::{ADMIN_PORT, DEFAULT_PORT, hex};
use hy_core::secret::Secret;
use hy_server::backup::{self, BackupKey};
use hy_server::keys::{ServerKeys, platform_protector, year_of};
use hy_server::logging::{self, V};
use hy_server::serve::{self, Limits};
use hy_server::{Config, Server, db};

#[cfg(windows)]
mod winservice;

struct Args {
    command: String,
    data_dir: Option<PathBuf>,
    host: Option<String>,
    api: SocketAddr,
    admin: SocketAddr,
    mirror: Option<PathBuf>,
    backups: Option<PathBuf>,
    backup: Option<PathBuf>,
    file: Option<PathBuf>,
    current: Option<String>,
    previous: Option<String>,
}

fn usage() -> String {
    "usage: hy-server <init|set-passphrase|fingerprint|serve|restore> [--data-dir DIR] [--mirror FILE] [--backups DIR] [--backup FILE] [--host NAME] [--api ADDR] [--admin ADDR]\n\
     Backups open only with backup.key in the keys folder: if that key is lost, every existing backup is \
     permanently unrecoverable. The keys are protected for this Windows installation only (machine DPAPI): \
     a copy of the keys folder is not a way to move or rebuild the server. A replacement server, an OS \
     reinstall or keys that cannot be opened mean a full reset: new keys, every PC re-pairs with a new slip, \
     and the old backups cannot be opened."
        .into()
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let command = it.next().ok_or_else(usage)?;
    let mut args = Args {
        command,
        data_dir: None,
        host: None,
        api: SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)),
        admin: SocketAddr::from(([127, 0, 0, 1], ADMIN_PORT)),
        mirror: None,
        backups: None,
        backup: None,
        file: None,
        current: None,
        previous: None,
    };
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--data-dir" => args.data_dir = Some(PathBuf::from(value)),
            "--host" => args.host = Some(value),
            "--mirror" => args.mirror = Some(PathBuf::from(value)),
            "--backups" => args.backups = Some(PathBuf::from(value)),
            "--backup" => args.backup = Some(PathBuf::from(value)),
            "--file" => args.file = Some(PathBuf::from(value)),
            "--current" => args.current = Some(value),
            "--previous" => args.previous = Some(value),
            "--api" => args.api = value.parse().map_err(|_| "bad --api address".to_owned())?,
            "--admin" => {
                args.admin = value
                    .parse()
                    .map_err(|_| "bad --admin address".to_owned())?
            }
            other => return Err(format!("unknown option {other}\n{}", usage())),
        }
    }
    Ok(args)
}

fn data_dir(args: &Args) -> Result<PathBuf, String> {
    if let Some(d) = &args.data_dir {
        return Ok(d.clone());
    }
    #[cfg(windows)]
    if let Some(pd) = std::env::var_os("ProgramData") {
        return Ok(PathBuf::from(pd).join("HealthierYou").join("Server"));
    }
    Err("give --data-dir".into())
}

fn host_name(args: &Args) -> String {
    args.host
        .clone()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "localhost".into())
}

fn print_identity(keys: &ServerKeys) {
    println!("fingerprint {}", hex(&keys.fingerprint));
    println!("six words   {}", fingerprint_phrase(&keys.fingerprint));
}

fn run() -> Result<(), String> {
    let args = parse()?;
    if args.command == "--version" {
        println!(
            "hy-server {} (hy-core {}, server schema v{})",
            env!("CARGO_PKG_VERSION"),
            hy_core::VERSION,
            db::MIGRATIONS.len()
        );
        return Ok(());
    }
    if args.command == "verify-release" {
        return verify_release(&args);
    }
    let dir = data_dir(&args)?;
    let protector = platform_protector();
    match args.command.as_str() {
        "init" => {
            let keys = init(&args, &dir, protector.as_ref())?;
            print_identity(&keys);
            Ok(())
        }
        "fingerprint" => {
            let keys = ServerKeys::load(&dir, protector.as_ref())
                .map_err(|e| format!("cannot open the keys: {e}"))?;
            print_identity(&keys);
            Ok(())
        }
        "set-passphrase" => {
            let keys = ServerKeys::load(&dir, protector.as_ref())
                .map_err(|e| format!("cannot open the keys: {e}"))?;
            let conn = db::open(&dir.join("server.db")).map_err(|e| format!("database: {e}"))?;
            let server = Server::new(conn, keys, Arc::new(SystemClock::new()), Config::default());
            eprintln!("Type the new admin passphrase (at least 12 characters) and press Enter:");
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(|e| format!("cannot read the passphrase: {e}"))?;
            let pass = Secret::new(line.trim_end_matches(['\r', '\n']).to_owned());
            line.clear();
            server
                .set_admin_passphrase(&pass)
                .map_err(|e| e.to_string())?;
            eprintln!("Admin passphrase set. Existing admin sessions were signed out.");
            Ok(())
        }
        "check-mirror" => check_mirror(&args, &dir, protector.as_ref()),
        "check-acl" => check_acl(&dir),
        "serve" => serve_cmd(
            &args,
            &dir,
            protector.as_ref(),
            Box::pin(async {
                let _ = tokio::signal::ctrl_c().await;
            }),
            &|| {},
        ),
        "restore" => {
            let file = args.backup.as_ref().ok_or("give --backup FILE")?;
            let key = BackupKey::load_or_create(&dir, protector.as_ref())
                .map_err(|e| format!("cannot open the backup key: {e}"))?;
            let now = chrono::Utc::now().timestamp_millis();
            let id = backup::restore(&dir, file, &key, now).map_err(|e| e.to_string())?;
            eprintln!(
                "Restored snapshot {}. The previous database was kept next to it. At the next start the server compares it with the mirror; users whose history is newer than this backup are held for recovery.",
                hex(&id[..4])
            );
            Ok(())
        }
        _ => Err(usage()),
    }
}

/// Create the keys and the database in a new data folder.
fn init(
    args: &Args,
    dir: &std::path::Path,
    protector: &dyn hy_server::keys::KeyProtector,
) -> Result<ServerKeys, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create the data folder: {e}"))?;
    let year = year_of(chrono::Utc::now());
    let keys = ServerKeys::create(dir, protector, &host_name(args), year)
        .map_err(|e| format!("cannot create keys: {e}"))?;
    db::open(&dir.join("server.db")).map_err(|e| format!("cannot create the database: {e}"))?;
    Ok(keys)
}

/// Check a server installer and its `.sig` with the update keys built into this server,
/// and the server's version policy (slice V1-6b). The server installer runs this with the
/// installed server before it replaces anything, so only a release signed with a trusted
/// key can update it. The policy is the companion's ([`hy_core::update::plan`]): a newer
/// version, or exactly the version this server was last updated from (`--previous`, a
/// rollback); the same version again or any other older version is refused.
fn verify_release(args: &Args) -> Result<(), String> {
    use hy_core::update::{
        Plan, Product, TrustedKeys, Version, plan, read_signature_file, sig_path_for,
        verify_release,
    };
    let file = args.file.as_ref().ok_or("give --file INSTALLER")?;
    let version = |v: &str| v.parse::<Version>().map_err(|e| e.to_string());
    let current = version(args.current.as_deref().ok_or("give --current VERSION")?)?;
    let previous = args.previous.as_deref().map(version).transpose()?;
    let keys = TrustedKeys::parse(env!("HY_UPDATE_KEYS")).map_err(|e| e.to_string())?;
    let sig = read_signature_file(&sig_path_for(file)).map_err(|e| e.to_string())?;
    let release = verify_release(&keys, file, &sig).map_err(|e| e.to_string())?;
    let what =
        match plan(Product::Server, current, previous, &release).map_err(|e| e.to_string())? {
            Plan::Upgrade => "upgrade",
            Plan::Rollback => "rollback",
        };
    println!(
        "verified {} version {} ({what})",
        release.file_name, release.version
    );
    Ok(())
}

/// Check that nobody but SYSTEM, Administrators and the service's own account owns or
/// can open anything in the data folder (slice V1-6b). The installer runs this after it
/// sets the rights and fails closed on any finding.
#[cfg(windows)]
fn check_acl(dir: &std::path::Path) -> Result<(), String> {
    use hy_server::acl::{ADMINISTRATORS, SYSTEM, evaluate, read_tree, sid_of};
    let service = sid_of(&format!("NT SERVICE\\{}", winservice::SERVICE_NAME))?;
    let entries = read_tree(dir)?;
    let problems = evaluate(dir, &entries, &[SYSTEM, ADMINISTRATORS, &service]);
    if problems.is_empty() {
        println!("rights: {} items, closed to other accounts", entries.len());
        Ok(())
    } else {
        Err(format!(
            "the data folder is open to other accounts:\n  {}",
            problems.join("\n  ")
        ))
    }
}

#[cfg(not(windows))]
fn check_acl(_dir: &std::path::Path) -> Result<(), String> {
    Err("check-acl reads Windows rights and runs on Windows only".into())
}

/// Check that `--mirror` is the mirror the data folder's database is bound to (slice
/// V1-6b). The server installer runs this, with the service stopped, before it points
/// the service at a mirror: pointing retained data at another (say empty) mirror would
/// make the next start take the global discard path.
fn check_mirror(
    args: &Args,
    dir: &std::path::Path,
    protector: &dyn hy_server::keys::KeyProtector,
) -> Result<(), String> {
    use hy_server::mirror::{Binding, check_binding};
    let mirror = args.mirror.as_ref().ok_or("give --mirror FILE")?;
    let db = dir.join("server.db");
    if !db.exists() {
        println!("mirror: no server data yet");
        return Ok(());
    }
    let keys =
        ServerKeys::load(dir, protector).map_err(|e| format!("cannot open the keys: {e}"))?;
    match check_binding(&db, &keys.key_id, mirror)? {
        Binding::NoData => println!("mirror: no server data yet"),
        Binding::NotYetBound => println!("mirror: the server data has no mirror yet"),
        Binding::Matches => println!("mirror: matches the server data"),
    }
    Ok(())
}

/// What ends `serve`: Ctrl+C on a console, the service manager's stop as a service.
type Shutdown = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// The Windows service's work (called by `winservice`): the keys are made at the first
/// start (V1-5 contract 6.2), by the service's own virtual account, then the server
/// runs until the service manager stops it. `ready` reports the service running once
/// both ports are open.
#[cfg(windows)]
fn service_body(stop: std::sync::mpsc::Receiver<()>, ready: &dyn Fn()) -> Result<(), String> {
    let args = parse()?;
    let dir = data_dir(&args)?;
    if option_env!("HY_SERVER_TEST_SCHEMA_CHANGE_THEN_FAIL").is_some() {
        return schema_change_then_fail(&dir);
    }
    let protector = platform_protector();
    if !ServerKeys::exist(&dir) {
        init(&args, &dir, protector.as_ref())?;
    }
    let shutdown: Shutdown = Box::pin(async move {
        let _ = tokio::task::spawn_blocking(move || stop.recv()).await;
    });
    serve_cmd(&args, &dir, protector.as_ref(), shutdown, ready)
}

/// CI only (slice V1-6b): a release that changes the database so that older servers
/// refuse it (a Contract migration), leaves that change in the WAL, then fails to start.
/// The installer must then put back both the previous program and the database copy it
/// made before the update, and keep the failed database with its WAL. Compiled in only
/// when `HY_SERVER_TEST_SCHEMA_CHANGE_THEN_FAIL` is set at build time (the CI test
/// release); the migration is not one of the server's own and never ships.
#[cfg(windows)]
fn schema_change_then_fail(dir: &std::path::Path) -> Result<(), String> {
    use hy_core::migrations::{Compat, MigrateOptions, Migration, migrate_with};
    let mut list = db::MIGRATIONS.to_vec();
    list.push(Migration {
        version: u32::try_from(list.len() + 1).unwrap_or(u32::MAX),
        name: "ci_contract_probe",
        sql: "CREATE TABLE ci_contract_probe (x INTEGER NOT NULL);",
        compat: Compat::Contract,
    });
    let mut conn =
        rusqlite::Connection::open(dir.join("server.db")).map_err(|e| format!("database: {e}"))?;
    let _mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .map_err(|e| format!("database: {e}"))?;
    conn.pragma_update(None, "wal_autocheckpoint", 0)
        .map_err(|e| format!("database: {e}"))?;
    migrate_with(
        &mut conn,
        &list,
        MigrateOptions {
            copy_before_upgrade: false,
        },
    )
    .map_err(|e| format!("database: {e}"))?;
    // Leave the change in the WAL (no checkpoint at close), as a crash would.
    std::mem::forget(conn);
    Err("test release: the database was changed, then the start failed".into())
}

fn serve_cmd(
    args: &Args,
    dir: &std::path::Path,
    protector: &dyn hy_server::keys::KeyProtector,
    shutdown: Shutdown,
    ready: &dyn Fn(),
) -> Result<(), String> {
    if !serve::admin_address_ok(&args.admin) {
        return Err("the admin page is served on a loopback address only".into());
    }
    let mirror = args
        .mirror
        .clone()
        .ok_or("give --mirror FILE on another volume (a second disk or a share)")?;
    hy_server::mirror::check_volume(dir, &mirror).map_err(|e| e.to_string())?;
    let backups = args.backups.clone().unwrap_or_else(|| dir.join("backups"));
    let backup_key = Arc::new(
        BackupKey::load_or_create(dir, protector)
            .map_err(|e| format!("cannot open the backup key: {e}"))?,
    );
    let keys =
        ServerKeys::load(dir, protector).map_err(|e| format!("cannot open the keys: {e}"))?;
    let conn = db::open(&dir.join("server.db")).map_err(|e| format!("database: {e}"))?;
    hy_server::chain::verify_all(&conn, &keys.key_id).map_err(|e| format!("control log: {e}"))?;
    let log_file =
        logging::RotatingFile::open(&dir.join("logs")).map_err(|e| format!("logs: {e}"))?;
    let log_file: &'static logging::RotatingFile = Box::leak(Box::new(log_file));
    logging::install(move || log_file);
    logging::install_panic_hook();
    let config = Config {
        host_name: host_name(args),
        api_port: args.api.port(),
        ..Config::default()
    };
    let server = Arc::new(Server::new(
        conn,
        keys,
        Arc::new(SystemClock::new()),
        config,
    ));
    logging::log(
        "startup",
        &[
            ("fingerprint", V::Prefix(&server.keys.fingerprint)),
            ("key_id", V::Prefix(&server.keys.key_id)),
        ],
    );
    let outcome = server.startup(&mirror)?;
    eprintln!("hy-server: control-log mirror {outcome:?}");
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(async move {
        let api = tokio::net::TcpListener::bind(args.api)
            .await
            .map_err(|e| format!("cannot listen on the API port: {e}"))?;
        let admin = tokio::net::TcpListener::bind(args.admin)
            .await
            .map_err(|e| format!("cannot listen on the admin port: {e}"))?;
        let (tx, rx) = tokio::sync::watch::channel(false);
        let maint = server.clone();
        let mut maint_rx = rx.clone();
        tokio::spawn(async move {
            loop {
                let m = maint.clone();
                let (dir, key) = (backups.clone(), backup_key.clone());
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = m.maintenance();
                    if let Err(e) = backup::run_if_due(&m, &dir, &key) {
                        logging::log("backup_failed", &[("error", V::Name(e.kind()))]);
                    }
                })
                .await;
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(60)) => {}
                    _ = maint_rx.changed() => return,
                }
            }
        });
        let api_task = tokio::spawn(serve::serve_api(
            api,
            server.clone(),
            Limits::default(),
            rx.clone(),
        ));
        let admin_task = tokio::spawn(serve::serve_admin(admin, server.clone(), rx));
        eprintln!("hy-server listening; admin page at http://{}", args.admin);
        ready();
        shutdown.await;
        let _ = tx.send(true);
        let _ = api_task.await;
        let _ = admin_task.await;
        Ok::<(), String>(())
    })
}

fn main() -> ExitCode {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("service") {
        return match winservice::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("hy-server: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hy-server: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_usage_text_gives_the_reset_route_for_lost_or_moved_keys() {
        let u = super::usage();
        assert!(u.contains("permanently unrecoverable"));
        assert!(u.contains("not a way to move or rebuild the server"));
        assert!(u.contains("every PC re-pairs with a new slip"));
    }
}
