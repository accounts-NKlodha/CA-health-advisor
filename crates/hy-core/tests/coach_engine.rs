//! Deterministic trace tests for the coaching engine (architecture 11: T-R1 to T-R6, the
//! one-at-a-time reminder of prototype rev 3, suppression and precedence, focus, Pomodoro,
//! clock changes and day boundaries). Time is replayed step by step; nothing sleeps.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use hy_core::coach::config::{CALM_GAP, Categories, LongDayCheckIn, SNOOZE};
use hy_core::coach::*;
use hy_core::schema::{
    Activity, CheckInOrigin, CheckInReply, EventPayload, NewEvent, PhoneLabel, PlanChange,
    ReflectionAnswer, ReminderOutcome, RoutineOutcome, RoutineVariant, SessionState, StepOutcome,
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
    /// Today's water entries, as the store lists them.
    entries: Vec<WaterEntry>,
    /// Entries recorded so far (for their ids).
    seq: u128,
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
            water_entries: Vec::new(),
            config,
            servings: Servings::default(),
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
            entries: Vec::new(),
            seq: 0,
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
            water_entries: self.entries.clone(),
            config: self.config,
            servings: Servings::default(),
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
    // The guide opens on today's routine.
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::MovementGuide(GuideView {
            stage: GuideStage::Intro,
            from_reminder: true,
            ..
        }))
    ));
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
    assert!(matches!(h.view().bubble, Some(Bubble::MovementGuide(_))));
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

// ------------------------------------------------------- guided movement routine

impl H {
    fn guide(&self) -> GuideView {
        self.view().guide.expect("the movement guide is open")
    }

    fn stage(&self) -> GuideStage {
        self.guide().stage
    }

    fn g(&mut self, a: GuideAction) -> StepOutput {
        self.input(Input::Guide(a))
    }

    /// `movement.step` events: (routine id, routine, step index, outcome).
    fn steps(&self) -> Vec<(Uuid, String, u32, StepOutcome)> {
        self.events
            .iter()
            .filter_map(|e| match &e.payload {
                EventPayload::MovementStep {
                    session_id,
                    routine_id,
                    step_index,
                    steps_total,
                    routine_version,
                    outcome,
                    ..
                } => {
                    assert_eq!((*steps_total, *routine_version), (3, 1));
                    Some((*session_id, routine_id.clone(), *step_index, *outcome))
                }
                _ => None,
            })
            .collect()
    }

    /// `movement.session` events: (routine id, routine, variant, outcome, done, skipped).
    #[allow(clippy::type_complexity)]
    fn sessions(
        &self,
    ) -> Vec<(
        Uuid,
        String,
        Option<RoutineVariant>,
        RoutineOutcome,
        u32,
        u32,
    )> {
        self.events
            .iter()
            .filter_map(|e| match &e.payload {
                EventPayload::MovementSession {
                    session_id,
                    routine_id,
                    variant,
                    outcome,
                    steps_done,
                    steps_skipped,
                    steps_total,
                    ..
                } => {
                    assert_eq!(*steps_total, 3);
                    Some((
                        *session_id,
                        routine_id.clone(),
                        *variant,
                        *outcome,
                        *steps_done,
                        *steps_skipped,
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn restart(&mut self, elapsed: Duration) -> StepOutput {
        self.wall += chrono::Duration::from_std(elapsed).unwrap();
        let out = self.e.restart(&self.ctx(elapsed));
        self.record(&out);
        out
    }
}

/// Movement offered at 10:45 on Tuesday 20 Oct, "Start movement", then `variant` chosen
/// on the first screen: the guide is on the first movement.
fn guided(variant: RoutineVariant) -> H {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    h.respond(Response::Start);
    let g = h.guide();
    assert_eq!((g.day, g.today), (Weekday::Tue, Weekday::Tue));
    assert_eq!(
        (g.stage, g.variant, g.open),
        (GuideStage::Intro, None, true)
    );
    assert!(g.from_reminder);
    assert_eq!(
        h.g(GuideAction::Variant(variant)).outcome,
        InputOutcome::Applied
    );
    assert_eq!(h.stage(), GuideStage::Step { index: 0, total: 3 });
    assert_eq!(h.guide().variant, Some(variant));
    h
}

#[test]
fn guide_every_movement_done_records_each_step_and_a_done_routine() {
    let mut h = guided(RoutineVariant::Seated);
    for i in 0..3 {
        assert_eq!(h.stage(), GuideStage::Step { index: i, total: 3 });
        h.wait(secs(40));
        h.g(GuideAction::Done);
    }
    let steps = h.steps();
    assert_eq!(steps.len(), 3);
    let id = steps[0].0;
    for (i, s) in steps.iter().enumerate() {
        assert_eq!(s, &(id, "tue".to_owned(), i as u32, StepOutcome::Done));
    }
    assert_eq!(
        h.sessions(),
        vec![(
            id,
            "tue".to_owned(),
            Some(RoutineVariant::Seated),
            RoutineOutcome::Done,
            3,
            0
        )]
    );
    assert_eq!(
        h.stage(),
        GuideStage::Result {
            outcome: RoutineOutcome::Done,
            done: 3,
            skipped: 0,
            total: 3,
            stopped: false
        }
    );
    assert_eq!(h.status(Activity::Movement), ChipStatus::Done);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(
        h.count_notices(|n| *n == Notice::Celebrate(Activity::Movement)),
        1
    );
    // The routine is over: the movement reminder is no longer in progress.
    assert_eq!(h.view().hold, None);
    assert_eq!(h.view().pet, PetMode::Idle);
    // The result closes by itself after a minute; another answer changes nothing.
    assert_eq!(
        h.g(GuideAction::Done).outcome,
        InputOutcome::Ignored("the routine is over")
    );
    h.wait(secs(60));
    assert_eq!(h.view().guide, None);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.steps().len(), 3);
    // The routine id is distinct from the reminder id.
    let reminder = h
        .events
        .iter()
        .find_map(|e| match e.payload {
            EventPayload::ReminderOffered { reminder_id, .. } => Some(reminder_id),
            _ => None,
        })
        .unwrap();
    assert_ne!(reminder, id);
}

#[test]
fn guide_stop_on_the_first_movement_is_none_and_keeps_the_timer() {
    let mut h = guided(RoutineVariant::Standing);
    h.wait(secs(20));
    h.g(GuideAction::Stop);
    assert!(h.steps().is_empty());
    let s = h.sessions();
    assert_eq!(s.len(), 1);
    assert_eq!(
        (s[0].2, s[0].3, s[0].4, s[0].5),
        (Some(RoutineVariant::Standing), RoutineOutcome::None, 0, 0)
    );
    assert_eq!(
        h.stage(),
        GuideStage::Result {
            outcome: RoutineOutcome::None,
            done: 0,
            skipped: 0,
            total: 3,
            stopped: true
        }
    );
    assert_eq!(h.status(Activity::Movement), ChipStatus::NoneRecorded);
    // Not reset: the 45 minutes before the reminder are still counted.
    assert!(h.accum(Activity::Movement) >= mins(45));
    assert_eq!(h.dismissals(), 0);
    h.g(GuideAction::Close);
    assert_eq!(h.view().guide, None);
    // Rescheduled a full cycle after the stop (10:45:20), not restarted.
    let (at, worked) = next_movement(&mut h);
    assert_eq!(at, NaiveTime::from_hms_opt(11, 30, 20).unwrap());
    assert!(worked >= mins(90));
}

#[test]
fn guide_every_movement_skipped_is_none_and_not_a_dismissal() {
    let mut h = guided(RoutineVariant::Seated);
    for _ in 0..3 {
        h.g(GuideAction::Skip);
    }
    let steps = h.steps();
    assert_eq!(
        steps.iter().map(|s| s.3).collect::<Vec<_>>(),
        vec![StepOutcome::Skipped; 3]
    );
    let s = h.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::None, 0, 3));
    assert!(matches!(
        h.stage(),
        GuideStage::Result {
            outcome: RoutineOutcome::None,
            stopped: false,
            ..
        }
    ));
    assert_eq!(h.status(Activity::Movement), ChipStatus::NoneRecorded);
    assert!(h.accum(Activity::Movement) >= mins(45));
    assert_eq!(h.dismissals(), 0);
}

#[test]
fn guide_one_done_then_stop_is_partial_and_resets_the_timer() {
    let mut h = guided(RoutineVariant::Standing);
    h.g(GuideAction::Done);
    h.g(GuideAction::Skip);
    h.g(GuideAction::Stop);
    assert_eq!(
        h.steps().iter().map(|s| (s.2, s.3)).collect::<Vec<_>>(),
        vec![(0, StepOutcome::Done), (1, StepOutcome::Skipped)]
    );
    let s = h.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::Partial, 1, 1));
    assert_eq!(
        h.stage(),
        GuideStage::Result {
            outcome: RoutineOutcome::Partial,
            done: 1,
            skipped: 1,
            total: 3,
            stopped: true
        }
    );
    assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(h.count_notices(|n| matches!(n, Notice::Celebrate(_))), 0);
}

#[test]
fn guide_skip_today_is_a_skip_with_no_catch_up() {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    h.respond(Response::Start);
    h.g(GuideAction::SkipToday);
    let s = h.sessions();
    assert_eq!(
        (s[0].1.as_str(), s[0].2, s[0].3, s[0].4, s[0].5),
        ("tue", None, RoutineOutcome::Skipped, 0, 0)
    );
    assert!(h.steps().is_empty());
    assert_eq!(h.status(Activity::Movement), ChipStatus::Skipped);
    assert_eq!(h.dismissals(), 1);
    assert!(h.accum(Activity::Movement) >= mins(45));
    assert_eq!(next_movement(&mut h).0, hm(11, 30));
}

#[test]
fn guide_later_from_a_reminder_snoozes_and_from_the_menu_records_nothing() {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    h.respond(Response::Start);
    h.g(GuideAction::Later);
    assert_eq!(h.view().guide, None);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Snoozed);
    assert!(h.sessions().is_empty());
    h.wait_until(mins(11), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 55));

    let mut m = H::only(true, false, false);
    m.wait(mins(10));
    m.input(Input::RoutineStarted);
    assert!(!m.guide().from_reminder);
    m.g(GuideAction::Later);
    assert_eq!(m.view().guide, None);
    assert_eq!(m.view().hold, None);
    assert!(m.sessions().is_empty());
    assert_eq!(m.accum(Activity::Movement), mins(10));
}

#[test]
fn guide_from_the_menu_done_restarts_the_timer_and_none_changes_nothing() {
    let mut h = H::only(true, false, false);
    h.wait(mins(20));
    h.input(Input::RoutineStarted);
    assert_eq!(h.view().hold, Some(Hold::ExerciseOrPomodoro));
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    for _ in 0..3 {
        h.g(GuideAction::Done);
    }
    assert_eq!(h.sessions()[0].3, RoutineOutcome::Done);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Done);
    assert_eq!(h.view().hold, None);
    h.g(GuideAction::Close);

    let mut n = H::only(true, false, false);
    n.wait(mins(20));
    n.input(Input::RoutineStarted);
    n.g(GuideAction::Variant(RoutineVariant::Standing));
    n.g(GuideAction::Stop);
    assert_eq!(n.sessions()[0].3, RoutineOutcome::None);
    assert_eq!(n.accum(Activity::Movement), mins(20));
    assert_eq!(n.status(Activity::Movement), ChipStatus::NotDue);
    // The movement reminder still comes at its usual time.
    n.g(GuideAction::Close);
    n.wait_until(mins(30), |v| v.reminder().is_some());
    assert_eq!(n.local(), hm(10, 45));
}

#[test]
fn guide_other_day_and_sunday_choose_any_routine() {
    let mut h = H::only(true, false, false);
    h.input(Input::RoutineStarted);
    h.g(GuideAction::OtherDay);
    assert_eq!(h.stage(), GuideStage::ChooseDay);
    h.g(GuideAction::Back);
    assert_eq!(h.stage(), GuideStage::Intro);
    h.g(GuideAction::OtherDay);
    assert_eq!(
        h.g(GuideAction::Day(Weekday::Sun)).outcome,
        InputOutcome::Invalid("choose a day from Monday to Saturday")
    );
    h.g(GuideAction::Day(Weekday::Sat));
    assert_eq!(
        (h.guide().day, h.stage()),
        (Weekday::Sat, GuideStage::Intro)
    );
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    h.g(GuideAction::Done);
    assert_eq!(h.steps()[0].1, "sat");

    // Sunday 25 Oct: nothing is scheduled; any day's routine may be chosen.
    let mut s = H::at(
        ist(25, 11, 0),
        cats(true, false, false),
        WaterSnapshot::default(),
    );
    s.input(Input::RoutineStarted);
    assert_eq!(
        (s.guide().day, s.stage()),
        (Weekday::Sun, GuideStage::Sunday)
    );
    // No routine yet: there is nothing to start or skip.
    assert!(matches!(
        s.g(GuideAction::Variant(RoutineVariant::Seated)).outcome,
        InputOutcome::Ignored(_)
    ));
    assert!(matches!(
        s.g(GuideAction::SkipToday).outcome,
        InputOutcome::Ignored(_)
    ));
    s.g(GuideAction::Day(Weekday::Mon));
    assert_eq!(s.stage(), GuideStage::Intro);
    s.g(GuideAction::Variant(RoutineVariant::Standing));
    s.g(GuideAction::Done);
    s.g(GuideAction::Done);
    s.g(GuideAction::Done);
    let sess = s.sessions();
    assert_eq!(
        (sess[0].1.as_str(), sess[0].3),
        ("mon", RoutineOutcome::Done)
    );
}

