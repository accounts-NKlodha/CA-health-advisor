//! Coaching settings (wireframe W4 and W7) and the fixed timing rules (architecture 4.3).
//!
//! Values marked P come from the PRD or the approved prototype; values marked D are
//! defaults chosen here where the architecture is silent, open to review. All Balanced
//! values are "starting values to test, not medical advice" (wireframe W4).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::schema::Activity;

/// Settings key under which [`CoachConfig`] is stored.
pub const CONFIG_KEY: &str = "coach_config";

const fn min(m: u64) -> Duration {
    Duration::from_secs(m * 60)
}

/// Input within this long counts as Active (4.3, D).
pub const ACTIVE_WITHIN: Duration = Duration::from_secs(60);
/// No input for longer than this, while unlocked, is Long quiet: the movement timer
/// pauses (4.3, D).
pub const LONG_QUIET_AFTER: Duration = min(15);
/// A lock or sleep at least this long is a confirmed break and resets the movement and
/// eye-rest timers; a shorter one only pauses them (4.3, D).
pub const CONFIRMED_BREAK_AFTER: Duration = min(2);
/// Calm gap after any answer before another reminder may appear (prototype rev 3, P).
pub const CALM_GAP: Duration = min(5);
/// Snooze length; it covers the one activity shown (P).
pub const SNOOZE: Duration = min(10);
/// A reminder bubble left unanswered this long collapses to a "waiting" badge (P).
pub const COLLAPSE_AFTER: Duration = min(10);
/// A waiting badge left this long after the reminder was first shown is closed and
/// logged as "offered, no response" (prototype rev 3: 20 minutes).
pub const BADGE_EXPIRES_AFTER: Duration = min(20);
/// Grace period after unlock or wake before any reminder (P).
pub const GRACE_AFTER_WAKE: Duration = min(2);
/// Length of the guided eye rest started from a reminder (P).
pub const EYE_REST_LENGTH: Duration = Duration::from_secs(20);
/// A movement routine started from a reminder that reports no result within this time is
/// treated as stopped with none recorded, without writing a routine record (D).
pub const ROUTINE_TIMEOUT: Duration = min(30);
/// The water confirmation and the phone note close by themselves this long after the
/// user's last answer on them (D). While one is open no reminder or card opens on its own.
pub const NOTE_SHOWN_FOR: Duration = Duration::from_secs(60);
/// The water panel closes by itself this long after the user's last answer in it (D).
pub const WATER_PANEL_SHOWN_FOR: Duration = min(3);
/// Dismissals or skips in one day that trigger the offer of the Light schedule (D, 4.3).
pub const DISMISSALS_FOR_LIGHTER: u32 = 3;
/// Water is "behind pace" when the logged amount is at least this far below the amount
/// expected by now (wireframe 8 item 12, approved sample rule).
pub const WATER_BEHIND_ML: u32 = 100;
/// Focus presets are free between these bounds, in minutes (prototype: custom 5 to 240).
pub const FOCUS_MINUTES: std::ops::RangeInclusive<u32> = 5..=240;
/// Pause lengths in minutes are accepted between these bounds (D).
pub const PAUSE_MINUTES: std::ops::RangeInclusive<u32> = 1..=24 * 60;

/// Reminder intensity (wireframe W4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intensity {
    /// Fewer reminders (D values).
    Light,
    /// The approved starting values.
    Balanced,
    /// More frequent reminders (D values).
    MoreOften,
}

impl Intensity {
    /// Work time between movement offers (Balanced 45 min, P).
    #[must_use]
    pub const fn movement_every(self) -> Duration {
        match self {
            Self::Light => min(60),
            Self::Balanced => min(45),
            Self::MoreOften => min(30),
        }
    }

    /// Screen time between eye-rest offers (Balanced 20 min, P).
    #[must_use]
    pub const fn eye_rest_every(self) -> Duration {
        match self {
            Self::Light => min(30),
            Self::Balanced => min(20),
            Self::MoreOften => min(15),
        }
    }

    /// Time between water offers while behind pace (Balanced 30 min, prototype).
    #[must_use]
    pub const fn water_every(self) -> Duration {
        match self {
            Self::Light => min(45),
            Self::Balanced => min(30),
            Self::MoreOften => min(20),
        }
    }
}

/// Which activities may be offered (wireframe W4 category switches; all on by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Categories {
    /// Movement reminders.
    pub movement: bool,
    /// Eye-rest reminders (optional; on in the approved prototype).
    pub eye_rest: bool,
    /// Hydration reminders.
    pub water: bool,
}

impl Categories {
    /// Whether `activity` is switched on.
    #[must_use]
    pub const fn allows(&self, activity: Activity) -> bool {
        match activity {
            Activity::Movement => self.movement,
            Activity::EyeRest => self.eye_rest,
            Activity::Water => self.water,
        }
    }
}

impl Default for Categories {
    fn default() -> Self {
        Self {
            movement: true,
            eye_rest: true,
            water: true,
        }
    }
}

/// The optional long-day check-in (owner approved default, 21 Oct: optional and
/// configurable, 8, 9 or 10 hours, 9 by default; PC activity is an estimate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongDayCheckIn {
    /// Offer the check-in automatically.
    pub enabled: bool,
    /// Hours of estimated PC activity today before it is offered: 8, 9 or 10.
    pub hours: u8,
}

