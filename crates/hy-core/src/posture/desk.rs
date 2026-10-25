//! Synthetic desk sessions for the posture check (slice V2-3; architecture 4.5 "Harness",
//! PRD V2 gate).
//!
//! The PRD's desk test is five camera and desk setups, two sessions each, with comfortable
//! reading and typing, a second display, staged sustained forward and sideways drifts,
//! absence and occlusion; architecture 4.5 adds a moved camera. This module plays that
//! protocol ([`script`]) through the real camera machine and detector
//! ([`crate::camera::Machine`]) with made-up feature numbers, labels each stretch as an
//! operator would, and scores the result with [`super::quality`]. It is the same report the
//! owner's real sessions produce (`Test-HealthierYou-Camera.ps1 -Phase Desk`).
//!
//! What it can and cannot show: the numbers come from a model of sitting (settling,
//! wandering, sensor noise, glances, reaches) chosen by hand, so a pass here shows only that
//! the harness counts correctly and that the thresholds hang together on plausible traces.
//! It says nothing about real people, cameras or light. Every result is marked synthetic,
//! and the thresholds stay provisional until the real sessions pass.
//!
//! No image of anyone is involved: each frame is nine numbers from a seeded generator, so
//! every run gives the same counts.

use std::time::Duration;

use chrono::DateTime;

use super::quality::{self, GateReport, Label, LabelKind, Logged, SessionReport, Source};
use super::{CueOutcome, Features, HeadRef, PostureState, Thresholds};
use crate::camera::{
    Action, CameraHold, CameraSettings, Consent, FrameIssue, FrameReport, Machine, NOTICE_VERSION,
    Started,
};

/// The posture cue's cooldown in the coaching engine (architecture 4.5: 30 minutes); the
/// harness answers cue requests as the engine would, with no focus or meeting.
const COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// The camera clock ticks this often (the companion's 500 ms clock).
const TICK: Duration = Duration::from_millis(500);
/// Stance changes ease in over about this long.
const EASE_S: f64 = 1.5;

/// One synthetic camera and desk setup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setup {
    /// 1 to 5.
    pub id: u32,
    /// What it stands for.
    pub name: &'static str,
    /// The head pair the worker sees.
    pub head: HeadRef,
    /// Shoulder width as a share of the frame.
    pub width: f64,
    /// The shoulders' midpoint.
    pub mid: (f64, f64),
    /// Shoulder tilt at rest (a camera off to one side).
    pub tilt: f64,
    /// Head roll at rest, degrees.
    pub roll: f64,
    /// Frame-to-frame noise, as a multiple of a still laptop camera's.
    pub noise: f64,
    /// How much the person settles and wanders, as a multiple of the usual.
    pub wander: f64,
    /// Share of frames that are not valid while someone sits in view.
    pub invalid: f64,
    /// Why those frames are not valid.
    pub invalid_issue: FrameIssue,
    /// Mean visibility of valid frames.
    pub visibility: f64,
    /// Inference time per frame, ms (over 250 ms drops the page to 1 fps).
    pub inference_ms: f64,
}

