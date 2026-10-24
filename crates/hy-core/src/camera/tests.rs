//! Camera session rules on synthetic times (architecture 11: T-C5 and the V2-1 states).

use super::*;

fn s(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_793_000_000, 0).unwrap()
}

fn started() -> Started {
    Started {
        width: 640,
        height: 480,
        cores: 4,
        model_ms: 480.0,
    }
}

fn person() -> FrameReport {
    FrameReport {
        valid: true,
        issue: None,
        inference_ms: 37.0,
        visibility: 0.9,
    }
}

fn nobody() -> FrameReport {
    FrameReport {
        valid: false,
        issue: Some(FrameIssue::Absent),
        inference_ms: 30.0,
        visibility: 0.0,
    }
}

/// A machine with consent, turned on and capturing since `t`.
fn capturing(t: Duration) -> Machine {
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    let fx = m.turn_on(t).unwrap();
    assert!(fx.contains(&Action::OpenPage), "{fx:?}");
    let fx = m.page_started(t, started());
    assert!(fx.contains(&Action::SetFps(FPS_HIGH)), "{fx:?}");
    m
}

/// Feed frames every `step` from `from` to `to` (exclusive).
fn feed(m: &mut Machine, from: Duration, to: Duration, step: Duration, f: FrameReport) {
    let mut t = from;
    while t < to {
        m.frame(t, f).unwrap();
        m.tick(t);
        t += step;
    }
}

#[test]
fn off_by_default_and_never_starts_without_consent() {
    let mut m = Machine::new(CameraSettings::default());
    assert_eq!(m.status(), Status::Off);
    assert!(m.resume(s(0)).is_empty());
    assert!(m.turn_on(s(1)).is_err());
    assert_eq!(m.status(), Status::Off);
    assert!(!m.page_open());
    assert_eq!(m.consent(at()), vec![Action::Persist]);
    let fx = m.turn_on(s(2)).unwrap();
    assert_eq!(fx[0], Action::Persist);
    assert!(fx.contains(&Action::OpenPage));
    assert_eq!(m.status(), Status::Starting);
    assert!(m.settings().on && m.settings().consent_current());
}

#[test]
fn a_consent_for_an_older_notice_asks_again() {
    let old = CameraSettings {
        consent: Some(Consent {
            notice_version: NOTICE_VERSION - 1,
            given_at: at(),
        }),
        on: true,
    };
    let mut m = Machine::new(old);
    assert_eq!(m.resume(s(0)), vec![Action::Persist]);
    assert!(!m.settings().on);
    assert_eq!(m.status(), Status::Off);
    assert!(m.turn_on(s(1)).is_err());
}

#[test]
fn the_choice_to_stay_on_starts_the_camera_after_a_restart_unless_held() {
    let on = CameraSettings {
        consent: Some(Consent {
            notice_version: NOTICE_VERSION,
            given_at: at(),
        }),
        on: true,
    };
    let mut m = Machine::new(on);
    assert!(m.resume(s(0)).contains(&Action::OpenPage));
    let mut m = Machine::new(on);
    // The engine's holds may arrive before the app resumes the camera.
    assert!(
        m.set_hold(s(0), Some(CameraHold::OffHours))
            .iter()
            .all(|a| *a != Action::OpenPage)
    );
    let fx = m.resume(s(0));
    assert!(!fx.contains(&Action::OpenPage));
    assert_eq!(m.status(), Status::Held(CameraHold::OffHours));
    // The hold ends: capture starts.
    assert!(m.set_hold(s(5), None).contains(&Action::OpenPage));
    assert_eq!(m.status(), Status::Starting);
}

#[test]
fn usable_frames_read_and_unusable_ones_give_no_reading_with_the_reason() {
    let mut m = capturing(s(0));
    assert_eq!(m.status(), Status::NoReading(NoReading::Starting));
    feed(&mut m, s(0), s(4), ms(333), person());
    assert_eq!(m.status(), Status::Reading);
    // Nobody in view for a few seconds: no reading, never a score.
    feed(&mut m, s(4), s(9), ms(333), nobody());
    assert_eq!(
        m.status(),
        Status::NoReading(NoReading::Frame(FrameIssue::Absent))
    );
    let low = FrameReport {
        valid: false,
        issue: Some(FrameIssue::LowConfidence),
        inference_ms: 40.0,
        visibility: 0.3,
    };
    feed(&mut m, s(9), s(14), ms(333), low);
    assert_eq!(
        m.status(),
        Status::NoReading(NoReading::Frame(FrameIssue::LowConfidence))
    );
    // One odd frame in a run of usable ones does not flip the status.
    feed(&mut m, s(14), s(18), ms(333), person());
    m.frame(s(18), nobody()).unwrap();
    assert_eq!(m.status(), Status::Reading);
    let st = m.stats();
    assert!(st.frames > 40 && st.valid_frames < st.frames);
}