#[test]
fn guide_closed_waits_behind_the_pet_and_holds_every_reminder() {
    let mut h = H::only(true, true, false);
    h.wait(mins(15));
    h.input(Input::RoutineStarted);
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    h.g(GuideAction::Done);
    h.input(Input::CloseBubble);
    assert_eq!(h.view().bubble, None);
    assert!(!h.guide().open);
    assert_eq!(h.view().pet, PetMode::Exercise);
    // Eye rest falls due at 10:20; nothing appears while the routine waits.
    h.wait(mins(10));
    assert_eq!(h.view().bubble, None);
    assert!(h.offers().is_empty());
    // A click on the pet brings the routine back at the same movement.
    h.input(Input::OpenPending);
    assert_eq!(h.stage(), GuideStage::Step { index: 1, total: 3 });
    assert!(matches!(h.view().bubble, Some(Bubble::MovementGuide(_))));
    h.g(GuideAction::Done);
    h.g(GuideAction::Done);
    // One at a time: the result, then the eye-rest reminder only after the calm gap.
    h.g(GuideAction::Close);
    assert_eq!(h.wait_until(mins(10), |v| v.reminder().is_some()), CALM_GAP);
    assert_eq!(h.shown(), Some(Activity::EyeRest));
}

#[test]
fn guide_result_card_keeps_the_bubble_until_it_closes_then_the_calm_gap_follows() {
    let mut h = H::only(true, true, false);
    h.wait(mins(15));
    h.input(Input::RoutineStarted);
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    h.wait(mins(10));
    h.g(GuideAction::Done);
    h.g(GuideAction::Done);
    h.g(GuideAction::Done);
    assert!(matches!(h.stage(), GuideStage::Result { .. }));
    // The result stays a minute; the eye-rest reminder waits for the calm gap.
    assert_eq!(h.wait_until(mins(10), |v| v.reminder().is_some()), CALM_GAP);
    assert_eq!(h.view().guide, None);
}

/// The guide waits behind the pet at movement `index` of a routine that still runs.
fn assert_behind_the_pet(h: &H, index: u32, why: &str) {
    let g = h.guide();
    assert!(!g.open, "{why}");
    assert_eq!(g.stage, GuideStage::Step { index, total: 3 }, "{why}");
    assert!(
        !matches!(h.view().bubble, Some(Bubble::MovementGuide(_))),
        "{why}"
    );
    assert!(
        h.view()
            .active_routine
            .is_some_and(|r| r.activity == Activity::Movement),
        "{why}"
    );
}

#[test]
fn guide_under_a_new_hold_waits_behind_the_pet_and_the_routine_keeps_running() {
    for hold in [
        NewHold::Lock,
        NewHold::Suspend,
        NewHold::Pause,
        NewHold::OffHours,
    ] {
        let why = format!("{hold:?}");
        let mut h = guided(RoutineVariant::Seated);
        h.g(GuideAction::Done);
        start_hold(&mut h, hold);
        // Nothing is recorded yet; the card goes behind the pet.
        assert!(h.sessions().is_empty(), "{why}");
        assert_behind_the_pet(&h, 1, &why);
        // It does not come back by itself when the hold ends, and nothing else appears.
        end_hold(&mut h, hold);
        assert_behind_the_pet(&h, 1, &why);
        assert!(h.view().reminder().is_none(), "{why}");
        // A click brings it back at the same movement.
        assert_eq!(h.input(Input::OpenPending).outcome, InputOutcome::Applied);
        assert!(matches!(h.view().bubble, Some(Bubble::MovementGuide(_))));
        h.g(GuideAction::Done);
        h.g(GuideAction::Done);
        let s = h.sessions();
        assert_eq!(s.len(), 1, "{why}");
        assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::Done, 3, 0));
        assert_eq!(h.steps().len(), 3, "{why}");
        assert_eq!(h.status(Activity::Movement), ChipStatus::Done);
    }
}

#[test]
fn guide_brought_back_under_a_pause_or_off_hours_is_answered_without_lifting_it() {
    for hold in [NewHold::Pause, NewHold::OffHours] {
        let why = format!("{hold:?}");
        let mut h = guided(RoutineVariant::Standing);
        h.g(GuideAction::Done);
        start_hold(&mut h, hold);
        let session = h.view().session;
        assert_behind_the_pet(&h, 1, &why);
        assert_eq!(h.input(Input::OpenPending).outcome, InputOutcome::Applied);
        // Opened by the user under the hold: it stays for that hold.
        h.wait(mins(1));
        assert!(matches!(h.view().bubble, Some(Bubble::MovementGuide(_))));
        h.g(GuideAction::Skip);
        h.g(GuideAction::Stop);
        let s = h.sessions();
        assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::Partial, 1, 1));
        assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);
        // The hold is still on.
        assert_eq!(h.view().session, session, "{why}");
        assert_ne!(session, SessionState::Active, "{why}");
    }
}

#[test]
fn guide_under_a_new_hold_before_any_answer_ends_only_by_its_time_limit() {
    let holds = [
        Input::Lock,
        Input::SetMeeting(true),
        Input::StartFocus {
            minutes: 30,
            replace_pomodoro: false,
        },
    ];
    for hold in holds {
        let mut h = guided(RoutineVariant::Seated);
        h.input(hold.clone());
        assert_behind_the_pet(&h, 0, &format!("{hold:?}"));
        assert!(h.sessions().is_empty(), "{hold:?}");
        // Unanswered for 30 minutes: as before, a routine started from a reminder that
        // ends unanswered is "none recorded", and nothing else is written.
        h.wait(mins(30));
        assert_eq!(h.view().guide, None, "{hold:?}");
        assert!(h.sessions().is_empty(), "{hold:?}");
        assert!(h.steps().is_empty());
        assert_eq!(h.status(Activity::Movement), ChipStatus::NoneRecorded);
        assert_eq!(h.view().active_routine, None);
    }
}

#[test]
fn guide_opened_during_a_meeting_goes_behind_the_pet_when_a_new_meeting_starts() {
    let mut h = H::only(true, false, false);
    h.input(Input::SetMeeting(true));
    h.input(Input::RoutineStarted);
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    h.wait(mins(1));
    assert_eq!(h.stage(), GuideStage::Step { index: 0, total: 3 });
    assert!(h.guide().open);
    h.input(Input::SetMeeting(false));
    h.g(GuideAction::Done);
    assert_eq!(h.stage(), GuideStage::Step { index: 1, total: 3 });
    // The meeting ended; starting it again is a new hold.
    h.input(Input::SetMeeting(true));
    assert_behind_the_pet(&h, 1, "meeting");
    assert!(h.sessions().is_empty());
    h.input(Input::OpenPending);
    h.g(GuideAction::Done);
    h.g(GuideAction::Skip);
    assert_eq!(h.sessions()[0].3, RoutineOutcome::Partial);
    assert!(h.view().meeting);
}

#[test]
fn guide_ends_after_thirty_minutes_keeping_answered_movements() {
    let mut h = guided(RoutineVariant::Seated);
    h.g(GuideAction::Done);
    h.g(GuideAction::Done);
    h.wait(mins(29));
    assert_eq!(h.stage(), GuideStage::Step { index: 2, total: 3 });
    h.wait(mins(1));
    assert_eq!(h.view().guide, None);
    let s = h.sessions();
    assert_eq!((s[0].3, s[0].4), (RoutineOutcome::Partial, 2));
    assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);
}

#[test]
fn guide_restart_mid_routine_keeps_answered_movements_and_a_fresh_one_restarts_cleanly() {
    let mut h = guided(RoutineVariant::Standing);
    h.g(GuideAction::Done);
    let out = h.restart(mins(1));
    assert_eq!(
        out.events
            .iter()
            .filter(|e| matches!(e.payload, EventPayload::MovementSession { .. }))
            .count(),
        1
    );
    let s = h.sessions();
    assert_eq!((s[0].3, s[0].4), (RoutineOutcome::Partial, 1));
    assert_eq!(h.view().guide, None);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);

    // Nothing answered: nothing recorded, "none recorded" as before.
    let mut n = guided(RoutineVariant::Standing);
    n.restart(mins(1));
    assert!(n.sessions().is_empty());
    assert_eq!(n.status(Activity::Movement), ChipStatus::NoneRecorded);
    // A new routine gets a new id.
    n.wait(mins(5));
    n.input(Input::RoutineStarted);
    n.g(GuideAction::Variant(RoutineVariant::Seated));
    n.g(GuideAction::Done);
    let mut m = guided(RoutineVariant::Standing);
    m.g(GuideAction::Done);
    assert_ne!(n.steps()[0].0, Uuid::nil());
}

#[test]
fn guide_is_ended_by_starting_a_pomodoro() {
    let mut h = guided(RoutineVariant::Seated);
    h.g(GuideAction::Done);
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    assert_eq!(h.view().guide, None);
    assert_eq!(h.sessions()[0].3, RoutineOutcome::Partial);
}

#[test]
fn guide_answered_in_one_go_from_the_menu_records_the_report() {
    use RoutineOutcome as O;
    // (answers in the guide, report) -> (outcome, done, skipped) recorded. Answered
    // movements stand; the report covers the movements not answered yet.
    let cases = [
        (vec![], O::Done, (O::Done, 3, 0)),
        (vec![GuideAction::Done], O::Done, (O::Done, 3, 0)),
        (vec![GuideAction::Skip], O::Done, (O::Partial, 2, 1)),
        (vec![GuideAction::Done], O::Partial, (O::Partial, 1, 0)),
        (vec![], O::Partial, (O::Partial, 1, 0)),
        (vec![GuideAction::Skip], O::Partial, (O::Partial, 1, 1)),
        (
            vec![GuideAction::Done, GuideAction::Skip],
            O::Partial,
            (O::Partial, 1, 1),
        ),
        (
            vec![GuideAction::Done, GuideAction::Done],
            O::Partial,
            (O::Partial, 2, 0),
        ),
        (vec![], O::None, (O::None, 0, 0)),
        (vec![GuideAction::Skip], O::None, (O::None, 0, 1)),
        (vec![GuideAction::Done], O::None, (O::Partial, 1, 0)),
        (vec![], O::Skipped, (O::Skipped, 0, 0)),
        (vec![GuideAction::Skip], O::Skipped, (O::None, 0, 1)),
        (vec![GuideAction::Done], O::Skipped, (O::Partial, 1, 0)),
        (
            vec![GuideAction::Skip, GuideAction::Done],
            O::Skipped,
            (O::Partial, 1, 1),
        ),
    ];
    for (answers, report, (outcome, done, skipped)) in cases {
        let why = format!("{answers:?} then {report:?}");
        let mut h = guided(RoutineVariant::Seated);
        for a in &answers {
            h.g(*a);
        }
        // Under a pause, as the menu and tray allow.
        h.input(Input::Pause(PauseLength::UntilResumed));
        assert_eq!(
            h.input(Input::RoutineFinished {
                offer: None,
                outcome: report,
            })
            .outcome,
            InputOutcome::Applied,
            "{why}"
        );
        assert_eq!(h.view().guide, None, "{why}");
        assert_eq!(h.view().active_routine, None, "{why}");
        let s = h.sessions();
        assert_eq!(s.len(), 1, "{why}");
        assert_eq!(s[0].0, h.steps().first().map_or(s[0].0, |st| st.0), "{why}");
        assert_eq!(
            (s[0].2, s[0].3, s[0].4, s[0].5),
            (Some(RoutineVariant::Seated), outcome, done, skipped),
            "{why}"
        );
        assert_eq!(h.steps().len(), answers.len(), "{why}");
        assert_counts_include_steps(&h, &why);
        // Done and partial restart the movement timer; none and skipped keep it.
        assert_eq!(
            h.accum(Activity::Movement) == Duration::ZERO,
            outcome.resets_movement_timer(),
            "{why}"
        );
        // The reminder's chip and dismissals follow the recorded outcome.
        let chip = match outcome {
            O::Done => ChipStatus::Done,
            O::Partial => ChipStatus::Partial,
            O::None => ChipStatus::NoneRecorded,
            O::Skipped => ChipStatus::Skipped,
        };
        assert_eq!(h.status(Activity::Movement), chip, "{why}");
        assert_eq!(h.dismissals(), u32::from(outcome == O::Skipped), "{why}");
    }

    // Before a variant is chosen there is no routine to record: the report applies to
    // the reminder alone, as for a routine started without the guide.
    let mut q = H::only(true, false, false);
    q.wait_until(mins(50), |v| v.reminder().is_some());
    let offer = q.offer();
    q.respond(Response::Start);
    q.input(Input::RoutineFinished {
        offer: Some(offer),
        outcome: RoutineOutcome::Done,
    });
    assert_eq!(q.view().guide, None);
    assert!(q.sessions().is_empty());
    assert_eq!(q.status(Activity::Movement), ChipStatus::Done);
}

