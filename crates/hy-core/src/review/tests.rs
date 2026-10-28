#![allow(clippy::unwrap_used)]

use chrono::{Datelike, NaiveDate, Weekday};

use super::*;
use crate::schema::{PhoneLabel, RoutineOutcome};
use crate::store::{ActivityDay, MonthKey};
use crate::time::WeekdaySet;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// 26 Oct 2026 is a Monday.
fn oct(day: u32) -> NaiveDate {
    d(2026, 10, day)
}

fn cal(today: NaiveDate, first_day: NaiveDate) -> Calendar {
    Calendar {
        today,
        first_day,
        workdays: WeekdaySet::MON_TO_SAT,
    }
}

fn tally(date: NaiveDate) -> ActivityDay {
    ActivityDay {
        date,
        pc_active_s: 3600,
        work_active_s: 3000,
        work_quiet_s: 600,
        unknown_s: 0,
        away_s: 0,
        confirmed_breaks: 1,
        longest_block_s: 2400,
        focus_sessions: 0,
        pomodoro_rounds: 0,
        screen_s: Some(4500),
        revision: 1,
    }
}

/// A day the PC recorded, with nothing logged.
fn recorded(date: NaiveDate) -> DayFacts {
    DayFacts {
        tally: Some(tally(date)),
        ..DayFacts::empty(date)
    }
}

fn week_of(start: NaiveDate, f: impl Fn(NaiveDate) -> DayFacts) -> Vec<DayFacts> {
    start.iter_days().take(7).map(f).collect()
}

fn phone_none() -> PhoneWeek {
    PhoneWeek {
        show: false,
        counts: PhoneCounts::default(),
        entries: Vec::new(),
    }
}

#[test]
fn weeks_start_on_monday() {
    for day in 26..=31 {
        assert_eq!(week_start(oct(day)), oct(26));
    }
    assert_eq!(week_start(d(2026, 11, 1)), oct(26)); // Sunday
    assert_eq!(week_start(d(2026, 11, 2)), d(2026, 11, 2));
    // Across a year end: Friday 1 Jan 2027 belongs to the week of Monday 28 Dec 2026.
    assert_eq!(week_start(d(2027, 1, 1)), d(2026, 12, 28));
}

#[test]
fn earliest_week_is_first_use_but_within_ninety_days() {
    let c = cal(oct(28), oct(21));
    assert_eq!(c.earliest_week(), oct(19));
    let long_ago = cal(oct(28), d(2025, 1, 6));
    // 90 days back from 28 Oct 2026 (inclusive) is 31 Jul 2026, in the week of 27 Jul.
    assert_eq!(long_ago.earliest_week(), d(2026, 7, 27));
}

#[test]
fn missing_days_are_no_data_never_zero() {
    let c = cal(oct(31), oct(1));
    let nothing = DayFacts::empty(oct(27));
    for m in Metric::ALL {
        assert_eq!(mark(&c, &nothing, m), DayMark::NoData, "{m:?}");
        assert!(!DayMark::NoData.counts());
    }
    // Recorded with nothing logged: a measured zero.
    let r = recorded(oct(27));
    assert_eq!(mark(&c, &r, Metric::Movement), DayMark::Zero);
    assert_eq!(mark(&c, &r, Metric::Water), DayMark::Zero);
    // No break was offered: not a zero.
    assert_eq!(mark(&c, &r, Metric::Breaks), DayMark::NothingDue);
    assert!(!DayMark::NothingDue.counts());
    // A day with events but no tally (the tracker had not saved yet) is recorded too.
    let events_only = DayFacts {
        events: 1,
        ..DayFacts::empty(oct(27))
    };
    assert_eq!(mark(&c, &events_only, Metric::Movement), DayMark::Zero);
}

