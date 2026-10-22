//! Versioned, forward-only schema migrations (architecture 8, "Migrations").
//!
//! - Each migration is an embedded SQL file with a version (1, 2, 3, ...) and runs in its
//!   own `BEGIN IMMEDIATE` transaction together with its bookkeeping row and
//!   `PRAGMA user_version`. A failing migration is rolled back completely; the ones before
//!   it stay applied, and the next start retries from there.
//! - Running the migrations again is a no-op (idempotent).
//! - Each applied migration's checksum is recorded. If a shipped migration is edited after
//!   release, opening a database that already applied it fails with
//!   [`Error::MigrationChecksum`] instead of silently diverging.
//! - Migrations follow "expand, then contract": an [`Compat::Expand`] step only adds, so
//!   the previous release can still open the database; a [`Compat::Contract`] step raises
//!   the lowest reader version. A build that finds a newer schema opens it if every
//!   contract step is one it knows, and otherwise refuses with [`Error::SchemaTooNew`],
//!   which lets the rollback path (V1-6) restore the pre-migration copy.
//! - Before upgrading a database that already has data, the store writes a consistent
//!   copy next to it with `VACUUM INTO` (`hy.db.pre-v<from>.bak`).

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::error::{Error, Result};

/// Whether older builds can still use the schema after this migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    /// Only adds tables, columns or indexes: the previous release can still open the database.
    Expand,
    /// Removes or changes something: readers older than this version must refuse the database.
    Contract,
}

/// One embedded migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Version reached after this migration (1-based, contiguous).
    pub version: u32,
    /// Short name.
    pub name: &'static str,
    /// SQL text.
    pub sql: &'static str,
    /// Compatibility class.
    pub compat: Compat,
}

impl Migration {
    /// Stable checksum of the SQL text (FNV-1a, 64 bit, line endings normalised so a
    /// Windows checkout with CRLF gives the same value).
    #[must_use]
    pub fn checksum(&self) -> String {
        checksum(self.sql)
    }
}

/// The migrations shipped in this build, in order.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../migrations/0001_initial.sql"),
        compat: Compat::Contract,
    },
    Migration {
        version: 2,
        name: "coach",
        sql: include_str!("../migrations/0002_coach.sql"),
        compat: Compat::Expand,
    },
    Migration {
        version: 3,
        name: "movement_routine",
        sql: include_str!("../migrations/0003_movement_routine.sql"),
        compat: Compat::Expand,
    },
    Migration {
        version: 4,
        name: "entry_corrections",
        sql: include_str!("../migrations/0004_entry_corrections.sql"),
        compat: Compat::Expand,
    },
];

/// The schema version this build writes.
#[must_use]
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

fn checksum(sql: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut bytes = sql.bytes().peekable();
    while let Some(b) = bytes.next() {
        if b == b'\r' && bytes.peek() == Some(&b'\n') {
            continue;
        }
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// Options for [`migrate_with`].
#[derive(Debug, Clone, Copy)]
pub struct MigrateOptions {
    /// Write `<db>.pre-v<from>.bak` before upgrading a file database that is already at
    /// version 1 or later.
    pub copy_before_upgrade: bool,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        Self {
            copy_before_upgrade: true,
        }
    }
}

/// What [`migrate_with`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Version found when opening.
    pub from: u32,
    /// Version after migrating.
    pub to: u32,
    /// Versions applied by this call, in order.
    pub applied: Vec<u32>,
    /// The pre-migration copy, if one was written.
    pub pre_migration_copy: Option<PathBuf>,
    /// True if the database is newer than this build but still compatible with it.
    pub opened_newer_compatible: bool,
}

/// Version and reader requirement of a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaStatus {
    /// `PRAGMA user_version`.
    pub version: u32,
    /// Lowest schema version a build must know to open the database.
    pub required_reader: u32,
}

const BOOKKEEPING: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY CHECK (version >= 1),
    name       TEXT NOT NULL,
    checksum   TEXT NOT NULL,
    contract   INTEGER NOT NULL CHECK (contract IN (0, 1)),
    applied_at INTEGER NOT NULL
) STRICT";

