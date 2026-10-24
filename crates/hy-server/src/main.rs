//! `hy-server`: the HealthierYou office LAN server (architecture 5).
//!
//! ```text
//! hy-server init            [--data-dir DIR] [--host NAME]   create keys and the database
//! hy-server set-passphrase  [--data-dir DIR]                 read the admin passphrase from stdin
//! hy-server fingerprint     [--data-dir DIR]                 print the fingerprint and six words
//! hy-server serve           [--data-dir DIR] --mirror FILE [--backups DIR] [--host NAME] [--api ADDR] [--admin ADDR]
//! hy-server restore         [--data-dir DIR] --backup FILE    restore a server backup (service stopped)
//! hy-server --version
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

struct Args {
    command: String,
    data_dir: Option<PathBuf>,
    host: Option<String>,
    api: SocketAddr,
    admin: SocketAddr,
    mirror: Option<PathBuf>,
    backups: Option<PathBuf>,
    backup: Option<PathBuf>,
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
    };
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--data-dir" => args.data_dir = Some(PathBuf::from(value)),
            "--host" => args.host = Some(value),
            "--mirror" => args.mirror = Some(PathBuf::from(value)),
            "--backups" => args.backups = Some(PathBuf::from(value)),
            "--backup" => args.backup = Some(PathBuf::from(value)),
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
    let dir = data_dir(&args)?;
    let protector = platform_protector();
    match args.command.as_str() {
        "init" => {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("cannot create the data folder: {e}"))?;
            let year = year_of(chrono::Utc::now());
            let keys = ServerKeys::create(&dir, protector.as_ref(), &host_name(&args), year)
                .map_err(|e| format!("cannot create keys: {e}"))?;
            db::open(&dir.join("server.db"))
                .map_err(|e| format!("cannot create the database: {e}"))?;
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
        "serve" => serve_cmd(&args, &dir, protector.as_ref()),
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

fn serve_cmd(
    args: &Args,
    dir: &std::path::Path,
    protector: &dyn hy_server::keys::KeyProtector,
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
        let _ = tokio::signal::ctrl_c().await;
        let _ = tx.send(true);
        let _ = api_task.await;
        let _ = admin_task.await;
        Ok::<(), String>(())
    })
}

fn main() -> ExitCode {
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
