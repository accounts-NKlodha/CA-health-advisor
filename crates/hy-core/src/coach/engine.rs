//! The coaching engine: one deterministic state machine for the activity tracker
//! (architecture 4.3), the one-at-a-time reminder (wireframe B2, prototype rev 3),
//! suppression and precedence, quiet focus, pause, meeting, presentation, the optional
//! long-day check-in and the user-started Pomodoro.
//!
//! # Time
//!
//! The engine is pure: it never reads a clock. Each [`Engine::step`] receives a
//! [`Context`] with the wall time (for the local date, work hours and the water pace)
//! and the monotonic time elapsed since the previous step. Every timer (movement and
//! eye-rest work time, snooze, calm gap, grace, collapse, focus, Pomodoro) runs on
//! *engine time*, the sum of those elapsed values, which only moves forward and is saved
//! with the state. Setting the wall clock back or forward therefore never fires a burst of
//! reminders and never cancels one; it only changes what "today" and "work hours" mean,
//! which the engine re-evaluates at once.
//!
//! # Rules implemented here (P: PRD or approved prototype; D: chosen here)
//!
//! - One reminder at a time among movement, eye rest and water. Each is answered on its
//!   own; an answer changes only that activity (P).
//! - After any answer, a 5-minute calm gap before the next reminder (P). Snooze 10 min,
//!   returns once (P). Closing without answering, or 10 min without an answer, collapses
//!   the reminder to a waiting badge, not another pop-up (P); the badge closes 20 min
//!   after the reminder first appeared and is logged as "offered, no response" (P).
//! - Movement routine results: done and partial restart the movement timer; none keeps the
//!   calm gap and the next due time but does not restart the timer; skip-today counts as a
//!   skip (P). All four reschedule movement.
//! - Water is offered only while behind the user's own pace by at least 100 ml; a reached
//!   goal pauses hydration reminders (P). The pace follows the time the user was at the PC
//!   inside the work window (unlocked, awake, app running), not the clock (DEC13).
//! - Precedence (architecture 4.3): pause, lock, sleep, off-hours; then focus, presentation,
//!   meeting; then an active exercise, break or Pomodoro; then one reminder. Anything held
//!   back is never replayed: when a pause, off-hours, focus, meeting or presentation ends,
//!   activities that fell due meanwhile wait for their next cycle (P, owner approved
//!   default for presentations). After unlock or wake there is a 2-minute grace period and
//!   then at most one reminder (P).
//! - A reminder that is showing or waiting when something starts holding reminders back is
//!   withdrawn and logged as "offered, no response" (D).
//! - The guided movement routine ([`guide`], slice V1-3) is the one bubble while it runs.
//!   Each movement's answer is a `movement.step` event and the result a `movement.session`
//!   event, in the same commit as the state. An immediate Stop, or every movement skipped,
//!   is "none recorded" and does not restart the movement timer (P).
//! - The deadline day and the recovery-day plan ([`recovery`], slice V1-3) use the same
//!   single bubble, after the reminders and with their calm gap.
//! - The water confirmation, the water panel and the phone note ([`note`], V1-3 follow-up)
//!   answer the user's own action in the same single bubble; while one is open nothing
//!   opens on its own.
//! - The weekly review offer ([`review`], slice V1-4) opens once a week at the user's
//!   review time, like the recovery cards that open on their own.

mod guide;
mod meditation;
mod note;
mod posture;
mod recovery;
mod review;

pub use posture::{CUE_BUSY, CUE_COOLDOWN, CUE_FOCUS, CUE_HELD, POSTURE_CUE_COOLDOWN};

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::{Builder, Uuid};

use super::config::{
    ACTIVE_WITHIN, BADGE_EXPIRES_AFTER, CALM_GAP, COLLAPSE_AFTER, CONFIRMED_BREAK_AFTER,
    CoachConfig, DISMISSALS_FOR_LIGHTER, EYE_REST_LENGTH, FOCUS_MINUTES, GRACE_AFTER_WAKE,
    Intensity, LONG_QUIET_AFTER, PAUSE_MINUTES, ROUTINE_TIMEOUT, SNOOZE,
};
use super::recovery::{CARD_CLOSES_AFTER, RecoveryPlan};
use super::types::{
    ActiveRoutine, Bubble, Chip, ChipStatus, CoachView, Confirmation, FocusEnd, HiddenReason, Hold,
    Input, InputOutcome, Notice, PauseLength, PetMode, PomodoroPause, PomodoroPhase, PomodoroView,
    ReminderReason, Response,
};
use super::water::{Servings, WaterEntry, WaterPace, WaterSnapshot, pace};
use crate::camera::HoldSources;
use crate::schema::{
    Activity, EventPayload, MAX_WATER_ENTRY_ML, MeasurementStatus, NewEvent, ReminderOutcome,
    RoutineOutcome, SessionState,
};
use crate::time::{UserZone, WorkSchedule};

/// Version of the saved [`EngineState`] layout.
pub const STATE_FORMAT: u32 = 1;

