//! Deterministic traces for the combined 5-minute break (DEC15): the cadence of one offer
//! every 30 minutes of time at the PC, the holds with no catch-up, the answers on its
//! reminder, the guided steps in order, the timers a break restarts, and the default for
//! settings saved before it. Time is replayed step by step; nothing sleeps.

// Test helpers outside `#[test]` functions may unwrap too.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use hy_core::clock::ManualClock;
use hy_core::coach::break_routine::{BREATHE_MAX, BREATHE_MIN, EYES_HOLD, WATER_HOLD};
use hy_core::coach::config::{CALM_GAP, CONFIG_KEY, SNOOZE};
use hy_core::coach::*;
use hy_core::schema::{
    Activity, BreakStep, EventPayload, NewEvent, ReminderOutcome, RoutineOutcome, RoutineVariant,
};
use hy_core::store::Store;
use hy_core::time::{UserZone, WorkSchedule};

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

/// Trace harness: a simulated user typing at the PC around a pure engine, ticking every
/// 5 seconds, with the default (combined) settings.
struct H {
    e: Engine,
    wall: DateTime<Utc>,
    config: CoachConfig,
    water: WaterSnapshot,
    last: StepOutput,
    events: Vec<NewEvent>,
}

impl H {
    fn with(config: CoachConfig, water: WaterSnapshot) -> Self {
        let wall = ist(20, 10, 0);
        let mut e = Engine::new(0x5eed);
        let last = e.step(
            &Self::context(wall, Duration::ZERO, config, water),
            Input::Refresh,
        );
        let events = last.events.clone();
        Self {
            e,
            wall,
            config,
            water,
            last,
            events,
        }
    }

    fn new() -> Self {
        Self::with(CoachConfig::default(), WaterSnapshot::default())
    }

    fn context(
        wall: DateTime<Utc>,
        elapsed: Duration,
        config: CoachConfig,
        water: WaterSnapshot,
    ) -> Context {
        Context {
            wall,
            elapsed,
            zone: UserZone::KOLKATA,
            schedule: WorkSchedule::default(),
            water,
            water_entries: Vec::new(),
            config,
            servings: Servings::default(),
        }
    }

    fn step(&mut self, elapsed: Duration, input: Input) -> StepOutput {
        self.wall += chrono::Duration::from_std(elapsed).unwrap();
        let ctx = Self::context(self.wall, elapsed, self.config, self.water);
        let out = self.e.step(&ctx, input);
        for ev in &out.events {
            if let EventPayload::WaterLogged { amount_ml } = ev.payload {
                self.water.logged_ml += amount_ml;
            }
        }
        self.events.extend(out.events.iter().cloned());
        self.last = out.clone();
        out
    }

    fn input(&mut self, input: Input) -> StepOutput {
        self.step(Duration::ZERO, input)
    }

    /// Let time pass with the user typing, ticking every 5 s.
    fn work(&mut self, d: Duration) {
        let mut left = d;
        while left > Duration::ZERO {
            let t = left.min(secs(5));
            left -= t;
            self.step(
                t,
                Input::Tick {
                    idle: Duration::ZERO,
                },
            );
        }
    }

    /// Let time pass ticking every 5 s with no input (locked or asleep).
    fn away(&mut self, d: Duration) {
        let mut left = d;
        while left > Duration::ZERO {
            let t = left.min(secs(5));
            left -= t;
            self.step(t, Input::Tick { idle: d - left });
        }
    }

    /// Tick until `pred` holds; returns how long it took. Panics after `max`.
    fn work_until(&mut self, max: Duration, pred: impl Fn(&CoachView) -> bool) -> Duration {
        let mut waited = Duration::ZERO;
        while !pred(self.view()) {
            assert!(waited < max, "condition not met within {max:?}");
            self.work(secs(5));
            waited += secs(5);
        }
        waited
    }

    fn view(&self) -> &CoachView {
        &self.last.view
    }

    fn break_offer(&self) -> Option<uuid::Uuid> {
        match self.view().bubble {
            Some(Bubble::BreakOffer { offer, .. }) => Some(offer),
            _ => None,
        }
    }

    fn respond(&mut self, response: Response) -> StepOutput {
        let offer = self.break_offer().expect("the break reminder is shown");
        self.input(Input::Respond { offer, response })
    }

