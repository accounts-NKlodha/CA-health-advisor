//! The self-test of the posture check's camera (slice V2-1), on the Windows CI runner.
//!
//! The runner has no webcam: every window of the self-test run gets Chromium's synthetic
//! camera (`windows::SELFTEST_CAMERA_ARG`, a generated test pattern with no person in it),
//! so the real camera page, the real pose model and the real watchdog run end to end, and
//! the frames correctly give "no reading" (no one in view). What needs a real webcam and
//! a person (desk sessions, a second webcam, a 2-hour soak) is on the physical-testing
//! list in `docs/V2_Camera.md`.
//!
//! Steps: the consent page opens without capture; consent starts the camera and the model;
//! frames are read at the set rate and give "no reading"; the CAM badge shows; the worker
//! cannot reach the network; a hidden camera window keeps reading; process-tree memory and
//! CPU are measured against the camera budget; a frozen worker is restarted once and a
//! second freeze stops the camera; a presentation hold releases the camera and its end
//! restarts it; Off releases the camera within two seconds and removes the badge; and a
//! camera refused by the system gives "blocked" while the pet keeps running, without the
//! refusal being remembered.
//!
//! Slice V2-2 adds: a calibration on the synthetic camera (no one in view) is "not
//! usable" with its reason; then, with the worker frozen, synthetic feature reports (made
//! up numbers, never an image) run a calibration, a sustained drift that asks the engine
//! for the cue and records one episode with its outcome, and a moved camera that
//! suspends cues.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hy_core::camera::{CameraHold, FrameIssue, FrameReport, NoReading, Status, Unavailable};
use hy_core::posture::{
    CalibrationIssue, Features, HeadRef, PostureKind, PostureReason, PostureState,
};
use tauri::Manager;

use super::{Report, on_main, pet_visible, wait_until};
use crate::shell::{CAMERA, PageCommand, Shell};
use crate::view::CameraPage;

/// The camera budget (architecture 4.5): at most this share of the PC's CPU...
const CPU_BUDGET_PCT: f64 = 10.0;
/// ...and at most this much memory for the camera page and its worker, MB.
const MEMORY_BUDGET_MB: f64 = 350.0;

fn page(shell: &Arc<Shell>) -> Option<CameraPage> {
    on_main(shell, |s| s.camera_page())
}

fn status(shell: &Arc<Shell>) -> Option<Status> {
    page(shell).map(|p| p.status)
}

fn frames(shell: &Arc<Shell>) -> u64 {
    page(shell).map_or(0, |p| p.stats.frames)
}

fn camera_window(shell: &Arc<Shell>) -> bool {
    on_main(shell, |s| s.app().get_webview_window(CAMERA).is_some()) == Some(true)
}

fn absent(s: Option<Status>) -> bool {
    s == Some(Status::NoReading(NoReading::Frame(FrameIssue::Absent)))
}

/// Frames read over `secs` seconds, as a rate.
fn rate(shell: &Arc<Shell>, secs: u64) -> (u64, f64) {
    let f0 = frames(shell);
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(secs));
    let n = frames(shell).saturating_sub(f0);
    #[allow(clippy::cast_precision_loss)]
    let r = n as f64 / t0.elapsed().as_secs_f64();
    (n, r)
}

/// Wait for capture to read frames again (after a start or restart).
fn reading_again(shell: &Arc<Shell>, timeout: Duration) -> bool {
    let f0 = frames(shell);
    wait_until(timeout, || {
        frames(shell) >= f0 + 3 && page(shell).is_some_and(|p| p.status.capturing())
    })
}

/// The process tree's use, sampled twice `secs` apart: (working set MB, private MB, CPU %
/// of the whole PC).
#[cfg(windows)]
fn usage(secs: u64) -> (f64, f64, f64) {
    let a = crate::win32::process_tree_usage();
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(secs));
    let b = crate::win32::process_tree_usage();
    let wall = t0.elapsed().as_secs_f64();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    #[allow(clippy::cast_precision_loss)]
    let mb = |v: u64| v as f64 / (1024.0 * 1024.0);
    let cpu = b.cpu.saturating_sub(a.cpu).as_secs_f64()
        / (wall * f64::from(u32::try_from(cores).unwrap_or(1)))
        * 100.0;
    (mb(b.working_set), mb(b.private_bytes), cpu)
}

