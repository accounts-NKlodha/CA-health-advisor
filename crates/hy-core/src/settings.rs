//! Field-level changes to the settings the engine follows (wireframes W4 to W7 and W1).
//!
//! A settings page sends only the field the user changed, never a whole copy of the
//! settings it loaded earlier: another window (the weekly review's Apply or Undo, the
//! setup wizard, a second change sent before the first answered) may have changed other
//! fields since, and writing back a stale copy would silently undo them. The store
//! applies each change to the values saved now ([`crate::store::Store::change_coach_config`]
//! and the other `change_` methods), and the companion runs those on the engine's own
//! thread, one at a time.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

use crate::coach::{CoachConfig, Intensity, MeditationTime, MovementVersion, ReviewTime, Servings};
use crate::error::{Error, Result};
use crate::time::{WeekdaySet, WorkSchedule};

/// One field of the coaching settings, as a settings page changes it. Serialised as a
/// single key, e.g. `{"eye_rest": false}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CoachChange {
    /// Light, Balanced or More often.
    Intensity(Intensity),
    /// The movement reminders switch.
    Movement(bool),
    /// The eye-rest reminders switch.
    EyeRest(bool),
    /// The water pacing switch (W4 and W5 show the same setting).
    Water(bool),
    /// Hide during presentations and full-screen apps.
    HideDuringPresentation(bool),
    /// The camera or microphone hint.
    MediaInUseHint(bool),
    /// The long-day check-in switch.
    LongDayCheckIn(bool),
    /// The long-day check-in's hours (8, 9 or 10).
    LongDayHours(u8),
    /// Pomodoro auto-start.
    PomodoroAutoStart(bool),
    /// The routine's default version.
    MovementVersion(MovementVersion),
    /// When the weekly review is offered.
    Review(ReviewTime),
    /// When the meditation pause is offered (off until a time is set).
    Meditation(MeditationTime),
}

impl CoachChange {
    /// `config` with this one field changed (the result is not yet validated).
    #[must_use]
    pub const fn applied_to(self, mut config: CoachConfig) -> CoachConfig {
        match self {
            Self::Intensity(v) => config.intensity = v,
            Self::Movement(v) => config.categories.movement = v,
            Self::EyeRest(v) => config.categories.eye_rest = v,
            Self::Water(v) => config.categories.water = v,
            Self::HideDuringPresentation(v) => config.hide_during_presentation = v,
            Self::MediaInUseHint(v) => config.media_in_use_hint = v,
            Self::LongDayCheckIn(v) => config.long_day_check_in.enabled = v,
            Self::LongDayHours(v) => config.long_day_check_in.hours = v,
            Self::PomodoroAutoStart(v) => config.pomodoro_auto_start = v,
            Self::MovementVersion(v) => config.movement_version = v,
            Self::Review(v) => config.review = v,
            Self::Meditation(v) => config.meditation = v,
        }
        config
    }
}

/// One change on the Water tab: the goal, or one serving button added or removed.
/// Serialised as a single key, e.g. `{"add_serving": 750}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WaterChange {
    /// The daily goal in ml, from today.
    Goal(u32),
    /// Add a serving button.
    AddServing(u32),
    /// Remove a serving button.
    RemoveServing(u32),
    /// The setup wizard's list edit: remove some buttons and add others in one change,
    /// against the buttons saved now (W1 step 5).
    EditServings {
        /// Amounts to add.
        add: Vec<u32>,
        /// Amounts to remove.
        remove: Vec<u32>,
    },
}

impl WaterChange {
    /// The serving buttons after this change, if it changes them (`None` for the goal).
    ///
    /// Adding a button already there and removing one that is gone are no change (another
    /// window already did it); removing the last button is refused.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the result breaks the serving rules.
    pub fn servings_after(&self, now: &Servings) -> Result<Option<Servings>> {
        let (add, remove): (&[u32], &[u32]) = match self {
            Self::Goal(_) => return Ok(None),
            Self::AddServing(ml) => (std::slice::from_ref(ml), &[]),
            Self::RemoveServing(ml) => (&[], std::slice::from_ref(ml)),
            Self::EditServings { add, remove } => (add, remove),
        };
        let mut v: Vec<u32> = now
            .ml()
            .iter()
            .copied()
            .filter(|x| !remove.contains(x))
            .collect();
        for ml in add {
            if !v.contains(ml) {
                v.push(*ml);
            }
        }
        if v.is_empty() {
            return Err(Error::invalid("keep at least one serving button"));
        }
        Servings::new(v).map(Some)
    }
}

/// The work window's start and end, changed together (they are checked together).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkHours {
    /// Local start, minute precision.
    #[serde(with = "crate::time::hhmm")]
    pub start: NaiveTime,
    /// Local end, minute precision.
    #[serde(with = "crate::time::hhmm")]
    pub end: NaiveTime,
}

/// The workweek fields a save changes; fields left out keep their saved value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkweekChange {
    /// The workdays.
    #[serde(default)]
    pub workdays: Option<WeekdaySet>,
    /// The work hours.
    #[serde(default)]
    pub hours: Option<WorkHours>,
    /// The time zone's name.
    #[serde(default)]
    pub zone: Option<String>,
}