#[test]
fn off_days_future_today_and_before_start() {
    let c = cal(oct(28), oct(26));
    // Sunday 25 Oct is before the first day: not started.
    assert_eq!(
        mark(&c, &DayFacts::empty(oct(25)), Metric::Water),
        DayMark::NotStarted
    );
    // Sunday 1 Nov is in the future.
    assert_eq!(
        mark(&c, &DayFacts::empty(d(2026, 11, 1)), Metric::Water),
        DayMark::Future
    );
    // Today is in progress unless already met.
    let mut today = recorded(oct(28));
    assert_eq!(mark(&c, &today, Metric::Movement), DayMark::Today);
    today.note_routine(RoutineOutcome::Partial);
    assert_eq!(mark(&c, &today, Metric::Movement), DayMark::Today);
    today.note_routine(RoutineOutcome::Done);
    assert_eq!(mark(&c, &today, Metric::Movement), DayMark::Met);

    // A Sunday with nothing done is off, recorded or not; a routine done on it shows.
    let c = cal(d(2026, 11, 3), oct(1));
    let sunday = d(2026, 11, 1);
    assert_eq!(sunday.weekday(), Weekday::Sun);
    assert_eq!(
        mark(&c, &DayFacts::empty(sunday), Metric::Movement),
        DayMark::Off
    );
    assert_eq!(mark(&c, &recorded(sunday), Metric::Movement), DayMark::Off);
    let mut done = recorded(sunday);
    done.note_routine(RoutineOutcome::Done);
    assert_eq!(mark(&c, &done, Metric::Movement), DayMark::Met);
}

#[test]
fn routine_marks_keep_the_best_result_of_the_day() {
    let c = cal(oct(31), oct(1));
    let mut f = recorded(oct(27));
    f.note_routine(RoutineOutcome::Skipped);
    assert_eq!(mark(&c, &f, Metric::Movement), DayMark::Zero);
    f.note_routine(RoutineOutcome::None);
    assert_eq!(mark(&c, &f, Metric::Movement), DayMark::Zero);
    f.note_routine(RoutineOutcome::Partial);
    assert_eq!(mark(&c, &f, Metric::Movement), DayMark::Partly);
    f.note_routine(RoutineOutcome::Done);
    f.note_routine(RoutineOutcome::Partial);
    assert_eq!(f.routine, Some(RoutineOutcome::Done));
    assert_eq!(f.routines_done, 1);
    assert_eq!(mark(&c, &f, Metric::Movement), DayMark::Met);
}

#[test]
fn water_is_judged_against_that_days_own_goal() {
    let c = cal(oct(31), oct(1));
    let day = |ml, goal| DayFacts {
        water_ml: ml,
        water_entries: u32::from(ml > 0),
        water_goal_ml: goal,
        ..recorded(oct(27))
    };
    assert_eq!(
        mark(&c, &day(2000, Some(2000)), Metric::Water),
        DayMark::Met
    );
    assert_eq!(
        mark(&c, &day(1999, Some(2000)), Metric::Water),
        DayMark::Partly
    );
    assert_eq!(mark(&c, &day(0, Some(2000)), Metric::Water), DayMark::Zero);
    // With no goal set, some water is "some logged", never a goal day.
    assert_eq!(mark(&c, &day(500, None), Metric::Water), DayMark::Partly);
    // Days without a goal are left out of the goal-day rate.
    let days = [day(2000, Some(2000)), day(500, None), day(0, Some(1500))];
    assert_eq!(rate(&c, &days, Metric::Water), Ratio::new(1, 2));
}

#[test]
fn breaks_count_answered_offers_only() {
    let c = cal(oct(31), oct(1));
    let day = |offered, taken| DayFacts {
        breaks_offered: offered,
        breaks_taken: taken,
        ..recorded(oct(27))
    };
    assert_eq!(mark(&c, &day(3, 3), Metric::Breaks), DayMark::Met);
    assert_eq!(mark(&c, &day(3, 1), Metric::Breaks), DayMark::Partly);
    assert_eq!(mark(&c, &day(3, 0), Metric::Breaks), DayMark::Zero);
    assert_eq!(mark(&c, &day(0, 0), Metric::Breaks), DayMark::NothingDue);
    let days = [day(4, 3), day(2, 0), day(0, 0), DayFacts::empty(oct(29))];
    assert_eq!(rate(&c, &days, Metric::Breaks), Ratio::new(3, 6));
}

