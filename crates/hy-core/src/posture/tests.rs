//! The posture detector on synthetic feature traces (architecture 11: T-C1 to T-C4 and
//! T-C6). No image of anyone: each frame is a handful of made-up numbers.

use super::*;

fn s(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

fn wall() -> DateTime<Utc> {
    DateTime::from_timestamp(1_793_000_000, 0).unwrap()
}

/// A person sitting as usual, seen with both ears.
fn base() -> Features {
    Features {
        head: HeadRef::Ears,
        gap: 0.55,
        width: 0.35,
        tilt: 0.0,
        roll: 0.0,
        mid_x: 0.5,
        mid_y: 0.75,
        span: 0.45,
        turn: 0.0,
    }
}

/// A small, repeatable wobble of `amp` at time `t`.
fn wobble(t: Duration, amp: f64, phase: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let x = t.as_millis() as f64 / 1000.0;
    amp * (x * 1.37 + phase).sin()
}

/// Natural sitting jitter, well inside the floors.
fn jitter(t: Duration, f: Features) -> Features {
    Features {
        gap: f.gap + wobble(t, 0.008, 0.0),
        width: f.width * (1.0 + wobble(t, 0.006, 1.0)),
        tilt: f.tilt + wobble(t, 0.005, 2.0),
        roll: f.roll + wobble(t, 0.8, 3.0),
        mid_x: f.mid_x + wobble(t, 0.004, 4.0),
        mid_y: f.mid_y + wobble(t, 0.004, 5.0),
        span: f.span * (1.0 + wobble(t, 0.01, 6.0)),
        turn: f.turn + wobble(t, 0.02, 7.0),
        ..f
    }
}

struct Run {
    d: Detector,
    t: Duration,
    out: Vec<PostureAction>,
}

impl Run {
    fn new(calibration: Option<Calibration>) -> Self {
        let mut d = Detector::new(calibration);
        let out = d.capture_started(Duration::ZERO);
        Self {
            d,
            t: Duration::ZERO,
            out,
        }
    }

    /// Frames at 3 a second for `secs`: `frame(t)` gives each one (None: not valid).
    fn feed(&mut self, secs: u64, frame: impl Fn(Duration) -> Option<Features>) {
        let end = self.t + s(secs);
        while self.t < end {
            let f = frame(self.t);
            let vis = if f.is_some() { 0.9 } else { 0.0 };
            let fx = self.d.frame(self.t, wall(), vis, f);
            self.out.extend(fx);
            let fx = self.d.tick(self.t, wall());
            self.out.extend(fx);
            self.t += Duration::from_millis(333);
        }
    }

    fn sit(&mut self, secs: u64) {
        self.feed(secs, |t| Some(jitter(t, base())));
    }

    fn calibrate(&mut self) {
        let fx = self.d.start_calibration(self.t);
        self.out.extend(fx);
        self.sit(20);
        assert_eq!(self.d.state(), PostureState::Ready, "{:?}", self.d.state());
    }

    fn take(&mut self) -> Vec<PostureAction> {
        std::mem::take(&mut self.out)
    }

    fn cues(&self) -> usize {
        self.out
            .iter()
            .filter(|a| **a == PostureAction::Cue)
            .count()
    }

    fn records(&self, kind: PostureKind) -> Vec<Observation> {
        self.out
            .iter()
            .filter_map(|a| match a {
                PostureAction::Record(o) if o.kind == kind => Some(*o),
                _ => None,
            })
            .collect()
    }
}

fn calibrated() -> Run {
    let mut r = Run::new(None);
    r.calibrate();
    r.take();
    r
}

// ------------------------------------------------------------------ calibration

#[test]
fn a_still_person_gives_a_usable_calibration_in_well_under_thirty_seconds() {
    let mut r = Run::new(None);
    assert_eq!(r.d.state(), PostureState::NotCalibrated);
    assert!(!r.d.scoring());
    r.d.start_calibration(r.t);
    assert!(r.d.calibrating());
    r.sit(5);
    assert!(matches!(
        r.d.state(),
        PostureState::Calibrating { valid, needed: 40 } if valid > 10 && valid < 40
    ));
    r.sit(15);
    assert_eq!(r.d.state(), PostureState::Ready);
    assert!(r.d.scoring());
    assert!(r.out.contains(&PostureAction::Persist));
    let cal = r.d.calibration().unwrap();
    assert_eq!(cal.head, HeadRef::Ears);
    assert!((cal.width.median - 0.35).abs() < 0.01);
    assert!((cal.mid_x - 0.5).abs() < 0.01);
    assert_eq!(cal.at, wall());
    cal.validate().unwrap();
    // Stored as numbers only, and read back.
    let json = serde_json::to_value(cal).unwrap();
    assert_eq!(serde_json::from_value::<Calibration>(json).unwrap(), cal);
    assert_eq!(r.d.stats().calibrations, 1);
}

fn calibration_result(frame: impl Fn(Duration) -> Option<Features>, vis: f64) -> PostureState {
    let mut d = Detector::new(None);
    d.capture_started(Duration::ZERO);
    d.start_calibration(Duration::ZERO);
    let mut t = Duration::ZERO;
    while d.calibrating() {
        let f = frame(t);
        d.frame(t, wall(), if f.is_some() { vis } else { 0.0 }, f);
        d.tick(t, wall());
        t += Duration::from_millis(333);
        assert!(t <= s(31), "a calibration ends within 30 s");
    }
    d.state()
}

#[test]
fn each_calibration_check_refuses_with_its_reason() {
    let ok = |t| Some(jitter(t, base()));
    assert_eq!(calibration_result(ok, 0.9), PostureState::Ready);
    // Out of view half of the time.
    let half = |t: Duration| (t.as_millis() % 2000 < 1000).then(|| jitter(t, base()));
    assert_eq!(
        calibration_result(half, 0.9),
        PostureState::NotUsable(CalibrationIssue::OutOfView)
    );
    // Too far, too close.
    let far = |t| {
        Some(Features {
            width: 0.10,
            ..jitter(t, base())
        })
    };
    assert_eq!(
        calibration_result(far, 0.9),
        PostureState::NotUsable(CalibrationIssue::TooFar)
    );
    let close = |t| {
        Some(Features {
            width: 0.80,
            ..jitter(t, base())
        })
    };
    assert_eq!(
        calibration_result(close, 0.9),
        PostureState::NotUsable(CalibrationIssue::TooClose)
    );
    // Poorly visible.
    assert_eq!(
        calibration_result(ok, 0.6),
        PostureState::NotUsable(CalibrationIssue::PoorlyVisible)
    );
    // Moving: the gap swings well beyond three floors.
    let moving = |t| {
        Some(Features {
            gap: 0.55 + wobble(t, 0.25, 0.0),
            ..base()
        })
    };
    assert_eq!(
        calibration_result(moving, 0.9),
        PostureState::NotUsable(CalibrationIssue::Moved)
    );
    // Valid often enough, but too few frames in 30 s: at one frame a second (the
    // governor's low rate), 30 s gives 30 valid frames.
    let mut d = Detector::new(None);
    d.capture_started(Duration::ZERO);
    d.start_calibration(Duration::ZERO);
    for n in 0..=31 {
        let t = s(n);
        d.frame(t, wall(), 0.9, Some(jitter(t, base())));
        d.tick(t, wall());
    }
    assert_eq!(
        d.state(),
        PostureState::NotUsable(CalibrationIssue::TooFewFrames)
    );
    for issue in [
        CalibrationIssue::TooFewFrames,
        CalibrationIssue::OutOfView,
        CalibrationIssue::TooFar,
        CalibrationIssue::TooClose,
        CalibrationIssue::PoorlyVisible,
        CalibrationIssue::Moved,
        CalibrationIssue::Interrupted,
    ] {
        assert!(!issue.words().is_empty() && issue.words().len() < 80);
    }
}

#[test]
fn a_failed_calibration_drops_the_old_one_and_gives_no_cues() {
    let mut r = calibrated();
    r.d.start_calibration(r.t);
    r.feed(31, |_| None);
    assert_eq!(
        r.d.state(),
        PostureState::NotUsable(CalibrationIssue::OutOfView)
    );
    assert!(r.d.calibration().is_none());
    assert!(r.out.contains(&PostureAction::Persist));
    r.take();
    r.feed(300, |t| {
        Some(Features {
            gap: 0.30,
            ..jitter(t, base())
        })
    });
    assert_eq!(r.cues(), 0);
}

#[test]
fn a_calibration_cut_short_by_the_camera_stopping_is_not_usable() {
    let mut r = Run::new(None);
    r.d.start_calibration(r.t);
    r.sit(5);
    r.d.capture_ended(r.t);
    assert_eq!(
        r.d.state(),
        PostureState::NotUsable(CalibrationIssue::Interrupted)
    );
    // Cancel keeps what was there.
    let mut r = calibrated();
    r.d.start_calibration(r.t);
    r.sit(3);
    assert_eq!(r.d.cancel_calibration(), vec![PostureAction::Render]);
    assert_eq!(r.d.state(), PostureState::Ready);
}

// ------------------------------------------------------------------ floors and drift

#[test]
fn t_c1_a_perfectly_still_calibration_is_bounded_by_the_floors() {
    // Calibrated on identical frames: every MAD is zero, so only the floors divide.
    let mut r = Run::new(None);
    r.d.start_calibration(r.t);
    r.feed(20, |_| Some(base()));
    assert_eq!(r.d.state(), PostureState::Ready);
    let cal = r.d.calibration().unwrap();
    assert!(cal.gap.mad == 0.0 && cal.roll.mad == 0.0);
    // One floor's worth of change is a z of exactly 1, never a huge number.
    let f = Features {
        gap: 0.55 + FLOOR_GAP,
        roll: FLOOR_ROLL,
        ..base()
    };
    let z = cal.scores(&f);
    assert!(
        (z.gap - 1.0).abs() < 1e-9 && (z.roll - 1.0).abs() < 1e-9,
        "{z:?}"
    );
    r.take();
    // Ten minutes of normal jitter: no cue.
    r.feed(600, |t| Some(jitter(t, base())));
    assert_eq!(r.cues(), 0);
    assert!(r.records(PostureKind::DriftEpisode).is_empty());
}

#[test]
fn t_c2_low_variance_calibration_with_larger_jitter_gives_no_false_cue() {
    let mut r = calibrated();
    // Thirty minutes of fidgeting well beyond the calibration's spread: short swings
    // that average out over ten seconds.
    r.feed(1800, |t| {
        let f = jitter(t, base());
        Some(Features {
            gap: f.gap + wobble(t, 0.04, 0.3),
            tilt: f.tilt + wobble(t, 0.03, 0.9),
            roll: f.roll + wobble(t, 4.0, 1.7),
            ..f
        })
    });
    assert_eq!(r.cues(), 0);
}

fn slumped(t: Duration) -> Option<Features> {
    // The head sinks towards the shoulders: the gap falls by 0.15 (7.5 floors).
    Some(Features {
        gap: 0.40,
        ..jitter(t, base())
    })
}

#[test]
fn sustained_drift_gives_one_cue_and_one_episode() {
    let mut r = calibrated();
    r.sit(30);
    let start = r.t;
    r.feed(55, slumped);
    assert_eq!(r.cues(), 0, "not before a minute of drift");
    r.feed(30, slumped);
    assert_eq!(
        r.cues(),
        1,
        "the cue follows within 30 s of the sustained threshold"
    );
    r.d.cue_outcome(CueOutcome::Shown);
    // Staying slumped: still one episode, no more cues.
    r.feed(300, slumped);
    assert_eq!(r.cues(), 1);
    assert!(r.records(PostureKind::DriftEpisode).is_empty());
    // Back to normal: the episode ends and is recorded once, with its cue outcome.
    r.sit(40);
    let eps = r.records(PostureKind::DriftEpisode);
    assert_eq!(eps.len(), 1, "{eps:?}");
    assert_eq!(eps[0].cue, Some(CueOutcome::Shown));
    let len = eps[0].duration_s;
    let slumped_for = u32::try_from((r.t - start).as_secs()).unwrap() - 40;
    assert!(
        len + 15 >= slumped_for && len <= slumped_for + 15,
        "{len} vs {slumped_for}"
    );
    eps[0].payload().validate().unwrap();
    let st = r.d.stats();
    assert_eq!((st.episodes, st.cues_shown), (1, 1));
    // A second episode later is a second cue request.
    r.take();
    r.feed(90, slumped);
    assert_eq!(r.cues(), 1);
    r.d.cue_outcome(CueOutcome::Cooldown);
    r.sit(40);
    assert_eq!(
        r.records(PostureKind::DriftEpisode)[0].cue,
        Some(CueOutcome::Cooldown)
    );
    assert_eq!(r.d.stats().cues_suppressed, 1);
}

#[test]
fn leaning_in_and_tilting_count_as_drift_too() {
    let changes: [fn(Features) -> Features; 3] = [
        |f| Features {
            width: f.width * 1.12,
            gap: f.gap - 0.03,
            ..f
        },
        |f| Features {
            tilt: f.tilt + 0.08,
            ..f
        },
        |f| Features {
            roll: f.roll + 12.0,
            ..f
        },
    ];
    for change in changes {
        let mut r = calibrated();
        r.feed(80, |t| Some(change(jitter(t, base()))));
        assert_eq!(r.cues(), 1);
    }
}

#[test]
fn t_c3_reaching_turning_and_standing_never_cue() {
    let mut r = calibrated();
    for _ in 0..10 {
        r.sit(60);
        // Reach for something: out of frame for 10 s.
        r.feed(10, |_| None);
        r.sit(30);
        // Turn to a second display for 40 s: the head turns and rolls a little.
        r.feed(40, |t| {
            Some(Features {
                turn: 0.35,
                roll: 6.0,
                span: 0.32,
                ..jitter(t, base())
            })
        });
        r.sit(30);
        // Stand up and stretch: no valid frame for 30 s.
        r.feed(30, |_| None);
        // Lean in to read something for 45 s.
        r.feed(45, |t| {
            Some(Features {
                width: 0.42,
                gap: 0.47,
                ..jitter(t, base())
            })
        });
    }
    assert_eq!(r.cues(), 0);
    assert!(r.records(PostureKind::DriftEpisode).is_empty());
}

#[test]
fn drift_with_too_few_valid_frames_is_not_an_episode() {
    let mut r = calibrated();
    // Slumped, but valid only half of the time (under 70%).
    r.feed(180, |t| {
        if t.as_millis() % 2000 < 1000 {
            slumped(t)
        } else {
            None
        }
    });
    assert_eq!(r.cues(), 0);
}

// ------------------------------------------------------------------ viewpoint check

fn moved_camera(t: Duration) -> Option<Features> {
    // The laptop pushed back: the shoulders sit higher and smaller in the frame.
    let f = jitter(t, base());
    Some(Features {
        mid_y: f.mid_y - 0.18,
        width: f.width * 0.75,
        ..f
    })
}

#[test]
fn t_c4_a_moved_camera_suspends_cues_then_asks_once_to_recalibrate() {
    let mut r = calibrated();
    r.sit(30);
    r.feed(25, moved_camera);
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::CameraMoved),
        "suspended within 20 s of the step"
    );
    assert!(!r.d.scoring());
    // No cue against the old baseline, however long it lasts (the new view also looks
    // like a large change in width).
    r.feed(130, moved_camera);
    assert_eq!(r.cues(), 0);
    // After two minutes of reading without a match: invalid, asked once.
    assert_eq!(r.d.state(), PostureState::Recalibrate);
    assert_eq!(
        r.out
            .iter()
            .filter(|a| **a == PostureAction::AskRecalibrate)
            .count(),
        1
    );
    assert!(r.d.calibration().is_none());
    assert!(r.out.contains(&PostureAction::Persist));
    let sus = r.records(PostureKind::BaselineSuspect);
    assert_eq!(sus.len(), 1);
    assert_eq!(sus[0].reason, Some(PostureReason::CameraMoved));
    assert!(sus[0].duration_s >= 110);
    sus[0].payload().validate().unwrap();
    // Nothing more is scored or asked until a recalibration.
    r.take();
    r.feed(300, |t| moved_camera(t).map(|f| Features { gap: 0.3, ..f }));
    assert_eq!(r.cues(), 0);
    assert!(!r.out.contains(&PostureAction::AskRecalibrate));
    assert_eq!(r.d.stats().invalidations, 1);
    // Recalibrating resumes.
    r.d.start_calibration(r.t);
    r.feed(20, moved_camera);
    assert_eq!(r.d.state(), PostureState::Ready);
}

