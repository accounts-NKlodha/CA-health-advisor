//! The local SQLite store (`%LOCALAPPDATA%\HealthierYou\data\hy.db`; architecture 6.1).
//!
//! The store opens the database in WAL mode with `synchronous=FULL` and foreign keys on,
//! runs the migrations, and gives this PC a stable origin-device id. It is single-writer:
//! wrap it in a mutex to share it between threads.
//!
//! Device-side control changes (architecture 5.3) are committed atomically: a disable, its
//! policy operation, its consent-log entry, its event and the purge of that category's
//! upload queue are one transaction, and the same holds for a deletion tombstone. Schema
//! triggers enforce the queue rules as a second line of defence, so a blocked date can
//! never be queued even by code that bypasses this API. The link to the office server,
//! its receipt and the device list are in `link` (V1-5).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Months, NaiveDate, Utc};
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{Error, Result};
use crate::ids::IdGenerator;
use crate::migrations::{self, MigrationReport};
use crate::schema::{
    Category, ConsentKind, DateRange, Event, EventPayload, EventSource, FUTURE_TOLERANCE, GoalKind,
    ImportProvenance, MeasurementStatus, NewEvent, PetEdge, PetTone, PolicyOpKind, PolicyOpState,
    TombstoneState, WATER_GOAL_RANGE_ML,
};
use crate::time::{UserZone, WorkSchedule};

mod backup;
mod coach;
mod link;
mod prefs;
mod review;
#[cfg(test)]
mod tests;

pub use coach::{ActivityDay, CoachCommit, SavedCoachState};
pub use link::{NewLink, PairedDevice, ServerLink, StateOutcome};

/// Setting keys with typed accessors.
pub mod keys {
    /// The user's IANA zone (`"Asia/Kolkata"`).
    pub const TIME_ZONE: &str = "time_zone";
    /// The user's [`crate::time::WorkSchedule`].
    pub const WORK_SCHEDULE: &str = "work_schedule";
}

/// Detailed events and daily summaries are kept for this many days (P).
pub const EVENT_RETENTION_DAYS: u64 = 90;
/// Monthly summaries are kept for this many months (P).
pub const MONTHLY_RETENTION_MONTHS: u32 = 24;

const DATE_FORMAT: &str = "%Y-%m-%d";

pub(crate) fn date_text(d: NaiveDate) -> String {
    d.format(DATE_FORMAT).to_string()
}

pub(crate) fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, DATE_FORMAT)
        .map_err(|e| Error::invalid(format!("stored date {s:?}: {e}")))
}

pub(crate) fn to_ms(t: DateTime<Utc>) -> i64 {
    t.timestamp_millis()
}

pub(crate) fn from_ms(ms: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
        .ok_or_else(|| Error::invalid(format!("timestamp {ms} out of range")))
}

fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| Error::invalid(format!("stored id {s:?}: {e}")))
}

/// Map an outbox-trigger refusal to [`Error::UploadBlocked`].
fn map_upload_error(e: rusqlite::Error) -> Error {
    if let rusqlite::Error::SqliteFailure(f, Some(msg)) = &e
        && f.code == ErrorCode::ConstraintViolation
        && let Some(reason) = msg.strip_prefix("upload_blocked: ")
    {
        return Error::UploadBlocked(reason.to_owned());
    }
    Error::Sqlite(e)
}

/// A month key such as `2026-10`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonthKey {
    year: i32,
    month: u32,
}

impl MonthKey {
    /// A month key.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the month is not 1 to 12 or the year not 1 to 9999.
    pub fn new(year: i32, month: u32) -> Result<Self> {
        if !(1..=12).contains(&month) || !(1..=9999).contains(&year) {
            return Err(Error::invalid(format!("bad month {year}-{month}")));
        }
        Ok(Self { year, month })
    }

    /// The month containing `date`.
    #[must_use]
    pub fn of(date: NaiveDate) -> Self {
        use chrono::Datelike;
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    /// The year.
    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }

    /// The month, 1 to 12.
    #[must_use]
    pub const fn month(self) -> u32 {
        self.month
    }

    /// The first day of the month.
    #[must_use]
    pub fn first_day(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap_or(NaiveDate::MIN)
    }

    /// The last day of the month.
    #[must_use]
    pub fn last_day(self) -> NaiveDate {
        self.succ()
            .first_day()
            .pred_opt()
            .unwrap_or_else(|| self.first_day())
    }

    /// The month before (kept at year 1 for January of year 1).
    #[must_use]
    pub const fn pred(self) -> Self {
        match (self.month, self.year) {
            (1, 1) => self,
            (1, y) => Self {
                year: y - 1,
                month: 12,
            },
            (m, y) => Self {
                year: y,
                month: m - 1,
            },
        }
    }

    /// The month after (kept at year 9999 for December of year 9999).
    #[must_use]
    pub const fn succ(self) -> Self {
        match (self.month, self.year) {
            (12, 9999) => self,
            (12, y) => Self {
                year: y + 1,
                month: 1,
            },
            (m, y) => Self {
                year: y,
                month: m + 1,
            },
        }
    }

    fn text(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    fn parse(s: &str) -> Result<Self> {
        let (y, m) = s
            .split_once('-')
            .ok_or_else(|| Error::invalid(format!("stored month {s:?}")))?;
        let year = y
            .parse()
            .map_err(|_| Error::invalid(format!("stored month {s:?}")))?;
        let month = m
            .parse()
            .map_err(|_| Error::invalid(format!("stored month {s:?}")))?;
        Self::new(year, month)
    }
}

impl std::fmt::Display for MonthKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::str::FromStr for MonthKey {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.len() != 7 || s.as_bytes().get(4) != Some(&b'-') {
            return Err(Error::invalid(format!("month {s:?} is not YYYY-MM")));
        }
        Self::parse(s)
    }
}

impl Serialize for MonthKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.text())
    }
}

