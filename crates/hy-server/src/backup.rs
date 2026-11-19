//! Encrypted server backups and restore (V1-5 contract 10; architecture 5.1 and 5.3).
//!
//! **Schedule.** While the service runs: daily at 13:00 IST, plus a catch-up whenever
//! the last backup is more than 24 h old (overnight uptime is never assumed). 14 daily
//! and 2 weekly copies are kept, none older than 30 days.
//!
//! **Container** (`.hysbak`). The database is copied with `VACUUM INTO` a temporary file,
//! which is read, encrypted and deleted:
//!
//! ```text
//! "HYSBAK1\0" ‖ u8 version=1 ‖ snapshot_id[16] ‖ i64be created_at ‖ nonce[12]
//!   ‖ u32be len(summary) ‖ summary (JSON: each user's control high-water)
//!   ‖ AES-256-GCM(backup.key, nonce, AAD = everything before, the database)
//! ```
//!
//! `backup.key` is 32 random bytes protected like the other server keys (machine-scope
//! DPAPI on Windows) and never written into a backup, so a backup cannot be restored on
//! another machine (contract 10). **If the key is lost, every existing backup is
//! permanently unrecoverable**. A copy of the keys folder opens only on this same Windows
//! installation (machine-scope DPAPI), so it guards against the files being deleted, not
//! against losing the PC: it is not a portable recovery route. On a replacement server,
//! after an OS reinstall, or whenever the keys cannot be opened, the approved route is the
//! full reset: new keys, every PC re-pairs with a new slip, and the old backups cannot be
//! opened (contract 6.2, owner decision O4). The admin page, the `hy-server` usage text and
//! `docs/Server_Operations.md` say so. The snapshot records its own `snapshot_id`; a restore
//! marks the restored database so the next start puts every user in recovery hold and
//! voids every unused slip (contract 9).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use hy_core::protocol::hex;
use hy_core::secret::Secret;
use rusqlite::params;

use crate::audit::{self, AuditEvent};
use crate::keys::{KeyProtector, ServerKeys, write_private};
use crate::logging::{self, V};
use crate::server::Server;

/// The container's magic bytes.
pub const MAGIC: &[u8; 8] = b"HYSBAK1\0";
/// The backup key's file name, in the keys folder.
pub const KEY_FILE: &str = "backup.key";
/// Daily copies kept.
pub const KEEP_DAILY: usize = 14;
/// Weekly copies kept.
pub const KEEP_WEEKLY: usize = 2;
/// No copy is kept longer.
pub const MAX_AGE_MS: i64 = 30 * 24 * 3600 * 1000;
const DAY_MS: i64 = 24 * 3600 * 1000;

/// Why a backup could not be written or restored.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// File system.
    #[error("backup file: {0}")]
    Io(#[from] io::Error),
    /// Database.
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    /// Not a backup of this server, or damaged.
    #[error("not a backup this server can open")]
    Invalid,
    /// A database log is present without its database and no interrupted restore
    /// explains it; nothing was changed so its committed changes are not lost.
    #[error("server.db-wal is present without server.db; nothing was changed")]
    UnpairedLog,
}

impl BackupError {
    /// A fixed name for the log (never a path or a message).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Db(_) => "database",
            Self::Invalid => "invalid",
            Self::UnpairedLog => "unpaired-log",
        }
    }
}

/// The backup encryption key.
pub struct BackupKey(Secret<[u8; 32]>);

impl std::fmt::Debug for BackupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackupKey([redacted])")
    }
}