/// How many local dates of counters the state keeps (older ones live in the store).
const DAYS_KEPT: usize = 7;

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn dur(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

const fn idx(a: Activity) -> usize {
    match a {
        Activity::Movement => 0,
        Activity::EyeRest => 1,
        Activity::Water => 2,
    }
}

const ORDER: [Activity; 3] = [Activity::Movement, Activity::EyeRest, Activity::Water];

/// Everything the engine needs to know about the outside world for one step.
#[derive(Debug, Clone)]
pub struct Context {
    /// Current wall time (UTC).
    pub wall: DateTime<Utc>,
    /// Monotonic time elapsed since the previous step (for the first step after a restart,
    /// the time the app was not running).
    pub elapsed: Duration,
    /// The user's time zone.
    pub zone: UserZone,
    /// The user's work hours.
    pub schedule: WorkSchedule,
    /// Today's water total and goal.
    pub water: WaterSnapshot,
    /// Today's water entries as they stand, oldest first (for the water panel).
    pub water_entries: Vec<WaterEntry>,
    /// The user's coaching settings.
    pub config: CoachConfig,
    /// The user's serving buttons (wireframe W5), for the water panel and the pet menu.
    pub servings: Servings,
}

/// Per local date tallies (architecture 4.3 "Reported as"). Every duration is an estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DayTally {
    /// Active or quiet PC time at any hour (the long-day check-in estimate), ms.
    pub pc_active_ms: u64,
    /// Active time inside work hours, not paused, ms.
    pub work_active_ms: u64,
    /// Quiet time inside work hours counted as work (estimated), ms.
    pub work_quiet_ms: u64,
    /// Long quiet inside work hours: unknown, ms.
    pub unknown_ms: u64,
    /// Locked or asleep (short locks and confirmed breaks), ms.
    pub away_ms: u64,
    /// Confirmed breaks (lock or sleep of 2 min or more, or a break the user marked).
    pub confirmed_breaks: u32,
    /// Longest continuous block of work time without a confirmed break or long quiet, ms.
    pub longest_block_ms: u64,
    /// Quiet focus sessions started.
    pub focus_sessions: u32,
    /// Pomodoro work intervals completed.
    pub pomodoro_rounds: u32,
    /// Screen time: at the PC (the app running, the session unlocked, the PC awake) at
    /// any hour, paused or not, however long without input, ms (DEC13).
    #[serde(default)]
    pub screen_ms: u64,
    /// At the PC inside the work window that starts on this date, ms. The water pace
    /// follows it (DEC13).
    #[serde(default)]
    pub window_screen_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DayState {
    tally: DayTally,
    dismissals: u32,
    lighter_offered: bool,
    check_in_done: bool,
    /// The user marked this date as deadline-heavy.
    #[serde(default)]
    deadline: bool,
    /// The recovery day's morning card was shown.
    #[serde(default)]
    recovery_morning: bool,
    /// Walk suggestion times used up today (shown, or passed while held).
    #[serde(default)]
    walk_slots: u32,
    /// The recovery day's end-of-day question was asked.
    #[serde(default)]
    reflection_asked: bool,
}

/// One activity's scheduling state. Exposed read-only for tests and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityState {
    /// Movement and eye rest: work or screen time counted since the last reset, ms.
    pub accum_ms: u64,
    /// Movement and eye rest: due once `accum_ms` reaches `base_ms` plus the interval.
    pub base_ms: u64,
    /// Water: engine time from which the next offer may come.
    pub ready_at: u64,
    /// A snooze returns the activity once at this engine time.
    pub snooze_until: Option<u64>,
    /// Engine time at which it became due (orders several due activities).
    pub due_since: Option<u64>,
    /// The last outcome today, shown on the chip until the activity is due again.
    pub last: Option<ChipStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OfferPhase {
    Showing { since: u64 },
    Waiting,
    InProgress { since: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Offer {
    id: Uuid,
    seq: u64,
    activity: Activity,
    phase: OfferPhase,
    first_shown: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Prompt {
    CheckIn,
    Lighter,
    DeadlineDay,
    RecoveryInvite,
    RecoveryDates,
    RecoveryPlanned,
    RecoveryMorning,
    RecoveryWalk,
    RecoveryReflection,
    ReviewOffer,
    MeditationInvite,
    PostureCue,
}

impl Prompt {
    /// Recovery cards and the weekly review offer close quietly when left unanswered.
    const fn is_recovery_card(self) -> bool {
        !matches!(self, Self::CheckIn | Self::Lighter | Self::DeadlineDay)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PauseUntil {
    At(u64),
    EndOfDay(NaiveDate),
    Resumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Away {
    since: u64,
    locked: bool,
    asleep: bool,
    confirmed: bool,
}

/// The holds above the Pomodoro in force at one moment, one bit for each source (pause,
/// lock, sleep, off-hours, focus, presentation, meeting). A Pomodoro card the user opens
/// under a hold remembers this set; the card is withdrawn as soon as the set gains any
/// source it did not have, so the exception covers only the holds the user saw when they
/// asked for the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct HoldSet(u8);

impl HoldSet {
    const EMPTY: Self = Self(0);
    const PAUSE: u8 = 1;
    const LOCK: u8 = 1 << 1;
    const SLEEP: u8 = 1 << 2;
    /// Away but neither locked nor asleep (only between a wake and its away ending).
    const AWAY: u8 = 1 << 3;
    const OFF_HOURS: u8 = 1 << 4;
    const FOCUS: u8 = 1 << 5;
    const PRESENTING: u8 = 1 << 6;
    const MEETING: u8 = 1 << 7;

    const fn with(self, bit: u8, on: bool) -> Self {
        if on { Self(self.0 | bit) } else { self }
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every source in `self` is also in `other`.
    const fn within(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pomodoro {
    work_ms: u64,
    break_ms: u64,
    round: u32,
    phase: PomodoroPhase,
    left_ms: u64,
    paused: Option<PomodoroPause>,
    card_open: bool,
    /// The holds above the Pomodoro in force when the user opened the open card (from the
    /// pet menu or the panel); empty for a card that opened on its own or with no hold.
    /// Such a card stays while the holds in force are among these; it is withdrawn when a
    /// hold starts that was not among them (a new lock, sleep or pause, a stronger hold,
    /// or one that ended and began again). Any other card is withdrawn when a hold starts.
    #[serde(default)]
    card_opened_under: HoldSet,
}

/// The engine's complete saved state. Opaque; serialised by the driver into the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineState {
    format: u32,
    salt: u64,
    offer_seq: u64,
    now: u64,
    idle_ms: u64,
    away: Option<Away>,
    acts: [ActivityState; 3],
    offer: Option<Offer>,
    gap_until: u64,
    grace_until: u64,
    prompt: Option<Prompt>,
    focus_until: Option<u64>,
    pause: Option<PauseUntil>,
    meeting: Option<NaiveDate>,
    presenting: bool,
    show_override: bool,
    media_in_use: bool,
    exercise_since: Option<u64>,
    pomodoro: Option<Pomodoro>,
    quiet_hold: Option<Hold>,
    work_date: Option<NaiveDate>,
    block_ms: u64,
    today: Option<NaiveDate>,
    days: BTreeMap<NaiveDate, DayState>,
    /// The guided movement routine, open or waiting behind the pet.
    #[serde(default)]
    guide: Option<guide::Guide>,
    /// Guided routines started (with `salt`, makes each routine's id unique).
    #[serde(default)]
    guide_seq: u64,
    /// For a card the user asked for (the "Deadline day" chain), the holds in force when
    /// it opened: it stays while only those hold. `None` for a card that opened on its own.
    #[serde(default)]
    prompt_opened_under: Option<HoldSet>,
    /// Engine time the open card appeared.
    #[serde(default)]
    prompt_since: u64,
    /// The one pending recovery-day plan.
    #[serde(default)]
    plan: Option<RecoveryPlan>,
    /// The water confirmation, the water panel or the phone note, when open.
    #[serde(default)]
    note: Option<note::NoteCard>,
    /// The weekly review offer (slice V1-4).
    #[serde(default)]
    review: review::ReviewOffer,
    /// The meditation invitation and pause (slice V2-4).
    #[serde(default)]
    meditation: meditation::Meditation,
    /// The posture cue's cooldown (slice V2-2).
    #[serde(default)]
    posture: posture::PostureCue,
}

/// What one step produced.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutput {
    /// What happened to the input.
    pub outcome: InputOutcome,
    /// The view to render.
    pub view: CoachView,
    /// Short-lived messages.
    pub notices: Vec<Notice>,
    /// Events to append in the same transaction as the new state.
    pub events: Vec<NewEvent>,
    /// Local dates whose tallies changed in this step.
    pub touched_days: Vec<(NaiveDate, DayTally)>,
    /// Something other than accumulated time changed (the driver saves at once).
    pub significant: bool,
}

#[derive(Debug, Default)]
struct Out {
    notices: Vec<Notice>,
    events: Vec<NewEvent>,
    touched: Vec<NaiveDate>,
    significant: bool,
    extra_water_ml: u32,
}

impl Out {
    fn touch(&mut self, d: NaiveDate) {
        if !self.touched.contains(&d) {
            self.touched.push(d);
        }
    }
}

/// The coaching engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    s: EngineState,
}

impl Engine {
    /// A fresh engine. `salt` makes this installation's reminder ids unique; the driver
    /// draws it from the operating system's random source once and saves it.
    #[must_use]
    pub fn new(salt: u64) -> Self {
        Self {
            s: EngineState {
                format: STATE_FORMAT,
                salt,
                offer_seq: 0,
                now: 0,
                idle_ms: 0,
                away: None,
                acts: [ActivityState::default(); 3],
                offer: None,
                gap_until: 0,
                grace_until: 0,
                prompt: None,
                focus_until: None,
                pause: None,
                meeting: None,
                presenting: false,
                show_override: false,
                media_in_use: false,
                exercise_since: None,
                pomodoro: None,
                quiet_hold: None,
                work_date: None,
                block_ms: 0,
                today: None,
                days: BTreeMap::new(),
                guide: None,
                guide_seq: 0,
                prompt_opened_under: None,
                prompt_since: 0,
                plan: None,
                note: None,
                review: review::ReviewOffer::default(),
                meditation: meditation::Meditation::default(),
                posture: posture::PostureCue::default(),
            },
        }
    }

    /// A fresh engine whose engine time continues from `engine_time` (used when a saved
    /// state cannot be read: engine time never goes backwards).
    #[must_use]
    pub fn new_continuing(salt: u64, engine_time: Duration) -> Self {
        let mut e = Self::new(salt);
        e.s.now = ms(engine_time);
        e
    }

    /// Rebuild from saved state.
    ///
    /// # Errors
    /// [`crate::Error::Invalid`] if the state was written by an unknown format.
    pub fn from_state(state: EngineState) -> crate::Result<Self> {
        if state.format != STATE_FORMAT {
            return Err(crate::Error::invalid(format!(
                "coach state format {} is not supported",
                state.format
            )));
        }
        Ok(Self { s: state })
    }

    /// The state to save.
    #[must_use]
    pub fn state(&self) -> &EngineState {
        &self.s
    }

    /// Engine time: monotonic milliseconds since the engine was created, including time
    /// the app was not running.
    #[must_use]
    pub fn engine_time(&self) -> Duration {
        dur(self.s.now)
    }

    /// One activity's scheduling state (read-only).
    #[must_use]
    pub fn activity(&self, a: Activity) -> &ActivityState {
        &self.s.acts[idx(a)]
    }

    /// Today's tally, if any time was counted today.
    #[must_use]
    pub fn tally(&self, date: NaiveDate) -> Option<DayTally> {
        self.s.days.get(&date).map(|d| d.tally)
    }

    /// Reminder dismissals and skips counted for `date`.
    #[must_use]
    pub fn dismissals(&self, date: NaiveDate) -> u32 {
        self.s.days.get(&date).map_or(0, |d| d.dismissals)
    }

    /// The one recovery-day plan kept (it may be for a past date until the day changes).
    #[must_use]
    pub fn recovery_plan(&self) -> Option<RecoveryPlan> {
        self.s.plan
    }

    /// The view for the current state, without advancing time.
    #[must_use]
    pub fn view(&self, ctx: &Context) -> CoachView {
        self.build_view(ctx, 0)
    }

    /// Apply the app's start after a restart: the time it was not running counts like a
    /// sleep of `ctx.elapsed` (a confirmed break if 2 minutes or more), a reminder that
    /// was open is closed as "offered, no response", a routine that was running is
    /// abandoned without a record, a running Pomodoro waits for Resume, and the usual
    /// grace period follows.
    pub fn restart(&mut self, ctx: &Context) -> StepOutput {
        let mut out = Out {
            significant: true,
            ..Out::default()
        };
        // Close what the previous run left open, at the moment it stopped. A routine in the
        // guide keeps the movements already answered.
        self.interrupt_guide(ctx, &mut out);
        self.interrupt_meditation(ctx, &mut out);
        if let Some(o) = self.s.offer {
            match o.phase {
                OfferPhase::Showing { .. } | OfferPhase::Waiting => {
                    self.resolve_no_response(ctx, &mut out);
                }
                OfferPhase::InProgress { .. } => self.abandon_in_progress(&mut out),
            }
        }
        self.clear_prompt();
        self.s.note = None;
        self.s.exercise_since = None;
        let since = self.s.now;
        let was_away = self.s.away;
        self.s.away = Some(Away {
            since: was_away.map_or(since, |a| a.since),
            locked: false,
            asleep: true,
            confirmed: was_away.is_some_and(|a| a.confirmed),
        });
        self.pause_pomodoro_for_lock();
        // The time closed counts as a sleep, a new hold: it withdraws a card the user
        // opened under other holds before the app stopped.
        self.withdraw_held_pomodoro_card(ctx, &mut out);
        self.advance(ctx, &mut out, self.s.idle_ms);
        self.s.away = self.s.away.map(|a| Away { asleep: false, ..a });
        self.end_away(ctx, &mut out);
        self.settle(ctx, &mut out);
        self.finish(ctx, InputOutcome::Applied, out)
    }

    /// Advance time by `ctx.elapsed`, then apply `input`.
    pub fn step(&mut self, ctx: &Context, input: Input) -> StepOutput {
        let mut out = Out::default();
        let projected = self.s.idle_ms.saturating_add(ms(ctx.elapsed));
        let idle_end = match &input {
            Input::Tick { idle } => ms(*idle),
            _ => projected,
        };
        self.advance(ctx, &mut out, idle_end);
        if input.is_user_interaction() {
            self.s.idle_ms = 0;
        }
        let is_tick = matches!(input, Input::Tick { .. } | Input::Refresh);
        let outcome = self.apply(ctx, input, &mut out);
        if outcome == InputOutcome::Applied && !is_tick {
            out.significant = true;
        }
        self.settle(ctx, &mut out);
        self.finish(ctx, outcome, out)
    }

    fn finish(&self, ctx: &Context, outcome: InputOutcome, out: Out) -> StepOutput {
        let touched_days = out
            .touched
            .iter()
            .filter_map(|d| self.s.days.get(d).map(|s| (*d, s.tally)))
            .collect();
        StepOutput {
            outcome,
            view: self.build_view(ctx, out.extra_water_ml),
            notices: out.notices,
            events: out.events,
            touched_days,
            significant: out.significant,
        }
    }

    // ------------------------------------------------------------------ queries

    fn off_hours(&self, ctx: &Context) -> bool {
        !ctx.schedule.is_work_time(ctx.wall, &ctx.zone)
    }

    fn presenting_effective(&self, ctx: &Context) -> bool {
        self.s.presenting && ctx.config.hide_during_presentation
    }

    fn visible(&self, ctx: &Context) -> bool {
        !(self.presenting_effective(ctx) && !self.s.show_override)
    }

    fn meeting_on(&self, ctx: &Context) -> bool {
        self.s.meeting.is_some() || (self.s.media_in_use && ctx.config.media_in_use_hint)
    }

    fn level1(&self, ctx: &Context) -> bool {
        self.s.pause.is_some() || self.s.away.is_some() || self.off_hours(ctx)
    }

    fn level2(&self, ctx: &Context) -> bool {
        self.s.focus_until.is_some() || self.presenting_effective(ctx) || self.meeting_on(ctx)
    }

    fn level3(&self) -> bool {
        self.s.exercise_since.is_some()
            || self.meditation_running()
            || self.s.pomodoro.is_some_and(|p| {
                p.card_open
                    || (p.paused.is_none()
                        && matches!(p.phase, PomodoroPhase::Work | PomodoroPhase::Break))
            })
            || self
                .s
                .offer
                .is_some_and(|o| matches!(o.phase, OfferPhase::InProgress { .. }))
    }

    /// A hold above the Pomodoro (pause, lock, sleep, off-hours, focus, presentation or
    /// meeting) is in force. Under it no Pomodoro card opens on its own and an open card
    /// is withdrawn, unless the user opened it while every hold now in force already was
    /// (see [`HoldSet`]). The Pomodoro's phase is kept, so the panel and the pet menu
    /// still offer it.
    fn pomodoro_card_held(&self, ctx: &Context) -> bool {
        !self.holds_above_pomodoro(ctx).is_empty()
    }

    /// Each source of a hold above the Pomodoro now in force (non-empty exactly when
    /// level 1 or level 2 holds).
    fn holds_above_pomodoro(&self, ctx: &Context) -> HoldSet {
        let away = self.s.away;
        HoldSet::EMPTY
            .with(HoldSet::PAUSE, self.s.pause.is_some())
            .with(HoldSet::LOCK, away.is_some_and(|a| a.locked))
            .with(HoldSet::SLEEP, away.is_some_and(|a| a.asleep))
            .with(HoldSet::AWAY, away.is_some_and(|a| !a.locked && !a.asleep))
            .with(HoldSet::OFF_HOURS, self.off_hours(ctx))
            .with(HoldSet::FOCUS, self.s.focus_until.is_some())
            .with(HoldSet::PRESENTING, self.presenting_effective(ctx))
            .with(HoldSet::MEETING, self.meeting_on(ctx))
    }

    /// The highest precedence level holding reminders back.
    fn hold(&self, ctx: &Context) -> Option<Hold> {
        if self.level1(ctx) {
            Some(Hold::PauseLockOffHours)
        } else if self.level2(ctx) {
            Some(Hold::FocusPresentationMeeting)
        } else if self.level3() {
            Some(Hold::ExerciseOrPomodoro)
        } else {
            None
        }
    }

    /// Holds that are "quiet periods": when they end, nothing is replayed. Lock and sleep
    /// are handled by the grace period instead; exercise and Pomodoro by the calm gap.
    fn quiet_hold(&self, ctx: &Context) -> Option<Hold> {
        if self.s.pause.is_some() || self.off_hours(ctx) {
            Some(Hold::PauseLockOffHours)
        } else if self.level2(ctx) {
            Some(Hold::FocusPresentationMeeting)
        } else {
            None
        }
    }

    /// The check-in may appear outside work hours (a long day often runs late), but not
    /// while paused, away, in focus, a meeting, a presentation, an exercise or a Pomodoro.
    fn prompt_blocked(&self, ctx: &Context) -> bool {
        self.s.pause.is_some() || self.s.away.is_some() || self.level2(ctx) || self.level3()
    }

    fn water_pace(&self, ctx: &Context, extra_ml: u32) -> Option<WaterPace> {
        let snap = WaterSnapshot {
            logged_ml: ctx.water.logged_ml.saturating_add(extra_ml),
            goal_ml: ctx.water.goal_ml,
        };
        pace(snap, ctx.wall, &ctx.schedule, &ctx.zone, |d| {
            dur(self.s.days.get(&d).map_or(0, |s| s.tally.window_screen_ms))
        })
    }

    fn is_due(&self, ctx: &Context, a: Activity) -> bool {
        if !ctx.config.categories.allows(a) {
            return false;
        }
        let st = &self.s.acts[idx(a)];
        let cycle_or_snooze = |cycle_due: bool| match st.snooze_until {
            Some(u) => self.s.now >= u,
            None => cycle_due,
        };
        match a {
            Activity::Movement | Activity::EyeRest => {
                cycle_or_snooze(st.accum_ms >= st.base_ms.saturating_add(ms(self.every(ctx, a))))
            }
            Activity::Water => {
                self.water_pace(ctx, 0).is_some_and(|p| p.is_behind())
                    && cycle_or_snooze(self.s.now >= st.ready_at)
            }
        }
    }

    fn today_state(&mut self, ctx: &Context) -> &mut DayState {
        let d = ctx.zone.local_date(ctx.wall);
        self.s.days.entry(d).or_default()
    }

    // ------------------------------------------------------------------ advance

    fn advance(&mut self, ctx: &Context, out: &mut Out, idle_end: u64) {
        let e = ms(ctx.elapsed);
        self.s.now = self.s.now.saturating_add(e);
        self.roll_day(ctx, out);
        self.account(ctx, out, e, idle_end);
        self.s.idle_ms = idle_end;
        self.run_timers(ctx, out, e);
    }

    /// Day boundary: chips start fresh, the meeting toggle and a rest-of-today pause end
    /// when the local date moves past them, and old counters are dropped.
    fn roll_day(&mut self, ctx: &Context, out: &mut Out) {
        let today = ctx.zone.local_date(ctx.wall);
        if self.s.today == Some(today) {
            return;
        }
        out.significant = true;
        self.s.today = Some(today);
        for a in &mut self.s.acts {
            a.last = None;
        }
        if self.s.meeting.is_some_and(|d| d < today) {
            self.s.meeting = None;
        }
        if matches!(self.s.pause, Some(PauseUntil::EndOfDay(d)) if d < today) {
            self.s.pause = None;
            out.notices.push(Notice::PauseEnded);
            self.resume_pomodoro_after_coaching_pause();
        }
        if self.s.plan.is_some_and(|p| p.date < today) {
            // The recovery day has passed; nothing is owed and nothing is carried over.
            self.s.plan = None;
        }
        self.s.days.entry(today).or_default();
        while self.s.days.len() > DAYS_KEPT {
            // Keep the newest dates; with a clock set back, "today" may be older than
            // some kept dates, so never drop today itself.
            let Some(oldest) = self.s.days.keys().copied().find(|d| *d != today) else {
                break;
            };
            self.s.days.remove(&oldest);
        }
    }

    /// Classify the interval that just passed and count it (tracker, architecture 4.3).
    fn account(&mut self, ctx: &Context, out: &mut Out, e: u64, idle_end: u64) {
        if e == 0 {
            return;
        }
        let end = ctx.wall;
        let start = end - chrono::Duration::milliseconds(i64::try_from(e).unwrap_or(i64::MAX));
        if let Some(mut away) = self.s.away {
            for seg in ctx.zone.split_by_local_day(start, end) {
                let len = ms_between(seg.start, seg.end);
                self.s.days.entry(seg.date).or_default().tally.away_ms += len;
                out.touch(seg.date);
            }
            if !away.confirmed && self.s.now.saturating_sub(away.since) >= ms(CONFIRMED_BREAK_AFTER)
            {
                away.confirmed = true;
                self.s.away = Some(away);
                self.confirmed_break(ctx, out);
            }
            return;
        }
        let paused = self.s.pause.is_some();
        // Split points: local midnights, work-window edges and the idle thresholds.
        let mut cuts = vec![start, end];
        for seg in ctx.zone.split_by_local_day(start, end) {
            cuts.push(seg.start);
            for d in [seg.date.pred_opt(), Some(seg.date)].into_iter().flatten() {
                if let Some(w) = ctx.schedule.window_for(d, &ctx.zone) {
                    cuts.extend([w.start, w.end]);
                }
            }
        }
        let last_input =
            end - chrono::Duration::milliseconds(i64::try_from(idle_end).unwrap_or(i64::MAX));
        for t in [ACTIVE_WITHIN, LONG_QUIET_AFTER] {
            if let Ok(d) = chrono::Duration::from_std(t) {
                cuts.push(last_input + d);
            }
        }
        cuts.retain(|t| *t >= start && *t <= end);
        cuts.sort();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let len = ms_between(a, b);
            if len == 0 {
                continue;
            }
            let mid = a + (b - a) / 2;
            let date = ctx.zone.local_date(mid);
            let idle_mid = if mid < last_input {
                0
            } else {
                ms_between(last_input, mid)
            };
            let class = if idle_mid < ms(ACTIVE_WITHIN) {
                SessionState::Active
            } else if idle_mid <= ms(LONG_QUIET_AFTER) {
                SessionState::Quiet
            } else {
                SessionState::LongQuiet
            };
            let window = ctx.schedule.window_at(mid, &ctx.zone);
            out.touch(date);
            let counts_as_work = window.is_some() && !paused;
            if let Some(w) = window
                && self.s.work_date != Some(w.date)
            {
                // A new work window starts with fresh movement and eye-rest timers.
                self.s.work_date = Some(w.date);
                self.fresh_timers();
                self.s.block_ms = 0;
                out.significant = true;
            }
            // At the PC, whatever the input: screen time, and the water pace's clock
            // inside the work window (counted on the date the window starts).
            if let Some(w) = window {
                self.s
                    .days
                    .entry(w.date)
                    .or_default()
                    .tally
                    .window_screen_ms += len;
                out.touch(w.date);
            }
            let day = &mut self.s.days.entry(date).or_default().tally;
            day.screen_ms += len;
            match class {
                SessionState::Active | SessionState::Quiet => {
                    day.pc_active_ms += len;
                    if counts_as_work {
                        if class == SessionState::Active {
                            day.work_active_ms += len;
                        } else {
                            day.work_quiet_ms += len;
                        }
                        self.s.block_ms += len;
                        day.longest_block_ms = day.longest_block_ms.max(self.s.block_ms);
                        for a in [Activity::Movement, Activity::EyeRest] {
                            self.s.acts[idx(a)].accum_ms += len;
                        }
                    }
                }
                _ => {
                    if counts_as_work {
                        day.unknown_ms += len;
                        self.s.block_ms = 0;
                    }
                }
            }
        }
    }

    fn run_timers(&mut self, ctx: &Context, out: &mut Out, e: u64) {
        let now = self.s.now;
        if self.s.focus_until.is_some_and(|t| now >= t) {
            self.s.focus_until = None;
            out.significant = true;
            out.notices.push(Notice::FocusEnded {
                reason: FocusEnd::Expired,
                silent: self.s.away.is_some(),
            });
        }
        if matches!(self.s.pause, Some(PauseUntil::At(t)) if now >= t) {
            self.s.pause = None;
            out.significant = true;
            out.notices.push(Notice::PauseEnded);
            self.resume_pomodoro_after_coaching_pause();
        }
        self.run_guide_timers(ctx, out);
        self.run_meditation_timers(ctx, out);
        if self
            .s
            .exercise_since
            .is_some_and(|t| now.saturating_sub(t) >= ms(ROUTINE_TIMEOUT))
        {
            self.s.exercise_since = None;
            self.s.gap_until = now + ms(CALM_GAP);
            out.significant = true;
        }
        self.run_pomodoro(ctx, out, e);
        if let Some(o) = self.s.offer {
            match o.phase {
                OfferPhase::Showing { since } => {
                    if now.saturating_sub(since) >= ms(COLLAPSE_AFTER) {
                        self.collapse(out);
                    }
                }
                OfferPhase::Waiting | OfferPhase::InProgress { .. } => {}
            }
        }
        if let Some(o) = self.s.offer {
            match o.phase {
                OfferPhase::Waiting => {
                    if now.saturating_sub(o.first_shown) >= ms(BADGE_EXPIRES_AFTER) {
                        self.resolve_no_response(ctx, out);
                    }
                }
                OfferPhase::InProgress { since } => match o.activity {
                    Activity::EyeRest if now.saturating_sub(since) >= ms(EYE_REST_LENGTH) => {
                        self.reset_timer(Activity::EyeRest);
                        self.finish_offer(o, ChipStatus::Done, out);
                        out.notices.push(Notice::Celebrate(Activity::EyeRest));
                    }
                    Activity::Movement if now.saturating_sub(since) >= ms(ROUTINE_TIMEOUT) => {
                        self.abandon_in_progress(out);
                    }
                    _ => {}
                },
                OfferPhase::Showing { .. } => {}
            }
        }
    }

    fn run_pomodoro(&mut self, ctx: &Context, out: &mut Out, e: u64) {
        // A card opens on its own only when nothing above the Pomodoro holds it back; a
        // card held back is not replayed when the hold ends (the pet still shows the
        // phase, and the user can open it).
        let visible = !self.pomodoro_card_held(ctx) && self.s.guide.is_none();
        let auto = ctx.config.pomodoro_auto_start;
        let Some(p) = self.s.pomodoro.as_mut() else {
            return;
        };
        if p.paused.is_some() || !matches!(p.phase, PomodoroPhase::Work | PomodoroPhase::Break) {
            return;
        }
        p.left_ms = p.left_ms.saturating_sub(e);
        if p.left_ms > 0 {
            return;
        }
        out.significant = true;
        match p.phase {
            PomodoroPhase::Work => {
                p.phase = PomodoroPhase::BreakOffered;
                p.card_open = visible;
                p.card_opened_under = HoldSet::EMPTY;
                out.notices.push(Notice::PomodoroRoundCounted);
                let d = ctx.zone.local_date(ctx.wall);
                self.s.days.entry(d).or_default().tally.pomodoro_rounds += 1;
                out.touch(d);
            }
            PomodoroPhase::Break => {
                if auto {
                    p.phase = PomodoroPhase::Work;
                    p.round += 1;
                    p.left_ms = p.work_ms;
                    p.card_open = false;
                } else {
                    p.phase = PomodoroPhase::RoundDone;
                    p.card_open = visible;
                    p.card_opened_under = HoldSet::EMPTY;
                }
            }
            PomodoroPhase::BreakOffered | PomodoroPhase::RoundDone => {}
        }
    }

    // ----------------------------------------------------------------- tracker

    fn fresh_timers(&mut self) {
        for a in [Activity::Movement, Activity::EyeRest] {
            let st = &mut self.s.acts[idx(a)];
            st.accum_ms = 0;
            st.base_ms = 0;
            st.snooze_until = None;
            st.due_since = None;
        }
    }

    /// A confirmed break restarts the movement and eye-rest timers. It is never recorded
    /// as movement and never changes a chip's outcome.
    fn confirmed_break(&mut self, ctx: &Context, out: &mut Out) {
        self.fresh_timers();
        self.s.block_ms = 0;
        let d = ctx.zone.local_date(ctx.wall);
        self.s.days.entry(d).or_default().tally.confirmed_breaks += 1;
        out.touch(d);
        out.significant = true;
    }

    fn end_away(&mut self, ctx: &Context, out: &mut Out) {
        let Some(away) = self.s.away else {
            return;
        };
        if away.locked || away.asleep {
            return;
        }
        let len = self.s.now.saturating_sub(away.since);
        if !away.confirmed && len >= ms(CONFIRMED_BREAK_AFTER) {
            self.confirmed_break(ctx, out);
        }
        self.s.away = None;
        self.s.idle_ms = 0;
        self.s.grace_until = self.s.now + ms(GRACE_AFTER_WAKE);
        // The "paused while you were away" card, unless something else still holds.
        let show = !self.pomodoro_card_held(ctx) && self.s.guide.is_none();
        if let Some(p) = self.s.pomodoro.as_mut()
            && p.paused == Some(PomodoroPause::LockOrSleep)
        {
            p.card_open = show;
            p.card_opened_under = HoldSet::EMPTY;
        }
    }

    fn pause_pomodoro_for_lock(&mut self) {
        if let Some(p) = self.s.pomodoro.as_mut()
            && matches!(p.phase, PomodoroPhase::Work | PomodoroPhase::Break)
            && p.paused.is_none()
        {
            p.paused = Some(PomodoroPause::LockOrSleep);
            p.card_open = false;
        }
    }

    fn resume_pomodoro_after_coaching_pause(&mut self) {
        if let Some(p) = self.s.pomodoro.as_mut()
            && p.paused == Some(PomodoroPause::CoachingPaused)
        {
            p.paused = None;
        }
    }

    // ---------------------------------------------------------------- reminders

    fn offer_id(&self, seq: u64) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.s.salt.to_be_bytes());
        bytes[8..].copy_from_slice(&seq.to_be_bytes());
        Builder::from_random_bytes(bytes).into_uuid()
    }

    fn reset_timer(&mut self, a: Activity) {
        let now = self.s.now;
        let st = &mut self.s.acts[idx(a)];
        st.snooze_until = None;
        st.due_since = None;
        match a {
            Activity::Movement | Activity::EyeRest => {
                st.accum_ms = 0;
                st.base_ms = 0;
            }
            Activity::Water => st.ready_at = now,
        }
    }

    /// Next offer after a full interval from now (skip, dismiss, no response, none,
    /// missed while quiet).
    fn reschedule(&mut self, ctx: &Context, a: Activity) {
        let now = self.s.now;
        let every = ms(self.every(ctx, a));
        let st = &mut self.s.acts[idx(a)];
        st.snooze_until = None;
        st.due_since = None;
        match a {
            Activity::Movement | Activity::EyeRest => st.base_ms = st.accum_ms,
            Activity::Water => st.ready_at = now + every,
        }
    }

    fn after_done(&mut self, ctx: &Context, a: Activity) {
        match a {
            Activity::Movement | Activity::EyeRest => self.reset_timer(a),
            Activity::Water => self.reschedule(ctx, a),
        }
    }

    fn collapse(&mut self, out: &mut Out) {
        if let Some(o) = self.s.offer.as_mut()
            && matches!(o.phase, OfferPhase::Showing { .. })
        {
            o.phase = OfferPhase::Waiting;
            out.significant = true;
        }
    }

    /// Close the current offer with a final status, start the calm gap.
    fn finish_offer(&mut self, o: Offer, status: ChipStatus, out: &mut Out) {
        self.s.acts[idx(o.activity)].last = Some(status);
        self.s.offer = None;
        self.s.gap_until = self.s.now + ms(CALM_GAP);
        out.significant = true;
        out.notices.push(Notice::ReminderResolved {
            offer: o.id,
            activity: o.activity,
            status,
        });
    }

    fn response_event(ctx: &Context, o: Offer, outcome: ReminderOutcome) -> NewEvent {
        let payload = EventPayload::ReminderResponse {
            reminder_id: o.id,
            activity: o.activity,
            outcome,
        };
        let e = if outcome == ReminderOutcome::NoResponse {
            NewEvent::inferred(payload, MeasurementStatus::Measured)
        } else {
            NewEvent::manual(payload)
        };
        e.at(ctx.wall)
    }

    fn resolve_no_response(&mut self, ctx: &Context, out: &mut Out) {
        let Some(o) = self.s.offer else {
            return;
        };
        out.events
            .push(Self::response_event(ctx, o, ReminderOutcome::NoResponse));
        self.reschedule(ctx, o.activity);
        self.finish_offer(o, ChipStatus::NoResponse, out);
    }

    /// A routine started from a reminder never reported back (or the app restarted):
    /// treated as stopped with none recorded, without writing a routine record.
    fn abandon_in_progress(&mut self, out: &mut Out) {
        let Some(o) = self.s.offer else {
            return;
        };
        let now = self.s.now;
        let st = &mut self.s.acts[idx(o.activity)];
        st.snooze_until = None;
        st.due_since = None;
        match o.activity {
            Activity::Movement | Activity::EyeRest => st.base_ms = st.accum_ms,
            Activity::Water => st.ready_at = now,
        }
        let status = match o.activity {
            Activity::Movement => ChipStatus::NoneRecorded,
            _ => ChipStatus::NoResponse,
        };
        self.finish_offer(o, status, out);
    }

    fn count_dismissal(&mut self, ctx: &Context) {
        let light = ctx.config.intensity == Intensity::Light;
        let day = self.today_state(ctx);
        day.dismissals += 1;
        let offer = day.dismissals >= DISMISSALS_FOR_LIGHTER && !day.lighter_offered && !light;
        if offer {
            day.lighter_offered = true;
            self.set_prompt(Prompt::Lighter, false, ctx);
        }
    }

    fn current_offer(&self, id: Uuid) -> Option<Offer> {
        self.s.offer.filter(|o| o.id == id)
    }

    fn respond(&mut self, ctx: &Context, id: Uuid, r: Response, out: &mut Out) -> InputOutcome {
        let Some(o) = self.current_offer(id) else {
            return InputOutcome::Ignored("stale reminder");
        };
        if !matches!(o.phase, OfferPhase::Showing { .. } | OfferPhase::Waiting) {
            return InputOutcome::Ignored("reminder already answered");
        }
        let a = o.activity;
        match r {
            Response::Done => {
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Done));
                self.after_done(ctx, a);
                self.finish_offer(o, ChipStatus::Done, out);
                out.notices.push(Notice::Celebrate(a));
                if a == Activity::Water {
                    // "Already did it" opens the water panel (wireframe B2).
                    self.open_water_panel_after_reminder(ctx, out);
                }
            }
            Response::LogWater { ml } => {
                if a != Activity::Water {
                    return InputOutcome::Ignored("only a water reminder logs water");
                }
                if !(1..=MAX_WATER_ENTRY_ML).contains(&ml) {
                    return InputOutcome::Invalid("water amount must be 1 to 2000 ml");
                }
                out.events.push(
                    NewEvent::manual(EventPayload::WaterLogged { amount_ml: ml }).at(ctx.wall),
                );
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Done));
                out.extra_water_ml = ml;
                self.after_done(ctx, a);
                self.finish_offer(o, ChipStatus::Done, out);
                out.notices.push(Notice::Celebrate(a));
            }
            Response::Start => {
                if a == Activity::Water {
                    return InputOutcome::Ignored("water has no guided start");
                }
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Started));
                if let Some(cur) = self.s.offer.as_mut() {
                    cur.phase = OfferPhase::InProgress { since: self.s.now };
                }
                if a == Activity::Movement {
                    self.open_guide(ctx, Some(o.id), out);
                }
            }
            Response::Snooze => {
                out.events
                    .push(Self::response_event(ctx, o, ReminderOutcome::Snoozed));
                self.snooze(a);
                self.finish_offer(o, ChipStatus::Snoozed, out);
            }
            Response::Skip | Response::Dismiss => {
                let (outcome, status) = if r == Response::Skip {
                    (ReminderOutcome::Skipped, ChipStatus::Skipped)
                } else {
                    (ReminderOutcome::Dismissed, ChipStatus::Dismissed)
                };
                out.events.push(Self::response_event(ctx, o, outcome));
                self.reschedule(ctx, a);
                self.finish_offer(o, status, out);
                self.count_dismissal(ctx);
            }
        }
        InputOutcome::Applied
    }

    fn snooze(&mut self, a: Activity) {
        let until = self.s.now + ms(SNOOZE);
        let st = &mut self.s.acts[idx(a)];
        st.snooze_until = Some(until);
        st.due_since = None;
    }

    fn routine_finished(
        &mut self,
        ctx: &Context,
        offer: Option<Uuid>,
        outcome: RoutineOutcome,
        out: &mut Out,
    ) -> InputOutcome {
        let status = match outcome {
            RoutineOutcome::Done => ChipStatus::Done,
            RoutineOutcome::Partial => ChipStatus::Partial,
            RoutineOutcome::None => ChipStatus::NoneRecorded,
            RoutineOutcome::Skipped => ChipStatus::Skipped,
        };
        let in_progress = self.s.offer.filter(|o| {
            o.activity == Activity::Movement && matches!(o.phase, OfferPhase::InProgress { .. })
        });

        match offer.or(in_progress.map(|o| o.id)) {
            Some(id) => {
                let Some(o) = self.current_offer(id) else {
                    return InputOutcome::Ignored("stale reminder");
                };
                if o.activity != Activity::Movement
                    || !matches!(o.phase, OfferPhase::InProgress { .. })
                {
                    return InputOutcome::Ignored("no movement routine in progress");
                }
                if outcome.resets_movement_timer() {
                    self.reset_timer(Activity::Movement);
                } else {
                    // None or skipped: calm gap and next due time, timer not restarted.
                    self.reschedule(ctx, Activity::Movement);
                }
                self.finish_offer(o, status, out);
                if outcome == RoutineOutcome::Skipped {
                    self.count_dismissal(ctx);
                }
            }
            None => {
                // From the pet menu: done or partial restarts the movement timer; none
                // and skipped change nothing (nothing was offered).
                self.s.exercise_since = None;
                if outcome.resets_movement_timer() {
                    self.reset_timer(Activity::Movement);
                    self.s.acts[idx(Activity::Movement)].last = Some(status);
                }
                self.s.gap_until = self.s.now + ms(CALM_GAP);
            }
        }
        if outcome == RoutineOutcome::Done {
            out.notices.push(Notice::Celebrate(Activity::Movement));
        }
        InputOutcome::Applied
    }

    // ------------------------------------------------------------------- inputs

    fn apply(&mut self, ctx: &Context, input: Input, out: &mut Out) -> InputOutcome {
        use InputOutcome::{Applied, Ignored};
        match input {
            Input::Tick { .. } | Input::Refresh => Applied,
            Input::Lock | Input::Suspend => {
                let lock = input == Input::Lock;
                match self.s.away.as_mut() {
                    Some(a) if (lock && a.locked) || (!lock && a.asleep) => {
                        return Ignored("duplicate signal");
                    }
                    Some(a) => {
                        if lock {
                            a.locked = true;
                        } else {
                            a.asleep = true;
                        }
                    }
                    None => {
                        self.s.away = Some(Away {
                            since: self.s.now,
                            locked: lock,
                            asleep: !lock,
                            confirmed: false,
                        });
                    }
                }
                self.pause_pomodoro_for_lock();
                Applied
            }
            Input::Unlock | Input::Resume => {
                let unlock = input == Input::Unlock;
                match self.s.away.as_mut() {
                    Some(a) if unlock && a.locked => a.locked = false,
                    Some(a) if !unlock && a.asleep => a.asleep = false,
                    _ => return Ignored("not locked or asleep"),
                }
                self.end_away(ctx, out);
                Applied
            }
            Input::PresentationChanged(on) => {
                if self.s.presenting == on {
                    return Ignored("duplicate signal");
                }
                self.s.presenting = on;
                if !on {
                    self.s.show_override = false;
                }
                // Cards that were open stay closed; nothing comes back afterwards.
                if on && let Some(p) = self.s.pomodoro.as_mut() {
                    p.card_open = false;
                }
                Applied
            }
            Input::MediaInUseChanged(on) => {
                if self.s.media_in_use == on {
                    return Ignored("duplicate signal");
                }
                self.s.media_in_use = on;
                Applied
            }
            Input::ShowPet => {
                if !self.presenting_effective(ctx) || self.s.show_override {
                    return Ignored("the pet is not hidden");
                }
                self.s.show_override = true;
                Applied
            }
            Input::Respond { offer, response } => self.respond(ctx, offer, response, out),
            Input::CloseBubble => {
                if self.close_open_card(ctx, out) {
                    Applied
                } else {
                    Ignored("no bubble")
                }
            }
            Input::OpenPending => {
                if !self.visible(ctx) {
                    return Ignored("the pet is hidden");
                }
                let holds = self.holds_above_pomodoro(ctx);
                if self.reopen_guide(ctx) {
                    Applied
                } else if let Some(o) = self.s.offer.as_mut()
                    && o.phase == OfferPhase::Waiting
                {
                    o.phase = OfferPhase::Showing { since: self.s.now };
                    Applied
                } else if self.s.offer.is_none()
                    && self.s.guide.is_none()
                    && self.s.prompt.is_none()
                    && let Some(p) = self.s.pomodoro.as_mut()
                    && !p.card_open
                    && (matches!(
                        p.phase,
                        PomodoroPhase::BreakOffered | PomodoroPhase::RoundDone
                    ) || p.paused == Some(PomodoroPause::LockOrSleep))
                {
                    p.card_open = true;
                    p.card_opened_under = holds;
                    // The user asked for this card: it takes the bubble from a note.
                    self.clear_note(out);
                    Applied
                } else {
                    Ignored("nothing waiting")
                }
            }
            Input::RoutineStarted => {
                if let Some(o) = self.s.offer
                    && o.activity == Activity::Movement
                    && matches!(o.phase, OfferPhase::Showing { .. } | OfferPhase::Waiting)
                {
                    return self.respond(ctx, o.id, Response::Start, out);
                }
                if self.s.exercise_since.is_some() || self.level3() {
                    return Ignored("an exercise or Pomodoro is already running");
                }
                self.s.exercise_since = Some(self.s.now);
                self.open_guide(ctx, None, out);
                Applied
            }
            Input::Guide(action) => self.guide_input(ctx, action, out),
            Input::RoutineFinished { offer, outcome } => {
                // With the guide running, the report is read against the movements
                // already answered; the reminder and the routine record get the same result.
                let r = self.routine_finished(ctx, offer, self.reported_outcome(outcome), out);
                if r == Applied {
                    // A report in one go answers the guide too.
                    self.report_guide(ctx, outcome, out);
                }
                r
            }
            Input::RoutineLater { offer } => {
                let Some(o) = self.current_offer(offer) else {
                    return Ignored("stale reminder");
                };
                if o.activity != Activity::Movement
                    || !matches!(o.phase, OfferPhase::InProgress { .. })
                {
                    return Ignored("no movement routine in progress");
                }
                self.snooze(Activity::Movement);
                self.finish_offer(o, ChipStatus::Snoozed, out);
                self.s.guide = None;
                Applied
            }
            Input::MarkBreak => {
                self.confirmed_break(ctx, out);
                Applied
            }
            Input::StartFocus {
                minutes,
                replace_pomodoro,
            } => {
                if !FOCUS_MINUTES.contains(&minutes) {
                    return InputOutcome::Invalid("focus must be 5 to 240 minutes");
                }
                if self.s.pomodoro.is_some() {
                    if !replace_pomodoro {
                        return InputOutcome::NeedsConfirmation(Confirmation::ReplacePomodoro);
                    }
                    self.s.pomodoro = None;
                }
                self.s.focus_until = Some(self.s.now + u64::from(minutes) * 60_000);
                let d = ctx.zone.local_date(ctx.wall);
                self.s.days.entry(d).or_default().tally.focus_sessions += 1;
                out.touch(d);
                Applied
            }
            Input::EndFocus => {
                if self.s.focus_until.take().is_none() {
                    return Ignored("focus is not on");
                }
                out.notices.push(Notice::FocusEnded {
                    reason: FocusEnd::Ended,
                    silent: false,
                });
                Applied
            }
            Input::Pause(len) => {
                let until = match len {
                    PauseLength::Minutes(m) => {
                        if !PAUSE_MINUTES.contains(&m) {
                            return InputOutcome::Invalid("pause must be 1 minute to 24 hours");
                        }
                        PauseUntil::At(self.s.now + u64::from(m) * 60_000)
                    }
                    PauseLength::RestOfToday => PauseUntil::EndOfDay(ctx.zone.local_date(ctx.wall)),
                    PauseLength::UntilResumed => PauseUntil::Resumed,
                };
                self.s.pause = Some(until);
                if let Some(p) = self.s.pomodoro.as_mut()
                    && matches!(p.phase, PomodoroPhase::Work | PomodoroPhase::Break)
                    && p.paused.is_none()
                {
                    p.paused = Some(PomodoroPause::CoachingPaused);
                }
                Applied
            }
            Input::ResumeCoaching => {
                if self.s.pause.take().is_none() {
                    return Ignored("coaching is not paused");
                }
                self.resume_pomodoro_after_coaching_pause();
                Applied
            }
            Input::SetMeeting(on) => {
                if self.s.meeting.is_some() == on {
                    return Ignored("duplicate toggle");
                }
                self.s.meeting = on.then(|| ctx.zone.local_date(ctx.wall));
                Applied
            }
            Input::StartPomodoro {
                preset,
                replace_focus,
                resume_coaching,
            } => {
                let Ok((work, brk)) = preset.lengths() else {
                    return InputOutcome::Invalid("custom Pomodoro out of range");
                };
                if self.s.pomodoro.is_some() {
                    return Ignored("a Pomodoro is already running");
                }
                if self.s.focus_until.is_some() && !replace_focus {
                    return InputOutcome::NeedsConfirmation(Confirmation::ReplaceFocus);
                }
                if self.s.pause.is_some() && !resume_coaching {
                    return InputOutcome::NeedsConfirmation(Confirmation::ResumeCoaching);
                }
                if self.s.focus_until.take().is_some() {
                    out.notices.push(Notice::FocusEnded {
                        reason: FocusEnd::Replaced,
                        silent: false,
                    });
                }
                self.s.pause = None;
                // A Pomodoro replaces an exercise: the movement guide and a meditation
                // pause stop (recorded as stopped) before its timer starts.
                self.interrupt_guide(ctx, out);
                self.interrupt_meditation(ctx, out);
                self.s.exercise_since = None;
                self.s.pomodoro = Some(Pomodoro {
                    work_ms: ms(work),
                    break_ms: ms(brk),
                    round: 1,
                    phase: PomodoroPhase::Work,
                    left_ms: ms(work),
                    paused: None,
                    card_open: false,
                    card_opened_under: HoldSet::EMPTY,
                });
                Applied
            }
            Input::PausePomodoro => match self.s.pomodoro.as_mut() {
                Some(p)
                    if p.paused.is_none()
                        && matches!(p.phase, PomodoroPhase::Work | PomodoroPhase::Break) =>
                {
                    p.paused = Some(PomodoroPause::User);
                    Applied
                }
                _ => Ignored("no running Pomodoro"),
            },
            Input::ResumePomodoro => {
                if self.s.pause.is_some() {
                    return Ignored("coaching is paused");
                }
                match self.s.pomodoro.as_mut() {
                    Some(p) if p.paused.is_some() => {
                        p.paused = None;
                        p.card_open = false;
                        Applied
                    }
                    _ => Ignored("no paused Pomodoro"),
                }
            }
            Input::EndPomodoro => {
                if self.s.pomodoro.take().is_none() {
                    return Ignored("no Pomodoro");
                }
                self.s.gap_until = self.s.now + ms(CALM_GAP);
                Applied
            }
            Input::StartPomodoroBreak => {
                let Some(p) = self.s.pomodoro.as_mut() else {
                    return Ignored("no Pomodoro");
                };
                if p.phase != PomodoroPhase::BreakOffered {
                    return Ignored("no break offered");
                }
                p.phase = PomodoroPhase::Break;
                p.left_ms = p.break_ms;
                p.card_open = false;
                p.paused = None;
                // The user takes a break: a confirmed break, never recorded as movement.
                self.confirmed_break(ctx, out);
                Applied
            }
            Input::SkipPomodoroBreak => {
                let auto = ctx.config.pomodoro_auto_start;
                let holds = self.holds_above_pomodoro(ctx);
                // The movement guide (open or waiting) or an open card keeps the bubble.
                let visible =
                    self.visible(ctx) && self.s.guide.is_none() && self.s.prompt.is_none();
                let Some(p) = self.s.pomodoro.as_mut() else {
                    return Ignored("no Pomodoro");
                };
                if p.phase != PomodoroPhase::BreakOffered {
                    return Ignored("no break offered");
                }
                if auto {
                    p.phase = PomodoroPhase::Work;
                    p.round += 1;
                    p.left_ms = p.work_ms;
                    p.card_open = false;
                } else {
                    // The user acted: the next card answers them, even under a hold, but
                    // only where they can see it (not while the pet is hidden). It takes
                    // the bubble from a note.
                    p.phase = PomodoroPhase::RoundDone;
                    p.card_open = visible;
                    p.card_opened_under = holds;
                    if visible {
                        self.clear_note(out);
                    }
                }
                Applied
            }
            Input::NextPomodoroRound => {
                let Some(p) = self.s.pomodoro.as_mut() else {
                    return Ignored("no Pomodoro");
                };
                if p.phase != PomodoroPhase::RoundDone {
                    return Ignored("the round is not over");
                }
                p.phase = PomodoroPhase::Work;
                p.round += 1;
                p.left_ms = p.work_ms;
                p.paused = None;
                p.card_open = false;
                Applied
            }
            Input::CheckIn(answer) => self.check_in(ctx, answer, out),
            Input::DeadlineDay => self.deadline_day(ctx, out),
            Input::Recovery(action) => self.recovery_input(ctx, action, out),
            Input::LighterSchedule { accept } => {
                if self.s.prompt != Some(Prompt::Lighter) {
                    return Ignored("no offer open");
                }
                self.clear_prompt();
                self.s.gap_until = self.s.now + ms(CALM_GAP);
                if accept {
                    out.notices.push(Notice::UseIntensity(Intensity::Light));
                }
                Applied
            }
            Input::Water(w) => self.water_input(ctx, w, out),
            Input::Phone(p) => self.phone_input(ctx, p, out),
            Input::Review(answer) => self.review_input(ctx, answer, out),
            Input::Meditation(action) => self.meditation_input(ctx, action, out),
            Input::PostureCue => self.posture_cue_input(ctx, out),
        }
    }

    /// Close the card on show as its close button does (no answer is recorded): a
    /// reminder collapses to its waiting badge, the movement guide waits behind the pet, a
    /// check-in, offer or recovery card closes with the calm gap, a note closes, and a
    /// Pomodoro card closes (its phase stays for the menu). Returns false if none is open.
    fn close_open_card(&mut self, ctx: &Context, out: &mut Out) -> bool {
        if let Some(o) = self.s.offer
            && matches!(o.phase, OfferPhase::Showing { .. })
        {
            self.collapse(out);
            true
        } else if self.close_guide_card(out) || self.close_meditation_card(ctx, out) {
            true
        } else if self.s.prompt.is_some() {
            self.clear_prompt();
            self.s.gap_until = self.s.now + ms(CALM_GAP);
            out.significant = true;
            true
        } else if self.clear_note(out) {
            true
        } else if let Some(p) = self.s.pomodoro.as_mut()
            && p.card_open
        {
            p.card_open = false;
            out.significant = true;
            true
        } else {
            false
        }
    }

    // ------------------------------------------------------------------- settle

    /// A hold above the Pomodoro withdraws its card (break, round done, or paused while
    /// away) unless the user opened it under holds that include every hold now in force:
    /// any other hold (a new lock, sleep or pause, a stronger hold, or one that ended and
    /// began again) withdraws it too. The phase stays; the card is not reopened when the
    /// hold ends.
    fn withdraw_held_pomodoro_card(&mut self, ctx: &Context, out: &mut Out) {
        let holds = self.holds_above_pomodoro(ctx);
        if let Some(p) = self.s.pomodoro.as_mut() {
            let keep = if !p.card_open {
                HoldSet::EMPTY
            } else if holds.within(p.card_opened_under) {
                // A hold that has ended no longer covers the card if it begins again.
                holds
            } else {
                p.card_open = false;
                out.significant = true;
                HoldSet::EMPTY
            };
            if p.card_opened_under != keep {
                p.card_opened_under = keep;
                out.significant = true;
            }
        }
    }

    /// Apply holds, then offer at most one thing.
    fn settle(&mut self, ctx: &Context, out: &mut Out) {
        // A hold that was not in force when the guide's card opened puts it behind the pet;
        // the routine keeps running.
        self.settle_guide(ctx, out);
        // Any hold that starts ends a meditation pause (recorded as stopped).
        self.settle_meditation(ctx, out);
        let hold = self.hold(ctx);
        // Anything holding reminders back withdraws a reminder that is showing or waiting.
        if let Some(o) = self.s.offer
            && matches!(o.phase, OfferPhase::Showing { .. } | OfferPhase::Waiting)
            && (hold.is_some() || !ctx.config.categories.allows(o.activity))
        {
            self.resolve_no_response(ctx, out);
        }
        self.settle_prompt(ctx, out);
        self.settle_note(ctx, out);
        self.withdraw_held_pomodoro_card(ctx, out);
        // End of a quiet period: nothing is replayed.
        let quiet = self.quiet_hold(ctx);
        if let Some(ended) = self.s.quiet_hold
            && quiet.is_none()
        {
            for a in ORDER {
                if self.is_due(ctx, a) {
                    self.reschedule(ctx, a);
                }
            }
            self.s.gap_until = self.s.gap_until.max(self.s.now + ms(CALM_GAP));
            out.notices.push(Notice::HoldEnded(ended));
            out.significant = true;
        }
        if quiet != self.s.quiet_hold {
            out.significant = true;
        }
        self.s.quiet_hold = quiet;
        // Track when each activity became due.
        let now = self.s.now;
        for a in ORDER {
            let due = self.is_due(ctx, a);
            let st = &mut self.s.acts[idx(a)];
            match (due, st.due_since) {
                (true, None) => st.due_since = Some(now),
                (false, Some(_)) => st.due_since = None,
                _ => {}
            }
        }
        let slot_free = self.s.offer.is_none()
            && self.s.prompt.is_none()
            && self.s.guide.is_none()
            && !self.meditation_open()
            && self.s.note.is_none();
        let calm = now >= self.s.gap_until && now >= self.s.grace_until;
        let held = self.hold(ctx).is_some();
        if !slot_free || held || !self.visible(ctx) {
            // A recovery walk time that arrives now is used up, not shown afterwards.
            self.use_up_held_walk(ctx, out);
        }
        if !(slot_free && calm && self.visible(ctx)) {
            return;
        }
        if !held {
            if self.maybe_recovery_card(ctx, out) {
                return;
            }
            let next = ORDER
                .iter()
                .copied()
                .filter(|a| self.is_due(ctx, *a))
                .min_by_key(|a| (self.s.acts[idx(*a)].due_since.unwrap_or(now), idx(*a)));
            if let Some(a) = next {
                self.show(ctx, a, out);
                return;
            }
        }
        if self.maybe_reflection(ctx, out) {
            return;
        }
        if self.maybe_review_offer(ctx, out) {
            return;
        }
        if self.maybe_meditation_invite(ctx, out) {
            return;
        }
        self.maybe_check_in(ctx, out);
    }

    /// Open a card in the bubble. `user`: the user asked for it (the "Deadline day"
    /// chain), so it may stay under the holds in force now; otherwise any hold that
    /// blocks cards withdraws it.
    fn set_prompt(&mut self, p: Prompt, user: bool, ctx: &Context) {
        self.s.prompt = Some(p);
        self.s.prompt_opened_under = user.then(|| self.holds_above_pomodoro(ctx));
        self.s.prompt_since = self.s.now;
    }

    fn clear_prompt(&mut self) {
        self.s.prompt = None;
        self.s.prompt_opened_under = None;
        self.review_closed();
    }

    /// Withdraw a card that a hold now blocks (a card the user asked for stays while only
    /// the holds it opened under are in force), and close a recovery card left
    /// unanswered.
    fn settle_prompt(&mut self, ctx: &Context, out: &mut Out) {
        let Some(p) = self.s.prompt else {
            return;
        };
        let holds = self.holds_above_pomodoro(ctx);
        let withdraw = !self.visible(ctx)
            || match self.s.prompt_opened_under {
                Some(under) => !holds.within(under),
                None => self.prompt_blocked(ctx),
            }
            || (p.is_recovery_card()
                && self.s.now.saturating_sub(self.s.prompt_since) >= ms(CARD_CLOSES_AFTER))
            // The meditation invitation keeps to work hours (the check-in may run late).
            || (p == Prompt::MeditationInvite && self.off_hours(ctx))
            // The posture cue keeps to work hours (the check-in may run late).
            || (p == Prompt::PostureCue && self.off_hours(ctx));
        if withdraw {
            self.clear_prompt();
            out.significant = true;
        } else if let Some(under) = self.s.prompt_opened_under
            && under != holds
        {
            // A hold that ended no longer covers the card if it begins again.
            self.s.prompt_opened_under = Some(holds);
            out.significant = true;
        }
    }

    fn show(&mut self, ctx: &Context, a: Activity, out: &mut Out) {
        self.s.offer_seq += 1;
        let seq = self.s.offer_seq;
        let id = self.offer_id(seq);
        let now = self.s.now;
        self.s.offer = Some(Offer {
            id,
            seq,
            activity: a,
            phase: OfferPhase::Showing { since: now },
            first_shown: now,
        });
        // A snooze returns the activity once.
        self.s.acts[idx(a)].snooze_until = None;
        out.events.push(
            NewEvent::inferred(
                EventPayload::ReminderOffered {
                    reminder_id: id,
                    activity: a,
                },
                MeasurementStatus::Measured,
            )
            .at(ctx.wall),
        );
        out.notices.push(Notice::ReminderShown {
            offer: id,
            activity: a,
        });
        out.significant = true;
    }

    fn maybe_check_in(&mut self, ctx: &Context, out: &mut Out) {
        let cfg = ctx.config.long_day_check_in;
        if !cfg.enabled || self.prompt_blocked(ctx) {
            return;
        }
        let threshold = u64::from(cfg.hours) * 3_600_000;
        let day = self.today_state(ctx);
        if day.check_in_done || day.tally.pc_active_ms < threshold {
            return;
        }
        day.check_in_done = true;
        self.set_prompt(Prompt::CheckIn, false, ctx);
        out.significant = true;
    }

    // --------------------------------------------------------------------- view

    fn chip(&self, ctx: &Context, a: Activity, water: Option<&WaterPace>) -> Chip {
        let st = &self.s.acts[idx(a)];
        let snoozed_for = st.snooze_until.map(|u| dur(u.saturating_sub(self.s.now)));
        let status = if !ctx.config.categories.allows(a) {
            ChipStatus::Off
        } else if a == Activity::Water && water.is_none() {
            ChipStatus::NoGoal
        } else if let Some(o) = self.s.offer.filter(|o| o.activity == a) {
            match o.phase {
                OfferPhase::Showing { .. } => ChipStatus::Showing,
                OfferPhase::Waiting => ChipStatus::Waiting,
                OfferPhase::InProgress { .. } => ChipStatus::InProgress,
            }
        } else if a == Activity::Water && water.is_some_and(|w| w.goal_reached) {
            ChipStatus::GoalReached
        } else if st.snooze_until.is_some() {
            ChipStatus::Snoozed
        } else if self.is_due(ctx, a) {
            ChipStatus::Due
        } else {
            st.last.unwrap_or(ChipStatus::NotDue)
        };
        Chip {
            activity: a,
            status,
            snoozed_for: if status == ChipStatus::Snoozed {
                snoozed_for
            } else {
                None
            },
        }
    }

    fn pomodoro_view(&self) -> Option<PomodoroView> {
        self.s.pomodoro.map(|p| PomodoroView {
            phase: p.phase,
            left: dur(p.left_ms),
            paused: p.paused,
            round: p.round,
        })
    }

    fn session_state(&self, ctx: &Context) -> SessionState {
        if let Some(a) = self.s.away {
            if a.confirmed {
                SessionState::ConfirmedBreak
            } else {
                SessionState::ShortLock
            }
        } else if self.s.pause.is_some() {
            SessionState::Paused
        } else if self.off_hours(ctx) {
            SessionState::OffHours
        } else if self.s.idle_ms < ms(ACTIVE_WITHIN) {
            SessionState::Active
        } else if self.s.idle_ms <= ms(LONG_QUIET_AFTER) {
            SessionState::Quiet
        } else {
            SessionState::LongQuiet
        }
    }

    fn pet_mode(&self, ctx: &Context) -> PetMode {
        let now = self.s.now;
        if !self.visible(ctx) {
            return PetMode::Hidden(HiddenReason::Presentation);
        }
        if self.s.away.is_some() {
            return PetMode::Away;
        }
        if let Some(p) = self.s.pause {
            return PetMode::Paused {
                left: match p {
                    PauseUntil::At(t) => Some(dur(t.saturating_sub(now))),
                    _ => None,
                },
                rest_of_today: matches!(p, PauseUntil::EndOfDay(_)),
            };
        }
        if self.off_hours(ctx) {
            return PetMode::OffHours;
        }
        if let Some(t) = self.s.focus_until {
            return PetMode::Focus {
                left: dur(t.saturating_sub(now)),
            };
        }
        if self.meeting_on(ctx) || self.presenting_effective(ctx) {
            return PetMode::Meeting;
        }
        if let Some(v) = self.pomodoro_view() {
            return PetMode::Pomodoro(v);
        }
        if let Some(o) = self.s.offer {
            return match o.phase {
                OfferPhase::InProgress { .. } => PetMode::Exercise,
                OfferPhase::Showing { .. } => PetMode::Reminder(o.activity),
                OfferPhase::Waiting => PetMode::Waiting(o.activity),
            };
        }
        if self.s.exercise_since.is_some() {
            return PetMode::Exercise;
        }
        if self.s.prompt.is_some() {
            return PetMode::Nudge;
        }
        if let Some(a) = ORDER
            .iter()
            .copied()
            .find(|a| self.s.acts[idx(*a)].snooze_until.is_some())
        {
            return PetMode::Snoozed(a);
        }
        PetMode::Idle
    }

    fn bubble(&self, ctx: &Context, water: Option<&WaterPace>) -> Option<Bubble> {
        if !self.visible(ctx) {
            return None;
        }
        if let Some(o) = self.s.offer
            && matches!(o.phase, OfferPhase::Showing { .. })
        {
            let st = &self.s.acts[idx(o.activity)];
            let reason = match o.activity {
                Activity::Movement => ReminderReason::Movement {
                    worked: dur(st.accum_ms),
                },
                Activity::EyeRest => ReminderReason::EyeRest {
                    screen_time: dur(st.accum_ms),
                },
                Activity::Water => ReminderReason::Water {
                    expected_ml: water.map_or(0, |w| w.expected_ml),
                    logged_ml: water.map_or(0, |w| w.logged_ml),
                    behind_ml: water.map_or(0, |w| w.behind_ml),
                },
            };
            return Some(Bubble::Reminder {
                offer: o.id,
                activity: o.activity,
                reason,
            });
        }
        if let Some(g) = self.guide_view(ctx)
            && g.open
        {
            return Some(Bubble::MovementGuide(g));
        }
        if let Some(m) = self.meditation_view() {
            return Some(Bubble::Meditation(m));
        }
        if let Some(p) = self.s.prompt {
            return Some(match p {
                Prompt::CheckIn => Bubble::LongDayCheckIn {
                    hours: ctx.config.long_day_check_in.hours,
                },
                Prompt::Lighter => Bubble::LighterSchedule,
                Prompt::ReviewOffer => self.review_bubble()?,
                Prompt::MeditationInvite => Bubble::MeditationInvite,
                Prompt::PostureCue => Bubble::PostureCue,
                _ => self.recovery_bubble(ctx, p)?,
            });
        }
        // A note the user asked for comes before a Pomodoro card, which waits for it.
        if let Some(n) = self.s.note {
            return Some(n.bubble());
        }
        let holds = self.holds_above_pomodoro(ctx);
        let p = self
            .s
            .pomodoro
            .filter(|p| p.card_open && holds.within(p.card_opened_under))?;
        if p.paused == Some(PomodoroPause::LockOrSleep) {
            return Some(Bubble::PomodoroPausedByLock);
        }
        match p.phase {
            PomodoroPhase::BreakOffered => Some(Bubble::PomodoroBreak {
                length: dur(p.break_ms),
                // Water only if hydration reminders are on and the user is genuinely
                // behind pace for the time at the PC today (wireframe B6, DEC13).
                water_behind: water
                    .copied()
                    .filter(|w| ctx.config.categories.water && w.is_behind()),
            }),
            PomodoroPhase::RoundDone => Some(Bubble::PomodoroRoundDone),
            PomodoroPhase::Work | PomodoroPhase::Break => None,
        }
    }

    fn build_view(&self, ctx: &Context, extra_water_ml: u32) -> CoachView {
        let water = self.water_pace(ctx, extra_water_ml);
        CoachView {
            visible: self.visible(ctx),
            pet: self.pet_mode(ctx),
            bubble: self.bubble(ctx, water.as_ref()),
            chips: ORDER.map(|a| self.chip(ctx, a, water.as_ref())),
            session: self.session_state(ctx),
            hold: self.hold(ctx),
            water,
            screen_time: dur(self
                .s
                .days
                .get(&ctx.zone.local_date(ctx.wall))
                .map_or(0, |d| d.tally.screen_ms)),
            pomodoro: self.pomodoro_view(),
            focus_left: self
                .s
                .focus_until
                .map(|t| dur(t.saturating_sub(self.s.now))),
            meeting: self.s.meeting.is_some(),
            active_routine: self.active_routine(),
            guide: self.guide_view(ctx),
            recovery: self.recovery_view(ctx),
            water_entries: ctx.water_entries.clone(),
            servings: ctx.servings.clone(),
            holds: HoldSources {
                away: self.s.away.is_some(),
                paused: self.s.pause.is_some(),
                off_hours: self.off_hours(ctx),
                focus: self.s.focus_until.is_some(),
                presenting: self.s.presenting,
                meeting: self.meeting_on(ctx),
            },
        }
    }

    /// The routine in progress, whatever the pet shows: one started from a reminder
    /// first (it is the one [`Input::RoutineFinished`] answers), else one started from the
    /// pet menu.
    fn active_routine(&self) -> Option<ActiveRoutine> {
        if let Some(o) = self.s.offer
            && matches!(o.phase, OfferPhase::InProgress { .. })
        {
            return Some(ActiveRoutine {
                activity: o.activity,
                offer: Some(o.id),
            });
        }
        self.s.exercise_since.map(|_| ActiveRoutine {
            activity: Activity::Movement,
            offer: None,
        })
    }
}

