//! Deterministic trace tests for the coaching engine (architecture 11: T-R1 to T-R6, the
//! one-at-a-time reminder of prototype rev 3, suppression and precedence, focus, Pomodoro,
//! clock changes and day boundaries). Time is replayed step by step; nothing sleeps.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use hy_core::coach::config::{CALM_GAP, Categories, LongDayCheckIn, SNOOZE};
use hy_core::coach::*;
use hy_core::schema::{
    Activity, EventPayload, NewEvent, ReminderOutcome, RoutineOutcome, SessionState,
};
use hy_core::time::{UserZone, WeekdaySet, WorkSchedule};
use uuid::Uuid;

fn mins(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// Local IST time on a date in Oct 2026 (20 Oct is a Tuesday).
fn ist(day: u32, h: u32, m: u32) -> DateTime<Utc> {
    UserZone::KOLKATA.resolve_local(
        NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap(),
    )
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

/// Trace harness: a simulated user, wall clock and store snapshot around a pure engine.
struct H {
    e: Engine,
    wall: DateTime<Utc>,
    zone: UserZone,
    schedule: WorkSchedule,
    config: CoachConfig,
    water: WaterSnapshot,
    idle: Duration,
    active: bool,
    tick: Duration,
    last: StepOutput,
    events: Vec<NewEvent>,
    notices: Vec<(Duration, Notice)>,
}

impl H {
    fn at(wall: DateTime<Utc>, config: CoachConfig, water: WaterSnapshot) -> Self {
        let mut e = Engine::new(0x5eed);
        let zone = UserZone::KOLKATA;
        let schedule = WorkSchedule::default();
        let ctx = Context {
            wall,
            elapsed: Duration::ZERO,
            zone,
            schedule: schedule.clone(),
            water,
            config,
        };
        let last = e.step(&ctx, Input::Refresh);
        let events = last.events.clone();
        Self {
            e,
            wall,
            zone,
            schedule,
            config,
            water,
            idle: Duration::ZERO,
            active: true,
            tick: secs(5),
            last,
            events,
            notices: Vec::new(),
        }
    }

    /// Tuesday 20 Oct 2026, 10:00, no water goal, Balanced.
    fn new() -> Self {
        Self::at(
            ist(20, 10, 0),
            CoachConfig::default(),
            WaterSnapshot::default(),
        )
    }

    fn only(movement: bool, eye: bool, water: bool) -> Self {
        Self::at(
            ist(20, 10, 0),
            cats(movement, eye, water),
            WaterSnapshot::default(),
        )
    }

    fn ctx(&self, elapsed: Duration) -> Context {
        Context {
            wall: self.wall,
            elapsed,
            zone: self.zone,
            schedule: self.schedule.clone(),
            water: self.water,
            config: self.config,
        }
    }

    fn step(&mut self, elapsed: Duration, input: Input) -> StepOutput {
        self.wall += chrono::Duration::from_std(elapsed).unwrap();
        let out = self.e.step(&self.ctx(elapsed), input);
        self.record(&out);
        out
    }

    fn record(&mut self, out: &StepOutput) {
        for ev in &out.events {
            if let EventPayload::WaterLogged { amount_ml } = ev.payload {
                self.water.logged_ml += amount_ml;
            }
        }
        self.events.extend(out.events.iter().cloned());
        let t = self.e.engine_time();
        self.notices
            .extend(out.notices.iter().cloned().map(|n| (t, n)));
        self.last = out.clone();
    }

    /// An input arriving now (no time passes).
    fn input(&mut self, input: Input) -> StepOutput {
        if !matches!(
            input,
            Input::Lock | Input::Unlock | Input::Suspend | Input::Resume
        ) && !matches!(
            input,
            Input::PresentationChanged(_) | Input::MediaInUseChanged(_)
        ) {
            self.idle = Duration::ZERO;
        }
        self.step(Duration::ZERO, input)
    }

    /// Let time pass in ticks, with the user typing (`active`) or not.
    fn wait(&mut self, d: Duration) {
        let mut left = d;
        while left > Duration::ZERO {
            let t = left.min(self.tick);
            left -= t;
            self.idle = if self.active {
                Duration::ZERO
            } else {
                self.idle + t
            };
            let idle = self.idle;
            self.step(t, Input::Tick { idle });
        }
    }

    /// Tick until `pred` holds; returns how long it took. Panics after `max`.
    fn wait_until(&mut self, max: Duration, pred: impl Fn(&CoachView) -> bool) -> Duration {
        let mut waited = Duration::ZERO;
        while !pred(self.view()) {
            assert!(waited < max, "condition not met within {max:?}");
            self.wait(self.tick);
            waited += self.tick;
        }
        waited
    }

    fn view(&self) -> &CoachView {
        &self.last.view
    }

    fn local(&self) -> NaiveTime {
        self.zone.local_datetime(self.wall).time()
    }

    fn today(&self) -> NaiveDate {
        self.zone.local_date(self.wall)
    }

    fn shown(&self) -> Option<Activity> {
        self.view().reminder().map(|(_, a)| a)
    }

    fn offer(&self) -> Uuid {
        self.view().reminder().expect("a reminder is shown").0
    }

    fn respond(&mut self, response: Response) -> StepOutput {
        let offer = self.offer();
        self.input(Input::Respond { offer, response })
    }

    fn status(&self, a: Activity) -> ChipStatus {
        self.view().chip(a).status
    }

    fn accum(&self, a: Activity) -> Duration {
        Duration::from_millis(self.e.activity(a).accum_ms)
    }

    fn offers(&self) -> Vec<Activity> {
        self.events
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::ReminderOffered { activity, .. } => Some(activity),
                _ => None,
            })
            .collect()
    }

    fn responses(&self) -> Vec<(Activity, ReminderOutcome)> {
        self.events
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::ReminderResponse {
                    activity, outcome, ..
                } => Some((activity, outcome)),
                _ => None,
            })
            .collect()
    }

    fn count_notices(&self, f: impl Fn(&Notice) -> bool) -> usize {
        self.notices.iter().filter(|(_, n)| f(n)).count()
    }
}

fn cats(movement: bool, eye_rest: bool, water: bool) -> CoachConfig {
    CoachConfig {
        categories: Categories {
            movement,
            eye_rest,
            water,
        },
        ..CoachConfig::default()
    }
}

fn water_only_at(wall: DateTime<Utc>, logged_ml: u32) -> H {
    H::at(
        wall,
        cats(false, false, true),
        WaterSnapshot {
            logged_ml,
            goal_ml: Some(2000),
        },
    )
}

// ------------------------------------------------------- one reminder at a time

#[test]
fn eye_rest_then_movement_one_at_a_time_without_burst() {
    let mut h = H::only(true, true, false);
    // Eye rest falls due after 20 minutes of screen time.
    h.wait_until(mins(25), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 20));
    assert_eq!(h.shown(), Some(Activity::EyeRest));
    assert_eq!(h.status(Activity::Movement), ChipStatus::NotDue);
    assert_eq!(h.status(Activity::Water), ChipStatus::Off);
    // Snooze eye rest: it returns once, ten minutes later (the calm gap ends earlier).
    h.respond(Response::Snooze);
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Snoozed);
    assert_eq!(h.view().chip(Activity::EyeRest).snoozed_for, Some(SNOOZE));
    h.wait_until(mins(15), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 30));
    assert_eq!(h.shown(), Some(Activity::EyeRest));
    h.respond(Response::Done);
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Done);
    // Movement is due at 10:45 (45 min of work).
    h.wait_until(mins(20), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 45));
    assert_eq!(h.shown(), Some(Activity::Movement));
    match h.view().bubble {
        Some(Bubble::Reminder {
            reason: ReminderReason::Movement { worked },
            ..
        }) => assert_eq!(worked, mins(45)),
        other => panic!("unexpected bubble {other:?}"),
    }
    // Eye rest falls due again at 10:50 while movement is open: it waits its turn.
    h.wait(mins(6));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Due);
    assert_eq!(h.shown(), Some(Activity::Movement));
    // Answering movement does not mark eye rest done; eye rest comes after the calm gap.
    h.respond(Response::Done);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Done);
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Due);
    assert_eq!(h.view().bubble, None);
    let waited = h.wait_until(mins(10), |v| v.reminder().is_some());
    assert_eq!(waited, CALM_GAP);
    assert_eq!(h.shown(), Some(Activity::EyeRest));
    assert_eq!(
        h.offers(),
        vec![
            Activity::EyeRest,
            Activity::EyeRest,
            Activity::Movement,
            Activity::EyeRest
        ]
    );
}

