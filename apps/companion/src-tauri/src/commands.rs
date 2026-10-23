//! IPC commands. Each window may call only the commands its capability file grants
//! (`capabilities/*.json`). The commands are synchronous, so Tauri runs them on the main
//! thread, which the shell requires; the exceptions are the server-link commands that
//! wait on the network, which are async and touch only the store.

use std::sync::Arc;

use chrono::NaiveDate;
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
use crate::shell::{BackupDone, ImportOutcome, Shell, SuggestionAnswer};
use crate::view::{PanelView, PetView, PlanPage, ReviewPage, SettingsPage, SettingsView};

type ShellState<'a> = State<'a, Arc<Shell>>;

// ------------------------------------------------------------------ pet window

/// The pet page's first view.
#[tauri::command]
pub fn pet_bootstrap(shell: ShellState<'_>) -> PetView {
    shell.current_pet_view()
}

/// The pet page drew its first view.
#[tauri::command]
pub fn pet_ready(shell: ShellState<'_>) {
    shell.mark_page_ready();
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
