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
//! its persistence. Later slices add reviews and sync on top of this crate.
//! Pairing, the server link (certificate pin and token) and the signed receipt are not
//! part of V1-0: they wait for the pairing security gate before V1-5.

pub mod clock;
pub mod coach;
pub mod error;
pub mod ids;
pub mod migrations;
pub mod schema;
pub mod store;
pub mod time;

pub use error::{Error, Result};

/// Version of this crate, reported by the companion and server skeletons.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