/// Movement showing at 10:51 while eye rest is due and waiting its turn.
fn movement_showing_eye_rest_waiting() -> H {
    let mut h = H::only(true, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some()); // eye rest 10:20
    h.respond(Response::Snooze);
    h.wait_until(mins(15), |v| v.reminder().is_some()); // eye rest again 10:30
    h.respond(Response::Dismiss);
    h.wait_until(mins(20), |v| v.reminder().is_some()); // movement 10:45
    assert_eq!(h.shown(), Some(Activity::Movement));
    h.wait(mins(6));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Due);
    h
}

#[test]
fn each_answer_changes_only_its_own_activity() {
    for response in [
        Response::Done,
        Response::Start,
        Response::Snooze,
        Response::Skip,
        Response::Dismiss,
    ] {
        let mut h = movement_showing_eye_rest_waiting();
        let others = [Activity::EyeRest, Activity::Water];
        let before = others.map(|a| (*h.e.activity(a), h.status(a)));
        h.respond(response);
        let after = others.map(|a| (*h.e.activity(a), h.status(a)));
        assert_eq!(
            before, after,
            "{response:?} on movement changed another activity"
        );
        assert_ne!(h.status(Activity::Movement), ChipStatus::Showing);
    }
}

#[test]
fn calm_gap_follows_every_kind_of_answer() {
    for response in [
        Response::Done,
        Response::Snooze,
        Response::Skip,
        Response::Dismiss,
    ] {
        let mut h = movement_showing_eye_rest_waiting();
        h.respond(response);
        assert_eq!(h.view().bubble, None);
        let waited = h.wait_until(mins(10), |v| v.reminder().is_some());
        assert_eq!(waited, CALM_GAP, "{response:?}");
        assert_eq!(h.shown(), Some(Activity::EyeRest));
    }
}

#[test]
fn snooze_returns_once_and_only_that_activity() {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 45));
    h.respond(Response::Snooze);
    assert_eq!(h.view().pet, PetMode::Snoozed(Activity::Movement));
    h.wait_until(mins(11), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 55));
    // The snooze was used: answering "Not now" now gives a full cycle, not another snooze.
    h.respond(Response::Dismiss);
    assert_eq!(h.e.activity(Activity::Movement).snooze_until, None);
    h.wait(mins(44));
    assert_eq!(h.view().bubble, None);
    h.wait_until(mins(2), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(11, 40));
    assert_eq!(
        h.responses(),
        vec![
            (Activity::Movement, ReminderOutcome::Snoozed),
            (Activity::Movement, ReminderOutcome::Dismissed)
        ]
    );
}

#[test]
fn closing_without_an_answer_leaves_a_waiting_badge_not_another_popup() {
    let mut h = H::only(true, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some()); // eye rest at 10:20
    let offer = h.offer();
    h.input(Input::CloseBubble);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.view().pet, PetMode::Waiting(Activity::EyeRest));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Waiting);
    // Clicking the pet reopens the same reminder.
    h.wait(mins(3));
    h.input(Input::OpenPending);
    assert_eq!(h.offer(), offer);
    h.input(Input::CloseBubble);
    // No other bubble while the badge waits; the badge closes 20 min after first shown.
    h.wait(mins(16));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Waiting);
    assert_eq!(h.offers().len(), 1);
    h.wait(mins(1) + secs(5));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::NoResponse);
    assert_eq!(
        h.responses(),
        vec![(Activity::EyeRest, ReminderOutcome::NoResponse)]
    );
    assert_eq!(h.view().pet, PetMode::Idle);
}

#[test]
fn an_unanswered_reminder_collapses_after_ten_minutes() {
    let mut h = H::only(false, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some());
    h.wait(mins(10) - secs(5));
    assert!(h.view().reminder().is_some());
    h.wait(secs(5));
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.view().pet, PetMode::Waiting(Activity::EyeRest));
    // Logged as no response only when the badge closes; then a full cycle follows.
    assert!(h.responses().is_empty());
    h.wait(mins(10));
    assert_eq!(
        h.responses(),
        vec![(Activity::EyeRest, ReminderOutcome::NoResponse)]
    );
    // Next eye rest after a further 20 minutes of screen time (not straight away).
    let waited = h.wait_until(mins(30), |v| v.reminder().is_some());
    assert_eq!(waited, mins(20));
}

#[test]
fn stale_or_duplicate_answers_change_nothing() {
    let mut h = H::only(true, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some());
    let offer = h.offer();
    let out = h.input(Input::Respond {
        offer: Uuid::nil(),
        response: Response::Done,
    });
    assert_eq!(out.outcome, InputOutcome::Ignored("stale reminder"));
    assert!(out.events.is_empty());
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Showing);
    h.respond(Response::Done);
    let again = h.input(Input::Respond {
        offer,
        response: Response::Skip,
    });
    assert_eq!(again.outcome, InputOutcome::Ignored("stale reminder"));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Done);
    assert_eq!(h.dismissals(), 0);
    // Water and Start mismatches are refused.
    h.wait_until(mins(30), |v| v.reminder().is_some());
    let out = h.respond(Response::LogWater { ml: 250 });
    assert_eq!(
        out.outcome,
        InputOutcome::Ignored("only a water reminder logs water")
    );
    assert_eq!(
        h.input(Input::Unlock).outcome,
        InputOutcome::Ignored("not locked or asleep")
    );
    assert_eq!(h.input(Input::CloseBubble).outcome, InputOutcome::Applied);
    assert_eq!(
        h.input(Input::RoutineFinished {
            offer: Some(Uuid::nil()),
            outcome: RoutineOutcome::Done
        })
        .outcome,
        InputOutcome::Ignored("stale reminder")
    );
}

impl H {
    fn dismissals(&self) -> u32 {
        self.e.dismissals(self.today())
    }
}

// ------------------------------------------------------------- routine outcomes

/// Movement offered at 10:45 with 45 min of work, started, then the routine reports
/// `outcome` one minute later.
fn routine(outcome: RoutineOutcome) -> H {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    assert_eq!(h.shown(), Some(Activity::Movement));
    let offer = h.offer();
    h.respond(Response::Start);
    assert_eq!(h.status(Activity::Movement), ChipStatus::InProgress);
    assert_eq!(h.view().pet, PetMode::Exercise);
    assert_eq!(h.view().bubble, None);
    h.wait(mins(1));
    let out = h.input(Input::RoutineFinished {
        offer: Some(offer),
        outcome,
    });
    assert_eq!(out.outcome, InputOutcome::Applied);
    assert_eq!(
        h.responses(),
        vec![(Activity::Movement, ReminderOutcome::Started)]
    );
    h
}

