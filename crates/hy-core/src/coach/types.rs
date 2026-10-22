//! The interface between the coaching engine and the shell (V1-1 pet window, tray and
//! bubbles).
//!
//! The shell sends [`Input`]s: what the user did and what Windows reported. After every
//! input the engine returns a [`CoachView`]: a complete, declarative description of what
//! the pet and its single bubble should show. The shell renders the view as it is; it never
//! decides on its own to show a reminder. Short-lived [`Notice`]s (for example "focus
//! ended") ride along for toasts and logging.
//!
//! The view has at most one [`Bubble`] by construction, so two reminders can never be
//! visible at once.

use std::time::Duration;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::PomodoroPreset;
use super::water::WaterPace;
use crate::schema::{Activity, RoutineOutcome, SessionState};

/// How long the user paused coaching for (wireframe B5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseLength {
    /// A number of minutes (30 or 60 in the menu).
    Minutes(u32),
    /// Until local midnight.
    RestOfToday,
    /// Until the user resumes.
    UntilResumed,
}

/// The user's answer to the one activity shown in a reminder (wireframe B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    /// "Already did it" (self-reported).
    Done,
    /// Water only: "Log N ml". Records the water entry and answers the reminder as done.
    LogWater {
        /// Amount, 1 to 2000 ml.
        ml: u32,
    },
    /// "Start movement" (opens the guide) or "Start 20 s eye rest".
    Start,
    /// "Snooze 10 min". Applies to this activity only and returns it once.
    Snooze,
    /// "Skip". Never counted as done.
    Skip,
    /// "Not now". Never counted as done.
    Dismiss,
}

/// Answer to the long-day check-in (wireframe B7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckInAnswer {
    /// "Yes, it was": the user marks today as deadline-heavy.
    Yes,
    /// "No".
    No,
    /// "Don't ask today".
    DontAskToday,
}

/// Something the shell reports. Every input is applied at the time the driver stamps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// The 5-second poll: time since the last keyboard or mouse input (GetLastInputInfo).
    Tick {
        /// Input idle time.
        idle: Duration,
    },
    /// Re-evaluate without any other change (after a settings change, or after water was
    /// logged from the panel). Idle time is projected from the last tick.
    Refresh,
    /// The session was locked.
    Lock,
    /// The session was unlocked.
    Unlock,
    /// The PC is about to sleep.
    Suspend,
    /// The PC woke up.
    Resume,
    /// Windows reported a presentation or full-screen app starting (`true`) or ending.
    PresentationChanged(bool),
    /// Experimental: camera or microphone in use (only used if the setting is on).
    MediaInUseChanged(bool),
    /// Tray "Show pet": shows the pet again during a presentation until it ends.
    ShowPet,
    /// The user answered the reminder `offer`.
    Respond {
        /// The offer being answered; stale offers are ignored.
        offer: Uuid,
        /// The answer.
        response: Response,
    },
    /// The bubble was closed without an answer (the x button or Escape).
    CloseBubble,
    /// The user clicked the pet while a reminder was waiting as a badge, or while a
    /// Pomodoro card was pending: reopen it. A Pomodoro card opened this way is shown even
    /// under a pause, off-hours, focus or meeting hold, because the user asked for it; a
    /// hold that starts later withdraws it.
    OpenPending,
    /// The movement guide was opened from the pet menu (not from a reminder).
    RoutineStarted,
    /// The movement guide finished. `offer` is the reminder it was started from, if any.
    RoutineFinished {
        /// The reminder that started the routine, if any.
        offer: Option<Uuid>,
        /// Done (all steps), partial (some), none (zero) or skipped (skip today).
        outcome: RoutineOutcome,
    },
    /// "Later" in a movement guide started from a reminder: snoozes movement.
    RoutineLater {
        /// The reminder that started the routine.
        offer: Uuid,
    },
    /// The user marks a break (a confirmed break; never recorded as movement).
    MarkBreak,
    /// Start quiet focus.
    StartFocus {
        /// 5 to 240 minutes.
        minutes: u32,
        /// Confirmed "replace the running Pomodoro".
        replace_pomodoro: bool,
    },
    /// End quiet focus early.
    EndFocus,
    /// Pause coaching.
    Pause(PauseLength),
    /// Resume coaching.
    ResumeCoaching,
    /// The manual "I'm in a meeting" toggle.
    SetMeeting(bool),
    /// Start a Pomodoro session (off until Start; wireframe B6).
    StartPomodoro {
        /// Preset.
        preset: PomodoroPreset,
        /// Confirmed "Replace quiet focus?".
        replace_focus: bool,
        /// Confirmed "Resume coaching for this session?".
        resume_coaching: bool,
    },
    /// Pause the Pomodoro timer.
    PausePomodoro,
    /// Resume the Pomodoro timer (also after a lock or sleep).
    ResumePomodoro,
    /// End the Pomodoro session.
    EndPomodoro,
    /// "Start break" on the break card.
    StartPomodoroBreak,
    /// "Skip break" (recorded as declined, not completed).
    SkipPomodoroBreak,
    /// "Start next round" (auto-start is off by default).
    NextPomodoroRound,
    /// Answer to the long-day check-in.
    CheckIn(CheckInAnswer),
    /// Answer to "Fewer reminders?" after the third dismissal of the day.
    LighterSchedule {
        /// "Use Light".
        accept: bool,
    },
}