    fn act(&mut self, a: BreakAction) -> StepOutput {
        let out = self.input(Input::Break(a));
        assert_eq!(out.outcome, InputOutcome::Applied, "{a:?}");
        out
    }

    fn guide(&mut self, a: GuideAction) {
        let out = self.input(Input::Guide(a));
        assert_eq!(out.outcome, InputOutcome::Applied, "{a:?}");
    }

    /// The break step on show, if a break card is open.
    fn break_step(&self) -> Option<(BreakStep, u32, u32, Duration)> {
        match self.view().bubble {
            Some(Bubble::Break(BreakView {
                stage:
                    BreakStage::Step {
                        step,
                        index,
                        total,
                        length,
                        ..
                    },
            })) => Some((step, index, total, length)),
            _ => None,
        }
    }

    fn in_break_guide(&self) -> bool {
        matches!(self.view().bubble, Some(Bubble::MovementGuide(g)) if g.in_break)
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

    fn responses(&self) -> Vec<ReminderOutcome> {
        self.events
            .iter()
            .filter_map(|e| match e.payload {
                EventPayload::ReminderResponse {
                    activity, outcome, ..
                } => {
                    assert_eq!(activity, Activity::Movement);
                    Some(outcome)
                }
                _ => None,
            })
            .collect()
    }

    /// The break results recorded: steps offered, done, outcome, stopped.
    fn sessions(&self) -> Vec<(Vec<BreakStep>, Vec<BreakStep>, RoutineOutcome, bool)> {
        self.events
            .iter()
            .filter_map(|e| match &e.payload {
                EventPayload::BreakSession {
                    steps,
                    done,
                    outcome,
                    stopped,
                    ..
                } => {
                    e.validate().unwrap();
                    Some((steps.clone(), done.clone(), *outcome, *stopped))
                }
                _ => None,
            })
            .collect()
    }

    fn accum(&self, a: Activity) -> Duration {
        Duration::from_millis(self.e.activity(a).accum_ms)
    }

    fn break_accum(&self) -> Duration {
        Duration::from_millis(self.e.break_timer().accum_ms)
    }
}

#[test]
fn two_hours_at_the_pc_bring_exactly_four_breaks_and_no_separate_reminders() {
    let mut h = H::with(
        CoachConfig::default(),
        // Behind pace all morning: the pace is shown, but raises no reminder of its own.
        WaterSnapshot {
            logged_ml: 0,
            goal_ml: Some(2000),
        },
    );
    let mut shown_at = Vec::new();
    let mut at = Duration::ZERO;
    while at < mins(120) {
        h.work(secs(5));
        at += secs(5);
        if h.break_offer().is_some() {
            shown_at.push(at);
            assert_eq!(h.view().pet, PetMode::BreakOffer);
            assert_eq!(
                h.view().chip(Activity::Movement).status,
                ChipStatus::Showing
            );
            h.respond(Response::Skip);
        }
        // Never any other reminder.
        assert!(!matches!(h.view().bubble, Some(Bubble::Reminder { .. })));
    }
    h.work(mins(1));
    assert_eq!(shown_at, [mins(30), mins(60), mins(90), mins(120)]);
    assert_eq!(h.offers(), [Activity::Movement; 4]);
    assert_eq!(h.responses(), [ReminderOutcome::Skipped; 4]);
    assert!(h.view().water.unwrap().is_behind());
    assert_eq!(h.view().chip(Activity::Water).status, ChipStatus::NotDue);
    assert_eq!(h.view().chip(Activity::EyeRest).status, ChipStatus::NotDue);
    // Three skips in a day do not offer the Light schedule: it changes nothing here.
    assert!(h.view().bubble.is_none());
}

#[test]
fn breaks_taken_at_once_keep_the_thirty_minute_cadence() {
    let mut h = H::new();
    let mut shown = 0;
    let mut at = Duration::ZERO;
    while at < mins(120) {
        h.work(secs(5));
        at += secs(5);
        if h.break_offer().is_some() {
            shown += 1;
            h.respond(Response::Start);
            // Every step answered at once.
            h.act(BreakAction::Next);
            h.act(BreakAction::Next);
            h.guide(GuideAction::Variant(RoutineVariant::Standing));
            for _ in 0..3 {
                h.guide(GuideAction::Done);
            }
            h.act(BreakAction::Next);
            h.act(BreakAction::Close);
        }
    }
    assert_eq!(shown, 4);
    assert_eq!(
        h.responses(),
        [ReminderOutcome::Started; 4],
        "a break started is a break taken"
    );
    assert!(h.sessions().iter().all(|s| s.2 == RoutineOutcome::Done));
}

#[test]
fn a_lock_holds_the_break_back_and_nothing_comes_in_a_burst_after() {
    let mut h = H::new();
    h.work_until(mins(31), |v| {
        matches!(v.bubble, Some(Bubble::BreakOffer { .. }))
    });
    h.respond(Response::Skip);
    h.work(mins(15));
    // A one-hour lock in the middle of the two hours.
    h.input(Input::Lock);
    h.away(mins(60));
    assert_eq!(h.offers().len(), 1, "nothing is offered while locked");
    assert!(h.view().bubble.is_none());
    h.input(Input::Unlock);
    // A lock that long is a confirmed break: the break timer starts afresh, so the next
    // break is a full 30 minutes of time at the PC away, and nothing is owed.
    assert_eq!(h.break_accum(), Duration::ZERO);
    h.work(mins(29));
    assert_eq!(h.offers().len(), 1, "no catch-up after the lock");
    h.work_until(mins(2), |v| {
        matches!(v.bubble, Some(Bubble::BreakOffer { .. }))
    });
    assert_eq!(h.offers().len(), 2);
}

#[test]
fn a_break_showing_when_a_meeting_starts_is_withdrawn_and_not_replayed() {
    let mut h = H::new();
    h.work_until(mins(31), |v| {
        matches!(v.bubble, Some(Bubble::BreakOffer { .. }))
    });
    h.input(Input::SetMeeting(true));
    assert!(h.view().bubble.is_none());
    assert_eq!(h.responses(), [ReminderOutcome::NoResponse]);
    h.work(mins(45));
    h.input(Input::SetMeeting(false));
    h.work(mins(10));
    assert_eq!(h.offers().len(), 1, "held back, never replayed");
}

#[test]
fn not_now_skip_and_snooze_on_the_break_reminder() {
    let mut h = H::new();
    // Not now: the next break a full interval later.
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Dismiss);
    assert_eq!(
        h.view().chip(Activity::Movement).status,
        ChipStatus::Dismissed
    );
    let waited = h.work_until(mins(31), |v| v.bubble.is_some());
    assert_eq!(waited, mins(30));
    // Snooze: back once, 10 minutes later.
    h.respond(Response::Snooze);
    assert_eq!(h.view().pet, PetMode::Snoozed(Activity::Movement));
    assert_eq!(h.view().chip(Activity::Movement).snoozed_for, Some(SNOOZE));
    let waited = h.work_until(mins(11), |v| v.bubble.is_some());
    assert_eq!(waited, SNOOZE);
    // Skip: a full interval again.
    h.respond(Response::Skip);
    let waited = h.work_until(mins(31), |v| v.bubble.is_some());
    assert_eq!(waited, mins(30));
    assert_eq!(
        h.responses(),
        [
            ReminderOutcome::Dismissed,
            ReminderOutcome::Snoozed,
            ReminderOutcome::Skipped
        ]
    );
    // Left unanswered: a waiting badge after 10 minutes, closed after 20 as no response.
    h.work(mins(10));
    assert_eq!(h.view().pet, PetMode::Waiting(Activity::Movement));
    h.work(mins(10));
    assert_eq!(h.responses().last(), Some(&ReminderOutcome::NoResponse));
    // A water answer is not one of the break reminder's.
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    let out = h.respond(Response::LogWater { ml: 250 });
    assert!(matches!(out.outcome, InputOutcome::Ignored(_)));
}

