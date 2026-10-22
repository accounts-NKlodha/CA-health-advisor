//! The coaching engine bound to the real store: restart persistence (close and reopen a
//! file database), clock rollback and forward jumps across a restart, stale writers,
//! durable one-response-per-offer, per-day tallies, the guided movement routine and the
//! recovery plan across restarts, and the 0002 and 0003 schema guards.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use hy_core::Error;
use hy_core::clock::{Clock, ManualClock};
use hy_core::coach::config::{CONFIG_KEY, Categories};
use hy_core::coach::*;
use hy_core::schema::{
    Activity, EventPayload, GoalKind, MeasurementStatus, NewEvent, ReminderOutcome, RoutineOutcome,
    RoutineVariant, StepOutcome,
};
use hy_core::store::{CoachCommit, Store};
use hy_core::time::{UserZone, WeekdaySet, WorkSchedule};
use uuid::Uuid;

fn mins(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

/// IST wall time on a date in Oct 2026 (20 Oct is a Tuesday).
fn ist(day: u32, h: u32, m: u32) -> DateTime<Utc> {
    UserZone::KOLKATA.resolve_local(
        NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap(),
    )
}

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
}

struct Env {
    _dir: tempfile::TempDir,
    path: PathBuf,
    clock: Arc<ManualClock>,
}

impl Env {
    fn new(start: DateTime<Utc>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hy.db");
        Self {
            _dir: dir,
            path,
            clock: Arc::new(ManualClock::new(start)),
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.path, self.clock.clone()).unwrap()
    }

    fn open(&self) -> Coach {
        Coach::open(self.store()).unwrap().0
    }

    fn open_with(&self, config: CoachConfig) -> Coach {
        let store = self.store();
        store.put_setting(CONFIG_KEY, &config).unwrap();
        Coach::open(store).unwrap().0
    }

    fn local(&self) -> NaiveTime {
        UserZone::KOLKATA.local_datetime(self.clock.now()).time()
    }
}

fn only(movement: bool, eye_rest: bool, water: bool) -> CoachConfig {
    CoachConfig {
        categories: Categories {
            movement,
            eye_rest,
            water,
        },
        ..CoachConfig::default()
    }
}

/// Tick every 5 s for `d` with the user typing.
fn work(env: &Env, c: &mut Coach, d: Duration) {
    let mut left = d;
    while left > Duration::ZERO {
        let t = left.min(Duration::from_secs(5));
        env.clock.advance(chrono::Duration::from_std(t).unwrap());
        c.tick(Duration::ZERO).unwrap();
        left -= t;
    }
}

fn work_until(env: &Env, c: &mut Coach, max: Duration, pred: impl Fn(&CoachView) -> bool) {
    let mut waited = Duration::ZERO;
    while !pred(c.view()) {
        assert!(waited < max, "condition not met within {max:?}");
        work(env, c, Duration::from_secs(5));
        waited += Duration::from_secs(5);
    }
}

fn reminder_events(store: &Store, day: NaiveDate) -> Vec<EventPayload> {
    store
        .events_on(day)
        .unwrap()
        .into_iter()
        .map(|e| e.payload)
        .filter(|p| {
            matches!(
                p,
                EventPayload::ReminderOffered { .. } | EventPayload::ReminderResponse { .. }
            )
        })
        .collect()
}

fn outcomes(store: &Store, day: NaiveDate) -> Vec<(Activity, ReminderOutcome)> {
    reminder_events(store, day)
        .into_iter()
        .filter_map(|p| match p {
            EventPayload::ReminderResponse {
                activity, outcome, ..
            } => Some((activity, outcome)),
            _ => None,
        })
        .collect()
}

fn path_of(env: &Env) -> &Path {
    &env.path
}

// ------------------------------------------------------------------ restarts

