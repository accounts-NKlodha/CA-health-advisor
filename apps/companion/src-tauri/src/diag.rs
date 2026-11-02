//! Companion observability (tracked work W2, architecture 10), wired to Windows. The
//! rules and formats are in `hy_core::diag` (tested on Linux); this module holds the
//! app's single log, its panic hook, the hourly resource summary and the diagnostics zip.
//!
//! - The log is `%LOCALAPPDATA%\HealthierYou\logs\companion.log` (the self-test's goes in
//!   its temp folder), JSON lines in the server's format, rotated at 10 MB or 7 days.
//!   Only typed values are written (`hy_core::diag::V`): event names, counts, sizes and
//!   fixed names, never a health value, a user or PC name, a path or a device id. The
//!   folder is program state: the uninstaller removes it.
//! - The panic hook logs where the panic happened (two path parts) and whether it was the
//!   window thread, never its message. A release build ends the process on a panic
//!   (`panic = "abort"`), so the hook then starts the app again (`--hy-after-panic`,
//!   which waits for this process to end first), at most once in 10 minutes
//!   (`hy_core::diag::restart_allowed`) so a fault at start-up cannot loop. A pet page
//!   whose webview died is made again by the shell (`shell::recover`).
//! - Every hour one `resources` line sums the process tree (the app and its WebView2
//!   processes): working set, private working set, private bytes, handles, threads, CPU.
//! - Settings > App & updates builds the diagnostics zip in memory
//!   (`hy_core::diag::bundle`), shows the user every file in it, and writes exactly those
//!   bytes only where the user chooses. Nothing is uploaded.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use chrono::{Datelike, Timelike, Utc};
use hy_core::diag::bundle::{self, Bundle, Preview, Stamp};
use hy_core::diag::log::RotatingLog;
use hy_core::diag::resources::{EVERY, Summary, TreeSample};
use hy_core::diag::{Level, V};

/// The argument a restart after a panic starts with, followed by the old process's id.
pub const ARG_AFTER_PANIC: &str = "--hy-after-panic";
/// The marker holding when the app last restarted itself after a panic (Unix seconds).
const RESTART_MARKER: &str = "last-panic-restart";

static LOG: OnceLock<RotatingLog> = OnceLock::new();
static STARTED: OnceLock<Instant> = OnceLock::new();

/// The log folder under the per-user local data folder.
#[must_use]
pub fn logs_dir(local_data: &Path) -> PathBuf {
    local_data.join("HealthierYou").join("logs")
}

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Open the log in `dir`. False if it could not be opened (the app runs without it) or
/// was already open.
pub fn init(dir: &Path) -> bool {
    let _ = STARTED.set(Instant::now());
    match RotatingLog::open(dir, now_s()) {
        Ok(l) => LOG.set(l).is_ok(),
        Err(e) => {
            eprintln!("HealthierYou: the log could not be opened: {e}");
            false
        }
    }
}

/// The open log's folder.
pub fn log_dir() -> Option<&'static Path> {
    LOG.get().map(RotatingLog::dir)
}

/// Write one line (nothing if the log is not open; a write error is ignored).
pub fn log(level: Level, event: &'static str, fields: &[(&'static str, V)]) {
    if let Some(l) = LOG.get() {
        let _ = l.write_line(
            &hy_core::diag::line(Utc::now(), level, event, fields),
            now_s(),
        );
    }
}

fn uptime() -> Duration {
    STARTED.get().map_or(Duration::ZERO, Instant::elapsed)
}

// ---------------------------------------------------------------------- panics

/// Install the panic hook (see the module notes). `restart`: start the app again after a
/// panic in a release build (off in the self-test, which must end with its report).
pub fn install_panic_hook(restart: bool) {
    std::panic::set_hook(Box::new(move |info| {
        let (file, line) = info
            .location()
            .map_or(("unknown", 0), |l| (l.file(), l.line()));
        let main = std::thread::current().name() == Some("main");
        if let Some(l) = LOG.get() {
            l.try_write_line(
                &hy_core::diag::panic_line(Utc::now(), file, line, main),
                now_s(),
            );
        }
        eprintln!(
            "HealthierYou: internal error at {}:{line}",
            hy_core::diag::short_location(file)
        );
        if restart && cfg!(panic = "abort") {
            restart_after_panic();
        }
    }));
}