impl BackupKey {
    /// Load `keys/backup.key`, creating it on first use.
    ///
    /// # Errors
    /// An I/O error, or a key file that cannot be opened on this machine.
    pub fn load_or_create(data_dir: &Path, protector: &dyn KeyProtector) -> io::Result<Self> {
        let path = ServerKeys::dir(data_dir).join(KEY_FILE);
        if !path.exists() {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).map_err(|_| io::Error::other("random source failed"))?;
            fs::create_dir_all(ServerKeys::dir(data_dir))?;
            write_private(&path, &protector.protect(&key)?)?;
            return Ok(Self(Secret::new(key)));
        }
        let plain = protector.unprotect(&fs::read(&path)?)?;
        let key: [u8; 32] = plain
            .expose()
            .as_slice()
            .try_into()
            .map_err(|_| io::Error::other("backup key length"))?;
        Ok(Self(Secret::new(key)))
    }

    /// A key from raw bytes (tests).
    #[must_use]
    pub fn from_bytes(key: [u8; 32]) -> Self {
        Self(Secret::new(key))
    }

    fn aead(&self) -> Result<LessSafeKey, BackupError> {
        UnboundKey::new(&AES_256_GCM, self.0.expose())
            .map(LessSafeKey::new)
            .map_err(|_| BackupError::Invalid)
    }
}

/// One written backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    /// The file.
    pub path: PathBuf,
    /// The snapshot it holds.
    pub snapshot_id: [u8; 16],
    /// Counted as a weekly copy.
    pub weekly: bool,
}

/// 13:00 IST on the day of `now` (IST), as UTC milliseconds.
fn today_at_13_ist(now: DateTime<Utc>) -> i64 {
    let ist = now.with_timezone(&chrono_tz::Asia::Kolkata);
    chrono_tz::Asia::Kolkata
        .with_ymd_and_hms(ist.year(), ist.month(), ist.day(), 13, 0, 0)
        .single()
        .map_or(i64::MAX, |t| t.timestamp_millis())
}

/// Whether a backup is due: none yet, the last is over 24 h old (catch-up), or today's
/// 13:00 IST has passed since the last.
#[must_use]
pub fn due(now: DateTime<Utc>, last_ms: Option<i64>) -> bool {
    let Some(last) = last_ms else {
        return true;
    };
    let now_ms = now.timestamp_millis();
    let slot = today_at_13_ist(now);
    now_ms - last > DAY_MS || (now_ms >= slot && last < slot)
}

/// When the last backup was written, if any.
///
/// # Errors
/// Any SQLite error.
pub fn last_backup(server: &Server) -> rusqlite::Result<Option<i64>> {
    server
        .db()
        .query_row("SELECT max(created_at) FROM backups", [], |r| r.get(0))
}

fn header(
    snapshot_id: &[u8; 16],
    created_at: i64,
    nonce: &[u8; NONCE_LEN],
    summary: &[u8],
) -> Vec<u8> {
    let mut h = Vec::with_capacity(8 + 1 + 16 + 8 + NONCE_LEN + 4 + summary.len());
    h.extend_from_slice(MAGIC);
    h.push(1);
    h.extend_from_slice(snapshot_id);
    h.extend_from_slice(&created_at.to_be_bytes());
    h.extend_from_slice(nonce);
    h.extend_from_slice(
        &u32::try_from(summary.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    h.extend_from_slice(summary);
    h
}

/// Whether this database came from a backup (it records the snapshot it was taken as).
#[must_use]
pub fn is_restored_copy(server: &Server) -> bool {
    server
        .db()
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM meta WHERE key = 'restored_from')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false)
}

