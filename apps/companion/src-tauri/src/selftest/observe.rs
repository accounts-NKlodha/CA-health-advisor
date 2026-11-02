//! The self-test of the companion's observability (tracked work W2, architecture 10), on
//! the real app: the log is written in its folder, a resource summary of the real process
//! tree reads sensibly and reaches the log, the diagnostics zip is built from the real
//! log, saved byte for byte as previewed and holds none of the PC's own names, and a pet
//! whose webview reports a lost page is made again (a new window that loads its page,
//! keeps its non-activating styles, is shown and keeps the hotkey).
//!
//! The WebView2 failure itself is not provoked: the self-test calls the shell's handler
//! with the renderer-exited kind, the value the watch passes on (the watch's
//! registration is checked by the log having no `webview_watch_failed` line). A real
//! renderer crash on an office PC is a physical check (docs/V1_Acceptance.md, W2).

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Report, on_main, pet_visible, wait_until};
use crate::shell::{PET, Shell};

fn log_text() -> String {
    crate::diag::log_dir()
        .map(|d| d.join(hy_core::diag::log::LOG_NAME))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

fn events(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            Some(v["fields"]["event"].as_str()?.to_owned())
        })
        .collect()
}

/// Run the observability checks (last in the self-test: the pet window is made again).
pub fn observability_end_to_end(r: &mut Report, shell: &Arc<Shell>) {
    // The log.
    let text = log_text();
    let ev = events(&text);
    let unreadable = text.lines().filter(|l| !l.trim().is_empty()).count() - ev.len();
    r.check(
        "log_written",
        crate::diag::log_dir().is_some() && ev.first().map(String::as_str) == Some("start"),
        format!(
            "{} lines ({} unreadable); events {:?}",
            ev.len(),
            unreadable,
            ev.iter().take(8).collect::<Vec<_>>()
        ),
    );
    r.check(
        "webview_watch_installed",
        !ev.iter().any(|e| e == "webview_watch_failed"),
        "no webview_watch_failed line",
    );

    // The resource summary of the real tree, over 2 s.
    let prev = crate::diag::sample();
    let at = Instant::now();
    std::thread::sleep(Duration::from_secs(2));
    let (now, summary) = crate::diag::log_resources(&prev, at);
    let logged = events(&log_text()).iter().any(|e| e == "resources");
    #[cfg(windows)]
    let sane = now.processes >= 2
        && now.working_set > 0
        && now
            .private_working_set
            .is_some_and(|p| p > 0 && p <= now.working_set * 2)
        && now.handles > 0
        && now.threads > 0
        && now.unreadable == 0;
    #[cfg(not(windows))]
    let sane = true;
    r.check(
        "resource_summary",
        sane && logged && summary.cpu_pct.is_some(),
        format!(
            "{} processes ({} unreadable), working set {} MB, private working set {:?} MB, private bytes {} MB, {} handles, {} threads, CPU {:.2}%; logged {logged}",
            now.processes,
            now.unreadable,
            now.working_set >> 20,
            now.private_working_set.map(|p| p >> 20),
            now.private_bytes >> 20,
            now.handles,
            now.threads,
            summary.cpu_pct.unwrap_or(f64::NAN)
        ),
    );

    diagnostics_zip(r);
    pet_recreated(r, shell);
}

fn diagnostics_zip(r: &mut Report) {
    let Some(dir) = crate::diag::log_dir() else {
        r.check("diagnostics_zip", false, "no log folder");
        return;
    };
    let d = crate::diag::Diagnostics::default();
    let p = d.prepare_from(dir, "0.0.0-selftest");
    let staged = d.staged_zip().unwrap_or_default();
    let out = std::env::temp_dir().join(format!("hy-selftest-diag-{}.zip", std::process::id()));
    let saved = d.save_to(&out).is_ok();
    let bytes = std::fs::read(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    let files = hy_core::diag::zip::read(&bytes).unwrap_or_default();
    let same_as_preview = files.len() == p.entries.len()
        && files
            .iter()
            .zip(&p.entries)
            .all(|((n, data), e)| *n == e.name && *data == e.text.as_bytes());
    // The PC's own names (only booleans are reported, never the names).
    let all: String = files
        .iter()
        .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
        .collect();
    let lower = all.to_ascii_lowercase();
    let leaks: Vec<&str> = ["USERNAME", "COMPUTERNAME", "USERPROFILE"]
        .into_iter()
        .filter(|k| {
            std::env::var(k)
                .ok()
                .filter(|v| v.len() >= 3)
                .is_some_and(|v| lower.contains(&v.to_ascii_lowercase()))
        })
        .collect();
    let has_events = p.events.contains_key("start") && p.events.contains_key("resources");
    r.check(
        "diagnostics_zip",
        saved && staged == bytes && same_as_preview && leaks.is_empty() && has_events,
        format!(
            "{} files, {} bytes; saved {saved}; same bytes as staged {}; files as previewed {same_as_preview}; {} log lines, {} values replaced; events {:?}; names found {:?}",
            files.len(),
            bytes.len(),
            staged == bytes,
            p.kept_lines,
            p.scrubbed,
            p.events,
            leaks
        ),
    );
}

#[cfg(windows)]
fn pet_hwnd(shell: &Arc<Shell>) -> Option<isize> {
    on_main(shell, |s| {
        use tauri::Manager;
        s.app()
            .get_webview_window(PET)
            .as_ref()
            .and_then(crate::shell::hwnd_of)
    })
    .flatten()
}

#[cfg(not(windows))]
fn pet_hwnd(_shell: &Arc<Shell>) -> Option<isize> {
    None
}

fn pet_recreated(r: &mut Report, shell: &Arc<Shell>) {
    let before = pet_hwnd(shell);
    // The renderer-exited kind (1), as the watch reports it.
    let _ = on_main(shell, |s| s.webview_failed(PET, 1));
    let reloaded = wait_until(Duration::from_secs(60), || {
        shell.page_ready() && pet_hwnd(shell).is_some_and(|h| Some(h) != before)
    });
    let after = pet_hwnd(shell);
    #[cfg(windows)]
    let hardened = after.is_some_and(crate::win32::is_hardened);
    #[cfg(not(windows))]
    let hardened = true;
    let visible = wait_until(Duration::from_secs(5), || pet_visible(shell) == Some(true));
    #[cfg(windows)]
    let hotkey = on_main(shell, |s| s.settings_view().hotkey_registered) == Some(true);
    #[cfg(not(windows))]
    let hotkey = true;
    let logged = events(&log_text()).iter().any(|e| e == "webview_recreated");
    r.check(
        "pet_webview_recreated",
        reloaded && hardened && visible && hotkey && logged,
        format!(
            "new window {}, page loaded {reloaded}, non-activating {hardened}, shown {visible}, hotkey registered {hotkey}, logged {logged}",
            after.is_some() && after != before
        ),
    );
}
