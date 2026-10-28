//! Drawing the coaching engine's view on the pet shell: the pet's state and tag, the one
//! bubble with its actions, and the engine's entries in the pet menu, keyboard panel and
//! tray (wireframe sections 3 and 4).
//!
//! Every function here is pure. The engine decides what is shown; this module only words
//! it. Action ids are the constants in [`action`], which [`super::route`] turns back into
//! engine inputs, so a button can never send something the card did not offer.

use std::time::Duration;

use chrono::{NaiveDate, NaiveTime, Weekday};
use hy_core::coach::meditation::{self, MEDITATION_MINUTES, MEDITATION_PARTS};
use hy_core::coach::movement::{
    INTRO_SAFETY, PROVISIONAL, PROVISIONAL_NOTICE, STEP_SAFETY, routine_for,
};
use hy_core::coach::water::{WATER_DEFAULT_SERVING_ML, WATER_STEP_ML};
use hy_core::coach::{
    Bubble as CoachBubble, Chip, ChipStatus, CoachView, GuideStage, GuideView, MeditationView,
    PetMode, PomodoroPause, PomodoroPhase, PomodoroView, RecoveryPlan, ReminderReason, WaterEdit,
};
use hy_core::schema::{Activity, PhoneLabel, RoutineOutcome, RoutineVariant, SessionState};
use uuid::Uuid;

use crate::shell::{
    ActionStyle, Bubble, BubbleAction, BubbleChip, BubbleFigure, BubbleProgress, BubbleRow,
    MAX_BUBBLE_ROWS, MenuAction,
};
use crate::state::PetState;

/// The serving logged by "Log 250 ml" and "+250 ml water" (wireframe B2 and B1).
pub const WATER_SERVING_ML: u32 = WATER_DEFAULT_SERVING_ML;

/// Most of today's water entries the panel lists (the latest), so the card stays a
/// readable size beside the pet (D). The shell allows more rows than this.
pub const PANEL_ENTRIES: usize = 12;
const _: () = assert!(PANEL_ENTRIES < MAX_BUBBLE_ROWS);

/// The phone labels offered on the note, in order (wireframe B8).
pub const PHONE_LABELS: [PhoneLabel; 3] =
    [PhoneLabel::Work, PhoneLabel::Personal, PhoneLabel::Unsure];