#[test]
fn frame_reports_are_checked() {
    let mut m = capturing(s(0));
    for bad in [
        FrameReport {
            inference_ms: f64::NAN,
            ..person()
        },
        FrameReport {
            inference_ms: -1.0,
            ..person()
        },
        FrameReport {
            visibility: 1.5,
            ..person()
        },
        FrameReport {
            issue: Some(FrameIssue::Absent),
            ..person()
        },
        FrameReport {
            issue: None,
            ..nobody()
        },
    ] {
        assert!(m.frame(s(1), bad).is_err(), "{bad:?}");
    }
    assert_eq!(m.stats().frames, 0);
    // Unknown fields from a page are refused.
    let text = r#"{"valid":true,"issue":null,"inference_ms":3,"visibility":1,"landmarks":[]}"#;
    assert!(serde_json::from_str::<FrameReport>(text).is_err());
}

#[test]
fn t_c5_a_stall_gives_no_reading_and_one_worker_restart_then_recovers() {
    let mut m = capturing(s(0));
    feed(&mut m, s(0), s(10), ms(333), person());
    assert_eq!(m.status(), Status::Reading);
    // The worker stops answering at 10 s: nothing before 5 s have passed...
    assert!(!m.tick(ms(14_900)).contains(&Action::RestartWorker));
    // ...then no reading (stalled) and one restart, with nothing fabricated.
    let fx = m.tick(s(15));
    assert!(fx.contains(&Action::RestartWorker), "{fx:?}");
    assert_eq!(m.status(), Status::NoReading(NoReading::Stalled));
    assert!(m.tick(s(16)).is_empty());
    assert_eq!(m.status(), Status::NoReading(NoReading::Stalled));
    // The restarted worker answers: reading again.
    feed(&mut m, s(17), s(21), ms(333), person());
    assert_eq!(m.status(), Status::Reading);
    let st = m.stats();
    assert_eq!((st.stalls, st.worker_restarts), (1, 1));
    assert!(m.settings().on);
}

#[test]
fn t_c5_a_second_stall_within_ten_minutes_stops_the_camera_and_tells_the_user() {
    let mut m = capturing(s(0));
    feed(&mut m, s(0), s(10), ms(333), person());
    assert!(m.tick(s(15)).contains(&Action::RestartWorker));
    feed(&mut m, s(16), s(300), ms(333), person());
    // Second stall 5 s after the last result, well within ten minutes of the first.
    let fx = m.tick(ms(305_000));
    assert!(!fx.contains(&Action::RestartWorker), "{fx:?}");
    assert!(fx.contains(&Action::RequestStop), "{fx:?}");
    assert!(fx.contains(&Action::Notify(CameraNotice::StoppedAfterStalls)));
    assert!(fx.contains(&Action::Persist));
    assert!(!m.settings().on);
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Stalled));
    assert!(!m.wants_capture());
    // A restarted worker that never answers stalls again 5 s later and stops it too.
    let mut m = capturing(s(0));
    feed(&mut m, s(0), s(3), ms(333), person());
    assert!(m.tick(s(8)).contains(&Action::RestartWorker));
    assert!(m.tick(s(13)).contains(&Action::RequestStop));
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Stalled));
}

#[test]
fn stalls_more_than_ten_minutes_apart_each_get_a_restart() {
    let mut m = capturing(s(0));
    feed(&mut m, s(0), s(10), ms(500), person());
    assert!(m.tick(s(15)).contains(&Action::RestartWorker));
    feed(&mut m, s(16), s(700), ms(500), person());
    assert!(m.tick(s(705)).contains(&Action::RestartWorker));
    assert!(m.settings().on);
    assert_eq!(m.stats().worker_restarts, 2);
}

