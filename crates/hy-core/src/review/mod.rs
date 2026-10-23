//! Weekly and monthly reviews (REV-01, REV-02; wireframes W13 and W14, slice V1-4).
//!
//! Every figure comes from what the user logged or the PC recorded, one local date at a
//! time:
//!
//! - the tracker's per-day tallies (`activity_day`; a missing row is no data, never zero);
//! - the current view of the event log, after corrections and removals: water entries,
//!   movement routine results, answers to movement and eye-rest reminders, and phone
//!   self-reports.
//!
//! Only per-day tallies are used. Nothing here looks at the time of day of an activity,
//! and the review never infers anything the user did not record.
//!
//! **Missing data is never zero** (wireframe design rule 4). Each day of each metric gets a
//! [`DayMark`]: a day the PC recorded with nothing logged is [`DayMark::Zero`] ("0, none
//! recorded"); a workday with no record at all is [`DayMark::NoData`] ("no data, app not
//! running") and is never counted as a miss. Rates count only days that were recorded
//! ([`DayMark::counts`]), so a missing day changes neither the numerator nor the
//! denominator. Today counts once its goal is met or the day is over.
//!
//! [`week`] and [`month`] are pure: the store reads the facts ([`crate::store::Store`]'s
//! `day_facts`) and these functions turn them into what the review windows show.

use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate, NaiveTime};
use serde::Serialize;
use uuid::Uuid;

use crate::schema::{PhoneLabel, RoutineOutcome};
use crate::store::{ActivityDay, EVENT_RETENTION_DAYS, MonthKey};
use crate::time::WeekdaySet;

#[cfg(test)]
mod tests;

/// Months shown on the monthly chart, the selected month included (D).
pub const MONTHS_SHOWN: usize = 6;
/// Days since first use before the monthly chart appears (wireframe W14 empty variant).
pub const MONTHLY_AFTER_DAYS: i64 = 14;
/// The baseline is the user's first four weeks (wireframe W14).
pub const BASELINE_DAYS: u64 = 28;
/// At most three highlights (wireframe W13).
pub const MAX_HIGHLIGHTS: usize = 3;

/// The Monday that starts the week containing `date` (weeks run Monday to Sunday).
#[must_use]
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_monday()))
}

/// Today, the first day the app was used, and the workdays: what turns facts into marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calendar {
    /// Today's local date in the user's current zone.
    pub today: NaiveDate,
    /// The first local date with any record (or the day the store was created).
    pub first_day: NaiveDate,
    /// The user's workdays. Past weeks are judged against the current workdays (D:
    /// workday changes are not kept with dates).
    pub workdays: WeekdaySet,
}

impl Calendar {
    /// Whether `date` is one of the user's workdays.
    #[must_use]
    pub fn is_workday(&self, date: NaiveDate) -> bool {
        self.workdays.contains(date.weekday())
    }

    /// The earliest week a weekly review can show: the week of first use, and no further
    /// back than detailed events are kept (90 days).
    #[must_use]
    pub fn earliest_week(&self) -> NaiveDate {
        let kept = self
            .today
            .checked_sub_days(Days::new(EVENT_RETENTION_DAYS - 1))
            .unwrap_or(self.today);
        week_start(self.first_day).max(week_start(kept))
    }
}

/// The three activities a review reports (wireframe W13 rings and W14 metrics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Movement routine: done days out of recorded days.
    Movement,
    /// Breaks taken when offered: movement and eye-rest reminders marked done or
    /// started, out of those answered (snoozes come back, so they are not counted).
    Breaks,
    /// Water goal days: days the user's own goal was reached, out of recorded days with a
    /// goal.
    Water,
}

impl Metric {
    /// Every metric, in display order.
    pub const ALL: [Self; 3] = [Self::Movement, Self::Breaks, Self::Water];
}

