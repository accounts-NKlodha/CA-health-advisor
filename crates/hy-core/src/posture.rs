//! The V2 posture check's detector (slice V2-2; architecture 4.5): calibration quality,
//! scale floors, sustained drift, the viewpoint check and the cue.
//!
//! The camera page's pose worker turns each usable frame into a few numbers
//! ([`Features`]); landmarks never leave the worker. This module keeps those numbers in
//! memory for at most [`HISTORY_FOR`] and decides, on synthetic times in the tests
//! (architecture 11: T-C1 to T-C4 and T-C6):
//!
//! - **Calibration.** The user sits as they normally do while the page collects
//!   [`CAL_FRAMES`] valid frames within [`CAL_WITHIN`]. It is usable only if every check
//!   passes ([`CalibrationIssue`]); otherwise the page says "Calibration not usable" with
//!   the reason and no cue is given. Only the numbers are stored ([`Calibration`]).
//! - **Scale floors.** Each feature's z-score divides by `max(1.4826 x MAD, floor)`
//!   ([`FLOOR_GAP`] and the others), so a perfectly still calibration can never make small
//!   jitter look large (T-C1, T-C2).
//! - **Drift.** The features are averaged over [`SMOOTH_OVER`]. When the largest smoothed
//!   |z| stays at or above [`Z_DRIFT`] for [`EPISODE_AFTER`] with at least
//!   [`EPISODE_MIN_VALID`] of frames usable, an episode starts and a cue is asked for
//!   ([`PostureAction::Cue`]); the coaching engine decides whether it shows (cooldown,
//!   focus, holds). Reaching, turning to a second display or standing up do not last a
//!   minute, so they never cue (T-C3). One episode is one event, written when it ends.
//! - **Viewpoint check.** Cues are suspended at once (the baseline is *suspect*) when the
//!   camera stream starts again, when the shoulders' position or scale in the frame jumps
//!   within two seconds and stays, or when the head-to-shoulder width ratio leaves its
//!   calibration band while posture stays steady and the head faces as it did at
//!   calibration. Only shoulder geometry and the head pair's width are used, so face or
//!   head motion alone never triggers it. A suspect baseline that does not match the
//!   calibration again within [`SUSPECT_FOR`] of reading becomes invalid: the user is
//!   asked once to recalibrate, and nothing is scored until they do (T-C4).
//! - **Restart.** A stored calibration comes back suspect; it is accepted if the first
//!   [`RESTART_CHECK`] of reading match it, and is otherwise invalid (T-C6).
//! - **Records.** `posture.observation` events only: drift episodes with what became of
//!   the cue, no-reading intervals with their reason, and suspect intervals with theirs.
//!   Slice V2-3 adds camera-on time ([`PostureKind::Capture`]) and the time with no
//!   usable calibration ([`PostureReason::NoCalibration`]), so a day's coverage can be
//!   derived from the records alone ([`quality::coverage`]). Open intervals are written
//!   at least every [`SEGMENT_EVERY`], so a crash loses little and a day's records stay
//!   close to its date. They stay on this PC.
//! - **Thresholds.** Every tunable D value is in [`Thresholds`]; the shipped ones are
//!   [`Thresholds::PROVISIONAL`]. Slice V2-3's harness ([`desk`], [`quality`]) measures
//!   them on synthetic desk sessions; they stay provisional until the owner's real desk
//!   sessions pass the PRD V2 gate and the owner approves them.

use std::collections::VecDeque;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
pub use crate::schema::{CueOutcome, PostureKind, PostureReason};

pub mod desk;
pub mod quality;

/// Valid frames a calibration collects (architecture 4.5: at least 40).
pub const CAL_FRAMES: u32 = 40;
/// A calibration lasts at most this long (architecture 4.5: at most 30 s).
pub const CAL_WITHIN: Duration = Duration::from_secs(30);
/// The least share of a calibration's frames that must be valid (architecture 4.5).
pub const CAL_MIN_VALID_SHARE: f64 = 0.6;
/// Shoulder width as a share of the frame width must be at least this (architecture 4.5).
pub const CAL_WIDTH_MIN: f64 = 0.15;
/// ... and at most this (architecture 4.5).
pub const CAL_WIDTH_MAX: f64 = 0.70;
/// The calibration frames' mean visibility must be at least this (architecture 4.5).
pub const CAL_MIN_VISIBILITY: f64 = 0.7;
/// Each feature's calibration MAD may be at most this many times its floor (the user held
/// still; architecture 4.5).
pub const CAL_MAD_MAX_FLOORS: f64 = 3.0;

/// The MAD's scale to a standard deviation for normal data.
pub const MAD_SCALE: f64 = 1.4826;
/// Floor for the ear-to-shoulder gap ratio (architecture 4.5).
pub const FLOOR_GAP: f64 = 0.02;
/// Floor for the shoulder width against the baseline (architecture 4.5).
pub const FLOOR_WIDTH: f64 = 0.02;
/// Floor for the shoulder tilt (architecture 4.5).
pub const FLOOR_TILT: f64 = 0.015;
/// Floor for the head roll, degrees (architecture 4.5).
pub const FLOOR_ROLL: f64 = 2.0;

/// Features are averaged over this long (architecture 4.5).
pub const SMOOTH_OVER: Duration = Duration::from_secs(10);
/// A smoothed |z| at or above this is drift (D).
pub const Z_DRIFT: f64 = 3.0;
/// An episode ends once the smoothed |z| stays below this (D; hysteresis). Slice V2-3's
/// synthetic sweep moved it from 2.0: with 2.0 an episode could outlast its drift while
/// the person settled about two floors from calibration, hiding the next drift
/// (`docs/V2_Quality.md`). Provisional ...
pub const Z_END: f64 = 2.5;
/// ... for this long (D).
pub const END_AFTER: Duration = Duration::from_secs(10);
/// Drift must last this long to be an episode (architecture 4.5).
pub const EPISODE_AFTER: Duration = Duration::from_secs(60);
/// ... with at least this share of frames valid meanwhile (architecture 4.5).
pub const EPISODE_MIN_VALID: f64 = 0.7;