#[test]
fn off_releases_within_two_seconds_even_if_the_page_hangs() {
    // The page confirms: closed at once, latency recorded.
    let mut m = capturing(s(0));
    feed(&mut m, s(0), s(5), ms(333), person());
    let fx = m.turn_off(s(10));
    assert!(fx.contains(&Action::RequestStop), "{fx:?}");
    assert!(fx.contains(&Action::Notify(CameraNotice::Off)));
    assert_eq!(m.status(), Status::Off);
    assert!(!m.wants_capture());
    // Late frames from the stopping page change nothing.
    assert!(m.frame(ms(10_100), person()).unwrap().is_empty());
    assert_eq!(m.page_stopped(ms(10_350)), vec![Action::ClosePage]);
    assert_eq!(m.stats().last_stop_ms, Some(350));
    assert!(!m.stats().last_stop_forced);
    assert!(!m.page_open());
    // The page never confirms: destroyed after CLOSE_AFTER, inside STOP_WITHIN.
    let mut m = capturing(s(0));
    m.turn_off(s(10));
    assert!(!m.tick(ms(10_900)).contains(&Action::ClosePage));
    assert!(m.tick(s(11)).contains(&Action::ClosePage));
    let st = m.stats();
    assert!(st.last_stop_forced);
    assert!(Duration::from_millis(st.last_stop_ms.unwrap()) < STOP_WITHIN);
    // A confirmation after the forced close is ignored.
    assert!(m.page_stopped(s(12)).is_empty());
}

#[test]
fn holds_release_capture_and_capture_starts_again_after() {
    for hold in [
        CameraHold::Away,
        CameraHold::Paused,
        CameraHold::OffHours,
        CameraHold::Presentation,
        CameraHold::Meeting,
    ] {
        let mut m = capturing(s(0));
        feed(&mut m, s(0), s(3), ms(333), person());
        let fx = m.set_hold(s(3), Some(hold));
        assert!(fx.contains(&Action::RequestStop), "{hold:?} {fx:?}");
        assert_eq!(m.status(), Status::Held(hold));
        assert!(m.settings().on, "a hold keeps the user's choice");
        assert_eq!(m.page_stopped(ms(3_200)), vec![Action::ClosePage]);
        // No watchdog during a hold.
        assert!(m.tick(s(60)).is_empty());
        let fx = m.set_hold(s(61), None);
        assert!(fx.contains(&Action::OpenPage), "{hold:?} {fx:?}");
        assert_eq!(m.status(), Status::Starting);
    }
    // A hold that changes reason keeps capture released.
    let mut m = capturing(s(0));
    m.set_hold(s(1), Some(CameraHold::Meeting));
    m.page_stopped(s(1));
    let fx = m.set_hold(s(2), Some(CameraHold::Away));
    assert!(!fx.contains(&Action::OpenPage), "{fx:?}");
    assert!(!m.page_open());
    assert_eq!(m.status(), Status::Held(CameraHold::Away));
    // Holds do nothing while the camera is off.
    let mut m = Machine::new(CameraSettings::default());
    assert!(m.set_hold(s(0), Some(CameraHold::Meeting)).is_empty());
    assert_eq!(m.status(), Status::Off);
}

#[test]
fn focus_is_not_a_camera_hold_and_the_strongest_hold_wins() {
    let none = HoldSources::default();
    assert_eq!(none.camera_hold(), None);
    assert_eq!(
        HoldSources {
            focus: true,
            ..none
        }
        .camera_hold(),
        None
    );
    let all = HoldSources {
        away: true,
        paused: true,
        off_hours: true,
        focus: true,
        presenting: true,
        meeting: true,
    };
    assert_eq!(all.camera_hold(), Some(CameraHold::Away));
    assert_eq!(
        HoldSources { away: false, ..all }.camera_hold(),
        Some(CameraHold::Paused)
    );
    assert_eq!(
        HoldSources {
            presenting: true,
            meeting: true,
            ..none
        }
        .camera_hold(),
        Some(CameraHold::Presentation)
    );
    assert_eq!(
        HoldSources {
            meeting: true,
            ..none
        }
        .camera_hold(),
        Some(CameraHold::Meeting)
    );
}

