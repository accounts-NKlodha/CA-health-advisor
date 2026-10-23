//! The setup wizard (wireframe W1), the sharing status (W8) and the full personal backup
//! (W9), on the shell.
//!
//! - **Setup** opens by itself on the first start of a new store and can be run again
//!   from Settings. Its steps save through the same commands as the settings tabs;
//!   "Finish" records that setup is done.
//! - **Sharing:** every category and whether it is on. Pairing with the office server is
//!   in [`crate::server_link`]; switching categories on arrives with sync (slice V1-5b).
//! - **Export** writes one encrypted `.hybackup` file to the user's Documents folder.
//! - **Import** decrypts a file the page staged, replaces this PC's data on the engine's
//!   own store connection (so nothing is written in between), then restarts the app so
//!   every part starts from the imported data. The replacement and the engine stop are
//!   one operation on the engine thread ([`CoachRunner::replace_and_stop`]): a slow
//!   import that outlasts the page's wait is reported as still running, and when it
//!   finishes the engine thread itself stops the engine, restarts the app and tells the
//!   page, so a late success is never left with the old engine running.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hy_core::backup::{EXTENSION, MAX_FILE_BYTES, Summary};
use hy_core::schema::Category;
use hy_pet::link::CoachLink;
use hy_pet::shell::Bubble;
use serde::Serialize;
use tauri::{Emitter, Manager};

use super::{SETUP, Shell, lock};
use crate::coach::{CALL_WAIT, CallError, CoachRunner};
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

/// What "Import" answers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ImportOutcome {
    /// Replaced; the app restarts.
    Done {
        /// What the file held.
        summary: Summary,
    },
    /// Still running after the wait (a slow key derivation or a large file). The page
    /// hears the outcome as [`ImportFinished`]; nothing is claimed either way until then.
    Pending,
}

/// The outcome of an import, sent to the pages as `hy://backup-import` by the engine
/// thread when it finishes (also when the page had stopped waiting).
#[derive(Debug, Clone, Serialize)]
pub struct ImportFinished {
    /// Replaced (the app restarts), or refused with nothing changed.
    pub ok: bool,
    /// What the file held, when replaced.
    pub summary: Option<Summary>,
    /// Why it was refused.
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SetupDone {
    at: chrono::DateTime<chrono::Utc>,
    /// Recorded without showing the wizard, for a store that was already in use.
    existing: bool,
}

pub(crate) const fn category_label(c: Category) -> &'static str {
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

    /// Run `f` on the shell's store (the server link's steps; never across a network
    /// wait).
    pub fn with_store<R>(&self, f: impl FnOnce(&mut hy_core::store::Store) -> R) -> R {
        f(&mut lock(&self.store))
    }

    /// The sharing tab's rows (W8): every category and whether it is on. Until sync
    /// (slice V1-5b) every category stays off, paired or not.
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
    /// worded for the page; nothing was changed. An import still running after the wait
    /// is [`ImportOutcome::Pending`], never an error.
    pub fn import_backup(self: &Arc<Self>, password: String) -> Result<ImportOutcome, String> {
        let file = lock(&self.inner)
            .staged_backup
            .clone()
            .ok_or_else(|| "Choose a backup file first.".to_owned())?;
        let Some(runner) = self.runner() else {
            // No engine (it failed to start): the shell's connection is the only writer.
            let summary = lock(&self.store)
                .import_backup(&file, &password)
                .map_err(|e| format!("Nothing was changed: {e}"))?;
            self.imported();
            return Ok(ImportOutcome::Done { summary });
        };
        let me = Arc::clone(self);
        import_on_engine(
            &runner,
            move |link| link.replace_data(|s| s.import_backup(&file, &password)),
            move |out| me.import_finished(out),
            CALL_WAIT,
        )
    }

    /// The engine thread finished an import (whether or not the page still waits).
    fn import_finished(self: &Arc<Self>, out: &hy_core::Result<Summary>) {
        let finished = match out {
            Ok(summary) => {
                self.imported();
                ImportFinished {
                    ok: true,
                    summary: Some(summary.clone()),
                    error: None,
                }
            }
            Err(e) => ImportFinished {
                ok: false,
                summary: None,
                error: Some(format!("Nothing was changed: {e}")),
            },
        };
        let _ = self.app.emit("hy://backup-import", finished);
    }

    /// The data was replaced and the engine stopped: restart so every part starts from
    /// the imported data.
    fn imported(&self) {
        lock(&self.inner).staged_backup = None;
        let app = self.app.clone();
        let _ = std::thread::Builder::new()
            .name("hy-restart".into())
            .spawn(move || {
                std::thread::sleep(RESTART_AFTER);
                app.restart();
            });
    }
}

/// Run an import on the engine thread ([`CoachRunner::replace_and_stop`]) and word the
/// answer for the page. A wait that runs out is [`ImportOutcome::Pending`]: the engine
/// thread completes the import (engine stop, restart, report) through `after`.
///
/// # Errors
/// The import was refused (nothing changed), the engine was not running (nothing ran),
/// or the engine thread ended without an answer (outcome unknown, worded so).
fn import_on_engine(
    runner: &CoachRunner,
    f: impl FnOnce(&mut CoachLink) -> hy_core::Result<Summary> + Send + 'static,
    after: impl FnOnce(&hy_core::Result<Summary>) + Send + 'static,
    wait: Duration,
) -> Result<ImportOutcome, String> {
    match runner.replace_and_stop(f, after, wait) {
        Ok(Ok(summary)) => Ok(ImportOutcome::Done { summary }),
        Ok(Err(e)) => Err(format!("Nothing was changed: {e}")),
        Err(CallError::Pending) => Ok(ImportOutcome::Pending),
        Err(CallError::NotRunning) => Err(
            "Nothing was changed: the coaching engine is not running. Restart HealthierYou and try again."
                .to_owned(),
        ),
        Err(CallError::Lost) => Err(
            "The import's outcome is not known: the coaching engine stopped before answering. \
             Restart HealthierYou to see which data it has."
                .to_owned(),
        ),
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
