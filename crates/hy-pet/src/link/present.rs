//! Drawing the coaching engine's view on the pet shell: the pet's state and tag, the one
//! bubble with its actions, and the engine's entries in the pet menu, keyboard panel and
//! tray (wireframe sections 3 and 4).
//!
//! Every function here is pure. The engine decides what is shown; this module only words
//! it. Action ids are the constants in [`action`], which [`super::route`] turns back into
//! engine inputs, so a button can never send something the card did not offer.

use std::time::Duration;

use chrono::{NaiveDate, Weekday};
use hy_core::coach::movement::{
    INTRO_SAFETY, PROVISIONAL, PROVISIONAL_NOTICE, STEP_SAFETY, routine_for,
};
use hy_core::coach::{
    Bubble as CoachBubble, Chip, ChipStatus, CoachView, GuideStage, GuideView, PetMode,
    PomodoroPause, PomodoroPhase, PomodoroView, RecoveryPlan, ReminderReason,
};
use hy_core::schema::{Activity, RoutineOutcome, SessionState};
use uuid::Uuid;

use crate::shell::{ActionStyle, Bubble, BubbleAction, BubbleChip, BubbleFigure, MenuAction};
use crate::state::PetState;

/// The serving logged by "Log 250 ml" and "+250 ml water" (wireframe B2 and B1).
pub const WATER_SERVING_ML: u32 = 250;

/// Action ids used in bubbles and menus.
pub mod action {
    // Reminder card (B2).
    /// "Already did it".
    pub const DONE: &str = "done";
    /// "Start movement" or "Start 20 s eye rest".
    pub const START: &str = "start";
    /// "Snooze 10 min".
    pub const SNOOZE: &str = "snooze";
    /// "Not now".
    pub const DISMISS: &str = "dismiss";
    /// "Log 250 ml".
    pub const LOG_WATER: &str = "log_water";
    // Pomodoro cards (B6).
    /// "Start break".
    pub const START_BREAK: &str = "start_break";
    /// "Skip break".
    pub const SKIP_BREAK: &str = "skip_break";
    /// "Start next round".
    pub const NEXT_ROUND: &str = "next_round";
    /// "Resume".
    pub const RESUME_POMODORO: &str = "resume_pomodoro";
    /// "End Pomodoro".
    pub const END_POMODORO: &str = "end_pomodoro";
    // Check-in (B7) and the lighter-schedule offer.
    /// "Yes, it was".
    pub const CHECK_IN_YES: &str = "yes";
    /// "No".
    pub const CHECK_IN_NO: &str = "no";
    /// "Don't ask today".
    pub const CHECK_IN_NOT_TODAY: &str = "dont_ask";
    /// "Use Light".
    pub const USE_LIGHT: &str = "use_light";
    /// "Keep current".
    pub const KEEP_INTENSITY: &str = "keep";
    // Movement guide (B4).
    /// "Seated" on the routine's first screen.
    pub const SEATED: &str = "seated";
    /// "Standing" on the routine's first screen.
    pub const STANDING: &str = "standing";
    /// "Done" on a movement.
    pub const STEP_DONE: &str = "step_done";
    /// "Skip this movement".
    pub const STEP_SKIP: &str = "step_skip";
    /// "Stop routine".
    pub const STOP: &str = "stop";
    /// "Later" (also on the recovery invitation).
    pub const LATER: &str = "later";
    /// "Skip today".
    pub const SKIP_TODAY: &str = "skip_today";
    /// "Other day".
    pub const OTHER_DAY: &str = "other_day";
    /// "Back" from the day chooser.
    pub const BACK: &str = "back";
    /// Prefix of a day's routine: `day_mon` to `day_sat`.
    pub const DAY_PREFIX: &str = "day_";
    /// "OK" (the routine's result, the planned day).
    pub const OK: &str = "ok";
    // Deadline day and recovery day (B7, flow 4).
    /// Prefix of "Tomorrow" and the date choices: `plan_2026-10-21`.
    pub const PLAN_PREFIX: &str = "plan_";
    /// "Choose a date" or "Reschedule".
    pub const CHOOSE_DATE: &str = "choose_date";
    /// "No thanks".
    pub const NO_THANKS: &str = "no_thanks";
    /// "Cancel plan" or "Cancel for today".
    pub const CANCEL_PLAN: &str = "cancel_plan";
    /// "Going now" on the walk suggestion.
    pub const WALK_NOW: &str = "walk_now";
    /// "Not now" on the walk suggestion.
    pub const WALK_NOT_NOW: &str = "walk_not_now";
    /// "Yes" to "Did this routine feel helpful?".
    pub const REFLECT_YES: &str = "reflect_yes";
    /// "Somewhat".
    pub const REFLECT_SOMEWHAT: &str = "reflect_somewhat";
    /// "No".
    pub const REFLECT_NO: &str = "reflect_no";
    /// "Skip" (nothing is recorded).
    pub const REFLECT_SKIP: &str = "reflect_skip";

