//! The weekly and monthly reviews over a real store (slice V1-4): corrections and
//! removals, reminder answers, missing days against measured zeros, goals judged by
//! date, week and month boundaries, a time-zone change, and relabelling a past phone
//! report.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use chrono::{DateTime, Days, NaiveDate, Utc};
use hy_core::Error;
use hy_core::clock::ManualClock;
use hy_core::coach::DayTally;
use hy_core::review::{DayMark, Metric, MonthStatus, Ratio, WeekStatus};
use hy_core::schema::{
    Activity, EventPayload, GoalKind, MeasurementStatus, NewEvent, PhoneLabel, ReminderOutcome,
    RoutineOutcome, RoutineVariant,
};
use hy_core::store::{CoachCommit, MonthKey, Store};
use hy_core::time::UserZone;
use uuid::Uuid;

/// IST wall time on a date in 2026.
fn ist(month: u32, day: u32, h: u32, m: u32) -> DateTime<Utc> {
    UserZone::KOLKATA.resolve_local(
        NaiveDate::from_ymd_opt(2026, month, day)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap(),
    )
}

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

struct Env {
    store: Store,
    clock: Arc<ManualClock>,
    revision: Option<u64>,
}

impl Env {
    /// A store created at `created` (its first day of use), with the clock then moved to
    /// `now`.
    fn new(created: DateTime<Utc>, now: DateTime<Utc>) -> Self {
        let clock = Arc::new(ManualClock::new(created));
        let store = Store::open_in_memory(clock.clone()).unwrap();
        clock.set_wall(now);
        Self {
            store,
            clock,
            revision: None,
        }
    }

    /// Record a tracker tally for `day` (the PC recorded the day).
    fn track(&mut self, day: NaiveDate) {
        let tally = DayTally {
            pc_active_ms: 3_600_000,
            work_active_ms: 3_000_000,
            longest_block_ms: 1_800_000,
            confirmed_breaks: 1,
            ..DayTally::default()
        };
        let rev = self
            .store
            .commit_coach_step(CoachCommit {
                expected_revision: self.revision,
                format: 1,
                engine_time_ms: 0,
                state: "{}",
                events: &[],
                days: &[(day, tally)],
            })
            .unwrap();
        self.revision = Some(rev);
    }

    fn add(&mut self, e: NewEvent) -> Uuid {
        self.store.append_event(e).unwrap().id
    }

    fn water(&mut self, at: DateTime<Utc>, ml: u32) -> Uuid {
        self.add(NewEvent::manual(EventPayload::WaterLogged { amount_ml: ml }).at(at))
    }

    fn phone(&mut self, at: DateTime<Utc>) -> Uuid {
        self.add(
            NewEvent::manual(EventPayload::PhoneInterruption {
                label: PhoneLabel::Unlabelled,
            })
            .at(at),
        )
    }

    fn routine(&mut self, at: DateTime<Utc>, outcome: RoutineOutcome) {
        let (done, variant) = match outcome {
            RoutineOutcome::Done => (3, Some(RoutineVariant::Seated)),
            RoutineOutcome::Partial => (1, Some(RoutineVariant::Seated)),
            RoutineOutcome::None => (0, Some(RoutineVariant::Standing)),
            RoutineOutcome::Skipped => (0, None),
        };
        self.add(
            NewEvent::manual(EventPayload::MovementSession {
                session_id: Uuid::now_v7(),
                routine_id: "mon".into(),
                routine_version: 1,
                variant,
                outcome,
                steps_done: done,
                steps_skipped: 0,
                steps_total: 3,
            })
            .at(at),
        );
    }

    fn reminder(&mut self, at: DateTime<Utc>, activity: Activity, outcome: ReminderOutcome) {
        let reminder_id = Uuid::now_v7();
        self.add(
            NewEvent::inferred(
                EventPayload::ReminderOffered {
                    reminder_id,
                    activity,
                },
                MeasurementStatus::Measured,
            )
            .at(at),
        );
        let payload = EventPayload::ReminderResponse {
            reminder_id,
            activity,
            outcome,
        };
        let e = if outcome == ReminderOutcome::NoResponse {
            NewEvent::inferred(payload, MeasurementStatus::Measured)
        } else {
            NewEvent::manual(payload)
        };
        self.add(e.at(at));
    }

    fn facts(&self, day: NaiveDate) -> hy_core::review::DayFacts {
        self.store.day_facts(day, day).unwrap().remove(0)
    }
}

