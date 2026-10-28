//! The water pace model (wireframe B2 and section 8 item 12, approved sample rule).
//!
//! The user's own daily goal is spread evenly over the user's own work window for the day,
//! and the share expected by now follows the time the user was at the PC inside that
//! window, not the clock (DEC13): time locked, asleep, with the PC off or with the app
//! not running adds nothing, so after an unlock or a start the pace goes on from where it
//! stopped instead of jumping ahead. The user is "behind pace" only when the logged amount
//! is at least [`WATER_BEHIND_ML`] below the amount expected by now. A daily total below
//! the goal is not, by itself, "behind". Once the goal is reached, hydration reminders
//! pause; manual logging stays available and nothing encourages drinking past the goal
//! (WAT-01).
//!
//! The water panel (wireframe B3) lists today's [`WaterEntry`]s and logs, corrects and
//! removes them. Its buttons use the user's [`Servings`] (wireframe W5) and the steps
//! below.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::WATER_BEHIND_ML;
use crate::error::{Error, Result};
use crate::schema::MAX_WATER_ENTRY_ML;
use crate::time::{UserZone, WorkSchedule};

/// The default serving buttons (wireframe B3; the prototype's defaults). The user can
/// change them in Settings, Water and workweek (W5): [`Servings`].
pub const WATER_SERVINGS_ML: [u32; 3] = [150, 250, 500];

/// Settings key under which [`Servings`] is stored.
pub const SERVINGS_KEY: &str = "water_servings";

/// At most this many serving buttons (they share one row in the bubble; D).
pub const MAX_SERVINGS: usize = 4;

/// The serving logged by "Log 250 ml" on a water reminder and "+250 ml water" on the pet
/// menu, and where the panel's other amount starts.
pub const WATER_DEFAULT_SERVING_ML: u32 = 250;

/// One press of "-50 ml" or "+50 ml" on an amount in the water panel (D).
pub const WATER_STEP_ML: u32 = 50;

/// The amounts the panel's buttons can set (D). The pet window never takes keyboard focus,
/// so amounts are chosen with buttons; the store accepts any amount from 1 to 2000 ml.
pub const WATER_PANEL_RANGE_ML: std::ops::RangeInclusive<u32> = WATER_STEP_ML..=MAX_WATER_ENTRY_ML;

/// The user's serving buttons (wireframe W5): 1 to [`MAX_SERVINGS`] distinct amounts
/// inside [`WATER_PANEL_RANGE_ML`], in steps of 10 ml, kept in ascending order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<u32>", into = "Vec<u32>")]
pub struct Servings(Vec<u32>);

impl Servings {
    /// Check and sort a list of serving sizes.
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn new(mut ml: Vec<u32>) -> Result<Self> {
        if ml.is_empty() || ml.len() > MAX_SERVINGS {
            return Err(Error::invalid(format!(
                "choose 1 to {MAX_SERVINGS} serving buttons"
            )));
        }
        for m in &ml {
            if !WATER_PANEL_RANGE_ML.contains(m) || !m.is_multiple_of(10) {
                return Err(Error::invalid(format!(
                    "a serving is {} to {} ml in steps of 10 ml",
                    WATER_PANEL_RANGE_ML.start(),
                    WATER_PANEL_RANGE_ML.end()
                )));
            }
        }
        ml.sort_unstable();
        if ml.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::invalid("each serving button needs its own amount"));
        }
        Ok(Self(ml))
    }

    /// The amounts, smallest first.
    #[must_use]
    pub fn ml(&self) -> &[u32] {
        &self.0
    }

    /// Whether `ml` is one of the buttons.
    #[must_use]
    pub fn contains(&self, ml: u32) -> bool {
        self.0.contains(&ml)
    }
}

impl Default for Servings {
    fn default() -> Self {
        Self(WATER_SERVINGS_ML.to_vec())
    }
}

impl TryFrom<Vec<u32>> for Servings {
    type Error = Error;

    fn try_from(ml: Vec<u32>) -> Result<Self> {
        Self::new(ml)
    }
}

impl From<Servings> for Vec<u32> {
    fn from(s: Servings) -> Self {
        s.0
    }
}

/// `ml` one step up or down, kept inside [`WATER_PANEL_RANGE_ML`].
#[must_use]
pub fn step_amount(ml: u32, up: bool) -> u32 {
    let next = if up {
        ml.saturating_add(WATER_STEP_ML)
    } else {
        ml.saturating_sub(WATER_STEP_ML)
    };
    next.clamp(*WATER_PANEL_RANGE_ML.start(), *WATER_PANEL_RANGE_ML.end())
}

/// One of today's water entries as it stands now (after corrections; removed entries are
/// not listed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaterEntry {
    /// The stored event that holds the entry's current amount. A correction or removal
    /// supersedes this event.
    pub id: Uuid,
    /// When it was drunk, in the time zone the entry was recorded in.
    pub time: NaiveTime,
    /// Amount in ml.
    pub amount_ml: u32,
}