    // Pet menu, keyboard panel and tray (B1, B5, B6, T1).
    /// "Show waiting reminder" or "Show Pomodoro card".
    pub const OPEN_PENDING: &str = "open_pending";
    /// "+250 ml water".
    pub const WATER: &str = "water_250";
    /// "Today's movement".
    pub const MOVEMENT_START: &str = "movement_start";
    /// "Deadline day" or "Plan a recovery day".
    pub const DEADLINE_DAY: &str = "deadline_day";
    /// "Recovery day: Thu 22 Oct".
    pub const RECOVERY_SHOW: &str = "recovery_show";
    /// "Mark a break".
    pub const BREAK: &str = "break";
    /// "End focus".
    pub const FOCUS_END: &str = "focus_end";
    /// Prefix of "Focus N min": `focus_<minutes>`, with `_rp` when it replaces a Pomodoro.
    pub const FOCUS_PREFIX: &str = "focus_";
    /// Suffix: the label says the running Pomodoro ends.
    pub const REPLACES_POMODORO: &str = "_rp";
    /// "Pause Pomodoro".
    pub const POMODORO_PAUSE: &str = "pomodoro_pause";
    /// "Resume Pomodoro".
    pub const POMODORO_RESUME: &str = "pomodoro_resume";
    /// "End Pomodoro".
    pub const POMODORO_END: &str = "pomodoro_end";
    /// Prefix of "Pomodoro 25/5" and "Pomodoro 50/10": `pomodoro_25` or `pomodoro_50`,
    /// with `_rf` when it replaces focus and `_rc` when it resumes coaching.
    pub const POMODORO_PREFIX: &str = "pomodoro_";
    /// Suffix: the label says quiet focus ends.
    pub const REPLACES_FOCUS: &str = "_rf";
    /// Suffix: the label says coaching resumes.
    pub const RESUMES_COACHING: &str = "_rc";
    /// "I'm in a meeting".
    pub const MEETING_ON: &str = "meeting_on";
    /// "Meeting over".
    pub const MEETING_OFF: &str = "meeting_off";
    /// "Pause 30 min".
    pub const PAUSE_30: &str = "pause_30";
    /// "Pause 1 hour".
    pub const PAUSE_60: &str = "pause_60";
    /// "Pause for today".
    pub const PAUSE_TODAY: &str = "pause_today";
    /// "Pause until I resume".
    pub const PAUSE_UNTIL_RESUMED: &str = "pause_on";
    /// "Resume coaching".
    pub const RESUME_COACHING: &str = "resume";
}

/// Which engine bubble a shell bubble shows. It is encoded in the bubble id, so an answer
/// is always matched to the card it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleKind {
    /// The reminder card for this offer.
    Reminder {
        /// The engine's offer id.
        offer: Uuid,
    },
    /// The Pomodoro break card.
    PomodoroBreak,
    /// "Break over. Start another round?"
    PomodoroRoundDone,
    /// "Pomodoro paused" after a lock or sleep.
    PomodoroPaused,
    /// The long-day check-in.
    CheckIn,
    /// "Fewer reminders?"
    Lighter,
    /// The guided movement routine.
    MovementGuide,
    /// "Deadline day?" from the pet menu.
    DeadlineDay,
    /// "Plan a gentler recovery day?"
    RecoveryInvite,
    /// "Choose a date".
    RecoveryDates,
    /// "Recovery day planned".
    RecoveryPlanned,
    /// The recovery day's morning card.
    RecoveryMorning,
    /// The optional walk suggestion.
    RecoveryWalk,
    /// "Did this routine feel helpful?"
    RecoveryReflection,
}

