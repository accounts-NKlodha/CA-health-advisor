//! The posture check's quality report (slice V2-3; architecture 4.5 "Quality report" and
//! "Harness", PRD V2 gate) and a day's coverage, both from the `posture.observation`
//! records alone.
//!
//! A desk session has two halves that never meet a picture:
//!
//! - **Labels** ([`Label`]): what the operator staged and when (comfortable reading and
//!   typing, a second display, a reach, a sustained forward or sideways drift, absence,
//!   occlusion, a moved camera). They describe movement relative to calibration, never
//!   posture quality. `scripts/windows/Test-HealthierYou-Camera.ps1 -Phase Desk` writes
//!   them as offsets in seconds; the synthetic harness ([`super::desk`]) writes them from
//!   its script.
//! - **The detector's log** ([`Logged`]): the records the app wrote meanwhile (camera-on
//!   segments, no-reading intervals by reason, suspect or no-calibration intervals by
//!   reason, and drift episodes with what became of the cue), read back from the local
//!   store ([`read_log`]) or taken from the machine in the harness.
//!
//! [`session_report`] matches the two and counts, as raw numbers (PRD: "raw counts, not a
//! single unsupported accuracy percentage"): planned check intervals and how many had a
//! usable reading; no-reading and suspect time by reason; staged drifts detected in time,
//! late or missed (and why); detector episodes true, duplicate or false (where they began);
//! cues shown and suppressed by reason; threshold-to-cue latency; absence and occlusion
//! shown as no reading; and moved cameras suspended in time. [`gate`] adds a setup's two
//! sessions and checks the PRD V2 gate per setup.
//!
//! A drift episode's record holds its length and is written when it ends, so its start is
//! the record's time less its length. It also holds when the cue was actually asked
//! (`cue_after_s`): at least [`EPISODE_AFTER`] after the drift began, but later when frames
//! without a reading held back the 70%-valid rule. Matching, latency and where a cue fell
//! use that time; only records from before slice V2-3 fall back to the start plus 60 s.
//! Short no-reading spells reach the log added up ([`super::BRIEF_EVERY`]), so coverage
//! counts every moment without a reading.
//!
//! Nothing here reads, keeps or writes a frame, a landmark or a feature: only the records'
//! kinds, reasons, outcomes and times. The report holds no name, path or device id.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use super::{
    CueOutcome, EPISODE_AFTER, Observation, PostureKind, PostureReason, STEP_HOLD, Thresholds,
};
use crate::error::{Error, Result};
use crate::schema::EventPayload;

/// The planned time is cut into check intervals this long (D; PRD "planned check
/// intervals").
pub const CHECK_INTERVAL_MS: i64 = 60_000;
/// A check interval counts as having a usable reading when at least this share of it had
/// one with a scored baseline (D).
pub const CHECK_USABLE_SHARE: f64 = 0.5;
/// When the cue is asked after the drift began (architecture 4.5: 60 s).
pub const EPISODE_AFTER_MS: i64 = duration_ms(EPISODE_AFTER);
/// A staged drift counts as detected when the cue was asked within this of its sustained
/// threshold (PRD V2 gate: 30 s).
pub const CUE_WITHIN_MS: i64 = 30_000;
/// An episode belongs to a staged drift when its cue was asked during the drift or up to
/// this long after it (D: a cue more than [`CUE_WITHIN_MS`] after the threshold counts as
/// late, not missed) ...
pub const MATCH_GRACE_MS: i64 = 30_000;
/// ... and it began no earlier than this before the operator's mark (D: the mark can
/// trail the movement by a few seconds). An episode already running before the drift
/// began is not a detection of it: it is a false episode, and the drift is missed.
pub const ONSET_BEFORE_MS: i64 = 10_000;
/// A moved camera counts as suspended in time when cues paused within this of the
/// operator's mark (D: architecture 4.5 holds a step for 20 s, after a step of up to 2 s
/// and the 1 s window that sees it; the T-C4 "within 20 s" is read from the step's end).
pub const SUSPEND_WITHIN_MS: i64 = duration_ms(STEP_HOLD) + 5_000;
/// A label counts as shown as no reading when at least this share of it had none.
pub const LABEL_COVERED_SHARE: f64 = 0.5;
/// PRD V2 gate: a usable reading in at least this share of planned check intervals.
pub const COVERAGE_MIN: f64 = 0.70;
/// PRD V2 gate: at least this share of staged drifts detected in time.
pub const DETECTION_MIN: f64 = 0.80;
/// PRD V2 gate: at least this many staged drifts per setup.
pub const STAGED_MIN: u32 = 10;
/// PRD V2 gate: at most one incorrect cue per this many minutes of comfortable work.
pub const FALSE_CUE_MINUTES: f64 = 30.0;
/// PRD V2 gate: this many setups ...
pub const SETUPS: u32 = 5;
/// ... of this many sessions each.
pub const SESSIONS: u32 = 2;

const fn duration_ms(d: std::time::Duration) -> i64 {
    // The constants used here are seconds long; no overflow.
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    let ms = d.as_millis() as i64;
    ms
}

/// What the operator staged (PRD V2 gate's list). Labels describe movement relative to
/// calibration, not posture quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Calibrating (not planned check time).
    Calibrate,
    /// Comfortable reading.
    Reading,
    /// Comfortable typing.
    Typing,
    /// Looking at a second display now and then.
    SecondDisplay,
    /// A short reach or stretch (comfortable work).
    Reach,
    /// A deliberate sustained forward drift (slump or lean towards the screen).
    DriftForward,
    /// A deliberate sustained sideways drift.
    DriftLateral,
    /// Out of view.
    Absent,
    /// Partly hidden (a hand or an object in front).
    Occluded,
    /// The camera moved (laptop pushed back, screen tilted), sitting as usual.
    Relocated,
    /// Capture released by a hold (lock, meeting): not planned check time.
    Paused,
}