impl<'de> serde::Deserialize<'de> for MonthKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A daily or monthly summary row for one category and origin device.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary<P> {
    /// The category.
    pub category: Category,
    /// The PC that recorded the underlying events.
    pub origin_device: Uuid,
    /// The local date or month.
    pub period: P,
    /// Increases with every rebuild; a row is replaced only by a higher revision.
    pub revision: u32,
    /// Share of planned observation actually covered, 0 to 1, where meaningful.
    pub coverage: Option<f64>,
    /// The category's summary fields.
    pub payload: serde_json::Value,
}

/// A daily summary.
pub type DailySummary = Summary<NaiveDate>;
/// A monthly summary.
pub type MonthlySummary = Summary<MonthKey>;

impl<P> Summary<P> {
    fn validate(&self) -> Result<()> {
        if self.revision == 0 {
            return Err(Error::invalid("summary revision starts at 1"));
        }
        if let Some(c) = self.coverage
            && !(0.0..=1.0).contains(&c)
        {
            return Err(Error::invalid(format!("coverage {c} is outside 0 to 1")));
        }
        if !self.payload.is_object() {
            return Err(Error::invalid("summary payload must be a JSON object"));
        }
        Ok(())
    }
}

/// The user's pet and edits (wireframe W3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetProfile {
    /// Pack id of the chosen pet.
    pub pet_id: String,
    /// Display name, 1 to 20 characters.
    pub display_name: String,
    /// Size, 75 to 150 percent.
    pub scale_pct: u16,
    /// Colour variant, 0 to 2.
    pub colour_variant: u8,
    /// Screen edge.
    pub edge: PetEdge,
    /// Dropped position for [`PetEdge::Free`].
    pub position: Option<(i32, i32)>,
    /// Animation on or off.
    pub motion: bool,
    /// Wording tone.
    pub tone: PetTone,
}

impl PetProfile {
    /// The defaults that "Reset to defaults" restores for a pet.
    #[must_use]
    pub fn defaults(pet_id: &str, default_name: &str) -> Self {
        Self {
            pet_id: pet_id.to_owned(),
            display_name: default_name.to_owned(),
            scale_pct: 100,
            colour_variant: 0,
            edge: PetEdge::Right,
            position: None,
            motion: true,
            tone: PetTone::Gentle,
        }
    }

    /// Check the profile's rules.
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        let name_len = self.display_name.chars().count();
        if !(1..=20).contains(&name_len) || self.display_name.trim().is_empty() {
            return Err(Error::invalid("pet name must be 1 to 20 characters"));
        }
        if self.pet_id.is_empty() || self.pet_id.len() > 64 {
            return Err(Error::invalid("bad pet id"));
        }
        if !(75..=150).contains(&self.scale_pct) {
            return Err(Error::invalid("pet size must be 75 to 150 percent"));
        }
        if self.colour_variant > 2 {
            return Err(Error::invalid("pet colour variant must be 0 to 2"));
        }
        Ok(())
    }
}

/// The local copy of one category's sharing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyState {
    /// The category.
    pub category: Category,
    /// On only after the server accepted an enable.
    pub enabled: bool,
    /// Dates before this are never uploaded.
    pub share_from: Option<NaiveDate>,
    /// Highest server sequence number seen for this category.
    pub seq: u64,
}

/// A local sharing-policy operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyOp {
    /// Idempotency key.
    pub op_id: Uuid,
    /// Durable local order: later user actions have higher numbers, across restarts and
    /// wall-clock changes.
    pub local_seq: u64,
    /// The category.
    pub category: Category,
    /// Enable or disable.
    pub kind: PolicyOpKind,
    /// The policy seq the device had when the user acted.
    pub base_seq: u64,
    /// Where the operation stands.
    pub state: PolicyOpState,
    /// The server's seq once accepted.
    pub server_seq: Option<u64>,
}

/// A deletion tombstone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tombstone {
    /// Idempotency key.
    pub request_id: Uuid,
    /// The category.
    pub category: Category,
    /// The deleted dates.
    pub range: DateRange,
    /// Where the request stands.
    pub state: TombstoneState,
}

/// Why a date cannot be uploaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadBlock {
    /// The category is off (or its enable is not yet accepted).
    CategoryOff,
    /// The date is before the category's `share_from`.
    BeforeShareFrom,
    /// The date is inside a deletion tombstone.
    Tombstoned,
}

/// What [`Store::accept_enable`] did with a server acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnableOutcome {
    /// The category is now on from the response's `share_from`.
    Applied,
    /// The user switched the category off after asking, so it stays off.
    SupersededByLocalDisable,
    /// A response with the same or a higher server seq was already applied; the policy
    /// is unchanged.
    StaleResponse,
}

/// A queued upload record.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxRecord {
    /// Queue position.
    pub id: i64,
    /// The category.
    pub category: Category,
    /// Always this PC.
    pub origin_device: Uuid,
    /// The summary's local date.
    pub local_date: NaiveDate,
    /// The summary revision.
    pub revision: u32,
    /// The summary fields.
    pub payload: serde_json::Value,
}

/// A consent-log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentEntry {
    /// When it was written.
    pub at: DateTime<Utc>,
    /// What happened.
    pub kind: ConsentKind,
    /// The category, if any.
    pub category: Option<Category>,
    /// The notice version shown, if any.
    pub notice_version: Option<String>,
    /// The policy operation or deletion request it belongs to.
    pub op_ref: Option<Uuid>,
}

/// Rows removed by [`Store::prune`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Events.
    pub events: usize,
    /// Daily summaries.
    pub daily: usize,
    /// Imported daily summaries.
    pub imported: usize,
    /// Monthly summaries.
    pub monthly: usize,
}

const EVENT_COLUMNS: &str = "id, schema_version, type, occurred_at, recorded_at, zone, local_date, \
     source, source_ref, status, confidence, supersedes, origin_device, payload";