#[test]
fn the_guided_steps_run_in_order_log_water_and_finish_by_resetting_the_timers() {
    let mut h = H::with(
        CoachConfig::default(),
        WaterSnapshot {
            logged_ml: 500,
            goal_ml: Some(2000),
        },
    );
    h.work_until(mins(31), |v| v.bubble.is_some());
    assert!(h.accum(Activity::Movement) >= mins(30));
    h.respond(Response::Start);
    assert_eq!(h.view().pet, PetMode::Exercise);
    assert_eq!(h.view().hold, Some(Hold::ExerciseOrPomodoro));
    // a. Water: "Log 250 ml" logs a real entry; the pace sees it at once.
    assert_eq!(h.break_step(), Some((BreakStep::Water, 0, 4, WATER_HOLD)));
    let out = h.act(BreakAction::LogWater);
    assert!(
        out.events
            .iter()
            .any(|e| e.payload == EventPayload::WaterLogged { amount_ml: 250 })
    );
    assert_eq!(out.view.water.unwrap().logged_ml, 750);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::Break(BreakView {
            stage: BreakStage::Step {
                water_ml: Some(250),
                ..
            }
        }))
    ));
    assert!(matches!(
        h.input(Input::Break(BreakAction::LogWater)).outcome,
        InputOutcome::Ignored(_)
    ));
    // The 10-second hold runs out on screen: on to the eyes.
    h.work(WATER_HOLD);
    assert_eq!(h.break_step(), Some((BreakStep::Eyes, 1, 4, EYES_HOLD)));
    h.work(EYES_HOLD);
    // c. The movement routine, in the guide.
    assert!(h.in_break_guide());
    h.guide(GuideAction::Variant(RoutineVariant::Seated));
    for _ in 0..3 {
        h.guide(GuideAction::Done);
    }
    // d. Closed-eye breathing for what is left of the 5 minutes.
    let (step, index, total, length) = h.break_step().unwrap();
    assert_eq!((step, index, total), (BreakStep::Breathe, 3, 4));
    assert!((BREATHE_MIN..=BREATHE_MAX).contains(&length), "{length:?}");
    h.work(length);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::Break(BreakView {
            stage: BreakStage::Done {
                outcome: RoutineOutcome::Done,
                done: 4,
                total: 4
            }
        }))
    ));
    assert_eq!(
        h.sessions(),
        [(
            BreakStep::ALL.to_vec(),
            BreakStep::ALL.to_vec(),
            RoutineOutcome::Done,
            false
        )]
    );
    let routines: Vec<_> = h
        .events
        .iter()
        .filter_map(|e| match e.payload {
            EventPayload::MovementSession { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(routines, [RoutineOutcome::Done]);
    assert_eq!(h.responses(), [ReminderOutcome::Started]);
    assert_eq!(h.view().chip(Activity::Movement).status, ChipStatus::Done);
    // A finished break restarts the break, eye-rest and movement timers.
    assert!(h.break_accum() < secs(5));
    assert!(h.accum(Activity::EyeRest) < secs(5));
    assert!(h.accum(Activity::Movement) < mins(1));
    h.act(BreakAction::Close);
    assert!(h.view().bubble.is_none());
}

#[test]
fn an_immediate_stop_or_every_step_skipped_records_none_done() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    let movement = h.accum(Activity::Movement);
    h.respond(Response::Start);
    h.act(BreakAction::Stop);
    assert!(h.view().bubble.is_none(), "a stopped break closes");
    assert_eq!(
        h.sessions(),
        [(BreakStep::ALL.to_vec(), vec![], RoutineOutcome::None, true)]
    );
    assert_eq!(
        h.view().chip(Activity::Movement).status,
        ChipStatus::NoneRecorded
    );
    assert_eq!(h.accum(Activity::Movement), movement, "not restarted");
    // The next break a full interval later.
    let waited = h.work_until(mins(31), |v| v.bubble.is_some());
    assert_eq!(waited, mins(30));

    // Every step skipped, the movement step with "Skip this step".
    let movement = h.accum(Activity::Movement);
    h.respond(Response::Start);
    h.act(BreakAction::Skip);
    h.act(BreakAction::Skip);
    assert!(h.in_break_guide());
    h.guide(GuideAction::Later);
    h.act(BreakAction::Skip);
    assert_eq!(
        h.sessions()[1],
        (BreakStep::ALL.to_vec(), vec![], RoutineOutcome::None, false)
    );
    assert_eq!(h.accum(Activity::Movement), movement);
    assert!(
        !h.events
            .iter()
            .any(|e| matches!(e.payload, EventPayload::MovementSession { .. })),
        "a skipped movement step records no routine"
    );
}

#[test]
fn stopping_part_way_records_the_steps_done() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    h.act(BreakAction::Skip);
    h.act(BreakAction::Next);
    h.guide(GuideAction::Variant(RoutineVariant::Seated));
    h.guide(GuideAction::Done);
    // The close button stops the break from inside the movement guide.
    h.input(Input::CloseBubble);
    assert!(h.view().bubble.is_none());
    assert!(h.view().guide.is_none());
    assert_eq!(
        h.sessions(),
        [(
            BreakStep::ALL.to_vec(),
            vec![BreakStep::Eyes, BreakStep::Movement],
            RoutineOutcome::Partial,
            true
        )]
    );
    // The routine is recorded as a partial stop, and both timers restart.
    assert!(h.events.iter().any(|e| matches!(
        e.payload,
        EventPayload::MovementSession {
            outcome: RoutineOutcome::Partial,
            steps_done: 1,
            ..
        }
    )));
    assert_eq!(h.accum(Activity::Movement), Duration::ZERO);
    assert_eq!(h.accum(Activity::EyeRest), Duration::ZERO);
    assert_eq!(
        h.view().chip(Activity::Movement).status,
        ChipStatus::Partial
    );
}

