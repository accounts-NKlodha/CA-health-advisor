//! The user's time zone, local days and work hours.
//!
//! - Local dates use the user's IANA zone, initially `Asia/Kolkata` (PRD section 8). The
//!   IANA database is compiled in (`chrono-tz`), so nothing depends on the Windows zone
//!   registry or on network access.
//! - A local day is the half-open interval `[day_start(d), day_start(d + 1))`. It can be
//!   23, 24 or 25 hours long, or shorter or longer by other amounts (Lord Howe's 30-minute
//!   shift), and in rare historical cases zero (Samoa skipped 30 Dec 2011).
//! - Sessions that cross midnight are split per local day ([`UserZone::split_by_local_day`];
//!   architecture 4.3, test T-R6).
//! - Local wall times that do not exist or occur twice because of a DST change are resolved
//!   with the "compatible" rule used by RFC 5545 and JavaScript Temporal: a time inside a
//!   gap moves forward by the length of the gap, and a repeated time means its first
//!   occurrence. India has no DST; the rule matters only if a user picks another zone.
//! - Work hours ([`WorkSchedule`]) are a start and end wall time on chosen workdays
//!   (default Monday to Saturday, 09:30 to 18:30, as in the approved prototype). An end at
//!   or before the start means the window runs past midnight and belongs to the day it
//!   starts on.

use std::fmt;
use std::str::FromStr;

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset,
    TimeZone, Timelike, Utc, Weekday,
};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// The default zone for every user (PRD section 8).
pub const DEFAULT_TIME_ZONE: &str = "Asia/Kolkata";

/// The longest DST gap the gap resolver searches across. The largest in the IANA
/// database is Samoa's skipped day in 2011 (24 h).
const MAX_GAP_SEARCH_MINUTES: i64 = 48 * 60;

/// A validated IANA time zone chosen by the user.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserZone(Tz);

impl UserZone {
    /// `Asia/Kolkata`, the default.
    pub const KOLKATA: Self = Self(Tz::Asia__Kolkata);

    /// Parse an IANA zone name such as `Asia/Kolkata`.
    ///
    /// # Errors
    /// [`Error::InvalidTimeZone`] if the name is not in the bundled IANA database.
    pub fn parse(name: &str) -> Result<Self> {
        name.parse::<Tz>()
            .map(Self)
            .map_err(|_| Error::InvalidTimeZone(name.to_owned()))
    }

    /// The IANA name, for example `Asia/Kolkata`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.0.name()
    }

    /// Every zone name in the bundled IANA database, sorted (for the time zone choice in
    /// Settings, wireframe W5).
    #[must_use]
    pub fn names() -> Vec<&'static str> {
        let mut names: Vec<&'static str> =
            chrono_tz::TZ_VARIANTS.iter().map(|z| z.name()).collect();
        names.sort_unstable();
        names
    }

    /// The underlying `chrono-tz` zone.
    #[must_use]
    pub fn tz(&self) -> Tz {
        self.0
    }

    /// The local wall time of instant `t`.
    #[must_use]
    pub fn local_datetime(&self, t: DateTime<Utc>) -> NaiveDateTime {
        t.with_timezone(&self.0).naive_local()
    }

    /// The local date of instant `t`.
    #[must_use]
    pub fn local_date(&self, t: DateTime<Utc>) -> NaiveDate {
        self.local_datetime(t).date()
    }

    /// The instant at which the local wall time reads `local`, using the compatible rule
    /// for DST gaps (move forward) and folds (first occurrence).
    #[must_use]
    pub fn resolve_local(&self, local: NaiveDateTime) -> DateTime<Utc> {
        match self.0.from_local_datetime(&local) {
            LocalResult::Single(t) => t.with_timezone(&Utc),
            LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
            LocalResult::None => self.resolve_gap(local),
        }
    }

    /// Every instant at which the local wall time reads `local` (empty inside a gap, two
    /// inside a fold).
    fn occurrences(&self, local: NaiveDateTime) -> Vec<DateTime<Utc>> {
        match self.0.from_local_datetime(&local) {
            LocalResult::Single(t) => vec![t.with_timezone(&Utc)],
            LocalResult::Ambiguous(a, b) => vec![a.with_timezone(&Utc), b.with_timezone(&Utc)],
            LocalResult::None => Vec::new(),
        }
    }

    /// Interpret a local time inside a DST gap with the offset in force before the gap.
    fn resolve_gap(&self, local: NaiveDateTime) -> DateTime<Utc> {
        let mut probe = local;
        for _ in 0..MAX_GAP_SEARCH_MINUTES {
            probe += Duration::minutes(1);
            if let Some(after) = self.0.from_local_datetime(&probe).earliest() {
                // `local - offset_after` is an instant shortly before the transition, so the
                // offset there is the one in force before the gap.
                let offset_after = i64::from(after.offset().fix().local_minus_utc());
                let before_gap = local - Duration::seconds(offset_after);
                let offset_before = i64::from(
                    self.0
                        .offset_from_utc_datetime(&before_gap)
                        .fix()
                        .local_minus_utc(),
                );
                return (local - Duration::seconds(offset_before)).and_utc();
            }
        }
        // Not reachable with the IANA database; keep a total function regardless.
        local.and_utc()
    }

    /// The first instant of local date `date`.
    #[must_use]
    pub fn day_start(&self, date: NaiveDate) -> DateTime<Utc> {
        self.resolve_local(date.and_time(NaiveTime::MIN))
    }

    /// The half-open UTC interval covered by local date `date`.
    #[must_use]
    pub fn day_bounds(&self, date: NaiveDate) -> DayBounds {
        let start = self.day_start(date);
        let end = date
            .succ_opt()
            .map_or(DateTime::<Utc>::MAX_UTC, |next| self.day_start(next));
        DayBounds { date, start, end }
    }

    /// The first local midnight strictly after `t`.
    fn next_midnight_after(&self, t: DateTime<Utc>) -> DateTime<Utc> {
        let mut date = self.local_date(t);
        for _ in 0..4 {
            let Some(next) = date.succ_opt() else { break };
            date = next;
            let midnight = date.and_time(NaiveTime::MIN);
            let mut candidates = self.occurrences(midnight);
            if candidates.is_empty() {
                candidates.push(self.resolve_gap(midnight));
            }
            if let Some(found) = candidates.into_iter().find(|c| *c > t) {
                return found;
            }
        }
        // Not reachable with the IANA database; guarantees progress regardless.
        t + Duration::days(1)
    }

    /// Split `[start, end)` into consecutive pieces that each lie within one local date.
    /// Returns nothing if `end <= start`.
    #[must_use]
    pub fn split_by_local_day(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<DaySegment> {
        let mut out = Vec::new();
        let mut cursor = start;
        while cursor < end {
            let date = self.local_date(cursor);
            let seg_end = self.next_midnight_after(cursor).min(end);
            out.push(DaySegment {
                date,
                start: cursor,
                end: seg_end,
            });
            cursor = seg_end;
        }
        out
    }
}