#[test]
fn a_camera_moved_back_restores_the_baseline() {
    let mut r = calibrated();
    r.feed(25, moved_camera);
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    r.sit(25);
    assert_eq!(r.d.state(), PostureState::Ready);
    let sus = r.records(PostureKind::BaselineSuspect);
    assert_eq!(sus.len(), 1);
    assert!(sus[0].duration_s >= 20);
}

#[test]
fn a_step_that_does_not_hold_or_a_slow_change_is_not_a_move() {
    let mut r = calibrated();
    // Bumped for 10 s, then back.
    r.feed(10, moved_camera);
    r.sit(60);
    assert_eq!(r.d.state(), PostureState::Ready);
    // A slow slide over a minute is no step.
    r.feed(60, |t| {
        #[allow(clippy::cast_precision_loss)]
        let k = (t.as_millis() % 60_000) as f64 / 60_000.0;
        let f = jitter(t, base());
        Some(Features {
            mid_x: f.mid_x + 0.06 * k,
            ..f
        })
    });
    assert_eq!(r.d.state(), PostureState::Ready);
}

#[test]
fn a_viewpoint_change_with_steady_posture_is_suspect_but_head_motion_is_not() {
    // The screen tilted: the head pair looks wider against the shoulders, posture
    // unchanged, the head facing as before.
    let mut r = calibrated();
    r.feed(30, |t| {
        let f = jitter(t, base());
        Some(Features {
            span: f.span * 1.3,
            ..f
        })
    });
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::ViewpointChanged)
    );
    // Turning the head changes the same ratio, but the turn shows it: no suspect.
    let mut r = calibrated();
    r.feed(120, |t| {
        let f = jitter(t, base());
        Some(Features {
            span: f.span * 0.7,
            turn: 0.4,
            ..f
        })
    });
    assert_eq!(r.d.state(), PostureState::Ready);
}

