//! IPC commands. Each window may call only the commands its capability file grants
//! (`capabilities/*.json`). The commands are synchronous, so Tauri runs them on the main
//! thread, which the shell requires; the exceptions are the server-link commands that
//! wait on the network, which are async and touch only the store, and the update-file
//! dialog, which waits on the user.

use std::sync::Arc;

use chrono::NaiveDate;
use hy_core::camera::FrameReport;
use hy_core::coach::RecoveryPlan;
use hy_core::prefs::Accessibility;
use hy_core::review::suggest::SuggestionKind;
use hy_core::review::{MonthReview, WeekReview};
use hy_core::schema::PhoneLabel;
use hy_core::settings::{CoachChange, WaterChange, WorkweekChange};
use hy_core::store::MonthKey;
use hy_pet::profile::PetEdit;
use tauri::State;
use uuid::Uuid;

use crate::server_link::{self, LinkView};
use crate::shell::{BackupDone, ImportOutcome, PageReport, Shell, SuggestionAnswer};
use crate::update::{AppPage, UpdateCheck, Updater};
use crate::view::{
    CameraPage, PanelView, PetView, PlanPage, ReviewPage, SettingsPage, SettingsView,
};

type ShellState<'a> = State<'a, Arc<Shell>>;
type UpdaterState<'a> = State<'a, Arc<Updater>>;

// ------------------------------------------------------------------ pet window

/// The pet page's first view.
#[tauri::command]
pub fn pet_bootstrap(shell: ShellState<'_>) -> PetView {
    shell.current_pet_view()
}

/// The pet page drew its first view. After an update this is the new version's
/// confirmation that it started (the bundled WebView2 runtime works).
#[tauri::command]
pub fn pet_ready(shell: ShellState<'_>, updater: UpdaterState<'_>) {
    shell.mark_page_ready();
    updater.page_ready();
}

/// A click on the pet (pointer down and up without moving).
#[tauri::command]
pub fn pet_click(shell: ShellState<'_>) {
    shell.pet_click();
}

/// The open bubble's measured size in CSS pixels, for the open item `token`.
#[tauri::command]
pub fn bubble_measured(shell: ShellState<'_>, token: u64, width: f64, height: f64) {
    shell.bubble_measured(token, width, height);
}

/// The bubble's close button.
#[tauri::command]
pub fn bubble_close(shell: ShellState<'_>) {
    shell.bubble_close();
}

/// An action in the engine's bubble.
#[tauri::command]
pub fn bubble_action(shell: ShellState<'_>, bubble_id: String, action_id: String) {
    shell.bubble_action(&bubble_id, &action_id);
}

/// An engine action in the pet menu or panel.
#[tauri::command]
pub fn menu_action(shell: ShellState<'_>, action_id: String) {
    shell.menu_action(&action_id);
}

/// Pointer down on the pet.
#[tauri::command]
pub fn drag_start(shell: ShellState<'_>) {
    shell.drag_start();
}

/// Pointer moved while down.
#[tauri::command]
pub fn drag_move(shell: ShellState<'_>) {
    shell.drag_move();
}

/// Pointer up. Without movement it is a click.
#[tauri::command]
pub fn drag_end(shell: ShellState<'_>, moved: bool) {
    if !shell.drag_end(moved) {
        shell.pet_click();
    }
}

// ------------------------------------------------------------ shared actions

/// A pet edit (Settings, pet menu or panel).
///
/// # Errors
/// The refusal, worded for the user.
#[tauri::command]
pub fn pet_edit(shell: ShellState<'_>, edit: PetEdit) -> Result<SettingsView, String> {
    shell.edit(&edit)
}

/// "Bring pet above clock".
#[tauri::command]
pub fn bring_pet(shell: ShellState<'_>) {
    shell.bring_above_clock();
}

/// "Hide pet" (shown again from the tray).
#[tauri::command]
pub fn hide_pet(shell: ShellState<'_>) {
    shell.hide_pet();
}

/// Open the settings window.
#[tauri::command]
pub fn open_settings(shell: ShellState<'_>) {
    shell.open_settings();
}

/// Open the review window (wireframe W13).
#[tauri::command]
pub fn open_review(shell: ShellState<'_>) {
    shell.open_review();
}

/// The week the review page should go to ("Open" on the weekly review offer), once.
#[tauri::command]
pub fn review_target(shell: ShellState<'_>) -> Option<NaiveDate> {
    shell.review_target()
}

// ----------------------------------------------------------------------- panel