/// Write one encrypted backup into `dir` and apply the retention rules.
///
/// # Errors
/// [`BackupError`]; a failed backup leaves no partial file behind.
pub fn write(server: &Server, dir: &Path, key: &BackupKey) -> Result<BackupInfo, BackupError> {
    fs::create_dir_all(dir)?;
    let now = server.clock.now();
    let now_ms = now.timestamp_millis();
    let mut snapshot_id = [0u8; 16];
    getrandom::fill(&mut snapshot_id).map_err(|_| io::Error::other("random source failed"))?;
    let stamp = now.format("%Y%m%d-%H%M%S").to_string();
    let name = format!("hy-server-{stamp}-{}.hysbak", hex(&snapshot_id[..4]));
    let tmp_db = dir.join(format!(".{name}.db.tmp"));
    let summary = {
        let conn = server.db();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('snapshot_id', ?1), ('snapshot_at', ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![hex(&snapshot_id), now_ms.to_string()],
        )?;
        let users: Vec<(Vec<u8>, i64)> = conn
            .prepare(
                "SELECT u.user_id, coalesce((SELECT max(seq) FROM control_log c WHERE c.user_id = u.user_id), 0)
                 FROM users u ORDER BY u.user_id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let _ = fs::remove_file(&tmp_db);
        conn.execute("VACUUM INTO ?1", [tmp_db.to_string_lossy()])?;
        serde_json::json!({
            "v": 1,
            "high_water": users
                .iter()
                .map(|(u, s)| serde_json::json!({"user": hex(u), "seq": s}))
                .collect::<Vec<_>>(),
        })
        .to_string()
    };
    let plain = fs::read(&tmp_db);
    let _ = fs::remove_file(&tmp_db);
    let mut data = plain?;
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| io::Error::other("random source failed"))?;
    let head = header(&snapshot_id, now_ms, &nonce, summary.as_bytes());
    key.aead()?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(head.as_slice()),
            &mut data,
        )
        .map_err(|_| BackupError::Invalid)?;
    let path = dir.join(&name);
    let part = dir.join(format!(".{name}.part"));
    {
        let mut f = fs::File::create(&part)?;
        io::Write::write_all(&mut f, &head)?;
        io::Write::write_all(&mut f, &data)?;
        f.sync_all()?;
    }
    fs::rename(&part, &path)?;
    let conn = server.db();
    let last_weekly: Option<i64> = conn.query_row(
        "SELECT max(created_at) FROM backups WHERE weekly = 1",
        [],
        |r| r.get(0),
    )?;
    let weekly = last_weekly.is_none_or(|w| now_ms - w >= 7 * DAY_MS);
    conn.execute(
        "INSERT INTO backups (file, snapshot_id, created_at, weekly) VALUES (?1, ?2, ?3, ?4)",
        params![name, snapshot_id.as_slice(), now_ms, weekly],
    )?;
    audit::record(&conn, now_ms, AuditEvent::BackupWritten, None, None, None)?;
    prune(&conn, dir, now_ms)?;
    drop(conn);
    logging::log("backup_written", &[("weekly", V::N(u64::from(weekly)))]);
    Ok(BackupInfo {
        path,
        snapshot_id,
        weekly,
    })
}

/// Keep the newest 14 daily and 2 weekly copies, none older than 30 days.
fn prune(conn: &rusqlite::Connection, dir: &Path, now_ms: i64) -> Result<(), BackupError> {
    let rows: Vec<(String, i64, bool)> = conn
        .prepare("SELECT file, created_at, weekly FROM backups ORDER BY created_at DESC")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let (mut daily, mut weekly) = (0usize, 0usize);
    for (file, at, is_weekly) in rows {
        let keep = now_ms - at <= MAX_AGE_MS
            && if is_weekly {
                weekly += 1;
                weekly <= KEEP_WEEKLY
            } else {
                daily += 1;
                daily <= KEEP_DAILY
            };
        if !keep {
            let _ = fs::remove_file(dir.join(&file));
            conn.execute("DELETE FROM backups WHERE file = ?1", [&file])?;
        }
    }
    Ok(())
}

/// Write a backup if one is due (the service calls this every minute).
///
/// # Errors
/// [`BackupError`].
pub fn run_if_due(
    server: &Server,
    dir: &Path,
    key: &BackupKey,
) -> Result<Option<BackupInfo>, BackupError> {
    if due(server.clock.now(), last_backup(server)?) {
        write(server, dir, key).map(Some)
    } else {
        Ok(None)
    }
}

/// Decrypt a backup file, returning its snapshot id and the database bytes.
///
/// # Errors
/// [`BackupError::Invalid`] for a damaged file, another key or another format.
pub fn open(file: &Path, key: &BackupKey) -> Result<([u8; 16], Vec<u8>), BackupError> {
    let bytes = fs::read(file)?;
    let fixed = 8 + 1 + 16 + 8 + NONCE_LEN + 4;
    if bytes.len() < fixed || &bytes[..8] != MAGIC || bytes[8] != 1 {
        return Err(BackupError::Invalid);
    }
    let snapshot_id: [u8; 16] = bytes[9..25].try_into().map_err(|_| BackupError::Invalid)?;
    let nonce: [u8; NONCE_LEN] = bytes[33..33 + NONCE_LEN]
        .try_into()
        .map_err(|_| BackupError::Invalid)?;
    let len_at = 33 + NONCE_LEN;
    let len = u32::from_be_bytes(
        bytes[len_at..len_at + 4]
            .try_into()
            .map_err(|_| BackupError::Invalid)?,
    ) as usize;
    let head_len = fixed.checked_add(len).ok_or(BackupError::Invalid)?;
    if bytes.len() < head_len {
        return Err(BackupError::Invalid);
    }
    let (head, body) = bytes.split_at(head_len);
    let mut data = body.to_vec();
    let plain = key
        .aead()?
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(head),
            &mut data,
        )
        .map_err(|_| BackupError::Invalid)?;
    Ok((snapshot_id, plain.to_vec()))
}