/// The session's counts include every movement answered in the guide (each has its
/// `movement.step` event): never fewer done or skipped than the steps recorded.
fn assert_counts_include_steps(h: &H, why: &str) {
    for s in h.sessions() {
        let steps: Vec<_> = h.steps().into_iter().filter(|st| st.0 == s.0).collect();
        let done = steps.iter().filter(|st| st.3 == StepOutcome::Done).count() as u32;
        let skipped = steps.len() as u32 - done;
        assert!(
            s.4 >= done && s.5 >= skipped,
            "{why}: {:?} {}/{} with steps {steps:?}",
            s.3,
            s.4,
            s.5
        );
    }
}

#[test]
fn guide_reported_skip_after_an_answer_is_a_stop() {
    // The shrunk trace from `engine_invariants_hold_for_any_trace`: a menu routine, seated,
    // the first movement done, then "skipped" reported in one go. Skip today is over once
    // a movement is answered, so this is a stop: partial, as the movement was done, with
    // the counts of the movements answered.
    let mut h = H::at(
        ist(20, 9, 40),
        CoachConfig::default(),
        WaterSnapshot {
            logged_ml: 0,
            goal_ml: None,
        },
    );
    h.input(Input::RoutineStarted);
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    h.g(GuideAction::Done);
    let report = |h: &mut H| {
        h.input(Input::RoutineFinished {
            offer: None,
            outcome: RoutineOutcome::Skipped,
        })
        .outcome
    };
    assert_eq!(report(&mut h), InputOutcome::Applied);
    assert_eq!(h.view().guide, None);
    let steps = h.steps();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].3, StepOutcome::Done);
    assert_eq!(
        h.sessions(),
        vec![(
            steps[0].0,
            steps[0].1.clone(),
            Some(RoutineVariant::Seated),
            RoutineOutcome::Partial,
            1,
            0
        )]
    );

    // From the menu after a done movement: partial, and the movement timer restarts.
    let menu = |answer: GuideAction| {
        let mut m = H::only(true, false, false);
        m.wait(mins(20));
        m.input(Input::RoutineStarted);
        m.g(GuideAction::Variant(RoutineVariant::Standing));
        m.g(answer);
        assert_eq!(report(&mut m), InputOutcome::Applied);
        m
    };
    let m = menu(GuideAction::Done);
    assert_eq!(
        m.steps().iter().map(|s| s.3).collect::<Vec<_>>(),
        vec![StepOutcome::Done]
    );
    let s = m.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::Partial, 1, 0));
    assert_eq!(m.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(m.status(Activity::Movement), ChipStatus::Partial);
    assert_eq!(m.dismissals(), 0);
    assert_eq!(m.count_notices(|n| matches!(n, Notice::Celebrate(_))), 0);

    // From the menu after a skipped movement: none, and the movement timer keeps running.
    let m = menu(GuideAction::Skip);
    assert_eq!(
        m.steps().iter().map(|s| s.3).collect::<Vec<_>>(),
        vec![StepOutcome::Skipped]
    );
    let s = m.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::None, 0, 1));
    assert_eq!(m.accum(Activity::Movement), mins(20));
    assert_eq!(m.status(Activity::Movement), ChipStatus::NotDue);
    assert_eq!(m.dismissals(), 0);

    // From a reminder after a done movement: partial, not a dismissal, the timer restarts.
    let mut r = guided(RoutineVariant::Seated);
    r.g(GuideAction::Done);
    assert_eq!(report(&mut r), InputOutcome::Applied);
    assert_eq!(
        r.steps().iter().map(|s| s.3).collect::<Vec<_>>(),
        vec![StepOutcome::Done]
    );
    let s = r.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::Partial, 1, 0));
    assert_eq!(r.status(Activity::Movement), ChipStatus::Partial);
    assert_eq!(r.dismissals(), 0);
    assert_eq!(r.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(r.view().active_routine, None);
    assert_eq!(r.count_notices(|n| matches!(n, Notice::Celebrate(_))), 0);

    // From a reminder after a skipped movement: none recorded, not a dismissal, the timer
    // kept and movement rescheduled a full cycle later.
    let mut r = guided(RoutineVariant::Seated);
    r.g(GuideAction::Skip);
    assert_eq!(report(&mut r), InputOutcome::Applied);
    assert_eq!(
        r.steps().iter().map(|s| s.3).collect::<Vec<_>>(),
        vec![StepOutcome::Skipped]
    );
    let s = r.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::None, 0, 1));
    assert_eq!(r.status(Activity::Movement), ChipStatus::NoneRecorded);
    assert_eq!(r.dismissals(), 0);
    assert!(r.accum(Activity::Movement) >= mins(45));
    assert_eq!(r.count_notices(|n| matches!(n, Notice::Celebrate(_))), 0);
    assert_eq!(next_movement(&mut r).0, hm(11, 30));

    // Before any answer a reported skip is still skip today.
    let mut k = guided(RoutineVariant::Seated);
    assert_eq!(report(&mut k), InputOutcome::Applied);
    let s = k.sessions();
    assert_eq!((s[0].3, s[0].4, s[0].5), (RoutineOutcome::Skipped, 0, 0));
    assert_eq!(k.status(Activity::Movement), ChipStatus::Skipped);
    assert_eq!(k.dismissals(), 1);
}

#[test]
fn guide_report_from_a_reminder_keeps_the_answered_movements() {
    use RoutineOutcome as O;
    // (answer in the guide, report) -> (outcome, done, skipped) recorded, for a routine
    // started from a reminder. The chip, dismissals, celebration and movement timer follow
    // the recorded outcome, and the counts include the answered movement.
    let cases = [
        (GuideAction::Done, O::Done, (O::Done, 3, 0)),
        (GuideAction::Done, O::Partial, (O::Partial, 1, 0)),
        (GuideAction::Done, O::None, (O::Partial, 1, 0)),
        (GuideAction::Done, O::Skipped, (O::Partial, 1, 0)),
        (GuideAction::Skip, O::Done, (O::Partial, 2, 1)),
        (GuideAction::Skip, O::Partial, (O::Partial, 1, 1)),
        (GuideAction::Skip, O::None, (O::None, 0, 1)),
        (GuideAction::Skip, O::Skipped, (O::None, 0, 1)),
    ];
    for (answer, report, (outcome, done, skipped)) in cases {
        let why = format!("{answer:?} then {report:?}");
        let mut r = guided(RoutineVariant::Seated);
        r.g(answer);
        assert_eq!(
            r.input(Input::RoutineFinished {
                offer: None,
                outcome: report,
            })
            .outcome,
            InputOutcome::Applied,
            "{why}"
        );
        assert_eq!(r.view().guide, None, "{why}");
        assert_eq!(r.view().active_routine, None, "{why}");
        assert_eq!(r.steps().len(), 1, "{why}");
        let s = r.sessions();
        assert_eq!(s.len(), 1, "{why}");
        assert_eq!(s[0].0, r.steps()[0].0, "{why}");
        assert_eq!((s[0].3, s[0].4, s[0].5), (outcome, done, skipped), "{why}");
        assert_counts_include_steps(&r, &why);
        let chip = match outcome {
            O::Done => ChipStatus::Done,
            O::Partial => ChipStatus::Partial,
            _ => ChipStatus::NoneRecorded,
        };
        assert_eq!(r.status(Activity::Movement), chip, "{why}");
        assert_eq!(r.dismissals(), 0, "{why}");
        assert_eq!(
            r.count_notices(|n| matches!(n, Notice::Celebrate(_))),
            usize::from(outcome == O::Done),
            "{why}"
        );
        if outcome.resets_movement_timer() {
            assert_eq!(r.accum(Activity::Movement), Duration::ZERO, "{why}");
        } else {
            assert!(r.accum(Activity::Movement) >= mins(45), "{why}");
            assert_eq!(next_movement(&mut r).0, hm(11, 30), "{why}");
        }
    }
}

#[test]
fn guide_actions_without_a_guide_or_on_the_wrong_screen_change_nothing() {
    let mut h = H::only(true, false, false);
    assert_eq!(
        h.g(GuideAction::Done).outcome,
        InputOutcome::Ignored("no movement guide")
    );
    h.input(Input::RoutineStarted);
    for a in [
        GuideAction::Done,
        GuideAction::Skip,
        GuideAction::Stop,
        GuideAction::Back,
    ] {
        assert!(matches!(h.g(a).outcome, InputOutcome::Ignored(_)), "{a:?}");
    }
    h.g(GuideAction::Variant(RoutineVariant::Seated));
    for a in [
        GuideAction::Later,
        GuideAction::SkipToday,
        GuideAction::OtherDay,
        GuideAction::Close,
    ] {
        assert!(matches!(h.g(a).outcome, InputOutcome::Ignored(_)), "{a:?}");
    }
    assert!(h.steps().is_empty());
    assert_eq!(h.stage(), GuideStage::Step { index: 0, total: 3 });
    // A second routine cannot start over the first.
    assert!(matches!(
        h.input(Input::RoutineStarted).outcome,
        InputOutcome::Ignored(_)
    ));
}

#[test]
fn guide_is_hidden_with_the_pet_during_a_presentation() {
    let mut h = guided(RoutineVariant::Seated);
    h.g(GuideAction::Done);
    h.input(Input::PresentationChanged(true));
    assert_eq!(h.view().bubble, None);
    assert!(!h.guide().open);
    assert!(h.sessions().is_empty());
    // The pet is hidden: nothing to click, the menu's report still works (tray).
    assert!(matches!(
        h.input(Input::OpenPending).outcome,
        InputOutcome::Ignored(_)
    ));
    h.input(Input::PresentationChanged(false));
    assert_behind_the_pet(&h, 1, "after the presentation");
    h.input(Input::PresentationChanged(true));
    h.input(Input::RoutineFinished {
        offer: None,
        outcome: RoutineOutcome::Done,
    });
    assert_eq!(h.sessions()[0].3, RoutineOutcome::Done);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Done);
}

#[test]
fn the_view_reports_a_running_routine_under_any_hold() {
    let manual = Some(ActiveRoutine {
        activity: Activity::Movement,
        offer: None,
    });
    let mut h = H::only(true, true, false);
    h.wait(mins(15));
    assert_eq!(h.view().active_routine, None);
    // Started under a pause: the pet shows the pause, the view still has the routine.
    h.input(Input::Pause(PauseLength::UntilResumed));
    assert_eq!(
        h.input(Input::RoutineStarted).outcome,
        InputOutcome::Applied
    );
    assert!(matches!(h.view().pet, PetMode::Paused { .. }));
    assert_eq!(h.view().active_routine, manual);
    // A second start is refused while it runs; a meeting on top changes nothing.
    assert!(matches!(
        h.input(Input::RoutineStarted).outcome,
        InputOutcome::Ignored(_)
    ));
    h.input(Input::SetMeeting(true));
    h.wait(mins(2));
    assert_eq!(h.view().active_routine, manual);
    // Stopped, none done: the routine ends, the movement timer is not restarted, and
    // the holds stay.
    let before = h.accum(Activity::Movement);
    assert!(before >= mins(15));
    h.input(Input::RoutineFinished {
        offer: None,
        outcome: RoutineOutcome::None,
    });
    assert_eq!(h.view().active_routine, None);
    assert_eq!(h.accum(Activity::Movement), before);
    assert!(matches!(h.view().pet, PetMode::Paused { .. }));
    assert!(h.view().meeting);

    // Started normally, then a pause: still reported; done restarts the timer.
    let mut h = H::only(true, true, false);
    h.wait(mins(15));
    h.input(Input::RoutineStarted);
    h.input(Input::Pause(PauseLength::Minutes(30)));
    assert_eq!(h.view().active_routine, manual);
    h.input(Input::RoutineFinished {
        offer: None,
        outcome: RoutineOutcome::Done,
    });
    assert_eq!(h.view().active_routine, None);
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Done);

    // From a reminder: the routine carries its offer, also under focus.
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    let offer = h.offer();
    h.respond(Response::Start);
    let from_reminder = Some(ActiveRoutine {
        activity: Activity::Movement,
        offer: Some(offer),
    });
    assert_eq!(h.view().active_routine, from_reminder);
    h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: false,
    });
    assert!(matches!(h.view().pet, PetMode::Focus { .. }));
    assert_eq!(h.view().active_routine, from_reminder);
    h.input(Input::RoutineFinished {
        offer: None,
        outcome: RoutineOutcome::Partial,
    });
    assert_eq!(h.view().active_routine, None);
    assert_eq!(h.status(Activity::Movement), ChipStatus::Partial);
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

// ------------------------------------------------- Pomodoro cards under quiet holds

fn classic_pomodoro(h: &mut H) {
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
}

fn is_note(b: Option<Bubble>) -> bool {
    matches!(
        b,
        Some(Bubble::WaterLogged { .. } | Bubble::WaterPanel { .. } | Bubble::PhoneNote { .. })
    )
}