/// The panel page's first view.
#[tauri::command]
pub fn panel_bootstrap(shell: ShellState<'_>) -> PanelView {
    shell.panel_view()
}

/// Escape or an action that finishes with the panel: close it and give focus back to
/// the window that had it (`restore`), or leave focus where the action put it.
#[tauri::command]
pub fn panel_close(shell: ShellState<'_>, restore: bool) {
    shell.close_panel(restore);
}

// -------------------------------------------------------------------- settings

/// The Settings page's view.
#[tauri::command]
pub fn settings_bootstrap(shell: ShellState<'_>) -> SettingsView {
    shell.settings_view()
}

/// Change the panel hotkey (`null` turns it off).
///
/// # Errors
/// The refusal, worded for the user.
#[tauri::command]
pub fn set_hotkey(shell: ShellState<'_>, hotkey: Option<String>) -> Result<SettingsView, String> {
    shell.set_hotkey(hotkey.as_deref())
}

// --------------------------------------------------------------------- reviews

/// The weekly review of the week containing `day` (default this week).
///
/// # Errors
/// A read failure, worded for the user.
#[tauri::command]
pub fn review_week(
    shell: ShellState<'_>,
    day: Option<NaiveDate>,
) -> Result<ReviewPage<WeekReview>, String> {
    shell.note_review_request(day);
    shell.review_week(day)
}

/// Monthly progress for `month` (`"2026-10"`; default this month).
///
/// # Errors
/// A read failure, worded for the user.
#[tauri::command]
pub fn review_month(
    shell: ShellState<'_>,
    month: Option<MonthKey>,
) -> Result<ReviewPage<MonthReview>, String> {
    shell.review_month(month)
}

/// Change a phone report's label from the weekly review; answers with the week again.
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn review_phone_label(
    shell: ShellState<'_>,
    week: Option<NaiveDate>,
    entry: Uuid,
    label: PhoneLabel,
) -> Result<ReviewPage<WeekReview>, String> {
    shell.review_phone_label(week, entry, label)
}

/// Apply, undo or hide one of the week's suggestions ("Try next week"); answers with
/// the week again.
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn review_suggestion(
    shell: ShellState<'_>,
    week: NaiveDate,
    kind: SuggestionKind,
    answer: SuggestionAnswer,
) -> Result<ReviewPage<WeekReview>, String> {
    shell.review_suggestion(week, kind, answer)
}

// ------------------------------------------------------------ settings tabs

/// The settings window's Reminders, Water and workweek, Movement programme, Focus and
/// Pomodoro and Accessibility tabs.
///
/// # Errors
/// A read failure, worded for the user.
#[tauri::command]
pub fn settings_page(shell: ShellState<'_>) -> Result<SettingsPage, String> {
    shell.settings_page()
}

/// Change one coaching setting (reminders, movement version, Pomodoro, review time).
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_coach_settings(
    shell: ShellState<'_>,
    change: CoachChange,
) -> Result<SettingsPage, String> {
    shell.save_coach_settings(change)
}

/// Change the water goal (from today), or add or remove one serving button.
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_water_settings(
    shell: ShellState<'_>,
    change: WaterChange,
) -> Result<SettingsPage, String> {
    shell.save_water_settings(change)
}

/// Change the workdays, hours or time zone (only the fields sent).
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_workweek(
    shell: ShellState<'_>,
    change: WorkweekChange,
) -> Result<SettingsPage, String> {
    shell.save_workweek(change)
}

/// Save the display preferences.
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_accessibility(
    shell: ShellState<'_>,
    a11y: Accessibility,
) -> Result<SettingsPage, String> {
    shell.save_accessibility(a11y)
}

// ------------------------------------------------------------------- plan

/// The recovery-day plan window's data.
///
/// # Errors
/// A read failure, worded for the user.
#[tauri::command]
pub fn plan_page(shell: ShellState<'_>) -> Result<PlanPage, String> {
    shell.plan_page()
}

/// "Save plan".
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn plan_save(shell: ShellState<'_>, plan: RecoveryPlan) -> Result<PlanPage, String> {
    shell.save_plan(plan)
}

/// "Cancel plan".
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn plan_cancel(shell: ShellState<'_>) -> Result<PlanPage, String> {
    shell.cancel_plan()
}

// ------------------------------------------------------------------- setup

/// Open the setup wizard (Settings, "Run setup again").
#[tauri::command]
pub fn open_setup(shell: ShellState<'_>) {
    shell.open_setup();
}