#[test]
fn a_hold_that_starts_ends_the_break_as_stopped() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    h.work(WATER_HOLD);
    h.input(Input::Lock);
    assert!(h.view().bubble.is_none());
    assert_eq!(
        h.sessions(),
        [(
            BreakStep::ALL.to_vec(),
            vec![BreakStep::Water],
            RoutineOutcome::Partial,
            true
        )]
    );
    // Starting a Pomodoro ends one too.
    h.input(Input::Unlock);
    h.work_until(mins(32), |v| v.bubble.is_some());
    h.respond(Response::Start);
    h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    assert_eq!(h.sessions().len(), 2);
    assert_eq!(h.sessions()[1].2, RoutineOutcome::None);
}

#[test]
fn steps_whose_category_is_off_are_left_out() {
    let mut config = CoachConfig::default();
    config.categories.water = false;
    config.categories.movement = false;
    let mut h = H::with(config, WaterSnapshot::default());
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    assert_eq!(h.break_step(), Some((BreakStep::Eyes, 0, 2, EYES_HOLD)));
    h.act(BreakAction::Next);
    assert_eq!(
        h.break_step().map(|s| (s.0, s.1)),
        Some((BreakStep::Breathe, 1))
    );
}

#[test]
fn the_interval_follows_the_setting_and_separate_restores_the_old_reminders() {
    let mut h = H::with(
        CoachConfig {
            break_minutes: 45,
            ..CoachConfig::default()
        },
        WaterSnapshot::default(),
    );
    let waited = h.work_until(mins(46), |v| v.bubble.is_some());
    assert_eq!(waited, mins(45));
    // Switching to separate reminders withdraws the break reminder; the eye rest, long
    // since due by its own timer, is offered after the calm gap.
    h.config.reminder_style = ReminderStyle::Separate;
    h.input(Input::Refresh);
    assert!(!matches!(h.view().bubble, Some(Bubble::BreakOffer { .. })));
    assert_eq!(h.responses(), [ReminderOutcome::NoResponse]);
    h.work_until(mins(6), |v| v.bubble.is_some());
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::Reminder {
            activity: Activity::Movement | Activity::EyeRest,
            ..
        })
    ));
}