#[test]
fn snooze_survives_close_and_reopen() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(false, true, false));
    work_until(&env, &mut c, mins(25), |v| v.reminder().is_some());
    let offer = c.view().reminder().unwrap().0;
    c.handle(Input::Respond {
        offer,
        response: Response::Snooze,
    })
    .unwrap();
    let rev = c.revision().unwrap();
    let store = c.close().unwrap();
    assert!(store.coach_state().unwrap().unwrap().revision > rev - 1);
    drop(store);
    // The app restarts 30 seconds later (a short gap: no reset).
    env.clock.advance(chrono::Duration::seconds(30));
    let (mut c, first) = Coach::open(env.store()).unwrap();
    assert_eq!(
        first.view.chip(Activity::EyeRest).status,
        ChipStatus::Snoozed
    );
    assert!(c.revision().unwrap() > rev);
    // The snooze still returns at 10:30 (after the 2-minute grace has passed).
    work_until(&env, &mut c, mins(15), |v| v.reminder().is_some());
    assert_eq!(env.local(), NaiveTime::from_hms_opt(10, 30, 0).unwrap());
    let evs = reminder_events(c.store(), date(20));
    assert_eq!(evs.len(), 3);
    assert_eq!(
        outcomes(c.store(), date(20)),
        vec![(Activity::EyeRest, ReminderOutcome::Snoozed)]
    );
}

#[test]
fn a_reminder_open_at_shutdown_is_closed_as_no_response_then_grace() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, true, false));
    work_until(&env, &mut c, mins(25), |v| v.reminder().is_some());
    drop(c); // crash: no flush; the offer was saved when it was shown
    env.clock.advance(chrono::Duration::seconds(20));
    let (mut c, first) = Coach::open(env.store()).unwrap();
    assert_eq!(first.view.bubble, None);
    assert_eq!(
        outcomes(c.store(), date(20)),
        vec![(Activity::EyeRest, ReminderOutcome::NoResponse)]
    );
    assert_eq!(
        first.view.chip(Activity::EyeRest).status,
        ChipStatus::NoResponse
    );
    work(&env, &mut c, mins(2));
    assert!(c.view().reminder().is_none());
}

#[test]
fn a_long_gap_while_closed_is_a_confirmed_break() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work(&env, &mut c, mins(30));
    assert_eq!(
        c.engine().activity(Activity::Movement).accum_ms,
        30 * 60_000
    );
    c.close().unwrap();
    env.clock.advance(chrono::Duration::minutes(10));
    let c = env.open();
    assert_eq!(c.engine().activity(Activity::Movement).accum_ms, 0);
    let day = c.store().activity_day(date(20)).unwrap().unwrap();
    assert_eq!(day.confirmed_breaks, 1);
    assert_eq!(day.away_s, 600);
    assert_eq!(day.work_active_s, 1800);
}

#[test]
fn a_short_restart_keeps_the_movement_timer() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work(&env, &mut c, mins(30));
    c.close().unwrap();
    env.clock.advance(chrono::Duration::seconds(40));
    let mut c = env.open();
    assert_eq!(
        c.engine().activity(Activity::Movement).accum_ms,
        30 * 60_000
    );
    work(&env, &mut c, mins(15));
    assert_eq!(
        c.engine().activity(Activity::Movement).accum_ms,
        45 * 60_000
    );
}

