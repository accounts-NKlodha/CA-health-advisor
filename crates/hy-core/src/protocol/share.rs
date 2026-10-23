//! What each sharing category may send (V1-5 contract 8.2): one versioned schema per
//! category, with `deny_unknown_fields`, bounded numbers and no free text.
//!
//! The server validates every uploaded payload with [`validate`], and the companion's
//! preview renders the exact record it would send through the same types, so a field that
//! is not listed here can never reach the server. Within-day timing, notes, serving names
//! and reminder text are not representable.

use serde::{Deserialize, Serialize};

use crate::schema::Category;

/// The payload schema version every row carries as `"v"`.
pub const SCHEMA_VERSION: u8 = 1;

/// Upper bound for counts of a day (reminders, reports, entries).
pub const MAX_DAY_COUNT: u32 = 500;
/// Minutes in a day.
pub const DAY_MINUTES: u32 = 1440;
/// Upper bound for water logged in a day, ml.
pub const MAX_WATER_ML: u32 = 20_000;

/// `hydration`: ml logged, goal ml, number of entries, coverage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hydration {
    /// Schema version.
    pub v: u8,
    /// Water logged, ml.
    pub ml: u32,
    /// The day's goal, ml, if set.
    pub goal_ml: Option<u32>,
    /// Number of entries.
    pub entries: u32,
    /// Share of the workday the companion was running, 0 to 1.
    pub coverage: Option<f64>,
}

/// How the day's movement routine went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineResult {
    /// Done.
    Done,
    /// Partly done.
    Partial,
    /// Not done.
    None,
    /// Skipped on purpose.
    Skipped,
}

/// `outcomes`: reminder counts and the routine result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcomes {
    /// Schema version.
    pub v: u8,
    /// Reminders offered.
    pub offered: u32,
    /// Done.
    pub done: u32,
    /// Snoozed.
    pub snoozed: u32,
    /// Skipped.
    pub skipped: u32,
    /// No response.
    pub no_response: u32,
    /// The routine's result, if there was one.
    pub routine: Option<RoutineResult>,
}

/// `work_session`: estimates only; no timing within the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkSession {
    /// Schema version.
    pub v: u8,
    /// Active minutes (estimate).
    pub active_min: u32,
    /// Longest block, minutes (estimate).
    pub longest_block_min: u32,
    /// Focus sessions.
    pub focus_count: u32,
    /// Pomodoros.
    pub pomodoro_count: u32,
}

/// `phone_reports`: counts by label only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneReports {
    /// Schema version.
    pub v: u8,
    /// Work.
    pub work: u32,
    /// Personal.
    pub personal: u32,
    /// Unsure.
    pub unsure: u32,
}

/// `posture_summary` (V2): minutes and counts only; never frames or landmarks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostureSummary {
    /// Schema version.
    pub v: u8,
    /// Minutes the camera was on.
    pub camera_on_min: u32,
    /// Share of those minutes with a usable reading, 0 to 1.
    pub usable_coverage: f64,
    /// Number of episodes.
    pub episodes: u32,
}

/// Why a payload was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPayload;

fn count(n: u32) -> Result<(), InvalidPayload> {
    if n <= MAX_DAY_COUNT {
        Ok(())
    } else {
        Err(InvalidPayload)
    }
}

fn minutes(n: u32) -> Result<(), InvalidPayload> {
    if n <= DAY_MINUTES {
        Ok(())
    } else {
        Err(InvalidPayload)
    }
}

fn share(x: f64) -> Result<(), InvalidPayload> {
    if x.is_finite() && (0.0..=1.0).contains(&x) {
        Ok(())
    } else {
        Err(InvalidPayload)
    }
}

fn version(v: u8) -> Result<(), InvalidPayload> {
    if v == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(InvalidPayload)
    }
}

impl Hydration {
    fn check(&self) -> Result<(), InvalidPayload> {
        version(self.v)?;
        if self.ml > MAX_WATER_ML || self.goal_ml.is_some_and(|g| g == 0 || g > MAX_WATER_ML) {
            return Err(InvalidPayload);
        }
        count(self.entries)?;
        self.coverage.map_or(Ok(()), share)
    }
}

impl Outcomes {
    fn check(&self) -> Result<(), InvalidPayload> {
        version(self.v)?;
        for n in [
            self.offered,
            self.done,
            self.snoozed,
            self.skipped,
            self.no_response,
        ] {
            count(n)?;
        }
        // Answers never outnumber the reminders offered.
        let answered = u64::from(self.done)
            + u64::from(self.snoozed)
            + u64::from(self.skipped)
            + u64::from(self.no_response);
        if answered > u64::from(self.offered) {
            return Err(InvalidPayload);
        }
        Ok(())
    }
}

impl WorkSession {
    fn check(&self) -> Result<(), InvalidPayload> {
        version(self.v)?;
        minutes(self.active_min)?;
        minutes(self.longest_block_min)?;
        if self.longest_block_min > self.active_min {
            return Err(InvalidPayload);
        }
        count(self.focus_count)?;
        count(self.pomodoro_count)
    }
}

