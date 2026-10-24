//! The V2 posture check's camera session (slice V2-1; architecture 4.5).
//!
//! This module holds the rules; the companion owns the camera page (its own WebView2
//! window, created only while the camera is on) and feeds this state machine what the
//! page and Windows report. Nothing here touches a camera, a window or a clock, so every
//! rule is tested with synthetic times (architecture 11: T-C5 worker stall).
//!
//! - **Consent and off by default.** The camera never starts without a recorded consent
//!   for the current notice version ([`NOTICE_VERSION`]). The user's choice to keep it on
//!   is stored ([`CameraSettings`]); "Turn camera off" stops capture at once.
//! - **Frames never reach this code.** The page runs MediaPipe pose lite in a Web Worker
//!   and sends one small [`FrameReport`] per frame: whether the frame was usable and, if
//!   not, why. Landmarks stay in the worker for one frame; frames are never stored or sent.
//! - **No reading is explicit.** A frame without a person, with low visibility or with
//!   the shoulders outside the frame is a "no reading" with its reason, never a score
//!   ([`Status::NoReading`]).
//! - **Conflicts.** Windows denying the camera, another app holding it, no camera, or a
//!   model that cannot load each give a named [`Unavailable`] state while the pet and every
//!   V1 feature go on. A busy or missing camera is tried again every
//!   [`RETRY_UNAVAILABLE_EVERY`]; a denied one waits for the user.
//! - **Holds.** Capture is released (the camera light goes off) while the PC is locked or
//!   asleep, coaching is paused, outside work hours, during a presentation or full-screen
//!   app and during a meeting ([`CameraHold`]); it starts again when the hold ends. Focus
//!   is not a hold: posture cues are dropped during focus (slice V2-2), capture goes on.
//! - **Watchdog.** No frame result for [`STALL_AFTER`] while capturing gives "no reading
//!   (stalled)" and one worker restart; a second stall within [`STALL_WINDOW`] stops the
//!   camera and tells the user.
//! - **Stop.** Stop asks the page to end the track and close its worker, then destroys
//!   the page as soon as it confirms, or after [`CLOSE_AFTER`] without confirmation, so
//!   capture always ends within [`STOP_WITHIN`] (PRD: two seconds).
//! - **Posture (slice V2-2).** The machine owns the [`crate::posture::Detector`]: it
//!   feeds it each frame's features, the stream's starts and ends and the reading's
//!   changes, and carries out its actions (the cue, records, saving the calibration, the
//!   one request to recalibrate). The page runs at [`FPS_HIGH`] while a calibration
//!   collects frames.
//! - **Budget.** The page runs pose lite at [`FPS_HIGH`] frames a second and drops to
//!   [`FPS_LOW`] when inference would use more than its share of the PC
//!   ([`FpsGovernor`]); camera mode as a whole must stay within 10% CPU and 350 MB
//!   (architecture 4.5, batch 1 C1 and C2).

use std::collections::VecDeque;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::posture::{
    Calibration, CueOutcome, Detector, Features, Observation, PostureAction, PostureReason,
    PostureState, PostureStats,
};

/// Settings key under which [`CameraSettings`] is stored.
pub const SETTINGS_KEY: &str = "camera";

/// The version of the consent text (wireframe W16). A new version asks again.
pub const NOTICE_VERSION: u32 = 1;

/// No frame result for this long while capturing is a stall (architecture 4.5).
pub const STALL_AFTER: Duration = Duration::from_secs(5);
/// A second stall within this window stops the camera (architecture 4.5).
pub const STALL_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Capture must end within this long of "off" (PRD V2 gate: release blocker).
pub const STOP_WITHIN: Duration = Duration::from_secs(2);
/// The page is destroyed this long after a stop request it has not confirmed, which
/// releases the camera even if the page hangs (D; leaves margin inside [`STOP_WITHIN`]).
pub const CLOSE_AFTER: Duration = Duration::from_secs(1);
/// The page must report the camera and the model ready within this long (D).
pub const START_TIMEOUT: Duration = Duration::from_secs(30);
/// A busy or missing camera is tried again this often while the camera is on (D).
pub const RETRY_UNAVAILABLE_EVERY: Duration = Duration::from_secs(60);
/// The reading shown is the majority of the frames of this window (D), so one odd frame
/// never flips the status.
pub const READING_WINDOW: Duration = Duration::from_secs(3);