#[test]
fn ratios_round_half_up_and_empty_has_no_percentage() {
    assert_eq!(Ratio::new(0, 0).pct, None);
    assert_eq!(Ratio::new(0, 4).pct, Some(0));
    assert_eq!(Ratio::new(1, 3).pct, Some(33));
    assert_eq!(Ratio::new(2, 3).pct, Some(67));
    assert_eq!(Ratio::new(1, 8).pct, Some(13)); // 12.5 rounds up
    assert_eq!(Ratio::new(6, 6).pct, Some(100));
}

#[test]
fn a_complete_week_with_mixed_results() {
    // Monday 19 to Sunday 25 Oct, reviewed on Monday 26 Oct; in use since 1 Oct.
    let c = cal(oct(26), oct(1));
    let days = week_of(oct(19), |date| {
        let mut f = recorded(date);
        f.water_goal_ml = Some(2000);
        match date.day() {
            19 => {
                f.note_routine(RoutineOutcome::Done);
                f.water_ml = 2100;
                f.breaks_offered = 3;
                f.breaks_taken = 3;
            }
            20 => {
                f.note_routine(RoutineOutcome::Partial);
                f.water_ml = 900;
                f.breaks_offered = 2;
                f.breaks_taken = 1;
            }
            21 => return DayFacts::empty(date), // app not running
            22 => {
                f.breaks_offered = 2;
            }
            23 => {
                f.note_routine(RoutineOutcome::Done);
                f.water_ml = 2000;
            }
            24 => {
                f.note_routine(RoutineOutcome::Skipped);
                f.water_ml = 0;
            }
            _ => return DayFacts::empty(date), // Sunday, off
        }
        f
    });
    let w = week(&c, &days, &[], phone_none());
    assert_eq!(w.start, oct(19));
    assert_eq!(w.end, oct(25));
    assert_eq!(w.status, WeekStatus::Normal);
    assert_eq!(
        w.coverage,
        Coverage {
            recorded: 5,
            workdays: 6
        }
    );
    let movement = &w.rows[0];
    assert_eq!(movement.metric, Metric::Movement);
    use DayMark::*;
    assert_eq!(
        movement.marks,
        vec![Met, Partly, NoData, Zero, Met, Zero, Off]
    );
    assert_eq!(movement.value, Ratio::new(2, 5));
    assert_eq!(
        (
            movement.met,
            movement.partly,
            movement.zero,
            movement.no_data
        ),
        (2, 1, 2, 1)
    );
    let breaks = &w.rows[1];
    assert_eq!(
        breaks.marks,
        vec![Met, Partly, NoData, Zero, NothingDue, NothingDue, Off]
    );
    assert_eq!(breaks.value, Ratio::new(4, 7));
    let water = &w.rows[2];
    assert_eq!(water.marks, vec![Met, Partly, NoData, Zero, Met, Zero, Off]);
    assert_eq!(water.value, Ratio::new(2, 5));
    assert_eq!(
        w.highlights,
        vec![
            "Movement routine done on 2 of 5 recorded days, plus 1 partly".to_owned(),
            "Breaks taken: 4 of 7 offered".to_owned(),
            "Water goal reached on 2 of 5 days".to_owned(),
        ]
    );
    assert_eq!(w.previous_week, Some(oct(12)));
    assert_eq!(w.next_week, Some(oct(26)));
    // No previous week was loaded, so no comparison.
    assert_eq!(w.comparison, None);
}