#[test]
fn a_running_break_survives_a_save_and_ends_on_restart() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    h.act(BreakAction::Next);
    let text = serde_json::to_string(h.e.state()).unwrap();
    let back = Engine::from_state(serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(back, h.e);
    let ctx = H::context(h.wall, mins(1), h.config, h.water);
    let out = h.e.restart(&ctx);
    assert!(out.events.iter().any(|e| matches!(
        &e.payload,
        EventPayload::BreakSession {
            outcome: RoutineOutcome::Partial,
            stopped: true,
            ..
        }
    )));
    assert!(out.view.bubble.is_none());
}

#[test]
fn settings_saved_before_the_combined_break_get_it_on_upgrade() {
    let clock = Arc::new(ManualClock::new(ist(20, 10, 0)));
    let store = Store::open_in_memory(clock.clone()).unwrap();
    // A settings record written before DEC15, without the new fields.
    store
        .put_setting(
            CONFIG_KEY,
            &serde_json::json!({
                "intensity": "balanced",
                "categories": { "movement": true, "eye_rest": true, "water": true },
                "hide_during_presentation": true,
                "media_in_use_hint": false,
                "long_day_check_in": { "enabled": true, "hours": 9 },
                "pomodoro_auto_start": false,
                "movement_version": "ask",
                "review": { "day": "sat", "at": "17:00" },
                "meditation": { "at": null }
            }),
        )
        .unwrap();
    let (mut c, _) = Coach::open(store).unwrap();
    let config = c.config().unwrap();
    assert_eq!(config.reminder_style, ReminderStyle::Combined);
    assert_eq!(config.break_minutes, 30);
    for _ in 0..(31 * 12) {
        clock.advance(chrono::Duration::seconds(5));
        c.tick(Duration::ZERO).unwrap();
        if c.view().bubble.is_some() {
            break;
        }
    }
    assert!(matches!(c.view().bubble, Some(Bubble::BreakOffer { .. })));
    // The offer and its answer are stored as a movement reminder.
    let offer = c.view().reminder().unwrap().0;
    c.handle(Input::Respond {
        offer,
        response: Response::Skip,
    })
    .unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let kinds: Vec<_> = c
        .store()
        .events_on(today)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.payload {
            EventPayload::ReminderOffered { activity, .. } => Some((activity, None)),
            EventPayload::ReminderResponse {
                activity, outcome, ..
            } => Some((activity, Some(outcome))),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            (Activity::Movement, None),
            (Activity::Movement, Some(ReminderOutcome::Skipped))
        ]
    );
}