/// Wait for the next movement offer and return (local time, work time in its reason).
fn next_movement(h: &mut H) -> (NaiveTime, Duration) {
    h.wait_until(mins(120), |v| v.reminder().is_some());
    match h.view().bubble {
        Some(Bubble::Reminder {
            activity: Activity::Movement,
            reason: ReminderReason::Movement { worked },
            ..
        }) => (h.local(), worked),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn routine_stopped_immediately_is_none_keeps_timer_and_is_rescheduled() {
    let mut h = routine(RoutineOutcome::None);
    assert_eq!(h.status(Activity::Movement), ChipStatus::NoneRecorded);
    // The movement timer is NOT reset: 46 minutes of work are still counted.
    assert_eq!(h.accum(Activity::Movement), mins(46));
    // The calm gap applies and movement is rescheduled a full cycle later.
    assert_eq!(h.view().bubble, None);
    let (at, worked) = next_movement(&mut h);
    assert_eq!(at, hm(11, 31));
    assert_eq!(worked, mins(91));
}

#[test]
fn routine_with_every_step_skipped_is_none() {
    // The guide reports zero steps done as `none` whatever was skipped.
    let mut h = routine(RoutineOutcome::None);
    assert_eq!(h.status(Activity::Movement), ChipStatus::NoneRecorded);
    assert_eq!(h.accum(Activity::Movement), mins(46));
    assert_eq!(h.dismissals(), 0);
    assert_eq!(next_movement(&mut h).0, hm(11, 31));
}

#[test]
fn routine_with_one_step_done_then_stop_is_partial_and_resets_timer() {
    let mut h = routine(RoutineOutcome::Partial);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    let (at, worked) = next_movement(&mut h);
    assert_eq!(at, hm(11, 31));
    assert_eq!(worked, mins(45));
}

#[test]
fn routine_with_every_step_done_is_done_and_resets_timer() {
    let mut h = routine(RoutineOutcome::Done);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Done);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(
        h.count_notices(|n| *n == Notice::Celebrate(Activity::Movement)),
        1
    );
    let (at, worked) = next_movement(&mut h);
    assert_eq!(at, hm(11, 31));
    assert_eq!(worked, mins(45));
}

#[test]
fn routine_skip_today_counts_as_a_skip() {
    let mut h = routine(RoutineOutcome::Skipped);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Skipped);
    assert_eq!(h.accum(Activity::Movement), mins(46));
    assert_eq!(h.dismissals(), 1);
    assert_eq!(next_movement(&mut h).0, hm(11, 31));
}

#[test]
fn routine_later_snoozes_movement_and_timeout_abandons() {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    let offer = h.offer();
    h.respond(Response::Start);
    h.input(Input::RoutineLater { offer });
    assert_eq!(h.status(Activity::Movement), ChipStatus::Snoozed);
    h.wait_until(mins(11), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 55));
    // Started and never reported back: after 30 minutes it is "none", without a record.
    h.respond(Response::Start);
    h.wait(mins(30));
    assert_eq!(h.status(Activity::Movement), ChipStatus::NoneRecorded);
    assert!(h.accum(Activity::Movement) > mins(45));
    assert_eq!(h.view().pet, PetMode::Idle);
}

#[test]
fn routine_from_the_menu_holds_reminders_and_resets_on_done() {
    let mut h = H::only(true, true, false);
    h.wait(mins(15));
    h.input(Input::RoutineStarted);
    assert_eq!(h.view().hold, Some(Hold::ExerciseOrPomodoro));
    assert_eq!(h.view().pet, PetMode::Exercise);
    h.wait(mins(10)); // eye rest would be due at 10:20
    assert_eq!(h.view().bubble, None);
    h.input(Input::RoutineFinished {
        offer: None,
        outcome: RoutineOutcome::Partial,
    });
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);
    // Eye rest is not marked done by the movement routine.
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Due);
    assert_eq!(h.wait_until(mins(10), |v| v.reminder().is_some()), CALM_GAP);
    assert_eq!(h.shown(), Some(Activity::EyeRest));
}

#[test]
fn eye_rest_start_runs_twenty_seconds_then_counts_done() {
    let mut h = H::only(false, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some());
    h.respond(Response::Start);
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::InProgress);
    h.wait(secs(15));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::InProgress);
    h.wait(secs(5));
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Done);
    assert_eq!(h.accum(Activity::EyeRest), Duration::ZERO);
}

// ------------------------------------------------------------------------ water

#[test]
fn water_is_offered_only_when_at_least_100_ml_behind_pace() {
    // 11:15 expects 390 ml (prototype sample scenario).
    let mut h = water_only_at(ist(20, 11, 15), 291);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.status(Activity::Water), ChipStatus::NotDue);
    assert_eq!(h.view().water.unwrap().expected_ml, 390);
    // Pace keeps moving: 99 ml behind is not enough, 100 ml is.
    h.water.logged_ml = 250;
    h.input(Input::Refresh);
    assert_eq!(h.shown(), Some(Activity::Water));
    match h.view().bubble {
        Some(Bubble::Reminder {
            reason:
                ReminderReason::Water {
                    expected_ml,
                    logged_ml,
                    behind_ml,
                },
            ..
        }) => assert_eq!((expected_ml, logged_ml, behind_ml), (390, 250, 140)),
        other => panic!("unexpected {other:?}"),
    }
    // "Log 250 ml" records the entry and answers water only.
    let out = h.respond(Response::LogWater { ml: 250 });
    assert!(matches!(
        out.events[0].payload,
        EventPayload::WaterLogged { amount_ml: 250 }
    ));
    assert_eq!(out.view.water.unwrap().logged_ml, 500);
    assert_eq!(h.status(Activity::Water), ChipStatus::Done);
    // Back on pace: nothing more for now. The pace keeps moving with the clock, so by
    // about 12:11 (600 ml expected) the user is behind again and water returns after its
    // 30-minute interval.
    h.wait(mins(30));
    assert_eq!(h.offers().len(), 1);
    h.wait_until(mins(60), |v| v.reminder().is_some());
    assert_eq!(h.local(), NaiveTime::from_hms_opt(12, 10, 40).unwrap());
}

#[test]
fn water_behind_again_returns_after_its_interval() {
    let mut h = water_only_at(ist(20, 11, 15), 250);
    assert_eq!(h.shown(), Some(Activity::Water));
    h.respond(Response::Done); // "already did it" without logging: still behind
    assert_eq!(h.wait_until(mins(40), |v| v.reminder().is_some()), mins(30));
}

#[test]
fn reaching_the_goal_pauses_hydration_reminders_but_logging_stays() {
    // 18:20 expects about 1960 ml; 1850 ml logged is 110 ml behind.
    let mut h = water_only_at(ist(20, 18, 20), 1850);
    assert_eq!(h.shown(), Some(Activity::Water));
    h.respond(Response::LogWater { ml: 150 });
    assert!(h.view().water.unwrap().goal_reached);
    assert_eq!(h.status(Activity::Water), ChipStatus::GoalReached);
    h.wait(mins(9));
    assert_eq!(h.offers().len(), 1);
    // Next workday morning the goal is not reached yet, but until then nothing more.
    h.wait(mins(120));
    assert_eq!(h.offers().len(), 1);
    // Manual logging past the goal is still accepted by the store (not the engine's job),
    // and the engine never encourages more.
    h.water.logged_ml = 2300;
    h.input(Input::Refresh);
    assert_eq!(h.status(Activity::Water), ChipStatus::GoalReached);
}

#[test]
fn no_goal_means_no_hydration_reminders() {
    let mut h = H::only(false, false, true);
    h.wait(mins(240));
    assert!(h.offers().is_empty());
    assert_eq!(h.status(Activity::Water), ChipStatus::NoGoal);
}

// ---------------------------------------------------- suppression and precedence

/// Eye rest is due at 10:20; returns a harness at 10:15 with movement and eye rest on.
fn before_eye_rest() -> H {
    let mut h = H::only(true, true, false);
    h.wait(mins(15));
    h
}

/// Hold reminders with `start` at 10:15, keep working for 20 minutes (eye rest falls due
/// at 10:20 if screen time still counts), release with `end`, and check nothing is
/// replayed: no bubble at the end, and the first eye rest comes `expected_wait` later.
fn no_replay(
    start: Input,
    end: Input,
    expected_hold: Hold,
    expected_pet: fn(&PetMode) -> bool,
    expected_wait: Duration,
) {
    let mut h = H::only(false, true, false);
    h.wait(mins(15));
    assert_eq!(
        h.input(start.clone()).outcome,
        InputOutcome::Applied,
        "{start:?}"
    );
    assert_eq!(h.view().hold, Some(expected_hold), "{start:?}");
    assert!(expected_pet(&h.view().pet), "{start:?}: {:?}", h.view().pet);
    h.wait(mins(20)); // eye rest fell due at 10:20 during the hold
    assert!(h.offers().is_empty(), "{start:?} let a reminder through");
    assert_eq!(
        h.input(end.clone()).outcome,
        InputOutcome::Applied,
        "{end:?}"
    );
    assert_eq!(h.view().bubble, None, "{end:?} replayed a reminder");
    assert_eq!(h.count_notices(|n| matches!(n, Notice::HoldEnded(_))), 1);
    let waited = h.wait_until(mins(40), |v| v.reminder().is_some());
    assert_eq!(waited, expected_wait, "{start:?}");
    assert_eq!(h.offers().len(), 1);
}