impl LabelKind {
    /// Every kind.
    pub const ALL: [Self; 11] = [
        Self::Calibrate,
        Self::Reading,
        Self::Typing,
        Self::SecondDisplay,
        Self::Reach,
        Self::DriftForward,
        Self::DriftLateral,
        Self::Absent,
        Self::Occluded,
        Self::Relocated,
        Self::Paused,
    ];

    /// The stored text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Calibrate => "calibrate",
            Self::Reading => "reading",
            Self::Typing => "typing",
            Self::SecondDisplay => "second_display",
            Self::Reach => "reach",
            Self::DriftForward => "drift_forward",
            Self::DriftLateral => "drift_lateral",
            Self::Absent => "absent",
            Self::Occluded => "occluded",
            Self::Relocated => "relocated",
            Self::Paused => "paused",
        }
    }

    /// Comfortable desk work (the false-cue denominator).
    #[must_use]
    pub const fn comfortable(self) -> bool {
        matches!(
            self,
            Self::Reading | Self::Typing | Self::SecondDisplay | Self::Reach
        )
    }

    /// A staged drift.
    #[must_use]
    pub const fn drift(self) -> bool {
        matches!(self, Self::DriftForward | Self::DriftLateral)
    }

    /// Planned check time: someone at the desk in view of an unmoved camera.
    #[must_use]
    pub const fn planned(self) -> bool {
        self.comfortable() || self.drift()
    }
}

impl std::str::FromStr for LabelKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| Error::invalid(format!("unknown label {s:?}")))
    }
}

/// One labelled stretch, in milliseconds from the session's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    /// What was staged.
    pub kind: LabelKind,
    /// Start.
    pub start_ms: i64,
    /// End (after the start).
    pub end_ms: i64,
}

impl Label {
    const fn span(&self) -> Span {
        (self.start_ms, self.end_ms)
    }
}

/// Read the operator's labels: a header `start_s,end_s,label`, then one stretch a line
/// (seconds from the session's start, decimals allowed).
///
/// # Errors
/// [`Error::Invalid`] naming the first bad line.
pub fn parse_labels(text: &str) -> Result<Vec<Label>> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || (i == 0 && line.starts_with("start_s")) {
            continue;
        }
        let bad = || Error::invalid(format!("label line {} is not start_s,end_s,label", i + 1));
        let mut cells = line.split(',').map(str::trim);
        let (Some(a), Some(b), Some(k), None) =
            (cells.next(), cells.next(), cells.next(), cells.next())
        else {
            return Err(bad());
        };
        let secs = |v: &str| -> Option<i64> {
            let x: f64 = v.parse().ok()?;
            // At most a day; whole milliseconds.
            #[allow(clippy::cast_possible_truncation)]
            (x.is_finite() && (0.0..=86_400.0).contains(&x)).then(|| (x * 1000.0).round() as i64)
        };
        let (Some(start_ms), Some(end_ms)) = (secs(a), secs(b)) else {
            return Err(bad());
        };
        if end_ms <= start_ms {
            return Err(bad());
        }
        out.push(Label {
            kind: k.parse()?,
            start_ms,
            end_ms,
        });
    }
    out.sort_by_key(|l| l.start_ms);
    Ok(out)
}

/// One record of the detector, as an interval in milliseconds (from the session's start
/// in a report; from any origin otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Logged {
    /// What was recorded.
    pub kind: PostureKind,
    /// When it began (the record's time less its length).
    pub start_ms: i64,
    /// When it ended (the record's time).
    pub end_ms: i64,
    /// Why (no-reading and suspect intervals).
    pub reason: Option<PostureReason>,
    /// What became of the cue (drift episodes).
    pub cue: Option<CueOutcome>,
    /// When the cue was asked (drift episodes recorded since slice V2-3), same origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cue_ms: Option<i64>,
}

impl Logged {
    /// From a stored record that ended at `end_ms`.
    #[must_use]
    pub fn from_record(
        end_ms: i64,
        kind: PostureKind,
        duration_s: u32,
        reason: Option<PostureReason>,
        cue: Option<CueOutcome>,
        cue_after_s: Option<u32>,
    ) -> Self {
        let start_ms = end_ms - i64::from(duration_s) * 1000;
        Self {
            kind,
            start_ms,
            end_ms,
            reason,
            cue,
            cue_ms: cue_after_s.map(|c| start_ms + i64::from(c) * 1000),
        }
    }

    /// From an observation handed over at `now_ms` (it ended `ago` earlier).
    #[must_use]
    pub fn from_observation(now_ms: i64, o: &Observation) -> Self {
        let ago = i64::try_from(o.ago.as_millis()).unwrap_or(0);
        Self::from_record(
            now_ms - ago,
            o.kind,
            o.duration_s,
            o.reason,
            o.cue,
            o.cue_after_s,
        )
    }

    /// When the cue was asked (drift episodes): the recorded time, or for a record written
    /// before slice V2-3 (which has none) [`EPISODE_AFTER`] after the drift began.
    #[must_use]
    pub fn cue_at_ms(&self) -> i64 {
        self.cue_ms.unwrap_or(self.start_ms + EPISODE_AFTER_MS)
    }

    const fn span(&self) -> Span {
        (self.start_ms, self.end_ms)
    }
}

// ------------------------------------------------------------------ intervals

type Span = (i64, i64);

