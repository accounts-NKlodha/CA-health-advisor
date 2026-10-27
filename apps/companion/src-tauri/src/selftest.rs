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
//! It then checks the coaching engine end to end on a simulated clock (the engine's store
//! connection uses a `ManualClock` starting on a Monday morning, and the signal pump is
//! off): a reminder falls due and appears as the single bubble beside the pet, answering
//! it on the real bubble records the response through the store, and a simulated
//! presentation (the same engine input the Windows detector sends) hides the pet and the
//! next reminder, with nothing replayed when it ends. Finally it runs the guided movement
//! routine from the pet menu on the real bubble (Seated, then Done on each movement) and
//! checks each movement card is laid out inside the work area and the store holds three
//! `movement.step` events and a done `movement.session`. Last, water and the phone note
//! (wireframes B3 and B8) on the real bubble: "+250 ml water" from the pet menu shows the
//! confirmation, Undo stores a retraction, Details opens the water panel (laid out inside
//! the work area with its entry rows), and an entry is logged, corrected and removed
//! there; "Phone interruption" is noted, labelled and undone, each checked in the store.
//! Then the review window (wireframes W13 and W14) opens under its own capability, its page
//! loads this week's review, monthly progress reads, and the window closes. Last, the
//! settings window loads its V1-4 tabs, a larger text size is saved and reaches the pet's
//! view, and the recovery-day plan window (W15) opens under its own capability and loads.
//! The setup wizard (W1) opens and loads, shows its sample reminder beside the pet and
//! finishes; and an encrypted backup (W9) is exported and opened again with its password.
//!
//! With `HY_SELFTEST_SOAK_S` set it then runs the accelerated soak ([`soak`], slice V1-7)
//! before the camera check below.
//!
//! Last, the posture check's camera (slice V2-1) runs end to end on Chromium's synthetic
//! camera, in [`camera`]: consent, start, frames, the network refused, a hidden window,
//! the camera budget, the watchdog, a presentation hold, Off within two seconds, and a
//! refused camera with the pet still running.
//!
//! With `HY_SELFTEST_CAMERA_SOAK_S` set it then runs the accelerated camera soak
//! ([`camera_soak`], slice V2-5) on the same synthetic camera.
//!
//! A CI runner is not an office PC: focus checks that depend on an interactive desktop
//! are recorded as informational, and real Windows acceptance of focus and DPI remains a
//! UAT item (architecture 11; A6, A8).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use hy_core::clock::ManualClock;
use hy_core::coach::{Bubble as CoachBubble, CoachView, GuideStage, Input};
use hy_core::schema::{EventPayload, PhoneLabel, ReminderOutcome, RoutineOutcome, RoutineVariant};
use hy_pet::geometry::{Monitor, Rect};
use hy_pet::link::present::{BubbleKind, action};
use hy_pet::placement::{SCALE_MAX, SCALE_MIN, SCALE_STEP, bring_above_clock, pet_window_physical};
use hy_pet::profile::PetEdit;
use hy_pet::shell::{Bubble, BubbleAction, BubbleOutcome, HideReason, PetShell};
use serde::Serialize;

use crate::coach::CoachRunner;
use crate::shell::{Shell, current_monitors};

mod camera;
pub mod camera_soak;
pub mod soak;

/// Where the simulated clock starts: Monday 26 Oct 2026, 10:00 in Kolkata, inside the
/// default work hours.
pub fn sim_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 26, 4, 30, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

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
    /// The accelerated soak, if it ran (slice V1-7).
    soak: Option<soak::SoakSummary>,
    /// The accelerated camera soak, if it ran (slice V2-5).
    camera_soak: Option<camera_soak::CameraSoakSummary>,
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