#[test]
fn not_enough_data_and_first_week_variants() {
    // Two of six workdays recorded.
    let c = cal(oct(26), oct(1));
    let days = week_of(oct(19), |date| {
        if matches!(date.day(), 19 | 22) {
            recorded(date)
        } else {
            DayFacts::empty(date)
        }
    });
    let w = week(&c, &days, &[], phone_none());
    assert_eq!(w.status, WeekStatus::NotEnoughData);
    assert_eq!(
        w.coverage,
        Coverage {
            recorded: 2,
            workdays: 6
        }
    );
    assert_eq!(w.rows[0].no_data, 4);
    // Missing days are not counted as misses: only the 2 recorded days count.
    assert_eq!(w.rows[0].value, Ratio::new(0, 2));

    // Started on Wednesday 28 Oct; today is Friday 30 Oct.
    let c = cal(oct(30), oct(28));
    let days = week_of(oct(26), |date| {
        if date >= oct(28) && date <= oct(30) {
            recorded(date)
        } else {
            DayFacts::empty(date)
        }
    });
    let w = week(&c, &days, &[], phone_none());
    assert_eq!(w.status, WeekStatus::FirstWeek);
    use DayMark::*;
    assert_eq!(
        w.rows[0].marks,
        vec![NotStarted, NotStarted, Zero, Zero, Today, Future, Future]
    );
    assert_eq!(
        w.coverage,
        Coverage {
            recorded: 3,
            workdays: 3
        }
    );
    assert_eq!(w.previous_week, None);
    assert_eq!(w.next_week, None);

    // A week entirely before first use.
    let c = cal(oct(30), oct(28));
    let days = week_of(oct(12), DayFacts::empty);
    let w = week(&c, &days, &[], phone_none());
    assert_eq!(w.status, WeekStatus::BeforeStart);
    assert!(w.highlights.is_empty());
}

#[test]
fn comparisons_start_after_two_full_weeks() {
    let full = |start: NaiveDate| {
        week_of(start, |date| {
            let mut f = recorded(date);
            if date.weekday() != Weekday::Sun {
                f.note_routine(RoutineOutcome::Done);
            }
            f
        })
    };
    // In use since Monday 12 Oct: the week of 19 Oct compares with the full week of 12 Oct.
    let c = cal(oct(26), oct(12));
    let w = week(&c, &full(oct(19)), &full(oct(12)), phone_none());
    let cmp = w.comparison.unwrap();
    assert_eq!(cmp.movement, Ratio::new(6, 6));
    assert_eq!(cmp.longest_block_s, Some(2400));
    // In use since Wednesday 14 Oct: the week of 12 Oct was not a full week.
    let c = cal(oct(26), oct(14));
    let w = week(&c, &full(oct(19)), &full(oct(12)), phone_none());
    assert_eq!(w.comparison, None);
    // A previous week with too little data gives no comparison.
    let c = cal(oct(26), oct(1));
    let sparse = week_of(oct(12), DayFacts::empty);
    let w = week(&c, &full(oct(19)), &sparse, phone_none());
    assert_eq!(w.comparison, None);
}

#[test]
fn highlights_are_short_neutral_and_at_most_three() {
    let c = cal(oct(26), oct(1));
    let days = week_of(oct(19), |date| {
        let mut f = recorded(date);
        f.tally = Some(ActivityDay {
            longest_block_s: 6900,
            focus_sessions: 1,
            pomodoro_rounds: 2,
            ..tally(date)
        });
        f
    });
    let w = week(&c, &days, &[], phone_none());
    // Nothing done or logged: no routine, break or water line; the tallies remain.
    assert_eq!(
        w.highlights,
        vec![
            "Longest work block: 1 h 55 min (estimate, this PC)".to_owned(),
            "7 focus sessions and 14 Pomodoro rounds".to_owned(),
        ]
    );
    for h in &w.highlights {
        let lower = h.to_lowercase();
        for word in ["great", "bad", "fail", "should", "missed", "streak"] {
            assert!(!lower.contains(word), "{h:?} contains {word:?}");
        }
    }
    assert_eq!(duration_text(45 * 60), "45 min");
    assert_eq!(duration_text(2 * 3600), "2 h");
    assert_eq!(duration_text(59), "0 min");
}