/// Sorted, merged, non-empty spans.
fn union(mut v: Vec<Span>) -> Vec<Span> {
    v.retain(|(a, b)| b > a);
    v.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The time `span` shares with merged `set`.
fn overlap(span: Span, set: &[Span]) -> i64 {
    set.iter()
        .map(|(a, b)| (span.1.min(*b) - span.0.max(*a)).max(0))
        .sum()
}

/// Merged `a` less merged `b`.
fn subtract(a: &[Span], b: &[Span]) -> Vec<Span> {
    let mut out = Vec::new();
    for &(mut s, e) in a {
        for &(bs, be) in b {
            if be <= s || bs >= e {
                continue;
            }
            if bs > s {
                out.push((s, bs));
            }
            s = s.max(be);
            if s >= e {
                break;
            }
        }
        if s < e {
            out.push((s, e));
        }
    }
    out
}

fn total(set: &[Span]) -> i64 {
    set.iter().map(|(a, b)| b - a).sum()
}

fn spans(logs: &[Logged], keep: impl Fn(&Logged) -> bool) -> Vec<Span> {
    union(logs.iter().filter(|l| keep(l)).map(Logged::span).collect())
}

fn secs_of(ms: i64) -> u64 {
    u64::try_from(ms.max(0) / 1000).unwrap_or(0)
}

// ------------------------------------------------------------------ a day's coverage

/// A day's posture coverage, derived from its records (architecture 4.5 "Stored": daily
/// coverage; V2-2 left it to V2-3). The parts add up: camera on = no reading + not scored
/// + usable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    /// Camera-on time, seconds.
    pub camera_on_s: u64,
    /// Of it, with no reading, seconds.
    pub no_reading_s: u64,
    /// Of the rest, not scored (suspect, or no usable calibration), seconds.
    pub unscored_s: u64,
    /// The rest: a reading against a calibration that holds, seconds.
    pub usable_s: u64,
    /// `usable_s / camera_on_s`; `None` without camera-on time (never a zero).
    pub usable_share: Option<f64>,
    /// Drift episodes.
    pub episodes: u32,
    /// Cues the pet showed.
    pub cues_shown: u32,
}

/// The coverage of a day's records (or any set of them).
#[must_use]
pub fn coverage(logs: &[Logged]) -> Coverage {
    let on = spans(logs, |l| l.kind == PostureKind::Capture);
    let nr = spans(logs, |l| l.kind == PostureKind::NoReading);
    let sus = spans(logs, |l| l.kind == PostureKind::BaselineSuspect);
    let camera_on = total(&on);
    let on_nr = subtract(&on, &subtract(&on, &nr));
    let no_reading = total(&on_nr);
    let rest = subtract(&on, &nr);
    let unscored = total(&rest) - total(&subtract(&rest, &sus));
    let usable = camera_on - no_reading - unscored;
    let episodes: Vec<&Logged> = logs
        .iter()
        .filter(|l| l.kind == PostureKind::DriftEpisode)
        .collect();
    #[allow(clippy::cast_precision_loss)]
    Coverage {
        camera_on_s: secs_of(camera_on),
        no_reading_s: secs_of(no_reading),
        unscored_s: secs_of(unscored),
        usable_s: secs_of(usable),
        usable_share: (camera_on > 0).then(|| usable as f64 / camera_on as f64),
        episodes: u32::try_from(episodes.len()).unwrap_or(u32::MAX),
        cues_shown: u32::try_from(
            episodes
                .iter()
                .filter(|e| e.cue == Some(CueOutcome::Shown))
                .count(),
        )
        .unwrap_or(u32::MAX),
    }
}

/// The coverage of the records' time from `from_ms` to `to_ms` (a local day): every
/// interval is clipped to the window, so each kind is cut at the same boundaries and the
/// parts still add up; episodes and cues count when their cue was asked within it.
#[must_use]
pub fn coverage_within(logs: &[Logged], from_ms: i64, to_ms: i64) -> Coverage {
    let clipped: Vec<Logged> = logs
        .iter()
        .filter_map(|l| {
            if l.kind == PostureKind::DriftEpisode {
                return (from_ms..to_ms).contains(&l.cue_at_ms()).then_some(*l);
            }
            let (start_ms, end_ms) = (l.start_ms.max(from_ms), l.end_ms.min(to_ms));
            (end_ms > start_ms).then_some(Logged {
                start_ms,
                end_ms,
                ..*l
            })
        })
        .collect();
    coverage(&clipped)
}

/// Turn stored rows (`occurred_at` in Unix ms, `type`, `payload`) into records; rows of
/// other types, or that this build cannot read, are skipped.
pub fn logged_from_rows(rows: impl IntoIterator<Item = (i64, String, String)>) -> Vec<Logged> {
    rows.into_iter()
        .filter_map(|(at, kind, payload)| {
            match EventPayload::from_stored(&kind, &payload).ok()? {
                EventPayload::PostureObservation {
                    kind,
                    duration_s,
                    reason,
                    cue,
                    cue_after_s,
                    ..
                } => Some(Logged::from_record(
                    at,
                    kind,
                    duration_s,
                    reason,
                    cue,
                    cue_after_s,
                )),
                _ => None,
            }
        })
        .collect()
}

/// The posture records in the store at `db` that ended from `since_ms` to `until_ms`
/// (Unix ms), as offsets from `since_ms`, clipped to the window. Read-only, like
/// [`crate::soak::read`]: the running app is not disturbed and nothing is created.
///
/// # Errors
/// [`Error::NotFound`] without a database; [`Error::Invalid`] for a reversed window;
/// [`Error::Sqlite`] if it cannot be read.
pub fn read_log(db: &Path, since_ms: i64, until_ms: i64) -> Result<Vec<Logged>> {
    if until_ms <= since_ms {
        return Err(Error::invalid("the session window is reversed"));
    }
    if !db.is_file() {
        return Err(Error::NotFound("the HealthierYou database".into()));
    }
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let rows: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT occurred_at, type, payload FROM events
             WHERE type = 'posture.observation' AND occurred_at > ?1 AND occurred_at <= ?2
             ORDER BY occurred_at",
        )?
        .query_map([since_ms, until_ms], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(logged_from_rows(rows)
        .into_iter()
        .map(|l| Logged {
            start_ms: l.start_ms.max(since_ms) - since_ms,
            end_ms: l.end_ms - since_ms,
            cue_ms: l.cue_ms.map(|c| c - since_ms),
            ..l
        })
        .collect())
}