/// How one day looks for one metric (wireframe W13 day strip and W14 day strip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DayMark {
    /// Done, goal met, or every offered break taken.
    Met,
    /// Partly: a partial routine, some water below the goal (or with no goal set), some
    /// offered breaks taken.
    Partly,
    /// Recorded, and nothing logged or done: "0, none recorded". A measured zero.
    Zero,
    /// Breaks only: recorded, but no break was offered that day.
    NothingDue,
    /// A workday with no record at all ("no data, app not running"). Not a miss.
    NoData,
    /// Not a workday, and nothing done.
    Off,
    /// Before the user started.
    NotStarted,
    /// Today, still in progress.
    Today,
    /// Later than today.
    Future,
}

impl DayMark {
    /// Whether the day counts in a rate: recorded, and complete (or already met).
    #[must_use]
    pub const fn counts(self) -> bool {
        matches!(self, Self::Met | Self::Partly | Self::Zero)
    }
}

/// Phone self-reports by label (DIS-01).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PhoneCounts {
    /// Work-related.
    pub work: u32,
    /// Personal.
    pub personal: u32,
    /// Unsure.
    pub unsure: u32,
    /// No label chosen.
    pub unlabelled: u32,
}

impl PhoneCounts {
    /// Count one report.
    pub fn add(&mut self, label: PhoneLabel) {
        let slot = match label {
            PhoneLabel::Work => &mut self.work,
            PhoneLabel::Personal => &mut self.personal,
            PhoneLabel::Unsure => &mut self.unsure,
            PhoneLabel::Unlabelled => &mut self.unlabelled,
        };
        *slot = slot.saturating_add(1);
    }

    /// All reports.
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.work
            .saturating_add(self.personal)
            .saturating_add(self.unsure)
            .saturating_add(self.unlabelled)
    }

    fn merge(&mut self, o: &Self) {
        self.work = self.work.saturating_add(o.work);
        self.personal = self.personal.saturating_add(o.personal);
        self.unsure = self.unsure.saturating_add(o.unsure);
        self.unlabelled = self.unlabelled.saturating_add(o.unlabelled);
    }
}

/// Everything a review uses about one local date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFacts {
    /// The local date (as recorded; a later time-zone change never moves it).
    pub date: NaiveDate,
    /// The tracker's tallies, if the PC recorded the day.
    pub tally: Option<ActivityDay>,
    /// Current events of any kind on the date (after corrections and removals).
    pub events: u32,
    /// Water logged (current entries only).
    pub water_ml: u32,
    /// Number of current water entries.
    pub water_entries: u32,
    /// The user's own water goal in force that day, if one was set.
    pub water_goal_ml: Option<u32>,
    /// The best routine result of the day (done, then partial, then none, then skipped).
    pub routine: Option<RoutineOutcome>,
    /// Routines completed.
    pub routines_done: u32,
    /// Movement and eye-rest reminders answered (snoozes excluded: they come back).
    pub breaks_offered: u32,
    /// Of those, marked done or started.
    pub breaks_taken: u32,
    /// Phone self-reports.
    pub phone: PhoneCounts,
}

const fn routine_rank(o: RoutineOutcome) -> u8 {
    match o {
        RoutineOutcome::Done => 3,
        RoutineOutcome::Partial => 2,
        RoutineOutcome::None => 1,
        RoutineOutcome::Skipped => 0,
    }
}

impl DayFacts {
    /// A day with nothing recorded.
    #[must_use]
    pub const fn empty(date: NaiveDate) -> Self {
        Self {
            date,
            tally: None,
            events: 0,
            water_ml: 0,
            water_entries: 0,
            water_goal_ml: None,
            routine: None,
            routines_done: 0,
            breaks_offered: 0,
            breaks_taken: 0,
            phone: PhoneCounts {
                work: 0,
                personal: 0,
                unsure: 0,
                unlabelled: 0,
            },
        }
    }

    /// Whether the PC recorded anything on the date: tracker tallies or any event.
    #[must_use]
    pub const fn recorded(&self) -> bool {
        self.tally.is_some() || self.events > 0
    }

