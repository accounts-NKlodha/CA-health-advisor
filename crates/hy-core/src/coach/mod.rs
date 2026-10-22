//! The activity tracker and coaching engine (slice V1-2; architecture 4.2 and 4.3).
//!
//! - [`engine`]: the pure, deterministic state machine. It is the only component that
//!   decides to show a reminder. It reads no clock and does no I/O, so traces replay
//!   exactly (architecture 11, T-R1 to T-R6).
//! - [`driver`]: [`Coach`], the engine bound to the store and the injected clock. The shell
//!   (V1-1) owns one `Coach`, feeds it [`Input`]s (the 5-second idle poll, lock, unlock,
//!   sleep, wake, presentation state and the user's answers) and renders the returned
//!   [`CoachView`]. The shell never shows a reminder on its own.
//! - [`config`]: the user's coaching settings and the fixed timings.
//! - [`water`]: the water pace model.
//! - [`movement`]: the guided movement routine's bundled content (provisional, pending
//!   the content review).
//! - [`recovery`]: the deadline day and the one pending recovery-day plan.
//! - [`types`]: inputs, the view, and notices.
//!
//! Persistence: the engine state, the reminder events and the per-day tracker tallies of
//! each step are committed in one transaction, ordered by a durable revision (migration
//! 0002). Nothing depends on wall-clock or identifier order.

pub mod config;
pub mod driver;
pub mod engine;
pub mod movement;
pub mod recovery;
pub mod types;
pub mod water;

pub use config::{CoachConfig, Intensity, PomodoroPreset};
pub use driver::Coach;
pub use engine::{ActivityState, Context, DayTally, Engine, EngineState, StepOutput};
pub use recovery::{RecoveryAction, RecoveryPlan, RecoveryView, Reflection};
pub use types::{
    Bubble, CheckInAnswer, Chip, ChipStatus, CoachView, Confirmation, FocusEnd, GuideAction,
    GuideStage, GuideView, HiddenReason, Hold, Input, InputOutcome, Notice, PauseLength, PetMode,
    PomodoroPause, PomodoroPhase, PomodoroView, ReminderReason, Response,
};
pub use water::{WaterPace, WaterSnapshot};