struct RawEvent {
    id: String,
    schema_version: u32,
    type_name: String,
    occurred_at: i64,
    recorded_at: i64,
    zone: String,
    local_date: String,
    source: EventSource,
    source_ref: Option<String>,
    status: MeasurementStatus,
    confidence: Option<f64>,
    supersedes: Option<String>,
    origin_device: String,
    payload: String,
}

impl RawEvent {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            schema_version: r.get(1)?,
            type_name: r.get(2)?,
            occurred_at: r.get(3)?,
            recorded_at: r.get(4)?,
            zone: r.get(5)?,
            local_date: r.get(6)?,
            source: r.get(7)?,
            source_ref: r.get(8)?,
            status: r.get(9)?,
            confidence: r.get(10)?,
            supersedes: r.get(11)?,
            origin_device: r.get(12)?,
            payload: r.get(13)?,
        })
    }

    fn into_event(self) -> Result<Event> {
        Ok(Event {
            id: parse_uuid(&self.id)?,
            schema_version: self.schema_version,
            occurred_at: from_ms(self.occurred_at)?,
            recorded_at: from_ms(self.recorded_at)?,
            zone: UserZone::parse(&self.zone)?,
            local_date: parse_date(&self.local_date)?,
            source: self.source,
            source_ref: self.source_ref,
            status: self.status,
            confidence: self.confidence,
            supersedes: self.supersedes.as_deref().map(parse_uuid).transpose()?,
            origin_device: parse_uuid(&self.origin_device)?,
            payload: EventPayload::from_stored(&self.type_name, &self.payload)?,
        })
    }
}

fn query_events(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(sql)?;
    let raws = stmt
        .query_map(args, RawEvent::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raws.into_iter().map(RawEvent::into_event).collect()
}

fn load_event(conn: &Connection, id: Uuid) -> Result<Option<Event>> {
    let sql = format!("SELECT {EVENT_COLUMNS} FROM events WHERE id = ?1");
    Ok(query_events(conn, &sql, [id.to_string()])?
        .into_iter()
        .next())
}

fn read_setting<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    let text: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    text.map(|t| serde_json::from_str(&t).map_err(Error::from))
        .transpose()
}

fn read_zone(conn: &Connection) -> Result<UserZone> {
    Ok(read_setting(conn, keys::TIME_ZONE)?.unwrap_or_default())
}

fn read_policy(conn: &Connection, category: Category) -> Result<PolicyState> {
    let (enabled, share_from, seq): (bool, Option<String>, i64) = conn.query_row(
        "SELECT enabled, share_from, seq FROM sharing_policy WHERE category = ?1",
        [category],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(PolicyState {
        category,
        enabled,
        share_from: share_from.as_deref().map(parse_date).transpose()?,
        seq: u64::try_from(seq).unwrap_or(0),
    })
}

fn upload_block_in(
    conn: &Connection,
    category: Category,
    date: NaiveDate,
) -> Result<Option<UploadBlock>> {
    let policy = read_policy(conn, category)?;
    if !policy.enabled {
        return Ok(Some(UploadBlock::CategoryOff));
    }
    if policy.share_from.is_some_and(|from| date < from) {
        return Ok(Some(UploadBlock::BeforeShareFrom));
    }
    let tombstoned: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM tombstones
         WHERE category = ?1 AND ?2 BETWEEN from_date AND to_date)",
        params![category, date_text(date)],
        |r| r.get(0),
    )?;
    Ok(tombstoned.then_some(UploadBlock::Tombstoned))
}

fn insert_consent(
    conn: &Connection,
    at: DateTime<Utc>,
    kind: ConsentKind,
    category: Option<Category>,
    notice_version: Option<&str>,
    op_ref: Option<Uuid>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO consent_log (at, kind, category, notice_version, op_ref) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            to_ms(at),
            kind,
            category,
            notice_version,
            op_ref.map(|u| u.to_string())
        ],
    )?;
    Ok(())
}

/// Everything needed to insert one event row.
struct EventInsert<'a> {
    id: Uuid,
    new: &'a NewEvent,
    occurred_at: DateTime<Utc>,
    recorded_at: DateTime<Utc>,
    zone: UserZone,
    origin_device: Uuid,
}

fn insert_event(conn: &Connection, e: EventInsert<'_>) -> Result<Event> {
    let (type_name, payload) = e.new.payload.to_stored()?;
    let local_date = e.zone.local_date(e.occurred_at);
    conn.execute(
        &format!(
            "INSERT INTO events ({EVENT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
        ),
        params![
            e.id.to_string(),
            crate::schema::EVENT_SCHEMA_VERSION,
            type_name,
            to_ms(e.occurred_at),
            to_ms(e.recorded_at),
            e.zone.name(),
            date_text(local_date),
            e.new.source,
            e.new.source_ref,
            e.new.status,
            e.new.confidence,
            e.new.supersedes.map(|u| u.to_string()),
            e.origin_device.to_string(),
            payload,
        ],
    )?;
    Ok(Event {
        id: e.id,
        schema_version: crate::schema::EVENT_SCHEMA_VERSION,
        occurred_at: from_ms(to_ms(e.occurred_at))?,
        recorded_at: from_ms(to_ms(e.recorded_at))?,
        zone: e.zone,
        local_date,
        source: e.new.source,
        source_ref: e.new.source_ref.clone(),
        status: e.new.status,
        confidence: e.new.confidence,
        supersedes: e.new.supersedes,
        origin_device: e.origin_device,
        payload: e.new.payload.clone(),
    })
}

/// Allocate the next durable local operation number inside the caller's transaction.
fn next_policy_op_seq(tx: &Connection) -> Result<i64> {
    Ok(tx.query_row(
        "UPDATE store_meta SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
         WHERE key = 'policy_op_seq' RETURNING CAST(value AS INTEGER)",
        [],
        |r| r.get(0),
    )?)
}

