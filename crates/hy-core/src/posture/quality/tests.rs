//! The quality report and a day's coverage on hand-made records (slice V2-3). Every number
//! is made up; nothing comes from a camera.

use super::*;

fn label(kind: LabelKind, from_s: i64, to_s: i64) -> Label {
    Label {
        kind,
        start_ms: from_s * 1000,
        end_ms: to_s * 1000,
    }
}

fn log(kind: PostureKind, from_s: i64, to_s: i64) -> Logged {
    Logged {
        kind,
        start_ms: from_s * 1000,
        end_ms: to_s * 1000,
        reason: None,
        cue: None,
    }
}

fn why(kind: PostureKind, from_s: i64, to_s: i64, reason: PostureReason) -> Logged {
    Logged {
        reason: Some(reason),
        ..log(kind, from_s, to_s)
    }
}

/// A drift episode that began at `from_s` (so its cue was asked 60 s later).
fn episode(from_s: i64, to_s: i64, cue: CueOutcome) -> Logged {
    Logged {
        cue: Some(cue),
        ..log(PostureKind::DriftEpisode, from_s, to_s)
    }
}

// ------------------------------------------------------------------ labels

#[test]
fn labels_are_read_from_seconds_with_a_header() {
    let text = "\u{feff}start_s,end_s,label\n0,30,calibrate\n30.5, 270 ,reading\n\n270,360,drift_forward\n";
    let l = parse_labels(text).unwrap();
    assert_eq!(
        l,
        vec![
            label(LabelKind::Calibrate, 0, 30),
            Label {
                kind: LabelKind::Reading,
                start_ms: 30_500,
                end_ms: 270_000
            },
            label(LabelKind::DriftForward, 270, 360),
        ]
    );
    for k in LabelKind::ALL {
        assert_eq!(k.as_str().parse::<LabelKind>().unwrap(), k);
    }
}

#[test]
fn bad_label_lines_are_refused() {
    for bad in [
        "0,30,slouching",
        "30,30,reading",
        "40,30,reading",
        "-1,30,reading",
        "0,30,reading,extra",
        "0,30",
        "a,30,reading",
        "0,90000,reading",
        "NaN,30,reading",
    ] {
        assert!(parse_labels(bad).is_err(), "{bad}");
    }
}

// ------------------------------------------------------------------ intervals

#[test]
fn interval_arithmetic() {
    assert_eq!(
        union(vec![(5, 7), (0, 2), (1, 3), (7, 9), (4, 4)]),
        vec![(0, 3), (5, 9)]
    );
    assert_eq!(
        subtract(&[(0, 10)], &[(2, 3), (5, 12)]),
        vec![(0, 2), (3, 5)]
    );
    assert_eq!(
        subtract(&[(0, 10), (20, 30)], &[(8, 22)]),
        vec![(0, 8), (22, 30)]
    );
    assert_eq!(overlap((0, 10), &[(2, 3), (5, 12)]), 6);
}

// ------------------------------------------------------------------ coverage

#[test]
fn a_days_coverage_adds_up_and_counts_overlaps_once() {
    let logs = [
        log(PostureKind::Capture, 0, 100),
        why(PostureKind::NoReading, 10, 20, PostureReason::Absent),
        // Overlaps the no-reading interval by 5 s: counted once.
        why(
            PostureKind::BaselineSuspect,
            15,
            30,
            PostureReason::CameraRestart,
        ),
        // Outside camera-on time: not counted.
        why(PostureKind::NoReading, 200, 260, PostureReason::Stalled),
        episode(40, 110, CueOutcome::Shown),
        episode(120, 190, CueOutcome::Cooldown),
    ];
    let c = coverage(&logs);
    assert_eq!(c.camera_on_s, 100);
    assert_eq!(c.no_reading_s, 10);
    assert_eq!(c.unscored_s, 10);
    assert_eq!(c.usable_s, 80);
    assert_eq!(c.camera_on_s, c.no_reading_s + c.unscored_s + c.usable_s);
    assert!((c.usable_share.unwrap() - 0.8).abs() < 1e-9);
    assert_eq!((c.episodes, c.cues_shown), (2, 1));
    // No camera-on time (records from before V2-3): no share, never a zero.
    let old = coverage(&[why(PostureKind::NoReading, 0, 10, PostureReason::Absent)]);
    assert_eq!(old.camera_on_s, 0);
    assert_eq!(old.usable_share, None);
}

