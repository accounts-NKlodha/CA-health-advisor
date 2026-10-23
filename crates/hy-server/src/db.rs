//! The server database, `server.db` (V1-5 contract 10).
//!
//! SQLite in WAL mode with `synchronous=FULL`, `secure_delete=ON` (deleted content is
//! overwritten) and foreign keys on. Its own forward-only migrations run through the
//! shared runner in `hy_core::migrations`.

use std::path::Path;
use std::time::Duration;

use hy_core::migrations::{Compat, MigrateOptions, Migration, migrate_with};
use rusqlite::Connection;

/// The server's migrations, in order.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "server",
    sql: include_str!("../migrations/0001_server.sql"),
    compat: Compat::Contract,
}];

fn init(mut conn: Connection) -> hy_core::Result<Connection> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    migrate_with(&mut conn, MIGRATIONS, MigrateOptions::default())?;
    Ok(conn)
}

/// Open (creating if needed) and migrate the database at `path`.
///
/// # Errors
/// Any SQLite or migration error.
pub fn open(path: &Path) -> hy_core::Result<Connection> {
    init(Connection::open(path)?)
}

/// An in-memory database, for tests.
///
/// # Errors
/// Any SQLite or migration error.
pub fn open_in_memory() -> hy_core::Result<Connection> {
    init(Connection::open_in_memory()?)
}

/// A 16-byte id column as an array.
///
/// # Errors
/// A conversion error if the column is not 16 bytes.
pub fn id16(v: Vec<u8>) -> rusqlite::Result<[u8; 16]> {
    v.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            "expected 16 bytes".into(),
        )
    })
}

/// A 32-byte hash column as an array.
///
/// # Errors
/// A conversion error if the column is not 32 bytes.
pub fn h32(v: Vec<u8>) -> rusqlite::Result<[u8; 32]> {
    v.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            "expected 32 bytes".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_with_the_contract_pragmas() {
        let conn = open_in_memory().unwrap();
        let fk: bool = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        let sd: i64 = conn
            .pragma_query_value(None, "secure_delete", |r| r.get(0))
            .unwrap();
        assert!(fk);
        assert_eq!(sd, 1);
        let dir = tempfile::tempdir().unwrap();
        let file = open(&dir.path().join("server.db")).unwrap();
        let mode: String = file
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        let sync: i64 = file
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(sync, 2, "FULL");
    }

    #[test]
    fn the_control_log_and_slips_resist_rewrites() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO users (user_id, alias, created_at) VALUES (zeroblob(16), 'A', 0);
             INSERT INTO control_log VALUES (zeroblob(16), 1, 0, 3, x'00', zeroblob(32), zeroblob(32));
             INSERT INTO slips (slip_hash, user_id, created_at, expires_at, used_at) VALUES (zeroblob(32), zeroblob(16), 0, 10, 5);",
        )
        .unwrap();
        assert!(
            conn.execute("UPDATE control_log SET at_ms = 1", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM control_log", []).is_err());
        assert!(conn.execute("UPDATE slips SET used_at = NULL", []).is_err());
        assert!(conn.execute("UPDATE slips SET used_at = 6", []).is_err());
    }
}
