//! The link between the engine and the shell, tested without Windows: the view-to-shell
//! mapping for each state, routing of every answer, and whole sequences with the real
//! engine and store on a simulated clock (a reminder falling due and being answered,
//! presentations, lock and sleep, the user hiding the pet, the tray's "Show pet"), with
//! the rule that at most one bubble is ever open and nothing withdrawn is replayed.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use hy_core::clock::ManualClock;
use hy_core::coach::movement::PROVISIONAL_NOTICE;
use hy_core::coach::water::WaterPace;
use hy_core::coach::{
    ActiveRoutine, Bubble as CoachBubble, CheckInAnswer, Chip, ChipStatus, CoachView, GuideAction,
    GuideStage, GuideView, HiddenReason, Input, PauseLength, PetMode, PhoneInput, PomodoroPause,
    PomodoroPhase, PomodoroPreset, PomodoroView, RecoveryAction, RecoveryPlan, RecoveryView,
    Reflection, ReminderReason, Response, Servings, WaterEdit, WaterEntry, WaterInput,
};
use hy_core::schema::{
    Activity, EventPayload, GoalKind, PhoneLabel, ReminderOutcome, RoutineOutcome, RoutineVariant,
    SessionState, StepOutcome,
};
use hy_core::store::Store;
use proptest::prelude::*;
use uuid::Uuid;

use super::present::{self, BubbleKind, action, menu_actions, pet_look};
use super::route::{
    Command, EntryChange, RouteState, bubble_change, bubble_command, menu_command, route,
};
use super::{CoachLink, LinkSink};
use crate::shell::{
    Bubble, BubbleOutcome, CloseCause, Effect, Handled, HideReason, MenuAction, Open, PetShell,
    ShellEvent, ShellEventSink, ShellModel,
};
use crate::state::PetState;

// ------------------------------------------------------------------ fixtures

fn chip(activity: Activity, status: ChipStatus) -> Chip {
    Chip {
        activity,
        status,
        snoozed_for: None,
    }
}

fn view(pet: PetMode) -> CoachView {
    CoachView {
        visible: !matches!(pet, PetMode::Hidden(_)),
        pet,
        bubble: None,
        chips: [
            chip(Activity::Movement, ChipStatus::NotDue),
            chip(Activity::EyeRest, ChipStatus::NotDue),
            chip(Activity::Water, ChipStatus::NoGoal),
        ],
        session: SessionState::Active,
        hold: None,
        water: None,
        water_entries: Vec::new(),
        servings: Servings::default(),
        pomodoro: None,
        focus_left: None,
        meeting: false,
        active_routine: None,
        guide: None,
        recovery: RecoveryView {
            today: oct(20),
            marked_today: false,
            plan: None,
            recovery_day: false,
            can_open: true,
        },
    }
}