#[test]
fn records_from_the_store_become_intervals_ending_at_their_time() {
    let data = |kind, duration_s| {
        let p = EventPayload::PostureObservation {
            kind,
            duration_s,
            reason: None,
            cue: None,
        };
        let v = serde_json::to_value(&p).unwrap();
        v["data"].to_string()
    };
    let rows = vec![
        (
            100_000,
            "posture.observation".to_owned(),
            data(PostureKind::Capture, 90),
        ),
        (
            100_000,
            "water.logged".to_owned(),
            r#"{"amount_ml":250}"#.to_owned(),
        ),
        (
            50_000,
            "posture.observation".to_owned(),
            r#"{"kind":"from_a_newer_build","duration_s":5}"#.to_owned(),
        ),
    ];
    let logs = logged_from_rows(rows);
    assert_eq!(logs, vec![log(PostureKind::Capture, 10, 100)]);
}

#[test]
fn an_observation_handed_over_late_ends_when_it_ended() {
    let o = Observation {
        kind: PostureKind::DriftEpisode,
        duration_s: 75,
        reason: None,
        cue: Some(CueOutcome::Shown),
        ago: std::time::Duration::from_secs(10),
    };
    let l = Logged::from_observation(200_000, &o);
    assert_eq!((l.start_ms, l.end_ms), (115_000, 190_000));
    assert_eq!(l.cue_at_ms(), 175_000);
}

#[test]
fn the_log_is_read_from_the_store_without_changing_it() {
    use crate::clock::ManualClock;
    use crate::schema::{MeasurementStatus, NewEvent};
    use crate::store::Store;
    use chrono::{TimeZone, Utc};

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("hy.db");
    let t0 = Utc.with_ymd_and_hms(2026, 11, 19, 5, 30, 0).unwrap();
    let since = t0.timestamp_millis();
    {
        let clock = ManualClock::new(t0 + chrono::TimeDelta::hours(2));
        let mut store = Store::open(&db, std::sync::Arc::new(clock)).unwrap();
        let mut add = |end_s: i64, kind, duration_s, cue| {
            let e = NewEvent::inferred(
                EventPayload::PostureObservation {
                    kind,
                    duration_s,
                    reason: None,
                    cue,
                },
                MeasurementStatus::Estimated,
            )
            .at(t0 + chrono::TimeDelta::seconds(end_s));
            store.append_event(e).unwrap();
        };
        // Began before the session: clipped to its start.
        add(20, PostureKind::Capture, 60, None);
        add(200, PostureKind::DriftEpisode, 90, Some(CueOutcome::Shown));
        // Ended after the window: not read.
        add(5000, PostureKind::Capture, 60, None);
        // Ended before the window: not read.
        add(-10, PostureKind::Capture, 60, None);
    }
    let logs = read_log(&db, since, since + 3_600_000).unwrap();
    assert_eq!(
        logs,
        vec![
            log(PostureKind::Capture, 0, 20),
            episode(110, 200, CueOutcome::Shown)
        ]
    );
    assert!(read_log(&db, since, since).is_err());
    assert!(matches!(
        read_log(&dir.path().join("none.db"), since, since + 1),
        Err(Error::NotFound(_))
    ));
}

// ------------------------------------------------------------------ one session

fn session() -> (Vec<Label>, Vec<Logged>) {
    let labels = vec![
        label(LabelKind::Calibrate, 0, 30),
        label(LabelKind::Reading, 30, 270),
        label(LabelKind::DriftForward, 270, 360),
        label(LabelKind::Typing, 360, 600),
        label(LabelKind::DriftLateral, 600, 690),
        label(LabelKind::Absent, 690, 735),
        label(LabelKind::Reading, 735, 975),
        label(LabelKind::DriftForward, 975, 1065),
        label(LabelKind::Typing, 1065, 1100),
        label(LabelKind::Occluded, 1100, 1130),
        label(LabelKind::Relocated, 1130, 1290),
    ];
    let logs = vec![
        log(PostureKind::Capture, 0, 1295),
        why(
            PostureKind::BaselineSuspect,
            0,
            14,
            PostureReason::NoCalibration,
        ),
        // The first drift: cue at 335 s, 5 s after its sustained threshold (330 s).
        episode(275, 380, CueOutcome::Shown),
        // A second episode inside the same drift's grace: a duplicate.
        episode(300, 400, CueOutcome::Cooldown),
        // During typing: a false episode with a cue shown.
        episode(450, 560, CueOutcome::Shown),
        // The second drift is missed (nothing logged).
        why(PostureKind::NoReading, 692, 735, PostureReason::Absent),
        // The third drift: cue at 1070 s, 35 s after its threshold (1035 s): late.
        episode(1010, 1080, CueOutcome::Cooldown),
        // The occlusion was shown as no reading for only 10 of 30 s.
        why(
            PostureKind::NoReading,
            1120,
            1130,
            PostureReason::LowConfidence,
        ),
        // The moved camera: paused 21 s after the mark.
        why(
            PostureKind::BaselineSuspect,
            1151,
            1290,
            PostureReason::CameraMoved,
        ),
    ];
    (labels, logs)
}

