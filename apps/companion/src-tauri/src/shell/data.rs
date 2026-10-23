//! The setup wizard (wireframe W1), the sharing status (W8) and the full personal backup
//! (W9), on the shell.
//!
//! - **Setup** opens by itself on the first start of a new store and can be run again
//!   from Settings. Its steps save through the same commands as the settings tabs;
//!   "Finish" records that setup is done.
//! - **Sharing:** this build has no office server yet, so it states "Not paired; nothing
//!   is shared" and lists every category as off.
//! - **Export** writes one encrypted `.hybackup` file to the user's Documents folder.
//! - **Import** decrypts a file the page staged, replaces this PC's data on the engine's
//!   own store connection (so nothing is written in between), then restarts the app so
//!   every part starts from the imported data.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hy_core::backup::{EXTENSION, MAX_FILE_BYTES, Summary};
use hy_core::schema::Category;
use hy_pet::link::CoachLink;
use hy_pet::shell::Bubble;
use serde::Serialize;
use tauri::Manager;

use super::{SETUP, Shell, lock};
use crate::view::SharingRow;
use crate::windows;

/// Settings key recording that setup was finished (or skipped for an existing store).
pub const SETUP_DONE_KEY: &str = "setup_done";

/// How long the page has to say "Restarting" before the app restarts after an import.
const RESTART_AFTER: Duration = Duration::from_millis(1500);

/// A written backup file.
#[derive(Debug, Clone, Serialize)]
pub struct BackupDone {
    /// Where it was written.
    pub path: String,
    /// What it holds.
    pub summary: Summary,
}

#[derive(Debug, Serialize)]
struct SetupDone {
    at: chrono::DateTime<chrono::Utc>,
    /// Recorded without showing the wizard, for a store that was already in use.
    existing: bool,
}

const fn category_label(c: Category) -> &'static str {
    match c {
        Category::Hydration => "Hydration summary",
        Category::Outcomes => "Reminder and exercise outcomes",
        Category::WorkSession => "Work-session summary",
        Category::PhoneReports => "Phone self-report counts",
        Category::PostureSummary => "Posture check summary (V2)",
    }
}

/// A file name not yet used in `dir`: `HealthierYou backup 2026-10-23.hybackup`, then
/// `... (2).hybackup` and so on.
fn free_name(dir: &Path, date: chrono::NaiveDate) -> PathBuf {
    let base = format!("HealthierYou backup {date}");
    let mut path = dir.join(format!("{base}.{EXTENSION}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base} ({n}).{EXTENSION}"));
        n += 1;
    }
    path
}

impl Shell {
    // -------------------------------------------------------------------- setup

    /// Whether setup should open by itself: a new store where it was never finished. A
    /// store already in use (from before setup existed) is marked done instead.
    pub fn setup_needed(&self) -> bool {
        let store = lock(&self.store);
        match store.setting::<serde_json::Value>(SETUP_DONE_KEY) {
            Ok(Some(_)) => false,
            Ok(None) => {
                let fresh = !store.has_history().unwrap_or(true);
                if !fresh {
                    let _ = store.put_setting(
                        SETUP_DONE_KEY,
                        &SetupDone {
                            at: store.clock().now(),
                            existing: true,
                        },
                    );
                }
                fresh
            }
            Err(_) => false,
        }
    }

    /// Open the setup wizard (or focus it).
    pub fn open_setup(self: &Arc<Self>) {
        let monitor = lock(&self.inner).core.monitor().cloned();
        if let Err(e) = windows::open_setup(&self.app, monitor.as_ref()) {
            eprintln!("HealthierYou: could not open setup: {e}");
        }
    }

