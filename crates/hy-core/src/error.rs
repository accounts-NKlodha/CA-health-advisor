//! Error type shared by every `hy-core` module.

/// Result alias used throughout `hy-core`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Every failure `hy-core` can report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite reported an error that is not one of the cases below.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A JSON value could not be encoded or decoded.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A file-system operation failed (for example the pre-migration copy).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The name is not a time zone in the bundled IANA database.
    #[error("unknown time zone {0:?}")]
    InvalidTimeZone(String),

    /// A value broke a domain rule before it reached the database.
    #[error("invalid: {0}")]
    Invalid(String),

    /// A record was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// The store refused to queue a record for upload (5.3 device side).
    #[error("upload blocked: {0}")]
    UploadBlocked(String),

    /// A write was based on state that another writer has since replaced (for example a
    /// coaching-engine step committed against an older revision). Nothing was written.
    #[error("stale: {0}")]
    Stale(String),

    /// The database was written by a newer build whose schema this build cannot use.
    #[error(
        "database schema v{found} needs a reader of at least v{required_reader}; this build supports up to v{supported}"
    )]
    SchemaTooNew {
        /// Schema version found in the database.
        found: u32,
        /// Lowest schema version a reader must support to open it.
        required_reader: u32,
        /// Highest schema version this build knows.
        supported: u32,
    },

    /// An applied migration no longer matches the migration shipped in this build.
    #[error("migration v{version} was changed after it was applied (checksum mismatch)")]
    MigrationChecksum {
        /// Version of the edited migration.
        version: u32,
    },

    /// A migration failed and was rolled back; earlier migrations stay applied.
    #[error("migration v{version} ({name}) failed and was rolled back: {source}")]
    MigrationFailed {
        /// Version of the failed migration.
        version: u32,
        /// Name of the failed migration.
        name: &'static str,
        /// Underlying SQLite error.
        source: rusqlite::Error,
    },

    /// The migration list itself is malformed (a build defect, caught by tests).
    #[error("bad migration list: {0}")]
    BadMigrationList(String),
}

impl Error {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }
}