impl BubbleKind {
    /// The kind of an engine bubble.
    #[must_use]
    pub const fn of(b: &CoachBubble) -> Self {
        match b {
            CoachBubble::Reminder { offer, .. } => Self::Reminder { offer: *offer },
            CoachBubble::PomodoroBreak { .. } => Self::PomodoroBreak,
            CoachBubble::PomodoroRoundDone => Self::PomodoroRoundDone,
            CoachBubble::PomodoroPausedByLock => Self::PomodoroPaused,
            CoachBubble::LongDayCheckIn { .. } => Self::CheckIn,
            CoachBubble::LighterSchedule => Self::Lighter,
            CoachBubble::MovementGuide(_) => Self::MovementGuide,
            CoachBubble::DeadlineDay => Self::DeadlineDay,
            CoachBubble::RecoveryInvite { .. } => Self::RecoveryInvite,
            CoachBubble::RecoveryDates { .. } => Self::RecoveryDates,
            CoachBubble::RecoveryPlanned { .. } => Self::RecoveryPlanned,
            CoachBubble::RecoveryMorning { .. } => Self::RecoveryMorning,
            CoachBubble::RecoveryWalk => Self::RecoveryWalk,
            CoachBubble::RecoveryReflection => Self::RecoveryReflection,
        }
    }

    /// The fixed keys (every kind but a reminder, whose key carries its offer).
    const FIXED: [(Self, &'static str); 13] = [
        (Self::PomodoroBreak, "pomodoro-break"),
        (Self::PomodoroRoundDone, "pomodoro-round"),
        (Self::PomodoroPaused, "pomodoro-paused"),
        (Self::CheckIn, "check-in"),
        (Self::Lighter, "lighter"),
        (Self::MovementGuide, "movement-guide"),
        (Self::DeadlineDay, "deadline-day"),
        (Self::RecoveryInvite, "recovery-invite"),
        (Self::RecoveryDates, "recovery-dates"),
        (Self::RecoveryPlanned, "recovery-planned"),
        (Self::RecoveryMorning, "recovery-morning"),
        (Self::RecoveryWalk, "recovery-walk"),
        (Self::RecoveryReflection, "recovery-reflection"),
    ];

    fn key(self) -> String {
        match self {
            Self::Reminder { offer } => format!("reminder:{offer}"),
            other => Self::FIXED
                .iter()
                .find(|(k, _)| *k == other)
                .map_or_else(String::new, |(_, key)| (*key).to_owned()),
        }
    }

    /// The shell bubble id for the `rev`-th bubble shown. Each show gets a new id, so the
    /// pet page measures it afresh and a click on an older copy is ignored.
    #[must_use]
    pub fn id(self, rev: u64) -> String {
        format!("{}#{rev}", self.key())
    }

    /// The kind encoded in a shell bubble id.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        let (key, rev) = id.rsplit_once('#')?;
        rev.parse::<u64>().ok()?;
        if let Some((kind, _)) = Self::FIXED.iter().find(|(_, k)| *k == key) {
            return Some(*kind);
        }
        Some(Self::Reminder {
            offer: Uuid::parse_str(key.strip_prefix("reminder:")?).ok()?,
        })
    }
}

/// Whole minutes, rounded up (a tag never says "0 min" while time is left).
fn ceil_min(d: Duration) -> u64 {
    d.as_secs().div_ceil(60)
}

/// Whole minutes, rounded down (for counted work time).
const fn floor_min(d: Duration) -> u64 {
    d.as_secs() / 60
}

fn hours_minutes(m: u64) -> String {
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// The activity's name.
#[must_use]
pub const fn activity_name(a: Activity) -> &'static str {
    match a {
        Activity::Movement => "Movement",
        Activity::EyeRest => "Eye rest",
        Activity::Water => "Water",
    }
}

/// The Pomodoro tag ("Work 24:55", "Break 04:10", "Break ready").
#[must_use]
pub fn pomodoro_tag(p: &PomodoroView) -> String {
    if p.paused.is_some() {
        return "Pomodoro paused".to_owned();
    }
    match p.phase {
        PomodoroPhase::Work => format!("Work {}", clock(p.left)),
        PomodoroPhase::Break => format!("Break {}", clock(p.left)),
        PomodoroPhase::BreakOffered => "Break ready".to_owned(),
        PomodoroPhase::RoundDone => "Round done".to_owned(),
    }
}