/// Frames a second while within budget (architecture 4.5: 2 to 3 fps; batch 1 ran 3).
pub const FPS_HIGH: u8 = 3;
/// Frames a second when over budget (architecture 4.5).
pub const FPS_LOW: u8 = 1;
/// The share of the whole PC that pose inference may use at [`FPS_HIGH`] before the
/// page drops to [`FPS_LOW`] (D). The camera budget is 10% for everything (capture,
/// decoding, the page and inference); this leaves the rest for the others.
pub const INFERENCE_SHARE_MAX: f64 = 0.06;
/// At [`FPS_LOW`], the page returns to [`FPS_HIGH`] only when inference at the high rate
/// would use less than this share (D; hysteresis against flapping).
pub const INFERENCE_SHARE_RETURN: f64 = 0.04;
/// A single inference longer than this cannot keep [`FPS_HIGH`] at all (D).
pub const INFERENCE_MS_MAX_HIGH: f64 = 250.0;
/// Inference times averaged for the decision (D).
const GOVERNOR_FRAMES: usize = 10;
/// How long inference must stay cheap at [`FPS_LOW`] before returning (D).
const GOVERNOR_RETURN_AFTER: Duration = Duration::from_secs(60);

/// The recorded consent (wireframe W16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    /// The notice version the user agreed to.
    pub notice_version: u32,
    /// When.
    pub given_at: DateTime<Utc>,
}

/// What is stored about the camera: numbers and choices only.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraSettings {
    /// The user's consent, if given.
    pub consent: Option<Consent>,
    /// The user wants the posture check on (it starts again after a restart). Off by
    /// default.
    pub on: bool,
    /// The posture calibration (slice V2-2): numbers only, never an image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<Calibration>,
}

impl CameraSettings {
    /// Whether the consent on record covers the current notice.
    #[must_use]
    pub fn consent_current(&self) -> bool {
        self.consent
            .is_some_and(|c| c.notice_version == NOTICE_VERSION)
    }
}

/// Why capture is released for a while (architecture 4.5 "Pausing and stop").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraHold {
    /// The session is locked or the PC asleep.
    Away,
    /// The user paused coaching.
    Paused,
    /// Outside work hours.
    OffHours,
    /// A presentation or full-screen app.
    Presentation,
    /// The manual meeting toggle (or the experimental camera-in-use hint).
    Meeting,
}

/// The engine's holds that matter to the camera, from its view
/// ([`crate::coach::CoachView::holds`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldSources {
    /// Locked, asleep, or the time the app was closed.
    pub away: bool,
    /// Coaching paused by the user.
    pub paused: bool,
    /// Outside work hours.
    pub off_hours: bool,
    /// Quiet focus (not a camera hold; cues are dropped instead, slice V2-2).
    pub focus: bool,
    /// Windows reports a presentation or full-screen app, whatever the hiding setting.
    pub presenting: bool,
    /// The meeting toggle, or the experimental media hint when switched on.
    pub meeting: bool,
}

impl HoldSources {
    /// The hold that releases the camera, if any (the strongest first).
    #[must_use]
    pub const fn camera_hold(&self) -> Option<CameraHold> {
        if self.away {
            Some(CameraHold::Away)
        } else if self.paused {
            Some(CameraHold::Paused)
        } else if self.off_hours {
            Some(CameraHold::OffHours)
        } else if self.presenting {
            Some(CameraHold::Presentation)
        } else if self.meeting {
            Some(CameraHold::Meeting)
        } else {
            None
        }
    }
}

/// Why the camera cannot be used (wireframe W16 access outcomes, W10 statuses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    /// Windows (or the app's permission) refused the camera. Waits for the user.
    Denied,
    /// Another app holds the camera. Tried again every minute.
    Busy,
    /// No camera was found. Tried again every minute.
    NoCamera,
    /// The bundled pose model could not load.
    ModelFailed,
    /// The page stalled twice within ten minutes; the camera was stopped.
    Stalled,
    /// The page did not start within [`START_TIMEOUT`] or failed in another way.
    Failed,
}

impl Unavailable {
    /// The state for a `getUserMedia` error name (a `DOMException` name).
    #[must_use]
    pub fn from_error_name(name: &str) -> Self {
        match name {
            "NotAllowedError" | "SecurityError" | "PermissionDeniedError" => Self::Denied,
            "NotReadableError" | "TrackStartError" | "AbortError" => Self::Busy,
            "NotFoundError" | "DevicesNotFoundError" | "OverconstrainedError" => Self::NoCamera,
            _ => Self::Failed,
        }
    }

    /// Whether the page stays open and tries again by itself.
    #[must_use]
    pub const fn retries(self) -> bool {
        matches!(self, Self::Busy | Self::NoCamera)
    }
}

/// Why a frame gave no reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameIssue {
    /// No person found.
    Absent,
    /// A person, but the nose, eyes or ears, or shoulders were not visible enough.
    LowConfidence,
    /// A shoulder is outside the frame.
    OutOfFrame,
}