#[test]
fn clock_set_back_while_closed_is_no_break_and_no_burst() {
    let env = Env::new(ist(20, 11, 0));
    let mut c = env.open_with(only(true, true, false));
    work(&env, &mut c, mins(19));
    let engine_time = c.engine().engine_time();
    c.close().unwrap();
    // Wall clock set back by two hours before the next start.
    env.clock.shift_wall(chrono::Duration::hours(-2));
    let (mut c, first) = Coach::open(env.store()).unwrap();
    assert_eq!(first.view.bubble, None);
    assert_eq!(c.engine().engine_time(), engine_time);
    assert_eq!(
        c.engine().activity(Activity::Movement).accum_ms,
        19 * 60_000
    );
    // 09:19 is before work hours: nothing is shown and nothing counts.
    work(&env, &mut c, mins(10));
    assert!(c.view().reminder().is_none());
    assert_eq!(c.view().pet, PetMode::OffHours);
    assert_eq!(c.engine().activity(Activity::EyeRest).accum_ms, 19 * 60_000);
    // Work hours start at 09:30 (the same work day, so the timers carry on): eye rest is
    // due at 09:31 but waits for the calm gap after the quiet period, then shows once.
    work_until(&env, &mut c, mins(30), |v| v.reminder().is_some());
    assert_eq!(env.local(), NaiveTime::from_hms_opt(9, 35, 0).unwrap());
    work(&env, &mut c, mins(10));
    let offered = reminder_events(c.store(), date(20))
        .into_iter()
        .filter(|p| matches!(p, EventPayload::ReminderOffered { .. }))
        .count();
    assert_eq!(offered, 1);
    // The saved engine time never went backwards.
    let saved = c.store().coach_state().unwrap().unwrap();
    assert!(saved.engine_time_ms >= u64::try_from(engine_time.as_millis()).unwrap());
}

#[test]
fn clock_set_forward_while_closed_counts_as_time_away() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work(&env, &mut c, mins(20));
    c.close().unwrap();
    env.clock.shift_wall(chrono::Duration::hours(3));
    let c = env.open();
    // Indistinguishable from three hours off: a confirmed break, no burst.
    assert_eq!(c.engine().activity(Activity::Movement).accum_ms, 0);
    assert_eq!(c.view().bubble, None);
}

#[test]
fn a_running_routine_at_shutdown_is_abandoned_without_a_record() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work_until(&env, &mut c, mins(50), |v| v.reminder().is_some());
    let offer = c.view().reminder().unwrap().0;
    c.handle(Input::Respond {
        offer,
        response: Response::Start,
    })
    .unwrap();
    c.close().unwrap();
    env.clock.advance(chrono::Duration::seconds(30));
    let c = env.open();
    assert_eq!(
        c.view().chip(Activity::Movement).status,
        ChipStatus::NoneRecorded
    );
    // The late result from the old guide is stale.
    let mut c = c;
    let out = c
        .handle(Input::RoutineFinished {
            offer: Some(offer),
            outcome: RoutineOutcome::Done,
        })
        .unwrap();
    assert_eq!(out.outcome, InputOutcome::Ignored("stale reminder"));
    assert_eq!(
        outcomes(c.store(), date(20)),
        vec![(Activity::Movement, ReminderOutcome::Started)]
    );
}

fn movement_events(store: &Store, day: NaiveDate) -> Vec<EventPayload> {
    store
        .events_on(day)
        .unwrap()
        .into_iter()
        .map(|e| e.payload)
        .filter(|p| {
            matches!(
                p,
                EventPayload::MovementStep { .. } | EventPayload::MovementSession { .. }
            )
        })
        .collect()
}

#[test]
fn a_guided_routine_is_stored_step_by_step_with_its_result() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work_until(&env, &mut c, mins(50), |v| v.reminder().is_some());
    let offer = c.view().reminder().unwrap().0;
    c.handle(Input::Respond {
        offer,
        response: Response::Start,
    })
    .unwrap();
    c.handle(Input::Guide(GuideAction::Variant(RoutineVariant::Seated)))
        .unwrap();
    for _ in 0..3 {
        c.handle(Input::Guide(GuideAction::Done)).unwrap();
    }
    let evs = movement_events(c.store(), date(20));
    assert_eq!(evs.len(), 4);
    let EventPayload::MovementSession {
        session_id,
        outcome,
        steps_done,
        variant,
        ..
    } = evs[3].clone()
    else {
        panic!("{evs:?}")
    };
    assert_eq!(
        (outcome, steps_done, variant),
        (RoutineOutcome::Done, 3, Some(RoutineVariant::Seated))
    );
    for (i, e) in evs[..3].iter().enumerate() {
        assert!(matches!(
            e,
            EventPayload::MovementStep { session_id: s, step_index, outcome: StepOutcome::Done, .. }
                if *s == session_id && *step_index == i as u32
        ));
    }
    assert_eq!(
        outcomes(c.store(), date(20)),
        vec![(Activity::Movement, ReminderOutcome::Started)]
    );
}