#[cfg(not(windows))]
fn usage(secs: u64) -> (f64, f64, f64) {
    std::thread::sleep(Duration::from_secs(secs));
    (0.0, 0.0, 0.0)
}

#[allow(clippy::too_many_lines)]
pub(super) fn camera_end_to_end(r: &mut Report, shell: &Arc<Shell>) {
    // The process tree with the camera off, for the camera's share below.
    let (off_ws, off_priv, off_cpu) = usage(5);

    // 1. Posture check opens the consent page; nothing captures before consent.
    let _ = on_main(shell, |s| s.camera_open());
    let opened = wait_until(Duration::from_secs(15), || camera_window(shell));
    let before = page(shell);
    let refused = on_main(shell, |s| !s.camera_permission(CAMERA)) == Some(true);
    r.check(
        "camera_consent_first",
        opened && before.as_ref().is_some_and(|p| !p.consented && !p.on && !p.capture) && refused,
        format!(
            "camera page open {opened}; consent recorded {:?}; camera permission refused before consent {refused}",
            before.as_ref().map(|p| p.consented)
        ),
    );

    // 2. "Turn on camera": the camera and the model start.
    let consent = on_main(shell, |s| s.camera_consent(true).is_ok()) == Some(true);
    let started = wait_until(Duration::from_secs(60), || {
        page(shell).is_some_and(|p| p.stats.model_ms.is_some() && p.stats.frames > 0)
    });
    let p = page(shell);
    let facts = on_main(shell, |s| s.camera_test_facts());
    let guarded = facts.as_ref().is_some_and(|f| f.guarded);
    r.check(
        "camera_started",
        consent && started && guarded,
        format!(
            "consent {consent}; permission guard installed {guarded}; started {started}; model load {:?} ms; capture path {:?}; status {:?}",
            p.as_ref().and_then(|p| p.stats.model_ms).map(f64::round),
            facts.as_ref().and_then(|f| f.path),
            p.as_ref().map(|p| p.words),
        ),
    );
    if !started {
        // Nothing below can pass; turn it off and stop here.
        let _ = on_main(shell, |s| s.camera_off());
        return;
    }

    // 3. Frames at the set rate; no person in the synthetic picture gives "no reading";
    //    the pet shows the CAM badge.
    let (n, fps) = rate(shell, 10);
    let set_fps = page(shell).map_or(0, |p| p.fps);
    let st = status(shell);
    r.check(
        "camera_frames_read",
        n >= 8,
        format!("{n} frames in 10 s ({fps:.1} a second; set rate {set_fps})"),
    );
    r.check(
        "camera_no_person_no_reading",
        absent(st),
        format!("status {st:?} (the synthetic camera shows no person)"),
    );
    let inference = page(shell).and_then(|p| p.stats.inference_ms);
    r.info(
        "camera_inference_ms",
        inference.is_some_and(|ms| ms <= 250.0),
        format!("mean inference {inference:?} ms a frame (batch 1 measured 37 ms)"),
    );
    let badge = on_main(shell, |s| s.current_pet_view().camera).flatten();
    r.check(
        "camera_badge_shown",
        badge.is_some(),
        format!("pet badge {badge:?}"),
    );

    // 4. The worker cannot reach the network (the content policy refuses it).
    let _ = on_main(shell, |s| s.camera_test_command(PageCommand::Probe));
    let probed = wait_until(Duration::from_secs(10), || {
        on_main(shell, |s| s.camera_test_facts().probe.is_some()) == Some(true)
    });
    let probe = on_main(shell, |s| s.camera_test_facts().probe).flatten();
    r.check(
        "camera_network_blocked",
        probed && probe.as_ref().is_some_and(|(blocked, _)| *blocked),
        format!("probe {probe:?}"),
    );

    // 5. A hidden camera window keeps reading (the worker owns the timing).
    let _ = on_main(shell, |s| {
        if let Some(w) = s.app().get_webview_window(CAMERA) {
            let _ = w.hide();
        }
    });
    std::thread::sleep(Duration::from_secs(2));
    let (hidden_n, hidden_fps) = rate(shell, 8);
    r.check(
        "camera_hidden_window_keeps_reading",
        hidden_n >= 4,
        format!("{hidden_n} frames in 8 s with the window hidden ({hidden_fps:.1} a second)"),
    );

    // 6. The camera budget: process-tree memory and CPU with the camera on.
    let (on_ws, on_priv, on_cpu) = usage(10);
    let added_ws = on_ws - off_ws;
    let added_cpu = on_cpu - off_cpu;
    r.info(
        "camera_budget_memory",
        added_ws <= MEMORY_BUDGET_MB,
        format!(
            "process tree working set {off_ws:.0} MB off, {on_ws:.0} MB on (camera adds {added_ws:.0} MB; budget {MEMORY_BUDGET_MB:.0} MB); private {off_priv:.0} MB off, {on_priv:.0} MB on"
        ),
    );
    r.info(
        "camera_budget_cpu",
        added_cpu <= CPU_BUDGET_PCT,
        format!(
            "process tree CPU {off_cpu:.1}% off, {on_cpu:.1}% on, of the whole PC (camera adds {added_cpu:.1}%; budget {CPU_BUDGET_PCT:.0}%); {} logical processors",
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
        ),
    );

    // 7. The watchdog: a frozen worker is restarted once and reading resumes; a second
    //    freeze within ten minutes stops the camera and turns the choice off.
    let restarts0 = page(shell).map_or(0, |p| p.stats.worker_restarts);
    let _ = on_main(shell, |s| s.camera_test_command(PageCommand::Freeze));
    let restarted = wait_until(Duration::from_secs(20), || {
        page(shell).is_some_and(|p| p.stats.worker_restarts > restarts0)
    });
    let recovered = restarted && reading_again(shell, Duration::from_secs(30));
    r.check(
        "camera_stall_restarts_worker",
        restarted && recovered,
        format!(
            "worker restarted {restarted}; reading again {recovered}; status {:?}",
            status(shell)
        ),
    );
    // This time the page is on screen and hangs (ignores Stop): an on-screen page that
    // explains a failure is normally kept, but a forced close must destroy it.
    let _ = on_main(shell, |s| s.camera_open());
    let _ = on_main(shell, |s| s.camera_test_command(PageCommand::Freeze));
    let _ = on_main(shell, |s| s.camera_test_command(PageCommand::Hang));
    let stopped = wait_until(Duration::from_secs(20), || {
        page(shell).is_some_and(|p| p.status == Status::Unavailable(Unavailable::Stalled) && !p.on)
    });
    let gone = wait_until(Duration::from_secs(5), || !camera_window(shell));
    let p = page(shell);
    let forced = p.as_ref().is_some_and(|p| p.stats.last_stop_forced);
    r.check(
        "camera_second_stall_stops",
        stopped && gone && forced,
        format!(
            "stopped {stopped}; visible hung camera page destroyed {gone}; forced {forced}; stalls {:?}; stop took {:?} ms",
            p.as_ref().map(|p| p.stats.stalls),
            p.as_ref().and_then(|p| p.stats.last_stop_ms)
        ),
    );

    // 8. Turned on again; a presentation releases the camera and its end restarts it.
    let _ = on_main(shell, |s| s.camera_try_again());
    let again = reading_again(shell, Duration::from_secs(60));
    let _ = on_main(shell, |s| s.camera_set_hold(Some(CameraHold::Presentation)));
    let held = wait_until(Duration::from_secs(10), || {
        status(shell) == Some(Status::Held(CameraHold::Presentation)) && !camera_window(shell)
    });
    let p = page(shell);
    let hold_stop = p.as_ref().and_then(|p| p.stats.last_stop_ms);
    let _ = on_main(shell, |s| s.camera_set_hold(None));
    let resumed = reading_again(shell, Duration::from_secs(60));
    r.check(
        "camera_presentation_releases",
        again && held && hold_stop.is_some_and(|ms| ms <= 2000) && resumed,
        format!(
            "on again {again}; held and page closed {held}; release took {hold_stop:?} ms; restarted after {resumed}"
        ),
    );

    posture(shell, r);

    // 9. Off: capture stops within two seconds and the badge goes at once.
    let _ = on_main(shell, |s| s.camera_off());
    let off = wait_until(Duration::from_secs(5), || {
        status(shell) == Some(Status::Off) && !camera_window(shell)
    });
    let p = page(shell);
    let stop_ms = p.as_ref().and_then(|p| p.stats.last_stop_ms);
    let forced = p.as_ref().is_some_and(|p| p.stats.last_stop_forced);
    let badge = on_main(shell, |s| s.current_pet_view().camera).flatten();
    r.check(
        "camera_off_releases_within_2s",
        off && stop_ms.is_some_and(|ms| ms <= 2000) && !forced && badge.is_none(),
        format!(
            "off and page closed {off}; capture released in {stop_ms:?} ms (page confirmed: {}); badge {badge:?}",
            !forced
        ),
    );
    std::thread::sleep(Duration::from_secs(3));
    let (after_ws, _, _) = usage(2);
    r.info(
        "camera_memory_returned",
        after_ws <= off_ws + 60.0,
        format!(
            "process tree working set {after_ws:.0} MB after off (was {off_ws:.0} MB before on)"
        ),
    );

    // 10. The camera refused (as with Windows camera access off): "blocked", the pet keeps
    //     running; the refusal is not remembered, so allowing it again works.
    let _ = on_main(shell, |s| s.camera_test_deny(true));
    let _ = on_main(shell, |s| s.camera_on());
    let denied = wait_until(Duration::from_secs(30), || {
        status(shell) == Some(Status::Unavailable(Unavailable::Denied))
    });
    let pet_ok = shell.page_ready() && pet_visible(shell) == Some(true);
    r.check(
        "camera_denied_pet_keeps_running",
        denied && pet_ok,
        format!(
            "blocked shown {denied}; pet running and visible {pet_ok}; status {:?}",
            status(shell)
        ),
    );
    let _ = on_main(shell, |s| s.camera_test_deny(false));
    let _ = on_main(shell, |s| s.camera_try_again());
    let allowed = reading_again(shell, Duration::from_secs(60));
    r.check(
        "camera_refusal_not_remembered",
        allowed,
        format!(
            "reading after allowing again {allowed}; status {:?}",
            status(shell)
        ),
    );
    let _ = on_main(shell, |s| s.camera_off());
    let off = wait_until(Duration::from_secs(5), || {
        status(shell) == Some(Status::Off) && !camera_window(shell)
    });
    r.check(
        "camera_off_at_end",
        off,
        format!("status {:?}", status(shell)),
    );
}