/// Bring the database to [`latest_version`] with the shipped [`MIGRATIONS`].
///
/// # Errors
/// See [`migrate_with`].
pub fn migrate(conn: &mut Connection) -> Result<MigrationReport> {
    migrate_with(conn, MIGRATIONS, MigrateOptions::default())
}

/// Read the schema version and reader requirement without changing anything.
///
/// # Errors
/// [`Error::Sqlite`] if the database cannot be read.
pub fn schema_status(conn: &Connection) -> Result<SchemaStatus> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let has_table: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
        [],
        |r| r.get(0),
    )?;
    let required_reader = if has_table {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations WHERE contract = 1",
            [],
            |r| r.get(0),
        )?
    } else {
        0
    };
    Ok(SchemaStatus {
        version,
        required_reader,
    })
}

fn validate_list(migrations: &[Migration]) -> Result<()> {
    for (i, m) in migrations.iter().enumerate() {
        let expected = u32::try_from(i + 1).unwrap_or(u32::MAX);
        if m.version != expected {
            return Err(Error::BadMigrationList(format!(
                "position {i} has version {}, expected {expected}",
                m.version
            )));
        }
        if m.name.is_empty() || m.sql.trim().is_empty() {
            return Err(Error::BadMigrationList(format!(
                "version {} has an empty name or body",
                m.version
            )));
        }
    }
    if migrations
        .first()
        .is_some_and(|m| m.compat != Compat::Contract)
    {
        return Err(Error::BadMigrationList(
            "the first migration creates the schema and must be Contract".into(),
        ));
    }
    Ok(())
}

/// Bring the database to the last version in `migrations`.
///
/// # Errors
/// - [`Error::BadMigrationList`] if versions are not 1, 2, 3, ... (a build defect);
/// - [`Error::SchemaTooNew`] if the database needs a newer reader than this list;
/// - [`Error::MigrationChecksum`] if an applied migration's text changed;
/// - [`Error::MigrationFailed`] if a migration failed (it was rolled back);
/// - [`Error::Invalid`] if the bookkeeping disagrees with `user_version`;
/// - [`Error::Io`] or [`Error::Sqlite`] if the pre-migration copy fails.
pub fn migrate_with(
    conn: &mut Connection,
    migrations: &[Migration],
    options: MigrateOptions,
) -> Result<MigrationReport> {
    validate_list(migrations)?;
    conn.execute_batch(BOOKKEEPING)?;

    let status = schema_status(conn)?;
    let recorded_max: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?;
    if recorded_max != status.version {
        return Err(Error::invalid(format!(
            "schema bookkeeping says v{recorded_max} but user_version is v{}",
            status.version
        )));
    }

    let supported = migrations.last().map_or(0, |m| m.version);
    for m in migrations
        .iter()
        .take_while(|m| m.version <= status.version)
    {
        let stored: Option<String> = conn
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = ?1",
                [m.version],
                |r| r.get(0),
            )
            .optional()?;
        if stored.as_deref() != Some(m.checksum().as_str()) {
            return Err(Error::MigrationChecksum { version: m.version });
        }
    }

    let mut report = MigrationReport {
        from: status.version,
        to: status.version,
        applied: Vec::new(),
        pre_migration_copy: None,
        opened_newer_compatible: false,
    };

    if status.version > supported {
        if status.required_reader > supported {
            return Err(Error::SchemaTooNew {
                found: status.version,
                required_reader: status.required_reader,
                supported,
            });
        }
        report.opened_newer_compatible = true;
        return Ok(report);
    }

    let pending: Vec<&Migration> = migrations
        .iter()
        .filter(|m| m.version > status.version)
        .collect();
    if pending.is_empty() {
        return Ok(report);
    }

    if options.copy_before_upgrade && status.version > 0 {
        report.pre_migration_copy = copy_database(conn, status.version)?;
    }

    for m in pending {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(m.sql)
            .map_err(|source| Error::MigrationFailed {
                version: m.version,
                name: m.name,
                source,
            })?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, checksum, contract, applied_at)
             VALUES (?1, ?2, ?3, ?4, CAST(unixepoch('subsec') * 1000 AS INTEGER))",
            params![
                m.version,
                m.name,
                m.checksum(),
                i64::from(m.compat == Compat::Contract)
            ],
        )?;
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
        report.applied.push(m.version);
        report.to = m.version;
    }
    Ok(report)
}