/// What the store knows about today's water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WaterSnapshot {
    /// Total logged today (current entries only, after corrections and undo).
    pub logged_ml: u32,
    /// The user's goal in force today, if one is set. Without a goal there is no pace and
    /// no hydration reminder.
    pub goal_ml: Option<u32>,
}

/// The pace at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaterPace {
    /// Logged today.
    pub logged_ml: u32,
    /// The user's goal.
    pub goal_ml: u32,
    /// Amount expected by now, rounded to 10 ml: the goal's share for the time the user
    /// was at the PC inside the window (0 before the window, at most the goal, 0 on a day
    /// without a work window).
    pub expected_ml: u32,
    /// How far behind the expected amount the user is (0 when on or ahead of pace).
    pub behind_ml: u32,
    /// The goal is reached: hydration reminders pause.
    pub goal_reached: bool,
}

impl WaterPace {
    /// Whether a hydration reminder may be due on pace grounds.
    #[must_use]
    pub const fn is_behind(&self) -> bool {
        !self.goal_reached && self.behind_ml >= WATER_BEHIND_ML
    }
}

/// The pace at `now`, or `None` without a goal. `present` gives the time the user was at
/// the PC so far inside the work window that starts on a date (the tracker's
/// [`DayTally::window_screen_ms`](super::DayTally::window_screen_ms)).
#[must_use]
pub fn pace(
    snapshot: WaterSnapshot,
    now: DateTime<Utc>,
    schedule: &WorkSchedule,
    zone: &UserZone,
    present: impl FnOnce(NaiveDate) -> Duration,
) -> Option<WaterPace> {
    let goal = snapshot.goal_ml?;
    let today = zone.local_date(now);
    // The window containing `now` governs (it may have started yesterday and run past
    // midnight); otherwise today's window, before or after it.
    let expected = schedule
        .window_at(now, zone)
        .or_else(|| schedule.window_for(today, zone))
        .map_or(0, |w| expected_in(goal, present(w.date), w.end - w.start));
    let goal_reached = snapshot.logged_ml >= goal;
    Some(WaterPace {
        logged_ml: snapshot.logged_ml,
        goal_ml: goal,
        expected_ml: expected,
        behind_ml: expected.saturating_sub(snapshot.logged_ml),
        goal_reached,
    })
}