/// A synthetic frame report: a person sitting as usual, changed by `change`.
fn synthetic(t: f64, change: impl Fn(Features) -> Features) -> FrameReport {
    let w = |amp: f64, phase: f64| amp * (t * 1.37 + phase).sin();
    let f = Features {
        head: HeadRef::Ears,
        gap: 0.55 + w(0.008, 0.0),
        width: 0.35 * (1.0 + w(0.006, 1.0)),
        tilt: w(0.005, 2.0),
        roll: w(0.8, 3.0),
        mid_x: 0.5 + w(0.004, 4.0),
        mid_y: 0.75 + w(0.004, 5.0),
        span: 0.45 * (1.0 + w(0.01, 6.0)),
        turn: w(0.02, 7.0),
    };
    FrameReport {
        valid: true,
        issue: None,
        inference_ms: 30.0,
        visibility: 0.9,
        features: Some(change(f)),
    }
}

/// Feed `secs` of synthetic frames at 3 a second (the clock moved on with each).
fn feed(shell: &Arc<Shell>, t: &mut f64, secs: u32, change: fn(Features) -> Features) -> bool {
    let step = Duration::from_millis(333);
    for _ in 0..secs * 3 {
        let report = synthetic(*t, change);
        if on_main(shell, move |s| s.camera_test_frame(report, step)) != Some(Ok(())) {
            return false;
        }
        *t += 0.333;
    }
    true
}