/// Why there is no reading now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoReading {
    /// The camera and model are ready but no frame has been read yet.
    Starting,
    /// Mostly unusable frames, for this reason.
    Frame(FrameIssue),
    /// No frame result for [`STALL_AFTER`].
    Stalled,
}

/// The posture check's state, as the pet, tray, settings and camera page show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Status {
    /// Off (the default).
    Off,
    /// The camera page is starting the camera and the model.
    Starting,
    /// Capturing, with usable frames.
    Reading,
    /// Capturing, but no usable reading.
    NoReading(NoReading),
    /// On, but capture is released during a hold.
    Held(CameraHold),
    /// On, but the camera cannot be used.
    Unavailable(Unavailable),
}

impl Status {
    /// Whether the camera may be capturing now (the CAM badge is shown).
    #[must_use]
    pub const fn capturing(self) -> bool {
        matches!(self, Self::Starting | Self::Reading | Self::NoReading(_))
    }

    /// The short words for the pet's tag, tray and settings (wireframe W10 and the state
    /// table).
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::Off => "Camera off",
            Self::Starting => "Camera starting",
            Self::Reading => "Camera on",
            Self::NoReading(NoReading::Stalled) => "Cam: no reading (stalled)",
            Self::NoReading(_) => "Cam: no reading",
            Self::Held(CameraHold::Away) => "Camera paused: away",
            Self::Held(CameraHold::Paused) => "Camera paused: coaching paused",
            Self::Held(CameraHold::OffHours) => "Camera paused: off hours",
            Self::Held(CameraHold::Presentation) => "Camera paused: presentation",
            Self::Held(CameraHold::Meeting) => "Camera paused: meeting",
            Self::Unavailable(Unavailable::Denied) => "Camera blocked by Windows",
            Self::Unavailable(Unavailable::Busy) => "Camera in use by another app",
            Self::Unavailable(Unavailable::NoCamera) => "No camera found",
            Self::Unavailable(Unavailable::ModelFailed) => "Posture model could not load",
            Self::Unavailable(Unavailable::Stalled) => "Camera stopped: not responding",
            Self::Unavailable(Unavailable::Failed) => "Camera could not start",
        }
    }
}

/// One frame's result from the page. Derived values only: never pixels or landmarks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameReport {
    /// The frame meets the valid-frame rule (architecture 4.5): nose, both eyes or both
    /// ears, and both shoulders with visibility of at least 0.5, both shoulders inside
    /// the frame.
    pub valid: bool,
    /// Why not, when not valid.
    pub issue: Option<FrameIssue>,
    /// Inference time for this frame, in milliseconds.
    pub inference_ms: f64,
    /// Mean visibility of the required landmarks (0 with no person).
    pub visibility: f64,
    /// The posture features of a valid frame (slice V2-2); landmarks stay in the worker.
    #[serde(default)]
    pub features: Option<Features>,
}

impl FrameReport {
    /// Check the report's own rules (it comes from a page).
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        if !self.inference_ms.is_finite() || !(0.0..=60_000.0).contains(&self.inference_ms) {
            return Err(Error::invalid("inference time must be 0 to 60 000 ms"));
        }
        if !self.visibility.is_finite() || !(0.0..=1.0).contains(&self.visibility) {
            return Err(Error::invalid("visibility must be 0 to 1"));
        }
        if self.valid == self.issue.is_some() {
            return Err(Error::invalid(
                "a frame is either valid or has an issue, not both",
            ));
        }
        if let Some(f) = &self.features {
            if !self.valid {
                return Err(Error::invalid("only a valid frame has posture features"));
            }
            f.validate()?;
        }
        Ok(())
    }
}

/// What the page reports when the camera and model are ready.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Started {
    /// The track's width in pixels.
    pub width: u32,
    /// The track's height in pixels.
    pub height: u32,
    /// Logical processors the page sees (`navigator.hardwareConcurrency`).
    pub cores: u32,
    /// Model load time in milliseconds.
    pub model_ms: f64,
}

/// Something the companion must do for the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Create the camera page (if it is not open) and start capture there.
    OpenPage,
    /// Ask the page to end its track and close its worker, then wait for
    /// [`Machine::page_stopped`] or [`CLOSE_AFTER`].
    RequestStop,
    /// Destroy the camera page (its memory and any capture end with it).
    ClosePage,
    /// Ask the page to restart its worker (the watchdog's one restart).
    RestartWorker,
    /// Ask the page to try the camera again (busy or missing).
    Retry,
    /// Tell the page the frame rate to run.
    SetFps(u8),
    /// Save [`Machine::settings`].
    Persist,
    /// Tell the user something once, beside the pet.
    Notify(CameraNotice),
    /// The status changed: redraw the pet's badge, the tray and open pages.
    Render,
    /// A posture drift episode started: ask the coaching engine for the cue, then report
    /// what became of it ([`Machine::cue_outcome`]).
    Cue,
    /// Record this posture observation (local only).
    Record(Observation),
}

