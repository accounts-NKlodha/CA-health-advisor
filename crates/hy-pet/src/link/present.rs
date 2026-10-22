//! Drawing the coaching engine's view on the pet shell: the pet's state and tag, the one
//! bubble with its actions, and the engine's entries in the pet menu, keyboard panel and
//! tray (wireframe sections 3 and 4).
//!
//! Every function here is pure. The engine decides what is shown; this module only words
//! it. Action ids are the constants in [`action`], which [`super::route`] turns back into
//! engine inputs, so a button can never send something the card did not offer.

use std::time::Duration;

use hy_core::coach::{
    Bubble as CoachBubble, Chip, ChipStatus, CoachView, PetMode, PomodoroPause, PomodoroPhase,
    PomodoroView, ReminderReason,
};
use hy_core::schema::{Activity, SessionState};
use uuid::Uuid;

use crate::shell::{ActionStyle, Bubble, BubbleAction, BubbleChip, MenuAction};
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

    // Pet menu, keyboard panel and tray (B1, B5, B6, T1).
    /// "Show waiting reminder" or "Show Pomodoro card".
    pub const OPEN_PENDING: &str = "open_pending";
    /// "+250 ml water".
    pub const WATER: &str = "water_250";
    /// "Today's movement".
    pub const MOVEMENT_START: &str = "movement_start";
    /// "Movement done".
    pub const MOVEMENT_DONE: &str = "movement_done";
    /// "Did part of it".
    pub const MOVEMENT_PARTIAL: &str = "movement_partial";
    /// "Stopped, none done".
    pub const MOVEMENT_STOP: &str = "movement_stop";
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
        }
    }

    fn key(self) -> String {
        match self {
            Self::Reminder { offer } => format!("reminder:{offer}"),
            Self::PomodoroBreak => "pomodoro-break".to_owned(),
            Self::PomodoroRoundDone => "pomodoro-round".to_owned(),
            Self::PomodoroPaused => "pomodoro-paused".to_owned(),
            Self::CheckIn => "check-in".to_owned(),
            Self::Lighter => "lighter".to_owned(),
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
        Some(match key {
            "pomodoro-break" => Self::PomodoroBreak,
            "pomodoro-round" => Self::PomodoroRoundDone,
            "pomodoro-paused" => Self::PomodoroPaused,
            "check-in" => Self::CheckIn,
            "lighter" => Self::Lighter,
            other => Self::Reminder {
                offer: Uuid::parse_str(other.strip_prefix("reminder:")?).ok()?,
            },
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
        id: String::new(),
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
    }
}

/// The shell bubble for the engine's bubble, without its id (the link numbers each show;
/// see [`BubbleKind::id`]).
#[must_use]
pub fn bubble(b: &CoachBubble, view: &CoachView) -> Bubble {
    use ActionStyle::{Link, Normal, Primary};
    let card = |title: &str, lines: Vec<String>, actions: Vec<BubbleAction>| Bubble {
        id: String::new(),
        title: title.to_owned(),
        lines,
        chips: Vec::new(),
        actions,
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
    }
}

/// Whether a click on the pet should reopen something the engine holds (a waiting
/// reminder, or a Pomodoro card the user has not answered) rather than open the menu.
#[must_use]
pub fn click_reopens(view: &CoachView) -> bool {
    view.visible && view.bubble.is_none() && (waiting(view) || pomodoro_card_pending(view))
}

const fn waiting(view: &CoachView) -> bool {
    matches!(view.pet, PetMode::Waiting(_))
}

fn pomodoro_card_pending(view: &CoachView) -> bool {
    let waiting_offer = [Activity::Movement, Activity::EyeRest, Activity::Water]
        .iter()
        .any(|a| view.chip(*a).status == ChipStatus::Waiting);
    !waiting_offer
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

    // The engine's routine state, not the pet's look: a pause, off-hours, focus, meeting
    // or presentation shows its own state over a running routine, and the user must
    // still be able to finish or stop it without lifting the hold. The tray carries
    // these too, for when the pet is hidden.
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