fn posture_state(shell: &Arc<Shell>) -> Option<PostureState> {
    on_main(shell, |s| s.camera_test_facts().posture)
}

/// Slice V2-2: calibration, drift, the cue and the viewpoint check.
fn posture(shell: &Arc<Shell>, r: &mut Report) {
    // On the real page and the synthetic camera (no one in view): not usable, with the
    // reason, after at most 30 s at the full frame rate.
    let started = on_main(shell, |s| s.camera_calibrate(true)).is_some_and(|p| p.is_ok());
    let full_rate = page(shell).is_some_and(|p| p.fps == hy_core::camera::FPS_HIGH);
    let refused = wait_until(Duration::from_secs(40), || {
        posture_state(shell) == Some(PostureState::NotUsable(CalibrationIssue::OutOfView))
    });
    r.check(
        "posture_calibration_out_of_view",
        started && full_rate && refused,
        format!(
            "calibration started {started}; full rate {full_rate}; refused as out of view {refused}; state {:?}",
            posture_state(shell)
        ),
    );

    // Synthetic numbers with the worker frozen (only these frames arrive).
    let _ = on_main(shell, |s| s.camera_test_command(PageCommand::Freeze));
    std::thread::sleep(Duration::from_millis(500));
    let mut t = 0.0;
    let _ = on_main(shell, |s| s.camera_calibrate(true));
    let fed = feed(shell, &mut t, 20, |f| f);
    let ready = posture_state(shell) == Some(PostureState::Ready);
    let stored = page(shell).is_some_and(|p| p.posture.calibrated_at.is_some());
    r.check(
        "posture_calibration_usable",
        fed && ready && stored,
        format!("fed {fed}; ready {ready}; stored {stored}"),
    );

    // A sustained drift: one cue asked of the engine, one episode recorded with its
    // outcome when it ends.
    let fed =
        feed(shell, &mut t, 85, |f| Features { gap: 0.40, ..f }) && feed(shell, &mut t, 20, |f| f);
    let answered = wait_until(Duration::from_secs(10), || {
        on_main(shell, |s| !s.camera_test_facts().cue_outcomes.is_empty()) == Some(true)
    });
    let facts = on_main(shell, |s| s.camera_test_facts());
    let outcomes = facts
        .as_ref()
        .map(|f| f.cue_outcomes.clone())
        .unwrap_or_default();
    let episodes: Vec<_> = facts
        .as_ref()
        .map(|f| {
            f.records
                .iter()
                .filter(|o| o.kind == PostureKind::DriftEpisode)
                .copied()
                .collect()
        })
        .unwrap_or_default();
    r.check(
        "posture_drift_cues_once",
        fed && answered
            && outcomes.len() == 1
            && episodes.len() == 1
            && episodes[0].cue == outcomes.first().copied(),
        format!("fed {fed}; engine answers {outcomes:?}; episodes recorded {episodes:?}"),
    );

    // The camera pushed back: cues suspended within 20 s.
    let fed = feed(shell, &mut t, 25, |f| Features {
        mid_y: f.mid_y - 0.18,
        width: f.width * 0.75,
        ..f
    });
    let state = posture_state(shell);
    r.check(
        "posture_moved_camera_suspends",
        fed && state == Some(PostureState::Suspect(PostureReason::CameraMoved)),
        format!("fed {fed}; state {state:?}"),
    );
}