impl PhoneReports {
    fn check(&self) -> Result<(), InvalidPayload> {
        version(self.v)?;
        count(self.work)?;
        count(self.personal)?;
        count(self.unsure)
    }
}

impl PostureSummary {
    fn check(&self) -> Result<(), InvalidPayload> {
        version(self.v)?;
        minutes(self.camera_on_min)?;
        share(self.usable_coverage)?;
        count(self.episodes)
    }
}

fn parse<T: for<'de> Deserialize<'de> + Serialize>(
    v: &serde_json::Value,
    check: impl Fn(&T) -> Result<(), InvalidPayload>,
) -> Result<serde_json::Value, InvalidPayload> {
    let t: T = serde_json::from_value(v.clone()).map_err(|_| InvalidPayload)?;
    check(&t)?;
    // Re-serialised from the typed value: the stored form holds only schema fields.
    let out = serde_json::to_value(&t).map_err(|_| InvalidPayload)?;
    // Every field must be present, an optional one as `null`: a missing key is refused
    // rather than read as "not set" (strict schemas, contract 8.2).
    let (Some(a), Some(b)) = (v.as_object(), out.as_object()) else {
        return Err(InvalidPayload);
    };
    if a.len() != b.len() || !b.keys().all(|k| a.contains_key(k)) {
        return Err(InvalidPayload);
    }
    Ok(out)
}

/// Validate a payload against its category's schema and return its canonical form.
///
/// # Errors
/// [`InvalidPayload`] for an unknown field, a missing field, a wrong type, a number out
/// of bounds or another schema version.
pub fn validate(
    category: Category,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, InvalidPayload> {
    match category {
        Category::Hydration => parse::<Hydration>(payload, Hydration::check),
        Category::Outcomes => parse::<Outcomes>(payload, Outcomes::check),
        Category::WorkSession => parse::<WorkSession>(payload, WorkSession::check),
        Category::PhoneReports => parse::<PhoneReports>(payload, PhoneReports::check),
        Category::PostureSummary => parse::<PostureSummary>(payload, PostureSummary::check),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_payloads_pass_and_come_back_canonical() {
        let h = json!({"v": 1, "ml": 1500, "goal_ml": 2000, "entries": 6, "coverage": 0.8});
        assert_eq!(validate(Category::Hydration, &h).unwrap(), h);
        let o = json!({"v": 1, "offered": 5, "done": 3, "snoozed": 1, "skipped": 0, "no_response": 1, "routine": "partial"});
        assert_eq!(validate(Category::Outcomes, &o).unwrap(), o);
        let w = json!({"v": 1, "active_min": 300, "longest_block_min": 70, "focus_count": 2, "pomodoro_count": 4});
        assert!(validate(Category::WorkSession, &w).is_ok());
        let p = json!({"v": 1, "work": 2, "personal": 1, "unsure": 0});
        assert!(validate(Category::PhoneReports, &p).is_ok());
        let s = json!({"v": 1, "camera_on_min": 120, "usable_coverage": 0.9, "episodes": 3});
        assert!(validate(Category::PostureSummary, &s).is_ok());
    }

    #[test]
    fn unknown_free_text_negative_huge_and_wrong_version_are_refused() {
        // T-S9 (schema part).
        let bad = [
            json!({"v": 1, "ml": 1500, "goal_ml": 2000, "entries": 6, "coverage": 0.8, "note": "x"}),
            json!({"v": 1, "ml": -1, "goal_ml": 2000, "entries": 6, "coverage": 0.8}),
            json!({"v": 1, "ml": 987_654, "goal_ml": 2000, "entries": 6, "coverage": 0.8}),
            json!({"v": 1, "ml": 1500, "goal_ml": 2000, "entries": 6, "coverage": 1.5}),
            json!({"v": 2, "ml": 1500, "goal_ml": 2000, "entries": 6, "coverage": 0.8}),
            json!({"v": 1, "ml": 1500, "entries": 6}),
            json!({"v": 1, "ml": "1500", "goal_ml": 2000, "entries": 6, "coverage": 0.8}),
            json!([1, 2]),
        ];
        for b in &bad {
            assert_eq!(validate(Category::Hydration, b), Err(InvalidPayload), "{b}");
        }
        let o = json!({"v": 1, "offered": 1, "done": 3, "snoozed": 0, "skipped": 0, "no_response": 0, "routine": null});
        assert!(
            validate(Category::Outcomes, &o).is_err(),
            "more answers than offers"
        );
        let o = json!({"v": 1, "offered": 1, "done": 1, "snoozed": 0, "skipped": 0, "no_response": 0, "routine": "great day"});
        assert!(validate(Category::Outcomes, &o).is_err(), "no free text");
        let w = json!({"v": 1, "active_min": 30, "longest_block_min": 70, "focus_count": 0, "pomodoro_count": 0});
        assert!(validate(Category::WorkSession, &w).is_err());
        let w = json!({"v": 1, "active_min": 30, "longest_block_min": 20, "focus_count": 0, "pomodoro_count": 0, "start": "09:00"});
        assert!(
            validate(Category::WorkSession, &w).is_err(),
            "no within-day timing"
        );
    }
}
