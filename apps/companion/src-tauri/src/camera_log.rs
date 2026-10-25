//! The camera soak log (slice V2-5; architecture 10 and 11 "Soak", assumption A2).
//!
//! `scripts/windows/Test-CameraSoak.ps1` samples the companion's processes from outside,
//! as the V1-7 soak does. What only the app knows about the posture check (its status,
//! the frame rate in force, frames read, watchdog stalls and restarts, and how long each
//! release of the camera took) it reads from this log: started with
//! `HY_CAMERA_SOAK_LOG=<file>` (and optionally `HY_CAMERA_SOAK_EVERY_S`, 5 by default), the
//! app appends one JSON line of those numbers every few seconds.
//!
//! Only counts, rates, times and status names are written: never a frame, a landmark, a
//! posture feature, a user name or a path, so the log can be shared. The app appends only
//! to a file that already exists (the script creates it), and stops when the file is
//! gone, has grown past [`MAX_BYTES`] or cannot be written, so a log left behind never
//! grows without end and nothing is written unless the script asked.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use hy_core::camera::{CameraHold, CameraStats, NoReading, Status};
use serde::Serialize;
use tauri::Manager;

use crate::shell::{CAMERA, Shell};

/// The environment variable naming the log file.
pub const ENV_LOG: &str = "HY_CAMERA_SOAK_LOG";
/// The environment variable for the interval, seconds (1 to 600).
pub const ENV_EVERY: &str = "HY_CAMERA_SOAK_EVERY_S";
/// The log stops at this size (about 4 days of lines at 5 s).
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_EVERY: Duration = Duration::from_secs(5);

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    /// Seconds since the log started (monotonic).
    pub t_s: f64,
    /// Wall-clock time, UTC (RFC 3339), to line up with the script's samples.
    pub utc: String,
    /// The status's kind: `off`, `starting`, `reading`, `no_reading`, `held` or
    /// `unavailable`.
    pub kind: &'static str,
    /// Its detail: the hold (`away`, `paused`, `off_hours`, `presentation`, `meeting`),
    /// why there is no reading (`starting`, `absent`, `low_confidence`, `out_of_frame`,
    /// `stalled`) or why the camera cannot be used (`denied`, `busy`, ...).
    pub detail: Option<String>,
    /// The user's choice is on.
    pub on: bool,
    /// The machine wants capture now (the page may be capturing).
    pub capture: bool,
    /// The camera page exists.
    pub page_open: bool,
    /// The frame rate in force.
    pub fps: u8,
    /// Frame results since the app started, and the usable ones.
    pub frames: u64,
    pub valid_frames: u64,
    /// Watchdog stalls and worker restarts since the app started.
    pub stalls: u32,
    pub worker_restarts: u32,
    /// Releases of the camera since the app started, those the page did not confirm, and
    /// the longest and the last, ms.
    pub stops: u32,
    pub forced_stops: u32,
    pub max_stop_ms: Option<u64>,
    pub last_stop_ms: Option<u64>,
    /// The mean recent model time per frame, ms.
    pub inference_ms: Option<f64>,
}

fn snake<T: Serialize>(v: &T) -> Option<String> {
    match serde_json::to_value(v).ok()? {
        serde_json::Value::String(s) => Some(s),
        // A detail with its own detail (`{"frame": "absent"}`): the inner name.
        serde_json::Value::Object(m) => m
            .values()
            .next()
            .and_then(|x| x.as_str().map(str::to_owned)),
        _ => None,
    }
}

/// The status as a kind and a detail.
#[must_use]
pub fn kind_and_detail(status: Status) -> (&'static str, Option<String>) {
    match status {
        Status::Off => ("off", None),
        Status::Starting => ("starting", None),
        Status::Reading => ("reading", None),
        Status::NoReading(n) => (
            "no_reading",
            match n {
                NoReading::Starting => Some("starting".to_owned()),
                NoReading::Stalled => Some("stalled".to_owned()),
                NoReading::Frame(issue) => snake(&issue),
            },
        ),
        Status::Held(h) => ("held", snake::<CameraHold>(&h)),
        Status::Unavailable(u) => ("unavailable", snake(&u)),
    }
}

impl Line {
    /// The line for one moment.
    #[must_use]
    pub fn new(
        t_s: f64,
        status: Status,
        on: bool,
        capture: bool,
        page_open: bool,
        stats: CameraStats,
    ) -> Self {
        let (kind, detail) = kind_and_detail(status);
        Self {
            t_s: (t_s * 10.0).round() / 10.0,
            utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            kind,
            detail,
            on,
            capture,
            page_open,
            fps: stats.fps,
            frames: stats.frames,
            valid_frames: stats.valid_frames,
            stalls: stats.stalls,
            worker_restarts: stats.worker_restarts,
            stops: stats.stops,
            forced_stops: stats.forced_stops,
            max_stop_ms: stats.max_stop_ms,
            last_stop_ms: stats.last_stop_ms,
            inference_ms: stats.inference_ms.map(|v| (v * 10.0).round() / 10.0),
        }
    }
}

