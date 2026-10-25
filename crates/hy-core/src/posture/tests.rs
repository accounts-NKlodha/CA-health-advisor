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
    let fx = r.d.cancel_calibration();
    assert_eq!(fx.last(), Some(&PostureAction::Render));
    // The seconds spent calibrating were not scored (slice V2-3).
    assert!(
        fx.iter().all(|a| match a {
            PostureAction::Record(o) =>
                o.kind == PostureKind::BaselineSuspect
                    && o.reason == Some(PostureReason::NoCalibration),
            a => *a == PostureAction::Render,
        })
    );
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

fn suspect_after_a_move() -> Run {
    let mut r = calibrated();
    r.feed(25, moved_camera);
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::CameraMoved)
    );
    r.take();
    r
}

#[test]
fn one_match_then_prolonged_invalid_readings_does_not_restore_the_baseline() {
    // No person in view.
    let mut r = suspect_after_a_move();
    r.sit(1);
    r.feed(25, |_| None);
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::CameraMoved),
        "frames without features are not matching evidence"
    );
    // A single matching frame afterwards starts a new run; it does not finish the old one.
    r.sit(1);
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    // Frames whose head pair differs from the calibration's are unusable too.
    let mut r = suspect_after_a_move();
    r.sit(1);
    r.feed(25, |t| {
        Some(Features {
            head: HeadRef::Eyes,
            ..jitter(t, base())
        })
    });
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    assert!(r.records(PostureKind::BaselineSuspect).is_empty());
}

#[test]
fn intermittent_dropouts_only_count_while_matches_stay_continuous() {
    // A second of match then three seconds of nothing, for a minute: never 20 s of
    // continuous match.
    let mut r = suspect_after_a_move();
    r.feed(60, |t| (t.as_secs() % 4 == 0).then(|| jitter(t, base())));
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    // Brief dropouts (one second in three) inside a sustained match: the run continues.
    let mut r = suspect_after_a_move();
    r.feed(19, |t| (t.as_secs() % 3 != 2).then(|| jitter(t, base())));
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    r.feed(3, |t| (t.as_secs() % 3 != 2).then(|| jitter(t, base())));
    assert_eq!(r.d.state(), PostureState::Ready);
}