/// Restore `file` as the server database in `data_dir` (the service must be stopped).
/// The current database is kept beside it as `server.db.before-restore-<ms>`. The
/// restored database is marked so the next start puts every user in recovery hold and
/// voids every unused slip; mirror entries newer than the snapshot are then replayed.
///
/// # Errors
/// [`BackupError`]; on error the current database is untouched.
pub fn restore(
    data_dir: &Path,
    file: &Path,
    key: &BackupKey,
    now_ms: i64,
) -> Result<[u8; 16], BackupError> {
    let (snapshot_id, plain) = open(file, key)?;
    recover_interrupted_restore(data_dir)?;
    let target = data_dir.join("server.db");
    let staging = data_dir.join("server.db.restoring");
    let _ = fs::remove_file(&staging);
    fs::write(&staging, &plain)?;
    {
        let conn = rusqlite::Connection::open(&staging)?;
        let id: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'snapshot_id'",
                [],
                |r| r.get(0),
            )
            .ok();
        if id.as_deref() != Some(hex(&snapshot_id).as_str()) {
            drop(conn);
            let _ = fs::remove_file(&staging);
            return Err(BackupError::Invalid);
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('restore_pending', ?1), ('restored_from', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [now_ms.to_string()],
        )?;
    }
    // The current database is kept whole: after a crash or power loss its latest
    // committed changes are only in `server.db-wal`, so the log moves with it under the
    // kept copy's name (SQLite finds a database's log by name; the `-shm` index is
    // rebuilt). The two moves are recorded first in `server.db.restore-state`, so a
    // restore interrupted between them is undone by the next attempt instead of leaving
    // the log to be deleted as a stale sidecar.
    let wal = data_dir.join("server.db-wal");
    let state = data_dir.join(RESTORE_STATE);
    let kept = data_dir.join(format!("server.db.before-restore-{now_ms}"));
    let kept_wal = with_suffix(&kept, "-wal");
    let set_aside = target.exists();
    if !set_aside && wal.exists() {
        let _ = fs::remove_file(&staging);
        return Err(BackupError::UnpairedLog);
    }
    if set_aside {
        write_synced(
            &state,
            kept.file_name().unwrap_or_default().as_encoded_bytes(),
        )?;
        fault("before-db-move")?;
        fs::rename(&target, &kept)?;
        fault("between-moves")?;
        if wal.exists()
            && let Err(e) = fs::rename(&wal, &kept_wal)
        {
            // If the undo fails too, its record stays for the next attempt.
            let _ = undo_set_aside(data_dir, &kept);
            let _ = fs::remove_file(&staging);
            return Err(e.into());
        }
        fault("before-install")?;
    }
    let _ = fs::remove_file(data_dir.join("server.db-shm"));
    let installed = fault("install").and_then(|()| fs::rename(&staging, &target));
    if let Err(e) = installed {
        if set_aside {
            let _ = undo_set_aside(data_dir, &kept);
        }
        let _ = fs::remove_file(&staging);
        return Err(e.into());
    }
    let _ = fs::remove_file(&state);
    Ok(snapshot_id)
}

/// Records which kept copy a restore in progress is moving the live database to.
const RESTORE_STATE: &str = "server.db.restore-state";

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::File::create(path)?;
    io::Write::write_all(&mut f, bytes)?;
    f.sync_all()
}