/// The interval from [`ENV_EVERY`].
fn every() -> Duration {
    std::env::var(ENV_EVERY)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| (1..=600).contains(s))
        .map_or(DEFAULT_EVERY, Duration::from_secs)
}

/// Append `line` to `path`, which must exist. Whether to go on.
fn append(path: &PathBuf, line: &Line) -> bool {
    let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) else {
        return false;
    };
    if f.metadata().map_or(true, |m| m.len() > MAX_BYTES) {
        return false;
    }
    let Ok(text) = serde_json::to_string(line) else {
        return false;
    };
    writeln!(f, "{text}").is_ok()
}

/// Start the log if [`ENV_LOG`] names a file that exists.
pub fn start_from_env(shell: &Arc<Shell>) {
    let Some(path) = std::env::var_os(ENV_LOG).map(PathBuf::from) else {
        return;
    };
    if !path.is_file() {
        eprintln!("HealthierYou: the camera soak log file does not exist; no log is written");
        return;
    }
    let every = every();
    let weak = Arc::downgrade(shell);
    let origin = Instant::now();
    let _ = std::thread::Builder::new()
        .name("hy-camera-log".into())
        .spawn(move || {
            loop {
                let Some(s) = weak.upgrade() else { return };
                let (tx, rx) = mpsc::channel();
                s.post(move |s| {
                    let page = s.camera_page();
                    let open = s.app().get_webview_window(CAMERA).is_some();
                    let _ = tx.send((page, open));
                });
                drop(s);
                if let Ok((page, open)) = rx.recv_timeout(Duration::from_secs(15)) {
                    let line = Line::new(
                        origin.elapsed().as_secs_f64(),
                        page.status,
                        page.on,
                        page.capture,
                        open,
                        page.stats,
                    );
                    if !append(&path, &line) {
                        return;
                    }
                }
                std::thread::sleep(every);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hy_core::camera::{FrameIssue, Unavailable};

    #[test]
    fn statuses_become_a_kind_and_a_plain_detail() {
        assert_eq!(kind_and_detail(Status::Off), ("off", None));
        assert_eq!(kind_and_detail(Status::Reading), ("reading", None));
        assert_eq!(
            kind_and_detail(Status::NoReading(NoReading::Frame(FrameIssue::Absent))),
            ("no_reading", Some("absent".to_owned()))
        );
        assert_eq!(
            kind_and_detail(Status::NoReading(NoReading::Stalled)),
            ("no_reading", Some("stalled".to_owned()))
        );
        assert_eq!(
            kind_and_detail(Status::Held(CameraHold::OffHours)),
            ("held", Some("off_hours".to_owned()))
        );
        assert_eq!(
            kind_and_detail(Status::Unavailable(Unavailable::Stalled)),
            ("unavailable", Some("stalled".to_owned()))
        );
        assert_eq!(
            kind_and_detail(Status::Unavailable(Unavailable::NoCamera)),
            ("unavailable", Some("no_camera".to_owned()))
        );
    }

    #[test]
    fn a_line_holds_numbers_and_names_only() {
        let stats = CameraStats {
            frames: 120,
            valid_frames: 3,
            stops: 2,
            max_stop_ms: Some(410),
            last_stop_ms: Some(350),
            inference_ms: Some(36.84),
            fps: 3,
            ..CameraStats::default()
        };
        let line = Line::new(
            12.345,
            Status::Held(CameraHold::Meeting),
            true,
            false,
            false,
            stats,
        );
        let v = serde_json::to_value(&line).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        let mut want = vec![
            "t_s",
            "utc",
            "kind",
            "detail",
            "on",
            "capture",
            "page_open",
            "fps",
            "frames",
            "valid_frames",
            "stalls",
            "worker_restarts",
            "stops",
            "forced_stops",
            "max_stop_ms",
            "last_stop_ms",
            "inference_ms",
        ];
        want.sort_unstable();
        assert_eq!(keys, want);
        assert_eq!(v["t_s"], 12.3);
        assert_eq!(v["detail"], "meeting");
        assert_eq!(v["inference_ms"], 36.8);
        // Every string is a fixed name or the time, never free text.
        for (k, x) in v.as_object().unwrap() {
            if let Some(s) = x.as_str() {
                assert!(
                    k == "utc" || s.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                    "{k}: {s}"
                );
            }
        }
    }

    #[test]
    fn the_log_appends_only_to_a_file_that_exists() {
        let dir = std::env::temp_dir().join(format!("hy-camera-log-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("camera-soak.jsonl");
        let _ = std::fs::remove_file(&path);
        let line = Line::new(
            1.0,
            Status::Off,
            false,
            false,
            false,
            CameraStats::default(),
        );
        assert!(!append(&path, &line), "no file, no log");
        assert!(!path.exists());
        std::fs::write(&path, "").unwrap();
        assert!(append(&path, &line));
        assert!(append(&path, &line));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        let back: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(back["kind"], "off");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
