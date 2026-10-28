//! Store support for the coaching engine (slice V1-2, migration 0002).
//!
//! An engine step is committed as one transaction: the new engine state (compare-and-set
//! on its revision), the events the step produced and the changed per-day tallies. The
//! revision, not the wall clock or event identifiers, is the durable order of steps, so a
//! writer holding an older state (a stale second instance, a step computed before a
//! restart) is refused with [`Error::Stale`] and nothing is written.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{
    EVENT_COLUMNS, EventInsert, Store, date_text, from_ms, insert_event, query_events, read_zone,
    to_ms,
};
use crate::coach::DayTally;
use crate::coach::water::WaterEntry;
use crate::error::{Error, Result};
use crate::schema::{EventPayload, FUTURE_TOLERANCE, NewEvent};

/// The saved engine state row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCoachState {
    /// Durable step counter, from 1.
    pub revision: u64,
    /// Layout version of `state`.
    pub format: u32,
    /// Engine time at the save, in milliseconds.
    pub engine_time_ms: u64,
    /// Wall time of the save.
    pub saved_at: DateTime<Utc>,
    /// The serialised engine state (JSON object).
    pub state: String,
}

/// One engine step to commit.
#[derive(Debug, Clone, Copy)]
pub struct CoachCommit<'a> {
    /// The revision the step was computed from; `None` for the very first save.
    pub expected_revision: Option<u64>,
    /// Layout version of `state`.
    pub format: u32,
    /// Engine time after the step, in milliseconds. Never decreases.
    pub engine_time_ms: u64,
    /// The serialised engine state after the step.
    pub state: &'a str,
    /// Events the step produced, appended in order.
    pub events: &'a [NewEvent],
    /// Per-day tallies the step changed.
    pub days: &'a [(NaiveDate, DayTally)],
}

/// One day's tracker tallies as stored (whole seconds; all estimates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityDay {
    /// The local date.
    pub date: NaiveDate,
    /// Active or quiet PC time at any hour.
    pub pc_active_s: u64,
    /// Active time in work hours.
    pub work_active_s: u64,
    /// Quiet time in work hours counted as work (estimated).
    pub work_quiet_s: u64,
    /// Long quiet in work hours (unknown).
    pub unknown_s: u64,
    /// Locked or asleep.
    pub away_s: u64,
    /// Confirmed breaks.
    pub confirmed_breaks: u32,
    /// Longest work block (estimate).
    pub longest_block_s: u64,
    /// Focus sessions started.
    pub focus_sessions: u32,
    /// Pomodoro work intervals completed.
    pub pomodoro_rounds: u32,
    /// Screen time: at the PC at any hour (DEC13). `None` for rows from before v7.
    pub screen_s: Option<u64>,
    /// Revision of the row.
    pub revision: u64,
}

fn secs(ms: u64) -> i64 {
    i64::try_from(ms / 1000).unwrap_or(i64::MAX)
}