/// Puts a set-aside database and its log back at the live names, then clears the record.
///
/// The log goes back first and the database last, so the live database reappears only once
/// its log is already beside it: a live `server.db` with a record therefore always means the
/// undo finished (or nothing was moved, or the backup was installed), never an undo half
/// done. Any error stops here with the record kept, so the next attempt resumes the undo.
fn undo_set_aside(data_dir: &Path, kept: &Path) -> io::Result<()> {
    let target = data_dir.join("server.db");
    let wal = data_dir.join("server.db-wal");
    let kept_wal = with_suffix(kept, "-wal");
    if kept_wal.exists() {
        if wal.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "server.db-wal is present while its set-aside log is waiting to go back",
            ));
        }
        fault("undo-wal")?;
        fs::rename(&kept_wal, &wal)?;
    }
    fault("between-undo-moves")?;
    if kept.exists() {
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "server.db is present while the set-aside database is waiting to go back",
            ));
        }
        fs::rename(kept, &target)?;
    }
    match fs::remove_file(data_dir.join(RESTORE_STATE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Finishes the record of an earlier restore that stopped part way. If the live
/// database is missing, the undo is resumed: the set-aside log and database go back to the
/// live names (see [`undo_set_aside`] for why the order makes this safe). If the live
/// database is present, that restore either never moved anything, installed its backup or
/// finished its undo, and the record is simply cleared.
fn recover_interrupted_restore(data_dir: &Path) -> io::Result<()> {
    let state = data_dir.join(RESTORE_STATE);
    let name = match fs::read(&state) {
        Ok(n) => n,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let name = String::from_utf8(name)
        .ok()
        .filter(|n| n.starts_with("server.db.before-restore-") && !n.contains(['/', '\\']))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "restore record"))?;
    if data_dir.join("server.db").exists() {
        return fs::remove_file(&state);
    }
    undo_set_aside(data_dir, &data_dir.join(name))?;
    if data_dir.join("server.db").exists() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "an interrupted restore could not be undone",
        ))
    }
}