fn is_pomodoro_card(b: Option<Bubble>) -> bool {
    matches!(
        b,
        Some(
            Bubble::PomodoroBreak { .. } | Bubble::PomodoroRoundDone | Bubble::PomodoroPausedByLock
        )
    )
}

#[test]
fn pomodoro_break_due_during_a_meeting_opens_no_card_and_is_not_replayed() {
    let mut h = H::new();
    classic_pomodoro(&mut h);
    h.wait(mins(20));
    h.input(Input::SetMeeting(true));
    h.wait(mins(10)); // the work phase ends at 10:25, inside the meeting
    assert_eq!(h.view().hold, Some(Hold::FocusPresentationMeeting));
    assert_eq!(h.view().pet, PetMode::Meeting);
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    assert_eq!(h.view().bubble, None);
    // The user may still open it on purpose during the meeting, and close it again.
    h.input(Input::OpenPending);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
    h.wait(mins(1));
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
    h.input(Input::CloseBubble);
    h.wait(mins(5));
    h.input(Input::SetMeeting(false));
    // Nothing comes back when the meeting ends; the phase is kept for the user.
    assert_eq!(h.view().bubble, None);
    h.wait(mins(1));
    assert!(!is_pomodoro_card(h.view().bubble));
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    h.input(Input::OpenPending);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
}

#[test]
fn pomodoro_round_done_during_a_meeting_opens_no_card() {
    let mut h = H::new();
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Custom {
            work_min: 10,
            break_min: 5,
        },
        replace_focus: false,
        resume_coaching: false,
    });
    h.wait(mins(10));
    h.input(Input::StartPomodoroBreak);
    h.input(Input::SetMeeting(true));
    h.wait(mins(6));
    assert_eq!(h.view().pomodoro.unwrap().phase, PomodoroPhase::RoundDone);
    assert_eq!(h.view().bubble, None);
    h.input(Input::SetMeeting(false));
    assert_eq!(h.view().bubble, None);
}

#[test]
fn pomodoro_break_due_after_hours_opens_no_card_and_is_not_replayed_next_morning() {
    let mut h = H::at(
        ist(20, 18, 15),
        CoachConfig::default(),
        WaterSnapshot::default(),
    );
    classic_pomodoro(&mut h);
    h.wait(mins(25)); // ends at 18:40, after the 18:30 end of work hours
    assert_eq!(h.view().hold, Some(Hold::PauseLockOffHours));
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    assert_eq!(h.view().bubble, None);
    // Through the night and into the next working morning, the card never opens itself.
    h.tick = mins(1);
    let start = h.today();
    while h.today() == start || h.local() < hm(9, 45) {
        h.wait(mins(1));
        assert!(!is_pomodoro_card(h.view().bubble), "card at {}", h.local());
    }
    assert_ne!(h.view().hold, Some(Hold::PauseLockOffHours));
}

#[test]
fn entering_pause_lock_or_meeting_withdraws_an_open_pomodoro_card_without_replay() {
    #[derive(Debug, Clone, Copy)]
    enum Q {
        Pause,
        Lock,
        Meeting,
    }
    for card_is_round_done in [false, true] {
        for q in [Q::Pause, Q::Lock, Q::Meeting] {
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
            if card_is_round_done {
                h.input(Input::SkipPomodoroBreak);
                assert_eq!(h.view().bubble, Some(Bubble::PomodoroRoundDone));
            } else {
                assert!(matches!(
                    h.view().bubble,
                    Some(Bubble::PomodoroBreak { .. })
                ));
            }
            let phase = h.view().pomodoro.unwrap().phase;
            let ctx = format!("{q:?}, round done {card_is_round_done}");
            match q {
                Q::Pause => h.input(Input::Pause(PauseLength::Minutes(30))),
                Q::Lock => h.input(Input::Lock),
                Q::Meeting => h.input(Input::SetMeeting(true)),
            };
            assert_eq!(h.view().bubble, None, "{ctx}: card withdrawn on entry");
            h.wait(mins(5));
            assert_eq!(h.view().bubble, None, "{ctx}: stays withdrawn");
            assert_eq!(h.view().pomodoro.unwrap().phase, phase, "{ctx}: phase kept");
            match q {
                Q::Pause => h.input(Input::ResumeCoaching),
                Q::Lock => h.input(Input::Unlock),
                Q::Meeting => h.input(Input::SetMeeting(false)),
            };
            assert_eq!(h.view().bubble, None, "{ctx}: no replay on exit");
            h.wait(mins(1));
            assert!(!is_pomodoro_card(h.view().bubble), "{ctx}: no replay later");
            // The phase is still there for the user to open on purpose.
            h.input(Input::OpenPending);
            assert!(is_pomodoro_card(h.view().bubble), "{ctx}: user can open it");
        }
    }
}

#[test]
fn a_card_opened_on_purpose_is_withdrawn_by_a_new_hold() {
    let mut h = H::new();
    classic_pomodoro(&mut h);
    h.wait(mins(20));
    h.input(Input::SetMeeting(true));
    h.wait(mins(10));
    h.input(Input::OpenPending); // opened on purpose during the meeting
    h.input(Input::SetMeeting(false)); // the card stays: the user asked for it
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
    h.input(Input::Pause(PauseLength::Minutes(30))); // a new hold starts
    assert_eq!(h.view().bubble, None);
    h.input(Input::ResumeCoaching);
    assert_eq!(h.view().bubble, None);
}

/// How a Pomodoro card the user opens during a meeting came to be waiting.
#[derive(Debug, Clone, Copy)]
enum HeldCard {
    /// Work ended during the meeting; the user opens the break card.
    BreakOffered,
    /// The break ended during the meeting; the user opens the round-done card.
    RoundDone,
    /// The user skipped the break during the meeting, which opens the round-done card.
    SkippedToRoundDone,
}

/// A hold that starts and ends while the meeting stays on.
#[derive(Debug, Clone, Copy)]
enum NewHold {
    Lock,
    Suspend,
    Pause,
    OffHours,
}

fn is_card(card: HeldCard, b: Option<Bubble>) -> bool {
    match card {
        HeldCard::BreakOffered => matches!(b, Some(Bubble::PomodoroBreak { .. })),
        HeldCard::RoundDone | HeldCard::SkippedToRoundDone => b == Some(Bubble::PomodoroRoundDone),
    }
}

/// Start a 10/2 Pomodoro, turn the meeting on, and leave a card of `card`'s kind open
/// because the user asked for it during the meeting.
fn card_opened_during_a_meeting(card: HeldCard) -> H {
    let mut h = H::new();
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Custom {
            work_min: 10,
            break_min: 2,
        },
        replace_focus: false,
        resume_coaching: false,
    });
    h.input(Input::SetMeeting(true));
    h.wait(mins(10));
    assert_eq!(
        h.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    assert_eq!(h.view().bubble, None, "no card opens itself in the meeting");
    match card {
        HeldCard::BreakOffered => {
            h.input(Input::OpenPending);
        }
        HeldCard::RoundDone => {
            h.input(Input::StartPomodoroBreak);
            h.wait(mins(2));
            assert_eq!(h.view().pomodoro.unwrap().phase, PomodoroPhase::RoundDone);
            assert_eq!(h.view().bubble, None, "no card opens itself in the meeting");
            h.input(Input::OpenPending);
        }
        HeldCard::SkippedToRoundDone => {
            h.input(Input::SkipPomodoroBreak);
        }
    }
    assert!(
        is_card(card, h.view().bubble),
        "{card:?}: the user opened it"
    );
    h
}

fn start_hold(h: &mut H, hold: NewHold) {
    match hold {
        NewHold::Lock => {
            h.input(Input::Lock);
        }
        NewHold::Suspend => {
            h.input(Input::Suspend);
        }
        NewHold::Pause => {
            h.input(Input::Pause(PauseLength::Minutes(30)));
        }
        NewHold::OffHours => {
            // The working day ends (the clock moves to 22:00 the same evening).
            h.wall += chrono::Duration::hours(11);
            h.input(Input::Refresh);
            assert_eq!(h.view().session, SessionState::OffHours);
        }
    }
}

fn end_hold(h: &mut H, hold: NewHold) {
    match hold {
        NewHold::Lock => {
            h.step(secs(30), Input::Tick { idle: secs(30) });
            h.input(Input::Unlock);
        }
        NewHold::Suspend => {
            h.step(secs(30), Input::Resume);
        }
        NewHold::Pause => {
            h.wait(secs(30));
            h.input(Input::ResumeCoaching);
        }
        NewHold::OffHours => {
            h.wall -= chrono::Duration::hours(11);
            h.input(Input::Refresh);
        }
    }
}

#[test]
fn a_card_opened_during_a_meeting_is_withdrawn_by_an_overlapping_new_hold() {
    for card in [
        HeldCard::BreakOffered,
        HeldCard::RoundDone,
        HeldCard::SkippedToRoundDone,
    ] {
        for hold in [
            NewHold::Lock,
            NewHold::Suspend,
            NewHold::Pause,
            NewHold::OffHours,
        ] {
            let ctx = format!("{card:?} then {hold:?}");
            let mut h = card_opened_during_a_meeting(card);
            let phase = h.view().pomodoro.unwrap().phase;
            start_hold(&mut h, hold);
            assert_eq!(
                h.view().bubble,
                None,
                "{ctx}: withdrawn when the hold starts"
            );
            end_hold(&mut h, hold);
            // The meeting is still on: the old card must not come back by itself.
            assert!(h.view().meeting, "{ctx}: meeting still on");
            assert_eq!(h.view().hold, Some(Hold::FocusPresentationMeeting), "{ctx}");
            assert_eq!(h.view().bubble, None, "{ctx}: not back after the hold");
            h.wait(mins(5));
            assert!(
                !is_pomodoro_card(h.view().bubble),
                "{ctx}: not back later in the meeting"
            );
            assert_eq!(h.view().pomodoro.unwrap().phase, phase, "{ctx}: phase kept");
            // Opening it on purpose still works, and the card then stays for the rest of
            // the meeting and after it.
            h.input(Input::OpenPending);
            assert!(is_card(card, h.view().bubble), "{ctx}: user can reopen it");
            h.wait(mins(1));
            assert!(is_card(card, h.view().bubble), "{ctx}: reopened card stays");
            h.input(Input::SetMeeting(false));
            assert!(
                is_card(card, h.view().bubble),
                "{ctx}: stays after the meeting"
            );
        }
    }
}

#[test]
fn a_card_opened_during_a_meeting_stays_while_only_that_meeting_holds() {
    let mut h = card_opened_during_a_meeting(HeldCard::BreakOffered);
    h.wait(mins(3));
    assert!(is_card(HeldCard::BreakOffered, h.view().bubble));
    // A duplicate meeting toggle is not a new hold.
    h.input(Input::SetMeeting(true));
    assert!(is_card(HeldCard::BreakOffered, h.view().bubble));
    h.input(Input::CloseBubble);
    assert_eq!(h.view().bubble, None);
    h.wait(mins(1));
    assert_eq!(h.view().bubble, None, "closed by the user, not reopened");
}

#[test]
fn a_hold_that_ends_and_begins_again_withdraws_a_card_opened_under_it() {
    let mut h = card_opened_during_a_meeting(HeldCard::BreakOffered);
    h.input(Input::Pause(PauseLength::UntilResumed));
    assert_eq!(h.view().bubble, None);
    // Opened on purpose under the meeting and the pause.
    h.input(Input::OpenPending);
    assert!(is_card(HeldCard::BreakOffered, h.view().bubble));
    // The pause ends: the card stays, the meeting still covers it.
    h.input(Input::ResumeCoaching);
    assert!(is_card(HeldCard::BreakOffered, h.view().bubble));
    // A new pause is a new hold.
    h.input(Input::Pause(PauseLength::UntilResumed));
    assert_eq!(h.view().bubble, None);
    h.input(Input::ResumeCoaching);
    assert_eq!(h.view().bubble, None);
    // Likewise the meeting itself: opened under it, it ends and begins again.
    h.input(Input::OpenPending);
    assert!(is_card(HeldCard::BreakOffered, h.view().bubble));
    h.input(Input::SetMeeting(false));
    assert!(is_card(HeldCard::BreakOffered, h.view().bubble));
    h.input(Input::SetMeeting(true));
    assert_eq!(h.view().bubble, None);
    h.input(Input::SetMeeting(false));
    assert_eq!(h.view().bubble, None);
}