    /// Note a routine result.
    pub fn note_routine(&mut self, outcome: RoutineOutcome) {
        if self
            .routine
            .is_none_or(|r| routine_rank(outcome) > routine_rank(r))
        {
            self.routine = Some(outcome);
        }
        if outcome == RoutineOutcome::Done {
            self.routines_done = self.routines_done.saturating_add(1);
        }
    }
}

/// The mark of `facts` for `metric`.
#[must_use]
pub fn mark(cal: &Calendar, facts: &DayFacts, metric: Metric) -> DayMark {
    let date = facts.date;
    if date > cal.today {
        return DayMark::Future;
    }
    let recorded = facts.recorded();
    if date < cal.first_day && !recorded {
        return DayMark::NotStarted;
    }
    let raw = recorded.then_some(match metric {
        Metric::Movement => match facts.routine {
            Some(RoutineOutcome::Done) => DayMark::Met,
            Some(RoutineOutcome::Partial) => DayMark::Partly,
            _ => DayMark::Zero,
        },
        Metric::Water => match facts.water_goal_ml {
            Some(goal) if facts.water_ml >= goal => DayMark::Met,
            _ if facts.water_ml > 0 => DayMark::Partly,
            _ => DayMark::Zero,
        },
        Metric::Breaks => {
            if facts.breaks_offered == 0 {
                DayMark::NothingDue
            } else if facts.breaks_taken >= facts.breaks_offered {
                DayMark::Met
            } else if facts.breaks_taken > 0 {
                DayMark::Partly
            } else {
                DayMark::Zero
            }
        }
    });
    if date == cal.today {
        return if raw == Some(DayMark::Met) {
            DayMark::Met
        } else {
            DayMark::Today
        };
    }
    let workday = cal.is_workday(date);
    match raw {
        None if workday => DayMark::NoData,
        None => DayMark::Off,
        Some(m @ (DayMark::Met | DayMark::Partly)) => m,
        Some(_) if !workday => DayMark::Off,
        Some(m) => m,
    }
}

/// A count out of a total, with its percentage (rounded half up) when the total is not 0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Ratio {
    /// Count.
    pub num: u32,
    /// Out of.
    pub den: u32,
    /// Percentage, `None` when there is nothing to count.
    pub pct: Option<u32>,
}

impl Ratio {
    /// `num` out of `den`.
    #[must_use]
    pub fn new(num: u32, den: u32) -> Self {
        let pct = (den > 0).then(|| {
            let v = (200 * u64::from(num) + u64::from(den)) / (2 * u64::from(den));
            u32::try_from(v).unwrap_or(u32::MAX)
        });
        Self { num, den, pct }
    }
}

/// The rate of `metric` over `days` (only days whose mark counts).
#[must_use]
pub fn rate(cal: &Calendar, days: &[DayFacts], metric: Metric) -> Ratio {
    let (mut num, mut den) = (0u32, 0u32);
    for f in days {
        let m = mark(cal, f, metric);
        if !m.counts() {
            continue;
        }
        match metric {
            Metric::Movement => {
                den += 1;
                num += u32::from(m == DayMark::Met);
            }
            Metric::Water => {
                if f.water_goal_ml.is_some() {
                    den += 1;
                    num += u32::from(m == DayMark::Met);
                }
            }
            Metric::Breaks => {
                den = den.saturating_add(f.breaks_offered);
                num = num.saturating_add(f.breaks_taken.min(f.breaks_offered));
            }
        }
    }
    Ratio::new(num, den)
}

/// Recorded workdays out of the workdays the user could have recorded (from the first day
/// of use to today) in a range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    /// Workdays with a record.
    pub recorded: u32,
    /// Workdays from first use to today in the range.
    pub workdays: u32,
}

fn coverage(cal: &Calendar, days: &[DayFacts]) -> Coverage {
    let mut c = Coverage::default();
    for f in days {
        if f.date < cal.first_day || f.date > cal.today || !cal.is_workday(f.date) {
            continue;
        }
        c.workdays += 1;
        c.recorded += u32::from(f.recorded());
    }
    c
}

