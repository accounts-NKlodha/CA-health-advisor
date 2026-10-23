//! Store support for the weekly and monthly reviews (slice V1-4; see [`crate::review`]).
//!
//! The reads use the same current view of the event log as the water total
//! ([`Store::current_events_on`]): an entry that a later event corrects or removes is not
//! counted, and removals are not counted themselves. Only local dates are read; no figure
//! depends on the time of day.

use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::{
    ActivityDay, EVENT_COLUMNS, MonthKey, Store, date_text, from_ms, parse_date, query_events,
    read_zone,
};
use crate::error::{Error, Result};
use crate::review::{
    self, Calendar, DayFacts, MonthReview, PhoneEntry, PhoneWeek, WeekReview, week_start,
};
use crate::schema::{Activity, EventPayload, GoalKind, NewEvent, PhoneLabel, ReminderOutcome};

/// The longest range [`Store::day_facts`] reads at once (D: a guard, not a product limit).
pub const MAX_FACT_DAYS: i64 = 800;

impl Store {
    /// The first local date the app was used: the earliest of the store's creation day
    /// (in the current zone), the earliest tracker tally and the earliest event.
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn first_use_date(&self) -> Result<NaiveDate> {
        let zone = read_zone(&self.conn)?;
        let created: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM store_meta WHERE key = 'created_at'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let created = created
            .and_then(|t| t.parse::<i64>().ok())
            .map(from_ms)
            .transpose()?
            .map_or_else(|| self.today(), |t| Ok(zone.local_date(t)))?;
        let earliest: Option<String> = self.conn.query_row(
            "SELECT MIN(d) FROM (SELECT MIN(local_date) AS d FROM activity_day
                                 UNION ALL SELECT MIN(local_date) FROM events)",
            [],
            |r| r.get(0),
        )?;
        Ok(match earliest {
            Some(d) => parse_date(&d)?.min(created),
            None => created,
        })
    }

    /// Today, the first day of use and the workdays, for the reviews.
    ///
    /// # Errors
    /// [`Error::Sqlite`] or [`Error::Json`] on a read failure.
    pub fn review_calendar(&self) -> Result<Calendar> {
        let today = self.today()?;
        Ok(Calendar {
            today,
            first_day: self.first_use_date()?.min(today),
            workdays: self.work_schedule()?.workdays,
        })
    }

    /// The water goal history, oldest first.
    fn water_goals(&self) -> Result<Vec<(NaiveDate, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT effective_from, value FROM goals_history WHERE kind = ?1 ORDER BY effective_from",
        )?;
        let rows = stmt
            .query_map([GoalKind::WaterDailyMl], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(d, v)| Ok((parse_date(&d)?, v)))
            .collect()
    }

    /// What the reviews use about each local date from `from` to `to` (inclusive), one
    /// entry per date, in order. A date with nothing recorded has an empty entry.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the range is reversed or longer than [`MAX_FACT_DAYS`];
    /// otherwise a read failure.
    pub fn day_facts(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<DayFacts>> {
        self.day_facts_for(from, to, None)
    }

    /// [`Store::day_facts`] counting only events recorded on `origin` (the shared
    /// records: a PC never uploads another PC's entries, such as rows from an imported
    /// backup, under its own id; T-L7).
    pub(super) fn day_facts_for(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        origin: Option<uuid::Uuid>,
    ) -> Result<Vec<DayFacts>> {
        let span = (to - from).num_days();
        if !(0..MAX_FACT_DAYS).contains(&span) {
            return Err(Error::invalid(format!(
                "review range {from} to {to} must be 1 to {MAX_FACT_DAYS} days"
            )));
        }
        let mut days: BTreeMap<NaiveDate, DayFacts> = from
            .iter_days()
            .take_while(|d| *d <= to)
            .map(|d| (d, DayFacts::empty(d)))
            .collect();

        let goals = self.water_goals()?;
        for (d, f) in &mut days {
            f.water_goal_ml = goals
                .iter()
                .rev()
                .find(|(eff, _)| eff <= d)
                .map(|(_, v)| *v);
        }

        let mut stmt = self.conn.prepare(
            "SELECT local_date, pc_active_s, work_active_s, work_quiet_s, unknown_s, away_s,
                    confirmed_breaks, longest_block_s, focus_sessions, pomodoro_rounds, revision
             FROM activity_day WHERE local_date BETWEEN ?1 AND ?2",
        )?;
        let tallies = stmt
            .query_map(params![date_text(from), date_text(to)], |r| {
                let u = |v: i64| u64::try_from(v).unwrap_or(0);
                Ok((
                    r.get::<_, String>(0)?,
                    u(r.get(1)?),
                    u(r.get(2)?),
                    u(r.get(3)?),
                    u(r.get(4)?),
                    u(r.get(5)?),
                    r.get::<_, u32>(6)?,
                    u(r.get(7)?),
                    r.get::<_, u32>(8)?,
                    r.get::<_, u32>(9)?,
                    u(r.get(10)?),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for t in tallies {
            let date = parse_date(&t.0)?;
            if let Some(f) = days.get_mut(&date) {
                f.tally = Some(ActivityDay {
                    date,
                    pc_active_s: t.1,
                    work_active_s: t.2,
                    work_quiet_s: t.3,
                    unknown_s: t.4,
                    away_s: t.5,
                    confirmed_breaks: t.6,
                    longest_block_s: t.7,
                    focus_sessions: t.8,
                    pomodoro_rounds: t.9,
                    revision: t.10,
                });
            }
        }

        // The current view: entries no later event corrects or removes, without the
        // removals themselves.
        let mut stmt = self.conn.prepare(
            "SELECT e.local_date, e.type, e.payload FROM events e
             WHERE e.local_date BETWEEN ?1 AND ?2 AND e.type <> 'entry.retracted'
               AND (?3 IS NULL OR e.origin_device = ?3)
               AND NOT EXISTS (SELECT 1 FROM events s WHERE s.supersedes = e.id)",
        )?;
        let origin = origin.map(|o| o.to_string());
        let rows = stmt
            .query_map(params![date_text(from), date_text(to), origin], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (date, type_name, payload) in rows {
            let date = parse_date(&date)?;
            let Some(f) = days.get_mut(&date) else {
                continue;
            };
            f.events = f.events.saturating_add(1);
            // A type this build does not know (written by a newer build) still shows the
            // day was recorded, and is otherwise ignored.
            let Ok(payload) = EventPayload::from_stored(&type_name, &payload) else {
                continue;
            };
            match payload {
                EventPayload::WaterLogged { amount_ml } => {
                    f.water_ml = f.water_ml.saturating_add(amount_ml);
                    f.water_entries = f.water_entries.saturating_add(1);
                }
                EventPayload::MovementSession { outcome, .. } => f.note_routine(outcome),
                EventPayload::ReminderResponse {
                    activity: Activity::Movement | Activity::EyeRest,
                    outcome,
                    ..
                } if outcome != ReminderOutcome::Snoozed => {
                    f.breaks_offered = f.breaks_offered.saturating_add(1);
                    if matches!(outcome, ReminderOutcome::Done | ReminderOutcome::Started) {
                        f.breaks_taken = f.breaks_taken.saturating_add(1);
                    }
                }
                EventPayload::PhoneInterruption { label } => f.phone.add(label),
                _ => {}
            }
        }
        Ok(days.into_values().collect())
    }

    /// The phone self-reports from `from` to `to` as they stand now, oldest first.
    ///
    /// # Errors
    /// A read failure.
    pub fn phone_entries(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<PhoneEntry>> {
        let sql = format!(
            "SELECT {EVENT_COLUMNS} FROM events e
             WHERE e.local_date BETWEEN ?1 AND ?2 AND e.type = 'phone.interruption'
               AND NOT EXISTS (SELECT 1 FROM events s WHERE s.supersedes = e.id)
             ORDER BY e.occurred_at, e.id"
        );
        Ok(
            query_events(&self.conn, &sql, params![date_text(from), date_text(to)])?
                .into_iter()
                .filter_map(|e| match e.payload {
                    EventPayload::PhoneInterruption { label } => Some(PhoneEntry {
                        id: e.id,
                        date: e.local_date,
                        time: e.zone.local_datetime(e.occurred_at).time(),
                        label,
                    }),
                    _ => None,
                })
                .collect(),
        )
    }

    /// Whether the user has ever noted a phone interruption (the review hides the section
    /// otherwise; wireframe section 8).
    ///
    /// # Errors
    /// [`Error::Sqlite`] on a read failure.
    pub fn has_phone_reports(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM events WHERE type = 'phone.interruption')",
            [],
            |r| r.get(0),
        )?)
    }

    /// Change the label of a phone self-report from the weekly review (wireframe W13,
    /// DIS-01). The new label is a correction that supersedes the report, so totals
    /// recalculate and the report keeps its date and time. Returns the correction's id,
    /// or `None` if the report already has that label.
    ///
    /// # Errors
    /// [`Error::NotFound`] if there is no such event; [`Error::Invalid`] if it is not a
    /// phone report, or was already corrected or removed (change the newest version).
    pub fn relabel_phone(&mut self, entry: Uuid, label: PhoneLabel) -> Result<Option<Uuid>> {
        let event = self
            .event(entry)?
            .ok_or_else(|| Error::NotFound(format!("phone report {entry}")))?;
        let EventPayload::PhoneInterruption { label: current } = event.payload else {
            return Err(Error::invalid("only a phone report can be relabelled here"));
        };
        if current == label {
            return Ok(None);
        }
        let new = NewEvent::manual(EventPayload::PhoneInterruption { label }).superseding(entry);
        Ok(Some(self.append_event(new)?.id))
    }

    /// The weekly review of the week containing `day` (default today), kept between the
    /// earliest week with detailed events and this week, with its suggestions (this week
    /// and the previous one).
    ///
    /// # Errors
    /// A read failure.
    pub fn week_review(&self, day: Option<NaiveDate>) -> Result<WeekReview> {
        let cal = self.review_calendar()?;
        let this_week = week_start(cal.today);
        let start = week_start(day.unwrap_or(cal.today)).clamp(cal.earliest_week(), this_week);
        let end = start + Days::new(6);
        let prev_start = start.checked_sub_days(Days::new(7)).unwrap_or(start);
        let all = self.day_facts(prev_start, end)?;
        let (previous, days) = all.split_at(all.len().saturating_sub(7));
        let phone = PhoneWeek {
            show: self.has_phone_reports()?,
            counts: review::phone_counts(days),
            entries: self.phone_entries(start, end)?,
        };
        let mut review = review::week(&cal, days, previous, phone);
        self.add_suggestions(&mut review)?;
        Ok(review)
    }

    /// Monthly progress for `month` (default this month), kept between the month of
    /// first use and this month.
    ///
    /// # Errors
    /// A read failure.
    pub fn month_review(&self, month: Option<MonthKey>) -> Result<MonthReview> {
        let cal = self.review_calendar()?;
        let selected = month
            .unwrap_or_else(|| MonthKey::of(cal.today))
            .clamp(MonthKey::of(cal.first_day), MonthKey::of(cal.today));
        let ((from, to), baseline) = review::month_ranges(&cal, selected);
        let facts = self.day_facts(from, to)?;
        let baseline_facts = match baseline {
            Some((b_from, b_to)) => self.day_facts(b_from, b_to)?,
            None => Vec::new(),
        };
        Ok(review::month(&cal, selected, &facts, &baseline_facts))
    }
}