/// The five synthetic setups (PRD: five representative camera and desk setups).
pub const SETUPS: [Setup; 5] = [
    Setup {
        id: 1,
        name: "laptop camera, daylight",
        head: HeadRef::Ears,
        width: 0.38,
        mid: (0.5, 0.72),
        tilt: 0.0,
        roll: 0.0,
        noise: 1.0,
        wander: 1.0,
        invalid: 0.02,
        invalid_issue: FrameIssue::LowConfidence,
        visibility: 0.92,
        inference_ms: 40.0,
    },
    Setup {
        id: 2,
        name: "laptop camera, glasses, evening room light",
        head: HeadRef::Eyes,
        width: 0.36,
        mid: (0.5, 0.74),
        tilt: 0.0,
        roll: 0.0,
        noise: 1.6,
        wander: 1.0,
        invalid: 0.12,
        invalid_issue: FrameIssue::LowConfidence,
        visibility: 0.78,
        inference_ms: 45.0,
    },
    Setup {
        id: 3,
        name: "external camera on the monitor, sitting farther back",
        head: HeadRef::Ears,
        width: 0.22,
        mid: (0.5, 0.8),
        tilt: 0.0,
        roll: 0.0,
        noise: 1.4,
        wander: 1.1,
        invalid: 0.04,
        invalid_issue: FrameIssue::LowConfidence,
        visibility: 0.88,
        inference_ms: 40.0,
    },
    Setup {
        id: 4,
        name: "slow PC (the page drops to 1 frame a second)",
        head: HeadRef::Ears,
        width: 0.40,
        mid: (0.5, 0.72),
        tilt: 0.0,
        roll: 0.0,
        noise: 1.2,
        wander: 1.0,
        invalid: 0.03,
        invalid_issue: FrameIssue::LowConfidence,
        visibility: 0.9,
        inference_ms: 320.0,
    },
    Setup {
        id: 5,
        name: "camera off to one side",
        head: HeadRef::Ears,
        width: 0.30,
        mid: (0.36, 0.74),
        tilt: 0.03,
        roll: 5.0,
        noise: 1.5,
        wander: 1.3,
        invalid: 0.08,
        invalid_issue: FrameIssue::OutOfFrame,
        visibility: 0.84,
        inference_ms: 50.0,
    },
];

/// A stance relative to sitting as usual.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Stance {
    gap: f64,
    /// Relative change of the shoulder width.
    width: f64,
    tilt: f64,
    roll: f64,
    mid_x: f64,
    mid_y: f64,
}

impl Stance {
    fn ease(&mut self, to: Self, k: f64) {
        self.gap += (to.gap - self.gap) * k;
        self.width += (to.width - self.width) * k;
        self.tilt += (to.tilt - self.tilt) * k;
        self.roll += (to.roll - self.roll) * k;
        self.mid_x += (to.mid_x - self.mid_x) * k;
        self.mid_y += (to.mid_y - self.mid_y) * k;
    }
}

/// What a step of the script does.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Act {
    /// Sit (still for a calibration, or at work) in this stance.
    Sit(Stance),
    /// At work, glancing at a second display now and then.
    Glances(Stance),
    /// A reach: a large, brief change, some frames out of frame.
    Reach(Stance),
    /// Out of view, or partly hidden.
    Unseen(FrameIssue),
    /// Sitting as usual, the camera moved (shift of the midpoint, scale of the width).
    Moved { dx: f64, dy: f64, scale: f64 },
    /// Capture released by a hold (a lock).
    Hold,
}

/// One labelled step.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Step {
    label: LabelKind,
    secs: u64,
    act: Act,
}