/// One metric's row in the weekly review: its marks and figures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricRow {
    /// The metric.
    pub metric: Metric,
    /// One mark per day of the week, Monday first.
    pub marks: Vec<DayMark>,
    /// Movement: done days of recorded days. Water: goal days of recorded days with a
    /// goal. Breaks: breaks taken of breaks offered.
    pub value: Ratio,
    /// Days met.
    pub met: u32,
    /// Days partly.
    pub partly: u32,
    /// Days at 0 (recorded, none logged).
    pub zero: u32,
    /// Workdays with no data.
    pub no_data: u32,
    /// Breaks only: recorded days with no break offered.
    pub nothing_due: u32,
}

fn row(cal: &Calendar, days: &[DayFacts], metric: Metric) -> MetricRow {
    let marks: Vec<DayMark> = days.iter().map(|f| mark(cal, f, metric)).collect();
    let n = |m: DayMark| u32::try_from(marks.iter().filter(|x| **x == m).count()).unwrap_or(0);
    MetricRow {
        metric,
        value: rate(cal, days, metric),
        met: n(DayMark::Met),
        partly: n(DayMark::Partly),
        zero: n(DayMark::Zero),
        no_data: n(DayMark::NoData),
        nothing_due: n(DayMark::NothingDue),
        marks,
    }
}

/// Which variant of the weekly review applies (wireframe W13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeekStatus {
    /// The week ended before the first day of use: nothing to show.
    BeforeStart,
    /// The user started during this week: "Comparisons start after two full weeks".
    FirstWeek,
    /// Fewer than half of the workdays so far were recorded: "Missing days are not counted
    /// as misses", and no suggestions.
    NotEnoughData,
    /// Enough data.
    Normal,
}

/// A day of the week, for the column headings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WeekDay {
    /// The date.
    pub date: NaiveDate,
    /// One of the user's workdays.
    pub workday: bool,
}

/// The tracker's tallies over a week (estimates, this PC).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct WeekTallies {
    /// The longest work block of any day, in seconds (an estimate).
    pub longest_block_s: Option<u64>,
    /// Quiet focus sessions started.
    pub focus_sessions: u32,
    /// Pomodoro work intervals completed.
    pub pomodoro_rounds: u32,
    /// Confirmed breaks (a lock or sleep of 2 minutes or more, or a break the user
    /// marked).
    pub confirmed_breaks: u32,
}

fn tallies(days: &[DayFacts]) -> WeekTallies {
    let mut t = WeekTallies::default();
    for d in days.iter().filter_map(|f| f.tally.as_ref()) {
        if d.longest_block_s > 0 {
            t.longest_block_s = Some(t.longest_block_s.unwrap_or(0).max(d.longest_block_s));
        }
        t.focus_sessions = t.focus_sessions.saturating_add(d.focus_sessions);
        t.pomodoro_rounds = t.pomodoro_rounds.saturating_add(d.pomodoro_rounds);
        t.confirmed_breaks = t.confirmed_breaks.saturating_add(d.confirmed_breaks);
    }
    t
}

/// The previous week's figures, once comparisons are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Comparison {
    /// Movement routine days.
    pub movement: Ratio,
    /// Breaks taken.
    pub breaks: Ratio,
    /// Water goal days.
    pub water: Ratio,
    /// The longest work block (estimate).
    pub longest_block_s: Option<u64>,
}

/// One phone self-report as it stands now (after corrections).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PhoneEntry {
    /// The event that holds the report now; a new label supersedes it.
    pub id: Uuid,
    /// The local date it was noted.
    pub date: NaiveDate,
    /// The local time it was noted, in the zone it was recorded in (shown only on this PC,
    /// as on the note itself).
    pub time: NaiveTime,
    /// Its label.
    pub label: PhoneLabel,
}

