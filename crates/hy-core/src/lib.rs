//! `hy-core`: the shared core of HealthierYou (architecture section 7).
//!
//! Slice V1-0 provides:
//!
//! - [`clock`]: the injectable wall and monotonic clock, and clock-change detection;
//! - [`time`]: the user's IANA time zone (default `Asia/Kolkata`), DST-safe local-day
//!   boundaries, splitting intervals at local midnight, and work-hour windows;
//! - [`ids`]: monotonic UUIDv7 event identifiers;
//! - [`schema`]: the event envelope, typed payloads, enumerations and their validation;
//! - [`migrations`]: the forward-only, checksummed, versioned migration runner;
//! - [`store`]: the local SQLite store (architecture 6.1).
//!
//! Slice V1-2 adds [`coach`]: the activity tracker, the coaching engine (one reminder at a
//! time, suppression and precedence, focus, pause, meeting, presentation, Pomodoro) and
//! its persistence.
//!
//! Slice V1-4 adds [`review`]: the weekly and monthly reviews, built from per-day tallies
//! and the current view of the event log, with "Try next week" suggestions; and
//! [`prefs`]: the display preferences (text size, reduced motion, contrast); and
//! [`backup`]: the encrypted full personal backup file.
//!
//! Slice V1-5 adds [`protocol`]: the office-server protocol shared by the server, the
//! companion and the simulator (pairing slip, fingerprint words, control-log chain,
//! signed heads, `/v1` bodies); [`secret`]: redacting, zeroizing secret values; and, with
//! the `link` feature, [`link`]: the pinned TLS 1.3 client.
//!
//! Slice V1-6 adds [`update`]: signed update files (minisign signatures checked against
//! built-in keys), what may be installed, the update record, keeping and restoring the
//! previous program files, and restoring the pre-migration copy of the database.

pub mod backup;
pub mod clock;
pub mod coach;
pub mod error;
pub mod ids;
#[cfg(feature = "link")]
pub mod link;
pub mod migrations;
pub mod prefs;
pub mod protocol;
pub mod review;
pub mod schema;
pub mod secret;
pub mod settings;
pub mod store;
pub mod time;
pub mod update;

pub use error::{Error, Result};

/// Version of this crate, reported by the companion and server skeletons.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