/// A step in the shoulders' position or scale must complete within this (architecture
/// 4.5).
pub const STEP_WITHIN: Duration = Duration::from_secs(2);
/// A step of at least this share of the frame in the shoulders' midpoint ...
pub const STEP_SHIFT: f64 = 0.15;
/// ... or of at least this in shoulder-width scale ...
pub const STEP_SCALE: f64 = 0.20;
/// ... held this long, makes the baseline suspect (architecture 4.5).
pub const STEP_HOLD: Duration = Duration::from_secs(20);
/// The head-to-shoulder width ratio's band is the calibration median plus or minus the
/// larger of 3 scaled MADs and this share of the median (D).
pub const SPAN_BAND_MIN: f64 = 0.10;
/// The head counts as facing as at calibration while its turn is within this of the
/// calibration median (D).
pub const TURN_TOLERANCE: f64 = 0.15;
/// The ratio must stay outside its band this long (D).
pub const VIEWPOINT_HOLD: Duration = Duration::from_secs(20);

/// A suspect baseline that has not matched again after this much reading is invalid
/// (architecture 4.5: 2 minutes).
pub const SUSPECT_FOR: Duration = Duration::from_secs(120);
/// A suspect baseline matches again after this long of continuous match (D): a run of
/// matching frames, each within [`READING_GAP`] of the one before. A frame without usable
/// features neither extends nor (on its own) breaks the run, but a gap longer than
/// [`READING_GAP`] between matching frames starts it again, and a frame that does not
/// match ends it.
pub const MATCH_FOR: Duration = Duration::from_secs(20);
/// After a restart, the first this-long of reading decides (architecture 4.5: 20 s).
pub const RESTART_CHECK: Duration = Duration::from_secs(20);
/// ... and at least this share of its frames must match (D).
pub const RESTART_MATCH_SHARE: f64 = 0.8;
/// Geometry matches the calibration while the shoulders' midpoint is within this share of
/// the frame of the calibrated one ... (D, half of [`STEP_SHIFT`])
pub const MATCH_SHIFT: f64 = 0.075;
/// ... and the shoulder width within this of the calibrated scale (D, half of
/// [`STEP_SCALE`]).
pub const MATCH_SCALE: f64 = 0.10;

/// Features are kept in memory at most this long (architecture 4.5: 90 s or less).
pub const HISTORY_FOR: Duration = Duration::from_secs(90);
/// Frames further apart than this do not count as continuous reading (D).
const READING_GAP: Duration = Duration::from_secs(2);
/// No-reading intervals shorter than this are not recorded (D).
pub const NO_READING_MIN: Duration = Duration::from_secs(5);
/// Open intervals (camera on, no reading, suspect, no calibration) are written at least
/// this often while capture runs, and start again (D, slice V2-3): a crash loses at most
/// this much, and a day's coverage is attributed to within this of its date.
pub const SEGMENT_EVERY: Duration = Duration::from_secs(15 * 60);

/// The tunable D values (architecture 4.5: "tuned in the desk test and frozen before
/// scoring"). The architecture's own numbers (the floors, 60 s, 70%, the step rule, 2
/// minutes, 20 s after a restart) are not here: changing them is a design change.
///
/// [`Thresholds::PROVISIONAL`] is what ships. It was checked only on synthetic desk
/// sessions ([`desk`]); it stays provisional until the owner's real desk sessions pass
/// the PRD V2 gate, and any change needs the owner's approval (architecture A4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    /// A smoothed |z| at or above this is drift.
    pub z_drift: f64,
    /// An episode ends once the smoothed |z| stays below this ...
    pub z_end: f64,
    /// ... for this many seconds.
    pub end_after_s: u64,
    /// The head-to-shoulder width ratio's band is at least this share of its median.
    pub span_band_min: f64,
    /// The head faces as at calibration while its turn is within this.
    pub turn_tolerance: f64,
    /// Geometry matches while the shoulders' midpoint is within this share of the frame.
    pub match_shift: f64,
    /// ... and the shoulder width within this of the calibrated scale.
    pub match_scale: f64,
}

impl Thresholds {
    /// The shipped values: provisional (synthetic traces only).
    pub const PROVISIONAL: Self = Self {
        z_drift: Z_DRIFT,
        z_end: Z_END,
        end_after_s: END_AFTER.as_secs(),
        span_band_min: SPAN_BAND_MIN,
        turn_tolerance: TURN_TOLERANCE,
        match_shift: MATCH_SHIFT,
        match_scale: MATCH_SCALE,
    };

    /// Check the values' own rules.
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let ok = positive(self.z_drift)
            && positive(self.z_end)
            && self.z_end < self.z_drift
            && (1..=120).contains(&self.end_after_s)
            && positive(self.span_band_min)
            && positive(self.turn_tolerance)
            && positive(self.match_shift)
            && self.match_shift < STEP_SHIFT
            && positive(self.match_scale)
            && self.match_scale < STEP_SCALE;
        if ok {
            Ok(())
        } else {
            Err(Error::invalid("posture thresholds out of range"))
        }
    }

    const fn end_after(&self) -> Duration {
        Duration::from_secs(self.end_after_s)
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Self::PROVISIONAL
    }
}

/// Which head landmarks a frame's features use: both ears, or both eyes when the ears
/// are not visible enough. A calibration uses one; frames with the other are not scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadRef {
    /// Both ears.
    Ears,
    /// Both eyes.
    Eyes,
}

/// One usable frame's numbers (architecture 4.5 "Features"). Hips and legs are ignored;
/// nothing here can be turned back into an image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    /// The head pair used.
    pub head: HeadRef,
    /// The vertical gap from the head pair's midpoint down to the shoulders' midpoint,
    /// divided by the shoulder width (slouching lowers it).
    pub gap: f64,
    /// The shoulder width as a share of the frame width (leaning in raises it).
    pub width: f64,
    /// The shoulders' height difference divided by their width (signed).
    pub tilt: f64,
    /// The head pair's angle to the horizontal, degrees (signed).
    pub roll: f64,
    /// The shoulders' midpoint, as shares of the frame width and height.
    pub mid_x: f64,
    /// See `mid_x`.
    pub mid_y: f64,
    /// The head pair's width divided by the shoulder width.
    pub span: f64,
    /// The nose's offset from the head pair's midpoint divided by the pair's width
    /// (turning the head moves it).
    pub turn: f64,
}

impl Features {
    /// Check the numbers' own rules (they come from a page).
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        let within = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let ok = within(self.gap, -10.0, 10.0)
            && self.width.is_finite()
            && self.width > 0.0
            && self.width <= 2.0
            && within(self.tilt, -10.0, 10.0)
            && within(self.roll, -180.0, 180.0)
            && within(self.mid_x, -1.0, 2.0)
            && within(self.mid_y, -1.0, 2.0)
            && self.span.is_finite()
            && self.span > 0.0
            && self.span <= 10.0
            && within(self.turn, -10.0, 10.0);
        if ok {
            Ok(())
        } else {
            Err(Error::invalid("posture features out of range"))
        }
    }
}

