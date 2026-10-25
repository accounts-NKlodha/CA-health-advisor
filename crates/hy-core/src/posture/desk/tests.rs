//! The synthetic desk sessions (slice V2-3). Every frame is nine made-up numbers.

use super::*;
use crate::posture::PostureKind;
use crate::posture::quality::{SESSIONS, coverage};

#[test]
fn the_shipped_thresholds_pass_the_synthetic_gate_on_unseen_sessions_too() {
    for seeds in [0, CHECK_SEEDS] {
        let (sessions, gate) = all_seeded(Thresholds::PROVISIONAL, seeds);
        assert_eq!(sessions.len(), 10);
        assert_eq!(gate.source, Source::Synthetic);
        assert!(gate.pass, "seeds {seeds}:\n{}", gate.text());
        for s in &gate.setups {
            assert_eq!(s.sessions, SESSIONS);
            assert_eq!(s.staged_drifts, 10);
            assert!(s.comfortable_min >= 45, "{}", s.comfortable_min);
            assert!(s.false_cues <= 1, "setup {}", s.setup);
            let mut l = s.latency_s.clone();
            l.sort_unstable();
            assert!(
                l[0] >= 0 && l[l.len() / 2] <= 15,
                "setup {}: {l:?}",
                s.setup
            );
        }
        assert!(gate.text().contains("SYNTHETIC TRACES"));
    }
}

#[test]
fn v2_2s_end_threshold_misses_drifts_that_follow_a_changed_comfortable_stance() {
    // With z_end 2.0 an episode can outlast its drift while the person settles about two
    // floors from calibration, so the next drift starts no new episode. The sweep that
    // moved z_end to 2.5 (docs/V2_Quality.md) shows it on the check set.
    let old = Thresholds {
        z_end: 2.0,
        ..Thresholds::PROVISIONAL
    };
    let (_, gate) = all_seeded(old, CHECK_SEEDS);
    assert!(!gate.pass, "{}", gate.text());
    let detected: u32 = gate.setups.iter().map(|s| s.detected).sum();
    let (_, now) = all_seeded(Thresholds::PROVISIONAL, CHECK_SEEDS);
    assert!(now.setups.iter().map(|s| s.detected).sum::<u32>() > detected);
}

#[test]
fn a_session_follows_the_protocol_and_is_repeatable() {
    let setup = &SETUPS[0];
    let (labels, logs) = run(setup, 1, Thresholds::PROVISIONAL);
    assert_eq!(
        run(setup, 1, Thresholds::PROVISIONAL),
        (labels.clone(), logs.clone())
    );
    // Back to back from 0, about 38 minutes, each kind of the PRD's list present.
    assert_eq!(labels[0].start_ms, 0);
    assert!(labels.windows(2).all(|w| w[0].end_ms == w[1].start_ms));
    assert_eq!(labels.last().unwrap().end_ms, 2_295_000);
    for k in [
        LabelKind::Calibrate,
        LabelKind::Reading,
        LabelKind::Typing,
        LabelKind::SecondDisplay,
        LabelKind::Reach,
        LabelKind::DriftForward,
        LabelKind::DriftLateral,
        LabelKind::Absent,
        LabelKind::Occluded,
        LabelKind::Relocated,
        LabelKind::Paused,
    ] {
        assert!(labels.iter().any(|l| l.kind == k), "{k:?}");
    }
    assert_eq!(labels.iter().filter(|l| l.kind.drift()).count(), 5);
    // The camera was on except during the lock, and the day's parts add up.
    let c = coverage(&logs);
    assert!((2_200..=2_240).contains(&c.camera_on_s), "{c:?}");
    assert_eq!(c.camera_on_s, c.no_reading_s + c.unscored_s + c.usable_s);
    assert!(c.usable_share.unwrap() > 0.8, "{c:?}");
    // The lock restarted the camera: a camera-restart suspicion, and two segments.
    assert!(logs.iter().any(|l| l.kind == PostureKind::BaselineSuspect
        && l.reason == Some(crate::posture::PostureReason::CameraRestart)));
    assert!(
        logs.iter()
            .filter(|l| l.kind == PostureKind::Capture)
            .count()
            >= 2
    );
}

#[test]
fn the_cue_time_derived_from_a_record_is_within_a_frame_or_two_of_the_real_one() {
    // The report finds each cue from its episode's record (start + 60 s); check that
    // against when the machine actually asked, at 3 fps and at 1 fps.
    for setup in [&SETUPS[0], &SETUPS[3]] {
        let (_, logs, cues) = play(setup, 2, 0, Thresholds::PROVISIONAL);
        let derived: Vec<i64> = logs
            .iter()
            .filter(|l| l.kind == PostureKind::DriftEpisode)
            .map(Logged::cue_at_ms)
            .collect();
        assert_eq!(derived.len(), cues.len(), "setup {}", setup.id);
        assert!(!cues.is_empty());
        for (d, c) in derived.iter().zip(&cues) {
            // Whole seconds in the record, plus at most one frame.
            assert!((c - d).abs() <= 2_000, "setup {}: {d} vs {c}", setup.id);
        }
    }
}

#[test]
fn the_grids_hold_only_valid_thresholds_around_the_shipped_ones() {
    let d = drift_grid();
    let v = viewpoint_grid();
    assert!(d.contains(&Thresholds::PROVISIONAL));
    assert!(v.contains(&Thresholds::PROVISIONAL));
    assert!(d.iter().chain(&v).all(|t| t.validate().is_ok()));
}