fn oct(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn guide(stage: GuideStage, variant: Option<RoutineVariant>) -> GuideView {
    GuideView {
        day: Weekday::Tue,
        today: Weekday::Tue,
        variant,
        stage,
        from_reminder: true,
        open: true,
        default_variant: None,
    }
}

fn plan(d: u32) -> RecoveryPlan {
    RecoveryPlan {
        date: oct(d),
        more_breaks: true,
        walk_at: NaiveTime::from_hms_opt(16, 0, 0),
        walk_repeat: true,
        lunch_away: true,
        calm_pause: true,
        finish_at: NaiveTime::from_hms_opt(18, 30, 0),
        reflection: true,
    }
}

fn mins(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

fn pomodoro(phase: PomodoroPhase, paused: Option<PomodoroPause>) -> PomodoroView {
    PomodoroView {
        phase,
        left: Duration::from_secs(24 * 60 + 55),
        paused,
        round: 1,
    }
}

fn all_bubbles() -> Vec<CoachBubble> {
    let offer = Uuid::from_u128(7);
    vec![
        CoachBubble::Reminder {
            offer,
            activity: Activity::Movement,
            reason: ReminderReason::Movement { worked: mins(48) },
        },
        CoachBubble::Reminder {
            offer,
            activity: Activity::EyeRest,
            reason: ReminderReason::EyeRest {
                screen_time: mins(20),
            },
        },
        CoachBubble::Reminder {
            offer,
            activity: Activity::Water,
            reason: ReminderReason::Water {
                expected_ml: 390,
                logged_ml: 250,
                behind_ml: 140,
            },
        },
        CoachBubble::PomodoroBreak {
            length: mins(5),
            water_behind: None,
        },
        CoachBubble::PomodoroRoundDone,
        CoachBubble::PomodoroPausedByLock,
        CoachBubble::LongDayCheckIn { hours: 9 },
        CoachBubble::LighterSchedule,
        CoachBubble::MovementGuide(guide(GuideStage::Intro, None)),
        CoachBubble::MovementGuide(GuideView {
            day: Weekday::Sun,
            today: Weekday::Sun,
            ..guide(GuideStage::Sunday, None)
        }),
        CoachBubble::MovementGuide(guide(GuideStage::ChooseDay, None)),
        CoachBubble::MovementGuide(guide(
            GuideStage::Step { index: 0, total: 3 },
            Some(RoutineVariant::Standing),
        )),
        CoachBubble::MovementGuide(guide(
            GuideStage::Step { index: 2, total: 3 },
            Some(RoutineVariant::Seated),
        )),
        CoachBubble::MovementGuide(guide(
            GuideStage::Result {
                outcome: RoutineOutcome::Partial,
                done: 1,
                skipped: 1,
                total: 3,
                stopped: true,
            },
            Some(RoutineVariant::Seated),
        )),
        CoachBubble::MovementGuide(guide(
            GuideStage::Result {
                outcome: RoutineOutcome::None,
                done: 0,
                skipped: 3,
                total: 3,
                stopped: false,
            },
            Some(RoutineVariant::Seated),
        )),
        CoachBubble::DeadlineDay,
        CoachBubble::RecoveryInvite { next: oct(21) },
        CoachBubble::RecoveryDates {
            dates: [Some(oct(22)), Some(oct(23)), Some(oct(24)), Some(oct(26))],
        },
        CoachBubble::RecoveryPlanned { plan: plan(21) },
        CoachBubble::RecoveryMorning {
            plan: plan(20),
            water_goal_ml: Some(2000),
        },
        CoachBubble::RecoveryWalk,
        CoachBubble::RecoveryReflection,
        CoachBubble::WaterLogged {
            entry: Some(entry_id(1)),
            ml: 250,
        },
        CoachBubble::WaterLogged {
            entry: None,
            ml: 250,
        },
        CoachBubble::WaterPanel {
            edit: None,
            other_ml: 250,
        },
        CoachBubble::WaterPanel {
            edit: Some(WaterEdit {
                entry: entry_id(2),
                ml: 300,
            }),
            other_ml: 2000,
        },
        CoachBubble::PhoneNote {
            entry: Some(entry_id(3)),
            at: NaiveTime::from_hms_opt(11, 52, 0).unwrap(),
            label: None,
        },
        CoachBubble::PhoneNote {
            entry: Some(entry_id(3)),
            at: NaiveTime::from_hms_opt(11, 52, 0).unwrap(),
            label: Some(PhoneLabel::Work),
        },
        CoachBubble::PhoneNote {
            entry: None,
            at: NaiveTime::from_hms_opt(11, 52, 0).unwrap(),
            label: None,
        },
    ]
}

fn entry_id(n: u128) -> Uuid {
    Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0000 + n)
}

/// Today's water entries `ml`, one every half hour from 09:00, ids `entry_id(1..)`.
fn entries(ml: &[u32]) -> Vec<WaterEntry> {
    ml.iter()
        .zip(1u128..)
        .map(|(ml, n)| WaterEntry {
            id: entry_id(n),
            time: NaiveTime::from_hms_opt(9, 0, 0).unwrap()
                + chrono::Duration::minutes(30 * i64::try_from(n - 1).unwrap()),
            amount_ml: *ml,
        })
        .collect()
}

/// A view with today's `entries` against a 2000 ml goal.
fn with_water(ml: &[u32]) -> CoachView {
    let mut v = view(PetMode::Idle);
    v.water_entries = entries(ml);
    let logged = ml.iter().sum();
    v.water = Some(WaterPace {
        logged_ml: logged,
        goal_ml: 2000,
        expected_ml: 500,
        behind_ml: 0,
        goal_reached: logged >= 2000,
    });
    v
}

/// Every action id on a shell bubble, in its rows and its own actions.
fn all_actions(b: &Bubble) -> Vec<String> {
    b.rows
        .iter()
        .flat_map(|r| r.actions.iter())
        .chain(b.actions.iter())
        .map(|a| a.id.clone())
        .collect()
}

// ------------------------------------------------------------ view mapping

#[test]
fn every_pet_mode_maps_to_a_state_with_words() {
    let cases: Vec<(PetMode, PetState, Option<&str>)> = vec![
        (PetMode::Idle, PetState::Idle, None),
        (
            PetMode::Hidden(HiddenReason::Presentation),
            PetState::Idle,
            None,
        ),
        (PetMode::Away, PetState::Quiet, Some("Away")),
        (
            PetMode::Paused {
                left: Some(mins(60)),
                rest_of_today: false,
            },
            PetState::Paused,
            Some("Paused 1 h"),
        ),
        (
            PetMode::Paused {
                left: Some(Duration::from_secs(25 * 60 + 1)),
                rest_of_today: false,
            },
            PetState::Paused,
            Some("Paused 26 min"),
        ),
        (
            PetMode::Paused {
                left: None,
                rest_of_today: true,
            },
            PetState::Paused,
            Some("Paused today"),
        ),
        (
            PetMode::Paused {
                left: None,
                rest_of_today: false,
            },
            PetState::Paused,
            Some("Paused"),
        ),
        (PetMode::OffHours, PetState::Quiet, None),
        (
            PetMode::Focus { left: mins(90) },
            PetState::Focus,
            Some("Focus 1 h 30 min"),
        ),
        (
            PetMode::Focus {
                left: Duration::from_secs(59),
            },
            PetState::Focus,
            Some("Focus 1 min"),
        ),
        (PetMode::Meeting, PetState::Meeting, None),
        (
            PetMode::Pomodoro(pomodoro(PomodoroPhase::Work, None)),
            PetState::Pomodoro,
            Some("Work 24:55"),
        ),
        (
            PetMode::Pomodoro(pomodoro(PomodoroPhase::Break, None)),
            PetState::Pomodoro,
            Some("Break 24:55"),
        ),
        (
            PetMode::Pomodoro(pomodoro(PomodoroPhase::BreakOffered, None)),
            PetState::Pomodoro,
            Some("Break ready"),
        ),
        (
            PetMode::Pomodoro(pomodoro(PomodoroPhase::RoundDone, None)),
            PetState::Pomodoro,
            Some("Round done"),
        ),
        (
            PetMode::Pomodoro(pomodoro(
                PomodoroPhase::Work,
                Some(PomodoroPause::LockOrSleep),
            )),
            PetState::Pomodoro,
            Some("Pomodoro paused"),
        ),
        (PetMode::Exercise, PetState::Exercise, None),
        (
            PetMode::Reminder(Activity::EyeRest),
            PetState::Reminder,
            Some("Eye rest"),
        ),
        (PetMode::Waiting(Activity::Water), PetState::Waiting, None),
        (PetMode::Nudge, PetState::Cue, None),
        (
            PetMode::Snoozed(Activity::Movement),
            PetState::Snoozed,
            None,
        ),
    ];
    for (mode, state, tag) in cases {
        let v = view(mode);
        assert_eq!(
            pet_look(&v, false),
            (state, tag.map(str::to_owned)),
            "{mode:?}"
        );
    }
    // A snoozed chip gives the time left.
    let mut v = view(PetMode::Snoozed(Activity::Movement));
    v.chips[0] = Chip {
        activity: Activity::Movement,
        status: ChipStatus::Snoozed,
        snoozed_for: Some(Duration::from_secs(7 * 60 + 30)),
    };
    assert_eq!(
        pet_look(&v, false),
        (PetState::Snoozed, Some("Snoozed 8 min".to_owned()))
    );
}

#[test]
fn celebration_shows_only_over_idle() {
    assert_eq!(
        pet_look(&view(PetMode::Idle), true).0,
        PetState::Celebration
    );
    assert_eq!(pet_look(&view(PetMode::Meeting), true).0, PetState::Meeting);
    assert_eq!(
        pet_look(&view(PetMode::Hidden(HiddenReason::Presentation)), true).0,
        PetState::Idle
    );
}

#[test]
fn every_engine_bubble_is_valid_and_every_action_routes() {
    let many: Vec<u32> = (0..30).map(|i| 50 + i * 10).collect();
    for v0 in [
        view(PetMode::Idle),
        with_water(&[250, 300]),
        with_water(&many),
    ] {
        for b in all_bubbles() {
            let mut v = v0.clone();
            v.bubble = Some(b);
            let mut shell = present::bubble(&b, &v);
            let kind = BubbleKind::of(&b);
            shell.id = kind.id(42);
            assert_eq!(shell.validate(), Ok(()), "{b:?}");
            assert_eq!(BubbleKind::parse(&shell.id), Some(kind));
            assert!(!shell.actions.is_empty());
            for a in all_actions(&shell) {
                assert!(
                    bubble_command(kind, &a).is_some() || bubble_change(kind, &a).is_some(),
                    "{b:?}: action {a} routes nowhere",
                );
                assert!(
                    bubble_command(kind, &a).is_none() || bubble_change(kind, &a).is_none(),
                    "{b:?}: action {a} routes twice",
                );
            }
        }
    }
}

#[test]
fn reminder_card_shows_one_activity_with_three_chips() {
    let mut v = view(PetMode::Reminder(Activity::Water));
    v.chips = [
        chip(Activity::Movement, ChipStatus::Due),
        chip(Activity::EyeRest, ChipStatus::Done),
        chip(Activity::Water, ChipStatus::Showing),
    ];
    let b = CoachBubble::Reminder {
        offer: Uuid::from_u128(1),
        activity: Activity::Water,
        reason: ReminderReason::Water {
            expected_ml: 390,
            logged_ml: 250,
            behind_ml: 140,
        },
    };
    let card = present::bubble(&b, &v);
    assert_eq!(
        card.title,
        "Water: 140 ml behind your own pace (about 390 ml expected by now)"
    );
    let chips: Vec<(&str, &str, bool)> = card
        .chips
        .iter()
        .map(|c| (c.label.as_str(), c.status.as_str(), c.highlighted))
        .collect();
    assert_eq!(
        chips,
        [
            ("Movement", "also due", false),
            ("Eye rest", "done (self-reported)", false),
            ("Water", "due", true),
        ]
    );
    let actions: Vec<&str> = card.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(
        actions,
        ["Log 250 ml", "Already did it", "Snooze 10 min", "Not now"]
    );
    let offer = Uuid::from_u128(1);
    let kind = BubbleKind::Reminder { offer };
    assert_eq!(
        bubble_command(kind, action::LOG_WATER),
        Some(Input::Respond {
            offer,
            response: Response::LogWater { ml: 250 }
        })
    );
}

#[test]
fn bubble_ids_parse_back_and_refuse_junk() {
    let offer = Uuid::from_u128(99);
    for k in [
        BubbleKind::Reminder { offer },
        BubbleKind::PomodoroBreak,
        BubbleKind::PomodoroRoundDone,
        BubbleKind::PomodoroPaused,
        BubbleKind::CheckIn,
        BubbleKind::Lighter,
    ] {
        assert_eq!(BubbleKind::parse(&k.id(3)), Some(k));
        assert_ne!(k.id(3), k.id(4));
    }
    for junk in [
        "",
        "reminder:xyz#1",
        "check-in",
        "check-in#x",
        "other#1",
        "selftest",
    ] {
        assert_eq!(BubbleKind::parse(junk), None, "{junk}");
    }
}

fn ids(m: &[MenuAction]) -> Vec<&str> {
    m.iter().map(|a| a.id.as_str()).collect()
}

#[test]
fn menu_follows_the_engine_state_and_every_entry_routes() {
    let mut views = Vec::new();
    let idle = view(PetMode::Idle);
    let m = menu_actions(&idle);
    assert_eq!(
        ids(&m),
        [
            "water_150",
            "water_250",
            "water_500",
            "water_panel",
            "movement_start",
            "break",
            "deadline_day",
            "focus_30",
            "focus_60",
            "focus_90",
            "pomodoro_25",
            "pomodoro_50",
            "phone_note",
            "meeting_on",
            "pause_30",
            "pause_60",
            "pause_today",
            "pause_on",
        ]
    );
    // With nothing running, only the pause entries go to the tray (wireframe T1:
    // "Pause..."); a running movement routine adds its results (below).
    let tray: Vec<&str> = m
        .iter()
        .filter(|a| a.in_tray)
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(tray, ["pause_30", "pause_60", "pause_today", "pause_on"]);
    // Entries that open a card keep the keyboard panel open to show it; the rest close it.
    let continuing: Vec<&str> = m
        .iter()
        .filter(|a| a.continues)
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(
        continuing,
        [
            "water_150",
            "water_250",
            "water_500",
            "water_panel",
            "phone_note"
        ]
    );
    // The water entry gives today's total, against the goal when there is one.
    assert!(
        m.iter()
            .any(|a| a.id == "water_panel" && a.label == "Water... (0 ml today)")
    );
    let watered = with_water(&[250, 500]);
    assert!(
        menu_actions(&watered)
            .iter()
            .any(|a| a.id == "water_panel" && a.label == "Water... (750 / 2000 ml)")
    );
    views.push(idle);

    // Paused: resume in the menu and tray; a Pomodoro says it resumes coaching.
    let mut paused = view(PetMode::Paused {
        left: None,
        rest_of_today: false,
    });
    paused.session = SessionState::Paused;
    let m = menu_actions(&paused);
    assert!(ids(&m).contains(&"resume"));
    assert!(m.iter().any(|a| a.id == "resume" && a.in_tray));
    assert!(
        m.iter()
            .any(|a| a.id == "pomodoro_25_rc" && a.label == "Pomodoro 25/5 (resumes coaching)")
    );
    views.push(paused);

    // Focus on: End focus; a Pomodoro says it ends focus.
    let mut focus = view(PetMode::Focus { left: mins(30) });
    focus.focus_left = Some(mins(30));
    let m = menu_actions(&focus);
    assert!(ids(&m).contains(&"focus_end"));
    assert!(
        m.iter()
            .any(|a| a.id == "pomodoro_50_rf" && a.label == "Pomodoro 50/10 (ends focus)")
    );
    views.push(focus);

    // A Pomodoro running: pause and end; focus says it ends the Pomodoro.
    let mut running = view(PetMode::Pomodoro(pomodoro(PomodoroPhase::Work, None)));
    running.pomodoro = Some(pomodoro(PomodoroPhase::Work, None));
    let m = menu_actions(&running);
    assert!(ids(&m).contains(&"pomodoro_pause"));
    assert!(ids(&m).contains(&"pomodoro_end"));
    assert!(
        m.iter()
            .any(|a| a.id == "focus_60_rp" && a.label == "Focus 60 min (ends Pomodoro)")
    );
    views.push(running);

    // A Pomodoro paused by a lock: resume, and a click reopens its card.
    let mut locked = view(PetMode::Pomodoro(pomodoro(
        PomodoroPhase::Work,
        Some(PomodoroPause::LockOrSleep),
    )));
    locked.pomodoro = Some(pomodoro(
        PomodoroPhase::Work,
        Some(PomodoroPause::LockOrSleep),
    ));
    let m = menu_actions(&locked);
    assert_eq!(m[0].id, "open_pending");
    assert_eq!(m[0].label, "Show Pomodoro card");
    assert!(ids(&m).contains(&"pomodoro_resume"));
    assert!(present::click_reopens(&locked));
    views.push(locked);

    // A waiting reminder: shown first; a click reopens it.
    let mut waiting = view(PetMode::Waiting(Activity::EyeRest));
    waiting.chips[1] = chip(Activity::EyeRest, ChipStatus::Waiting);
    let m = menu_actions(&waiting);
    assert_eq!(m[0].label, "Show waiting reminder");
    assert!(present::click_reopens(&waiting));
    views.push(waiting);

    // Movement started from a reminder: the result entries replace "Today's movement",
    // in the menu and the tray.
    let results = ["movement_done", "movement_partial", "movement_stop"];
    let in_tray = |m: &[MenuAction]| -> Vec<String> {
        m.iter()
            .filter(|a| a.in_tray)
            .map(|a| a.id.clone())
            .collect()
    };
    let mut moving = view(PetMode::Exercise);
    moving.chips[0] = chip(Activity::Movement, ChipStatus::InProgress);
    moving.active_routine = Some(ActiveRoutine {
        activity: Activity::Movement,
        offer: Some(Uuid::from_u128(7)),
    });
    let m = menu_actions(&moving);
    for r in results {
        assert!(ids(&m).contains(&r), "{r}");
        assert!(in_tray(&m).contains(&r.to_owned()), "{r}");
    }
    assert!(!ids(&m).contains(&"movement_start"));
    views.push(moving);
    // A routine from the pet menu under any hold: the pet shows the hold, and the result
    // entries follow the engine's routine state, not the pet.
    let manual = Some(ActiveRoutine {
        activity: Activity::Movement,
        offer: None,
    });
    let mut held = Vec::new();
    let mut paused_moving = view(PetMode::Paused {
        left: None,
        rest_of_today: false,
    });
    paused_moving.session = SessionState::Paused;
    held.push(paused_moving);
    held.push(view(PetMode::OffHours));
    held.push(view(PetMode::Focus { left: mins(30) }));
    let mut meeting_moving = view(PetMode::Meeting);
    meeting_moving.meeting = true;
    held.push(meeting_moving);
    held.push(view(PetMode::Hidden(HiddenReason::Presentation)));
    held.push(view(PetMode::Away));
    for mut v in held {
        v.active_routine = manual;
        let m = menu_actions(&v);
        for r in results {
            assert!(ids(&m).contains(&r), "{r} under {:?}", v.pet);
            assert!(in_tray(&m).contains(&r.to_owned()), "{r} under {:?}", v.pet);
        }
        assert!(!ids(&m).contains(&"movement_start"), "{:?}", v.pet);
        views.push(v);
    }
    // The pet alone does not make a routine: no routine state, no result entries.
    let exercise_look = view(PetMode::Exercise);
    assert!(ids(&menu_actions(&exercise_look)).contains(&"movement_start"));
    // A guided routine waiting behind the pet: the menu brings its card back, and the
    // result entries answer it in one go, as for any running routine.
    let mut guided = view(PetMode::Exercise);
    guided.chips[0] = chip(Activity::Movement, ChipStatus::InProgress);
    guided.active_routine = Some(ActiveRoutine {
        activity: Activity::Movement,
        offer: Some(Uuid::from_u128(7)),
    });
    guided.guide = Some(GuideView {
        open: false,
        ..guide(
            GuideStage::Step { index: 1, total: 3 },
            Some(RoutineVariant::Seated),
        )
    });
    let m = menu_actions(&guided);
    assert_eq!(m[0].label, "Show movement guide");
    assert!(present::click_reopens(&guided));
    for r in results {
        assert!(ids(&m).contains(&r), "{r}");
    }
    assert!(!ids(&m).contains(&"movement_start"));
    views.push(guided);
    // The same under a hold that came after it (a pause or a meeting): the pet shows the
    // hold, the guide waits behind it.
    let mut held_guides = vec![view(PetMode::Paused {
        left: None,
        rest_of_today: false,
    })];
    held_guides[0].session = SessionState::Paused;
    let mut meeting_guide = view(PetMode::Meeting);
    meeting_guide.meeting = true;
    held_guides.push(meeting_guide);
    for mut v in held_guides {
        v.active_routine = manual;
        v.guide = Some(GuideView {
            open: false,
            ..guide(GuideStage::Intro, None)
        });
        let m = menu_actions(&v);
        assert_eq!(m[0].label, "Show movement guide", "{:?}", v.pet);
        for r in results {
            assert!(ids(&m).contains(&r), "{r} under {:?}", v.pet);
        }
        views.push(v);
    }
    // Hidden with the pet during a presentation: no card to bring back, the tray answers.
    let mut presenting = view(PetMode::Hidden(HiddenReason::Presentation));
    presenting.active_routine = manual;
    presenting.guide = Some(GuideView {
        open: false,
        ..guide(GuideStage::Intro, None)
    });
    assert!(!present::click_reopens(&presenting));
    views.push(presenting);

    // Deadline day and the recovery plan.
    let fresh = view(PetMode::Idle);
    assert!(ids(&menu_actions(&fresh)).contains(&"deadline_day"));
    let mut marked = view(PetMode::Idle);
    marked.recovery.marked_today = true;
    let m = menu_actions(&marked);
    assert!(
        m.iter()
            .any(|a| a.id == "deadline_day" && a.label == "Plan a recovery day")
    );
    views.push(marked);
    let mut planned = view(PetMode::Idle);
    planned.recovery.marked_today = true;
    planned.recovery.plan = Some(plan(22));
    let m = menu_actions(&planned);
    assert!(
        m.iter()
            .any(|a| a.id == "recovery_show" && a.label == "Recovery day: Thu 22 Oct")
    );
    assert!(!ids(&m).contains(&"deadline_day"));
    views.push(planned);
    let mut busy = view(PetMode::Idle);
    busy.recovery.can_open = false;
    assert!(!ids(&menu_actions(&busy)).contains(&"deadline_day"));
    // Eye rest running is not a movement routine.
    let mut eyes = view(PetMode::Exercise);
    eyes.chips[1] = chip(Activity::EyeRest, ChipStatus::InProgress);
    eyes.active_routine = Some(ActiveRoutine {
        activity: Activity::EyeRest,
        offer: Some(Uuid::from_u128(8)),
    });
    let m = menu_actions(&eyes);
    assert!(ids(&m).contains(&"movement_start"));
    assert!(!ids(&m).contains(&"movement_done"));

    let mut meeting = view(PetMode::Meeting);
    meeting.meeting = true;
    assert!(ids(&menu_actions(&meeting)).contains(&"meeting_off"));
    views.push(meeting);

    for v in &views {
        for a in menu_actions(v) {
            assert!(menu_command(&a.id).is_some(), "{} routes nowhere", a.id);
        }
    }
}

// ---------------------------------------------------------------- routing

#[test]
fn menu_entries_route_to_engine_inputs() {
    let coach = |i| Some(Command::Coach(i));
    assert_eq!(
        menu_command("pause_30"),
        coach(Input::Pause(PauseLength::Minutes(30)))
    );
    assert_eq!(
        menu_command("pause_today"),
        coach(Input::Pause(PauseLength::RestOfToday))
    );
    assert_eq!(
        menu_command("pause_on"),
        coach(Input::Pause(PauseLength::UntilResumed))
    );
    assert_eq!(menu_command("resume"), coach(Input::ResumeCoaching));
    for ml in [150, 250, 500] {
        assert_eq!(
            menu_command(&format!("water_{ml}")),
            Some(Command::LogWater { ml })
        );
    }
    assert_eq!(
        menu_command("water_panel"),
        coach(Input::Water(WaterInput::OpenPanel))
    );
    assert_eq!(menu_command("phone_note"), Some(Command::NotePhone));
    assert_eq!(
        menu_command("focus_60"),
        coach(Input::StartFocus {
            minutes: 60,
            replace_pomodoro: false
        })
    );
    assert_eq!(
        menu_command("focus_90_rp"),
        coach(Input::StartFocus {
            minutes: 90,
            replace_pomodoro: true
        })
    );
    assert_eq!(
        menu_command("pomodoro_25"),
        coach(Input::StartPomodoro {
            preset: PomodoroPreset::Classic,
            replace_focus: false,
            resume_coaching: false
        })
    );
    assert_eq!(
        menu_command("pomodoro_50_rf_rc"),
        coach(Input::StartPomodoro {
            preset: PomodoroPreset::Long,
            replace_focus: true,
            resume_coaching: true
        })
    );
    assert_eq!(menu_command("movement_start"), coach(Input::RoutineStarted));
    assert_eq!(menu_command("deadline_day"), coach(Input::DeadlineDay));
    assert_eq!(
        menu_command("recovery_show"),
        coach(Input::Recovery(RecoveryAction::ShowPlan))
    );
    assert_eq!(
        menu_command("movement_partial"),
        coach(Input::RoutineFinished {
            offer: None,
            outcome: RoutineOutcome::Partial
        })
    );
    assert_eq!(menu_command("meeting_on"), coach(Input::SetMeeting(true)));
    // Unknown, out-of-range or malformed entries route nowhere.
    for junk in [
        "",
        "focus_45",
        "focus_x",
        "focus_60_rp_rp",
        "focus_60_rf",
        "pomodoro_30",
        "pomodoro_25_zz",
        "pomodoro_25_rc_rc",
        "water_40",
        "water_",
        "water_2010",
        "water_255",
        "water_0250",
        "water_250x",
        "quit",
    ] {
        assert_eq!(menu_command(junk), None, "{junk}");
    }
}

#[test]
fn bubble_events_route_only_for_the_bubble_on_show() {
    let offer = Uuid::from_u128(5);
    let id = BubbleKind::Reminder { offer }.id(1);
    let state = RouteState {
        bubble_id: Some(id.clone()),
        click_reopens: false,
    };
    let act = |bubble_id: &str, action_id: &str| ShellEvent::BubbleAction {
        bubble_id: bubble_id.into(),
        action_id: action_id.into(),
    };
    let r = route(&state, &act(&id, "snooze"));
    assert_eq!(r.handled, Handled::Yes);
    assert_eq!(
        r.command,
        Some(Command::Answer {
            bubble_id: id.clone(),
            input: Input::Respond {
                offer,
                response: Response::Snooze
            }
        })
    );
    // An older copy of the card, or an unknown action, does nothing.
    let old = BubbleKind::Reminder { offer }.id(0);
    assert_eq!(route(&state, &act(&old, "done")).command, None);
    assert_eq!(route(&state, &act(&id, "launch")).command, None);
    // Closing: the close button collapses the reminder, a replacement is ignored, and a
    // bubble the engine already withdrew is ignored.
    let closed = |cause| ShellEvent::BubbleClosed {
        bubble_id: id.clone(),
        cause,
    };
    let close = Some(Command::Answer {
        bubble_id: id.clone(),
        input: Input::CloseBubble,
    });
    assert_eq!(route(&state, &closed(CloseCause::User)).command, close);
    assert_eq!(route(&state, &closed(CloseCause::Hidden)).command, close);
    assert_eq!(route(&state, &closed(CloseCause::Replaced)).command, None);
    let withdrawn = RouteState::default();
    assert_eq!(route(&withdrawn, &closed(CloseCause::Hidden)).command, None);
    assert_eq!(route(&withdrawn, &closed(CloseCause::User)).command, None);

    // Check-in and Pomodoro cards.
    let check_in = BubbleKind::CheckIn.id(2);
    let state = RouteState {
        bubble_id: Some(check_in.clone()),
        click_reopens: false,
    };
    assert_eq!(
        route(&state, &act(&check_in, "dont_ask")).command,
        Some(Command::Answer {
            bubble_id: check_in.clone(),
            input: Input::CheckIn(CheckInAnswer::DontAskToday)
        })
    );
}

#[test]
fn clicks_and_show_pet_route_to_the_engine() {
    let click = ShellEvent::PetClicked {
        state: PetState::Waiting,
    };
    let r = route(
        &RouteState {
            bubble_id: None,
            click_reopens: true,
        },
        &click,
    );
    assert_eq!(r.handled, Handled::Yes);
    assert_eq!(r.command, Some(Command::Coach(Input::OpenPending)));
    // Nothing to reopen: the shell opens the pet menu.
    let r = route(&RouteState::default(), &click);
    assert_eq!(r.handled, Handled::No);
    assert_eq!(r.command, None);
    assert_eq!(
        route(&RouteState::default(), &ShellEvent::PetShownByUser).command,
        Some(Command::Coach(Input::ShowPet))
    );
    assert_eq!(
        route(
            &RouteState::default(),
            &ShellEvent::MenuAction {
                action_id: "break".into()
            }
        )
        .command,
        Some(Command::Coach(Input::MarkBreak))
    );
}

// ------------------------------------------------------- the whole loop

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap()
}