#[test]
fn a_session_is_counted_as_raw_numbers() {
    let (labels, logs) = session();
    let r = session_report(Source::Desk, 3, 1, Thresholds::PROVISIONAL, &labels, &logs);
    assert_eq!((r.setup, r.session), (3, 1));
    assert_eq!(r.labelled_s, 1290);
    assert_eq!(r.camera_on_s, 1295);
    // Reading 4 + drift 1 + typing 4 + drift 1 + reading 4 + drift 1.
    assert_eq!(r.check_intervals, 15);
    assert_eq!(r.usable_intervals, 15);
    assert_eq!(r.comfortable_s, 240 + 240 + 240 + 35);
    assert_eq!(r.no_reading_s["absent"], 43);
    assert_eq!(r.no_reading_s["low_confidence"], 10);
    assert_eq!(r.unscored_s["no_calibration"], 14);
    assert_eq!(r.unscored_s["camera_moved"], 139);
    assert_eq!(r.staged_drifts, 3);
    assert_eq!(r.detected, 1);
    assert_eq!(r.detected_late, 1);
    assert_eq!(
        (r.missed_unscored, r.missed_no_reading, r.missed_other),
        (0, 0, 1)
    );
    assert_eq!(r.latency_s, vec![5, 35]);
    assert_eq!(r.episodes, 4);
    assert_eq!(r.true_episodes, 2);
    assert_eq!(r.duplicate_episodes, 1);
    assert_eq!(r.false_episodes.get("typing"), Some(&1));
    assert_eq!(r.false_episode_count(), 1);
    assert_eq!(r.cues_shown, 2);
    assert_eq!(r.cues_suppressed.get("cooldown"), Some(&2));
    assert_eq!(r.false_cues, 1);
    assert_eq!((r.absences, r.absences_no_reading), (1, 1));
    assert_eq!((r.occlusions, r.occlusions_no_reading), (1, 0));
    assert_eq!(r.cues_while_unseen, 0);
    assert_eq!((r.relocations, r.relocations_suspended), (1, 1));
    assert_eq!(r.suspend_delay_s, vec![Some(21)]);
    assert_eq!(r.cues_while_moved, 0);
    assert_eq!(r.false_suspect_s, 0);
    let t = r.text();
    assert!(
        t.contains("staged drifts 3: detected 1 in time, 1 late"),
        "{t}"
    );
}

#[test]
fn a_missed_drift_says_why() {
    let labels = vec![
        label(LabelKind::DriftForward, 0, 90),
        label(LabelKind::DriftLateral, 100, 190),
        label(LabelKind::DriftForward, 200, 290),
        label(LabelKind::Relocated, 300, 400),
        label(LabelKind::Typing, 400, 700),
    ];
    let logs = vec![
        log(PostureKind::Capture, 0, 700),
        why(
            PostureKind::BaselineSuspect,
            0,
            60,
            PostureReason::CameraRestart,
        ),
        why(PostureKind::NoReading, 100, 160, PostureReason::Absent),
        // Its cue (at 310 s) falls within the third drift's grace, late for it (threshold
        // 260 s), and while the camera was moved.
        episode(250, 400, CueOutcome::Shown),
        // Cued before the first drift began: not a detection of it.
        episode(-100, 30, CueOutcome::Cooldown),
        // A viewpoint suspicion during typing: a false suspension.
        why(
            PostureKind::BaselineSuspect,
            500,
            530,
            PostureReason::ViewpointChanged,
        ),
    ];
    let r = session_report(Source::Desk, 1, 1, Thresholds::PROVISIONAL, &labels, &logs);
    assert_eq!(
        (
            r.detected,
            r.detected_late,
            r.missed_unscored,
            r.missed_no_reading,
            r.missed_other
        ),
        (0, 1, 1, 1, 0)
    );
    assert_eq!(r.latency_s, vec![50]);
    assert_eq!(r.false_episodes.get("unlabelled"), Some(&1));
    assert_eq!(r.cues_while_moved, 1);
    assert_eq!(r.relocations_suspended, 0);
    assert_eq!(r.suspend_delay_s, vec![None]);
    assert_eq!(r.false_suspect_s, 30);
}