// ------------------------------------------------------------------ one session

/// Where a report's numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Synthetic traces ([`super::desk`]): they show the harness works and the thresholds
    /// hang together, not that the check works at a real desk.
    Synthetic,
    /// A real desk session with the operator's labels.
    Desk,
}

/// One session's raw counts (architecture 4.5 "Quality report").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionReport {
    /// Synthetic or a real desk.
    pub source: Source,
    /// The setup (1 to [`SETUPS`]).
    pub setup: u32,
    /// The session (1 to [`SESSIONS`]).
    pub session: u32,
    /// The thresholds the detector used.
    pub thresholds: Thresholds,
    /// Labelled time, seconds.
    pub labelled_s: u64,
    /// Camera-on time, seconds.
    pub camera_on_s: u64,
    /// Planned check intervals (whole [`CHECK_INTERVAL_MS`] stretches of planned labels).
    pub check_intervals: u32,
    /// Of them, with a usable reading.
    pub usable_intervals: u32,
    /// No-reading time by reason, seconds (reasons can overlap in time).
    pub no_reading_s: BTreeMap<String, u64>,
    /// Suspect, invalid or no-calibration time by reason, seconds.
    pub unscored_s: BTreeMap<String, u64>,
    /// Comfortable desk work, seconds.
    pub comfortable_s: u64,
    /// Staged sustained drifts.
    pub staged_drifts: u32,
    /// Detected, with the cue asked within [`CUE_WITHIN_MS`] of the sustained threshold.
    pub detected: u32,
    /// Detected, later than that.
    pub detected_late: u32,
    /// Missed while the baseline was suspect or not calibrated for most of it.
    pub missed_unscored: u32,
    /// Missed with no reading for most of it.
    pub missed_no_reading: u32,
    /// Missed otherwise (the drift stayed under the threshold, or too briefly).
    pub missed_other: u32,
    /// Threshold-to-cue latency of each detection, seconds (signed: negative when the cue
    /// came before 60 s after the operator's mark).
    pub latency_s: Vec<i64>,
    /// Detector episodes.
    pub episodes: u32,
    /// Matched to a staged drift.
    pub true_episodes: u32,
    /// A second episode within the same staged drift.
    pub duplicate_episodes: u32,
    /// Matched to none: where each one began (a label, or `unlabelled`).
    pub false_episodes: BTreeMap<String, u32>,
    /// Cues the pet showed.
    pub cues_shown: u32,
    /// Cues not shown, by why (cooldown, focus, held, busy, no answer).
    pub cues_suppressed: BTreeMap<String, u32>,
    /// Cues shown for a false or duplicate episode.
    pub false_cues: u32,
    /// Staged absences ...
    pub absences: u32,
    /// ... shown as no reading.
    pub absences_no_reading: u32,
    /// Staged occlusions ...
    pub occlusions: u32,
    /// ... shown as no reading.
    pub occlusions_no_reading: u32,
    /// Cues asked during absence or occlusion (nothing may be inferred from no reading).
    pub cues_while_unseen: u32,
    /// Staged camera moves ...
    pub relocations: u32,
    /// ... with cues paused within [`SUSPEND_WITHIN_MS`].
    pub relocations_suspended: u32,
    /// The delay from each mark to the pause, seconds (`None`: never paused).
    pub suspend_delay_s: Vec<Option<i64>>,
    /// Cues asked while the camera was moved (none may be scored against the old
    /// baseline).
    pub cues_while_moved: u32,
    /// Camera-moved or viewpoint suspect time during comfortable work, seconds (false
    /// suspensions; A17).
    pub false_suspect_s: u64,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Match a session's labels and the detector's records (both in ms from its start).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn session_report(
    source: Source,
    setup: u32,
    session: u32,
    thresholds: Thresholds,
    labels: &[Label],
    logs: &[Logged],
) -> SessionReport {
    let on = spans(logs, |l| l.kind == PostureKind::Capture);
    let nr = spans(logs, |l| l.kind == PostureKind::NoReading);
    let unscored = spans(logs, |l| l.kind == PostureKind::BaselineSuspect);
    let bad = union(nr.iter().chain(&unscored).copied().collect());
    let usable = subtract(&on, &bad);
    let moved = spans(logs, |l| {
        l.kind == PostureKind::BaselineSuspect
            && matches!(
                l.reason,
                Some(PostureReason::CameraMoved | PostureReason::ViewpointChanged)
            )
    });

    let by_reason = |kind: PostureKind| {
        let mut m: BTreeMap<String, u64> = BTreeMap::new();
        for l in logs.iter().filter(|l| l.kind == kind) {
            if let Some(r) = l.reason {
                *m.entry(r.as_str().to_owned()).or_default() += secs_of(l.end_ms - l.start_ms);
            }
        }
        m
    };
    let no_reading_s = by_reason(PostureKind::NoReading);
    let unscored_s = by_reason(PostureKind::BaselineSuspect);

    // Planned check intervals.
    let (mut check_intervals, mut usable_intervals) = (0_u32, 0_u32);
    for l in labels.iter().filter(|l| l.kind.planned()) {
        let mut a = l.start_ms;
        while a + CHECK_INTERVAL_MS <= l.end_ms {
            check_intervals += 1;
            #[allow(clippy::cast_precision_loss)]
            let share =
                overlap((a, a + CHECK_INTERVAL_MS), &usable) as f64 / CHECK_INTERVAL_MS as f64;
            if share >= CHECK_USABLE_SHARE {
                usable_intervals += 1;
            }
            a += CHECK_INTERVAL_MS;
        }
    }

    // Episodes against staged drifts.
    let drifts: Vec<&Label> = labels.iter().filter(|l| l.kind.drift()).collect();
    let mut matched: Vec<Option<i64>> = vec![None; drifts.len()];
    let mut episodes: Vec<&Logged> = logs
        .iter()
        .filter(|l| l.kind == PostureKind::DriftEpisode)
        .collect();
    episodes.sort_by_key(|e| e.start_ms);
    let (mut true_episodes, mut duplicate_episodes) = (0_u32, 0_u32);
    let mut false_episodes: BTreeMap<String, u32> = BTreeMap::new();
    let (mut cues_shown, mut false_cues) = (0_u32, 0_u32);
    let mut cues_suppressed: BTreeMap<String, u32> = BTreeMap::new();
    let (mut cues_while_unseen, mut cues_while_moved) = (0_u32, 0_u32);
    let label_at = |t: i64| {
        labels
            .iter()
            .find(|l| l.start_ms <= t && t < l.end_ms)
            .map(|l| l.kind)
    };
    for e in &episodes {
        let shown = e.cue == Some(CueOutcome::Shown);
        if shown {
            cues_shown += 1;
        } else {
            let why = e.cue.unwrap_or(CueOutcome::NoAnswer).as_str().to_owned();
            *cues_suppressed.entry(why).or_default() += 1;
        }
        match label_at(e.cue_at_ms()) {
            Some(LabelKind::Absent | LabelKind::Occluded) => cues_while_unseen += 1,
            Some(LabelKind::Relocated) => cues_while_moved += 1,
            _ => {}
        }
        // An episode belongs to a drift when its cue was asked after the drift began and
        // before the grace after it ended, and it began with the drift: an episode
        // already running when a drift began (it started from a comfortable stance) is
        // not a detection of that drift, whenever its cue came.
        let belongs = |d: &Label| {
            (d.start_ms..d.end_ms + MATCH_GRACE_MS).contains(&e.cue_at_ms())
                && e.start_ms >= d.start_ms - ONSET_BEFORE_MS
        };
        // The first unmatched drift it belongs to, else the first (a duplicate).
        let hit = (0..drifts.len())
            .find(|&i| matched[i].is_none() && belongs(drifts[i]))
            .or_else(|| (0..drifts.len()).find(|&i| belongs(drifts[i])));
        match hit {
            Some(i) if matched[i].is_none() => {
                matched[i] = Some(e.cue_at_ms());
                true_episodes += 1;
            }
            Some(_) => {
                duplicate_episodes += 1;
                false_cues += u32::from(shown);
            }
            None => {
                let at = label_at(e.start_ms).map_or("unlabelled", LabelKind::as_str);
                *false_episodes.entry(at.to_owned()).or_default() += 1;
                false_cues += u32::from(shown);
            }
        }
    }
    let (mut detected, mut detected_late) = (0_u32, 0_u32);
    let (mut missed_unscored, mut missed_no_reading, mut missed_other) = (0_u32, 0_u32, 0_u32);
    let mut latency_s = Vec::new();
    for (d, m) in drifts.iter().zip(&matched) {
        let threshold = d.start_ms + EPISODE_AFTER_MS;
        if let Some(cue_at) = m {
            let late = cue_at - threshold;
            latency_s.push(late.div_euclid(1000));
            if late <= CUE_WITHIN_MS {
                detected += 1;
            } else {
                detected_late += 1;
            }
        } else {
            let len = d.end_ms - d.start_ms;
            if 2 * overlap(d.span(), &unscored) >= len {
                missed_unscored += 1;
            } else if 2 * overlap(d.span(), &nr) >= len {
                missed_no_reading += 1;
            } else {
                missed_other += 1;
            }
        }
    }

    // Absence, occlusion and moved cameras.
    let covered = |l: &Label| {
        #[allow(clippy::cast_precision_loss)]
        let share = overlap(l.span(), &nr) as f64 / (l.end_ms - l.start_ms) as f64;
        share >= LABEL_COVERED_SHARE
    };
    let of = |k: LabelKind| labels.iter().filter(move |l| l.kind == k);
    let mut suspend_delay_s = Vec::new();
    let mut relocations_suspended = 0;
    for l in of(LabelKind::Relocated) {
        let delay = moved
            .iter()
            .find(|(a, b)| *b > l.start_ms && *a < l.end_ms)
            .map(|(a, _)| (a - l.start_ms).max(0));
        if delay.is_some_and(|d| d <= SUSPEND_WITHIN_MS) {
            relocations_suspended += 1;
        }
        suspend_delay_s.push(delay.map(|d| d.div_euclid(1000)));
    }
    let comfortable = union(
        labels
            .iter()
            .filter(|l| l.kind.comfortable())
            .map(Label::span)
            .collect(),
    );

    SessionReport {
        source,
        setup,
        session,
        thresholds,
        labelled_s: secs_of(total(&union(labels.iter().map(Label::span).collect()))),
        camera_on_s: secs_of(total(&on)),
        check_intervals,
        usable_intervals,
        no_reading_s,
        unscored_s,
        comfortable_s: secs_of(total(&comfortable)),
        staged_drifts: count(drifts.len()),
        detected,
        detected_late,
        missed_unscored,
        missed_no_reading,
        missed_other,
        latency_s,
        episodes: count(episodes.len()),
        true_episodes,
        duplicate_episodes,
        false_episodes,
        cues_shown,
        cues_suppressed,
        false_cues,
        absences: count(of(LabelKind::Absent).count()),
        absences_no_reading: count(of(LabelKind::Absent).filter(|l| covered(l)).count()),
        occlusions: count(of(LabelKind::Occluded).count()),
        occlusions_no_reading: count(of(LabelKind::Occluded).filter(|l| covered(l)).count()),
        cues_while_unseen,
        relocations: count(of(LabelKind::Relocated).count()),
        relocations_suspended,
        suspend_delay_s,
        cues_while_moved,
        false_suspect_s: secs_of(comfortable.iter().map(|s| overlap(*s, &moved)).sum::<i64>()),
    }
}

