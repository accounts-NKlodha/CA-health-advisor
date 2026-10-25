//! The accelerated camera soak (slice V2-5; architecture 11 "Soak", assumption A2), run by
//! the Windows CI job after the camera self-test:
//! `HY_SELFTEST=<report> HY_SELFTEST_CAMERA_SOAK_S=<real seconds> hy-companion.exe`.
//!
//! The real 2-hour soak runs on an office PC with a real webcam
//! (`scripts/windows/Test-CameraSoak.ps1`). A CI runner has no webcam and cannot wait two
//! hours, so this phase keeps the posture check capturing on Chromium's synthetic camera
//! (a generated pattern with no person in it) for the given real time, and packs into it
//! many more releases than a working day has. Each cycle reads frames for 20 s (the rate
//! is measured against the rate the governor set), then releases the camera one way, in
//! turn: a lock and a sleep (through the coaching engine, as the Windows signals arrive),
//! a presentation (the engine's input), a meeting hold, and the user's Off; it checks the
//! page confirmed the release within two seconds and the camera page is gone, holds it a
//! few seconds, ends it, and times how long capture takes to read again.
//!
//! The script outside samples the process tree (memory, handles, threads, CPU, and its
//! TCP connections) throughout and checks there is no growth trend and no internet
//! connection; it also reads the camera log (`crate::camera_log`). This side checks what
//! only the app knows:
//!
//! - the frame rate: every 20 s window delivered the rate the governor set (at least 80%
//!   in the median, 50% in the worst window) and never more than 3 a second;
//! - every release was confirmed by the page within two seconds, nothing was forced, and
//!   the machine counted each stop once;
//! - capture read again after every release;
//! - the watchdog never fired: no stall and no worker restart on a synthetic camera that
//!   never stops sending frames.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use hy_core::camera::{CameraHold, CameraStats, FPS_HIGH, Status};
use serde::Serialize;
use tauri::Manager;

use super::{Report, on_main, wait_until};
use crate::coach::{CoachRunner, SystemSignal};
use crate::shell::{CAMERA, Shell};
use hy_core::coach::Input;

/// Frames are counted over this long in each cycle.
const STEADY: Duration = Duration::from_secs(20);
/// A release is held this long before it ends.
const HOLD: Duration = Duration::from_secs(4);
/// The deadline for a release (architecture 4.5; PRD "off releases capture promptly").
const STOP_WITHIN_MS: u64 = 2000;

/// What the camera soak is asked to do.
#[derive(Debug, Clone, Copy)]
pub struct CameraSoakPlan {
    /// Real time to run for.
    pub real: Duration,
}

impl CameraSoakPlan {
    /// The plan from `HY_SELFTEST_CAMERA_SOAK_S`, if the camera soak was asked for.
    pub fn from_env() -> Option<Self> {
        let secs: u64 = std::env::var("HY_SELFTEST_CAMERA_SOAK_S")
            .ok()?
            .parse()
            .ok()?;
        (secs > 0).then(|| Self {
            real: Duration::from_secs(secs),
        })
    }
}

/// The ways the soak releases the camera, in turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Release {
    Lock,
    Sleep,
    Presentation,
    Meeting,
    Off,
}

const CYCLE: [Release; 5] = [
    Release::Lock,
    Release::Presentation,
    Release::Sleep,
    Release::Meeting,
    Release::Off,
];

impl Release {
    const fn expected(self) -> Status {
        match self {
            Self::Lock | Self::Sleep => Status::Held(CameraHold::Away),
            Self::Presentation => Status::Held(CameraHold::Presentation),
            Self::Meeting => Status::Held(CameraHold::Meeting),
            Self::Off => Status::Off,
        }
    }
}