#[test]
fn corrections_and_removals_change_the_review() {
    // Created Monday 19 Oct; now Thursday 22 Oct.
    let mut env = Env::new(ist(10, 19, 9, 0), ist(10, 22, 17, 0));
    env.store
        .set_goal(GoalKind::WaterDailyMl, 1000, date(10, 19))
        .unwrap();
    env.track(date(10, 20));
    let a = env.water(ist(10, 20, 10, 0), 250);
    let b = env.water(ist(10, 20, 11, 0), 300);
    // Correct a to 400 ml, then correct the correction to 450 ml; remove b.
    let a2 = env.add(NewEvent::manual(EventPayload::WaterLogged { amount_ml: 400 }).superseding(a));
    env.add(NewEvent::manual(EventPayload::WaterLogged { amount_ml: 450 }).superseding(a2));
    env.add(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(b));
    let f = env.facts(date(10, 20));
    assert_eq!((f.water_ml, f.water_entries), (450, 1));
    assert_eq!(f.water_ml, env.store.water_total_on(date(10, 20)).unwrap());

    // Two phone notes: one labelled from the review, one removed.
    let p1 = env.phone(ist(10, 20, 12, 0));
    let p2 = env.phone(ist(10, 20, 15, 0));
    let p1b = env
        .store
        .relabel_phone(p1, PhoneLabel::Work)
        .unwrap()
        .unwrap();
    env.add(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(p2));
    let f = env.facts(date(10, 20));
    assert_eq!(
        (f.phone.work, f.phone.unlabelled, f.phone.total()),
        (1, 0, 1)
    );

    let w = env.store.week_review(None).unwrap();
    assert_eq!(w.start, date(10, 19));
    let water = w.rows.iter().find(|r| r.metric == Metric::Water).unwrap();
    // Monday not recorded (the store was created then, but nothing ran): no data.
    assert_eq!(water.marks[0], DayMark::NoData);
    assert_eq!(water.marks[1], DayMark::Partly);
    assert_eq!(water.marks[3], DayMark::Today);
    assert_eq!(w.phone.entries.len(), 1);
    assert_eq!(w.phone.entries[0].id, p1b);
    assert_eq!(w.phone.entries[0].label, PhoneLabel::Work);
    assert_eq!(w.phone.entries[0].date, date(10, 20));
    assert!(w.phone.show);
    assert_eq!(w.phone.counts.work, 1);

    // 550 ml more reaches the goal: the day becomes a goal day.
    env.water(ist(10, 20, 16, 0), 550);
    let w = env.store.week_review(None).unwrap();
    let water = w.rows.iter().find(|r| r.metric == Metric::Water).unwrap();
    assert_eq!(water.marks[1], DayMark::Met);
    assert_eq!(water.value, Ratio::new(1, 1));
}

#[test]
fn relabelling_a_phone_report_follows_the_correction_rules() {
    let mut env = Env::new(ist(10, 19, 9, 0), ist(10, 22, 17, 0));
    let p = env.phone(ist(10, 20, 12, 0));
    // The same label changes nothing.
    assert_eq!(
        env.store.relabel_phone(p, PhoneLabel::Unlabelled).unwrap(),
        None
    );
    let p2 = env
        .store
        .relabel_phone(p, PhoneLabel::Personal)
        .unwrap()
        .unwrap();
    // The report keeps its date and time.
    let e = env.store.event(p2).unwrap().unwrap();
    assert_eq!(e.local_date, date(10, 20));
    assert_eq!(e.occurred_at, ist(10, 20, 12, 0));
    assert_eq!(e.supersedes, Some(p));
    // The old version cannot be changed again; the newest can.
    assert!(matches!(
        env.store.relabel_phone(p, PhoneLabel::Work),
        Err(Error::Invalid(_))
    ));
    env.store
        .relabel_phone(p2, PhoneLabel::Unsure)
        .unwrap()
        .unwrap();
    // Only phone reports, and only existing ones.
    let w = env.water(ist(10, 20, 13, 0), 200);
    assert!(matches!(
        env.store.relabel_phone(w, PhoneLabel::Work),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        env.store.relabel_phone(Uuid::now_v7(), PhoneLabel::Work),
        Err(Error::NotFound(_))
    ));
    // A removed report cannot be relabelled.
    let p3 = env.phone(ist(10, 21, 9, 0));
    env.add(NewEvent::manual(EventPayload::EntryRetracted {}).superseding(p3));
    assert!(env.store.relabel_phone(p3, PhoneLabel::Work).is_err());
}