/// A feature's centre and spread at calibration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stat {
    /// Median.
    pub median: f64,
    /// Median absolute deviation.
    pub mad: f64,
}

impl Stat {
    fn of(values: &[f64]) -> Self {
        let median = median(values);
        let dev: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
        Self {
            median,
            mad: median_of(dev),
        }
    }

    /// The floored z-score of `x` (architecture 4.5).
    #[must_use]
    pub fn z(&self, x: f64, floor: f64) -> f64 {
        (x - self.median) / (MAD_SCALE * self.mad).max(floor)
    }
}

fn median(values: &[f64]) -> f64 {
    median_of(values.to_vec())
}

fn median_of(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        f64::midpoint(v[n / 2 - 1], v[n / 2])
    }
}

/// A usable calibration: numbers only (architecture 4.5 "Stored").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    /// When it was taken.
    pub at: DateTime<Utc>,
    /// The head pair it used.
    pub head: HeadRef,
    /// Ear-to-shoulder gap ratio.
    pub gap: Stat,
    /// Shoulder width (share of the frame width); scored as the ratio to its median.
    pub width: Stat,
    /// Shoulder tilt.
    pub tilt: Stat,
    /// Head roll, degrees.
    pub roll: Stat,
    /// Shoulders' midpoint, share of the frame width.
    pub mid_x: f64,
    /// Shoulders' midpoint, share of the frame height.
    pub mid_y: f64,
    /// Head-to-shoulder width ratio.
    pub span: Stat,
    /// Head turn.
    pub turn: Stat,
}

/// The four posture z-scores of some features against a calibration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scores {
    /// Gap ratio.
    pub gap: f64,
    /// Width against the baseline.
    pub width: f64,
    /// Tilt.
    pub tilt: f64,
    /// Head roll.
    pub roll: f64,
}

impl Scores {
    /// The largest |z|.
    #[must_use]
    pub const fn max_abs(&self) -> f64 {
        self.gap
            .abs()
            .max(self.width.abs())
            .max(self.tilt.abs())
            .max(self.roll.abs())
    }
}

impl Calibration {
    /// The width's relative spread (the MAD of the width divided by its median).
    fn width_rel(&self) -> Stat {
        Stat {
            median: 1.0,
            mad: self.width.mad / self.width.median.max(f64::EPSILON),
        }
    }

    /// The floored z-scores of `f` (architecture 4.5 "Scale floor").
    #[must_use]
    pub fn scores(&self, f: &Features) -> Scores {
        Scores {
            gap: self.gap.z(f.gap, FLOOR_GAP),
            width: self
                .width_rel()
                .z(f.width / self.width.median.max(f64::EPSILON), FLOOR_WIDTH),
            tilt: self.tilt.z(f.tilt, FLOOR_TILT),
            roll: self.roll.z(f.roll, FLOOR_ROLL),
        }
    }

    /// The head-to-shoulder ratio's band.
    fn span_band(&self, th: &Thresholds) -> (f64, f64) {
        let half = (3.0 * MAD_SCALE * self.span.mad).max(th.span_band_min * self.span.median);
        (self.span.median - half, self.span.median + half)
    }

    /// Whether the shoulders sit where and as large as at calibration, and the head pair's
    /// ratio is in its band (the camera has not moved).
    fn geometry_matches(&self, f: &Features, th: &Thresholds) -> bool {
        let shift = (f.mid_x - self.mid_x).hypot(f.mid_y - self.mid_y);
        let scale = (f.width / self.width.median.max(f64::EPSILON) - 1.0).abs();
        let (lo, hi) = self.span_band(th);
        shift < th.match_shift
            && scale < th.match_scale
            && (f.head != self.head || (lo..=hi).contains(&f.span))
    }

    /// Check the numbers (they come from the store).
    ///
    /// # Errors
    /// [`Error::Invalid`] if a number is not usable.
    pub fn validate(&self) -> Result<()> {
        let stats = [
            self.gap, self.width, self.tilt, self.roll, self.span, self.turn,
        ];
        let ok = stats
            .iter()
            .all(|s| s.median.is_finite() && s.mad.is_finite() && s.mad >= 0.0)
            && self.width.median > 0.0
            && self.span.median > 0.0
            && self.mid_x.is_finite()
            && self.mid_y.is_finite();
        if ok {
            Ok(())
        } else {
            Err(Error::invalid("stored calibration numbers are not usable"))
        }
    }
}

/// Why a calibration is not usable (architecture 4.5; the words are the page's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationIssue {
    /// Fewer than [`CAL_FRAMES`] valid frames within [`CAL_WITHIN`].
    TooFewFrames,
    /// Less than [`CAL_MIN_VALID_SHARE`] of the frames were valid.
    OutOfView,
    /// The shoulders took less than [`CAL_WIDTH_MIN`] of the frame width.
    TooFar,
    /// The shoulders took more than [`CAL_WIDTH_MAX`] of the frame width.
    TooClose,
    /// The mean visibility was below [`CAL_MIN_VISIBILITY`].
    PoorlyVisible,
    /// A feature varied more than [`CAL_MAD_MAX_FLOORS`] times its floor.
    Moved,
    /// The camera stopped (off, a hold or a conflict) before it finished.
    Interrupted,
}

impl CalibrationIssue {
    /// The reason in words, after "Calibration not usable: ".
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::TooFewFrames => "not enough clear frames in 30 seconds",
            Self::OutOfView => "you were out of view or not clearly visible for much of it",
            Self::TooFar => "you are too far from the camera (both shoulders look small)",
            Self::TooClose => "you are too close to the camera",
            Self::PoorlyVisible => "the light is too poor or you are partly hidden",
            Self::Moved => "you moved during it; sit still as usual and try again",
            Self::Interrupted => "the camera stopped before it finished",
        }
    }
}

/// What the page and settings show about calibration and the baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum PostureState {
    /// No calibration: no cues.
    NotCalibrated,
    /// Collecting frames.
    Calibrating {
        /// Valid frames so far.
        valid: u32,
        /// Valid frames needed.
        needed: u32,
    },
    /// The last calibration failed a check: no cues.
    NotUsable(CalibrationIssue),
    /// Calibrated, and the baseline holds: cues may be given.
    Ready,
    /// Cues are suspended while the baseline is checked.
    Suspect(PostureReason),
    /// The baseline no longer holds: recalibrate to resume cues.
    Recalibrate,
}