#[cfg(test)]
thread_local! {
    static FAULT: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Test-only fault injection: a stop at `point` as a crash would (no clean-up).
#[allow(clippy::unnecessary_wraps)]
fn fault(point: &'static str) -> io::Result<()> {
    #[cfg(test)]
    if FAULT.with(|f| f.borrow().contains(&point)) {
        return Err(io::Error::other(point));
    }
    let _ = point;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// A backup file of a minimal database recording `snapshot_id`.
    fn synthetic_backup(dir: &Path, key: &BackupKey, snapshot_id: [u8; 16]) -> PathBuf {
        let db = dir.join("snapshot.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
                .unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('snapshot_id', ?1)",
                [hex(&snapshot_id)],
            )
            .unwrap();
        }
        let mut data = fs::read(&db).unwrap();
        let nonce = [9u8; NONCE_LEN];
        let head = header(&snapshot_id, 1, &nonce, b"{}");
        key.aead()
            .unwrap()
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(head.as_slice()),
                &mut data,
            )
            .unwrap();
        let file = dir.join("synthetic.hysbak");
        fs::write(&file, [head, data].concat()).unwrap();
        file
    }

    #[test]
    fn restore_keeps_the_current_database_whole_with_its_write_ahead_log() {
        // The service stopped uncleanly (a crash or power loss), so committed changes are
        // still only in `server.db-wal`. The copy kept "next to it" must hold them.
        let live = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(live.path().join("server.db")).unwrap();
        let _mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER NOT NULL); INSERT INTO t (x) VALUES (42);")
            .unwrap();
        // Copy the files as a crash leaves them (the connection is still open).
        for name in ["server.db", "server.db-wal"] {
            fs::copy(live.path().join(name), data.path().join(name)).unwrap();
        }
        drop(conn);
        assert!(data.path().join("server.db-wal").exists());

        let key = BackupKey::from_bytes([3u8; 32]);
        let src = tempfile::tempdir().unwrap();
        let file = synthetic_backup(src.path(), &key, [5u8; 16]);
        restore(data.path(), &file, &key, 1234).unwrap();

        let kept = data.path().join("server.db.before-restore-1234");
        let conn = rusqlite::Connection::open(&kept).unwrap();
        let x: i64 = conn
            .query_row("SELECT x FROM t", [], |r| r.get(0))
            .expect("the kept database holds what was committed before the restore");
        assert_eq!(x, 42);
        drop(conn);
        // The restored database starts with no write-ahead log of another database.
        assert!(!data.path().join("server.db-wal").exists());
        let conn = rusqlite::Connection::open(data.path().join("server.db")).unwrap();
        let id: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'snapshot_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id, hex(&[5u8; 16]));
    }

    /// A data folder as an unclean stop leaves it: `server.db` with a committed row
    /// (x = 42) that is only in `server.db-wal`.
    fn crashed_data_dir() -> tempfile::TempDir {
        let live = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(live.path().join("server.db")).unwrap();
        let _mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch("CREATE TABLE t (x INTEGER NOT NULL); INSERT INTO t (x) VALUES (42);")
            .unwrap();
        for name in ["server.db", "server.db-wal"] {
            fs::copy(live.path().join(name), data.path().join(name)).unwrap();
        }
        drop(conn);
        data
    }

    fn row_in(db: &Path) -> i64 {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.query_row("SELECT x FROM t", [], |r| r.get(0))
            .expect("the committed row is still there")
    }

    /// Reads the row from a copy of `db` and its log, leaving the files themselves as they
    /// are (opening them would fold the log into the database).
    fn row_in_copy(db: &Path) -> i64 {
        let tmp = tempfile::tempdir().unwrap();
        let copy = tmp.path().join("copy.db");
        fs::copy(db, &copy).unwrap();
        let wal = with_suffix(db, "-wal");
        if wal.exists() {
            fs::copy(&wal, with_suffix(&copy, "-wal")).unwrap();
        }
        row_in(&copy)
    }

    fn snapshot_in(db: &Path) -> String {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'snapshot_id'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn with_fault<T>(point: &'static str, f: impl FnOnce() -> T) -> T {
        with_faults(&[point], f)
    }

    fn with_faults<T>(points: &[&'static str], f: impl FnOnce() -> T) -> T {
        FAULT.with(|c| *c.borrow_mut() = points.to_vec());
        let out = f();
        FAULT.with(|c| c.borrow_mut().clear());
        out
    }

    #[test]
    fn an_undo_interrupted_between_its_moves_is_resumed() {
        let data = crashed_data_dir();
        let key = BackupKey::from_bytes([3u8; 32]);
        let src = tempfile::tempdir().unwrap();
        let file = synthetic_backup(src.path(), &key, [5u8; 16]);
        // The install fails, and the undo stops (as a power cut would) after putting the
        // log back but before the database.
        let r = with_faults(&["install", "between-undo-moves"], || {
            restore(data.path(), &file, &key, 1000)
        });
        assert!(r.is_err());
        assert!(!data.path().join("server.db").exists());
        assert!(data.path().join("server.db-wal").exists());
        assert!(data.path().join(RESTORE_STATE).exists(), "the record stays");
        // The next attempt resumes the undo and restores, keeping the committed row.
        restore(data.path(), &file, &key, 2000).unwrap();
        assert_eq!(
            row_in(&data.path().join("server.db.before-restore-2000")),
            42
        );
        assert_eq!(snapshot_in(&data.path().join("server.db")), hex(&[5u8; 16]));
        assert!(!data.path().join(RESTORE_STATE).exists());
    }

    #[test]
    fn a_failed_log_move_during_undo_keeps_the_pair_and_the_record() {
        let data = crashed_data_dir();
        let key = BackupKey::from_bytes([3u8; 32]);
        let src = tempfile::tempdir().unwrap();
        let file = synthetic_backup(src.path(), &key, [5u8; 16]);
        let r = with_faults(&["install", "undo-wal"], || {
            restore(data.path(), &file, &key, 1000)
        });
        assert!(r.is_err());
        // The database was not put back without its log: both wait under the kept name.
        assert!(!data.path().join("server.db").exists());
        assert!(!data.path().join("server.db-wal").exists());
        let kept = data.path().join("server.db.before-restore-1000");
        assert!(with_suffix(&kept, "-wal").exists());
        assert_eq!(row_in_copy(&kept), 42);
        assert!(data.path().join(RESTORE_STATE).exists(), "the record stays");
        // A later attempt that also fails to move the log back changes nothing.
        let again = with_fault("undo-wal", || restore(data.path(), &file, &key, 1500));
        assert!(again.is_err());
        assert!(data.path().join(RESTORE_STATE).exists());
        assert!(!data.path().join("server.db").exists());
        // Once the move works, the undo finishes and the restore goes ahead.
        restore(data.path(), &file, &key, 2000).unwrap();
        assert_eq!(
            row_in(&data.path().join("server.db.before-restore-2000")),
            42
        );
        assert_eq!(snapshot_in(&data.path().join("server.db")), hex(&[5u8; 16]));
    }

    #[test]
    fn a_restore_interrupted_part_way_is_undone_by_the_next_attempt() {
        for point in ["before-db-move", "between-moves", "before-install"] {
            let data = crashed_data_dir();
            let key = BackupKey::from_bytes([3u8; 32]);
            let src = tempfile::tempdir().unwrap();
            let file = synthetic_backup(src.path(), &key, [5u8; 16]);
            // The first attempt stops as a crash would, with no clean-up.
            assert!(with_fault(point, || restore(data.path(), &file, &key, 1000)).is_err());
            // The next attempt finds the half-done move and completes the restore.
            restore(data.path(), &file, &key, 2000).unwrap();
            let kept = data.path().join("server.db.before-restore-2000");
            assert_eq!(row_in(&kept), 42, "committed changes kept ({point})");
            assert_eq!(snapshot_in(&data.path().join("server.db")), hex(&[5u8; 16]));
            assert!(!data.path().join("server.db-wal").exists());
            assert!(!data.path().join(RESTORE_STATE).exists());
        }
    }

    #[test]
    fn a_failed_install_puts_the_live_database_back_with_its_log() {
        let data = crashed_data_dir();
        let key = BackupKey::from_bytes([3u8; 32]);
        let src = tempfile::tempdir().unwrap();
        let file = synthetic_backup(src.path(), &key, [5u8; 16]);
        assert!(with_fault("install", || restore(data.path(), &file, &key, 1000)).is_err());
        assert_eq!(row_in(&data.path().join("server.db")), 42);
        assert!(!data.path().join("server.db.before-restore-1000").exists());
        assert!(!data.path().join("server.db.restoring").exists());
        assert!(!data.path().join(RESTORE_STATE).exists());
    }

    #[test]
    fn a_log_without_its_database_is_never_deleted() {
        let data = crashed_data_dir();
        fs::remove_file(data.path().join("server.db")).unwrap();
        let before = fs::read(data.path().join("server.db-wal")).unwrap();
        let key = BackupKey::from_bytes([3u8; 32]);
        let src = tempfile::tempdir().unwrap();
        let file = synthetic_backup(src.path(), &key, [5u8; 16]);
        assert!(matches!(
            restore(data.path(), &file, &key, 1000),
            Err(BackupError::UnpairedLog)
        ));
        assert_eq!(fs::read(data.path().join("server.db-wal")).unwrap(), before);
        assert!(!data.path().join("server.db").exists());
    }

    #[test]
    fn backups_are_due_daily_at_13_ist_and_as_a_catch_up() {
        // 13:00 IST is 07:30 UTC.
        let last = at("2026-10-23T07:31:00Z").timestamp_millis();
        assert!(due(at("2026-10-23T08:00:00Z"), None));
        assert!(!due(at("2026-10-23T20:00:00Z"), Some(last)));
        assert!(
            due(at("2026-10-24T07:30:00Z"), Some(last)),
            "next 13:00 IST"
        );
        let morning = at("2026-10-23T03:00:00Z").timestamp_millis();
        assert!(due(at("2026-10-23T07:30:00Z"), Some(morning)));
        assert!(!due(at("2026-10-23T07:00:00Z"), Some(morning)));
        // Catch-up: the PC was off at 13:00; more than 24 h since the last.
        let old = at("2026-10-21T09:00:00Z").timestamp_millis();
        assert!(due(at("2026-10-22T10:00:00Z"), Some(old)));
    }
}