/// The pet's state and tag for a view (wireframe section 4). `celebrating` shows the
/// brief celebration after a done answer, only when the pet would otherwise be idle.
#[must_use]
pub fn pet_look(view: &CoachView, celebrating: bool) -> (PetState, Option<String>) {
    match view.pet {
        // Not drawn while hidden; the state is kept simple so nothing changes on unhide.
        PetMode::Hidden(_) => (PetState::Idle, None),
        PetMode::Idle if celebrating => (PetState::Celebration, None),
        PetMode::Idle => (PetState::Idle, None),
        // Locked or asleep: the moon badge with its own words.
        PetMode::Away => (PetState::Quiet, Some("Away".to_owned())),
        PetMode::Paused {
            left,
            rest_of_today,
        } => {
            let tag = if rest_of_today {
                "Paused today".to_owned()
            } else {
                left.map_or_else(
                    || "Paused".to_owned(),
                    |l| format!("Paused {}", hours_minutes(ceil_min(l))),
                )
            };
            (PetState::Paused, Some(tag))
        }
        PetMode::OffHours => (PetState::Quiet, None),
        PetMode::Focus { left } => (
            PetState::Focus,
            Some(format!("Focus {}", hours_minutes(ceil_min(left)))),
        ),
        PetMode::Meeting => (PetState::Meeting, None),
        PetMode::Pomodoro(p) => (PetState::Pomodoro, Some(pomodoro_tag(&p))),
        PetMode::Exercise => (PetState::Exercise, None),
        PetMode::Reminder(a) => (PetState::Reminder, Some(activity_name(a).to_owned())),
        PetMode::Waiting(_) => (PetState::Waiting, None),
        PetMode::Nudge => (PetState::Cue, None),
        PetMode::Snoozed(a) => (
            PetState::Snoozed,
            view.chip(a)
                .snoozed_for
                .map(|d| format!("Snoozed {}", hours_minutes(ceil_min(d)))),
        ),
    }
}

/// A chip's status in words (wireframe B2: status never depends on colour alone).
#[must_use]
pub fn chip_status(c: &Chip) -> String {
    match c.status {
        ChipStatus::Off => "off".to_owned(),
        ChipStatus::NoGoal => "no goal set".to_owned(),
        ChipStatus::NotDue => "not due".to_owned(),
        // Another activity that is also due waits for its own turn.
        ChipStatus::Due => "also due".to_owned(),
        ChipStatus::Showing => "due".to_owned(),
        ChipStatus::Waiting => "waiting".to_owned(),
        ChipStatus::InProgress => "in progress".to_owned(),
        ChipStatus::Snoozed => c.snoozed_for.map_or_else(
            || "snoozed".to_owned(),
            |d| format!("snoozed {}", hours_minutes(ceil_min(d))),
        ),
        ChipStatus::Done => "done (self-reported)".to_owned(),
        ChipStatus::Partial => "partial (self-reported)".to_owned(),
        ChipStatus::NoneRecorded => "none recorded".to_owned(),
        ChipStatus::Skipped => "skipped".to_owned(),
        ChipStatus::Dismissed => "not now".to_owned(),
        ChipStatus::NoResponse => "no response".to_owned(),
        ChipStatus::GoalReached => "goal reached".to_owned(),
    }
}

fn act(id: &str, label: &str, style: ActionStyle) -> BubbleAction {
    BubbleAction {
        id: id.to_owned(),
        label: label.to_owned(),
        style,
    }
}

fn reminder_card(activity: Activity, reason: ReminderReason, chips: &[Chip; 3]) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    let (title, lines) = match reason {
        ReminderReason::Movement { worked } => (
            format!(
                "Movement: {} min of work since your last break",
                floor_min(worked)
            ),
            vec!["Work time is an estimate from keyboard and mouse use.".to_owned()],
        ),
        ReminderReason::EyeRest { screen_time } => (
            "Eye rest: look away for 20 seconds".to_owned(),
            vec![format!(
                "About {} min of screen time since your last eye rest (estimate).",
                floor_min(screen_time)
            )],
        ),
        ReminderReason::Water {
            expected_ml,
            logged_ml,
            behind_ml,
        } => (
            format!(
                "Water: {behind_ml} ml behind your own pace (about {expected_ml} ml expected by now)"
            ),
            vec![format!("Logged today: {logged_ml} ml.")],
        ),
    };
    let start = match activity {
        Activity::Movement => act(action::START, "Start movement", Primary),
        Activity::EyeRest => act(action::START, "Start 20 s eye rest", Primary),
        Activity::Water => act(
            action::LOG_WATER,
            &format!("Log {WATER_SERVING_ML} ml"),
            Primary,
        ),
    };
    Bubble {
        title,
        lines,
        chips: chips
            .iter()
            .map(|c| BubbleChip {
                label: activity_name(c.activity).to_owned(),
                status: chip_status(c),
                highlighted: c.activity == activity,
            })
            .collect(),
        actions: vec![
            start,
            act(action::DONE, "Already did it", Normal),
            act(action::SNOOZE, "Snooze 10 min", Normal),
            act(action::DISMISS, "Not now", Link),
        ],
        ..Bubble::default()
    }
}