impl PostureState {
    /// The short words for the page and settings.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::NotCalibrated => "Not calibrated: no posture cues yet",
            Self::Calibrating { .. } => "Calibrating: sit as you normally do",
            Self::NotUsable(_) => "Calibration not usable",
            Self::Ready => "Calibrated",
            Self::Suspect(PostureReason::AppRestart | PostureReason::CameraRestart) => {
                "Checking the calibration: cues paused"
            }
            Self::Suspect(_) => "Camera position changed? Cues paused",
            Self::Recalibrate => "Recalibrate to resume posture cues",
        }
    }
}

/// One posture observation to record (the `posture.observation` payload's fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Observation {
    /// What was observed.
    pub kind: PostureKind,
    /// Length, seconds (at least 1).
    pub duration_s: u32,
    /// Why (no-reading and suspect intervals).
    pub reason: Option<PostureReason>,
    /// What became of the cue (drift episodes).
    pub cue: Option<CueOutcome>,
    /// How long before it was handed over it ended (an episode ends when the calm began,
    /// [`Thresholds::end_after_s`] earlier). The record's time is its end, so its start is
    /// that time less `duration_s` (slice V2-3). Not stored.
    #[serde(skip)]
    pub ago: Duration,
}

impl Observation {
    /// The event payload.
    #[must_use]
    pub const fn payload(self) -> crate::schema::EventPayload {
        crate::schema::EventPayload::PostureObservation {
            kind: self.kind,
            duration_s: self.duration_s,
            reason: self.reason,
            cue: self.cue,
        }
    }
}

/// Something the detector needs done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PostureAction {
    /// An episode started: ask the coaching engine to show the cue, then report what
    /// became of it ([`Detector::cue_outcome`]).
    Cue,
    /// Write this observation.
    Record(Observation),
    /// The calibration changed (taken, or dropped): save it.
    Persist,
    /// The baseline became invalid: ask the user once to recalibrate.
    AskRecalibrate,
    /// The state the page shows changed.
    Render,
}

/// Counts for the V2-3 quality report and the self-test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct PostureStats {
    /// Calibrations started.
    pub calibrations: u32,
    /// Drift episodes started.
    pub episodes: u32,
    /// Cues the engine showed.
    pub cues_shown: u32,
    /// Cues it did not show.
    pub cues_suppressed: u32,
    /// Times the baseline became suspect.
    pub suspects: u32,
    /// Times it became invalid.
    pub invalidations: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    t: Duration,
    visibility: f64,
    features: Option<Features>,
}