/// The setup wizard's data.
///
/// # Errors
/// A read failure, worded for the user.
#[tauri::command]
pub fn setup_page(shell: ShellState<'_>) -> Result<SettingsPage, String> {
    shell.setup_page()
}

/// Show the wizard's sample reminder beside the pet.
#[tauri::command]
pub fn setup_sample(shell: ShellState<'_>) {
    shell.setup_sample();
}

/// "Finish": setup is done; the wizard closes.
///
/// # Errors
/// A write failure, worded for the user.
#[tauri::command]
pub fn setup_finish(shell: ShellState<'_>) -> Result<(), String> {
    shell.setup_finish()
}

// ------------------------------------------------------------------ backup

/// "Export my data": an encrypted backup in the Documents folder.
///
/// # Errors
/// The refusal, worded for the user; nothing was written.
#[tauri::command]
pub fn backup_export(shell: ShellState<'_>, password: String) -> Result<BackupDone, String> {
    shell.export_backup(&password)
}

/// The bytes of a backup file the page read (sent as the raw request body).
///
/// # Errors
/// Not a raw body, or a file too large to be a backup.
#[tauri::command]
pub fn backup_stage(
    shell: ShellState<'_>,
    request: tauri::ipc::Request<'_>,
) -> Result<usize, String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => shell.stage_backup(bytes.clone()),
        tauri::ipc::InvokeBody::Json(_) => Err("The file could not be read.".to_owned()),
    }
}

/// "Import": replace this PC's data with the staged backup's, then restart; or, after the
/// wait, "still running" (the outcome follows as `hy://backup-import`).
///
/// # Errors
/// The refusal, worded for the user.
#[tauri::command]
pub fn backup_import(shell: ShellState<'_>, password: String) -> Result<ImportOutcome, String> {
    shell.import_backup(password)
}

// ------------------------------------------------------------- office server

/// The pairing section of Settings > Sharing & server.
///
/// # Errors
/// A read failure, worded for the user.
#[tauri::command]
pub fn server_status(shell: ShellState<'_>) -> Result<LinkView, String> {
    server_link::view(&shell, None)
}

/// "Pair" with the slip the user pasted or typed.
///
/// # Errors
/// The reason, worded for the user; nothing was stored.
#[tauri::command]
pub async fn server_pair(shell: ShellState<'_>, slip: String) -> Result<LinkView, String> {
    server_link::pair(shell.inner().clone(), slip).await
}

/// "Check again": ask the server for this PC's state.
///
/// # Errors
/// A store failure, worded for the user.
#[tauri::command]
pub async fn server_refresh(shell: ShellState<'_>) -> Result<LinkView, String> {
    server_link::refresh(shell.inner().clone()).await
}

/// "Unpair this PC".
///
/// # Errors
/// A store failure, worded for the user.
#[tauri::command]
pub async fn server_unpair(shell: ShellState<'_>) -> Result<LinkView, String> {
    server_link::unpair(shell.inner().clone()).await
}

/// Switch an optional category on (after its notice) or off.
///
/// # Errors
/// The reason, worded for the user.
#[tauri::command]
pub async fn server_share_set(
    shell: ShellState<'_>,
    category: String,
    enabled: bool,
) -> Result<LinkView, String> {
    server_link::set_sharing(shell.inner().clone(), category, enabled).await
}

/// Dismiss a sharing notice.
///
/// # Errors
/// A store failure, worded for the user.
#[tauri::command]
pub fn server_notice_dismiss(shell: ShellState<'_>, id: u64) -> Result<LinkView, String> {
    server_link::dismiss_notice(&shell, id)
}

/// Today's record for a category exactly as it would be sent.
///
/// # Errors
/// The reason, worded for the user.
#[tauri::command]
pub fn server_share_preview(
    shell: ShellState<'_>,
    category: String,
) -> Result<Option<String>, String> {
    server_link::preview(&shell, &category)
}

/// "Restore from server".
///
/// # Errors
/// A store failure, worded for the user.
#[tauri::command]
pub async fn server_restore(shell: ShellState<'_>) -> Result<LinkView, String> {
    server_link::restore(shell.inner().clone()).await
}

/// "Server address": the IP address to try when the server moved (empty clears it).
///
/// # Errors
/// A malformed address or a store failure, worded for the user.
#[tauri::command]
pub fn server_set_address(shell: ShellState<'_>, address: String) -> Result<LinkView, String> {
    server_link::set_address(&shell, &address)
}