/// A one-off message beside the pet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraNotice {
    /// The camera became unavailable while the camera page was not on screen.
    Unavailable(Unavailable),
    /// The camera was stopped after a second stall.
    StoppedAfterStalls,
    /// The camera was turned off ("Camera off. Capture stopped.").
    Off,
    /// The posture baseline no longer holds (the camera moved): recalibrate.
    Recalibrate,
}

/// The frame rate decision (architecture 4.5: drop to 1 fps if over budget).
#[derive(Debug, Clone, Default)]
pub struct FpsGovernor {
    recent: VecDeque<f64>,
    low: bool,
    /// Since when inference at [`FPS_LOW`] has looked cheap enough to return.
    cheap_since: Option<Duration>,
}

/// The share of the whole PC that inference taking `mean_ms` per frame uses at `fps`.
#[must_use]
pub fn inference_share(mean_ms: f64, fps: u8, cores: u32) -> f64 {
    mean_ms * f64::from(fps) / 1000.0 / f64::from(cores.max(1))
}

impl FpsGovernor {
    /// The frame rate to run now.
    #[must_use]
    pub const fn fps(&self) -> u8 {
        if self.low { FPS_LOW } else { FPS_HIGH }
    }

    /// The mean of the recent inference times, if any.
    #[must_use]
    pub fn mean_ms(&self) -> Option<f64> {
        if self.recent.is_empty() {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = self.recent.len() as f64;
        Some(self.recent.iter().sum::<f64>() / n)
    }

    /// Record one inference time; returns the new frame rate if it changed.
    pub fn record(&mut self, now: Duration, inference_ms: f64, cores: u32) -> Option<u8> {
        if self.recent.len() == GOVERNOR_FRAMES {
            self.recent.pop_front();
        }
        self.recent.push_back(inference_ms);
        // Decide only on a full window, so one slow first frame (model warm-up) does not
        // drop the rate.
        if self.recent.len() < GOVERNOR_FRAMES {
            return None;
        }
        let mean = self.mean_ms()?;
        let share_high = inference_share(mean, FPS_HIGH, cores);
        if self.low {
            if share_high < INFERENCE_SHARE_RETURN && mean <= INFERENCE_MS_MAX_HIGH {
                let since = *self.cheap_since.get_or_insert(now);
                if now.saturating_sub(since) >= GOVERNOR_RETURN_AFTER {
                    self.low = false;
                    self.cheap_since = None;
                    self.recent.clear();
                    return Some(FPS_HIGH);
                }
            } else {
                self.cheap_since = None;
            }
        } else if share_high > INFERENCE_SHARE_MAX || mean > INFERENCE_MS_MAX_HIGH {
            self.low = true;
            self.cheap_since = None;
            self.recent.clear();
            return Some(FPS_LOW);
        }
        None
    }

    /// Forget the recent times (a new capture), keeping the rate.
    pub fn reset_window(&mut self) {
        self.recent.clear();
        self.cheap_since = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Off,
    /// Capture asked for; waiting for the page's `started` (or an error).
    Starting {
        since: Duration,
    },
    /// The page reported the camera and model ready.
    Capturing {
        last_result: Duration,
    },
    Held(CameraHold),
    Unavailable {
        reason: Unavailable,
        retry_at: Option<Duration>,
    },
}

/// Measurements for the quality report and the self-test.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct CameraStats {
    /// Frame results received since the app started.
    pub frames: u64,
    /// Usable frames among them.
    pub valid_frames: u64,
    /// Watchdog stalls since the app started.
    pub stalls: u32,
    /// Worker restarts asked for.
    pub worker_restarts: u32,
    /// The last stop's time from request to the page confirming release, ms.
    pub last_stop_ms: Option<u64>,
    /// Whether the last stop needed the page to be destroyed unconfirmed.
    pub last_stop_forced: bool,
    /// The last model load time the page reported, ms.
    pub model_ms: Option<f64>,
    /// Mean of the recent inference times, ms.
    pub inference_ms: Option<f64>,
    /// The frame rate in force.
    pub fps: u8,
}

/// The camera session's state machine. Times are offsets from any fixed origin (the
/// companion uses its start), never wall-clock time.
#[derive(Debug, Clone)]
pub struct Machine {
    settings: CameraSettings,
    phase: Phase,
    hold: Option<CameraHold>,
    /// The page has been asked to open and has not been closed.
    page_open: bool,
    /// A stop was requested at this time and the page has not confirmed or been closed.
    stopping: Option<Duration>,
    stall_times: VecDeque<Duration>,
    window: VecDeque<(Duration, Option<FrameIssue>)>,
    cores: u32,
    governor: FpsGovernor,
    stats: CameraStats,
    status: Status,
    detector: Detector,
    /// The latest time the machine was given.
    now: Duration,
    /// The wall-clock time at offset zero (for the calibration's date).
    wall_origin: DateTime<Utc>,
}

impl Machine {
    /// A machine for the stored settings. It does nothing until [`Machine::resume`] (the
    /// app start) or a user action.
    #[must_use]
    pub fn new(settings: CameraSettings) -> Self {
        Self {
            settings,
            phase: Phase::Off,
            hold: None,
            page_open: false,
            stopping: None,
            stall_times: VecDeque::new(),
            window: VecDeque::new(),
            cores: 1,
            governor: FpsGovernor::default(),
            stats: CameraStats {
                fps: FPS_HIGH,
                ..CameraStats::default()
            },
            status: Status::Off,
            detector: Detector::new(settings.calibration),
            now: Duration::ZERO,
            wall_origin: DateTime::UNIX_EPOCH,
        }
    }