/// "Your logging goal is reached; hydration reminders are paused" (wireframe B3 and section
/// 8 item 11), with manual logging still available.
pub const GOAL_REACHED: &str = "Your logging goal is reached; hydration reminders are paused. You can still log water by hand.";

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
    // Water confirmation, water panel and phone note (B3, B8).
    /// "Undo" on the water confirmation or the phone note.
    pub const UNDO: &str = "undo";
    /// "Details" on the water confirmation: the water panel.
    pub const DETAILS: &str = "details";
    /// Prefix of a serving button in the panel: `add_150`, `add_250`, `add_500`.
    pub const ADD_PREFIX: &str = "add_";
    /// "-50 ml" on the panel's other amount.
    pub const OTHER_LESS: &str = "other_less";
    /// "+50 ml" on the panel's other amount.
    pub const OTHER_MORE: &str = "other_more";
    /// "Add N ml" for the other amount.
    pub const OTHER_ADD: &str = "other_add";
    /// Prefix of "Edit" on an entry, followed by the entry's id.
    pub const ENTRY_EDIT_PREFIX: &str = "entry_edit_";
    /// Prefix of "Remove" on an entry, followed by the entry's id.
    pub const ENTRY_REMOVE_PREFIX: &str = "entry_remove_";
    /// "-50 ml" on the entry being edited.
    pub const EDIT_LESS: &str = "edit_less";
    /// "+50 ml" on the entry being edited.
    pub const EDIT_MORE: &str = "edit_more";
    /// "Save" on the entry being edited (a correction).
    pub const EDIT_SAVE: &str = "edit_save";
    /// "Cancel" on the entry being edited.
    pub const EDIT_CANCEL: &str = "edit_cancel";
    /// Prefix of a phone label: `label_work`, `label_personal`, `label_unsure`.
    pub const LABEL_PREFIX: &str = "label_";
    /// "Edit plan" on the planned recovery day: closes the card; the companion opens the
    /// plan window (wireframe W15).
    pub const EDIT_PLAN: &str = "edit_plan";
    // Weekly review offer (B9).
    /// Prefix of "Open", followed by the week's Monday: `review_open_2026-10-19`. Closes
    /// the card; the companion opens the review window on that week.
    pub const REVIEW_OPEN_PREFIX: &str = "review_open_";
    /// "Later" (offered once more on the next workday).
    pub const REVIEW_LATER: &str = "review_later";
    /// "Skip this week".
    pub const REVIEW_SKIP: &str = "review_skip";
    // Meditation invitation and pause (B12, V2-4).
    /// Prefix of a length on the invitation: `meditate_2`, `meditate_5`, `meditate_10`.
    pub const MEDITATE_PREFIX: &str = "meditate_";
    /// "Not today" on the invitation.
    pub const MEDITATE_NOT_TODAY: &str = "meditate_not_today";
    /// "Next" in the pause.
    pub const MEDITATE_NEXT: &str = "meditate_next";
    /// "Pause" in the pause.
    pub const MEDITATE_PAUSE: &str = "meditate_pause";
    /// "Resume" in the pause.
    pub const MEDITATE_RESUME: &str = "meditate_resume";
    /// The other version (breath-focused or eyes open).
    pub const MEDITATE_SWITCH: &str = "meditate_switch";
    /// "Stop" in the pause.
    pub const MEDITATE_STOP: &str = "meditate_stop";

    // The posture cue (slice V2-2).
    /// "Recalibrate": closes the cue; the companion opens the camera page to calibrate.
    pub const POSTURE_RECALIBRATE: &str = "posture_recalibrate";

    // Pet menu, keyboard panel and tray (B1, B5, B6, T1).
    /// "Show waiting reminder" or "Show Pomodoro card".
    pub const OPEN_PENDING: &str = "open_pending";
    /// "+250 ml water".
    pub const WATER: &str = "water_250";
    /// Prefix of a serving on the pet menu: `water_150`, `water_250`, `water_500`.
    pub const WATER_PREFIX: &str = "water_";
    /// "Water..." (the water panel).
    pub const WATER_PANEL: &str = "water_panel";
    /// "Phone interruption".
    pub const PHONE_NOTE: &str = "phone_note";
    /// "Today's movement".
    pub const MOVEMENT_START: &str = "movement_start";
    /// "Movement done".
    pub const MOVEMENT_DONE: &str = "movement_done";
    /// "Did part of it".
    pub const MOVEMENT_PARTIAL: &str = "movement_partial";
    /// "Stopped, none done".
    pub const MOVEMENT_STOP: &str = "movement_stop";
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
    /// The water confirmation ("Logged 250 ml").
    WaterLogged,
    /// The water panel.
    WaterPanel,
    /// The phone interruption note.
    PhoneNote,
    /// The weekly review offer.
    ReviewOffer,
    /// The meditation invitation.
    MeditationInvite,
    /// The meditation pause.
    Meditation,
    /// The posture cue.
    PostureCue,
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
            CoachBubble::WaterLogged { .. } => Self::WaterLogged,
            CoachBubble::WaterPanel { .. } => Self::WaterPanel,
            CoachBubble::PhoneNote { .. } => Self::PhoneNote,
            CoachBubble::ReviewOffer { .. } => Self::ReviewOffer,
            CoachBubble::MeditationInvite => Self::MeditationInvite,
            CoachBubble::Meditation(_) => Self::Meditation,
            CoachBubble::PostureCue => Self::PostureCue,
        }
    }

    /// The fixed keys (every kind but a reminder, whose key carries its offer).
    const FIXED: [(Self, &'static str); 20] = [
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
        (Self::WaterLogged, "water-logged"),
        (Self::WaterPanel, "water-panel"),
        (Self::PhoneNote, "phone-note"),
        (Self::ReviewOffer, "review-offer"),
        (Self::MeditationInvite, "meditation-invite"),
        (Self::Meditation, "meditation"),
        (Self::PostureCue, "posture-cue"),
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
            // With a default version (wireframe W6) the first button starts it and the
            // other version is offered instead of asking.
            let (first, second) = match g.default_variant {
                Some(RoutineVariant::Seated) => (
                    act(action::SEATED, "Start (seated)", Primary),
                    act(action::STANDING, "Standing instead", Normal),
                ),
                Some(RoutineVariant::Standing) => (
                    act(action::STANDING, "Start (standing)", Primary),
                    act(action::SEATED, "Seated instead", Normal),
                ),
                None => (
                    act(action::SEATED, "Seated", Primary),
                    act(action::STANDING, "Standing", Normal),
                ),
            };
            b.actions = vec![
                first,
                second,
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

/// Today's water: the total logged (current entries) and the user's own goal, if set.
#[must_use]
pub fn water_today(view: &CoachView) -> (u32, Option<u32>) {
    view.water.map_or_else(
        || {
            let sum = view
                .water_entries
                .iter()
                .fold(0u32, |s, e| s.saturating_add(e.amount_ml));
            (sum, None)
        },
        |w| (w.logged_ml, Some(w.goal_ml)),
    )
}

/// "750 / 2000 ml (your goal)" or "750 ml today (no goal set)".
fn water_total_words(logged: u32, goal: Option<u32>) -> String {
    goal.map_or_else(
        || format!("{logged} ml today (no goal set)"),
        |g| format!("{logged} / {g} ml (your goal)"),
    )
}

const fn phone_label_name(l: PhoneLabel) -> &'static str {
    match l {
        PhoneLabel::Work => "Work",
        PhoneLabel::Personal => "Personal",
        PhoneLabel::Unsure => "Unsure",
        PhoneLabel::Unlabelled => "No label",
    }
}

/// The action id suffix for a phone label.
#[must_use]
pub const fn phone_label_key(l: PhoneLabel) -> &'static str {
    l.as_str()
}

const SELF_REPORTED_WATER: &str = "Self-reported. A change is saved as a correction.";

fn water_logged_card(entry: Option<Uuid>, ml: u32, view: &CoachView) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    let (logged, goal) = water_today(view);
    let mut b = Bubble {
        note: Some(SELF_REPORTED_WATER.to_owned()),
        continues: true,
        ..Bubble::default()
    };
    if entry.is_some() {
        b.title = goal.map_or_else(
            || format!("Logged {ml} ml, {logged} ml today"),
            |g| format!("Logged {ml} ml, {logged} / {g} ml"),
        );
        if goal.is_some_and(|g| logged >= g) {
            b.lines.push(GOAL_REACHED.to_owned());
        }
        b.actions = vec![
            act(action::UNDO, "Undo", Normal),
            act(action::DETAILS, "Details", Link),
        ];
    } else {
        b.title = format!("Removed {ml} ml");
        b.lines
            .push(format!("Today: {}.", water_total_words(logged, goal)));
        b.actions = vec![
            act(action::OK, "OK", Primary),
            act(action::DETAILS, "Details", Link),
        ];
    }
    b
}