// ------------------------------------------------------------------ restarts

#[test]
fn t_c6_a_stored_calibration_is_accepted_only_if_the_first_twenty_seconds_match() {
    let cal = calibrated().d.calibration().unwrap();
    // Same desk: accepted after 20 s of reading.
    let mut r = Run::new(Some(cal));
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::AppRestart)
    );
    assert!(!r.d.scoring());
    r.feed(10, |_| None);
    r.sit(15);
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::AppRestart)
    );
    r.sit(10);
    assert_eq!(r.d.state(), PostureState::Ready);
    assert_eq!(
        r.records(PostureKind::BaselineSuspect)[0].reason,
        Some(PostureReason::AppRestart)
    );
    // Moved overnight: invalid after the first 20 s, asked once, dropped.
    let mut r = Run::new(Some(cal));
    r.feed(25, moved_camera);
    assert_eq!(r.d.state(), PostureState::Recalibrate);
    assert!(r.out.contains(&PostureAction::AskRecalibrate));
    assert!(r.d.calibration().is_none());
    // Unusable stored numbers are ignored.
    let bad = Calibration {
        width: Stat {
            median: f64::NAN,
            mad: 0.0,
        },
        ..cal
    };
    assert_eq!(
        Detector::new(Some(bad)).state(),
        PostureState::NotCalibrated
    );
}