#[test]
fn pomodoro_unlock_card_waits_for_no_other_hold() {
    let mut h = H::new();
    classic_pomodoro(&mut h);
    h.wait(mins(10));
    h.input(Input::SetMeeting(true));
    h.input(Input::Lock);
    h.step(secs(30), Input::Tick { idle: secs(30) });
    h.input(Input::Unlock);
    // Still in the meeting: the "paused while you were away" card does not open itself.
    assert_eq!(h.view().bubble, None);
    let p = h.view().pomodoro.unwrap();
    assert_eq!(p.paused, Some(PomodoroPause::LockOrSleep));
    h.input(Input::SetMeeting(false));
    assert_eq!(h.view().bubble, None);
    // The explicit resume path still works.
    h.input(Input::OpenPending);
    assert_eq!(h.view().bubble, Some(Bubble::PomodoroPausedByLock));
    h.input(Input::ResumePomodoro);
    assert_eq!(h.view().pomodoro.unwrap().paused, None);
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

// ------------------------------------------------- deadline day and recovery day

impl H {
    fn check_ins(&self) -> Vec<(CheckInOrigin, CheckInReply)> {
        self.events
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::RecoveryCheckIn { origin, answer } => Some((origin, answer)),
                _ => None,
            })
            .collect()
    }

    fn plan_changes(&self) -> Vec<(NaiveDate, PlanChange)> {
        self.events
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::RecoveryPlanChanged { plan_date, change } => {
                    Some((plan_date, change))
                }
                _ => None,
            })
            .collect()
    }

    fn reflections(&self) -> Vec<ReflectionAnswer> {
        self.events
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::RecoveryReflection { answer } => Some(answer),
                _ => None,
            })
            .collect()
    }

    fn rec(&mut self, a: RecoveryAction) -> StepOutput {
        self.input(Input::Recovery(a))
    }
}

fn oct(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

/// No reminders and no long-day check-in, so only the recovery cards appear.
fn quiet_config() -> CoachConfig {
    CoachConfig {
        long_day_check_in: LongDayCheckIn {
            enabled: false,
            hours: 9,
        },
        ..cats(false, false, false)
    }
}

#[test]
fn long_day_check_in_yes_is_recorded_and_invites_a_recovery_day() {
    let mut h = long_day(LongDayCheckIn::default());
    h.wait_until(mins(9 * 60 + 5), |v| v.bubble.is_some());
    h.input(Input::CheckIn(CheckInAnswer::Yes));
    assert_eq!(
        h.check_ins(),
        vec![(CheckInOrigin::LongDay, CheckInReply::Yes)]
    );
    assert!(h.view().recovery.marked_today);
    // After hours, the invitation follows the answer (the user is there to see it).
    assert_eq!(
        h.view().bubble,
        Some(Bubble::RecoveryInvite { next: oct(21) })
    );
    h.rec(RecoveryAction::PlanFor(oct(21)));
    let plan = h.view().recovery.plan.unwrap();
    assert_eq!(plan.date, oct(21));
    assert_eq!(
        (plan.walk_at, plan.finish_at),
        (Some(hm(16, 0)), Some(hm(18, 30)))
    );
    assert_eq!(h.view().bubble, Some(Bubble::RecoveryPlanned { plan }));
    assert_eq!(h.plan_changes(), vec![(oct(21), PlanChange::Saved)]);
    h.rec(RecoveryAction::Ok);
    assert_eq!(h.view().bubble, None);
}

#[test]
fn long_day_check_in_no_and_dont_ask_today_are_recorded_and_ask_nothing_more() {
    for (answer, reply) in [
        (CheckInAnswer::No, CheckInReply::No),
        (CheckInAnswer::DontAskToday, CheckInReply::DontAskToday),
    ] {
        let mut h = long_day(LongDayCheckIn::default());
        h.wait_until(mins(9 * 60 + 5), |v| v.bubble.is_some());
        h.input(Input::CheckIn(answer));
        assert_eq!(h.check_ins(), vec![(CheckInOrigin::LongDay, reply)]);
        assert!(!h.view().recovery.marked_today);
        assert_eq!(h.view().recovery.plan, None);
        h.wait(mins(180));
        assert_eq!(h.view().bubble, None);
        assert_eq!(h.check_ins().len(), 1);
    }
}

#[test]
fn deadline_day_from_the_menu_marks_today_and_plans_a_chosen_date() {
    let mut h = H::at(ist(20, 15, 0), quiet_config(), WaterSnapshot::default());
    assert!(h.view().recovery.can_open);
    h.input(Input::DeadlineDay);
    assert_eq!(h.view().bubble, Some(Bubble::DeadlineDay));
    assert!(!h.view().recovery.can_open);
    h.input(Input::CheckIn(CheckInAnswer::Yes));
    assert_eq!(
        h.check_ins(),
        vec![(CheckInOrigin::Menu, CheckInReply::Yes)]
    );
    assert!(
        h.notices
            .iter()
            .any(|(_, n)| *n == Notice::DeadlineDayConfirmed(oct(20)))
    );
    assert_eq!(
        h.view().bubble,
        Some(Bubble::RecoveryInvite { next: oct(21) })
    );
    h.rec(RecoveryAction::ChooseDate);
    // The next working days after the first suggestion (Sunday is not a workday).
    assert_eq!(
        h.view().bubble,
        Some(Bubble::RecoveryDates {
            dates: [Some(oct(22)), Some(oct(23)), Some(oct(24)), Some(oct(26))]
        })
    );
    h.rec(RecoveryAction::PlanFor(oct(24)));
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::RecoveryPlanned { plan }) if plan.date == oct(24)
    ));
    h.rec(RecoveryAction::Ok);
    // Already marked with a plan pending: the menu shows the planned day again.
    h.input(Input::DeadlineDay);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::RecoveryPlanned { .. })
    ));
    h.input(Input::CloseBubble);
    h.rec(RecoveryAction::ShowPlan);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::RecoveryPlanned { .. })
    ));
    h.rec(RecoveryAction::CancelPlan);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.view().recovery.plan, None);
    assert_eq!(
        h.plan_changes(),
        vec![
            (oct(24), PlanChange::Saved),
            (oct(24), PlanChange::Cancelled)
        ]
    );
    // Marked, no plan: the invitation again ("Plan a recovery day").
    h.wait(CALM_GAP);
    h.input(Input::DeadlineDay);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::RecoveryInvite { .. })
    ));
    h.rec(RecoveryAction::NoThanks);
    assert_eq!(h.view().bubble, None);
    assert_eq!(h.check_ins().len(), 1);
}

#[test]
fn deadline_day_no_and_later_change_nothing() {
    let mut h = H::at(ist(20, 15, 0), quiet_config(), WaterSnapshot::default());
    h.input(Input::DeadlineDay);
    h.input(Input::CheckIn(CheckInAnswer::No));
    assert_eq!(h.check_ins(), vec![(CheckInOrigin::Menu, CheckInReply::No)]);
    assert!(!h.view().recovery.marked_today);
    assert_eq!(h.view().bubble, None);
    h.input(Input::DeadlineDay);
    h.input(Input::CheckIn(CheckInAnswer::Yes));
    h.rec(RecoveryAction::Later);
    assert_eq!(h.view().bubble, None);
    assert!(h.view().recovery.marked_today);
    assert!(h.plan_changes().is_empty());
}

#[test]
fn deadline_day_waits_for_a_free_bubble_and_follows_the_one_at_a_time_rule() {
    let mut h = H::only(true, false, false);
    h.wait_until(mins(50), |v| v.reminder().is_some());
    assert_eq!(
        h.input(Input::DeadlineDay).outcome,
        InputOutcome::Ignored("another card is open")
    );
    h.respond(Response::Skip);
    // Asked for: it opens at once, even inside the calm gap.
    h.input(Input::DeadlineDay);
    assert_eq!(h.view().bubble, Some(Bubble::DeadlineDay));
    // No reminder while the card is open.
    h.input(Input::CheckIn(CheckInAnswer::Yes));
    h.wait(mins(5));
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::RecoveryInvite { .. })
    ));
    assert_eq!(h.offers().len(), 1);
    // Unanswered, the invitation closes quietly after 10 minutes, recording nothing.
    h.wait(mins(5));
    assert_eq!(h.view().bubble, None);
    assert!(h.plan_changes().is_empty());
    // Recovery answers with no card open are ignored.
    assert!(matches!(
        h.rec(RecoveryAction::PlanFor(oct(21))).outcome,
        InputOutcome::Ignored(_)
    ));
    assert!(matches!(
        h.input(Input::CheckIn(CheckInAnswer::Yes)).outcome,
        InputOutcome::Ignored(_)
    ));
}

#[test]
fn deadline_day_card_stays_in_a_meeting_and_a_new_hold_withdraws_it() {
    let mut h = H::at(ist(20, 15, 0), quiet_config(), WaterSnapshot::default());
    h.input(Input::SetMeeting(true));
    h.input(Input::DeadlineDay);
    h.wait(mins(2));
    assert_eq!(h.view().bubble, Some(Bubble::DeadlineDay));
    h.input(Input::Lock);
    assert_eq!(h.view().bubble, None);
    h.step(secs(30), Input::Unlock);
    assert_eq!(h.view().bubble, None);
    assert!(h.check_ins().is_empty());
}

#[test]
fn plan_dates_are_after_today_and_within_thirty_days() {
    let mut h = H::at(ist(20, 15, 0), quiet_config(), WaterSnapshot::default());
    let mut plan = RecoveryPlan::default_for(oct(20), &h.schedule, &h.zone);
    assert!(matches!(
        h.rec(RecoveryAction::SavePlan(plan)).outcome,
        InputOutcome::Invalid(_)
    ));
    plan.date = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
    assert!(matches!(
        h.rec(RecoveryAction::SavePlan(plan)).outcome,
        InputOutcome::Invalid(_)
    ));
    plan.date = oct(22);
    assert_eq!(
        h.rec(RecoveryAction::SavePlan(plan)).outcome,
        InputOutcome::Applied
    );
    // One pending plan: a new date replaces it.
    plan.date = oct(23);
    h.rec(RecoveryAction::SavePlan(plan));
    assert_eq!(h.view().recovery.plan.unwrap().date, oct(23));
    assert_eq!(
        h.plan_changes(),
        vec![
            (oct(22), PlanChange::Saved),
            (oct(22), PlanChange::Cancelled),
            (oct(23), PlanChange::Saved)
        ]
    );
    assert!(h.e.recovery_plan().is_some());
}

/// Plan Wednesday 21 Oct from Tuesday evening (with `plan`'s items), sleep overnight and
/// wake at 09:00 on the recovery day.
fn recovery_day(config: CoachConfig, edit: impl FnOnce(&mut RecoveryPlan)) -> H {
    let mut h = H::at(ist(20, 17, 0), config, WaterSnapshot::default());
    let mut plan = RecoveryPlan::default_for(oct(21), &h.schedule, &h.zone);
    edit(&mut plan);
    assert_eq!(
        h.rec(RecoveryAction::SavePlan(plan)).outcome,
        InputOutcome::Applied
    );
    h.input(Input::Suspend);
    h.step(mins(16 * 60), Input::Resume);
    assert_eq!((h.today(), h.local()), (oct(21), hm(9, 0)));
    assert!(h.view().recovery.recovery_day);
    h.tick = secs(30);
    h
}

#[test]
fn recovery_day_morning_card_walk_and_end_of_day_question() {
    let mut h = recovery_day(quiet_config(), |_| {});
    h.water.goal_ml = Some(2000);
    // The morning card, once, after the calm gap that follows the start of work hours.
    h.wait_until(mins(40), |v| v.bubble.is_some());
    assert_eq!(h.local(), hm(9, 35));
    let plan = h.view().recovery.plan.unwrap();
    assert_eq!(
        h.view().bubble,
        Some(Bubble::RecoveryMorning {
            plan,
            water_goal_ml: Some(2000)
        })
    );
    h.rec(RecoveryAction::Ok);
    // The walk suggestion at 16:00, once (no repeat chosen).
    h.wait_until(mins(7 * 60), |v| v.bubble.is_some());
    assert_eq!(
        (h.local(), h.view().bubble),
        (hm(16, 0), Some(Bubble::RecoveryWalk))
    );
    h.rec(RecoveryAction::WalkNotNow);
    // The end-of-day question after the planned finish (18:30), once.
    h.wait_until(mins(3 * 60), |v| v.bubble.is_some());
    assert_eq!(
        (h.local(), h.view().bubble),
        (hm(18, 30), Some(Bubble::RecoveryReflection))
    );
    h.rec(RecoveryAction::Reflect(Reflection::Somewhat));
    assert_eq!(h.reflections(), vec![ReflectionAnswer::Somewhat]);
    h.wait(mins(5 * 60));
    assert_eq!(h.view().bubble, None);
    // The next day the plan is over; nothing is carried over.
    h.wait(mins(60));
    assert_eq!(h.today(), oct(22));
    assert_eq!(h.view().recovery.plan, None);
    assert!(!h.view().recovery.recovery_day);
}

#[test]
fn recovery_day_skipping_the_question_records_nothing_and_cards_close_unanswered() {
    let mut h = recovery_day(quiet_config(), |_| {});
    h.wait_until(mins(40), |v| v.bubble.is_some());
    // Unanswered, the morning card closes after 10 minutes and is not shown again.
    h.wait(mins(10));
    assert_eq!(h.view().bubble, None);
    h.wait_until(mins(9 * 60), |v| {
        v.bubble == Some(Bubble::RecoveryReflection)
    });
    h.rec(RecoveryAction::Reflect(Reflection::Skip));
    assert!(h.reflections().is_empty());
}