/// The shell's rules without windows, behaving as the companion does: effects that tell
/// the engine something go to the registered sink, and an unhandled click opens the menu.
#[derive(Default)]
struct FakeShell {
    model: Mutex<ShellModel>,
    sink: Mutex<Option<Arc<dyn ShellEventSink>>>,
    /// Every bubble id shown, in order.
    shown: Mutex<Vec<String>>,
}

impl std::fmt::Debug for FakeShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeShell")
    }
}

impl FakeShell {
    fn apply(&self, fx: Vec<Effect>) {
        for e in fx {
            if let Effect::Emit(ev) = e {
                let sink = lock(&self.sink).clone();
                let handled = sink.map_or(Handled::No, |s| s.on_event(&ev));
                if matches!(ev, ShellEvent::PetClicked { .. }) && handled == Handled::No {
                    let fx = lock(&self.model).open_menu();
                    self.apply(fx);
                }
            }
        }
    }

    fn with(&self, f: impl FnOnce(&mut ShellModel) -> Vec<Effect>) {
        let fx = f(&mut lock(&self.model));
        self.apply(fx);
    }

    fn open(&self) -> Open {
        lock(&self.model).open().clone()
    }

    fn open_bubble(&self) -> Option<Bubble> {
        match self.open() {
            Open::Bubble { bubble } => Some(bubble),
            _ => None,
        }
    }

    fn visible(&self) -> bool {
        lock(&self.model).is_visible()
    }

    fn state(&self) -> (PetState, Option<String>) {
        let m = lock(&self.model);
        let (s, t) = m.state();
        (s, t.map(str::to_owned))
    }

    fn menu_ids(&self) -> Vec<String> {
        lock(&self.model)
            .menu_actions()
            .iter()
            .map(|a| a.id.clone())
            .collect()
    }

    fn tray_ids(&self) -> Vec<String> {
        lock(&self.model)
            .tray()
            .actions
            .into_iter()
            .map(|a| a.id)
            .collect()
    }

    // What the user does.
    fn press(&self, action_id: &str) {
        let b = self.open_bubble().expect("a bubble is open");
        self.with(|m| m.bubble_action(&b.id, action_id));
    }

    fn close_button(&self) {
        self.with(ShellModel::user_close);
    }

    fn click_pet(&self) {
        self.with(ShellModel::pet_clicked);
    }

    fn menu(&self, id: &str) {
        self.with(|m| m.menu_action(id));
    }

    fn user_hide(&self) {
        self.with(|m| m.hide(HideReason::User));
    }

    fn tray_show(&self) {
        self.with(ShellModel::user_show);
    }
}

impl PetShell for FakeShell {
    fn set_state(&self, state: PetState, tag: Option<String>) {
        self.with(|m| m.set_state(state, tag));
    }

    fn show_bubble(&self, bubble: Bubble) -> BubbleOutcome {
        let id = bubble.id.clone();
        let (outcome, fx) = lock(&self.model).show_bubble(bubble);
        if outcome == BubbleOutcome::Shown {
            lock(&self.shown).push(id);
        }
        self.apply(fx);
        outcome
    }

    fn close_bubble(&self, bubble_id: &str) {
        self.with(|m| m.close_bubble(bubble_id));
    }

    fn hide(&self, reason: HideReason) {
        self.with(|m| m.hide(reason));
    }

    fn unhide(&self, reason: HideReason) {
        self.with(|m| m.unhide(reason));
    }

    fn is_visible(&self) -> bool {
        self.visible()
    }

    fn set_menu_actions(&self, actions: Vec<MenuAction>) {
        self.with(|m| m.set_menu_actions(actions));
    }

    fn set_event_sink(&self, sink: Arc<dyn ShellEventSink>) {
        *lock(&self.sink) = Some(sink);
    }
}

/// Monday 26 Oct 2026, 10:00 in Kolkata (inside the default work hours).
fn monday_ten() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 26, 4, 30, 0).unwrap()
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()
}

struct H {
    clock: Arc<ManualClock>,
    shell: Arc<FakeShell>,
    link: CoachLink,
    queue: Arc<Mutex<Vec<Command>>>,
    /// Offers the engine withdrew or resolved; none may ever be shown again.
    finished: BTreeSet<Uuid>,
}

impl H {
    fn new() -> Self {
        Self::with_store(|_| {})
    }

    fn with_store(prepare: impl FnOnce(&Store)) -> Self {
        Self::starting(monday_ten(), prepare)
    }

    /// Start at `wall` (UTC).
    fn at(wall: chrono::DateTime<Utc>) -> Self {
        Self::starting(wall, |_| {})
    }

    fn starting(wall: chrono::DateTime<Utc>, prepare: impl FnOnce(&Store)) -> Self {
        let clock = Arc::new(ManualClock::new(wall));
        let store = Store::open_in_memory(clock.clone()).unwrap();
        prepare(&store);
        let shell = Arc::new(FakeShell::default());
        let (link, _) = CoachLink::open(store, shell.clone()).unwrap();
        let queue = Arc::new(Mutex::new(Vec::new()));
        let q = Arc::clone(&queue);
        shell.set_event_sink(Arc::new(LinkSink::new(link.router(), move |c| {
            lock(&q).push(c);
        })));
        let mut h = Self {
            clock,
            shell,
            link,
            queue,
            finished: BTreeSet::new(),
        };
        h.check();
        h
    }

    /// Apply everything the shell's sink forwarded (the companion's engine thread).
    fn pump(&mut self) {
        loop {
            let cmds = std::mem::take(&mut *lock(&self.queue));
            if cmds.is_empty() {
                break;
            }
            for c in cmds {
                self.link.command(c).unwrap();
            }
        }
        self.check();
    }

    fn input(&mut self, i: Input) {
        self.link.input(i).unwrap();
        self.pump();
    }

    /// Let `secs` seconds pass with the user active, ticking every 5 s.
    fn work(&mut self, secs: u64) {
        for _ in 0..secs / 5 {
            self.clock.advance(chrono::Duration::seconds(5));
            self.input(Input::Tick {
                idle: Duration::ZERO,
            });
        }
    }

    /// Wall and monotonic time pass with no tick (locked or asleep).
    fn pass(&mut self, secs: u64) {
        self.clock
            .advance(chrono::Duration::seconds(i64::try_from(secs).unwrap()));
    }

    /// Work until the engine shows a reminder; returns its offer and activity.
    fn work_until_reminder(&mut self, limit_min: u64) -> (Uuid, Activity) {
        for _ in 0..limit_min * 12 {
            self.work(5);
            if let Some(r) = self.link.view().reminder() {
                return r;
            }
        }
        panic!("no reminder within {limit_min} min");
    }