#[test]
fn breaks_count_answers_to_movement_and_eye_rest_without_snoozes() {
    let mut env = Env::new(ist(10, 19, 9, 0), ist(10, 22, 17, 0));
    let t = ist(10, 20, 11, 0);
    env.reminder(t, Activity::Movement, ReminderOutcome::Done);
    env.reminder(t, Activity::EyeRest, ReminderOutcome::Started);
    env.reminder(t, Activity::Movement, ReminderOutcome::Snoozed);
    env.reminder(t, Activity::EyeRest, ReminderOutcome::Dismissed);
    env.reminder(t, Activity::Movement, ReminderOutcome::NoResponse);
    env.reminder(t, Activity::Movement, ReminderOutcome::Skipped);
    // Water is not a break.
    env.reminder(t, Activity::Water, ReminderOutcome::Done);
    let f = env.facts(date(10, 20));
    assert_eq!((f.breaks_offered, f.breaks_taken), (5, 2));
    let w = env.store.week_review(None).unwrap();
    let breaks = w.rows.iter().find(|r| r.metric == Metric::Breaks).unwrap();
    assert_eq!(breaks.marks[1], DayMark::Partly);
    assert_eq!(breaks.value, Ratio::new(2, 5));
}

#[test]
fn routines_keep_the_best_result_of_each_day() {
    let mut env = Env::new(ist(10, 19, 9, 0), ist(10, 24, 17, 0));
    env.routine(ist(10, 19, 11, 0), RoutineOutcome::Skipped);
    env.routine(ist(10, 20, 11, 0), RoutineOutcome::None);
    env.routine(ist(10, 20, 15, 0), RoutineOutcome::Partial);
    env.routine(ist(10, 21, 11, 0), RoutineOutcome::Partial);
    env.routine(ist(10, 21, 15, 0), RoutineOutcome::Done);
    env.track(date(10, 22));
    let w = env.store.week_review(None).unwrap();
    let m = &w
        .rows
        .iter()
        .find(|r| r.metric == Metric::Movement)
        .unwrap();
    use DayMark::*;
    assert_eq!(&m.marks[..6], &[Zero, Partly, Met, Zero, NoData, Today]);
    assert_eq!(m.value, Ratio::new(1, 4));
}

#[test]
fn goals_are_judged_by_the_goal_in_force_that_day() {
    let mut env = Env::new(ist(10, 19, 9, 0), ist(10, 24, 17, 0));
    env.store
        .set_goal(GoalKind::WaterDailyMl, 2000, date(10, 19))
        .unwrap();
    env.store
        .set_goal(GoalKind::WaterDailyMl, 1500, date(10, 21))
        .unwrap();
    for day in 19..=23 {
        env.water(ist(10, day, 12, 0), 1600);
    }
    let w = env.store.week_review(None).unwrap();
    let water = w.rows.iter().find(|r| r.metric == Metric::Water).unwrap();
    use DayMark::*;
    assert_eq!(&water.marks[..5], &[Partly, Partly, Met, Met, Met]);
    assert_eq!(water.value, Ratio::new(3, 5));
}

#[test]
fn week_and_month_boundaries_follow_local_dates() {
    // Sunday 1 Nov 23:59 IST and Monday 2 Nov 00:01 IST are in different weeks; Saturday 31
    // Oct and Sunday 1 Nov in different months.
    let mut env = Env::new(ist(10, 26, 9, 0), ist(11, 3, 12, 0));
    env.water(ist(10, 31, 23, 59), 100);
    env.water(ist(11, 1, 23, 59), 200);
    env.water(ist(11, 2, 0, 1), 400);
    let this = env.store.week_review(None).unwrap();
    assert_eq!(this.start, date(11, 2));
    let last = env.store.week_review(Some(date(10, 28))).unwrap();
    assert_eq!(last.start, date(10, 26));
    assert_eq!(last.end, date(11, 1));
    let f = env.store.day_facts(date(10, 31), date(11, 2)).unwrap();
    let ml: Vec<u32> = f.iter().map(|d| d.water_ml).collect();
    assert_eq!(ml, [100, 200, 400]);
    let water = last
        .rows
        .iter()
        .find(|r| r.metric == Metric::Water)
        .unwrap();
    // No goal set: some water is "partly", and the Sunday with water shows too.
    assert_eq!(water.marks[5], DayMark::Partly);
    assert_eq!(water.marks[6], DayMark::Partly);

    let nov = env.store.month_review(None).unwrap();
    assert_eq!(nov.month, MonthKey::new(2026, 11).unwrap());
    assert_eq!(nov.days.len(), 30);
    assert_eq!(nov.days[0].water, DayMark::Partly);
    let oct = env
        .store
        .month_review(Some(MonthKey::new(2026, 10).unwrap()))
        .unwrap();
    assert_eq!(oct.days.len(), 31);
    assert_eq!(oct.days[30].water, DayMark::Partly);
    assert_eq!(oct.next_month, Some(MonthKey::new(2026, 11).unwrap()));
    // Months before first use or after today are not offered.
    assert_eq!(oct.previous_month, None);
    assert_eq!(nov.next_month, None);
    assert_eq!(oct.status, MonthStatus::TooEarly);
}