    /// Set the wall-clock time at offset zero (the companion's start).
    pub const fn set_wall_origin(&mut self, origin: DateTime<Utc>) {
        self.wall_origin = origin;
    }

    fn wall(&self, now: Duration) -> DateTime<Utc> {
        self.wall_origin + chrono::TimeDelta::from_std(now).unwrap_or_default()
    }

    /// The posture calibration's and baseline's state.
    #[must_use]
    pub fn posture_state(&self) -> PostureState {
        self.detector.state()
    }

    /// Posture counts so far.
    #[must_use]
    pub const fn posture_stats(&self) -> PostureStats {
        self.detector.stats()
    }

    /// "Calibrate": collect a calibration from the frames now coming in.
    ///
    /// # Errors
    /// [`Error::Invalid`] unless the camera is capturing.
    pub fn calibrate(&mut self, now: Duration) -> Result<Vec<Action>> {
        self.now = self.now.max(now);
        if !matches!(self.phase, Phase::Capturing { .. }) || self.stopping.is_some() {
            return Err(Error::invalid("calibration needs the camera on and reading"));
        }
        let before = self.fps();
        let fx = self.detector.start_calibration(now);
        let mut out = Vec::new();
        self.posture(fx, &mut out);
        self.fps_changed(before, &mut out);
        Ok(out)
    }

    /// "Cancel" on a calibration in progress.
    pub fn cancel_calibration(&mut self) -> Vec<Action> {
        let before = self.fps();
        let fx = self.detector.cancel_calibration();
        let mut out = Vec::new();
        self.posture(fx, &mut out);
        self.fps_changed(before, &mut out);
        out
    }

    /// What became of the cue asked for by [`Action::Cue`].
    pub fn cue_outcome(&mut self, outcome: CueOutcome) {
        self.detector.cue_outcome(outcome);
    }

    /// The stored settings as they stand.
    #[must_use]
    pub const fn settings(&self) -> CameraSettings {
        self.settings
    }

    /// The status now.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// Measurements so far.
    #[must_use]
    pub const fn stats(&self) -> CameraStats {
        self.stats
    }

    /// The frame rate the page should run (the full rate while calibrating).
    #[must_use]
    pub const fn fps(&self) -> u8 {
        if self.detector.calibrating() {
            FPS_HIGH
        } else {
            self.governor.fps()
        }
    }

    /// Whether the page should be capturing now (open and not asked to stop).
    #[must_use]
    pub const fn wants_capture(&self) -> bool {
        matches!(self.phase, Phase::Starting { .. } | Phase::Capturing { .. })
            && self.stopping.is_none()
    }

    /// Whether a page is open (or asked to open) and not yet closed.
    #[must_use]
    pub const fn page_open(&self) -> bool {
        self.page_open
    }

    // ---------------------------------------------------------------- user actions

    /// The app started: if the user left the posture check on (with a current consent),
    /// start it, unless a hold applies (the last one given to [`Machine::set_hold`]).
    pub fn resume(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        if self.settings.on && self.settings.consent_current() {
            let mut out = Vec::new();
            self.start(now, &mut out);
            out
        } else {
            if self.settings.on {
                // A consent for an older notice: wait for the user to agree again.
                self.settings.on = false;
                return vec![Action::Persist];
            }
            Vec::new()
        }
    }