fn try_log(level: Level, event: &'static str, fields: &[(&'static str, V)]) {
    if let Some(l) = LOG.get() {
        l.try_write_line(
            &hy_core::diag::line(Utc::now(), level, event, fields),
            now_s(),
        );
    }
}

/// Start the app again after a panic, unless it already did within the last 10 minutes.
fn restart_after_panic() {
    let Some(dir) = log_dir() else { return };
    let marker = dir.join(RESTART_MARKER);
    let last = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    let now = now_s();
    if !hy_core::diag::restart_allowed(last, now) {
        try_log(Level::Error, "panic_restart_skipped", &[]);
        return;
    }
    let _ = std::fs::write(&marker, now.to_string());
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let started = std::process::Command::new(exe)
        .arg(ARG_AFTER_PANIC)
        .arg(std::process::id().to_string())
        .spawn()
        .is_ok();
    try_log(
        Level::Error,
        "panic_restart",
        &[("started", V::Bool(started))],
    );
}

/// `--hy-after-panic <pid>`: wait (up to 20 s) for the process that panicked to end, so
/// two companions never run at once, then start normally. True if this run is such a
/// restart.
pub fn after_panic(args: &[std::ffi::OsString]) -> bool {
    if args.first().is_none_or(|a| a != ARG_AFTER_PANIC) {
        return false;
    }
    #[cfg(windows)]
    if let Some(pid) = args.get(1).and_then(|a| a.to_str()?.parse::<u32>().ok()) {
        crate::win32::wait_for_exit(pid, Duration::from_secs(20));
    }
    true
}

// ------------------------------------------------------------------- resources

/// The process tree now (Windows; nothing elsewhere).
#[must_use]
pub fn sample() -> TreeSample {
    #[cfg(windows)]
    {
        crate::win32::process_tree_usage()
    }
    #[cfg(not(windows))]
    {
        TreeSample::default()
    }
}

fn cores() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| u32::try_from(n.get()).unwrap_or(u32::MAX))
}

/// Log the summary of the window from `prev`, taken at `prev_at`, to now; returns the
/// new reading and the summary.
pub fn log_resources(prev: &TreeSample, prev_at: Instant) -> (TreeSample, Summary) {
    let now = sample();
    let s = Summary::between(prev, &now, prev_at.elapsed(), cores(), uptime());
    log(Level::Info, "resources", &s.fields());
    (now, s)
}

/// Write the resource summary every hour, for the app's life.
pub fn start_resource_summary() {
    let _ = std::thread::Builder::new()
        .name("hy-resources".into())
        .spawn(|| {
            let mut prev = sample();
            let mut at = Instant::now();
            loop {
                std::thread::sleep(EVERY);
                let (now, _) = log_resources(&prev, at);
                prev = now;
                at = Instant::now();
            }
        });
}

// ----------------------------------------------------------------- diagnostics

/// The PC's own names, removed from the zip if any slipped into the log.
fn planted() -> Vec<String> {
    let mut v: Vec<String> = [
        "USERNAME",
        "USERDOMAIN",
        "USERDNSDOMAIN",
        "COMPUTERNAME",
        "USERPROFILE",
        "HOMEPATH",
        "LOCALAPPDATA",
        "APPDATA",
        "LOGONSERVER",
        "ONEDRIVE",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok())
    .map(|s| s.trim_start_matches('\\').to_owned())
    .filter(|s| !s.is_empty())
    .collect();
    if let Ok(exe) = std::env::current_exe() {
        v.push(exe.display().to_string());
    }
    v
}

fn stamp() -> Stamp {
    let n = chrono::Local::now();
    Stamp {
        date: (n.year(), n.month(), n.day()),
        time: (n.hour(), n.minute(), n.second()),
    }
}

/// The diagnostics zip being reviewed: built by "Prepare", written by "Save", dropped by
/// "Cancel".
#[derive(Debug, Default)]
pub struct Diagnostics {
    staged: Mutex<Option<Bundle>>,
}