impl Input {
    /// Inputs that are themselves keyboard or mouse input at the moment they arrive.
    pub(crate) const fn is_user_interaction(&self) -> bool {
        !matches!(
            self,
            Self::Tick { .. }
                | Self::Refresh
                | Self::Lock
                | Self::Unlock
                | Self::Suspend
                | Self::Resume
                | Self::PresentationChanged(_)
                | Self::MediaInUseChanged(_)
        )
    }
}

/// What happened to an input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputOutcome {
    /// Applied.
    Applied,
    /// Ignored: stale (an answer to a reminder that is no longer shown, a duplicate
    /// signal) or not applicable now. Nothing changed.
    Ignored(&'static str),
    /// Refused until the user confirms (wireframe B6 guards).
    NeedsConfirmation(Confirmation),
    /// Refused because a value is out of range.
    Invalid(&'static str),
}

/// A confirmation the shell must ask for before repeating the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    /// "Replace quiet focus?"
    ReplaceFocus,
    /// "Replace the running Pomodoro?"
    ReplacePomodoro,
    /// "Resume coaching for this session?"
    ResumeCoaching,
}

/// The status of one activity, shown on its chip in the reminder card (wireframe B2).
/// Each activity's status changes only through its own answers, its own timers and
/// tracker events (a confirmed break restarts the movement and eye-rest timers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChipStatus {
    /// Category switched off.
    Off,
    /// Water only: no goal set, so no pace.
    NoGoal,
    /// Not due.
    NotDue,
    /// Due; waits for its turn, the calm gap or the end of a quiet period.
    Due,
    /// Shown in the bubble now.
    Showing,
    /// Collapsed to the waiting badge.
    Waiting,
    /// Started from the reminder (movement guide or eye rest running).
    InProgress,
    /// Snoozed; returns once.
    Snoozed,
    /// Done (self-reported).
    Done,
    /// Movement routine partly done.
    Partial,
    /// Movement routine stopped with none recorded.
    NoneRecorded,
    /// Skipped (not done).
    Skipped,
    /// Dismissed with "Not now" (not done).
    Dismissed,
    /// Offered, no response (not done).
    NoResponse,
    /// Water only: the goal is reached; hydration reminders are paused.
    GoalReached,
}

/// One activity's chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    /// The activity.
    pub activity: Activity,
    /// Its status.
    pub status: ChipStatus,
    /// Time until a snooze ends, when snoozed.
    pub snoozed_for: Option<Duration>,
}