    /// The user agreed to the consent text (wireframe W16 "Turn on camera").
    pub fn consent(&mut self, at: DateTime<Utc>) -> Vec<Action> {
        self.settings.consent = Some(Consent {
            notice_version: NOTICE_VERSION,
            given_at: at,
        });
        vec![Action::Persist]
    }

    /// "Turn on camera". Refused (returns `Err`) without a current consent, so the caller
    /// shows the consent text first.
    ///
    /// # Errors
    /// [`Error::Invalid`] if no current consent is recorded.
    pub fn turn_on(&mut self, now: Duration) -> Result<Vec<Action>> {
        self.now = self.now.max(now);
        if !self.settings.consent_current() {
            return Err(Error::invalid("the camera needs your consent first"));
        }
        let mut out = Vec::new();
        if !self.settings.on {
            self.settings.on = true;
            out.push(Action::Persist);
        }
        match self.phase {
            Phase::Off | Phase::Unavailable { .. } => self.start(now, &mut out),
            Phase::Starting { .. } | Phase::Capturing { .. } | Phase::Held(_) => {}
        }
        Ok(out)
    }

    /// "Turn camera off" (from the camera page, the pet menu, the tray or settings).
    pub fn turn_off(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        if self.settings.on {
            self.settings.on = false;
            out.push(Action::Persist);
        }
        let was_on = self.phase != Phase::Off;
        self.release(now, &mut out);
        self.phase = Phase::Off;
        if was_on {
            out.push(Action::Notify(CameraNotice::Off));
        }
        self.refresh(&mut out);
        out
    }

    /// Withdraw consent: the camera goes off and asks again next time.
    pub fn withdraw_consent(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = self.turn_off(now);
        if self.settings.consent.take().is_some() && !out.contains(&Action::Persist) {
            out.push(Action::Persist);
        }
        out
    }

    /// "Try again" after the camera was denied, failed or stopped.
    pub fn try_again(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        if self.settings.on && matches!(self.phase, Phase::Unavailable { .. }) {
            self.start(now, &mut out);
        }
        out
    }

    // ---------------------------------------------------------------- holds

    /// The engine's holds changed.
    pub fn set_hold(&mut self, now: Duration, hold: Option<CameraHold>) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        if self.hold == hold {
            return out;
        }
        self.hold = hold;
        if !self.settings.on {
            return out;
        }
        match (hold, self.phase) {
            (Some(h), Phase::Starting { .. } | Phase::Capturing { .. }) => {
                self.release(now, &mut out);
                self.phase = Phase::Held(h);
            }
            (Some(h), Phase::Unavailable { reason, .. }) if reason.retries() => {
                // A busy or missing camera is not tried during a hold.
                self.release(now, &mut out);
                self.phase = Phase::Held(h);
            }
            (Some(h), Phase::Held(_)) => self.phase = Phase::Held(h),
            (None, Phase::Held(_)) => self.start(now, &mut out),
            _ => {}
        }
        self.refresh(&mut out);
        out
    }

    // ---------------------------------------------------------------- page reports