/// A seeded generator (xorshift64*), so every run gives the same counts.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    #[allow(clippy::cast_precision_loss)]
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn sign(&mut self) -> f64 {
        if self.unit() < 0.5 { -1.0 } else { 1.0 }
    }

    /// Standard normal (Box-Muller).
    fn normal(&mut self) -> f64 {
        let u = self.unit().max(1e-12);
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// A comfortable stance: sitting a little differently from calibration, as people do
/// (normal spreads of about one floor per feature, so now and then a comfortable stretch
/// sits two or three floors away: the source of false cues).
fn settle(rng: &mut Rng, w: f64, lean: f64) -> Stance {
    Stance {
        gap: 0.018 * w * rng.normal() - 0.015 * lean,
        width: 0.012 * w * rng.normal() + 0.02 * lean,
        tilt: 0.008 * w * rng.normal(),
        roll: 1.2 * w * rng.normal(),
        mid_x: 0.008 * rng.normal(),
        mid_y: 0.008 * rng.normal() + 0.01 * lean,
    }
}

fn drift(rng: &mut Rng, forward: bool, subtle: bool) -> Stance {
    let k = if subtle { 0.6 } else { 1.0 };
    if forward {
        Stance {
            gap: -rng.between(0.08, 0.16) * k,
            width: rng.between(0.03, 0.10) * k,
            mid_y: 0.02 * k,
            ..Stance::default()
        }
    } else {
        let s = rng.sign();
        Stance {
            gap: -0.005,
            tilt: s * rng.between(0.04, 0.09) * k,
            roll: s * rng.between(5.0, 12.0) * k,
            mid_x: s * 0.03 * k,
            ..Stance::default()
        }
    }
}

/// The desk protocol for one session (about 38 minutes): calibration; comfortable reading
/// and typing (about 25 minutes, a little differently from calibration each time); two
/// stretches with a second display; two reaches; five staged 90-second drifts (forward and
/// sideways, one of them subtle); an absence; an occlusion; a lock that releases the
/// camera; and a moved camera held for 160 s, then put back and recalibrated.
fn script(setup: &Setup, session: u32, rng: &mut Rng) -> Vec<Step> {
    use LabelKind::{
        Absent, Calibrate, DriftForward, DriftLateral, Occluded, Paused, Reach, Reading, Relocated,
        SecondDisplay, Typing,
    };
    let w = setup.wander;
    let subtle = usize::try_from(rng.next() % 5).unwrap_or(0);
    let mut drifts = 0;
    let mut d = |rng: &mut Rng, forward: bool| {
        let s = drift(rng, forward, drifts == subtle);
        drifts += 1;
        Step {
            label: if forward { DriftForward } else { DriftLateral },
            secs: 90,
            act: Act::Sit(s),
        }
    };
    let work = |label: LabelKind, secs: u64, rng: &mut Rng| {
        let lean = match label {
            Reading => 1.0,
            Typing => 0.5,
            _ => 0.0,
        };
        let s = settle(rng, w, lean);
        Step {
            label,
            secs,
            act: if label == SecondDisplay {
                Act::Glances(s)
            } else {
                Act::Sit(s)
            },
        }
    };
    let reach = |rng: &mut Rng| Step {
        label: Reach,
        secs: 10,
        act: Act::Reach(Stance {
            gap: -0.08,
            width: 0.05,
            tilt: rng.sign() * 0.08,
            roll: 8.0,
            mid_x: 0.10,
            mid_y: 0.03,
        }),
    };
    let calibrate = Step {
        label: Calibrate,
        secs: 30,
        act: Act::Sit(Stance::default()),
    };
    let last_forward = session.is_multiple_of(2);
    let moved = if !session.is_multiple_of(2) {
        // The laptop pushed back: smaller in the frame.
        Act::Moved {
            dx: 0.0,
            dy: -0.04,
            scale: 0.75,
        }
    } else {
        // The screen tilted: higher in the frame.
        Act::Moved {
            dx: 0.0,
            dy: 0.20,
            scale: 0.95,
        }
    };
    vec![
        calibrate,
        work(Reading, 240, rng),
        d(rng, true),
        work(Typing, 240, rng),
        work(SecondDisplay, 75, rng),
        d(rng, false),
        reach(rng),
        work(Typing, 120, rng),
        reach(rng),
        Step {
            label: Absent,
            secs: 45,
            act: Act::Unseen(FrameIssue::Absent),
        },
        work(Reading, 180, rng),
        d(rng, true),
        Step {
            label: Paused,
            secs: 60,
            act: Act::Hold,
        },
        work(Typing, 180, rng),
        Step {
            label: Occluded,
            secs: 30,
            act: Act::Unseen(FrameIssue::LowConfidence),
        },
        work(SecondDisplay, 75, rng),
        d(rng, false),
        work(Reading, 180, rng),
        d(rng, last_forward),
        work(Typing, 120, rng),
        Step {
            label: Relocated,
            secs: 160,
            act: moved,
        },
        calibrate,
        work(Typing, 60, rng),
    ]
}

/// The companion's part, played by the harness: the page, the clock and the engine's
/// answer to a cue.
struct Shell {
    m: Machine,
    t: Duration,
    fps: u8,
    page_due: bool,
    last_shown: Option<Duration>,
    logs: Vec<Logged>,
    /// When each cue was asked, ms (to check what the report derives from the records).
    cues: Vec<i64>,
}

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

impl Shell {
    fn apply(&mut self, actions: Vec<Action>) {
        let mut queue = std::collections::VecDeque::from(actions);
        while let Some(a) = queue.pop_front() {
            let more = match a {
                Action::OpenPage | Action::Retry => {
                    self.page_due = true;
                    Vec::new()
                }
                Action::RequestStop => self.m.page_stopped(self.t),
                Action::ClosePage | Action::DestroyPage => self.m.page_gone(self.t),
                Action::SetFps(f) => {
                    self.fps = f;
                    Vec::new()
                }
                Action::Cue => {
                    self.cues.push(ms(self.t));
                    let shown = self
                        .last_shown
                        .is_none_or(|at| self.t.saturating_sub(at) >= COOLDOWN);
                    if shown {
                        self.last_shown = Some(self.t);
                    }
                    self.m.cue_outcome(if shown {
                        CueOutcome::Shown
                    } else {
                        CueOutcome::Cooldown
                    });
                    Vec::new()
                }
                Action::Record(o) => {
                    self.logs.push(Logged::from_observation(ms(self.t), &o));
                    Vec::new()
                }
                Action::Persist | Action::Notify(_) | Action::Render | Action::RestartWorker => {
                    Vec::new()
                }
            };
            queue.extend(more);
        }
    }

    fn start_page(&mut self) {
        if std::mem::take(&mut self.page_due) {
            let fx = self.m.page_started(
                self.t,
                Started {
                    width: 640,
                    height: 480,
                    cores: 8,
                    model_ms: 120.0,
                },
            );
            self.apply(fx);
        }
    }

    fn tick(&mut self) {
        let fx = self.m.tick(self.t);
        self.apply(fx);
    }
}

/// The person's features this frame (before the camera's geometry).
fn features(setup: &Setup, s: &Stance, t: f64, rng: &mut Rng, seed: f64) -> Features {
    let n = setup.noise;
    let w = setup.wander;
    // Slow wandering: two slow waves per feature.
    let wave = |amp: f64, p1: f64, p2: f64, ph: f64| {
        amp * w * (0.6 * (t / p1 + ph + seed).sin() + 0.4 * (t / p2 + 2.0 * ph + seed).sin())
    };
    let (gap0, span0) = match setup.head {
        HeadRef::Ears => (0.55, 0.45),
        HeadRef::Eyes => (0.72, 0.26),
    };
    Features {
        head: setup.head,
        gap: gap0 + s.gap + wave(0.006, 37.0, 91.0, 0.3) + 0.006 * n * rng.normal(),
        width: setup.width
            * (1.0 + s.width + wave(0.005, 43.0, 113.0, 1.1) + 0.005 * n * rng.normal()),
        tilt: setup.tilt + s.tilt + wave(0.004, 29.0, 71.0, 2.3) + 0.004 * n * rng.normal(),
        roll: setup.roll + s.roll + wave(0.6, 31.0, 83.0, 3.7) + 0.6 * n * rng.normal(),
        mid_x: setup.mid.0 + s.mid_x + 0.003 * n * rng.normal(),
        mid_y: setup.mid.1 + s.mid_y + 0.003 * n * rng.normal(),
        span: span0 * (1.0 + 0.008 * n * rng.normal()),
        turn: 0.02 * n * rng.normal(),
    }
}

/// The seeds of the check set: sessions the sweep never saw, to show a choice did not just
/// fit the tuning set.
pub const CHECK_SEEDS: u64 = 7;

/// Play one session through the camera machine: the labels and the detector's records
/// (ms from the session's start).
#[must_use]
pub fn run(setup: &Setup, session: u32, th: Thresholds) -> (Vec<Label>, Vec<Logged>) {
    run_seeded(setup, session, 0, th)
}

/// [`run`] with another set of seeds (`seeds` 0 is the tuning set; the check set uses
/// [`CHECK_SEEDS`]).
#[must_use]
pub fn run_seeded(
    setup: &Setup,
    session: u32,
    seeds: u64,
    th: Thresholds,
) -> (Vec<Label>, Vec<Logged>) {
    let (labels, logs, _) = play(setup, session, seeds, th);
    (labels, logs)
}

/// [`run_seeded`], with the times the cues were asked.
#[allow(clippy::too_many_lines)]
fn play(
    setup: &Setup,
    session: u32,
    seeds: u64,
    th: Thresholds,
) -> (Vec<Label>, Vec<Logged>, Vec<i64>) {
    let mut rng = Rng::new(u64::from(setup.id) * 1000 + u64::from(session) + seeds * 100_000);
    let seed = rng.between(0.0, 100.0);
    let steps = script(setup, session, &mut rng);
    let settings = CameraSettings {
        consent: Some(Consent {
            notice_version: NOTICE_VERSION,
            given_at: DateTime::UNIX_EPOCH,
        }),
        on: true,
        calibration: None,
    };
    let mut sh = Shell {
        m: Machine::with_thresholds(settings, th),
        t: Duration::ZERO,
        fps: 3,
        page_due: false,
        last_shown: None,
        logs: Vec::new(),
        cues: Vec::new(),
    };
    let fx = sh.m.resume(Duration::ZERO);
    sh.apply(fx);
    sh.start_page();
    let mut labels = Vec::new();
    let mut stance = Stance::default();
    let mut camera = (0.0, 0.0, 1.0);
    let mut next_tick = Duration::ZERO;
    let mut planned = Duration::ZERO;
    for step in steps {
        // Labels follow the plan; frames run until the first one at or past its end.
        let begin = planned;
        let end = begin + Duration::from_secs(step.secs);
        planned = end;
        labels.push(Label {
            kind: step.label,
            start_ms: ms(begin),
            end_ms: ms(end),
        });
        match step.act {
            Act::Hold => {
                let fx = sh.m.set_hold(sh.t, Some(CameraHold::Away));
                sh.apply(fx);
            }
            Act::Moved { dx, dy, scale } => camera = (dx, dy, scale),
            _ => {}
        }
        if step.label == LabelKind::Calibrate {
            // The camera is back where it was; calibrate (again, if the first one fails).
            camera = (0.0, 0.0, 1.0);
            stance = Stance::default();
            if let Ok(fx) = sh.m.calibrate(sh.t) {
                sh.apply(fx);
            }
        }
        while sh.t < end {
            if sh.t >= next_tick {
                sh.tick();
                next_tick = sh.t + TICK;
            }
            sh.start_page();
            let gap = Duration::from_millis(1000 / u64::from(sh.fps.max(1)));
            if step.act == Act::Hold {
                sh.t += TICK;
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let into = sh.t.saturating_sub(begin).as_millis() as f64 / 1000.0;
            #[allow(clippy::cast_precision_loss)]
            let k = (gap.as_millis() as f64 / 1000.0 / EASE_S).min(1.0);
            let mut invalid = (rng.unit() < setup.invalid).then_some(setup.invalid_issue);
            let mut glance = false;
            match step.act {
                Act::Sit(to) | Act::Glances(to) => stance.ease(to, k),
                Act::Reach(to) => {
                    stance.ease(to, k);
                    if rng.unit() < 0.3 {
                        invalid = Some(FrameIssue::OutOfFrame);
                    }
                }
                Act::Moved { .. } => stance.ease(Stance::default(), k),
                Act::Unseen(issue) => invalid = Some(issue),
                Act::Hold => {}
            }
            if let Act::Glances(_) = step.act {
                glance = into % 25.0 < 10.0;
            }
            let report = if let Some(issue) = invalid {
                FrameReport {
                    valid: false,
                    issue: Some(issue),
                    inference_ms: setup.inference_ms,
                    visibility: if issue == FrameIssue::Absent {
                        0.0
                    } else {
                        0.3
                    },
                    features: None,
                }
            } else {
                let mut f = features(setup, &stance, sh.t.as_secs_f64(), &mut rng, seed);
                if glance {
                    // Turned to a second display: the nose moves off the head pair's
                    // middle and the pair looks narrower.
                    f.turn += 0.45;
                    f.span *= 0.82;
                    f.roll += 2.0;
                }
                f.mid_x = 0.5 + (f.mid_x - 0.5) * camera.2 + camera.0;
                f.mid_y = 0.5 + (f.mid_y - 0.5) * camera.2 + camera.1;
                f.width *= camera.2;
                FrameReport {
                    valid: true,
                    issue: None,
                    inference_ms: setup.inference_ms,
                    visibility: (setup.visibility + 0.03 * rng.normal()).clamp(0.5, 1.0),
                    features: Some(f),
                }
            };
            if let Ok(fx) = sh.m.frame(sh.t, report) {
                sh.apply(fx);
            }
            sh.t += gap;
            if step.label == LabelKind::Calibrate
                && matches!(sh.m.posture_state(), PostureState::NotUsable(_))
                && end.saturating_sub(sh.t) > Duration::from_secs(15)
                && let Ok(fx) = sh.m.calibrate(sh.t)
            {
                sh.apply(fx);
            }
        }
        if step.act == Act::Hold {
            let fx = sh.m.set_hold(sh.t, None);
            sh.apply(fx);
            sh.start_page();
        }
    }
    let fx = sh.m.turn_off(sh.t);
    sh.apply(fx);
    (labels, sh.logs, sh.cues)
}

/// One synthetic session's report.
#[must_use]
pub fn session(setup: &Setup, session: u32, seeds: u64, th: Thresholds) -> SessionReport {
    let (labels, logs) = run_seeded(setup, session, seeds, th);
    quality::session_report(Source::Synthetic, setup.id, session, th, &labels, &logs)
}

/// All five setups, two sessions each (the tuning set), scored against the PRD V2 gate.
#[must_use]
pub fn all(th: Thresholds) -> (Vec<SessionReport>, GateReport) {
    all_seeded(th, 0)
}

/// [`all`] on another set of seeds.
#[must_use]
pub fn all_seeded(th: Thresholds, seeds: u64) -> (Vec<SessionReport>, GateReport) {
    let sessions: Vec<SessionReport> = SETUPS
        .iter()
        .flat_map(|s| (1..=quality::SESSIONS).map(move |n| (s, n)))
        .map(|(s, n)| session(s, n, seeds, th))
        .collect();
    let gate = quality::gate(&sessions).unwrap_or_else(|_| GateReport {
        source: Source::Synthetic,
        thresholds: th,
        setups: Vec::new(),
        pass: false,
    });
    (sessions, gate)
}

/// The drift thresholds the sweep tries: z to start, z to end and the calm before an
/// episode ends, around the provisional values (the viewpoint values stay as shipped).
#[must_use]
pub fn drift_grid() -> Vec<Thresholds> {
    let mut out = Vec::new();
    for z_drift in [2.0, 2.5, 3.0, 3.5, 4.0] {
        for z_end in [1.5, 2.0, 2.5] {
            if z_end >= z_drift {
                continue;
            }
            for end_after_s in [5, 10, 20] {
                out.push(Thresholds {
                    z_drift,
                    z_end,
                    end_after_s,
                    ..Thresholds::PROVISIONAL
                });
            }
        }
    }
    out
}

/// The viewpoint thresholds the sweep tries: the head-ratio band, the turn tolerance and
/// the match tolerances (the drift values stay as shipped).
#[must_use]
pub fn viewpoint_grid() -> Vec<Thresholds> {
    let mut out = Vec::new();
    for span_band_min in [0.05, 0.10, 0.15] {
        for turn_tolerance in [0.10, 0.15, 0.25] {
            for (match_shift, match_scale) in [(0.05, 0.075), (0.075, 0.10), (0.10, 0.15)] {
                out.push(Thresholds {
                    span_band_min,
                    turn_tolerance,
                    match_shift,
                    match_scale,
                    ..Thresholds::PROVISIONAL
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