impl WorkweekChange {
    /// `schedule` with the changed fields (the result is not yet validated).
    #[must_use]
    pub fn schedule_after(&self, mut schedule: WorkSchedule) -> WorkSchedule {
        if let Some(w) = self.workdays {
            schedule.workdays = w;
        }
        if let Some(h) = self.hours {
            schedule.start = h.start;
            schedule.end = h.end;
        }
        schedule
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_coach_change_touches_only_its_field() {
        let mut other = CoachConfig {
            movement_version: MovementVersion::Seated,
            intensity: Intensity::Light,
            ..CoachConfig::default()
        };
        other.categories.water = false;
        let after = CoachChange::EyeRest(false).applied_to(other);
        assert!(!after.categories.eye_rest);
        assert_eq!(
            CoachConfig {
                categories: other.categories,
                ..after
            },
            other
        );
        let mut eye = other;
        eye.categories.eye_rest = false;
        assert_eq!(after, eye);
        assert_eq!(
            CoachChange::LongDayHours(10)
                .applied_to(other)
                .long_day_check_in,
            crate::coach::config::LongDayCheckIn {
                enabled: true,
                hours: 10
            }
        );
    }

    #[test]
    fn coach_changes_use_one_key() {
        let c: CoachChange = serde_json::from_str(r#"{"eye_rest":false}"#).unwrap();
        assert_eq!(c, CoachChange::EyeRest(false));
        let c: CoachChange =
            serde_json::from_str(r#"{"review":{"day":"fri","at":"18:00"}}"#).unwrap();
        assert!(matches!(c, CoachChange::Review(_)));
        let c: CoachChange = serde_json::from_str(r#"{"meditation":{"at":"15:00"}}"#).unwrap();
        assert!(matches!(
            c,
            CoachChange::Meditation(MeditationTime { at: Some(_) })
        ));
        let c: CoachChange = serde_json::from_str(r#"{"meditation":{"at":null}}"#).unwrap();
        assert_eq!(c, CoachChange::Meditation(MeditationTime { at: None }));
        let c: CoachChange = serde_json::from_str(r#"{"movement_version":"seated"}"#).unwrap();
        assert_eq!(c, CoachChange::MovementVersion(MovementVersion::Seated));
        assert!(serde_json::from_str::<CoachChange>(r#"{"eye_rest":false,"water":true}"#).is_err());
        assert!(serde_json::from_str::<CoachChange>(r#"{"config":{}}"#).is_err());
    }

    #[test]
    fn serving_changes_apply_to_the_buttons_saved_now() {
        let now = Servings::new(vec![150, 250, 500, 750]).unwrap();
        let after = WaterChange::RemoveServing(150)
            .servings_after(&now)
            .unwrap()
            .unwrap();
        assert_eq!(after.ml(), [250, 500, 750]);
        // Already added or already removed elsewhere: no change.
        let same = WaterChange::AddServing(750).servings_after(&now).unwrap();
        assert_eq!(same.unwrap(), now);
        let same = WaterChange::RemoveServing(300)
            .servings_after(&now)
            .unwrap();
        assert_eq!(same.unwrap(), now);
        assert!(WaterChange::AddServing(300).servings_after(&now).is_err()); // a fifth
        let one = Servings::new(vec![250]).unwrap();
        assert!(
            WaterChange::RemoveServing(250)
                .servings_after(&one)
                .is_err()
        );
        assert_eq!(WaterChange::Goal(2000).servings_after(&one).unwrap(), None);
        // The wizard's list edit: 750 added elsewhere stays; 150 swapped for 200.
        let edit = WaterChange::EditServings {
            add: vec![200],
            remove: vec![150],
        };
        assert_eq!(
            edit.servings_after(&now).unwrap().unwrap().ml(),
            [200, 250, 500, 750]
        );
        let all = WaterChange::EditServings {
            add: vec![],
            remove: vec![150, 250, 500, 750],
        };
        assert!(all.servings_after(&now).is_err());
        let swap = WaterChange::EditServings {
            add: vec![300],
            remove: vec![250],
        };
        assert_eq!(
            swap.servings_after(&one).unwrap().unwrap().ml(),
            [300] // remove before add: never empty in between
        );
    }

    #[test]
    fn a_workweek_change_keeps_the_fields_left_out() {
        let saved = WorkSchedule {
            workdays: WeekdaySet::MON_TO_SAT,
            start: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
        };
        let c: WorkweekChange =
            serde_json::from_str(r#"{"hours":{"start":"09:00","end":"17:30"}}"#).unwrap();
        let after = c.schedule_after(saved.clone());
        assert_eq!(after.workdays, saved.workdays);
        assert_eq!(after.end, NaiveTime::from_hms_opt(17, 30, 0).unwrap());
        assert_eq!(
            WorkweekChange::default().schedule_after(saved.clone()),
            saved
        );
        assert!(serde_json::from_str::<WorkweekChange>(r#"{"schedule":{}}"#).is_err());
    }
}