    /// Work time counted toward the movement reminder.
    fn movement_accum(&self) -> u64 {
        self.link
            .coach()
            .engine()
            .activity(Activity::Movement)
            .accum_ms
    }

    /// A movement routine runs: its three result entries, and not "Today's movement",
    /// are in the pet menu and keyboard panel (one list) and in the tray.
    fn assert_movement_results_reachable(&self) {
        assert!(
            self.link
                .view()
                .active_routine
                .is_some_and(|r| r.activity == Activity::Movement)
        );
        let menu = self.shell.menu_ids();
        let tray = self.shell.tray_ids();
        for r in [
            action::MOVEMENT_DONE,
            action::MOVEMENT_PARTIAL,
            action::MOVEMENT_STOP,
        ] {
            assert!(menu.iter().any(|m| m == r), "{r} not in the menu: {menu:?}");
            assert!(tray.iter().any(|m| m == r), "{r} not in the tray: {tray:?}");
        }
        assert!(!menu.iter().any(|m| m == action::MOVEMENT_START));
    }

    /// No routine runs: "Today's movement" is back and the results are gone.
    fn assert_no_movement_running(&self) {
        assert_eq!(self.link.view().active_routine, None);
        let menu = self.shell.menu_ids();
        assert!(menu.iter().any(|m| m == action::MOVEMENT_START));
        assert!(!menu.iter().any(|m| m == action::MOVEMENT_STOP));
        assert!(
            !self
                .shell
                .tray_ids()
                .iter()
                .any(|m| m == action::MOVEMENT_STOP)
        );
    }

    fn events(&self) -> Vec<EventPayload> {
        self.link
            .coach()
            .store()
            .events_on(today())
            .unwrap()
            .into_iter()
            .map(|e| e.payload)
            .collect()
    }

    fn offers(&self) -> Vec<Uuid> {
        self.events()
            .into_iter()
            .filter_map(|p| match p {
                EventPayload::ReminderOffered { reminder_id, .. } => Some(reminder_id),
                _ => None,
            })
            .collect()
    }

    fn responses(&self) -> Vec<(Uuid, ReminderOutcome)> {
        self.events()
            .into_iter()
            .filter_map(|p| match p {
                EventPayload::ReminderResponse {
                    reminder_id,
                    outcome,
                    ..
                } => Some((reminder_id, outcome)),
                _ => None,
            })
            .collect()
    }

    // What the user does, then the engine thread catches up.
    fn press(&mut self, action_id: &str) {
        self.shell.press(action_id);
        self.pump();
    }

    fn close_button(&mut self) {
        self.shell.close_button();
        self.pump();
    }

    fn click_pet(&mut self) {
        self.shell.click_pet();
        self.pump();
    }

    fn menu(&mut self, id: &str) {
        self.shell.menu(id);
        self.pump();
    }

    fn user_hide(&mut self) {
        self.shell.user_hide();
        self.pump();
    }

    fn tray_show(&mut self) {
        self.shell.tray_show();
        self.pump();
    }

    /// Press each of `action_ids` on the card now shown, all before the engine thread
    /// applies any of them (quick clicks), then let it catch up.
    fn press_quickly(&mut self, action_ids: &[&str]) {
        for a in action_ids {
            self.shell.press(a);
        }
        assert_eq!(
            lock(&self.queue).len(),
            action_ids.len(),
            "every click was routed from the card on show"
        );
        self.pump();
    }

    /// The commands routed from the shell and not yet applied, taken off the queue.
    fn take_queued(&self) -> Vec<Command> {
        std::mem::take(&mut *lock(&self.queue))
    }

    /// The `movement.step` events: (routine, movement index, outcome).
    fn movement_steps(&self) -> Vec<(Uuid, u32, StepOutcome)> {
        self.events()
            .into_iter()
            .filter_map(|p| match p {
                EventPayload::MovementStep {
                    session_id,
                    step_index,
                    outcome,
                    ..
                } => Some((session_id, step_index, outcome)),
                _ => None,
            })
            .collect()
    }

    /// The guide's stage.
    fn guide_stage(&self) -> Option<GuideStage> {
        self.link.view().guide.map(|g| g.stage)
    }

    /// The rules that hold after every step.
    fn check(&mut self) {
        let view = self.link.view().clone();
        let open = self.shell.open_bubble();
        // The shell's one bubble is always the one the link showed last.
        if let Some(b) = &open {
            assert_eq!(Some(b.id.as_str()), self.link.shown_bubble_id());
            // ...and it is the engine's current bubble.
            let kind = BubbleKind::parse(&b.id).unwrap();
            assert_eq!(view.bubble.as_ref().map(BubbleKind::of), Some(kind));
        }
        // While the engine hides the pet, it is hidden for a presentation with nothing
        // open, and the engine has no bubble.
        if !view.visible {
            assert!(view.bubble.is_none());
            assert!(
                lock(&self.shell.model)
                    .hidden_reasons()
                    .contains(&HideReason::Presentation)
            );
            assert_eq!(self.shell.open(), Open::None);
        }
        // A reminder that was answered, withdrawn or expired is never shown again.
        for (offer, _) in self.responses() {
            self.finished.insert(offer);
        }
        if let Some(b) = &open
            && let Some(BubbleKind::Reminder { offer }) = BubbleKind::parse(&b.id)
        {
            assert!(
                !self.finished.contains(&offer),
                "a finished reminder came back"
            );
        }
        // At most one reminder is waiting for an answer.
        let answered: BTreeSet<Uuid> = self.responses().into_iter().map(|(o, _)| o).collect();
        let open_offers = self
            .offers()
            .into_iter()
            .filter(|o| !answered.contains(o))
            .count();
        assert!(open_offers <= 1, "{open_offers} reminders open at once");
    }
}

#[test]
fn a_due_reminder_is_the_single_bubble_and_its_answer_is_recorded() {
    let mut h = H::new();
    assert_eq!(h.shell.state(), (PetState::Idle, None));
    assert!(h.shell.menu_ids().contains(&"pause_30".to_owned()));
    // Balanced: eye rest is due after 20 minutes of screen time.
    let (offer, activity) = h.work_until_reminder(25);
    assert_eq!(activity, Activity::EyeRest);
    let b = h.shell.open_bubble().expect("the reminder is on show");
    assert_eq!(b.title, "Eye rest: look away for 20 seconds");
    assert_eq!(
        BubbleKind::parse(&b.id),
        Some(BubbleKind::Reminder { offer })
    );
    assert_eq!(
        h.shell.state(),
        (PetState::Reminder, Some("Eye rest".to_owned()))
    );
    // A minute later the card is not redrawn (the same id stays).
    h.work(60);
    assert_eq!(h.shell.open_bubble().map(|x| x.id), Some(b.id.clone()));
    assert_eq!(lock(&h.shell.shown).len(), 1);
    // "Already did it" is recorded through the store, and the bubble closes.
    h.press(action::DONE);
    assert!(h.responses().contains(&(offer, ReminderOutcome::Done)));
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.shell.state().0, PetState::Celebration);
    // The celebration ends on the next tick after 3 s.
    h.work(5);
    assert_eq!(h.shell.state().0, PetState::Idle);
    // A second press on the old card does nothing.
    let before = h.responses().len();
    h.shell.with(|m| m.bubble_action(&b.id, action::SNOOZE));
    h.pump();
    assert_eq!(h.responses().len(), before);
}

#[test]
fn each_reminder_answer_reaches_the_engine() {
    for (act, outcome) in [
        (action::SNOOZE, ReminderOutcome::Snoozed),
        (action::DISMISS, ReminderOutcome::Dismissed),
        (action::START, ReminderOutcome::Started),
    ] {
        let mut h = H::new();
        let (offer, _) = h.work_until_reminder(25);
        h.press(act);
        assert!(h.responses().contains(&(offer, outcome)), "{act}");
        assert_eq!(h.shell.open(), Open::None);
    }
    // Snoozed: the pet says so.
    let mut h = H::new();
    h.work_until_reminder(25);
    h.press(action::SNOOZE);
    assert_eq!(
        h.shell.state(),
        (PetState::Snoozed, Some("Snoozed 10 min".to_owned()))
    );
}

#[test]
fn a_water_reminder_logs_water_from_the_card() {
    let mut h = H::with_store(|s| {
        s.set_goal(GoalKind::WaterDailyMl, 2000, today()).unwrap();
    });
    // Water falls behind pace before eye rest is due on this schedule? Work until a
    // water card shows (eye rest may come first; answer it to make room).
    let offer = loop {
        let (offer, activity) = h.work_until_reminder(120);
        if activity == Activity::Water {
            break offer;
        }
        h.press(action::DONE);
    };
    let b = h.shell.open_bubble().unwrap();
    assert!(b.title.starts_with("Water: "), "{}", b.title);
    h.press(action::LOG_WATER);
    assert!(h.responses().contains(&(offer, ReminderOutcome::Done)));
    assert!(
        h.events()
            .contains(&EventPayload::WaterLogged { amount_ml: 250 })
    );
}

#[test]
fn closing_the_card_collapses_it_and_a_click_reopens_it() {
    let mut h = H::new();
    let (offer, _) = h.work_until_reminder(25);
    h.close_button();
    // No answer is recorded; the pet shows the waiting badge.
    assert!(h.responses().is_empty());
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.shell.state().0, PetState::Waiting);
    assert_eq!(h.shell.menu_ids()[0], "open_pending");
    // A click on the pet asks the engine to reopen the same reminder.
    h.click_pet();
    let b = h.shell.open_bubble().expect("reopened");
    assert_eq!(
        BubbleKind::parse(&b.id),
        Some(BubbleKind::Reminder { offer })
    );
    // A click while it is open closes it again (collapse, not an answer).
    h.click_pet();
    assert_eq!(h.shell.open(), Open::None);
    assert!(h.responses().is_empty());
    // With nothing waiting, a click opens the pet menu.
    let mut h = H::new();
    h.click_pet();
    assert_eq!(h.shell.open(), Open::Menu);
}

#[test]
fn a_presentation_hides_the_bubble_and_nothing_is_replayed() {
    let mut h = H::new();
    let (offer, _) = h.work_until_reminder(25);
    let shown_before = lock(&h.shell.shown).len();
    // Presentation detected: the pet and its bubble hide, the reminder is withdrawn.
    h.input(Input::PresentationChanged(true));
    assert!(!h.shell.visible());
    assert_eq!(h.shell.open(), Open::None);
    assert!(h.link.hidden_for_presentation());
    assert!(
        h.responses()
            .contains(&(offer, ReminderOutcome::NoResponse))
    );
    // Ten minutes of presenting: nothing shows.
    h.work(10 * 60);
    assert!(!h.shell.visible());
    assert_eq!(lock(&h.shell.shown).len(), shown_before);
    // The presentation ends: the pet comes back quietly, with no bubble.
    h.input(Input::PresentationChanged(false));
    assert!(h.shell.visible());
    assert_eq!(h.shell.open(), Open::None);
    assert!(!h.link.hidden_for_presentation());
    let offers_at_end = h.offers().len();
    // No burst afterwards: nothing within the calm gap.
    h.work(4 * 60);
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.offers().len(), offers_at_end);
    assert_eq!(lock(&h.shell.shown).len(), shown_before);
}

#[test]
fn the_user_hiding_the_pet_collapses_the_reminder_and_nothing_pops_up() {
    let mut h = H::new();
    h.work_until_reminder(25);
    h.user_hide();
    // Collapsed to the badge, not answered.
    assert!(h.responses().is_empty());
    assert_eq!(h.link.shown_bubble_id(), None);
    // Shown again: the badge, no bubble.
    h.tray_show();
    assert!(h.shell.visible());
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.shell.state().0, PetState::Waiting);
}

#[test]
fn a_reminder_due_while_the_user_hid_the_pet_waits_as_a_badge() {
    let mut h = H::new();
    h.user_hide();
    // The shell refuses the reminder, so it collapses to the badge at once.
    for _ in 0..25 * 12 {
        h.work(5);
        if !h.offers().is_empty() {
            break;
        }
    }
    let offer = h.offers()[0];
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.link.shown_bubble_id(), None);
    assert_eq!(h.link.view().pet, PetMode::Waiting(Activity::EyeRest));
    assert!(h.responses().is_empty());
    h.tray_show();
    assert_eq!(h.shell.open(), Open::None);
    h.click_pet();
    assert_eq!(
        h.shell.open_bubble().and_then(|b| BubbleKind::parse(&b.id)),
        Some(BubbleKind::Reminder { offer })
    );
}

#[test]
fn tray_show_pet_overrides_a_presentation_until_it_ends() {
    let mut h = H::new();
    h.input(Input::PresentationChanged(true));
    assert!(!h.shell.visible());
    h.tray_show();
    // The engine was told, so the link does not hide the pet again.
    assert!(h.link.view().visible);
    assert!(h.shell.visible());
    h.work(60);
    assert!(h.shell.visible());
    // The next presentation hides it again.
    h.input(Input::PresentationChanged(false));
    h.input(Input::PresentationChanged(true));
    assert!(!h.shell.visible());
}