/// Why the one reminder is shown (its one-line reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderReason {
    /// "Movement: N min of work since your last break" (estimate).
    Movement {
        /// Work time counted since the last confirmed break or movement.
        worked: Duration,
    },
    /// "Eye rest: look away for 20 seconds".
    EyeRest {
        /// Screen time counted since the last eye rest or confirmed break.
        screen_time: Duration,
    },
    /// "Water: N ml behind your own pace (about M ml expected by now)".
    Water {
        /// Expected by now.
        expected_ml: u32,
        /// Logged today.
        logged_ml: u32,
        /// Behind by.
        behind_ml: u32,
    },
}

/// The Pomodoro phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PomodoroPhase {
    /// Work interval.
    Work,
    /// Work ended; the break card waits for Start break, Skip break or End.
    BreakOffered,
    /// Break interval.
    Break,
    /// Break ended (or skipped); "Start next round?" waits.
    RoundDone,
}

/// Why the Pomodoro timer is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PomodoroPause {
    /// The user pressed Pause.
    User,
    /// The PC was locked or slept; the user must resume (P: no catch-up).
    LockOrSleep,
    /// The user paused coaching.
    CoachingPaused,
}

/// The Pomodoro session, for the pet tag ("Work 24:59") and controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PomodoroView {
    /// Phase.
    pub phase: PomodoroPhase,
    /// Time left in a running phase.
    pub left: Duration,
    /// Paused, and why.
    pub paused: Option<PomodoroPause>,
    /// Round number, from 1.
    pub round: u32,
}

/// The single bubble beside the pet.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bubble {
    /// The reminder card: one activity with its own actions, plus the three chips.
    Reminder {
        /// Offer id: echo it back in [`Input::Respond`].
        offer: Uuid,
        /// The activity offered.
        activity: Activity,
        /// Its reason line.
        reason: ReminderReason,
    },
    /// "Break" card of a Pomodoro session; mentions water only if genuinely behind pace.
    ///
    /// The three Pomodoro cards never open by themselves under a hold above the Pomodoro
    /// (pause, lock or sleep, off-hours, focus, presentation, meeting), and a hold that
    /// starts withdraws an open one, even one the user opened during another hold. They
    /// are not reopened when the hold ends; the phase is
    /// kept in [`CoachView::pomodoro`] and the user can open the card with
    /// [`Input::OpenPending`].
    PomodoroBreak {
        /// Break length.
        length: Duration,
        /// Present only when behind pace for the time of day.
        water_behind: Option<WaterPace>,
    },
    /// "Break over. Start another round?"
    PomodoroRoundDone,
    /// "Pomodoro paused. Your PC was locked. Resume or End."
    PomodoroPausedByLock,
    /// The optional long-day check-in.
    LongDayCheckIn {
        /// Hours of estimated PC activity today (labelled an estimate).
        hours: u8,
    },
    /// "Fewer reminders?" after the third dismissal today.
    LighterSchedule,
}

/// Why the pet is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenReason {
    /// Presentation or full screen (owner approved default, 21 Oct).
    Presentation,
}

/// The pet's state, badge and tag (wireframe section 4). The shell maps it to art,
/// an icon badge and a text tag, so no state depends on colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetMode {
    /// Hidden; only the tray stays.
    Hidden(HiddenReason),
    /// The session is locked or the PC asleep.
    Away,
    /// Coaching paused ("Paused 1 h").
    Paused {
        /// Time left, if the pause has a fixed length.
        left: Option<Duration>,
        /// Paused for the rest of the day.
        rest_of_today: bool,
    },
    /// Outside work hours ("Off hours").
    OffHours,
    /// Quiet focus ("Focus 60 min").
    Focus {
        /// Time left.
        left: Duration,
    },
    /// Manual meeting or the experimental media hint ("Meeting - paused").
    Meeting,
    /// Pomodoro running ("Work 24:59").
    Pomodoro(PomodoroView),
    /// A movement routine or eye rest is in progress.
    Exercise,
    /// The reminder card is open.
    Reminder(Activity),
    /// "1 reminder waiting" badge.
    Waiting(Activity),
    /// A check-in or offer is open ("!").
    Nudge,
    /// An activity is snoozed ("zz").
    Snoozed(Activity),
    /// Default.
    Idle,
}

