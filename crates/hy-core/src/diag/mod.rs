//! Companion observability (tracked work W2, architecture 10): the companion's own
//! JSON-line log, its hourly resource summary, the panic and webview-failure rules, and the
//! user-reviewed diagnostics zip. The companion (`apps/companion/src-tauri/src/diag.rs`)
//! wires these to Windows; everything here is plain Rust, tested on Linux.
//!
//! The log follows the server's conventions (`hy-server` `logging`): one JSON object per
//! line, `{"timestamp": ..., "level": ..., "fields": {"event": ..., "detail": "<JSON>"}}`,
//! so one parser reads both logs (`scripts/windows/Test-Soak.ps1` does). Every field is a
//! closed, typed value ([`V`]): counts, sizes, rounded decimals, booleans and names fixed
//! in code. There is no variant for free text, so a health value typed by the user, a
//! user or PC name, a path or a device id cannot be logged by mistake. A panic logs only
//! its source location, cut to the last two path parts (a dependency's path would hold
//! the build machine's user folder), never its message.
//!
//! - [`log`]: the rotating file (10 MB, or 7 days since its first line, whichever comes
//!   first; rotated files kept 7 days, at most [`log::KEEP_FILES`]);
//! - [`resources`]: the hourly summary of the process tree (working set, private working
//!   set, private bytes, handles, threads, CPU);
//! - [`redact`]: the second pass every line goes through before it enters a
//!   diagnostics zip (paths, e-mail addresses, long ids and the PC's own names removed,
//!   unreadable lines dropped);
//! - [`bundle`]: the diagnostics zip's contents, built for the user to read before saving;
//! - [`zip`]: a minimal stored (uncompressed) zip writer and reader.

pub mod bundle;
pub mod log;
pub mod redact;
pub mod resources;
pub mod zip;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value};

/// An allowlisted log value (the server's `V`, for the companion's own fields).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum V {
    /// A count, size, duration in whole units or version part.
    N(u64),
    /// A measurement such as megabytes or a percentage, rounded to two decimals.
    Dec(f64),
    /// A fixed name chosen in code (window, reason, outcome, version string).
    Name(&'static str),
    /// A yes or no.
    Bool(bool),
}

impl V {
    fn render(self) -> Value {
        match self {
            Self::N(n) => Value::from(n),
            Self::Dec(d) if d.is_finite() => {
                serde_json::Number::from_f64((d * 100.0).round() / 100.0)
                    .map_or(Value::Null, Value::Number)
            }
            Self::Dec(_) => Value::Null,
            Self::Name(n) => Value::String(n.to_owned()),
            Self::Bool(b) => Value::Bool(b),
        }
    }
}

/// A log level, written as the server's tracing levels are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Normal events and summaries.
    Info,
    /// Something recovered from (a webview restarted).
    Warn,
    /// A panic, or a recovery that gave up.
    Error,
}

impl Level {
    /// The level's name in the log.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

fn envelope(at: DateTime<Utc>, level: Level, fields: Map<String, Value>) -> String {
    let mut o = Map::new();
    o.insert(
        "timestamp".into(),
        Value::String(at.to_rfc3339_opts(SecondsFormat::Micros, true)),
    );
    o.insert("level".into(), Value::String(level.as_str().into()));
    o.insert("fields".into(), Value::Object(fields));
    Value::Object(o).to_string()
}

/// One log line (no newline): the event and its allowlisted fields, the fields as a JSON
/// string under `detail` exactly as the server writes them.
#[must_use]
pub fn line(
    at: DateTime<Utc>,
    level: Level,
    event: &'static str,
    fields: &[(&'static str, V)],
) -> String {
    let mut detail = Map::new();
    for (k, v) in fields {
        detail.insert((*k).to_owned(), v.render());
    }
    let mut f = Map::new();
    f.insert("event".into(), Value::String(event.into()));
    f.insert(
        "detail".into(),
        Value::String(Value::Object(detail).to_string()),
    );
    envelope(at, level, f)
}

/// The last two parts of a source path (`src/shell.rs`), whatever the separator. The
/// companion's own files are short relative paths already; a dependency's file is an
/// absolute path on the build machine, whose user folder must not be logged.
#[must_use]
pub fn short_location(file: &str) -> String {
    let parts: Vec<&str> = file
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != "?" && !p.ends_with(':'))
        .collect();
    let start = parts.len().saturating_sub(2);
    parts[start..].join("/")
}

/// The panic line: where it happened (see [`short_location`]) and whether it was the main
/// (window) thread. Never the panic's message, which may hold any value.
#[must_use]
pub fn panic_line(at: DateTime<Utc>, file: &str, line_no: u32, main_thread: bool) -> String {
    let mut detail = Map::new();
    detail.insert("file".into(), Value::String(short_location(file)));
    detail.insert("line".into(), Value::from(line_no));
    detail.insert("main_thread".into(), Value::Bool(main_thread));
    let mut f = Map::new();
    f.insert("event".into(), Value::String("panic".into()));
    f.insert(
        "detail".into(),
        Value::String(Value::Object(detail).to_string()),
    );
    envelope(at, Level::Error, f)
}

/// The shortest time between two restarts after a panic. A panic again within this time
/// leaves the app closed, so a fault at start-up cannot loop.
pub const RESTART_GAP_S: u64 = 600;

/// Whether the app may start itself again after a panic now (Unix seconds), given when it
/// last did (from its marker file).
#[must_use]
pub fn restart_allowed(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|t| t > now || now - t >= RESTART_GAP_S)
}

/// What a WebView2 process failure means for the window it was reported on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebviewFailure {
    /// The failure kind's name for the log.
    pub kind: &'static str,
    /// The page is gone (its renderer or the whole browser process exited, or the page
    /// hangs): the window must be made again. Other processes (GPU, utility, plug-in)
    /// are restarted by WebView2 itself and the page goes on.
    pub page_lost: bool,
}

/// The meaning of `COREWEBVIEW2_PROCESS_FAILED_KIND` value `kind`.
#[must_use]
pub const fn webview_failure(kind: i32) -> WebviewFailure {
    let (kind, page_lost) = match kind {
        0 => ("browser_process_exited", true),
        1 => ("render_process_exited", true),
        2 => ("render_process_unresponsive", true),
        3 => ("frame_render_process_exited", false),
        4 => ("utility_process_exited", false),
        5 => ("sandbox_helper_process_exited", false),
        6 => ("gpu_process_exited", false),
        7 => ("ppapi_plugin_process_exited", false),
        8 => ("ppapi_broker_process_exited", false),
        9 => ("unknown_process_exited", false),
        _ => ("other", false),
    };
    WebviewFailure { kind, page_lost }
}

/// At most `max` recoveries in any `window`: a page that dies again at once is not made
/// again for ever.
#[derive(Debug, Clone)]
pub struct RecoveryBudget {
    max: usize,
    window: Duration,
    times: VecDeque<Instant>,
}

impl RecoveryBudget {
    /// The pet window's budget: 5 recreations in 10 minutes.
    #[must_use]
    pub fn pet() -> Self {
        Self::new(5, Duration::from_secs(600))
    }

    /// A budget of `max` in any `window`.
    #[must_use]
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            times: VecDeque::new(),
        }
    }

    /// Take one recovery at `now`, if the budget allows it.
    pub fn try_take(&mut self, now: Instant) -> bool {
        while self
            .times
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.window)
        {
            self.times.pop_front();
        }
        if self.times.len() >= self.max {
            return false;
        }
        self.times.push_back(now);
        true
    }
}