/// One release and its end.
#[derive(Debug, Clone, Serialize)]
struct ReleaseResult {
    kind: Release,
    /// The expected status shown, the page gone and the stop counted.
    released: bool,
    stop_ms: Option<u64>,
    forced: bool,
    /// Still released at the end of the hold.
    stayed_released: bool,
    /// Capture read again after the release ended.
    resumed: bool,
    resume_ms: Option<u64>,
}

/// One window of steady capture.
#[derive(Debug, Clone, Copy, Serialize)]
struct RateWindow {
    set_fps: u8,
    frames: u64,
    secs: f64,
    fps: f64,
}

/// What the camera soak did, for the report and the script outside.
#[derive(Debug, Default, Serialize)]
pub struct CameraSoakSummary {
    /// Real wall-clock start and end (UTC, RFC 3339): the script samples only between.
    pub started_utc: String,
    pub ended_utc: String,
    real_s: u64,
    cycles: u32,
    releases: Vec<ReleaseResult>,
    windows: Vec<RateWindow>,
    /// Windows skipped because the governor changed the rate during them.
    windows_rate_changed: u32,
    frames: u64,
    stalls: u32,
    worker_restarts: u32,
    stops: u32,
    forced_stops: u32,
    max_stop_ms: Option<u64>,
}

fn page(shell: &Arc<Shell>) -> Option<crate::view::CameraPage> {
    on_main(shell, |s| s.camera_page())
}

fn stats(shell: &Arc<Shell>) -> CameraStats {
    page(shell).map(|p| p.stats).unwrap_or_default()
}

fn status(shell: &Arc<Shell>) -> Option<Status> {
    page(shell).map(|p| p.status)
}

fn camera_window(shell: &Arc<Shell>) -> bool {
    on_main(shell, |s| s.app().get_webview_window(CAMERA).is_some()) == Some(true)
}

/// Wait for capture to read frames again (after a start or a release).
fn reading_again(shell: &Arc<Shell>, timeout: Duration) -> bool {
    let f0 = stats(shell).frames;
    wait_until(timeout, || {
        page(shell).is_some_and(|p| p.stats.frames >= f0 + 3 && p.status.capturing())
    })
}

fn release(shell: &Arc<Shell>, runner: &CoachRunner, kind: Release) {
    match kind {
        Release::Lock => runner.system(SystemSignal::Lock),
        Release::Sleep => runner.system(SystemSignal::Suspend),
        Release::Presentation => runner.input(Input::PresentationChanged(true)),
        Release::Meeting => {
            let _ = on_main(shell, |s| s.camera_set_hold(Some(CameraHold::Meeting)));
        }
        Release::Off => {
            let _ = on_main(shell, |s| s.camera_off());
        }
    }
}

fn end_release(shell: &Arc<Shell>, runner: &CoachRunner, kind: Release) {
    match kind {
        Release::Lock => runner.system(SystemSignal::Unlock),
        Release::Sleep => runner.system(SystemSignal::Resume),
        Release::Presentation => runner.input(Input::PresentationChanged(false)),
        Release::Meeting => {
            let _ = on_main(shell, |s| s.camera_set_hold(None));
        }
        Release::Off => {
            let _ = on_main(shell, |s| s.camera_on());
        }
    }
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        f64::midpoint(v[n / 2 - 1], v[n / 2])
    })
}