// ------------------------------------------- review follow-ups: exclusivity, water, style

/// Water the shell has recorded (the menu's quick log or the water panel), told to the
/// engine as the shell does.
fn shell_logged(h: &mut H, ml: u32) -> StepOutput {
    h.water.logged_ml += ml;
    h.input(Input::Water(WaterInput::Logged {
        entry: uuid::Uuid::from_u128(u128::from(ml)),
        ml,
    }))
}

fn engine_water_entries(h: &H) -> usize {
    h.events
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::WaterLogged { .. }))
        .count()
}

#[test]
fn a_paused_pomodoro_cannot_resume_beside_a_running_break() {
    let mut h = H::new();
    let out = h.input(Input::StartPomodoro {
        preset: PomodoroPreset::Classic,
        replace_focus: false,
        resume_coaching: false,
    });
    assert_eq!(out.outcome, InputOutcome::Applied);
    h.work(mins(1));
    assert_eq!(h.input(Input::PausePomodoro).outcome, InputOutcome::Applied);
    // Still at the PC: the break reminder comes on its own timer.
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    assert!(h.view().in_break);
    let left = h.view().pomodoro.unwrap().left;
    // Resume is refused while the break runs: the work timer stays put.
    let out = h.input(Input::ResumePomodoro);
    assert_eq!(out.outcome, InputOutcome::Ignored("a break is running"));
    h.work(secs(10));
    let p = h.view().pomodoro.unwrap();
    assert!(p.paused.is_some());
    assert_eq!(p.left, left);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Eyes));
    // No other Pomodoro transition starts a timer beside it either.
    for i in [
        Input::NextPomodoroRound,
        Input::StartPomodoroBreak,
        Input::SkipPomodoroBreak,
    ] {
        assert_eq!(
            h.input(i).outcome,
            InputOutcome::Ignored("a break is running")
        );
    }
    // Once the break has ended, the Pomodoro resumes as usual.
    h.act(BreakAction::Stop);
    assert!(!h.view().in_break);
    assert_eq!(h.sessions().len(), 1);
    assert_eq!(
        h.input(Input::ResumePomodoro).outcome,
        InputOutcome::Applied
    );
    h.work(secs(10));
    assert!(h.view().pomodoro.unwrap().left < left);
    assert_eq!(h.sessions().len(), 1, "no second break result");
}

#[test]
fn water_logged_from_the_menu_completes_the_water_step_and_the_break_goes_on() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Water));
    // A preset or custom amount from the menu: the shell records it; the break takes it.
    let out = shell_logged(&mut h, 300);
    assert_eq!(out.outcome, InputOutcome::Applied);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Eyes));
    assert_eq!(
        engine_water_entries(&h),
        0,
        "the shell's entry is the only one"
    );
    assert!(h.sessions().is_empty(), "the break was not stopped");
    // On a later step it is just recorded; the break goes on, with no card over it.
    shell_logged(&mut h, 150);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Eyes));
    assert!(h.sessions().is_empty());
    h.act(BreakAction::Next);
    assert!(h.in_break_guide());
    shell_logged(&mut h, 200);
    assert!(h.in_break_guide());
    h.guide(GuideAction::Later);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Breathe));
    h.act(BreakAction::Next);
    let s = h.sessions();
    assert_eq!(s.len(), 1);
    assert_eq!(
        s[0].1,
        [BreakStep::Water, BreakStep::Eyes, BreakStep::Breathe]
    );
    assert!(!s[0].3);
    assert_eq!(engine_water_entries(&h), 0);
}