fn load_policy_op(conn: &Connection, op_id: Uuid) -> Result<Option<PolicyOp>> {
    let row: Option<(i64, Category, PolicyOpKind, i64, PolicyOpState, Option<i64>)> = conn
        .query_row(
            "SELECT local_seq, category, kind, base_seq, state, server_seq FROM policy_ops WHERE op_id = ?1",
            [op_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    Ok(row.map(
        |(local_seq, category, kind, base_seq, state, server_seq)| PolicyOp {
            op_id,
            local_seq: u64::try_from(local_seq).unwrap_or(0),
            category,
            kind,
            base_seq: u64::try_from(base_seq).unwrap_or(0),
            state,
            server_seq: server_seq.and_then(|s| u64::try_from(s).ok()),
        },
    ))
}

fn sharing_event(
    tx: &Connection,
    ids: &IdGenerator,
    device: Uuid,
    now: DateTime<Utc>,
    category: Category,
    enabled: bool,
) -> Result<()> {
    let new = NewEvent::manual(EventPayload::SharingPolicyChanged { category, enabled });
    insert_event(
        tx,
        EventInsert {
            id: ids.next(now),
            new: &new,
            occurred_at: now,
            recorded_at: now,
            zone: read_zone(tx)?,
            origin_device: device,
        },
    )?;
    Ok(())
}

/// The local store.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
    clock: Arc<dyn Clock>,
    ids: IdGenerator,
    device_id: Uuid,
    migration: MigrationReport,
}

impl Store {
    /// Open (creating if needed) and migrate the database at `path`.
    ///
    /// # Errors
    /// Any error from SQLite, the migrations or the pre-migration copy.
    pub fn open(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::init(Connection::open(path)?, clock)
    }

    /// An in-memory store, for tests and simulations.
    ///
    /// # Errors
    /// Any error from SQLite or the migrations.
    pub fn open_in_memory(clock: Arc<dyn Clock>) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, clock)
    }

    fn init(mut conn: Connection, clock: Arc<dyn Clock>) -> Result<Self> {
        conn.busy_timeout(StdDuration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let fk: bool = conn.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
        if !fk {
            return Err(Error::invalid("SQLite foreign keys are unavailable"));
        }
        let migration = migrations::migrate(&mut conn)?;
        let ids = IdGenerator::new();
        let existing: Option<String> = conn
            .query_row(
                "SELECT value FROM store_meta WHERE key = 'device_id'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let device_id = match existing {
            Some(text) => parse_uuid(&text)?,
            None => {
                let id = ids.next(clock.now());
                conn.execute(
                    "INSERT INTO store_meta (key, value) VALUES ('device_id', ?1), ('created_at', ?2)",
                    params![id.to_string(), to_ms(clock.now()).to_string()],
                )?;
                id
            }
        };
        Ok(Self {
            conn,
            clock,
            ids,
            device_id,
            migration,
        })
    }

    /// This PC's origin-device id, created once and kept for the life of the database.
    #[must_use]
    pub fn device_id(&self) -> Uuid {
        self.device_id
    }

    /// What the migrations did when the store was opened.
    #[must_use]
    pub fn migration_report(&self) -> &MigrationReport {
        &self.migration
    }

    /// The injected clock.
    #[must_use]
    pub fn clock(&self) -> &dyn Clock {
        &*self.clock
    }

    /// Today's local date in the user's zone, from the current wall clock.
    ///
    /// # Errors
    /// [`Error::Sqlite`] or [`Error::Json`] if the zone setting cannot be read.
    pub fn today(&self) -> Result<NaiveDate> {
        Ok(self.user_zone()?.local_date(self.clock.now()))
    }

    // ---------------------------------------------------------------- settings

    /// Read a setting.
    ///
    /// # Errors
    /// [`Error::Json`] if the stored value does not match `T`.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        read_setting(&self.conn, key)
    }

    /// Write a setting.
    ///
    /// # Errors
    /// [`Error::Sqlite`] if the key is empty or longer than 64 characters.
    pub fn put_setting<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, serde_json::to_string(value)?, to_ms(self.clock.now())],
        )?;
        Ok(())
    }

    /// The user's time zone (default `Asia/Kolkata`).
    ///
    /// # Errors
    /// [`Error::Json`] if the stored zone is not valid.
    pub fn user_zone(&self) -> Result<UserZone> {
        read_zone(&self.conn)
    }

    /// Change the user's time zone. Events already stored keep their zone and local date.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure.
    pub fn set_user_zone(&self, zone: UserZone) -> Result<()> {
        self.put_setting(keys::TIME_ZONE, &zone)
    }

    /// The user's work hours (default Monday to Saturday, 09:30 to 18:30).
    ///
    /// # Errors
    /// [`Error::Json`] if the stored schedule is not valid.
    pub fn work_schedule(&self) -> Result<WorkSchedule> {
        Ok(self.setting(keys::WORK_SCHEDULE)?.unwrap_or_default())
    }

    /// Change the user's work hours.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the schedule breaks its rules.
    pub fn set_work_schedule(&self, schedule: &WorkSchedule) -> Result<()> {
        schedule.validate()?;
        self.put_setting(keys::WORK_SCHEDULE, schedule)
    }

    // ------------------------------------------------------------------- goals

    /// Set a goal from `effective_from` onward. Earlier dates keep the goal they had.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the value is outside the accepted range.
    pub fn set_goal(&self, kind: GoalKind, value: u32, effective_from: NaiveDate) -> Result<()> {
        match kind {
            GoalKind::WaterDailyMl if !WATER_GOAL_RANGE_ML.contains(&value) => {
                return Err(Error::invalid(format!(
                    "water goal {value} ml is outside {} to {} ml",
                    WATER_GOAL_RANGE_ML.start(),
                    WATER_GOAL_RANGE_ML.end()
                )));
            }
            GoalKind::WaterDailyMl => {}
        }
        self.conn.execute(
            "INSERT INTO goals_history (kind, effective_from, value, recorded_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (kind, effective_from) DO UPDATE SET value = excluded.value, recorded_at = excluded.recorded_at",
            params![kind, date_text(effective_from), value, to_ms(self.clock.now())],
        )?;
        Ok(())
    }

    /// The goal in force on `date`, if one had been set by then.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn goal_on(&self, kind: GoalKind, date: NaiveDate) -> Result<Option<u32>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM goals_history WHERE kind = ?1 AND effective_from <= ?2
                 ORDER BY effective_from DESC LIMIT 1",
                params![kind, date_text(date)],
                |r| r.get(0),
            )
            .optional()?)
    }

    // --------------------------------------------------------------------- pet

    /// The saved pet profile, if setup has chosen one.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn pet_profile(&self) -> Result<Option<PetProfile>> {
        Ok(self
            .conn
            .query_row(
                "SELECT pet_id, display_name, scale_pct, colour_variant, edge, pos_x, pos_y, motion, tone
                 FROM pet_profile WHERE id = 1",
                [],
                |r| {
                    let x: Option<i32> = r.get(5)?;
                    let y: Option<i32> = r.get(6)?;
                    Ok(PetProfile {
                        pet_id: r.get(0)?,
                        display_name: r.get(1)?,
                        scale_pct: r.get(2)?,
                        colour_variant: r.get(3)?,
                        edge: r.get(4)?,
                        position: x.zip(y),
                        motion: r.get(7)?,
                        tone: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    /// Save the pet profile (edits apply instantly; wireframe W3).
    ///
    /// # Errors
    /// [`Error::Invalid`] if the profile breaks its rules.
    pub fn save_pet_profile(&self, p: &PetProfile) -> Result<()> {
        p.validate()?;
        self.conn.execute(
            "INSERT INTO pet_profile (id, pet_id, display_name, scale_pct, colour_variant, edge, pos_x, pos_y, motion, tone, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id) DO UPDATE SET pet_id = excluded.pet_id, display_name = excluded.display_name,
               scale_pct = excluded.scale_pct, colour_variant = excluded.colour_variant, edge = excluded.edge,
               pos_x = excluded.pos_x, pos_y = excluded.pos_y, motion = excluded.motion, tone = excluded.tone,
               updated_at = excluded.updated_at",
            params![
                p.pet_id,
                p.display_name,
                p.scale_pct,
                p.colour_variant,
                p.edge,
                p.position.map(|(x, _)| x),
                p.position.map(|(_, y)| y),
                p.motion,
                p.tone,
                to_ms(self.clock.now()),
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------ events

    /// Record an event.
    ///
    /// - The local date is computed from `occurred_at` in the user's current zone and
    ///   stored with the zone; neither is ever rewritten.
    /// - A correction (`supersedes` with the same type) or retraction (`entry.retracted`)
    ///   keeps the corrected entry's zone, and by default its time, so the change lands on
    ///   the day the entry belongs to. An entry can be superseded only once; correct the
    ///   correction to change it again.
    /// - Backfilled times are allowed; times more than five minutes ahead of the clock are
    ///   refused.
    ///
    /// # Errors
    /// [`Error::Invalid`] if a rule is broken, [`Error::NotFound`] if the superseded
    /// event does not exist.
    pub fn append_event(&mut self, new: NewEvent) -> Result<Event> {
        new.validate()?;
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (zone, occurred_at) = match new.supersedes {
            None => (read_zone(&tx)?, new.occurred_at.unwrap_or(now)),
            Some(target_id) => {
                let target = load_event(&tx, target_id)?
                    .ok_or_else(|| Error::NotFound(format!("event {target_id}")))?;
                if !target.payload.is_correctable() {
                    return Err(Error::invalid(format!(
                        "{} cannot be corrected or retracted",
                        target.payload.type_name()
                    )));
                }
                let retraction = matches!(new.payload, EventPayload::EntryRetracted {});
                if !retraction && new.payload.type_name() != target.payload.type_name() {
                    return Err(Error::invalid(
                        "a correction must have the same type as the entry it corrects",
                    ));
                }
                if retraction && new.occurred_at.is_some_and(|t| t != target.occurred_at) {
                    return Err(Error::invalid("a retraction keeps the entry's time"));
                }
                let taken: bool = tx.query_row(
                    "SELECT EXISTS (SELECT 1 FROM events WHERE supersedes = ?1)",
                    [target_id.to_string()],
                    |r| r.get(0),
                )?;
                if taken {
                    return Err(Error::invalid(
                        "this entry was already corrected or removed; change the newest version",
                    ));
                }
                (target.zone, new.occurred_at.unwrap_or(target.occurred_at))
            }
        };
        if occurred_at > now + FUTURE_TOLERANCE {
            return Err(Error::invalid("event time is in the future"));
        }
        let event = insert_event(
            &tx,
            EventInsert {
                id: self.ids.next(now),
                new: &new,
                occurred_at,
                recorded_at: now,
                zone,
                origin_device: self.device_id,
            },
        )?;
        tx.commit()?;
        Ok(event)
    }

    /// One event by id.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn event(&self, id: Uuid) -> Result<Option<Event>> {
        load_event(&self.conn, id)
    }

    /// Every event stored for local date `date`, including superseded ones, in time order.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn events_on(&self, date: NaiveDate) -> Result<Vec<Event>> {
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM events WHERE local_date = ?1 ORDER BY occurred_at, id"
        );
        query_events(&self.conn, &sql, [date_text(date)])
    }

    /// The current view of local date `date`: events that no later event corrects or
    /// retracts, without the retractions themselves. Totals computed from this view
    /// reconcile after edits, undo and day changes (WAT-01).
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn current_events_on(&self, date: NaiveDate) -> Result<Vec<Event>> {
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM events e
             WHERE e.local_date = ?1 AND e.type <> 'entry.retracted'
               AND NOT EXISTS (SELECT 1 FROM events s WHERE s.supersedes = e.id)
             ORDER BY e.occurred_at, e.id"
        );
        query_events(&self.conn, &sql, [date_text(date)])
    }

    // --------------------------------------------------------------- summaries

    /// Insert or replace a daily summary if its revision is higher than the stored one.
    /// Returns whether it was written.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the row breaks its rules.
    pub fn upsert_daily_summary(&self, s: &DailySummary) -> Result<bool> {
        s.validate()?;
        let n = self.conn.execute(
            "INSERT INTO daily_summary (category, origin_device, local_date, revision, coverage, payload, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (category, origin_device, local_date) DO UPDATE SET
               revision = excluded.revision, coverage = excluded.coverage,
               payload = excluded.payload, updated_at = excluded.updated_at
             WHERE excluded.revision > daily_summary.revision",
            params![
                s.category,
                s.origin_device.to_string(),
                date_text(s.period),
                s.revision,
                s.coverage,
                s.payload.to_string(),
                to_ms(self.clock.now()),
            ],
        )?;
        Ok(n == 1)
    }

    /// A daily summary. `None` means no data for that day, which is not zero.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn daily_summary(
        &self,
        category: Category,
        origin: Uuid,
        date: NaiveDate,
    ) -> Result<Option<DailySummary>> {
        let row: Option<(u32, Option<f64>, String)> = self
            .conn
            .query_row(
                "SELECT revision, coverage, payload FROM daily_summary
                 WHERE category = ?1 AND origin_device = ?2 AND local_date = ?3",
                params![category, origin.to_string(), date_text(date)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(revision, coverage, payload)| {
            Ok(Summary {
                category,
                origin_device: origin,
                period: date,
                revision,
                coverage,
                payload: serde_json::from_str(&payload)?,
            })
        })
        .transpose()
    }

    /// Insert or replace a monthly summary if its revision is higher.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the row breaks its rules.
    pub fn upsert_monthly_summary(&self, s: &MonthlySummary) -> Result<bool> {
        s.validate()?;
        let n = self.conn.execute(
            "INSERT INTO monthly_summary (category, origin_device, month, revision, coverage, payload, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (category, origin_device, month) DO UPDATE SET
               revision = excluded.revision, coverage = excluded.coverage,
               payload = excluded.payload, updated_at = excluded.updated_at
             WHERE excluded.revision > monthly_summary.revision",
            params![
                s.category,
                s.origin_device.to_string(),
                s.period.text(),
                s.revision,
                s.coverage,
                s.payload.to_string(),
                to_ms(self.clock.now()),
            ],
        )?;
        Ok(n == 1)
    }

    /// The monthly summaries for a category, oldest first.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn monthly_summaries(&self, category: Category) -> Result<Vec<MonthlySummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT origin_device, month, revision, coverage, payload FROM monthly_summary
             WHERE category = ?1 ORDER BY month, origin_device",
        )?;
        let rows = stmt
            .query_map([category], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(origin, month, revision, coverage, payload)| {
                Ok(Summary {
                    category,
                    origin_device: parse_uuid(&origin)?,
                    period: MonthKey::parse(&month)?,
                    revision,
                    coverage,
                    payload: serde_json::from_str(&payload)?,
                })
            })
            .collect()
    }

    /// Store a read-only imported summary (architecture 5.4). For the same {category,
    /// origin, date}, a backup import always wins over a server restore; otherwise the
    /// higher revision wins. Returns whether it was written.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the row breaks its rules.
    pub fn upsert_imported_summary(
        &self,
        s: &DailySummary,
        provenance: ImportProvenance,
    ) -> Result<bool> {
        s.validate()?;
        let n = self.conn.execute(
            "INSERT INTO imported_summary
               (category, origin_device, local_date, revision, coverage, payload, provenance, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (category, origin_device, local_date) DO UPDATE SET
               revision = excluded.revision, coverage = excluded.coverage, payload = excluded.payload,
               provenance = excluded.provenance, imported_at = excluded.imported_at
             WHERE (excluded.provenance = 'backup_import' AND imported_summary.provenance = 'server_restore')
                OR (excluded.provenance = imported_summary.provenance
                    AND excluded.revision > imported_summary.revision)",
            params![
                s.category,
                s.origin_device.to_string(),
                date_text(s.period),
                s.revision,
                s.coverage,
                s.payload.to_string(),
                provenance,
                to_ms(self.clock.now()),
            ],
        )?;
        Ok(n == 1)
    }

    /// An imported summary and where it came from.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn imported_summary(
        &self,
        category: Category,
        origin: Uuid,
        date: NaiveDate,
    ) -> Result<Option<(DailySummary, ImportProvenance)>> {
        let row: Option<(u32, Option<f64>, String, ImportProvenance)> = self
            .conn
            .query_row(
                "SELECT revision, coverage, payload, provenance FROM imported_summary
                 WHERE category = ?1 AND origin_device = ?2 AND local_date = ?3",
                params![category, origin.to_string(), date_text(date)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(revision, coverage, payload, provenance)| {
            Ok((
                Summary {
                    category,
                    origin_device: origin,
                    period: date,
                    revision,
                    coverage,
                    payload: serde_json::from_str(&payload)?,
                },
                provenance,
            ))
        })
        .transpose()
    }

    // ------------------------------------------------- sharing (device side)

    /// The local sharing policy for a category.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn sharing_policy(&self, category: Category) -> Result<PolicyState> {
        read_policy(&self.conn, category)
    }

    /// A policy operation by id.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn policy_op(&self, op_id: Uuid) -> Result<Option<PolicyOp>> {
        load_policy_op(&self.conn, op_id)
    }

    /// Switch sharing off for a category. Always accepted and effective at once, even
    /// offline: in one transaction the policy is turned off, the category's upload queue
    /// is purged, and a pending disable operation, a consent-log entry and an event are
    /// written. Returns the operation id.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn disable_sharing(&mut self, category: Category) -> Result<Uuid> {
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let policy = read_policy(&tx, category)?;
        tx.execute(
            "UPDATE sharing_policy SET enabled = 0, updated_at = ?2 WHERE category = ?1",
            params![category, to_ms(now)],
        )?;
        let op_id = self.ids.next(now);
        tx.execute(
            "INSERT INTO policy_ops (op_id, local_seq, category, kind, base_seq, state, server_seq, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'disable', ?4, 'pending', NULL, ?5, ?5)",
            params![
                op_id.to_string(),
                next_policy_op_seq(&tx)?,
                category,
                i64::try_from(policy.seq).unwrap_or(i64::MAX),
                to_ms(now)
            ],
        )?;
        insert_consent(
            &tx,
            now,
            ConsentKind::Disabled,
            Some(category),
            None,
            Some(op_id),
        )?;
        sharing_event(&tx, &self.ids, self.device_id, now, category, false)?;
        tx.commit()?;
        Ok(op_id)
    }

    /// Ask to switch sharing on. Nothing is shared until the server accepts it
    /// ([`Store::accept_enable`]). Returns the operation id.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn request_enable(&mut self, category: Category, notice_version: &str) -> Result<Uuid> {
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let policy = read_policy(&tx, category)?;
        let op_id = self.ids.next(now);
        tx.execute(
            "INSERT INTO policy_ops (op_id, local_seq, category, kind, base_seq, state, server_seq, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'enable', ?4, 'pending', NULL, ?5, ?5)",
            params![
                op_id.to_string(),
                next_policy_op_seq(&tx)?,
                category,
                i64::try_from(policy.seq).unwrap_or(i64::MAX),
                to_ms(now)
            ],
        )?;
        insert_consent(
            &tx,
            now,
            ConsentKind::NoticeShown,
            Some(category),
            Some(notice_version),
            Some(op_id),
        )?;
        insert_consent(
            &tx,
            now,
            ConsentKind::EnableRequested,
            Some(category),
            Some(notice_version),
            Some(op_id),
        )?;
        tx.commit()?;
        Ok(op_id)
    }

    /// Record the server's acceptance of an enable.
    ///
    /// The acknowledgement is always recorded on the operation, but it changes the policy
    /// only if it is fresh:
    /// - if the user switched the category off after asking (a disable with a higher
    ///   durable [`PolicyOp::local_seq`]), the category stays off;
    /// - if a response with the same or a higher server seq was already applied, this
    ///   older response changes nothing (no `enabled`, `share_from` or `seq` rollback).
    ///
    /// # Errors
    /// [`Error::NotFound`] if the operation does not exist; [`Error::Invalid`] if it is not
    /// a pending enable or the seq is 0.
    pub fn accept_enable(
        &mut self,
        op_id: Uuid,
        server_seq: u64,
        share_from: NaiveDate,
    ) -> Result<EnableOutcome> {
        let now = self.clock.now();
        let seq = i64::try_from(server_seq).map_err(|_| Error::invalid("server seq too large"))?;
        if seq < 1 {
            return Err(Error::invalid("server seq starts at 1"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let op = load_policy_op(&tx, op_id)?
            .ok_or_else(|| Error::NotFound(format!("policy op {op_id}")))?;
        if op.kind != PolicyOpKind::Enable || op.state != PolicyOpState::Pending {
            return Err(Error::invalid("only a pending enable can be accepted"));
        }
        let policy = read_policy(&tx, op.category)?;
        tx.execute(
            "UPDATE policy_ops SET state = 'accepted', server_seq = ?2, updated_at = ?3 WHERE op_id = ?1",
            params![op_id.to_string(), seq, to_ms(now)],
        )?;
        let disabled_since: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM policy_ops
             WHERE category = ?1 AND kind = 'disable' AND local_seq > ?2)",
            params![op.category, i64::try_from(op.local_seq).unwrap_or(i64::MAX)],
            |r| r.get(0),
        )?;
        let outcome = if server_seq <= policy.seq {
            EnableOutcome::StaleResponse
        } else if disabled_since {
            EnableOutcome::SupersededByLocalDisable
        } else {
            EnableOutcome::Applied
        };
        match outcome {
            EnableOutcome::StaleResponse => {}
            EnableOutcome::SupersededByLocalDisable => {
                tx.execute(
                    "UPDATE sharing_policy SET seq = ?2, updated_at = ?3 WHERE category = ?1",
                    params![op.category, seq, to_ms(now)],
                )?;
            }
            EnableOutcome::Applied => {
                tx.execute(
                    "UPDATE sharing_policy SET enabled = 1, share_from = ?2, seq = ?3, updated_at = ?4
                     WHERE category = ?1",
                    params![op.category, date_text(share_from), seq, to_ms(now)],
                )?;
                insert_consent(
                    &tx,
                    now,
                    ConsentKind::Enabled,
                    Some(op.category),
                    None,
                    Some(op_id),
                )?;
                sharing_event(&tx, &self.ids, self.device_id, now, op.category, true)?;
            }
        }
        tx.commit()?;
        Ok(outcome)
    }

    /// Record a deletion of shared data for a date range. In one transaction the
    /// permanent tombstone and its consent-log entry are written and queued records for
    /// those dates are purged. Returns the request id.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn record_deletion(&mut self, category: Category, range: DateRange) -> Result<Uuid> {
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request_id = self.ids.next(now);
        tx.execute(
            "INSERT INTO tombstones (request_id, category, from_date, to_date, state, server_seq, created_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', NULL, ?5)",
            params![
                request_id.to_string(),
                category,
                date_text(range.from),
                date_text(range.to),
                to_ms(now)
            ],
        )?;
        insert_consent(
            &tx,
            now,
            ConsentKind::DeletionRequested,
            Some(category),
            None,
            Some(request_id),
        )?;
        tx.commit()?;
        Ok(request_id)
    }

    /// The tombstones for a category, oldest first.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn tombstones(&self, category: Category) -> Result<Vec<Tombstone>> {
        let mut stmt = self.conn.prepare(
            "SELECT request_id, from_date, to_date, state FROM tombstones
             WHERE category = ?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map([category], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, TombstoneState>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, from, to, state)| {
                Ok(Tombstone {
                    request_id: parse_uuid(&id)?,
                    category,
                    range: DateRange::new(parse_date(&from)?, parse_date(&to)?)?,
                    state,
                })
            })
            .collect()
    }

    /// Why `date` cannot be uploaded for `category`, or `None` if it can.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn upload_block(&self, category: Category, date: NaiveDate) -> Result<Option<UploadBlock>> {
        upload_block_in(&self.conn, category, date)
    }

    /// Queue this PC's summary for upload. Refused for blocked dates. A newer revision
    /// replaces queued older ones for the same date (they are deleted, never re-stamped).
    ///
    /// # Errors
    /// [`Error::UploadBlocked`] for a blocked date; [`Error::Invalid`] if the same or a
    /// newer revision is already queued or the payload is not an object.
    pub fn enqueue_upload(
        &mut self,
        category: Category,
        date: NaiveDate,
        revision: u32,
        payload: &serde_json::Value,
    ) -> Result<i64> {
        if revision == 0 || !payload.is_object() {
            return Err(Error::invalid(
                "upload needs a revision of 1 or more and an object payload",
            ));
        }
        let now = self.clock.now();
        let device = self.device_id.to_string();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(block) = upload_block_in(&tx, category, date)? {
            return Err(Error::UploadBlocked(format!("{block:?}")));
        }
        let newer: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM outbox
             WHERE category = ?1 AND origin_device = ?2 AND local_date = ?3 AND revision >= ?4)",
            params![category, device, date_text(date), revision],
            |r| r.get(0),
        )?;
        if newer {
            return Err(Error::invalid(
                "the same or a newer revision is already queued",
            ));
        }
        tx.execute(
            "DELETE FROM outbox WHERE category = ?1 AND origin_device = ?2 AND local_date = ?3",
            params![category, device, date_text(date)],
        )?;
        tx.execute(
            "INSERT INTO outbox (category, origin_device, local_date, revision, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![category, device, date_text(date), revision, payload.to_string(), to_ms(now)],
        )
        .map_err(map_upload_error)?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// The upload queue, oldest first.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn outbox(&self) -> Result<Vec<OutboxRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, category, origin_device, local_date, revision, payload FROM outbox ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Category>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, category, origin, date, revision, payload)| {
                Ok(OutboxRecord {
                    id,
                    category,
                    origin_device: parse_uuid(&origin)?,
                    local_date: parse_date(&date)?,
                    revision,
                    payload: serde_json::from_str(&payload)?,
                })
            })
            .collect()
    }

    /// The consent log, oldest first.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn consent_log(&self) -> Result<Vec<ConsentEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, kind, category, notice_version, op_ref FROM consent_log ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, ConsentKind>(1)?,
                    r.get::<_, Option<Category>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(at, kind, category, notice_version, op_ref)| {
                Ok(ConsentEntry {
                    at: from_ms(at)?,
                    kind,
                    category,
                    notice_version,
                    op_ref: op_ref.as_deref().map(parse_uuid).transpose()?,
                })
            })
            .collect()
    }

    // --------------------------------------------------------------- retention

    /// Remove rows past their retention (architecture 6.1): events, daily and imported
    /// summaries older than 90 days before `today`, and monthly summaries older than 24
    /// months. Sharing state, tombstones and the consent log are not pruned here.
    ///
    /// Event pruning is chain-safe. A correction can move an entry to another date, so an
    /// expired correction may supersede a newer-dated entry. That superseded entry (and
    /// anything it supersedes) is removed with it, early, rather than becoming current
    /// again. [`Store::current_events_on`] therefore gives the same answer for every kept
    /// date before and after pruning.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a write failure, in which case nothing changed.
    pub fn prune(&mut self, today: NaiveDate) -> Result<PruneReport> {
        let day_cutoff = date_text(
            today
                .checked_sub_days(chrono::Days::new(EVENT_RETENTION_DAYS))
                .unwrap_or(NaiveDate::MIN),
        );
        let month_cutoff = MonthKey::of(
            today
                .checked_sub_months(Months::new(MONTHLY_RETENTION_MONTHS))
                .unwrap_or(NaiveDate::MIN),
        )
        .text();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let report = PruneReport {
            // Chain-safe: an expired event takes every event it supersedes with it, however
            // far back the chain goes. Those are all superseded history, so the current view
            // never changes, and no corrected or retracted entry can become current again.
            // Events that supersede an expired one stay current (their link is cleared).
            events: tx.execute(
                "WITH RECURSIVE expired(id) AS (
                     SELECT id FROM events WHERE local_date < ?1
                     UNION
                     SELECT e.supersedes FROM events e JOIN expired x ON e.id = x.id
                     WHERE e.supersedes IS NOT NULL
                 )
                 DELETE FROM events WHERE id IN (SELECT id FROM expired)",
                [&day_cutoff],
            )?,
            daily: tx.execute(
                "DELETE FROM daily_summary WHERE local_date < ?1",
                [&day_cutoff],
            )?,
            imported: tx.execute(
                "DELETE FROM imported_summary WHERE local_date < ?1",
                [&day_cutoff],
            )?,
            monthly: tx.execute(
                "DELETE FROM monthly_summary WHERE month < ?1",
                [&month_cutoff],
            )?,
        };
        tx.commit()?;
        Ok(report)
    }
}