/// The weekly review's phone section (self-reported, not detected).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhoneWeek {
    /// Shown only to users who have ever noted one (wireframe section 8).
    pub show: bool,
    /// This week's reports by label.
    pub counts: PhoneCounts,
    /// This week's reports, oldest first, with editable labels.
    pub entries: Vec<PhoneEntry>,
}

/// The weekly review (wireframe W13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WeekReview {
    /// Monday.
    pub start: NaiveDate,
    /// Sunday.
    pub end: NaiveDate,
    /// Today.
    pub today: NaiveDate,
    /// Which variant applies.
    pub status: WeekStatus,
    /// The seven days.
    pub days: Vec<WeekDay>,
    /// "Recorded on N of M workdays".
    pub coverage: Coverage,
    /// Movement, breaks and water.
    pub rows: Vec<MetricRow>,
    /// The tracker's tallies (estimates).
    pub tallies: WeekTallies,
    /// Up to three short highlights.
    pub highlights: Vec<String>,
    /// The previous week, once two full weeks of use allow a comparison.
    pub comparison: Option<Comparison>,
    /// Phone self-reports.
    pub phone: PhoneWeek,
    /// The Monday of the previous week, if the review can go back.
    pub previous_week: Option<NaiveDate>,
    /// The Monday of the next week, if it has started.
    pub next_week: Option<NaiveDate>,
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// "45 min" or "1 h 55 min".
#[must_use]
pub fn duration_text(secs: u64) -> String {
    let mins = secs / 60;
    match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn days_word(n: u32) -> &'static str {
    if n == 1 { "day" } else { "days" }
}

fn highlights(rows: &[MetricRow], t: &WeekTallies) -> Vec<String> {
    let get = |m: Metric| rows.iter().find(|r| r.metric == m);
    let mut out = Vec::new();
    if let Some(r) = get(Metric::Movement)
        && r.met > 0
    {
        let (num, den) = (r.value.num, r.value.den);
        let mut s = format!(
            "Movement routine done on {num} of {den} recorded {}",
            days_word(den)
        );
        if r.partly > 0 {
            s.push_str(&format!(", plus {} partly", r.partly));
        }
        out.push(s);
    }
    if let Some(r) = get(Metric::Breaks)
        && r.value.den > 0
    {
        out.push(format!(
            "Breaks taken: {} of {} offered",
            r.value.num, r.value.den
        ));
    }
    if let Some(r) = get(Metric::Water)
        && r.value.num > 0
    {
        let (num, den) = (r.value.num, r.value.den);
        out.push(format!(
            "Water goal reached on {num} of {den} {}",
            days_word(den)
        ));
    }
    if let Some(s) = t.longest_block_s {
        out.push(format!(
            "Longest work block: {} (estimate, this PC)",
            duration_text(s)
        ));
    }
    if t.focus_sessions > 0 || t.pomodoro_rounds > 0 {
        let parts: Vec<String> = [
            (t.focus_sessions, "focus session", "focus sessions"),
            (t.pomodoro_rounds, "Pomodoro round", "Pomodoro rounds"),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| plural(n, one, many))
        .collect();
        out.push(parts.join(" and "));
    }
    out.truncate(MAX_HIGHLIGHTS);
    out
}