impl Diagnostics {
    /// Build the zip from the log folder `dir` and keep it for saving; the preview shows
    /// every file in it.
    pub fn prepare_from(&self, dir: &Path, version: &str) -> Preview {
        let names = planted();
        let planted: Vec<&str> = names.iter().map(String::as_str).collect();
        let about = [
            ("HealthierYou version", version.to_owned()),
            (
                "System",
                format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            ),
            ("Logical processors", cores().to_string()),
            ("Running for", format!("{} min", uptime().as_secs() / 60)),
        ];
        let b = bundle::build(dir, &about, &planted, stamp());
        let preview = b.preview.clone();
        *self.staged.lock().unwrap_or_else(PoisonError::into_inner) = Some(b);
        preview
    }

    /// As [`prepare_from`](Self::prepare_from), from the app's log folder.
    ///
    /// # Errors
    /// The log is not open.
    pub fn prepare(&self, version: &str) -> Result<Preview, String> {
        let dir =
            log_dir().ok_or_else(|| "HealthierYou keeps no log on this PC yet.".to_owned())?;
        log(Level::Info, "diagnostics_prepared", &[]);
        Ok(self.prepare_from(dir, version))
    }

    /// The staged zip's suggested name, if one is staged.
    pub fn suggested_name(&self) -> Option<String> {
        self.staged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|b| b.preview.suggested_name.clone())
    }

    /// Write the staged zip, exactly as previewed, to `path`, and drop it.
    ///
    /// # Errors
    /// Nothing staged, or the file cannot be written; worded for the page.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut staged = self.staged.lock().unwrap_or_else(PoisonError::into_inner);
        let b = staged
            .as_ref()
            .ok_or_else(|| "Prepare the diagnostics first.".to_owned())?;
        std::fs::write(path, &b.zip).map_err(|e| format!("The zip could not be saved: {e}"))?;
        let bytes = b.zip.len() as u64;
        *staged = None;
        drop(staged);
        log(Level::Info, "diagnostics_saved", &[("bytes", V::N(bytes))]);
        Ok(())
    }

    /// Drop the staged zip ("Cancel").
    pub fn discard(&self) {
        *self.staged.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// The staged zip's bytes (the self-test compares them with what it saved).
    pub fn staged_zip(&self) -> Option<Vec<u8>> {
        self.staged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|b| b.zip.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_lives_under_healthieryou_logs() {
        let p = logs_dir(Path::new("base"));
        assert!(p.ends_with("HealthierYou/logs") || p.ends_with("HealthierYou\\logs"));
    }

    #[test]
    fn only_the_after_panic_argument_is_a_restart() {
        use std::ffi::OsString;
        assert!(!after_panic(&[]));
        assert!(!after_panic(&[OsString::from("--hy-soak-stats")]));
        // A process id that has ended (or never was) does not hold the start.
        assert!(after_panic(&[
            OsString::from(ARG_AFTER_PANIC),
            OsString::from("4294967294")
        ]));
    }

    #[test]
    fn the_diagnostics_zip_is_saved_as_previewed_and_then_dropped() {
        let dir = std::env::temp_dir().join(format!("hy-diag-test-{}", std::process::id()));
        let log = RotatingLog::open(&dir, now_s()).unwrap();
        log.write_line(
            &hy_core::diag::line(Utc::now(), Level::Info, "start", &[]),
            now_s(),
        )
        .unwrap();
        let d = Diagnostics::default();
        let p = d.prepare_from(&dir, "1.2.3");
        assert_eq!(p.kept_lines, 1);
        let staged = d.staged_zip().unwrap();
        let out = dir.join("out.zip");
        d.save_to(&out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), staged);
        assert!(d.staged_zip().is_none());
        assert!(d.save_to(&out).is_err());
        let files = hy_core::diag::zip::read(&staged).unwrap();
        assert!(String::from_utf8_lossy(&files[0].1).contains("HealthierYou version: 1.2.3"));
        d.prepare_from(&dir, "1.2.3");
        d.discard();
        assert!(d.suggested_name().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