    /// The wizard's sample reminder (step 8): a card beside the pet that shows how a
    /// reminder looks. It closes by itself.
    pub fn setup_sample(self: &Arc<Self>) {
        let id = "setup-sample".to_owned();
        let text = "This is how a reminder looks: one at a time, beside your pet, never in your way. \
                    Real reminders start within your work hours. No sound.";
        self.run(|i, _| {
            let mut fx = i.core.model.show_notice(Bubble::notice(&id, text));
            fx.push(hy_pet::shell::Effect::Render);
            fx
        });
        let me = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("hy-notice".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(super::NOTICE_MS));
                me.post(move |s| s.run(|i, _| i.core.model.notice_expired("setup-sample")));
            });
    }

    /// "Finish": record that setup is done and close the wizard.
    ///
    /// # Errors
    /// A store write failure, worded for the page.
    pub fn setup_finish(&self) -> Result<(), String> {
        {
            let store = lock(&self.store);
            store
                .put_setting(
                    SETUP_DONE_KEY,
                    &SetupDone {
                        at: store.clock().now(),
                        existing: false,
                    },
                )
                .map_err(|e| format!("Setup could not be saved: {e}"))?;
        }
        if let Some(w) = self.app.get_webview_window(SETUP) {
            let _ = w.close();
        }
        Ok(())
    }

    /// Note that the wizard page loaded (for the self-test).
    pub fn setup_opened(&self) {
        lock(&self.inner).setup_loaded = true;
    }

    /// The id of the notice on show, if any (for the self-test).
    pub fn notice_id(&self) -> Option<String> {
        match lock(&self.inner).core.model.open() {
            hy_pet::shell::Open::Notice { bubble } => Some(bubble.id.clone()),
            _ => None,
        }
    }

    /// Whether the wizard page loaded (for the self-test).
    pub fn setup_loaded(&self) -> bool {
        lock(&self.inner).setup_loaded
    }

    // ------------------------------------------------------------------ sharing

    /// The sharing tab's rows (W8): every category and whether it is on. Without an
    /// office server (slice V1-5) every category is off.
    ///
    /// # Errors
    /// A store read failure.
    pub(super) fn sharing_rows(store: &hy_core::store::Store) -> hy_core::Result<Vec<SharingRow>> {
        Category::ALL
            .iter()
            .filter(|c| **c != Category::PostureSummary)
            .map(|c| {
                Ok(SharingRow {
                    category: c.as_str(),
                    label: category_label(*c),
                    enabled: store.sharing_policy(*c)?.enabled,
                })
            })
            .collect()
    }

    // ------------------------------------------------------------------- backup

    /// Where exports are written: the user's Documents folder.
    pub fn backup_dir(&self) -> Option<PathBuf> {
        self.app.path().document_dir().ok()
    }

    /// "Export my data": write an encrypted backup to the Documents folder.
    ///
    /// # Errors
    /// A short password, a missing Documents folder or a write failure, worded for the
    /// page; nothing was written.
    pub fn export_backup(&self, password: &str) -> Result<BackupDone, String> {
        let dir = self
            .backup_dir()
            .ok_or_else(|| "The Documents folder could not be found.".to_owned())?;
        self.export_backup_to(&dir, password)
    }

    /// Write an encrypted backup into `dir`.
    ///
    /// # Errors
    /// See [`Shell::export_backup`].
    pub fn export_backup_to(&self, dir: &Path, password: &str) -> Result<BackupDone, String> {
        let (bytes, summary, path) = {
            let store = lock(&self.store);
            let (bytes, summary) = store
                .export_backup(password)
                .map_err(|e| format!("Nothing was written: {e}"))?;
            let today = store
                .today()
                .map_err(|e| format!("Nothing was written: {e}"))?;
            (bytes, summary, free_name(dir, today))
        };
        std::fs::create_dir_all(dir).map_err(|e| format!("Nothing was written: {e}"))?;
        std::fs::write(&path, bytes).map_err(|e| format!("Nothing was written: {e}"))?;
        Ok(BackupDone {
            path: path.display().to_string(),
            summary,
        })
    }

    /// Keep the bytes of a backup file the page read, until the password arrives.
    ///
    /// # Errors
    /// A file too large to be a backup.
    pub fn stage_backup(&self, bytes: Vec<u8>) -> Result<usize, String> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err("This file is too large to be a HealthierYou backup.".to_owned());
        }
        let n = bytes.len();
        lock(&self.inner).staged_backup = Some(bytes);
        Ok(n)
    }

    /// "Import": replace this PC's data with the staged file's, then restart.
    ///
    /// # Errors
    /// No file staged, a wrong password, a damaged or newer file, or a refused row,
    /// worded for the page; nothing was changed.
    pub fn import_backup(self: &Arc<Self>, password: String) -> Result<Summary, String> {
        let file = lock(&self.inner)
            .staged_backup
            .clone()
            .ok_or_else(|| "Choose a backup file first.".to_owned())?;
        let result = match self.runner() {
            Some(runner) => runner
                .call(move |link: &mut CoachLink| {
                    link.replace_data(|s| s.import_backup(&file, &password))
                })
                .ok_or_else(|| "The app did not answer in time. Nothing was changed.".to_owned())?,
            None => lock(&self.store).import_backup(&file, &password),
        };
        let summary = result.map_err(|e| format!("Nothing was changed: {e}"))?;
        lock(&self.inner).staged_backup = None;
        // The engine's saved state went with the old data: stop it before it steps again.
        if let Some(runner) = self.runner() {
            runner.shutdown(RESTART_AFTER);
        }
        let app = self.app.clone();
        let _ = std::thread::Builder::new()
            .name("hy-restart".into())
            .spawn(move || {
                std::thread::sleep(RESTART_AFTER);
                app.restart();
            });
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_never_overwrite_an_earlier_file() {
        let dir = std::env::temp_dir().join(format!("hy-backup-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        let first = free_name(&dir, day);
        assert!(first.ends_with("HealthierYou backup 2026-10-23.hybackup"));
        std::fs::write(&first, b"x").unwrap();
        let second = free_name(&dir, day);
        assert!(second.ends_with("HealthierYou backup 2026-10-23 (2).hybackup"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