#[test]
fn pause_holds_reminders_and_nothing_is_replayed() {
    no_replay(
        Input::Pause(PauseLength::UntilResumed),
        Input::ResumeCoaching,
        Hold::PauseLockOffHours,
        |p| {
            matches!(
                p,
                PetMode::Paused {
                    left: None,
                    rest_of_today: false
                }
            )
        },
        // Paused time is not counted, so eye rest had 5 minutes of screen time left.
        mins(5),
    );
}

#[test]
fn focus_holds_reminders_and_nothing_is_replayed() {
    no_replay(
        Input::StartFocus {
            minutes: 60,
            replace_pomodoro: false,
        },
        Input::EndFocus,
        Hold::FocusPresentationMeeting,
        |p| matches!(p, PetMode::Focus { .. }),
        // Eye rest fell due during focus: it waits a full cycle, it is not replayed.
        mins(20),
    );
}

#[test]
fn meeting_holds_reminders_and_nothing_is_replayed() {
    no_replay(
        Input::SetMeeting(true),
        Input::SetMeeting(false),
        Hold::FocusPresentationMeeting,
        |p| *p == PetMode::Meeting,
        mins(20),
    );
}

#[test]
fn presentation_hides_pet_and_bubbles_and_nothing_is_replayed() {
    no_replay(
        Input::PresentationChanged(true),
        Input::PresentationChanged(false),
        Hold::FocusPresentationMeeting,
        |p| *p == PetMode::Hidden(HiddenReason::Presentation),
        mins(20),
    );
}

#[test]
fn presentation_withdraws_an_open_reminder_and_tray_show_pet_keeps_quiet() {
    let mut h = before_eye_rest();
    h.wait(mins(5));
    assert_eq!(h.shown(), Some(Activity::EyeRest));
    h.input(Input::PresentationChanged(true));
    assert!(!h.view().visible);
    assert_eq!(h.view().bubble, None);
    assert_eq!(
        h.responses(),
        vec![(Activity::EyeRest, ReminderOutcome::NoResponse)]
    );
    // Tray "Show pet": the pet comes back, reminders stay held until the presentation ends.
    h.input(Input::ShowPet);
    assert!(h.view().visible);
    assert_eq!(h.view().hold, Some(Hold::FocusPresentationMeeting));
    h.wait(mins(30));
    assert_eq!(h.offers().len(), 1);
    h.input(Input::PresentationChanged(false));
    assert_eq!(h.view().bubble, None);
    // A second presentation hides the pet again (the override ended with the first).
    h.input(Input::PresentationChanged(true));
    assert!(!h.view().visible);
}

#[test]
fn presentation_detection_can_be_switched_off() {
    let mut h = H::at(
        ist(20, 10, 0),
        CoachConfig {
            hide_during_presentation: false,
            ..cats(true, true, false)
        },
        WaterSnapshot::default(),
    );
    h.input(Input::PresentationChanged(true));
    assert!(h.view().visible);
    assert_eq!(h.view().hold, None);
    h.wait_until(mins(25), |v| v.reminder().is_some());
}

#[test]
fn media_hint_is_experimental_and_off_by_default() {
    let mut h = before_eye_rest();
    h.input(Input::MediaInUseChanged(true));
    assert_eq!(h.view().hold, None);
    h.config.media_in_use_hint = true;
    h.input(Input::Refresh);
    assert_eq!(h.view().hold, Some(Hold::FocusPresentationMeeting));
    assert_eq!(h.view().pet, PetMode::Meeting);
}

#[test]
fn precedence_pause_over_focus_over_pomodoro() {
    let mut h = before_eye_rest();
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    assert_eq!(h.view().hold, Some(Hold::ExerciseOrPomodoro));
    assert!(matches!(h.view().pet, PetMode::Pomodoro(_)));
    h.input(Input::SetMeeting(true));
    assert_eq!(h.view().hold, Some(Hold::FocusPresentationMeeting));
    assert_eq!(h.view().pet, PetMode::Meeting);
    h.input(Input::Pause(PauseLength::Minutes(30)));
    assert_eq!(h.view().hold, Some(Hold::PauseLockOffHours));
    assert!(matches!(
        h.view().pet,
        PetMode::Paused { left: Some(_), .. }
    ));
    // Pausing coaching pauses the Pomodoro timer; the 30-minute pause ends on its own.
    assert_eq!(
        h.view().pomodoro.unwrap().paused,
        Some(PomodoroPause::CoachingPaused)
    );
    h.wait(mins(30));
    assert_eq!(h.count_notices(|n| *n == Notice::PauseEnded), 1);
    assert_eq!(h.view().pomodoro.unwrap().paused, None);
    assert_eq!(h.view().pet, PetMode::Meeting);
    h.input(Input::SetMeeting(false));
    assert!(matches!(h.view().pet, PetMode::Pomodoro(_)));
}

#[test]
fn pause_rest_of_today_ends_at_local_midnight() {
    let mut h = H::only(true, true, false);
    h.input(Input::Pause(PauseLength::RestOfToday));
    assert!(matches!(
        h.view().pet,
        PetMode::Paused {
            rest_of_today: true,
            ..
        }
    ));
    h.tick = mins(5);
    h.active = false;
    h.wait(mins(13 * 60 + 55)); // 23:55
    assert!(matches!(h.view().pet, PetMode::Paused { .. }));
    h.wait(mins(10)); // 00:05 next day
    assert_eq!(h.count_notices(|n| *n == Notice::PauseEnded), 1);
    assert_eq!(h.view().pet, PetMode::OffHours);
}

#[test]
fn meeting_toggle_ends_at_the_day_change() {
    let mut h = H::only(true, true, false);
    h.input(Input::SetMeeting(true));
    h.tick = mins(10);
    h.active = false;
    h.wait(mins(24 * 60));
    assert!(!h.view().meeting);
}

#[test]
fn off_hours_withdraw_hold_and_the_next_window_starts_fresh() {
    let mut h = H::at(
        ist(20, 18, 0),
        cats(true, false, false),
        WaterSnapshot::default(),
    );
    h.wait(mins(29));
    // Hand-craft a movement offer near the end of the day: 45 min would be 18:45, so
    // shorten with "More often" (30 min).
    h.config.intensity = Intensity::MoreOften;
    h.input(Input::Refresh);
    assert_eq!(h.shown(), None); // 29 min worked
    h.wait(mins(1)); // 18:30: off-hours start exactly when movement would be due
    assert_eq!(h.view().pet, PetMode::OffHours);
    assert_eq!(h.view().session, SessionState::OffHours);
    assert!(h.offers().is_empty());
    // Work after hours is not counted toward the movement timer.
    let before = h.accum(Activity::Movement);
    h.wait(mins(60));
    assert_eq!(h.accum(Activity::Movement), before);
    // Next morning the window starts with fresh timers and nothing replayed.
    h.tick = mins(5);
    h.active = false;
    h.wait(mins(14 * 60)); // 09:30 Wednesday
    h.active = true;
    h.tick = secs(5);
    h.wait(secs(5));
    assert_eq!(h.local(), NaiveTime::from_hms_opt(9, 30, 5).unwrap());
    assert_eq!(h.accum(Activity::Movement), secs(5));
    assert_eq!(h.view().bubble, None);
    let waited = h.wait_until(mins(40), |v| v.reminder().is_some());
    assert_eq!(waited, mins(30) - secs(5));
}

#[test]
fn a_reminder_open_at_the_end_of_work_hours_is_withdrawn() {
    let mut h = H::at(
        ist(20, 17, 40),
        cats(true, true, false),
        WaterSnapshot::default(),
    );
    h.wait_until(mins(25), |v| v.reminder().is_some()); // eye rest at 18:00
    h.input(Input::CloseBubble);
    h.wait(mins(19)); // badge still waiting at 18:19
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Waiting);
    h.input(Input::OpenPending);
    h.wait(mins(11)); // 18:30
    assert_eq!(h.view().bubble, None);
    assert_eq!(
        h.responses(),
        vec![(Activity::EyeRest, ReminderOutcome::NoResponse)]
    );
}