/// Start the self-test on its own thread. `coach` is the engine's runner and its
/// simulated clock, if the engine started.
pub fn spawn(
    shell: Arc<Shell>,
    coach: Option<(Arc<CoachRunner>, Arc<ManualClock>)>,
    report_path: PathBuf,
    idle_after: Duration,
    soak: Option<soak::SoakPlan>,
) {
    let _ = std::thread::Builder::new()
        .name("hy-selftest".into())
        .spawn(move || {
            let mut report = run(&shell);
            match &coach {
                Some((runner, clock)) => coach_end_to_end(&mut report, &shell, runner, clock),
                None => report.check("coach_started", false, "the coaching engine did not start"),
            }
            let passed = report.checks.iter().all(|c| c.ok || !c.required);
            if let (Some(plan), Some((runner, clock)), true) = (&soak, &coach, passed) {
                let summary = soak::run(&mut report, &shell, runner, clock, plan);
                report.soak = Some(summary);
            }
            camera::camera_end_to_end(&mut report, &shell);
            if let Some(plan) = camera_soak::CameraSoakPlan::from_env() {
                if report.checks.iter().all(|c| c.ok || !c.required) {
                    let runner = coach.as_ref().map(|(r, _)| r);
                    report.camera_soak = Some(camera_soak::run(&mut report, &shell, runner, plan));
                } else {
                    report.check(
                        "camera_soak_started",
                        false,
                        "not run: a required self-test check failed first",
                    );
                }
            }
            report.ok = report.checks.iter().all(|c| c.ok || !c.required);
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
    // Display scaling (slice V1-6): the manifest declares per-monitor v2 DPI awareness, so
    // Windows never bitmap-stretches the pet or the screens.
    #[cfg(windows)]
    {
        let awareness = crate::win32::dpi_awareness();
        r.check(
            "dpi_per_monitor_aware",
            awareness == 2,
            format!("DPI awareness {awareness} (2 = per monitor)"),
        );
    }
    // Run on the installed app (CI lifecycle job): the bundled fixed WebView2 runtime is
    // the one in use, so nothing is downloaded and no system runtime is needed. Only
    // booleans are reported, never the folder (it contains the user name).
    if std::env::var_os("HY_SELFTEST_EXPECT_FIXED_RUNTIME").is_some() {
        let folder = std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").map(PathBuf::from);
        let next_to_exe = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("webview2-fixed")));
        // Tauri sets the folder from its own canonical exe path, so compare canonical
        // forms (a verbatim `\\?\` prefix names the same folder).
        let canonical =
            |p: &Option<PathBuf>| p.as_ref().and_then(|p| std::fs::canonicalize(p).ok());
        let set = folder.is_some();
        let runtime = folder
            .as_ref()
            .is_some_and(|f| f.join("msedgewebview2.exe").is_file());
        let same = canonical(&folder).is_some() && canonical(&folder) == canonical(&next_to_exe);
        let ok = set && runtime && same;
        r.check(
            "fixed_webview2_runtime",
            ok,
            format!("runtime folder set {set}, runtime there {runtime}, beside the app {same}"),
        );
    }
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
            format!("{SCALE_MIN}% to {SCALE_MAX}% in steps of {SCALE_STEP}")
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
        ..Bubble::default()
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

// ------------------------------------------------------------ coaching engine

/// Let `secs` simulated seconds pass with the user active, ticking every 5 s as the signal
/// pump does. Returns the engine's view after the last tick.
fn sim_work(runner: &CoachRunner, clock: &ManualClock, secs: u64) -> Option<CoachView> {
    let mut view = None;
    for _ in 0..secs / 5 {
        clock.advance(chrono::Duration::seconds(5));
        view = runner.call(|l| {
            let _ = l.input(Input::Tick {
                idle: Duration::ZERO,
            });
            l.view().clone()
        });
        view.as_ref()?;
    }
    view
}

/// Work until the engine's view satisfies `done`, at most `limit_min` simulated minutes.
/// Returns the view and the simulated minutes it took.
fn sim_work_until(
    runner: &CoachRunner,
    clock: &ManualClock,
    limit_min: u64,
    done: impl Fn(&CoachView) -> bool,
) -> Option<(CoachView, u64)> {
    for i in 0..limit_min * 12 {
        let v = sim_work(runner, clock, 5)?;
        if done(&v) {
            return Some((v, (i + 1) * 5 / 60));
        }
    }
    None
}

/// Today's reminder offers and responses in the engine's store.
fn reminder_events(runner: &CoachRunner) -> (Vec<String>, Vec<(String, ReminderOutcome)>) {
    runner
        .call(|l| {
            let store = l.coach().store();
            let events = store
                .today()
                .and_then(|d| store.events_on(d))
                .unwrap_or_default();
            let mut offers = Vec::new();
            let mut responses = Vec::new();
            for e in events {
                match e.payload {
                    EventPayload::ReminderOffered { reminder_id, .. } => {
                        offers.push(reminder_id.to_string());
                    }
                    EventPayload::ReminderResponse {
                        reminder_id,
                        outcome,
                        ..
                    } => responses.push((reminder_id.to_string(), outcome)),
                    _ => {}
                }
            }
            (offers, responses)
        })
        .unwrap_or_default()
}

/// The bubble the link last showed, and the engine's offer in it.
fn shown(runner: &CoachRunner) -> (Option<String>, Option<String>) {
    runner
        .call(|l| {
            let id = l.shown_bubble_id().map(str::to_owned);
            let offer = l.view().reminder().map(|(o, _)| o.to_string());
            (id, offer)
        })
        .unwrap_or_default()
}

/// The engine's bubble open beside the pet, laid out inside the work area.
fn engine_bubble_on_show(shell: &Arc<Shell>, id: &str) -> bool {
    let g = shell.pet_geometry();
    shell.open_bubble_id().as_deref() == Some(id)
        && match (g.bubble_window, g.monitor) {
            (Some(w), Some(m)) => m.work_area.contains(&w),
            _ => false,
        }
}

#[allow(clippy::too_many_lines)]
fn coach_end_to_end(
    r: &mut Report,
    shell: &Arc<Shell>,
    runner: &Arc<CoachRunner>,
    clock: &Arc<ManualClock>,
) {
    // 1. A reminder falls due on the simulated clock and is the single bubble.
    let due = sim_work_until(runner, clock, 60, |v| v.reminder().is_some());
    let (id, offer) = shown(runner);
    let parsed = id.as_deref().and_then(BubbleKind::parse);
    let id_matches = matches!(
        (parsed, &offer),
        (Some(BubbleKind::Reminder { offer: o }), Some(want)) if &o.to_string() == want
    );
    let on_show = id
        .as_deref()
        .is_some_and(|id| wait_until(Duration::from_secs(10), || engine_bubble_on_show(shell, id)));
    let (offers, responses) = reminder_events(runner);
    let open_offers = offers
        .iter()
        .filter(|o| !responses.iter().any(|(r, _)| r == *o))
        .count();
    r.check(
        "coach_reminder_due_single_bubble",
        due.is_some() && id_matches && on_show && open_offers == 1,
        format!(
            "due after {:?} simulated min ({:?}); link bubble {id:?}; shell bubble {:?}; on show inside the work area {on_show}; open offers {open_offers}",
            due.as_ref().map(|(_, m)| m),
            due.as_ref().and_then(|(v, _)| v.reminder()).map(|(_, a)| a),
            shell.open_bubble_id(),
        ),
    );

    // 2. "Already did it" on the real bubble is routed to the engine and recorded.
    if let Some(bubble_id) = id.clone() {
        let _ = on_main(shell, move |s| s.bubble_action(&bubble_id, action::DONE));
    }
    let recorded = offer.as_ref().is_some_and(|o| {
        wait_until(Duration::from_secs(10), || {
            reminder_events(runner)
                .1
                .iter()
                .any(|(r, out)| r == o && *out == ReminderOutcome::Done)
        })
    });
    let closed = wait_until(Duration::from_secs(5), || shell.open_bubble_id().is_none());
    r.check(
        "coach_answer_recorded_in_store",
        recorded && closed,
        format!("reminder.response done for {offer:?}: {recorded}; bubble closed {closed}"),
    );

    // 3. The next reminder; a presentation hides it, and nothing comes back afterwards.
    let next = sim_work_until(runner, clock, 60, |v| v.reminder().is_some());
    let (id2, offer2) = shown(runner);
    let on_show2 = id2
        .as_deref()
        .is_some_and(|id| wait_until(Duration::from_secs(10), || engine_bubble_on_show(shell, id)));
    runner.input(Input::PresentationChanged(true));
    let hid = wait_until(Duration::from_secs(5), || {
        pet_visible(shell) == Some(false) && shell.open_bubble_id().is_none()
    });
    let withdrawn = offer2.as_ref().is_some_and(|o| {
        reminder_events(runner)
            .1
            .iter()
            .any(|(r, out)| r == o && *out == ReminderOutcome::NoResponse)
    });
    let _ = sim_work(runner, clock, 10 * 60);
    let still_hidden = pet_visible(shell) == Some(false) && shell.open_bubble_id().is_none();
    runner.input(Input::PresentationChanged(false));
    let back = wait_until(Duration::from_secs(5), || pet_visible(shell) == Some(true));
    let offers_at_end = reminder_events(runner).0.len();
    let _ = sim_work(runner, clock, 4 * 60);
    std::thread::sleep(Duration::from_secs(1));
    let (link_bubble, _) = shown(runner);
    let replayed = shell.open_bubble_id().is_some()
        || link_bubble.is_some()
        || reminder_events(runner).0.len() != offers_at_end;
    r.check(
        "coach_presentation_hides_without_replay",
        next.is_some() && on_show2 && hid && withdrawn && still_hidden && back && !replayed,
        format!(
            "next reminder {offer2:?} on show {on_show2}; hidden {hid}; withdrawn as no response {withdrawn}; hidden after 10 simulated min {still_hidden}; back {back}; replayed {replayed}"
        ),
    );

    // 4. The guided movement routine from the pet menu, answered on the real bubble.
    guided_routine_end_to_end(r, shell, runner);

    // 5. Water and the phone note from the pet menu, answered on the real bubble.
    water_and_phone_end_to_end(r, shell, runner);

    // 6. The weekly review window (wireframes W13 and W14) from the tray or pet menu.
    review_window(r, shell);

    // 7. The settings tabs and the recovery-day plan window (slice V1-4).
    settings_window(r, shell);
    plan_window(r, shell);
    setup_window(r, shell);
    backup_export(r, shell);

    // 8. The engine's state is saved (as on exit).
    let saved = runner.call(|l| l.flush().is_ok()) == Some(true);
    r.check("coach_state_saved", saved, "flush of the engine state");
}

/// The guide's stage and the bubble the link last showed.
fn guide_shown(runner: &CoachRunner) -> (Option<GuideStage>, Option<String>) {
    runner
        .call(|l| {
            (
                l.view().guide.map(|g| g.stage),
                l.shown_bubble_id().map(str::to_owned),
            )
        })
        .unwrap_or_default()
}

/// Today's routine events: (steps answered, the routine results).
fn routine_events(runner: &CoachRunner) -> (usize, Vec<(RoutineOutcome, Option<RoutineVariant>)>) {
    runner
        .call(|l| {
            let store = l.coach().store();
            let events = store
                .today()
                .and_then(|d| store.events_on(d))
                .unwrap_or_default();
            let mut steps = 0;
            let mut results = Vec::new();
            for e in events {
                match e.payload {
                    EventPayload::MovementStep { .. } => steps += 1,
                    EventPayload::MovementSession {
                        outcome, variant, ..
                    } => results.push((outcome, variant)),
                    _ => {}
                }
            }
            (steps, results)
        })
        .unwrap_or_default()
}

/// Press `action_id` on the shown guide card and wait for the guide to reach `next`, laid
/// out beside the pet inside the work area. Returns whether it did.
fn guide_press(
    shell: &Arc<Shell>,
    runner: &CoachRunner,
    action_id: &'static str,
    next: GuideStage,
) -> bool {
    let (_, id) = guide_shown(runner);
    let Some(id) = id else {
        return false;
    };
    let _ = on_main(shell, move |s| s.bubble_action(&id, action_id));
    wait_until(Duration::from_secs(10), || {
        let (stage, id) = guide_shown(runner);
        stage == Some(next) && id.is_some_and(|id| engine_bubble_on_show(shell, &id))
    })
}

fn guided_routine_end_to_end(r: &mut Report, shell: &Arc<Shell>, runner: &CoachRunner) {
    let _ = on_main(shell, |s| s.menu_action(action::MOVEMENT_START));
    let intro = wait_until(Duration::from_secs(10), || {
        let (stage, id) = guide_shown(runner);
        stage == Some(GuideStage::Intro)
            && id.is_some_and(|id| {
                BubbleKind::parse(&id) == Some(BubbleKind::MovementGuide)
                    && engine_bubble_on_show(shell, &id)
            })
    });
    let mut cards = vec![("intro", intro)];
    cards.push((
        "movement 1",
        guide_press(
            shell,
            runner,
            action::SEATED,
            GuideStage::Step { index: 0, total: 3 },
        ),
    ));
    cards.push((
        "movement 2",
        guide_press(
            shell,
            runner,
            action::STEP_DONE,
            GuideStage::Step { index: 1, total: 3 },
        ),
    ));
    cards.push((
        "movement 3",
        guide_press(
            shell,
            runner,
            action::STEP_DONE,
            GuideStage::Step { index: 2, total: 3 },
        ),
    ));
    cards.push((
        "result",
        guide_press(
            shell,
            runner,
            action::STEP_DONE,
            GuideStage::Result {
                outcome: RoutineOutcome::Done,
                done: 3,
                skipped: 0,
                total: 3,
                stopped: false,
            },
        ),
    ));
    let (steps, results) = routine_events(runner);
    let stored = steps == 3 && results == [(RoutineOutcome::Done, Some(RoutineVariant::Seated))];
    // "OK" closes the result.
    if let (_, Some(id)) = guide_shown(runner) {
        let _ = on_main(shell, move |s| s.bubble_action(&id, action::OK));
    }
    let closed = wait_until(Duration::from_secs(5), || {
        guide_shown(runner).0.is_none() && shell.open_bubble_id().is_none()
    });
    r.check(
        "coach_guided_routine_recorded",
        cards.iter().all(|(_, ok)| *ok) && stored && closed,
        format!(
            "cards shown inside the work area {cards:?}; movement.step events {steps}; results {results:?}; closed {closed}"
        ),
    );
}

// ------------------------------------------------------- water and phone (B3, B8)

/// The engine's card and the bubble id the link last showed for it.
fn card(runner: &CoachRunner) -> (Option<CoachBubble>, Option<String>) {
    runner
        .call(|l| (l.view().bubble, l.shown_bubble_id().map(str::to_owned)))
        .unwrap_or_default()
}

/// Today's current water amounts, the phone notes standing, and the retractions stored.
fn entries(runner: &CoachRunner) -> (Vec<u32>, Vec<PhoneLabel>, usize) {
    runner
        .call(|l| {
            let store = l.coach().store();
            let Ok(day) = store.today() else {
                return (Vec::new(), Vec::new(), 0);
            };
            let water = store
                .water_entries_on(day)
                .unwrap_or_default()
                .into_iter()
                .map(|e| e.amount_ml)
                .collect();
            let phone = store
                .current_events_on(day)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|e| match e.payload {
                    EventPayload::PhoneInterruption { label } => Some(label),
                    _ => None,
                })
                .collect();
            let retracted = store
                .events_on(day)
                .unwrap_or_default()
                .iter()
                .filter(|e| matches!(e.payload, EventPayload::EntryRetracted {}))
                .count();
            (water, phone, retracted)
        })
        .unwrap_or_default()
}