#[derive(Debug, Clone)]
struct Calibrating {
    since: Duration,
    frames: Vec<Sample>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Check {
    /// Matches again after [`MATCH_FOR`] of continuous match; invalid after
    /// [`SUSPECT_FOR`] of reading. `matching` is the current run of matching frames: when
    /// it began and its latest matching frame.
    Revert {
        matching: Option<(Duration, Duration)>,
    },
    /// The first [`RESTART_CHECK`] of reading decides.
    Restart { matched: u32, seen: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Baseline {
    /// No calibration (or the last one failed or was dropped).
    None,
    Valid,
    Suspect {
        reason: PostureReason,
        /// When this suspect interval began (for its record), if capture is running.
        since: Option<Duration>,
        /// Reading time while suspect.
        reading: Duration,
        check: Check,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Episode {
    since: Duration,
    cue: Option<CueOutcome>,
    calm_since: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Step {
    at: Duration,
    mid: (f64, f64),
    width: f64,
}

/// The posture detector. Times are offsets from any fixed origin, as in
/// [`crate::camera::Machine`], which owns it.
#[derive(Debug, Clone)]
pub struct Detector {
    calibration: Option<Calibration>,
    baseline: Baseline,
    calibrating: Option<Calibrating>,
    failed: Option<CalibrationIssue>,
    /// Asked to recalibrate (once) since the baseline became invalid.
    invalid: bool,
    history: VecDeque<Sample>,
    last_valid: Option<Duration>,
    drift_since: Option<Duration>,
    episode: Option<Episode>,
    no_reading: Option<(Duration, PostureReason)>,
    step: Option<Step>,
    view_out_since: Option<Duration>,
    capturing: bool,
    started_once: bool,
    stats: PostureStats,
    th: Thresholds,
    /// The open camera-on segment.
    capture_since: Option<Duration>,
    /// The open interval with no usable calibration (none, calibrating or invalid).
    unscored_since: Option<Duration>,
    /// The latest time given.
    last_now: Duration,
}

fn secs(d: Duration) -> u32 {
    u32::try_from(d.as_secs()).unwrap_or(u32::MAX)
}

impl Detector {
    /// A detector with the stored calibration (it comes back suspect: T-C6).
    #[must_use]
    pub fn new(calibration: Option<Calibration>) -> Self {
        Self::with_thresholds(calibration, Thresholds::PROVISIONAL)
    }

    /// [`Detector::new`] with other thresholds (the V2-3 harness's sweep). Values that
    /// break [`Thresholds::validate`] are replaced by the provisional ones.
    #[must_use]
    pub fn with_thresholds(calibration: Option<Calibration>, th: Thresholds) -> Self {
        let th = if th.validate().is_ok() {
            th
        } else {
            Thresholds::PROVISIONAL
        };
        let calibration = calibration.filter(|c| c.validate().is_ok());
        let baseline = if calibration.is_some() {
            Baseline::Suspect {
                reason: PostureReason::AppRestart,
                since: None,
                reading: Duration::ZERO,
                check: Check::Restart {
                    matched: 0,
                    seen: 0,
                },
            }
        } else {
            Baseline::None
        };
        Self {
            calibration,
            baseline,
            calibrating: None,
            failed: None,
            invalid: false,
            history: VecDeque::new(),
            last_valid: None,
            drift_since: None,
            episode: None,
            no_reading: None,
            step: None,
            view_out_since: None,
            capturing: false,
            started_once: false,
            stats: PostureStats::default(),
            th,
            capture_since: None,
            unscored_since: None,
            last_now: Duration::ZERO,
        }
    }

    /// The thresholds in use.
    #[must_use]
    pub const fn thresholds(&self) -> Thresholds {
        self.th
    }

    /// The calibration to store.
    #[must_use]
    pub const fn calibration(&self) -> Option<Calibration> {
        self.calibration
    }

    /// Whether a calibration is collecting frames (the page runs at the full rate).
    #[must_use]
    pub const fn calibrating(&self) -> bool {
        self.calibrating.is_some()
    }

    /// Counts so far.
    #[must_use]
    pub const fn stats(&self) -> PostureStats {
        self.stats
    }

    /// What the page shows.
    #[must_use]
    pub fn state(&self) -> PostureState {
        if let Some(c) = &self.calibrating {
            let valid = u32::try_from(c.frames.iter().filter(|s| s.features.is_some()).count())
                .unwrap_or(u32::MAX);
            return PostureState::Calibrating {
                valid,
                needed: CAL_FRAMES,
            };
        }
        match self.baseline {
            Baseline::Valid => PostureState::Ready,
            Baseline::Suspect { reason, .. } => PostureState::Suspect(reason),
            Baseline::None if self.invalid => PostureState::Recalibrate,
            Baseline::None => self
                .failed
                .map_or(PostureState::NotCalibrated, PostureState::NotUsable),
        }
    }

    /// Whether drift is scored now (calibrated and the baseline holds).
    #[must_use]
    pub const fn scoring(&self) -> bool {
        matches!(self.baseline, Baseline::Valid) && self.calibrating.is_none()
    }

    // ------------------------------------------------------------ capture

    /// The camera stream started (or started again): a stored or current calibration is
    /// suspect until it matches (architecture 4.5 (a) and "Restart").
    pub fn capture_started(&mut self, now: Duration) -> Vec<PostureAction> {
        let mut out = Vec::new();
        self.last_now = self.last_now.max(now);
        self.capturing = true;
        self.capture_since = Some(now);
        let first = !std::mem::replace(&mut self.started_once, true);
        self.history.clear();
        self.last_valid = None;
        if self.calibration.is_none() {
            self.track_unscored(now, &mut out);
            return out;
        }
        match self.baseline {
            Baseline::Suspect {
                reason: PostureReason::AppRestart,
                ..
            } if first => {
                self.baseline = Baseline::Suspect {
                    reason: PostureReason::AppRestart,
                    since: Some(now),
                    reading: Duration::ZERO,
                    check: Check::Restart {
                        matched: 0,
                        seen: 0,
                    },
                };
                self.stats.suspects += 1;
            }
            Baseline::Valid | Baseline::Suspect { .. } => {
                self.suspect(now, PostureReason::CameraRestart, &mut out);
            }
            Baseline::None => {}
        }
        self.track_unscored(now, &mut out);
        out.push(PostureAction::Render);
        out
    }

    /// The camera stream ended (off, a hold, a stall or a conflict): open intervals are
    /// recorded, a calibration in progress fails, and the memory is dropped.
    pub fn capture_ended(&mut self, now: Duration) -> Vec<PostureAction> {
        let mut out = Vec::new();
        if !self.capturing {
            return out;
        }
        self.last_now = self.last_now.max(now);
        self.capturing = false;
        if self.calibrating.take().is_some() {
            self.failed = Some(CalibrationIssue::Interrupted);
            out.push(PostureAction::Render);
        }
        if let Some(start) = self.capture_since.take() {
            Self::record_capture(&mut out, now.saturating_sub(start));
        }
        self.track_unscored(now, &mut out);
        self.end_episode(now, now, &mut out);
        self.end_no_reading(now, &mut out);
        if let Baseline::Suspect { reason, since, .. } = &mut self.baseline
            && let Some(start) = since.take()
        {
            Self::record_interval(
                &mut out,
                PostureKind::BaselineSuspect,
                now.saturating_sub(start),
                *reason,
            );
        }
        self.history.clear();
        self.last_valid = None;
        self.drift_since = None;
        self.step = None;
        self.view_out_since = None;
        out
    }

    /// The reading changed: `None` while frames give a reading, else why not.
    pub fn reading_changed(
        &mut self,
        now: Duration,
        no_reading: Option<PostureReason>,
    ) -> Vec<PostureAction> {
        let mut out = Vec::new();
        self.last_now = self.last_now.max(now);
        match (self.no_reading, no_reading) {
            (Some((_, was)), Some(reason)) if was == reason => {}
            (_, Some(reason)) => {
                self.end_no_reading(now, &mut out);
                self.no_reading = Some((now, reason));
            }
            (Some(_), None) => self.end_no_reading(now, &mut out),
            (None, None) => {}
        }
        out
    }

    // ------------------------------------------------------------ calibration

    /// Start collecting a calibration (the user chose Calibrate while capturing). Cues
    /// stop until it finishes.
    pub fn start_calibration(&mut self, now: Duration) -> Vec<PostureAction> {
        let mut out = Vec::new();
        self.last_now = self.last_now.max(now);
        self.end_episode(now, now, &mut out);
        self.calibrating = Some(Calibrating {
            since: now,
            frames: Vec::new(),
        });
        self.stats.calibrations += 1;
        self.track_unscored(now, &mut out);
        out.push(PostureAction::Render);
        out
    }

    /// The user cancelled the calibration: the previous state stands.
    pub fn cancel_calibration(&mut self) -> Vec<PostureAction> {
        if self.calibrating.take().is_some() {
            let mut out = Vec::new();
            self.track_unscored(self.last_now, &mut out);
            out.push(PostureAction::Render);
            out
        } else {
            Vec::new()
        }
    }

    fn finish_calibration(
        &mut self,
        now: Duration,
        out: &mut Vec<PostureAction>,
        at: DateTime<Utc>,
    ) {
        let Some(c) = self.calibrating.take() else {
            return;
        };
        if let Baseline::Suspect {
            reason,
            since: Some(start),
            ..
        } = self.baseline
        {
            Self::record_interval(
                out,
                PostureKind::BaselineSuspect,
                now.saturating_sub(start),
                reason,
            );
        }
        match assess(&c.frames, at) {
            Ok(cal) => {
                self.calibration = Some(cal);
                self.baseline = Baseline::Valid;
                self.failed = None;
                self.invalid = false;
            }
            Err(issue) => {
                // A failed calibration gives no cues, whatever came before.
                self.calibration = None;
                self.baseline = Baseline::None;
                self.failed = Some(issue);
                self.invalid = false;
            }
        }
        self.history.clear();
        self.drift_since = None;
        self.step = None;
        self.view_out_since = None;
        out.push(PostureAction::Persist);
        out.push(PostureAction::Render);
    }

    // ------------------------------------------------------------ frames

    /// One frame (valid with features, or not). `wall` is when, for a calibration that
    /// finishes on it.
    pub fn frame(
        &mut self,
        now: Duration,
        wall: DateTime<Utc>,
        visibility: f64,
        features: Option<Features>,
    ) -> Vec<PostureAction> {
        let mut out = Vec::new();
        if !self.capturing {
            return out;
        }
        self.last_now = self.last_now.max(now);
        self.segment(now, &mut out);
        let sample = Sample {
            t: now,
            visibility,
            features,
        };
        if let Some(c) = self.calibrating.as_mut() {
            c.frames.push(sample);
            let valid = c.frames.iter().filter(|s| s.features.is_some()).count();
            let done = valid >= CAL_FRAMES as usize || now.saturating_sub(c.since) >= CAL_WITHIN;
            if done {
                self.finish_calibration(now, &mut out, wall);
                self.track_unscored(now, &mut out);
            } else if features.is_some() {
                out.push(PostureAction::Render);
            }
            return out;
        }
        self.history.push_back(sample);
        while self
            .history
            .front()
            .is_some_and(|s| now.saturating_sub(s.t) > HISTORY_FOR)
        {
            self.history.pop_front();
        }
        let Some(cal) = self.calibration else {
            return out;
        };
        // Reading time: valid frames close enough together.
        let reading = match (features, self.last_valid) {
            (Some(_), Some(last)) if now.saturating_sub(last) <= READING_GAP => {
                now.saturating_sub(last)
            }
            _ => Duration::ZERO,
        };
        if features.is_some() {
            self.last_valid = Some(now);
        }
        match self.baseline {
            Baseline::None => {}
            Baseline::Suspect { .. } => {
                self.check_suspect(now, &cal, features, reading, &mut out);
                self.track_unscored(now, &mut out);
            }
            Baseline::Valid => {
                if self.viewpoint_check(now, &cal, &mut out) {
                    return out;
                }
                self.score(now, &cal, &mut out);
            }
        }
        out
    }

    /// The calibration clock (a calibration with too few frames ends at [`CAL_WITHIN`]).
    pub fn tick(&mut self, now: Duration, wall: DateTime<Utc>) -> Vec<PostureAction> {
        let mut out = Vec::new();
        self.last_now = self.last_now.max(now);
        if let Some(c) = &self.calibrating
            && now.saturating_sub(c.since) >= CAL_WITHIN
        {
            self.finish_calibration(now, &mut out, wall);
            self.track_unscored(now, &mut out);
        }
        if self.capturing {
            self.segment(now, &mut out);
        }
        out
    }

    /// What became of the last episode's cue.
    pub fn cue_outcome(&mut self, outcome: CueOutcome) {
        if outcome == CueOutcome::Shown {
            self.stats.cues_shown += 1;
        } else {
            self.stats.cues_suppressed += 1;
        }
        if let Some(e) = self.episode.as_mut()
            && e.cue.is_none()
        {
            e.cue = Some(outcome);
        }
    }

    // ------------------------------------------------------------ internals

    /// The smoothed features over [`SMOOTH_OVER`] (valid frames with the calibration's
    /// head pair), if any.
    fn smoothed(&self, now: Duration, head: HeadRef) -> Option<Features> {
        let recent: Vec<Features> = self
            .history
            .iter()
            .rev()
            .take_while(|s| now.saturating_sub(s.t) <= SMOOTH_OVER)
            .filter_map(|s| s.features)
            .filter(|f| f.head == head)
            .collect();
        if recent.is_empty() {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = recent.len() as f64;
        let mean = |g: fn(&Features) -> f64| recent.iter().map(g).sum::<f64>() / n;
        Some(Features {
            head,
            gap: mean(|f| f.gap),
            width: mean(|f| f.width),
            tilt: mean(|f| f.tilt),
            roll: mean(|f| f.roll),
            mid_x: mean(|f| f.mid_x),
            mid_y: mean(|f| f.mid_y),
            span: mean(|f| f.span),
            turn: mean(|f| f.turn),
        })
    }

    /// The share of frames since `from` that were valid.
    fn valid_share(&self, from: Duration) -> f64 {
        let (n, valid) = self
            .history
            .iter()
            .filter(|s| s.t >= from)
            .fold((0_u32, 0_u32), |(n, v), s| {
                (n + 1, v + u32::from(s.features.is_some()))
            });
        if n == 0 {
            0.0
        } else {
            f64::from(valid) / f64::from(n)
        }
    }

    fn score(&mut self, now: Duration, cal: &Calibration, out: &mut Vec<PostureAction>) {
        let Some(smooth) = self.smoothed(now, cal.head) else {
            return;
        };
        let drift = cal.scores(&smooth).max_abs();
        let th = self.th;
        if drift >= th.z_drift {
            let since = *self.drift_since.get_or_insert(now);
            if let Some(e) = self.episode.as_mut() {
                e.calm_since = None;
            } else if now.saturating_sub(since) >= EPISODE_AFTER
                && self.valid_share(since) >= EPISODE_MIN_VALID
            {
                self.episode = Some(Episode {
                    since,
                    cue: None,
                    calm_since: None,
                });
                self.stats.episodes += 1;
                out.push(PostureAction::Cue);
            }
        } else if let Some(e) = self.episode.as_mut() {
            if drift < th.z_end {
                let calm = *e.calm_since.get_or_insert(now);
                if now.saturating_sub(calm) >= th.end_after() {
                    self.end_episode(calm, now, out);
                    self.drift_since = None;
                }
            } else {
                e.calm_since = None;
            }
        } else {
            self.drift_since = None;
        }
    }

    /// The viewpoint check while the baseline holds; true if it became suspect.
    fn viewpoint_check(
        &mut self,
        now: Duration,
        cal: &Calibration,
        out: &mut Vec<PostureAction>,
    ) -> bool {
        // (b) A step in shoulder geometry, completed within two seconds, held 20 s.
        let window = |from: Duration, to: Duration| -> Option<((f64, f64), f64)> {
            let pts: Vec<Features> = self
                .history
                .iter()
                .filter(|s| s.t > from && s.t <= to)
                .filter_map(|s| s.features)
                .collect();
            if pts.is_empty() {
                return None;
            }
            let xs: Vec<f64> = pts.iter().map(|f| f.mid_x).collect();
            let ys: Vec<f64> = pts.iter().map(|f| f.mid_y).collect();
            let ws: Vec<f64> = pts.iter().map(|f| f.width).collect();
            Some(((median(&xs), median(&ys)), median(&ws)))
        };
        let second = Duration::from_secs(1);
        let current = window(now.saturating_sub(second), now);
        let jumped = |a: ((f64, f64), f64), b: ((f64, f64), f64), k: f64| {
            let shift = (b.0.0 - a.0.0).hypot(b.0.1 - a.0.1);
            let scale = (b.1 / a.1.max(f64::EPSILON) - 1.0).abs();
            shift >= STEP_SHIFT * k || scale >= STEP_SCALE * k
        };
        if let Some(step) = self.step {
            match current {
                Some(cur) if jumped(((step.mid), step.width), cur, 0.5) => {
                    if now.saturating_sub(step.at) >= STEP_HOLD {
                        self.suspect(now, PostureReason::CameraMoved, out);
                        return true;
                    }
                }
                Some(_) => self.step = None,
                None => {}
            }
        } else if now >= STEP_WITHIN + second
            && let (Some(before), Some(cur)) = (
                window(
                    now.saturating_sub(STEP_WITHIN + second),
                    now.saturating_sub(STEP_WITHIN),
                ),
                current,
            )
            && jumped(before, cur, 1.0)
        {
            self.step = Some(Step {
                at: now,
                mid: before.0,
                width: before.1,
            });
        }
        // (c) The head pair's ratio outside its band while posture stays steady and the
        // head faces as at calibration.
        if let Some(smooth) = self.smoothed(now, cal.head) {
            let (lo, hi) = cal.span_band(&self.th);
            let scores = cal.scores(&smooth);
            let z = self.th.z_drift;
            let steady = scores.gap.abs() < z && scores.tilt.abs() < z && scores.roll.abs() < z;
            let facing = (smooth.turn - cal.turn.median).abs() <= self.th.turn_tolerance;
            if steady && facing && !(lo..=hi).contains(&smooth.span) {
                let since = *self.view_out_since.get_or_insert(now);
                if now.saturating_sub(since) >= VIEWPOINT_HOLD {
                    self.suspect(now, PostureReason::ViewpointChanged, out);
                    return true;
                }
            } else {
                self.view_out_since = None;
            }
        }
        false
    }

    fn suspect(&mut self, now: Duration, reason: PostureReason, out: &mut Vec<PostureAction>) {
        self.end_episode(now, now, out);
        if let Baseline::Suspect {
            reason: was,
            since: Some(start),
            ..
        } = self.baseline
        {
            Self::record_interval(
                out,
                PostureKind::BaselineSuspect,
                now.saturating_sub(start),
                was,
            );
        }
        self.baseline = Baseline::Suspect {
            reason,
            since: Some(now),
            reading: Duration::ZERO,
            check: Check::Revert { matching: None },
        };
        self.stats.suspects += 1;
        self.drift_since = None;
        self.step = None;
        self.view_out_since = None;
        out.push(PostureAction::Render);
    }

    fn check_suspect(
        &mut self,
        now: Duration,
        cal: &Calibration,
        features: Option<Features>,
        reading: Duration,
        out: &mut Vec<PostureAction>,
    ) {
        let Baseline::Suspect {
            reason,
            since,
            reading: total,
            check,
        } = self.baseline
        else {
            return;
        };
        let total = total + reading;
        let matches = features
            .filter(|f| f.head == cal.head)
            .map(|f| cal.geometry_matches(&f, &self.th));
        let (check, verdict) = match check {
            Check::Restart { matched, seen } => {
                let (matched, seen) = match matches {
                    Some(m) => (matched + u32::from(m), seen + 1),
                    None => (matched, seen),
                };
                let verdict = (total >= RESTART_CHECK).then(|| {
                    seen > 0 && f64::from(matched) / f64::from(seen) >= RESTART_MATCH_SHARE
                });
                (Check::Restart { matched, seen }, verdict)
            }
            Check::Revert { matching } => {
                // Only matching frames are evidence: the run's length is measured between
                // its first and latest matching frames, and a stretch without usable
                // features longer than READING_GAP ends it.
                let continues =
                    |(_, last): &(Duration, Duration)| now.saturating_sub(*last) <= READING_GAP;
                let matching = match matches {
                    Some(true) => Some(match matching.filter(continues) {
                        Some((since, _)) => (since, now),
                        None => (now, now),
                    }),
                    Some(false) => None,
                    None => matching.filter(continues),
                };
                let verdict = if matching
                    .is_some_and(|(since, last)| last.saturating_sub(since) >= MATCH_FOR)
                {
                    Some(true)
                } else if total >= SUSPECT_FOR {
                    Some(false)
                } else {
                    None
                };
                (Check::Revert { matching }, verdict)
            }
        };
        match verdict {
            None => {
                self.baseline = Baseline::Suspect {
                    reason,
                    since,
                    reading: total,
                    check,
                };
            }
            Some(accepted) => {
                if let Some(start) = since {
                    Self::record_interval(
                        out,
                        PostureKind::BaselineSuspect,
                        now.saturating_sub(start),
                        reason,
                    );
                }
                if accepted {
                    self.baseline = Baseline::Valid;
                    self.history.retain(|s| s.t >= now);
                } else {
                    // Nothing is scored against the old baseline until recalibration.
                    self.baseline = Baseline::None;
                    self.calibration = None;
                    self.invalid = true;
                    self.stats.invalidations += 1;
                    out.push(PostureAction::Persist);
                    out.push(PostureAction::AskRecalibrate);
                }
                out.push(PostureAction::Render);
            }
        }
    }

    /// End the episode at `end` (seen at `now`).
    fn end_episode(&mut self, end: Duration, now: Duration, out: &mut Vec<PostureAction>) {
        if let Some(e) = self.episode.take() {
            let d = end.saturating_sub(e.since);
            out.push(PostureAction::Record(Observation {
                kind: PostureKind::DriftEpisode,
                duration_s: secs(d).max(1),
                reason: None,
                cue: Some(e.cue.unwrap_or(CueOutcome::NoAnswer)),
                ago: now.saturating_sub(end),
            }));
        }
        self.drift_since = None;
    }

    /// Whether drift cannot be scored for want of a usable calibration (none, failed,
    /// invalid until recalibration, or collecting one) while capture runs.
    const fn unscored(&self) -> bool {
        self.capturing && (self.calibrating.is_some() || matches!(self.baseline, Baseline::None))
    }

    /// Open or close the no-calibration interval as the state stands (slice V2-3).
    fn track_unscored(&mut self, now: Duration, out: &mut Vec<PostureAction>) {
        match (self.unscored(), self.unscored_since) {
            (true, None) => self.unscored_since = Some(now),
            (false, Some(start)) => {
                self.unscored_since = None;
                Self::record_interval(
                    out,
                    PostureKind::BaselineSuspect,
                    now.saturating_sub(start),
                    PostureReason::NoCalibration,
                );
            }
            _ => {}
        }
    }

    /// Write the open intervals once the camera-on segment reaches [`SEGMENT_EVERY`], and
    /// start them again (an episode stays one event).
    fn segment(&mut self, now: Duration, out: &mut Vec<PostureAction>) {
        let Some(start) = self.capture_since else {
            return;
        };
        if now.saturating_sub(start) < SEGMENT_EVERY {
            return;
        }
        Self::record_capture(out, now.saturating_sub(start));
        self.capture_since = Some(now);
        if let Some(since) = self.unscored_since {
            Self::record_interval(
                out,
                PostureKind::BaselineSuspect,
                now.saturating_sub(since),
                PostureReason::NoCalibration,
            );
            self.unscored_since = Some(now);
        }
        if let Some((since, reason)) = self.no_reading
            && now.saturating_sub(since) >= NO_READING_MIN
        {
            Self::record_interval(
                out,
                PostureKind::NoReading,
                now.saturating_sub(since),
                reason,
            );
            self.no_reading = Some((now, reason));
        }
        if let Baseline::Suspect {
            reason,
            since: Some(since),
            ..
        } = &mut self.baseline
        {
            let d = now.saturating_sub(*since);
            if d >= Duration::from_secs(1) {
                Self::record_interval(out, PostureKind::BaselineSuspect, d, *reason);
                *since = now;
            }
        }
    }

    fn record_capture(out: &mut Vec<PostureAction>, d: Duration) {
        if d >= Duration::from_secs(1) {
            out.push(PostureAction::Record(Observation {
                kind: PostureKind::Capture,
                duration_s: secs(d),
                reason: None,
                cue: None,
                ago: Duration::ZERO,
            }));
        }
    }

    fn end_no_reading(&mut self, now: Duration, out: &mut Vec<PostureAction>) {
        if let Some((since, reason)) = self.no_reading.take() {
            let d = now.saturating_sub(since);
            if d >= NO_READING_MIN {
                Self::record_interval(out, PostureKind::NoReading, d, reason);
            }
        }
    }

    fn record_interval(
        out: &mut Vec<PostureAction>,
        kind: PostureKind,
        d: Duration,
        reason: PostureReason,
    ) {
        if d >= Duration::from_secs(1) {
            out.push(PostureAction::Record(Observation {
                kind,
                duration_s: secs(d),
                reason: Some(reason),
                cue: None,
                ago: Duration::ZERO,
            }));
        }
    }
}

/// What became of a cue, from the coaching engine's answer to
/// [`crate::coach::Input::PostureCue`].
#[must_use]
pub fn cue_outcome_of(outcome: &crate::coach::InputOutcome) -> CueOutcome {
    use crate::coach::InputOutcome;
    use crate::coach::engine::{CUE_BUSY, CUE_COOLDOWN, CUE_FOCUS, CUE_HELD};
    match outcome {
        InputOutcome::Applied => CueOutcome::Shown,
        InputOutcome::Ignored(why) if *why == CUE_FOCUS => CueOutcome::Focus,
        InputOutcome::Ignored(why) if *why == CUE_HELD => CueOutcome::Held,
        InputOutcome::Ignored(why) if *why == CUE_COOLDOWN => CueOutcome::Cooldown,
        InputOutcome::Ignored(why) if *why == CUE_BUSY => CueOutcome::Busy,
        _ => CueOutcome::NoAnswer,
    }
}

/// Judge a calibration's frames (architecture 4.5 "Calibration quality", all required):
/// the calibration, or the first check that fails.
fn assess(
    frames: &[Sample],
    at: DateTime<Utc>,
) -> std::result::Result<Calibration, CalibrationIssue> {
    let ears = frames
        .iter()
        .filter(|s| s.features.is_some_and(|f| f.head == HeadRef::Ears))
        .count();
    let eyes = frames
        .iter()
        .filter(|s| s.features.is_some_and(|f| f.head == HeadRef::Eyes))
        .count();
    let head = if ears >= eyes {
        HeadRef::Ears
    } else {
        HeadRef::Eyes
    };
    let used: Vec<(f64, Features)> = frames
        .iter()
        .filter_map(|s| {
            s.features
                .filter(|f| f.head == head)
                .map(|f| (s.visibility, f))
        })
        .collect();
    #[allow(clippy::cast_precision_loss)]
    let share = if frames.is_empty() {
        0.0
    } else {
        used.len() as f64 / frames.len() as f64
    };
    if used.len() < CAL_FRAMES as usize {
        return Err(if share < CAL_MIN_VALID_SHARE {
            CalibrationIssue::OutOfView
        } else {
            CalibrationIssue::TooFewFrames
        });
    }
    if share < CAL_MIN_VALID_SHARE {
        return Err(CalibrationIssue::OutOfView);
    }
    let col = |g: fn(&Features) -> f64| used.iter().map(|(_, f)| g(f)).collect::<Vec<f64>>();
    let width = Stat::of(&col(|f| f.width));
    if width.median < CAL_WIDTH_MIN {
        return Err(CalibrationIssue::TooFar);
    }
    if width.median > CAL_WIDTH_MAX {
        return Err(CalibrationIssue::TooClose);
    }
    #[allow(clippy::cast_precision_loss)]
    let visibility = used.iter().map(|(v, _)| v).sum::<f64>() / used.len() as f64;
    if visibility < CAL_MIN_VISIBILITY {
        return Err(CalibrationIssue::PoorlyVisible);
    }
    let cal = Calibration {
        at,
        head,
        gap: Stat::of(&col(|f| f.gap)),
        width,
        tilt: Stat::of(&col(|f| f.tilt)),
        roll: Stat::of(&col(|f| f.roll)),
        mid_x: median(&col(|f| f.mid_x)),
        mid_y: median(&col(|f| f.mid_y)),
        span: Stat::of(&col(|f| f.span)),
        turn: Stat::of(&col(|f| f.turn)),
    };
    let still = cal.gap.mad <= CAL_MAD_MAX_FLOORS * FLOOR_GAP
        && cal.width_rel().mad <= CAL_MAD_MAX_FLOORS * FLOOR_WIDTH
        && cal.tilt.mad <= CAL_MAD_MAX_FLOORS * FLOOR_TILT
        && cal.roll.mad <= CAL_MAD_MAX_FLOORS * FLOOR_ROLL;
    if !still {
        return Err(CalibrationIssue::Moved);
    }
    Ok(cal)
}

#[cfg(test)]
mod tests;