impl SessionReport {
    /// Detector false episodes, all places.
    #[must_use]
    pub fn false_episode_count(&self) -> u32 {
        self.false_episodes.values().sum()
    }
}

// ------------------------------------------------------------------ the gate

/// One setup's sessions added up and checked against the PRD V2 gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupGate {
    /// The setup.
    pub setup: u32,
    /// Sessions found.
    pub sessions: u32,
    /// Planned check intervals.
    pub check_intervals: u32,
    /// With a usable reading.
    pub usable_intervals: u32,
    /// Staged drifts.
    pub staged_drifts: u32,
    /// Detected in time.
    pub detected: u32,
    /// Detected late.
    pub detected_late: u32,
    /// Missed.
    pub missed: u32,
    /// Latencies, seconds.
    pub latency_s: Vec<i64>,
    /// Comfortable work, minutes (whole).
    pub comfortable_min: u64,
    /// Cues shown for no staged drift.
    pub false_cues: u32,
    /// Detector false and duplicate episodes (reported apart, so the cooldown cannot hide
    /// a poor detector: PRD).
    pub false_episodes: u32,
    /// Absences and occlusions staged.
    pub unseen: u32,
    /// Of them, shown as no reading.
    pub unseen_no_reading: u32,
    /// Cues during absence, occlusion or a moved camera.
    pub cues_unseen_or_moved: u32,
    /// Moved cameras staged.
    pub relocations: u32,
    /// Of them, suspended in time.
    pub relocations_suspended: u32,
    /// False suspensions, seconds.
    pub false_suspect_s: u64,
    /// At least [`COVERAGE_MIN`] of check intervals usable.
    pub coverage_ok: bool,
    /// At least [`STAGED_MIN`] drifts and [`DETECTION_MIN`] of them detected in time.
    pub detection_ok: bool,
    /// At most one false cue per [`FALSE_CUE_MINUTES`] of comfortable work.
    pub false_cues_ok: bool,
    /// Every absence and occlusion shown as no reading, with no cue.
    pub unseen_ok: bool,
    /// Every moved camera suspended in time, with no cue.
    pub relocation_ok: bool,
    /// [`SESSIONS`] sessions found and every check passed.
    pub pass: bool,
}