/// Wait until the engine's card is `kind`, is not the card shown as `old`, and is laid out
/// beside the pet inside the work area.
fn card_on_show_after(
    shell: &Arc<Shell>,
    runner: &CoachRunner,
    kind: BubbleKind,
    old: Option<&str>,
) -> bool {
    wait_until(Duration::from_secs(10), || {
        let (_, id) = card(runner);
        id.is_some_and(|id| {
            Some(id.as_str()) != old
                && BubbleKind::parse(&id) == Some(kind)
                && engine_bubble_on_show(shell, &id)
        })
    })
}

fn card_on_show(shell: &Arc<Shell>, runner: &CoachRunner, kind: BubbleKind) -> bool {
    card_on_show_after(shell, runner, kind, None)
}

/// Press `action_id` on the card on show, then wait for the stored entries to match
/// `want` and the redrawn card (each answer changes it, so it gets a new id) of `kind`
/// to be on show, so the next press is made on it.
fn card_press(
    shell: &Arc<Shell>,
    runner: &CoachRunner,
    action_id: String,
    kind: BubbleKind,
    want: impl Fn(&(Vec<u32>, Vec<PhoneLabel>, usize)) -> bool,
) -> bool {
    let (_, id) = card(runner);
    let Some(id) = id else {
        return false;
    };
    let pressed = id.clone();
    let _ = on_main(shell, move |s| s.bubble_action(&id, &action_id));
    wait_until(Duration::from_secs(10), || want(&entries(runner)))
        && card_on_show_after(shell, runner, kind, Some(&pressed))
}