#[test]
fn recovery_day_breaks_come_a_step_more_often_and_water_is_unchanged() {
    let mut h = recovery_day(cats(true, false, false), |_| {});
    h.wait_until(mins(40), |v| v.bubble.is_some());
    h.rec(RecoveryAction::Ok);
    // Balanced is 45 minutes; on the recovery day movement comes every 30.
    h.wait_until(mins(60), |v| v.reminder().is_some());
    assert_eq!(h.local(), hm(10, 0));

    let mut off = recovery_day(cats(true, false, false), |p| p.more_breaks = false);
    off.wait_until(mins(40), |v| v.bubble.is_some());
    off.rec(RecoveryAction::Ok);
    off.wait_until(mins(60), |v| v.reminder().is_some());
    assert_eq!(off.local(), hm(10, 15));
}

#[test]
fn recovery_walk_repeats_every_ninety_minutes_and_a_held_one_is_not_replayed() {
    let mut h = recovery_day(quiet_config(), |p| {
        p.walk_at = Some(hm(11, 0));
        p.walk_repeat = true;
        p.reflection = false;
    });
    h.wait_until(mins(40), |v| v.bubble.is_some());
    h.rec(RecoveryAction::Ok);
    let mut walks = Vec::new();
    // A meeting covers the 12:30 suggestion.
    while h.local() < hm(18, 40) {
        if h.local() == hm(12, 25) {
            h.input(Input::SetMeeting(true));
        }
        if h.local() == hm(12, 45) {
            h.input(Input::SetMeeting(false));
        }
        if h.view().bubble == Some(Bubble::RecoveryWalk) {
            walks.push(h.local());
            h.rec(RecoveryAction::WalkNow);
        }
        h.wait(h.tick);
    }
    // 11:00, (12:30 held), 14:00, 15:30, 17:00; none at or after the 18:30 finish.
    assert_eq!(walks, vec![hm(11, 0), hm(14, 0), hm(15, 30), hm(17, 0)]);
}

/// The repeating walk from 11:00 on the recovery day, with the morning card acknowledged.
/// `at` runs at each tick before the walk check (it may answer or open something); the
/// local times of the walk suggestions shown until 18:40 are returned, each answered.
fn walks_with(mut at: impl FnMut(&mut H)) -> Vec<NaiveTime> {
    let mut h = recovery_day(quiet_config(), |p| {
        p.walk_at = Some(hm(11, 0));
        p.walk_repeat = true;
        p.reflection = false;
    });
    h.wait_until(mins(40), |v| v.bubble.is_some());
    h.rec(RecoveryAction::Ok);
    let mut walks = Vec::new();
    while h.local() < hm(18, 40) {
        at(&mut h);
        if h.view().bubble == Some(Bubble::RecoveryWalk) {
            walks.push(h.local());
            h.rec(RecoveryAction::WalkNow);
        }
        h.wait(h.tick);
    }
    walks
}

/// The walks after a 12:30 slot that was used up: the next ones come at their times.
fn walks_without_1230() -> Vec<NaiveTime> {
    vec![hm(11, 0), hm(14, 0), hm(15, 30), hm(17, 0)]
}

#[test]
fn recovery_walk_due_in_a_short_meeting_is_not_shown_after_it() {
    // The meeting ends at 12:31; its calm gap ends at 12:36, still inside the 12:30
    // walk's 10-minute window. The walk time passed while held: it is used up.
    let walks = walks_with(|h| {
        if h.local() == hm(12, 25) {
            h.input(Input::SetMeeting(true));
        }
        if h.local() == hm(12, 31) {
            h.input(Input::SetMeeting(false));
        }
    });
    assert_eq!(walks, walks_without_1230());
}

#[test]
fn recovery_walk_due_while_another_card_is_open_is_not_shown_after_it() {
    // "Deadline day?" from the pet menu is open at 12:30 and answered at 12:31.
    let walks = walks_with(|h| {
        if h.local() == hm(12, 29) {
            assert_eq!(h.input(Input::DeadlineDay).outcome, InputOutcome::Applied);
        }
        if h.local() == hm(12, 31) {
            assert_eq!(
                h.input(Input::CheckIn(CheckInAnswer::No)).outcome,
                InputOutcome::Applied
            );
        }
    });
    assert_eq!(walks, walks_without_1230());
}

#[test]
fn recovery_walk_due_while_the_guide_is_open_is_not_shown_after_it() {
    // The movement guide is open from 12:28 and closed with "Later" at 12:32.
    let walks = walks_with(|h| {
        if h.local() == hm(12, 28) {
            assert_eq!(
                h.input(Input::RoutineStarted).outcome,
                InputOutcome::Applied
            );
        }
        if h.local() == hm(12, 32) {
            assert_eq!(
                h.input(Input::Guide(GuideAction::Later)).outcome,
                InputOutcome::Applied
            );
        }
    });
    assert_eq!(walks, walks_without_1230());
}

#[test]
fn recovery_walk_after_only_a_calm_gap_is_delayed_inside_its_window() {
    // The calm gap after a card that closed before the walk time only delays it inside
    // its window: answered at 12:27, the 12:30 walk comes when the gap ends at 12:32.
    let walks = walks_with(|h| {
        if h.local() == hm(12, 26) {
            h.input(Input::DeadlineDay);
        }
        if h.local() == hm(12, 27) {
            h.input(Input::CheckIn(CheckInAnswer::No));
        }
    });
    assert_eq!(
        walks,
        vec![hm(11, 0), hm(12, 32), hm(14, 0), hm(15, 30), hm(17, 0)]
    );
}

#[test]
fn recovery_day_cancelled_on_the_day_stops_its_cards() {
    let mut h = recovery_day(quiet_config(), |_| {});
    h.wait_until(mins(40), |v| v.bubble.is_some());
    h.rec(RecoveryAction::CancelPlan);
    assert_eq!(h.view().bubble, None);
    assert_eq!(
        h.plan_changes().last(),
        Some(&(oct(21), PlanChange::Cancelled))
    );
    h.wait(mins(10 * 60));
    assert_eq!(h.view().bubble, None);
}

#[test]
fn recovery_plan_and_marks_survive_a_restart() {
    let mut h = H::at(ist(20, 15, 0), quiet_config(), WaterSnapshot::default());
    h.input(Input::DeadlineDay);
    h.input(Input::CheckIn(CheckInAnswer::Yes));
    h.rec(RecoveryAction::PlanFor(oct(21)));
    let state = h.e.state().clone();
    let json = serde_json::to_string(&state).unwrap();
    let back: EngineState = serde_json::from_str(&json).unwrap();
    h.e = Engine::from_state(back).unwrap();
    h.restart(mins(1));
    // The open card closes at the restart; the mark and the plan stay.
    assert_eq!(h.view().bubble, None);
    assert!(h.view().recovery.marked_today);
    assert_eq!(h.view().recovery.plan.unwrap().date, oct(21));
}

// ------------------------------------------- water and phone notes (B3, B8)

impl H {
    /// The shell recorded a water entry: it joins today's entries and the total, and the
    /// engine is told.
    fn log(&mut self, ml: u32) -> (Uuid, StepOutput) {
        self.seq += 1;
        let id = Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0000 + self.seq);
        self.entries.push(WaterEntry {
            id,
            time: self.local(),
            amount_ml: ml,
        });
        self.water.logged_ml += ml;
        let out = self.input(Input::Water(WaterInput::Logged { entry: id, ml }));
        (id, out)
    }

    /// The shell removed entry `id` (Undo or Remove).
    fn unlog(&mut self, id: Uuid) {
        let i = self.entries.iter().position(|e| e.id == id).unwrap();
        let e = self.entries.remove(i);
        self.water.logged_ml = self.water.logged_ml.saturating_sub(e.amount_ml);
    }

    fn w(&mut self, input: WaterInput) -> StepOutput {
        self.input(Input::Water(input))
    }

    fn note(&self) -> Option<Bubble> {
        self.view().bubble.filter(|b| {
            matches!(
                b,
                Bubble::WaterLogged { .. } | Bubble::WaterPanel { .. } | Bubble::PhoneNote { .. }
            )
        })
    }
}

fn water_goal() -> H {
    H::at(
        ist(20, 10, 0),
        cats(false, false, true),
        WaterSnapshot {
            logged_ml: 0,
            goal_ml: Some(2000),
        },
    )
}

#[test]
fn a_logged_entry_opens_its_confirmation_and_undo_keeps_it_open() {
    let mut h = water_goal();
    let (id, out) = h.log(250);
    assert_eq!(out.outcome, InputOutcome::Applied);
    assert_eq!(
        h.note(),
        Some(Bubble::WaterLogged {
            entry: Some(id),
            ml: 250
        })
    );
    assert_eq!(h.view().water_entries.len(), 1);
    h.unlog(id);
    assert_eq!(h.w(WaterInput::Undone).outcome, InputOutcome::Applied);
    assert_eq!(
        h.note(),
        Some(Bubble::WaterLogged {
            entry: None,
            ml: 250
        })
    );
    // A second Undo has nothing to undo.
    assert!(matches!(
        h.w(WaterInput::Undone).outcome,
        InputOutcome::Ignored(_)
    ));
    // Out-of-range amounts are refused.
    assert!(matches!(
        h.w(WaterInput::Logged {
            entry: Uuid::nil(),
            ml: 0
        })
        .outcome,
        InputOutcome::Invalid(_)
    ));
    assert!(matches!(
        h.w(WaterInput::Logged {
            entry: Uuid::nil(),
            ml: 2001
        })
        .outcome,
        InputOutcome::Invalid(_)
    ));
    // Details opens the panel; it closes after three minutes without an answer, then the
    // calm gap follows.
    h.w(WaterInput::OpenPanel);
    assert!(matches!(
        h.note(),
        Some(Bubble::WaterPanel {
            edit: None,
            other_ml: 250
        })
    ));
    h.wait(mins(2) + secs(55));
    assert!(h.note().is_some());
    h.wait(secs(10));
    assert_eq!(h.note(), None);
    assert!(h.count_notices(|n| matches!(n, Notice::Celebrate(_))) == 0);
}

#[test]
fn the_panel_edits_steps_and_cancels_within_its_range() {
    let mut h = water_goal();
    let (a, _) = h.log(250);
    let (b, _) = h.log(1990);
    h.w(WaterInput::OpenPanel);
    // Editing needs an entry that stands today.
    assert!(matches!(
        h.w(WaterInput::Edit(Uuid::nil())).outcome,
        InputOutcome::Ignored(_)
    ));
    assert!(matches!(
        h.w(WaterInput::StepEdit { up: true }).outcome,
        InputOutcome::Ignored(_)
    ));
    h.w(WaterInput::Edit(b));
    h.w(WaterInput::StepEdit { up: true });
    h.w(WaterInput::StepEdit { up: true });
    assert_eq!(
        h.note(),
        Some(Bubble::WaterPanel {
            edit: Some(WaterEdit { entry: b, ml: 2000 }),
            other_ml: 250
        }),
        "the amount stays within 2000 ml"
    );
    // The other amount steps on its own while an entry is edited.
    h.w(WaterInput::StepOther { up: false });
    for _ in 0..10 {
        h.w(WaterInput::StepOther { up: false });
    }
    assert_eq!(
        h.note(),
        Some(Bubble::WaterPanel {
            edit: Some(WaterEdit { entry: b, ml: 2000 }),
            other_ml: 50
        }),
        "and stays at 50 ml or more"
    );
    h.w(WaterInput::CancelEdit);
    assert!(matches!(
        h.note(),
        Some(Bubble::WaterPanel { edit: None, .. })
    ));
    // An entry removed elsewhere stops being edited.
    h.w(WaterInput::Edit(a));
    h.unlog(a);
    h.wait(secs(5));
    assert!(matches!(
        h.note(),
        Some(Bubble::WaterPanel { edit: None, .. })
    ));
    // Changed (a save or a removal) ends editing too; the panel stays.
    h.w(WaterInput::Edit(b));
    assert_eq!(h.w(WaterInput::Changed).outcome, InputOutcome::Applied);
    assert!(matches!(
        h.note(),
        Some(Bubble::WaterPanel { edit: None, .. })
    ));
    // An answer restarts the panel's timer.
    h.wait(mins(2));
    h.w(WaterInput::StepOther { up: true });
    h.wait(mins(2));
    assert!(h.note().is_some());
    // Logged in the panel: the panel stays and lists it.
    let (c, _) = h.log(150);
    assert!(matches!(h.note(), Some(Bubble::WaterPanel { .. })));
    assert!(h.view().water_entries.iter().any(|e| e.id == c));
}

#[test]
fn reaching_the_goal_celebrates_once_and_hydration_reminders_pause() {
    let mut h = water_goal();
    h.log(1900);
    h.log(150);
    h.log(150);
    let celebrations = h.count_notices(|n| matches!(n, Notice::Celebrate(Activity::Water)));
    assert_eq!(celebrations, 1);
    assert!(h.view().water.unwrap().goal_reached);
    let before = h.offers().len();
    h.input(Input::CloseBubble);
    h.wait(mins(240));
    assert_eq!(
        h.offers().len(),
        before,
        "no hydration reminder after the goal"
    );
}