/// Build the weekly review. `days` are the seven days from Monday; `previous` the seven
/// days of the week before (empty if not loaded); `phone` the week's phone section.
#[must_use]
pub fn week(
    cal: &Calendar,
    days: &[DayFacts],
    previous: &[DayFacts],
    phone: PhoneWeek,
) -> WeekReview {
    let start = days
        .first()
        .map_or_else(|| week_start(cal.today), |f| f.date);
    let end = start + Days::new(6);
    let rows: Vec<MetricRow> = Metric::ALL.iter().map(|m| row(cal, days, *m)).collect();
    let cov = coverage(cal, days);
    let status = if end < cal.first_day {
        WeekStatus::BeforeStart
    } else if cal.first_day >= start {
        WeekStatus::FirstWeek
    } else if cov.recorded * 2 < cov.workdays || (cov.workdays == 0 && cov.recorded == 0) {
        WeekStatus::NotEnoughData
    } else {
        WeekStatus::Normal
    };
    let t = tallies(days);
    // Two full weeks: the previous week was used from its Monday, and had enough data.
    let comparison = (previous.len() == 7
        && start
            .checked_sub_days(Days::new(7))
            .is_some_and(|p| cal.first_day <= p)
        && status == WeekStatus::Normal)
        .then(|| {
            let pc = coverage(cal, previous);
            (pc.recorded * 2 >= pc.workdays && pc.recorded > 0).then(|| Comparison {
                movement: rate(cal, previous, Metric::Movement),
                breaks: rate(cal, previous, Metric::Breaks),
                water: rate(cal, previous, Metric::Water),
                longest_block_s: tallies(previous).longest_block_s,
            })
        })
        .flatten();
    let earliest = cal.earliest_week();
    let previous_week = start
        .checked_sub_days(Days::new(7))
        .filter(|p| *p >= earliest);
    let next_week = Some(start + Days::new(7)).filter(|n| *n <= cal.today);
    WeekReview {
        start,
        end,
        today: cal.today,
        status,
        days: days
            .iter()
            .map(|f| WeekDay {
                date: f.date,
                workday: cal.is_workday(f.date),
            })
            .collect(),
        coverage: cov,
        highlights: if status == WeekStatus::BeforeStart {
            Vec::new()
        } else {
            highlights(&rows, &t)
        },
        rows,
        tallies: t,
        comparison,
        phone,
        previous_week,
        next_week,
    }
}

/// One month on the monthly chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MonthBar {
    /// The month.
    pub month: MonthKey,
    /// The month has not ended.
    pub in_progress: bool,
    /// Recorded workdays out of workdays from first use to today.
    pub coverage: Coverage,
    /// Movement routine days.
    pub movement: Ratio,
    /// Breaks taken.
    pub breaks: Ratio,
    /// Water goal days.
    pub water: Ratio,
}

/// One day of the selected month, for the day strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MonthDay {
    /// The date.
    pub date: NaiveDate,
    /// One of the user's workdays.
    pub workday: bool,
    /// Movement mark.
    pub movement: DayMark,
    /// Breaks mark.
    pub breaks: DayMark,
    /// Water mark.
    pub water: DayMark,
}

/// The user's first four weeks, the dashed baseline (wireframe W14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Baseline {
    /// First day of use.
    pub from: NaiveDate,
    /// The 28th day.
    pub to: NaiveDate,
    /// Movement routine days.
    pub movement: Ratio,
    /// Breaks taken.
    pub breaks: Ratio,
    /// Water goal days.
    pub water: Ratio,
}

/// Which variant of the monthly page applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonthStatus {
    /// Fewer than two weeks since first use: "Not enough data yet".
    TooEarly,
    /// The chart is shown.
    Normal,
}

/// Monthly progress (wireframe W14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MonthReview {
    /// The selected month.
    pub month: MonthKey,
    /// Today.
    pub today: NaiveDate,
    /// Which variant applies.
    pub status: MonthStatus,
    /// Days with a record since first use, within the months loaded.
    pub days_used: u32,
    /// Up to six months ending with the selected one, oldest first, from the month of
    /// first use.
    pub bars: Vec<MonthBar>,
    /// The first four weeks, once they are over.
    pub baseline: Option<Baseline>,
    /// The selected month, day by day.
    pub days: Vec<MonthDay>,
    /// The month before, if the review can go back.
    pub previous_month: Option<MonthKey>,
    /// The month after, if it has started.
    pub next_month: Option<MonthKey>,
}

/// The first month shown with `selected`.
#[must_use]
pub fn first_shown_month(cal: &Calendar, selected: MonthKey) -> MonthKey {
    let floor = MonthKey::of(cal.first_day);
    let mut m = selected;
    for _ in 1..MONTHS_SHOWN {
        if m <= floor {
            break;
        }
        m = m.pred();
    }
    m
}