#[test]
fn a_guided_routine_running_at_shutdown_keeps_its_answered_steps() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work_until(&env, &mut c, mins(50), |v| v.reminder().is_some());
    let offer = c.view().reminder().unwrap().0;
    c.handle(Input::Respond {
        offer,
        response: Response::Start,
    })
    .unwrap();
    c.handle(Input::Guide(GuideAction::Variant(RoutineVariant::Standing)))
        .unwrap();
    c.handle(Input::Guide(GuideAction::Done)).unwrap();
    c.close().unwrap();
    env.clock.advance(chrono::Duration::seconds(30));
    let mut c = env.open();
    // The restart ends the routine as a stop: one movement done is partial.
    assert_eq!(c.view().guide, None);
    assert_eq!(
        c.view().chip(Activity::Movement).status,
        ChipStatus::Partial
    );
    let evs = movement_events(c.store(), date(20));
    assert_eq!(evs.len(), 2);
    assert!(matches!(
        evs[1],
        EventPayload::MovementSession {
            outcome: RoutineOutcome::Partial,
            steps_done: 1,
            steps_skipped: 0,
            ..
        }
    ));
    // Late answers to the old guide change nothing.
    let out = c.handle(Input::Guide(GuideAction::Done)).unwrap();
    assert_eq!(out.outcome, InputOutcome::Ignored("no movement guide"));
    assert_eq!(movement_events(c.store(), date(20)).len(), 2);
}

#[test]
fn a_guide_waiting_behind_the_pet_and_a_recovery_plan_survive_a_save() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    c.handle(Input::DeadlineDay).unwrap();
    c.handle(Input::CheckIn(CheckInAnswer::Yes)).unwrap();
    c.handle(Input::Recovery(RecoveryAction::PlanFor(date(21))))
        .unwrap();
    c.handle(Input::Recovery(RecoveryAction::Ok)).unwrap();
    c.handle(Input::RoutineStarted).unwrap();
    c.handle(Input::Guide(GuideAction::Variant(RoutineVariant::Seated)))
        .unwrap();
    c.handle(Input::CloseBubble).unwrap();
    let saved = c.view().guide.unwrap();
    assert!(!saved.open);
    // Reopen the store as another reader: the saved state carries both.
    let store = c.close().unwrap();
    let state = store.coach_state().unwrap().unwrap();
    let engine = Engine::from_state(serde_json::from_str(&state.state).unwrap()).unwrap();
    assert_eq!(engine.recovery_plan().unwrap().date, date(21));
    let kinds: Vec<_> = store
        .events_on(date(20))
        .unwrap()
        .into_iter()
        .map(|e| e.payload.type_name())
        .collect();
    assert_eq!(kinds, vec!["recovery.check_in", "recovery.plan"]);
}

#[test]
fn routine_steps_and_results_are_guarded_by_the_database() {
    let env = Env::new(ist(20, 10, 0));
    let mut s = env.store();
    let id = Uuid::from_u128(7);
    let step = |i| {
        NewEvent::manual(EventPayload::MovementStep {
            session_id: id,
            routine_id: "tue".to_owned(),
            routine_version: 1,
            variant: RoutineVariant::Seated,
            step_index: i,
            steps_total: 3,
            outcome: StepOutcome::Done,
        })
    };
    let result = || {
        NewEvent::manual(EventPayload::MovementSession {
            session_id: id,
            routine_id: "tue".to_owned(),
            routine_version: 1,
            variant: Some(RoutineVariant::Seated),
            outcome: RoutineOutcome::Partial,
            steps_done: 1,
            steps_skipped: 0,
            steps_total: 3,
        })
    };
    s.append_event(step(0)).unwrap();
    // Each step is answered once.
    assert!(s.append_event(step(0)).is_err());
    let r = s.append_event(result()).unwrap();
    // One result per routine, and no step after it.
    assert!(s.append_event(result()).is_err());
    assert!(s.append_event(step(1)).is_err());
    // Routine results are not corrected through the store (the raw-SQL guards in
    // schema_constraints show a correction would not count as a second result).
    assert!(s.append_event(result().superseding(r.id)).is_err());
    // Another routine is independent.
    let other = NewEvent::manual(EventPayload::MovementStep {
        session_id: Uuid::from_u128(8),
        routine_id: "tue".to_owned(),
        routine_version: 1,
        variant: RoutineVariant::Standing,
        step_index: 0,
        steps_total: 3,
        outcome: StepOutcome::Skipped,
    });
    s.append_event(other).unwrap();
}