#[test]
fn logging_and_undoing_moves_the_water_pace() {
    // 13:00: well behind a 2000 ml pace with nothing logged.
    let mut h = H::at(
        ist(20, 13, 0),
        cats(false, false, true),
        WaterSnapshot {
            logged_ml: 0,
            goal_ml: Some(2000),
        },
    );
    assert!(h.view().water.unwrap().is_behind());
    let (id, _) = h.log(2000);
    assert!(!h.view().water.unwrap().is_behind());
    h.unlog(id);
    h.w(WaterInput::Undone);
    assert!(h.view().water.unwrap().is_behind());
}

#[test]
fn a_phone_note_takes_a_label_and_undo() {
    let mut h = H::new();
    let first = Uuid::from_u128(1);
    let second = Uuid::from_u128(2);
    assert!(matches!(
        h.input(Input::Phone(PhoneInput::Labelled {
            entry: second,
            label: PhoneLabel::Work
        }))
        .outcome,
        InputOutcome::Ignored(_)
    ));
    h.input(Input::Phone(PhoneInput::Noted { entry: first }));
    assert_eq!(
        h.note(),
        Some(Bubble::PhoneNote {
            entry: Some(first),
            at: hm(10, 0),
            label: None
        })
    );
    h.input(Input::Phone(PhoneInput::Labelled {
        entry: second,
        label: PhoneLabel::Work,
    }));
    assert_eq!(
        h.note(),
        Some(Bubble::PhoneNote {
            entry: Some(second),
            at: hm(10, 0),
            label: Some(PhoneLabel::Work)
        })
    );
    h.input(Input::Phone(PhoneInput::Undone));
    assert!(matches!(
        h.note(),
        Some(Bubble::PhoneNote { entry: None, .. })
    ));
    // Nothing to label or undo once removed.
    assert!(matches!(
        h.input(Input::Phone(PhoneInput::Undone)).outcome,
        InputOutcome::Ignored(_)
    ));
    assert!(matches!(
        h.input(Input::Phone(PhoneInput::Labelled {
            entry: first,
            label: PhoneLabel::Unsure
        }))
        .outcome,
        InputOutcome::Ignored(_)
    ));
    // It closes after a minute; the calm gap follows.
    h.wait(secs(65));
    assert_eq!(h.note(), None);
}

#[test]
fn a_note_replaces_a_reminder_and_holds_back_the_next_until_the_calm_gap() {
    let mut h = H::only(false, true, false);
    h.wait_until(mins(25), |v| v.reminder().is_some());
    let offer = h.offer();
    h.input(Input::Phone(PhoneInput::Noted {
        entry: Uuid::from_u128(1),
    }));
    assert!(h.note().is_some());
    assert_eq!(h.status(Activity::EyeRest), ChipStatus::Waiting);
    assert!(h.responses().is_empty());
    // Closing the note: the reminder waits as its badge, nothing pops up within the gap.
    h.input(Input::CloseBubble);
    assert_eq!(h.view().bubble, None);
    h.wait(CALM_GAP - secs(10));
    assert_eq!(h.view().bubble, None);
    // The user can bring it back; the same offer.
    h.input(Input::OpenPending);
    assert_eq!(h.offer(), offer);
    // ...which replaces a note too.
    h.log(250);
    assert!(h.note().is_some());
    h.input(Input::OpenPending);
    assert_eq!(h.offer(), offer);
    assert_eq!(h.note(), None);
}

#[test]
fn while_a_note_is_open_no_reminder_opens_by_itself() {
    let mut h = H::only(false, true, false);
    h.wait(mins(19));
    h.w(WaterInput::OpenPanel);
    // Eye rest falls due at 20 minutes; the panel is open, so it waits.
    for _ in 0..30 {
        h.wait(secs(5));
        h.w(WaterInput::StepOther { up: true });
        h.w(WaterInput::StepOther { up: false });
    }
    assert!(h.offers().is_empty());
    assert!(h.note().is_some());
    // The panel closes; a reminder only after the calm gap.
    h.input(Input::CloseBubble);
    let waited = h.wait_until(mins(10), |v| v.reminder().is_some());
    assert!(waited >= CALM_GAP - secs(5), "{waited:?}");
}

#[test]
fn a_new_hold_or_a_hidden_pet_closes_a_note_and_it_never_returns() {
    let mut h = water_goal();
    h.log(250);
    h.input(Input::SetMeeting(true));
    assert_eq!(h.note(), None);
    h.input(Input::SetMeeting(false));
    h.wait(mins(1));
    assert_eq!(h.note(), None);
    // Opened during a meeting, it stays while the meeting lasts...
    h.input(Input::SetMeeting(true));
    h.w(WaterInput::OpenPanel);
    h.wait(secs(30));
    assert!(h.note().is_some());
    // ...and closes for focus, a further hold.
    h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: true,
    });
    assert_eq!(h.note(), None);
    h.input(Input::EndFocus);
    h.input(Input::SetMeeting(false));
    // A meeting that ended and starts again closes a note opened after it ended.
    h.input(Input::SetMeeting(true));
    h.input(Input::SetMeeting(false));
    h.w(WaterInput::OpenPanel);
    h.input(Input::SetMeeting(true));
    assert_eq!(h.note(), None);
    h.input(Input::SetMeeting(false));
    // A presentation hides the pet and closes it; nothing opens while hidden.
    h.w(WaterInput::OpenPanel);
    h.input(Input::PresentationChanged(true));
    assert_eq!(h.view().bubble, None);
    let (_, out) = h.log(250);
    assert_eq!(
        out.outcome,
        InputOutcome::Applied,
        "logging never needs the card"
    );
    assert_eq!(h.view().bubble, None);
    assert!(matches!(
        h.w(WaterInput::OpenPanel).outcome,
        InputOutcome::Ignored(_)
    ));
    h.input(Input::PresentationChanged(false));
    assert_eq!(h.view().bubble, None);
    // A lock closes it too.
    h.w(WaterInput::OpenPanel);
    h.input(Input::Lock);
    h.input(Input::Unlock);
    assert_eq!(h.note(), None);
}

#[test]
fn a_note_and_the_guide_or_a_pomodoro_card_take_turns() {
    let mut h = H::only(true, false, false);
    // The guide waits behind the pet while the user logs water, and comes back on request.
    h.input(Input::RoutineStarted);
    assert!(matches!(h.view().bubble, Some(Bubble::MovementGuide(_))));
    h.log(250);
    assert!(h.note().is_some());
    assert!(!h.view().guide.unwrap().open);
    h.input(Input::OpenPending);
    assert!(matches!(h.view().bubble, Some(Bubble::MovementGuide(_))));
    h.input(Input::Guide(GuideAction::Stop));
    h.input(Input::Guide(GuideAction::Close));
    // A Pomodoro break that falls due while a note is open waits for it.
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: true,
        resume_coaching: true,
    });
    h.wait(mins(24) + secs(50));
    h.input(Input::Phone(PhoneInput::Noted {
        entry: Uuid::from_u128(9),
    }));
    h.wait(secs(20));
    assert!(matches!(h.view().bubble, Some(Bubble::PhoneNote { .. })));
    h.input(Input::CloseBubble);
    h.wait(secs(5));
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
    // A note replaces the Pomodoro card, which stays in the menu.
    h.log(150);
    assert!(h.note().is_some());
    h.input(Input::OpenPending);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
}

#[test]
fn already_did_it_on_a_water_reminder_opens_the_panel() {
    let mut h = H::at(
        ist(20, 13, 0),
        cats(false, false, true),
        WaterSnapshot {
            logged_ml: 0,
            goal_ml: Some(2000),
        },
    );
    h.wait_until(mins(30), |v| v.reminder().is_some());
    h.respond(Response::Done);
    assert!(
        h.responses()
            .contains(&(Activity::Water, ReminderOutcome::Done))
    );
    assert!(matches!(h.note(), Some(Bubble::WaterPanel { .. })));
}