#[test]
fn the_water_panel_opens_over_a_break_which_waits_and_comes_back() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    h.work(secs(5));
    assert_eq!(
        h.input(Input::Water(WaterInput::OpenPanel)).outcome,
        InputOutcome::Applied
    );
    assert!(matches!(h.view().bubble, Some(Bubble::WaterPanel { .. })));
    assert!(h.view().in_break);
    // The water step waits behind the panel.
    h.work(secs(30));
    assert!(matches!(h.view().bubble, Some(Bubble::WaterPanel { .. })));
    // Cancel (the close button) returns to the break where it was.
    assert_eq!(h.input(Input::CloseBubble).outcome, InputOutcome::Applied);
    let left = match h.view().bubble {
        Some(Bubble::Break(BreakView {
            stage:
                BreakStage::Step {
                    step: BreakStep::Water,
                    left,
                    ..
                },
        })) => left,
        b => panic!("the break's water step, not {b:?}"),
    };
    assert_eq!(left, WATER_HOLD - secs(5));
    assert!(h.sessions().is_empty(), "cancelling the panel is not Stop");
    // The panel's own time closing it returns to the break too.
    h.input(Input::Water(WaterInput::OpenPanel));
    h.work(hy_core::coach::config::WATER_PANEL_SHOWN_FOR);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Water));
    assert!(h.sessions().is_empty());
    // A different amount logged in the panel completes the water step: one entry, the
    // panel closes and the break goes on to the eyes.
    h.input(Input::Water(WaterInput::OpenPanel));
    shell_logged(&mut h, 400);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Eyes));
    assert_eq!(engine_water_entries(&h), 0);
    // On the eye step the panel opens and closes again without ending the break.
    h.input(Input::Water(WaterInput::OpenPanel));
    h.input(Input::CloseBubble);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Eyes));
    h.act(BreakAction::Stop);
    let s = h.sessions();
    assert_eq!(s.len(), 1, "exactly one break result");
    assert_eq!(s[0].1, [BreakStep::Water]);
    assert!(s[0].3);
}

#[test]
fn the_break_log_button_and_a_menu_log_record_one_entry_each() {
    let mut h = H::new();
    h.work_until(mins(31), |v| v.bubble.is_some());
    h.respond(Response::Start);
    h.act(BreakAction::LogWater);
    assert_eq!(engine_water_entries(&h), 1);
    // A second drink from the menu on the same step: the shell's entry, the step done.
    shell_logged(&mut h, 250);
    assert_eq!(engine_water_entries(&h), 1);
    assert_eq!(h.break_step().map(|s| s.0), Some(BreakStep::Eyes));
    assert!(h.sessions().is_empty());
}

/// Switching explicitly to separate reminders keeps the movement and eye-rest timers
/// accrued while the combined break was on (DEC15): long-overdue reminders come one at a
/// time, with the calm gap between them, never as a burst.
#[test]
fn switching_to_separate_reminders_brings_no_catch_up_burst() {
    let mut h = H::new();
    // Two hours with every break skipped: movement and eye rest are long overdue.
    for _ in 0..4 {
        h.work_until(mins(31), |v| v.bubble.is_some());
        h.respond(Response::Skip);
    }
    h.work(mins(6));
    assert!(h.accum(Activity::Movement) >= mins(120));
    let before = h.offers().len();
    h.config.reminder_style = ReminderStyle::Separate;
    h.input(Input::Refresh);
    h.work(secs(5));
    // One reminder at once.
    assert_eq!(h.offers().len(), before + 1);
    let first = match h.view().bubble {
        Some(Bubble::Reminder {
            offer, activity, ..
        }) => (offer, activity),
        b => panic!("a separate reminder, not {b:?}"),
    };
    h.input(Input::Respond {
        offer: first.0,
        response: Response::Done,
    });
    // Nothing else during the calm gap.
    h.work(CALM_GAP - secs(5));
    assert_eq!(h.offers().len(), before + 1);
    assert!(h.view().bubble.is_none(), "{:?}", h.view().bubble);
    // Then at most the other overdue reminder, again one.
    h.work(secs(10));
    assert_eq!(h.offers().len(), before + 2);
    assert!(matches!(
        h.view().bubble,
        Some(Bubble::Reminder { activity, .. }) if activity != first.1
    ));
}