impl Default for UserZone {
    fn default() -> Self {
        Self::KOLKATA
    }
}

impl fmt::Debug for UserZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UserZone({})", self.name())
    }
}

impl fmt::Display for UserZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for UserZone {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl Serialize for UserZone {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for UserZone {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Self::parse(&name).map_err(serde::de::Error::custom)
    }
}

/// The UTC interval of one local date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayBounds {
    /// The local date.
    pub date: NaiveDate,
    /// First instant of the date (inclusive).
    pub start: DateTime<Utc>,
    /// First instant of the next date (exclusive).
    pub end: DateTime<Utc>,
}

impl DayBounds {
    /// Length of the local day: usually 24 h, but 23 h or 25 h on DST days.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    /// Whether instant `t` falls on this local date.
    #[must_use]
    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.start <= t && t < self.end
    }
}

/// One local-day piece of a longer interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySegment {
    /// The local date of every instant in the piece.
    pub date: NaiveDate,
    /// Start (inclusive).
    pub start: DateTime<Utc>,
    /// End (exclusive).
    pub end: DateTime<Utc>,
}

impl DaySegment {
    /// Length of the piece.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.end - self.start
    }
}

const WEEKDAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// A set of weekdays, serialised as a list such as `["mon","tue"]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WeekdaySet(u8);

impl WeekdaySet {
    /// No days.
    pub const EMPTY: Self = Self(0);
    /// Monday to Saturday, the default workweek (Sunday optional).
    pub const MON_TO_SAT: Self = Self(0b0011_1111);
    /// Every day.
    pub const ALL: Self = Self(0b0111_1111);

    fn bit(day: Weekday) -> u8 {
        1 << day.num_days_from_monday()
    }

    /// Whether `day` is in the set.
    #[must_use]
    pub fn contains(self, day: Weekday) -> bool {
        self.0 & Self::bit(day) != 0
    }

    /// The set with `day` added.
    #[must_use]
    pub fn with(self, day: Weekday) -> Self {
        Self(self.0 | Self::bit(day))
    }

    /// The set with `day` removed.
    #[must_use]
    pub fn without(self, day: Weekday) -> Self {
        Self(self.0 & !Self::bit(day))
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The days in the set, Monday first.
    pub fn iter(self) -> impl Iterator<Item = Weekday> {
        let mut day = Weekday::Mon;
        (0..7).filter_map(move |_| {
            let current = day;
            day = day.succ();
            self.contains(current).then_some(current)
        })
    }
}

impl FromIterator<Weekday> for WeekdaySet {
    fn from_iter<I: IntoIterator<Item = Weekday>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

impl fmt::Debug for WeekdaySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(
                self.iter()
                    .map(|d| WEEKDAY_NAMES[d.num_days_from_monday() as usize]),
            )
            .finish()
    }
}