#[test]
fn a_restart_closes_a_note() {
    let mut h = water_goal();
    h.log(250);
    h.restart(secs(30));
    assert_eq!(h.note(), None);
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
        Guide(u8),
        DeadlineDay,
        Recovery(u8),
        Restart(u16),
        Log(u8),
        Note(u8),
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
            6 => (0u8..10).prop_map(Act::Guide),
            1 => Just(Act::DeadlineDay),
            3 => (0u8..9).prop_map(Act::Recovery),
            1 => (0u16..30).prop_map(Act::Restart),
            2 => (0u8..3).prop_map(Act::Log),
            4 => (0u8..10).prop_map(Act::Note),
        ]
    }

    /// The holds above the Pomodoro the checker can see from the view and the wall clock.
    /// `pause` is `None` while hidden behind being away.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    struct SeenHolds {
        away: bool,
        pause: Option<bool>,
        off_hours: bool,
        focus: bool,
        meeting: bool,
    }

    impl SeenHolds {
        fn of(v: &CoachView, off_hours: bool) -> Self {
            let away = matches!(
                v.session,
                SessionState::ShortLock | SessionState::ConfirmedBreak
            );
            Self {
                away,
                pause: (!away).then_some(v.session == SessionState::Paused),
                off_hours,
                focus: v.focus_left.is_some(),
                meeting: v.meeting,
            }
        }

        /// A hold is in force now that was seen not to be before.
        fn gained(self, now: Self) -> bool {
            (now.away && !self.away)
                || (now.pause == Some(true) && self.pause == Some(false))
                || (now.off_hours && !self.off_hours)
                || (now.focus && !self.focus)
                || (now.meeting && !self.meeting)
        }
    }

    struct Checker {
        offered: HashMap<Uuid, Activity>,
        answered: HashSet<Uuid>,
        last_resolved: Option<Duration>,
        last_hold_end: Option<Duration>,
        last_wake: Option<Duration>,
        prev_bubble: Option<Bubble>,
        prev_away: bool,
        /// A Pomodoro card the user opened during the current stretch of holds above the
        /// Pomodoro, keyed by phase and round.
        user_card: Option<(PomodoroPhase, u32)>,
        prev_holds: SeenHolds,
        /// Guided routines with a recorded result.
        sessions: HashSet<Uuid>,
        /// Answers recorded for each guided routine, in order.
        steps: HashMap<Uuid, Vec<StepOutcome>>,
        /// The step being checked is time passing with the user at the desk.
        ticking: bool,
    }

    impl Checker {
        fn check(
            &mut self,
            now: Duration,
            out: &StepOutput,
            off_hours: bool,
            user_open: bool,
            new_lock: bool,
            report: bool,
        ) -> Result<(), TestCaseError> {
            let v = &out.view;
            // A hold that starts withdraws every Pomodoro card, even one the user opened
            // during another hold; it does not come back until the user opens it again.
            let holds = SeenHolds::of(v, off_hours);
            let gained = new_lock || self.prev_holds.gained(holds);
            self.prev_holds = holds;
            let card = super::is_pomodoro_card(v.bubble);
            if gained && !user_open {
                prop_assert!(
                    !card,
                    "Pomodoro card {:?} kept when a new hold started ({:?})",
                    v.bubble,
                    holds
                );
                self.user_card = None;
                // The movement guide goes behind the pet the same way, and a water or
                // phone note closes.
                prop_assert!(
                    !matches!(v.bubble, Some(Bubble::MovementGuide(_))),
                    "guide card kept when a new hold started ({:?})",
                    holds
                );
                prop_assert!(
                    !super::is_note(v.bubble),
                    "note {:?} kept when a new hold started ({:?})",
                    v.bubble,
                    holds
                );
            }
            // No Pomodoro card under a hold above the Pomodoro unless the user opened it
            // during this stretch of holds (entering a hold withdraws any open card).
            let held = matches!(
                v.hold,
                Some(Hold::PauseLockOffHours | Hold::FocusPresentationMeeting)
            );
            let key = v.pomodoro.map(|p| (p.phase, p.round));
            if !held {
                self.user_card = None;
            } else if user_open && card {
                self.user_card = key;
            } else if self.user_card != key {
                self.user_card = None;
            }
            if held && card {
                prop_assert!(
                    self.user_card.is_some(),
                    "unsolicited Pomodoro card {:?} under {:?}",
                    v.bubble,
                    v.hold
                );
            }
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
                    // A routine's answers come in order, each once, and never after its
                    // result; its result is recorded once and matches its answers.
                    EventPayload::MovementStep {
                        session_id,
                        step_index,
                        outcome,
                        ..
                    } => {
                        prop_assert!(!self.sessions.contains(&session_id), "step after result");
                        let s = self.steps.entry(session_id).or_default();
                        prop_assert_eq!(step_index as usize, s.len(), "steps out of order");
                        s.push(outcome);
                    }
                    EventPayload::MovementSession {
                        session_id,
                        outcome,
                        steps_done,
                        steps_skipped,
                        steps_total,
                        ..
                    } => {
                        prop_assert!(self.sessions.insert(session_id), "two results");
                        let s = self.steps.get(&session_id).cloned().unwrap_or_default();
                        let recorded = s.len() as u32;
                        let done_steps =
                            s.iter().filter(|o| **o == StepOutcome::Done).count() as u32;
                        let (done, skipped) = if report {
                            // Reported in one go from the menu: the counts follow the
                            // report for the movements not answered, never more movements
                            // than the routine has, and they include every answered one.
                            prop_assert!(recorded <= steps_total);
                            prop_assert!(steps_done + steps_skipped <= steps_total);
                            prop_assert!(
                                steps_done >= done_steps && steps_skipped >= recorded - done_steps,
                                "{:?} with {} done, {} skipped after answers {:?}",
                                outcome,
                                steps_done,
                                steps_skipped,
                                s
                            );
                            (steps_done, steps_skipped)
                        } else {
                            prop_assert_eq!(
                                (steps_done, steps_skipped),
                                (done_steps, recorded - done_steps)
                            );
                            (done_steps, recorded - done_steps)
                        };
                        let ok = match outcome {
                            RoutineOutcome::Done => done == steps_total,
                            RoutineOutcome::Partial => done > 0 && done < steps_total,
                            RoutineOutcome::None => done == 0,
                            RoutineOutcome::Skipped => s.is_empty() && done == 0 && skipped == 0,
                        };
                        prop_assert!(ok, "{:?} with {} done, {} skipped", outcome, done, skipped);
                    }
                    _ => {}
                }
            }
            // The guide is the one bubble while it runs: nothing else is offered and it
            // holds reminders back like an exercise.
            if let Some(g) = v.guide {
                prop_assert!(v.reminder().is_none(), "reminder over the guide");
                if !matches!(g.stage, GuideStage::Result { .. }) {
                    prop_assert!(v.hold.is_some(), "running guide without a hold");
                }
                if g.open && v.visible {
                    prop_assert!(
                        matches!(v.bubble, Some(Bubble::MovementGuide(_))),
                        "guide open but bubble {:?}",
                        v.bubble
                    );
                }
            }
            // Nothing opens by itself over a note while time passes: only the user
            // replaces it. A Pomodoro card waits for the note, and may open in the step
            // where the note's own time ends (the deterministic traces check the wait).
            if super::is_note(self.prev_bubble) && self.ticking {
                prop_assert!(
                    v.bubble.is_none()
                        || super::is_note(v.bubble)
                        || super::is_pomodoro_card(v.bubble),
                    "{:?} replaced the note {:?} by itself",
                    v.bubble,
                    self.prev_bubble
                );
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

    fn off_hours(h: &H) -> bool {
        !h.schedule.is_work_time(h.wall, &h.zone)
    }

    /// Apply one action; returns every step's output with the engine time after it and
    /// whether the wall time was then outside work hours.
    fn run(h: &mut H, a: &Act) -> Vec<(Duration, StepOutput, bool)> {
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
                    outs.push((h.e.engine_time(), out, off_hours(h)));
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
            Act::Guide(k) => h.input(Input::Guide(match k {
                0 => GuideAction::Variant(RoutineVariant::Seated),
                1 => GuideAction::Variant(RoutineVariant::Standing),
                2 | 3 => GuideAction::Done,
                4 => GuideAction::Skip,
                5 => GuideAction::Stop,
                6 => GuideAction::Later,
                7 => GuideAction::SkipToday,
                8 => GuideAction::OtherDay,
                _ => GuideAction::Day(Weekday::Thu),
            })),
            Act::DeadlineDay => h.input(Input::DeadlineDay),
            Act::Recovery(k) => {
                let tomorrow = h.today().succ_opt().unwrap();
                h.input(Input::Recovery(match k {
                    0 => RecoveryAction::PlanFor(tomorrow),
                    1 => RecoveryAction::ChooseDate,
                    2 => RecoveryAction::Later,
                    3 => RecoveryAction::Ok,
                    4 => RecoveryAction::CancelPlan,
                    5 => RecoveryAction::WalkNow,
                    6 => RecoveryAction::Reflect(Reflection::Yes),
                    7 => RecoveryAction::ShowPlan,
                    _ => RecoveryAction::NoThanks,
                }))
            }
            Act::Log(k) => h.log([150, 250, 500][usize::from(k)]).1,
            Act::Note(k) => {
                let last = h.entries.last().map(|e| e.id);
                match k {
                    0 => {
                        if let Some(id) = last {
                            h.unlog(id);
                        }
                        h.w(WaterInput::Undone)
                    }
                    1 => h.w(WaterInput::OpenPanel),
                    2 => h.w(WaterInput::Edit(last.unwrap_or_else(Uuid::nil))),
                    3 => h.w(WaterInput::StepEdit { up: true }),
                    4 => h.w(WaterInput::StepOther { up: false }),
                    5 => h.w(WaterInput::CancelEdit),
                    6 => {
                        if let Some(id) = last {
                            h.unlog(id);
                        }
                        h.w(WaterInput::Changed)
                    }
                    7 => h.input(Input::Phone(PhoneInput::Noted {
                        entry: Uuid::from_u128(0xf00 + u128::from(k)),
                    })),
                    8 => h.input(Input::Phone(PhoneInput::Labelled {
                        entry: Uuid::from_u128(0xf01),
                        label: PhoneLabel::Work,
                    })),
                    _ => h.input(Input::Phone(PhoneInput::Undone)),
                }
            }
            Act::Restart(m) => {
                // The restart closes what was open at the moment the app stopped, before
                // the time it was closed.
                let out = h.restart(mins(u64::from(m)));
                let at = h.e.engine_time().saturating_sub(mins(u64::from(m)));
                return vec![(at, out, off_hours(h))];
            }
        };
        vec![(h.e.engine_time(), one, off_hours(h))]
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Never two visible, never a burst, calm gap, grace after wake, no replay after a
        /// hold, one response per offer, an answer changes only its own activity, no
        /// Pomodoro card the user did not open under a hold above the Pomodoro, and none
        /// kept when a new hold starts.
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
                user_card: None,
                prev_holds: SeenHolds::default(),
                sessions: HashSet::new(),
                steps: HashMap::new(),
                ticking: false,
            };
            c.prev_holds = SeenHolds::of(h.view(), off_hours(&h));
            for e in h.events.clone() {
                if let EventPayload::ReminderOffered { reminder_id, activity } = e.payload {
                    c.offered.insert(reminder_id, activity);
                }
            }
            c.prev_bubble = h.view().bubble;
            for a in &acts {
                let shown = h.view().reminder().map(|(_, a)| a);
                let before: Vec<_> = Activity::ALL.iter().map(|x| *h.e.activity(*x)).collect();
                let user_open = matches!(a, Act::OpenPending | Act::PomoSkip);
                c.ticking = matches!(a, Act::Wait(..));
                for (now, out, off) in run(&mut h, a) {
                    let new_lock = matches!(a, Act::Lock | Act::Suspend)
                        && out.outcome == InputOutcome::Applied;
                    let report = matches!(a, Act::Routine(_));
                    c.check(now, &out, off, user_open, new_lock, report)?;
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

// ------------------------------------------------------- weekly review offer (V1-4)

fn review_offer(v: &CoachView) -> Option<(NaiveDate, bool)> {
    match v.bubble {
        Some(Bubble::ReviewOffer { week, later }) => Some((week, later)),
        _ => None,
    }
}

/// Only the weekly review offer can open (no reminders), Saturday 24 Oct at `h:m`.
fn review_harness(h: u32, m: u32) -> H {
    H::at(
        ist(24, h, m),
        cats(false, false, false),
        WaterSnapshot::default(),
    )
}

#[test]
fn review_offer_opens_once_at_the_review_time() {
    let mut h = review_harness(16, 50);
    h.wait(mins(5));
    assert_eq!(review_offer(h.view()), None, "not before 17:00");
    h.wait_until(mins(6), |v| review_offer(v).is_some());
    assert_eq!(h.local(), hm(17, 0));
    let monday = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
    assert_eq!(review_offer(h.view()), Some((monday, true)));
    assert_eq!(
        h.input(Input::Review(ReviewAnswer::Open)).outcome,
        InputOutcome::Applied
    );
    assert_eq!(h.view().bubble, None);
    h.wait(mins(30));
    assert_eq!(review_offer(h.view()), None, "never twice in a week");
    assert_eq!(
        h.input(Input::Review(ReviewAnswer::Skip)).outcome,
        InputOutcome::Ignored("no review offer open")
    );
}

#[test]
fn review_offer_needs_recorded_activity_this_week() {
    // Started for the first time after the review time: nothing recorded yet this week,
    // so nothing is offered until this PC has recorded some time.
    let mut h = review_harness(17, 30);
    assert_eq!(review_offer(h.view()), None);
    h.wait_until(mins(1), |v| review_offer(v).is_some());
}

#[test]
fn review_offer_later_comes_back_once_on_the_next_workday() {
    let mut h = review_harness(16, 55);
    h.wait_until(mins(6), |v| review_offer(v).is_some());
    h.input(Input::Review(ReviewAnswer::Later));
    assert_eq!(h.view().bubble, None);
    // Sunday is not a workday; Monday 26 Oct is.
    h.wall = ist(25, 12, 0);
    h.wait(mins(10));
    assert_eq!(review_offer(h.view()), None);
    h.wall = ist(26, 9, 40);
    h.wait_until(mins(6), |v| review_offer(v).is_some());
    assert_eq!(h.today(), NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
    let monday = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
    assert_eq!(
        review_offer(h.view()),
        Some((monday, false)),
        "no second Later"
    );
    assert_eq!(
        h.input(Input::Review(ReviewAnswer::Later)).outcome,
        InputOutcome::Ignored("Later was already used for this week")
    );
    h.input(Input::Review(ReviewAnswer::Skip));
    h.wait(mins(30));
    assert_eq!(review_offer(h.view()), None);
}

#[test]
fn review_offer_waits_for_holds_and_closes_quietly() {
    let mut h = review_harness(16, 55);
    h.input(Input::SetMeeting(true));
    h.wait(mins(10));
    assert_eq!(review_offer(h.view()), None, "held during a meeting");
    h.input(Input::SetMeeting(false));
    h.wait_until(mins(6), |v| review_offer(v).is_some());
    // Unanswered, it closes after 10 minutes and is not shown again this week.
    h.wait_until(mins(11), |v| review_offer(v).is_none());
    h.wait(mins(20));
    assert_eq!(review_offer(h.view()), None);
}

#[test]
fn review_offer_follows_the_chosen_time() {
    let config = CoachConfig {
        categories: Categories {
            movement: false,
            eye_rest: false,
            water: false,
        },
        review: ReviewTime {
            day: Weekday::Fri,
            at: hm(11, 0),
        },
        ..CoachConfig::default()
    };
    let mut h = H::at(ist(23, 10, 55), config, WaterSnapshot::default());
    h.wait_until(mins(6), |v| review_offer(v).is_some());
    assert_eq!(h.local(), hm(11, 0));
}

#[test]
fn default_movement_version_reaches_the_guide() {
    let config = CoachConfig {
        movement_version: MovementVersion::Standing,
        ..CoachConfig::default()
    };
    let mut h = H::at(ist(20, 10, 0), config, WaterSnapshot::default());
    h.input(Input::RoutineStarted);
    assert_eq!(h.guide().default_variant, Some(RoutineVariant::Standing));
    let mut ask = H::new();
    ask.input(Input::RoutineStarted);
    assert_eq!(ask.guide().default_variant, None);
}

#[test]
fn the_view_reports_each_hold_for_the_camera() {
    use hy_core::camera::{CameraHold, HoldSources};
    let mut h = H::new();
    assert_eq!(h.view().holds, HoldSources::default());
    assert_eq!(h.view().holds.camera_hold(), None);
    // A presentation is reported even when the pet is kept on screen.
    let mut shown = H::at(
        ist(20, 10, 0),
        CoachConfig {
            hide_during_presentation: false,
            ..CoachConfig::default()
        },
        WaterSnapshot::default(),
    );
    shown.input(Input::PresentationChanged(true));
    assert!(shown.view().visible);
    assert!(shown.view().holds.presenting);
    assert_eq!(
        shown.view().holds.camera_hold(),
        Some(CameraHold::Presentation)
    );
    h.input(Input::SetMeeting(true));
    assert_eq!(h.view().holds.camera_hold(), Some(CameraHold::Meeting));
    h.input(Input::SetMeeting(false));
    h.input(Input::StartFocus {
        minutes: 30,
        replace_pomodoro: false,
    });
    assert!(h.view().holds.focus);
    assert_eq!(h.view().holds.camera_hold(), None, "focus keeps capture");
    h.input(Input::EndFocus);
    h.input(Input::Pause(PauseLength::Minutes(30)));
    assert_eq!(h.view().holds.camera_hold(), Some(CameraHold::Paused));
    h.input(Input::ResumeCoaching);
    h.input(Input::Lock);
    assert_eq!(h.view().holds.camera_hold(), Some(CameraHold::Away));
    h.input(Input::Unlock);
    assert_eq!(h.view().holds.camera_hold(), None);
    // Off-hours: 19:00 on a workday.
    let mut late = H::at(
        ist(20, 19, 0),
        CoachConfig::default(),
        WaterSnapshot::default(),
    );
    late.input(Input::Refresh);
    assert_eq!(late.view().holds.camera_hold(), Some(CameraHold::OffHours));
    // Views saved before V2-1 still load.
    let mut v = serde_json::to_value(h.view()).unwrap();
    v.as_object_mut().unwrap().remove("holds");
    let back: CoachView = serde_json::from_value(v).unwrap();
    assert_eq!(back.holds, HoldSources::default());
}