#[test]
fn switching_a_category_off_withdraws_it_and_it_is_never_offered() {
    let mut h = before_eye_rest();
    h.wait(mins(5));
    assert_eq!(h.shown(), Some(Activity::EyeRest));
    h.config.categories.eye_rest = false;
    h.input(Input::Refresh);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Off);
    h.wait(mins(120));
    assert_eq!(
        h.offers()
            .iter()
            .filter(|a| **a == Activity::EyeRest)
            .count(),
        1
    );
}

#[test]
fn third_dismissal_of_the_day_offers_the_light_schedule_once() {
    let mut h = H::only(false, true, false);
    for _ in 0..3 {
        h.wait_until(mins(30), |v| v.reminder().is_some());
        h.respond(Response::Dismiss);
    }
    assert_eq!(h.view().bubble, Some(Bubble::LighterSchedule));
    assert_eq!(h.view().pet, PetMode::Nudge);
    let out = h.input(Input::LighterSchedule { accept: true });
    assert!(
        out.notices
            .contains(&Notice::UseIntensity(Intensity::Light))
    );
    h.wait_until(mins(30), |v| v.reminder().is_some());
    h.respond(Response::Skip);
    assert_eq!(h.dismissals(), 4);
    assert_ne!(h.view().bubble, Some(Bubble::LighterSchedule));
}

// --------------------------------------------------------------- tracker T-R1..6

#[test]
fn t_r1_reading_without_input_cue_once_at_45_minutes_no_reset() {
    let mut h = H::only(true, false, false);
    h.wait(mins(35));
    h.active = false; // reading: no keyboard or mouse input for 50 minutes
    h.wait(mins(5));
    assert_eq!(h.view().session, SessionState::Quiet);
    h.wait_until(mins(10), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 45)); // 35 active + 10 quiet = 45 counted
    assert_eq!(h.view().session, SessionState::Quiet);
    h.wait(mins(40)); // to 11:25, 50 minutes of reading in all
    assert_eq!(h.view().session, SessionState::LongQuiet);
    assert_eq!(h.offers(), vec![Activity::Movement]); // shown once
    // Not reset: quiet counted until 15 minutes without input (35 + 15 = 50).
    assert_eq!(h.accum(Activity::Movement), mins(50));
    assert_eq!(
        h.responses(),
        vec![(Activity::Movement, ReminderOutcome::NoResponse)]
    );
}

#[test]
fn t_r2_away_unlocked_twenty_minutes_pauses_after_fifteen_no_reset_no_stacking() {
    let mut h = H::only(true, false, false);
    h.wait(mins(10));
    h.active = false;
    h.wait(mins(20));
    assert_eq!(h.view().session, SessionState::LongQuiet);
    assert_eq!(h.accum(Activity::Movement), mins(25));
    assert!(h.offers().is_empty());
    let t = h.e.tally(h.today()).unwrap();
    assert_eq!(t.unknown_ms, 5 * 60_000);
    assert_eq!(t.work_quiet_ms, 14 * 60_000);
    assert_eq!(t.confirmed_breaks, 0);
    h.active = true;
    h.wait(mins(20));
    assert_eq!(h.accum(Activity::Movement), mins(45));
    h.wait(secs(5));
    assert_eq!(h.offers(), vec![Activity::Movement]);
}

#[test]
fn t_r3_lock_three_minutes_resets_then_one_reminder_after_grace() {
    // Water becomes due during the lock: 125 ml logged, 10:30 expects 220, 10:33 expects 230.
    let mut h = H::at(
        ist(20, 10, 0),
        cats(true, false, true),
        WaterSnapshot {
            logged_ml: 125,
            goal_ml: Some(2000),
        },
    );
    h.wait(mins(30));
    assert!(h.offers().is_empty());
    assert_eq!(h.accum(Activity::Movement), mins(30));
    h.input(Input::Lock);
    assert_eq!(h.view().pet, PetMode::Away);
    h.step(mins(2), Input::Tick { idle: mins(2) });
    assert_eq!(h.view().session, SessionState::ConfirmedBreak);
    h.step(mins(1), Input::Tick { idle: mins(3) });
    h.input(Input::Unlock);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(h.e.tally(h.today()).unwrap().confirmed_breaks, 1);
    assert_eq!(h.view().bubble, None);
    let waited = h.wait_until(mins(5), |v| v.reminder().is_some());
    assert_eq!(waited, mins(2));
    assert_eq!(h.shown(), Some(Activity::Water));
    h.respond(Response::Done);
    h.wait(mins(5));
    assert_eq!(h.offers().len(), 1);
}

#[test]
fn t_r4_lock_one_minute_pauses_without_reset() {
    let mut h = H::only(true, false, false);
    h.wait(mins(30));
    h.input(Input::Lock);
    h.step(mins(1), Input::Tick { idle: mins(1) });
    assert_eq!(h.view().session, SessionState::ShortLock);
    h.input(Input::Unlock);
    assert_eq!(h.accum(Activity::Movement), mins(30));
    assert_eq!(h.e.tally(h.today()).unwrap().confirmed_breaks, 0);
    assert_eq!(h.e.tally(h.today()).unwrap().away_ms, 60_000);
}

#[test]
fn t_r5_sleep_two_hours_no_burst_and_pomodoro_waits_for_resume() {
    let mut h = H::at(
        ist(20, 10, 0),
        CoachConfig::default(),
        WaterSnapshot {
            logged_ml: 300,
            goal_ml: Some(2000),
        },
    );
    h.wait(mins(5));
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    h.wait(mins(13));
    let left = h.view().pomodoro.unwrap().left;
    assert_eq!(left, mins(12));
    h.input(Input::Lock);
    h.input(Input::Suspend);
    // Two hours asleep: the driver reports them as elapsed on wake.
    let out = h.step(mins(120), Input::Resume);
    assert!(out.events.is_empty());
    h.input(Input::Unlock);
    let p = h.view().pomodoro.unwrap();
    assert_eq!(p.paused, Some(PomodoroPause::LockOrSleep));
    assert_eq!(p.left, mins(12));
    assert_eq!(p.round, 1);
    assert_eq!(h.view().bubble, Some(Bubble::PomodoroPausedByLock));
    // Water is far behind and eye rest and movement were reset by the sleep, but the
    // Pomodoro card holds the slot: no reminder at all until the user decides.
    h.wait(mins(20));
    assert!(h.offers().is_empty());
    h.input(Input::EndPomodoro);
    // Then one reminder after the calm gap; the next only after it is closed.
    assert_eq!(h.wait_until(mins(10), |v| v.reminder().is_some()), CALM_GAP);
    h.wait(mins(15));
    assert_eq!(h.offers(), vec![Activity::Water]);
}

#[test]
fn t_r6_midnight_during_quiet_is_split_by_local_day() {
    let overnight = WorkSchedule {
        workdays: WeekdaySet::ALL,
        start: hm(18, 0),
        end: hm(2, 0),
    };
    let mut h = H::at(
        ist(20, 23, 50),
        cats(true, false, false),
        WaterSnapshot::default(),
    );
    h.schedule = overnight;
    h.wait(mins(5)); // active until 23:55
    let before = h.accum(Activity::Movement);
    // One 10-minute interval with no input, crossing midnight.
    let out = h.step(mins(10), Input::Tick { idle: mins(10) });
    let d1 = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
    let t1 = h.e.tally(d1).unwrap();
    let t2 = h.e.tally(d2).unwrap();
    assert_eq!(t1.work_active_ms, (5 * 60 + 60) * 1000);
    assert_eq!(t1.work_quiet_ms, 4 * 60_000);
    assert_eq!(t2.work_active_ms, 0);
    assert_eq!(t2.work_quiet_ms, 5 * 60_000);
    assert_eq!(t1.pc_active_ms + t2.pc_active_ms, 15 * 60_000);
    let touched: Vec<_> = out.touched_days.iter().map(|(d, _)| *d).collect();
    assert!(touched.contains(&d1) && touched.contains(&d2));
    // The overnight window belongs to the 20th: no reset at midnight.
    assert_eq!(h.accum(Activity::Movement), before + mins(10));
    assert_eq!(h.view().session, SessionState::Quiet);
}

