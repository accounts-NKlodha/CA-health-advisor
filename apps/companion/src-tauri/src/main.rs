//! HealthierYou desktop companion (architecture 4).
//!
//! V1-0 skeleton: it starts Tauri with one placeholder screens window and opens (creating
//! and migrating if needed) the local store at `%LOCALAPPDATA%\HealthierYou\data\hy.db`.
//! The pet window, tray, tracker, engine and screens are later slices (V1-1 to V1-4).
//! No IPC commands are exposed yet, and no capabilities are granted to the webview.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hy_core::clock::SystemClock;
use hy_core::store::Store;
use tauri::Manager;

/// The store's location under the per-user local data folder (architecture 6.1).
fn store_path(local_data: PathBuf) -> PathBuf {
    local_data.join("HealthierYou").join("data").join("hy.db")
}

fn main() {
    let result = tauri::Builder::default()
        .setup(|app| {
            let path = store_path(app.path().local_data_dir()?);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let store = Store::open(&path, Arc::new(SystemClock::new()))?;
            app.manage(Mutex::new(store));
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("HealthierYou failed to start: {e}");
        std::process::exit(1);
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
}
