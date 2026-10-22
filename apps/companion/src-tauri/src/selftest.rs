//! A scripted self-test of the real pet shell, run by the Windows CI job:
//! `HY_SELFTEST=<report.json> hy-companion.exe`.
//!
//! It starts the app normally (with a throw-away store in the temp folder), waits for the
//! pet page to load, then checks on the real windows: the pet window's non-activating
//! styles, placement inside the work area at every size, "Bring pet above clock",
//! hide and show without a focus change, presentation hiding through the engine API,
//! a bubble laid out inside the work area, the pet menu on click, the hotkey, and the
//! keyboard panel's focus hand-back. It writes a JSON report and exits with 0 when every
//! required check passed.
//!
//! A CI runner is not an office PC: focus checks that depend on an interactive desktop
//! are recorded as informational, and real Windows acceptance of focus and DPI remains a
//! UAT item (architecture 11; A6, A8).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hy_pet::geometry::{Monitor, Rect};
use hy_pet::placement::{SCALE_MAX, SCALE_MIN, SCALE_STEP, bring_above_clock, pet_window_physical};
use hy_pet::profile::PetEdit;
use hy_pet::shell::{Bubble, BubbleAction, BubbleOutcome, HideReason, PetShell};
use serde::Serialize;

use crate::shell::{Shell, current_monitors};

#[derive(Debug, Serialize)]
struct Check {
    name: &'static str,
    ok: bool,
    required: bool,
    detail: String,
}

#[derive(Debug, Default, Serialize)]
struct Report {
    ok: bool,
    checks: Vec<Check>,
    monitors: Vec<Monitor>,
    webview2: String,
}

impl Report {
    fn check(&mut self, name: &'static str, ok: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name,
            ok,
            required: true,
            detail: detail.into(),
        });
    }

    fn info(&mut self, name: &'static str, ok: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name,
            ok,
            required: false,
            detail: detail.into(),
        });
    }
}

/// Run `f` on the main thread and wait for its result.
fn on_main<T: Send + 'static>(
    shell: &Arc<Shell>,
    f: impl FnOnce(&Arc<Shell>) -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    shell.post(move |s| {
        let _ = tx.send(f(s));
    });
    rx.recv_timeout(Duration::from_secs(15)).ok()
}

fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    f()
}

#[cfg(windows)]
fn pet_visible(shell: &Arc<Shell>) -> Option<bool> {
    on_main(shell, |s| {
        use tauri::Manager;
        s.app()
            .get_webview_window(crate::shell::PET)
            .as_ref()
            .and_then(crate::shell::hwnd_of)
            .is_some_and(crate::win32::is_visible)
    })
}

#[cfg(not(windows))]
fn pet_visible(shell: &Arc<Shell>) -> Option<bool> {
    on_main(shell, |s| s.handle().is_visible())
}

/// The pet window's real rectangle as the operating system reports it (physical pixels),
/// read on the main thread: what the window-position calls actually produced.
fn real_window_rect(shell: &Arc<Shell>) -> Option<Rect> {
    on_main(shell, |s| {
        use tauri::Manager;
        let w = s.app().get_webview_window(crate::shell::PET)?;
        let pos = w.outer_position().ok()?;
        let size = w.outer_size().ok()?;
        Some(Rect::new(
            pos.x,
            pos.y,
            i32::try_from(size.width).ok()?,
            i32::try_from(size.height).ok()?,
        ))
    })
    .flatten()
}