#[test]
fn lock_and_unlock_withdraw_and_give_a_grace_period() {
    let mut h = H::new();
    let (offer, _) = h.work_until_reminder(25);
    h.input(Input::Lock);
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.shell.state(), (PetState::Quiet, Some("Away".to_owned())));
    assert!(
        h.responses()
            .contains(&(offer, ReminderOutcome::NoResponse))
    );
    // Three minutes locked: a confirmed break; after unlock a 2-minute grace, no burst.
    h.pass(180);
    h.input(Input::Unlock);
    assert_eq!(h.shell.state().0, PetState::Idle);
    let offers = h.offers().len();
    h.work(115);
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.offers().len(), offers);
}

#[test]
fn sleep_and_wake_with_lock_signals_in_windows_order() {
    let mut h = H::new();
    h.work(10 * 60);
    // Windows: lock, then suspend; on wake, resume, then unlock. Two hours asleep.
    h.input(Input::Lock);
    h.input(Input::Suspend);
    h.pass(2 * 3600);
    h.input(Input::Resume);
    assert_eq!(h.shell.state().0, PetState::Quiet);
    // A repeated resume (both resume broadcasts arrive) changes nothing.
    h.input(Input::Resume);
    h.input(Input::Unlock);
    assert_eq!(h.shell.state().0, PetState::Idle);
    // No burst after waking: the timers restarted with the confirmed break.
    h.work(2 * 60);
    assert_eq!(h.shell.open(), Open::None);
    assert!(h.offers().is_empty());
}

#[test]
fn a_pomodoro_paused_by_a_lock_offers_its_card_on_a_click() {
    let mut h = H::new();
    h.menu("pomodoro_25");
    assert_eq!(h.shell.state().0, PetState::Pomodoro);
    assert_eq!(h.shell.state().1.as_deref(), Some("Work 25:00"));
    h.work(60);
    assert_eq!(h.shell.state().1.as_deref(), Some("Work 24:00"));
    h.input(Input::Lock);
    h.pass(60);
    h.input(Input::Unlock);
    // Back from the lock: "Pomodoro paused. Resume or End." (no catch-up).
    h.work(10);
    let b = h.shell.open_bubble().expect("the paused card");
    assert_eq!(BubbleKind::parse(&b.id), Some(BubbleKind::PomodoroPaused));
    // Closed without an answer, a click on the pet brings it back.
    h.close_button();
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.shell.menu_ids()[0], "open_pending");
    h.click_pet();
    let b = h.shell.open_bubble().expect("the paused card again");
    assert_eq!(BubbleKind::parse(&b.id), Some(BubbleKind::PomodoroPaused));
    h.press(action::RESUME_POMODORO);
    assert_eq!(h.shell.open(), Open::None);
    assert!(
        h.link
            .view()
            .pomodoro
            .is_some_and(|p| p.paused.is_none() && p.phase == PomodoroPhase::Work)
    );
    h.menu("pomodoro_end");
    assert!(h.link.view().pomodoro.is_none());
}

#[test]
fn menu_and_tray_entries_change_the_engine() {
    let mut h = H::new();
    // Pause from the tray: no reminders, the pet says so, resume comes back.
    h.menu("pause_60");
    assert_eq!(
        h.shell.state(),
        (PetState::Paused, Some("Paused 1 h".to_owned()))
    );
    assert!(h.shell.menu_ids().contains(&"resume".to_owned()));
    h.work(30 * 60);
    assert_eq!(h.shell.open(), Open::None);
    assert!(h.offers().is_empty());
    h.menu("resume");
    assert_eq!(h.shell.state().0, PetState::Idle);
    // Focus, then a Pomodoro that says it ends focus.
    h.menu("focus_30");
    assert_eq!(
        h.shell.state(),
        (PetState::Focus, Some("Focus 30 min".to_owned()))
    );
    assert!(h.shell.menu_ids().contains(&"pomodoro_25_rf".to_owned()));
    h.menu("pomodoro_25_rf");
    assert_eq!(h.shell.state().0, PetState::Pomodoro);
    assert!(h.link.view().focus_left.is_none());
    h.menu("pomodoro_end");
    // Meeting on and off.
    h.menu("meeting_on");
    assert_eq!(h.shell.state().0, PetState::Meeting);
    h.menu("meeting_off");
    assert_eq!(h.shell.state().0, PetState::Idle);
    // Water from the menu is logged.
    h.menu("water_250");
    assert!(
        h.events()
            .contains(&EventPayload::WaterLogged { amount_ml: 250 })
    );
    // Movement from the menu opens the guide.
    h.menu("movement_start");
    assert_eq!(h.shell.state().0, PetState::Exercise);
    assert!(!h.shell.menu_ids().contains(&"movement_start".to_owned()));
    assert!(h.link.view().guide.is_some());
    // An entry the menu does not list is ignored by the shell.
    h.menu("movement_done");
}

fn movement_only(store: &Store) {
    let config = hy_core::coach::CoachConfig {
        categories: hy_core::coach::config::Categories {
            movement: true,
            eye_rest: false,
            water: false,
        },
        ..hy_core::coach::CoachConfig::default()
    };
    store
        .put_setting(hy_core::coach::config::CONFIG_KEY, &config)
        .unwrap();
}

#[test]
fn a_guided_routine_runs_end_to_end_through_the_shell() {
    let mut h = H::with_store(movement_only);
    let (_, a) = h.work_until_reminder(50);
    assert_eq!(a, Activity::Movement);
    h.press(action::START);
    // Monday's routine, first screen.
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Today's movement: Monday");
    assert_eq!(b.note.as_deref(), Some(PROVISIONAL_NOTICE));
    assert!(b.continues);
    h.press(action::SEATED);
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Movement 1 of 3: Neck side stretch");
    assert_eq!(b.steps.len(), 4);
    assert_eq!(b.figure.as_ref().unwrap().id, "neck");
    assert_eq!(b.note.as_deref(), Some(PROVISIONAL_NOTICE));
    let first_id = b.id.clone();
    h.press(action::STEP_DONE);
    // The next movement is a new bubble (redrawn, numbered afresh).
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Movement 2 of 3: Shoulder rolls");
    assert_ne!(b.id, first_id);
    // Closing puts the routine behind the pet; a click brings it back.
    h.close_button();
    assert!(h.shell.open_bubble().is_none());
    assert_eq!(
        h.shell.menu_ids().first().map(String::as_str),
        Some("open_pending")
    );
    h.click_pet();
    assert_eq!(
        h.shell.open_bubble().unwrap().title,
        "Movement 2 of 3: Shoulder rolls"
    );
    h.press(action::STEP_SKIP);
    h.press(action::STEP_DONE);
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Saved as partial");
    assert!(!b.continues);
    h.press(action::OK);
    assert!(h.shell.open_bubble().is_none());
    let steps: Vec<_> = h
        .events()
        .into_iter()
        .filter_map(|p| match p {
            EventPayload::MovementStep {
                step_index,
                outcome,
                ..
            } => Some((step_index, outcome)),
            _ => None,
        })
        .collect();
    assert_eq!(
        steps,
        vec![
            (0, StepOutcome::Done),
            (1, StepOutcome::Skipped),
            (2, StepOutcome::Done)
        ]
    );
    assert!(h.events().iter().any(|p| matches!(
        p,
        EventPayload::MovementSession {
            outcome: RoutineOutcome::Partial,
            steps_done: 2,
            steps_skipped: 1,
            ..
        }
    )));
    assert_eq!(
        h.link.view().chip(Activity::Movement).status,
        ChipStatus::Partial
    );
}

#[test]
fn a_guided_routine_under_a_new_hold_waits_behind_the_pet_and_is_answered_from_the_tray() {
    let mut h = H::with_store(movement_only);
    let (_, a) = h.work_until_reminder(50);
    assert_eq!(a, Activity::Movement);
    h.press(action::START);
    h.press(action::STANDING);
    h.press(action::STEP_DONE);
    // A pause starts: the pet shows it, the guide goes behind the pet, nothing is lost.
    h.menu("pause_on");
    assert_eq!(h.shell.state().0, PetState::Paused);
    assert!(h.shell.open_bubble().is_none());
    h.assert_movement_results_reachable();
    assert_eq!(
        h.shell.menu_ids().first().map(String::as_str),
        Some(action::OPEN_PENDING)
    );
    // A click brings it back at the same movement, still paused.
    h.click_pet();
    assert_eq!(
        h.shell.open_bubble().unwrap().title,
        "Movement 2 of 3: Shoulder rolls"
    );
    assert_eq!(h.shell.state().0, PetState::Paused);
    // A presentation hides the pet; the tray answers the routine in one go.
    h.input(Input::PresentationChanged(true));
    assert!(!h.shell.visible());
    assert!(
        h.shell
            .tray_ids()
            .iter()
            .any(|m| m == action::MOVEMENT_PARTIAL)
    );
    h.menu(action::MOVEMENT_PARTIAL);
    h.assert_no_movement_running();
    assert!(h.link.view().guide.is_none());
    assert!(h.events().iter().any(|p| matches!(
        p,
        EventPayload::MovementSession {
            outcome: RoutineOutcome::Partial,
            variant: Some(RoutineVariant::Standing),
            steps_done: 1,
            steps_skipped: 0,
            ..
        }
    )));
    assert_eq!(h.movement_accum(), 0);
}

#[test]
fn deadline_day_and_a_recovery_plan_run_through_the_shell() {
    let mut h = H::with_store(movement_only);
    h.menu("deadline_day");
    assert_eq!(h.shell.open_bubble().unwrap().title, "Deadline day?");
    h.press(action::CHECK_IN_YES);
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Plan a gentler recovery day?");
    assert_eq!(b.actions[0].label, "Tomorrow (Tue 27 Oct)");
    assert_eq!(b.actions[0].id, "plan_2026-10-27");
    h.press(action::CHOOSE_DATE);
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(
        b.actions
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Wed 28 Oct",
            "Thu 29 Oct",
            "Fri 30 Oct",
            "Sat 31 Oct",
            "Later"
        ]
    );
    h.press("plan_2026-10-29");
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Recovery day planned: Thu 29 Oct");
    h.press(action::OK);
    assert!(h.shell.open_bubble().is_none());
    assert!(h.shell.menu_ids().contains(&"recovery_show".to_owned()));
    let kinds: Vec<_> = h
        .events()
        .into_iter()
        .map(|p| p.type_name())
        .filter(|t| t.starts_with("recovery."))
        .collect();
    assert_eq!(kinds, vec!["recovery.check_in", "recovery.plan"]);
    // The menu shows the plan again, and it can be cancelled there.
    h.menu("recovery_show");
    h.press(action::CANCEL_PLAN);
    assert!(h.shell.open_bubble().is_none());
    assert!(h.link.view().recovery.plan.is_none());
}

// ---------------------------------- answers queued before the engine applies them

#[test]
fn two_quick_clicks_on_one_movement_answer_it_once() {
    let mut h = H::with_store(movement_only);
    h.menu(action::MOVEMENT_START);
    h.press(action::SEATED);
    assert_eq!(
        h.guide_stage(),
        Some(GuideStage::Step { index: 0, total: 3 })
    );
    // "Done" twice on the first movement, both routed before either is applied.
    h.press_quickly(&[action::STEP_DONE, action::STEP_DONE]);
    let steps = h.movement_steps();
    assert_eq!(steps.len(), 1, "one movement shown, one answer: {steps:?}");
    assert_eq!((steps[0].1, steps[0].2), (0, StepOutcome::Done));
    assert_eq!(
        h.guide_stage(),
        Some(GuideStage::Step { index: 1, total: 3 })
    );
    assert_eq!(
        h.shell.open_bubble().unwrap().title,
        "Movement 2 of 3: Shoulder rolls"
    );
    // "Done" then "Skip this movement" on the second: only the first click counts.
    h.press_quickly(&[action::STEP_DONE, action::STEP_SKIP]);
    let steps: Vec<_> = h.movement_steps().into_iter().map(|s| (s.1, s.2)).collect();
    assert_eq!(steps, vec![(0, StepOutcome::Done), (1, StepOutcome::Done)]);
    assert_eq!(
        h.guide_stage(),
        Some(GuideStage::Step { index: 2, total: 3 })
    );
    // The last movement, clicked twice: one answer, then the result.
    h.press_quickly(&[action::STEP_SKIP, action::STEP_SKIP]);
    assert_eq!(h.movement_steps().len(), 3);
    assert!(matches!(
        h.guide_stage(),
        Some(GuideStage::Result {
            done: 2,
            skipped: 1,
            ..
        })
    ));
}

