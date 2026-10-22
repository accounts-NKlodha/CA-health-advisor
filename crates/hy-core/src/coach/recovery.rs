//! The deadline day and the recovery-day plan (REC-01; wireframe B7, W15 and flow 4,
//! accepted by the owner on 21 Oct 2026).
//!
//! - A long active day (optional, 9 hours of estimated PC activity by default) or the
//!   pet menu's "Deadline day" asks whether today was deadline-heavy. Only the user
//!   decides; hours of PC activity are an estimate and never a sign of stress or
//!   tiredness, and the app infers neither.
//! - "Yes" marks the day and invites a gentler recovery day: tomorrow, another date, later
//!   or no thanks. It is a personal routine, not a leave request.
//! - One pending [`RecoveryPlan`] is kept. On its date the morning card lists the plan,
//!   short breaks come a little more often (if chosen), an optional walk is suggested at
//!   the chosen time, and one end-of-day question is asked. Water stays at the user's own
//!   goal and nothing extra is owed.
//!
//! Check-in answers, plan changes and the reflection are events that stay on this PC
//! (architecture 6.2: recovery plans and reflections are never shared in V1 or V2).

use std::time::Duration;

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::time::{UserZone, WorkSchedule};

/// The optional walk can repeat every this long (wireframe W15).
pub const WALK_REPEAT_EVERY: Duration = Duration::from_secs(90 * 60);
/// A walk suggestion is offered only within this long after its time; one that falls in
/// a quiet period or a busy moment is skipped, never replayed (D).
pub const WALK_WINDOW: Duration = Duration::from_secs(10 * 60);
/// A recovery card left unanswered this long closes quietly, recording nothing (D).
pub const CARD_CLOSES_AFTER: Duration = Duration::from_secs(10 * 60);
/// How far ahead a recovery day can be planned, in days (D).
pub const PLAN_AHEAD_DAYS: i64 = 30;
/// Dates offered by "Choose a date" after the first suggestion (prototype: four).
pub const DATE_CHOICES: usize = 4;
/// Default walk time when it falls inside the work hours (prototype: 16:00).
pub const DEFAULT_WALK: (u32, u32) = (16, 0);

/// The one pending recovery-day plan (wireframe W15). Each item can be switched off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPlan {
    /// The recovery day.
    pub date: NaiveDate,
    /// Short movement and screen-free breaks offered a little more often.
    pub more_breaks: bool,
    /// An optional, comfortable 10 to 15 minute walk at this local time.
    pub walk_at: Option<NaiveTime>,
    /// Repeat the walk suggestion every 90 minutes.
    pub walk_repeat: bool,
    /// Lunch away from the screen, if practical (listed only).
    pub lunch_away: bool,
    /// One brief calming pause (listed only).
    pub calm_pause: bool,
    /// Finish work around this local time (listed only; the reflection follows it).
    pub finish_at: Option<NaiveTime>,
    /// One end-of-day question: "Did this routine feel helpful?"
    pub reflection: bool,
}

impl RecoveryPlan {
    /// The prototype's starting plan for `date`: every item on, the walk at 16:00 (or the
    /// middle of the work hours if 16:00 is outside them) and the finish at the end of the
    /// work hours. The user can change or cancel it.
    #[must_use]
    pub fn default_for(date: NaiveDate, schedule: &WorkSchedule, zone: &UserZone) -> Self {
        let window = schedule.window_for(date, zone);
        let local = |t| zone.local_datetime(t).time();
        let (walk_at, finish_at) = match window {
            Some(w) => {
                let (start, end) = (local(w.start), local(w.end));
                let walk = NaiveTime::from_hms_opt(DEFAULT_WALK.0, DEFAULT_WALK.1, 0)
                    .filter(|t| *t > start && *t < end)
                    .unwrap_or_else(|| local(w.start + (w.end - w.start) / 2));
                (Some(walk), Some(end))
            }
            None => (None, None),
        };
        Self {
            date,
            more_breaks: true,
            walk_at,
            walk_repeat: false,
            lunch_away: true,
            calm_pause: true,
            finish_at,
            reflection: true,
        }
    }