/// The user saw the new-device notice for a PC.
///
/// # Errors
/// A malformed id or a store failure, worded for the user.
#[tauri::command]
pub fn server_notice_seen(shell: ShellState<'_>, device_ref: String) -> Result<LinkView, String> {
    server_link::notice_seen(&shell, &device_ref)
}

// ------------------------------------------------------------- app and updates (V1-6)

/// Settings > App & updates.
#[tauri::command]
pub fn app_page(updater: UpdaterState<'_>) -> AppPage {
    updater.page()
}

/// "Install update from file": the Open dialog, then the signature and version checks.
/// `None` if the user cancelled the dialog. Async: the dialog waits on the user and the
/// check hashes the whole installer, so neither may hold the main thread.
///
/// # Errors
/// Why the file is refused, worded for the page; nothing changed.
#[tauri::command]
pub async fn update_choose(
    app: tauri::AppHandle,
    updater: UpdaterState<'_>,
) -> Result<Option<UpdateCheck>, String> {
    use tauri::Manager;
    let owner = app
        .get_webview_window(crate::shell::SCREENS)
        .as_ref()
        .and_then(crate::shell::hwnd_of)
        .unwrap_or(0);
    let updater = Arc::clone(&updater);
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = choose_installer(owner) else {
            return Ok(None);
        };
        updater.check(&path).map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(windows)]
fn choose_installer(owner: isize) -> Option<std::path::PathBuf> {
    crate::win32::choose_installer(owner, "Choose the HealthierYou update file")
}

#[cfg(not(windows))]
fn choose_installer(_owner: isize) -> Option<std::path::PathBuf> {
    None
}

/// "Install and restart": stage the checked file, start the update helper, then close
/// normally (the coaching engine saves first) once the page has its answer.
///
/// # Errors
/// Why nothing was installed, worded for the page.
#[tauri::command]
pub fn update_install(app: tauri::AppHandle, updater: UpdaterState<'_>) -> Result<(), String> {
    updater.start_install()?;
    let _ = std::thread::Builder::new()
        .name("hy-update-exit".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            app.exit(0);
        });
    Ok(())
}

/// Start at sign-in on or off.
///
/// # Errors
/// Not an installed copy, or a registry or file error, worded for the page.
#[tauri::command]
pub fn set_autostart(updater: UpdaterState<'_>, enabled: bool) -> Result<AppPage, String> {
    crate::update::set_autostart(updater.dirs(), enabled)?;
    Ok(updater.page())
}

// ------------------------------------------------------------------ posture check (V2-1)

/// "Posture check" from the pet menu, the panel or Settings: the camera page.
#[tauri::command]
pub fn camera_open(shell: ShellState<'_>) {
    shell.camera_open();
}

/// "Turn camera off" from anywhere: capture ends within two seconds.
#[tauri::command]
pub fn camera_off(shell: ShellState<'_>) -> CameraPage {
    shell.camera_off()
}

/// "Turn camera on" in Settings (the consent page opens if none is recorded).
#[tauri::command]
pub fn camera_on(shell: ShellState<'_>) -> CameraPage {
    shell.camera_on()
}

/// The camera page's view (also the settings' Posture check tab).
#[tauri::command]
pub fn camera_page(shell: ShellState<'_>) -> CameraPage {
    shell.camera_page()
}

/// "Turn on camera" (`accept`) or "Not now" on the consent text (wireframe W16).
///
/// # Errors
/// The refusal in words.
#[tauri::command]
pub fn camera_consent(shell: ShellState<'_>, accept: bool) -> Result<CameraPage, String> {
    shell.camera_consent(accept)
}

/// "Try again" after the camera was blocked, busy, missing or stopped.
#[tauri::command]
pub fn camera_try_again(shell: ShellState<'_>) -> CameraPage {
    shell.camera_try_again()
}

/// "Withdraw consent" in Settings: off, and the consent text is shown next time.
#[tauri::command]
pub fn camera_withdraw(shell: ShellState<'_>) -> CameraPage {
    shell.camera_withdraw()
}

/// The camera page's report (started, error, stopped).
///
/// # Errors
/// A malformed report.
#[tauri::command]
pub fn camera_report(shell: ShellState<'_>, report: PageReport) -> Result<(), String> {
    shell.camera_report(report)
}

/// One frame's derived result (never the image or the landmarks).
///
/// # Errors
/// A report that breaks its rules.
#[tauri::command]
pub fn camera_frame(shell: ShellState<'_>, report: FrameReport) -> Result<(), String> {
    shell.camera_frame(report)
}
