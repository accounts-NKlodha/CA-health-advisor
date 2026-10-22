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
//! `movement.step` events and a done `movement.session`.
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
use hy_core::coach::{CoachView, GuideStage, Input};
use hy_core::schema::{EventPayload, ReminderOutcome, RoutineOutcome, RoutineVariant};
use hy_pet::geometry::{Monitor, Rect};
use hy_pet::link::present::{BubbleKind, action};
use hy_pet::placement::{SCALE_MAX, SCALE_MIN, SCALE_STEP, bring_above_clock, pet_window_physical};
use hy_pet::profile::PetEdit;
use hy_pet::shell::{Bubble, BubbleAction, BubbleOutcome, HideReason, PetShell};
use serde::Serialize;

use crate::coach::CoachRunner;
use crate::shell::{Shell, current_monitors};

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
) {
    let _ = std::thread::Builder::new()
        .name("hy-selftest".into())
        .spawn(move || {
            let mut report = run(&shell);
            match &coach {
                Some((runner, clock)) => coach_end_to_end(&mut report, &shell, runner, clock),
                None => report.check("coach_started", false, "the coaching engine did not start"),
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

    // 5. The engine's state is saved (as on exit).
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