/// Serde for one weekday as its short lowercase name (`"sat"`).
pub(crate) mod weekday_name {
    use chrono::Weekday;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::WEEKDAY_NAMES;

    pub fn serialize<S: Serializer>(d: &Weekday, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(WEEKDAY_NAMES[d.num_days_from_monday() as usize])
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Weekday, D::Error> {
        let name = String::deserialize(d)?;
        let index = WEEKDAY_NAMES
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown weekday {name:?}")))?;
        Weekday::try_from(u8::try_from(index).unwrap_or(0))
            .map_err(|_| serde::de::Error::custom("bad weekday index"))
    }
}

impl Serialize for WeekdaySet {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_seq(
            self.iter()
                .map(|d| WEEKDAY_NAMES[d.num_days_from_monday() as usize]),
        )
    }
}

impl<'de> Deserialize<'de> for WeekdaySet {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(d)?;
        let mut set = Self::EMPTY;
        for name in names {
            let index = WEEKDAY_NAMES
                .iter()
                .position(|n| *n == name)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown weekday {name:?}")))?;
            let day = Weekday::try_from(u8::try_from(index).unwrap_or(0))
                .map_err(|_| serde::de::Error::custom("bad weekday index"))?;
            if set.contains(day) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate weekday {name:?}"
                )));
            }
            set = set.with(day);
        }
        Ok(set)
    }
}

/// Serde for a minute-precision local time as `"HH:MM"`.
pub(crate) mod hhmm {
    use chrono::{NaiveTime, Timelike};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{:02}:{:02}", t.hour(), t.minute()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() != 5 {
            return Err(serde::de::Error::custom(format!(
                "time {text:?} must be HH:MM"
            )));
        }
        NaiveTime::parse_from_str(&text, "%H:%M")
            .map_err(|e| serde::de::Error::custom(format!("time {text:?}: {e}")))
    }
}

/// The user's work hours (RHY-01, WAT-02; wireframes W1 step 4 and W5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkSchedule {
    /// Days with a work window (default Monday to Saturday).
    pub workdays: WeekdaySet,
    /// Local start time, minute precision.
    #[serde(with = "hhmm")]
    pub start: NaiveTime,
    /// Local end time, minute precision. At or before `start` means the window ends on
    /// the next day.
    #[serde(with = "hhmm")]
    pub end: NaiveTime,
}

impl Default for WorkSchedule {
    fn default() -> Self {
        Self {
            workdays: WeekdaySet::MON_TO_SAT,
            start: NaiveTime::from_hms_opt(9, 30, 0).unwrap_or(NaiveTime::MIN),
            end: NaiveTime::from_hms_opt(18, 30, 0).unwrap_or(NaiveTime::MIN),
        }
    }
}

impl WorkSchedule {
    /// Check the schedule's own rules.
    ///
    /// # Errors
    /// [`Error::Invalid`] if start equals end or either has seconds.
    pub fn validate(&self) -> Result<()> {
        if self.start == self.end {
            return Err(Error::invalid("work hours start and end must differ"));
        }
        for t in [self.start, self.end] {
            if t.second() != 0 || t.nanosecond() != 0 {
                return Err(Error::invalid("work hours use whole minutes"));
            }
        }
        Ok(())
    }

    /// Whether the window runs past local midnight.
    #[must_use]
    pub fn is_overnight(&self) -> bool {
        self.end <= self.start
    }

    /// The work window that starts on local date `date`, if `date` is a workday.
    #[must_use]
    pub fn window_for(&self, date: NaiveDate, zone: &UserZone) -> Option<WorkWindow> {
        if !self.workdays.contains(date.weekday()) {
            return None;
        }
        let end_date = if self.is_overnight() {
            date.succ_opt()?
        } else {
            date
        };
        let start = zone.resolve_local(date.and_time(self.start));
        if zone.local_date(start) != date {
            // The date does not exist (a skipped day): the gap rule would otherwise move
            // this window onto the next day's window.
            return None;
        }
        let end = zone.resolve_local(end_date.and_time(self.end));
        (end > start).then_some(WorkWindow { date, start, end })
    }

    /// The work window containing instant `t`, if any.
    #[must_use]
    pub fn window_at(&self, t: DateTime<Utc>, zone: &UserZone) -> Option<WorkWindow> {
        let today = zone.local_date(t);
        [Some(today), today.pred_opt()]
            .into_iter()
            .flatten()
            .filter_map(|d| self.window_for(d, zone))
            .find(|w| w.contains(t))
    }

    /// Whether instant `t` is within work hours.
    #[must_use]
    pub fn is_work_time(&self, t: DateTime<Utc>, zone: &UserZone) -> bool {
        self.window_at(t, zone).is_some()
    }