fn amount(ml: u32) -> String {
    format!("{ml} ml")
}

fn water_panel_card(edit: Option<WaterEdit>, other_ml: u32, view: &CoachView) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    let (logged, goal) = water_today(view);
    let less = format!("\u{2212}{WATER_STEP_ML} ml");
    let more = format!("+{WATER_STEP_ML} ml");
    let mut lines = vec![water_total_words(logged, goal)];
    if goal.is_some_and(|g| logged >= g) {
        lines.push(GOAL_REACHED.to_owned());
    }
    let entries = &view.water_entries;
    if entries.is_empty() {
        lines.push("No entries yet today. That means nothing logged, not zero water.".to_owned());
    } else if entries.len() > PANEL_ENTRIES {
        lines.push(format!(
            "Showing the latest {PANEL_ENTRIES} of {} entries.",
            entries.len()
        ));
    }
    let mut rows: Vec<BubbleRow> = entries[entries.len().saturating_sub(PANEL_ENTRIES)..]
        .iter()
        .map(|e| {
            let when = e.time.format("%H:%M");
            match edit.filter(|x| x.entry == e.id) {
                Some(x) => BubbleRow {
                    text: format!("{when}, {} ml, change to {}", e.amount_ml, amount(x.ml)),
                    actions: vec![
                        act(action::EDIT_LESS, &less, Normal),
                        act(action::EDIT_MORE, &more, Normal),
                        act(action::EDIT_SAVE, "Save", Primary),
                        act(action::EDIT_CANCEL, "Cancel", Link),
                    ],
                },
                None => BubbleRow {
                    text: format!("{when}, {} ml", e.amount_ml),
                    actions: vec![
                        act(
                            &format!("{}{}", action::ENTRY_EDIT_PREFIX, e.id),
                            "Edit",
                            Link,
                        ),
                        act(
                            &format!("{}{}", action::ENTRY_REMOVE_PREFIX, e.id),
                            "Remove",
                            Link,
                        ),
                    ],
                },
            }
        })
        .collect();
    rows.push(BubbleRow {
        text: format!("Other amount: {}", amount(other_ml)),
        actions: vec![
            act(action::OTHER_LESS, &less, Normal),
            act(action::OTHER_MORE, &more, Normal),
            act(
                action::OTHER_ADD,
                &format!("Add {}", amount(other_ml)),
                Normal,
            ),
        ],
    });
    Bubble {
        title: "Water today".to_owned(),
        lines,
        progress: goal.map(|g| BubbleProgress {
            value: logged,
            max: g,
            label: format!("{logged} of {g} ml"),
        }),
        rows,
        actions: view
            .servings
            .ml()
            .iter()
            .map(|ml| {
                act(
                    &format!("{}{ml}", action::ADD_PREFIX),
                    &format!("+{ml} ml"),
                    if *ml == WATER_SERVING_ML {
                        Primary
                    } else {
                        Normal
                    },
                )
            })
            .collect(),
        note: Some(SELF_REPORTED_WATER.to_owned()),
        continues: true,
        ..Bubble::default()
    }
}