#[test]
fn an_answer_queued_before_a_routine_stopped_does_not_reach_the_next_one() {
    let mut h = H::with_store(movement_only);
    h.menu(action::MOVEMENT_START);
    h.press(action::SEATED);
    // "Done" on the first movement is routed but not yet applied...
    h.shell.press(action::STEP_DONE);
    let stale = h.take_queued();
    assert_eq!(stale.len(), 1);
    // ...while the routine is stopped from the tray and a new one is started.
    h.menu(action::MOVEMENT_STOP);
    h.assert_no_movement_running();
    if h.link.view().guide.is_some() {
        h.press(action::OK);
    }
    h.menu(action::MOVEMENT_START);
    h.press(action::SEATED);
    assert_eq!(
        h.guide_stage(),
        Some(GuideStage::Step { index: 0, total: 3 })
    );
    // The old answer arrives now: it belonged to the stopped routine, so it is refused.
    for c in stale {
        let out = h.link.command(c).unwrap();
        assert!(
            matches!(out.outcome, hy_core::coach::InputOutcome::Ignored(_)),
            "{:?}",
            out.outcome
        );
    }
    h.check();
    assert_eq!(h.movement_steps(), vec![]);
    assert_eq!(
        h.guide_stage(),
        Some(GuideStage::Step { index: 0, total: 3 })
    );
    // The new routine's own first movement still answers normally.
    h.press(action::STEP_DONE);
    assert_eq!(h.movement_steps().len(), 1);
    assert_eq!(
        h.guide_stage(),
        Some(GuideStage::Step { index: 1, total: 3 })
    );
}

#[test]
fn two_quick_answers_on_one_recovery_card_apply_only_the_first() {
    let mut h = H::with_store(movement_only);
    h.menu("deadline_day");
    // "Yes" twice: one check-in answer, then the invitation.
    h.press_quickly(&[action::CHECK_IN_YES, action::CHECK_IN_YES]);
    assert_eq!(
        h.shell.open_bubble().unwrap().title,
        "Plan a gentler recovery day?"
    );
    h.press("plan_2026-10-27");
    assert_eq!(
        h.shell.open_bubble().unwrap().title,
        "Recovery day planned: Tue 27 Oct"
    );
    // "OK" then "Cancel plan" on the same card: the card was answered by "OK", so the
    // second click is refused and the plan stays.
    h.press_quickly(&[action::OK, action::CANCEL_PLAN]);
    assert!(h.shell.open_bubble().is_none());
    assert!(h.link.view().recovery.plan.is_some());
    let kinds: Vec<_> = h
        .events()
        .into_iter()
        .map(|p| p.type_name())
        .filter(|t| t.starts_with("recovery."))
        .collect();
    assert_eq!(kinds, vec!["recovery.check_in", "recovery.plan"]);
}

#[test]
fn two_quick_answers_on_one_reminder_record_one_answer() {
    let mut h = H::new();
    let (offer, _) = h.work_until_reminder(50);
    h.press_quickly(&[action::DONE, action::SNOOZE]);
    assert_eq!(h.responses(), vec![(offer, ReminderOutcome::Done)]);
}

#[test]
fn guide_and_recovery_actions_route_only_from_their_own_cards() {
    let g = BubbleKind::MovementGuide;
    let coach = Some;
    assert_eq!(
        bubble_command(g, "seated"),
        coach(Input::Guide(GuideAction::Variant(RoutineVariant::Seated)))
    );
    assert_eq!(
        bubble_command(g, "day_sat"),
        coach(Input::Guide(GuideAction::Day(Weekday::Sat)))
    );
    assert_eq!(bubble_command(g, "day_sun"), None);
    assert_eq!(bubble_command(g, "plan_2026-10-27"), None);
    assert_eq!(
        bubble_command(BubbleKind::RecoveryDates, "plan_2026-10-27"),
        coach(Input::Recovery(RecoveryAction::PlanFor(oct(27))))
    );
    assert_eq!(
        bubble_command(BubbleKind::RecoveryDates, "plan_2026-13-01"),
        None
    );
    assert_eq!(bubble_command(BubbleKind::RecoveryDates, "no_thanks"), None);
    assert_eq!(
        bubble_command(BubbleKind::RecoveryReflection, "reflect_skip"),
        coach(Input::Recovery(RecoveryAction::Reflect(Reflection::Skip)))
    );
    assert_eq!(
        bubble_command(BubbleKind::DeadlineDay, "dont_ask"),
        None,
        "the menu's question has no \"Don't ask today\""
    );
    assert_eq!(
        bubble_command(BubbleKind::DeadlineDay, "yes"),
        coach(Input::CheckIn(CheckInAnswer::Yes))
    );
}

// ------------------------------------------- movement results under holds

/// Monday 26 Oct 2026, 18:10 in Kolkata (work hours end at 18:30).
fn monday_evening() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 26, 12, 40, 0).unwrap()
}

#[test]
fn movement_started_under_a_pause_or_meeting_keeps_its_results_reachable() {
    for (hold, end, look) in [
        ("pause_on", "resume", PetState::Paused),
        ("meeting_on", "meeting_off", PetState::Meeting),
    ] {
        let mut h = H::new();
        h.work(10 * 60);
        h.menu(hold);
        let held = h.shell.state();
        assert_eq!(held.0, look);
        // Today's movement under the hold: the pet keeps showing the hold, and the
        // results replace "Today's movement" in the menu, panel and tray.
        h.menu(action::MOVEMENT_START);
        assert_eq!(h.shell.state(), held, "{hold}");
        h.assert_movement_results_reachable();
        // The guide the user opened stays for this hold; a click on the pet puts it
        // behind the pet, and the menu (the review's repro) still offers the results,
        // with "Show movement guide" to bring it back.
        assert_eq!(
            h.shell.open_bubble().map(|b| b.title),
            Some("Today's movement: Monday".to_owned())
        );
        h.click_pet();
        assert_eq!(h.shell.open(), Open::None);
        h.work(60);
        h.assert_movement_results_reachable();
        assert_eq!(
            h.shell.menu_ids().first().map(String::as_str),
            Some(action::OPEN_PENDING)
        );
        // Stopped, none done: the routine ends, the movement timer is not restarted,
        // nothing is marked done, and the hold stays.
        let before = h.movement_accum();
        assert!(before > 0);
        h.menu(action::MOVEMENT_STOP);
        h.assert_no_movement_running();
        assert_eq!(h.movement_accum(), before, "{hold}");
        assert_eq!(
            h.link.view().chip(Activity::Movement).status,
            ChipStatus::NotDue
        );
        assert_eq!(h.shell.state(), held);
        // It can be started again under the same hold; done restarts the timer.
        h.menu(action::MOVEMENT_START);
        h.assert_movement_results_reachable();
        h.menu(action::MOVEMENT_DONE);
        h.assert_no_movement_running();
        assert_eq!(h.movement_accum(), 0);
        assert_eq!(
            h.link.view().chip(Activity::Movement).status,
            ChipStatus::Done
        );
        assert_eq!(h.shell.state(), held);
        // The hold was never lifted to get there.
        assert!(h.shell.menu_ids().iter().any(|m| m == end), "{hold}");
    }
}

#[test]
fn a_pause_meeting_or_focus_over_a_running_movement_keeps_its_results_reachable() {
    for (hold, look, answer) in [
        ("pause_30", PetState::Paused, action::MOVEMENT_PARTIAL),
        ("pause_on", PetState::Paused, action::MOVEMENT_STOP),
        ("meeting_on", PetState::Meeting, action::MOVEMENT_DONE),
        ("focus_30", PetState::Focus, action::MOVEMENT_STOP),
    ] {
        let mut h = H::new();
        h.work(10 * 60);
        h.menu(action::MOVEMENT_START);
        assert_eq!(h.shell.state().0, PetState::Exercise);
        h.assert_movement_results_reachable();
        h.menu(hold);
        let held = h.shell.state();
        assert_eq!(held.0, look, "{hold}");
        h.work(60);
        h.assert_movement_results_reachable();
        let before = h.movement_accum();
        assert!(before > 0);
        h.menu(answer);
        h.assert_no_movement_running();
        let chip = h.link.view().chip(Activity::Movement).status;
        match answer {
            action::MOVEMENT_DONE => {
                assert_eq!(h.movement_accum(), 0);
                assert_eq!(chip, ChipStatus::Done);
            }
            action::MOVEMENT_PARTIAL => {
                assert_eq!(h.movement_accum(), 0);
                assert_eq!(chip, ChipStatus::Partial);
            }
            _ => {
                // None does not reset the movement timer.
                assert_eq!(h.movement_accum(), before, "{hold}");
                assert_eq!(chip, ChipStatus::NotDue);
            }
        }
        // Still held: the answer did not lift the hold.
        assert_eq!(h.shell.state().0, look, "{hold}");
    }
}

#[test]
fn off_hours_keep_movement_results_reachable() {
    let mut h = H::at(monday_evening());
    h.work(10 * 60); // 18:20
    h.menu(action::MOVEMENT_START);
    assert_eq!(h.shell.state().0, PetState::Exercise);
    // Work hours end while it runs: the pet goes quiet, the results stay.
    h.work(12 * 60); // 18:32
    assert_eq!(h.link.view().pet, PetMode::OffHours);
    assert_eq!(h.shell.state(), (PetState::Quiet, None));
    h.assert_movement_results_reachable();
    let before = h.movement_accum();
    assert!(before > 0);
    h.menu(action::MOVEMENT_STOP);
    h.assert_no_movement_running();
    assert_eq!(h.movement_accum(), before);
    assert_eq!(h.link.view().pet, PetMode::OffHours);
    // Started during off-hours: reachable too, and partial restarts the timer.
    h.menu(action::MOVEMENT_START);
    assert_eq!(h.shell.state(), (PetState::Quiet, None));
    h.assert_movement_results_reachable();
    h.menu(action::MOVEMENT_PARTIAL);
    h.assert_no_movement_running();
    assert_eq!(h.movement_accum(), 0);
    assert_eq!(
        h.link.view().chip(Activity::Movement).status,
        ChipStatus::Partial
    );
    assert_eq!(h.link.view().pet, PetMode::OffHours);
}

#[test]
fn a_presentation_keeps_movement_results_in_the_tray() {
    let mut h = H::new();
    h.work(10 * 60);
    h.menu(action::MOVEMENT_START);
    h.input(Input::PresentationChanged(true));
    // The pet is hidden; the tray still offers the results.
    assert!(!h.shell.visible());
    h.assert_movement_results_reachable();
    h.menu(action::MOVEMENT_DONE);
    h.assert_no_movement_running();
    assert_eq!(h.movement_accum(), 0);
    assert!(!h.shell.visible());
}

#[test]
fn a_reminder_routine_under_a_later_hold_keeps_its_results_reachable() {
    let mut h = H::new();
    // Answer eye rest until movement is offered, then start it from the card.
    let offer = loop {
        let (offer, activity) = h.work_until_reminder(60);
        if activity == Activity::Movement {
            break offer;
        }
        h.press(action::DONE);
    };
    h.press(action::START);
    assert_eq!(
        h.link.view().active_routine.and_then(|r| r.offer),
        Some(offer)
    );
    h.menu("pause_on");
    assert_eq!(h.shell.state().0, PetState::Paused);
    h.assert_movement_results_reachable();
    let before = h.movement_accum();
    h.menu(action::MOVEMENT_STOP);
    h.assert_no_movement_running();
    // None from a reminder: recorded as none, the timer is not restarted.
    assert_eq!(h.movement_accum(), before);
    assert_eq!(
        h.link.view().chip(Activity::Movement).status,
        ChipStatus::NoneRecorded
    );
    assert_eq!(h.shell.state().0, PetState::Paused);
}

// ------------------------------------------- water and phone cards (B3, B8)

#[test]
fn water_cards_say_what_was_logged_against_the_users_goal() {
    let id = entry_id(1);
    let logged = CoachBubble::WaterLogged {
        entry: Some(id),
        ml: 250,
    };
    let mut v = with_water(&[500, 250]);
    let card = present::bubble(&logged, &v);
    assert_eq!(card.title, "Logged 250 ml, 750 / 2000 ml");
    assert!(card.lines.is_empty());
    let labels: Vec<&str> = card.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["Undo", "Details"]);
    assert!(card.continues);
    // Reaching the goal: reminders pause, logging stays; no praise past it.
    let full = with_water(&[1000, 1000]);
    let card = present::bubble(&logged, &full);
    assert_eq!(card.title, "Logged 250 ml, 2000 / 2000 ml");
    assert_eq!(card.lines, [present::GOAL_REACHED]);
    // No goal: the total alone.
    v.water = None;
    assert_eq!(
        present::bubble(&logged, &v).title,
        "Logged 250 ml, 750 ml today"
    );
    // After Undo.
    let undone = CoachBubble::WaterLogged {
        entry: None,
        ml: 250,
    };
    let card = present::bubble(&undone, &with_water(&[500]));
    assert_eq!(card.title, "Removed 250 ml");
    assert_eq!(card.lines, ["Today: 500 / 2000 ml (your goal)."]);
    let ids: Vec<&str> = card.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["ok", "details"]);
}