/// A weekday's full name.
#[must_use]
pub const fn day_name(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// The action id suffix for a day's routine (`mon` to `sun`).
#[must_use]
pub const fn day_key(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

/// The scheduled days, Monday to Saturday.
const ROUTINE_DAYS: [Weekday; 6] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
];

/// A date as the cards show it ("Wed 21 Oct").
#[must_use]
pub fn short_date(d: NaiveDate) -> String {
    d.format("%a %-d %b").to_string()
}

const fn provisional_note() -> Option<&'static str> {
    if PROVISIONAL {
        Some(PROVISIONAL_NOTICE)
    } else {
        None
    }
}

fn guide_card(g: &GuideView) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    let note = provisional_note().map(str::to_owned);
    let day_actions = |extra: BubbleAction| {
        let mut v: Vec<_> = ROUTINE_DAYS
            .iter()
            .map(|d| {
                act(
                    &format!("{}{}", action::DAY_PREFIX, day_key(*d)),
                    day_name(*d),
                    if *d == g.day { Primary } else { Normal },
                )
            })
            .collect();
        v.push(extra);
        v
    };
    let routine = routine_for(g.day);
    let mut b = Bubble {
        note: note.clone(),
        continues: true,
        ..Bubble::default()
    };
    match (g.stage, routine) {
        (GuideStage::Sunday, _) | (GuideStage::Intro, None) => {
            b.title = "Sunday: no routine scheduled".to_owned();
            b.lines = vec![
                "Sunday is optional. If you'd like to move, choose any day's routine.".to_owned(),
            ];
            b.actions = day_actions(act(action::LATER, "Later", Link));
        }
        (GuideStage::Intro, Some(r)) => {
            b.title = if g.day == g.today {
                format!("Today's movement: {}", day_name(g.day))
            } else {
                format!("{}'s movement routine", day_name(g.day))
            };
            b.lines = r
                .movements
                .iter()
                .map(|e| {
                    let m = &e.main;
                    let seated = e
                        .seated
                        .map(|s| format!(" (seated version: {})", s.name.to_lowercase()))
                        .unwrap_or_default();
                    format!("{}: {}, {}{seated}", m.name, m.place, m.dose)
                })
                .collect();
            b.lines.push(INTRO_SAFETY.to_owned());
            b.actions = vec![
                act(action::SEATED, "Seated", Primary),
                act(action::STANDING, "Standing", Normal),
                act(action::LATER, "Later", Normal),
                act(action::OTHER_DAY, "Other day", Normal),
                act(action::SKIP_TODAY, "Skip today", Link),
            ];
        }
        (GuideStage::ChooseDay, _) => {
            b.title = "Choose a day's routine".to_owned();
            b.lines = ROUTINE_DAYS
                .iter()
                .filter_map(|d| {
                    let r = routine_for(*d)?;
                    let names: Vec<_> = r.movements.iter().map(|e| e.main.name).collect();
                    Some(format!("{}: {}", day_name(*d), names.join(", ")))
                })
                .collect();
            b.actions = day_actions(act(action::BACK, "Back", Link));
        }
        (GuideStage::Step { index, total }, Some(r)) => {
            let e = r.movements[(index as usize).min(r.movements.len() - 1)];
            let m = e.for_variant(g.variant);
            b.title = format!("Movement {} of {total}: {}", index + 1, m.name);
            b.lines = vec![
                format!("{}, {}", m.place, m.dose),
                format!("{}: {}", m.alternative_title, m.alternative),
                STEP_SAFETY.to_owned(),
            ];
            b.steps = m.steps.iter().map(|s| (*s).to_owned()).collect();
            b.figure = Some(BubbleFigure {
                id: m.figure.to_owned(),
                label: m.figure_label.to_owned(),
            });
            b.actions = vec![
                act(action::STEP_DONE, "Done", Primary),
                act(action::STEP_SKIP, "Skip this movement", Normal),
                act(action::STOP, "Stop routine", Link),
            ];
        }
        (GuideStage::Step { .. }, None) => {
            // Not reachable (a step always has a routine); offer a way out.
            b.title = "Movement".to_owned();
            b.actions = vec![act(action::STOP, "Stop routine", Primary)];
        }
        (
            GuideStage::Result {
                outcome,
                done,
                skipped,
                total,
                stopped,
            },
            _,
        ) => {
            let skipped_part = if skipped > 0 {
                format!(", {skipped} skipped")
            } else {
                String::new()
            };
            let (title, line) = match outcome {
                RoutineOutcome::Done => (
                    "Routine complete".to_owned(),
                    format!("All {total} movements done. Saved as self-reported."),
                ),
                RoutineOutcome::Partial => (
                    "Saved as partial".to_owned(),
                    format!(
                        "{done} of {total} movements done{skipped_part}{}. Recorded as partial (self-reported), not complete. No catch-up needed.",
                        if stopped { ", stopped early" } else { "" }
                    ),
                ),
                RoutineOutcome::None => (
                    "Stopped, none recorded".to_owned(),
                    format!(
                        "No movements were marked done{}. Nothing is counted as movement and your movement timer is not reset. No catch-up needed.",
                        if skipped > 0 {
                            format!(" ({skipped} skipped)")
                        } else {
                            String::new()
                        }
                    ),
                ),
                RoutineOutcome::Skipped => (
                    "Skipped for today".to_owned(),
                    "Nothing is owed and there is no catch-up.".to_owned(),
                ),
            };
            b.title = title;
            b.lines = vec![line];
            b.actions = vec![act(action::OK, "OK", Primary)];
            b.note = None;
            b.continues = false;
        }
    }
    b
}