/// The id of today's water entry in the panel, if exactly one stands.
fn only_entry(runner: &CoachRunner) -> Option<String> {
    runner
        .call(|l| match l.view().water_entries.as_slice() {
            [e] => Some(e.id.to_string()),
            _ => None,
        })
        .flatten()
}

fn water_and_phone_end_to_end(r: &mut Report, shell: &Arc<Shell>, runner: &CoachRunner) {
    let (_, _, retracted0) = entries(runner);
    // The quick log and its Undo.
    let _ = on_main(shell, |s| s.menu_action(action::WATER));
    let logged = wait_until(Duration::from_secs(10), || entries(runner).0 == [250])
        && card_on_show(shell, runner, BubbleKind::WaterLogged);
    let undone = card_press(
        shell,
        runner,
        action::UNDO.to_owned(),
        BubbleKind::WaterLogged,
        |e| e.0.is_empty() && e.2 == retracted0 + 1,
    );
    // Details: the panel, then a serving, a correction and a removal.
    let panel = card_press(
        shell,
        runner,
        action::DETAILS.to_owned(),
        BubbleKind::WaterPanel,
        |e| e.0.is_empty(),
    );
    let added = card_press(
        shell,
        runner,
        format!("{}500", action::ADD_PREFIX),
        BubbleKind::WaterPanel,
        |e| e.0 == [500],
    );
    let edit = only_entry(runner).is_some_and(|id| {
        card_press(
            shell,
            runner,
            format!("{}{id}", action::ENTRY_EDIT_PREFIX),
            BubbleKind::WaterPanel,
            |e| e.0 == [500],
        )
    });
    let more = card_press(
        shell,
        runner,
        action::EDIT_MORE.to_owned(),
        BubbleKind::WaterPanel,
        |e| e.0 == [500],
    ) && wait_until(
        Duration::from_secs(5),
        || matches!(card(runner).0, Some(CoachBubble::WaterPanel { edit: Some(e), .. }) if e.ml == 550),
    );
    let corrected = card_press(
        shell,
        runner,
        action::EDIT_SAVE.to_owned(),
        BubbleKind::WaterPanel,
        |e| e.0 == [550],
    );
    let removed = only_entry(runner).is_some_and(|id| {
        card_press(
            shell,
            runner,
            format!("{}{id}", action::ENTRY_REMOVE_PREFIX),
            BubbleKind::WaterPanel,
            |e| e.0.is_empty() && e.2 == retracted0 + 2,
        )
    });
    let _ = on_main(shell, |s| s.bubble_close());
    let closed = wait_until(Duration::from_secs(5), || shell.open_bubble_id().is_none());
    let steps = [
        ("logged", logged),
        ("undo", undone),
        ("panel", panel),
        ("add", added),
        ("edit", edit),
        ("step", more),
        ("save", corrected),
        ("remove", removed),
        ("closed", closed),
    ];
    r.check(
        "water_panel_end_to_end",
        steps.iter().all(|(_, ok)| *ok),
        format!(
            "cards inside the work area and stored entries {steps:?}; now {:?}",
            entries(runner)
        ),
    );

    // The phone note, a label, then Undo.
    let _ = on_main(shell, |s| s.menu_action(action::PHONE_NOTE));
    let noted = wait_until(Duration::from_secs(10), || {
        entries(runner).1 == [PhoneLabel::Unlabelled]
    }) && card_on_show(shell, runner, BubbleKind::PhoneNote);
    let labelled = card_press(
        shell,
        runner,
        format!("{}work", action::LABEL_PREFIX),
        BubbleKind::PhoneNote,
        |e| e.1 == [PhoneLabel::Work],
    );
    let undone = card_press(
        shell,
        runner,
        action::UNDO.to_owned(),
        BubbleKind::PhoneNote,
        |e| e.1.is_empty() && e.2 == retracted0 + 3,
    );
    let _ = on_main(shell, |s| s.bubble_close());
    let closed = wait_until(Duration::from_secs(5), || shell.open_bubble_id().is_none());
    let steps = [
        ("noted", noted),
        ("labelled", labelled),
        ("undo", undone),
        ("closed", closed),
    ];
    r.check(
        "phone_note_end_to_end",
        steps.iter().all(|(_, ok)| *ok),
        format!(
            "cards inside the work area and stored notes {steps:?}; now {:?}",
            entries(runner)
        ),
    );
}