#[test]
fn a_time_zone_change_never_moves_recorded_days() {
    // 23:30 IST on Tuesday 20 Oct is 19:00 in London the same day, but 00:30 IST on
    // Wednesday 21 Oct is 20:00 in London on Tuesday. Each entry keeps the date it was
    // recorded with.
    let mut env = Env::new(ist(10, 19, 9, 0), ist(10, 22, 12, 0));
    env.water(ist(10, 20, 23, 30), 300);
    env.water(ist(10, 21, 0, 30), 500);
    env.store
        .set_user_zone(UserZone::parse("Europe/London").unwrap())
        .unwrap();
    let f = env.store.day_facts(date(10, 20), date(10, 21)).unwrap();
    assert_eq!((f[0].water_ml, f[1].water_ml), (300, 500));
    // Today now follows the new zone: 06:30 UTC on 22 Oct is 07:30 in London.
    let w = env.store.week_review(None).unwrap();
    assert_eq!(w.today, date(10, 22));
    // An entry recorded in London lands on the London date: 20:00 UTC on 22 Oct is 21:00
    // in London on 22 Oct (it would be 01:30 on 23 Oct in Kolkata).
    let utc = |h, m| {
        DateTime::<Utc>::from_naive_utc_and_offset(date(10, 22).and_hms_opt(h, m, 0).unwrap(), Utc)
    };
    env.clock.set_wall(utc(20, 30));
    env.water(utc(20, 0), 250);
    let f = env.store.day_facts(date(10, 22), date(10, 23)).unwrap();
    assert_eq!((f[0].water_ml, f[1].water_ml), (250, 0));
}

#[test]
fn reviews_go_back_only_to_first_use_and_retention() {
    // Created Wednesday 21 Oct: the first week.
    let mut env = Env::new(ist(10, 21, 9, 0), ist(10, 23, 12, 0));
    env.track(date(10, 21));
    env.track(date(10, 22));
    let w = env.store.week_review(None).unwrap();
    assert_eq!(w.status, WeekStatus::FirstWeek);
    assert_eq!(w.previous_week, None);
    assert_eq!(w.next_week, None);
    // Asking for an earlier or a later week gives the nearest one available.
    assert_eq!(
        env.store.week_review(Some(date(9, 1))).unwrap().start,
        date(10, 19)
    );
    assert_eq!(
        env.store.week_review(Some(date(12, 1))).unwrap().start,
        date(10, 19)
    );
    assert_eq!(env.store.first_use_date().unwrap(), date(10, 21));
    // A backfilled entry from before the store existed moves the first day back.
    env.water(ist(10, 12, 12, 0), 200);
    assert_eq!(env.store.first_use_date().unwrap(), date(10, 12));
    let w = env.store.week_review(None).unwrap();
    assert_eq!(w.previous_week, Some(date(10, 12)));
}

#[test]
fn day_facts_refuses_unreasonable_ranges() {
    let env = Env::new(ist(10, 21, 9, 0), ist(10, 23, 12, 0));
    assert!(env.store.day_facts(date(10, 23), date(10, 22)).is_err());
    let far = date(10, 1).checked_add_days(Days::new(900)).unwrap();
    assert!(env.store.day_facts(date(10, 1), far).is_err());
    assert_eq!(
        env.store
            .day_facts(date(10, 1), date(10, 31))
            .unwrap()
            .len(),
        31
    );
}