#[test]
fn pomodoro_survives_a_restart_paused() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open();
    c.handle(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    })
    .unwrap();
    work(&env, &mut c, mins(10));
    c.close().unwrap();
    env.clock.advance(chrono::Duration::minutes(30));
    let c = env.open();
    let p = c.view().pomodoro.unwrap();
    assert_eq!(p.paused, Some(PomodoroPause::LockOrSleep));
    assert_eq!(p.left, mins(15));
    assert_eq!(c.view().bubble, Some(Bubble::PomodoroPausedByLock));
}

#[test]
fn a_card_opened_during_a_meeting_does_not_survive_a_restart() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open();
    c.handle(Input::StartPomodoro {
        preset: PomodoroPreset::Custom {
            work_min: 10,
            break_min: 2,
        },
        replace_focus: false,
        resume_coaching: false,
    })
    .unwrap();
    c.handle(Input::SetMeeting(true)).unwrap();
    work(&env, &mut c, mins(10));
    assert_eq!(c.view().bubble, None);
    c.handle(Input::OpenPending).unwrap();
    assert!(matches!(
        c.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
    c.close().unwrap();
    env.clock.advance(chrono::Duration::minutes(1));
    // The time closed counts as a sleep: a new hold that withdraws the card. The meeting
    // is still on, so it does not come back by itself.
    let mut c = env.open();
    assert!(c.view().meeting);
    assert_eq!(c.view().bubble, None);
    assert_eq!(
        c.view().pomodoro.unwrap().phase,
        PomodoroPhase::BreakOffered
    );
    work(&env, &mut c, mins(3));
    assert_eq!(c.view().bubble, None);
    c.handle(Input::OpenPending).unwrap();
    assert!(matches!(
        c.view().bubble,
        Some(Bubble::PomodoroBreak { .. })
    ));
}

// ------------------------------------------------------- durable ordering, stale

#[test]
fn a_stale_writer_is_refused_and_rolled_back() {
    let env = Env::new(ist(20, 10, 0));
    let mut a = env.open_with(only(false, true, false));
    work(&env, &mut a, mins(19));
    a.flush().unwrap();
    // A second instance opens the same database and moves the state on.
    let mut b = env.open();
    work(&env, &mut b, mins(2));
    b.flush().unwrap();
    let rev_b = b.revision().unwrap();
    // The first instance's next save is refused, and its memory rolls back.
    let before = a.engine().clone();
    let err = a.handle(Input::SetMeeting(true)).unwrap_err();
    assert!(matches!(err, Error::Stale(_)), "{err:?}");
    assert_eq!(a.engine(), &before);
    assert_eq!(a.store().coach_state().unwrap().unwrap().revision, rev_b);
    assert!(!b.view().meeting);
}

#[test]
fn commit_guards_revision_and_engine_time() {
    let env = Env::new(ist(20, 10, 0));
    let mut s = env.store();
    let state = "{}";
    let commit = |expected, t| CoachCommit {
        expected_revision: expected,
        format: 1,
        engine_time_ms: t,
        state,
        events: &[],
        days: &[],
    };
    assert_eq!(s.commit_coach_step(commit(None, 10)).unwrap(), 1);
    assert!(matches!(
        s.commit_coach_step(commit(None, 20)),
        Err(Error::Stale(_))
    ));
    assert!(matches!(
        s.commit_coach_step(commit(Some(5), 20)),
        Err(Error::Stale(_))
    ));
    assert_eq!(s.commit_coach_step(commit(Some(1), 20)).unwrap(), 2);
    // Engine time never goes backwards (schema trigger).
    assert!(s.commit_coach_step(commit(Some(2), 19)).is_err());
    assert_eq!(s.coach_state().unwrap().unwrap().revision, 2);
    // A bad event rolls the whole step back.
    let bad = [NewEvent::manual(EventPayload::WaterLogged { amount_ml: 0 })];
    let c = CoachCommit {
        events: &bad,
        ..commit(Some(2), 30)
    };
    assert!(s.commit_coach_step(c).is_err());
    assert_eq!(s.coach_state().unwrap().unwrap().engine_time_ms, 20);
    // The revision trigger refuses a direct jump.
    drop(s);
    let conn = rusqlite::Connection::open(path_of(&env)).unwrap();
    assert!(
        conn.execute("UPDATE coach_state SET revision = revision + 2", [])
            .is_err()
    );
}

#[test]
fn one_offer_and_at_most_one_response_per_reminder() {
    let env = Env::new(ist(20, 10, 0));
    let mut s = env.store();
    let id = Uuid::from_u128(42);
    let offered = |a| {
        NewEvent::inferred(
            EventPayload::ReminderOffered {
                reminder_id: id,
                activity: a,
            },
            MeasurementStatus::Measured,
        )
    };
    let response = |a, outcome| {
        NewEvent::manual(EventPayload::ReminderResponse {
            reminder_id: id,
            activity: a,
            outcome,
        })
    };
    // A response needs a stored offer of the same activity.
    assert!(
        s.append_event(response(Activity::Water, ReminderOutcome::Done))
            .is_err()
    );
    s.append_event(offered(Activity::Water)).unwrap();
    assert!(s.append_event(offered(Activity::Water)).is_err());
    assert!(
        s.append_event(response(Activity::Movement, ReminderOutcome::Done))
            .is_err()
    );
    s.append_event(response(Activity::Water, ReminderOutcome::Snoozed))
        .unwrap();
    // A second answer to the same reminder is refused by the database itself.
    assert!(
        s.append_event(response(Activity::Water, ReminderOutcome::Done))
            .is_err()
    );
}

// ---------------------------------------------------- tallies, water, settings

#[test]
fn tallies_are_saved_per_local_day_across_midnight() {
    let env = Env::new(ist(20, 23, 50));
    let store = env.store();
    store
        .set_work_schedule(&WorkSchedule {
            workdays: WeekdaySet::ALL,
            start: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
        })
        .unwrap();
    store
        .put_setting(CONFIG_KEY, &only(false, false, false))
        .unwrap();
    let (mut c, _) = Coach::open(store).unwrap();
    work(&env, &mut c, mins(5));
    // Ten minutes without input across midnight, in one poll.
    env.clock.advance(chrono::Duration::minutes(10));
    c.tick(mins(10)).unwrap();
    let store = c.close().unwrap();
    let d1 = store.activity_day(date(20)).unwrap().unwrap();
    let d2 = store.activity_day(date(21)).unwrap().unwrap();
    assert_eq!(d1.work_active_s, 6 * 60);
    assert_eq!(d1.work_quiet_s, 4 * 60);
    assert_eq!(d2.work_quiet_s, 5 * 60);
    assert_eq!(d2.work_active_s, 0);
    assert_eq!(store.activity_day(date(22)).unwrap(), None);
}

#[test]
fn routine_ticks_are_saved_at_most_once_a_minute() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(false, false, false));
    // The first tick opens the day's work window (saved at once).
    work(&env, &mut c, Duration::from_secs(5));
    let rev = c.revision().unwrap();
    work(&env, &mut c, Duration::from_secs(55));
    assert_eq!(c.revision().unwrap(), rev);
    work(&env, &mut c, Duration::from_secs(5));
    assert_eq!(c.revision().unwrap(), rev + 1);
    // A significant input is saved at once.
    c.handle(Input::SetMeeting(true)).unwrap();
    assert_eq!(c.revision().unwrap(), rev + 2);
    // Closing saves what is left.
    work(&env, &mut c, Duration::from_secs(20));
    let store = c.close().unwrap();
    assert_eq!(store.coach_state().unwrap().unwrap().revision, rev + 3);
}