fn phone_note_card(entry: Option<Uuid>, at: NaiveTime, label: Option<PhoneLabel>) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    if entry.is_none() {
        return Bubble {
            title: "Phone note removed".to_owned(),
            actions: vec![act(action::OK, "OK", Primary)],
            continues: true,
            ..Bubble::default()
        };
    }
    let line = label.map_or_else(
        || "Optional: what was it?".to_owned(),
        |l| format!("Saved as {}.", phone_label_name(l).to_lowercase()),
    );
    let mut actions: Vec<_> = PHONE_LABELS
        .iter()
        .map(|l| {
            act(
                &format!("{}{}", action::LABEL_PREFIX, phone_label_key(*l)),
                phone_label_name(*l),
                if label == Some(*l) { Primary } else { Normal },
            )
        })
        .collect();
    actions.push(act(action::UNDO, "Undo", Link));
    Bubble {
        title: format!("Phone interruption noted, {}", at.format("%H:%M")),
        lines: vec![line],
        actions,
        note: Some("Self-reported. The app does not detect phone use.".to_owned()),
        continues: true,
        ..Bubble::default()
    }
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
                    "Water: {} ml behind your own pace for your time at the PC today.",
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
                    act(action::EDIT_PLAN, "Edit plan", Normal),
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
        CoachBubble::WaterLogged { entry, ml } => water_logged_card(entry, ml, view),
        CoachBubble::WaterPanel { edit, other_ml } => water_panel_card(edit, other_ml, view),
        CoachBubble::PhoneNote { entry, at, label } => phone_note_card(entry, at, label),
        CoachBubble::ReviewOffer { week, later } => {
            let mut actions = vec![act(
                &format!("{}{}", action::REVIEW_OPEN_PREFIX, week.format("%Y-%m-%d")),
                "Open",
                Primary,
            )];
            if later {
                actions.push(act(action::REVIEW_LATER, "Later", Normal));
            }
            actions.push(act(action::REVIEW_SKIP, "Skip this week", Link));
            card(
                "Your week in review is ready",
                vec!["About a minute. Optional.".to_owned()],
                actions,
            )
        }
        CoachBubble::MeditationInvite => {
            let mut actions: Vec<_> = MEDITATION_MINUTES
                .iter()
                .map(|m| {
                    act(
                        &format!("{}{m}", action::MEDITATE_PREFIX),
                        &format!("{m} min"),
                        if *m == meditation::DEFAULT_MINUTES {
                            Primary
                        } else {
                            Normal
                        },
                    )
                })
                .collect();
            actions.push(act(action::MEDITATE_NOT_TODAY, "Not today", Link));
            Bubble {
                note: meditation_note(),
                ..card(
                    meditation::INVITATION_TITLE,
                    vec![meditation::INVITATION_LINE.to_owned()],
                    actions,
                )
            }
        }
        CoachBubble::Meditation(m) => meditation_card(&m),
        CoachBubble::PostureCue => card(
            POSTURE_CUE_TITLE,
            vec![POSTURE_CUE_LINE.to_owned()],
            vec![
                act(action::OK, "OK", Primary),
                act(action::POSTURE_RECALIBRATE, "Recalibrate", Link),
            ],
        ),
    }
}