/// Open the review window, wait for its page to load the week through its capability,
/// read monthly progress, then close it.
fn review_window(r: &mut Report, shell: &Arc<Shell>) {
    use tauri::Manager;
    let _ = on_main(shell, |s| s.open_review());
    let opened = wait_until(Duration::from_secs(10), || {
        on_main(shell, |s| {
            s.app().get_webview_window(crate::shell::REVIEW).is_some()
        }) == Some(true)
    });
    let loaded = wait_until(Duration::from_secs(30), || shell.review_loaded());
    let offer = offer_navigation(shell);
    let week = shell.review_week(None);
    let week_ok = week.as_ref().is_ok_and(|p| p.review.days.len() == 7);
    let month = shell.review_month(None);
    let month_ok = month.as_ref().is_ok_and(|p| !p.review.bars.is_empty());
    let _ = on_main(shell, |s| {
        if let Some(w) = s.app().get_webview_window(crate::shell::REVIEW) {
            let _ = w.close();
        }
    });
    let closed = wait_until(Duration::from_secs(10), || {
        on_main(shell, |s| {
            s.app().get_webview_window(crate::shell::REVIEW).is_none()
        }) == Some(true)
    });
    let week_detail = match &week {
        Ok(p) => format!(
            "{:?}, {} of {} workdays recorded",
            p.review.status, p.review.coverage.recorded, p.review.coverage.workdays
        ),
        Err(e) => e.clone(),
    };
    let month_detail = match &month {
        Ok(p) => format!("{:?}, {} months", p.review.status, p.review.bars.len()),
        Err(e) => e.clone(),
    };
    r.check(
        "review_window",
        opened && loaded && week_ok && month_ok && closed,
        format!(
            "opened {opened}; page loaded {loaded}; week {week_detail}; month {month_detail}; closed {closed}"
        ),
    );
    r.check(
        "review_offer_week",
        offer.0,
        format!("open window, then a new one: {}", offer.1),
    );
}

