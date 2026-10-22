//! HealthierYou desktop companion (architecture 4).
//!
//! Slice V1-1: the pet shell. It opens (creating and migrating if needed) the local store
//! at `%LOCALAPPDATA%\HealthierYou\data\hy.db`, loads the ten pet packs, and runs:
//!
//! - the always-present pet window (transparent, always on top, never takes focus),
//!   placed inside the work area at every size and Windows scaling, and placed again
//!   after display, scaling and taskbar changes;
//! - the tray menu (show or hide pet, Bring pet above clock, pet panel, settings, quit);
//! - the keyboard pet panel (Ctrl+Alt+H by default), which takes focus on purpose and
//!   hands it back on Escape;
//! - the settings window with the pet library and edits.
//!
//! The tracker, coaching engine and suppression (V1-2) drive the pet through
//! `hy_pet::shell::PetShell`, implemented by [`shell::ShellHandle`].

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod selftest;
mod shell;
mod tray;
mod view;
#[cfg(windows)]
mod win32;
mod windows;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hy_core::clock::SystemClock;
use hy_core::store::Store;
use hy_pet::pack::Library;
use tauri::{Manager, RunEvent};

use crate::shell::Shell;

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

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let selftest = selftest_report();
    // The self-test never touches the user's data: it uses a fresh store in the temp
    // folder.
    let data_root = if selftest.is_some() {
        std::env::temp_dir().join(format!("hy-selftest-{}", std::process::id()))
    } else {
        app.path().local_data_dir()?
    };
    let path = store_path(data_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let store = Store::open(&path, Arc::new(SystemClock::new()))?;
    let (library, refused) = Library::builtin()?;
    for e in &refused {
        eprintln!("HealthierYou: pet pack refused: {e}");
    }
    let shell = Shell::new(app.handle().clone(), store, library)?;
    app.manage(Arc::clone(&shell));
    windows::create_pet_window(app.handle())?;
    tray::create(app.handle(), &shell)?;
    shell.start();
    if let Some(report) = selftest {
        let idle = std::env::var("HY_SELFTEST_IDLE_S")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        selftest::spawn(Arc::clone(&shell), report, Duration::from_secs(idle));
    }
    Ok(())
}

fn main() {
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
        ])
        .setup(setup)
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, event| {
            // Closing the settings window or the panel must not quit: the pet and tray
            // stay. Only "Quit" in the tray (an explicit exit code) ends the app.
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
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
        let m = view::menu_view(&profile, &[], None);
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["greeting"], "Hi, I'm Shelly.");
        assert_eq!(json["presets"][3]["short"], "XL");
    }

    #[test]
    fn every_window_capability_lists_only_known_commands() {
        let caps = [
            include_str!("../capabilities/pet.json"),
            include_str!("../capabilities/panel.json"),
            include_str!("../capabilities/screens.json"),
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