#[test]
fn phone_counts_merge_by_label() {
    let mut a = DayFacts::empty(oct(26));
    a.phone.add(PhoneLabel::Work);
    a.phone.add(PhoneLabel::Unlabelled);
    let mut b = DayFacts::empty(oct(27));
    b.phone.add(PhoneLabel::Work);
    b.phone.add(PhoneLabel::Personal);
    let c = phone_counts(&[a, b]);
    assert_eq!((c.work, c.personal, c.unsure, c.unlabelled), (2, 1, 0, 1));
    assert_eq!(c.total(), 4);
}

fn facts_for(from: NaiveDate, to: NaiveDate, f: impl Fn(NaiveDate) -> DayFacts) -> Vec<DayFacts> {
    from.iter_days().take_while(|x| *x <= to).map(f).collect()
}

#[test]
fn monthly_bars_rates_and_coverage() {
    // In use since 1 Jun 2026; today is 20 Oct 2026; July had no data at all.
    let c = cal(oct(20), d(2026, 6, 1));
    let sel = MonthKey::new(2026, 10).unwrap();
    let ((from, to), baseline) = month_ranges(&c, sel);
    assert_eq!(from, d(2026, 6, 1)); // five months back is May, before first use
    assert_eq!(to, d(2026, 10, 31));
    assert_eq!(baseline, Some((d(2026, 6, 1), d(2026, 6, 28))));
    let facts = facts_for(from, to, |date| {
        if date.month() == 7 {
            return DayFacts::empty(date);
        }
        let mut f = recorded(date);
        f.water_goal_ml = Some(2000);
        if date.month() == 8 {
            // August: recorded, nothing logged: a measured zero.
            return f;
        }
        if date.day() % 2 == 0 {
            f.note_routine(RoutineOutcome::Done);
            f.water_ml = 2000;
        }
        f
    });
    let base = facts_for(d(2026, 6, 1), d(2026, 6, 28), |date| {
        facts.iter().find(|f| f.date == date).unwrap().clone()
    });
    let m = month(&c, sel, &facts, &base);
    assert_eq!(m.status, MonthStatus::Normal);
    let months: Vec<String> = m.bars.iter().map(|b| b.month.to_string()).collect();
    assert_eq!(
        months,
        ["2026-06", "2026-07", "2026-08", "2026-09", "2026-10"]
    );
    let july = m.bars[1];
    assert_eq!(july.coverage.recorded, 0);
    assert_eq!(july.movement.pct, None); // no data, not 0
    assert_eq!(july.kind.movement, BarKind::NoData);
    let august = m.bars[2];
    assert!(august.coverage.recorded > 0);
    assert_eq!(august.movement.pct, Some(0)); // measured zero
    assert_eq!(august.water.pct, Some(0));
    assert_eq!(august.kind.movement, BarKind::Zero);
    assert_eq!(m.bars[4].kind.movement, BarKind::Measured);
    let october = m.bars[4];
    assert!(october.in_progress);
    assert!(!m.bars[3].in_progress);
    // October so far: 1 to 20, Sundays 4, 11 and 18 off: 17 workdays, all recorded.
    assert_eq!(
        october.coverage,
        Coverage {
            recorded: 17,
            workdays: 17
        }
    );
    // The 17 workdays count (today too: its routine is done), and so do Sundays 4 and 18,
    // when a routine was done; Sunday 11 is off.
    assert_eq!(october.movement, Ratio::new(10, 19)); // even days 2 to 20
    assert_eq!(m.days.len(), 31);
    assert_eq!(m.days[19].movement, DayMark::Met); // 20 Oct, done (even day)
    assert_eq!(m.days[20].movement, DayMark::Future);
    let b = m.baseline.unwrap();
    assert!(b.movement.den > 0);
    assert_eq!(m.previous_month, Some(MonthKey::new(2026, 9).unwrap()));
    assert_eq!(m.next_month, None);
}