// ------------------------------------------------------------------ the gate

fn passing(setup: u32, session: u32) -> SessionReport {
    let labels: Vec<Label> = (0..5)
        .flat_map(|i| {
            let t = i * 600;
            [
                label(LabelKind::Typing, t, t + 400),
                label(LabelKind::DriftForward, t + 400, t + 490),
                label(LabelKind::Reading, t + 490, t + 600),
            ]
        })
        .chain([
            label(LabelKind::Absent, 3000, 3045),
            label(LabelKind::Relocated, 3045, 3200),
        ])
        .collect();
    let mut logs = vec![
        log(PostureKind::Capture, 0, 3200),
        why(PostureKind::NoReading, 3002, 3045, PostureReason::Absent),
        why(
            PostureKind::BaselineSuspect,
            3066,
            3200,
            PostureReason::CameraMoved,
        ),
    ];
    for i in 0..5 {
        let t = i * 600 + 400;
        logs.push(episode(
            t + 5,
            t + 100,
            if i == 0 {
                CueOutcome::Shown
            } else {
                CueOutcome::Cooldown
            },
        ));
    }
    session_report(
        Source::Synthetic,
        setup,
        session,
        Thresholds::PROVISIONAL,
        &labels,
        &logs,
    )
}

#[test]
fn the_gate_adds_a_setups_sessions_and_checks_each_part() {
    let all: Vec<SessionReport> = (1..=SETUPS)
        .flat_map(|s| (1..=SESSIONS).map(move |n| passing(s, n)))
        .collect();
    let g = gate(&all).unwrap();
    assert!(g.pass, "{}", g.text());
    assert_eq!(g.setups.len(), 5);
    let s = &g.setups[0];
    assert_eq!((s.staged_drifts, s.detected), (10, 10));
    assert_eq!(s.latency_s, vec![5; 10]);
    let t = g.text();
    assert!(t.contains("SYNTHETIC TRACES"), "{t}");
    assert!(t.contains("PROVISIONAL"), "{t}");
    assert!(t.contains("Overall: PASS"), "{t}");

    // One session per setup: too few drifts and sessions.
    let half: Vec<SessionReport> = (1..=SETUPS).map(|s| passing(s, 1)).collect();
    let g = gate(&half).unwrap();
    assert!(!g.pass);
    assert!(g.setups.iter().all(|s| !s.detection_ok && !s.pass));

    // Four setups: incomplete.
    let g = gate(&all[..8]).unwrap();
    assert!(!g.pass);
    assert!(g.text().contains("only 4 of 5 setups"));

    // Two false cues in 85 minutes of comfortable work are allowed; three are not.
    let mut extra = all.clone();
    extra[0].false_cues = 2;
    assert!(gate(&extra).unwrap().setups[0].false_cues_ok);
    extra[0].false_cues = 3;
    let g = gate(&extra).unwrap();
    assert!(!g.setups[0].false_cues_ok && !g.pass);

    // An absence not shown as no reading, or a moved camera never paused, fails.
    let mut miss = all.clone();
    miss[2].absences_no_reading = 0;
    miss[4].relocations_suspended = 0;
    let g = gate(&miss).unwrap();
    assert!(!g.setups[1].unseen_ok && !g.setups[2].relocation_ok);

    // Coverage under 70% fails.
    let mut dim = all;
    dim[6].usable_intervals = 0;
    dim[7].usable_intervals = 10;
    assert!(!gate(&dim).unwrap().setups[3].coverage_ok);
}

#[test]
fn the_gate_refuses_what_cannot_be_added() {
    assert!(gate(&[]).is_err());
    let a = passing(1, 1);
    assert!(gate(&[a.clone(), a.clone()]).is_err());
    let mut desk = passing(1, 2);
    desk.source = Source::Desk;
    assert!(gate(&[a.clone(), desk]).is_err());
    let mut other = passing(1, 2);
    other.thresholds.z_drift = 3.5;
    assert!(gate(&[a, other]).is_err());
}