/// The whole gate: every setup, and whether the PRD V2 gate passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateReport {
    /// Synthetic, or real desks (mixed sources are refused).
    pub source: Source,
    /// The thresholds (all sessions must share them).
    pub thresholds: Thresholds,
    /// Per setup, in order.
    pub setups: Vec<SetupGate>,
    /// [`SETUPS`] setups found, each passed.
    pub pass: bool,
}

/// Add up sessions per setup and check the gate.
///
/// # Errors
/// [`Error::Invalid`] with no sessions, mixed sources or thresholds, or a session given
/// twice.
pub fn gate(sessions: &[SessionReport]) -> Result<GateReport> {
    let first = sessions
        .first()
        .ok_or_else(|| Error::invalid("no sessions to add up"))?;
    if sessions
        .iter()
        .any(|s| s.source != first.source || s.thresholds != first.thresholds)
    {
        return Err(Error::invalid(
            "sessions with different sources or thresholds cannot be added up",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !sessions.iter().all(|s| seen.insert((s.setup, s.session))) {
        return Err(Error::invalid("a session was given twice"));
    }
    let mut setups: BTreeMap<u32, Vec<&SessionReport>> = BTreeMap::new();
    for s in sessions {
        setups.entry(s.setup).or_default().push(s);
    }
    let setups: Vec<SetupGate> = setups
        .into_iter()
        .map(|(setup, ss)| setup_gate(setup, &ss))
        .collect();
    let pass = count(setups.len()) >= SETUPS && setups.iter().all(|s| s.pass);
    Ok(GateReport {
        source: first.source,
        thresholds: first.thresholds,
        setups,
        pass,
    })
}

fn setup_gate(setup: u32, ss: &[&SessionReport]) -> SetupGate {
    let sum = |f: fn(&SessionReport) -> u32| ss.iter().map(|s| f(s)).sum::<u32>();
    let check_intervals = sum(|s| s.check_intervals);
    let usable_intervals = sum(|s| s.usable_intervals);
    let staged_drifts = sum(|s| s.staged_drifts);
    let detected = sum(|s| s.detected);
    let detected_late = sum(|s| s.detected_late);
    let missed = sum(|s| s.missed_unscored + s.missed_no_reading + s.missed_other);
    let false_cues = sum(|s| s.false_cues);
    let false_episodes = sum(|s| s.false_episode_count() + s.duplicate_episodes);
    let unseen = sum(|s| s.absences + s.occlusions);
    let unseen_no_reading = sum(|s| s.absences_no_reading + s.occlusions_no_reading);
    let cues_unseen_or_moved = sum(|s| s.cues_while_unseen + s.cues_while_moved);
    let relocations = sum(|s| s.relocations);
    let relocations_suspended = sum(|s| s.relocations_suspended);
    let comfortable_s: u64 = ss.iter().map(|s| s.comfortable_s).sum();
    #[allow(clippy::cast_precision_loss)]
    let coverage_ok = check_intervals > 0
        && f64::from(usable_intervals) >= COVERAGE_MIN * f64::from(check_intervals);
    let detection_ok = staged_drifts >= STAGED_MIN
        && f64::from(detected) >= DETECTION_MIN * f64::from(staged_drifts);
    #[allow(clippy::cast_precision_loss)]
    let allowance = (comfortable_s as f64 / 60.0 / FALSE_CUE_MINUTES).max(1.0);
    let false_cues_ok = f64::from(false_cues) <= allowance;
    let unseen_ok = unseen_no_reading == unseen && cues_unseen_or_moved == 0;
    let relocation_ok = relocations_suspended == relocations;
    let sessions = count(ss.len());
    SetupGate {
        setup,
        sessions,
        check_intervals,
        usable_intervals,
        staged_drifts,
        detected,
        detected_late,
        missed,
        latency_s: ss
            .iter()
            .flat_map(|s| s.latency_s.iter().copied())
            .collect(),
        comfortable_min: comfortable_s / 60,
        false_cues,
        false_episodes,
        unseen,
        unseen_no_reading,
        cues_unseen_or_moved,
        relocations,
        relocations_suspended,
        false_suspect_s: ss.iter().map(|s| s.false_suspect_s).sum(),
        coverage_ok,
        detection_ok,
        false_cues_ok,
        unseen_ok,
        relocation_ok,
        pass: sessions >= SESSIONS
            && coverage_ok
            && detection_ok
            && false_cues_ok
            && unseen_ok
            && relocation_ok,
    }
}

fn ok(b: bool) -> &'static str {
    if b { "PASS" } else { "FAIL" }
}

fn spread(v: &[i64]) -> String {
    if v.is_empty() {
        return "none".into();
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    format!(
        "min {} s, median {} s, max {} s",
        s[0],
        s[s.len() / 2],
        s[s.len() - 1]
    )
}

impl GateReport {
    /// The report in words, with the raw counts (shareable: numbers only).
    #[must_use]
    pub fn text(&self) -> String {
        let mut t = String::new();
        let th = &self.thresholds;
        let _ = writeln!(
            t,
            "HealthierYou posture check: quality report (PRD V2 gate, architecture 4.5)"
        );
        match self.source {
            Source::Synthetic => {
                let _ = writeln!(
                    t,
                    "SOURCE: SYNTHETIC TRACES. This shows the harness and that the thresholds hang\n\
                     together; it is not a desk test. The thresholds stay PROVISIONAL until real\n\
                     desk sessions pass and the owner approves them."
                );
            }
            Source::Desk => {
                let _ = writeln!(t, "SOURCE: real desk sessions with the operator's labels.");
            }
        }
        let _ = writeln!(
            t,
            "Thresholds: z_drift {}, z_end {}, end after {} s, span band {}, turn tolerance {}, match shift {}, match scale {}",
            th.z_drift,
            th.z_end,
            th.end_after_s,
            th.span_band_min,
            th.turn_tolerance,
            th.match_shift,
            th.match_scale
        );
        for s in &self.setups {
            let _ = writeln!(
                t,
                "\nSetup {} ({} session(s)): {}",
                s.setup,
                s.sessions,
                ok(s.pass)
            );
            let _ = writeln!(
                t,
                "  [{}] coverage: {} of {} planned check intervals had a usable reading (at least 70%)",
                ok(s.coverage_ok),
                s.usable_intervals,
                s.check_intervals
            );
            let _ = writeln!(
                t,
                "  [{}] detection: {} of {} staged drifts detected with the cue within 30 s ({} late, {} missed; at least 10 staged and 80%)",
                ok(s.detection_ok),
                s.detected,
                s.staged_drifts,
                s.detected_late,
                s.missed
            );
            let _ = writeln!(
                t,
                "       threshold-to-cue latency: {}",
                spread(&s.latency_s)
            );
            let _ = writeln!(
                t,
                "  [{}] false cues: {} in {} min of comfortable work (at most 1 per 30 min); detector false or duplicate episodes: {}",
                ok(s.false_cues_ok),
                s.false_cues,
                s.comfortable_min,
                s.false_episodes
            );
            let _ = writeln!(
                t,
                "  [{}] no reading: {} of {} absences and occlusions shown as no reading; cues while unseen or moved: {}",
                ok(s.unseen_ok),
                s.unseen_no_reading,
                s.unseen,
                s.cues_unseen_or_moved
            );
            let _ = writeln!(
                t,
                "  [{}] moved camera: {} of {} suspended within {} s; false suspension during comfortable work: {} s",
                ok(s.relocation_ok),
                s.relocations_suspended,
                s.relocations,
                SUSPEND_WITHIN_MS / 1000,
                s.false_suspect_s
            );
        }
        let found = count(self.setups.len());
        let _ = writeln!(
            t,
            "\nOverall: {}{}",
            ok(self.pass),
            if found < SETUPS {
                format!(" (only {found} of {SETUPS} setups)")
            } else {
                String::new()
            }
        );
        t
    }
}

impl SessionReport {
    /// The session's counts in words (shareable: numbers only).
    #[must_use]
    pub fn text(&self) -> String {
        let mut t = String::new();
        let map = |m: &BTreeMap<String, u64>| {
            if m.is_empty() {
                "none".to_owned()
            } else {
                m.iter()
                    .map(|(k, v)| format!("{k} {v} s"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let map32 = |m: &BTreeMap<String, u32>| {
            if m.is_empty() {
                "none".to_owned()
            } else {
                m.iter()
                    .map(|(k, v)| format!("{k} {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        let _ = writeln!(
            t,
            "Setup {} session {} ({}): labelled {} s, camera on {} s",
            self.setup,
            self.session,
            match self.source {
                Source::Synthetic => "SYNTHETIC",
                Source::Desk => "desk",
            },
            self.labelled_s,
            self.camera_on_s
        );
        let _ = writeln!(
            t,
            "  check intervals: {} planned, {} with a usable reading",
            self.check_intervals, self.usable_intervals
        );
        let _ = writeln!(t, "  no reading: {}", map(&self.no_reading_s));
        let _ = writeln!(t, "  suspect or not calibrated: {}", map(&self.unscored_s));
        let _ = writeln!(
            t,
            "  staged drifts {}: detected {} in time, {} late; missed {} (suspect), {} (no reading), {} (other); latency {}",
            self.staged_drifts,
            self.detected,
            self.detected_late,
            self.missed_unscored,
            self.missed_no_reading,
            self.missed_other,
            spread(&self.latency_s)
        );
        let _ = writeln!(
            t,
            "  detector episodes {}: true {}, duplicate {}, false {} ({})",
            self.episodes,
            self.true_episodes,
            self.duplicate_episodes,
            self.false_episode_count(),
            map32(&self.false_episodes)
        );
        let _ = writeln!(
            t,
            "  cues: shown {}, suppressed {} ({}), false cues {}; comfortable work {} s",
            self.cues_shown,
            self.cues_suppressed.values().sum::<u32>(),
            map32(&self.cues_suppressed),
            self.false_cues,
            self.comfortable_s
        );
        let _ = writeln!(
            t,
            "  absences {} ({} no reading), occlusions {} ({} no reading), cues while unseen {}",
            self.absences,
            self.absences_no_reading,
            self.occlusions,
            self.occlusions_no_reading,
            self.cues_while_unseen
        );
        let delays: Vec<String> = self
            .suspend_delay_s
            .iter()
            .map(|d| d.map_or_else(|| "never".to_owned(), |d| format!("{d} s")))
            .collect();
        let _ = writeln!(
            t,
            "  moved camera {}: suspended in time {} (delays: {}), cues while moved {}, false suspension {} s",
            self.relocations,
            self.relocations_suspended,
            if delays.is_empty() {
                "none".to_owned()
            } else {
                delays.join(", ")
            },
            self.cues_while_moved,
            self.false_suspect_s
        );
        t
    }
}

// ------------------------------------------------------------------ the command

/// `hy-companion.exe --hy-posture-report ...` (slice V2-3), for
/// `scripts/windows/Test-HealthierYou-Camera.ps1 -Phase Desk` and `-Phase DeskReport`.
/// Read-only on the store; it writes only the report files it is given. `None` if the
/// first argument is not this command; otherwise the exit code: 0 written, 1 a read or
/// write failure, 4 bad arguments.
///
/// - `--hy-posture-report session --since <Unix ms> --until <Unix ms> --labels <csv>
///   --setup <1-5> --session <1-2> --report <json> [--text <txt>] [--db <file>]`: one desk
///   session's raw counts, from the operator's labels (seconds from `--since`) and the
///   store's records in the window. The thresholds are the ones this build ships.
/// - `--hy-posture-report gate --report <json> [--text <txt>] <session json>...`: the
///   sessions added up per setup and checked against the PRD V2 gate.
#[must_use]
pub fn command(args: &[std::ffi::OsString], default_db: Option<&Path>) -> Option<i32> {
    if args.first()? != "--hy-posture-report" {
        return None;
    }
    let text: Vec<String> = args
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let value = |name: &str| {
        text.iter()
            .position(|a| a == name)
            .and_then(|i| text.get(i + 1))
            .cloned()
    };
    let Some(report) = value("--report") else {
        return Some(4);
    };
    let outcome = match text.first().map(String::as_str) {
        Some("session") => {
            let num = |name: &str| value(name).and_then(|v| v.parse::<i64>().ok());
            let small = |name: &str, max: u32| {
                value(name)
                    .and_then(|v| v.parse::<u32>().ok())
                    .filter(|n| (1..=max).contains(n))
            };
            let db = value("--db")
                .map(std::path::PathBuf::from)
                .or_else(|| default_db.map(Path::to_path_buf));
            let (Some(since), Some(until), Some(labels), Some(setup), Some(session), Some(db)) = (
                num("--since"),
                num("--until"),
                value("--labels"),
                small("--setup", SETUPS),
                small("--session", SESSIONS),
                db,
            ) else {
                return Some(4);
            };
            session_command(&db, since, until, Path::new(&labels), setup, session)
                .map(|r| (serde_json::to_string_pretty(&r), r.text()))
        }
        Some("gate") => {
            // The session files: every argument after "gate" that is not an option or an
            // option's value.
            let mut files = Vec::new();
            let mut i = 1;
            while i < text.len() {
                if text[i].starts_with("--") {
                    i += 2;
                } else {
                    files.push(std::path::PathBuf::from(&text[i]));
                    i += 1;
                }
            }
            if files.is_empty() {
                return Some(4);
            }
            gate_command(&files).map(|g| (serde_json::to_string_pretty(&g), g.text()))
        }
        _ => return Some(4),
    };
    let (json, words) = match outcome {
        Ok((Ok(json), words)) => (json, words),
        Ok((Err(e), _)) => (
            serde_json::json!({ "error": e.to_string() }).to_string(),
            String::new(),
        ),
        Err(e) => (
            serde_json::json!({ "error": e.to_string() }).to_string(),
            String::new(),
        ),
    };
    let failed = words.is_empty();
    let mut ok = std::fs::write(&report, json).is_ok();
    if let Some(t) = value("--text") {
        ok &= std::fs::write(t, words).is_ok();
    }
    Some(if ok && !failed { 0 } else { 1 })
}

fn session_command(
    db: &Path,
    since: i64,
    until: i64,
    labels: &Path,
    setup: u32,
    session: u32,
) -> Result<SessionReport> {
    let labels = parse_labels(&std::fs::read_to_string(labels)?)?;
    let logs = read_log(db, since, until)?;
    Ok(session_report(
        Source::Desk,
        setup,
        session,
        Thresholds::PROVISIONAL,
        &labels,
        &logs,
    ))
}

fn gate_command(files: &[std::path::PathBuf]) -> Result<GateReport> {
    let sessions = files
        .iter()
        .map(|f| {
            let text = std::fs::read_to_string(f)?;
            serde_json::from_str::<SessionReport>(&text).map_err(Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    gate(&sessions)
}

#[cfg(test)]
mod tests;