/// The plan's items in words, only those switched on (wireframe W15).
fn plan_items(plan: &RecoveryPlan, water_goal_ml: Option<u32>) -> Vec<String> {
    let mut v = Vec::new();
    if plan.more_breaks {
        v.push("Short breaks offered a little more often".to_owned());
    }
    if let Some(t) = plan.walk_at {
        v.push(format!(
            "Optional walk at {}{}",
            t.format("%H:%M"),
            if plan.walk_repeat {
                ", repeated every 90 min"
            } else {
                ""
            }
        ));
    }
    if let Some(g) = water_goal_ml {
        v.push(format!("Your water goal stays {g} ml"));
    }
    if plan.lunch_away {
        v.push("Lunch away from the screen if you can".to_owned());
    }
    if plan.calm_pause {
        v.push("One brief calming pause".to_owned());
    }
    if let Some(t) = plan.finish_at {
        v.push(format!("Finish around {}", t.format("%H:%M")));
    }
    v
}

fn plan_action(d: NaiveDate, label: &str, style: ActionStyle) -> BubbleAction {
    act(
        &format!("{}{}", action::PLAN_PREFIX, d.format("%Y-%m-%d")),
        label,
        style,
    )
}

/// The shell bubble for the engine's bubble, without its id (the link numbers each show;
/// see [`BubbleKind::id`]).
#[must_use]
pub fn bubble(b: &CoachBubble, view: &CoachView) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    let card = |title: &str, lines: Vec<String>, actions: Vec<BubbleAction>| Bubble {
        title: title.to_owned(),
        lines,
        actions,
        ..Bubble::default()
    };
    match *b {
        CoachBubble::Reminder {
            activity, reason, ..
        } => reminder_card(activity, reason, &view.chips),
        CoachBubble::PomodoroBreak {
            length,
            water_behind,
        } => {
            let mut lines =
                vec!["Look away from the screen, stand up or walk a little.".to_owned()];
            if let Some(w) = water_behind {
                lines.push(format!(
                    "Water: {} ml behind your own pace for the time of day.",
                    w.behind_ml
                ));
            }
            card(
                &format!("Break time: {}", hours_minutes(ceil_min(length))),
                lines,
                vec![
                    act(action::START_BREAK, "Start break", Primary),
                    act(action::SKIP_BREAK, "Skip break", Normal),
                    act(action::END_POMODORO, "End Pomodoro", Link),
                ],
            )
        }
        CoachBubble::PomodoroRoundDone => card(
            "Break over. Start another round?",
            Vec::new(),
            vec![
                act(action::NEXT_ROUND, "Start next round", Primary),
                act(action::END_POMODORO, "End Pomodoro", Normal),
            ],
        ),
        CoachBubble::PomodoroPausedByLock => card(
            "Pomodoro paused",
            vec!["Your PC was locked or asleep. Resume or End.".to_owned()],
            vec![
                act(action::RESUME_POMODORO, "Resume", Primary),
                act(action::END_POMODORO, "End", Normal),
            ],
        ),
        CoachBubble::LongDayCheckIn { hours } => card(
            "Long day check-in",
            vec![
                format!(
                    "This PC has been active for about {hours} hours today. That is an estimate of PC activity, not a sign of stress or tiredness."
                ),
                "Would you like to mark today as a deadline-heavy day?".to_owned(),
            ],
            vec![
                act(action::CHECK_IN_YES, "Yes, it was", Primary),
                act(action::CHECK_IN_NO, "No", Normal),
                act(action::CHECK_IN_NOT_TODAY, "Don't ask today", Link),
            ],
        ),
        CoachBubble::LighterSchedule => card(
            "Fewer reminders?",
            vec![
                "You set aside several reminders today. The Light schedule offers them less often."
                    .to_owned(),
            ],
            vec![
                act(action::USE_LIGHT, "Use Light", Primary),
                act(action::KEEP_INTENSITY, "Keep current", Link),
            ],
        ),
        CoachBubble::MovementGuide(g) => guide_card(&g),
        CoachBubble::DeadlineDay => card(
            "Deadline day?",
            vec![
                "Would you like to mark today as a deadline-heavy day?".to_owned(),
                "Only you decide. The app does not infer stress or tiredness.".to_owned(),
            ],
            vec![
                act(action::CHECK_IN_YES, "Yes, it was", Primary),
                act(action::CHECK_IN_NO, "No", Normal),
            ],
        ),
        CoachBubble::RecoveryInvite { next } => {
            let label = if view.recovery.today.succ_opt() == Some(next) {
                format!("Tomorrow ({})", short_date(next))
            } else {
                short_date(next)
            };
            card(
                "Plan a gentler recovery day?",
                vec![
                    "A personal routine: more short breaks, an optional walk and a calm finish. Not a leave request."
                        .to_owned(),
                ],
                vec![
                    plan_action(next, &label, Primary),
                    act(action::CHOOSE_DATE, "Choose a date", Normal),
                    act(action::LATER, "Later", Normal),
                    act(action::NO_THANKS, "No thanks", Link),
                ],
            )
        }
        CoachBubble::RecoveryDates { dates } => {
            let mut actions: Vec<_> = dates
                .iter()
                .flatten()
                .map(|d| plan_action(*d, &short_date(*d), Normal))
                .collect();
            actions.push(act(action::LATER, "Later", Link));
            card("Choose a date", Vec::new(), actions)
        }
        CoachBubble::RecoveryPlanned { plan } => {
            let mut lines = vec!["I'll show it in the morning.".to_owned()];
            let items = plan_items(&plan, None);
            if !items.is_empty() {
                lines.push(format!("{}.", items.join("; ")));
            }
            card(
                &format!("Recovery day planned: {}", short_date(plan.date)),
                lines,
                vec![
                    act(action::OK, "OK", Primary),
                    act(action::CHOOSE_DATE, "Reschedule", Normal),
                    act(action::CANCEL_PLAN, "Cancel plan", Link),
                ],
            )
        }
        CoachBubble::RecoveryMorning {
            plan,
            water_goal_ml,
        } => card(
            "Today is your recovery day",
            plan_items(&plan, water_goal_ml),
            vec![
                act(action::OK, "Sounds good", Primary),
                act(action::CANCEL_PLAN, "Cancel for today", Link),
            ],
        ),
        CoachBubble::RecoveryWalk => card(
            "Optional walk",
            vec![
                "A comfortable 10 to 15 minute walk, if it suits you. Skipping is fine.".to_owned(),
            ],
            vec![
                act(action::WALK_NOW, "Going now", Primary),
                act(action::WALK_NOT_NOW, "Not now", Normal),
            ],
        ),
        CoachBubble::RecoveryReflection => card(
            "End of your recovery day",
            vec![
                "Did this routine feel helpful?".to_owned(),
                "Your answer stays on this PC.".to_owned(),
            ],
            vec![
                act(action::REFLECT_YES, "Yes", Normal),
                act(action::REFLECT_SOMEWHAT, "Somewhat", Normal),
                act(action::REFLECT_NO, "No", Normal),
                act(action::REFLECT_SKIP, "Skip", Link),
            ],
        ),
    }
}