    /// The window containing `t`, or else the next one to start (searching 8 days ahead).
    /// `None` if there are no workdays.
    #[must_use]
    pub fn current_or_next_window(&self, t: DateTime<Utc>, zone: &UserZone) -> Option<WorkWindow> {
        let mut date = zone.local_date(t).pred_opt()?;
        for _ in 0..10 {
            if let Some(w) = self.window_for(date, zone)
                && w.end > t
            {
                return Some(w);
            }
            date = date.succ_opt()?;
        }
        None
    }
}

/// One concrete work window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkWindow {
    /// The local date the window starts on (an overnight window belongs to this date).
    pub date: NaiveDate,
    /// Start (inclusive).
    pub start: DateTime<Utc>,
    /// End (exclusive).
    pub end: DateTime<Utc>,
}

impl WorkWindow {
    /// Length of the window (DST-aware).
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    /// Whether `t` is inside the window.
    #[must_use]
    pub fn contains(&self, t: DateTime<Utc>) -> bool {
        self.start <= t && t < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        date(y, mo, d).and_hms_opt(h, mi, 0).unwrap()
    }

    fn zone(name: &str) -> UserZone {
        UserZone::parse(name).unwrap()
    }

    fn hm(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    // ---- zone parsing and serialisation ----

    #[test]
    fn default_zone_is_kolkata() {
        assert_eq!(UserZone::default().name(), DEFAULT_TIME_ZONE);
        assert_eq!(UserZone::KOLKATA, zone("Asia/Kolkata"));
    }

    #[test]
    fn zone_parse_rejects_unknown_names() {
        for bad in ["", "IST", "Asia/Kolkatta", "asia/kolkata ", "+05:30"] {
            assert!(
                matches!(UserZone::parse(bad), Err(Error::InvalidTimeZone(_))),
                "{bad:?} should be rejected"
            );
        }
        assert_eq!(
            "Europe/London".parse::<UserZone>().unwrap().name(),
            "Europe/London"
        );
    }

    #[test]
    fn zone_serialises_as_its_iana_name() {
        let json = serde_json::to_string(&UserZone::KOLKATA).unwrap();
        assert_eq!(json, "\"Asia/Kolkata\"");
        let back: UserZone = serde_json::from_str("\"America/New_York\"").unwrap();
        assert_eq!(back.name(), "America/New_York");
        assert!(serde_json::from_str::<UserZone>("\"Mars/Base\"").is_err());
        assert_eq!(format!("{:?}", UserZone::KOLKATA), "UserZone(Asia/Kolkata)");
        assert_eq!(UserZone::KOLKATA.to_string(), "Asia/Kolkata");
    }

    // ---- IST local days ----

    #[test]
    fn ist_day_starts_at_1830_utc_the_previous_day() {
        let ist = UserZone::KOLKATA;
        let b = ist.day_bounds(date(2026, 10, 21));
        assert_eq!(b.start, utc(2026, 10, 20, 18, 30));
        assert_eq!(b.end, utc(2026, 10, 21, 18, 30));
        assert_eq!(b.duration(), Duration::hours(24));
        assert_eq!(
            ist.local_date(utc(2026, 10, 20, 18, 29) + Duration::seconds(59)),
            date(2026, 10, 20)
        );
        assert_eq!(
            ist.local_date(utc(2026, 10, 20, 18, 30)),
            date(2026, 10, 21)
        );
        assert!(b.contains(b.start) && !b.contains(b.end));
    }

    #[test]
    fn every_ist_day_of_2026_is_24_hours() {
        let ist = UserZone::KOLKATA;
        let mut d = date(2026, 1, 1);
        while d.year() == 2026 {
            assert_eq!(ist.day_bounds(d).duration(), Duration::hours(24), "{d}");
            d = d.succ_opt().unwrap();
        }
    }

    #[test]
    fn ist_session_across_midnight_is_split() {
        // T-R6 shape: 23:00 to 01:30 IST.
        let ist = UserZone::KOLKATA;
        let start = ist.resolve_local(local(2026, 10, 21, 23, 0));
        let end = ist.resolve_local(local(2026, 10, 22, 1, 30));
        let parts = ist.split_by_local_day(start, end);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].date, date(2026, 10, 21));
        assert_eq!(parts[0].duration(), Duration::minutes(60));
        assert_eq!(parts[1].date, date(2026, 10, 22));
        assert_eq!(parts[1].duration(), Duration::minutes(90));
        assert_eq!(parts[0].end, parts[1].start);
        assert_eq!(parts[1].start, utc(2026, 10, 21, 18, 30));
    }

    #[test]
    fn split_handles_empty_reversed_and_multi_day_intervals() {
        let ist = UserZone::KOLKATA;
        let t = utc(2026, 10, 21, 4, 0);
        assert!(ist.split_by_local_day(t, t).is_empty());
        assert!(ist.split_by_local_day(t, t - Duration::hours(1)).is_empty());
        let parts = ist.split_by_local_day(t, t + Duration::days(3));
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[1].duration(), Duration::hours(24));
        assert_eq!(parts[2].duration(), Duration::hours(24));
        let total: Duration = parts.iter().map(DaySegment::duration).sum();
        assert_eq!(total, Duration::days(3));
    }

    #[test]
    fn split_exactly_at_midnight_does_not_create_empty_pieces() {
        let ist = UserZone::KOLKATA;
        let midnight = ist.day_start(date(2026, 10, 22));
        let parts = ist.split_by_local_day(midnight - Duration::hours(1), midnight);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].date, date(2026, 10, 21));
        let parts = ist.split_by_local_day(midnight, midnight + Duration::hours(1));
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].date, date(2026, 10, 22));
    }

    // ---- DST zones ----

    #[test]
    fn new_york_dst_days_are_23_and_25_hours() {
        let ny = zone("America/New_York");
        assert_eq!(
            ny.day_bounds(date(2026, 3, 8)).duration(),
            Duration::hours(23)
        );
        assert_eq!(
            ny.day_bounds(date(2026, 11, 1)).duration(),
            Duration::hours(25)
        );
        assert_eq!(
            ny.day_bounds(date(2026, 7, 1)).duration(),
            Duration::hours(24)
        );
    }

    #[test]
    fn gap_times_move_forward_and_fold_times_take_the_first_occurrence() {
        let ny = zone("America/New_York");
        // 02:30 does not exist on 8 Mar 2026: it resolves to 03:30 EDT (07:30 UTC).
        assert_eq!(
            ny.resolve_local(local(2026, 3, 8, 2, 30)),
            utc(2026, 3, 8, 7, 30)
        );
        // 01:30 occurs twice on 1 Nov 2026: the first (EDT, 05:30 UTC) is chosen.
        assert_eq!(
            ny.resolve_local(local(2026, 11, 1, 1, 30)),
            utc(2026, 11, 1, 5, 30)
        );
        // Ordinary times are exact.
        assert_eq!(
            ny.resolve_local(local(2026, 7, 1, 9, 0)),
            utc(2026, 7, 1, 13, 0)
        );
    }

    #[test]
    fn lord_howe_half_hour_shift() {
        let lh = zone("Australia/Lord_Howe");
        // DST starts first Sunday in October (4 Oct 2026) and ends first Sunday in April.
        assert_eq!(
            lh.day_bounds(date(2026, 10, 4)).duration(),
            Duration::minutes(23 * 60 + 30)
        );
        assert_eq!(
            lh.day_bounds(date(2026, 4, 5)).duration(),
            Duration::minutes(24 * 60 + 30)
        );
    }

    #[test]
    fn midnight_inside_a_gap_starts_the_day_after_the_gap() {
        // Chile springs forward at 24:00 on the first Saturday of September: on Sunday
        // 6 Sep 2026 local midnight does not exist and the day starts at 01:00 (-03).
        let scl = zone("America/Santiago");
        let b = scl.day_bounds(date(2026, 9, 6));
        assert_eq!(scl.local_datetime(b.start), local(2026, 9, 6, 1, 0));
        assert_eq!(b.duration(), Duration::hours(23));
        assert_eq!(
            scl.local_date(b.start - Duration::seconds(1)),
            date(2026, 9, 5)
        );
        // Falling back at 24:00 makes the Saturday before 25 hours long.
        assert_eq!(
            scl.day_bounds(date(2026, 4, 4)).duration(),
            Duration::hours(25)
        );
    }

    #[test]
    fn midnight_inside_a_fold_starts_at_the_first_midnight() {
        // Cuba falls back from 01:00 to 00:00 on the first Sunday of November.
        let hav = zone("America/Havana");
        let b = hav.day_bounds(date(2026, 11, 1));
        assert_eq!(b.duration(), Duration::hours(25));
        assert_eq!(b.start, utc(2026, 11, 1, 4, 0));
        // Both passes through 00:30 are on 1 Nov.
        assert_eq!(hav.local_date(utc(2026, 11, 1, 4, 30)), date(2026, 11, 1));
        assert_eq!(hav.local_date(utc(2026, 11, 1, 5, 30)), date(2026, 11, 1));
    }

    #[test]
    fn a_skipped_day_has_zero_length_and_never_appears_in_splits() {
        // Samoa moved across the date line: 30 Dec 2011 did not exist.
        let apia = zone("Pacific/Apia");
        let skipped = apia.day_bounds(date(2011, 12, 30));
        assert_eq!(skipped.duration(), Duration::zero());
        let start = apia.day_start(date(2011, 12, 29));
        let end = apia.day_bounds(date(2011, 12, 31)).end;
        let parts = apia.split_by_local_day(start, end);
        let dates: Vec<_> = parts.iter().map(|p| p.date).collect();
        assert_eq!(dates, vec![date(2011, 12, 29), date(2011, 12, 31)]);
    }

    #[test]
    fn new_york_night_session_across_fall_back_is_split_correctly() {
        let ny = zone("America/New_York");
        let start = ny.resolve_local(local(2026, 10, 31, 22, 0));
        let end = ny.resolve_local(local(2026, 11, 2, 2, 0));
        let parts = ny.split_by_local_day(start, end);
        let lens: Vec<_> = parts.iter().map(|p| p.duration().num_hours()).collect();
        assert_eq!(lens, vec![2, 25, 2]);
    }

    // ---- work schedule ----

    #[test]
    fn default_schedule_matches_the_approved_prototype() {
        let s = WorkSchedule::default();
        s.validate().unwrap();
        assert_eq!(s.workdays, WeekdaySet::MON_TO_SAT);
        assert!(!s.workdays.contains(Weekday::Sun));
        assert_eq!((s.start, s.end), (hm(9, 30), hm(18, 30)));
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::json!({
                "workdays": ["mon", "tue", "wed", "thu", "fri", "sat"],
                "start": "09:30",
                "end": "18:30"
            })
        );
    }

    #[test]
    fn schedule_json_round_trips_and_rejects_bad_input() {
        let s = WorkSchedule {
            workdays: [Weekday::Sun, Weekday::Mon].into_iter().collect(),
            start: hm(22, 0),
            end: hm(2, 0),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(
            json,
            r#"{"workdays":["mon","sun"],"start":"22:00","end":"02:00"}"#
        );
        assert_eq!(serde_json::from_str::<WorkSchedule>(&json).unwrap(), s);
        for bad in [
            r#"{"workdays":["mon","funday"],"start":"09:30","end":"18:30"}"#,
            r#"{"workdays":["mon","mon"],"start":"09:30","end":"18:30"}"#,
            r#"{"workdays":["mon"],"start":"9:30","end":"18:30"}"#,
            r#"{"workdays":["mon"],"start":"09:30:00","end":"18:30"}"#,
            r#"{"workdays":["mon"],"start":"25:00","end":"18:30"}"#,
            r#"{"workdays":["mon"],"start":"09:30","end":"18:30","extra":1}"#,
            r#"{"workdays":["mon"],"start":"09:30"}"#,
        ] {
            assert!(serde_json::from_str::<WorkSchedule>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn schedule_validation() {
        let mut s = WorkSchedule::default();
        s.end = s.start;
        assert!(matches!(s.validate(), Err(Error::Invalid(_))));
        let s = WorkSchedule {
            start: NaiveTime::from_hms_opt(9, 30, 15).unwrap(),
            ..WorkSchedule::default()
        };
        assert!(s.validate().is_err());
        let overnight = WorkSchedule {
            start: hm(22, 0),
            end: hm(6, 0),
            ..WorkSchedule::default()
        };
        overnight.validate().unwrap();
        assert!(overnight.is_overnight());
    }

    #[test]
    fn weekday_set_operations() {
        let s = WeekdaySet::EMPTY.with(Weekday::Wed).with(Weekday::Fri);
        assert!(s.contains(Weekday::Wed) && !s.contains(Weekday::Thu));
        assert_eq!(
            s.iter().collect::<Vec<_>>(),
            vec![Weekday::Wed, Weekday::Fri]
        );
        assert_eq!(s.without(Weekday::Wed).iter().count(), 1);
        assert!(WeekdaySet::EMPTY.is_empty());
        assert_eq!(WeekdaySet::ALL.iter().count(), 7);
        assert_eq!(WeekdaySet::MON_TO_SAT.with(Weekday::Sun), WeekdaySet::ALL);
        assert_eq!(format!("{s:?}"), r#"["wed", "fri"]"#);
    }

    #[test]
    fn ist_work_hours_boundaries_are_start_inclusive_end_exclusive() {
        let ist = UserZone::KOLKATA;
        let s = WorkSchedule::default();
        // Wednesday 21 Oct 2026.
        let at = |h, m| ist.resolve_local(local(2026, 10, 21, h, m));
        assert!(!s.is_work_time(at(9, 29), &ist));
        assert!(s.is_work_time(at(9, 30), &ist));
        assert!(s.is_work_time(at(18, 29), &ist));
        assert!(!s.is_work_time(at(18, 30), &ist));
        let w = s.window_for(date(2026, 10, 21), &ist).unwrap();
        assert_eq!(w.start, utc(2026, 10, 21, 4, 0));
        assert_eq!(w.duration(), Duration::hours(9));
    }

    #[test]
    fn sunday_is_off_and_saturday_is_on_by_default() {
        let ist = UserZone::KOLKATA;
        let s = WorkSchedule::default();
        assert!(s.window_for(date(2026, 10, 25), &ist).is_none()); // Sunday
        assert!(s.window_for(date(2026, 10, 24), &ist).is_some()); // Saturday
        let sunday_noon = ist.resolve_local(local(2026, 10, 25, 12, 0));
        assert!(!s.is_work_time(sunday_noon, &ist));
        // From Saturday evening the next window is Monday morning.
        let sat_evening = ist.resolve_local(local(2026, 10, 24, 19, 0));
        let next = s.current_or_next_window(sat_evening, &ist).unwrap();
        assert_eq!(next.date, date(2026, 10, 26));
        assert_eq!(ist.local_datetime(next.start), local(2026, 10, 26, 9, 30));
        // Inside a window, the current one is returned.
        let mon_ten = ist.resolve_local(local(2026, 10, 26, 10, 0));
        assert_eq!(s.current_or_next_window(mon_ten, &ist), Some(next));
    }

    #[test]
    fn no_workdays_means_no_windows() {
        let s = WorkSchedule {
            workdays: WeekdaySet::EMPTY,
            ..WorkSchedule::default()
        };
        let t = utc(2026, 10, 21, 6, 0);
        assert!(!s.is_work_time(t, &UserZone::KOLKATA));
        assert!(s.current_or_next_window(t, &UserZone::KOLKATA).is_none());
    }

    #[test]
    fn overnight_window_belongs_to_the_day_it_starts() {
        let ist = UserZone::KOLKATA;
        let s = WorkSchedule {
            start: hm(22, 0),
            end: hm(2, 0),
            ..WorkSchedule::default()
        };
        // Saturday 22:00 to Sunday 02:00 counts, although Sunday is not a workday.
        let sun_0100 = ist.resolve_local(local(2026, 10, 25, 1, 0));
        let w = s.window_at(sun_0100, &ist).unwrap();
        assert_eq!(w.date, date(2026, 10, 24));
        assert_eq!(w.duration(), Duration::hours(4));
        // Sunday 22:00 does not start a window; Sunday 02:00 is the exclusive end.
        assert!(!s.is_work_time(ist.resolve_local(local(2026, 10, 25, 22, 30)), &ist));
        assert!(!s.is_work_time(ist.resolve_local(local(2026, 10, 25, 2, 0)), &ist));
        // Monday 01:00 is not covered: Sunday has no window.
        assert!(!s.is_work_time(ist.resolve_local(local(2026, 10, 26, 1, 0)), &ist));
        // Tuesday 01:00 is Monday's window.
        let w = s
            .window_at(ist.resolve_local(local(2026, 10, 27, 1, 0)), &ist)
            .unwrap();
        assert_eq!(w.date, date(2026, 10, 26));
    }

    #[test]
    fn work_windows_are_dst_aware() {
        let ny = zone("America/New_York");
        let overnight = WorkSchedule {
            workdays: WeekdaySet::ALL,
            start: hm(22, 0),
            end: hm(6, 0),
        };
        // The night of the fall-back change has an extra hour.
        let w = overnight.window_for(date(2026, 10, 31), &ny).unwrap();
        assert_eq!(w.duration(), Duration::hours(9));
        // A window starting inside the spring gap starts when the gap ends.
        let early = WorkSchedule {
            workdays: WeekdaySet::ALL,
            start: hm(2, 30),
            end: hm(10, 0),
        };
        let w = early.window_for(date(2026, 3, 8), &ny).unwrap();
        assert_eq!(ny.local_datetime(w.start), local(2026, 3, 8, 3, 30));
        assert_eq!(w.duration(), Duration::minutes(6 * 60 + 30));
        // A window lying entirely inside the gap moves forward by the gap, like any
        // other gap time, and keeps its length.
        let inside = WorkSchedule {
            workdays: WeekdaySet::ALL,
            start: hm(2, 0),
            end: hm(2, 30),
        };
        let w = inside.window_for(date(2026, 3, 8), &ny).unwrap();
        assert_eq!(ny.local_datetime(w.start), local(2026, 3, 8, 3, 0));
        assert_eq!(w.duration(), Duration::minutes(30));
        // On a day that does not exist there is no window at all.
        let apia = zone("Pacific/Apia");
        assert!(
            WorkSchedule::default()
                .window_for(date(2011, 12, 30), &apia)
                .is_none()
        );
        assert!(
            WorkSchedule::default()
                .window_for(date(2011, 12, 31), &apia)
                .is_some()
        );
    }

    #[test]
    fn wall_clock_change_moves_work_hours_with_the_clock() {
        // Work hours are evaluated on whatever the wall clock says now.
        use crate::clock::{Clock, ManualClock};
        let ist = UserZone::KOLKATA;
        let s = WorkSchedule::default();
        let clock = ManualClock::new(ist.resolve_local(local(2026, 10, 21, 18, 0)));
        assert!(s.is_work_time(clock.now(), &ist));
        clock.shift_wall(Duration::hours(1)); // clock set forward past 18:30
        assert!(!s.is_work_time(clock.now(), &ist));
        clock.shift_wall(Duration::hours(-12)); // set back to 07:00
        assert!(!s.is_work_time(clock.now(), &ist));
        assert_eq!(ist.local_date(clock.now()), date(2026, 10, 21));
        clock.shift_wall(Duration::hours(-8)); // set back across midnight
        assert_eq!(ist.local_date(clock.now()), date(2026, 10, 20));
    }

    // ---- properties ----

    const ZONES: &[&str] = &[
        "Asia/Kolkata",
        "America/New_York",
        "Europe/London",
        "Australia/Lord_Howe",
        "America/Santiago",
        "America/Havana",
        "Asia/Kathmandu",
        "America/St_Johns",
        "Pacific/Chatham",
        "Pacific/Apia",
        "Asia/Beirut",
    ];

    fn instant() -> impl Strategy<Value = DateTime<Utc>> {
        // 2000-01-01 to 2040-01-01, minute precision.
        (946_684_800i64 / 60..2_208_988_800i64 / 60)
            .prop_map(|m| DateTime::<Utc>::from_timestamp(m * 60, 0).unwrap())
    }

    proptest! {
        #[test]
        fn local_date_matches_day_bounds(z in prop::sample::select(ZONES), t in instant()) {
            let zone = zone(z);
            let d = zone.local_date(t);
            let b = zone.day_bounds(d);
            prop_assert!(b.contains(t), "{z} {t} -> {d} bounds {:?}", b);
        }

        #[test]
        fn split_pieces_are_contiguous_and_dated(
            z in prop::sample::select(ZONES),
            t in instant(),
            minutes in 0i64..(6 * 24 * 60),
        ) {
            let zone = zone(z);
            let end = t + Duration::minutes(minutes);
            let parts = zone.split_by_local_day(t, end);
            if minutes == 0 {
                prop_assert!(parts.is_empty());
            } else {
                prop_assert_eq!(parts.first().unwrap().start, t);
                prop_assert_eq!(parts.last().unwrap().end, end);
                for w in parts.windows(2) {
                    prop_assert_eq!(w[0].end, w[1].start);
                    prop_assert!(w[0].date < w[1].date);
                }
                for p in &parts {
                    prop_assert!(p.start < p.end);
                    prop_assert_eq!(zone.local_date(p.start), p.date);
                    prop_assert_eq!(zone.local_date(p.end - Duration::milliseconds(1)), p.date);
                }
                let total: Duration = parts.iter().map(DaySegment::duration).sum();
                prop_assert_eq!(total, end - t);
            }
        }

        #[test]
        fn resolve_local_round_trips_existing_times(z in prop::sample::select(ZONES), t in instant()) {
            let zone = zone(z);
            let wall = zone.local_datetime(t);
            let back = zone.resolve_local(wall);
            // Exact unless the wall time is repeated, when the first occurrence is chosen.
            prop_assert!(back <= t);
            prop_assert_eq!(zone.local_datetime(back), wall);
        }

        #[test]
        fn work_windows_never_overlap_and_start_on_their_date(
            z in prop::sample::select(ZONES),
            t in instant(),
            start in 0u32..24,
            end in 0u32..24,
        ) {
            prop_assume!(start != end);
            let zone = zone(z);
            let s = WorkSchedule { workdays: WeekdaySet::ALL, start: hm(start, 0), end: hm(end, 0) };
            let first = zone.local_date(t);
            let windows: Vec<WorkWindow> = (0..4)
                .filter_map(|i| first.checked_add_days(chrono::Days::new(i)))
                .filter_map(|d| s.window_for(d, &zone))
                .collect();
            for w in &windows {
                prop_assert_eq!(zone.local_date(w.start), w.date);
                prop_assert!(w.end > w.start);
            }
            for pair in windows.windows(2) {
                prop_assert!(pair[0].end <= pair[1].start, "{z}: {:?} overlaps {:?}", pair[0], pair[1]);
            }
        }

        #[test]
        fn work_time_agrees_with_window_for(t in instant(), start in 0u32..24, end in 0u32..24) {
            prop_assume!(start != end);
            let s = WorkSchedule { workdays: WeekdaySet::MON_TO_SAT, start: hm(start, 0), end: hm(end, 0) };
            let ist = UserZone::KOLKATA;
            let today = ist.local_date(t);
            let expected = [Some(today), today.pred_opt()]
                .into_iter()
                .flatten()
                .filter_map(|d| s.window_for(d, &ist))
                .any(|w| w.contains(t));
            prop_assert_eq!(s.is_work_time(t, &ist), expected);
            if let Some(w) = s.current_or_next_window(t, &ist) {
                prop_assert!(w.end > t);
                prop_assert!(w.duration() > Duration::zero());
            }
        }
    }
}
