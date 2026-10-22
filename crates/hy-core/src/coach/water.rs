//! The water pace model (wireframe B2 and section 8 item 12, approved sample rule).
//!
//! The user's own daily goal is spread evenly over the user's own work window for the day.
//! The user is "behind pace" only when the logged amount is at least
//! [`WATER_BEHIND_ML`] below the amount expected by now. A daily total below the goal is
//! not, by itself, "behind". Once the goal is reached, hydration reminders pause; manual
//! logging stays available and nothing encourages drinking past the goal (WAT-01).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::config::WATER_BEHIND_ML;
use crate::time::{UserZone, WorkSchedule};

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
    /// Amount expected by now, rounded to 10 ml (0 before the window, the goal after it,
    /// 0 on a day without a work window).
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

/// The pace at `now`, or `None` without a goal.
#[must_use]
pub fn pace(
    snapshot: WaterSnapshot,
    now: DateTime<Utc>,
    schedule: &WorkSchedule,
    zone: &UserZone,
) -> Option<WaterPace> {
    let goal = snapshot.goal_ml?;
    let today = zone.local_date(now);
    // The window containing `now` governs (it may have started yesterday and run past
    // midnight); otherwise today's window, before or after it.
    let expected = schedule
        .window_at(now, zone)
        .or_else(|| schedule.window_for(today, zone))
        .map_or(0, |w| expected_in(goal, now, w.start, w.end));
    let goal_reached = snapshot.logged_ml >= goal;
    Some(WaterPace {
        logged_ml: snapshot.logged_ml,
        goal_ml: goal,
        expected_ml: expected,
        behind_ml: expected.saturating_sub(snapshot.logged_ml),
        goal_reached,
    })
}

fn expected_in(goal: u32, now: DateTime<Utc>, start: DateTime<Utc>, end: DateTime<Utc>) -> u32 {
    let span = (end - start).num_milliseconds().max(1);
    let done = (now - start).num_milliseconds().clamp(0, span);
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

    fn p(logged: u32, h: u32, m: u32) -> WaterPace {
        pace(
            snap(logged),
            at(h, m),
            &WorkSchedule::default(),
            &UserZone::KOLKATA,
        )
        .unwrap()
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
        )
        .unwrap();
        assert!(!over.is_behind());
    }

    #[test]
    fn no_goal_means_no_pace() {
        assert_eq!(
            pace(
                WaterSnapshot::default(),
                at(12, 0),
                &WorkSchedule::default(),
                &UserZone::KOLKATA
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
        // 02:00 on the 21st is half way through the window that began at 22:00 on the 20th.
        let t = UserZone::KOLKATA.resolve_local(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 21)
                .unwrap()
                .and_hms_opt(2, 0, 0)
                .unwrap(),
        );
        let w = pace(snap(0), t, &schedule, &UserZone::KOLKATA).unwrap();
        assert_eq!(w.expected_ml, 1000);
    }
}