/// Whether a click on the pet should reopen something the engine holds (a waiting
/// reminder, the movement guide waiting behind the pet, or a Pomodoro card the user has
/// not answered) rather than open the menu.
#[must_use]
pub fn click_reopens(view: &CoachView) -> bool {
    view.visible
        && view.bubble.is_none()
        && (waiting(view) || guide_waiting(view) || pomodoro_card_pending(view))
}

fn guide_waiting(view: &CoachView) -> bool {
    view.guide.is_some_and(|g| !g.open)
}

const fn waiting(view: &CoachView) -> bool {
    matches!(view.pet, PetMode::Waiting(_))
}

fn pomodoro_card_pending(view: &CoachView) -> bool {
    let waiting_offer = [Activity::Movement, Activity::EyeRest, Activity::Water]
        .iter()
        .any(|a| view.chip(*a).status == ChipStatus::Waiting);
    !waiting_offer
        && view.guide.is_none()
        && view.pomodoro.is_some_and(|p| {
            matches!(
                p.phase,
                PomodoroPhase::BreakOffered | PomodoroPhase::RoundDone
            ) || p.paused == Some(PomodoroPause::LockOrSleep)
        })
}

fn menu(id: &str, label: &str, in_tray: bool) -> MenuAction {
    MenuAction {
        id: id.to_owned(),
        label: label.to_owned(),
        in_tray,
    }
}

