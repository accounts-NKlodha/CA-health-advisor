//! The V1-4 settings tabs (wireframes W4 to W7 and W12), the recovery-day plan window
//! (W15) and the weekly review's suggestions, on the shell.
//!
//! Settings the engine follows (coaching settings, serving buttons, work hours, time
//! zone, water goal) and the suggestions that change them are written through the
//! engine's own store connection on its thread ([`CoachLink::change_settings`]), which
//! refreshes the engine at once, so the engine never reads a half-made change and only
//! one connection writes them. Display preferences are the shell's own and are written
//! here. Without a running engine (it failed to start) the shell's connection is used.

use std::sync::Arc;

use chrono::{Datelike, Days, NaiveDate, Weekday};
use hy_core::coach::movement::{LIBRARY, PROGRAMME, PROVISIONAL, PROVISIONAL_NOTICE};
use hy_core::coach::recovery::PLAN_AHEAD_DAYS;
use hy_core::coach::water::{MAX_SERVINGS, WATER_PANEL_RANGE_ML};
use hy_core::coach::{CoachConfig, Input, InputOutcome, RecoveryAction, RecoveryPlan, Servings};
use hy_core::prefs::Accessibility;
use hy_core::review::WeekReview;
use hy_core::review::suggest::SuggestionKind;
use hy_core::schema::{GoalKind, WATER_GOAL_RANGE_ML};
use hy_core::store::Store;
use hy_core::time::{UserZone, WorkSchedule};
use hy_pet::link::CoachLink;
use serde::Deserialize;
use tauri::{Emitter, Manager};

use super::{PLAN, Shell, lock};
use crate::coach::CoachRunner;
use crate::view::{LibraryMovement, PlanDate, PlanPage, ProgrammeDay, ReviewPage, SettingsPage};
use crate::windows;

/// What the review page asks of a suggestion (wireframe W13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionAnswer {
    /// "Apply".
    Apply,
    /// "Undo".
    Undo,
    /// "Not for me".
    Hide,
}