#[test]
fn t_r6_variant_off_hours_midnight_counts_pc_activity_only() {
    let mut h = H::at(
        ist(20, 23, 58),
        cats(true, false, false),
        WaterSnapshot::default(),
    );
    h.wait(mins(4));
    let d1 = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
    assert_eq!(h.e.tally(d1).unwrap().pc_active_ms, 2 * 60_000);
    assert_eq!(h.e.tally(d2).unwrap().pc_active_ms, 2 * 60_000);
    assert_eq!(h.e.tally(d2).unwrap().work_active_ms, 0);
}

// ---------------------------------------------------------------------- focus

#[test]
fn focus_expires_silently_while_locked_and_does_not_replay() {
    let mut h = before_eye_rest();
    h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: false,
    });
    assert_eq!(h.view().focus_left, Some(mins(30)));
    h.wait(mins(20));
    h.input(Input::Lock);
    h.step(mins(15), Input::Tick { idle: mins(15) });
    assert!(h.notices.iter().any(|(_, n)| *n
        == Notice::FocusEnded {
            reason: FocusEnd::Expired,
            silent: true
        }));
    h.input(Input::Unlock);
    assert_eq!(h.view().focus_left, None);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.e.tally(h.today()).unwrap().focus_sessions, 1);
}

#[test]
fn focus_runs_on_elapsed_time_not_the_wall_clock() {
    let mut h = before_eye_rest();
    h.input(Input::StartFocus {
        minutes: 60,
        replace_pomodoro: false,
    });
    // Someone sets the clock forward an hour: focus is not cut short.
    h.wall += chrono::Duration::hours(1);
    h.input(Input::Refresh);
    assert_eq!(h.view().focus_left, Some(mins(60)));
    h.wall -= chrono::Duration::hours(1);
    h.wait(mins(59));
    assert_eq!(h.view().focus_left, Some(mins(1)));
    h.wait(mins(1));
    assert_eq!(h.view().focus_left, None);
}

#[test]
fn focus_bounds_and_pomodoro_guard() {
    let mut h = H::new();
    let bad = h.input(Input::StartFocus {
        minutes: 4,
        replace_pomodoro: false,
    });
    assert!(matches!(bad.outcome, InputOutcome::Invalid(_)));
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    let ask = h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: false,
    });
    assert_eq!(
        ask.outcome,
        InputOutcome::NeedsConfirmation(Confirmation::ReplacePomodoro)
    );
    assert!(h.view().pomodoro.is_some());
    h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: true,
    });
    assert!(h.view().pomodoro.is_none());
    assert!(matches!(h.view().pet, PetMode::Focus { .. }));
}

// ------------------------------------------------------------------- Pomodoro

#[test]
fn pomodoro_is_off_until_started_holds_reminders_and_offers_a_break() {
    let mut h = before_eye_rest();
    assert!(h.view().pomodoro.is_none());
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    let p = h.view().pomodoro.unwrap();
    assert_eq!(
        (p.phase, p.left, p.round),
        (PomodoroPhase::Work, mins(25), 1)
    );
    h.wait(mins(25)); // eye rest due at 10:20 is held
    assert!(h.offers().is_empty());
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    assert_eq!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak {
            length: mins(5),
            water_behind: None
        })
    );
    assert_eq!(h.e.tally(h.today()).unwrap().pomodoro_rounds, 1);
    // Taking the break is a confirmed break: movement and eye-rest timers restart.
    h.input(Input::StartPomodoroBreak);
    assert_eq!(h.accum(Activity::EyeRest), Duration::ZERO);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    h.wait(mins(5));
    assert_eq!(h.view().bubble, Some(Bubble::PomodoroRoundDone));
    h.input(Input::NextPomodoroRound);
    assert_eq!(h.view().pomodoro.unwrap().round, 2);
    h.input(Input::EndPomodoro);
    assert!(h.view().pomodoro.is_none());
    assert_eq!(h.view().bubble, None);
}

#[test]
fn pomodoro_guards_ask_before_replacing_focus_or_resuming_coaching() {
    let mut h = H::new();
    h.input(Input::StartFocus {
        minutes: 60,
        replace_pomodoro: false,
    });
    let start = |replace_focus, resume_coaching| Input::StartPomodoro {
        preset: PomodoroPreset::Long,
        replace_focus,
        resume_coaching,
    };
    assert_eq!(
        h.input(start(false, false)).outcome,
        InputOutcome::NeedsConfirmation(Confirmation::ReplaceFocus)
    );
    h.input(Input::EndFocus);
    h.input(Input::Pause(PauseLength::UntilResumed));
    assert_eq!(
        h.input(start(false, false)).outcome,
        InputOutcome::NeedsConfirmation(Confirmation::ResumeCoaching)
    );
    assert_eq!(h.input(start(false, true)).outcome, InputOutcome::Applied);
    assert_eq!(h.view().hold, Some(Hold::ExerciseOrPomodoro));
    assert_eq!(h.view().pomodoro.unwrap().left, mins(50));
    let mut h = H::new();
    h.input(Input::StartFocus {
        minutes: 60,
        replace_pomodoro: false,
    });
    let out = h.input(start(true, false));
    assert!(out.notices.contains(&Notice::FocusEnded {
        reason: FocusEnd::Replaced,
        silent: false
    }));
    let bad = h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Custom {
            work_min: 200,
            break_min: 5,
        },
        replace_focus: false,
        resume_coaching: false,
    });
    assert!(matches!(
        bad.outcome,
        InputOutcome::Ignored(_) | InputOutcome::Invalid(_)
    ));
}

#[test]
fn pomodoro_lock_pauses_until_resumed_without_catch_up() {
    let mut h = H::new();
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    h.wait(mins(10));
    h.input(Input::Lock);
    h.step(secs(30), Input::Tick { idle: secs(30) });
    h.input(Input::Unlock);
    assert_eq!(h.view().pomodoro.unwrap().left, mins(15));
    assert_eq!(h.view().bubble, Some(Bubble::PomodoroPausedByLock));
    h.wait(mins(30));
    assert_eq!(h.view().pomodoro.unwrap().left, mins(15));
    // While it is paused and its card is closed, normal reminders may come back.
    h.input(Input::CloseBubble);
    assert_eq!(h.view().hold, None);
    h.input(Input::ResumePomodoro);
    h.wait(mins(15));
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
}

#[test]
fn pomodoro_skip_break_and_auto_start() {
    let mut h = H::new();
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Custom {
            work_min: 10,
            break_min: 2,
        },
        replace_focus: false,
        resume_coaching: false,
    });
    h.wait(mins(10));
    h.input(Input::SkipPomodoroBreak);
    assert_eq!(h.view().bubble, Some(Bubble::PomodoroRoundDone));
    // Skipping is not a break: timers keep counting.
    assert!(h.accum(Activity::Movement) >= mins(10));
    h.input(Input::EndPomodoro);
    // With auto-start on, the next round begins after the break by itself.
    h.config.pomodoro_auto_start = true;
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Custom {
            work_min: 10,
            break_min: 2,
        },
        replace_focus: false,
        resume_coaching: false,
    });
    h.wait(mins(10));
    h.input(Input::StartPomodoroBreak);
    h.wait(mins(2));
    let p = h.view().pomodoro.unwrap();
    assert_eq!((p.phase, p.round), (PomodoroPhase::Work, 2));
    assert_eq!(h.view().bubble, None);
}