#[test]
fn an_off_day_only_month_is_measured_not_no_data() {
    // In use since 1 Jun 2026, Monday to Saturday workdays. August's only record is a
    // Sunday with some water below the goal; September's only record is Sunday 6 Sep,
    // with the routine done and the water goal reached. No workday was recorded in
    // either month, yet those days count (an off day counts once something was done).
    let c = cal(oct(20), d(2026, 6, 1));
    let sel = MonthKey::new(2026, 10).unwrap();
    let ((from, to), _) = month_ranges(&c, sel);
    let facts = facts_for(from, to, |date| {
        let mut f = DayFacts::empty(date);
        if date == d(2026, 8, 2) {
            f.events = 1;
            f.water_entries = 1;
            f.water_ml = 500;
            f.water_goal_ml = Some(2000);
        }
        if date == d(2026, 9, 6) {
            f.events = 3;
            f.note_routine(RoutineOutcome::Done);
            f.water_entries = 2;
            f.water_ml = 2000;
            f.water_goal_ml = Some(2000);
        }
        f
    });
    let m = month(&c, sel, &facts, &[]);
    let bar = |key: &str| *m.bars.iter().find(|b| b.month.to_string() == key).unwrap();
    let sep = bar("2026-09");
    assert_eq!(sep.coverage.recorded, 0);
    assert_eq!(sep.movement, Ratio::new(1, 1));
    assert_eq!(sep.water, Ratio::new(1, 1));
    assert_eq!(sep.kind.movement, BarKind::Measured);
    assert_eq!(sep.kind.water, BarKind::Measured);
    assert_eq!(sep.kind.breaks, BarKind::NoData); // no break offered, no workday recorded
    let aug = bar("2026-08");
    assert_eq!(aug.coverage.recorded, 0);
    assert_eq!(aug.water, Ratio::new(0, 1));
    assert_eq!(aug.kind.water, BarKind::Zero); // a measured zero, not "no data"
    assert_eq!(aug.kind.movement, BarKind::NoData); // the Sunday is off for movement
    let jul = bar("2026-07");
    assert_eq!(jul.kind.movement, BarKind::NoData);
    assert_eq!(jul.kind.water, BarKind::NoData);
    let v = serde_json::to_value(sep).unwrap();
    assert_eq!(v["kind"]["movement"], "measured");
    assert_eq!(v["kind"]["breaks"], "no_data");
    // The same Sunday in the weekly review counts as 1 of 1.
    let w = week(
        &c,
        &week_of(d(2026, 8, 31), |date| {
            facts.iter().find(|f| f.date == date).unwrap().clone()
        }),
        &[],
        phone_none(),
    );
    assert_eq!(w.coverage.recorded, 0);
    assert_eq!(w.rows[0].value, Ratio::new(1, 1));
    assert_eq!(w.rows[0].marks[6], DayMark::Met);
}

#[test]
fn a_month_with_workdays_but_no_goal_has_nothing_to_count() {
    let c = cal(oct(20), d(2026, 6, 1));
    let sel = MonthKey::new(2026, 9).unwrap();
    let ((from, to), _) = month_ranges(&c, sel);
    let facts = facts_for(from, to, |date| {
        let mut f = recorded(date);
        f.water_ml = 300;
        f
    });
    let m = month(&c, sel, &facts, &[]);
    let sep = *m.bars.last().unwrap();
    assert!(sep.coverage.recorded > 0);
    assert_eq!(sep.water.den, 0);
    assert_eq!(sep.kind.water, BarKind::NothingToCount);
    assert_eq!(sep.kind.movement, BarKind::Zero);
}