const fn day_short(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

impl Shell {
    fn runner(&self) -> Option<Arc<CoachRunner>> {
        self.app
            .try_state::<Arc<CoachRunner>>()
            .map(|r| Arc::clone(r.inner()))
    }

    /// Run a settings change on the engine's store connection (then the engine
    /// refreshes), or on the shell's when the engine is not running.
    fn change_settings<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> hy_core::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let result = match self.runner() {
            Some(runner) => runner
                .call(move |link: &mut CoachLink| link.change_settings(f))
                .ok_or_else(|| "The app did not answer in time. Try again.".to_owned())?,
            None => f(&mut lock(&self.store)),
        };
        result.map_err(|e| format!("Nothing was changed: {e}"))
    }

    // ---------------------------------------------------------------- settings

    /// The settings window's coaching, water, workweek, movement and accessibility tabs.
    ///
    /// # Errors
    /// A store read failure, worded for the page.
    pub fn settings_page(&self) -> Result<SettingsPage, String> {
        let a11y = {
            let mut i = lock(&self.inner);
            i.settings_loaded = true;
            i.a11y
        };
        let hotkey = self.hotkey_label();
        let store = lock(&self.store);
        let read = |e: hy_core::Error| format!("Settings could not be read: {e}");
        let today = store.today().map_err(read)?;
        Ok(SettingsPage {
            coach: store.coach_config().map_err(read)?,
            schedule: store.work_schedule().map_err(read)?,
            zone: store.user_zone().map_err(read)?.name(),
            zones: UserZone::names(),
            water_goal_ml: store.goal_on(GoalKind::WaterDailyMl, today).map_err(read)?,
            goal_min_ml: *WATER_GOAL_RANGE_ML.start(),
            goal_max_ml: *WATER_GOAL_RANGE_ML.end(),
            servings: store.water_servings().map_err(read)?,
            serving_min_ml: *WATER_PANEL_RANGE_ML.start(),
            serving_max_ml: *WATER_PANEL_RANGE_ML.end(),
            max_servings: MAX_SERVINGS,
            a11y,
            programme: PROGRAMME
                .iter()
                .map(|r| ProgrammeDay {
                    day: day_short(r.day),
                    movements: r.movements.iter().map(|e| e.main.name).collect(),
                })
                .collect(),
            library: LIBRARY
                .iter()
                .map(|e| LibraryMovement {
                    name: e.main.name,
                    place: e.main.place,
                    dose: e.main.dose,
                    seated: e.seated.map(|s| s.name),
                })
                .collect(),
            provisional: PROVISIONAL.then_some(PROVISIONAL_NOTICE),
            hotkey,
        })
    }

    /// Save the coaching settings (Reminders, Movement programme, Focus and Pomodoro, and
    /// the review time; wireframes W4 to W7).
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn save_coach_settings(&self, config: CoachConfig) -> Result<SettingsPage, String> {
        self.change_settings(move |s| s.set_coach_config(&config))?;
        self.settings_page()
    }

    /// Save the water goal (from today; earlier days keep the goal they had) and the
    /// serving buttons (wireframe W5).
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn save_water_settings(
        &self,
        goal_ml: Option<u32>,
        servings: Vec<u32>,
    ) -> Result<SettingsPage, String> {
        let servings = Servings::new(servings).map_err(|e| format!("Nothing was changed: {e}"))?;
        self.change_settings(move |s| {
            let today = s.today()?;
            if let Some(goal) = goal_ml
                && s.goal_on(GoalKind::WaterDailyMl, today)? != Some(goal)
            {
                s.set_goal(GoalKind::WaterDailyMl, goal, today)?;
            }
            s.set_water_servings(&servings)
        })?;
        self.settings_page()
    }

    /// Save the workdays, work hours and time zone (wireframe W5).
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn save_workweek(
        &self,
        schedule: WorkSchedule,
        zone: &str,
    ) -> Result<SettingsPage, String> {
        let zone = UserZone::parse(zone).map_err(|e| format!("Nothing was changed: {e}"))?;
        self.change_settings(move |s| {
            s.set_work_schedule(&schedule)?;
            s.set_user_zone(zone)
        })?;
        self.settings_page()
    }

    /// Save the display preferences (wireframe W12). Every open page follows them at
    /// once: the pet and panel through their views, the windows through an event.
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn save_accessibility(
        self: &Arc<Self>,
        a11y: Accessibility,
    ) -> Result<SettingsPage, String> {
        lock(&self.store)
            .set_accessibility(&a11y)
            .map_err(|e| format!("Nothing was changed: {e}"))?;
        lock(&self.inner).a11y = a11y;
        self.render();
        let _ = self.app.emit("hy://a11y", a11y);
        self.settings_page()
    }

    /// The display preferences every page follows.
    pub fn display_prefs(&self) -> Accessibility {
        lock(&self.inner).a11y
    }

    /// Whether a settings page loaded its tabs (for the self-test).
    pub fn settings_loaded(&self) -> bool {
        lock(&self.inner).settings_loaded
    }

    // -------------------------------------------------------------- suggestions

    /// Apply, undo or hide a suggestion of the week starting `week`, then that week's
    /// review again.
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn review_suggestion(
        &self,
        week: NaiveDate,
        kind: SuggestionKind,
        answer: SuggestionAnswer,
    ) -> Result<ReviewPage<WeekReview>, String> {
        self.change_settings(move |s| match answer {
            SuggestionAnswer::Apply => s.apply_suggestion(week, kind),
            SuggestionAnswer::Undo => s.undo_suggestion(week, kind),
            SuggestionAnswer::Hide => s.hide_suggestion(week, kind),
        })?;
        self.review_week(Some(week))
    }

    // --------------------------------------------------------------------- plan

    /// Open the recovery-day plan window (or focus it).
    pub fn open_plan(self: &Arc<Self>) {
        let monitor = lock(&self.inner).core.monitor().cloned();
        if let Err(e) = windows::open_plan(&self.app, monitor.as_ref()) {
            eprintln!("HealthierYou: could not open the plan: {e}");
        }
    }

    /// The plan window's data: the saved plan (from the engine), a starting plan for the
    /// next workday, and the dates it can move to.
    ///
    /// # Errors
    /// A read failure, or the engine is not running.
    pub fn plan_page(&self) -> Result<PlanPage, String> {
        let a11y = {
            let mut i = lock(&self.inner);
            i.plan_loaded = true;
            i.a11y
        };
        let runner = self.runner().ok_or_else(|| {
            "Recovery days need the coaching engine, which is not running.".to_owned()
        })?;
        let plan = runner
            .call(|link: &mut CoachLink| link.view().recovery.plan)
            .ok_or_else(|| "The app did not answer in time. Try again.".to_owned())?;
        let store = lock(&self.store);
        let read = |e: hy_core::Error| format!("The plan could not be read: {e}");
        let today = store.today().map_err(read)?;
        let schedule = store.work_schedule().map_err(read)?;
        let zone = store.user_zone().map_err(read)?;
        let dates: Vec<PlanDate> = (1..=PLAN_AHEAD_DAYS)
            .filter_map(|d| today.checked_add_days(Days::new(u64::try_from(d).unwrap_or(1))))
            .map(|date| PlanDate {
                date,
                workday: schedule.workdays.contains(date.weekday()),
            })
            .collect();
        let next = dates
            .iter()
            .find(|d| d.workday)
            .or(dates.first())
            .map_or(today, |d| d.date);
        Ok(PlanPage {
            plan: plan.filter(|p| p.date > today),
            draft: RecoveryPlan::default_for(next, &schedule, &zone),
            dates,
            water_goal_ml: store.goal_on(GoalKind::WaterDailyMl, today).map_err(read)?,
            a11y,
        })
    }

    fn recovery(&self, action: RecoveryAction) -> Result<PlanPage, String> {
        let runner = self.runner().ok_or_else(|| {
            "Recovery days need the coaching engine, which is not running.".to_owned()
        })?;
        let outcome = runner
            .call(move |link: &mut CoachLink| {
                link.input(Input::Recovery(action)).map(|o| o.outcome)
            })
            .ok_or_else(|| "The app did not answer in time. Try again.".to_owned())?
            .map_err(|e| format!("Nothing was changed: {e}"))?;
        match outcome {
            InputOutcome::Applied => self.plan_page(),
            InputOutcome::Invalid(why) | InputOutcome::Ignored(why) => {
                Err(format!("Nothing was changed: {why}."))
            }
            InputOutcome::NeedsConfirmation(_) => Err("Nothing was changed.".to_owned()),
        }
    }

    /// "Save plan": the edited plan replaces the pending one (one plan is kept).
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn save_plan(&self, plan: RecoveryPlan) -> Result<PlanPage, String> {
        self.recovery(RecoveryAction::SavePlan(plan))
    }

    /// "Cancel plan".
    ///
    /// # Errors
    /// The refusal, worded for the page; nothing was changed.
    pub fn cancel_plan(&self) -> Result<PlanPage, String> {
        self.recovery(RecoveryAction::CancelPlan)
    }

    /// Whether the plan window loaded (for the self-test).
    pub fn plan_loaded(&self) -> bool {
        lock(&self.inner).plan_loaded
    }

    /// Close the plan window, if open.
    pub fn close_plan(&self) {
        if let Some(w) = self.app.get_webview_window(PLAN) {
            let _ = w.close();
        }
    }
}
