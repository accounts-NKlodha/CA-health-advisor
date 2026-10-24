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
}

impl BackupError {
    /// A fixed name for the log (never a path or a message).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Db(_) => "database",
            Self::Invalid => "invalid",
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
    if target.exists() {
        fs::rename(
            &target,
            data_dir.join(format!("server.db.before-restore-{now_ms}")),
        )?;
    }
    for ext in ["server.db-wal", "server.db-shm"] {
        let _ = fs::remove_file(data_dir.join(ext));
    }
    fs::rename(&staging, &target)?;
    Ok(snapshot_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
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