    /// Check the plan against today's date.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the date is not after today and within
    /// [`PLAN_AHEAD_DAYS`], or a repeating walk has no time.
    pub fn validate(&self, today: NaiveDate) -> Result<()> {
        let ahead = (self.date - today).num_days();
        if !(1..=PLAN_AHEAD_DAYS).contains(&ahead) {
            return Err(Error::invalid(format!(
                "a recovery day is planned 1 to {PLAN_AHEAD_DAYS} days ahead"
            )));
        }
        if self.walk_repeat && self.walk_at.is_none() {
            return Err(Error::invalid("a repeating walk needs a time"));
        }
        Ok(())
    }
}

/// The answer to "Did this routine feel helpful?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reflection {
    /// Yes.
    Yes,
    /// Somewhat.
    Somewhat,
    /// No.
    No,
    /// Skip (nothing is recorded).
    Skip,
}

/// What the user chose on a recovery card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Plan the recovery day on this date (the starting plan; one pending plan is kept).
    PlanFor(NaiveDate),
    /// "Choose a date" (or "Reschedule" on the planned day): show the next few working
    /// days.
    ChooseDate,
    /// "Later": close; "Deadline day" on the pet menu plans it later.
    Later,
    /// "No thanks": close; nothing changes.
    NoThanks,
    /// "OK" or "Sounds good": close the card.
    Ok,
    /// Save an edited plan (the plan screen).
    SavePlan(RecoveryPlan),
    /// "Cancel plan" or "Cancel for today".
    CancelPlan,
    /// "Going now" on the walk suggestion.
    WalkNow,
    /// "Not now" on the walk suggestion.
    WalkNotNow,
    /// The end-of-day reflection.
    Reflect(Reflection),
    /// "Recovery day" on the pet menu: show the planned day (the morning card on the day).
    ShowPlan,
}

/// The recovery state for the pet menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryView {
    /// Today's local date.
    pub today: NaiveDate,
    /// Today is marked as deadline-heavy.
    pub marked_today: bool,
    /// The one pending plan (its date is today or later).
    pub plan: Option<RecoveryPlan>,
    /// Today is the planned recovery day.
    pub recovery_day: bool,
    /// "Deadline day" on the pet menu would open a card now.
    pub can_open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn default_plan_matches_the_prototype_for_default_hours() {
        let p = RecoveryPlan::default_for(date(21), &WorkSchedule::default(), &UserZone::KOLKATA);
        assert_eq!(p.walk_at, NaiveTime::from_hms_opt(16, 0, 0));
        assert_eq!(p.finish_at, NaiveTime::from_hms_opt(18, 30, 0));
        assert!(p.more_breaks && p.lunch_away && p.calm_pause && p.reflection);
        assert!(!p.walk_repeat);
    }

    #[test]
    fn default_walk_moves_inside_short_hours_and_off_days_have_no_times() {
        let s = WorkSchedule {
            start: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            ..WorkSchedule::default()
        };
        let p = RecoveryPlan::default_for(date(21), &s, &UserZone::KOLKATA);
        assert_eq!(p.walk_at, NaiveTime::from_hms_opt(11, 0, 0));
        assert_eq!(p.finish_at, NaiveTime::from_hms_opt(14, 0, 0));
        // 25 Oct 2026 is a Sunday, outside the default Monday to Saturday week.
        let sunday =
            RecoveryPlan::default_for(date(25), &WorkSchedule::default(), &UserZone::KOLKATA);
        assert_eq!((sunday.walk_at, sunday.finish_at), (None, None));
    }

    #[test]
    fn plan_dates_are_after_today_and_within_thirty_days() {
        let mut p =
            RecoveryPlan::default_for(date(21), &WorkSchedule::default(), &UserZone::KOLKATA);
        assert!(p.validate(date(20)).is_ok());
        assert!(p.validate(date(21)).is_err());
        assert!(p.validate(date(22)).is_err());
        p.date = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        assert!(p.validate(date(20)).is_ok());
        p.date = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert!(p.validate(date(20)).is_err());
        p.date = date(21);
        p.walk_at = None;
        p.walk_repeat = true;
        assert!(p.validate(date(20)).is_err());
    }

    #[test]
    fn plan_round_trips_and_refuses_unknown_fields() {
        let p = RecoveryPlan::default_for(date(21), &WorkSchedule::default(), &UserZone::KOLKATA);
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<RecoveryPlan>(&text).unwrap(), p);
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["leave"] = serde_json::json!(true);
        assert!(serde_json::from_value::<RecoveryPlan>(v).is_err());
    }
}