/// "Open" on the weekly review offer goes to its week: in the review window already open
/// (told by an event, it asks for that week), then in a new window (whose first request
/// is that week, never this week first). Leaves the review window open.
fn offer_navigation(shell: &Arc<Shell>) -> (bool, String) {
    use chrono::Days;
    let Some(week) = shell
        .review_week(None)
        .ok()
        .and_then(|p| p.review.start.checked_sub_days(Days::new(7)))
    else {
        return (false, "no week to offer".into());
    };
    let _ = shell.take_review_requests();
    let _ = on_main(shell, move |s| s.open_review_on(week));
    let open_ok = wait_until(Duration::from_secs(10), || {
        let asked = shell.take_review_requests();
        asked.contains(&Some(week))
    });
    let _ = on_main(shell, |s| {
        use tauri::Manager;
        if let Some(w) = s.app().get_webview_window(crate::shell::REVIEW) {
            let _ = w.close();
        }
    });
    let closed = wait_until(Duration::from_secs(10), || {
        !window_open(shell, crate::shell::REVIEW)
    });
    let _ = shell.take_review_requests();
    let _ = on_main(shell, move |s| s.open_review_on(week));
    let mut first = None;
    let new_ok = wait_until(Duration::from_secs(30), || {
        if first.is_none() {
            first = shell.take_review_requests().first().copied();
        }
        first.is_some()
    }) && first == Some(Some(week));
    let taken = shell.review_target().is_none();
    (
        open_ok && closed && new_ok && taken,
        format!(
            "offered {week}; open window asked for it {open_ok}; closed {closed}; new window's first request {first:?}; target taken {taken}"
        ),
    )
}