#[test]
fn the_water_panel_lists_todays_entries_with_edit_and_remove() {
    let panel = |edit, other_ml| CoachBubble::WaterPanel { edit, other_ml };
    // Empty: nothing logged is not zero water.
    let v = with_water(&[]);
    let card = present::bubble(&panel(None, 250), &v);
    assert_eq!(card.title, "Water today");
    assert_eq!(
        card.lines,
        [
            "0 / 2000 ml (your goal)",
            "No entries yet today. That means nothing logged, not zero water."
        ]
    );
    let p = card.progress.as_ref().unwrap();
    assert_eq!(
        (p.value, p.max, p.label.as_str()),
        (0, 2000, "0 of 2000 ml")
    );
    assert_eq!(card.rows.len(), 1);
    assert_eq!(card.rows[0].text, "Other amount: 250 ml");
    let labels: Vec<&str> = card.rows[0]
        .actions
        .iter()
        .map(|a| a.label.as_str())
        .collect();
    assert_eq!(labels, ["\u{2212}50 ml", "+50 ml", "Add 250 ml"]);
    let servings: Vec<(&str, &str)> = card
        .actions
        .iter()
        .map(|a| (a.id.as_str(), a.label.as_str()))
        .collect();
    assert_eq!(
        servings,
        [
            ("add_150", "+150 ml"),
            ("add_250", "+250 ml"),
            ("add_500", "+500 ml")
        ]
    );
    // Entries, one being edited.
    let v = with_water(&[250, 300]);
    let card = present::bubble(
        &panel(
            Some(WaterEdit {
                entry: entry_id(2),
                ml: 350,
            }),
            400,
        ),
        &v,
    );
    let rows: Vec<&str> = card.rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        rows,
        [
            "09:00, 250 ml",
            "09:30, 300 ml, change to 350 ml",
            "Other amount: 400 ml"
        ]
    );
    let first: Vec<String> = card.rows[0].actions.iter().map(|a| a.id.clone()).collect();
    assert_eq!(
        first,
        [
            format!("entry_edit_{}", entry_id(1)),
            format!("entry_remove_{}", entry_id(1))
        ]
    );
    let editing: Vec<&str> = card.rows[1].actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        editing,
        ["edit_less", "edit_more", "edit_save", "edit_cancel"]
    );
    // Many entries: the latest twelve, and a line saying so.
    let many: Vec<u32> = (1..=15).map(|i| i * 10).collect();
    let card = present::bubble(&panel(None, 250), &with_water(&many));
    assert_eq!(card.rows.len(), present::PANEL_ENTRIES + 1);
    assert!(
        card.rows[0].text.ends_with(", 40 ml"),
        "{}",
        card.rows[0].text
    );
    assert!(
        card.lines
            .contains(&"Showing the latest 12 of 15 entries.".to_owned())
    );
    // No goal: no progress bar, the total alone.
    let mut v = with_water(&[250]);
    v.water = None;
    let card = present::bubble(&panel(None, 250), &v);
    assert!(card.progress.is_none());
    assert_eq!(card.lines[0], "250 ml today (no goal set)");
}

#[test]
fn the_phone_note_offers_an_optional_label_and_undo() {
    let at = NaiveTime::from_hms_opt(11, 52, 0).unwrap();
    let note = |entry, label| CoachBubble::PhoneNote { entry, at, label };
    let v = view(PetMode::Idle);
    let card = present::bubble(&note(Some(entry_id(1)), None), &v);
    assert_eq!(card.title, "Phone interruption noted, 11:52");
    assert_eq!(card.lines, ["Optional: what was it?"]);
    let labels: Vec<&str> = card.actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["Work", "Personal", "Unsure", "Undo"]);
    assert_eq!(
        card.note.as_deref(),
        Some("Self-reported. The app does not detect phone use.")
    );
    let card = present::bubble(&note(Some(entry_id(1)), Some(PhoneLabel::Personal)), &v);
    assert_eq!(card.lines, ["Saved as personal."]);
    let primary: Vec<&str> = card
        .actions
        .iter()
        .filter(|a| a.style == crate::shell::ActionStyle::Primary)
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(primary, ["label_personal"]);
    let card = present::bubble(&note(None, None), &v);
    assert_eq!(card.title, "Phone note removed");
}

#[test]
fn water_and_phone_actions_route_only_from_their_own_cards() {
    let id = entry_id(0xab);
    let w = BubbleKind::WaterLogged;
    let p = BubbleKind::WaterPanel;
    let n = BubbleKind::PhoneNote;
    assert_eq!(bubble_change(w, "undo"), Some(EntryChange::UndoWater));
    assert_eq!(
        bubble_command(w, "details"),
        Some(Input::Water(WaterInput::OpenPanel))
    );
    assert_eq!(
        bubble_change(p, "add_500"),
        Some(EntryChange::LogWater { ml: 500 })
    );
    // Any serving button the user set up routes (the shell passes on only the card's own
    // actions); malformed amounts do not.
    assert_eq!(
        bubble_change(p, "add_750"),
        Some(EntryChange::LogWater { ml: 750 })
    );
    assert_eq!(bubble_change(p, "add_305"), None);
    assert_eq!(bubble_change(p, "add_0300"), None);
    assert_eq!(bubble_change(p, "other_add"), Some(EntryChange::AddOther));
    assert_eq!(bubble_change(p, "edit_save"), Some(EntryChange::SaveEdit));
    assert_eq!(
        bubble_change(p, &format!("entry_remove_{id}")),
        Some(EntryChange::RemoveWater(id))
    );
    assert_eq!(
        bubble_command(p, &format!("entry_edit_{id}")),
        Some(Input::Water(WaterInput::Edit(id)))
    );
    assert_eq!(
        bubble_command(p, "other_more"),
        Some(Input::Water(WaterInput::StepOther { up: true }))
    );
    assert_eq!(
        bubble_command(p, "edit_less"),
        Some(Input::Water(WaterInput::StepEdit { up: false }))
    );
    // Ids must be in the form the card prints them.
    let upper = id.to_string().to_uppercase();
    assert_eq!(bubble_change(p, &format!("entry_remove_{upper}")), None);
    assert_eq!(bubble_command(p, "entry_edit_x"), None);
    assert_eq!(bubble_change(p, "undo"), None);
    assert_eq!(
        bubble_change(n, "label_work"),
        Some(EntryChange::LabelPhone(PhoneLabel::Work))
    );
    assert_eq!(bubble_change(n, "label_unlabelled"), None);
    assert_eq!(bubble_change(n, "undo"), Some(EntryChange::UndoPhone));
    assert_eq!(bubble_change(BubbleKind::CheckIn, "undo"), None);
    assert_eq!(bubble_change(w, "label_work"), None);
    for k in [w, p, n] {
        assert_eq!(BubbleKind::parse(&k.id(9)), Some(k));
    }
    // A change carries the card it was asked on, and only that card routes.
    let shown = w.id(3);
    let state = RouteState {
        bubble_id: Some(shown.clone()),
        click_reopens: false,
    };
    let undo = |bubble_id: &str| ShellEvent::BubbleAction {
        bubble_id: bubble_id.into(),
        action_id: "undo".into(),
    };
    assert_eq!(
        route(&state, &undo(&shown)).command,
        Some(Command::Change {
            bubble_id: shown.clone(),
            change: EntryChange::UndoWater
        })
    );
    assert_eq!(route(&state, &undo(&w.id(2))).command, None);
}

fn with_goal(store: &Store) {
    store
        .set_goal(GoalKind::WaterDailyMl, 2000, today())
        .unwrap();
}

impl H {
    /// Today's current water entries' amounts, from the store.
    fn water(&self) -> Vec<u32> {
        self.link
            .coach()
            .store()
            .water_entries_on(today())
            .unwrap()
            .into_iter()
            .map(|e| e.amount_ml)
            .collect()
    }

    fn type_count(&self, name: &str) -> usize {
        self.events()
            .iter()
            .filter(|p| p.type_name() == name)
            .count()
    }

    fn title(&self) -> Option<String> {
        self.shell.open_bubble().map(|b| b.title)
    }

    /// The id of the action labelled `label` on the open card (rows included).
    fn action_labelled(&self, label: &str) -> String {
        let b = self.shell.open_bubble().expect("a card is open");
        b.rows
            .iter()
            .flat_map(|r| r.actions.iter())
            .chain(b.actions.iter())
            .find(|a| a.label == label)
            .map(|a| a.id.clone())
            .unwrap_or_else(|| panic!("no {label} on {}", b.title))
    }
}

#[test]
fn a_quick_log_confirms_and_undo_removes_it_once() {
    let mut h = H::with_store(with_goal);
    h.menu("water_250");
    assert_eq!(h.title().as_deref(), Some("Logged 250 ml, 250 / 2000 ml"));
    assert_eq!(h.water(), [250]);
    // Undo twice quickly: one retraction; the card says what was removed.
    h.press_quickly(&[action::UNDO, action::UNDO]);
    assert_eq!(h.type_count("entry.retracted"), 1);
    assert_eq!(h.water(), Vec::<u32>::new());
    assert_eq!(h.title().as_deref(), Some("Removed 250 ml"));
    assert_eq!(h.link.view().water.unwrap().logged_ml, 0);
    // OK closes it.
    h.press(action::OK);
    assert_eq!(h.shell.open(), Open::None);
    // Another serving from the menu, and the total in the menu follows.
    h.menu("water_500");
    assert_eq!(h.title().as_deref(), Some("Logged 500 ml, 500 / 2000 ml"));
    let menu = lock(&h.shell.model).menu_actions().to_vec();
    assert!(menu.iter().any(|a| a.label == "Water... (500 / 2000 ml)"));
    // It closes by itself after a minute.
    h.work(65);
    assert_eq!(h.shell.open(), Open::None);
    assert_eq!(h.water(), [500]);
}

#[test]
fn the_water_panel_logs_corrects_and_removes_entries() {
    let mut h = H::with_store(with_goal);
    h.menu("water_250");
    h.press(action::DETAILS);
    assert_eq!(h.title().as_deref(), Some("Water today"));
    // Edit the entry: +50 twice, then Save twice quickly: one correction.
    let edit = h.action_labelled("Edit");
    h.press(&edit);
    h.press(action::EDIT_MORE);
    h.press(action::EDIT_MORE);
    let b = h.shell.open_bubble().unwrap();
    assert!(
        b.rows[0].text.ends_with("change to 350 ml"),
        "{}",
        b.rows[0].text
    );
    h.press_quickly(&[action::EDIT_SAVE, action::EDIT_SAVE]);
    assert_eq!(h.water(), [350]);
    assert_eq!(
        h.type_count("water.logged"),
        2,
        "the entry and its correction"
    );
    let all = h.link.coach().store().events_on(today()).unwrap();
    let correction = all
        .iter()
        .find(|e| e.supersedes.is_some())
        .expect("the correction supersedes the entry");
    assert_eq!(
        correction.payload,
        EventPayload::WaterLogged { amount_ml: 350 }
    );
    let b = h.shell.open_bubble().unwrap();
    assert!(b.rows[0].text.ends_with(", 350 ml"), "{}", b.rows[0].text);
    // Edit then Cancel changes nothing.
    let edit = h.action_labelled("Edit");
    h.press(&edit);
    h.press(action::EDIT_LESS);
    h.press(action::EDIT_CANCEL);
    assert_eq!(h.water(), [350]);
    // A serving and the other amount (stepped to 350 ml) from the panel, which stays.
    h.press("add_150");
    h.press(action::OTHER_MORE);
    h.press(action::OTHER_MORE);
    let add = h.action_labelled("Add 350 ml");
    h.press(&add);
    assert_eq!(h.water(), [350, 150, 350]);
    assert_eq!(h.title().as_deref(), Some("Water today"));
    assert_eq!(h.link.view().water.unwrap().logged_ml, 850);
    // Remove the first entry: a retraction; totals follow.
    let remove = h.action_labelled("Remove");
    h.press(&remove);
    assert_eq!(h.water(), [150, 350]);
    assert_eq!(h.type_count("entry.retracted"), 1);
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.lines[0], "500 / 2000 ml (your goal)");
    // Three minutes without an answer: the panel closes.
    h.work(185);
    assert_eq!(h.shell.open(), Open::None);
}

#[test]
fn removing_every_entry_shows_the_empty_state() {
    let mut h = H::with_store(with_goal);
    h.menu("water_150");
    h.press(action::DETAILS);
    let remove = h.action_labelled("Remove");
    h.press_quickly(&[&remove, &remove]);
    assert_eq!(h.type_count("entry.retracted"), 1);
    let b = h.shell.open_bubble().unwrap();
    assert!(
        b.lines.contains(
            &"No entries yet today. That means nothing logged, not zero water.".to_owned()
        )
    );
}

#[test]
fn reaching_the_goal_celebrates_once_and_pauses_hydration_reminders() {
    let mut h = H::with_store(|s| {
        s.set_goal(GoalKind::WaterDailyMl, 600, today()).unwrap();
    });
    h.menu("water_500");
    assert_ne!(h.shell.state().0, PetState::Celebration);
    h.menu("water_150");
    assert_eq!(h.shell.state().0, PetState::Celebration);
    let b = h.shell.open_bubble().unwrap();
    assert_eq!(b.title, "Logged 150 ml, 650 / 600 ml");
    assert_eq!(b.lines, [present::GOAL_REACHED]);
    assert!(h.link.view().water.unwrap().goal_reached);
    h.work(10);
    h.menu("water_150");
    assert_ne!(
        h.shell.state().0,
        PetState::Celebration,
        "no praise past the goal"
    );
    // No water reminder for the rest of the morning.
    h.work(90 * 60);
    assert!(!h.events().iter().any(|p| matches!(
        p,
        EventPayload::ReminderOffered {
            activity: Activity::Water,
            ..
        }
    )));
}