/// Which precedence level currently holds reminders back (architecture 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hold {
    /// Level 1: pause, lock, sleep or off-hours.
    PauseLockOffHours,
    /// Level 2: focus, presentation or meeting.
    FocusPresentationMeeting,
    /// Level 3: an active exercise, break or Pomodoro.
    ExerciseOrPomodoro,
}

/// Everything the shell needs to draw the pet and its bubble.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoachView {
    /// Whether the pet window is shown at all.
    pub visible: bool,
    /// The pet's state.
    pub pet: PetMode,
    /// The single bubble, if any. Always `None` while the pet is hidden.
    pub bubble: Option<Bubble>,
    /// Movement, eye rest and water, in that order.
    pub chips: [Chip; 3],
    /// The tracker's session state (architecture 4.3).
    pub session: SessionState,
    /// The highest precedence level holding reminders back, if any.
    pub hold: Option<Hold>,
    /// Today's water pace, if a goal is set.
    pub water: Option<WaterPace>,
    /// The Pomodoro session, if one exists.
    pub pomodoro: Option<PomodoroView>,
    /// Time left in quiet focus.
    pub focus_left: Option<Duration>,
    /// The manual meeting toggle.
    pub meeting: bool,
}

impl CoachView {
    /// The chip for `activity`.
    #[must_use]
    pub fn chip(&self, activity: Activity) -> Chip {
        let i = match activity {
            Activity::Movement => 0,
            Activity::EyeRest => 1,
            Activity::Water => 2,
        };
        self.chips[i]
    }

    /// The reminder offer in the bubble, if the bubble is a reminder.
    #[must_use]
    pub fn reminder(&self) -> Option<(Uuid, Activity)> {
        match self.bubble {
            Some(Bubble::Reminder {
                offer, activity, ..
            }) => Some((offer, activity)),
            _ => None,
        }
    }
}

/// Why focus ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusEnd {
    /// Its time ran out (silently if the PC was locked).
    Expired,
    /// The user ended it.
    Ended,
    /// A Pomodoro replaced it.
    Replaced,
}

/// A short-lived message for the shell (toast, celebration, settings change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    /// A reminder was shown.
    ReminderShown {
        /// Offer id.
        offer: Uuid,
        /// Activity.
        activity: Activity,
    },
    /// A reminder was answered or closed; the activity's outcome is final.
    ReminderResolved {
        /// Offer id.
        offer: Uuid,
        /// Activity.
        activity: Activity,
        /// Final chip status.
        status: ChipStatus,
    },
    /// A done answer: a brief, optional celebration.
    Celebrate(Activity),
    /// Focus ended. Nothing is replayed.
    FocusEnded {
        /// Why.
        reason: FocusEnd,
        /// The PC was locked or asleep, so the end is silent.
        silent: bool,
    },
    /// The pause ended on its own.
    PauseEnded,
    /// A Pomodoro work interval finished (one round counted).
    PomodoroRoundCounted,
    /// The user accepted the Light schedule; the driver saves the setting.
    UseIntensity(super::config::Intensity),
    /// The user confirmed today as deadline-heavy (for the recovery-day flow, V1-3).
    DeadlineDayConfirmed(NaiveDate),
    /// The saved engine state could not be read and coaching restarted from defaults
    /// (stored events and tallies are untouched).
    StateReset,
    /// Something held reminders back and has ended. Nothing is replayed: activities that
    /// fell due meanwhile wait for their next cycle.
    HoldEnded(Hold),
}