    /// The page has the camera track and the model loaded.
    pub fn page_started(&mut self, now: Duration, started: Started) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        if !matches!(
            self.phase,
            Phase::Starting { .. } | Phase::Unavailable { .. }
        ) || self.stopping.is_some()
            || !self.settings.on
        {
            // A late report from a page asked to stop: make sure it lets go.
            if self.page_open && self.stopping.is_none() && !self.wants_capture() {
                self.release(now, &mut out);
            }
            return out;
        }
        self.cores = started.cores.max(1);
        self.stats.model_ms = Some(started.model_ms);
        self.governor.reset_window();
        self.window.clear();
        self.phase = Phase::Capturing { last_result: now };
        out.push(Action::SetFps(self.fps()));
        let fx = self.detector.capture_started(now);
        self.posture(fx, &mut out);
        self.refresh(&mut out);
        out
    }

    /// The page could not get the camera (a `getUserMedia` error name) or the model
    /// (`model: true`).
    pub fn page_error(&mut self, now: Duration, name: &str, model: bool) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        if !self.settings.on || self.stopping.is_some() || matches!(self.phase, Phase::Held(_)) {
            return out;
        }
        let reason = if model {
            Unavailable::ModelFailed
        } else {
            Unavailable::from_error_name(name)
        };
        self.unavailable(now, reason, &mut out);
        out
    }

    /// One frame's result.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the report breaks its rules (nothing changes).
    pub fn frame(&mut self, now: Duration, report: FrameReport) -> Result<Vec<Action>> {
        self.now = self.now.max(now);
        report.validate()?;
        let mut out = Vec::new();
        let Phase::Capturing { .. } = self.phase else {
            return Ok(out);
        };
        if self.stopping.is_some() {
            return Ok(out);
        }
        self.phase = Phase::Capturing { last_result: now };
        self.stats.frames += 1;
        if report.valid {
            self.stats.valid_frames += 1;
        }
        self.window.push_back((now, report.issue));
        while self
            .window
            .front()
            .is_some_and(|(t, _)| now.saturating_sub(*t) > READING_WINDOW)
        {
            self.window.pop_front();
        }
        let before = self.fps();
        self.governor.record(now, report.inference_ms, self.cores);
        self.stats.inference_ms = self.governor.mean_ms();
        let wall = self.wall(now);
        let fx = self
            .detector
            .frame(now, wall, report.visibility, report.features);
        self.posture(fx, &mut out);
        self.fps_changed(before, &mut out);
        self.stats.fps = self.fps();
        self.refresh(&mut out);
        Ok(out)
    }

    /// The page confirmed its track ended and its worker closed.
    pub fn page_stopped(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        if let Some(asked) = self.stopping.take() {
            self.stats.last_stop_ms = Some(millis(now.saturating_sub(asked)));
            self.stats.last_stop_forced = false;
            self.page_open = false;
            out.push(Action::ClosePage);
        }
        out
    }

    /// The camera page went away without being asked (its window was destroyed or its
    /// web content crashed).
    pub fn page_gone(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        let was_open = self.page_open;
        self.page_open = false;
        if self.stopping.take().is_some() {
            return out;
        }
        if was_open && self.settings.on {
            match self.phase {
                Phase::Starting { .. } | Phase::Capturing { .. } => {
                    self.unavailable(now, Unavailable::Failed, &mut out);
                }
                // A busy or missing camera's page closed by the user: the retry goes on
                // and reopens the page (hidden) when it is due.
                _ => {}
            }
        }
        out
    }

    // ---------------------------------------------------------------- time

    /// Called about once a second: the watchdog, the start timeout, the stop deadline and
    /// the retry of a busy or missing camera.
    pub fn tick(&mut self, now: Duration) -> Vec<Action> {
        self.now = self.now.max(now);
        let mut out = Vec::new();
        let before = self.fps();
        let wall = self.wall(now);
        let fx = self.detector.tick(now, wall);
        self.posture(fx, &mut out);
        self.fps_changed(before, &mut out);
        if let Some(asked) = self.stopping
            && now.saturating_sub(asked) >= CLOSE_AFTER
        {
            self.stopping = None;
            self.page_open = false;
            self.stats.last_stop_ms = Some(millis(now.saturating_sub(asked)));
            self.stats.last_stop_forced = true;
            out.push(Action::ClosePage);
        }
        match self.phase {
            Phase::Starting { since } if now.saturating_sub(since) >= START_TIMEOUT => {
                self.unavailable(now, Unavailable::Failed, &mut out);
            }
            Phase::Capturing { last_result }
                if now.saturating_sub(last_result) >= STALL_AFTER && self.stopping.is_none() =>
            {
                self.stall(now, &mut out);
            }
            Phase::Unavailable {
                retry_at: Some(at), ..
            } if now >= at && self.hold.is_none() => {
                self.phase = Phase::Starting { since: now };
                // The page stays open while it retries; it is reopened if it went.
                out.push(if self.page_open {
                    Action::Retry
                } else {
                    self.page_open = true;
                    Action::OpenPage
                });
            }
            _ => {}
        }
        self.refresh(&mut out);
        out
    }

    // ---------------------------------------------------------------- internals

    fn start(&mut self, now: Duration, out: &mut Vec<Action>) {
        if let Some(h) = self.hold {
            self.phase = Phase::Held(h);
            self.refresh(out);
            return;
        }
        self.phase = Phase::Starting { since: now };
        self.window.clear();
        if self.page_open && self.stopping.is_none() {
            out.push(Action::Retry);
        } else {
            self.page_open = true;
            out.push(Action::OpenPage);
        }
        self.refresh(out);
    }

    fn release(&mut self, now: Duration, out: &mut Vec<Action>) {
        let fx = self.detector.capture_ended(now);
        self.posture(fx, out);
        if self.page_open && self.stopping.is_none() {
            self.stopping = Some(now);
            out.push(Action::RequestStop);
        }
        self.window.clear();
    }

    fn unavailable(&mut self, now: Duration, reason: Unavailable, out: &mut Vec<Action>) {
        let retry_at = reason.retries().then(|| now + RETRY_UNAVAILABLE_EVERY);
        if !reason.retries() {
            self.release(now, out);
        }
        self.phase = Phase::Unavailable { reason, retry_at };
        out.push(Action::Notify(CameraNotice::Unavailable(reason)));
        self.refresh(out);
    }

    fn stall(&mut self, now: Duration, out: &mut Vec<Action>) {
        self.stats.stalls += 1;
        while self
            .stall_times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) > STALL_WINDOW)
        {
            self.stall_times.pop_front();
        }
        self.stall_times.push_back(now);
        if self.stall_times.len() >= 2 {
            // Stop the camera and tell the user; they can turn it on again.
            self.stall_times.clear();
            self.settings.on = false;
            out.push(Action::Persist);
            self.release(now, out);
            self.phase = Phase::Unavailable {
                reason: Unavailable::Stalled,
                retry_at: None,
            };
            out.push(Action::Notify(CameraNotice::StoppedAfterStalls));
        } else {
            // One worker restart. "No reading (stalled)" stays until a frame arrives, and
            // the restarted worker gets a fresh stall timer from now.
            self.stats.worker_restarts += 1;
            self.window.clear();
            self.phase = Phase::Capturing { last_result: now };
            out.push(Action::RestartWorker);
            if self.status != Status::NoReading(NoReading::Stalled) {
                self.status = Status::NoReading(NoReading::Stalled);
                out.push(Action::Render);
            }
            return;
        }
        self.refresh(out);
    }

    fn computed_status(&self) -> Status {
        match self.phase {
            Phase::Off => Status::Off,
            Phase::Starting { .. } => Status::Starting,
            Phase::Held(h) => Status::Held(h),
            Phase::Unavailable { reason, .. } => Status::Unavailable(reason),
            Phase::Capturing { .. } => {
                if self.window.is_empty() {
                    // Keep "stalled" until a frame arrives after a worker restart.
                    if self.status == Status::NoReading(NoReading::Stalled) {
                        return self.status;
                    }
                    return Status::NoReading(NoReading::Starting);
                }
                let valid = self.window.iter().filter(|(_, i)| i.is_none()).count();
                if valid * 2 >= self.window.len() {
                    Status::Reading
                } else {
                    Status::NoReading(NoReading::Frame(self.common_issue()))
                }
            }
        }
    }

    fn common_issue(&self) -> FrameIssue {
        let count = |issue| {
            self.window
                .iter()
                .filter(|(_, i)| *i == Some(issue))
                .count()
        };
        [
            FrameIssue::Absent,
            FrameIssue::LowConfidence,
            FrameIssue::OutOfFrame,
        ]
        .into_iter()
        .max_by_key(|i| count(*i))
        .unwrap_or(FrameIssue::Absent)
    }

    fn refresh(&mut self, out: &mut Vec<Action>) {
        let status = self.computed_status();
        if status != self.status {
            self.status = status;
            if !out.contains(&Action::Render) {
                out.push(Action::Render);
            }
            let reason = match status {
                Status::NoReading(NoReading::Frame(FrameIssue::Absent)) => {
                    Some(PostureReason::Absent)
                }
                Status::NoReading(NoReading::Frame(FrameIssue::LowConfidence)) => {
                    Some(PostureReason::LowConfidence)
                }
                Status::NoReading(NoReading::Frame(FrameIssue::OutOfFrame)) => {
                    Some(PostureReason::OutOfFrame)
                }
                Status::NoReading(NoReading::Stalled) => Some(PostureReason::Stalled),
                _ => None,
            };
            let now = self.last_now();
            let fx = self.detector.reading_changed(now, reason);
            self.posture(fx, out);
        }
    }

    /// The time of the latest event the machine saw.
    const fn last_now(&self) -> Duration {
        self.now
    }

    /// Carry out the detector's actions.
    fn posture(&mut self, fx: Vec<PostureAction>, out: &mut Vec<Action>) {
        for a in fx {
            let action = match a {
                PostureAction::Cue => Action::Cue,
                PostureAction::Record(o) => Action::Record(o),
                PostureAction::Persist => {
                    self.settings.calibration = self.detector.calibration();
                    Action::Persist
                }
                PostureAction::AskRecalibrate => Action::Notify(CameraNotice::Recalibrate),
                PostureAction::Render => Action::Render,
            };
            if !(matches!(action, Action::Render | Action::Persist) && out.contains(&action)) {
                out.push(action);
            }
        }
    }

    /// Tell the page a new frame rate if it changed (a calibration starting or ending, or
    /// the governor).
    fn fps_changed(&self, before: u8, out: &mut Vec<Action>) {
        let fps = self.fps();
        if fps != before {
            out.retain(|a| !matches!(a, Action::SetFps(_)));
            out.push(Action::SetFps(fps));
        }
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