fn ms_between(a: DateTime<Utc>, b: DateTime<Utc>) -> u64 {
    u64::try_from((b - a).num_milliseconds()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ctx(wall: DateTime<Utc>, elapsed: Duration) -> Context {
        Context {
            wall,
            elapsed,
            zone: UserZone::KOLKATA,
            schedule: WorkSchedule::default(),
            water: WaterSnapshot::default(),
            water_entries: Vec::new(),
            config: CoachConfig::default(),
            servings: Servings::default(),
        }
    }

    #[test]
    fn state_round_trips_through_json() {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 20, 4, 30, 0).unwrap();
        let mut e = Engine::new(7);
        e.step(&ctx(t0, Duration::ZERO), Input::Refresh);
        for i in 1..=300 {
            let t = t0 + chrono::Duration::seconds(5 * i);
            e.step(
                &ctx(t, Duration::from_secs(5)),
                Input::Tick {
                    idle: Duration::ZERO,
                },
            );
        }
        e.step(
            &ctx(t0 + chrono::Duration::seconds(1500), Duration::ZERO),
            Input::SetMeeting(true),
        );
        let text = serde_json::to_string(e.state()).unwrap();
        let back = Engine::from_state(serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn unknown_state_format_is_refused() {
        let mut state = Engine::new(1).state().clone();
        state.format = STATE_FORMAT + 1;
        assert!(Engine::from_state(state).is_err());
        let mut v = serde_json::to_value(Engine::new(1).state()).unwrap();
        v["surprise"] = serde_json::json!(true);
        assert!(serde_json::from_value::<EngineState>(v).is_err());
    }

    #[test]
    fn offer_ids_are_unique_per_salt_and_sequence() {
        let a = Engine::new(1);
        let b = Engine::new(2);
        let ids: std::collections::HashSet<_> = (1..=1000)
            .flat_map(|seq| [a.offer_id(seq), b.offer_id(seq)])
            .collect();
        assert_eq!(ids.len(), 2000);
        assert_eq!(a.offer_id(1).get_version_num(), 4);
    }
}