fn unsigned(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl Store {
    /// The saved coaching-engine state, if any.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn coach_state(&self) -> Result<Option<SavedCoachState>> {
        let row: Option<(i64, u32, i64, i64, String)> = self
            .conn
            .query_row(
                "SELECT revision, format, engine_time_ms, saved_at, state FROM coach_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        row.map(|(revision, format, engine_time_ms, saved_at, state)| {
            Ok(SavedCoachState {
                revision: unsigned(revision),
                format,
                engine_time_ms: unsigned(engine_time_ms),
                saved_at: from_ms(saved_at)?,
                state,
            })
        })
        .transpose()
    }

    /// Commit one engine step atomically. Returns the new revision.
    ///
    /// # Errors
    /// - [`Error::Stale`] if the stored revision is not `expected_revision` (nothing is
    ///   written);
    /// - [`Error::Invalid`] if an event breaks a rule (nothing is written);
    /// - [`Error::Sqlite`] if a constraint refuses a row, for example a second response
    ///   to the same reminder.
    pub fn commit_coach_step(&mut self, c: CoachCommit<'_>) -> Result<u64> {
        for e in c.events {
            e.validate()?;
            if e.supersedes.is_some() {
                return Err(Error::invalid(
                    "engine events never correct or retract another event",
                ));
            }
        }
        let now = self.clock.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<i64> = tx
            .query_row("SELECT revision FROM coach_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let stored = stored.map(unsigned);
        if stored != c.expected_revision {
            return Err(Error::Stale(format!(
                "coach state is at revision {stored:?}, the step was computed from {:?}",
                c.expected_revision
            )));
        }
        let revision = c.expected_revision.map_or(1, |r| r + 1);
        let engine_time = i64::try_from(c.engine_time_ms)
            .map_err(|_| Error::invalid("engine time out of range"))?;
        let rev_sql =
            i64::try_from(revision).map_err(|_| Error::invalid("revision out of range"))?;
        if c.expected_revision.is_none() {
            tx.execute(
                "INSERT INTO coach_state (id, revision, format, engine_time_ms, saved_at, state)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5)",
                params![rev_sql, c.format, engine_time, to_ms(now), c.state],
            )?;
        } else {
            tx.execute(
                "UPDATE coach_state SET revision = ?1, format = ?2, engine_time_ms = ?3,
                    saved_at = ?4, state = ?5 WHERE id = 1",
                params![rev_sql, c.format, engine_time, to_ms(now), c.state],
            )?;
        }
        let zone = read_zone(&tx)?;
        for e in c.events {
            let occurred_at = e.occurred_at.unwrap_or(now);
            if occurred_at > now + FUTURE_TOLERANCE {
                return Err(Error::invalid("event time is in the future"));
            }
            insert_event(
                &tx,
                EventInsert {
                    id: self.ids.next(now),
                    new: e,
                    occurred_at,
                    recorded_at: now,
                    zone,
                    origin_device: self.device_id,
                },
            )?;
        }
        for (date, t) in c.days {
            tx.execute(
                "INSERT INTO activity_day (local_date, pc_active_s, work_active_s, work_quiet_s,
                    unknown_s, away_s, confirmed_breaks, longest_block_s, focus_sessions,
                    pomodoro_rounds, screen_s, revision, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?12, 1, ?11)
                 ON CONFLICT (local_date) DO UPDATE SET
                    pc_active_s = excluded.pc_active_s, work_active_s = excluded.work_active_s,
                    work_quiet_s = excluded.work_quiet_s, unknown_s = excluded.unknown_s,
                    away_s = excluded.away_s, confirmed_breaks = excluded.confirmed_breaks,
                    longest_block_s = excluded.longest_block_s,
                    focus_sessions = excluded.focus_sessions,
                    pomodoro_rounds = excluded.pomodoro_rounds, screen_s = excluded.screen_s,
                    revision = activity_day.revision + 1, updated_at = excluded.updated_at",
                params![
                    date_text(*date),
                    secs(t.pc_active_ms),
                    secs(t.work_active_ms),
                    secs(t.work_quiet_ms),
                    secs(t.unknown_ms),
                    secs(t.away_ms),
                    t.confirmed_breaks,
                    secs(t.longest_block_ms),
                    t.focus_sessions,
                    t.pomodoro_rounds,
                    to_ms(now),
                    secs(t.screen_ms),
                ],
            )?;
        }
        tx.commit()?;
        Ok(revision)
    }

    /// The tracker tallies stored for `date`. `None` means no data, never zero.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn activity_day(&self, date: NaiveDate) -> Result<Option<ActivityDay>> {
        Ok(self
            .conn
            .query_row(
                "SELECT pc_active_s, work_active_s, work_quiet_s, unknown_s, away_s,
                    confirmed_breaks, longest_block_s, focus_sessions, pomodoro_rounds, revision,
                    screen_s
                 FROM activity_day WHERE local_date = ?1",
                [date_text(date)],
                |r| {
                    Ok(ActivityDay {
                        date,
                        pc_active_s: unsigned(r.get(0)?),
                        work_active_s: unsigned(r.get(1)?),
                        work_quiet_s: unsigned(r.get(2)?),
                        unknown_s: unsigned(r.get(3)?),
                        away_s: unsigned(r.get(4)?),
                        confirmed_breaks: r.get(5)?,
                        longest_block_s: unsigned(r.get(6)?),
                        focus_sessions: r.get(7)?,
                        pomodoro_rounds: r.get(8)?,
                        revision: unsigned(r.get(9)?),
                        screen_s: r.get::<_, Option<i64>>(10)?.map(unsigned),
                    })
                },
            )
            .optional()?)
    }

    /// Total water logged on `date`: current entries only, after corrections and undo
    /// (the same view as [`Store::current_events_on`]).
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn water_total_on(&self, date: NaiveDate) -> Result<u32> {
        let total: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(json_extract(e.payload, '$.amount_ml')), 0) FROM events e
             WHERE e.local_date = ?1 AND e.type = 'water.logged'
               AND NOT EXISTS (SELECT 1 FROM events s WHERE s.supersedes = e.id)",
            params![date_text(date)],
            |r| r.get(0),
        )?;
        Ok(u32::try_from(total).unwrap_or(u32::MAX))
    }

    /// Today's water entries on `date` as they stand (after corrections; removed entries
    /// are left out), oldest first, each with the local time of the zone it was recorded
    /// in. The same view as [`Store::water_total_on`].
    ///
    /// # Errors
    /// [`Error::Sqlite`] or [`Error::Json`] on a read failure.
    pub fn water_entries_on(&self, date: NaiveDate) -> Result<Vec<WaterEntry>> {
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM events e
             WHERE e.local_date = ?1 AND e.type = 'water.logged'
               AND NOT EXISTS (SELECT 1 FROM events s WHERE s.supersedes = e.id)
             ORDER BY e.occurred_at, e.id"
        );
        Ok(query_events(&self.conn, &sql, [date_text(date)])?
            .into_iter()
            .filter_map(|e| match e.payload {
                EventPayload::WaterLogged { amount_ml } => Some(WaterEntry {
                    id: e.id,
                    time: e.zone.local_datetime(e.occurred_at).time(),
                    amount_ml,
                }),
                _ => None,
            })
            .collect())
    }
}