impl Default for LongDayCheckIn {
    fn default() -> Self {
        Self {
            enabled: true,
            hours: 9,
        }
    }
}

/// The user's coaching settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoachConfig {
    /// Light, Balanced or More often.
    pub intensity: Intensity,
    /// Category switches.
    pub categories: Categories,
    /// Hide the pet and every bubble during presentations and full-screen apps (owner
    /// approved default, 21 Oct: on).
    pub hide_during_presentation: bool,
    /// Treat "camera or microphone in use" as a meeting (experimental, off; A11).
    pub media_in_use_hint: bool,
    /// The long-day check-in.
    pub long_day_check_in: LongDayCheckIn,
    /// Start the next Pomodoro round automatically (off; wireframe W7).
    pub pomodoro_auto_start: bool,
}

impl Default for CoachConfig {
    fn default() -> Self {
        Self {
            intensity: Intensity::Balanced,
            categories: Categories::default(),
            hide_during_presentation: true,
            media_in_use_hint: false,
            long_day_check_in: LongDayCheckIn::default(),
            pomodoro_auto_start: false,
        }
    }
}

impl CoachConfig {
    /// Check the settings' own rules.
    ///
    /// # Errors
    /// [`Error::Invalid`] if the check-in hours are not 8, 9 or 10.
    pub fn validate(&self) -> Result<()> {
        if !(8..=10).contains(&self.long_day_check_in.hours) {
            return Err(Error::invalid("long-day check-in hours must be 8, 9 or 10"));
        }
        Ok(())
    }

    /// The offer interval for `activity` at the current intensity.
    #[must_use]
    pub const fn every(&self, activity: Activity) -> Duration {
        match activity {
            Activity::Movement => self.intensity.movement_every(),
            Activity::EyeRest => self.intensity.eye_rest_every(),
            Activity::Water => self.intensity.water_every(),
        }
    }
}

/// A Pomodoro preset (wireframe B6: 25/5, 50/10 or custom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PomodoroPreset {
    /// 25 minutes of work, 5 of break.
    Classic,
    /// 50 minutes of work, 10 of break.
    Long,
    /// Custom: work 5 to 120 minutes, break 1 to 30 minutes (D bounds).
    Custom {
        /// Work minutes.
        work_min: u32,
        /// Break minutes.
        break_min: u32,
    },
}

impl PomodoroPreset {
    /// Work and break lengths.
    ///
    /// # Errors
    /// [`Error::Invalid`] if a custom preset is outside its bounds.
    pub fn lengths(self) -> Result<(Duration, Duration)> {
        let (w, b) = match self {
            Self::Classic => (25, 5),
            Self::Long => (50, 10),
            Self::Custom {
                work_min,
                break_min,
            } => {
                if !(5..=120).contains(&work_min) || !(1..=30).contains(&break_min) {
                    return Err(Error::invalid(
                        "custom Pomodoro needs 5 to 120 work minutes and 1 to 30 break minutes",
                    ));
                }
                (u64::from(work_min), u64::from(break_min))
            }
        };
        Ok((min(w), min(b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balanced_defaults_match_the_approved_prototype() {
        let c = CoachConfig::default();
        assert_eq!(c.every(Activity::Movement), min(45));
        assert_eq!(c.every(Activity::EyeRest), min(20));
        assert_eq!(c.every(Activity::Water), min(30));
        assert!(c.hide_during_presentation);
        assert!(!c.media_in_use_hint);
        assert!(!c.pomodoro_auto_start);
        assert_eq!(
            c.long_day_check_in,
            LongDayCheckIn {
                enabled: true,
                hours: 9
            }
        );
        assert!(c.categories.allows(Activity::EyeRest));
        c.validate().unwrap();
    }

    #[test]
    fn intensities_are_ordered() {
        for a in Activity::ALL {
            let every = |i| {
                CoachConfig {
                    intensity: i,
                    ..CoachConfig::default()
                }
                .every(*a)
            };
            assert!(every(Intensity::Light) > every(Intensity::Balanced));
            assert!(every(Intensity::Balanced) > every(Intensity::MoreOften));
        }
    }

    #[test]
    fn check_in_hours_are_eight_nine_or_ten() {
        for (h, ok) in [(7, false), (8, true), (9, true), (10, true), (11, false)] {
            let c = CoachConfig {
                long_day_check_in: LongDayCheckIn {
                    enabled: true,
                    hours: h,
                },
                ..CoachConfig::default()
            };
            assert_eq!(c.validate().is_ok(), ok, "hours {h}");
        }
    }

    #[test]
    fn config_round_trips_and_refuses_unknown_fields() {
        let c = CoachConfig::default();
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<CoachConfig>(&text).unwrap(), c);
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<CoachConfig>(v).is_err());
    }

    #[test]
    fn pomodoro_presets() {
        assert_eq!(
            PomodoroPreset::Classic.lengths().unwrap(),
            (min(25), min(5))
        );
        assert_eq!(PomodoroPreset::Long.lengths().unwrap(), (min(50), min(10)));
        let custom = |w, b| {
            PomodoroPreset::Custom {
                work_min: w,
                break_min: b,
            }
            .lengths()
        };
        assert_eq!(custom(40, 8).unwrap(), (min(40), min(8)));
        assert!(custom(4, 5).is_err());
        assert!(custom(121, 5).is_err());
        assert!(custom(25, 0).is_err());
        assert!(custom(25, 31).is_err());
    }
}