/// Run the camera soak and add its checks to `r`.
#[allow(clippy::too_many_lines)]
pub(super) fn run(
    r: &mut Report,
    shell: &Arc<Shell>,
    runner: Option<&Arc<CoachRunner>>,
    plan: CameraSoakPlan,
) -> CameraSoakSummary {
    let mut s = CameraSoakSummary {
        started_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        ..CameraSoakSummary::default()
    };
    let Some(runner) = runner else {
        r.check(
            "camera_soak_started",
            false,
            "the coaching engine did not start (it carries lock, sleep and presentation)",
        );
        s.ended_utc = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        return s;
    };
    // The engine's holds must leave the camera free (the simulated clock is in work
    // hours), or nothing would capture.
    let holds = runner.call(|l| l.view().holds);
    let _ = on_main(shell, |s| s.camera_on());
    let started = reading_again(shell, Duration::from_secs(60));
    r.check(
        "camera_soak_started",
        started,
        format!(
            "reading on the synthetic camera {started}; engine holds {holds:?}; status {:?}",
            status(shell)
        ),
    );
    if !started {
        let _ = on_main(shell, |s| s.camera_off());
        s.ended_utc = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        return s;
    }

    let first = stats(shell);
    let t0 = Instant::now();
    while t0.elapsed() < plan.real && t0.elapsed() < plan.real * 2 {
        // Steady capture: the frame rate against the rate in force.
        let a = page(shell);
        let w0 = Instant::now();
        std::thread::sleep(STEADY);
        let b = page(shell);
        if let (Some(a), Some(b)) = (a, b) {
            if a.fps == b.fps && a.status.capturing() && b.status.capturing() {
                let secs = w0.elapsed().as_secs_f64();
                let frames = b.stats.frames.saturating_sub(a.stats.frames);
                #[allow(clippy::cast_precision_loss)]
                s.windows.push(RateWindow {
                    set_fps: b.fps,
                    frames,
                    secs: (secs * 10.0).round() / 10.0,
                    fps: (frames as f64 / secs * 100.0).round() / 100.0,
                });
            } else {
                s.windows_rate_changed += 1;
            }
        }
        if t0.elapsed() >= plan.real {
            break;
        }

        // One release, in turn.
        let kind = CYCLE[s.cycles as usize % CYCLE.len()];
        let stops0 = stats(shell).stops;
        release(shell, runner, kind);
        let released = wait_until(Duration::from_secs(10), || {
            status(shell) == Some(kind.expected())
                && !camera_window(shell)
                && stats(shell).stops > stops0
        });
        let st = stats(shell);
        std::thread::sleep(HOLD);
        let stayed = status(shell) == Some(kind.expected()) && !camera_window(shell);
        let t_end = Instant::now();
        end_release(shell, runner, kind);
        let resumed = reading_again(shell, Duration::from_secs(60));
        let resume_ms =
            resumed.then(|| u64::try_from(t_end.elapsed().as_millis()).unwrap_or(u64::MAX));
        s.releases.push(ReleaseResult {
            kind,
            released,
            stop_ms: released.then_some(st.last_stop_ms).flatten(),
            forced: st.last_stop_forced,
            stayed_released: stayed,
            resumed,
            resume_ms,
        });
        s.cycles += 1;
        if !resumed {
            // Nothing more can be measured; the checks below report it.
            break;
        }
    }
    let last = stats(shell);
    s.real_s = t0.elapsed().as_secs();
    s.frames = last.frames.saturating_sub(first.frames);
    s.stalls = last.stalls.saturating_sub(first.stalls);
    s.worker_restarts = last.worker_restarts.saturating_sub(first.worker_restarts);
    s.stops = last.stops.saturating_sub(first.stops);
    s.forced_stops = last.forced_stops.saturating_sub(first.forced_stops);
    s.max_stop_ms = s.releases.iter().filter_map(|x| x.stop_ms).max();

    let n = s.releases.len();
    let kinds_seen = CYCLE
        .iter()
        .filter(|k| s.releases.iter().any(|x| x.kind == **k))
        .count();
    r.check(
        "camera_soak_completed",
        kinds_seen == CYCLE.len() && s.real_s <= plan.real.as_secs() * 2 + 120,
        format!(
            "{} cycles in {} s (asked {} s); every release kind exercised: {kinds_seen} of {}; {} frames read",
            s.cycles,
            s.real_s,
            plan.real.as_secs(),
            CYCLE.len(),
            s.frames
        ),
    );

    let ratios: Vec<f64> = s
        .windows
        .iter()
        .map(|w| w.fps / f64::from(w.set_fps.max(1)))
        .collect();
    let med = median(ratios.clone());
    let worst = ratios.iter().copied().fold(f64::INFINITY, f64::min);
    let fastest = s.windows.iter().map(|w| w.fps).fold(0.0, f64::max);
    let low = s.windows.iter().filter(|w| w.set_fps < FPS_HIGH).count();
    r.check(
        "camera_soak_frame_rate",
        s.windows.len() >= 5
            && med.is_some_and(|m| m >= 0.8)
            && worst >= 0.5
            && fastest <= f64::from(FPS_HIGH) * 1.2,
        format!(
            "{} windows of 20 s (another {} skipped: the governor changed the rate); delivered/set median {:.2}, worst {worst:.2}; fastest {fastest:.2} a second (cap {FPS_HIGH}); {low} windows at the reduced rate; rates {:?}",
            s.windows.len(),
            s.windows_rate_changed,
            med.unwrap_or(0.0),
            s.windows.iter().map(|w| w.fps).collect::<Vec<_>>()
        ),
    );

    let late: Vec<_> = s
        .releases
        .iter()
        .filter(|x| !x.released || x.forced || x.stop_ms.is_none_or(|ms| ms > STOP_WITHIN_MS))
        .map(|x| (x.kind, x.stop_ms, x.forced))
        .collect();
    let stop_list: Vec<_> = s.releases.iter().map(|x| x.stop_ms).collect();
    r.check(
        "camera_soak_releases_within_2s",
        n > 0
            && late.is_empty()
            && s.releases.iter().all(|x| x.stayed_released)
            && s.stops == u32::try_from(n).unwrap_or(0)
            && s.forced_stops == 0,
        format!(
            "{n} releases ({} lock, {} sleep, {} presentation, {} meeting, {} off); stop times {stop_list:?} ms, longest {:?} ms (deadline {STOP_WITHIN_MS}); late, unconfirmed or forced {late:?}; stops counted {} (forced {}); all stayed released through the hold {}",
            s.releases.iter().filter(|x| x.kind == Release::Lock).count(),
            s.releases.iter().filter(|x| x.kind == Release::Sleep).count(),
            s.releases.iter().filter(|x| x.kind == Release::Presentation).count(),
            s.releases.iter().filter(|x| x.kind == Release::Meeting).count(),
            s.releases.iter().filter(|x| x.kind == Release::Off).count(),
            s.max_stop_ms,
            s.stops,
            s.forced_stops,
            s.releases.iter().all(|x| x.stayed_released)
        ),
    );

    let resume: Vec<_> = s.releases.iter().map(|x| x.resume_ms).collect();
    r.check(
        "camera_soak_reads_again_after_each_release",
        n > 0 && s.releases.iter().all(|x| x.resumed),
        format!("time to read again after each release ended {resume:?} ms (limit 60000)"),
    );

    r.check(
        "camera_soak_watchdog_bounded",
        s.stalls == 0 && s.worker_restarts == 0 && s.frames > 0,
        format!(
            "{} stalls and {} worker restarts over {} frames (none allowed: the synthetic camera never stops sending)",
            s.stalls, s.worker_restarts, s.frames
        ),
    );

    let _ = on_main(shell, |s| s.camera_off());
    let off = wait_until(Duration::from_secs(5), || {
        status(shell) == Some(Status::Off) && !camera_window(shell)
    });
    r.check(
        "camera_soak_off_at_end",
        off,
        format!("status {:?}", status(shell)),
    );
    s.ended_utc = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_exercises_every_release_and_each_has_its_status() {
        for k in [
            Release::Lock,
            Release::Sleep,
            Release::Presentation,
            Release::Meeting,
            Release::Off,
        ] {
            assert!(CYCLE.contains(&k));
            assert!(!k.expected().capturing(), "{k:?} must release capture");
        }
    }

    #[test]
    fn median_of_odd_and_even() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(vec![4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }
}
