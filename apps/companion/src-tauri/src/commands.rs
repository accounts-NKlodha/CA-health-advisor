//! IPC commands. Each window may call only the commands its capability file grants
//! (`capabilities/*.json`). The commands are synchronous, so Tauri runs them on the main
//! thread, which the shell requires.

use std::sync::Arc;

use chrono::NaiveDate;
use hy_core::coach::{CoachConfig, RecoveryPlan};
use hy_core::prefs::Accessibility;
use hy_core::review::suggest::SuggestionKind;
use hy_core::review::{MonthReview, WeekReview};
use hy_core::schema::PhoneLabel;
use hy_core::store::MonthKey;
use hy_core::time::WorkSchedule;
use hy_pet::profile::PetEdit;
use tauri::State;
use uuid::Uuid;

use crate::shell::{Shell, SuggestionAnswer};
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

/// Save the coaching settings (reminders, movement version, Pomodoro, review time).
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_coach_settings(
    shell: ShellState<'_>,
    config: CoachConfig,
) -> Result<SettingsPage, String> {
    shell.save_coach_settings(config)
}

/// Save the water goal (from today) and the serving buttons.
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_water_settings(
    shell: ShellState<'_>,
    goal_ml: Option<u32>,
    servings: Vec<u32>,
) -> Result<SettingsPage, String> {
    shell.save_water_settings(goal_ml, servings)
}

/// Save the workdays, hours and time zone.
///
/// # Errors
/// The refusal, worded for the user; nothing was changed.
#[tauri::command]
pub fn save_workweek(
    shell: ShellState<'_>,
    schedule: WorkSchedule,
    zone: String,
) -> Result<SettingsPage, String> {
    shell.save_workweek(schedule, &zone)
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