#[test]
fn reports_round_trip_through_json_and_hold_numbers_only() {
    let r = passing(2, 1);
    let json = serde_json::to_string(&r).unwrap();
    let back: SessionReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
    for w in ["\\\\", "Users", "@"] {
        assert!(!json.contains(w), "{w}");
    }
}

// ------------------------------------------------------------------ the command

#[test]
fn the_command_writes_a_desk_sessions_report_then_the_gate() {
    use crate::clock::ManualClock;
    use crate::schema::{MeasurementStatus, NewEvent};
    use crate::store::Store;
    use chrono::{TimeZone, Utc};
    use std::ffi::OsString;

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("hy.db");
    let t0 = Utc.with_ymd_and_hms(2026, 11, 19, 5, 30, 0).unwrap();
    let since = t0.timestamp_millis();
    {
        let clock = ManualClock::new(t0 + chrono::TimeDelta::hours(2));
        let mut store = Store::open(&db, std::sync::Arc::new(clock)).unwrap();
        for (end_s, kind, duration_s, cue) in [
            (600, PostureKind::Capture, 600, None),
            (200, PostureKind::DriftEpisode, 100, Some(CueOutcome::Shown)),
        ] {
            let e = NewEvent::inferred(
                EventPayload::PostureObservation {
                    kind,
                    duration_s,
                    reason: None,
                    cue,
                },
                MeasurementStatus::Estimated,
            )
            .at(t0 + chrono::TimeDelta::seconds(end_s));
            store.append_event(e).unwrap();
        }
    }
    let labels = dir.path().join("labels.csv");
    std::fs::write(
        &labels,
        "start_s,end_s,label\n0,30,calibrate\n30,90,reading\n90,180,drift_forward\n180,600,typing\n",
    )
    .unwrap();
    let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
    let p = |f: &str| dir.path().join(f).to_string_lossy().into_owned();
    let (s, u) = (since.to_string(), (since + 700_000).to_string());
    let args = a(&[
        "--hy-posture-report",
        "session",
        "--since",
        &s,
        "--until",
        &u,
        "--labels",
        &labels.to_string_lossy(),
        "--setup",
        "2",
        "--session",
        "1",
        "--report",
        &p("s.json"),
        "--text",
        &p("s.txt"),
    ]);
    assert_eq!(command(&args, Some(&db)), Some(0));
    let r: SessionReport =
        serde_json::from_str(&std::fs::read_to_string(p("s.json")).unwrap()).unwrap();
    assert_eq!(r.source, Source::Desk);
    assert_eq!(
        (r.setup, r.session, r.staged_drifts, r.detected),
        (2, 1, 1, 1)
    );
    assert_eq!(r.latency_s, vec![10]);
    assert_eq!(r.thresholds, Thresholds::PROVISIONAL);
    assert!(
        std::fs::read_to_string(p("s.txt"))
            .unwrap()
            .contains("(desk)")
    );

    let gate_args = a(&[
        "--hy-posture-report",
        "gate",
        "--report",
        &p("g.json"),
        &p("s.json"),
        "--text",
        &p("g.txt"),
    ]);
    assert_eq!(command(&gate_args, None), Some(0));
    let g: GateReport =
        serde_json::from_str(&std::fs::read_to_string(p("g.json")).unwrap()).unwrap();
    assert!(!g.pass);
    assert_eq!(g.setups.len(), 1);
    let t = std::fs::read_to_string(p("g.txt")).unwrap();
    assert!(
        t.contains("real desk sessions") && t.contains("only 1 of 5 setups"),
        "{t}"
    );

    // Not this command; bad arguments; a store that is not there.
    assert_eq!(command(&a(&["--hy-soak-stats"]), Some(&db)), None);
    assert_eq!(
        command(&a(&["--hy-posture-report", "session"]), Some(&db)),
        Some(4)
    );
    let mut bad = args.clone();
    bad[9] = OsString::from("6");
    assert_eq!(command(&bad, Some(&db)), Some(4));
    assert_eq!(
        command(
            &a(&["--hy-posture-report", "gate", "--report", &p("x.json")]),
            None
        ),
        Some(4)
    );
    let gone = dir.path().join("none.db");
    assert_eq!(command(&args, Some(&gone)), Some(1));
    assert!(
        std::fs::read_to_string(p("s.json"))
            .unwrap()
            .contains("error")
    );
}