fn meditation_note() -> Option<String> {
    meditation::PROVISIONAL.then(|| meditation::PROVISIONAL_NOTICE.to_owned())
}

/// The meditation pause's card (wireframe B12): one part at a time, with Next, Pause or
/// Resume, Stop and the other version; then the last screen. No countdown is drawn (the
/// card would change every few seconds); the part number shows the way through.
fn meditation_card(m: &MeditationView) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    if m.done {
        return Bubble {
            title: meditation::DONE_TITLE.to_owned(),
            actions: vec![act(action::OK, "OK", Primary)],
            note: meditation_note(),
            ..Bubble::default()
        };
    }
    let parts = meditation::parts(m.version);
    let index = usize::try_from(m.part).unwrap_or(0).min(parts.len() - 1);
    let part = parts[index];
    let mut lines: Vec<String> = part.lines.iter().map(|l| (*l).to_owned()).collect();
    if m.paused {
        lines.push("Paused. Resume when you are ready.".to_owned());
    }
    let last = m.part + 1 >= MEDITATION_PARTS;
    Bubble {
        title: part.title.to_owned(),
        lines,
        progress: Some(BubbleProgress {
            value: m.part + 1,
            max: MEDITATION_PARTS,
            label: format!(
                "Part {} of {MEDITATION_PARTS} ({}-minute pause)",
                m.part + 1,
                m.minutes
            ),
        }),
        actions: vec![
            act(
                action::MEDITATE_NEXT,
                if last { "Finish" } else { "Next" },
                Primary,
            ),
            if m.paused {
                act(action::MEDITATE_RESUME, "Resume", Normal)
            } else {
                act(action::MEDITATE_PAUSE, "Pause", Normal)
            },
            act(action::MEDITATE_STOP, "Stop", Normal),
            act(
                action::MEDITATE_SWITCH,
                meditation::switch_label(m.version),
                Link,
            ),
        ],
        note: meditation_note(),
        continues: true,
        ..Bubble::default()
    }
}

/// The posture cue's title (slice V2-2).
pub const POSTURE_CUE_TITLE: &str = "Posture check";
/// The posture cue's line: a change from the user's own calibration, never a judgement
/// of their posture and not medical advice.
pub const POSTURE_CUE_LINE: &str = "For about a minute you have been sitting differently from your calibration. Settle back into a position that is comfortable for you, if you like.";

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
        continues: false,
    }
}

/// An entry that opens a card, so the keyboard panel stays open to show it.
fn menu_card(id: &str, label: &str) -> MenuAction {
    MenuAction {
        continues: true,
        ..menu(id, label, false)
    }
}

/// The engine's entries in the pet menu and keyboard panel (and the pause entries in the
/// tray) for a view (wireframe B1, B5, B6 and T1).
///
/// While a movement routine runs ([`CoachView::active_routine`]), its three result
/// entries replace "Today's movement" in the menu, the panel and the tray, whatever hold
/// the pet is showing.
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
    // Water (B1, B3): the serving buttons and the panel, with today's total against the
    // user's own goal. Each opens its card, which the keyboard panel then shows.
    for ml in view.servings.ml() {
        m.push(menu_card(
            &format!("{}{ml}", a::WATER_PREFIX),
            &format!("+{ml} ml water"),
        ));
    }
    let (logged, goal) = water_today(view);
    let total = goal.map_or_else(
        || format!("{logged} ml today"),
        |g| format!("{logged} / {g} ml"),
    );
    m.push(menu_card(a::WATER_PANEL, &format!("Water... ({total})")));

    // The engine's routine state, not the pet's look: a pause, off-hours, focus, meeting
    // or presentation shows its own state over a running routine, and the user must
    // still be able to finish or stop it without lifting the hold. The tray carries
    // these too, for when the pet is hidden. A guided routine counts as running too: these
    // entries answer it in one go, beside its own card or "Show movement guide" above.
    let movement_running = view
        .active_routine
        .is_some_and(|r| r.activity == Activity::Movement);
    if movement_running {
        m.push(menu(a::MOVEMENT_DONE, "Movement done", true));
        m.push(menu(a::MOVEMENT_PARTIAL, "Did part of it", true));
        m.push(menu(a::MOVEMENT_STOP, "Stopped, none done", true));
    } else {
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

    m.push(menu_card(a::PHONE_NOTE, "Phone interruption"));
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
