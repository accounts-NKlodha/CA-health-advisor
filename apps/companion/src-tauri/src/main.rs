//! HealthierYou desktop companion (architecture 4).
//!
//! Slice V1-1: the pet shell. It opens (creating and migrating if needed) the local store
//! at `%LOCALAPPDATA%\HealthierYou\data\hy.db`, loads the ten pet packs, and runs:
//!
//! - the always-present pet window (transparent, always on top, never takes focus),
//!   placed inside the work area at every size and Windows scaling, and placed again
//!   after display, scaling and taskbar changes;
//! - the tray menu (show or hide pet, Bring pet above clock, pet panel, weekly review,
//!   settings, quit);
//! - the keyboard pet panel (Ctrl+Alt+H by default), which takes focus on purpose and
//!   hands it back on Escape;
//! - the settings window with the pet library and edits;
//! - the review window (slice V1-4): the weekly review and monthly progress, read from
//!   the store's per-day tallies and events (`hy_core::review`), with phone-report label
//!   corrections;
//! - pairing with the office server (slice V1-5a) in [`server_link`]: the slip, the six
//!   words and check code for the administrator, "Check again" and unpair;
//! - install and update (slice V1-6) in [`update`]: start at sign-in, installing a signed
//!   update file with the previous version kept and put back if the new one does not
//!   start, and going back to the pre-upgrade copy of the data when an older version
//!   cannot read the newer one. The same program is the update helper and answers the
//!   command-line update steps.
//!
//! The tracker, coaching engine and suppression (V1-2) drive the pet through
//! `hy_pet::shell::PetShell`, implemented by [`shell::ShellHandle`]. [`coach`] runs the
//! engine on its own thread over a second connection to the same store, feeds it the
//! Windows signals (input idle time every 5 s, lock and unlock, sleep and wake,
//! presentation and full-screen state) and routes the user's answers from the bubble, pet
//! menu, tray and keyboard panel back to it; `hy_pet::link` holds the rules. On exit the
//! engine's state is saved.
//!
//! The V2 posture check (slice V2-1) runs in its own camera page while the user has it
//! on: `hy_core::camera` holds its rules, `shell::camera` carries them out, and only the
//! camera page may use the camera (see [`windows`]).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod coach;
mod commands;
mod selftest;
mod server_link;
mod shell;
mod tray;
mod update;
mod view;
#[cfg(windows)]
mod win32;
mod windows;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hy_core::clock::{Clock, ManualClock, SystemClock};
use hy_core::store::Store;
use hy_pet::pack::Library;
use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::{Manager, RunEvent};

use crate::coach::{CoachRunner, SHUTDOWN_WAIT};
use crate::shell::Shell;
use crate::update::{Dirs, Updater};

/// The store's location under the per-user local data folder (architecture 6.1).
fn store_path(local_data: PathBuf) -> PathBuf {
    local_data.join("HealthierYou").join("data").join("hy.db")
}

