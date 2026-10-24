//! Soak statistics (slice V1-7, architecture 10 and 11): what an unattended run left in
//! the local store, read without changing it.
//!
//! The 8-hour soak script (`scripts/windows/Test-Soak.ps1`) samples the running
//! companion's processes from outside. What only the store knows (how big the database
//! and its write-ahead log grew, and when reminders were offered) it reads through
//! `hy-companion.exe --hy-soak-stats`, which calls [`read`]. The database is opened
//! read-only, never migrated or created, so the running app is not disturbed (SQLite's
//! write-ahead log lets a reader in while the app writes).
//!
//! The result holds counts, sizes and minute offsets only: no payload, amount, note,
//! label, path or device id is read or returned, so the report can be shared.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::error::{Error, Result};

/// What the store held for a soak window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoakStats {
    /// Size of the database file, in bytes.
    pub db_bytes: u64,
    /// Size of its write-ahead log (`-wal`), in bytes; 0 if there is none.
    pub wal_bytes: u64,
    /// Events recorded since the window started, by event type (for example
    /// `reminder.offered`). Types only: never a payload.
    pub events_by_type: BTreeMap<String, u64>,
    /// When each reminder was offered, in minutes since the window started, in order.
    pub offers_at_min: Vec<f64>,
    /// The shortest time between two consecutive offers, in minutes (`None` with fewer
    /// than two offers). The engine's cooldown is 5 minutes (architecture 4.3), so a
    /// smaller gap is a burst.
    pub min_offer_gap_min: Option<f64>,
    /// The most offers in any 60-minute window starting at an offer.
    pub max_offers_in_an_hour: u32,
}

/// The soak statistics of the store at `db` since `since_ms` (Unix milliseconds, UTC).
///
/// # Errors
/// [`Error::NotFound`] if there is no database at `db`; [`Error::Sqlite`] if it cannot be
/// read; [`Error::Io`] if its size cannot be read.
pub fn read(db: &Path, since_ms: i64) -> Result<SoakStats> {
    if !db.is_file() {
        return Err(Error::NotFound("the HealthierYou database".into()));
    }
    let db_bytes = std::fs::metadata(db)?.len();
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    let wal_bytes = std::fs::metadata(Path::new(&wal)).map_or(0, |m| m.len());
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let mut events_by_type = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT type, count(*) FROM events WHERE recorded_at >= ?1 GROUP BY type ORDER BY type",
        )?;
        let rows = stmt.query_map([since_ms], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (kind, n) = row?;
            events_by_type.insert(kind, u64::try_from(n).unwrap_or(0));
        }
    }
    let offers: Vec<i64> = conn
        .prepare(
            "SELECT occurred_at FROM events WHERE type = 'reminder.offered' AND recorded_at >= ?1
             ORDER BY occurred_at",
        )?
        .query_map([since_ms], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(summarise(
        db_bytes,
        wal_bytes,
        events_by_type,
        &offers,
        since_ms,
    ))
}

/// Minutes between two Unix-millisecond times, to 0.1 minute.
#[allow(clippy::cast_precision_loss)]
fn minutes(from_ms: i64, to_ms: i64) -> f64 {
    ((to_ms - from_ms) as f64 / 6_000.0).round() / 10.0
}

fn summarise(
    db_bytes: u64,
    wal_bytes: u64,
    events_by_type: BTreeMap<String, u64>,
    offers: &[i64],
    since_ms: i64,
) -> SoakStats {
    let min_offer_gap_min = offers
        .windows(2)
        .map(|w| w[1] - w[0])
        .min()
        .map(|gap| minutes(0, gap));
    let max_offers_in_an_hour = offers
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            offers[i..]
                .iter()
                .take_while(|&&t| t < start + 3_600_000)
                .count()
        })
        .max()
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
    SoakStats {
        db_bytes,
        wal_bytes,
        events_by_type,
        offers_at_min: offers.iter().map(|&t| minutes(since_ms, t)).collect(),
        min_offer_gap_min,
        max_offers_in_an_hour,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;

    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::clock::{Clock, ManualClock};
    use crate::schema::{Activity, EventPayload, NewEvent};
    use crate::store::Store;

    #[test]
    fn gaps_and_hourly_peaks_are_computed_from_offer_times() {
        let m = 60_000;
        let s = summarise(
            10,
            0,
            BTreeMap::new(),
            &[5 * m, 50 * m, 56 * m, 100 * m, 200 * m],
            0,
        );
        assert_eq!(s.offers_at_min, vec![5.0, 50.0, 56.0, 100.0, 200.0]);
        assert_eq!(s.min_offer_gap_min, Some(6.0));
        // 50, 56 and 100 fall in the hour that starts at 50.
        assert_eq!(s.max_offers_in_an_hour, 3);
        let none = summarise(0, 0, BTreeMap::new(), &[], 0);
        assert_eq!(none.min_offer_gap_min, None);
        assert_eq!(none.max_offers_in_an_hour, 0);
    }

    #[test]
    fn a_missing_database_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("hy.db");
        assert!(matches!(read(&db, 0), Err(Error::NotFound(_))));
        assert!(!db.exists(), "a read never creates a database");
    }

    #[test]
    fn a_store_in_use_is_read_without_payloads_or_changes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("hy.db");
        let start = Utc.with_ymd_and_hms(2026, 10, 26, 5, 0, 0).unwrap();
        let clock = Arc::new(ManualClock::new(start));
        let mut store = Store::open(&db, clock.clone()).unwrap();
        let before = clock.now().timestamp_millis();
        for (i, activity) in [Activity::Movement, Activity::Water]
            .into_iter()
            .enumerate()
        {
            clock.advance(chrono::Duration::minutes(45));
            store
                .append_event(NewEvent::system(EventPayload::ReminderOffered {
                    reminder_id: uuid::Uuid::from_u128(u128::try_from(i).unwrap() + 1),
                    activity,
                }))
                .unwrap();
        }
        store
            .append_event(NewEvent::manual(EventPayload::WaterLogged {
                amount_ml: 987_654 % 2000,
            }))
            .unwrap();
        // The store stays open (as in the running app) while it is read.
        let s = read(&db, before).unwrap();
        assert!(s.db_bytes > 0);
        assert_eq!(s.events_by_type.get("reminder.offered"), Some(&2));
        assert_eq!(s.events_by_type.get("water.logged"), Some(&1));
        assert_eq!(s.offers_at_min, vec![45.0, 90.0]);
        assert_eq!(s.min_offer_gap_min, Some(45.0));
        assert_eq!(s.max_offers_in_an_hour, 2);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("1654"), "no amount is read: {json}");
        assert!(!json.contains(&dir.path().display().to_string()), "no path");
        // Nothing after the window's start is counted twice, and a later window is empty.
        let later = read(&db, clock.now().timestamp_millis() + 1).unwrap();
        assert!(later.events_by_type.is_empty() && later.offers_at_min.is_empty());
        drop(store);
    }
}