/// Whether the window `label` is open, asked on the main thread.
fn window_open(shell: &Arc<Shell>, label: &'static str) -> bool {
    use tauri::Manager;
    on_main(shell, move |s| s.app().get_webview_window(label).is_some()) == Some(true)
}

/// Close the window `label`, if open, and wait for it to go.
fn close_window(shell: &Arc<Shell>, label: &'static str) -> bool {
    use tauri::Manager;
    let _ = on_main(shell, move |s| {
        if let Some(w) = s.app().get_webview_window(label) {
            let _ = w.close();
        }
    });
    wait_until(Duration::from_secs(10), || !window_open(shell, label))
}

/// Open the settings window, wait for its page to load the V1-4 tabs, save a larger
/// text size and check the pet's view follows it, put it back, then close the window.
fn settings_window(r: &mut Report, shell: &Arc<Shell>) {
    use hy_core::prefs::Accessibility;
    let _ = on_main(shell, |s| s.open_settings());
    let opened = wait_until(Duration::from_secs(10), || {
        window_open(shell, crate::shell::SCREENS)
    });
    let loaded = wait_until(Duration::from_secs(30), || shell.settings_loaded());
    let page = shell.settings_page();
    let page_ok = page
        .as_ref()
        .is_ok_and(|p| !p.zones.is_empty() && !p.programme.is_empty());
    let larger = Accessibility {
        text_pct: 125,
        ..Accessibility::default()
    };
    let saved = on_main(shell, move |s| s.save_accessibility(larger).is_ok()) == Some(true);
    let followed = shell.display_prefs().text_pct == 125;
    let restored = on_main(shell, |s| {
        s.save_accessibility(Accessibility::default()).is_ok()
    }) == Some(true);
    let closed = close_window(shell, crate::shell::SCREENS);
    let detail = match &page {
        Ok(p) => format!(
            "{} zones, {} programme days",
            p.zones.len(),
            p.programme.len()
        ),
        Err(e) => e.clone(),
    };
    r.check(
        "settings_window",
        opened && loaded && page_ok && saved && followed && restored && closed,
        format!(
            "opened {opened}; page loaded {loaded}; {detail}; text size saved {saved}; pet followed {followed}; restored {restored}; closed {closed}"
        ),
    );
}