#[test]
fn pomodoro_break_card_mentions_water_only_when_behind_pace() {
    // 2000 ml goal, 150 ml logged at 10:00; by 10:25 about 200 ml is expected: on pace.
    let mut h = H::at(
        ist(20, 10, 0),
        cats(false, false, true),
        WaterSnapshot {
            logged_ml: 150,
            goal_ml: Some(2000),
        },
    );
    let start = Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    };
    h.input(start.clone());
    h.wait(mins(25));
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak {
            water_behind: None,
            ..
        })
    ));
    h.input(Input::EndPomodoro);
    // Behind pace by 11:20: the next break card mentions water.
    h.wait(mins(30));
    h.input(start);
    h.wait(mins(25));
    match h.view().bubble {
        Some(Bubble::PomodoroBreak {
            water_behind: Some(w),
            ..
        }) => assert!(w.behind_ml >= 100),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn pomodoro_break_card_is_not_replayed_after_a_presentation() {
    let mut h = H::new();
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    h.wait(mins(20));
    h.input(Input::PresentationChanged(true));
    h.wait(mins(10));
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    h.input(Input::PresentationChanged(false));
    assert_eq!(h.view().bubble, None);
    // The pet still shows "Break time"; a click opens the card.
    h.input(Input::OpenPending);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
}

// --------------------------------------------------------------- clock changes

#[test]
fn clock_set_back_neither_fires_nor_cancels_a_snooze() {
    let mut h = H::only(false, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some());
    h.respond(Response::Snooze);
    h.wait(mins(3));
    h.wall -= chrono::Duration::minutes(50); // 10:23 becomes 09:33, still work hours
    h.input(Input::Refresh);
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Snoozed);
    let waited = h.wait_until(mins(15), |v| v.reminder().is_some());
    assert_eq!(waited, mins(7));
}

#[test]
fn clock_set_forward_cannot_fire_a_burst() {
    let mut h = H::only(true, true, false);
    h.wait(mins(10));
    h.wall += chrono::Duration::hours(3); // 10:10 becomes 13:10
    h.input(Input::Refresh);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.accum(Activity::EyeRest), mins(10));
    h.wait_until(mins(15), |v| v.reminder().is_some());
    assert_eq!(h.accum(Activity::EyeRest), mins(20));
    assert_eq!(h.offers().len(), 1);
}

#[test]
fn clock_jump_into_off_hours_withdraws_and_jump_back_replays_nothing() {
    let mut h = before_eye_rest();
    h.wait(mins(5));
    assert_eq!(h.shown(), Some(Activity::EyeRest));
    h.wall += chrono::Duration::hours(10); // 20:20: off-hours
    h.input(Input::Refresh);
    assert_eq!(h.view().pet, PetMode::OffHours);
    assert_eq!(
        h.responses(),
        vec![(Activity::EyeRest, ReminderOutcome::NoResponse)]
    );
    h.wall -= chrono::Duration::hours(10);
    h.input(Input::Refresh);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.count_notices(|n| matches!(n, Notice::HoldEnded(_))), 1);
    h.wait(CALM_GAP);
    assert_eq!(h.offers().len(), 1);
}

#[test]
fn clock_rollback_across_midnight_keeps_day_counters() {
    let mut h = H::only(false, true, false);
    for _ in 0..2 {
        h.wait_until(mins(30), |v| v.reminder().is_some());
        h.respond(Response::Dismiss);
    }
    let today = h.today();
    assert_eq!(h.e.dismissals(today), 2);
    h.wall -= chrono::Duration::hours(24); // yesterday
    h.input(Input::Refresh);
    assert_eq!(h.e.dismissals(h.today()), 0);
    h.wall += chrono::Duration::hours(24); // back to today
    h.input(Input::Refresh);
    assert_eq!(h.e.dismissals(today), 2);
    h.wait_until(mins(30), |v| v.reminder().is_some());
    h.respond(Response::Dismiss);
    assert_eq!(h.view().bubble, Some(Bubble::LighterSchedule));
}

// ------------------------------------------------------------ long-day check-in

fn long_day(check_in: LongDayCheckIn) -> H {
    let mut h = H::at(
        ist(20, 9, 30),
        CoachConfig {
            long_day_check_in: check_in,
            ..cats(false, false, false)
        },
        WaterSnapshot::default(),
    );
    h.tick = mins(1);
    h
}

#[test]
fn long_day_check_in_after_nine_hours_of_estimated_pc_activity() {
    let mut h = long_day(LongDayCheckIn::default());
    h.wait(mins(9 * 60 - 1));
    assert_eq!(h.view().bubble, None);
    h.wait(mins(1));
    assert_eq!(h.local(), hm(18, 30));
    assert_eq!(h.view().bubble, Some(Bubble::LongDayCheckIn { hours: 9 }));
    let out = h.input(Input::CheckIn(CheckInAnswer::Yes));
    assert!(
        out.notices
            .contains(&Notice::DeadlineDayConfirmed(h.today()))
    );
    h.wait(mins(120));
    assert_eq!(h.view().bubble, None);
}

#[test]
fn long_day_check_in_hours_are_configurable_and_it_can_be_off() {
    let mut h = long_day(LongDayCheckIn {
        enabled: true,
        hours: 10,
    });
    h.wait(mins(9 * 60 + 30));
    assert_eq!(h.view().bubble, None);
    h.wait(mins(30));
    assert_eq!(h.view().bubble, Some(Bubble::LongDayCheckIn { hours: 10 }));
    h.input(Input::CheckIn(CheckInAnswer::DontAskToday));
    let mut off = long_day(LongDayCheckIn {
        enabled: false,
        hours: 9,
    });
    off.wait(mins(12 * 60));
    assert_eq!(off.view().bubble, None);
}

#[test]
fn long_day_check_in_waits_for_focus_to_end() {
    let mut h = long_day(LongDayCheckIn::default());
    h.wait(mins(8 * 60 + 50));
    h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: false,
    });
    h.wait(mins(20));
    assert_eq!(h.view().bubble, None);
    h.wait(mins(10));
    // Focus ended at 18:30; the check-in is not a held reminder, it is offered after the
    // calm gap that follows the end of focus.
    h.wait(CALM_GAP);
    assert_eq!(h.view().bubble, Some(Bubble::LongDayCheckIn { hours: 9 }));
}

// ---------------------------------------------------------- property: invariants