#[test]
fn conflicts_are_named_and_a_busy_or_missing_camera_is_tried_again() {
    assert_eq!(
        Unavailable::from_error_name("NotAllowedError"),
        Unavailable::Denied
    );
    assert_eq!(
        Unavailable::from_error_name("NotReadableError"),
        Unavailable::Busy
    );
    assert_eq!(
        Unavailable::from_error_name("NotFoundError"),
        Unavailable::NoCamera
    );
    assert_eq!(
        Unavailable::from_error_name("OverconstrainedError"),
        Unavailable::NoCamera
    );
    assert_eq!(
        Unavailable::from_error_name("SomethingElse"),
        Unavailable::Failed
    );

    // Denied: capture released, the user is told, nothing retried.
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    m.turn_on(s(0)).unwrap();
    let fx = m.page_error(s(1), "NotAllowedError", false);
    assert!(fx.contains(&Action::RequestStop));
    assert!(fx.contains(&Action::Notify(CameraNotice::Unavailable(
        Unavailable::Denied
    ))));
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Denied));
    assert_eq!(m.status().words(), "Camera blocked by Windows");
    m.page_stopped(s(1));
    assert!(m.tick(s(300)).is_empty());
    // "Try again" starts it once more.
    assert!(m.try_again(s(301)).contains(&Action::OpenPage));
    assert_eq!(m.status(), Status::Starting);

    // Busy: the page stays open and tries again every minute; then it works.
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    m.turn_on(s(0)).unwrap();
    let fx = m.page_error(s(1), "NotReadableError", false);
    assert!(!fx.contains(&Action::RequestStop), "{fx:?}");
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Busy));
    assert!(m.tick(s(60)).is_empty());
    assert!(m.tick(s(61)).contains(&Action::Retry));
    assert_eq!(m.status(), Status::Starting);
    m.page_started(s(62), started());
    assert!(m.status().capturing());
    // A hold during a busy wait releases the page; no retries meanwhile.
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    m.turn_on(s(0)).unwrap();
    m.page_error(s(1), "NotFoundError", false);
    let fx = m.set_hold(s(2), Some(CameraHold::Meeting));
    assert!(fx.contains(&Action::RequestStop));
    m.page_stopped(s(2));
    assert!(m.tick(s(120)).is_empty());
    assert!(m.set_hold(s(121), None).contains(&Action::OpenPage));

    // The model fails to load.
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    m.turn_on(s(0)).unwrap();
    m.page_error(s(1), "TypeError", true);
    assert_eq!(m.status(), Status::Unavailable(Unavailable::ModelFailed));
}

#[test]
fn a_page_that_never_starts_or_disappears_is_reported() {
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    m.turn_on(s(0)).unwrap();
    assert!(m.tick(s(29)).is_empty());
    let fx = m.tick(s(30));
    assert!(fx.contains(&Action::RequestStop));
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Failed));

    let mut m = capturing(s(0));
    let fx = m.page_gone(s(5));
    assert!(fx.contains(&Action::Notify(CameraNotice::Unavailable(
        Unavailable::Failed
    ))));
    assert!(!m.page_open());
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Failed));
    // Gone while stopping is the expected end.
    let mut m = capturing(s(0));
    m.turn_off(s(1));
    assert!(m.page_gone(s(1)).is_empty());
    assert_eq!(m.status(), Status::Off);
    // The user closes the page of a busy camera: the retry goes on and reopens it.
    let mut m = capturing(s(0));
    m.page_error(s(1), "NotReadableError", false);
    assert!(m.page_gone(s(2)).is_empty());
    assert_eq!(m.status(), Status::Unavailable(Unavailable::Busy));
    let fx = m.tick(s(1) + RETRY_UNAVAILABLE_EVERY);
    assert!(fx.contains(&Action::OpenPage));
    assert_eq!(m.status(), Status::Starting);
}

#[test]
fn a_late_start_after_off_is_released() {
    let mut m = Machine::new(CameraSettings::default());
    m.consent(at());
    m.turn_on(s(0)).unwrap();
    m.turn_off(s(1));
    m.page_stopped(s(1));
    // The page reported started anyway (it was reopened by mistake): it is told to stop.
    let fx = m.page_started(s(2), started());
    assert!(!fx.contains(&Action::SetFps(FPS_HIGH)));
    assert_eq!(m.status(), Status::Off);
}

#[test]
fn withdrawing_consent_turns_the_camera_off_and_asks_again() {
    let mut m = capturing(s(0));
    let fx = m.withdraw_consent(s(1));
    assert!(fx.contains(&Action::RequestStop));
    assert!(fx.contains(&Action::Persist));
    assert!(m.settings().consent.is_none());
    assert!(m.turn_on(s(2)).is_err());
}