/// Open the recovery-day plan window, wait for its page to load, then close it.
fn plan_window(r: &mut Report, shell: &Arc<Shell>) {
    let _ = on_main(shell, |s| s.open_plan());
    let opened = wait_until(Duration::from_secs(10), || {
        window_open(shell, crate::shell::PLAN)
    });
    let loaded = wait_until(Duration::from_secs(30), || shell.plan_loaded());
    let page = shell.plan_page();
    let page_ok = page.as_ref().is_ok_and(|p| !p.dates.is_empty());
    let _ = on_main(shell, |s| s.close_plan());
    let closed = wait_until(Duration::from_secs(10), || {
        !window_open(shell, crate::shell::PLAN)
    });
    let detail = match &page {
        Ok(p) => format!("{} dates, draft for {}", p.dates.len(), p.draft.date),
        Err(e) => e.clone(),
    };
    r.check(
        "plan_window",
        opened && loaded && page_ok && closed,
        format!("opened {opened}; page loaded {loaded}; {detail}; closed {closed}"),
    );
}

/// Open the setup wizard, wait for its page to load, show the sample reminder beside
/// the pet, then Finish (which records setup as done and closes the wizard).
fn setup_window(r: &mut Report, shell: &Arc<Shell>) {
    let _ = on_main(shell, |s| s.open_setup());
    let opened = wait_until(Duration::from_secs(10), || {
        window_open(shell, crate::shell::SETUP)
    });
    let loaded = wait_until(Duration::from_secs(30), || shell.setup_loaded());
    let _ = on_main(shell, |s| s.setup_sample());
    let sample = wait_until(Duration::from_secs(5), || {
        shell.notice_id().as_deref() == Some("setup-sample")
    });
    let finished = on_main(shell, |s| s.setup_finish().is_ok()) == Some(true);
    let closed = wait_until(Duration::from_secs(10), || {
        !window_open(shell, crate::shell::SETUP)
    });
    let recorded = !shell.setup_needed();
    r.check(
        "setup_window",
        opened && loaded && sample && finished && closed && recorded,
        format!(
            "opened {opened}; page loaded {loaded}; sample reminder shown {sample}; finished {finished}; closed {closed}; recorded {recorded}"
        ),
    );
}

/// Export an encrypted backup into the temp folder with the real key cost, open it again
/// with its password, check a wrong password is refused, and remove the file.
fn backup_export(r: &mut Report, shell: &Arc<Shell>) {
    use hy_core::backup::Contents;
    let dir = std::env::temp_dir().join(format!("hy-selftest-backup-{}", std::process::id()));
    let started = std::time::Instant::now();
    let done = shell.export_backup_to(&dir, "self-test password");
    let took = started.elapsed();
    let (ok, detail) = match &done {
        Ok(d) => {
            let bytes = std::fs::read(&d.path).unwrap_or_default();
            let opened = Contents::open(&bytes, "self-test password");
            let refused = Contents::open(&bytes, "not the password").is_err();
            let events = opened.as_ref().map_or(0, |c| c.rows("events"));
            let ok = opened.is_ok() && refused && events == d.summary.events && events > 0;
            (
                ok,
                format!(
                    "{} bytes, {} events, {} days; opened {}; wrong password refused {refused}; {} ms",
                    bytes.len(),
                    d.summary.events,
                    d.summary.days,
                    opened.is_ok(),
                    took.as_millis()
                ),
            )
        }
        Err(e) => (false, e.clone()),
    };
    let _ = std::fs::remove_dir_all(&dir);
    r.check("backup_export", ok, detail);
}