#[test]
fn logging_water_from_a_reminder_is_stored_with_the_answer() {
    let env = Env::new(ist(20, 11, 15));
    let store = env.store();
    store
        .set_goal(GoalKind::WaterDailyMl, 2000, date(20))
        .unwrap();
    store
        .put_setting(CONFIG_KEY, &only(false, false, true))
        .unwrap();
    let (mut c, first) = Coach::open(store).unwrap();
    let (offer, activity) = first.view.reminder().unwrap();
    assert_eq!(activity, Activity::Water);
    let out = c
        .handle(Input::Respond {
            offer,
            response: Response::LogWater { ml: 250 },
        })
        .unwrap();
    assert_eq!(out.view.water.unwrap().logged_ml, 250);
    assert_eq!(c.store().water_total_on(date(20)).unwrap(), 250);
    assert_eq!(
        outcomes(c.store(), date(20)),
        vec![(Activity::Water, ReminderOutcome::Done)]
    );
    // Water logged later from the panel is picked up on the next step.
    c.store_mut()
        .append_event(NewEvent::manual(EventPayload::WaterLogged {
            amount_ml: 500,
        }))
        .unwrap();
    let out = c.handle(Input::Refresh).unwrap();
    assert_eq!(out.view.water.unwrap().logged_ml, 750);
}