#[test]
fn a_phone_note_takes_an_optional_label_and_undo() {
    let mut h = H::new();
    h.menu("phone_note");
    assert_eq!(
        h.title().as_deref(),
        Some("Phone interruption noted, 10:00")
    );
    assert!(h.events().contains(&EventPayload::PhoneInterruption {
        label: PhoneLabel::Unlabelled
    }));
    // "Work", then "Work" again (nothing new), then "Personal" and "Undo" quickly: the
    // first of those applies.
    h.press("label_work");
    assert_eq!(h.shell.open_bubble().unwrap().lines, ["Saved as work."]);
    h.press("label_work");
    assert_eq!(h.type_count("phone.interruption"), 2);
    h.press_quickly(&["label_personal", action::UNDO]);
    assert_eq!(h.type_count("phone.interruption"), 3);
    assert_eq!(h.type_count("entry.retracted"), 0);
    let current: Vec<_> = h
        .link
        .coach()
        .store()
        .current_events_on(today())
        .unwrap()
        .into_iter()
        .filter(|e| e.payload.type_name() == "phone.interruption")
        .map(|e| e.payload)
        .collect();
    assert_eq!(
        current,
        [EventPayload::PhoneInterruption {
            label: PhoneLabel::Personal
        }]
    );
    h.press(action::UNDO);
    assert_eq!(h.type_count("entry.retracted"), 1);
    assert_eq!(h.title().as_deref(), Some("Phone note removed"));
    assert!(
        h.link
            .coach()
            .store()
            .current_events_on(today())
            .unwrap()
            .iter()
            .all(|e| e.payload.type_name() != "phone.interruption")
    );
    // The note closes after a minute.
    h.work(65);
    assert_eq!(h.shell.open(), Open::None);
}

#[test]
fn a_new_hold_or_hiding_the_pet_closes_a_note_and_nothing_returns() {
    let mut h = H::with_store(with_goal);
    h.menu("water_panel");
    assert_eq!(h.title().as_deref(), Some("Water today"));
    h.menu("meeting_on");
    assert_eq!(h.shell.open(), Open::None);
    h.menu("meeting_off");
    h.work(30);
    assert_eq!(h.shell.open(), Open::None);
    // Opened under a meeting, the note stays while it lasts.
    h.menu("meeting_on");
    h.menu("phone_note");
    assert!(h.title().is_some());
    h.work(10);
    assert!(h.title().is_some());
    h.menu("meeting_off");
    // A presentation closes it.
    h.menu("water_250");
    h.input(Input::PresentationChanged(true));
    h.input(Input::PresentationChanged(false));
    assert_eq!(h.shell.open(), Open::None);
    // While the user hides the pet, logging still records and no card pops up later.
    h.user_hide();
    h.menu("water_250");
    assert_eq!(h.water(), [250, 250]);
    h.tray_show();
    assert_eq!(h.shell.open(), Open::None);
}

#[test]
fn a_note_replaces_a_reminder_which_then_waits_as_its_badge() {
    let mut h = H::new();
    let (offer, _) = h.work_until_reminder(25);
    h.menu("phone_note");
    assert!(h.title().unwrap().starts_with("Phone interruption noted"));
    assert!(h.responses().is_empty(), "replacing is not an answer");
    // The note closes; after the calm gap a click brings the reminder back.
    h.press("label_unsure");
    h.close_button();
    assert_eq!(h.shell.open(), Open::None);
    h.click_pet();
    assert_eq!(
        h.shell.open_bubble().map(|b| BubbleKind::parse(&b.id)),
        Some(Some(BubbleKind::Reminder { offer }))
    );
}

#[test]
fn already_did_it_on_a_water_reminder_opens_the_panel() {
    let mut h = H::with_store(with_goal);
    let offer = loop {
        let (offer, activity) = h.work_until_reminder(120);
        if activity == Activity::Water {
            break offer;
        }
        h.press(action::DONE);
    };
    h.press(action::DONE);
    assert!(h.responses().contains(&(offer, ReminderOutcome::Done)));
    assert_eq!(h.title().as_deref(), Some("Water today"));
    h.press("add_500");
    assert_eq!(h.water(), [500]);
}

#[test]
fn a_change_queued_before_its_card_changed_writes_nothing() {
    let mut h = H::with_store(with_goal);
    h.menu("water_250");
    h.shell.press(action::UNDO);
    let stale = h.take_queued();
    // Meanwhile the user logs another serving: a new confirmation.
    h.menu("water_150");
    for c in stale {
        let out = h.link.command(c).unwrap();
        assert!(matches!(
            out.outcome,
            hy_core::coach::InputOutcome::Ignored(_)
        ));
    }
    h.check();
    assert_eq!(h.water(), [250, 150]);
    assert_eq!(h.type_count("entry.retracted"), 0);
    // A phone note answered by the engine thread directly also ignores a stale input.
    let out = h.link.input(Input::Phone(PhoneInput::Undone)).unwrap();
    assert!(matches!(
        out.outcome,
        hy_core::coach::InputOutcome::Ignored(_)
    ));
}

#[derive(Debug, Clone)]
enum Step {
    Work(u64),
    Lock(u64),
    Sleep(u64),
    Presentation(bool),
    Press(usize),
    Close,
    Click,
    UserHide,
    TrayShow,
    Menu(usize),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (1u64..40).prop_map(|m| Step::Work(m * 60)),
        1 => (1u64..6).prop_map(|m| Step::Lock(m * 60)),
        1 => (1u64..120).prop_map(|m| Step::Sleep(m * 60)),
        1 => any::<bool>().prop_map(Step::Presentation),
        4 => (0usize..32).prop_map(Step::Press),
        1 => Just(Step::Close),
        1 => Just(Step::Click),
        1 => Just(Step::UserHide),
        1 => Just(Step::TrayShow),
        2 => (0usize..24).prop_map(Step::Menu),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Whatever happens, at most one bubble is open, it is the engine's current one,
    /// nothing is shown while the engine hides the pet, and no finished reminder returns
    /// (checked after every step by `H::check`).
    #[test]
    fn at_most_one_bubble_and_no_replay(steps in proptest::collection::vec(step(), 1..30)) {
        let mut h = H::new();
        for s in steps {
            match s {
                Step::Work(secs) => h.work(secs),
                Step::Lock(secs) => {
                    h.input(Input::Lock);
                    h.pass(secs);
                    h.input(Input::Unlock);
                }
                Step::Sleep(secs) => {
                    h.input(Input::Suspend);
                    h.pass(secs);
                    h.input(Input::Resume);
                }
                Step::Presentation(on) => h.input(Input::PresentationChanged(on)),
                Step::Press(i) => {
                    if let Some(b) = h.shell.open_bubble() {
                        let all = all_actions(&b);
                        let a = all[i % all.len()].clone();
                        h.press(&a);
                    }
                }
                Step::Close => {
                    h.close_button();
                }
                Step::Click => {
                    h.click_pet();
                }
                Step::UserHide => {
                    h.user_hide();
                }
                Step::TrayShow => {
                    h.tray_show();
                }
                Step::Menu(i) => {
                    let ids = h.shell.menu_ids();
                    if !ids.is_empty() {
                        let id = ids[i % ids.len()].clone();
                        h.menu(&id);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ V1-4

#[test]
fn panel_and_menu_use_the_users_serving_buttons() {
    let mut v = view(PetMode::Idle);
    v.servings = Servings::new(vec![200, 750]).unwrap();
    let ids: Vec<String> = present::menu_actions(&v)
        .into_iter()
        .map(|a| a.id)
        .filter(|id| id.starts_with("water_") && id != "water_panel")
        .collect();
    assert_eq!(ids, vec!["water_200", "water_750"]);
    v.bubble = Some(CoachBubble::WaterPanel {
        edit: None,
        other_ml: 250,
    });
    let b = present::bubble(v.bubble.as_ref().unwrap(), &v);
    let adds: Vec<&str> = b
        .actions
        .iter()
        .map(|a| a.id.as_str())
        .filter(|id| id.starts_with("add_"))
        .collect();
    assert_eq!(adds, vec!["add_200", "add_750"]);
    for id in adds {
        assert!(bubble_change(BubbleKind::WaterPanel, id).is_some());
    }
}

#[test]
fn a_default_movement_version_starts_that_version() {
    let mut v = view(PetMode::Exercise);
    let mut g = guide(GuideStage::Intro, None);
    g.default_variant = Some(RoutineVariant::Standing);
    v.bubble = Some(CoachBubble::MovementGuide(g));
    let b = present::bubble(v.bubble.as_ref().unwrap(), &v);
    let first: Vec<(&str, &str)> = b
        .actions
        .iter()
        .take(2)
        .map(|a| (a.id.as_str(), a.label.as_str()))
        .collect();
    assert_eq!(
        first,
        vec![
            ("standing", "Start (standing)"),
            ("seated", "Seated instead")
        ]
    );
    assert_eq!(b.actions[0].style, crate::shell::ActionStyle::Primary);
    assert_eq!(
        bubble_command(BubbleKind::MovementGuide, "standing"),
        Some(Input::Guide(GuideAction::Variant(RoutineVariant::Standing)))
    );
}

#[test]
fn the_review_offer_routes_its_three_answers() {
    let mut v = view(PetMode::Idle);
    v.bubble = Some(CoachBubble::ReviewOffer {
        week: oct(19),
        later: true,
    });
    let b = present::bubble(v.bubble.as_ref().unwrap(), &v);
    assert_eq!(b.title, "Your week in review is ready");
    let ids: Vec<&str> = b.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["review_open_2026-10-19", "review_later", "review_skip"]
    );
    let k = BubbleKind::ReviewOffer;
    assert_eq!(BubbleKind::parse(&k.id(3)), Some(k));
    assert_eq!(
        bubble_command(k, "review_open_2026-10-19"),
        Some(Input::Review(hy_core::coach::ReviewAnswer::Open))
    );
    assert_eq!(
        bubble_command(k, "review_later"),
        Some(Input::Review(hy_core::coach::ReviewAnswer::Later))
    );
    assert_eq!(
        bubble_command(k, "review_skip"),
        Some(Input::Review(hy_core::coach::ReviewAnswer::Skip))
    );
    assert_eq!(bubble_command(k, "review_open_soon"), None);
    assert_eq!(super::review_week("review_open_2026-10-19"), Some(oct(19)));
    assert_eq!(super::review_week("review_later"), None);
    // The second offer of the week has no Later.
    v.bubble = Some(CoachBubble::ReviewOffer {
        week: oct(19),
        later: false,
    });
    let b = present::bubble(v.bubble.as_ref().unwrap(), &v);
    assert!(b.actions.iter().all(|a| a.id != "review_later"));
}

#[test]
fn edit_plan_closes_the_planned_card() {
    assert_eq!(
        bubble_command(BubbleKind::RecoveryPlanned, "edit_plan"),
        Some(Input::Recovery(RecoveryAction::Ok))
    );
}

#[test]
fn settings_changed_through_the_link_reach_the_engine_at_once() {
    let mut h = H::new();
    assert_eq!(h.link.view().servings.ml(), &[150, 250, 500]);
    let servings = hy_core::coach::Servings::new(vec![200, 750]).unwrap();
    h.link
        .change_settings(|s| s.set_water_servings(&servings))
        .unwrap();
    // No step needed: the change refreshes the engine's view.
    assert_eq!(h.link.view().servings.ml(), &[200, 750]);
    // A refused change leaves everything as it was.
    let refused = h.link.change_settings(|s| {
        s.set_work_schedule(&hy_core::time::WorkSchedule {
            start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            ..hy_core::time::WorkSchedule::default()
        })
    });
    assert!(refused.is_err());
    assert_eq!(h.link.view().servings.ml(), &[200, 750]);
    h.pump();
}

#[test]
fn field_changes_from_two_windows_both_reach_the_engine() {
    use hy_core::settings::{CoachChange, WaterChange};
    let mut h = H::new();
    // Settings shows 150/250/500. The review adds 750 (another window); then Settings
    // removes 150 from its older copy: only that button goes.
    h.link
        .change_settings(|s| {
            let today = s.today()?;
            s.change_water(&WaterChange::AddServing(750), today)
        })
        .unwrap();
    h.link
        .change_settings(|s| {
            let today = s.today()?;
            s.change_water(&WaterChange::RemoveServing(150), today)
        })
        .unwrap();
    assert_eq!(h.link.view().servings.ml(), &[250, 500, 750]);
    let seated = h
        .link
        .change_settings(|s| {
            s.change_coach_config(CoachChange::MovementVersion(
                hy_core::coach::MovementVersion::Seated,
            ))
        })
        .unwrap();
    let after = h
        .link
        .change_settings(|s| s.change_coach_config(CoachChange::EyeRest(false)))
        .unwrap();
    assert_eq!(after.movement_version, seated.movement_version);
    assert!(!after.categories.eye_rest);
    h.pump();
}