/// The dates [`month`] needs facts for: the shown months, and the baseline range if it is
/// over (`None` otherwise).
#[must_use]
pub fn month_ranges(
    cal: &Calendar,
    selected: MonthKey,
) -> ((NaiveDate, NaiveDate), Option<(NaiveDate, NaiveDate)>) {
    let shown = (
        first_shown_month(cal, selected).first_day(),
        selected.last_day(),
    );
    let baseline_end = cal.first_day + Days::new(BASELINE_DAYS - 1);
    let baseline = (baseline_end < cal.today).then_some((cal.first_day, baseline_end));
    (shown, baseline)
}

fn slice_of<'a>(
    by_date: &'a BTreeMap<NaiveDate, &'a DayFacts>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<DayFacts> {
    from.iter_days()
        .take_while(|d| *d <= to)
        .map(|d| {
            by_date
                .get(&d)
                .map_or_else(|| DayFacts::empty(d), |f| (*f).clone())
        })
        .collect()
}

/// Build monthly progress for `selected`. `facts` cover the shown months and
/// `baseline_facts` the baseline range (see [`month_ranges`]); missing dates count as days
/// with no record.
#[must_use]
pub fn month(
    cal: &Calendar,
    selected: MonthKey,
    facts: &[DayFacts],
    baseline_facts: &[DayFacts],
) -> MonthReview {
    let by_date: BTreeMap<NaiveDate, &DayFacts> = facts.iter().map(|f| (f.date, f)).collect();
    let mut bars = Vec::new();
    let mut m = first_shown_month(cal, selected);
    loop {
        let days = slice_of(&by_date, m.first_day(), m.last_day());
        bars.push(MonthBar {
            month: m,
            in_progress: m.last_day() >= cal.today,
            coverage: coverage(cal, &days),
            movement: rate(cal, &days, Metric::Movement),
            breaks: rate(cal, &days, Metric::Breaks),
            water: rate(cal, &days, Metric::Water),
        });
        if m >= selected {
            break;
        }
        m = m.succ();
    }
    let month_days = slice_of(&by_date, selected.first_day(), selected.last_day());
    let days: Vec<MonthDay> = month_days
        .iter()
        .map(|f| MonthDay {
            date: f.date,
            workday: cal.is_workday(f.date),
            movement: mark(cal, f, Metric::Movement),
            breaks: mark(cal, f, Metric::Breaks),
            water: mark(cal, f, Metric::Water),
        })
        .collect();
    let days_used = u32::try_from(
        facts
            .iter()
            .filter(|f| f.date >= cal.first_day && f.date <= cal.today && f.recorded())
            .count(),
    )
    .unwrap_or(u32::MAX);
    let (_, baseline_range) = month_ranges(cal, selected);
    let baseline = baseline_range.map(|(from, to)| {
        let b: BTreeMap<NaiveDate, &DayFacts> =
            baseline_facts.iter().map(|f| (f.date, f)).collect();
        let days = slice_of(&b, from, to);
        Baseline {
            from,
            to,
            movement: rate(cal, &days, Metric::Movement),
            breaks: rate(cal, &days, Metric::Breaks),
            water: rate(cal, &days, Metric::Water),
        }
    });
    let status = if (cal.today - cal.first_day).num_days() < MONTHLY_AFTER_DAYS {
        MonthStatus::TooEarly
    } else {
        MonthStatus::Normal
    };
    MonthReview {
        month: selected,
        today: cal.today,
        status,
        days_used,
        bars,
        baseline,
        days,
        previous_month: Some(selected.pred())
            .filter(|p| *p < selected && *p >= MonthKey::of(cal.first_day)),
        next_month: Some(selected.succ())
            .filter(|n| *n > selected && *n <= MonthKey::of(cal.today)),
    }
}

/// Merge phone counts over days.
#[must_use]
pub fn phone_counts(days: &[DayFacts]) -> PhoneCounts {
    let mut c = PhoneCounts::default();
    for f in days {
        c.merge(&f.phone);
    }
    c
}