/// Write a consistent copy of a file database before it is upgraded.
fn copy_database(conn: &Connection, from: u32) -> Result<Option<PathBuf>> {
    let Some(path) = conn.path().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let dest = PathBuf::from(format!("{path}.pre-v{from}.bak"));
    if dest.exists() {
        // Left by an earlier attempt that failed before finishing; same source version.
        std::fs::remove_file(&dest)?;
    }
    conn.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned checksum of the v1 migration. v1 is frozen from its first release (the merge
    /// of slice V1-0); after that, if this test fails, a released migration was edited:
    /// add a new migration instead.
    const V1_CHECKSUM: &str = "94477528e439b81e";
    const V2_CHECKSUM: &str = "6d0b8382cc44f5c6";
    /// v3 (slice V1-3) is frozen once released.
    const V3_CHECKSUM: &str = "bdee60776c4c2f38";
    /// v4 (the V1-3 water panel and phone note) is frozen once released.
    const V4_CHECKSUM: &str = "902adf35de7113c1";

    fn mem() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    fn schema_dump(conn: &Connection) -> Vec<(String, String)> {
        let mut stmt = conn
            .prepare("SELECT name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |r| r.get(0),
        )
        .unwrap()
    }

    const BASE: Migration = Migration {
        version: 1,
        name: "base",
        sql: "CREATE TABLE a (id INTEGER PRIMARY KEY, v TEXT) STRICT;",
        compat: Compat::Contract,
    };
    const ADD_COLUMN: Migration = Migration {
        version: 2,
        name: "add column",
        sql: "ALTER TABLE a ADD COLUMN w INTEGER;",
        compat: Compat::Expand,
    };
    const ADD_TABLE: Migration = Migration {
        version: 3,
        name: "add table",
        sql: "CREATE TABLE b (id INTEGER PRIMARY KEY) STRICT;",
        compat: Compat::Expand,
    };
    const BROKEN: Migration = Migration {
        version: 3,
        name: "broken",
        sql: "CREATE TABLE half_done (id INTEGER); THIS IS NOT SQL;",
        compat: Compat::Expand,
    };

    #[test]
    fn shipped_list_is_well_formed() {
        validate_list(MIGRATIONS).unwrap();
        assert_eq!(latest_version(), MIGRATIONS.len() as u32);
        assert_eq!(MIGRATIONS[0].compat, Compat::Contract);
    }

    #[test]
    fn released_migrations_are_frozen() {
        assert_eq!(MIGRATIONS[0].checksum(), V1_CHECKSUM);
        assert_eq!(MIGRATIONS[1].checksum(), V2_CHECKSUM);
        assert_eq!(MIGRATIONS[1].compat, Compat::Expand);
        assert_eq!(MIGRATIONS[2].checksum(), V3_CHECKSUM);
        assert_eq!(MIGRATIONS[2].compat, Compat::Expand);
        assert_eq!(MIGRATIONS[3].checksum(), V4_CHECKSUM);
        assert_eq!(MIGRATIONS[3].compat, Compat::Expand);
    }

    #[test]
    fn checksum_ignores_line_endings_only() {
        assert_eq!(checksum("a;\nb;\n"), checksum("a;\r\nb;\r\n"));
        assert_ne!(checksum("a;\nb;\n"), checksum("a;\nc;\n"));
        assert_ne!(checksum("a;\rb;"), checksum("a;b;"));
    }

    #[test]
    fn migrates_an_empty_database_to_latest() {
        let mut conn = mem();
        let report = migrate(&mut conn).unwrap();
        assert_eq!(report.from, 0);
        assert_eq!(report.to, latest_version());
        assert_eq!(report.applied, (1..=latest_version()).collect::<Vec<_>>());
        assert_eq!(report.pre_migration_copy, None);
        let status = schema_status(&conn).unwrap();
        assert_eq!(status.version, latest_version());
        assert_eq!(status.required_reader, 1);
        for t in [
            "store_meta",
            "events",
            "daily_summary",
            "monthly_summary",
            "imported_summary",
            "goals_history",
            "settings",
            "pet_profile",
            "sharing_policy",
            "policy_ops",
            "tombstones",
            "consent_log",
            "outbox",
            "schema_migrations",
            "coach_state",
            "activity_day",
        ] {
            assert!(table_exists(&conn, t), "missing table {t}");
        }
        // Pairing material is deliberately absent until V1-5.
        assert!(!table_exists(&conn, "server_link"));
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = mem();
        migrate(&mut conn).unwrap();
        let before = schema_dump(&conn);
        let report = migrate(&mut conn).unwrap();
        assert!(report.applied.is_empty());
        assert_eq!(report.from, report.to);
        assert_eq!(schema_dump(&conn), before);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, i64::from(latest_version()));
    }

    #[test]
    fn applies_only_pending_steps_in_order() {
        let mut conn = mem();
        let r = migrate_with(&mut conn, &[BASE], MigrateOptions::default()).unwrap();
        assert_eq!(r.applied, vec![1]);
        conn.execute("INSERT INTO a (v) VALUES ('kept')", [])
            .unwrap();
        let r = migrate_with(
            &mut conn,
            &[BASE, ADD_COLUMN, ADD_TABLE],
            MigrateOptions::default(),
        )
        .unwrap();
        assert_eq!((r.from, r.to), (1, 3));
        assert_eq!(r.applied, vec![2, 3]);
        assert!(table_exists(&conn, "b"));
        let v: String = conn.query_row("SELECT v FROM a", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "kept");
        assert_eq!(schema_status(&conn).unwrap().required_reader, 1);
    }

    #[test]
    fn failed_migration_rolls_back_and_can_be_retried() {
        let mut conn = mem();
        let err = migrate_with(
            &mut conn,
            &[BASE, ADD_COLUMN, BROKEN],
            MigrateOptions::default(),
        )
        .unwrap_err();
        assert!(
            matches!(err, Error::MigrationFailed { version: 3, .. }),
            "{err}"
        );
        let status = schema_status(&conn).unwrap();
        assert_eq!(status.version, 2);
        assert!(
            !table_exists(&conn, "half_done"),
            "partial migration leaked"
        );
        let r = migrate_with(
            &mut conn,
            &[BASE, ADD_COLUMN, ADD_TABLE],
            MigrateOptions::default(),
        )
        .unwrap();
        assert_eq!(r.applied, vec![3]);
    }

    #[test]
    fn edited_migration_is_detected() {
        let mut conn = mem();
        migrate_with(&mut conn, &[BASE], MigrateOptions::default()).unwrap();
        let edited = Migration {
            sql: "CREATE TABLE a (id INTEGER PRIMARY KEY, v TEXT, extra INTEGER) STRICT;",
            ..BASE
        };
        let err = migrate_with(&mut conn, &[edited], MigrateOptions::default()).unwrap_err();
        assert!(
            matches!(err, Error::MigrationChecksum { version: 1 }),
            "{err}"
        );
    }

    #[test]
    fn newer_schema_opens_only_if_compatible() {
        // A newer build added an expand-only step: this build can still open it.
        let mut conn = mem();
        migrate_with(&mut conn, &[BASE, ADD_COLUMN], MigrateOptions::default()).unwrap();
        let r = migrate_with(&mut conn, &[BASE], MigrateOptions::default()).unwrap();
        assert!(r.opened_newer_compatible);
        assert_eq!((r.from, r.to), (2, 2));
        assert!(r.applied.is_empty());

        // A newer build contracted the schema: this build must refuse it.
        let contract = Migration {
            compat: Compat::Contract,
            ..ADD_COLUMN
        };
        let mut conn = mem();
        migrate_with(&mut conn, &[BASE, contract], MigrateOptions::default()).unwrap();
        let err = migrate_with(&mut conn, &[BASE], MigrateOptions::default()).unwrap_err();
        assert!(
            matches!(
                err,
                Error::SchemaTooNew {
                    found: 2,
                    required_reader: 2,
                    supported: 1
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn malformed_lists_are_rejected() {
        let mut conn = mem();
        let starts_at_two = Migration { version: 2, ..BASE };
        assert!(matches!(
            migrate_with(&mut conn, &[starts_at_two], MigrateOptions::default()),
            Err(Error::BadMigrationList(_))
        ));
        assert!(matches!(
            migrate_with(&mut conn, &[BASE, ADD_TABLE], MigrateOptions::default()),
            Err(Error::BadMigrationList(_))
        ));
        let expand_first = Migration {
            compat: Compat::Expand,
            ..BASE
        };
        assert!(matches!(
            migrate_with(&mut conn, &[expand_first], MigrateOptions::default()),
            Err(Error::BadMigrationList(_))
        ));
        let empty = Migration { sql: "  ", ..BASE };
        assert!(matches!(
            migrate_with(&mut conn, &[empty], MigrateOptions::default()),
            Err(Error::BadMigrationList(_))
        ));
        assert_eq!(schema_status(&conn).unwrap().version, 0);
    }

    #[test]
    fn inconsistent_bookkeeping_is_refused() {
        let mut conn = mem();
        conn.pragma_update(None, "user_version", 5).unwrap();
        assert!(matches!(migrate(&mut conn), Err(Error::Invalid(_))));
    }

    #[test]
    fn upgrading_a_file_database_writes_a_pre_migration_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hy.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            let r = migrate_with(&mut conn, &[BASE], MigrateOptions::default()).unwrap();
            assert_eq!(r.pre_migration_copy, None, "a new database needs no copy");
            conn.execute("INSERT INTO a (v) VALUES ('before upgrade')", [])
                .unwrap();
        }
        let mut conn = Connection::open(&path).unwrap();
        let r = migrate_with(&mut conn, &[BASE, ADD_COLUMN], MigrateOptions::default()).unwrap();
        let copy = r.pre_migration_copy.expect("copy written");
        assert_eq!(copy, dir.path().join("hy.db.pre-v1.bak"));
        let old = Connection::open(&copy).unwrap();
        assert_eq!(schema_status(&old).unwrap().version, 1);
        let v: String = old.query_row("SELECT v FROM a", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "before upgrade");
        // No copy when nothing is pending, or when disabled.
        let r = migrate_with(&mut conn, &[BASE, ADD_COLUMN], MigrateOptions::default()).unwrap();
        assert_eq!(r.pre_migration_copy, None);
        let r = migrate_with(
            &mut conn,
            &[BASE, ADD_COLUMN, ADD_TABLE],
            MigrateOptions {
                copy_before_upgrade: false,
            },
        )
        .unwrap();
        assert_eq!(r.pre_migration_copy, None);
        assert!(!dir.path().join("hy.db.pre-v2.bak").exists());
    }

    #[test]
    fn a_stale_copy_from_a_failed_attempt_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hy.db");
        let mut conn = Connection::open(&path).unwrap();
        migrate_with(&mut conn, &[BASE], MigrateOptions::default()).unwrap();
        std::fs::write(dir.path().join("hy.db.pre-v1.bak"), b"stale").unwrap();
        let r = migrate_with(&mut conn, &[BASE, ADD_COLUMN], MigrateOptions::default()).unwrap();
        let old = Connection::open(r.pre_migration_copy.unwrap()).unwrap();
        assert_eq!(schema_status(&old).unwrap().version, 1);
    }

    #[test]
    fn check_lists_in_the_migration_match_the_rust_enums() {
        use crate::schema::*;
        let sql = MIGRATIONS[0].sql;
        let list = |values: Vec<&str>| {
            values
                .iter()
                .map(|v| format!("'{v}'"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let expect = |column: &str, values: Vec<&str>| {
            let needle = format!("{column} IN ({})", list(values));
            assert!(sql.contains(&needle), "migration lacks `{needle}`");
        };
        expect(
            "source",
            EventSource::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect(
            "status",
            MeasurementStatus::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect(
            "category",
            Category::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect("kind", GoalKind::ALL.iter().map(|v| v.as_str()).collect());
        expect(
            "kind",
            PolicyOpKind::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect(
            "state",
            PolicyOpState::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect(
            "state",
            TombstoneState::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect(
            "provenance",
            ImportProvenance::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect(
            "kind",
            ConsentKind::ALL.iter().map(|v| v.as_str()).collect(),
        );
        expect("edge", PetEdge::ALL.iter().map(|v| v.as_str()).collect());
        expect("tone", PetTone::ALL.iter().map(|v| v.as_str()).collect());
        // Every category gets a seeded, switched-off policy row.
        for c in Category::ALL {
            assert!(
                sql.contains(&format!("('{c}', 0, NULL, 0, 0)")),
                "no seed row for {c}"
            );
        }
    }
}