fn expected_in(goal: u32, present: Duration, window: chrono::TimeDelta) -> u32 {
    let span = window.num_milliseconds().max(1);
    let done = i64::try_from(present.as_millis())
        .unwrap_or(i64::MAX)
        .clamp(0, span);
    // Linear share of the goal, rounded to the nearest 10 ml as in the prototype.
    let raw = i128::from(goal) * i128::from(done) / i128::from(span);
    let rounded = (raw + 5) / 10 * 10;
    u32::try_from(rounded).unwrap_or(goal).min(goal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, TimeZone};

    /// Tuesday 20 Oct 2026 at `h:m` IST.
    fn at(h: u32, m: u32) -> DateTime<Utc> {
        UserZone::KOLKATA.resolve_local(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 20)
                .unwrap()
                .and_hms_opt(h, m, 0)
                .unwrap(),
        )
    }

    fn snap(logged: u32) -> WaterSnapshot {
        WaterSnapshot {
            logged_ml: logged,
            goal_ml: Some(2000),
        }
    }

    /// The pace at `h:m` for a user at the PC all the time since the window opened at
    /// 09:30 (so the time at the PC equals the clock time inside the window).
    fn p(logged: u32, h: u32, m: u32) -> WaterPace {
        let present = (at(h, m) - at(9, 30))
            .clamp(chrono::TimeDelta::zero(), at(18, 30) - at(9, 30))
            .to_std()
            .unwrap();
        at_pc(logged, h, m, present)
    }

    /// The pace at `h:m` after `present` at the PC inside today's window.
    fn at_pc(logged: u32, h: u32, m: u32, present: Duration) -> WaterPace {
        pace(
            snap(logged),
            at(h, m),
            &WorkSchedule::default(),
            &UserZone::KOLKATA,
            |d| {
                assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
                present
            },
        )
        .unwrap()
    }

    fn hours(h: u64) -> Duration {
        Duration::from_secs(h * 3600)
    }

    #[test]
    fn panel_steps_stay_in_range() {
        assert_eq!(step_amount(250, true), 300);
        assert_eq!(step_amount(250, false), 200);
        assert_eq!(step_amount(50, false), 50);
        assert_eq!(step_amount(2000, true), 2000);
        // An amount entered off the steps moves onto the range.
        assert_eq!(step_amount(30, true), 80);
        assert_eq!(step_amount(30, false), 50);
        assert_eq!(step_amount(1990, true), 2000);
        assert!(
            WATER_SERVINGS_ML
                .iter()
                .all(|s| WATER_PANEL_RANGE_ML.contains(s))
        );
        assert!(WATER_SERVINGS_ML.contains(&WATER_DEFAULT_SERVING_ML));
    }

    #[test]
    fn servings_follow_their_rules() {
        assert_eq!(Servings::default().ml(), &WATER_SERVINGS_ML);
        let s = Servings::new(vec![750, 150, 250, 500]).unwrap();
        assert_eq!(s.ml(), &[150, 250, 500, 750]);
        assert!(s.contains(750));
        assert!(Servings::new(vec![]).is_err());
        assert!(Servings::new(vec![100, 200, 300, 400, 500]).is_err());
        assert!(Servings::new(vec![40]).is_err());
        assert!(Servings::new(vec![2010]).is_err());
        assert!(Servings::new(vec![255]).is_err());
        assert!(Servings::new(vec![250, 250]).is_err());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "[150,250,500,750]");
        assert_eq!(serde_json::from_str::<Servings>(&json).unwrap(), s);
        assert!(serde_json::from_str::<Servings>("[250,250]").is_err());
    }

    #[test]
    fn prototype_sample_scenario() {
        // 250 ml logged at 11:15 against about 390 ml expected (09:30 to 18:30, 2000 ml).
        let w = p(250, 11, 15);
        assert_eq!(w.expected_ml, 390);
        assert_eq!(w.behind_ml, 140);
        assert!(w.is_behind());
    }

    #[test]
    fn behind_needs_at_least_100_ml() {
        // 11:15 expects 390 ml.
        assert!(p(290, 11, 15).is_behind()); // exactly 100 under
        assert!(!p(291, 11, 15).is_behind()); // 99 under
        assert!(!p(500, 11, 15).is_behind()); // ahead
        assert_eq!(p(500, 11, 15).behind_ml, 0);
    }

    #[test]
    fn below_goal_is_not_behind_by_itself() {
        // Early in the day a small total is on pace even though it is far below the goal.
        let w = p(0, 9, 45);
        assert!(w.expected_ml < 100);
        assert!(!w.is_behind());
    }

    #[test]
    fn window_edges_and_goal() {
        assert_eq!(p(0, 8, 0).expected_ml, 0);
        assert_eq!(p(0, 9, 30).expected_ml, 0);
        assert_eq!(p(0, 14, 0).expected_ml, 1000);
        assert_eq!(p(0, 18, 30).expected_ml, 2000);
        assert_eq!(p(0, 22, 0).expected_ml, 2000);
        let reached = p(2000, 12, 0);
        assert!(reached.goal_reached);
        assert!(!reached.is_behind());
        // Even far behind the schedule, a reached goal pauses hydration reminders.
        let over = pace(
            WaterSnapshot {
                logged_ml: 2100,
                goal_ml: Some(2000),
            },
            at(18, 0),
            &WorkSchedule::default(),
            &UserZone::KOLKATA,
            |_| hours(8),
        )
        .unwrap();
        assert!(!over.is_behind());
    }

    #[test]
    fn pace_follows_time_at_the_pc_not_the_clock() {
        // 14:00 with only 2 h at the PC (locked, asleep, off or not running the rest):
        // 2 h of the 9 h window expects 444 ml, rounded to 440, not the clock's 1000 ml.
        let w = at_pc(250, 14, 0, hours(2));
        assert_eq!(w.expected_ml, 440);
        assert_eq!(w.behind_ml, 190);
        assert!(w.is_behind());
        assert!(!at_pc(350, 14, 0, hours(2)).is_behind());
        // Nothing at the PC yet (the app started just now): nothing expected.
        assert_eq!(at_pc(0, 16, 0, Duration::ZERO).expected_ml, 0);
        // After the window the share stays what the time at the PC earned.
        assert_eq!(at_pc(0, 22, 0, hours(3)).expected_ml, 670);
        // The share never passes the goal, whatever is reported.
        assert_eq!(at_pc(0, 18, 30, hours(12)).expected_ml, 2000);
    }

    #[test]
    fn no_goal_means_no_pace() {
        assert_eq!(
            pace(
                WaterSnapshot::default(),
                at(12, 0),
                &WorkSchedule::default(),
                &UserZone::KOLKATA,
                |_| hours(2),
            ),
            None
        );
    }

    #[test]
    fn non_workday_expects_nothing() {
        // Sunday 25 Oct 2026.
        let sunday = Utc.with_ymd_and_hms(2026, 10, 25, 7, 0, 0).unwrap();
        let w = pace(
            snap(0),
            sunday,
            &WorkSchedule::default(),
            &UserZone::KOLKATA,
            |_| hours(5),
        )
        .unwrap();
        assert_eq!(w.expected_ml, 0);
        assert!(!w.is_behind());
    }

    #[test]
    fn overnight_window_uses_the_window_that_started_yesterday() {
        let schedule = WorkSchedule {
            workdays: crate::time::WeekdaySet::ALL,
            start: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        };
        // 02:00 on the 21st, after 4 h at the PC in the window that began at 22:00 on the
        // 20th: half the goal, counted on the window's own date.
        let t = UserZone::KOLKATA.resolve_local(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 21)
                .unwrap()
                .and_hms_opt(2, 0, 0)
                .unwrap(),
        );
        let w = pace(snap(0), t, &schedule, &UserZone::KOLKATA, |d| {
            assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap());
            hours(4)
        })
        .unwrap();
        assert_eq!(w.expected_ml, 1000);
    }
}