fn foreground() -> isize {
    #[cfg(windows)]
    {
        crate::win32::foreground()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Start the self-test on its own thread.
pub fn spawn(shell: Arc<Shell>, report_path: PathBuf, idle_after: Duration) {
    let _ = std::thread::Builder::new()
        .name("hy-selftest".into())
        .spawn(move || {
            let report = run(&shell);
            std::thread::sleep(idle_after);
            let code = i32::from(!report.ok);
            let text = serde_json::to_string_pretty(&report).unwrap_or_default();
            if let Err(e) = std::fs::write(&report_path, text) {
                eprintln!("HealthierYou self-test: cannot write report: {e}");
            }
            shell.app().exit(code);
        });
}

#[allow(clippy::too_many_lines)]
fn run(shell: &Arc<Shell>) -> Report {
    let mut r = Report {
        webview2: tauri::webview_version().unwrap_or_default(),
        ..Report::default()
    };
    let loaded = wait_until(Duration::from_secs(60), || shell.page_ready());
    r.check(
        "pet_page_loaded",
        loaded,
        "pet.html loaded under the CSP and called pet_bootstrap and pet_ready through its capability",
    );
    let monitors = on_main(shell, |s| current_monitors(s.app())).unwrap_or_default();
    r.monitors.clone_from(&monitors);
    r.check(
        "monitors_reported",
        !monitors.is_empty(),
        format!("{} monitor(s)", monitors.len()),
    );

    #[cfg(windows)]
    {
        let styled = on_main(shell, |s| {
            use tauri::Manager;
            s.app()
                .get_webview_window(crate::shell::PET)
                .as_ref()
                .and_then(crate::shell::hwnd_of)
                .map(|h| (crate::win32::is_hardened(h), crate::win32::ex_style(h)))
        })
        .flatten();
        r.check(
            "pet_window_noactivate_toolwindow",
            styled.is_some_and(|(ok, _)| ok),
            format!("extended style {:#x?}", styled.map(|(_, ex)| ex)),
        );
    }

    // Inside the work area: the pet, the laid-out bubble window, the rectangle given to
    // the window-position call, and the window's real rectangle from the system.
    let inside = |shell: &Arc<Shell>| {
        let g = shell.pet_geometry();
        let real = real_window_rect(shell);
        match (g.pet, g.monitor) {
            (Some(p), Some(m)) => {
                let wa = m.work_area;
                let ok = wa.contains(&p)
                    && g.bubble_window.is_none_or(|w| wa.contains(&w))
                    && g.applied.is_some_and(|a| wa.contains(&a))
                    && real.is_some_and(|r| wa.contains(&r));
                (
                    ok,
                    format!(
                        "pet {p:?} bubble window {:?} applied {:?} real {real:?} work area {wa:?}",
                        g.bubble_window, g.applied
                    ),
                    Some((p, m)),
                )
            }
            _ => (false, "no placement".into(), None),
        }
    };
    let laid_out = |shell: &Arc<Shell>| shell.pet_geometry().bubble_window.is_some();

    // Fresh store: the pet starts at home, above the clock.
    let home = bring_above_clock(&monitors, 100).map(|p| p.rect);
    let (ok, detail, now) = inside(shell);
    r.check(
        "start_position_above_clock",
        ok && now.map(|(p, _)| p) == home,
        format!("{detail}; expected home {home:?}"),
    );

    // Every size keeps the pet inside the work area and at the right physical size.
    let mut size_failures = Vec::new();
    let mut pct = SCALE_MIN;
    while pct <= SCALE_MAX {
        let res = on_main(shell, move |s| s.edit(&PetEdit::Size { pct }).map(|_| ()));
        let (ok, detail, now) = inside(shell);
        let size_ok = now.is_some_and(|(p, m)| p.width == pet_window_physical(pct, &m));
        if !(ok && size_ok && matches!(res, Some(Ok(())))) {
            size_failures.push(format!("{pct}%: {detail}"));
        }
        pct += SCALE_STEP;
    }
    r.check(
        "every_size_inside_work_area",
        size_failures.is_empty(),
        if size_failures.is_empty() {
            "75% to 150% in steps of 5".to_owned()
        } else {
            size_failures.join("; ")
        },
    );
    let persisted = on_main(shell, |s| s.settings_view().size_pct);
    r.check(
        "size_persisted",
        persisted == Some(SCALE_MAX),
        format!("{persisted:?}"),
    );

    // Lose the pet, then bring it back above the clock without taking focus.
    let fg_before = foreground();
    let lost = monitors
        .first()
        .map(|m| (m.work_area.right() - 20, m.work_area.y));
    if let Some((x, y)) = lost {
        let _ = on_main(shell, move |s| {
            s.edit(&PetEdit::Edge {
                edge: hy_core::schema::PetEdge::Free,
            })
            .and_then(|_| s.edit(&PetEdit::Position { x, y }))
        });
    }
    let _ = on_main(shell, |s| s.hide_pet());
    let _ = on_main(shell, |s| s.bring_above_clock());
    let expected = bring_above_clock(&monitors, SCALE_MAX).map(|p| p.rect);
    let (ok, detail, now) = inside(shell);
    r.check(
        "bring_pet_above_clock",
        ok && now.map(|(p, _)| p) == expected && pet_visible(shell) == Some(true),
        format!("{detail}; expected {expected:?}"),
    );

    // Hide and show from the tray: visibility changes, focus does not.
    let _ = on_main(shell, |s| s.toggle_from_tray());
    let hidden = pet_visible(shell) == Some(false);
    let _ = on_main(shell, |s| s.toggle_from_tray());
    let shown = pet_visible(shell) == Some(true);
    r.check(
        "tray_hide_and_show",
        hidden && shown,
        format!("hidden {hidden}, shown {shown}"),
    );

    // Presentation hiding through the engine's interface: nothing shown, nothing replayed.
    let handle = shell.handle();
    handle.hide(HideReason::Presentation);
    let hid = wait_until(Duration::from_secs(5), || pet_visible(shell) == Some(false));
    let suppressed = handle.show_bubble(Bubble::notice("selftest-hidden", "Hidden test"));
    handle.unhide(HideReason::Presentation);
    let back = wait_until(Duration::from_secs(5), || pet_visible(shell) == Some(true));
    let replayed = laid_out(shell);
    r.check(
        "presentation_hide_api",
        hid && suppressed == BubbleOutcome::Suppressed && back && !replayed,
        format!("hidden {hid}, outcome {suppressed:?}, back {back}, replayed {replayed}"),
    );

    // A bubble from the engine is laid out beside the pet inside the work area.
    let bubble = Bubble {
        id: "selftest".into(),
        title: "Self-test bubble".into(),
        lines: vec!["Laid out beside the pet, inside the work area.".into()],
        chips: vec![],
        actions: vec![BubbleAction {
            id: "ok".into(),
            label: "OK".into(),
            style: hy_pet::shell::ActionStyle::Primary,
        }],
    };
    let outcome = handle.show_bubble(bubble);
    let measured = wait_until(Duration::from_secs(10), || laid_out(shell));
    let (ok, detail, _) = inside(shell);
    r.check(
        "bubble_inside_work_area",
        outcome == BubbleOutcome::Shown && measured && ok,
        format!("{outcome:?}, measured {measured}; {detail}"),
    );

    // With the engine's bubble open, move the pet to the top-left corner and bring it
    // home: the bubble stays open and moves with the pet, and the real window stays
    // inside the work area (review finding on PR 7).
    let corner = monitors.first().map(|m| (m.work_area.x, m.work_area.y));
    if let Some((x, y)) = corner {
        let _ = on_main(shell, move |s| s.edit(&PetEdit::Position { x, y }));
    }
    let moved_ok = wait_until(Duration::from_secs(5), || {
        inside(shell).0 && laid_out(shell)
    });
    let _ = on_main(shell, |s| s.bring_above_clock());
    let home = bring_above_clock(&monitors, SCALE_MAX).map(|p| p.rect);
    let back = wait_until(Duration::from_secs(5), || {
        let g = shell.pet_geometry();
        g.pet == home
            && g.bubble_window
                .is_some_and(|w| home.is_some_and(|h| w.contains(&h)))
    });
    let (ok, detail, _) = inside(shell);
    let still_open = laid_out(shell);
    r.check(
        "bring_home_with_engine_bubble_open",
        moved_ok && back && ok && still_open,
        format!("moved {moved_ok}, home with bubble {back}, open {still_open}; {detail}; expected pet {home:?}"),
    );
    handle.close_bubble("selftest");
    let closed = wait_until(Duration::from_secs(5), || !laid_out(shell));
    r.check(
        "bubble_closed_by_engine",
        closed,
        "window back to the pet alone",
    );

    // A click with no engine opens the pet menu; a second click closes it.
    let _ = on_main(shell, |s| s.pet_click());
    let menu = wait_until(Duration::from_secs(10), || laid_out(shell));
    let _ = on_main(shell, |s| s.pet_click());
    let menu_closed = wait_until(Duration::from_secs(5), || !laid_out(shell));
    r.check(
        "pet_click_menu",
        menu && menu_closed,
        format!("opened {menu}, closed {menu_closed}"),
    );
    let fg_after = foreground();
    r.check(
        "no_focus_change_from_pet",
        fg_before == fg_after,
        format!("foreground before {fg_before:#x}, after {fg_after:#x}"),
    );

    #[cfg(windows)]
    {
        let registered = on_main(shell, |s| s.settings_view().hotkey_registered);
        r.check(
            "panel_hotkey_registered",
            registered == Some(true),
            "Ctrl+Alt+H registered with RegisterHotKey",
        );
    }

    // The keyboard panel takes focus and Escape hands it back (informational on CI).
    let before = foreground();
    let _ = on_main(shell, |s| s.open_panel());
    std::thread::sleep(Duration::from_secs(2));
    let panel_fg = foreground();
    let prev = on_main(shell, |s| s.panel_prev_foreground()).unwrap_or(0);
    let _ = on_main(shell, |s| s.close_panel(true));
    std::thread::sleep(Duration::from_secs(1));
    let after = foreground();
    r.info(
        "panel_focus_round_trip",
        panel_fg != before && (prev == 0 || after == prev),
        format!(
            "before {before:#x}, with panel {panel_fg:#x}, remembered {prev:#x}, after Escape {after:#x}; real focus return is UAT (A8)"
        ),
    );

    r.ok = r.checks.iter().all(|c| c.ok || !c.required);
    r
}