mod property {
    use super::*;
    use proptest::prelude::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Debug, Clone)]
    enum Act {
        Wait(u16, bool),
        Respond(u8),
        Close,
        OpenPending,
        Routine(u8),
        RoutineStarted,
        Lock,
        Unlock,
        Suspend,
        Resume(u16),
        Present(bool),
        Meeting(bool),
        Focus(u8),
        EndFocus,
        Pause(u8),
        ResumeCoaching,
        PomoStart,
        PomoEnd,
        PomoBreak,
        PomoSkip,
        PomoNext,
        PomoResume,
        ShiftWall(i16),
        Water(u16),
        CheckIn(u8),
        Lighter(bool),
        MarkBreak,
        ShowPet,
    }

    fn act() -> impl Strategy<Value = Act> {
        // Waiting dominates and every hold has a more frequent release, so traces spend
        // most of their time where reminders can actually appear.
        prop_oneof![
            30 => (1u16..2400, prop::bool::weighted(0.8)).prop_map(|(s, a)| Act::Wait(s, a)),
            10 => (0u8..6).prop_map(Act::Respond),
            3 => Just(Act::Close),
            3 => Just(Act::OpenPending),
            2 => (0u8..4).prop_map(Act::Routine),
            1 => Just(Act::RoutineStarted),
            1 => Just(Act::Lock),
            3 => Just(Act::Unlock),
            1 => Just(Act::Suspend),
            3 => (0u16..300).prop_map(Act::Resume),
            1 => Just(Act::Present(true)),
            3 => Just(Act::Present(false)),
            1 => Just(Act::Meeting(true)),
            3 => Just(Act::Meeting(false)),
            1 => (0u8..3).prop_map(Act::Focus),
            3 => Just(Act::EndFocus),
            1 => (0u8..3).prop_map(Act::Pause),
            3 => Just(Act::ResumeCoaching),
            1 => Just(Act::PomoStart),
            3 => Just(Act::PomoEnd),
            1 => Just(Act::PomoBreak),
            1 => Just(Act::PomoSkip),
            1 => Just(Act::PomoNext),
            1 => Just(Act::PomoResume),
            1 => (-720i16..720).prop_map(Act::ShiftWall),
            2 => (0u16..600).prop_map(Act::Water),
            1 => (0u8..3).prop_map(Act::CheckIn),
            1 => any::<bool>().prop_map(Act::Lighter),
            1 => Just(Act::MarkBreak),
            1 => Just(Act::ShowPet),
        ]
    }

    struct Checker {
        offered: HashMap<Uuid, Activity>,
        answered: HashSet<Uuid>,
        last_resolved: Option<Duration>,
        last_hold_end: Option<Duration>,
        last_wake: Option<Duration>,
        prev_bubble: Option<Bubble>,
        prev_away: bool,
    }

    impl Checker {
        fn check(&mut self, now: Duration, out: &StepOutput) -> Result<(), TestCaseError> {
            let v = &out.view;
            // One bubble at most (by type), never while hidden.
            prop_assert!(v.bubble.is_none() || v.visible, "bubble while hidden");
            let busy = v
                .chips
                .iter()
                .filter(|c| {
                    matches!(
                        c.status,
                        ChipStatus::Showing | ChipStatus::Waiting | ChipStatus::InProgress
                    )
                })
                .count();
            prop_assert!(busy <= 1, "two activities active: {:?}", v.chips);
            let away = matches!(
                v.session,
                SessionState::ShortLock | SessionState::ConfirmedBreak
            );
            if self.prev_away && !away {
                self.last_wake = Some(now);
            }
            self.prev_away = away;
            for n in &out.notices {
                if let Notice::HoldEnded(_) = n {
                    self.last_hold_end = Some(now);
                }
            }
            for e in &out.events {
                match e.payload {
                    EventPayload::ReminderOffered {
                        reminder_id,
                        activity,
                    } => {
                        prop_assert!(self.offered.insert(reminder_id, activity).is_none());
                        if let Some(t) = self.last_resolved {
                            prop_assert!(now >= t + CALM_GAP, "calm gap broken");
                        }
                        if let Some(t) = self.last_hold_end {
                            prop_assert!(now >= t + CALM_GAP, "replayed after a hold");
                        }
                        if let Some(t) = self.last_wake {
                            prop_assert!(now >= t + hy_core::coach::config::GRACE_AFTER_WAKE);
                        }
                    }
                    EventPayload::ReminderResponse {
                        reminder_id,
                        activity,
                        ..
                    } => {
                        prop_assert_eq!(self.offered.get(&reminder_id), Some(&activity));
                        prop_assert!(self.answered.insert(reminder_id), "two responses");
                    }
                    _ => {}
                }
            }
            if let Some((offer, a)) = v.reminder() {
                prop_assert_eq!(v.chip(a).status, ChipStatus::Showing);
                prop_assert!(v.hold.is_none(), "reminder shown while held: {:?}", v.hold);
                prop_assert_eq!(self.offered.get(&offer), Some(&a));
                // Never a different reminder straight after another (a burst).
                if let Some(Bubble::Reminder { offer: prev, .. }) = self.prev_bubble {
                    prop_assert_eq!(prev, offer, "consecutive pop-ups");
                }
            }
            for n in &out.notices {
                if let Notice::ReminderResolved { .. } = n {
                    self.last_resolved = Some(now);
                }
            }
            self.prev_bubble = v.bubble;
            Ok(())
        }
    }

    /// Apply one action; returns every step's output with the engine time after it.
    fn run(h: &mut H, a: &Act) -> Vec<(Duration, StepOutput)> {
        let offer = h.view().reminder().map(|(o, _)| o);
        let pomo = |preset| Input::StartPomodoro {
            preset,
            replace_focus: true,
            resume_coaching: true,
        };
        let one = match a.clone() {
            Act::Wait(s, active) => {
                h.active = active;
                let mut outs = Vec::new();
                let mut left = u64::from(s);
                while left > 0 {
                    let t = left.min(30);
                    left -= t;
                    h.idle = if active {
                        Duration::ZERO
                    } else {
                        h.idle + secs(t)
                    };
                    let idle = h.idle;
                    let out = h.step(secs(t), Input::Tick { idle });
                    outs.push((h.e.engine_time(), out));
                }
                return outs;
            }
            Act::Respond(k) => {
                let response = match k {
                    0 => Response::Done,
                    1 => Response::LogWater { ml: 250 },
                    2 => Response::Start,
                    3 => Response::Snooze,
                    4 => Response::Skip,
                    _ => Response::Dismiss,
                };
                h.input(Input::Respond {
                    offer: offer.unwrap_or_else(Uuid::nil),
                    response,
                })
            }
            Act::Close => h.input(Input::CloseBubble),
            Act::OpenPending => h.input(Input::OpenPending),
            Act::Routine(k) => h.input(Input::RoutineFinished {
                offer: None,
                outcome: [
                    RoutineOutcome::Done,
                    RoutineOutcome::Partial,
                    RoutineOutcome::None,
                    RoutineOutcome::Skipped,
                ][usize::from(k)],
            }),
            Act::RoutineStarted => h.input(Input::RoutineStarted),
            Act::Lock => h.input(Input::Lock),
            Act::Unlock => h.input(Input::Unlock),
            Act::Suspend => h.input(Input::Suspend),
            Act::Resume(m) => h.step(mins(u64::from(m)), Input::Resume),
            Act::Present(b) => h.input(Input::PresentationChanged(b)),
            Act::Meeting(b) => h.input(Input::SetMeeting(b)),
            Act::Focus(k) => h.input(Input::StartFocus {
                minutes: [30, 60, 90][usize::from(k)],
                replace_pomodoro: true,
            }),
            Act::EndFocus => h.input(Input::EndFocus),
            Act::Pause(k) => h.input(Input::Pause(match k {
                0 => PauseLength::Minutes(30),
                1 => PauseLength::RestOfToday,
                _ => PauseLength::UntilResumed,
            })),
            Act::ResumeCoaching => h.input(Input::ResumeCoaching),
            Act::PomoStart => h.input(pomo(PomodoroPreset::Classic)),
            Act::PomoEnd => h.input(Input::EndPomodoro),
            Act::PomoBreak => h.input(Input::StartPomodoroBreak),
            Act::PomoSkip => h.input(Input::SkipPomodoroBreak),
            Act::PomoNext => h.input(Input::NextPomodoroRound),
            Act::PomoResume => h.input(Input::ResumePomodoro),
            Act::ShiftWall(m) => {
                h.wall += chrono::Duration::minutes(i64::from(m));
                h.input(Input::Refresh)
            }
            Act::Water(ml) => {
                h.water.logged_ml = u32::from(ml) * 5;
                h.input(Input::Refresh)
            }
            Act::CheckIn(k) => h.input(Input::CheckIn(
                [
                    CheckInAnswer::Yes,
                    CheckInAnswer::No,
                    CheckInAnswer::DontAskToday,
                ][usize::from(k)],
            )),
            Act::Lighter(b) => h.input(Input::LighterSchedule { accept: b }),
            Act::MarkBreak => h.input(Input::MarkBreak),
            Act::ShowPet => h.input(Input::ShowPet),
        };
        vec![(h.e.engine_time(), one)]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Never two visible, never a burst, calm gap, grace after wake, no replay after a
        /// hold, one response per offer, and an answer changes only its own activity.
        #[test]
        fn engine_invariants_hold_for_any_trace(
            acts in prop::collection::vec(act(), 1..160),
            goal in prop::option::of(1000u32..3000),
        ) {
            let mut h = H::at(ist(20, 9, 40), CoachConfig::default(), WaterSnapshot {
                logged_ml: 0,
                goal_ml: goal,
            });
            let mut c = Checker {
                offered: HashMap::new(),
                answered: HashSet::new(),
                last_resolved: None,
                last_hold_end: None,
                last_wake: None,
                prev_bubble: None,
                prev_away: false,
            };
            for e in h.events.clone() {
                if let EventPayload::ReminderOffered { reminder_id, activity } = e.payload {
                    c.offered.insert(reminder_id, activity);
                }
            }
            c.prev_bubble = h.view().bubble;
            for a in &acts {
                let shown = h.view().reminder().map(|(_, a)| a);
                let before: Vec<_> = Activity::ALL.iter().map(|x| *h.e.activity(*x)).collect();
                for (now, out) in run(&mut h, a) {
                    c.check(now, &out)?;
                }
                if let (Act::Respond(_), Some(s)) = (a, shown) {
                    for (i, x) in Activity::ALL.iter().enumerate() {
                        if *x != s {
                            prop_assert_eq!(&before[i], h.e.activity(*x), "answer to {} changed {}", s, x);
                        }
                    }
                }
            }
        }
    }
}