#[test]
fn accepting_the_light_schedule_is_saved() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(false, true, false));
    for _ in 0..3 {
        work_until(&env, &mut c, mins(30), |v| v.reminder().is_some());
        let offer = c.view().reminder().unwrap().0;
        c.handle(Input::Respond {
            offer,
            response: Response::Dismiss,
        })
        .unwrap();
    }
    assert_eq!(c.view().bubble, Some(Bubble::LighterSchedule));
    c.handle(Input::LighterSchedule { accept: true }).unwrap();
    assert_eq!(c.config().unwrap().intensity, Intensity::Light);
    // set_config validates.
    let mut bad = c.config().unwrap();
    bad.long_day_check_in.hours = 12;
    assert!(c.set_config(&bad).is_err());
}

#[test]
fn resume_uses_wall_time_when_the_monotonic_clock_stood_still() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(true, false, false));
    work(&env, &mut c, mins(30));
    c.handle(Input::Suspend).unwrap();
    // Asleep for 20 minutes on a platform whose monotonic clock stops during sleep.
    env.clock.shift_wall(chrono::Duration::minutes(20));
    let out = c.handle(Input::Resume).unwrap();
    assert_eq!(c.engine().activity(Activity::Movement).accum_ms, 0);
    assert_eq!(out.view.bubble, None);
    assert_eq!(
        c.store()
            .activity_day(date(20))
            .unwrap()
            .unwrap()
            .confirmed_breaks,
        1
    );
}

#[test]
fn an_unreadable_saved_state_restarts_coaching_without_losing_history() {
    let env = Env::new(ist(20, 10, 0));
    let mut c = env.open_with(only(false, true, false));
    work_until(&env, &mut c, mins(25), |v| v.reminder().is_some());
    let rev = c.revision().unwrap();
    drop(c);
    let conn = rusqlite::Connection::open(path_of(&env)).unwrap();
    conn.execute(
        "UPDATE coach_state SET revision = revision + 1, state = '{\"format\": 99}'",
        [],
    )
    .unwrap();
    drop(conn);
    let (c, first) = Coach::open(env.store()).unwrap();
    assert!(first.notices.contains(&Notice::StateReset));
    assert_eq!(c.revision(), Some(rev + 2));
    assert_eq!(reminder_events(c.store(), date(20)).len(), 1);
}