#[test]
fn a_camera_restart_is_suspect_until_the_view_matches_again() {
    let mut r = calibrated();
    r.sit(10);
    let fx = r.d.capture_ended(r.t);
    r.out.extend(fx);
    r.t += s(600);
    let fx = r.d.capture_started(r.t);
    r.out.extend(fx);
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::CameraRestart)
    );
    r.sit(25);
    assert_eq!(r.d.state(), PostureState::Ready);
    let sus = r.records(PostureKind::BaselineSuspect);
    assert_eq!(sus.len(), 1);
    assert_eq!(sus[0].reason, Some(PostureReason::CameraRestart));
    assert!(sus[0].duration_s < 60, "the hold itself is not counted");
}

// ------------------------------------------------------------------ records

#[test]
fn the_camera_stopping_ends_an_episode_and_open_intervals() {
    let mut r = calibrated();
    r.feed(80, slumped);
    assert_eq!(r.cues(), 1);
    // The engine never answered.
    let fx = r.d.capture_ended(r.t);
    r.out.extend(fx);
    let eps = r.records(PostureKind::DriftEpisode);
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].cue, Some(CueOutcome::NoAnswer));
    // Nothing is kept after the stream ends.
    assert!(r.d.history.is_empty());
}

#[test]
fn no_reading_intervals_are_recorded_with_their_reason() {
    let mut d = Detector::new(None);
    let mut out = d.capture_started(s(0));
    out.extend(d.reading_changed(s(10), Some(PostureReason::Absent)));
    out.extend(d.reading_changed(s(40), Some(PostureReason::OutOfFrame)));
    out.extend(d.reading_changed(s(42), None));
    out.extend(d.reading_changed(s(50), Some(PostureReason::Stalled)));
    out.extend(d.capture_ended(s(58)));
    let rec: Vec<_> = out
        .iter()
        .filter_map(|a| match a {
            PostureAction::Record(o) => Some((o.kind, o.duration_s, o.reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rec,
        vec![
            (PostureKind::NoReading, 30, Some(PostureReason::Absent)),
            // 2 s out of frame is too short to record.
            (PostureKind::NoReading, 8, Some(PostureReason::Stalled)),
        ]
    );
}

#[test]
fn memory_holds_at_most_ninety_seconds_of_numbers() {
    let mut r = calibrated();
    r.sit(400);
    let oldest = r.d.history.front().unwrap().t;
    assert!(r.t - oldest <= HISTORY_FOR + s(1));
}

#[test]
fn features_from_a_page_are_checked() {
    base().validate().unwrap();
    for bad in [
        Features {
            gap: f64::NAN,
            ..base()
        },
        Features {
            width: 0.0,
            ..base()
        },
        Features {
            roll: 400.0,
            ..base()
        },
        Features {
            span: -1.0,
            ..base()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
    let json = serde_json::to_value(base()).unwrap();
    assert_eq!(json["head"], "ears");
    assert!(
        serde_json::from_value::<Features>(serde_json::json!({
            "head": "ears", "gap": 0.5, "width": 0.3, "tilt": 0, "roll": 0, "mid_x": 0.5,
            "mid_y": 0.7, "span": 0.4, "turn": 0, "landmarks": []
        }))
        .is_err(),
        "nothing else rides along"
    );
}

#[test]
fn the_engines_answers_map_to_cue_outcomes() {
    use crate::coach::InputOutcome;
    use crate::coach::engine::{CUE_BUSY, CUE_COOLDOWN, CUE_FOCUS, CUE_HELD};
    assert_eq!(cue_outcome_of(&InputOutcome::Applied), CueOutcome::Shown);
    assert_eq!(
        cue_outcome_of(&InputOutcome::Ignored(CUE_FOCUS)),
        CueOutcome::Focus
    );
    assert_eq!(
        cue_outcome_of(&InputOutcome::Ignored(CUE_HELD)),
        CueOutcome::Held
    );
    assert_eq!(
        cue_outcome_of(&InputOutcome::Ignored(CUE_COOLDOWN)),
        CueOutcome::Cooldown
    );
    assert_eq!(
        cue_outcome_of(&InputOutcome::Ignored(CUE_BUSY)),
        CueOutcome::Busy
    );
    assert_eq!(
        cue_outcome_of(&InputOutcome::Ignored("something else")),
        CueOutcome::NoAnswer
    );
}