/// The engine's entries in the pet menu and keyboard panel (and the pause entries in the
/// tray) for a view (wireframe B1, B5, B6 and T1).
///
/// The engine asks for a confirmation before replacing focus or a Pomodoro, or before a
/// Pomodoro resumes paused coaching. The menu never skips that silently: an entry that
/// would do one of these says so in its label, and only such an entry carries the
/// confirmation in its id.
#[must_use]
pub fn menu_actions(view: &CoachView) -> Vec<MenuAction> {
    use action as a;
    let mut m = Vec::new();
    if click_reopens(view) {
        let label = if waiting(view) {
            "Show waiting reminder"
        } else if guide_waiting(view) {
            "Show movement guide"
        } else {
            "Show Pomodoro card"
        };
        m.push(menu(a::OPEN_PENDING, label, false));
    }
    m.push(menu(
        a::WATER,
        &format!("+{WATER_SERVING_ML} ml water"),
        false,
    ));

    // The guide answers a running routine itself (its card, or the entry above while it
    // waits behind the pet), so the menu only starts one.
    let exercising =
        view.guide.is_some() || view.chip(Activity::Movement).status == ChipStatus::InProgress;
    if !exercising {
        m.push(menu(a::MOVEMENT_START, "Today's movement", false));
    }
    m.push(menu(a::BREAK, "Mark a break", false));

    // Deadline day and the recovery plan (B7, flow 4), when a card can open.
    let r = view.recovery;
    if r.can_open {
        if let Some(p) = r.plan {
            let label = if p.date == r.today {
                "Today is your recovery day".to_owned()
            } else {
                format!("Recovery day: {}", short_date(p.date))
            };
            m.push(menu(a::RECOVERY_SHOW, &label, false));
        }
        if !r.marked_today {
            m.push(menu(a::DEADLINE_DAY, "Deadline day", false));
        } else if r.plan.is_none() {
            m.push(menu(a::DEADLINE_DAY, "Plan a recovery day", false));
        }
    }

    let paused = view.session == SessionState::Paused;
    if view.focus_left.is_some() {
        m.push(menu(a::FOCUS_END, "End focus", false));
    } else {
        let replaces = view.pomodoro.is_some();
        for min in [30, 60, 90] {
            let (id, label) = if replaces {
                (
                    format!("{}{min}{}", a::FOCUS_PREFIX, a::REPLACES_POMODORO),
                    format!("Focus {min} min (ends Pomodoro)"),
                )
            } else {
                (
                    format!("{}{min}", a::FOCUS_PREFIX),
                    format!("Focus {min} min"),
                )
            };
            m.push(menu(&id, &label, false));
        }
    }

    match view.pomodoro {
        Some(p) => {
            if p.paused.is_some() {
                m.push(menu(a::POMODORO_RESUME, "Resume Pomodoro", false));
            } else if matches!(p.phase, PomodoroPhase::Work | PomodoroPhase::Break) {
                m.push(menu(a::POMODORO_PAUSE, "Pause Pomodoro", false));
            }
            m.push(menu(a::POMODORO_END, "End Pomodoro", false));
        }
        None => {
            let focus = view.focus_left.is_some();
            let suffix = match (focus, paused) {
                (false, false) => "",
                (true, false) => " (ends focus)",
                (false, true) => " (resumes coaching)",
                (true, true) => " (ends focus, resumes coaching)",
            };
            let flags = format!(
                "{}{}",
                if focus { a::REPLACES_FOCUS } else { "" },
                if paused { a::RESUMES_COACHING } else { "" }
            );
            for (work, brk) in [(25, 5), (50, 10)] {
                m.push(menu(
                    &format!("{}{work}{flags}", a::POMODORO_PREFIX),
                    &format!("Pomodoro {work}/{brk}{suffix}"),
                    false,
                ));
            }
        }
    }

    if view.meeting {
        m.push(menu(a::MEETING_OFF, "Meeting over", false));
    } else {
        m.push(menu(a::MEETING_ON, "I'm in a meeting", false));
    }
    if paused {
        m.push(menu(a::RESUME_COACHING, "Resume coaching", true));
    } else {
        m.push(menu(a::PAUSE_30, "Pause 30 min", true));
        m.push(menu(a::PAUSE_60, "Pause 1 hour", true));
        m.push(menu(a::PAUSE_TODAY, "Pause for today", true));
        m.push(menu(a::PAUSE_UNTIL_RESUMED, "Pause until I resume", true));
    }
    m
}