#[test]
fn the_rate_drops_to_one_fps_over_budget_and_returns_when_cheap() {
    // 4 cores at 3 fps: 6% of the PC is 80 ms per frame.
    assert!((inference_share(80.0, 3, 4) - 0.06).abs() < 1e-9);
    let mut g = FpsGovernor::default();
    assert_eq!(g.fps(), FPS_HIGH);
    // A slow first frame (warm-up) alone changes nothing.
    assert_eq!(g.record(s(0), 900.0, 4), None);
    for i in 1..9 {
        assert_eq!(g.record(s(i), 30.0, 4), None);
    }
    // Batch 1 speed on 8 logical processors stays at 3 fps.
    let mut g = FpsGovernor::default();
    for i in 0..30 {
        assert_eq!(g.record(s(i), 37.0, 8), None);
    }
    // A slower PC: 100 ms on 4 processors is 7.5%, so 1 fps.
    let mut g = FpsGovernor::default();
    let mut changed = None;
    for i in 0..10 {
        changed = changed.or(g.record(s(i), 100.0, 4));
    }
    assert_eq!(changed, Some(FPS_LOW));
    assert_eq!(g.fps(), FPS_LOW);
    // It returns only after a minute of cheap frames.
    let mut back = None;
    let mut t = 10;
    while back.is_none() && t < 200 {
        back = g.record(s(t), 40.0, 4);
        t += 1;
    }
    assert_eq!(back, Some(FPS_HIGH));
    assert!(t >= 10 + 10 + 60, "returned at {t} s");
    // One that cannot keep up at all.
    let mut g = FpsGovernor::default();
    let mut changed = None;
    for i in 0..10 {
        changed = changed.or(g.record(s(i), 300.0, 64));
    }
    assert_eq!(changed, Some(FPS_LOW));
}

#[test]
fn the_machine_tells_the_page_its_frame_rate() {
    let mut m = capturing(s(0));
    let mut sets = Vec::new();
    let mut t = s(0);
    for _ in 0..12 {
        let slow = FrameReport {
            inference_ms: 200.0,
            ..person()
        };
        for a in m.frame(t, slow).unwrap() {
            if let Action::SetFps(f) = a {
                sets.push(f);
            }
        }
        t += s(1);
    }
    assert_eq!(sets, vec![FPS_LOW]);
    assert_eq!(m.fps(), FPS_LOW);
    assert_eq!(m.stats().fps, FPS_LOW);
}

#[test]
fn status_words_and_badge() {
    assert!(!Status::Off.capturing());
    assert!(Status::Starting.capturing());
    assert!(Status::Reading.capturing());
    assert!(Status::NoReading(NoReading::Stalled).capturing());
    assert!(!Status::Held(CameraHold::Meeting).capturing());
    assert!(!Status::Unavailable(Unavailable::Busy).capturing());
    assert_eq!(Status::Reading.words(), "Camera on");
    assert_eq!(
        Status::NoReading(NoReading::Frame(FrameIssue::Absent)).words(),
        "Cam: no reading"
    );
    assert_eq!(
        Status::Unavailable(Unavailable::Busy).words(),
        "Camera in use by another app"
    );
    let json =
        serde_json::to_value(Status::NoReading(NoReading::Frame(FrameIssue::OutOfFrame))).unwrap();
    assert_eq!(json["kind"], "no_reading");
    assert_eq!(json["detail"]["frame"], "out_of_frame");
    let json = serde_json::to_value(Status::Held(CameraHold::Presentation)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "held", "detail": "presentation"})
    );
}

#[test]
fn settings_store_numbers_and_choices_only() {
    let s = CameraSettings {
        consent: Some(Consent {
            notice_version: 1,
            given_at: at(),
        }),
        on: true,
    };
    let v = serde_json::to_value(s).unwrap();
    assert_eq!(v["on"], true);
    assert_eq!(v["consent"]["notice_version"], 1);
    assert_eq!(serde_json::from_value::<CameraSettings>(v).unwrap(), s);
    assert!(
        serde_json::from_str::<CameraSettings>(r#"{"consent":null,"on":false,"frames":[]}"#)
            .is_err()
    );
    assert!(!CameraSettings::default().on);
}