/// The self-test's report path, if `HY_SELFTEST` is set (CI only).
fn selftest_report() -> Option<PathBuf> {
    std::env::var_os("HY_SELFTEST")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn setup(
    app: &mut tauri::App,
    version: hy_core::update::Version,
) -> Result<(), Box<dyn std::error::Error>> {
    let selftest = selftest_report();
    // The self-test never touches the user's data: it uses a fresh store in the temp
    // folder.
    let data_root = if selftest.is_some() {
        std::env::temp_dir().join(format!("hy-selftest-{}", std::process::id()))
    } else {
        app.path().local_data_dir()?
    };
    #[cfg(windows)]
    win32::mark_running();
    let dirs = Dirs::under(&data_root);
    let path = store_path(data_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // A data restore cut short (a crash, or an undo that failed) is finished or undone
    // from its recovery record first; if that cannot be done, stop without opening (and
    // so creating) an empty database.
    if selftest.is_none() && !update::settle_incomplete_restore(&path) {
        std::process::exit(1);
    }
    let store = match Store::open(&path, Arc::new(SystemClock::new())) {
        // An older version after a rollback, facing data a newer one changed: offer the
        // copy taken before that upgrade, or close without changing anything.
        Err(hy_core::Error::SchemaTooNew { .. }) if selftest.is_none() => {
            if !update::offer_data_rollback(&path, version) {
                std::process::exit(1);
            }
            Store::open(&path, Arc::new(SystemClock::new()))?
        }
        other => other?,
    };
    app.manage(Arc::new(Updater::new(version, dirs)));
    let (library, refused) = Library::builtin()?;
    for e in &refused {
        eprintln!("HealthierYou: pet pack refused: {e}");
    }
    let shell = Shell::new(app.handle().clone(), store, library)?;
    app.manage(Arc::clone(&shell));
    windows::create_pet_window(app.handle())?;
    tray::create(app.handle(), &shell)?;
    shell.start();

    // The coaching engine, over its own connection to the store. The self-test gives it a
    // simulated clock and drives it without the signal pump.
    let sim = selftest
        .is_some()
        .then(|| Arc::new(ManualClock::new(selftest::sim_start())));
    let coach_clock: Arc<dyn Clock> = match &sim {
        Some(c) => c.clone(),
        None => Arc::new(SystemClock::new()),
    };
    let runner = Store::open(&path, coach_clock)
        .and_then(|store| CoachRunner::start(&shell, store, sim.is_none()));
    let runner = match runner {
        Ok(r) => {
            app.manage(Arc::clone(&r));
            let weak = Arc::downgrade(&r);
            shell.set_system_listener(Arc::new(move |s| {
                if let Some(r) = weak.upgrade() {
                    r.system(s);
                }
            }));
            Some(r)
        }
        Err(e) => {
            // The pet still runs; reminders are off until the problem is fixed.
            eprintln!("HealthierYou: coaching could not start: {e}");
            None
        }
    };

    // The office-server sync runs on its own while the app runs (not in the self-test).
    if selftest.is_none() {
        server_link::start_periodic_sync(&shell);
    }

    // The posture check (V2-1): back on if the user left it on (after the engine has had
    // its first chance to report holds), and its clock (watchdog, stop deadline,
    // retries). The self-test drives the camera itself.
    if selftest.is_none() {
        let s = Arc::clone(&shell);
        shell.post(move |_| s.camera_resume());
    }
    shell::start_camera_clock(&shell);

    // A new store opens the setup wizard (W1). The self-test opens it itself.
    if selftest.is_none() && shell.setup_needed() {
        shell.open_setup();
    }

    if let Some(report) = selftest {
        let idle = std::env::var("HY_SELFTEST_IDLE_S")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        selftest::spawn(
            Arc::clone(&shell),
            runner.zip(sim),
            report,
            Duration::from_secs(idle),
        );
    }
    Ok(())
}

fn main() {
    let context = tauri::generate_context!();
    // The version the installer and the update rules know this build by (tauri.conf.json).
    let version = context
        .package_info()
        .version
        .to_string()
        .parse()
        .unwrap_or(hy_core::update::Version::new(0, 0, 0));
    // The update helper and the command-line update steps run without any window.
    if let Some(code) = update::command_line(version) {
        std::process::exit(code);
    }
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::pet_bootstrap,
            commands::pet_ready,
            commands::pet_click,
            commands::bubble_measured,
            commands::bubble_close,
            commands::bubble_action,
            commands::menu_action,
            commands::drag_start,
            commands::drag_move,
            commands::drag_end,
            commands::pet_edit,
            commands::bring_pet,
            commands::hide_pet,
            commands::open_settings,
            commands::panel_bootstrap,
            commands::panel_close,
            commands::settings_bootstrap,
            commands::set_hotkey,
            commands::open_review,
            commands::review_target,
            commands::review_week,
            commands::review_month,
            commands::review_phone_label,
            commands::review_suggestion,
            commands::settings_page,
            commands::save_coach_settings,
            commands::save_water_settings,
            commands::save_workweek,
            commands::save_accessibility,
            commands::plan_page,
            commands::plan_save,
            commands::plan_cancel,
            commands::open_setup,
            commands::setup_page,
            commands::setup_sample,
            commands::setup_finish,
            commands::backup_export,
            commands::backup_stage,
            commands::backup_import,
            commands::server_status,
            commands::server_pair,
            commands::server_refresh,
            commands::server_unpair,
            commands::server_notice_seen,
            commands::server_set_address,
            commands::server_share_set,
            commands::server_notice_dismiss,
            commands::server_share_preview,
            commands::server_restore,
            commands::app_page,
            commands::update_choose,
            commands::update_install,
            commands::set_autostart,
            commands::camera_open,
            commands::camera_off,
            commands::camera_on,
            commands::camera_page,
            commands::camera_consent,
            commands::camera_try_again,
            commands::camera_withdraw,
            commands::camera_report,
            commands::camera_frame,
        ])
        // The camera permission (architecture 4.5): only the camera page, only while the
        // posture check wants capture; everything else is refused. On Windows the guard in
        // `windows` also makes sure no answer is saved in the WebView2 profile.
        .on_permission_request(|webview, kind| {
            let camera = matches!(kind, PermissionKind::Camera);
            let allowed = camera
                && webview
                    .try_state::<Arc<Shell>>()
                    .is_some_and(|s| s.camera_permission(webview.label()));
            if allowed {
                PermissionResponse::Allow
            } else {
                PermissionResponse::Deny
            }
        })
        .setup(move |app| setup(app, version))
        .build(context);
    match app {
        Ok(app) => app.run(|app, event| match event {
            // Closing the settings window or the panel must not quit: the pet and tray
            // stay. Only "Quit" in the tray (an explicit exit code) ends the app.
            RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            // A clean exit saves the engine's state first.
            RunEvent::Exit => {
                if let Some(r) = app.try_state::<Arc<CoachRunner>>() {
                    r.shutdown(SHUTDOWN_WAIT);
                }
            }
            _ => {}
        }),
        Err(e) => {
            eprintln!("HealthierYou failed to start: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_lives_under_healthieryou_data() {
        let p = store_path(PathBuf::from("base"));
        assert!(p.ends_with("HealthierYou/data/hy.db") || p.ends_with("HealthierYou\\data\\hy.db"));
    }

    #[test]
    fn store_opens_and_migrates_in_a_temp_folder() {
        let dir = std::env::temp_dir().join(format!("hy-companion-test-{}", std::process::id()));
        let path = store_path(dir.clone());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let store = Store::open(&path, Arc::new(SystemClock::new())).unwrap();
        assert_eq!(
            store.migration_report().to,
            hy_core::migrations::latest_version()
        );
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn builtin_packs_load_for_the_companion() {
        let (lib, refused) = Library::builtin().unwrap();
        assert!(refused.is_empty());
        assert_eq!(lib.packs().len(), 10);
    }

    #[test]
    fn views_serialise_for_the_pages() {
        let (lib, _) = Library::builtin().unwrap();
        let profile = hy_pet::profile::default_profile(&lib);
        let v = view::settings_view(&profile, &lib, Some("Ctrl+Alt+H".into()), true);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["packs"].as_array().unwrap().len(), 10);
        assert_eq!(json["packs"][4]["art_status"], "pending_review");
        assert_eq!(json["packs"][4]["selectable"], false);
        assert_eq!(json["edge"], "right");
        let cam = view::CameraMenu::of(hy_core::camera::Status::Off, false);
        let m = view::menu_view(&profile, &[], None, cam);
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["greeting"], "Hi, I'm Shelly.");
        assert_eq!(json["presets"][3]["short"], "XL");
        assert_eq!(json["camera"]["label"], "Posture check");
        assert_eq!(json["camera"]["on"], false);
    }

    #[test]
    fn review_pages_serialise_for_the_review_window() {
        let dir = std::env::temp_dir().join(format!("hy-companion-review-{}", std::process::id()));
        let path = store_path(dir.clone());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let store = Store::open(&path, Arc::new(SystemClock::new())).unwrap();
        let week = view::ReviewPage {
            review: store.week_review(None).unwrap(),
            motion: true,
            a11y: store.accessibility().unwrap(),
        };
        let json = serde_json::to_value(&week).unwrap();
        assert_eq!(json["motion"], true);
        assert_eq!(json["review"]["status"], "first_week");
        assert_eq!(json["review"]["days"].as_array().unwrap().len(), 7);
        assert_eq!(json["review"]["rows"][0]["metric"], "movement");
        assert_eq!(
            json["review"]["rows"][2]["marks"].as_array().unwrap().len(),
            7
        );
        let month = view::ReviewPage {
            review: store.month_review(None).unwrap(),
            motion: false,
            a11y: store.accessibility().unwrap(),
        };
        let json = serde_json::to_value(&month).unwrap();
        assert_eq!(json["review"]["status"], "too_early");
        assert_eq!(json["review"]["month"].as_str().unwrap().len(), 7);
        assert_eq!(json["a11y"]["text_pct"], 100);
        // Each bar carries the engine's kind per metric, in the names Month.tsx maps.
        for metric in ["movement", "breaks", "water"] {
            let kind = json["review"]["bars"][0]["kind"][metric].as_str().unwrap();
            assert!(
                ["no_data", "nothing_to_count", "zero", "measured"].contains(&kind),
                "{metric}: {kind}"
            );
        }
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_the_pages_send_are_accepted() {
        // The shapes the settings and plan pages send (ui/src/shared/types.ts).
        let config: hy_core::coach::CoachConfig = serde_json::from_value(serde_json::json!({
            "intensity": "more_often",
            "categories": { "movement": true, "eye_rest": false, "water": true },
            "hide_during_presentation": true,
            "media_in_use_hint": false,
            "long_day_check_in": { "enabled": true, "hours": 10 },
            "pomodoro_auto_start": true,
            "movement_version": "seated",
            "review": { "day": "fri", "at": "18:00" }
        }))
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.review.day, chrono::Weekday::Fri);
        // Saves send one field at a time (CoachChange, WaterChange, WorkweekChange).
        use hy_core::settings::{CoachChange, WaterChange, WorkweekChange};
        for (sent, field) in [
            (serde_json::json!({ "intensity": "light" }), "intensity"),
            (serde_json::json!({ "eye_rest": false }), "eye_rest"),
            (serde_json::json!({ "water": true }), "water"),
            (
                serde_json::json!({ "long_day_hours": 10 }),
                "long_day_hours",
            ),
            (
                serde_json::json!({ "movement_version": "seated" }),
                "movement_version",
            ),
            (
                serde_json::json!({ "review": { "day": "fri", "at": "18:00" } }),
                "review",
            ),
        ] {
            let change: CoachChange = serde_json::from_value(sent.clone()).unwrap();
            assert_eq!(serde_json::to_value(change).unwrap(), sent, "{field}");
        }
        assert!(
            serde_json::from_value::<CoachChange>(serde_json::to_value(config).unwrap()).is_err()
        );
        for sent in [
            serde_json::json!({ "goal": 2000 }),
            serde_json::json!({ "add_serving": 750 }),
            serde_json::json!({ "remove_serving": 150 }),
            serde_json::json!({ "edit_servings": { "add": [200], "remove": [150] } }),
        ] {
            serde_json::from_value::<WaterChange>(sent).unwrap();
        }
        let week: WorkweekChange = serde_json::from_value(serde_json::json!({
            "workdays": ["mon", "tue", "wed", "thu", "fri"],
            "hours": { "start": "09:00", "end": "17:30" },
            "zone": "Asia/Kolkata"
        }))
        .unwrap();
        week.schedule_after(hy_core::time::WorkSchedule::default())
            .validate()
            .unwrap();
        let only_zone: WorkweekChange =
            serde_json::from_value(serde_json::json!({ "zone": "Europe/London" })).unwrap();
        assert!(only_zone.workdays.is_none() && only_zone.hours.is_none());
        let a11y: hy_core::prefs::Accessibility = serde_json::from_value(serde_json::json!({
            "text_pct": 150, "reduced_motion": true, "high_contrast": false
        }))
        .unwrap();
        a11y.validate().unwrap();
        let plan: hy_core::coach::RecoveryPlan = serde_json::from_value(serde_json::json!({
            "date": "2026-10-27",
            "more_breaks": true,
            "walk_at": "16:30:00",
            "walk_repeat": true,
            "lunch_away": false,
            "calm_pause": true,
            "finish_at": null,
            "reflection": true
        }))
        .unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        plan.validate(today).unwrap();
        // And what the pages are sent reads back the same way.
        let json = serde_json::to_value(plan).unwrap();
        assert_eq!(json["walk_at"], "16:30:00");
        let json = serde_json::to_value(config).unwrap();
        assert_eq!(
            json["review"],
            serde_json::json!({ "day": "fri", "at": "18:00" })
        );
    }

    #[test]
    fn the_review_offer_week_is_taken_once_and_dropped_by_other_opens() {
        let week = chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let mut t = shell::ReviewTarget::default();
        assert_eq!(t.take(), None);
        // "Open" on the offer: the page takes the week once, then holds it itself.
        t.offer(week);
        assert_eq!(t.take(), Some(week));
        assert_eq!(t.take(), None);
        // Opened from the tray before a page took it: dropped, so a later window starts
        // on this week.
        t.offer(week);
        t.clear();
        assert_eq!(t.take(), None);
        // A second offer replaces the first.
        t.offer(week - chrono::Days::new(7));
        t.offer(week);
        assert_eq!(t.take(), Some(week));
    }

    #[test]
    fn every_window_capability_lists_only_known_commands() {
        let caps = [
            include_str!("../capabilities/pet.json"),
            include_str!("../capabilities/panel.json"),
            include_str!("../capabilities/screens.json"),
            include_str!("../capabilities/review.json"),
            include_str!("../capabilities/setup.json"),
            include_str!("../capabilities/plan.json"),
            include_str!("../capabilities/camera.json"),
        ];
        let build_rs = include_str!("../build.rs");
        for cap in caps {
            let v: serde_json::Value = serde_json::from_str(cap).unwrap();
            for p in v["permissions"].as_array().unwrap() {
                let p = p.as_str().unwrap();
                if let Some(cmd) = p.strip_prefix("allow-") {
                    let name = cmd.replace('-', "_");
                    assert!(
                        build_rs.contains(&format!("\"{name}\"")),
                        "{p} is not a registered command"
                    );
                }
            }
        }
    }
}