#[test]
fn sustained_matching_after_invalid_readings_restores_the_baseline() {
    let mut r = suspect_after_a_move();
    r.sit(1);
    r.feed(25, |_| None);
    r.feed(10, |t| (t.as_secs() % 4 == 0).then(|| jitter(t, base())));
    r.feed(3, |_| None);
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    r.sit(19);
    assert!(matches!(r.d.state(), PostureState::Suspect(_)));
    r.sit(2);
    assert_eq!(r.d.state(), PostureState::Ready);
    let sus = r.records(PostureKind::BaselineSuspect);
    assert_eq!(sus.len(), 1);
    assert_eq!(sus[0].reason, Some(PostureReason::CameraMoved));
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
    let of = |k: PostureKind| rec.iter().filter(|r| r.0 == k).copied().collect::<Vec<_>>();
    assert_eq!(
        of(PostureKind::NoReading),
        vec![
            (PostureKind::NoReading, 30, Some(PostureReason::Absent)),
            // 2 s out of frame is too short for its own record: it is added up with any
            // other short spells and written before the next long one (V2-3).
            (PostureKind::NoReading, 2, Some(PostureReason::OutOfFrame)),
            (PostureKind::NoReading, 8, Some(PostureReason::Stalled)),
        ]
    );
    let spells: Vec<_> = out
        .iter()
        .filter_map(|a| match a {
            PostureAction::Record(o) if o.kind == PostureKind::NoReading => Some(o.spells),
            _ => None,
        })
        .collect();
    assert_eq!(spells, vec![None, Some(1), None]);
    // V2-3: the camera-on time, and (with no calibration) the time not scored.
    assert_eq!(
        of(PostureKind::Capture),
        vec![(PostureKind::Capture, 58, None)]
    );
    assert_eq!(
        of(PostureKind::BaselineSuspect),
        vec![(
            PostureKind::BaselineSuspect,
            58,
            Some(PostureReason::NoCalibration)
        )]
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

// ------------------------------------------------------------------ slice V2-3

#[test]
fn an_episode_that_ends_in_calm_is_handed_over_with_when_it_ended() {
    let mut r = calibrated();
    r.sit(20);
    r.feed(90, slumped);
    assert_eq!(r.cues(), 1);
    r.sit(40);
    let eps = r.records(PostureKind::DriftEpisode);
    assert_eq!(eps.len(), 1);
    // Written END_AFTER after the calm began, so it ended that long before.
    let ago = eps[0].ago.as_secs_f64();
    assert!((10.0..=12.0).contains(&ago), "{ago}");
    // Its length runs from when the drift began (give or take smoothing) to the calm, so
    // its start is its end less its length.
    let d = eps[0].duration_s;
    assert!((85..=100).contains(&d), "{d}");
}

#[test]
fn open_intervals_are_written_every_segment_and_start_again() {
    let mut d = Detector::new(None);
    let mut out = d.capture_started(s(0));
    out.extend(d.reading_changed(s(10), Some(PostureReason::Absent)));
    for t in [300, 600, 899, 900, 901, 1000] {
        out.extend(d.tick(s(t), wall()));
    }
    let rec = |out: &[PostureAction]| -> Vec<(PostureKind, u32, Option<PostureReason>)> {
        out.iter()
            .filter_map(|a| match a {
                PostureAction::Record(o) => Some((o.kind, o.duration_s, o.reason)),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        rec(&out),
        vec![
            (PostureKind::Capture, 900, None),
            (
                PostureKind::BaselineSuspect,
                900,
                Some(PostureReason::NoCalibration)
            ),
            (PostureKind::NoReading, 890, Some(PostureReason::Absent)),
        ]
    );
    let out = d.capture_ended(s(1200));
    assert_eq!(
        rec(&out),
        vec![
            (PostureKind::Capture, 300, None),
            (
                PostureKind::BaselineSuspect,
                300,
                Some(PostureReason::NoCalibration)
            ),
            (PostureKind::NoReading, 300, Some(PostureReason::Absent)),
        ]
    );
    for a in &out {
        if let PostureAction::Record(o) = a {
            o.payload().validate().unwrap();
        }
    }
}

#[test]
fn a_suspect_baseline_is_written_every_segment_too() {
    let mut r = calibrated();
    r.take();
    // A camera restart with nobody matching: suspect until it is invalid (2 minutes of
    // reading), so feed absent frames for 20 minutes (no reading time accumulates).
    let fx = r.d.capture_ended(r.t);
    r.out.extend(fx);
    let fx = r.d.capture_started(r.t);
    r.out.extend(fx);
    r.feed(16 * 60, |_| None);
    let sus: Vec<_> = r
        .records(PostureKind::BaselineSuspect)
        .into_iter()
        .filter(|o| o.reason == Some(PostureReason::CameraRestart))
        .collect();
    assert_eq!(sus.len(), 1, "{sus:?}");
    assert!((899..=901).contains(&sus[0].duration_s));
    assert_eq!(
        r.d.state(),
        PostureState::Suspect(PostureReason::CameraRestart)
    );
}

#[test]
fn time_without_a_usable_calibration_is_recorded_until_recalibration() {
    let mut r = calibrated();
    r.sit(30);
    r.feed(160, moved_camera);
    assert_eq!(r.d.state(), PostureState::Recalibrate);
    r.take();
    r.feed(60, moved_camera);
    r.d.start_calibration(r.t);
    r.feed(20, moved_camera);
    assert_eq!(r.d.state(), PostureState::Ready);
    let none: Vec<_> = r
        .records(PostureKind::BaselineSuspect)
        .into_iter()
        .filter(|o| o.reason == Some(PostureReason::NoCalibration))
        .collect();
    assert_eq!(none.len(), 1, "{none:?}");
    // From the invalidation (about 20 s before the end of the 160 s), through the 60 s,
    // to the end of the calibration (about 13 s).
    assert!((85..=100).contains(&none[0].duration_s), "{none:?}");
}

#[test]
fn thresholds_are_checked_and_bad_ones_fall_back_to_the_shipped_values() {
    let p = Thresholds::PROVISIONAL;
    assert!(p.validate().is_ok());
    assert_eq!(Thresholds::default(), p);
    assert_eq!((p.z_drift, p.z_end, p.end_after_s), (3.0, 2.5, 10));
    for bad in [
        Thresholds { z_end: 3.0, ..p },
        Thresholds {
            z_drift: f64::NAN,
            ..p
        },
        Thresholds {
            end_after_s: 0,
            ..p
        },
        Thresholds {
            match_shift: STEP_SHIFT,
            ..p
        },
        Thresholds {
            match_scale: -0.1,
            ..p
        },
        Thresholds {
            turn_tolerance: 0.0,
            ..p
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
        assert_eq!(Detector::with_thresholds(None, bad).thresholds(), p);
    }
    let other = Thresholds { z_drift: 3.5, ..p };
    assert_eq!(Detector::with_thresholds(None, other).thresholds(), other);
    // A higher start threshold ignores a drift the shipped one cues.
    let mut d = Run {
        d: Detector::with_thresholds(
            None,
            Thresholds {
                z_drift: 9.0,
                z_end: 2.5,
                ..p
            },
        ),
        t: Duration::ZERO,
        out: Vec::new(),
    };
    let fx = d.d.capture_started(Duration::ZERO);
    d.out.extend(fx);
    d.calibrate();
    d.feed(120, slumped);
    assert_eq!(d.cues(), 0);
}

// ------------------------------------------------------------------ V2-3 review fixes

#[test]
fn a_cue_held_back_by_frames_without_a_reading_records_when_it_was_asked() {
    use crate::posture::quality::{self, Label, LabelKind, Logged, Source};
    let ms = |d: Duration| i64::try_from(d.as_millis()).unwrap();
    let mut r = calibrated();
    r.sit(30);
    let start = r.t;
    // Two clear seconds slumped, then fifty without a reading, then clear and slumped:
    // the drift began near the start, but the 70%-valid rule holds the cue back.
    r.feed(2, slumped);
    r.feed(50, |_| None);
    let (mut cue_at, mut handed) = (None, None);
    let mut episode = None;
    while episode.is_none() {
        assert!(r.t < start + s(400), "no episode record");
        let in_drift = r.t < start + s(160);
        let f = if in_drift {
            slumped(r.t)
        } else {
            Some(jitter(r.t, base()))
        };
        let fx = r.d.frame(r.t, wall(), 0.9, f);
        if fx.contains(&PostureAction::Cue) {
            cue_at = Some(r.t);
            r.d.cue_outcome(CueOutcome::Shown);
        }
        for a in &fx {
            if let PostureAction::Record(o) = a
                && o.kind == PostureKind::DriftEpisode
            {
                episode = Some(*o);
                handed = Some(r.t);
            }
        }
        r.t += Duration::from_millis(333);
    }
    let (cue_at, o) = (cue_at.unwrap(), episode.unwrap());
    // The detector asked well over a minute and a half after the drift began.
    assert!(cue_at >= start + s(100), "{:?}", cue_at - start);
    o.payload().validate().unwrap();
    let l = Logged::from_observation(ms(handed.unwrap()), &o);
    // The record says when the cue was actually asked (to within half a second) ...
    assert!(
        (l.cue_at_ms() - ms(cue_at)).abs() <= 600,
        "{l:?} vs {cue_at:?}"
    );
    // ... where its start plus 60 s would have been far too early.
    assert!(ms(cue_at) - (l.start_ms + 60_000) >= 30_000, "{l:?}");
    // So the report counts it late, not detected within 30 s.
    let labels = [
        Label {
            kind: LabelKind::Reading,
            start_ms: 0,
            end_ms: ms(start),
        },
        Label {
            kind: LabelKind::DriftForward,
            start_ms: ms(start),
            end_ms: ms(start + s(160)),
        },
    ];
    let rep = quality::session_report(
        Source::Synthetic,
        1,
        1,
        Thresholds::PROVISIONAL,
        &labels,
        &[l],
    );
    assert_eq!(
        (rep.detected, rep.detected_late, rep.true_episodes),
        (0, 1, 1)
    );
    let late = rep.latency_s[0];
    assert!(
        late > 30 && late == (ms(cue_at - start) - 60_000).div_euclid(1000),
        "{late}"
    );
}

#[test]
fn short_no_reading_spells_are_added_up_and_written_together() {
    let mut d = Detector::new(None);
    let mut out = d.capture_started(s(0));
    // Twenty-four 3 s spells, alternating reasons, 2 s apart: 2 minutes in all.
    for i in 0..24_u64 {
        let reason = if i % 2 == 0 {
            PostureReason::Absent
        } else {
            PostureReason::LowConfidence
        };
        out.extend(d.reading_changed(s(5 * i), Some(reason)));
        out.extend(d.reading_changed(s(5 * i + 3), None));
    }
    out.extend(d.capture_ended(s(125)));
    let nr: Vec<Observation> = out
        .iter()
        .filter_map(|a| match a {
            PostureAction::Record(o) if o.kind == PostureKind::NoReading => Some(*o),
            _ => None,
        })
        .collect();
    // Written about once a minute, never one event a spell; every second is counted.
    assert!((2..=3).contains(&nr.len()), "{nr:?}");
    assert_eq!(nr.iter().map(|o| o.duration_s).sum::<u32>(), 72);
    assert_eq!(nr.iter().filter_map(|o| o.spells).sum::<u32>(), 24);
    for o in &nr {
        o.payload().validate().unwrap();
    }
    // Each ends when its last spell ended, and spans no more than BRIEF_EVERY.
    let first = nr[0];
    assert_eq!(first.spells, Some(12));
    assert_eq!(first.duration_s, 36);
}
