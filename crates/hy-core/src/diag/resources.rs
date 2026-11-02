//! The hourly resource summary (architecture 10): what the companion's own process tree
//! (the app and the WebView2 processes it started) used, as one `resources` log line.
//!
//! The field names are the ones `scripts/windows/Test-Soak.ps1` uses for its outside
//! samples (`working_set_mb`, `private_ws_mb`, `private_bytes_mb`, `handles`, `threads`,
//! `cpu_pct`), so the two can be read side by side. The private working set is the
//! app's own memory that the budgets count (DEC14).

use std::time::Duration;

use super::V;

/// One reading of the process tree, summed over its processes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeSample {
    /// Processes counted.
    pub processes: u32,
    /// Working set, bytes.
    pub working_set: u64,
    /// Private working set, bytes; `None` if a process of the tree could not report it
    /// (Windows before 10 1809), so a partial sum is never shown as the whole.
    pub private_working_set: Option<u64>,
    /// Private bytes (commit charge).
    pub private_bytes: u64,
    /// Open handles.
    pub handles: u64,
    /// Threads.
    pub threads: u64,
    /// Kernel plus user CPU time since each process started.
    pub cpu: Duration,
}

/// One summary line's numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// The reading at the end of the window.
    pub now: TreeSample,
    /// CPU used over the window, percent of the whole PC (all logical processors);
    /// `None` for a window of no length.
    pub cpu_pct: Option<f64>,
    /// The window's length.
    pub window: Duration,
    /// Time since the app started.
    pub uptime: Duration,
}

#[allow(clippy::cast_precision_loss)]
fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

impl Summary {
    /// The summary of the window from `prev` to `now`, `window` long, on a PC with
    /// `cores` logical processors. CPU time of a process that ended during the window
    /// drops out of the sum, so the percentage is never negative but may read low then.
    #[must_use]
    pub fn between(
        prev: &TreeSample,
        now: &TreeSample,
        window: Duration,
        cores: u32,
        uptime: Duration,
    ) -> Self {
        let wall = window.as_secs_f64() * f64::from(cores.max(1));
        let cpu_pct =
            (wall > 0.0).then(|| now.cpu.saturating_sub(prev.cpu).as_secs_f64() / wall * 100.0);
        Self {
            now: *now,
            cpu_pct,
            window,
            uptime,
        }
    }

    /// The log line's fields, megabytes and percentages rounded to two decimals.
    #[must_use]
    pub fn fields(&self) -> Vec<(&'static str, V)> {
        let n = &self.now;
        let mut f = vec![
            ("processes", V::N(u64::from(n.processes))),
            ("working_set_mb", V::Dec(mb(n.working_set))),
        ];
        if let Some(p) = n.private_working_set {
            f.push(("private_ws_mb", V::Dec(mb(p))));
        }
        f.push(("private_bytes_mb", V::Dec(mb(n.private_bytes))));
        f.push(("handles", V::N(n.handles)));
        f.push(("threads", V::N(n.threads)));
        if let Some(c) = self.cpu_pct {
            f.push(("cpu_pct", V::Dec(c)));
        }
        f.push(("window_s", V::N(self.window.as_secs())));
        f.push(("uptime_s", V::N(self.uptime.as_secs())));
        f
    }
}

/// How often the summary is written.
pub const EVERY: Duration = Duration::from_secs(3600);