#[test]
fn monthly_chart_waits_two_weeks_and_baseline_four() {
    let c = cal(oct(20), oct(10));
    let sel = MonthKey::of(oct(20));
    let ((from, to), baseline) = month_ranges(&c, sel);
    assert_eq!(baseline, None);
    let facts = facts_for(from, to, |date| {
        if date >= oct(10) && date <= oct(20) {
            recorded(date)
        } else {
            DayFacts::empty(date)
        }
    });
    let m = month(&c, sel, &facts, &[]);
    assert_eq!(m.status, MonthStatus::TooEarly);
    assert_eq!(m.days_used, 11);
    assert_eq!(m.bars.len(), 1);
    assert_eq!(m.previous_month, None);
    // Exactly 14 days after first use the chart appears.
    let c = cal(oct(24), oct(10));
    assert_eq!(month(&c, sel, &facts, &[]).status, MonthStatus::Normal);
}

#[test]
fn month_keys_walk_across_year_ends_and_month_lengths() {
    let dec = MonthKey::new(2026, 12).unwrap();
    assert_eq!(dec.succ(), MonthKey::new(2027, 1).unwrap());
    assert_eq!(dec.succ().pred(), dec);
    assert_eq!(dec.last_day(), d(2026, 12, 31));
    assert_eq!(MonthKey::new(2028, 2).unwrap().last_day(), d(2028, 2, 29));
    assert_eq!(MonthKey::new(2026, 2).unwrap().last_day(), d(2026, 2, 28));
    assert_eq!(MonthKey::new(2026, 9).unwrap().last_day(), d(2026, 9, 30));
    assert_eq!("2026-10".parse::<MonthKey>().unwrap().first_day(), oct(1));
    for bad in ["2026-13", "2026-1", "202610", "2026-10-01", ""] {
        assert!(bad.parse::<MonthKey>().is_err(), "{bad:?}");
    }
    assert_eq!(
        serde_json::to_string(&MonthKey::new(2026, 3).unwrap()).unwrap(),
        "\"2026-03\""
    );
    // Six months shown, never before the month of first use.
    let c = cal(d(2027, 2, 10), d(2026, 1, 5));
    assert_eq!(
        first_shown_month(&c, MonthKey::new(2027, 2).unwrap()),
        MonthKey::new(2026, 9).unwrap()
    );
    let c = cal(d(2027, 2, 10), d(2026, 12, 5));
    assert_eq!(
        first_shown_month(&c, MonthKey::new(2027, 2).unwrap()),
        MonthKey::new(2026, 12).unwrap()
    );
}

#[test]
fn reviews_serialise_for_the_page() {
    let c = cal(oct(26), oct(1));
    let w = week(&c, &week_of(oct(19), recorded), &[], phone_none());
    let v = serde_json::to_value(&w).unwrap();
    assert_eq!(v["start"], "2026-10-19");
    assert_eq!(v["status"], "normal");
    assert_eq!(v["rows"][0]["metric"], "movement");
    assert_eq!(v["rows"][1]["marks"][0], "nothing_due");
    assert_eq!(v["rows"][0]["value"]["pct"], 0);
    assert_eq!(v["coverage"]["recorded"], 6);
}

#[test]
fn screen_time_is_shown_per_day_and_missing_is_no_data() {
    // DEC13: each day's screen time, from the tally; a day without one is no data.
    let c = cal(oct(22), oct(1));
    let days = week_of(oct(19), |date| {
        if date == oct(20) {
            let mut f = recorded(date);
            f.tally = Some(ActivityDay {
                screen_s: None,
                ..tally(date)
            });
            f
        } else if date <= oct(22) {
            recorded(date)
        } else {
            DayFacts::empty(date)
        }
    });
    let w = week(&c, &days, &[], phone_none());
    let screen: Vec<_> = w.days.iter().map(|d| d.screen_s).collect();
    assert_eq!(
        screen,
        vec![Some(4500), None, Some(4500), Some(4500), None, None, None]
    );
    let sel = MonthKey::of(oct(22));
    let ((from, to), _) = month_ranges(&c, sel);
    let facts = facts_for(from, to, |date| {
        if date <= oct(22) {
            recorded(date)
        } else {
            DayFacts::empty(date)
        }
    });
    let m = month(&c, sel, &facts, &[]);
    assert_eq!(m.days[0].screen_s, Some(4500));
    assert_eq!(m.days[30].screen_s, None);
}
